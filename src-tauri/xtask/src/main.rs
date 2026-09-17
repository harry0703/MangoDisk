use std::{
    env,
    path::PathBuf,
    process::Command,
    sync::{Arc, Mutex},
    time::Instant,
};

use mangodisk_core::{
    configure_application_paths, ApplicationPaths, ApplicationUninstallService,
    BaselineComparisonOptions, BenchmarkDatasetOptions, BenchmarkDatasetService,
    BenchmarkSourceInfo, CleanupBaselineComparisonService, CleanupBaselineOptions,
    CleanupBaselineService, CoreErrorCode, EngineBenchmarkComparisonOptions,
    EngineBenchmarkComparisonService, EngineBenchmarkOptions, EngineBenchmarkService,
    APPLICATION_IDENTIFIER,
};
use mangodisk_platform::application_directories;

const BASELINE_FLAG: &str = "--cleanup-baseline";
const COMPARISON_FLAG: &str = "--compare-cleanup-baselines";
const DATASET_FLAG: &str = "--generate-benchmark-dataset";
const ENGINE_BENCHMARK_FLAG: &str = "--engine-benchmark";
const ENGINE_COMPARISON_FLAG: &str = "--compare-engine-benchmarks";
const APPLICATION_UNINSTALL_BENCHMARK_FLAG: &str = "--application-uninstall-benchmark";
const DEFAULT_LABEL: &str = "manual";
const DEFAULT_RUNS: usize = 3;

#[derive(Debug)]
struct BaselineCliOptions {
    label: String,
    environment_id: Option<String>,
    note: Option<String>,
    runs: usize,
    output_directory: Option<PathBuf>,
    project_roots: Vec<String>,
    deep_project_discovery: bool,
}

#[derive(Debug)]
struct BaselineComparisonCliOptions {
    baseline_path: PathBuf,
    candidate_path: PathBuf,
    output_path: PathBuf,
}

#[derive(Debug)]
struct BenchmarkDatasetCliOptions {
    output_directory: PathBuf,
    seed: Option<u64>,
    recreate: bool,
}

#[derive(Debug)]
struct EngineBenchmarkCliOptions {
    label: String,
    environment_id: Option<String>,
    note: Option<String>,
    runs: usize,
    dataset_manifest_path: PathBuf,
    output_directory: PathBuf,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("MangoDisk repository task failed: {error}");
        std::process::exit(1);
    }
}

