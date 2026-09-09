mod ai_model_storage;
mod codex_archived_sessions;
mod conda_cache;
mod docker_build_cache;
#[cfg(any(windows, target_os = "macos", test))]
mod dropbox_cache;
mod project_artifact_schema;
mod project_artifacts;
mod project_root_index;
mod rust_toolchains;

/// Only catalog-owned project rule IDs can request closure of the fixed Codex
/// application set. Never accept process names supplied by the WebView.
pub(super) fn project_artifact_close_processes(id: &str) -> Option<Vec<String>> {
    project_artifacts::contains(id).then(super::codex_worktrees::application_process_names)
}
#[cfg(target_os = "macos")]
mod user_cache_inventory;
#[cfg(windows)]
mod windows_system_cleanup;
#[cfg(target_os = "macos")]
mod xcode_storage;

use std::{
    path::{Path, PathBuf},
    time::Instant,
};

use crate::{
    applications::binary_optimization::macos_universal_binaries,
    applications::catalog::ApplicationInventory,
    cleanup::{
        source_selection::SourceSelectionPolicy, CleanupActionKind, CleanupActionReason,
        CleanupActionResult, CleanupActionStatus, CleanupCategory, CleanupGroup, RiskLevel,
        ScanItemStatus, ScanRuleResult,
    },
    shared::operation::OperationGuard,
};
use mangodisk_platform::ControlledExecutable;

use conda_cache::CondaCacheCleaner;
use docker_build_cache::DockerBuildCacheCleaner;

static CONDA_CACHE_CLEANER: CondaCacheCleaner = CondaCacheCleaner;
static DOCKER_BUILD_CACHE_CLEANER: DockerBuildCacheCleaner = DockerBuildCacheCleaner;
static CLEANERS: [&dyn CleanupCleaner; 2] = [&CONDA_CACHE_CLEANER, &DOCKER_BUILD_CACHE_CLEANER];

pub(crate) struct CleanerExecutionRequest<'a> {
    pub(crate) rule_ids: &'a [String],
    pub(crate) inventory: &'a ApplicationInventory,
    pub(crate) declared_roots: &'a [PathBuf],
    pub(crate) project_roots: &'a [String],
    pub(crate) selected_volume_scope: bool,
    pub(crate) source_selections: &'a SourceSelectionPolicy,
    pub(crate) dry_run: bool,
    pub(crate) operation: &'a OperationGuard,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CleanerPreviewStatus {
    NotApplicable,
    Limited,
    Ready,
}

struct CleanerPreview {
    status: CleanerPreviewStatus,
    bytes: u64,
    item_count: u64,
    elapsed_ms: u64,
}

trait CleanupCleaner: Send + Sync {
    fn id(&self) -> &'static str;
    fn revision(&self) -> &'static str;
    fn category(&self) -> CleanupCategory;
    fn executable_aliases(&self) -> &'static [&'static str];
    fn preview(
        &self,
        executable: &ControlledExecutable,
        is_cancelled: &(dyn Fn() -> bool + Sync),
    ) -> CleanerPreview;
    fn execute(
        &self,
        executable: &ControlledExecutable,
        dry_run: bool,
        operation: &OperationGuard,
    ) -> CleanupActionResult;

    fn preview_for_inventory(
        &self,
        inventory: &ApplicationInventory,
        is_cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ScanRuleResult {
        let started = Instant::now();
        let Some(executable) = inventory.executable(self.executable_aliases()) else {
            return self.to_scan_rule(CleanerPreview {
                // A missing executable is definitive only when inventory capture
                // completed. Partial inventory must fail closed as Limited.
                status: if inventory.executable_inventory_complete() {
                    CleanerPreviewStatus::NotApplicable
                } else {
                    CleanerPreviewStatus::Limited
                },
                bytes: 0,
                item_count: 0,
                elapsed_ms: started.elapsed().as_millis() as u64,
            });
        };
        self.to_scan_rule(self.preview(&executable, is_cancelled))
    }

    fn execute_for_inventory(
        &self,
        inventory: &ApplicationInventory,
        dry_run: bool,
        operation: &OperationGuard,
    ) -> CleanupActionResult {
        let Some(executable) = inventory.executable(self.executable_aliases()) else {
            return CleanupActionResult {
                rule_id: self.id().to_string(),
                action_kind: CleanupActionKind::Command,
                status: CleanupActionStatus::Failed,
                reason_code: Some(CleanupActionReason::RequiredToolUnavailable),
                bytes_expected: 0,
                released_bytes: 0,
                affected_item_count: 0,
                failed_item_count: 1,
                running_processes: Vec::new(),
            };
        };
        self.execute(&executable, dry_run, operation)
    }

    fn to_scan_rule(&self, preview: CleanerPreview) -> ScanRuleResult {
        let (available, selectable, status) = match preview.status {
            CleanerPreviewStatus::NotApplicable => (false, false, ScanItemStatus::NotApplicable),
            CleanerPreviewStatus::Limited => (true, false, ScanItemStatus::Limited),
            CleanerPreviewStatus::Ready if preview.bytes == 0 && preview.item_count == 0 => {
                (true, false, ScanItemStatus::Clean)
            }
            CleanerPreviewStatus::Ready => (true, true, ScanItemStatus::Found),
        };
        ScanRuleResult {
            rule_id: self.id().to_string(),
            category: self.category(),
            group: CleanupGroup::from(self.category()),
            risk: RiskLevel::Recoverable,
            default_selected: false,
            recommended_selected: false,
            bytes: preview.bytes,
            file_count: preview.item_count,
            available,
            selectable,
            status,
            running_processes: Vec::new(),
            requires_app_close: false,
            sources: Vec::new(),
            source_count: 0,
            sources_truncated: false,
            scan_elapsed_ms: preview.elapsed_ms,
        }
    }
}

