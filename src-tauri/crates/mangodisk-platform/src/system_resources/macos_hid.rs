//! Read only named CPU/GPU HID temperature sensors; PMU and proximity are excluded.
use crate::{PlatformError, PlatformErrorCode, PlatformResult};
use core_foundation::{
    array::CFArray,
    base::{CFType, CFTypeRef, TCFType},
    dictionary::CFDictionary,
    number::CFNumber,
    string::{CFString, CFStringRef},
};
use std::ptr;

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    fn IOHIDEventSystemClientCreate(allocator: *const std::ffi::c_void) -> CFTypeRef;
    fn IOHIDEventSystemClientSetMatching(client: CFTypeRef, matching: CFTypeRef);
    fn IOHIDEventSystemClientCopyServices(client: CFTypeRef) -> CFTypeRef;
    fn IOHIDServiceClientCopyProperty(service: CFTypeRef, key: CFStringRef) -> CFTypeRef;
    fn IOHIDServiceClientCopyEvent(
        service: CFTypeRef,
        kind: i64,
        options: i32,
        timeout: i64,
    ) -> CFTypeRef;
    fn IOHIDEventGetFloatValue(event: CFTypeRef, field: i64) -> f64;
}

#[derive(Clone, Copy, Debug)]
pub(super) enum Sensor {
    Cpu,
    Gpu,
}

fn identified(name: &str, sensor: Sensor) -> bool {
    match sensor {
        Sensor::Cpu => {
            name.starts_with("pACC MTR Temp Sensor") || name.starts_with("eACC MTR Temp Sensor")
        }
        Sensor::Gpu => name.starts_with("GPU MTR Temp Sensor"),
    }
}

fn owned(raw: CFTypeRef) -> Option<CFType> {
    (!raw.is_null()).then(|| unsafe { CFType::wrap_under_create_rule(raw) })
}

pub(super) fn read(sensor: Sensor) -> PlatformResult<(f64, u32)> {
    let samples = read_samples(sensor)?;
    if !samples.failures.is_empty() {
        return Err(PlatformError::new(
            PlatformErrorCode::Unsupported,
            format!("source=iohid sensor={sensor:?} reason=no_identified_temperature"),
        ));
    }
    average(&samples.values).ok_or_else(|| {
        PlatformError::new(
            PlatformErrorCode::Unsupported,
            format!("source=iohid sensor={sensor:?} reason=no_identified_temperature"),
        )
    })
}

pub(super) fn read_samples(
    sensor: Sensor,
) -> PlatformResult<super::temperature_samples::TemperatureSamples> {
    let unavailable = || {
        PlatformError::new(
            PlatformErrorCode::Unsupported,
            format!("source=iohid sensor={sensor:?} reason=no_identified_temperature"),
        )
    };
    let client =
        owned(unsafe { IOHIDEventSystemClientCreate(ptr::null()) }).ok_or_else(unavailable)?;
    let matching = CFDictionary::from_CFType_pairs(&[
        (CFString::new("PrimaryUsagePage"), CFNumber::from(0xff00i32)),
        (CFString::new("PrimaryUsage"), CFNumber::from(5i32)),
    ]);
    unsafe {
        IOHIDEventSystemClientSetMatching(client.as_CFTypeRef(), matching.as_CFTypeRef());
    }
    let services = owned(unsafe { IOHIDEventSystemClientCopyServices(client.as_CFTypeRef()) })
        .ok_or_else(unavailable)?;
    let services = services.downcast::<CFArray>().ok_or_else(unavailable)?;
    let key = CFString::new("Product");
    let mut values = Vec::new();
    for index in 0..services.len() {
        let service = unsafe {
            core_foundation::array::CFArrayGetValueAtIndex(services.as_concrete_TypeRef(), index)
        };
        let Some(name) =
            owned(unsafe { IOHIDServiceClientCopyProperty(service, key.as_concrete_TypeRef()) })
        else {
            continue;
        };
        let Some(name) = name.downcast::<CFString>() else {
            continue;
        };
        if !identified(&name.to_string(), sensor) {
            continue;
        }
        let value = owned(unsafe { IOHIDServiceClientCopyEvent(service, 15, 0, 0) })
            .ok_or_else(|| {
                PlatformError::new(
                    PlatformErrorCode::OperationFailed,
                    "HID temperature event unavailable",
                )
            })
            .and_then(|event| {
                let value = unsafe { IOHIDEventGetFloatValue(event.as_CFTypeRef(), 15 << 16) };
                reported_temperature(value)
            });
        values.push((name.to_string(), value));
    }
    Ok(super::temperature_samples::TemperatureSamples::collect(
        values,
    ))
}

