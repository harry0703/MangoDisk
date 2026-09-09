use std::{
    collections::BTreeSet,
    panic::{catch_unwind, AssertUnwindSafe},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, OnceLock,
    },
    time::Instant,
};

use mangodisk_platform::{
    current_platform, PlatformCancellation, PlatformError, PlatformErrorCode,
    PlatformMutationState, PlatformSystemMaintenanceCompletion, PlatformSystemMaintenanceProgress,
    PlatformSystemMaintenanceState, PlatformSystemMaintenanceStatus,
    SystemMaintenancePlatform as _,
};

use crate::{
    filesystem::metadata::now_ms,
    shared::{
        operation::{CoordinatedOperationKind, OperationCancellationToken, OperationGuard},
        CoreError, CoreErrorCode, CoreResult,
    },
};

use super::{
    catalog::{definition, definitions, MaintenanceResource},
    CatalogSession, SystemMaintenanceCatalog, SystemMaintenanceCatalogSummary,
    SystemMaintenanceExecutionItemResult, SystemMaintenanceExecutionRequest,
    SystemMaintenanceExecutionStatus, SystemMaintenanceFailureReason, SystemMaintenanceItem,
    SystemMaintenanceJob, SystemMaintenanceJobStatus, SystemMaintenanceMutationState,
    SystemMaintenancePlatform, SystemMaintenanceProgress, SystemMaintenanceRuntimeState,
    SystemMaintenanceStatus, SYSTEM_MAINTENANCE_CATALOG_SCHEMA_VERSION,
};

const MAX_AUTHORIZATION_PROMPT_CHARS: usize = 240;
const MAX_CONCURRENT_EXECUTIONS: usize = 2;
const MAX_RETAINED_EXECUTIONS: usize = 64;

static CATALOG_SESSION: OnceLock<Mutex<Option<CatalogSession>>> = OnceLock::new();
static EXECUTION_REGISTRY: OnceLock<Mutex<ExecutionRegistry>> = OnceLock::new();

/// Receives durable job snapshots whenever one background execution changes state.
pub type SystemMaintenanceJobUpdateSink = Arc<dyn Fn(SystemMaintenanceJob) + Send + Sync>;

struct ExecutionEntry {
    public: SystemMaintenanceJob,
    authorization_prompt: String,
    resources: Vec<MaintenanceResource>,
    requires_elevation: bool,
    cancellation: Arc<AtomicBool>,
    sink: SystemMaintenanceJobUpdateSink,
}

#[derive(Default)]
struct ExecutionRegistry {
    next_sequence: u64,
    operation_id: Option<u64>,
    session_guard: Option<OperationGuard>,
    entries: Vec<ExecutionEntry>,
}

struct ExecutionDispatch {
    operation_id: u64,
    execution_id: String,
    task_id: String,
    authorization_prompt: String,
    cancellation: Arc<AtomicBool>,
}

struct ScheduledExecutions {
    updates: Vec<(SystemMaintenanceJobUpdateSink, SystemMaintenanceJob)>,
    dispatches: Vec<ExecutionDispatch>,
}

impl ScheduledExecutions {
    fn empty() -> Self {
        Self {
            updates: Vec::new(),
            dispatches: Vec::new(),
        }
    }
}

pub struct SystemMaintenanceService;

impl SystemMaintenanceService {
    pub fn cancel_scan() {
        OperationCancellationToken::system_maintenance_scan().cancel();
    }

    pub fn scan() -> CoreResult<SystemMaintenanceCatalog> {
        let operation = OperationGuard::start(CoordinatedOperationKind::SystemMaintenanceScan)?;
        let catalog = capture_catalog(&operation)?;
        replace_catalog_session(CatalogSession {
            public: catalog.clone(),
        })?;
        log::info!(
            "system_maintenance_catalog_scanned operation_id={} platform={:?} item_count={} recommended_count={} available_count={} unavailable_count={} elapsed_ms={}",
            operation.id(),
            catalog.platform,
            catalog.summary.item_count,
            catalog.summary.recommended_count,
            catalog.summary.available_count,
            catalog.summary.unavailable_count,
            catalog.elapsed_ms
        );
        operation.complete();
        Ok(catalog)
    }

    /// Enqueues one maintenance task and returns immediately with its current state.
    ///
    /// One parent operation guard remains alive for the full queue lifetime. This preserves the
    /// filesystem mutation boundary while the scheduler safely overlaps independent maintenance
    /// resources inside that boundary. Read-only scans and unrelated configuration domains remain
    /// available while the session is active.
    pub fn start_execution(
        request: SystemMaintenanceExecutionRequest,
        sink: SystemMaintenanceJobUpdateSink,
    ) -> CoreResult<SystemMaintenanceJob> {
        validate_execution_request(&request)?;
        let session = current_catalog_session()?;
        if session.public.scan_id != request.scan_id {
            return Err(CoreError::invalid_input(
                "system maintenance catalog session has expired",
            ));
        }
        let scanned_item = session
            .public
            .items
            .iter()
            .find(|item| item.task_id == request.task_id)
            .ok_or_else(|| CoreError::invalid_input("system maintenance task is unknown"))?;
        if matches!(
            scanned_item.status,
            SystemMaintenanceStatus::Healthy | SystemMaintenanceStatus::Unavailable
        ) {
            return Err(CoreError::invalid_input(
                "system maintenance task is not executable",
            ));
        }

        let definition = definition(&request.task_id)
            .ok_or_else(|| CoreError::invalid_input("system maintenance task is unknown"))?;
        let mut resources = definition.resources.to_vec();
        if scanned_item.requires_elevation {
            resources.push(MaintenanceResource::Elevation);
        }

        let (job, scheduled) = {
            let mut registry = execution_registry()?;
            if let Some(entry) = registry.entries.iter().rev().find(|entry| {
                entry.public.scan_id == request.scan_id && entry.public.task_id == request.task_id
            }) {
                if is_active(entry.public.status) {
                    return Err(CoreError::operation_busy(
                        "the system maintenance task is already queued or running",
                    ));
                }
                if !is_retryable(entry) {
                    return Err(CoreError::invalid_input(
                        "the system maintenance catalog must refresh before rerunning this task",
                    ));
                }
            }

            if registry.session_guard.is_none() {
                let operation =
                    OperationGuard::start(CoordinatedOperationKind::SystemMaintenanceExecution)?;
                registry.operation_id = Some(operation.id());
                registry.session_guard = Some(operation);
            }
            let operation_id = registry.operation_id.ok_or_else(|| {
                CoreError::operation_failed("system maintenance execution session is unavailable")
            })?;
            registry.next_sequence = registry.next_sequence.saturating_add(1);
            let execution_id = format!(
                "system-maintenance-{operation_id}-{}",
                registry.next_sequence
            );
            let job = SystemMaintenanceJob {
                execution_id: execution_id.clone(),
                scan_id: request.scan_id.clone(),
                task_id: request.task_id.clone(),
                revision: 1,
                status: SystemMaintenanceJobStatus::Queued,
                cancelable: true,
                queued_at_ms: now_ms(),
                started_at_ms: None,
                finished_at_ms: None,
                progress: None,
                result: None,
            };
            registry.entries.push(ExecutionEntry {
                public: job,
                authorization_prompt: request.authorization_prompt,
                resources,
                requires_elevation: scanned_item.requires_elevation,
                cancellation: Arc::new(AtomicBool::new(false)),
                sink,
            });
            prune_finished_executions(&mut registry);
            let queued_job = registry
                .entries
                .iter()
                .find(|entry| entry.public.execution_id == execution_id)
                .map(|entry| entry.public.clone())
                .ok_or_else(|| {
                    CoreError::operation_failed("system maintenance job registration failed")
                })?;
            // Record acceptance before scheduling can transition an immediately runnable job to
            // `Running`. Preserving this causal order makes production timelines unambiguous and
            // keeps queue depth attributable to the state observed at enqueue time.
            log::info!(
                "system_maintenance_job_enqueued operation_id={} execution_id={} task_id={} revision={} state={:?} requires_elevation={} queue_depth={}",
                operation_id,
                queued_job.execution_id,
                queued_job.task_id,
                queued_job.revision,
                queued_job.status,
                scanned_item.requires_elevation,
                registry
                    .entries
                    .iter()
                    .filter(|entry| entry.public.status == SystemMaintenanceJobStatus::Queued)
                    .count()
            );
            let scheduled = schedule_executions(&mut registry)?;
            let job = registry
                .entries
                .iter()
                .find(|entry| entry.public.execution_id == execution_id)
                .map(|entry| entry.public.clone())
                .ok_or_else(|| {
                    CoreError::operation_failed("system maintenance job registration failed")
                })?;
            (job, scheduled)
        };
        publish_and_dispatch(scheduled);
        Ok(job)
    }

