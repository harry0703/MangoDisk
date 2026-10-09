use std::{
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
    time::Instant,
};

use serde::Serialize;

use crate::filesystem::metadata::display_path;

const PROGRESS_INTERVAL_MS: u64 = 100;
const UNPUBLISHED: u64 = u64::MAX;

/// Progress crosses adapter boundaries as stable identifiers, never localized text.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TraversalStage {
    Analyzing,
    ValidatingFiles,
    HashingFiles,
    DiscoveringApplications,
    CheckingProcesses,
    ValidatingApplications,
    InspectingApplications,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TraversalProgress {
    pub operation_id: u64,
    pub current_stage: TraversalStage,
    pub current_path: String,
    pub items_scanned: u64,
    pub bytes_scanned: u64,
    pub completed_steps: u64,
    pub total_steps: u64,
    pub found_items: u64,
    pub found_bytes: u64,
    pub elapsed_ms: u64,
}

/// Receives UI-independent progress from a Core operation.
///
/// Desktop and CLI adapters implement the same contract without exposing a
/// Tauri handle, terminal renderer, or transport type to the Core.
pub trait ProgressSink: Send + Sync + 'static {
    fn report(&self, progress: TraversalProgress);
}

impl<F> ProgressSink for F
where
    F: Fn(TraversalProgress) + Send + Sync + 'static,
{
    fn report(&self, progress: TraversalProgress) {
        self(progress);
    }
}

/// Maintains throttling and atomic counters for disk traversal progress.
///
/// Cleanup can inspect rules concurrently while storage scans visit files at
/// high frequency. A shared tracker keeps operation IDs, elapsed time, and
/// counters consistent for every adapter.
pub(crate) struct ProgressTracker {
    operation_id: u64,
    callback: Box<dyn ProgressSink>,
    started_at: Instant,
    last_emit_ms: AtomicU64,
    items_scanned: AtomicU64,
    bytes_scanned: AtomicU64,
    completed_steps: AtomicU64,
    total_steps: AtomicU64,
    found_items: AtomicU64,
    found_bytes: AtomicU64,
}

/// Owns progress observations produced by one fallible native traversal.
///
/// Native directory aggregation streams useful live totals before its final
/// result is known. If that fast path fails, Core must retry with the portable
/// walker without retaining those partial totals. A lease records only its own
/// contribution, so rollback remains correct even when independent cleanup
/// roots are measured concurrently through the same tracker.
pub(crate) struct ScanObservationLease<'a> {
    tracker: &'a ProgressTracker,
    items_scanned: AtomicU64,
    bytes_scanned: AtomicU64,
    committed: bool,
}

impl ProgressTracker {
    pub(crate) fn new(operation_id: u64, callback: impl ProgressSink, total_steps: u64) -> Self {
        Self::from_sink(operation_id, Box::new(callback), total_steps)
    }

    pub(crate) fn from_sink(
        operation_id: u64,
        callback: Box<dyn ProgressSink>,
        total_steps: u64,
    ) -> Self {
        Self {
            operation_id,
            callback,
            started_at: Instant::now(),
            last_emit_ms: AtomicU64::new(UNPUBLISHED),
            items_scanned: AtomicU64::new(0),
            bytes_scanned: AtomicU64::new(0),
            completed_steps: AtomicU64::new(0),
            total_steps: AtomicU64::new(total_steps),
            found_items: AtomicU64::new(0),
            found_bytes: AtomicU64::new(0),
        }
    }

    /// Some operations discover their work set while reading an operating
    /// system catalog. Updating the total after discovery keeps the initial
    /// phase observable without inventing a percentage before the amount of
    /// work is known.
    pub(crate) fn set_total_steps(&self, total_steps: u64) {
        self.total_steps.store(total_steps, Ordering::Relaxed);
    }

