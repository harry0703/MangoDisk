use super::*;
use crate::resident::sampling_schedule::Demand;
use mangodisk_core::{
    system_resources::models::{MemoryOverview, SystemResourceSnapshot},
    CoreError,
};
use mangodisk_platform::system_resources::{
    cpu::{details::CpuDetails, CpuSample},
    gpu::GpuSample,
    memory::{MemoryPressure, ProcessMemoryKind},
};

fn completion(metric: MetricId, result: mangodisk_core::CoreResult<Observation>) -> Completion {
    Completion {
        task: metric.into(),
        generation: 0,
        monotonic_ms: 1000,
        timestamp_ms: 1000,
        duration_ms: 0,
        queue_ms: 0,
        result,
    }
}

#[test]
fn baseline_retries_remain_bounded_and_return_to_the_regular_cadence() {
    for metric in [MetricId::Cpu, MetricId::Gpu] {
        let mut state = ObservationState::new(false);
        let mut cache = ResourceCache::default();
        let mut slot = SamplingSlot::default();
        slot.update(Demand {
            active: true,
            ..Demand::default()
        });
        let catalogue = AtomicU8::new(7);
        for (attempt, now) in [0, 250, 500].into_iter().enumerate() {
            let generation = slot
                .begin(now, 2000)
                .expect("the accepted sample must be due");
            assert!(slot.complete(generation));
            let sample = match metric {
                MetricId::Cpu => Observation::Cpu {
                    identity: Default::default(),
                    sample: Ok(CpuSample::Baseline),
                },
                MetricId::Gpu => Observation::Gpu {
                    power: None,
                    fan_speed: None,
                    sample: crate::resident::sampling_reading::Observed::at(
                        Ok(GpuSample::Baseline),
                        1000,
                    ),
                    adapters: vec![],
                    temperature: None,
                },
                _ => unreachable!(),
            };
            state.apply(
                completion(metric, Ok(sample)),
                &mut cache,
                &mut slot,
                ObservationContext {
                    catalogue: &catalogue,
                    now_ms: &|| now,
                    previous_status: MetricStatus::Loading,
                },
                |_| panic!("resource baselines must not warm application icons"),
            );
            assert!(slot.begin(now + 249, 2000).is_none());
            if attempt == 2 {
                assert!(slot.begin(now + 250, 2000).is_none());
                assert!(slot.begin(now + 2000, 2000).is_some());
            }
        }
    }
}

#[test]
fn accepted_memory_observations_warm_icons_once_without_changing_the_snapshot() {
    let mut state = ObservationState::new(true);
    let mut cache = ResourceCache::default();
    let mut slot = SamplingSlot::default();
    let catalogue = AtomicU8::new(7);
    let mut warm_count = 0;
    for _ in 0..2 {
        let mut snapshot = SystemResourceSnapshot {
            schema_version: 5,
            sampled_at_ms: 1000,
            memory: MemoryOverview {
                total_bytes: 100,
                used_bytes: 75,
                free_bytes: 25,
                swap_used_bytes: 0,
                used_percent: 75,
                pressure: MemoryPressure::Normal,
            },
            processes: Some(ProcessMemorySummary {
                usage_kind: ProcessMemoryKind::native(),
                applications: vec![],
                readable_process_count: 3,
                omitted_process_count: 0,
            }),
        };
        let summary = snapshot.processes.take().unwrap();
        state.apply(
            completion(MetricId::Memory, Ok(Observation::Memory(snapshot))),
            &mut cache,
            &mut slot,
            ObservationContext {
                catalogue: &catalogue,
                now_ms: &|| 1000,
                previous_status: MetricStatus::Loading,
            },
            |_| panic!("overview sampling must not enumerate or warm applications"),
        );
        state.apply(
            completion(
                MetricId::Memory,
                Ok(Observation::MemoryProcesses(
                    crate::resident::sampling_reading::Observed::at(Ok(summary), 1000),
                )),
            ),
            &mut cache,
            &mut slot,
            ObservationContext {
                catalogue: &catalogue,
                now_ms: &|| 1000,
                previous_status: MetricStatus::Loading,
            },
            |summary| {
                assert_eq!(summary.readable_process_count, 3);
                warm_count += 1;
            },
        );
    }
    assert_eq!(warm_count, 1);
    assert!(!state.warm_icons);
    assert_eq!(
        cache
            .snapshot(1000)
            .memory
            .value
            .unwrap()
            .memory
            .used_percent,
        75
    );
}

