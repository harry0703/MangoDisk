use std::{
    collections::{HashMap, HashSet},
    ffi::OsStr,
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

use objc2::rc::autoreleasepool;
use objc2_app_kit::NSRunningApplication;

use crate::{
    command::{
        run_controlled_command, ControlledCommandLimits, ControlledEnvironmentPolicy,
        ControlledExecutable,
    },
    ApplicationProcessCloseMode, ApplicationProcessCloseResult, ApplicationProcessTarget,
    PlatformError, PlatformResult,
};

const PROCESS_COMMAND_TIMEOUT: Duration = Duration::from_secs(5);
const PROCESS_COMMAND_OUTPUT_LIMIT: usize = 4 * 1024 * 1024;
const GRACEFUL_CLOSE_TIMEOUT: Duration = Duration::from_secs(5);
const FORCE_CLOSE_TIMEOUT: Duration = Duration::from_secs(2);
const CLOSE_QUIESCENCE_INTERVAL: Duration = Duration::from_millis(500);
const CLOSE_POLL_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Debug, Clone)]
struct ProcessInstance {
    pid: i32,
    executable_path: PathBuf,
    display_name: String,
}

pub(super) fn close(
    target: &ApplicationProcessTarget,
    mode: ApplicationProcessCloseMode,
) -> PlatformResult<ApplicationProcessCloseResult> {
    close_many(std::slice::from_ref(target), mode)
        .into_iter()
        .next()
        .expect("a single process-close target must produce one result")
}