    /// Rule completion can arrive from several workers. The counter measures
    /// completed inspection steps rather than files so adapters receive a
    /// stable determinate progress value.
    pub(crate) fn complete_step(&self, stage: TraversalStage, path: &Path, bytes: u64) {
        let total_steps = self.total_steps.load(Ordering::Relaxed);
        let advanced = self
            .completed_steps
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |completed| {
                (completed < total_steps).then_some(completed + 1)
            })
            .is_ok();
        // Duplicate completion means the caller raced or reported a step
        // twice. Publish the latest state, but never let counters exceed the
        // plan because reports and cleanup plans consume these values.
        if advanced && bytes > 0 {
            self.found_items.fetch_add(1, Ordering::Relaxed);
            self.found_bytes.fetch_add(bytes, Ordering::Relaxed);
        }
        self.emit(stage, path);
    }

    pub(crate) fn visit_directory(&self, stage: TraversalStage, path: &Path) {
        self.emit(stage, path);
    }

    pub(crate) fn visit_file(&self, stage: TraversalStage, path: &Path, bytes: u64) {
        self.items_scanned.fetch_add(1, Ordering::Relaxed);
        self.bytes_scanned.fetch_add(bytes, Ordering::Relaxed);
        self.emit(stage, path);
    }

    /// Native volume indexes and domain scanners report trusted aggregates
    /// after validation. This preserves live counters without manufacturing
    /// millions of callbacks solely for progress rendering. Each producer is
    /// responsible for submitting a measured batch at most once.
    pub(crate) fn observe_files(
        &self,
        stage: TraversalStage,
        path: &Path,
        file_count: u64,
        bytes: u64,
    ) {
        self.items_scanned.fetch_add(file_count, Ordering::Relaxed);
        self.bytes_scanned.fetch_add(bytes, Ordering::Relaxed);
        self.emit(stage, path);
    }

    /// Starts a provisional observation scope for a native operation that can
    /// fall back after publishing partial progress.
    pub(crate) fn begin_scan_observation(&self) -> ScanObservationLease<'_> {
        ScanObservationLease {
            tracker: self,
            items_scanned: AtomicU64::new(0),
            bytes_scanned: AtomicU64::new(0),
            committed: false,
        }
    }

    fn remove_scan_observations(&self, file_count: u64, bytes: u64) {
        let _ = self
            .items_scanned
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                Some(current.saturating_sub(file_count))
            });
        let _ = self
            .bytes_scanned
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                Some(current.saturating_sub(bytes))
            });
    }

    /// Read completed root observations after its workers have joined, before accumulating
    /// the next root. Retries can reset a root-local tracker without losing previous totals.
    pub(crate) fn scan_observation_counts(&self) -> (u64, u64) {
        (
            self.items_scanned.load(Ordering::Relaxed),
            self.bytes_scanned.load(Ordering::Relaxed),
        )
    }

    /// A persistence failure retries traversal with an in-memory sink. Reset
    /// observations before retrying so the final event is neither partial nor
    /// doubled. Disk operations are coordinated and this traversal is serial,
    /// so the reset cannot erase another worker's valid count.
    pub(crate) fn reset_scan_observations_for_retry(&self) {
        self.items_scanned.store(0, Ordering::Relaxed);
        self.bytes_scanned.store(0, Ordering::Relaxed);
        self.last_emit_ms.store(UNPUBLISHED, Ordering::Relaxed);
    }

    /// Replaces path observations with the completed scan's reconciled physical allocation.
    /// Called after workers exit, before publishing the final analysis event.
    pub(crate) fn replace_scan_observations(&self, files: u64, bytes: u64) {
        self.items_scanned.store(files, Ordering::Relaxed);
        self.bytes_scanned.store(bytes, Ordering::Relaxed);
    }

    /// Normal progress is throttled to 100 ms. Counters advance before
    /// throttling so a later or final event always contains the latest state.
    pub(crate) fn emit(&self, stage: TraversalStage, path: &Path) {
        self.emit_at(stage, path, self.elapsed_ms());
    }

    fn emit_at(&self, stage: TraversalStage, path: &Path, current_ms: u64) {
        let previous_ms = self.last_emit_ms.load(Ordering::Relaxed);
        if previous_ms != UNPUBLISHED
            && current_ms.saturating_sub(previous_ms) < PROGRESS_INTERVAL_MS
        {
            return;
        }
        if self
            .last_emit_ms
            .compare_exchange(
                previous_ms,
                current_ms,
                Ordering::Relaxed,
                Ordering::Relaxed,
            )
            .is_err()
        {
            return;
        }
        self.publish(stage, path, current_ms);
    }

    /// The coordinator calls this once after every worker exits. It bypasses
    /// throttling so adapters always observe the completed state.
    pub(crate) fn finish(&self, stage: TraversalStage, path: &Path) {
        let current_ms = self.elapsed_ms();
        self.last_emit_ms.store(current_ms, Ordering::Relaxed);
        self.publish(stage, path, current_ms);
    }

    fn elapsed_ms(&self) -> u64 {
        // Durations and throttling must survive wall-clock corrections. Keep
        // calendar timestamps confined to persisted observations and history.
        u64::try_from(self.started_at.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    fn publish(&self, stage: TraversalStage, path: &Path, current_ms: u64) {
        self.callback.report(TraversalProgress {
            operation_id: self.operation_id,
            current_stage: stage,
            current_path: display_path(path),
            items_scanned: self.items_scanned.load(Ordering::Relaxed),
            bytes_scanned: self.bytes_scanned.load(Ordering::Relaxed),
            completed_steps: self.completed_steps.load(Ordering::Relaxed),
            total_steps: self.total_steps.load(Ordering::Relaxed),
            found_items: self.found_items.load(Ordering::Relaxed),
            found_bytes: self.found_bytes.load(Ordering::Relaxed),
            elapsed_ms: current_ms,
        });
    }
}

impl ScanObservationLease<'_> {
    /// Publishes one measured native batch and remembers exactly how much this
    /// lease contributed to the shared tracker.
    pub(crate) fn observe(&self, stage: TraversalStage, path: &Path, files: u64, bytes: u64) {
        self.items_scanned.fetch_add(files, Ordering::Relaxed);
        self.bytes_scanned.fetch_add(bytes, Ordering::Relaxed);
        self.tracker.observe_files(stage, path, files, bytes);
    }

    /// Reconciles bounded callback batches with the authoritative completed
    /// aggregate before making them permanent. A mismatch is corrected rather
    /// than allowed to leak into UI counters.
    pub(crate) fn commit_exact(
        &mut self,
        stage: TraversalStage,
        path: &Path,
        expected_files: u64,
        expected_bytes: u64,
    ) {
        let observed_files = self.items_scanned.load(Ordering::Relaxed);
        let observed_bytes = self.bytes_scanned.load(Ordering::Relaxed);
        if expected_files > observed_files || expected_bytes > observed_bytes {
            self.tracker.observe_files(
                stage,
                path,
                expected_files.saturating_sub(observed_files),
                expected_bytes.saturating_sub(observed_bytes),
            );
        }
        if observed_files > expected_files || observed_bytes > expected_bytes {
            self.tracker.remove_scan_observations(
                observed_files.saturating_sub(expected_files),
                observed_bytes.saturating_sub(expected_bytes),
            );
            self.tracker.emit(stage, path);
        }
        self.committed = true;
    }
}

