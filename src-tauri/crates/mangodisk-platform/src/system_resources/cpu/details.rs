//! Optional CPU identity and frequency observations, independent of utilization.
use super::temperature::{CpuTemperature, CpuTemperatureReader, TEMPERATURE_INTERVAL_MS};
use crate::PlatformResult;
#[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
use crate::{PlatformError, PlatformErrorCode};
use serde::Serialize;
use std::time::{Duration, Instant};

#[path = "temperature/diagnostics.rs"]
mod temperature_diagnostics;

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
mod macos;
#[cfg(windows)]
mod windows;

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CpuIdentity {
    pub model: Option<String>,
    pub nominal_frequency_mhz: Option<f64>,
    pub frequency_classes: Vec<CpuFrequencyClass>,
}
/// Processor identity is immutable for the lifetime of this desktop process.
pub fn cpu_identity() -> CpuIdentity {
    static IDENTITY: std::sync::OnceLock<CpuIdentity> = std::sync::OnceLock::new();
    IDENTITY.get_or_init(identity).clone()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CpuFrequencyClass {
    Average,
    Efficiency,
    Performance,
    Super,
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
    pub super_mhz: Option<f64>,
    pub source: CpuFrequencySource,
    pub activity: Vec<CpuFrequencyActivity>,
}

/// Mean residency ratios across the observed channels of one core class.
/// An absent entry means no complete, positive-duration observation exists.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CpuFrequencyActivity {
    pub class: CpuFrequencyClass,
    pub active_ratio: f64,
    pub idle_ratio: f64,
    pub down_ratio: f64,
    pub off_ratio: f64,
}
impl CpuFrequencyActivity {
    pub fn is_idle(&self) -> bool {
        let ratios = [
            self.active_ratio,
            self.idle_ratio,
            self.down_ratio,
            self.off_ratio,
        ];
        ratios
            .iter()
            .all(|v| v.is_finite() && (0.0..=1.0).contains(v))
            && self.active_ratio == 0.0
            && (self.idle_ratio + self.down_ratio + self.off_ratio - 1.0).abs() < 0.000001
    }
}

pub struct CpuDetails {
    pub identity: CpuIdentity,
    // None means no demand; Ok(None) means no completed sampling interval yet.
    pub frequency: Option<PlatformResult<Option<CpuFrequency>>>,
    pub temperature: Option<PlatformResult<CpuTemperature>>,
}

#[derive(Default)]
pub struct CpuDetailsReader {
    identity: Option<CpuIdentity>,
    temperature: Option<CpuTemperatureReader>,
    temperature_due: Option<Instant>,
    temperature_failures: u8,
    temperature_failure_code: Option<crate::PlatformErrorCode>,
    temperature_identified: bool,
    temperature_diagnostics: temperature_diagnostics::Diagnostics,
    #[cfg(windows)]
    frequency: Option<windows::FrequencyReader>,
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    frequency: Option<macos::FrequencyReader>,
}
impl CpuDetailsReader {
    pub fn read(&mut self, detailed: bool, temperature_requested: bool) -> CpuDetails {
        let identity = self.identity.get_or_insert_with(cpu_identity).clone();
        let temperature = self.read_temperature(temperature_requested, identity.model.as_deref());
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
        requested: bool,
        model: Option<&str>,
    ) -> Option<PlatformResult<CpuTemperature>> {
        if !requested {
            self.temperature = None;
            self.temperature_due = None;
            self.temperature_failures = 0;
            self.temperature_failure_code = None;
            self.temperature_identified = false;
            self.temperature_diagnostics = Default::default();
            return None;
        }
        let now = Instant::now();
        if self.temperature_due.is_some_and(|due| now < due) {
            return None;
        }
        let mut stage = "initialize";
        let result = (|| {
            if self.temperature.is_none() {
                self.temperature = Some(CpuTemperatureReader::new(model)?);
            }
            stage = "read";
            self.temperature
                .as_mut()
                .expect("temperature reader initialized")
                .read()
        })();
        Some(self.observe_temperature(result, model, stage, now))
    }

    fn observe_temperature(
        &mut self,
        result: PlatformResult<CpuTemperature>,
        model: Option<&str>,
        stage: &'static str,
        now: Instant,
    ) -> PlatformResult<CpuTemperature> {
        // A sensor previously observed on this CPU becoming absent is a recovery
        // condition, not proof that the processor model lost its capability.
        let result = result.map_err(|error| {
            if self.temperature_identified && error.code() == crate::PlatformErrorCode::Unsupported
            {
                crate::PlatformError::new(
                    crate::PlatformErrorCode::OperationFailed,
                    format!("identified CPU temperature temporarily unavailable: {error}"),
                )
            } else {
                error
            }
        });
        if let Ok(value) = &result {
            self.temperature_identified = true;
            self.temperature_failures = 0;
            self.temperature_failure_code = None;
            self.temperature_diagnostics.observe(value, now);
        } else if let Err(error) = &result {
            if self.temperature_failure_code != Some(error.code()) {
                self.temperature_failures = 0;
            }
            self.temperature_failure_code = Some(error.code());
            self.temperature_failures = self.temperature_failures.saturating_add(1);
        }
        self.temperature_due = temperature_deadline(
            now,
            result.as_ref().map(|_| ()).map_err(|e| e.code()),
            self.temperature_failures,
        );
        if let Err(error) = &result {
            // Invalid numeric samples do not invalidate a healthy native handle.
            if error.code() != crate::PlatformErrorCode::InvalidData {
                self.temperature = None;
            }
            let retry_after_ms = self.temperature_due.map_or(0, |due| {
                due.saturating_duration_since(now).as_millis() as u64
            });
            self.temperature_diagnostics
                .observe_error(model, stage, error, retry_after_ms);
        }
        result
    }
    pub fn resume_temperature(&mut self) {
        self.temperature = None;
        self.temperature_due = None;
        self.temperature_failures = 0;
        self.temperature_failure_code = None;
    }

