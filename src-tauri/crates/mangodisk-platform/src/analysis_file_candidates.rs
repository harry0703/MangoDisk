use std::{cmp::Reverse, collections::BinaryHeap};

use crate::FastAnalysisFile;

/// Caps supplemental candidates while a directory is enumerated. Native metadata
/// is reused while the caller's limit bounds the largest retained file rows.
#[derive(Debug)]
pub(crate) struct AnalysisFileCandidates {
    files: BinaryHeap<Reverse<FastAnalysisFile>>,
    limit: usize,
}

impl Default for AnalysisFileCandidates {
    fn default() -> Self {
        Self::new(64)
    }
}

impl AnalysisFileCandidates {
    pub(crate) fn new(limit: usize) -> Self {
        Self {
            files: BinaryHeap::new(),
            limit,
        }
    }
    #[cfg(target_os = "macos")]
    pub(crate) fn limit(&self) -> usize {
        self.limit
    }
    pub(crate) fn would_retain(&self, bytes: u64, path: &std::path::Path) -> bool {
        bytes > 0
            && self.limit > 0
            && (self.files.len() < self.limit
                || self.files.peek().is_some_and(|smallest| {
                    bytes > smallest.0.allocated_bytes
                        || (bytes == smallest.0.allocated_bytes
                            && FastAnalysisFile::compare_paths(path, &smallest.0.path).is_lt())
                }))
    }

    pub(crate) fn push(&mut self, file: FastAnalysisFile) {
        if file.allocated_bytes == 0 || self.limit == 0 {
            return;
        }
        if self.files.len() < self.limit {
            self.files.push(Reverse(file));
        } else if self.files.peek().is_some_and(|smallest| file > smallest.0) {
            *self
                .files
                .peek_mut()
                .expect("a full candidate heap is nonempty") = Reverse(file);
        }
    }

    pub(crate) fn into_files(self) -> impl Iterator<Item = FastAnalysisFile> {
        self.files.into_iter().map(|file| file.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    #[test]
    fn ranks_non_utf8_names_by_the_displayed_path() {
        use std::os::unix::ffi::OsStringExt;
        let paths = [
            std::path::PathBuf::from(std::ffi::OsString::from_vec(vec![0x80])),
            std::path::PathBuf::from("é"),
        ];
        for reverse in [false, true] {
            let mut candidates = AnalysisFileCandidates::new(1);
            let mut order = paths.clone();
            if reverse {
                order.reverse();
            }
            for path in order {
                if candidates.would_retain(4096, &path) {
                    candidates.push(FastAnalysisFile {
                        parent_file_count: 2,
                        path,
                        allocated_bytes: 4096,
                        logical_bytes: 4096,
                        modified_at_ms: None,
                    });
                }
            }
            assert_eq!(
                candidates.into_files().next().unwrap().path.to_str(),
                Some("é")
            );
        }
        let colliding = std::path::PathBuf::from(std::ffi::OsString::from_vec(vec![0x81]));
        assert_eq!(paths[0].to_string_lossy(), colliding.to_string_lossy());
        assert_ne!(
            FastAnalysisFile::compare_paths(&paths[0], &colliding),
            std::cmp::Ordering::Equal
        );
    }

    #[test]
    fn retains_largest_files_with_stable_path_ties_and_ignores_zero_allocation() {
        let mut candidates = AnalysisFileCandidates::default();
        for index in (0..1024).rev() {
            candidates.push(FastAnalysisFile {
                parent_file_count: 0,
                path: format!("file-{index:04}").into(),
                allocated_bytes: index / 2,
                logical_bytes: index,
                modified_at_ms: None,
            });
        }
        let mut files = candidates.into_files().collect::<Vec<_>>();
        files.sort_by(|left, right| right.cmp(left));
        assert_eq!(files.len(), 64);
        assert_eq!(files[0].path.to_str(), Some("file-1022"));
        assert_eq!(files[63].path.to_str(), Some("file-0961"));
    }
}
