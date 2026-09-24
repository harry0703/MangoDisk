use std::{
    fs, io,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, RecvTimeoutError},
        Arc, Condvar, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

use crate::{
    FastAnalysisQuery, FastAnalysisRecord, FastAnalysisScanError, FastAnalysisSummary, ScanPurpose,
};

use super::{directories, volumes, LinuxPlatform};

const MAX_DIRECTORY_WORKERS: usize = 4;
const RESULT_QUEUE_FACTOR: usize = 2;
const RESULT_POLL_INTERVAL: Duration = Duration::from_millis(40);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct DirectoryTotals {
    logical_bytes: u64,
    allocated_bytes: u64,
    file_count: u64,
    skipped_count: u64,
}

impl DirectoryTotals {
    fn progress_bytes(self, purpose: ScanPurpose) -> u64 {
        match purpose {
            ScanPurpose::DuplicateFiles => self.logical_bytes,
            _ => self.allocated_bytes,
        }
    }

    fn add_file(
        &mut self,
        logical_bytes: u64,
        allocated_bytes: u64,
    ) -> Result<(), FastAnalysisScanError> {
        self.logical_bytes = self
            .logical_bytes
            .checked_add(logical_bytes)
            .ok_or_else(|| platform_error("directory_bytes_overflow"))?;
        self.allocated_bytes = self
            .allocated_bytes
            .checked_add(allocated_bytes)
            .ok_or_else(|| platform_error("directory_allocation_overflow"))?;
        self.file_count = self
            .file_count
            .checked_add(1)
            .ok_or_else(|| platform_error("directory_file_count_overflow"))?;
        Ok(())
    }

    fn add_directory(&mut self, child: Self) -> Result<(), FastAnalysisScanError> {
        self.logical_bytes = self
            .logical_bytes
            .checked_add(child.logical_bytes)
            .ok_or_else(|| platform_error("directory_bytes_overflow"))?;
        self.allocated_bytes = self
            .allocated_bytes
            .checked_add(child.allocated_bytes)
            .ok_or_else(|| platform_error("directory_allocation_overflow"))?;
        self.file_count = self
            .file_count
            .checked_add(child.file_count)
            .ok_or_else(|| platform_error("directory_file_count_overflow"))?;
        self.skipped_count = self
            .skipped_count
            .checked_add(child.skipped_count)
            .ok_or_else(|| platform_error("directory_skipped_count_overflow"))?;
        Ok(())
    }

    fn skip_entry(&mut self) -> Result<(), FastAnalysisScanError> {
        self.skipped_count = self
            .skipped_count
            .checked_add(1)
            .ok_or_else(|| platform_error("directory_skipped_count_overflow"))?;
        Ok(())
    }
}

#[derive(Debug)]
struct DirectoryTask {
    node_id: usize,
    path: PathBuf,
    is_root: bool,
}

#[derive(Debug)]
struct DirectoryReadResult {
    task: DirectoryTask,
    direct_totals: DirectoryTotals,
    child_directories: Vec<PathBuf>,
    candidates: Vec<PathBuf>,
    entry_count: u64,
}

#[derive(Clone)]
struct DirectoryReadPolicy {
    excluded_roots: Arc<[PathBuf]>,
    root_device: u64,
    purpose: ScanPurpose,
    should_prune_directory: fn(&Path) -> bool,
    large_file_minimum_bytes: u64,
}

#[derive(Debug)]
struct PendingDirectory {
    path: PathBuf,
    parent_id: Option<usize>,
    totals: DirectoryTotals,
    pending_children: usize,
    has_been_read: bool,
}

#[derive(Default)]
struct DirectoryTaskQueue {
    state: Mutex<DirectoryTaskQueueState>,
    ready: Condvar,
}

#[derive(Default)]
struct DirectoryTaskQueueState {
    tasks: Vec<DirectoryTask>,
    stopped: bool,
}

#[derive(Default)]
struct ScanDiagnostics {
    entry_count: u64,
    directory_count: u64,
    candidate_count: u64,
    consumer_elapsed: Duration,
}

