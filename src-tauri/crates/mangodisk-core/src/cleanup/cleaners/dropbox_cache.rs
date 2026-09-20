//! Dropbox temporary-cache cleaner for Windows and macOS.
//!
//! Dropbox account directories can be moved and personal and team accounts can
//! coexist, so a guessed `Dropbox` folder is not a safe root. This cleaner
//! trusts only absolute account paths from Dropbox's own `info.json`. On macOS
//! File Provider setups it separately discovers Apple's managed Dropbox group
//! container, as documented by Dropbox. Both layouts narrow deletion boundaries
//! to direct children of the vendor-defined cache root.

use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

#[cfg(any(windows, target_os = "macos"))]
use std::{
    sync::{Mutex, OnceLock},
    time::Instant,
};

use mangodisk_platform::{current_platform, Platform};
use serde_json::Value;

use crate::{
    cleanup::measurement::measure_path_filtered,
    filesystem::{
        metadata::{is_link_like, modified_ms},
        permanent_delete::{delete_path_permanently, prepare_path_for_permanent_delete},
    },
    shared::operation::OperationGuard,
};

#[cfg(any(windows, target_os = "macos"))]
use crate::filesystem::metadata::display_path;

#[cfg(any(windows, target_os = "macos"))]
use crate::{
    applications::catalog::ProcessSnapshot,
    cleanup::{
        source_selection::SourceScope, CleanupActionKind, CleanupActionReason, CleanupActionResult,
        CleanupActionStatus, CleanupCategory, CleanupGroup, CleanupSourceDetail, RiskLevel,
        ScanItemStatus, ScanRuleResult,
    },
};

#[cfg(any(windows, target_os = "macos"))]
pub(super) const CLEANER_ID: &str = "special.dropbox-cache";
#[cfg(any(windows, target_os = "macos"))]
pub(super) const CLEANER_REVISION: &str =
    "dropbox-cache-v3-macos-file-provider-process-owners-and-live-revalidation";

const CACHE_DIRECTORY_NAME: &str = ".dropbox.cache";
#[cfg(target_os = "macos")]
const FILE_PROVIDER_CONTAINER_SUFFIX: &str = ".com.getdropbox.dropbox.sync";
#[cfg(target_os = "macos")]
const FILE_PROVIDER_CACHE_DIRECTORY_NAME: &str = "root-mount";
const MAX_CONFIG_BYTES: u64 = 1024 * 1024;
#[cfg(any(windows, target_os = "macos"))]
const MAX_PREVIEW_SOURCES: usize = 256;
#[cfg(windows)]
const REQUIRED_STOPPED_PROCESSES: [&str; 2] = ["Dropbox.exe", "DropboxUpdate.exe"];
#[cfg(target_os = "macos")]
const REQUIRED_STOPPED_PROCESSES: [&str; 5] = [
    "Dropbox",
    "DropboxFileProviderExtension",
    "DropboxFileProvider",
    "DropboxFileProviderCH",
    "DropboxActivityProvider",
];

