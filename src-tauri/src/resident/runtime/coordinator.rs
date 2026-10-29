//! Drive desktop sampling without moving presentation lifetime into Core.
use super::{
    observation::{ObservationContext, ObservationState},
    ResidentState,
};
use crate::resident::{
    disk_activity, network_traffic, presentation, process_cpu_sampling,
    sampling_diagnostics::{Query, SamplingDiagnostics},
    sampling_schedule::{Demand, SamplingSlot},
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
    let jobs = MetricId::ALL.map(|metric| sampling_workers::start(metric, origin, sender.clone()));
    let mut slots: [SamplingSlot; 5] = std::array::from_fn(|_| SamplingSlot::default());
    let auxiliary = sampling_workers::start_cpu_auxiliary(origin, sender.clone());
    let mut auxiliary_slot = SamplingSlot::default();
    let disk_sensors = sampling_workers::start_disk_sensors(origin, sender.clone());
    let mut sensor_slot = SamplingSlot::default();
    let mut auxiliary_expired = false;
    let mut cache = ResourceCache::default();
    if let Some(traffic) = network_traffic::install(&app, &worker) {
        cache.set_network_traffic(traffic);
    }
    let disk_activity = disk_activity::start(origin, sender.clone());
    let mut disk_slot = SamplingSlot::default();
    let mut disk_status = MetricStatus::Loading;
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
        if delayed && !loop_delayed {
            log::warn!("resident_sampling_delayed loop_ms={loop_ms} wall_gap_ms={wall_loop_ms} wall_clock_reversed={wall_clock_reversed}");
        } else if loop_delayed && !delayed {
            log::info!("resident_sampling_recovered");
        }
        was_active = active;
        loop_delayed = delayed;
        let mut demands = worker.demands(observations.warm_icons);
        let auxiliary_demand = demands[0].clone();
        let sensor_demand = disk_sensor_demand(demands[4].clone(), cache.disk_volume());
        for (metric, demand) in MetricId::ALL.into_iter().zip(&mut demands) {
            *demand = metric_demand(metric, demand.clone());
        }
        if sensor_slot.demand.temperature_device != sensor_demand.temperature_device {
            cache.clear_disk_temperature();
        }
        if sensor_slot.update(sensor_demand.clone()) && !sensor_demand.active {
            let _ = disk_sensors.try_send(sampling_workers::Request::Release);
        }
        if let Some(generation) = sensor_slot.begin(
            origin.elapsed().as_millis() as u64,
            MetricId::Disk.interval_ms(),
        ) {
            if disk_sensors
                .send(sampling_workers::Request::Sample {
                    generation,
                    baseline_generation: sensor_slot.baseline_generation(),
                    demand: sensor_demand,
                })
                .is_err()
            {
                return;
            }
        }
        if auxiliary_slot.update(auxiliary_demand.clone()) && !auxiliary_demand.active {
            let _ = auxiliary.try_send(sampling_workers::Request::Release);
        }
        if let Some(generation) = auxiliary_slot.begin(
            origin.elapsed().as_millis() as u64,
            MetricId::Cpu.interval_ms(),
        ) {
            if auxiliary
                .send(sampling_workers::Request::Sample {
                    generation,
                    baseline_generation: auxiliary_slot.baseline_generation(),
                    demand: auxiliary_demand,
                })
                .is_err()
            {
                return;
            }
        }
        let mut changed = false;
        changed |= process_cpu.poll(
            active,
            active && worker.cpu_processes_visible(),
            worker.process_refresh.swap(false, Ordering::Relaxed),
            origin.elapsed().as_millis() as u64,
            &mut cache,
        );
        if disk_slot.update(worker.disk_activity_demand()) {
            cache.stop_disk_io();
            log::info!("resident_disk_io_demand active={}", disk_slot.demand.active);
        }
        if let Some(generation) = disk_slot.begin(origin.elapsed().as_millis() as u64, 2000) {
            if disk_activity.send(generation).is_err() {
                return;
            }
        }
        for (index, metric) in MetricId::ALL.into_iter().enumerate() {
            let selection_changed = slots[index].demand.selection != demands[index].selection;
            let temperature_device_changed =
                slots[index].demand.temperature_device != demands[index].temperature_device;
            if slots[index].update(demands[index].clone()) {
                if temperature_device_changed {
                    cache.clear_disk_temperature();
                }
                if selection_changed {
                    cache.reset(metric);
                } else if !demands[index].active {
                    cache.suspend(metric);
                }
                if !demands[index].active {
                    // A queued query will release again when its completion arrives.
                    // Never block the coordinator behind a native worker.
                    let _ = jobs[index].try_send(sampling_workers::Request::Release);
                }
            }
            let interval_ms = metric.interval_ms();
            if let Some(generation) =
                slots[index].begin(origin.elapsed().as_millis() as u64, interval_ms)
            {
                if jobs[index]
                    .send(sampling_workers::Request::Sample {
                        generation,
                        baseline_generation: slots[index].baseline_generation(),
                        demand: demands[index].clone(),
                    })
                    .is_err()
                {
                    return;
                }
            }
        }
        let event = if demands.iter().any(|demand| demand.active)
            || disk_slot.demand.active
            || display_pending.is_some()
            || changed
        {
            match events.recv_timeout(Duration::from_millis(
                slots
                    .iter()
                    .map(|slot| slot.wait_ms(origin.elapsed().as_millis() as u64))
                    .min()
                    .unwrap_or(1000)
                    .min(auxiliary_slot.wait_ms(origin.elapsed().as_millis() as u64))
                    .min(sensor_slot.wait_ms(origin.elapsed().as_millis() as u64))
                    .min(disk_slot.wait_ms(origin.elapsed().as_millis() as u64))
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
            Some(SamplingEvent::CpuAuxiliary(completion)) => {
                auxiliary_slot.update(worker.demands(observations.warm_icons)[0].clone());
                if !auxiliary_slot.demand.active {
                    let _ = auxiliary.try_send(sampling_workers::Request::Release);
                }
                if auxiliary_slot.complete(completion.generation) {
                    diagnostics.record(Query::CpuAuxiliary, completion.duration_ms);
                    let expired = completion.duration_ms > MetricId::Cpu.freshness_ms();
                    if expired != auxiliary_expired {
                        log::info!(
                            "resident_cpu_auxiliary_state expired={expired} query_ms={}",
                            completion.duration_ms
                        );
                    }
                    auxiliary_expired = expired;
                    if expired {
                        diagnostics.discard(Query::CpuAuxiliary);
                    } else {
                        changed = true;
                        observations.apply(
                            *completion,
                            &mut cache,
                            &mut auxiliary_slot,
                            ObservationContext {
                                catalogue: &worker.catalogue,
                                now_ms: &|| origin.elapsed().as_millis() as u64,
                                previous_status: previous_status[0],
                            },
                            |_| {},
                        );
                    }
                } else {
                    diagnostics.discard(Query::CpuAuxiliary);
                }
                None
            }
            Some(SamplingEvent::DiskSensors(mut completion)) => {
                let demand = disk_sensor_demand(
                    worker.demands(observations.warm_icons)[4].clone(),
                    cache.disk_volume(),
                );
                if sensor_slot.demand.temperature_device != demand.temperature_device {
                    cache.clear_disk_temperature();
                }
                sensor_slot.update(demand);
                if !sensor_slot.demand.active {
                    let _ = disk_sensors.try_send(sampling_workers::Request::Release);
                }
                if sensor_slot.complete(completion.generation) {
                    diagnostics.record(Query::DiskSensors, completion.duration_ms);
                    if completion.duration_ms > MetricId::Disk.freshness_ms() {
                        diagnostics.discard(Query::DiskSensors);
                        completion.result = Ok(sampling_workers::Observation::DiskSensors(
                            Box::new(Err(mangodisk_platform::PlatformError::new(
                                mangodisk_platform::PlatformErrorCode::OperationFailed,
                                "disk_sensors stage=sample reason=observation_expired",
                            ))),
                        ));
                    }
                    changed = true;
                    observations.apply(
                        *completion,
                        &mut cache,
                        &mut sensor_slot,
                        ObservationContext {
                            catalogue: &worker.catalogue,
                            now_ms: &|| origin.elapsed().as_millis() as u64,
                            previous_status: previous_status[4],
                        },
                        |_| {},
                    );
                } else {
                    diagnostics.discard(Query::DiskSensors);
                }
                None
            }
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
            Some(SamplingEvent::DiskIo(completion)) => {
                if disk_slot.update(worker.disk_activity_demand()) {
                    cache.stop_disk_io();
                    log::info!("resident_disk_io_demand active={}", disk_slot.demand.active);
                }
                if disk_slot.complete(completion.generation) {
                    changed = true;
                    diagnostics.record(Query::DiskIo, completion.duration_ms);
                    if completion.duration_ms > 5000 {
                        cache.fail_disk_io(MetricStatus::Stale);
                    } else {
                        match completion.result {
                            Ok(devices) => cache.disk_io(
                                devices,
                                completion.monotonic_ms,
                                completion.timestamp_ms,
                            ),
                            Err(error) => {
                                if disk_status != MetricStatus::Unsupported {
                                    log::warn!(
                                        "resident_disk_io_unavailable code={:?} error={}",
                                        error.code(),
                                        mangodisk_platform::diagnostics::text(&error)
                                    );
                                }
                                cache.fail_disk_io(MetricStatus::Unsupported);
                            }
                        }
                    }
                } else {
                    diagnostics.discard(Query::DiskIo);
                }
                None
            }
            event => event,
        };
        if let Some(SamplingEvent::Completed(completion)) = event {
            let index = MetricId::ALL
                .iter()
                .position(|metric| *metric == completion.metric)
                .expect("known metric");
            // Reconcile demand before accepting a completion: disable/selection
            // commands may have raced the queued wake and this native result.
            let demand = metric_demand(
                completion.metric,
                worker.demands(observations.warm_icons)[index].clone(),
            );
            let selection_changed = slots[index].demand.selection != demand.selection;
            let temperature_device_changed =
                slots[index].demand.temperature_device != demand.temperature_device;
            if slots[index].update(demand) {
                if temperature_device_changed {
                    cache.clear_disk_temperature();
                }
                if selection_changed {
                    cache.reset(completion.metric);
                } else if !slots[index].demand.active {
                    cache.suspend(completion.metric);
                }
            }
            if !slots[index].demand.active {
                // Disabled workers receive no further periodic queries; explicitly
                // release native readers after a previously queued sample finishes.
                let _ = jobs[index].try_send(sampling_workers::Request::Release);
            }
            if slots[index].complete(completion.generation) {
                changed = true;
                diagnostics.record(completion.metric.into(), completion.duration_ms);
                if completion.duration_ms > completion.metric.freshness_ms() {
                    if previous_status[index] != MetricStatus::Stale {
                        log::warn!(
                            "resident_sample_expired metric={:?} duration_ms={}",
                            completion.metric,
                            completion.duration_ms
                        );
                    }
                    cache.fail(completion.metric, MetricStatus::Stale);
                    diagnostics.discard(completion.metric.into());
                } else {
                    observations.apply(
                        *completion,
                        &mut cache,
                        &mut slots[index],
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
                diagnostics.discard(completion.metric.into());
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
                            slots[1].demand.selection.is_none(),
                            resources.gpu_detail_history.len(),
                            resources.gpu_temperature_history.len()
                        );
                    }
                }
                if resources.network.status == MetricStatus::Ready {
                    if let Some(network) = &resources.network.value {
                        if reading
                            .resources
                            .network
                            .value
                            .as_ref()
                            .is_none_or(|previous| previous.interface.id != network.interface.id)
                        {
                            log::info!(
                                "resident_network_selected reason={:?}",
                                network.selection_reason
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

// Auxiliary controls must not invalidate a valid utilization interval.
fn metric_demand(metric: MetricId, demand: Demand) -> Demand {
    if metric == MetricId::Cpu {
        Demand {
            active: demand.active,
            ..Default::default()
        }
    } else if metric == MetricId::Disk {
        Demand {
            active: demand.active,
            selection: demand.selection,
            ..Default::default()
        }
    } else {
        demand
    }
}

fn disk_sensor_demand(
    mut demand: Demand,
    confirmed: Option<&mangodisk_platform::system_resources::disk::ResourceVolume>,
) -> Demand {
    // Core attributes health/details to a confirmed capacity source. Wait for that
    // identity before warm-up; an independently selected temperature stays usable.
    let matches = confirmed.is_some_and(|volume| {
        demand
            .selection
            .as_ref()
            .map_or(volume.system, |id| id == &volume.id)
    });
    demand.disk_health &= matches;
    demand.detailed &= matches;
    demand.active = demand.active && (demand.disk_health || demand.temperature || demand.detailed);
    demand
}

#[cfg(test)]
mod tests {
    use super::*;
    use mangodisk_platform::system_resources::disk::ResourceVolume;

    #[test]
    fn sensors_wait_for_matching_capacity_without_blocking_independent_temperature() {
        let volume = ResourceVolume {
            id: "system".into(),
            name: "Disk".into(),
            system: true,
            mount_point: "/".into(),
            mount_points: vec![],
            file_system: None,
            devices: vec![],
        };
        let demand = Demand {
            active: true,
            disk_health: true,
            detailed: true,
            ..Default::default()
        };
        assert!(!disk_sensor_demand(demand.clone(), None).active);
        assert!(disk_sensor_demand(demand.clone(), Some(&volume)).active);
        let changed = Demand {
            selection: Some("other".into()),
            ..demand.clone()
        };
        assert!(!disk_sensor_demand(changed, Some(&volume)).active);
        let temperature = disk_sensor_demand(
            Demand {
                temperature: true,
                temperature_device: Some("fixed-device".into()),
                ..demand.clone()
            },
            None,
        );
        assert!(temperature.active && temperature.temperature);
        assert!(!temperature.disk_health && !temperature.detailed);
        let capacity = metric_demand(MetricId::Disk, demand);
        assert!(capacity.active);
        assert!(!capacity.disk_health && !capacity.detailed && !capacity.temperature);
    }
}