struct AnalysisCoordinator<'a> {
    pending_nodes: Vec<Option<PendingDirectory>>,
    outstanding_tasks: usize,
    task_queue: &'a DirectoryTaskQueue,
    consumer: &'a mut dyn FnMut(FastAnalysisRecord) -> Result<(), String>,
    report_progress: &'a mut dyn FnMut(&Path, u64, u64),
    purpose: ScanPurpose,
    diagnostics: ScanDiagnostics,
    root_totals: Option<DirectoryTotals>,
}

impl DirectoryTaskQueue {
    fn push_many(&self, tasks: impl IntoIterator<Item = DirectoryTask>) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.tasks.extend(tasks);
        self.ready.notify_all();
    }

    fn pop(&self) -> Option<DirectoryTask> {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        loop {
            if let Some(task) = state.tasks.pop() {
                return Some(task);
            }
            if state.stopped {
                return None;
            }
            state = self
                .ready
                .wait(state)
                .unwrap_or_else(|error| error.into_inner());
        }
    }

    fn stop(&self) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.stopped = true;
        state.tasks.clear();
        self.ready.notify_all();
    }
}

impl<'a> AnalysisCoordinator<'a> {
    fn new(
        root: &Path,
        purpose: ScanPurpose,
        task_queue: &'a DirectoryTaskQueue,
        report_progress: &'a mut dyn FnMut(&Path, u64, u64),
        consumer: &'a mut dyn FnMut(FastAnalysisRecord) -> Result<(), String>,
    ) -> Self {
        Self {
            pending_nodes: vec![Some(PendingDirectory {
                path: root.to_path_buf(),
                parent_id: None,
                totals: DirectoryTotals::default(),
                pending_children: 0,
                has_been_read: false,
            })],
            outstanding_tasks: 1,
            task_queue,
            consumer,
            report_progress,
            purpose,
            diagnostics: ScanDiagnostics::default(),
            root_totals: None,
        }
    }

    fn consume(&mut self, result: DirectoryReadResult) -> Result<(), FastAnalysisScanError> {
        self.diagnostics.entry_count = self
            .diagnostics
            .entry_count
            .checked_add(result.entry_count)
            .ok_or_else(|| platform_error("entry_count_overflow"))?;
        (self.report_progress)(
            &result.task.path,
            result.direct_totals.file_count,
            result.direct_totals.progress_bytes(self.purpose),
        );
        for candidate in result.candidates {
            emit_candidate(candidate, self.consumer, &mut self.diagnostics)?;
        }

        let child_count = result.child_directories.len();
        let node = self
            .pending_nodes
            .get_mut(result.task.node_id)
            .and_then(Option::as_mut)
            .ok_or_else(|| platform_error("directory_node_missing"))?;
        node.totals = result.direct_totals;
        node.pending_children = child_count;
        node.has_been_read = true;

        let mut child_tasks = Vec::with_capacity(child_count);
        for path in result.child_directories {
            let node_id = self.pending_nodes.len();
            self.pending_nodes.push(Some(PendingDirectory {
                path: path.clone(),
                parent_id: Some(result.task.node_id),
                totals: DirectoryTotals::default(),
                pending_children: 0,
                has_been_read: false,
            }));
            child_tasks.push(DirectoryTask {
                node_id,
                path,
                is_root: false,
            });
        }
        self.outstanding_tasks = self
            .outstanding_tasks
            .checked_add(child_count)
            .ok_or_else(|| platform_error("outstanding_task_count_overflow"))?;
        self.task_queue.push_many(child_tasks);

        if child_count == 0 {
            finalize_directory_chain(
                result.task.node_id,
                &mut self.pending_nodes,
                self.consumer,
                &mut self.diagnostics,
                &mut self.root_totals,
            )?;
        }
        Ok(())
    }
}

