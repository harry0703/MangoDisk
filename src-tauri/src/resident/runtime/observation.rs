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
    gpu_retries: u8,
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
            Ok(Observation::CpuAuxiliary(observation)) => {
                let super::super::cpu_auxiliary_sampling::Observation {
                    details,
                    battery,
                    system_power,
                    cpu_power,
                    battery_present,
                    fan_speed,
                } = *observation;
                if matches!(&details.frequency, Some(Ok(None))) {
                    if self.frequency_retries < 2 {
                        self.frequency_retries += 1;
                        slot.retry_after(now_ms(), 250);
                    }
                } else {
                    self.frequency_retries = 0;
                }
                cache.battery_presence(battery_present, completion.timestamp_ms);
                if let Some(sample) = fan_speed {
                    cache.fan_speed(sample, completion.timestamp_ms);
                }
                if let Some(sample) = cpu_power {
                    cache.cpu_power(sample, completion.timestamp_ms);
                }
                if let Some(sample) = system_power {
                    cache.system_power(sample, completion.timestamp_ms);
                }
                if let Some(sample) = battery {
                    cache.battery(sample, completion.timestamp_ms);
                }
                cache.cpu_details(details, completion.timestamp_ms);
            }
            Ok(Observation::Cpu { sample }) => {
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
                    cache.fan_speed(sample, completion.timestamp_ms);
                }
                if let Some(sample) = power {
                    cache.gpu_power(sample, completion.timestamp_ms);
                }
                let missing_selection = slot
                    .demand
                    .selection
                    .as_deref()
                    .is_some_and(|id| !adapters.iter().any(|adapter| adapter.id == id));
                cache.gpu_adapters(adapters);
                match sample {
                    Ok(sample) => {
                        let baseline = matches!(
                            sample,
                            mangodisk_platform::system_resources::gpu::GpuSample::Baseline
                        );
                        cache.gpu(
                            sample,
                            slot.demand.selection.as_deref(),
                            completion.timestamp_ms,
                        );
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
                    cache.gpu_temperatures(temperature, completion.timestamp_ms);
                }
            }
            Ok(Observation::Memory(snapshot)) => {
                let warm_summary = self
                    .warm_icons
                    .then(|| snapshot.processes.clone())
                    .flatten();
                cache.memory(snapshot);
                if self.warm_icons {
                    if let Some(summary) = warm_summary {
                        warm_application_icons(summary);
                    }
                    self.warm_icons = false;
                }
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
                cache.fail(completion.metric, MetricStatus::Unsupported);
            }
            Err(error) => {
                if completion.metric == MetricId::Memory {
                    self.warm_icons = false;
                }
                if previous_status != MetricStatus::Failed {
                    log::warn!(
                        "resident_sample_failed metric={:?} code={:?} error={}",
                        completion.metric,
                        error.code(),
                        mangodisk_platform::diagnostics::text(&error)
                    );
                }
                if completion.metric == MetricId::Disk {
                    cache.fail_disk_capacity(MetricStatus::Failed);
                } else {
                    cache.fail(completion.metric, MetricStatus::Failed);
                }
                if completion.metric == MetricId::Network {
                    catalogue.fetch_and(!1, Ordering::Relaxed);
                }
                if completion.metric == MetricId::Disk {
                    catalogue.fetch_and(!2, Ordering::Relaxed);
                }
                if completion.metric == MetricId::Gpu {
                    catalogue.fetch_and(!4, Ordering::Relaxed);
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "observation_tests.rs"]
mod tests;
