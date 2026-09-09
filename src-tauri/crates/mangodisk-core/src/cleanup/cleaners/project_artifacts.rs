use std::{
    cmp::Reverse,
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Mutex, OnceLock,
    },
    thread,
    time::Instant,
};

use mangodisk_platform::{current_platform, DirectoryTreeAggregateError, Platform};
#[cfg(not(test))]
use mangodisk_platform::{
    ProjectMarkerCandidateQuery, ProjectMarkerCandidateScanError, ScanDeviceClass, UserDirectories,
};

use crate::{
    cleanup::codex_worktrees,
    cleanup::measurement::MeasureResult,
    cleanup::{
        source_selection::{SourceScope, SourceSelectionPolicy},
        CleanupActionKind, CleanupActionReason, CleanupActionResult, CleanupActionStatus,
        CleanupSourceDetail, RiskLevel, ScanItemStatus, ScanRuleResult,
    },
    filesystem::{
        metadata::{diagnostic_path, display_path, is_link_like, latest_timestamp, modified_ms},
        permanent_delete::{delete_path_permanently, prepare_path_for_permanent_delete},
    },
    shared::operation::OperationGuard,
};

use super::project_artifact_schema::{
    parse_catalog, ProjectArtifactRuleSource, ProjectArtifactSource, ProjectMatchSource,
};
use super::project_root_index;

include!(concat!(
    env!("OUT_DIR"),
    "/embedded-project-artifact-rules.rs"
));

const MAX_CONFIGURED_ROOTS: usize = 32;
#[cfg(not(test))]
const MAX_STANDARD_ROOTS: usize = 10_000;
#[cfg(not(test))]
const MAX_DEEP_ROOTS: usize = 10_000;
const MAX_DISCOVERY_DEPTH: usize = 64;
const MAX_DISCOVERED_DIRECTORIES: usize = 500_000;
const MAX_DISCOVERED_PROJECTS: usize = 50_000;
const MAX_STANDARD_PROJECT_ROOTS: usize = 96;
const MEASUREMENT_WORKER_LIMIT: usize = 4;
const PROGRESS_FILE_BATCH_SIZE: u64 = 128;
const ALWAYS_SKIPPED_DIRECTORIES: &[&str] = &[".git", ".hg", ".svn"];
const DEEP_HOME_EXCLUSIONS: &[&str] = &["Applications", "Library"];
const DEEP_SYSTEM_VOLUME_EXCLUSIONS: &[&str] = &[
    "$Recycle.Bin",
    "Applications",
    "Library",
    "Program Files",
    "Program Files (x86)",
    "ProgramData",
    "Recovery",
    "System",
    "System Volume Information",
    "Users",
    "Windows",
    "bin",
    "cores",
    "dev",
    "private",
    "sbin",
    "usr",
];
const DEEP_ALWAYS_SKIPPED_DIRECTORIES: &[&str] = &[
    "$Recycle.Bin",
    "Program Files",
    "Program Files (x86)",
    "ProgramData",
    "Recovery",
    "System Volume Information",
    "Windows",
];

static CURRENT_PLATFORM_RULES: OnceLock<Result<Vec<ProjectArtifactRuleSource>, String>> =
    OnceLock::new();

#[derive(Debug, Clone)]
struct ProjectMatch {
    rule_index: usize,
    project_root: PathBuf,
    allow_descendant_scan: bool,
}

#[derive(Debug, Clone)]
struct ArtifactDraft {
    rule_index: usize,
    project_root: PathBuf,
    path: PathBuf,
}

#[derive(Debug, Clone)]
struct ArtifactCandidate {
    project_root: PathBuf,
    codex_checkout: Option<PathBuf>,
    path: PathBuf,
    bytes: u64,
    file_count: u64,
    modified_at_ms: Option<u64>,
    measurement_limited: bool,
}

#[derive(Debug)]
struct RulePlan {
    source: ProjectArtifactRuleSource,
    candidates: Vec<ArtifactCandidate>,
}

#[derive(Debug)]
struct CatalogPlan {
    rules: Vec<RulePlan>,
    limited: bool,
    elapsed_ms: u64,
}

#[derive(Debug, Default)]
struct CleanupSourceSummary {
    sources: Vec<CleanupSourceDetail>,
    source_count: u64,
}

#[derive(Default)]
struct ArtifactMeasurement {
    measured: MeasureResult,
    modified_at_ms: Option<u64>,
}

#[derive(Debug)]
struct DirectoryEntries {
    file_names: Vec<String>,
    directories: Vec<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProjectRootMode {
    Explicit,
    Standard,
    Deep,
    SelectedVolumes,
}

#[derive(Debug, Default)]
struct ProjectDiscoveryRoots {
    exact_roots: Vec<PathBuf>,
    recursive_roots: Vec<PathBuf>,
}

impl ProjectDiscoveryRoots {
    #[cfg(not(test))]
    fn extend(&mut self, other: Self) {
        self.exact_roots.extend(other.exact_roots);
        self.recursive_roots.extend(other.recursive_roots);
    }

    fn root_count(&self) -> usize {
        self.exact_roots.len() + self.recursive_roots.len()
    }

    fn is_empty(&self) -> bool {
        self.exact_roots.is_empty() && self.recursive_roots.is_empty()
    }
}

#[cfg(not(test))]
enum IndexedProjectRootOutcome {
    Indexed {
        exact_roots: Vec<PathBuf>,
        fallback_roots: Vec<PathBuf>,
    },
    Fallback,
    Cancelled,
}

#[cfg(not(test))]
struct IndexedProjectRootRequest<'a> {
    search_root: &'a Path,
    allowed_roots: &'a [PathBuf],
    file_names: &'a [String],
    file_suffixes: &'a [String],
    prune_names: &'a HashSet<String>,
    is_cancelled: &'a (dyn Fn() -> bool + Sync),
    report_path: &'a (dyn Fn(&Path) + Sync),
    report_files: &'a (dyn Fn(&Path, u64, u64) + Sync),
}

pub(super) fn preview_all(
    configured_roots: &[String],
    deep_project_discovery: bool,
    is_cancelled: &(dyn Fn() -> bool + Sync),
    report_path: &(dyn Fn(&Path) + Sync),
    report_files: &(dyn Fn(&Path, u64, u64) + Sync),
) -> Vec<ScanRuleResult> {
    let rules = match current_platform_rules() {
        Ok(rules) => rules,
        Err(error) => {
            log::error!(
                "project_artifact_catalog_load_failed error_digest={}",
                blake3::hash(error.as_bytes()).to_hex()
            );
            return Vec::new();
        }
    };
    match build_plan_with_progress(
        configured_roots,
        deep_project_discovery,
        rules,
        is_cancelled,
        report_path,
        report_files,
        configured_codex_home().as_deref(),
    ) {
        Ok(plan) => {
            let processes = plan
                .rules
                .iter()
                .flat_map(|rule| &rule.candidates)
                .any(|candidate| candidate.codex_checkout.is_some())
                .then(codex_worktrees::blocking_processes);
            plan.rules
                .iter()
                .map(|rule| {
                    let complete_bytes: u64 = rule
                        .candidates
                        .iter()
                        .filter(|candidate| !candidate.measurement_limited)
                        .map(|candidate| candidate.bytes)
                        .sum();
                    let complete_file_count: u64 = rule
                        .candidates
                        .iter()
                        .filter(|candidate| !candidate.measurement_limited)
                        .map(|candidate| candidate.file_count)
                        .sum();
                    let limited_bytes: u64 = rule
                        .candidates
                        .iter()
                        .filter(|candidate| candidate.measurement_limited)
                        .map(|candidate| candidate.bytes)
                        .sum();
                    let limited_file_count: u64 = rule
                        .candidates
                        .iter()
                        .filter(|candidate| candidate.measurement_limited)
                        .map(|candidate| candidate.file_count)
                        .sum();
                    let status = if plan.limited {
                        ScanItemStatus::Limited
                    } else if complete_bytes > 0 {
                        ScanItemStatus::Found
                    } else if limited_bytes > 0 {
                        ScanItemStatus::Limited
                    } else {
                        ScanItemStatus::Clean
                    };
                    // A mixed rule exposes only fully measured bytes as
                    // reclaimable. If every candidate is limited, preserve the
                    // accessible estimate for the inspection report.
                    let (bytes, file_count) = if status == ScanItemStatus::Found {
                        (complete_bytes, complete_file_count)
                    } else {
                        (
                            complete_bytes.saturating_add(limited_bytes),
                            complete_file_count.saturating_add(limited_file_count),
                        )
                    };
                    let mut result = scan_result(
                        &rule.source,
                        status,
                        bytes,
                        file_count,
                        plan.elapsed_ms / plan.rules.len().max(1) as u64,
                        cleanup_source_details(&rule.candidates),
                    );
                    apply_codex_process_guard(&mut result, &rule.candidates, processes.as_ref());
                    result
                })
                .collect()
        }
        Err(error) => {
            log::warn!(
                "project_artifact_preview_failed error_digest={}",
                blake3::hash(error.as_bytes()).to_hex()
            );
            rules
                .iter()
                .map(|rule| {
                    scan_result(
                        rule,
                        ScanItemStatus::Limited,
                        0,
                        0,
                        0,
                        CleanupSourceSummary::default(),
                    )
                })
                .collect()
        }
    }
}

pub(super) fn preview_limited_all() -> Vec<ScanRuleResult> {
    current_platform_rules()
        .unwrap_or_default()
        .iter()
        .map(|rule| {
            scan_result(
                rule,
                ScanItemStatus::Limited,
                0,
                0,
                0,
                CleanupSourceSummary::default(),
            )
        })
        .collect()
}

pub(super) fn contains(id: &str) -> bool {
    current_platform_rules().is_ok_and(|rules| rules.iter().any(|rule| rule.id == id))
}

pub(super) fn count() -> usize {
    current_platform_rules().map_or(0, <[_]>::len)
}

pub(super) fn catalog_digest() -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"mangodisk-project-artifact-catalog-v3-codex-worktrees");
    for (name, source) in EMBEDDED_PROJECT_ARTIFACT_RULE_SOURCES {
        hasher.update(name.as_bytes());
        hasher.update(source.as_bytes());
    }
    hasher.finalize().to_hex().to_string()
}

#[cfg(test)]
pub(super) fn execute_selected(
    selected_ids: &[String],
    configured_roots: &[String],
    source_selections: &SourceSelectionPolicy,
    dry_run: bool,
    operation: &OperationGuard,
) -> Vec<CleanupActionResult> {
    execute_selected_with_progress(
        selected_ids,
        configured_roots,
        false,
        source_selections,
        dry_run,
        operation,
        |_, _| {},
    )
}