pub(super) fn analyze_records(
    _platform: &LinuxPlatform,
    query: FastAnalysisQuery<'_>,
    is_cancelled: &(dyn Fn() -> bool + Sync),
    report_progress: &mut dyn FnMut(&Path, u64, u64),
    consumer: &mut dyn FnMut(FastAnalysisRecord) -> Result<(), String>,
) -> Result<FastAnalysisSummary, FastAnalysisScanError> {
    if is_cancelled() {
        return Err(FastAnalysisScanError::Cancelled);
    }
    let root_metadata = fs::symlink_metadata(query.root)
        .map_err(|error| platform_io_error("root_metadata", query.root, &error))?;
    if !root_metadata.is_dir() || root_metadata.file_type().is_symlink() {
        return Err(platform_error("root_is_not_a_physical_directory"));
    }

    let device_concurrency = volumes::volume_for_path(query.root)
        .map(|volume| volume.scan_concurrency)
        .unwrap_or_else(|error| {
            log::warn!(
                "linux_native_analysis_device_detection_failed diagnostic={}",
                error.diagnostic()
            );
            crate::ScanConcurrency::conservative(crate::ScanDeviceClass::Unknown)
        });
    let worker_count = thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(1)
        .min(device_concurrency.worker_limit)
        .clamp(1, MAX_DIRECTORY_WORKERS);
    log::debug!(
        "linux_native_analysis_scheduler device_class={} worker_count={}",
        device_concurrency.class.as_str(),
        worker_count
    );
    let task_queue = Arc::new(DirectoryTaskQueue::default());
    let abort = Arc::new(AtomicBool::new(false));
    let policy = DirectoryReadPolicy {
        excluded_roots: Arc::from(query.excluded_roots),
        root_device: root_metadata.dev(),
        purpose: query.purpose,
        should_prune_directory: query.should_prune_directory,
        large_file_minimum_bytes: query.large_file_minimum_bytes,
    };
    let queue_capacity = worker_count.saturating_mul(RESULT_QUEUE_FACTOR).max(1);
    let (result_sender, result_receiver) = mpsc::sync_channel(queue_capacity);
    let mut workers = Vec::with_capacity(worker_count);
    for worker_index in 0..worker_count {
        let worker_task_queue = Arc::clone(&task_queue);
        let worker_abort = Arc::clone(&abort);
        let worker_policy = policy.clone();
        let worker_sender = result_sender.clone();
        let worker = thread::Builder::new()
            .name(format!("mangodisk-linux-analysis-{worker_index}"))
            .spawn(move || {
                while let Some(task) = worker_task_queue.pop() {
                    if worker_abort.load(Ordering::Relaxed) {
                        break;
                    }
                    let result = read_directory(&worker_policy, task, &worker_abort);
                    if worker_sender.send(result).is_err() {
                        break;
                    }
                }
            });
        match worker {
            Ok(worker) => workers.push(worker),
            Err(error) => {
                abort.store(true, Ordering::Relaxed);
                task_queue.stop();
                for worker in workers {
                    let _ = worker.join();
                }
                return Err(platform_io_error(
                    "spawn_analysis_worker",
                    query.root,
                    &error,
                ));
            }
        }
    }
    drop(result_sender);

    let mut coordinator = AnalysisCoordinator::new(
        query.root,
        query.purpose,
        task_queue.as_ref(),
        report_progress,
        consumer,
    );
    task_queue.push_many([DirectoryTask {
        node_id: 0,
        path: query.root.to_path_buf(),
        is_root: true,
    }]);
    let mut scan_result = Ok(());
    while coordinator.outstanding_tasks > 0 {
        if is_cancelled() {
            scan_result = Err(FastAnalysisScanError::Cancelled);
            break;
        }
        let read_result = match result_receiver.recv_timeout(RESULT_POLL_INTERVAL) {
            Ok(result) => result,
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => {
                scan_result = Err(platform_error("analysis_workers_disconnected"));
                break;
            }
        };
        coordinator.outstanding_tasks -= 1;
        match read_result.and_then(|result| coordinator.consume(result)) {
            Ok(()) => {}
            Err(error) => {
                scan_result = Err(error);
                break;
            }
        }
    }

    abort.store(true, Ordering::Relaxed);
    task_queue.stop();
    // Cancellation or a consumer failure may leave workers blocked on the bounded result queue.
    // Disconnect it before joining so those sends fail promptly and every worker can exit.
    drop(result_receiver);
    for worker in workers {
        if worker.join().is_err() && scan_result.is_ok() {
            scan_result = Err(platform_error("analysis_worker_panicked"));
        }
    }
    scan_result?;
    let totals = coordinator
        .root_totals
        .ok_or_else(|| platform_error("root_totals_missing"))?;
    Ok(FastAnalysisSummary {
        root_logical_bytes: totals.logical_bytes,
        root_allocated_bytes: totals.allocated_bytes,
        root_file_count: totals.file_count,
        root_skipped_count: totals.skipped_count,
        page_count: coordinator.diagnostics.directory_count,
        entry_count: coordinator.diagnostics.entry_count,
        directory_count: coordinator.diagnostics.directory_count,
        candidate_count: coordinator.diagnostics.candidate_count,
        returned_bytes: 0,
        consumer_elapsed_ms: u64::try_from(coordinator.diagnostics.consumer_elapsed.as_millis())
            .unwrap_or(u64::MAX),
        strategy: "linux_parallel_read_dir_metadata_v1",
    })
}

