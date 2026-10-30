use crate::system_resources::cpu::CpuBaselineReason;
use mangodisk_platform::system_resources::battery::{BatteryPowerState, BatterySample};
use mangodisk_platform::system_resources::cpu::{details::CpuDetails, CpuSample};
#[test]
fn disk_temperature_has_independent_freshness_and_clears_failed_or_changed_sources() {
    use super::*;
    use mangodisk_platform::system_resources::disk_temperature::{
        DiskTemperature, DiskTemperatureSource,
    };
    let mut cache = ResourceCache::default();
    let sample = || DiskTemperature {
        volume_id: "system".into(),
        volume_name: "Disk".into(),
        celsius: 34.85,
        source: DiskTemperatureSource::NvmeSmart,
    };
    cache.disk_temperature(Ok(sample()), 1000);
    assert_eq!(
        cache.snapshot(91_000).disk_temperature.status,
        MetricStatus::Ready
    );
    assert_eq!(
        cache.snapshot(91_001).disk_temperature.status,
        MetricStatus::Stale
    );
    cache.disk_temperature(
        Err(mangodisk_platform::PlatformError::new(
            mangodisk_platform::PlatformErrorCode::Unsupported,
            "no sensor",
        )),
        92_000,
    );
    assert_eq!(
        cache.snapshot(92_000).disk_temperature.status,
        MetricStatus::Unsupported
    );
    assert!(cache.snapshot(92_000).disk_temperature.value.is_none());
    let mut invalid = sample();
    invalid.celsius = f64::NAN;
    cache.disk_temperature(Ok(invalid), 93_000);
    assert_eq!(
        cache.snapshot(93_000).disk_temperature.status,
        MetricStatus::Failed
    );
    cache.disk_temperature(Ok(sample()), 94_000);
    cache.reset(MetricId::Disk);
    assert!(cache.snapshot(94_000).disk_temperature.value.is_none());
    cache.disk_temperature(Ok(sample()), 95_000);
    cache.clear_disk_temperature();
    assert_eq!(
        cache.snapshot(95_000).disk_temperature.status,
        MetricStatus::Loading
    );
}
#[test]
fn absent_battery_clears_only_battery_values_and_unknown_preserves_presence() {
    use super::*;
    use mangodisk_platform::system_resources::{battery::BatterySource, power::SystemPower};
    let mut cache = ResourceCache::default();
    cache.system_power(Ok(SystemPower { watts: 25.0 }), 1000);
    cache.battery_presence(Some(true), 1000);
    cache.battery(
        Ok(BatterySample {
            details: None,
            temperature: Some(BatteryTemperature {
                celsius: 30.0,
                source: BatterySource::AppleSmc,
            }),
            charging_power: Some(ChargingPower {
                watts: 0.0,
                charging: false,
                external_connected: true,
                source: BatterySource::AppleSmartBattery,
            }),
            battery_power: Some(BatteryPower {
                watts: 0.0,
                state: BatteryPowerState::Idle,
                source: BatterySource::AppleSmartBattery,
            }),
        }),
        1000,
    );
    cache.battery_presence(None, 1100);
    assert_eq!(cache.snapshot(1100).battery_present, Some(true));
    cache.battery_presence(Some(false), 1200);
    let reading = cache.snapshot(1200);
    assert_eq!(reading.battery_present, Some(false));
    assert_eq!(
        reading.battery_temperature.status,
        MetricStatus::Unsupported
    );
    assert!(reading.battery_temperature.value.is_none());
    assert!(reading.charging_power.value.is_none());
    assert!(reading.battery_power.value.is_none());
    assert_eq!(reading.system_power.value.unwrap().watts, 25.0);
    cache.battery_presence(None, 1300);
    assert_eq!(cache.snapshot(1300).battery_present, Some(false));
}

#[test]
fn system_power_expiry_and_failures_never_fabricate_zero_watts() {
    use super::*;
    use mangodisk_platform::{
        system_resources::power::SystemPower, PlatformError, PlatformErrorCode,
    };
    let mut cache = ResourceCache::default();
    cache.system_power(Ok(SystemPower { watts: 27.5 }), 1000);
    assert_eq!(
        cache.snapshot(6000).system_power.status,
        MetricStatus::Ready
    );
    assert_eq!(
        cache.snapshot(6001).system_power.status,
        MetricStatus::Stale
    );
    for (code, status) in [
        (PlatformErrorCode::Unsupported, MetricStatus::Unsupported),
        (PlatformErrorCode::OperationFailed, MetricStatus::Failed),
    ] {
        cache.system_power(Err(PlatformError::new(code, "SMC PSTR test failure")), 7000);
        let reading = cache.snapshot(7000).system_power;
        assert_eq!(reading.status, status);
        assert!(reading.value.is_none());
        assert!(reading.sampled_at_ms.is_none());
    }
    for watts in [f64::NAN, f64::INFINITY, -1.0, 3000.1] {
        cache.system_power(Ok(SystemPower { watts }), 8000);
        assert_eq!(
            cache.snapshot(8000).system_power.status,
            MetricStatus::Failed
        );
        assert!(cache.snapshot(8000).system_power.value.is_none());
    }
    cache.system_power(Ok(SystemPower { watts: 0.0 }), 9000);
    assert_eq!(cache.snapshot(9000).system_power.value.unwrap().watts, 0.0);
}

use super::*;
use mangodisk_platform::system_resources::cpu::CpuCounters;

#[test]
fn battery_fields_expire_independently_and_failures_cannot_become_zero_watts() {
    use mangodisk_platform::system_resources::battery::BatterySource;
    let mut cache = ResourceCache::default();
    cache.battery(
        Ok(BatterySample {
            details: None,
            battery_power: Some(BatteryPower {
                watts: 24.0,
                state: BatteryPowerState::Discharging,
                source: BatterySource::AppleSmartBattery,
            }),
            temperature: Some(BatteryTemperature {
                celsius: 30.5,
                source: BatterySource::AppleSmartBattery,
            }),
            charging_power: Some(ChargingPower {
                watts: 0.0,
                charging: false,
                external_connected: true,
                source: BatterySource::AppleSmartBattery,
            }),
        }),
        1000,
    );
    assert_eq!(
        cache.snapshot(6000).charging_power.status,
        MetricStatus::Ready
    );
    assert_eq!(
        cache.snapshot(6001).battery_temperature.status,
        MetricStatus::Stale
    );
    assert_eq!(
        cache.snapshot(6001).battery_power.status,
        MetricStatus::Stale
    );
    cache.battery(
        Ok(BatterySample {
            details: None,
            battery_power: None,
            temperature: None,
            charging_power: None,
        }),
        7000,
    );
    assert_eq!(
        cache.snapshot(7000).charging_power.status,
        MetricStatus::Unsupported
    );
    assert!(cache.snapshot(7000).charging_power.value.is_none());
    assert!(cache.snapshot(7000).battery_power.value.is_none());
    cache.battery(
        Err(mangodisk_platform::PlatformError::new(
            mangodisk_platform::PlatformErrorCode::OperationFailed,
            "read failed",
        )),
        8000,
    );
    assert_eq!(
        cache.snapshot(8000).battery_temperature.status,
        MetricStatus::Failed
    );
    assert!(cache.snapshot(8000).charging_power.value.is_none());
    assert_eq!(
        cache.snapshot(8000).battery_power.status,
        MetricStatus::Failed
    );
}