/// Executes project rules sequentially and reports the boundary of the rule
/// that is actually running. Project discovery remains shared because
/// rebuilding the same catalog for every selected ecosystem would multiply
/// disk traversal cost.
pub(super) fn execute_selected_with_progress<F>(
    selected_ids: &[String],
    configured_roots: &[String],
    selected_volume_scope: bool,
    source_selections: &SourceSelectionPolicy,
    dry_run: bool,
    operation: &OperationGuard,
    mut progress: F,
) -> Vec<CleanupActionResult>
where
    F: FnMut(&str, Option<&CleanupActionResult>),
{
    if selected_ids.is_empty() {
        return Vec::new();
    }
    let rules = match current_platform_rules() {
        Ok(rules) => rules,
        Err(error) => {
            log::error!(
                "project_artifact_execute_failed reason=catalogLoad error_digest={}",
                blake3::hash(error.as_bytes()).to_hex()
            );
            return failed_actions_with_progress(
                selected_ids,
                CleanupActionReason::CleanerUnavailable,
                &mut progress,
            );
        }
    };
    let plan = match build_plan(configured_roots, selected_volume_scope, rules, &|| {
        operation.cancelled().load(Ordering::Relaxed)
    }) {
        Ok(plan) if !plan.limited => plan,
        Ok(_) => {
            return failed_actions_with_progress(
                selected_ids,
                CleanupActionReason::PreflightFailed,
                &mut progress,
            )
        }
        Err(error) => {
            log::warn!(
                "project_artifact_execute_failed reason=preflight error_digest={}",
                blake3::hash(error.as_bytes()).to_hex()
            );
            return failed_actions_with_progress(
                selected_ids,
                CleanupActionReason::PreflightFailed,
                &mut progress,
            );
        }
    };
    let plans_by_id = plan
        .rules
        .iter()
        .map(|rule| (rule.source.id.as_str(), rule))
        .collect::<HashMap<_, _>>();
    selected_ids
        .iter()
        .map(|id| {
            progress(id, None);
            let action = plans_by_id.get(id.as_str()).map_or_else(
                || failed_action(id, CleanupActionReason::CleanerUnavailable),
                |rule| execute_rule(rule, source_selections.scope(id), dry_run, operation),
            );
            progress(id, Some(&action));
            action
        })
        .collect()
}

fn failed_actions_with_progress<F>(
    selected_ids: &[String],
    reason: CleanupActionReason,
    progress: &mut F,
) -> Vec<CleanupActionResult>
where
    F: FnMut(&str, Option<&CleanupActionResult>),
{
    selected_ids
        .iter()
        .map(|id| {
            progress(id, None);
            let action = failed_action(id, reason);
            progress(id, Some(&action));
            action
        })
        .collect()
}

fn execute_rule(
    rule: &RulePlan,
    source_scope: Option<&SourceScope>,
    dry_run: bool,
    operation: &OperationGuard,
) -> CleanupActionResult {
    execute_rule_with_process_check(
        rule,
        source_scope,
        dry_run,
        operation,
        &codex_worktrees::blocking_processes,
    )
}

// Only process discovery is injectable. Fixtures still exercise real Git
// validation, filesystem preflight, deletion, and result aggregation.
fn execute_rule_with_process_check(
    rule: &RulePlan,
    source_scope: Option<&SourceScope>,
    dry_run: bool,
    operation: &OperationGuard,
    process_check: &dyn Fn() -> Result<Vec<String>, String>,
) -> CleanupActionResult {
    if source_scope.is_some_and(|scope| {
        scope
            .validate_known_paths(
                rule.candidates
                    .iter()
                    .map(|candidate| candidate.path.as_path()),
            )
            .is_err()
    }) {
        return failed_action(&rule.source.id, CleanupActionReason::PreflightFailed);
    }
    let candidates = rule
        .candidates
        .iter()
        // Partial measurements remain visible in preview diagnostics but can
        // never enter dry-run or real execution, even if an adapter submits a
        // forged source selection.
        .filter(|candidate| !candidate.measurement_limited)
        .filter(|candidate| source_scope.is_none_or(|scope| scope.selects(&candidate.path)))
        .collect::<Vec<_>>();
    let bytes_expected = candidates.iter().map(|candidate| candidate.bytes).sum();
    if dry_run {
        return CleanupActionResult {
            rule_id: rule.source.id.clone(),
            action_kind: CleanupActionKind::Delete,
            status: CleanupActionStatus::Previewed,
            reason_code: None,
            bytes_expected,
            released_bytes: 0,
            affected_item_count: 0,
            failed_item_count: 0,
            running_processes: Vec::new(),
        };
    }

    let mut released_bytes = 0_u64;
    let mut affected_item_count = 0_u64;
    let mut failed_item_count = 0_u64;
    let mut blocked_item_count = 0_u64;
    let mut preflight_failed_count = 0_u64;
    let mut running_processes = Vec::<String>::new();
    for candidate in candidates {
        if operation.cancelled().load(Ordering::Relaxed) {
            failed_item_count = failed_item_count.saturating_add(1);
            break;
        }
        let prepared = match prepare_path_for_permanent_delete(&candidate.path) {
            Ok(prepared) => prepared,
            Err(error) => {
                failed_item_count = failed_item_count.saturating_add(1);
                log::warn!(
                    "project_artifact_delete_skipped rule_id={} path={} reason=identityCaptureFailed error_digest={}",
                    rule.source.id,
                    diagnostic_path(&candidate.path),
                    blake3::hash(error.to_string().as_bytes()).to_hex()
                );
                continue;
            }
        };
        if !project_matches(&candidate.project_root, &rule.source.project_match)
            || validate_candidate(&candidate.project_root, &candidate.path).is_err()
        {
            failed_item_count = failed_item_count.saturating_add(1);
            log::warn!(
                "project_artifact_delete_skipped rule_id={} path={} reason=revalidationFailed",
                rule.source.id,
                diagnostic_path(&candidate.path)
            );
            continue;
        }
        let live = measure_directory(&candidate.path, &|| {
            operation.cancelled().load(Ordering::Relaxed)
        });
        if live.measured.skipped_count > 0 {
            failed_item_count = failed_item_count.saturating_add(1);
            log::warn!(
                "project_artifact_delete_skipped rule_id={} path={} reason=incompleteMeasurement skipped_count={}",
                rule.source.id,
                diagnostic_path(&candidate.path),
                live.measured.skipped_count
            );
            continue;
        }
        // Revalidate provenance and writers after measurement, immediately
        // before deletion. A stale preview cannot authorize an active checkout.
        if let Some(checkout) = &candidate.codex_checkout {
            if !codex_worktrees::is_linked_checkout(checkout) {
                failed_item_count = failed_item_count.saturating_add(1);
                preflight_failed_count += 1;
                log::warn!(
                    "project_artifact_delete_skipped operation_id={} rule_id={} reason=codexCheckoutChanged",
                    operation.id(), rule.source.id
                );
                continue;
            }
            match process_check() {
                Ok(names) if names.is_empty() => {}
                Ok(names) => {
                    failed_item_count += 1;
                    blocked_item_count += 1;
                    for name in names {
                        if !running_processes
                            .iter()
                            .any(|existing| existing.eq_ignore_ascii_case(&name))
                        {
                            running_processes.push(name);
                        }
                    }
                    log::info!(
                        "project_artifact_delete_skipped operation_id={} rule_id={} reason=codexWritersRunning process_count={}",
                        operation.id(), rule.source.id, running_processes.len()
                    );
                    continue;
                }
                Err(error) => {
                    failed_item_count += 1;
                    preflight_failed_count += 1;
                    log::warn!(
                        "project_artifact_delete_skipped operation_id={} rule_id={} reason=codexProcessInspectionFailed error_digest={}",
                        operation.id(), rule.source.id, blake3::hash(error.as_bytes()).to_hex()
                    );
                    continue;
                }
            }
        }
        match delete_path_permanently(prepared, live.measured.bytes, live.measured.file_count) {
            Ok(()) => {
                released_bytes = released_bytes.saturating_add(live.measured.bytes);
                affected_item_count = affected_item_count.saturating_add(live.measured.file_count);
            }
            Err(error) => {
                released_bytes = released_bytes.saturating_add(error.released_bytes());
                affected_item_count =
                    affected_item_count.saturating_add(error.affected_item_count());
                failed_item_count = failed_item_count.saturating_add(1);
                log::warn!(
                    "project_artifact_delete_failed rule_id={} path={} partial={} released_bytes={} affected_item_count={} error_digest={}",
                    rule.source.id,
                    diagnostic_path(&candidate.path),
                    error.is_partial(),
                    error.released_bytes(),
                    error.affected_item_count(),
                    blake3::hash(error.to_string().as_bytes()).to_hex()
                );
            }
        }
    }
    let cancelled = operation.cancelled().load(Ordering::Relaxed);
    let status = if failed_item_count == 0 {
        CleanupActionStatus::Completed
    } else if released_bytes > 0 || affected_item_count > 0 {
        CleanupActionStatus::Partial
    } else if !cancelled && blocked_item_count == failed_item_count {
        CleanupActionStatus::Blocked
    } else {
        CleanupActionStatus::Failed
    };
    log::info!(
        "project_artifact_execution_finished operation_id={} rule_id={} status={:?} released_bytes={} affected_item_count={} failed_item_count={} blocked_item_count={} preflight_failed_count={} process_count={}",
        operation.id(), rule.source.id, status, released_bytes, affected_item_count,
        failed_item_count, blocked_item_count, preflight_failed_count, running_processes.len()
    );
    CleanupActionResult {
        rule_id: rule.source.id.clone(),
        action_kind: CleanupActionKind::Delete,
        status,
        reason_code: if cancelled {
            Some(CleanupActionReason::Cancelled)
        } else if failed_item_count > 0 && blocked_item_count == failed_item_count {
            Some(CleanupActionReason::RunningProcesses)
        } else if failed_item_count > 0 && preflight_failed_count == failed_item_count {
            Some(CleanupActionReason::PreflightFailed)
        } else {
            (failed_item_count > 0).then_some(CleanupActionReason::ItemsSkipped)
        },
        bytes_expected,
        released_bytes,
        affected_item_count,
        failed_item_count,
        running_processes,
    }
}

