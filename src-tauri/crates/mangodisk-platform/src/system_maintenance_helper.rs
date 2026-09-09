use std::{
    ffi::{OsStr, OsString},
    io::{BufRead, BufReader, ErrorKind, Read, Write},
    net::{Ipv4Addr, SocketAddrV4, TcpListener, TcpStream},
    os::windows::ffi::OsStrExt,
    ptr,
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};

use serde::{de::DeserializeOwned, Deserialize, Serialize};
use windows_sys::Win32::{
    Foundation::{CloseHandle, GetLastError, ERROR_CANCELLED, HANDLE, WAIT_FAILED, WAIT_OBJECT_0},
    Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY},
    System::Threading::{
        GetCurrentProcess, GetExitCodeProcess, OpenProcessToken, WaitForSingleObject,
    },
    UI::{
        Shell::{ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW},
        WindowsAndMessaging::SW_HIDE,
    },
};

use crate::{
    PlatformError, PlatformErrorCode, PlatformMutationState, PlatformResult,
    PlatformSystemMaintenancePhase, PlatformSystemMaintenanceProgress,
    PlatformSystemMaintenanceProgressSink,
};

pub(crate) mod step_diagnostics;

use step_diagnostics::{log_step_diagnostics, MaintenanceStepDiagnostic};

const HELPER_FLAG: &str = "--mangodisk-system-maintenance-helper-v3";
// The helper always comes from the running executable; reject mismatched schemas.
const PROTOCOL: &str = "mangodisk-system-maintenance-helper-v3";
const HELPER_START_TIMEOUT: Duration = Duration::from_secs(120);
const HELPER_POLL_INTERVAL: Duration = Duration::from_millis(50);
const MAX_MESSAGE_BYTES: usize = 64 * 1024;
const HELPER_FAILURE_EXIT_CODE: i32 = 70;

static SESSION: OnceLock<Mutex<Option<ElevatedMaintenanceSession>>> = OnceLock::new();
#[cfg(test)]
static ELEVATION_LAUNCH_COUNT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PrivilegedMaintenanceOutcome {
    pub requires_restart: bool,
    pub diagnostics: PrivilegedProcessDiagnostics,
}

#[derive(Debug)]
pub(crate) struct PrivilegedMaintenanceFailure {
    pub error: PlatformError,
    pub stage: PrivilegedFailureStage,
    pub native_error_code: Option<i32>,
    pub diagnostics: Option<PrivilegedProcessDiagnostics>,
}

