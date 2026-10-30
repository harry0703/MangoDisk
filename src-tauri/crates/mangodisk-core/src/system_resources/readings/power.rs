//! Cached power readings preserve independent failures, freshness, and bounded history.
use super::ResourceCache;
use crate::system_resources::metrics::{MetricReading, MetricStatus, TrendPoint};
use mangodisk_platform::system_resources::battery::{BatteryPowerState, BatterySample};

impl ResourceCache {
    pub fn system_power(
        &mut self,
        sample: mangodisk_platform::PlatformResult<
            mangodisk_platform::system_resources::power::SystemPower,
        >,
        timestamp_ms: u64,
    ) {
        use mangodisk_platform::system_resources::power::SYSTEM_POWER_MAX_WATTS;
        self.readings.system_power = match sample {
            Ok(value)
                if value.watts.is_finite()
                    && (0.0..=SYSTEM_POWER_MAX_WATTS).contains(&value.watts) =>
            {
                self.system_power_history.push(TrendPoint {
                    sampled_at_ms: timestamp_ms,
                    primary: value.watts,
                    secondary: None,
                    gap_before: std::mem::take(&mut self.system_power_interrupted),
                });
                MetricReading::ready(value, timestamp_ms)
            }
            result => {
                self.system_power_interrupted = true;
                MetricReading {
                    status: match result {
                        Err(error)
                            if error.code()
                                == mangodisk_platform::PlatformErrorCode::Unsupported =>
                        {
                            MetricStatus::Unsupported
                        }
                        _ => MetricStatus::Failed,
                    },
                    sampled_at_ms: None,
                    value: None,
                }
            }
        };
    }

    pub fn battery_presence(&mut self, present: Option<bool>, timestamp_ms: u64) {
        if let Some(present) = present {
            self.readings.battery_present = Some(present);
            if !present {
                self.battery(
                    Err(mangodisk_platform::PlatformError::new(
                        mangodisk_platform::PlatformErrorCode::Unsupported,
                        "physical battery absent",
                    )),
                    timestamp_ms,
                );
            }
        }
    }

