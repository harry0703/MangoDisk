#[cfg(not(target_os = "linux"))]
fn main() {
    println!("linux_analysis_core_benchmark status=unsupported platform=non_linux");
}

#[cfg(target_os = "linux")]
fn main() {
    if let Err(error) = linux_benchmark::run() {
        eprintln!("linux_analysis_core_benchmark status=failed error={error}");
        std::process::exit(1);
    }
}

#[cfg(target_os = "linux")]
mod linux_benchmark {
    use std::{
        env,
        path::PathBuf,
        sync::{Arc, Mutex},
        time::Instant,
    };

    use mangodisk_core::{
        configure_application_paths, AnalysisService, ApplicationPaths, TraversalProgress,
    };

    pub(super) fn run() -> Result<(), String> {
        let root = env::args()
            .nth(1)
            .map(PathBuf::from)
            .ok_or_else(|| "benchmark root argument is required".to_string())?;
        if !root.is_absolute() || !root.is_dir() {
            return Err("benchmark root must be an existing absolute directory".to_string());
        }
        let state_root = env::var_os("MANGODISK_BENCHMARK_STATE_ROOT")
            .map(PathBuf::from)
            .ok_or_else(|| "MANGODISK_BENCHMARK_STATE_ROOT is required".to_string())?;
        std::fs::create_dir_all(&state_root)
            .map_err(|error| format!("failed to create benchmark state root: {error}"))?;
        let root = std::fs::canonicalize(root)
            .map_err(|error| format!("failed to canonicalize benchmark root: {error}"))?;
        let state_root = std::fs::canonicalize(state_root)
            .map_err(|error| format!("failed to canonicalize benchmark state root: {error}"))?;
        if root.starts_with(&state_root) || state_root.starts_with(&root) {
            return Err("benchmark state and scan root must not contain each other".to_string());
        }
        configure_application_paths(
            ApplicationPaths::new(
                state_root.join("data"),
                state_root.join("cache"),
                state_root.join("runtime"),
            )
            .map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())?;

        let final_progress = Arc::new(Mutex::new(None::<TraversalProgress>));
        let callback_progress = Arc::clone(&final_progress);
        let started = Instant::now();
        let result = AnalysisService::analyze_with_progress(
            Some(root.to_string_lossy().into_owned()),
            true,
            move |progress| {
                if let Ok(mut latest) = callback_progress.lock() {
                    *latest = Some(progress);
                }
            },
        )
        .map_err(|error| error.to_string())?;
        let elapsed_ms = started.elapsed().as_millis();

        let mut entries = result
            .entries
            .iter()
            .map(|entry| {
                (
                    *blake3::hash(entry.path.as_bytes()).as_bytes(),
                    entry.bytes,
                    entry.file_count,
                    entry.is_directory,
                )
            })
            .collect::<Vec<_>>();
        entries.sort_unstable();
        let mut hasher = blake3::Hasher::new();
        for (path, bytes, file_count, is_directory) in &entries {
            hasher.update(path);
            hasher.update(&bytes.to_le_bytes());
            hasher.update(&file_count.to_le_bytes());
            hasher.update(&[u8::from(*is_directory)]);
        }
        let progress = final_progress
            .lock()
            .map_err(|_| "failed to read final analysis progress".to_string())?
            .clone()
            .ok_or_else(|| "analysis did not emit final progress".to_string())?;
        println!(
            "linux_analysis_core_benchmark status=ok total_bytes={} skipped={} entries={} files_observed={} bytes_observed={} result_digest={} elapsed_ms={}",
            result.total_bytes,
            result.skipped_count,
            result.entries.len(),
            progress.items_scanned,
            progress.bytes_scanned,
            hasher.finalize().to_hex(),
            elapsed_ms,
        );
        Ok(())
    }
}
