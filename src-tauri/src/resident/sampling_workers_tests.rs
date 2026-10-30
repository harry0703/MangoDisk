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
        run_worker(Task::DiskSensors, Instant::now(), events, requests, || {
            Sensor::Probe(probe.take().unwrap())
        });
    });
    let request = || Request::Sample {
        generation: 1,
        baseline_generation: 1,
        resume_generation: 0,
        requested_at_ms: 0,
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
        run_worker(Task::PowerSensors, Instant::now(), events, requests, || {
            Sensor::Probe(probe.take().unwrap())
        });
    });
    let request = || Request::Sample {
        generation: 1,
        baseline_generation: 1,
        resume_generation: 0,
        requested_at_ms: 0,
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
        SamplingEvent::Completed(_)
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
            Task::from(MetricId::Gpu),
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
                resume_generation: 0,
                requested_at_ms: 0,
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
            Task::from(MetricId::Cpu),
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
            resume_generation: 0,
            requested_at_ms: 0,
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
            Task::from(MetricId::Memory),
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
        resume_generation: 0,
        requested_at_ms: 0,
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
    for task in [Task::Memory, Task::MemoryProcesses, Task::Network] {
        let (events, completions) = mpsc::sync_channel(8);
        let requests = start_task(task, Instant::now(), events);
        for generation in [1, 3, 5] {
            requests
                .send(Request::Sample {
                    generation,
                    baseline_generation: generation,
                    resume_generation: 0,
                    requested_at_ms: 0,
                    demand: Demand {
                        active: true,
                        detailed: task == Task::MemoryProcesses,
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
                Observation::Memory(snapshot) => assert!(snapshot.processes.is_none()),
                Observation::MemoryProcesses(reading) => {
                    assert!(reading.value.unwrap().readable_process_count > 0)
                }
                Observation::Network(interfaces) => assert!(!interfaces.is_empty()),
                _ => panic!("unexpected native sensor"),
            }
            requests.send(Request::Release).unwrap();
        }
    }
}

#[test]
#[cfg(target_os = "macos")]
#[ignore = "requires an identified native CPU temperature sensor"]
fn native_temperature_is_independent_of_blocked_battery_and_auxiliary_sources() {
    for blocked_task in [Task::Battery, Task::PowerSensors, Task::CpuFrequency] {
        let (events, completions) = mpsc::sync_channel(4);
        let temperature = start_task(Task::CpuTemperature, Instant::now(), events.clone());
        let (blocked_sender, requests) = mpsc::sync_channel(1);
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
            run_worker(blocked_task, Instant::now(), events, requests, || {
                Sensor::Probe(probe.take().unwrap())
            });
        });
        let request = || Request::Sample {
            generation: 1,
            baseline_generation: 1,
            resume_generation: 0,
            requested_at_ms: 0,
            demand: Demand {
                active: true,
                temperature: true,
                ..Default::default()
            },
        };
        blocked_sender.send(request()).unwrap();
        starts.recv_timeout(Duration::from_secs(3)).unwrap();
        let at = Instant::now();
        temperature.send(request()).unwrap();
        let observed = completions.recv_timeout(Duration::from_secs(10));
        let elapsed_us = at.elapsed().as_micros();
        proceed.send(()).unwrap();
        drop(blocked_sender);
        drop(temperature);
        worker.join().unwrap();
        let SamplingEvent::Completed(observation) =
            observed.expect("temperature must arrive before the blocked source is released")
        else {
            panic!("expected temperature completion")
        };
        assert_eq!(observation.task, Task::CpuTemperature);
        let Observation::CpuTemperature(observed) = observation.result.unwrap() else {
            panic!("expected temperature reading")
        };
        let temperature = observed.value.temperature.unwrap().unwrap();
        assert!(temperature.celsius > 0.0);
        println!("native_temperature_isolation blocked_task={blocked_task:?} temperature_us={elapsed_us} sampled_at_ms={:?}", temperature.sampled_at_ms);
    }
}

#[test]
fn native_memory_overview_finishes_while_process_enumeration_is_blocked() {
    let (events, completions) = mpsc::sync_channel(4);
    let overview = start_task(Task::Memory, Instant::now(), events.clone());
    let (processes, requests) = mpsc::sync_channel(1);
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
            Task::MemoryProcesses,
            Instant::now(),
            events,
            requests,
            || Sensor::Probe(probe.take().unwrap()),
        );
    });
    let request = || Request::Sample {
        generation: 1,
        baseline_generation: 1,
        resume_generation: 0,
        requested_at_ms: 0,
        demand: Demand {
            active: true,
            detailed: true,
            ..Default::default()
        },
    };
    processes.send(request()).unwrap();
    starts.recv_timeout(Duration::from_secs(3)).unwrap();
    let at = Instant::now();
    overview.send(request()).unwrap();
    let observed = completions.recv_timeout(Duration::from_secs(10));
    let elapsed_us = at.elapsed().as_micros();
    proceed.send(()).unwrap();
    drop(processes);
    drop(overview);
    worker.join().unwrap();
    let SamplingEvent::Completed(completion) =
        observed.expect("overview must complete before process enumeration is released")
    else {
        panic!("expected memory completion")
    };
    assert_eq!(completion.task, Task::Memory);
    let Observation::Memory(snapshot) = completion.result.unwrap() else {
        panic!("expected memory overview")
    };
    assert!(snapshot.memory.total_bytes > 0);
    assert!(snapshot.processes.is_none());
    println!("native_memory_isolation overview_us={elapsed_us} blocked_task=MemoryProcesses");
}