fn read_directory(
    policy: &DirectoryReadPolicy,
    task: DirectoryTask,
    abort: &AtomicBool,
) -> Result<DirectoryReadResult, FastAnalysisScanError> {
    check_aborted(abort)?;
    let entries = match fs::read_dir(&task.path) {
        Ok(entries) => entries,
        Err(_error) if !task.is_root => {
            return Ok(DirectoryReadResult {
                task,
                direct_totals: DirectoryTotals {
                    skipped_count: 1,
                    ..DirectoryTotals::default()
                },
                child_directories: Vec::new(),
                candidates: Vec::new(),
                entry_count: 0,
            });
        }
        Err(error) => return Err(platform_io_error("open_root_directory", &task.path, &error)),
    };
    let mut totals = DirectoryTotals::default();
    let mut child_directories = Vec::new();
    let mut candidates = Vec::new();
    let mut entry_count = 0_u64;
    for entry in entries {
        check_aborted(abort)?;
        entry_count = entry_count
            .checked_add(1)
            .ok_or_else(|| platform_error("entry_count_overflow"))?;
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => {
                totals.skip_entry()?;
                continue;
            }
        };
        let path = entry.path();
        // DirEntry::metadata follows links; inspect the directory entry itself so
        // native traversal cannot count or descend into a symbolic-link target.
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(_) => {
                totals.skip_entry()?;
                continue;
            }
        };
        if metadata.file_type().is_symlink() || metadata.dev() != policy.root_device {
            totals.skip_entry()?;
            continue;
        }
        if metadata.is_dir() {
            if policy
                .excluded_roots
                .iter()
                .any(|excluded| path.starts_with(excluded))
            {
                continue;
            }
            if (policy.should_prune_directory)(&path) || should_skip(&path, policy.purpose, true) {
                totals.skip_entry()?;
                continue;
            }
            child_directories.push(path);
        } else if metadata.is_file() {
            if should_skip(&path, policy.purpose, false) {
                totals.skip_entry()?;
                continue;
            }
            let logical_bytes = metadata.len();
            let allocated_bytes = metadata.blocks().saturating_mul(512);
            totals.add_file(logical_bytes, allocated_bytes)?;
            let candidate_purpose = match policy.purpose {
                ScanPurpose::DuplicateFiles => ScanPurpose::DuplicateFiles,
                _ => ScanPurpose::LargeFiles,
            };
            let candidate_bytes = match policy.purpose {
                ScanPurpose::DuplicateFiles => logical_bytes,
                _ => allocated_bytes,
            };
            if candidate_bytes >= policy.large_file_minimum_bytes
                && !should_skip(&path, candidate_purpose, false)
            {
                candidates.push(path);
            }
        } else {
            totals.skip_entry()?;
        }
    }
    Ok(DirectoryReadResult {
        task,
        direct_totals: totals,
        child_directories,
        candidates,
        entry_count,
    })
}

fn should_skip(path: &Path, purpose: ScanPurpose, is_directory: bool) -> bool {
    if purpose == ScanPurpose::Cleanup {
        return false;
    }
    if directories::is_system_critical(path) {
        return true;
    }
    if matches!(
        purpose,
        ScanPurpose::LargeFiles | ScanPurpose::DuplicateFiles
    ) {
        if directories::is_package_manager_owned(path) {
            return true;
        }
        if is_directory && directories::is_unwritable_by_current_user(path) {
            return true;
        }
    }
    false
}

