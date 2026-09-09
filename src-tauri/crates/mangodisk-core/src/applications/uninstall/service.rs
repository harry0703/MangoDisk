use std::collections::{HashMap, HashSet};
#[cfg(target_os = "macos")]
use std::path::Path;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

#[cfg(windows)]
use mangodisk_platform::ApplicationInstallScope;
use mangodisk_platform::{
    current_platform, ApplicationInventorySource, ApplicationUninstallRegistration,
    InstalledApplication, Platform, PlatformCancellation,
};
#[cfg(target_os = "macos")]
use mangodisk_platform::{
    macos_privileged_application_removal_supported, remove_application_bundle_with_privileges,
    MacosPrivilegedApplicationRemovalOutcome,
};

#[cfg(target_os = "macos")]
use crate::filesystem::permanent_delete::{
    delete_path_permanently, prepare_path_for_permanent_delete,
};
use crate::shared::operation::OPERATION_CANCELLED_ERROR;
use crate::{
    applications::{
        catalog::{ProcessSnapshot, ScanContext},
        process_control::{close_resolved_applications, ResolvedApplicationCloseTarget},
    },
    filesystem::metadata::{display_path, now_ms},
    history::{
        ApplicationUninstallApplicationDetails, ApplicationUninstallOperationDetails,
        HistoryService, OperationCategory, OperationDetails, OperationOutcome, OperationRecord,
        OPERATION_RECORD_SCHEMA_VERSION,
    },
    shared::progress::{ProgressSink, ProgressTracker, TraversalStage},
    shared::{
        operation::{CoordinatedOperationKind, OperationCancellationToken, OperationGuard},
        CoreError, CoreResult,
    },
};

#[cfg(target_os = "macos")]
use super::execution;
#[cfg(target_os = "macos")]
use super::macos;
#[cfg(windows)]
use super::models::ApplicationUninstallActionResult;
use super::models::{
    ApplicationUninstallActionReason, ApplicationUninstallActionStatus,
    ApplicationUninstallBatchPlan, ApplicationUninstallBatchPreparation,
    ApplicationUninstallBatchResult, ApplicationUninstallBatchSelection,
    ApplicationUninstallCandidate, ApplicationUninstallCapability,
    ApplicationUninstallCloseRequest, ApplicationUninstallComponentKind,
    ApplicationUninstallComponentSummary, ApplicationUninstallExecutionItemResult,
    ApplicationUninstallExecutionItemStatus, ApplicationUninstallExecutionMode,
    ApplicationUninstallExecutionProgress, ApplicationUninstallExecutionStage,
    ApplicationUninstallInspection, ApplicationUninstallInstallerKind,
    ApplicationUninstallInventorySource, ApplicationUninstallPlan, ApplicationUninstallPlanItem,
    ApplicationUninstallPlatform, ApplicationUninstallRecordState, ApplicationUninstallResult,
    ApplicationUninstallScanResult, ApplicationUninstallSourceIdentity,
    APPLICATION_UNINSTALL_SCAN_SCHEMA_VERSION,
};
#[cfg(windows)]
use super::windows;
use super::{batch, plan, preflight};

#[cfg(windows)]
const CURRENT_APPLICATION_NAME: &str = "MangoDisk";

struct PreflightCandidate {
    result: ApplicationUninstallResult,
    inspection: Option<ApplicationUninstallInspection>,
    process_target: Option<ResolvedApplicationCloseTarget>,
}

struct UninstallExecutionReporter<F> {
    handler: F,
    started: Instant,
    completed_applications: Vec<ApplicationUninstallExecutionItemResult>,
    completed_application_count: u64,
    total_application_count: u64,
    affected_application_count: u64,
    failed_application_count: u64,
    released_bytes: u64,
}

impl<F> UninstallExecutionReporter<F>
where
    F: FnMut(ApplicationUninstallExecutionProgress),
{
    fn new(total_application_count: usize, handler: F) -> Self {
        Self {
            handler,
            started: Instant::now(),
            completed_applications: Vec::with_capacity(total_application_count),
            completed_application_count: 0,
            total_application_count: total_application_count as u64,
            affected_application_count: 0,
            failed_application_count: 0,
            released_bytes: 0,
        }
    }

    fn emit(
        &mut self,
        stage: ApplicationUninstallExecutionStage,
        current_application_id: Option<&str>,
    ) {
        (self.handler)(ApplicationUninstallExecutionProgress {
            stage,
            current_application_id: current_application_id.map(str::to_owned),
            completed_applications: self.completed_applications.clone(),
            completed_application_count: self.completed_application_count,
            total_application_count: self.total_application_count,
            affected_application_count: self.affected_application_count,
            failed_application_count: self.failed_application_count,
            released_bytes: self.released_bytes,
            elapsed_ms: self.started.elapsed().as_millis() as u64,
        });
    }

    fn record(&mut self, result: &ApplicationUninstallResult) {
        self.completed_applications
            .push(ApplicationUninstallExecutionItemResult {
                application_id: result.application_id.clone(),
                status: if result
                    .actions
                    .iter()
                    .any(|action| action.status == ApplicationUninstallActionStatus::Cancelled)
                {
                    ApplicationUninstallExecutionItemStatus::Cancelled
                } else if result.failed_item_count > 0 {
                    ApplicationUninstallExecutionItemStatus::Failed
                } else {
                    ApplicationUninstallExecutionItemStatus::Completed
                },
                released_bytes: result.released_bytes,
            });
        self.completed_application_count = self
            .completed_application_count
            .saturating_add(1)
            .min(self.total_application_count);
        self.affected_application_count = self
            .affected_application_count
            .saturating_add(u64::from(result.affected_item_count > 0));
        self.failed_application_count = self
            .failed_application_count
            .saturating_add(u64::from(result.failed_item_count > 0));
        self.released_bytes = self.released_bytes.saturating_add(result.released_bytes);
    }
}

pub struct ApplicationUninstallService;

impl ApplicationUninstallService {
    pub fn cancel_scan() {
        OperationCancellationToken::application_scan().cancel();
    }

    /// Requests cooperative cancellation of the active uninstall batch.
    ///
    /// A native Windows uninstaller may own a separate interactive process and
    /// cannot be terminated safely. Core stops waiting without terminating that
    /// process, records it as continuing externally, and prevents every later
    /// application in the batch from starting.
    pub fn cancel_execution() {
        OperationCancellationToken::applications().cancel();
    }

    /// Closes applications selected from a trusted uninstall catalog snapshot.
    ///
    /// Stable application IDs cross the adapter boundary. Core resolves the
    /// corresponding process names and application path from its catalog so
    /// callers cannot provide arbitrary process identities.
    pub fn close_applications_from_catalog(
        request: ApplicationUninstallCloseRequest,
        scan: &mut ApplicationUninstallScanResult,
    ) -> CoreResult<crate::ApplicationCloseBatchResult> {
        let operation = OperationGuard::start(CoordinatedOperationKind::ApplicationClose)?;
        if !scan.catalog_actionable {
            return Err(CoreError::operation_failed(
                "application uninstall catalog is not actionable",
            ));
        }
        let selected = request
            .application_ids
            .iter()
            .cloned()
            .collect::<HashSet<_>>();
        if selected.is_empty() || selected.len() != request.application_ids.len() {
            return Err(CoreError::invalid_input(
                "the application close selection is invalid",
            ));
        }

        let mut targets = Vec::with_capacity(request.application_ids.len());
        for application_id in &request.application_ids {
            let candidate = scan
                .candidates
                .iter()
                .find(|candidate| candidate.application_id == *application_id)
                .ok_or_else(|| {
                    CoreError::invalid_input(
                        "the application close request contains an unknown application",
                    )
                })?;
            if candidate.running_processes.is_empty() {
                return Err(CoreError::invalid_input(
                    "the application close target is not running in the reviewed catalog",
                ));
            }
            targets.push(close_target(candidate));
        }
        let result = close_resolved_applications(targets, request.mode)?;
        refresh_catalog_after_close(scan, &result);
        operation.complete();
        Ok(result)
    }

    /// Explicitly removes the selected installation record, independently of uninstallability.
    /// Native code resolves the ID under fixed registry roots and revalidates that exact snapshot.
    /// A catalog-wide revision, running processes, or missing files cannot veto this separate
    /// user action. No application paths or filesystem cleanup plans participate in it.
    pub fn remove_application_record(application_id: &str, dry_run: bool) -> CoreResult<()> {
        let operation = OperationGuard::start(CoordinatedOperationKind::Applications)?;
        // Reject arbitrary transport input before it can enter diagnostic logs.
        if !application_id
            .strip_prefix("application-")
            .is_some_and(|suffix| {
                suffix.len() == 24 && suffix.bytes().all(|byte| byte.is_ascii_hexdigit())
            })
        {
            return Err(CoreError::invalid_input("invalid application record ID"));
        }
        log::info!("application_record_removal_started operation_id={} application_id={} dry_run={dry_run}", operation.id(), application_id);
        let result = current_platform().remove_application_record(application_id, dry_run);
        if let Err(error) = &result {
            if error.code() == mangodisk_platform::PlatformErrorCode::UserCancelled {
                log::info!("application_record_removal_cancelled operation_id={} application_id={application_id}", operation.id());
            } else {
                log::warn!("application_record_removal_failed operation_id={} application_id={} code={:?} mutation_state={:?} error_digest={}", operation.id(), application_id, error.code(), error.mutation_state(), blake3::hash(error.as_bytes()).to_hex());
            }
        }
        result?;
        log::info!("application_record_removal_finished operation_id={} application_id={} dry_run={dry_run} verified=true files_deleted=0", operation.id(), application_id);
        operation.complete();
        Ok(())
    }

    pub fn scan() -> CoreResult<ApplicationUninstallScanResult> {
        let operation = OperationGuard::start(CoordinatedOperationKind::ApplicationScan)?;
        let result = scan_without_guard(operation.id(), true, None, operation.cancellation_flag())?;
        operation.complete();
        Ok(result)
    }

    pub fn scan_with_progress(
        callback: impl ProgressSink,
    ) -> CoreResult<ApplicationUninstallScanResult> {
        let operation = OperationGuard::start(CoordinatedOperationKind::ApplicationScan)?;
        let result = scan_without_guard(
            operation.id(),
            true,
            Some(Box::new(callback)),
            operation.cancellation_flag(),
        )?;
        operation.complete();
        Ok(result)
    }

    pub fn inspect(application_id: &str) -> CoreResult<ApplicationUninstallInspection> {
        if application_id.trim().is_empty() {
            return Err(CoreError::invalid_input(
                "application uninstall identifier is empty",
            ));
        }
        let operation = OperationGuard::start(CoordinatedOperationKind::ApplicationPreparation)?;
        let started = Instant::now();
        let scan = scan_without_guard(operation.id(), false, None, operation.cancellation_flag())?;
        if !scan.catalog_actionable {
            return Err(CoreError::operation_failed(
                "application uninstall catalog is not actionable",
            ));
        }
        let candidate = scan
            .candidates
            .iter()
            .find(|candidate| candidate.application_id == application_id)
            .ok_or_else(|| "application uninstall candidate is unavailable".to_string())?;
        let catalog_revision = scan
            .catalog_revision
            .as_deref()
            .ok_or_else(|| "application uninstall catalog revision is unavailable".to_string())?;

        let inspection = inspect_candidate(candidate, catalog_revision, started)?;

        log::info!(
            "application_uninstall_inspection_ready operation_id={} component_count={} total_bytes={} default_selected_bytes={} elapsed_ms={}",
            operation.id(),
            inspection.components.len(),
            inspection.total_bytes,
            inspection.default_selected_bytes,
            inspection.elapsed_ms
        );
        operation.complete();
        Ok(inspection)
    }

    pub fn create_plan(
        inspection: &ApplicationUninstallInspection,
        component_ids: &[String],
    ) -> CoreResult<ApplicationUninstallPlan> {
        Ok(plan::create_plan(inspection, component_ids)?)
    }

    pub fn create_plan_for_application(
        application_id: &str,
        component_ids: &[String],
    ) -> CoreResult<ApplicationUninstallPlan> {
        // Re-inspect in Core so GUI and other adapters submit only stable
        // component IDs. Paths, sizes, and snapshots are always sourced from
        // the current operating-system inventory at the trust boundary.
        let inspection = Self::inspect(application_id)?;
        Self::create_plan(&inspection, component_ids)
    }

    pub fn create_reviewed_plan(
        application_id: String,
        catalog_revision: String,
        items: Vec<ApplicationUninstallPlanItem>,
    ) -> Result<ApplicationUninstallPlan, String> {
        plan::create_reviewed_plan(application_id, catalog_revision, items)
    }

    pub fn validate_plan(plan: &ApplicationUninstallPlan) -> Result<(), String> {
        plan::validate_plan(plan)
    }

    pub fn create_batch_plan(
        selections: &[ApplicationUninstallBatchSelection],
    ) -> CoreResult<ApplicationUninstallBatchPlan> {
        validate_batch_selections(selections)?;
        let operation = OperationGuard::start(CoordinatedOperationKind::ApplicationPreparation)?;
        let started = Instant::now();
        let scan = scan_without_guard(operation.id(), false, None, operation.cancellation_flag())?;
        let (batch_plan, _) = create_batch_plan_from_scan(selections, &scan)?;
        log::info!(
            "application_uninstall_batch_plan_ready operation_id={} application_count={} component_count={} expected_bytes={} elapsed_ms={}",
            operation.id(),
            batch_plan.plans.len(),
            batch_plan.plans.iter().map(|plan| plan.items.len()).sum::<usize>(),
            batch_plan.expected_bytes,
            started.elapsed().as_millis()
        );
        operation.complete();
        Ok(batch_plan)
    }

    pub fn prepare_batch(
        selections: &[ApplicationUninstallBatchSelection],
    ) -> CoreResult<ApplicationUninstallBatchPreparation> {
        validate_batch_selections(selections)?;
        let operation = OperationGuard::start(CoordinatedOperationKind::ApplicationPreparation)?;
        let started = Instant::now();
        let scan = scan_without_guard(operation.id(), false, None, operation.cancellation_flag())?;
        let preparation = prepare_batch_from_scan(operation.id(), selections, &scan, started)?;
        operation.complete();
        Ok(preparation)
    }

