#[cfg(target_os = "macos")]
use std::time::Instant;

#[cfg(target_os = "macos")]
use crate::cleanup::{CleanupSourceBlockReason, CleanupSourceDetail};
#[cfg(target_os = "macos")]
use crate::filesystem::metadata::diagnostic_path;
use crate::{
    applications::catalog::ApplicationInventory,
    cleanup::{
        source_selection::SourceScope, CleanupActionKind, CleanupActionReason, CleanupActionResult,
        CleanupActionStatus, CleanupGroup, RiskLevel, ScanItemStatus, ScanRuleResult,
    },
    shared::operation::OperationGuard,
};

pub(crate) const CLEANER_ID: &str = "special.macos-universal-binaries";
pub(crate) const CLEANER_REVISION: &str = "5-complete-source-inventory";

#[cfg(target_os = "macos")]
mod platform {
    use std::{
        collections::{BTreeMap, BTreeSet, HashSet, VecDeque},
        env,
        ffi::{CString, OsStr},
        fs::{self, File, OpenOptions},
        io::{self, Read, Seek, SeekFrom, Write},
        os::unix::ffi::OsStrExt,
        path::{Component, Path, PathBuf},
        process::Command,
        sync::{
            atomic::{AtomicUsize, Ordering},
            Mutex, OnceLock,
        },
        thread,
        time::UNIX_EPOCH,
    };

    use mangodisk_platform::{current_platform, InstalledApplication, Platform};
    use plist::Value;

    use super::*;

    const CPU_TYPE_X86_64: u32 = 0x0100_0007;
    const CPU_TYPE_ARM64: u32 = 0x0100_000c;
    const FAT_MAGIC: u32 = 0xcafe_babe;
    const FAT_MAGIC_64: u32 = 0xcafe_babf;
    const FAT_HEADER_BYTES: usize = 8;
    const FAT_ARCH_BYTES: usize = 20;
    const FAT_ARCH_64_BYTES: usize = 32;
    const MAX_ARCHITECTURES: usize = 16;
    const MAX_DISCOVERY_WORKERS: usize = 4;
    const MAX_SIGNED_BUNDLES_PER_APPLICATION: usize = 2_048;
    const MAX_SIGNED_COMPONENTS_PER_APPLICATION: usize = 8_192;
    const MAX_CODE_RESOURCES_BYTES: u64 = 64 * 1024 * 1024;
    static NATIVE_CPU_TYPE: OnceLock<u32> = OnceLock::new();

    #[derive(Debug, Clone)]
    struct BinarySlice {
        offset: u64,
        bytes: u64,
    }

    #[derive(Debug, Clone)]
    struct UniversalBinaryCandidate {
        application_name: String,
        application_bundle_path: PathBuf,
        executable_path: PathBuf,
        native_slice: BinarySlice,
        original_bytes: u64,
        reclaimable_bytes: u64,
        modified_at_ms: Option<u64>,
        block_reason: Option<CleanupSourceBlockReason>,
    }

    pub(crate) fn preview(
        inventory: &ApplicationInventory,
        is_cancelled: &(dyn Fn() -> bool + Sync),
        report_path: &(dyn Fn(&Path) + Sync),
    ) -> ScanRuleResult {
        let started = Instant::now();
        if !inventory.application_inventory_complete() {
            return limited_rule(started.elapsed().as_millis() as u64);
        }

        let candidates = discover_candidates(inventory, is_cancelled, report_path);
        let bytes = candidates
            .iter()
            .map(|candidate| candidate.reclaimable_bytes)
            .sum();
        let file_count = candidates.len() as u64;
        let mut applications =
            BTreeMap::<PathBuf, (u64, u64, Option<u64>, Option<CleanupSourceBlockReason>)>::new();
        let mut running_processes = BTreeSet::new();
        for candidate in &candidates {
            let application = applications
                .entry(candidate.application_bundle_path.clone())
                .or_default();
            application.0 = application.0.saturating_add(candidate.reclaimable_bytes);
            application.1 += 1;
            application.2 = application.2.max(candidate.modified_at_ms);
            application.3 = candidate.block_reason;
            if candidate.block_reason == Some(CleanupSourceBlockReason::RequiresClose) {
                running_processes.insert(candidate.application_name.clone());
            }
        }
        let source_count = applications.len() as u64;
        let mut sources = applications
            .into_iter()
            .map(
                |(application_bundle_path, (bytes, file_count, modified_at_ms, block_reason))| {
                    CleanupSourceDetail {
                        path: application_bundle_path.to_string_lossy().into_owned(),
                        bytes,
                        file_count,
                        modified_at_ms,
                        block_reason,
                    }
                },
            )
            .collect::<Vec<_>>();
        sources.sort_by(|left, right| {
            right
                .bytes
                .cmp(&left.bytes)
                .then_with(|| left.path.cmp(&right.path))
        });
        ScanRuleResult {
            rule_id: CLEANER_ID.to_string(),
            category: crate::cleanup::CleanupCategory::ApplicationOptimization,
            group: CleanupGroup::ApplicationOptimization,
            risk: RiskLevel::Recoverable,
            default_selected: false,
            recommended_selected: false,
            bytes,
            file_count,
            available: true,
            selectable: bytes > 0,
            status: if bytes > 0 {
                ScanItemStatus::Found
            } else {
                ScanItemStatus::Clean
            },
            running_processes: running_processes.into_iter().collect(),
            requires_app_close: candidates.iter().any(|candidate| {
                candidate.block_reason == Some(CleanupSourceBlockReason::RequiresClose)
            }),
            sources,
            source_count,
            sources_truncated: false,
            scan_elapsed_ms: started.elapsed().as_millis() as u64,
        }
    }