#[test]
fn sampling_repro_transient_temperature_failure_retains_diagnostic_value() {
    use mangodisk_platform::system_resources::cpu::temperature::{
        CpuTemperatureKind, CpuTemperatureSource,
    };
    let mut cache = ResourceCache::default();
    let details = |temperature| CpuDetails {
        identity: CpuIdentity::default(),
        frequency: None,
        temperature,
    };
    cache.cpu_details(
        details(Some(Ok(CpuTemperature {
            celsius: 48.0,
            sensor_count: 1,
            sampled_at_ms: Some(1000),
            kind: CpuTemperatureKind::CoreAverage,
            source: CpuTemperatureSource::AppleSmc,
        }))),
        1000,
    );
    cache.cpu_details(
        details(Some(Err(mangodisk_platform::PlatformError::new(
            mangodisk_platform::PlatformErrorCode::OperationFailed,
            "temporary transport failure",
        )))),
        5000,
    );
    let reading = cache.snapshot(5000);
    assert_eq!(reading.cpu_temperature.status, MetricStatus::Failed);
    assert_eq!(
        reading.cpu_temperature.sampled_at_ms,
        Some(1000),
        "a temporary failure must retain the last real observation and its age"
    );
    assert_eq!(reading.cpu_temperature.value.unwrap().celsius, 48.0);
}

#[test]
fn cached_native_temperature_preserves_history_and_original_freshness() {
    use mangodisk_platform::system_resources::cpu::temperature::{
        CpuTemperatureKind, CpuTemperatureSource,
    };
    let mut cache = ResourceCache::default();
    let details = |sampled_at_ms| CpuDetails {
        identity: CpuIdentity::default(),
        frequency: None,
        temperature: Some(Ok(CpuTemperature {
            celsius: 48.0,
            sensor_count: 1,
            kind: CpuTemperatureKind::Package,
            source: CpuTemperatureSource::IntelDtsPawnIo,
            sampled_at_ms: Some(sampled_at_ms),
        })),
    };
    cache.cpu_details(details(1000), 1000);
    cache.cpu_details(details(5000), 5000);
    cache.cpu_details(details(5000), 6000);
    let reading = cache.snapshot(6000);
    assert_eq!(reading.cpu_temperature_history.len(), 2);
    assert_eq!(reading.cpu_temperature.sampled_at_ms, Some(5000));
    assert_eq!(
        cache.snapshot(15001).cpu_temperature.status,
        MetricStatus::Stale
    );
}

#[test]
fn native_temperature_rejects_future_or_expired_sample_timestamps() {
    use mangodisk_platform::system_resources::cpu::temperature::{
        CpuTemperatureKind, CpuTemperatureSource,
    };
    for sampled_at_ms in [1, 20_001] {
        let mut cache = ResourceCache::default();
        cache.cpu_details(
            CpuDetails {
                identity: CpuIdentity::default(),
                frequency: None,
                temperature: Some(Ok(CpuTemperature {
                    celsius: 50.0,
                    kind: CpuTemperatureKind::Package,
                    source: CpuTemperatureSource::IntelDtsPawnIo,
                    sensor_count: 1,
                    sampled_at_ms: Some(sampled_at_ms),
                })),
            },
            20_000,
        );
        let reading = cache.snapshot(20_000);
        assert_eq!(reading.cpu_temperature.status, MetricStatus::Failed);
        assert!(reading.cpu_temperature.value.is_none());
        assert!(reading.cpu_temperature_history.is_empty());
    }
}

#[test]
fn temperature_freshness_failure_and_sensor_identity_are_independent_of_cpu_load() {
    use mangodisk_platform::system_resources::cpu::temperature::{
        CpuTemperatureKind, CpuTemperatureSource,
    };
    use mangodisk_platform::{PlatformError, PlatformErrorCode};
    let mut cache = ResourceCache::default();
    let details = |temperature| CpuDetails {
        identity: CpuIdentity::default(),
        frequency: None,
        temperature,
    };
    let value = || CpuTemperature {
        celsius: 48.5,
        sensor_count: 16,
        sampled_at_ms: None,
        kind: CpuTemperatureKind::CoreAverage,
        source: CpuTemperatureSource::AppleSmc,
    };
    cache.cpu_details(details(Some(Ok(value()))), 1000);
    cache.fail(MetricId::Cpu, MetricStatus::Failed);
    assert_eq!(
        cache.snapshot(1000).cpu_temperature.status,
        MetricStatus::Ready
    );
    cache.cpu_details(details(None), 5000);
    assert_eq!(
        cache.snapshot(5000).cpu_temperature.sampled_at_ms,
        Some(1000)
    );
    assert_eq!(
        cache.snapshot(11001).cpu_temperature.status,
        MetricStatus::Stale
    );
    cache.cpu_details(
        details(Some(Err(PlatformError::new(
            PlatformErrorCode::OperationFailed,
            "sensor disconnected",
        )))),
        12000,
    );
    let failed = cache.snapshot(12000);
    assert_eq!(failed.cpu_temperature.status, MetricStatus::Failed);
    assert_eq!(failed.cpu_temperature.value.unwrap().celsius, 48.5);
    assert_eq!(failed.cpu_temperature_history.len(), 1);
    cache.cpu_details(details(Some(Ok(value()))), 13000);
    let mut changed = value();
    changed.sensor_count = 8;
    cache.cpu_details(details(Some(Ok(changed))), 17000);
    assert_eq!(cache.snapshot(17000).cpu_temperature_history.len(), 1);
    let mut invalid = value();
    invalid.celsius = f64::NAN;
    cache.cpu_details(details(Some(Ok(invalid))), 18000);
    assert_eq!(
        cache.snapshot(18000).cpu_temperature.status,
        MetricStatus::Failed
    );
    cache.cpu_details(
        details(Some(Err(PlatformError::new(
            PlatformErrorCode::Unsupported,
            "no CPU sensor",
        )))),
        19000,
    );
    assert_eq!(
        cache.snapshot(19000).cpu_temperature.status,
        MetricStatus::Unsupported
    );
    cache.reset(MetricId::Cpu);
    assert!(cache.snapshot(20000).cpu_temperature.value.is_none());
}

#[test]
fn temperature_recovery_keeps_real_gaps_and_bounded_diagnostic_retention() {
    use mangodisk_platform::system_resources::cpu::temperature::{
        CpuTemperatureKind, CpuTemperatureSource,
    };
    let details = |celsius| CpuDetails {
        identity: CpuIdentity::default(),
        frequency: None,
        temperature: Some(Ok(CpuTemperature {
            celsius,
            sensor_count: 1,
            sampled_at_ms: None,
            kind: CpuTemperatureKind::CoreAverage,
            source: CpuTemperatureSource::AppleSmc,
        })),
    };
    let mut cache = ResourceCache::default();
    cache.cpu_details(details(40.0), 1000);
    cache.cpu_details(
        CpuDetails {
            identity: CpuIdentity::default(),
            frequency: None,
            temperature: Some(Err(mangodisk_platform::PlatformError::new(
                mangodisk_platform::PlatformErrorCode::Io,
                "temporary IO failure",
            ))),
        },
        5000,
    );
    cache.cpu_details(details(42.0), 9000);
    let reading = cache.snapshot(9000);
    assert_eq!(reading.cpu_temperature_history.len(), 2);
    assert!(reading.cpu_temperature_history[1].gap_before);
    cache.cpu_details(
        CpuDetails {
            identity: CpuIdentity::default(),
            frequency: None,
            temperature: Some(Err(mangodisk_platform::PlatformError::new(
                mangodisk_platform::PlatformErrorCode::Io,
                "persistent IO failure",
            ))),
        },
        13000,
    );
    assert!(cache.snapshot(39000).cpu_temperature.value.is_some());
    assert!(cache.expire(39001), "clearing an aged diagnostic value must trigger publication even if its status is unchanged");
    assert!(cache.snapshot(39001).cpu_temperature.value.is_none());
    assert_eq!(cache.snapshot(39001).cpu_temperature_history.len(), 2);
}

