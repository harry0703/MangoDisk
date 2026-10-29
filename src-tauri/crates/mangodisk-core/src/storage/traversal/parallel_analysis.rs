use std::{
    collections::VecDeque,
    panic::{catch_unwind, AssertUnwindSafe},
    sync::{mpsc, Mutex},
    thread,
    time::Duration,
};

use super::*;

#[cfg(windows)]
const WORKERS_ENV: &str = "MANGODISK_WINDOWS_ANALYSIS_WORKERS";
const MAX_WORKERS: usize = 16;
const FILE_BATCH_SIZE: usize = 256;

#[cfg(windows)]
pub(super) fn worker_count(root: &Path) -> usize {
    let scheduling = current_platform()
        .volumes()
        .unwrap_or_default()
        .into_iter()
        .filter(|volume| {
            current_platform().path_is_same_or_child(root, Path::new(&volume.mount_point))
        })
        .max_by_key(|volume| Path::new(&volume.mount_point).components().count())
        .map(|volume| volume.scan_concurrency)
        .unwrap_or_else(|| {
            mangodisk_platform::ScanConcurrency::conservative(
                mangodisk_platform::ScanDeviceClass::Unknown,
            )
        });
    let automatic = automatic_worker_count(
        scheduling,
        thread::available_parallelism()
            .map(usize::from)
            .unwrap_or(1),
    );
    let workers = match std::env::var(WORKERS_ENV) {
        Ok(value) => match value.parse::<usize>() {
            Ok(value) if (1..=MAX_WORKERS).contains(&value) => value,
            _ => {
                log::warn!(
                    "windows_analysis_worker_override_invalid value={} outcome=automatic",
                    mangodisk_platform::diagnostics::text(&value)
                );
                automatic
            }
        },
        Err(_) => automatic,
    };
    log::info!(
        "windows_analysis_scheduler root={} device_class={} automatic_workers={} workers={}",
        diagnostic_path(root),
        scheduling.class.as_str(),
        automatic,
        workers
    );
    workers
}

// Metadata traversal is lighter than content scanning. Keep the latter's device policy
// intact while using measured limits for the two Windows analysis implementations.
#[cfg(any(windows, test))]
fn automatic_worker_count(
    scheduling: mangodisk_platform::ScanConcurrency,
    available: usize,
) -> usize {
    if scheduling.class == mangodisk_platform::ScanDeviceClass::SolidState {
        if cfg!(target_arch = "aarch64") {
            available.clamp(1, 4)
        } else {
            available.saturating_mul(2).clamp(1, MAX_WORKERS)
        }
    } else {
        available.min(scheduling.worker_limit).clamp(1, MAX_WORKERS)
    }
}

enum Task {
    Directory(usize, PathBuf),
    Files(usize, PathBuf, Vec<AnalysisFileEntry>),
}

struct PendingDirectory {
    path: PathBuf,
    parent: Option<usize>,
    metadata: Option<fs::Metadata>,
    read: Option<AnalysisDirectoryRead>,
    pending_children: usize,
}

struct ActiveTask {
    path: PathBuf,
    kind: &'static str,
    started: Instant,
}

fn active_task_summary(active: &Mutex<Vec<Option<ActiveTask>>>) -> String {
    let active = active.lock().unwrap_or_else(|error| error.into_inner());
    let mut tasks = active
        .iter()
        .enumerate()
        .filter_map(|(worker, task)| {
            task.as_ref()
                .map(|task| (worker, task, task.started.elapsed()))
        })
        .collect::<Vec<_>>();
    tasks.sort_unstable_by_key(|(_, _, elapsed)| std::cmp::Reverse(*elapsed));
    tasks
        .into_iter()
        .take(3)
        .map(|(worker, task, elapsed)| {
            format!(
                "worker={} kind={} task_ms={} path={}",
                worker,
                task.kind,
                elapsed.as_millis(),
                crate::filesystem::metadata::display_path(&task.path)
            )
        })
        .collect::<Vec<_>>()
        .join(" | ")
}