    pub fn prepare_batch_from_catalog(
        selections: &[ApplicationUninstallBatchSelection],
        scan: &ApplicationUninstallScanResult,
    ) -> CoreResult<ApplicationUninstallBatchPreparation> {
        validate_batch_selections(selections)?;
        let operation = OperationGuard::start(CoordinatedOperationKind::ApplicationPreparation)?;
        let started = Instant::now();
        let preparation = prepare_batch_from_scan(operation.id(), selections, scan, started)?;
        operation.complete();
        Ok(preparation)
    }

    pub fn validate_batch_plan(plan: &ApplicationUninstallBatchPlan) -> Result<(), String> {
        batch::validate_plan(plan)
    }

    pub fn execute_batch(
        batch_plan: ApplicationUninstallBatchPlan,
        dry_run: bool,
    ) -> CoreResult<ApplicationUninstallBatchResult> {
        Self::execute_batch_with_progress(batch_plan, dry_run, None, |_| {})
    }

    /// Executes a prepared batch and forwards an ephemeral native authorization prompt.
    ///
    /// The prompt is UI context only: it is never persisted or logged, and platforms
    /// that do not display MangoDisk-managed authorization UI ignore it.
    pub fn execute_batch_with_progress<F>(
        batch_plan: ApplicationUninstallBatchPlan,
        dry_run: bool,
        authorization_prompt: Option<&str>,
        progress: F,
    ) -> CoreResult<ApplicationUninstallBatchResult>
    where
        F: FnMut(ApplicationUninstallExecutionProgress),
    {
        Self::validate_batch_plan(&batch_plan)?;
        let operation = OperationGuard::start(CoordinatedOperationKind::Applications)?;
        let started_at_ms = now_ms();
        let started = Instant::now();
        let mut progress = UninstallExecutionReporter::new(batch_plan.plans.len(), progress);
        progress.emit(ApplicationUninstallExecutionStage::Validating, None);
        let scan = scan_without_guard(operation.id(), false, None, operation.cancellation_flag())?;
        if !scan.catalog_actionable {
            return Err(CoreError::operation_failed(
                "application uninstall catalog is not actionable",
            ));
        }

        // The global inventory revision can change when an unrelated app is
        // installed or removed. Revalidate each selected registration and its
        // component fingerprint instead of rejecting every plan for unrelated
        // catalog churn. This preserves the mutation-boundary safety check while
        // allowing users to uninstall another app without a manual rescan.
        let preflighted = batch_plan
            .plans
            .iter()
            .map(|plan| preview_candidate(plan, &scan, started))
            .collect::<Result<Vec<_>, _>>()?;

        let mut results = if dry_run {
            preflighted
                .into_iter()
                .map(|candidate| {
                    progress.record(&candidate.result);
                    progress.emit(
                        ApplicationUninstallExecutionStage::Validating,
                        Some(&candidate.result.application_id),
                    );
                    candidate.result
                })
                .collect::<Vec<_>>()
        } else {
            // Native uninstallers can block for minutes and may open their own
            // UI. Execute one prepared application at a time so the current
            // target and completed count always describe real batch state.
            let mut results = Vec::with_capacity(batch_plan.plans.len());
            for (plan, candidate) in batch_plan.plans.iter().zip(preflighted) {
                progress.emit(
                    ApplicationUninstallExecutionStage::Uninstalling,
                    Some(&plan.application_id),
                );
                // Cancellation is checked only at an application boundary. A
                // currently open native uninstaller remains in control of its
                // own process, while no later application is allowed to start.
                let result = if operation.ensure_not_cancelled().is_err() {
                    preflight::cancel_all(plan, candidate.result.application_name, None)
                } else {
                    execute_preflighted(
                        plan,
                        candidate,
                        operation.cancellation_flag(),
                        authorization_prompt,
                    )
                };
                log::info!(
                    "application_uninstall_item_finished operation_id={} application_id={} plan_id={} dry_run={} affected_count={} failed_count={} reason={}",
                    operation.id(), result.application_id, result.plan_id, result.dry_run,
                    result.affected_item_count, result.failed_item_count,
                    result.actions.iter().find_map(|action| action.reason).map_or("none", |reason| reason.stable_code())
                );
                progress.record(&result);
                progress.emit(
                    ApplicationUninstallExecutionStage::Uninstalling,
                    Some(&plan.application_id),
                );
                results.push(result);
            }
            results
        };
        let history_saved = !dry_run
            && save_batch_execution_history(
                operation.id(),
                started_at_ms,
                now_ms(),
                &batch_plan,
                &results,
                &scan.candidates,
            );
        for result in &mut results {
            result.history_saved = history_saved;
        }
        let result = aggregate_batch_result(&batch_plan, dry_run, results);
        progress.emit(ApplicationUninstallExecutionStage::Finalizing, None);
        let cancelled_application_count = result
            .results
            .iter()
            .filter(|application| {
                application
                    .actions
                    .iter()
                    .any(|action| action.status == ApplicationUninstallActionStatus::Cancelled)
            })
            .count();
        log::info!(
            "application_uninstall_batch_finished operation_id={} dry_run={} application_count={} affected_application_count={} failed_application_count={} cancelled_application_count={} component_count={} affected_count={} failed_count={} expected_bytes={} released_bytes={} history_saved={} elapsed_ms={}",
            operation.id(),
            dry_run,
            result.selected_application_count,
            result.affected_application_count,
            result.failed_application_count,
            cancelled_application_count,
            batch_plan.plans.iter().map(|plan| plan.items.len()).sum::<usize>(),
            result.affected_item_count,
            result.failed_item_count,
            result.expected_bytes,
            result.released_bytes,
            history_saved,
            started.elapsed().as_millis()
        );
        operation.complete();
        Ok(result)
    }

    pub fn execute(
        plan: ApplicationUninstallPlan,
        dry_run: bool,
    ) -> CoreResult<ApplicationUninstallResult> {
        Self::validate_plan(&plan)?;
        let operation = OperationGuard::start(CoordinatedOperationKind::Applications)?;
        let started_at_ms = now_ms();
        let started = Instant::now();
        let scan = scan_without_guard(operation.id(), false, None, operation.cancellation_flag())?;
        if !scan.catalog_actionable {
            return Err(CoreError::operation_failed(
                "application uninstall catalog is not actionable",
            ));
        }

        let preflight = preview_candidate(&plan, &scan, started)?;
        if dry_run {
            log_preflight(operation.id(), &plan, &preflight.result, started);
            operation.complete();
            return Ok(preflight.result);
        }

        let mut result = execute_preflighted(&plan, preflight, operation.cancellation_flag(), None);
        let application = scan
            .candidates
            .iter()
            .find(|application| application.application_id == plan.application_id);
        result.history_saved = save_execution_history(
            operation.id(),
            started_at_ms,
            now_ms(),
            &plan,
            &result,
            application,
        );
        log::info!(
            "application_uninstall_execution_finished operation_id={} component_count={} affected_count={} failed_count={} expected_bytes={} released_bytes={} history_saved={} elapsed_ms={}",
            operation.id(),
            plan.items.len(),
            result.affected_item_count,
            result.failed_item_count,
            result.expected_bytes,
            result.released_bytes,
            result.history_saved,
            started.elapsed().as_millis()
        );
        operation.complete();
        Ok(result)
    }
}

fn close_target(candidate: &ApplicationUninstallCandidate) -> ResolvedApplicationCloseTarget {
    let mut executable_names = candidate.running_processes.clone();
    executable_names.push(candidate.name.clone());
    executable_names.push(candidate.primary_identifier.clone());
    executable_names.sort_by_key(|name| name.to_ascii_lowercase());
    executable_names.dedup_by(|left, right| left.eq_ignore_ascii_case(right));
    ResolvedApplicationCloseTarget {
        target_id: candidate.application_id.clone(),
        // Exact platform inventory paths are the primary authority. Process
        // names are retained only for catalog sources without an executable
        // path; OR-matching both could close another installation's helper.
        executable_names: if candidate.executable_paths.is_empty() {
            executable_names
        } else {
            Vec::new()
        },
        executable_paths: candidate.executable_paths.clone(),
    }
}

/// Applies a trusted process-close result to the catalog snapshot used for
/// uninstall planning. Closing a process does not change the platform
/// inventory revision, so rebuilding application components and related-path
/// summaries would add substantial latency without improving safety. Targets
/// that still have a matching process remain blocked; only targets confirmed
/// as fully stopped receive their underlying platform capability.
fn refresh_catalog_after_close(
    scan: &mut ApplicationUninstallScanResult,
    result: &crate::ApplicationCloseBatchResult,
) {
    let stopped_ids = result
        .targets
        .iter()
        .filter(|target| {
            target.status == crate::ApplicationCloseTargetStatus::Completed
                && target.remaining_processes.is_empty()
        })
        .map(|target| target.target_id.as_str())
        .collect::<HashSet<_>>();

    for candidate in &mut scan.candidates {
        if stopped_ids.contains(candidate.application_id.as_str())
            && candidate.capability == ApplicationUninstallCapability::ApplicationRunning
        {
            candidate.running_processes.clear();
            candidate.capability = capability_after_close(candidate);
        }
    }
    scan.ready_count = scan
        .candidates
        .iter()
        .filter(|candidate| candidate.capability.supports_execution())
        .count() as u64;
    scan.blocked_count = scan.candidates.len() as u64 - scan.ready_count;
}

#[cfg(target_os = "macos")]
fn capability_after_close(
    candidate: &ApplicationUninstallCandidate,
) -> ApplicationUninstallCapability {
    let Some(path) = candidate.application_path.as_deref().map(Path::new) else {
        // A running candidate originally passed the bundle safety checks. If
        // its path is unexpectedly unavailable now, fail closed instead of
        // making permanent deletion actionable from incomplete evidence.
        return ApplicationUninstallCapability::ViewOnly;
    };
    if !macos_bundle_is_deletable_without_elevation(path) {
        return if macos_privileged_application_removal_supported(path) {
            ApplicationUninstallCapability::RequiresElevation
        } else {
            ApplicationUninstallCapability::ViewOnly
        };
    }
    ApplicationUninstallCapability::Ready
}

#[cfg(windows)]
fn capability_after_close(
    candidate: &ApplicationUninstallCandidate,
) -> ApplicationUninstallCapability {
    let Some(registration) = candidate.uninstall_registration.as_ref() else {
        return ApplicationUninstallCapability::ViewOnly;
    };
    if matches!(
        registration,
        ApplicationUninstallRegistration::WindowsMsi {
            scope: ApplicationInstallScope::Machine,
            ..
        } | ApplicationUninstallRegistration::WindowsChocolatey { .. }
    ) {
        return ApplicationUninstallCapability::RequiresElevation;
    }
    ApplicationUninstallCapability::Ready
}

#[cfg(not(any(target_os = "macos", windows)))]
fn capability_after_close(
    _candidate: &ApplicationUninstallCandidate,
) -> ApplicationUninstallCapability {
    ApplicationUninstallCapability::ViewOnly
}

fn prepare_batch_from_scan(
    operation_id: u64,
    selections: &[ApplicationUninstallBatchSelection],
    scan: &ApplicationUninstallScanResult,
    started: Instant,
) -> Result<ApplicationUninstallBatchPreparation, String> {
    let (plan, inspections) = create_batch_plan_from_scan(selections, scan)?;
    let results = plan
        .plans
        .iter()
        .zip(inspections)
        .map(|(application_plan, inspection)| preflight::compare(application_plan, &inspection))
        .collect();
    let preview = aggregate_batch_result(&plan, true, results);
    log::info!(
        "application_uninstall_batch_prepared operation_id={} application_count={} component_count={} expected_bytes={} failed_count={} elapsed_ms={}",
        operation_id,
        plan.plans.len(),
        plan.plans.iter().map(|plan| plan.items.len()).sum::<usize>(),
        plan.expected_bytes,
        preview.failed_item_count,
        started.elapsed().as_millis()
    );
    Ok(ApplicationUninstallBatchPreparation { plan, preview })
}

fn create_batch_plan_from_scan(
    selections: &[ApplicationUninstallBatchSelection],
    scan: &ApplicationUninstallScanResult,
) -> Result<
    (
        ApplicationUninstallBatchPlan,
        Vec<ApplicationUninstallInspection>,
    ),
    String,
