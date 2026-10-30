//! Per-device temperature history remains independent of utilization failures.
use super::metrics::{
    MetricReading, MetricStatus, Trend, TrendHistory, TrendPoint, TEMPERATURE_RETENTION_MS,
};
use mangodisk_platform::{
    system_resources::gpu::temperature::{
        GpuTemperature, GpuTemperatureSample, GPU_TEMPERATURE_FRESHNESS_MS,
    },
    PlatformError, PlatformErrorCode,
};
use std::collections::BTreeMap;

#[derive(Default)]
struct DeviceTemperature {
    reading: MetricReading<GpuTemperature>,
    history: Trend,
    interrupted: bool,
    identity: Option<TemperatureIdentity>,
}
#[derive(PartialEq, Eq)]
struct TemperatureIdentity {
    name: String,
    kind: mangodisk_platform::system_resources::gpu::temperature::GpuTemperatureKind,
    source: mangodisk_platform::system_resources::gpu::temperature::GpuTemperatureSource,
    count: u32,
}
impl DeviceTemperature {
    fn fail(&mut self, status: MetricStatus) {
        self.reading.status = status;
        self.interrupted = true;
        if matches!(
            status,
            MetricStatus::Unsupported | MetricStatus::Disconnected
        ) {
            self.reading.value = None;
            self.reading.sampled_at_ms = None;
            self.history.clear();
            self.identity = None;
        }
    }
    fn fail_error(&mut self, error: &PlatformError) {
        self.fail(error_status(error));
        if matches!(
            error.code(),
            PlatformErrorCode::AccessDenied | PlatformErrorCode::ItemChanged
        ) {
            self.reading.value = None;
            self.reading.sampled_at_ms = None;
            self.history.clear();
            self.identity = None;
        }
    }
    fn apply(&mut self, sample: GpuTemperatureSample, now: u64) {
        match sample.reading {
            Ok(value) if value.valid() && value.adapter_id == sample.adapter_id => {
                let identity = TemperatureIdentity {
                    name: value.adapter_name.clone(),
                    kind: value.kind,
                    source: value.source,
                    count: value.sensor_count,
                };
                if self
                    .identity
                    .as_ref()
                    .is_some_and(|previous| previous != &identity)
                {
                    self.history.clear();
                }
                self.identity = Some(identity);
                self.history.push(TrendPoint {
                    sampled_at_ms: now,
                    primary: value.celsius,
                    secondary: None,
                    gap_before: self.interrupted,
                });
                self.interrupted = false;
                self.reading = MetricReading::ready(value, now);
            }
            Ok(_) => self.fail(MetricStatus::Failed),
            Err(error) => self.fail_error(&error),
        }
    }
}
fn error_status(error: &PlatformError) -> MetricStatus {
    match error.code() {
        PlatformErrorCode::Unsupported => MetricStatus::Unsupported,
        PlatformErrorCode::ItemChanged => MetricStatus::Disconnected,
        _ => MetricStatus::Failed,
    }
}

