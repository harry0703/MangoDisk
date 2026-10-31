use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use super::*;

static NEXT_DELETE_SANDBOX_ID: AtomicU64 = AtomicU64::new(1);

struct DeleteSandbox(PathBuf);

impl DeleteSandbox {
    fn new() -> Self {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let id = NEXT_DELETE_SANDBOX_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "mangodisk-permanent-delete-{}-{unique}-{id}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("create the permanent-delete fixture");
        Self(path)
    }
}

impl Drop for DeleteSandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
#[ignore = "manual paired isolated deletion benchmark"]
fn manual_observed_deletion_comparison() {
    for count in [20, 200, 2000] {
        let mut native = Vec::new();
        let mut observed = Vec::new();
        for iteration in 0..31 {
            for observe in [iteration % 2 == 0, iteration % 2 != 0] {
                let fixture = tempfile::tempdir().unwrap();
                let directory = fixture.path().join("target");
                fs::create_dir(&directory).unwrap();
                for index in 0..count {
                    fs::write(directory.join(format!("file-{index:05}")), [1; 64]).unwrap();
                }
                let prepared = prepare_path_for_permanent_delete(&directory).unwrap();
                let started = Instant::now();
                let removal = if observe {
                    RemovalMode::SnapshotDirectoryTree
                } else {
                    RemovalMode::DirectoryTree
                };
                delete_directly(prepared, count * 64, count, removal).unwrap();
                let sample = started.elapsed().as_secs_f64() * 1000.0;
                if observe {
                    observed.push(sample);
                } else {
                    native.push(sample);
                }
                assert!(!directory.exists());
            }
        }
        for (mode, mut samples) in [("native", native), ("analysis", observed)] {
            samples.sort_by(f64::total_cmp);
            println!("paired_delete_benchmark files={count} mode={mode} iterations={} p50_ms={:.3} p95_ms={:.3} max_ms={:.3}", samples.len(), samples[15], samples[29], samples[30]);
        }
    }
}

#[test]
fn deletion_diagnostics_preserve_native_error_and_kind() {
    let expected = std::io::Error::from_raw_os_error(5);
    let expected_kind = expected.kind();
    let error = deletion_entry_io::<()>(Path::new("fixture"), "unlink", Err(expected))
        .expect_err("diagnostics must preserve deletion failures");
    assert_eq!(error.raw_os_error(), Some(5));
    assert_eq!(error.kind(), expected_kind);
}

fn create_sparse_file(path: &Path, logical_bytes: u64) -> fs::File {
    let file = fs::File::create(path).expect("create the sparse deletion fixture");
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::{
            Foundation::HANDLE,
            System::{Ioctl::FSCTL_SET_SPARSE, IO::DeviceIoControl},
        };

        let mut returned = 0_u32;
        let marked_sparse = unsafe {
            DeviceIoControl(
                file.as_raw_handle() as HANDLE,
                FSCTL_SET_SPARSE,
                std::ptr::null(),
                0,
                std::ptr::null_mut(),
                0,
                &mut returned,
                std::ptr::null_mut(),
            )
        };
        assert_ne!(marked_sparse, 0, "mark the deletion fixture as sparse");
    }
    file.set_len(logical_bytes)
        .expect("extend the sparse deletion fixture");
    file
}

#[cfg(unix)]
struct UnixSocketFixture {
    path: PathBuf,
    _listener: std::os::unix::net::UnixListener,
}

#[cfg(unix)]
impl UnixSocketFixture {
    fn new() -> Self {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        // Unix-domain socket paths are short and bounded. `/tmp` is a
        // platform-approved system alias on macOS and a normal root on
        // other Unix systems.
        let path = PathBuf::from("/tmp").join(format!(
            "mangodisk-permanent-delete-socket-{}-{unique}.sock",
            std::process::id()
        ));
        let listener = std::os::unix::net::UnixListener::bind(&path)
            .expect("the Unix socket fixture should be bound");
        Self {
            path,
            _listener: listener,
        }
    }
}

#[cfg(unix)]
impl Drop for UnixSocketFixture {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

#[cfg(unix)]
fn create_fifo(path: &Path) {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};