pub(crate) fn preview_all(
    inventory: &ApplicationInventory,
    declared_roots: &[PathBuf],
    project_roots: &[String],
    deep_project_discovery: bool,
    cancellation: &mangodisk_platform::PlatformCancellation,
    report_path: &(dyn Fn(&Path) + Sync),
    report_files: &(dyn Fn(&Path, u64, u64) + Sync),
) -> Vec<ScanRuleResult> {
    #[cfg(not(target_os = "macos"))]
    let _ = declared_roots;
    let is_cancelled = || cancellation.is_cancelled();
    /*
     * Project discovery owns the trusted project-root index used by specialized
     * cleaners. Build that evidence before previewing Rust toolchains, while
     * appending the project-artifact rows in their established UI order below.
     */
    let project_artifacts_started = Instant::now();
    let project_artifact_results = project_artifacts::preview_all(
        project_roots,
        deep_project_discovery,
        &is_cancelled,
        report_path,
        report_files,
    );
    log::debug!(
        "cleanup_cleaner_group_preview_finished group=projectArtifacts elapsed_ms={}",
        project_artifacts_started.elapsed().as_millis()
    );
    let registered_started = Instant::now();
    let mut results = CLEANERS
        .iter()
        .map(|cleaner| cleaner.preview_for_inventory(inventory, &is_cancelled))
        .collect::<Vec<_>>();
    log::debug!(
        "cleanup_cleaner_group_preview_finished group=registered elapsed_ms={}",
        registered_started.elapsed().as_millis()
    );
    let ai_model_started = Instant::now();
    results.extend(ai_model_storage::preview_all(
        &is_cancelled,
        report_path,
        report_files,
    ));
    log::debug!(
        "cleanup_cleaner_group_preview_finished group=aiModel elapsed_ms={}",
        ai_model_started.elapsed().as_millis()
    );
    let archived_sessions_started = Instant::now();
    results.push(codex_archived_sessions::preview(&is_cancelled, report_path));
    log::debug!(
        "cleanup_cleaner_group_preview_finished group=codexArchivedSessions elapsed_ms={}",
        archived_sessions_started.elapsed().as_millis()
    );
    let rust_toolchains_started = Instant::now();
    results.push(rust_toolchains::preview(
        inventory,
        project_roots,
        &is_cancelled,
        report_path,
        report_files,
    ));
    log::debug!(
        "cleanup_cleaner_group_preview_finished group=rustToolchains elapsed_ms={}",
        rust_toolchains_started.elapsed().as_millis()
    );
    let universal_binary_started = Instant::now();
    results.push(macos_universal_binaries::preview(
        inventory,
        &is_cancelled,
        report_path,
    ));
    log::debug!(
        "cleanup_cleaner_group_preview_finished group=universalBinary elapsed_ms={}",
        universal_binary_started.elapsed().as_millis()
    );
    #[cfg(target_os = "macos")]
    {
        let user_cache_started = Instant::now();
        results.push(user_cache_inventory::preview(
            inventory,
            declared_roots,
            &is_cancelled,
            report_path,
            report_files,
        ));
        log::debug!(
            "cleanup_cleaner_group_preview_finished group=userCacheInventory elapsed_ms={}",
            user_cache_started.elapsed().as_millis()
        );
        let xcode_started = Instant::now();
        results.extend(xcode_storage::preview_all(
            inventory,
            &is_cancelled,
            report_path,
        ));
        log::debug!(
            "cleanup_cleaner_group_preview_finished group=xcodeStorage elapsed_ms={}",
            xcode_started.elapsed().as_millis()
        );
    }
    #[cfg(any(windows, target_os = "macos"))]
    {
        let dropbox_started = Instant::now();
        results.push(dropbox_cache::preview(
            &is_cancelled,
            report_path,
            report_files,
        ));
        log::debug!(
            "cleanup_cleaner_group_preview_finished group=dropboxCache elapsed_ms={}",
            dropbox_started.elapsed().as_millis()
        );
    }
    #[cfg(windows)]
    {
        let windows_system_started = Instant::now();
        results.extend(windows_system_cleanup::preview_all(cancellation));
        log::debug!(
            "cleanup_cleaner_group_preview_finished group=windowsSystem elapsed_ms={}",
            windows_system_started.elapsed().as_millis()
        );
    }
    results.extend(project_artifact_results);
    results
}

