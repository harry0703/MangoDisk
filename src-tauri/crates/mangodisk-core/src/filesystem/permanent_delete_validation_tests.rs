use std::{io::Write, sync::Mutex};

use super::*;

fn fixture() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("mangodisk-delete-validation-")
        .tempdir()
        .expect("create a disposable deletion fixture")
}

fn staging_count(parent: &Path) -> usize {
    fs::read_dir(parent)
        .expect("inspect staging remnants")
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with(".mangodisk-delete-")
        })
        .count()
}

#[test]
#[cfg(windows)]
fn locked_file_failure_retains_the_native_code_and_removal_stage() {
    use std::os::windows::fs::OpenOptionsExt;
    let sandbox = fixture();
    let path = sandbox.path().join("locked.bin");
    fs::write(&path, b"keep").unwrap();
    let metadata = fs::metadata(&path).unwrap();
    let _locked = fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&path)
        .unwrap();
    let error = delete_file_candidate_permanently(&PermanentDeleteCandidate {
        path: path.to_string_lossy().into_owned(),
        expected_bytes: 4,
        expected_modified_at_ms: modified_ms(&metadata),
    })
    .unwrap_err();
    assert_eq!(error.native_code(), Some(32));
    assert_eq!(error.reason(), Some(CoreErrorReason::ResourceBusy));
    assert_eq!(error.stage(), "remove");
    assert!(!error.is_partial());
    drop(_locked);
    assert_eq!(fs::read(&path).unwrap(), b"keep");
    assert_eq!(staging_count(sandbox.path()), 0);
}

#[test]
fn successful_deletion_removes_file_without_creating_staging() {
    let sandbox = fixture();
    let path = sandbox.path().join("payload.bin");
    fs::write(&path, b"payload").unwrap();
    let prepared = prepare_path_for_permanent_delete(&path).unwrap();
    delete_path_permanently(prepared, 7, 1).unwrap();
    assert!(!path.exists());
    assert_eq!(staging_count(sandbox.path()), 0);
}

#[test]
fn nonempty_directory_failure_leaves_the_item_in_place() {
    let sandbox = fixture();
    let path = sandbox.path().join("directory");
    fs::create_dir(&path).unwrap();
    let prepared = prepare_path_for_permanent_delete(&path).unwrap();
    fs::write(path.join("concurrent.bin"), b"keep").unwrap();
    let error = delete_empty_directory_permanently(prepared).unwrap_err();
    assert!(!error.is_partial());
    assert_eq!(fs::read(path.join("concurrent.bin")).unwrap(), b"keep");
    assert_eq!(staging_count(sandbox.path()), 0);
}

struct BenchmarkLogger(Mutex<Option<std::io::BufWriter<fs::File>>>);

impl log::Log for BenchmarkLogger {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.level() <= log::Level::Info
    }

    fn log(&self, record: &log::Record<'_>) {
        if self.enabled(record.metadata()) {
            if let Some(output) = self.0.lock().unwrap().as_mut() {
                writeln!(output, "{} {}", record.level(), record.args()).unwrap();
            }
        }
    }

    fn flush(&self) {
        if let Some(output) = self.0.lock().unwrap().as_mut() {
            output.flush().unwrap();
        }
    }
}

static BENCHMARK_LOGGER: BenchmarkLogger = BenchmarkLogger(Mutex::new(None));