    pub fn cancel_execution(execution_id: &str) -> CoreResult<SystemMaintenanceJob> {
        if execution_id.trim().is_empty() {
            return Err(CoreError::invalid_input(
                "system maintenance execution identifier is required",
            ));
        }

        let (job, scheduled, update, released_operation) = {
            let mut registry = execution_registry()?;
            let operation_id = registry.operation_id;
            let index = registry
                .entries
                .iter()
                .position(|entry| entry.public.execution_id == execution_id)
                .ok_or_else(|| CoreError::invalid_input("system maintenance job is unknown"))?;

            match registry.entries[index].public.status {
                SystemMaintenanceJobStatus::Queued => {
                    let task_id = registry.entries[index].public.task_id.clone();
                    registry.entries[index].public.status = SystemMaintenanceJobStatus::Finished;
                    registry.entries[index].public.cancelable = false;
                    registry.entries[index].public.finished_at_ms = Some(now_ms());
                    registry.entries[index].public.result = Some(cancelled_item(task_id));
                    advance_job_revision(&mut registry.entries[index].public);
                }
                SystemMaintenanceJobStatus::Running
                    if registry.entries[index].public.cancelable =>
                {
                    registry.entries[index]
                        .cancellation
                        .store(true, Ordering::Relaxed);
                    registry.entries[index].public.status = SystemMaintenanceJobStatus::Cancelling;
                    registry.entries[index].public.cancelable = false;
                    advance_job_revision(&mut registry.entries[index].public);
                }
                SystemMaintenanceJobStatus::Running | SystemMaintenanceJobStatus::Cancelling => {
                    return Err(CoreError::invalid_input(
                        "the running maintenance task cannot be cancelled safely",
                    ));
                }
                SystemMaintenanceJobStatus::Finished => {
                    return Ok(registry.entries[index].public.clone());
                }
            }

            let job = registry.entries[index].public.clone();
            let update = (Arc::clone(&registry.entries[index].sink), job.clone());
            prune_finished_executions(&mut registry);
            let scheduled = schedule_executions(&mut registry)?;
            let released_operation = release_session_if_idle(&mut registry);
            log::info!(
                "system_maintenance_job_cancel_requested operation_id={} execution_id={} task_id={} revision={} state={:?}",
                operation_id
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "none".to_string()),
                job.execution_id,
                job.task_id,
                job.revision,
                job.status
            );
            (job, scheduled, update, released_operation)
        };
        if let Some(operation_id) = released_operation {
            log::info!(
                "system_maintenance_execution_session_finished operation_id={operation_id} reason=queue_empty"
            );
        }
        publish_job_update(update.0, update.1);
        publish_and_dispatch(scheduled);
        Ok(job)
    }

    pub fn runtime_state() -> CoreResult<SystemMaintenanceRuntimeState> {
        let catalog = optional_catalog_session()?.map(|session| session.public);
        let executions = execution_registry()?
            .entries
            .iter()
            .map(|entry| entry.public.clone())
            .collect();
        Ok(SystemMaintenanceRuntimeState {
            catalog,
            executions,
        })
    }
}

fn schedule_executions(registry: &mut ExecutionRegistry) -> CoreResult<ScheduledExecutions> {
    let operation_id = registry.operation_id.ok_or_else(|| {
        CoreError::operation_failed("system maintenance execution session is unavailable")
    })?;
    let mut scheduled = ScheduledExecutions::empty();
    while running_count(registry) < MAX_CONCURRENT_EXECUTIONS {
        let running_resources = registry
            .entries
            .iter()
            .filter(|entry| {
                matches!(
                    entry.public.status,
                    SystemMaintenanceJobStatus::Running | SystemMaintenanceJobStatus::Cancelling
                )
            })
            .flat_map(|entry| entry.resources.iter().copied())
            .collect::<Vec<_>>();
        let Some(index) = registry.entries.iter().position(|entry| {
            entry.public.status == SystemMaintenanceJobStatus::Queued
                && !entry
                    .resources
                    .iter()
                    .any(|resource| running_resources.contains(resource))
        }) else {
            break;
        };

        let entry = &mut registry.entries[index];
        entry.public.status = SystemMaintenanceJobStatus::Running;
        // Once an elevated native command starts, forcibly stopping it could leave a partially
        // repaired system. Queued elevated jobs remain cancelable until this transition.
        entry.public.cancelable = !entry.requires_elevation;
        entry.public.started_at_ms = Some(now_ms());
        advance_job_revision(&mut entry.public);
        log::info!(
            "system_maintenance_job_started operation_id={} execution_id={} task_id={} revision={} queue_wait_ms={} cancelable={}",
            operation_id,
            entry.public.execution_id,
            entry.public.task_id,
            entry.public.revision,
            entry
                .public
                .started_at_ms
                .unwrap_or(entry.public.queued_at_ms)
                .saturating_sub(entry.public.queued_at_ms),
            entry.public.cancelable
        );
        scheduled
            .updates
            .push((Arc::clone(&entry.sink), entry.public.clone()));
        scheduled.dispatches.push(ExecutionDispatch {
            operation_id,
            execution_id: entry.public.execution_id.clone(),
            task_id: entry.public.task_id.clone(),
            authorization_prompt: entry.authorization_prompt.clone(),
            cancellation: Arc::clone(&entry.cancellation),
        });
    }
    Ok(scheduled)
}

fn publish_and_dispatch(scheduled: ScheduledExecutions) {
    for (sink, update) in scheduled.updates {
        publish_job_update(sink, update);
    }
    for dispatch in scheduled.dispatches {
        let execution_id = dispatch.execution_id.clone();
        let task_id = dispatch.task_id.clone();
        let operation_id = dispatch.operation_id;
        if let Err(error) = std::thread::Builder::new()
            .name(format!("maintenance-{execution_id}"))
            .spawn(move || run_execution(dispatch))
        {
            log::error!(
                "system_maintenance_worker_start_failed operation_id={} execution_id={} task_id={} error_kind={:?} os_error_code={}",
                operation_id,
                execution_id,
                task_id,
                error.kind(),
                error
                    .raw_os_error()
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "none".to_string())
            );
            finish_execution(
                &execution_id,
                failed_item(
                    operation_id,
                    &execution_id,
                    task_id,
                    PlatformErrorCode::OperationFailed,
                    SystemMaintenanceMutationState::NotChanged,
                    None,
                    0,
                ),
            );
        }
    }
}

