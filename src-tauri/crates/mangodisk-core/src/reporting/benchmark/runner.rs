use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Instant,
};

use mangodisk_platform::{current_platform, Platform};

use super::{
    dataset::BenchmarkDatasetService,
    render::render_markdown,
    report::{
        summarize_runs, BenchmarkDatasetSummary, BenchmarkDetailMetric, BenchmarkErrorSummary,
        BenchmarkExpectation, EngineBenchmarkArtifacts, EngineBenchmarkOptions,
        EngineBenchmarkReport, ModuleBenchmarkReport, ModuleBenchmarkRun, BENCHMARK_REPORT_KIND,
        BENCHMARK_SCHEMA_VERSION,
    },
    system::{environment_info, local_timestamp, now_ms},
};
use crate::{
    cleanup::CleanupScanResult,
    filesystem::metadata::display_path,
    shared::TraversalProgress,
    storage::{
        analysis::AnalysisResult,
        duplicates::DuplicateFilesResult,
        large_files::{LargeFileScanMode, LargeFilesResult},
    },
    AnalysisService, CleanupScanService, DuplicateFileService, LargeFileService,
};

const MAX_BENCHMARK_RUNS: usize = 10;
const MAX_LABEL_CHARACTERS: usize = 100;
const MAX_NOTE_CHARACTERS: usize = 500;
const LARGE_FILE_MINIMUM_BYTES: u64 = 50 * 1024 * 1024;
const DUPLICATE_MINIMUM_BYTES: u64 = 1;

#[derive(Debug, Default)]
struct ProgressCapture {
    first_progress_ms: Option<u64>,
    first_result_ms: Option<u64>,
    maximum_items_scanned: u64,
    maximum_bytes_scanned: u64,
    maximum_enumeration_bytes: u64,
    maximum_hash_read_bytes: u64,
    maximum_hash_completed_steps: u64,
    maximum_hash_total_steps: u64,
}

impl ProgressCapture {
    fn record(&mut self, progress: TraversalProgress) {
        if progress.items_scanned > 0 || progress.completed_steps > 0 {
            self.first_progress_ms.get_or_insert(progress.elapsed_ms);
        }
        if progress.found_items > 0 || progress.found_bytes > 0 {
            self.first_result_ms.get_or_insert(progress.elapsed_ms);
        }
        self.maximum_items_scanned = self.maximum_items_scanned.max(progress.items_scanned);
        self.maximum_bytes_scanned = self.maximum_bytes_scanned.max(progress.bytes_scanned);
        match progress.current_stage {
            crate::shared::TraversalStage::HashingFiles => {
                self.maximum_hash_read_bytes =
                    self.maximum_hash_read_bytes.max(progress.bytes_scanned);
                self.maximum_hash_completed_steps = self
                    .maximum_hash_completed_steps
                    .max(progress.completed_steps);
                self.maximum_hash_total_steps =
                    self.maximum_hash_total_steps.max(progress.total_steps);
            }
            _ => {
                self.maximum_enumeration_bytes =
                    self.maximum_enumeration_bytes.max(progress.bytes_scanned);
            }
        }
    }

    fn record_duplicate_group(&mut self, elapsed_ms: u64) {
        self.first_result_ms.get_or_insert(elapsed_ms);
    }
}

pub struct EngineBenchmarkService;

impl EngineBenchmarkService {
    /// Runs the four scan modules sequentially so measured timings are not distorted by another
    /// benchmark module competing for CPU or storage bandwidth. Cleanup uses the real user
    /// environment, while the other modules use the fixed dataset. Reports distinguish these
    /// workload kinds so comparisons cannot treat them as equivalent performance baselines.
    pub fn generate(options: EngineBenchmarkOptions) -> Result<EngineBenchmarkArtifacts, String> {
        validate_options(&options)?;
        let manifest = BenchmarkDatasetService::read_manifest(&options.dataset_manifest_path)?;
        let dataset_root = PathBuf::from(&manifest.root_path);
        validate_dataset_root(&options.dataset_manifest_path, &dataset_root)?;
        fs::create_dir_all(&options.output_directory).map_err(|error| {
            format!(
                "failed to create engine benchmark report directory {}: {error}",
                options.output_directory.display()
            )
        })?;

        let disk = current_platform()
            .system_volume()
            .map_err(|error| error.to_string())?
            .into();
        let environment = environment_info(options.environment_id.as_deref(), &disk);
        let generated_at_ms = now_ms();
        let generated_at_local = local_timestamp(generated_at_ms);
        let fixed_workload_digest = manifest.logical_digest.as_str();
        let expected_analysis_bytes = manifest.allocated_bytes.unwrap_or(manifest.logical_bytes);
        let modules = vec![
            benchmark_cleanup(options.runs).unwrap_or_else(|error| {
                failed_module(
                    "deepClean",
                    "environment",
                    "unavailable",
                    "parallelRuleBatches",
                    BenchmarkExpectation::default(),
                    &error,
                )
            }),
            benchmark_analysis(
                options.runs,
                &dataset_root,
                fixed_workload_digest,
                manifest.logical_file_count,
                expected_analysis_bytes,
            )
            .unwrap_or_else(|error| {
                failed_module(
                    "diskAnalysis",
                    "fixedDataset",
                    fixed_workload_digest,
                    "recursiveTraversalAndAggregate",
                    BenchmarkExpectation {
                        result_count: None,
                        result_bytes: Some(expected_analysis_bytes),
                    },
                    &error,
                )
            }),
            benchmark_large_files(
                options.runs,
                &dataset_root,
                fixed_workload_digest,
                manifest.expected_large_file_count,
                manifest.expected_large_file_bytes,
            )
            .unwrap_or_else(|error| {
                failed_module(
                    "largeFiles",
                    "fixedDataset",
                    fixed_workload_digest,
                    "platformCandidatesWithTraversalFallback",
                    BenchmarkExpectation {
                        result_count: Some(manifest.expected_large_file_count),
                        result_bytes: Some(manifest.expected_large_file_bytes),
                    },
                    &error,
                )
            }),
            benchmark_duplicate_files(
                options.runs,
                &dataset_root,
                fixed_workload_digest,
                manifest.expected_duplicate_group_count,
                manifest.expected_duplicate_file_count,
                manifest.expected_reclaimable_bytes,
            )
            .unwrap_or_else(|error| {
                failed_module(
                    "duplicateFiles",
                    "fixedDataset",
                    fixed_workload_digest,
                    "sizeThenSampleThenFullHash",
                    BenchmarkExpectation {
                        result_count: Some(manifest.expected_duplicate_group_count),
                        result_bytes: Some(manifest.expected_reclaimable_bytes),
                    },
                    &error,
                )
            }),
        ];
        let report = EngineBenchmarkReport {
            schema_version: BENCHMARK_SCHEMA_VERSION,
            report_kind: BENCHMARK_REPORT_KIND,
            label: options.label,
            note: options.note,
            generated_at_ms,
            generated_at_local,
            source: options.source,
            environment,
            disk,
            dataset: BenchmarkDatasetSummary::from(&manifest),
            modules,
        };
        let file_stem = format!(
            "engine-suite-{}-{}-{}",
            std::env::consts::OS,
            sanitize_label(&report.label),
            generated_at_ms
        );
        let json_path = options.output_directory.join(format!("{file_stem}.json"));
        let markdown_path = options.output_directory.join(format!("{file_stem}.md"));
        let json = serde_json::to_vec_pretty(&report)
            .map_err(|error| format!("failed to serialize the engine benchmark report: {error}"))?;
        write_atomic(&json_path, &[json, b"\n".to_vec()].concat())?;
        write_atomic(&markdown_path, render_markdown(&report).as_bytes())?;
        log::info!(
            "engine_benchmark_generated label={} runs={} dataset_id={} json_file={} markdown_file={}",
            report.label,
            options.runs,
            report.dataset.dataset_id,
            file_name(&json_path),
            file_name(&markdown_path)
        );
        Ok(EngineBenchmarkArtifacts {
            json_path,
            markdown_path,
        })
    }
}