#[test]
fn frequency_baselines_preserve_sample_age_and_failures_never_show_a_clock() {
    use mangodisk_platform::system_resources::cpu::details::CpuFrequencySource;
    use mangodisk_platform::{PlatformError, PlatformErrorCode};
    let mut cache = ResourceCache::default();
    let detail = |frequency| CpuDetails {
        identity: CpuIdentity {
            model: Some("CPU model".into()),
            nominal_frequency_mhz: Some(3700.0),
            frequency_classes: vec![
                mangodisk_platform::system_resources::cpu::details::CpuFrequencyClass::Average,
            ],
        },
        frequency,
        temperature: None,
    };
    cache.cpu_details(
        detail(Some(Ok(Some(CpuFrequency {
            average_mhz: Some(4773.0),
            efficiency_mhz: None,
            performance_mhz: None,
            super_mhz: None,
            source: CpuFrequencySource::WindowsPerformance,
            activity: Vec::new(),
        })))),
        1000,
    );
    cache.cpu_details(detail(Some(Ok(None))), 2000);
    let readings = cache.snapshot(2000);
    assert_eq!(readings.cpu_frequency.sampled_at_ms, Some(1000));
    assert_eq!(
        readings.cpu_frequency.value.unwrap().average_mhz,
        Some(4773.0)
    );
    let aged = 1000 + MetricId::Cpu.freshness_ms() + 1;
    cache.cpu_details(detail(None), aged);
    assert_eq!(
        cache.snapshot(aged).cpu_frequency.status,
        MetricStatus::Stale
    );
    cache.cpu_details(
        detail(Some(Err(PlatformError::new(
            PlatformErrorCode::Unsupported,
            "no counter",
        )))),
        aged + 1,
    );
    let readings = cache.snapshot(aged + 1);
    assert_eq!(readings.cpu_frequency.status, MetricStatus::Unsupported);
    assert!(readings.cpu_frequency.value.is_none());
    cache.reset(MetricId::Cpu);
    assert_eq!(
        cache
            .snapshot(aged + 1)
            .cpu_identity
            .unwrap()
            .model
            .as_deref(),
        Some("CPU model")
    );
}

#[test]
fn completed_idle_frequency_interval_clears_previous_clocks_and_recovers() {
    use mangodisk_platform::system_resources::cpu::details::CpuFrequencySource;
    let mut cache = ResourceCache::default();
    for (time, mhz) in [(1000, Some(2000.0)), (2000, None), (3000, Some(2500.0))] {
        cache.cpu_details(
            CpuDetails {
                identity: CpuIdentity::default(),
                frequency: Some(Ok(Some(CpuFrequency {
                    average_mhz: None,
                    efficiency_mhz: mhz,
                    performance_mhz: mhz,
                    super_mhz: None,
                    source: CpuFrequencySource::ApplePerformanceStates,
                    activity: Vec::new(),
                }))),
                temperature: None,
            },
            time,
        );
        let reading = cache.snapshot(time).cpu_frequency;
        assert_eq!(reading.status, MetricStatus::Ready);
        assert_eq!(reading.sampled_at_ms, Some(time));
        let value = reading.value.unwrap();
        assert_eq!(value.efficiency_mhz, mhz);
        assert_eq!(value.performance_mhz, mhz);
    }
}

#[test]
fn disconnected_gpu_selection_preserves_catalogue_without_an_idle_sample() {
    use mangodisk_platform::system_resources::gpu::{GpuAdapter, GpuAdapterUsage, GpuSample};
    let mut cache = ResourceCache::default();
    cache.gpu_adapters(vec![GpuAdapter {
        id: "b".into(),
        name: "GPU B".into(),
    }]);
    cache.gpu(
        GpuSample::Usage(vec![GpuAdapterUsage {
            id: "b".into(),
            name: "GPU B".into(),
            used_percent: 56.0,
            details: None,
        }]),
        Some("a"),
        1000,
    );
    let result = cache.snapshot(1000);
    assert_eq!(result.gpu.status, MetricStatus::Disconnected);
    assert!(result.gpu.value.is_none());
    assert!(result.gpu_history.is_empty());
    assert_eq!(result.gpu_adapters.len(), 1);
}
#[test]
fn gpu_baselines_and_failures_preserve_only_real_history_and_original_freshness() {
    use mangodisk_platform::system_resources::gpu::{GpuAdapterUsage, GpuSample};
    let mut cache = ResourceCache::default();
    cache.gpu(GpuSample::Baseline, None, 1000);
    assert!(cache.snapshot(1000).gpu.value.is_none());
    cache.gpu(
        GpuSample::Usage(vec![GpuAdapterUsage {
            id: "gpu-1".into(),
            name: "GPU 1".into(),
            used_percent: 0.0,
            details: None,
        }]),
        None,
        2000,
    );
    cache.gpu(GpuSample::Baseline, None, 3000);
    let valid = cache.snapshot(3000);
    assert_eq!(valid.gpu.sampled_at_ms, Some(2000));
    assert_eq!(valid.gpu_history.len(), 1);
    assert_eq!(valid.gpu.status, MetricStatus::Ready);
    assert_eq!(cache.snapshot(7001).gpu.status, MetricStatus::Stale);
    cache.fail(MetricId::Gpu, MetricStatus::Unsupported);
    assert_eq!(cache.snapshot(8000).gpu.status, MetricStatus::Unsupported);
    assert_eq!(cache.snapshot(8000).gpu_history.len(), 1);
    cache.reset(MetricId::Gpu);
    assert!(cache.snapshot(8000).gpu.value.is_none());
    assert!(cache.snapshot(8000).gpu_history.is_empty());
}

#[test]
fn automatic_gpu_switches_restore_each_devices_observed_history() {
    use mangodisk_platform::system_resources::gpu::{
        temperature::{
            GpuTemperature, GpuTemperatureKind, GpuTemperatureSample, GpuTemperatureSource,
        },
        GpuAdapterUsage, GpuSample,
    };
    let mut cache = ResourceCache::default();
    let usage = |amd, intel| {
        GpuSample::Usage(
            [("amd", amd), ("intel", intel)]
                .into_iter()
                .map(|(id, used_percent)| GpuAdapterUsage {
                    id: id.into(),
                    name: id.into(),
                    used_percent,
                    details: None,
                })
                .collect(),
        )
    };
    let temperatures = |celsius| {
        Ok(vec![
            GpuTemperatureSample {
                adapter_id: "amd".into(),
                reading: Ok(GpuTemperature {
                    adapter_id: "amd".into(),
                    adapter_name: "AMD".into(),
                    celsius,
                    kind: GpuTemperatureKind::Device,
                    source: GpuTemperatureSource::Driver,
                    sensor_count: 1,
                }),
            },
            GpuTemperatureSample {
                adapter_id: "intel".into(),
                reading: Err(mangodisk_platform::PlatformError::new(
                    mangodisk_platform::PlatformErrorCode::Unsupported,
                    "temperature unavailable",
                )),
            },
        ])
    };
    cache.gpu(usage(30.0, 5.0), None, 1000);
    cache.gpu_temperatures(temperatures(53.0), 1000);
    cache.gpu(usage(5.0, 40.0), None, 3000);
    assert_eq!(cache.snapshot(3000).gpu.value.unwrap().adapter_id, "intel");
    cache.gpu(usage(6.0, 50.0), None, 5000);
    cache.gpu_temperatures(temperatures(54.0), 5000);
    cache.gpu(usage(35.0, 4.0), None, 7000);
    let restored = cache.snapshot(7000);
    assert_eq!(restored.gpu.value.unwrap().adapter_id, "amd");
    assert_eq!(
        restored
            .gpu_detail_history
            .iter()
            .map(|p| p.primary)
            .collect::<Vec<_>>(),
        vec![30.0, 5.0, 6.0, 35.0]
    );
    assert_eq!(
        restored
            .gpu_temperature_history
            .iter()
            .map(|p| p.sampled_at_ms)
            .collect::<Vec<_>>(),
        vec![1000, 5000]
    );
    assert_eq!(restored.gpu_temperature.sampled_at_ms, Some(5000));
    assert_eq!(restored.gpu_temperature.value.unwrap().celsius, 54.0);
}

