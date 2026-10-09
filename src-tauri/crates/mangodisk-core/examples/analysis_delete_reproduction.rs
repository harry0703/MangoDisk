//! Isolated deletion probe: `analysis_delete_reproduction [idle|writer|recreate] [files] [names]`.
//! The writer retains a directory handle across staging, like a process using
//! an already-open working directory. All files and application state are temporary.

#[cfg(unix)]
mod probe {
    use std::{
        ffi::CString,
        fs,
        os::fd::AsRawFd,
        path::Path,
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc, Barrier,
        },
        thread,
        time::{Duration, Instant},
    };

    use mangodisk_core::{AnalysisService, ApplicationPaths};

    struct Logger;
    impl log::Log for Logger {
        fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
            metadata.level() <= log::Level::Info
        }
        fn log(&self, record: &log::Record<'_>) {
            if self.enabled(record.metadata()) {
                eprintln!("{} {} {}", record.level(), record.target(), record.args());
            }
        }
        fn flush(&self) {}
    }
    static LOGGER: Logger = Logger;

    fn count_files(path: &Path) -> std::io::Result<u64> {
        if !path.exists() {
            return Ok(0);
        }
        let mut count = 0;
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                count += count_files(&entry.path())?;
            } else {
                count += 1;
            }
        }
        Ok(count)
    }

    pub fn run() -> Result<(), Box<dyn std::error::Error>> {
        let mode = std::env::args().nth(1).unwrap_or_else(|| "idle".into());
        if !matches!(mode.as_str(), "idle" | "writer" | "recreate") {
            return Err("expected idle, writer, or recreate".into());
        }
        log::set_logger(&LOGGER).map_err(|error| error.to_string())?;
        log::set_max_level(log::LevelFilter::Info);
        let sandbox = tempfile::Builder::new()
            .prefix("mangodisk-delete-probe-")
            .tempdir()?;
        let root = sandbox.path().canonicalize()?;
        let scanned_root = root.join("scan");
        let target = scanned_root.join("selected");
        let writing = target.join("active-writer");
        fs::create_dir_all(&writing)?;
        mangodisk_core::configure_application_paths(ApplicationPaths::new(
            root.join("data"),
            root.join("cache"),
            root.join("runtime"),
        )?)?;
        let initial_files: u64 = std::env::args()
            .nth(2)
            .map(|value| value.parse())
            .transpose()?
            .unwrap_or(20_000);
        if initial_files > 400_000 {
            return Err("fixture limit is 400000 files".into());
        }
        let names_enabled = std::env::args().nth(3).as_deref() == Some("names");
        for index in 0..initial_files {
            fs::write(writing.join(format!("initial-{index}")), b"fixture")?;
        }
        let exclusions = mangodisk_core::ScanExclusionOptions {
            paths: Vec::new(),
            names: if names_enabled {
                vec![mangodisk_core::ScanNameExclusion {
                    name: "never-created-exclusion".into(),
                    kind: mangodisk_core::ExcludedNameKind::Folder,
                }]
            } else {
                Vec::new()
            },
        };
        let scan = AnalysisService::analyze_with_exclusions_progress(
            Some(scanned_root.to_string_lossy().into_owned()),
            true,
            exclusions,
            |_| {},
        )?;
        let handle = fs::File::open(&writing)?;
        let stop = Arc::new(AtomicBool::new(false));
        let start = Arc::new(Barrier::new(2));
        let writer = if mode == "writer" {
            let stop = Arc::clone(&stop);
            let start = Arc::clone(&start);
            Some(thread::spawn(move || {
                let deadline = Instant::now() + Duration::from_secs(10);
                let mut created = 0_u64;
                let mut last_error = None;
                start.wait();
                // Unique names make initial + created - remaining an independent
                // actual deletion count. The writer never removes any entry.
                while !stop.load(Ordering::Relaxed)
                    && Instant::now() < deadline
                    && created < 100_000
                {
                    let name = CString::new(format!("new-{created}")).unwrap();
                    // SAFETY: the owned directory and CString outlive this call;
                    // O_EXCL/O_NOFOLLOW prevent following or overwriting an entry.
                    let fd = unsafe {
                        libc::openat(
                            handle.as_raw_fd(),
                            name.as_ptr(),
                            libc::O_WRONLY
                                | libc::O_CREAT
                                | libc::O_EXCL
                                | libc::O_NOFOLLOW
                                | libc::O_CLOEXEC,
                            0o600,
                        )
                    };
                    if fd < 0 {
                        last_error = std::io::Error::last_os_error().raw_os_error();
                        break;
                    }
                    // SAFETY: openat returned a new descriptor owned by this loop.
                    unsafe {
                        libc::close(fd);
                    }
                    created += 1;
                    if created.is_multiple_of(128) {
                        thread::yield_now();
                    }
                }
                (created, last_error)
            }))
        } else if mode == "recreate" {
            let target = target.clone();
            let start = Arc::clone(&start);
            Some(thread::spawn(move || {
                start.wait();
                let deadline = Instant::now() + Duration::from_secs(10);
                while target.exists() && Instant::now() < deadline {
                    thread::yield_now();
                }
                fs::create_dir(&target).expect("recreate original directory");
                fs::write(target.join("new-content"), b"preserve me").unwrap();
                (1, None)
            }))
        } else {
            None
        };
        if writer.is_some() {
            start.wait();
        }
        let started = Instant::now();
        let result = AnalysisService::delete_entry_permanently(
            scan.scan_id,
            target.to_string_lossy().into_owned(),
        );
        let delete_ms = started.elapsed().as_millis();
        stop.store(true, Ordering::Relaxed);
        let (created, writer_error) = writer
            .map(|worker| worker.join().expect("writer must finish"))
            .unwrap_or_default();
        let remaining = count_files(&target)?;
        let actual_removed = initial_files + created - remaining;
        let staging_left = fs::read_dir(&scanned_root)?
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name() != "selected")
            .count();
        println!("probe mode={mode} initial_files={initial_files} names_enabled={names_enabled} writer_created={created} writer_error={writer_error:?} remaining_files={remaining} actual_removed_files={actual_removed} target_exists={} staging_left={staging_left} elapsed_ms={delete_ms} result={result:?}", target.exists());
        if mode == "recreate" {
            assert!(result.as_ref().unwrap().requires_rescan);
            assert!(AnalysisService::resolve_open_target(
                scan.scan_id,
                target.to_string_lossy().into_owned(),
            )
            .is_err());
            assert_eq!(fs::read(target.join("new-content"))?, b"preserve me");
            let refreshed = AnalysisService::analyze_with_progress(
                Some(scanned_root.to_string_lossy().into_owned()),
                true,
                |_| {},
            )?;
            assert_eq!(refreshed.entries[0].file_count, 1);
            println!("recreated_original verification=passed old_session=expired new_content=preserved refreshed_files=1");
        }
        // A fresh scan after the writer stops must permit deleting the remainder.
        if result.is_err() && target.exists() {
            let scan = AnalysisService::analyze_with_progress(
                Some(scanned_root.to_string_lossy().into_owned()),
                true,
                |_| {},
            )?;
            let retry = AnalysisService::delete_entry_permanently(
                scan.scan_id,
                target.to_string_lossy().into_owned(),
            );
            println!(
                "retry writer_stopped=true target_exists={} result={retry:?}",
                target.exists()
            );
            retry?;
        }
        if mode == "idle" {
            result?;
        }
        Ok(())
    }
}