    let native_path = CString::new(path.as_os_str().as_bytes())
        .expect("the FIFO fixture path should not contain a null byte");
    // SAFETY: `native_path` is a valid, null-terminated pathname and each
    // caller confines the fixture to an isolated test sandbox.
    let result = unsafe { libc::mkfifo(native_path.as_ptr(), 0o600) };
    assert_eq!(
        result,
        0,
        "the FIFO fixture should be created: {}",
        std::io::Error::last_os_error()
    );
}

#[cfg(unix)]
fn assert_special_file_is_rejected_promptly(path: &Path) {
    use std::{sync::mpsc, time::Duration};

    let target = path.to_path_buf();
    let (sender, receiver) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        let result = prepare_path_for_permanent_delete(&target)
            .map(|_| ())
            .map_err(|error| error.to_string());
        let _ = sender.send(result);
    });

    let result = receiver
        .recv_timeout(Duration::from_secs(1))
        .expect("special-file validation must not block");
    worker
        .join()
        .expect("the special-file validation worker should finish");
    let error = result.expect_err("special files must not enter permanent deletion");
    assert!(
        error.contains("only regular files and directories"),
        "the rejection should identify the unsupported file type"
    );
}

#[test]
fn permanent_delete_removes_a_matching_regular_file() {
    let sandbox = DeleteSandbox::new();
    let path = sandbox.0.join("large.bin");
    fs::write(&path, b"large-file-fixture").expect("write the deletion fixture");
    let metadata = fs::metadata(&path).expect("read the deletion fixture");
    let candidate = PermanentDeleteCandidate {
        path: path.to_string_lossy().into_owned(),
        expected_bytes: metadata.len(),
        expected_modified_at_ms: modified_ms(&metadata),
    };

    let expected_usage = current_platform().file_space_usage(&path, &metadata);
    let (_, usage) = delete_file_candidate_permanently(&candidate)
        .expect("a matching regular file should be permanently deleted");

    assert_eq!(usage, expected_usage);
    assert!(!path.exists());
}

#[test]
fn permanent_delete_reports_allocated_space_for_a_sparse_file() {
    let sandbox = DeleteSandbox::new();
    let path = sandbox.0.join("sparse.bin");
    let file = create_sparse_file(&path, 64 * 1024 * 1024);
    let metadata = file.metadata().expect("read the sparse deletion metadata");
    let expected_usage = current_platform().file_space_usage(&path, &metadata);
    assert!(expected_usage.allocated_bytes < expected_usage.logical_bytes);
    drop(file);
    let candidate = PermanentDeleteCandidate {
        path: path.to_string_lossy().into_owned(),
        expected_bytes: metadata.len(),
        expected_modified_at_ms: modified_ms(&metadata),
    };

    let (_, usage) = delete_file_candidate_permanently(&candidate)
        .expect("the sparse fixture should be permanently deleted");

    assert_eq!(usage, expected_usage);
    assert!(!path.exists());
}

#[cfg(unix)]
#[test]
fn permanent_delete_rejects_a_fifo_without_blocking_or_mutation() {
    use std::os::unix::fs::FileTypeExt;

    let sandbox = DeleteSandbox::new();
    let path = sandbox.0.join("stale.pipe");
    create_fifo(&path);

    assert_special_file_is_rejected_promptly(&path);

    let metadata = fs::symlink_metadata(&path).expect("the FIFO fixture should remain");
    assert!(metadata.file_type().is_fifo());
}

#[cfg(unix)]
#[test]
fn permanent_delete_rejects_a_unix_socket_without_blocking_or_mutation() {
    use std::os::unix::fs::FileTypeExt;

    let socket = UnixSocketFixture::new();
    assert_special_file_is_rejected_promptly(&socket.path);

    let metadata =
        fs::symlink_metadata(&socket.path).expect("the Unix socket fixture should remain");
    assert!(metadata.file_type().is_socket());
}