#[test]
fn gpu_selection_resets_restore_only_the_selected_devices_history_until_removal() {
    use mangodisk_platform::system_resources::gpu::{GpuAdapter, GpuAdapterUsage, GpuSample};
    let mut cache = ResourceCache::default();
    let catalogue = || {
        vec![
            GpuAdapter {
                id: "a".into(),
                name: "A".into(),
            },
            GpuAdapter {
                id: "b".into(),
                name: "B".into(),
            },
        ]
    };
    let sample = || {
        GpuSample::Usage(vec![
            GpuAdapterUsage {
                id: "a".into(),
                name: "A".into(),
                used_percent: 10.0,
                details: None,
            },
            GpuAdapterUsage {
                id: "b".into(),
                name: "B".into(),
                used_percent: 80.0,
                details: None,
            },
        ])
    };
    cache.gpu_adapters(catalogue());
    cache.gpu(sample(), Some("a"), 1000);
    cache.reset(MetricId::Gpu);
    assert!(cache.snapshot(2000).gpu_detail_history.is_empty());
    cache.gpu(sample(), Some("b"), 3000);
    let b = cache.snapshot(3000);
    assert_eq!(b.gpu_detail_history.len(), 2);
    assert!(b
        .gpu_detail_history
        .iter()
        .all(|point| point.primary == 80.0));
    cache.reset(MetricId::Gpu);
    cache.gpu(sample(), Some("a"), 19000);
    let a = cache.snapshot(19000);
    assert_eq!(
        a.gpu_detail_history
            .iter()
            .map(|point| point.sampled_at_ms)
            .collect::<Vec<_>>(),
        vec![1000, 3000, 19000]
    );
    assert!(a
        .gpu_detail_history
        .iter()
        .all(|point| point.primary == 10.0));
    cache.gpu_adapters(vec![GpuAdapter {
        id: "b".into(),
        name: "B".into(),
    }]);
    assert_eq!(cache.gpu_device_histories.len(), 1);
    assert!(cache.snapshot(19000).gpu_detail_history.is_empty());
    cache.gpu_adapters(catalogue());
    cache.gpu(sample(), Some("a"), 21000);
    assert_eq!(cache.snapshot(21000).gpu_detail_history.len(), 1);
    cache.gpu_adapters(vec![GpuAdapter {
        id: "a".into(),
        name: "Replacement".into(),
    }]);
    cache.gpu(
        GpuSample::Usage(vec![GpuAdapterUsage {
            id: "a".into(),
            name: "Replacement".into(),
            used_percent: 20.0,
            details: None,
        }]),
        Some("a"),
        23000,
    );
    let replaced = cache.snapshot(23000);
    assert_eq!(replaced.gpu_detail_history.len(), 1);
    assert_eq!(replaced.gpu_detail_history[0].primary, 20.0);
}

#[test]
fn gpu_catalogue_renumbering_preserves_the_surviving_devices_histories() {
    use mangodisk_platform::system_resources::gpu::{
        temperature::{
            GpuTemperature, GpuTemperatureKind, GpuTemperatureSample, GpuTemperatureSource,
        },
        GpuAdapter, GpuAdapterUsage, GpuSample,
    };
    let mut cache = ResourceCache::default();
    cache.gpu_adapters(vec![
        GpuAdapter {
            id: "a".into(),
            name: "GPU 0 · A".into(),
        },
        GpuAdapter {
            id: "b".into(),
            name: "GPU 1 · B".into(),
        },
    ]);
    for timestamp in [1000, 5000] {
        cache.gpu(
            GpuSample::Usage(vec![GpuAdapterUsage {
                id: "b".into(),
                name: "B".into(),
                used_percent: 80.0,
                details: None,
            }]),
            Some("b"),
            timestamp,
        );
        cache.gpu_temperatures(
            Ok(vec![GpuTemperatureSample {
                adapter_id: "b".into(),
                reading: Ok(GpuTemperature {
                    adapter_id: "b".into(),
                    adapter_name: "B".into(),
                    celsius: 70.0,
                    kind: GpuTemperatureKind::Device,
                    source: GpuTemperatureSource::Driver,
                    sensor_count: 1,
                }),
            }]),
            timestamp,
        );
    }
    cache.gpu_adapters(vec![GpuAdapter {
        id: "b".into(),
        name: "GPU 0 · B".into(),
    }]);
    let reading = cache.snapshot(6000);
    assert_eq!(reading.gpu_detail_history.len(), 2);
    assert_eq!(reading.gpu_temperature_history.len(), 2);
    assert_eq!(reading.gpu_temperature.sampled_at_ms, Some(5000));
    assert_eq!(reading.gpu_temperature.status, MetricStatus::Ready);
}

#[test]
fn replacement_temperature_observed_before_utilization_recovery_stays_available() {
    use mangodisk_platform::system_resources::gpu::{
        temperature::{
            GpuTemperature, GpuTemperatureKind, GpuTemperatureSample, GpuTemperatureSource,
        },
        GpuAdapterUsage, GpuSample,
    };
    let mut cache = ResourceCache::default();
    let usage = |name: &str| {
        GpuSample::Usage(vec![GpuAdapterUsage {
            id: "a".into(),
            name: name.into(),
            used_percent: 10.0,
            details: None,
        }])
    };
    let temperature = |name: &str| {
        Ok(vec![GpuTemperatureSample {
            adapter_id: "a".into(),
            reading: Ok(GpuTemperature {
                adapter_id: "a".into(),
                adapter_name: name.into(),
                celsius: 51.0,
                kind: GpuTemperatureKind::Device,
                source: GpuTemperatureSource::Driver,
                sensor_count: 1,
            }),
        }])
    };
    cache.gpu(usage("Original"), Some("a"), 1000);
    cache.gpu_temperatures(temperature("Original"), 1000);
    cache.gpu(GpuSample::Baseline, Some("a"), 5000);
    cache.gpu_temperatures(temperature("Replacement"), 5000);
    cache.gpu(usage("Replacement"), Some("a"), 7000);
    let recovered = cache.snapshot(7000);
    assert_eq!(recovered.gpu_temperature.status, MetricStatus::Ready);
    assert_eq!(recovered.gpu_temperature.sampled_at_ms, Some(5000));
    assert_eq!(recovered.gpu_temperature_history.len(), 1);
    assert_eq!(recovered.gpu_detail_history.len(), 1);
}