fn run_execution(dispatch: ExecutionDispatch) {
    let operation_id = dispatch.operation_id;
    let execution_id = dispatch.execution_id.clone();
    let task_id = dispatch.task_id.clone();
    let started = Instant::now();
    let result =
        contain_execution_panic(operation_id, &execution_id, &task_id, started, move || {
            execute_task(
                dispatch.operation_id,
                &dispatch.execution_id,
                dispatch.task_id,
                dispatch.authorization_prompt,
                dispatch.cancellation,
            )
        });
    finish_execution(&execution_id, result);
}

/// Contains unexpected adapter panics so one native fault cannot strand the process-wide
/// operation guard or leave every queued maintenance task blocked indefinitely. A panic may occur
/// after a native tool has already changed the system, so the public result deliberately preserves
/// that uncertainty instead of claiming that nothing happened.
fn contain_execution_panic<F>(
    operation_id: u64,
    execution_id: &str,
    task_id: &str,
    started: Instant,
    execute: F,
) -> SystemMaintenanceExecutionItemResult
where
    F: FnOnce() -> SystemMaintenanceExecutionItemResult,
{
    match catch_unwind(AssertUnwindSafe(execute)) {
        Ok(result) => result,
        Err(_) => {
            log::error!(
                "system_maintenance_worker_panicked operation_id={operation_id} execution_id={execution_id} task_id={task_id} elapsed_ms={}",
                started.elapsed().as_millis()
            );
            failed_item(
                operation_id,
                execution_id,
                task_id.to_string(),
                PlatformErrorCode::OperationFailed,
                SystemMaintenanceMutationState::MayHaveChanged,
                None,
                started.elapsed().as_millis(),
            )
        }
    }
}

fn finish_execution(execution_id: &str, result: SystemMaintenanceExecutionItemResult) {
    let outcome = (|| -> CoreResult<(
        (SystemMaintenanceJobUpdateSink, SystemMaintenanceJob),
        ScheduledExecutions,
        Option<u64>,
    )> {
        let mut registry = execution_registry()?;
        let operation_id = registry.operation_id;
        let index = registry
            .entries
            .iter()
            .position(|entry| entry.public.execution_id == execution_id)
            .ok_or_else(|| CoreError::operation_failed("system maintenance job disappeared"))?;
        let entry = &mut registry.entries[index];
        entry.public.status = SystemMaintenanceJobStatus::Finished;
        entry.public.cancelable = false;
        entry.public.finished_at_ms = Some(now_ms());
        entry.public.progress = None;
        entry.public.result = Some(result.clone());
        advance_job_revision(&mut entry.public);
        let terminal = (Arc::clone(&entry.sink), entry.public.clone());
        prune_finished_executions(&mut registry);
        let scheduled = schedule_executions(&mut registry)?;
        let released_operation = release_session_if_idle(&mut registry);
        log::info!(
            "system_maintenance_job_finished operation_id={} execution_id={} task_id={} revision={} status={:?} failure_reason={:?} mutation_state={:?} restart_required={}",
            operation_id
                .map(|value| value.to_string())
                .unwrap_or_else(|| "none".to_string()),
            execution_id,
            result.task_id,
            terminal.1.revision,
            result.status,
            result.failure_reason,
            result.mutation_state,
            result.requires_restart
        );
        Ok((terminal, scheduled, released_operation))
    })();

    match outcome {
        Ok((terminal, scheduled, released_operation)) => {
            if let Some(operation_id) = released_operation {
                log::info!(
                    "system_maintenance_execution_session_finished operation_id={operation_id} reason=queue_empty"
                );
            }
            publish_job_update(terminal.0, terminal.1);
            publish_and_dispatch(scheduled);
        }
        Err(error) => {
            log::error!(
                "system_maintenance_job_finalize_failed execution_id={} code={:?}",
                execution_id,
                error.code()
            );
        }
    }
}

fn execute_task(
    operation_id: u64,
    execution_id: &str,
    task_id: String,
    authorization_prompt: String,
    cancellation_flag: Arc<AtomicBool>,
) -> SystemMaintenanceExecutionItemResult {
    let started = Instant::now();
    let cancellation = PlatformCancellation::new(move || cancellation_flag.load(Ordering::Relaxed));
    log::info!(
        "system_maintenance_task_started operation_id={operation_id} execution_id={execution_id} task_id={task_id}"
    );
    let progress_execution_id = execution_id.to_string();
    let progress_task_id = task_id.clone();
    let progress_sink = move |progress| {
        if let Err(error) = update_execution_progress(&progress_execution_id, progress) {
            log::warn!(
                "system_maintenance_progress_update_failed operation_id={} execution_id={} task_id={} code={:?} phase={:?} current_step={} total_steps={} percent={}",
                operation_id,
                progress_execution_id,
                progress_task_id,
                error.code(),
                progress.phase,
                optional_u8_log_value(progress.current_step),
                optional_u8_log_value(progress.total_steps),
                optional_u8_log_value(progress.percent)
            );
        }
    };
    match current_platform().execute_system_maintenance(
        &task_id,
        &cancellation,
        Some(&authorization_prompt),
        &progress_sink,
    ) {
        Ok(outcome) if outcome.task_id == task_id => {
            let completion_status = match outcome.completion {
                PlatformSystemMaintenanceCompletion::Completed => {
                    SystemMaintenanceExecutionStatus::Completed
                }
                PlatformSystemMaintenanceCompletion::Started => {
                    SystemMaintenanceExecutionStatus::Started
                }
            };
            let failure_reason =
                (!outcome.verified).then_some(SystemMaintenanceFailureReason::VerificationFailed);
            let status = if failure_reason.is_some() {
                SystemMaintenanceExecutionStatus::Failed
            } else {
                completion_status
            };
            let mutation_state = if outcome.changed {
                SystemMaintenanceMutationState::Changed
            } else {
                SystemMaintenanceMutationState::NotChanged
            };
            if failure_reason.is_some() {
                log::warn!(
                    "system_maintenance_task_failed operation_id={} execution_id={} task_id={} code=VerificationFailed mutation_state={:?} restart_required={} elapsed_ms={}",
                    operation_id,
                    execution_id,
                    task_id,
                    mutation_state,
                    outcome.requires_restart,
                    started.elapsed().as_millis()
                );
            } else {
                log::info!(
                    "system_maintenance_task_native_finished operation_id={} execution_id={} task_id={} status={:?} mutation_state={:?} verified={} restart_required={} elapsed_ms={}",
                    operation_id,
                    execution_id,
                    task_id,
                    status,
                    mutation_state,
                    outcome.verified,
                    outcome.requires_restart,
                    started.elapsed().as_millis()
                );
            }
            SystemMaintenanceExecutionItemResult {
                task_id,
                status,
                mutation_state,
                verified: outcome.verified,
                requires_restart: outcome.requires_restart,
                failure_reason,
            }
        }
        Ok(_) => failed_item(
            operation_id,
            execution_id,
            task_id,
            PlatformErrorCode::InvalidData,
            SystemMaintenanceMutationState::MayHaveChanged,
            None,
            started.elapsed().as_millis(),
        ),
        Err(error) => failed_platform_item(
            operation_id,
            execution_id,
            task_id,
            &error,
            started.elapsed().as_millis(),
        ),
    }
}

