//! Independent cached results and interval calculations; no clocks, threads, or desktop APIs.
mod component_power;
mod power;
use mangodisk_platform::system_resources::{
    battery::{BatteryPower, BatteryTemperature, ChargingPower, BATTERY_FRESHNESS_MS},
    cpu::{
        details::{CpuDetails, CpuFrequency, CpuIdentity},
        temperature::{CpuTemperature, TEMPERATURE_FRESHNESS_MS},
        CpuSample,
    },
    disk::{ResourceVolume, VolumeCapacity},
    network::{InterfaceSample, NetworkInterface},
};
use serde::Serialize;
use std::{collections::BTreeMap, sync::Arc};

use super::{
    cpu::{CpuBaselineReason, CpuDelta},
    disk::{self, DiskUsage},
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
    pub volumes: Vec<ResourceVolume>,
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
            schema_version: 37,
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
            volumes: Vec::new(),
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
    disk_io_delta: DiskIoDelta,
    system_power_history: Trend,
    system_power_interrupted: bool,
    cpu_history: Trend,
    cpu_temperature_history: Trend,
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
    pub fn cpu(
        &mut self,
        counters: impl Into<CpuSample>,
        monotonic_ms: u64,
        timestamp_ms: u64,
    ) -> Option<CpuBaselineReason> {
        match self.cpu_delta.observe(counters.into(), monotonic_ms) {
            Ok(value) => {
                self.cpu_history.push(TrendPoint {
                    sampled_at_ms: timestamp_ms,
                    primary: value.used_percent,
                    secondary: None,
                    gap_before: false,
                });
                self.readings.cpu = MetricReading::ready(value, timestamp_ms);
                None
            }
            Err(reason) => {
                // A rejected interval is not a new reading. Keep the last real
                // value and history with their original timestamps until expiry;
                // rebuilding one baseline must not erase an entire minute.
                self.readings
                    .cpu
                    .expire(timestamp_ms, MetricId::Cpu.freshness_ms());
                Some(reason)
            }
        }
    }

    pub fn cpu_details(&mut self, details: CpuDetails, timestamp_ms: u64) {
        self.readings.cpu_identity = Some(details.identity);
        match details.temperature {
            Some(Ok(value))
                if value.celsius.is_finite()
                    && value.celsius > 0.0
                    && value.celsius <= 150.0
                    && value.sensor_count > 0
                    && value.sampled_at_ms.is_none_or(|sampled| {
                        sampled <= timestamp_ms
                            && timestamp_ms - sampled <= TEMPERATURE_FRESHNESS_MS
                    }) =>
            {
                let sampled_at_ms = value.sampled_at_ms.unwrap_or(timestamp_ms);
                let repeated = self.readings.cpu_temperature.sampled_at_ms == Some(sampled_at_ms)
                    && self
                        .readings
                        .cpu_temperature
                        .value
                        .as_ref()
                        .is_some_and(|previous| {
                            previous.kind == value.kind
                                && previous.source == value.source
                                && previous.sensor_count == value.sensor_count
                        });
                if self
                    .readings
                    .cpu_temperature
                    .value
                    .as_ref()
                    .is_some_and(|previous| {
                        previous.kind != value.kind
                            || previous.source != value.source
                            || previous.sensor_count != value.sensor_count
                    })
                {
                    self.cpu_temperature_history.clear();
                }
                // Service cache hits are the same native observation. Do not
                // reset the trend or manufacture a newer sample timestamp.
                if !repeated {
                    self.cpu_temperature_history.push(TrendPoint {
                        sampled_at_ms,
                        primary: value.celsius,
                        secondary: None,
                        gap_before: false,
                    });
                }
                self.readings.cpu_temperature = MetricReading::ready(value, sampled_at_ms);
            }
            Some(result) => {
                let unsupported = result.as_ref().err().is_some_and(|error| {
                    error.code() == mangodisk_platform::PlatformErrorCode::Unsupported
                });
                self.readings.cpu_temperature = MetricReading {
                    status: if unsupported {
                        MetricStatus::Unsupported
                    } else {
                        MetricStatus::Failed
                    },
                    value: None,
                    sampled_at_ms: None,
                };
                // A recovery must not connect through a known failed observation or
                // combine different sensor sets after the old value is discarded.
                self.cpu_temperature_history.clear();
            }
            None => self
                .readings
                .cpu_temperature
                .expire(timestamp_ms, TEMPERATURE_FRESHNESS_MS),
        }
        match details.frequency {
            Some(Ok(Some(value))) => {
                self.readings.cpu_frequency = MetricReading::ready(value, timestamp_ms)
            }
            Some(Err(error)) => {
                self.readings.cpu_frequency.status =
                    if error.code() == mangodisk_platform::PlatformErrorCode::Unsupported {
                        MetricStatus::Unsupported
                    } else {
                        MetricStatus::Failed
                    };
                self.readings.cpu_frequency.value = None;
                self.readings.cpu_frequency.sampled_at_ms = None;
            }
            _ => self
                .readings
                .cpu_frequency
                .expire(timestamp_ms, MetricId::Cpu.freshness_ms()),
        }
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
        let (selected, reason) =
            network::select(&interfaces, manual, self.selected_interface.as_deref());
        let Some(selected) = selected.filter(|sample| sample.interface.connected) else {
            self.fail(MetricId::Network, MetricStatus::Disconnected);
            return reason;
        };
        if self.selected_interface.as_deref() != Some(&selected.interface.id) {
            self.network_history.clear();
            self.readings.network = MetricReading::default();
        }
        self.selected_interface = Some(selected.interface.id.clone());
        let Some(counters) = selected.counters else {
            self.fail(MetricId::Network, MetricStatus::Failed);
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
                gap_before: false,
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

    pub fn clear_cpu_processes(&mut self) {
        self.readings.cpu_processes = MetricReading::default();
    }

    pub fn volumes(&mut self, volumes: Vec<ResourceVolume>) {
        self.readings.volumes = volumes;
    }

    pub fn disk(&mut self, volume: ResourceVolume, capacity: VolumeCapacity, timestamp_ms: u64) {
        if let Some(value) = disk::usage(volume, capacity) {
            if self
                .readings
                .disk_health
                .value
                .as_ref()
                .is_some_and(|health| health.volume_id != value.volume.id)
            {
                self.readings.disk_health = MetricReading::default();
                self.readings.disk_detail_temperature = MetricReading::default();
            }
            self.readings.disk = MetricReading::ready(value, timestamp_ms);
        } else {
            self.fail(MetricId::Disk, MetricStatus::Failed);
        }
    }

    pub fn disk_health(
        &mut self,
        result: mangodisk_platform::PlatformResult<
            mangodisk_platform::system_resources::disk_health::DiskHealth,
        >,
        timestamp_ms: u64,
    ) {
        match result {
            Ok(value)
                if !value.volume_id.is_empty()
                    && self
                        .readings
                        .disk
                        .value
                        .as_ref()
                        .is_some_and(|disk| disk.volume.id == value.volume_id) =>
            {
                self.readings.disk_health = MetricReading::ready(value, timestamp_ms)
            }
            result => {
                // An unrelated completion cannot be reused as this disk's observation.
                let status = match result {
                    Err(error)
                        if error.code() == mangodisk_platform::PlatformErrorCode::Unsupported =>
                    {
                        MetricStatus::Unsupported
                    }
                    Err(error)
                        if error.code() == mangodisk_platform::PlatformErrorCode::ItemChanged =>
                    {
                        MetricStatus::Disconnected
                    }
                    Err(_) => MetricStatus::Failed,
                    Ok(_) => MetricStatus::Disconnected,
                };
                self.readings.disk_health.mark_unavailable(status);
            }
        }
    }

    pub fn clear_disk_temperature(&mut self) {
        self.readings.disk_temperature = MetricReading::default();
    }

    pub fn disk_temperature(
        &mut self,
        result: mangodisk_platform::PlatformResult<
            mangodisk_platform::system_resources::disk_temperature::DiskTemperature,
        >,
        timestamp_ms: u64,
    ) {
        self.readings.disk_temperature = Self::temperature_reading(result, timestamp_ms);
    }

    pub fn disk_detail_temperature(
        &mut self,
        result: mangodisk_platform::PlatformResult<
            mangodisk_platform::system_resources::disk_temperature::DiskTemperature,
        >,
        timestamp_ms: u64,
    ) {
        self.readings.disk_detail_temperature = Self::temperature_reading(result, timestamp_ms);
    }

    fn temperature_reading(
        result: mangodisk_platform::PlatformResult<
            mangodisk_platform::system_resources::disk_temperature::DiskTemperature,
        >,
        timestamp_ms: u64,
    ) -> MetricReading<mangodisk_platform::system_resources::disk_temperature::DiskTemperature>
    {
        let unavailable = |status| MetricReading {
            status,
            sampled_at_ms: None,
            value: None,
        };
        match result {
            Ok(value) if value.valid() => MetricReading::ready(value, timestamp_ms),
            Ok(_) => unavailable(MetricStatus::Failed),
            Err(error) => unavailable(
                if error.code() == mangodisk_platform::PlatformErrorCode::Unsupported {
                    MetricStatus::Unsupported
                } else if error.code() == mangodisk_platform::PlatformErrorCode::ItemChanged {
                    MetricStatus::Disconnected
                } else {
                    MetricStatus::Failed
                },
            ),
        }
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
            MetricId::Network => self.network_delta.reset(),
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
            }
            MetricId::Memory => {
                self.readings.memory = MetricReading::default();
                self.memory_history.clear();
            }
            MetricId::Network => {
                self.network_delta.reset();
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
        self.readings
            .cpu_temperature
            .expire(now_ms, TEMPERATURE_FRESHNESS_MS);
        self.readings
            .memory_processes
            .expire(now_ms, MetricId::Memory.freshness_ms());
        self.readings
            .cpu_processes
            .expire(now_ms, MetricId::Cpu.freshness_ms());
        self.readings
            .gpu_details
            .expire(now_ms, MetricId::Gpu.freshness_ms());
        gpu_changed || before != self.statuses()
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
mod tests {
    use mangodisk_platform::system_resources::battery::{BatteryPowerState, BatterySample};
    #[test]
    fn disk_temperature_has_independent_freshness_and_clears_failed_or_changed_sources() {
        use super::*;
        use mangodisk_platform::system_resources::disk_temperature::{
            DiskTemperature, DiskTemperatureSource,
        };
        let mut cache = ResourceCache::default();
        let sample = || DiskTemperature {
            volume_id: "system".into(),
            volume_name: "Disk".into(),
            celsius: 34.85,
            source: DiskTemperatureSource::NvmeSmart,
        };
        cache.disk_temperature(Ok(sample()), 1000);
        assert_eq!(
            cache.snapshot(91_000).disk_temperature.status,
            MetricStatus::Ready
        );
        assert_eq!(
            cache.snapshot(91_001).disk_temperature.status,
            MetricStatus::Stale
        );
        cache.disk_temperature(
            Err(mangodisk_platform::PlatformError::new(
                mangodisk_platform::PlatformErrorCode::Unsupported,
                "no sensor",
            )),
            92_000,
        );
        assert_eq!(
            cache.snapshot(92_000).disk_temperature.status,
            MetricStatus::Unsupported
        );
        assert!(cache.snapshot(92_000).disk_temperature.value.is_none());
        let mut invalid = sample();
        invalid.celsius = f64::NAN;
        cache.disk_temperature(Ok(invalid), 93_000);
        assert_eq!(
            cache.snapshot(93_000).disk_temperature.status,
            MetricStatus::Failed
        );
        cache.disk_temperature(Ok(sample()), 94_000);
        cache.reset(MetricId::Disk);
        assert!(cache.snapshot(94_000).disk_temperature.value.is_none());
        cache.disk_temperature(Ok(sample()), 95_000);
        cache.clear_disk_temperature();
        assert_eq!(
            cache.snapshot(95_000).disk_temperature.status,
            MetricStatus::Loading
        );
    }
    #[test]
    fn absent_battery_clears_only_battery_values_and_unknown_preserves_presence() {
        use super::*;
        use mangodisk_platform::system_resources::{battery::BatterySource, power::SystemPower};
        let mut cache = ResourceCache::default();
        cache.system_power(Ok(SystemPower { watts: 25.0 }), 1000);
        cache.battery_presence(Some(true), 1000);
        cache.battery(
            Ok(BatterySample {
                details: None,
                temperature: Some(BatteryTemperature {
                    celsius: 30.0,
                    source: BatterySource::AppleSmc,
                }),
                charging_power: Some(ChargingPower {
                    watts: 0.0,
                    charging: false,
                    external_connected: true,
                    source: BatterySource::AppleSmartBattery,
                }),
                battery_power: Some(BatteryPower {
                    watts: 0.0,
                    state: BatteryPowerState::Idle,
                    source: BatterySource::AppleSmartBattery,
                }),
            }),
            1000,
        );
        cache.battery_presence(None, 1100);
        assert_eq!(cache.snapshot(1100).battery_present, Some(true));
        cache.battery_presence(Some(false), 1200);
        let reading = cache.snapshot(1200);
        assert_eq!(reading.battery_present, Some(false));
        assert_eq!(
            reading.battery_temperature.status,
            MetricStatus::Unsupported
        );
        assert!(reading.battery_temperature.value.is_none());
        assert!(reading.charging_power.value.is_none());
        assert!(reading.battery_power.value.is_none());
        assert_eq!(reading.system_power.value.unwrap().watts, 25.0);
        cache.battery_presence(None, 1300);
        assert_eq!(cache.snapshot(1300).battery_present, Some(false));
    }

    #[test]
    fn system_power_expiry_and_failures_never_fabricate_zero_watts() {
        use super::*;
        use mangodisk_platform::{
            system_resources::power::SystemPower, PlatformError, PlatformErrorCode,
        };
        let mut cache = ResourceCache::default();
        cache.system_power(Ok(SystemPower { watts: 27.5 }), 1000);
        assert_eq!(
            cache.snapshot(6000).system_power.status,
            MetricStatus::Ready
        );
        assert_eq!(
            cache.snapshot(6001).system_power.status,
            MetricStatus::Stale
        );
        for (code, status) in [
            (PlatformErrorCode::Unsupported, MetricStatus::Unsupported),
            (PlatformErrorCode::OperationFailed, MetricStatus::Failed),
        ] {
            cache.system_power(Err(PlatformError::new(code, "SMC PSTR test failure")), 7000);
            let reading = cache.snapshot(7000).system_power;
            assert_eq!(reading.status, status);
            assert!(reading.value.is_none());
            assert!(reading.sampled_at_ms.is_none());
        }
        for watts in [f64::NAN, f64::INFINITY, -1.0, 3000.1] {
            cache.system_power(Ok(SystemPower { watts }), 8000);
            assert_eq!(
                cache.snapshot(8000).system_power.status,
                MetricStatus::Failed
            );
            assert!(cache.snapshot(8000).system_power.value.is_none());
        }
        cache.system_power(Ok(SystemPower { watts: 0.0 }), 9000);
        assert_eq!(cache.snapshot(9000).system_power.value.unwrap().watts, 0.0);
    }

    use super::*;
    use mangodisk_platform::system_resources::cpu::CpuCounters;

    #[test]
    fn battery_fields_expire_independently_and_failures_cannot_become_zero_watts() {
        use mangodisk_platform::system_resources::battery::BatterySource;
        let mut cache = ResourceCache::default();
        cache.battery(
            Ok(BatterySample {
                details: None,
                battery_power: Some(BatteryPower {
                    watts: 24.0,
                    state: BatteryPowerState::Discharging,
                    source: BatterySource::AppleSmartBattery,
                }),
                temperature: Some(BatteryTemperature {
                    celsius: 30.5,
                    source: BatterySource::AppleSmartBattery,
                }),
                charging_power: Some(ChargingPower {
                    watts: 0.0,
                    charging: false,
                    external_connected: true,
                    source: BatterySource::AppleSmartBattery,
                }),
            }),
            1000,
        );
        assert_eq!(
            cache.snapshot(6000).charging_power.status,
            MetricStatus::Ready
        );
        assert_eq!(
            cache.snapshot(6001).battery_temperature.status,
            MetricStatus::Stale
        );
        assert_eq!(
            cache.snapshot(6001).battery_power.status,
            MetricStatus::Stale
        );
        cache.battery(
            Ok(BatterySample {
                details: None,
                battery_power: None,
                temperature: None,
                charging_power: None,
            }),
            7000,
        );
        assert_eq!(
            cache.snapshot(7000).charging_power.status,
            MetricStatus::Unsupported
        );
        assert!(cache.snapshot(7000).charging_power.value.is_none());
        assert!(cache.snapshot(7000).battery_power.value.is_none());
        cache.battery(
            Err(mangodisk_platform::PlatformError::new(
                mangodisk_platform::PlatformErrorCode::OperationFailed,
                "read failed",
            )),
            8000,
        );
        assert_eq!(
            cache.snapshot(8000).battery_temperature.status,
            MetricStatus::Failed
        );
        assert!(cache.snapshot(8000).charging_power.value.is_none());
        assert_eq!(
            cache.snapshot(8000).battery_power.status,
            MetricStatus::Failed
        );
    }

    #[test]
    fn cached_native_temperature_preserves_history_and_original_freshness() {
        use mangodisk_platform::system_resources::cpu::temperature::{
            CpuTemperatureKind, CpuTemperatureSource,
        };
        let mut cache = ResourceCache::default();
        let details = |sampled_at_ms| CpuDetails {
            identity: CpuIdentity::default(),
            frequency: None,
            temperature: Some(Ok(CpuTemperature {
                celsius: 48.0,
                sensor_count: 1,
                kind: CpuTemperatureKind::Package,
                source: CpuTemperatureSource::IntelDtsPawnIo,
                sampled_at_ms: Some(sampled_at_ms),
            })),
        };
        cache.cpu_details(details(1000), 1000);
        cache.cpu_details(details(5000), 5000);
        cache.cpu_details(details(5000), 6000);
        let reading = cache.snapshot(6000);
        assert_eq!(reading.cpu_temperature_history.len(), 2);
        assert_eq!(reading.cpu_temperature.sampled_at_ms, Some(5000));
        assert_eq!(
            cache.snapshot(15001).cpu_temperature.status,
            MetricStatus::Stale
        );
    }

    #[test]
    fn native_temperature_rejects_future_or_expired_sample_timestamps() {
        use mangodisk_platform::system_resources::cpu::temperature::{
            CpuTemperatureKind, CpuTemperatureSource,
        };
        for sampled_at_ms in [1, 20_001] {
            let mut cache = ResourceCache::default();
            cache.cpu_details(
                CpuDetails {
                    identity: CpuIdentity::default(),
                    frequency: None,
                    temperature: Some(Ok(CpuTemperature {
                        celsius: 50.0,
                        kind: CpuTemperatureKind::Package,
                        source: CpuTemperatureSource::IntelDtsPawnIo,
                        sensor_count: 1,
                        sampled_at_ms: Some(sampled_at_ms),
                    })),
                },
                20_000,
            );
            let reading = cache.snapshot(20_000);
            assert_eq!(reading.cpu_temperature.status, MetricStatus::Failed);
            assert!(reading.cpu_temperature.value.is_none());
            assert!(reading.cpu_temperature_history.is_empty());
        }
    }

    #[test]
    fn temperature_freshness_failure_and_sensor_identity_are_independent_of_cpu_load() {
        use mangodisk_platform::system_resources::cpu::temperature::{
            CpuTemperatureKind, CpuTemperatureSource,
        };
        use mangodisk_platform::{PlatformError, PlatformErrorCode};
        let mut cache = ResourceCache::default();
        let details = |temperature| CpuDetails {
            identity: CpuIdentity::default(),
            frequency: None,
            temperature,
        };
        let value = || CpuTemperature {
            celsius: 48.5,
            sensor_count: 16,
            sampled_at_ms: None,
            kind: CpuTemperatureKind::CoreAverage,
            source: CpuTemperatureSource::AppleSmc,
        };
        cache.cpu_details(details(Some(Ok(value()))), 1000);
        cache.fail(MetricId::Cpu, MetricStatus::Failed);
        assert_eq!(
            cache.snapshot(1000).cpu_temperature.status,
            MetricStatus::Ready
        );
        cache.cpu_details(details(None), 5000);
        assert_eq!(
            cache.snapshot(5000).cpu_temperature.sampled_at_ms,
            Some(1000)
        );
        assert_eq!(
            cache.snapshot(11001).cpu_temperature.status,
            MetricStatus::Stale
        );
        cache.cpu_details(
            details(Some(Err(PlatformError::new(
                PlatformErrorCode::OperationFailed,
                "sensor disconnected",
            )))),
            12000,
        );
        let failed = cache.snapshot(12000);
        assert_eq!(failed.cpu_temperature.status, MetricStatus::Failed);
        assert!(failed.cpu_temperature.value.is_none());
        assert!(failed.cpu_temperature_history.is_empty());
        cache.cpu_details(details(Some(Ok(value()))), 13000);
        let mut changed = value();
        changed.sensor_count = 8;
        cache.cpu_details(details(Some(Ok(changed))), 17000);
        assert_eq!(cache.snapshot(17000).cpu_temperature_history.len(), 1);
        let mut invalid = value();
        invalid.celsius = f64::NAN;
        cache.cpu_details(details(Some(Ok(invalid))), 18000);
        assert_eq!(
            cache.snapshot(18000).cpu_temperature.status,
            MetricStatus::Failed
        );
        cache.cpu_details(
            details(Some(Err(PlatformError::new(
                PlatformErrorCode::Unsupported,
                "no CPU sensor",
            )))),
            19000,
        );
        assert_eq!(
            cache.snapshot(19000).cpu_temperature.status,
            MetricStatus::Unsupported
        );
        cache.reset(MetricId::Cpu);
        assert!(cache.snapshot(20000).cpu_temperature.value.is_none());
    }

    #[test]
    fn frequency_baselines_preserve_sample_age_and_failures_never_show_a_clock() {
        use mangodisk_platform::system_resources::cpu::details::CpuFrequencySource;
        use mangodisk_platform::{PlatformError, PlatformErrorCode};
        let mut cache = ResourceCache::default();
        let detail = |frequency| CpuDetails {
            identity: CpuIdentity {
                model: Some("CPU model".into()),
                nominal_frequency_mhz: Some(3700.0),
                frequency_classes: vec![
                    mangodisk_platform::system_resources::cpu::details::CpuFrequencyClass::Average,
                ],
            },
            frequency,
            temperature: None,
        };
        cache.cpu_details(
            detail(Some(Ok(Some(CpuFrequency {
                average_mhz: Some(4773.0),
                efficiency_mhz: None,
                performance_mhz: None,
                super_mhz: None,
                source: CpuFrequencySource::WindowsPerformance,
                activity: Vec::new(),
            })))),
            1000,
        );
        cache.cpu_details(detail(Some(Ok(None))), 2000);
        let readings = cache.snapshot(2000);
        assert_eq!(readings.cpu_frequency.sampled_at_ms, Some(1000));
        assert_eq!(
            readings.cpu_frequency.value.unwrap().average_mhz,
            Some(4773.0)
        );
        let aged = 1000 + MetricId::Cpu.freshness_ms() + 1;
        cache.cpu_details(detail(None), aged);
        assert_eq!(
            cache.snapshot(aged).cpu_frequency.status,
            MetricStatus::Stale
        );
        cache.cpu_details(
            detail(Some(Err(PlatformError::new(
                PlatformErrorCode::Unsupported,
                "no counter",
            )))),
            aged + 1,
        );
        let readings = cache.snapshot(aged + 1);
        assert_eq!(readings.cpu_frequency.status, MetricStatus::Unsupported);
        assert!(readings.cpu_frequency.value.is_none());
        cache.reset(MetricId::Cpu);
        assert_eq!(
            cache
                .snapshot(aged + 1)
                .cpu_identity
                .unwrap()
                .model
                .as_deref(),
            Some("CPU model")
        );
    }

    #[test]
    fn completed_idle_frequency_interval_clears_previous_clocks_and_recovers() {
        use mangodisk_platform::system_resources::cpu::details::CpuFrequencySource;
        let mut cache = ResourceCache::default();
        for (time, mhz) in [(1000, Some(2000.0)), (2000, None), (3000, Some(2500.0))] {
            cache.cpu_details(
                CpuDetails {
                    identity: CpuIdentity::default(),
                    frequency: Some(Ok(Some(CpuFrequency {
                        average_mhz: None,
                        efficiency_mhz: mhz,
                        performance_mhz: mhz,
                        super_mhz: None,
                        source: CpuFrequencySource::ApplePerformanceStates,
                        activity: Vec::new(),
                    }))),
                    temperature: None,
                },
                time,
            );
            let reading = cache.snapshot(time).cpu_frequency;
            assert_eq!(reading.status, MetricStatus::Ready);
            assert_eq!(reading.sampled_at_ms, Some(time));
            let value = reading.value.unwrap();
            assert_eq!(value.efficiency_mhz, mhz);
            assert_eq!(value.performance_mhz, mhz);
        }
    }

    #[test]
    fn disconnected_gpu_selection_preserves_catalogue_without_an_idle_sample() {
        use mangodisk_platform::system_resources::gpu::{GpuAdapter, GpuAdapterUsage, GpuSample};
        let mut cache = ResourceCache::default();
        cache.gpu_adapters(vec![GpuAdapter {
            id: "b".into(),
            name: "GPU B".into(),
        }]);
        cache.gpu(
            GpuSample::Usage(vec![GpuAdapterUsage {
                id: "b".into(),
                name: "GPU B".into(),
                used_percent: 56.0,
                details: None,
            }]),
            Some("a"),
            1000,
        );
        let result = cache.snapshot(1000);
        assert_eq!(result.gpu.status, MetricStatus::Disconnected);
        assert!(result.gpu.value.is_none());
        assert!(result.gpu_history.is_empty());
        assert_eq!(result.gpu_adapters.len(), 1);
    }
    #[test]
    fn gpu_baselines_and_failures_preserve_only_real_history_and_original_freshness() {
        use mangodisk_platform::system_resources::gpu::{GpuAdapterUsage, GpuSample};
        let mut cache = ResourceCache::default();
        cache.gpu(GpuSample::Baseline, None, 1000);
        assert!(cache.snapshot(1000).gpu.value.is_none());
        cache.gpu(
            GpuSample::Usage(vec![GpuAdapterUsage {
                id: "gpu-1".into(),
                name: "GPU 1".into(),
                used_percent: 0.0,
                details: None,
            }]),
            None,
            2000,
        );
        cache.gpu(GpuSample::Baseline, None, 3000);
        let valid = cache.snapshot(3000);
        assert_eq!(valid.gpu.sampled_at_ms, Some(2000));
        assert_eq!(valid.gpu_history.len(), 1);
        assert_eq!(valid.gpu.status, MetricStatus::Ready);
        assert_eq!(cache.snapshot(7001).gpu.status, MetricStatus::Stale);
        cache.fail(MetricId::Gpu, MetricStatus::Unsupported);
        assert_eq!(cache.snapshot(8000).gpu.status, MetricStatus::Unsupported);
        assert_eq!(cache.snapshot(8000).gpu_history.len(), 1);
        cache.reset(MetricId::Gpu);
        assert!(cache.snapshot(8000).gpu.value.is_none());
        assert!(cache.snapshot(8000).gpu_history.is_empty());
    }

    #[test]
    fn automatic_gpu_switches_restore_each_devices_observed_history() {
        use mangodisk_platform::system_resources::gpu::{
            temperature::{
                GpuTemperature, GpuTemperatureKind, GpuTemperatureSample, GpuTemperatureSource,
            },
            GpuAdapterUsage, GpuSample,
        };
        let mut cache = ResourceCache::default();
        let usage = |amd, intel| {
            GpuSample::Usage(
                [("amd", amd), ("intel", intel)]
                    .into_iter()
                    .map(|(id, used_percent)| GpuAdapterUsage {
                        id: id.into(),
                        name: id.into(),
                        used_percent,
                        details: None,
                    })
                    .collect(),
            )
        };
        let temperatures = |celsius| {
            Ok(vec![
                GpuTemperatureSample {
                    adapter_id: "amd".into(),
                    reading: Ok(GpuTemperature {
                        adapter_id: "amd".into(),
                        adapter_name: "AMD".into(),
                        celsius,
                        kind: GpuTemperatureKind::Device,
                        source: GpuTemperatureSource::Driver,
                        sensor_count: 1,
                    }),
                },
                GpuTemperatureSample {
                    adapter_id: "intel".into(),
                    reading: Err(mangodisk_platform::PlatformError::new(
                        mangodisk_platform::PlatformErrorCode::Unsupported,
                        "temperature unavailable",
                    )),
                },
            ])
        };
        cache.gpu(usage(30.0, 5.0), None, 1000);
        cache.gpu_temperatures(temperatures(53.0), 1000);
        cache.gpu(usage(5.0, 40.0), None, 3000);
        assert_eq!(cache.snapshot(3000).gpu.value.unwrap().adapter_id, "intel");
        cache.gpu(usage(6.0, 50.0), None, 5000);
        cache.gpu_temperatures(temperatures(54.0), 5000);
        cache.gpu(usage(35.0, 4.0), None, 7000);
        let restored = cache.snapshot(7000);
        assert_eq!(restored.gpu.value.unwrap().adapter_id, "amd");
        assert_eq!(
            restored
                .gpu_detail_history
                .iter()
                .map(|p| p.primary)
                .collect::<Vec<_>>(),
            vec![30.0, 5.0, 6.0, 35.0]
        );
        assert_eq!(
            restored
                .gpu_temperature_history
                .iter()
                .map(|p| p.sampled_at_ms)
                .collect::<Vec<_>>(),
            vec![1000, 5000]
        );
        assert_eq!(restored.gpu_temperature.sampled_at_ms, Some(5000));
        assert_eq!(restored.gpu_temperature.value.unwrap().celsius, 54.0);
    }

    #[test]
    fn gpu_selection_resets_restore_only_the_selected_devices_history_until_removal() {
        use mangodisk_platform::system_resources::gpu::{GpuAdapter, GpuAdapterUsage, GpuSample};
        let mut cache = ResourceCache::default();
        let catalogue = || {
            vec![
                GpuAdapter {
                    id: "a".into(),
                    name: "A".into(),
                },
                GpuAdapter {
                    id: "b".into(),
                    name: "B".into(),
                },
            ]
        };
        let sample = || {
            GpuSample::Usage(vec![
                GpuAdapterUsage {
                    id: "a".into(),
                    name: "A".into(),
                    used_percent: 10.0,
                    details: None,
                },
                GpuAdapterUsage {
                    id: "b".into(),
                    name: "B".into(),
                    used_percent: 80.0,
                    details: None,
                },
            ])
        };
        cache.gpu_adapters(catalogue());
        cache.gpu(sample(), Some("a"), 1000);
        cache.reset(MetricId::Gpu);
        assert!(cache.snapshot(2000).gpu_detail_history.is_empty());
        cache.gpu(sample(), Some("b"), 3000);
        let b = cache.snapshot(3000);
        assert_eq!(b.gpu_detail_history.len(), 2);
        assert!(b
            .gpu_detail_history
            .iter()
            .all(|point| point.primary == 80.0));
        cache.reset(MetricId::Gpu);
        cache.gpu(sample(), Some("a"), 19000);
        let a = cache.snapshot(19000);
        assert_eq!(
            a.gpu_detail_history
                .iter()
                .map(|point| point.sampled_at_ms)
                .collect::<Vec<_>>(),
            vec![1000, 3000, 19000]
        );
        assert!(a
            .gpu_detail_history
            .iter()
            .all(|point| point.primary == 10.0));
        cache.gpu_adapters(vec![GpuAdapter {
            id: "b".into(),
            name: "B".into(),
        }]);
        assert_eq!(cache.gpu_device_histories.len(), 1);
        assert!(cache.snapshot(19000).gpu_detail_history.is_empty());
        cache.gpu_adapters(catalogue());
        cache.gpu(sample(), Some("a"), 21000);
        assert_eq!(cache.snapshot(21000).gpu_detail_history.len(), 1);
        cache.gpu_adapters(vec![GpuAdapter {
            id: "a".into(),
            name: "Replacement".into(),
        }]);
        cache.gpu(
            GpuSample::Usage(vec![GpuAdapterUsage {
                id: "a".into(),
                name: "Replacement".into(),
                used_percent: 20.0,
                details: None,
            }]),
            Some("a"),
            23000,
        );
        let replaced = cache.snapshot(23000);
        assert_eq!(replaced.gpu_detail_history.len(), 1);
        assert_eq!(replaced.gpu_detail_history[0].primary, 20.0);
    }

    #[test]
    fn gpu_catalogue_renumbering_preserves_the_surviving_devices_histories() {
        use mangodisk_platform::system_resources::gpu::{
            temperature::{
                GpuTemperature, GpuTemperatureKind, GpuTemperatureSample, GpuTemperatureSource,
            },
            GpuAdapter, GpuAdapterUsage, GpuSample,
        };
        let mut cache = ResourceCache::default();
        cache.gpu_adapters(vec![
            GpuAdapter {
                id: "a".into(),
                name: "GPU 0 · A".into(),
            },
            GpuAdapter {
                id: "b".into(),
                name: "GPU 1 · B".into(),
            },
        ]);
        for timestamp in [1000, 5000] {
            cache.gpu(
                GpuSample::Usage(vec![GpuAdapterUsage {
                    id: "b".into(),
                    name: "B".into(),
                    used_percent: 80.0,
                    details: None,
                }]),
                Some("b"),
                timestamp,
            );
            cache.gpu_temperatures(
                Ok(vec![GpuTemperatureSample {
                    adapter_id: "b".into(),
                    reading: Ok(GpuTemperature {
                        adapter_id: "b".into(),
                        adapter_name: "B".into(),
                        celsius: 70.0,
                        kind: GpuTemperatureKind::Device,
                        source: GpuTemperatureSource::Driver,
                        sensor_count: 1,
                    }),
                }]),
                timestamp,
            );
        }
        cache.gpu_adapters(vec![GpuAdapter {
            id: "b".into(),
            name: "GPU 0 · B".into(),
        }]);
        let reading = cache.snapshot(6000);
        assert_eq!(reading.gpu_detail_history.len(), 2);
        assert_eq!(reading.gpu_temperature_history.len(), 2);
        assert_eq!(reading.gpu_temperature.sampled_at_ms, Some(5000));
        assert_eq!(reading.gpu_temperature.status, MetricStatus::Ready);
    }

    #[test]
    fn replacement_temperature_observed_before_utilization_recovery_stays_available() {
        use mangodisk_platform::system_resources::gpu::{
            temperature::{
                GpuTemperature, GpuTemperatureKind, GpuTemperatureSample, GpuTemperatureSource,
            },
            GpuAdapterUsage, GpuSample,
        };
        let mut cache = ResourceCache::default();
        let usage = |name: &str| {
            GpuSample::Usage(vec![GpuAdapterUsage {
                id: "a".into(),
                name: name.into(),
                used_percent: 10.0,
                details: None,
            }])
        };
        let temperature = |name: &str| {
            Ok(vec![GpuTemperatureSample {
                adapter_id: "a".into(),
                reading: Ok(GpuTemperature {
                    adapter_id: "a".into(),
                    adapter_name: name.into(),
                    celsius: 51.0,
                    kind: GpuTemperatureKind::Device,
                    source: GpuTemperatureSource::Driver,
                    sensor_count: 1,
                }),
            }])
        };
        cache.gpu(usage("Original"), Some("a"), 1000);
        cache.gpu_temperatures(temperature("Original"), 1000);
        cache.gpu(GpuSample::Baseline, Some("a"), 5000);
        cache.gpu_temperatures(temperature("Replacement"), 5000);
        cache.gpu(usage("Replacement"), Some("a"), 7000);
        let recovered = cache.snapshot(7000);
        assert_eq!(recovered.gpu_temperature.status, MetricStatus::Ready);
        assert_eq!(recovered.gpu_temperature.sampled_at_ms, Some(5000));
        assert_eq!(recovered.gpu_temperature_history.len(), 1);
        assert_eq!(recovered.gpu_detail_history.len(), 1);
    }

    #[test]
    fn detail_history_isolated_when_automatic_selection_changes_devices() {
        use mangodisk_platform::system_resources::gpu::{details::*, GpuAdapterUsage, GpuSample};
        let sample = |a: f64, b: f64, detailed: bool| {
            GpuSample::Usage(
                [("a", a), ("b", b)]
                    .into_iter()
                    .map(|(id, used_percent)| GpuAdapterUsage {
                        id: id.into(),
                        name: id.into(),
                        used_percent,
                        details: detailed.then_some(GpuDetails {
                            activities: vec![],
                            telemetry: GpuTelemetry::default(),
                            memory_architecture: GpuMemoryArchitecture::Unified,
                            memory_status: GpuMemoryStatus::Unsupported,
                            memory: None,
                        }),
                    })
                    .collect(),
            )
        };
        let mut cache = ResourceCache::default();
        cache.gpu(sample(30.0, 5.0, true), None, 1000);
        cache.gpu(sample(35.0, 5.0, true), None, 3000);
        assert_eq!(cache.snapshot(3000).gpu_detail_history.len(), 2);
        cache.gpu(sample(3.0, 56.0, true), None, 5000);
        let switched = cache.snapshot(5000);
        assert_eq!(switched.gpu_history.len(), 3);
        assert_eq!(switched.gpu_detail_history.len(), 3);
        assert_eq!(switched.gpu_details.value.as_ref().unwrap().adapter_id, "b");
        assert!(switched.gpu.value.as_ref().unwrap().details.is_none());
        cache.gpu(sample(5.0, 60.0, false), None, 7000);
        assert_eq!(cache.snapshot(7000).gpu_details.sampled_at_ms, Some(5000));
        assert_eq!(
            cache.snapshot(10001).gpu_details.status,
            MetricStatus::Stale
        );
        cache.fail(MetricId::Gpu, MetricStatus::Disconnected);
        assert_eq!(
            cache.snapshot(11000).gpu_details.status,
            MetricStatus::Disconnected
        );
        cache.reset(MetricId::Gpu);
        assert!(cache.snapshot(11000).gpu_detail_history.is_empty());
        assert!(cache.snapshot(11000).gpu_details.value.is_none());
    }

    #[test]
    fn gpu_background_samples_keep_the_device_curve_continuous_without_fabricating_engines() {
        use mangodisk_platform::system_resources::gpu::{details::*, GpuAdapterUsage, GpuSample};
        let sample = |id: &str, detailed: bool| {
            GpuSample::Usage(vec![GpuAdapterUsage {
                id: id.into(),
                name: id.into(),
                used_percent: 20.0,
                details: detailed.then_some(GpuDetails {
                    activities: vec![GpuActivity {
                        id: "renderer".into(),
                        kind: GpuActivityKind::Renderer,
                        name: None,
                        used_percent: 12.0,
                        included_in_summary: false,
                    }],
                    telemetry: GpuTelemetry::default(),
                    memory_architecture: GpuMemoryArchitecture::Unified,
                    memory_status: GpuMemoryStatus::Unsupported,
                    memory: None,
                }),
            }])
        };
        let mut cache = ResourceCache::default();
        for time in (1000..=11000).step_by(2000) {
            cache.gpu(sample("a", time == 1000 || time == 11000), None, time);
        }
        let reopened = cache.snapshot(11000);
        assert_eq!(reopened.gpu_detail_adapter_id.as_deref(), Some("a"));
        assert_eq!(reopened.gpu_detail_history.len(), 6);
        assert!(reopened
            .gpu_detail_history
            .windows(2)
            .all(|pair| pair[1].sampled_at_ms - pair[0].sampled_at_ms == 2000));
        assert_eq!(reopened.gpu_renderer_history.len(), 2);
        assert!(reopened.gpu_tiler_history.is_empty());
        for time in (13000..=201000).step_by(2000) {
            cache.gpu(sample("a", true), None, time);
        }
        let retained = cache.snapshot(201000);
        assert!(retained.gpu_renderer_history.len() <= 96);
        assert!(retained
            .gpu_renderer_history
            .iter()
            .all(|point| point.sampled_at_ms >= 121000));
        cache.gpu(sample("b", false), None, 203000);
        let switched = cache.snapshot(203000);
        assert_eq!(switched.gpu_detail_adapter_id.as_deref(), Some("b"));
        assert_eq!(switched.gpu_detail_history.len(), 1);
        assert!(switched.gpu_renderer_history.is_empty());
        cache.reset(MetricId::Gpu);
        assert!(cache.snapshot(203000).gpu_detail_adapter_id.is_none());
    }

    #[test]
    fn snapshots_share_immutable_rankings_and_keep_the_wire_shape() {
        let mut cache = ResourceCache::default();
        cache.cpu_processes(MetricReading::ready(
            ProcessCpuSummary {
                usage_scale:
                    mangodisk_platform::system_resources::process_cpu::CpuUsageScale::TotalCapacity,
                applications: vec![],
                readable_process_count: 12,
                omitted_process_count: 1,
            },
            1000,
        ));
        let first = cache.snapshot(1000);
        let second = cache.snapshot(2000);
        assert!(Arc::ptr_eq(
            first.cpu_processes.value.as_ref().unwrap(),
            second.cpu_processes.value.as_ref().unwrap()
        ));
        let wire = serde_json::to_value(&second).unwrap();
        assert_eq!(wire["schemaVersion"], 37);
        assert_eq!(wire["cpuProcesses"]["value"]["readableProcessCount"], 12);
        assert!(!cache.expire(2000));
        assert!(cache.expire(6001));
        assert!(!cache.expire(6002));
    }

    #[test]
    fn memory_overview_updates_preserve_detail_timestamp_and_empty_samples_clear_rows() {
        use super::super::models::{MemoryOverview, ProcessMemorySummary};
        let mut cache = ResourceCache::default();
        let sample = |at, processes| SystemResourceSnapshot {
            schema_version: 4,
            sampled_at_ms: at,
            memory: MemoryOverview {
                total_bytes: 100,
                used_bytes: 40,
                free_bytes: 60,
                swap_used_bytes: 0,
                pressure: mangodisk_platform::system_resources::memory::MemoryPressure::Unsupported,
                used_percent: 40,
            },
            processes,
        };
        let details = ProcessMemorySummary {
            usage_kind: mangodisk_platform::system_resources::memory::ProcessMemoryKind::native(),
            applications: vec![],
            readable_process_count: 2,
            omitted_process_count: 0,
        };
        cache.memory(sample(1000, Some(details)));
        cache.memory(sample(12000, None));
        let reading = cache.snapshot(12000);
        assert_eq!(reading.memory.status, MetricStatus::Ready);
        assert_eq!(reading.memory_processes.sampled_at_ms, Some(1000));
        assert_eq!(reading.memory_processes.status, MetricStatus::Stale);
        assert_eq!(
            reading
                .memory_processes
                .value
                .unwrap()
                .readable_process_count,
            2
        );
        cache.memory(sample(
            13000,
            Some(ProcessMemorySummary {
                usage_kind: mangodisk_platform::system_resources::memory::ProcessMemoryKind::native(
                ),
                applications: vec![],
                readable_process_count: 0,
                omitted_process_count: 0,
            }),
        ));
        assert_eq!(
            cache
                .snapshot(13000)
                .memory_processes
                .value
                .unwrap()
                .readable_process_count,
            0
        );
        cache.fail(MetricId::Memory, MetricStatus::Failed);
        assert_eq!(
            cache.snapshot(14000).memory_processes.status,
            MetricStatus::Failed
        );
        cache.suspend(MetricId::Memory);
        assert!(cache.snapshot(14000).memory_processes.value.is_none());
    }

    #[test]
    fn cpu_ranking_survives_baseline_and_failure_with_original_sample_time() {
        let mut cache = ResourceCache::default();
        let summary = ProcessCpuSummary {
            usage_scale:
                mangodisk_platform::system_resources::process_cpu::CpuUsageScale::TotalCapacity,
            applications: vec![],
            readable_process_count: 12,
            omitted_process_count: 1,
        };
        cache.cpu_processes(MetricReading::ready(summary.clone(), 1000));
        cache.cpu_processes(MetricReading::default());
        let warm = cache.snapshot(2000).cpu_processes;
        assert_eq!(warm.status, MetricStatus::Ready);
        assert_eq!(warm.sampled_at_ms, Some(1000));
        assert_eq!(warm.value.unwrap().readable_process_count, 12);
        assert_eq!(
            cache.snapshot(6001).cpu_processes.status,
            MetricStatus::Stale
        );
        cache.cpu_processes(MetricReading {
            status: MetricStatus::Failed,
            ..Default::default()
        });
        let failed = cache.snapshot(7000).cpu_processes;
        assert_eq!(failed.status, MetricStatus::Failed);
        assert_eq!(failed.sampled_at_ms, Some(1000));
        assert!(failed.value.is_some());
        cache.cpu_processes(MetricReading::ready(summary, 8000));
        assert_eq!(
            cache.snapshot(8000).cpu_processes.status,
            MetricStatus::Ready
        );
        cache.clear_cpu_processes();
        assert!(cache.snapshot(8001).cpu_processes.value.is_none());
    }

    #[test]
    fn application_cpu_readings_expire_and_clear_without_erasing_overview_history() {
        let mut cache = ResourceCache::default();
        cache.cpu_processes(MetricReading::ready(
            ProcessCpuSummary {
                usage_scale:
                    mangodisk_platform::system_resources::process_cpu::CpuUsageScale::TotalCapacity,
                applications: vec![],
                readable_process_count: 0,
                omitted_process_count: 2,
            },
            1000,
        ));
        assert_eq!(cache.snapshot(1000).schema_version, 37);
        assert_eq!(
            cache.snapshot(6001).cpu_processes.status,
            MetricStatus::Stale
        );
        cache.clear_cpu_processes();
        assert_eq!(
            cache.snapshot(7000).cpu_processes.status,
            MetricStatus::Loading
        );
        assert!(cache.snapshot(7000).cpu_processes.value.is_none());
    }

    #[test]
    fn native_cpu_baselines_do_not_refresh_old_values() {
        let mut cache = ResourceCache::default();
        let valid = CpuSample::Percent {
            used: 7.0,
            interval_ms: 1000,
        };
        assert_eq!(cache.cpu(valid, 1000, 1000), None);
        assert_eq!(
            cache.cpu(CpuSample::Baseline, 2000, 2000),
            Some(CpuBaselineReason::FirstSample)
        );
        assert_eq!(cache.readings.cpu.sampled_at_ms, Some(1000));
        assert_eq!(cache.readings.cpu.value.as_ref().unwrap().used_percent, 7.0);
        let _ = cache.cpu(CpuSample::Baseline, 7000, 7000);
        assert_eq!(cache.readings.cpu.status, MetricStatus::Stale);
        assert_eq!(cache.readings.cpu.sampled_at_ms, Some(1000));
        assert_eq!(cache.cpu(valid, 8000, 8000), None);
        assert_eq!(cache.readings.cpu.status, MetricStatus::Ready);
    }

    #[test]
    fn network_suspend_preserves_history_but_switching_interfaces_clears_it() {
        use mangodisk_platform::system_resources::network::{InterfaceKind, NetworkCounters};
        let sample = |id: &str, received| {
            vec![InterfaceSample {
                interface: NetworkInterface {
                    details: None,
                    id: id.into(),
                    name: "test".into(),
                    kind: InterfaceKind::Ethernet,
                    connected: true,
                    physical: true,
                    default_route_metric: Some(1),
                },
                counters: Some(NetworkCounters {
                    received,
                    transmitted: 0,
                    ..Default::default()
                }),
            }]
        };
        let mut cache = ResourceCache::default();
        cache.network(sample("one", 0), None, 0, 0);
        cache.network(sample("one", 1000), None, 1000, 1000);
        cache.suspend(MetricId::Network);
        cache.network(sample("one", 2000), None, 2000, 2000);
        assert_eq!(cache.snapshot(2000).network_history.len(), 1);
        assert_eq!(cache.snapshot(2000).network.sampled_at_ms, Some(1000));
        cache.network(sample("two", 5000), None, 3000, 3000);
        assert!(cache.snapshot(3000).network_history.is_empty());
        assert_eq!(cache.snapshot(3000).network.status, MetricStatus::Loading);
        cache.network(sample("two", 6000), None, 4000, 4000);
        assert_eq!(cache.snapshot(4000).network_history.len(), 1);
    }

    #[test]
    fn disk_activity_preserves_real_history_while_rebuilding_a_baseline() {
        use mangodisk_platform::system_resources::disk_io::DeviceCounters;
        let mut cache = ResourceCache::default();
        let sample = |value| {
            vec![DeviceCounters {
                id: "device".into(),
                read_bytes: value,
                written_bytes: value,
            }]
        };
        cache.disk_io(sample(0), 0, 0);
        assert_eq!(cache.snapshot(0).disk_io.status, MetricStatus::Loading);
        cache.disk_io(sample(2000), 2000, 2000);
        let snapshot = cache.snapshot(2000);
        assert_eq!(
            snapshot.disk_io.value.unwrap().read_bytes_per_second,
            1000.0
        );
        assert_eq!(snapshot.disk.status, MetricStatus::Loading);
        assert_eq!(snapshot.disk_io_history.len(), 1);
        cache.fail_disk_io(MetricStatus::Unsupported);
        assert_eq!(
            cache.snapshot(3000).disk_io.status,
            MetricStatus::Unsupported
        );
        cache.disk_io(sample(4000), 4000, 4000);
        assert_eq!(cache.snapshot(4000).disk_io_history.len(), 1);
        cache.disk_io(sample(6000), 6000, 6000);
        assert_eq!(cache.snapshot(11001).disk_io.status, MetricStatus::Stale);
        cache.stop_disk_io();
        cache.disk_io(sample(8000), 8000, 8000);
        assert_eq!(cache.snapshot(12000).disk_io.status, MetricStatus::Stale);
    }

    #[test]
    fn memory_history_contains_occupancy_and_clears_on_demand_reset() {
        let mut cache = ResourceCache::default();
        for time in [0, 3000, 6000] {
            cache.memory(SystemResourceSnapshot {
                schema_version: 4,
                sampled_at_ms: time,
                memory: super::super::models::MemoryOverview {
                    total_bytes: 100,
                    used_bytes: 80,
                    free_bytes: 20,
                    swap_used_bytes: 5,
                    pressure:
                        mangodisk_platform::system_resources::memory::MemoryPressure::Unsupported,
                    used_percent: 80,
                },
                processes: None,
            });
        }
        assert_eq!(cache.snapshot(6000).memory_history.len(), 3);
        assert_eq!(cache.snapshot(6000).memory_history[0].primary, 80.0);
        cache.reset(MetricId::Memory);
        assert!(cache.snapshot(6000).memory_history.is_empty());
    }

    #[test]
    fn cpu_recovery_and_panel_suspend_preserve_real_readings_until_expiry() {
        let mut cache = ResourceCache::default();
        let first = CpuCounters { busy: 10, idle: 90 };
        assert_eq!(cache.cpu(first, 0, 0), Some(CpuBaselineReason::FirstSample));
        let next = CpuCounters {
            busy: 20,
            idle: 180,
        };
        assert_eq!(cache.cpu(next, 2000, 2000), None);
        assert_eq!(
            cache.cpu(next, 4000, 4000),
            Some(CpuBaselineReason::NoProgress)
        );
        let reading = cache.snapshot(4000);
        assert_eq!(reading.cpu.status, MetricStatus::Ready);
        assert_eq!(reading.cpu.sampled_at_ms, Some(2000));
        assert_eq!(reading.cpu_history.len(), 1);
        cache.suspend(MetricId::Cpu);
        assert_eq!(
            cache.cpu(next, 4200, 4200),
            Some(CpuBaselineReason::FirstSample)
        );
        assert_eq!(cache.snapshot(4200).cpu.status, MetricStatus::Ready);
        assert_eq!(cache.snapshot(7001).cpu.status, MetricStatus::Stale);
        assert_eq!(cache.snapshot(7001).cpu_history.len(), 1);
        assert_eq!(
            cache.cpu(
                CpuCounters {
                    busy: 30,
                    idle: 270
                },
                8000,
                8000
            ),
            None
        );
        assert_eq!(cache.snapshot(8000).cpu.status, MetricStatus::Ready);
    }

    #[test]
    fn one_metric_failure_preserves_other_values_and_expires_them_independently() {
        let mut cache = ResourceCache::default();
        let _ = cache.cpu(CpuCounters { busy: 0, idle: 0 }, 0, 0);
        let _ = cache.cpu(CpuCounters { busy: 10, idle: 90 }, 2000, 2000);
        cache.fail(MetricId::Network, MetricStatus::Failed);
        let values = cache.snapshot(2000);
        assert_eq!(values.cpu.status, MetricStatus::Ready);
        assert_eq!(values.network.status, MetricStatus::Failed);
        assert_eq!(values.cpu.value.unwrap().used_percent, 10.0);
        assert_eq!(cache.snapshot(7001).cpu.status, MetricStatus::Stale);
        cache.reset(MetricId::Cpu);
        let resumed = cache.snapshot(8000);
        assert_eq!(resumed.cpu.status, MetricStatus::Loading);
        assert!(resumed.cpu_history.is_empty());
        assert_eq!(resumed.network.status, MetricStatus::Failed);
    }
}
