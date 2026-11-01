use super::{
    query::{directory_entry, file_entry},
    AnalysisRemainderParent, AnalysisRemainderRequest, AnalysisScanMode, DirectoryEntryInfo,
};
use crate::{
    filesystem::metadata::{is_link_like, modified_ms},
    storage::index::cache::{read_snapshot, DirectoryAggregate, IndexedFile},
};
use mangodisk_platform::{current_platform, Platform, ScanPurpose, SkipReason};
#[cfg(any(windows, test))]
use std::collections::HashMap;
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

#[cfg(any(windows, test))]
pub(super) fn measure_snapshot_files(
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

/// Optional direct-child loading belongs here; retained-index queries never call it.
pub(super) fn read_analysis_children(
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

pub(super) enum AnalysisChildMetadata {
    ScanSnapshot,
    LiveView,
}

pub(super) fn read_analysis_children_with_metadata(
    root: &Path,
    excluded_roots: &[PathBuf],
    excluded_names: &mangodisk_platform::NameExclusions,
    metadata_mode: AnalysisChildMetadata,
) -> Result<Vec<(fs::DirEntry, PathBuf, fs::Metadata)>, String> {
    let started = std::time::Instant::now();
    let mut failures = 0_usize;
    let mut rows = Vec::new();
    let children = fs::read_dir(root).map_err(|error| {
        log_read_failure(root, "open", &error);
        format!("failed to read the analysis root: {error}")
    })?;
    for entry in children {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                if failures < 4 {
                    log_read_failure(root, "enumerate", &error);
                }
                failures += 1;
                continue;
            }
        };
        let path = entry.path();
        if excluded_roots
            .iter()
            .any(|excluded| current_platform().path_is_same_or_child(&path, excluded))
        {
            continue;
        }
        // Initial assembly may reuse enumeration facts. Reopened pages retain their existing
        // live-read behavior; the resulting view is frozen before consumers receive it.
        let metadata = match metadata_mode {
            AnalysisChildMetadata::ScanSnapshot => {
                crate::filesystem::metadata::scan_entry_metadata(&entry, &path)
            }
            AnalysisChildMetadata::LiveView => fs::symlink_metadata(&path),
        };
        let metadata = match metadata {
            Ok(metadata) => metadata,
            Err(error) => {
                if failures < 4 {
                    log_read_failure(&path, "metadata", &error);
                }
                failures += 1;
                continue;
            }
        };
        if metadata.is_dir()
            && current_platform().should_skip(&path, root, ScanPurpose::Analysis)
                == Some(SkipReason::SystemCritical)
        {
            continue;
        }
        if !is_link_like(&metadata) && !excluded_names.matches_entry(&path, metadata.is_dir()) {
            rows.push((entry, path, metadata));
        }
    }
    let source = match metadata_mode {
        AnalysisChildMetadata::ScanSnapshot => "scan_metadata",
        AnalysisChildMetadata::LiveView => "live_metadata",
    };
    log::info!(
        "analysis_directory_loaded root={} source={} entries={} read_failures={} elapsed_ms={}",
        crate::filesystem::metadata::diagnostic_path(root),
        source,
        rows.len(),
        failures,
        started.elapsed().as_millis()
    );
    Ok(rows)
}

fn log_read_failure(path: &Path, stage: &str, error: &std::io::Error) {
    log::warn!("analysis_directory_read_failed path={} stage={} error_code={:?} error={} outcome=not_loaded", crate::filesystem::metadata::diagnostic_path(path), stage, error.raw_os_error(), mangodisk_platform::diagnostics::text(error));
}

pub(super) fn build_analysis_entries(
    children: Vec<(fs::DirEntry, PathBuf, fs::Metadata)>,
    scan_mode: AnalysisScanMode,
    mut directory_aggregate: impl FnMut(&Path) -> Option<DirectoryAggregate>,
    mut indexed_file: impl FnMut(&Path) -> Option<IndexedFile>,
) -> Vec<DirectoryEntryInfo> {
    children
        .into_iter()
        .map(|(_, path, metadata)| {
            let mut entry = if metadata.is_dir() {
                directory_entry(
                    &path,
                    directory_aggregate(&path).unwrap_or(DirectoryAggregate {
                        skipped_count: 1,
                        omissions: mangodisk_platform::ScanOmissionCounts {
                            other: 1,
                            ..Default::default()
                        },
                        ..Default::default()
                    }),
                )
            } else {
                let usage = indexed_file(&path)
                    .filter(|file| file_snapshot_matches(file, &metadata))
                    .map(|file| mangodisk_platform::FileSpaceUsage {
                        logical_bytes: file.logical_bytes,
                        allocated_bytes: file.bytes,
                    })
                    .unwrap_or_else(|| scan_mode.file_usage(&path, &metadata));
                let mut entry = file_entry(
                    &path,
                    IndexedFile {
                        shared_identity: None,
                        bytes: usage.allocated_bytes,
                        logical_bytes: usage.logical_bytes,
                        modified_at_ms: modified_ms(&metadata),
                    },
                );
                entry.file_count = u64::from(metadata.is_file());
                entry
            };
            entry.modified_at_ms = modified_ms(&metadata);
            entry
        })
        .collect()
}

/// Reads current direct children, reusing scan sizes for indexed entries.
/// Read-only details tolerate filesystem and cache changes without extending
/// the authoritative snapshot used by destructive operations.
pub(in crate::storage) fn analysis_remainder_entries(
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
