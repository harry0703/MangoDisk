//! Optional CPU identity and frequency observations, independent of utilization.
use super::temperature::{CpuTemperature, CpuTemperatureReader, TEMPERATURE_INTERVAL_MS};
use crate::{PlatformError, PlatformErrorCode, PlatformResult};
use serde::Serialize;
use std::time::{Duration, Instant};

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
mod macos;
#[cfg(windows)]
mod windows;

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CpuIdentity {
    pub model: Option<String>,
    pub nominal_frequency_mhz: Option<f64>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CpuFrequencySource {
    WindowsPerformance,
    ApplePerformanceStates,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CpuFrequency {
    pub average_mhz: Option<f64>,
    pub efficiency_mhz: Option<f64>,
    pub performance_mhz: Option<f64>,
    pub source: CpuFrequencySource,
}

pub struct CpuDetails {
    pub identity: CpuIdentity,
    // None means no demand, Ok(None) means a baseline or no active observation.
    pub frequency: Option<PlatformResult<Option<CpuFrequency>>>,
    pub temperature: Option<PlatformResult<CpuTemperature>>,
}

#[derive(Default)]
pub struct CpuDetailsReader {
    identity: Option<CpuIdentity>,
    temperature: Option<CpuTemperatureReader>,
    temperature_due: Option<Instant>,
    #[cfg(windows)]
    frequency: Option<windows::FrequencyReader>,
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    frequency: Option<macos::FrequencyReader>,
}
impl CpuDetailsReader {
    pub fn read(&mut self, detailed: bool, temperature_visible: bool) -> CpuDetails {
        let identity = self.identity.get_or_insert_with(identity).clone();
        let temperature = self.read_temperature(temperature_visible, identity.model.as_deref());
        #[cfg(any(windows, all(target_os = "macos", target_arch = "aarch64")))]
        let frequency = if detailed {
            let reader = self.frequency.get_or_insert_with(Default::default);
            Some(reader.read())
        } else {
            if let Some(reader) = &mut self.frequency {
                reader.pause();
            }
            None
        };
        #[cfg(not(any(windows, all(target_os = "macos", target_arch = "aarch64"))))]
        let frequency = detailed.then(|| Err(unsupported()));
        CpuDetails {
            identity,
            frequency,
            temperature,
        }
    }
    fn read_temperature(
        &mut self,
        visible: bool,
        model: Option<&str>,
    ) -> Option<PlatformResult<CpuTemperature>> {
        if !visible {
            self.temperature = None;
            self.temperature_due = None;
            return None;
        }
        let now = Instant::now();
        if self.temperature_due.is_some_and(|due| now < due) {
            return None;
        }
        let result = (|| {
            if self.temperature.is_none() {
                self.temperature = Some(CpuTemperatureReader::new(model)?);
            }
            self.temperature
                .as_mut()
                .expect("temperature reader initialized")
                .read()
        })();
        let delay = if result.is_ok() {
            TEMPERATURE_INTERVAL_MS
        } else {
            30_000
        };
        self.temperature_due = Some(now + Duration::from_millis(delay));
        if let Err(error) = &result {
            self.temperature = None;
            if error.code() != PlatformErrorCode::Unsupported {
                log::warn!(
                    "cpu_temperature_read_failed code={:?} error={}",
                    error.code(),
                    crate::diagnostics::text(error)
                );
            }
        }
        Some(result)
    }
    pub fn reset(&mut self) {
        self.temperature = None;
        self.temperature_due = None;
        #[cfg(any(windows, all(target_os = "macos", target_arch = "aarch64")))]
        {
            if let Some(reader) = &mut self.frequency {
                reader.pause();
            }
        }
    }
}
fn unsupported() -> PlatformError {
    PlatformError::new(
        PlatformErrorCode::Unsupported,
        "CPU frequency source unavailable",
    )
}
#[cfg(any(test, windows, target_os = "macos"))]
fn valid_mhz(value: f64) -> Option<f64> {
    (value.is_finite() && (1.0..=20_000.0).contains(&value)).then_some(value)
}
#[cfg(windows)]
fn identity() -> CpuIdentity {
    windows::identity()
}
#[cfg(target_os = "macos")]
fn identity() -> CpuIdentity {
    fn sysctl_bytes(name: &std::ffi::CStr) -> Option<Vec<u8>> {
        let mut size = 0;
        if unsafe {
            libc::sysctlbyname(
                name.as_ptr(),
                std::ptr::null_mut(),
                &mut size,
                std::ptr::null_mut(),
                0,
            )
        } != 0
            || size == 0
            || size > 1024
        {
            return None;
        }
        let mut bytes = vec![0; size];
        if unsafe {
            libc::sysctlbyname(
                name.as_ptr(),
                bytes.as_mut_ptr().cast(),
                &mut size,
                std::ptr::null_mut(),
                0,
            )
        } != 0
        {
            return None;
        }
        bytes.truncate(size);
        Some(bytes)
    }
    let model = sysctl_bytes(c"machdep.cpu.brand_string")
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .map(|text| text.trim_end_matches('\0').trim().to_owned())
        .filter(|text| !text.is_empty());
    // Apple Silicon's maximum DVFS state is not an advertised base frequency.
    #[cfg(target_arch = "aarch64")]
    let nominal_frequency_mhz = None;
    #[cfg(not(target_arch = "aarch64"))]
    let nominal_frequency_mhz = sysctl_bytes(c"hw.cpufrequency")
        .and_then(|bytes| <[u8; 8]>::try_from(bytes).ok())
        .and_then(|bytes| valid_mhz(u64::from_ne_bytes(bytes) as f64 / 1_000_000.0));
    CpuIdentity {
        model,
        nominal_frequency_mhz,
    }
}
#[cfg(not(any(windows, target_os = "macos")))]
fn identity() -> CpuIdentity {
    CpuIdentity::default()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn frequency_rejects_zero_nonfinite_and_implausible_values() {
        for value in [0.0, -1.0, f64::NAN, f64::INFINITY, 20_001.0] {
            assert_eq!(valid_mhz(value), None);
        }
        assert_eq!(valid_mhz(4774.29), Some(4774.29));
    }
}

#[cfg(test)]
mod temperature_tests {
    use super::*;
    #[test]
    fn hidden_panels_release_temperature_resources_and_unsupported_probes_back_off() {
        let mut reader = CpuDetailsReader::default();
        assert!(reader.read_temperature(false, None).is_none());
        assert!(reader.temperature_due.is_none());
        #[cfg(any(windows, all(target_os = "macos", target_arch = "aarch64")))]
        {
            let missing = reader
                .read_temperature(true, Some("VirtualApple"))
                .unwrap()
                .unwrap_err();
            assert_eq!(missing.code(), PlatformErrorCode::Unsupported);
            assert!(reader
                .read_temperature(true, Some("VirtualApple"))
                .is_none());
            assert!(reader.temperature.is_none());
        }
        reader.temperature_due = Some(Instant::now() + Duration::from_secs(30));
        assert!(reader.read_temperature(false, None).is_none());
        assert!(reader.temperature_due.is_none());
        reader.reset();
        assert!(reader.temperature.is_none());
    }
}