fn build_plan(
    configured_roots: &[String],
    deep_project_discovery: bool,
    rules: &[ProjectArtifactRuleSource],
    is_cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<CatalogPlan, String> {
    build_plan_with_progress(
        configured_roots,
        deep_project_discovery,
        rules,
        is_cancelled,
        &|_| {},
        &|_, _, _| {},
        configured_codex_home().as_deref(),
    )
}

fn build_plan_with_progress(
    configured_roots: &[String],
    deep_project_discovery: bool,
    rules: &[ProjectArtifactRuleSource],
    is_cancelled: &(dyn Fn() -> bool + Sync),
    report_path: &(dyn Fn(&Path) + Sync),
    report_files: &(dyn Fn(&Path, u64, u64) + Sync),
    codex_home: Option<&Path>,
) -> Result<CatalogPlan, String> {
    let started = Instant::now();
    let mode = if deep_project_discovery && !configured_roots.is_empty() {
        ProjectRootMode::SelectedVolumes
    } else if !configured_roots.is_empty() {
        ProjectRootMode::Explicit
    } else if deep_project_discovery {
        ProjectRootMode::Deep
    } else {
        ProjectRootMode::Standard
    };
    let root_discovery_started = Instant::now();
    let mut roots = match mode {
        ProjectRootMode::Explicit => ProjectDiscoveryRoots {
            exact_roots: Vec::new(),
            recursive_roots: normalize_roots(configured_roots)?,
        },
        ProjectRootMode::Standard => automatic_project_roots(
            rules,
            ProjectRootMode::Standard,
            &[],
            is_cancelled,
            report_path,
            report_files,
        )?,
        ProjectRootMode::Deep => automatic_project_roots(
            rules,
            ProjectRootMode::Deep,
            &[],
            is_cancelled,
            report_path,
            report_files,
        )?,
        ProjectRootMode::SelectedVolumes => {
            let selected_volume_roots = normalize_roots(configured_roots)?;
            automatic_project_roots(
                rules,
                ProjectRootMode::SelectedVolumes,
                &selected_volume_roots,
                is_cancelled,
                report_path,
                report_files,
            )?
        }
    };
    // The regular home discovery intentionally excludes hidden application
    // state. Add only positively identified linked checkouts, not `.codex`.
    // Explicit project requests keep their original, user-selected scope.
    let codex_roots = if mode != ProjectRootMode::Explicit {
        let checkouts = automatic_codex_worktrees(codex_home, is_cancelled);
        roots.recursive_roots.extend(checkouts.iter().cloned());
        checkouts
    } else {
        Vec::new()
    };
    let root_discovery_elapsed_ms = root_discovery_started.elapsed().as_millis();
    if roots.is_empty() {
        log::info!(
            "project_artifact_plan_built root_mode={} root_count=0 project_count=0 candidate_count=0 limited=false elapsed_ms={}",
            root_mode_name(mode),
            started.elapsed().as_millis()
        );
        return Ok(CatalogPlan {
            rules: rules
                .iter()
                .cloned()
                .map(|source| RulePlan {
                    source,
                    candidates: Vec::new(),
                })
                .collect(),
            limited: false,
            elapsed_ms: started.elapsed().as_millis() as u64,
        });
    }
    let automatic = mode != ProjectRootMode::Explicit;
    let project_discovery_started = Instant::now();
    // Native marker scans already identify the exact directory that owns a project marker.
    // Re-scanning an ancestor recursively discarded that information and repeated most directory
    // I/O. Exact roots only need rule validation in their own directory; recursive traversal is
    // reserved for explicit roots and roots where the platform fast path was unavailable.
    let (mut projects, exact_limited) = discover_projects(
        &roots.exact_roots,
        rules,
        automatic,
        mode != ProjectRootMode::Explicit,
        0,
        is_cancelled,
        report_path,
    )?;
    let (recursive_projects, recursive_limited) = discover_projects(
        &roots.recursive_roots,
        rules,
        automatic,
        mode != ProjectRootMode::Explicit,
        MAX_DISCOVERY_DEPTH,
        is_cancelled,
        report_path,
    )?;
    projects.extend(recursive_projects);
    sort_and_deduplicate_project_matches(&mut projects);
    let discovery_limited = exact_limited || recursive_limited;
    let project_discovery_elapsed_ms = project_discovery_started.elapsed().as_millis();
    let cached_projects_started = Instant::now();
    if matches!(
        mode,
        ProjectRootMode::Standard | ProjectRootMode::SelectedVolumes
    ) {
        projects.extend(cached_project_matches(rules, is_cancelled, report_path)?);
        sort_and_deduplicate_project_matches(&mut projects);
    }
    let cached_projects_elapsed_ms = cached_projects_started.elapsed().as_millis();
    /*
     * Both automatic discovery and explicitly configured roots are validated
     * by Core before reaching this point. Persist every complete discovery so
     * specialized cleaners can protect project-owned runtimes during execute,
     * where adapter-supplied roots are intentionally discarded.
     */
    if !discovery_limited && !is_cancelled() {
        update_project_root_index(&projects);
    }
    if mode == ProjectRootMode::Standard {
        // Retain old verified Codex checkouts independently of the recent-
        // project cap: abandoned worktrees are useful cleanup candidates.
        let codex_projects = projects
            .iter()
            .filter(|project| {
                codex_roots.iter().any(|root| {
                    current_platform().path_is_same_or_child(&project.project_root, root)
                })
            })
            .cloned()
            .collect::<Vec<_>>();
        retain_recent_project_matches(&mut projects, MAX_STANDARD_PROJECT_ROOTS);
        projects.extend(codex_projects);
        sort_and_deduplicate_project_matches(&mut projects);
    }
    let draft_collection_started = Instant::now();
    let drafts = collect_artifact_drafts(&projects, rules, is_cancelled, report_path);
    let draft_collection_elapsed_ms = draft_collection_started.elapsed().as_millis();
    let discovered_draft_count = drafts.len();
    let drafts = deduplicate_artifacts(drafts);
    let suppressed_draft_count = discovered_draft_count.saturating_sub(drafts.len());
    let measurement_started = Instant::now();
    let candidates = measure_artifacts(drafts, is_cancelled, report_path, report_files);
    let measurement_elapsed_ms = measurement_started.elapsed().as_millis();
    let mut candidates_by_rule = vec![Vec::new(); rules.len()];
    // Match the canonical project paths, including fixed macOS system aliases.
    // Failure keeps the lexical boundary rather than discarding protection.
    let codex_home = codex_home.map(|home| {
        current_platform()
            .canonicalize_no_links(home)
            .unwrap_or_else(|_| home.to_path_buf())
    });
    let mut protected_projects = HashMap::new();
    let limited = discovery_limited;
    for (draft, measured) in candidates {
        let measurement_limited = measured.measured.skipped_count > 0;
        if measurement_limited {
            // One unreadable descendant must not hide a large, otherwise
            // measurable build directory. Preserve the accessible estimate,
            // but block only this candidate so complete sibling projects can
            // remain available through the same declarative rule.
            log::warn!(
                "project_artifact_measurement_incomplete rule_id={} path={} skipped_count={}",
                rules[draft.rule_index].id,
                diagnostic_path(&draft.path),
                measured.measured.skipped_count
            );
        }
        if measured.measured.bytes == 0 && measured.measured.file_count == 0 {
            continue;
        }
        candidates_by_rule[draft.rule_index].push(ArtifactCandidate {
            // Discovery provenance is a fast path, not the safety boundary.
            // Native/index results need protection even when Codex discovery
            // failed. Cache classification per project across artifact rules.
            codex_checkout: protected_projects
                .entry(draft.project_root.clone())
                .or_insert_with(|| {
                    codex_roots
                        .iter()
                        .find(|root| {
                            current_platform().path_is_same_or_child(&draft.project_root, root)
                        })
                        .cloned()
                        .or_else(|| {
                            codex_worktrees::protected_checkout(
                                &draft.project_root,
                                codex_home.as_deref(),
                            )
                        })
                })
                .clone(),
            project_root: draft.project_root,
            modified_at_ms: measured.modified_at_ms,
            path: draft.path,
            bytes: measured.measured.bytes,
            file_count: measured.measured.file_count,
            measurement_limited,
        });
    }
    let plans = rules
        .iter()
        .cloned()
        .zip(candidates_by_rule)
        .map(|(source, mut candidates)| {
            candidates.sort_by_cached_key(|candidate| path_identity_key(&candidate.path));
            RulePlan { source, candidates }
        })
        .collect::<Vec<_>>();
    log::info!(
        "project_artifact_plan_built root_mode={} root_count={} project_count={} candidate_count={} suppressed_candidate_count={} limited={} root_discovery_elapsed_ms={} project_discovery_elapsed_ms={} cached_projects_elapsed_ms={} draft_collection_elapsed_ms={} measurement_elapsed_ms={} elapsed_ms={}",
        root_mode_name(mode),
        roots.root_count(),
        projects.len(),
        plans.iter().map(|rule| rule.candidates.len()).sum::<usize>(),
        suppressed_draft_count,
        limited,
        root_discovery_elapsed_ms,
        project_discovery_elapsed_ms,
        cached_projects_elapsed_ms,
        draft_collection_elapsed_ms,
        measurement_elapsed_ms,
        started.elapsed().as_millis()
    );
    Ok(CatalogPlan {
        rules: plans,
        limited: limited || is_cancelled(),
        elapsed_ms: started.elapsed().as_millis() as u64,
    })
}

fn configured_codex_home() -> Option<PathBuf> {
    #[cfg(not(test))]
    {
        codex_worktrees::home()
            .map_err(|error| {
                log::warn!(
                    "codex_worktree_home_unavailable error_digest={}",
                    blake3::hash(error.to_string().as_bytes()).to_hex()
                );
            })
            .ok()
    }
    #[cfg(test)]
    {
        // Tests inject an isolated home at the plan boundary while retaining
        // the real discovery and protection logic inside the plan builder.
        None
    }
}

fn automatic_codex_worktrees(
    home: Option<&Path>,
    is_cancelled: &(dyn Fn() -> bool + Sync),
) -> Vec<PathBuf> {
    let Some(home) = home else {
        return Vec::new();
    };
    let started = Instant::now();
    match codex_worktrees::discover(home, is_cancelled) {
        Ok(roots) => {
            log::info!(
                "codex_worktrees_discovered checkout_count={} elapsed_ms={}",
                roots.len(),
                started.elapsed().as_millis()
            );
            roots
        }
        Err(error) => {
            // An unavailable optional location must not suppress unrelated
            // cleanup results or broaden discovery to the whole data home.
            log::warn!(
                "codex_worktree_discovery_skipped error_digest={}",
                blake3::hash(error.to_string().as_bytes()).to_hex()
            );
            Vec::new()
        }
    }
}

fn retain_recent_project_matches(projects: &mut Vec<ProjectMatch>, maximum_roots: usize) {
    let mut root_activity = HashMap::<String, (PathBuf, u64)>::new();
    for project in projects.iter() {
        let modified_at_ms = path_modified_ms(&project.project_root);
        root_activity
            .entry(path_identity_key(&project.project_root))
            .and_modify(|(_, current)| *current = (*current).max(modified_at_ms))
            .or_insert_with(|| (project.project_root.clone(), modified_at_ms));
    }
    let discovered_root_count = root_activity.len();
    if discovered_root_count <= maximum_roots {
        return;
    }
    let mut ranked_roots = root_activity.into_values().collect::<Vec<_>>();
    ranked_roots
        .sort_by_cached_key(|(path, modified)| (Reverse(*modified), path_identity_key(path)));
    ranked_roots.truncate(maximum_roots);
    let retained = ranked_roots
        .into_iter()
        .map(|(root, _)| path_identity_key(&root))
        .collect::<HashSet<_>>();
    projects.retain(|project| retained.contains(&path_identity_key(&project.project_root)));
    log::info!(
        "project_artifact_standard_projects_limited discovered_root_count={} retained_root_count={} retained_match_count={}",
        discovered_root_count,
        retained.len(),
        projects.len()
    );
}

fn path_modified_ms(path: &Path) -> u64 {
    fs::symlink_metadata(path)
        .ok()
        .and_then(|metadata| modified_ms(&metadata))
        .unwrap_or(0)
}

const fn root_mode_name(mode: ProjectRootMode) -> &'static str {
    match mode {
        ProjectRootMode::Explicit => "explicit",
        ProjectRootMode::Standard => "standard",
        ProjectRootMode::Deep => "deep",
        ProjectRootMode::SelectedVolumes => "selectedVolumes",
    }
}

