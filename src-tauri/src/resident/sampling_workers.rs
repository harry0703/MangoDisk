//! Bounded native workers: a slow disk cannot delay CPU, network, or window callbacks.
use mangodisk_core::{
    system_resources::{
        metrics::MetricId, models::SystemResourceSnapshot, service::SystemResourceService,
    },
    CoreResult,
};
use mangodisk_platform::system_resources::{
    cpu::{CpuReader, CpuSample},
    disk::{ResourceVolume, VolumeCapacity},
    gpu::{GpuAdapter, GpuReader, GpuSample},
    network::{InterfaceSample, NetworkReader},
};
use std::{
    sync::mpsc::{self, SyncSender},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use super::sampling_schedule::Demand;

pub enum Observation {
    DiskSensors(Box<mangodisk_platform::PlatformResult<super::disk_sensor_sampling::Observation>>),
    #[cfg(test)]
    Unsupported,
    Cpu {
        sample: mangodisk_platform::PlatformResult<CpuSample>,
    },
    CpuAuxiliary(Box<super::cpu_auxiliary_sampling::Observation>),
    GpuCatalogue(Vec<GpuAdapter>),
    Gpu {
        power: Option<
            mangodisk_platform::PlatformResult<
                mangodisk_platform::system_resources::component_power::GpuPower,
            >,
        >,
        fan_speed: Option<
            mangodisk_platform::PlatformResult<
                mangodisk_platform::system_resources::fans::FanSample,
            >,
        >,
        temperature: Option<
            mangodisk_platform::PlatformResult<
                Vec<mangodisk_platform::system_resources::gpu::temperature::GpuTemperatureSample>,
            >,
        >,
        sample: mangodisk_platform::PlatformResult<GpuSample>,
        adapters: Vec<GpuAdapter>,
    },
    Memory(SystemResourceSnapshot),
    Network(Vec<InterfaceSample>),
    Disk {
        volumes: Vec<ResourceVolume>,
        selected: Option<(
            Box<ResourceVolume>,
            mangodisk_platform::PlatformResult<VolumeCapacity>,
        )>,
    },
}

pub enum Request {
    Sample {
        generation: u64,
        baseline_generation: u64,
        demand: Demand,
    },
    Release,
}
pub struct Completion {
    pub metric: MetricId,
    pub generation: u64,
    pub monotonic_ms: u64,
    pub timestamp_ms: u64,
    pub duration_ms: u64,
    pub result: CoreResult<Observation>,
}
pub enum SamplingEvent {
    Wake,
    DiskIo(super::disk_activity::Completion),
    Completed(Box<Completion>),
    CpuAuxiliary(Box<Completion>),
    DiskSensors(Box<Completion>),
    ProcessCpu(super::process_cpu_sampling::Completion),
}

pub fn timestamp_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

pub fn start(
    metric: MetricId,
    origin: Instant,
    events: SyncSender<SamplingEvent>,
) -> SyncSender<Request> {
    let (sender, requests) = mpsc::sync_channel::<Request>(1);
    std::thread::spawn(move || {
        run_worker(WorkerKind::Metric(metric), origin, events, requests, || {
            Sensor::new(metric)
        });
    });
    sender
}

#[derive(Clone, Copy, Debug)]
enum WorkerKind {
    Metric(MetricId),
    CpuAuxiliary,
    DiskSensors,
}
impl WorkerKind {
    fn metric(self) -> MetricId {
        match self {
            Self::Metric(metric) => metric,
            Self::CpuAuxiliary => MetricId::Cpu,
            Self::DiskSensors => MetricId::Disk,
        }
    }
    fn completion(self, value: Box<Completion>) -> SamplingEvent {
        match self {
            Self::Metric(_) => SamplingEvent::Completed(value),
            Self::CpuAuxiliary => SamplingEvent::CpuAuxiliary(value),
            Self::DiskSensors => SamplingEvent::DiskSensors(value),
        }
    }
}
pub fn start_cpu_auxiliary(
    origin: Instant,
    events: SyncSender<SamplingEvent>,
) -> SyncSender<Request> {
    let (sender, requests) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        run_worker(WorkerKind::CpuAuxiliary, origin, events, requests, || {
            Sensor::CpuAuxiliary(Box::default())
        })
    });
    sender
}