pub(crate) type PrivilegedMaintenanceResult =
    Result<PrivilegedMaintenanceOutcome, PrivilegedMaintenanceFailure>;

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) enum PrivilegedFailureStage {
    Preparation,
    Launch,
    Wait,
    ExitCodeRead,
    ProcessExit,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PrivilegedProcessDiagnostics {
    pub wait_status: u32,
    pub wait_error_code: Option<u32>,
    pub exit_code_read_succeeded: bool,
    pub exit_code: u32,
    pub progress_channel_enabled: bool,
    pub progress_channel_authenticated: bool,
    pub progress_channel_failed: bool,
    pub progress_channel_error_code: Option<i32>,
    pub progress_channel_setup_failed: bool,
    pub progress_channel_setup_error_code: Option<i32>,
    pub progress_rejected_connection_count: u32,
    pub progress_event_count: u32,
    pub elapsed_ms: u64,
    pub steps: Box<[MaintenanceStepDiagnostic]>,
}

impl PrivilegedMaintenanceFailure {
    pub(crate) fn new(error: PlatformError, stage: PrivilegedFailureStage) -> Self {
        Self {
            error,
            stage,
            native_error_code: None,
            diagnostics: None,
        }
    }

    pub(crate) fn with_native_error(
        error: PlatformError,
        stage: PrivilegedFailureStage,
        native_error_code: i32,
    ) -> Self {
        Self {
            error,
            stage,
            native_error_code: Some(native_error_code),
            diagnostics: None,
        }
    }

    pub(crate) fn with_diagnostics(
        error: PlatformError,
        stage: PrivilegedFailureStage,
        diagnostics: PrivilegedProcessDiagnostics,
    ) -> Self {
        Self {
            error,
            stage,
            native_error_code: None,
            diagnostics: Some(diagnostics),
        }
    }
}

impl From<PlatformError> for PrivilegedMaintenanceFailure {
    fn from(error: PlatformError) -> Self {
        Self::new(error, PrivilegedFailureStage::Preparation)
    }
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
enum HelperRequest {
    Ping {
        protocol: String,
        token: String,
        request_id: u64,
    },
    Execute {
        protocol: String,
        token: String,
        request_id: u64,
        task_id: String,
    },
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
enum HelperEvent {
    Ready {
        protocol: String,
        token: String,
    },
    Pong {
        protocol: String,
        request_id: u64,
    },
    Progress {
        protocol: String,
        request_id: u64,
        phase: PlatformSystemMaintenancePhase,
        current_step: Option<u8>,
        total_steps: Option<u8>,
        percent: Option<u8>,
    },
    Completed {
        protocol: String,
        request_id: u64,
        requires_restart: bool,
        diagnostics: PrivilegedProcessDiagnostics,
    },
    Failed {
        protocol: String,
        request_id: u64,
        error_code: WireErrorCode,
        mutation_possible: bool,
        diagnostic_digest: String,
        failure_stage: PrivilegedFailureStage,
        native_error_code: Option<i32>,
        diagnostics: Option<PrivilegedProcessDiagnostics>,
    },
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
enum WireErrorCode {
    AccessDenied,
    UserCancelled,
    ItemChanged,
    InvalidData,
    InvalidPath,
    Io,
    OperationFailed,
    Unsupported,
}

struct ElevatedMaintenanceSession {
    writer: TcpStream,
    reader: BufReader<TcpStream>,
    process: HANDLE,
    token: String,
    next_request_id: u64,
    session_label: String,
    started_at: Instant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HelperTransportStage {
    WriteRequest,
    ReadResponse,
    CorrelateResponse,
}

enum SessionExecutionError {
    Transport {
        stage: HelperTransportStage,
        error: PlatformError,
    },
    Remote(PlatformError),
}

// Windows process handles remain valid across threads, and the enclosing session mutex serializes
// every wait and close. The session owns this handle exactly once for liveness diagnostics.
// Closing it does not terminate a task that may already be mutating the system; the helper exits
// after its TCP channel closes and any in-flight native task returns.
unsafe impl Send for ElevatedMaintenanceSession {}

impl Drop for ElevatedMaintenanceSession {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.process) };
    }
}

/// Runs the narrow Windows maintenance helper before Tauri initializes.
///
/// The helper accepts only stable task identifiers compiled into MangoDisk. The desktop process
/// cannot provide executable paths, arguments, scripts, or registry data across this boundary.
pub fn run_system_maintenance_helper_mode<I>(arguments: I) -> Option<i32>
where
    I: IntoIterator<Item = OsString>,
{
    let arguments = arguments.into_iter().collect::<Vec<_>>();
    if arguments.get(1).and_then(|value| value.to_str()) != Some(HELPER_FLAG) {
        return None;
    }
    let result = helper_arguments(&arguments).and_then(|(port, token)| {
        if !current_process_is_elevated()? {
            return Err(PlatformError::new(
                PlatformErrorCode::AccessDenied,
                "system maintenance helper is not elevated",
            ));
        }
        run_helper_session(port, &token)
    });
    Some(if result.is_ok() {
        0
    } else {
        HELPER_FAILURE_EXIT_CODE
    })
}

/// Executes one allowlisted task through a reusable, application-scoped elevated helper.
///
/// Windows grants elevation to a process rather than caching consent for later `runas` calls. A
/// single helper therefore remains alive while the desktop process owns its authenticated channel,
/// allowing later tasks to reuse the same UAC approval without elevating Tauri or the WebView.
pub(crate) fn execute_with_privileges(
    task_id: &str,
    progress: &PlatformSystemMaintenanceProgressSink,
) -> PlatformResult<PrivilegedMaintenanceOutcome> {
    let registry = SESSION.get_or_init(|| Mutex::new(None));
    let mut session = registry.lock().map_err(|_| {
        PlatformError::operation_failed("system maintenance helper session lock is unavailable")
    })?;

    let reusable = match session.as_mut() {
        Some(current) => match current.ping() {
            Ok(()) => true,
            Err(error) => {
                log::info!(
                    "windows_system_maintenance_helper_session_expired session_id={} code={:?} elapsed_ms={}",
                    current.session_label,
                    error.code(),
                    current.started_at.elapsed().as_millis()
                );
                false
            }
        },
        None => false,
    };
    if !reusable {
        *session = None;
        progress(PlatformSystemMaintenanceProgress::phase(
            PlatformSystemMaintenancePhase::WaitingForAuthorization,
        ));
        *session = Some(ElevatedMaintenanceSession::start()?);
    }

    let current = session.as_mut().ok_or_else(|| {
        PlatformError::operation_failed("system maintenance helper session is unavailable")
    })?;
    let result = current.execute(task_id, progress);
    match result {
        Ok(outcome) => Ok(outcome),
        Err(SessionExecutionError::Transport { stage, error }) => {
            // A broken response channel after dispatch cannot prove whether the native command ran.
            // Discard the session so the next explicit user action obtains a fresh authorization.
            log::warn!(
                "windows_system_maintenance_helper_session_discarded session_id={} reason=transport_failure stage={:?} code={:?} mutation_state={:?} error_digest={}",
                current.session_label,
                stage,
                error.code(),
                error.mutation_state(),
                blake3::hash(error.as_bytes()).to_hex()
            );
            *session = None;
            Err(error)
        }
        Err(SessionExecutionError::Remote(error)) => Err(error),
    }
}

impl ElevatedMaintenanceSession {
    fn start() -> PlatformResult<Self> {
        let listener = TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
            .map_err(|error| helper_io("bind system maintenance helper channel", &error))?;
        listener
            .set_nonblocking(true)
            .map_err(|error| helper_io("configure system maintenance helper channel", &error))?;
        let port = listener
            .local_addr()
            .map_err(|error| helper_io("read system maintenance helper address", &error))?
            .port();
        let token = secure_token()?;
        // Logs use a one-way correlation label rather than exposing any prefix of the bearer
        // capability sent to the elevated helper.
        let session_label = blake3::hash(token.as_bytes())
            .to_hex()
            .chars()
            .take(12)
            .collect::<String>();
        let process = launch_elevated_helper(port, &token, &session_label)?;
        let started = Instant::now();
        let mut rejected_connection_count = 0_u32;

        loop {
            if started.elapsed() >= HELPER_START_TIMEOUT {
                unsafe { CloseHandle(process) };
                return Err(PlatformError::operation_failed(
                    "system maintenance helper connection timed out",
                ));
            }
            let wait = unsafe { WaitForSingleObject(process, 0) };
            if wait == WAIT_OBJECT_0 || wait == WAIT_FAILED {
                let mut exit_code = HELPER_FAILURE_EXIT_CODE as u32;
                let read = unsafe { GetExitCodeProcess(process, &mut exit_code) };
                unsafe { CloseHandle(process) };
                log::warn!(
                    "windows_system_maintenance_helper_start_failed session_id={} wait_status={} exit_code_read_succeeded={} exit_code={} rejected_connection_count={} elapsed_ms={}",
                    session_label,
                    wait,
                    read != 0,
                    exit_code,
                    rejected_connection_count,
                    started.elapsed().as_millis()
                );
                return Err(PlatformError::operation_failed(
                    "system maintenance helper exited before authentication",
                ));
            }

            match listener.accept() {
                Ok((stream, _)) => match authenticate_helper(stream, &token) {
                    Ok((writer, reader)) => {
                        log::info!(
                            "windows_system_maintenance_helper_session_started session_id={} rejected_connection_count={} elapsed_ms={}",
                            session_label,
                            rejected_connection_count,
                            started.elapsed().as_millis()
                        );
                        return Ok(Self {
                            writer,
                            reader,
                            process,
                            token,
                            next_request_id: 1,
                            session_label,
                            started_at: Instant::now(),
                        });
                    }
                    Err(_) => {
                        rejected_connection_count = rejected_connection_count.saturating_add(1);
                    }
                },
                Err(error) if error.kind() == ErrorKind::WouldBlock => {
                    std::thread::sleep(HELPER_POLL_INTERVAL);
                }
                Err(error) => {
                    unsafe { CloseHandle(process) };
                    return Err(helper_io("accept system maintenance helper", &error));
                }
            }
        }
    }

    fn ping(&mut self) -> PlatformResult<()> {
        let request_id = self.take_request_id();
        write_message(
            &mut self.writer,
            &HelperRequest::Ping {
                protocol: PROTOCOL.to_owned(),
                token: self.token.clone(),
                request_id,
            },
        )?;
        match read_message::<HelperEvent>(&mut self.reader)? {
            HelperEvent::Pong {
                protocol,
                request_id: response_id,
            } if protocol == PROTOCOL && response_id == request_id => {
                log::info!(
                    "windows_system_maintenance_helper_session_reused session_id={} elapsed_ms={}",
                    self.session_label,
                    self.started_at.elapsed().as_millis()
                );
                Ok(())
            }
            _ => Err(PlatformError::new(
                PlatformErrorCode::InvalidData,
                "system maintenance helper ping correlation failed",
            )),
        }
    }

    fn execute(
        &mut self,
        task_id: &str,
        progress: &PlatformSystemMaintenanceProgressSink,
    ) -> Result<PrivilegedMaintenanceOutcome, SessionExecutionError> {
        let request_id = self.take_request_id();
        let started = Instant::now();
        write_message(
            &mut self.writer,
            &HelperRequest::Execute {
                protocol: PROTOCOL.to_owned(),
                token: self.token.clone(),
                request_id,
                task_id: task_id.to_owned(),
            },
        )
        .map_err(PlatformError::with_possible_side_effects)
        .map_err(|error| SessionExecutionError::Transport {
            stage: HelperTransportStage::WriteRequest,
            error,
        })?;
        loop {
            match read_message::<HelperEvent>(&mut self.reader)
                .map_err(PlatformError::with_possible_side_effects)
                .map_err(|error| SessionExecutionError::Transport {
                    stage: HelperTransportStage::ReadResponse,
                    error,
                })? {
                HelperEvent::Progress {
                    protocol,
                    request_id: response_id,
                    phase,
                    current_step,
                    total_steps,
                    percent,
                } if protocol == PROTOCOL && response_id == request_id => {
                    progress(PlatformSystemMaintenanceProgress {
                        phase,
                        current_step,
                        total_steps,
                        percent,
                    });
                }
                HelperEvent::Completed {
                    protocol,
                    request_id: response_id,
                    requires_restart,
                    diagnostics,
                } if protocol == PROTOCOL && response_id == request_id => {
                    log_privileged_execution(
                        &self.session_label,
                        task_id,
                        request_id,
                        "completed",
                        None,
                        &diagnostics,
                    );
                    log::info!(
                        "windows_system_maintenance_helper_task_finished session_id={} task_id={} request_id={} status=completed restart_required={} elapsed_ms={}",
                        self.session_label,
                        task_id,
                        request_id,
                        requires_restart,
                        started.elapsed().as_millis()
                    );
                    return Ok(PrivilegedMaintenanceOutcome {
                        requires_restart,
                        diagnostics,
                    });
                }
                HelperEvent::Failed {
                    protocol,
                    request_id: response_id,
                    error_code,
                    mutation_possible,
                    diagnostic_digest,
                    failure_stage,
                    native_error_code,
                    diagnostics,
                } if protocol == PROTOCOL && response_id == request_id => {
                    log::warn!(
                        "windows_system_maintenance_helper_task_failed session_id={} task_id={} request_id={} code={:?} failure_stage={:?} native_error_code={} mutation_possible={} diagnostic_digest={} elapsed_ms={}",
                        self.session_label,
                        task_id,
                        request_id,
                        error_code,
                        failure_stage,
                        native_error_code
                            .map(|value| value.to_string())
                            .unwrap_or_else(|| "none".to_string()),
                        mutation_possible,
                        diagnostic_digest,
                        started.elapsed().as_millis()
                    );
                    if let Some(diagnostics) = diagnostics.as_ref() {
                        log_privileged_execution(
                            &self.session_label,
                            task_id,
                            request_id,
                            "failed",
                            Some(failure_stage),
                            diagnostics,
                        );
                    }
                    let error = diagnostics.as_ref().and_then(|diagnostics| diagnostics.steps.iter().find(|record| record.result == step_diagnostics::MaintenanceStepResult::Failed))
                        .map(step_diagnostics::classify_failure).unwrap_or_else(|| PlatformError::new(
                        error_code.into(),
                        format!(
                            "system maintenance helper task failed: diagnostic_digest={diagnostic_digest}"
                        ),
                    ));
                    return Err(SessionExecutionError::Remote(if mutation_possible {
                        error.with_possible_side_effects()
                    } else {
                        error
                    }));
                }
                _ => {
                    return Err(SessionExecutionError::Transport {
                        stage: HelperTransportStage::CorrelateResponse,
                        error: PlatformError::new(
                            PlatformErrorCode::InvalidData,
                            "system maintenance helper response correlation failed",
                        )
                        .with_possible_side_effects(),
                    });
                }
            }
        }
    }

    fn take_request_id(&mut self) -> u64 {
        let current = self.next_request_id;
        self.next_request_id = self.next_request_id.saturating_add(1);
        current
    }
}

fn run_helper_session(port: u16, token: &str) -> PlatformResult<()> {
    let address = SocketAddrV4::new(Ipv4Addr::LOCALHOST, port);
    let stream = TcpStream::connect_timeout(&address.into(), Duration::from_secs(10))
        .map_err(|error| helper_io("connect system maintenance helper channel", &error))?;
    stream
        .set_nodelay(true)
        .map_err(|error| helper_io("configure system maintenance helper channel", &error))?;
    let mut writer = stream
        .try_clone()
        .map_err(|error| helper_io("clone system maintenance helper channel", &error))?;
    write_message(
        &mut writer,
        &HelperEvent::Ready {
            protocol: PROTOCOL.to_owned(),
            token: token.to_owned(),
        },
    )?;
    let writer = Arc::new(Mutex::new(writer));
    let mut reader = BufReader::new(stream);
    let mut last_request_id = 0_u64;

    loop {
        let request = match read_helper_request(&mut reader)? {
            HelperRead::Message(request) => request,
            HelperRead::Closed => return Ok(()),
        };
        match request {
            HelperRequest::Ping {
                protocol,
                token: request_token,
                request_id,
            } => {
                validate_request(
                    &protocol,
                    &request_token,
                    request_id,
                    last_request_id,
                    token,
                )?;
                last_request_id = request_id;
                let mut stream = writer.lock().map_err(|_| {
                    PlatformError::operation_failed(
                        "system maintenance helper writer lock is unavailable",
                    )
                })?;
                write_message(
                    &mut stream,
                    &HelperEvent::Pong {
                        protocol: PROTOCOL.to_owned(),
                        request_id,
                    },
                )?;
            }
            HelperRequest::Execute {
                protocol,
                token: request_token,
                request_id,
                task_id,
            } => {
                validate_request(
                    &protocol,
                    &request_token,
                    request_id,
                    last_request_id,
                    token,
                )?;
                last_request_id = request_id;
                let progress_writer = Arc::clone(&writer);
                let progress_sink = move |progress: PlatformSystemMaintenanceProgress| {
                    if let Ok(mut stream) = progress_writer.lock() {
                        let _ = write_message(
                            &mut stream,
                            &HelperEvent::Progress {
                                protocol: PROTOCOL.to_owned(),
                                request_id,
                                phase: progress.phase,
                                current_step: progress.current_step,
                                total_steps: progress.total_steps,
                                percent: progress.percent,
                            },
                        );
                    }
                };
                let result =
                    crate::windows::system_maintenance_helper_execute(&task_id, &progress_sink);
                let event = match result {
                    Ok(outcome) => HelperEvent::Completed {
                        protocol: PROTOCOL.to_owned(),
                        request_id,
                        requires_restart: outcome.requires_restart,
                        diagnostics: outcome.diagnostics,
                    },
                    Err(failure) => HelperEvent::Failed {
                        protocol: PROTOCOL.to_owned(),
                        request_id,
                        error_code: failure.error.code().into(),
                        mutation_possible: failure.error.mutation_state()
                            == PlatformMutationState::MayHaveChanged,
                        diagnostic_digest: blake3::hash(failure.error.as_bytes())
                            .to_hex()
                            .to_string(),
                        failure_stage: failure.stage,
                        native_error_code: failure.native_error_code,
                        diagnostics: failure.diagnostics,
                    },
                };
                let mut stream = writer.lock().map_err(|_| {
                    PlatformError::operation_failed(
                        "system maintenance helper writer lock is unavailable",
                    )
                })?;
                write_message(&mut stream, &event)?;
            }
        }
    }
}

enum HelperRead<T> {
    Message(T),
    Closed,
}

fn read_helper_request(
    reader: &mut BufReader<TcpStream>,
) -> PlatformResult<HelperRead<HelperRequest>> {
    let mut line = String::new();
    let length = match (&mut *reader)
        .take((MAX_MESSAGE_BYTES + 1) as u64)
        .read_line(&mut line)
    {
        Ok(length) => length,
        // A desktop crash or ordinary process teardown can close the cloned Windows socket with
        // a reset instead of a FIN. Both mean that the capability owner is gone, so the helper
        // must end normally rather than leave a misleading failure exit code behind.
        Err(error)
            if matches!(
                error.kind(),
                ErrorKind::ConnectionReset
                    | ErrorKind::ConnectionAborted
                    | ErrorKind::BrokenPipe
                    | ErrorKind::UnexpectedEof
            ) =>
        {
            return Ok(HelperRead::Closed);
        }
        Err(error) => {
            return Err(helper_io("read system maintenance helper request", &error));
        }
    };
    if length == 0 {
        return Ok(HelperRead::Closed);
    }
    if length > MAX_MESSAGE_BYTES {
        return Err(PlatformError::new(
            PlatformErrorCode::InvalidData,
            "system maintenance helper message is too large",
        ));
    }
    let request = serde_json::from_str(line.trim_end()).map_err(|_| {
        PlatformError::new(
            PlatformErrorCode::InvalidData,
            "system maintenance helper message is invalid",
        )
    })?;
    Ok(HelperRead::Message(request))
}

fn authenticate_helper(
    stream: TcpStream,
    token: &str,
) -> PlatformResult<(TcpStream, BufReader<TcpStream>)> {
    // Windows can propagate a listener's nonblocking state to an accepted socket. The listener
    // remains nonblocking so UAC startup can be polled, but an authenticated session must wait for
    // delayed native progress instead of treating an ordinary WouldBlock as a broken transport.
    stream.set_nonblocking(false).map_err(|error| {
        helper_io(
            "configure system maintenance helper blocking channel",
            &error,
        )
    })?;
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .map_err(|error| helper_io("configure system maintenance helper handshake", &error))?;
    stream
        .set_nodelay(true)
        .map_err(|error| helper_io("configure system maintenance helper channel", &error))?;
    let mut reader = BufReader::new(
        stream
            .try_clone()
            .map_err(|error| helper_io("clone system maintenance helper channel", &error))?,
    );
    match read_message::<HelperEvent>(&mut reader)? {
        HelperEvent::Ready {
            protocol,
            token: response_token,
        } if protocol == PROTOCOL && response_token == token => {
            reader.get_ref().set_read_timeout(None).map_err(|error| {
                helper_io(
                    "configure system maintenance helper response timeout",
                    &error,
                )
            })?;
            Ok((stream, reader))
        }
        _ => Err(PlatformError::new(
            PlatformErrorCode::AccessDenied,
            "system maintenance helper authentication failed",
        )),
    }
}

fn validate_request(
    protocol: &str,
    request_token: &str,
    request_id: u64,
    last_request_id: u64,
    expected_token: &str,
) -> PlatformResult<()> {
    if protocol != PROTOCOL || request_token != expected_token {
        return Err(PlatformError::new(
            PlatformErrorCode::AccessDenied,
            "system maintenance helper request authentication failed",
        ));
    }
    if request_id <= last_request_id {
        return Err(PlatformError::new(
            PlatformErrorCode::InvalidData,
            "system maintenance helper request sequence is invalid",
        ));
    }
    Ok(())
}

fn helper_arguments(arguments: &[OsString]) -> PlatformResult<(u16, String)> {
    if arguments.len() != 4 {
        return Err(PlatformError::new(
            PlatformErrorCode::InvalidData,
            "system maintenance helper arguments are invalid",
        ));
    }
    let port = arguments[2]
        .to_str()
        .and_then(|value| value.parse::<u16>().ok())
        .filter(|value| *value != 0)
        .ok_or_else(|| {
            PlatformError::new(
                PlatformErrorCode::InvalidData,
                "system maintenance helper port is invalid",
            )
        })?;
    let token = arguments[3].to_str().ok_or_else(|| {
        PlatformError::new(
            PlatformErrorCode::InvalidData,
            "system maintenance helper token is invalid",
        )
    })?;
    if token.len() != 64 || !token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(PlatformError::new(
            PlatformErrorCode::InvalidData,
            "system maintenance helper token is invalid",
        ));
    }
    Ok((port, token.to_ascii_lowercase()))
}

fn launch_elevated_helper(port: u16, token: &str, session_label: &str) -> PlatformResult<HANDLE> {
    #[cfg(test)]
    ELEVATION_LAUNCH_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let executable = helper_executable()?;
    if !executable.is_absolute() || !executable.is_file() {
        return Err(PlatformError::new(
            PlatformErrorCode::InvalidPath,
            "system maintenance helper executable is invalid",
        ));
    }
    let executable = wide(executable.as_os_str());
    let verb = wide(OsStr::new("runas"));
    let parameters = wide(OsStr::new(&format!("{HELPER_FLAG} {port} {token}")));
    let mut execution = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS,
        lpVerb: verb.as_ptr(),
        lpFile: executable.as_ptr(),
        lpParameters: parameters.as_ptr(),
        nShow: SW_HIDE,
        ..unsafe { std::mem::zeroed() }
    };
    log::info!("windows_system_maintenance_helper_elevation_started session_id={session_label}");
    if unsafe { ShellExecuteExW(&mut execution) } == 0 {
        let code = unsafe { GetLastError() };
        log::warn!(
            "windows_system_maintenance_helper_elevation_failed session_id={session_label} stage=launch error_code={code}"
        );
        return Err(PlatformError::new(
            if code == ERROR_CANCELLED {
                PlatformErrorCode::UserCancelled
            } else {
                PlatformErrorCode::OperationFailed
            },
            "system maintenance helper elevation request failed",
        ));
    }
    if execution.hProcess.is_null() {
        return Err(PlatformError::operation_failed(
            "system maintenance helper process handle is unavailable",
        ));
    }
    Ok(execution.hProcess)
}

