use std::{
    ffi::OsStr,
    io::{ErrorKind, Read},
    net::{Ipv4Addr, SocketAddrV4, TcpListener, TcpStream},
    os::windows::ffi::OsStrExt,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use base64::{engine::general_purpose::STANDARD, Engine as _};

use windows_sys::Win32::{
    Foundation::{CloseHandle, GetLastError, WAIT_FAILED, WAIT_OBJECT_0, WAIT_TIMEOUT},
    System::{
        SystemInformation::GetWindowsDirectoryW,
        Threading::{GetExitCodeProcess, WaitForSingleObject},
    },
    UI::{
        Shell::{ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW},
        WindowsAndMessaging::SW_HIDE,
    },
};

#[cfg(test)]
#[path = "system_maintenance/step_tests.rs"]
mod step_tests;
#[path = "system_maintenance/task_scripts.rs"]
mod task_scripts;

use crate::system_maintenance_helper::step_diagnostics::{
    classify_failure, MaintenanceStage, MaintenanceStepDiagnostic, MaintenanceStepResult,
};
use crate::system_maintenance_helper::{
    PrivilegedFailureStage, PrivilegedMaintenanceFailure, PrivilegedMaintenanceOutcome,
    PrivilegedMaintenanceResult, PrivilegedProcessDiagnostics,
};
use crate::{
    run_controlled_command, ControlledCommandError, ControlledCommandLimits,
    ControlledEnvironmentPolicy, ControlledExecutable, PlatformCancellation, PlatformError,
    PlatformErrorCode, PlatformResult, PlatformSystemMaintenanceCompletion,
    PlatformSystemMaintenanceDiagnosticCode, PlatformSystemMaintenanceExecution,
    PlatformSystemMaintenancePhase, PlatformSystemMaintenanceProgress,
    PlatformSystemMaintenanceProgressSink, PlatformSystemMaintenanceState,
    PlatformSystemMaintenanceStatus,
};

const SYSTEM_INTEGRITY: &str = "windows.maintenance.system-integrity";
const SEARCH_INDEX: &str = "windows.maintenance.search-index";
const EXPLORER_CACHE: &str = "windows.maintenance.explorer-cache";
const UPDATE_COMPONENTS: &str = "windows.maintenance.update-components";
const PRINT_QUEUE: &str = "windows.maintenance.print-queue";
const PERFORMANCE_COUNTERS: &str = "windows.maintenance.performance-counters";
const SYSTEM_DISK: &str = "windows.maintenance.system-disk";
const AUDIO_SERVICE: &str = "windows.maintenance.audio-service";
const STORE_CACHE: &str = "windows.maintenance.store-cache";
const DNS_CACHE: &str = "windows.maintenance.dns-cache";
const TIME_SYNC: &str = "windows.maintenance.time-sync";

const SUPPORTED_TASKS: &[&str] = &[
    SYSTEM_INTEGRITY,
    SEARCH_INDEX,
    EXPLORER_CACHE,
    UPDATE_COMPONENTS,
    PRINT_QUEUE,
    PERFORMANCE_COUNTERS,
    SYSTEM_DISK,
    AUDIO_SERVICE,
    STORE_CACHE,
    DNS_CACHE,
    TIME_SYNC,
];

const DEFAULT_LIMITS: ControlledCommandLimits = ControlledCommandLimits {
    timeout: Duration::from_secs(60),
    stdout_bytes: 64 * 1024,
    stderr_bytes: 64 * 1024,
};
const MAINTENANCE_STEPS_SCRIPT: &str = include_str!("system_maintenance/steps.ps1");
static PROGRESS_CHANNEL_SEQUENCE: AtomicU64 = AtomicU64::new(1);
const PROGRESS_CHANNEL_BUFFER_LIMIT: usize = 64 * 1024;
const EXPLORER_CACHE_SCRIPT: &str = r#"Stop-Process -Name explorer -Force -ErrorAction SilentlyContinue; Remove-Item -LiteralPath "$env:LOCALAPPDATA\IconCache.db" -Force -ErrorAction SilentlyContinue; Remove-Item -Path "$env:LOCALAPPDATA\Microsoft\Windows\Explorer\iconcache*.db" -Force -ErrorAction SilentlyContinue; Remove-Item -Path "$env:LOCALAPPDATA\Microsoft\Windows\Explorer\thumbcache*.db" -Force -ErrorAction SilentlyContinue; Start-Process explorer.exe"#;
const EXPLORER_PROCESS_QUERY_SCRIPT: &str = r#"if ($null -ne (Get-Process -Name explorer -ErrorAction SilentlyContinue | Select-Object -First 1)) { Write-Output 'running' } else { Write-Output 'stopped' }"#;

pub(crate) fn scan(
    task_ids: &[&str],
    cancellation: &PlatformCancellation,
) -> PlatformResult<Vec<PlatformSystemMaintenanceState>> {
    validate_ids(task_ids)?;
    let windows = windows_directory()?;
    let mut states = Vec::with_capacity(task_ids.len());
    for task_id in task_ids {
        if cancellation.is_cancelled() {
            return Err(PlatformError::operation_failed(
                "system maintenance scan was cancelled",
            ));
        }
        states.push(match *task_id {
            SYSTEM_INTEGRITY => availability_state(
                SYSTEM_INTEGRITY,
                system_executable(&windows, "dism.exe").is_file()
                    && system_executable(&windows, "sfc.exe").is_file()
                    && powershell_path(&windows).is_file(),
                true,
            ),
            SEARCH_INDEX => {
                service_task_state(SEARCH_INDEX, "WSearch", true, &windows, cancellation)
            }
            EXPLORER_CACHE => {
                availability_state(EXPLORER_CACHE, powershell_path(&windows).is_file(), false)
            }
            UPDATE_COMPONENTS => update_task_state(&windows, cancellation),
            PRINT_QUEUE => service_task_state(PRINT_QUEUE, "Spooler", true, &windows, cancellation),
            PERFORMANCE_COUNTERS => availability_state(
                PERFORMANCE_COUNTERS,
                system_executable(&windows, "lodctr.exe").is_file()
                    && wbem_executable(&windows, "winmgmt.exe").is_file()
                    && powershell_path(&windows).is_file(),
                true,
            ),
            SYSTEM_DISK => availability_state(
                SYSTEM_DISK,
                system_executable(&windows, "chkdsk.exe").is_file()
                    && powershell_path(&windows).is_file(),
                true,
            ),
            AUDIO_SERVICE => {
                service_task_state(AUDIO_SERVICE, "Audiosrv", true, &windows, cancellation)
            }
            STORE_CACHE => availability_state(
                STORE_CACHE,
                system_executable(&windows, "wsreset.exe").is_file(),
                true,
            ),
            DNS_CACHE => availability_state(
                DNS_CACHE,
                system_executable(&windows, "ipconfig.exe").is_file(),
                true,
            ),
            TIME_SYNC => service_task_state(TIME_SYNC, "W32Time", true, &windows, cancellation),
            _ => unreachable!("validated maintenance identifier"),
        });
    }
    Ok(states)
}

pub(crate) fn execute(
    task_id: &str,
    cancellation: &PlatformCancellation,
    _authorization_prompt: Option<&str>,
    progress: &PlatformSystemMaintenanceProgressSink,
) -> PlatformResult<PlatformSystemMaintenanceExecution> {
    validate_ids(&[task_id])?;
    let windows = windows_directory()?;
    match task_id {
        SYSTEM_INTEGRITY => {
            let elevated = run_with_privileges(task_id, progress)?;
            progress(PlatformSystemMaintenanceProgress::phase(
                PlatformSystemMaintenancePhase::Verifying,
            ));
            Ok(execution(
                task_id,
                true,
                true,
                elevated.requires_restart,
                false,
            ))
        }
        SEARCH_INDEX => {
            run_with_privileges(task_id, progress)?;
            let verified =
                verify_after_mutation(service_running("WSearch", &windows, cancellation))?;
            Ok(execution(task_id, true, verified, false, true))
        }
        EXPLORER_CACHE => {
            progress(PlatformSystemMaintenanceProgress::phase(
                PlatformSystemMaintenancePhase::RefreshingShellCaches,
            ));
            run_powershell(
                &windows,
                EXPLORER_CACHE_SCRIPT,
                cancellation,
                DEFAULT_LIMITS,
            )?;
            progress(PlatformSystemMaintenanceProgress::phase(
                PlatformSystemMaintenancePhase::Verifying,
            ));
            let verified = verify_after_mutation(explorer_running(&windows, cancellation))?;
            Ok(execution(task_id, true, verified, false, false))
        }
        UPDATE_COMPONENTS => {
            run_with_privileges(task_id, progress)?;
            // BITS and Windows Update are trigger-start services and may return to Stopped
            // immediately after the scan request. Their continued registration is the stable
            // postcondition; requiring Running would incorrectly report a successful repair as
            // failed on a healthy Windows installation.
            let mut verified = true;
            for service in ["bits", "cryptsvc", "wuauserv"] {
                verified &= verify_after_mutation(service_exists(service, &windows, cancellation))?;
            }
            // UsoClient starts the update scan asynchronously. Report `Started` so the UI does
            // not claim that Windows Update has already finished checking or downloading.
            Ok(execution(task_id, true, verified, false, true))
        }
        PRINT_QUEUE => {
            run_with_privileges(task_id, progress)?;
            let verified =
                verify_after_mutation(service_running("Spooler", &windows, cancellation))?;
            Ok(execution(task_id, true, verified, false, false))
        }
        PERFORMANCE_COUNTERS => {
            run_with_privileges(task_id, progress)?;
            Ok(execution(task_id, true, true, false, false))
        }
        SYSTEM_DISK => {
            run_with_privileges(task_id, progress)?;
            // `chkdsk /scan` is an online diagnostic and does not mutate a healthy volume. A
            // successful native exit code is its authoritative verification result.
            Ok(execution(task_id, false, true, false, false))
        }
        AUDIO_SERVICE => {
            run_with_privileges(task_id, progress)?;
            let verified =
                verify_after_mutation(service_running("Audiosrv", &windows, cancellation))?;
            Ok(execution(task_id, true, verified, false, false))
        }
        STORE_CACHE => {
            run_with_privileges(task_id, progress)?;
            Ok(execution(task_id, true, true, false, false))
        }
        DNS_CACHE | TIME_SYNC => {
            run_with_privileges(task_id, progress)?;
            Ok(execution(task_id, true, true, false, false))
        }
        _ => unreachable!("validated maintenance identifier"),
    }
}

fn update_task_state(
    windows: &Path,
    cancellation: &PlatformCancellation,
) -> PlatformSystemMaintenanceState {
    if !powershell_path(windows).is_file() {
        return unavailable(
            UPDATE_COMPONENTS,
            true,
            PlatformSystemMaintenanceDiagnosticCode::ToolUnavailable,
        );
    }
    let all_present = ["bits", "cryptsvc", "wuauserv"]
        .into_iter()
        .all(|service| service_exists(service, windows, cancellation).unwrap_or(false));
    if all_present {
        available(UPDATE_COMPONENTS, true)
    } else {
        unavailable(
            UPDATE_COMPONENTS,
            true,
            PlatformSystemMaintenanceDiagnosticCode::ComponentUnavailable,
        )
    }
}

fn service_task_state(
    task_id: &str,
    service: &str,
    requires_elevation: bool,
    windows: &Path,
    cancellation: &PlatformCancellation,
) -> PlatformSystemMaintenanceState {
    match service_exists(service, windows, cancellation) {
        Ok(true) => available(task_id, requires_elevation),
        Ok(false) => unavailable(
            task_id,
            requires_elevation,
            PlatformSystemMaintenanceDiagnosticCode::ComponentUnavailable,
        ),
        Err(_) => unavailable(
            task_id,
            requires_elevation,
            PlatformSystemMaintenanceDiagnosticCode::CheckFailed,
        ),
    }
}

fn service_exists(
    service: &str,
    windows: &Path,
    cancellation: &PlatformCancellation,
) -> PlatformResult<bool> {
    match run(
        &system_executable(windows, "sc.exe"),
        &["query", service],
        cancellation,
        DEFAULT_LIMITS,
        "windows_maintenance_service_query",
        CommandEffect::ReadOnly,
    ) {
        Ok(_) => Ok(true),
        Err(error) if error.code() == PlatformErrorCode::OperationFailed => Ok(false),
        Err(error) => Err(error),
    }
}

fn service_running(
    service: &str,
    windows: &Path,
    cancellation: &PlatformCancellation,
) -> PlatformResult<bool> {
    let output = run(
        &system_executable(windows, "sc.exe"),
        &["query", service],
        cancellation,
        DEFAULT_LIMITS,
        "windows_maintenance_service_query",
        CommandEffect::ReadOnly,
    )?;
    Ok(String::from_utf8_lossy(&output).contains("RUNNING"))
}

fn run_powershell(
    windows: &Path,
    script: &str,
    cancellation: &PlatformCancellation,
    limits: ControlledCommandLimits,
) -> PlatformResult<Vec<u8>> {
    run(
        &powershell_path(windows),
        &["-NoProfile", "-NonInteractive", "-Command", script],
        cancellation,
        limits,
        "windows_maintenance_powershell",
        CommandEffect::MayMutate,
    )
}

fn run_read_only_powershell(
    windows: &Path,
    script: &str,
    cancellation: &PlatformCancellation,
    limits: ControlledCommandLimits,
) -> PlatformResult<Vec<u8>> {
    run(
        &powershell_path(windows),
        &["-NoProfile", "-NonInteractive", "-Command", script],
        cancellation,
        limits,
        "windows_maintenance_powershell_query",
        CommandEffect::ReadOnly,
    )
}

fn explorer_running(windows: &Path, cancellation: &PlatformCancellation) -> PlatformResult<bool> {
    let output = run_read_only_powershell(
        windows,
        EXPLORER_PROCESS_QUERY_SCRIPT,
        cancellation,
        DEFAULT_LIMITS,
    )?;
    Ok(String::from_utf8_lossy(&output).trim() == "running")
}

/// A postcondition query happens after the native maintenance command may have changed system
/// state. Preserve that ordering in the error contract so Core never labels a verification-tool
/// failure as a safe, unchanged retry.
fn verify_after_mutation<T>(result: PlatformResult<T>) -> PlatformResult<T> {
    result.map_err(|error| {
        let reason = if error.code() == PlatformErrorCode::AccessDenied {
            crate::PlatformFailureReason::VerificationPermissionDenied
        } else {
            crate::PlatformFailureReason::VerificationFailed
        };
        error
            .with_possible_side_effects()
            .with_failure_reason(reason)
    })
}

fn run_with_privileges(
    task_id: &str,
    progress: &PlatformSystemMaintenanceProgressSink,
) -> PlatformResult<PrivilegedMaintenanceOutcome> {
    crate::system_maintenance_helper::execute_with_privileges(task_id, progress)
}

/// Executes one privileged catalog entry after the dedicated helper has verified its own token.
///
/// This entry point remains crate-private so neither Core nor the frontend can bypass the helper
/// protocol or provide executable material. Every script and executable is selected again from
/// the platform-owned allowlist inside the elevated process.
pub(crate) fn execute_with_current_privileges(
    task_id: &str,
    progress: &PlatformSystemMaintenanceProgressSink,
) -> PrivilegedMaintenanceResult {
    let body = task_scripts::recipe(task_id).ok_or_else(|| {
        PlatformError::new(
            PlatformErrorCode::Unsupported,
            "unsupported maintenance recipe",
        )
    })?;
    let windows = windows_directory()?;
    let phase = match task_id {
        SYSTEM_INTEGRITY => PlatformSystemMaintenancePhase::RepairingComponentImage,
        SEARCH_INDEX => PlatformSystemMaintenancePhase::RebuildingSearchIndex,
        UPDATE_COMPONENTS => PlatformSystemMaintenancePhase::RestartingServices,
        PRINT_QUEUE => PlatformSystemMaintenancePhase::RepairingPrintQueue,
        PERFORMANCE_COUNTERS => PlatformSystemMaintenancePhase::RebuildingPerformanceCounters,
        SYSTEM_DISK => PlatformSystemMaintenancePhase::CheckingSystemDisk,
        AUDIO_SERVICE => PlatformSystemMaintenancePhase::RestartingAudioService,
        STORE_CACHE => PlatformSystemMaintenancePhase::ResettingStoreCache,
        TIME_SYNC => PlatformSystemMaintenancePhase::SynchronizingTime,
        DNS_CACHE => PlatformSystemMaintenancePhase::RefreshingNetwork,
        _ => unreachable!("validated maintenance recipe"),
    };
    let (mut channel, setup_error) = match ElevatedProgressChannel::bind() {
        Ok(channel) => (Some(channel), None),
        Err(error) => (None, error.raw_os_error()),
    };
    let script = maintenance_script(&windows, body, channel.as_ref());
    run_privileged_powershell(
        &windows,
        task_id,
        &script,
        phase,
        progress,
        channel.as_mut(),
        setup_error,
    )
}

fn maintenance_script(
    windows: &Path,
    body: &str,
    channel: Option<&ElevatedProgressChannel>,
) -> String {
    MAINTENANCE_STEPS_SCRIPT
        .replace("__BODY__", body)
        .replace(
            "__WINDOWS__",
            &windows.to_string_lossy().replace('\'', "''"),
        )
        .replace(
            "__PORT__",
            &channel.map_or(0, |channel| channel.port).to_string(),
        )
        .replace(
            "__TOKEN__",
            channel.map_or("", |channel| channel.token.as_str()),
        )
}

fn run_privileged_powershell(
    windows: &Path,
    task_id: &str,
    script: &str,
    phase: PlatformSystemMaintenancePhase,
    progress: &PlatformSystemMaintenanceProgressSink,
    channel: Option<&mut ElevatedProgressChannel>,
    progress_channel_setup_error_code: Option<i32>,
) -> PrivilegedMaintenanceResult {
    let executable = powershell_path(windows);
    if !executable.is_file() {
        return Err(PlatformError::new(
            PlatformErrorCode::Unsupported,
            "Windows PowerShell is unavailable",
        )
        .into());
    }
    // PowerShell's UTF-16LE encoded-command transport avoids fragile nested quoting. The payload
    // is selected from the fixed platform catalog after helper authentication and never crosses
    // the IPC boundary, so a WebView request cannot repurpose elevation for arbitrary commands.
    let encoded_script = STANDARD.encode(
        script
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<_>>(),
    );
    let parameters = format!(
        "-NoProfile -NonInteractive -ExecutionPolicy Bypass -EncodedCommand {encoded_script}"
    );
    run_privileged_process(
        &executable,
        &parameters,
        task_id,
        phase,
        progress,
        channel,
        progress_channel_setup_error_code,
    )
}

fn run_privileged_process(
    executable: &Path,
    parameters: &str,
    _task_id: &str,
    phase: PlatformSystemMaintenancePhase,
    progress: &PlatformSystemMaintenanceProgressSink,
    mut channel: Option<&mut ElevatedProgressChannel>,
    progress_channel_setup_error_code: Option<i32>,
) -> PrivilegedMaintenanceResult {
    let started = Instant::now();
    let progress_channel_enabled = channel.is_some();
    let mut progress_channel_authenticated = false;
    let mut progress_channel_failed = false;
    let mut progress_channel_error_code = None;
    let mut progress_event_count = 0;
    let mut progress_rejected_connection_count = 0;
    let executable = wide(executable.as_os_str());
    let parameters = wide(OsStr::new(parameters));
    let mut execution = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS,
        lpFile: executable.as_ptr(),
        lpParameters: parameters.as_ptr(),
        nShow: SW_HIDE,
        ..unsafe { std::mem::zeroed() }
    };
    if unsafe { ShellExecuteExW(&mut execution) } == 0 {
        let error_code = unsafe { GetLastError() } as i32;
        return Err(PrivilegedMaintenanceFailure::with_native_error(
            PlatformError::operation_failed(format!(
                "system maintenance privileged process launch failed: error_code={error_code}",
            )),
            PrivilegedFailureStage::Launch,
            error_code,
        ));
    }
    if execution.hProcess.is_null() {
        return Err(PrivilegedMaintenanceFailure::new(
            PlatformError::operation_failed(
                "system maintenance elevated process handle is unavailable",
            )
            .with_possible_side_effects(),
            PrivilegedFailureStage::Launch,
        ));
    }
    progress(PlatformSystemMaintenanceProgress::phase(phase));
    let mut steps = Vec::new();
    let wait = loop {
        if let Some(progress_channel) = channel.as_deref_mut() {
            let poll_result = progress_channel.poll(progress);
            progress_channel_authenticated = progress_channel.authenticated;
            progress_event_count = progress_channel.event_count;
            progress_rejected_connection_count = progress_channel.rejected_connection_count;
            if let Err(error) = poll_result {
                progress_channel_failed = true;
                progress_channel_error_code = error.raw_os_error();
                // Stop consuming a broken channel without discarding previously accepted
                // evidence. Native failure classification and parent-side logs still need it.
                steps = std::mem::take(&mut progress_channel.steps);
                // Closing the reader also lets the child's best-effort writer stop sending.
                progress_channel.stream = None;
                channel = None;
            }
        }
        let wait = unsafe { WaitForSingleObject(execution.hProcess, 250) };
        if wait != WAIT_TIMEOUT {
            break wait;
        }
    };
    let wait_error_code = (wait == WAIT_FAILED).then(|| unsafe { GetLastError() });
    if let Some(progress_channel) = channel {
        let poll_result = progress_channel.poll(progress);
        progress_channel_authenticated = progress_channel.authenticated;
        progress_event_count = progress_channel.event_count;
        progress_rejected_connection_count = progress_channel.rejected_connection_count;
        steps = std::mem::take(&mut progress_channel.steps);
        if let Err(error) = poll_result {
            progress_channel_failed = true;
            progress_channel_error_code = error.raw_os_error();
        }
    }
    let mut exit_code = u32::MAX;
    let read = unsafe { GetExitCodeProcess(execution.hProcess, &mut exit_code) };
    unsafe { CloseHandle(execution.hProcess) };
    let diagnostics = PrivilegedProcessDiagnostics {
        wait_status: wait,
        wait_error_code,
        exit_code_read_succeeded: read != 0,
        exit_code,
        progress_channel_enabled,
        progress_channel_authenticated,
        progress_channel_failed,
        progress_channel_error_code,
        progress_channel_setup_failed: progress_channel_setup_error_code.is_some(),
        progress_channel_setup_error_code,
        progress_rejected_connection_count,
        progress_event_count,
        elapsed_ms: started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
        steps: steps.into_boxed_slice(),
    };
    privileged_execution_from_status(diagnostics)
}

