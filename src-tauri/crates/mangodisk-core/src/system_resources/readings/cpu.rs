use super::ResourceCache;
use crate::system_resources::{
    cpu::CpuBaselineReason,
    metrics::{MetricId, MetricReading, MetricStatus, TrendPoint},
};
use mangodisk_platform::system_resources::cpu::{
    details::CpuDetails, temperature::TEMPERATURE_FRESHNESS_MS, CpuSample,
};

impl ResourceCache {
    pub fn cpu(
        &mut self,
        counters: impl Into<CpuSample>,
        monotonic_ms: u64,
        timestamp_ms: u64,
    ) -> Option<CpuBaselineReason> {
        match self.cpu_delta.observe(counters.into(), monotonic_ms) {
            Ok(value) => {
                self.cpu_history.push(TrendPoint {
                    sampled_at_ms: timestamp_ms,
                    primary: value.used_percent,
                    secondary: None,
                    gap_before: false,
                });
                self.readings.cpu = MetricReading::ready(value, timestamp_ms);
                None
            }
            Err(reason) => {
                // A rejected interval is not a new reading. Keep the last real
                // value and history with their original timestamps until expiry;
                // rebuilding one baseline must not erase an entire minute.
                self.readings
                    .cpu
                    .expire(timestamp_ms, MetricId::Cpu.freshness_ms());
                Some(reason)
            }
        }
    }

    pub fn cpu_details(&mut self, details: CpuDetails, timestamp_ms: u64) {
        self.readings.cpu_identity = Some(details.identity);
        match details.temperature {
            Some(Ok(value))
                if value.celsius.is_finite()
                    && value.celsius > 0.0
                    && value.celsius <= 150.0
                    && value.sensor_count > 0
                    && value.sampled_at_ms.is_none_or(|sampled| {
                        sampled <= timestamp_ms
                            && timestamp_ms - sampled <= TEMPERATURE_FRESHNESS_MS
                    }) =>
            {
                let sampled_at_ms = value.sampled_at_ms.unwrap_or(timestamp_ms);
                let repeated = self.readings.cpu_temperature.sampled_at_ms == Some(sampled_at_ms)
                    && self
                        .readings
                        .cpu_temperature
                        .value
                        .as_ref()
                        .is_some_and(|previous| {
                            previous.kind == value.kind
                                && previous.source == value.source
                                && previous.sensor_count == value.sensor_count
                        });
                if self
                    .readings
                    .cpu_temperature
                    .value
                    .as_ref()
                    .is_some_and(|previous| {
                        previous.kind != value.kind
                            || previous.source != value.source
                            || previous.sensor_count != value.sensor_count
                    })
                {
                    self.cpu_temperature_history.clear();
                }
                // Service cache hits are the same native observation. Do not
                // reset the trend or manufacture a newer sample timestamp.
                if !repeated {
                    self.cpu_temperature_history.push(TrendPoint {
                        sampled_at_ms,
                        primary: value.celsius,
                        secondary: None,
                        gap_before: false,
                    });
                }
                self.readings.cpu_temperature = MetricReading::ready(value, sampled_at_ms);
            }
            Some(result) => {
                let unsupported = result.as_ref().err().is_some_and(|error| {
                    error.code() == mangodisk_platform::PlatformErrorCode::Unsupported
                });
                self.readings.cpu_temperature = MetricReading {
                    status: if unsupported {
                        MetricStatus::Unsupported
                    } else {
                        MetricStatus::Failed
                    },
                    value: None,
                    sampled_at_ms: None,
                };
                // A recovery must not connect through a known failed observation or
                // combine different sensor sets after the old value is discarded.
                self.cpu_temperature_history.clear();
            }
            None => self
                .readings
                .cpu_temperature
                .expire(timestamp_ms, TEMPERATURE_FRESHNESS_MS),
        }
        match details.frequency {
            Some(Ok(Some(value))) => {
                self.readings.cpu_frequency = MetricReading::ready(value, timestamp_ms)
            }
            Some(Err(error)) => {
                self.readings.cpu_frequency.status =
                    if error.code() == mangodisk_platform::PlatformErrorCode::Unsupported {
                        MetricStatus::Unsupported
                    } else {
                        MetricStatus::Failed
                    };
                self.readings.cpu_frequency.value = None;
                self.readings.cpu_frequency.sampled_at_ms = None;
            }
            _ => self
                .readings
                .cpu_frequency
                .expire(timestamp_ms, MetricId::Cpu.freshness_ms()),
        }
    }
}
