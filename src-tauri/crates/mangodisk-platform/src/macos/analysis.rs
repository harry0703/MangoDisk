use std::{
    fs, io,
    os::macos::fs::MetadataExt as MacOsMetadataExt,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc::{self, RecvTimeoutError},
        Arc, Condvar, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

use crate::analysis_file_candidates::AnalysisFileCandidates;
use crate::{
    FastAnalysisFile, FastAnalysisRecord, FastAnalysisScanError, FastAnalysisSummary, Platform,
    ScanPurpose,
};

use super::{
    bulk_directory::{
        worker_count, AlignedBuffer, BulkDirectory, BulkDirectoryEntry, VNODE_TYPE_DIRECTORY,
        VNODE_TYPE_REGULAR_FILE,
    },
    is_dataless_flags, MacOsPlatform,
};

const RESULT_POLL_INTERVAL: Duration = Duration::from_millis(40);
const MAX_DIRECTORY_WORKERS: usize = 8;
static ACTIVE_ANALYSIS_REAPERS: AtomicUsize = AtomicUsize::new(0);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct DirectoryTotals {
    logical_bytes: u64,
    allocated_bytes: u64,
    file_count: u64,
    direct_file_count: u64,
    skipped_count: u64,
}

impl DirectoryTotals {
    /// Selects the progress unit promised by the caller-facing scan purpose.
    ///
    /// Disk analysis and large-file discovery describe real disk consumption, while exact
    /// duplicate grouping is keyed by logical length. Keeping this choice beside the native
    /// totals prevents platform-specific scanners from silently presenting different units.
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
        self.direct_file_count += u64::from(allocated_bytes > 0);
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

#[derive(Default)]
struct AnalysisDiagnostics {
    page_count: u64,
    entry_count: u64,
    directory_count: u64,
    candidate_count: u64,
    returned_bytes: u64,
    consumer_elapsed: Duration,
    remote_file_count: u64,
    remote_directory_count: u64,
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
    hard_links: Vec<FastAnalysisRecord>,
    analysis_files: AnalysisFileCandidates,
    page_count: u64,
    entry_count: u64,
    returned_bytes: u64,
    remote_file_count: u64,
    remote_directory_count: u64,
}

struct EntryPolicy<'a> {
    name_exclusions: &'a crate::NameExclusions,
    excluded_roots: &'a [PathBuf],
    platform: &'a MacOsPlatform,
    root: &'a Path,
    root_device: u64,
    purpose: ScanPurpose,
    should_prune_directory: fn(&Path) -> bool,
    large_file_minimum_bytes: u64,
}

/// Groups one native analysis request so the platform boundary remains explicit as scan options
/// evolve. The consumer stays separate because it owns the streamed result lifetime.
pub(super) struct AnalysisScanRequest<'a> {
    pub(super) retained_file_limit: usize,
    pub(super) name_exclusions: &'a crate::NameExclusions,
    pub(super) excluded_roots: &'a [PathBuf],
    pub(super) root: &'a Path,
    pub(super) purpose: ScanPurpose,
    pub(super) large_file_minimum_bytes: u64,
    pub(super) is_cancelled: &'a (dyn Fn() -> bool + Sync),
    pub(super) should_prune_directory: fn(&Path) -> bool,
    pub(super) report_progress: &'a mut dyn FnMut(&Path, u64, u64),
}

#[derive(Clone)]
struct DirectoryReadPolicy {
    retained_file_limit: usize,
    name_exclusions: crate::NameExclusions,
    excluded_roots: Arc<[PathBuf]>,
    root: Arc<PathBuf>,
    root_device: u64,
    purpose: ScanPurpose,
    should_prune_directory: fn(&Path) -> bool,
    large_file_minimum_bytes: u64,
}

#[derive(Default)]
struct DirectoryReadAccumulator {
    totals: DirectoryTotals,
    child_directories: Vec<PathBuf>,
    candidates: Vec<PathBuf>,
    hard_links: Vec<FastAnalysisRecord>,
    analysis_files: AnalysisFileCandidates,
    remote_file_count: u64,
    remote_directory_count: u64,
}