struct ElevatedProgressChannel {
    listener: TcpListener,
    port: u16,
    stream: Option<TcpStream>,
    token: String,
    authenticated: bool,
    rejected_connection_count: u32,
    event_count: u32,
    buffer: Vec<u8>,
    steps: Vec<MaintenanceStepDiagnostic>,
}

impl ElevatedProgressChannel {
    fn bind() -> std::io::Result<Self> {
        let listener = TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))?;
        listener.set_nonblocking(true)?;
        let port = listener.local_addr()?.port();
        let sequence = PROGRESS_CHANNEL_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        Ok(Self {
            listener,
            port,
            stream: None,
            token: format!("{:x}{:x}{:x}", std::process::id(), sequence, timestamp),
            authenticated: false,
            rejected_connection_count: 0,
            event_count: 0,
            buffer: Vec::with_capacity(512),
            steps: Vec::new(),
        })
    }

    fn poll(&mut self, progress: &PlatformSystemMaintenanceProgressSink) -> std::io::Result<()> {
        if self.stream.is_none() {
            match self.listener.accept() {
                Ok((stream, _)) => {
                    stream.set_nonblocking(true)?;
                    self.stream = Some(stream);
                }
                Err(error) if error.kind() == ErrorKind::WouldBlock => return Ok(()),
                Err(error) => return Err(error),
            }
        }

        let mut chunk = [0_u8; 512];
        let mut peer_closed = false;
        if let Some(stream) = self.stream.as_mut() {
            loop {
                match stream.read(&mut chunk) {
                    Ok(0) => {
                        peer_closed = true;
                        break;
                    }
                    Ok(length) => {
                        if self.buffer.len().saturating_add(length) > PROGRESS_CHANNEL_BUFFER_LIMIT
                        {
                            return Err(std::io::Error::new(
                                ErrorKind::InvalidData,
                                "maintenance progress message exceeded its limit",
                            ));
                        }
                        self.buffer.extend_from_slice(&chunk[..length]);
                    }
                    Err(error) if error.kind() == ErrorKind::WouldBlock => break,
                    Err(error) => return Err(error),
                }
            }
        }
        let mut rejected = false;
        while let Some(newline) = self.buffer.iter().position(|byte| *byte == b'\n') {
            let line = self.buffer.drain(..=newline).collect::<Vec<_>>();
            let line = String::from_utf8_lossy(&line);
            let line = line.trim();
            if !self.authenticated {
                if line == self.token {
                    self.authenticated = true;
                } else {
                    rejected = true;
                    break;
                }
                continue;
            }
            if line.starts_with('{') {
                let record: MaintenanceStepDiagnostic =
                    serde_json::from_str(line).map_err(|_| {
                        std::io::Error::new(
                            ErrorKind::InvalidData,
                            "invalid maintenance step diagnostic",
                        )
                    })?;
                if record.index == 0
                    || record.index > 32
                    || usize::from(record.index) > self.steps.len() + 1
                {
                    return Err(std::io::Error::new(
                        ErrorKind::InvalidData,
                        "invalid maintenance step index",
                    ));
                }
                let index = usize::from(record.index - 1);
                if index == self.steps.len() {
                    if record.stage == MaintenanceStage::Verify {
                        progress(PlatformSystemMaintenanceProgress::phase(
                            PlatformSystemMaintenancePhase::Verifying,
                        ));
                    }
                    self.steps.push(record);
                } else {
                    self.steps[index] = record;
                }
                self.event_count = self.event_count.saturating_add(1);
            } else if let Some(value) = parse_system_integrity_progress(line) {
                self.event_count = self.event_count.saturating_add(1);
                progress(value);
            }
        }
        if !self.authenticated && (rejected || peer_closed) {
            // Only the fixed elevated script knows the per-run token. Releasing an invalid or
            // abandoned first connection allows the legitimate elevated process to connect on a
            // later poll instead of silently losing all progress for the rest of a long repair.
            self.stream = None;
            self.buffer.clear();
            self.rejected_connection_count = self.rejected_connection_count.saturating_add(1);
        }
        Ok(())
    }
}