#[cfg(test)]
pub(crate) fn reset_elevation_launch_count() {
    ELEVATION_LAUNCH_COUNT.store(0, std::sync::atomic::Ordering::Relaxed);
    if let Some(session) = SESSION.get() {
        *session
            .lock()
            .expect("the test helper session mutex must remain available") = None;
    }
}

#[cfg(test)]
pub(crate) fn elevation_launch_count() -> u64 {
    ELEVATION_LAUNCH_COUNT.load(std::sync::atomic::Ordering::Relaxed)
}

#[cfg(not(test))]
fn helper_executable() -> PlatformResult<std::path::PathBuf> {
    std::env::current_exe()
        .map_err(|error| helper_io("resolve system maintenance helper executable", &error))
}

#[cfg(test)]
fn helper_executable() -> PlatformResult<std::path::PathBuf> {
    // A Rust test harness does not dispatch application helper flags. Explicit real-host tests
    // therefore name a built MangoDisk executable, while production builds always use themselves.
    std::env::var_os("MANGODISK_TEST_MAINTENANCE_HELPER_EXE")
        .map(std::path::PathBuf::from)
        .ok_or_else(|| {
            PlatformError::new(
                PlatformErrorCode::Unsupported,
                "real maintenance tests require a built MangoDisk helper executable",
            )
        })
}