#[derive(Debug)]
struct PendingDirectory {
    retained_file_limit: usize,
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

struct AnalysisCoordinator<'a> {
    pending_nodes: Vec<Option<PendingDirectory>>,
    outstanding_tasks: usize,
    task_queue: &'a DirectoryTaskQueue,
    consumer: &'a mut dyn FnMut(FastAnalysisRecord) -> Result<(), String>,
    report_progress: &'a mut dyn FnMut(&Path, u64, u64),
    purpose: ScanPurpose,
    diagnostics: AnalysisDiagnostics,
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
                retained_file_limit: 0,
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
            diagnostics: AnalysisDiagnostics::default(),
            root_totals: None,
        }
    }

    fn consume(&mut self, result: DirectoryReadResult) -> Result<(), FastAnalysisScanError> {
        self.diagnostics.page_count = self
            .diagnostics
            .page_count
            .checked_add(result.page_count)
            .ok_or_else(|| platform_error("page_count_overflow"))?;
        self.diagnostics.entry_count = self
            .diagnostics
            .entry_count
            .checked_add(result.entry_count)
            .ok_or_else(|| platform_error("entry_count_overflow"))?;
        self.diagnostics.returned_bytes = self
            .diagnostics
            .returned_bytes
            .checked_add(result.returned_bytes)
            .ok_or_else(|| platform_error("returned_bytes_overflow"))?;
        self.diagnostics.remote_file_count = self
            .diagnostics
            .remote_file_count
            .saturating_add(result.remote_file_count);
        self.diagnostics.remote_directory_count = self
            .diagnostics
            .remote_directory_count
            .saturating_add(result.remote_directory_count);
        // Report only files directly contained by this completed directory. Descendant totals are
        // merged later during post-order finalization, so using direct values keeps live progress
        // exact without counting the same subtree more than once.
        (self.report_progress)(
            &result.task.path,
            result.direct_totals.file_count,
            result.direct_totals.progress_bytes(self.purpose),
        );
        for record in result.hard_links {
            (self.consumer)(record).map_err(FastAnalysisScanError::Consumer)?;
        }
        let retained_file_limit = result.analysis_files.limit();
        for mut file in result.analysis_files.into_files() {
            file.parent_file_count = result.direct_totals.direct_file_count;
            (self.consumer)(FastAnalysisRecord::AnalysisFile(file))
                .map_err(FastAnalysisScanError::Consumer)?;
        }
        for candidate in result.candidates {
            emit_candidate(candidate, self.consumer, &mut self.diagnostics)?;
        }

        let child_count = result.child_directories.len();
        let node = self
            .pending_nodes
            .get_mut(result.task.node_id)
            .and_then(Option::as_mut)
            .ok_or_else(|| platform_error("directory_node_missing"))?;
        node.retained_file_limit = retained_file_limit;
        node.totals = result.direct_totals;
        node.pending_children = child_count;
        node.has_been_read = true;

        let mut child_tasks = Vec::with_capacity(child_count);
        for path in result.child_directories {
            let node_id = self.pending_nodes.len();
            self.pending_nodes.push(Some(PendingDirectory {
                retained_file_limit: 0,
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
    _platform: &MacOsPlatform,
    request: AnalysisScanRequest<'_>,
    consumer: &mut dyn FnMut(FastAnalysisRecord) -> Result<(), String>,
) -> Result<FastAnalysisSummary, FastAnalysisScanError> {
    ensure_worker_pool_available(&ACTIVE_ANALYSIS_REAPERS)?;
    let AnalysisScanRequest {
        retained_file_limit,
        name_exclusions,
        excluded_roots,
        root,
        purpose,
        large_file_minimum_bytes,
        is_cancelled,
        should_prune_directory,
        report_progress,
    } = request;
    if is_cancelled() {
        return Err(FastAnalysisScanError::Cancelled);
    }
    let root_metadata =
        fs::symlink_metadata(root).map_err(|error| platform_io_error("root_metadata", &error))?;
    if !root_metadata.is_dir() || root_metadata.file_type().is_symlink() {
        return Err(platform_error("root_is_not_a_physical_directory"));
    }
    if is_dataless_flags(MacOsMetadataExt::st_flags(&root_metadata)) {
        // Checking the root before opening it prevents getattrlistbulk from materializing a
        // dataless File Provider directory. The log intentionally records only a stable reason
        // code and entry kind so cloud safety incidents remain auditable without exposing paths.
        log::info!(
            "macos_native_analysis_blocked reason=remote_placeholder entry_kind=root_directory"
        );
        return Err(platform_error("root_is_remote_placeholder"));
    }
    let worker_count = worker_count(MAX_DIRECTORY_WORKERS);
    let task_queue = Arc::new(DirectoryTaskQueue::default());
    let abort = Arc::new(AtomicBool::new(false));
    let read_policy = DirectoryReadPolicy {
        retained_file_limit,
        name_exclusions: name_exclusions.clone(),
        excluded_roots: Arc::from(excluded_roots),
        root: Arc::new(root.to_path_buf()),
        root_device: root_metadata.dev(),
        purpose,
        should_prune_directory,
        large_file_minimum_bytes,
    };
    let (result_sender, result_receiver) = mpsc::channel();
    let mut workers = Vec::with_capacity(worker_count);
    for worker_index in 0..worker_count {
        let worker_task_queue = Arc::clone(&task_queue);
        let worker_abort = Arc::clone(&abort);
        let worker_read_policy = read_policy.clone();
        let result_sender = result_sender.clone();
        let worker = thread::Builder::new()
            .name(format!("mangodisk-analysis-{worker_index}"))
            .spawn(move || {
                let platform = MacOsPlatform;
                let mut buffer = AlignedBuffer::new();
                while let Some(task) = worker_task_queue.pop() {
                    if worker_abort.load(Ordering::Relaxed) {
                        break;
                    }
                    let result = read_directory(
                        &platform,
                        &worker_read_policy,
                        task,
                        &worker_abort,
                        &mut buffer,
                    );
                    if result_sender.send(result).is_err() {
                        break;
                    }
                }
            });
        match worker {
            Ok(worker) => workers.push(worker),
            Err(error) => {
                // A partial pool owns queue and channel clones. Stop and join it before returning
                // so a rare thread-creation failure cannot leave detached filesystem readers
                // running after core falls back to generic traversal.
                abort.store(true, Ordering::Relaxed);
                task_queue.stop();
                for worker in workers {
                    let _ = worker.join();
                }
                return Err(platform_io_error("spawn_analysis_worker", &error));
            }
        }
    }
    drop(result_sender);

    let mut coordinator = AnalysisCoordinator::new(
        root,
        purpose,
        task_queue.as_ref(),
        report_progress,
        consumer,
    );
    task_queue.push_many([DirectoryTask {
        node_id: 0,
        path: root.to_path_buf(),
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
        let read_result = match read_result {
            Ok(result) => result,
            Err(error) => {
                scan_result = Err(error);
                break;
            }
        };
        if let Err(error) = coordinator.consume(read_result) {
            scan_result = Err(error);
            break;
        }
    }

    abort.store(true, Ordering::Relaxed);
    task_queue.stop();
    finish_workers(workers, &mut scan_result);
    scan_result?;
    let totals = coordinator
        .root_totals
        .ok_or_else(|| platform_error("root_totals_missing"))?;
    if coordinator.diagnostics.remote_file_count > 0
        || coordinator.diagnostics.remote_directory_count > 0
    {
        log::info!(
            "macos_native_analysis_remote_placeholders_skipped file_count={} directory_count={} total_count={}",
            coordinator.diagnostics.remote_file_count,
            coordinator.diagnostics.remote_directory_count,
            coordinator
                .diagnostics
                .remote_file_count
                .saturating_add(coordinator.diagnostics.remote_directory_count)
        );
    }
    Ok(FastAnalysisSummary {
        root_logical_bytes: totals.logical_bytes,
        root_allocated_bytes: totals.allocated_bytes,
        root_file_count: totals.file_count,
        root_skipped_count: totals.skipped_count,
        page_count: coordinator.diagnostics.page_count,
        entry_count: coordinator.diagnostics.entry_count,
        directory_count: coordinator.diagnostics.directory_count,
        candidate_count: coordinator.diagnostics.candidate_count,
        returned_bytes: coordinator.diagnostics.returned_bytes,
        consumer_elapsed_ms: u64::try_from(coordinator.diagnostics.consumer_elapsed.as_millis())
            .unwrap_or(u64::MAX),
        strategy: "darwin_parallel_getattrlistbulk_allocated_files_v4",
    })
}

fn ensure_worker_pool_available(active_reapers: &AtomicUsize) -> Result<(), FastAnalysisScanError> {
    let active_reaper_count = active_reapers.load(Ordering::Acquire);
    if active_reaper_count == 0 {
        return Ok(());
    }
    log::info!(
        "macos_native_analysis_deferred reason=workers_reaping active_reaper_count={active_reaper_count}"
    );
    Err(FastAnalysisScanError::Busy)
}

/// Finishes the native reader pool without making a failed or cancelled request wait on a
/// filesystem call that the process cannot interrupt. Darwin may keep `open` or
/// `getattrlistbulk` inside the kernel while a directory provider is unresponsive. Successful
/// scans still join synchronously because their summary depends on every worker having exited.
/// Failed scans transfer the handles to one bounded reaper thread so the UI and operation
/// scheduler can settle promptly while operating-system resources are reclaimed when the calls
/// eventually return.
fn finish_workers(
    workers: Vec<thread::JoinHandle<()>>,
    scan_result: &mut Result<(), FastAnalysisScanError>,
) {
    finish_workers_with_reaper_count(workers, scan_result, &ACTIVE_ANALYSIS_REAPERS);
}

fn finish_workers_with_reaper_count(
    workers: Vec<thread::JoinHandle<()>>,
    scan_result: &mut Result<(), FastAnalysisScanError>,
    active_reapers: &'static AtomicUsize,
) {
    if scan_result.is_ok() {
        for worker in workers {
            if worker.join().is_err() && scan_result.is_ok() {
                *scan_result = Err(platform_error("analysis_worker_panicked"));
            }
        }
        return;
    }

    let worker_count = workers.len();
    let reason = match scan_result {
        Err(FastAnalysisScanError::Cancelled) => "cancelled",
        Err(FastAnalysisScanError::Busy) => "busy",
        Err(FastAnalysisScanError::Consumer(_)) => "consumer_rejected",
        Err(FastAnalysisScanError::Platform(_)) => "platform_error",
        Ok(()) => unreachable!("successful scans join workers synchronously"),
    };
    let active_reaper_count = active_reapers.fetch_add(1, Ordering::AcqRel) + 1;
    log::info!("macos_native_analysis_workers_reaping worker_count={worker_count} reason={reason}");
    let started = Instant::now();
    if let Err(error) = thread::Builder::new()
        .name("mangodisk-analysis-reaper".to_string())
        .spawn(move || {
            let panicked_count = workers
                .into_iter()
                .map(|worker| usize::from(worker.join().is_err()))
                .sum::<usize>();
            // State transitions must not live inside a log argument: disabled log levels are
            // allowed to skip argument evaluation entirely.
            let remaining_reaper_count = active_reapers
                .fetch_sub(1, Ordering::AcqRel)
                .saturating_sub(1);
            log::info!(
                "macos_native_analysis_workers_reaped worker_count={} panicked_count={} active_reaper_count={} elapsed_ms={}",
                worker_count,
                panicked_count,
                remaining_reaper_count,
                started.elapsed().as_millis()
            );
        })
    {
        // A failed reaper spawn drops the join handles, which safely detaches the readers. The
        // active count intentionally remains non-zero because those detached readers can no
        // longer be observed; failing closed prevents later scans from creating unbounded worker
        // pools. Keep the diagnostic compact and avoid exposing any scanned path.
        log::warn!(
            "macos_native_analysis_reaper_spawn_failed worker_count={} io_kind={:?}",
            worker_count,
            error.kind()
        );
    } else {
        log::debug!(
            "macos_native_analysis_reaper_started active_reaper_count={active_reaper_count}"
        );
    }
}

fn read_directory(
    platform: &MacOsPlatform,
    policy: &DirectoryReadPolicy,
    task: DirectoryTask,
    abort: &AtomicBool,
    buffer: &mut AlignedBuffer,
) -> Result<DirectoryReadResult, FastAnalysisScanError> {
    check_aborted(abort)?;
    let directory = match BulkDirectory::open(&task.path) {
        Ok(directory) => directory,
        Err(_error) if !task.is_root => {
            return Ok(DirectoryReadResult {
                task,
                direct_totals: DirectoryTotals {
                    skipped_count: 1,
                    ..DirectoryTotals::default()
                },
                child_directories: Vec::new(),
                candidates: Vec::new(),
                hard_links: Vec::new(),
                analysis_files: AnalysisFileCandidates::default(),
                page_count: 0,
                entry_count: 0,
                returned_bytes: 0,
                remote_file_count: 0,
                remote_directory_count: 0,
            });
        }
        Err(error) => return Err(platform_io_error("open_root_directory", &error)),
    };
    let retained_file_limit = policy.retained_file_limit;
    let policy = EntryPolicy {
        name_exclusions: &policy.name_exclusions,
        excluded_roots: &policy.excluded_roots,
        platform,
        root: policy.root.as_path(),
        root_device: policy.root_device,
        purpose: policy.purpose,
        should_prune_directory: policy.should_prune_directory,
        large_file_minimum_bytes: policy.large_file_minimum_bytes,
    };
    let mut accumulator = DirectoryReadAccumulator {
        analysis_files: AnalysisFileCandidates::new(retained_file_limit),
        ..DirectoryReadAccumulator::default()
    };
    let mut page_count = 0_u64;
    let mut entry_count_total = 0_u64;
    let mut returned_bytes = 0_u64;

    loop {
        check_aborted(abort)?;
        let entries = directory
            .read_page(buffer)
            .map_err(|error| platform_io_error("read_directory_attributes", &error))?;
        if entries.is_empty() {
            break;
        }
        page_count = page_count
            .checked_add(1)
            .ok_or_else(|| platform_error("page_count_overflow"))?;
        for entry in entries {
            check_aborted(abort)?;
            entry_count_total = entry_count_total
                .checked_add(1)
                .ok_or_else(|| platform_error("entry_count_overflow"))?;
            returned_bytes = returned_bytes
                .checked_add(u64::try_from(entry.record_length).unwrap_or(u64::MAX))
                .ok_or_else(|| platform_error("returned_bytes_overflow"))?;
            process_entry(&policy, &task.path, entry, &mut accumulator)?;
        }
    }

    Ok(DirectoryReadResult {
        task,
        direct_totals: accumulator.totals,
        child_directories: accumulator.child_directories,
        candidates: accumulator.candidates,
        hard_links: accumulator.hard_links,
        analysis_files: accumulator.analysis_files,
        page_count,
        entry_count: entry_count_total,
        returned_bytes,
        remote_file_count: accumulator.remote_file_count,
        remote_directory_count: accumulator.remote_directory_count,
    })
}

fn process_entry(
    policy: &EntryPolicy<'_>,
    parent: &Path,
    entry: BulkDirectoryEntry,
    accumulator: &mut DirectoryReadAccumulator,
) -> Result<(), FastAnalysisScanError> {
    if entry.attribute_error != 0 || entry.name.as_encoded_bytes().is_empty() {
        return accumulator.totals.skip_entry();
    }
    let path = parent.join(entry.name);
    if policy
        .platform
        .should_skip(&path, policy.root, policy.purpose)
        .is_some()
    {
        return accumulator.totals.skip_entry();
    }
    if entry.device != policy.root_device || entry.mount_status & libc::DIR_MNTSTATUS_MNTPOINT != 0
    {
        return accumulator.totals.skip_entry();
    }

    if policy
        .name_exclusions
        .matches_entry(&path, entry.object_type == VNODE_TYPE_DIRECTORY)
    {
        return Ok(());
    }
    match entry.object_type {
        VNODE_TYPE_DIRECTORY => {
            if is_dataless_flags(entry.flags) {
                // Dataless directories are traversal boundaries, not ordinary empty folders.
                // Enqueueing one would make the next getattrlistbulk call fetch its remote listing.
                accumulator.remote_directory_count =
                    accumulator.remote_directory_count.saturating_add(1);
                return accumulator.totals.skip_entry();
            }
            if policy
                .excluded_roots
                .iter()
                .any(|root| path.starts_with(root))
            {
                // This subtree belongs to another selected root or an explicit exclusion;
                // it is not an inaccessible entry and must not inflate failure statistics.
                return Ok(());
            }
            if (policy.should_prune_directory)(&path) {
                return accumulator.totals.skip_entry();
            }
            accumulator.child_directories.push(path);
            Ok(())
        }
        VNODE_TYPE_REGULAR_FILE => {
            if is_dataless_flags(entry.flags) {
                // The logical length belongs to remote content. Counting it as local usage makes
                // disk analysis exceed the selected volume's occupied space, while opening it for
                // large-file or duplicate classification can materialize the file. The bulk page
                // already contains `st_flags`, so this protection adds no per-entry syscall.
                accumulator.remote_file_count = accumulator.remote_file_count.saturating_add(1);
                return accumulator.totals.skip_entry();
            }
            accumulator
                .totals
                .add_file(entry.logical_bytes, entry.allocated_bytes)?;
            if policy.purpose == ScanPurpose::Analysis && entry.link_count > 1 {
                accumulator
                    .hard_links
                    .push(FastAnalysisRecord::HardLinkedFile {
                        path: path.clone(),
                        identity: crate::PhysicalFileIdentity {
                            volume: entry.device,
                            index: entry.file_id,
                        },
                        logical_bytes: entry.logical_bytes,
                        allocated_bytes: entry.allocated_bytes,
                        modified_at_ms: entry.modified_at_ms,
                    });
            }
            let candidate_purpose = match policy.purpose {
                ScanPurpose::DuplicateFiles => ScanPurpose::DuplicateFiles,
                _ => ScanPurpose::LargeFiles,
            };
            let candidate_bytes = match policy.purpose {
                ScanPurpose::DuplicateFiles => entry.logical_bytes,
                _ => entry.allocated_bytes,
            };
            if policy.purpose == ScanPurpose::Analysis
                && entry.link_count <= 1
                && entry.allocated_bytes > 0
                && accumulator
                    .analysis_files
                    .would_retain(entry.allocated_bytes, &path)
                && policy
                    .platform
                    .should_skip(&path, policy.root, ScanPurpose::Analysis)
                    .is_none()
            {
                accumulator.analysis_files.push(FastAnalysisFile {
                    parent_file_count: 0,
                    path: path.clone(),
                    allocated_bytes: entry.allocated_bytes,
                    logical_bytes: entry.logical_bytes,
                    modified_at_ms: entry.modified_at_ms,
                });
            }
            if candidate_bytes >= policy.large_file_minimum_bytes
                && policy
                    .platform
                    .should_skip(&path, policy.root, candidate_purpose)
                    .is_none()
            {
                accumulator.candidates.push(path);
            }
            Ok(())
        }
        _ => accumulator.totals.skip_entry(),
    }
}

fn finalize_directory_chain(
    mut node_id: usize,
    pending_nodes: &mut [Option<PendingDirectory>],
    consumer: &mut dyn FnMut(FastAnalysisRecord) -> Result<(), String>,
    diagnostics: &mut AnalysisDiagnostics,
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
        emit_directory(
            &node.path,
            node.totals,
            node.retained_file_limit,
            consumer,
            diagnostics,
        )?;
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
    retained_file_limit: usize,
    consumer: &mut dyn FnMut(FastAnalysisRecord) -> Result<(), String>,
    diagnostics: &mut AnalysisDiagnostics,
) -> Result<(), FastAnalysisScanError> {
    let started = Instant::now();
    consumer(FastAnalysisRecord::Directory {
        retained_file_limit,
        path: path.to_path_buf(),
        logical_bytes: totals.logical_bytes,
        allocated_bytes: totals.allocated_bytes,
        file_count: totals.file_count,
        direct_file_count: totals.direct_file_count,
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
    diagnostics: &mut AnalysisDiagnostics,
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

fn platform_io_error(operation: &str, error: &io::Error) -> FastAnalysisScanError {
    FastAnalysisScanError::Platform(format!("{operation}:io_kind={:?}", error.kind()))
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        fs::File,
        io::Write,
        os::unix::fs::symlink,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;

    static ANALYSIS_TEST_LOCK: Mutex<()> = Mutex::new(());

    fn analysis_test_lock() -> std::sync::MutexGuard<'static, ()> {
        ANALYSIS_TEST_LOCK
            .lock()
            // The lock only isolates tests that intentionally share the production reaper
            // counter. Recovering a poisoned guard prevents one failed assertion from making
            // every later native analysis test fail with an unrelated Busy result.
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn wait_for_analysis_reapers() {
        for _ in 0..1_000 {
            if ACTIVE_ANALYSIS_REAPERS.load(Ordering::Acquire) == 0 {
                return;
            }
            thread::sleep(Duration::from_millis(1));
        }
        panic!("the native analysis test reaper did not finish");
    }

    #[test]
    fn selected_subtree_is_pruned_without_counting_it_as_unavailable() {
        let root = Path::new("/fixture");
        let excluded = [root.join("selected")];
        let policy = EntryPolicy {
            name_exclusions: &crate::NameExclusions::default(),
            excluded_roots: &excluded,
            platform: &MacOsPlatform,
            root,
            root_device: 7,
            purpose: ScanPurpose::LargeFiles,
            should_prune_directory: |_| false,
            large_file_minimum_bytes: 1,
        };
        let mut accumulator = DirectoryReadAccumulator::default();
        for name in ["selected", "selected-other"] {
            let entry = BulkDirectoryEntry {
                name: name.into(),
                device: 7,
                file_id: 1,
                link_count: 1,
                object_type: VNODE_TYPE_DIRECTORY,
                mount_status: 0,
                flags: 0,
                logical_bytes: 0,
                allocated_bytes: 0,
                modified_at_ms: None,
                attribute_error: 0,
                record_length: 64,
            };
            process_entry(&policy, root, entry, &mut accumulator).unwrap();
        }
        assert_eq!(
            accumulator.child_directories,
            vec![root.join("selected-other")]
        );
        assert_eq!(accumulator.totals.skipped_count, 0);
    }

    #[test]
    fn dataless_directory_is_skipped_before_it_enters_the_worker_queue() {
        let root = Path::new("/fixture");
        let policy = EntryPolicy {
            name_exclusions: &crate::NameExclusions::default(),
            excluded_roots: &[],
            platform: &MacOsPlatform,
            root,
            root_device: 7,
            purpose: ScanPurpose::Analysis,
            should_prune_directory: |_| false,
            large_file_minimum_bytes: 1,
        };
        let entry = BulkDirectoryEntry {
            name: "remote-directory".into(),
            device: 7,
            file_id: 1,
            link_count: 1,
            object_type: VNODE_TYPE_DIRECTORY,
            mount_status: 0,
            flags: super::super::SF_DATALESS,
            logical_bytes: 0,
            allocated_bytes: 0,
            modified_at_ms: None,
            attribute_error: 0,
            record_length: 64,
        };
        let mut accumulator = DirectoryReadAccumulator::default();

        process_entry(&policy, root, entry, &mut accumulator)
            .expect("dataless directory should be safely skipped");

        assert!(accumulator.child_directories.is_empty());
        assert_eq!(accumulator.totals.skipped_count, 1);
        assert_eq!(accumulator.remote_directory_count, 1);
        assert_eq!(accumulator.remote_file_count, 0);
    }

    #[test]
    fn native_analysis_reports_allocated_and_logical_sizes_and_skips_links() {
        let _analysis_lock = analysis_test_lock();
        let root = unique_fixture_root("logical-size");
        fs::create_dir_all(root.join("nested/deeper")).expect("create fixture directories");
        write_bytes(&root.join("root.bin"), 13);
        write_bytes(&root.join("nested/child.bin"), 29);
        write_bytes(&root.join("nested/deeper/large.bin"), 71);
        symlink(root.join("root.bin"), root.join("linked.bin")).expect("create fixture link");
        let root_allocated = MacOsPlatform
            .file_space_usage(
                &root.join("root.bin"),
                &fs::metadata(root.join("root.bin")).expect("read root fixture metadata"),
            )
            .allocated_bytes;
        let child_allocated = MacOsPlatform
            .file_space_usage(
                &root.join("nested/child.bin"),
                &fs::metadata(root.join("nested/child.bin")).expect("read child fixture metadata"),
            )
            .allocated_bytes;
        let large_allocated = MacOsPlatform
            .file_space_usage(
                &root.join("nested/deeper/large.bin"),
                &fs::metadata(root.join("nested/deeper/large.bin"))
                    .expect("read large fixture metadata"),
            )
            .allocated_bytes;

        let mut directories = HashMap::new();
        let mut candidates = Vec::new();
        let mut progress_batches = Vec::new();
        let summary = analyze_records(
            &MacOsPlatform,
            AnalysisScanRequest {
                retained_file_limit: 64,
                name_exclusions: &crate::NameExclusions::default(),
                excluded_roots: &[],
                root: &root,
                purpose: ScanPurpose::Analysis,
                large_file_minimum_bytes: 50,
                is_cancelled: &|| false,
                should_prune_directory: |_| false,
                report_progress: &mut |path, file_count, bytes| {
                    progress_batches.push((path.to_path_buf(), file_count, bytes));
                },
            },
            &mut |record| {
                match record {
                    FastAnalysisRecord::Directory {
                        path,
                        logical_bytes,
                        allocated_bytes,
                        file_count,
                        skipped_count,
                        ..
                    } => {
                        directories.insert(
                            path,
                            (logical_bytes, allocated_bytes, file_count, skipped_count),
                        );
                    }
                    FastAnalysisRecord::LargeFileCandidate(path) => candidates.push(path),
                    FastAnalysisRecord::HardLinkedFile { .. }
                    | FastAnalysisRecord::AnalysisFile(_) => {}
                }
                Ok(())
            },
        )
        .expect("scan fixture");

        assert_eq!(summary.root_logical_bytes, 113);
        assert_eq!(
            summary.root_allocated_bytes,
            root_allocated + child_allocated + large_allocated
        );
        assert_eq!(summary.root_file_count, 3);
        assert_eq!(summary.root_skipped_count, 1);
        assert_eq!(summary.directory_count, 3);
        let mut expected_candidates = [
            (root.join("root.bin"), root_allocated),
            (root.join("nested/child.bin"), child_allocated),
            (root.join("nested/deeper/large.bin"), large_allocated),
        ]
        .into_iter()
        .filter_map(|(path, allocated_bytes)| (allocated_bytes >= 50).then_some(path))
        .collect::<Vec<_>>();
        expected_candidates.sort();
        candidates.sort();
        assert_eq!(summary.candidate_count, expected_candidates.len() as u64);
        assert_eq!(
            directories.get(&root),
            Some(&(
                113,
                root_allocated + child_allocated + large_allocated,
                3,
                1
            ))
        );
        assert_eq!(
            directories.get(&root.join("nested")),
            Some(&(100, child_allocated + large_allocated, 2, 0))
        );
        assert_eq!(
            directories.get(&root.join("nested/deeper")),
            Some(&(71, large_allocated, 1, 0))
        );
        assert_eq!(candidates, expected_candidates);
        assert_eq!(
            progress_batches
                .iter()
                .map(|(_, file_count, _)| file_count)
                .sum::<u64>(),
            summary.root_file_count
        );
        assert_eq!(
            progress_batches
                .iter()
                .map(|(_, _, bytes)| bytes)
                .sum::<u64>(),
            summary.root_allocated_bytes
        );

        fs::remove_dir_all(root).expect("remove fixture");
    }

    #[test]
    fn native_analysis_does_not_charge_sparse_holes_to_disk_usage() {
        let _analysis_lock = analysis_test_lock();
        let root = unique_fixture_root("sparse-allocation");
        fs::create_dir_all(&root).expect("create sparse fixture directory");
        let path = root.join("sparse.bin");
        fs::File::create(&path)
            .expect("create sparse fixture")
            .set_len(64 * 1024 * 1024)
            .expect("extend sparse fixture");

        let mut candidates = Vec::new();
        let summary = analyze_records(
            &MacOsPlatform,
            AnalysisScanRequest {
                retained_file_limit: 64,
                name_exclusions: &crate::NameExclusions::default(),
                excluded_roots: &[],
                root: &root,
                purpose: ScanPurpose::Analysis,
                large_file_minimum_bytes: 1,
                is_cancelled: &|| false,
                should_prune_directory: |_| false,
                report_progress: &mut |_, _, _| {},
            },
            &mut |record| {
                if let FastAnalysisRecord::LargeFileCandidate(path) = record {
                    candidates.push(path);
                }
                Ok(())
            },
        )
        .expect("scan sparse fixture");

        assert_eq!(summary.root_logical_bytes, 64 * 1024 * 1024);
        assert!(summary.root_allocated_bytes < summary.root_logical_bytes);
        assert!(candidates.is_empty());

        let mut duplicate_candidates = Vec::new();
        let mut duplicate_progress_bytes = 0_u64;
        analyze_records(
            &MacOsPlatform,
            AnalysisScanRequest {
                retained_file_limit: 64,
                name_exclusions: &crate::NameExclusions::default(),
                excluded_roots: &[],
                root: &root,
                purpose: ScanPurpose::DuplicateFiles,
                large_file_minimum_bytes: 1,
                is_cancelled: &|| false,
                should_prune_directory: |_| false,
                report_progress: &mut |_, _, bytes| {
                    duplicate_progress_bytes = duplicate_progress_bytes.saturating_add(bytes);
                },
            },
            &mut |record| {
                if let FastAnalysisRecord::LargeFileCandidate(path) = record {
                    duplicate_candidates.push(path);
                }
                Ok(())
            },
        )
        .expect("scan the sparse duplicate fixture");
        assert_eq!(duplicate_candidates, vec![path]);
        assert_eq!(duplicate_progress_bytes, 64 * 1024 * 1024);
        fs::remove_dir_all(root).expect("remove sparse fixture");
    }

    #[test]
    fn native_analysis_honors_cancellation_before_opening_root() {
        let _analysis_lock = analysis_test_lock();
        let result = analyze_records(
            &MacOsPlatform,
            AnalysisScanRequest {
                retained_file_limit: 64,
                name_exclusions: &crate::NameExclusions::default(),
                excluded_roots: &[],
                root: Path::new("/does-not-need-to-exist"),
                purpose: ScanPurpose::Analysis,
                large_file_minimum_bytes: 1,
                is_cancelled: &|| true,
                should_prune_directory: |_| false,
                report_progress: &mut |_, _, _| {},
            },
            &mut |_| Ok(()),
        );
        assert!(matches!(result, Err(FastAnalysisScanError::Cancelled)));
    }

    #[test]
    fn native_analysis_stops_workers_when_consumer_rejects_a_record() {
        let _analysis_lock = analysis_test_lock();
        let root = unique_fixture_root("consumer-error");
        fs::create_dir_all(root.join("nested/deeper")).expect("create fixture directories");
        write_bytes(&root.join("nested/deeper/large.bin"), 71);

        let result = analyze_records(
            &MacOsPlatform,
            AnalysisScanRequest {
                retained_file_limit: 64,
                name_exclusions: &crate::NameExclusions::default(),
                excluded_roots: &[],
                root: &root,
                purpose: ScanPurpose::Analysis,
                large_file_minimum_bytes: 1,
                is_cancelled: &|| false,
                should_prune_directory: |_| false,
                report_progress: &mut |_, _, _| {},
            },
            &mut |_| Err("fixture consumer rejected record".to_string()),
        );

        assert!(matches!(
            result,
            Err(FastAnalysisScanError::Consumer(error))
                if error == "fixture consumer rejected record"
        ));
        fs::remove_dir_all(root).expect("remove fixture");
        // Failed scans intentionally hand workers to an asynchronous reaper. Keep the shared
        // test lock until that production state is clean, otherwise another parallel test can
        // observe a legitimate Busy response caused by this fixture rather than its own input.
        wait_for_analysis_reapers();
    }

    #[test]
    fn failed_scan_does_not_wait_for_a_blocked_worker() {
        static TEST_ACTIVE_REAPERS: AtomicUsize = AtomicUsize::new(0);
        let (started_sender, started_receiver) = mpsc::channel();
        let worker = thread::spawn(move || {
            started_sender.send(()).expect("signal worker start");
            thread::sleep(Duration::from_millis(250));
        });
        started_receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("worker should start");
        let mut result = Err(FastAnalysisScanError::Cancelled);
        let started = Instant::now();

        finish_workers_with_reaper_count(vec![worker], &mut result, &TEST_ACTIVE_REAPERS);

        assert!(started.elapsed() < Duration::from_millis(100));
        assert!(matches!(result, Err(FastAnalysisScanError::Cancelled)));
        assert!(matches!(
            ensure_worker_pool_available(&TEST_ACTIVE_REAPERS),
            Err(FastAnalysisScanError::Busy)
        ));
        for _ in 0..1_000 {
            if TEST_ACTIVE_REAPERS.load(Ordering::Acquire) == 0 {
                return;
            }
            thread::sleep(Duration::from_millis(1));
        }
        panic!("the test reaper did not finish");
    }

    fn unique_fixture_root(label: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "mangodisk-bulk-analysis-{label}-{}-{unique}",
            std::process::id()
        ))
    }

    fn write_bytes(path: &Path, count: usize) {
        let mut file = File::create(path).expect("create fixture file");
        file.write_all(&vec![7; count]).expect("write fixture file");
    }
}