fn benchmark_cleanup(runs: usize) -> Result<ModuleBenchmarkReport, String> {
    let filesystem_catalog_digest = crate::cleanup::rules::compatibility_digest()?;
    let cleaner_catalog_digest = crate::cleanup::cleaners::catalog_digest();
    let workload_digest =
        cleanup_workload_digest(&filesystem_catalog_digest, &cleaner_catalog_digest);
    let catalog = crate::cleanup::rules::catalog_diagnostics()?;
    let mut results = Vec::with_capacity(runs);
    let mut rule_samples = BTreeMap::<String, Vec<(u64, u64, u64, String)>>::new();
    let mut inventory_application_count = 0_u64;
    let mut inventory_process_count = 0_u64;
    let mut filtered_rule_count = 0_u64;
    for run_number in 1..=runs {
        let capture = Arc::new(Mutex::new(ProgressCapture::default()));
        let callback_capture = Arc::clone(&capture);
        let started = Instant::now();
        let snapshot = CleanupScanService::scan_with_progress(move |progress| {
            if let Ok(mut capture) = callback_capture.lock() {
                capture.record(progress);
            }
        })
        .map_err(|error| error.to_string())?;
        let elapsed_ms = started.elapsed().as_millis() as u64;
        inventory_application_count = snapshot.inventory_application_count;
        inventory_process_count = snapshot.inventory_process_count;
        filtered_rule_count = snapshot.filtered_rule_count;
        let capture = capture
            .lock()
            .map_err(|_| "failed to read cleanup benchmark progress".to_string())?;
        let result_count = snapshot.rules.iter().filter(|rule| rule.selectable).count() as u64;
        let filesystem_rules = snapshot
            .rules
            .iter()
            .filter(|rule| !crate::cleanup::cleaners::contains(&rule.rule_id))
            .collect::<Vec<_>>();
        let cleaner_rules = snapshot
            .rules
            .iter()
            .filter(|rule| crate::cleanup::cleaners::contains(&rule.rule_id))
            .collect::<Vec<_>>();
        let filesystem_result_bytes = filesystem_rules.iter().map(|rule| rule.bytes).sum();
        let cleaner_result_bytes = cleaner_rules.iter().map(|rule| rule.bytes).sum();
        for rule in &snapshot.rules {
            rule_samples.entry(rule.rule_id.clone()).or_default().push((
                rule.scan_elapsed_ms,
                rule.file_count,
                rule.bytes,
                format!("{:?}", rule.status),
            ));
        }
        results.push(ModuleBenchmarkRun {
            run_number,
            first_progress_ms: capture.first_progress_ms,
            first_result_ms: capture
                .first_result_ms
                .or((result_count > 0).then_some(elapsed_ms)),
            total_elapsed_ms: elapsed_ms,
            files_visited: capture.maximum_items_scanned,
            bytes_observed: capture.maximum_bytes_scanned,
            result_count,
            result_bytes: snapshot.reclaimable_bytes,
            skipped_count: snapshot.warning_count,
            result_digest: cleanup_digest(&snapshot),
            phase_elapsed_ms: BTreeMap::from([
                (
                    "applicability".to_string(),
                    snapshot.applicability_elapsed_ms,
                ),
                (
                    "ruleScan".to_string(),
                    snapshot
                        .elapsed_ms
                        .saturating_sub(snapshot.applicability_elapsed_ms),
                ),
            ]),
            work_metrics: BTreeMap::from([
                (
                    "filesystemRuleCount".to_string(),
                    filesystem_rules.len() as u64,
                ),
                (
                    "filesystemFoundRuleCount".to_string(),
                    filesystem_rules
                        .iter()
                        .filter(|rule| rule.selectable)
                        .count() as u64,
                ),
                ("filesystemResultBytes".to_string(), filesystem_result_bytes),
                (
                    "specialCleanerCount".to_string(),
                    cleaner_rules.len() as u64,
                ),
                (
                    "specialFoundCount".to_string(),
                    cleaner_rules.iter().filter(|rule| rule.selectable).count() as u64,
                ),
                (
                    "specialReadyCount".to_string(),
                    cleaner_rules
                        .iter()
                        .filter(|rule| {
                            matches!(
                                rule.status,
                                crate::cleanup::ScanItemStatus::Found
                                    | crate::cleanup::ScanItemStatus::Clean
                            )
                        })
                        .count() as u64,
                ),
                ("specialResultBytes".to_string(), cleaner_result_bytes),
                (
                    "specialLimitedCount".to_string(),
                    cleaner_rules
                        .iter()
                        .filter(|rule| rule.status == crate::cleanup::ScanItemStatus::Limited)
                        .count() as u64,
                ),
                (
                    "specialNotApplicableCount".to_string(),
                    cleaner_rules
                        .iter()
                        .filter(|rule| rule.status == crate::cleanup::ScanItemStatus::NotApplicable)
                        .count() as u64,
                ),
                (
                    "specialScanElapsedMs".to_string(),
                    cleaner_rules.iter().map(|rule| rule.scan_elapsed_ms).sum(),
                ),
            ]),
            expectation_met: true,
        });
    }
    let input_files = results
        .iter()
        .map(|run| run.files_visited)
        .max()
        .unwrap_or_default();
    let input_bytes = results
        .iter()
        .map(|run| run.bytes_observed)
        .max()
        .unwrap_or_default();
    let mut detail_metrics = rule_samples
        .into_iter()
        .map(|(id, samples)| {
            let first = samples.first().cloned().unwrap_or_default();
            BenchmarkDetailMetric {
                id,
                median_elapsed_ms: median(
                    &samples.iter().map(|sample| sample.0).collect::<Vec<_>>(),
                ),
                result_count: first.1,
                result_bytes: first.2,
                result_consistent_across_runs: samples.iter().all(|sample| {
                    sample.1 == first.1 && sample.2 == first.2 && sample.3 == first.3
                }),
            }
        })
        .collect::<Vec<_>>();
    detail_metrics.sort_by(|left, right| {
        right
            .median_elapsed_ms
            .cmp(&left.median_elapsed_ms)
            .then_with(|| left.id.cmp(&right.id))
    });
    Ok(ModuleBenchmarkReport {
        module: "deepClean",
        workload_kind: "environment",
        workload_digest,
        scan_mode: CleanupScanService::engine_info().strategy,
        fast_path: "notApplicable",
        expected_result: BenchmarkExpectation::default(),
        error_summary: None,
        summary: summarize_runs(&results, input_files, input_bytes),
        runs: results,
        detail_metrics,
        phase_notes: vec![
            "Deep clean uses the real rule catalog and user caches, so its performance is not compared with fixed-dataset modules.".to_string(),
            "ruleScan is the complete parallel rule scan; per-rule timings remain in the deep-clean baseline report.".to_string(),
            "The process snapshot runs in parallel with rule traversal; only processRunning applicability probes wait before traversal.".to_string(),
            "The applicability stage includes applicability preflight, volume identification, and scan-plan compilation.".to_string(),
            "resultDigest covers filesystem rules only; specialized cleaners are recorded independently through workMetrics and detailMetrics.".to_string(),
            format!(
                "The workload digest combines filesystem rule catalog {} and specialized cleaner registry {}.",
                filesystem_catalog_digest, cleaner_catalog_digest
            ),
            format!(
                "Rule coverage: {} declarative rules; lifecycle {:?}; metadata digest {}.",
                catalog.total_count, catalog.lifecycle_counts, catalog.metadata_digest
            ),
            format!(
                "Applicability inventory: {} applications, {} processes; preflight filtered {} inapplicable rules.",
                inventory_application_count, inventory_process_count, filtered_rule_count
            ),
        ],
    })
}