#[test]
fn detail_history_isolated_when_automatic_selection_changes_devices() {
    use mangodisk_platform::system_resources::gpu::{details::*, GpuAdapterUsage, GpuSample};
    let sample = |a: f64, b: f64, detailed: bool| {
        GpuSample::Usage(
            [("a", a), ("b", b)]
                .into_iter()
                .map(|(id, used_percent)| GpuAdapterUsage {
                    id: id.into(),
                    name: id.into(),
                    used_percent,
                    details: detailed.then_some(GpuDetails {
                        activities: vec![],
                        telemetry: GpuTelemetry::default(),
                        memory_architecture: GpuMemoryArchitecture::Unified,
                        memory_status: GpuMemoryStatus::Unsupported,
                        memory: None,
                    }),
                })
                .collect(),
        )
    };
    let mut cache = ResourceCache::default();
    cache.gpu(sample(30.0, 5.0, true), None, 1000);
    cache.gpu(sample(35.0, 5.0, true), None, 3000);
    assert_eq!(cache.snapshot(3000).gpu_detail_history.len(), 2);
    cache.gpu(sample(3.0, 56.0, true), None, 5000);
    let switched = cache.snapshot(5000);
    assert_eq!(switched.gpu_history.len(), 3);
    assert_eq!(switched.gpu_detail_history.len(), 3);
    assert_eq!(switched.gpu_details.value.as_ref().unwrap().adapter_id, "b");
    assert!(switched.gpu.value.as_ref().unwrap().details.is_none());
    cache.gpu(sample(5.0, 60.0, false), None, 7000);
    assert_eq!(cache.snapshot(7000).gpu_details.sampled_at_ms, Some(5000));
    assert_eq!(
        cache.snapshot(10001).gpu_details.status,
        MetricStatus::Stale
    );
    cache.fail(MetricId::Gpu, MetricStatus::Disconnected);
    assert_eq!(
        cache.snapshot(11000).gpu_details.status,
        MetricStatus::Disconnected
    );
    cache.reset(MetricId::Gpu);
    assert!(cache.snapshot(11000).gpu_detail_history.is_empty());
    assert!(cache.snapshot(11000).gpu_details.value.is_none());
}

#[test]
fn gpu_background_samples_keep_the_device_curve_continuous_without_fabricating_engines() {
    use mangodisk_platform::system_resources::gpu::{details::*, GpuAdapterUsage, GpuSample};
    let sample = |id: &str, detailed: bool| {
        GpuSample::Usage(vec![GpuAdapterUsage {
            id: id.into(),
            name: id.into(),
            used_percent: 20.0,
            details: detailed.then_some(GpuDetails {
                activities: vec![GpuActivity {
                    id: "renderer".into(),
                    kind: GpuActivityKind::Renderer,
                    name: None,
                    used_percent: 12.0,
                    included_in_summary: false,
                }],
                telemetry: GpuTelemetry::default(),
                memory_architecture: GpuMemoryArchitecture::Unified,
                memory_status: GpuMemoryStatus::Unsupported,
                memory: None,
            }),
        }])
    };
    let mut cache = ResourceCache::default();
    for time in (1000..=11000).step_by(2000) {
        cache.gpu(sample("a", time == 1000 || time == 11000), None, time);
    }
    let reopened = cache.snapshot(11000);
    assert_eq!(reopened.gpu_detail_adapter_id.as_deref(), Some("a"));
    assert_eq!(reopened.gpu_detail_history.len(), 6);
    assert!(reopened
        .gpu_detail_history
        .windows(2)
        .all(|pair| pair[1].sampled_at_ms - pair[0].sampled_at_ms == 2000));
    assert_eq!(reopened.gpu_renderer_history.len(), 2);
    assert!(reopened.gpu_tiler_history.is_empty());
    for time in (13000..=201000).step_by(2000) {
        cache.gpu(sample("a", true), None, time);
    }
    let retained = cache.snapshot(201000);
    assert!(retained.gpu_renderer_history.len() <= 96);
    assert!(retained
        .gpu_renderer_history
        .iter()
        .all(|point| point.sampled_at_ms >= 121000));
    cache.gpu(sample("b", false), None, 203000);
    let switched = cache.snapshot(203000);
    assert_eq!(switched.gpu_detail_adapter_id.as_deref(), Some("b"));
    assert_eq!(switched.gpu_detail_history.len(), 1);
    assert!(switched.gpu_renderer_history.is_empty());
    cache.reset(MetricId::Gpu);
    assert!(cache.snapshot(203000).gpu_detail_adapter_id.is_none());
}

#[test]
fn snapshots_share_immutable_rankings_and_keep_the_wire_shape() {
    let mut cache = ResourceCache::default();
    cache.cpu_processes(MetricReading::ready(
        ProcessCpuSummary {
            usage_scale:
                mangodisk_platform::system_resources::process_cpu::CpuUsageScale::TotalCapacity,
            applications: vec![],
            readable_process_count: 12,
            omitted_process_count: 1,
        },
        1000,
    ));
    let first = cache.snapshot(1000);
    let second = cache.snapshot(2000);
    assert!(Arc::ptr_eq(
        first.cpu_processes.value.as_ref().unwrap(),
        second.cpu_processes.value.as_ref().unwrap()
    ));
    let wire = serde_json::to_value(&second).unwrap();
    assert_eq!(wire["schemaVersion"], 39);
    assert_eq!(wire["cpuProcesses"]["value"]["readableProcessCount"], 12);
    assert!(!cache.expire(2000));
    assert!(cache.expire(6001));
    assert!(!cache.expire(6002));
}

#[test]
fn memory_overview_updates_preserve_detail_timestamp_and_empty_samples_clear_rows() {
    use super::super::models::{MemoryOverview, ProcessMemorySummary};
    let mut cache = ResourceCache::default();
    let sample = |at, processes| SystemResourceSnapshot {
        schema_version: 5,
        sampled_at_ms: at,
        memory: MemoryOverview {
            total_bytes: 100,
            used_bytes: 40,
            free_bytes: 60,
            swap_used_bytes: 0,
            pressure: mangodisk_platform::system_resources::memory::MemoryPressure::Unsupported,
            used_percent: 40,
        },
        processes,
    };
    let details = ProcessMemorySummary {
        usage_kind: mangodisk_platform::system_resources::memory::ProcessMemoryKind::native(),
        applications: vec![],
        readable_process_count: 2,
        omitted_process_count: 0,
    };
    cache.memory(sample(1000, Some(details)));
    cache.memory(sample(12000, None));
    let reading = cache.snapshot(12000);
    assert_eq!(reading.memory.status, MetricStatus::Ready);
    assert_eq!(reading.memory_processes.sampled_at_ms, Some(1000));
    assert_eq!(reading.memory_processes.status, MetricStatus::Stale);
    assert_eq!(
        reading
            .memory_processes
            .value
            .unwrap()
            .readable_process_count,
        2
    );
    cache.memory(sample(
        13000,
        Some(ProcessMemorySummary {
            usage_kind: mangodisk_platform::system_resources::memory::ProcessMemoryKind::native(),
            applications: vec![],
            readable_process_count: 0,
            omitted_process_count: 0,
        }),
    ));
    assert_eq!(
        cache
            .snapshot(13000)
            .memory_processes
            .value
            .unwrap()
            .readable_process_count,
        0
    );
    cache.fail(MetricId::Memory, MetricStatus::Failed);
    assert_eq!(
        cache.snapshot(14000).memory_processes.status,
        MetricStatus::Failed
    );
    cache.suspend(MetricId::Memory);
    assert!(cache.snapshot(14000).memory_processes.value.is_none());
}