impl Drop for ScanObservationLease<'_> {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        self.tracker.remove_scan_observations(
            self.items_scanned.load(Ordering::Relaxed),
            self.bytes_scanned.load(Ordering::Relaxed),
        );
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    #[test]
    fn relative_zero_is_a_valid_first_throttle_tick() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&events);
        let tracker = ProgressTracker::new(1, move |event| captured.lock().unwrap().push(event), 0);
        let path = Path::new("/fixture");
        for elapsed_ms in [0, 0, 99, 100, 100, 199, 200] {
            tracker.emit_at(TraversalStage::Analyzing, path, elapsed_ms);
        }
        let events = events.lock().unwrap();
        assert_eq!(
            events
                .iter()
                .map(|event| event.elapsed_ms)
                .collect::<Vec<_>>(),
            [0, 100, 200],
            "the zero tick must not disable throttling"
        );
    }

    #[test]
    fn retry_keeps_operation_duration_and_final_progress_bypasses_throttling() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&events);
        let mut tracker =
            ProgressTracker::new(1, move |event| captured.lock().unwrap().push(event), 0);
        tracker.started_at = Instant::now() - std::time::Duration::from_secs(18);
        let path = Path::new("/fixture");
        tracker.visit_file(TraversalStage::Analyzing, path, 64);
        tracker.reset_scan_observations_for_retry();
        tracker.visit_file(TraversalStage::Analyzing, path, 32);
        tracker.finish(TraversalStage::Analyzing, path);
        let events = events.lock().unwrap();
        assert_eq!(events.len(), 3);
        assert!(events
            .iter()
            .all(|event| (18_000..19_000).contains(&event.elapsed_ms)));
        assert!(events
            .windows(2)
            .all(|pair| pair[0].elapsed_ms <= pair[1].elapsed_ms));
        assert_eq!(events[2].items_scanned, 1);
        assert_eq!(events[2].bytes_scanned, 32);
    }

    #[test]
    fn a_new_operation_starts_its_own_duration() {
        let events = Arc::new(Mutex::new(Vec::new()));
        for (operation_id, seconds) in [(1, 18), (2, 0)] {
            let captured = Arc::clone(&events);
            let mut tracker = ProgressTracker::new(
                operation_id,
                move |event| captured.lock().unwrap().push(event),
                0,
            );
            tracker.started_at = Instant::now() - std::time::Duration::from_secs(seconds);
            tracker.finish(TraversalStage::Analyzing, Path::new("/fixture"));
        }
        let events = events.lock().unwrap();
        assert_eq!(events[0].operation_id, 1);
        assert!((18_000..19_000).contains(&events[0].elapsed_ms));
        assert_eq!(events[1].operation_id, 2);
        assert!(events[1].elapsed_ms < 1000);
    }

    #[test]
    fn frequent_progress_is_coalesced_without_losing_the_final_state() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&events);
        let tracker = ProgressTracker::new(
            1,
            move |progress| {
                captured
                    .lock()
                    .expect("the progress lock should remain valid")
                    .push(progress)
            },
            2,
        );
        let path = Path::new("/fixture");

        tracker.complete_step(TraversalStage::Analyzing, path, 10);
        tracker.complete_step(TraversalStage::Analyzing, path, 20);
        for _ in 0..100 {
            tracker.visit_file(TraversalStage::Analyzing, path, 1);
        }
        tracker.finish(TraversalStage::Analyzing, path);

        let events = events
            .lock()
            .expect("the progress lock should remain valid");
        assert_eq!(
            events.len(),
            2,
            "frequent progress events should be coalesced within the interval"
        );
        let final_event = events.last().expect("the final event should be present");
        assert_eq!(final_event.completed_steps, 2);
        assert_eq!(final_event.total_steps, 2);
        assert_eq!(final_event.found_items, 2);
        assert_eq!(final_event.found_bytes, 30);
    }

    #[test]
    fn native_aggregate_observes_final_file_totals_once() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&events);
        let tracker = ProgressTracker::new(
            1,
            move |progress| {
                captured
                    .lock()
                    .expect("the progress lock should remain valid")
                    .push(progress)
            },
            0,
        );
        let path = Path::new("/fixture");

        tracker.observe_files(TraversalStage::Analyzing, path, 123, 456);
        tracker.finish(TraversalStage::Analyzing, path);

        let events = events
            .lock()
            .expect("the progress lock should remain valid");
        let final_event = events.last().expect("the final event should be present");
        assert_eq!(final_event.items_scanned, 123);
        assert_eq!(final_event.bytes_scanned, 456);
    }

    #[test]
    fn failed_native_observation_is_rolled_back_before_fallback() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&events);
        let tracker = ProgressTracker::new(
            1,
            move |progress| {
                captured
                    .lock()
                    .expect("the progress lock should remain valid")
                    .push(progress)
            },
            0,
        );
        let path = Path::new("/fixture");

        {
            let observation = tracker.begin_scan_observation();
            observation.observe(TraversalStage::Analyzing, path, 10, 100);
        }
        tracker.visit_file(TraversalStage::Analyzing, path, 25);
        tracker.finish(TraversalStage::Analyzing, path);

        let events = events.lock().expect("progress lock must remain valid");
        let final_event = events.last().expect("final progress must be present");
        assert_eq!(final_event.items_scanned, 1);
        assert_eq!(final_event.bytes_scanned, 25);
    }

    #[test]
    fn successful_native_observation_reconciles_to_the_exact_aggregate() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&events);
        let tracker = ProgressTracker::new(
            1,
            move |progress| {
                captured
                    .lock()
                    .expect("the progress lock should remain valid")
                    .push(progress)
            },
            0,
        );
        let path = Path::new("/fixture");

        {
            let mut observation = tracker.begin_scan_observation();
            observation.observe(TraversalStage::Analyzing, path, 8, 80);
            observation.commit_exact(TraversalStage::Analyzing, path, 10, 100);
        }
        tracker.finish(TraversalStage::Analyzing, path);

        let events = events.lock().expect("progress lock must remain valid");
        let final_event = events.last().expect("final progress must be present");
        assert_eq!(final_event.items_scanned, 10);
        assert_eq!(final_event.bytes_scanned, 100);
    }

    #[test]
    fn duplicate_completion_cannot_exceed_total_steps() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&events);
        let tracker = ProgressTracker::new(
            1,
            move |progress| {
                captured
                    .lock()
                    .expect("the progress lock should remain valid")
                    .push(progress)
            },
            1,
        );
        let path = Path::new("/fixture");

        tracker.complete_step(TraversalStage::Analyzing, path, 10);
        tracker.complete_step(TraversalStage::Analyzing, path, 20);
        tracker.finish(TraversalStage::Analyzing, path);

        let events = events
            .lock()
            .expect("the progress lock should remain valid");
        let final_event = events.last().expect("the final event should be present");
        assert_eq!(final_event.completed_steps, 1);
        assert_eq!(final_event.found_items, 1);
        assert_eq!(final_event.found_bytes, 10);
    }

    #[test]
    fn persistence_retry_resets_traversal_counts_but_keeps_rule_progress() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&events);
        let tracker = ProgressTracker::new(
            1,
            move |progress| {
                captured
                    .lock()
                    .expect("the progress lock should remain valid")
                    .push(progress)
            },
            1,
        );
        let path = Path::new("/fixture");

        tracker.complete_step(TraversalStage::Analyzing, path, 10);
        tracker.visit_file(TraversalStage::Analyzing, path, 20);
        tracker.visit_file(TraversalStage::Analyzing, path, 30);
        tracker.reset_scan_observations_for_retry();
        tracker.visit_file(TraversalStage::Analyzing, path, 40);
        tracker.finish(TraversalStage::Analyzing, path);

        let events = events
            .lock()
            .expect("the progress lock should remain valid");
        let final_event = events.last().expect("the final event should be present");
        assert_eq!(final_event.items_scanned, 1);
        assert_eq!(final_event.bytes_scanned, 40);
        assert_eq!(final_event.completed_steps, 1);
        assert_eq!(final_event.found_items, 1);
        assert_eq!(final_event.found_bytes, 10);
    }

    #[test]
    fn file_progress_reports_the_active_file() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&events);
        let tracker = ProgressTracker::new(
            1,
            move |progress| {
                captured
                    .lock()
                    .expect("progress lock must remain valid")
                    .push(progress)
            },
            1,
        );
        let file = Path::new("fixture/item.bin");

        tracker.visit_file(TraversalStage::Analyzing, file, 40);

        let events = events.lock().expect("progress lock must remain valid");
        let event = events.last().expect("file traversal must publish progress");
        assert_eq!(event.current_path, display_path(file));
    }

    #[test]
    fn discovered_work_can_set_a_determinate_total_after_progress_starts() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&events);
        let tracker = ProgressTracker::new(
            1,
            move |progress| {
                captured
                    .lock()
                    .expect("progress lock must remain valid")
                    .push(progress)
            },
            0,
        );
        let source = Path::new("registry");
        let application = Path::new("Example");

        tracker.emit(TraversalStage::DiscoveringApplications, source);
        tracker.set_total_steps(2);
        tracker.complete_step(TraversalStage::InspectingApplications, application, 1024);
        tracker.finish(TraversalStage::InspectingApplications, application);

        let events = events.lock().expect("progress lock must remain valid");
        let first = events
            .first()
            .expect("catalog discovery must be observable");
        assert_eq!(first.total_steps, 0);
        let final_event = events.last().expect("final progress must be published");
        assert_eq!(final_event.total_steps, 2);
        assert_eq!(final_event.completed_steps, 1);
        assert_eq!(final_event.found_bytes, 1024);
    }
}
