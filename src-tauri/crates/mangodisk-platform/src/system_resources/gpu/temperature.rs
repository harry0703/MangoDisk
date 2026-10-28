//! Identified GPU temperatures, independent of utilization availability.
use crate::PlatformResult;
use serde::Serialize;

pub const GPU_TEMPERATURE_INTERVAL_MS: u64 = 4_000;
pub const GPU_TEMPERATURE_FRESHNESS_MS: u64 = 10_000;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum GpuTemperatureKind {
    SensorAverage,
    Device,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum GpuTemperatureSource {
    AppleSmc,
    AppleHid,
    Driver,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GpuTemperature {
    pub adapter_id: String,
    pub adapter_name: String,
    pub celsius: f64,
    pub kind: GpuTemperatureKind,
    pub source: GpuTemperatureSource,
    pub sensor_count: u32,
}
impl GpuTemperature {
    pub fn valid(&self) -> bool {
        !self.adapter_id.is_empty()
            && self.celsius.is_finite()
            && self.celsius > 0.0
            && self.celsius <= 150.0
            && self.celsius != 128.0
            && self.sensor_count > 0
    }
}
#[derive(Debug)]
pub struct GpuTemperatureSample {
    pub adapter_id: String,
    pub reading: PlatformResult<GpuTemperature>,
}