/// Run alone with `--ignored --exact --nocapture --test-threads=1`.
/// Set TEMP/TMP to a disposable volume to exercise its real filesystem driver.
/// Fixture creation and cleanup are excluded; production info/warning logging is enabled.
#[test]
#[ignore = "bounded permanent-delete filesystem and logging benchmark"]
fn benchmark_permanent_delete_filesystem() {
    let log_path = std::env::var_os("MANGODISK_DELETE_BENCHMARK_LOG")
        .expect("provide a diagnostic log outside the disposable fixture");
    *BENCHMARK_LOGGER.0.lock().unwrap() = Some(std::io::BufWriter::new(
        fs::File::create(log_path).expect("create benchmark diagnostics"),
    ));
    log::set_logger(&BENCHMARK_LOGGER).expect("run the benchmark in its own test process");
    log::set_max_level(log::LevelFilter::Info);

    for run in 0..6 {
        for (workload, operations, file_count, file_bytes, retain_root) in [
            ("zero_byte_file", 100, 1, 0, false),
            ("small_file", 100, 1, 4_096, false),
            ("large_file", 8, 1, 1_048_576, false),
            ("directory_64", 8, 64, 32, false),
            ("directory_600", 2, 600, 32, false),
            ("retain_directory_64", 8, 64, 32, true),
        ] {
            let sandbox = fixture();
            let payload = vec![42_u8; file_bytes];
            let mut paths = Vec::new();
            for index in 0..operations {
                let path = sandbox.path().join(format!("item-{index:04}"));
                if file_count == 1 {
                    fs::write(&path, &payload).unwrap();
                } else {
                    fs::create_dir(&path).unwrap();
                    if retain_root {
                        fs::create_dir(path.join("preexisting-empty")).unwrap();
                    }
                    for child in 0..file_count {
                        fs::write(path.join(format!("{child:04}.bin")), &payload).unwrap();
                    }
                }
                paths.push(path);
            }
            let mut elapsed_us = 0_u128;
            let mut completed = 0;
            let mut failed = 0;
            let mut restored = 0;
            for path in &paths {
                let started = Instant::now();
                let result = prepare_path_for_permanent_delete(path).and_then(|prepared| {
                    if retain_root {
                        delete_directory_contents_permanently_with_cancellation(prepared, &|| false)
                            .map(|_| ())
                    } else {
                        delete_path_permanently(
                            prepared,
                            (file_count * file_bytes) as u64,
                            file_count as u64,
                        )
                    }
                });
                elapsed_us += started.elapsed().as_micros();
                match result {
                    Ok(_) => {
                        completed += 1;
                        if retain_root {
                            assert_eq!(fs::read_dir(path).unwrap().count(), 1);
                            assert_eq!(
                                fs::read_dir(path.join("preexisting-empty"))
                                    .unwrap()
                                    .count(),
                                0
                            );
                        } else {
                            assert!(!path.exists());
                        }
                    }
                    Err(_) => {
                        failed += 1;
                        if path.exists() {
                            restored += 1;
                            let content = if file_count == 1 {
                                path.clone()
                            } else {
                                path.join("0000.bin")
                            };
                            assert_eq!(fs::read(content).unwrap(), payload);
                        }
                    }
                }
            }
            log::Log::flush(&BENCHMARK_LOGGER);
            println!(
                "delete_benchmark {}",
                serde_json::json!({
                    "run": run, "warmup": run == 0, "workload": workload,
                    "operations": operations, "elapsed_us": elapsed_us,
                    "completed": completed, "failed": failed, "visible_after_failure": restored,
                    "staging_remnants": staging_count(sandbox.path())
                })
            );
        }
    }
    log::Log::flush(&BENCHMARK_LOGGER);
    *BENCHMARK_LOGGER.0.lock().unwrap() = None;
}

#[test]
fn replacement_with_matching_size_and_timestamp_is_preserved() {
    let sandbox = fixture();
    let path = sandbox.path().join("candidate.bin");
    let original = sandbox.path().join("original.bin");
    fs::write(&path, b"original").unwrap();
    let prepared = prepare_path_for_permanent_delete(&path).unwrap();
    let modified = prepared.metadata().modified().unwrap();
    fs::rename(&path, &original).unwrap();
    fs::write(&path, b"replaced").unwrap();
    fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(modified))
        .unwrap();
    assert_eq!(
        fs::metadata(&path).unwrap().len(),
        prepared.metadata().len()
    );
    assert_eq!(
        modified_ms(&fs::metadata(&path).unwrap()),
        modified_ms(prepared.metadata())
    );
    let error = delete_path_permanently(prepared, 8, 1).unwrap_err();
    assert_eq!(error.reason(), Some(CoreErrorReason::ItemChanged));
    assert_eq!(fs::read(&path).unwrap(), b"replaced");
    assert_eq!(fs::read(&original).unwrap(), b"original");
    assert_eq!(staging_count(sandbox.path()), 0);
}

