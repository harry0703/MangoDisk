use std::{
    cmp::Reverse,
    collections::{hash_map::Entry, BinaryHeap, HashMap},
    path::PathBuf,
};

use mangodisk_platform::{
    current_platform, FastAnalysisFile, FilesystemChangeToken, PhysicalFileIdentity, Platform,
    ScanPurpose,
};

use crate::storage::index::cache::{DirectoryAggregate, IndexedFile};

pub(super) const ANALYSIS_FILES_PER_DIRECTORY: usize = 1000;
const ANALYSIS_FILE_BUDGET: usize = ANALYSIS_FILES_PER_DIRECTORY * 128;

/// Collects one completed scan in memory.
///
/// Scan results are rebuildable, session-scoped data. Keeping a single authoritative result avoids
/// duplicating millions of derived records before the UI can render them. The bounded cache that
/// owns the completed sink decides when an older root is released.
pub(super) struct IndexRecordSink {
    directories: HashMap<PathBuf, DirectoryAggregate>,
    files: HashMap<PathBuf, IndexedFile>,
    hard_links: HashMap<(u64, u64), Vec<(PathBuf, IndexedFile)>>,
    change_token: Option<FilesystemChangeToken>,
    analysis_files: AnalysisCandidates,
}

pub(super) struct CompletedIndexSink {
    pub(super) directories: HashMap<PathBuf, DirectoryAggregate>,
    pub(super) files: HashMap<PathBuf, IndexedFile>,
    pub(super) change_token: Option<FilesystemChangeToken>,
}

/// Keep a bounded allocation-ranked subset, with deterministic path ties.
/// The scan-wide budget caps extra retained metadata independently of file count.
pub(super) struct AnalysisCandidates {
    files: BinaryHeap<Reverse<(u64, FastAnalysisFile)>>,
    limit: usize,
}
impl AnalysisCandidates {
    pub(super) fn new(limit: usize) -> Self {
        Self {
            files: BinaryHeap::new(),
            limit,
        }
    }
    pub(super) fn would_retain(&self, bytes: u64, path: &std::path::Path) -> bool {
        bytes > 0
            && self.limit > 0
            && (self.files.len() < self.limit
                || self.files.peek().is_some_and(|Reverse((_, smallest))| {
                    bytes > smallest.allocated_bytes
                        || (bytes == smallest.allocated_bytes
                            && FastAnalysisFile::compare_paths(path, &smallest.path).is_lt())
                }))
    }
    pub(super) fn push(&mut self, file: FastAnalysisFile) {
        if file.allocated_bytes == 0 || self.limit == 0 {
            return;
        }
        // Prefer expensive directories before ranking their file rows. A global
        // size-only budget would evict every row of a huge small-file directory.
        let candidate = (file.parent_file_count, file);
        if self.files.len() < self.limit {
            self.files.push(Reverse(candidate));
        } else if self
            .files
            .peek()
            .is_some_and(|smallest| candidate > smallest.0)
        {
            *self
                .files
                .peek_mut()
                .expect("a full candidate heap is nonempty") = Reverse(candidate);
        }
    }
    pub(super) fn into_files(self) -> impl Iterator<Item = FastAnalysisFile> {
        self.files.into_iter().map(|Reverse((_, file))| file)
    }
}

impl IndexRecordSink {
    pub(super) fn memory(change_token: Option<FilesystemChangeToken>) -> Self {
        Self {
            directories: HashMap::new(),
            files: HashMap::new(),
            hard_links: HashMap::new(),
            change_token,
            analysis_files: AnalysisCandidates::new(ANALYSIS_FILE_BUDGET),
        }
    }

    #[cfg(any(windows, test))]
    pub(super) fn merge_directory_records(&mut self, partial: Self) -> Result<(), String> {
        for (path, aggregate) in partial.directories {
            self.push_directory(path, aggregate)?;
        }
        for (path, file) in partial.files {
            self.push_large_file(path, file)?;
        }
        for (identity, links) in partial.hard_links {
            self.hard_links.entry(identity).or_default().extend(links);
        }
        for file in partial.analysis_files.into_files() {
            self.push_analysis_file(file);
        }
        Ok(())
    }

