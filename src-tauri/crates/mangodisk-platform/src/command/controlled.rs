use std::{
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
    process::{Command, ExitStatus, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

const PROCESS_POLL_INTERVAL: Duration = Duration::from_millis(10);
#[cfg(target_os = "macos")]
const CONTROLLED_ENV_ALLOWLIST: &[&str] = &["HOME", "TMPDIR"];
#[cfg(target_os = "linux")]
const CONTROLLED_ENV_ALLOWLIST: &[&str] = &[
    "HOME",
    "TMPDIR",
    "XDG_CONFIG_HOME",
    "XDG_DATA_HOME",
    "XDG_CACHE_HOME",
];
#[cfg(windows)]
const CONTROLLED_ENV_ALLOWLIST: &[&str] = &[
    "SYSTEMDRIVE",
    "SYSTEMROOT",
    "WINDIR",
    "PROGRAMDATA",
    "ALLUSERSPROFILE",
    "USERPROFILE",
    "HOMEDRIVE",
    "HOMEPATH",
    "APPDATA",
    "LOCALAPPDATA",
    "TEMP",
    "TMP",
];

/// Controls whether a child process receives the normal desktop environment.
///
/// System inventory commands need the complete Windows or macOS process
/// contract. Tool cleaners use the isolated policy when variables such as
/// `DOCKER_HOST` could redirect a destructive command to another target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlledEnvironmentPolicy {
    Inherit,
    Isolated,
}

impl ControlledEnvironmentPolicy {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Inherit => "inherit",
            Self::Isolated => "isolated",
        }
    }
}

/// Controls completion-level diagnostics for commands that are invoked repeatedly as one
/// higher-level operation. Exceptional terminal states are always logged; only ordinary process
/// exits may be aggregated by the caller to avoid flooding the application log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlledCommandLogPolicy {
    EveryCompletion,
    ExceptionalOnly,
}

/// An executable captured by the platform inventory. Business code may retain
/// and pass this value but cannot construct it from rule text or user input.
/// Identity is checked again before launch to reject path replacement races.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlledExecutable {
    canonical_path: PathBuf,
    identity: ExecutableIdentity,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ExecutableIdentity {
    length: u64,
    modified_ns: Option<u128>,
    created_ns: Option<u128>,
    platform_file_id: PlatformFileId,
}

#[cfg(unix)]
#[derive(Debug, Clone, PartialEq, Eq)]
struct PlatformFileId {
    device: u64,
    inode: u64,
}

#[cfg(windows)]
#[derive(Debug, Clone, PartialEq, Eq)]
struct PlatformFileId {
    volume_serial_number: u32,
    file_index: u64,
}

impl ControlledExecutable {
    /// Captures an executable found by the platform tool inventory. The
    /// canonical path and file identity are stored together so a different
    /// regular file at the same path cannot pass pre-launch validation.
    pub(crate) fn capture(path: &Path) -> Result<Self, ControlledCommandError> {
        let (canonical_path, identity) = executable_identity(path)?;
        Ok(Self {
            canonical_path,
            identity,
        })
    }

    pub(crate) fn validated_path(&self) -> Result<&Path, ControlledCommandError> {
        let (canonical_path, identity) = executable_identity(&self.canonical_path)?;
        if canonical_path != self.canonical_path || identity != self.identity {
            return Err(ControlledCommandError::ExecutableChanged);
        }
        // Windows controlled commands allow only PE entry points. Executing
        // `.cmd` or `.bat` files would reintroduce a command interpreter. The
        // inventory may still record scripts for applicability checks.
        #[cfg(windows)]
        if !canonical_path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("exe"))
        {
            return Err(ControlledCommandError::InvalidExecutable);
        }
        Ok(&self.canonical_path)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ControlledCommandLimits {
    pub timeout: Duration,
    pub stdout_bytes: usize,
    pub stderr_bytes: usize,
}

#[derive(Debug)]
pub struct ControlledCommandOutput {
    pub status: ExitStatus,
    pub stdout: Vec<u8>,
    pub stderr_bytes: usize,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlledCommandError {
    InvalidExecutable,
    ExecutableChanged,
    SpawnFailed,
    ReaderFailed,
    WaitFailed,
    Cancelled,
    TimedOut,
    OutputLimitExceeded,
}

impl ControlledCommandError {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidExecutable => "invalidExecutable",
            Self::ExecutableChanged => "executableChanged",
            Self::SpawnFailed => "spawnFailed",
            Self::ReaderFailed => "readerFailed",
            Self::WaitFailed => "waitFailed",
            Self::Cancelled => "cancelled",
            Self::TimedOut => "timedOut",
            Self::OutputLimitExceeded => "outputLimitExceeded",
        }
    }
}

