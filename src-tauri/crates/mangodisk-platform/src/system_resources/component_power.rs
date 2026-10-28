//! Component energy estimates are independent of whole-system SMC power.
use crate::PlatformResult;
#[cfg(not(all(target_os = "macos", target_arch = "x86_64")))]
use crate::{PlatformError, PlatformErrorCode};
use serde::Serialize;
use std::time::{Duration, Instant};

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
mod macos;
#[cfg(all(target_os = "macos", any(test, target_arch = "x86_64")))]
pub(crate) mod macos_smc;

pub const COMPONENT_POWER_FRESHNESS_MS: u64 = 5_000;
pub const COMPONENT_POWER_MAX_WATTS: f64 = 1_000.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Component {
    Cpu,
    Gpu,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ComponentPowerSource {
    IoReport,
    AppleSmc,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComponentPower {
    pub watts: f64,
    pub source: ComponentPowerSource,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GpuPower {
    pub adapter_id: String,
    #[serde(flatten)]
    pub power: ComponentPower,
}

pub struct ComponentPowerReader {
    component: Component,
    retry_at: Option<Instant>,
    diagnostic: Option<String>,
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    reader: Option<macos::Reader>,
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    reader: Option<macos_smc::Reader>,
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    smc_sensor: macos_smc::Sensor,
}
impl ComponentPowerReader {
    pub fn new(component: Component) -> Self {
        Self {
            component,
            retry_at: None,
            diagnostic: None,
            #[cfg(target_os = "macos")]
            reader: None,
            #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
            smc_sensor: match component {
                Component::Cpu => macos_smc::Sensor::CpuPackage,
                Component::Gpu => macos_smc::Sensor::DiscreteGpu,
            },
        }
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    pub(crate) fn integrated_gpu() -> Self {
        Self {
            smc_sensor: macos_smc::Sensor::IntegratedGpu,
            ..Self::new(Component::Gpu)
        }
    }
    pub fn read(&mut self) -> Option<PlatformResult<ComponentPower>> {
        let now = Instant::now();
        if self.retry_at.is_some_and(|at| now < at) {
            return None;
        }
        let result = self.sample();
        let diagnostic = match &result {
            Ok(Some(power)) => Some(format!("ready source={:?}", power.source)),
            Ok(None) => None,
            Err(error) => Some(format!(
                "{:?}:{}",
                error.code(),
                crate::diagnostics::text(error)
            )),
        };
        if let Some(diagnostic) = diagnostic {
            if self.diagnostic.as_ref() != Some(&diagnostic) {
                log::info!(
                    "component_power_observation component={:?} outcome={diagnostic}",
                    self.component
                );
                self.diagnostic = Some(diagnostic);
            }
        }
        if result.is_err() {
            self.retry_at = Some(now + Duration::from_secs(30));
            #[cfg(target_os = "macos")]
            {
                self.reader = None;
            }
        }
        result.transpose()
    }
    fn sample(&mut self) -> PlatformResult<Option<ComponentPower>> {
        #[cfg(target_os = "macos")]
        {
            if self.reader.is_none() {
                #[cfg(target_arch = "aarch64")]
                {
                    self.reader = Some(macos::Reader::new(self.component)?);
                }
                #[cfg(target_arch = "x86_64")]
                {
                    self.reader = Some(macos_smc::Reader::new(self.smc_sensor)?);
                }
            }
            self.reader
                .as_mut()
                .expect("power reader initialized")
                .read()
        }
        #[cfg(not(target_os = "macos"))]
        Err(unavailable("platform"))
    }
}
#[cfg(not(all(target_os = "macos", target_arch = "x86_64")))]
fn unavailable(stage: &str) -> PlatformError {
    PlatformError::new(
        PlatformErrorCode::Unsupported,
        format!(
            "source=component_power stage={stage} reason=identified_component_energy_unavailable"
        ),
    )
}

#[cfg(any(test, all(target_os = "macos", target_arch = "aarch64")))]
fn channel_matches(component: Component, name: &str) -> bool {
    match component {
        Component::Cpu => {
            name == "CPU Energy"
                || name
                    .strip_prefix("DIE_")
                    .and_then(|s| s.strip_suffix("_CPU Energy"))
                    .is_some_and(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
        }
        Component::Gpu => name == "GPU Energy",
    }
}
#[cfg(any(test, all(target_os = "macos", target_arch = "aarch64")))]
fn energy_watts(energy: i64, unit: &str, seconds: f64) -> Option<f64> {
    let divisor = match unit {
        "mJ" => 1e3,
        "uJ" => 1e6,
        "nJ" => 1e9,
        _ => return None,
    };
    if energy < 0 || !seconds.is_finite() || seconds <= 0.0 {
        return None;
    }
    let watts = energy as f64 / divisor / seconds;
    (watts.is_finite() && (0.0..=COMPONENT_POWER_MAX_WATTS).contains(&watts)).then_some(watts)
}

#[cfg(any(test, all(target_os = "macos", target_arch = "aarch64")))]
fn sample_seconds(monotonic: Duration, wall: Option<Duration>) -> Option<f64> {
    let wall = wall?;
    ((100..=5_000).contains(&monotonic.as_millis())
        && (100..=5_000).contains(&wall.as_millis())
        && monotonic.abs_diff(wall) <= Duration::from_millis(250))
    .then_some(monotonic.as_secs_f64())
}

impl Default for ComponentPowerReader {
    fn default() -> Self {
        Self::new(Component::Cpu)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pauses_sleep_and_wall_clock_changes_require_a_new_baseline() {
        assert_eq!(
            sample_seconds(Duration::from_secs(2), Some(Duration::from_secs(2))),
            Some(2.0)
        );
        for (monotonic, wall) in [
            (0, Some(0)),
            (2, Some(60)),
            (60, Some(60)),
            (2, None),
            (2, Some(4)),
        ] {
            assert!(sample_seconds(
                Duration::from_secs(monotonic),
                wall.map(Duration::from_secs)
            )
            .is_none());
        }
    }
    #[test]
    fn energy_units_and_component_identities_do_not_mix_domains() {
        for (energy, unit) in [(20_000, "mJ"), (20_000_000, "uJ"), (20_000_000_000, "nJ")] {
            assert_eq!(energy_watts(energy, unit, 2.0), Some(10.0));
        }
        assert_eq!(energy_watts(0, "mJ", 2.0), Some(0.0));
        for (energy, unit, seconds) in [
            (-1, "mJ", 1.0),
            (100, "W", 1.0),
            (100, "mJ", 0.0),
            (i64::MAX, "mJ", 1.0),
        ] {
            assert!(energy_watts(energy, unit, seconds).is_none());
        }
        assert!(channel_matches(Component::Cpu, "DIE_0_CPU Energy"));
        assert!(channel_matches(Component::Cpu, "DIE_1_CPU Energy"));
        for name in ["GPU Energy", "ANE0", "DRAM Energy", "Unknown CPU Energy"] {
            assert!(!channel_matches(Component::Cpu, name));
        }
        assert!(channel_matches(Component::Gpu, "GPU Energy"));
        assert!(!channel_matches(Component::Gpu, "GPU SRAM0"));
    }
}
