//! Partial CPU observations retain only valid readings from identified sensors.
use crate::{PlatformError, PlatformErrorCode, PlatformResult};
use std::time::{Duration, Instant};

#[derive(Debug)]
pub(super) struct TemperatureSamples {
    pub values: Vec<f64>,
    pub failures: Vec<(String, PlatformError)>,
}

impl TemperatureSamples {
    pub fn collect(
        readings: impl IntoIterator<Item = (String, PlatformResult<Option<f64>>)>,
    ) -> Self {
        let mut samples = Self {
            values: vec![],
            failures: vec![],
        };
        for (sensor, reading) in readings {
            match reading {
                Ok(Some(value)) if value.is_finite() && value > 0.0 && value <= 150.0 => {
                    samples.values.push(value);
                }
                reading => {
                    let error = match reading {
                        Err(error) => error,
                        Ok(value) => PlatformError::new(
                            PlatformErrorCode::InvalidData,
                            format!("temperature invalid value={value:?}"),
                        ),
                    };
                    samples.failures.push((sensor, error));
                }
            }
        }
        samples
    }

    pub fn into_values(self) -> PlatformResult<Vec<f64>> {
        if !self.values.is_empty() {
            return Ok(self.values);
        }
        Err(PlatformError::new(
            self.failures
                .iter()
                // Native failures must reopen the source even when an earlier
                // sensor only returned an invalid numeric value.
                .find(|(_, error)| {
                    !matches!(
                        error.code(),
                        PlatformErrorCode::InvalidData | PlatformErrorCode::Unsupported
                    )
                })
                .or_else(|| self.failures.first())
                .map_or(PlatformErrorCode::Unsupported, |(_, error)| error.code()),
            format!(
                "CPU temperature has no valid sensors total={} failures={}",
                self.failures.len(),
                self.failure_details()
            ),
        ))
    }

    fn failure_details(&self) -> String {
        let details = self
            .failures
            .iter()
            .map(|(sensor, error)| {
                format!(
                    "sensor={} code={:?} error={}",
                    crate::diagnostics::text(sensor),
                    error.code(),
                    crate::diagnostics::text(error)
                )
            })
            .collect::<Vec<_>>()
            .join("; ");
        crate::diagnostics::bounded_message(&details, 4096)
    }
}

#[derive(Default)]
pub(super) struct Diagnostics {
    previous: Option<(usize, Vec<(String, PlatformErrorCode)>)>,
    reported_at: Option<Instant>,
}

impl Diagnostics {
    fn should_report(&mut self, samples: &TemperatureSamples, now: Instant) -> bool {
        let mut failures: Vec<_> = samples
            .failures
            .iter()
            .map(|(sensor, error)| (sensor.clone(), error.code()))
            .collect();
        // Enumeration order is not a change in availability. The enum's local
        // discriminant is only a tie-breaker for duplicate sensor names.
        failures.sort_by(|a, b| a.0.cmp(&b.0).then((a.1 as u8).cmp(&(b.1 as u8))));
        let state = (samples.values.len(), failures);
        let changed = self.previous.as_ref() != Some(&state);
        if !changed
            && self
                .reported_at
                .is_some_and(|at| now.saturating_duration_since(at) < Duration::from_secs(60))
        {
            return false;
        }
        self.previous = Some(state);
        self.reported_at = Some(now);
        true
    }