fn normalize_roots(configured_roots: &[String]) -> Result<Vec<PathBuf>, String> {
    if configured_roots.len() > MAX_CONFIGURED_ROOTS {
        return Err(format!(
            "project artifact scanning supports at most {MAX_CONFIGURED_ROOTS} roots"
        ));
    }
    normalize_root_paths(
        configured_roots.iter().map(PathBuf::from).collect(),
        "configured",
    )
}

#[cfg(not(test))]
fn automatic_project_roots(
    rules: &[ProjectArtifactRuleSource],
    mode: ProjectRootMode,
    selected_volume_roots: &[PathBuf],
    is_cancelled: &(dyn Fn() -> bool + Sync),
    report_path: &(dyn Fn(&Path) + Sync),
    report_files: &(dyn Fn(&Path, u64, u64) + Sync),
) -> Result<ProjectDiscoveryRoots, String> {
    let user_directories = current_platform()
        .user_directories()
        .map_err(|error| error.to_string())?;
    let local_volume_roots = match mode {
        ProjectRootMode::Standard => Vec::new(),
        ProjectRootMode::Deep => current_platform()
            .volumes()
            .unwrap_or_else(|error| {
                log::warn!(
                    "project_artifact_volume_discovery_failed error_digest={}",
                    blake3::hash(error.as_bytes()).to_hex()
                );
                Vec::new()
            })
            .into_iter()
            .filter(|volume| {
                !matches!(
                    volume.scan_concurrency.class,
                    ScanDeviceClass::Network | ScanDeviceClass::Removable
                )
            })
            .map(|volume| PathBuf::from(volume.mount_point))
            .collect::<Vec<_>>(),
        ProjectRootMode::SelectedVolumes => selected_volume_roots.to_vec(),
        ProjectRootMode::Explicit => {
            return Err("explicit project roots cannot use automatic discovery".to_string());
        }
    };
    let candidates = match mode {
        ProjectRootMode::Standard => standard_project_root_candidates(
            user_directories.home_directory(),
            &standard_runtime_data_paths(&user_directories),
            rules,
            is_cancelled,
            report_path,
            report_files,
        ),
        ProjectRootMode::Deep => deep_project_root_candidates(
            user_directories.home_directory(),
            &current_platform().system_volume_path(),
            &local_volume_roots,
            rules,
            is_cancelled,
            report_path,
            report_files,
        ),
        ProjectRootMode::SelectedVolumes => {
            let mut standard = standard_project_root_candidates(
                user_directories.home_directory(),
                &standard_runtime_data_paths(&user_directories),
                rules,
                is_cancelled,
                report_path,
                report_files,
            );
            standard.extend(deep_project_root_candidates(
                user_directories.home_directory(),
                &current_platform().system_volume_path(),
                &local_volume_roots,
                rules,
                is_cancelled,
                report_path,
                report_files,
            ));
            standard
        }
        ProjectRootMode::Explicit => unreachable!("explicit roots bypass automatic discovery"),
    };
    let mut roots = ProjectDiscoveryRoots {
        exact_roots: normalize_exact_root_paths(candidates.exact_roots, "automaticExact")?,
        recursive_roots: normalize_root_paths(candidates.recursive_roots, "automaticFallback")?,
    };
    let root_limit = match mode {
        ProjectRootMode::Standard => MAX_STANDARD_ROOTS,
        ProjectRootMode::Deep | ProjectRootMode::SelectedVolumes => MAX_DEEP_ROOTS,
        ProjectRootMode::Explicit => unreachable!("explicit roots bypass automatic discovery"),
    };
    if roots.root_count() > root_limit {
        let discovered_count = roots.root_count();
        roots.exact_roots.truncate(root_limit);
        let remaining = root_limit.saturating_sub(roots.exact_roots.len());
        roots.recursive_roots.truncate(remaining);
        log::warn!(
            "project_artifact_automatic_roots_truncated discovered_count={discovered_count} retained_count={root_limit}"
        );
    }
    log::info!(
        "project_artifact_roots_discovered mode={} exact_root_count={} recursive_root_count={} volume_count={}",
        root_mode_name(mode),
        roots.exact_roots.len(),
        roots.recursive_roots.len(),
        local_volume_roots.len()
    );
    Ok(roots)
}

#[cfg(not(test))]
fn cached_project_matches(
    rules: &[ProjectArtifactRuleSource],
    is_cancelled: &(dyn Fn() -> bool + Sync),
    report_path: &(dyn Fn(&Path) + Sync),
) -> Result<Vec<ProjectMatch>, String> {
    let user_directories = current_platform()
        .user_directories()
        .map_err(|error| error.to_string())?;
    let allowed_roots = standard_discovery_roots(
        user_directories.home_directory(),
        &standard_runtime_data_paths(&user_directories),
    );
    let prune_names = artifact_prune_names(rules);
    let cached = project_root_index::load().unwrap_or_else(|error| {
        log::warn!(
            "project_root_index_load_failed error_digest={}",
            blake3::hash(error.as_bytes()).to_hex()
        );
        Vec::new()
    });
    let indexed_count = cached.len();
    let maximum_cached_roots = MAX_STANDARD_PROJECT_ROOTS.saturating_mul(2);
    let mut eligible_roots = cached
        .into_iter()
        .filter(|root| {
            automatic_project_root_allowed(root, &allowed_roots, &prune_names)
                && is_real_directory(root)
        })
        .map(|root| {
            let modified_at_ms = path_modified_ms(&root);
            (root, modified_at_ms)
        })
        .collect::<Vec<_>>();
    eligible_roots.sort_by(|(left_path, left_modified), (right_path, right_modified)| {
        right_modified
            .cmp(left_modified)
            .then_with(|| left_path.cmp(right_path))
    });
    eligible_roots.truncate(maximum_cached_roots);
    let mut projects = Vec::new();
    let mut visited = HashSet::new();
    for (root, _) in eligible_roots {
        if is_cancelled() {
            break;
        }
        report_path(&root);
        // The index stores paths that were canonicalized when first discovered.
        // Re-canonicalizing every historical project made routine scans scale
        // with the complete index. Every artifact path is still validated
        // without links before it can enter a cleanup plan.
        if !visited.insert(path_identity_key(&root)) {
            continue;
        }
        for (rule_index, rule) in rules.iter().enumerate() {
            if project_matches_known_file_markers(&root, &rule.project_match) {
                projects.push(ProjectMatch {
                    rule_index,
                    project_root: root.clone(),
                    allow_descendant_scan: false,
                });
            }
        }
    }
    log::info!(
        "project_artifact_cached_projects_checked indexed_count={} checked_count={} match_count={}",
        indexed_count,
        visited.len(),
        projects.len()
    );
    Ok(projects)
}

#[cfg(test)]
fn cached_project_matches(
    _rules: &[ProjectArtifactRuleSource],
    _is_cancelled: &(dyn Fn() -> bool + Sync),
    _report_path: &(dyn Fn(&Path) + Sync),
) -> Result<Vec<ProjectMatch>, String> {
    Ok(Vec::new())
}

#[cfg(test)]
fn automatic_project_roots(
    _rules: &[ProjectArtifactRuleSource],
    _mode: ProjectRootMode,
    selected_volume_roots: &[PathBuf],
    _is_cancelled: &(dyn Fn() -> bool + Sync),
    _report_path: &(dyn Fn(&Path) + Sync),
    _report_files: &(dyn Fn(&Path, u64, u64) + Sync),
) -> Result<ProjectDiscoveryRoots, String> {
    // Unit tests must not scan the contributor's real workspaces. Candidate
    // discovery is covered with isolated fixtures below.
    Ok(ProjectDiscoveryRoots {
        exact_roots: Vec::new(),
        recursive_roots: selected_volume_roots.to_vec(),
    })
}

