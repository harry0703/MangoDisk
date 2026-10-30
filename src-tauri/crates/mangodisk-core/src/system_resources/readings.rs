//! Independent cached results and interval calculations; no clocks, threads, or desktop APIs.
mod component_power;
mod cpu;
mod disk;
mod power;
use mangodisk_platform::system_resources::{
    battery::{BatteryPower, BatteryTemperature, ChargingPower, BATTERY_FRESHNESS_MS},
    cpu::{
        details::{CpuFrequency, CpuIdentity},
        temperature::CpuTemperature,
    },
    disk::ResourceVolume,
    network::{InterfaceSample, NetworkInterface},
};
use serde::Serialize;
use std::{collections::BTreeMap, sync::Arc};

use super::{
    cpu::CpuDelta,
    disk::DiskUsage,
    disk_io::{DiskIoDelta, DiskIoRate},
    gpu::{self, GpuUsage},
    metrics::{CpuUsage, MetricId, MetricReading, MetricStatus, Trend, TrendHistory, TrendPoint},
    models::{ProcessCpuSummary, SystemResourceSnapshot},
    network::{self, NetworkDelta, NetworkRate, NetworkSelectionReason},
};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GpuDeviceReading {
    pub adapter_id: String,
    pub adapter_name: String,
    pub usage: MetricReading<GpuUsage>,
    pub usage_history: TrendHistory,
    pub temperature:
        MetricReading<mangodisk_platform::system_resources::gpu::temperature::GpuTemperature>,
    pub temperature_history: TrendHistory,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceReadings {
    pub schema_version: u32,
    pub observed_at_ms: u64,
    pub cpu: MetricReading<CpuUsage>,
    pub cpu_identity: Option<CpuIdentity>,
    pub cpu_frequency: MetricReading<CpuFrequency>,
    pub cpu_temperature: MetricReading<CpuTemperature>,
    pub cpu_temperature_history: TrendHistory,
    pub gpu_temperature:
        MetricReading<mangodisk_platform::system_resources::gpu::temperature::GpuTemperature>,
    pub gpu_temperature_history: TrendHistory,
    pub battery_details:
        MetricReading<mangodisk_platform::system_resources::battery::details::BatteryDetails>,
    pub system_power_history: TrendHistory,
    pub battery_temperature: MetricReading<BatteryTemperature>,
    pub battery_present: Option<bool>,
    pub charging_power: MetricReading<ChargingPower>,
    pub battery_power: MetricReading<BatteryPower>,
    pub fan_speed: MetricReading<mangodisk_platform::system_resources::fans::FanSample>,
    pub cpu_power:
        MetricReading<mangodisk_platform::system_resources::component_power::ComponentPower>,
    pub gpu_power: MetricReading<mangodisk_platform::system_resources::component_power::GpuPower>,
    pub system_power: MetricReading<mangodisk_platform::system_resources::power::SystemPower>,
    pub gpu: MetricReading<GpuUsage>,
    pub gpu_details: MetricReading<GpuUsage>,
    pub gpu_detail_history: TrendHistory,
    pub gpu_detail_adapter_id: Option<String>,
    pub gpu_renderer_history: TrendHistory,
    pub gpu_tiler_history: TrendHistory,
    pub cpu_processes: MetricReading<Arc<ProcessCpuSummary>>,
    pub memory_processes: MetricReading<Arc<super::models::ProcessMemorySummary>>,
    pub memory: MetricReading<SystemResourceSnapshot>,
    pub network: MetricReading<NetworkRate>,
    pub disk: MetricReading<DiskUsage>,
    pub disk_temperature:
        MetricReading<mangodisk_platform::system_resources::disk_temperature::DiskTemperature>,
    pub disk_detail_temperature:
        MetricReading<mangodisk_platform::system_resources::disk_temperature::DiskTemperature>,
    pub disk_io: MetricReading<DiskIoRate>,
    pub disk_health: MetricReading<mangodisk_platform::system_resources::disk_health::DiskHealth>,
    pub interfaces: Vec<NetworkInterface>,
    pub volumes: Arc<[ResourceVolume]>,
    pub gpu_adapters: Vec<mangodisk_platform::system_resources::gpu::GpuAdapter>,
    pub gpu_devices: Vec<GpuDeviceReading>,
    pub cpu_history: TrendHistory,
    pub gpu_history: TrendHistory,
    pub network_history: TrendHistory,
    pub memory_history: TrendHistory,
    pub disk_io_history: TrendHistory,
}

impl Default for ResourceReadings {
    fn default() -> Self {
        Self {
            schema_version: 39,
            observed_at_ms: 0,
            cpu: MetricReading::default(),
            cpu_identity: None,
            cpu_frequency: MetricReading::default(),
            cpu_temperature: MetricReading::default(),
            cpu_temperature_history: Default::default(),
            gpu_temperature: MetricReading::default(),
            gpu_temperature_history: Default::default(),
            battery_details: MetricReading::default(),
            system_power_history: Default::default(),
            battery_temperature: MetricReading::default(),
            battery_present: None,
            charging_power: MetricReading::default(),
            battery_power: MetricReading::default(),
            fan_speed: MetricReading::default(),
            cpu_power: component_power::initial(),
            gpu_power: component_power::initial(),
            system_power: MetricReading::default(),
            gpu: MetricReading::default(),
            gpu_details: MetricReading::default(),
            gpu_detail_history: Default::default(),
            gpu_detail_adapter_id: None,
            gpu_renderer_history: Default::default(),
            gpu_tiler_history: Default::default(),
            cpu_processes: MetricReading::default(),
            memory_processes: MetricReading::default(),
            memory: MetricReading::default(),
            network: MetricReading::default(),
            disk: MetricReading::default(),
            disk_temperature: MetricReading::default(),
            disk_detail_temperature: MetricReading::default(),
            disk_io: MetricReading::default(),
            disk_health: MetricReading::default(),
            interfaces: Vec::new(),
            volumes: Arc::from([]),
            gpu_adapters: Vec::new(),
            gpu_devices: Vec::new(),
            cpu_history: Default::default(),
            gpu_history: Default::default(),
            network_history: Default::default(),
            memory_history: Default::default(),
            disk_io_history: Default::default(),
        }
    }
}

#[derive(Default)]
pub struct ResourceCache {
    readings: ResourceReadings,
    cpu_delta: CpuDelta,
    network_delta: NetworkDelta,
    network_selection: network::NetworkSelection,
    network_interrupted: bool,
    disk_io_delta: DiskIoDelta,
    system_power_history: Trend,
    system_power_interrupted: bool,
    cpu_history: Trend,
    cpu_temperature_history: Trend,
    cpu_temperature_interrupted: bool,
    cpu_temperature_identity: Option<cpu::TemperatureIdentity>,
    gpu_temperature: super::gpu_temperature::GpuTemperatureCache,
    gpu_history: Trend,
    gpu_device_histories: BTreeMap<String, super::gpu_history::GpuDeviceHistory>,
    gpu_detail_device: Option<String>,
    network_history: Trend,
    memory_history: Trend,
    disk_io_history: Trend,
    selected_interface: Option<String>,
}

impl ResourceCache {
    pub fn set_network_traffic(
        &mut self,
        traffic: std::sync::Arc<super::network_traffic::NetworkTraffic>,
    ) {
        self.network_delta.set_traffic(traffic);
    }
    pub fn fan_speed(
        &mut self,
        sample: mangodisk_platform::PlatformResult<
            mangodisk_platform::system_resources::fans::FanSample,
        >,
        timestamp_ms: u64,
    ) {
        self.readings.fan_speed = match sample {
            Ok(value) if value.is_valid() => MetricReading::ready(value, timestamp_ms),
            result => MetricReading {
                status: match result {
                    Err(error)
                        if error.code() == mangodisk_platform::PlatformErrorCode::Unsupported =>
                    {
                        MetricStatus::Unsupported
                    }
                    _ => MetricStatus::Failed,
                },
                sampled_at_ms: None,
                value: None,
            },
        };
    }

    pub fn gpu_temperatures(
        &mut self,
        samples: mangodisk_platform::PlatformResult<
            Vec<mangodisk_platform::system_resources::gpu::temperature::GpuTemperatureSample>,
        >,
        timestamp_ms: u64,
    ) {
        let automatic = self
            .readings
            .gpu
            .value
            .as_ref()
            .filter(|_| self.readings.gpu.status == MetricStatus::Ready)
            .map(|value| value.adapter_id.as_str());
        match samples {
            Ok(samples) => self
                .gpu_temperature
                .apply(samples, None, automatic, timestamp_ms),
            Err(error) => self.gpu_temperature.error(&error),
        }
    }

    pub fn gpu(
        &mut self,
        sample: mangodisk_platform::system_resources::gpu::GpuSample,
        selected: Option<&str>,
        timestamp_ms: u64,
    ) {
        match sample {
            mangodisk_platform::system_resources::gpu::GpuSample::Baseline => {
                self.readings
                    .gpu
                    .expire(timestamp_ms, MetricId::Gpu.freshness_ms());
            }
            mangodisk_platform::system_resources::gpu::GpuSample::Usage(adapters) => {
                for (id, history) in &mut self.gpu_device_histories {
                    if !adapters.iter().any(|adapter| {
                        adapter.id == *id
                            && adapter.used_percent.is_finite()
                            && (0.0..=100.0).contains(&adapter.used_percent)
                    }) {
                        history.fail(MetricStatus::Failed);
                    }
                }
                for adapter in &adapters {
                    if adapter.used_percent.is_finite()
                        && (0.0..=100.0).contains(&adapter.used_percent)
                    {
                        let replaced = self
                            .gpu_device_histories
                            .entry(adapter.id.clone())
                            .or_default()
                            .observe(adapter, timestamp_ms);
                        if replaced {
                            self.gpu_temperature
                                .invalidate_replaced_device(&adapter.id, &adapter.name);
                            if self.gpu_detail_device.as_deref() == Some(&adapter.id) {
                                self.readings.gpu_details = MetricReading::default();
                            }
                        }
                    }
                }
                if let Some(mut value) = gpu::select(adapters, selected) {
                    if self.gpu_detail_device.as_deref() != Some(&value.adapter_id) {
                        self.readings.gpu_details = MetricReading::default();
                        self.gpu_detail_device = Some(value.adapter_id.clone());
                    }
                    self.gpu_temperature.select(Some(&value.adapter_id));
                    if value.details.is_some() {
                        self.readings.gpu_details =
                            MetricReading::ready(value.clone(), timestamp_ms);
                    }
                    // Details belong to a selected device, not to the system maximum's history.
                    value.details = None;
                    self.gpu_history.push(TrendPoint {
                        sampled_at_ms: timestamp_ms,
                        primary: value.used_percent,
                        secondary: None,
                        gap_before: false,
                    });
                    self.readings.gpu = MetricReading::ready(value, timestamp_ms);
                } else {
                    let status = if selected.is_some_and(|id| {
                        !self
                            .readings
                            .gpu_adapters
                            .iter()
                            .any(|adapter| adapter.id == id)
                    }) {
                        MetricStatus::Disconnected
                    } else {
                        MetricStatus::Failed
                    };
                    self.readings.gpu.status = status;
                    self.readings.gpu_details.status = status;
                }
            }
        }
    }

    pub fn gpu_adapters(
        &mut self,
        adapters: Vec<mangodisk_platform::system_resources::gpu::GpuAdapter>,
    ) {
        // Catalogue captions include display ordinals, which can change when another
        // GPU disappears. Native sample names detect replacements independently.
        let present = |id: &str| adapters.iter().any(|adapter| adapter.id == id);
        if self
            .readings
            .gpu_power
            .value
            .as_ref()
            .is_some_and(|power| !present(&power.adapter_id))
        {
            self.readings
                .gpu_power
                .mark_unavailable(MetricStatus::Disconnected);
        }
        self.gpu_device_histories.retain(|id, _| present(id));
        self.gpu_temperature.retain(present);
        for adapter in &adapters {
            self.gpu_device_histories
                .entry(adapter.id.clone())
                .or_default();
        }
        self.readings.gpu_adapters = adapters;
    }

    pub fn memory(&mut self, mut snapshot: SystemResourceSnapshot) {
        if let Some(summary) = snapshot.processes.take() {
            self.readings.memory_processes =
                MetricReading::ready(Arc::new(summary), snapshot.sampled_at_ms);
        }
        let sampled_at_ms = snapshot.sampled_at_ms;
        self.memory_history.push(TrendPoint {
            sampled_at_ms,
            primary: f64::from(snapshot.memory.used_percent),
            secondary: None,
            gap_before: false,
        });
        self.readings.memory = MetricReading::ready(snapshot, sampled_at_ms);
    }

    pub fn disk_io(
        &mut self,
        devices: Vec<mangodisk_platform::system_resources::disk_io::DeviceCounters>,
        monotonic_ms: u64,
        timestamp_ms: u64,
    ) {
        if let Some(value) = self.disk_io_delta.sample(devices, monotonic_ms) {
            self.disk_io_history.push(TrendPoint {
                sampled_at_ms: timestamp_ms,
                primary: value.read_bytes_per_second,
                secondary: Some(value.written_bytes_per_second),
                gap_before: false,
            });
            self.readings.disk_io = MetricReading::ready(value, timestamp_ms);
        } else {
            self.readings.disk_io.expire(timestamp_ms, 5000);
        }
    }
    pub fn stop_disk_io(&mut self) {
        // Closing a panel pauses collection, not the history of real activity.
        self.disk_io_delta.reset();
    }
    pub fn fail_disk_io(&mut self, status: MetricStatus) {
        self.readings.disk_io.status = status;
        self.disk_io_delta.reset();
    }

    pub fn network(
        &mut self,
        interfaces: Vec<InterfaceSample>,
        manual: Option<&str>,
        monotonic_ms: u64,
        timestamp_ms: u64,
    ) -> NetworkSelectionReason {
        self.readings.interfaces = network::catalogue(&interfaces, manual);
        let (selected, reason) = self.network_selection.select(
            &interfaces,
            manual,
            self.selected_interface.as_deref(),
            monotonic_ms,
        );
        let Some(selected) = selected else {
            self.readings.network.status = MetricStatus::Disconnected;
            self.network_delta.reset();
            self.network_interrupted = true;
            return reason;
        };
        if self.selected_interface.as_deref() != Some(&selected.interface.id) {
            self.network_history.clear();
            self.network_delta.reset();
            self.network_interrupted = false;
            self.readings.network = MetricReading::default();
        }
        self.selected_interface = Some(selected.interface.id.clone());
        let Some(counters) = selected.counters else {
            // Enumeration succeeded: one adapter's unreadable counters must
            // not cancel confirmation of a healthy replacement interface.
            self.readings.network.status = MetricStatus::Failed;
            self.network_delta.reset();
            self.network_interrupted = true;
            return reason;
        };
        if let Some(value) = self.network_delta.sample(
            &selected.interface,
            counters,
            monotonic_ms,
            reason,
            timestamp_ms,
        ) {
            self.network_history.push(TrendPoint {
                sampled_at_ms: timestamp_ms,
                primary: value.received_bytes_per_second,
                secondary: Some(value.transmitted_bytes_per_second),
                gap_before: std::mem::take(&mut self.network_interrupted),
            });
            self.readings.network = MetricReading::ready(value, timestamp_ms);
        } else {
            self.readings
                .network
                .expire(timestamp_ms, MetricId::Network.freshness_ms());
        }
        reason
    }

    pub fn cpu_processes(&mut self, reading: MetricReading<ProcessCpuSummary>) {
        if reading.value.is_some() || self.readings.cpu_processes.value.is_none() {
            self.readings.cpu_processes = MetricReading {
                status: reading.status,
                sampled_at_ms: reading.sampled_at_ms,
                value: reading.value.map(Arc::new),
            };
        } else if reading.status != MetricStatus::Loading {
            // A failed query changes freshness, not the last successful observation.
            self.readings.cpu_processes.status = reading.status;
        }
        // Baseline-only observations retain the original timestamp. snapshot() expires
        // them normally; explicit disabling still clears the ranking and its identity.
    }

    pub fn memory_processes(
        &mut self,
        reading: MetricReading<super::models::ProcessMemorySummary>,
    ) {
        if reading.value.is_some() || self.readings.memory_processes.value.is_none() {
            self.readings.memory_processes = MetricReading {
                status: reading.status,
                sampled_at_ms: reading.sampled_at_ms,
                value: reading.value.map(Arc::new),
            };
        } else if reading.status != MetricStatus::Loading {
            self.readings.memory_processes.status = reading.status;
        }
    }

    pub fn fail_memory_overview(&mut self, status: MetricStatus) {
        self.readings.memory.status = status;
    }

    pub fn clear_cpu_processes(&mut self) {
        self.readings.cpu_processes = MetricReading::default();
    }

    pub fn fail(&mut self, metric: MetricId, status: MetricStatus) {
        match metric {
            MetricId::Cpu => {
                self.readings.cpu.status = status;
                self.cpu_delta.reset();
            }
            MetricId::Memory => {
                self.readings.memory.status = status;
                self.readings.memory_processes.status = status;
            }
            MetricId::Network => {
                self.readings.network.status = status;
                self.network_delta.reset();
                self.network_selection.interrupt();
                self.network_interrupted = true;
            }
            MetricId::Gpu => {
                self.readings.gpu.status = status;
                self.readings.gpu_details.status = status;
                for history in self.gpu_device_histories.values_mut() {
                    // Catalogue removal owns disconnection. A missing selected
                    // device does not disconnect other catalogued GPUs.
                    history.fail(if status == MetricStatus::Disconnected {
                        MetricStatus::Failed
                    } else {
                        status
                    });
                }
            }
            MetricId::Disk => {
                self.readings.disk.status = status;
                self.readings.disk_health.mark_unavailable(status);
                self.readings.disk_temperature.mark_unavailable(status);
                self.readings
                    .disk_detail_temperature
                    .mark_unavailable(status);
            }
        }
    }

    /// Pausing collection preserves real history and original freshness. A new
    /// baseline prevents rates from averaging across the disabled interval.
    pub fn suspend(&mut self, metric: MetricId) {
        match metric {
            MetricId::Cpu => self.cpu_delta.reset(),
            MetricId::Network => {
                self.network_delta.reset();
                self.network_selection.interrupt();
                self.network_interrupted = true;
            }
            MetricId::Memory => self.readings.memory_processes = MetricReading::default(),
            MetricId::Gpu | MetricId::Disk => {}
        }
    }

    /// A changed source must not attribute an old device's history to a new one.
    pub fn reset(&mut self, metric: MetricId) {
        match metric {
            MetricId::Cpu => {
                self.cpu_delta.reset();
                self.cpu_history.clear();
                self.readings.cpu = MetricReading::default();
                self.readings.cpu_frequency = MetricReading::default();
                self.readings.cpu_temperature = MetricReading::default();
                self.cpu_temperature_history.clear();
                self.cpu_temperature_interrupted = false;
                self.cpu_temperature_identity = None;
            }
            MetricId::Memory => {
                self.readings.memory = MetricReading::default();
                self.memory_history.clear();
            }
            MetricId::Network => {
                self.network_delta.reset();
                self.network_selection.interrupt();
                self.network_interrupted = false;
                self.network_history.clear();
                self.readings.network = MetricReading::default();
            }
            MetricId::Gpu => {
                self.gpu_temperature.clear_selection();
                self.gpu_history.clear();
                self.gpu_detail_device = None;
                self.readings.gpu_details = MetricReading::default();
                self.readings.gpu = MetricReading::default();
            }
            MetricId::Disk => {
                self.readings.disk = MetricReading::default();
                self.readings.disk_health = MetricReading::default();
                self.clear_disk_temperature();
                self.readings.disk_detail_temperature = MetricReading::default();
            }
        }
    }

    /// Resume counter baselines without inventing activity across suspension.
    pub fn resume_sampling(&mut self) {
        self.cpu_delta.reset();
        self.network_delta.reset();
        self.network_selection.interrupt();
        self.network_interrupted = true;
        self.disk_io_delta.reset();
        self.cpu_temperature_interrupted = true;
        self.system_power_interrupted = true;
    }

    /// Update freshness without copying application identities or rebuilding histories.
    pub fn expire(&mut self, now_ms: u64) -> bool {
        let before = self.statuses();
        self.readings.cpu_power.expire(now_ms, 5_000);
        self.readings.gpu_power.expire(now_ms, 5_000);
        let mut gpu_changed = false;
        for history in self.gpu_device_histories.values_mut() {
            let previous = history.reading.status;
            history.reading.expire(now_ms, MetricId::Gpu.freshness_ms());
            gpu_changed |= previous != history.reading.status;
        }
        gpu_changed |= self.gpu_temperature.expire(now_ms);
        self.readings.gpu_temperature = self.gpu_temperature.reading.clone();
        self.readings.observed_at_ms = now_ms;
        self.readings
            .cpu
            .expire(now_ms, MetricId::Cpu.freshness_ms());
        self.readings
            .memory
            .expire(now_ms, MetricId::Memory.freshness_ms());
        self.readings
            .network
            .expire(now_ms, MetricId::Network.freshness_ms());
        self.readings
            .disk
            .expire(now_ms, MetricId::Disk.freshness_ms());
        self.readings
            .gpu
            .expire(now_ms, MetricId::Gpu.freshness_ms());
        self.readings
            .battery_temperature
            .expire(now_ms, BATTERY_FRESHNESS_MS);
        self.readings
            .charging_power
            .expire(now_ms, BATTERY_FRESHNESS_MS);
        self.readings
            .battery_power
            .expire(now_ms, BATTERY_FRESHNESS_MS);
        self.readings.fan_speed.expire(
            now_ms,
            mangodisk_platform::system_resources::fans::FAN_FRESHNESS_MS,
        );
        self.readings
            .battery_details
            .expire(now_ms, BATTERY_FRESHNESS_MS);
        self.readings.system_power.expire(
            now_ms,
            mangodisk_platform::system_resources::power::SYSTEM_POWER_FRESHNESS_MS,
        );
        self.readings.disk_health.expire(
            now_ms,
            mangodisk_platform::system_resources::disk_health::DISK_HEALTH_FRESHNESS_MS,
        );
        self.readings.disk_io.expire(now_ms, 5000);
        self.readings.disk_detail_temperature.expire(
            now_ms,
            mangodisk_platform::system_resources::disk_temperature::DISK_TEMPERATURE_FRESHNESS_MS,
        );
        self.readings.disk_temperature.expire(
            now_ms,
            mangodisk_platform::system_resources::disk_temperature::DISK_TEMPERATURE_FRESHNESS_MS,
        );
        self.readings
            .cpu_frequency
            .expire(now_ms, MetricId::Cpu.freshness_ms());
        let temperature_cleared = self.expire_cpu_temperature(now_ms);
        self.readings
            .memory_processes
            .expire(now_ms, MetricId::Memory.freshness_ms());
        self.readings
            .cpu_processes
            .expire(now_ms, MetricId::Cpu.freshness_ms());
        self.readings
            .gpu_details
            .expire(now_ms, MetricId::Gpu.freshness_ms());
        temperature_cleared || gpu_changed || before != self.statuses()
    }

    fn statuses(&self) -> [MetricStatus; 23] {
        [
            self.readings.cpu_power.status,
            self.readings.gpu_power.status,
            self.readings.battery_details.status,
            self.readings.fan_speed.status,
            self.readings.system_power.status,
            self.readings.battery_temperature.status,
            self.readings.charging_power.status,
            self.readings.battery_power.status,
            self.readings.cpu.status,
            self.readings.cpu_frequency.status,
            self.readings.cpu_temperature.status,
            self.readings.gpu_temperature.status,
            self.readings.gpu.status,
            self.readings.gpu_details.status,
            self.readings.memory.status,
            self.readings.network.status,
            self.readings.disk.status,
            self.readings.disk_temperature.status,
            self.readings.disk_detail_temperature.status,
            self.readings.disk_health.status,
            self.readings.disk_io.status,
            self.readings.cpu_processes.status,
            self.readings.memory_processes.status,
        ]
    }

    pub fn snapshot(&mut self, now_ms: u64) -> ResourceReadings {
        self.expire(now_ms);
        self.readings.gpu_devices = self
            .readings
            .gpu_adapters
            .iter()
            .map(|adapter| {
                let history = self.gpu_device_histories.get_mut(&adapter.id);
                let (usage, usage_history) = history
                    .map(|history| (history.reading.clone(), history.usage.snapshot(now_ms)))
                    .unwrap_or_else(|| (MetricReading::default(), Default::default()));
                let (temperature, temperature_history) =
                    self.gpu_temperature.device_snapshot(&adapter.id, now_ms);
                GpuDeviceReading {
                    adapter_id: adapter.id.clone(),
                    adapter_name: adapter.name.clone(),
                    usage,
                    usage_history,
                    temperature,
                    temperature_history,
                }
            })
            .collect();
        self.readings.gpu_temperature_history = self.gpu_temperature.history(now_ms);
        self.readings.memory_history = self.memory_history.snapshot(now_ms);
        self.readings.disk_io_history = self.disk_io_history.snapshot(now_ms);
        self.readings.system_power_history = self.system_power_history.snapshot(now_ms);
        self.readings.cpu_history = self.cpu_history.snapshot(now_ms);
        self.readings.cpu_temperature_history = self.cpu_temperature_history.snapshot(now_ms);
        self.readings.gpu_history = self.gpu_history.snapshot(now_ms);
        self.readings
            .gpu_detail_adapter_id
            .clone_from(&self.gpu_detail_device);
        if let Some(history) = self
            .gpu_detail_device
            .as_ref()
            .and_then(|id| self.gpu_device_histories.get_mut(id))
        {
            self.readings.gpu_detail_history = history.usage.snapshot(now_ms);
            self.readings.gpu_renderer_history = history.renderer.snapshot(now_ms);
            self.readings.gpu_tiler_history = history.tiler.snapshot(now_ms);
        } else {
            self.readings.gpu_detail_history = Default::default();
            self.readings.gpu_renderer_history = Default::default();
            self.readings.gpu_tiler_history = Default::default();
        }
        self.readings.network_history = self.network_history.snapshot(now_ms);
        if let Some(network) = self.readings.network.value.as_mut() {
            network.cumulative = self.network_delta.cumulative(&network.interface.id);
        }
        self.readings.clone()
    }
}

#[cfg(test)]
#[path = "readings_tests.rs"]
mod tests;