/// Dispatches repository-only reporting and benchmark tasks without linking
/// Tauri or starting a WebView. Product CLI behavior belongs to `mangodisk-cli`.
fn run() -> Result<(), String> {
    configure_storage()?;
    let arguments = env::args().skip(1).collect::<Vec<_>>();
    if arguments
        .iter()
        .any(|argument| matches!(argument.as_str(), "--help" | "-h"))
    {
        println!("{}", help_text());
        return Ok(());
    }
    if arguments
        .iter()
        .any(|argument| argument == ENGINE_COMPARISON_FLAG)
    {
        let options = parse_engine_comparison_options(&arguments)?;
        let artifacts =
            EngineBenchmarkComparisonService::compare(EngineBenchmarkComparisonOptions {
                baseline_path: options.baseline_path,
                candidate_path: options.candidate_path,
                output_path: options.output_path,
            })?;
        println!("Markdown: {}", artifacts.markdown_path.display());
        return Ok(());
    }
    if arguments
        .iter()
        .any(|argument| argument == APPLICATION_UNINSTALL_BENCHMARK_FLAG)
    {
        run_application_uninstall_benchmark(parse_application_uninstall_benchmark_runs(
            &arguments,
        )?)?;
        return Ok(());
    }
    if arguments
        .iter()
        .any(|argument| argument == ENGINE_BENCHMARK_FLAG)
    {
        let options = parse_engine_benchmark_options(&arguments)?;
        let artifacts = EngineBenchmarkService::generate(EngineBenchmarkOptions {
            label: options.label,
            environment_id: options.environment_id,
            note: options.note,
            runs: options.runs,
            dataset_manifest_path: options.dataset_manifest_path,
            output_directory: options.output_directory,
            source: source_info()?,
        })?;
        println!("JSON: {}", artifacts.json_path.display());
        println!("Markdown: {}", artifacts.markdown_path.display());
        return Ok(());
    }
    if arguments.iter().any(|argument| argument == DATASET_FLAG) {
        let options = parse_dataset_options(&arguments)?;
        let artifacts = BenchmarkDatasetService::generate(BenchmarkDatasetOptions {
            parent_directory: options.output_directory,
            seed: options.seed,
            recreate: options.recreate,
        })?;
        println!("Dataset: {}", artifacts.dataset_directory.display());
        println!("JSON: {}", artifacts.manifest_path.display());
        println!("Markdown: {}", artifacts.markdown_path.display());
        return Ok(());
    }
    if arguments.iter().any(|argument| argument == COMPARISON_FLAG) {
        let options = parse_comparison_options(&arguments)?;
        let artifacts = CleanupBaselineComparisonService::compare(BaselineComparisonOptions {
            baseline_path: options.baseline_path,
            candidate_path: options.candidate_path,
            output_path: options.output_path,
        })?;
        println!("Markdown: {}", artifacts.markdown_path.display());
        return Ok(());
    }
    if !arguments.iter().any(|argument| argument == BASELINE_FLAG) {
        return Err(help_text().to_string());
    }

    let options = parse_options(&arguments)?;
    let artifacts = CleanupBaselineService::generate(CleanupBaselineOptions {
        label: options.label,
        environment_id: options.environment_id,
        note: options.note,
        runs: options.runs,
        output_directory: options.output_directory,
        project_roots: options.project_roots,
        deep_project_discovery: options.deep_project_discovery,
        source: source_info()?,
    })?;
    println!("JSON: {}", artifacts.json_path.display());
    println!("Markdown: {}", artifacts.markdown_path.display());
    Ok(())
}

fn parse_application_uninstall_benchmark_runs(arguments: &[String]) -> Result<usize, String> {
    let mut runs = 6_usize;
    let mut index = 0;
    while index < arguments.len() {
        match arguments[index].as_str() {
            APPLICATION_UNINSTALL_BENCHMARK_FLAG | "--" => {}
            "--runs" => {
                let value = next_value(arguments, &mut index, "--runs")?;
                runs = value
                    .parse()
                    .map_err(|_| format!("--runs is not a valid integer: {value}"))?;
            }
            argument => {
                return Err(format!(
                    "unsupported application uninstall benchmark argument: {argument}\n\n{}",
                    help_text()
                ));
            }
        }
        index += 1;
    }
    if !(2..=10).contains(&runs) {
        return Err("application uninstall benchmark runs must be from 2 to 10".to_string());
    }
    Ok(runs)
}

