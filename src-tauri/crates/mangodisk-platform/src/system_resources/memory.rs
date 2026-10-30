use std::path::PathBuf;

use sysinfo::{MemoryRefreshKind, System};
#[cfg(not(any(windows, target_os = "macos")))]
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, UpdateKind};

use crate::{PlatformError, PlatformErrorCode, PlatformResult};

#[derive(Debug, Clone)]
pub struct ProcessMemory {
    pub pid: u32,
    pub name: String,
    pub executable: Option<PathBuf>,
    pub used_bytes: Option<u64>,
    pub is_application: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ProcessMemoryKind {
    PhysicalFootprint,
    PrivateWorkingSet,
    ResidentSet,
}
impl ProcessMemoryKind {
    pub fn native() -> Self {
        if cfg!(target_os = "macos") {
            Self::PhysicalFootprint
        } else if cfg!(windows) {
            Self::PrivateWorkingSet
        } else {
            Self::ResidentSet
        }
    }
}

/// Native system pressure, distinct from RAM utilization or free-memory percentages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum MemoryPressure {
    Unsupported,
    Unavailable,
    Normal,
    Warning,
    Critical,
}

#[derive(Debug, Clone)]
pub struct NativeMemorySnapshot {
    pub total_bytes: u64,
    pub used_bytes: u64,
    pub free_bytes: u64,
    pub swap_used_bytes: u64,
    pub pressure: MemoryPressure,
    pub process_memory_kind: ProcessMemoryKind,
    /// None means no process enumeration was requested, not an empty process list.
    pub processes: Option<Vec<ProcessMemory>>,
}

/// Allows deterministic Core tests without replacing native APIs in production.
pub trait MemorySource: Send {
    fn sample(&mut self, include_processes: bool) -> PlatformResult<NativeMemorySnapshot>;
}

pub struct MemorySampler {
    #[cfg(target_os = "macos")]
    pressure: super::memory_macos::PressureReader,
    system: System,
    processes: Option<ProcessMemorySampler>,
}

impl Default for MemorySampler {
    fn default() -> Self {
        // Do not load CPU topology, disks, users, or process command lines at startup.
        Self {
            system: System::new(),
            #[cfg(target_os = "macos")]
            pressure: Default::default(),
            processes: None,
        }
    }
}

impl MemorySource for MemorySampler {
    fn sample(&mut self, include_processes: bool) -> PlatformResult<NativeMemorySnapshot> {
        self.system
            .refresh_memory_specifics(MemoryRefreshKind::everything());
        if self.system.total_memory() == 0 {
            return Err(PlatformError::new(
                PlatformErrorCode::OperationFailed,
                "system memory counters are unavailable",
            ));
        }
        let processes = if include_processes {
            Some(
                self.processes
                    .get_or_insert_with(Default::default)
                    .sample_processes()?
                    .processes,
            )
        } else {
            None
        };
        #[cfg(target_os = "macos")]
        let (used_bytes, free_bytes) = super::memory_macos::overview(self.system.total_memory())?;
        #[cfg(not(target_os = "macos"))]
        let (used_bytes, free_bytes) = (self.system.used_memory(), self.system.free_memory());
        Ok(NativeMemorySnapshot {
            total_bytes: self.system.total_memory(),
            used_bytes,
            free_bytes,
            swap_used_bytes: self.system.used_swap(),
            #[cfg(target_os = "macos")]
            pressure: self.pressure.read(),
            #[cfg(not(target_os = "macos"))]
            pressure: MemoryPressure::Unsupported,
            process_memory_kind: ProcessMemoryKind::native(),
            processes,
        })
    }
}

pub struct NativeProcessMemorySnapshot {
    pub kind: ProcessMemoryKind,
    pub processes: Vec<ProcessMemory>,
}

pub trait ProcessMemorySource: Send {
    fn sample_processes(&mut self) -> PlatformResult<NativeProcessMemorySnapshot>;
}

