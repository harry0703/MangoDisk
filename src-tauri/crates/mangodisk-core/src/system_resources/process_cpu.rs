//! Application CPU deltas: fresh intervals only, bounded by the current process set.
use super::{
    application_identity,
    models::{ApplicationCpu, CpuProcess, ProcessCpuSummary},
};
use crate::{applications::running_identity, CoreResult};
use mangodisk_platform::system_resources::process_cpu::{
    CpuUsageScale, ProcessCpuSampler, ProcessCpuSnapshot, ProcessCpuSource, ProcessLocationStatus,
};
use std::collections::HashMap;

#[derive(Default)]
pub struct ProcessCpuDelta {
    previous: HashMap<(u32, u64), u64>,
    baseline: Option<(u64, usize, CpuUsageScale)>,
}
impl ProcessCpuDelta {
    pub fn observe(
        &mut self,
        snapshot: ProcessCpuSnapshot,
        monotonic_ms: u64,
        current_pid: u32,
    ) -> Option<ProcessCpuSummary> {
        self.observe_grouped(snapshot, monotonic_ms, current_pid, cfg!(windows))
    }

    fn observe_grouped(
        &mut self,
        snapshot: ProcessCpuSnapshot,
        monotonic_ms: u64,
        current_pid: u32,
        grouped: bool,
    ) -> Option<ProcessCpuSummary> {
        let interval = self
            .baseline
            .replace((
                monotonic_ms,
                snapshot.logical_cpu_count,
                snapshot.usage_scale,
            ))
            .and_then(|(before, cpus, scale)| {
                monotonic_ms.checked_sub(before).filter(|delta| {
                    (200..=5000).contains(delta)
                        && cpus == snapshot.logical_cpu_count
                        && cpus > 0
                        && scale == snapshot.usage_scale
                })
            });
        let capacity = match snapshot.usage_scale {
            CpuUsageScale::SingleCore => snapshot.logical_cpu_count as f64 * 100.0,
            CpuUsageScale::TotalCapacity => 100.0,
        };
        let divisor = match snapshot.usage_scale {
            CpuUsageScale::SingleCore => 1.0,
            CpuUsageScale::TotalCapacity => snapshot.logical_cpu_count as f64,
        };
        let own_path = snapshot
            .processes
            .iter()
            .find(|p| p.pid == current_pid)
            .and_then(|p| p.executable.as_deref())
            .map(running_identity::application_path);
        let mut next = HashMap::with_capacity(snapshot.processes.len());
        let mut applications = Vec::<ApplicationCpu>::new();
        let mut readable_process_count = 0;
        let mut omitted_process_count = 0;
        for p in snapshot.processes {
            let key = (p.pid, p.started_at);
            let delta = p.cpu_time_ms.and_then(|time| {
                self.previous
                    .get(&key)
                    .and_then(|old| time.checked_sub(*old))
            });
            if let Some(time) = p.cpu_time_ms {
                next.insert(key, time);
            }
            let Some((delta, interval)) = delta.zip(interval) else {
                omitted_process_count += 1;
                continue;
            };
            if p.name.trim().is_empty() {
                omitted_process_count += 1;
                continue;
            }
            readable_process_count += 1;
            let process_name = p.name.clone();
            let mut application = application_identity::identify(
                p.pid,
                p.name,
                p.executable.as_deref(),
                own_path.as_deref(),
            );
            // Windows uses the same executable identity as memory; macOS preserves
            // Activity Monitor's per-process presentation. CPU never offers app-wide quit.
            if !grouped || p.executable.is_none() {
                application.id = format!("cpu:{}:{}", p.pid, p.started_at);
            }
            application.name = process_name;
            application.process_count = 1;
            application.can_quit = false;
            application.can_force_quit = false;
            let used_percent =
                (delta as f64 * 100.0 / interval as f64 / divisor).clamp(0.0, capacity);
            applications.push(ApplicationCpu {
                processes: vec![CpuProcess {
                    pid: p.pid,
                    started_at: p.started_at,
                    used_percent,
                }],
                location_status: if p.executable.is_some() {
                    ProcessLocationStatus::Available
                } else {
                    p.location_status
                },
                application,
                pid: p.pid,
                used_percent,
            });
        }
        self.previous = next;
        interval.map(|_| {
            if grouped {
                applications = super::application_groups::aggregate(
                    applications
                        .into_iter()
                        .map(|row| (row.application.clone(), row)),
                    |total, row| {
                        total.used_percent += row.used_percent;
                        total.processes.extend(row.processes);
                    },
                )
                .into_iter()
                .map(|(application, mut row)| {
                    row.application = application;
                    row.processes.sort_by(|a, b| {
                        b.used_percent
                            .total_cmp(&a.used_percent)
                            .then(a.pid.cmp(&b.pid))
                    });
                    row.processes
                        .truncate(super::application_groups::RANKING_LIMIT);
                    row
                })
                .collect();
            }
            applications.sort_by(|left, right| {
                right
                    .used_percent
                    .total_cmp(&left.used_percent)
                    .then_with(|| left.application.name.cmp(&right.application.name))
                    .then_with(|| left.application.id.cmp(&right.application.id))
            });
            applications.truncate(super::application_groups::RANKING_LIMIT);
            ProcessCpuSummary {
                usage_scale: snapshot.usage_scale,
                applications,
                readable_process_count,
                omitted_process_count,
            }
        })
    }
}
pub struct ProcessCpuService<S = ProcessCpuSampler> {
    source: S,
    delta: ProcessCpuDelta,
}
impl Default for ProcessCpuService {
    fn default() -> Self {
        Self::new(ProcessCpuSampler::default())
    }
}
impl<S: ProcessCpuSource> ProcessCpuService<S> {
    pub fn new(source: S) -> Self {
        Self {
            source,
            delta: ProcessCpuDelta::default(),
        }
    }
    pub fn sample(&mut self, monotonic_ms: u64) -> CoreResult<Option<ProcessCpuSummary>> {
        let snapshot = match self.source.sample() {
            Ok(snapshot) => snapshot,
            Err(error) => {
                self.delta = ProcessCpuDelta::default();
                return Err(error.into());
            }
        };
        Ok(self
            .delta
            .observe(snapshot, monotonic_ms, std::process::id()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mangodisk_platform::system_resources::process_cpu::ProcessCpuCounter;

    fn process(
        pid: u32,
        started_at: u64,
        path: Option<&str>,
        cpu_time_ms: u64,
    ) -> ProcessCpuCounter {
        ProcessCpuCounter {
            pid,
            started_at,
            name: format!("Process {pid}"),
            executable: path.map(Into::into),
            location_status: ProcessLocationStatus::Unavailable,
            cpu_time_ms: Some(cpu_time_ms),
        }
    }
    fn snapshot(processes: Vec<ProcessCpuCounter>) -> ProcessCpuSnapshot {
        ProcessCpuSnapshot {
            logical_cpu_count: 4,
            usage_scale: CpuUsageScale::TotalCapacity,
            processes,
        }
    }
    #[test]
    fn windows_groups_all_executable_processes_before_truncating_and_retains_members() {
        let mut delta = ProcessCpuDelta::default();
        let sample = |time| {
            snapshot(
                (1..=40)
                    .map(|pid| process(pid, 1, Some("C:/Apps/Code.exe"), time))
                    .chain([
                        process(41, 1, Some("D:/Other/Code.exe"), time),
                        process(42, 1, None, time),
                        process(43, 1, None, time),
                    ])
                    .collect(),
            )
        };
        assert!(delta.observe_grouped(sample(0), 0, 99, true).is_none());
        let value = delta.observe_grouped(sample(10), 1000, 99, true).unwrap();
        assert_eq!(value.applications.len(), 4);
        assert_eq!(value.applications[0].application.process_count, 40);
        assert_eq!(value.applications[0].used_percent, 10.0);
        assert_eq!(value.applications[0].processes.len(), 40);
        assert!(!value.applications[0].application.can_quit);
        assert!(!value.applications[0].application.can_force_quit);
        assert_eq!(value.readable_process_count, 43);
    }

    #[test]
    fn member_limit_preserves_full_group_usage_and_process_count() {
        let mut delta = ProcessCpuDelta::default();
        let sample = |time| {
            snapshot(
                (1..=100)
                    .map(|pid| process(pid, 1, Some("C:/Apps/Code.exe"), time))
                    .collect(),
            )
        };
        delta.observe_grouped(sample(0), 0, 999, true);
        let value = delta.observe_grouped(sample(10), 1000, 999, true).unwrap();
        let group = &value.applications[0];
        assert_eq!(group.application.process_count, 100);
        assert_eq!(group.used_percent, 25.0);
        assert_eq!(group.processes.len(), 50);
        assert_eq!(value.readable_process_count, 100);
    }

    #[test]
    fn total_capacity_ranking_keeps_helpers_and_application_instances_separate() {
        let mut delta = ProcessCpuDelta::default();
        let rows = |offset: u64| {
            snapshot(vec![
                process(
                    1,
                    10,
                    Some("/Browser.app/Contents/MacOS/Browser"),
                    100 + offset,
                ),
                process(
                    2,
                    10,
                    Some("/Browser.app/Contents/Frameworks/Helper.app/Contents/MacOS/Helper"),
                    200 + offset,
                ),
                process(
                    3,
                    10,
                    Some("/other/Browser.app/Contents/MacOS/Browser"),
                    300,
                ),
            ])
        };
        assert!(delta.observe_grouped(rows(0), 0, 1, false).is_none());
        let value = delta.observe_grouped(rows(1000), 1000, 1, false).unwrap();
        assert_eq!(value.applications.len(), 3);
        assert_eq!(value.applications[0].used_percent, 25.0);
        assert_eq!(value.applications[0].application.process_count, 1);
        assert_eq!(value.applications[1].used_percent, 25.0);
        assert_ne!(
            value.applications[0].application.id,
            value.applications[1].application.id
        );
        assert_eq!(value.applications[0].pid, 1);
        assert!(!value.applications[0].application.can_quit);
        assert!(!value.applications[0].application.can_force_quit);
        assert_eq!(value.applications[2].used_percent, 0.0);
        let json = serde_json::to_value(value).unwrap();
        assert!(json["applications"][0]["id"].is_string());
        assert_eq!(json["applications"][0]["usedPercent"], 25.0);
        assert!(json["applications"][0].get("application").is_none());
    }
    #[test]
    fn single_core_scale_preserves_multicore_usage_and_matches_native_tools() {
        let mut delta = ProcessCpuDelta::default();
        let source = |time| {
            let mut value = snapshot(vec![process(1, 1, None, time)]);
            value.usage_scale = CpuUsageScale::SingleCore;
            value
        };
        assert!(delta.observe(source(0), 0, 99).is_none());
        let value = delta.observe(source(2500), 1000, 99).unwrap();
        assert_eq!(value.applications[0].used_percent, 250.0);
        assert_eq!(
            serde_json::to_value(&value).unwrap()["usageScale"],
            "singleCore"
        );
        let bounded = delta.observe(source(12500), 2000, 99).unwrap();
        assert_eq!(bounded.applications[0].used_percent, 400.0);
        assert!(delta
            .observe(snapshot(vec![process(1, 1, None, 13000)]), 3000, 99)
            .is_none());
    }
    #[test]
    fn pid_reuse_new_processes_and_counter_rollback_require_a_fresh_baseline() {
        let mut delta = ProcessCpuDelta::default();
        delta.observe(
            snapshot(vec![process(1, 1, None, 500), process(2, 1, None, 500)]),
            0,
            99,
        );
        let value = delta
            .observe(
                snapshot(vec![
                    process(1, 2, None, 9000),
                    process(2, 1, None, 0),
                    process(3, 1, None, 100),
                ]),
                1000,
                99,
            )
            .unwrap();
        assert!(value.applications.is_empty());
        assert_eq!(value.omitted_process_count, 3);
        let value = delta
            .observe(snapshot(vec![process(1, 2, None, 9100)]), 2000, 99)
            .unwrap();
        assert_eq!(value.readable_process_count, 1);
        assert_eq!(value.applications[0].used_percent, 2.5);
        assert_eq!(delta.previous.len(), 1);
    }
    #[test]
    fn sleep_clock_reset_short_intervals_and_topology_changes_never_fabricate_rates() {
        for at in [0, 199, 5001] {
            let mut delta = ProcessCpuDelta::default();
            delta.observe(snapshot(vec![process(1, 1, None, 0)]), 0, 99);
            assert!(delta
                .observe(snapshot(vec![process(1, 1, None, 100)]), at, 99)
                .is_none());
            assert_eq!(
                delta
                    .observe(snapshot(vec![process(1, 1, None, 200)]), at + 1000, 99)
                    .unwrap()
                    .applications[0]
                    .used_percent,
                2.5
            );
        }
        let mut delta = ProcessCpuDelta::default();
        delta.observe(snapshot(vec![]), 1000, 99);
        assert!(delta.observe(snapshot(vec![]), 999, 99).is_none());
        let mut changed = snapshot(vec![]);
        changed.logical_cpu_count = 8;
        assert!(delta.observe(changed, 1999, 99).is_none());
    }
    #[test]
    fn bounded_ranking_tracks_all_baselines_and_clamps_cpu() {
        let mut delta = ProcessCpuDelta::default();
        delta.observe(
            snapshot((1..=100).map(|pid| process(pid, 1, None, 0)).collect()),
            0,
            999,
        );
        let value = delta
            .observe(
                snapshot(
                    (1..=100)
                        .map(|pid| process(pid, 1, None, pid as u64 * 10))
                        .collect(),
                ),
                1000,
                999,
            )
            .unwrap();
        assert_eq!(value.applications.len(), 50);
        assert_eq!(value.applications[0].pid, 100);
        assert_eq!(value.applications[49].pid, 51);
        assert_eq!(value.readable_process_count, 100);
        let next = delta
            .observe(
                snapshot(
                    (1..=100)
                        .map(|pid| {
                            process(
                                pid,
                                1,
                                None,
                                pid as u64 * 10 + if pid == 1 { 5000 } else { 10 },
                            )
                        })
                        .collect(),
                ),
                2000,
                999,
            )
            .unwrap();
        assert_eq!(next.applications[0].pid, 1);
        assert_eq!(next.applications[0].used_percent, 100.0);
        assert!(value
            .applications
            .iter()
            .all(|row| (0.0..=100.0).contains(&row.used_percent)));
    }
    #[test]
    fn unreadable_counters_never_appear_idle_or_reuse_a_stale_baseline() {
        let mut delta = ProcessCpuDelta::default();
        delta.observe(snapshot(vec![process(1, 1, None, 0)]), 0, 99);
        let mut unreadable = process(1, 1, None, 1000);
        unreadable.cpu_time_ms = None;
        let value = delta.observe(snapshot(vec![unreadable]), 1000, 99).unwrap();
        assert!(value.applications.is_empty());
        assert_eq!(value.omitted_process_count, 1);
        let value = delta
            .observe(snapshot(vec![process(1, 1, None, 2000)]), 2000, 99)
            .unwrap();
        assert!(value.applications.is_empty());
        let value = delta
            .observe(snapshot(vec![process(1, 1, None, 2100)]), 3000, 99)
            .unwrap();
        assert_eq!(value.applications[0].used_percent, 2.5);
    }
    #[test]
    fn source_failure_requires_two_fresh_observations_after_recovery() {
        struct Source(
            std::collections::VecDeque<mangodisk_platform::PlatformResult<ProcessCpuSnapshot>>,
        );
        impl ProcessCpuSource for Source {
            fn sample(&mut self) -> mangodisk_platform::PlatformResult<ProcessCpuSnapshot> {
                self.0.pop_front().unwrap()
            }
        }
        let mut service = ProcessCpuService::new(Source(std::collections::VecDeque::from([
            Ok(snapshot(vec![process(1, 1, None, 0)])),
            Err(mangodisk_platform::PlatformError::new(
                mangodisk_platform::PlatformErrorCode::OperationFailed,
                "test source failed",
            )),
            Ok(snapshot(vec![process(1, 1, None, 2000)])),
            Ok(snapshot(vec![process(1, 1, None, 2100)])),
        ])));
        assert!(service.sample(0).unwrap().is_none());
        assert!(service.sample(1000).is_err());
        assert!(service.sample(2000).unwrap().is_none());
        assert_eq!(
            service.sample(3000).unwrap().unwrap().applications[0].used_percent,
            2.5
        );
    }
    #[test]
    fn native_workload_produces_nonzero_application_cpu_usage() {
        let mut service = ProcessCpuService::default();
        let start = std::time::Instant::now();
        assert!(service.sample(0).unwrap().is_none());
        let busy_at = std::time::Instant::now();
        while busy_at.elapsed() < std::time::Duration::from_millis(500) {
            std::hint::black_box(
                (0..1000).fold(1u64, |acc, n| acc.wrapping_mul(31).wrapping_add(n)),
            );
        }
        let busy = service
            .sample(start.elapsed().as_millis() as u64)
            .unwrap()
            .unwrap();
        let own = busy
            .applications
            .iter()
            .find(|row| {
                row.processes
                    .iter()
                    .any(|process| process.pid == std::process::id())
            })
            .expect("workload should be ranked");
        let busy_percent = own.used_percent;
        let cores = mangodisk_platform::system_resources::process_cpu::ProcessCpuSampler::default()
            .sample()
            .unwrap()
            .logical_cpu_count;
        assert!(
            busy_percent
                * if busy.usage_scale == CpuUsageScale::SingleCore {
                    1.0
                } else {
                    cores as f64
                }
                > 10.0,
            "native CPU units must match elapsed milliseconds"
        );
    }
}
