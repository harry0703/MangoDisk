//! Cancellable directory traversal and bounded parallel removal.
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    thread,
};

use mangodisk_platform::{current_platform, Platform};

use super::{deletion_entry_io, PermanentDeleteOutcome, RemovalFailure};

// Small directories remain serial because thread startup can cost more than
// their filesystem work. Larger batches use conservative platform caps chosen
// from repeated release benchmarks; higher concurrency caused unstable delete
// latency, especially on macOS and Windows systems with active file indexing.
const PARALLEL_DELETE_ENTRY_THRESHOLD: usize = 512;
const PARALLEL_DELETE_BATCH_SIZE: usize = 8_192;
#[cfg(target_os = "macos")]
const MAX_PARALLEL_DELETE_WORKERS: usize = 2;
#[cfg(windows)]
const MAX_PARALLEL_DELETE_WORKERS: usize = 4;
#[cfg(target_os = "linux")]
const MAX_PARALLEL_DELETE_WORKERS: usize = 4;

/// Removes a tree in place while observing cancellation between directory entries.
///
/// `remove_dir_all` has no cancellation hook, which is unacceptable for the
/// large small-file trees most likely to be cancelled. The traversal uses
/// `symlink_metadata` and the platform link-like policy. Unix symbolic-link
/// entries are unlinked without following their targets. Other link-like
/// entries, including Windows reparse points, stop deletion and leave the
/// remainder in place.
pub(super) fn remove_directory_tree_cancellable(
    root: &Path,
    is_cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<PermanentDeleteOutcome, RemovalFailure> {
    let mut outcome = PermanentDeleteOutcome::default();
    remove_directory_tree_entry(root, is_cancelled, &mut outcome, None).map_err(|error| {
        RemovalFailure {
            error,
            verified_outcome: Some(outcome),
        }
    })?;
    Ok(outcome)
}

pub(super) fn remove_directory_tree_entry(
    path: &Path,
    is_cancelled: &(dyn Fn() -> bool + Sync),
    outcome: &mut PermanentDeleteOutcome,
    names: Option<(&mangodisk_platform::NameExclusions, &Path)>,
) -> Result<(), std::io::Error> {
    if is_cancelled() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Interrupted,
            "directory tree deletion cancelled",
        ));
    }
    let metadata = deletion_entry_io(path, "metadata", fs::symlink_metadata(path))?;
    remove_directory_tree_entry_with_metadata(path, metadata, is_cancelled, outcome, names)
}

fn remove_directory_tree_entry_with_metadata(
    path: &Path,
    metadata: fs::Metadata,
    is_cancelled: &(dyn Fn() -> bool + Sync),
    outcome: &mut PermanentDeleteOutcome,
    names: Option<(&mangodisk_platform::NameExclusions, &Path)>,
) -> Result<(), std::io::Error> {
    if names
        .is_some_and(|(policy, root)| path != root && policy.matches_entry(path, metadata.is_dir()))
    {
        return Err(std::io::Error::other(
            "an excluded name appeared during directory deletion",
        ));
    }
    #[cfg(unix)]
    if metadata.file_type().is_symlink() {
        deletion_entry_io(path, "unlink", fs::remove_file(path))?;
        outcome.had_irreversible_mutation = true;
        return Ok(());
    }
    if current_platform().is_link_like(&metadata) {
        return Err(std::io::Error::other(
            "a link or reparse point appeared during directory tree deletion",
        ));
    }
    if metadata.is_file() {
        let bytes = metadata.len();
        deletion_entry_io(path, "unlink", fs::remove_file(path))?;
        outcome.had_irreversible_mutation = true;
        outcome.released_bytes = outcome.released_bytes.saturating_add(bytes);
        outcome.affected_item_count = outcome.affected_item_count.saturating_add(1);
        return Ok(());
    }
    if !metadata.is_dir() {
        return Err(std::io::Error::other(
            "an unsupported filesystem entry appeared during directory tree deletion",
        ));
    }

    for entry in deletion_entry_io(path, "read_directory", fs::read_dir(path))? {
        if is_cancelled() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Interrupted,
                "directory tree deletion cancelled",
            ));
        }
        let entry = deletion_entry_io(path, "read_entry", entry)?;
        let child = entry.path();
        let metadata = deletion_entry_io(&child, "metadata", entry.metadata())?;
        if metadata.is_dir() {
            // Directory recursion must recheck the live link/reparse type. File
            // unlink never follows its target; Windows enumeration metadata
            // avoids another file metadata query without weakening that boundary.
            remove_directory_tree_entry(&child, is_cancelled, outcome, names)?;
        } else {
            if is_cancelled() {
                return Err(directory_contents_cancelled_error());
            }
            remove_directory_tree_entry_with_metadata(
                &child,
                metadata,
                is_cancelled,
                outcome,
                names,
            )?;
        }
    }
    if is_cancelled() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Interrupted,
            "directory tree deletion cancelled",
        ));
    }
    deletion_entry_io(path, "remove_directory", fs::remove_dir(path))?;
    outcome.had_irreversible_mutation = true;
    Ok(())
}

pub(super) fn remove_directory_contents_cancellable(
    root: &Path,
    is_cancelled: &(dyn Fn() -> bool + Sync),
    allow_parallel: bool,
) -> Result<PermanentDeleteOutcome, RemovalFailure> {
    let mut outcome = PermanentDeleteOutcome::default();
    remove_directory_contents_entry(root, is_cancelled, &mut outcome, allow_parallel).map_err(
        |error| RemovalFailure {
            error,
            verified_outcome: Some(outcome),
        },
    )?;
    Ok(outcome)
}