pub(super) fn measure(
    root: &Path,
    traversal: &mut AnalysisTraversal<'_>,
    workers: usize,
) -> Result<Option<DirectoryAggregate>, String> {
    if traversal.cancelled.load(Ordering::Relaxed) {
        return Err(OPERATION_CANCELLED_ERROR.to_string());
    }
    let workers = workers.clamp(1, MAX_WORKERS);
    let scan_root = traversal.scan_root;
    let root_metadata = &traversal.root_metadata;
    let purpose = traversal.purpose;
    let scan_mode = traversal.scan_mode;
    let progress = traversal.progress;
    let scanned_at_ms = traversal.scanned_at_ms;
    let exclusions = traversal.scan_exclusions;
    let cancelled = traversal.cancelled;
    let sink = &mut *traversal.sink;
    let abort = AtomicBool::new(false);
    let active_tasks = Mutex::new(
        (0..workers)
            .map(|_| None)
            .collect::<Vec<Option<ActiveTask>>>(),
    );
    let (tasks, task_receiver) = mpsc::channel::<Task>();
    let task_receiver = Mutex::new(task_receiver);
    let (results, result_receiver) = mpsc::sync_channel(workers * 2);
    let skip_log_budget = &traversal.skip_log_budget;
    thread::scope(|scope| {
        let mut handles = Vec::with_capacity(workers);
        let mut outcome = Ok(None);
        let mut spawn_failed = false;
        for index in 0..workers {
            let results = results.clone();
            let task_receiver = &task_receiver;
            let abort = &abort;
            let active_tasks = &active_tasks;
            let handle = thread::Builder::new()
                .name(format!("mangodisk-analysis-{index}"))
                .spawn_scoped(scope, move || loop {
                    let task = task_receiver
                        .lock()
                        .ok()
                        .and_then(|queue| queue.recv().ok());
                    let Some(task) = task else { break };
                    let (id, is_directory) = match &task {
                        Task::Directory(id, _) => (*id, true),
                        Task::Files(id, _, _) => (*id, false),
                    };
                    if abort.load(Ordering::Relaxed) {
                        break;
                    }
                    let (path, kind) = match &task {
                        Task::Directory(_, path) => (path, "directory"),
                        Task::Files(_, path, _) => (path, "file_batch"),
                    };
                    active_tasks.lock().unwrap_or_else(|error| error.into_inner())[index] = Some(ActiveTask { path: path.clone(), kind, started: Instant::now() });
                    let result = catch_unwind(AssertUnwindSafe(|| {
                        let mut partial = IndexRecordSink::memory(None);
                        let mut local = AnalysisTraversal {
            skip_log_budget: Arc::clone(skip_log_budget),
                            scan_root,
                            root_metadata: root_metadata.clone(),
                            purpose,
                            scan_mode,
                            progress,
                            scanned_at_ms,
                            sink: &mut partial,
                            cancelled: abort,
                            scan_exclusions: exclusions,
                        };
                        let read = match task {
                            Task::Directory(_, path) => {
                                // ARM64 keeps directory tasks to avoid extra queued per-file metadata queries.
                                // The measured x64 workload benefits from splitting large descendants.
                                let split_files =
                                    path == scan_root || !cfg!(target_arch = "aarch64");
                                read_analysis_directory_with_file_queries(
                                    &path,
                                    &mut local,
                                    split_files,
                                )?
                            }
                            Task::Files(_, directory, files) => {
                                read_analysis_file_batch(&directory, files, &mut local)?
                            }
                        };
                        Ok::<_, String>((read, partial))
                    }));
                    let activity = active_tasks.lock().unwrap_or_else(|error| error.into_inner())[index].take();
                    let result = result.unwrap_or_else(|_| {
                        if let Some(activity) = activity {
                            log::error!("analysis_worker_panicked operation_id={} worker={} kind={} path={} task_ms={}", progress.operation_id(), index, activity.kind, diagnostic_path(&activity.path), activity.started.elapsed().as_millis());
                        }
                        Err("an analysis directory worker panicked".into())
                    });
                    if results.send((id, is_directory, result)).is_err() {
                        break;
                    }
                });
            match handle {
                Ok(handle) => handles.push(handle),
                Err(error) => {
                    log::warn!(
                        "windows_analysis_worker_unavailable root={} stage=spawn error_code={:?} error={} outcome=serial_fallback",
                        diagnostic_path(root),
                        error.raw_os_error(),
                        mangodisk_platform::diagnostics::text(&error.to_string())
                    );
                    spawn_failed = true;
                    outcome = Err("analysis worker pool unavailable".into());
                    break;
                }
            }
        }
        drop(results);
        let mut nodes = vec![Some(PendingDirectory {
            path: root.to_path_buf(),
            parent: None,
            metadata: None,
            read: None,
            pending_children: 0,
        })];
        let mut outstanding = 0;
        let mut pending_tasks = VecDeque::from([Task::Directory(0, root.to_path_buf())]);
        let health_started = Instant::now();
        let last_health_log = std::sync::atomic::AtomicU64::new(0);
        while (outstanding > 0 || !pending_tasks.is_empty()) && outcome.is_ok() {
            if cancelled.load(Ordering::Relaxed) {
                outcome = Err(OPERATION_CANCELLED_ERROR.to_string());
                break;
            }
            if super::analysis_diagnostics::report_due(
                &last_health_log,
                health_started.elapsed().as_millis() as u64,
            ) {
                let (files, bytes) = progress.scan_observation_counts();
                log::info!(
                    "analysis_worker_progress operation_id={} process_id={} root={} workers={} in_flight={} queued_tasks={} discovered_directories={} files={} bytes={} active_tasks={} elapsed_ms={}",
                    progress.operation_id(), std::process::id(), diagnostic_path(root), workers, outstanding, pending_tasks.len(), nodes.len(), files, bytes,
                    mangodisk_platform::diagnostics::text(&active_task_summary(&active_tasks)), health_started.elapsed().as_millis()
                );
            }
            // Limit dispatched work to the pool size. Prioritize file batches in the local queue
            // so wide trees cannot accumulate snapshots for millions of files ahead of queries.
            while outstanding < workers {
                let Some(task) = pending_tasks.pop_front() else {
                    break;
                };
                if tasks.send(task).is_err() {
                    outcome = Err("analysis directory workers disconnected".into());
                    break;
                }
                outstanding += 1;
            }
            if outcome.is_err() {
                break;
            }
            let result = match result_receiver.recv_timeout(Duration::from_millis(40)) {
                Ok(result) => result,
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    outcome = Err("analysis directory workers disconnected".into());
                    break;
                }
            };
            outstanding -= 1;
            outcome = (|| {
                let (id, is_directory, result) = result;
                let (mut read, partial) = result?;
                sink.merge_directory_records(partial)?;
                if is_directory {
                    let children = std::mem::take(&mut read.children);
                    let mut files = std::mem::take(&mut read.files).into_iter();
                    let node = nodes[id].as_mut().expect("an outstanding node is pending");
                    node.pending_children = children.len();
                    let directory = node.path.clone();
                    node.read = Some(read);
                    loop {
                        let batch: Vec<_> = files.by_ref().take(FILE_BATCH_SIZE).collect();
                        if batch.is_empty() {
                            break;
                        }
                        nodes[id].as_mut().unwrap().pending_children += 1;
                        pending_tasks.push_front(Task::Files(id, directory.clone(), batch));
                    }
                    for (path, metadata) in children {
                        let child_id = nodes.len();
                        nodes.push(Some(PendingDirectory {
                            path: path.clone(),
                            parent: Some(id),
                            metadata: Some(metadata),
                            read: None,
                            pending_children: 0,
                        }));
                        pending_tasks.push_back(Task::Directory(child_id, path));
                    }
                } else {
                    let node = nodes[id]
                        .as_mut()
                        .expect("a file batch's directory is pending");
                    let directory = node
                        .read
                        .as_mut()
                        .expect("file work follows directory discovery");
                    directory.aggregate.bytes += read.aggregate.bytes;
                    directory.aggregate.logical_bytes += read.aggregate.logical_bytes;
                    directory.aggregate.file_count += read.aggregate.file_count;
                    directory.aggregate.direct_file_count += read.aggregate.direct_file_count;
                    directory.aggregate.skipped_count += read.aggregate.skipped_count;
                    directory
                        .aggregate
                        .omissions
                        .merge(read.aggregate.omissions);
                    directory
                        .fingerprint_entries
                        .extend(read.fingerprint_entries);
                    for file in read.analysis_files.into_files() {
                        directory.analysis_files.push(file);
                    }
                    node.pending_children -= 1;
                }
                complete_ready(id, &mut nodes, purpose, scan_root, sink)
            })();
        }
        abort.store(true, Ordering::Relaxed);
        // Disconnect both queues before joining. Workers may be waiting for a task or blocked
        // behind result backpressure when cancellation or the first failure stops consumption.
        drop(tasks);
        drop(result_receiver);
        for handle in handles {
            if handle.join().is_err() && outcome.is_ok() {
                outcome = Err("an analysis worker stopped unexpectedly".into());
            }
        }
        if cancelled.load(Ordering::Relaxed) {
            return Err(OPERATION_CANCELLED_ERROR.to_string());
        }
        // No task is dispatched until every worker has started, so a failed spawn leaves
        // the sink and progress untouched and permits the original serial fallback.
        if spawn_failed {
            return Ok(None);
        }
        outcome?
            .map(Some)
            .ok_or_else(|| "parallel analysis did not complete its root".to_string())
    })
}