fn benchmark_analysis(
    runs: usize,
    root: &Path,
    workload_digest: &str,
    expected_files: u64,
    expected_bytes: u64,
) -> Result<ModuleBenchmarkReport, String> {
    let cache_reuse_supported = match current_platform().capture_filesystem_change_token(root) {
        Ok(token) => token.is_some(),
        Err(error) => {
            log::warn!(
                "engine_benchmark_analysis_cache_probe_failed diagnostic={}",
                error.diagnostic()
            );
            false
        }
    };
    let mut results = Vec::with_capacity(runs);
    let mut memory_reuse_samples = Vec::with_capacity(runs);
    let mut cache_validation_samples = Vec::with_capacity(runs);
    let mut session_restore_samples = Vec::with_capacity(runs);
    let mut session_validation_samples = Vec::with_capacity(runs);
    let mut fast_path_modes = Vec::with_capacity(runs);
    let mut strategies = Vec::new();
    let mut fallback_reasons = Vec::new();
    let mut layout_page_counts = Vec::with_capacity(runs);
    let mut layout_entry_counts = Vec::with_capacity(runs);
    let mut directory_counts = Vec::with_capacity(runs);
    let mut candidate_counts = Vec::with_capacity(runs);
    for run_number in 1..=runs {
        let capture = Arc::new(Mutex::new(ProgressCapture::default()));
        let callback_capture = Arc::clone(&capture);
        let started = Instant::now();
        let (result, diagnostics) = AnalysisService::analyze_with_diagnostics(
            Some(display_path(root)),
            true,
            move |progress| {
                if let Ok(mut capture) = callback_capture.lock() {
                    capture.record(progress);
                }
            },
        )
        .map_err(|error| error.to_string())?;
        let elapsed_ms = started.elapsed().as_millis() as u64;
        let capture = capture
            .lock()
            .map_err(|_| "failed to read disk-analysis benchmark progress".to_string())?;
        let result_count = result.entries.len() as u64;
        let result_digest = analysis_digest(&result);
        fast_path_modes.push(diagnostics.fast_path);
        if !diagnostics.strategy.is_empty() {
            strategies.push(diagnostics.strategy);
        }
        if let Some(reason) = diagnostics.fallback_reason {
            fallback_reasons.push(reason);
        }
        layout_page_counts.push(diagnostics.layout_page_count);
        layout_entry_counts.push(diagnostics.layout_entry_count);
        directory_counts.push(diagnostics.directory_count);
        candidate_counts.push(diagnostics.candidate_count);
        if cache_reuse_supported {
            // Scan results intentionally live only for the current process. Measure the navigation
            // path that product pages actually use instead of simulating a nonexistent persistent
            // restore. Platforms without reliable change history must rescan, so a cache miss is
            // expected there and is reported as an unsupported capability instead of a scan error.
            let restore_started = Instant::now();
            let (restored, restored_diagnostics) =
                AnalysisService::analyze_with_diagnostics(Some(display_path(root)), false, |_| {})
                    .map_err(|error| error.to_string())?;
            let restore_elapsed_ms = restore_started.elapsed().as_millis() as u64;
            let restore_digest = analysis_digest(&restored);
            if restore_digest != result_digest || restored_diagnostics.traversal_ms != 0 {
                return Err(
                    "disk-analysis memory reuse missed the snapshot or changed the result"
                        .to_string(),
                );
            }
            memory_reuse_samples.push((
                restore_elapsed_ms,
                restored.entries.len() as u64,
                restored.total_bytes,
                restore_digest,
            ));
            cache_validation_samples.push((
                restored_diagnostics.cache_validation_ms,
                restored.entries.len() as u64,
                restored.total_bytes,
                analysis_digest(&restored),
            ));
            // A repeated request verifies stable current-session reuse after the change monitor has
            // already been observed once.
            let session_restore_started = Instant::now();
            let (session_restored, session_diagnostics) =
                AnalysisService::analyze_with_diagnostics(Some(display_path(root)), false, |_| {})
                    .map_err(|error| error.to_string())?;
            let session_restore_elapsed_ms = session_restore_started.elapsed().as_millis() as u64;
            let session_digest = analysis_digest(&session_restored);
            if session_digest != result_digest || session_diagnostics.traversal_ms != 0 {
                return Err("disk-analysis repeated memory reuse changed the result".to_string());
            }
            session_restore_samples.push((
                session_restore_elapsed_ms,
                session_restored.entries.len() as u64,
                session_restored.total_bytes,
                session_digest.clone(),
            ));
            session_validation_samples.push((
                session_diagnostics.cache_validation_ms,
                session_restored.entries.len() as u64,
                session_restored.total_bytes,
                session_digest,
            ));
        }
        results.push(ModuleBenchmarkRun {
            run_number,
            first_progress_ms: clamp_elapsed(capture.first_progress_ms, elapsed_ms),
            first_result_ms: (result_count > 0).then_some(elapsed_ms),
            total_elapsed_ms: elapsed_ms,
            files_visited: capture.maximum_items_scanned,
            bytes_observed: capture.maximum_bytes_scanned,
            result_count,
            result_bytes: result.total_bytes,
            skipped_count: result.skipped_count,
            result_digest,
            phase_elapsed_ms: BTreeMap::from([
                (
                    "cacheValidation".to_string(),
                    diagnostics.cache_validation_ms,
                ),
                (
                    "enumerateAndAggregate".to_string(),
                    diagnostics.traversal_ms,
                ),
                ("cacheWrite".to_string(), diagnostics.cache_write_ms),
                ("resultBuild".to_string(), diagnostics.result_build_ms),
            ]),
            work_metrics: BTreeMap::new(),
            expectation_met: result.total_bytes == expected_bytes
                && capture.maximum_items_scanned == expected_files
                && capture.maximum_bytes_scanned == expected_bytes
                && result.skipped_count == 0,
        });
    }
    Ok(ModuleBenchmarkReport {
        module: "diskAnalysis",
        workload_kind: "fixedDataset",
        workload_digest: workload_digest.to_string(),
        scan_mode: "platformAggregateWithTraversalFallback",
        fast_path: aggregate_fast_path(&fast_path_modes),
        expected_result: BenchmarkExpectation {
            result_count: None,
            result_bytes: Some(expected_bytes),
        },
        error_summary: None,
        summary: summarize_runs(
            &results,
            results
                .iter()
                .map(|run| run.files_visited)
                .max()
                .unwrap_or_default(),
            results
                .iter()
                .map(|run| run.bytes_observed)
                .max()
                .unwrap_or_default(),
        ),
        runs: results,
        detail_metrics: if cache_reuse_supported {
            vec![
                restore_detail_metric("memoryReuse", &memory_reuse_samples),
                restore_detail_metric("cacheValidityCheck", &cache_validation_samples),
                restore_detail_metric("sessionCacheRestore", &session_restore_samples),
                restore_detail_metric(
                    "sessionCacheValidityCheck",
                    &session_validation_samples,
                ),
            ]
        } else {
            Vec::new()
        },
        phase_notes: vec![
            "Enumeration and directory aggregation are reported as enumerateAndAggregate. cacheWrite publishes the completed result to the process-scoped memory cache."
                .to_string(),
            if cache_reuse_supported {
                "memoryReuse and sessionCacheRestore measure current-session navigation after platform change-history validation."
                    .to_string()
            } else {
                "Memory reuse is unavailable because this platform or volume cannot provide a reliable filesystem change token; forced-refresh traversal remains benchmarked."
                    .to_string()
            },
            format!(
                "Platform aggregation strategy: {}; at most {} pages and {} layout records per run, producing {} directories and {} large-file candidates.",
                if strategies.is_empty() {
                    "generic traversal fallback".to_string()
                } else {
                    strategies.sort_unstable();
                    strategies.dedup();
                    strategies.join(", ")
                },
                layout_page_counts.into_iter().max().unwrap_or_default(),
                layout_entry_counts.into_iter().max().unwrap_or_default(),
                directory_counts.into_iter().max().unwrap_or_default(),
                candidate_counts.into_iter().max().unwrap_or_default(),
            ),
            format!(
                "Fast-path fallback reason: {}.",
                if fallback_reasons.is_empty() {
                    "none".to_string()
                } else {
                    fallback_reasons.sort_unstable();
                    fallback_reasons.dedup();
                    fallback_reasons.join(", ")
                }
            ),
        ],
    })
}