#[test]
fn nested_unicode_directory_deletion_removes_only_the_target() {
    let sandbox = fixture();
    let mut parent = sandbox.path().to_path_buf();
    for index in 0..10 {
        parent = parent.join(format!("nested-directory-segment-{index:03}"));
    }
    fs::create_dir_all(&parent).unwrap();
    let path = parent.join("\u{8d44}\u{6e90}-\u{1f642}");
    fs::create_dir(&path).unwrap();
    fs::write(path.join("\u{96f6}\u{5b57}\u{8282}.txt"), b"").unwrap();
    fs::write(parent.join("keep.bin"), b"keep").unwrap();
    let prepared = prepare_path_for_permanent_delete(&path).unwrap();
    let outcome =
        delete_directory_tree_permanently_with_cancellation(prepared, 0, 1, &|| false).unwrap();
    assert_eq!(outcome.affected_item_count(), 1);
    assert_eq!(outcome.released_bytes(), 0);
    assert!(!path.exists());
    assert_eq!(fs::read(parent.join("keep.bin")).unwrap(), b"keep");
    assert_eq!(staging_count(&parent), 0);
}

#[test]
fn cancellation_before_removal_leaves_target_without_reporting_partial_deletion() {
    let sandbox = fixture();
    let path = sandbox.path().join("directory");
    fs::create_dir(&path).unwrap();
    fs::write(path.join("zero.bin"), b"").unwrap();
    let prepared = prepare_path_for_permanent_delete(&path).unwrap();
    let error =
        delete_directory_tree_permanently_with_cancellation(prepared, 0, 1, &|| true).unwrap_err();
    assert!(!error.is_partial());
    assert_eq!(error.affected_item_count(), 0);
    assert!(path.join("zero.bin").exists());
    assert_eq!(staging_count(sandbox.path()), 0);
}

#[cfg(windows)]
#[test]
fn readonly_file_deletion_reports_its_actual_outcome() {
    let sandbox = fixture();
    let path = sandbox.path().join("readonly.bin");
    fs::write(&path, b"keep").unwrap();
    let original_permissions = fs::metadata(&path).unwrap().permissions();
    let mut permissions = original_permissions.clone();
    permissions.set_readonly(true);
    fs::set_permissions(&path, permissions.clone()).unwrap();
    let result = prepare_path_for_permanent_delete(&path)
        .and_then(|prepared| delete_path_permanently(prepared, 4, 1));
    match result {
        Ok(()) => assert!(!path.exists()),
        Err(error) => {
            fs::set_permissions(&path, original_permissions).unwrap();
            assert_eq!(error.native_code, Some(5));
            assert_eq!(error.reason(), Some(CoreErrorReason::AccessDeniedOrBusy));
            assert_eq!(fs::read(&path).unwrap(), b"keep");
        }
    }
    assert_eq!(staging_count(sandbox.path()), 0);
}

#[cfg(windows)]
#[test]
fn locked_child_failure_retains_directory_and_preserves_native_error() {
    use std::os::windows::fs::OpenOptionsExt;
    let sandbox = fixture();
    let path = sandbox.path().join("directory");
    fs::create_dir(&path).unwrap();
    let child = path.join("locked.bin");
    fs::write(&child, b"keep").unwrap();
    let locked = fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&child)
        .unwrap();
    let prepared = prepare_path_for_permanent_delete(&path).unwrap();
    let error = delete_path_permanently(prepared, 4, 1).unwrap_err();
    assert!(matches!(error.native_code, Some(5 | 32)));
    assert!(matches!(
        error.reason(),
        Some(CoreErrorReason::ResourceBusy | CoreErrorReason::AccessDeniedOrBusy)
    ));
    assert!(path.exists());
    drop(locked);
    assert_eq!(fs::read(&child).unwrap(), b"keep");
    assert_eq!(staging_count(sandbox.path()), 0);
}

