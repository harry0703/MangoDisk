use std::{
    collections::{BTreeMap, HashMap},
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc::sync_channel,
        Arc,
    },
    thread,
    time::Instant,
};

use mangodisk_platform::{
    current_platform, DirectoryTreeAggregateError, Platform, PlatformCancellation, ScanDeviceClass,
    VolumeInfo,
};

use crate::{
    applications::catalog::{ApplicationInventory, ProcessSnapshot, ScanContext},
    cleanup::applicability::{evaluate_rule, rule_requires_process, Applicability},
    cleanup::measurement::MeasureResult,
    cleanup::{
        cleaners,
        exclusions::CleanupExclusions,
        rules::{
            compile_scan_plan, compile_scoped_rules, prepare_custom_scan_rules, ApplicabilityProbe,
            RootScanTask, RuleRiskLevel, ScanPlan,
        },
        source_selection::cleanup_source_path,
        CleanupApplicationIcon, CleanupGroup, CleanupScanEngineInfo, CleanupScanResult,
        CleanupSourceDetail, CustomCleanupRule, RiskLevel, ScanItemStatus, ScanRuleResult,
    },
    filesystem::{
        metadata::{display_path, is_link_like, latest_timestamp, modified_ms, now_ms},
        permanent_delete::{physical_path_identity_snapshot, PhysicalPathIdentity},
    },
    shared::{
        operation::{CoordinatedOperationKind, OperationGuard},
        progress::ProgressTracker,
        CoreResult, ProgressSink, TraversalStage,
    },
};

const CLEANUP_SCAN_WORKER_LIMIT: usize = 4;
const MAX_CLEANUP_SOURCE_DETAILS: usize = 256;
// Empty-directory authorization is retained only until the matching cleanup
// result expires. Bounding each rule prevents a user-selected tree containing
// millions of empty directories from turning the session cache into an
// unbounded in-memory filesystem index. Overflow fails closed and marks the
// rule limited instead of authorizing directories that were not retained.
const MAX_EMPTY_DIRECTORY_AUTHORIZATIONS_PER_RULE: usize = 4_096;
const CLEANUP_SCAN_SCHEMA_VERSION: &str = "1.9";

pub struct CleanupScanService;

/// Runs cleaner previews beside filesystem measurement without letting a
/// worker outlive its scan operation. Drop cancels and joins early-returned
/// tasks, while `finish` consumes the result on the successful path.
struct CleanerPreviewTask {
    enabled: bool,
    task: Option<thread::JoinHandle<Vec<ScanRuleResult>>>,
    stop: Arc<AtomicBool>,
    started: Instant,
}

impl CleanerPreviewTask {
    fn start(
        inventory: ApplicationInventory,
        declared_roots: Vec<PathBuf>,
        project_roots: Vec<String>,
        deep_project_discovery: bool,
        operation_cancelled: Arc<AtomicBool>,
        progress: Arc<ProgressTracker>,
        exclusions: CleanupExclusions,
    ) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let task = thread::Builder::new()
            .name("mangodisk-cleaner-preview".to_string())
            .spawn(move || {
                let cancellation = PlatformCancellation::new(move || {
                    operation_cancelled.load(Ordering::Relaxed)
                        || worker_stop.load(Ordering::Relaxed)
                });
                let report_path = |path: &Path| {
                    progress.visit_directory(TraversalStage::Analyzing, path);
                };
                let report_files = |path: &Path, file_count: u64, bytes: u64| {
                    progress.observe_files(TraversalStage::Analyzing, path, file_count, bytes);
                };
                cleaners::preview_all(cleaners::CleanerPreviewRequest {
                    inventory: &inventory,
                    declared_roots: &declared_roots,
                    project_roots: &project_roots,
                    deep_project_discovery,
                    cancellation: &cancellation,
                    report_path: &report_path,
                    report_files: &report_files,
                    exclusions: &exclusions,
                })
            })
            .map_err(|error| {
                log::warn!(
                    "cleanup_cleaner_preview_failed reason=workerSpawnFailed error={}",
                    mangodisk_platform::diagnostics::text(&error)
                );
            })
            .ok();
        Self {
            enabled: true,
            task,
            stop,
            started: Instant::now(),
        }
    }

    fn disabled() -> Self {
        Self {
            enabled: false,
            task: None,
            stop: Arc::new(AtomicBool::new(false)),
            started: Instant::now(),
        }
    }

    fn finish(mut self) -> (Vec<ScanRuleResult>, u64, u64) {
        let wait_started = Instant::now();
        let rules = self.join_or_limited();
        (
            rules,
            self.started.elapsed().as_millis() as u64,
            wait_started.elapsed().as_millis() as u64,
        )
    }

    fn join_or_limited(&mut self) -> Vec<ScanRuleResult> {
        if !self.enabled {
            return Vec::new();
        }
        self.task
            .take()
            .map(|task| {
                task.join().unwrap_or_else(|_| {
                    log::warn!("cleanup_cleaner_preview_failed reason=workerPanicked");
                    cleaners::preview_limited_all()
                })
            })
            .unwrap_or_else(cleaners::preview_limited_all)
    }
}

impl Drop for CleanerPreviewTask {
    fn drop(&mut self) {
        let Some(task) = self.task.take() else {
            return;
        };
        self.stop.store(true, Ordering::Relaxed);
        if task.join().is_err() {
            log::warn!("cleanup_cleaner_preview_failed reason=workerPanickedDuringCleanup");
        }
    }
}

impl CleanupScanService {
    pub fn scan_previous_installations_with_privileges() -> CoreResult<ScanRuleResult> {
        #[cfg(windows)]
        {
            cleaners::preview_windows_previous_installations_with_privileges().map_err(|error| {
                if error.code() == mangodisk_platform::PlatformErrorCode::UserCancelled {
                    crate::shared::CoreError::operation_cancelled()
                } else {
                    error.into()
                }
            })
        }
        #[cfg(not(windows))]
        Err(crate::shared::CoreError::operation_failed(
            "privileged previous-installation scanning is available only on Windows",
        ))
    }

    pub fn scan_with_progress(callback: impl ProgressSink) -> CoreResult<CleanupScanResult> {
        Self::scan_cleanup_candidates_with_options(
            Vec::new(),
            false,
            Vec::new(),
            true,
            Vec::new(),
            callback,
        )
    }

    pub fn scan_with_excluded_paths(
        excluded_paths: Vec<String>,
        callback: impl ProgressSink,
    ) -> CoreResult<CleanupScanResult> {
        Self::scan_cleanup_candidates_with_options(
            Vec::new(),
            false,
            Vec::new(),
            true,
            excluded_paths,
            callback,
        )
    }

    pub fn scan_with_deep_project_discovery(
        deep_project_discovery: bool,
        callback: impl ProgressSink,
    ) -> CoreResult<CleanupScanResult> {
        Self::scan_cleanup_candidates_with_options(
            Vec::new(),
            deep_project_discovery,
            Vec::new(),
            true,
            Vec::new(),
            callback,
        )
    }

    pub fn scan_with_project_roots(
        project_roots: Vec<String>,
        callback: impl ProgressSink,
    ) -> CoreResult<CleanupScanResult> {
        Self::scan_cleanup_candidates_with_options(
            project_roots,
            false,
            Vec::new(),
            true,
            Vec::new(),
            callback,
        )
    }

    pub fn scan_with_selected_volumes(
        volume_roots: Vec<String>,
        callback: impl ProgressSink,
    ) -> CoreResult<CleanupScanResult> {
        let volume_roots = super::volume_scope::resolve_selected_volume_roots(
            &volume_roots,
            super::volume_scope::SelectedVolumeScopeOperation::Scan,
        )?;
        Self::scan_cleanup_candidates_with_options(
            volume_roots,
            true,
            Vec::new(),
            true,
            Vec::new(),
            callback,
        )
    }

    pub fn scan_with_selected_volumes_and_excluded_paths(
        volume_roots: Vec<String>,
        excluded_paths: Vec<String>,
        callback: impl ProgressSink,
    ) -> CoreResult<CleanupScanResult> {
        let volume_roots = super::volume_scope::resolve_selected_volume_roots(
            &volume_roots,
            super::volume_scope::SelectedVolumeScopeOperation::Scan,
        )?;
        Self::scan_cleanup_candidates_with_options(
            volume_roots,
            true,
            Vec::new(),
            true,
            excluded_paths,
            callback,
        )
    }

    pub fn scan_with_custom_rules(
        custom_rules: Vec<CustomCleanupRule>,
        include_standard_rules: bool,
        callback: impl ProgressSink,
    ) -> CoreResult<CleanupScanResult> {
        Self::scan_cleanup_candidates_with_options(
            Vec::new(),
            false,
            custom_rules,
            include_standard_rules,
            Vec::new(),
            callback,
        )
    }