fn current_process_is_elevated() -> PlatformResult<bool> {
    let mut token = ptr::null_mut();
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(PlatformError::operation_failed(
            "query system maintenance helper process token failed",
        ));
    }
    let mut elevation = TOKEN_ELEVATION::default();
    let mut returned_bytes = 0_u32;
    let queried = unsafe {
        GetTokenInformation(
            token,
            TokenElevation,
            (&mut elevation as *mut TOKEN_ELEVATION).cast(),
            size_of::<TOKEN_ELEVATION>() as u32,
            &mut returned_bytes,
        )
    };
    unsafe { CloseHandle(token) };
    if queried == 0 || returned_bytes < size_of::<TOKEN_ELEVATION>() as u32 {
        return Err(PlatformError::operation_failed(
            "read system maintenance helper elevation failed",
        ));
    }
    Ok(elevation.TokenIsElevated != 0)
}

fn secure_token() -> PlatformResult<String> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes).map_err(|error| {
        PlatformError::operation_failed(format!(
            "generate system maintenance helper token: {error:?}"
        ))
    })?;
    let mut value = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(value, "{byte:02x}");
    }
    Ok(value)
}

fn write_message<T: Serialize>(stream: &mut TcpStream, message: &T) -> PlatformResult<()> {
    let mut bytes = serde_json::to_vec(message).map_err(|_| {
        PlatformError::new(
            PlatformErrorCode::InvalidData,
            "serialize system maintenance helper message failed",
        )
    })?;
    if bytes.len() >= MAX_MESSAGE_BYTES {
        return Err(PlatformError::new(
            PlatformErrorCode::InvalidData,
            "system maintenance helper message is too large",
        ));
    }
    bytes.push(b'\n');
    stream
        .write_all(&bytes)
        .and_then(|()| stream.flush())
        .map_err(|error| helper_io("write system maintenance helper message", &error))
}