#[test]
fn cpu_ranking_survives_baseline_and_failure_with_original_sample_time() {
    let mut cache = ResourceCache::default();
    let summary = ProcessCpuSummary {
        usage_scale:
            mangodisk_platform::system_resources::process_cpu::CpuUsageScale::TotalCapacity,
        applications: vec![],
        readable_process_count: 12,
        omitted_process_count: 1,
    };
    cache.cpu_processes(MetricReading::ready(summary.clone(), 1000));
    cache.cpu_processes(MetricReading::default());
    let warm = cache.snapshot(2000).cpu_processes;
    assert_eq!(warm.status, MetricStatus::Ready);
    assert_eq!(warm.sampled_at_ms, Some(1000));
    assert_eq!(warm.value.unwrap().readable_process_count, 12);
    assert_eq!(
        cache.snapshot(6001).cpu_processes.status,
        MetricStatus::Stale
    );
    cache.cpu_processes(MetricReading {
        status: MetricStatus::Failed,
        ..Default::default()
    });
    let failed = cache.snapshot(7000).cpu_processes;
    assert_eq!(failed.status, MetricStatus::Failed);
    assert_eq!(failed.sampled_at_ms, Some(1000));
    assert!(failed.value.is_some());
    cache.cpu_processes(MetricReading::ready(summary, 8000));
    assert_eq!(
        cache.snapshot(8000).cpu_processes.status,
        MetricStatus::Ready
    );
    cache.clear_cpu_processes();
    assert!(cache.snapshot(8001).cpu_processes.value.is_none());
}

#[test]
fn application_cpu_readings_expire_and_clear_without_erasing_overview_history() {
    let mut cache = ResourceCache::default();
    cache.cpu_processes(MetricReading::ready(
        ProcessCpuSummary {
            usage_scale:
                mangodisk_platform::system_resources::process_cpu::CpuUsageScale::TotalCapacity,
            applications: vec![],
            readable_process_count: 0,
            omitted_process_count: 2,
        },
        1000,
    ));
    assert_eq!(cache.snapshot(1000).schema_version, 39);
    assert_eq!(
        cache.snapshot(6001).cpu_processes.status,
        MetricStatus::Stale
    );
    cache.clear_cpu_processes();
    assert_eq!(
        cache.snapshot(7000).cpu_processes.status,
        MetricStatus::Loading
    );
    assert!(cache.snapshot(7000).cpu_processes.value.is_none());
}

#[test]
fn native_cpu_baselines_do_not_refresh_old_values() {
    let mut cache = ResourceCache::default();
    let valid = CpuSample::Percent {
        used: 7.0,
        interval_ms: 1000,
    };
    assert_eq!(cache.cpu(valid, 1000, 1000), None);
    assert_eq!(
        cache.cpu(CpuSample::Baseline, 2000, 2000),
        Some(CpuBaselineReason::FirstSample)
    );
    assert_eq!(cache.readings.cpu.sampled_at_ms, Some(1000));
    assert_eq!(cache.readings.cpu.value.as_ref().unwrap().used_percent, 7.0);
    let _ = cache.cpu(CpuSample::Baseline, 7000, 7000);
    assert_eq!(cache.readings.cpu.status, MetricStatus::Stale);
    assert_eq!(cache.readings.cpu.sampled_at_ms, Some(1000));
    assert_eq!(cache.cpu(valid, 8000, 8000), None);
    assert_eq!(cache.readings.cpu.status, MetricStatus::Ready);
}

#[test]
fn unavailable_manual_network_uses_live_fallback_without_changing_preference() {
    use super::*;
    use mangodisk_platform::system_resources::network::{InterfaceKind, NetworkCounters};
    let sample = |received| {
        vec![InterfaceSample {
            interface: NetworkInterface {
                details: None,
                id: "wifi".into(),
                name: "Wi-Fi".into(),
                kind: InterfaceKind::Wifi,
                connected: true,
                physical: true,
                default_route_metric: Some(1),
            },
            counters: Some(NetworkCounters {
                received,
                transmitted: 0,
                ..Default::default()
            }),
        }]
    };
    let mut cache = ResourceCache::default();
    cache.network(sample(100), Some("removed-lan"), 0, 0);
    cache.network(sample(1124), Some("removed-lan"), 1000, 1000);
    let snapshot = cache.snapshot(1000);
    assert_eq!(snapshot.network.status, MetricStatus::Ready);
    let value = snapshot.network.value.unwrap();
    assert_eq!(value.interface.id, "wifi");
    assert_eq!(value.received_bytes_per_second, 1024.0);
    assert_eq!(value.selection_reason, NetworkSelectionReason::DefaultRoute);
}

#[test]
fn network_suspend_preserves_history_but_switching_interfaces_clears_it() {
    use mangodisk_platform::system_resources::network::{InterfaceKind, NetworkCounters};
    let sample = |id: &str, received| {
        vec![InterfaceSample {
            interface: NetworkInterface {
                details: None,
                id: id.into(),
                name: "test".into(),
                kind: InterfaceKind::Ethernet,
                connected: true,
                physical: true,
                default_route_metric: Some(1),
            },
            counters: Some(NetworkCounters {
                received,
                transmitted: 0,
                ..Default::default()
            }),
        }]
    };
    let mut cache = ResourceCache::default();
    cache.network(sample("one", 0), None, 0, 0);
    cache.network(sample("one", 1000), None, 1000, 1000);
    cache.suspend(MetricId::Network);
    cache.network(sample("one", 2000), None, 2000, 2000);
    assert_eq!(cache.snapshot(2000).network_history.len(), 1);
    assert_eq!(cache.snapshot(2000).network.sampled_at_ms, Some(1000));
    cache.network(sample("two", 5000), None, 3000, 3000);
    assert_eq!(
        cache.snapshot(3000).network.status,
        MetricStatus::Disconnected
    );
    cache.network(sample("two", 6000), None, 4000, 4000);
    assert!(cache.snapshot(4000).network_history.is_empty());
    assert_eq!(cache.snapshot(4000).network.status, MetricStatus::Loading);
    cache.network(sample("two", 6100), None, 5000, 5000);
    assert_eq!(cache.snapshot(5000).network_history.len(), 1);
}

#[test]
fn disk_activity_preserves_real_history_while_rebuilding_a_baseline() {
    use mangodisk_platform::system_resources::disk_io::DeviceCounters;
    let mut cache = ResourceCache::default();
    let sample = |value| {
        vec![DeviceCounters {
            id: "device".into(),
            read_bytes: value,
            written_bytes: value,
        }]
    };
    cache.disk_io(sample(0), 0, 0);
    assert_eq!(cache.snapshot(0).disk_io.status, MetricStatus::Loading);
    cache.disk_io(sample(2000), 2000, 2000);
    let snapshot = cache.snapshot(2000);
    assert_eq!(
        snapshot.disk_io.value.unwrap().read_bytes_per_second,
        1000.0
    );
    assert_eq!(snapshot.disk.status, MetricStatus::Loading);
    assert_eq!(snapshot.disk_io_history.len(), 1);
    cache.fail_disk_io(MetricStatus::Unsupported);
    assert_eq!(
        cache.snapshot(3000).disk_io.status,
        MetricStatus::Unsupported
    );
    cache.disk_io(sample(4000), 4000, 4000);
    assert_eq!(cache.snapshot(4000).disk_io_history.len(), 1);
    cache.disk_io(sample(6000), 6000, 6000);
    assert_eq!(cache.snapshot(11001).disk_io.status, MetricStatus::Stale);
    cache.stop_disk_io();
    cache.disk_io(sample(8000), 8000, 8000);
    assert_eq!(cache.snapshot(12000).disk_io.status, MetricStatus::Stale);
}