> {
    if !scan.catalog_actionable {
        return Err("application uninstall catalog is not actionable".to_string());
    }
    let catalog_revision = scan
        .catalog_revision
        .as_deref()
        .ok_or_else(|| "application uninstall catalog revision is unavailable".to_string())?;
    let mut inspections = HashMap::with_capacity(selections.len());
    let mut application_plans = Vec::with_capacity(selections.len());
    for selection in selections {
        let candidate = scan
            .candidates
            .iter()
            .find(|candidate| candidate.application_id == selection.application_id)
            .ok_or_else(|| "application uninstall candidate is unavailable".to_string())?;
        if !candidate.capability.supports_execution() {
            return Err("application is not ready for uninstall planning".to_string());
        }
        let inspection = inspect_candidate(candidate, catalog_revision, Instant::now())?;
        let application_plan = plan::create_plan(&inspection, &selection.component_ids)?;
        inspections.insert(application_plan.application_id.clone(), inspection);
        application_plans.push(application_plan);
    }
    let batch_plan = batch::create_plan(catalog_revision.to_string(), application_plans)?;
    let sorted_inspections = batch_plan
        .plans
        .iter()
        .map(|application_plan| {
            inspections
                .remove(&application_plan.application_id)
                .ok_or_else(|| "application uninstall inspection is unavailable".to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok((batch_plan, sorted_inspections))
}

fn preview_candidate(
    plan: &ApplicationUninstallPlan,
    scan: &ApplicationUninstallScanResult,
    started: Instant,
) -> Result<PreflightCandidate, String> {
    let Some(candidate) = scan
        .candidates
        .iter()
        .find(|candidate| candidate.application_id == plan.application_id)
    else {
        return Ok(PreflightCandidate {
            result: preflight::fail_all(
                plan,
                None,
                ApplicationUninstallActionReason::ApplicationUnavailable,
            ),
            inspection: None,
            process_target: None,
        });
    };
    let process_target = Some(close_target(candidate));
    if !candidate.capability.supports_execution() {
        let reason = if candidate.capability == ApplicationUninstallCapability::ApplicationRunning {
            ApplicationUninstallActionReason::ApplicationRunning
        } else {
            ApplicationUninstallActionReason::UnsupportedExecutor
        };
        return Ok(PreflightCandidate {
            result: preflight::fail_all(plan, Some(candidate.name.clone()), reason),
            inspection: None,
            process_target,
        });
    }
    let catalog_revision = scan
        .catalog_revision
        .as_deref()
        .ok_or_else(|| "application uninstall catalog revision is unavailable".to_string())?;
    let inspection = match inspect_candidate(candidate, catalog_revision, started) {
        Ok(inspection) => inspection,
        Err(error) => {
            log::warn!(
                "application_uninstall_preflight_inspection_failed error_digest={}",
                blake3::hash(error.as_bytes()).to_hex()
            );
            return Ok(PreflightCandidate {
                result: preflight::fail_all(
                    plan,
                    Some(candidate.name.clone()),
                    ApplicationUninstallActionReason::ComponentUnavailable,
                ),
                inspection: None,
                process_target,
            });
        }
    };
    let result = preflight::compare(plan, &inspection);
    Ok(PreflightCandidate {
        result,
        inspection: Some(inspection),
        process_target,
    })
}

fn log_preflight(
    operation_id: u64,
    plan: &ApplicationUninstallPlan,
    result: &ApplicationUninstallResult,
    started: Instant,
) {
    log::info!(
        "application_uninstall_preflight_finished operation_id={} component_count={} previewed_count={} failed_count={} expected_bytes={} previewed_bytes={} elapsed_ms={}",
        operation_id,
        plan.items.len(),
        result.previewed_item_count,
        result.failed_item_count,
        result.expected_bytes,
        result.previewed_bytes,
        started.elapsed().as_millis()
    );
}

#[cfg(target_os = "macos")]
fn execute_preflighted(
    plan: &ApplicationUninstallPlan,
    preflighted: PreflightCandidate,
    _cancellation: Arc<AtomicBool>,
    authorization_prompt: Option<&str>,
) -> ApplicationUninstallResult {
    if preflighted.result.failed_item_count > 0 {
        let reason = preflighted
            .result
            .actions
            .iter()
            .find_map(|action| action.reason)
            .unwrap_or(ApplicationUninstallActionReason::ComponentChanged);
        let mut result = preflight::fail_all(plan, preflighted.result.application_name, reason);
        result.dry_run = false;
        return result;
    }
    let Some(inspection) = preflighted.inspection else {
        let mut result = preflight::fail_all(
            plan,
            preflighted.result.application_name,
            ApplicationUninstallActionReason::ComponentUnavailable,
        );
        result.dry_run = false;
        return result;
    };
    match application_target_is_running(preflighted.process_target.as_ref()) {
        Ok(false) => {}
        Ok(true) => {
            let mut result = preflight::fail_all(
                plan,
                Some(inspection.application_name),
                ApplicationUninstallActionReason::ApplicationRunning,
            );
            result.dry_run = false;
            return result;
        }
        Err(error) => {
            log::warn!(
                "application_uninstall_process_recheck_failed error_digest={}",
                blake3::hash(error.as_bytes()).to_hex()
            );
            let mut result = preflight::fail_all(
                plan,
                Some(inspection.application_name),
                ApplicationUninstallActionReason::ProcessStateUnavailable,
            );
            result.dry_run = false;
            return result;
        }
    }

    execution::execute_with(
        plan,
        &inspection,
        macos::component_matches,
        |component| {
            let path = component
                .path
                .as_deref()
                .ok_or(ApplicationUninstallActionReason::ComponentUnavailable)?;
            if !macos::component_matches(component) {
                return Err(execution::DeleteFailure::new(
                    ApplicationUninstallActionReason::ComponentChanged,
                    0,
                ));
            }
            if inspection.capability == ApplicationUninstallCapability::RequiresElevation
                && component.kind == ApplicationUninstallComponentKind::ApplicationBinary
            {
                log::info!(
                    "application_uninstall_privileged_removal_requested component_id={}",
                    component.component_id
                );
                return match remove_application_bundle_with_privileges(
                    Path::new(path),
                    authorization_prompt,
                ) {
                    Ok(MacosPrivilegedApplicationRemovalOutcome::Completed) => Ok(()),
                    Ok(MacosPrivilegedApplicationRemovalOutcome::UserCancelled) => {
                        log::info!(
                            "application_uninstall_privileged_removal_cancelled component_id={}",
                            component.component_id
                        );
                        Err(execution::DeleteFailure::cancelled())
                    }
                    Ok(MacosPrivilegedApplicationRemovalOutcome::ItemChanged) => {
                        log::warn!(
                            "application_uninstall_privileged_removal_stopped component_id={} reason=item_changed",
                            component.component_id
                        );
                        Err(execution::DeleteFailure::new(
                            ApplicationUninstallActionReason::ComponentChanged,
                            0,
                        ))
                    }
                    Ok(MacosPrivilegedApplicationRemovalOutcome::RecoveryRequired) => {
                        log::error!(
                            "application_uninstall_privileged_removal_failed component_id={} reason=recovery_required",
                            component.component_id
                        );
                        Err(execution::DeleteFailure::new(
                            ApplicationUninstallActionReason::RecoveryRequired,
                            0,
                        ))
                    }
                    Err(error) => {
                        log::warn!(
                            "application_uninstall_privileged_removal_failed component_id={} error_code={:?} error_digest={}",
                            component.component_id,
                            error.code(),
                            blake3::hash(error.as_bytes()).to_hex()
                        );
                        Err(execution::DeleteFailure::new(
                            ApplicationUninstallActionReason::PermanentDeleteFailed,
                            0,
                        ))
                    }
                };
            }
            let prepared = prepare_path_for_permanent_delete(Path::new(path)).map_err(|_| {
                execution::DeleteFailure::new(ApplicationUninstallActionReason::ComponentChanged, 0)
            })?;
            // Bind the just-in-time component snapshot to the prepared
            // physical identity. A replacement after this check is rejected
            // by the staging boundary before any bytes are removed.
            if !macos::component_matches(component) {
                return Err(execution::DeleteFailure::new(
                    ApplicationUninstallActionReason::ComponentChanged,
                    0,
                ));
            }
            delete_path_permanently(prepared, component.bytes, component.file_count).map_err(|error| {
                log::warn!(
                    "application_uninstall_permanent_delete_failed component_id={} partial={} released_bytes={} error_digest={}",
                    component.component_id,
                    error.is_partial(),
                    error.released_bytes(),
                    blake3::hash(error.to_string().as_bytes()).to_hex()
                );
                execution::DeleteFailure::new(
                    ApplicationUninstallActionReason::PermanentDeleteFailed,
                    error.released_bytes(),
                )
            })
        },
        macos::component_is_absent,
    )
}

#[cfg(windows)]
fn execute_preflighted(
    plan: &ApplicationUninstallPlan,
    preflighted: PreflightCandidate,
    cancellation: Arc<AtomicBool>,
    _authorization_prompt: Option<&str>,
) -> ApplicationUninstallResult {
    if preflighted.result.failed_item_count > 0 {
        let reason = preflighted
            .result
            .actions
            .iter()
            .find_map(|action| action.reason)
            .unwrap_or(ApplicationUninstallActionReason::ComponentChanged);
        let mut result = preflight::fail_all(plan, preflighted.result.application_name, reason);
        result.dry_run = false;
        return result;
    }
    let Some(inspection) = preflighted.inspection else {
        let mut result = preflight::fail_all(
            plan,
            preflighted.result.application_name,
            ApplicationUninstallActionReason::ComponentUnavailable,
        );
        result.dry_run = false;
        return result;
    };
    match application_target_is_running(preflighted.process_target.as_ref()) {
        Ok(false) => {}
        Ok(true) => {
            let mut result = preflight::fail_all(
                plan,
                Some(inspection.application_name),
                ApplicationUninstallActionReason::ApplicationRunning,
            );
            result.dry_run = false;
            return result;
        }
        Err(error) => {
            log::warn!(
                "application_uninstall_process_recheck_failed error_digest={}",
                blake3::hash(error.as_bytes()).to_hex()
            );
            let mut result = preflight::fail_all(
                plan,
                Some(inspection.application_name),
                ApplicationUninstallActionReason::ProcessStateUnavailable,
            );
            result.dry_run = false;
            return result;
        }
    }
    let component = inspection
        .components
        .iter()
        .find(|component| component.kind == ApplicationUninstallComponentKind::NativeInstaller);
    let Some(component) = component else {
        let mut result = preflight::fail_all(
            plan,
            Some(inspection.application_name),
            ApplicationUninstallActionReason::ComponentUnavailable,
        );
        result.dry_run = false;
        return result;
    };
    match windows::execute_registration(&inspection, &plan.plan_id, cancellation) {
        Ok(windows::ApplicationUninstallExecution::Completed(outcome)) => {
            ApplicationUninstallResult {
                plan_id: plan.plan_id.clone(),
                application_id: plan.application_id.clone(),
                application_name: Some(inspection.application_name),
                expected_bytes: plan.expected_bytes,
                previewed_bytes: 0,
                released_bytes: component.bytes,
                previewed_item_count: 0,
                affected_item_count: 1,
                failed_item_count: 0,
                released_bytes_is_estimate: true,
                restart_required: matches!(
                    outcome,
                    mangodisk_platform::ApplicationUninstallExecutionOutcome::RestartRequired
                ),
                dry_run: false,
                actions: vec![ApplicationUninstallActionResult {
                    component_id: component.component_id.clone(),
                    kind: component.kind,
                    status: ApplicationUninstallActionStatus::Completed,
                    reason: None,
                    expected_bytes: component.bytes,
                    released_bytes: component.bytes,
                }],
                history_saved: false,
            }
        }
        Ok(windows::ApplicationUninstallExecution::Detached) => preflight::cancel_all(
            plan,
            Some(inspection.application_name),
            Some(ApplicationUninstallActionReason::ExternalUninstallerContinuing),
        ),
        Ok(windows::ApplicationUninstallExecution::Cancelled) => {
            log::info!(
                "application_uninstall_native_execution_result_cancelled reason=native_user_cancelled"
            );
            let mut result = preflight::cancel_all(plan, Some(inspection.application_name), None);
            result.dry_run = false;
            result
        }
        Err(reason) => {
            log::warn!(
                "application_uninstall_native_execution_result_failed application_id={} plan_id={} reason={}",
                plan.application_id, plan.plan_id, reason.stable_code()
            );
            let mut result = preflight::fail_all(plan, Some(inspection.application_name), reason);
            result.dry_run = false;
            result
        }
    }
}

#[cfg(any(target_os = "macos", windows))]
fn application_target_is_running(
    target: Option<&ResolvedApplicationCloseTarget>,
) -> Result<bool, String> {
    let target = target.ok_or_else(|| "application process identity is unavailable".to_string())?;
    ProcessSnapshot::capture().map(|processes| {
        !processes
            .matching_application_processes(&target.executable_names, &target.executable_paths)
            .is_empty()
    })
}

#[cfg(not(any(target_os = "macos", windows)))]
fn execute_preflighted(
    plan: &ApplicationUninstallPlan,
    preflighted: PreflightCandidate,
    _cancellation: Arc<AtomicBool>,
    _authorization_prompt: Option<&str>,
) -> ApplicationUninstallResult {
    let mut result = preflight::fail_all(
        plan,
        preflighted.result.application_name,
        ApplicationUninstallActionReason::UnsupportedExecutor,
    );
    result.dry_run = false;
    result
}

fn save_execution_history(
    operation_id: u64,
    started_at_ms: u64,
    finished_at_ms: u64,
    plan: &ApplicationUninstallPlan,
    result: &ApplicationUninstallResult,
    application: Option<&ApplicationUninstallCandidate>,
) -> bool {
    let Some(application) = application else {
        log::warn!("application_uninstall_history_identity_missing operation_id={operation_id}");
        return false;
    };
    execution_history_record(UninstallHistoryRecordInput {
        operation_id,
        started_at_ms,
        finished_at_ms,
        batch_id: &plan.plan_id,
        expected_bytes: plan.expected_bytes,
        plans: std::slice::from_ref(plan),
        results: std::slice::from_ref(result),
        candidates: std::slice::from_ref(application),
    })
    .is_some_and(|record| append_uninstall_history(operation_id, record))
}

fn save_batch_execution_history(
    operation_id: u64,
    started_at_ms: u64,
    finished_at_ms: u64,
    plan: &ApplicationUninstallBatchPlan,
    results: &[ApplicationUninstallResult],
    candidates: &[ApplicationUninstallCandidate],
) -> bool {
    execution_history_record(UninstallHistoryRecordInput {
        operation_id,
        started_at_ms,
        finished_at_ms,
        batch_id: &plan.batch_id,
        expected_bytes: plan.expected_bytes,
        plans: &plan.plans,
        results,
        candidates,
    })
    .is_some_and(|record| append_uninstall_history(operation_id, record))
}

struct UninstallHistoryRecordInput<'a> {
    operation_id: u64,
    started_at_ms: u64,
    finished_at_ms: u64,
    batch_id: &'a str,
    expected_bytes: u64,
    plans: &'a [ApplicationUninstallPlan],
    results: &'a [ApplicationUninstallResult],
    candidates: &'a [ApplicationUninstallCandidate],
}

fn execution_history_record(input: UninstallHistoryRecordInput<'_>) -> Option<OperationRecord> {
    let UninstallHistoryRecordInput {
        operation_id,
        started_at_ms,
        finished_at_ms,
        batch_id,
        expected_bytes,
        plans,
        results,
        candidates,
    } = input;
    if plans.len() != results.len() {
        log::warn!(
            "application_uninstall_history_result_count_mismatch operation_id={} plan_count={} result_count={}",
            operation_id,
            plans.len(),
            results.len()
        );
        return None;
    }
    let applications = plans
        .iter()
        .zip(results)
        .map(|(application_plan, result)| {
            candidates
                .iter()
                .find(|candidate| candidate.application_id == application_plan.application_id)
                .map(|candidate| application_history_details(application_plan, result, candidate))
        })
        .collect::<Option<Vec<_>>>();
    let Some(applications) = applications.filter(|applications| !applications.is_empty()) else {
        log::warn!("application_uninstall_history_identity_missing operation_id={operation_id}");
        return None;
    };
    let released_bytes_is_estimate = results
        .iter()
        .any(|result| result.released_bytes_is_estimate);
    let affected_application_count = results
        .iter()
        .filter(|result| result.affected_item_count > 0)
        .count() as u64;
    let failed_application_count = results
        .iter()
        .filter(|result| result.failed_item_count > 0)
        .count() as u64;
    let released_bytes = results.iter().fold(0_u64, |total, result| {
        total.saturating_add(result.released_bytes)
    });
    let restart_required = results.iter().any(|result| result.restart_required);
    let record = OperationRecord {
        schema_version: OPERATION_RECORD_SCHEMA_VERSION,
        operation_id: format!("application-uninstall-{operation_id}-{finished_at_ms}"),
        category: OperationCategory::ApplicationUninstall,
        started_at_ms,
        finished_at_ms,
        outcome: if results.iter().any(|result| {
            result
                .actions
                .iter()
                .any(|action| action.status == ApplicationUninstallActionStatus::Cancelled)
        }) {
            OperationOutcome::Cancelled
        } else if failed_application_count == 0 {
            OperationOutcome::Completed
        } else {
            OperationOutcome::CompletedWithWarnings
        },
        dry_run: false,
        selected_item_count: applications.len() as u64,
        affected_item_count: affected_application_count,
        expected_bytes,
        released_bytes: (!released_bytes_is_estimate).then_some(released_bytes),
        released_bytes_is_estimate,
        failed_item_count: failed_application_count,
        details: OperationDetails::ApplicationUninstall(application_uninstall_details(
            batch_id.to_string(),
            restart_required,
            applications,
        )),
    };
    Some(record)
}

fn application_history_details(
    plan: &ApplicationUninstallPlan,
    result: &ApplicationUninstallResult,
    application: &ApplicationUninstallCandidate,
) -> ApplicationUninstallApplicationDetails {
    ApplicationUninstallApplicationDetails {
        restart_required: result.restart_required,
        plan_id: plan.plan_id.clone(),
        application_id: plan.application_id.clone(),
        application_name: application.name.clone(),
        application_identifier: application.primary_identifier.clone(),
        application_version: application.version.clone(),
        application_publisher: application.publisher.clone(),
        application_platform: application.platform,
        installer_kind: application.installer_kind,
        component_ids: plan
            .items
            .iter()
            .map(|item| item.component_id.clone())
            .collect(),
        actions: result.actions.clone(),
    }
}

fn application_uninstall_details(
    batch_id: String,
    restart_required: bool,
    applications: Vec<ApplicationUninstallApplicationDetails>,
) -> ApplicationUninstallOperationDetails {
    ApplicationUninstallOperationDetails {
        batch_id,
        applications,
        restart_required,
    }
}

fn append_uninstall_history(operation_id: u64, record: OperationRecord) -> bool {
    match HistoryService::append(record) {
        Ok(()) => true,
        Err(error) => {
            log::warn!(
                "application_uninstall_history_save_failed operation_id={} error_digest={}",
                operation_id,
                blake3::hash(error.diagnostic().as_bytes()).to_hex()
            );
            false
        }
    }
}

fn validate_batch_selections(
    selections: &[ApplicationUninstallBatchSelection],
) -> Result<(), String> {
    batch::validate_application_count(selections.len())?;
    let unique = selections
        .iter()
        .map(|selection| selection.application_id.trim())
        .collect::<HashSet<_>>();
    if unique.len() != selections.len()
        || selections
            .iter()
            .any(|selection| selection.application_id.trim() != selection.application_id)
        || unique
            .iter()
            .any(|application_id| application_id.is_empty())
        || selections.iter().any(|selection| {
            selection.component_ids.is_empty()
                || selection.component_ids.iter().any(|component_id| {
                    component_id.trim() != component_id || component_id.is_empty()
                })
                || selection.component_ids.iter().collect::<HashSet<_>>().len()
                    != selection.component_ids.len()
        })
    {
        return Err("application uninstall batch selection is invalid".to_string());
    }
    Ok(())
}

fn aggregate_batch_result(
    plan: &ApplicationUninstallBatchPlan,
    dry_run: bool,
    results: Vec<ApplicationUninstallResult>,
) -> ApplicationUninstallBatchResult {
    ApplicationUninstallBatchResult {
        batch_id: plan.batch_id.clone(),
        expected_bytes: plan.expected_bytes,
        previewed_bytes: results.iter().fold(0_u64, |total, result| {
            total.saturating_add(result.previewed_bytes)
        }),
        released_bytes: results.iter().fold(0_u64, |total, result| {
            total.saturating_add(result.released_bytes)
        }),
        selected_application_count: results.len() as u64,
        previewed_application_count: results
            .iter()
            .filter(|result| result.previewed_item_count > 0)
            .count() as u64,
        affected_application_count: results
            .iter()
            .filter(|result| result.affected_item_count > 0)
            .count() as u64,
        failed_application_count: results
            .iter()
            .filter(|result| result.failed_item_count > 0)
            .count() as u64,
        previewed_item_count: results.iter().fold(0_u64, |total, result| {
            total.saturating_add(result.previewed_item_count)
        }),
        affected_item_count: results.iter().fold(0_u64, |total, result| {
            total.saturating_add(result.affected_item_count)
        }),
        failed_item_count: results.iter().fold(0_u64, |total, result| {
            total.saturating_add(result.failed_item_count)
        }),
        released_bytes_is_estimate: results
            .iter()
            .any(|result| result.released_bytes_is_estimate),
        restart_required: results.iter().any(|result| result.restart_required),
        dry_run,
        results,
    }
}

#[cfg(target_os = "macos")]
fn inspect_candidate(
    candidate: &ApplicationUninstallCandidate,
    catalog_revision: &str,
    started: Instant,
) -> Result<ApplicationUninstallInspection, String> {
    macos::inspect_candidate(candidate, catalog_revision, started)
}

#[cfg(windows)]
fn inspect_candidate(
    candidate: &ApplicationUninstallCandidate,
    catalog_revision: &str,
    started: Instant,
) -> Result<ApplicationUninstallInspection, String> {
    windows::inspect_candidate(candidate, catalog_revision, started)
}

#[cfg(not(any(target_os = "macos", windows)))]
fn inspect_candidate(
    _candidate: &ApplicationUninstallCandidate,
    _catalog_revision: &str,
    _started: Instant,
) -> Result<ApplicationUninstallInspection, String> {
    Err("application uninstall inspection is not supported on this system".to_string())
}

fn scan_without_guard(
    operation_id: u64,
    include_component_summaries: bool,
    progress_sink: Option<Box<dyn ProgressSink>>,
    cancellation: Arc<AtomicBool>,
) -> CoreResult<ApplicationUninstallScanResult> {
    let started = Instant::now();
    let progress = progress_sink.map(|sink| ProgressTracker::from_sink(operation_id, sink, 0));
    ensure_scan_not_cancelled(&cancellation)?;
    if let Some(progress) = &progress {
        progress.emit(
            TraversalStage::DiscoveringApplications,
            application_catalog_source(),
        );
    }
    let inventory_started = Instant::now();
    let platform_cancellation_flag = Arc::clone(&cancellation);
    let platform_cancellation =
        PlatformCancellation::new(move || platform_cancellation_flag.load(Ordering::Relaxed));
    let (mut context, mut revision_before) =
        ScanContext::capture_with_revision_and_cancellation(&platform_cancellation);
    let mut inventory_elapsed_ms = inventory_started.elapsed().as_millis();
    ensure_scan_not_cancelled(&cancellation)?;
    if let Some(progress) = &progress {
        progress.emit(TraversalStage::CheckingProcesses, std::path::Path::new(""));
    }
    let process_snapshot_started = Instant::now();
    let mut processes = ProcessSnapshot::capture_with_cancellation(&platform_cancellation);
    let mut process_snapshot_elapsed_ms = process_snapshot_started.elapsed().as_millis();
    ensure_scan_not_cancelled(&cancellation)?;
    let mut revision_after = current_platform()
        .system_inventory_revision_with_cancellation(&platform_cancellation)
        .ok();
    ensure_scan_not_cancelled(&cancellation)?;

    // A package manager or the operating system can update inventory evidence
    // while the first cold scan is reading it. Retry that specific race once
    // instead of publishing an empty, incomplete catalog that looks like the
    // machine has no applications. A second mismatch still fails closed via
    // `inventory_complete` below and remains visible to the adapter.
    if should_retry_changed_inventory(
        context.inventory.application_inventory_complete(),
        processes.is_ok(),
        revision_before.as_deref(),
        revision_after.as_deref(),
    ) {
        log::info!(
            "application_uninstall_inventory_changed_during_scan operation_id={operation_id} action=retry_once"
        );
        if let Some(progress) = &progress {
            progress.emit(
                TraversalStage::DiscoveringApplications,
                application_catalog_source(),
            );
        }
        let retry_inventory_started = Instant::now();
        (context, revision_before) =
            ScanContext::capture_with_revision_and_cancellation(&platform_cancellation);
        inventory_elapsed_ms =
            inventory_elapsed_ms.saturating_add(retry_inventory_started.elapsed().as_millis());
        ensure_scan_not_cancelled(&cancellation)?;
        if let Some(progress) = &progress {
            progress.emit(TraversalStage::CheckingProcesses, std::path::Path::new(""));
        }
        let retry_process_started = Instant::now();
        processes = ProcessSnapshot::capture_with_cancellation(&platform_cancellation);
        process_snapshot_elapsed_ms =
            process_snapshot_elapsed_ms.saturating_add(retry_process_started.elapsed().as_millis());
        ensure_scan_not_cancelled(&cancellation)?;
        revision_after = current_platform()
            .system_inventory_revision_with_cancellation(&platform_cancellation)
            .ok();
        ensure_scan_not_cancelled(&cancellation)?;
    }
    if let Some(progress) = &progress {
        progress.emit(
            TraversalStage::ValidatingApplications,
            application_catalog_source(),
        );
    }
    let candidate_build_started = Instant::now();
    let catalog_stable = revision_before.is_some() && revision_before == revision_after;
    let process_snapshot_complete = processes.is_ok();
    let inventory_complete = context.inventory.application_inventory_complete()
        && catalog_stable
        && process_snapshot_complete;
    let catalog_actionable = catalog_is_actionable(
        context.inventory.application_inventory_complete(),
        catalog_stable,
        process_snapshot_complete,
    );

    // A partial macOS inventory can still contain verified application
    // bundles. Exclude unreadable bundles while preserving the normal
    // uninstall behavior of every candidate whose identity can be verified.
    let installed_applications = context.inventory.installed_applications();
    let self_excluded_count = installed_applications
        .iter()
        .filter(|application| is_current_application(application))
        .count() as u64;
    let discovered = discovered_candidates(installed_applications, processes.as_ref().ok());
    let hidden_count = self_excluded_count.saturating_add(
        discovered
            .iter()
            .filter(|candidate| !is_visible_candidate(candidate))
            .count() as u64,
    );
    let mut candidates = discovered
        .into_iter()
        .filter(is_visible_candidate)
        .collect::<Vec<_>>();
    #[cfg(windows)]
    let observation_stats = super::windows_observations::annotate(&mut candidates, &cancellation)
        .map_err(map_scan_error)?;
    #[cfg(not(windows))]
    let observation_stats = (0_u64, 0_u64);
    candidates.sort_by(|left, right| {
        left.name
            .to_ascii_lowercase()
            .cmp(&right.name.to_ascii_lowercase())
            .then_with(|| left.application_id.cmp(&right.application_id))
    });
    let candidate_build_elapsed_ms = candidate_build_started.elapsed().as_millis();
    if let Some(progress) = &progress {
        progress.set_total_steps(candidates.len() as u64);
    }
    let mut last_progress_path = PathBuf::new();
    let component_summary_started = Instant::now();
    #[cfg(target_os = "macos")]
    let mut summary_metrics = macos::ComponentSummaryMetrics::default();
    if include_component_summaries {
        for candidate in &mut candidates {
            ensure_scan_not_cancelled(&cancellation)?;
            last_progress_path = candidate
                .application_path
                .as_deref()
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from(&candidate.name));
            if let Some(progress) = &progress {
                progress
                    .visit_directory(TraversalStage::InspectingApplications, &last_progress_path);
            }
            #[cfg(target_os = "macos")]
            let (components, associated_data_complete) = macos::summarize_candidate(
                candidate,
                &cancellation,
                progress.as_ref(),
                &mut summary_metrics,
            )
            .map_err(map_scan_error)?;
            #[cfg(windows)]
            let (components, associated_data_complete) = windows::summarize_candidate(candidate);
            #[cfg(not(any(target_os = "macos", windows)))]
            let (components, associated_data_complete) = (Vec::new(), false);
            if candidate.capability.supports_execution() && !has_primary_component(&components) {
                candidate.capability = ApplicationUninstallCapability::ViewOnly;
            }
            candidate.total_bytes = components.iter().fold(0_u64, |total, component| {
                total.saturating_add(component.bytes)
            });
            if candidate.total_bytes == 0 {
                // Windows uninstall registrations often expose a useful
                // logical size even when MangoDisk cannot safely enumerate or
                // uninstall the installer payload. Preserve that platform
                // estimate for catalog presentation instead of showing 0 B.
                candidate.total_bytes = candidate.estimated_bytes;
            }
            candidate.default_selected_bytes = components
                .iter()
                .filter(|component| component.default_selected)
                .fold(0_u64, |total, component| {
                    total.saturating_add(component.bytes)
                });
            candidate.components = components;
            candidate.associated_data_complete = associated_data_complete;
            if let Some(progress) = &progress {
                progress.complete_step(
                    TraversalStage::InspectingApplications,
                    &last_progress_path,
                    candidate.total_bytes,
                );
            }
        }
    }
    if let Some(progress) = &progress {
        progress.finish(TraversalStage::InspectingApplications, &last_progress_path);
    }
    ensure_scan_not_cancelled(&cancellation)?;
    let component_summary_elapsed_ms = component_summary_started.elapsed().as_millis();
    let ready_count = candidates
        .iter()
        .filter(|candidate| candidate.capability.supports_execution())
        .count() as u64;
    let blocked_count = candidates.len() as u64 - ready_count;

    super::system_classification::log_catalog(
        operation_id,
        &candidates,
        include_component_summaries,
    );

    // Summaries alone cannot explain one user's disabled row. Emit the catalog decision with
    // the same redacted reference used by inventory logs, including cached inventories on later scans.
    for candidate in candidates
        .iter()
        .filter(|candidate| !candidate.capability.supports_execution())
    {
        log::info!(
            "application_uninstall_candidate_blocked operation_id={} application_id={} capability={:?} record_state={:?} reason={} running_process_count={} has_application_location={} known_executable_count={}",
            operation_id, candidate.application_id, candidate.capability, candidate.record_state,
            candidate.uninstall_diagnostic.map_or("none", |reason| reason.stable_code()),
            candidate.running_processes.len(), candidate.application_path.is_some(), candidate.executable_paths.len()
        );
    }

    log::info!(
            "application_uninstall_catalog_ready operation_id={} candidate_count={} ready_count={} blocked_count={} hidden_count={} self_excluded_count={} catalog_actionable={} inventory_complete={} component_summaries={} inventory_elapsed_ms={} process_snapshot_elapsed_ms={} candidate_build_elapsed_ms={} component_summary_elapsed_ms={} elapsed_ms={}",
            operation_id,
            candidates.len(),
            ready_count,
            blocked_count,
            hidden_count,
            self_excluded_count,
            catalog_actionable,
            inventory_complete,
            include_component_summaries,
            inventory_elapsed_ms,
            process_snapshot_elapsed_ms,
            candidate_build_elapsed_ms,
            component_summary_elapsed_ms,
            started.elapsed().as_millis()
        );
    #[cfg(target_os = "macos")]
    log::info!(
        "application_uninstall_component_summary_metrics operation_id={} native_component_count={} portable_fallback_count={} spotlight_size_hit_count={} spotlight_size_fallback_count={} association_tree_count={} measured_file_count={} measured_bytes={} incomplete_component_count={}",
        operation_id,
        summary_metrics.native_component_count,
        summary_metrics.portable_fallback_count,
        summary_metrics.spotlight_size_hit_count,
        summary_metrics.spotlight_size_fallback_count,
        summary_metrics.association_tree_count,
        summary_metrics.measured_file_count,
        summary_metrics.measured_bytes,
        summary_metrics.incomplete_component_count,
    );
    Ok(ApplicationUninstallScanResult {
        schema_version: APPLICATION_UNINSTALL_SCAN_SCHEMA_VERSION,
        scanned_at_ms: now_ms(),
        supported: true,
        execution_supported: cfg!(any(target_os = "macos", windows)),
        catalog_actionable,
        inventory_complete,
        catalog_revision: revision_after,
        candidates,
        ready_count,
        blocked_count,
        hidden_count,
        #[cfg(windows)]
        related_directory_count: observation_stats.directory_count,
        #[cfg(not(windows))]
        related_directory_count: observation_stats.0,
        #[cfg(windows)]
        related_path_scan_elapsed_ms: observation_stats.elapsed_ms,
        #[cfg(not(windows))]
        related_path_scan_elapsed_ms: observation_stats.1,
        elapsed_ms: started.elapsed().as_millis() as u64,
    })
}

fn ensure_scan_not_cancelled(cancellation: &AtomicBool) -> CoreResult<()> {
    if cancellation.load(Ordering::Relaxed) {
        Err(CoreError::operation_cancelled())
    } else {
        Ok(())
    }
}

fn should_retry_changed_inventory(
    inventory_complete: bool,
    processes_complete: bool,
    revision_before: Option<&str>,
    revision_after: Option<&str>,
) -> bool {
    inventory_complete
        && processes_complete
        && revision_before
            .zip(revision_after)
            .is_some_and(|(before, after)| before != after)
}

fn catalog_is_actionable(
    application_inventory_complete: bool,
    catalog_stable: bool,
    process_snapshot_complete: bool,
) -> bool {
    #[cfg(target_os = "macos")]
    {
        let _ = application_inventory_complete;
        catalog_stable && process_snapshot_complete
    }
    #[cfg(not(target_os = "macos"))]
    {
        application_inventory_complete && catalog_stable && process_snapshot_complete
    }
}

fn map_scan_error(error: String) -> CoreError {
    if error == OPERATION_CANCELLED_ERROR {
        CoreError::operation_cancelled()
    } else {
        CoreError::operation_failed(error)
    }
}

#[cfg(windows)]
fn application_catalog_source() -> &'static std::path::Path {
    std::path::Path::new(r"HKCU / HKLM\Software\Microsoft\Windows\CurrentVersion\Uninstall")
}