fn read_message<T: DeserializeOwned>(reader: &mut BufReader<TcpStream>) -> PlatformResult<T> {
    let mut line = String::new();
    let length = (&mut *reader)
        .take((MAX_MESSAGE_BYTES + 1) as u64)
        .read_line(&mut line)
        .map_err(|error| helper_io("read system maintenance helper message", &error))?;
    if length == 0 {
        return Err(PlatformError::new(
            PlatformErrorCode::Io,
            "system maintenance helper channel closed",
        ));
    }
    if length > MAX_MESSAGE_BYTES {
        return Err(PlatformError::new(
            PlatformErrorCode::InvalidData,
            "system maintenance helper message is too large",
        ));
    }
    serde_json::from_str(line.trim_end()).map_err(|_| {
        PlatformError::new(
            PlatformErrorCode::InvalidData,
            "system maintenance helper message is invalid",
        )
    })
}

fn helper_io(operation: &'static str, error: &std::io::Error) -> PlatformError {
    PlatformError::io(operation, error)
}

fn log_privileged_execution(
    session_id: &str,
    task_id: &str,
    request_id: u64,
    status: &str,
    failure_stage: Option<PrivilegedFailureStage>,
    diagnostics: &PrivilegedProcessDiagnostics,
) {
    log_step_diagnostics(session_id, task_id, request_id, &diagnostics.steps);
    log::info!(
        "windows_system_maintenance_privileged_process_finished session_id={} task_id={} request_id={} status={} failure_stage={:?} wait_status={} wait_error_code={} exit_code_read_succeeded={} exit_code={} progress_channel_enabled={} progress_channel_authenticated={} progress_channel_failed={} progress_channel_error_code={} progress_channel_setup_failed={} progress_channel_setup_error_code={} progress_rejected_connection_count={} progress_event_count={} diagnostic_step_count={} native_elapsed_ms={}",
        session_id,
        task_id,
        request_id,
        status,
        failure_stage,
        diagnostics.wait_status,
        diagnostics
            .wait_error_code
            .map(|value| value.to_string())
            .unwrap_or_else(|| "none".to_string()),
        diagnostics.exit_code_read_succeeded,
        diagnostics.exit_code,
        diagnostics.progress_channel_enabled,
        diagnostics.progress_channel_authenticated,
        diagnostics.progress_channel_failed,
        diagnostics
            .progress_channel_error_code
            .map(|value| value.to_string())
            .unwrap_or_else(|| "none".to_string()),
        diagnostics.progress_channel_setup_failed,
        diagnostics
            .progress_channel_setup_error_code
            .map(|value| value.to_string())
            .unwrap_or_else(|| "none".to_string()),
        diagnostics.progress_rejected_connection_count,
        diagnostics.progress_event_count,
        diagnostics.steps.len(),
        diagnostics.elapsed_ms
    );
}

