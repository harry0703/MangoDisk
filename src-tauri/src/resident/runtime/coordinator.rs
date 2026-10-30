//! Drive desktop sampling without moving presentation lifetime into Core.
use super::{
    observation::{ObservationContext, ObservationState},
    ResidentState,
};
use crate::resident::{
    network_traffic, presentation, process_cpu_sampling,
    sampling_diagnostics::{Query, SamplingDiagnostics},
    sampling_dispatch::PollingWorkers,
    sampling_policy::Task,
    sampling_workers::{self, SamplingEvent},
    tray_display,
};
use mangodisk_core::system_resources::{
    metrics::{MetricId, MetricStatus},
    readings::ResourceCache,
};
use std::{
    sync::{
        atomic::Ordering,
        mpsc::{self, Receiver, SyncSender},
        Arc,
    },
    time::{Duration, Instant},
};

pub(super) fn run(
    app: tauri::AppHandle,
    worker: Arc<ResidentState>,
    sender: SyncSender<SamplingEvent>,
    events: Receiver<SamplingEvent>,
) {
    let presentation = presentation::Presentation::start(app.clone(), worker.clone());
    let origin = Instant::now();
    let mut jobs = PollingWorkers::start(origin, sender.clone());
    let mut cache = ResourceCache::default();
    let mut disk_status = MetricStatus::Loading;
    if let Some(traffic) = network_traffic::install(&app, &worker) {
        cache.set_network_traffic(traffic);
    }
    let mut observations = ObservationState::new(worker.enabled());
    let mut previous_status = [MetricStatus::Loading; 5];
    let mut diagnostics = SamplingDiagnostics::default();
    let mut summary_at = Instant::now();
    let mut process_cpu = process_cpu_sampling::ProcessCpuSampling::start(origin, sender.clone());
    let mut loop_at = Instant::now();
    let mut loop_wall_at = sampling_workers::timestamp_ms();
    let mut was_active = worker.enabled();
    let mut loop_max_ms = 0u128;
    let mut loop_delayed = false;
    let mut display_pending: Option<Instant> = None;
    let mut display_updated = Instant::now() - Duration::from_secs(1);
    log::info!(
        "resident_gpu_sampling_policy keep_history_when_hidden={} interval_ms={}",
        cfg!(any(target_os = "macos", windows)),
        MetricId::Gpu.interval_ms()
    );
    loop {
        let loop_ms = loop_at.elapsed().as_millis();
        loop_at = Instant::now();
        let wall_now = sampling_workers::timestamp_ms();
        let wall_loop_ms = wall_now.saturating_sub(loop_wall_at);
        let wall_clock_reversed = wall_now < loop_wall_at;
        loop_wall_at = wall_now;
        loop_max_ms = loop_max_ms.max(loop_ms);
        // An idle disabled service can sleep indefinitely; that is intentional.
        // While active, the coordinator should wake at least once per second.
        let active = worker.enabled();
        let delayed = was_active && active && loop_ms > 2000;
        if was_active
            && active
            && (wall_clock_reversed || u128::from(wall_loop_ms).abs_diff(loop_ms) > 2000)
        {
            log::info!("resident_clock_gap monotonic_gap_ms={loop_ms} wall_gap_ms={wall_loop_ms} wall_clock_reversed={wall_clock_reversed} outcome=revalidate_freshness");
        }
        let resumed = wall_clock_reversed
            || u128::from(wall_loop_ms).abs_diff(loop_ms) > 2000
            || loop_ms > 10_000;
        if resumed && (active || jobs.active()) {
            jobs.resume();
            process_cpu.resume();
            cache.resume_sampling();
            log::info!("resident_sampling_resumed wall_gap_ms={wall_loop_ms} monotonic_gap_ms={loop_ms} outcome=reprobe_sources_preserve_sample_times");
        }
        if delayed && !loop_delayed {
            log::warn!("resident_sampling_delayed loop_ms={loop_ms} wall_gap_ms={wall_loop_ms} wall_clock_reversed={wall_clock_reversed}");
        } else if loop_delayed && !delayed {
            log::info!("resident_sampling_recovered");
        }
        was_active = active;
        loop_delayed = delayed;
        let process_refresh = worker.process_refresh.swap(false, Ordering::Relaxed);
        if process_refresh {
            jobs.slot(Task::MemoryProcesses).request_now();
        }
        let demands = worker.demands(observations.warm_icons);
        let failed = jobs.dispatch(&demands, &mut cache, origin.elapsed().as_millis() as u64);
        let mut changed = !failed.is_empty();
        for task in failed {
            diagnostics.discard(task);
            observations.worker_closed(task, &mut cache, sampling_workers::timestamp_ms());
        }
        changed |= process_cpu.poll(
            active,
            active && worker.cpu_processes_visible(),
            process_refresh,
            origin.elapsed().as_millis() as u64,
            &mut cache,
        );
        let event = if jobs.active() || display_pending.is_some() || changed {
            match events.recv_timeout(Duration::from_millis(
                jobs.wait_ms(origin.elapsed().as_millis() as u64)
                    .min(process_cpu.wait_ms(origin.elapsed().as_millis() as u64))
                    .min(display_pending.map_or(1000, |deadline| {
                        (deadline
                            .saturating_duration_since(Instant::now())
                            .as_millis() as u64)
                            .max(1)
                    })),
            )) {
                Ok(event) => Some(event),
                Err(mpsc::RecvTimeoutError::Timeout) => None,
                Err(_) => break,
            }
        } else {
            match events.recv() {
                Ok(event) => Some(event),
                Err(_) => break,
            }
        };
        if matches!(event, Some(SamplingEvent::Wake)) {
            presentation.publish();
            display_updated = Instant::now();
            display_pending = None;
        }
        let event = match event {
            Some(SamplingEvent::ProcessCpu(completion)) => {
                diagnostics.record(Query::CpuProcesses, completion.duration_ms);
                changed |= process_cpu.poll(
                    worker.enabled(),
                    worker.cpu_processes_visible(),
                    false,
                    origin.elapsed().as_millis() as u64,
                    &mut cache,
                );
                if process_cpu.complete(completion, origin.elapsed().as_millis() as u64, &mut cache)
                {
                    changed = true;
                } else {
                    diagnostics.discard(Query::CpuProcesses);
                }
                None
            }
            event => event,
        };
        if let Some(SamplingEvent::Completed(completion)) = event {
            let task = completion.task;
            let metric = task.metric();
            let index = metric as usize;
            if jobs.accept(
                &completion,
                &worker.demands(observations.warm_icons),
                &mut cache,
                origin.elapsed().as_millis() as u64,
            ) {
                changed = true;
                diagnostics.completed(&completion, origin.elapsed().as_millis() as u64);
                if task.is_primary()
                    && task != Query::Gpu
                    && completion.duration_ms > metric.freshness_ms()
                {
                    if previous_status[index] != MetricStatus::Stale {
                        log::warn!(
                            "resident_sample_expired metric={:?} duration_ms={}",
                            metric,
                            completion.duration_ms
                        );
                    }
                    if metric == MetricId::Memory {
                        cache.fail_memory_overview(MetricStatus::Stale);
                    } else if metric == MetricId::Disk {
                        cache.fail_disk_capacity(MetricStatus::Stale);
                    } else {
                        cache.fail(metric, MetricStatus::Stale);
                    }
                    diagnostics.discard(task);
                } else {
                    observations.apply(
                        *completion,
                        &mut cache,
                        jobs.slot(task),
                        ObservationContext {
                            catalogue: &worker.catalogue,
                            now_ms: &|| origin.elapsed().as_millis() as u64,
                            previous_status: previous_status[index],
                        },
                        |summary| {
                            let app = app.clone();
                            // Icon I/O has its own bounded one-time warm-up;
                            // it cannot block sampling or publication.
                            std::thread::spawn(move || {
                                crate::resident::application_icons::warm(&app, &summary)
                            });
                        },
                    );
                }
            } else {
                diagnostics.discard(task);
            }
        }
        let now_ms = sampling_workers::timestamp_ms();
        changed |= cache.expire(now_ms);
        if changed {
            let resources = cache.snapshot(now_ms);
            if resources.disk_io.status != disk_status {
                log::info!(
                    "resident_disk_io_state from={:?} to={:?} scope=all_devices",
                    disk_status,
                    resources.disk_io.status
                );
                disk_status = resources.disk_io.status;
            }
            let sample_times = [
                resources.cpu.sampled_at_ms,
                resources.gpu.sampled_at_ms,
                resources.memory.sampled_at_ms,
                resources.network.sampled_at_ms,
                resources.disk.sampled_at_ms,
            ];
            let statuses = [
                resources.cpu.status,
                resources.gpu.status,
                resources.memory.status,
                resources.network.status,
                resources.disk.status,
            ];
            for index in 0..MetricId::ALL.len() {
                if statuses[index] != previous_status[index] {
                    log::info!(
                        "resident_metric_state metric={:?} from={:?} to={:?} now_ms={} sampled_at_ms={:?} sample_age_ms={:?} sample_in_future={} freshness_ms={} interval_ms={} coordinator_loop_ms={} wall_loop_ms={} wall_clock_reversed={}",
                        MetricId::ALL[index], previous_status[index], statuses[index], now_ms, sample_times[index],
                        sample_times[index].map(|at| now_ms.saturating_sub(at)), sample_times[index].is_some_and(|at| at > now_ms),
                        MetricId::ALL[index].freshness_ms(), MetricId::ALL[index].interval_ms(), loop_ms, wall_loop_ms, wall_clock_reversed
                    );
                }
            }
            previous_status = statuses;
            {
                let mut reading = worker
                    .reading
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                if let Some(gpu) = resources
                    .gpu
                    .value
                    .as_ref()
                    .filter(|_| resources.gpu.status == MetricStatus::Ready)
                {
                    let previous = reading.resources.gpu.value.as_ref();
                    if previous.is_none_or(|previous| previous.adapter_id != gpu.adapter_id) {
                        log::info!(
                            "resident_gpu_selected previous={} adapter={} name={} automatic={} usage_history_points={} temperature_history_points={}",
                            mangodisk_platform::diagnostics::text(previous.map_or("none", |value| value.adapter_id.as_str())),
                            mangodisk_platform::diagnostics::text(&gpu.adapter_id),
                            mangodisk_platform::diagnostics::text(&gpu.adapter_name),
                            jobs.slot(Query::Gpu).demand.selection.is_none(),
                            resources.gpu_detail_history.len(),
                            resources.gpu_temperature_history.len()
                        );
                    }
                }
                if resources.network.status == MetricStatus::Ready {
                    if let Some(network) = &resources.network.value {
                        if reading.resources.network.status != MetricStatus::Ready
                            || reading
                                .resources
                                .network
                                .value
                                .as_ref()
                                .is_none_or(|previous| {
                                    previous.interface.id != network.interface.id
                                })
                        {
                            log::info!(
                                "resident_network_selected reason={:?} automatic={} temporary_fallback={} kind={:?}",
                                network.selection_reason,
                                jobs.slot(Query::Network).demand.selection.is_none(),
                                jobs.slot(Query::Network).demand.selection.as_deref()
                                    .is_some_and(|saved| saved != network.interface.id),
                                network.interface.kind
                            );
                        }
                    }
                }
                reading.revision += 1;
                reading.resources = resources;
                // A reset may finish after snapshot construction but before
                // this lock. Never publish that snapshot's pre-reset totals.
                if let Some(network) = reading.resources.network.value.as_mut() {
                    network.cumulative = worker
                        .network_traffic
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .as_ref()
                        .and_then(|traffic| traffic.total(&network.interface.id));
                }
            }
            // Independent workers commonly finish within a few milliseconds.
            // Native status-bar layout is expensive. Limit periodic display
            // publications to once per second; native and WebView consumers
            // receive the same snapshot after combining completion bursts.
            display_pending.get_or_insert_with(|| {
                (Instant::now() + Duration::from_millis(50))
                    .max(display_updated + Duration::from_secs(1))
            });
        }
        if process_cpu.take_refresh_completed()
            || display_pending.is_some_and(|deadline| Instant::now() >= deadline)
        {
            presentation.publish();
            display_updated = Instant::now();
            display_pending = None;
        }
        if summary_at.elapsed() >= Duration::from_secs(300) {
            diagnostics.log_and_reset(summary_at.elapsed().as_millis());
            tray_display::log_summary(&app, loop_max_ms);
            loop_max_ms = 0;
            summary_at = Instant::now();
        }
    }
}