pub fn start_disk_sensors(
    origin: Instant,
    events: SyncSender<SamplingEvent>,
) -> SyncSender<Request> {
    let (sender, requests) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        run_worker(WorkerKind::DiskSensors, origin, events, requests, || {
            Sensor::DiskSensors(Box::default())
        })
    });
    sender
}

fn run_worker(
    kind: WorkerKind,
    origin: Instant,
    events: SyncSender<SamplingEvent>,
    requests: mpsc::Receiver<Request>,
    mut create: impl FnMut() -> Sensor,
) {
    let metric = kind.metric();
    let mut sensor: Option<Sensor> = None;
    let mut generation = None;
    while let Ok(request) = requests.recv() {
        let Request::Sample {
            generation: requested_generation,
            baseline_generation,
            demand,
        } = request
        else {
            // Drop on the owning worker, after any native query has finished. Never
            // cancel a network subscription from its callback or the coordinator.
            if sensor.take().is_some() {
                log::info!(
                    "resident_sampling_resources_released metric={metric:?} worker={kind:?}"
                );
            }
            generation = None;
            continue;
        };
        let sensor = sensor.get_or_insert_with(|| {
            log::info!("resident_sampling_resources_acquired metric={metric:?} worker={kind:?}");
            create()
        });
        if generation.replace(baseline_generation) != Some(baseline_generation) {
            sensor.reset_baselines();
        }
        let started = Instant::now();
        let sample_started_at_ms = timestamp_ms();
        let result = sensor.sample(&demand, sample_started_at_ms);
        let completion = Completion {
            metric,
            generation: requested_generation,
            monotonic_ms: origin.elapsed().as_millis() as u64,
            // Native services may timestamp a reading during the blocking query.
            // Validate its freshness against completion, not the query's start.
            timestamp_ms: timestamp_ms(),
            duration_ms: started.elapsed().as_millis() as u64,
            result,
        };
        if events.send(kind.completion(Box::new(completion))).is_err() {
            break;
        }
    }
}

