//! Reproducible sensor-cost probe; emits aggregate measurements without process identities.
//! `cpu-memory <ticks> <interval> --burst` isolates process queries without sleeping or
//! reading global CPU, for cost-per-query comparisons above Windows CPU timer granularity.
use mangodisk_platform::system_resources::{
    cpu::{details::CpuDetailsReader, CpuReader},
    gpu::GpuReader,
    memory::{MemorySampler, MemorySource},
    network::NetworkReader,
    process_cpu::{ProcessCpuSampler, ProcessCpuSource},
};
use std::time::{Duration, Instant};
use sysinfo::{Pid, System};
#[cfg(not(windows))]
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mode = args.get(1).map(String::as_str).unwrap_or("overview");
    let seconds: u64 = args.get(2).map(|s| s.parse().unwrap()).unwrap_or(60);
    assert!(matches!(
        mode,
        "overview"
            | "overview-temperature"
            | "memory"
            | "cpu"
            | "cpu-background"
            | "cpu-memory"
            | "network"
            | "gpu"
            | "gpu-detail"
            | "overview-gpu"
    ));
    assert!((10..=3600).contains(&seconds));
    let process_interval: u64 = args.get(3).map(|s| s.parse().unwrap()).unwrap_or(2);
    assert!([2, 4].contains(&process_interval));
    let burst = args.get(4).is_some_and(|arg| arg == "--burst");
    assert!(!burst || mode == "cpu-memory");
    let mut cpu: CpuReader = Default::default();
    let mut cpu_details = CpuDetailsReader::default();
    let temperature_queries = std::cell::Cell::new(0u64);
    let temperature_ready = std::cell::Cell::new(0u64);
    let mut memory = MemorySampler::default();
    let mut gpu = GpuReader::default();
    let mut network = NetworkReader::default();
    let mut processes = (mode.starts_with("cpu")).then(ProcessCpuSampler::default);
    let mut observer = System::new();
    let pid = Pid::from_u32(std::process::id());
    let refresh = |observer: &mut System| own_usage(observer, pid);
    let mut sample = |cpu: &mut CpuReader, memory: &mut MemorySampler, tick: u64| {
        if mode == "overview-temperature" && tick.is_multiple_of(2) {
            if let Some(result) = cpu_details.read(false, true).temperature {
                temperature_queries.set(temperature_queries.get() + 1);
                temperature_ready.set(temperature_ready.get() + u64::from(result.is_ok()));
            }
        }
        if matches!(mode, "gpu" | "gpu-detail" | "overview-gpu") && tick.is_multiple_of(2) {
            if mode == "gpu-detail" {
                gpu.read_detailed()
            } else {
                gpu.read()
            }
            .expect("GPU counters are readable");
        }
        if matches!(mode, "gpu" | "gpu-detail") {
            return;
        }
        if mode == "network" {
            network.read().expect("network counters are readable");
            return;
        }
        if !burst && tick.is_multiple_of(2) {
            cpu.read().expect("CPU overview is readable");
        }
        if let Some(sampler) = processes.as_mut() {
            if tick.is_multiple_of(process_interval) {
                sampler.sample().expect("process CPU is readable");
            }
        }
        if tick.is_multiple_of(3) {
            memory
                .sample(matches!(mode, "memory" | "cpu-memory"))
                .expect("memory is readable");
        }
    };
    let cold_started = Instant::now();
    sample(&mut cpu, &mut memory, 0);
    let cold_sample_micros = cold_started.elapsed().as_micros() as u64;
    if !burst {
        std::thread::sleep(Duration::from_secs(1));
    }
    for tick in 1..8 {
        sample(&mut cpu, &mut memory, tick);
        if !burst {
            std::thread::sleep(Duration::from_secs(1));
        }
    }
    let (before_cpu, before_rss) = refresh(&mut observer);
    let before_helpers = helper_cpu_ms();
    let before_temperature_queries = temperature_queries.get();
    let before_temperature_ready = temperature_ready.get();
    let started = Instant::now();
    let mut durations = Vec::new();
    let mut rss_min = before_rss;
    let mut rss_max = before_rss;
    for tick in 0..seconds {
        let at = Instant::now();
        sample(&mut cpu, &mut memory, tick);
        // GPU-only modes have idle ticks between reads. Excluding them keeps
        // latency percentiles representative of actual sensor calls.
        if !matches!(mode, "gpu" | "gpu-detail") || tick.is_multiple_of(2) {
            durations.push(at.elapsed().as_micros() as u64);
        }
        let (_, rss) = refresh(&mut observer);
        rss_min = rss_min.min(rss);
        rss_max = rss_max.max(rss);
        if !burst {
            std::thread::sleep(Duration::from_secs(1).saturating_sub(at.elapsed()));
        }
    }
    let (after_cpu, after_rss) = refresh(&mut observer);
    let helper_cpu = helper_cpu_ms().saturating_sub(before_helpers);
    durations.sort_unstable();
    println!(
        "{}",
        serde_json::json!({
            "schemaVersion": 1, "mode": mode, "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH, "samples": durations.len(),
            "measurementTicks": seconds,
            "temperatureQueries": temperature_queries.get() - before_temperature_queries,
            "temperatureReadySamples": temperature_ready.get() - before_temperature_ready,
            "burst": burst, "cpuMilliseconds": after_cpu - before_cpu,
            "processIntervalSeconds": process_interval,
            "coldSampleMicros": cold_sample_micros,
            "gpuIntervalSeconds": if matches!(mode, "gpu" | "gpu-detail" | "overview-gpu") { Some(2) } else { None },
            "networkIntervalSeconds": if mode == "network" { Some(1) } else { None },
            "processQueries": if mode.starts_with("cpu") { seconds.div_ceil(process_interval) } else { 0 },
            "elapsedSeconds": started.elapsed().as_secs_f64(),
            "cpuSingleCorePercent": (after_cpu - before_cpu) as f64 / started.elapsed().as_secs_f64() / 10.0,
            "helperCpuSingleCorePercent": helper_cpu as f64 / started.elapsed().as_secs_f64() / 10.0,
            "cpuIncludingHelpersSingleCorePercent": (after_cpu - before_cpu + helper_cpu) as f64 / started.elapsed().as_secs_f64() / 10.0,
            "rssStartBytes": before_rss, "rssEndBytes": after_rss,
            "rssMinBytes": rss_min, "rssMaxBytes": rss_max,
            "sampleP50Micros": durations[durations.len() / 2],
            "sampleP95Micros": durations[(durations.len() - 1) * 95 / 100],
            "sampleMaxMicros": durations[durations.len() - 1],
        })
    );
}