fn parse_system_integrity_progress(line: &str) -> Option<PlatformSystemMaintenanceProgress> {
    let (phase, percent) = line.split_once('|')?;
    let percent = if percent.is_empty() {
        None
    } else {
        Some(percent.parse::<u8>().ok()?.min(100))
    };
    match phase {
        "repairingComponentImage" => Some(PlatformSystemMaintenanceProgress::step(
            PlatformSystemMaintenancePhase::RepairingComponentImage,
            1,
            2,
            percent,
        )),
        "checkingSystemFiles" => Some(PlatformSystemMaintenanceProgress::step(
            PlatformSystemMaintenancePhase::CheckingSystemFiles,
            2,
            2,
            percent,
        )),
        _ => None,
    }
}

fn privileged_execution_from_status(
    diagnostics: PrivilegedProcessDiagnostics,
) -> PrivilegedMaintenanceResult {
    let failure_stage = if diagnostics.wait_status != WAIT_OBJECT_0 {
        Some(PrivilegedFailureStage::Wait)
    } else if !diagnostics.exit_code_read_succeeded {
        Some(PrivilegedFailureStage::ExitCodeRead)
    } else if !matches!(diagnostics.exit_code, 0 | 3010) {
        Some(PrivilegedFailureStage::ProcessExit)
    } else {
        None
    };
    if let Some(stage) = failure_stage {
        let step_failure = diagnostics
            .steps
            .iter()
            .find(|record| record.result == MaintenanceStepResult::Failed);
        let error = step_failure
            .map(classify_failure)
            .unwrap_or_else(|| {
                PlatformError::operation_failed(format!(
                    "maintenance process failed: stage={stage:?} exit_code={}",
                    diagnostics.exit_code
                ))
            })
            .with_possible_side_effects();
        let native_error_code = step_failure.and_then(|record| record.native_error);
        let mut failure = PrivilegedMaintenanceFailure::with_diagnostics(error, stage, diagnostics);
        failure.native_error_code = native_error_code;
        return Err(failure);
    }
    Ok(PrivilegedMaintenanceOutcome {
        requires_restart: diagnostics.exit_code == 3010,
        diagnostics,
    })
}