fn complete_ready(
    mut id: usize,
    nodes: &mut [Option<PendingDirectory>],
    purpose: ScanPurpose,
    root: &Path,
    sink: &mut IndexRecordSink,
) -> Result<Option<DirectoryAggregate>, String> {
    loop {
        let node = nodes[id].as_ref().expect("a completing node is pending");
        if node.read.is_none() || node.pending_children != 0 {
            return Ok(None);
        }
        let node = nodes[id].take().expect("a ready node exists");
        let aggregate = finish_analysis_directory(
            &node.path,
            node.read.expect("a ready node has been read"),
            purpose,
            root,
            sink,
        )?;
        let Some(parent) = node.parent else {
            return Ok(Some(aggregate));
        };
        let parent_node = nodes[parent].as_mut().expect("a child's parent is pending");
        add_analysis_child(
            parent_node
                .read
                .as_mut()
                .expect("a parent is read before its children"),
            &node.path,
            node.metadata
                .as_ref()
                .expect("a child retains discovery metadata"),
            aggregate,
            purpose,
        );
        parent_node.pending_children -= 1;
        id = parent;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(root: &Path, workers: usize) -> CompletedIndexSink {
        let progress = Arc::new(ProgressTracker::new(1, |_| {}, 0));
        let cancelled = AtomicBool::new(false);
        let mut sink = IndexRecordSink::memory(None);
        let mut traversal = AnalysisTraversal {
            skip_log_budget: Default::default(),
            scan_root: root,
            root_metadata: fs::symlink_metadata(root).unwrap(),
            purpose: ScanPurpose::Analysis,
            scan_mode: AnalysisScanMode::Standard,
            progress: &progress,
            scanned_at_ms: 123,
            sink: &mut sink,
            cancelled: &cancelled,
            scan_exclusions: None,
        };
        if workers == 0 {
            measure_analysis_directory(root, &mut traversal).unwrap();
        } else {
            measure(root, &mut traversal, workers).unwrap();
        }
        sink.finish_analysis().unwrap()
    }

    #[test]
    fn automatic_limits_keep_unknown_and_rotational_volumes_conservative() {
        use mangodisk_platform::{ScanConcurrency, ScanDeviceClass};
        assert_eq!(automatic_worker_count(ScanConcurrency::rotational(), 16), 2);
        for class in [
            ScanDeviceClass::Unknown,
            ScanDeviceClass::Network,
            ScanDeviceClass::Removable,
        ] {
            assert_eq!(
                automatic_worker_count(ScanConcurrency::conservative(class), 16),
                1
            );
        }
        assert_eq!(
            automatic_worker_count(ScanConcurrency::solid_state(), 1),
            if cfg!(target_arch = "aarch64") { 1 } else { 2 }
        );
        assert_eq!(
            automatic_worker_count(ScanConcurrency::solid_state(), 16),
            if cfg!(target_arch = "aarch64") { 4 } else { 16 }
        );
    }

    #[test]
    fn worker_orders_preserve_all_directory_fingerprints_and_retained_files() {
        let root = tempfile::tempdir().unwrap();
        for directory in 0..32 {
            let child = root.path().join(format!("branch-{directory:02}"));
            fs::create_dir_all(child.join("nested")).unwrap();
            for file in 0..80 {
                fs::write(child.join(format!("file-{file:02}")), vec![1; file + 1]).unwrap();
            }
            fs::write(child.join("nested/content"), b"nested content").unwrap();
        }
        fs::hard_link(
            root.path().join("branch-00/file-79"),
            root.path().join("branch-31/alias"),
        )
        .unwrap();
        for file in 0..1024 {
            fs::write(
                root.path().join(format!("root-file-{file:04}")),
                vec![2; file + 1],
            )
            .unwrap();
        }
        #[cfg(windows)]
        {
            let path = root.path().join("wof.bin");
            fs::write(&path, vec![0_u8; 1024 * 1024]).unwrap();
            let output = std::process::Command::new("compact.exe")
                .args(["/C", "/F", "/EXE:XPRESS4K"])
                .arg(&path)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "WOF fixture compression must succeed"
            );
            let metadata = fs::symlink_metadata(&path).unwrap();
            let usage = current_platform().file_space_usage(&path, &metadata);
            assert!(
                usage.allocated_bytes < usage.logical_bytes,
                "WOF fixture must have native compressed usage before comparing scan results"
            );
        }
        let baseline = snapshot(root.path(), 0);
        for workers in [1, 2, 4, 8] {
            let candidate = snapshot(root.path(), workers);
            assert_eq!(baseline.directories.len(), candidate.directories.len());
            for (path, expected) in &baseline.directories {
                let actual = &candidate.directories[path];
                assert_eq!(
                    (
                        expected.bytes,
                        expected.logical_bytes,
                        expected.file_count,
                        expected.direct_file_count,
                        expected.skipped_count,
                        expected.fingerprint
                    ),
                    (
                        actual.bytes,
                        actual.logical_bytes,
                        actual.file_count,
                        actual.direct_file_count,
                        actual.skipped_count,
                        actual.fingerprint
                    ),
                    "directory facts must not depend on completion order: {path:?}"
                );
            }
            assert_eq!(baseline.files.len(), candidate.files.len());
            for (path, expected) in &baseline.files {
                let actual = &candidate.files[path];
                assert_eq!(
                    (
                        expected.bytes,
                        expected.logical_bytes,
                        expected.modified_at_ms
                    ),
                    (actual.bytes, actual.logical_bytes, actual.modified_at_ms),
                    "bounded candidates and hard-link ownership must be stable: {path:?}"
                );
            }
        }
    }

    #[test]
    fn worker_panic_returns_an_error_and_joins_the_pool() {
        let root = tempfile::tempdir().unwrap();
        let progress = Arc::new(ProgressTracker::new(
            11,
            |_| panic!("synthetic worker failure"),
            0,
        ));
        let cancelled = AtomicBool::new(false);
        let mut sink = IndexRecordSink::memory(None);
        let mut traversal = AnalysisTraversal {
            skip_log_budget: Default::default(),
            scan_root: root.path(),
            root_metadata: fs::symlink_metadata(root.path()).unwrap(),
            purpose: ScanPurpose::Analysis,
            scan_mode: AnalysisScanMode::Standard,
            progress: &progress,
            scanned_at_ms: 0,
            sink: &mut sink,
            cancelled: &cancelled,
            scan_exclusions: None,
        };
        let started = Instant::now();
        assert_eq!(
            measure(root.path(), &mut traversal, 4).err().as_deref(),
            Some("an analysis directory worker panicked")
        );
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn cancellation_joins_workers_without_returning_a_completed_snapshot() {
        let root = tempfile::tempdir().unwrap();
        for directory in 0..64 {
            let path = root.path().join(format!("branch-{directory}"));
            fs::create_dir(&path).unwrap();
            fs::write(path.join("file"), b"content").unwrap();
        }
        let cancelled = Arc::new(AtomicBool::new(false));
        let callback_cancelled = Arc::clone(&cancelled);
        let progress = Arc::new(ProgressTracker::new(
            1,
            move |_| callback_cancelled.store(true, Ordering::Relaxed),
            0,
        ));
        let mut sink = IndexRecordSink::memory(None);
        let mut traversal = AnalysisTraversal {
            skip_log_budget: Default::default(),
            scan_root: root.path(),
            root_metadata: fs::symlink_metadata(root.path()).unwrap(),
            purpose: ScanPurpose::Analysis,
            scan_mode: AnalysisScanMode::Standard,
            progress: &progress,
            scanned_at_ms: 0,
            sink: &mut sink,
            cancelled: &cancelled,
            scan_exclusions: None,
        };
        let started = Instant::now();
        assert_eq!(
            measure(root.path(), &mut traversal, 4).err().as_deref(),
            Some(OPERATION_CANCELLED_ERROR)
        );
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn queued_file_replaced_with_a_directory_link_is_rejected() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("queued-file");
        fs::write(&path, b"original").unwrap();
        let file = AnalysisFileEntry {
            path: path.clone(),
            metadata: fs::symlink_metadata(&path).unwrap(),
        };
        fs::remove_file(&path).unwrap();
        let target = tempfile::tempdir().unwrap();
        fs::write(target.path().join("outside"), b"outside scope").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(target.path(), &path).unwrap();
        #[cfg(windows)]
        assert!(std::process::Command::new("cmd.exe")
            .args(["/d", "/c", "mklink", "/J"])
            .arg(&path)
            .arg(target.path())
            .output()
            .unwrap()
            .status
            .success());
        let progress = Arc::new(ProgressTracker::new(1, |_| {}, 0));
        let cancelled = AtomicBool::new(false);
        let mut sink = IndexRecordSink::memory(None);
        let mut traversal = AnalysisTraversal {
            skip_log_budget: Default::default(),
            scan_root: root.path(),
            root_metadata: fs::symlink_metadata(root.path()).unwrap(),
            purpose: ScanPurpose::Analysis,
            scan_mode: AnalysisScanMode::Standard,
            progress: &progress,
            scanned_at_ms: 0,
            sink: &mut sink,
            cancelled: &cancelled,
            scan_exclusions: None,
        };
        let read = read_analysis_file_batch(root.path(), vec![file], &mut traversal).unwrap();
        assert_eq!(read.aggregate.file_count, 0);
        assert_eq!(read.aggregate.skipped_count, 1);
        assert!(read.fingerprint_entries.is_empty());
        #[cfg(windows)]
        fs::remove_dir(&path).unwrap();
    }

    #[test]
    fn queued_directory_replaced_with_a_link_is_rejected_when_execution_starts() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("queued");
        let target = tempfile::tempdir().unwrap();
        fs::create_dir(&path).unwrap();
        let root_metadata = fs::symlink_metadata(root.path()).unwrap();
        fs::write(target.path().join("must-not-read"), b"outside scope").unwrap();
        fs::remove_dir(&path).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(target.path(), &path).unwrap();
        #[cfg(windows)]
        assert!(std::process::Command::new("cmd.exe")
            .args(["/d", "/c", "mklink", "/J"])
            .arg(&path)
            .arg(target.path())
            .output()
            .unwrap()
            .status
            .success());
        let progress = Arc::new(ProgressTracker::new(1, |_| {}, 0));
        let cancelled = AtomicBool::new(false);
        let mut sink = IndexRecordSink::memory(None);
        let mut traversal = AnalysisTraversal {
            skip_log_budget: Default::default(),
            scan_root: root.path(),
            root_metadata,
            purpose: ScanPurpose::Analysis,
            scan_mode: AnalysisScanMode::Standard,
            progress: &progress,
            scanned_at_ms: 0,
            sink: &mut sink,
            cancelled: &cancelled,
            scan_exclusions: None,
        };
        let read = read_analysis_directory(&path, &mut traversal).unwrap();
        assert_eq!(read.aggregate.file_count, 0);
        assert_eq!(read.aggregate.skipped_count, 1);
        assert!(read.children.is_empty());
        #[cfg(windows)]
        fs::remove_dir(&path).unwrap();
    }
}