#[cfg(windows)]
mod windows_probe {
    use std::{fs, os::windows::fs::OpenOptionsExt, time::Instant};

    use mangodisk_core::{AnalysisService, ApplicationPaths};

    /// A deny-all file handle verifies that Windows rejects staging before
    /// mutation and permits a fresh delete after the handle is released.
    pub fn run() -> Result<(), Box<dyn std::error::Error>> {
        let sandbox = tempfile::Builder::new()
            .prefix("mangodisk-delete-probe-")
            .tempdir()?;
        let root = sandbox.path().canonicalize()?;
        let scanned_root = root.join("scan");
        let target = scanned_root.join("selected");
        fs::create_dir_all(&target)?;
        mangodisk_core::configure_application_paths(ApplicationPaths::new(
            root.join("data"),
            root.join("cache"),
            root.join("runtime"),
        )?)?;
        let initial_files: usize = std::env::args()
            .nth(2)
            .map(|value| value.parse())
            .transpose()?
            .unwrap_or(20_000);
        if initial_files > 100_000 {
            return Err("fixture limit is 100000 files".into());
        }
        for index in 0..initial_files {
            fs::write(target.join(format!("initial-{index}")), b"fixture")?;
        }
        let locked_path = target.join("locked-file");
        fs::write(&locked_path, b"keep until retry")?;
        let scan = AnalysisService::analyze_with_progress(
            Some(scanned_root.to_string_lossy().into_owned()),
            true,
            |_| {},
        )?;
        let selected_path = scan
            .entries
            .iter()
            .find(|entry| entry.name == "selected")
            .ok_or("the fixture was not published by the scan")?
            .path
            .clone();
        let handle = fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&locked_path)?;
        let started = Instant::now();
        let first = AnalysisService::delete_entry_permanently(scan.scan_id, selected_path.clone());
        println!(
            "windows_probe files={} first_elapsed_ms={} first={first:?} target_exists={} locked_content_exists={}",
            initial_files,
            started.elapsed().as_millis(),
            target.exists(),
            locked_path.exists()
        );
        drop(handle);
        let first_error = first.expect_err("an exclusive file handle must stop deletion");
        if first_error.mutation_state() != mangodisk_platform::PlatformMutationState::NotAttempted {
            return Err(
                format!("expected staging to stop before mutation: {first_error:?}").into(),
            );
        }
        if !target.exists() || !locked_path.exists() {
            return Err("the rejected delete changed the selected directory".into());
        }
        if AnalysisService::resolve_open_target(scan.scan_id, selected_path).is_err() {
            return Err("the untouched analysis session became invalid".into());
        }
        let retry_scan = AnalysisService::analyze_with_progress(
            Some(scanned_root.to_string_lossy().into_owned()),
            true,
            |_| {},
        )?;
        let retry_path = retry_scan
            .entries
            .iter()
            .find(|entry| entry.name == "selected")
            .ok_or("the remainder was not published by the fresh scan")?
            .path
            .clone();
        let retry_started = Instant::now();
        let retry = AnalysisService::delete_entry_permanently(retry_scan.scan_id, retry_path);
        println!(
            "windows_probe retry_elapsed_ms={} retry={retry:?} target_exists={}",
            retry_started.elapsed().as_millis(),
            target.exists()
        );
        retry?;
        if target.exists() {
            return Err("retry left the selected directory behind".into());
        }
        Ok(())
    }
}

fn main() {
    #[cfg(unix)]
    probe::run().expect("isolated deletion probe should complete");
    #[cfg(windows)]
    windows_probe::run().expect("isolated Windows deletion probe should complete");
}