#[test]
fn permanent_delete_preserves_a_file_that_changed_after_scanning() {
    let sandbox = DeleteSandbox::new();
    let path = sandbox.0.join("changed.bin");
    fs::write(&path, b"original").expect("write the original deletion fixture");
    let metadata = fs::metadata(&path).expect("read the original deletion fixture");
    let candidate = PermanentDeleteCandidate {
        path: path.to_string_lossy().into_owned(),
        expected_bytes: metadata.len(),
        expected_modified_at_ms: modified_ms(&metadata),
    };
    fs::write(&path, b"changed-content").expect("change the deletion fixture");

    let error = delete_file_candidate_permanently(&candidate)
        .expect_err("a changed file must not be permanently deleted");

    assert!(error.to_string().contains("changed after scanning"));
    assert!(path.exists());
}

#[test]
fn permanent_delete_path_removes_a_preflighted_directory() {
    let sandbox = DeleteSandbox::new();
    let path = sandbox.0.join("directory");
    fs::create_dir_all(&path).expect("create the directory deletion fixture");
    fs::write(path.join("payload.bin"), b"payload").expect("write the directory deletion fixture");

    let prepared =
        prepare_path_for_permanent_delete(&path).expect("the directory target should be prepared");
    delete_path_permanently(prepared, b"payload".len() as u64, 1)
        .expect("a preflighted directory should be permanently deleted");

    assert!(!path.exists());
    assert!(
        fs::read_dir(&sandbox.0)
            .expect("read the deletion sandbox")
            .all(|entry| !entry
                .expect("inspect remaining sibling entries")
                .file_name()
                .to_string_lossy()
                .starts_with(".mangodisk-delete-")),
        "successful deletion must not leave a staging directory"
    );
}

#[test]
fn name_guard_preserves_descendants_created_after_preflight() {
    for kind in [
        mangodisk_platform::ExcludedNameKind::File,
        mangodisk_platform::ExcludedNameKind::Folder,
    ] {
        let sandbox = DeleteSandbox::new();
        let path = sandbox.0.join("guarded-directory");
        fs::create_dir(&path).unwrap();
        let prepared = prepare_path_for_permanent_delete(&path).unwrap();
        let names =
            mangodisk_platform::NameExclusions::compile(&[mangodisk_platform::ScanNameExclusion {
                name: "keep".into(),
                kind,
            }])
            .unwrap();
        // The parent identity remains valid when a protected child appears after preflight.
        let protected = path.join("keep");
        match kind {
            mangodisk_platform::ExcludedNameKind::File => {
                fs::write(&protected, b"protected").unwrap()
            }
            mangodisk_platform::ExcludedNameKind::Folder => fs::create_dir(&protected).unwrap(),
        }
        let error = delete_directory_tree_with_name_exclusions(prepared, 0, 0, &names, &|| false)
            .unwrap_err();
        assert!(error.to_string().contains("excluded name"));
        assert!(
            protected.exists(),
            "the matching child must survive deletion"
        );
        assert!(sandbox.0.read_dir().unwrap().all(|entry| !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".mangodisk-delete-")));
    }
}

#[test]
fn platform_name_guard_preserves_reserved_directory_created_after_preflight() {
    let sandbox = DeleteSandbox::new();
    let path = sandbox.0.join("guarded-directory");
    fs::create_dir(&path).unwrap();
    let prepared = prepare_path_for_permanent_delete(&path).unwrap();
    let mut names = mangodisk_platform::NameExclusions::default();
    names.protect_folder_names(&[".Spotlight-V100"]);
    let protected = path.join(".spotlight-v100");
    fs::create_dir(&protected).unwrap();
    fs::write(protected.join("payload.bin"), b"protected").unwrap();

    let error =
        delete_directory_tree_with_name_exclusions(prepared, 0, 0, &names, &|| false).unwrap_err();
    assert!(error.to_string().contains("excluded name"));
    assert!(protected.join("payload.bin").exists());
    assert!(sandbox.0.read_dir().unwrap().all(|entry| !entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".mangodisk-delete-")));
}

