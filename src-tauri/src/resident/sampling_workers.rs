//! Bounded native workers: a slow disk cannot delay CPU, network, or window callbacks.
use mangodisk_core::{
    system_resources::{
        memory_sampling::MemorySamplingService, metrics::MetricId, models::SystemResourceSnapshot,
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

use super::{
    sampling_policy::Task,
    sampling_reading::{NativeReading, Observed},
    sampling_schedule::Demand,
};

pub enum Observation {
    DiskIo(
        mangodisk_platform::PlatformResult<
            Vec<mangodisk_platform::system_resources::disk_io::DeviceCounters>,
        >,
    ),
    DiskSensors(Box<mangodisk_platform::PlatformResult<super::disk_sensor_sampling::Observation>>),
    #[cfg(test)]
    Unsupported,
    Cpu {
        identity: mangodisk_platform::system_resources::cpu::details::CpuIdentity,
        sample: mangodisk_platform::PlatformResult<CpuSample>,
    },
    PowerSensors(Box<super::power_sensor_sampling::Observation>),
    CpuTemperature(Observed<mangodisk_platform::system_resources::cpu::details::CpuDetails>),
    Battery(super::battery_sampling::Observation),
    CpuFrequency(Observed<mangodisk_platform::system_resources::cpu::details::CpuDetails>),
    GpuCatalogue(Vec<GpuAdapter>),
    Gpu {
        power:
            Option<NativeReading<mangodisk_platform::system_resources::component_power::GpuPower>>,
        fan_speed: Option<NativeReading<mangodisk_platform::system_resources::fans::FanSample>>,
        temperature: Option<
            NativeReading<
                Vec<mangodisk_platform::system_resources::gpu::temperature::GpuTemperatureSample>,
            >,
        >,
        sample: NativeReading<GpuSample>,
        adapters: Vec<GpuAdapter>,
    },
    Memory(SystemResourceSnapshot),
    MemoryProcesses(
        Observed<CoreResult<mangodisk_core::system_resources::models::ProcessMemorySummary>>,
    ),
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
        resume_generation: u64,
        requested_at_ms: u64,
        demand: Demand,
    },
    Release,
}
pub struct Completion {
    pub task: Task,
    pub generation: u64,
    pub monotonic_ms: u64,
    pub timestamp_ms: u64,
    pub duration_ms: u64,
    pub queue_ms: u64,
    pub result: CoreResult<Observation>,
}
pub enum SamplingEvent {
    Wake,
    Completed(Box<Completion>),
    ProcessCpu(super::process_cpu_sampling::Completion),
}

pub fn timestamp_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
pub fn start(
    metric: MetricId,
    origin: Instant,
    events: SyncSender<SamplingEvent>,
) -> SyncSender<Request> {
    start_task(metric.into(), origin, events)
}

#[cfg(test)]
pub fn start_task(
    task: Task,
    origin: Instant,
    events: SyncSender<SamplingEvent>,
) -> SyncSender<Request> {
    spawn_task(task, origin, events).0
}

pub fn spawn_task(
    task: Task,
    origin: Instant,
    events: SyncSender<SamplingEvent>,
) -> (SyncSender<Request>, std::thread::JoinHandle<()>) {
    let (sender, requests) = mpsc::sync_channel(1);
    let thread = std::thread::spawn(move || {
        run_worker(task, origin, events, requests, || match task {
            Task::DiskIo => Sensor::DiskIo(Box::default()),
            Task::PowerSensors => Sensor::PowerSensors(Box::default()),
            Task::CpuTemperature => Sensor::CpuTemperature(Box::default()),
            Task::CpuFrequency => Sensor::CpuFrequency(Box::default()),
            Task::Battery => Sensor::Battery(Box::default()),
            Task::DiskSensors => Sensor::DiskSensors(Box::default()),
            Task::MemoryProcesses => Sensor::MemoryProcesses(Box::default()),
            _ => Sensor::new(task.metric()),
        })
    });
    (sender, thread)
}

fn run_worker(
    kind: Task,
    origin: Instant,
    events: SyncSender<SamplingEvent>,
    requests: mpsc::Receiver<Request>,
    mut create: impl FnMut() -> Sensor,
) {
    let metric = kind.metric();
    let mut sensor: Option<Sensor> = None;
    let mut generation = None;
    let mut native_epoch = None;
    while let Ok(request) = requests.recv() {
        let Request::Sample {
            generation: requested_generation,
            baseline_generation,
            resume_generation,
            requested_at_ms,
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
            native_epoch = None;
            continue;
        };
        let sensor = sensor.get_or_insert_with(|| {
            log::info!("resident_sampling_resources_acquired metric={metric:?} worker={kind:?}");
            create()
        });
        let resumed = native_epoch.is_some_and(|previous| previous != resume_generation);
        native_epoch = Some(resume_generation);
        let baseline_changed = generation.replace(baseline_generation) != Some(baseline_generation);
        if resumed {
            sensor.resume_native();
        } else if baseline_changed {
            sensor.reset_baselines();
        }
        let queue_ms = (origin.elapsed().as_millis() as u64).saturating_sub(requested_at_ms);
        let started = Instant::now();
        let sample_started_at_ms = timestamp_ms();
        let result = sensor.sample(&demand, sample_started_at_ms);
        let completion = Completion {
            task: kind,
            generation: requested_generation,
            monotonic_ms: origin.elapsed().as_millis() as u64,
            // Native services may timestamp a reading during the blocking query.
            // Validate its freshness against completion, not the query's start.
            timestamp_ms: timestamp_ms(),
            duration_ms: started.elapsed().as_millis() as u64,
            queue_ms,
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
    },
    PowerSensors(Box<super::power_sensor_sampling::Reader>),
    CpuTemperature(Box<mangodisk_platform::system_resources::cpu::details::CpuDetailsReader>),
    Battery(Box<super::battery_sampling::Reader>),
    CpuFrequency(Box<mangodisk_platform::system_resources::cpu::details::CpuDetailsReader>),
    Gpu(Box<GpuReader>),
    Memory(Box<MemorySamplingService>),
    MemoryProcesses(Box<mangodisk_core::system_resources::process_memory::ProcessMemoryService>),
    Network(NetworkReader),
    DiskIo(Box<mangodisk_platform::system_resources::disk_io::DiskIoReader>),
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
    fn resume_native(&mut self) {
        match self {
            Self::Cpu { reader } => reader.reset(),
            Self::CpuTemperature(reader) => reader.resume_temperature(),
            Self::CpuFrequency(reader) => **reader = Default::default(),
            Self::PowerSensors(reader) => **reader = Default::default(),
            Self::Battery(reader) => **reader = Default::default(),
            Self::Gpu(reader) => **reader = Default::default(),
            Self::Memory(reader) => **reader = Default::default(),
            Self::MemoryProcesses(reader) => **reader = Default::default(),
            Self::Network(reader) => *reader = Default::default(),
            Self::DiskIo(reader) => **reader = Default::default(),
            Self::Disk { capacity_state } => *capacity_state = None,
            // SMART owns no persistent native connection; its source-bound cache
            // remains subject to the refreshed catalogue and original timestamps.
            Self::DiskSensors(_) => {}
            #[cfg(test)]
            Self::Probe(_) => self.reset_baselines(),
        }
    }
    fn sample(&mut self, demand: &Demand, timestamp_ms: u64) -> CoreResult<Observation> {
        use mangodisk_platform::system_resources::disk;
        Ok(match self {
            Self::Cpu { reader } => Observation::Cpu {
                identity: mangodisk_platform::system_resources::cpu::details::cpu_identity(),
                sample: reader.read(),
            },
            Self::PowerSensors(reader) => Observation::PowerSensors(Box::new(reader.read(demand))),
            Self::CpuTemperature(reader) => Observation::CpuTemperature(Observed::read(|| {
                reader.read(false, demand.temperature)
            })),
            Self::Battery(reader) => Observation::Battery(reader.read(demand)),
            Self::CpuFrequency(reader) => {
                Observation::CpuFrequency(Observed::read(|| reader.read(demand.frequency, false)))
            }
            Self::Gpu(reader) if demand.catalogue_only => {
                Observation::GpuCatalogue(reader.catalogue()?)
            }
            Self::Gpu(reader) => {
                let sample = Observed::read(|| {
                    if demand.detailed {
                        reader.read_detailed()
                    } else {
                        reader.read()
                    }
                });
                let temperature = demand
                    .temperature
                    .then(|| Observed::read(|| reader.read_temperatures()).into_optional())
                    .flatten();
                #[cfg(windows)]
                let fan_speed = if demand.fan_speed {
                    Some(Observed::read(|| reader.read_fans()))
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
                            let Ok(GpuSample::Usage(adapters)) = &sample.value else {
                                return None;
                            };
                            let index =
                                mangodisk_core::system_resources::gpu::select_adapter_index(
                                    adapters, None,
                                )?;
                            Some(adapters[index].id.as_str())
                        });
                        Observed::read(|| reader.read_power(selected)).into_optional()
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
            Self::Memory(reader) => Observation::Memory(reader.sample(false, timestamp_ms)?),
            Self::MemoryProcesses(reader) => {
                Observation::MemoryProcesses(Observed::read(|| reader.sample()))
            }
            Self::Network(reader) => Observation::Network(reader.read()?),
            Self::DiskIo(reader) => Observation::DiskIo(reader.read()),
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
#[path = "sampling_workers_tests.rs"]
mod tests;
