use std::{
    collections::{HashMap, VecDeque},
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};

use mangodisk_platform::{
    current_platform, FileSpaceUsage, FilesystemChangeMonitor, FilesystemChangeStatus,
    FilesystemChangeToken, Platform, ScanPurpose,
};

use super::metrics::ByteMetric;

use crate::shared::operation::OPERATION_CANCELLED_ERROR;

const ANALYSIS_CACHE_ROOT_LIMIT: usize = 2;
const ANALYSIS_CACHE_UNAVAILABLE_ERROR: &str = "the analysis cache is unavailable";

static ANALYSIS_CACHE: OnceLock<Mutex<AnalysisCache>> = OnceLock::new();

#[derive(Clone, Copy, Default)]
pub(crate) struct DirectoryAggregate {
    /// Proven scan-time file retention bound, independent of later global eviction.
    pub(crate) retained_file_limit: usize,
    pub(crate) byte_metric: ByteMetric,
    /// Bytes in this snapshot's declared metric; logical scans never claim allocation.
    pub(crate) bytes: u64,
    /// Content length retained for immutable snapshot and delete preflight checks.
    pub(crate) logical_bytes: u64,
    pub(crate) file_count: u64,
    /// Direct files with positive charged allocation, after hard-link reconciliation.
    pub(crate) direct_file_count: u64,
    pub(crate) skipped_count: u64,
    pub(crate) omissions: mangodisk_platform::ScanOmissionCounts,
    pub(crate) scanned_at_ms: u64,
    pub(crate) fingerprint: Option<[u8; 32]>,
}

#[derive(Clone, Copy)]
pub(crate) struct IndexedFile {
    /// Retained explicitly: empty and sparse files can also have zero allocation.
    pub(crate) shared_identity: Option<mangodisk_platform::PhysicalFileIdentity>,
    pub(crate) bytes: u64,
    pub(crate) logical_bytes: u64,
    pub(crate) modified_at_ms: Option<u64>,
}

#[derive(Default)]
struct AnalysisCache {
    navigation: super::navigation::NavigationIndex,
    directories: HashMap<PathBuf, DirectoryAggregate>,
    files: HashMap<PathBuf, IndexedFile>,
    scan_roots: HashMap<PathBuf, ScanPurpose>,
    /// Identifies the shared storage-scan exclusion configuration used to build each root.
    scan_configurations: HashMap<PathBuf, [u8; 32]>,
    /// Retains active exclusions so cached descendant pages omit excluded placeholders.
    scan_exclusions: HashMap<PathBuf, (Vec<PathBuf>, mangodisk_platform::NameExclusions)>,
    /// Monotonic operation identifiers prevent an older concurrent scan from replacing a newer
    /// snapshot of the same root after it finishes later.
    publish_generations: HashMap<PathBuf, u64>,
    /// Destructive cache updates increment this revision. A scan captures it before traversal and
    /// skips cache publication if a deletion happened while its private snapshot was being built.
    mutation_revision: u64,
    /// Orders cached roots from least recently used to most recently used.
    ///
    /// Directory and file maps are intentionally shared across roots to keep lookup inexpensive.
    /// The separate order only owns eviction policy and must be updated whenever a completed root
    /// is stored or reused.
    root_recency: VecDeque<PathBuf>,
    change_tokens: HashMap<PathBuf, Option<FilesystemChangeToken>>,
    change_monitors: HashMap<PathBuf, CachedChangeMonitor>,
}

struct CachedChangeMonitor {
    token: FilesystemChangeToken,
    monitor: FilesystemChangeMonitor,
}

enum ChangeValidation {
    Valid(Option<FilesystemChangeMonitor>),
    Stale,
}

pub(crate) enum CacheReuseDecision {
    Reusable,
    Miss,
}

/// Groups publication-only metadata so callers cannot confuse snapshot data with cache ordering
/// controls. The scan response is built from its private snapshot before this policy is applied.
pub(crate) struct SnapshotPublication {
    purpose: ScanPurpose,
    refresh: bool,
    change_token: Option<FilesystemChangeToken>,
    generation: u64,
    expected_mutation_revision: u64,
    configuration_fingerprint: [u8; 32],
    excluded_roots: Vec<PathBuf>,
    excluded_names: Option<mangodisk_platform::NameExclusions>,
}

