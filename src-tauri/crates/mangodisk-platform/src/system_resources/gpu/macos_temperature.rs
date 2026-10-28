//! Known SMC identities; arbitrary temperature prefixes are never GPU identities.
//! Mappings: https://github.com/exelban/stats/blob/ee4265f3b9afdffebd3273cf6a83b9327ead45b5/Modules/Sensors/values.swift
use super::temperature::GpuTemperatureKind;
use crate::{
    system_resources::macos_smc::TemperatureSensors, PlatformError, PlatformErrorCode,
    PlatformResult,
};

pub(super) struct Reader {
    smc: PlatformResult<SmcReader>,
    hid_allowed: bool,
    model: String,
    class: String,
    unambiguous: bool,
    retry_at: std::time::Instant,
}
impl Reader {
    pub(super) fn new(model: &str, class: &str, unambiguous: bool) -> PlatformResult<Self> {
        Ok(Self {
            smc: SmcReader::new(model, class, unambiguous),
            hid_allowed: unambiguous && class.contains("agx"),
            model: model.into(),
            class: class.into(),
            unambiguous,
            retry_at: std::time::Instant::now() + std::time::Duration::from_secs(30),
        })
    }
    pub(super) fn read(
        &mut self,
    ) -> PlatformResult<(
        f64,
        u32,
        GpuTemperatureKind,
        super::temperature::GpuTemperatureSource,
    )> {
        if self.smc.is_err() && std::time::Instant::now() >= self.retry_at {
            self.smc = SmcReader::new(&self.model, &self.class, self.unambiguous);
            self.retry_at = std::time::Instant::now() + std::time::Duration::from_secs(30);
        }
        let was_connected = self.smc.is_ok();
        let primary = self
            .smc
            .as_ref()
            .map_err(Clone::clone)
            .and_then(SmcReader::read)
            .map(|(celsius, count, kind)| {
                (
                    celsius,
                    count,
                    kind,
                    super::temperature::GpuTemperatureSource::AppleSmc,
                )
            });
        if was_connected {
            if let Err(error) = &primary {
                if error.code() != crate::PlatformErrorCode::InvalidData {
                    self.smc = Err(error.clone());
                    self.retry_at = std::time::Instant::now() + std::time::Duration::from_secs(30);
                }
            }
        }
        if !self.hid_allowed {
            return primary;
        }
        crate::system_resources::macos_hid::fallback(primary, || {
            let (celsius, count) = crate::system_resources::macos_hid::read(
                crate::system_resources::macos_hid::Sensor::Gpu,
            )?;
            Ok((
                celsius,
                count,
                GpuTemperatureKind::SensorAverage,
                super::temperature::GpuTemperatureSource::AppleHid,
            ))
        })
    }
}
struct SmcReader {
    sensors: TemperatureSensors,
    kind: GpuTemperatureKind,
    reject_power_gated_readings: bool,
}
impl SmcReader {
    pub(super) fn new(model: &str, class: &str, unambiguous: bool) -> PlatformResult<Self> {
        let keys = sensor_keys(model, class, unambiguous).ok_or_else(unsupported)?;
        let sensors = TemperatureSensors::new(keys)?;
        if sensors.len() == 0 {
            return Err(unsupported());
        }
        let kind = if class.contains("agx") {
            GpuTemperatureKind::SensorAverage
        } else {
            GpuTemperatureKind::Device
        };
        Ok(Self {
            sensors,
            kind,
            reject_power_gated_readings: model == "Apple A18 Pro",
        })
    }
    pub(super) fn read(&self) -> PlatformResult<(f64, u32, GpuTemperatureKind)> {
        let values = self.sensors.read()?;
        Ok((
            sensor_average(&values, self.reject_power_gated_readings)?,
            values.len() as u32,
            self.kind,
        ))
    }
}
fn sensor_average(values: &[f64], reject_power_gated_readings: bool) -> PlatformResult<f64> {
    if values.is_empty() || values.contains(&128.0) {
        return Err(PlatformError::new(
            PlatformErrorCode::InvalidData,
            "GPU SMC temperature sentinel or empty sensor group",
        ));
    }
    // Stats #3625 reports approximately 0.2 C on A18 Pro while its GPU is gated.
    // Reject the whole interval rather than dropping a sensor and changing the mean.
    if reject_power_gated_readings && values.iter().any(|value| *value <= 1.0) {
        return Err(PlatformError::new(
            PlatformErrorCode::InvalidData,
            "GPU SMC temperature stage=validate model=A18Pro reason=near_zero_power_gated_reading",
        ));
    }
    Ok(values.iter().sum::<f64>() / values.len() as f64)
}
fn unsupported() -> PlatformError {
    PlatformError::new(
        PlatformErrorCode::Unsupported,
        "Identified GPU temperature sensors unavailable",
    )
}
fn sensor_keys(model: &str, class: &str, unambiguous: bool) -> Option<&'static [[u8; 4]]> {
    if !unambiguous {
        return None;
    }
    if class.contains("intel") {
        return Some(const { &[*b"TCGC"] });
    }
    // TGDD cannot identify which AMD accelerator owns a shared SMC sensor.
    // Keep driver readings on AMD until an internal physical-device mapping exists.
    if !class.contains("agx") {
        return None;
    }
    // Stats aab2e1b5: Modules/Sensors/values.swift identifies these A18 Pro GPU sensors.
    if model == "Apple A18 Pro" {
        return Some(const { &[*b"Tg05", *b"Tg0D", *b"Tg0L", *b"Tg0e"] });
    }
    let chip = model
        .strip_prefix("Apple M")?
        .split_whitespace()
        .next()?
        .parse::<u32>()
        .ok()?;
    let suffix = model.strip_prefix(&format!("Apple M{chip}"))?;
    if !["", " Pro", " Max", " Ultra"].contains(&suffix) {
        return None;
    }
    match chip {
        1 => Some(const { &[*b"Tg05", *b"Tg0D", *b"Tg0L", *b"Tg0T"] }),
        2 => Some(const { &[*b"Tg0f", *b"Tg0j"] }),
        3 => Some(
            const {
                &[
                    *b"Tf14", *b"Tf18", *b"Tf19", *b"Tf1A", *b"Tf24", *b"Tf28", *b"Tf29", *b"Tf2A",
                ]
            },
        ),
        4 if model == "Apple M4" => Some(
            const {
                &[
                    *b"Tg0G", *b"Tg0H", *b"Tg0K", *b"Tg0L", *b"Tg0d", *b"Tg0e", *b"Tg0j", *b"Tg0k",
                ]
            },
        ),
        4 if ["Apple M4 Pro", "Apple M4 Max", "Apple M4 Ultra"].contains(&model) => Some(
            const {
                &[
                    *b"Tg1U", *b"Tg1k", *b"Tg0K", *b"Tg0L", *b"Tg0d", *b"Tg0e", *b"Tg0j", *b"Tg0k",
                ]
            },
        ),
        5 => Some(
            const {
                &[
                    *b"Tg0U", *b"Tg0X", *b"Tg0d", *b"Tg0g", *b"Tg0j", *b"Tg1Y", *b"Tg1c", *b"Tg1g",
                ]
            },
        ),
        // Stats ee4265f3: Modules/Sensors/values.swift#L540-L551;
        // Kit/plugins/SystemKit.swift limits m6Gen to M6, M6 Pro and M6 Max.
        6 if ["Apple M6", "Apple M6 Pro", "Apple M6 Max"].contains(&model) => Some(
            const {
                &[
                    *b"Tg1e", *b"Tg0d", *b"Tg0c", *b"Tg0b", *b"Tg1c", *b"Tg1d", *b"Tg07", *b"Tg1f",
                    *b"Tg09", *b"Tg0a", *b"Tg1g", *b"Tg1h",
                ]
            },
        ),
        _ => None,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a18_pro_power_gated_sensors_do_not_become_near_zero_temperatures() {
        // Stats #3625 observes approximately 0.2 C while the GPU is power-gated.
        for values in [&[0.2; 4][..], &[42.9, 0.2, 43.6, 43.3], &[0.199_999; 4]] {
            assert_eq!(
                sensor_average(values, true).unwrap_err().code(),
                PlatformErrorCode::InvalidData
            );
        }
        assert!((sensor_average(&[42.9, 43.8, 43.6, 43.3], true).unwrap() - 43.4).abs() < 0.000001);
        assert_eq!(sensor_average(&[0.2], false).unwrap(), 0.2);
        assert!(sensor_average(&[128.0], false).is_err());
        assert!(sensor_average(&[], true).is_err());
    }
    #[test]
    fn identities_exclude_cpu_proximity_and_ambiguous_devices() {
        let keys = sensor_keys("Apple M3 Max", "agx", true).unwrap();
        assert_eq!(keys.len(), 8);
        assert!(!keys.contains(b"Tf04"));
        assert!(!keys.contains(b"TG0P"));
        assert!(sensor_keys("Apple M7", "agx", true).is_none());
        assert!(sensor_keys("Apple M3 Unknown", "agx", true).is_none());
        assert!(sensor_keys("VirtualApple", "agx", true).is_none());
        assert!(sensor_keys("Apple M3 Max", "agx", false).is_none());
        assert!(sensor_keys("Intel", "amd", true).is_none());
        assert_eq!(sensor_keys("Intel", "intel", true), Some(&[*b"TCGC"][..]));
    }
    #[test]
    fn m4_variants_keep_distinct_sensor_maps() {
        assert!(sensor_keys("Apple M4", "agx", true)
            .unwrap()
            .contains(b"Tg0G"));
        assert!(!sensor_keys("Apple M4 Pro", "agx", true)
            .unwrap()
            .contains(b"Tg0G"));
        assert!(sensor_keys("Apple M4 Pro", "agx", true)
            .unwrap()
            .contains(b"Tg1U"));
        assert!(sensor_keys("Apple M4 Unknown", "agx", true).is_none());
    }

    #[test]
    fn a18_pro_gpu_keys_preserve_verified_adapter_identity() {
        let keys =
            sensor_keys("Apple A18 Pro", "agx", true).expect("identified A18 Pro GPU sensors");
        assert_eq!(keys, &[*b"Tg05", *b"Tg0D", *b"Tg0L", *b"Tg0e"]);
        for key in [b"Te05", b"Te0S", b"Tp05", b"Tp0D", b"TG0P"] {
            assert!(!keys.contains(key));
        }
        assert!(sensor_keys("Apple A18 Pro", "agx", false).is_none());
        assert!(sensor_keys("Apple A18 Pro", "amd", true).is_none());
        for model in ["Apple A18", "Apple A18 Pro Max", "Apple A19 Pro"] {
            assert!(sensor_keys(model, "agx", true).is_none());
        }
    }

    #[test]
    fn m6_gpu_keys_match_identified_upstream_sensors_without_ambiguous_fallbacks() {
        let expected = [
            *b"Tg1e", *b"Tg0d", *b"Tg0c", *b"Tg0b", *b"Tg1c", *b"Tg1d", *b"Tg07", *b"Tg1f",
            *b"Tg09", *b"Tg0a", *b"Tg1g", *b"Tg1h",
        ];
        for model in ["Apple M6", "Apple M6 Pro", "Apple M6 Max"] {
            assert_eq!(sensor_keys(model, "agx", true), Some(expected.as_slice()));
            assert!(sensor_keys(model, "agx", false).is_none());
            assert!(sensor_keys(model, "amd", true).is_none());
        }
        for model in ["Apple M6 Ultra", "Apple M6 Unknown", "Apple M6Pro"] {
            assert!(sensor_keys(model, "agx", true).is_none());
        }
    }
}