/// Returns the full registry when the preview worker cannot produce a result.
///
/// Limited entries remain visible but not selectable, so an optional tool
/// failure cannot expand cleanup scope or hide unrelated filesystem rules.
pub(crate) fn preview_limited_all() -> Vec<ScanRuleResult> {
    let mut results = CLEANERS
        .iter()
        .map(|cleaner| {
            cleaner.to_scan_rule(CleanerPreview {
                status: CleanerPreviewStatus::Limited,
                bytes: 0,
                item_count: 0,
                elapsed_ms: 0,
            })
        })
        .collect::<Vec<_>>();
    results.extend(ai_model_storage::preview_limited_all());
    results.push(codex_archived_sessions::limited_rule());
    results.push(rust_toolchains::limited_rule());
    results.push(macos_universal_binaries::limited_rule(0));
    #[cfg(target_os = "macos")]
    {
        results.push(user_cache_inventory::limited_rule());
        results.extend(xcode_storage::preview_limited_all());
    }
    #[cfg(any(windows, target_os = "macos"))]
    {
        results.push(dropbox_cache::limited_rule());
    }
    #[cfg(windows)]
    {
        results.extend(windows_system_cleanup::preview_limited_all());
    }
    results.extend(project_artifacts::preview_limited_all());
    results
}

#[cfg(windows)]
pub(crate) fn preview_windows_previous_installations_with_privileges(
) -> Result<ScanRuleResult, mangodisk_platform::PlatformError> {
    windows_system_cleanup::preview_previous_installations_with_privileges()
}

pub(crate) fn contains(id: &str) -> bool {
    ai_model_storage::contains(id)
        || id == codex_archived_sessions::CLEANER_ID
        || id == rust_toolchains::CLEANER_ID
        || id == macos_universal_binaries::CLEANER_ID
        || cfg!(target_os = "macos") && user_cache_inventory_contains(id)
        || cfg!(target_os = "macos") && xcode_cleaner_contains(id)
        || dropbox_cache_cleaner_contains(id)
        || cfg!(windows) && windows_system_cleaner_contains(id)
        || CLEANERS.iter().any(|cleaner| cleaner.id() == id)
        || project_artifacts::contains(id)
}

/// Returns the queue used by specialized cleaners without changing execution
/// behavior. Project rules remain last because they share one discovery plan;
/// interleaving them would repeat traversal work for every project ecosystem.
pub(crate) fn execution_rule_ids(ids: &[String]) -> Vec<String> {
    ids.iter()
        .filter(|id| !project_artifacts::contains(id))
        .chain(ids.iter().filter(|id| project_artifacts::contains(id)))
        .cloned()
        .collect()
}

