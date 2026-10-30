use super::ResourceCache;
use crate::system_resources::{
    cpu::CpuBaselineReason,
    metrics::{MetricId, MetricReading, MetricStatus, TrendPoint, TEMPERATURE_RETENTION_MS},
};
use mangodisk_platform::system_resources::cpu::{
    details::{CpuDetails, CpuFrequency, CpuIdentity},
    temperature::{CpuTemperature, TEMPERATURE_FRESHNESS_MS},
    CpuSample,
};

#[derive(PartialEq, Eq)]
pub(super) struct TemperatureIdentity {
    source: mangodisk_platform::system_resources::cpu::temperature::CpuTemperatureSource,
    kind: mangodisk_platform::system_resources::cpu::temperature::CpuTemperatureKind,
    count: u32,
}

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
        self.cpu_temperature(details.temperature, timestamp_ms);
        self.cpu_frequency(details.identity, details.frequency, timestamp_ms);
    }

    pub fn cpu_identity(&mut self, identity: CpuIdentity) {
        self.readings.cpu_identity = Some(identity);
    }

    pub fn cpu_temperature(
        &mut self,
        result: Option<mangodisk_platform::PlatformResult<CpuTemperature>>,
        timestamp_ms: u64,
    ) {
        match result {
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
                let identity = TemperatureIdentity {
                    source: value.source,
                    kind: value.kind,
                    count: value.sensor_count,
                };
                if self
                    .cpu_temperature_identity
                    .as_ref()
                    .is_some_and(|previous| previous != &identity)
                {
                    self.cpu_temperature_history.clear();
                }
                self.cpu_temperature_identity = Some(identity);
                // Service cache hits are the same native observation. Do not
                // reset the trend or manufacture a newer sample timestamp.
                if !repeated {
                    self.cpu_temperature_history.push(TrendPoint {
                        sampled_at_ms,
                        primary: value.celsius,
                        secondary: None,
                        gap_before: self.cpu_temperature_interrupted,
                    });
                    self.cpu_temperature_interrupted = false;
                }
                self.readings.cpu_temperature = MetricReading::ready(value, sampled_at_ms);
            }
            Some(result) => {
                let invalidated = result.as_ref().err().is_some_and(|error| {
                    matches!(
                        error.code(),
                        mangodisk_platform::PlatformErrorCode::Unsupported
                            | mangodisk_platform::PlatformErrorCode::ItemChanged
                            | mangodisk_platform::PlatformErrorCode::AccessDenied
                    )
                });
                let status =
                    if result.as_ref().err().is_some_and(|e| {
                        e.code() == mangodisk_platform::PlatformErrorCode::Unsupported
                    }) {
                        MetricStatus::Unsupported
                    } else {
                        MetricStatus::Failed
                    };
                self.cpu_temperature_interrupted = true;
                if invalidated {
                    self.readings.cpu_temperature = MetricReading {
                        status,
                        value: None,
                        sampled_at_ms: None,
                    };
                    self.cpu_temperature_history.clear();
                    self.cpu_temperature_identity = None;
                } else {
                    // Retain a bounded diagnostic observation without making it live.
                    self.readings.cpu_temperature.mark_unavailable(status);
                }
            }
            None => {}
        }
        self.expire_cpu_temperature(timestamp_ms);
    }

    pub fn fail_cpu_frequency(&mut self) {
        self.readings.cpu_frequency = MetricReading {
            status: MetricStatus::Failed,
            sampled_at_ms: None,
            value: None,
        };
    }

    pub fn cpu_frequency(
        &mut self,
        identity: CpuIdentity,
        frequency: Option<mangodisk_platform::PlatformResult<Option<CpuFrequency>>>,
        timestamp_ms: u64,
    ) {
        self.cpu_identity(identity);
        match frequency {
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
    pub(super) fn expire_cpu_temperature(&mut self, now_ms: u64) -> bool {
        let had_value = self.readings.cpu_temperature.value.is_some();
        let changed = self.readings.cpu_temperature.expire_retained(
            now_ms,
            TEMPERATURE_FRESHNESS_MS,
            TEMPERATURE_RETENTION_MS,
        );
        if had_value && self.readings.cpu_temperature.value.is_none() {
            self.cpu_temperature_interrupted = true;
        }
        changed
    }
}
