//! Identified CPU temperature sensors; SMC transport is shared with GPU sampling.
//! Key identities: https://github.com/exelban/stats/blob/ee4265f3b9afdffebd3273cf6a83b9327ead45b5/Modules/Sensors/values.swift
use super::{unsupported, CpuTemperature, CpuTemperatureKind, CpuTemperatureSource};
use crate::{system_resources::macos_smc::TemperatureSensors, PlatformResult};
#[cfg(target_arch = "x86_64")]
use crate::{PlatformError, PlatformErrorCode};
#[cfg(target_arch = "x86_64")]
use std::mem::size_of;

pub(super) struct Reader {
    smc: PlatformResult<SmcReader>,
    model: Option<String>,
    retry_at: std::time::Instant,
}
impl Reader {
    pub(super) fn new(model: Option<&str>) -> PlatformResult<Self> {
        Ok(Self {
            smc: SmcReader::new(model),
            model: model.map(str::to_owned),
            retry_at: std::time::Instant::now() + std::time::Duration::from_secs(30),
        })
    }
    pub(super) fn read(&mut self) -> PlatformResult<CpuTemperature> {
        if self.smc.is_err() && std::time::Instant::now() >= self.retry_at {
            self.smc = SmcReader::new(self.model.as_deref());
            self.retry_at = std::time::Instant::now() + std::time::Duration::from_secs(30);
        }
        let was_connected = self.smc.is_ok();
        let primary = self
            .smc
            .as_mut()
            .map_err(|error| error.clone())
            .and_then(SmcReader::read);
        if was_connected {
            if let Err(error) = &primary {
                if error.code() != crate::PlatformErrorCode::InvalidData {
                    self.smc = Err(error.clone());
                    self.retry_at = std::time::Instant::now() + std::time::Duration::from_secs(30);
                }
            }
        }
        crate::system_resources::macos_hid::fallback(primary, || {
            let (celsius, sensor_count) = crate::system_resources::macos_hid::read(
                crate::system_resources::macos_hid::Sensor::Cpu,
            )?;
            Ok(CpuTemperature {
                celsius,
                sensor_count,
                sampled_at_ms: None,
                kind: CpuTemperatureKind::CoreAverage,
                source: CpuTemperatureSource::AppleHid,
            })
        })
    }
}
struct SmcReader {
    sensors: TemperatureSensors,
    kind: CpuTemperatureKind,
}
impl SmcReader {
    pub(super) fn new(model: Option<&str>) -> PlatformResult<Self> {
        #[cfg(target_arch = "x86_64")]
        let (keys, _) = cpu_keys(model).ok_or_else(unsupported)?;
        #[cfg(not(target_arch = "x86_64"))]
        let (keys, kind) = cpu_keys(model).ok_or_else(unsupported)?;
        let sensors = TemperatureSensors::new(keys)?;
        #[cfg(target_arch = "x86_64")]
        let (sensors, kind) = {
            let mut sensors = sensors;
            let kind = select_intel_sensors(&mut sensors)?.ok_or_else(unsupported)?;
            (sensors, kind)
        };
        if sensors.len() == 0 {
            return Err(unsupported());
        }
        Ok(Self { sensors, kind })
    }
    pub(super) fn read(&mut self) -> PlatformResult<CpuTemperature> {
        let values = self.sensors.read()?;
        Ok(CpuTemperature {
            celsius: if self.kind == CpuTemperatureKind::CoreMaximum {
                values.iter().copied().fold(f64::NEG_INFINITY, f64::max)
            } else {
                values.iter().sum::<f64>() / values.len() as f64
            },
            sensor_count: values.len() as u32,
            sampled_at_ms: None,
            kind: self.kind,
            source: CpuTemperatureSource::AppleSmc,
        })
    }
}
#[cfg(target_arch = "x86_64")]
fn usable_sensors(sensors: &TemperatureSensors, expected: usize) -> PlatformResult<bool> {
    if sensors.len() != expected {
        return Ok(false);
    }
    // Present keys can contain disabled-sensor sentinels. Native query and
    // response-size errors propagate instead of silently changing the sensor.
    sensors.has_valid_readings()
}
#[cfg(target_arch = "x86_64")]
fn select_intel_sensors(
    sensors: &mut TemperatureSensors,
) -> PlatformResult<Option<CpuTemperatureKind>> {
    if usable_sensors(sensors, 1)? {
        log::info!("cpu_temperature_sensor_selected sensor=TCAD kind=Package");
        return Ok(Some(CpuTemperatureKind::Package));
    }
    if let Some(keys) = intel_core_keys(intel_physical_cores()?) {
        sensors.select(keys)?;
        // Require the complete physical core set to exclude unused core slots.
        if usable_sensors(sensors, keys.len())? {
            log::info!(
                "cpu_temperature_sensor_selected kind=CoreMaximum count={}",
                keys.len()
            );
            return Ok(Some(CpuTemperatureKind::CoreMaximum));
        }
    }
    for (key, kind) in INTEL_FALLBACK_SENSORS {
        sensors.select(&[key])?;
        if usable_sensors(sensors, 1)? {
            log::info!(
                "cpu_temperature_sensor_selected sensor={} kind={kind:?}",
                String::from_utf8_lossy(&key)
            );
            return Ok(Some(kind));
        }
    }
    Ok(None)
}
#[cfg(target_arch = "x86_64")]
const INTEL_FALLBACK_SENSORS: [([u8; 4], CpuTemperatureKind); 4] = [
    (*b"TC0D", CpuTemperatureKind::Diode),
    (*b"TC0E", CpuTemperatureKind::Diode),
    (*b"TC0F", CpuTemperatureKind::Diode),
    (*b"TC0P", CpuTemperatureKind::Proximity),
];
#[cfg(target_arch = "x86_64")]
fn intel_physical_cores() -> PlatformResult<usize> {
    let mut count = 0u32;
    let mut size = size_of::<u32>();
    let code = unsafe {
        libc::sysctlbyname(
            c"hw.physicalcpu_max".as_ptr(),
            (&mut count as *mut u32).cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if code != 0 || size != size_of::<u32>() {
        return Err(PlatformError::new(
            PlatformErrorCode::OperationFailed,
            format!(
                "CPU physical core count read failed: {}",
                std::io::Error::last_os_error()
            ),
        ));
    }
    Ok(count as usize)
}

#[cfg(target_arch = "x86_64")]
fn intel_core_keys(count: usize) -> Option<&'static [[u8; 4]]> {
    // TC1C through TC8C are identified CPU core sensors; do not enumerate other
    // TC-prefixed keys, which include proximity and integrated GPU temperatures.
    const KEYS: [[u8; 4]; 8] = [
        *b"TC1C", *b"TC2C", *b"TC3C", *b"TC4C", *b"TC5C", *b"TC6C", *b"TC7C", *b"TC8C",
    ];
    (1..=KEYS.len()).contains(&count).then(|| &KEYS[..count])
}
fn cpu_keys(model: Option<&str>) -> Option<(&'static [[u8; 4]], CpuTemperatureKind)> {
    #[cfg(target_arch = "x86_64")]
    {
        let _ = model;
        Some((const { &[*b"TCAD"] }, CpuTemperatureKind::Package))
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        // A prefix alone is not an identity: Tf includes both CPU and GPU sensors,
        // and Tp0P is a power-board sensor on Intel. Unknown chips fail closed.
        let model = model?;
        // Stats aab2e1b5: Modules/Sensors/values.swift identifies these A18 Pro cores.
        if model == "Apple A18 Pro" {
            return Some((
                const { &[*b"Te05", *b"Te0S", *b"Tp05", *b"Tp0D"] },
                CpuTemperatureKind::CoreAverage,
            ));
        }
        let generation = model
            .strip_prefix("Apple M")?
            .split_whitespace()
            .next()?
            .parse::<u32>()
            .ok()?;
        let keys: &'static [[u8; 4]] = match generation {
            1 => {
                const {
                    &[
                        *b"Tp09", *b"Tp0T", *b"Tp01", *b"Tp05", *b"Tp0D", *b"Tp0H", *b"Tp0L",
                        *b"Tp0P", *b"Tp0X", *b"Tp0b",
                    ]
                }
            }
            2 => {
                const {
                    &[
                        *b"Tp1h", *b"Tp1t", *b"Tp1p", *b"Tp1l", *b"Tp01", *b"Tp05", *b"Tp09",
                        *b"Tp0D", *b"Tp0X", *b"Tp0b", *b"Tp0f", *b"Tp0j",
                    ]
                }
            }
            3 => {
                const {
                    &[
                        *b"Te05", *b"Te0L", *b"Te0P", *b"Te0S", *b"Tf04", *b"Tf09", *b"Tf0A",
                        *b"Tf0B", *b"Tf0D", *b"Tf0E", *b"Tf44", *b"Tf49", *b"Tf4A", *b"Tf4B",
                        *b"Tf4D", *b"Tf4E",
                    ]
                }
            }
            4 => {
                const {
                    &[
                        *b"Te05", *b"Te0S", *b"Te09", *b"Te0H", *b"Tp01", *b"Tp05", *b"Tp09",
                        *b"Tp0D", *b"Tp0V", *b"Tp0Y", *b"Tp0b", *b"Tp0e",
                    ]
                }
            }
            5 => {
                const {
                    &[
                        *b"Tp00", *b"Tp04", *b"Tp08", *b"Tp0C", *b"Tp0G", *b"Tp0K", *b"Tp0O",
                        *b"Tp0R", *b"Tp0U", *b"Tp0X", *b"Tp0a", *b"Tp0d", *b"Tp0g", *b"Tp0j",
                        *b"Tp0m", *b"Tp0p", *b"Tp0u", *b"Tp0y",
                    ]
                }
            }
            // Stats ee4265f3: Modules/Sensors/values.swift#L519-L538;
            // Kit/plugins/SystemKit.swift limits m6Gen to M6, M6 Pro and M6 Max.
            6 if ["Apple M6", "Apple M6 Pro", "Apple M6 Max"].contains(&model) => {
                const {
                    &[
                        *b"Tp0j", *b"Tp0g", *b"Tp09", *b"Tp07", *b"Tp0m", *b"Tp0L", *b"Tp0I",
                        *b"Tp0d", *b"Tp0E", *b"Tp0G", *b"Tp0b", *b"Tp05", *b"Te07", *b"Te08",
                        *b"Te09", *b"Te0c", *b"Te0h", *b"Te0i",
                    ]
                }
            }
            _ => return None,
        };
        Some((keys, CpuTemperatureKind::CoreAverage))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_arch = "x86_64")]
    #[test]
    #[ignore = "requires an Intel Mac with readable TC0P and PSTR fallback sensors"]
    fn native_intel_fallback_preserves_proximity_identity_and_power_reading() {
        let mut reader = Reader::new(None).unwrap();
        let temperature = reader.read().unwrap();
        assert_eq!(temperature.kind, CpuTemperatureKind::Proximity);
        assert_eq!(temperature.sensor_count, 1);
        let raw = crate::system_resources::macos_smc::ValueReader::new()
            .unwrap()
            .read(*b"TC0P")
            .unwrap()
            .unwrap();
        assert!((raw - temperature.celsius).abs() < 2.0);
        let power = crate::system_resources::power::SystemPowerReader::default()
            .read()
            .unwrap();
        assert!(power.watts > 0.0 && power.watts <= 3_000.0);
    }
    #[cfg(target_arch = "x86_64")]
    #[test]
    fn intel_fallback_keeps_diode_and_proximity_measurements_distinct() {
        assert_eq!(
            INTEL_FALLBACK_SENSORS[0],
            (*b"TC0D", CpuTemperatureKind::Diode)
        );
        assert_eq!(
            INTEL_FALLBACK_SENSORS[3],
            (*b"TC0P", CpuTemperatureKind::Proximity)
        );
        assert!(!INTEL_FALLBACK_SENSORS
            .iter()
            .any(|(key, _)| key == b"TCGC" || key == b"TC0H"));
    }
    #[cfg(target_arch = "x86_64")]
    #[test]
    fn intel_fallback_excludes_unused_slots_and_unidentified_topologies() {
        let keys = intel_core_keys(6).unwrap();
        assert_eq!(keys.len(), 6);
        assert_eq!(keys.last(), Some(b"TC6C"));
        for key in [b"TC7C", b"TC8C", b"TC0P", b"TCGC"] {
            assert!(!keys.contains(key));
        }
        assert!(intel_core_keys(0).is_none());
        assert!(intel_core_keys(9).is_none());
        assert_eq!(cpu_keys(None).unwrap().1, CpuTemperatureKind::Package);
    }
    #[cfg(target_arch = "aarch64")]
    #[test]
    fn model_specific_keys_exclude_gpu_and_pmu_sensors() {
        let (keys, kind) = cpu_keys(Some("Apple M3 Max")).unwrap();
        assert_eq!(keys.len(), 16);
        assert_eq!(kind, CpuTemperatureKind::CoreAverage);
        assert!(!keys.contains(b"Tf14"));
        assert!(!keys.contains(b"Tp0P"));
        assert!(cpu_keys(Some("Apple M7")).is_none());
        assert!(cpu_keys(None).is_none());
        assert!(cpu_keys(Some("VirtualApple")).is_none());
    }

    #[cfg(target_arch = "aarch64")]
    #[test]
    fn a18_pro_cpu_keys_preserve_verified_core_identity() {
        let (keys, kind) = cpu_keys(Some("Apple A18 Pro")).expect("identified A18 Pro CPU sensors");
        assert_eq!(keys, &[*b"Te05", *b"Te0S", *b"Tp05", *b"Tp0D"]);
        assert_eq!(kind, CpuTemperatureKind::CoreAverage);
        for key in [b"Tg05", b"Tg0D", b"Tg0L", b"Tg0e", b"TC0P"] {
            assert!(!keys.contains(key));
        }
        for model in ["Apple A18", "Apple A18 Pro Max", "Apple A19 Pro"] {
            assert!(cpu_keys(Some(model)).is_none(), "unverified model: {model}");
        }
    }

    #[cfg(target_arch = "aarch64")]
    #[test]
    fn m6_cpu_keys_match_identified_upstream_core_sensors() {
        let expected = [
            *b"Tp0j", *b"Tp0g", *b"Tp09", *b"Tp07", *b"Tp0m", *b"Tp0L", *b"Tp0I", *b"Tp0d",
            *b"Tp0E", *b"Tp0G", *b"Tp0b", *b"Tp05", *b"Te07", *b"Te08", *b"Te09", *b"Te0c",
            *b"Te0h", *b"Te0i",
        ];
        for model in ["Apple M6", "Apple M6 Pro", "Apple M6 Max"] {
            let (keys, kind) = cpu_keys(Some(model)).expect("identified M6 CPU sensor map");
            assert_eq!(keys, expected);
            assert_eq!(kind, CpuTemperatureKind::CoreAverage);
        }
        for model in ["Apple M6 Ultra", "Apple M6 Unknown", "Apple M6Pro"] {
            assert!(cpu_keys(Some(model)).is_none(), "unverified model: {model}");
        }
    }
}