/// Measures one cold inventory build followed by hot revision-cache scans in the same process.
/// Only aggregate counts and an irreversible result digest leave the task, so benchmark output can
/// be retained under `.local/reports` without disclosing application names or private paths.
fn run_application_uninstall_benchmark(runs: usize) -> Result<(), String> {
    let mut durations = Vec::with_capacity(runs);
    let mut digests = Vec::with_capacity(runs);
    let mut inventory_completeness = Vec::with_capacity(runs);
    let mut first_progress_values = Vec::with_capacity(runs);
    for run_index in 0..runs {
        let started = Instant::now();
        let first_progress = Arc::new(Mutex::new(None));
        let callback_progress = Arc::clone(&first_progress);
        let scan = ApplicationUninstallService::scan_with_progress(move |_| {
            if let Ok(mut first) = callback_progress.lock() {
                first.get_or_insert_with(|| started.elapsed().as_millis() as u64);
            }
        })
        .map_err(|error| error.to_string())?;
        let elapsed_ms = started.elapsed().as_millis() as u64;
        let first_progress_ms = first_progress
            .lock()
            .ok()
            .and_then(|value| *value)
            .unwrap_or(elapsed_ms);
        let component_count = scan
            .candidates
            .iter()
            .map(|candidate| candidate.components.len() as u64)
            .sum::<u64>();
        let file_count = scan
            .candidates
            .iter()
            .flat_map(|candidate| &candidate.components)
            .map(|component| component.file_count)
            .sum::<u64>();
        let bytes = scan
            .candidates
            .iter()
            .map(|candidate| candidate.total_bytes)
            .sum::<u64>();
        let incomplete_count = scan
            .candidates
            .iter()
            .filter(|candidate| !candidate.associated_data_complete)
            .count();
        let serialized = serde_json::to_vec(&scan.candidates)
            .map_err(|error| format!("failed to serialize benchmark result: {error}"))?;
        let digest = blake3::hash(&serialized).to_hex().to_string();
        println!(
            "application_uninstall_benchmark_run run={} cold={} inventory_complete={} candidate_count={} component_count={} file_count={} bytes={} incomplete_count={} first_progress_ms={} elapsed_ms={} result_digest={}",
            run_index + 1,
            run_index == 0,
            scan.inventory_complete,
            scan.candidates.len(),
            component_count,
            file_count,
            bytes,
            incomplete_count,
            first_progress_ms,
            elapsed_ms,
            digest,
        );
        durations.push(elapsed_ms);
        digests.push(digest);
        inventory_completeness.push(scan.inventory_complete);
        first_progress_values.push(first_progress_ms);
    }
    if inventory_completeness.iter().any(|complete| !complete) {
        return Err(
            "application uninstall benchmark produced an incomplete inventory snapshot".to_string(),
        );
    }
    let mut repeated = durations[1..].to_vec();
    repeated.sort_unstable();
    let repeated_median_ms = median(&repeated);
    let repeated_p95_ms = percentile_95(&repeated);
    let first_progress_p95_ms = percentile_95(&first_progress_values);
    let result_consistent = digests.windows(2).all(|pair| pair[0] == pair[1]);
    let cancellation_latencies = application_uninstall_cancellation_latencies(runs)?;
    let cancellation_p95_ms = percentile_95(&cancellation_latencies);
    println!(
        "application_uninstall_benchmark_summary runs={} cold_ms={} repeated_median_ms={} repeated_p95_ms={} first_progress_p95_ms={} cancellation_p95_ms={} result_consistent={}",
        runs,
        durations[0],
        repeated_median_ms,
        repeated_p95_ms,
        first_progress_p95_ms,
        cancellation_p95_ms,
        result_consistent,
    );
    Ok(())
}

/// Cancels from the first progress callback so every sample exercises the
/// adapter-visible cancellation boundary deterministically, including the
/// revision and inventory hand-off that follows the initial event.
fn application_uninstall_cancellation_latencies(runs: usize) -> Result<Vec<u64>, String> {
    let mut latencies = Vec::with_capacity(runs);
    for run_index in 0..runs {
        let requested_at = Arc::new(Mutex::new(None));
        let callback_requested_at = Arc::clone(&requested_at);
        let result = ApplicationUninstallService::scan_with_progress(move |_| {
            if let Ok(mut requested_at) = callback_requested_at.lock() {
                if requested_at.is_none() {
                    *requested_at = Some(Instant::now());
                    ApplicationUninstallService::cancel_scan();
                }
            }
        });
        match result {
            Err(error) if error.code() == CoreErrorCode::OperationCancelled => {}
            Err(error) => {
                return Err(format!(
                    "application uninstall cancellation benchmark failed unexpectedly: {error}"
                ));
            }
            Ok(_) => {
                return Err(
                    "application uninstall cancellation benchmark unexpectedly completed"
                        .to_string(),
                );
            }
        }
        let latency_ms = requested_at
            .lock()
            .ok()
            .and_then(|value| *value)
            .map(|requested_at| requested_at.elapsed().as_millis() as u64)
            .ok_or_else(|| {
                "application uninstall cancellation benchmark emitted no progress".to_string()
            })?;
        println!(
            "application_uninstall_cancellation_run run={} latency_ms={}",
            run_index + 1,
            latency_ms
        );
        latencies.push(latency_ms);
    }
    Ok(latencies)
}