#[cfg(target_os = "macos")]
fn application_catalog_source() -> &'static std::path::Path {
    std::path::Path::new("/Applications")
}

#[cfg(not(any(target_os = "macos", windows)))]
fn application_catalog_source() -> &'static std::path::Path {
    std::path::Path::new("")
}

fn has_primary_component(components: &[ApplicationUninstallComponentSummary]) -> bool {
    components.iter().any(|component| {
        matches!(
            component.kind,
            ApplicationUninstallComponentKind::ApplicationBinary
                | ApplicationUninstallComponentKind::NativeInstaller
        )
    })
}

#[cfg(target_os = "macos")]
fn is_visible_candidate(candidate: &ApplicationUninstallCandidate) -> bool {
    matches!(
        candidate.capability,
        ApplicationUninstallCapability::Ready
            | ApplicationUninstallCapability::ApplicationRunning
            | ApplicationUninstallCapability::RequiresElevation
    )
}

#[cfg(not(target_os = "macos"))]
fn is_visible_candidate(_candidate: &ApplicationUninstallCandidate) -> bool {
    true
}

fn candidate(
    application: &InstalledApplication,
    processes: &ProcessSnapshot,
) -> ApplicationUninstallCandidate {
    let running_processes = processes.matching_application_processes(
        &[
            application.name.clone(),
            application.primary_identifier.clone(),
        ],
        &application.executable_paths,
    );
    let application_path = application
        .bundle_path
        .as_deref()
        .filter(|path| path.exists())
        .or_else(|| {
            application
                .executable_paths
                .iter()
                .map(PathBuf::as_path)
                .find(|path| path.exists())
        })
        .map(display_path);
    #[cfg(windows)]
    let application_path = if matches!(
        application.uninstall_registration.as_ref(),
        Some(ApplicationUninstallRegistration::WindowsAppx { .. })
    ) {
        // Package roots live under the access-controlled WindowsApps tree.
        // Explorer prompts for permanent access instead of safely revealing
        // the app, so AppX entries must not expose a misleading folder action.
        None
    } else {
        application_path
    };

    ApplicationUninstallCandidate {
        system_kind: super::system_classification::classify(application),
        application_id: application_id(application),
        primary_identifier: application.primary_identifier.clone(),
        source_identities: application
            .source_identities
            .iter()
            .map(|identity| ApplicationUninstallSourceIdentity {
                source: match identity.source {
                    ApplicationInventorySource::MacosBundle => {
                        ApplicationUninstallInventorySource::MacosBundle
                    }
                    ApplicationInventorySource::WindowsRegistry => {
                        ApplicationUninstallInventorySource::WindowsRegistry
                    }
                    ApplicationInventorySource::WindowsMsi => {
                        ApplicationUninstallInventorySource::WindowsMsi
                    }
                    ApplicationInventorySource::WindowsAppx => {
                        ApplicationUninstallInventorySource::WindowsAppx
                    }
                    ApplicationInventorySource::Winget => {
                        ApplicationUninstallInventorySource::Winget
                    }
                    ApplicationInventorySource::Steam => ApplicationUninstallInventorySource::Steam,
                    ApplicationInventorySource::Scoop => ApplicationUninstallInventorySource::Scoop,
                    ApplicationInventorySource::Chocolatey => {
                        ApplicationUninstallInventorySource::Chocolatey
                    }
                },
                identifier: identity.identifier.clone(),
            })
            .collect(),
        name: application.name.clone(),
        version: application.version.clone(),
        publisher: application.publisher.clone(),
        estimated_bytes: application.estimated_bytes,
        last_used_at_ms: application.last_used_at_ms,
        installed_at_ms: application.installed_at_ms,
        platform: platform_kind(),
        installer_kind: application
            .uninstall_registration
            .as_ref()
            .map(|registration| match registration {
                ApplicationUninstallRegistration::WindowsMsi { .. } => {
                    ApplicationUninstallInstallerKind::WindowsMsi
                }
                ApplicationUninstallRegistration::WindowsAppx { .. } => {
                    ApplicationUninstallInstallerKind::WindowsAppx
                }
                ApplicationUninstallRegistration::WindowsScoop { .. } => {
                    ApplicationUninstallInstallerKind::WindowsScoop
                }
                ApplicationUninstallRegistration::WindowsChocolatey { .. } => {
                    ApplicationUninstallInstallerKind::WindowsChocolatey
                }
                ApplicationUninstallRegistration::WindowsRegistered { .. } => {
                    ApplicationUninstallInstallerKind::WindowsRegistered
                }
            }),
        execution_mode: application
            .uninstall_registration
            .as_ref()
            .map(|registration| match registration {
                ApplicationUninstallRegistration::WindowsMsi { .. }
                | ApplicationUninstallRegistration::WindowsAppx { .. }
                | ApplicationUninstallRegistration::WindowsScoop { .. } => {
                    ApplicationUninstallExecutionMode::Silent
                }
                ApplicationUninstallRegistration::WindowsChocolatey { .. } => {
                    ApplicationUninstallExecutionMode::ExternalClient
                }
                ApplicationUninstallRegistration::WindowsRegistered { command_kind, .. } => {
                    match command_kind {
                        mangodisk_platform::WindowsRegisteredUninstallKind::WingetProduct => {
                            ApplicationUninstallExecutionMode::ExternalClient
                        }
                        mangodisk_platform::WindowsRegisteredUninstallKind::Executable
                        | mangodisk_platform::WindowsRegisteredUninstallKind::Rundll32
                        | mangodisk_platform::WindowsRegisteredUninstallKind::BatchScript
                        | mangodisk_platform::WindowsRegisteredUninstallKind::UserPowerShellScript => {
                            ApplicationUninstallExecutionMode::Interactive
                        }
                    }
                }
            }),
        capability: capability(application, &running_processes),
        record_state: if running_processes.is_empty() {
            record_state(application)
        } else {
            ApplicationUninstallRecordState::Installed
        },
        uninstall_diagnostic: application.uninstall_diagnostic
            .filter(|_| application.uninstall_registration.is_none()),
        icon_path: application
            .icon_path
            .as_ref()
            .map(|path| display_path(path)),
        application_path,
        possible_related_paths: Vec::new(),
        running_processes,
        executable_paths: application.executable_paths.clone(),
        total_bytes: 0,
        default_selected_bytes: 0,
        associated_data_complete: false,
        components: Vec::new(),
        #[cfg(windows)]
        uninstall_registration: application.uninstall_registration.clone(),
    }
}