fn benchmark_large_files(
    runs: usize,
    root: &Path,
    workload_digest: &str,
    expected_count: u64,
    expected_bytes: u64,
) -> Result<ModuleBenchmarkReport, String> {
    let mut results = Vec::with_capacity(runs);
    let mut filter_samples = Vec::with_capacity(runs);
    let mut fast_path_modes = Vec::with_capacity(runs);
    let mut fallback_reasons = Vec::new();
    let mut candidate_counts = Vec::with_capacity(runs);
    let mut candidate_peak_in_flight = Vec::with_capacity(runs);
    let mut candidate_strategies = Vec::with_capacity(runs);
    for run_number in 1..=runs {
        let capture = Arc::new(Mutex::new(ProgressCapture::default()));
        let callback_capture = Arc::clone(&capture);
        let started = Instant::now();
        let (result, diagnostics) = LargeFileService::find_with_diagnostics(
            vec![display_path(root)],
            LARGE_FILE_MINIMUM_BYTES,
            LargeFileScanMode::Complete,
            vec![],
            move |progress| {
                if let Ok(mut capture) = callback_capture.lock() {
                    capture.record(progress);
                }
            },
        )
        .map_err(|error| error.to_string())?;
        let elapsed_ms = started.elapsed().as_millis() as u64;
        let capture = capture
            .lock()
            .map_err(|_| "failed to read large-file benchmark progress".to_string())?;
        fast_path_modes.push(diagnostics.fast_path);
        if let Some(reason) = diagnostics.fallback_reason {
            fallback_reasons.push(reason);
        }
        candidate_counts.push(diagnostics.candidate_count);
        candidate_peak_in_flight.push(diagnostics.candidate_peak_in_flight);
        if !diagnostics.candidate_strategy.is_empty() {
            candidate_strategies.push(diagnostics.candidate_strategy);
        }
        let result_digest = large_file_digest(&result, root);
        let filter_started = Instant::now();
        let filtered = result.filtered(LARGE_FILE_MINIMUM_BYTES);
        let filter_elapsed_ms = filter_started.elapsed().as_millis() as u64;
        let filter_digest = large_file_digest(&filtered, root);
        if filter_digest != result_digest {
            return Err("large-file in-memory filtering changed the scan result".to_string());
        }
        filter_samples.push((
            filter_elapsed_ms,
            filtered.total_count,
            filtered.total_bytes,
            filter_digest,
        ));
        results.push(ModuleBenchmarkRun {
            run_number,
            first_progress_ms: clamp_elapsed(capture.first_progress_ms, elapsed_ms),
            first_result_ms: (result.total_count > 0).then_some(elapsed_ms),
            total_elapsed_ms: elapsed_ms,
            files_visited: capture.maximum_items_scanned,
            bytes_observed: capture.maximum_bytes_scanned,
            result_count: result.total_count,
            result_bytes: result.total_bytes,
            skipped_count: result.skipped_count,
            result_digest,
            phase_elapsed_ms: BTreeMap::from([
                (
                    "candidateDiscovery".to_string(),
                    diagnostics.candidate_discovery_ms,
                ),
                (
                    "candidateBackpressure".to_string(),
                    diagnostics.candidate_backpressure_ms,
                ),
                (
                    "validateOrTraverse".to_string(),
                    diagnostics.validation_or_traversal_ms,
                ),
                ("resultSort".to_string(), diagnostics.result_build_ms),
            ]),
            work_metrics: BTreeMap::new(),
            expectation_met: result.total_count == expected_count
                && result.total_bytes == expected_bytes
                && result.skipped_count == 0,
        });
    }
    candidate_strategies.sort_unstable();
    candidate_strategies.dedup();
    let candidate_note = format!(
        "Candidate-stream strategy: {}; at most {} records per run and {} peak in-flight records from Platform to Core.",
        if candidate_strategies.is_empty() {
            "generic traversal fallback".to_string()
        } else {
            candidate_strategies.join(", ")
        },
        candidate_counts.into_iter().max().unwrap_or_default(),
        candidate_peak_in_flight
            .into_iter()
            .max()
            .unwrap_or_default()
    );
    let timing_note =
        "candidateDiscovery is the end-to-end wall time for platform enumeration and Core validation. validateOrTraverse accumulates consumer time, while candidateBackpressure accumulates producer wait time. These metrics overlap and must not be added. cacheWrite publishes the completed result to process memory."
            .to_string();
    Ok(ModuleBenchmarkReport {
        module: "largeFiles",
        workload_kind: "fixedDataset",
        workload_digest: workload_digest.to_string(),
        scan_mode: "platformCandidatesWithTraversalFallback",
        fast_path: aggregate_fast_path(&fast_path_modes),
        expected_result: BenchmarkExpectation {
            result_count: Some(expected_count),
            result_bytes: Some(expected_bytes),
        },
        error_summary: None,
        summary: summarize_runs(
            &results,
            results
                .iter()
                .map(|run| run.files_visited)
                .max()
                .unwrap_or_default(),
            results
                .iter()
                .map(|run| run.bytes_observed)
                .max()
                .unwrap_or_default(),
        ),
        runs: results,
        detail_metrics: vec![restore_detail_metric("thresholdFilter", &filter_samples)],
        phase_notes: if fallback_reasons.is_empty() {
            vec![
                timing_note,
                "thresholdFilter applies the selected minimum size to the active in-memory candidate snapshot without filesystem traversal or change-history tracking."
                    .to_string(),
                candidate_note,
            ]
        } else {
            fallback_reasons.sort_unstable();
            fallback_reasons.dedup();
            vec![
                format!(
                    "Platform fast-path fallback reason: {}; fallback traversal is recorded in validateOrTraverse.",
                    fallback_reasons.join(", ")
                ),
                timing_note,
                "thresholdFilter applies the selected minimum size to the active in-memory candidate snapshot without filesystem traversal or change-history tracking."
                    .to_string(),
                candidate_note,
            ]
        },
    })
}

