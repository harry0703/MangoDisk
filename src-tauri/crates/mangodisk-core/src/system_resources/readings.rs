//! Independent cached results and interval calculations; no clocks, threads, or desktop APIs.
use mangodisk_platform::system_resources::{
    cpu::{
        details::{CpuDetails, CpuFrequency, CpuIdentity},
        temperature::{CpuTemperature, TEMPERATURE_FRESHNESS_MS},
        CpuSample,
    },
    disk::{ResourceVolume, VolumeCapacity},
    network::{InterfaceSample, NetworkInterface},
};
use serde::Serialize;
use std::sync::Arc;

use super::{
    cpu::{CpuBaselineReason, CpuDelta},
    disk::{self, DiskUsage},
    disk_io::{DiskIoDelta, DiskIoRate},
    gpu::{self, GpuUsage},
    metrics::{CpuUsage, MetricId, MetricReading, MetricStatus, Trend, TrendPoint},
    models::{ProcessCpuSummary, SystemResourceSnapshot},
    network::{self, NetworkDelta, NetworkRate, NetworkSelectionReason},
};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceReadings {
    pub schema_version: u32,
    pub observed_at_ms: u64,
    pub cpu: MetricReading<CpuUsage>,
    pub cpu_identity: Option<CpuIdentity>,
    pub cpu_frequency: MetricReading<CpuFrequency>,
    pub cpu_temperature: MetricReading<CpuTemperature>,
    pub cpu_temperature_history: Vec<TrendPoint>,
    pub gpu: MetricReading<GpuUsage>,
    pub gpu_details: MetricReading<GpuUsage>,
    pub gpu_detail_history: Vec<TrendPoint>,
    pub gpu_detail_adapter_id: Option<String>,
    pub gpu_renderer_history: Vec<TrendPoint>,
    pub gpu_tiler_history: Vec<TrendPoint>,
    pub cpu_processes: MetricReading<Arc<ProcessCpuSummary>>,
    pub memory_processes: MetricReading<Arc<super::models::ProcessMemorySummary>>,
    pub memory: MetricReading<SystemResourceSnapshot>,
    pub network: MetricReading<NetworkRate>,
    pub disk: MetricReading<DiskUsage>,
    pub disk_io: MetricReading<DiskIoRate>,
    pub interfaces: Vec<NetworkInterface>,
    pub volumes: Vec<ResourceVolume>,
    pub gpu_adapters: Vec<mangodisk_platform::system_resources::gpu::GpuAdapter>,
    pub cpu_history: Vec<TrendPoint>,
    pub gpu_history: Vec<TrendPoint>,
    pub network_history: Vec<TrendPoint>,
    pub memory_history: Vec<TrendPoint>,
    pub disk_io_history: Vec<TrendPoint>,
}

impl Default for ResourceReadings {
    fn default() -> Self {
        Self {
            schema_version: 16,
            observed_at_ms: 0,
            cpu: MetricReading::default(),
            cpu_identity: None,
            cpu_frequency: MetricReading::default(),
            cpu_temperature: MetricReading::default(),
            cpu_temperature_history: Vec::new(),
            gpu: MetricReading::default(),
            gpu_details: MetricReading::default(),
            gpu_detail_history: Vec::new(),
            gpu_detail_adapter_id: None,
            gpu_renderer_history: Vec::new(),
            gpu_tiler_history: Vec::new(),
            cpu_processes: MetricReading::default(),
            memory_processes: MetricReading::default(),
            memory: MetricReading::default(),
            network: MetricReading::default(),
            disk: MetricReading::default(),
            disk_io: MetricReading::default(),
            interfaces: Vec::new(),
            volumes: Vec::new(),
            gpu_adapters: Vec::new(),
            cpu_history: Vec::new(),
            gpu_history: Vec::new(),
            network_history: Vec::new(),
            memory_history: Vec::new(),
            disk_io_history: Vec::new(),
        }
    }
}

#[derive(Default)]
pub struct ResourceCache {
    readings: ResourceReadings,
    cpu_delta: CpuDelta,
    network_delta: NetworkDelta,
    disk_io_delta: DiskIoDelta,
    cpu_history: Trend,
    cpu_temperature_history: Trend,
    gpu_history: Trend,
    gpu_detail_history: Trend,
    gpu_detail_device: Option<String>,
    gpu_renderer_history: Trend,
    gpu_tiler_history: Trend,
    network_history: Trend,
    memory_history: Trend,
    disk_io_history: Trend,
    selected_interface: Option<String>,
}

