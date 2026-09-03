use std::{
    collections::{hash_map::Entry, HashMap},
    path::PathBuf,
};

use mangodisk_platform::FilesystemChangeToken;

use crate::storage::index::cache::{DirectoryAggregate, IndexedFile};

/// Collects one completed scan in memory.
///
/// Scan results are rebuildable, session-scoped data. Keeping a single authoritative result avoids
/// duplicating millions of derived records before the UI can render them. The bounded cache that
/// owns the completed sink decides when an older root is released.
pub(super) struct IndexRecordSink {
    directories: HashMap<PathBuf, DirectoryAggregate>,
    files: HashMap<PathBuf, IndexedFile>,
    change_token: Option<FilesystemChangeToken>,
}

pub(super) struct CompletedIndexSink {
    pub(super) directories: HashMap<PathBuf, DirectoryAggregate>,
    pub(super) files: HashMap<PathBuf, IndexedFile>,
    pub(super) change_token: Option<FilesystemChangeToken>,
}

impl IndexRecordSink {
    pub(super) fn memory(change_token: Option<FilesystemChangeToken>) -> Self {
        Self {
            directories: HashMap::new(),
            files: HashMap::new(),
            change_token,
        }
    }

    pub(super) fn push_directory(
        &mut self,
        path: PathBuf,
        aggregate: DirectoryAggregate,
    ) -> Result<(), String> {
        if self.directories.insert(path, aggregate).is_some() {
            return Err("the in-memory index received a duplicate directory record".to_string());
        }
        Ok(())
    }

    pub(super) fn push_large_file(
        &mut self,
        path: PathBuf,
        file: IndexedFile,
    ) -> Result<(), String> {
        if self.files.insert(path, file).is_some() {
            return Err("the in-memory index received a duplicate large-file record".to_string());
        }
        Ok(())
    }

    /// Inserts a validated record from an advisory candidate source.
    ///
    /// Native filesystem indexes may repeat a path, so candidate ingestion is idempotent while
    /// authoritative traversal streams keep their strict duplicate checks.
    pub(super) fn insert_large_file_candidate(&mut self, path: PathBuf, file: IndexedFile) -> bool {
        match self.files.entry(path) {
            Entry::Vacant(entry) => {
                entry.insert(file);
                true
            }
            Entry::Occupied(_) => false,
        }
    }

    pub(super) fn finish(self) -> Result<CompletedIndexSink, String> {
        Ok(CompletedIndexSink {
            directories: self.directories,
            files: self.files,
            change_token: self.change_token,
        })
    }
}
