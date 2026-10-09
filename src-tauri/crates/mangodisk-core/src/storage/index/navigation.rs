use std::{
    collections::{HashMap, HashSet},
    ffi::OsString,
    path::{Path, PathBuf},
};

use super::cache::{DirectoryAggregate, IndexedFile};
use crate::storage::analysis::AnalysisDirectoryNode;

/// Derived sibling names avoid walking the whole scan on each navigation. Names
/// share their parent's path and never extend the authoritative filesystem facts.
#[derive(Default)]
pub(super) struct NavigationIndex {
    directories: HashMap<PathBuf, Vec<OsString>>,
    files: HashMap<PathBuf, Vec<OsString>>,
}

impl NavigationIndex {
    pub(super) fn build(
        directories: &HashMap<PathBuf, DirectoryAggregate>,
        files: &HashMap<PathBuf, IndexedFile>,
    ) -> Self {
        let mut index = Self::default();
        for path in directories.keys() {
            Self::insert(&mut index.directories, path);
        }
        for path in files.keys() {
            Self::insert(&mut index.files, path);
        }
        index
    }

    fn insert(index: &mut HashMap<PathBuf, Vec<OsString>>, path: &Path) {
        if let (Some(parent), Some(name)) = (path.parent(), path.file_name()) {
            index.entry(parent.into()).or_default().push(name.into());
        }
    }

    pub(super) fn remove(&mut self, target: &Path, is_directory: bool) {
        if is_directory {
            self.directories
                .retain(|parent, _| !parent.starts_with(target));
            self.files.retain(|parent, _| !parent.starts_with(target));
        }
        let entries = if is_directory {
            &mut self.directories
        } else {
            &mut self.files
        };
        if let Some(names) = target.parent().and_then(|parent| entries.get_mut(parent)) {
            names.retain(|name| Some(name.as_os_str()) != target.file_name());
        }
    }

    pub(super) fn directory_count(
        &self,
        parent: &Path,
        directories: &HashMap<PathBuf, DirectoryAggregate>,
    ) -> u64 {
        self.directories
            .get(parent)
            .into_iter()
            .flatten()
            .filter(|name| {
                directories
                    .get(&parent.join(name))
                    .is_some_and(|directory| directory.bytes > 0)
            })
            .count() as u64
    }

    pub(super) fn direct_snapshot(
        &self,
        root: &Path,
        directories: &HashMap<PathBuf, DirectoryAggregate>,
        files: &HashMap<PathBuf, IndexedFile>,
    ) -> (
        HashMap<PathBuf, DirectoryAggregate>,
        HashMap<PathBuf, IndexedFile>,
    ) {
        let direct_directories = self
            .directories
            .get(root)
            .into_iter()
            .flatten()
            .filter_map(|name| {
                let path = root.join(name);
                directories.get(&path).map(|value| (path, *value))
            })
            .collect();
        let direct_files = self
            .files
            .get(root)
            .into_iter()
            .flatten()
            .filter_map(|name| {
                let path = root.join(name);
                files.get(&path).map(|value| (path, *value))
            })
            .collect();
        (direct_directories, direct_files)
    }

    /// A leaf has no descendant aliases, so its own retained names suffice to
    /// select shared identities without inspecting unrelated scan-wide rows.
    pub(super) fn leaf_shared_identities(
        &self,
        root: &Path,
        files: &HashMap<PathBuf, IndexedFile>,
    ) -> Option<HashSet<(u64, u64)>> {
        if self
            .directories
            .get(root)
            .is_some_and(|names| !names.is_empty())
        {
            return None;
        }
        Some(
            self.files
                .get(root)
                .into_iter()
                .flatten()
                .filter_map(|name| {
                    files
                        .get(&root.join(name))?
                        .shared_identity
                        .map(|identity| (identity.volume, identity.index))
                })
                .collect(),
        )
    }

