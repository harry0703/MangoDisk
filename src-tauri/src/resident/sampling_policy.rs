//! Desktop task identities and demand projections; native caches keep their own cadence.
use super::sampling_schedule::Demand;
use mangodisk_core::system_resources::{metrics::MetricId, readings::ResourceCache};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Task {
    Cpu,
    Gpu,
    Memory,
    Network,
    Disk,
    DiskIo,
    CpuProcesses,
    PowerSensors,
    DiskSensors,
    CpuTemperature,
    Battery,
    CpuFrequency,
    MemoryProcesses,
}
impl Task {
    pub const ALL: [Self; 13] = [
        Self::Cpu,
        Self::Gpu,
        Self::Memory,
        Self::Network,
        Self::Disk,
        Self::DiskIo,
        Self::CpuProcesses,
        Self::PowerSensors,
        Self::DiskSensors,
        Self::CpuTemperature,
        Self::Battery,
        Self::CpuFrequency,
        Self::MemoryProcesses,
    ];
    pub const WORKERS: [Self; 12] = [
        Self::Cpu,
        Self::Gpu,
        Self::Memory,
        Self::Network,
        Self::Disk,
        Self::PowerSensors,
        Self::DiskSensors,
        Self::CpuTemperature,
        Self::Battery,
        Self::CpuFrequency,
        Self::MemoryProcesses,
        Self::DiskIo,
    ];
    pub fn metric(self) -> MetricId {
        match self {
            Self::Cpu
            | Self::CpuProcesses
            | Self::PowerSensors
            | Self::CpuTemperature
            | Self::Battery
            | Self::CpuFrequency => MetricId::Cpu,
            Self::Gpu => MetricId::Gpu,
            Self::Memory | Self::MemoryProcesses => MetricId::Memory,
            Self::Network => MetricId::Network,
            Self::Disk | Self::DiskIo | Self::DiskSensors => MetricId::Disk,
        }
    }
    pub fn is_primary(self) -> bool {
        matches!(
            self,
            Self::Cpu | Self::Gpu | Self::Memory | Self::Network | Self::Disk
        )
    }
    pub fn interval_ms(self, demand: &Demand) -> u64 {
        match self {
            Self::Battery if !demand.battery => 30_000,
            Self::CpuTemperature if !cfg!(windows) => {
                mangodisk_platform::system_resources::cpu::temperature::TEMPERATURE_INTERVAL_MS
            }
            Self::CpuTemperature | Self::Battery | Self::DiskIo => 2_000,
            Self::CpuProcesses if demand.detailed => 2_000,
            Self::CpuProcesses => 4_000,
            _ => self.metric().interval_ms(),
        }
    }
    pub fn demand(self, metrics: &[Demand; 5], cache: &ResourceCache) -> Demand {
        let demand = metrics[self.metric() as usize].clone();
        match self {
            Self::Cpu | Self::Disk => Demand {
                active: demand.active,
                selection: demand.selection,
                ..Default::default()
            },
            Self::CpuTemperature => Demand {
                active: demand.active && demand.temperature,
                temperature: demand.temperature,
                ..Default::default()
            },
            Self::Battery => Demand {
                active: demand.active && (demand.battery || demand.prepare_battery),
                battery: demand.battery,
                prepare_battery: demand.prepare_battery,
                ..Default::default()
            },
            Self::DiskIo => Demand {
                active: demand.disk_io,
                ..Default::default()
            },
            Self::Memory => Demand {
                active: demand.active,
                ..Default::default()
            },
            Self::MemoryProcesses => Demand {
                active: demand.active && demand.detailed,
                detailed: demand.detailed,
                ..Default::default()
            },
            Self::CpuFrequency => Demand {
                active: demand.active && demand.frequency,
                frequency: demand.frequency,
                ..Default::default()
            },
            Self::PowerSensors => Demand {
                active: demand.active
                    && (demand.system_power || demand.component_power || demand.fan_speed),
                system_power: demand.system_power,
                component_power: demand.component_power,
                fan_speed: demand.fan_speed,
                ..Default::default()
            },
            Self::DiskSensors => {
                let mut demand = disk_sensor_demand(demand, cache.disk_volume());
                let volumes = cache.volume_catalogue();
                demand.active &= !volumes.is_empty();
                demand.volumes = (!volumes.is_empty()).then_some(volumes);
                demand
            }
            _ => demand,
        }
    }
}
impl From<MetricId> for Task {
    fn from(metric: MetricId) -> Self {
        match metric {
            MetricId::Cpu => Self::Cpu,
            MetricId::Gpu => Self::Gpu,
            MetricId::Memory => Self::Memory,
            MetricId::Network => Self::Network,
            MetricId::Disk => Self::Disk,
        }
    }
}
fn disk_sensor_demand(
    mut demand: Demand,
    confirmed: Option<&mangodisk_platform::system_resources::disk::ResourceVolume>,
) -> Demand {
    let matches = confirmed.is_some_and(|volume| {
        demand
            .selection
            .as_ref()
            .map_or(volume.system, |id| id == &volume.id)
    });
    demand.disk_health &= matches;
    demand.detailed &= matches;
    demand.active &= demand.disk_health || demand.temperature || demand.detailed;
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
        let mut metrics = std::array::from_fn(|_| Demand::default());
        metrics[4] = demand;
        let capacity = Task::Disk.demand(&metrics, &ResourceCache::default());
        assert!(capacity.active);
        assert!(!capacity.disk_health && !capacity.detailed && !capacity.temperature);
    }
    #[test]
    fn projections_keep_source_demands_independent_and_preparation_low_frequency() {
        let mut metrics = std::array::from_fn(|_| Demand::default());
        metrics[0] = Demand {
            active: true,
            temperature: true,
            prepare_battery: true,
            ..Default::default()
        };
        let cache = ResourceCache::default();
        let temperature = Task::CpuTemperature.demand(&metrics, &cache);
        let battery = Task::Battery.demand(&metrics, &cache);
        assert!(temperature.active && temperature.temperature);
        assert!(!temperature.battery && !temperature.prepare_battery && !temperature.frequency);
        assert!(battery.active && battery.prepare_battery);
        assert!(!battery.temperature && !battery.frequency);
        assert_eq!(Task::Battery.interval_ms(&battery), 30000);
        assert!(!Task::PowerSensors.demand(&metrics, &cache).active);
        metrics[0].battery = true;
        assert_eq!(
            Task::Battery.interval_ms(&Task::Battery.demand(&metrics, &cache)),
            2000
        );
        metrics[0].prepare_battery = false;
        metrics[0].battery = false;
        assert!(!Task::Battery.demand(&metrics, &cache).active);
        assert!(Task::CpuTemperature.demand(&metrics, &cache).active);
    }
    #[test]
    fn worker_and_diagnostic_registries_have_unique_complete_task_identities() {
        for (index, task) in Task::ALL.iter().enumerate() {
            assert_eq!(*task as usize, index);
            assert_eq!(Task::ALL.iter().filter(|other| *other == task).count(), 1);
        }
        for metric in MetricId::ALL {
            assert!(Task::WORKERS.contains(&Task::from(metric)));
        }
        assert!(!Task::WORKERS.contains(&Task::CpuProcesses));
        assert!(Task::WORKERS.contains(&Task::DiskIo));
    }
}
