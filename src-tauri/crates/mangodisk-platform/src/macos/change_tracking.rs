use std::{
    ffi::{c_void, CStr, OsString},
    fs,
    os::unix::{ffi::OsStrExt, ffi::OsStringExt, fs::MetadataExt},
    panic::{catch_unwind, AssertUnwindSafe},
    path::{Path, PathBuf},
    ptr::NonNull,
    sync::{
        atomic::{AtomicU8, Ordering},
        mpsc::{sync_channel, Receiver, RecvTimeoutError, SyncSender},
        Arc, Condvar, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use dispatch2::{DispatchQueue, DispatchQueueAttr, DispatchRetained};
use objc2_core_foundation::{CFArray, CFString};
use objc2_core_services::{
    kFSEventStreamCreateFlagNoDefer, kFSEventStreamCreateFlagWatchRoot,
    kFSEventStreamEventFlagEventIdsWrapped, kFSEventStreamEventFlagHistoryDone,
    kFSEventStreamEventFlagKernelDropped, kFSEventStreamEventFlagMount,
    kFSEventStreamEventFlagMustScanSubDirs, kFSEventStreamEventFlagRootChanged,
    kFSEventStreamEventFlagUnmount, kFSEventStreamEventFlagUserDropped, ConstFSEventStreamRef,
    FSEventStreamContext, FSEventStreamCreate, FSEventStreamFlushAsync, FSEventStreamInvalidate,
    FSEventStreamRef, FSEventStreamRelease, FSEventStreamSetDispatchQueue, FSEventStreamStart,
    FSEventStreamStop, FSEventsCopyUUIDForDevice, FSEventsGetCurrentEventId,
};

use crate::{
    FilesystemChangeImpactError, FilesystemChangeImpactOutcome, FilesystemChangeImpactPlan,
    FilesystemChangeImpactSummary, FilesystemChangeImpactUnavailable, FilesystemChangeMonitor,
    FilesystemChangeMonitorBackend, FilesystemChangeStatus, FilesystemChangeToken,
};

const QUERY_TIMEOUT: Duration = Duration::from_secs(2);
const POLL_INTERVAL: Duration = Duration::from_millis(25);
const OWNER_SHUTDOWN_TIMEOUT: Duration = Duration::from_millis(250);
// The monitor must catch up with history and become clean promptly, so event coalescing must not
// retain the default latency window.
const QUERY_LATENCY_SECONDS: f64 = 0.0;
const HISTORY_INVALID_FLAGS: u32 = kFSEventStreamEventFlagMustScanSubDirs
    | kFSEventStreamEventFlagUserDropped
    | kFSEventStreamEventFlagKernelDropped
    | kFSEventStreamEventFlagEventIdsWrapped
    | kFSEventStreamEventFlagRootChanged
    | kFSEventStreamEventFlagMount
    | kFSEventStreamEventFlagUnmount;
const MAX_IMPACT_EVENTS: u64 = 100_000;
const MAX_DIRTY_DIRECTORIES: usize = 4_096;
const MAX_DIRTY_PATH_BYTES: usize = 2 * 1024 * 1024;

const STATUS_PENDING: u8 = 0;
const STATUS_CLEAN: u8 = 1;
const STATUS_CHANGED: u8 = 2;
const STATUS_HISTORY_UNAVAILABLE: u8 = 3;

struct MonitorState {
    status: AtomicU8,
    wait_lock: Mutex<()>,
    notification: Condvar,
}

impl MonitorState {
    fn pending() -> Self {
        Self {
            status: AtomicU8::new(STATUS_PENDING),
            wait_lock: Mutex::new(()),
            notification: Condvar::new(),
        }
    }

    fn status(&self) -> FilesystemChangeStatus {
        match self.status.load(Ordering::Acquire) {
            STATUS_PENDING => FilesystemChangeStatus::Pending,
            STATUS_CLEAN => FilesystemChangeStatus::Clean,
            STATUS_CHANGED => FilesystemChangeStatus::Changed,
            _ => FilesystemChangeStatus::HistoryUnavailable,
        }
    }

    fn publish(&self, status: FilesystemChangeStatus) {
        let requested = status_code(status);
        loop {
            let current = self.status.load(Ordering::Acquire);
            // State may move from pending to its first conclusion or from clean to changed or
            // unavailable. Changed and history unavailable are fail-closed terminal states and
            // must never return to clean after a later HistoryDone event.
            let can_transition = current == STATUS_PENDING
                || (current == STATUS_CLEAN
                    && matches!(
                        status,
                        FilesystemChangeStatus::Changed
                            | FilesystemChangeStatus::HistoryUnavailable
                    ));
            if !can_transition || current == requested {
                return;
            }
            if self
                .status
                .compare_exchange(current, requested, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                self.notification.notify_all();
                return;
            }
        }
    }

    fn wait_until_ready(
        &self,
        deadline: Instant,
        is_cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<FilesystemChangeStatus, String> {
        let mut guard = self
            .wait_lock
            .lock()
            .map_err(|_| "FSEvents monitor state lock is poisoned".to_string())?;
        loop {
            let status = self.status();
            if status != FilesystemChangeStatus::Pending {
                return Ok(status);
            }
            if is_cancelled() {
                return Err("scan cancelled".to_string());
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                self.publish(FilesystemChangeStatus::HistoryUnavailable);
                log::warn!("filesystem_change_monitor_timed_out platform=macos");
                return Ok(FilesystemChangeStatus::HistoryUnavailable);
            }
            let wait = remaining.min(POLL_INTERVAL);
            let (next_guard, _) = self
                .notification
                .wait_timeout(guard, wait)
                .map_err(|_| "FSEvents monitor wait lock is poisoned".to_string())?;
            guard = next_guard;
        }
    }
}

fn status_code(status: FilesystemChangeStatus) -> u8 {
    match status {
        FilesystemChangeStatus::Pending => STATUS_PENDING,
        FilesystemChangeStatus::Clean => STATUS_CLEAN,
        FilesystemChangeStatus::Changed => STATUS_CHANGED,
        FilesystemChangeStatus::HistoryUnavailable => STATUS_HISTORY_UNAVAILABLE,
    }
}

struct CallbackContext {
    state: Arc<MonitorState>,
}

#[derive(Default)]
struct ImpactCollection {
    completed: bool,
    unavailable: Option<FilesystemChangeImpactUnavailable>,
    page_count: u64,
    record_count: u64,
    path_bytes: usize,
    dirty_directories: Vec<PathBuf>,
}

struct ImpactState {
    collection: Mutex<ImpactCollection>,
    notification: Condvar,
}

impl ImpactState {
    fn pending() -> Self {
        Self {
            collection: Mutex::new(ImpactCollection::default()),
            notification: Condvar::new(),
        }
    }

    fn fail(&self, reason: FilesystemChangeImpactUnavailable) {
        let mut collection = self
            .collection
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        collection.unavailable = Some(reason);
        collection.completed = true;
        self.notification.notify_all();
    }
}

struct ImpactCallbackContext {
    state: Arc<ImpactState>,
    root: PathBuf,
    start_cursor: u64,
    end_cursor: u64,
}

struct MonitorOwner {
    handle: JoinHandle<()>,
    completion_receiver: Receiver<()>,
}

struct MacOsMonitorBackend {
    state: Arc<MonitorState>,
    stop_sender: SyncSender<()>,
    owner: Mutex<Option<MonitorOwner>>,
}

/// The FSEvent stream and callback context form one lifecycle unit. This explicit RAII guard stops
/// the stream and drains its queue before releasing the callback context even if the owner panics
/// during startup, flush, or waiting, preventing CoreServices from accessing a dropped Rust value.
struct OwnedEventStream<C> {
    stream: FSEventStreamRef,
    queue: DispatchRetained<DispatchQueue>,
    _callback_context: Box<C>,
    started: bool,
}

impl<C> Drop for OwnedEventStream<C> {
    fn drop(&mut self) {
        // SAFETY: The owner thread exclusively owns the stream. The context remains a guard field
        // until Drop returns, and synchronously draining the serial queue guarantees that no
        // callback can use it afterward.
        unsafe {
            if self.started {
                FSEventStreamStop(self.stream);
            }
            FSEventStreamInvalidate(self.stream);
        }
        self.queue.exec_sync(|| {});
        unsafe {
            FSEventStreamRelease(self.stream);
        }
    }
}

impl FilesystemChangeMonitorBackend for MacOsMonitorBackend {
    fn status(&self) -> FilesystemChangeStatus {
        self.state.status()
    }

    fn continuously_tracks_changes(&self) -> bool {
        true
    }
}

impl Drop for MacOsMonitorBackend {
    fn drop(&mut self) {
        let started = Instant::now();
        let _ = self.stop_sender.try_send(());
        let owner = match self.owner.lock() {
            Ok(mut owner) => owner.take(),
            Err(_) => {
                log::error!("filesystem_change_monitor_owner_lock_poisoned platform=macos");
                None
            }
        };
        if let Some(owner) = owner {
            if owner.handle.thread().id() == thread::current().id() {
                log::error!("filesystem_change_monitor_self_join_prevented platform=macos");
            } else {
                match owner
                    .completion_receiver
                    .recv_timeout(OWNER_SHUTDOWN_TIMEOUT)
                {
                    Ok(()) | Err(RecvTimeoutError::Disconnected) => {
                        if owner.handle.join().is_err() {
                            log::error!("filesystem_change_monitor_owner_panicked platform=macos");
                        }
                    }
                    Err(RecvTimeoutError::Timeout) => {
                        // After detaching the JoinHandle, the owner still exclusively owns the
                        // stream, queue, context, and Arc state and can finish cleanup safely in the
                        // background. An abnormal CoreServices shutdown cannot block the caller
                        // indefinitely.
                        log::error!("filesystem_change_monitor_shutdown_timed_out platform=macos");
                    }
                }
            }
        }
        log::debug!(
            "filesystem_change_monitor_released platform=macos elapsed_ms={}",
            started.elapsed().as_millis()
        );
    }
}

/// Before a full scan, retain only the current host event boundary and volume UUID. An absolute-path
/// stream correctly covers macOS Data volume firmlinks such as `/Users`, while the UUID prevents a
/// reused device number from being mistaken for the original volume.
pub(super) fn capture_token(root: &Path) -> Result<Option<FilesystemChangeToken>, String> {
    // An external volume may contain per-host event IDs from another machine that overlap local
    // IDs. This absolute-path stream supports APFS firmlinks and is therefore enabled only for the
    // current host's system volume. External volumes keep using full scans until a per-device
    // stream is available; their history cannot be reused safely.
    if root == Path::new("/Volumes") || root.starts_with("/Volumes") {
        return Ok(None);
    }
    let started = Instant::now();
    let device = device_id(root)?;
    // Custom mount points need not live under /Volumes. Comparing device IDs prevents an external
    // volume from bypassing the path check and treating another host's overlapping per-host event
    // IDs as continuous local history.
    if device != device_id(Path::new("/"))? {
        return Ok(None);
    }
    let volume_id = volume_id_for_device(device)?;
    // SAFETY: This function takes no arguments and only reads fseventsd's current system event ID.
    let cursor = unsafe { FSEventsGetCurrentEventId() };
    log::debug!(
        "filesystem_change_token_captured platform=macos elapsed_ms={}",
        started.elapsed().as_millis()
    );
    Ok(Some(FilesystemChangeToken {
        volume_id,
        history_id: 0,
        cursor,
    }))
}

pub(super) fn impact_plan(
    root: &Path,
    token: &FilesystemChangeToken,
    is_cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<FilesystemChangeImpactOutcome, FilesystemChangeImpactError> {
    if is_cancelled() {
        return Err(FilesystemChangeImpactError::Cancelled);
    }
    let started = Instant::now();
    if root == Path::new("/Volumes") || root.starts_with("/Volumes") {
        return Ok(FilesystemChangeImpactOutcome::Unavailable(
            FilesystemChangeImpactUnavailable::UnsupportedRoot,
        ));
    }
    let root_device = device_id(root).map_err(FilesystemChangeImpactError::Platform)?;
    let system_device = device_id(Path::new("/")).map_err(FilesystemChangeImpactError::Platform)?;
    if root_device != system_device {
        return Ok(FilesystemChangeImpactOutcome::Unavailable(
            FilesystemChangeImpactUnavailable::UnsupportedRoot,
        ));
    }
    let volume_id =
        volume_id_for_device(root_device).map_err(FilesystemChangeImpactError::Platform)?;
    if volume_id != token.volume_id {
        return Ok(FilesystemChangeImpactOutcome::Unavailable(
            FilesystemChangeImpactUnavailable::HistoryUnavailable,
        ));
    }
    let Some(root_text) = root.to_str() else {
        return Ok(FilesystemChangeImpactOutcome::Unavailable(
            FilesystemChangeImpactUnavailable::UnsupportedRoot,
        ));
    };
    // Capture the upper boundary before starting the stream. Events created concurrently after
    // this point are ignored and remain available after the returned token, making the plan a
    // stable half-open history window instead of a moving target.
    let end_cursor = unsafe { FSEventsGetCurrentEventId() };
    if token.cursor > end_cursor {
        return Ok(FilesystemChangeImpactOutcome::Unavailable(
            FilesystemChangeImpactUnavailable::HistoryUnavailable,
        ));
    }
    if token.cursor == end_cursor {
        return Ok(complete_impact_plan(
            token,
            end_cursor,
            ImpactCollection::default(),
            started,
        ));
    }

    let state = Arc::new(ImpactState::pending());
    let path = CFString::from_str(root_text);
    let paths = CFArray::from_objects(&[&*path]);
    let erased_paths: &CFArray =
        unsafe { &*((&*paths as *const CFArray<CFString>).cast::<CFArray>()) };
    let queue = DispatchQueue::new(
        "app.mangodisk.analysis-change-impact",
        DispatchQueueAttr::SERIAL,
    );
    let mut callback_context = Box::new(ImpactCallbackContext {
        state: Arc::clone(&state),
        root: root.to_path_buf(),
        start_cursor: token.cursor,
        end_cursor,
    });
    let mut stream_context = FSEventStreamContext {
        version: 0,
        info: (&mut *callback_context as *mut ImpactCallbackContext).cast(),
        retain: None,
        release: None,
        copyDescription: None,
    };
    let stream = unsafe {
        FSEventStreamCreate(
            None,
            Some(impact_event_callback),
            &mut stream_context,
            erased_paths,
            token.cursor,
            QUERY_LATENCY_SECONDS,
            kFSEventStreamCreateFlagNoDefer | kFSEventStreamCreateFlagWatchRoot,
        )
    };
    if stream.is_null() {
        return Err(FilesystemChangeImpactError::Platform(
            "unable_to_create_fsevents_impact_stream".to_string(),
        ));
    }
    let mut owned_stream = OwnedEventStream {
        stream,
        queue,
        _callback_context: callback_context,
        started: false,
    };
    unsafe {
        FSEventStreamSetDispatchQueue(stream, Some(&owned_stream.queue));
    }
    let stream_started = unsafe { FSEventStreamStart(stream) };
    owned_stream.started = stream_started;
    if !stream_started {
        return Err(FilesystemChangeImpactError::Platform(
            "unable_to_start_fsevents_impact_stream".to_string(),
        ));
    }
    unsafe {
        FSEventStreamFlushAsync(stream);
    }
    let collection = wait_for_impact(&state, Instant::now() + QUERY_TIMEOUT, is_cancelled)?;
    drop(owned_stream);
    if let Some(reason) = collection.unavailable {
        return Ok(FilesystemChangeImpactOutcome::Unavailable(reason));
    }
    Ok(complete_impact_plan(token, end_cursor, collection, started))
}

fn wait_for_impact(
    state: &ImpactState,
    deadline: Instant,
    is_cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<ImpactCollection, FilesystemChangeImpactError> {
    let mut collection = state.collection.lock().map_err(|_| {
        FilesystemChangeImpactError::Platform("fsevents_impact_lock_poisoned".into())
    })?;
    loop {
        if collection.completed {
            return Ok(std::mem::take(&mut *collection));
        }
        if is_cancelled() {
            return Err(FilesystemChangeImpactError::Cancelled);
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            collection.completed = true;
            collection.unavailable = Some(FilesystemChangeImpactUnavailable::HistoryUnavailable);
            return Ok(std::mem::take(&mut *collection));
        }
        let (next, _) = state
            .notification
            .wait_timeout(collection, remaining.min(POLL_INTERVAL))
            .map_err(|_| {
                FilesystemChangeImpactError::Platform("fsevents_impact_wait_poisoned".into())
            })?;
        collection = next;
    }
}

fn complete_impact_plan(
    token: &FilesystemChangeToken,
    end_cursor: u64,
    collection: ImpactCollection,
    started: Instant,
) -> FilesystemChangeImpactOutcome {
    let dirty_directories = compress_dirty_directories(collection.dirty_directories);
    FilesystemChangeImpactOutcome::Complete(FilesystemChangeImpactPlan {
        summary: FilesystemChangeImpactSummary {
            start_cursor: token.cursor,
            end_cursor,
            page_count: collection.page_count,
            record_count: collection.record_count,
            other_records: collection.record_count,
            directory_records: collection.record_count,
            dirty_directory_count: u64::try_from(dirty_directories.len()).unwrap_or(u64::MAX),
            returned_bytes: u64::try_from(collection.path_bytes).unwrap_or(u64::MAX),
            elapsed_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            strategy: "macos_fsevents_directory_impact_v1",
            ..FilesystemChangeImpactSummary::default()
        },
        dirty_directories,
        next_token: FilesystemChangeToken {
            volume_id: token.volume_id,
            history_id: token.history_id,
            cursor: end_cursor,
        },
    })
}

fn compress_dirty_directories(mut paths: Vec<PathBuf>) -> Vec<PathBuf> {
    paths.sort_by(|left, right| {
        left.components()
            .count()
            .cmp(&right.components().count())
            .then_with(|| {
                left.as_os_str()
                    .as_bytes()
                    .cmp(right.as_os_str().as_bytes())
            })
    });
    let mut retained = Vec::<PathBuf>::with_capacity(paths.len());
    for path in paths {
        if retained.iter().any(|ancestor| path.starts_with(ancestor)) {
            continue;
        }
        retained.push(path);
    }
    retained.sort_by(|left, right| {
        left.as_os_str()
            .as_bytes()
            .cmp(right.as_os_str().as_bytes())
    });
    retained
}

pub(super) fn start_monitor(
    root: &Path,
    token: &FilesystemChangeToken,
    is_cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<Option<FilesystemChangeMonitor>, String> {
    if is_cancelled() {
        return Err("scan cancelled".to_string());
    }
    let started = Instant::now();
    if volume_id(root)? != token.volume_id {
        let monitor = terminal_monitor(FilesystemChangeStatus::HistoryUnavailable);
        log_validation(FilesystemChangeStatus::HistoryUnavailable, started, false);
        return Ok(Some(monitor));
    }
    root.to_str()
        .ok_or_else(|| "FSEvents does not support a non-Unicode scan root".to_string())?;

    let state = Arc::new(MonitorState::pending());
    let (stop_sender, stop_receiver) = sync_channel(1);
    let (startup_sender, startup_receiver) = sync_channel(1);
    let (completion_sender, completion_receiver) = sync_channel(1);
    let owner_state = Arc::clone(&state);
    let owner_root = root.to_path_buf();
    let owner_token = *token;
    let panic_sender = startup_sender.clone();
    let owner = thread::Builder::new()
        .name("mangodisk-fsevents-monitor".to_string())
        .spawn(move || {
            let result = catch_unwind(AssertUnwindSafe(|| {
                run_monitor_owner(
                    owner_root,
                    owner_token,
                    Arc::clone(&owner_state),
                    stop_receiver,
                    startup_sender,
                )
            }));
            if result.is_err() {
                owner_state.publish(FilesystemChangeStatus::HistoryUnavailable);
                let _ = panic_sender.try_send(Err("FSEvents monitor owner panicked".to_string()));
                log::error!("filesystem_change_monitor_owner_panicked platform=macos");
            }
            let _ = completion_sender.try_send(());
        })
        .map_err(|error| format!("unable to start the FSEvents monitor thread: {error}"))?;

    let backend = Arc::new(MacOsMonitorBackend {
        state: Arc::clone(&state),
        stop_sender,
        owner: Mutex::new(Some(MonitorOwner {
            handle: owner,
            completion_receiver,
        })),
    });
    let monitor = FilesystemChangeMonitor::new(backend);
    let deadline = Instant::now() + QUERY_TIMEOUT;
    wait_for_startup(&startup_receiver, deadline, is_cancelled)?;
    let status = state.wait_until_ready(deadline, is_cancelled)?;
    log_validation(status, started, true);
    Ok(Some(monitor))
}

fn terminal_monitor(status: FilesystemChangeStatus) -> FilesystemChangeMonitor {
    let state = Arc::new(MonitorState::pending());
    state.publish(status);
    let (stop_sender, _stop_receiver) = sync_channel(1);
    FilesystemChangeMonitor::new(Arc::new(MacOsMonitorBackend {
        state,
        stop_sender,
        owner: Mutex::new(None),
    }))
}

fn wait_for_startup(
    receiver: &Receiver<Result<(), String>>,
    deadline: Instant,
    is_cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<(), String> {
    loop {
        if is_cancelled() {
            return Err("scan cancelled".to_string());
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err("FSEvents monitor startup timed out".to_string());
        }
        match receiver.recv_timeout(remaining.min(POLL_INTERVAL)) {
            Ok(result) => return result,
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => {
                return Err("FSEvents monitor startup channel disconnected".to_string());
            }
        }
    }
}

fn run_monitor_owner(
    root: PathBuf,
    token: FilesystemChangeToken,
    state: Arc<MonitorState>,
    stop_receiver: Receiver<()>,
    startup_sender: SyncSender<Result<(), String>>,
) {
    let Some(root) = root.to_str() else {
        state.publish(FilesystemChangeStatus::HistoryUnavailable);
        let _ = startup_sender.try_send(Err(
            "FSEvents does not support a non-Unicode scan root".to_string()
        ));
        return;
    };
    let path = CFString::from_str(root);
    let paths = CFArray::from_objects(&[&*path]);
    // The CoreServices C API declares this as an untyped CFArrayRef, while the objc2 constructor
    // preserves its CFString generic. Both have the same underlying layout; this cast erases only
    // the static generic and does not change ownership or contents.
    let erased_paths: &CFArray =
        unsafe { &*((&*paths as *const CFArray<CFString>).cast::<CFArray>()) };
    // Queue construction may allocate and must finish before creating the raw FSEvent stream. Once
    // created, the stream immediately enters the RAII guard so later unwinding cannot leak the
    // callback context.
    let queue = DispatchQueue::new(
        "app.mangodisk.cache-dirty-monitor",
        DispatchQueueAttr::SERIAL,
    );
    let mut callback_context = Box::new(CallbackContext {
        state: Arc::clone(&state),
    });
    let mut stream_context = FSEventStreamContext {
        version: 0,
        info: (&mut *callback_context as *mut CallbackContext).cast(),
        retain: None,
        release: None,
        copyDescription: None,
    };
    // SAFETY: paths contains CFString values, and the context remains alive until the stream is
    // fully stopped and its queue is drained.
    let stream = unsafe {
        FSEventStreamCreate(
            None,
            Some(event_callback),
            &mut stream_context,
            erased_paths,
            token.cursor,
            QUERY_LATENCY_SECONDS,
            kFSEventStreamCreateFlagNoDefer | kFSEventStreamCreateFlagWatchRoot,
        )
    };
    if stream.is_null() {
        state.publish(FilesystemChangeStatus::HistoryUnavailable);
        let _ = startup_sender.try_send(Err("unable to create the FSEvents monitor".to_string()));
        return;
    }

    let mut owned_stream = OwnedEventStream {
        stream,
        queue,
        _callback_context: callback_context,
        started: false,
    };
    // SAFETY: stream is a newly created valid reference, and the queue remains alive until after
    // invalidation and synchronous draining.
    unsafe {
        FSEventStreamSetDispatchQueue(stream, Some(&owned_stream.queue));
    }
    // SAFETY: The stream is bound to the serial queue and its callback context remains valid.
    let started_stream = unsafe { FSEventStreamStart(stream) };
    owned_stream.started = started_stream;
    if started_stream {
        let _ = startup_sender.try_send(Ok(()));
        // An asynchronous flush lets the initial history catch-up become clean promptly while the
        // owner remains responsive to stop requests.
        unsafe {
            FSEventStreamFlushAsync(stream);
        }
        loop {
            if matches!(
                state.status(),
                FilesystemChangeStatus::Changed | FilesystemChangeStatus::HistoryUnavailable
            ) {
                break;
            }
            match stop_receiver.recv_timeout(POLL_INTERVAL) {
                Ok(()) | Err(RecvTimeoutError::Disconnected) => break,
                Err(RecvTimeoutError::Timeout) => {}
            }
        }
    } else {
        state.publish(FilesystemChangeStatus::HistoryUnavailable);
        let _ = startup_sender.try_send(Err("unable to start the FSEvents monitor".to_string()));
    }

    drop(owned_stream);
}

fn device_id(root: &Path) -> Result<libc::dev_t, String> {
    let metadata = fs::metadata(root)
        .map_err(|error| format!("unable to read scan-root device metadata: {error}"))?;
    libc::dev_t::try_from(metadata.dev())
        .map_err(|_| "scan-root device ID exceeds the FSEvents supported range".to_string())
}

fn volume_id(root: &Path) -> Result<[u8; 16], String> {
    volume_id_for_device(device_id(root)?)
}

fn volume_id_for_device(device: libc::dev_t) -> Result<[u8; 16], String> {
    // SAFETY: device comes from the scan root's stat st_dev field, and CFRetained owns the result.
    let uuid = unsafe { FSEventsCopyUUIDForDevice(device) }
        .ok_or_else(|| "the current volume has no available FSEvents UUID".to_string())?;
    Ok(uuid.uuid_bytes().into())
}

unsafe extern "C-unwind" fn event_callback(
    _stream: ConstFSEventStreamRef,
    info: *mut c_void,
    event_count: usize,
    _event_paths: NonNull<c_void>,
    event_flags: NonNull<u32>,
    _event_ids: NonNull<u64>,
) {
    // A panic must not cross the CoreServices FFI callback. Every path reads only the fixed-size
    // flags array and retains no event paths, so memory does not grow with event history length.
    let callback_result = catch_unwind(|| {
        // SAFETY: info points to the CallbackContext owned by the monitor, which remains alive
        // until after the queue is drained.
        let context = unsafe { &*(info.cast::<CallbackContext>()) };
        // SAFETY: CoreServices guarantees that the callback flags array contains at least
        // event_count entries.
        let flags = unsafe { std::slice::from_raw_parts(event_flags.as_ptr(), event_count) };
        if let Some(status) = classify_event_flags(flags) {
            context.state.publish(status);
        }
    });
    if callback_result.is_err() {
        // SAFETY: Same ownership guarantee as above. The panic is contained, so the monitor can
        // still be marked permanently untrusted.
        let context = unsafe { &*(info.cast::<CallbackContext>()) };
        context
            .state
            .publish(FilesystemChangeStatus::HistoryUnavailable);
        log::error!("filesystem_change_monitor_callback_panicked platform=macos");
    }
}

unsafe extern "C-unwind" fn impact_event_callback(
    _stream: ConstFSEventStreamRef,
    info: *mut c_void,
    event_count: usize,
    event_paths: NonNull<c_void>,
    event_flags: NonNull<u32>,
    event_ids: NonNull<u64>,
) {
    let callback_result = catch_unwind(|| {
        let context = unsafe { &*(info.cast::<ImpactCallbackContext>()) };
        let flags = unsafe { std::slice::from_raw_parts(event_flags.as_ptr(), event_count) };
        let ids = unsafe { std::slice::from_raw_parts(event_ids.as_ptr(), event_count) };
        // Without UseCFTypes, CoreServices supplies a stable array of NUL-terminated paths for the
        // duration of this callback. Copy only directory paths from the fixed history window; raw
        // private paths never enter logs or persisted diagnostics.
        let paths = unsafe {
            std::slice::from_raw_parts(
                event_paths.as_ptr().cast::<*const libc::c_char>(),
                event_count,
            )
        };
        let mut collection = context
            .state
            .collection
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        collection.page_count = collection.page_count.saturating_add(1);
        for index in 0..event_count {
            let event_flags = flags[index];
            if event_flags & HISTORY_INVALID_FLAGS != 0 {
                collection.unavailable =
                    Some(FilesystemChangeImpactUnavailable::HistoryUnavailable);
                collection.completed = true;
                break;
            }
            if event_flags & kFSEventStreamEventFlagHistoryDone != 0 {
                collection.completed = true;
                continue;
            }
            let event_id = ids[index];
            if event_id <= context.start_cursor || event_id > context.end_cursor {
                continue;
            }
            collection.record_count = collection.record_count.saturating_add(1);
            if collection.record_count > MAX_IMPACT_EVENTS {
                collection.unavailable = Some(FilesystemChangeImpactUnavailable::ResourceLimit);
                collection.completed = true;
                break;
            }
            let path_pointer = paths[index];
            if path_pointer.is_null() {
                collection.unavailable =
                    Some(FilesystemChangeImpactUnavailable::HistoryUnavailable);
                collection.completed = true;
                break;
            }
            let path_bytes = unsafe { CStr::from_ptr(path_pointer) }.to_bytes();
            // FSEvents may append a separator to directory paths. Change tracking hashes raw path
            // bytes, so retaining that separator would make an existing directory look absent and
            // apply its complete replacement aggregate on top of the old parent total. Normalize
            // components lexically here; canonicalization is not valid because deletion events can
            // legitimately refer to paths that no longer exist.
            let path = normalize_event_path(PathBuf::from(OsString::from_vec(path_bytes.to_vec())));
            if !path.starts_with(&context.root) {
                collection.unavailable =
                    Some(FilesystemChangeImpactUnavailable::HistoryUnavailable);
                collection.completed = true;
                break;
            }
            collection.path_bytes = collection.path_bytes.saturating_add(path_bytes.len());
            if collection.path_bytes > MAX_DIRTY_PATH_BYTES
                || collection.dirty_directories.len() >= MAX_DIRTY_DIRECTORIES
            {
                collection.unavailable = Some(FilesystemChangeImpactUnavailable::ResourceLimit);
                collection.completed = true;
                break;
            }
            collection.dirty_directories.push(path);
        }
        if collection.completed {
            context.state.notification.notify_all();
        }
    });
    if callback_result.is_err() {
        let context = unsafe { &*(info.cast::<ImpactCallbackContext>()) };
        context
            .state
            .fail(FilesystemChangeImpactUnavailable::HistoryUnavailable);
        log::error!("filesystem_change_impact_callback_panicked platform=macos");
    }
}

fn classify_event_flags(flags: &[u32]) -> Option<FilesystemChangeStatus> {
    let history_unavailable = flags.iter().any(|flags| flags & HISTORY_INVALID_FLAGS != 0);
    let history_done = flags
        .iter()
        .any(|flags| flags & kFSEventStreamEventFlagHistoryDone != 0);
    let changed = flags
        .iter()
        .any(|flags| flags & kFSEventStreamEventFlagHistoryDone == 0);
    if history_unavailable {
        Some(FilesystemChangeStatus::HistoryUnavailable)
    } else if changed {
        Some(FilesystemChangeStatus::Changed)
    } else if history_done {
        Some(FilesystemChangeStatus::Clean)
    } else {
        None
    }
}

fn normalize_event_path(path: PathBuf) -> PathBuf {
    path.components().collect()
}

fn log_validation(status: FilesystemChangeStatus, started: Instant, monitor_started: bool) {
    let elapsed_ms = started.elapsed().as_millis();
    match status {
        FilesystemChangeStatus::Pending => {
            log::warn!(
                "filesystem_change_monitor_validated platform=macos status=pending elapsed_ms={elapsed_ms}"
            );
        }
        FilesystemChangeStatus::Clean => {
            log::debug!(
                "filesystem_change_monitor_validated platform=macos status=clean monitor_started={monitor_started} elapsed_ms={elapsed_ms}"
            );
        }
        FilesystemChangeStatus::Changed => {
            log::info!(
                "filesystem_change_monitor_validated platform=macos status=changed monitor_started={monitor_started} elapsed_ms={elapsed_ms}"
            );
        }
        FilesystemChangeStatus::HistoryUnavailable => {
            log::warn!(
                "filesystem_change_monitor_validated platform=macos status=history_unavailable monitor_started={monitor_started} elapsed_ms={elapsed_ms}"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn monitor_state_is_monotonic() {
        let state = MonitorState::pending();
        state.publish(FilesystemChangeStatus::Clean);
        assert_eq!(state.status(), FilesystemChangeStatus::Clean);
        state.publish(FilesystemChangeStatus::Pending);
        state.publish(FilesystemChangeStatus::Clean);
        assert_eq!(state.status(), FilesystemChangeStatus::Clean);
        state.publish(FilesystemChangeStatus::Changed);
        state.publish(FilesystemChangeStatus::Clean);
        assert_eq!(state.status(), FilesystemChangeStatus::Changed);

        let unavailable = MonitorState::pending();
        unavailable.publish(FilesystemChangeStatus::HistoryUnavailable);
        unavailable.publish(FilesystemChangeStatus::Clean);
        assert_eq!(
            unavailable.status(),
            FilesystemChangeStatus::HistoryUnavailable
        );
    }

    #[test]
    fn history_flags_are_fail_closed() {
        assert_ne!(
            HISTORY_INVALID_FLAGS & kFSEventStreamEventFlagEventIdsWrapped,
            0
        );
        assert_ne!(
            HISTORY_INVALID_FLAGS & kFSEventStreamEventFlagMustScanSubDirs,
            0
        );
        assert_eq!(
            HISTORY_INVALID_FLAGS & kFSEventStreamEventFlagHistoryDone,
            0
        );
    }

    #[test]
    fn dirty_directories_are_deduplicated_and_ancestor_compressed() {
        let paths = vec![
            PathBuf::from("/Users/example/Library/Caches/app/nested"),
            PathBuf::from("/Users/example/Library/Caches/app"),
            PathBuf::from("/Users/example/Library/Caches/app"),
            PathBuf::from("/Users/example/Downloads"),
        ];

        assert_eq!(
            compress_dirty_directories(paths),
            vec![
                PathBuf::from("/Users/example/Downloads"),
                PathBuf::from("/Users/example/Library/Caches/app"),
            ]
        );
    }

    #[test]
    fn event_paths_drop_trailing_separators_without_touching_components() {
        assert_eq!(
            normalize_event_path(PathBuf::from("/Users/example/Library/Caches/")),
            PathBuf::from("/Users/example/Library/Caches")
        );
    }

    #[test]
    fn ordinary_event_wins_over_history_done_in_same_batch() {
        assert_eq!(
            classify_event_flags(&[0, kFSEventStreamEventFlagHistoryDone]),
            Some(FilesystemChangeStatus::Changed)
        );
        assert_eq!(
            classify_event_flags(&[kFSEventStreamEventFlagHistoryDone]),
            Some(FilesystemChangeStatus::Clean)
        );
        assert_eq!(
            classify_event_flags(&[
                kFSEventStreamEventFlagHistoryDone,
                kFSEventStreamEventFlagKernelDropped,
            ]),
            Some(FilesystemChangeStatus::HistoryUnavailable)
        );
    }

    #[test]
    fn pending_wait_observes_delayed_cancel() {
        let state = Arc::new(MonitorState::pending());
        let cancelled = Arc::new(AtomicU8::new(0));
        let cancellation_signal = Arc::clone(&cancelled);
        let cancellation_thread = thread::spawn(move || {
            thread::sleep(POLL_INTERVAL);
            cancellation_signal.store(1, Ordering::Release);
        });
        let started = Instant::now();
        assert_eq!(
            state
                .wait_until_ready(Instant::now() + QUERY_TIMEOUT, &|| {
                    cancelled.load(Ordering::Acquire) == 1
                })
                .expect_err("an active history wait must observe cancellation"),
            "scan cancelled"
        );
        cancellation_thread
            .join()
            .expect("cancellation thread should finish normally");
        assert!(
            started.elapsed() < QUERY_TIMEOUT / 2,
            "cancellation polling must not wait for the full query timeout"
        );
    }

    #[test]
    fn backend_drop_does_not_wait_for_stuck_owner() {
        let state = Arc::new(MonitorState::pending());
        let (stop_sender, _stop_receiver) = sync_channel(1);
        let (completion_sender, completion_receiver) = sync_channel(1);
        let owner = thread::spawn(move || {
            // Simulate an abnormal block during CoreServices cleanup. The backend must detach
            // safely after the fixed timeout while the owner retains its resources, so cache
            // clearing or cancellation cannot block indefinitely.
            thread::sleep(Duration::from_secs(1));
            drop(completion_sender);
        });
        let backend = MacOsMonitorBackend {
            state,
            stop_sender,
            owner: Mutex::new(Some(MonitorOwner {
                handle: owner,
                completion_receiver,
            })),
        };

        let started = Instant::now();
        drop(backend);
        assert!(
            started.elapsed() < Duration::from_millis(600),
            "an unresponsive owner must detach after the 250 ms wait limit"
        );
    }

    fn stable_clean_monitor(root: &Path) -> FilesystemChangeMonitor {
        for _ in 0..20 {
            let token = capture_token(root)
                .expect("FSEvents token capture should succeed")
                .expect("the system volume should support an FSEvents token");
            let monitor = start_monitor(root, &token, &|| false)
                .expect("FSEvents monitor startup should succeed")
                .expect("the system volume should support an FSEvents monitor");
            if monitor.status() == FilesystemChangeStatus::Clean {
                return monitor;
            }
            drop(monitor);
            thread::sleep(POLL_INTERVAL);
        }
        panic!("FSEvents history did not stabilize for the fixture directory");
    }

    #[test]
    #[ignore = "executed explicitly by the real FSEvents change-detection validation"]
    fn real_monitor_observes_change_after_clean() {
        let root =
            std::env::temp_dir().join(format!("mangodisk-fsevents-test-{}", std::process::id()));
        fs::create_dir_all(&root).expect("FSEvents fixture directory should be created");
        let monitor = stable_clean_monitor(&root);
        let file_path = root.join("changed.bin");
        let file = fs::File::create(&file_path).expect("fixture file should be created");
        file.sync_all()
            .expect("fixture file should be synchronized");
        for _ in 0..80 {
            if monitor.status() == FilesystemChangeStatus::Changed {
                break;
            }
            thread::sleep(POLL_INTERVAL);
        }
        assert_eq!(monitor.status(), FilesystemChangeStatus::Changed);
        drop(monitor);
        fs::remove_dir_all(root).expect("FSEvents fixture directory should be removed");
    }

    #[test]
    #[ignore = "executed explicitly by the real FSEvents impact-plan validation"]
    fn real_impact_plan_reports_only_changed_subtrees() {
        let requested_root = std::env::temp_dir().join(format!(
            "mangodisk-fsevents-impact-test-{}",
            std::process::id()
        ));
        fs::create_dir_all(&requested_root).expect("impact fixture directory should be created");
        // macOS exposes /var as a symlink to /private/var. Production scan roots are canonicalized
        // before token capture, so the real integration fixture must use the same spelling as
        // FSEvents or its correctly reported path would appear outside the requested scope.
        let root = fs::canonicalize(&requested_root)
            .expect("impact fixture root should have a canonical path");
        let changed = root.join("changed");
        fs::create_dir_all(&changed).expect("impact fixture directory should be created");
        let (token, monitor) = (0..20)
            .find_map(|_| {
                let token = capture_token(&root)
                    .expect("FSEvents token capture should succeed")
                    .expect("the system volume should support an FSEvents token");
                let monitor = start_monitor(&root, &token, &|| false)
                    .expect("FSEvents monitor startup should succeed")
                    .expect("the system volume should support an FSEvents monitor");
                if monitor.status() == FilesystemChangeStatus::Clean {
                    Some((token, monitor))
                } else {
                    drop(monitor);
                    thread::sleep(POLL_INTERVAL);
                    None
                }
            })
            .expect("FSEvents history should stabilize before the fixture mutation");
        let file_path = changed.join("data.bin");
        let file = fs::File::create(&file_path).expect("fixture file should be created");
        file.sync_all()
            .expect("fixture file should be synchronized");
        for _ in 0..80 {
            if monitor.status() == FilesystemChangeStatus::Changed {
                break;
            }
            thread::sleep(POLL_INTERVAL);
        }
        assert_eq!(monitor.status(), FilesystemChangeStatus::Changed);
        drop(monitor);

        let mut changed_plan = None;
        for _ in 0..40 {
            let outcome = impact_plan(&root, &token, &|| false)
                .expect("FSEvents impact query should succeed");
            if let FilesystemChangeImpactOutcome::Complete(plan) = outcome {
                if plan.has_changes() {
                    changed_plan = Some(plan);
                    break;
                }
            }
            thread::sleep(POLL_INTERVAL);
        }
        let changed_plan =
            changed_plan.expect("the impact plan should observe the changed subtree");
        assert!(changed_plan
            .dirty_directories
            .iter()
            .all(|path| path.starts_with(&root)));

        let clean_outcome = impact_plan(&root, &changed_plan.next_token, &|| false)
            .expect("the next fixed history window should be queryable");
        let FilesystemChangeImpactOutcome::Complete(clean_plan) = clean_outcome else {
            panic!("an unchanged scope should retain complete history");
        };
        assert!(!clean_plan.has_changes());
        fs::remove_dir_all(requested_root).expect("impact fixture directory should be removed");
    }

    #[test]
    #[ignore = "executed explicitly by the real FSEvents latency baseline"]
    fn real_clean_monitor_hot_status_p95_stays_below_5_ms() {
        let root = std::env::temp_dir().join(format!(
            "mangodisk-fsevents-latency-test-{}",
            std::process::id()
        ));
        fs::create_dir_all(&root).expect("FSEvents latency fixture directory should be created");
        let initial_started = Instant::now();
        let monitor = stable_clean_monitor(&root);
        let initial_elapsed = initial_started.elapsed();

        let mut samples = Vec::with_capacity(100);
        for _ in 0..100 {
            let started = Instant::now();
            assert_eq!(monitor.status(), FilesystemChangeStatus::Clean);
            samples.push(started.elapsed());
        }
        samples.sort_unstable();
        let p95 = samples[94];
        println!(
            "FSEvents monitor initial={} ms, hot status P95={} us",
            initial_elapsed.as_millis(),
            p95.as_micros()
        );
        assert!(
            p95 < Duration::from_millis(5),
            "in-session monitor status-read P95 should remain below 5 ms"
        );
        let drop_started = Instant::now();
        drop(monitor);
        assert!(
            drop_started.elapsed() < Duration::from_millis(250),
            "clean monitor teardown should remain below 250 ms"
        );
        fs::remove_dir_all(root).expect("FSEvents latency fixture directory should be removed");
    }

    #[test]
    #[ignore = "executed explicitly by the real FSEvents lifecycle validation"]
    fn real_monitor_pre_cancel_and_volume_mismatch_fail_closed() {
        let root = std::env::temp_dir().join(format!(
            "mangodisk-fsevents-lifecycle-test-{}",
            std::process::id()
        ));
        fs::create_dir_all(&root).expect("FSEvents lifecycle fixture directory should be created");
        let token = capture_token(&root)
            .expect("FSEvents token capture should succeed")
            .expect("the system volume should support an FSEvents token");

        let started = Instant::now();
        let cancellation_error = match start_monitor(&root, &token, &|| true) {
            Err(error) => error,
            Ok(_) => panic!("cancellation must prevent monitor startup"),
        };
        assert_eq!(cancellation_error, "scan cancelled");
        assert!(started.elapsed() < POLL_INTERVAL);

        let mut mismatched = token;
        mismatched.volume_id[0] ^= 0xff;
        let monitor = start_monitor(&root, &mismatched, &|| false)
            .expect("a volume mismatch should return a status")
            .expect("the system volume should return a terminal monitor");
        assert_eq!(monitor.status(), FilesystemChangeStatus::HistoryUnavailable);
        fs::remove_dir_all(root).expect("FSEvents lifecycle fixture directory should be removed");
    }
}