struct BoundedRead {
    retained: Vec<u8>,
    total_bytes: usize,
}

/// Runs a command registered in core at compile time. This entry point never
/// invokes a shell, chooses the executable, or constructs business arguments.
/// Callers provide an inventory-captured path and static arguments.
///
/// The stdout and stderr readers continuously drain their pipes while retaining
/// only the configured byte limits. Limit, timeout, and cancellation paths kill
/// and wait for the child before joining readers to avoid deadlocks and zombies.
pub fn run_controlled_command(
    command_id: &'static str,
    executable: &ControlledExecutable,
    args: &[&str],
    environment_policy: ControlledEnvironmentPolicy,
    limits: ControlledCommandLimits,
    is_cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<ControlledCommandOutput, ControlledCommandError> {
    run_controlled_command_with_log_policy(
        command_id,
        executable,
        args,
        environment_policy,
        limits,
        ControlledCommandLogPolicy::EveryCompletion,
        is_cancelled,
    )
}

pub fn run_controlled_command_with_log_policy(
    command_id: &'static str,
    executable: &ControlledExecutable,
    args: &[&str],
    environment_policy: ControlledEnvironmentPolicy,
    limits: ControlledCommandLimits,
    log_policy: ControlledCommandLogPolicy,
    is_cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<ControlledCommandOutput, ControlledCommandError> {
    let executable = executable.validated_path()?;
    if is_cancelled() {
        return Err(ControlledCommandError::Cancelled);
    }

    let started = Instant::now();
    let mut command = Command::new(executable);
    configure_background_process(&mut command);
    configure_controlled_environment(&mut command, environment_policy);
    let mut child = command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| {
            log_command_error(command_id, "spawn", &error);
            ControlledCommandError::SpawnFailed
        })?;
    let stdout = child.stdout.take().ok_or_else(|| {
        let _ = child.kill();
        let _ = child.wait();
        ControlledCommandError::ReaderFailed
    })?;
    let stderr = child.stderr.take().ok_or_else(|| {
        let _ = child.kill();
        let _ = child.wait();
        ControlledCommandError::ReaderFailed
    })?;
    let output_exceeded = Arc::new(AtomicBool::new(false));
    let stdout_reader = spawn_reader(
        "mangodisk-command-stdout",
        stdout,
        limits.stdout_bytes,
        Arc::clone(&output_exceeded),
    );
    let stderr_reader = spawn_reader(
        "mangodisk-command-stderr",
        stderr,
        limits.stderr_bytes,
        Arc::clone(&output_exceeded),
    );
    let (stdout_reader, stderr_reader) = match (stdout_reader, stderr_reader) {
        (Ok(stdout_reader), Ok(stderr_reader)) => (stdout_reader, stderr_reader),
        (stdout_reader, stderr_reader) => {
            let _ = child.kill();
            let _ = child.wait();
            if let Ok(reader) = stdout_reader {
                let _ = reader.join();
            }
            if let Ok(reader) = stderr_reader {
                let _ = reader.join();
            }
            return Err(ControlledCommandError::ReaderFailed);
        }
    };

    let mut terminal_error = None;
    let status = loop {
        if is_cancelled() {
            terminal_error = Some(ControlledCommandError::Cancelled);
        } else if output_exceeded.load(Ordering::Relaxed) {
            terminal_error = Some(ControlledCommandError::OutputLimitExceeded);
        } else if started.elapsed() >= limits.timeout {
            terminal_error = Some(ControlledCommandError::TimedOut);
        }
        if terminal_error.is_some() {
            let _ = child.kill();
            break match child.wait() {
                Ok(status) => status,
                Err(error) => {
                    log_command_error(command_id, "wait_after_kill", &error);
                    terminal_error = Some(ControlledCommandError::WaitFailed);
                    // Try one final reap after wait fails. Even a second error
                    // must flow through the joins below so pipe threads release
                    // their resources before the error is returned.
                    child.wait().unwrap_or_else(|retry_error| {
                        log_command_error(command_id, "wait_after_kill_retry", &retry_error);
                        failed_exit_status()
                    })
                }
            };
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => thread::sleep(PROCESS_POLL_INTERVAL),
            Err(error) => {
                log_command_error(command_id, "try_wait", &error);
                let _ = child.kill();
                let _ = child.wait();
                terminal_error = Some(ControlledCommandError::WaitFailed);
                // Kill and wait have completed; this status has no business meaning.
                break failed_exit_status();
            }
        }
    };

    // Join both readers before propagating either error. Sequential `?` would
    // drop the stderr handle when stdout fails and leak a background thread on
    // the exceptional path.
    let stdout = join_reader(command_id, stdout_reader);
    let stderr = join_reader(command_id, stderr_reader);
    let stdout = stdout?;
    let stderr = stderr?;
    // A child can exit before a reader sets the limit flag, allowing the main
    // thread to observe success first. Recheck actual byte totals after both
    // joins to close the fast-write-and-exit race.
    if terminal_error.is_none()
        && (stdout.total_bytes > limits.stdout_bytes || stderr.total_bytes > limits.stderr_bytes)
    {
        terminal_error = Some(ControlledCommandError::OutputLimitExceeded);
    }
    if terminal_error != Some(ControlledCommandError::Cancelled)
        && (!status.success() || terminal_error.is_some())
    {
        // Some probes use nonzero exits for normal absence, such as `defaults read` on an
        // unset preference. Let their caller classify the result, retaining details in DEBUG.
        // Actual transport/termination failures stay visible regardless of completion policy.
        let level = if terminal_error.is_none()
            && log_policy == ControlledCommandLogPolicy::ExceptionalOnly
        {
            log::Level::Debug
        } else {
            log::Level::Warn
        };
        log::log!(
            level,
            "controlled_command_failed command_id={} executable={} exit_code={:?} terminal_error={:?} stderr={}",
            command_id,
            crate::diagnostics::text(&executable.display()),
            status.code(),
            terminal_error,
            crate::diagnostics::text(&String::from_utf8_lossy(&stderr.retained))
        );
    }
    if let Some(error) = terminal_error {
        log::info!(
            "controlled_command_finished command_id={} environment_policy={} status={} stdout_bytes={} stderr_bytes={} elapsed_ms={}",
            command_id,
            environment_policy.as_str(),
            error.as_str(),
            stdout.total_bytes,
            stderr.total_bytes,
            started.elapsed().as_millis()
        );
        return Err(error);
    }
    if log_policy == ControlledCommandLogPolicy::EveryCompletion {
        log::info!(
            "controlled_command_finished command_id={} environment_policy={} status=completed success={} stdout_bytes={} stderr_bytes={} elapsed_ms={}",
            command_id,
            environment_policy.as_str(),
            status.success(),
            stdout.total_bytes,
            stderr.total_bytes,
            started.elapsed().as_millis()
        );
    }
    Ok(ControlledCommandOutput {
        status,
        stdout: stdout.retained,
        stderr_bytes: stderr.total_bytes,
        elapsed_ms: started.elapsed().as_millis() as u64,
    })
}

/// Prevents console helper processes from flashing a terminal window while a
/// desktop scan is running. This changes only the Windows process window mode;
/// stdout and stderr ownership remain the caller's responsibility.
pub fn configure_background_process(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;

        // CREATE_NO_WINDOW is intentionally applied to inventory and decoding
        // helpers. A native GUI uninstaller can still display its own windows.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }

    #[cfg(not(windows))]
    let _ = command;
}

/// Applies environment isolation only when the caller has identified a target
/// redirection risk. Native inventory commands inherit the OS environment so
/// PowerShell modules, Known Folders, and COM catalog caches keep valid paths.
/// Isolated tool commands retain the minimum platform variables needed by the
/// child runtime while excluding tool-specific remote target configuration.
/// Their working directory is also moved away from the repository: some CLIs
/// fall back to relative state directories when their own configuration is
/// absent, and a read-only preview must never create those directories beside
/// application source files.
fn configure_controlled_environment(
    command: &mut Command,
    environment_policy: ControlledEnvironmentPolicy,
) {
    if environment_policy == ControlledEnvironmentPolicy::Isolated {
        command.env_clear();
        for key in CONTROLLED_ENV_ALLOWLIST {
            if let Some(value) = std::env::var_os(key) {
                command.env(key, value);
            }
        }
        command.current_dir(isolated_working_directory(std::env::temp_dir()));
    }
}

fn isolated_working_directory(temporary_directory: PathBuf) -> PathBuf {
    if temporary_directory.is_absolute() {
        // Do not require the directory to exist here. A stale absolute TEMP
        // path must make process spawn fail closed instead of silently
        // inheriting the application working directory.
        return temporary_directory;
    }

    // Rust permits a process-provided temporary directory to be relative. A
    // filesystem root is deliberately non-private and normally non-writable to
    // desktop tools, so an unexpected relative fallback cannot create state in
    // a development checkout or user-selected working directory.
    if let Ok(executable) = std::env::current_exe() {
        if let Some(root) = executable
            .ancestors()
            .find(|ancestor| ancestor.is_absolute() && ancestor.parent().is_none())
        {
            return root.to_path_buf();
        }
    }
    #[cfg(windows)]
    return PathBuf::from(r"C:\");
    #[cfg(not(windows))]
    PathBuf::from("/")
}

fn executable_identity(
    executable: &Path,
) -> Result<(PathBuf, ExecutableIdentity), ControlledCommandError> {
    if !executable.is_absolute() {
        return Err(ControlledCommandError::InvalidExecutable);
    }
    let canonical =
        fs::canonicalize(executable).map_err(|_| ControlledCommandError::InvalidExecutable)?;
    // Open once and read metadata plus platform identity from the same handle.
    // Separate path lookups could observe mixed attributes during replacement.
    let file = fs::File::open(&canonical).map_err(|_| ControlledCommandError::InvalidExecutable)?;
    let metadata = file
        .metadata()
        .map_err(|_| ControlledCommandError::InvalidExecutable)?;
    if !metadata.is_file() {
        return Err(ControlledCommandError::InvalidExecutable);
    }
    Ok((
        canonical,
        ExecutableIdentity {
            length: metadata.len(),
            modified_ns: system_time_ns(metadata.modified().ok()),
            created_ns: system_time_ns(metadata.created().ok()),
            platform_file_id: platform_file_id(&file, &metadata)?,
        },
    ))
}

fn system_time_ns(value: Option<std::time::SystemTime>) -> Option<u128> {
    value?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|value| {
            u128::from(value.as_secs())
                .saturating_mul(1_000_000_000)
                .saturating_add(u128::from(value.subsec_nanos()))
        })
}

#[cfg(unix)]
fn platform_file_id(
    _file: &fs::File,
    metadata: &fs::Metadata,
) -> Result<PlatformFileId, ControlledCommandError> {
    use std::os::unix::fs::MetadataExt;
    Ok(PlatformFileId {
        device: metadata.dev(),
        inode: metadata.ino(),
    })
}

#[cfg(windows)]
fn platform_file_id(
    file: &fs::File,
    _metadata: &fs::Metadata,
) -> Result<PlatformFileId, ControlledCommandError> {
    use std::{mem::MaybeUninit, os::windows::io::AsRawHandle};
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
    };

    let mut information = MaybeUninit::<BY_HANDLE_FILE_INFORMATION>::zeroed();
    // SAFETY: `file` remains open during the call, so the raw handle is valid.
    // A nonzero Windows API result fully initializes the output structure.
    let succeeded =
        unsafe { GetFileInformationByHandle(file.as_raw_handle() as _, information.as_mut_ptr()) };
    if succeeded == 0 {
        return Err(ControlledCommandError::InvalidExecutable);
    }
    // SAFETY: API success was checked above and the system initialized all fields.
    let information = unsafe { information.assume_init() };
    Ok(PlatformFileId {
        volume_serial_number: information.dwVolumeSerialNumber,
        file_index: (u64::from(information.nFileIndexHigh) << 32)
            | u64::from(information.nFileIndexLow),
    })
}