fn discovered_candidates(
    applications: &[InstalledApplication],
    processes: Option<&ProcessSnapshot>,
) -> Vec<ApplicationUninstallCandidate> {
    let Some(processes) = processes else {
        // Without a reliable process snapshot, Core cannot distinguish a
        // running application from a safe-to-review application. Fail closed
        // instead of publishing candidates with an incorrect capability.
        return Vec::new();
    };
    applications
        .iter()
        .filter(|application| !is_current_application(application))
        .map(|application| candidate(application, processes))
        .collect()
}

fn is_current_application(application: &InstalledApplication) -> bool {
    if application
        .identifiers
        .iter()
        .chain(std::iter::once(&application.primary_identifier))
        .any(|identifier| identifier.eq_ignore_ascii_case(crate::APPLICATION_IDENTIFIER))
    {
        return true;
    }

    #[cfg(windows)]
    {
        application
            .name
            .eq_ignore_ascii_case(CURRENT_APPLICATION_NAME)
    }

    #[cfg(not(windows))]
    {
        false
    }
}

#[cfg(windows)]
fn record_state(application: &InstalledApplication) -> ApplicationUninstallRecordState {
    let Some(bundle_path) = application.bundle_path.as_deref() else {
        return ApplicationUninstallRecordState::Installed;
    };
    if application.uninstall_registration.is_some()
        || application.uninstall_diagnostic.is_some_and(|reason| {
            !matches!(
                reason,
                mangodisk_platform::ApplicationUninstallDiagnostic::CommandMissing
                    | mangodisk_platform::ApplicationUninstallDiagnostic::ExecutableMissing
            )
        })
        || !registered_path_is_missing(bundle_path)
        || application.executable_paths.is_empty()
        || application
            .executable_paths
            .iter()
            .any(|path| !registered_path_is_missing(path))
    {
        return ApplicationUninstallRecordState::Installed;
    }
    ApplicationUninstallRecordState::OrphanedRegistration
}

#[cfg(any(windows, test))]
use mangodisk_platform::registered_application_path_is_missing as registered_path_is_missing;

#[cfg(not(windows))]
fn record_state(_application: &InstalledApplication) -> ApplicationUninstallRecordState {
    ApplicationUninstallRecordState::Installed
}

fn application_id(application: &InstalledApplication) -> String {
    mangodisk_platform::application_uninstall_diagnostic_id(&application.catalog_identifier)
}

#[cfg(target_os = "macos")]
fn capability(
    application: &InstalledApplication,
    running_processes: &[String],
) -> ApplicationUninstallCapability {
    let Some(path) = application.bundle_path.as_deref() else {
        return ApplicationUninstallCapability::ProtectedApplication;
    };
    if path.starts_with("/System/Applications") || path.starts_with("/System/Library") {
        return ApplicationUninstallCapability::ProtectedApplication;
    }
    if !path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("app"))
    {
        return ApplicationUninstallCapability::ProtectedApplication;
    }
    if !running_processes.is_empty() {
        return ApplicationUninstallCapability::ApplicationRunning;
    }
    if !macos_bundle_is_deletable_without_elevation(path) {
        return if macos_privileged_application_removal_supported(path) {
            ApplicationUninstallCapability::RequiresElevation
        } else {
            ApplicationUninstallCapability::ViewOnly
        };
    }
    ApplicationUninstallCapability::Ready
}

#[cfg(target_os = "macos")]
fn macos_bundle_is_deletable_without_elevation(path: &std::path::Path) -> bool {
    use std::{ffi::CString, fs, os::unix::ffi::OsStrExt};

    let writable_directory = |directory: &std::path::Path| {
        CString::new(directory.as_os_str().as_bytes()).is_ok_and(|directory| unsafe {
            libc::access(directory.as_ptr(), libc::W_OK | libc::X_OK) == 0
        })
    };

    let Some(parent) = path.parent() else {
        return false;
    };
    if !writable_directory(parent) {
        return false;
    }

    // Permanent deletion first renames the bundle into staging and then
    // removes the staged tree. Removing every child requires write and search
    // access to each real directory in that tree. A writable bundle root alone
    // is insufficient when a nested root-owned directory is protected.
    let mut pending_directories = vec![path.to_path_buf()];
    while let Some(directory) = pending_directories.pop() {
        if !writable_directory(&directory) {
            return false;
        }
        let Ok(entries) = fs::read_dir(&directory) else {
            return false;
        };
        for entry in entries {
            let Ok(entry) = entry else {
                return false;
            };
            let Ok(file_type) = entry.file_type() else {
                return false;
            };
            // A symlink is removed from its containing directory. Following it
            // could cross the application boundary and misclassify permissions.
            if file_type.is_dir() && !file_type.is_symlink() {
                pending_directories.push(entry.path());
            }
        }
    }
    true
}

