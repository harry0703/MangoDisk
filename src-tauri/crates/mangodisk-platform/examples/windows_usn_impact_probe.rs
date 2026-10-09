#[cfg(not(windows))]
fn main() {
    println!("windows_usn_impact_probe status=unsupported platform=non_windows");
}

#[cfg(windows)]
fn main() {
    if let Err(error) = windows_probe::run() {
        eprintln!("windows_usn_impact_probe status=failed error={error}");
        std::process::exit(1);
    }
}

#[cfg(windows)]
mod windows_probe {
    use std::{
        collections::{HashMap, HashSet},
        env, fs,
        io::Write,
        os::windows::ffi::OsStrExt,
        path::{Path, PathBuf},
        sync::{
            atomic::{AtomicUsize, Ordering},
            Mutex,
        },
        time::Instant,
    };

    use mangodisk_platform::{
        current_platform, FastAnalysisRecord, FastAnalysisScanError, FilesystemChangeImpactError,
        FilesystemChangeImpactOutcome, FilesystemChangeToken, Platform,
    };
    use windows_sys::Win32::System::{
        ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS},
        Threading::GetCurrentProcess,
    };

    const DEFAULT_SCALE_COUNT: usize = 100_000;

    struct AnalysisSnapshot {
        directories: HashMap<String, (PathBuf, u64, u64, u64)>,
        elapsed_ms: u128,
    }

    struct AnalysisComparison {
        changed_directories: usize,
        covered: bool,
        before_elapsed_ms: u128,
        after_elapsed_ms: u128,
    }

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum ProbeMode {
        NoChanges,
        Create,
        Modify,
        Delete,
        Rename,
        CrossMove,
        DeleteTree,
        HardLink,
        CreateMany,
        Cancel,
        CancelMidRead,
        FixedUpperBound,
        InvalidToken,
        Subdirectory,
    }

    pub(super) fn run() -> Result<(), String> {
        let root = env::args()
            .nth(1)
            .map(PathBuf::from)
            .unwrap_or_else(|| current_platform().system_volume_path());
        let mode = parse_mode(env::args().nth(2).as_deref())?;
        let scale_count = env::args()
            .nth(3)
            .map(|value| {
                value
                    .parse::<usize>()
                    .map_err(|_| "scale is not a positive integer".to_string())
                    .and_then(|count| {
                        (count > 0)
                            .then_some(count)
                            .ok_or_else(|| "scale must exceed zero".to_string())
                    })
            })
            .transpose()?
            .unwrap_or(DEFAULT_SCALE_COUNT);
        // A fixed fixture name gives the same event matrix a stable dirty digest. If an earlier
        // abnormal exit left the directory behind, refuse to overwrite it instead of deleting
        // automatically so the diagnostic cannot cross its ownership boundary.
        let fixture = root.join("mangodisk-impact-probe-v1");
        if fixture.exists() {
            return Err(
                "the controlled fixture already exists; inspect and remove it manually".to_string(),
            );
        }
        let result = run_fixture(&root, &fixture, mode, scale_count);
        // The probe owns fixture cleanup and performs it after impact planning, so cleanup cannot
        // contaminate the current result. On failure, still attempt cleanup while preserving the
        // original error so cleanup noise cannot hide a Journal problem.
        let cleanup = fs::remove_dir_all(&fixture);
        match (result, cleanup) {
            (Err(error), _) => Err(error),
            (Ok(()), Err(error)) if error.kind() != std::io::ErrorKind::NotFound => {
                Err(format!("failed to remove the controlled fixture: {error}"))
            }
            (Ok(()), _) => Ok(()),
        }
    }

    fn run_fixture(
        root: &Path,
        fixture: &Path,
        mode: ProbeMode,
        scale_count: usize,
    ) -> Result<(), String> {
        let left = fixture.join("left");
        let right = fixture.join("right");
        let tree = fixture.join("tree");
        fs::create_dir_all(&left)
            .map_err(|error| format!("failed to create left fixture: {error}"))?;
        fs::create_dir_all(&right)
            .map_err(|error| format!("failed to create right fixture: {error}"))?;

        prepare_before_token(mode, &left, &tree)?;
        let platform = current_platform();
        let before_analysis = if mode_supports_analysis_comparison(mode) {
            Some(analysis_snapshot(root, &platform)?)
        } else {
            None
        };
        let token = platform
            .capture_filesystem_change_token(root)
            .map_err(|error| format!("failed to capture USN token: {error}"))?
            .ok_or_else(|| "the current volume cannot capture a USN token".to_string())?;

        if mode == ProbeMode::Cancel {
            let started = Instant::now();
            let result = platform.filesystem_change_impact_plan(root, &token, &|| true);
            return match result {
                Err(FilesystemChangeImpactError::Cancelled) => {
                    println!(
                        "windows_usn_impact_probe status=cancelled mode=cancel elapsed_ms={}",
                        started.elapsed().as_millis()
                    );
                    Ok(())
                }
                _ => Err("pre-cancellation did not return Cancelled".to_string()),
            };
        }
        if mode == ProbeMode::CancelMidRead {
            mutate_after_token(ProbeMode::CreateMany, &left, &right, &tree, scale_count)?;
            return run_mid_read_cancellation(root, &token, &platform);
        }
        if mode == ProbeMode::FixedUpperBound {
            return run_fixed_upper_bound(root, &left, &token, &platform);
        }
        if mode == ProbeMode::InvalidToken {
            let mut invalid = token;
            invalid.history_id ^= 1;
            return print_plan(root, mode, &invalid, &[], None, &platform);
        }
        if mode == ProbeMode::Subdirectory {
            return print_plan(&left, mode, &token, &[], None, &platform);
        }

        let expected = mutate_after_token(mode, &left, &right, &tree, scale_count)?;
        print_plan(
            root,
            mode,
            &token,
            &expected,
            before_analysis.as_ref(),
            &platform,
        )
    }

    fn prepare_before_token(mode: ProbeMode, left: &Path, tree: &Path) -> Result<(), String> {
        match mode {
            ProbeMode::Modify | ProbeMode::Delete | ProbeMode::Rename | ProbeMode::CrossMove => {
                fs::write(left.join("existing.bin"), b"before")
                    .map_err(|error| format!("failed to prepare file fixture: {error}"))?;
            }
            ProbeMode::HardLink => {
                fs::write(left.join("source.bin"), b"hard-link")
                    .map_err(|error| format!("failed to prepare hard-link fixture: {error}"))?;
            }
            ProbeMode::DeleteTree => {
                fs::create_dir_all(tree.join("child")).map_err(|error| {
                    format!("failed to prepare directory-tree fixture: {error}")
                })?;
                fs::write(tree.join("child").join("sample.bin"), b"tree")
                    .map_err(|error| format!("failed to prepare directory-tree file: {error}"))?;
            }
            _ => {}
        }
        Ok(())
    }

    fn mutate_after_token(
        mode: ProbeMode,
        left: &Path,
        right: &Path,
        tree: &Path,
        scale_count: usize,
    ) -> Result<Vec<PathBuf>, String> {
        match mode {
            ProbeMode::NoChanges => Ok(Vec::new()),
            ProbeMode::Create => {
                fs::write(left.join("created.bin"), b"created")
                    .map_err(|error| format!("failed to create file: {error}"))?;
                Ok(vec![left.to_path_buf()])
            }
            ProbeMode::Modify => {
                let mut file = fs::OpenOptions::new()
                    .append(true)
                    .open(left.join("existing.bin"))
                    .map_err(|error| format!("failed to open modification fixture: {error}"))?;
                file.write_all(b"-changed")
                    .map_err(|error| format!("failed to modify fixture: {error}"))?;
                Ok(vec![left.to_path_buf()])
            }
            ProbeMode::Delete => {
                fs::remove_file(left.join("existing.bin"))
                    .map_err(|error| format!("failed to remove file fixture: {error}"))?;
                Ok(vec![left.to_path_buf()])
            }
            ProbeMode::Rename => {
                fs::rename(left.join("existing.bin"), left.join("renamed.bin"))
                    .map_err(|error| format!("failed to rename within one directory: {error}"))?;
                Ok(vec![left.to_path_buf()])
            }
            ProbeMode::CrossMove => {
                fs::rename(left.join("existing.bin"), right.join("moved.bin"))
                    .map_err(|error| format!("failed to move across directories: {error}"))?;
                Ok(vec![left.to_path_buf(), right.to_path_buf()])
            }
            ProbeMode::DeleteTree => {
                fs::remove_dir_all(tree)
                    .map_err(|error| format!("failed to remove directory-tree fixture: {error}"))?;
                Ok(vec![tree
                    .parent()
                    .ok_or_else(|| "directory-tree fixture has no parent".to_string())?
                    .to_path_buf()])
            }
            ProbeMode::HardLink => {
                fs::hard_link(left.join("source.bin"), right.join("linked.bin"))
                    .map_err(|error| format!("failed to create hard-link fixture: {error}"))?;
                // A new hard link adds an entry only to the target directory; the source
                // directory's file entry, size, and child count remain unchanged. Requiring both
                // parents would misclassify a safe plan as incomplete and widen the local rescan.
                Ok(vec![right.to_path_buf()])
            }
            ProbeMode::CreateMany => {
                for index in 0..scale_count {
                    fs::write(left.join(format!("record-{index:06}.tmp")), [])
                        .map_err(|error| format!("failed to create scale fixture: {error}"))?;
                }
                Ok(vec![left.to_path_buf()])
            }
            ProbeMode::Cancel
            | ProbeMode::CancelMidRead
            | ProbeMode::FixedUpperBound
            | ProbeMode::InvalidToken
            | ProbeMode::Subdirectory => Err("invalid probe mode execution order".to_string()),
        }
    }

    fn run_mid_read_cancellation(
        root: &Path,
        token: &FilesystemChangeToken,
        platform: &impl Platform,
    ) -> Result<(), String> {
        let checks = AtomicUsize::new(0);
        let started = Instant::now();
        let outcome = platform.filesystem_change_impact_plan(root, token, &|| {
            checks.fetch_add(1, Ordering::Relaxed) >= 1_024
        });
        match outcome {
            Err(FilesystemChangeImpactError::Cancelled) => {
                println!(
                    "windows_usn_impact_probe status=cancelled mode=cancel_mid_read checks={} elapsed_ms={}",
                    checks.load(Ordering::Relaxed),
                    started.elapsed().as_millis()
                );
                Ok(())
            }
            _ => Err("mid-page cancellation did not return Cancelled".to_string()),
        }
    }

    fn run_fixed_upper_bound(
        root: &Path,
        left: &Path,
        token: &FilesystemChangeToken,
        platform: &impl Platform,
    ) -> Result<(), String> {
        let checks = AtomicUsize::new(0);
        let write_error = Mutex::<Option<String>>::new(None);
        let first = platform
            .filesystem_change_impact_plan(root, token, &|| {
                let call = checks.fetch_add(1, Ordering::Relaxed) + 1;
                // The second cancellation check happens after querying the Journal upper bound
                // and before reading the first page. Injecting a change here deterministically
                // verifies the half-open interval without thread scheduling or a fixed sleep.
                if call == 2 {
                    if let Err(error) = fs::write(left.join("after-upper.bin"), b"after-upper") {
                        *write_error
                            .lock()
                            .expect("write error lock must not be poisoned") =
                            Some(error.to_string());
                    }
                }
                false
            })
            .map_err(format_impact_error)?
            .ok_or_else(|| "Windows Platform did not provide impact planning".to_string())?;
        if let Some(error) = write_error
            .into_inner()
            .map_err(|_| "write error lock is poisoned".to_string())?
        {
            return Err(format!("fixed-upper-bound fixture write failed: {error}"));
        }
        let FilesystemChangeImpactOutcome::Complete(first_plan) = first else {
            return Err("fixed-upper-bound first stage did not return Complete".to_string());
        };
        if first_plan.has_changes() {
            return Err(
                "an event injected after the upper bound entered the current plan".to_string(),
            );
        }

        let second = platform
            .filesystem_change_impact_plan(root, &first_plan.next_token, &|| false)
            .map_err(format_impact_error)?
            .ok_or_else(|| {
                "Windows Platform did not provide second-stage impact planning".to_string()
            })?;
        let FilesystemChangeImpactOutcome::Complete(second_plan) = second else {
            return Err("fixed-upper-bound second stage did not return Complete".to_string());
        };
        let covered = second_plan
            .dirty_directories
            .iter()
            .any(|dirty| path_is_same_or_child(left, dirty));
        println!(
            "windows_usn_impact_probe status=complete mode=fixed_upper_bound first_start={} first_end={} first_dirty={} second_start={} second_end={} second_dirty={} expected_covered={}",
            first_plan.summary.start_cursor,
            first_plan.summary.end_cursor,
            first_plan.summary.dirty_directory_count,
            second_plan.summary.start_cursor,
            second_plan.summary.end_cursor,
            second_plan.summary.dirty_directory_count,
            covered
        );
        if !covered {
            return Err("the next plan did not cover the event after the upper bound".to_string());
        }
        Ok(())
    }

    fn print_plan(
        root: &Path,
        mode: ProbeMode,
        token: &FilesystemChangeToken,
        expected: &[PathBuf],
        before_analysis: Option<&AnalysisSnapshot>,
        platform: &impl Platform,
    ) -> Result<(), String> {
        let started = Instant::now();
        let outcome = platform
            .filesystem_change_impact_plan(root, token, &|| false)
            .map_err(format_impact_error)?
            .ok_or_else(|| "Windows Platform did not provide impact planning".to_string())?;
        let elapsed_ms = started.elapsed().as_millis();
        let (peak_working_set_bytes, working_set_bytes) = process_memory_bytes()?;
        match outcome {
            FilesystemChangeImpactOutcome::Unavailable(reason) => {
                println!(
                    "windows_usn_impact_probe status=unavailable mode={} reason={} root_digest={} elapsed_ms={} peak_working_set_bytes={} working_set_bytes={}",
                    mode_name(mode),
                    reason.as_str(),
                    path_digest_hex(root),
                    elapsed_ms,
                    peak_working_set_bytes,
                    working_set_bytes
                );
            }
            FilesystemChangeImpactOutcome::Complete(plan) => {
                let analysis =
                    compare_analysis(root, before_analysis, &plan.dirty_directories, platform)?;
                let mut dirty_digests = plan
                    .dirty_directories
                    .iter()
                    .map(|path| path_digest_hex(path))
                    .collect::<Vec<_>>();
                dirty_digests.sort_unstable();
                let expected_covered = expected.iter().all(|path| {
                    plan.dirty_directories
                        .iter()
                        .any(|dirty| path_is_same_or_child(path, dirty))
                });
                println!(
                    "windows_usn_impact_probe status=complete mode={} root_digest={} start_cursor={} end_cursor={} next_cursor={} pages={} records={} data={} create_delete={} rename={} metadata={} other={} directory_records={} parent_cache_peak={} dirty_directories={} dirty_digest={} expected_paths={} expected_covered={} analysis_changed_directories={} analysis_covered={} analysis_before_ms={} analysis_after_ms={} returned_bytes={} plan_elapsed_ms={} elapsed_ms={} peak_working_set_bytes={} working_set_bytes={}",
                    mode_name(mode),
                    path_digest_hex(root),
                    plan.summary.start_cursor,
                    plan.summary.end_cursor,
                    plan.next_token.cursor,
                    plan.summary.page_count,
                    plan.summary.record_count,
                    plan.summary.data_change_records,
                    plan.summary.create_delete_records,
                    plan.summary.rename_records,
                    plan.summary.metadata_change_records,
                    plan.summary.other_records,
                    plan.summary.directory_records,
                    plan.summary.parent_cache_peak,
                    plan.summary.dirty_directory_count,
                    digest_list(&dirty_digests),
                    expected.len(),
                    expected_covered,
                    analysis.changed_directories,
                    analysis.covered,
                    analysis.before_elapsed_ms,
                    analysis.after_elapsed_ms,
                    plan.summary.returned_bytes,
                    plan.summary.elapsed_ms,
                    elapsed_ms,
                    peak_working_set_bytes,
                    working_set_bytes
                );
                if plan.next_token.cursor != plan.summary.end_cursor {
                    return Err(
                        "successful plan did not advance to the fixed upper bound".to_string()
                    );
                }
                if !expected_covered {
                    return Err(
                        "successful plan did not cover every expected changed path".to_string()
                    );
                }
                if !analysis.covered {
                    return Err(
                        "successful plan did not cover directories changed in full analysis"
                            .to_string(),
                    );
                }
                if mode == ProbeMode::NoChanges && plan.has_changes() {
                    return Err(
                        "no-change probe unexpectedly returned dirty directories".to_string()
                    );
                }
            }
        }
        Ok(())
    }

    fn analysis_snapshot(
        root: &Path,
        platform: &impl Platform,
    ) -> Result<AnalysisSnapshot, String> {
        let started = Instant::now();
        let mut directories = HashMap::new();
        let summary = platform
            .fast_analysis_records(
                mangodisk_platform::FastAnalysisQuery {
                    retained_file_limit: 0,
                    name_exclusions: &mangodisk_platform::NameExclusions::default(),
                    excluded_roots: &[],
                    root,
                    purpose: mangodisk_platform::ScanPurpose::Analysis,
                    large_file_minimum_bytes: u64::MAX,
                    should_prune_directory: |_| false,
                },
                &|| false,
                &mut |_, _, _| {},
                &mut |record| {
                    if let FastAnalysisRecord::Directory {
                        path,
                        allocated_bytes: bytes,
                        file_count,
                        skipped_count,
                        ..
                    } = record
                    {
                        directories
                            .insert(normalize(&path), (path, bytes, file_count, skipped_count));
                    }
                    Ok(())
                },
            )
            .map_err(format_analysis_error)?
            .ok_or_else(|| "the root does not support native Windows analysis A/B".to_string())?;
        if directories.len() as u64 != summary.directory_count {
            return Err("analysis A/B directory output does not match its summary".to_string());
        }
        Ok(AnalysisSnapshot {
            directories,
            elapsed_ms: started.elapsed().as_millis(),
        })
    }

    fn compare_analysis(
        root: &Path,
        before: Option<&AnalysisSnapshot>,
        dirty_directories: &[PathBuf],
        platform: &impl Platform,
    ) -> Result<AnalysisComparison, String> {
        let Some(before) = before else {
            return Ok(AnalysisComparison {
                changed_directories: 0,
                covered: true,
                before_elapsed_ms: 0,
                after_elapsed_ms: 0,
            });
        };
        let after = analysis_snapshot(root, platform)?;
        let identities = before
            .directories
            .keys()
            .chain(after.directories.keys())
            .collect::<HashSet<_>>();
        let changed = identities
            .into_iter()
            .filter_map(|identity| {
                let before_value = before.directories.get(identity);
                let after_value = after.directories.get(identity);
                (before_value != after_value)
                    .then(|| after_value.or(before_value).map(|value| &value.0))
                    .flatten()
            })
            .collect::<Vec<_>>();
        // A local refresh rescans dirty subtrees and recalculates ancestor aggregates. Therefore,
        // an A/B changed directory can be below a dirty root or be its ancestor; only paths outside
        // both relationships indicate missing coverage.
        let covered = changed.iter().all(|changed_path| {
            dirty_directories.iter().any(|dirty| {
                path_is_same_or_child(changed_path, dirty)
                    || path_is_same_or_child(dirty, changed_path)
            })
        });
        Ok(AnalysisComparison {
            changed_directories: changed.len(),
            covered,
            before_elapsed_ms: before.elapsed_ms,
            after_elapsed_ms: after.elapsed_ms,
        })
    }

    fn mode_supports_analysis_comparison(mode: ProbeMode) -> bool {
        matches!(
            mode,
            ProbeMode::NoChanges
                | ProbeMode::Create
                | ProbeMode::Modify
                | ProbeMode::Delete
                | ProbeMode::Rename
                | ProbeMode::CrossMove
                | ProbeMode::HardLink
                | ProbeMode::CreateMany
        )
    }

    fn format_analysis_error(error: FastAnalysisScanError) -> String {
        match error {
            FastAnalysisScanError::Cancelled => {
                "analysis A/B was cancelled unexpectedly".to_string()
            }
            FastAnalysisScanError::Busy => {
                "analysis A/B could not start because native workers are busy".to_string()
            }
            FastAnalysisScanError::Platform(error) => {
                format!(
                    "analysis A/B platform error digest={}",
                    blake3::hash(error.as_bytes()).to_hex()
                )
            }
            FastAnalysisScanError::Consumer(error) => {
                format!(
                    "analysis A/B consumer error digest={}",
                    blake3::hash(error.as_bytes()).to_hex()
                )
            }
        }
    }

    fn parse_mode(value: Option<&str>) -> Result<ProbeMode, String> {
        match value.unwrap_or("no-changes") {
            "no-changes" => Ok(ProbeMode::NoChanges),
            "create" => Ok(ProbeMode::Create),
            "modify" => Ok(ProbeMode::Modify),
            "delete" => Ok(ProbeMode::Delete),
            "rename" => Ok(ProbeMode::Rename),
            "cross-move" => Ok(ProbeMode::CrossMove),
            "delete-tree" => Ok(ProbeMode::DeleteTree),
            "hard-link" => Ok(ProbeMode::HardLink),
            "create-many" => Ok(ProbeMode::CreateMany),
            "cancel" => Ok(ProbeMode::Cancel),
            "cancel-mid-read" => Ok(ProbeMode::CancelMidRead),
            "fixed-upper-bound" => Ok(ProbeMode::FixedUpperBound),
            "invalid-token" => Ok(ProbeMode::InvalidToken),
            "subdirectory" => Ok(ProbeMode::Subdirectory),
            _ => Err("unsupported probe mode".to_string()),
        }
    }

    fn mode_name(mode: ProbeMode) -> &'static str {
        match mode {
            ProbeMode::NoChanges => "no_changes",
            ProbeMode::Create => "create",
            ProbeMode::Modify => "modify",
            ProbeMode::Delete => "delete",
            ProbeMode::Rename => "rename",
            ProbeMode::CrossMove => "cross_move",
            ProbeMode::DeleteTree => "delete_tree",
            ProbeMode::HardLink => "hard_link",
            ProbeMode::CreateMany => "create_many",
            ProbeMode::Cancel => "cancel",
            ProbeMode::CancelMidRead => "cancel_mid_read",
            ProbeMode::FixedUpperBound => "fixed_upper_bound",
            ProbeMode::InvalidToken => "invalid_token",
            ProbeMode::Subdirectory => "subdirectory",
        }
    }

    fn format_impact_error(error: FilesystemChangeImpactError) -> String {
        match error {
            FilesystemChangeImpactError::Cancelled => {
                "planning was cancelled unexpectedly".to_string()
            }
            FilesystemChangeImpactError::Platform(error) => {
                format!(
                    "planning platform error digest={}",
                    blake3::hash(error.as_bytes()).to_hex()
                )
            }
        }
    }

    fn normalize(path: &Path) -> String {
        let value = path.to_string_lossy().replace('/', "\\");
        value
            .strip_prefix(r"\\?\UNC\")
            .map(|path| format!(r"\\{path}"))
            .or_else(|| value.strip_prefix(r"\\?\").map(str::to_string))
            .unwrap_or(value)
            .trim_end_matches('\\')
            .to_ascii_lowercase()
    }

    fn path_is_same_or_child(path: &Path, root: &Path) -> bool {
        let path = normalize(path);
        let root = normalize(root);
        path == root
            || path
                .strip_prefix(&root)
                .is_some_and(|suffix| suffix.starts_with('\\'))
    }

    fn path_digest_hex(path: &Path) -> String {
        let mut hasher = blake3::Hasher::new();
        for unit in path.as_os_str().encode_wide() {
            let normalized = if (u16::from(b'A')..=u16::from(b'Z')).contains(&unit) {
                unit + u16::from(b'a' - b'A')
            } else {
                unit
            };
            hasher.update(&normalized.to_le_bytes());
        }
        hasher.finalize().to_hex().to_string()
    }

    fn digest_list(digests: &[String]) -> String {
        let mut hasher = blake3::Hasher::new();
        for digest in digests {
            hasher.update(digest.as_bytes());
        }
        hasher.finalize().to_hex().to_string()
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
                "failed to read probe process memory: {}",
                std::io::Error::last_os_error()
            ));
        }
        Ok((counters.PeakWorkingSetSize, counters.WorkingSetSize))
    }
}