fn run(
    path: &Path,
    arguments: &[&str],
    cancellation: &PlatformCancellation,
    limits: ControlledCommandLimits,
    command_id: &'static str,
    effect: CommandEffect,
) -> PlatformResult<Vec<u8>> {
    let executable = ControlledExecutable::capture(path).map_err(command_error)?;
    let output = run_controlled_command(
        command_id,
        &executable,
        arguments,
        ControlledEnvironmentPolicy::Inherit,
        limits,
        &|| cancellation.is_cancelled(),
    )
    .map_err(|error| command_execution_error(error, effect))?;
    if !output.status.success() {
        let stdout_digest = blake3::hash(&output.stdout).to_hex().to_string();
        let stderr_bytes = output.stderr_bytes;
        log::warn!("windows_maintenance_command_failed command_id={command_id} effect={effect:?} exit_code={:?} stdout_digest={stdout_digest} stderr_bytes={stderr_bytes}", output.status.code());
        let error = PlatformError::operation_failed(format!("maintenance command failed: command_id={command_id} exit_code={:?} stdout_digest={stdout_digest} stderr_bytes={stderr_bytes}", output.status.code()));
        return Err(match effect {
            CommandEffect::ReadOnly => error,
            CommandEffect::MayMutate => error.with_possible_side_effects(),
        });
    }
    Ok(output.stdout)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CommandEffect {
    ReadOnly,
    MayMutate,
}

fn command_execution_error(error: ControlledCommandError, effect: CommandEffect) -> PlatformError {
    let process_never_started = matches!(
        error,
        ControlledCommandError::InvalidExecutable
            | ControlledCommandError::ExecutableChanged
            | ControlledCommandError::SpawnFailed
    );
    let error = command_error(error);
    if effect == CommandEffect::MayMutate && !process_never_started {
        error.with_possible_side_effects()
    } else {
        error
    }
}

fn command_error(error: ControlledCommandError) -> PlatformError {
    let timed_out = matches!(error, ControlledCommandError::TimedOut);
    let diagnostic = format!("maintenance command could not complete: reason={error:?}");
    let error = PlatformError::new(
        match error {
            ControlledCommandError::Cancelled => PlatformErrorCode::UserCancelled,
            ControlledCommandError::InvalidExecutable
            | ControlledCommandError::ExecutableChanged => PlatformErrorCode::Unsupported,
            ControlledCommandError::SpawnFailed
            | ControlledCommandError::ReaderFailed
            | ControlledCommandError::WaitFailed
            | ControlledCommandError::TimedOut
            | ControlledCommandError::OutputLimitExceeded => PlatformErrorCode::OperationFailed,
        },
        diagnostic,
    );
    if timed_out {
        error.with_failure_reason(crate::PlatformFailureReason::TimedOut)
    } else {
        error
    }
}

fn windows_directory() -> PlatformResult<PathBuf> {
    let mut buffer = vec![0_u16; 32_768];
    let length = unsafe { GetWindowsDirectoryW(buffer.as_mut_ptr(), buffer.len() as u32) };
    if length == 0 || length as usize >= buffer.len() {
        return Err(PlatformError::operation_failed(
            "Windows directory is unavailable",
        ));
    }
    buffer.truncate(length as usize);
    let path = PathBuf::from(String::from_utf16(&buffer).map_err(|_| {
        PlatformError::new(
            PlatformErrorCode::InvalidData,
            "Windows directory contains invalid text",
        )
    })?);
    if !path.is_absolute() || !path.is_dir() {
        return Err(PlatformError::new(
            PlatformErrorCode::InvalidPath,
            "Windows directory is invalid",
        ));
    }
    Ok(path)
}

fn system_executable(windows: &Path, name: &str) -> PathBuf {
    windows.join("System32").join(name)
}

fn wbem_executable(windows: &Path, name: &str) -> PathBuf {
    windows.join("System32").join("wbem").join(name)
}

fn powershell_path(windows: &Path) -> PathBuf {
    windows
        .join("System32")
        .join("WindowsPowerShell")
        .join("v1.0")
        .join("powershell.exe")
}

fn wide(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain(std::iter::once(0)).collect()
}

fn validate_ids(task_ids: &[&str]) -> PlatformResult<()> {
    if task_ids.is_empty()
        || task_ids
            .iter()
            .any(|task_id| !SUPPORTED_TASKS.contains(task_id))
    {
        return Err(PlatformError::new(
            PlatformErrorCode::Unsupported,
            "system maintenance task identifier is unsupported",
        ));
    }
    Ok(())
}

fn availability_state(
    task_id: &str,
    is_available: bool,
    requires_elevation: bool,
) -> PlatformSystemMaintenanceState {
    if is_available {
        available(task_id, requires_elevation)
    } else {
        unavailable(
            task_id,
            requires_elevation,
            PlatformSystemMaintenanceDiagnosticCode::ToolUnavailable,
        )
    }
}

fn available(task_id: &str, requires_elevation: bool) -> PlatformSystemMaintenanceState {
    PlatformSystemMaintenanceState {
        task_id: task_id.to_string(),
        status: PlatformSystemMaintenanceStatus::Available,
        requires_elevation,
        diagnostic: None,
    }
}

fn unavailable(
    task_id: &str,
    requires_elevation: bool,
    diagnostic: PlatformSystemMaintenanceDiagnosticCode,
) -> PlatformSystemMaintenanceState {
    PlatformSystemMaintenanceState {
        task_id: task_id.to_string(),
        status: PlatformSystemMaintenanceStatus::Unavailable,
        requires_elevation,
        diagnostic: Some(diagnostic),
    }
}

fn execution(
    task_id: &str,
    changed: bool,
    verified: bool,
    requires_restart: bool,
    started: bool,
) -> PlatformSystemMaintenanceExecution {
    PlatformSystemMaintenanceExecution {
        task_id: task_id.to_string(),
        changed,
        verified,
        requires_restart,
        completion: if started {
            PlatformSystemMaintenanceCompletion::Started
        } else {
            PlatformSystemMaintenanceCompletion::Completed
        },
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    fn process_diagnostics(
        wait_status: u32,
        exit_code_read: i32,
        exit_code: u32,
    ) -> PrivilegedProcessDiagnostics {
        PrivilegedProcessDiagnostics {
            wait_status,
            wait_error_code: (wait_status == WAIT_FAILED).then_some(1),
            exit_code_read_succeeded: exit_code_read != 0,
            exit_code,
            progress_channel_enabled: false,
            progress_channel_authenticated: false,
            progress_channel_failed: false,
            progress_channel_error_code: None,
            progress_channel_setup_failed: false,
            progress_channel_setup_error_code: None,
            progress_rejected_connection_count: 0,
            progress_event_count: 0,
            elapsed_ms: 1,
            steps: Box::new([]),
        }
    }

    #[test]
    fn unknown_task_identifiers_are_rejected() {
        assert!(validate_ids(&["windows.maintenance.unknown"]).is_err());
    }

    #[test]
    fn progress_channel_rejects_an_invalid_peer_then_accepts_the_elevated_writer() {
        let mut channel = ElevatedProgressChannel::bind().expect("progress channel must bind");
        let mut invalid = TcpStream::connect((Ipv4Addr::LOCALHOST, channel.port))
            .expect("invalid peer must connect");
        invalid
            .write_all(b"invalid-token\n")
            .expect("invalid peer must write");
        let observed = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink_output = std::sync::Arc::clone(&observed);
        let sink = move |progress| {
            sink_output
                .lock()
                .expect("progress collection must remain available")
                .push(progress);
        };
        for _ in 0..20 {
            channel.poll(&sink).expect("invalid peer must be contained");
            if channel.rejected_connection_count == 1 {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(channel.rejected_connection_count, 1);
        assert!(!channel.authenticated);
        assert!(channel.stream.is_none());

        let mut valid = TcpStream::connect((Ipv4Addr::LOCALHOST, channel.port))
            .expect("elevated writer must connect after rejection");
        valid
            .write_all(format!("{}\ncheckingSystemFiles|42\n", channel.token).as_bytes())
            .expect("elevated writer must publish progress");
        for _ in 0..20 {
            channel
                .poll(&sink)
                .expect("valid progress must be accepted");
            if channel.event_count == 1 {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }

        assert!(channel.authenticated);
        assert_eq!(channel.event_count, 1);
        assert_eq!(
            observed
                .lock()
                .expect("progress collection must remain available")
                .as_slice(),
            &[PlatformSystemMaintenanceProgress::step(
                PlatformSystemMaintenancePhase::CheckingSystemFiles,
                2,
                2,
                Some(42),
            )]
        );
    }

    #[test]
    fn privileged_exit_code_preserves_restart_and_uncertain_failure_states() {
        assert!(
            !privileged_execution_from_status(process_diagnostics(WAIT_OBJECT_0, 1, 0))
                .expect("zero exit must succeed")
                .requires_restart
        );
        assert!(
            privileged_execution_from_status(process_diagnostics(WAIT_OBJECT_0, 1, 3010))
                .expect("3010 must remain a successful restart-required result")
                .requires_restart
        );
        let error = privileged_execution_from_status(process_diagnostics(WAIT_OBJECT_0, 1, 1))
            .expect_err("a failed privileged process may have already changed system state");
        assert_eq!(
            error.error.mutation_state(),
            crate::PlatformMutationState::MayHaveChanged
        );
        assert_eq!(error.stage, PrivilegedFailureStage::ProcessExit);
        assert_eq!(
            error
                .diagnostics
                .expect("native diagnostics must survive")
                .exit_code,
            1
        );
    }

    #[test]
    fn parses_only_typed_elevated_progress() {
        assert_eq!(
            parse_system_integrity_progress("repairingComponentImage|37"),
            Some(PlatformSystemMaintenanceProgress::step(
                PlatformSystemMaintenancePhase::RepairingComponentImage,
                1,
                2,
                Some(37),
            ))
        );
        assert_eq!(
            parse_system_integrity_progress("checkingSystemFiles|"),
            Some(PlatformSystemMaintenanceProgress::step(
                PlatformSystemMaintenancePhase::CheckingSystemFiles,
                2,
                2,
                None,
            ))
        );
        assert!(parse_system_integrity_progress("unknown|10").is_none());
        assert!(parse_system_integrity_progress("checkingSystemFiles|invalid").is_none());
    }

    #[test]
    fn mutating_command_spawn_failure_is_known_to_leave_system_unchanged() {
        let spawn_error = command_execution_error(
            ControlledCommandError::SpawnFailed,
            CommandEffect::MayMutate,
        );
        assert_eq!(
            spawn_error.mutation_state(),
            crate::PlatformMutationState::NotAttempted
        );

        let timeout_error =
            command_execution_error(ControlledCommandError::TimedOut, CommandEffect::MayMutate);
        assert_eq!(
            timeout_error.mutation_state(),
            crate::PlatformMutationState::MayHaveChanged
        );

        let read_only_error =
            command_execution_error(ControlledCommandError::TimedOut, CommandEffect::ReadOnly);
        assert_eq!(
            read_only_error.mutation_state(),
            crate::PlatformMutationState::NotAttempted
        );
    }

    /// Runs one real maintenance task on a disposable or explicitly authorized Windows host.
    ///
    /// The environment variable keeps the destructive scope finite and makes each invocation
    /// independently attributable in logs. `MANGODISK_TEST_MAINTENANCE_HELPER_EXE` must point to
    /// the built MangoDisk executable because the Rust test harness cannot enter application
    /// helper mode. Keeping this test ignored prevents ordinary CI and contributor test runs from
    /// restarting Explorer, services, or long-running repair tools.
    #[test]
    #[ignore = "changes Windows system state; run one authorized task at a time"]
    fn actual_maintenance_task_executes_and_verifies() {
        let task_id = std::env::var("MANGODISK_MAINTENANCE_TASK")
            .expect("MANGODISK_MAINTENANCE_TASK must name one supported maintenance task");
        assert!(SUPPORTED_TASKS.contains(&task_id.as_str()));

        let cancellation = PlatformCancellation::new(|| false);
        let state = scan(&[task_id.as_str()], &cancellation)
            .expect("the maintenance task must be scannable on this Windows host")
            .into_iter()
            .next()
            .expect("the platform must return the requested maintenance task");
        assert_ne!(state.status, PlatformSystemMaintenanceStatus::Unavailable);
        if task_id == STORE_CACHE {
            assert!(state.requires_elevation);
        }

        let observed_progress = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let progress_output = std::sync::Arc::clone(&observed_progress);
        let progress_sink = move |progress: PlatformSystemMaintenanceProgress| {
            println!(
                "phase={:?} step={:?}/{:?} percent={:?}",
                progress.phase, progress.current_step, progress.total_steps, progress.percent
            );
            progress_output
                .lock()
                .expect("progress test mutex must remain available")
                .push(progress);
        };
        let outcome = execute(&task_id, &cancellation, None, &progress_sink)
            .expect("the authorized maintenance task must execute successfully");
        assert_eq!(outcome.task_id, task_id);
        assert_eq!(outcome.changed, task_id != SYSTEM_DISK);
        assert!(outcome.verified);
        if task_id == SYSTEM_INTEGRITY {
            let progress = observed_progress
                .lock()
                .expect("progress test mutex must remain available");
            assert!(progress.iter().any(|value| {
                value.phase == PlatformSystemMaintenancePhase::RepairingComponentImage
            }));
            assert!(progress.iter().any(|value| {
                value.phase == PlatformSystemMaintenancePhase::CheckingSystemFiles
            }));
            assert!(progress.iter().any(|value| value.percent.is_some()));
        }
    }

    /// Executes two explicitly selected elevated tasks through one desktop-process session.
    ///
    /// The test validates the process-level behavior that protocol-only tests cannot prove: the
    /// second task must reuse the authenticated helper instead of issuing another `runas` launch.
    /// Both identifiers remain operator supplied because every supported elevated task changes or
    /// inspects real Windows state.
    #[test]
    #[ignore = "changes Windows system state and opens UAC; run only on an authorized VM"]
    fn actual_elevated_maintenance_tasks_reuse_one_helper_session() {
        let task_ids = std::env::var("MANGODISK_MAINTENANCE_REUSE_TASKS")
            .expect("MANGODISK_MAINTENANCE_REUSE_TASKS must contain two comma-separated task IDs")
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .collect::<Vec<_>>();
        assert_eq!(task_ids.len(), 2, "exactly two task IDs are required");
        assert!(task_ids
            .iter()
            .all(|task_id| SUPPORTED_TASKS.contains(&task_id.as_str())));

        let cancellation = PlatformCancellation::new(|| false);
        let task_refs = task_ids.iter().map(String::as_str).collect::<Vec<_>>();
        let states = scan(&task_refs, &cancellation)
            .expect("both maintenance tasks must be scannable on this Windows host");
        assert!(states.iter().all(|state| {
            state.status != PlatformSystemMaintenanceStatus::Unavailable && state.requires_elevation
        }));

        crate::system_maintenance_helper::reset_elevation_launch_count();
        let progress = |_progress: PlatformSystemMaintenanceProgress| {};
        for task_id in &task_ids {
            let outcome = execute(task_id, &cancellation, None, &progress)
                .expect("both authorized maintenance tasks must execute successfully");
            assert!(outcome.verified);
        }
        assert_eq!(
            crate::system_maintenance_helper::elevation_launch_count(),
            1,
            "the second task must reuse the first elevated helper process"
        );
    }
}