#[test]
fn cancellation_after_zero_byte_removal_retains_accurate_partial_counts() {
    let sandbox = fixture();
    let path = sandbox.path().join("directory");
    fs::create_dir(&path).unwrap();
    for index in 0..3 {
        fs::write(path.join(format!("{index}.bin")), b"").unwrap();
    }
    let prepared = prepare_path_for_permanent_delete(&path).unwrap();
    let error = delete_directory_tree_permanently_with_cancellation(prepared, 0, 3, &|| {
        fs::read_dir(&path).unwrap().count() < 3
    })
    .unwrap_err();
    assert!(error.is_partial());
    assert_eq!(error.released_bytes(), 0);
    assert_eq!(error.affected_item_count(), 1);
    assert_eq!(fs::read_dir(&path).unwrap().count(), 2);
    assert_eq!(staging_count(sandbox.path()), 0);
}

#[cfg(windows)]
#[test]
#[ignore = "real NTFS ACL denial; run with TEMP/TMP on NTFS"]
fn ntfs_access_denial_preserves_the_target() {
    use std::process::Command;
    struct DeniedFixture(PathBuf);
    impl Drop for DeniedFixture {
        fn drop(&mut self) {
            let result = Command::new("icacls")
                .arg(&self.0)
                .args(["/remove:d", "*S-1-1-0"])
                .output();
            assert!(
                result.is_ok_and(|output| output.status.success()),
                "restore the disposable fixture ACL"
            );
        }
    }
    let sandbox = fixture();
    let path = sandbox.path().join("denied.bin");
    fs::write(&path, b"keep").unwrap();
    let output = Command::new("icacls")
        .arg(&path)
        .args(["/deny", "*S-1-1-0:(D)"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "deny deletion only on the disposable file"
    );
    let file_guard = DeniedFixture(path.clone());
    let output = Command::new("icacls")
        .arg(sandbox.path())
        .args(["/deny", "*S-1-1-0:(DC)"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "deny delete-child only on the disposable parent"
    );
    let parent_guard = DeniedFixture(sandbox.path().to_path_buf());
    let result = prepare_path_for_permanent_delete(&path)
        .and_then(|prepared| delete_path_permanently(prepared, 4, 1));
    drop(parent_guard);
    drop(file_guard);
    let error = result.expect_err("an explicit deletion ACL denial must stop deletion");
    assert_eq!(error.native_code, Some(5));
    assert_eq!(error.reason(), Some(CoreErrorReason::AccessDeniedOrBusy));
    assert_eq!(fs::read(&path).unwrap(), b"keep");
    assert_eq!(staging_count(sandbox.path()), 0);
}

#[cfg(windows)]
#[test]
#[ignore = "native reparse-point fixture; run on NTFS with a regular desktop token"]
fn ntfs_native_identity_query_rejects_junction() {
    let sandbox = fixture();
    let real = sandbox.path().join("real");
    let junction = sandbox.path().join("junction");
    fs::create_dir(&real).unwrap();
    fs::write(real.join("keep.bin"), b"keep").unwrap();
    let output = std::process::Command::new("cmd.exe")
        .args(["/C", "mklink", "/J"])
        .arg(&junction)
        .arg(&real)
        .output()
        .unwrap();
    assert!(output.status.success(), "create a disposable NTFS junction");
    let error = physical_path_identity(&junction).unwrap_err();
    assert!(error.to_string().contains("link or reparse point"));
    assert_eq!(fs::read(real.join("keep.bin")).unwrap(), b"keep");
    fs::remove_dir(junction).unwrap();
}

#[test]
fn deletion_failure_metadata_preserves_native_reason_stage_and_mutation() {
    use crate::filesystem::{
        PermanentDeleteFailureReason as Reason, PermanentDeleteFailureStage as Stage,
    };
    let error = permanent_delete_io_error(
        "create staging",
        std::io::Error::from(std::io::ErrorKind::PermissionDenied),
    )
    .at_stage("create_staging");
    let failure = error.into_failure("/fixture/denied".into());
    assert_eq!(failure.details.reason, Reason::AccessDenied);
    assert_eq!(failure.details.stage, Stage::CreateStaging);
    assert!(!failure.details.partial);
    let failure =
        PermanentDeleteError::before_mutation_with_reason("changed", CoreErrorReason::ItemChanged)
            .into_failure("/fixture/stale".into());
    assert_eq!(failure.details.reason, Reason::ItemChanged);
    let mut partial = PermanentDeleteError::before_mutation_with_reason(
        "interrupted",
        CoreErrorReason::ResourceBusy,
    )
    .at_stage("remove");
    partial.partial = true;
    partial.released_bytes = 4;
    partial.affected_item_count = 1;
    partial.native_code = Some(32);
    let value = serde_json::to_value(partial.into_failure("/fixture/partial".into())).unwrap();
    assert_eq!(value["reason"], "resourceBusy");
    assert_eq!(value["nativeCode"], 32);
    assert_eq!(value["partial"], true);
    assert_eq!(value["remainingRestored"], false);
    assert_eq!(value["stage"], "remove");
    assert!(value.get("details").is_none(), "transport fields stay flat");
}

#[cfg(windows)]
#[test]
fn locked_file_reports_sharing_violation_at_direct_removal() {
    use crate::filesystem::{
        PermanentDeleteFailureReason as Reason, PermanentDeleteFailureStage as Stage,
    };
    use std::os::windows::fs::OpenOptionsExt;
    let sandbox = fixture();
    let path = sandbox.path().join("locked.bin");
    fs::write(&path, b"locked duplicate content").unwrap();
    let metadata = fs::metadata(&path).unwrap();
    let locked = fs::OpenOptions::new()
        .read(true)
        .share_mode(3)
        .open(&path)
        .unwrap();
    let candidate = PermanentDeleteCandidate {
        path: path.to_string_lossy().into_owned(),
        expected_bytes: metadata.len(),
        expected_modified_at_ms: modified_ms(&metadata),
    };
    let error = delete_file_candidate_permanently(&candidate).unwrap_err();
    let failure = error.into_failure(candidate.path).details;
    assert_eq!(failure.reason, Reason::ResourceBusy);
    assert_eq!(failure.native_code, Some(32));
    assert_eq!(failure.stage, Stage::Remove);
    assert!(!failure.partial);
    assert_eq!(fs::read(&path).unwrap(), b"locked duplicate content");
    assert_eq!(staging_count(sandbox.path()), 0);
    drop(locked);
}

#[test]
fn missing_candidate_preserves_not_found_metadata_during_path_validation() {
    use crate::filesystem::{
        PermanentDeleteFailureReason as Reason, PermanentDeleteFailureStage as Stage,
    };
    let sandbox = fixture();
    let path = sandbox.path().join("removed-after-scan.bin");
    fs::write(&path, b"temporary duplicate").unwrap();
    let metadata = fs::metadata(&path).unwrap();
    fs::remove_file(&path).unwrap();
    let candidate = PermanentDeleteCandidate {
        path: path.to_string_lossy().into_owned(),
        expected_bytes: metadata.len(),
        expected_modified_at_ms: modified_ms(&metadata),
    };
    let failure = delete_file_candidate_permanently(&candidate)
        .unwrap_err()
        .into_failure(candidate.path)
        .details;
    println!("missing_candidate_failure={failure:?}");
    assert_eq!(failure.reason, Reason::NotFound);
    assert!(failure.native_code.is_some());
    assert_eq!(failure.stage, Stage::PrepareTarget);
    assert!(!failure.partial);
}

#[cfg(unix)]
#[test]
#[ignore = "requires unprivileged permission enforcement on a disposable local directory"]
fn denied_candidate_preserves_native_metadata_during_path_validation() {
    use crate::filesystem::{
        PermanentDeleteFailureReason as Reason, PermanentDeleteFailureStage as Stage,
    };
    use std::os::unix::fs::PermissionsExt;
    assert_ne!(unsafe { libc::geteuid() }, 0, "run as an ordinary user");
    let sandbox = fixture();
    let parent = sandbox.path().join("denied");
    fs::create_dir(&parent).unwrap();
    let path = parent.join("keep.bin");
    fs::write(&path, b"keep").unwrap();
    let metadata = fs::metadata(&path).unwrap();
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o000)).unwrap();
    let result = delete_file_candidate_permanently(&PermanentDeleteCandidate {
        path: path.to_string_lossy().into_owned(),
        expected_bytes: metadata.len(),
        expected_modified_at_ms: modified_ms(&metadata),
    });
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o700)).unwrap();
    let failure = result
        .unwrap_err()
        .into_failure(path.to_string_lossy().into_owned())
        .details;
    println!("denied_candidate_failure={failure:?}");
    assert_eq!(failure.reason, Reason::AccessDenied);
    assert_eq!(failure.native_code, Some(libc::EACCES));
    assert_eq!(failure.stage, Stage::PrepareTarget);
    assert_eq!(fs::read(&path).unwrap(), b"keep");
}