fn median(sorted: &[u64]) -> u64 {
    let middle = sorted.len() / 2;
    if sorted.len().is_multiple_of(2) {
        sorted[middle - 1]
            .saturating_add(sorted[middle])
            .saturating_div(2)
    } else {
        sorted[middle]
    }
}

fn percentile_95(values: &[u64]) -> u64 {
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let index = (sorted.len() * 95).div_ceil(100).saturating_sub(1);
    sorted[index]
}

fn configure_storage() -> Result<(), String> {
    let directories =
        application_directories(APPLICATION_IDENTIFIER).map_err(|error| error.to_string())?;
    let paths = ApplicationPaths::from_base_directories(
        directories.local_data_directory,
        directories.cache_directory,
    )
    .map_err(|error| error.to_string())?;
    configure_application_paths(paths).map_err(|error| error.to_string())
}

fn parse_engine_comparison_options(
    arguments: &[String],
) -> Result<BaselineComparisonCliOptions, String> {
    parse_comparison_paths(arguments, ENGINE_COMPARISON_FLAG, "engine benchmark")
}

fn parse_engine_benchmark_options(
    arguments: &[String],
) -> Result<EngineBenchmarkCliOptions, String> {
    let mut label = DEFAULT_LABEL.to_string();
    let mut environment_id = None;
    let mut note = None;
    let mut runs = DEFAULT_RUNS;
    let mut dataset_manifest_path = None;
    let mut output_directory = None;
    let mut index = 0;
    while index < arguments.len() {
        match arguments[index].as_str() {
            ENGINE_BENCHMARK_FLAG | "--" => {}
            "--label" => label = next_value(arguments, &mut index, "--label")?,
            "--environment-id" => {
                environment_id = Some(next_value(arguments, &mut index, "--environment-id")?);
            }
            "--note" => note = Some(next_value(arguments, &mut index, "--note")?),
            "--runs" => {
                let value = next_value(arguments, &mut index, "--runs")?;
                runs = value
                    .parse()
                    .map_err(|_| format!("--runs is not a valid integer: {value}"))?;
            }
            "--dataset-manifest" => {
                dataset_manifest_path = Some(PathBuf::from(next_value(
                    arguments,
                    &mut index,
                    "--dataset-manifest",
                )?));
            }
            "--output-dir" => {
                output_directory = Some(PathBuf::from(next_value(
                    arguments,
                    &mut index,
                    "--output-dir",
                )?));
            }
            "--help" | "-h" => return Err(help_text().to_string()),
            argument => {
                return Err(format!(
                    "unsupported engine benchmark argument: {argument}\n\n{}",
                    help_text()
                ))
            }
        }
        index += 1;
    }
    Ok(EngineBenchmarkCliOptions {
        label,
        environment_id,
        note,
        runs,
        dataset_manifest_path: dataset_manifest_path
            .ok_or_else(|| "--dataset-manifest requires a value".to_string())?,
        output_directory: output_directory
            .ok_or_else(|| "--output-dir requires a value".to_string())?,
    })
}

fn source_info() -> Result<BenchmarkSourceInfo, String> {
    Ok(BenchmarkSourceInfo {
        application_version: env!("CARGO_PKG_VERSION").to_string(),
        source_commit: git_output(&["rev-parse", "HEAD"])?,
        source_dirty_at_build: git_output(&["status", "--porcelain", "--untracked-files=normal"])
            .map(|output| !output.is_empty())?,
        build_profile: if cfg!(debug_assertions) {
            "debug".to_string()
        } else {
            "release".to_string()
        },
    })
}

