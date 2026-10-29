use super::*;
use std::sync::atomic::AtomicBool;

#[derive(Debug, PartialEq, Eq)]
enum LifecycleEvent {
    Callback {
        context_alive: bool,
    },
    Operation {
        stage: EventStreamStage,
        on_callback_queue: bool,
    },
    NativeContextDrop,
    ContextDrop,
}

struct LifecycleProbe {
    events: SyncSender<LifecycleEvent>,
    context_dropped: AtomicBool,
}

struct ContextProbe(Arc<LifecycleProbe>);

struct RetainedStream(FSEventStreamRef);

impl Drop for RetainedStream {
    fn drop(&mut self) {
        // SAFETY: The test acquired one extra native reference, independently of the stream guard.
        unsafe { objc2_core_services::FSEventStreamRelease(self.0) };
    }
}

impl Drop for ContextProbe {
    fn drop(&mut self) {
        self.0.context_dropped.store(true, Ordering::Release);
        let _ = self.0.events.try_send(LifecycleEvent::ContextDrop);
    }
}

unsafe extern "C-unwind" fn retain_probe(info: *const c_void) -> *const c_void {
    // SAFETY: Create receives a pointer backed by the test's live Arc. The native stream acquires
    // its own reference so delayed cleanup remains safe even if an assertion or timeout unwinds.
    unsafe { Arc::<LifecycleProbe>::increment_strong_count(info.cast()) };
    info
}

unsafe extern "C-unwind" fn release_probe(info: *const c_void) {
    // SAFETY: This consumes exactly the reference acquired by retain_probe for the native stream.
    let probe = unsafe { Arc::<LifecycleProbe>::from_raw(info.cast()) };
    let _ = probe.events.try_send(LifecycleEvent::NativeContextDrop);
}

unsafe extern "C-unwind" fn unused_event_callback(
    _stream: ConstFSEventStreamRef,
    _info: *mut c_void,
    _count: usize,
    _paths: NonNull<c_void>,
    _flags: NonNull<u32>,
    _ids: NonNull<u64>,
) {
}

fn assert_context_teardown(owner_panics: bool) {
    let (events, receiver) = sync_channel(8);
    let probe = Arc::new(LifecycleProbe {
        events,
        context_dropped: AtomicBool::new(false),
    });
    let queue = DispatchQueue::new("app.mangodisk.lifecycle-test", DispatchQueueAttr::SERIAL);
    queue.set_specific(NonNull::from(probe.as_ref()).cast(), || {});

    let path = CFString::from_str("/");
    let paths = CFArray::from_objects(&[&*path]);
    // SAFETY: This only erases CFArray's static CFString element type for the CoreServices API.
    let erased_paths: &CFArray =
        unsafe { &*((&*paths as *const CFArray<CFString>).cast::<CFArray>()) };
    let mut context = FSEventStreamContext {
        version: 0,
        info: Arc::as_ptr(&probe).cast_mut().cast(),
        retain: Some(retain_probe),
        release: Some(release_probe),
        copyDescription: None,
    };
    // SAFETY: The path and context are valid. This stream is never started, so no filesystem
    // events or private APIs are needed to exercise teardown after an incomplete startup.
    let stream = unsafe {
        FSEventStreamCreate(
            None,
            Some(unused_event_callback),
            &mut context,
            erased_paths,
            objc2_core_services::kFSEventStreamEventIdSinceNow,
            QUERY_LATENCY_SECONDS,
            kFSEventStreamCreateFlagNoDefer,
        )
    };
    assert!(!stream.is_null(), "create the native lifecycle fixture");
    let operation_probe = Arc::clone(&probe);
    let owned = OwnedEventStream {
        lifecycle: EventStreamLifecycle {
            stream,
            started: false,
            observer: Some(Box::new(move |stage| {
                let key = NonNull::from(operation_probe.as_ref()).cast();
                // SAFETY: The Arc keeps this queue-specific key alive throughout observation.
                let on_callback_queue = !unsafe { dispatch2::dispatch_get_specific(key) }.is_null();
                let _ = operation_probe.events.try_send(LifecycleEvent::Operation {
                    stage,
                    on_callback_queue,
                });
            })),
        },
        queue,
        _callback_context: Box::new(ContextProbe(Arc::clone(&probe))),
    };
    // SAFETY: The stream and its serial queue remain owned by the guard until teardown finishes.
    unsafe { FSEventStreamSetDispatchQueue(stream, Some(&owned.queue)) };
    // SAFETY: Retain the valid stream to deliberately defer its final native destructor beyond
    // Rust context destruction. It may then execute outside the guard's callback queue.
    let deferred_release = unsafe {
        objc2_core_services::FSEventStreamRetain(stream);
        RetainedStream(stream)
    };
    let callback_probe = Arc::clone(&probe);
    owned.queue.exec_async(move || {
        let _ = callback_probe.events.try_send(LifecycleEvent::Callback {
            context_alive: !callback_probe.context_dropped.load(Ordering::Acquire),
        });
    });

    let result = catch_unwind(AssertUnwindSafe(move || {
        let _owned = owned;
        assert!(
            !owner_panics,
            "simulate an owner panic after queue scheduling"
        );
    }));
    assert_eq!(result.is_err(), owner_panics);
    drop(deferred_release);
    let observed: Vec<_> = (0..5)
        .map(|_| {
            receiver
                .recv_timeout(Duration::from_secs(2))
                .expect("native callbacks and the context must finish teardown")
        })
        .collect();
    assert_eq!(
        &observed[..3],
        &[
            LifecycleEvent::Callback {
                context_alive: true,
            },
            LifecycleEvent::Operation {
                stage: EventStreamStage::Invalidation,
                on_callback_queue: true,
            },
            LifecycleEvent::Operation {
                stage: EventStreamStage::Release,
                on_callback_queue: true,
            },
        ],
        "native teardown must run on the callback queue after pending callbacks"
    );
    assert!(
        observed.contains(&LifecycleEvent::NativeContextDrop),
        "CoreServices must release its retained context"
    );
    // CoreServices may retain the invalidated stream beyond the guard's final release. Its native
    // context destructor need not precede Rust context destruction once pending callbacks drain.
    assert!(observed.contains(&LifecycleEvent::ContextDrop));
}

#[test]
fn unstarted_stream_serializes_native_teardown_with_callbacks() {
    assert_context_teardown(false);
}

#[test]
fn owner_unwind_releases_context_after_pending_callbacks() {
    assert_context_teardown(true);
}