/// Executes all selected targets against shared snapshots and one deadline.
/// This keeps a large confirmation selection bounded to five seconds instead
/// of multiplying the timeout and `/bin/ps` process cost by the target count.
pub(super) fn close_many(
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
        Err(error) => return snapshot_error_results(&validation_errors, error),
    };
    let mut matched = targets
        .iter()
        .zip(&validation_errors)
        .map(|(target, error)| {
            if error.is_none() {
                matching_processes_in_snapshot(target, &initial_snapshot)
            } else {
                Vec::new()
            }
        })
        .collect::<Vec<_>>();

    let mut requests = HashMap::new();
    for process in matched.iter().flatten() {
        requests
            .entry(process.pid)
            .or_insert_with(|| request_close(process, mode));
    }

    let timeout = match mode {
        ApplicationProcessCloseMode::Graceful => GRACEFUL_CLOSE_TIMEOUT,
        ApplicationProcessCloseMode::Force => FORCE_CLOSE_TIMEOUT,
    };
    let deadline = Instant::now() + timeout;
    let mut empty_since = None;
    let observed_any = matched.iter().any(|processes| !processes.is_empty());
    let final_snapshot = loop {
        let snapshot = match process_snapshot() {
            Ok(snapshot) => snapshot,
            Err(error) => return snapshot_error_results(&validation_errors, error),
        };
        let remaining = targets
            .iter()
            .zip(&validation_errors)
            .map(|(target, error)| {
                if error.is_none() {
                    matching_processes_in_snapshot(target, &snapshot)
                } else {
                    Vec::new()
                }
            })
            .collect::<Vec<_>>();
        let has_remaining = remaining.iter().any(|processes| !processes.is_empty());
        let now = Instant::now();
        if !has_remaining {
            // An explicit force action must observe a brief quiet interval so a
            // delayed helper restart cannot escape through the first empty snapshot.
            let since = empty_since.get_or_insert(now);
            if mode != ApplicationProcessCloseMode::Force
                || !observed_any
                || now.duration_since(*since) >= CLOSE_QUIESCENCE_INTERVAL
                || now >= deadline
            {
                break snapshot;
            }
        } else {
            empty_since = None;
            if now >= deadline {
                break snapshot;
            }
            if mode == ApplicationProcessCloseMode::Force {
                // Reuse the same path/owner checks for new instances. A PID is
                // attempted once, and a shared deadline bounds a restarting app.
                for (observed, processes) in matched.iter_mut().zip(&remaining) {
                    for process in processes {
                        if !observed
                            .iter()
                            .any(|candidate| candidate.pid == process.pid)
                        {
                            observed.push(process.clone());
                        }
                        requests
                            .entry(process.pid)
                            .or_insert_with(|| request_close(process, mode));
                    }
                }
            }
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
            let remaining = matching_processes_in_snapshot(target, &final_snapshot);
            Ok(ApplicationProcessCloseResult {
                matched_process_count: matched.len() as u64,
                requested_process_count: matched
                    .iter()
                    .filter(|process| requests.get(&process.pid).copied().unwrap_or(false))
                    .count() as u64,
                remaining_processes: unique_display_names(&remaining),
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

fn request_close(process: &ProcessInstance, mode: ApplicationProcessCloseMode) -> bool {
    if !can_signal_process(process) {
        log::warn!("macos_process_close_skipped pid={} path={} mode={mode:?} reason=identity_unavailable_or_changed",
            process.pid, crate::diagnostics::text(&process.executable_path.display()));
        return false;
    }

    autoreleasepool(|_| {
        if let Some(application) =
            NSRunningApplication::runningApplicationWithProcessIdentifier(process.pid)
                .filter(|application| application.bundleURL().is_some())
        {
            let accepted = match mode {
                ApplicationProcessCloseMode::Graceful => application.terminate(),
                ApplicationProcessCloseMode::Force => application.forceTerminate(),
            };
            log::info!("macos_process_close_requested pid={} path={} mode={mode:?} method=appkit accepted={accepted}",
                process.pid, crate::diagnostics::text(&process.executable_path.display()));
            if accepted || mode == ApplicationProcessCloseMode::Graceful {
                return accepted;
            }
            // Only an explicit force-close request may bypass an AppKit refusal.
            // Recheck the live executable before signaling to retain the PID reuse guard.
            if !can_signal_process(process) {
                return false;
            }
        }
        let signal = match mode {
            ApplicationProcessCloseMode::Graceful => libc::SIGTERM,
            ApplicationProcessCloseMode::Force => libc::SIGKILL,
        };
        let accepted = unsafe { libc::kill(process.pid, signal) == 0 };
        let error = if accepted {
            None
        } else {
            Some(std::io::Error::last_os_error())
        };
        log::info!("macos_process_close_requested pid={} path={} mode={mode:?} method=signal signal={signal} accepted={accepted} error={}",
            process.pid, crate::diagnostics::text(&process.executable_path.display()), crate::diagnostics::text(&format!("{error:?}")));
        accepted
    })
}

fn can_signal_process(process: &ProcessInstance) -> bool {
    process_owned_by_current_user(process.pid)
        && process.pid > 0
        && process.pid != std::process::id() as i32
        && current_executable_path(process.pid)
            .is_some_and(|path| normalize_path(&path) == normalize_path(&process.executable_path))
}

fn process_owned_by_current_user(pid: i32) -> bool {
    let mut info = std::mem::MaybeUninit::<libc::proc_bsdinfo>::zeroed();
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as i32;
    let read = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDTBSDINFO,
            0,
            info.as_mut_ptr().cast(),
            size,
        )
    };
    read == size && unsafe { info.assume_init().pbi_uid == libc::geteuid() }
}

#[cfg(test)]
fn matching_processes(target: &ApplicationProcessTarget) -> PlatformResult<Vec<ProcessInstance>> {
    process_snapshot().map(|snapshot| matching_processes_in_snapshot(target, &snapshot))
}

fn matching_processes_in_snapshot(
    target: &ApplicationProcessTarget,
    snapshot: &[ProcessInstance],
) -> Vec<ProcessInstance> {
    let names = target
        .executable_names
        .iter()
        .map(|name| normalize_name(name))
        .collect::<HashSet<_>>();
    let paths = target
        .executable_paths
        .iter()
        .map(|path| normalize_path(path))
        .collect::<Vec<_>>();
    snapshot
        .iter()
        .filter(|process| {
            process.pid != std::process::id() as i32
                && if paths.is_empty() {
                    names.contains(&normalize_name(&process.display_name))
                        || process_runs_inside_named_bundle(process, &names)
                } else {
                    paths.iter().any(|path| process_matches_path(process, path))
                }
        })
        .cloned()
        .collect()
}

/// macOS applications keep resident XPC services and helpers running under
/// their own executable names inside the owning application bundle. The
/// cleanup process guard attributes those processes to the bundle name, so
/// the close target must apply the same rule; otherwise a rule can be blocked
/// by a helper the close step reports as unmatched.
fn process_runs_inside_named_bundle(process: &ProcessInstance, names: &HashSet<String>) -> bool {
    process
        .executable_path
        .components()
        .filter_map(|component| {
            let component = component.as_os_str().to_string_lossy();
            let bundle_name = component.strip_suffix(".app")?;
            Some(normalize_name(bundle_name))
        })
        .any(|bundle_name| names.contains(&bundle_name))
}

fn snapshot_error_results(
    validation_errors: &[Option<PlatformError>],
    snapshot_error: PlatformError,
) -> Vec<PlatformResult<ApplicationProcessCloseResult>> {
    validation_errors
        .iter()
        .map(|validation_error| {
            Err(validation_error
                .clone()
                .unwrap_or_else(|| snapshot_error.clone()))
        })
        .collect()
}

fn current_executable_path(pid: i32) -> Option<PathBuf> {
    let mut buffer = vec![0_u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
    let length = unsafe {
        libc::proc_pidpath(
            pid,
            buffer.as_mut_ptr().cast(),
            libc::PROC_PIDPATHINFO_MAXSIZE as u32,
        )
    };
    if length <= 0 {
        return None;
    }
    buffer.truncate(length as usize);
    Some(PathBuf::from(OsStr::from_bytes(&buffer)))
}

fn process_matches_path(process: &ProcessInstance, target_path: &Path) -> bool {
    let executable = normalize_path(&process.executable_path);
    if executable == target_path {
        return true;
    }
    // macOS uninstall targets commonly identify the application bundle while
    // `ps` reports its executable below `Contents/MacOS`.
    target_path
        .extension()
        .is_some_and(|value| value.eq_ignore_ascii_case("app"))
        && executable.starts_with(target_path.join("Contents"))
}

fn process_snapshot() -> PlatformResult<Vec<ProcessInstance>> {
    let executable = ControlledExecutable::capture(Path::new("/bin/ps")).map_err(|error| {
        PlatformError::operation_failed(format!(
            "macos_process_control_executable_invalid reason={}",
            error.as_str()
        ))
    })?;
    let output = run_controlled_command(
        "macos-process-control-snapshot",
        &executable,
        &["-axo", "pid=,comm="],
        ControlledEnvironmentPolicy::Inherit,
        ControlledCommandLimits {
            timeout: PROCESS_COMMAND_TIMEOUT,
            stdout_bytes: PROCESS_COMMAND_OUTPUT_LIMIT,
            stderr_bytes: 64 * 1024,
        },
        &|| false,
    )
    .map_err(|error| {
        PlatformError::operation_failed(format!(
            "macos_process_control_snapshot_failed reason={}",
            error.as_str()
        ))
    })?;
    if !output.status.success() {
        return Err(PlatformError::operation_failed(
            "macOS process control snapshot command failed",
        ));
    }

    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(parse_process_line)
        .collect())
}

fn parse_process_line(line: &str) -> Option<ProcessInstance> {
    let line = line.trim();
    let separator = line.find(char::is_whitespace)?;
    let pid = line[..separator].parse::<i32>().ok()?;
    let executable = line[separator..].trim();
    if executable.is_empty() {
        return None;
    }
    let executable_path = PathBuf::from(executable);
    let display_name = executable_path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| executable.to_string());
    Some(ProcessInstance {
        pid,
        executable_path,
        display_name,
    })
}

fn unique_display_names(processes: &[ProcessInstance]) -> Vec<String> {
    let mut names = processes
        .iter()
        .map(|process| process.display_name.clone())
        .collect::<Vec<_>>();
    names.sort_by_key(|name| name.to_ascii_lowercase());
    names.dedup_by(|left, right| left.eq_ignore_ascii_case(right));
    names
}

fn normalize_name(name: &str) -> String {
    Path::new(name.trim())
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_lowercase()
}

fn normalize_path(path: &Path) -> PathBuf {
    PathBuf::from(path.to_string_lossy().trim_end_matches('/'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{process::Command, sync::Mutex};

    static PROCESS_CLOSE_TEST_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn ownership_checks_accept_current_user_and_fail_closed_for_missing_processes() {
        assert!(process_owned_by_current_user(std::process::id() as i32));
        assert!(!process_owned_by_current_user(i32::MAX));
        assert!(!process_owned_by_current_user(-1));
    }

    #[test]
    fn refuses_self_and_stale_executable_identity() {
        let mut process = ProcessInstance {
            pid: std::process::id() as i32,
            executable_path: std::env::current_exe().unwrap(),
            display_name: "test".to_owned(),
        };
        assert!(!can_signal_process(&process));
        process.pid = unsafe { libc::getppid() };
        process.executable_path = PathBuf::from("/nonexistent/changed-executable");
        assert!(!can_signal_process(&process));
    }

    #[test]
    fn parses_process_paths_with_spaces() {
        let process =
            parse_process_line("  42 /Applications/WPS Office.app/Contents/MacOS/wpsoffice")
                .expect("the process line should parse");
        assert_eq!(process.pid, 42);
        assert_eq!(process.display_name, "wpsoffice");
    }

    #[test]
    fn bundle_target_matches_its_executable() {
        let process = ProcessInstance {
            pid: 42,
            executable_path: PathBuf::from("/Applications/WPS Office.app/Contents/MacOS/wpsoffice"),
            display_name: "wpsoffice".to_string(),
        };
        assert!(process_matches_path(
            &process,
            Path::new("/Applications/WPS Office.app")
        ));
    }

    #[test]
    fn exact_paths_disable_same_name_fallback() {
        let snapshot = vec![ProcessInstance {
            pid: 42,
            executable_path: PathBuf::from("/Applications/Other.app/Contents/MacOS/SharedHelper"),
            display_name: "SharedHelper".to_string(),
        }];
        let target = ApplicationProcessTarget {
            executable_names: vec!["SharedHelper".to_string()],
            executable_paths: vec![PathBuf::from(
                "/Applications/Expected.app/Contents/MacOS/SharedHelper",
            )],
        };

        assert!(matching_processes_in_snapshot(&target, &snapshot).is_empty());
    }

    #[test]
    fn name_target_matches_bundle_contained_helpers() {
        let snapshot = vec![ProcessInstance {
            pid: 42,
            executable_path: PathBuf::from(
                "/System/Applications/Podcasts.app/Contents/XPCServices/PodcastSync.xpc/Contents/MacOS/PodcastSync",
            ),
            display_name: "PodcastSync".to_string(),
        }];
        let target = ApplicationProcessTarget {
            executable_names: vec!["Podcasts".to_string()],
            executable_paths: Vec::new(),
        };

        assert_eq!(
            matching_processes_in_snapshot(&target, &snapshot).len(),
            1,
            "a helper inside the named bundle must match the close target exactly as the cleanup guard counts it as running"
        );
    }

    #[test]
    fn name_target_ignores_unrelated_bundle_suffixes() {
        let snapshot = vec![ProcessInstance {
            pid: 42,
            executable_path: PathBuf::from(
                "/Applications/Podcasts Utility.app/Contents/MacOS/Podcasts Utility",
            ),
            display_name: "Podcasts Utility".to_string(),
        }];
        let target = ApplicationProcessTarget {
            executable_names: vec!["Podcasts".to_string()],
            executable_paths: Vec::new(),
        };

        assert!(
            matching_processes_in_snapshot(&target, &snapshot).is_empty(),
            "only an exact bundle-name match may attribute a helper to the target application"
        );
    }

    #[test]
    #[ignore = "launches and terminates a real child process"]
    fn closes_spawned_command_by_exact_path() {
        run_spawned_command_close(ApplicationProcessCloseMode::Graceful, "graceful");
    }

    #[test]
    #[ignore = "launches and force-terminates a real child process"]
    fn force_closes_spawned_command_by_exact_path() {
        run_spawned_command_close(ApplicationProcessCloseMode::Force, "force");
    }

    #[test]
    #[ignore = "launches and force-terminates real replacement processes"]
    fn force_closes_a_delayed_replacement_before_reporting_success() {
        let _guard = PROCESS_CLOSE_TEST_LOCK.lock().unwrap();
        let executable = PathBuf::from("/bin/sleep");
        let target = ApplicationProcessTarget {
            executable_names: Vec::new(),
            executable_paths: vec![executable.clone()],
        };
        assert!(
            matching_processes(&target).unwrap().is_empty(),
            "this ignored test requires an isolated machine without another sleep process"
        );
        let child = Command::new(&executable).arg("30").spawn().unwrap();
        let first_pid = child.id() as i32;
        let replacement_executable = executable.clone();
        let (sender, receiver) = std::sync::mpsc::channel();
        let reaper = thread::spawn(move || {
            let _ = child.wait_with_output();
            thread::sleep(Duration::from_millis(350));
            let replacement = Command::new(&replacement_executable)
                .arg("30")
                .spawn()
                .unwrap();
            sender.send(replacement.id() as i32).unwrap();
            replacement.wait_with_output().unwrap()
        });
        let result = close(
            &ApplicationProcessTarget {
                executable_names: Vec::new(),
                executable_paths: vec![executable],
            },
            ApplicationProcessCloseMode::Force,
        );
        // Bound cleanup even when the implementation regresses and returns early.
        unsafe {
            libc::kill(first_pid, libc::SIGKILL);
        }
        let replacement_pid = receiver.recv_timeout(Duration::from_secs(3)).unwrap();
        let replacement_was_running = unsafe { libc::kill(replacement_pid, 0) == 0 };
        unsafe {
            libc::kill(replacement_pid, libc::SIGKILL);
        }
        let _ = reaper.join();
        let result = result.unwrap();
        assert!(
            !replacement_was_running,
            "the delayed replacement must end before success"
        );
        assert_eq!(result.matched_process_count, 2);
        assert_eq!(result.requested_process_count, 2);
        assert!(result.remaining_processes.is_empty());
    }

    fn run_spawned_command_close(mode: ApplicationProcessCloseMode, suffix: &str) {
        let _guard = PROCESS_CLOSE_TEST_LOCK
            .lock()
            .expect("the process-close test lock should be available");
        let executable = PathBuf::from("/bin/sleep");
        let target = ApplicationProcessTarget {
            executable_names: Vec::new(),
            executable_paths: vec![executable.clone()],
        };
        assert!(
            matching_processes(&target)
                .expect("the initial process snapshot should succeed")
                .is_empty(),
            "the ignored smoke test requires an isolated machine without another sleep process"
        );
        let child = Command::new(&executable)
            .arg("30")
            .spawn()
            .expect("the test process should start");
        let child_pid = child.id() as i32;
        // Reap the child concurrently so the post-close snapshot does not
        // mistake a zombie owned by this test harness for a live process.
        let reaper = thread::spawn(move || child.wait_with_output());
        let result = close(&target, mode);

        // Ensure the test cannot leave its isolated child behind if the close
        // implementation returned without terminating it.
        unsafe {
            libc::kill(child_pid, libc::SIGKILL);
        }
        let _ = reaper.join();

        let result =
            result.unwrap_or_else(|error| panic!("{suffix} close should succeed: {error}"));
        assert_eq!(result.matched_process_count, 1);
        assert_eq!(result.requested_process_count, 1);
        assert!(result.remaining_processes.is_empty());
    }
}
