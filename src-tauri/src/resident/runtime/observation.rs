//! Apply accepted observations and retain bounded startup/recovery state.
use crate::resident::{
    sampling_schedule::SamplingSlot,
    sampling_workers::{Completion, Observation},
};
use mangodisk_core::system_resources::{
    cpu::CpuBaselineReason,
    metrics::{MetricId, MetricStatus},
    models::ProcessMemorySummary,
    readings::ResourceCache,
};
use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Default)]
pub(super) struct ObservationState {
    pub(super) warm_icons: bool,
    cpu_baseline: Option<CpuBaselineReason>,
    cpu_retries: u8,
    frequency_retries: u8,
    frequency_generation: Option<u64>,
    gpu_retries: u8,
    memory_process_failed: bool,
    disk_io_error: Option<mangodisk_platform::PlatformErrorCode>,
}

pub(super) struct ObservationContext<'a> {
    pub(super) catalogue: &'a AtomicU8,
    pub(super) now_ms: &'a dyn Fn() -> u64,
    pub(super) previous_status: MetricStatus,
}

impl ObservationState {
    pub(super) fn new(warm_icons: bool) -> Self {
        Self {
            warm_icons,
            ..Self::default()
        }
    }

    pub(super) fn worker_closed(
        &mut self,
        task: super::super::sampling_policy::Task,
        cache: &mut ResourceCache,
        now_ms: u64,
    ) {
        use super::super::sampling_policy::Task;
        let error = || {
            mangodisk_platform::PlatformError::new(
                mangodisk_platform::PlatformErrorCode::OperationFailed,
                format!("sampling task={task:?} stage=dispatch reason=worker_closed"),
            )
        };
        match task {
            Task::CpuTemperature => cache.cpu_temperature(Some(Err(error())), now_ms),
            Task::CpuFrequency => cache.fail_cpu_frequency(),
            Task::Battery => cache.battery(Err(error()), now_ms),
            Task::PowerSensors => {
                cache.cpu_power(Err(error()), now_ms);
                cache.system_power(Err(error()), now_ms);
                cache.fan_speed(Err(error()), now_ms);
            }
            Task::DiskSensors => {
                cache.disk_health(Err(error()), now_ms);
                cache.disk_temperature(Err(error()), now_ms);
                cache.disk_detail_temperature(Err(error()), now_ms);
            }
            Task::Disk => cache.fail_disk_capacity(MetricStatus::Failed),
            Task::Memory => cache.fail_memory_overview(MetricStatus::Failed),
            Task::MemoryProcesses => {
                cache.memory_processes(mangodisk_core::system_resources::metrics::MetricReading {
                    status: MetricStatus::Failed,
                    sampled_at_ms: None,
                    value: None,
                })
            }
            Task::CpuProcesses => {
                cache.cpu_processes(mangodisk_core::system_resources::metrics::MetricReading {
                    status: MetricStatus::Failed,
                    sampled_at_ms: None,
                    value: None,
                })
            }
            Task::DiskIo => cache.fail_disk_io(MetricStatus::Failed),
            Task::Gpu => {
                cache.fail(MetricId::Gpu, MetricStatus::Failed);
                cache.gpu_temperatures(Err(error()), now_ms);
                cache.gpu_power(Err(error()), now_ms);
                #[cfg(windows)]
                cache.fan_speed(Err(error()), now_ms);
            }
            _ => cache.fail(task.metric(), MetricStatus::Failed),
        }
    }

