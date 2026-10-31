use std::{fs, io::Write, os::windows::fs::OpenOptionsExt, sync::Mutex, time::Instant};

use crate::{filesystem::PermanentDeleteCandidate, storage::duplicates::DuplicateFileService};

struct DiagnosticLogger(Mutex<Option<std::io::BufWriter<fs::File>>>);

impl log::Log for DiagnosticLogger {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.level() <= log::Level::Info
    }

    fn log(&self, record: &log::Record<'_>) {
        if self.enabled(record.metadata()) {
            if let Some(file) = self.0.lock().unwrap().as_mut() {
                writeln!(file, "{} {}", record.level(), record.args()).unwrap();
            }
        }
    }

    fn flush(&self) {
        if let Some(file) = self.0.lock().unwrap().as_mut() {
            file.flush().unwrap();
        }
    }
}

static LOGGER: DiagnosticLogger = DiagnosticLogger(Mutex::new(None));

/// Run alone on disposable NTFS/exFAT/FAT32 fixtures with production logging enabled.
/// Creation, scanning, and cleanup are excluded from the measured deletion batch.
#[test]
#[ignore = "native duplicate-delete diagnostics and batch performance validation"]
fn benchmark_duplicate_delete_volume_diagnostics() {
    let _operation_lock = crate::shared::operation::test_operation_lock();
    let log_path = std::env::var_os("MANGODISK_VOLUME_DIAGNOSTICS_LOG")
        .expect("provide a diagnostic output path");
    *LOGGER.0.lock().unwrap() = Some(std::io::BufWriter::new(
        fs::File::create(&log_path).unwrap(),
    ));
    log::set_logger(&LOGGER).expect("run in an isolated test process");
    log::set_max_level(log::LevelFilter::Info);
    for run in 0..6 {
        for (workload, count, locked) in [
            ("success_1", 1, false),
            ("success_100", 100, false),
            ("locked_1", 1, true),
            ("locked_100", 100, true),
        ] {
            let fixture = tempfile::Builder::new()
                .prefix("mangodisk-volume-benchmark-")
                .tempdir()
                .unwrap();
            for group in 0..count {
                let content = format!("synthetic duplicate payload group {group:04}");
                for copy in 0..2 {
                    fs::write(
                        fixture.path().join(format!("{group:04}-{copy}.bin")),
                        &content,
                    )
                    .unwrap();
                }
            }
            let result = DuplicateFileService::find_paged_with_progress(
                vec![fixture.path().to_string_lossy().into_owned()],
                1,
                |_| {},
                |_| {},
            )
            .unwrap();
            let page = DuplicateFileService::page(result.scan_id, 0, u64::MAX).unwrap();
            assert_eq!(page.groups.len(), count);
            let candidates = page
                .groups
                .iter()
                .map(|group| {
                    let entry = &group.entries[0];
                    PermanentDeleteCandidate {
                        path: entry.path.clone(),
                        expected_bytes: entry.bytes,
                        expected_modified_at_ms: entry.modified_at_ms,
                    }
                })
                .collect::<Vec<_>>();
            let locks = if locked {
                candidates
                    .iter()
                    .map(|candidate| {
                        fs::OpenOptions::new()
                            .read(true)
                            .share_mode(3)
                            .open(&candidate.path)
                            .unwrap()
                    })
                    .collect::<Vec<_>>()
            } else {
                Vec::new()
            };
            let started = Instant::now();
            let deletion =
                DuplicateFileService::delete_files_permanently(result.scan_id, candidates).unwrap();
            let elapsed_us = started.elapsed().as_micros();
            // Await only after timing the deletion, so fixture teardown cannot race
            // the background query and every diagnostic record can be checked.
            super::wait_for_volume_diagnostics();
            assert_eq!(deletion.removed_paths.len(), if locked { 0 } else { count });
            assert_eq!(deletion.failed.len(), if locked { count } else { 0 });
            if locked {
                assert!(page.groups.iter().all(|group| group
                    .entries
                    .iter()
                    .all(|entry| std::path::Path::new(&entry.path).exists())));
            }
            assert!(!fs::read_dir(fixture.path())
                .unwrap()
                .filter_map(Result::ok)
                .any(|entry| entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".mangodisk-delete-")));
            println!("VOLUME_BATCH run={run} workload={workload} requested={count} removed={} failed={} elapsed_us={elapsed_us}", deletion.removed_paths.len(), deletion.failed.len());
            drop(locks);
        }
    }
    super::log_failed_volume_samples(
        "duplicates",
        0,
        0,
        &[crate::filesystem::PermanentDeleteFailure {
            path: "\\\\?\\Volume{00000000-0000-0000-0000-000000000000}\\missing.bin".into(),
            details: crate::filesystem::models::PermanentDeleteFailureDetails {
                message: "synthetic unavailable-volume probe".into(),
                ..Default::default()
            },
        }],
    );
    super::wait_for_volume_diagnostics();
    log::logger().flush();
    // The baseline adapters intentionally do not emit volume samples. The explicit
    // probe still exercises native query failures without touching any user data.
    let diagnostics = fs::read_to_string(&log_path).unwrap();
    assert!(diagnostics.contains("outcome=query_failed"));
    assert!(diagnostics.contains("filesystem=\"unknown\""));
}
