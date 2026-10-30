//! One CPU observation shared across clients; requests never accelerate native sampling.
use mangodisk_privileged_protocol::{
    CapabilityStatus, Reply, TemperatureReading, IDLE_RELEASE_MS, SAMPLE_INTERVAL_MS,
};
use std::time::{Duration, Instant};

#[derive(Default)]
pub(crate) struct CpuCache {
    result: Option<(CapabilityStatus, Option<TemperatureReading>)>,
    due: Option<Instant>,
    failures: u8,
    pub(crate) idle: Option<Instant>,
}
impl CpuCache {
    pub(crate) fn release(&mut self) {
        *self = Self::default();
    }
    pub(crate) fn read(
        &mut self,
        now: Instant,
        sample: impl FnOnce() -> Result<TemperatureReading, CapabilityStatus>,
    ) -> Reply {
        self.idle = Some(now + Duration::from_millis(IDLE_RELEASE_MS));
        if self.due.is_none_or(|due| now >= due) {
            let state = match sample() {
                Ok(reading) => (CapabilityStatus::Ready, Some(reading)),
                Err(status) => (status, None),
            };
            let delay = match state.0 {
                CapabilityStatus::Ready => {
                    self.failures = 0;
                    SAMPLE_INTERVAL_MS
                }
                CapabilityStatus::SensorUnavailable => {
                    self.failures = self.failures.saturating_add(1);
                    SAMPLE_INTERVAL_MS
                        .saturating_mul(1u64 << self.failures.saturating_sub(1).min(3))
                        .min(30_000)
                }
                _ => {
                    self.failures = 0;
                    30_000
                }
            };
            self.due = Some(now + Duration::from_millis(delay));
            self.result = Some(state);
        }
        let (status, reading) = self.result.clone().expect("CPU result initialized");
        Reply::CpuTemperature { status, reading }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sample() -> TemperatureReading {
        TemperatureReading {
            celsius: 44.0,
            sampled_at_ms: 1000,
        }
    }
    #[test]
    fn repeated_clients_share_one_real_sample_and_keep_its_timestamp() {
        let mut cache = CpuCache::default();
        let start = Instant::now();
        cache.read(start, || Ok(sample()));
        for milliseconds in [0, 1, 1000, 3999] {
            let reply = cache.read(start + Duration::from_millis(milliseconds), || {
                panic!("cache hit must not read hardware")
            });
            let Reply::CpuTemperature {
                reading: Some(reading),
                ..
            } = reply
            else {
                panic!("expected a cached observation");
            };
            assert_eq!(reading.sampled_at_ms, 1000);
        }
        let mut reads = 0;
        cache.read(start + Duration::from_millis(4000), || {
            reads += 1;
            Ok(sample())
        });
        assert_eq!(reads, 1);
    }
    #[test]
    fn failed_sampling_discards_previous_readings_and_backs_off() {
        let mut cache = CpuCache::default();
        let start = Instant::now();
        cache.read(start, || Ok(sample()));
        let reply = cache.read(start + Duration::from_secs(4), || {
            Err(CapabilityStatus::SensorUnavailable)
        });
        assert!(matches!(
            reply,
            Reply::CpuTemperature {
                reading: None,
                status: CapabilityStatus::SensorUnavailable
            }
        ));
        cache.read(start + Duration::from_secs(7), || {
            panic!("failed sensors must back off")
        });
        cache.read(start + Duration::from_secs(8), || Ok(sample()));
    }
    #[test]
    fn idle_release_clears_the_cache_and_reopening_reads_fresh_hardware() {
        let mut cache = CpuCache::default();
        let start = Instant::now();
        assert!(cache.idle.is_none());
        cache.read(start, || Ok(sample()));
        assert_eq!(
            cache.idle,
            Some(start + Duration::from_millis(IDLE_RELEASE_MS))
        );
        cache.release();
        assert!(cache.idle.is_none());
        let reply = cache.read(start + Duration::from_secs(1), || {
            Err(CapabilityStatus::DriverMissing)
        });
        assert!(matches!(
            reply,
            Reply::CpuTemperature {
                reading: None,
                status: CapabilityStatus::DriverMissing
            }
        ));
    }
    #[test]
    fn cached_failures_do_not_accelerate_or_extend_native_recovery() {
        let mut cache = CpuCache::default();
        let mut at = Instant::now();
        for delay in [4000, 8000, 16000, 30000, 30000] {
            cache.read(at, || Err(CapabilityStatus::SensorUnavailable));
            assert_eq!(cache.due, Some(at + Duration::from_millis(delay)));
            cache.read(at + Duration::from_millis(delay - 1), || {
                panic!("a cached failure must not retry hardware")
            });
            at += Duration::from_millis(delay);
        }
        cache.read(at, || Ok(sample()));
        assert_eq!(cache.failures, 0);
        assert_eq!(
            cache.due,
            Some(at + Duration::from_millis(SAMPLE_INTERVAL_MS))
        );
        for state in [
            CapabilityStatus::DriverMissing,
            CapabilityStatus::DriverAccessDenied,
            CapabilityStatus::UnsupportedHardware,
        ] {
            cache.release();
            cache.read(at, || Err(state));
            assert_eq!(cache.due, Some(at + Duration::from_secs(30)));
        }
    }
}