#[cfg(not(test))]
fn deep_project_root_candidates(
    home: &Path,
    system_volume: &Path,
    local_volume_roots: &[PathBuf],
    rules: &[ProjectArtifactRuleSource],
    is_cancelled: &(dyn Fn() -> bool + Sync),
    report_path: &(dyn Fn(&Path) + Sync),
    report_files: &(dyn Fn(&Path, u64, u64) + Sync),
) -> ProjectDiscoveryRoots {
    let allowed_roots = deep_discovery_roots(home, system_volume, local_volume_roots);
    let file_names = project_marker_file_names(rules);
    let file_suffixes = project_marker_file_suffixes(rules);
    let prune_names = artifact_prune_names(rules);
    let mut exact_roots = Vec::new();
    let mut fallback_roots = Vec::new();

    for volume_root in local_volume_roots {
        if is_cancelled() {
            break;
        }
        let scoped_roots = deep_roots_for_volume(
            &allowed_roots,
            volume_root,
            system_volume,
            local_volume_roots,
        );
        if scoped_roots.is_empty() {
            continue;
        }
        match indexed_project_roots(IndexedProjectRootRequest {
            search_root: volume_root,
            allowed_roots: &scoped_roots,
            file_names: &file_names,
            file_suffixes: &file_suffixes,
            prune_names: &prune_names,
            is_cancelled,
            report_path,
            report_files,
        }) {
            IndexedProjectRootOutcome::Indexed {
                exact_roots: indexed,
                fallback_roots: fallback,
            } => {
                exact_roots.extend(indexed);
                fallback_roots.extend(fallback);
            }
            IndexedProjectRootOutcome::Fallback => fallback_roots.extend(scoped_roots),
            IndexedProjectRootOutcome::Cancelled => break,
        }
    }

    ProjectDiscoveryRoots {
        exact_roots,
        recursive_roots: fallback_roots,
    }
}

#[cfg(not(test))]
fn standard_project_root_candidates(
    home: &Path,
    runtime_data_paths: &[PathBuf],
    rules: &[ProjectArtifactRuleSource],
    is_cancelled: &(dyn Fn() -> bool + Sync),
    report_path: &(dyn Fn(&Path) + Sync),
    report_files: &(dyn Fn(&Path, u64, u64) + Sync),
) -> ProjectDiscoveryRoots {
    let allowed_roots = standard_discovery_roots(home, runtime_data_paths);
    if allowed_roots.is_empty() {
        return ProjectDiscoveryRoots::default();
    }
    let file_names = project_marker_file_names(rules);
    let file_suffixes = project_marker_file_suffixes(rules);
    let prune_names = artifact_prune_names(rules);
    match indexed_project_roots(IndexedProjectRootRequest {
        search_root: home,
        allowed_roots: &allowed_roots,
        file_names: &file_names,
        file_suffixes: &file_suffixes,
        prune_names: &prune_names,
        is_cancelled,
        report_path,
        report_files,
    }) {
        IndexedProjectRootOutcome::Indexed {
            exact_roots,
            fallback_roots,
        } => ProjectDiscoveryRoots {
            exact_roots,
            recursive_roots: fallback_roots,
        },
        IndexedProjectRootOutcome::Fallback => ProjectDiscoveryRoots {
            exact_roots: Vec::new(),
            recursive_roots: allowed_roots,
        },
        IndexedProjectRootOutcome::Cancelled => ProjectDiscoveryRoots::default(),
    }
}

#[cfg(not(test))]
fn indexed_project_roots(request: IndexedProjectRootRequest<'_>) -> IndexedProjectRootOutcome {
    let IndexedProjectRootRequest {
        search_root,
        allowed_roots,
        file_names,
        file_suffixes,
        prune_names,
        is_cancelled,
        report_path,
        report_files,
    } = request;
    let mut indexed_candidates = Vec::new();
    let mut fallback_roots = Vec::new();
    let prune_names_for_native_scan = native_project_scan_prune_names(prune_names);
    let mut candidate_count = 0_u64;
    let mut fallback_root_count = 0_usize;
    let mut strategies = HashSet::new();
    let mut consent_protected_roots_unavailable = false;

    // The native scanner must walk the exact same roots as the portable fallback. Scanning the
    // common ancestor first and rejecting candidates afterwards still performs all filesystem I/O
    // under excluded runtime-data trees, which made standard scans slower without changing their
    // results. A failed or unsupported root is appended as a fallback discovery root, so one
    // inaccessible volume cannot discard candidates already obtained from another volume.
    for allowed_root in allowed_roots {
        if is_cancelled() {
            return IndexedProjectRootOutcome::Cancelled;
        }
        if consent_protected_roots_unavailable && macos_root_requires_explicit_consent(allowed_root)
        {
            log::info!(
                "project_marker_fast_scan_root_skipped scope={} reason=consent_protected_access_unavailable",
                diagnostic_path(allowed_root)
            );
            continue;
        }
        let root_started = Instant::now();
        let result = current_platform().fast_project_marker_candidates(
            ProjectMarkerCandidateQuery {
                root: allowed_root,
                file_names,
                file_suffixes,
                pruned_directory_names: &prune_names_for_native_scan,
                maximum_depth: MAX_DISCOVERY_DEPTH,
            },
            is_cancelled,
            &|progress| report_path(&progress.current_directory),
            &mut |marker_path| {
                report_path(&marker_path);
                if let Some(project_root) = validated_marker_project_root(
                    &marker_path,
                    allowed_roots,
                    prune_names,
                    MAX_DISCOVERY_DEPTH,
                ) {
                    indexed_candidates.push(project_root);
                }
                Ok(())
            },
        );
        match result {
            Ok(Some(summary)) => {
                candidate_count = candidate_count.saturating_add(summary.candidate_count);
                strategies.insert(summary.strategy);
                // Publish counters only after the native root completes. Paths remain live during
                // enumeration, while delayed counter commit prevents a failed fast path and its
                // portable fallback from double-counting the same filesystem work.
                report_files(allowed_root, summary.file_count, 0);
                log::info!(
                    "project_marker_fast_scan_root_completed scope={} strategy={} candidate_count={} file_count={} directory_count={} elapsed_ms={}",
                    diagnostic_path(allowed_root),
                    summary.strategy,
                    summary.candidate_count,
                    summary.file_count,
                    summary.directory_count,
                    root_started.elapsed().as_millis()
                );
            }
            Ok(None) => {
                fallback_root_count = fallback_root_count.saturating_add(1);
                fallback_roots.push(allowed_root.clone());
                log::info!(
                    "project_marker_fast_scan_root_unavailable scope={} elapsed_ms={}",
                    diagnostic_path(allowed_root),
                    root_started.elapsed().as_millis()
                );
            }
            Err(ProjectMarkerCandidateScanError::Cancelled) => {
                return IndexedProjectRootOutcome::Cancelled;
            }
            Err(ProjectMarkerCandidateScanError::Unavailable(error)) => {
                if macos_root_requires_explicit_consent(allowed_root) {
                    // Access to Desktop, Documents, and the other protected
                    // home folders is granted to the process as one macOS
                    // privacy capability. A timeout on one of them predicts
                    // the same block for its peers, so skip only that known
                    // protected set while continuing custom project roots.
                    consent_protected_roots_unavailable = true;
                }
                log::warn!(
                    "project_marker_fast_scan_root_skipped scope={} elapsed_ms={} reason=unavailable error_digest={}",
                    diagnostic_path(allowed_root),
                    root_started.elapsed().as_millis(),
                    blake3::hash(error.as_bytes()).to_hex()
                );
            }
            Err(ProjectMarkerCandidateScanError::Platform(error))
            | Err(ProjectMarkerCandidateScanError::Consumer(error)) => {
                fallback_root_count = fallback_root_count.saturating_add(1);
                fallback_roots.push(allowed_root.clone());
                log::warn!(
                    "project_marker_fast_scan_root_failed scope={} elapsed_ms={} error_digest={}",
                    diagnostic_path(allowed_root),
                    root_started.elapsed().as_millis(),
                    blake3::hash(error.as_bytes()).to_hex()
                );
            }
        }
    }

    if strategies.is_empty() && fallback_root_count == allowed_roots.len() {
        log::info!(
            "project_marker_fast_scan_unavailable scope={} fallback_root_count={}",
            diagnostic_path(search_root),
            fallback_root_count
        );
        return IndexedProjectRootOutcome::Fallback;
    }

    log::info!(
        "project_marker_fast_scan_completed strategy_count={} scope={} candidate_count={} accepted_or_fallback_root_count={} fallback_root_count={}",
        strategies.len(),
        diagnostic_path(search_root),
        candidate_count,
        indexed_candidates.len(),
        fallback_root_count
    );
    IndexedProjectRootOutcome::Indexed {
        exact_roots: indexed_candidates,
        fallback_roots,
    }
}

#[cfg(not(test))]
fn native_project_scan_prune_names(prune_names: &HashSet<String>) -> Vec<String> {
    let mut names = prune_names.iter().cloned().collect::<Vec<_>>();
    names.extend(
        ALWAYS_SKIPPED_DIRECTORIES
            .iter()
            .chain(DEEP_ALWAYS_SKIPPED_DIRECTORIES.iter())
            .map(|name| (*name).to_string()),
    );
    names.sort();
    names.dedup();
    names
}

#[cfg(not(test))]
fn project_marker_file_names(rules: &[ProjectArtifactRuleSource]) -> Vec<String> {
    let mut names = rules
        .iter()
        .flat_map(|rule| rule.project_match.file_names_any.iter().cloned())
        .collect::<Vec<_>>();
    names.sort();
    names.dedup();
    names
}

#[cfg(not(test))]
fn project_marker_file_suffixes(rules: &[ProjectArtifactRuleSource]) -> Vec<String> {
    let mut suffixes = rules
        .iter()
        .flat_map(|rule| rule.project_match.file_suffixes_any.iter().cloned())
        .collect::<Vec<_>>();
    suffixes.sort();
    suffixes.dedup();
    suffixes
}

#[cfg(not(test))]
fn deep_roots_for_volume(
    allowed_roots: &[PathBuf],
    volume_root: &Path,
    system_volume: &Path,
    local_volume_roots: &[PathBuf],
) -> Vec<PathBuf> {
    if paths_equal(volume_root, system_volume) {
        let other_volumes = local_volume_roots
            .iter()
            .filter(|other| !paths_equal(other, system_volume))
            .collect::<Vec<_>>();
        allowed_roots
            .iter()
            .filter(|root| {
                !other_volumes
                    .iter()
                    .any(|other| path_is_same_or_child(root, other))
            })
            .cloned()
            .collect()
    } else {
        allowed_roots
            .iter()
            .filter(|root| path_is_same_or_child(root, volume_root))
            .cloned()
            .collect()
    }
}

fn validated_marker_project_root(
    marker_path: &Path,
    allowed_roots: &[PathBuf],
    prune_names: &HashSet<String>,
    maximum_depth: usize,
) -> Option<PathBuf> {
    let metadata = fs::symlink_metadata(marker_path).ok()?;
    if !metadata.is_file() || is_link_like(&metadata) {
        return None;
    }
    let project_root = marker_path.parent()?;
    if !is_real_directory(project_root) {
        return None;
    }
    let allowed_root = allowed_roots
        .iter()
        .find(|allowed| path_is_same_or_child(project_root, allowed))?;
    let relative_depth = current_platform()
        .relative_path(project_root, allowed_root)?
        .components()
        .count();
    (relative_depth <= maximum_depth
        && automatic_project_root_allowed(project_root, allowed_roots, prune_names))
    .then(|| project_root.to_path_buf())
}