    pub fn observe(&mut self, source: &str, model: Option<&str>, samples: &TemperatureSamples) {
        if !self.should_report(samples, Instant::now()) {
            return;
        }
        let valid = samples.values.len();
        let invalid = samples.failures.len();
        let total = valid + invalid;
        let outcome = if valid == 0 {
            "unavailable"
        } else if invalid == 0 {
            "ready"
        } else {
            "partial"
        };
        let details = crate::diagnostics::text(&samples.failure_details());
        let model = crate::diagnostics::text(model.unwrap_or("unknown"));
        if invalid == 0 {
            log::info!("cpu_temperature_sensor_state source={source} model={model} total={total} valid={valid} invalid={invalid} outcome={outcome}");
        } else {
            log::warn!("cpu_temperature_sensor_state source={source} model={model} total={total} valid={valid} invalid={invalid} outcome={outcome} failures={details}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn numeric_failure_cannot_mask_transport_failure_when_every_sensor_is_unavailable() {
        let samples = TemperatureSamples::collect([
            ("Tp0Y".into(), Ok(Some(0.0))),
            (
                "Te05".into(),
                Err(PlatformError::new(
                    PlatformErrorCode::OperationFailed,
                    "SMC native_code=0xe00002c0",
                )),
            ),
        ]);
        let error = super::super::macos_hid::fallback(samples.into_values(), || {
            Err(PlatformError::new(
                PlatformErrorCode::Unsupported,
                "no HID CPU sensors",
            ))
        })
        .unwrap_err();
        assert_eq!(
            error.code(),
            PlatformErrorCode::OperationFailed,
            "native connection failures must trigger reopening rather than numeric-only retry"
        );
    }
    #[test]
    fn all_failed_sensors_preserve_permission_errors_in_either_enumeration_order() {
        for denied_first in [true, false] {
            let mut readings = vec![
                ("Tp0Y".into(), Ok(Some(0.0))),
                (
                    "Te05".into(),
                    Err(PlatformError::new(
                        PlatformErrorCode::AccessDenied,
                        "SMC permission revoked",
                    )),
                ),
            ];
            if denied_first {
                readings.reverse();
            }
            assert_eq!(
                TemperatureSamples::collect(readings)
                    .into_values()
                    .unwrap_err()
                    .code(),
                PlatformErrorCode::AccessDenied
            );
        }
    }
    #[test]
    fn duplicate_hid_names_with_different_errors_have_an_order_independent_state() {
        let mut diagnostics = Diagnostics::default();
        let now = Instant::now();
        let mut readings = vec![
            ("pACC MTR Temp Sensor".into(), Ok(Some(0.0))),
            (
                "pACC MTR Temp Sensor".into(),
                Err(PlatformError::operation_failed("HID event absent")),
            ),
        ];
        assert!(diagnostics.should_report(&TemperatureSamples::collect(readings.clone()), now));
        readings.reverse();
        assert!(!diagnostics.should_report(
            &TemperatureSamples::collect(readings),
            now + Duration::from_secs(4)
        ));
    }
    #[test]
    fn diagnostic_state_does_not_change_when_the_same_failed_sensors_are_reordered() {
        let mut diagnostics = Diagnostics::default();
        let now = Instant::now();
        let samples = TemperatureSamples::collect([
            ("Tp0Y".into(), Ok(Some(0.0))),
            ("Tp0D".into(), Ok(Some(-1.0))),
            ("Te05".into(), Ok(Some(50.0))),
        ]);
        assert!(diagnostics.should_report(&samples, now));
        let reordered = TemperatureSamples::collect([
            ("Tp0D".into(), Ok(Some(-1.0))),
            ("Tp0Y".into(), Ok(Some(0.0))),
            ("Te05".into(), Ok(Some(51.0))),
        ]);
        assert!(
            !diagnostics.should_report(&reordered, now + Duration::from_secs(4)),
            "enumeration order is not a sensor state transition"
        );
    }
    fn batch(values: &[f64]) -> TemperatureSamples {
        TemperatureSamples::collect(
            values
                .iter()
                .enumerate()
                .map(|(i, value)| (format!("sensor{i}"), Ok(Some(*value)))),
        )
    }
    #[test]
    fn one_invalid_sensor_keeps_the_other_eight_cpu_readings() {
        let mut values = [50.0; 9];
        values[5] = 0.0;
        let samples = batch(&values);
        assert_eq!(samples.values.len(), 8);
        assert_eq!(samples.failures.len(), 1);
        assert!(samples.failure_details().contains("value=Some(0.0)"));
        assert_eq!(samples.into_values().unwrap(), vec![50.0; 8]);
    }
    #[test]
    fn one_valid_sensor_is_enough_and_no_invalid_number_becomes_a_reading() {
        let samples = batch(&[0.0, -1.0, f64::NAN, f64::INFINITY, 151.0, 53.0]);
        assert_eq!(samples.failures.len(), 5);
        assert_eq!(samples.into_values().unwrap(), [53.0]);
        assert!(batch(&[0.0, f64::NAN]).into_values().is_err());
        assert!(batch(&[]).into_values().is_err());
    }
    #[test]
    fn missing_values_and_native_errors_do_not_hide_other_sensors() {
        let samples = TemperatureSamples::collect([
            (
                "Tp0Y".into(),
                Err(PlatformError::new(
                    PlatformErrorCode::Io,
                    "native_code=0x1234",
                )),
            ),
            ("Tp0D".into(), Ok(None)),
            ("Te05".into(), Ok(Some(48.0))),
        ]);
        assert_eq!(samples.values, [48.0]);
        let details = samples.failure_details();
        assert!(
            details.contains("Tp0Y")
                && details.contains("native_code=0x1234")
                && details.contains("Tp0D")
        );
        let all_failed = TemperatureSamples::collect([(
            "Tp0Y".into(),
            Err(PlatformError::new(
                PlatformErrorCode::AccessDenied,
                "access revoked",
            )),
        )])
        .into_values()
        .unwrap_err();
        assert_eq!(all_failed.code(), PlatformErrorCode::AccessDenied);
    }
    #[test]
    fn diagnostics_report_transitions_recovery_and_bounded_periodic_summaries() {
        let mut diagnostics = Diagnostics::default();
        let now = Instant::now();
        assert!(diagnostics.should_report(&batch(&[50.0, 0.0]), now));
        assert!(!diagnostics.should_report(&batch(&[51.0, -1.0]), now + Duration::from_secs(4)));
        assert!(diagnostics.should_report(&batch(&[51.0, -1.0]), now + Duration::from_secs(60)));
        assert!(diagnostics.should_report(&batch(&[51.0, 52.0]), now + Duration::from_secs(64)));
        assert!(diagnostics.should_report(&batch(&[0.0, 0.0]), now + Duration::from_secs(68)));
    }
    #[test]
    fn oversized_native_errors_remain_bounded_and_record_boundaries_are_escaped() {
        let samples = TemperatureSamples::collect([(
            "Tp0Y".into(),
            Err(PlatformError::new(
                PlatformErrorCode::Io,
                format!("native_error\n{}", "x".repeat(10_000)),
            )),
        )]);
        let details = samples.failure_details();
        assert!(details.len() < 4200);
        assert!(details.contains("Tp0Y") && details.contains("native_error\\n"));
        assert!(!details.contains('\n'));
        assert!(details.ends_with("…[truncated]"));
    }
}