impl SnapshotPublication {
    pub(crate) const fn new(
        purpose: ScanPurpose,
        refresh: bool,
        change_token: Option<FilesystemChangeToken>,
        generation: u64,
        expected_mutation_revision: u64,
    ) -> Self {
        Self {
            purpose,
            refresh,
            change_token,
            generation,
            expected_mutation_revision,
            configuration_fingerprint: [0; 32],
            excluded_roots: Vec::new(),
            excluded_names: None,
        }
    }

    pub(crate) const fn with_configuration_fingerprint(
        mut self,
        configuration_fingerprint: [u8; 32],
    ) -> Self {
        self.configuration_fingerprint = configuration_fingerprint;
        self
    }

    pub(crate) fn with_excluded_names(
        mut self,
        names: &mangodisk_platform::NameExclusions,
    ) -> Self {
        self.excluded_names = Some(names.clone());
        self
    }

    pub(crate) fn with_excluded_roots(mut self, excluded_roots: &[PathBuf]) -> Self {
        self.excluded_roots = excluded_roots.to_vec();
        self
    }
}

impl ChangeValidation {
    fn into_valid_monitor(self) -> Option<Option<FilesystemChangeMonitor>> {
        match self {
            Self::Valid(monitor) => Some(monitor),
            Self::Stale => None,
        }
    }
}