fn benchmark_duplicate_files(
    runs: usize,
    root: &Path,
    workload_digest: &str,
    expected_groups: u64,
    expected_duplicate_files: u64,
    expected_reclaimable_bytes: u64,
) -> Result<ModuleBenchmarkReport, String> {
    let mut results = Vec::with_capacity(runs);
    let mut sample_plan_name = "unknown";
    let mut cache_hit_observed = false;
    for run_number in 1..=runs {
        let capture = Arc::new(Mutex::new(ProgressCapture::default()));
        let callback_capture = Arc::clone(&capture);
        let group_callback_capture = Arc::clone(&capture);
        let started = Instant::now();
        let (result, diagnostics) = DuplicateFileService::find_with_stream_diagnostics(
            vec![display_path(root)],
            DUPLICATE_MINIMUM_BYTES,
            move |progress| {
                if let Ok(mut capture) = callback_capture.lock() {
                    capture.record(progress);
                }
            },
            move |batch| {
                if let Ok(mut capture) = group_callback_capture.lock() {
                    capture.record_duplicate_group(batch.elapsed_ms);
                }
            },
        )
        .map_err(|error| error.to_string())?;
        sample_plan_name = diagnostics.sample_plan;
        cache_hit_observed |= diagnostics.sample_hash_cache_hit_count > 0
            || diagnostics.full_hash_cache_hit_count > 0;
        let elapsed_ms = started.elapsed().as_millis() as u64;
        let capture = capture
            .lock()
            .map_err(|_| "failed to read duplicate-file benchmark progress".to_string())?;
        results.push(ModuleBenchmarkRun {
            run_number,
            first_progress_ms: clamp_elapsed(capture.first_progress_ms, elapsed_ms),
            first_result_ms: clamp_elapsed(capture.first_result_ms, elapsed_ms),
            total_elapsed_ms: elapsed_ms,
            files_visited: result.scanned_file_count,
            bytes_observed: capture.maximum_bytes_scanned,
            result_count: result.total_group_count,
            result_bytes: result.reclaimable_bytes,
            skipped_count: result.skipped_count,
            result_digest: duplicate_digest(&result, root),
            phase_elapsed_ms: BTreeMap::from([
                (
                    "enumerateAndSizeGroup".to_string(),
                    diagnostics.enumeration_and_size_group_ms,
                ),
                (
                    "groupAndIdentity".to_string(),
                    diagnostics.group_and_identity_ms,
                ),
                ("cacheLoad".to_string(), diagnostics.cache_load_ms),
                (
                    "cacheValidation".to_string(),
                    diagnostics.cache_validation_ms,
                ),
                ("sampleHash".to_string(), diagnostics.sample_hash_ms),
                ("fullHash".to_string(), diagnostics.full_hash_ms),
                ("cacheWrite".to_string(), diagnostics.cache_write_ms),
                ("resultSort".to_string(), diagnostics.result_sort_ms),
            ]),
            work_metrics: BTreeMap::from([
                (
                    "sizeGroupCandidates".to_string(),
                    diagnostics.size_group_candidate_count,
                ),
                (
                    "physicalAliasesFiltered".to_string(),
                    diagnostics.physical_alias_filtered_count,
                ),
                (
                    "identityUnavailable".to_string(),
                    diagnostics.identity_unavailable_count,
                ),
                (
                    "identityWorkers".to_string(),
                    diagnostics.identity_worker_count,
                ),
                (
                    "identityPeakInFlight".to_string(),
                    diagnostics.identity_peak_in_flight,
                ),
                ("identityHints".to_string(), diagnostics.identity_hint_count),
                (
                    "identityHintsVerified".to_string(),
                    diagnostics.identity_hint_verified_count,
                ),
                (
                    "identityHintFallbackDirectories".to_string(),
                    diagnostics.identity_hint_fallback_directory_count,
                ),
                (
                    "sampleHashCandidates".to_string(),
                    diagnostics.sample_hash_candidate_count,
                ),
                ("sampleHashBytes".to_string(), diagnostics.sample_hash_bytes),
                (
                    "fullHashCandidates".to_string(),
                    diagnostics.full_hash_candidate_count,
                ),
                ("fullHashBytes".to_string(), diagnostics.full_hash_bytes),
                (
                    "fullySparseCandidates".to_string(),
                    diagnostics.fully_sparse_candidate_count,
                ),
                (
                    "fullySparseGroups".to_string(),
                    diagnostics.fully_sparse_group_count,
                ),
                (
                    "fullySparseLogicalBytesSkipped".to_string(),
                    diagnostics.fully_sparse_logical_bytes_skipped,
                ),
                (
                    "allocatedRangeQueryFallbacks".to_string(),
                    diagnostics.allocated_range_query_fallback_count,
                ),
                (
                    "sampleHashWorkers".to_string(),
                    diagnostics.sample_hash_worker_count,
                ),
                (
                    "sampleHashPeakInFlight".to_string(),
                    diagnostics.sample_hash_peak_in_flight,
                ),
                (
                    "fullHashWorkers".to_string(),
                    diagnostics.full_hash_worker_count,
                ),
                (
                    "fullHashPeakInFlight".to_string(),
                    diagnostics.full_hash_peak_in_flight,
                ),
                (
                    "hashResultQueueCapacity".to_string(),
                    diagnostics.hash_result_queue_capacity,
                ),
                (
                    "cacheSnapshotFound".to_string(),
                    diagnostics.cache_snapshot_found,
                ),
                (
                    "cacheCandidateMatches".to_string(),
                    diagnostics.cache_candidate_match_count,
                ),
                (
                    "sampleHashCacheHits".to_string(),
                    diagnostics.sample_hash_cache_hit_count,
                ),
                (
                    "fullHashCacheHits".to_string(),
                    diagnostics.full_hash_cache_hit_count,
                ),
                (
                    "cacheFallbacks".to_string(),
                    diagnostics.cache_fallback_count,
                ),
                (
                    "cacheWriteEntries".to_string(),
                    diagnostics.cache_write_entry_count,
                ),
                (
                    "streamedGroupBatches".to_string(),
                    diagnostics.streamed_group_batch_count,
                ),
                (
                    "streamedGroups".to_string(),
                    diagnostics.streamed_group_count,
                ),
                (
                    "firstStreamedGroupMs".to_string(),
                    diagnostics.first_streamed_group_ms.unwrap_or_default(),
                ),
                (
                    "progressEnumerationBytes".to_string(),
                    capture.maximum_enumeration_bytes,
                ),
                (
                    "progressHashReadBytes".to_string(),
                    capture.maximum_hash_read_bytes,
                ),
                (
                    "progressHashCompletedSteps".to_string(),
                    capture.maximum_hash_completed_steps,
                ),
                (
                    "progressHashTotalSteps".to_string(),
                    capture.maximum_hash_total_steps,
                ),
            ]),
            expectation_met: result.total_group_count == expected_groups
                && result.duplicate_file_count == expected_duplicate_files
                && result.reclaimable_bytes == expected_reclaimable_bytes,
        });
    }
    Ok(ModuleBenchmarkReport {
        module: "duplicateFiles",
        workload_kind: "fixedDataset",
        workload_digest: workload_digest.to_string(),
        scan_mode: "sizeThenSampleThenFullHash",
        fast_path: if cache_hit_observed {
            "memoryHashCache"
        } else {
            "freshHash"
        },
        expected_result: BenchmarkExpectation {
            result_count: Some(expected_groups),
            result_bytes: Some(expected_reclaimable_bytes),
        },
        error_summary: None,
        summary: summarize_runs(
            &results,
            results
                .iter()
                .map(|run| run.files_visited)
                .max()
                .unwrap_or_default(),
            results
                .iter()
                .map(|run| run.bytes_observed)
                .max()
                .unwrap_or_default(),
        ),
        runs: results,
        detail_metrics: Vec::new(),
        phase_notes: vec![
            format!(
                "Sampling scheme: {}; enumeration, cache validation, sampling, full hashing, and sorting are timed separately. The hash stage records bounded-worker-pool wall time.",
                sample_plan_name
            ),
            "Workload metrics record bytes read, candidates, cache hits, fallbacks, workers, and peak in-flight tasks. A cached full hash can authorize a duplicate match only after continuous change-history validation."
                .to_string(),
            "Progress metrics keep enumeration bytes separate from bytes actually read during each hash stage; hash completed/total steps expose determinate UI progress without changing duplicate identity semantics."
                .to_string(),
            "firstResult comes from a real full-hash grouping callback. Streaming sends only a bounded initial set; paged sessions serve the final stable result on demand."
                .to_string(),
        ],
    })
}