#[derive(Default)]
pub(super) struct GpuTemperatureCache {
    pub reading: MetricReading<GpuTemperature>,
    devices: BTreeMap<String, DeviceTemperature>,
    adapter_id: Option<String>,
}
impl GpuTemperatureCache {
    fn refresh_reading(&mut self) {
        if let Some(id) = &self.adapter_id {
            self.reading = self
                .devices
                .get(id)
                .map(|device| device.reading.clone())
                .unwrap_or(MetricReading {
                    status: MetricStatus::Disconnected,
                    value: None,
                    sampled_at_ms: None,
                });
        }
    }
    pub fn select(&mut self, id: Option<&str>) {
        if let Some(id) = id {
            if self.adapter_id.as_deref() != Some(id) {
                self.devices.entry(id.into()).or_default();
                self.adapter_id = Some(id.into());
                self.refresh_reading();
            }
        }
    }
    pub fn apply(
        &mut self,
        samples: Vec<GpuTemperatureSample>,
        selected: Option<&str>,
        automatic: Option<&str>,
        now: u64,
    ) {
        let id = selected
            .or(automatic)
            .map(str::to_owned)
            .or_else(|| {
                self.adapter_id
                    .clone()
                    .filter(|id| samples.iter().any(|sample| sample.adapter_id == *id))
            })
            .or_else(|| {
                samples
                    .iter()
                    .filter(|sample| sample.reading.as_ref().is_ok_and(GpuTemperature::valid))
                    .map(|sample| &sample.adapter_id)
                    .min()
                    .cloned()
            })
            .or_else(|| {
                samples
                    .iter()
                    .map(|sample| &sample.adapter_id)
                    .min()
                    .cloned()
            });
        let present = samples
            .iter()
            .any(|sample| Some(&sample.adapter_id) == id.as_ref());
        // Collect every real native observation, including devices not currently displayed.
        for sample in samples {
            self.devices
                .entry(sample.adapter_id.clone())
                .or_default()
                .apply(sample, now);
        }
        self.select(id.as_deref());
        if !present {
            let status = if selected.is_some() {
                MetricStatus::Disconnected
            } else {
                MetricStatus::Unsupported
            };
            if let Some(id) = &self.adapter_id {
                self.devices.entry(id.clone()).or_default().fail(status);
            }
            self.refresh_reading();
            self.reading.status = status;
        } else {
            self.refresh_reading();
        }
    }
    pub fn error(&mut self, error: &PlatformError) {
        let status = error_status(error);
        // A failed batch supplies no trustworthy status for any cached device.
        // Per-device failures still affect only their own entry in apply().
        for device in self.devices.values_mut() {
            device.fail_error(error);
        }
        self.refresh_reading();
        self.reading.status = status;
    }
    pub fn invalidate_replaced_device(&mut self, id: &str, native_name: &str) {
        // Temperature can identify a replacement before utilization has a new baseline.
        // Keep that verified reading rather than introducing an artificial outage.
        if self.devices.get(id).is_some_and(|device| {
            device
                .reading
                .value
                .as_ref()
                .is_some_and(|value| value.adapter_name == native_name)
        }) {
            return;
        }
        self.devices.remove(id);
        self.refresh_reading();
    }
    pub fn clear_selection(&mut self) {
        self.adapter_id = None;
        self.reading = Default::default();
    }
    pub fn retain(&mut self, mut present: impl FnMut(&str) -> bool) {
        self.devices.retain(|id, _| present(id));
        self.refresh_reading();
    }
    pub fn expire(&mut self, now: u64) -> bool {
        let mut changed = false;
        for device in self.devices.values_mut() {
            let had_value = device.reading.value.is_some();
            changed |= device.reading.expire_retained(
                now,
                GPU_TEMPERATURE_FRESHNESS_MS,
                TEMPERATURE_RETENTION_MS,
            );
            if had_value && device.reading.value.is_none() {
                device.interrupted = true;
            }
        }
        self.refresh_reading();
        changed
    }
    pub fn history(&mut self, now: u64) -> TrendHistory {
        self.adapter_id
            .as_ref()
            .and_then(|id| self.devices.get_mut(id))
            .map(|device| device.history.snapshot(now))
            .unwrap_or_default()
    }