#[test]
fn cancellable_directory_delete_reports_live_removed_content() {
    let sandbox = DeleteSandbox::new();
    let path = sandbox.0.join("cancellable-directory");
    fs::create_dir_all(path.join("nested")).expect("create the cancellable fixture");
    fs::write(path.join("first.bin"), b"first").expect("write the first fixture");
    fs::write(path.join("nested/second.bin"), b"second").expect("write the second fixture");
    let prepared = prepare_path_for_permanent_delete(&path)
        .expect("the cancellable directory should be prepared");

    let outcome = delete_directory_tree_permanently_with_cancellation(prepared, 1, 1, &|| false)
        .expect("the cancellable directory should be removed");

    assert_eq!(outcome.released_bytes(), 11);
    assert_eq!(outcome.affected_item_count(), 2);
    assert!(!path.exists());
}

#[cfg(unix)]
#[test]
fn cancellable_directory_delete_unlinks_a_descendant_symlink_without_following_it() {
    use std::os::unix::fs::symlink;

    let sandbox = DeleteSandbox::new();
    let path = sandbox.0.join("cancellable-directory-with-link");
    let external = sandbox.0.join("external");
    let protected_file = external.join("must-remain.bin");
    fs::create_dir_all(&path).expect("create the cancellable fixture");
    fs::create_dir_all(&external).expect("create the external fixture");
    fs::write(&protected_file, b"protected").expect("write the protected fixture");
    symlink(&external, path.join("external-link")).expect("create the descendant symlink");
    let prepared = prepare_path_for_permanent_delete(&path)
        .expect("the directory containing the symlink should be prepared");

    let outcome = delete_directory_tree_permanently_with_cancellation(prepared, 0, 0, &|| false)
        .expect("the approved tree should be removed without following its symlink");

    assert_eq!(outcome.released_bytes(), 0);
    assert_eq!(outcome.affected_item_count(), 0);
    assert!(
        !path.exists(),
        "the approved directory tree must be removed"
    );
    assert!(
        protected_file.exists(),
        "the external symlink target must remain untouched"
    );
}

#[cfg(unix)]
#[test]
fn cancellation_after_descendant_symlink_unlink_reports_partial_deletion() {
    use std::os::unix::fs::symlink;

    let sandbox = DeleteSandbox::new();
    let path = sandbox.0.join("cancelled-directory-with-link");
    let external_link = path.join("external-link");
    let external = sandbox.0.join("external");
    let protected_file = external.join("must-remain.bin");
    fs::create_dir_all(&path).expect("create the cancellation fixture");
    fs::create_dir_all(&external).expect("create the external fixture");
    fs::write(&protected_file, b"protected").expect("write the protected fixture");
    symlink(&external, &external_link).expect("create the descendant symlink");
    let prepared = prepare_path_for_permanent_delete(&path)
        .expect("the directory containing the symlink should be prepared");
    let checks = AtomicU64::new(0);

    let error = delete_directory_tree_permanently_with_cancellation(prepared, 0, 0, &|| {
        checks.fetch_add(1, Ordering::Relaxed) >= 3
    })
    .expect_err("cancellation should stop deletion after the symlink is unlinked");

    assert!(error.is_partial());
    assert_eq!(error.released_bytes(), 0);
    assert_eq!(error.affected_item_count(), 0);
    assert_eq!(checks.load(Ordering::Relaxed), 4);
    assert!(
        path.exists(),
        "the remaining directory must remain in place"
    );
    assert!(
        fs::symlink_metadata(&external_link).is_err(),
        "the unlinked descendant symlink must not be restored"
    );
    assert!(
        protected_file.exists(),
        "the external symlink target must remain untouched"
    );
}

#[cfg(unix)]
#[test]
fn directory_contents_delete_unlinks_a_descendant_symlink_without_following_it() {
    use std::os::unix::fs::symlink;

    let sandbox = DeleteSandbox::new();
    let path = sandbox.0.join("directory-contents-with-link");
    let retained_empty = path.join("retained-empty");
    let external_link = path.join("external-link");
    let external = sandbox.0.join("external");
    let protected_file = external.join("must-remain.bin");
    fs::create_dir_all(&retained_empty).expect("create the retained empty directory");
    fs::create_dir_all(&external).expect("create the external fixture");
    fs::write(&protected_file, b"protected").expect("write the protected fixture");
    symlink(&external, &external_link).expect("create the descendant symlink");
    let prepared = prepare_path_for_permanent_delete(&path)
        .expect("the directory containing the symlink should be prepared");

    let outcome = delete_directory_contents_permanently_with_cancellation(prepared, &|| false)
        .expect("the approved contents should be removed without following their symlink");

    assert_eq!(outcome.released_bytes(), 0);
    assert_eq!(outcome.affected_item_count(), 0);
    assert!(
        path.exists(),
        "the retained directory skeleton must remain in place"
    );
    assert!(
        retained_empty.exists(),
        "the pre-existing empty directory must remain"
    );
    assert!(
        fs::symlink_metadata(&external_link).is_err(),
        "the descendant symlink entry must be removed"
    );
    assert!(
        protected_file.exists(),
        "the external symlink target must remain untouched"
    );
}