fn reported_temperature(value: f64) -> PlatformResult<Option<f64>> {
    if value == 128.0 {
        Err(PlatformError::new(
            PlatformErrorCode::InvalidData,
            "HID temperature sentinel value=128",
        ))
    } else {
        Ok(Some(value))
    }
}

fn average(values: &[f64]) -> Option<(f64, u32)> {
    (!values.is_empty()
        && values
            .iter()
            .all(|value| value.is_finite() && *value > 0.0 && *value <= 150.0 && *value != 128.0))
    .then(|| {
        (
            values.iter().sum::<f64>() / values.len() as f64,
            values.len() as u32,
        )
    })
}

pub(super) fn fallback<T>(
    primary: PlatformResult<T>,
    read: impl FnOnce() -> PlatformResult<T>,
) -> PlatformResult<T> {
    primary.or_else(|primary| {
        read().map_err(|secondary| {
            PlatformError::new(
                if primary.code() == PlatformErrorCode::Unsupported {
                    secondary.code()
                } else {
                    primary.code()
                },
                format!(
                    "temperature primary={} fallback={}",
                    crate::diagnostics::text(&primary),
                    crate::diagnostics::text(&secondary)
                ),
            )
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hid_cpu_partial_group_keeps_a_real_reading_after_sentinels_and_missing_events() {
        let samples = super::super::temperature_samples::TemperatureSamples::collect([
            ("pACC MTR Temp Sensor0".into(), reported_temperature(128.0)),
            ("pACC MTR Temp Sensor1".into(), reported_temperature(0.0)),
            (
                "eACC MTR Temp Sensor0".into(),
                Err(PlatformError::new(
                    PlatformErrorCode::OperationFailed,
                    "HID temperature event unavailable",
                )),
            ),
            ("eACC MTR Temp Sensor1".into(), reported_temperature(57.0)),
        ]);
        assert_eq!(samples.failures.len(), 3);
        assert_eq!(samples.into_values().unwrap(), [57.0]);
        assert!(
            average(&[128.0, 0.0, 57.0]).is_none(),
            "GPU complete-group validation remains strict"
        );
    }
    #[test]
    fn only_identified_sensors_and_complete_valid_groups_are_accepted() {
        assert!(identified("pACC MTR Temp Sensor0", Sensor::Cpu));
        assert!(identified("eACC MTR Temp Sensor1", Sensor::Cpu));
        assert!(identified("GPU MTR Temp Sensor0", Sensor::Gpu));
        for name in [
            "PMU tdie1",
            "PMU tdev1",
            "gas gauge battery",
            "NAND CH0 temp",
            "GPU MTR Temp Sensor0",
        ] {
            assert!(!identified(name, Sensor::Cpu));
        }
        assert!(!identified("pACC MTR Temp Sensor0", Sensor::Gpu));
        assert_eq!(average(&[40.0, 60.0]), Some((50.0, 2)));
        for value in [0.0, -1.0, 128.0, 151.0, f64::NAN, f64::INFINITY] {
            assert!(average(&[40.0, value]).is_none());
        }
        assert!(average(&[]).is_none());
    }
    #[test]
    fn smc_success_skips_hid_and_each_failed_observation_can_fall_back() {
        assert_eq!(
            fallback(Ok(40), || panic!("SMC remains preferred")).unwrap(),
            40
        );
        assert_eq!(
            fallback(
                Err(PlatformError::new(PlatformErrorCode::Io, "SMC failure")),
                || Ok(50)
            )
            .unwrap(),
            50
        );
        assert_eq!(
            fallback(Ok(41), || panic!("SMC recovers independently")).unwrap(),
            41
        );
        assert!(fallback::<u32>(
            Err(PlatformError::new(PlatformErrorCode::Io, "SMC failure")),
            || Err(PlatformError::new(
                PlatformErrorCode::Unsupported,
                "no HID sensor"
            ))
        )
        .is_err());
    }
}
