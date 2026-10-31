use super::{
    hierarchy::{build_directory_hierarchy, hierarchy_from_navigation, HIERARCHY_MAX_DEPTH},
    AnalysisDirectoryNode, AnalysisRemainderParent, AnalysisRemainderRequest, AnalysisResult,
    AnalysisScanMode, DirectoryEntryInfo, ANALYSIS_VISIBLE_ENTRY_LIMIT,
};
use crate::{
    filesystem::metadata::{display_fingerprint, display_path, is_link_like, modified_ms},
    storage::index::cache::{read_snapshot, DirectoryAggregate, IndexedFile},
};
use mangodisk_platform::{current_platform, Platform, ScanPurpose, SkipReason};
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
};
pub(crate) fn analysis_result(root: &Path) -> Result<Option<AnalysisResult>, String> {
    let started = std::time::Instant::now();
    let (root_aggregate, excluded_roots, cached_entries, revision) = read_snapshot(|cache| {
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
                    .direct_snapshot(root, cache.directories, cache.files);
            cached_navigation_entries(root, aggregate, &directories, &files)
        });
        let revision = (cache.mutation_revision, cache.publish_generations.clone());
        (aggregate, excluded_roots, cached_entries, revision)
    })?;
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
    read_snapshot(|cache| {
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
        let shared = match cache.navigation.leaf_shared_identities(root, cache.files) {
            Some(identities) => {
                shared_allocations_for_identities(root, scope, cache.files, Some(identities))
            }
            None => shared_allocations(root, scope, cache.files),
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
        retain_shared_directories(&mut result, cache.directories, Some(cache.navigation));
        let rows_ms = started
            .elapsed()
            .as_millis()
            .saturating_sub(metadata_ms + shared_ms);
        result.directory_hierarchy =
            hierarchy_from_navigation(cache.navigation, root, cache.directories, cache.files);
        result.index_snapshot = super::export::index_snapshot(&cache, root);
        log::info!("analysis_cache_projection_finished root={} projected_rows={} positive_children={} visible_rows={} snapshot_rows={} indexed_directories={} indexed_files={} metadata_ms={} shared_ms={} rows_ms={} hierarchy_ms={} elapsed_ms={}", crate::filesystem::metadata::diagnostic_path(root), projected_rows, result.total_entry_count, result.entries.len(), snapshot_rows, cache.directories.len(), cache.files.len(), metadata_ms, shared_ms, rows_ms, started.elapsed().as_millis().saturating_sub(metadata_ms + shared_ms + rows_ms), started.elapsed().as_millis());
        Ok(Some(result))
    })?
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
    let mut entries = read_snapshot(|cache| {
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
            .is_some_and(|aggregate| aggregate.byte_metric != parent.scan_mode.into())
        {
            return Err("the analysis index mode changed; scan again".to_string());
        }
        Ok(build_analysis_entries(
            children,
            parent.scan_mode,
            |path| cache.directories.get(path).copied(),
            |path| cache.files.get(path).copied(),
        ))
    })??;
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
    let measured =
        measure_snapshot_files(&children, files, workers, root_aggregate.byte_metric.into());
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
    navigation: Option<&crate::storage::index::navigation::NavigationIndex>,
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
            size_status: Default::default(),
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
            size_status: super::coverage::entry_size_status(*directory),
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
        root_aggregate.byte_metric.into(),
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
        index_snapshot: None,
        policy_version: super::ANALYSIS_SCAN_POLICY_VERSION,
        coverage: super::AnalysisCoverage::new(
            root_aggregate.omissions,
            root_aggregate.skipped_count,
        ),
        scan_mode: root_aggregate.byte_metric.into(),
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
                directory_aggregate(&path).unwrap_or(DirectoryAggregate {
                    skipped_count: 1,
                    omissions: mangodisk_platform::ScanOmissionCounts {
                        other: 1,
                        ..Default::default()
                    },
                    ..Default::default()
                })
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
                size_status: super::coverage::entry_size_status(aggregate),
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
            size_status: Default::default(),
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
#[path = "index_projection_tests.rs"]
mod tests;
