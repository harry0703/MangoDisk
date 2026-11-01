use std::{
    collections::{HashMap, HashSet},
    ffi::OsString,
    path::{Path, PathBuf},
};

use super::cache::{DirectoryAggregate, IndexedFile};

/// Derived sibling names avoid walking the whole scan on each navigation. Names
/// share their parent's path and never extend the authoritative filesystem facts.
#[derive(Default)]
pub(in crate::storage) struct NavigationIndex {
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

    pub(in crate::storage) fn directory_count(
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

    /// A leaf has no descendant aliases, so its own retained names suffice to
    /// select shared identities without inspecting unrelated scan-wide rows.
    pub(in crate::storage) fn leaf_shared_identities(
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

    pub(in crate::storage) fn directory_names(&self, parent: &Path) -> Option<&[OsString]> {
        self.directories.get(parent).map(Vec::as_slice)
    }
    pub(in crate::storage) fn file_names(&self, parent: &Path) -> Option<&[OsString]> {
        self.files.get(parent).map(Vec::as_slice)
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
                serde_json::to_value(
                    crate::storage::analysis::hierarchy::hierarchy_from_navigation(
                        index,
                        root,
                        directories,
                        files
                    )
                    .unwrap()
                )
                .unwrap(),
                serde_json::to_value(
                    crate::storage::analysis::hierarchy::build_directory_hierarchy(
                        root,
                        directories,
                        files
                    )
                )
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
        assert!(index
            .directory_names(&removed_branch)
            .unwrap_or_default()
            .is_empty());
        assert!(index
            .file_names(&removed_branch)
            .unwrap_or_default()
            .is_empty());
        assert_eq!(index.directory_count(root, &directories), 89);
    }
    #[test]
    fn inconsistent_navigation_rejects_projection_instead_of_losing_a_child() {
        let root = Path::new("/fixture");
        let directories = HashMap::from([(
            root.into(),
            DirectoryAggregate {
                bytes: 4096,
                ..Default::default()
            },
        )]);
        let mut files = HashMap::from([(
            root.join("file"),
            IndexedFile {
                shared_identity: None,
                bytes: 4096,
                logical_bytes: 4096,
                modified_at_ms: None,
            },
        )]);
        let navigation = NavigationIndex::build(&directories, &files);
        files.clear();
        assert!(
            crate::storage::analysis::hierarchy::hierarchy_from_navigation(
                &navigation,
                root,
                &directories,
                &files
            )
            .is_err()
        );
    }
}