/// Run separately because production log capture owns the process-global logger.
#[test]
#[ignore = "isolated production deletion diagnostics"]
fn validate_direct_delete_failure_diagnostics() {
    let log_path = PathBuf::from(std::env::var_os("MANGODISK_DELETE_BENCHMARK_LOG").unwrap());
    *BENCHMARK_LOGGER.0.lock().unwrap() = Some(std::io::BufWriter::new(
        fs::File::create(&log_path).unwrap(),
    ));
    log::set_logger(&BENCHMARK_LOGGER).unwrap();
    log::set_max_level(log::LevelFilter::Info);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let sandbox = fixture();
        let root = sandbox.path().join("selected");
        let blocked = root.join("denied-\nentry");
        fs::create_dir_all(&blocked).unwrap();
        fs::write(blocked.join("payload.bin"), b"private fixture payload").unwrap();
        let prepared = prepare_path_for_permanent_delete(&root).unwrap();
        fs::set_permissions(&blocked, fs::Permissions::from_mode(0o000)).unwrap();
        let failure =
            delete_directory_tree_permanently_with_cancellation(prepared, 23, 1, &|| false);
        fs::set_permissions(&blocked, fs::Permissions::from_mode(0o700)).unwrap();
        let failure = failure.unwrap_err();
        assert_eq!(failure.native_code(), Some(libc::EACCES));
        assert!(!failure.is_partial());
        assert_eq!(
            fs::read(blocked.join("payload.bin")).unwrap(),
            b"private fixture payload"
        );
    }
    #[cfg(windows)]
    locked_child_failure_retains_directory_and_preserves_native_error();
    log::Log::flush(&BENCHMARK_LOGGER);
    let diagnostic = fs::read_to_string(&log_path).unwrap();
    assert!(diagnostic.contains("stage=remove mode=direct outcome=failed"));
    assert!(diagnostic.contains("remaining_location=original"));
    assert!(diagnostic.contains("native_code=Some("));
    assert!(!diagnostic.contains("private fixture payload"));
    assert!(!diagnostic.contains(".mangodisk-delete-"));
    #[cfg(unix)]
    {
        assert!(diagnostic.contains("stage=read_directory"));
        assert!(diagnostic.contains("denied-\\nentry"));
        assert!(!diagnostic.contains("denied-\nentry"));
    }
    *BENCHMARK_LOGGER.0.lock().unwrap() = None;
}
