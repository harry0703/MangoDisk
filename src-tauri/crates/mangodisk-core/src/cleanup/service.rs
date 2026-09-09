use std::{
    collections::{HashMap, HashSet},
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

use sha2::{Digest, Sha256};

use crate::{
    cleanup::{
        CleanupActionKind, CleanupActionResult, CleanupApplicationCloseRequest,
        CleanupExecutionProgress, CleanupExecutionRuleResult, CleanupExecutionStage,
        CleanupRequest, CleanupResult, CustomCleanupRule,
    },
    history::{summarize_deep_cleanup, CleanupOperationDetails, DeepCleanupOperationDetails},
};

use crate::{
    applications::{
        catalog::{ProcessSnapshot, ScanContext},
        process_control::{close_resolved_applications, ResolvedApplicationCloseTarget},
    },
    cleanup::applicability::{evaluate_rule, rule_requires_process, Applicability},
    cleanup::cleaners,
    cleanup::rule_execution::{
        cancelled_action, execute_rule, measure_owned_rule, process_inspection_unavailable_action,
        DeleteStats, RuleExecutionContext,
    },
    cleanup::rules::{compile_scan_plan, compile_scoped_rules, registry},
    cleanup::source_selection::SourceSelectionPolicy,
    filesystem::metadata::{display_path, now_ms},
    history::HistoryService,
    shared::{
        operation::{CoordinatedOperationKind, OperationCancellationToken, OperationGuard},
        CoreError, CoreResult,
    },
};

#[cfg(test)]
use std::fs;

#[cfg(test)]
use crate::cleanup::{
    rule_execution::{
        delete_root_contents, delete_root_contents_with_progress, validate_rule_root,
        DeleteRootContentsPolicy,
    },
    rules::{CompiledRule, MatcherSpec},
};

#[cfg(test)]
use crate::filesystem::permanent_delete::{
    delete_directory_contents_permanently_with_cancellation_serial,
    physical_path_identity_snapshot, prepare_path_for_permanent_delete,
};

pub struct CleanupService;

const ITEM_PROGRESS_INTERVAL: Duration = Duration::from_millis(80);

struct ExecutionProgressReporter<F> {
    handler: F,
    started: Instant,
    planned_rule_ids: Vec<String>,
    total_rule_count: u64,
    validated_rule_count: u64,
    completed_rule_count: u64,
    checked_item_count: u64,
    checked_bytes: u64,
    affected_item_count: u64,
    released_bytes: u64,
    current_item_path: Option<String>,
    current_rule_affected_item_count: u64,
    current_rule_released_bytes: u64,
    completed_rule_results: Vec<CleanupExecutionRuleResult>,
    last_item_emit: Option<Instant>,
}

impl<F> ExecutionProgressReporter<F>
where
    F: FnMut(CleanupExecutionProgress),
{
    fn new(planned_rule_ids: Vec<String>, handler: F) -> Self {
        let total_rule_count = planned_rule_ids.len();
        Self {
            handler,
            started: Instant::now(),
            planned_rule_ids,
            total_rule_count: total_rule_count as u64,
            validated_rule_count: 0,
            completed_rule_count: 0,
            checked_item_count: 0,
            checked_bytes: 0,
            affected_item_count: 0,
            released_bytes: 0,
            current_item_path: None,
            current_rule_affected_item_count: 0,
            current_rule_released_bytes: 0,
            completed_rule_results: Vec::with_capacity(total_rule_count),
            last_item_emit: None,
        }
    }

    fn emit(&mut self, stage: CleanupExecutionStage, current_rule_id: Option<&str>) {
        (self.handler)(CleanupExecutionProgress {
            stage,
            planned_rule_ids: self.planned_rule_ids.clone(),
            current_rule_id: current_rule_id.map(str::to_owned),
            current_item_path: self.current_item_path.clone(),
            current_rule_affected_item_count: self.current_rule_affected_item_count,
            current_rule_released_bytes: self.current_rule_released_bytes,
            completed_rule_results: self.completed_rule_results.clone(),
            validated_rule_count: self.validated_rule_count,
            completed_rule_count: self.completed_rule_count,
            total_rule_count: self.total_rule_count,
            checked_item_count: self.checked_item_count,
            checked_bytes: self.checked_bytes,
            affected_item_count: self
                .affected_item_count
                .saturating_add(self.current_rule_affected_item_count),
            released_bytes: self
                .released_bytes
                .saturating_add(self.current_rule_released_bytes),
            elapsed_ms: self.started.elapsed().as_millis() as u64,
        });
    }

    fn record_validation(&mut self, item_count: u64, bytes: u64) {
        self.validated_rule_count = self
            .validated_rule_count
            .saturating_add(1)
            .min(self.total_rule_count);
        self.checked_item_count = self.checked_item_count.saturating_add(item_count);
        self.checked_bytes = self.checked_bytes.saturating_add(bytes);
    }

    fn finish_validation(&mut self) {
        // When a measurement stage exists, rules without a generic filesystem
        // measurement are already ready to execute or own their specialized
        // validation. Complete the stage without presenting those rules as
        // missing file checks.
        self.validated_rule_count = self.total_rule_count;
    }

    fn begin_rule(&mut self) {
        self.current_item_path = None;
        self.current_rule_affected_item_count = 0;
        self.current_rule_released_bytes = 0;
        self.last_item_emit = None;
    }

    fn record_item(&mut self, rule_id: &str, path: &Path, stats: &DeleteStats) {
        self.current_rule_affected_item_count = stats.affected_item_count;
        self.current_rule_released_bytes = stats.deleted_bytes;
        let now = Instant::now();
        if self
            .last_item_emit
            .is_some_and(|last_emit| now.duration_since(last_emit) < ITEM_PROGRESS_INTERVAL)
        {
            return;
        }
        // Path conversion allocates on both Windows and macOS. Do it only for
        // snapshots that will actually cross the adapter boundary; deletion
        // may otherwise pay this cost tens of thousands of times per rule.
        self.current_item_path = Some(display_path(path));
        self.last_item_emit = Some(now);
        self.emit(CleanupExecutionStage::Cleaning, Some(rule_id));
    }

    fn record_action(&mut self, action: &CleanupActionResult) {
        self.current_item_path = None;
        self.current_rule_affected_item_count = 0;
        self.current_rule_released_bytes = 0;
        self.last_item_emit = None;
        self.completed_rule_count = self
            .completed_rule_count
            .saturating_add(1)
            .min(self.total_rule_count);
        self.affected_item_count = self
            .affected_item_count
            .saturating_add(action.affected_item_count);
        self.released_bytes = self.released_bytes.saturating_add(action.released_bytes);
        self.completed_rule_results
            .push(CleanupExecutionRuleResult {
                rule_id: action.rule_id.clone(),
                status: action.status,
                affected_item_count: action.affected_item_count,
                released_bytes: action.released_bytes,
            });
    }
}

impl CleanupService {
    /// Closes applications referenced by trusted cleanup rules.
    ///
    /// The WebView supplies only stable rule IDs. Process aliases come from
    /// the validated rule catalog or fixed Core policy, never from the adapter,
    /// so an adapter cannot turn this workflow into
    /// an arbitrary process termination primitive.
    pub fn close_applications(
        request: CleanupApplicationCloseRequest,
    ) -> CoreResult<crate::ApplicationCloseBatchResult> {
        let operation = OperationGuard::start(CoordinatedOperationKind::ApplicationClose)?;
        let selected = request.rule_ids.iter().cloned().collect::<HashSet<_>>();
        if selected.is_empty() || selected.len() != request.rule_ids.len() {
            return Err(CoreError::invalid_input(
                "the cleanup application close selection is invalid",
            ));
        }
        let rules = registry()?;
        let mut targets = Vec::with_capacity(request.rule_ids.len());
        for rule_id in &request.rule_ids {
            if let Some(executable_names) = cleaners::project_artifact_close_processes(rule_id) {
                targets.push(ResolvedApplicationCloseTarget {
                    target_id: rule_id.clone(),
                    executable_names,
                    executable_paths: Vec::new(),
                });
                continue;
            }
            let rule = rules
                .iter()
                .find(|rule| rule.id == rule_id.as_str())
                .ok_or_else(|| {
                    CoreError::invalid_input(
                        "the cleanup application close request contains an unknown rule",
                    )
                })?;
            if rule.required_stopped_processes.is_empty() {
                return Err(CoreError::invalid_input(
                    "the cleanup rule does not define a close requirement",
                ));
            }
            targets.push(ResolvedApplicationCloseTarget {
                target_id: rule.id.to_string(),
                executable_names: rule.required_stopped_processes.clone(),
                executable_paths: Vec::new(),
            });
        }
        let result = close_resolved_applications(targets, request.mode)?;
        operation.complete();
        Ok(result)
    }

    /// Requests cooperative cancellation of the active cleanup execution.
    ///
    /// Files that were already removed remain reflected in the result and
    /// history. Long-running platform commands may finish their current native
    /// step before observing the token, but no later cleanup rule is started.
    pub fn cancel() {
        OperationCancellationToken::cleanup().cancel();
    }

    pub fn execute(request: CleanupRequest) -> CoreResult<CleanupResult> {
        Self::execute_with_progress(request, |_| {})
    }

    pub fn execute_with_progress<F>(
        request: CleanupRequest,
        progress: F,
    ) -> CoreResult<CleanupResult>
    where
        F: FnMut(CleanupExecutionProgress),
    {
        let operation_id = format!("deep-cleanup-{}", now_ms());
        Self::execute_deep_cleanup_step_with_progress(request, operation_id, progress)
    }

    pub fn execute_deep_cleanup_step(
        request: CleanupRequest,
        deep_cleanup_operation_id: String,
    ) -> CoreResult<CleanupResult> {
        Self::execute_deep_cleanup_step_with_progress(request, deep_cleanup_operation_id, |_| {})
    }

    pub fn execute_deep_cleanup_step_with_progress<F>(
        request: CleanupRequest,
        deep_cleanup_operation_id: String,
        progress: F,
    ) -> CoreResult<CleanupResult>
    where
        F: FnMut(CleanupExecutionProgress),
    {
        Self::execute_deep_cleanup_step_with_scope(
            request,
            deep_cleanup_operation_id,
            false,
            Vec::new(),
            true,
            Arc::new(HashMap::new()),
            progress,
        )
    }

    pub fn execute_deep_cleanup_step_with_selected_volumes_and_progress<F>(
        mut request: CleanupRequest,
        deep_cleanup_operation_id: String,
        progress: F,
    ) -> CoreResult<CleanupResult>
    where
        F: FnMut(CleanupExecutionProgress),
    {
        request.project_roots = super::volume_scope::resolve_selected_volume_roots(
            &request.project_roots,
            super::volume_scope::SelectedVolumeScopeOperation::Cleanup,
        )?;
        Self::execute_deep_cleanup_step_with_scope(
            request,
            deep_cleanup_operation_id,
            true,
            Vec::new(),
            true,
            Arc::new(HashMap::new()),
            progress,
        )
    }

    pub fn execute_deep_cleanup_step_with_custom_rules_and_progress<F>(
        request: CleanupRequest,
        deep_cleanup_operation_id: String,
        custom_scan_id: u64,
        custom_rules: Vec<CustomCleanupRule>,
        include_standard_rules: bool,
        progress: F,
    ) -> CoreResult<CleanupResult>
    where
        F: FnMut(CleanupExecutionProgress),
    {
        let custom_session =
            super::custom_session::resolve(custom_scan_id, &custom_rules, include_standard_rules)?;
        Self::execute_deep_cleanup_step_with_scope(
            request,
            deep_cleanup_operation_id,
            false,
            custom_session.rules,
            include_standard_rules,
            custom_session.empty_directory_authorizations,
            progress,
        )
    }

    fn execute_deep_cleanup_step_with_scope<F>(
        request: CleanupRequest,
        deep_cleanup_operation_id: String,
        selected_volume_scope: bool,
        custom_rules: Vec<CustomCleanupRule>,
        include_standard_rules: bool,
        empty_directory_authorizations: Arc<super::custom_session::EmptyDirectoryAuthorizations>,
        progress: F,
    ) -> CoreResult<CleanupResult>
    where
        F: FnMut(CleanupExecutionProgress),
    {
        let operation = OperationGuard::start(CoordinatedOperationKind::Cleanup)?;
        if request.rule_ids.is_empty() {
            return Err(CoreError::invalid_input(
                "at least one cleanup rule must be selected",
            ));
        }
        if deep_cleanup_operation_id.trim().is_empty() {
            return Err(CoreError::invalid_input(
                "deep cleanup operation id must not be empty",
            ));
        }
        let started_at_ms = now_ms();
        let started = Instant::now();
        let selected = request.rule_ids.iter().cloned().collect::<HashSet<_>>();
        if selected.len() != request.rule_ids.len() {
            return Err(CoreError::invalid_input(
                "the cleanup plan contains duplicate rules",
            ));
        }
        let source_selection_policy =
            SourceSelectionPolicy::from_request(&selected, &request.source_selections)?;
        let custom_rule_count = custom_rules.len();
        let rules = compile_scoped_rules(&custom_rules, include_standard_rules)?;
        if selected.iter().any(|id| {
            !rules.iter().any(|rule| rule.id == id.as_str())
                && (!include_standard_rules || !cleaners::contains(id))
        }) {
            return Err(CoreError::invalid_input(
                "the cleanup plan contains an unknown rule",
            ));
        }
        let cleaner_rule_ids = request
            .rule_ids
            .iter()
            .filter(|id| include_standard_rules && cleaners::contains(id))
            .cloned()
            .collect::<Vec<_>>();
        // The execution pipeline validates and runs declarative filesystem
        // rules first, then specialized cleaners. Publish that deterministic
        // queue so adapters never mistake selection order for execution order.
        let mut planned_rule_ids = rules
            .iter()
            .filter(|rule| selected.contains(rule.id.as_str()))
            .map(|rule| rule.id.clone())
            .collect::<Vec<_>>();
        planned_rule_ids.extend(cleaners::execution_rule_ids(&cleaner_rule_ids));
        if planned_rule_ids.len() != request.rule_ids.len() {
            return Err(CoreError::operation_failed(
                "cleanup execution planning did not preserve every selected rule",
            ));
        }
        let mut progress = ExecutionProgressReporter::new(planned_rule_ids, progress);
        let validation_started = Instant::now();
        let applicability_context = if include_standard_rules {
            ScanContext::capture()
        } else {
            ScanContext::empty()
        };
        let applicability_process_snapshot = if rules.iter().any(rule_requires_process) {
            match ProcessSnapshot::capture() {
                Ok(snapshot) => Some(snapshot),
                Err(error) => {
                    log::warn!(
                        "cleanup_applicability_process_snapshot_failed error_digest={}",
                        blake3::hash(error.as_bytes()).to_hex()
                    );
                    None
                }
            }
        } else {
            None
        };
        let availability = rules
            .iter()
            .map(|rule| {
                evaluate_rule(
                    &applicability_context.inventory,
                    rule,
                    applicability_process_snapshot.as_ref(),
                ) != Applicability::NotApplicable
            })
            .collect::<Vec<_>>();
        // Recompile ownership for every applicable rule, not only selected
        // rules. An unselected child rule still protects its files from a
        // selected parent rule.
        let ownership_plan = compile_scan_plan(rules, &availability, &[])?;
        let selected_rule_indices = ownership_plan
            .rules
            .iter()
            .enumerate()
            .filter(|(_, rule)| selected.contains(rule.id.as_str()))
            .map(|(rule_index, _)| rule_index)
            .collect::<Vec<_>>();
        // A dry run has no deletion traversal from which to derive an estimate.
        // Source-scoped execution must also prove every untrusted UI path is a
        // live source before mutation. Ordinary whole-rule cleanup can instead
        // validate and account for each candidate during its deletion pass.
        let measured_rule_count = selected_rule_indices
            .iter()
            .filter(|rule_index| {
                requires_preflight_measurement(
                    request.dry_run,
                    source_selection_policy
                        .scope(&ownership_plan.rules[**rule_index].id)
                        .is_some(),
                )
            })
            .count();
        progress.emit(preparation_stage(measured_rule_count), None);
        let mut measured_rules = Vec::with_capacity(selected_rule_indices.len());
        for rule_index in &selected_rule_indices {
            operation.ensure_not_cancelled()?;
            let rule = &ownership_plan.rules[*rule_index];
            let requires_measurement = requires_preflight_measurement(
                request.dry_run,
                source_selection_policy.scope(&rule.id).is_some(),
            );
            if !requires_measurement {
                measured_rules.push(None);
                continue;
            }
            progress.emit(CleanupExecutionStage::Validating, Some(&rule.id));
            let measured = measure_owned_rule(
                &ownership_plan,
                *rule_index,
                source_selection_policy.scope(&rule.id),
            )?;
            progress.record_validation(measured.file_count, measured.bytes);
            progress.emit(CleanupExecutionStage::Validating, Some(&rule.id));
            measured_rules.push(Some(measured));
        }
        // No filesystem mutation has happened yet. Even executions that skip
        // measurement may still be cancelled cleanly before the first rule.
        operation.ensure_not_cancelled()?;
        let validation_elapsed_ms = validation_started.elapsed().as_millis() as u64;
        log::info!(
            "cleanup_started operation_id={} ownership_plan_id={} rule_count={} custom_rule_count={} include_standard_rules={} filesystem_rule_count={} cleaner_rule_count={} measured_rule_count={} validation_elapsed_ms={} rule_ids={:?} dry_run={}",
            operation.id(),
            ownership_plan.plan_id,
            request.rule_ids.len(),
            custom_rule_count,
            include_standard_rules,
            selected_rule_indices.len(),
            cleaner_rule_ids.len(),
            measured_rule_count,
            validation_elapsed_ms,
            request.rule_ids,
            request.dry_run
        );

        let plan_id = format!("plan-{started_at_ms}");
        let plan_hash = plan_hash(
            &plan_id,
            &request.rule_ids,
            &request.project_roots,
            &request.source_selections,
            request.dry_run,
        );
        let mut actions = Vec::new();
        if measured_rule_count > 0 {
            progress.finish_validation();
        }
        for (rule_index, measured) in selected_rule_indices.into_iter().zip(measured_rules) {
            let rule = &ownership_plan.rules[rule_index];
            progress.begin_rule();
            progress.emit(CleanupExecutionStage::Cleaning, Some(&rule.id));
            let action = if operation.ensure_not_cancelled().is_err() {
                cancelled_action(
                    &rule.id,
                    CleanupActionKind::Delete,
                    measured.as_ref().map_or(0, |measurement| measurement.bytes),
                )
            } else {
                // Process state is volatile. Capture it immediately before each
                // process-gated rule instead of reusing one snapshot across a
                // potentially long cleanup batch. The rule execution guard then
                // refreshes it at a bounded interval while deletion continues.
                let process_snapshot = if rule.requires_app_close() {
                    ProcessSnapshot::capture()
                } else {
                    Ok(ProcessSnapshot::default())
                };
                match process_snapshot {
                    Ok(process_snapshot) => {
                        let mut report_item = |path: &Path, stats: &DeleteStats| {
                            progress.record_item(&rule.id, path, stats);
                        };
                        execute_rule(
                            rule,
                            rule_index,
                            measured,
                            &RuleExecutionContext {
                                ownership_plan: &ownership_plan,
                                process_snapshot: &process_snapshot,
                                source_scope: source_selection_policy.scope(&rule.id),
                                empty_directory_authorizations: empty_directory_authorizations
                                    .get(&rule.id),
                                operation: &operation,
                                dry_run: request.dry_run,
                            },
                            &mut report_item,
                        )
                    }
                    Err(error) => {
                        log::warn!(
                            "cleanup_rule_process_snapshot_failed rule_id={} error_digest={}",
                            rule.id,
                            blake3::hash(error.as_bytes()).to_hex()
                        );
                        process_inspection_unavailable_action(
                            &rule.id,
                            measured.as_ref().map_or(0, |measurement| measurement.bytes),
                        )
                    }
                }
            };
            progress.record_action(&action);
            progress.emit(CleanupExecutionStage::Cleaning, Some(&rule.id));
            actions.push(action);
        }
        let active_rule_roots = ownership_plan.active_rule_roots();
        actions.extend(cleaners::execute_selected_with_progress(
            cleaners::CleanerExecutionRequest {
                rule_ids: &cleaner_rule_ids,
                inventory: &applicability_context.inventory,
                declared_roots: &active_rule_roots,
                project_roots: &request.project_roots,
                selected_volume_scope,
                source_selections: &source_selection_policy,
                dry_run: request.dry_run,
                operation: &operation,
            },
            |rule_id, action| {
                if let Some(action) = action {
                    progress.record_action(action);
                } else {
                    progress.begin_rule();
                }
                progress.emit(CleanupExecutionStage::Cleaning, Some(rule_id));
            },
        ));
        let expected_bytes = actions.iter().map(|action| action.bytes_expected).sum();
        let released_bytes = actions.iter().map(|action| action.released_bytes).sum();
        let affected_item_count = actions
            .iter()
            .map(|action| action.affected_item_count)
            .sum();
        let failed_item_count = actions.iter().map(|action| action.failed_item_count).sum();
        progress.emit(CleanupExecutionStage::Finalizing, None);
        let record = summarize_deep_cleanup(
            deep_cleanup_operation_id,
            started_at_ms,
            now_ms(),
            request.dry_run,
            DeepCleanupOperationDetails {
                cleanup: Some(CleanupOperationDetails {
                    selected_rule_ids: request.rule_ids,
                    expected_bytes,
                    actions: actions.clone(),
                }),
                application_leftovers: None,
            },
        );
        // History is an auxiliary audit capability after deletion has occurred.
        // A persistence failure must not report an irreversible successful
        // operation as failed; structured state and logs preserve the failure.
        let history_saved = match HistoryService::upsert_deep_cleanup(record.clone()) {
            Ok(()) => true,
            Err(error) => {
                log::warn!(
                    "cleanup_history_save_failed operation_id={} run_id={} error_digest={}",
                    operation.id(),
                    record.operation_id,
                    blake3::hash(error.diagnostic().as_bytes()).to_hex()
                );
                false
            }
        };
        log::info!(
            "cleanup_finished operation_id={} status={} expected_bytes={} released_bytes={} affected_item_count={} failed_item_count={} history_saved={} elapsed_ms={}",
            operation.id(),
            record.outcome.as_str(),
            expected_bytes,
            released_bytes,
            affected_item_count,
            failed_item_count,
            history_saved,
            started.elapsed().as_millis()
        );
        operation.complete();
        Ok(CleanupResult {
            plan_id,
            plan_hash,
            expected_bytes,
            released_bytes,
            affected_item_count,
            failed_item_count,
            dry_run: request.dry_run,
            actions,
            record,
            history_saved,
        })
    }
}

/// Limits the expensive read-only traversal to cases that cannot safely derive
/// their result from the destructive pass. A preview has no destructive pass,
/// while a source-scoped request must authenticate untrusted UI paths before
/// any selected file is removed.
fn requires_preflight_measurement(dry_run: bool, has_source_scope: bool) -> bool {
    dry_run || has_source_scope
}

fn preparation_stage(measured_rule_count: usize) -> CleanupExecutionStage {
    if measured_rule_count == 0 {
        CleanupExecutionStage::Cleaning
    } else {
        CleanupExecutionStage::Validating
    }
}

fn plan_hash(
    plan_id: &str,
    rule_ids: &[String],
    project_roots: &[String],
    source_selections: &[crate::cleanup::CleanupSourceSelection],
    dry_run: bool,
) -> String {
    let mut normalized = rule_ids.to_vec();
    normalized.sort();
    let mut normalized_roots = project_roots.to_vec();
    normalized_roots.sort();
    let mut hasher = Sha256::new();
    update_hash_field(&mut hasher, plan_id.as_bytes());
    update_hash_field(&mut hasher, if dry_run { b"dry-run" } else { b"apply" });
    for id in normalized {
        update_hash_field(&mut hasher, id.as_bytes());
    }
    for root in normalized_roots {
        update_hash_field(&mut hasher, root.as_bytes());
    }
    let mut normalized_sources = source_selections.to_vec();
    normalized_sources.sort_by(|left, right| left.rule_id.cmp(&right.rule_id));
    for selection in normalized_sources {
        update_hash_field(&mut hasher, selection.rule_id.as_bytes());
        update_hash_field(
            &mut hasher,
            match selection.mode {
                crate::cleanup::CleanupSourceSelectionMode::Include => b"include",
                crate::cleanup::CleanupSourceSelectionMode::Exclude => b"exclude",
            },
        );
        let mut paths = selection.paths;
        paths.sort();
        for path in paths {
            update_hash_field(&mut hasher, path.as_bytes());
        }
    }
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn update_hash_field(hasher: &mut Sha256, value: &[u8]) {
    hasher.update((value.len() as u64).to_le_bytes());
    hasher.update(value);
}

#[cfg(test)]
include!("service_tests/common.rs");

#[cfg(all(test, target_os = "macos"))]
include!("service_tests/macos.rs");

#[cfg(all(test, windows))]
include!("service_tests/windows.rs");