    pub(super) fn apply(
        &mut self,
        completion: Completion,
        cache: &mut ResourceCache,
        slot: &mut SamplingSlot,
        context: ObservationContext<'_>,
        warm_application_icons: impl FnOnce(ProcessMemorySummary),
    ) {
        let ObservationContext {
            catalogue,
            now_ms,
            previous_status,
        } = context;
        match completion.result {
            Ok(Observation::PowerSensors(observation)) => {
                let super::super::power_sensor_sampling::Observation {
                    system_power,
                    cpu_power,
                    fan_speed,
                } = *observation;
                if let Some(sample) = fan_speed {
                    let at = sample.sampled_at_ms;
                    cache.fan_speed(sample.result(completion.timestamp_ms, 5000), at);
                }
                if let Some(sample) = cpu_power {
                    let at = sample.sampled_at_ms;
                    cache.cpu_power(sample.result(completion.timestamp_ms, 5000), at);
                }
                if let Some(sample) = system_power {
                    let at = sample.sampled_at_ms;
                    cache.system_power(sample.result(completion.timestamp_ms, 5000), at);
                }
            }
            Ok(Observation::CpuFrequency(details)) => {
                if self.frequency_generation.replace(completion.generation)
                    != Some(completion.generation)
                {
                    self.frequency_retries = 0;
                }
                let details_at = details.sampled_at_ms;
                let details_fresh = details.fresh(completion.timestamp_ms, 5000);
                let mut details = details.value;
                if !details_fresh && matches!(details.frequency, Some(Ok(_))) {
                    details.frequency = Some(Err(expired_observation()));
                }
                if matches!(&details.frequency, Some(Ok(None))) {
                    if self.frequency_retries < 2 {
                        self.frequency_retries += 1;
                        slot.retry_after(now_ms(), 250);
                    }
                } else {
                    self.frequency_retries = 0;
                }
                cache.cpu_frequency(details.identity, details.frequency, details_at);
            }
            Ok(Observation::CpuTemperature(details)) => {
                let fresh = details.fresh(completion.timestamp_ms, 10_000);
                let mut details = details.value;
                if !fresh && matches!(details.temperature, Some(Ok(_))) {
                    details.temperature = Some(Err(expired_observation()));
                }
                cache.cpu_identity(details.identity);
                cache.cpu_temperature(details.temperature, completion.timestamp_ms);
            }
            Ok(Observation::Battery(observation)) => {
                let present = if observation.present.fresh(completion.timestamp_ms, 5000) {
                    observation.present.value
                } else {
                    None
                };
                cache.battery_presence(present, completion.timestamp_ms);
                if let Some(sample) = observation.sample {
                    let at = sample.sampled_at_ms;
                    cache.battery(sample.result(completion.timestamp_ms, 5000), at);
                }
            }
            Ok(Observation::Cpu { identity, sample }) => {
                cache.cpu_identity(identity);
                if let Err(error) = &sample {
                    let status =
                        if error.code() == mangodisk_platform::PlatformErrorCode::Unsupported {
                            MetricStatus::Unsupported
                        } else {
                            MetricStatus::Failed
                        };
                    if previous_status != status {
                        log::warn!(
                            "resident_sample_failed metric=Cpu code={:?} error={}",
                            error.code(),
                            mangodisk_platform::diagnostics::text(error)
                        );
                    }
                    cache.fail(MetricId::Cpu, status);
                }
                if let Ok(counters) = sample {
                    if let Some(reason) =
                        cache.cpu(counters, completion.monotonic_ms, completion.timestamp_ms)
                    {
                        if self.cpu_baseline != Some(reason) {
                            log::info!(
                                "resident_cpu_baseline reason={:?} query_ms={}",
                                reason,
                                completion.duration_ms
                            );
                        }
                        self.cpu_baseline = Some(reason);
                        // At most two quick retries per recovery episode. Native
                        // jobs remain serialized and persistent failures return to
                        // normal cadence rather than creating a tight polling loop.
                        if self.cpu_retries < 2 {
                            self.cpu_retries += 1;
                            slot.retry_after(now_ms(), 250);
                        }
                    } else {
                        if self.cpu_baseline.take().is_some() {
                            let cpu_retries = self.cpu_retries;
                            log::info!("resident_cpu_recovered quick_retries={cpu_retries}");
                        }
                        self.cpu_retries = 0;
                    }
                }
            }
            Ok(Observation::GpuCatalogue(adapters)) => {
                cache.gpu_adapters(adapters);
                catalogue.fetch_and(!4, Ordering::Relaxed);
            }
            Ok(Observation::Gpu {
                power,
                fan_speed,
                sample,
                adapters,
                temperature,
            }) => {
                if let Some(sample) = fan_speed {
                    let at = sample.sampled_at_ms;
                    cache.fan_speed(sample.result(completion.timestamp_ms, 5000), at);
                }
                if let Some(sample) = power {
                    let at = sample.sampled_at_ms;
                    cache.gpu_power(sample.result(completion.timestamp_ms, 5000), at);
                }
                let missing_selection = slot
                    .demand
                    .selection
                    .as_deref()
                    .is_some_and(|id| !adapters.iter().any(|adapter| adapter.id == id));
                cache.gpu_adapters(adapters);
                let sample_at = sample.sampled_at_ms;
                match sample.result(completion.timestamp_ms, MetricId::Gpu.freshness_ms()) {
                    Ok(sample) => {
                        let baseline = matches!(
                            sample,
                            mangodisk_platform::system_resources::gpu::GpuSample::Baseline
                        );
                        cache.gpu(sample, slot.demand.selection.as_deref(), sample_at);
                        if baseline && self.gpu_retries < 2 {
                            self.gpu_retries += 1;
                            slot.retry_after(now_ms(), 250);
                        } else {
                            self.gpu_retries = 0;
                            catalogue.fetch_and(!4, Ordering::Relaxed);
                        }
                    }
                    Err(error) => {
                        let status = if missing_selection {
                            MetricStatus::Disconnected
                        } else if error.code() == mangodisk_platform::PlatformErrorCode::Unsupported
                        {
                            MetricStatus::Unsupported
                        } else {
                            MetricStatus::Failed
                        };
                        if status == MetricStatus::Failed && previous_status != status {
                            log::warn!(
                                "resident_sample_failed metric=Gpu code={:?} error={}",
                                error.code(),
                                mangodisk_platform::diagnostics::text(&error)
                            );
                        }
                        cache.fail(MetricId::Gpu, status);
                        catalogue.fetch_and(!4, Ordering::Relaxed);
                    }
                }
                if let Some(temperature) = temperature {
                    let at = temperature.sampled_at_ms;
                    cache.gpu_temperatures(temperature.result(completion.timestamp_ms, 10000), at);
                }
            }
            Ok(Observation::DiskIo(result)) => {
                if completion.duration_ms > 5000 {
                    cache.fail_disk_io(MetricStatus::Stale);
                } else {
                    match result {
                        Ok(devices) => {
                            cache.disk_io(devices, completion.monotonic_ms, completion.timestamp_ms)
                        }
                        Err(error) => {
                            if self.disk_io_error != Some(error.code()) {
                                log::warn!(
                                    "resident_disk_io_unavailable code={:?} error={}",
                                    error.code(),
                                    mangodisk_platform::diagnostics::text(&error)
                                );
                            }
                            self.disk_io_error = Some(error.code());
                            let status = if error.code()
                                == mangodisk_platform::PlatformErrorCode::Unsupported
                            {
                                MetricStatus::Unsupported
                            } else {
                                MetricStatus::Failed
                            };
                            cache.fail_disk_io(status);
                        }
                    }
                }
            }
            Ok(Observation::Memory(snapshot)) => cache.memory(snapshot),
            Ok(Observation::MemoryProcesses(observed)) => {
                use mangodisk_core::system_resources::metrics::MetricReading;
                let valid =
                    observed.fresh(completion.timestamp_ms, MetricId::Memory.freshness_ms());
                let at = observed.sampled_at_ms;
                match observed.value {
                    Ok(summary) if valid => {
                        if self.memory_process_failed {
                            log::info!("resident_memory_processes_recovered");
                        }
                        self.memory_process_failed = false;
                        if self.warm_icons {
                            warm_application_icons(summary.clone());
                        }
                        cache.memory_processes(MetricReading::ready(summary, at));
                    }
                    result => {
                        if !self.memory_process_failed {
                            let error = match result {
                                Ok(_) => "observation expired".into(),
                                Err(error) => mangodisk_platform::diagnostics::text(&error),
                            };
                            log::warn!("resident_memory_processes_failed sampled_at_ms={at} observed_at_ms={} error={error}", completion.timestamp_ms);
                        }
                        self.memory_process_failed = true;
                        cache.memory_processes(MetricReading {
                            status: MetricStatus::Failed,
                            sampled_at_ms: None,
                            value: None,
                        });
                    }
                }
                self.warm_icons = false;
            }
            Ok(Observation::Network(interfaces)) => {
                cache.network(
                    interfaces,
                    slot.demand.selection.as_deref(),
                    completion.monotonic_ms,
                    completion.timestamp_ms,
                );
                catalogue.fetch_and(!1, Ordering::Relaxed);
            }
            Ok(Observation::Disk { volumes, selected }) => {
                cache.volumes(volumes);
                if let Some((volume, capacity)) = selected {
                    match capacity {
                        Ok(capacity) => cache.disk(*volume, capacity, completion.timestamp_ms),
                        Err(_) => cache.fail_disk_capacity(MetricStatus::Failed),
                    }
                } else {
                    cache.fail_disk_capacity(MetricStatus::Disconnected);
                }
                catalogue.fetch_and(!2, Ordering::Relaxed);
            }
            Ok(Observation::DiskSensors(result)) => {
                let super::super::disk_sensor_sampling::Observation {
                    health,
                    temperature,
                    detail_temperature,
                } = match *result {
                    Ok(observation) => observation,
                    Err(error) => {
                        log::warn!(
                            "resident_disk_sensors_failed code={:?} error={}",
                            error.code(),
                            mangodisk_platform::diagnostics::text(&error)
                        );
                        super::super::disk_sensor_sampling::Observation {
                            health: slot
                                .demand
                                .disk_health
                                .then(|| (completion.timestamp_ms, Err(error.clone()))),
                            temperature: slot
                                .demand
                                .temperature
                                .then(|| (completion.timestamp_ms, Err(error.clone()))),
                            detail_temperature: slot
                                .demand
                                .detailed
                                .then_some((completion.timestamp_ms, Err(error))),
                        }
                    }
                };
                if let Some((sampled_at_ms, temperature)) = temperature {
                    cache.disk_temperature(temperature, sampled_at_ms);
                } else if slot.demand.temperature {
                    cache.disk_temperature(
                        Err(mangodisk_platform::PlatformError::item_changed(
                            "disk_temperature selected volume unavailable",
                        )),
                        completion.timestamp_ms,
                    );
                }
                if let Some((sampled_at_ms, result)) = detail_temperature {
                    cache.disk_detail_temperature(result, sampled_at_ms);
                } else if slot.demand.detailed {
                    cache.disk_detail_temperature(
                        Err(mangodisk_platform::PlatformError::item_changed(
                            "disk_temperature detail volume unavailable",
                        )),
                        completion.timestamp_ms,
                    );
                }
                // Hiding details stops temperature demand, not the last
                // observation's lifetime. Source changes still reset it.
                if let Some((sampled_at_ms, health)) = health {
                    cache.disk_health(health, sampled_at_ms);
                } else if slot.demand.disk_health {
                    cache.disk_health(
                        Err(mangodisk_platform::PlatformError::item_changed(
                            "disk_health selected volume unavailable",
                        )),
                        completion.timestamp_ms,
                    );
                }
            }
            #[cfg(test)]
            Ok(Observation::Unsupported) => {
                cache.fail(completion.task.metric(), MetricStatus::Unsupported);
            }
            Err(error) => {
                if previous_status != MetricStatus::Failed {
                    log::warn!(
                        "resident_sample_failed metric={:?} code={:?} error={}",
                        completion.task.metric(),
                        error.code(),
                        mangodisk_platform::diagnostics::text(&error)
                    );
                }
                if completion.task == super::super::sampling_policy::Task::Memory {
                    cache.fail_memory_overview(MetricStatus::Failed);
                } else if completion.task.metric() == MetricId::Disk {
                    cache.fail_disk_capacity(MetricStatus::Failed);
                } else {
                    cache.fail(completion.task.metric(), MetricStatus::Failed);
                }
                if completion.task.metric() == MetricId::Network {
                    catalogue.fetch_and(!1, Ordering::Relaxed);
                }
                if completion.task.metric() == MetricId::Disk {
                    catalogue.fetch_and(!2, Ordering::Relaxed);
                }
                if completion.task.metric() == MetricId::Gpu {
                    catalogue.fetch_and(!4, Ordering::Relaxed);
                }
            }
        }
    }
}

fn expired_observation() -> mangodisk_platform::PlatformError {
    mangodisk_platform::PlatformError::new(
        mangodisk_platform::PlatformErrorCode::OperationFailed,
        "sampling stage=validate reason=observation_expired",
    )
}

#[cfg(test)]
#[path = "observation_tests.rs"]
mod tests;