    pub fn device_snapshot(
        &mut self,
        id: &str,
        now: u64,
    ) -> (MetricReading<GpuTemperature>, TrendHistory) {
        self.devices
            .get_mut(id)
            .map(|device| (device.reading.clone(), device.history.snapshot(now)))
            .unwrap_or_else(|| {
                (
                    MetricReading {
                        status: if matches!(
                            self.reading.status,
                            MetricStatus::Unsupported | MetricStatus::Failed
                        ) {
                            self.reading.status
                        } else {
                            MetricStatus::Loading
                        },
                        sampled_at_ms: None,
                        value: None,
                    },
                    Default::default(),
                )
            })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use mangodisk_platform::system_resources::gpu::temperature::{
        GpuTemperatureKind, GpuTemperatureSource,
    };
    fn sample(id: &str, celsius: f64) -> GpuTemperatureSample {
        GpuTemperatureSample {
            adapter_id: id.into(),
            reading: Ok(GpuTemperature {
                adapter_id: id.into(),
                adapter_name: id.into(),
                celsius,
                kind: GpuTemperatureKind::SensorAverage,
                source: GpuTemperatureSource::AppleSmc,
                sensor_count: 8,
            }),
        }
    }
    #[test]
    fn temperature_survives_utilization_absence_and_display_pauses_with_real_timestamps() {
        let mut cache = GpuTemperatureCache::default();
        for time in [1000, 5000, 9000, 13000] {
            cache.apply(vec![sample("a", 51.0)], None, None, time);
        }
        assert_eq!(cache.history(13000).len(), 4);
        assert_eq!(cache.reading.sampled_at_ms, Some(13000));
        cache.expire(23001);
        assert_eq!(cache.reading.status, MetricStatus::Stale);
    }
    #[test]
    fn manual_selection_never_falls_back_and_device_changes_do_not_mix_history() {
        let mut cache = GpuTemperatureCache::default();
        cache.apply(
            vec![sample("a", 51.0), sample("b", 80.0)],
            Some("a"),
            Some("b"),
            1000,
        );
        assert_eq!(cache.reading.value.as_ref().unwrap().adapter_id, "a");
        cache.apply(vec![sample("b", 80.0)], Some("a"), Some("b"), 5000);
        assert_eq!(cache.reading.status, MetricStatus::Disconnected);
        assert!(cache.history(5000).is_empty());
        cache.apply(vec![sample("b", 80.0)], Some("b"), None, 9000);
        assert_eq!(cache.history(9000).len(), 3);
        assert!(cache
            .history(9000)
            .iter()
            .all(|point| point.primary == 80.0));
    }
    #[test]
    fn temporary_failures_preserve_history_without_manufacturing_new_samples() {
        let mut cache = GpuTemperatureCache::default();
        cache.apply(vec![sample("a", 51.0)], None, None, 1000);
        cache.error(&PlatformError::new(
            PlatformErrorCode::OperationFailed,
            "read failed",
        ));
        assert_eq!(cache.reading.status, MetricStatus::Failed);
        assert_eq!(cache.reading.sampled_at_ms, Some(1000));
        cache.apply(vec![sample("a", 52.0)], None, None, 9000);
        assert_eq!(cache.history(9000).len(), 2);
    }
    #[test]
    fn changing_sensor_meaning_resets_history_and_unsupported_drops_retained_values() {
        let mut cache = GpuTemperatureCache::default();
        cache.apply(vec![sample("a", 51.0)], Some("a"), None, 1000);
        let mut next = sample("a", 52.0);
        let value = next.reading.as_mut().unwrap();
        value.kind = GpuTemperatureKind::Device;
        value.source = GpuTemperatureSource::Driver;
        value.sensor_count = 1;
        cache.apply(vec![next], Some("a"), None, 5000);
        assert_eq!(cache.history(5000).len(), 1);
        cache.error(&PlatformError::new(
            PlatformErrorCode::Unsupported,
            "no sensor",
        ));
        assert!(cache.history(5000).is_empty());
        assert!(cache.reading.value.is_none());
        assert!(cache.reading.sampled_at_ms.is_none());
    }

    #[test]
    fn invalid_temperatures_and_mismatched_sensor_identity_are_rejected() {
        for value in [0.0, -1.0, f64::NAN, f64::INFINITY, 151.0, 128.0] {
            let mut cache = GpuTemperatureCache::default();
            cache.apply(vec![sample("a", value)], Some("a"), None, 1000);
            assert_eq!(cache.reading.status, MetricStatus::Failed);
            assert!(cache.history(1000).is_empty());
        }
        let mut bad = sample("a", 51.0);
        bad.reading.as_mut().unwrap().adapter_id = "b".into();
        let mut cache = GpuTemperatureCache::default();
        cache.apply(vec![bad], Some("a"), None, 1000);
        assert_eq!(cache.reading.status, MetricStatus::Failed);
    }

    #[test]
    fn switching_preserves_real_outages_and_catalogue_removal_discards_only_that_device() {
        let mut cache = GpuTemperatureCache::default();
        cache.select(Some("a"));
        cache.expire(1000);
        assert_eq!(cache.reading.status, MetricStatus::Loading);
        cache.apply(
            vec![sample("a", 51.0), sample("b", 70.0)],
            None,
            Some("a"),
            1000,
        );
        cache.select(Some("b"));
        cache.error(&PlatformError::new(
            PlatformErrorCode::OperationFailed,
            "temporary query failure",
        ));
        cache.apply(
            vec![sample("a", 52.0), sample("b", 71.0)],
            None,
            Some("b"),
            17000,
        );
        cache.select(Some("a"));
        cache.expire(17000);
        assert_eq!(cache.reading.sampled_at_ms, Some(17000));
        assert_eq!(
            cache
                .history(17000)
                .iter()
                .map(|point| point.sampled_at_ms)
                .collect::<Vec<_>>(),
            vec![1000, 17000]
        );
        cache.expire(27001);
        assert_eq!(cache.reading.status, MetricStatus::Stale);
        cache.retain(|id| id == "b");
        cache.expire(27001);
        assert_eq!(cache.reading.status, MetricStatus::Disconnected);
        assert!(cache.reading.value.is_none());
        assert!(cache.history(27001).is_empty());
        assert_eq!(cache.devices.len(), 1);
        cache.apply(vec![sample("a", 53.0)], Some("a"), None, 29000);
        assert_eq!(cache.history(29000).len(), 1);
    }

    #[test]
    fn a_failed_temperature_batch_cannot_be_bypassed_by_switching_devices() {
        for (code, expected) in [
            (PlatformErrorCode::OperationFailed, MetricStatus::Failed),
            (PlatformErrorCode::Unsupported, MetricStatus::Unsupported),
        ] {
            let mut cache = GpuTemperatureCache::default();
            cache.apply(
                vec![sample("a", 51.0), sample("b", 70.0)],
                None,
                Some("a"),
                1000,
            );
            cache.error(&PlatformError::new(code, "temperature batch unavailable"));
            cache.select(Some("b"));
            cache.expire(5000);
            assert_eq!(cache.reading.status, expected);
            if expected == MetricStatus::Failed {
                assert_eq!(cache.reading.sampled_at_ms, Some(1000));
                assert_eq!(cache.history(5000).len(), 1);
            } else {
                assert!(cache.reading.value.is_none());
                assert!(cache.history(5000).is_empty());
            }
        }
    }

    #[test]
    fn native_replacement_and_per_device_recovery_do_not_restore_other_failed_readings() {
        let mut cache = GpuTemperatureCache::default();
        cache.apply(
            vec![sample("a", 51.0), sample("b", 70.0)],
            None,
            Some("a"),
            1000,
        );
        cache.error(&PlatformError::new(
            PlatformErrorCode::OperationFailed,
            "batch failed",
        ));
        cache.apply(vec![sample("a", 52.0)], None, Some("a"), 5000);
        assert_eq!(cache.reading.status, MetricStatus::Ready);
        cache.select(Some("b"));
        cache.expire(5000);
        assert_eq!(cache.reading.status, MetricStatus::Failed);
        assert_eq!(cache.reading.sampled_at_ms, Some(1000));
        cache.apply(vec![sample("b", 71.0)], None, Some("b"), 17000);
        assert_eq!(
            cache
                .history(17000)
                .iter()
                .map(|point| point.sampled_at_ms)
                .collect::<Vec<_>>(),
            vec![1000, 17000]
        );
        let mut replacement = sample("b", 60.0);
        replacement.reading.as_mut().unwrap().adapter_name = "Replacement".into();
        cache.apply(vec![replacement], None, Some("b"), 21000);
        assert_eq!(cache.history(21000).len(), 1);
        cache.select(Some("a"));
        cache.expire(21000);
        assert_eq!(cache.reading.status, MetricStatus::Stale);
        assert_eq!(cache.history(21000).len(), 2);
    }

    #[test]
    fn unavailable_temperature_without_a_selected_device_keeps_its_status() {
        let mut cache = GpuTemperatureCache::default();
        cache.error(&PlatformError::new(
            PlatformErrorCode::Unsupported,
            "no sensors",
        ));
        cache.expire(1000);
        assert_eq!(cache.reading.status, MetricStatus::Unsupported);
        cache.apply(vec![], None, None, 2000);
        cache.expire(3000);
        assert_eq!(cache.reading.status, MetricStatus::Unsupported);
    }
    #[test]
    fn revoked_temperature_access_discards_cached_values_and_history() {
        let mut cache = GpuTemperatureCache::default();
        cache.apply(vec![sample("a", 51.0)], None, Some("a"), 1000);
        cache.error(&PlatformError::new(
            PlatformErrorCode::AccessDenied,
            "access revoked",
        ));
        assert!(cache.reading.value.is_none());
        assert!(cache.history(2000).is_empty());
    }
    #[test]
    fn diagnostic_eviction_preserves_source_identity_and_recovery_gap() {
        let mut cache = GpuTemperatureCache::default();
        cache.apply(vec![sample("a", 51.0)], None, Some("a"), 1000);
        cache.error(&PlatformError::new(
            PlatformErrorCode::OperationFailed,
            "temporary error",
        ));
        assert!(!cache.expire(31000));
        assert!(cache.reading.value.is_some());
        assert!(cache.expire(31001));
        assert!(cache.reading.value.is_none());
        cache.apply(vec![sample("a", 52.0)], None, Some("a"), 33000);
        assert_eq!(cache.history(33000).len(), 2);
        assert!(cache.history(33000)[1].gap_before);
        cache.expire(64000);
        let mut replacement = sample("a", 53.0);
        replacement.reading.as_mut().unwrap().adapter_name = "Replacement".into();
        cache.apply(vec![replacement], None, Some("a"), 65000);
        assert_eq!(
            cache.history(65000).len(),
            1,
            "evicting the old numeric value cannot forget the source meaning"
        );
    }
}