    pub fn battery(
        &mut self,
        sample: mangodisk_platform::PlatformResult<BatterySample>,
        timestamp_ms: u64,
    ) {
        fn unavailable<T>(status: MetricStatus) -> MetricReading<T> {
            MetricReading {
                status,
                sampled_at_ms: None,
                value: None,
            }
        }
        match sample {
            Ok(sample) => {
                self.readings.battery_details = sample.details.map_or_else(
                    || unavailable(MetricStatus::Unsupported),
                    |value| MetricReading::ready(value, timestamp_ms),
                );
                self.readings.battery_power = match sample.battery_power {
                    Some(value)
                        if value.watts.is_finite()
                            && (0.0..=300.0).contains(&value.watts)
                            && (value.state != BatteryPowerState::Idle || value.watts == 0.0) =>
                    {
                        MetricReading::ready(value, timestamp_ms)
                    }
                    Some(_) => MetricReading {
                        status: MetricStatus::Failed,
                        sampled_at_ms: None,
                        value: None,
                    },
                    None => MetricReading {
                        status: MetricStatus::Unsupported,
                        sampled_at_ms: None,
                        value: None,
                    },
                };
                self.readings.battery_temperature = match sample.temperature {
                    Some(value)
                        if value.celsius.is_finite()
                            && value.celsius > 0.0
                            && value.celsius <= 100.0 =>
                    {
                        MetricReading::ready(value, timestamp_ms)
                    }
                    Some(_) => unavailable(MetricStatus::Failed),
                    None => unavailable(MetricStatus::Unsupported),
                };
                self.readings.charging_power = match sample.charging_power {
                    Some(value)
                        if value.watts.is_finite()
                            && (0.0..=300.0).contains(&value.watts)
                            && ((value.charging && value.external_connected)
                                || value.watts == 0.0) =>
                    {
                        MetricReading::ready(value, timestamp_ms)
                    }
                    Some(_) => MetricReading {
                        status: MetricStatus::Failed,
                        sampled_at_ms: None,
                        value: None,
                    },
                    None => MetricReading {
                        status: MetricStatus::Unsupported,
                        sampled_at_ms: None,
                        value: None,
                    },
                };
            }
            Err(error) => {
                let status = if error.code() == mangodisk_platform::PlatformErrorCode::Unsupported {
                    MetricStatus::Unsupported
                } else if error.code() == mangodisk_platform::PlatformErrorCode::ItemChanged {
                    MetricStatus::Disconnected
                } else {
                    MetricStatus::Failed
                };
                self.readings.battery_details.mark_unavailable(status);
                self.readings.battery_temperature = unavailable(status);
                self.readings.charging_power = MetricReading {
                    status,
                    sampled_at_ms: None,
                    value: None,
                };
                self.readings.battery_power = MetricReading {
                    status,
                    sampled_at_ms: None,
                    value: None,
                };
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mangodisk_platform::{
        system_resources::{
            battery::details::{BatteryDetails, ChargeState},
            power::SystemPower,
        },
        PlatformError, PlatformErrorCode,
    };

    #[test]
    fn finishing_state_survives_cached_snapshots_and_updates_after_charge_completion() {
        let mut cache = ResourceCache::default();
        let sample = |charge_state| BatterySample {
            details: Some(BatteryDetails {
                charge_percent: Some(100.0),
                charge_state,
                ..Default::default()
            }),
            temperature: None,
            charging_power: None,
            battery_power: None,
        };
        cache.battery(Ok(sample(ChargeState::Finishing)), 1000);
        let cached = serde_json::to_value(cache.snapshot(6001)).unwrap();
        assert_eq!(cached["schemaVersion"], 39);
        assert_eq!(cached["batteryDetails"]["status"], "stale");
        assert_eq!(
            cached["batteryDetails"]["value"]["chargeState"],
            "finishing"
        );
        cache.battery(Ok(sample(ChargeState::Full)), 7000);
        let fresh = serde_json::to_value(cache.snapshot(7000)).unwrap();
        assert_eq!(fresh["batteryDetails"]["status"], "ready");
        assert_eq!(fresh["batteryDetails"]["value"]["chargeState"], "full");
    }

    #[test]
    fn battery_details_keep_original_observation_on_failure_and_clear_on_removal() {
        let mut cache = ResourceCache::default();
        let sample = || BatterySample {
            details: Some(BatteryDetails {
                charge_percent: Some(80.0),
                ..Default::default()
            }),
            temperature: None,
            charging_power: None,
            battery_power: None,
        };
        cache.battery(Ok(sample()), 1000);
        cache.system_power(Ok(SystemPower { watts: 20.0 }), 6001);
        let expired = cache.snapshot(6001);
        assert_eq!(expired.battery_details.status, MetricStatus::Stale);
        assert_eq!(expired.battery_details.sampled_at_ms, Some(1000));
        assert_eq!(expired.system_power.status, MetricStatus::Ready);
        cache.battery(Ok(sample()), 7000);
        assert_eq!(
            cache.snapshot(7000).battery_details.status,
            MetricStatus::Ready
        );
        cache.battery(
            Err(PlatformError::new(
                PlatformErrorCode::Io,
                "test query failure",
            )),
            8000,
        );
        assert_eq!(
            cache.snapshot(8000).battery_details.sampled_at_ms,
            Some(7000)
        );
        assert!(cache.snapshot(8000).battery_details.value.is_some());
        assert_eq!(
            cache.snapshot(8000).battery_details.status,
            MetricStatus::Failed
        );
        cache.battery(Ok(sample()), 9000);
        cache.battery_presence(Some(false), 9001);
        assert_eq!(
            cache.snapshot(9001).battery_details.status,
            MetricStatus::Unsupported
        );
        assert!(cache.snapshot(9001).battery_details.value.is_none());
    }

    #[test]
    fn quick_power_failure_recovery_retains_an_explicit_history_boundary() {
        let mut cache = ResourceCache::default();
        cache.system_power(Ok(SystemPower { watts: 20.0 }), 1000);
        cache.system_power(
            Err(PlatformError::new(
                PlatformErrorCode::Io,
                "transient PSTR failure",
            )),
            1250,
        );
        cache.system_power(Ok(SystemPower { watts: f64::NAN }), 1300);
        assert_eq!(cache.snapshot(1300).system_power_history.len(), 1);
        cache.system_power(Ok(SystemPower { watts: 21.0 }), 1500);
        cache.system_power(Ok(SystemPower { watts: 22.0 }), 2000);
        let wire = serde_json::to_value(cache.snapshot(2000)).unwrap();
        assert_eq!(wire["systemPowerHistory"].as_array().unwrap().len(), 3);
        assert!(wire["systemPowerHistory"][0].get("gapBefore").is_none());
        assert_eq!(wire["systemPowerHistory"][1]["gapBefore"], true);
        assert!(wire["systemPowerHistory"][2].get("gapBefore").is_none());
    }

    #[test]
    fn system_power_history_is_bounded_and_never_fills_failed_samples_with_zero() {
        let mut cache = ResourceCache::default();
        cache.system_power(Ok(SystemPower { watts: 0.0 }), 1000);
        cache.system_power(Ok(SystemPower { watts: f64::NAN }), 3000);
        cache.system_power(
            Err(PlatformError::new(
                PlatformErrorCode::Io,
                "test sensor failure",
            )),
            5000,
        );
        let failed = cache.snapshot(5000);
        assert_eq!(failed.system_power.status, MetricStatus::Failed);
        assert_eq!(failed.system_power_history.len(), 1);
        assert_eq!(failed.system_power_history[0].primary, 0.0);
        cache.system_power(Ok(SystemPower { watts: 30.0 }), 7000);
        let resumed = cache.snapshot(7000);
        assert_eq!(resumed.system_power_history.len(), 2);
        assert_eq!(resumed.system_power_history[1].sampled_at_ms, 7000);
        for at in (9000..=209000).step_by(2000) {
            cache.system_power(Ok(SystemPower { watts: 10.0 }), at);
        }
        let retained = cache.snapshot(209000);
        assert!(retained.system_power_history.len() <= 41);
        assert!(retained.system_power_history[0].sampled_at_ms >= 129000);
        assert!(cache.snapshot(300000).system_power_history.is_empty());
    }
}