#[cfg(target_os = "macos")]
fn xcode_cleaner_contains(id: &str) -> bool {
    xcode_storage::contains(id)
}

#[cfg(not(target_os = "macos"))]
fn xcode_cleaner_contains(_id: &str) -> bool {
    false
}

#[cfg(target_os = "macos")]
fn user_cache_inventory_contains(id: &str) -> bool {
    id == user_cache_inventory::CLEANER_ID
}

#[cfg(not(target_os = "macos"))]
fn user_cache_inventory_contains(_id: &str) -> bool {
    false
}

#[cfg(any(windows, target_os = "macos"))]
fn dropbox_cache_cleaner_contains(id: &str) -> bool {
    id == dropbox_cache::CLEANER_ID
}

#[cfg(not(any(windows, target_os = "macos")))]
fn dropbox_cache_cleaner_contains(_id: &str) -> bool {
    false
}

#[cfg(windows)]
fn windows_system_cleaner_contains(id: &str) -> bool {
    windows_system_cleanup::contains(id)
}

#[cfg(not(windows))]
fn windows_system_cleaner_contains(_id: &str) -> bool {
    false
}

#[cfg(test)]
pub(crate) fn execute_selected(
    ids: &[String],
    inventory: &ApplicationInventory,
    declared_roots: &[PathBuf],
    project_roots: &[String],
    source_selections: &SourceSelectionPolicy,
    dry_run: bool,
    operation: &OperationGuard,
) -> Vec<CleanupActionResult> {
    execute_selected_with_progress(
        CleanerExecutionRequest {
            rule_ids: ids,
            inventory,
            declared_roots,
            project_roots,
            selected_volume_scope: false,
            source_selections,
            dry_run,
            operation,
        },
        |_, _| {},
    )
}

