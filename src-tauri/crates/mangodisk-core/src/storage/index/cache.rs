use std::{
    collections::{HashMap, HashSet, VecDeque},
    fs,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};

use mangodisk_platform::{
    current_platform, FileSpaceUsage, FilesystemChangeMonitor, FilesystemChangeStatus,
    FilesystemChangeToken, Platform, ScanPurpose, SkipReason,
};

use crate::storage::analysis::{AnalysisScanMode, ANALYSIS_VISIBLE_ENTRY_LIMIT};

use crate::{
    filesystem::metadata::{display_fingerprint, display_path, is_link_like, modified_ms},
    shared::operation::OPERATION_CANCELLED_ERROR,
    storage::{
        analysis::{
            AnalysisDirectoryNode, AnalysisRemainderParent, AnalysisRemainderRequest,
            AnalysisResult, DirectoryEntryInfo,
        },
        large_files::LargeFileEntry,
    },
};

const ANALYSIS_CACHE_ROOT_LIMIT: usize = 2;
pub(super) const HIERARCHY_MAX_DEPTH: usize = 6;
pub(super) const HIERARCHY_MAX_CHILDREN: usize = 64;
pub(super) const HIERARCHY_MAX_NODES: usize = 2048;
const ANALYSIS_CACHE_UNAVAILABLE_ERROR: &str = "the analysis cache is unavailable";

static ANALYSIS_CACHE: OnceLock<Mutex<AnalysisCache>> = OnceLock::new();

