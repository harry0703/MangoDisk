//! Application memory rankings are independent of machine-wide memory sampling.
use super::{memory, models::ProcessMemorySummary};
use crate::CoreResult;
use mangodisk_platform::system_resources::memory::{ProcessMemorySampler, ProcessMemorySource};

pub struct ProcessMemoryService<S = ProcessMemorySampler> {
    source: S,
}
impl Default for ProcessMemoryService {
    fn default() -> Self {
        Self {
            source: Default::default(),
        }
    }
}
impl<S: ProcessMemorySource> ProcessMemoryService<S> {
    pub fn sample(&mut self) -> CoreResult<ProcessMemorySummary> {
        let snapshot = self.source.sample_processes()?;
        Ok(memory::summarize(
            snapshot.processes,
            std::process::id(),
            snapshot.kind,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mangodisk_platform::{
        system_resources::memory::{NativeProcessMemorySnapshot, ProcessMemory, ProcessMemoryKind},
        PlatformResult,
    };
    struct Source(bool);
    impl ProcessMemorySource for Source {
        fn sample_processes(&mut self) -> PlatformResult<NativeProcessMemorySnapshot> {
            let first = std::mem::replace(&mut self.0, false);
            Ok(NativeProcessMemorySnapshot {
                kind: ProcessMemoryKind::native(),
                processes: if first {
                    vec![ProcessMemory {
                        pid: 1,
                        name: "Editor".into(),
                        executable: None,
                        used_bytes: Some(100),
                        is_application: false,
                    }]
                } else {
                    vec![]
                },
            })
        }
    }
    #[test]
    fn exited_processes_replace_previous_rankings_without_querying_memory_overview() {
        let mut service = ProcessMemoryService {
            source: Source(true),
        };
        assert_eq!(service.sample().unwrap().readable_process_count, 1);
        let empty = service.sample().unwrap();
        assert_eq!(empty.readable_process_count, 0);
        assert!(empty.applications.is_empty());
    }
}
