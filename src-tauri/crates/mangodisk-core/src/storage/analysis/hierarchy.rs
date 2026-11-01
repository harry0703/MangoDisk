use std::{
    borrow::Cow,
    collections::HashMap,
    convert::Infallible,
    path::{Path, PathBuf},
};

use super::{
    query::{AnalysisQuery, ChildMeasurement, KnownChild},
    AnalysisDirectoryNode, DirectoryEntryInfo,
};
use crate::{
    filesystem::metadata::display_path,
    storage::index::cache::{DirectoryAggregate, IndexedFile},
    CoreResult,
};

pub(super) const HIERARCHY_MAX_DEPTH: usize = 6;
const HIERARCHY_MAX_CHILDREN: usize = 64;
const HIERARCHY_MAX_NODES: usize = 2048;

/// One projection algorithm serves both private completed scans and the published index.
/// Providers own child lookup; chart depth, sibling and node budgets stay independent of retention.
fn project<'a, E>(
    parent: &Path,
    depth: usize,
    remaining: &mut usize,
    read: &mut impl FnMut(&Path, usize) -> Result<Vec<KnownChild<'a>>, E>,
    count: &mut impl FnMut(&Path, DirectoryAggregate) -> u64,
) -> Result<(Vec<AnalysisDirectoryNode>, Vec<DirectoryEntryInfo>), E> {
    if depth > HIERARCHY_MAX_DEPTH || *remaining == 0 {
        return Ok((Vec::new(), Vec::new()));
    }
    let mut siblings = read(parent, depth)?;
    let compare = |left: &KnownChild<'_>, right: &KnownChild<'_>| {
        right
            .bytes()
            .cmp(&left.bytes())
            .then_with(|| left.path.cmp(&right.path))
    };
    if siblings.len() > HIERARCHY_MAX_CHILDREN {
        siblings.select_nth_unstable_by(HIERARCHY_MAX_CHILDREN, compare);
        siblings.truncate(HIERARCHY_MAX_CHILDREN);
    }
    siblings.sort_unstable_by(compare);
    let mut directories = Vec::new();
    let mut files = Vec::new();
    for child in siblings {
        if *remaining == 0 {
            break;
        }
        *remaining -= 1;
        match child.measurement {
            ChildMeasurement::Directory(aggregate) => {
                let total_entry_count = count(&child.path, *aggregate);
                let (children, files) = project(&child.path, depth + 1, remaining, read, count)?;
                directories.push(AnalysisDirectoryNode {
                    name: child
                        .path
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                    path: display_path(&child.path),
                    bytes: aggregate.bytes,
                    file_count: aggregate.file_count,
                    total_entry_count,
                    children,
                    files,
                });
            }
            ChildMeasurement::File(_) => files.push(child.entry()),
        }
    }
    Ok((directories, files))
}

pub(crate) fn build_directory_hierarchy(
    root: &Path,
    directories: &HashMap<PathBuf, DirectoryAggregate>,
    files: &HashMap<PathBuf, IndexedFile>,
) -> Vec<AnalysisDirectoryNode> {
    let minimum_bytes = directories
        .get(root)
        .map_or(1, |aggregate| (aggregate.bytes / 2000).max(1));
    let mut children: HashMap<&Path, Vec<KnownChild<'_>>> = HashMap::new();
    // Private scans have no navigation index yet. Group only visible-depth candidates once,
    // rather than walking every index record again for each projected directory.
    for (path, value) in directories {
        if value.bytes < minimum_bytes {
            continue;
        }
        let Ok(relative) = path.strip_prefix(root) else {
            continue;
        };
        let depth = relative.components().take(HIERARCHY_MAX_DEPTH + 1).count();
        if depth == 0 || depth > HIERARCHY_MAX_DEPTH {
            continue;
        }
        if let Some(parent) = path.parent() {
            children.entry(parent).or_default().push(KnownChild {
                path: Cow::Borrowed(path),
                measurement: ChildMeasurement::Directory(value),
            });
        }
    }
    for (path, value) in files {
        if value.bytes < minimum_bytes {
            continue;
        }
        let Ok(relative) = path.strip_prefix(root) else {
            continue;
        };
        let depth = relative.components().take(HIERARCHY_MAX_DEPTH + 1).count();
        if !(2..=HIERARCHY_MAX_DEPTH).contains(&depth) {
            continue;
        }
        if let Some(parent) = path.parent() {
            children.entry(parent).or_default().push(KnownChild {
                path: Cow::Borrowed(path),
                measurement: ChildMeasurement::File(value),
            });
        }
    }
    let mut selected_paths = Vec::new();
    let mut counts = Vec::new();
    let mut remaining = HIERARCHY_MAX_NODES;
    let result = project::<Infallible>(
        root,
        1,
        &mut remaining,
        &mut |parent, _| Ok(children.remove(parent).unwrap_or_default()),
        &mut |path, aggregate| {
            selected_paths.push(path.to_path_buf());
            counts.push(aggregate.direct_file_count);
            aggregate.direct_file_count
        },
    );
    let mut nodes = match result {
        Ok((nodes, _)) => nodes,
        Err(never) => match never {},
    };
    let positions: HashMap<_, _> = selected_paths
        .iter()
        .enumerate()
        .map(|(index, path)| (path.as_path(), index))
        .collect();
    for (path, aggregate) in directories {
        if aggregate.bytes > 0 {
            if let Some(index) = path.parent().and_then(|parent| positions.get(parent)) {
                counts[*index] += 1;
            }
        }
    }
    fn apply_counts(nodes: &mut [AnalysisDirectoryNode], counts: &mut impl Iterator<Item = u64>) {
        for node in nodes {
            node.total_entry_count = counts
                .next()
                .expect("each projected directory has one counter");
            apply_counts(&mut node.children, counts);
        }
    }
    apply_counts(&mut nodes, &mut counts.into_iter());
    nodes
}

#[cfg(test)]
pub(in crate::storage) fn hierarchy_from_navigation(
    index: &crate::storage::index::navigation::NavigationIndex,
    root: &Path,
    directories: &HashMap<PathBuf, DirectoryAggregate>,
    files: &HashMap<PathBuf, IndexedFile>,
) -> CoreResult<Vec<AnalysisDirectoryNode>> {
    let query = AnalysisQuery::with_navigation(directories, files, index);
    hierarchy_from_query(&query, root)
}

pub(super) fn hierarchy_from_query(
    query: &AnalysisQuery<'_>,
    root: &Path,
) -> CoreResult<Vec<AnalysisDirectoryNode>> {
    let Some(aggregate) = query.directories.get(root) else {
        return Ok(Vec::new());
    };
    let minimum_bytes = (aggregate.bytes / 2000).max(1);
    let mut remaining = HIERARCHY_MAX_NODES;
    project(
        root,
        1,
        &mut remaining,
        &mut |parent, depth| {
            query
                .children(parent)
                .filter_map(|value| match value {
                    Ok(child)
                        if child.bytes() >= minimum_bytes
                            && (depth >= 2
                                || matches!(child.measurement, ChildMeasurement::Directory(_))) =>
                    {
                        Some(Ok(child))
                    }
                    Ok(_) => None,
                    Err(error) => Some(Err(error)),
                })
                .collect::<CoreResult<Vec<_>>>()
        },
        &mut |path, aggregate| aggregate.direct_file_count + query.positive_directory_count(path),
    )
    .map(|(nodes, _)| nodes)
}