#[test]
fn memory_history_contains_occupancy_and_clears_on_demand_reset() {
    let mut cache = ResourceCache::default();
    for time in [0, 3000, 6000] {
        cache.memory(SystemResourceSnapshot {
            schema_version: 5,
            sampled_at_ms: time,
            memory: super::super::models::MemoryOverview {
                total_bytes: 100,
                used_bytes: 80,
                free_bytes: 20,
                swap_used_bytes: 5,
                pressure: mangodisk_platform::system_resources::memory::MemoryPressure::Unsupported,
                used_percent: 80,
            },
            processes: None,
        });
    }
    assert_eq!(cache.snapshot(6000).memory_history.len(), 3);
    assert_eq!(cache.snapshot(6000).memory_history[0].primary, 80.0);
    cache.reset(MetricId::Memory);
    assert!(cache.snapshot(6000).memory_history.is_empty());
}

#[test]
fn cpu_recovery_and_panel_suspend_preserve_real_readings_until_expiry() {
    let mut cache = ResourceCache::default();
    let first = CpuCounters { busy: 10, idle: 90 };
    assert_eq!(cache.cpu(first, 0, 0), Some(CpuBaselineReason::FirstSample));
    let next = CpuCounters {
        busy: 20,
        idle: 180,
    };
    assert_eq!(cache.cpu(next, 2000, 2000), None);
    assert_eq!(
        cache.cpu(next, 4000, 4000),
        Some(CpuBaselineReason::NoProgress)
    );
    let reading = cache.snapshot(4000);
    assert_eq!(reading.cpu.status, MetricStatus::Ready);
    assert_eq!(reading.cpu.sampled_at_ms, Some(2000));
    assert_eq!(reading.cpu_history.len(), 1);
    cache.suspend(MetricId::Cpu);
    assert_eq!(
        cache.cpu(next, 4200, 4200),
        Some(CpuBaselineReason::FirstSample)
    );
    assert_eq!(cache.snapshot(4200).cpu.status, MetricStatus::Ready);
    assert_eq!(cache.snapshot(7001).cpu.status, MetricStatus::Stale);
    assert_eq!(cache.snapshot(7001).cpu_history.len(), 1);
    assert_eq!(
        cache.cpu(
            CpuCounters {
                busy: 30,
                idle: 270
            },
            8000,
            8000
        ),
        None
    );
    assert_eq!(cache.snapshot(8000).cpu.status, MetricStatus::Ready);
}

#[test]
fn one_metric_failure_preserves_other_values_and_expires_them_independently() {
    let mut cache = ResourceCache::default();
    let _ = cache.cpu(CpuCounters { busy: 0, idle: 0 }, 0, 0);
    let _ = cache.cpu(CpuCounters { busy: 10, idle: 90 }, 2000, 2000);
    cache.fail(MetricId::Network, MetricStatus::Failed);
    let values = cache.snapshot(2000);
    assert_eq!(values.cpu.status, MetricStatus::Ready);
    assert_eq!(values.network.status, MetricStatus::Failed);
    assert_eq!(values.cpu.value.unwrap().used_percent, 10.0);
    assert_eq!(cache.snapshot(7001).cpu.status, MetricStatus::Stale);
    cache.reset(MetricId::Cpu);
    let resumed = cache.snapshot(8000);
    assert_eq!(resumed.cpu.status, MetricStatus::Loading);
    assert!(resumed.cpu_history.is_empty());
    assert_eq!(resumed.network.status, MetricStatus::Failed);
}

#[test]
fn network_fallback_and_restore_keep_actual_identity_totals_and_history_separate() {
    use super::*;
    use mangodisk_platform::system_resources::network::{InterfaceKind, NetworkCounters};
    let directory = tempfile::tempdir().unwrap();
    let traffic = Arc::new(
        super::super::network_traffic::NetworkTraffic::open(directory.path().join("traffic.json"))
            .unwrap(),
    );
    let mut cache = ResourceCache::default();
    cache.set_network_traffic(traffic.clone());
    let rows = |lan_connected, wifi_connected, time: u64| {
        [
            ("lan", lan_connected, 1, 1000, 2),
            ("wifi", wifi_connected, 2, 100_000, 5),
        ]
        .into_iter()
        .map(
            |(id, connected, metric, base, multiplier)| InterfaceSample {
                interface: NetworkInterface {
                    details: None,
                    id: id.into(),
                    name: id.into(),
                    kind: InterfaceKind::Ethernet,
                    physical: true,
                    connected,
                    default_route_metric: Some(metric),
                },
                counters: Some(NetworkCounters {
                    received: base + time * multiplier,
                    transmitted: 0,
                    ..Default::default()
                }),
            },
        )
        .collect()
    };
    cache.network(rows(true, true, 0), Some("lan"), 0, 1);
    cache.network(rows(true, true, 1000), Some("lan"), 1000, 1000);
    assert_eq!(
        cache.snapshot(1000).network.value.unwrap().interface.id,
        "lan"
    );
    cache.network(rows(false, true, 2000), Some("lan"), 2000, 2000);
    assert_eq!(
        cache.snapshot(2000).network.status,
        MetricStatus::Disconnected
    );
    cache.network(rows(false, true, 3000), Some("lan"), 3000, 3000);
    assert_eq!(cache.snapshot(3000).network.status, MetricStatus::Loading);
    assert!(cache.snapshot(3000).network.value.is_none());
    assert!(cache.snapshot(3000).network_history.is_empty());
    cache.network(rows(false, true, 4000), Some("lan"), 4000, 4000);
    let snapshot = cache.snapshot(4000);
    let wifi = snapshot.network.value.unwrap();
    assert_eq!(wifi.interface.id, "wifi");
    assert_eq!(wifi.received_bytes_per_second, 5000.0);
    assert_eq!(wifi.received_bytes_since_start, 5000);
    assert_eq!(wifi.cumulative.unwrap().received_bytes, "5000");
    assert_eq!(snapshot.network_history.len(), 1);
    assert!(
        !snapshot
            .interfaces
            .iter()
            .find(|item| item.id == "lan")
            .unwrap()
            .connected
    );
    cache.network(rows(true, true, 5000), Some("lan"), 5000, 5000);
    assert_eq!(
        cache.snapshot(5000).network.value.unwrap().interface.id,
        "wifi"
    );
    cache.network(rows(true, true, 6000), Some("lan"), 6000, 6000);
    assert!(cache.snapshot(6000).network_history.is_empty());
    assert!(cache.snapshot(6000).network.value.is_none());
    cache.network(rows(true, true, 7000), Some("lan"), 7000, 7000);
    let restored = cache.snapshot(7000).network.value.unwrap();
    assert_eq!(restored.interface.id, "lan");
    assert_eq!(restored.selection_reason, NetworkSelectionReason::Manual);
    assert_eq!(restored.received_bytes_per_second, 2000.0);
    assert_eq!(restored.received_bytes_since_start, 2000);
    assert_eq!(restored.cumulative.unwrap().received_bytes, "4000");
    assert_eq!(traffic.total("wifi").unwrap().received_bytes, "10000");
    cache.network(rows(false, false, 8000), Some("lan"), 8000, 8000);
    assert_eq!(
        cache.snapshot(8000).network.status,
        MetricStatus::Disconnected
    );
    cache.network(rows(true, true, 9000), Some("lan"), 9000, 9000);
    cache.network(rows(true, true, 10000), Some("lan"), 10000, 10000);
    let snapshot = cache.snapshot(10000);
    assert!(snapshot.network_history.last().unwrap().gap_before);
    assert_eq!(
        snapshot
            .network
            .value
            .unwrap()
            .cumulative
            .unwrap()
            .received_bytes,
        "6000"
    );
    // An idle, connected manual link must not hand control to the busier fallback.
    cache.network(rows(true, true, 10000), Some("lan"), 11000, 11000);
    let idle = cache.snapshot(11000).network.value.unwrap();
    assert_eq!(idle.interface.id, "lan");
    assert_eq!(idle.received_bytes_per_second, 0.0);
}