fn update_execution_progress(
    execution_id: &str,
    platform_progress: PlatformSystemMaintenanceProgress,
) -> CoreResult<()> {
    if platform_progress
        .percent
        .is_some_and(|percent| percent > 100)
        || matches!(
            (platform_progress.current_step, platform_progress.total_steps),
            (Some(current), Some(total)) if current == 0 || total == 0 || current > total
        )
        || platform_progress.current_step.is_some() != platform_progress.total_steps.is_some()
    {
        return Err(CoreError::new(
            CoreErrorCode::Platform,
            "system maintenance adapter returned invalid progress",
        ));
    }

    let progress = SystemMaintenanceProgress {
        phase: platform_progress.phase,
        current_step: platform_progress.current_step,
        total_steps: platform_progress.total_steps,
        percent: platform_progress.percent,
    };
    let (operation_id, sink, job, phase_changed, percent_milestone) = {
        let mut registry = execution_registry()?;
        let operation_id = registry.operation_id;
        let entry = registry
            .entries
            .iter_mut()
            .find(|entry| entry.public.execution_id == execution_id)
            .ok_or_else(|| CoreError::operation_failed("system maintenance job disappeared"))?;
        if !matches!(
            entry.public.status,
            SystemMaintenanceJobStatus::Running | SystemMaintenanceJobStatus::Cancelling
        ) || entry.public.progress.as_ref() == Some(&progress)
        {
            return Ok(());
        }
        let previous = entry.public.progress;
        let phase_changed = previous.is_none_or(|value| value.phase != progress.phase);
        let percent_milestone = progress.percent.is_some_and(|percent| {
            previous.and_then(|value| value.percent).unwrap_or(0) / 10 != percent / 10
        });
        entry.public.progress = Some(progress);
        advance_job_revision(&mut entry.public);
        (
            operation_id,
            Arc::clone(&entry.sink),
            entry.public.clone(),
            phase_changed,
            percent_milestone,
        )
    };

    if phase_changed || percent_milestone {
        log::info!(
            "system_maintenance_task_progress operation_id={} execution_id={} task_id={} revision={} phase={:?} current_step={} total_steps={} percent={}",
            operation_id
                .map(|value| value.to_string())
                .unwrap_or_else(|| "none".to_string()),
            execution_id,
            job.task_id,
            job.revision,
            progress.phase,
            optional_u8_log_value(progress.current_step),
            optional_u8_log_value(progress.total_steps),
            optional_u8_log_value(progress.percent)
        );
    }
    publish_job_update(sink, job);
    Ok(())
}

/// Isolates desktop event consumers from the scheduler lifecycle. Event delivery is best-effort:
/// runtime snapshots remain authoritative, and a faulty consumer must never prevent native work
/// from starting, finishing, or releasing its resources.
fn publish_job_update(sink: SystemMaintenanceJobUpdateSink, job: SystemMaintenanceJob) {
    let execution_id = job.execution_id.clone();
    let task_id = job.task_id.clone();
    let revision = job.revision;
    let status = job.status;
    if catch_unwind(AssertUnwindSafe(|| sink(job))).is_err() {
        log::error!(
            "system_maintenance_job_update_sink_panicked execution_id={execution_id} task_id={task_id} revision={revision} state={status:?}"
        );
    }
}

fn running_count(registry: &ExecutionRegistry) -> usize {
    registry
        .entries
        .iter()
        .filter(|entry| {
            matches!(
                entry.public.status,
                SystemMaintenanceJobStatus::Running | SystemMaintenanceJobStatus::Cancelling
            )
        })
        .count()
}

fn advance_job_revision(job: &mut SystemMaintenanceJob) {
    job.revision = job.revision.saturating_add(1);
}

fn optional_u8_log_value(value: Option<u8>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "none".to_string())
}

fn release_session_if_idle(registry: &mut ExecutionRegistry) -> Option<u64> {
    if registry
        .entries
        .iter()
        .any(|entry| is_active(entry.public.status))
    {
        return None;
    }
    let operation_id = registry.operation_id.take();
    if let Some(operation) = registry.session_guard.take() {
        // Release while holding the registry mutex so a new enqueue cannot observe an empty
        // registry before the process-wide lock has actually been dropped.
        operation.complete();
        drop(operation);
    }
    operation_id
}

fn is_active(status: SystemMaintenanceJobStatus) -> bool {
    status != SystemMaintenanceJobStatus::Finished
}

/// A failed command that provably did not change system state can safely be retried against the
/// same catalog. Successful or uncertain executions must wait for a new scan so stale diagnostics
/// cannot authorize an unintended second mutation.
fn is_retryable(entry: &ExecutionEntry) -> bool {
    entry.public.status == SystemMaintenanceJobStatus::Finished
        && entry.public.result.as_ref().is_some_and(|result| {
            result.status == SystemMaintenanceExecutionStatus::Failed
                && result.mutation_state == SystemMaintenanceMutationState::NotChanged
        })
}

fn prune_finished_executions(registry: &mut ExecutionRegistry) {
    let finished_count = registry
        .entries
        .iter()
        .filter(|entry| entry.public.status == SystemMaintenanceJobStatus::Finished)
        .count();
    let remove_count = finished_count.saturating_sub(MAX_RETAINED_EXECUTIONS);
    if remove_count == 0 {
        return;
    }
    let mut remaining = remove_count;
    registry.entries.retain(|entry| {
        if remaining > 0 && entry.public.status == SystemMaintenanceJobStatus::Finished {
            remaining -= 1;
            false
        } else {
            true
        }
    });
}

fn cancelled_item(task_id: String) -> SystemMaintenanceExecutionItemResult {
    SystemMaintenanceExecutionItemResult {
        task_id,
        status: SystemMaintenanceExecutionStatus::Failed,
        mutation_state: SystemMaintenanceMutationState::NotChanged,
        verified: false,
        requires_restart: false,
        failure_reason: Some(SystemMaintenanceFailureReason::UserCancelled),
    }
}

fn capture_catalog(operation: &OperationGuard) -> CoreResult<SystemMaintenanceCatalog> {
    let started = Instant::now();
    let definitions = definitions();
    let task_ids = definitions
        .iter()
        .map(|definition| definition.id)
        .collect::<Vec<_>>();
    let cancellation_flag = operation.cancellation_flag();
    let cancellation = PlatformCancellation::new(move || cancellation_flag.load(Ordering::Relaxed));
    let platform_states = current_platform()
        .scan_system_maintenance(&task_ids, &cancellation)
        .map_err(CoreError::from)?;
    operation.ensure_not_cancelled()?;

    assemble_catalog(operation.id(), started, platform_states)
}

/// Converts the platform inventory into the stable Core catalog contract.
///
/// Keeping adapter validation separate from native I/O makes malformed platform responses
/// deterministic to test. More importantly, every GUI and CLI adapter receives the same
/// fail-closed behavior when a platform implementation returns duplicate, unknown, or missing
/// tasks after an operating-system update.
fn assemble_catalog(
    operation_id: u64,
    started: Instant,
    platform_states: Vec<PlatformSystemMaintenanceState>,
) -> CoreResult<SystemMaintenanceCatalog> {
    let definitions = definitions();
    let mut seen = BTreeSet::new();
    let mut items = Vec::with_capacity(definitions.len());
    for state in platform_states {
        if !seen.insert(state.task_id.clone()) {
            return Err(CoreError::new(
                CoreErrorCode::Platform,
                "system maintenance adapter returned a duplicate task",
            ));
        }
        let definition = definition(&state.task_id).ok_or_else(|| {
            CoreError::new(
                CoreErrorCode::Platform,
                "system maintenance adapter returned an unknown task",
            )
        })?;
        let status = map_status(state.status);
        items.push(SystemMaintenanceItem {
            task_id: state.task_id,
            category: definition.category,
            risk_level: definition.risk_level,
            status,
            requires_elevation: state.requires_elevation,
            requires_restart: definition.requires_restart,
            estimated_duration_seconds: definition.estimated_duration_seconds,
            diagnostic: state.diagnostic,
        });
        if status == SystemMaintenanceStatus::Unavailable {
            log::info!(
                "system_maintenance_task_unavailable operation_id={} task_id={} diagnostic={:?}",
                operation_id,
                items
                    .last()
                    .map(|item| item.task_id.as_str())
                    .unwrap_or("unknown"),
                items.last().and_then(|item| item.diagnostic)
            );
        }
    }
    if items.len() != definitions.len() {
        return Err(CoreError::new(
            CoreErrorCode::Platform,
            "system maintenance adapter returned an incomplete catalog",
        ));
    }
    items.sort_by_key(|item| {
        definitions
            .iter()
            .position(|definition| definition.id == item.task_id)
            .unwrap_or(usize::MAX)
    });
    let summary = summarize(&items);
    let scanned_at_ms = now_ms();
    Ok(SystemMaintenanceCatalog {
        schema_version: SYSTEM_MAINTENANCE_CATALOG_SCHEMA_VERSION,
        scan_id: format!("system-maintenance-{operation_id}-{scanned_at_ms}"),
        platform: current_platform_name(),
        scanned_at_ms,
        elapsed_ms: started.elapsed().as_millis() as u64,
        items,
        summary,
    })
}