/// Process enumeration owns no dependency on machine-wide memory counters.
#[derive(Default)]
pub struct ProcessMemorySampler {
    #[cfg(target_os = "macos")]
    native: super::process_snapshot_macos::ProcessSnapshotReader,
    #[cfg(windows)]
    native: super::process_snapshot_windows::ProcessSnapshotReader,
    #[cfg(not(any(windows, target_os = "macos")))]
    system: System,
}
impl ProcessMemorySource for ProcessMemorySampler {
    fn sample_processes(&mut self) -> PlatformResult<NativeProcessMemorySnapshot> {
        #[cfg(windows)]
        let processes = self
            .native
            .read()
            .ok_or_else(|| {
                PlatformError::new(
                    PlatformErrorCode::OperationFailed,
                    "private working set counters unavailable",
                )
            })?
            .into_iter()
            .map(|row| ProcessMemory {
                pid: row.counter.pid,
                name: row.counter.name,
                executable: row.counter.executable,
                used_bytes: Some(row.private_working_set_bytes),
                is_application: false,
            })
            .collect();
        #[cfg(target_os = "macos")]
        let processes = self
            .native
            .read()?
            .into_iter()
            .map(|row| ProcessMemory {
                pid: row.counter.pid,
                name: row.counter.name,
                executable: row.counter.executable,
                used_bytes: row.footprint,
                is_application: false,
            })
            .collect::<Vec<_>>();
        #[cfg(not(any(windows, target_os = "macos")))]
        let processes = {
            // RSS platforms need one process pass for identity and resident memory.
            let refresh = ProcessRefreshKind::nothing()
                // Linux tasks share process RSS and must not become application rows.
                .without_tasks()
                .with_exe(UpdateKind::OnlyIfNotSet)
                .with_memory();
            self.system
                .refresh_processes_specifics(ProcessesToUpdate::All, true, refresh);
            self.system
                .processes()
                .iter()
                .map(|(pid, process)| ProcessMemory {
                    pid: pid.as_u32(),
                    name: process.name().to_string_lossy().into_owned(),
                    executable: process.exe().map(PathBuf::from),
                    used_bytes: Some(process.memory()),
                    is_application: false,
                })
                .collect::<Vec<_>>()
        };
        #[cfg(target_os = "macos")]
        let processes = {
            let mut rows = processes;
            super::memory_macos::fill_application_metadata(&mut rows);
            rows
        };
        Ok(NativeProcessMemorySnapshot {
            kind: ProcessMemoryKind::native(),
            processes,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "linux")]
    #[test]
    fn process_snapshots_do_not_count_threads_as_separate_processes() {
        use super::super::process_cpu::{ProcessCpuSampler, ProcessCpuSource};
        use std::sync::{mpsc, Arc, Barrier};

        let barrier = Arc::new(Barrier::new(5));
        let (sender, receiver) = mpsc::channel();
        let workers = (0..4)
            .map(|_| {
                let barrier = Arc::clone(&barrier);
                let sender = sender.clone();
                std::thread::spawn(move || {
                    let path = std::fs::read_link("/proc/thread-self").unwrap();
                    let tid = path
                        .file_name()
                        .unwrap()
                        .to_str()
                        .unwrap()
                        .parse::<u32>()
                        .unwrap();
                    sender.send(tid).unwrap();
                    barrier.wait();
                })
            })
            .collect::<Vec<_>>();
        let tids = (0..4).map(|_| receiver.recv().unwrap()).collect::<Vec<_>>();
        let memory = MemorySampler::default().sample(true);
        let cpu = ProcessCpuSampler::default().sample();
        // Release workers before asserting so a failed sample cannot strand test threads.
        barrier.wait();
        for worker in workers {
            worker.join().unwrap();
        }
        let memory = memory.unwrap().processes.unwrap();
        let cpu = cpu.unwrap().processes;
        assert!(memory.iter().any(|row| row.pid == std::process::id()));
        assert!(cpu.iter().any(|row| row.pid == std::process::id()));
        assert!(
            memory.iter().all(|row| !tids.contains(&row.pid)),
            "thread RSS must not be counted as process memory"
        );
        assert!(
            cpu.iter().all(|row| !tids.contains(&row.pid)),
            "thread CPU must not be counted twice with process CPU"
        );
    }

    #[test]
    fn native_sampling_can_skip_processes_and_observe_this_process() {
        let mut sampler = MemorySampler::default();
        let overview = sampler
            .sample(false)
            .expect("memory counters should be readable");
        assert!(overview.total_bytes > 0);
        assert!(overview.processes.is_none());
        let detailed = sampler
            .sample(true)
            .expect("process sampling should complete");
        assert!(detailed.processes.unwrap().iter().any(|process| {
            process.pid == std::process::id() && process.used_bytes.is_some_and(|bytes| bytes > 0)
        }));
    }
}
