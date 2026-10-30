//! One bounded dispatch path for native polling tasks; sources retain their own readers.
use super::{
    sampling_policy::Task,
    sampling_schedule::{Demand, SamplingSlot},
    sampling_workers::{self, Completion, Request, SamplingEvent},
};
use mangodisk_core::system_resources::readings::ResourceCache;
use std::{
    sync::mpsc::{SyncSender, TrySendError},
    time::Instant,
};

struct Worker {
    task: Task,
    requests: SyncSender<Request>,
    slot: SamplingSlot,
    closed_until_ms: Option<u64>,
    thread: Option<std::thread::JoinHandle<()>>,
}
pub struct PollingWorkers {
    workers: [Worker; Task::WORKERS.len()],
    resume_generation: u64,
    origin: Instant,
    events: SyncSender<SamplingEvent>,
}
impl PollingWorkers {
    pub fn start(origin: Instant, events: SyncSender<SamplingEvent>) -> Self {
        Self {
            resume_generation: 0,
            origin,
            events: events.clone(),
            workers: Task::WORKERS.map(|task| {
                let (requests, thread) = sampling_workers::spawn_task(task, origin, events.clone());
                Worker {
                    task,
                    requests,
                    slot: Default::default(),
                    closed_until_ms: None,
                    thread: Some(thread),
                }
            }),
        }
    }
    pub fn dispatch(
        &mut self,
        metrics: &[Demand; 5],
        cache: &mut ResourceCache,
        now_ms: u64,
    ) -> Vec<Task> {
        let mut failed = Vec::new();
        for worker in &mut self.workers {
            update(worker, worker.task.demand(metrics, cache), cache);
            if worker.closed_until_ms.is_none()
                && worker
                    .thread
                    .as_ref()
                    .is_some_and(|thread| thread.is_finished())
            {
                worker.thread.take();
                worker.slot.retire();
                worker.closed_until_ms = Some(now_ms.saturating_add(30_000));
                failed.push(worker.task);
                log::error!("resident_sampling_worker_exited task={:?} outcome=defer_restart retry_after_ms=30000", worker.task);
            }
            if let Some(until) = worker.closed_until_ms {
                if !worker.slot.demand.active || now_ms < until {
                    continue;
                }
                let (requests, thread) =
                    sampling_workers::spawn_task(worker.task, self.origin, self.events.clone());
                worker.requests = requests;
                worker.thread = Some(thread);
                worker.closed_until_ms = None;
                worker.slot.resume();
                log::info!("resident_sampling_worker_restarted task={:?}", worker.task);
            }
            let interval = worker.task.interval_ms(&worker.slot.demand);
            if let Some(generation) = worker.slot.begin(now_ms, interval) {
                match worker.requests.try_send(Request::Sample {
                    generation,
                    baseline_generation: worker.slot.baseline_generation(),
                    resume_generation: self.resume_generation,
                    requested_at_ms: now_ms,
                    demand: worker.slot.demand.clone(),
                }) {
                    Ok(()) => {}
                    Err(TrySendError::Full(_)) => {
                        worker.slot.complete(generation);
                        worker.slot.retry_after(now_ms, 250);
                    }
                    Err(TrySendError::Disconnected(_)) => {
                        worker.slot.complete(generation);
                        worker.closed_until_ms = Some(now_ms.saturating_add(30_000));
                        worker.slot.postpone_after(now_ms, 30_000);
                        failed.push(worker.task);
                        log::error!("resident_sampling_worker_closed task={:?} outcome=defer_restart retry_after_ms=30000", worker.task);
                    }
                }
            }
        }
        failed
    }
    pub fn accept(
        &mut self,
        completion: &Completion,
        metrics: &[Demand; 5],
        cache: &mut ResourceCache,
        now_ms: u64,
    ) -> bool {
        let worker = self
            .workers
            .iter_mut()
            .find(|worker| worker.task == completion.task)
            .expect("registered polling task");
        update(worker, worker.task.demand(metrics, cache), cache);
        if !worker.slot.demand.active {
            let _ = worker.requests.try_send(Request::Release);
        }
        let accepted = worker.slot.complete(completion.generation);
        if accepted {
            // Completion-based spacing never runs ahead of a source's own cache
            // deadline or replays overdue work after a slow native operation.
            worker
                .slot
                .postpone_after(now_ms, worker.task.interval_ms(&worker.slot.demand));
        }
        accepted
    }
    pub fn slot(&mut self, task: Task) -> &mut SamplingSlot {
        &mut self
            .workers
            .iter_mut()
            .find(|worker| worker.task == task)
            .expect("registered polling task")
            .slot
    }
    pub fn resume(&mut self) {
        self.resume_generation = self.resume_generation.wrapping_add(1);
        for worker in &mut self.workers {
            worker.slot.resume();
            if worker.closed_until_ms.is_some() {
                worker.closed_until_ms = Some(0);
            }
        }
    }

    pub fn active(&self) -> bool {
        self.workers.iter().any(|worker| worker.slot.demand.active)
    }
    pub fn wait_ms(&self, now_ms: u64) -> u64 {
        self.workers
            .iter()
            .map(|worker| {
                worker
                    .closed_until_ms
                    .filter(|_| worker.slot.demand.active)
                    .map_or_else(
                        || worker.slot.wait_ms(now_ms),
                        |until| until.saturating_sub(now_ms).clamp(1, 1000),
                    )
            })
            .min()
            .unwrap_or(1000)
    }
}
fn update(worker: &mut Worker, demand: Demand, cache: &mut ResourceCache) {
    if worker.task == Task::DiskSensors
        && (worker.slot.demand.temperature_device != demand.temperature_device
            || (demand.temperature_device.is_none()
                && worker.slot.demand.selection != demand.selection))
    {
        cache.clear_disk_temperature();
    }
    let selection_changed = worker.slot.demand.selection != demand.selection;
    if worker.slot.update(demand) {
        let demand = &worker.slot.demand;
        log::info!("resident_sampling_demand task={:?} active={} detailed={} temperature={} live_battery={} prepare_battery={} system_power={} component_power={} fan_speed={} catalogue_only={} has_selection={} interval_ms={} baseline_generation={}", worker.task, demand.active, demand.detailed, demand.temperature, demand.battery, demand.prepare_battery, demand.system_power, demand.component_power, demand.fan_speed, demand.catalogue_only, demand.selection.is_some(), worker.task.interval_ms(demand), worker.slot.baseline_generation());
        if worker.task == Task::DiskIo {
            cache.stop_disk_io();
        }
        if worker.task.is_primary() {
            if selection_changed {
                if worker.task == Task::Disk {
                    cache.reset_disk_selection();
                } else {
                    cache.reset(worker.task.metric());
                }
            } else if !worker.slot.demand.active {
                cache.suspend(worker.task.metric());
            }
        }
        if !worker.slot.demand.active {
            let _ = worker.requests.try_send(Request::Release);
        }
    }
}

#[cfg(test)]
#[path = "sampling_dispatch_tests.rs"]
mod tests;