/// Runs platform- or tool-specific cleaners and reports rule boundaries.
///
/// Reporting only at rule boundaries gives the desktop adapter meaningful
/// state without producing per-file events for large directories.
pub(crate) fn execute_selected_with_progress<F>(
    request: CleanerExecutionRequest<'_>,
    mut progress: F,
) -> Vec<CleanupActionResult>
where
    F: FnMut(&str, Option<&CleanupActionResult>),
{
    let CleanerExecutionRequest {
        rule_ids: ids,
        inventory,
        declared_roots,
        project_roots,
        selected_volume_scope,
        source_selections,
        dry_run,
        operation,
    } = request;
    #[cfg(not(target_os = "macos"))]
    let _ = declared_roots;
    let execution_rule_ids = execution_rule_ids(ids);
    let project_start = execution_rule_ids
        .iter()
        .position(|id| project_artifacts::contains(id))
        .unwrap_or(execution_rule_ids.len());
    let (direct_ids, project_ids) = execution_rule_ids.split_at(project_start);
    let mut actions = Vec::with_capacity(ids.len());
    for id in direct_ids {
        progress(id, None);
        // The immediately invoked closure preserves the original early-return
        // structure. Platform-specific branches use `#[cfg]`; a continuous
        // `else if` chain would become an incomplete expression on targets
        // where an intermediate branch is compiled out.
        let action = (|| {
            if operation.ensure_not_cancelled().is_err() {
                #[cfg(windows)]
                let is_windows_recycle_bin = id == windows_system_cleanup::RECYCLE_BIN_ID;
                #[cfg(not(windows))]
                let is_windows_recycle_bin = false;
                let action_kind = if ai_model_storage::contains(id)
                    || id == codex_archived_sessions::CLEANER_ID
                    || dropbox_cache_cleaner_contains(id)
                    || is_windows_recycle_bin
                {
                    CleanupActionKind::Delete
                } else if id == macos_universal_binaries::CLEANER_ID {
                    CleanupActionKind::Optimize
                } else {
                    CleanupActionKind::Command
                };
                return cancelled_action(id, action_kind);
            }
            if ai_model_storage::contains(id) {
                return ai_model_storage::execute(
                    id,
                    source_selections.scope(id),
                    dry_run,
                    operation,
                );
            }
            if id == codex_archived_sessions::CLEANER_ID {
                return codex_archived_sessions::execute(
                    source_selections.scope(id),
                    dry_run,
                    operation,
                );
            }
            if id == rust_toolchains::CLEANER_ID {
                return rust_toolchains::execute(
                    inventory,
                    project_roots,
                    source_selections.scope(id),
                    dry_run,
                    operation,
                );
            }
            if id == macos_universal_binaries::CLEANER_ID {
                return macos_universal_binaries::execute(
                    inventory,
                    source_selections.scope(id),
                    dry_run,
                    operation,
                );
            }
            #[cfg(target_os = "macos")]
            if id == user_cache_inventory::CLEANER_ID {
                return user_cache_inventory::execute(
                    inventory,
                    declared_roots,
                    source_selections.scope(id),
                    dry_run,
                    operation,
                );
            }
            #[cfg(target_os = "macos")]
            if xcode_storage::contains(id) {
                return xcode_storage::execute(
                    id,
                    inventory,
                    source_selections.scope(id),
                    dry_run,
                    operation,
                );
            }
            #[cfg(any(windows, target_os = "macos"))]
            if id == dropbox_cache::CLEANER_ID {
                return dropbox_cache::execute(source_selections.scope(id), dry_run, operation);
            }
            #[cfg(windows)]
            if windows_system_cleanup::contains(id) {
                if source_selections.scope(id).is_some() {
                    return failed_source_selection_action(
                        id,
                        windows_system_cleanup::action_kind(id),
                    );
                }
                return windows_system_cleanup::execute(id, dry_run, operation);
            }
            if source_selections.scope(id).is_some() {
                return failed_source_selection_action(id, CleanupActionKind::Command);
            }
            CLEANERS
                .iter()
                .find(|cleaner| cleaner.id() == id)
                .map(|cleaner| cleaner.execute_for_inventory(inventory, dry_run, operation))
                .unwrap_or_else(|| {
                    // The request is validated before execution. Keep this fail-closed
                    // fallback so a future registry regression cannot panic after other
                    // rules have already completed.
                    log::error!(
                        "cleanup_cleaner_execute_failed reason=registryInvariant rule_id={id}"
                    );
                    CleanupActionResult {
                        rule_id: id.to_string(),
                        action_kind: CleanupActionKind::Command,
                        status: CleanupActionStatus::Failed,
                        reason_code: Some(CleanupActionReason::CleanerUnavailable),
                        bytes_expected: 0,
                        released_bytes: 0,
                        affected_item_count: 0,
                        failed_item_count: 1,
                        running_processes: Vec::new(),
                    }
                })
        })();
        progress(id, Some(&action));
        actions.push(action);
    }
    if operation.ensure_not_cancelled().is_err() {
        for id in project_ids {
            progress(id, None);
            let action = cancelled_action(id, CleanupActionKind::Delete);
            progress(id, Some(&action));
            actions.push(action);
        }
    } else {
        let project_actions = project_artifacts::execute_selected_with_progress(
            project_ids,
            project_roots,
            selected_volume_scope,
            source_selections,
            dry_run,
            operation,
            |rule_id, action| progress(rule_id, action),
        );
        actions.extend(project_actions);
    }
    let order = ids
        .iter()
        .enumerate()
        .map(|(index, id)| (id.as_str(), index))
        .collect::<std::collections::HashMap<_, _>>();
    actions.sort_by_key(|action| {
        order
            .get(action.rule_id.as_str())
            .copied()
            .unwrap_or(usize::MAX)
    });
    actions
}

fn failed_source_selection_action(
    rule_id: &str,
    action_kind: CleanupActionKind,
) -> CleanupActionResult {
    CleanupActionResult {
        rule_id: rule_id.to_string(),
        action_kind,
        status: CleanupActionStatus::Failed,
        reason_code: Some(CleanupActionReason::PreflightFailed),
        bytes_expected: 0,
        released_bytes: 0,
        affected_item_count: 0,
        failed_item_count: 1,
        running_processes: Vec::new(),
    }
}

fn cancelled_action(rule_id: &str, action_kind: CleanupActionKind) -> CleanupActionResult {
    CleanupActionResult {
        rule_id: rule_id.to_string(),
        action_kind,
        status: CleanupActionStatus::Blocked,
        reason_code: Some(CleanupActionReason::Cancelled),
        bytes_expected: 0,
        released_bytes: 0,
        affected_item_count: 0,
        failed_item_count: 1,
        running_processes: Vec::new(),
    }
}