    pub(crate) fn execute(
        inventory: &ApplicationInventory,
        source_scope: Option<&SourceScope>,
        dry_run: bool,
        operation: &OperationGuard,
    ) -> CleanupActionResult {
        let mut candidates = discover_candidates(
            inventory,
            &|| operation.cancelled().load(Ordering::Relaxed),
            &|_| {},
        );
        if let Some(scope) = source_scope {
            if scope
                .validate_known_paths(
                    candidates
                        .iter()
                        .map(|candidate| candidate.application_bundle_path.as_path()),
                )
                .is_err()
            {
                return failed_action(CleanupActionReason::PreflightFailed);
            }
            candidates.retain(|candidate| scope.selects(&candidate.application_bundle_path));
        }
        let bytes_expected = candidates
            .iter()
            .map(|candidate| candidate.reclaimable_bytes)
            .sum();
        if dry_run {
            return completed_action(CleanupActionStatus::Previewed, bytes_expected, 0, 0);
        }

        let mut released_bytes = 0_u64;
        let mut optimized_count = 0_u64;
        let mut failed_item_count = 0_u64;
        let mut optimization_failure_count = 0_u64;
        let mut running_processes = BTreeSet::new();
        let mut applications = BTreeMap::<PathBuf, Vec<&UniversalBinaryCandidate>>::new();
        for candidate in &candidates {
            applications
                .entry(candidate.application_bundle_path.clone())
                .or_default()
                .push(candidate);
        }
        // Discovery may take several seconds for large applications. Refresh
        // process state immediately before mutation so an application launched
        // during scanning is never modified while it is running.
        let current_running_paths = running_executable_paths();
        for (index, application_candidates) in applications.values().enumerate() {
            if operation.cancelled().load(Ordering::Relaxed) {
                let remaining_count = applications
                    .values()
                    .skip(index)
                    .map(|candidates| candidates.len() as u64)
                    .sum::<u64>();
                failed_item_count += remaining_count;
                optimization_failure_count += remaining_count;
                break;
            }
            let candidate = application_candidates[0];
            if candidate.block_reason == Some(CleanupSourceBlockReason::RequiresClose)
                || application_is_running(
                    &candidate.application_bundle_path,
                    &current_running_paths,
                )
            {
                failed_item_count += application_candidates.len() as u64;
                running_processes.insert(candidate.application_name.clone());
                continue;
            }
            match optimize_application(application_candidates, operation.id(), index) {
                Ok((released, optimized)) => {
                    released_bytes = released_bytes.saturating_add(released);
                    optimized_count += optimized;
                }
                Err(error) => {
                    failed_item_count += application_candidates.len() as u64;
                    optimization_failure_count += application_candidates.len() as u64;
                    log::warn!(
                        "macos_universal_binary_optimize_failed bundle={} components={} error={}",
                        diagnostic_path(&candidate.application_bundle_path),
                        application_candidates.len(),
                        mangodisk_platform::diagnostics::text(&error)
                    );
                }
            }
        }

        let status = if failed_item_count == 0 {
            CleanupActionStatus::Completed
        } else if optimized_count > 0 {
            CleanupActionStatus::Partial
        } else {
            CleanupActionStatus::Failed
        };
        CleanupActionResult {
            rule_id: CLEANER_ID.to_string(),
            action_kind: CleanupActionKind::Optimize,
            status,
            reason_code: (failed_item_count > 0).then_some(if optimization_failure_count > 0 {
                if optimized_count > 0 {
                    CleanupActionReason::ItemsSkipped
                } else {
                    CleanupActionReason::PreflightFailed
                }
            } else {
                CleanupActionReason::RunningProcesses
            }),
            bytes_expected,
            released_bytes,
            affected_item_count: optimized_count,
            failed_item_count,
            running_processes: running_processes.into_iter().collect(),
        }
    }