/// Reads repository provenance at execution time. Unlike build-script change
/// tracking, this also detects untracked reports or configuration files added
/// after the maintenance binary was compiled.
fn git_output(arguments: &[&str]) -> Result<String, String> {
    let repository_root = repository_root()?;
    let safe_directory = format!("safe.directory={}", repository_root.display());
    let output = Command::new("git")
        .args(["-c", &safe_directory])
        .args(arguments)
        .current_dir(&repository_root)
        .output()
        .map_err(|error| {
            format!(
                "failed to run Git in {}: {error}",
                repository_root.display()
            )
        })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(format!(
            "Git command failed in {} with status {}: {stderr}",
            repository_root.display(),
            output.status
        ));
    }
    String::from_utf8(output.stdout)
        .map(|value| value.trim().to_string())
        .map_err(|error| format!("Git output was not valid UTF-8: {error}"))
}

fn repository_root() -> Result<PathBuf, String> {
    // The workspace location is a compile-time property of this repository
    // tool. Git state is still read at execution time so later source changes
    // cannot be mistaken for the state that existed during compilation.
    let manifest_directory = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_directory
        .parent()
        .and_then(|path| path.parent())
        .map(PathBuf::from)
        .ok_or_else(|| "xtask is not inside the expected workspace layout".to_string())
}

fn parse_dataset_options(arguments: &[String]) -> Result<BenchmarkDatasetCliOptions, String> {
    let mut output_directory = None;
    let mut seed = None;
    let mut recreate = false;
    let mut index = 0;
    while index < arguments.len() {
        match arguments[index].as_str() {
            DATASET_FLAG | "--" => {}
            "--output-dir" => {
                output_directory = Some(PathBuf::from(next_value(
                    arguments,
                    &mut index,
                    "--output-dir",
                )?));
            }
            "--seed" => {
                let value = next_value(arguments, &mut index, "--seed")?;
                seed = Some(
                    value
                        .parse()
                        .map_err(|_| format!("--seed is not a valid unsigned integer: {value}"))?,
                );
            }
            "--recreate" => recreate = true,
            "--help" | "-h" => return Err(help_text().to_string()),
            argument => {
                return Err(format!(
                    "unsupported dataset argument: {argument}\n\n{}",
                    help_text()
                ))
            }
        }
        index += 1;
    }
    Ok(BenchmarkDatasetCliOptions {
        output_directory: output_directory
            .ok_or_else(|| "--output-dir requires a value".to_string())?,
        seed,
        recreate,
    })
}

fn parse_options(arguments: &[String]) -> Result<BaselineCliOptions, String> {
    let mut options = BaselineCliOptions {
        label: DEFAULT_LABEL.to_string(),
        environment_id: None,
        note: None,
        runs: DEFAULT_RUNS,
        output_directory: None,
        project_roots: Vec::new(),
        deep_project_discovery: false,
    };
    let mut index = 0;
    while index < arguments.len() {
        match arguments[index].as_str() {
            // Some pnpm/npm versions forward the `--` script separator. Accept
            // it explicitly so documented commands behave consistently.
            BASELINE_FLAG | "--" => {}
            "--label" => {
                options.label = next_value(arguments, &mut index, "--label")?;
            }
            "--environment-id" => {
                options.environment_id =
                    Some(next_value(arguments, &mut index, "--environment-id")?);
            }
            "--note" => {
                options.note = Some(next_value(arguments, &mut index, "--note")?);
            }
            "--runs" => {
                let value = next_value(arguments, &mut index, "--runs")?;
                options.runs = value
                    .parse()
                    .map_err(|_| format!("--runs is not a valid integer: {value}"))?;
            }
            "--output-dir" => {
                options.output_directory = Some(PathBuf::from(next_value(
                    arguments,
                    &mut index,
                    "--output-dir",
                )?));
            }
            "--project-root" => {
                options
                    .project_roots
                    .push(next_value(arguments, &mut index, "--project-root")?);
            }
            "--deep-project-discovery" => options.deep_project_discovery = true,
            "--help" | "-h" => return Err(help_text().to_string()),
            argument => {
                return Err(format!(
                    "unsupported baseline argument: {argument}\n\n{}",
                    help_text()
                ))
            }
        }
        index += 1;
    }
    if options.deep_project_discovery && !options.project_roots.is_empty() {
        return Err("--deep-project-discovery cannot be combined with --project-root".to_string());
    }
    Ok(options)
}