/// Reuse is intentionally limited to the current process. A completed scan has one authoritative
/// result in memory, avoiding duplicate storage and write backpressure on the traversal path.
pub(crate) fn reuse_snapshot_decision(
    root: &Path,
    purpose: ScanPurpose,
    byte_metric: ByteMetric,
    configuration_fingerprint: [u8; 32],
    is_cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<CacheReuseDecision, String> {
    if is_cancelled() {
        return Err(OPERATION_CANCELLED_ERROR.to_string());
    }

    let candidate = {
        let cache = cache()
            .lock()
            .map_err(|_| ANALYSIS_CACHE_UNAVAILABLE_ERROR.to_string())?;
        cache
            .directories
            .contains_key(root)
            .then(|| {
                cache
                    .scan_roots
                    .iter()
                    .filter(|(scan_root, _)| root.starts_with(scan_root))
                    .max_by_key(|(scan_root, _)| scan_root.components().count())
                    .map(|(scan_root, purpose)| {
                        let token = cache.change_tokens.get(scan_root).copied().flatten();
                        let monitor = token.and_then(|token| {
                            cache
                                .change_monitors
                                .get(scan_root)
                                .filter(|cached| cached.token == token)
                                .map(|cached| cached.monitor.clone())
                        });
                        (
                            scan_root.clone(),
                            *purpose,
                            cache
                                .scan_configurations
                                .get(scan_root)
                                .copied()
                                .unwrap_or([0; 32]),
                            token,
                            monitor,
                            scan_root != root,
                        )
                    })
            })
            .flatten()
    };

    let Some((scan_root, cached_purpose, cached_configuration, token, monitor, is_descendant_page)) =
        candidate
    else {
        return Ok(CacheReuseDecision::Miss);
    };
    if cached_purpose != purpose {
        evict_memory_root(&scan_root)?;
        return Ok(CacheReuseDecision::Miss);
    }
    let cached_mode = cache()
        .lock()
        .map_err(|_| ANALYSIS_CACHE_UNAVAILABLE_ERROR.to_string())?
        .directories
        .get(root)
        .map(|aggregate| aggregate.byte_metric);
    if cached_mode != Some(byte_metric) {
        log::info!(
            "analysis_cache_invalidated root={} reason=scan_mode_changed requested_mode={}",
            crate::filesystem::metadata::diagnostic_path(&scan_root),
            byte_metric.as_str()
        );
        evict_memory_root(&scan_root)?;
        return Ok(CacheReuseDecision::Miss);
    }
    if cached_configuration != configuration_fingerprint {
        log::info!(
            "analysis_cache_invalidated root={} reason=scan_configuration_changed",
            crate::filesystem::metadata::diagnostic_path(&scan_root)
        );
        evict_memory_root(&scan_root)?;
        return Ok(CacheReuseDecision::Miss);
    }

    // Descendant navigation is a view over the immutable active analysis. Revalidating the
    // pre-scan change token here makes a busy home directory immediately stale even though the
    // user only changed pages. Explicit refresh and destructive preflight remain freshness
    // boundaries.
    if is_descendant_page {
        mark_root_recent(&scan_root)?;
        return Ok(CacheReuseDecision::Reusable);
    }

    if let Some(new_monitor) =
        validate_change_token(&scan_root, token, monitor, is_cancelled)?.into_valid_monitor()
    {
        if let (Some(token), Some(new_monitor)) = (token, new_monitor) {
            install_change_monitor(&scan_root, token, new_monitor)?;
        }
        mark_root_recent(&scan_root)?;
        return Ok(CacheReuseDecision::Reusable);
    }

    if is_cancelled() {
        return Err(OPERATION_CANCELLED_ERROR.to_string());
    }
    evict_memory_root(&scan_root)?;
    Ok(CacheReuseDecision::Miss)
}

pub(crate) fn store_memory_only(
    root: &Path,
    root_aggregate: DirectoryAggregate,
    scanned_directories: HashMap<PathBuf, DirectoryAggregate>,
    scanned_files: HashMap<PathBuf, IndexedFile>,
    publication: SnapshotPublication,
) -> Result<bool, String> {
    let removed_monitors = {
        let mut cache = cache()
            .lock()
            .map_err(|_| ANALYSIS_CACHE_UNAVAILABLE_ERROR.to_string())?;
        if cache.mutation_revision != publication.expected_mutation_revision {
            log::info!(
                "analysis_cache_publish_skipped generation={} reason=concurrent_mutation expected_revision={} actual_revision={}",
                publication.generation,
                publication.expected_mutation_revision,
                cache.mutation_revision
            );
            return Ok(false);
        }
        // Refreshing an ancestor removes descendant snapshots, and refreshing a descendant
        // rewrites entries that also belong to an ancestor snapshot. Compare every overlapping
        // root so a slower older scan cannot erase or partially mix a newer concurrent result.
        if cache
            .publish_generations
            .iter()
            .any(|(cached_root, generation)| {
                (cached_root.starts_with(root) || root.starts_with(cached_root))
                    && *generation > publication.generation
            })
        {
            log::info!(
                "analysis_cache_publish_skipped generation={} reason=newer_snapshot",
                publication.generation
            );
            return Ok(false);
        }
        let mut removed_monitors = Vec::new();
        // Flattened maps must never patch an ancestor with a different byte metric.
        let incompatible_roots: Vec<_> = cache
            .scan_roots
            .keys()
            .filter(|cached_root| {
                root.starts_with(cached_root.as_path()) || cached_root.starts_with(root)
            })
            .filter(|cached_root| {
                cache
                    .directories
                    .get(*cached_root)
                    .is_some_and(|aggregate| aggregate.byte_metric != root_aggregate.byte_metric)
            })
            .cloned()
            .collect();
        for cached_root in incompatible_roots {
            removed_monitors.extend(evict_cached_root(&mut cache, &cached_root));
        }
        // Allocation ownership is scoped to a full scan. Refreshing only a child cannot patch
        // an ancestor with shared allocation: a surviving link in a sibling may own the bytes.
        let shared_roots: Vec<_> = cache
            .scan_roots
            .keys()
            .filter(|cached_root| root.starts_with(cached_root.as_path()))
            .filter(|cached_root| has_shared_allocation(&cache, cached_root))
            .cloned()
            .collect();
        for cached_root in shared_roots {
            removed_monitors.extend(evict_cached_root(&mut cache, &cached_root));
        }
        // The directory and file maps are flattened across cached roots. Any overlapping
        // publication must therefore replace its subtree atomically, even when the caller did not
        // request an explicit refresh. This occurs when two compatible scan kinds finish out of
        // order on an ancestor and descendant. Leaving the old nested root metadata behind would
        // let its older change token evict records now owned by the newer ancestor snapshot.
        let replaces_overlapping_snapshot = publication.refresh
            || cache
                .scan_roots
                .keys()
                .any(|cached_root| cached_root.starts_with(root) || root.starts_with(cached_root));
        if replaces_overlapping_snapshot {
            if let Some(previous) = cache.directories.get(root).copied() {
                for (path, aggregate) in &mut cache.directories {
                    if path != root && root.starts_with(path) {
                        aggregate.bytes = aggregate
                            .bytes
                            .saturating_sub(previous.bytes)
                            .saturating_add(root_aggregate.bytes);
                        aggregate.logical_bytes = aggregate
                            .logical_bytes
                            .saturating_sub(previous.logical_bytes)
                            .saturating_add(root_aggregate.logical_bytes);
                        aggregate.file_count = aggregate
                            .file_count
                            .saturating_sub(previous.file_count)
                            .saturating_add(root_aggregate.file_count);
                        aggregate.skipped_count = aggregate
                            .skipped_count
                            .saturating_sub(previous.skipped_count)
                            .saturating_add(root_aggregate.skipped_count);
                        aggregate
                            .omissions
                            .replace_subtree(previous.omissions, root_aggregate.omissions);
                        aggregate.scanned_at_ms = root_aggregate.scanned_at_ms;
                    }
                }
            }
            cache.directories.retain(|path, _| !path.starts_with(root));
            cache.files.retain(|path, _| !path.starts_with(root));
            cache.scan_roots.retain(|path, _| !path.starts_with(root));
            cache
                .scan_configurations
                .retain(|path, _| !path.starts_with(root));
            cache
                .scan_exclusions
                .retain(|path, _| !path.starts_with(root));
            cache
                .publish_generations
                .retain(|path, _| !path.starts_with(root));
            cache.root_recency.retain(|path| !path.starts_with(root));
            cache
                .change_tokens
                .retain(|path, _| !path.starts_with(root));
            removed_monitors.extend(take_monitors(&mut cache, |path| path.starts_with(root)));
        } else if cache
            .change_monitors
            .get(root)
            .is_some_and(|cached| Some(cached.token) != publication.change_token)
        {
            if let Some(cached) = cache.change_monitors.remove(root) {
                removed_monitors.push(cached.monitor);
            }
        }
        // Overlapping roots are removed before applying the capacity limit so replacing a cached
        // descendant with its ancestor reuses that slot instead of evicting an unrelated root.
        while !cache.scan_roots.contains_key(root)
            && cache.scan_roots.len() >= ANALYSIS_CACHE_ROOT_LIMIT
        {
            let least_recent_root = cache
                .root_recency
                .front()
                .cloned()
                .ok_or_else(|| "the analysis cache root recency is inconsistent".to_string())?;
            let roots_before = cache.scan_roots.len();
            removed_monitors.extend(evict_cached_root(&mut cache, &least_recent_root));
            log::info!(
                "analysis_cache_root_evicted roots_before={} roots_after={} root_limit={}",
                roots_before,
                cache.scan_roots.len(),
                ANALYSIS_CACHE_ROOT_LIMIT
            );
        }
        cache.directories.extend(scanned_directories);
        cache.files.extend(scanned_files);
        cache.navigation =
            super::navigation::NavigationIndex::build(&cache.directories, &cache.files);
        cache
            .scan_roots
            .insert(root.to_path_buf(), publication.purpose);
        cache
            .scan_configurations
            .insert(root.to_path_buf(), publication.configuration_fingerprint);
        cache.scan_exclusions.insert(
            root.to_path_buf(),
            (
                publication.excluded_roots,
                publication.excluded_names.unwrap_or_default(),
            ),
        );
        cache
            .publish_generations
            .insert(root.to_path_buf(), publication.generation);
        touch_root(&mut cache, root);
        cache
            .change_tokens
            .insert(root.to_path_buf(), publication.change_token);
        removed_monitors
    };
    drop(removed_monitors);
    Ok(true)
}

pub(crate) fn mutation_revision() -> Result<u64, String> {
    cache()
        .lock()
        .map(|cache| cache.mutation_revision)
        .map_err(|_| ANALYSIS_CACHE_UNAVAILABLE_ERROR.to_string())
}

pub(crate) fn remove_entry(
    target: &Path,
    removed_usage: FileSpaceUsage,
    file_count: u64,
    is_directory: bool,
) {
    remove_entry_in_mode(
        target,
        removed_usage,
        file_count,
        is_directory,
        ByteMetric::Allocated,
    );
}

pub(crate) fn remove_entry_in_mode(
    target: &Path,
    removed_usage: FileSpaceUsage,
    file_count: u64,
    is_directory: bool,
    byte_metric: ByteMetric,
) {
    remove_entry_with_allocation_credits(
        target,
        removed_usage,
        file_count,
        is_directory,
        byte_metric,
        None,
    );
}

pub(crate) fn remove_entry_with_allocation_credits(
    target: &Path,
    removed_usage: FileSpaceUsage,
    file_count: u64,
    is_directory: bool,
    byte_metric: ByteMetric,
    credits: Option<&[AllocationCredit<'_>]>,
) {
    let removed_monitors = {
        let Ok(mut cache) = cache().lock() else {
            log::warn!("analysis_cache_update_failed reason=poisoned_lock");
            return;
        };
        cache.mutation_revision = cache.mutation_revision.saturating_add(1);
        // Deletion may originate from an older result or another storage module.
        // Its byte metric cannot repair an incompatible index; rebuild that root instead.
        let mut incompatible_monitors = Vec::new();
        let incompatible_roots: Vec<_> = cache
            .scan_roots
            .keys()
            .filter(|root| target.starts_with(root.as_path()))
            .filter(|root| {
                cache
                    .directories
                    .get(*root)
                    .is_some_and(|aggregate| aggregate.byte_metric != byte_metric)
            })
            .cloned()
            .collect();
        for root in incompatible_roots {
            incompatible_monitors.extend(evict_cached_root(&mut cache, &root));
        }
        let shared_roots: Vec<_> = cache
            .scan_roots
            .keys()
            .filter(|root| target.starts_with(root.as_path()))
            .filter(|_| credits.is_none() && has_shared_allocation(&cache, target))
            .cloned()
            .collect();
        let mut shared_monitors = incompatible_monitors;
        for root in shared_roots {
            shared_monitors.extend(evict_cached_root(&mut cache, &root));
        }
        let removed_aggregate = is_directory
            .then(|| cache.directories.get(target).copied())
            .flatten();
        let removed_usage = if is_directory {
            cache
                .directories
                .get(target)
                .map(|aggregate| FileSpaceUsage {
                    logical_bytes: aggregate.logical_bytes,
                    allocated_bytes: aggregate.bytes,
                })
        } else {
            cache.files.get(target).map(|file| FileSpaceUsage {
                logical_bytes: file.logical_bytes,
                allocated_bytes: file.bytes,
            })
        }
        .unwrap_or(removed_usage);
        let removed_monitors = if is_directory {
            cache.files.retain(|path, _| !path.starts_with(target));
            cache
                .directories
                .retain(|path, _| !path.starts_with(target));
            cache.scan_roots.retain(|path, _| !path.starts_with(target));
            cache
                .scan_configurations
                .retain(|path, _| !path.starts_with(target));
            cache
                .scan_exclusions
                .retain(|path, _| !path.starts_with(target));
            cache
                .publish_generations
                .retain(|path, _| !path.starts_with(target));
            cache.root_recency.retain(|path| !path.starts_with(target));
            cache
                .change_tokens
                .retain(|path, _| !path.starts_with(target));
            take_monitors(&mut cache, |path| path.starts_with(target))
        } else {
            cache.files.remove(target);
            Vec::new()
        };
        cache.navigation.remove(target, is_directory);
        for (directory, aggregate) in &mut cache.directories {
            if target.starts_with(directory) {
                aggregate.bytes = aggregate
                    .bytes
                    .saturating_sub(aggregate.byte_metric.displayed_bytes(removed_usage));
                aggregate.logical_bytes = aggregate
                    .logical_bytes
                    .saturating_sub(removed_usage.logical_bytes);
                aggregate.file_count = aggregate.file_count.saturating_sub(file_count);
                if !is_directory
                    && target.parent() == Some(directory.as_path())
                    && aggregate.byte_metric.displayed_bytes(removed_usage) > 0
                {
                    aggregate.direct_file_count = aggregate.direct_file_count.saturating_sub(1);
                }
                if let Some(removed) = removed_aggregate {
                    aggregate.skipped_count = aggregate
                        .skipped_count
                        .saturating_sub(removed.skipped_count);
                    aggregate
                        .omissions
                        .replace_subtree(removed.omissions, Default::default());
                }
                aggregate.fingerprint = None;
            }
        }
        for credit in credits.into_iter().flatten() {
            let path = Path::new(&credit.path);
            let Some(file) = cache.files.get_mut(path) else {
                continue;
            };
            // Independently refreshed roots may already charge this alias.
            if file.bytes > 0 || credit.bytes == 0 {
                continue;
            }
            file.bytes = credit.bytes;
            for (directory, aggregate) in &mut cache.directories {
                if path.starts_with(directory) {
                    aggregate.bytes = aggregate.bytes.saturating_add(credit.bytes);
                    if path.parent() == Some(directory.as_path()) {
                        aggregate.direct_file_count += 1;
                    }
                    aggregate.fingerprint = None;
                }
            }
        }
        shared_monitors.extend(removed_monitors);
        shared_monitors
    };
    drop(removed_monitors);
}

pub(crate) fn clear_all() -> Result<(), String> {
    let previous = {
        let mut cache = cache()
            .lock()
            .map_err(|_| ANALYSIS_CACHE_UNAVAILABLE_ERROR.to_string())?;
        let next_revision = cache.mutation_revision.saturating_add(1);
        let mut previous = std::mem::take(&mut *cache);
        cache.mutation_revision = next_revision;
        previous.mutation_revision = 0;
        previous
    };
    drop(previous);
    Ok(())
}

#[cfg(test)]
pub(crate) fn memory_entry_counts() -> Result<(usize, usize, usize), String> {
    let cache = cache()
        .lock()
        .map_err(|_| ANALYSIS_CACHE_UNAVAILABLE_ERROR.to_string())?;
    Ok((
        cache.scan_roots.len(),
        cache.directories.len(),
        cache.files.len(),
    ))
}

fn cache() -> &'static Mutex<AnalysisCache> {
    ANALYSIS_CACHE.get_or_init(|| Mutex::new(AnalysisCache::default()))
}

fn validate_change_token(
    root: &Path,
    token: Option<FilesystemChangeToken>,
    monitor: Option<FilesystemChangeMonitor>,
    is_cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<ChangeValidation, String> {
    if is_cancelled() {
        return Err(OPERATION_CANCELLED_ERROR.to_string());
    }
    let Some(token) = token else {
        return Ok(ChangeValidation::Stale);
    };
    if let Some(monitor) = monitor {
        return Ok(match monitor.status() {
            FilesystemChangeStatus::Clean => ChangeValidation::Valid(None),
            FilesystemChangeStatus::Pending
            | FilesystemChangeStatus::Changed
            | FilesystemChangeStatus::HistoryUnavailable => ChangeValidation::Stale,
        });
    }

    let started = current_platform().start_filesystem_change_monitor(root, &token, is_cancelled);
    if is_cancelled() {
        return Err(OPERATION_CANCELLED_ERROR.to_string());
    }
    match started {
        Ok(Some(monitor)) if monitor.status() == FilesystemChangeStatus::Clean => {
            let reusable_monitor = cfg!(target_os = "macos").then_some(monitor);
            Ok(ChangeValidation::Valid(reusable_monitor))
        }
        Ok(Some(_) | None) => Ok(ChangeValidation::Stale),
        Err(error) => {
            log::warn!(
                "analysis_cache_change_validation_failed diagnostic={}",
                error.diagnostic()
            );
            Ok(ChangeValidation::Stale)
        }
    }
}

fn install_change_monitor(
    root: &Path,
    token: FilesystemChangeToken,
    monitor: FilesystemChangeMonitor,
) -> Result<(), String> {
    let removed_monitors = {
        let mut cache = cache()
            .lock()
            .map_err(|_| ANALYSIS_CACHE_UNAVAILABLE_ERROR.to_string())?;
        let mut removed_monitors = Vec::new();
        if let Some(previous) = cache
            .change_monitors
            .insert(root.to_path_buf(), CachedChangeMonitor { token, monitor })
        {
            removed_monitors.push(previous.monitor);
        }
        removed_monitors
    };
    drop(removed_monitors);
    Ok(())
}

fn take_monitors(
    cache: &mut AnalysisCache,
    mut matches: impl FnMut(&Path) -> bool,
) -> Vec<FilesystemChangeMonitor> {
    let roots = cache
        .change_monitors
        .keys()
        .filter(|root| matches(root))
        .cloned()
        .collect::<Vec<_>>();
    roots
        .into_iter()
        .filter_map(|root| cache.change_monitors.remove(&root))
        .map(|cached| cached.monitor)
        .collect()
}

fn evict_memory_root(root: &Path) -> Result<(), String> {
    let removed_monitors = {
        let mut cache = cache()
            .lock()
            .map_err(|_| ANALYSIS_CACHE_UNAVAILABLE_ERROR.to_string())?;
        evict_cached_root(&mut cache, root)
    };
    drop(removed_monitors);
    Ok(())
}

fn mark_root_recent(root: &Path) -> Result<(), String> {
    let mut cache = cache()
        .lock()
        .map_err(|_| ANALYSIS_CACHE_UNAVAILABLE_ERROR.to_string())?;
    if cache.scan_roots.contains_key(root) {
        touch_root(&mut cache, root);
    }
    Ok(())
}

fn touch_root(cache: &mut AnalysisCache, root: &Path) {
    cache.root_recency.retain(|cached| cached != root);
    cache.root_recency.push_back(root.to_path_buf());
}

/// Removes one independently cached root and any nested refresh roots that share its records.
/// Nested roots cannot outlive their owner because the flattened directory and file maps contain
/// overlapping keys. Distinct roots remain available and preserve their relative recency.
fn evict_cached_root(cache: &mut AnalysisCache, root: &Path) -> Vec<FilesystemChangeMonitor> {
    cache.navigation.remove(root, true);
    cache.directories.retain(|path, _| !path.starts_with(root));
    cache.files.retain(|path, _| !path.starts_with(root));
    cache.scan_roots.retain(|path, _| !path.starts_with(root));
    cache
        .scan_configurations
        .retain(|path, _| !path.starts_with(root));
    cache
        .scan_exclusions
        .retain(|path, _| !path.starts_with(root));
    cache
        .publish_generations
        .retain(|path, _| !path.starts_with(root));
    cache.root_recency.retain(|path| !path.starts_with(root));
    cache
        .change_tokens
        .retain(|path, _| !path.starts_with(root));
    take_monitors(cache, |path| path.starts_with(root))
}

fn has_shared_allocation(cache: &AnalysisCache, root: &Path) -> bool {
    cache
        .files
        .iter()
        .any(|(path, file)| path.starts_with(root) && file.shared_identity.is_some())
}

/// Borrows one consistent index revision for domain-owned projections. The callback cannot
/// retain references after the cache lock is released or mutate publication state.
pub(in crate::storage) struct IndexReadView<'a> {
    pub(in crate::storage) navigation: &'a super::navigation::NavigationIndex,
    pub(in crate::storage) directories: &'a HashMap<PathBuf, DirectoryAggregate>,
    pub(in crate::storage) files: &'a HashMap<PathBuf, IndexedFile>,
    pub(in crate::storage) scan_roots: &'a HashMap<PathBuf, ScanPurpose>,
    pub(in crate::storage) scan_exclusions:
        &'a HashMap<PathBuf, (Vec<PathBuf>, mangodisk_platform::NameExclusions)>,
    pub(in crate::storage) publish_generations: &'a HashMap<PathBuf, u64>,
    pub(in crate::storage) mutation_revision: u64,
}
pub(in crate::storage) fn read_snapshot<T>(
    read: impl FnOnce(IndexReadView<'_>) -> T,
) -> Result<T, String> {
    let cache = cache()
        .lock()
        .map_err(|_| ANALYSIS_CACHE_UNAVAILABLE_ERROR.to_string())?;
    Ok(read(IndexReadView {
        navigation: &cache.navigation,
        directories: &cache.directories,
        files: &cache.files,
        scan_roots: &cache.scan_roots,
        scan_exclusions: &cache.scan_exclusions,
        publish_generations: &cache.publish_generations,
        mutation_revision: cache.mutation_revision,
    }))
}

#[cfg(test)]
#[path = "cache_tests.rs"]
mod tests;

pub(crate) struct AllocationCredit<'a> {
    pub(crate) path: &'a str,
    pub(crate) bytes: u64,
}
