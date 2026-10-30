use super::super::sampling_workers::Observation;
use super::*;
use std::sync::mpsc::{self, Receiver};

fn fixture() -> (PollingWorkers, Vec<(Task, Receiver<Request>)>) {
    let mut receivers = Vec::new();
    let workers = Task::WORKERS.map(|task| {
        let (requests, receiver) = mpsc::sync_channel(1);
        receivers.push((task, receiver));
        Worker {
            task,
            requests,
            slot: Default::default(),
            closed_until_ms: None,
            thread: None,
        }
    });
    (
        PollingWorkers {
            workers,
            resume_generation: 0,
            origin: Instant::now(),
            events: mpsc::sync_channel(8).0,
        },
        receivers,
    )
}
fn completion(task: Task, generation: u64) -> Completion {
    Completion {
        task,
        generation,
        monotonic_ms: 0,
        timestamp_ms: 0,
        duration_ms: 0,
        queue_ms: 0,
        result: Ok(Observation::Unsupported),
    }
}
fn receive(receivers: &[(Task, Receiver<Request>)], task: Task) -> Request {
    receivers
        .iter()
        .find(|(other, _)| *other == task)
        .unwrap()
        .1
        .try_recv()
        .expect("expected bounded request")
}
fn metrics() -> [Demand; 5] {
    std::array::from_fn(|_| Demand::default())
}

#[test]
fn worker_failure_does_not_stop_other_sources() {
    let (mut workers, mut receivers) = fixture();
    let dead = receivers
        .iter()
        .position(|(task, _)| *task == Task::Gpu)
        .unwrap();
    drop(receivers.remove(dead));
    let mut demands = metrics();
    demands[0].active = true;
    demands[1].active = true;
    demands[2].active = true;
    let failed = workers.dispatch(&demands, &mut ResourceCache::default(), 0);
    assert_eq!(failed, vec![Task::Gpu]);
    assert!(workers.wait_ms(1000) > 0);
    assert!(matches!(
        receive(&receivers, Task::Memory),
        Request::Sample { .. }
    ));
}

#[test]
fn full_queue_defers_one_task_without_blocking_other_tasks() {
    let (mut workers, receivers) = fixture();
    workers
        .workers
        .iter()
        .find(|worker| worker.task == Task::Gpu)
        .unwrap()
        .requests
        .send(Request::Release)
        .unwrap();
    let mut demands = metrics();
    demands[0].active = true;
    demands[1].active = true;
    workers.dispatch(&demands, &mut ResourceCache::default(), 0);
    assert!(matches!(
        receive(&receivers, Task::Cpu),
        Request::Sample { .. }
    ));
    assert!(workers.slot(Task::Gpu).begin(249, 2000).is_none());
    assert!(workers.slot(Task::Gpu).begin(250, 2000).is_some());
}

#[test]
fn resume_rejects_old_results_and_waits_for_the_old_native_query_to_exit() {
    let (mut workers, receivers) = fixture();
    let mut demands = metrics();
    demands[0].active = true;
    let mut cache = ResourceCache::default();
    workers.dispatch(&demands, &mut cache, 0);
    let Request::Sample {
        generation,
        baseline_generation,
        ..
    } = receive(&receivers, Task::Cpu)
    else {
        panic!("expected sample")
    };
    workers.resume();
    workers.dispatch(&demands, &mut cache, 0);
    assert!(receivers
        .iter()
        .find(|(task, _)| *task == Task::Cpu)
        .unwrap()
        .1
        .try_recv()
        .is_err());
    assert!(!workers.accept(&completion(Task::Cpu, generation), &demands, &mut cache, 0));
    workers.dispatch(&demands, &mut cache, 0);
    let Request::Sample {
        generation: next,
        baseline_generation: next_baseline,
        resume_generation,
        ..
    } = receive(&receivers, Task::Cpu)
    else {
        panic!("expected resumed sample")
    };
    assert_ne!(next_baseline, baseline_generation);
    assert_eq!(resume_generation, 1);
    assert!(workers.accept(&completion(Task::Cpu, next), &demands, &mut cache, 200));
    assert!(workers.slot(Task::Cpu).begin(2199, 2000).is_none());
    assert!(workers.slot(Task::Cpu).begin(2200, 2000).is_some());
}

#[test]
fn frequency_visibility_never_invalidates_an_inflight_temperature() {
    let (mut workers, receivers) = fixture();
    let mut demands = metrics();
    demands[0] = Demand {
        active: true,
        temperature: true,
        ..Default::default()
    };
    let mut cache = ResourceCache::default();
    workers.dispatch(&demands, &mut cache, 0);
    let Request::Sample { generation, .. } = receive(&receivers, Task::CpuTemperature) else {
        panic!("expected temperature")
    };
    demands[0].frequency = true;
    workers.dispatch(&demands, &mut cache, 1);
    assert!(matches!(
        receive(&receivers, Task::CpuFrequency),
        Request::Sample { .. }
    ));
    assert!(workers.accept(
        &completion(Task::CpuTemperature, generation),
        &demands,
        &mut cache,
        1
    ));
}

#[test]
fn battery_preparation_stays_slow_and_opening_power_requests_a_live_observation() {
    let (mut workers, receivers) = fixture();
    let mut demands = metrics();
    demands[0] = Demand {
        active: true,
        prepare_battery: true,
        ..Default::default()
    };
    let mut cache = ResourceCache::default();
    workers.dispatch(&demands, &mut cache, 0);
    let Request::Sample { generation, .. } = receive(&receivers, Task::Battery) else {
        panic!("expected preparation")
    };
    assert!(workers.accept(
        &completion(Task::Battery, generation),
        &demands,
        &mut cache,
        0
    ));
    workers.dispatch(&demands, &mut cache, 29999);
    assert!(receivers
        .iter()
        .find(|(task, _)| *task == Task::Battery)
        .unwrap()
        .1
        .try_recv()
        .is_err());
    demands[0].battery = true;
    workers.dispatch(&demands, &mut cache, 29999);
    let Request::Sample { generation, .. } = receive(&receivers, Task::Battery) else {
        panic!("opening Power must sample immediately")
    };
    demands[0].active = false;
    assert!(!workers.accept(
        &completion(Task::Battery, generation),
        &demands,
        &mut cache,
        30000
    ));
    assert!(matches!(
        receive(&receivers, Task::Battery),
        Request::Release
    ));
}

#[test]
fn an_exited_inflight_worker_becomes_restartable_without_waiting_for_a_completion() {
    let (mut workers, receivers) = fixture();
    let mut demands = metrics();
    demands[0].active = true;
    let mut cache = ResourceCache::default();
    workers.dispatch(&demands, &mut cache, 0);
    let Request::Sample { generation, .. } = receive(&receivers, Task::Cpu) else {
        panic!("expected sample")
    };
    // Simulate the lifecycle observation produced by a finished native thread.
    let thread = std::thread::spawn(|| ());
    while !thread.is_finished() {
        std::thread::yield_now();
    }
    workers
        .workers
        .iter_mut()
        .find(|worker| worker.task == Task::Cpu)
        .unwrap()
        .thread = Some(thread);
    drop(receivers);
    let failed = workers.dispatch(&demands, &mut cache, 1000);
    assert_eq!(
        failed,
        vec![Task::Cpu],
        "a dead in-flight worker cannot send its completion"
    );
    assert!(!workers.accept(
        &completion(Task::Cpu, generation),
        &demands,
        &mut cache,
        1001
    ));
}