#[derive(Clone, Copy, Default)]
pub(crate) struct DirectoryAggregate {
    /// Proven scan-time file retention bound, independent of later global eviction.
    pub(crate) retained_file_limit: usize,
    pub(crate) scan_mode: AnalysisScanMode,
    /// Bytes in this snapshot's declared metric; logical scans never claim allocation.
    pub(crate) bytes: u64,
    /// Content length retained for immutable snapshot and delete preflight checks.
    pub(crate) logical_bytes: u64,
    pub(crate) file_count: u64,
    /// Direct files with positive charged allocation, after hard-link reconciliation.
    pub(crate) direct_file_count: u64,
    pub(crate) skipped_count: u64,
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
pub(crate) fn reuse_analysis_mode_decision(
    root: &Path,
    scan_mode: AnalysisScanMode,
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
    if cached_purpose != ScanPurpose::Analysis {
        evict_memory_root(&scan_root)?;
        return Ok(CacheReuseDecision::Miss);
    }
    let cached_mode = cache()
        .lock()
        .map_err(|_| ANALYSIS_CACHE_UNAVAILABLE_ERROR.to_string())?
        .directories
        .get(root)
        .map(|aggregate| aggregate.scan_mode);
    if cached_mode != Some(scan_mode) {
        log::info!(
            "analysis_cache_invalidated root={} reason=scan_mode_changed requested_mode={}",
            crate::filesystem::metadata::diagnostic_path(&scan_root),
            scan_mode.as_str()
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

pub(crate) fn large_file_entries_from_snapshot(
    root: &Path,
    files: &HashMap<PathBuf, IndexedFile>,
) -> Vec<LargeFileEntry> {
    let mut entries = files
        .iter()
        .filter(|(path, file)| {
            path.starts_with(root)
                && file.bytes >= crate::storage::large_files::LARGE_FILE_CANDIDATE_FLOOR_BYTES
        })
        .filter(|(path, _)| {
            current_platform()
                .should_skip(path, root, ScanPurpose::LargeFiles)
                .is_none()
        })
        .map(|(path, file)| large_file_entry(path, root, *file))
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| {
        right
            .bytes
            .cmp(&left.bytes)
            .then_with(|| left.path.cmp(&right.path))
    });
    entries
}

fn large_file_entry(path: &Path, root: &Path, file: IndexedFile) -> LargeFileEntry {
    LargeFileEntry {
        name: path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default(),
        path: display_path(path),
        parent_path: display_path(path.parent().unwrap_or(root)),
        bytes: file.bytes,
        logical_bytes: file.logical_bytes,
        modified_at_ms: file.modified_at_ms,
    }
}

pub(crate) fn analysis_result(root: &Path) -> Result<Option<AnalysisResult>, String> {
    let started = std::time::Instant::now();
    let (root_aggregate, excluded_roots, cached_entries, revision) = {
        let cache = cache()
            .lock()
            .map_err(|_| ANALYSIS_CACHE_UNAVAILABLE_ERROR.to_string())?;
        let aggregate = cache.directories.get(root).copied();
        let excluded_roots = cache
            .scan_roots
            .keys()
            .filter(|scan_root| current_platform().path_is_same_or_child(root, scan_root))
            .max_by_key(|scan_root| scan_root.components().count())
            .and_then(|scan_root| cache.scan_exclusions.get(scan_root))
            .cloned()
            .unwrap_or_default();
        let cached_entries = aggregate.and_then(|aggregate| {
            let (directories, files) =
                cache
                    .navigation
                    .direct_snapshot(root, &cache.directories, &cache.files);
            cached_navigation_entries(root, aggregate, &directories, &files)
        });
        let revision = (cache.mutation_revision, cache.publish_generations.clone());
        (aggregate, excluded_roots, cached_entries, revision)
    };
    let Some(root_aggregate) = root_aggregate else {
        return Ok(None);
    };

    let children = if cached_entries.is_some() {
        Vec::new()
    } else {
        read_analysis_children(root, &excluded_roots.0, &excluded_roots.1)?
    };
    let projected_rows = cached_entries
        .as_ref()
        .map_or(children.len(), |(entries, _)| entries.len());
    let metadata_ms = started.elapsed().as_millis();
    let cache = cache()
        .lock()
        .map_err(|_| ANALYSIS_CACHE_UNAVAILABLE_ERROR.to_string())?;
    if revision != (cache.mutation_revision, cache.publish_generations.clone()) {
        log::info!(
            "analysis_cache_projection_changed root={} outcome=cache_miss",
            crate::filesystem::metadata::diagnostic_path(root)
        );
        return Ok(None);
    }
    let scope = cache
        .scan_roots
        .keys()
        .filter(|scope| root.starts_with(scope))
        .max_by_key(|scope| scope.components().count())
        .map_or(root, |scope| scope.as_path());
    let shared = match cache.navigation.leaf_shared_identities(root, &cache.files) {
        Some(identities) => {
            shared_allocations_for_identities(root, scope, &cache.files, Some(identities))
        }
        None => shared_allocations(root, scope, &cache.files),
    };
    let shared_ms = started.elapsed().as_millis().saturating_sub(metadata_ms);
    let snapshot_rows = cached_entries.is_some();
    let mut result = if let Some((entries, count)) = cached_entries {
        build_analysis_result_from_entries(root, root_aggregate, entries, shared, count)
    } else {
        build_analysis_result(
            root,
            root_aggregate,
            children,
            |path| cache.directories.get(path).copied(),
            |path| cache.files.get(path).copied(),
            shared,
        )
    };
    retain_shared_directories(&mut result, &cache.directories, Some(&cache.navigation));
    let rows_ms = started
        .elapsed()
        .as_millis()
        .saturating_sub(metadata_ms + shared_ms);
    result.directory_hierarchy = cache
        .navigation
        .hierarchy(root, &cache.directories, &cache.files);
    log::info!("analysis_cache_projection_finished root={} projected_rows={} positive_children={} visible_rows={} snapshot_rows={} indexed_directories={} indexed_files={} metadata_ms={} shared_ms={} rows_ms={} hierarchy_ms={} elapsed_ms={}", crate::filesystem::metadata::diagnostic_path(root), projected_rows, result.total_entry_count, result.entries.len(), snapshot_rows, cache.directories.len(), cache.files.len(), metadata_ms, shared_ms, rows_ms, started.elapsed().as_millis().saturating_sub(metadata_ms + shared_ms + rows_ms), started.elapsed().as_millis());
    Ok(Some(result))
}

/// Reads current direct children, reusing scan sizes for indexed entries.
/// Read-only details tolerate filesystem and cache changes without extending
/// the authoritative snapshot used by destructive operations.
pub(crate) fn analysis_remainder_entries(
    root: &Path,
    parent: &AnalysisRemainderParent,
    request: &AnalysisRemainderRequest,
) -> Result<(u64, Vec<DirectoryEntryInfo>), String> {
    // Exclusions belong to the published session, not to the evictable index.
    let exclusions = crate::storage::exclusions::StorageScanExclusions::resolve_options(
        root,
        &parent.exclusions,
    )
    .map_err(|error| error.to_string())?;
    // Do not hold the shared scan-cache lock while enumerating filesystem metadata.
    let mut children = read_analysis_children(root, exclusions.roots(), exclusions.names())?;
    let mut entries = {
        let cache = cache()
            .lock()
            .map_err(|_| ANALYSIS_CACHE_UNAVAILABLE_ERROR.to_string())?;
        // Directory sizes require recursive scan data. Do not silently hide all
        // directories if that data was evicted; direct-file lists need no index.
        if !cache.directories.contains_key(root)
            && children.iter().any(|(_, _, metadata)| metadata.is_dir())
        {
            return Err("the analysis directory index is unavailable; scan again".to_string());
        }
        children.retain(|(_, path, metadata)| {
            if metadata.is_dir() {
                cache.directories.contains_key(path)
            } else {
                metadata.is_file()
            }
        });
        if cache
            .directories
            .get(root)
            .is_some_and(|aggregate| aggregate.scan_mode != parent.scan_mode)
        {
            return Err("the analysis index mode changed; scan again".to_string());
        }
        build_analysis_entries(
            children,
            parent.scan_mode,
            |path| cache.directories.get(path).copied(),
            |path| cache.files.get(path).copied(),
        )
    };
    let children_bytes = entries
        .iter()
        .try_fold(0_u64, |total, entry| total.checked_add(entry.bytes))
        .ok_or_else(|| "analysis child byte total overflowed".to_string())?;
    let visible_paths = request
        .visible_paths
        .iter()
        .map(|path| current_platform().path_identity_key(Path::new(path)))
        .collect::<HashSet<_>>();
    entries.retain(|entry| {
        entry.bytes > 0
            && !visible_paths
                .contains(&current_platform().path_identity_key(Path::new(&entry.path)))
    });
    let total_bytes = entries
        .iter()
        .try_fold(0_u64, |total, entry| total.checked_add(entry.bytes))
        .ok_or_else(|| "analysis remainder byte total overflowed".to_string())?;
    if children_bytes != parent.bytes || total_bytes != request.expected_bytes {
        log::debug!(
            "analysis_remainder_totals_changed path={} snapshot_parent_bytes={} listed_children_bytes={} snapshot_remainder_bytes={} listed_remainder_bytes={} outcome=displayed",
            crate::filesystem::metadata::diagnostic_path(root),
            parent.bytes,
            children_bytes,
            request.expected_bytes,
            total_bytes
        );
    }
    entries.sort_by(|left, right| {
        right
            .bytes
            .cmp(&left.bytes)
            .then_with(|| left.path.cmp(&right.path))
    });
    Ok((total_bytes, entries))
}

pub(crate) fn analysis_result_from_snapshot(
    root: &Path,
    root_aggregate: DirectoryAggregate,
    directories: &HashMap<PathBuf, DirectoryAggregate>,
    files: &HashMap<PathBuf, IndexedFile>,
    excluded_roots: &[PathBuf],
    excluded_names: &mangodisk_platform::NameExclusions,
    workers: usize,
) -> Result<AnalysisResult, String> {
    if let Some((entries, count)) =
        cached_navigation_entries(root, root_aggregate, directories, files)
    {
        let mut result = build_analysis_result_from_entries(
            root,
            root_aggregate,
            entries,
            shared_allocations(root, root, files),
            count,
        );
        retain_shared_directories(&mut result, directories, None);
        result.directory_hierarchy = build_directory_hierarchy(root, directories, files);
        return Ok(result);
    }
    let children = read_analysis_children_with_metadata(
        root,
        excluded_roots,
        excluded_names,
        AnalysisChildMetadata::ScanSnapshot,
    )?;
    #[cfg(any(windows, test))]
    let measured = measure_snapshot_files(&children, files, workers, root_aggregate.scan_mode);
    #[cfg(not(any(windows, test)))]
    let _ = workers;
    let mut result = build_analysis_result(
        root,
        root_aggregate,
        children,
        |path| directories.get(path).copied(),
        |path| {
            #[cfg(any(windows, test))]
            if let Some(file) = measured.get(path) {
                return Some(*file);
            }
            files.get(path).copied()
        },
        shared_allocations(root, root, files),
    );
    retain_shared_directories(&mut result, directories, None);
    result.directory_hierarchy = build_directory_hierarchy(root, directories, files);
    Ok(result)
}

#[cfg(any(windows, test))]
fn measure_snapshot_files(
    children: &[(fs::DirEntry, PathBuf, fs::Metadata)],
    indexed: &HashMap<PathBuf, IndexedFile>,
    workers: usize,
    scan_mode: AnalysisScanMode,
) -> HashMap<PathBuf, IndexedFile> {
    let queries: Vec<_> = children
        .iter()
        .filter(|(_, path, metadata)| {
            !metadata.is_dir()
                && !indexed
                    .get(path)
                    .is_some_and(|file| file_snapshot_matches(file, metadata))
        })
        .collect();
    if workers <= 1 || queries.len() < 256 {
        return HashMap::new();
    }
    let root = children
        .first()
        .and_then(|(_, path, _)| path.parent())
        .unwrap_or(Path::new(""));
    let chunks: Vec<_> = queries
        .chunks(queries.len().div_ceil(workers.clamp(1, 16)))
        .collect();
    std::thread::scope(|scope| {
        let mut pending = Vec::new();
        for (index, chunk) in chunks.into_iter().enumerate() {
            match std::thread::Builder::new()
                .name(format!("mangodisk-analysis-rows-{index}"))
                .spawn_scoped(scope, move || {
                    chunk
                        .iter()
                        .map(|(_, path, metadata)| {
                            let usage = scan_mode.file_usage(path, metadata);
                            (
                                path.clone(),
                                IndexedFile {
                                    shared_identity: None,
                                    bytes: usage.allocated_bytes,
                                    logical_bytes: usage.logical_bytes,
                                    modified_at_ms: modified_ms(metadata),
                                },
                            )
                        })
                        .collect::<Vec<_>>()
                }) {
                Ok(handle) => pending.push(handle),
                Err(error) => {
                    log::warn!("analysis_snapshot_worker_unavailable root={} stage=spawn error_code={:?} error={error} outcome=live_query_fallback", crate::filesystem::metadata::diagnostic_path(root), error.raw_os_error());
                }
            }
        }
        let mut measured = HashMap::with_capacity(queries.len());
        for handle in pending {
            match handle.join() {
                Ok(rows) => measured.extend(rows),
                Err(_) => log::warn!(
                    "analysis_snapshot_worker_unavailable root={} stage=join reason=worker_panic outcome=live_query_fallback", crate::filesystem::metadata::diagnostic_path(root)
                ),
            }
        }
        measured
    })
}

fn file_snapshot_matches(file: &IndexedFile, metadata: &fs::Metadata) -> bool {
    // Shared allocation is reconciled in the original scan metric. Remeasuring
    // one surviving name would mix live bytes with unchanged directory totals.
    file.shared_identity
        .is_some_and(|identity| current_platform().matches_file_identity(metadata, identity))
        || file.bytes == 0
        || (file.logical_bytes == metadata.len() && file.modified_at_ms == modified_ms(metadata))
}

/// Retain only shared-file ancestors, including zero-charge branches. This is
/// enough to restore allocation transfers after the larger index is evicted.
fn retain_shared_directories(
    result: &mut AnalysisResult,
    directories: &HashMap<PathBuf, DirectoryAggregate>,
    navigation: Option<&super::navigation::NavigationIndex>,
) {
    if result.shared_allocations.is_empty() {
        return;
    }
    let root = Path::new(&result.root);
    let mut paths = HashSet::new();
    for file in result
        .shared_allocations
        .iter()
        .flat_map(|group| &group.files)
    {
        let Some(parent) = Path::new(&file.path).parent() else {
            continue;
        };
        if parent == root {
            continue;
        }
        let Ok(relative) = parent.strip_prefix(root) else {
            continue;
        };
        let mut ancestor = root.to_path_buf();
        // Parse each relative path once instead of repeatedly stripping every
        // ancestor. One extra level preserves the depth-six chart's child counts.
        for component in relative.components().take(HIERARCHY_MAX_DEPTH + 1) {
            ancestor.push(component);
            paths.insert(ancestor.clone());
        }
    }
    let mut counts: HashMap<PathBuf, u64> = paths
        .iter()
        .filter_map(|path| {
            directories
                .get(path)
                .map(|aggregate| (path.clone(), aggregate.direct_file_count))
        })
        .collect();
    if let Some(navigation) = navigation {
        for (path, count) in &mut counts {
            *count += navigation.directory_count(path, directories);
        }
    } else {
        for (path, aggregate) in directories {
            if aggregate.bytes > 0 {
                if let Some(count) = path.parent().and_then(|parent| counts.get_mut(parent)) {
                    *count += 1;
                }
            }
        }
    }
    result.shared_directories = std::sync::Arc::new(
        paths
            .into_iter()
            .filter_map(|path| {
                let aggregate = directories.get(&path)?;
                Some(AnalysisDirectoryNode {
                    name: path.file_name()?.to_string_lossy().into_owned(),
                    path: display_path(&path),
                    bytes: aggregate.bytes,
                    file_count: aggregate.file_count,
                    total_entry_count: counts.get(&path).copied().unwrap_or(0),
                    children: Vec::new(),
                    files: Vec::new(),
                })
            })
            .collect(),
    );
}

pub(crate) fn build_directory_hierarchy(
    root: &Path,
    directories: &HashMap<PathBuf, DirectoryAggregate>,
    files: &HashMap<PathBuf, IndexedFile>,
) -> Vec<AnalysisDirectoryNode> {
    const MAX_DEPTH: usize = HIERARCHY_MAX_DEPTH;
    const MAX_CHILDREN: usize = HIERARCHY_MAX_CHILDREN;
    const MAX_NODES: usize = HIERARCHY_MAX_NODES;
    enum HierarchyEntry<'a> {
        Directory(&'a DirectoryAggregate),
        File(&'a IndexedFile),
    }
    impl HierarchyEntry<'_> {
        fn bytes(&self) -> u64 {
            match self {
                Self::Directory(directory) => directory.bytes,
                Self::File(file) => file.bytes,
            }
        }
    }
    let minimum_bytes = directories
        .get(root)
        .map_or(1, |aggregate| (aggregate.bytes / 2000).max(1));
    let mut children: HashMap<&Path, Vec<(&Path, HierarchyEntry<'_>)>> = HashMap::new();
    // Reuse both indexes without enumerating descendant files again. Reject small
    // candidates before parsing paths while navigation holds the shared cache lock.
    for (path, aggregate) in directories {
        if aggregate.bytes < minimum_bytes {
            continue;
        }
        let Ok(relative) = path.strip_prefix(root) else {
            continue;
        };
        let depth = relative.components().take(MAX_DEPTH + 1).count();
        if depth == 0 || depth > MAX_DEPTH {
            continue;
        }
        if let Some(parent) = path.parent() {
            children
                .entry(parent)
                .or_default()
                .push((path, HierarchyEntry::Directory(aggregate)));
        }
    }
    for (path, file) in files {
        if file.bytes < minimum_bytes {
            continue;
        }
        let Ok(relative) = path.strip_prefix(root) else {
            continue;
        };
        let depth = relative.components().take(MAX_DEPTH + 1).count();
        // Root files already belong to the complete flat result. Only descendants
        // need a read-only projection in the additional chart levels.
        if !(2..=MAX_DEPTH).contains(&depth) {
            continue;
        }
        if let Some(parent) = path.parent() {
            children
                .entry(parent)
                .or_default()
                .push((path, HierarchyEntry::File(file)));
        }
    }
    fn project<'a>(
        parent: &Path,
        children: &mut HashMap<&'a Path, Vec<(&'a Path, HierarchyEntry<'a>)>>,
        remaining: &mut usize,
        counts: &mut HashMap<&'a Path, u64>,
    ) -> (Vec<AnalysisDirectoryNode>, Vec<DirectoryEntryInfo>) {
        if *remaining == 0 {
            return (Vec::new(), Vec::new());
        }
        // Rank files and directories together so large files cannot be hidden by
        // directory-only limits. Unreachable branches never need sorting.
        let mut siblings = children.remove(parent).unwrap_or_default();
        siblings.sort_by(|(left_path, left), (right_path, right)| {
            right
                .bytes()
                .cmp(&left.bytes())
                .then_with(|| left_path.cmp(right_path))
        });
        siblings.truncate(MAX_CHILDREN);
        let mut nodes = Vec::new();
        let mut file_nodes = Vec::new();
        for (path, entry) in siblings {
            if *remaining == 0 {
                break;
            }
            *remaining -= 1;
            let name = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            match entry {
                HierarchyEntry::Directory(aggregate) => {
                    let (descendants, files) = project(path, children, remaining, counts);
                    counts.insert(path, aggregate.direct_file_count);
                    nodes.push(AnalysisDirectoryNode {
                        name,
                        path: display_path(path),
                        bytes: aggregate.bytes,
                        file_count: aggregate.file_count,
                        total_entry_count: 0,
                        children: descendants,
                        files,
                    });
                }
                HierarchyEntry::File(file) => file_nodes.push(DirectoryEntryInfo {
                    name,
                    path: display_path(path),
                    bytes: file.bytes,
                    logical_bytes: file.logical_bytes,
                    file_count: 1,
                    is_directory: false,
                    modified_at_ms: file.modified_at_ms,
                    content_fingerprint: None,
                }),
            }
        }
        (nodes, file_nodes)
    }
    let mut remaining = MAX_NODES;
    let mut counts = HashMap::new();
    let mut nodes = project(root, &mut children, &mut remaining, &mut counts).0;
    // Count small child directories too, but retain counters only for projected nodes.
    // This touches the in-memory directory index once; files need no second traversal.
    for (path, aggregate) in directories {
        if aggregate.bytes > 0 {
            if let Some(count) = path.parent().and_then(|parent| counts.get_mut(parent)) {
                *count += 1;
            }
        }
    }
    let counts: HashMap<String, u64> = counts
        .into_iter()
        .map(|(path, count)| (display_path(path), count))
        .collect();
    fn apply_counts(nodes: &mut [AnalysisDirectoryNode], counts: &HashMap<String, u64>) {
        for node in nodes {
            node.total_entry_count = counts.get(&node.path).copied().unwrap_or(0);
            apply_counts(&mut node.children, counts);
        }
    }
    apply_counts(&mut nodes, &counts);
    nodes
}

fn rank_visible_analysis_entries(entries: &mut Vec<DirectoryEntryInfo>) {
    let compare = |left: &DirectoryEntryInfo, right: &DirectoryEntryInfo| {
        right
            .bytes
            .cmp(&left.bytes)
            .then_with(|| left.path.cmp(&right.path))
    };
    // The result publishes only the largest bounded prefix. Partition first so
    // a directory with tens of thousands of files does not sort its entire tail.
    if entries.len() > ANALYSIS_VISIBLE_ENTRY_LIMIT {
        entries.select_nth_unstable_by(ANALYSIS_VISIBLE_ENTRY_LIMIT, compare);
        entries.truncate(ANALYSIS_VISIBLE_ENTRY_LIMIT);
    }
    entries.sort_unstable_by(compare);
}

fn read_analysis_children(
    root: &Path,
    excluded_roots: &[PathBuf],
    excluded_names: &mangodisk_platform::NameExclusions,
) -> Result<Vec<(fs::DirEntry, PathBuf, fs::Metadata)>, String> {
    read_analysis_children_with_metadata(
        root,
        excluded_roots,
        excluded_names,
        AnalysisChildMetadata::LiveView,
    )
}

enum AnalysisChildMetadata {
    ScanSnapshot,
    LiveView,
}

fn read_analysis_children_with_metadata(
    root: &Path,
    excluded_roots: &[PathBuf],
    excluded_names: &mangodisk_platform::NameExclusions,
    metadata_mode: AnalysisChildMetadata,
) -> Result<Vec<(fs::DirEntry, PathBuf, fs::Metadata)>, String> {
    Ok(fs::read_dir(root)
        .map_err(|error| format!("failed to read the analysis root: {error}"))?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            if excluded_roots
                .iter()
                .any(|excluded| current_platform().path_is_same_or_child(&path, excluded))
            {
                return None;
            }
            // Only initial result assembly can reuse enumeration facts. Reopened pages retain
            // live queries, independently of the immutable directory aggregates in the cache.
            let metadata = match metadata_mode {
                AnalysisChildMetadata::ScanSnapshot => {
                    crate::filesystem::metadata::scan_entry_metadata(&entry, &path)
                }
                AnalysisChildMetadata::LiveView => fs::symlink_metadata(&path),
            }
            .ok()?;
            // A directory omitted by the scan's system-safety policy has no aggregate.
            // Do not reintroduce it as a misleading zero-byte entry during result assembly.
            if metadata.is_dir()
                && current_platform().should_skip(&path, root, ScanPurpose::Analysis)
                    == Some(SkipReason::SystemCritical)
            {
                return None;
            }
            (!is_link_like(&metadata) && !excluded_names.matches_entry(&path, metadata.is_dir()))
                .then_some((entry, path, metadata))
        })
        .collect())
}

/// Reuse the active scan's largest file rows for file-heavy directories. Global
/// retention can remove a directory's tail, so the recorded native limit alone
/// is insufficient: verify that enough ordinary rows still survive. Shared
/// aliases are retained independently and their charged owners count separately.
/// More positive files than the visible limit also proves that omitted ordinary
/// zero-allocation files cannot affect the published rows or truncation flag.
/// Explicit refresh and destructive preflight remain the freshness boundaries.
fn cached_navigation_entries(
    root: &Path,
    aggregate: DirectoryAggregate,
    directories: &HashMap<PathBuf, DirectoryAggregate>,
    files: &HashMap<PathBuf, IndexedFile>,
) -> Option<(Vec<DirectoryEntryInfo>, usize)> {
    let direct_files: Vec<_> = files
        .iter()
        .filter(|(path, _)| path.parent() == Some(root))
        .collect();
    // More than the visible limit of positive children proves omitted empty
    // files cannot change the prefix or truncation. A complete direct snapshot
    // also preserves zero-charge aliases below that boundary.
    let complete_files = aggregate.file_count == direct_files.len() as u64;
    let positive_directories = directories
        .iter()
        .filter(|(path, directory)| path.parent() == Some(root) && directory.bytes > 0)
        .count();
    let total_entry_count = aggregate.direct_file_count as usize + positive_directories;
    if !complete_files && total_entry_count <= ANALYSIS_VISIBLE_ENTRY_LIMIT {
        return None;
    }
    let shared_owners = direct_files
        .iter()
        .filter(|(_, file)| file.bytes > 0 && file.shared_identity.is_some())
        .count();
    let ordinary_count = aggregate
        .direct_file_count
        .saturating_sub(shared_owners as u64);
    let retained_ordinary = direct_files
        .iter()
        .filter(|(_, file)| file.bytes > 0 && file.shared_identity.is_none())
        .count();
    if retained_ordinary < ordinary_count.min(ANALYSIS_VISIBLE_ENTRY_LIMIT as u64) as usize
        || (retained_ordinary < ordinary_count as usize
            && aggregate.retained_file_limit < ANALYSIS_VISIBLE_ENTRY_LIMIT)
    {
        return None;
    }
    let mut entries: Vec<_> = direct_files
        .into_iter()
        .map(|(path, file)| DirectoryEntryInfo {
            name: path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            path: display_path(path),
            bytes: file.bytes,
            logical_bytes: file.logical_bytes,
            file_count: 1,
            is_directory: false,
            modified_at_ms: file.modified_at_ms,
            content_fingerprint: None,
        })
        .collect();
    for (path, directory) in directories {
        if path.parent() != Some(root) {
            continue;
        }
        entries.push(DirectoryEntryInfo {
            name: path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            path: display_path(path),
            bytes: directory.bytes,
            logical_bytes: directory.logical_bytes,
            file_count: directory.file_count,
            is_directory: true,
            modified_at_ms: None,
            content_fingerprint: directory.fingerprint.map(display_fingerprint),
        });
    }
    Some((entries, total_entry_count))
}

fn build_analysis_result(
    root: &Path,
    root_aggregate: DirectoryAggregate,
    children: Vec<(fs::DirEntry, PathBuf, fs::Metadata)>,
    directory_aggregate: impl FnMut(&Path) -> Option<DirectoryAggregate>,
    indexed_file: impl FnMut(&Path) -> Option<IndexedFile>,
    shared_allocations: std::sync::Arc<Vec<crate::storage::analysis::SharedAllocation>>,
) -> AnalysisResult {
    let entries = build_analysis_entries(
        children,
        root_aggregate.scan_mode,
        directory_aggregate,
        indexed_file,
    );
    let total_entry_count = entries.iter().filter(|entry| entry.bytes > 0).count();
    build_analysis_result_from_entries(
        root,
        root_aggregate,
        entries,
        shared_allocations,
        total_entry_count,
    )
}

fn build_analysis_result_from_entries(
    root: &Path,
    root_aggregate: DirectoryAggregate,
    mut entries: Vec<DirectoryEntryInfo>,
    shared_allocations: std::sync::Arc<Vec<crate::storage::analysis::SharedAllocation>>,
    total_entry_count: usize,
) -> AnalysisResult {
    let shared_paths: HashSet<_> = shared_allocations
        .iter()
        .flat_map(|group| &group.files)
        .filter_map(|file| {
            let path = Path::new(&file.path);
            if path.parent() == Some(root) {
                return Some(file.path.clone());
            }
            let relative = path.strip_prefix(root).ok()?;
            Some(display_path(
                &root.join(relative.components().next()?.as_os_str()),
            ))
        })
        .collect();
    let shared_entries = std::sync::Arc::new(
        entries
            .iter()
            .filter(|entry| shared_paths.contains(&entry.path))
            .cloned()
            .collect(),
    );
    let truncated = entries.len() > ANALYSIS_VISIBLE_ENTRY_LIMIT
        || total_entry_count > ANALYSIS_VISIBLE_ENTRY_LIMIT;
    rank_visible_analysis_entries(&mut entries);

    AnalysisResult {
        scan_mode: root_aggregate.scan_mode,
        scan_id: 0,
        root: display_path(root),
        scanned_at_ms: root_aggregate.scanned_at_ms,
        total_bytes: root_aggregate.bytes,
        total_entry_count,
        skipped_count: root_aggregate.skipped_count,
        truncated,
        entries,
        directory_hierarchy: Vec::new(),
        shared_allocations,
        shared_entries,
        shared_directories: Default::default(),
    }
}

fn build_analysis_entries(
    children: Vec<(fs::DirEntry, PathBuf, fs::Metadata)>,
    scan_mode: AnalysisScanMode,
    mut directory_aggregate: impl FnMut(&Path) -> Option<DirectoryAggregate>,
    mut indexed_file: impl FnMut(&Path) -> Option<IndexedFile>,
) -> Vec<DirectoryEntryInfo> {
    children
        .into_iter()
        .map(|(entry, path, metadata)| {
            let aggregate = if metadata.is_dir() {
                directory_aggregate(&path).unwrap_or_default()
            } else {
                let usage = indexed_file(&path)
                    // Reopened read-only lists must reflect changed ordinary files.
                    // Zero-charge aliases keep ownership until deletion reconciles their group.
                    .filter(|file| file_snapshot_matches(file, &metadata))
                    .map(|file| mangodisk_platform::FileSpaceUsage {
                        logical_bytes: file.logical_bytes,
                        allocated_bytes: file.bytes,
                    })
                    .unwrap_or_else(|| scan_mode.file_usage(&path, &metadata));
                DirectoryAggregate {
                    bytes: usage.allocated_bytes,
                    logical_bytes: usage.logical_bytes,
                    file_count: u64::from(metadata.is_file()),
                    ..DirectoryAggregate::default()
                }
            };
            DirectoryEntryInfo {
                name: entry.file_name().to_string_lossy().into_owned(),
                path: display_path(&path),
                bytes: aggregate.bytes,
                logical_bytes: aggregate.logical_bytes,
                file_count: aggregate.file_count,
                is_directory: metadata.is_dir(),
                modified_at_ms: modified_ms(&metadata),
                content_fingerprint: metadata
                    .is_dir()
                    .then(|| aggregate.fingerprint.map(display_fingerprint))
                    .flatten(),
            }
        })
        .collect()
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
                    .is_some_and(|aggregate| aggregate.scan_mode != root_aggregate.scan_mode)
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
        AnalysisScanMode::Standard,
    );
}