fn wide(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain(std::iter::once(0)).collect()
}

impl From<PlatformErrorCode> for WireErrorCode {
    fn from(value: PlatformErrorCode) -> Self {
        match value {
            PlatformErrorCode::AccessDenied => Self::AccessDenied,
            PlatformErrorCode::UserCancelled => Self::UserCancelled,
            PlatformErrorCode::ItemChanged => Self::ItemChanged,
            PlatformErrorCode::InvalidData => Self::InvalidData,
            PlatformErrorCode::InvalidPath => Self::InvalidPath,
            PlatformErrorCode::Io => Self::Io,
            PlatformErrorCode::OperationFailed => Self::OperationFailed,
            PlatformErrorCode::Unsupported => Self::Unsupported,
        }
    }
}

impl From<WireErrorCode> for PlatformErrorCode {
    fn from(value: WireErrorCode) -> Self {
        match value {
            WireErrorCode::AccessDenied => Self::AccessDenied,
            WireErrorCode::UserCancelled => Self::UserCancelled,
            WireErrorCode::ItemChanged => Self::ItemChanged,
            WireErrorCode::InvalidData => Self::InvalidData,
            WireErrorCode::InvalidPath => Self::InvalidPath,
            WireErrorCode::Io => Self::Io,
            WireErrorCode::OperationFailed => Self::OperationFailed,
            WireErrorCode::Unsupported => Self::Unsupported,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normal_launch_does_not_enter_helper_mode() {
        assert_eq!(
            run_system_maintenance_helper_mode([OsString::from("MangoDisk")]),
            None
        );
    }

    #[test]
    fn helper_arguments_require_a_bounded_hex_token() {
        let valid = vec![
            OsString::from("MangoDisk"),
            OsString::from(HELPER_FLAG),
            OsString::from("49152"),
            OsString::from("a".repeat(64)),
        ];
        assert_eq!(
            helper_arguments(&valid).expect("valid helper arguments must parse"),
            (49152, "a".repeat(64))
        );

        let mut invalid = valid;
        invalid[3] = OsString::from("not-a-capability");
        assert!(helper_arguments(&invalid).is_err());
    }

    #[test]
    fn helper_rejects_replayed_request_sequences() {
        let token = "a".repeat(64);
        assert!(validate_request(PROTOCOL, &token, 2, 1, &token).is_ok());
        assert!(validate_request(PROTOCOL, &token, 1, 1, &token).is_err());
        assert!(validate_request(PROTOCOL, &"b".repeat(64), 2, 1, &token).is_err());
    }

    #[test]
    fn helper_session_reuses_one_authenticated_channel_until_parent_disconnects() {
        let listener = TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
            .expect("the test listener must bind");
        let port = listener.local_addr().expect("address must exist").port();
        let token = "a".repeat(64);
        let helper_token = token.clone();
        let helper = std::thread::spawn(move || run_helper_session(port, &helper_token));
        let (stream, _) = listener.accept().expect("the helper must connect");
        let (mut writer, mut reader) = authenticate_helper(stream, &token)
            .expect("the helper must authenticate with the shared capability");

        for request_id in [1, 2] {
            if request_id == 2 {
                std::thread::sleep(Duration::from_millis(75));
            }
            write_message(
                &mut writer,
                &HelperRequest::Ping {
                    protocol: PROTOCOL.to_owned(),
                    token: token.clone(),
                    request_id,
                },
            )
            .expect("the reusable session must accept a ping");
            assert_eq!(
                read_message::<HelperEvent>(&mut reader).expect("the helper must answer the ping"),
                HelperEvent::Pong {
                    protocol: PROTOCOL.to_owned(),
                    request_id,
                }
            );
        }

        drop(reader);
        drop(writer);
        helper
            .join()
            .expect("the helper thread must not panic")
            .expect("the helper must exit cleanly after the parent disconnects");
    }

    #[test]
    fn authenticated_session_waits_for_a_delayed_response_after_nonblocking_accept() {
        let listener = TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
            .expect("the test listener must bind");
        listener
            .set_nonblocking(true)
            .expect("the test listener must become nonblocking");
        let port = listener.local_addr().expect("address must exist").port();
        let token = "a".repeat(64);
        let helper_token = token.clone();
        let helper = std::thread::spawn(move || {
            let stream = TcpStream::connect(SocketAddrV4::new(Ipv4Addr::LOCALHOST, port))
                .expect("the test helper must connect");
            let mut writer = stream
                .try_clone()
                .expect("the test helper channel must clone");
            write_message(
                &mut writer,
                &HelperEvent::Ready {
                    protocol: PROTOCOL.to_owned(),
                    token: helper_token,
                },
            )
            .expect("the test helper must authenticate");
            std::thread::sleep(Duration::from_millis(75));
            write_message(
                &mut writer,
                &HelperEvent::Pong {
                    protocol: PROTOCOL.to_owned(),
                    request_id: 1,
                },
            )
            .expect("the delayed response must remain writable");
        });
        let started = Instant::now();
        let stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error) if error.kind() == ErrorKind::WouldBlock => {
                    assert!(
                        started.elapsed() < Duration::from_secs(2),
                        "the test helper must connect before the timeout"
                    );
                    std::thread::sleep(Duration::from_millis(1));
                }
                Err(error) => panic!("accepting the test helper failed: {error}"),
            }
        };
        let (_, mut reader) = authenticate_helper(stream, &token)
            .expect("the helper must authenticate with the shared capability");