fn spawn_reader(
    name: &str,
    reader: impl Read + Send + 'static,
    limit: usize,
    output_exceeded: Arc<AtomicBool>,
) -> io::Result<thread::JoinHandle<io::Result<BoundedRead>>> {
    thread::Builder::new()
        .name(name.to_string())
        .spawn(move || bounded_read(reader, limit, &output_exceeded))
}

fn bounded_read(
    mut reader: impl Read,
    limit: usize,
    output_exceeded: &AtomicBool,
) -> io::Result<BoundedRead> {
    let mut retained = Vec::with_capacity(limit.min(64 * 1024));
    let mut total_bytes = 0_usize;
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        total_bytes = total_bytes.saturating_add(read);
        let remaining = limit.saturating_sub(retained.len());
        retained.extend_from_slice(&buffer[..read.min(remaining)]);
        if total_bytes > limit {
            output_exceeded.store(true, Ordering::Relaxed);
        }
    }
    Ok(BoundedRead {
        retained,
        total_bytes,
    })
}

fn join_reader(
    command_id: &str,
    reader: thread::JoinHandle<io::Result<BoundedRead>>,
) -> Result<BoundedRead, ControlledCommandError> {
    reader
        .join()
        .map_err(|_| ControlledCommandError::ReaderFailed)?
        .map_err(|error| {
            log_command_error(command_id, "read", &error);
            ControlledCommandError::ReaderFailed
        })
}

