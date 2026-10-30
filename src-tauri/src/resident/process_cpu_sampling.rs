//! Application counters have their own cadence and cannot delay overview readings.
use super::sampling_schedule::{Demand, SamplingSlot};
use super::sampling_workers::{timestamp_ms, SamplingEvent};
use mangodisk_core::system_resources::{
    metrics::{MetricId, MetricReading, MetricStatus},
    readings::ResourceCache,
};
use mangodisk_core::{
    system_resources::{models::ProcessCpuSummary, process_cpu::ProcessCpuService},
    CoreResult,
};
use std::{
    sync::mpsc::{self, SyncSender, TrySendError},
    time::Instant,
};

const RESTART_DELAY_MS: u64 = 30_000;

pub enum Request {
    Sample { generation: u64 },
    Reset,
}
pub struct Completion {
    pub generation: u64,
    pub timestamp_ms: u64,
    pub duration_ms: u64,
    pub result: CoreResult<Option<ProcessCpuSummary>>,
}

pub struct ProcessCpuSampling {
    slot: SamplingSlot,
    requests: SyncSender<Request>,
    visible: bool,
    retries: u8,
    status: MetricStatus,
    refresh_started: Option<Instant>,
    refresh_completed: bool,
    closed_until_ms: Option<u64>,
    thread: Option<std::thread::JoinHandle<()>>,
    origin: Instant,
    events: SyncSender<SamplingEvent>,
}
impl ProcessCpuSampling {
    pub fn start(origin: Instant, events: SyncSender<SamplingEvent>) -> Self {
        let (requests, thread) = start_worker(origin, events.clone());
        Self {
            slot: Default::default(),
            requests,
            thread: Some(thread),
            visible: false,
            retries: 0,
            status: MetricStatus::Loading,
            refresh_started: None,
            refresh_completed: false,
            closed_until_ms: None,
            origin,
            events,
        }
    }
    pub fn poll(
        &mut self,
        active: bool,
        visible: bool,
        refresh: bool,
        now_ms: u64,
        cache: &mut ResourceCache,
    ) -> bool {
        let exited = self.closed_until_ms.is_none()
            && self
                .thread
                .as_ref()
                .is_some_and(|thread| thread.is_finished());
        if exited {
            self.thread.take();
            self.slot.retire();
            self.closed_until_ms = Some(now_ms.saturating_add(RESTART_DELAY_MS));
            self.status = MetricStatus::Failed;
            cache.cpu_processes(MetricReading {
                status: MetricStatus::Failed,
                ..Default::default()
            });
            log::error!("resident_process_cpu_worker_exited outcome=defer_restart retry_after_ms={RESTART_DELAY_MS}");
        }
        if let Some(until) = self.closed_until_ms {
            if !active || now_ms < until {
                return exited;
            }
            let (requests, thread) = start_worker(self.origin, self.events.clone());
            self.requests = requests;
            self.thread = Some(thread);
            self.closed_until_ms = None;
            self.slot.resume();
            log::info!("resident_process_cpu_worker_restarted");
        }
        let changed = self.slot.update(Demand {
            active,
            ..Default::default()
        });
        if changed {
            self.retries = 0;
            self.refresh_started = None;
            if !active {
                cache.clear_cpu_processes();
                let _ = self.requests.try_send(Request::Reset);
            }
        }
        if changed || self.visible != visible {
            log::info!(
                "resident_process_cpu_demand active={active} visible={visible} interval_ms={}",
                interval_ms(visible)
            );
        }
        self.visible = visible;
        if !visible {
            self.refresh_started = None;
        }
        if active && refresh {
            self.refresh_started = Some(Instant::now());
            self.slot.request_now();
        }
        if visible {
            self.slot.retry_after(now_ms, interval_ms(true));
        }
        if let Some(generation) = self.slot.begin(now_ms, interval_ms(visible)) {
            match self.requests.try_send(Request::Sample { generation }) {
                Ok(()) => {}
                Err(TrySendError::Full(_)) => {
                    self.slot.complete(generation);
                    self.slot.retry_after(now_ms, 250);
                }
                Err(TrySendError::Disconnected(_)) => {
                    self.slot.complete(generation);
                    self.slot.update(Demand::default());
                    self.status = MetricStatus::Failed;
                    self.closed_until_ms = Some(now_ms.saturating_add(RESTART_DELAY_MS));
                    cache.cpu_processes(MetricReading {
                        status: MetricStatus::Failed,
                        ..Default::default()
                    });
                    log::error!("resident_process_cpu_worker_closed generation={generation} outcome=defer_restart retry_after_ms={RESTART_DELAY_MS}");
                    return true;
                }
            }
        }
        changed
    }
    pub fn resume(&mut self) {
        self.slot.resume();
        self.retries = 0;
        self.refresh_started = None;
    }

    pub fn wait_ms(&self, now_ms: u64) -> u64 {
        self.closed_until_ms.map_or_else(
            || self.slot.wait_ms(now_ms),
            |until| until.saturating_sub(now_ms).clamp(1, 1000),
        )
    }