enum Sensor {
    Cpu {
        reader: CpuReader,
    },
    CpuAuxiliary(Box<super::cpu_auxiliary_sampling::Reader>),
    Gpu(Box<GpuReader>),
    Memory(Box<SystemResourceService>),
    Network(NetworkReader),
    Disk {
        capacity_state: Option<String>,
    },
    DiskSensors(Box<super::disk_sensor_sampling::Reader>),
    #[cfg(test)]
    Probe(tests::Probe),
}
impl Sensor {
    fn new(metric: MetricId) -> Self {
        match metric {
            #[cfg(windows)]
            MetricId::Cpu => Self::Cpu {
                reader: CpuReader::default(),
            },
            #[cfg(not(windows))]
            MetricId::Cpu => Self::Cpu { reader: CpuReader },
            MetricId::Gpu => Self::Gpu(Box::default()),
            MetricId::Memory => Self::Memory(Box::default()),
            MetricId::Network => Self::Network(NetworkReader::default()),
            MetricId::Disk => Self::Disk {
                capacity_state: None,
            },
        }
    }
    fn reset_baselines(&mut self) {
        if let Self::Cpu { reader, .. } = self {
            // Re-enabling must prime a fresh interval, including short pauses.
            reader.reset();
        }
        if let Self::Gpu(reader) = self {
            reader.reset();
        }
        #[cfg(test)]
        if let Self::Probe(probe) = self {
            probe
                .resets
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    }
    fn sample(&mut self, demand: &Demand, timestamp_ms: u64) -> CoreResult<Observation> {
        use mangodisk_platform::system_resources::disk;
        Ok(match self {
            Self::Cpu { reader } => Observation::Cpu {
                sample: reader.read(),
            },
            Self::CpuAuxiliary(reader) => Observation::CpuAuxiliary(Box::new(reader.read(demand))),
            Self::Gpu(reader) if demand.catalogue_only => {
                Observation::GpuCatalogue(reader.catalogue()?)
            }
            Self::Gpu(reader) => {
                let sample = if demand.detailed {
                    reader.read_detailed()
                } else {
                    reader.read()
                };
                let temperature = demand
                    .temperature
                    .then(|| reader.read_temperatures())
                    .flatten();
                #[cfg(windows)]
                let fan_speed = if demand.fan_speed {
                    Some(reader.read_fans())
                } else {
                    reader.pause_fans();
                    None
                };
                #[cfg(not(windows))]
                let fan_speed = None;
                let power = demand
                    .component_power
                    .then(|| {
                        let selected = demand.selection.as_deref().or_else(|| {
                            let Ok(GpuSample::Usage(adapters)) = &sample else {
                                return None;
                            };
                            let index =
                                mangodisk_core::system_resources::gpu::select_adapter_index(
                                    adapters, None,
                                )?;
                            Some(adapters[index].id.as_str())
                        });
                        reader.read_power(selected)
                    })
                    .flatten();
                Observation::Gpu {
                    power,
                    fan_speed,
                    temperature,
                    sample,
                    adapters: reader.adapters(),
                }
            }
            Self::Memory(reader) => {
                Observation::Memory(reader.sample(demand.detailed, timestamp_ms)?)
            }
            Self::Network(reader) => Observation::Network(reader.read()?),
            Self::DiskSensors(reader) => {
                Observation::DiskSensors(Box::new(reader.read(demand, timestamp_ms)))
            }
            Self::Disk { capacity_state } => {
                let volumes = disk::list()?;
                let selected = mangodisk_core::system_resources::disk::select(
                    &volumes,
                    demand.selection.as_deref(),
                );
                let selected = selected.map(|volume| {
                    let capacity = disk::capacity(volume);
                    let identity = match &capacity {
                        Ok(_) => format!("{}:ready", volume.id),
                        Err(error) => {
                            format!("{}:{:?}:{}", volume.id, error.code(), error.diagnostic())
                        }
                    };
                    if capacity_state.as_ref() != Some(&identity) {
                        if let Err(error) = &capacity {
                            log::info!(
                                "resident_disk_capacity_unavailable volume={} code={:?} error={}",
                                mangodisk_platform::diagnostics::text(&volume.name),
                                error.code(),
                                mangodisk_platform::diagnostics::text(&error)
                            );
                        } else if capacity_state.is_some() {
                            log::info!(
                                "resident_disk_capacity_recovered volume={}",
                                mangodisk_platform::diagnostics::text(&volume.name)
                            );
                        }
                        *capacity_state = Some(identity);
                    }
                    (Box::new(volume.clone()), capacity)
                });
                Observation::Disk { volumes, selected }
            }
            #[cfg(test)]
            Self::Probe(probe) => {
                probe.started.send(()).unwrap();
                probe.proceed.recv().unwrap();
                Observation::Unsupported
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex, Weak};
    use std::time::Duration;

    #[test]
    fn blocked_disk_sensors_do_not_hold_native_capacity_observations() {
        let (events, completions) = mpsc::sync_channel(4);
        let capacity = start(MetricId::Disk, Instant::now(), events.clone());
        let (sensors, requests) = mpsc::sync_channel(1);
        let (started, starts) = mpsc::channel();
        let (proceed, wait) = mpsc::channel();
        let (retired, _) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            let mut probe = Some(Probe {
                owner: None,
                started,
                proceed: wait,
                retired,
                resets: Default::default(),
            });
            run_worker(
                WorkerKind::DiskSensors,
                Instant::now(),
                events,
                requests,
                || Sensor::Probe(probe.take().unwrap()),
            );
        });
        let request = || Request::Sample {
            generation: 1,
            baseline_generation: 1,
            demand: Demand {
                active: true,
                disk_health: true,
                ..Default::default()
            },
        };
        sensors.send(request()).unwrap();
        starts.recv_timeout(Duration::from_secs(3)).unwrap();
        capacity.send(request()).unwrap();
        let completion = completions.recv_timeout(Duration::from_secs(30));
        // Always release and join the controlled query, including assertion failures.
        proceed.send(()).unwrap();
        drop(sensors);
        drop(capacity);
        worker.join().unwrap();
        let SamplingEvent::Completed(result) =
            completion.expect("capacity must finish before the slow sensor is released")
        else {
            panic!("expected independent capacity observation")
        };
        assert!(matches!(
            result.result,
            Ok(Observation::Disk {
                selected: Some((_, Ok(_))),
                ..
            })
        ));
    }

    pub(super) struct Probe {
        owner: Option<Arc<()>>,
        pub(super) started: mpsc::Sender<()>,
        pub(super) proceed: mpsc::Receiver<()>,
        retired: mpsc::Sender<()>,
        pub(super) resets: Arc<std::sync::atomic::AtomicUsize>,
    }
    impl Drop for Probe {
        fn drop(&mut self) {
            self.owner.take();
            let _ = self.retired.send(());
        }
    }

    #[test]
    fn blocked_auxiliary_queries_do_not_block_native_cpu_observations() {
        let (events, completions) = mpsc::sync_channel(4);
        let cpu = start(MetricId::Cpu, Instant::now(), events.clone());
        let (auxiliary, requests) = mpsc::sync_channel(1);
        let (started, starts) = mpsc::channel();
        let (proceed, wait) = mpsc::channel();
        let (retired, _) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            let mut probe = Some(Probe {
                owner: None,
                started,
                proceed: wait,
                retired,
                resets: Default::default(),
            });
            run_worker(
                WorkerKind::CpuAuxiliary,
                Instant::now(),
                events,
                requests,
                || Sensor::Probe(probe.take().unwrap()),
            );
        });
        let request = || Request::Sample {
            generation: 1,
            baseline_generation: 1,
            demand: Demand {
                active: true,
                ..Default::default()
            },
        };
        auxiliary.send(request()).unwrap();
        starts.recv_timeout(Duration::from_secs(3)).unwrap();
        cpu.send(request()).unwrap();
        // Verify independence while the auxiliary worker remains blocked, without
        // imposing a sampling deadline on Windows' cold native-provider setup.
        assert!(matches!(
            completions
                .recv_timeout(Duration::from_secs(30))
                .expect("CPU worker must complete before the auxiliary worker is released"),
            SamplingEvent::Completed(_)
        ));
        proceed.send(()).unwrap();
        assert!(matches!(
            completions.recv_timeout(Duration::from_secs(3)).unwrap(),
            SamplingEvent::CpuAuxiliary(_)
        ));
        drop(auxiliary);
        drop(cpu);
        worker.join().unwrap();
    }

