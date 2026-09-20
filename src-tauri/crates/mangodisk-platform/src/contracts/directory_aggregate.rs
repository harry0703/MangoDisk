use std::path::PathBuf;

#[cfg(not(target_os = "linux"))]
const PROGRESS_ENTRY_BATCH: u64 = 4_096;

/// Aggregates files that share one direct child of a scanned directory.
///
/// Files directly inside the scan root use the root itself as `path`. Deeper
/// files use their first descendant below the root. This filesystem-level
/// grouping lets cleanup and analysis consumers avoid retaining every file
/// path while preserving a stable drill-down boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectoryTreeSourceAggregate {
    pub path: PathBuf,
    pub bytes: u64,
    pub file_count: u64,
    pub modified_at_ms: Option<u64>,
}

/// Complete logical-size measurement of one directory tree.
///
/// Native implementations return this value only after the entire tree has
/// been inspected. A partial aggregate must never be published because callers
/// may use it to construct a cleanup plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectoryTreeAggregate {
    pub bytes: u64,
    pub file_count: u64,
    pub skipped_count: u64,
    pub sources: Vec<DirectoryTreeSourceAggregate>,
    pub strategy: &'static str,
}

/// Physical directories discovered directly below one root.
///
/// Windows application observation needs only this shallow fact set. A native implementation can
/// reuse directory enumeration records to reject reparse points without issuing one metadata call
/// per child. `observed_count` includes files and rejected entries so a caller-provided limit has
/// the same meaning as `ReadDir::take` in the portable implementation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectPhysicalDirectoryEnumeration {
    pub directories: Vec<PathBuf>,
    pub observed_count: usize,
    pub strategy: &'static str,
}

/// Cancellation stays distinct from native traversal failure so callers never
/// restart a slow fallback scan after the user has cancelled the operation.
#[derive(Debug)]
pub enum DirectoryTreeAggregateError {
    Cancelled,
    Platform(String),
}

/// Coalesces hot-path traversal observations before they reach Core's own
/// time-based progress throttle. Keeping the entry batch identical on each
/// platform avoids millions of callbacks while still refreshing long scans.
#[cfg(not(target_os = "linux"))]
pub(crate) struct DirectoryAggregateProgress<'a> {
    callback: &'a (dyn Fn(&std::path::Path, u64, u64) + Sync),
    pending_entries: u64,
    pending_files: u64,
    pending_bytes: u64,
}

#[cfg(not(target_os = "linux"))]
impl<'a> DirectoryAggregateProgress<'a> {
    pub(crate) fn new(callback: &'a (dyn Fn(&std::path::Path, u64, u64) + Sync)) -> Self {
        Self {
            callback,
            pending_entries: 0,
            pending_files: 0,
            pending_bytes: 0,
        }
    }

    /// Records one already-measured traversal batch and publishes it only when
    /// enough directory entries have accumulated. Native implementations keep
    /// their kernel-sized reads intact while Core receives useful live totals
    /// instead of a path-only heartbeat.
    pub(crate) fn observe(
        &mut self,
        path: &std::path::Path,
        entry_count: u64,
        file_count: u64,
        bytes: u64,
    ) {
        self.pending_entries = self.pending_entries.saturating_add(entry_count);
        self.pending_files = self.pending_files.saturating_add(file_count);
        self.pending_bytes = self.pending_bytes.saturating_add(bytes);
        if self.pending_entries >= PROGRESS_ENTRY_BATCH {
            self.flush(path);
        }
    }

    /// Publishes the final partial batch after a successful traversal. Calling
    /// this before returning guarantees that reported totals reconcile exactly
    /// with the completed aggregate.
    pub(crate) fn finish(&mut self, path: &std::path::Path) {
        if self.pending_entries > 0 || self.pending_files > 0 || self.pending_bytes > 0 {
            self.flush(path);
        }
    }

    fn flush(&mut self, path: &std::path::Path) {
        (self.callback)(path, self.pending_files, self.pending_bytes);
        self.pending_entries = 0;
        self.pending_files = 0;
        self.pending_bytes = 0;
    }
}