fn log_command_error(command_id: &str, stage: &str, error: &io::Error) {
    // Keep the native cause as well as the OS code: custom I/O errors may have no code.
    // Command arguments and environment variables are deliberately not logged.
    log::warn!(
        "controlled_command_error command_id={} stage={} error_kind={:?} os_error_code={} error={}",
        command_id,
        stage,
        error.kind(),
        error
            .raw_os_error()
            .map(|code| code.to_string())
            .unwrap_or_else(|| "none".to_string()),
        crate::diagnostics::text(&error)
    );
}

#[cfg(unix)]
fn failed_exit_status() -> ExitStatus {
    use std::os::unix::process::ExitStatusExt;
    ExitStatus::from_raw(1 << 8)
}

#[cfg(windows)]
fn failed_exit_status() -> ExitStatus {
    use std::os::windows::process::ExitStatusExt;
    ExitStatus::from_raw(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_limits(stdout_bytes: usize, timeout: Duration) -> ControlledCommandLimits {
        ControlledCommandLimits {
            timeout,
            stdout_bytes,
            stderr_bytes: 1024,
        }
    }

    fn run_fixture(
        fixture: &str,
        limits: ControlledCommandLimits,
        cancelled: &AtomicBool,
    ) -> Result<ControlledCommandOutput, ControlledCommandError> {
        let executable = ControlledExecutable::capture(
            &std::env::current_exe().expect("the current test process should be available"),
        )
        .expect("the test process should be a controlled executable");
        run_controlled_command(
            "test.fixture",
            &executable,
            &["--ignored", "--exact", fixture, "--nocapture"],
            ControlledEnvironmentPolicy::Inherit,
            limits,
            &|| cancelled.load(Ordering::Relaxed),
        )
    }

    #[test]
    #[ignore = "captures actual child-process stderr for diagnostic review"]
    fn actual_failed_command_preserves_diagnostic_output() {
        struct Evidence(std::sync::Mutex<Vec<String>>);
        impl log::Log for Evidence {
            fn enabled(&self, _: &log::Metadata<'_>) -> bool {
                true
            }
            fn log(&self, record: &log::Record<'_>) {
                self.0
                    .lock()
                    .unwrap()
                    .push(format!("{} {}", record.level(), record.args()));
            }
            fn flush(&self) {}
        }
        static EVIDENCE: Evidence = Evidence(std::sync::Mutex::new(Vec::new()));
        log::set_logger(&EVIDENCE).unwrap();
        log::set_max_level(log::LevelFilter::Debug);
        let executable = ControlledExecutable::capture(&std::env::current_exe().unwrap()).unwrap();
        let output = run_controlled_command(
            "diagnostic-native-failure",
            &executable,
            &[
                "--ignored",
                "--exact",
                "command::controlled::tests::failed_diagnostic_child",
                "--nocapture",
            ],
            ControlledEnvironmentPolicy::Inherit,
            ControlledCommandLimits {
                timeout: Duration::from_secs(20),
                stdout_bytes: 8192,
                stderr_bytes: 8192,
            },
            &|| false,
        )
        .unwrap();
        assert!(!output.status.success());
        let ordinary = run_controlled_command_with_log_policy(
            "diagnostic-ordinary-probe",
            &executable,
            &[
                "--ignored",
                "--exact",
                "command::controlled::tests::failed_diagnostic_child",
                "--nocapture",
            ],
            ControlledEnvironmentPolicy::Inherit,
            test_limits(8192, Duration::from_secs(20)),
            ControlledCommandLogPolicy::ExceptionalOnly,
            &|| false,
        )
        .unwrap();
        assert!(!ordinary.status.success());
        let timeout = run_controlled_command_with_log_policy(
            "diagnostic-probe-timeout",
            &executable,
            &[
                "--ignored",
                "--exact",
                "command::controlled::tests::timeout_fixture",
                "--nocapture",
            ],
            ControlledEnvironmentPolicy::Inherit,
            test_limits(8192, Duration::from_millis(200)),
            ControlledCommandLogPolicy::ExceptionalOnly,
            &|| false,
        )
        .unwrap_err();
        assert_eq!(timeout, ControlledCommandError::TimedOut);
        let records = EVIDENCE.0.lock().unwrap();
        assert!(records.iter().any(|line| line
            .starts_with("DEBUG controlled_command_failed command_id=diagnostic-ordinary-probe")
            && line.contains("os error")));
        assert!(!records
            .iter()
            .any(|line| line.starts_with("WARN") && line.contains("diagnostic-ordinary-probe")));
        assert!(records.iter().any(|line| line
            .starts_with("WARN controlled_command_failed command_id=diagnostic-probe-timeout")
            && line.contains("TimedOut")));
        let failure = records
            .iter()
            .find(|line| line.contains("controlled_command_failed "))
            .unwrap();
        assert!(failure.contains("diagnostic-native-failure"));
        assert!(failure.contains("mangodisk-missing-diagnostic-target"));
        assert!(failure.contains("os error"));
        assert!(!failure.contains('\n'));
        for line in records.iter() {
            println!("{line}");
        }
    }

    #[test]
    #[ignore = "child of the actual native diagnostic test"]
    fn failed_diagnostic_child() {
        let path = std::env::temp_dir()
            .join("mangodisk-missing-diagnostic-target")
            .join("absent.txt");
        assert!(!path.exists());
        let error = fs::read(&path).unwrap_err();
        eprintln!(
            "read_file path={} kind={:?} error={error}",
            path.display(),
            error.kind()
        );
        std::process::exit(7);
    }

    #[test]
    fn bounded_command_retains_output_and_reaps_process() {
        let cancelled = AtomicBool::new(false);
        let output = run_fixture(
            "command::controlled::tests::successful_command_fixture",
            test_limits(1024, Duration::from_secs(2)),
            &cancelled,
        )
        .expect("the success fixture should finish normally");
        assert!(output.status.success());
        assert!(String::from_utf8_lossy(&output.stdout).contains("fixture-output"));
    }

    #[test]
    fn relative_path_is_not_a_controlled_executable() {
        let result = ControlledExecutable::capture(Path::new("cargo"));
        assert_eq!(
            result.unwrap_err(),
            ControlledCommandError::InvalidExecutable
        );
    }

    #[test]
    fn executable_change_after_capture_fails_closed() {
        use std::io::Write;

        let extension = if cfg!(windows) { "exe" } else { "bin" };
        let path = std::env::temp_dir().join(format!(
            "mangodisk-controlled-executable-{}.{extension}",
            std::process::id()
        ));
        fs::write(&path, b"original").expect("the identity fixture should be created");
        let executable = ControlledExecutable::capture(&path)
            .expect("the initial regular file should have a stable identity");
        let mut file = fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .expect("the fixture should reopen");
        file.write_all(b"-changed")
            .expect("the fixture identity should change");
        file.sync_all().expect("the fixture change should sync");

        assert_eq!(
            executable.validated_path().unwrap_err(),
            ControlledCommandError::ExecutableChanged
        );
        fs::remove_file(path).expect("the identity fixture should be removed");
    }

    #[test]
    fn output_limit_terminates_process() {
        let cancelled = AtomicBool::new(false);
        let result = run_fixture(
            "command::controlled::tests::large_stdout_fixture",
            test_limits(128, Duration::from_secs(2)),
            &cancelled,
        );
        assert_eq!(
            result.unwrap_err(),
            ControlledCommandError::OutputLimitExceeded
        );
    }

    #[test]
    fn timeout_terminates_process() {
        let cancelled = AtomicBool::new(false);
        let result = run_fixture(
            "command::controlled::tests::timeout_fixture",
            test_limits(1024, Duration::from_millis(30)),
            &cancelled,
        );
        assert_eq!(result.unwrap_err(), ControlledCommandError::TimedOut);
    }

    #[test]
    fn cancellation_terminates_and_reaps_process_promptly() {
        let executable = ControlledExecutable::capture(
            &std::env::current_exe().expect("the current test process should be available"),
        )
        .expect("the test process should be a controlled executable");
        let started = Instant::now();
        let result = run_controlled_command(
            "test.cancel",
            &executable,
            &[
                "--ignored",
                "--exact",
                "command::controlled::tests::timeout_fixture",
                "--nocapture",
            ],
            ControlledEnvironmentPolicy::Inherit,
            test_limits(1024, Duration::from_secs(2)),
            &|| started.elapsed() >= Duration::from_millis(30),
        );

        assert_eq!(result.unwrap_err(), ControlledCommandError::Cancelled);
        assert!(
            started.elapsed() < Duration::from_millis(250),
            "cancellation should finish within the UI response budget"
        );
    }

    #[test]
    fn stderr_limit_uses_same_fail_closed_policy_as_stdout() {
        let cancelled = AtomicBool::new(false);
        let executable = ControlledExecutable::capture(
            &std::env::current_exe().expect("the current test process should be available"),
        )
        .expect("the test process should be a controlled executable");
        let result = run_controlled_command(
            "test.stderr-limit",
            &executable,
            &[
                "--ignored",
                "--exact",
                "command::controlled::tests::large_stderr_fixture",
                "--nocapture",
            ],
            ControlledEnvironmentPolicy::Inherit,
            ControlledCommandLimits {
                timeout: Duration::from_secs(2),
                stdout_bytes: 1024,
                stderr_bytes: 128,
            },
            &|| cancelled.load(Ordering::Relaxed),
        );

        assert_eq!(
            result.unwrap_err(),
            ControlledCommandError::OutputLimitExceeded
        );
    }

    #[test]
    fn controlled_environment_removes_target_changing_variables() {
        let mut command = Command::new(
            std::env::current_exe().expect("the current test process should be available"),
        );
        // Inject the variables into the child command instead of mutating the
        // process environment so parallel tests remain isolated. Configuration
        // must remove them before spawn to preserve the local-target boundary.
        command
            .env("DOCKER_HOST", "tcp://remote.example.invalid:2375")
            .env("DOCKER_CONTEXT", "remote-context");
        configure_controlled_environment(&mut command, ControlledEnvironmentPolicy::Isolated);
        let output = command
            .args([
                "--ignored",
                "--exact",
                "command::controlled::tests::isolated_environment_fixture",
                "--nocapture",
            ])
            .output()
            .expect("the isolated environment fixture should start");

        assert!(
            output.status.success(),
            "removed Docker target variables should not reach the child: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn isolated_working_directory_never_uses_a_relative_fallback() {
        let selected = isolated_working_directory(PathBuf::from("relative-temp"));

        assert!(selected.is_absolute());
        assert_ne!(selected, PathBuf::from("relative-temp"));
    }

    #[test]
    fn inherited_environment_preserves_the_parent_process_contract() {
        let mut command = Command::new(
            std::env::current_exe().expect("the current test process should be available"),
        );
        command.env("MANGODISK_ENVIRONMENT_POLICY_FIXTURE", "available");
        configure_controlled_environment(&mut command, ControlledEnvironmentPolicy::Inherit);
        let output = command
            .args([
                "--ignored",
                "--exact",
                "command::controlled::tests::inherited_environment_fixture",
                "--nocapture",
            ])
            .output()
            .expect("the inherited environment fixture should start");

        assert!(
            output.status.success(),
            "the inherited environment should reach the child: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    #[ignore = "executed only as the controlled success child"]
    fn successful_command_fixture() {
        println!("fixture-output");
    }

    #[test]
    #[ignore = "executed only to verify the controlled environment"]
    fn isolated_environment_fixture() {
        assert!(std::env::var_os("DOCKER_HOST").is_none());
        assert!(std::env::var_os("DOCKER_CONTEXT").is_none());
        let current_directory = std::env::current_dir()
            .expect("the isolated command must receive a valid working directory");
        assert_eq!(
            fs::canonicalize(current_directory)
                .expect("the working directory must be canonicalizable"),
            fs::canonicalize(std::env::temp_dir())
                .expect("the temporary directory must be canonicalizable"),
            "isolated commands must not inherit the repository working directory"
        );
        #[cfg(windows)]
        for key in [
            "SYSTEMDRIVE",
            "PROGRAMDATA",
            "ALLUSERSPROFILE",
            "USERPROFILE",
            "APPDATA",
            "LOCALAPPDATA",
        ] {
            assert!(
                std::env::var_os(key).is_some_and(|value| !value.is_empty()),
                "the isolated Windows runtime requires {key}"
            );
        }
        #[cfg(windows)]
        assert!(
            std::env::var_os("USERPROFILE").is_some_and(|value| Path::new(&value).is_absolute()),
            "Docker configuration needs an absolute Windows user profile"
        );
        #[cfg(target_os = "macos")]
        assert!(
            std::env::var_os("HOME").is_some_and(|value| Path::new(&value).is_absolute()),
            "isolated macOS tools need an absolute home directory"
        );
    }

    #[test]
    #[ignore = "executed only to verify inherited environment behavior"]
    fn inherited_environment_fixture() {
        assert_eq!(
            std::env::var_os("MANGODISK_ENVIRONMENT_POLICY_FIXTURE").as_deref(),
            Some(std::ffi::OsStr::new("available"))
        );
    }

    #[test]
    #[ignore = "executed only as the controlled large-output child"]
    fn large_stdout_fixture() {
        println!("{}", "x".repeat(16 * 1024));
    }

    #[test]
    #[ignore = "executed only as the controlled large-stderr child"]
    fn large_stderr_fixture() {
        eprintln!("{}", "x".repeat(16 * 1024));
    }

    #[test]
    #[ignore = "executed only as the controlled timeout child"]
    fn timeout_fixture() {
        thread::sleep(Duration::from_secs(2));
    }
}