fn finalize_directory_chain(
    mut node_id: usize,
    pending_nodes: &mut [Option<PendingDirectory>],
    consumer: &mut dyn FnMut(FastAnalysisRecord) -> Result<(), String>,
    diagnostics: &mut ScanDiagnostics,
    root_totals: &mut Option<DirectoryTotals>,
) -> Result<(), FastAnalysisScanError> {
    loop {
        let node = pending_nodes
            .get_mut(node_id)
            .and_then(Option::take)
            .ok_or_else(|| platform_error("directory_node_missing_during_finalize"))?;
        if !node.has_been_read || node.pending_children != 0 {
            return Err(platform_error("directory_finalized_before_children"));
        }
        emit_directory(&node.path, node.totals, consumer, diagnostics)?;
        let Some(parent_id) = node.parent_id else {
            *root_totals = Some(node.totals);
            return Ok(());
        };
        let parent = pending_nodes
            .get_mut(parent_id)
            .and_then(Option::as_mut)
            .ok_or_else(|| platform_error("parent_directory_node_missing"))?;
        parent.totals.add_directory(node.totals)?;
        parent.pending_children = parent
            .pending_children
            .checked_sub(1)
            .ok_or_else(|| platform_error("pending_child_count_underflow"))?;
        if !parent.has_been_read || parent.pending_children != 0 {
            return Ok(());
        }
        node_id = parent_id;
    }
}

fn emit_directory(
    path: &Path,
    totals: DirectoryTotals,
    consumer: &mut dyn FnMut(FastAnalysisRecord) -> Result<(), String>,
    diagnostics: &mut ScanDiagnostics,
) -> Result<(), FastAnalysisScanError> {
    let started = Instant::now();
    consumer(FastAnalysisRecord::Directory {
        path: path.to_path_buf(),
        logical_bytes: totals.logical_bytes,
        allocated_bytes: totals.allocated_bytes,
        file_count: totals.file_count,
        skipped_count: totals.skipped_count,
    })
    .map_err(FastAnalysisScanError::Consumer)?;
    diagnostics.consumer_elapsed += started.elapsed();
    diagnostics.directory_count = diagnostics
        .directory_count
        .checked_add(1)
        .ok_or_else(|| platform_error("directory_count_overflow"))?;
    Ok(())
}

fn emit_candidate(
    path: PathBuf,
    consumer: &mut dyn FnMut(FastAnalysisRecord) -> Result<(), String>,
    diagnostics: &mut ScanDiagnostics,
) -> Result<(), FastAnalysisScanError> {
    let started = Instant::now();
    consumer(FastAnalysisRecord::LargeFileCandidate(path))
        .map_err(FastAnalysisScanError::Consumer)?;
    diagnostics.consumer_elapsed += started.elapsed();
    diagnostics.candidate_count = diagnostics
        .candidate_count
        .checked_add(1)
        .ok_or_else(|| platform_error("candidate_count_overflow"))?;
    Ok(())
}

fn check_aborted(abort: &AtomicBool) -> Result<(), FastAnalysisScanError> {
    if abort.load(Ordering::Relaxed) {
        Err(FastAnalysisScanError::Cancelled)
    } else {
        Ok(())
    }
}

fn platform_error(code: &str) -> FastAnalysisScanError {
    FastAnalysisScanError::Platform(code.to_string())
}

fn platform_io_error(operation: &str, path: &Path, error: &io::Error) -> FastAnalysisScanError {
    FastAnalysisScanError::Platform(format!(
        "{operation}:path={} io_kind={:?} os_code={:?} diagnostic={}",
        crate::diagnostics::text(&path.display()),
        error.kind(),
        error.raw_os_error(),
        crate::diagnostics::text(error)
    ))
}