#[test]
fn cancellable_directory_delete_retains_remaining_content() {
    use std::sync::atomic::{AtomicU64, Ordering};

    let sandbox = DeleteSandbox::new();
    let path = sandbox.0.join("cancelled-directory");
    fs::create_dir_all(&path).expect("create the cancellation fixture");
    for index in 0..32 {
        fs::write(path.join(format!("{index:02}.cache")), b"cache")
            .expect("write the cancellation fixture");
    }
    let prepared = prepare_path_for_permanent_delete(&path)
        .expect("the cancelled directory should be prepared");
    let checks = AtomicU64::new(0);

    let error = delete_directory_tree_permanently_with_cancellation(prepared, 32 * 5, 32, &|| {
        checks.fetch_add(1, Ordering::Relaxed) >= 5
    })
    .expect_err("cancellation should stop the direct directory traversal");

    assert!(error.is_partial());
    assert!(error.released_bytes() > 0);
    assert!(error.released_bytes() < 32 * 5);
    assert!(error.affected_item_count() > 0);
    assert!(error.affected_item_count() < 32);
    assert!(path.exists(), "remaining content must remain in place");
    assert_eq!(
        fs::read_dir(&path)
            .expect("read the remaining directory")
            .count() as u64
            + error.affected_item_count(),
        32
    );
}

/// Measures return latency after cancellation during a large small-file delete.
///
/// The diagnostic creates a large temporary dataset and is ignored by
/// default. Its output contains only counts and timings, never fixture paths.
#[test]
#[ignore = "filesystem cancellation latency benchmark"]
fn benchmark_cancellable_directory_delete_latency() {
    use std::{
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        },
        time::{Duration, Instant},
    };

    let file_count = std::env::var("MANGODISK_CLEANUP_BENCHMARK_FILE_COUNT")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|count| *count > 0)
        .unwrap_or(20_000);
    let sandbox = DeleteSandbox::new();
    let path = sandbox.0.join("cancellation-latency");
    fs::create_dir_all(&path).expect("create the cancellation benchmark root");
    for index in 0..file_count {
        fs::write(path.join(format!("{index:08}.cache")), b"cache")
            .expect("write the cancellation benchmark file");
    }
    let prepared = prepare_path_for_permanent_delete(&path)
        .expect("prepare the cancellation benchmark directory");
    let cancelled = Arc::new(AtomicBool::new(false));
    let cancellation_signal = Arc::clone(&cancelled);
    let signal_delay = Duration::from_millis(50);
    let signal_thread = std::thread::spawn(move || {
        std::thread::sleep(signal_delay);
        cancellation_signal.store(true, Ordering::Release);
    });

    let started = Instant::now();
    let error = delete_directory_tree_permanently_with_cancellation(
        prepared,
        file_count * 5,
        file_count,
        &|| cancelled.load(Ordering::Acquire),
    )
    .expect_err("the asynchronous cancellation signal should stop deletion");
    let total_elapsed = started.elapsed();
    signal_thread
        .join()
        .expect("join the cancellation signal thread");
    let response_elapsed = total_elapsed.saturating_sub(signal_delay);

    assert!(
        path.exists(),
        "remaining files must remain in place after cancellation"
    );
    assert!(error.affected_item_count() < file_count);
    assert!(
        response_elapsed < Duration::from_millis(500),
        "cancellation should be observed within 500 ms"
    );
    println!(
        "cleanup_whole_root_cancellation file_count={file_count} total_ms={:.2} response_ms={:.2} affected_item_count={}",
        total_elapsed.as_secs_f64() * 1_000.0,
        response_elapsed.as_secs_f64() * 1_000.0,
        error.affected_item_count()
    );
}

