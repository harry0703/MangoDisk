//! Per-observation acquisition times, independent of later reads in the same worker.
use mangodisk_platform::{PlatformError, PlatformErrorCode, PlatformResult};
use std::time::Instant;

pub struct Observed<T> {
    pub value: T,
    pub sampled_at_ms: u64,
    pub duration_ms: u64,
}
impl<T> Observed<T> {
    pub fn read(read: impl FnOnce() -> T) -> Self {
        let started = Instant::now();
        let value = read();
        Self {
            value,
            sampled_at_ms: super::sampling_workers::timestamp_ms(),
            duration_ms: started.elapsed().as_millis() as u64,
        }
    }
    pub fn fresh(&self, now_ms: u64, lifetime_ms: u64) -> bool {
        now_ms >= self.sampled_at_ms
            && now_ms - self.sampled_at_ms <= lifetime_ms
            && self.duration_ms <= lifetime_ms
    }
    #[cfg(test)]
    pub fn at(value: T, sampled_at_ms: u64) -> Self {
        Self {
            value,
            sampled_at_ms,
            duration_ms: 0,
        }
    }
}
pub type NativeReading<T> = Observed<PlatformResult<T>>;
impl<T> Observed<Option<T>> {
    pub fn into_optional(self) -> Option<Observed<T>> {
        self.value.map(|value| Observed {
            value,
            sampled_at_ms: self.sampled_at_ms,
            duration_ms: self.duration_ms,
        })
    }
}
impl<T> Observed<PlatformResult<T>> {
    pub fn result(self, now_ms: u64, lifetime_ms: u64) -> PlatformResult<T> {
        if self.value.is_err() || self.fresh(now_ms, lifetime_ms) {
            self.value
        } else {
            Err(PlatformError::new(PlatformErrorCode::OperationFailed,
            format!("sampling stage=validate reason=expired sampled_at_ms={} observed_at_ms={now_ms} duration_ms={} freshness_ms={lifetime_ms}", self.sampled_at_ms, self.duration_ms)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn freshness_uses_the_observations_own_time_and_native_duration() {
        let reading: Observed<PlatformResult<i32>> = Observed::at(Ok(42), 1000);
        assert!(reading.fresh(6000, 5000));
        assert!(!reading.fresh(6001, 5000));
        assert!(!reading.fresh(999, 5000));
        let slow = Observed {
            value: Ok(42),
            sampled_at_ms: 9000,
            duration_ms: 5001,
        };
        assert!(slow.result(9000, 5000).is_err());
    }
    #[test]
    fn expired_errors_never_lose_capability_or_permission_semantics() {
        for code in [
            PlatformErrorCode::AccessDenied,
            PlatformErrorCode::Unsupported,
            PlatformErrorCode::ItemChanged,
        ] {
            let reading: Observed<PlatformResult<()>> =
                Observed::at(Err(PlatformError::new(code, "native failure")), 1000);
            assert_eq!(reading.result(20000, 5000).unwrap_err().code(), code);
        }
    }
}
