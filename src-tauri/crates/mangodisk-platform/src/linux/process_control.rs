use std::{
    collections::{BTreeSet, HashMap},
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

use crate::{
    ApplicationProcessCloseMode, ApplicationProcessCloseResult, ApplicationProcessTarget,
    PlatformError, PlatformResult,
};

const GRACEFUL_CLOSE_TIMEOUT: Duration = Duration::from_secs(5);
const FORCE_CLOSE_TIMEOUT: Duration = Duration::from_secs(2);
const CLOSE_POLL_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProcessInstance {
    pid: i32,
    executable_name: String,
    executable_path: Option<PathBuf>,
    start_time_ticks: u64,
}

pub(crate) fn close(
    target: &ApplicationProcessTarget,
    mode: ApplicationProcessCloseMode,
) -> PlatformResult<ApplicationProcessCloseResult> {
    close_many(std::slice::from_ref(target), mode)
        .into_iter()
        .next()
        .expect("a single process-close target must produce one result")
}

pub(crate) fn close_many(
    targets: &[ApplicationProcessTarget],
    mode: ApplicationProcessCloseMode,
) -> Vec<PlatformResult<ApplicationProcessCloseResult>> {
    if targets.is_empty() {
        return Vec::new();
    }
    let validation_errors = targets
        .iter()
        .map(|target| validate_target(target).err())
        .collect::<Vec<_>>();
    if validation_errors.iter().all(Option::is_some) {
        return validation_errors
            .into_iter()
            .map(|error| Err(error.expect("every target was invalid")))
            .collect();
    }
    let initial_snapshot = match process_snapshot() {
        Ok(snapshot) => snapshot,
        Err(error) => {
            return validation_errors
                .into_iter()
                .map(|validation_error| Err(validation_error.unwrap_or_else(|| error.clone())))
                .collect();
        }
    };
    let matched = targets
        .iter()
        .zip(&validation_errors)
        .map(|(target, error)| {
            if error.is_none() {
                matching_processes(target, &initial_snapshot)
            } else {
                Vec::new()
            }
        })
        .collect::<Vec<_>>();
    let signal = match mode {
        ApplicationProcessCloseMode::Graceful => libc::SIGTERM,
        ApplicationProcessCloseMode::Force => libc::SIGKILL,
    };
    let mut requests = HashMap::new();
    for (target, processes) in targets.iter().zip(&matched) {
        for process in processes {
            requests
                .entry(process.pid)
                .or_insert_with(|| request_close(process, target, signal));
        }
    }

    let timeout = match mode {
        ApplicationProcessCloseMode::Graceful => GRACEFUL_CLOSE_TIMEOUT,
        ApplicationProcessCloseMode::Force => FORCE_CLOSE_TIMEOUT,
    };
    let deadline = Instant::now() + timeout;
    let final_snapshot = loop {
        let snapshot = match process_snapshot() {
            Ok(snapshot) => snapshot,
            Err(error) => {
                return validation_errors
                    .into_iter()
                    .map(|validation_error| Err(validation_error.unwrap_or_else(|| error.clone())))
                    .collect();
            }
        };
        let has_remaining = targets
            .iter()
            .zip(&validation_errors)
            .any(|(target, error)| {
                error.is_none() && !matching_processes(target, &snapshot).is_empty()
            });
        if !has_remaining || Instant::now() >= deadline {
            break snapshot;
        }
        thread::sleep(CLOSE_POLL_INTERVAL);
    };

    targets
        .iter()
        .zip(validation_errors)
        .zip(matched)
        .map(|((target, validation_error), matched)| {
            if let Some(error) = validation_error {
                return Err(error);
            }
            let remaining = matching_processes(target, &final_snapshot);
            Ok(ApplicationProcessCloseResult {
                matched_process_count: matched.len() as u64,
                requested_process_count: matched
                    .iter()
                    .filter(|process| requests.get(&process.pid).copied().unwrap_or(false))
                    .count() as u64,
                remaining_processes: remaining
                    .into_iter()
                    .map(|process| process.executable_name)
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect(),
            })
        })
        .collect()
}

fn validate_target(target: &ApplicationProcessTarget) -> PlatformResult<()> {
    if target.executable_names.is_empty() && target.executable_paths.is_empty() {
        return Err(PlatformError::operation_failed(
            "application process target contains no identity",
        ));
    }
    Ok(())
}

fn process_snapshot() -> PlatformResult<Vec<ProcessInstance>> {
    let entries = std::fs::read_dir("/proc")
        .map_err(|error| PlatformError::io("read Linux process table", &error))?;
    let effective_uid = unsafe { libc::geteuid() };
    let mut processes = Vec::new();
    for entry in entries {
        let Ok(entry) = entry else {
            continue;
        };
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<i32>().ok())
        else {
            continue;
        };
        if pid == std::process::id() as i32
            || entry
                .metadata()
                .map_or(true, |metadata| metadata.uid() != effective_uid)
        {
            continue;
        }
        let Ok(executable_name) = std::fs::read_to_string(entry.path().join("comm")) else {
            continue;
        };
        let Ok(stat) = std::fs::read_to_string(entry.path().join("stat")) else {
            continue;
        };
        let Some(start_time_ticks) = parse_start_time_ticks(&stat) else {
            continue;
        };
        processes.push(ProcessInstance {
            pid,
            executable_name: executable_name.trim_end().to_string(),
            executable_path: std::fs::read_link(entry.path().join("exe")).ok(),
            start_time_ticks,
        });
    }
    Ok(processes)
}