fn remove_directory_contents_entry(
    path: &Path,
    is_cancelled: &(dyn Fn() -> bool + Sync),
    outcome: &mut PermanentDeleteOutcome,
    allow_parallel: bool,
) -> Result<bool, std::io::Error> {
    if is_cancelled() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Interrupted,
            "directory contents deletion cancelled",
        ));
    }
    let metadata = deletion_entry_io(path, "metadata", fs::symlink_metadata(path))?;
    #[cfg(unix)]
    if metadata.file_type().is_symlink() {
        deletion_entry_io(path, "unlink", fs::remove_file(path))?;
        outcome.had_irreversible_mutation = true;
        return Ok(true);
    }
    if current_platform().is_link_like(&metadata) {
        return Err(std::io::Error::other(
            "a link or reparse point appeared during directory contents deletion",
        ));
    }
    if metadata.is_file() {
        let bytes = metadata.len();
        deletion_entry_io(path, "unlink", fs::remove_file(path))?;
        outcome.had_irreversible_mutation = true;
        outcome.released_bytes = outcome.released_bytes.saturating_add(bytes);
        outcome.affected_item_count = outcome.affected_item_count.saturating_add(1);
        return Ok(true);
    }
    if !metadata.is_dir() {
        return Err(std::io::Error::other(
            "an unsupported filesystem entry appeared during directory contents deletion",
        ));
    }

    let mut entries = deletion_entry_io(path, "read_directory", fs::read_dir(path))?;
    let mut had_entry = false;
    let mut all_removed = true;
    loop {
        let batch = entries
            .by_ref()
            .take(PARALLEL_DELETE_BATCH_SIZE)
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<Result<Vec<_>, _>>()?;
        if batch.is_empty() {
            break;
        }
        had_entry = true;
        if allow_parallel && batch.len() >= PARALLEL_DELETE_ENTRY_THRESHOLD {
            let result = remove_directory_entries_parallel(&batch, is_cancelled);
            outcome.add(result.outcome);
            all_removed &= result.all_removed;
            if let Some(error) = result.error {
                return Err(error);
            }
        } else {
            for child in batch {
                if is_cancelled() {
                    return Err(directory_contents_cancelled_error());
                }
                if !remove_directory_contents_entry(&child, is_cancelled, outcome, allow_parallel)?
                {
                    all_removed = false;
                }
            }
        }
    }
    if had_entry && all_removed {
        deletion_entry_io(path, "remove_directory", fs::remove_dir(path))?;
        outcome.had_irreversible_mutation = true;
        return Ok(true);
    }
    Ok(false)
}

struct ParallelDirectoryRemovalResult {
    outcome: PermanentDeleteOutcome,
    all_removed: bool,
    error: Option<std::io::Error>,
}

fn remove_directory_entries_parallel(
    entries: &[PathBuf],
    is_cancelled: &(dyn Fn() -> bool + Sync),
) -> ParallelDirectoryRemovalResult {
    let worker_count = thread::available_parallelism()
        .map_or(1, usize::from)
        .min(MAX_PARALLEL_DELETE_WORKERS)
        .min(entries.len());
    if worker_count <= 1 {
        return remove_directory_entries_serial(entries, is_cancelled, true);
    }

    let stopped = AtomicBool::new(false);
    let worker_results = thread::scope(|scope| {
        (0..worker_count)
            .map(|worker_index| {
                let stopped = &stopped;
                scope.spawn(move || {
                    let mut result = ParallelDirectoryRemovalResult {
                        outcome: PermanentDeleteOutcome::default(),
                        all_removed: true,
                        error: None,
                    };
                    for path in entries.iter().skip(worker_index).step_by(worker_count) {
                        if stopped.load(Ordering::Relaxed) {
                            break;
                        }
                        if is_cancelled() {
                            stopped.store(true, Ordering::Relaxed);
                            result.error = Some(directory_contents_cancelled_error());
                            break;
                        }
                        match remove_directory_contents_entry(
                            path,
                            is_cancelled,
                            &mut result.outcome,
                            false,
                        ) {
                            Ok(removed) => result.all_removed &= removed,
                            Err(error) => {
                                stopped.store(true, Ordering::Relaxed);
                                result.error = Some(error);
                                break;
                            }
                        }
                    }
                    result
                })
            })
            .collect::<Vec<_>>()
            .into_iter()
            .map(|worker| worker.join())
            .collect::<Vec<_>>()
    });

    let mut combined = ParallelDirectoryRemovalResult {
        outcome: PermanentDeleteOutcome::default(),
        all_removed: true,
        error: None,
    };
    for worker in worker_results {
        match worker {
            Ok(result) => {
                combined.outcome.add(result.outcome);
                combined.all_removed &= result.all_removed;
                if combined.error.is_none() {
                    combined.error = result.error;
                }
            }
            Err(_) if combined.error.is_none() => {
                combined.error = Some(std::io::Error::other(
                    "a parallel directory deletion worker stopped unexpectedly",
                ));
            }
            Err(_) => {}
        }
    }
    combined
}

fn remove_directory_entries_serial(
    entries: &[PathBuf],
    is_cancelled: &(dyn Fn() -> bool + Sync),
    allow_parallel: bool,
) -> ParallelDirectoryRemovalResult {
    let mut result = ParallelDirectoryRemovalResult {
        outcome: PermanentDeleteOutcome::default(),
        all_removed: true,
        error: None,
    };
    for path in entries {
        if is_cancelled() {
            result.error = Some(directory_contents_cancelled_error());
            break;
        }
        match remove_directory_contents_entry(
            path,
            is_cancelled,
            &mut result.outcome,
            allow_parallel,
        ) {
            Ok(removed) => result.all_removed &= removed,
            Err(error) => {
                result.error = Some(error);
                break;
            }
        }
    }
    result
}

fn directory_contents_cancelled_error() -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::Interrupted,
        "directory contents deletion cancelled",
    )
}