    fn discover_candidates(
        inventory: &ApplicationInventory,
        is_cancelled: &(dyn Fn() -> bool + Sync),
        report_path: &(dyn Fn(&Path) + Sync),
    ) -> Vec<UniversalBinaryCandidate> {
        let running_paths = running_executable_paths();
        let current_executable = env::current_exe()
            .ok()
            .and_then(|path| fs::canonicalize(path).ok());
        let native_cpu = native_cpu_type();
        let applications = inventory.installed_applications();
        let next_application = AtomicUsize::new(0);
        let candidates = Mutex::new(Vec::new());
        let worker_count = thread::available_parallelism()
            .map(usize::from)
            .unwrap_or(1)
            .min(MAX_DISCOVERY_WORKERS)
            .min(applications.len())
            .max(1);
        thread::scope(|scope| {
            for _ in 0..worker_count {
                scope.spawn(|| loop {
                    if is_cancelled() {
                        break;
                    }
                    let index = next_application.fetch_add(1, Ordering::Relaxed);
                    let Some(application) = applications.get(index) else {
                        break;
                    };
                    let discovered = discover_application_candidates(
                        application,
                        &running_paths,
                        current_executable.as_deref(),
                        native_cpu,
                        is_cancelled,
                        report_path,
                    );
                    if !discovered.is_empty() {
                        candidates
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner())
                            .extend(discovered);
                    }
                });
            }
        });
        let mut candidates = candidates
            .into_inner()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        candidates.sort_by(|left, right| {
            left.application_bundle_path
                .cmp(&right.application_bundle_path)
                .then_with(|| left.executable_path.cmp(&right.executable_path))
        });
        candidates
    }

    fn discover_application_candidates(
        application: &InstalledApplication,
        running_paths: &HashSet<PathBuf>,
        current_executable: Option<&Path>,
        native_cpu: u32,
        is_cancelled: &(dyn Fn() -> bool + Sync),
        report_path: &(dyn Fn(&Path) + Sync),
    ) -> Vec<UniversalBinaryCandidate> {
        let Some(bundle_path) = eligible_bundle_path(application) else {
            return Vec::new();
        };
        let Ok(canonical_bundle) = fs::canonicalize(bundle_path) else {
            return Vec::new();
        };
        if current_executable.is_some_and(|current| current.starts_with(&canonical_bundle)) {
            return Vec::new();
        }
        let application_running = application_is_running(&canonical_bundle, running_paths);
        report_path(&canonical_bundle);

        let mut candidates = discover_signed_executables(
            &canonical_bundle,
            &application.executable_paths,
            is_cancelled,
        )
        .into_iter()
        .filter_map(|executable_path| {
            let metadata = fs::symlink_metadata(&executable_path).ok()?;
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return None;
            }
            let native_slice = read_native_slice(&executable_path, native_cpu).ok()??;
            if native_slice.bytes >= metadata.len() {
                return None;
            }
            Some(UniversalBinaryCandidate {
                application_name: application.name.clone(),
                application_bundle_path: canonical_bundle.clone(),
                executable_path,
                reclaimable_bytes: metadata.len() - native_slice.bytes,
                native_slice,
                original_bytes: metadata.len(),
                modified_at_ms: metadata
                    .modified()
                    .ok()
                    .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
                    .map(|value| value.as_millis() as u64),
                block_reason: None,
            })
        })
        .collect::<Vec<_>>();
        if candidates.is_empty()
            || !application_transaction_is_writable(&canonical_bundle, &candidates)
        {
            return Vec::new();
        }
        let block_reason = application_running.then_some(CleanupSourceBlockReason::RequiresClose);
        for candidate in &mut candidates {
            candidate.block_reason = block_reason;
        }
        candidates
    }

    fn eligible_bundle_path(application: &InstalledApplication) -> Option<&Path> {
        let bundle = application.bundle_path.as_deref()?;
        if bundle.starts_with("/System")
            || application
                .identifiers
                .iter()
                .any(|identifier| identifier.starts_with("com.apple."))
            || !bundle
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("app"))
        {
            return None;
        }
        Some(bundle)
    }

    fn discover_signed_executables(
        application_bundle_path: &Path,
        application_executables: &[PathBuf],
        is_cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Vec<PathBuf> {
        let Ok(application_bundle_path) = fs::canonicalize(application_bundle_path) else {
            return Vec::new();
        };
        let mut executables = HashSet::new();
        let mut visited_bundles = HashSet::new();
        let mut pending_bundles = VecDeque::from([application_bundle_path.to_path_buf()]);

        for executable in application_executables {
            if let Some(executable) =
                canonical_path_inside_bundle(executable, &application_bundle_path)
            {
                executables.insert(executable);
            }
        }

        while let Some(bundle_path) = pending_bundles.pop_front() {
            if is_cancelled() {
                break;
            }
            if visited_bundles.len() >= MAX_SIGNED_BUNDLES_PER_APPLICATION
                || executables.len() >= MAX_SIGNED_COMPONENTS_PER_APPLICATION
            {
                log::warn!(
                    "macos_universal_binary_discovery_limited bundle={}",
                    diagnostic_path(&application_bundle_path)
                );
                break;
            }
            let Some(bundle_path) =
                canonical_path_inside_bundle(&bundle_path, &application_bundle_path)
            else {
                continue;
            };
            if !visited_bundles.insert(bundle_path.clone()) {
                continue;
            }
            if let Some(executable) = bundle_main_executable(&bundle_path)
                .and_then(|path| canonical_path_inside_bundle(&path, &application_bundle_path))
            {
                executables.insert(executable);
            }
            for nested_code in signed_nested_code(&bundle_path, &application_bundle_path) {
                if nested_code.is_dir() {
                    pending_bundles.push_back(nested_code);
                } else if nested_code.is_file() {
                    executables.insert(nested_code);
                }
            }
        }

        let mut executables = executables.into_iter().collect::<Vec<_>>();
        executables.sort();
        executables
    }

    fn bundle_main_executable(bundle_path: &Path) -> Option<PathBuf> {
        let candidates = [
            (
                bundle_path.join("Contents/Info.plist"),
                bundle_path.join("Contents/MacOS"),
            ),
            (
                bundle_path.join("Versions/Current/Resources/Info.plist"),
                bundle_path.join("Versions/Current"),
            ),
            (
                bundle_path.join("Resources/Info.plist"),
                bundle_path.to_path_buf(),
            ),
            (bundle_path.join("Info.plist"), bundle_path.to_path_buf()),
        ];
        candidates
            .into_iter()
            .find_map(|(info_path, executable_root)| {
                let dictionary = Value::from_file(info_path).ok()?.into_dictionary()?;
                let executable = dictionary.get("CFBundleExecutable")?.as_string()?;
                Some(executable_root.join(executable))
            })
    }

    fn signed_nested_code(bundle_path: &Path, application_bundle_path: &Path) -> Vec<PathBuf> {
        let code_resources = [
            bundle_path.join("Contents/_CodeSignature/CodeResources"),
            bundle_path.join("Versions/Current/_CodeSignature/CodeResources"),
            bundle_path.join("_CodeSignature/CodeResources"),
        ]
        .into_iter()
        .find(|path| path.is_file());
        let Some(code_resources) = code_resources else {
            return Vec::new();
        };
        if fs::metadata(&code_resources)
            .is_ok_and(|metadata| metadata.len() > MAX_CODE_RESOURCES_BYTES)
        {
            log::warn!(
                "macos_universal_binary_manifest_skipped reason=sizeLimit path={}",
                diagnostic_path(&code_resources)
            );
            return Vec::new();
        }
        let Some(content_root) = code_resources.parent().and_then(Path::parent) else {
            return Vec::new();
        };
        let Some(files) = Value::from_file(&code_resources)
            .ok()
            .and_then(Value::into_dictionary)
            .and_then(|mut dictionary| dictionary.remove("files2"))
            .and_then(Value::into_dictionary)
        else {
            return Vec::new();
        };

        files
            .into_iter()
            .filter_map(|(relative_path, seal)| {
                let signed_as_code = seal
                    .as_dictionary()
                    .is_some_and(|attributes| attributes.contains_key("requirement"));
                if !signed_as_code || !is_safe_relative_path(Path::new(&relative_path)) {
                    return None;
                }
                canonical_path_inside_bundle(
                    &content_root.join(relative_path),
                    application_bundle_path,
                )
            })
            .collect()
    }

    fn canonical_path_inside_bundle(
        path: &Path,
        application_bundle_path: &Path,
    ) -> Option<PathBuf> {
        let canonical = fs::canonicalize(path).ok()?;
        canonical
            .starts_with(application_bundle_path)
            .then_some(canonical)
    }

    fn is_safe_relative_path(path: &Path) -> bool {
        !path.as_os_str().is_empty()
            && path
                .components()
                .all(|component| matches!(component, Component::Normal(_)))
    }

    fn application_transaction_is_writable(
        application_bundle_path: &Path,
        candidates: &[UniversalBinaryCandidate],
    ) -> bool {
        application_bundle_path
            .parent()
            .is_some_and(directory_is_writable)
            && candidates.iter().all(|candidate| {
                candidate
                    .executable_path
                    .parent()
                    .is_some_and(directory_is_writable)
            })
    }

    fn application_is_running(
        application_bundle_path: &Path,
        running_paths: &HashSet<PathBuf>,
    ) -> bool {
        running_paths
            .iter()
            .any(|running| running.starts_with(application_bundle_path))
    }

    fn directory_is_writable(path: &Path) -> bool {
        let Ok(path) = CString::new(path.as_os_str().as_bytes()) else {
            return false;
        };
        // Atomic replacement depends on write and search permission for every
        // parent directory. Checking access up front prevents the preview from
        // promising space that the current unprivileged process cannot release.
        // SAFETY: CString provides a valid NUL-terminated path for this call.
        unsafe { libc::access(path.as_ptr(), libc::W_OK | libc::X_OK) == 0 }
    }

    fn native_cpu_type() -> u32 {
        *NATIVE_CPU_TYPE.get_or_init(|| {
            // Compile-time architecture is x86_64 when MangoDisk itself is
            // launched through Rosetta. The hardware capability is the safer
            // source because an Apple silicon Mac should retain arm64 code.
            let apple_silicon = Command::new("/usr/sbin/sysctl")
                .args(["-n", "hw.optional.arm64"])
                .output()
                .ok()
                .filter(|output| output.status.success())
                .is_some_and(|output| output.stdout.starts_with(b"1"));
            if apple_silicon || cfg!(target_arch = "aarch64") {
                CPU_TYPE_ARM64
            } else {
                CPU_TYPE_X86_64
            }
        })
    }

    fn read_native_slice(path: &Path, native_cpu: u32) -> io::Result<Option<BinarySlice>> {
        let mut file = File::open(path)?;
        let mut header = [0_u8; FAT_HEADER_BYTES];
        if file.read_exact(&mut header).is_err() {
            return Ok(None);
        }
        let magic = u32::from_be_bytes(header[0..4].try_into().expect("fixed header width"));
        let entry_bytes = match magic {
            FAT_MAGIC => FAT_ARCH_BYTES,
            FAT_MAGIC_64 => FAT_ARCH_64_BYTES,
            _ => return Ok(None),
        };
        let architecture_count =
            u32::from_be_bytes(header[4..8].try_into().expect("fixed header width")) as usize;
        if !(2..=MAX_ARCHITECTURES).contains(&architecture_count) {
            return Ok(None);
        }
        let mut entries = vec![0_u8; architecture_count * entry_bytes];
        file.read_exact(&mut entries)?;
        for entry in entries.chunks_exact(entry_bytes) {
            let cpu_type =
                u32::from_be_bytes(entry[0..4].try_into().expect("fixed architecture width"));
            if cpu_type != native_cpu {
                continue;
            }
            let (offset, bytes) = if magic == FAT_MAGIC_64 {
                (
                    u64::from_be_bytes(entry[8..16].try_into().expect("fixed offset width")),
                    u64::from_be_bytes(entry[16..24].try_into().expect("fixed size width")),
                )
            } else {
                (
                    u32::from_be_bytes(entry[8..12].try_into().expect("fixed offset width")) as u64,
                    u32::from_be_bytes(entry[12..16].try_into().expect("fixed size width")) as u64,
                )
            };
            let file_bytes = file.metadata()?.len();
            if bytes == 0 || offset > file_bytes || bytes > file_bytes - offset {
                return Ok(None);
            }
            return Ok(Some(BinarySlice { offset, bytes }));
        }
        Ok(None)
    }

    fn optimize_application(
        candidates: &[&UniversalBinaryCandidate],
        operation_id: u64,
        index: usize,
    ) -> Result<(u64, u64), String> {
        let Some(first_candidate) = candidates.first() else {
            return Ok((0, 0));
        };
        let application_bundle_path = &first_candidate.application_bundle_path;
        if candidates
            .iter()
            .any(|candidate| candidate.application_bundle_path.as_path() != application_bundle_path)
        {
            return Err("application transaction contains multiple bundles".to_string());
        }
        verify_bundle(application_bundle_path, "preflight")?;
        let gatekeeper_accepted_before = assess_gatekeeper(application_bundle_path);

        let bundle_name = application_bundle_path
            .file_name()
            .unwrap_or_else(|| OsStr::new("application"))
            .to_string_lossy();
        let bundle_parent = application_bundle_path
            .parent()
            .ok_or_else(|| "the application bundle has no parent directory".to_string())?;
        let transaction_path =
            bundle_parent.join(format!(".{bundle_name}.mangodisk-{operation_id}-{index}"));
        if transaction_path.exists() {
            return Err("a previous optimization artifact is still present".to_string());
        }
        fs::create_dir(&transaction_path)
            .map_err(|error| format!("cannot create application transaction: {error}"))?;
        let mut staged = Vec::with_capacity(candidates.len());
        for (component_index, candidate) in candidates.iter().enumerate() {
            match stage_candidate(candidate, &transaction_path, component_index) {
                Ok(candidate) => staged.push(candidate),
                Err(error) => {
                    cleanup_transaction(&staged, &transaction_path);
                    return Err(error);
                }
            }
        }
        for staged_index in 0..staged.len() {
            let candidate = &mut staged[staged_index];
            if let Err(error) = fs::rename(&candidate.temporary_path, &candidate.executable_path) {
                rollback_transaction(&staged[..staged_index], &transaction_path)?;
                cleanup_transaction(&staged[staged_index..], &transaction_path);
                return Err(format!("cannot install native executable: {error}"));
            }
            candidate.installed = true;
        }

        let postflight = verify_bundle(application_bundle_path, "postflight").and_then(|()| {
            if gatekeeper_accepted_before && !assess_gatekeeper(application_bundle_path) {
                Err("application no longer passes Gatekeeper assessment".to_string())
            } else {
                Ok(())
            }
        });
        if let Err(error) = postflight {
            rollback_transaction(&staged, &transaction_path)?;
            return Err(error);
        }

        let released = candidates
            .iter()
            .map(|candidate| candidate.reclaimable_bytes)
            .sum();
        fs::remove_dir_all(&transaction_path)
            .map_err(|error| format!("cannot release staged original executables: {error}"))?;
        log::info!(
            "macos_universal_binary_optimized bundle={} components={} released_bytes={}",
            diagnostic_path(application_bundle_path),
            candidates.len(),
            released,
        );
        Ok((released, candidates.len() as u64))
    }

    struct StagedCandidate {
        executable_path: PathBuf,
        temporary_path: PathBuf,
        backup_path: PathBuf,
        installed: bool,
    }

    fn stage_candidate(
        candidate: &UniversalBinaryCandidate,
        transaction_path: &Path,
        component_index: usize,
    ) -> Result<StagedCandidate, String> {
        let metadata = fs::metadata(&candidate.executable_path)
            .map_err(|error| format!("metadata unavailable: {error}"))?;
        if metadata.len() != candidate.original_bytes {
            return Err("an executable changed after scanning".to_string());
        }
        let Some(current_slice) = read_native_slice(&candidate.executable_path, native_cpu_type())
            .map_err(|error| format!("cannot inspect executable: {error}"))?
        else {
            return Err("an executable is no longer a supported universal binary".to_string());
        };
        if current_slice.offset != candidate.native_slice.offset
            || current_slice.bytes != candidate.native_slice.bytes
        {
            return Err("an executable architecture layout changed after scanning".to_string());
        }

        let temporary_path = transaction_path.join(format!("native-{component_index}"));
        let backup_path = transaction_path.join(format!("original-{component_index}"));
        write_native_slice(
            &candidate.executable_path,
            &temporary_path,
            &current_slice,
            metadata.permissions(),
        )
        .map_err(|error| format!("cannot create native executable: {error}"))?;
        // Hard links retain the original inodes without duplicating their data.
        // Every replacement is prepared before the first executable changes.
        if let Err(error) = fs::hard_link(&candidate.executable_path, &backup_path) {
            let _ = fs::remove_file(&temporary_path);
            return Err(format!("cannot stage original executable: {error}"));
        }
        Ok(StagedCandidate {
            executable_path: candidate.executable_path.clone(),
            temporary_path,
            backup_path,
            installed: false,
        })
    }

    fn rollback_transaction(
        staged: &[StagedCandidate],
        transaction_path: &Path,
    ) -> Result<(), String> {
        let mut rollback_error = None;
        for candidate in staged.iter().rev().filter(|candidate| candidate.installed) {
            if let Err(error) =
                rollback_candidate(&candidate.executable_path, &candidate.backup_path)
            {
                rollback_error.get_or_insert(error);
            }
        }
        for candidate in staged {
            let _ = fs::remove_file(&candidate.temporary_path);
            if !candidate.installed {
                let _ = fs::remove_file(&candidate.backup_path);
            }
        }
        if let Some(error) = rollback_error {
            // A failed restore must retain its original hard link. Removing the
            // transaction directory here would destroy the last recovery copy.
            log::error!(
                "macos_universal_binary_rollback_incomplete transaction={} error={}",
                diagnostic_path(transaction_path),
                mangodisk_platform::diagnostics::text(&error)
            );
            Err(error)
        } else {
            let _ = fs::remove_dir(transaction_path);
            Ok(())
        }
    }

    fn cleanup_transaction(staged: &[StagedCandidate], transaction_path: &Path) {
        for candidate in staged {
            let _ = fs::remove_file(&candidate.temporary_path);
            let _ = fs::remove_file(&candidate.backup_path);
        }
        let _ = fs::remove_dir_all(transaction_path);
    }

    fn write_native_slice(
        source_path: &Path,
        destination_path: &Path,
        slice: &BinarySlice,
        permissions: fs::Permissions,
    ) -> io::Result<()> {
        let mut source = File::open(source_path)?;
        source.seek(SeekFrom::Start(slice.offset))?;
        let mut destination = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(destination_path)?;
        let copied = io::copy(&mut source.take(slice.bytes), &mut destination)?;
        if copied != slice.bytes {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "universal binary changed while extracting native slice",
            ));
        }
        destination.flush()?;
        destination.sync_all()?;
        fs::set_permissions(destination_path, permissions)?;
        Ok(())
    }

    fn rollback_candidate(executable_path: &Path, backup_path: &Path) -> Result<(), String> {
        fs::rename(backup_path, executable_path)
            .map_err(|error| format!("cannot restore original executable: {error}"))
    }

    fn verify_bundle(bundle_path: &Path, phase: &str) -> Result<(), String> {
        let output = Command::new("/usr/bin/codesign")
            .args(["--verify", "--deep", "--strict"])
            .arg(bundle_path)
            .output()
            .map_err(|error| format!("cannot start signature verification: {error}"))?;
        if output.status.success() {
            return Ok(());
        }
        log::warn!(
            "macos_universal_binary_signature_failed phase={} bundle={} stderr={}",
            phase,
            diagnostic_path(bundle_path),
            mangodisk_platform::diagnostics::text(&String::from_utf8_lossy(&output.stderr))
        );
        Err(format!(
            "application signature verification failed during {phase}"
        ))
    }

    fn assess_gatekeeper(bundle_path: &Path) -> bool {
        Command::new("/usr/sbin/spctl")
            .args(["--assess", "--type", "execute"])
            .arg(bundle_path)
            .output()
            .is_ok_and(|output| output.status.success())
    }

    fn running_executable_paths() -> HashSet<PathBuf> {
        current_platform()
            .running_process_names()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|path| fs::canonicalize(path).ok())
            .collect()
    }

    fn completed_action(
        status: CleanupActionStatus,
        bytes_expected: u64,
        released_bytes: u64,
        optimized_count: u64,
    ) -> CleanupActionResult {
        CleanupActionResult {
            rule_id: CLEANER_ID.to_string(),
            action_kind: CleanupActionKind::Optimize,
            status,
            reason_code: None,
            bytes_expected,
            released_bytes,
            affected_item_count: optimized_count,
            failed_item_count: 0,
            running_processes: Vec::new(),
        }
    }

    fn failed_action(reason: CleanupActionReason) -> CleanupActionResult {
        CleanupActionResult {
            rule_id: CLEANER_ID.to_string(),
            action_kind: CleanupActionKind::Optimize,
            status: CleanupActionStatus::Failed,
            reason_code: Some(reason),
            bytes_expected: 0,
            released_bytes: 0,
            affected_item_count: 0,
            failed_item_count: 1,
            running_processes: Vec::new(),
        }
    }

    #[cfg(test)]
    mod tests {
        use std::{
            fs,
            os::unix::fs::PermissionsExt,
            time::{SystemTime, UNIX_EPOCH},
        };

        use crate::applications::catalog::ScanContext;

        use super::*;

        #[test]
        fn fat_macho_parser_finds_the_native_slice() {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock should be available")
                .as_nanos();
            let root = env::temp_dir().join(format!(
                "mangodisk-universal-parser-{}-{unique}",
                std::process::id()
            ));
            fs::create_dir_all(&root).expect("temporary directory should be created");
            let source = root.join("universal");
            let destination = root.join("native");
            let mut bytes = vec![0_u8; 8_196];
            bytes[0..4].copy_from_slice(&FAT_MAGIC.to_be_bytes());
            bytes[4..8].copy_from_slice(&2_u32.to_be_bytes());
            write_fat_arch(&mut bytes[8..28], CPU_TYPE_X86_64, 4_096, 4);
            write_fat_arch(&mut bytes[28..48], CPU_TYPE_ARM64, 8_192, 4);
            bytes[4_096..4_100].copy_from_slice(b"x86!");
            bytes[8_192..8_196].copy_from_slice(b"arm!");
            fs::write(&source, bytes).expect("fixture should be written");

            let slice = read_native_slice(&source, CPU_TYPE_ARM64)
                .expect("fixture should be readable")
                .expect("arm64 slice should be found");
            assert_eq!(slice.offset, 8_192);
            assert_eq!(slice.bytes, 4);
            write_native_slice(
                &source,
                &destination,
                &slice,
                fs::metadata(&source)
                    .expect("fixture metadata should be readable")
                    .permissions(),
            )
            .expect("native slice should be extracted");
            assert_eq!(
                fs::read(&destination).expect("native output should be readable"),
                b"arm!"
            );
            fs::remove_dir_all(root).expect("temporary fixture should be removed");
        }

        #[test]
        fn hard_link_transaction_restores_the_original_atomically() {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock should be available")
                .as_nanos();
            let root = env::temp_dir().join(format!(
                "mangodisk-universal-rollback-{}-{unique}",
                std::process::id()
            ));
            fs::create_dir_all(&root).expect("temporary directory should be created");
            let executable = root.join("executable");
            let backup = root.join("original");
            let replacement = root.join("native");
            fs::write(&executable, b"universal").expect("original should be written");
            fs::hard_link(&executable, &backup).expect("backup hard link should be created");
            fs::write(&replacement, b"native").expect("replacement should be written");
            fs::rename(&replacement, &executable).expect("replacement should be installed");

            rollback_candidate(&executable, &backup).expect("rollback should succeed");
            assert_eq!(
                fs::read(&executable).expect("restored executable should be readable"),
                b"universal"
            );
            fs::remove_dir_all(root).expect("temporary fixture should be removed");
        }

        #[test]
        fn failed_transaction_rollback_retains_the_original_hard_link() {
            let root = temporary_root("rollback-failure");
            let backup = root.join("original");
            let temporary = root.join("native");
            fs::write(&backup, b"universal").expect("backup should be written");
            fs::write(&temporary, b"native").expect("temporary file should be written");
            let staged = StagedCandidate {
                executable_path: root.join("missing-parent/executable"),
                temporary_path: temporary.clone(),
                backup_path: backup.clone(),
                installed: true,
            };

            let result = rollback_transaction(&[staged], &root);

            assert!(result.is_err(), "the unavailable destination must fail");
            assert!(backup.is_file(), "the recovery copy must be retained");
            assert!(
                !temporary.exists(),
                "temporary native slices remain safe to remove"
            );
            fs::remove_dir_all(root).expect("temporary fixture should be removed");
        }

        #[test]
        fn unwritable_directory_requires_authorized_application_transaction() {
            let root = temporary_root("permission-check");
            let protected = root.join("protected");
            fs::create_dir(&protected).expect("protected directory should be created");
            fs::set_permissions(&protected, fs::Permissions::from_mode(0o555))
                .expect("protected permissions should be applied");

            assert!(
                !directory_is_writable(&protected),
                "unprivileged cleanup must not update a protected directory"
            );

            fs::set_permissions(&protected, fs::Permissions::from_mode(0o755))
                .expect("cleanup permissions should be restored");
            fs::remove_dir_all(root).expect("temporary fixture should be removed");
        }

        #[test]
        fn signature_manifest_excludes_macho_files_sealed_as_resources() {
            let root = temporary_root("signature-manifest");
            let bundle = root.join("Fixture.app");
            let main = bundle.join("Contents/MacOS/Fixture");
            let nested_code = bundle.join("Contents/Frameworks/signed-helper");
            let sealed_resource = bundle.join("Contents/Resources/resource-helper");
            fs::create_dir_all(main.parent().expect("main executable should have a parent"))
                .expect("main directory should be created");
            fs::create_dir_all(
                nested_code
                    .parent()
                    .expect("nested executable should have a parent"),
            )
            .expect("nested code directory should be created");
            fs::create_dir_all(
                sealed_resource
                    .parent()
                    .expect("resource executable should have a parent"),
            )
            .expect("resource directory should be created");
            write_universal_fixture(&main);
            write_universal_fixture(&nested_code);
            write_universal_fixture(&sealed_resource);

            let mut info = plist::Dictionary::new();
            info.insert(
                "CFBundleExecutable".to_string(),
                Value::String("Fixture".to_string()),
            );
            Value::Dictionary(info)
                .to_file_xml(bundle.join("Contents/Info.plist"))
                .expect("Info.plist should be written");

            let signature_directory = bundle.join("Contents/_CodeSignature");
            fs::create_dir_all(&signature_directory)
                .expect("signature directory should be created");
            let mut signed_attributes = plist::Dictionary::new();
            signed_attributes.insert(
                "requirement".to_string(),
                Value::String("identifier signed-helper".to_string()),
            );
            let mut resource_attributes = plist::Dictionary::new();
            resource_attributes.insert("hash2".to_string(), Value::Data(vec![0; 32]));
            let mut files = plist::Dictionary::new();
            files.insert(
                "Frameworks/signed-helper".to_string(),
                Value::Dictionary(signed_attributes),
            );
            files.insert(
                "Resources/resource-helper".to_string(),
                Value::Dictionary(resource_attributes),
            );
            let mut manifest = plist::Dictionary::new();
            manifest.insert("files2".to_string(), Value::Dictionary(files));
            Value::Dictionary(manifest)
                .to_file_xml(signature_directory.join("CodeResources"))
                .expect("signature manifest should be written");

            let discovered =
                discover_signed_executables(&bundle, std::slice::from_ref(&main), &|| false);
            assert!(discovered.contains(&fs::canonicalize(main).expect("main should resolve")));
            assert!(discovered
                .contains(&fs::canonicalize(nested_code).expect("nested code should resolve")));
            assert!(!discovered.contains(
                &fs::canonicalize(sealed_resource).expect("resource code should resolve")
            ));
            fs::remove_dir_all(root).expect("temporary fixture should be removed");
        }

        #[test]
        #[ignore = "reads installed macOS applications"]
        fn real_preview_reports_discovered_application_binaries() {
            let context = ScanContext::capture();
            let rule = preview(&context.inventory, &|| false, &|_| {});
            println!(
                "universal binary preview: components={}, bytes={}, applications={}, elapsed_ms={}",
                rule.file_count, rule.bytes, rule.source_count, rule.scan_elapsed_ms
            );
            for source in &rule.sources {
                println!(
                    "  bytes={} components={} block_reason={:?} path={}",
                    source.bytes, source.file_count, source.block_reason, source.path
                );
            }
            assert_eq!(rule.rule_id, CLEANER_ID);
            assert_eq!(rule.risk, RiskLevel::Recoverable);
            assert!(!rule.default_selected);
            assert!(rule.source_count <= rule.file_count);
            assert!(rule
                .sources
                .iter()
                .all(|source| source.path.ends_with(".app") && source.bytes > 0));
        }

        #[test]
        #[ignore = "copies and optimizes a locally installed application"]
        fn real_application_copy_remains_signed_and_executable() {
            let source_bundle = Path::new("/Applications/Godot_mono.app");
            let source_executable = source_bundle.join("Contents/MacOS/Godot");
            if !source_executable.is_file() {
                println!("Godot_mono.app is unavailable; isolated execution check skipped");
                return;
            }

            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock should be available")
                .as_nanos();
            let root = env::temp_dir().join(format!(
                "mangodisk-universal-execution-{}-{unique}",
                std::process::id()
            ));
            fs::create_dir_all(&root).expect("temporary directory should be created");
            let bundle = root.join("Godot_mono.app");
            let copy = Command::new("/usr/bin/ditto")
                .arg(source_bundle)
                .arg(&bundle)
                .output()
                .expect("ditto should start");
            assert!(copy.status.success(), "application copy should succeed");

            let executable = bundle.join("Contents/MacOS/Godot");
            let metadata = fs::metadata(&executable).expect("copied executable should exist");
            let native_slice = read_native_slice(&executable, native_cpu_type())
                .expect("copied executable should be readable")
                .expect("copied executable should be universal");
            let candidate = UniversalBinaryCandidate {
                application_name: "Godot".to_string(),
                application_bundle_path: bundle.clone(),
                executable_path: executable.clone(),
                reclaimable_bytes: metadata.len() - native_slice.bytes,
                native_slice,
                original_bytes: metadata.len(),
                modified_at_ms: None,
                block_reason: None,
            };
            let (released, optimized) = optimize_application(&[&candidate], 0, 0)
                .expect("isolated optimization should succeed");
            assert!(released > 0);
            assert_eq!(optimized, 1);
            let version = Command::new(&executable)
                .arg("--version")
                .output()
                .expect("optimized executable should start");
            assert!(version.status.success(), "optimized application should run");
            assert!(
                String::from_utf8_lossy(&version.stdout).contains("4."),
                "optimized application should report its version"
            );
            let assessment = Command::new("/usr/sbin/spctl")
                .args(["--assess", "--type", "execute"])
                .arg(&bundle)
                .output()
                .expect("Gatekeeper assessment should start");
            assert!(
                assessment.status.success(),
                "optimized application should retain its notarized trust: {}",
                String::from_utf8_lossy(&assessment.stderr)
            );
            fs::remove_dir_all(root).expect("temporary application copy should be removed");
        }

        #[test]
        #[ignore = "copies and optimizes nested signed code in a locally installed application"]
        fn real_nested_code_copy_retains_outer_signature_and_gatekeeper_trust() {
            let source_bundle = Path::new("/Applications/Discord.app");
            if !source_bundle.is_dir() {
                println!("Discord.app is unavailable; nested code check skipped");
                return;
            }
            let root = temporary_root("nested-execution");
            let bundle = root.join("Discord.app");
            let copy = Command::new("/usr/bin/ditto")
                .arg(source_bundle)
                .arg(&bundle)
                .output()
                .expect("ditto should start");
            assert!(copy.status.success(), "application copy should succeed");

            let main = bundle.join("Contents/MacOS/Discord");
            let executable_paths = discover_signed_executables(&bundle, &[main], &|| false);
            let candidates = executable_paths
                .into_iter()
                .filter_map(|executable_path| {
                    candidate_for_path("Discord", &bundle, executable_path)
                })
                .collect::<Vec<_>>();
            let references = candidates.iter().collect::<Vec<_>>();
            let resource_binary = bundle.join("Contents/Resources/updater.node");
            let resource_bytes_before = fs::metadata(&resource_binary)
                .expect("resource binary should exist")
                .len();

            let (released, optimized) = optimize_application(&references, 0, 0)
                .expect("nested code optimization should succeed");
            assert!(released > 150 * 1024 * 1024);
            assert!(optimized > 1);
            assert_eq!(
                fs::metadata(&resource_binary)
                    .expect("resource binary should remain")
                    .len(),
                resource_bytes_before,
                "Mach-O code sealed as a resource must remain unchanged"
            );
            verify_bundle(&bundle, "test").expect("optimized application should remain signed");
            assert!(
                assess_gatekeeper(&bundle),
                "optimized application should retain Gatekeeper trust"
            );
            fs::remove_dir_all(root).expect("temporary application copy should be removed");
        }

        #[test]
        #[ignore = "copies and optimizes a locally installed Mac App Store application"]
        fn real_app_store_copy_retains_receipt_signature_and_gatekeeper_trust() {
            let source_bundle = Path::new("/Applications/BaiduNetdisk.app");
            if !source_bundle.join("Contents/_MASReceipt/receipt").is_file() {
                println!("BaiduNetdisk.app receipt is unavailable; App Store check skipped");
                return;
            }
            let root = temporary_root("app-store-execution");
            let bundle = root.join("BaiduNetdisk.app");
            let copy = Command::new("/usr/bin/ditto")
                .arg(source_bundle)
                .arg(&bundle)
                .output()
                .expect("ditto should start");
            assert!(copy.status.success(), "application copy should succeed");

            let receipt = bundle.join("Contents/_MASReceipt/receipt");
            let receipt_hash_before =
                blake3::hash(&fs::read(&receipt).expect("receipt should be readable"));
            let main =
                bundle_main_executable(&bundle).expect("application main executable should exist");
            let executable_paths = discover_signed_executables(&bundle, &[main], &|| false);
            let candidates = executable_paths
                .into_iter()
                .filter_map(|executable_path| {
                    candidate_for_path("BaiduNetdisk", &bundle, executable_path)
                })
                .collect::<Vec<_>>();
            let references = candidates.iter().collect::<Vec<_>>();
            let (released, optimized) = optimize_application(&references, 0, 0)
                .expect("App Store application optimization should succeed");

            assert!(released > 150 * 1024 * 1024);
            assert!(optimized > 1);
            assert_eq!(
                blake3::hash(&fs::read(&receipt).expect("receipt should remain readable")),
                receipt_hash_before,
                "the App Store receipt must remain unchanged"
            );
            verify_bundle(&bundle, "test").expect("optimized application should remain signed");
            assert!(
                assess_gatekeeper(&bundle),
                "optimized App Store application should retain Gatekeeper trust"
            );
            fs::remove_dir_all(root).expect("temporary application copy should be removed");
        }

        #[test]
        #[ignore = "copies and optimizes a large application with many nested components"]
        fn real_large_nested_application_copy_remains_valid() {
            const WECOM_BUNDLE_NAME: &str = "\u{4f01}\u{4e1a}\u{5fae}\u{4fe1}.app";
            let source_bundle = Path::new("/Applications").join(WECOM_BUNDLE_NAME);
            if !source_bundle.is_dir() {
                println!("WeCom is unavailable; large nested application check skipped");
                return;
            }
            let root = temporary_root("large-nested-execution");
            let bundle = root.join(WECOM_BUNDLE_NAME);
            let copy = Command::new("/usr/bin/ditto")
                .arg(&source_bundle)
                .arg(&bundle)
                .output()
                .expect("ditto should start");
            assert!(copy.status.success(), "application copy should succeed");

            let main =
                bundle_main_executable(&bundle).expect("application main executable should exist");
            let executable_paths = discover_signed_executables(&bundle, &[main], &|| false);
            let candidates = executable_paths
                .into_iter()
                .filter_map(|executable_path| candidate_for_path("WeCom", &bundle, executable_path))
                .collect::<Vec<_>>();
            let references = candidates.iter().collect::<Vec<_>>();
            let (released, optimized) = optimize_application(&references, 0, 0)
                .expect("large nested application optimization should succeed");

            assert!(released > 500 * 1024 * 1024);
            assert!(optimized > 100);
            verify_bundle(&bundle, "test").expect("optimized application should remain signed");
            assert!(
                assess_gatekeeper(&bundle),
                "optimized application should retain Gatekeeper trust"
            );
            fs::remove_dir_all(root).expect("temporary application copy should be removed");
        }

        fn candidate_for_path(
            application_name: &str,
            application_bundle_path: &Path,
            executable_path: PathBuf,
        ) -> Option<UniversalBinaryCandidate> {
            let metadata = fs::metadata(&executable_path).ok()?;
            let native_slice = read_native_slice(&executable_path, native_cpu_type())
                .ok()
                .flatten()?;
            Some(UniversalBinaryCandidate {
                application_name: application_name.to_string(),
                application_bundle_path: application_bundle_path.to_path_buf(),
                executable_path,
                reclaimable_bytes: metadata.len().saturating_sub(native_slice.bytes),
                native_slice,
                original_bytes: metadata.len(),
                modified_at_ms: None,
                block_reason: None,
            })
        }

        fn temporary_root(label: &str) -> PathBuf {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock should be available")
                .as_nanos();
            let root = env::temp_dir().join(format!(
                "mangodisk-universal-{label}-{}-{unique}",
                std::process::id()
            ));
            fs::create_dir_all(&root).expect("temporary directory should be created");
            root
        }

        fn write_universal_fixture(path: &Path) {
            let mut bytes = vec![0_u8; 8_196];
            bytes[0..4].copy_from_slice(&FAT_MAGIC.to_be_bytes());
            bytes[4..8].copy_from_slice(&2_u32.to_be_bytes());
            write_fat_arch(&mut bytes[8..28], CPU_TYPE_X86_64, 4_096, 4);
            write_fat_arch(&mut bytes[28..48], CPU_TYPE_ARM64, 8_192, 4);
            bytes[4_096..4_100].copy_from_slice(b"x86!");
            bytes[8_192..8_196].copy_from_slice(b"arm!");
            fs::write(path, bytes).expect("universal fixture should be written");
        }

        fn write_fat_arch(buffer: &mut [u8], cpu_type: u32, offset: u32, bytes: u32) {
            buffer[0..4].copy_from_slice(&cpu_type.to_be_bytes());
            buffer[4..8].copy_from_slice(&0_u32.to_be_bytes());
            buffer[8..12].copy_from_slice(&offset.to_be_bytes());
            buffer[12..16].copy_from_slice(&bytes.to_be_bytes());
            buffer[16..20].copy_from_slice(&0_u32.to_be_bytes());
        }
    }
}