fn matching_processes(
    target: &ApplicationProcessTarget,
    snapshot: &[ProcessInstance],
) -> Vec<ProcessInstance> {
    snapshot
        .iter()
        .filter(|process| {
            if target.executable_paths.is_empty() {
                target
                    .executable_names
                    .iter()
                    .any(|name| name == &process.executable_name)
            } else {
                process.executable_path.as_deref().is_some_and(|path| {
                    target
                        .executable_paths
                        .iter()
                        .any(|target_path| paths_equal(target_path, path))
                })
            }
        })
        .cloned()
        .collect()
}

fn request_close(
    captured: &ProcessInstance,
    target: &ApplicationProcessTarget,
    signal: i32,
) -> bool {
    let Some(current) = process_instance(captured.pid) else {
        return false;
    };
    if current.start_time_ticks != captured.start_time_ticks
        || current.executable_name != captured.executable_name
        || matching_processes(target, std::slice::from_ref(&current)).is_empty()
    {
        return false;
    }
    unsafe { libc::kill(captured.pid, signal) == 0 }
}

fn process_instance(pid: i32) -> Option<ProcessInstance> {
    let root = PathBuf::from(format!("/proc/{pid}"));
    let metadata = std::fs::metadata(&root).ok()?;
    if metadata.uid() != unsafe { libc::geteuid() } {
        return None;
    }
    let executable_name = std::fs::read_to_string(root.join("comm")).ok()?;
    let stat = std::fs::read_to_string(root.join("stat")).ok()?;
    Some(ProcessInstance {
        pid,
        executable_name: executable_name.trim_end().to_string(),
        executable_path: std::fs::read_link(root.join("exe")).ok(),
        start_time_ticks: parse_start_time_ticks(&stat)?,
    })
}

fn parse_start_time_ticks(stat: &str) -> Option<u64> {
    let fields_after_name = stat.rsplit_once(") ")?.1;
    fields_after_name.split_whitespace().nth(19)?.parse().ok()
}

fn paths_equal(left: &Path, right: &Path) -> bool {
    normalize_path(left) == normalize_path(right)
}

fn normalize_path(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_start_time_after_a_process_name_with_spaces_and_parentheses() {
        let stat =
            "42 (name with ) parenthesis) S 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 98765 20";
        assert_eq!(parse_start_time_ticks(stat), Some(98_765));
    }

    #[test]
    fn exact_paths_disable_same_name_fallback() {
        let snapshot = vec![ProcessInstance {
            pid: 42,
            executable_name: "shared-helper".into(),
            executable_path: Some(PathBuf::from("/opt/other/shared-helper")),
            start_time_ticks: 10,
        }];
        let target = ApplicationProcessTarget {
            executable_names: vec!["shared-helper".into()],
            executable_paths: vec![PathBuf::from("/opt/expected/shared-helper")],
        };

        assert!(matching_processes(&target, &snapshot).is_empty());
    }

    #[test]
    fn empty_targets_are_rejected() {
        assert!(validate_target(&ApplicationProcessTarget::default()).is_err());
    }
}