fn automatic_project_root_allowed(
    project_root: &Path,
    allowed_roots: &[PathBuf],
    prune_names: &HashSet<String>,
) -> bool {
    let Some(allowed_root) = allowed_roots
        .iter()
        .find(|allowed| path_is_same_or_child(project_root, allowed))
    else {
        return false;
    };
    let Some(relative) = current_platform().relative_path(project_root, allowed_root) else {
        return false;
    };
    !relative.components().any(|component| {
        let name = component.as_os_str().to_string_lossy();
        name.starts_with('.')
            || ALWAYS_SKIPPED_DIRECTORIES
                .iter()
                .any(|candidate| path_name_eq(&name, candidate))
            || DEEP_ALWAYS_SKIPPED_DIRECTORIES
                .iter()
                .any(|candidate| path_name_eq(&name, candidate))
            || prune_names
                .iter()
                .any(|candidate| path_name_eq(&name, candidate))
    })
}

fn deep_discovery_roots(
    home: &Path,
    system_volume: &Path,
    local_volume_roots: &[PathBuf],
) -> Vec<PathBuf> {
    let mut roots = immediate_real_directories(home)
        .into_iter()
        .filter(|path| {
            !has_excluded_name(path, DEEP_HOME_EXCLUSIONS) && !has_hidden_file_name(path)
        })
        .collect::<Vec<_>>();
    for volume_root in local_volume_roots {
        if paths_equal(volume_root, system_volume) {
            roots.extend(
                immediate_real_directories(volume_root)
                    .into_iter()
                    .filter(|path| {
                        !has_excluded_name(path, DEEP_SYSTEM_VOLUME_EXCLUSIONS)
                            && !has_hidden_file_name(path)
                    }),
            );
        } else {
            roots.push(volume_root.clone());
        }
    }
    roots
}

fn standard_discovery_roots(home: &Path, runtime_data_paths: &[PathBuf]) -> Vec<PathBuf> {
    immediate_real_directories(home)
        .into_iter()
        .filter(|path| !has_hidden_file_name(path))
        // Platform APIs provide cache and application-data paths. Excluding
        // their first ancestor under the profile avoids traversing runtime
        // data without assuming that users name development folders in a
        // particular language or convention.
        .filter(|path| {
            !runtime_data_paths
                .iter()
                .any(|runtime_path| path_is_same_or_child(runtime_path, path))
        })
        .collect()
}

#[cfg(any(not(test), target_os = "macos"))]
fn macos_root_requires_explicit_consent(path: &Path) -> bool {
    #[cfg(target_os = "macos")]
    {
        let Ok(user_directories) = current_platform().user_directories() else {
            return false;
        };
        path.parent().is_some_and(|parent| {
            paths_equal(parent, user_directories.home_directory())
                && has_excluded_name(
                    path,
                    &[
                        "Desktop",
                        "Documents",
                        "Downloads",
                        "Movies",
                        "Music",
                        "Pictures",
                    ],
                )
        })
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = path;
        false
    }
}

#[cfg(not(test))]
fn standard_runtime_data_paths(user_directories: &UserDirectories) -> Vec<PathBuf> {
    user_directories
        .application_storage_directories()
        .iter()
        .cloned()
        .chain(std::iter::once(
            user_directories.temporary_directory().to_path_buf(),
        ))
        .collect()
}

fn has_hidden_file_name(path: &Path) -> bool {
    path.file_name()
        .is_some_and(|name| name.to_string_lossy().starts_with('.'))
}

fn has_excluded_name(path: &Path, excluded_names: &[&str]) -> bool {
    let name = path
        .file_name()
        .map(|value| value.to_string_lossy())
        .unwrap_or_default();
    excluded_names
        .iter()
        .any(|excluded| path_name_eq(&name, excluded))
}

fn paths_equal(left: &Path, right: &Path) -> bool {
    current_platform().paths_equal(left, right)
}

fn path_identity_key(path: &Path) -> String {
    current_platform().path_identity_key(path)
}

fn path_is_same_or_child(path: &Path, root: &Path) -> bool {
    current_platform().path_is_same_or_child(path, root)
}

fn path_component_count(path: &Path) -> usize {
    PathBuf::from(current_platform().display_path(path))
        .components()
        .count()
}

fn path_or_ancestor_is_retained(path: &Path, retained_identities: &HashSet<String>) -> bool {
    // Candidate paths are canonicalized before deduplication. Walking their
    // lexical ancestors keeps containment checks bounded by path depth while
    // still delegating case folding and verbatim-prefix handling to the
    // platform-owned identity contract.
    path.ancestors()
        .any(|ancestor| retained_identities.contains(&path_identity_key(ancestor)))
}

fn update_project_root_index(projects: &[ProjectMatch]) {
    let mut roots = project_root_index::load().unwrap_or_else(|error| {
        log::warn!(
            "project_root_index_merge_load_failed error_digest={}",
            blake3::hash(error.as_bytes()).to_hex()
        );
        Vec::new()
    });
    roots.extend(projects.iter().map(|project| project.project_root.clone()));
    if let Err(error) = project_root_index::save(&roots) {
        log::warn!(
            "project_root_index_save_failed error_digest={}",
            blake3::hash(error.as_bytes()).to_hex()
        );
    } else {
        log::info!("project_root_index_updated root_count={}", roots.len());
    }
}

fn immediate_real_directories(parent: &Path) -> Vec<PathBuf> {
    let mut directories = fs::read_dir(parent)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| is_real_directory(path))
        .collect::<Vec<_>>();
    directories.sort();
    directories
}

fn is_real_directory(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|metadata| metadata.is_dir() && !is_link_like(&metadata))
}

fn normalize_root_paths(paths: Vec<PathBuf>, source: &str) -> Result<Vec<PathBuf>, String> {
    normalize_root_paths_with_policy(paths, source, true)
}

fn normalize_exact_root_paths(paths: Vec<PathBuf>, source: &str) -> Result<Vec<PathBuf>, String> {
    normalize_root_paths_with_policy(paths, source, false)
}

fn normalize_root_paths_with_policy(
    paths: Vec<PathBuf>,
    source: &str,
    remove_descendants: bool,
) -> Result<Vec<PathBuf>, String> {
    let mut roots = Vec::<PathBuf>::new();
    let mut root_identities = HashSet::<String>::new();
    for path in paths {
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.is_dir() && !is_link_like(&metadata) => metadata,
            Ok(_) => {
                log::warn!(
                    "project_artifact_root_skipped source={} path={} reason=notDirectoryOrLink",
                    source,
                    diagnostic_path(&path)
                );
                continue;
            }
            Err(error) => {
                if source == "configured" {
                    log::warn!(
                        "project_artifact_root_skipped source={} path={} reason=metadata error_kind={:?}",
                        source,
                        diagnostic_path(&path),
                        error.kind()
                    );
                }
                continue;
            }
        };
        let _ = metadata;
        let canonical = match current_platform().canonicalize_no_links(&path) {
            Ok(path) => path,
            Err(error) => {
                log::warn!(
                    "project_artifact_root_skipped source={} path={} reason=canonicalize error_digest={}",
                    source,
                    diagnostic_path(&path),
                    blake3::hash(error.as_bytes()).to_hex()
                );
                continue;
            }
        };
        if root_identities.insert(path_identity_key(&canonical)) {
            roots.push(canonical);
        }
    }
    roots.sort_by_cached_key(|root| (path_component_count(root), path_identity_key(root)));
    if !remove_descendants {
        return Ok(roots);
    }
    let mut deduplicated = Vec::<PathBuf>::new();
    let mut retained_identities = HashSet::<String>::new();
    for root in roots {
        if path_or_ancestor_is_retained(&root, &retained_identities) {
            continue;
        }
        retained_identities.insert(path_identity_key(&root));
        deduplicated.push(root);
    }
    Ok(deduplicated)
}

fn discover_projects(
    roots: &[PathBuf],
    rules: &[ProjectArtifactRuleSource],
    skip_hidden_directories: bool,
    deep: bool,
    maximum_depth: usize,
    is_cancelled: &(dyn Fn() -> bool + Sync),
    report_path: &(dyn Fn(&Path) + Sync),
) -> Result<(Vec<ProjectMatch>, bool), String> {
    let prune_names = artifact_prune_names(rules);
    let mut stack = roots
        .iter()
        .cloned()
        .map(|path| (path, 0_usize))
        .collect::<Vec<_>>();
    let mut projects = Vec::new();
    let mut visited = 0_usize;
    let mut limited = false;
    while let Some((directory, depth)) = stack.pop() {
        if is_cancelled() {
            return Ok((projects, true));
        }
        report_path(&directory);
        visited += 1;
        if visited > MAX_DISCOVERED_DIRECTORIES || projects.len() >= MAX_DISCOVERED_PROJECTS {
            limited = true;
            break;
        }
        let entries = match read_directory_entries(&directory) {
            Ok(entries) => entries,
            Err(error) => {
                log::debug!(
                    "project_artifact_directory_skipped path={} error_kind={:?}",
                    diagnostic_path(&directory),
                    error.kind()
                );
                continue;
            }
        };
        for (rule_index, rule) in rules.iter().enumerate() {
            if project_matches_entries(&directory, &entries.file_names, &rule.project_match) {
                projects.push(ProjectMatch {
                    rule_index,
                    project_root: directory.clone(),
                    allow_descendant_scan: true,
                });
            }
        }
        if depth >= maximum_depth {
            continue;
        }
        for child in entries.directories.into_iter().rev() {
            let name = child
                .file_name()
                .map(|value| value.to_string_lossy())
                .unwrap_or_default();
            if ALWAYS_SKIPPED_DIRECTORIES
                .iter()
                .any(|candidate| path_name_eq(&name, candidate))
                || (skip_hidden_directories && name.starts_with('.'))
                || (deep
                    && DEEP_ALWAYS_SKIPPED_DIRECTORIES
                        .iter()
                        .any(|candidate| path_name_eq(&name, candidate)))
                || prune_names
                    .iter()
                    .any(|candidate| path_name_eq(&name, candidate))
            {
                continue;
            }
            stack.push((child, depth + 1));
        }
    }
    sort_and_deduplicate_project_matches(&mut projects);
    Ok((projects, limited))
}