fn validate_execution_request(request: &SystemMaintenanceExecutionRequest) -> CoreResult<()> {
    let authorization_prompt = request.authorization_prompt.trim();
    if request.scan_id.is_empty()
        || definition(&request.task_id).is_none()
        || authorization_prompt.is_empty()
        || authorization_prompt.chars().count() > MAX_AUTHORIZATION_PROMPT_CHARS
        || authorization_prompt.contains(['\r', '\n'])
    {
        return Err(CoreError::invalid_input(
            "system maintenance execution request is invalid",
        ));
    }
    Ok(())
}

fn summarize(items: &[SystemMaintenanceItem]) -> SystemMaintenanceCatalogSummary {
    let count = |status| items.iter().filter(|item| item.status == status).count() as u64;
    SystemMaintenanceCatalogSummary {
        item_count: items.len() as u64,
        recommended_count: count(SystemMaintenanceStatus::Recommended),
        available_count: count(SystemMaintenanceStatus::Available),
        healthy_count: count(SystemMaintenanceStatus::Healthy),
        unavailable_count: count(SystemMaintenanceStatus::Unavailable),
    }
}

fn failed_item(
    operation_id: u64,
    execution_id: &str,
    task_id: String,
    code: PlatformErrorCode,
    mutation_state: SystemMaintenanceMutationState,
    error_digest: Option<&str>,
    elapsed_ms: u128,
) -> SystemMaintenanceExecutionItemResult {
    let failure_reason = match code {
        PlatformErrorCode::AccessDenied => SystemMaintenanceFailureReason::PermissionDenied,
        PlatformErrorCode::Unsupported => SystemMaintenanceFailureReason::Unsupported,
        PlatformErrorCode::UserCancelled => SystemMaintenanceFailureReason::UserCancelled,
        PlatformErrorCode::ItemChanged
        | PlatformErrorCode::InvalidData
        | PlatformErrorCode::InvalidPath
        | PlatformErrorCode::Io
        | PlatformErrorCode::OperationFailed => SystemMaintenanceFailureReason::PlatformFailure,
    };
    if failure_reason == SystemMaintenanceFailureReason::UserCancelled {
        log::info!(
            "system_maintenance_task_cancelled operation_id={operation_id} execution_id={execution_id} task_id={task_id} mutation_state={mutation_state:?} elapsed_ms={elapsed_ms}"
        );
    } else {
        log::warn!(
            "system_maintenance_task_failed operation_id={} execution_id={} task_id={} code={:?} mutation_state={:?} error_digest={} elapsed_ms={}",
            operation_id,
            execution_id,
            task_id,
            code,
            mutation_state,
            error_digest.unwrap_or("none"),
            elapsed_ms
        );
    }
    SystemMaintenanceExecutionItemResult {
        task_id,
        status: SystemMaintenanceExecutionStatus::Failed,
        mutation_state,
        verified: false,
        requires_restart: false,
        failure_reason: Some(failure_reason),
    }
}

fn failed_platform_item(
    operation_id: u64,
    execution_id: &str,
    task_id: String,
    error: &PlatformError,
    elapsed_ms: u128,
) -> SystemMaintenanceExecutionItemResult {
    // The digest lets support correlate repeated platform failures without persisting command
    // output, account names, paths, or other private details carried by the original error.
    let digest = blake3::hash(error.as_bytes()).to_hex().to_string();
    let mutation_state = match error.mutation_state() {
        PlatformMutationState::NotAttempted => SystemMaintenanceMutationState::NotChanged,
        PlatformMutationState::MayHaveChanged => SystemMaintenanceMutationState::MayHaveChanged,
    };
    let mut result = failed_item(
        operation_id,
        execution_id,
        task_id,
        error.code(),
        mutation_state,
        Some(&digest),
        elapsed_ms,
    );
    if let Some(reason) = error.failure_reason() {
        use mangodisk_platform::PlatformFailureReason as Reason;
        result.failure_reason = Some(match reason {
            Reason::ToolUnavailable => SystemMaintenanceFailureReason::ToolUnavailable,
            Reason::ServiceDisabled => SystemMaintenanceFailureReason::ServiceDisabled,
            Reason::ServiceUnavailable => SystemMaintenanceFailureReason::ServiceUnavailable,
            Reason::DependencyUnavailable => SystemMaintenanceFailureReason::DependencyUnavailable,
            Reason::ServiceBusy => SystemMaintenanceFailureReason::ServiceBusy,
            Reason::TimedOut => SystemMaintenanceFailureReason::TimedOut,
            Reason::VerificationFailed => SystemMaintenanceFailureReason::VerificationFailed,
            Reason::VerificationPermissionDenied => {
                SystemMaintenanceFailureReason::VerificationPermissionDenied
            }
        });
    }
    result
}

fn map_status(status: PlatformSystemMaintenanceStatus) -> SystemMaintenanceStatus {
    match status {
        PlatformSystemMaintenanceStatus::Healthy => SystemMaintenanceStatus::Healthy,
        PlatformSystemMaintenanceStatus::Recommended => SystemMaintenanceStatus::Recommended,
        PlatformSystemMaintenanceStatus::Available => SystemMaintenanceStatus::Available,
        PlatformSystemMaintenanceStatus::Unavailable => SystemMaintenanceStatus::Unavailable,
    }
}

#[cfg(target_os = "macos")]
const fn current_platform_name() -> SystemMaintenancePlatform {
    SystemMaintenancePlatform::Macos
}

#[cfg(windows)]
const fn current_platform_name() -> SystemMaintenancePlatform {
    SystemMaintenancePlatform::Windows
}

fn catalog_session() -> &'static Mutex<Option<CatalogSession>> {
    CATALOG_SESSION.get_or_init(|| Mutex::new(None))
}

fn replace_catalog_session(session: CatalogSession) -> CoreResult<()> {
    *catalog_session()
        .lock()
        .map_err(|_| CoreError::operation_failed("system maintenance catalog is unavailable"))? =
        Some(session);
    Ok(())
}

fn optional_catalog_session() -> CoreResult<Option<CatalogSession>> {
    catalog_session()
        .lock()
        .map_err(|_| CoreError::operation_failed("system maintenance catalog is unavailable"))
        .map(|session| session.clone())
}

fn current_catalog_session() -> CoreResult<CatalogSession> {
    optional_catalog_session()?
        .ok_or_else(|| CoreError::invalid_input("system maintenance catalog has not been scanned"))
}

