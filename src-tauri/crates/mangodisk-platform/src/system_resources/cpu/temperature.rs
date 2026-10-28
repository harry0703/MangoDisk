//! Identified CPU sensors with explicit measurement kinds; anonymous ACPI zones are excluded.
use crate::{PlatformError, PlatformErrorCode, PlatformResult};
use serde::Serialize;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

pub const TEMPERATURE_INTERVAL_MS: u64 = 4_000;
pub const TEMPERATURE_FRESHNESS_MS: u64 = 10_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CpuTemperatureKind {
    CoreAverage,
    Package,
    CoreMaximum,
    Diode,
    Proximity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CpuTemperatureSource {
    AppleSmc,
    AppleHid,
    LinuxHwmon,
    IntelDtsPawnIo,
    AmdZenPawnIo,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CpuTemperature {
    pub celsius: f64,
    pub kind: CpuTemperatureKind,
    pub source: CpuTemperatureSource,
    pub sensor_count: u32,
    #[serde(skip)]
    pub sampled_at_ms: Option<u64>,
}

pub struct CpuTemperatureReader {
    #[cfg(windows)]
    reader: windows::Reader,
    #[cfg(target_os = "macos")]
    reader: macos::Reader,
    #[cfg(target_os = "linux")]
    reader: linux::Reader,
}

impl CpuTemperatureReader {
    pub fn new(model: Option<&str>) -> PlatformResult<Self> {
        #[cfg(target_os = "macos")]
        return macos::Reader::new(model).map(|reader| Self { reader });
        #[cfg(target_os = "linux")]
        {
            let _ = model;
            Ok(Self {
                reader: linux::Reader::new()?,
            })
        }
        #[cfg(windows)]
        {
            let _ = model;
            windows::Reader::new().map(|reader| Self { reader })
        }
        #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
        {
            let _ = model;
            Err(unsupported())
        }
    }

    pub fn read(&mut self) -> PlatformResult<CpuTemperature> {
        #[cfg(any(windows, target_os = "macos", target_os = "linux"))]
        return self.reader.read();
        #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
        Err(unsupported())
    }
}

pub(super) fn unsupported() -> PlatformError {
    PlatformError::new(
        PlatformErrorCode::Unsupported,
        "verified CPU temperature sensor unavailable",
    )
}

// This only rejects invalid transport values and disabled-sensor sentinels;
// it is not a hardware-specific safe operating range or an alert threshold.
#[cfg(any(test, target_os = "linux"))]
pub(super) fn valid_celsius(value: f64) -> bool {
    value.is_finite() && value > 0.0 && value <= 150.0
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unavailable_and_invalid_values_never_become_zero_degrees() {
        for value in [0.0, -1.0, 151.0, f64::NAN, f64::INFINITY] {
            assert!(!valid_celsius(value));
        }
        assert!(valid_celsius(105.0));
        assert!(valid_celsius(150.0));
    }
    #[cfg(all(windows, not(target_arch = "x86_64")))]
    #[test]
    fn non_x64_hardware_never_uses_anonymous_acpi_temperatures() {
        assert_eq!(
            CpuTemperatureReader::new(None).err().unwrap().code(),
            PlatformErrorCode::Unsupported
        );
    }
}