    pub fn scan_with_custom_rules_and_excluded_paths(
        custom_rules: Vec<CustomCleanupRule>,
        include_standard_rules: bool,
        excluded_paths: Vec<String>,
        callback: impl ProgressSink,
    ) -> CoreResult<CleanupScanResult> {
        Self::scan_cleanup_candidates_with_options(
            Vec::new(),
            false,
            custom_rules,
            include_standard_rules,
            excluded_paths,
            callback,
        )
    }

    fn scan_cleanup_candidates_with_options(
        project_roots: Vec<String>,
        deep_project_discovery: bool,
        custom_rules: Vec<CustomCleanupRule>,
        include_standard_rules: bool,
        excluded_paths: Vec<String>,
        callback: impl ProgressSink,
    ) -> CoreResult<CleanupScanResult> {
        let operation = OperationGuard::start(CoordinatedOperationKind::CleanupScan)?;
        // The setting belongs to project-artifact discovery, not custom-only rules.
        let excluded_paths = if include_standard_rules {
            excluded_paths
        } else {
            Vec::new()
        };
        let exclusions = CleanupExclusions::resolve(&excluded_paths)?;
        if !excluded_paths.is_empty() {
            log::info!(
                "cleanup_scan_exclusion_policy operation_id={} scope={} excluded_path_count={}",
                operation.id(),
                "projectArtifactsOnly",
                excluded_paths.len()
            );
        }
        let filesystem_exclusions = CleanupExclusions::default();
        let started = Instant::now();
        // Windows process enumeration can take hundreds of milliseconds. It is
        // independent from inventory capture and traversal, so start it early.
        let mut process_snapshot_task = include_standard_rules.then(|| {
            thread::spawn(|| {
                let started = Instant::now();
                (
                    ProcessSnapshot::capture(),
                    started.elapsed().as_millis() as u64,
                )
            })
        });
        let disk = current_platform()
            .system_volume()
            .map_err(|error| error.to_string())?
            .into();
        let custom_rule_count = custom_rules.len();
        let (effective_custom_rules, missing_custom_root_count) =
            prepare_custom_scan_rules(&custom_rules)?;
        if missing_custom_root_count > 0 {
            log::warn!("custom_cleanup_scan_roots_skipped operation_id={} requested_rule_count={} active_rule_count={} missing_root_count={} reason=notFound", operation.id(), custom_rules.len(), effective_custom_rules.len(), missing_custom_root_count);
        }
        let definitions = compile_scoped_rules(&effective_custom_rules, include_standard_rules)?;
        let applicability_started = Instant::now();
        let scan_context = if include_standard_rules {
            ScanContext::capture()
        } else {
            ScanContext::empty()
        };
        let requires_process_for_applicability = definitions.iter().any(rule_requires_process);
        // Only applicability probes need process data before traversal. Close-
        // application validation may join later without delaying first results.
        let mut process_snapshot = if requires_process_for_applicability {
            resolve_process_snapshot(&mut process_snapshot_task)
        } else {
            None
        };
        let availability = definitions
            .iter()
            .map(|rule| {
                evaluate_rule(&scan_context.inventory, rule, process_snapshot.as_ref())
                    != Applicability::NotApplicable
            })
            .collect::<Vec<_>>();
        let volumes = current_platform().volumes().unwrap_or_else(|error| {
            log::warn!(
                "scan_plan_volume_inventory_failed error={}",
                mangodisk_platform::diagnostics::text(&error)
            );
            Vec::new()
        });
        let volume_roots = volumes
            .iter()
            .map(|volume| PathBuf::from(&volume.mount_point))
            .collect::<Vec<_>>();
        let plan = compile_scan_plan(definitions, &availability, &volume_roots)?;
        let applicability_elapsed_ms = applicability_started.elapsed().as_millis() as u64;
        let progress = Arc::new(ProgressTracker::new(
            operation.id(),
            callback,
            plan.rules.len().saturating_add(if include_standard_rules {
                cleaners::count()
            } else {
                0
            }) as u64,
        ));
        // Cleaner previews must share the same progress tracker as the
        // declarative filesystem scan. Starting it after plan compilation costs
        // only the short planning window, while project discovery can now report
        // its active path instead of leaving the UI on the final cache directory.
        let cleaner_preview_task = if include_standard_rules {
            CleanerPreviewTask::start(
                scan_context.inventory.clone(),
                plan.active_rule_roots(),
                project_roots,
                deep_project_discovery,
                operation.cancellation_flag(),
                Arc::clone(&progress),
                exclusions.clone(),
            )
        } else {
            CleanerPreviewTask::disabled()
        };
        let available_workers = thread::available_parallelism()
            .map(|value| value.get())
            .unwrap_or(2)
            .min(CLEANUP_SCAN_WORKER_LIMIT);
        let (worker_count, scheduling_classes) =
            scan_worker_count(&plan, &volumes, available_workers);
        log::info!(
            "cleanup_scan_scheduler_configured worker_count={worker_count} available_worker_count={available_workers} device_classes={scheduling_classes}"
        );
        let filesystem_scan_started = Instant::now();
        let measurements = measure_scan_plan(
            &plan,
            &progress,
            operation.cancelled(),
            worker_count,
            &filesystem_exclusions,
        )?;
        let filesystem_scan_elapsed_ms = filesystem_scan_started.elapsed().as_millis() as u64;
        for rule_index in &plan.completed_without_io {
            let progress_path = plan.rules[*rule_index]
                .roots
                .first()
                .map(PathBuf::as_path)
                .unwrap_or_else(|| Path::new("/"));
            progress.complete_step(TraversalStage::Analyzing, progress_path, 0);
        }
        log::info!(
            "cleanup_scan_plan_compiled plan_id={} root_task_count={} rule_count={} completed_without_io={}",
            plan.plan_id,
            plan.root_tasks.len(),
            plan.rules.len(),
            plan.completed_without_io.len()
        );
        operation.ensure_not_cancelled()?;
        let process_snapshot_wait_started = Instant::now();
        if process_snapshot.is_none() && process_snapshot_task.is_some() {
            process_snapshot = resolve_process_snapshot(&mut process_snapshot_task);
        }
        let process_snapshot_wait_ms = process_snapshot_wait_started.elapsed().as_millis() as u64;
        let (cleaner_rules, cleaner_wall_elapsed_ms, cleaner_wait_ms) =
            cleaner_preview_task.finish();
        for rule in &cleaner_rules {
            progress.complete_step(
                TraversalStage::Analyzing,
                &current_platform().system_volume_path(),
                rule.bytes,
            );
        }
        operation.ensure_not_cancelled()?;
        let process_count = process_snapshot
            .as_ref()
            .map_or(0, |snapshot| snapshot.process_count);
        let cleaner_warning_count = cleaner_rules
            .iter()
            .filter(|rule| rule.status == ScanItemStatus::Limited)
            .count() as u64;
        let cleaner_ready_count = cleaner_rules
            .iter()
            .filter(|rule| matches!(rule.status, ScanItemStatus::Found | ScanItemStatus::Clean))
            .count() as u64;
        let cleaner_not_applicable_count = cleaner_rules
            .iter()
            .filter(|rule| rule.status == ScanItemStatus::NotApplicable)
            .count() as u64;
        let cleaner_scan_elapsed_ms = cleaner_rules
            .iter()
            .map(|rule| rule.scan_elapsed_ms)
            .sum::<u64>();
        let cleaner_count = cleaner_rules.len();
        let ScanPlanMeasurements {
            rules: measured_rules,
            elapsed_by_rule: scan_elapsed_by_rule,
            sources_by_rule,
            empty_directories_by_rule,
        } = measurements;
        let empty_directory_authorizations = plan
            .rules
            .iter()
            .zip(empty_directories_by_rule)
            .filter_map(|(rule, directories)| {
                (!directories.is_empty()).then(|| (rule.id.clone(), directories))
            })
            .collect::<super::custom_session::EmptyDirectoryAuthorizations>();
        let application_identifiers_by_rule = plan
            .rules
            .iter()
            .map(|rule| {
                (
                    rule.id.clone(),
                    cleanup_rule_application_identifiers(&rule.applicability),
                )
            })
            .collect::<HashMap<_, _>>();
        let warning_count = measured_rules
            .iter()
            .map(|measured| measured.skipped_count)
            .sum::<u64>()
            .saturating_add(cleaner_warning_count);
        let mut rules = plan
            .rules
            .into_iter()
            .zip(measured_rules)
            .zip(availability)
            .zip(scan_elapsed_by_rule)
            .zip(sources_by_rule)
            .map(
                |((((rule, measured), available), scan_elapsed_ms), sources)| {
                    let running = if measured.bytes > 0 && rule.requires_app_close() {
                        process_snapshot.as_ref().map_or_else(Vec::new, |snapshot| {
                            snapshot.matching_processes(&rule.required_stopped_processes)
                        })
                    } else {
                        Vec::new()
                    };
                    let status = scan_item_status(
                        available,
                        measured.bytes,
                        measured.skipped_count,
                        &running,
                    );
                    let (sources, source_count, sources_truncated) =
                        summarize_cleanup_sources(sources);
                    ScanRuleResult {
                        rule_id: rule.id.to_string(),
                        category: rule.category,
                        group: cleanup_group(rule.category, &rule.roots),
                        risk: public_risk(rule.risk),
                        default_selected: rule.default_selected,
                        recommended_selected: rule.recommended_selected,
                        bytes: measured.bytes,
                        file_count: measured.file_count,
                        available,
                        selectable: available && measured.bytes > 0,
                        status,
                        running_processes: running,
                        requires_app_close: rule.requires_app_close(),
                        sources,
                        source_count,
                        sources_truncated,
                        scan_elapsed_ms,
                    }
                },
            )
            .collect::<Vec<_>>();
        rules.extend(cleaner_rules);
        let application_icons = cleanup_application_icons(
            &rules,
            &scan_context.inventory,
            process_snapshot.as_ref(),
            &application_identifiers_by_rule,
        );
        let safe_bytes = rules
            .iter()
            .filter(|item| item.selectable && matches!(item.risk, RiskLevel::Safe))
            .map(|item| item.bytes)
            .sum();
        let reclaimable_bytes = rules
            .iter()
            .filter(|item| item.selectable)
            .map(|item| item.bytes)
            .sum();
        let found_count = rules.iter().filter(|item| item.selectable).count();
        let truncated_source_rule_count =
            rules.iter().filter(|item| item.sources_truncated).count();
        let clean_count = rules
            .iter()
            .filter(|item| item.status == ScanItemStatus::Clean)
            .count();
        let not_applicable_count = rules
            .iter()
            .filter(|item| item.status == ScanItemStatus::NotApplicable)
            .count();
        let result_group_counts = rules
            .iter()
            .filter(|item| item.selectable)
            .fold(BTreeMap::<&str, usize>::new(), |mut counts, item| {
                *counts.entry(item.group.as_str()).or_default() += 1;
                counts
            })
            .into_iter()
            .map(|(group, count)| format!("{group}:{count}"))
            .collect::<Vec<_>>()
            .join(",");
        let applicable_rule_count = rules.len().saturating_sub(not_applicable_count);
        let elapsed_ms = started.elapsed().as_millis() as u64;
        log::info!(
            "cleanup_scan_finished operation_id={} rule_count={} custom_rule_count={} include_standard_rules={} applicable_rule_count={} filtered_rule_count={} found_count={} clean_count={} truncated_source_rule_count={} result_group_counts={} reclaimable_bytes={} skipped_count={} applicability_elapsed_ms={} filesystem_scan_elapsed_ms={} cleaner_count={} cleaner_ready_count={} cleaner_limited_count={} cleaner_not_applicable_count={} cleaner_scan_elapsed_ms={} cleaner_wall_elapsed_ms={} cleaner_wait_ms={} process_snapshot_wait_ms={} inventory_application_count={} inventory_process_count={} application_icon_count={} elapsed_ms={}",
            operation.id(),
            rules.len(),
            custom_rule_count,
            include_standard_rules,
            applicable_rule_count,
            not_applicable_count,
            found_count,
            clean_count,
            truncated_source_rule_count,
            result_group_counts,
            reclaimable_bytes,
            warning_count,
            applicability_elapsed_ms,
            filesystem_scan_elapsed_ms,
            cleaner_count,
            cleaner_ready_count,
            cleaner_warning_count,
            cleaner_not_applicable_count,
            cleaner_scan_elapsed_ms,
            cleaner_wall_elapsed_ms,
            cleaner_wait_ms,
            process_snapshot_wait_ms,
            scan_context.inventory.application_count,
            process_count,
            application_icons.len(),
            elapsed_ms
        );
        progress.finish(
            TraversalStage::Analyzing,
            &current_platform().system_volume_path(),
        );
        let custom_scan_id = (!custom_rules.is_empty())
            .then(|| {
                super::custom_session::publish(
                    custom_rules,
                    effective_custom_rules,
                    include_standard_rules,
                    excluded_paths,
                    empty_directory_authorizations,
                )
            })
            .transpose()?;
        operation.complete();
        Ok(CleanupScanResult {
            schema_version: CLEANUP_SCAN_SCHEMA_VERSION.to_string(),
            custom_scan_id,
            missing_custom_root_count,
            scanned_at_ms: now_ms(),
            disk,
            rules,
            application_icons,
            warning_count,
            safe_bytes,
            reclaimable_bytes,
            applicability_elapsed_ms,
            applicable_rule_count: applicable_rule_count as u64,
            filtered_rule_count: not_applicable_count as u64,
            inventory_application_count: scan_context.inventory.application_count as u64,
            inventory_process_count: process_count as u64,
            elapsed_ms,
        })
    }