#[cfg(any(windows, target_os = "macos"))]
static LAST_PREVIEW: OnceLock<Mutex<Option<Vec<DropboxCacheCandidate>>>> = OnceLock::new();

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DropboxCacheLayout {
    AccountRoot,
    #[cfg(target_os = "macos")]
    FileProvider,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DropboxCacheCandidate {
    layout: DropboxCacheLayout,
    cache_root: PathBuf,
    path: PathBuf,
    bytes: u64,
    file_count: u64,
    modified_at_ms: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DiscoveryError {
    NotApplicable,
    Cancelled,
    Incomplete,
}

#[cfg(any(windows, target_os = "macos"))]
pub(super) fn preview(
    is_cancelled: &(dyn Fn() -> bool + Sync),
    report_path: &(dyn Fn(&Path) + Sync),
    report_files: &(dyn Fn(&Path, u64, u64) + Sync),
) -> ScanRuleResult {
    let started = Instant::now();
    replace_preview(None);
    let candidates = match discover_platform_candidates(is_cancelled, report_path, report_files) {
        Ok(candidates) => candidates,
        Err(DiscoveryError::NotApplicable) => {
            return unavailable_rule(
                ScanItemStatus::NotApplicable,
                started.elapsed().as_millis() as u64,
            );
        }
        Err(DiscoveryError::Cancelled) => {
            return unavailable_rule(
                ScanItemStatus::Limited,
                started.elapsed().as_millis() as u64,
            );
        }
        Err(DiscoveryError::Incomplete) => {
            log::warn!("dropbox_cache_preview_limited reason=incompleteDiscovery");
            return unavailable_rule(
                ScanItemStatus::Limited,
                started.elapsed().as_millis() as u64,
            );
        }
    };
    let running_processes = match running_dropbox_processes() {
        Ok(processes) => processes,
        Err(()) => {
            log::warn!("dropbox_cache_preview_limited reason=processSnapshotUnavailable");
            return unavailable_rule(
                ScanItemStatus::Limited,
                started.elapsed().as_millis() as u64,
            );
        }
    };
    if !replace_preview(Some(candidates.clone())) {
        log::warn!("dropbox_cache_preview_limited reason=previewSnapshotUnavailable");
        return unavailable_rule(
            ScanItemStatus::Limited,
            started.elapsed().as_millis() as u64,
        );
    }
    candidate_rule(
        candidates,
        running_processes,
        started.elapsed().as_millis() as u64,
    )
}

#[cfg(any(windows, target_os = "macos"))]
pub(super) fn limited_rule() -> ScanRuleResult {
    replace_preview(None);
    unavailable_rule(ScanItemStatus::Limited, 0)
}

#[cfg(any(windows, target_os = "macos"))]
pub(super) fn execute(
    source_scope: Option<&SourceScope>,
    dry_run: bool,
    operation: &OperationGuard,
) -> CleanupActionResult {
    let Some(expected_all) = preview_snapshot() else {
        log::warn!("dropbox_cache_preflight_failed reason=missingPreview");
        return failed_action(0, CleanupActionReason::PreflightFailed, Vec::new());
    };
    if source_scope.is_some_and(|scope| {
        scope
            .validate_known_paths(
                expected_all
                    .iter()
                    .map(|candidate| candidate.path.as_path()),
            )
            .is_err()
    }) {
        return failed_action(0, CleanupActionReason::PreflightFailed, Vec::new());
    }
    let selected = expected_all
        .iter()
        .filter(|candidate| source_scope.is_none_or(|scope| scope.selects(&candidate.path)))
        .cloned()
        .collect::<Vec<_>>();
    let expected_bytes = selected.iter().map(|candidate| candidate.bytes).sum();

    let running_processes = match running_dropbox_processes() {
        Ok(processes) => processes,
        Err(()) => {
            log::warn!("dropbox_cache_preflight_failed reason=processSnapshotUnavailable");
            return failed_action(
                expected_bytes,
                CleanupActionReason::PreflightFailed,
                Vec::new(),
            );
        }
    };
    if !running_processes.is_empty() {
        return failed_action(
            expected_bytes,
            CleanupActionReason::RunningProcesses,
            running_processes,
        );
    }

    let cancelled = || {
        operation
            .cancelled()
            .load(std::sync::atomic::Ordering::Relaxed)
    };
    let actual = match discover_platform_candidates(&cancelled, &|_| {}, &|_, _, _| {}) {
        Ok(candidates) => candidates,
        Err(_) => {
            log::warn!("dropbox_cache_preflight_failed reason=rediscoveryFailed");
            return failed_action(
                expected_bytes,
                CleanupActionReason::PreflightFailed,
                Vec::new(),
            );
        }
    };
    if actual != expected_all {
        log::warn!(
            "dropbox_cache_preflight_failed reason=candidateSnapshotChanged expected_count={} actual_count={}",
            expected_all.len(),
            actual.len()
        );
        return failed_action(
            expected_bytes,
            CleanupActionReason::PreflightFailed,
            Vec::new(),
        );
    }
    if dry_run {
        return completed_action(
            CleanupActionStatus::Previewed,
            expected_bytes,
            0,
            0,
            0,
            None,
            Vec::new(),
        );
    }

    // Dropbox requires the client to be fully closed before cache cleanup. A
    // single preflight proves only one point in time, so recapture processes at
    // every deletion boundary in case the user restarts Dropbox mid-operation.
    let outcome = delete_candidates(&selected, operation, &mut running_dropbox_processes);
    replace_preview(None);
    log::info!(
        "dropbox_cache_cleanup_finished candidate_count={} expected_bytes={} released_bytes={} affected_items={} failed_items={} cancelled={} process_snapshot_unavailable={} running_process_count={}",
        selected.len(),
        expected_bytes,
        outcome.released_bytes,
        outcome.affected_items,
        outcome.failed_items,
        outcome.cancelled,
        outcome.process_snapshot_unavailable,
        outcome.running_processes.len()
    );
    let status = if (outcome.cancelled || !outcome.running_processes.is_empty())
        && outcome.affected_items == 0
    {
        CleanupActionStatus::Blocked
    } else if outcome.failed_items == 0 {
        CleanupActionStatus::Completed
    } else if outcome.affected_items > 0 {
        CleanupActionStatus::Partial
    } else {
        CleanupActionStatus::Failed
    };
    let reason = if outcome.cancelled {
        Some(CleanupActionReason::Cancelled)
    } else if !outcome.running_processes.is_empty() {
        Some(CleanupActionReason::RunningProcesses)
    } else if outcome.process_snapshot_unavailable {
        Some(CleanupActionReason::PreflightFailed)
    } else {
        (outcome.failed_items > 0).then_some(CleanupActionReason::ItemsSkipped)
    };
    completed_action(
        status,
        expected_bytes,
        outcome.released_bytes,
        outcome.affected_items,
        outcome.failed_items,
        reason,
        outcome.running_processes,
    )
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct DeleteOutcome {
    released_bytes: u64,
    affected_items: u64,
    failed_items: u64,
    cancelled: bool,
    process_snapshot_unavailable: bool,
    running_processes: Vec<String>,
}

fn delete_candidates<F>(
    candidates: &[DropboxCacheCandidate],
    operation: &OperationGuard,
    process_guard: &mut F,
) -> DeleteOutcome
where
    F: FnMut() -> Result<Vec<String>, ()>,
{
    let mut outcome = DeleteOutcome::default();
    for candidate in candidates {
        if operation.ensure_not_cancelled().is_err() {
            outcome.cancelled = true;
            break;
        }
        match process_guard() {
            Ok(running_processes) if running_processes.is_empty() => {}
            Ok(running_processes) => {
                outcome.failed_items = outcome.failed_items.saturating_add(1);
                outcome.running_processes = running_processes;
                log::warn!("dropbox_cache_delete_blocked reason=applicationRestarted");
                break;
            }
            Err(()) => {
                outcome.failed_items = outcome.failed_items.saturating_add(1);
                outcome.process_snapshot_unavailable = true;
                log::warn!("dropbox_cache_delete_blocked reason=processSnapshotUnavailable");
                break;
            }
        }
        if !candidate_has_safe_boundary(candidate) {
            outcome.failed_items = outcome.failed_items.saturating_add(1);
            continue;
        }
        let prepared = match prepare_path_for_permanent_delete(&candidate.path) {
            Ok(prepared) => prepared,
            Err(error) => {
                outcome.failed_items = outcome.failed_items.saturating_add(1);
                log::warn!(
                    "dropbox_cache_delete_skipped reason=identityCaptureFailed error={}",
                    mangodisk_platform::diagnostics::text(&error)
                );
                continue;
            }
        };
        // Measure the same direct child immediately before deletion. This stops
        // MangoDisk from deleting a tree that differs from the preview after a
        // late write, and also fails closed on links or unreadable entries.
        let live = measure_path_filtered(&candidate.path, None, &|_, _| true);
        if live.skipped_count > 0
            || live.bytes != candidate.bytes
            || live.file_count != candidate.file_count
        {
            outcome.failed_items = outcome.failed_items.saturating_add(1);
            continue;
        }
        match delete_path_permanently(prepared, live.bytes, live.file_count) {
            Ok(()) => {
                outcome.released_bytes = outcome.released_bytes.saturating_add(live.bytes);
                outcome.affected_items = outcome.affected_items.saturating_add(live.file_count);
            }
            Err(error) => {
                outcome.released_bytes = outcome
                    .released_bytes
                    .saturating_add(error.released_bytes());
                outcome.affected_items = outcome
                    .affected_items
                    .saturating_add(error.affected_item_count());
                outcome.failed_items = outcome.failed_items.saturating_add(1);
                log::warn!(
                    "dropbox_cache_delete_failed partial={} released_bytes={} affected_item_count={} error={}",
                    error.is_partial(),
                    error.released_bytes(),
                    error.affected_item_count(),
                    mangodisk_platform::diagnostics::text(&error)
                );
            }
        }
    }
    outcome
}

#[cfg(windows)]
fn dropbox_config_paths() -> Result<Vec<PathBuf>, ()> {
    let directories = current_platform().user_directories().map_err(|_| ())?;
    let mut paths = directories
        .application_storage_directories()
        .iter()
        .map(|root| root.join("Dropbox/info.json"))
        .collect::<Vec<_>>();
    paths.sort_by_key(|path| normalized_path_key(path));
    paths.dedup_by(|left, right| normalized_path_key(left) == normalized_path_key(right));
    Ok(paths)
}

#[cfg(target_os = "macos")]
fn dropbox_config_paths() -> Result<Vec<PathBuf>, ()> {
    let directories = current_platform().user_directories().map_err(|_| ())?;
    Ok(vec![directories
        .home_directory()
        .join(".dropbox/info.json")])
}

#[cfg(any(windows, target_os = "macos"))]
fn running_dropbox_processes() -> Result<Vec<String>, ()> {
    let names = REQUIRED_STOPPED_PROCESSES
        .iter()
        .map(|name| (*name).to_string())
        .collect::<Vec<_>>();
    ProcessSnapshot::capture()
        .map(|snapshot| snapshot.matching_processes(&names))
        .map_err(|_| ())
}

#[cfg(any(windows, target_os = "macos"))]
fn discover_platform_candidates(
    is_cancelled: &(dyn Fn() -> bool + Sync),
    report_path: &(dyn Fn(&Path) + Sync),
    report_files: &(dyn Fn(&Path, u64, u64) + Sync),
) -> Result<Vec<DropboxCacheCandidate>, DiscoveryError> {
    let config_paths = dropbox_config_paths().map_err(|_| DiscoveryError::Incomplete)?;
    #[cfg(windows)]
    {
        discover_candidates(&config_paths, is_cancelled, report_path, report_files)
    }
    #[cfg(target_os = "macos")]
    {
        discover_macos_candidates(&config_paths, is_cancelled, report_path, report_files)
    }
}

#[cfg(target_os = "macos")]
fn discover_macos_candidates(
    config_paths: &[PathBuf],
    is_cancelled: &(dyn Fn() -> bool + Sync),
    report_path: &(dyn Fn(&Path) + Sync),
    report_files: &(dyn Fn(&Path, u64, u64) + Sync),
) -> Result<Vec<DropboxCacheCandidate>, DiscoveryError> {
    let mut roots = Vec::new();
    let legacy_config_exists = config_paths.iter().any(|path| path.exists());
    if legacy_config_exists {
        roots.extend(
            read_account_roots(config_paths, is_cancelled)?
                .into_iter()
                .map(|account_root| DropboxCacheRoot {
                    layout: DropboxCacheLayout::AccountRoot,
                    path: account_root.join(CACHE_DIRECTORY_NAME),
                }),
        );
    }
    roots.extend(discover_file_provider_cache_roots(is_cancelled)?);
    if roots.is_empty() {
        return Err(DiscoveryError::NotApplicable);
    }
    discover_candidates_in_roots(&roots, is_cancelled, report_path, report_files)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DropboxCacheRoot {
    layout: DropboxCacheLayout,
    path: PathBuf,
}

#[cfg(target_os = "macos")]
fn discover_file_provider_cache_roots(
    is_cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<Vec<DropboxCacheRoot>, DiscoveryError> {
    let directories = current_platform()
        .user_directories()
        .map_err(|_| DiscoveryError::Incomplete)?;
    let group_containers = directories
        .home_directory()
        .join("Library/Group Containers");
    discover_file_provider_cache_roots_in(&group_containers, is_cancelled)
}

#[cfg(target_os = "macos")]
fn discover_file_provider_cache_roots_in(
    group_containers: &Path,
    is_cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<Vec<DropboxCacheRoot>, DiscoveryError> {
    let entries = match fs::read_dir(group_containers) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(_) => return Err(DiscoveryError::Incomplete),
    };
    let mut roots = Vec::new();
    for entry in entries {
        if is_cancelled() {
            return Err(DiscoveryError::Cancelled);
        }
        let entry = entry.map_err(|_| DiscoveryError::Incomplete)?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if !name.ends_with(FILE_PROVIDER_CONTAINER_SUFFIX) {
            continue;
        }
        let container = entry.path();
        let metadata = fs::symlink_metadata(&container).map_err(|_| DiscoveryError::Incomplete)?;
        if is_link_like(&metadata) || !metadata.is_dir() {
            return Err(DiscoveryError::Incomplete);
        }
        current_platform()
            .validate_path_no_links(&container)
            .map_err(|_| DiscoveryError::Incomplete)?;
        roots.push(DropboxCacheRoot {
            layout: DropboxCacheLayout::FileProvider,
            path: container.join(FILE_PROVIDER_CACHE_DIRECTORY_NAME),
        });
    }
    roots.sort_by_key(|root| normalized_path_key(&root.path));
    roots.dedup_by(|left, right| {
        normalized_path_key(&left.path) == normalized_path_key(&right.path)
    });
    Ok(roots)
}

#[cfg(any(windows, test))]
fn discover_candidates(
    config_paths: &[PathBuf],
    is_cancelled: &(dyn Fn() -> bool + Sync),
    report_path: &(dyn Fn(&Path) + Sync),
    report_files: &(dyn Fn(&Path, u64, u64) + Sync),
) -> Result<Vec<DropboxCacheCandidate>, DiscoveryError> {
    // Resolve every account root from configuration before appending the fixed
    // cache leaf. This supports custom drives and team accounts without carrying
    // broad third-party wildcards into a permanent-deletion path.
    let roots = read_account_roots(config_paths, is_cancelled)?
        .into_iter()
        .map(|account_root| DropboxCacheRoot {
            layout: DropboxCacheLayout::AccountRoot,
            path: account_root.join(CACHE_DIRECTORY_NAME),
        })
        .collect::<Vec<_>>();
    discover_candidates_in_roots(&roots, is_cancelled, report_path, report_files)
}

fn discover_candidates_in_roots(
    roots: &[DropboxCacheRoot],
    is_cancelled: &(dyn Fn() -> bool + Sync),
    report_path: &(dyn Fn(&Path) + Sync),
    report_files: &(dyn Fn(&Path, u64, u64) + Sync),
) -> Result<Vec<DropboxCacheCandidate>, DiscoveryError> {
    let mut candidates = Vec::new();
    let mut visited_cache_roots = HashSet::new();
    for root in roots {
        if is_cancelled() {
            return Err(DiscoveryError::Cancelled);
        }
        let cache_root = root.path.clone();
        let cache_key = normalized_path_key(&cache_root);
        if !visited_cache_roots.insert(cache_key) || !cache_root.exists() {
            continue;
        }
        report_path(&cache_root);
        let metadata = fs::symlink_metadata(&cache_root).map_err(|_| DiscoveryError::Incomplete)?;
        if is_link_like(&metadata) || !metadata.is_dir() {
            return Err(DiscoveryError::Incomplete);
        }
        current_platform()
            .validate_path_no_links(&cache_root)
            .map_err(|_| DiscoveryError::Incomplete)?;
        let entries = fs::read_dir(&cache_root).map_err(|_| DiscoveryError::Incomplete)?;
        for entry in entries {
            if is_cancelled() {
                return Err(DiscoveryError::Cancelled);
            }
            let entry = entry.map_err(|_| DiscoveryError::Incomplete)?;
            let path = entry.path();
            report_path(&path);
            let metadata = fs::symlink_metadata(&path).map_err(|_| DiscoveryError::Incomplete)?;
            if is_link_like(&metadata) {
                return Err(DiscoveryError::Incomplete);
            }
            let measured = measure_path_filtered(&path, None, &|_, _| true);
            if measured.skipped_count > 0 {
                return Err(DiscoveryError::Incomplete);
            }
            // Empty directories reclaim nothing and should not create selectable
            // source rows with zero benefit.
            if measured.file_count == 0 && measured.bytes == 0 {
                continue;
            }
            report_files(&path, measured.file_count, measured.bytes);
            candidates.push(DropboxCacheCandidate {
                layout: root.layout,
                cache_root: cache_root.clone(),
                path,
                bytes: measured.bytes,
                file_count: measured.file_count,
                modified_at_ms: modified_ms(&metadata),
            });
        }
    }
    candidates.sort_by(|left, right| {
        right
            .bytes
            .cmp(&left.bytes)
            .then_with(|| normalized_path_key(&left.path).cmp(&normalized_path_key(&right.path)))
    });
    Ok(candidates)
}

fn read_account_roots(
    config_paths: &[PathBuf],
    is_cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<Vec<PathBuf>, DiscoveryError> {
    let mut found_config = false;
    let mut roots = Vec::new();
    let mut root_keys = HashSet::new();
    for config_path in config_paths {
        if is_cancelled() {
            return Err(DiscoveryError::Cancelled);
        }
        // The configuration controls later scan roots and is therefore a trust
        // boundary. Links, oversized files, invalid JSON, and relative paths all
        // make discovery incomplete; never fall back to a guessed Dropbox root.
        let metadata = match fs::symlink_metadata(config_path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => return Err(DiscoveryError::Incomplete),
        };
        found_config = true;
        if is_link_like(&metadata) || !metadata.is_file() || metadata.len() > MAX_CONFIG_BYTES {
            return Err(DiscoveryError::Incomplete);
        }
        current_platform()
            .validate_path_no_links(config_path)
            .map_err(|_| DiscoveryError::Incomplete)?;
        let bytes = fs::read(config_path).map_err(|_| DiscoveryError::Incomplete)?;
        if bytes.len() as u64 > MAX_CONFIG_BYTES {
            return Err(DiscoveryError::Incomplete);
        }
        let document: Value =
            serde_json::from_slice(&bytes).map_err(|_| DiscoveryError::Incomplete)?;
        let accounts = document.as_object().ok_or(DiscoveryError::Incomplete)?;
        for account_name in ["personal", "business"] {
            let Some(account) = accounts.get(account_name) else {
                continue;
            };
            let path = account
                .as_object()
                .and_then(|object| object.get("path"))
                .and_then(Value::as_str)
                .filter(|path| !path.is_empty())
                .map(PathBuf::from)
                .ok_or(DiscoveryError::Incomplete)?;
            if !path.is_absolute() {
                return Err(DiscoveryError::Incomplete);
            }
            if root_keys.insert(normalized_path_key(&path)) {
                roots.push(path);
            }
        }
    }
    if !found_config {
        return Err(DiscoveryError::NotApplicable);
    }
    if roots.is_empty() {
        return Err(DiscoveryError::Incomplete);
    }
    roots.sort_by_key(|path| normalized_path_key(path));
    Ok(roots)
}

fn candidate_has_safe_boundary(candidate: &DropboxCacheCandidate) -> bool {
    let root_name_is_expected = match candidate.layout {
        DropboxCacheLayout::AccountRoot => candidate
            .cache_root
            .file_name()
            .is_some_and(|name| name == CACHE_DIRECTORY_NAME),
        #[cfg(target_os = "macos")]
        DropboxCacheLayout::FileProvider => {
            candidate
                .cache_root
                .file_name()
                .is_some_and(|name| name == FILE_PROVIDER_CACHE_DIRECTORY_NAME)
                && candidate
                    .cache_root
                    .parent()
                    .and_then(Path::file_name)
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.ends_with(FILE_PROVIDER_CONTAINER_SUFFIX))
        }
    };
    root_name_is_expected
        && candidate.path.parent() == Some(candidate.cache_root.as_path())
        && candidate.path != candidate.cache_root
}

fn normalized_path_key(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/").to_lowercase()
}

#[cfg(any(windows, target_os = "macos"))]
fn replace_preview(next: Option<Vec<DropboxCacheCandidate>>) -> bool {
    LAST_PREVIEW
        .get_or_init(|| Mutex::new(None))
        .lock()
        .map(|mut preview| *preview = next)
        .is_ok()
}

#[cfg(any(windows, target_os = "macos"))]
fn preview_snapshot() -> Option<Vec<DropboxCacheCandidate>> {
    LAST_PREVIEW
        .get_or_init(|| Mutex::new(None))
        .lock()
        .ok()
        .and_then(|preview| preview.clone())
}

#[cfg(any(windows, target_os = "macos"))]
fn candidate_rule(
    candidates: Vec<DropboxCacheCandidate>,
    running_processes: Vec<String>,
    elapsed_ms: u64,
) -> ScanRuleResult {
    let bytes = candidates.iter().map(|candidate| candidate.bytes).sum();
    let file_count = candidates
        .iter()
        .map(|candidate| candidate.file_count)
        .sum();
    let source_count = candidates.len() as u64;
    let sources = candidates
        .iter()
        .take(MAX_PREVIEW_SOURCES)
        .map(|candidate| CleanupSourceDetail {
            path: display_path(&candidate.path),
            bytes: candidate.bytes,
            file_count: candidate.file_count,
            modified_at_ms: candidate.modified_at_ms,
            block_reason: None,
        })
        .collect::<Vec<_>>();
    let status = if candidates.is_empty() {
        ScanItemStatus::Clean
    } else if running_processes.is_empty() {
        ScanItemStatus::Found
    } else {
        ScanItemStatus::RequiresClose
    };
    ScanRuleResult {
        rule_id: CLEANER_ID.to_string(),
        category: CleanupCategory::Application,
        group: CleanupGroup::Application,
        risk: RiskLevel::Recoverable,
        default_selected: false,
        recommended_selected: true,
        bytes,
        file_count,
        available: true,
        selectable: !candidates.is_empty(),
        status,
        running_processes,
        requires_app_close: true,
        sources,
        source_count,
        sources_truncated: source_count > MAX_PREVIEW_SOURCES as u64,
        scan_elapsed_ms: elapsed_ms,
    }
}

#[cfg(any(windows, target_os = "macos"))]
fn unavailable_rule(status: ScanItemStatus, elapsed_ms: u64) -> ScanRuleResult {
    ScanRuleResult {
        rule_id: CLEANER_ID.to_string(),
        category: CleanupCategory::Application,
        group: CleanupGroup::Application,
        risk: RiskLevel::Recoverable,
        default_selected: false,
        recommended_selected: true,
        bytes: 0,
        file_count: 0,
        available: status != ScanItemStatus::NotApplicable,
        selectable: false,
        status,
        running_processes: Vec::new(),
        requires_app_close: true,
        sources: Vec::new(),
        source_count: 0,
        sources_truncated: false,
        scan_elapsed_ms: elapsed_ms,
    }
}

#[cfg(any(windows, target_os = "macos"))]
fn failed_action(
    bytes_expected: u64,
    reason: CleanupActionReason,
    running_processes: Vec<String>,
) -> CleanupActionResult {
    CleanupActionResult {
        rule_id: CLEANER_ID.to_string(),
        action_kind: CleanupActionKind::Delete,
        status: if matches!(
            reason,
            CleanupActionReason::Cancelled | CleanupActionReason::RunningProcesses
        ) {
            CleanupActionStatus::Blocked
        } else {
            CleanupActionStatus::Failed
        },
        reason_code: Some(reason),
        bytes_expected,
        released_bytes: 0,
        affected_item_count: 0,
        failed_item_count: 1,
        running_processes,
    }
}

#[cfg(any(windows, target_os = "macos"))]
fn completed_action(
    status: CleanupActionStatus,
    bytes_expected: u64,
    released_bytes: u64,
    affected_item_count: u64,
    failed_item_count: u64,
    reason_code: Option<CleanupActionReason>,
    running_processes: Vec<String>,
) -> CleanupActionResult {
    CleanupActionResult {
        rule_id: CLEANER_ID.to_string(),
        action_kind: CleanupActionKind::Delete,
        status,
        reason_code,
        bytes_expected,
        released_bytes,
        affected_item_count,
        failed_item_count,
        running_processes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(any(windows, target_os = "macos"))]
    use crate::cleanup::{
        source_selection::SourceSelectionPolicy, CleanupSourceSelection, CleanupSourceSelectionMode,
    };
    use crate::shared::operation::CoordinatedOperationKind;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn test_directory(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "mangodisk-dropbox-cache-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("the system clock must be after the Unix epoch")
                .as_nanos()
        ));
        fs::create_dir_all(&path).expect("the Dropbox fixture root must be created");
        path
    }

    #[cfg(any(windows, target_os = "macos"))]
    #[test]
    fn dropbox_cache_is_recommended_for_interactive_cleanup() {
        let rule = candidate_rule(Vec::new(), Vec::new(), 0);

        assert!(!rule.default_selected);
        assert!(rule.recommended_selected);
    }

    fn write_info_json(config: &Path, personal: &Path, business: Option<&Path>) {
        fs::create_dir_all(config.parent().expect("the config must have a parent")).unwrap();
        let mut document = serde_json::json!({
            "personal": { "path": personal, "host": 1, "is_team": false }
        });
        if let Some(business) = business {
            document["business"] = serde_json::json!({
                "path": business,
                "host": 2,
                "is_team": true
            });
        }
        fs::write(config, serde_json::to_vec(&document).unwrap()).unwrap();
    }

    #[test]
    fn info_json_discovers_custom_personal_and_business_roots_once() {
        let fixture = test_directory("accounts");
        let config = fixture.join("AppData/Dropbox/info.json");
        let duplicate = fixture.join("LocalAppData/Dropbox/info.json");
        let personal = fixture.join("Custom/Dropbox Personal");
        let business = fixture.join("External/Organization Dropbox");
        write_info_json(&config, &personal, Some(&business));
        write_info_json(&duplicate, &personal, Some(&business));

        let roots = read_account_roots(&[config, duplicate], &|| false).unwrap();

        assert_eq!(roots.len(), 2);
        assert!(roots.contains(&personal));
        assert!(roots.contains(&business));
        fs::remove_dir_all(fixture).unwrap();
    }

    #[test]
    fn malformed_or_relative_account_paths_fail_closed() {
        let fixture = test_directory("invalid-config");
        let malformed = fixture.join("malformed/Dropbox/info.json");
        fs::create_dir_all(malformed.parent().unwrap()).unwrap();
        fs::write(&malformed, b"not-json").unwrap();
        assert_eq!(
            read_account_roots(&[malformed], &|| false),
            Err(DiscoveryError::Incomplete)
        );

        let relative = fixture.join("relative/Dropbox/info.json");
        fs::create_dir_all(relative.parent().unwrap()).unwrap();
        fs::write(&relative, br#"{"personal":{"path":"relative/dropbox"}}"#).unwrap();
        assert_eq!(
            read_account_roots(&[relative], &|| false),
            Err(DiscoveryError::Incomplete)
        );
        fs::remove_dir_all(fixture).unwrap();
    }

    #[test]
    fn discovery_only_returns_direct_children_of_dropbox_cache() {
        let fixture = test_directory("discovery");
        let config = fixture.join("AppData/Dropbox/info.json");
        let account = fixture.join("Custom Dropbox");
        let cache = account.join(CACHE_DIRECTORY_NAME);
        let first = cache.join("old-files");
        let second = cache.join("staging.bin");
        fs::create_dir_all(first.join("nested")).unwrap();
        fs::write(first.join("nested/payload.bin"), b"payload").unwrap();
        fs::write(&second, b"stage").unwrap();
        fs::write(account.join("user-document.txt"), b"keep").unwrap();
        write_info_json(&config, &account, None);

        let candidates = discover_candidates(&[config], &|| false, &|_| {}, &|_, _, _| {}).unwrap();

        assert_eq!(candidates.len(), 2);
        assert!(candidates.iter().any(|candidate| candidate.path == first));
        assert!(candidates.iter().any(|candidate| candidate.path == second));
        assert!(candidates
            .iter()
            .all(|candidate| candidate.path.parent() == Some(cache.as_path())));
        fs::remove_dir_all(fixture).unwrap();
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn file_provider_discovery_accepts_only_documented_group_container_roots() {
        let fixture = test_directory("file-provider-roots");
        let group_containers = fixture.join("Library/Group Containers");
        let valid_container = group_containers.join("123456.com.getdropbox.dropbox.sync");
        let unrelated_container = group_containers.join("123456.com.example.sync");
        fs::create_dir_all(valid_container.join(FILE_PROVIDER_CACHE_DIRECTORY_NAME)).unwrap();
        fs::create_dir_all(unrelated_container.join(FILE_PROVIDER_CACHE_DIRECTORY_NAME)).unwrap();

        let roots = discover_file_provider_cache_roots_in(&group_containers, &|| false).unwrap();

        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0].layout, DropboxCacheLayout::FileProvider);
        assert_eq!(
            roots[0].path,
            valid_container.join(FILE_PROVIDER_CACHE_DIRECTORY_NAME)
        );
        fs::remove_dir_all(fixture).unwrap();
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn file_provider_execution_deletes_cache_children_and_preserves_container_data() {
        let _operation_lock = crate::shared::operation::test_operation_lock();
        let fixture = test_directory("file-provider-execution");
        let container = fixture.join("123456.com.getdropbox.dropbox.sync");
        let cache_root = container.join(FILE_PROVIDER_CACHE_DIRECTORY_NAME);
        let cache_child = cache_root.join("temporary-transfer");
        let preserved = container.join("container-state.db");
        fs::create_dir_all(&cache_child).unwrap();
        fs::write(cache_child.join("payload.bin"), b"payload").unwrap();
        fs::write(&preserved, b"keep").unwrap();
        let roots = vec![DropboxCacheRoot {
            layout: DropboxCacheLayout::FileProvider,
            path: cache_root.clone(),
        }];
        let candidates =
            discover_candidates_in_roots(&roots, &|| false, &|_| {}, &|_, _, _| {}).unwrap();
        let operation = OperationGuard::start(CoordinatedOperationKind::Cleanup).unwrap();

        let outcome = delete_candidates(&candidates, &operation, &mut || Ok(Vec::new()));

        operation.complete();
        assert_eq!(outcome.failed_items, 0);
        assert_eq!(outcome.released_bytes, 7);
        assert!(!cache_child.exists());
        assert!(cache_root.exists());
        assert_eq!(fs::read(preserved).unwrap(), b"keep");
        fs::remove_dir_all(fixture).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn linked_cache_root_fails_closed() {
        use std::os::unix::fs::symlink;

        let fixture = test_directory("linked-root");
        let config = fixture.join("AppData/Dropbox/info.json");
        let account = fixture.join("Custom Dropbox");
        let target = fixture.join("outside");
        fs::create_dir_all(&account).unwrap();
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("payload.bin"), b"keep").unwrap();
        symlink(&target, account.join(CACHE_DIRECTORY_NAME)).unwrap();
        write_info_json(&config, &account, None);

        assert_eq!(
            discover_candidates(&[config], &|| false, &|_| {}, &|_, _, _| {}),
            Err(DiscoveryError::Incomplete)
        );
        assert!(target.join("payload.bin").exists());
        fs::remove_dir_all(fixture).unwrap();
    }

    #[test]
    fn fixture_execution_deletes_cache_children_and_preserves_account_data() {
        let _operation_lock = crate::shared::operation::test_operation_lock();
        let fixture = test_directory("execution");
        let config = fixture.join("AppData/Dropbox/info.json");
        let account = fixture.join("Custom Dropbox");
        let cache = account.join(CACHE_DIRECTORY_NAME);
        let candidate_path = cache.join("old-files");
        fs::create_dir_all(&candidate_path).unwrap();
        fs::write(candidate_path.join("payload.bin"), b"payload").unwrap();
        let user_document = account.join("user-document.txt");
        fs::write(&user_document, b"keep").unwrap();
        write_info_json(&config, &account, None);
        let candidates = discover_candidates(&[config], &|| false, &|_| {}, &|_, _, _| {}).unwrap();
        let operation = OperationGuard::start(CoordinatedOperationKind::Cleanup).unwrap();

        let outcome = delete_candidates(&candidates, &operation, &mut || Ok(Vec::new()));

        operation.complete();
        assert_eq!(outcome.failed_items, 0);
        assert_eq!(outcome.affected_items, 1);
        assert_eq!(outcome.released_bytes, 7);
        assert!(!candidate_path.exists());
        assert!(cache.exists());
        assert_eq!(fs::read(user_document).unwrap(), b"keep");
        fs::remove_dir_all(fixture).unwrap();
    }

    #[test]
    fn execution_stops_when_dropbox_restarts_between_candidates() {
        let _operation_lock = crate::shared::operation::test_operation_lock();
        let fixture = test_directory("process-restart");
        let config = fixture.join("AppData/Dropbox/info.json");
        let account = fixture.join("Custom Dropbox");
        let cache = account.join(CACHE_DIRECTORY_NAME);
        let first = cache.join("first");
        let second = cache.join("second");
        fs::create_dir_all(&first).unwrap();
        fs::create_dir_all(&second).unwrap();
        fs::write(first.join("payload.bin"), b"1234567").unwrap();
        fs::write(second.join("payload.bin"), b"12345").unwrap();
        write_info_json(&config, &account, None);
        let candidates = discover_candidates(&[config], &|| false, &|_| {}, &|_, _, _| {}).unwrap();
        let operation = OperationGuard::start(CoordinatedOperationKind::Cleanup).unwrap();
        let mut guard_calls = 0_u8;

        let outcome = delete_candidates(&candidates, &operation, &mut || {
            guard_calls = guard_calls.saturating_add(1);
            if guard_calls == 1 {
                Ok(Vec::new())
            } else {
                Ok(vec!["Dropbox.exe".to_string()])
            }
        });

        operation.complete();
        assert_eq!(outcome.affected_items, 1);
        assert_eq!(outcome.failed_items, 1);
        assert_eq!(outcome.running_processes, ["Dropbox.exe"]);
        assert!(!first.exists());
        assert!(second.exists());
        fs::remove_dir_all(fixture).unwrap();
    }

    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "requires a signed-in Dropbox client with an active cache owner"]
    fn real_macos_process_snapshot_blocks_active_owner_cleanup() {
        struct CacheFixtureCleanup(PathBuf);

        impl Drop for CacheFixtureCleanup {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }

        let _operation_lock = crate::shared::operation::test_operation_lock();
        let running = running_dropbox_processes()
            .expect("the macOS process inventory must be available for a real diagnostic");
        assert!(
            !running.is_empty(),
            "the real diagnostic requires one active Dropbox cache owner"
        );

        // A freshly synchronized File Provider cache can legitimately contain
        // only empty bookkeeping directories, which production preview omits.
        // Create one uniquely owned payload so the real process gate is tested
        // without assigning cleanup meaning to any user or Dropbox-owned entry.
        let cache_root = discover_file_provider_cache_roots(&|| false)
            .expect("the documented Dropbox File Provider root must be discoverable")
            .into_iter()
            .next()
            .expect("the real diagnostic requires one File Provider cache root")
            .path;
        let fixture_path = cache_root.join(format!(
            "mangodisk-running-process-validation-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("the system clock must be after the Unix epoch")
                .as_nanos()
        ));
        let _cleanup = CacheFixtureCleanup(fixture_path.clone());
        fs::create_dir(&fixture_path).expect("the isolated cache fixture must be created");
        fs::write(fixture_path.join("payload.bin"), b"payload")
            .expect("the isolated cache payload must be created");

        // Exercise the complete production entry points while Dropbox owns the
        // File Provider cache. Real mode is intentional: the process gate must
        // block before source selection, revalidation, or deletion can mutate it.
        let rule = preview(&|| false, &|_| {}, &|_, _, _| {});
        assert_eq!(rule.status, ScanItemStatus::RequiresClose);
        assert!(rule.source_count > 0);
        let operation = OperationGuard::start(CoordinatedOperationKind::Cleanup).unwrap();
        let action = execute(None, false, &operation);
        operation.complete();

        assert_eq!(action.status, CleanupActionStatus::Blocked);
        assert_eq!(
            action.reason_code,
            Some(CleanupActionReason::RunningProcesses)
        );
        assert_eq!(action.released_bytes, 0);
        assert_eq!(action.affected_item_count, 0);
        assert!(fixture_path.exists());
    }

    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "permanently clears the real Dropbox File Provider cache"]
    fn real_macos_file_provider_cleanup_preserves_sync_root() {
        struct CacheMarkerCleanup(PathBuf);

        impl Drop for CacheMarkerCleanup {
            fn drop(&mut self) {
                let _ = fs::remove_file(&self.0);
            }
        }

        fn direct_entry_metadata_digest(root: &Path) -> blake3::Hash {
            use std::os::unix::ffi::OsStrExt;

            let mut entries = fs::read_dir(root)
                .expect("the real Dropbox sync root must be readable")
                .map(|entry| entry.expect("the sync-root entry must be readable"))
                .collect::<Vec<_>>();
            entries.sort_by_key(|entry| entry.file_name());
            let mut hasher = blake3::Hasher::new();
            for entry in entries {
                let metadata = fs::symlink_metadata(entry.path())
                    .expect("the sync-root entry metadata must be readable");
                hasher.update(entry.file_name().as_bytes());
                hasher.update(&metadata.len().to_le_bytes());
                hasher.update(&[
                    u8::from(metadata.is_file()),
                    u8::from(metadata.is_dir()),
                    u8::from(is_link_like(&metadata)),
                ]);
            }
            hasher.finalize()
        }

        assert_eq!(
            std::env::var("MANGODISK_REAL_DROPBOX_MACOS_CACHE").as_deref(),
            Ok("1"),
            "set MANGODISK_REAL_DROPBOX_MACOS_CACHE=1 to authorize the real cache diagnostic"
        );
        let _operation_lock = crate::shared::operation::test_operation_lock();
        assert!(
            running_dropbox_processes()
                .expect("the macOS process inventory must be available")
                .is_empty(),
            "Dropbox and every File Provider owner process must be stopped"
        );

        let config_paths = dropbox_config_paths()
            .expect("the macOS home directory must be available for a real diagnostic");
        let account_root = read_account_roots(&config_paths, &|| false)
            .expect("the real Dropbox configuration must expose an account root")
            .into_iter()
            .next()
            .expect("the real diagnostic requires one Dropbox account root");
        let sync_root_before = direct_entry_metadata_digest(&account_root);
        let roots = discover_file_provider_cache_roots(&|| false)
            .expect("the documented Dropbox File Provider root must be discoverable");
        let cache_root = roots
            .first()
            .expect("the real diagnostic requires one File Provider cache root")
            .path
            .clone();
        let cache_candidate = fs::read_dir(&cache_root)
            .expect("the File Provider cache root must be readable")
            .map(|entry| entry.expect("the File Provider cache entry must be readable"))
            .find_map(|entry| {
                let path = entry.path();
                fs::symlink_metadata(&path)
                    .ok()
                    .filter(|metadata| metadata.is_dir() && !is_link_like(metadata))
                    .map(|_| path)
            })
            .expect("the signed-in client must expose one cache directory");

        // The new profile can contain only empty Dropbox-owned bookkeeping
        // directories. A unique payload makes that real cache entry measurable;
        // production cleanup still selects and deletes the complete documented
        // direct child, allowing restart to prove that Dropbox recreates it.
        let marker_path = cache_candidate.join(format!(
            "mangodisk-file-provider-validation-{}-{}.bin",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("the system clock must be after the Unix epoch")
                .as_nanos()
        ));
        let _marker_cleanup = CacheMarkerCleanup(marker_path.clone());
        fs::write(&marker_path, b"payload").expect("the isolated cache payload must be created");

        let rule = preview(&|| false, &|_| {}, &|_, _, _| {});
        assert_eq!(rule.status, ScanItemStatus::Found);
        let selected_paths = rule
            .sources
            .iter()
            .map(|source| PathBuf::from(&source.path))
            .filter(|path| path == &cache_candidate)
            .collect::<Vec<_>>();
        assert_eq!(selected_paths.len(), 1);
        assert!(selected_paths
            .iter()
            .all(|path| candidate_has_safe_boundary(&DropboxCacheCandidate {
                layout: DropboxCacheLayout::FileProvider,
                cache_root: path
                    .parent()
                    .expect("a selected source must have a parent")
                    .into(),
                path: path.clone(),
                bytes: 0,
                file_count: 0,
                modified_at_ms: None,
            })));

        let selected_ids = HashSet::from([CLEANER_ID.to_string()]);
        let selection = CleanupSourceSelection {
            rule_id: CLEANER_ID.to_string(),
            mode: CleanupSourceSelectionMode::Include,
            paths: selected_paths
                .iter()
                .map(|path| display_path(path))
                .collect(),
        };
        let policy = SourceSelectionPolicy::from_request(&selected_ids, &[selection]).unwrap();
        let operation = OperationGuard::start(CoordinatedOperationKind::Cleanup).unwrap();
        let action = execute(policy.scope(CLEANER_ID), false, &operation);
        operation.complete();

        eprintln!(
            "real_macos_dropbox_cache_result released_bytes={} affected_items={}",
            action.released_bytes, action.affected_item_count
        );
        assert_eq!(action.status, CleanupActionStatus::Completed);
        assert!(action.released_bytes > 0);
        assert!(action.affected_item_count > 0);
        assert!(selected_paths.iter().all(|path| !path.exists()));
        assert!(cache_root.exists());
        assert!(account_root.exists());
        assert!(sync_root_before == direct_entry_metadata_digest(&account_root));
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "requires an installed and running Dropbox desktop client"]
    fn real_windows_process_snapshot_detects_dropbox() {
        let _operation_lock = crate::shared::operation::test_operation_lock();
        let running = running_dropbox_processes()
            .expect("the Windows process inventory must be available for a real diagnostic");

        assert!(
            running
                .iter()
                .any(|name| name.eq_ignore_ascii_case("Dropbox.exe")),
            "the real diagnostic requires Dropbox.exe to be running"
        );

        // Exercise the production preview and execute entry points against the
        // real Dropbox configuration. Execute uses real mode intentionally: the
        // running-process gate must block before any deletion can begin.
        let rule = preview(&|| false, &|_| {}, &|_, _, _| {});
        assert_eq!(rule.status, ScanItemStatus::RequiresClose);
        assert!(rule.source_count > 0);
        let operation = OperationGuard::start(CoordinatedOperationKind::Cleanup).unwrap();
        let action = execute(None, false, &operation);
        operation.complete();
        assert_eq!(action.status, CleanupActionStatus::Blocked);
        assert_eq!(
            action.reason_code,
            Some(CleanupActionReason::RunningProcesses)
        );
        assert_eq!(action.released_bytes, 0);
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "creates and deletes one isolated child under a real Dropbox cache"]
    fn real_windows_source_selection_deletes_only_owned_cache_fixture() {
        struct CacheFixtureCleanup {
            fixture_path: PathBuf,
            cache_root: PathBuf,
            cache_root_was_present: bool,
        }

        impl Drop for CacheFixtureCleanup {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.fixture_path);
                if !self.cache_root_was_present {
                    let _ = fs::remove_dir(&self.cache_root);
                }
            }
        }

        let _operation_lock = crate::shared::operation::test_operation_lock();
        assert!(
            running_dropbox_processes()
                .expect("the Windows process inventory must be available")
                .is_empty(),
            "Dropbox and DropboxUpdate must be stopped before the real cleanup diagnostic"
        );
        let config_paths = dropbox_config_paths()
            .expect("the Windows AppData directories must be available for a real diagnostic");
        let account_roots = read_account_roots(&config_paths, &|| false)
            .expect("the real Dropbox configuration must expose an account root");
        let account_root = account_roots
            .first()
            .expect("the real diagnostic requires at least one Dropbox account root");
        let cache_root = account_root.join(CACHE_DIRECTORY_NAME);
        let cache_root_was_present = cache_root.exists();
        fs::create_dir_all(&cache_root).unwrap();
        let existing_candidates =
            discover_candidates(&config_paths, &|| false, &|_| {}, &|_, _, _| {})
                .expect("the existing real Dropbox cache must have safe boundaries");
        let fixture_path = cache_root.join(format!(
            "mangodisk-rule-validation-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("the system clock must be after the Unix epoch")
                .as_nanos()
        ));
        let _cleanup = CacheFixtureCleanup {
            fixture_path: fixture_path.clone(),
            cache_root: cache_root.clone(),
            cache_root_was_present,
        };
        fs::create_dir(&fixture_path).unwrap();
        fs::write(fixture_path.join("payload.bin"), b"payload").unwrap();

        let rule = preview(&|| false, &|_| {}, &|_, _, _| {});
        assert_eq!(rule.status, ScanItemStatus::Found);
        assert!(rule
            .sources
            .iter()
            .any(|source| Path::new(&source.path) == fixture_path));
        let selected_ids = HashSet::from([CLEANER_ID.to_string()]);
        let selection = CleanupSourceSelection {
            rule_id: CLEANER_ID.to_string(),
            mode: CleanupSourceSelectionMode::Include,
            paths: vec![fixture_path.to_string_lossy().into_owned()],
        };
        let policy = SourceSelectionPolicy::from_request(&selected_ids, &[selection]).unwrap();
        let operation = OperationGuard::start(CoordinatedOperationKind::Cleanup).unwrap();
        let action = execute(policy.scope(CLEANER_ID), false, &operation);
        operation.complete();

        assert_eq!(action.status, CleanupActionStatus::Completed);
        assert_eq!(action.released_bytes, 7);
        assert_eq!(action.affected_item_count, 1);
        assert!(!fixture_path.exists());
        assert!(cache_root.exists());
        assert!(account_root.exists());
        for candidate in existing_candidates {
            assert!(candidate.path.exists());
            let current = measure_path_filtered(&candidate.path, None, &|_, _| true);
            assert_eq!(current.bytes, candidate.bytes);
            assert_eq!(current.file_count, candidate.file_count);
            assert_eq!(current.skipped_count, 0);
        }
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "writes an isolated Dropbox info.json into the test user's AppData"]
    fn real_windows_appdata_preview_and_execution_delete_only_cache_children() {
        struct FixtureCleanup {
            config_path: PathBuf,
            account_root: PathBuf,
            config_parent_was_present: bool,
        }

        impl Drop for FixtureCleanup {
            fn drop(&mut self) {
                let _ = fs::remove_file(&self.config_path);
                if !self.config_parent_was_present {
                    if let Some(parent) = self.config_path.parent() {
                        let _ = fs::remove_dir(parent);
                    }
                }
                let _ = fs::remove_dir_all(&self.account_root);
            }
        }

        let _operation_lock = crate::shared::operation::test_operation_lock();
        let config_paths = dropbox_config_paths()
            .expect("the Windows AppData directories must be available for a real diagnostic");
        assert!(
            config_paths.iter().all(|path| !path.exists()),
            "the real diagnostic refuses to overwrite an existing Dropbox account configuration"
        );
        assert!(
            running_dropbox_processes()
                .expect("the Windows process inventory must be available")
                .is_empty(),
            "Dropbox and DropboxUpdate must be stopped before the real cleanup diagnostic"
        );

        let config_path = config_paths
            .into_iter()
            .next()
            .expect("Windows must expose at least one AppData directory");
        let config_parent_was_present = config_path.parent().is_some_and(Path::exists);
        let account_root = test_directory("real-windows-appdata");
        let cache_root = account_root.join(CACHE_DIRECTORY_NAME);
        let cache_child = cache_root.join("staged-download");
        let account_document = account_root.join("preserved-document.txt");
        fs::create_dir_all(&cache_child).unwrap();
        fs::write(cache_child.join("payload.bin"), b"payload").unwrap();
        fs::write(&account_document, b"keep").unwrap();
        let _cleanup = FixtureCleanup {
            config_path: config_path.clone(),
            account_root: account_root.clone(),
            config_parent_was_present,
        };
        write_info_json(&config_path, &account_root, None);

        let rule = preview(&|| false, &|_| {}, &|_, _, _| {});
        assert_eq!(rule.status, ScanItemStatus::Found);
        assert_eq!(rule.bytes, 7);
        assert_eq!(rule.file_count, 1);
        assert_eq!(rule.source_count, 1);

        let operation = OperationGuard::start(CoordinatedOperationKind::Cleanup).unwrap();
        let action = execute(None, false, &operation);
        operation.complete();

        assert_eq!(action.status, CleanupActionStatus::Completed);
        assert_eq!(action.released_bytes, 7);
        assert_eq!(action.affected_item_count, 1);
        assert!(!cache_child.exists());
        assert!(cache_root.exists());
        assert_eq!(fs::read(account_document).unwrap(), b"keep");
    }
}
