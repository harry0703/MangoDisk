//! Independent component power status never fabricates a zero on failure.
use super::{MetricReading, MetricStatus, ResourceCache};
use mangodisk_platform::{
    system_resources::component_power::{ComponentPower, GpuPower, COMPONENT_POWER_MAX_WATTS},
    PlatformErrorCode, PlatformResult,
};

pub(super) fn initial<T>() -> MetricReading<T> {
    MetricReading {
        status: if cfg!(target_os = "macos") {
            MetricStatus::Loading
        } else {
            MetricStatus::Unsupported
        },
        sampled_at_ms: None,
        value: None,
    }
}
fn reading<T>(
    sample: PlatformResult<T>,
    at: u64,
    valid: impl FnOnce(&T) -> bool,
) -> MetricReading<T> {
    match sample {
        Ok(value) if valid(&value) => MetricReading::ready(value, at),
        result => MetricReading {
            status: if result.is_err_and(|e| e.code() == PlatformErrorCode::Unsupported) {
                MetricStatus::Unsupported
            } else {
                MetricStatus::Failed
            },
            sampled_at_ms: None,
            value: None,
        },
    }
}
fn valid(watts: f64) -> bool {
    watts.is_finite() && (0.0..=COMPONENT_POWER_MAX_WATTS).contains(&watts)
}
impl ResourceCache {
    pub fn cpu_power(&mut self, sample: PlatformResult<ComponentPower>, at: u64) {
        self.readings.cpu_power = reading(sample, at, |power| valid(power.watts));
    }
    pub fn gpu_power(&mut self, sample: PlatformResult<GpuPower>, at: u64) {
        self.readings.gpu_power = reading(sample, at, |power| {
            !power.adapter_id.is_empty() && valid(power.power.watts)
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mangodisk_platform::{
        system_resources::component_power::ComponentPowerSource, PlatformError,
    };
    #[test]
    fn intel_smc_power_preserves_its_source_and_cache_contract() {
        let mut cache = ResourceCache::default();
        cache.cpu_power(
            Ok(ComponentPower {
                watts: 12.5,
                source: ComponentPowerSource::AppleSmc,
            }),
            1000,
        );
        let wire = serde_json::to_value(cache.snapshot(1000)).unwrap();
        assert_eq!(wire["schemaVersion"], 39);
        assert_eq!(wire["cpuPower"]["value"]["source"], "appleSmc");
        assert_eq!(wire["cpuPower"]["value"]["watts"], 12.5);
        assert_eq!(cache.snapshot(6001).cpu_power.status, MetricStatus::Stale);
    }
    #[test]
    fn removed_gpu_invalidates_power_before_the_replacement_has_a_baseline() {
        use mangodisk_platform::system_resources::gpu::GpuAdapter;
        let mut cache = ResourceCache::default();
        cache.gpu_adapters(vec![GpuAdapter {
            id: "original".into(),
            name: "Apple GPU".into(),
        }]);
        cache.gpu_power(
            Ok(GpuPower {
                adapter_id: "original".into(),
                power: ComponentPower {
                    watts: 20.0,
                    source: ComponentPowerSource::IoReport,
                },
            }),
            1000,
        );
        cache.gpu_adapters(vec![GpuAdapter {
            id: "original".into(),
            name: "Renamed Apple GPU".into(),
        }]);
        assert_eq!(cache.snapshot(1500).gpu_power.status, MetricStatus::Ready);
        cache.gpu_adapters(vec![GpuAdapter {
            id: "replacement".into(),
            name: "Apple GPU".into(),
        }]);
        let snapshot = cache.snapshot(2000);
        assert_eq!(snapshot.gpu_power.status, MetricStatus::Disconnected);
        assert!(snapshot.gpu_power.value.is_none());
        assert!(snapshot.gpu_power.sampled_at_ms.is_none());
        cache.gpu_adapters(vec![GpuAdapter {
            id: "original".into(),
            name: "Apple GPU".into(),
        }]);
        assert_eq!(
            cache.snapshot(2500).gpu_power.status,
            MetricStatus::Disconnected
        );
        cache.gpu_power(
            Ok(GpuPower {
                adapter_id: "original".into(),
                power: ComponentPower {
                    watts: 0.0,
                    source: ComponentPowerSource::IoReport,
                },
            }),
            3000,
        );
        assert_eq!(cache.snapshot(3000).gpu_power.status, MetricStatus::Ready);
    }
    #[test]
    fn independent_failures_and_expiry_do_not_become_idle_power() {
        let mut cache = ResourceCache::default();
        let power = |watts| ComponentPower {
            watts,
            source: ComponentPowerSource::IoReport,
        };
        cache.cpu_power(Ok(power(0.0)), 1000);
        cache.gpu_power(
            Ok(GpuPower {
                adapter_id: "apple-gpu".into(),
                power: power(20.0),
            }),
            1000,
        );
        assert_eq!(cache.snapshot(1000).cpu_power.value.unwrap().watts, 0.0);
        cache.cpu_power(
            Err(PlatformError::new(PlatformErrorCode::Io, "counter failure")),
            2000,
        );
        let sample = cache.snapshot(2000);
        assert_eq!(sample.cpu_power.status, MetricStatus::Failed);
        assert!(sample.cpu_power.value.is_none());
        assert_eq!(sample.gpu_power.status, MetricStatus::Ready);
        assert_eq!(cache.snapshot(6001).gpu_power.status, MetricStatus::Stale);
        cache.cpu_power(Ok(power(f64::NAN)), 7000);
        assert_eq!(cache.snapshot(7000).cpu_power.status, MetricStatus::Failed);
        cache.cpu_power(Ok(power(5.0)), 8000);
        assert_eq!(cache.snapshot(8000).cpu_power.status, MetricStatus::Ready);
    }
}