    #[test]
    fn detail_changes_preserve_the_owned_native_counter_baseline() {
        let (requests, jobs) = mpsc::sync_channel(1);
        let (events, completed) = mpsc::sync_channel(1);
        let (started, starts) = mpsc::channel();
        let (proceed, wait) = mpsc::channel();
        let (retired, _) = mpsc::channel();
        let resets = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let observed = resets.clone();
        let worker = std::thread::spawn(move || {
            let mut probe = Some(Probe {
                owner: None,
                started,
                proceed: wait,
                retired,
                resets,
            });
            run_worker(
                WorkerKind::Metric(MetricId::Gpu),
                Instant::now(),
                events,
                jobs,
                || Sensor::Probe(probe.take().unwrap()),
            );
        });
        for (generation, baseline_generation, expected) in [(1, 1, 1), (2, 1, 1), (3, 2, 2)] {
            requests
                .send(Request::Sample {
                    generation,
                    baseline_generation,
                    demand: Demand {
                        active: true,
                        detailed: generation > 1,
                        ..Default::default()
                    },
                })
                .unwrap();
            starts.recv_timeout(Duration::from_secs(3)).unwrap();
            assert_eq!(
                observed.load(std::sync::atomic::Ordering::Relaxed),
                expected
            );
            proceed.send(()).unwrap();
            completed.recv_timeout(Duration::from_secs(3)).unwrap();
        }
        drop(requests);
        worker.join().unwrap();
    }

