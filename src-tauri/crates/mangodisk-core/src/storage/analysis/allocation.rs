use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use crate::storage::index::cache::{self, DirectoryAggregate, IndexedFile};

use mangodisk_platform::{current_platform, Platform};

use super::{AnalysisDirectoryNode, AnalysisResult, DirectoryEntryInfo};

fn contains(root: &str, path: &str) -> bool {
    current_platform().path_is_same_or_child(Path::new(path), Path::new(root))
}

/// Reassign only the groups touched by this deletion. Known alias paths are
/// checked by physical identity; no directory or volume traversal is needed.
pub(super) fn reconcile_shared_allocations(
    result: &mut AnalysisResult,
    removed_path: &Path,
) -> Result<Vec<DirectoryEntryInfo>, String> {
    let mut credits = Vec::new();
    let allocations = Arc::make_mut(&mut result.shared_allocations);
    for group in allocations.iter_mut() {
        group.files.retain(|file| {
            !current_platform().path_is_same_or_child(Path::new(&file.path), removed_path)
        });
        if !current_platform().path_is_same_or_child(Path::new(&group.owner), removed_path) {
            continue;
        }
        let mut owner = None;
        for file in &group.files {
            match fs::symlink_metadata(&file.path) {
                Ok(metadata)
                    if current_platform().matches_file_identity(&metadata, group.identity) =>
                {
                    owner = Some(file.clone());
                    break;
                }
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(format!(
                        "failed to reconcile shared allocation at {}: {error}",
                        crate::filesystem::metadata::diagnostic_path(Path::new(&file.path))
                    ))
                }
            }
        }
        if let Some(mut owner) = owner {
            owner.bytes = group.bytes;
            group.owner = owner.path.clone();
            for file in &mut group.files {
                file.bytes = if file.path == owner.path {
                    group.bytes
                } else {
                    0
                };
            }
            credits.push(owner);
        } else {
            group.files.clear();
        }
    }
    allocations.retain(|group| !group.files.is_empty());
    Ok(credits)
}

/// The source result removes one direct child. Its scan ID stays authoritative
/// so a second delete can use the reconciled snapshot immediately.
pub(super) fn remove_direct_entry(result: &mut AnalysisResult, removed_path: &Path) {
    let displayed_bytes = result
        .entries
        .iter()
        .find(|entry| current_platform().paths_equal(Path::new(&entry.path), removed_path))
        .map_or(0, |entry| entry.bytes);
    result
        .entries
        .retain(|entry| !current_platform().paths_equal(Path::new(&entry.path), removed_path));
    result.total_bytes = result.total_bytes.saturating_sub(displayed_bytes);
    if displayed_bytes > 0 {
        result.total_entry_count = result.total_entry_count.saturating_sub(1);
    }
    result
        .directory_hierarchy
        .retain(|node| !current_platform().paths_equal(Path::new(&node.path), removed_path));
    Arc::make_mut(&mut result.shared_directories).retain(|node| {
        !current_platform().path_is_same_or_child(Path::new(&node.path), removed_path)
    });
    Arc::make_mut(&mut result.shared_entries)
        .retain(|entry| !current_platform().paths_equal(Path::new(&entry.path), removed_path));
}

pub(super) fn apply_allocation_credits(
    result: &mut AnalysisResult,
    credits: &[DirectoryEntryInfo],
) {
    for file in credits {
        if !contains(&result.root, &file.path) || file.bytes == 0 {
            continue;
        }
        result.total_bytes = result.total_bytes.saturating_add(file.bytes);
        if let Some(entry) = Arc::make_mut(&mut result.shared_entries)
            .iter_mut()
            .find(|entry| contains(&entry.path, &file.path))
        {
            if entry.bytes == 0 {
                result.total_entry_count += 1;
            }
            entry.bytes = entry.bytes.saturating_add(file.bytes);
            if entry.is_directory {
                entry.content_fingerprint = None;
            }
            if let Some(visible) = result
                .entries
                .iter_mut()
                .find(|visible| visible.path == entry.path)
            {
                *visible = entry.clone();
            } else {
                // Newly charged aliases can outrank formerly visible rows.
                result.entries.push(entry.clone());
            }
        }
        let directories = Arc::make_mut(&mut result.shared_directories);
        let newly_positive: Vec<_> = directories
            .iter()
            .filter(|node| node.bytes == 0 && contains(&node.path, &file.path))
            .map(|node| node.path.clone())
            .collect();
        for node in directories {
            if !contains(&node.path, &file.path) {
                continue;
            }
            node.bytes = node.bytes.saturating_add(file.bytes);
            node.total_entry_count +=
                u64::from(Path::new(&file.path).parent() == Some(Path::new(&node.path)));
            node.total_entry_count += newly_positive
                .iter()
                .filter(|path| Path::new(path).parent() == Some(Path::new(&node.path)))
                .count() as u64;
        }
    }
    result.entries.sort_unstable_by(|left, right| {
        right
            .bytes
            .cmp(&left.bytes)
            .then_with(|| left.path.cmp(&right.path))
    });
    result.entries.truncate(super::ANALYSIS_VISIBLE_ENTRY_LIMIT);
    if !credits.is_empty() {
        rebuild_hierarchy(result);
    }
}

/// Reuse the scan projection's budgets and ordering. Retained shared ancestors
/// fill missing branches without traversing the filesystem or copying its index.
fn rebuild_hierarchy(result: &mut AnalysisResult) {
    fn retain(
        nodes: &[AnalysisDirectoryNode],
        directories: &mut HashMap<PathBuf, DirectoryAggregate>,
        files: &mut HashMap<PathBuf, IndexedFile>,
        counts: &mut HashMap<String, u64>,
    ) {
        for node in nodes {
            directories.insert(
                PathBuf::from(&node.path),
                DirectoryAggregate {
                    bytes: node.bytes,
                    file_count: node.file_count,
                    ..Default::default()
                },
            );
            counts.insert(node.path.clone(), node.total_entry_count);
            for file in &node.files {
                retain_file(file, files);
            }
            retain(&node.children, directories, files, counts);
        }
    }
    fn retain_file(file: &DirectoryEntryInfo, files: &mut HashMap<PathBuf, IndexedFile>) {
        files.insert(
            PathBuf::from(&file.path),
            IndexedFile {
                shared_identity: None,
                bytes: file.bytes,
                logical_bytes: file.logical_bytes,
                modified_at_ms: file.modified_at_ms,
            },
        );
    }
    fn apply_counts(nodes: &mut [AnalysisDirectoryNode], counts: &HashMap<String, u64>) {
        for node in nodes {
            node.total_entry_count = counts.get(&node.path).copied().unwrap_or(0);
            apply_counts(&mut node.children, counts);
        }
    }
    let mut directories = HashMap::new();
    let mut files = HashMap::new();
    let mut counts = HashMap::new();
    retain(
        &result.directory_hierarchy,
        &mut directories,
        &mut files,
        &mut counts,
    );
    retain(
        &result.shared_directories,
        &mut directories,
        &mut files,
        &mut counts,
    );
    for file in result
        .shared_allocations
        .iter()
        .flat_map(|group| &group.files)
    {
        retain_file(file, &mut files);
    }
    directories.insert(
        PathBuf::from(&result.root),
        DirectoryAggregate {
            bytes: result.total_bytes,
            ..Default::default()
        },
    );
    result.directory_hierarchy =
        cache::build_directory_hierarchy(Path::new(&result.root), &directories, &files);
    apply_counts(&mut result.directory_hierarchy, &counts);
}