#[cfg(test)]
mod tests {
    use std::{
        os::unix::fs::symlink,
        sync::mpsc,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;

    struct DirectoryCleanup(PathBuf);

    impl Drop for DirectoryCleanup {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn fixture_root(label: &str) -> (PathBuf, DirectoryCleanup) {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "mangodisk-linux-analysis-{label}-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&root).expect("create native analysis fixture");
        (root.clone(), DirectoryCleanup(root))
    }

    #[test]
    fn native_analysis_aggregates_files_and_skips_links() {
        let (root, _cleanup) = fixture_root("aggregate");
        let child = root.join("child");
        let file = child.join("sample.bin");
        let link = root.join("sample-link");
        let directory_link = root.join("child-link");
        fs::create_dir(&child).expect("create fixture child directory");
        fs::write(&file, vec![7_u8; 4_097]).expect("write fixture file");
        symlink(&file, &link).expect("create fixture symbolic link");
        symlink(&child, &directory_link).expect("create fixture directory link");
        let metadata = fs::symlink_metadata(&file).expect("read fixture metadata");
        let expected_allocated_bytes = metadata.blocks().saturating_mul(512);

        let mut records = Vec::new();
        let mut progress_files = 0_u64;
        let mut progress_bytes = 0_u64;
        let summary = analyze_records(
            &LinuxPlatform,
            FastAnalysisQuery {
                excluded_roots: &[],
                root: &root,
                purpose: ScanPurpose::Analysis,
                large_file_minimum_bytes: 1,
                should_prune_directory: |_| false,
            },
            &|| false,
            &mut |_, files, bytes| {
                progress_files = progress_files.saturating_add(files);
                progress_bytes = progress_bytes.saturating_add(bytes);
            },
            &mut |record| {
                records.push(record);
                Ok(())
            },
        )
        .expect("run native analysis");

        assert_eq!(summary.root_logical_bytes, 4_097);
        assert_eq!(summary.root_allocated_bytes, expected_allocated_bytes);
        assert_eq!(summary.root_file_count, 1);
        assert_eq!(summary.root_skipped_count, 2);
        assert_eq!(summary.entry_count, 4);
        assert_eq!(summary.directory_count, 2);
        assert_eq!(summary.candidate_count, 1);
        assert_eq!(progress_files, 1);
        assert_eq!(progress_bytes, expected_allocated_bytes);
        assert_eq!(
            records
                .iter()
                .filter(|record| matches!(record, FastAnalysisRecord::Directory { .. }))
                .count(),
            2
        );
        assert!(records.iter().any(
            |record| matches!(record, FastAnalysisRecord::LargeFileCandidate(path) if path == &file)
        ));
    }

    #[test]
    fn native_analysis_honors_cancellation_before_start() {
        let (root, _cleanup) = fixture_root("cancelled");
        let result = analyze_records(
            &LinuxPlatform,
            FastAnalysisQuery {
                excluded_roots: &[],
                root: &root,
                purpose: ScanPurpose::Analysis,
                large_file_minimum_bytes: 1,
                should_prune_directory: |_| false,
            },
            &|| true,
            &mut |_, _, _| {},
            &mut |_| Ok(()),
        );
        assert!(matches!(result, Err(FastAnalysisScanError::Cancelled)));
    }

    #[test]
    fn native_analysis_disconnects_workers_after_consumer_failure() {
        let (root, _cleanup) = fixture_root("consumer-failure");
        for index in 0..64 {
            let directory = root.join(format!("child-{index}"));
            fs::create_dir(&directory).expect("create fixture child directory");
            fs::write(directory.join("candidate.bin"), [index as u8])
                .expect("write fixture candidate");
        }
        let (sender, receiver) = mpsc::sync_channel(1);
        let scan_root = root.clone();
        let worker = thread::spawn(move || {
            let result = analyze_records(
                &LinuxPlatform,
                FastAnalysisQuery {
                    excluded_roots: &[],
                    root: &scan_root,
                    purpose: ScanPurpose::Analysis,
                    large_file_minimum_bytes: 0,
                    should_prune_directory: |_| false,
                },
                &|| false,
                &mut |_, _, _| {},
                &mut |record| match record {
                    FastAnalysisRecord::LargeFileCandidate(_) => Err("expected".to_string()),
                    FastAnalysisRecord::Directory { .. } => Ok(()),
                },
            );
            let _ = sender.send(result);
        });

        let result = receiver
            .recv_timeout(Duration::from_secs(5))
            .expect("consumer failure must not deadlock worker shutdown");
        worker.join().expect("join fixture scan thread");
        assert!(matches!(
            result,
            Err(FastAnalysisScanError::Consumer(error)) if error == "expected"
        ));
    }
}