#[cfg(windows)]
fn capability(
    application: &InstalledApplication,
    running_processes: &[String],
) -> ApplicationUninstallCapability {
    let Some(registration) = application.uninstall_registration.as_ref() else {
        return ApplicationUninstallCapability::ViewOnly;
    };
    if !running_processes.is_empty() {
        return ApplicationUninstallCapability::ApplicationRunning;
    }
    if matches!(
        registration,
        ApplicationUninstallRegistration::WindowsMsi {
            scope: ApplicationInstallScope::Machine,
            ..
        } | ApplicationUninstallRegistration::WindowsChocolatey { .. }
    ) {
        return ApplicationUninstallCapability::RequiresElevation;
    }
    // The cold inventory validates every registration before exposing it, and scan_without_guard
    // rejects the complete candidate set unless the source revision is identical before and after
    // construction. Re-querying each AppX registration here previously launched one PowerShell
    // process per package. Planning and preflight still revalidate the selected registration at
    // the mutation boundary, so using the stable inventory fact for list capability removes only
    // redundant catalog work and does not weaken uninstall safety.
    ApplicationUninstallCapability::Ready
}

#[cfg(not(any(target_os = "macos", windows)))]
fn capability(
    _application: &InstalledApplication,
    _running_processes: &[String],
) -> ApplicationUninstallCapability {
    ApplicationUninstallCapability::ViewOnly
}

#[cfg(target_os = "macos")]
const fn platform_kind() -> ApplicationUninstallPlatform {
    ApplicationUninstallPlatform::MacosBundle
}

#[cfg(windows)]
const fn platform_kind() -> ApplicationUninstallPlatform {
    ApplicationUninstallPlatform::WindowsRegistry
}