impl ResourceCache {
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
                    && value.sensor_count > 0 =>
            {
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
                self.cpu_temperature_history.push(TrendPoint {
                    sampled_at_ms: timestamp_ms,
                    primary: value.celsius,
                    secondary: None,
                });
                self.readings.cpu_temperature = MetricReading::ready(value, timestamp_ms);
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
                if let Some(mut value) = gpu::select(adapters, selected) {
                    if self.gpu_detail_device.as_deref() != Some(&value.adapter_id) {
                        self.gpu_detail_history.clear();
                        self.gpu_renderer_history.clear();
                        self.gpu_tiler_history.clear();
                        self.readings.gpu_details = MetricReading::default();
                        self.gpu_detail_device = Some(value.adapter_id.clone());
                    }
                    // The panel summary uses every real sample for this device,
                    // including lightweight background observations while details pause.
                    self.gpu_detail_history.push(TrendPoint {
                        sampled_at_ms: timestamp_ms,
                        primary: value.used_percent,
                        secondary: None,
                    });
                    if let Some(details) = &value.details {
                        use mangodisk_platform::system_resources::gpu::details::GpuActivityKind;
                        for (kind, history) in [
                            (GpuActivityKind::Renderer, &mut self.gpu_renderer_history),
                            (GpuActivityKind::Tiler, &mut self.gpu_tiler_history),
                        ] {
                            let maximum = details
                                .activities
                                .iter()
                                .filter(|activity| activity.kind == kind)
                                .map(|activity| activity.used_percent)
                                .filter(|value| value.is_finite() && (0.0..=100.0).contains(value))
                                .max_by(f64::total_cmp);
                            if let Some(primary) = maximum {
                                history.push(TrendPoint {
                                    sampled_at_ms: timestamp_ms,
                                    primary,
                                    secondary: None,
                                });
                            }
                        }
                        self.readings.gpu_details =
                            MetricReading::ready(value.clone(), timestamp_ms);
                    }
                    // Details belong to a selected device, not to the system maximum's history.
                    value.details = None;
                    self.gpu_history.push(TrendPoint {
                        sampled_at_ms: timestamp_ms,
                        primary: value.used_percent,
                        secondary: None,
                    });
                    self.readings.gpu = MetricReading::ready(value, timestamp_ms);
                } else {
                    self.fail(
                        MetricId::Gpu,
                        if selected.is_some_and(|id| {
                            !self
                                .readings
                                .gpu_adapters
                                .iter()
                                .any(|adapter| adapter.id == id)
                        }) {
                            MetricStatus::Disconnected
                        } else {
                            MetricStatus::Failed
                        },
                    );
                }
            }
        }
    }

    pub fn gpu_adapters(
        &mut self,
        adapters: Vec<mangodisk_platform::system_resources::gpu::GpuAdapter>,
    ) {
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
        self.readings.interfaces = interfaces
            .iter()
            .map(|sample| sample.interface.clone())
            .collect();
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
        if let Some(value) =
            self.network_delta
                .sample(&selected.interface, counters, monotonic_ms, reason)
        {
            self.network_history.push(TrendPoint {
                sampled_at_ms: timestamp_ms,
                primary: value.received_bytes_per_second,
                secondary: Some(value.transmitted_bytes_per_second),
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
            self.readings.disk = MetricReading::ready(value, timestamp_ms);
        } else {
            self.fail(MetricId::Disk, MetricStatus::Failed);
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
            }
            MetricId::Disk => self.readings.disk.status = status,
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
                self.gpu_history.clear();
                self.gpu_detail_history.clear();
                self.gpu_renderer_history.clear();
                self.gpu_tiler_history.clear();
                self.gpu_detail_device = None;
                self.readings.gpu_details = MetricReading::default();
                self.readings.gpu = MetricReading::default();
            }
            MetricId::Disk => self.readings.disk = MetricReading::default(),
        }
    }

    /// Update freshness without copying application identities or rebuilding histories.
    pub fn expire(&mut self, now_ms: u64) -> bool {
        let before = self.statuses();
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
        self.readings.disk_io.expire(now_ms, 5000);
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
        before != self.statuses()
    }

    fn statuses(&self) -> [MetricStatus; 11] {
        [
            self.readings.cpu.status,
            self.readings.cpu_frequency.status,
            self.readings.cpu_temperature.status,
            self.readings.gpu.status,
            self.readings.gpu_details.status,
            self.readings.memory.status,
            self.readings.network.status,
            self.readings.disk.status,
            self.readings.disk_io.status,
            self.readings.cpu_processes.status,
            self.readings.memory_processes.status,
        ]
    }

    pub fn snapshot(&mut self, now_ms: u64) -> ResourceReadings {
        self.expire(now_ms);
        self.readings.memory_history = self.memory_history.snapshot(now_ms);
        self.readings.disk_io_history = self.disk_io_history.snapshot(now_ms);
        self.readings.cpu_history = self.cpu_history.snapshot(now_ms);
        self.readings.cpu_temperature_history = self.cpu_temperature_history.snapshot(now_ms);
        self.readings.gpu_history = self.gpu_history.snapshot(now_ms);
        self.readings.gpu_detail_history = self.gpu_detail_history.snapshot(now_ms);
        self.readings
            .gpu_detail_adapter_id
            .clone_from(&self.gpu_detail_device);
        self.readings.gpu_renderer_history = self.gpu_renderer_history.snapshot(now_ms);
        self.readings.gpu_tiler_history = self.gpu_tiler_history.snapshot(now_ms);
        self.readings.network_history = self.network_history.snapshot(now_ms);
        self.readings.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mangodisk_platform::system_resources::cpu::CpuCounters;

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
            },
            frequency,
            temperature: None,
        };
        cache.cpu_details(
            detail(Some(Ok(Some(CpuFrequency {
                average_mhz: Some(4773.0),
                efficiency_mhz: None,
                performance_mhz: None,
                source: CpuFrequencySource::WindowsPerformance,
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
        assert_eq!(switched.gpu_detail_history.len(), 1);
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
        assert_eq!(wire["schemaVersion"], 16);
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
        assert_eq!(cache.snapshot(1000).schema_version, 16);
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