    #[test]
    fn completion_timestamp_includes_time_spent_acquiring_native_samples() {
        let (requests, jobs) = mpsc::sync_channel(1);
        let (events, completed) = mpsc::sync_channel(1);
        let (started, starts) = mpsc::channel();
        let (proceed, wait) = mpsc::channel();
        let (retired, _) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            let mut probe = Some(Probe {
                owner: None,
                started,
                proceed: wait,
                retired,
                resets: Default::default(),
            });
            run_worker(
                WorkerKind::Metric(MetricId::Cpu),
                Instant::now(),
                events,
                jobs,
                || Sensor::Probe(probe.take().unwrap()),
            );
        });
        requests
            .send(Request::Sample {
                generation: 1,
                baseline_generation: 1,
                demand: Demand {
                    active: true,
                    temperature: true,
                    ..Default::default()
                },
            })
            .unwrap();
        starts.recv_timeout(Duration::from_secs(3)).unwrap();
        // A native service timestamps its reading during the blocking query.
        std::thread::sleep(Duration::from_millis(10));
        let native_sampled_at_ms = timestamp_ms();
        proceed.send(()).unwrap();
        let SamplingEvent::Completed(completion) =
            completed.recv_timeout(Duration::from_secs(3)).unwrap()
        else {
            panic!("expected completion")
        };
        drop(requests);
        worker.join().unwrap();
        assert!(
            completion.timestamp_ms >= native_sampled_at_ms,
            "a freshly acquired native sample must not appear to come from the future"
        );
    }

    #[test]
    fn release_waits_for_queries_and_reenable_creates_a_new_owner_without_resetting_on_hide() {
        let (requests, jobs) = mpsc::sync_channel(1);
        let (events, completed) = mpsc::sync_channel(8);
        let (started, starts) = mpsc::channel();
        let (retired, retirements) = mpsc::channel();
        let owners = Arc::new(Mutex::new(Vec::<(Weak<()>, mpsc::Sender<()>)>::new()));
        let observed = owners.clone();
        let worker = std::thread::spawn(move || {
            run_worker(
                WorkerKind::Metric(MetricId::Memory),
                Instant::now(),
                events,
                jobs,
                || {
                    let owner = Arc::new(());
                    let (proceed, wait) = mpsc::channel();
                    observed
                        .lock()
                        .unwrap()
                        .push((Arc::downgrade(&owner), proceed));
                    Sensor::Probe(Probe {
                        owner: Some(owner),
                        started: started.clone(),
                        proceed: wait,
                        retired: retired.clone(),
                        resets: Default::default(),
                    })
                },
            );
        });
        let timeout = Duration::from_secs(3);
        let request = |generation, detailed| Request::Sample {
            generation,
            baseline_generation: generation,
            demand: Demand {
                active: true,
                detailed,
                ..Default::default()
            },
        };
        assert!(
            owners.lock().unwrap().is_empty(),
            "idle workers acquire no native resources"
        );
        requests.send(request(1, true)).unwrap();
        starts.recv_timeout(timeout).unwrap();
        requests.try_send(Request::Release).unwrap();
        assert!(
            owners.lock().unwrap()[0].0.upgrade().is_some(),
            "an in-flight native query still owns its handles"
        );
        owners.lock().unwrap()[0].1.send(()).unwrap();
        let SamplingEvent::Completed(first) = completed.recv_timeout(timeout).unwrap() else {
            panic!("expected completion")
        };
        assert_eq!(first.generation, 1);
        retirements.recv_timeout(timeout).unwrap();
        assert!(
            owners.lock().unwrap()[0].0.upgrade().is_none(),
            "release must retire the last owner after completion"
        );
        requests.send(request(3, true)).unwrap();
        starts.recv_timeout(timeout).unwrap();
        assert_eq!(
            owners.lock().unwrap().len(),
            2,
            "reenable lazily acquires a fresh owner"
        );
        owners.lock().unwrap()[1].1.send(()).unwrap();
        completed.recv_timeout(timeout).unwrap();
        // Hiding the panel changes detail demand, not the active resource lifetime.
        requests.send(request(4, false)).unwrap();
        starts.recv_timeout(timeout).unwrap();
        assert_eq!(owners.lock().unwrap().len(), 2);
        assert!(retirements.try_recv().is_err());
        owners.lock().unwrap()[1].1.send(()).unwrap();
        completed.recv_timeout(timeout).unwrap();
        requests.send(Request::Release).unwrap();
        retirements.recv_timeout(timeout).unwrap();
        assert!(owners.lock().unwrap()[1].0.upgrade().is_none());
        drop(requests);
        worker.join().unwrap();
    }

    #[test]
    fn native_memory_and_network_sampling_resume_after_resource_release() {
        for metric in [MetricId::Memory, MetricId::Network] {
            let (events, completions) = mpsc::sync_channel(8);
            let requests = start(metric, Instant::now(), events);
            for generation in [1, 3, 5] {
                requests
                    .send(Request::Sample {
                        generation,
                        baseline_generation: generation,
                        demand: Demand {
                            active: true,
                            detailed: metric == MetricId::Memory,
                            ..Default::default()
                        },
                    })
                    .unwrap();
                let SamplingEvent::Completed(completion) =
                    completions.recv_timeout(Duration::from_secs(10)).unwrap()
                else {
                    panic!("expected completion")
                };
                assert_eq!(completion.generation, generation);
                let observation = completion
                    .result
                    .expect("native reader must recover after recreation");
                match observation {
                    Observation::Memory(snapshot) => assert!(snapshot.processes.is_some()),
                    Observation::Network(interfaces) => assert!(!interfaces.is_empty()),
                    _ => panic!("unexpected native sensor"),
                }
                requests.send(Request::Release).unwrap();
            }
        }
    }
}