#[cfg(not(windows))]
fn own_usage(observer: &mut System, pid: Pid) -> (u64, u64) {
    observer.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[pid]),
        true,
        ProcessRefreshKind::nothing().with_cpu().with_memory(),
    );
    let process = observer.process(pid).expect("probe process is readable");
    (process.accumulated_cpu_time(), process.memory())
}

#[cfg(windows)]
fn own_usage(_observer: &mut System, _pid: Pid) -> (u64, u64) {
    use windows_sys::Win32::{
        Foundation::FILETIME,
        System::{
            ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS},
            Threading::{GetCurrentProcess, GetProcessTimes},
        },
    };
    let mut created = FILETIME::default();
    let mut exited = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    let mut memory = PROCESS_MEMORY_COUNTERS::default();
    memory.cb = std::mem::size_of_val(&memory) as u32;
    // Observe only this process with public counters. A full sysinfo process refresh
    // would add unrelated enumeration work to the Windows sensor-cost measurement.
    unsafe {
        let process = GetCurrentProcess();
        assert_ne!(
            GetProcessTimes(process, &mut created, &mut exited, &mut kernel, &mut user),
            0
        );
        assert_ne!(GetProcessMemoryInfo(process, &mut memory, memory.cb), 0);
    }
    let time =
        |value: FILETIME| (u64::from(value.dwHighDateTime) << 32) | u64::from(value.dwLowDateTime);
    (
        (time(kernel) + time(user)) / 10_000,
        memory.WorkingSetSize as u64,
    )
}

#[cfg(target_os = "macos")]
fn helper_cpu_ms() -> u64 {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::zeroed();
    if unsafe { libc::getrusage(libc::RUSAGE_CHILDREN, usage.as_mut_ptr()) } != 0 {
        return 0;
    }
    let usage = unsafe { usage.assume_init() };
    let millis = |time: libc::timeval| time.tv_sec as u64 * 1000 + time.tv_usec as u64 / 1000;
    millis(usage.ru_utime) + millis(usage.ru_stime)
}
#[cfg(not(target_os = "macos"))]
fn helper_cpu_ms() -> u64 {
    0
}