    pub fn complete(
        &mut self,
        completion: Completion,
        now_ms: u64,
        cache: &mut ResourceCache,
    ) -> bool {
        if !self.slot.complete(completion.generation) {
            if !self.slot.demand.active {
                let _ = self.requests.try_send(Request::Reset);
            }
            return false;
        }
        let reading = if completion.duration_ms > MetricId::Cpu.freshness_ms() {
            MetricReading {
                status: MetricStatus::Stale,
                ..Default::default()
            }
        } else {
            match completion.result {
                Ok(Some(summary)) => {
                    self.retries = 0;
                    if let Some(started) = self.refresh_started.take() {
                        self.refresh_completed = true;
                        log::info!(
                            "resident_process_cpu_refreshed elapsed_ms={} query_ms={} visible={}",
                            started.elapsed().as_millis(),
                            completion.duration_ms,
                            self.visible
                        );
                    }
                    MetricReading::ready(summary, completion.timestamp_ms)
                }
                Ok(None) => {
                    if self.retries < 2 {
                        self.retries += 1;
                        self.slot.retry_after(now_ms, 250);
                    }
                    MetricReading::default()
                }
                Err(error) => {
                    if self.status != MetricStatus::Failed {
                        log::warn!(
                            "resident_process_cpu_failed code={:?} error={}",
                            error.code(),
                            mangodisk_platform::diagnostics::text(&error)
                        );
                    }
                    MetricReading {
                        status: MetricStatus::Failed,
                        ..Default::default()
                    }
                }
            }
        };
        if reading.status != self.status {
            log::info!(
                "resident_process_cpu_state from={:?} to={:?} query_ms={}",
                self.status,
                reading.status,
                completion.duration_ms
            );
        }
        self.status = reading.status;
        cache.cpu_processes(reading);
        true
    }
    pub fn take_refresh_completed(&mut self) -> bool {
        std::mem::take(&mut self.refresh_completed)
    }
}
fn interval_ms(visible: bool) -> u64 {
    super::sampling_policy::Task::CpuProcesses.interval_ms(&Demand {
        detailed: visible,
        ..Default::default()
    })
}