    /// Describes behavior recorded by cross-version baselines. Update these
    /// facts whenever scheduling, indexing, or incremental behavior changes.
    pub fn engine_info() -> CleanupScanEngineInfo {
        CleanupScanEngineInfo {
            // These values are persisted in longitudinal reports. They retain
            // their historical wording until the scan algorithm itself changes.
            strategy: "compiled-root-plan-native-complete-root-aggregate-with-safe-fallback-v10",
            rule_catalog_mode:
                "v2-embedded-declarative-with-special-and-project-artifact-registries",
            configured_worker_limit: CLEANUP_SCAN_WORKER_LIMIT,
            scan_result_persistence_enabled: false,
            single_pass_rule_matching: true,
            incremental_scan_enabled: false,
        }
    }

    pub fn cancel() {
        OperationGuard::cancel(CoordinatedOperationKind::CleanupScan);
    }
}

fn cleanup_application_icons(
    rules: &[ScanRuleResult],
    inventory: &ApplicationInventory,
    processes: Option<&ProcessSnapshot>,
    identifiers_by_rule: &HashMap<String, Vec<String>>,
) -> Vec<CleanupApplicationIcon> {
    let mut process_groups = Vec::<(String, Vec<String>)>::new();
    let mut group_indexes = HashMap::<String, usize>::new();
    for rule in rules {
        let rule_identifiers = identifiers_by_rule
            .get(&rule.rule_id)
            .map(Vec::as_slice)
            .unwrap_or_default();
        for process_name in &rule.running_processes {
            let normalized = process_name.to_ascii_lowercase();
            let index = *group_indexes.entry(normalized).or_insert_with(|| {
                process_groups.push((process_name.clone(), Vec::new()));
                process_groups.len() - 1
            });
            for identifier in rule_identifiers {
                if !process_groups[index]
                    .1
                    .iter()
                    .any(|existing| existing.eq_ignore_ascii_case(identifier))
                {
                    process_groups[index].1.push(identifier.clone());
                }
            }
        }
    }

    let requested_process_count = process_groups.len();
    let icons = process_groups
        .into_iter()
        .filter_map(|(process_name, identifiers)| {
            let icon_path = processes
                .and_then(|snapshot| {
                    inventory.application_icon_path_for_running_process(&process_name, snapshot)
                })
                .or_else(|| inventory.application_icon_path_for_process(&process_name))
                .or_else(|| inventory.application_icon_path_for_identifiers(&identifiers))?;
            Some(CleanupApplicationIcon {
                process_name,
                icon_path: display_path(icon_path),
            })
        })
        .collect::<Vec<_>>();
    log::debug!(
        "cleanup_application_icons_resolved requested_process_count={} resolved_count={} unresolved_count={}",
        requested_process_count,
        icons.len(),
        requested_process_count.saturating_sub(icons.len())
    );
    icons
}

fn cleanup_rule_application_identifiers(probes: &[ApplicabilityProbe]) -> Vec<String> {
    fn collect(probe: &ApplicabilityProbe, identifiers: &mut Vec<String>) {
        match probe {
            ApplicabilityProbe::ApplicationInstalled(values) => identifiers.extend(values.clone()),
            ApplicabilityProbe::ApplicationVersion { identifier, .. } => {
                identifiers.push(identifier.clone());
            }
            ApplicabilityProbe::AnyOf(items) | ApplicabilityProbe::AllOf(items) => {
                for item in items {
                    collect(item, identifiers);
                }
            }
            // A negated application probe describes something that must not
            // own the rule, so it cannot be trusted as icon association data.
            ApplicabilityProbe::Not(_)
            | ApplicabilityProbe::AnyRootExists
            | ApplicabilityProbe::PathExists(_)
            | ApplicabilityProbe::ExecutableAvailable(_)
            | ApplicabilityProbe::SystemVersion { .. }
            | ApplicabilityProbe::FileSystemIn(_)
            | ApplicabilityProbe::CapabilityAvailable(_)
            | ApplicabilityProbe::ProcessRunning(_) => {}
        }
    }

    let mut identifiers = Vec::new();
    for probe in probes {
        collect(probe, &mut identifiers);
    }
    identifiers.sort_by_key(|value| value.to_ascii_lowercase());
    identifiers.dedup_by(|left, right| left.eq_ignore_ascii_case(right));
    identifiers
}

