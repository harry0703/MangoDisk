//! Slow physical-device sensors never hold the independent capacity worker.
use super::{
    disk_health_sampling::HealthSampler, disk_temperature_sampling::TemperatureSampler,
    sampling_schedule::Demand,
};
use mangodisk_platform::{
    system_resources::{disk_health::DiskHealth, disk_temperature::DiskTemperature},
    PlatformResult,
};

type HealthReading = (u64, PlatformResult<DiskHealth>);
type TemperatureReading = (u64, PlatformResult<DiskTemperature>);

pub struct Observation {
    pub health: Option<HealthReading>,
    pub temperature: Option<TemperatureReading>,
    pub detail_temperature: Option<TemperatureReading>,
}

#[derive(Default)]
pub struct Reader {
    health: HealthSampler,
    temperature: TemperatureSampler,
    temperature_state: Option<String>,
    detail_temperature_state: Option<String>,
}

impl Reader {
    pub fn read(&mut self, demand: &Demand, timestamp_ms: u64) -> PlatformResult<Observation> {
        let Self {
            health,
            temperature: temperature_sampler,
            temperature_state,
            detail_temperature_state,
        } = self;
        let volumes = demand.volumes.as_deref().unwrap_or(&[]);
        let selected =
            mangodisk_core::system_resources::disk::select(volumes, demand.selection.as_deref());
        let health_reading = selected
            .filter(|_| demand.disk_health || demand.detailed)
            .map(|volume| health.observation(volume, timestamp_ms, demand.detailed));
        if selected.is_none() {
            health.clear();
        }
        let temperature_volume = mangodisk_core::system_resources::disk::temperature_volume(
            volumes,
            demand.selection.as_deref(),
            demand.temperature_device.as_deref(),
        );
        if temperature_volume.is_none() && selected.is_none() {
            temperature_sampler.clear();
        }
        let temperature = if demand.temperature {
            temperature_volume.map(|volume| {
                temperature_sampler.read(
                    volume,
                    timestamp_ms,
                    health_reading.as_ref(),
                    temperature_state,
                )
            })
        } else {
            None
        };
        let detail_temperature = if demand.detailed && demand.temperature {
            selected.map(|volume| {
                temperature_sampler.read(
                    volume,
                    timestamp_ms,
                    health_reading.as_ref(),
                    detail_temperature_state,
                )
            })
        } else {
            None
        };
        Ok(Observation {
            health: health_reading,
            temperature,
            detail_temperature,
        })
    }
}