fn start_worker(
    origin: Instant,
    events: SyncSender<SamplingEvent>,
) -> (SyncSender<Request>, std::thread::JoinHandle<()>) {
    let (sender, requests) = mpsc::sync_channel(1);
    let thread = std::thread::spawn(move || {
        let mut service = None;
        let mut generation = None;
        while let Ok(request) = requests.recv() {
            let Request::Sample {
                generation: requested,
            } = request
            else {
                service = None;
                generation = None;
                continue;
            };
            if generation.replace(requested) != Some(requested) {
                service = None;
            }
            let started = Instant::now();
            let timestamp_ms = timestamp_ms();
            let result = service
                .get_or_insert_with(ProcessCpuService::default)
                .sample(origin.elapsed().as_millis() as u64);
            if events
                .send(SamplingEvent::ProcessCpu(Completion {
                    generation: requested,
                    timestamp_ms,
                    duration_ms: started.elapsed().as_millis() as u64,
                    result,
                }))
                .is_err()
            {
                break;
            }
        }
    });
    (sender, thread)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mangodisk_platform::system_resources::process_cpu::CpuUsageScale;

    fn summary() -> ProcessCpuSummary {
        ProcessCpuSummary {
            usage_scale: CpuUsageScale::SingleCore,
            applications: vec![],
            readable_process_count: 0,
            omitted_process_count: 0,
        }
    }
    #[test]
    fn reopening_preserves_the_last_reading_and_reuses_an_in_flight_sample() {
        let (requests, jobs) = mpsc::sync_channel(4);
        let mut sampling = ProcessCpuSampling {
            slot: Default::default(),
            requests,
            visible: false,
            retries: 0,
            status: MetricStatus::Ready,
            refresh_started: None,
            refresh_completed: false,
            closed_until_ms: None,
            origin: Instant::now(),
            events: mpsc::sync_channel(8).0,
            thread: None,
        };
        let mut cache = ResourceCache::default();
        cache.cpu_processes(MetricReading::ready(summary(), 0));
        sampling.poll(true, false, false, 0, &mut cache);
        let Request::Sample { generation } = jobs.try_recv().unwrap() else {
            panic!("expected sample")
        };
        sampling.poll(true, true, true, 1000, &mut cache);
        assert!(jobs.try_recv().is_err());
        assert!(cache.snapshot(1000).cpu_processes.value.is_some());
        assert!(sampling.complete(
            Completion {
                generation,
                timestamp_ms: 1000,
                duration_ms: 1,
                result: Ok(Some(summary()))
            },
            1000,
            &mut cache
        ));
        assert!(sampling.take_refresh_completed());
        sampling.poll(true, true, false, 2999, &mut cache);
        assert!(jobs.try_recv().is_err());
        sampling.poll(true, true, false, 3000, &mut cache);
        assert!(matches!(jobs.try_recv(), Ok(Request::Sample { .. })));
    }
    #[test]
    fn a_disabled_generation_cannot_restore_stale_process_rows() {
        let (requests, jobs) = mpsc::sync_channel(4);
        let mut sampling = ProcessCpuSampling {
            slot: Default::default(),
            requests,
            visible: false,
            retries: 0,
            status: MetricStatus::Loading,
            refresh_started: None,
            refresh_completed: false,
            closed_until_ms: None,
            origin: Instant::now(),
            events: mpsc::sync_channel(8).0,
            thread: None,
        };
        let mut cache = ResourceCache::default();
        sampling.poll(true, false, false, 0, &mut cache);
        let Request::Sample { generation } = jobs.try_recv().unwrap() else {
            panic!("expected sample")
        };
        sampling.poll(false, false, false, 100, &mut cache);
        assert!(!sampling.complete(
            Completion {
                generation,
                timestamp_ms: 100,
                duration_ms: 1,
                result: Ok(Some(summary()))
            },
            100,
            &mut cache
        ));
        assert!(cache.snapshot(100).cpu_processes.value.is_none());
    }
    fn fixture() -> (ProcessCpuSampling, mpsc::Receiver<Request>, ResourceCache) {
        let (requests, jobs) = mpsc::sync_channel(4);
        let sampling = ProcessCpuSampling {
            slot: Default::default(),
            requests,
            visible: false,
            retries: 0,
            status: MetricStatus::Ready,
            refresh_started: None,
            refresh_completed: false,
            closed_until_ms: None,
            origin: Instant::now(),
            events: mpsc::sync_channel(8).0,
            thread: None,
        };
        let mut cache = ResourceCache::default();
        cache.cpu_processes(MetricReading::ready(summary(), 0));
        (sampling, jobs, cache)
    }
    #[test]
    fn a_queued_reset_does_not_block_the_resident_sampling_loop() {
        let (mut sampling, _, mut cache) = fixture();
        let (requests, jobs) = mpsc::sync_channel(1);
        requests.send(Request::Reset).unwrap();
        sampling.requests = requests;
        let (completed, results) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            sampling.poll(true, false, false, 0, &mut cache);
            completed.send((sampling, cache)).unwrap();
        });
        let result = results.recv_timeout(std::time::Duration::from_secs(1));
        if result.is_err() {
            drop(jobs);
            worker.join().unwrap();
            panic!("a queued reset blocked the overview sampling loop");
        }
        worker.join().unwrap();
        let (mut sampling, mut cache) = result.unwrap();
        assert!(sampling.closed_until_ms.is_none());
        assert!(matches!(jobs.try_recv(), Ok(Request::Reset)));
        sampling.poll(true, false, false, 249, &mut cache);
        assert!(jobs.try_recv().is_err());
        sampling.poll(true, false, false, 250, &mut cache);
        assert!(matches!(jobs.try_recv(), Ok(Request::Sample { .. })));
    }

    #[test]
    fn background_waits_four_seconds_and_opening_requests_a_sample_immediately() {
        let (mut sampling, jobs, mut cache) = fixture();
        sampling.poll(true, false, false, 0, &mut cache);
        let Request::Sample { generation } = jobs.try_recv().unwrap() else {
            panic!("expected sample")
        };
        sampling.complete(
            Completion {
                generation,
                timestamp_ms: 0,
                duration_ms: 1,
                result: Ok(Some(summary())),
            },
            1,
            &mut cache,
        );
        sampling.poll(true, false, false, 3999, &mut cache);
        assert!(jobs.try_recv().is_err());
        sampling.poll(true, false, false, 4000, &mut cache);
        let Request::Sample { generation } = jobs.try_recv().unwrap() else {
            panic!("expected sample")
        };
        sampling.complete(
            Completion {
                generation,
                timestamp_ms: 4000,
                duration_ms: 1,
                result: Ok(Some(summary())),
            },
            4001,
            &mut cache,
        );
        sampling.poll(true, true, true, 4500, &mut cache);
        assert!(matches!(jobs.try_recv(), Ok(Request::Sample { .. })));
    }
    #[test]
    fn exited_inflight_process_sampling_rejects_old_results_and_waits_before_restarting() {
        let (mut sampling, jobs, mut cache) = fixture();
        sampling.poll(true, false, false, 0, &mut cache);
        let Request::Sample { generation } = jobs.try_recv().unwrap() else {
            panic!("expected request")
        };
        let thread = std::thread::spawn(|| ());
        while !thread.is_finished() {
            std::thread::yield_now();
        }
        sampling.thread = Some(thread);
        assert!(sampling.poll(true, false, false, 1000, &mut cache));
        assert_eq!(sampling.closed_until_ms, Some(31000));
        assert_eq!(sampling.wait_ms(1500), 1000);
        assert!(!sampling.poll(true, false, false, 30999, &mut cache));
        assert!(jobs.try_recv().is_err());
        assert!(!sampling.complete(
            Completion {
                generation,
                timestamp_ms: 1000,
                duration_ms: 1,
                result: Ok(Some(summary()))
            },
            2000,
            &mut cache
        ));
        assert_eq!(
            cache.snapshot(2000).cpu_processes.status,
            MetricStatus::Failed
        );
    }
}