fn sort_and_deduplicate_project_matches(projects: &mut Vec<ProjectMatch>) {
    projects.sort_by_cached_key(|project| {
        (
            path_identity_key(&project.project_root),
            project.rule_index,
            Reverse(project.allow_descendant_scan),
        )
    });
    projects.dedup_by(|left, right| {
        left.rule_index == right.rule_index && paths_equal(&left.project_root, &right.project_root)
    });
}

fn read_directory_entries(path: &Path) -> std::io::Result<DirectoryEntries> {
    let mut file_names = Vec::new();
    let mut directories = Vec::new();
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let metadata = fs::symlink_metadata(entry.path())?;
        if is_link_like(&metadata) {
            continue;
        }
        if metadata.is_dir() {
            directories.push(entry.path());
        } else if metadata.is_file() {
            file_names.push(entry.file_name().to_string_lossy().into_owned());
        }
    }
    directories.sort();
    Ok(DirectoryEntries {
        file_names,
        directories,
    })
}

fn project_matches(project_root: &Path, project_match: &ProjectMatchSource) -> bool {
    read_directory_entries(project_root).is_ok_and(|entries| {
        project_matches_entries(project_root, &entries.file_names, project_match)
    })
}

#[cfg(not(test))]
fn project_matches_known_file_markers(
    project_root: &Path,
    project_match: &ProjectMatchSource,
) -> bool {
    !project_match.file_names_any.is_empty()
        && project_match
            .file_names_any
            .iter()
            .any(|file_name| safe_marker_file_exists(project_root, file_name))
        && project_match
            .relative_paths_all
            .iter()
            .all(|relative| safe_required_path_exists(project_root, relative))
        && (project_match.relative_paths_any.is_empty()
            || project_match
                .relative_paths_any
                .iter()
                .any(|relative| safe_required_path_exists(project_root, relative)))
}

#[cfg(not(test))]
fn safe_marker_file_exists(project_root: &Path, file_name: &str) -> bool {
    let path = project_root.join(file_name);
    fs::symlink_metadata(&path).is_ok_and(|metadata| metadata.is_file() && !is_link_like(&metadata))
        && current_platform().validate_path_no_links(&path).is_ok()
}

fn project_matches_entries(
    project_root: &Path,
    file_names: &[String],
    project_match: &ProjectMatchSource,
) -> bool {
    let marker_matches = file_names.iter().any(|file_name| {
        project_match
            .file_names_any
            .iter()
            .any(|candidate| path_name_eq(file_name, candidate))
            || project_match
                .file_suffixes_any
                .iter()
                .any(|suffix| path_name_ends_with(file_name, suffix))
    });
    marker_matches
        && project_match
            .relative_paths_all
            .iter()
            .all(|relative| safe_required_path_exists(project_root, relative))
        && (project_match.relative_paths_any.is_empty()
            || project_match
                .relative_paths_any
                .iter()
                .any(|relative| safe_required_path_exists(project_root, relative)))
}

fn safe_required_path_exists(project_root: &Path, relative: &str) -> bool {
    let path = join_rule_path(project_root, relative);
    fs::symlink_metadata(&path).is_ok_and(|metadata| !is_link_like(&metadata))
        && current_platform().validate_path_no_links(&path).is_ok()
}

fn collect_artifact_drafts(
    projects: &[ProjectMatch],
    rules: &[ProjectArtifactRuleSource],
    is_cancelled: &(dyn Fn() -> bool + Sync),
    report_path: &(dyn Fn(&Path) + Sync),
) -> Vec<ArtifactDraft> {
    let mut drafts = Vec::new();
    let prune_names = artifact_prune_names(rules);
    for project in projects {
        if is_cancelled() {
            break;
        }
        report_path(&project.project_root);
        for artifact in &rules[project.rule_index].artifacts {
            match artifact {
                ProjectArtifactSource::RelativeDirectory { path } => {
                    let candidate = join_rule_path(&project.project_root, path);
                    if let Ok(candidate) = validate_candidate(&project.project_root, &candidate) {
                        drafts.push(ArtifactDraft {
                            rule_index: project.rule_index,
                            project_root: project.project_root.clone(),
                            path: candidate,
                        });
                    }
                }
                ProjectArtifactSource::DescendantDirectory { name, max_depth } => {
                    if !project.allow_descendant_scan {
                        continue;
                    }
                    drafts.extend(discover_descendant_artifacts(
                        project,
                        name,
                        *max_depth,
                        &prune_names,
                        is_cancelled,
                        report_path,
                    ));
                }
            }
        }
    }
    drafts
}

fn discover_descendant_artifacts(
    project: &ProjectMatch,
    target_name: &str,
    max_depth: usize,
    prune_names: &HashSet<String>,
    is_cancelled: &(dyn Fn() -> bool + Sync),
    report_path: &(dyn Fn(&Path) + Sync),
) -> Vec<ArtifactDraft> {
    let mut stack = vec![(project.project_root.clone(), 0_usize)];
    let mut drafts = Vec::new();
    while let Some((directory, depth)) = stack.pop() {
        if is_cancelled() || depth >= max_depth {
            continue;
        }
        report_path(&directory);
        let Ok(entries) = read_directory_entries(&directory) else {
            continue;
        };
        for child in entries.directories {
            let name = child
                .file_name()
                .map(|value| value.to_string_lossy())
                .unwrap_or_default();
            if path_name_eq(&name, target_name) {
                if let Ok(child) = validate_candidate(&project.project_root, &child) {
                    drafts.push(ArtifactDraft {
                        rule_index: project.rule_index,
                        project_root: project.project_root.clone(),
                        path: child,
                    });
                }
                continue;
            }
            if ALWAYS_SKIPPED_DIRECTORIES
                .iter()
                .any(|candidate| path_name_eq(&name, candidate))
                || prune_names
                    .iter()
                    .any(|candidate| path_name_eq(&name, candidate))
            {
                continue;
            }
            stack.push((child, depth + 1));
        }
    }
    drafts
}

fn validate_candidate(project_root: &Path, candidate: &Path) -> Result<PathBuf, String> {
    if paths_equal(candidate, project_root) {
        return Err("an artifact directory cannot equal its project root".to_string());
    }
    let metadata = fs::symlink_metadata(candidate).map_err(|error| error.to_string())?;
    if !metadata.is_dir() || is_link_like(&metadata) {
        return Err("an artifact candidate must be a real directory".to_string());
    }
    current_platform()
        .validate_path_no_links(candidate)
        .map_err(|error| error.to_string())?;
    let canonical = current_platform()
        .canonicalize_no_links(candidate)
        .map_err(|error| error.to_string())?;
    if current_platform().paths_equal(&canonical, project_root)
        || !current_platform().path_is_same_or_child(&canonical, project_root)
    {
        return Err("an artifact candidate escaped its project root".to_string());
    }
    Ok(canonical)
}

fn deduplicate_artifacts(mut drafts: Vec<ArtifactDraft>) -> Vec<ArtifactDraft> {
    drafts.sort_by_cached_key(|draft| {
        (
            path_component_count(&draft.path),
            path_identity_key(&draft.path),
            draft.rule_index,
        )
    });
    let mut retained = Vec::<ArtifactDraft>::new();
    let mut retained_identities = HashSet::<String>::new();
    for draft in drafts {
        if path_or_ancestor_is_retained(&draft.path, &retained_identities) {
            continue;
        }
        retained_identities.insert(path_identity_key(&draft.path));
        retained.push(draft);
    }
    retained
}

fn measure_artifacts(
    drafts: Vec<ArtifactDraft>,
    is_cancelled: &(dyn Fn() -> bool + Sync),
    report_path: &(dyn Fn(&Path) + Sync),
    report_files: &(dyn Fn(&Path, u64, u64) + Sync),
) -> Vec<(ArtifactDraft, ArtifactMeasurement)> {
    if drafts.is_empty() {
        return Vec::new();
    }
    let next = AtomicUsize::new(0);
    let results = Mutex::new(
        (0..drafts.len())
            .map(|_| None)
            .collect::<Vec<Option<ArtifactMeasurement>>>(),
    );
    let worker_count = thread::available_parallelism()
        .map(|value| value.get())
        .unwrap_or(2)
        .min(MEASUREMENT_WORKER_LIMIT)
        .min(drafts.len());
    thread::scope(|scope| {
        for _ in 0..worker_count {
            scope.spawn(|| loop {
                let index = next.fetch_add(1, Ordering::Relaxed);
                if index >= drafts.len() || is_cancelled() {
                    break;
                }
                let measured = measure_directory_with_progress(
                    &drafts[index].path,
                    is_cancelled,
                    report_path,
                    report_files,
                );
                if let Ok(mut values) = results.lock() {
                    values[index] = Some(measured);
                } else {
                    break;
                }
            });
        }
    });
    let measured = results
        .into_inner()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    drafts
        .into_iter()
        .zip(measured)
        .map(|(draft, result)| {
            (
                draft,
                result.unwrap_or(ArtifactMeasurement {
                    measured: MeasureResult {
                        bytes: 0,
                        file_count: 0,
                        skipped_count: 1,
                    },
                    modified_at_ms: None,
                }),
            )
        })
        .collect()
}

fn measure_directory(root: &Path, is_cancelled: &(dyn Fn() -> bool + Sync)) -> ArtifactMeasurement {
    measure_directory_with_progress(root, is_cancelled, &|_| {}, &|_, _, _| {})
}

fn measure_directory_with_progress(
    root: &Path,
    is_cancelled: &(dyn Fn() -> bool + Sync),
    report_path: &(dyn Fn(&Path) + Sync),
    report_files: &(dyn Fn(&Path, u64, u64) + Sync),
) -> ArtifactMeasurement {
    // Project-artifact measurement still publishes its authoritative totals
    // through `report_files` after success. Ignore provisional native counts
    // here so a failed native path can use the portable fallback without
    // duplicating UI counters.
    let native_progress = |path: &Path, _: u64, _: u64| report_path(path);
    match current_platform().fast_project_artifact_tree_aggregate(
        root,
        is_cancelled,
        &native_progress,
    ) {
        Ok(Some(aggregate)) => {
            report_files(root, aggregate.file_count, aggregate.bytes);
            let modified_at_ms = aggregate
                .sources
                .iter()
                .filter_map(|source| source.modified_at_ms)
                .max();
            log::debug!(
                "project_artifact_directory_aggregate_finished strategy={} file_count={} bytes={} skipped_count={}",
                aggregate.strategy,
                aggregate.file_count,
                aggregate.bytes,
                aggregate.skipped_count
            );
            return ArtifactMeasurement {
                measured: MeasureResult {
                    bytes: aggregate.bytes,
                    file_count: aggregate.file_count,
                    skipped_count: aggregate.skipped_count,
                },
                modified_at_ms,
            };
        }
        Ok(None) => {}
        Err(DirectoryTreeAggregateError::Cancelled) => {
            return ArtifactMeasurement {
                measured: MeasureResult {
                    bytes: 0,
                    file_count: 0,
                    skipped_count: 1,
                },
                modified_at_ms: None,
            };
        }
        Err(DirectoryTreeAggregateError::Platform(error)) => {
            log::warn!(
                "project_artifact_directory_aggregate_fallback error_digest={}",
                &blake3::hash(error.as_bytes()).to_hex()[..12]
            );
        }
    }

    portable_measure_directory_with_progress(root, is_cancelled, report_path, report_files)
}