    pub fn reset(&mut self) {
        // Detail visibility resets frequency baselines, not temperature cadence.
        // Temperature resources are released by read_temperature(false) or Drop.
        #[cfg(any(windows, all(target_os = "macos", target_arch = "aarch64")))]
        {
            if let Some(reader) = &mut self.frequency {
                reader.pause();
            }
        }
    }
}
fn temperature_deadline(
    now: Instant,
    result: Result<(), crate::PlatformErrorCode>,
    failures: u8,
) -> Option<Instant> {
    match result {
        // The privileged service owns sensor failure backoff across all clients.
        // Transport and permission failures still have a local retry budget.
        Ok(()) | Err(crate::PlatformErrorCode::OperationFailed) if cfg!(windows) => None,
        Ok(()) => Some(now + Duration::from_millis(TEMPERATURE_INTERVAL_MS)),
        Err(code) => Some(
            now + Duration::from_millis(crate::system_resources::temperature_retry::delay_ms(
                code,
                failures,
                TEMPERATURE_INTERVAL_MS,
            )),
        ),
    }
}
#[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
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
    #[cfg(target_arch = "aarch64")]
    let frequency_classes = model
        .as_deref()
        .map(macos::frequency_classes)
        .unwrap_or_default();
    #[cfg(not(target_arch = "aarch64"))]
    let frequency_classes = Vec::new();
    CpuIdentity {
        model,
        nominal_frequency_mhz,
        frequency_classes,
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
    fn sampling_repro_transient_temperature_failure_retries_within_one_interval() {
        let now = Instant::now();
        let due = temperature_deadline(now, Err(crate::PlatformErrorCode::OperationFailed), 1);
        assert!(
            due.is_none_or(|due| due <= now + Duration::from_millis(TEMPERATURE_INTERVAL_MS)),
            "one temporary sensor failure must not suppress the next regular sample"
        );
    }

    #[test]
    fn service_cached_observations_do_not_add_a_second_sampling_delay() {
        let now = Instant::now();
        assert_eq!(
            temperature_deadline(now, Ok(()), 0),
            if cfg!(windows) {
                None
            } else {
                Some(now + Duration::from_millis(TEMPERATURE_INTERVAL_MS))
            }
        );
        assert_eq!(
            temperature_deadline(now, Err(crate::PlatformErrorCode::Unsupported), 1),
            Some(now + Duration::from_secs(30))
        );
    }
    #[test]
    fn resetting_detail_baselines_preserves_temperature_cadence_and_backoff() {
        for delay in [TEMPERATURE_INTERVAL_MS, 30_000] {
            let due = Instant::now() + Duration::from_millis(delay);
            let mut reader = CpuDetailsReader {
                temperature_due: Some(due),
                ..Default::default()
            };
            reader.reset();
            assert_eq!(reader.temperature_due, Some(due));
            assert!(reader.read_temperature(true, None).is_none());
            assert!(reader.read_temperature(false, None).is_none());
            assert!(reader.temperature_due.is_none());
        }
    }

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
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    use crate::PlatformErrorCode;
    #[test]
    fn hidden_panels_release_temperature_resources_and_unsupported_probes_back_off() {
        let mut reader = CpuDetailsReader::default();
        assert!(reader.read_temperature(false, None).is_none());
        assert!(reader.temperature_due.is_none());
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
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

#[cfg(all(test, target_os = "macos"))]
mod recovery_tests {
    use super::*;
    use crate::PlatformError;
    #[test]
    #[ignore = "requires an identified native CPU temperature sensor"]
    fn native_cpu_temperature_recovers_without_losing_its_sensor_identity() {
        let mut reader = CpuDetailsReader::default();
        let initial = reader.read(false, true).temperature.unwrap().unwrap();
        assert!(reader.temperature.is_some());
        let now = Instant::now();
        let invalid = PlatformError::new(
            crate::PlatformErrorCode::InvalidData,
            "test gated sensor interval",
        );
        assert!(reader
            .observe_temperature(Err(invalid), None, "read", now)
            .is_err());
        assert!(
            reader.temperature.is_some(),
            "numeric gating must retain the native handle"
        );
        assert_eq!(
            reader.temperature_due,
            Some(now + Duration::from_millis(TEMPERATURE_INTERVAL_MS))
        );
        reader.temperature_due = Some(now - Duration::from_secs(1));
        let recovered = reader.read(false, true).temperature.unwrap().unwrap();
        assert_eq!(initial.source, recovered.source);
        let absent = PlatformError::new(
            crate::PlatformErrorCode::Unsupported,
            "test identified sensor temporarily absent",
        );
        let error = reader
            .observe_temperature(Err(absent), None, "read", now)
            .unwrap_err();
        assert_eq!(error.code(), crate::PlatformErrorCode::OperationFailed);
        assert!(reader.temperature.is_none());
        assert_eq!(
            reader.temperature_due,
            Some(now + Duration::from_millis(TEMPERATURE_INTERVAL_MS))
        );
        reader.resume_temperature();
        let reopened = reader.read(false, true).temperature.unwrap().unwrap();
        assert!(reader.temperature_identified);
        assert_eq!(initial.source, reopened.source);
        println!("native_cpu_temperature_recovery initial_celsius={} recovered_celsius={} reopened_celsius={} retry_ms={TEMPERATURE_INTERVAL_MS}", initial.celsius, recovered.celsius, reopened.celsius);
    }
}