#[cfg(all(test, not(target_os = "linux")))]
pub(crate) fn reference_directory_tree_aggregate(root: &std::path::Path) -> DirectoryTreeAggregate {
    use std::{collections::BTreeMap, fs, time::UNIX_EPOCH};

    fn visit(
        root: &std::path::Path,
        directory: &std::path::Path,
        sources: &mut BTreeMap<PathBuf, DirectoryTreeSourceAggregate>,
        skipped_count: &mut u64,
    ) {
        let Ok(entries) = fs::read_dir(directory) else {
            *skipped_count = skipped_count.saturating_add(1);
            return;
        };
        for entry in entries {
            let Ok(entry) = entry else {
                *skipped_count = skipped_count.saturating_add(1);
                continue;
            };
            let path = entry.path();
            let Ok(metadata) = fs::symlink_metadata(&path) else {
                *skipped_count = skipped_count.saturating_add(1);
                continue;
            };
            if metadata.file_type().is_symlink() {
                *skipped_count = skipped_count.saturating_add(1);
                continue;
            }
            if metadata.is_dir() {
                visit(root, &path, sources, skipped_count);
                continue;
            }
            if !metadata.is_file() {
                *skipped_count = skipped_count.saturating_add(1);
                continue;
            }
            let source_path = path
                .strip_prefix(root)
                .ok()
                .and_then(|relative| relative.components().next())
                .map(|component| root.join(component.as_os_str()))
                .filter(|candidate| candidate != &path)
                .unwrap_or_else(|| root.to_path_buf());
            let source = sources.entry(source_path.clone()).or_insert_with(|| {
                DirectoryTreeSourceAggregate {
                    path: source_path,
                    bytes: 0,
                    file_count: 0,
                    modified_at_ms: None,
                }
            });
            source.bytes = source.bytes.saturating_add(metadata.len());
            source.file_count = source.file_count.saturating_add(1);
            let modified_at_ms = metadata.modified().ok().and_then(|modified| {
                modified
                    .duration_since(UNIX_EPOCH)
                    .ok()
                    .and_then(|duration| u64::try_from(duration.as_millis()).ok())
            });
            source.modified_at_ms = match (source.modified_at_ms, modified_at_ms) {
                (Some(left), Some(right)) => Some(left.max(right)),
                (Some(value), None) | (None, Some(value)) => Some(value),
                (None, None) => None,
            };
        }
    }

    let mut sources = BTreeMap::new();
    let mut skipped_count = 0;
    visit(root, root, &mut sources, &mut skipped_count);
    let sources = sources.into_values().collect::<Vec<_>>();
    let bytes = sources
        .iter()
        .fold(0_u64, |total, source| total.saturating_add(source.bytes));
    let file_count = sources.iter().fold(0_u64, |total, source| {
        total.saturating_add(source.file_count)
    });
    DirectoryTreeAggregate {
        bytes,
        file_count,
        skipped_count,
        sources,
        strategy: "test-reference-walker",
    }
}

#[cfg(all(test, not(target_os = "linux")))]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    #[test]
    fn progress_is_reported_once_per_bounded_entry_batch() {
        let reports = AtomicU64::new(0);
        let files = AtomicU64::new(0);
        let bytes = AtomicU64::new(0);
        let callback = |_: &std::path::Path, file_count: u64, measured_bytes: u64| {
            reports.fetch_add(1, Ordering::Relaxed);
            files.fetch_add(file_count, Ordering::Relaxed);
            bytes.fetch_add(measured_bytes, Ordering::Relaxed);
        };
        let mut progress = DirectoryAggregateProgress::new(&callback);

        for _ in 0..PROGRESS_ENTRY_BATCH.saturating_mul(2).saturating_sub(1) {
            progress.observe(std::path::Path::new("fixture"), 1, 1, 2);
        }
        assert_eq!(reports.load(Ordering::Relaxed), 1);

        progress.observe(std::path::Path::new("fixture"), 1, 1, 2);
        assert_eq!(reports.load(Ordering::Relaxed), 2);
        assert_eq!(files.load(Ordering::Relaxed), PROGRESS_ENTRY_BATCH * 2);
        assert_eq!(bytes.load(Ordering::Relaxed), PROGRESS_ENTRY_BATCH * 4);
    }

    #[test]
    fn progress_finish_publishes_the_final_partial_batch() {
        let reports = AtomicU64::new(0);
        let files = AtomicU64::new(0);
        let callback = |_: &std::path::Path, file_count: u64, _: u64| {
            reports.fetch_add(1, Ordering::Relaxed);
            files.fetch_add(file_count, Ordering::Relaxed);
        };
        let mut progress = DirectoryAggregateProgress::new(&callback);

        progress.observe(std::path::Path::new("child"), 3, 2, 10);
        assert_eq!(reports.load(Ordering::Relaxed), 0);
        progress.finish(std::path::Path::new("root"));

        assert_eq!(reports.load(Ordering::Relaxed), 1);
        assert_eq!(files.load(Ordering::Relaxed), 2);
    }
}