/// Preserves the platform-independent reference implementation used when a native aggregate is
/// unavailable or fails. Keeping it callable in isolation also lets regression tests compare the
/// optimized path against the original project-artifact semantics on the same fixture.
fn portable_measure_directory_with_progress(
    root: &Path,
    is_cancelled: &(dyn Fn() -> bool + Sync),
    report_path: &(dyn Fn(&Path) + Sync),
    report_files: &(dyn Fn(&Path, u64, u64) + Sync),
) -> ArtifactMeasurement {
    let mut result = ArtifactMeasurement::default();
    let mut stack = vec![root.to_path_buf()];
    let mut pending_file_count = 0_u64;
    let mut pending_bytes = 0_u64;
    let mut latest_file_path = None;
    while let Some(path) = stack.pop() {
        if is_cancelled() {
            result.measured.skipped_count = result.measured.skipped_count.saturating_add(1);
            break;
        }
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(_) => {
                result.measured.skipped_count = result.measured.skipped_count.saturating_add(1);
                continue;
            }
        };
        if is_link_like(&metadata) || metadata.is_file() {
            // Generated dependency trees commonly contain symbolic links.
            // Count link metadata without following its target so a safe link
            // does not degrade the entire artifact rule to Limited. Candidate
            // root links are still rejected by validation before measurement.
            result.measured.bytes = result.measured.bytes.saturating_add(metadata.len());
            result.measured.file_count = result.measured.file_count.saturating_add(1);
            result.modified_at_ms = latest_timestamp(result.modified_at_ms, modified_ms(&metadata));
            pending_file_count = pending_file_count.saturating_add(1);
            pending_bytes = pending_bytes.saturating_add(metadata.len());
            latest_file_path = Some(path);
            // Project artifact measurement can outlive the declarative rule
            // scan by several seconds. Batching keeps the UI counter live
            // without adding one atomic callback for every generated file.
            if pending_file_count >= PROGRESS_FILE_BATCH_SIZE {
                report_files(
                    latest_file_path
                        .as_deref()
                        .expect("a progress batch must contain a file path"),
                    pending_file_count,
                    pending_bytes,
                );
                pending_file_count = 0;
                pending_bytes = 0;
            }
        } else if metadata.is_dir() {
            report_path(&path);
            match fs::read_dir(&path) {
                Ok(entries) => {
                    for entry in entries {
                        match entry {
                            Ok(entry) => stack.push(entry.path()),
                            Err(_) => {
                                result.measured.skipped_count =
                                    result.measured.skipped_count.saturating_add(1)
                            }
                        }
                    }
                }
                Err(_) => {
                    result.measured.skipped_count = result.measured.skipped_count.saturating_add(1)
                }
            }
        } else {
            result.measured.skipped_count = result.measured.skipped_count.saturating_add(1);
        }
    }
    if pending_file_count > 0 {
        report_files(
            latest_file_path
                .as_deref()
                .expect("a progress batch must contain a file path"),
            pending_file_count,
            pending_bytes,
        );
    }
    result
}

fn artifact_prune_names(rules: &[ProjectArtifactRuleSource]) -> HashSet<String> {
    rules
        .iter()
        .flat_map(|rule| &rule.artifacts)
        .filter_map(|artifact| match artifact {
            ProjectArtifactSource::RelativeDirectory { path } => Path::new(path)
                .components()
                .next()
                .map(|component| component.as_os_str().to_string_lossy().into_owned()),
            ProjectArtifactSource::DescendantDirectory { name, .. } => Some(name.clone()),
        })
        .collect()
}

fn join_rule_path(root: &Path, relative: &str) -> PathBuf {
    relative
        .split(['/', '\\'])
        .fold(root.to_path_buf(), |path, component| path.join(component))
}

fn scan_result(
    rule: &ProjectArtifactRuleSource,
    status: ScanItemStatus,
    bytes: u64,
    file_count: u64,
    elapsed_ms: u64,
    source_summary: CleanupSourceSummary,
) -> ScanRuleResult {
    let available = status != ScanItemStatus::NotApplicable;
    ScanRuleResult {
        rule_id: rule.id.clone(),
        category: crate::cleanup::CleanupCategory::Project,
        group: crate::cleanup::CleanupGroup::Project,
        risk: RiskLevel::Recoverable,
        default_selected: false,
        recommended_selected: false,
        bytes,
        file_count,
        available,
        selectable: status == ScanItemStatus::Found && bytes > 0,
        status,
        running_processes: Vec::new(),
        requires_app_close: false,
        sources: source_summary.sources,
        source_count: source_summary.source_count,
        sources_truncated: false,
        scan_elapsed_ms: elapsed_ms,
    }
}

fn apply_codex_process_guard(
    result: &mut ScanRuleResult,
    candidates: &[ArtifactCandidate],
    processes: Option<&Result<Vec<String>, String>>,
) {
    let paths = candidates
        .iter()
        .filter(|candidate| candidate.codex_checkout.is_some())
        .map(|candidate| display_path(&candidate.path))
        .collect::<HashSet<_>>();
    if paths.is_empty() {
        return;
    }
    let (reason, status) = match processes {
        Some(Ok(names)) if names.is_empty() => return,
        Some(Ok(names)) => {
            result.running_processes = names.clone();
            result.requires_app_close = true;
            (
                crate::cleanup::CleanupSourceBlockReason::RequiresClose,
                ScanItemStatus::RequiresClose,
            )
        }
        Some(Err(_)) | None => {
            log::warn!("codex_worktree_process_check_failed");
            (
                crate::cleanup::CleanupSourceBlockReason::IncompleteMeasurement,
                ScanItemStatus::Limited,
            )
        }
    };
    for source in &mut result.sources {
        if paths.contains(&source.path) && source.block_reason.is_none() {
            source.block_reason = Some(reason);
        }
    }
    // Known writers are a pre-cleanup warning, not a selection restriction.
    // Execution still checks live processes before deleting each candidate.
    // Unknown process state and incomplete measurements remain hard blocks.
    let is_selectable = |source: &&CleanupSourceDetail| {
        source.block_reason.is_none()
            || source.block_reason == Some(crate::cleanup::CleanupSourceBlockReason::RequiresClose)
    };
    if result.sources.iter().filter(is_selectable).count() == 0 {
        result.selectable = false;
        if result.status != ScanItemStatus::Limited {
            result.status = status;
        }
    } else if result.selectable {
        // Keep hard-blocked source estimates for inspection without including
        // them in selectable totals. The source inventory is complete here.
        result.bytes = result
            .sources
            .iter()
            .filter(is_selectable)
            .map(|source| source.bytes)
            .sum();
        result.file_count = result
            .sources
            .iter()
            .filter(is_selectable)
            .map(|source| source.file_count)
            .sum();
    }
    log::info!(
        "codex_worktree_sources_guarded rule_id={} source_count={} process_count={} selectable_bytes={}",
        result.rule_id,
        paths.len(),
        result.running_processes.len(),
        if result.selectable { result.bytes } else { 0 }
    );
}

fn cleanup_source_details(candidates: &[ArtifactCandidate]) -> CleanupSourceSummary {
    let source_count = candidates.len() as u64;
    let mut sources = candidates
        .iter()
        .map(|candidate| CleanupSourceDetail {
            path: display_path(&candidate.path),
            bytes: candidate.bytes,
            file_count: candidate.file_count,
            modified_at_ms: candidate.modified_at_ms,
            block_reason: candidate
                .measurement_limited
                .then_some(crate::cleanup::CleanupSourceBlockReason::IncompleteMeasurement),
        })
        .collect::<Vec<_>>();
    sources.sort_by(|left, right| {
        right
            .bytes
            .cmp(&left.bytes)
            .then_with(|| left.path.cmp(&right.path))
    });
    CleanupSourceSummary {
        sources,
        source_count,
    }
}

fn failed_action(id: &str, reason: CleanupActionReason) -> CleanupActionResult {
    CleanupActionResult {
        rule_id: id.to_string(),
        action_kind: CleanupActionKind::Delete,
        status: CleanupActionStatus::Failed,
        reason_code: Some(reason),
        bytes_expected: 0,
        released_bytes: 0,
        affected_item_count: 0,
        failed_item_count: 1,
        running_processes: Vec::new(),
    }
}

fn current_platform_rules() -> Result<&'static [ProjectArtifactRuleSource], String> {
    match CURRENT_PLATFORM_RULES.get_or_init(|| {
        parse_catalog(EMBEDDED_PROJECT_ARTIFACT_RULE_SOURCES).map(|rules| {
            rules
                .into_iter()
                .filter(|rule| {
                    rule.platforms
                        .iter()
                        .any(|platform| platform == current_platform_name())
                })
                .collect()
        })
    }) {
        Ok(rules) => Ok(rules),
        Err(error) => Err(error.clone()),
    }
}

#[cfg(target_os = "macos")]
const fn current_platform_name() -> &'static str {
    "macos"
}

#[cfg(windows)]
const fn current_platform_name() -> &'static str {
    "windows"
}

#[cfg(windows)]
fn path_name_eq(left: &str, right: &str) -> bool {
    left.eq_ignore_ascii_case(right)
}

#[cfg(not(windows))]
fn path_name_eq(left: &str, right: &str) -> bool {
    left == right
}

#[cfg(windows)]
fn path_name_ends_with(value: &str, suffix: &str) -> bool {
    value
        .to_ascii_lowercase()
        .ends_with(&suffix.to_ascii_lowercase())
}

#[cfg(not(windows))]
fn path_name_ends_with(value: &str, suffix: &str) -> bool {
    value.ends_with(suffix)
}

#[cfg(test)]
#[path = "project_artifacts_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "project_artifacts_execution_tests.rs"]
mod execution_tests;