fn parse_comparison_options(arguments: &[String]) -> Result<BaselineComparisonCliOptions, String> {
    parse_comparison_paths(arguments, COMPARISON_FLAG, "baseline")
}

fn parse_comparison_paths(
    arguments: &[String],
    command_flag: &str,
    command_name: &str,
) -> Result<BaselineComparisonCliOptions, String> {
    let mut baseline_path = None;
    let mut candidate_path = None;
    let mut output_path = None;
    let mut index = 0;
    while index < arguments.len() {
        match arguments[index].as_str() {
            argument if argument == command_flag || argument == "--" => {}
            "--baseline" => {
                baseline_path = Some(PathBuf::from(next_value(
                    arguments,
                    &mut index,
                    "--baseline",
                )?));
            }
            "--candidate" => {
                candidate_path = Some(PathBuf::from(next_value(
                    arguments,
                    &mut index,
                    "--candidate",
                )?));
            }
            "--output" => {
                output_path = Some(PathBuf::from(next_value(
                    arguments, &mut index, "--output",
                )?));
            }
            "--help" | "-h" => return Err(help_text().to_string()),
            argument => {
                return Err(format!(
                    "unsupported {command_name} comparison argument: {argument}\n\n{}",
                    help_text()
                ))
            }
        }
        index += 1;
    }
    Ok(BaselineComparisonCliOptions {
        baseline_path: baseline_path.ok_or_else(|| "--baseline requires a value".to_string())?,
        candidate_path: candidate_path.ok_or_else(|| "--candidate requires a value".to_string())?,
        output_path: output_path.ok_or_else(|| "--output requires a value".to_string())?,
    })
}

fn next_value(
    arguments: &[String],
    current_index: &mut usize,
    option_name: &str,
) -> Result<String, String> {
    *current_index += 1;
    arguments
        .get(*current_index)
        .filter(|value| !value.starts_with("--"))
        .cloned()
        .ok_or_else(|| format!("{option_name} requires a value"))
}

fn help_text() -> &'static str {
    "Usage:\n\
     xtask --cleanup-baseline [options]\n\
     xtask --compare-cleanup-baselines --baseline <JSON> --candidate <JSON> --output <Markdown>\n\
     xtask --generate-benchmark-dataset --output-dir <directory> [--seed <integer>] [--recreate]\n\
     xtask --engine-benchmark --dataset-manifest <JSON> --output-dir <directory> [options]\n\
     xtask --compare-engine-benchmarks --baseline <JSON> --candidate <JSON> --output <Markdown>\n\
     xtask --application-uninstall-benchmark [--runs <count>]\n\
     \n\
     --label <label>       Report label using letters, digits, dots, hyphens, or underscores\n\
     --environment-id <ID> Stable benchmark environment identifier\n\
     --runs <count>        Scan count from 1 to 10; defaults to 3\n\
     --output-dir <path>   JSON and Markdown output directory\n\
     --project-root <path> Project artifact root; may be repeated\n\
     --deep-project-discovery Search local volumes for development projects\n\
     --note <text>         Environment note up to 500 characters\n\
     --baseline <JSON>     Baseline report for comparison\n\
     --candidate <JSON>    Candidate report for comparison\n\
     --output <Markdown>   Comparison report output path\n\
     --seed <integer>      Fixed dataset seed; defaults to 20260717\n\
     --recreate            Recreate an existing valid dataset explicitly\n\
     --dataset-manifest <JSON> Fixed dataset manifest for the engine suite"
}