#[cfg(windows)]
#[test]
fn windows_delete_errors_preserve_actionable_failure_reasons() {
    assert_eq!(
        permanent_delete_io_reason(&std::io::Error::from_raw_os_error(32)),
        Some(CoreErrorReason::ResourceBusy)
    );
    assert_eq!(
        permanent_delete_io_reason(&std::io::Error::from_raw_os_error(5)),
        Some(CoreErrorReason::AccessDeniedOrBusy)
    );
}

#[test]
fn permanent_delete_preserves_a_same_path_replacement() {
    let sandbox = DeleteSandbox::new();
    let path = sandbox.0.join("directory");
    let moved_original = sandbox.0.join("moved-original");
    fs::create_dir_all(&path).expect("create the original deletion target");
    fs::write(path.join("original.bin"), b"original").expect("write the original deletion target");

    let prepared =
        prepare_path_for_permanent_delete(&path).expect("the original target should be prepared");
    fs::rename(&path, &moved_original).expect("move the original target aside");
    fs::create_dir_all(&path).expect("create the same-path replacement");
    fs::write(path.join("replacement.bin"), b"replacement")
        .expect("write the same-path replacement");

    let error = delete_path_permanently(prepared, b"original".len() as u64, 1)
        .expect_err("a same-path replacement must not be deleted");

    assert!(error.to_string().contains("replaced"));
    assert!(
        path.join("replacement.bin").exists(),
        "the same-path replacement must remain in place"
    );
    assert!(
        moved_original.join("original.bin").exists(),
        "the original object must remain untouched after being moved"
    );
}

#[test]
fn empty_directory_delete_preserves_a_concurrent_new_file() {
    let sandbox = DeleteSandbox::new();
    let path = sandbox.0.join("directory");
    fs::create_dir_all(&path).expect("create the empty directory fixture");
    let prepared =
        prepare_path_for_permanent_delete(&path).expect("the empty directory should be prepared");
    fs::write(path.join("concurrent.bin"), b"concurrent")
        .expect("write a file after the directory was prepared");

    let error = delete_empty_directory_permanently(prepared)
        .expect_err("a directory that became non-empty must not be deleted");

    assert!(error.to_string().contains("remaining entries"));
    assert!(
        path.join("concurrent.bin").exists(),
        "a concurrently created file must remain visible"
    );
}

#[test]
fn directory_not_empty_has_a_stable_failure_reason() {
    let error = std::io::Error::from(std::io::ErrorKind::DirectoryNotEmpty);
    assert_eq!(
        permanent_delete_io_reason(&error),
        Some(CoreErrorReason::DirectoryNotEmpty)
    );
}

#[cfg(target_os = "macos")]
#[test]
fn permanent_delete_retains_remaining_directory_after_recursive_failure() {
    use std::os::unix::fs::PermissionsExt;

    let sandbox = DeleteSandbox::new();
    let path = sandbox.0.join("directory");
    let blocked = path.join("blocked");
    let payload = blocked.join("payload.bin");
    fs::create_dir_all(&blocked).expect("create the deletion fixture");
    fs::write(&payload, b"payload").expect("write the deletion fixture");
    fs::set_permissions(&blocked, fs::Permissions::from_mode(0o000))
        .expect("block traversal of the deletion fixture");

    let prepared =
        prepare_path_for_permanent_delete(&path).expect("the deletion target should be prepared");
    let error = delete_path_permanently(prepared, b"payload".len() as u64, 1)
        .expect_err("an unreadable descendant should fail recursive deletion");

    assert!(
        error.to_string().contains("remaining entries"),
        "the failure must explain that remaining contents remain in place"
    );
    assert!(
        path.exists(),
        "the remaining directory must stay at its original path"
    );
    fs::set_permissions(&blocked, fs::Permissions::from_mode(0o700))
        .expect("restore fixture permissions");
    assert!(
        payload.exists(),
        "the blocked payload must remain available"
    );
}