#[test]
fn manual_network_recovery_is_not_blocked_by_fallback_counter_failure() {
    use super::*;
    use mangodisk_platform::system_resources::network::{InterfaceKind, NetworkCounters};
    let rows = |restored, time| {
        ["lan", "wifi"]
            .into_iter()
            .map(|id| InterfaceSample {
                interface: NetworkInterface {
                    details: None,
                    id: id.into(),
                    name: id.into(),
                    kind: InterfaceKind::Ethernet,
                    physical: true,
                    connected: id == "wifi" || restored,
                    default_route_metric: Some(1),
                },
                counters: (id == "lan" || !restored).then_some(NetworkCounters {
                    received: time,
                    transmitted: 0,
                    ..Default::default()
                }),
            })
            .collect()
    };
    let mut cache = ResourceCache::default();
    cache.network(rows(false, 0), Some("lan"), 0, 1);
    cache.network(rows(false, 1000), Some("lan"), 1000, 1000);
    assert_eq!(
        cache.snapshot(1000).network.value.unwrap().interface.id,
        "wifi"
    );
    cache.network(rows(true, 2000), Some("lan"), 2000, 2000);
    assert_eq!(cache.snapshot(2000).network.status, MetricStatus::Failed);
    cache.network(rows(true, 3000), Some("lan"), 3000, 3000);
    assert_eq!(cache.snapshot(3000).network.status, MetricStatus::Loading);
    cache.network(rows(true, 4000), Some("lan"), 4000, 4000);
    let snapshot = cache.snapshot(4000);
    assert_eq!(snapshot.network.status, MetricStatus::Ready);
    assert_eq!(snapshot.network.value.unwrap().interface.id, "lan");
}

#[test]
fn alternating_routes_after_disconnect_restore_live_rates_without_mixing_sources() {
    use mangodisk_platform::system_resources::network::{InterfaceKind, NetworkCounters};
    let directory = tempfile::tempdir().unwrap();
    let traffic = std::sync::Arc::new(
        super::super::network_traffic::NetworkTraffic::open(directory.path().join("traffic.json"))
            .unwrap(),
    );
    let mut cache = ResourceCache::default();
    cache.set_network_traffic(traffic.clone());
    for second in 0u64..=8 {
        let rows = ["manual", "wifi", "ethernet"]
            .into_iter()
            .enumerate()
            .map(|(index, id)| InterfaceSample {
                interface: NetworkInterface {
                    details: None,
                    id: id.into(),
                    name: id.into(),
                    kind: InterfaceKind::Ethernet,
                    physical: true,
                    connected: index != 0 || second == 0,
                    default_route_metric: Some(if index == 0 {
                        0
                    } else if second % 2 == index as u64 % 2 {
                        1
                    } else {
                        2
                    }),
                },
                counters: Some(NetworkCounters {
                    received: index as u64 * 100_000 + second * (index as u64 + 1) * 1000,
                    transmitted: 0,
                    ..Default::default()
                }),
            })
            .collect();
        cache.network(rows, Some("manual"), second * 1000, second * 1000 + 1);
        if second >= 3 {
            let snapshot = cache.snapshot(second * 1000 + 1);
            assert_eq!(snapshot.network.status, MetricStatus::Ready);
            let rate = snapshot.network.value.unwrap();
            assert_eq!(rate.interface.id, "wifi");
            assert_eq!(rate.received_bytes_per_second, 2000.0);
            assert_eq!(rate.received_bytes_since_start, (second - 2) * 2000);
            assert_eq!(snapshot.network_history.len(), (second - 2) as usize);
        }
    }
    assert!(traffic.total("manual").is_none());
    assert!(traffic.total("ethernet").is_none());
    assert_eq!(traffic.total("wifi").unwrap().received_bytes, "12000");
}

#[test]
#[ignore = "requires an active native network interface"]
fn native_network_fallback_reports_measured_counters_without_a_matching_preference() {
    use mangodisk_platform::system_resources::network::NetworkReader;
    let mut reader = NetworkReader::default();
    let mut cache = ResourceCache::default();
    let origin = std::time::Instant::now();
    let mut previous = None;
    let mut verified = 0;
    for sample in 0..3 {
        let rows = reader.read().expect("native network sampling must succeed");
        let time = origin.elapsed().as_millis() as u64;
        cache.network(
            rows.clone(),
            Some("native-test-missing-interface"),
            time,
            time + 1,
        );
        let snapshot = cache.snapshot(time + 1);
        if let Some(rate) = snapshot.network.value {
            assert_eq!(snapshot.network.status, MetricStatus::Ready);
            assert!(rate.interface.connected && rate.interface.physical);
            assert_ne!(rate.selection_reason, super::NetworkSelectionReason::Manual);
            let counters = rows
                .iter()
                .find(|row| row.interface.id == rate.interface.id)
                .and_then(|row| row.counters)
                .expect("actual source must have native counters");
            if let Some((id, received, transmitted, then)) = previous.as_ref() {
                assert_eq!(
                    &rate.interface.id, id,
                    "native verification requires a stable connected source"
                );
                let elapsed = (time - then) as f64;
                assert_eq!(
                    rate.received_bytes_per_second,
                    (counters.received - received) as f64 * 1000.0 / elapsed
                );
                assert_eq!(
                    rate.transmitted_bytes_per_second,
                    (counters.transmitted - transmitted) as f64 * 1000.0 / elapsed
                );
                verified += 1;
            }
        }
        let baseline = rows.iter().find(|row| {
            row.interface.physical
                && row.interface.connected
                && Some(row.interface.id.as_str()) == cache.selected_interface.as_deref()
        });
        previous = baseline.and_then(|row| {
            row.counters.map(|counters| {
                (
                    row.interface.id.clone(),
                    counters.received,
                    counters.transmitted,
                    time,
                )
            })
        });
        if sample < 2 {
            std::thread::sleep(std::time::Duration::from_millis(1100));
        }
    }
    assert_eq!(verified, 2, "both native intervals must produce live rates");
    assert_eq!(
        cache
            .snapshot(origin.elapsed().as_millis() as u64 + 1)
            .network
            .status,
        MetricStatus::Ready
    );
    eprintln!("native_network_fallback verified=true samples=3 source=physical preference=missing");
}