fn execution_registry() -> CoreResult<std::sync::MutexGuard<'static, ExecutionRegistry>> {
    EXECUTION_REGISTRY
        .get_or_init(|| Mutex::new(ExecutionRegistry::default()))
        .lock()
        .map_err(|_| {
            CoreError::operation_failed("system maintenance execution registry is unavailable")
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn platform_states() -> Vec<PlatformSystemMaintenanceState> {
        let statuses = [
            PlatformSystemMaintenanceStatus::Healthy,
            PlatformSystemMaintenanceStatus::Recommended,
            PlatformSystemMaintenanceStatus::Available,
            PlatformSystemMaintenanceStatus::Unavailable,
        ];
        definitions()
            .iter()
            .enumerate()
            .map(|(index, definition)| PlatformSystemMaintenanceState {
                task_id: definition.id.to_string(),
                status: statuses[index % statuses.len()],
                requires_elevation: index % 2 == 0,
                diagnostic: (index % statuses.len() == 3).then_some(
                    mangodisk_platform::PlatformSystemMaintenanceDiagnosticCode::ToolUnavailable,
                ),
            })
            .collect()
    }

    fn reset_global_service_state() {
        if let Ok(mut registry) = execution_registry() {
            *registry = ExecutionRegistry::default();
        }
        if let Ok(mut session) = catalog_session().lock() {
            *session = None;
        }
    }

    /// Ensures process-wide test state is released even when an assertion fails. The production
    /// service intentionally retains terminal jobs, so leaking a fixture would make later tests
    /// order-dependent and could also keep the global operation coordinator occupied.
    struct GlobalServiceStateReset;

    impl Drop for GlobalServiceStateReset {
        fn drop(&mut self) {
            reset_global_service_state();
        }
    }

    fn scheduler_entry(
        execution_id: &str,
        status: SystemMaintenanceJobStatus,
        resources: Vec<MaintenanceResource>,
        requires_elevation: bool,
    ) -> ExecutionEntry {
        ExecutionEntry {
            public: SystemMaintenanceJob {
                execution_id: execution_id.to_string(),
                scan_id: "scan-test".to_string(),
                task_id: execution_id.to_string(),
                revision: 1,
                status,
                cancelable: true,
                queued_at_ms: 0,
                started_at_ms: None,
                finished_at_ms: None,
                progress: None,
                result: None,
            },
            authorization_prompt: "Authorize maintenance".to_string(),
            resources,
            requires_elevation,
            cancellation: Arc::new(AtomicBool::new(false)),
            sink: Arc::new(|_| {}),
        }
    }

    #[test]
    fn specific_native_causes_survive_partial_changes_and_verification() {
        for (native, expected) in [
            (
                mangodisk_platform::PlatformFailureReason::ServiceDisabled,
                SystemMaintenanceFailureReason::ServiceDisabled,
            ),
            (
                mangodisk_platform::PlatformFailureReason::VerificationPermissionDenied,
                SystemMaintenanceFailureReason::VerificationPermissionDenied,
            ),
        ] {
            let error = PlatformError::operation_failed("fixture")
                .with_failure_reason(native)
                .with_possible_side_effects();
            let result = failed_platform_item(
                1,
                "fixture",
                "windows.maintenance.update-components".to_owned(),
                &error,
                1,
            );
            assert_eq!(result.failure_reason, Some(expected));
            assert_eq!(
                result.mutation_state,
                SystemMaintenanceMutationState::MayHaveChanged
            );
            assert_eq!(result.status, SystemMaintenanceExecutionStatus::Failed);
            let wire = serde_json::to_value(&result).unwrap();
            assert_eq!(
                wire["failureReason"],
                serde_json::to_value(expected).unwrap()
            );
        }
    }

    #[test]
    fn execution_rejects_unknown_tasks() {
        assert!(
            validate_execution_request(&SystemMaintenanceExecutionRequest {
                scan_id: "scan".to_string(),
                task_id: "unknown".to_string(),
                authorization_prompt: "Authorize maintenance".to_string(),
            })
            .is_err()
        );
    }

    #[test]
    fn execution_rejects_unsafe_authorization_prompts() {
        let task_id = definitions()[0].id.to_string();
        for authorization_prompt in ["", "   ", "Authorize\nmaintenance"] {
            assert!(
                validate_execution_request(&SystemMaintenanceExecutionRequest {
                    scan_id: "scan".to_string(),
                    task_id: task_id.clone(),
                    authorization_prompt: authorization_prompt.to_string(),
                })
                .is_err()
            );
        }
    }

    #[test]
    fn catalog_contract_maps_states_and_restores_definition_order() {
        let mut states = platform_states();
        states.reverse();

        let catalog = assemble_catalog(41, Instant::now(), states)
            .expect("a complete platform catalog must be accepted");

        assert_eq!(
            catalog.schema_version,
            SYSTEM_MAINTENANCE_CATALOG_SCHEMA_VERSION
        );
        assert!(catalog.scan_id.starts_with("system-maintenance-41-"));
        assert_eq!(catalog.summary, summarize(&catalog.items));
        assert_eq!(catalog.items.len(), definitions().len());
        for (index, item) in catalog.items.iter().enumerate() {
            assert_eq!(item.task_id, definitions()[index].id);
            assert_eq!(item.requires_elevation, index % 2 == 0);
        }
    }

    #[test]
    fn catalog_contract_rejects_duplicate_unknown_and_incomplete_states() {
        let states = platform_states();

        let mut duplicate = states.clone();
        duplicate.push(states[0].clone());
        assert_eq!(
            assemble_catalog(42, Instant::now(), duplicate)
                .expect_err("duplicate task states must fail closed")
                .code(),
            CoreErrorCode::Platform
        );

        let mut unknown = states.clone();
        unknown[0].task_id = "maintenance.unknown".to_string();
        assert_eq!(
            assemble_catalog(42, Instant::now(), unknown)
                .expect_err("unknown task states must fail closed")
                .code(),
            CoreErrorCode::Platform
        );

        let mut incomplete = states;
        incomplete.pop();
        assert_eq!(
            assemble_catalog(42, Instant::now(), incomplete)
                .expect_err("incomplete platform catalogs must fail closed")
                .code(),
            CoreErrorCode::Platform
        );
    }

    #[test]
    fn public_queue_state_can_be_restored_cancelled_and_retried_without_native_work() {
        let _operation_lock = crate::shared::operation::test_operation_lock();
        reset_global_service_state();
        let _reset = GlobalServiceStateReset;
        let definition = definitions()[0];
        let scan_id = "scan-public-queue".to_string();
        let item = SystemMaintenanceItem {
            task_id: definition.id.to_string(),
            category: definition.category,
            risk_level: definition.risk_level,
            status: SystemMaintenanceStatus::Recommended,
            requires_elevation: false,
            requires_restart: definition.requires_restart,
            estimated_duration_seconds: definition.estimated_duration_seconds,
            diagnostic: None,
        };
        replace_catalog_session(CatalogSession {
            public: SystemMaintenanceCatalog {
                schema_version: SYSTEM_MAINTENANCE_CATALOG_SCHEMA_VERSION,
                scan_id: scan_id.clone(),
                platform: current_platform_name(),
                scanned_at_ms: now_ms(),
                elapsed_ms: 1,
                summary: summarize(std::slice::from_ref(&item)),
                items: vec![item],
            },
        })
        .expect("catalog fixture must install");

        // A synthetic conflicting job keeps the public request queued. This exercises the real
        // service lifecycle without allowing a worker thread to invoke a native maintenance tool.
        let operation = OperationGuard::start(CoordinatedOperationKind::SystemMaintenanceExecution)
            .expect("maintenance execution operation must start");
        let operation_id = operation.id();
        {
            let mut registry = execution_registry().expect("registry must remain available");
            registry.operation_id = Some(operation_id);
            registry.session_guard = Some(operation);
            registry.entries.push(scheduler_entry(
                "synthetic-blocker",
                SystemMaintenanceJobStatus::Running,
                definition.resources.to_vec(),
                false,
            ));
        }

        let updates = Arc::new(Mutex::new(Vec::<SystemMaintenanceJob>::new()));
        let sink_updates = Arc::clone(&updates);
        let request = SystemMaintenanceExecutionRequest {
            scan_id: scan_id.clone(),
            task_id: definition.id.to_string(),
            authorization_prompt: "Authorize maintenance".to_string(),
        };
        let queued = SystemMaintenanceService::start_execution(
            request.clone(),
            Arc::new(move |job| {
                sink_updates
                    .lock()
                    .expect("update collector must remain available")
                    .push(job);
            }),
        )
        .expect("conflicting maintenance request must enqueue");
        assert_eq!(queued.status, SystemMaintenanceJobStatus::Queued);
        assert_eq!(queued.revision, 1);

        let restored =
            SystemMaintenanceService::runtime_state().expect("runtime state must remain readable");
        assert_eq!(
            restored.catalog.expect("catalog must be retained").scan_id,
            scan_id
        );
        assert_eq!(restored.executions.len(), 2);

        let cancelled = SystemMaintenanceService::cancel_execution(&queued.execution_id)
            .expect("queued execution must cancel safely");
        assert_eq!(cancelled.status, SystemMaintenanceJobStatus::Finished);
        assert_eq!(cancelled.revision, 2);
        assert_eq!(
            cancelled
                .result
                .as_ref()
                .and_then(|result| result.failure_reason),
            Some(SystemMaintenanceFailureReason::UserCancelled)
        );
        assert_eq!(
            updates
                .lock()
                .expect("update collector must remain available")
                .as_slice(),
            std::slice::from_ref(&cancelled)
        );
        assert_eq!(
            SystemMaintenanceService::cancel_execution(&queued.execution_id)
                .expect("terminal cancellation must be idempotent"),
            cancelled
        );

        // A cancelled queued job made no system change, so retrying against the same catalog is
        // safe. It must still remain queued behind the same synthetic resource conflict.
        let retried = SystemMaintenanceService::start_execution(request, Arc::new(|_| {}))
            .expect("unchanged cancellation must remain retryable");
        assert_eq!(retried.status, SystemMaintenanceJobStatus::Queued);
        assert_ne!(retried.execution_id, queued.execution_id);
        SystemMaintenanceService::cancel_execution(&retried.execution_id)
            .expect("retry fixture must cancel cleanly");

        assert!(SystemMaintenanceService::cancel_execution("").is_err());
        assert!(SystemMaintenanceService::cancel_execution("missing-job").is_err());
    }

    #[test]
    fn running_cancellation_and_progress_keep_events_and_runtime_state_consistent() {
        let _operation_lock = crate::shared::operation::test_operation_lock();
        reset_global_service_state();
        let _reset = GlobalServiceStateReset;
        let updates = Arc::new(Mutex::new(Vec::<SystemMaintenanceJob>::new()));
        let sink_updates = Arc::clone(&updates);
        let mut entry = scheduler_entry(
            "running-safe-cancel",
            SystemMaintenanceJobStatus::Running,
            vec![MaintenanceResource::Network],
            false,
        );
        entry.sink = Arc::new(move |job| {
            sink_updates
                .lock()
                .expect("update collector must remain available")
                .push(job);
        });
        let cancellation = Arc::clone(&entry.cancellation);
        let operation = OperationGuard::start(CoordinatedOperationKind::SystemMaintenanceExecution)
            .expect("maintenance execution operation must start");
        {
            let mut registry = execution_registry().expect("registry must remain available");
            registry.operation_id = Some(operation.id());
            registry.session_guard = Some(operation);
            registry.entries.push(entry);
        }

        let progress = PlatformSystemMaintenanceProgress::step(
            mangodisk_platform::PlatformSystemMaintenancePhase::RefreshingNetwork,
            1,
            2,
            Some(40),
        );
        update_execution_progress("running-safe-cancel", progress)
            .expect("valid progress must be published");
        update_execution_progress("running-safe-cancel", progress)
            .expect("duplicate progress must be an idempotent no-op");
        assert_eq!(
            updates
                .lock()
                .expect("update collector must remain available")
                .len(),
            1
        );
        assert_eq!(
            update_execution_progress(
                "running-safe-cancel",
                PlatformSystemMaintenanceProgress::step(
                    mangodisk_platform::PlatformSystemMaintenancePhase::RefreshingNetwork,
                    2,
                    1,
                    Some(101),
                ),
            )
            .expect_err("invalid progress must fail closed")
            .code(),
            CoreErrorCode::Platform
        );

        let cancelling = SystemMaintenanceService::cancel_execution("running-safe-cancel")
            .expect("a cancelable running job must accept cancellation");
        assert_eq!(cancelling.status, SystemMaintenanceJobStatus::Cancelling);
        assert_eq!(cancelling.revision, 3);
        assert!(!cancelling.cancelable);
        assert!(cancellation.load(Ordering::Relaxed));
        let restored =
            SystemMaintenanceService::runtime_state().expect("runtime state must remain readable");
        assert_eq!(restored.executions, vec![cancelling.clone()]);
        assert_eq!(
            updates
                .lock()
                .expect("update collector must remain available")
                .last(),
            Some(&cancelling)
        );
        assert!(SystemMaintenanceService::cancel_execution("running-safe-cancel").is_err());
    }

    #[test]
    fn scheduler_starts_two_independent_jobs() {
        let mut registry = ExecutionRegistry {
            operation_id: Some(7),
            entries: vec![
                scheduler_entry(
                    "network",
                    SystemMaintenanceJobStatus::Queued,
                    vec![MaintenanceResource::Network],
                    false,
                ),
                scheduler_entry(
                    "search",
                    SystemMaintenanceJobStatus::Queued,
                    vec![MaintenanceResource::SearchIndex],
                    false,
                ),
                scheduler_entry(
                    "shell",
                    SystemMaintenanceJobStatus::Queued,
                    vec![MaintenanceResource::ShellCache],
                    false,
                ),
            ],
            ..ExecutionRegistry::default()
        };

        let scheduled = schedule_executions(&mut registry).expect("jobs must schedule");
        assert_eq!(scheduled.dispatches.len(), 2);
        assert_eq!(running_count(&registry), 2);
        assert_eq!(
            registry.entries[2].public.status,
            SystemMaintenanceJobStatus::Queued
        );
    }

    #[test]
    fn scheduler_queues_conflicting_jobs() {
        let mut registry = ExecutionRegistry {
            operation_id: Some(8),
            entries: vec![
                scheduler_entry(
                    "quicklook",
                    SystemMaintenanceJobStatus::Queued,
                    vec![MaintenanceResource::ShellCache],
                    false,
                ),
                scheduler_entry(
                    "icons",
                    SystemMaintenanceJobStatus::Queued,
                    vec![MaintenanceResource::ShellCache],
                    false,
                ),
            ],
            ..ExecutionRegistry::default()
        };

        let scheduled = schedule_executions(&mut registry).expect("jobs must schedule");
        assert_eq!(scheduled.dispatches.len(), 1);
        assert_eq!(
            registry.entries[1].public.status,
            SystemMaintenanceJobStatus::Queued
        );
    }

    #[test]
    fn scheduler_serializes_elevated_jobs() {
        #[cfg(target_os = "macos")]
        let repair_resource = MaintenanceResource::FileSystemPermissions;
        #[cfg(windows)]
        let repair_resource = MaintenanceResource::SystemRepair;
        let mut registry = ExecutionRegistry {
            operation_id: Some(9),
            entries: vec![
                scheduler_entry(
                    "repair",
                    SystemMaintenanceJobStatus::Queued,
                    vec![repair_resource, MaintenanceResource::Elevation],
                    true,
                ),
                scheduler_entry(
                    "dns",
                    SystemMaintenanceJobStatus::Queued,
                    vec![MaintenanceResource::Network, MaintenanceResource::Elevation],
                    true,
                ),
            ],
            ..ExecutionRegistry::default()
        };

        let scheduled = schedule_executions(&mut registry).expect("jobs must schedule");
        assert_eq!(scheduled.dispatches.len(), 1);
        assert!(!registry.entries[0].public.cancelable);
        assert_eq!(
            registry.entries[1].public.status,
            SystemMaintenanceJobStatus::Queued
        );
    }

    #[test]
    fn scheduler_starts_waiting_job_after_conflict_finishes() {
        let mut registry = ExecutionRegistry {
            operation_id: Some(10),
            entries: vec![
                scheduler_entry(
                    "quicklook",
                    SystemMaintenanceJobStatus::Running,
                    vec![MaintenanceResource::ShellCache],
                    false,
                ),
                scheduler_entry(
                    "icons",
                    SystemMaintenanceJobStatus::Queued,
                    vec![MaintenanceResource::ShellCache],
                    false,
                ),
            ],
            ..ExecutionRegistry::default()
        };

        let initially_scheduled = schedule_executions(&mut registry).expect("jobs must schedule");
        assert!(initially_scheduled.dispatches.is_empty());
        registry.entries[0].public.status = SystemMaintenanceJobStatus::Finished;
        let scheduled = schedule_executions(&mut registry).expect("jobs must schedule");

        assert_eq!(scheduled.dispatches.len(), 1);
        assert_eq!(
            registry.entries[1].public.status,
            SystemMaintenanceJobStatus::Running
        );
    }

    #[test]
    fn only_unchanged_failures_can_retry_without_a_new_scan() {
        let mut retryable = scheduler_entry(
            "retryable",
            SystemMaintenanceJobStatus::Finished,
            vec![MaintenanceResource::Network],
            false,
        );
        retryable.public.result = Some(SystemMaintenanceExecutionItemResult {
            task_id: "retryable".to_string(),
            status: SystemMaintenanceExecutionStatus::Failed,
            mutation_state: SystemMaintenanceMutationState::NotChanged,
            verified: false,
            requires_restart: false,
            failure_reason: Some(SystemMaintenanceFailureReason::PlatformFailure),
        });
        assert!(is_retryable(&retryable));

        retryable
            .public
            .result
            .as_mut()
            .expect("the retry fixture must have a result")
            .mutation_state = SystemMaintenanceMutationState::MayHaveChanged;
        assert!(!is_retryable(&retryable));

        retryable
            .public
            .result
            .as_mut()
            .expect("the retry fixture must have a result")
            .mutation_state = SystemMaintenanceMutationState::NotChanged;
        retryable
            .public
            .result
            .as_mut()
            .expect("the retry fixture must have a result")
            .status = SystemMaintenanceExecutionStatus::Completed;
        assert!(!is_retryable(&retryable));
    }

    #[test]
    fn finished_execution_retention_is_bounded_to_the_latest_jobs() {
        let mut registry = ExecutionRegistry {
            entries: (0..=MAX_RETAINED_EXECUTIONS)
                .map(|index| {
                    scheduler_entry(
                        &format!("finished-{index}"),
                        SystemMaintenanceJobStatus::Finished,
                        vec![MaintenanceResource::Network],
                        false,
                    )
                })
                .collect(),
            ..ExecutionRegistry::default()
        };

        prune_finished_executions(&mut registry);

        assert_eq!(registry.entries.len(), MAX_RETAINED_EXECUTIONS);
        assert_eq!(registry.entries[0].public.execution_id, "finished-1");
    }

    #[test]
    fn platform_side_effect_uncertainty_is_preserved() {
        let error = PlatformError::operation_failed("failed after native write")
            .with_possible_side_effects();
        let item = failed_platform_item(
            7,
            "execution-test",
            "maintenance.test".to_string(),
            &error,
            10,
        );

        assert_eq!(
            item.mutation_state,
            SystemMaintenanceMutationState::MayHaveChanged
        );
        assert_eq!(
            item.failure_reason,
            Some(SystemMaintenanceFailureReason::PlatformFailure)
        );
    }

    #[test]
    fn queued_cancellation_remains_a_typed_result() {
        let item = cancelled_item("maintenance.test".to_string());

        assert_eq!(
            item.failure_reason,
            Some(SystemMaintenanceFailureReason::UserCancelled)
        );
        assert_eq!(
            item.mutation_state,
            SystemMaintenanceMutationState::NotChanged
        );
    }

    #[test]
    fn worker_panics_become_uncertain_terminal_results() {
        let result = contain_execution_panic(
            11,
            "execution-panic",
            "maintenance.test",
            Instant::now(),
            || panic!("simulated platform panic"),
        );

        assert_eq!(result.status, SystemMaintenanceExecutionStatus::Failed);
        assert_eq!(
            result.mutation_state,
            SystemMaintenanceMutationState::MayHaveChanged
        );
        assert_eq!(
            result.failure_reason,
            Some(SystemMaintenanceFailureReason::PlatformFailure)
        );
    }

    #[test]
    fn panicking_update_sinks_do_not_escape_the_delivery_boundary() {
        let job = scheduler_entry(
            "delivery-panic",
            SystemMaintenanceJobStatus::Running,
            vec![MaintenanceResource::Network],
            false,
        )
        .public;

        publish_job_update(Arc::new(|_| panic!("simulated event sink panic")), job);
    }

    /// Exercises the complete background registry against one low-impact native maintenance
    /// task. It remains ignored because it intentionally refreshes a real platform cache.
    #[test]
    #[ignore = "changes real system cache state; run only on an explicitly authorized host"]
    fn actual_background_execution_returns_before_native_completion() {
        #[cfg(target_os = "macos")]
        let task_id = "macos.maintenance.quicklook-cache";
        #[cfg(windows)]
        let task_id = "windows.maintenance.dns-cache";

        let catalog = SystemMaintenanceService::scan().expect("catalog scan must succeed");
        let (sender, receiver) = std::sync::mpsc::channel();
        let requested_at = Instant::now();
        let initial = SystemMaintenanceService::start_execution(
            SystemMaintenanceExecutionRequest {
                scan_id: catalog.scan_id,
                task_id: task_id.to_string(),
                authorization_prompt: "MangoDisk needs permission to run system maintenance"
                    .to_string(),
            },
            Arc::new(move |job| {
                sender
                    .send(job)
                    .expect("job update receiver must remain alive");
            }),
        )
        .expect("background maintenance must enqueue");

        assert!(requested_at.elapsed() < std::time::Duration::from_secs(2));
        assert!(matches!(
            initial.status,
            SystemMaintenanceJobStatus::Queued | SystemMaintenanceJobStatus::Running
        ));

        let finished = loop {
            let update = receiver
                .recv_timeout(std::time::Duration::from_secs(60))
                .expect("background maintenance must publish a terminal state");
            if update.status == SystemMaintenanceJobStatus::Finished {
                break update;
            }
        };
        let result = finished.result.expect("terminal job must contain a result");
        assert_ne!(result.status, SystemMaintenanceExecutionStatus::Failed);
        assert!(SystemMaintenanceService::runtime_state()
            .expect("runtime state must remain readable")
            .executions
            .iter()
            .all(|job| job.status == SystemMaintenanceJobStatus::Finished));
    }
}