pub(crate) fn count() -> usize {
    #[cfg(target_os = "macos")]
    let platform_cleaner_count = 5;
    #[cfg(windows)]
    let platform_cleaner_count = windows_system_cleanup::count() + 1;
    #[cfg(not(any(target_os = "macos", windows)))]
    let platform_cleaner_count = 0;
    CLEANERS.len()
        + ai_model_storage::count()
        + 2
        + 1
        + platform_cleaner_count
        + project_artifacts::count()
}

/// Produces a stable workload digest for compile-time cleanup cleaners.
pub(crate) fn catalog_digest() -> String {
    let mut cleaners = CLEANERS.to_vec();
    cleaners.sort_by_key(|cleaner| cleaner.id());
    let mut hasher = blake3::Hasher::new();
    for cleaner in cleaners {
        hasher.update(cleaner.id().as_bytes());
        hasher.update(cleaner.revision().as_bytes());
    }
    hasher.update(ai_model_storage::catalog_digest().as_bytes());
    hasher.update(codex_archived_sessions::CLEANER_ID.as_bytes());
    hasher.update(codex_archived_sessions::CLEANER_REVISION.as_bytes());
    hasher.update(rust_toolchains::CLEANER_ID.as_bytes());
    hasher.update(rust_toolchains::CLEANER_REVISION.as_bytes());
    hasher.update(macos_universal_binaries::CLEANER_ID.as_bytes());
    hasher.update(macos_universal_binaries::CLEANER_REVISION.as_bytes());
    #[cfg(target_os = "macos")]
    {
        hasher.update(dropbox_cache::CLEANER_ID.as_bytes());
        hasher.update(dropbox_cache::CLEANER_REVISION.as_bytes());
        hasher.update(user_cache_inventory::CLEANER_ID.as_bytes());
        hasher.update(user_cache_inventory::CLEANER_REVISION.as_bytes());
        hasher.update(xcode_storage::DEVICE_SUPPORT_ID.as_bytes());
        hasher.update(xcode_storage::SIMULATOR_RUNTIME_ID.as_bytes());
        hasher.update(xcode_storage::ARCHIVES_ID.as_bytes());
        hasher.update(xcode_storage::CLEANER_REVISION.as_bytes());
    }
    #[cfg(windows)]
    {
        hasher.update(dropbox_cache::CLEANER_ID.as_bytes());
        hasher.update(dropbox_cache::CLEANER_REVISION.as_bytes());
        hasher.update(windows_system_cleanup::catalog_digest().as_bytes());
    }
    hasher.update(project_artifacts::catalog_digest().as_bytes());
    hasher.finalize().to_hex().to_string()
}

#[cfg(test)]
mod tests {
    #[test]
    fn project_app_closure_accepts_only_known_catalog_ids_and_fixed_apps() {
        assert_eq!(
            super::project_artifact_close_processes("project.rust-build-artifacts"),
            Some(vec!["Codex".into(), "ChatGPT".into()])
        );
        assert!(super::project_artifact_close_processes("project.unknown").is_none());
        assert!(super::project_artifact_close_processes("node").is_none());
        assert!(super::project_artifact_close_processes("app.chatgpt-cache").is_none());
    }
    use super::*;

    #[test]
    fn registry_accepts_only_compile_time_cleaner_ids() {
        assert!(count() >= 21);
        assert!(contains("special.conda-cache"));
        assert!(contains("special.docker-build-cache"));
        assert!(contains("special.ai-model-hugging-face"));
        assert!(contains("special.ai-model-ollama"));
        assert!(contains(codex_archived_sessions::CLEANER_ID));
        assert!(contains(rust_toolchains::CLEANER_ID));
        assert!(contains(macos_universal_binaries::CLEANER_ID));
        assert!(contains("project.rust-build-artifacts"));
        assert!(!contains("special.unknown"));
        assert!(!contains("docker builder prune --all"));
        assert_eq!(catalog_digest().len(), 64);
    }

    #[test]
    fn execution_queue_keeps_shared_project_discovery_last() {
        let selected = vec![
            "project.rust-build-artifacts".to_string(),
            "special.docker-build-cache".to_string(),
            "project.node-build-artifacts".to_string(),
            "special.conda-cache".to_string(),
        ];

        assert_eq!(
            execution_rule_ids(&selected),
            vec![
                "special.docker-build-cache",
                "special.conda-cache",
                "project.rust-build-artifacts",
                "project.node-build-artifacts",
            ]
        );
    }