#[test]
fn failed_observations_clear_only_their_catalogue_request_and_preserve_other_demand() {
    for (metric, remaining) in [
        (MetricId::Cpu, 7),
        (MetricId::Gpu, 3),
        (MetricId::Network, 6),
        (MetricId::Disk, 5),
        (MetricId::Memory, 7),
    ] {
        let mut state = ObservationState::new(true);
        let mut cache = ResourceCache::default();
        let mut slot = SamplingSlot::default();
        let catalogue = AtomicU8::new(7);
        state.apply(
            completion(
                metric,
                Err(CoreError::operation_failed("fixture sample unavailable")),
            ),
            &mut cache,
            &mut slot,
            ObservationContext {
                catalogue: &catalogue,
                now_ms: &|| 1000,
                previous_status: MetricStatus::Loading,
            },
            |_| panic!("a failed observation must not warm application icons"),
        );
        assert_eq!(catalogue.load(Ordering::Relaxed), remaining);
        assert!(
            state.warm_icons,
            "an overview failure must not stop the independent process warm-up"
        );
        let reading = cache.snapshot(1000);
        let status = match metric {
            MetricId::Cpu => reading.cpu.status,
            MetricId::Gpu => reading.gpu.status,
            MetricId::Memory => reading.memory.status,
            MetricId::Network => reading.network.status,
            MetricId::Disk => reading.disk.status,
        };
        assert_eq!(status, MetricStatus::Failed);
    }
}

#[test]
fn auxiliary_failures_preserve_fresh_utilization_and_history() {
    use mangodisk_platform::{PlatformError, PlatformErrorCode};
    let mut state = ObservationState::new(false);
    let mut cache = ResourceCache::default();
    cache.cpu(
        CpuSample::Percent {
            used: 42.0,
            interval_ms: 2000,
        },
        1000,
        1000,
    );
    let mut slot = SamplingSlot::default();
    let catalogue = AtomicU8::new(0);
    state.apply(
        completion(
            MetricId::Cpu,
            Ok(Observation::CpuFrequency(
                crate::resident::sampling_reading::Observed::at(
                    CpuDetails {
                        identity: Default::default(),
                        frequency: Some(Err(PlatformError::new(
                            PlatformErrorCode::Unsupported,
                            "frequency unavailable",
                        ))),
                        temperature: None,
                    },
                    1000,
                ),
            )),
        ),
        &mut cache,
        &mut slot,
        ObservationContext {
            catalogue: &catalogue,
            now_ms: &|| 1000,
            previous_status: MetricStatus::Ready,
        },
        |_| panic!("auxiliary observations cannot enumerate applications"),
    );
    state.apply(
        completion(
            MetricId::Cpu,
            Ok(Observation::CpuTemperature(
                crate::resident::sampling_reading::Observed::at(
                    CpuDetails {
                        identity: Default::default(),
                        frequency: None,
                        temperature: Some(Err(PlatformError::new(
                            PlatformErrorCode::AccessDenied,
                            "temperature denied",
                        ))),
                    },
                    1000,
                ),
            )),
        ),
        &mut cache,
        &mut slot,
        ObservationContext {
            catalogue: &catalogue,
            now_ms: &|| 1000,
            previous_status: MetricStatus::Ready,
        },
        |_| {},
    );
    let reading = cache.snapshot(1000);
    assert_eq!(reading.cpu.status, MetricStatus::Ready);
    assert_eq!(reading.cpu.value.unwrap().used_percent, 42.0);
    assert_eq!(reading.cpu_history.len(), 1);
    assert_eq!(reading.cpu_frequency.status, MetricStatus::Unsupported);
    assert_eq!(reading.cpu_temperature.status, MetricStatus::Failed);
}

#[test]
fn sensor_discovery_failure_never_overwrites_confirmed_capacity() {
    use mangodisk_platform::{
        system_resources::disk::{ResourceVolume, VolumeCapacity},
        PlatformError, PlatformErrorCode,
    };
    let mut state = ObservationState::new(false);
    let mut cache = ResourceCache::default();
    cache.disk(
        ResourceVolume {
            id: "one".into(),
            name: "Disk".into(),
            system: true,
            mount_point: "/".into(),
            mount_points: vec![],
            file_system: None,
            devices: vec![],
        },
        VolumeCapacity {
            total_bytes: 100,
            available_bytes: 50,
        },
        1000,
    );
    let mut slot = SamplingSlot::default();
    slot.update(Demand {
        active: true,
        disk_health: true,
        ..Default::default()
    });
    let catalogue = AtomicU8::new(0);
    state.apply(
        completion(
            MetricId::Disk,
            Ok(Observation::DiskSensors(Box::new(Err(PlatformError::new(
                PlatformErrorCode::Io,
                "disk sensors catalogue unavailable",
            ))))),
        ),
        &mut cache,
        &mut slot,
        ObservationContext {
            catalogue: &catalogue,
            now_ms: &|| 1000,
            previous_status: MetricStatus::Ready,
        },
        |_| {},
    );
    let reading = cache.snapshot(1000);
    assert_eq!(reading.disk.status, MetricStatus::Ready);
    assert_eq!(reading.disk.value.unwrap().volume.id, "one");
    assert_eq!(reading.disk_health.status, MetricStatus::Failed);
}