fn restore_detail_metric(id: &str, samples: &[(u64, u64, u64, String)]) -> BenchmarkDetailMetric {
    let first = samples.first().cloned().unwrap_or_default();
    BenchmarkDetailMetric {
        id: id.to_string(),
        median_elapsed_ms: median(&samples.iter().map(|sample| sample.0).collect::<Vec<_>>()),
        result_count: first.1,
        result_bytes: first.2,
        result_consistent_across_runs: samples
            .iter()
            .all(|sample| sample.1 == first.1 && sample.2 == first.2 && sample.3 == first.3),
    }
}

/// Preserves measurements from successful modules when one scanner fails, so a cross-platform run
/// retains diagnostic value. Raw errors can contain user paths; reports and logs store only a
/// stable error code and an irreversible digest.
fn failed_module(
    module: &'static str,
    workload_kind: &'static str,
    workload_digest: &str,
    scan_mode: &'static str,
    expected_result: BenchmarkExpectation,
    error: &str,
) -> ModuleBenchmarkReport {
    let digest = blake3::hash(error.as_bytes()).to_hex().to_string();
    log::error!(
        "engine_benchmark_module_failed module={module} error={}",
        mangodisk_platform::diagnostics::text(error)
    );
    ModuleBenchmarkReport {
        module,
        workload_kind,
        workload_digest: workload_digest.to_string(),
        scan_mode,
        fast_path: "unknown",
        expected_result,
        error_summary: Some(BenchmarkErrorSummary {
            code: "scanFailed",
            digest,
        }),
        summary: summarize_runs(&[], 0, 0),
        runs: Vec::new(),
        detail_metrics: Vec::new(),
        phase_notes: vec![
            "Module execution failed. The report stores only an error digest and never persists raw errors that may contain user paths.".to_string(),
        ],
    }
}