pub(crate) fn remove_entry_in_mode(
    target: &Path,
    removed_usage: FileSpaceUsage,
    file_count: u64,
    is_directory: bool,
    scan_mode: AnalysisScanMode,
) {
    remove_entry_with_allocation_credits(
        target,
        removed_usage,
        file_count,
        is_directory,
        scan_mode,
        None,
    );
}

pub(crate) fn remove_entry_with_allocation_credits(
    target: &Path,
    removed_usage: FileSpaceUsage,
    file_count: u64,
    is_directory: bool,
    scan_mode: AnalysisScanMode,
    credits: Option<&[DirectoryEntryInfo]>,
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
                    .is_some_and(|aggregate| aggregate.scan_mode != scan_mode)
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
                    .saturating_sub(aggregate.scan_mode.displayed_bytes(removed_usage));
                aggregate.logical_bytes = aggregate
                    .logical_bytes
                    .saturating_sub(removed_usage.logical_bytes);
                aggregate.file_count = aggregate.file_count.saturating_sub(file_count);
                if !is_directory
                    && target.parent() == Some(directory.as_path())
                    && aggregate.scan_mode.displayed_bytes(removed_usage) > 0
                {
                    aggregate.direct_file_count = aggregate.direct_file_count.saturating_sub(1);
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

/// Retain complete known groups, including owners outside a cached child view.
fn shared_allocations(
    root: &Path,
    scope: &Path,
    files: &HashMap<PathBuf, IndexedFile>,
) -> std::sync::Arc<Vec<crate::storage::analysis::SharedAllocation>> {
    // Child navigation needs complete aliases only for identities in that child.
    // Select them before allocating rows: a small page must not rebuild every
    // unrelated hard-link group from a large parent scan.
    let selected_identities: Option<HashSet<_>> = (root != scope).then(|| {
        files
            .iter()
            .filter_map(|(path, file)| {
                file.shared_identity
                    .filter(|_| path.starts_with(root))
                    .map(|identity| (identity.volume, identity.index))
            })
            .collect()
    });
    shared_allocations_for_identities(root, scope, files, selected_identities)
}

fn shared_allocations_for_identities(
    root: &Path,
    scope: &Path,
    files: &HashMap<PathBuf, IndexedFile>,
    selected_identities: Option<HashSet<(u64, u64)>>,
) -> std::sync::Arc<Vec<crate::storage::analysis::SharedAllocation>> {
    use crate::storage::analysis::SharedAllocation;
    if selected_identities.as_ref().is_some_and(HashSet::is_empty) {
        return Default::default();
    }
    let mut groups: HashMap<(u64, u64), SharedAllocation> = HashMap::new();
    for (path, file) in files {
        let Some(identity) = file.shared_identity else {
            continue;
        };
        if selected_identities
            .as_ref()
            .is_some_and(|selected| !selected.contains(&(identity.volume, identity.index)))
        {
            continue;
        }
        if !path.starts_with(scope) {
            continue;
        }
        let group = groups
            .entry((identity.volume, identity.index))
            .or_insert_with(|| SharedAllocation {
                identity,
                owner: display_path(path),
                bytes: 0,
                files: Vec::new(),
            });
        if file.bytes > group.bytes {
            group.bytes = file.bytes;
            group.owner = display_path(path);
        }
        group.files.push(DirectoryEntryInfo {
            name: path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default(),
            path: display_path(path),
            bytes: file.bytes,
            logical_bytes: file.logical_bytes,
            file_count: 1,
            is_directory: false,
            modified_at_ms: file.modified_at_ms,
            content_fingerprint: None,
        });
    }
    let mut groups: Vec<_> = groups
        .into_values()
        .filter(|group| {
            group
                .files
                .iter()
                .any(|file| Path::new(&file.path).starts_with(root))
        })
        .collect();
    for group in &mut groups {
        group
            .files
            .sort_unstable_by(|left, right| left.path.cmp(&right.path));
    }
    groups.sort_unstable_by(|left, right| left.owner.cmp(&right.owner));
    std::sync::Arc::new(groups)
}

#[cfg(test)]
mod tests {
    #[test]
    fn cached_child_shared_groups_preserve_sibling_owners_among_unrelated_groups() {
        use super::*;
        let scope = std::env::temp_dir().join("mangodisk-navigation-groups");
        let child = scope.join("selected");
        let sibling = scope.join("sibling");
        let mut files = HashMap::new();
        for index in 0..20_000 {
            let identity = mangodisk_platform::PhysicalFileIdentity { volume: 1, index };
            for (parent, bytes) in [(&sibling, 4096), (&scope, 0)] {
                files.insert(
                    parent.join(format!("unrelated-{index}.bin")),
                    IndexedFile {
                        shared_identity: Some(identity),
                        bytes,
                        logical_bytes: 4096,
                        modified_at_ms: None,
                    },
                );
            }
        }
        let identity = mangodisk_platform::PhysicalFileIdentity {
            volume: 1,
            index: 20_000,
        };
        for (parent, bytes) in [(&sibling, 4096), (&child, 0)] {
            files.insert(
                parent.join("shared.bin"),
                IndexedFile {
                    shared_identity: Some(identity),
                    bytes,
                    logical_bytes: 4096,
                    modified_at_ms: None,
                },
            );
        }
        let started = std::time::Instant::now();
        for _ in 0..5 {
            let groups = shared_allocations(&child, &scope, &files);
            assert_eq!(groups.len(), 1);
            assert_eq!(groups[0].identity, identity);
            assert_eq!(groups[0].bytes, 4096);
            assert_eq!(groups[0].owner, display_path(&sibling.join("shared.bin")));
            assert_eq!(groups[0].files.len(), 2);
        }
        eprintln!(
            "navigation_group_projection files={} iterations=5 elapsed_us={}",
            files.len(),
            started.elapsed().as_micros()
        );
        let empty = scope.join("ordinary");
        assert!(shared_allocations(&empty, &scope, &files).is_empty());
    }

    #[test]
    fn empty_and_sparse_files_do_not_form_shared_allocation_groups() {
        use super::*;
        let root = std::env::temp_dir().join("mangodisk-shared-allocation-fixture");
        let ordinary = IndexedFile {
            shared_identity: None,
            bytes: 0,
            logical_bytes: 0,
            modified_at_ms: None,
        };
        let files = HashMap::from([
            (root.join("empty.bin"), ordinary),
            (
                root.join("sparse.bin"),
                IndexedFile {
                    logical_bytes: 8192,
                    ..ordinary
                },
            ),
        ]);
        assert!(shared_allocations(&root, &root, &files).is_empty());
    }

    #[test]
    fn bounded_analysis_ranking_matches_full_sort_at_limits_and_equal_byte_ties() {
        for count in [0, 1, 499, 500, 501, 30_000] {
            let mut entries = (0..count)
                .rev()
                .map(|index| super::DirectoryEntryInfo {
                    name: format!("file-{index}"),
                    path: format!("/fixture/file-{index:05}"),
                    bytes: (index * 7919 % 97) as u64,
                    logical_bytes: 0,
                    file_count: 1,
                    is_directory: false,
                    modified_at_ms: None,
                    content_fingerprint: None,
                })
                .collect::<Vec<_>>();
            let mut expected = entries.clone();
            expected.sort_by(|left, right| {
                right
                    .bytes
                    .cmp(&left.bytes)
                    .then_with(|| left.path.cmp(&right.path))
            });
            expected.truncate(super::ANALYSIS_VISIBLE_ENTRY_LIMIT);
            super::rank_visible_analysis_entries(&mut entries);
            assert_eq!(
                entries.iter().map(|entry| &entry.path).collect::<Vec<_>>(),
                expected.iter().map(|entry| &entry.path).collect::<Vec<_>>()
            );
        }
    }

    #[test]
    #[ignore = "manual ranking benchmark on a fixed large direct-child workload"]
    fn benchmark_bounded_analysis_ranking() {
        let source = (0..100_000)
            .map(|index| super::DirectoryEntryInfo {
                name: format!("file-{index}"),
                path: format!("/fixture/file-{index:06}"),
                bytes: (index * 7919 % 104729) as u64,
                logical_bytes: 0,
                file_count: 1,
                is_directory: false,
                modified_at_ms: None,
                content_fingerprint: None,
            })
            .collect::<Vec<_>>();
        for run in 0..10 {
            for bounded in if run % 2 == 0 {
                [false, true]
            } else {
                [true, false]
            } {
                let mut entries = source.clone();
                let start = std::time::Instant::now();
                if bounded {
                    super::rank_visible_analysis_entries(&mut entries);
                } else {
                    entries.sort_by(|left, right| {
                        right
                            .bytes
                            .cmp(&left.bytes)
                            .then_with(|| left.path.cmp(&right.path))
                    });
                    entries.truncate(super::ANALYSIS_VISIBLE_ENTRY_LIMIT);
                }
                println!(
                    "analysis_ranking bounded={bounded} run={run} elapsed_us={}",
                    start.elapsed().as_micros()
                );
                assert_eq!(entries.len(), 500);
            }
        }
    }

    use super::*;
    use crate::storage::large_files::LARGE_FILE_CANDIDATE_FLOOR_BYTES;

    #[test]
    fn hierarchy_projection_preserves_byte_floor_and_bounded_sorted_branches() {
        let root = Path::new("fixture");
        let mut directories = HashMap::new();
        directories.insert(
            root.to_path_buf(),
            DirectoryAggregate {
                bytes: 2_000_000,
                ..DirectoryAggregate::default()
            },
        );
        for index in 0..96 {
            let branch = root.join(format!("branch-{index:02}"));
            directories.insert(
                branch.clone(),
                DirectoryAggregate {
                    bytes: 20_000 + index,
                    file_count: 2,
                    ..DirectoryAggregate::default()
                },
            );
            for (name, bytes) in [("visible", 1_000), ("small", 999)] {
                directories.insert(
                    branch.join(name),
                    DirectoryAggregate {
                        bytes,
                        file_count: 1,
                        ..DirectoryAggregate::default()
                    },
                );
            }
        }
        directories.insert(
            PathBuf::from("unrelated/large"),
            DirectoryAggregate {
                bytes: 3_000_000,
                ..DirectoryAggregate::default()
            },
        );
        let nodes = build_directory_hierarchy(root, &directories, &HashMap::new());
        assert_eq!(nodes.len(), 64);
        for (offset, node) in nodes.iter().enumerate() {
            assert_eq!(node.name, format!("branch-{:02}", 95 - offset));
            assert_eq!(node.total_entry_count, 2);
            assert_eq!(node.children.len(), 1);
            assert_eq!(node.children[0].name, "visible");
            assert_eq!(node.children[0].bytes, 1_000);
        }
    }

    #[test]
    fn parallel_initial_rows_preserve_native_usage_stale_files_and_zero_charge_aliases() {
        let root = tempfile::tempdir().unwrap();
        for index in 0..600 {
            fs::write(
                root.path().join(format!("file-{index:04}")),
                vec![0_u8; index + 1],
            )
            .unwrap();
        }
        #[cfg(windows)]
        {
            let path = root.path().join("wof.bin");
            fs::write(&path, vec![0_u8; 1024 * 1024]).unwrap();
            let output = std::process::Command::new("compact.exe")
                .args(["/C", "/F", "/EXE:XPRESS4K"])
                .arg(&path)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "WOF fixture compression must succeed"
            );
            let metadata = fs::symlink_metadata(&path).unwrap();
            let usage = current_platform().file_space_usage(&path, &metadata);
            assert!(
                usage.allocated_bytes < usage.logical_bytes,
                "WOF fixture must have native compressed usage before comparing scan results"
            );
        }
        let files = HashMap::from([
            (
                root.path().join("file-0000"),
                IndexedFile {
                    shared_identity: None,
                    bytes: 0,
                    logical_bytes: 1,
                    modified_at_ms: None,
                },
            ),
            (
                root.path().join("file-0001"),
                IndexedFile {
                    shared_identity: None,
                    bytes: 9999,
                    logical_bytes: 9999,
                    modified_at_ms: None,
                },
            ),
        ]);
        let build = |workers| {
            analysis_result_from_snapshot(
                root.path(),
                DirectoryAggregate {
                    scanned_at_ms: 100,
                    ..DirectoryAggregate::default()
                },
                &HashMap::new(),
                &files,
                &[],
                &mangodisk_platform::NameExclusions::default(),
                workers,
            )
            .unwrap()
        };
        let expected = serde_json::to_value(build(1)).unwrap();
        for workers in [2, 4, 8] {
            assert_eq!(serde_json::to_value(build(workers)).unwrap(), expected);
        }
    }

    #[test]
    fn analysis_result_bounds_large_projections_and_preserves_exact_totals() {
        let root = tempfile::tempdir().unwrap();
        for index in 1..=ANALYSIS_VISIBLE_ENTRY_LIMIT {
            fs::write(root.path().join(format!("{index:05}.bin")), []).unwrap();
        }
        let build = || {
            let children = read_analysis_children(
                root.path(),
                &[],
                &mangodisk_platform::NameExclusions::default(),
            )
            .unwrap();
            build_analysis_result(
                root.path(),
                DirectoryAggregate::default(),
                children,
                |_| None,
                |path| {
                    let bytes = path.file_stem()?.to_str()?.parse().ok()?;
                    Some(IndexedFile {
                        shared_identity: None,
                        bytes,
                        logical_bytes: 0,
                        modified_at_ms: modified_ms(&fs::metadata(path).unwrap()),
                    })
                },
                Default::default(),
            )
        };
        let exact = build();
        assert_eq!(exact.entries.len(), ANALYSIS_VISIBLE_ENTRY_LIMIT);
        assert!(!exact.truncated);
        assert_eq!(exact.total_entry_count, ANALYSIS_VISIBLE_ENTRY_LIMIT);

        fs::write(
            root.path()
                .join(format!("{:05}.bin", ANALYSIS_VISIBLE_ENTRY_LIMIT + 1)),
            [],
        )
        .unwrap();
        let limited = build();
        assert_eq!(limited.entries.len(), ANALYSIS_VISIBLE_ENTRY_LIMIT);
        assert!(limited.truncated);
        assert_eq!(limited.total_entry_count, ANALYSIS_VISIBLE_ENTRY_LIMIT + 1);
        assert_eq!(
            limited.entries.first().unwrap().bytes,
            (ANALYSIS_VISIBLE_ENTRY_LIMIT + 1) as u64
        );
        assert_eq!(limited.entries.last().unwrap().name, "00002.bin");
        fs::write(root.path().join("000.bin"), []).unwrap();
        assert_eq!(
            build().total_entry_count,
            ANALYSIS_VISIBLE_ENTRY_LIMIT + 1,
            "zero-byte items are not part of Other"
        );
        assert_eq!(
            serde_json::to_value(limited).unwrap()["totalEntryCount"],
            ANALYSIS_VISIBLE_ENTRY_LIMIT + 1
        );
    }

    fn store_test_analysis_root(root: &Path, scanned_at_ms: u64) {
        let aggregate = DirectoryAggregate {
            scanned_at_ms,
            ..DirectoryAggregate::default()
        };
        store_memory_only(
            root,
            aggregate,
            HashMap::from([(root.to_path_buf(), aggregate)]),
            HashMap::new(),
            SnapshotPublication::new(
                ScanPurpose::Analysis,
                true,
                None,
                scanned_at_ms,
                mutation_revision().expect("test cache revision should load"),
            ),
        )
        .expect("test analysis result should store");
    }

    #[test]
    fn remainder_entries_include_unindexed_small_files_and_reconcile_allocated_bytes() {
        let _operation_lock = crate::shared::operation::test_operation_lock();
        clear_all().unwrap();
        let directory = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(directory.path()).unwrap();
        let allocated = |path: &Path| {
            current_platform()
                .file_space_usage(path, &fs::symlink_metadata(path).unwrap())
                .allocated_bytes
        };
        let visible = root.join("visible.bin");
        fs::write(&visible, vec![3; 32_768]).unwrap();
        let mut remainder_bytes = 0;
        for index in 0..230 {
            let path = root.join(format!("small-{index:03}.bin"));
            fs::write(&path, vec![index as u8; 1_024]).unwrap();
            remainder_bytes += allocated(&path);
        }
        let nested = root.join("nested");
        fs::create_dir(&nested).unwrap();
        let child = nested.join("child.bin");
        fs::write(&child, vec![1; 1_024]).unwrap();
        let child_bytes = allocated(&child);
        remainder_bytes += child_bytes;
        let aggregate = DirectoryAggregate {
            bytes: remainder_bytes + allocated(&visible),
            scanned_at_ms: 12,
            ..Default::default()
        };
        store_memory_only(
            &root,
            aggregate,
            HashMap::from([
                (root.clone(), aggregate),
                (
                    nested.clone(),
                    DirectoryAggregate {
                        bytes: child_bytes,
                        scanned_at_ms: 12,
                        ..Default::default()
                    },
                ),
            ]),
            HashMap::new(),
            SnapshotPublication::new(
                ScanPurpose::Analysis,
                true,
                None,
                12,
                mutation_revision().unwrap(),
            ),
        )
        .unwrap();
        let parent = AnalysisRemainderParent {
            scan_mode: AnalysisScanMode::Standard,
            path: display_path(&root),
            bytes: aggregate.bytes,
            exclusions: Default::default(),
        };
        let mut request = AnalysisRemainderRequest {
            schema_version: AnalysisRemainderRequest::SCHEMA_VERSION,
            snapshot_id: None,
            scan_id: 1,
            parent_path: parent.path.clone(),
            visible_paths: vec![display_path(&visible)],
            expected_bytes: remainder_bytes,
            offset: 0,
        };
        let (total_bytes, entries) = analysis_remainder_entries(&root, &parent, &request).unwrap();
        assert_eq!(entries.len(), 231);
        assert_eq!(total_bytes, remainder_bytes);
        assert_eq!(
            entries.iter().map(|entry| entry.bytes).sum::<u64>(),
            remainder_bytes
        );
        assert_eq!(
            entries
                .iter()
                .map(|entry| &entry.path)
                .collect::<HashSet<_>>()
                .len(),
            231
        );
        assert!(entries
            .iter()
            .any(|entry| entry.name == "nested" && entry.is_directory));
        assert!(entries.iter().all(|entry| entry.name != "visible.bin"));
        fs::write(root.join("new.bin"), vec![7; 1_024]).unwrap();
        request.offset = 0;
        let (changed_bytes, changed_entries) =
            analysis_remainder_entries(&root, &parent, &request).unwrap();
        assert_eq!(changed_entries.len(), 232);
        assert!(changed_bytes > remainder_bytes);
    }

    #[test]
    fn missing_change_token_is_stale() {
        let validation =
            validate_change_token(Path::new("/missing-change-token"), None, None, &|| false)
                .expect("validation should succeed");
        assert!(matches!(validation, ChangeValidation::Stale));
    }

    #[test]
    fn large_file_entries_are_derived_from_an_index_snapshot() {
        let root = PathBuf::from("/memory-large-files");
        let file = root.join("large.bin");
        let mut files = HashMap::from([(
            file,
            IndexedFile {
                shared_identity: None,
                bytes: LARGE_FILE_CANDIDATE_FLOOR_BYTES,
                logical_bytes: LARGE_FILE_CANDIDATE_FLOOR_BYTES,
                modified_at_ms: Some(5),
            },
        )]);

        files.insert(
            root.join("chart-candidate.bin"),
            IndexedFile {
                shared_identity: None,
                bytes: 4096,
                logical_bytes: 4096,
                modified_at_ms: None,
            },
        );
        let entries = large_file_entries_from_snapshot(&root, &files);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].bytes, LARGE_FILE_CANDIDATE_FLOOR_BYTES);
    }

    #[test]
    fn uncached_file_removal_subtracts_allocated_and_logical_sizes_independently() {
        let _operation_lock = crate::shared::operation::test_operation_lock();
        clear_all().expect("cache should clear");
        let root = PathBuf::from("/memory-analysis-removal");
        let file = root.join("small.bin");
        let aggregate = DirectoryAggregate {
            bytes: 4_096,
            logical_bytes: 1,
            file_count: 1,
            scanned_at_ms: 10,
            ..DirectoryAggregate::default()
        };
        store_memory_only(
            &root,
            aggregate,
            HashMap::from([(root.clone(), aggregate)]),
            HashMap::new(),
            SnapshotPublication::new(
                ScanPurpose::Analysis,
                true,
                None,
                10,
                mutation_revision().expect("test cache revision should load"),
            ),
        )
        .expect("analysis result should store");

        remove_entry(
            &file,
            FileSpaceUsage {
                logical_bytes: 1,
                allocated_bytes: 4_096,
            },
            1,
            false,
        );

        let cache = cache().lock().expect("cache should remain available");
        let updated = cache
            .directories
            .get(&root)
            .expect("the containing aggregate should remain cached");
        assert_eq!(updated.bytes, 0);
        assert_eq!(updated.logical_bytes, 0);
        assert_eq!(updated.file_count, 0);
        drop(cache);
        clear_all().expect("cache should clear");
    }

    #[test]
    fn clearing_memory_removes_the_only_scan_result() {
        let _operation_lock = crate::shared::operation::test_operation_lock();
        clear_all().expect("cache should clear");
        let root = PathBuf::from("/memory-only-result");
        let aggregate = DirectoryAggregate {
            scanned_at_ms: 9,
            ..DirectoryAggregate::default()
        };
        store_memory_only(
            &root,
            aggregate,
            HashMap::from([(root.clone(), aggregate)]),
            HashMap::new(),
            SnapshotPublication::new(
                ScanPurpose::Analysis,
                true,
                None,
                9,
                mutation_revision().expect("test cache revision should load"),
            ),
        )
        .expect("memory result should store");
        clear_all().expect("cache should clear");
        assert!(analysis_result(&root)
            .expect("cache lookup should succeed")
            .is_none());
    }

    #[test]
    fn older_concurrent_snapshot_cannot_replace_a_newer_generation() {
        let _operation_lock = crate::shared::operation::test_operation_lock();
        clear_all().expect("cache should clear");
        let root = PathBuf::from("/memory-generation-order");
        let newer = DirectoryAggregate {
            bytes: 2,
            logical_bytes: 2,
            file_count: 1,
            scanned_at_ms: 2,
            ..DirectoryAggregate::default()
        };
        let revision = mutation_revision().expect("cache revision should load");
        assert!(store_memory_only(
            &root,
            newer,
            HashMap::from([(root.clone(), newer)]),
            HashMap::new(),
            SnapshotPublication::new(ScanPurpose::Analysis, true, None, 20, revision),
        )
        .expect("newer snapshot should publish"));

        let older = DirectoryAggregate {
            bytes: 1,
            logical_bytes: 1,
            file_count: 1,
            scanned_at_ms: 1,
            ..DirectoryAggregate::default()
        };
        assert!(!store_memory_only(
            &root,
            older,
            HashMap::from([(root.clone(), older)]),
            HashMap::new(),
            SnapshotPublication::new(ScanPurpose::Analysis, true, None, 10, revision),
        )
        .expect("older snapshot should be skipped safely"));
        assert_eq!(
            cache()
                .lock()
                .expect("cache should remain available")
                .directories
                .get(&root)
                .expect("newer root should remain")
                .scanned_at_ms,
            2
        );
        clear_all().expect("cache should clear");
    }

    #[test]
    fn older_ancestor_snapshot_cannot_erase_a_newer_descendant() {
        let _operation_lock = crate::shared::operation::test_operation_lock();
        clear_all().expect("cache should clear");
        let root = PathBuf::from("/memory-overlapping-generation");
        let descendant = root.join("nested");
        let newer = DirectoryAggregate {
            bytes: 2,
            logical_bytes: 2,
            file_count: 1,
            scanned_at_ms: 2,
            ..DirectoryAggregate::default()
        };
        let revision = mutation_revision().expect("cache revision should load");
        assert!(store_memory_only(
            &descendant,
            newer,
            HashMap::from([(descendant.clone(), newer)]),
            HashMap::new(),
            SnapshotPublication::new(ScanPurpose::Analysis, true, None, 20, revision),
        )
        .expect("newer descendant should publish"));

        let older = DirectoryAggregate {
            bytes: 1,
            logical_bytes: 1,
            file_count: 1,
            scanned_at_ms: 1,
            ..DirectoryAggregate::default()
        };
        assert!(!store_memory_only(
            &root,
            older,
            HashMap::from([(root.clone(), older)]),
            HashMap::new(),
            SnapshotPublication::new(ScanPurpose::Analysis, true, None, 10, revision),
        )
        .expect("older ancestor should be skipped safely"));
        let cache = cache().lock().expect("cache should remain available");
        assert!(cache.directories.contains_key(&descendant));
        assert!(!cache.directories.contains_key(&root));
        drop(cache);
        clear_all().expect("cache should clear");
    }

    #[test]
    fn newer_ancestor_replaces_cached_descendant_without_an_explicit_refresh() {
        let _operation_lock = crate::shared::operation::test_operation_lock();
        clear_all().expect("cache should clear");
        let root = PathBuf::from("/memory-overlapping-replacement");
        let descendant = root.join("nested");
        let unrelated = PathBuf::from("/memory-overlapping-unrelated");
        let stale_file = descendant.join("stale.bin");
        store_test_analysis_root(&unrelated, 5);
        let old = DirectoryAggregate {
            bytes: 1,
            logical_bytes: 1,
            file_count: 1,
            scanned_at_ms: 1,
            ..DirectoryAggregate::default()
        };
        let revision = mutation_revision().expect("cache revision should load");
        assert!(store_memory_only(
            &descendant,
            old,
            HashMap::from([(descendant.clone(), old)]),
            HashMap::from([(
                stale_file.clone(),
                IndexedFile {
                    shared_identity: None,
                    bytes: 1,
                    logical_bytes: 1,
                    modified_at_ms: None,
                },
            )]),
            SnapshotPublication::new(ScanPurpose::Analysis, true, None, 10, revision),
        )
        .expect("descendant snapshot should publish"));

        let replacement = DirectoryAggregate {
            bytes: 2,
            logical_bytes: 2,
            file_count: 1,
            scanned_at_ms: 2,
            ..DirectoryAggregate::default()
        };
        assert!(store_memory_only(
            &root,
            replacement,
            HashMap::from([
                (root.clone(), replacement),
                (descendant.clone(), replacement),
            ]),
            HashMap::new(),
            SnapshotPublication::new(ScanPurpose::Analysis, false, None, 20, revision),
        )
        .expect("newer ancestor snapshot should publish"));

        let cache = cache().lock().expect("cache should remain available");
        assert_eq!(cache.scan_roots.len(), 2);
        assert!(cache.scan_roots.contains_key(&root));
        assert!(cache.scan_roots.contains_key(&unrelated));
        assert!(!cache.scan_roots.contains_key(&descendant));
        assert!(!cache.files.contains_key(&stale_file));
        assert_eq!(
            cache
                .directories
                .get(&descendant)
                .expect("replacement descendant should remain")
                .scanned_at_ms,
            2
        );
        drop(cache);
        clear_all().expect("cache should clear");
    }

    #[test]
    fn concurrent_mutation_prevents_stale_snapshot_publication() {
        let _operation_lock = crate::shared::operation::test_operation_lock();
        clear_all().expect("cache should clear");
        let root = PathBuf::from("/memory-concurrent-mutation");
        let aggregate = DirectoryAggregate {
            bytes: 1,
            logical_bytes: 1,
            file_count: 1,
            scanned_at_ms: 1,
            ..DirectoryAggregate::default()
        };
        let revision = mutation_revision().expect("cache revision should load");
        remove_entry(
            &root.join("removed.bin"),
            FileSpaceUsage {
                logical_bytes: 1,
                allocated_bytes: 1,
            },
            1,
            false,
        );

        assert!(!store_memory_only(
            &root,
            aggregate,
            HashMap::from([(root.clone(), aggregate)]),
            HashMap::new(),
            SnapshotPublication::new(ScanPurpose::Analysis, true, None, 1, revision),
        )
        .expect("stale publication should be skipped safely"));
        assert!(!cache()
            .lock()
            .expect("cache should remain available")
            .directories
            .contains_key(&root));
        clear_all().expect("cache should clear");
    }

    #[test]
    fn storing_a_third_root_evicts_only_the_least_recent_root() {
        let _operation_lock = crate::shared::operation::test_operation_lock();
        clear_all().expect("cache should clear");
        let root_a = PathBuf::from("/memory-lru-a");
        let root_b = PathBuf::from("/memory-lru-b");
        let root_c = PathBuf::from("/memory-lru-c");

        store_test_analysis_root(&root_a, 1);
        store_test_analysis_root(&root_b, 2);
        store_test_analysis_root(&root_c, 3);

        let cache = cache().lock().expect("cache should be readable");
        assert!(!cache.scan_roots.contains_key(&root_a));
        assert!(!cache.directories.contains_key(&root_a));
        assert!(cache.scan_roots.contains_key(&root_b));
        assert!(cache.directories.contains_key(&root_b));
        assert!(cache.scan_roots.contains_key(&root_c));
        assert!(cache.directories.contains_key(&root_c));
        assert_eq!(
            cache.root_recency,
            VecDeque::from([root_b.clone(), root_c.clone()])
        );
        drop(cache);
        clear_all().expect("cache should clear");
    }

    #[test]
    fn marking_a_root_recent_updates_its_eviction_recency() {
        let _operation_lock = crate::shared::operation::test_operation_lock();
        clear_all().expect("cache should clear");
        let root_a = PathBuf::from("/memory-lru-reused-a");
        let root_b = PathBuf::from("/memory-lru-reused-b");
        let root_c = PathBuf::from("/memory-lru-reused-c");

        store_test_analysis_root(&root_a, 1);
        store_test_analysis_root(&root_b, 2);
        mark_root_recent(&root_a).expect("the cached root should move to the recent end");
        store_test_analysis_root(&root_c, 3);

        let cache = cache().lock().expect("cache should be readable");
        assert!(cache.scan_roots.contains_key(&root_a));
        assert!(!cache.scan_roots.contains_key(&root_b));
        assert!(cache.scan_roots.contains_key(&root_c));
        assert_eq!(
            cache.root_recency,
            VecDeque::from([root_a.clone(), root_c.clone()])
        );
        drop(cache);
        clear_all().expect("cache should clear");
    }
}
