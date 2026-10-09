#[cfg(not(windows))]
fn main() {
    println!("windows_analysis_benchmark status=unsupported platform=non_windows");
}

#[cfg(windows)]
fn main() {
    if let Err(error) = windows_benchmark::run() {
        eprintln!("windows_analysis_benchmark status=failed error={error}");
        std::process::exit(1);
    }
}

#[cfg(windows)]
mod windows_benchmark {
    use std::{
        env,
        os::windows::ffi::OsStrExt,
        path::{Path, PathBuf},
        time::Instant,
    };

    use mangodisk_platform::{
        current_platform, FastAnalysisRecord, FastAnalysisScanError, Platform,
    };
    use windows_sys::Win32::System::{
        ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS},
        Threading::GetCurrentProcess,
    };

    const DEFAULT_MINIMUM_MIB: u64 = 50;
    static LOGGER: BenchmarkLogger = BenchmarkLogger;

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum BenchmarkMode {
        Scan,
        CancelImmediately,
        FailConsumer,
    }

    struct BenchmarkLogger;

    impl log::Log for BenchmarkLogger {
        fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
            metadata.level() <= log::Level::Info
        }

        fn log(&self, record: &log::Record<'_>) {
            if self.enabled(record.metadata()) {
                eprintln!("{} {}", record.level(), record.args());
            }
        }

        fn flush(&self) {}
    }

    pub(super) fn run() -> Result<(), String> {
        log::set_logger(&LOGGER)
            .map(|()| log::set_max_level(log::LevelFilter::Info))
            .map_err(|error| format!("failed to initialize benchmark logging: {error}"))?;
        let root = env::args()
            .nth(1)
            .map(PathBuf::from)
            .unwrap_or_else(|| current_platform().system_volume_path());
        let minimum_mib = env::args()
            .nth(2)
            .map(|value| {
                value
                    .parse::<u64>()
                    .map_err(|_| format!("minimum file size is not a valid MiB value: {value}"))
            })
            .transpose()?
            .unwrap_or(DEFAULT_MINIMUM_MIB);
        let minimum_bytes = minimum_mib
            .checked_mul(1024 * 1024)
            .ok_or_else(|| "minimum file size exceeds the supported range".to_string())?;
        let mode = match env::args().nth(3).as_deref() {
            None | Some("scan") => BenchmarkMode::Scan,
            Some("cancel") => BenchmarkMode::CancelImmediately,
            Some("consumer-fail") => BenchmarkMode::FailConsumer,
            Some(value) => return Err(format!("unsupported benchmark mode: {value}")),
        };
        let consumer_failure_record = env::args()
            .nth(4)
            .map(|value| {
                value
                    .parse::<u64>()
                    .map_err(|_| {
                        format!("consumer failure position is not a positive integer: {value}")
                    })
                    .and_then(|record| {
                        (record > 0)
                            .then_some(record)
                            .ok_or_else(|| "consumer failure position must exceed zero".to_string())
                    })
            })
            .transpose()?
            .unwrap_or(1);

        // The diagnostic needs an enumeration-order-independent result digest, so it retains
        // fixed-size path digests instead of full user paths. Production streams directly into
        // the Core sink and does not allocate these two vectors.
        let mut directories = Vec::new();
        let mut candidates = Vec::new();
        let mut consumed_records = 0_u64;
        let started = Instant::now();
        let scan_result = current_platform().fast_analysis_records(
            mangodisk_platform::FastAnalysisQuery {
                retained_file_limit: 0,
                name_exclusions: &mangodisk_platform::NameExclusions::default(),
                excluded_roots: &[],
                root: &root,
                purpose: mangodisk_platform::ScanPurpose::Analysis,
                large_file_minimum_bytes: minimum_bytes,
                should_prune_directory: |_| false,
            },
            &|| mode == BenchmarkMode::CancelImmediately,
            &mut |_, _, _| {},
            &mut |record| {
                consumed_records = consumed_records.saturating_add(1);
                // The failure point can occur mid-stream to prove that Platform propagates the Nth
                // consumer error without calling it again. The first record remains the default so
                // existing manual probe commands keep their behavior.
                if mode == BenchmarkMode::FailConsumer
                    && consumed_records == consumer_failure_record
                {
                    return Err("benchmark_consumer_failure".to_string());
                }
                match record {
                    FastAnalysisRecord::Directory {
                        path,
                        logical_bytes,
                        allocated_bytes,
                        file_count,
                        skipped_count,
                        ..
                    } => directories.push((
                        path_digest(&path),
                        logical_bytes,
                        allocated_bytes,
                        file_count,
                        skipped_count,
                    )),
                    FastAnalysisRecord::HardLinkedFile { .. }
                    | FastAnalysisRecord::AnalysisFile(_) => {}
                    FastAnalysisRecord::LargeFileCandidate(path) => {
                        candidates.push(path_digest(&path));
                    }
                }
                Ok(())
            },
        );
        match (mode, &scan_result) {
            (BenchmarkMode::CancelImmediately, Err(FastAnalysisScanError::Cancelled)) => {
                println!(
                    "windows_analysis_benchmark status=cancelled elapsed_ms={}",
                    started.elapsed().as_millis()
                );
                return Ok(());
            }
            (BenchmarkMode::FailConsumer, Err(FastAnalysisScanError::Consumer(error)))
                if error == "benchmark_consumer_failure" =>
            {
                if consumed_records != consumer_failure_record {
                    return Err(format!(
                        "consumer received records after failure: expected={consumer_failure_record}, actual={consumed_records}"
                    ));
                }
                println!(
                    "windows_analysis_benchmark status=consumer_failed failure_record={} consumed_records={} elapsed_ms={}",
                    consumer_failure_record,
                    consumed_records,
                    started.elapsed().as_millis()
                );
                return Ok(());
            }
            (BenchmarkMode::CancelImmediately, _) | (BenchmarkMode::FailConsumer, _) => {
                return Err(
                    "benchmark fault injection did not return the expected error".to_string(),
                );
            }
            (BenchmarkMode::Scan, _) => {}
        }
        let summary = scan_result
            .map_err(format_scan_error)?
            .ok_or_else(|| "the root does not support native Windows analysis".to_string())?;

        directories.sort_unstable();
        candidates.sort_unstable();
        let mut result_hasher = blake3::Hasher::new();
        for (path, logical_bytes, allocated_bytes, file_count, skipped_count) in &directories {
            result_hasher.update(path);
            result_hasher.update(&logical_bytes.to_le_bytes());
            result_hasher.update(&allocated_bytes.to_le_bytes());
            result_hasher.update(&file_count.to_le_bytes());
            result_hasher.update(&skipped_count.to_le_bytes());
        }
        let mut candidate_hasher = blake3::Hasher::new();
        for path in &candidates {
            candidate_hasher.update(path);
        }
        let (peak_working_set_bytes, working_set_bytes) = process_memory_bytes()?;
        println!(
            "windows_analysis_benchmark status=ok strategy={} root_digest={} threshold_bytes={} pages={} entries={} summary_directories={} emitted_directories={} summary_candidates={} emitted_candidates={} root_logical_bytes={} root_bytes={} root_files={} root_skipped={} result_digest={} candidate_digest={} consumer_ms={} returned_bytes={} peak_working_set_bytes={} working_set_bytes={} elapsed_ms={}",
            summary.strategy,
            path_digest_hex(&root),
            minimum_bytes,
            summary.page_count,
            summary.entry_count,
            summary.directory_count,
            directories.len(),
            summary.candidate_count,
            candidates.len(),
            summary.root_logical_bytes,
            summary.root_allocated_bytes,
            summary.root_file_count,
            summary.root_skipped_count,
            result_hasher.finalize().to_hex(),
            candidate_hasher.finalize().to_hex(),
            summary.consumer_elapsed_ms,
            summary.returned_bytes,
            peak_working_set_bytes,
            working_set_bytes,
            started.elapsed().as_millis(),
        );
        Ok(())
    }

    fn format_scan_error(error: FastAnalysisScanError) -> String {
        match error {
            FastAnalysisScanError::Cancelled => "scan was cancelled unexpectedly".to_string(),
            FastAnalysisScanError::Busy => {
                "native analysis workers were unexpectedly busy".to_string()
            }
            FastAnalysisScanError::Platform(error) => format!("platform scan failed: {error}"),
            FastAnalysisScanError::Consumer(error) => format!("result consumer failed: {error}"),
        }
    }

    fn path_digest(path: &Path) -> [u8; 32] {
        let mut hasher = blake3::Hasher::new();
        for unit in path.as_os_str().encode_wide() {
            let normalized = if (u16::from(b'A')..=u16::from(b'Z')).contains(&unit) {
                unit + u16::from(b'a' - b'A')
            } else {
                unit
            };
            hasher.update(&normalized.to_le_bytes());
        }
        *hasher.finalize().as_bytes()
    }

    fn path_digest_hex(path: &Path) -> String {
        blake3::Hash::from_bytes(path_digest(path))
            .to_hex()
            .to_string()
    }

    fn process_memory_bytes() -> Result<(usize, usize), String> {
        let mut counters = PROCESS_MEMORY_COUNTERS {
            cb: u32::try_from(std::mem::size_of::<PROCESS_MEMORY_COUNTERS>()).map_err(|_| {
                "process memory structure exceeds the Windows API range".to_string()
            })?,
            ..Default::default()
        };
        let succeeded =
            unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, counters.cb) };
        if succeeded == 0 {
            return Err(format!(
                "failed to read benchmark process memory: {}",
                std::io::Error::last_os_error()
            ));
        }
        Ok((counters.PeakWorkingSetSize, counters.WorkingSetSize))
    }
}