        assert_eq!(
            read_message::<HelperEvent>(&mut reader)
                .expect("the authenticated channel must wait for the delayed response"),
            HelperEvent::Pong {
                protocol: PROTOCOL.to_owned(),
                request_id: 1,
            }
        );
        helper.join().expect("the test helper must not panic");
    }

    #[test]
    fn capability_tokens_are_full_width_random_hex() {
        let first = secure_token().expect("the operating system RNG must be available");
        let second = secure_token().expect("the operating system RNG must remain available");
        assert_eq!(first.len(), 64);
        assert!(first.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert_ne!(first, second);
    }

    #[test]
    fn protocol_roundtrips_the_declared_camel_case_contract() {
        let request = HelperRequest::Execute {
            protocol: PROTOCOL.to_owned(),
            token: "a".repeat(64),
            request_id: 7,
            task_id: "windows.maintenance.time-sync".to_string(),
        };
        let serialized = serde_json::to_string(&request).expect("request must serialize");

        assert!(serialized.contains(r#""requestId":7"#));
        assert!(serialized.contains(r#""taskId":"windows.maintenance.time-sync""#));
        assert_eq!(
            serde_json::from_str::<HelperRequest>(&serialized)
                .expect("the declared request must deserialize"),
            request
        );
    }

    #[test]
    fn protocol_rejects_fields_outside_the_typed_contract() {
        let message = format!(
            r#"{{"type":"ping","protocol":"{PROTOCOL}","token":"{}","requestId":1,"command":"whoami"}}"#,
            "a".repeat(64)
        );
        assert!(serde_json::from_str::<HelperRequest>(&message).is_err());
    }

    #[test]
    fn remote_io_failures_remain_distinct_from_transport_failures() {
        let remote = SessionExecutionError::Remote(PlatformError::new(
            PlatformErrorCode::Io,
            "simulated remote task failure",
        ));

        assert!(matches!(remote, SessionExecutionError::Remote(_)));
    }
}