fn validate_options(options: &EngineBenchmarkOptions) -> Result<(), String> {
    if options.runs == 0 || options.runs > MAX_BENCHMARK_RUNS {
        return Err(format!(
            "engine benchmark run count must be between 1 and {MAX_BENCHMARK_RUNS}"
        ));
    }
    if options.label.is_empty()
        || options.label.chars().count() > MAX_LABEL_CHARACTERS
        || !options
            .label
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || ".-_".contains(character))
    {
        return Err(
            "engine benchmark label must contain 1-100 ASCII letters, digits, dots, hyphens, or underscores"
                .to_string(),
        );
    }
    if options
        .note
        .as_ref()
        .is_some_and(|note| note.chars().count() > MAX_NOTE_CHARACTERS)
    {
        return Err(format!(
            "engine benchmark note cannot exceed {MAX_NOTE_CHARACTERS} characters"
        ));
    }
    Ok(())
}

fn clamp_elapsed(value: Option<u64>, total_elapsed_ms: u64) -> Option<u64> {
    value.map(|milliseconds| milliseconds.min(total_elapsed_ms))
}

fn aggregate_fast_path(modes: &[&'static str]) -> &'static str {
    let Some(first) = modes.first().copied() else {
        return "unknown";
    };
    if modes.iter().all(|mode| *mode == first) {
        first
    } else {
        "mixed"
    }
}

fn median(values: &[u64]) -> u64 {
    if values.is_empty() {
        return 0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let middle = sorted.len() / 2;
    if sorted.len().is_multiple_of(2) {
        sorted[middle - 1].saturating_add(sorted[middle]) / 2
    } else {
        sorted[middle]
    }
}

fn validate_dataset_root(manifest_path: &Path, root: &Path) -> Result<(), String> {
    let manifest_parent = manifest_path
        .parent()
        .ok_or_else(|| "benchmark dataset manifest has no parent directory".to_string())?;
    let manifest_parent = fs::canonicalize(manifest_parent)
        .map_err(|error| format!("failed to canonicalize benchmark dataset directory: {error}"))?;
    let root = fs::canonicalize(root)
        .map_err(|error| format!("failed to access benchmark dataset root: {error}"))?;
    if root.parent() != Some(manifest_parent.as_path()) {
        return Err(
            "benchmark dataset root is not an immediate child of the manifest directory"
                .to_string(),
        );
    }
    Ok(())
}

fn cleanup_digest(snapshot: &CleanupScanResult) -> String {
    let mut rules = snapshot
        .rules
        .iter()
        .filter(|rule| !crate::cleanup::cleaners::contains(&rule.rule_id))
        .collect::<Vec<_>>();
    rules.sort_by(|left, right| left.rule_id.cmp(&right.rule_id));
    let mut hasher = blake3::Hasher::new();
    for rule in rules {
        hasher.update(rule.rule_id.as_bytes());
        hasher.update(&rule.bytes.to_le_bytes());
        hasher.update(&rule.file_count.to_le_bytes());
        hasher.update(format!("{:?}", rule.status).as_bytes());
    }
    hasher.finalize().to_hex().to_string()
}

fn cleanup_workload_digest(filesystem_digest: &str, special_digest: &str) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(filesystem_digest.as_bytes());
    hasher.update(special_digest.as_bytes());
    hasher.finalize().to_hex().to_string()
}

fn analysis_digest(result: &AnalysisResult) -> String {
    let mut entries = result.entries.iter().collect::<Vec<_>>();
    entries.sort_by(|left, right| left.name.cmp(&right.name));
    let mut hasher = blake3::Hasher::new();
    hasher.update(&result.total_bytes.to_le_bytes());
    hasher.update(&result.skipped_count.to_le_bytes());
    for entry in entries {
        hasher.update(entry.name.as_bytes());
        hasher.update(&entry.bytes.to_le_bytes());
        hasher.update(&entry.file_count.to_le_bytes());
        hasher.update(&[u8::from(entry.is_directory)]);
        // Directory fingerprints include modification times for product cache invalidation, but
        // equal fixed content receives different timestamps on macOS and Windows. This result
        // digest covers only cross-platform stable names, types, counts, and bytes; the separate
        // logicalDigest verifies the actual dataset content.
    }
    hasher.finalize().to_hex().to_string()
}

fn large_file_digest(result: &LargeFilesResult, root: &Path) -> String {
    let mut entries = result
        .entries
        .iter()
        .map(|entry| (relative_path(root, &entry.path), entry.bytes))
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    let mut hasher = blake3::Hasher::new();
    for (relative_path, bytes) in entries {
        hasher.update(relative_path.as_bytes());
        hasher.update(&bytes.to_le_bytes());
    }
    hasher.finalize().to_hex().to_string()
}

fn duplicate_digest(result: &DuplicateFilesResult, root: &Path) -> String {
    let mut groups = result.groups.iter().collect::<Vec<_>>();
    groups.sort_by(|left, right| left.hash.cmp(&right.hash));
    let mut hasher = blake3::Hasher::new();
    for group in groups {
        hasher.update(group.hash.as_bytes());
        hasher.update(&group.bytes_per_file.to_le_bytes());
        let mut entries = group
            .entries
            .iter()
            .map(|entry| relative_path(root, &entry.path))
            .collect::<Vec<_>>();
        entries.sort();
        for relative_path in entries {
            hasher.update(relative_path.as_bytes());
        }
    }
    hasher.finalize().to_hex().to_string()
}

fn relative_path(root: &Path, value: &str) -> String {
    let root = normalized_path_text(root);
    let value = normalized_path_text(Path::new(value));
    value
        .strip_prefix(&root)
        .and_then(|suffix| suffix.strip_prefix('/').or(Some(suffix)))
        .unwrap_or(&value)
        .to_string()
}

fn normalized_path_text(path: &Path) -> String {
    current_platform()
        .path_identity_key(path)
        .replace('\\', "/")
}

fn sanitize_label(label: &str) -> String {
    label
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || "-_.".contains(character) {
                character
            } else {
                '-'
            }
        })
        .collect()
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_else(|| "unknown".to_string())
}

fn write_atomic(path: &Path, content: &[u8]) -> Result<(), String> {
    let temporary = path.with_extension(format!("tmp-{}-{}", std::process::id(), now_ms()));
    fs::write(&temporary, content).map_err(|error| {
        format!(
            "failed to write temporary report {}: {error}",
            temporary.display()
        )
    })?;
    if let Err(error) = fs::rename(&temporary, path) {
        let _ = fs::remove_file(&temporary);
        return Err(format!(
            "failed to save engine benchmark report {}: {error}",
            path.display()
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::relative_path;
    use std::path::Path;

    #[cfg(windows)]
    #[test]
    fn benchmark_digest_normalizes_windows_prefix_and_casing() {
        assert_eq!(
            relative_path(
                Path::new(r"\\?\C:\Data\Fixed"),
                r"c:\data\fixed\Folder\File.bin"
            ),
            "folder/file.bin"
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn benchmark_digest_uses_stable_relative_path() {
        assert_eq!(
            relative_path(
                Path::new("/tmp/mangodisk-fixed"),
                "/tmp/mangodisk-fixed/folder/file.bin"
            ),
            "folder/file.bin"
        );
    }
}