#[cfg(target_os = "macos")]
pub(crate) use platform::{execute, preview};

#[cfg(not(target_os = "macos"))]
pub(crate) fn preview(
    _inventory: &ApplicationInventory,
    _is_cancelled: &(dyn Fn() -> bool + Sync),
    _report_path: &(dyn Fn(&std::path::Path) + Sync),
) -> ScanRuleResult {
    not_applicable_rule(0)
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn execute(
    _inventory: &ApplicationInventory,
    _source_scope: Option<&SourceScope>,
    _dry_run: bool,
    _operation: &OperationGuard,
) -> CleanupActionResult {
    CleanupActionResult {
        rule_id: CLEANER_ID.to_string(),
        action_kind: CleanupActionKind::Optimize,
        status: CleanupActionStatus::Failed,
        reason_code: Some(CleanupActionReason::CleanerUnavailable),
        bytes_expected: 0,
        released_bytes: 0,
        affected_item_count: 0,
        failed_item_count: 1,
        running_processes: Vec::new(),
    }
}

pub(crate) fn limited_rule(elapsed_ms: u64) -> ScanRuleResult {
    ScanRuleResult {
        rule_id: CLEANER_ID.to_string(),
        category: crate::cleanup::CleanupCategory::ApplicationOptimization,
        group: CleanupGroup::ApplicationOptimization,
        risk: RiskLevel::Recoverable,
        default_selected: false,
        recommended_selected: false,
        bytes: 0,
        file_count: 0,
        available: true,
        selectable: false,
        status: ScanItemStatus::Limited,
        running_processes: Vec::new(),
        requires_app_close: false,
        sources: Vec::new(),
        source_count: 0,
        sources_truncated: false,
        scan_elapsed_ms: elapsed_ms,
    }
}

#[cfg(not(target_os = "macos"))]
fn not_applicable_rule(elapsed_ms: u64) -> ScanRuleResult {
    ScanRuleResult {
        rule_id: CLEANER_ID.to_string(),
        category: crate::cleanup::CleanupCategory::ApplicationOptimization,
        group: CleanupGroup::ApplicationOptimization,
        risk: RiskLevel::Recoverable,
        default_selected: false,
        recommended_selected: false,
        bytes: 0,
        file_count: 0,
        available: false,
        selectable: false,
        status: ScanItemStatus::NotApplicable,
        running_processes: Vec::new(),
        requires_app_close: false,
        sources: Vec::new(),
        source_count: 0,
        sources_truncated: false,
        scan_elapsed_ms: elapsed_ms,
    }
}
