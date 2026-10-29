use super::{AnalysisDirectoryNode, DirectoryEntryInfo};
use crate::{
    filesystem::metadata::display_path,
    storage::index::{
        cache::{DirectoryAggregate, IndexedFile},
        navigation::NavigationIndex,
    },
};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};
pub(super) const HIERARCHY_MAX_DEPTH: usize = 6;
const HIERARCHY_MAX_CHILDREN: usize = 64;
const HIERARCHY_MAX_NODES: usize = 2048;
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
                    size_status: Default::default(),
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

pub(in crate::storage) fn hierarchy_from_navigation(
    index: &NavigationIndex,
    root: &Path,
    directories: &HashMap<PathBuf, DirectoryAggregate>,
    files: &HashMap<PathBuf, IndexedFile>,
) -> Vec<AnalysisDirectoryNode> {
    struct Projection<'a> {
        index: &'a NavigationIndex,
        directories: &'a HashMap<PathBuf, DirectoryAggregate>,
        files: &'a HashMap<PathBuf, IndexedFile>,
        minimum_bytes: u64,
        remaining: usize,
        selected_directories: HashMap<PathBuf, DirectoryAggregate>,
        selected_files: HashMap<PathBuf, IndexedFile>,
    }
    impl Projection<'_> {
        fn visit(&mut self, parent: &Path, depth: usize) {
            if depth > HIERARCHY_MAX_DEPTH || self.remaining == 0 {
                return;
            }
            let mut siblings = Vec::new();
            for name in self.index.directory_names(parent).iter().copied().flatten() {
                let path = parent.join(name);
                if let Some(directory) = self
                    .directories
                    .get(&path)
                    .filter(|directory| directory.bytes >= self.minimum_bytes)
                {
                    siblings.push((path, directory.bytes, true));
                }
            }
            if depth >= 2 {
                for name in self.index.file_names(parent).iter().copied().flatten() {
                    let path = parent.join(name);
                    if let Some(file) = self
                        .files
                        .get(&path)
                        .filter(|file| file.bytes >= self.minimum_bytes)
                    {
                        siblings.push((path, file.bytes, false));
                    }
                }
            }
            siblings.sort_unstable_by(|left, right| {
                right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0))
            });
            siblings.truncate(HIERARCHY_MAX_CHILDREN);
            for (path, _, is_directory) in siblings {
                if self.remaining == 0 {
                    break;
                }
                self.remaining -= 1;
                if is_directory {
                    self.selected_directories
                        .insert(path.clone(), self.directories[&path]);
                    self.visit(&path, depth + 1);
                } else {
                    self.selected_files.insert(path.clone(), self.files[&path]);
                }
            }
        }
    }
    let Some(aggregate) = directories.get(root) else {
        return Vec::new();
    };
    let mut projection = Projection {
        index,
        directories,
        files,
        minimum_bytes: (aggregate.bytes / 2000).max(1),
        remaining: HIERARCHY_MAX_NODES,
        selected_directories: HashMap::from([(root.into(), *aggregate)]),
        selected_files: HashMap::new(),
    };
    projection.visit(root, 1);
    let mut nodes = build_directory_hierarchy(
        root,
        &projection.selected_directories,
        &projection.selected_files,
    );
    // Rendered paths may omit Windows canonicalization prefixes. Derive
    // counters from native keys before crossing that display boundary.
    let counts: HashMap<_, _> = projection
        .selected_directories
        .iter()
        .map(|(path, directory)| {
            (
                crate::filesystem::metadata::display_path(path),
                directory.direct_file_count + index.directory_count(path, directories),
            )
        })
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
