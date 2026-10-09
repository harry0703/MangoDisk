//! Bounded native workers: a slow disk cannot delay CPU, network, or window callbacks.
use mangodisk_core::{
    system_resources::{
        metrics::MetricId, models::SystemResourceSnapshot, service::SystemResourceService,
    },
    CoreResult,
};
use mangodisk_platform::system_resources::{
    cpu::{
        details::{CpuDetails, CpuDetailsReader},
        CpuReader, CpuSample,
    },
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
    #[cfg(test)]
    Unsupported,
    Cpu {
        sample: mangodisk_platform::PlatformResult<CpuSample>,
        details: CpuDetails,
    },
    GpuCatalogue(Vec<GpuAdapter>),
    Gpu {
        sample: mangodisk_platform::PlatformResult<GpuSample>,
        adapters: Vec<GpuAdapter>,
    },
    Memory(SystemResourceSnapshot),
    Network(Vec<InterfaceSample>),
    Disk {
        volumes: Vec<ResourceVolume>,
        selected: Option<(ResourceVolume, VolumeCapacity)>,
    },
}

pub enum Request {
    Sample { generation: u64, demand: Demand },
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
        run_worker(metric, origin, events, requests, || Sensor::new(metric));
    });
    sender
}

fn run_worker(
    metric: MetricId,
    origin: Instant,
    events: SyncSender<SamplingEvent>,
    requests: mpsc::Receiver<Request>,
    mut create: impl FnMut() -> Sensor,
) {
    let mut sensor: Option<Sensor> = None;
    let mut generation = None;
    while let Ok(request) = requests.recv() {
        let Request::Sample {
            generation: requested_generation,
            demand,
        } = request
        else {
            // Drop on the owning worker, after any native query has finished. Never
            // cancel a network subscription from its callback or the coordinator.
            if sensor.take().is_some() {
                log::info!("resident_sampling_resources_released metric={metric:?}");
            }
            generation = None;
            continue;
        };
        let sensor = sensor.get_or_insert_with(|| {
            log::info!("resident_sampling_resources_acquired metric={metric:?}");
            create()
        });
        if generation.replace(requested_generation) != Some(requested_generation) {
            sensor.reset_baselines();
        }
        let started = Instant::now();
        let timestamp_ms = timestamp_ms();
        let result = sensor.sample(&demand, timestamp_ms);
        let completion = Completion {
            metric,
            generation: requested_generation,
            monotonic_ms: origin.elapsed().as_millis() as u64,
            timestamp_ms,
            duration_ms: started.elapsed().as_millis() as u64,
            result,
        };
        if events
            .send(SamplingEvent::Completed(Box::new(completion)))
            .is_err()
        {
            break;
        }
    }
}

enum Sensor {
    Cpu {
        reader: CpuReader,
        details: Box<CpuDetailsReader>,
    },
    Gpu(Box<GpuReader>),
    Memory(Box<SystemResourceService>),
    Network(NetworkReader),
    Disk,
    #[cfg(test)]
    Probe(tests::Probe),
}
impl Sensor {
    fn new(metric: MetricId) -> Self {
        match metric {
            #[cfg(windows)]
            MetricId::Cpu => Self::Cpu {
                reader: CpuReader::default(),
                details: Box::default(),
            },
            #[cfg(not(windows))]
            MetricId::Cpu => Self::Cpu {
                reader: CpuReader,
                details: Box::default(),
            },
            MetricId::Gpu => Self::Gpu(Box::default()),
            MetricId::Memory => Self::Memory(Box::default()),
            MetricId::Network => Self::Network(NetworkReader::default()),
            MetricId::Disk => Self::Disk,
        }
    }
    fn reset_baselines(&mut self) {
        if let Self::Cpu { reader, details } = self {
            details.reset();
            // Re-enabling must prime a fresh interval, including short pauses.
            reader.reset();
        }
        if let Self::Gpu(reader) = self {
            reader.reset();
        }
    }
    fn sample(&mut self, demand: &Demand, timestamp_ms: u64) -> CoreResult<Observation> {
        use mangodisk_platform::system_resources::disk;
        Ok(match self {
            Self::Cpu { reader, details } => Observation::Cpu {
                sample: reader.read(),
                details: details.read(demand.detailed, demand.temperature),
            },
            Self::Gpu(reader) if demand.catalogue_only => {
                Observation::GpuCatalogue(reader.catalogue()?)
            }
            Self::Gpu(reader) => {
                let sample = if demand.detailed {
                    reader.read_detailed()
                } else {
                    reader.read()
                };
                Observation::Gpu {
                    sample,
                    adapters: reader.adapters(),
                }
            }
            Self::Memory(reader) => {
                Observation::Memory(reader.sample(demand.detailed, timestamp_ms)?)
            }
            Self::Network(reader) => Observation::Network(reader.read()?),
            Self::Disk => {
                let volumes = disk::list()?;
                let selected = mangodisk_core::system_resources::disk::select(
                    &volumes,
                    demand.selection.as_deref(),
                )
                .map(|volume| disk::capacity(volume).map(|capacity| (volume.clone(), capacity)))
                .transpose()?;
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

    pub(super) struct Probe {
        owner: Option<Arc<()>>,
        pub(super) started: mpsc::Sender<()>,
        pub(super) proceed: mpsc::Receiver<()>,
        retired: mpsc::Sender<()>,
    }
    impl Drop for Probe {
        fn drop(&mut self) {
            self.owner.take();
            let _ = self.retired.send(());
        }
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
            run_worker(MetricId::Memory, Instant::now(), events, jobs, || {
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
                })
            });
        });
        let timeout = Duration::from_secs(3);
        let request = |generation, detailed| Request::Sample {
            generation,
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