    #[test]
    fn zero_byte_metadata_items_remain_visible_and_selectable() {
        let rule = CONDA_CACHE_CLEANER.to_scan_rule(CleanerPreview {
            status: CleanerPreviewStatus::Ready,
            bytes: 0,
            item_count: 2,
            elapsed_ms: 1,
        });
        assert_eq!(rule.status, ScanItemStatus::Found);
        assert!(rule.selectable);
    }

    /// This ignored test uses Docker's read-only preview and a MangoDisk dry run.
    /// It never invokes prune.
    #[test]
    #[ignore = "requires Docker with a running local daemon"]
    fn real_docker_preview_and_dry_run_do_not_delete() {
        let _operation_lock = crate::shared::operation::test_operation_lock();
        let context = crate::applications::catalog::ScanContext::capture();
        let cancellation = mangodisk_platform::PlatformCancellation::new(|| false);
        let rules = preview_all(
            &context.inventory,
            &[],
            &[],
            false,
            &cancellation,
            &|_| {},
            &|_, _, _| {},
        );
        let rule = rules
            .iter()
            .find(|rule| rule.rule_id == "special.docker-build-cache")
            .expect("the cleanup cleaner registry must include Docker build cache");
        assert!(rule.available, "Docker must be available when installed");
        assert_ne!(
            rule.status,
            ScanItemStatus::Limited,
            "a real preview must not degrade"
        );

        let operation =
            OperationGuard::start(crate::shared::operation::CoordinatedOperationKind::Cleanup)
                .expect("the dry run should start");
        let actions = execute_selected(
            &["special.docker-build-cache".to_string()],
            &context.inventory,
            &[],
            &[],
            &SourceSelectionPolicy::empty(),
            true,
            &operation,
        );
        operation.complete();
        let action = &actions[0];

        assert_eq!(action.status, CleanupActionStatus::Previewed);
        assert_eq!(action.bytes_expected, rule.bytes);
        assert_eq!(action.released_bytes, 0);
        assert_eq!(action.affected_item_count, 0);
        println!(
            "Docker read-only preview: records={}, reclaimable_bytes={}, elapsed_ms={}",
            rule.file_count, rule.bytes, rule.scan_elapsed_ms
        );
    }

    /// This ignored test uses Conda's official JSON dry run and a MangoDisk dry
    /// run. It never invokes the mutating Conda command.
    #[test]
    #[ignore = "requires Conda installed on the test host"]
    fn real_conda_preview_and_dry_run_do_not_delete() {
        let _operation_lock = crate::shared::operation::test_operation_lock();
        let context = crate::applications::catalog::ScanContext::capture();
        let cancellation = mangodisk_platform::PlatformCancellation::new(|| false);
        let rules = preview_all(
            &context.inventory,
            &[],
            &[],
            false,
            &cancellation,
            &|_| {},
            &|_, _, _| {},
        );
        let rule = rules
            .iter()
            .find(|rule| rule.rule_id == "special.conda-cache")
            .expect("the cleanup cleaner registry must include Conda cache");
        assert!(rule.available, "Conda must be available when installed");
        assert_ne!(
            rule.status,
            ScanItemStatus::Limited,
            "the official Conda dry run must not degrade"
        );

        let operation =
            OperationGuard::start(crate::shared::operation::CoordinatedOperationKind::Cleanup)
                .expect("the dry run should start");
        let actions = execute_selected(
            &["special.conda-cache".to_string()],
            &context.inventory,
            &[],
            &[],
            &SourceSelectionPolicy::empty(),
            true,
            &operation,
        );
        operation.complete();
        let action = &actions[0];

        assert_eq!(action.status, CleanupActionStatus::Previewed);
        assert_eq!(action.bytes_expected, rule.bytes);
        assert_eq!(action.released_bytes, 0);
        assert_eq!(action.affected_item_count, 0);
        println!(
            "Conda read-only preview: records={}, reclaimable_bytes={}, elapsed_ms={}",
            rule.file_count, rule.bytes, rule.scan_elapsed_ms
        );
    }
}