    pub(super) fn hierarchy(
        &self,
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
                if depth > super::cache::HIERARCHY_MAX_DEPTH || self.remaining == 0 {
                    return;
                }
                let mut siblings = Vec::new();
                for name in self.index.directories.get(parent).into_iter().flatten() {
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
                    for name in self.index.files.get(parent).into_iter().flatten() {
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
                siblings.truncate(super::cache::HIERARCHY_MAX_CHILDREN);
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
            index: self,
            directories,
            files,
            minimum_bytes: (aggregate.bytes / 2000).max(1),
            remaining: super::cache::HIERARCHY_MAX_NODES,
            selected_directories: HashMap::from([(root.into(), *aggregate)]),
            selected_files: HashMap::new(),
        };
        projection.visit(root, 1);
        let mut nodes = super::cache::build_directory_hierarchy(
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
                    directory.direct_file_count + self.directory_count(path, directories),
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leaf_identity_selection_preserves_zero_aliases_and_rejects_nested_directories() {
        let root = Path::new("/fixture");
        let leaf = root.join("leaf");
        let identity = mangodisk_platform::PhysicalFileIdentity {
            volume: 1,
            index: 2,
        };
        let mut directories = HashMap::from([
            (root.into(), DirectoryAggregate::default()),
            (leaf.clone(), DirectoryAggregate::default()),
        ]);
        let mut files = HashMap::from([
            (
                leaf.join("alias"),
                IndexedFile {
                    shared_identity: Some(identity),
                    bytes: 0,
                    logical_bytes: 4096,
                    modified_at_ms: None,
                },
            ),
            (
                root.join("unrelated"),
                IndexedFile {
                    shared_identity: Some(mangodisk_platform::PhysicalFileIdentity {
                        volume: 1,
                        index: 3,
                    }),
                    bytes: 4096,
                    logical_bytes: 4096,
                    modified_at_ms: None,
                },
            ),
        ]);
        let mut index = NavigationIndex::build(&directories, &files);
        assert_eq!(
            index.leaf_shared_identities(&leaf, &files),
            Some(HashSet::from([(1, 2)]))
        );
        assert!(index.leaf_shared_identities(root, &files).is_none());
        let nested = leaf.join("empty-child");
        directories.insert(nested.clone(), DirectoryAggregate::default());
        index = NavigationIndex::build(&directories, &files);
        assert!(index.leaf_shared_identities(&leaf, &files).is_none());
        directories.remove(&nested);
        index.remove(&nested, true);
        files.remove(&leaf.join("alias"));
        index.remove(&leaf.join("alias"), false);
        assert_eq!(
            index.leaf_shared_identities(&leaf, &files),
            Some(HashSet::new())
        );
    }

    #[test]
    fn sibling_projection_matches_full_index_limits_counts_and_deletions() {
        #[cfg(windows)]
        let root = Path::new(r"\\?\C:\fixture");
        #[cfg(not(windows))]
        let root = Path::new("/fixture");
        let aggregate = DirectoryAggregate {
            bytes: 2_000_000,
            direct_file_count: 3,
            ..Default::default()
        };
        let mut directories = HashMap::from([(root.into(), aggregate)]);
        let mut files = HashMap::new();
        for parent in 0..90 {
            let branch = root.join(format!("\u{76ee}\u{5f55}-{parent:03}"));
            directories.insert(branch.clone(), aggregate);
            for child in 0..75 {
                let leaf = branch.join(format!("child-{child:03}"));
                directories.insert(
                    leaf.clone(),
                    DirectoryAggregate {
                        bytes: if child % 7 == 0 { 1 } else { 10_000 },
                        ..aggregate
                    },
                );
                let mut path = leaf;
                for _ in 0..8 {
                    path = path.join("deep");
                    directories.insert(path.clone(), aggregate);
                }
            }
            for file in 0..90 {
                files.insert(
                    branch.join(format!("file-{file:03}")),
                    IndexedFile {
                        shared_identity: None,
                        bytes: if file % 3 == 0 { 0 } else { 10_000 },
                        logical_bytes: 10_000,
                        modified_at_ms: Some(1),
                    },
                );
            }
        }
        let mut index = NavigationIndex::build(&directories, &files);
        let compare = |index: &NavigationIndex,
                       directories: &HashMap<PathBuf, DirectoryAggregate>,
                       files: &HashMap<PathBuf, IndexedFile>| {
            assert_eq!(
                serde_json::to_value(index.hierarchy(root, directories, files)).unwrap(),
                serde_json::to_value(super::super::cache::build_directory_hierarchy(
                    root,
                    directories,
                    files
                ))
                .unwrap()
            );
        };
        compare(&index, &directories, &files);
        let removed_file = root.join("\u{76ee}\u{5f55}-000/file-001");
        files.remove(&removed_file);
        index.remove(&removed_file, false);
        let removed_branch = root.join("\u{76ee}\u{5f55}-001");
        directories.retain(|path, _| !path.starts_with(&removed_branch));
        files.retain(|path, _| !path.starts_with(&removed_branch));
        index.remove(&removed_branch, true);
        compare(&index, &directories, &files);
        let (direct_directories, direct_files) =
            index.direct_snapshot(&removed_branch, &directories, &files);
        assert!(direct_directories.is_empty() && direct_files.is_empty());
        assert_eq!(index.directory_count(root, &directories), 89);
    }
}
