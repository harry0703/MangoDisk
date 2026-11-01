use super::{
    directory_loader::{
        build_analysis_entries, read_analysis_children, read_analysis_children_with_metadata,
        AnalysisChildMetadata,
    },
    hierarchy::{build_directory_hierarchy, HIERARCHY_MAX_DEPTH},
    query::{AnalysisQuery, CachedList},
    AnalysisDirectoryNode, AnalysisResult, DirectoryEntryInfo, ANALYSIS_VISIBLE_ENTRY_LIMIT,
};
use crate::{
    filesystem::metadata::display_path,
    storage::index::cache::{read_snapshot, DirectoryAggregate, IndexedFile},
};
use mangodisk_platform::{current_platform, Platform};
use std::{
    collections::{HashMap, HashSet},
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
        let cached_entries = aggregate
            .map(|aggregate| AnalysisQuery::from_cache(&cache).cached_list(root, aggregate))
            .transpose()
            .map_err(|error| error.to_string());
        let revision = (cache.mutation_revision, cache.publish_generations.clone());
        (aggregate, excluded_roots, cached_entries, revision)
    })?;
    let cached_entries = cached_entries?;
    let Some(root_aggregate) = root_aggregate else {
        return Ok(None);
    };

    let cached_entries = match cached_entries {
        Some(CachedList::Ready {
            entries,
            total_entry_count,
        }) => Some((entries, total_entry_count)),
        Some(CachedList::NeedsDirectoryRead(reason)) => {
            log::info!(
                "analysis_directory_read_required root={} reason={} stage=navigation",
                crate::filesystem::metadata::diagnostic_path(root),
                reason.as_str()
            );
            None
        }
        None => None,
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
        let (entries, count) = if let Some(rows) = cached_entries {
            rows
        } else {
            let entries = build_analysis_entries(
                children,
                root_aggregate.byte_metric.into(),
                |path| cache.directories.get(path).copied(),
                |path| cache.files.get(path).copied(),
            );
            let count = entries.iter().filter(|entry| entry.bytes > 0).count();
            (entries, count)
        };
        let query = AnalysisQuery::from_cache(&cache);
        let (mut result, hierarchy_ms) =
            project_result(&query, root, root_aggregate, entries, count, shared)?;
        let rows_ms = started
            .elapsed()
            .as_millis()
            .saturating_sub(metadata_ms + shared_ms + hierarchy_ms);
        result.index_snapshot = super::query::index_snapshot(&cache, root);
        log::info!("analysis_cache_projection_finished root={} projected_rows={} positive_children={} visible_rows={} snapshot_rows={} indexed_directories={} indexed_files={} metadata_ms={} shared_ms={} rows_ms={} hierarchy_ms={} elapsed_ms={}", crate::filesystem::metadata::diagnostic_path(root), projected_rows, result.total_entry_count, result.entries.len(), snapshot_rows, cache.directories.len(), cache.files.len(), metadata_ms, shared_ms, rows_ms, hierarchy_ms, started.elapsed().as_millis());
        Ok(Some(result))
    })?
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
    let query = AnalysisQuery::from_snapshot(directories, files);
    let (entries, total_entry_count) = match query
        .cached_list(root, root_aggregate)
        .map_err(|error| error.to_string())?
    {
        CachedList::Ready {
            entries,
            total_entry_count,
        } => (entries, total_entry_count),
        CachedList::NeedsDirectoryRead(reason) => {
            log::info!(
                "analysis_directory_read_required root={} reason={} stage=initial_result",
                crate::filesystem::metadata::diagnostic_path(root),
                reason.as_str()
            );
            let children = read_analysis_children_with_metadata(
                root,
                excluded_roots,
                excluded_names,
                AnalysisChildMetadata::ScanSnapshot,
            )?;
            #[cfg(any(windows, test))]
            let measured = super::directory_loader::measure_snapshot_files(
                &children,
                files,
                workers,
                root_aggregate.byte_metric.into(),
            );
            #[cfg(not(any(windows, test)))]
            let _ = workers;
            let entries = build_analysis_entries(
                children,
                root_aggregate.byte_metric.into(),
                |path| directories.get(path).copied(),
                |path| {
                    #[cfg(any(windows, test))]
                    if let Some(file) = measured.get(path) {
                        return Some(*file);
                    }
                    files.get(path).copied()
                },
            );
            let count = entries.iter().filter(|entry| entry.bytes > 0).count();
            (entries, count)
        }
    };
    project_result(
        &query,
        root,
        root_aggregate,
        entries,
        total_entry_count,
        shared_allocations(root, root, files),
    )
    .map(|(result, _)| result)
}

fn project_result(
    query: &AnalysisQuery<'_>,
    root: &Path,
    aggregate: DirectoryAggregate,
    entries: Vec<DirectoryEntryInfo>,
    total_entry_count: usize,
    shared: std::sync::Arc<Vec<crate::storage::analysis::SharedAllocation>>,
) -> Result<(AnalysisResult, u128), String> {
    let mut result =
        build_analysis_result_from_entries(root, aggregate, entries, shared, total_entry_count);
    retain_shared_directories(&mut result, query.directories, query.navigation());
    let started = std::time::Instant::now();
    result.directory_hierarchy = if query.navigation().is_some() {
        super::hierarchy::hierarchy_from_query(query, root).map_err(|error| error.to_string())?
    } else {
        build_directory_hierarchy(root, query.directories, query.files)
    };
    Ok((result, started.elapsed().as_millis()))
}

/// Keep the bounded shared-file ancestors needed for allocation transfers after index eviction.
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
    super::query::rank_entries(&mut entries);

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
        group.files.push(super::query::file_entry(path, *file));
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