#[cfg(not(any(target_os = "macos", windows)))]
const fn platform_kind() -> ApplicationUninstallPlatform {
    ApplicationUninstallPlatform::WindowsRegistry
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn missing_registration_path_requires_an_absolute_absent_path_on_an_online_root() {
        let directory = tempfile::tempdir().expect("fixture directory should be created");
        assert!(!registered_path_is_missing(directory.path()));
        let canonical = directory
            .path()
            .canonicalize()
            .expect("fixture should resolve");
        assert!(registered_path_is_missing(&canonical.join("absent.exe")));
        assert!(!registered_path_is_missing(std::path::Path::new(
            "relative/absent.exe"
        )));
    }

    #[cfg(unix)]
    #[test]
    fn dangling_registration_links_are_not_evidence_of_a_removed_application() {
        let directory = tempfile::tempdir().expect("fixture directory should be created");
        let link = directory.path().join("application");
        std::os::unix::fs::symlink(directory.path().join("missing"), &link)
            .expect("fixture link should be created");
        assert!(!registered_path_is_missing(&link));
        assert!(!registered_path_is_missing(&link.join("missing-child.exe")));
    }

    #[test]
    fn catalog_exposes_the_same_redacted_reference_and_typed_reason_as_inventory() {
        let mut application = fixture_application();
        application.uninstall_diagnostic =
            Some(mangodisk_platform::ApplicationUninstallDiagnostic::ExecutableAccessDenied);
        let item = candidate(&application, &ProcessSnapshot::default());
        let json = serde_json::to_value(&item).expect("candidate should serialize");
        assert_eq!(json["uninstallDiagnostic"], "executableAccessDenied");
        assert_eq!(
            json["applicationId"],
            mangodisk_platform::application_uninstall_diagnostic_id(
                &application.catalog_identifier
            )
        );
        assert!(!item
            .application_id
            .contains(&application.primary_identifier));
    }

    #[test]
    fn verified_record_removal_updates_only_the_selected_catalog_row() {
        let mut removed = candidate(&fixture_application(), &ProcessSnapshot::default());
        removed.application_id = "removed".to_string();
        removed.capability = ApplicationUninstallCapability::ViewOnly;
        let mut remaining = removed.clone();
        remaining.application_id = "remaining".to_string();
        remaining.capability = ApplicationUninstallCapability::Ready;
        let mut scan = ApplicationUninstallScanResult {
            schema_version: APPLICATION_UNINSTALL_SCAN_SCHEMA_VERSION,
            scanned_at_ms: 123,
            supported: true,
            execution_supported: true,
            catalog_actionable: true,
            inventory_complete: true,
            catalog_revision: Some("original-revision".to_string()),
            candidates: vec![removed, remaining],
            ready_count: 1,
            blocked_count: 1,
            hidden_count: 0,
            related_directory_count: 0,
            related_path_scan_elapsed_ms: 0,
            elapsed_ms: 1,
        };
        assert_eq!(scan.remove_record("removed"), 1);
        assert_eq!(scan.candidates.len(), 1);
        assert_eq!(scan.candidates[0].application_id, "remaining");
        assert_eq!((scan.ready_count, scan.blocked_count), (1, 0));
        assert_eq!(scan.scanned_at_ms, 123);
        assert_eq!(scan.catalog_revision.as_deref(), Some("original-revision"));
        assert_eq!(scan.remove_record("removed"), 0);
    }

    #[test]
    fn stable_inventory_retry_requires_two_different_complete_revisions() {
        assert!(should_retry_changed_inventory(
            true,
            true,
            Some("before"),
            Some("after")
        ));
        assert!(!should_retry_changed_inventory(
            true,
            true,
            Some("stable"),
            Some("stable")
        ));
        assert!(!should_retry_changed_inventory(
            false,
            true,
            Some("before"),
            Some("after")
        ));
        assert!(!should_retry_changed_inventory(
            true,
            false,
            Some("before"),
            Some("after")
        ));
        assert!(!should_retry_changed_inventory(
            true,
            true,
            None,
            Some("after")
        ));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn partial_macos_inventory_keeps_verified_candidates_actionable() {
        assert!(catalog_is_actionable(false, true, true));
        assert!(!catalog_is_actionable(false, false, true));
        assert!(!catalog_is_actionable(false, true, false));
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn partial_non_macos_inventory_remains_non_actionable() {
        assert!(!catalog_is_actionable(false, true, true));
        assert!(catalog_is_actionable(true, true, true));
    }

    fn fixture_application() -> InstalledApplication {
        InstalledApplication {
            #[cfg(windows)]
            system_signed: false,
            uninstall_diagnostic: None,
            catalog_identifier: "macos-bundle:/Applications/Example Editor.app".to_string(),
            source_identities: Vec::new(),
            primary_identifier: "com.example.Editor".to_string(),
            identifiers: vec![
                "Example Editor".to_string(),
                "com.example.Editor".to_string(),
            ],
            name: "Example Editor".to_string(),
            version: Some("1.0".to_string()),
            publisher: Some("Example".to_string()),
            estimated_bytes: 42_000,
            last_used_at_ms: Some(1_700_000_000_000),
            installed_at_ms: Some(1_690_000_000_000),
            icon_path: Some(PathBuf::from("/Applications/Example Editor.app")),
            bundle_path: Some(PathBuf::from("/Applications/Example Editor.app")),
            executable_paths: vec![PathBuf::from(
                "/Applications/Example Editor.app/Contents/MacOS/Example Editor",
            )],
            uninstall_registration: None,
        }
    }

    fn batch_result_fixture(
        application_id: &str,
        affected_item_count: u64,
        failed_item_count: u64,
        released_bytes: u64,
    ) -> ApplicationUninstallResult {
        ApplicationUninstallResult {
            plan_id: format!("plan-{application_id}"),
            application_id: application_id.to_string(),
            application_name: Some(application_id.to_string()),
            expected_bytes: 100,
            previewed_bytes: 0,
            released_bytes,
            previewed_item_count: 0,
            affected_item_count,
            failed_item_count,
            released_bytes_is_estimate: false,
            restart_required: false,
            dry_run: false,
            actions: Vec::new(),
            history_saved: affected_item_count > 0,
        }
    }

    fn history_candidate(application_id: &str, name: &str) -> ApplicationUninstallCandidate {
        let mut candidate = candidate(&fixture_application(), &ProcessSnapshot::default());
        candidate.application_id = application_id.to_string();
        candidate.primary_identifier = format!("identifier.{application_id}");
        candidate.name = name.to_string();
        candidate
    }

    #[test]
    fn close_target_prefers_exact_executable_paths_over_process_names() {
        let mut candidate = history_candidate("application-exact", "Exact Application");
        candidate.running_processes = vec!["SharedHelper".to_string()];
        candidate.executable_paths = vec![PathBuf::from(
            "/Applications/Exact.app/Contents/MacOS/SharedHelper",
        )];

        let target = close_target(&candidate);

        assert!(target.executable_names.is_empty());
        assert_eq!(target.executable_paths, candidate.executable_paths);
    }

    #[test]
    fn close_target_keeps_stable_names_when_no_process_is_running() {
        let mut candidate = history_candidate("application-name", "Example Application");
        candidate.executable_paths.clear();

        let target = close_target(&candidate);

        assert!(target.executable_paths.is_empty());
        assert!(target
            .executable_names
            .iter()
            .any(|name| name == "Example Application"));
        assert!(target
            .executable_names
            .iter()
            .any(|name| name == "identifier.application-name"));
    }

    #[test]
    fn close_result_updates_only_fully_stopped_catalog_targets() {
        let mut stopped = history_candidate("application-stopped", "Stopped Application");
        stopped.capability = ApplicationUninstallCapability::ApplicationRunning;
        stopped.running_processes = vec!["stopped-helper".to_string()];
        let mut remaining = history_candidate("application-remaining", "Remaining Application");
        remaining.capability = ApplicationUninstallCapability::ApplicationRunning;
        remaining.running_processes = vec!["remaining-helper".to_string()];
        let mut failed = history_candidate("application-failed", "Failed Application");
        failed.capability = ApplicationUninstallCapability::ApplicationRunning;
        failed.running_processes = vec!["failed-helper".to_string()];
        let mut scan = ApplicationUninstallScanResult {
            schema_version: APPLICATION_UNINSTALL_SCAN_SCHEMA_VERSION,
            scanned_at_ms: 1,
            supported: true,
            execution_supported: true,
            catalog_actionable: true,
            inventory_complete: true,
            catalog_revision: Some("revision-1".to_string()),
            candidates: vec![stopped, remaining, failed],
            ready_count: 0,
            blocked_count: 3,
            hidden_count: 0,
            related_directory_count: 0,
            related_path_scan_elapsed_ms: 0,
            elapsed_ms: 1,
        };
        let result = crate::ApplicationCloseBatchResult {
            mode: crate::ApplicationCloseMode::Graceful,
            matched_process_count: 2,
            requested_process_count: 2,
            remaining_process_count: 1,
            failed_target_count: 1,
            targets: vec![
                crate::ApplicationCloseTargetResult {
                    target_id: "application-stopped".to_string(),
                    status: crate::ApplicationCloseTargetStatus::Completed,
                    matched_process_count: 1,
                    requested_process_count: 1,
                    remaining_processes: Vec::new(),
                },
                crate::ApplicationCloseTargetResult {
                    target_id: "application-remaining".to_string(),
                    status: crate::ApplicationCloseTargetStatus::Completed,
                    matched_process_count: 1,
                    requested_process_count: 1,
                    remaining_processes: vec!["remaining-helper".to_string()],
                },
                crate::ApplicationCloseTargetResult {
                    target_id: "application-failed".to_string(),
                    status: crate::ApplicationCloseTargetStatus::Failed,
                    matched_process_count: 0,
                    requested_process_count: 0,
                    remaining_processes: Vec::new(),
                },
            ],
            elapsed_ms: 1,
        };

        refresh_catalog_after_close(&mut scan, &result);

        assert!(scan.candidates[0].running_processes.is_empty());
        assert_ne!(
            scan.candidates[0].capability,
            ApplicationUninstallCapability::ApplicationRunning
        );
        assert_eq!(
            scan.candidates[1].capability,
            ApplicationUninstallCapability::ApplicationRunning
        );
        assert_eq!(
            scan.candidates[1].running_processes,
            vec!["remaining-helper"]
        );
        assert_eq!(
            scan.candidates[2].capability,
            ApplicationUninstallCapability::ApplicationRunning
        );
        assert_eq!(scan.candidates[2].running_processes, vec!["failed-helper"]);
        assert_eq!(scan.ready_count + scan.blocked_count, 3);
    }

    #[test]
    fn uninstall_progress_tracks_serial_application_boundaries() {
        let mut snapshots = Vec::new();
        {
            let mut reporter =
                UninstallExecutionReporter::new(2, |progress| snapshots.push(progress));
            reporter.emit(ApplicationUninstallExecutionStage::Validating, None);

            let completed = batch_result_fixture("application-a", 1, 0, 100);
            reporter.emit(
                ApplicationUninstallExecutionStage::Uninstalling,
                Some(&completed.application_id),
            );
            reporter.record(&completed);
            reporter.emit(
                ApplicationUninstallExecutionStage::Uninstalling,
                Some(&completed.application_id),
            );

            let failed = batch_result_fixture("application-b", 0, 1, 0);
            reporter.emit(
                ApplicationUninstallExecutionStage::Uninstalling,
                Some(&failed.application_id),
            );
            reporter.record(&failed);
            reporter.emit(ApplicationUninstallExecutionStage::Finalizing, None);
        }

        assert_eq!(
            snapshots
                .iter()
                .map(|progress| progress.stage)
                .collect::<Vec<_>>(),
            vec![
                ApplicationUninstallExecutionStage::Validating,
                ApplicationUninstallExecutionStage::Uninstalling,
                ApplicationUninstallExecutionStage::Uninstalling,
                ApplicationUninstallExecutionStage::Uninstalling,
                ApplicationUninstallExecutionStage::Finalizing,
            ]
        );
        assert!(snapshots.iter().all(|progress| {
            progress.completed_application_count <= progress.total_application_count
        }));
        let final_snapshot = snapshots
            .last()
            .expect("final uninstall progress must be emitted");
        assert_eq!(final_snapshot.completed_application_count, 2);
        assert_eq!(final_snapshot.affected_application_count, 1);
        assert_eq!(final_snapshot.failed_application_count, 1);
        assert_eq!(final_snapshot.released_bytes, 100);
        assert_eq!(
            final_snapshot.completed_applications,
            vec![
                ApplicationUninstallExecutionItemResult {
                    application_id: "application-a".to_string(),
                    status: ApplicationUninstallExecutionItemStatus::Completed,
                    released_bytes: 100,
                },
                ApplicationUninstallExecutionItemResult {
                    application_id: "application-b".to_string(),
                    status: ApplicationUninstallExecutionItemStatus::Failed,
                    released_bytes: 0,
                },
            ]
        );
        let wire = serde_json::to_value(final_snapshot)
            .expect("execution progress should serialize for desktop events");
        assert_eq!(
            wire["completedApplications"][0]["applicationId"],
            "application-a"
        );
        assert_eq!(wire["completedApplications"][0]["status"], "completed");
    }

    #[test]
    fn application_identity_is_stable_when_alias_order_changes() {
        let mut application = fixture_application();
        let expected = application_id(&application);
        application.identifiers.reverse();
        assert_eq!(application_id(&application), expected);
    }

    #[test]
    fn application_identity_is_stable_when_inventory_sources_are_enriched() {
        let mut application = fixture_application();
        let expected = application_id(&application);
        application
            .source_identities
            .push(mangodisk_platform::ApplicationSourceIdentity {
                source: ApplicationInventorySource::Winget,
                identifier: "Example.Editor".to_string(),
            });
        application.identifiers.push("Example.Editor".to_string());
        assert_eq!(application_id(&application), expected);
    }

    #[test]
    fn valid_applications_remain_visible_when_inventory_is_incomplete() {
        let applications = vec![fixture_application()];

        let discovered = discovered_candidates(&applications, Some(&ProcessSnapshot::default()));

        assert_eq!(discovered.len(), 1);
        assert_eq!(discovered[0].primary_identifier, "com.example.Editor");
    }

    #[test]
    fn applications_are_hidden_when_process_detection_is_unavailable() {
        let applications = vec![fixture_application()];

        assert!(discovered_candidates(&applications, None).is_empty());
    }

    #[test]
    fn current_application_identifier_is_excluded_from_uninstall_candidates() {
        let mut application = fixture_application();
        application.primary_identifier = crate::APPLICATION_IDENTIFIER.to_string();
        application.identifiers = vec![crate::APPLICATION_IDENTIFIER.to_string()];

        let discovered = discovered_candidates(&[application], Some(&ProcessSnapshot::default()));

        assert!(discovered.is_empty());
    }

    #[cfg(windows)]
    #[test]
    fn windows_product_registration_is_excluded_from_uninstall_candidates() {
        let mut application = fixture_application();
        application.primary_identifier = "future-installer-identity".to_string();
        application.identifiers = vec!["future-installer-identity".to_string()];
        application.name = CURRENT_APPLICATION_NAME.to_string();
        application.publisher = Some("Future MangoDisk Publisher".to_string());
        application.source_identities.clear();

        let discovered = discovered_candidates(&[application], Some(&ProcessSnapshot::default()));

        assert!(discovered.is_empty());
    }

    #[test]
    fn ready_catalog_entry_requires_a_primary_uninstall_component() {
        let summary = |kind| ApplicationUninstallComponentSummary {
            component_id: "component".to_string(),
            kind,
            risk: super::super::models::ApplicationUninstallRisk::Rebuildable,
            path: None,
            bytes: 1,
            file_count: 1,
            default_selected: false,
        };

        assert!(!has_primary_component(&[summary(
            ApplicationUninstallComponentKind::Cache,
        )]));
        assert!(has_primary_component(&[summary(
            ApplicationUninstallComponentKind::ApplicationBinary,
        )]));
        assert!(has_primary_component(&[summary(
            ApplicationUninstallComponentKind::NativeInstaller,
        )]));
    }

    #[cfg(windows)]
    #[test]
    fn appx_candidate_does_not_expose_the_protected_package_directory() {
        let mut application = fixture_application();
        application.uninstall_registration = Some(ApplicationUninstallRegistration::WindowsAppx {
            package_family_name: "Example_123".to_string(),
            package_full_name: "Example_1.0.0.0_x64__123".to_string(),
            estimated_bytes: 42_000,
        });

        let candidate = candidate(&application, &ProcessSnapshot::default());

        assert!(candidate.application_path.is_none());
        assert_eq!(
            candidate.record_state,
            ApplicationUninstallRecordState::Installed
        );
        assert!(candidate.icon_path.is_some());
    }

    #[cfg(windows)]
    #[test]
    fn candidate_does_not_expose_a_stale_registered_directory() {
        let mut application = fixture_application();
        let missing = std::env::temp_dir().join(format!(
            "mangodisk-missing-application-{}",
            std::process::id()
        ));
        application.bundle_path = Some(missing.clone());
        application.executable_paths = vec![missing.join("application.exe")];

        let candidate = candidate(&application, &ProcessSnapshot::default());

        assert!(candidate.application_path.is_none());
        assert_eq!(
            candidate.record_state,
            ApplicationUninstallRecordState::OrphanedRegistration
        );
    }

    #[cfg(windows)]
    #[test]
    fn registered_executables_delegate_elevation_to_windows_shell() {
        let registration = |scope| ApplicationUninstallRegistration::WindowsRegistered {
            key_name: "Example".to_string(),
            scope,
            registry_view: mangodisk_platform::WindowsRegistryView::Registry64,
            command_kind: mangodisk_platform::WindowsRegisteredUninstallKind::Executable,
            command_digest: "a".repeat(64),
            estimated_bytes: 42_000,
        };
        let mut application = fixture_application();
        application.uninstall_registration = Some(registration(ApplicationInstallScope::Machine));
        assert_eq!(
            capability(&application, &[]),
            ApplicationUninstallCapability::Ready
        );

        application.uninstall_registration =
            Some(registration(ApplicationInstallScope::CurrentUser));
        assert_eq!(
            capability(&application, &[]),
            ApplicationUninstallCapability::Ready
        );
    }

    #[test]
    fn batch_result_keeps_successful_applications_when_another_fails() {
        let plans = vec![
            plan::create_reviewed_plan(
                "application-a".to_string(),
                "revision-1".to_string(),
                vec![ApplicationUninstallPlanItem {
                    component_id: "component-a".to_string(),
                    kind: crate::applications::uninstall::models::ApplicationUninstallComponentKind::ApplicationBinary,
                    expected_bytes: 100,
                    expected_file_count: 1,
                    expected_snapshot_fingerprint: "a".repeat(64),
                }],
            )
            .expect("first application plan should be valid"),
            plan::create_reviewed_plan(
                "application-b".to_string(),
                "revision-1".to_string(),
                vec![ApplicationUninstallPlanItem {
                    component_id: "component-b".to_string(),
                    kind: crate::applications::uninstall::models::ApplicationUninstallComponentKind::ApplicationBinary,
                    expected_bytes: 100,
                    expected_file_count: 1,
                    expected_snapshot_fingerprint: "b".repeat(64),
                }],
            )
            .expect("second application plan should be valid"),
        ];
        let batch = batch::create_plan("revision-1".to_string(), plans)
            .expect("batch plan should be valid");

        let results = vec![
            batch_result_fixture("application-a", 1, 0, 100),
            batch_result_fixture("application-b", 0, 1, 0),
        ];
        let history = execution_history_record(UninstallHistoryRecordInput {
            operation_id: 42,
            started_at_ms: 10,
            finished_at_ms: 25,
            batch_id: &batch.batch_id,
            expected_bytes: batch.expected_bytes,
            plans: &batch.plans,
            results: &results,
            candidates: &[
                history_candidate("application-a", "Application A"),
                history_candidate("application-b", "Application B"),
            ],
        })
        .expect("one batch history record should contain every selected application");
        assert_eq!(history.selected_item_count, 2);
        assert_eq!(history.affected_item_count, 1);
        assert_eq!(history.failed_item_count, 1);
        let OperationDetails::ApplicationUninstall(details) = history.details else {
            panic!("batch uninstall history must preserve application details");
        };
        assert_eq!(details.applications.len(), 2);
        assert_eq!(details.applications[0].application_name, "Application A");
        assert_eq!(details.applications[1].application_name, "Application B");

        let result = aggregate_batch_result(&batch, false, results);

        assert_eq!(result.selected_application_count, 2);
        assert_eq!(result.affected_application_count, 1);
        assert_eq!(result.failed_application_count, 1);
        assert_eq!(result.affected_item_count, 1);
        assert_eq!(result.failed_item_count, 1);
        assert_eq!(result.released_bytes, 100);
        assert_eq!(result.results.len(), 2);
    }

    #[test]
    fn cancelled_applications_mark_batch_history_without_counting_as_failures() {
        let plans = vec![
            plan::create_reviewed_plan(
                "application-a".to_string(),
                "revision-1".to_string(),
                vec![ApplicationUninstallPlanItem {
                    component_id: "component-a".to_string(),
                    kind: ApplicationUninstallComponentKind::ApplicationBinary,
                    expected_bytes: 100,
                    expected_file_count: 1,
                    expected_snapshot_fingerprint: "a".repeat(64),
                }],
            )
            .expect("first application plan should be valid"),
            plan::create_reviewed_plan(
                "application-b".to_string(),
                "revision-1".to_string(),
                vec![ApplicationUninstallPlanItem {
                    component_id: "component-b".to_string(),
                    kind: ApplicationUninstallComponentKind::ApplicationBinary,
                    expected_bytes: 100,
                    expected_file_count: 1,
                    expected_snapshot_fingerprint: "b".repeat(64),
                }],
            )
            .expect("second application plan should be valid"),
        ];
        let batch = batch::create_plan("revision-1".to_string(), plans)
            .expect("batch plan should be valid");
        let results = vec![
            batch_result_fixture("application-a", 1, 0, 100),
            preflight::cancel_all(&batch.plans[1], Some("Application B".to_string()), None),
        ];

        let history = execution_history_record(UninstallHistoryRecordInput {
            operation_id: 43,
            started_at_ms: 10,
            finished_at_ms: 25,
            batch_id: &batch.batch_id,
            expected_bytes: batch.expected_bytes,
            plans: &batch.plans,
            results: &results,
            candidates: &[
                history_candidate("application-a", "Application A"),
                history_candidate("application-b", "Application B"),
            ],
        })
        .expect("cancelled batch history should retain every selected application");
        let result = aggregate_batch_result(&batch, false, results);

        assert_eq!(history.outcome, OperationOutcome::Cancelled);
        assert_eq!(history.failed_item_count, 0);
        assert_eq!(result.selected_application_count, 2);
        assert_eq!(result.affected_application_count, 1);
        assert_eq!(result.failed_application_count, 0);
    }

    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "permanently uninstalls only the explicitly named disposable macOS bundle"]
    fn real_macos_bundle_fixture_completes_the_full_uninstall_workflow() {
        let application_name = std::env::var("MANGODISK_TEST_MACOS_UNINSTALL_NAME")
            .expect("set MANGODISK_TEST_MACOS_UNINSTALL_NAME to a disposable application bundle");
        assert!(
            !application_name.trim().is_empty(),
            "the disposable application name must not be empty"
        );
        let mut scan = ApplicationUninstallService::scan()
            .expect("the macOS application catalog should be available");
        let application_id = scan
            .candidates
            .iter()
            .find(|candidate| candidate.name == application_name)
            .map(|candidate| candidate.application_id.clone())
            .expect("the disposable application should be present");
        if scan
            .candidates
            .iter()
            .find(|candidate| candidate.application_id == application_id)
            .is_some_and(|candidate| {
                candidate.capability == ApplicationUninstallCapability::ApplicationRunning
            })
        {
            let close_result = ApplicationUninstallService::close_applications_from_catalog(
                ApplicationUninstallCloseRequest {
                    application_ids: vec![application_id.clone()],
                    mode: crate::ApplicationCloseMode::Force,
                },
                &mut scan,
            )
            .expect("the running disposable application should close");
            assert_eq!(close_result.remaining_process_count, 0);
        }
        let candidate = scan
            .candidates
            .iter()
            .find(|candidate| candidate.application_id == application_id)
            .expect("the disposable application should remain in the catalog snapshot");
        assert_eq!(candidate.capability, ApplicationUninstallCapability::Ready);
        let application_path = candidate
            .application_path
            .as_deref()
            .map(PathBuf::from)
            .expect("the disposable bundle path should be available");
        let component_ids = candidate
            .components
            .iter()
            .filter(|component| component.default_selected)
            .map(|component| component.component_id.clone())
            .collect::<Vec<_>>();
        let preparation = ApplicationUninstallService::prepare_batch_from_catalog(
            &[ApplicationUninstallBatchSelection {
                application_id: application_id.clone(),
                component_ids,
            }],
            &scan,
        )
        .expect("the disposable application should pass uninstall preflight");
        assert_eq!(preparation.preview.failed_application_count, 0);

        let result = ApplicationUninstallService::execute_batch(preparation.plan, false)
            .expect("the disposable application should uninstall");
        assert_eq!(result.affected_application_count, 1, "{result:#?}");
        assert_eq!(result.failed_application_count, 0, "{result:#?}");
        assert!(
            !application_path.exists(),
            "the disposable application bundle should be removed"
        );
        let refreshed = ApplicationUninstallService::scan()
            .expect("the macOS application catalog should refresh");
        assert!(
            refreshed
                .candidates
                .iter()
                .all(|candidate| candidate.application_id != application_id),
            "the removed application must disappear from the refreshed catalog"
        );
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "uninstalls only the explicitly named Scoop fixture"]
    fn real_scoop_fixture_completes_the_full_uninstall_workflow() {
        let package_name = std::env::var("MANGODISK_TEST_SCOOP_UNINSTALL_PACKAGE")
            .expect("set MANGODISK_TEST_SCOOP_UNINSTALL_PACKAGE to a disposable Scoop package");
        real_package_fixture_completes_the_full_uninstall_workflow(
            &package_name,
            ApplicationUninstallInventorySource::Scoop,
            ApplicationUninstallInstallerKind::WindowsScoop,
            ApplicationUninstallExecutionMode::Silent,
            ApplicationUninstallCapability::Ready,
        );
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "uninstalls only the explicitly named WinGet fixture"]
    fn real_winget_fixture_completes_the_full_uninstall_workflow() {
        let package_name = std::env::var("MANGODISK_TEST_WINGET_UNINSTALL_PACKAGE")
            .expect("set MANGODISK_TEST_WINGET_UNINSTALL_PACKAGE to a disposable WinGet package");
        real_package_fixture_completes_the_full_uninstall_workflow(
            &package_name,
            ApplicationUninstallInventorySource::Winget,
            ApplicationUninstallInstallerKind::WindowsRegistered,
            ApplicationUninstallExecutionMode::ExternalClient,
            ApplicationUninstallCapability::Ready,
        );
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "uninstalls only the explicitly named Chocolatey fixture"]
    fn real_chocolatey_fixture_completes_the_full_uninstall_workflow() {
        let package_name = std::env::var("MANGODISK_TEST_CHOCOLATEY_UNINSTALL_PACKAGE")
            .expect("set MANGODISK_TEST_CHOCOLATEY_UNINSTALL_PACKAGE to a disposable package");
        real_package_fixture_completes_the_full_uninstall_workflow(
            &package_name,
            ApplicationUninstallInventorySource::Chocolatey,
            ApplicationUninstallInstallerKind::WindowsChocolatey,
            ApplicationUninstallExecutionMode::ExternalClient,
            ApplicationUninstallCapability::RequiresElevation,
        );
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "uninstalls only the explicitly named registered application fixture"]
    fn real_registered_fixture_completes_the_full_uninstall_workflow() {
        let application_name = std::env::var("MANGODISK_TEST_REGISTERED_UNINSTALL_NAME").expect(
            "set MANGODISK_TEST_REGISTERED_UNINSTALL_NAME to a disposable registered application",
        );
        real_named_fixture_completes_the_full_uninstall_workflow(
            &application_name,
            ApplicationUninstallInstallerKind::WindowsRegistered,
            ApplicationUninstallExecutionMode::Interactive,
            ApplicationUninstallCapability::Ready,
        );
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "uninstalls only the explicitly named MSI application fixture"]
    fn real_msi_fixture_completes_the_full_uninstall_workflow() {
        let application_name = std::env::var("MANGODISK_TEST_MSI_UNINSTALL_NAME")
            .expect("set MANGODISK_TEST_MSI_UNINSTALL_NAME to a disposable MSI application");
        real_named_fixture_completes_the_full_uninstall_workflow(
            &application_name,
            ApplicationUninstallInstallerKind::WindowsMsi,
            ApplicationUninstallExecutionMode::Silent,
            ApplicationUninstallCapability::RequiresElevation,
        );
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "uninstalls only the explicitly named AppX application fixture"]
    fn real_appx_fixture_completes_the_full_uninstall_workflow() {
        let application_name = std::env::var("MANGODISK_TEST_APPX_UNINSTALL_NAME")
            .expect("set MANGODISK_TEST_APPX_UNINSTALL_NAME to a disposable AppX application");
        real_named_fixture_completes_the_full_uninstall_workflow(
            &application_name,
            ApplicationUninstallInstallerKind::WindowsAppx,
            ApplicationUninstallExecutionMode::Silent,
            ApplicationUninstallCapability::Ready,
        );
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "reads only the explicitly named running application fixture"]
    fn real_running_fixture_is_blocked_before_uninstall() {
        let application_name = std::env::var("MANGODISK_TEST_RUNNING_APPLICATION_NAME").expect(
            "set MANGODISK_TEST_RUNNING_APPLICATION_NAME to an application that is currently running",
        );
        init_real_fixture_logger();
        let scan = ApplicationUninstallService::scan()
            .expect("the Windows application catalog should be available");
        let candidate = scan
            .candidates
            .iter()
            .find(|candidate| candidate.name.eq_ignore_ascii_case(&application_name))
            .expect("the running application should be present");

        assert_eq!(
            candidate.capability,
            ApplicationUninstallCapability::ApplicationRunning
        );
        assert!(
            !candidate.running_processes.is_empty(),
            "the blocked candidate should include running-process evidence"
        );
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "force-closes and uninstalls only the explicitly named disposable registered fixture"]
    fn real_running_registered_fixture_force_closes_and_uninstalls() {
        let application_name = std::env::var("MANGODISK_TEST_RUNNING_REGISTERED_UNINSTALL_NAME")
            .expect(
                "set MANGODISK_TEST_RUNNING_REGISTERED_UNINSTALL_NAME to a disposable registered application",
            );
        init_real_fixture_logger();
        let mut scan = ApplicationUninstallService::scan()
            .expect("the Windows application catalog should be available");
        let candidate = scan
            .candidates
            .iter()
            .find(|candidate| candidate.name.eq_ignore_ascii_case(&application_name))
            .expect("the running disposable application should be present");
        assert_eq!(
            candidate.capability,
            ApplicationUninstallCapability::ApplicationRunning
        );
        let application_id = candidate.application_id.clone();
        let close_result = ApplicationUninstallService::close_applications_from_catalog(
            ApplicationUninstallCloseRequest {
                application_ids: vec![application_id.clone()],
                mode: crate::ApplicationCloseMode::Force,
            },
            &mut scan,
        )
        .expect("the disposable application should force-close");
        assert_eq!(close_result.matched_process_count, 1);
        assert_eq!(close_result.remaining_process_count, 0);
        let stopped_candidate = scan
            .candidates
            .iter()
            .find(|candidate| candidate.application_id == application_id)
            .expect("the stopped candidate should remain in the catalog snapshot");
        assert!(stopped_candidate.running_processes.is_empty());
        assert_eq!(
            stopped_candidate.capability,
            ApplicationUninstallCapability::Ready
        );

        real_named_fixture_completes_the_full_uninstall_workflow(
            &application_name,
            ApplicationUninstallInstallerKind::WindowsRegistered,
            ApplicationUninstallExecutionMode::Interactive,
            ApplicationUninstallCapability::Ready,
        );
    }

    #[cfg(windows)]
    fn real_named_fixture_completes_the_full_uninstall_workflow(
        application_name: &str,
        expected_installer_kind: ApplicationUninstallInstallerKind,
        expected_execution_mode: ApplicationUninstallExecutionMode,
        expected_capability: ApplicationUninstallCapability,
    ) {
        init_real_fixture_logger();
        assert!(
            !application_name.trim().is_empty(),
            "the disposable application name must not be empty"
        );
        let scan = ApplicationUninstallService::scan()
            .expect("the Windows application catalog should be available");
        let candidate = scan
            .candidates
            .iter()
            .find(|candidate| candidate.name.eq_ignore_ascii_case(application_name))
            .expect("the disposable application should be present");
        assert_eq!(candidate.installer_kind, Some(expected_installer_kind));
        assert_eq!(candidate.execution_mode, Some(expected_execution_mode));
        assert_eq!(candidate.capability, expected_capability);
        let application_id = candidate.application_id.clone();
        let component_ids = candidate
            .components
            .iter()
            .filter(|component| component.default_selected)
            .map(|component| component.component_id.clone())
            .collect::<Vec<_>>();
        assert_eq!(
            component_ids.len(),
            1,
            "an application should expose one required native component"
        );
        let selection = ApplicationUninstallBatchSelection {
            application_id: application_id.clone(),
            component_ids,
        };
        let preparation =
            ApplicationUninstallService::prepare_batch_from_catalog(&[selection], &scan)
                .expect("the application uninstall should pass preflight");
        assert_eq!(preparation.preview.failed_application_count, 0);
        assert_eq!(preparation.preview.previewed_application_count, 1);

        let result = ApplicationUninstallService::execute_batch(preparation.plan, false)
            .expect("the application uninstall should execute");
        assert_eq!(
            result.affected_application_count, 1,
            "the native application uninstall should affect one application: {result:#?}"
        );
        assert_eq!(
            result.failed_application_count, 0,
            "the native application uninstall should not fail: {result:#?}"
        );

        let refreshed = ApplicationUninstallService::scan()
            .expect("the Windows application catalog should refresh");
        assert!(
            refreshed
                .candidates
                .iter()
                .all(|candidate| candidate.application_id != application_id),
            "the removed application must disappear from the refreshed catalog"
        );
    }

    #[cfg(windows)]
    fn real_package_fixture_completes_the_full_uninstall_workflow(
        package_name: &str,
        source: ApplicationUninstallInventorySource,
        expected_installer_kind: ApplicationUninstallInstallerKind,
        expected_execution_mode: ApplicationUninstallExecutionMode,
        expected_capability: ApplicationUninstallCapability,
    ) {
        init_real_fixture_logger();
        assert!(
            !package_name.trim().is_empty(),
            "the disposable package name must not be empty"
        );
        let expected_identifier = match source {
            ApplicationUninstallInventorySource::Scoop => {
                format!("current-user:{package_name}")
            }
            _ => package_name.to_string(),
        };
        let scan = ApplicationUninstallService::scan()
            .expect("the Windows application catalog should be available");
        let candidate = scan
            .candidates
            .iter()
            .find(|candidate| {
                candidate.source_identities.iter().any(|identity| {
                    identity.source == source
                        && identity
                            .identifier
                            .eq_ignore_ascii_case(&expected_identifier)
                })
            })
            .expect("the disposable package should be present");
        assert_eq!(candidate.installer_kind, Some(expected_installer_kind));
        assert_eq!(candidate.execution_mode, Some(expected_execution_mode));
        assert_eq!(candidate.capability, expected_capability);
        let component_ids = candidate
            .components
            .iter()
            .filter(|component| component.default_selected)
            .map(|component| component.component_id.clone())
            .collect::<Vec<_>>();
        assert_eq!(
            component_ids.len(),
            1,
            "a package should expose one required native component"
        );
        let selection = ApplicationUninstallBatchSelection {
            application_id: candidate.application_id.clone(),
            component_ids,
        };
        let preparation =
            ApplicationUninstallService::prepare_batch_from_catalog(&[selection], &scan)
                .expect("the package uninstall should pass preflight");
        assert_eq!(preparation.preview.failed_application_count, 0);
        assert_eq!(preparation.preview.previewed_application_count, 1);

        let result = ApplicationUninstallService::execute_batch(preparation.plan, false)
            .expect("the package uninstall should execute");
        assert_eq!(
            result.affected_application_count, 1,
            "the native package uninstall should affect one application: {result:#?}"
        );
        assert_eq!(
            result.failed_application_count, 0,
            "the native package uninstall should not fail: {result:#?}"
        );

        let refreshed = ApplicationUninstallService::scan()
            .expect("the Windows application catalog should refresh");
        assert!(
            refreshed.candidates.iter().all(|candidate| {
                candidate.source_identities.iter().all(|identity| {
                    identity.source != source
                        || !identity
                            .identifier
                            .eq_ignore_ascii_case(&expected_identifier)
                })
            }),
            "the removed package must disappear from the refreshed catalog"
        );
    }

    #[cfg(windows)]
    fn init_real_fixture_logger() {
        struct FixtureLogger;

        impl log::Log for FixtureLogger {
            fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
                metadata.level() <= log::Level::Debug
            }

            fn log(&self, record: &log::Record<'_>) {
                if self.enabled(record.metadata()) {
                    eprintln!("{} {}: {}", record.level(), record.target(), record.args());
                }
            }

            fn flush(&self) {}
        }

        static LOGGER: FixtureLogger = FixtureLogger;
        let _ = log::set_logger(&LOGGER);
        log::set_max_level(log::LevelFilter::Debug);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn system_application_is_never_ready() {
        let mut application = fixture_application();
        application.bundle_path = Some(PathBuf::from("/System/Applications/Mail.app"));
        assert_eq!(
            capability(&application, &[]),
            ApplicationUninstallCapability::ProtectedApplication
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn non_writable_bundle_outside_supported_roots_remains_view_only() {
        use std::{fs, os::unix::fs::PermissionsExt};

        let root = std::env::temp_dir().join(format!(
            "mangodisk-uninstall-capability-root-{}",
            std::process::id()
        ));
        let bundle = root.join("Example.app");
        fs::create_dir_all(bundle.join("Contents"))
            .expect("the disposable application bundle should be created");
        fs::set_permissions(&bundle, fs::Permissions::from_mode(0o555))
            .expect("the disposable bundle should become read-only");

        let mut application = fixture_application();
        application.bundle_path = Some(bundle.clone());
        assert_eq!(
            capability(&application, &[]),
            ApplicationUninstallCapability::ViewOnly
        );

        fs::set_permissions(&bundle, fs::Permissions::from_mode(0o755))
            .expect("the disposable bundle should become removable again");
        fs::remove_dir_all(root).expect("the disposable application bundle should be removed");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn non_writable_nested_bundle_outside_supported_roots_remains_view_only() {
        use std::{fs, os::unix::fs::PermissionsExt};

        let root = std::env::temp_dir().join(format!(
            "mangodisk-uninstall-capability-nested-{}",
            std::process::id()
        ));
        let bundle = root.join("Example.app");
        let protected = bundle.join("Contents/Resources/Protected");
        fs::create_dir_all(&protected)
            .expect("the disposable nested application directory should be created");
        fs::set_permissions(&protected, fs::Permissions::from_mode(0o555))
            .expect("the disposable nested directory should become read-only");

        let mut application = fixture_application();
        application.bundle_path = Some(bundle.clone());
        assert_eq!(
            capability(&application, &[]),
            ApplicationUninstallCapability::ViewOnly
        );

        fs::set_permissions(&protected, fs::Permissions::from_mode(0o755))
            .expect("the disposable nested directory should become removable again");
        fs::remove_dir_all(root).expect("the disposable application bundle should be removed");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn supported_application_roots_advertise_elevation_for_non_deletable_bundles() {
        let mut application = fixture_application();
        application.bundle_path = Some(PathBuf::from("/Applications/Example.APP"));
        assert_eq!(
            capability(&application, &[]),
            ApplicationUninstallCapability::RequiresElevation
        );

        application.bundle_path = Some(
            current_platform()
                .user_directories()
                .expect("the test requires macOS user directories")
                .home_directory()
                .join("Applications/Example.app"),
        );
        assert_eq!(
            capability(&application, &[]),
            ApplicationUninstallCapability::RequiresElevation
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn application_requiring_elevation_remains_visible_in_the_catalog() {
        let mut candidate = candidate(&fixture_application(), &ProcessSnapshot::default());
        candidate.capability = ApplicationUninstallCapability::RequiresElevation;

        assert!(is_visible_candidate(&candidate));
    }
}