fn cleanup_group(category: crate::cleanup::CleanupCategory, roots: &[PathBuf]) -> CleanupGroup {
    #[cfg(target_os = "macos")]
    {
        if roots.iter().any(|root| is_macos_xcode_data_root(root)) {
            return CleanupGroup::Xcode;
        }
        if roots.iter().any(|root| is_macos_user_cache_root(root)) {
            return CleanupGroup::UserCache;
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = roots;

    CleanupGroup::from(category)
}

#[cfg(target_os = "macos")]
fn is_macos_xcode_data_root(path: &Path) -> bool {
    let Ok(user_directories) = current_platform().user_directories() else {
        return false;
    };
    let developer = user_directories
        .home_directory()
        .join("Library")
        .join("Developer");
    path.starts_with(developer.join("Xcode")) || path.starts_with(developer.join("CoreSimulator"))
}

#[cfg(target_os = "macos")]
fn is_macos_user_cache_root(path: &Path) -> bool {
    let Ok(user_directories) = current_platform().user_directories() else {
        return false;
    };

    // `/Library/Caches` belongs to the system group. Only the active user's
    // cache tree is aggregated into the user-facing user-cache group.
    path.starts_with(user_directories.cache_directory())
}

fn measure_scan_plan(
    plan: &ScanPlan,
    progress: &Arc<ProgressTracker>,
    cancelled: &AtomicBool,
    worker_count: usize,
    exclusions: &CleanupExclusions,
) -> Result<ScanPlanMeasurements, String> {
    let mut measured = (0..plan.rules.len())
        .map(|_| MeasureResult::default())
        .collect::<Vec<_>>();
    let mut elapsed_by_rule = vec![0u64; plan.rules.len()];
    let mut sources_by_rule = (0..plan.rules.len())
        .map(|_| HashMap::new())
        .collect::<Vec<HashMap<PathBuf, SourceMeasurement>>>();
    let mut empty_directories_by_rule = (0..plan.rules.len())
        .map(|_| HashMap::new())
        .collect::<Vec<HashMap<PathBuf, PhysicalPathIdentity>>>();
    let mut task_counts = vec![0usize; plan.rules.len()];
    for task in &plan.root_tasks {
        for rule_index in task.rule_indices() {
            task_counts[rule_index] += 1;
        }
    }
    let mut remaining_tasks = task_counts.clone();
    if plan.root_tasks.is_empty() {
        return Ok(ScanPlanMeasurements {
            rules: measured,
            elapsed_by_rule,
            sources_by_rule,
            empty_directories_by_rule,
        });
    }
    let worker_count = worker_count.max(1).min(plan.root_tasks.len());
    let completed_task_count = thread::scope(|scope| -> Result<usize, String> {
        let next_task = Arc::new(AtomicUsize::new(0));
        // The channel carries root aggregates, never individual files. Matching
        // its capacity to worker count bounds memory while providing backpressure.
        let (result_sender, result_receiver) = sync_channel(worker_count);
        let workers = (0..worker_count)
            .map(|_| {
                let result_sender = result_sender.clone();
                let progress = Arc::clone(progress);
                let next_task = Arc::clone(&next_task);
                scope.spawn(move || loop {
                    if cancelled.load(Ordering::Relaxed) {
                        break;
                    }
                    let task_index = next_task.fetch_add(1, Ordering::Relaxed);
                    let Some(task) = plan.root_tasks.get(task_index) else {
                        break;
                    };
                    let started = Instant::now();
                    progress.emit(TraversalStage::Analyzing, &task.root);
                    let mut task_measured = HashMap::new();
                    let mut task_sources = HashMap::new();
                    let mut task_empty_directories = HashMap::new();
                    let context = RootMeasurementContext {
                        task,
                        rules: &plan.rules,
                        progress: &progress,
                        cancelled,
                        exclusions,
                    };
                    measure_root_task(
                        &context,
                        &mut task_measured,
                        &mut task_sources,
                        &mut task_empty_directories,
                    );
                    let result = RootTaskMeasurement {
                        task_index,
                        measured: task_measured,
                        sources: task_sources,
                        empty_directories: task_empty_directories,
                        elapsed_ms: started.elapsed().as_millis() as u64,
                    };
                    if result_sender.send(result).is_err() {
                        break;
                    }
                })
            })
            .collect::<Vec<_>>();
        drop(result_sender);

        let mut completed_task_count = 0;
        while let Ok(task_result) = result_receiver.recv() {
            completed_task_count += 1;
            let task = &plan.root_tasks[task_result.task_index];
            for (rule_index, result) in task_result.measured {
                merge_measure_result(&mut measured[rule_index], result);
            }
            for (rule_index, sources) in task_result.sources {
                let rule_sources = &mut sources_by_rule[rule_index];
                for (path, source) in sources {
                    merge_source_measurement(rule_sources.entry(path).or_default(), source);
                }
            }
            for (rule_index, directories) in task_result.empty_directories {
                let rule_directories = &mut empty_directories_by_rule[rule_index];
                for (path, identity) in directories {
                    if rule_directories.contains_key(&path) {
                        continue;
                    }
                    if rule_directories.len() >= MAX_EMPTY_DIRECTORY_AUTHORIZATIONS_PER_RULE {
                        measured[rule_index].skipped_count =
                            measured[rule_index].skipped_count.saturating_add(1);
                        continue;
                    }
                    rule_directories.insert(path, identity);
                }
            }
            // A merged task has one shared I/O wall time. Attribute it to each
            // participating rule for diagnostics, but never sum it as total time.
            for rule_index in task.rule_indices() {
                elapsed_by_rule[rule_index] =
                    elapsed_by_rule[rule_index].saturating_add(task_result.elapsed_ms);
                remaining_tasks[rule_index] = remaining_tasks[rule_index].saturating_sub(1);
                if !cancelled.load(Ordering::Relaxed) && remaining_tasks[rule_index] == 0 {
                    let progress_path = plan.rules[rule_index]
                        .roots
                        .first()
                        .map(PathBuf::as_path)
                        .unwrap_or_else(|| Path::new("/"));
                    progress.complete_step(
                        TraversalStage::Analyzing,
                        progress_path,
                        measured[rule_index].bytes,
                    );
                }
            }
        }
        for worker in workers {
            worker
                .join()
                .map_err(|_| "cleanup scan worker terminated unexpectedly".to_string())?;
        }
        if cancelled.load(Ordering::Relaxed) {
            // Partial task results live only in this local aggregate. Return an
            // error immediately so no caller can publish an incomplete snapshot.
            return Err("scan cancelled".to_string());
        }
        if completed_task_count != plan.root_tasks.len() {
            return Err(format!(
                "cleanup scan incomplete: expected {}, completed {}",
                plan.root_tasks.len(),
                completed_task_count
            ));
        }
        Ok(completed_task_count)
    })?;
    log::debug!(
        "cleanup_scan_scheduler_finished worker_count={} task_count={} completed_task_count={} cancelled={}",
        worker_count,
        plan.root_tasks.len(),
        completed_task_count,
        cancelled.load(Ordering::Relaxed)
    );
    Ok(ScanPlanMeasurements {
        rules: measured,
        elapsed_by_rule,
        sources_by_rule,
        empty_directories_by_rule,
    })
}

fn scan_worker_count(
    plan: &ScanPlan,
    volumes: &[VolumeInfo],
    available_workers: usize,
) -> (usize, String) {
    let mut classes = Vec::new();
    let mut device_limit = CLEANUP_SCAN_WORKER_LIMIT;
    for task in &plan.root_tasks {
        let scheduling = volumes
            .iter()
            .find(|volume| {
                current_platform().paths_equal(Path::new(&volume.mount_point), &task.volume_root)
            })
            .map(|volume| volume.scan_concurrency)
            .unwrap_or_else(|| {
                mangodisk_platform::ScanConcurrency::conservative(ScanDeviceClass::Unknown)
            });
        device_limit = device_limit.min(scheduling.worker_limit);
        if !classes.contains(&scheduling.class) {
            classes.push(scheduling.class);
        }
    }
    if classes.is_empty() {
        classes.push(ScanDeviceClass::Unknown);
        device_limit = 1;
    }
    classes.sort_by_key(|class| class.as_str());
    let class_summary = classes
        .into_iter()
        .map(ScanDeviceClass::as_str)
        .collect::<Vec<_>>()
        .join(",");
    // A plan may span several roots or volumes. One worker pool uses the lowest
    // participating device limit to avoid multiplying I/O on a physical device.
    (
        available_workers
            .max(1)
            .min(device_limit)
            .min(plan.root_tasks.len().max(1)),
        class_summary,
    )
}

struct ScanPlanMeasurements {
    rules: Vec<MeasureResult>,
    elapsed_by_rule: Vec<u64>,
    sources_by_rule: Vec<HashMap<PathBuf, SourceMeasurement>>,
    empty_directories_by_rule: Vec<HashMap<PathBuf, PhysicalPathIdentity>>,
}

#[derive(Debug, Default)]
struct SourceMeasurement {
    bytes: u64,
    file_count: u64,
    modified_at_ms: Option<u64>,
}

struct RootTaskMeasurement {
    task_index: usize,
    measured: HashMap<usize, MeasureResult>,
    sources: HashMap<usize, HashMap<PathBuf, SourceMeasurement>>,
    empty_directories: HashMap<usize, HashMap<PathBuf, PhysicalPathIdentity>>,
    elapsed_ms: u64,
}

fn measure_root_task(
    context: &RootMeasurementContext<'_>,
    measured: &mut HashMap<usize, MeasureResult>,
    sources: &mut HashMap<usize, HashMap<PathBuf, SourceMeasurement>>,
    empty_directories: &mut HashMap<usize, HashMap<PathBuf, PhysicalPathIdentity>>,
) {
    let path = &context.task.root;
    if context.cancelled.load(Ordering::Relaxed) || context.exclusions.matches(path) {
        return;
    }
    let Ok(metadata) = fs::symlink_metadata(path) else {
        // Preserve prior semantics when applications remove entries after
        // enumeration: this race is not a permission or link safety skip.
        return;
    };
    let aggregate_rule_index = if metadata.is_dir() && !is_link_like(&metadata) {
        context.task.complete_root_rule_index(context.rules)
    } else {
        None
    };
    if let Some(rule_index) = aggregate_rule_index.filter(|_| !context.exclusions.intersects(path))
    {
        let is_cancelled = || context.cancelled.load(Ordering::Relaxed);
        // Native traversal can take several seconds for package-manager
        // caches. Publish bounded measured batches so the UI remains useful,
        // while the lease removes partial totals if the platform path fails
        // and Core must retry with the portable walker.
        let mut observation = context.progress.begin_scan_observation();
        let report_progress = |current: &Path, file_count: u64, bytes: u64| {
            observation.observe(TraversalStage::Analyzing, current, file_count, bytes);
        };
        match current_platform().fast_directory_tree_aggregate(
            path,
            &is_cancelled,
            &report_progress,
        ) {
            Ok(Some(aggregate)) => {
                observation.commit_exact(
                    TraversalStage::Analyzing,
                    path,
                    aggregate.file_count,
                    aggregate.bytes,
                );
                merge_measure_result(
                    measured.entry(rule_index).or_default(),
                    MeasureResult {
                        bytes: aggregate.bytes,
                        file_count: aggregate.file_count,
                        skipped_count: aggregate.skipped_count,
                    },
                );
                let rule_sources = sources.entry(rule_index).or_default();
                for source in aggregate.sources {
                    merge_source_measurement(
                        rule_sources.entry(source.path).or_default(),
                        SourceMeasurement {
                            bytes: source.bytes,
                            file_count: source.file_count,
                            modified_at_ms: source.modified_at_ms,
                        },
                    );
                }
                log::debug!(
                    "cleanup_directory_aggregate_finished strategy={} file_count={} bytes={} skipped_count={}",
                    aggregate.strategy,
                    aggregate.file_count,
                    aggregate.bytes,
                    aggregate.skipped_count
                );
                return;
            }
            Ok(None) => {}
            Err(DirectoryTreeAggregateError::Cancelled) => return,
            Err(DirectoryTreeAggregateError::Platform(error)) => {
                log::warn!(
                    "cleanup_directory_aggregate_fallback error={}",
                    mangodisk_platform::diagnostics::text(&error)
                );
            }
        }
    }
    measure_root_entry(
        context,
        path,
        metadata,
        measured,
        sources,
        empty_directories,
    );
}

struct RootMeasurementContext<'a> {
    task: &'a RootScanTask,
    rules: &'a [crate::cleanup::rules::CompiledRule],
    progress: &'a Arc<ProgressTracker>,
    cancelled: &'a AtomicBool,
    exclusions: &'a CleanupExclusions,
}

fn measure_root_entry(
    context: &RootMeasurementContext<'_>,
    path: &Path,
    metadata: fs::Metadata,
    measured: &mut HashMap<usize, MeasureResult>,
    sources: &mut HashMap<usize, HashMap<PathBuf, SourceMeasurement>>,
    empty_directories: &mut HashMap<usize, HashMap<PathBuf, PhysicalPathIdentity>>,
) {
    if context.cancelled.load(Ordering::Relaxed) || context.exclusions.matches(path) {
        return;
    }
    if is_link_like(&metadata) {
        record_skipped(context.task, path, context.rules, measured);
        return;
    }
    if metadata.is_file() {
        context
            .progress
            .visit_file(TraversalStage::Analyzing, path, metadata.len());
        if let Some(owner) = context.task.matching_owner(path, &metadata, context.rules) {
            let result = measured.entry(owner.rule_index).or_default();
            result.bytes = result.bytes.saturating_add(metadata.len());
            result.file_count = result.file_count.saturating_add(1);
            let source = sources
                .entry(owner.rule_index)
                .or_default()
                .entry(cleanup_source_path(&owner.root, path))
                .or_default();
            source.bytes = source.bytes.saturating_add(metadata.len());
            source.file_count = source.file_count.saturating_add(1);
            source.modified_at_ms = latest_timestamp(source.modified_at_ms, modified_ms(&metadata));
        }
        return;
    }
    if !metadata.is_dir() {
        record_skipped(context.task, path, context.rules, measured);
        return;
    }
    context
        .progress
        .visit_directory(TraversalStage::Analyzing, path);
    if !context.task.should_descend(path, context.rules) {
        return;
    }
    let Ok(entries) = fs::read_dir(path) else {
        record_skipped(context.task, path, context.rules, measured);
        return;
    };
    let mut had_entry = false;
    let mut enumeration_complete = true;
    for entry in entries {
        if context.cancelled.load(Ordering::Relaxed) {
            return;
        }
        match entry {
            Ok(entry) => {
                had_entry = true;
                let child_path = entry.path();
                // DirEntry can reuse attributes returned by the directory enumeration on
                // Windows. Passing them into recursion avoids another path lookup while
                // preserving symlink-aware metadata semantics on every platform.
                match entry.metadata() {
                    Ok(metadata) => measure_root_entry(
                        context,
                        &child_path,
                        metadata,
                        measured,
                        sources,
                        empty_directories,
                    ),
                    Err(_) => record_skipped(context.task, &child_path, context.rules, measured),
                }
            }
            Err(_) => {
                enumeration_complete = false;
                record_skipped(context.task, path, context.rules, measured);
            }
        }
    }
    if !had_entry && enumeration_complete {
        record_empty_directory_authorization(context, path, measured, empty_directories);
    }
}

fn record_empty_directory_authorization(
    context: &RootMeasurementContext<'_>,
    path: &Path,
    measured: &mut HashMap<usize, MeasureResult>,
    empty_directories: &mut HashMap<usize, HashMap<PathBuf, PhysicalPathIdentity>>,
) {
    // Linux filesystems may immediately reuse an inode after an empty directory
    // is removed. A `(device, inode)` scan snapshot therefore cannot prove that
    // the current path is the reviewed directory. Leave empty directories in
    // place until the Linux adapter can retain a non-reusable object identity
    // across the review and execution boundary.
    if cfg!(target_os = "linux") {
        return;
    }
    let Some(rule_index) = context.task.empty_directory_owner(path, context.rules) else {
        return;
    };
    let rule_directories = empty_directories.entry(rule_index).or_default();
    if rule_directories.contains_key(path) {
        return;
    }
    if rule_directories.len() >= MAX_EMPTY_DIRECTORY_AUTHORIZATIONS_PER_RULE {
        let result = measured.entry(rule_index).or_default();
        result.skipped_count = result.skipped_count.saturating_add(1);
        return;
    }
    match physical_path_identity_snapshot(path) {
        Ok(identity) => {
            rule_directories.insert(path.to_path_buf(), identity);
        }
        Err(_) => {
            let result = measured.entry(rule_index).or_default();
            result.skipped_count = result.skipped_count.saturating_add(1);
        }
    }
}

fn merge_source_measurement(target: &mut SourceMeasurement, source: SourceMeasurement) {
    target.bytes = target.bytes.saturating_add(source.bytes);
    target.file_count = target.file_count.saturating_add(source.file_count);
    target.modified_at_ms = latest_timestamp(target.modified_at_ms, source.modified_at_ms);
}

fn summarize_cleanup_sources(
    sources: HashMap<PathBuf, SourceMeasurement>,
) -> (Vec<CleanupSourceDetail>, u64, bool) {
    let mut sources = sources
        .into_iter()
        .filter(|(_, source)| source.bytes > 0 || source.file_count > 0)
        .map(|(path, source)| CleanupSourceDetail {
            path: display_path(&path),
            bytes: source.bytes,
            file_count: source.file_count,
            modified_at_ms: source.modified_at_ms,
            block_reason: None,
        })
        .collect::<Vec<_>>();
    sources.sort_by(|left, right| {
        right
            .bytes
            .cmp(&left.bytes)
            .then_with(|| left.path.cmp(&right.path))
    });
    let source_count = sources.len() as u64;
    let sources_truncated = sources.len() > MAX_CLEANUP_SOURCE_DETAILS;
    sources.truncate(MAX_CLEANUP_SOURCE_DETAILS);
    (sources, source_count, sources_truncated)
}

fn record_skipped(
    task: &RootScanTask,
    path: &Path,
    rules: &[crate::cleanup::rules::CompiledRule],
    measured: &mut HashMap<usize, MeasureResult>,
) {
    if let Some(owner) = task.fallback_owner(path, rules) {
        measured.entry(owner.rule_index).or_default().skipped_count += 1;
    }
}

fn merge_measure_result(target: &mut MeasureResult, source: MeasureResult) {
    target.bytes = target.bytes.saturating_add(source.bytes);
    target.file_count = target.file_count.saturating_add(source.file_count);
    target.skipped_count = target.skipped_count.saturating_add(source.skipped_count);
}

fn resolve_process_snapshot(
    task: &mut Option<thread::JoinHandle<(Result<ProcessSnapshot, String>, u64)>>,
) -> Option<ProcessSnapshot> {
    match task.take()?.join() {
        Ok((Ok(snapshot), elapsed_ms)) => {
            log::debug!("process_snapshot_capture_finished elapsed_ms={elapsed_ms}");
            Some(snapshot)
        }
        Ok((Err(error), elapsed_ms)) => {
            log::warn!(
                "process_snapshot_capture_failed elapsed_ms={} error={}",
                elapsed_ms,
                mangodisk_platform::diagnostics::text(&error)
            );
            None
        }
        Err(_) => {
            log::error!("process_snapshot_worker_panicked");
            None
        }
    }
}

/// Centralizes status inference so clients do not reconstruct it from counters.
/// Permission limits become `Limited` only when no measurable content exists.
/// `CleanupScanResult::warning_count` retains diagnostics for unreadable directories.
fn scan_item_status(
    available: bool,
    bytes: u64,
    skipped_count: u64,
    running: &[String],
) -> ScanItemStatus {
    if !available {
        return ScanItemStatus::NotApplicable;
    }
    if bytes > 0 && !running.is_empty() {
        return ScanItemStatus::RequiresClose;
    }
    if bytes > 0 {
        return ScanItemStatus::Found;
    }
    if skipped_count > 0 {
        return ScanItemStatus::Limited;
    }
    ScanItemStatus::Clean
}

/// The current client exposes two stable groups. Recoverable and high-impact
/// rules both require confirmation while the domain retains all three levels.
const fn public_risk(risk: RuleRiskLevel) -> RiskLevel {
    match risk {
        RuleRiskLevel::Safe => RiskLevel::Safe,
        RuleRiskLevel::Recoverable | RuleRiskLevel::HighImpact => RiskLevel::Recoverable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct DirectoryCleanup(PathBuf);

    impl Drop for DirectoryCleanup {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn icon_identifiers_include_only_positive_application_probes() {
        let probes = vec![ApplicabilityProbe::AnyOf(vec![
            ApplicabilityProbe::ApplicationInstalled(vec![
                "bot.zenai".to_string(),
                "ZenAion".to_string(),
            ]),
            ApplicabilityProbe::AllOf(vec![ApplicabilityProbe::ApplicationVersion {
                identifier: "com.openai.codex".to_string(),
                minimum: None,
                maximum_exclusive: None,
            }]),
            ApplicabilityProbe::Not(Box::new(ApplicabilityProbe::ApplicationInstalled(vec![
                "excluded.app".to_string(),
            ]))),
        ])];

        assert_eq!(
            cleanup_rule_application_identifiers(&probes),
            vec![
                "bot.zenai".to_string(),
                "com.openai.codex".to_string(),
                "ZenAion".to_string(),
            ]
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_user_cache_roots_receive_the_shared_product_group() {
        let home = PathBuf::from(std::env::var_os("HOME").expect("macOS test must have a home"));
        let roots = vec![
            home.join("Library/Caches/Google/Chrome"),
            home.join("Library/Application Support/Google/Chrome"),
        ];

        assert_eq!(
            cleanup_group(crate::cleanup::CleanupCategory::Browser, &roots),
            CleanupGroup::UserCache
        );
        assert_eq!(
            cleanup_group(
                crate::cleanup::CleanupCategory::Development,
                &[PathBuf::from("/Users/example/.cargo/registry")],
            ),
            CleanupGroup::Development
        );
        assert_eq!(
            cleanup_group(
                crate::cleanup::CleanupCategory::System,
                &[PathBuf::from("/Library/Caches/com.apple.system")],
            ),
            CleanupGroup::System
        );
        assert_eq!(
            cleanup_group(
                crate::cleanup::CleanupCategory::Development,
                &[home.join("Library/Developer/CoreSimulator/Caches")],
            ),
            CleanupGroup::Xcode
        );
    }

    #[test]
    fn cleanup_source_summary_limits_locations_in_size_order() {
        let measured = (0..620_u64)
            .map(|index| {
                (
                    PathBuf::from(format!("/fixture/source-{index}")),
                    SourceMeasurement {
                        bytes: index + 1,
                        file_count: 1,
                        modified_at_ms: Some(index),
                    },
                )
            })
            .collect::<HashMap<_, _>>();

        let (sources, source_count, sources_truncated) = summarize_cleanup_sources(measured);

        assert_eq!(sources.len(), MAX_CLEANUP_SOURCE_DETAILS);
        assert_eq!(source_count, 620);
        assert!(sources_truncated);
        assert!(sources
            .windows(2)
            .all(|pair| pair[0].bytes >= pair[1].bytes));
        assert_eq!(sources.first().map(|source| source.bytes), Some(620));
        assert_eq!(sources.last().map(|source| source.bytes), Some(365));
    }

    #[test]
    fn custom_only_scan_returns_no_standard_cleanup_rules() {
        let _operation_lock = crate::shared::operation::test_operation_lock();
        let root = std::env::temp_dir().join(format!(
            "mangodisk-custom-only-scan-{}-{}",
            std::process::id(),
            now_ms()
        ));
        let _sandbox_cleanup = DirectoryCleanup(root.clone());
        fs::create_dir_all(&root).expect("custom scan root should be created");
        fs::write(root.join("fixture.tmp"), [1_u8; 17])
            .expect("custom scan fixture should be written");
        let rule = CustomCleanupRule {
            schema_version: crate::cleanup::CUSTOM_CLEANUP_RULE_SCHEMA_VERSION,
            id: "fixture-rule".to_string(),
            name: "Fixture files".to_string(),
            roots: vec![root.to_string_lossy().into_owned()],
            name_patterns: vec!["*.tmp".to_string()],
            minimum_bytes: None,
            maximum_bytes: None,
            modified_time: crate::cleanup::CustomCleanupModifiedTime::Any,
            recursive: true,
            remove_empty_directories: false,
        };

        let result = CleanupScanService::scan_with_custom_rules(vec![rule], false, |_| {})
            .expect("custom-only scan should succeed");

        assert_eq!(result.rules.len(), 1);
        assert_eq!(result.rules[0].rule_id, "custom.fixture-rule");
        assert!(result.rules[0].bytes >= 17);
        assert!(result.custom_scan_id.is_some());
    }

    #[test]
    fn custom_scan_collapses_nested_roots_without_misattributing_sources() {
        let _operation_lock = crate::shared::operation::test_operation_lock();
        let root = std::env::temp_dir().join(format!(
            "mangodisk-custom-overlap-scan-{}-{}",
            std::process::id(),
            now_ms()
        ));
        let _sandbox_cleanup = DirectoryCleanup(root.clone());
        let nested = root.join("nested");
        let sibling = root.join("sibling");
        fs::create_dir_all(&nested).expect("create the nested custom scan root");
        fs::create_dir_all(&sibling).expect("create the sibling custom scan source");
        fs::write(nested.join("nested.tmp"), [1_u8; 11])
            .expect("write the nested custom scan fixture");
        fs::write(sibling.join("sibling.tmp"), [2_u8; 13])
            .expect("write the sibling custom scan fixture");
        let canonical_root = fs::canonicalize(&root).expect("canonicalize the custom scan root");
        let canonical_nested = canonical_root.join("nested");
        let canonical_sibling = canonical_root.join("sibling");
        let rule = CustomCleanupRule {
            schema_version: crate::cleanup::CUSTOM_CLEANUP_RULE_SCHEMA_VERSION,
            id: "overlap-rule".to_string(),
            name: "Overlapping roots".to_string(),
            roots: vec![
                nested.to_string_lossy().into_owned(),
                root.to_string_lossy().into_owned(),
            ],
            name_patterns: vec!["*.tmp".to_string()],
            minimum_bytes: None,
            maximum_bytes: None,
            modified_time: crate::cleanup::CustomCleanupModifiedTime::Any,
            recursive: true,
            remove_empty_directories: false,
        };

        let result = CleanupScanService::scan_with_custom_rules(vec![rule], false, |_| {})
            .expect("the overlapping custom scan should succeed");
        let scanned_rule = result
            .rules
            .iter()
            .find(|item| item.rule_id == "custom.overlap-rule")
            .expect("the custom scan result must exist");
        let source_paths = scanned_rule
            .sources
            .iter()
            .map(|source| PathBuf::from(&source.path))
            .collect::<Vec<_>>();

        assert_eq!(scanned_rule.file_count, 2);
        assert_eq!(scanned_rule.source_count, 2);
        assert!(source_paths
            .iter()
            .any(|source| current_platform().paths_equal(source, &canonical_nested)));
        assert!(source_paths
            .iter()
            .any(|source| current_platform().paths_equal(source, &canonical_sibling)));
    }

    #[test]
    fn overlapping_rules_traverse_once_and_assign_one_owner() {
        let root = std::env::temp_dir().join(format!(
            "mangodisk-overlap-scan-{}-{}",
            std::process::id(),
            now_ms()
        ));
        let _sandbox_cleanup = DirectoryCleanup(root.clone());
        let child = root.join("nested");
        fs::create_dir_all(&child).expect("overlap fixture directory should be created");
        fs::write(root.join("outside.bin"), [0_u8; 4]).expect("parent fixture should be written");
        fs::write(child.join("cache.tmp"), [0_u8; 5]).expect("matched fixture should be written");
        fs::write(child.join("other.bin"), [0_u8; 6]).expect("child fixture should be written");

        let plan = compile_scan_plan(
            vec![
                crate::cleanup::rules::CompiledRule::fixture(
                    "system.parent",
                    root.clone(),
                    crate::cleanup::CleanupCategory::System,
                    crate::cleanup::rules::MatcherSpec::All,
                ),
                crate::cleanup::rules::CompiledRule::fixture(
                    "application.child",
                    child,
                    crate::cleanup::CleanupCategory::Application,
                    crate::cleanup::rules::MatcherSpec::ExtensionIn(vec!["tmp".to_string()]),
                ),
            ],
            &[true, true],
            &[std::env::temp_dir()],
        )
        .expect("overlapping rules should compile");
        assert_eq!(
            plan.root_tasks.len(),
            1,
            "nested roots should traverse once"
        );

        let progress = Arc::new(ProgressTracker::new(0, |_| {}, 2));
        let cancelled = AtomicBool::new(false);
        let measurements = measure_scan_plan(
            &plan,
            &progress,
            &cancelled,
            1,
            &CleanupExclusions::default(),
        )
        .expect("scan plan should succeed");
        let measured = &measurements.rules;

        assert_eq!(
            measured[0].bytes, 10,
            "parent rule must not double count child files"
        );
        assert_eq!(measured[0].file_count, 2);
        assert_eq!(measured[1].bytes, 5);
        assert_eq!(measured[1].file_count, 1);
        assert_eq!(
            measured.iter().map(|result| result.bytes).sum::<u64>(),
            15,
            "one file must not belong to two cleanup actions"
        );
        assert_eq!(measurements.sources_by_rule[0].len(), 2);
        assert!(measurements.sources_by_rule[0].contains_key(&root));
        assert!(measurements.sources_by_rule[0].contains_key(&root.join("nested")));
        assert_eq!(measurements.sources_by_rule[1].len(), 1);
    }

    #[test]
    fn single_and_bounded_multi_worker_results_match() {
        let root = std::env::temp_dir().join(format!(
            "mangodisk-scheduler-parity-{}-{}",
            std::process::id(),
            now_ms()
        ));
        let _sandbox_cleanup = DirectoryCleanup(root.clone());
        let first_root = root.join("first");
        let second_root = root.join("second");
        fs::create_dir_all(&first_root).expect("first scheduler root should be created");
        fs::create_dir_all(&second_root).expect("second scheduler root should be created");
        fs::write(first_root.join("one.bin"), [1_u8; 11]).expect("first fixture should be written");
        fs::write(first_root.join("two.bin"), [2_u8; 13])
            .expect("second fixture should be written");
        fs::write(second_root.join("three.bin"), [3_u8; 17])
            .expect("third fixture should be written");

        let plan = compile_scan_plan(
            vec![
                crate::cleanup::rules::CompiledRule::fixture(
                    "application.first",
                    first_root,
                    crate::cleanup::CleanupCategory::Application,
                    crate::cleanup::rules::MatcherSpec::All,
                ),
                crate::cleanup::rules::CompiledRule::fixture(
                    "application.second",
                    second_root,
                    crate::cleanup::CleanupCategory::Application,
                    crate::cleanup::rules::MatcherSpec::All,
                ),
            ],
            &[true, true],
            &[std::env::temp_dir()],
        )
        .expect("sibling roots should compile");

        let cancelled = AtomicBool::new(false);
        let serial_progress = Arc::new(ProgressTracker::new(0, |_| {}, 2));
        let parallel_progress = Arc::new(ProgressTracker::new(0, |_| {}, 2));
        let serial = measure_scan_plan(
            &plan,
            &serial_progress,
            &cancelled,
            1,
            &CleanupExclusions::default(),
        )
        .expect("single worker should succeed");
        let parallel = measure_scan_plan(
            &plan,
            &parallel_progress,
            &cancelled,
            4,
            &CleanupExclusions::default(),
        )
        .expect("multiple workers should succeed");

        assert_eq!(serial.rules.len(), parallel.rules.len());
        for (serial, parallel) in serial.rules.iter().zip(&parallel.rules) {
            assert_eq!(serial.bytes, parallel.bytes);
            assert_eq!(serial.file_count, parallel.file_count);
            assert_eq!(serial.skipped_count, parallel.skipped_count);
        }
        assert_eq!(serial.elapsed_by_rule.len(), parallel.elapsed_by_rule.len());
        assert_eq!(serial.sources_by_rule.len(), parallel.sources_by_rule.len());
    }

    #[test]
    fn cleanup_sources_group_by_first_directory_and_keep_root_files() {
        let root = Path::new("/tmp/mangodisk-cleanup-source");
        assert_eq!(
            cleanup_source_path(root, &root.join("browser/cache/data.bin")),
            root.join("browser")
        );
        assert_eq!(
            cleanup_source_path(root, &root.join("direct.log")),
            root.to_path_buf()
        );
        assert_eq!(
            cleanup_source_path(root, Path::new("/tmp/outside/data.bin")),
            root.to_path_buf()
        );
    }

    #[test]
    fn cleanup_source_summary_keeps_largest_bounded_details() {
        let sources = (0..MAX_CLEANUP_SOURCE_DETAILS + 7)
            .map(|index| {
                (
                    PathBuf::from(format!("/tmp/source-{index:03}")),
                    SourceMeasurement {
                        bytes: index as u64 + 1,
                        file_count: 1,
                        modified_at_ms: None,
                    },
                )
            })
            .collect::<HashMap<_, _>>();

        let (details, source_count, truncated) = summarize_cleanup_sources(sources);

        assert_eq!(details.len(), MAX_CLEANUP_SOURCE_DETAILS);
        assert_eq!(source_count, (MAX_CLEANUP_SOURCE_DETAILS + 7) as u64);
        assert!(truncated);
        assert_eq!(details.first().map(|source| source.bytes), Some(263));
        assert_eq!(details.last().map(|source| source.bytes), Some(8));
    }

    #[test]
    fn cleanup_source_summary_does_not_mark_exact_limit_as_truncated() {
        let sources = (0..MAX_CLEANUP_SOURCE_DETAILS)
            .map(|index| {
                (
                    PathBuf::from(format!("/tmp/source-{index:03}")),
                    SourceMeasurement {
                        bytes: 1,
                        file_count: 1,
                        modified_at_ms: None,
                    },
                )
            })
            .collect::<HashMap<_, _>>();

        let (details, source_count, truncated) = summarize_cleanup_sources(sources);

        assert_eq!(details.len(), MAX_CLEANUP_SOURCE_DETAILS);
        assert_eq!(source_count, MAX_CLEANUP_SOURCE_DETAILS as u64);
        assert!(!truncated);
    }

    #[test]
    fn scheduler_uses_lowest_volume_and_cpu_limit() {
        let volume_root = std::env::temp_dir();
        let rules = (0..4)
            .map(|index| {
                crate::cleanup::rules::CompiledRule::fixture(
                    &format!("application.scheduler-{index}"),
                    volume_root.join(format!("scheduler-{index}")),
                    crate::cleanup::CleanupCategory::Application,
                    crate::cleanup::rules::MatcherSpec::All,
                )
            })
            .collect::<Vec<_>>();
        let plan = compile_scan_plan(rules, &[true; 4], std::slice::from_ref(&volume_root))
            .expect("scheduler fixture should compile");
        let volume = |scan_concurrency| VolumeInfo {
            name: "fixture".to_string(),
            mount_point: volume_root.display().to_string(),
            total_bytes: 0,
            available_bytes: 0,
            used_bytes: 0,
            scan_concurrency,
        };

        let (ssd_workers, ssd_classes) = scan_worker_count(
            &plan,
            &[volume(mangodisk_platform::ScanConcurrency::solid_state())],
            8,
        );
        assert_eq!(ssd_workers, 4);
        assert_eq!(ssd_classes, "solid_state");

        let (cpu_limited_workers, _) = scan_worker_count(
            &plan,
            &[volume(mangodisk_platform::ScanConcurrency::solid_state())],
            2,
        );
        assert_eq!(cpu_limited_workers, 2);

        let (removable_workers, removable_classes) = scan_worker_count(
            &plan,
            &[volume(mangodisk_platform::ScanConcurrency::conservative(
                ScanDeviceClass::Removable,
            ))],
            8,
        );
        assert_eq!(removable_workers, 1);
        assert_eq!(removable_classes, "removable");

        let (unknown_workers, unknown_classes) = scan_worker_count(&plan, &[], 8);
        assert_eq!(unknown_workers, 1);
        assert_eq!(unknown_classes, "unknown");
    }

    #[test]
    fn pre_cancelled_scheduler_claims_no_tasks_under_250_ms_p95() {
        let root = std::env::temp_dir().join(format!(
            "mangodisk-scheduler-cancel-{}-{}",
            std::process::id(),
            now_ms()
        ));
        let _sandbox_cleanup = DirectoryCleanup(root.clone());
        fs::create_dir_all(&root).expect("cancellation fixture root should be created");
        fs::write(root.join("fixture.bin"), [7_u8; 32])
            .expect("cancellation fixture should be written");
        let plan = compile_scan_plan(
            vec![crate::cleanup::rules::CompiledRule::fixture(
                "application.cancel",
                root,
                crate::cleanup::CleanupCategory::Application,
                crate::cleanup::rules::MatcherSpec::All,
            )],
            &[true],
            &[std::env::temp_dir()],
        )
        .expect("cancellation fixture should compile");
        let cancelled = AtomicBool::new(true);
        let mut samples = Vec::new();

        for _ in 0..20 {
            let progress = Arc::new(ProgressTracker::new(0, |_| {}, 1));
            let started = Instant::now();
            let error = match measure_scan_plan(
                &plan,
                &progress,
                &cancelled,
                4,
                &CleanupExclusions::default(),
            ) {
                Ok(_) => panic!("cancellation must return an error"),
                Err(error) => error,
            };
            samples.push(started.elapsed().as_millis() as u64);
            assert_eq!(error, "scan cancelled");
        }
        samples.sort_unstable();
        let p95_index = samples
            .len()
            .saturating_mul(95)
            .div_ceil(100)
            .saturating_sub(1)
            .min(samples.len() - 1);
        assert!(
            samples[p95_index] < 250,
            "pre-cancelled scheduler P95 must be below 250 ms; actual={} ms",
            samples[p95_index]
        );
    }

    #[test]
    fn cancellation_after_task_claim_is_under_250_ms_p95() {
        let root = std::env::temp_dir().join(format!(
            "mangodisk-scheduler-inflight-cancel-{}-{}",
            std::process::id(),
            now_ms()
        ));
        let _sandbox_cleanup = DirectoryCleanup(root.clone());
        fs::create_dir_all(&root).expect("claimed-task fixture root should be created");
        fs::write(root.join("fixture.bin"), [9_u8; 32])
            .expect("claimed-task fixture should be written");
        let plan = compile_scan_plan(
            vec![crate::cleanup::rules::CompiledRule::fixture(
                "application.inflight-cancel",
                root,
                crate::cleanup::CleanupCategory::Application,
                crate::cleanup::rules::MatcherSpec::All,
            )],
            &[true],
            &[std::env::temp_dir()],
        )
        .expect("claimed-task fixture should compile");
        let mut samples = Vec::new();

        for _ in 0..20 {
            let cancelled = Arc::new(AtomicBool::new(false));
            let callback_cancelled = Arc::clone(&cancelled);
            let progress = Arc::new(ProgressTracker::new(
                0,
                move |_| callback_cancelled.store(true, Ordering::Relaxed),
                1,
            ));
            let started = Instant::now();
            let error = match measure_scan_plan(
                &plan,
                &progress,
                &cancelled,
                4,
                &CleanupExclusions::default(),
            ) {
                Ok(_) => panic!("in-task cancellation must return an error"),
                Err(error) => error,
            };
            samples.push(started.elapsed().as_millis() as u64);
            assert!(
                cancelled.load(Ordering::Relaxed),
                "progress callback must cancel"
            );
            assert_eq!(error, "scan cancelled");
        }
        samples.sort_unstable();
        let p95_index = samples
            .len()
            .saturating_mul(95)
            .div_ceil(100)
            .saturating_sub(1)
            .min(samples.len() - 1);
        assert!(
            samples[p95_index] < 250,
            "post-claim cancellation P95 must be below 250 ms; actual={} ms",
            samples[p95_index]
        );
    }

    /// This read-only integration test scans real cache directories. It remains
    /// ignored so ordinary unit tests do not depend on workstation contents.
    #[test]
    #[ignore = "requires scanning real cache directories on the current host"]
    fn real_cleanup_scan_returns_complete_rule_statuses() {
        let _operation_lock = crate::shared::operation::test_operation_lock();
        let expected_rule_count = crate::cleanup::rules::registry()
            .expect("rule registry should load")
            .len()
            .saturating_add(cleaners::count());
        let snapshot = CleanupScanService::scan_with_progress(|_| {})
            .expect("the real cleanup scan must succeed");

        assert_eq!(snapshot.rules.len(), expected_rule_count);
        assert!(
            snapshot.rules.len() >= 20,
            "scan should retain baseline coverage"
        );
        let found_count = snapshot.rules.iter().filter(|rule| rule.selectable).count();
        let detailed_source_count = snapshot
            .rules
            .iter()
            .map(|rule| rule.sources.len())
            .sum::<usize>();
        println!(
            "real_scan checked={} found={} sources={} releasable_bytes={} elapsed_ms={}",
            snapshot.rules.len(),
            found_count,
            detailed_source_count,
            snapshot.reclaimable_bytes,
            snapshot.elapsed_ms
        );
        assert!(snapshot.rules.iter().all(|rule| {
            rule.selectable
                == (rule.available
                    && rule.bytes > 0
                    && !matches!(
                        rule.status,
                        ScanItemStatus::Limited
                            | ScanItemStatus::ReviewOnly
                            | ScanItemStatus::RequiresElevation
                    ))
                && (rule.status != ScanItemStatus::Found || rule.bytes > 0)
                && (rule.status != ScanItemStatus::Clean || rule.bytes == 0)
                && (rule.status != ScanItemStatus::NotApplicable || !rule.available)
                && rule.source_count >= rule.sources.len() as u64
                && rule.sources_truncated == (rule.source_count > rule.sources.len() as u64)
                && rule.sources.iter().all(|source| {
                    !source.path.is_empty() && (source.bytes > 0 || source.file_count > 0)
                })
        }));
    }
}