    pub(super) fn push_analysis_file(&mut self, file: FastAnalysisFile) {
        self.analysis_files.push(file);
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

    pub(super) fn push_hard_link(
        &mut self,
        path: PathBuf,
        identity: PhysicalFileIdentity,
        mut file: IndexedFile,
    ) {
        file.shared_identity = Some(identity);
        self.hard_links
            .entry((identity.volume, identity.index))
            .or_default()
            .push((path, file));
    }

    /// Assign shared allocation to a stable path within this scan, regardless of worker ordering.
    /// Only multiply linked files are retained; ordinary files add no identity-map overhead.
    pub(super) fn finish_analysis(mut self) -> Result<CompletedIndexSink, String> {
        // Ownership is resolved after native enumeration. Bound its per-directory
        // candidate buffers too, prioritizing large branches when the budget is full.
        let parent_limit = ANALYSIS_FILE_BUDGET / ANALYSIS_FILES_PER_DIRECTORY;
        let mut parents = BinaryHeap::new();
        for (path, directory) in &self.directories {
            if directory.direct_file_count == 0 {
                continue;
            }
            let rank = (directory.bytes, path.as_path());
            if parents.len() < parent_limit {
                parents.push(Reverse(rank));
            } else if parents.peek().is_some_and(|smallest| rank > smallest.0) {
                *parents.peek_mut().expect("a full parent heap is nonempty") = Reverse(rank);
            }
        }
        let mut owner_files: HashMap<PathBuf, AnalysisCandidates> = parents
            .into_iter()
            .map(|Reverse((_, path))| {
                (
                    path.to_path_buf(),
                    AnalysisCandidates::new(ANALYSIS_FILES_PER_DIRECTORY),
                )
            })
            .collect();
        for mut links in self.hard_links.into_values() {
            links.sort_unstable_by(|left, right| left.0.cmp(&right.0));
            // Select charged owners only after all aliases are known. Ordinary files
            // have already been bounded by their native directory reader.
            if let Some((path, file)) = links.first() {
                // Retain owners too, even below the candidate floor or when the
                // other links are outside this scan. Cached child sessions must
                // preserve the same shared-allocation deletion boundary.
                self.files.insert(path.clone(), *file);
                if let Some(candidates) =
                    path.parent().and_then(|parent| owner_files.get_mut(parent))
                {
                    if file.bytes > 0
                        && candidates.would_retain(file.bytes, path)
                        && current_platform()
                            .should_skip(path, path.parent().unwrap_or(path), ScanPurpose::Analysis)
                            .is_none()
                    {
                        candidates.push(FastAnalysisFile {
                            parent_file_count: 0,
                            path: path.clone(),
                            allocated_bytes: file.bytes,
                            logical_bytes: file.logical_bytes,
                            modified_at_ms: file.modified_at_ms,
                        });
                    }
                }
            }
            for (path, mut file) in links.into_iter().skip(1) {
                for ancestor in path.ancestors().skip(1) {
                    if let Some(directory) = self.directories.get_mut(ancestor) {
                        directory.bytes =
                            directory.bytes.checked_sub(file.bytes).ok_or_else(|| {
                                "hard-link allocation exceeds the containing directory".to_string()
                            })?;
                        // Removing a link can change allocation ownership in another subtree.
                        directory.fingerprint = None;
                    }
                }
                if file.bytes > 0 {
                    if let Some(parent) = path
                        .parent()
                        .and_then(|parent| self.directories.get_mut(parent))
                    {
                        parent.direct_file_count =
                            parent.direct_file_count.checked_sub(1).ok_or_else(|| {
                                "hard-link count exceeds the containing directory".to_string()
                            })?;
                    }
                }
                file.bytes = 0;
                // Keep zero-charge aliases even below the candidate floor so live row assembly
                // cannot reintroduce their full allocation through its metadata fallback.
                self.files.insert(path, file);
            }
        }
        for candidates in owner_files.into_values() {
            for file in candidates.into_files() {
                self.analysis_files.push(file);
            }
        }
        let mut candidates = self.analysis_files.into_files().collect::<Vec<_>>();
        candidates.sort_unstable_by(|left, right| right.cmp(left));
        let mut parent_counts = HashMap::new();
        for file in candidates {
            if self
                .files
                .get(&file.path)
                .is_some_and(|indexed| indexed.bytes == 0)
            {
                continue;
            }
            let Some(parent) = file.path.parent() else {
                continue;
            };
            let count = parent_counts.entry(parent.to_path_buf()).or_insert(0);
            if *count >= ANALYSIS_FILES_PER_DIRECTORY {
                continue;
            }
            *count += 1;
            // Zero-charge aliases remain authoritative even if a candidate was stale.
            self.files.entry(file.path).or_insert(IndexedFile {
                shared_identity: None,
                bytes: file.allocated_bytes,
                logical_bytes: file.logical_bytes,
                modified_at_ms: file.modified_at_ms,
            });
        }
        Ok(CompletedIndexSink {
            directories: self.directories,
            files: self.files,
            change_token: self.change_token,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn global_retention_preserves_heavy_small_file_directories() {
        let mut candidates = AnalysisCandidates::new(5);
        for index in 0..10 {
            candidates.push(FastAnalysisFile {
                parent_file_count: 10,
                path: format!("/large/{index}").into(),
                allocated_bytes: 1_000_000,
                logical_bytes: 1_000_000,
                modified_at_ms: None,
            });
        }
        for index in 0..3 {
            candidates.push(FastAnalysisFile {
                parent_file_count: 223_905,
                path: format!("/small/{index}").into(),
                allocated_bytes: 4096,
                logical_bytes: 4096,
                modified_at_ms: None,
            });
        }
        let retained: Vec<_> = candidates.into_files().collect();
        assert_eq!(retained.len(), 5);
        assert_eq!(
            retained
                .iter()
                .filter(|file| file.path.starts_with("/small"))
                .count(),
            3
        );
    }
    #[test]
    fn supplemental_analysis_files_stay_bounded_and_never_replace_zero_charge_aliases() {
        let mut sink = IndexRecordSink::memory(None);
        let candidate_count = ANALYSIS_FILE_BUDGET + ANALYSIS_FILES_PER_DIRECTORY;
        for index in (0..candidate_count).rev() {
            sink.push_analysis_file(FastAnalysisFile {
                parent_file_count: 0,
                path: format!(
                    "/fixture/parent-{}/file-{index:05}",
                    index / ANALYSIS_FILES_PER_DIRECTORY
                )
                .into(),
                allocated_bytes: (index + 1) as u64,
                logical_bytes: (index + 1) as u64,
                modified_at_ms: None,
            });
        }
        assert_eq!(sink.analysis_files.files.len(), ANALYSIS_FILE_BUDGET);
        let last_index = candidate_count - 1;
        let alias = PathBuf::from(format!(
            "/fixture/parent-{}/file-{last_index:05}",
            last_index / ANALYSIS_FILES_PER_DIRECTORY
        ));
        sink.push_large_file(
            alias.clone(),
            IndexedFile {
                shared_identity: None,
                bytes: 0,
                logical_bytes: candidate_count as u64,
                modified_at_ms: None,
            },
        )
        .unwrap();
        let snapshot = sink.finish_analysis().unwrap();
        assert_eq!(snapshot.files.len(), ANALYSIS_FILE_BUDGET);
        assert_eq!(snapshot.files[&alias].bytes, 0);
        assert!(!snapshot
            .files
            .contains_key(std::path::Path::new("/fixture/parent-0/file-00000")));
    }
}
