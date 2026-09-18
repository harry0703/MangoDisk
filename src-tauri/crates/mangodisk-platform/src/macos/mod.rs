mod allocated_content;
mod analysis;
mod bulk_directory;
mod change_tracking;
mod directories;
mod directory_aggregate;
mod inventory;
mod privacy;
mod privileged_uninstall;
mod process_control;
mod project_markers;
mod startup;
mod system_maintenance;
mod system_settings;
mod volumes;

use std::{
    ffi::OsString,
    fs,
    io::{BufRead, BufReader, Read},
    os::macos::fs::MetadataExt as MacOsMetadataExt,
    os::unix::ffi::OsStringExt,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    sync::{
        atomic::{AtomicU64, AtomicUsize, Ordering},
        mpsc::{sync_channel, RecvTimeoutError},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

use crate::{
    ApplicationComponentAggregate, ApplicationComponentAggregateError, ApplicationDirectories,
    ApplicationProcessCloseMode, ApplicationProcessCloseResult, ApplicationProcessTarget,
    DirectoryTreeAggregate, DirectoryTreeAggregateError, FastAnalysisQuery, FastAnalysisRecord,
    FastAnalysisScanError, FastAnalysisSummary, FileSpaceUsage, FilesystemChangeImpactError,
    FilesystemChangeImpactOutcome, FilesystemChangeMonitor, FilesystemChangeToken,
    LargeFileCandidateScanError, LargeFileCandidateSummary, Platform, PlatformCancellation,
    PlatformError, PlatformResult, PlatformSystemSettingChangeRequest,
    PlatformSystemSettingChangeResult, PlatformSystemSettingState, PrivacyPlatform,
    ProjectMarkerCandidateProgress, ProjectMarkerCandidateQuery, ProjectMarkerCandidateScanError,
    ProjectMarkerCandidateSummary, ScanPurpose, SkipReason, StartupPlatform, SystemInventory,
    SystemMaintenancePlatform, SystemSettingsPlatform, UserDirectories, VolumeInfo,
};

const SPOTLIGHT_CANDIDATE_CHANNEL_CAPACITY: usize = 128;
const SPOTLIGHT_MAX_PATH_BYTES: u64 = 16 * 1024;
const COMMAND_DIAGNOSTIC_LIMIT_BYTES: usize = 64 * 1024;
const SPOTLIGHT_STATUS_TIMEOUT: Duration = Duration::from_secs(3);
const SPOTLIGHT_QUERY_TIMEOUT: Duration = Duration::from_secs(5);

pub struct MacOsPlatform;

// macOS has no supported local AI model store yet; keep the default empty
// result so discovery remains available and deterministic.
impl crate::AiModelDiscoveryPlatform for MacOsPlatform {}

impl PrivacyPlatform for MacOsPlatform {
    fn discover_privacy_sources(
        &self,
        cancellation: &PlatformCancellation,
    ) -> PlatformResult<crate::PlatformPrivacyDiscovery> {
        privacy::discover(cancellation)
    }

    fn clear_system_privacy_trace(
        &self,
        trace: crate::PlatformPrivacySystemTraceKind,
    ) -> PlatformResult<bool> {
        privacy::clear(trace)
    }

    fn clear_application_privacy_trace(
        &self,
        trace: crate::PlatformPrivacyApplicationNativeTraceKind,
    ) -> PlatformResult<bool> {
        privacy::clear_application_trace(trace)
    }

    fn system_privacy_trace_details(
        &self,
        trace: crate::PlatformPrivacySystemTraceKind,
        offset: u64,
        limit: u32,
    ) -> PlatformResult<Vec<crate::PlatformPrivacyDetailEntry>> {
        privacy::system_details(trace, offset, limit)
    }

    fn application_privacy_trace_details(
        &self,
        trace: crate::PlatformPrivacyApplicationNativeTraceKind,
        offset: u64,
        limit: u32,
    ) -> PlatformResult<Vec<crate::PlatformPrivacyDetailEntry>> {
        privacy::application_details(trace, offset, limit)
    }
}

impl StartupPlatform for MacOsPlatform {
    fn scan_startup_sources(
        &self,
        cancellation: &PlatformCancellation,
    ) -> PlatformResult<Vec<crate::PlatformStartupSourceResult>> {
        startup::scan(cancellation)
    }

    fn change_startup_item(
        &self,
        request: &crate::PlatformStartupChangeRequest,
        authorization_prompt: Option<&str>,
    ) -> PlatformResult<crate::PlatformStartupChangeResult> {
        startup::change(request, authorization_prompt)
    }

    fn change_startup_items(
        &self,
        requests: &[crate::PlatformStartupChangeRequest],
        authorization_prompt: Option<&str>,
    ) -> PlatformResult<Vec<PlatformResult<crate::PlatformStartupChangeResult>>> {
        startup::change_many(requests, authorization_prompt)
    }
}

impl SystemSettingsPlatform for MacOsPlatform {
    fn scan_system_settings(
        &self,
        setting_ids: &[&str],
        cancellation: &PlatformCancellation,
    ) -> PlatformResult<Vec<PlatformSystemSettingState>> {
        system_settings::scan(setting_ids, cancellation)
    }

    fn change_system_setting(
        &self,
        request: &PlatformSystemSettingChangeRequest,
    ) -> PlatformResult<PlatformSystemSettingChangeResult> {
        system_settings::change(request)
    }

    fn change_system_settings(
        &self,
        requests: &[PlatformSystemSettingChangeRequest],
    ) -> PlatformResult<Vec<PlatformResult<PlatformSystemSettingChangeResult>>> {
        system_settings::change_many(requests)
    }
}

impl SystemMaintenancePlatform for MacOsPlatform {
    fn scan_system_maintenance(
        &self,
        task_ids: &[&str],
        cancellation: &PlatformCancellation,
    ) -> PlatformResult<Vec<crate::PlatformSystemMaintenanceState>> {
        system_maintenance::scan(task_ids, cancellation)
    }

    fn execute_system_maintenance(
        &self,
        task_id: &str,
        cancellation: &PlatformCancellation,
        authorization_prompt: Option<&str>,
        progress: &crate::PlatformSystemMaintenanceProgressSink,
    ) -> PlatformResult<crate::PlatformSystemMaintenanceExecution> {
        system_maintenance::execute(task_id, cancellation, authorization_prompt, progress)
    }
}

pub(crate) fn startup_helper_change_many(
    requests: &[crate::startup_helper::StartupHelperChangeRequest],
    interactive_user_id: u32,
) -> Vec<PlatformResult<crate::PlatformStartupChangeResult>> {
    startup::helper_change_many(requests, interactive_user_id)
}

// Darwin exposes cloud placeholders through `SF_DATALESS` in `st_flags`. The value is part of
// the macOS `sys/stat.h` ABI but is not currently exported by the Rust libc crate.
pub(super) const SF_DATALESS: u32 = 0x4000_0000;

pub(super) fn is_dataless_flags(flags: u32) -> bool {
    flags & SF_DATALESS != 0
}

pub(crate) fn application_directories(identifier: &str) -> PlatformResult<ApplicationDirectories> {
    directories::application_directories(identifier)
}

pub fn remove_application_bundle_with_privileges(
    target: &Path,
    authorization_prompt: Option<&str>,
) -> PlatformResult<crate::MacosPrivilegedApplicationRemovalOutcome> {
    privileged_uninstall::remove_application_bundle_with_privileges(target, authorization_prompt)
}

pub fn macos_privileged_application_removal_supported(target: &Path) -> bool {
    privileged_uninstall::application_target_is_supported(target)
}

impl Platform for MacOsPlatform {
    fn os_name(&self) -> &'static str {
        "macos"
    }
    fn system_volume_path(&self) -> PathBuf {
        PathBuf::from("/")
    }
    fn system_volume(&self) -> PlatformResult<VolumeInfo> {
        volumes::system_volume().map_err(Into::into)
    }
    fn volumes(&self) -> PlatformResult<Vec<VolumeInfo>> {
        volumes::volumes().map_err(Into::into)
    }

    fn user_directories(&self) -> PlatformResult<UserDirectories> {
        directories::user_directories()
    }

    fn system_inventory(&self) -> PlatformResult<SystemInventory> {
        inventory::system_inventory(&PlatformCancellation::new(|| false)).map_err(Into::into)
    }

    fn system_inventory_with_cancellation(
        &self,
        cancellation: &PlatformCancellation,
    ) -> PlatformResult<SystemInventory> {
        inventory::system_inventory(cancellation).map_err(Into::into)
    }

    fn system_inventory_revision(&self) -> PlatformResult<String> {
        inventory::system_inventory_revision().map_err(Into::into)
    }

    fn system_inventory_revision_with_cancellation(
        &self,
        cancellation: &PlatformCancellation,
    ) -> PlatformResult<String> {
        inventory::system_inventory_revision_with_cancellation(cancellation).map_err(Into::into)
    }

    fn running_process_names(&self) -> PlatformResult<Vec<String>> {
        inventory::running_process_names(&PlatformCancellation::new(|| false)).map_err(Into::into)
    }

    fn running_process_names_with_cancellation(
        &self,
        cancellation: &PlatformCancellation,
    ) -> PlatformResult<Vec<String>> {
        inventory::running_process_names(cancellation).map_err(Into::into)
    }

    fn close_application_processes(
        &self,
        target: &ApplicationProcessTarget,
        mode: ApplicationProcessCloseMode,
    ) -> PlatformResult<ApplicationProcessCloseResult> {
        process_control::close(target, mode)
    }

    fn close_application_processes_many(
        &self,
        targets: &[ApplicationProcessTarget],
        mode: ApplicationProcessCloseMode,
    ) -> Vec<PlatformResult<ApplicationProcessCloseResult>> {
        process_control::close_many(targets, mode)
    }

    fn is_link_like(&self, metadata: &fs::Metadata) -> bool {
        metadata.file_type().is_symlink() || is_dataless_flags(MacOsMetadataExt::st_flags(metadata))
    }

    fn is_same_filesystem(&self, root: &fs::Metadata, candidate: &fs::Metadata) -> bool {
        root.dev() == candidate.dev()
    }

    fn file_space_usage(&self, _path: &Path, metadata: &fs::Metadata) -> FileSpaceUsage {
        FileSpaceUsage {
            logical_bytes: metadata.len(),
            allocated_bytes: metadata.blocks().saturating_mul(512),
        }
    }

    fn file_has_allocated_content(
        &self,
        file: &fs::File,
        logical_bytes: u64,
    ) -> PlatformResult<Option<bool>> {
        allocated_content::has_allocated_content(file, logical_bytes)
    }

    fn is_allowed_system_path_alias(&self, path: &Path) -> bool {
        // macOS maintains these three root links for compatibility with traditional Unix paths.
        // User temporary directories live under /var/folders, so rejecting /var as an ordinary
        // link would break cleanup, duplicate scanning, and development scopes under /tmp.
        // Only the exact system aliases are accepted; descendants and user-created links cannot
        // use this exception to bypass scope validation.
        matches!(path.to_str(), Some("/var" | "/tmp" | "/etc"))
    }

    fn should_skip(
        &self,
        path: &Path,
        scan_root: &Path,
        purpose: ScanPurpose,
    ) -> Option<SkipReason> {
        if purpose == ScanPurpose::Cleanup {
            return None;
        }
        if directories::is_system_critical(path) {
            return Some(SkipReason::SystemCritical);
        }
        if matches!(
            purpose,
            ScanPurpose::LargeFiles | ScanPurpose::DuplicateFiles
        ) && directories::is_shared_library_or_application(path)
        {
            return Some(SkipReason::SystemCritical);
        }
        if purpose == ScanPurpose::LargeFiles && directories::is_protected_large_file_scope(path) {
            return Some(SkipReason::SystemCritical);
        }
        if purpose == ScanPurpose::DuplicateFiles && directories::is_protected_duplicate_scope(path)
        {
            return Some(SkipReason::SystemCritical);
        }
        if purpose == ScanPurpose::DuplicateFiles
            && directories::is_transient_duplicate_scope(path, scan_root)
        {
            return Some(SkipReason::SystemCritical);
        }
        None
    }

    fn validate_cleanup_root(&self, path: &Path) -> PlatformResult<()> {
        let canonical = fs::canonicalize(path)
            .map_err(|error| PlatformError::io("canonicalize path", &error))?;
        if canonical.parent().is_none() {
            return Err(PlatformError::invalid_path(
                "cleanup of a volume root is forbidden",
            ));
        }
        if directories::is_protected_cleanup_path(&canonical) {
            return Err(PlatformError::invalid_path(
                "cleanup of a protected macOS directory is forbidden",
            ));
        }
        let known = self.user_directories()?;
        for name in [
            "Desktop",
            "Documents",
            "Downloads",
            "Pictures",
            "Movies",
            "Music",
        ] {
            if canonical.starts_with(known.home_directory().join(name)) {
                return Err(PlatformError::invalid_path(
                    "cleanup of a personal data directory is forbidden",
                ));
            }
        }
        Ok(())
    }

    fn validate_user_selected_cleanup_root(&self, path: &Path) -> PlatformResult<()> {
        let canonical = fs::canonicalize(path)
            .map_err(|error| PlatformError::io("canonicalize path", &error))?;
        if canonical.parent().is_none() {
            return Err(PlatformError::invalid_path(
                "cleanup of a volume root is forbidden",
            ));
        }
        if directories::is_protected_cleanup_path(&canonical) {
            return Err(PlatformError::invalid_path(
                "cleanup of a protected macOS directory is forbidden",
            ));
        }
        let user_directories = self.user_directories()?;
        if canonical.starts_with(user_directories.home_directory().join("Applications")) {
            return Err(PlatformError::invalid_path(
                "cleanup of an application installation directory is forbidden",
            ));
        }
        Ok(())
    }

    fn fast_directory_tree_aggregate(
        &self,
        root: &Path,
        is_cancelled: &(dyn Fn() -> bool + Sync),
        report_progress: &(dyn Fn(&Path, u64, u64) + Sync),
    ) -> Result<Option<DirectoryTreeAggregate>, DirectoryTreeAggregateError> {
        directory_aggregate::measure_cleanup(root, is_cancelled, report_progress).map(Some)
    }

    fn fast_project_artifact_tree_aggregate(
        &self,
        root: &Path,
        is_cancelled: &(dyn Fn() -> bool + Sync),
        report_progress: &(dyn Fn(&Path, u64, u64) + Sync),
    ) -> Result<Option<DirectoryTreeAggregate>, DirectoryTreeAggregateError> {
        directory_aggregate::measure_project_artifact(root, is_cancelled, report_progress).map(Some)
    }

    fn fast_application_component_aggregate(
        &self,
        root: &Path,
        is_cancelled: &(dyn Fn() -> bool + Sync),
        report_progress: &(dyn Fn(&Path, u64, u64) + Sync),
    ) -> Result<Option<ApplicationComponentAggregate>, ApplicationComponentAggregateError> {
        directory_aggregate::measure_application_component(root, is_cancelled, report_progress)
            .map(Some)
    }

    fn fast_analysis_records(
        &self,
        query: FastAnalysisQuery<'_>,
        is_cancelled: &(dyn Fn() -> bool + Sync),
        report_progress: &mut dyn FnMut(&Path, u64, u64),
        consumer: &mut dyn FnMut(FastAnalysisRecord) -> Result<(), String>,
    ) -> Result<Option<FastAnalysisSummary>, FastAnalysisScanError> {
        analysis::analyze_records(
            self,
            analysis::AnalysisScanRequest {
                root: query.root,
                purpose: query.purpose,
                large_file_minimum_bytes: query.large_file_minimum_bytes,
                is_cancelled,
                should_prune_directory: query.should_prune_directory,
                report_progress,
            },
            consumer,
        )
        .map(Some)
    }

    fn capture_filesystem_change_token(
        &self,
        root: &Path,
    ) -> PlatformResult<Option<FilesystemChangeToken>> {
        change_tracking::capture_token(root).map_err(Into::into)
    }

    fn start_filesystem_change_monitor(
        &self,
        root: &Path,
        token: &FilesystemChangeToken,
        is_cancelled: &(dyn Fn() -> bool + Sync),
    ) -> PlatformResult<Option<FilesystemChangeMonitor>> {
        change_tracking::start_monitor(root, token, is_cancelled).map_err(Into::into)
    }

    fn filesystem_change_monitor_is_continuous(&self) -> bool {
        true
    }

    fn filesystem_change_impact_plan(
        &self,
        root: &Path,
        token: &FilesystemChangeToken,
        is_cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<Option<FilesystemChangeImpactOutcome>, FilesystemChangeImpactError> {
        change_tracking::impact_plan(root, token, is_cancelled).map(Some)
    }

    fn fast_large_file_candidates(
        &self,
        root: &Path,
        minimum_bytes: u64,
        is_cancelled: &(dyn Fn() -> bool + Sync),
        consumer: &mut dyn FnMut(PathBuf) -> Result<(), String>,
    ) -> Result<Option<LargeFileCandidateSummary>, LargeFileCandidateScanError> {
        // Spotlight indexing is a volume-level setting. Resolve only the status-check target to
        // the containing mount; the actual query below remains restricted to the exact scan root.
        // This path intentionally implements an advisory quick scan and does not claim that every
        // recent file is indexed. Complete scans use the authoritative bulk-enumeration contract.
        let index_root = volumes::mount_point(root).map_err(|error| {
            LargeFileCandidateScanError::Platform(format!(
                "unable to resolve the Spotlight volume: {error}"
            ))
        })?;
        let mut index_status_command = Command::new("/usr/bin/mdutil");
        index_status_command
            .env("LC_ALL", "C")
            .arg("-s")
            .arg(index_root);
        let index_status =
            run_command_interruptible(index_status_command, SPOTLIGHT_STATUS_TIMEOUT, is_cancelled)
                .map_err(|error| {
                    if is_cancelled() {
                        LargeFileCandidateScanError::Cancelled
                    } else {
                        LargeFileCandidateScanError::Platform(format!(
                            "unable to inspect the Spotlight index state: {error}"
                        ))
                    }
                })?;
        if !index_status.status.success() || !spotlight_index_enabled(&index_status) {
            let detail = command_detail(&index_status);
            return Err(LargeFileCandidateScanError::Platform(format!(
                "Spotlight has not fully indexed the scan scope; use directory traversal: {detail}"
            )));
        }

        // Spotlight maintains file metadata in the background, so size queries usually complete
        // within a few hundred milliseconds. Passing Command arguments directly handles spaces and
        // non-ASCII paths while preventing shell injection through a user-controlled scan root.
        // The -0 protocol uses NUL separators, and a bounded channel consumes stdout incrementally
        // so paths containing newlines or non-Unicode bytes never require an unbounded candidate
        // vector.
        let query = format!("kMDItemFSSize >= {minimum_bytes}");
        let mut query_command = Command::new("/usr/bin/mdfind");
        query_command
            .env("LC_ALL", "C")
            .arg("-0")
            .arg("-onlyin")
            .arg(root)
            .arg(query);
        stream_nul_candidates(
            query_command,
            SPOTLIGHT_QUERY_TIMEOUT,
            is_cancelled,
            consumer,
        )
        .map(Some)
    }

    fn fast_project_marker_candidates(
        &self,
        query: ProjectMarkerCandidateQuery<'_>,
        is_cancelled: &(dyn Fn() -> bool + Sync),
        report_progress: &(dyn Fn(ProjectMarkerCandidateProgress) + Sync),
        consumer: &mut dyn FnMut(PathBuf) -> Result<(), String>,
    ) -> Result<Option<ProjectMarkerCandidateSummary>, ProjectMarkerCandidateScanError> {
        // Project discovery must be complete even when Spotlight is stale or disabled. The bulk
        // directory scanner keeps current filesystem semantics while avoiding one stat call per
        // entry and pruning generated trees before descent.
        project_markers::scan(
            project_markers::ProjectMarkerScanRequest {
                root: query.root,
                file_names: query.file_names,
                file_suffixes: query.file_suffixes,
                pruned_directory_names: query.pruned_directory_names,
                maximum_depth: query.maximum_depth,
                is_cancelled,
                report_progress,
            },
            consumer,
        )
        .map(Some)
    }
}

enum SpotlightStreamMessage {
    Candidate(Vec<u8>),
    ReadFailed(String),
}

struct BoundedDiagnostic {
    bytes: Vec<u8>,
    truncated: bool,
}

/// `mdfind` may return hundreds of thousands of paths. The stdout reader retains one path buffer
/// and propagates backpressure to the child through a fixed-capacity channel, so a slow Core
/// consumer cannot turn this path into another unbounded result collection.
fn stream_nul_candidates(
    mut command: Command,
    timeout: Duration,
    is_cancelled: &(dyn Fn() -> bool + Sync),
    consumer: &mut dyn FnMut(PathBuf) -> Result<(), String>,
) -> Result<LargeFileCandidateSummary, LargeFileCandidateScanError> {
    let started = Instant::now();
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| {
            LargeFileCandidateScanError::Platform(format!(
                "unable to start the Spotlight large-file query: {error}"
            ))
        })?;
    let stdout = match child.stdout.take() {
        Some(stdout) => stdout,
        None => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(LargeFileCandidateScanError::Platform(
                "unable to capture Spotlight stdout".to_string(),
            ));
        }
    };
    let stderr = match child.stderr.take() {
        Some(stderr) => stderr,
        None => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(LargeFileCandidateScanError::Platform(
                "unable to capture Spotlight stderr".to_string(),
            ));
        }
    };
    let (sender, receiver) = sync_channel(SPOTLIGHT_CANDIDATE_CHANNEL_CAPACITY);
    let pending = Arc::new(AtomicUsize::new(0));
    let reader_pending = Arc::clone(&pending);
    let peak_in_flight = Arc::new(AtomicUsize::new(0));
    let reader_peak_in_flight = Arc::clone(&peak_in_flight);
    let backpressure_nanos = Arc::new(AtomicU64::new(0));
    let reader_backpressure_nanos = Arc::clone(&backpressure_nanos);
    let stdout_reader = thread::spawn(move || {
        let result = read_nul_candidate_stream(BufReader::new(stdout), |bytes| {
            let queued = reader_pending
                .fetch_add(1, Ordering::AcqRel)
                .saturating_add(1);
            reader_peak_in_flight.fetch_max(queued, Ordering::AcqRel);
            let send_started = Instant::now();
            if sender
                .send(SpotlightStreamMessage::Candidate(bytes))
                .is_err()
            {
                reader_pending.fetch_sub(1, Ordering::AcqRel);
                return false;
            }
            let elapsed = u64::try_from(send_started.elapsed().as_nanos()).unwrap_or(u64::MAX);
            reader_backpressure_nanos.fetch_add(elapsed, Ordering::Relaxed);
            true
        });
        if let Err(error) = result {
            let _ = sender.send(SpotlightStreamMessage::ReadFailed(error));
        }
    });
    let stderr_reader = thread::spawn(move || drain_diagnostic_stream(stderr));

    let mut candidate_count = 0_u64;
    let mut consumer_elapsed = Duration::ZERO;
    let consume_result = loop {
        if is_cancelled() {
            break Err(LargeFileCandidateScanError::Cancelled);
        }
        if started.elapsed() >= timeout {
            break Err(LargeFileCandidateScanError::Platform(format!(
                "Spotlight large-file query timed out after {} ms",
                timeout.as_millis()
            )));
        }
        match receiver.recv_timeout(Duration::from_millis(25)) {
            Ok(SpotlightStreamMessage::Candidate(bytes)) => {
                pending.fetch_sub(1, Ordering::AcqRel);
                candidate_count = candidate_count.saturating_add(1);
                let path = PathBuf::from(OsString::from_vec(bytes));
                let consumer_started = Instant::now();
                if let Err(error) = consumer(path) {
                    break Err(LargeFileCandidateScanError::Consumer(error));
                }
                consumer_elapsed = consumer_elapsed.saturating_add(consumer_started.elapsed());
            }
            Ok(SpotlightStreamMessage::ReadFailed(error)) => {
                break Err(LargeFileCandidateScanError::Platform(format!(
                    "unable to read Spotlight large-file results: {error}"
                )));
            }
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => break Ok(()),
        }
    };

    drop(receiver);
    let mut consume_result = consume_result;
    let status_result = if consume_result.is_err() {
        let _ = child.kill();
        child.wait()
    } else {
        loop {
            if is_cancelled() {
                consume_result = Err(LargeFileCandidateScanError::Cancelled);
                let _ = child.kill();
                break child.wait();
            }
            if started.elapsed() >= timeout {
                consume_result = Err(LargeFileCandidateScanError::Platform(format!(
                    "Spotlight large-file query timed out after {} ms",
                    timeout.as_millis()
                )));
                let _ = child.kill();
                break child.wait();
            }
            match child.try_wait() {
                Ok(Some(status)) => break Ok(status),
                Ok(None) => thread::sleep(Duration::from_millis(25)),
                Err(error) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break Err(error);
                }
            }
        }
    };
    let stdout_result = stdout_reader.join().map_err(|_| {
        LargeFileCandidateScanError::Platform("Spotlight stdout reader panicked".to_string())
    });
    let stderr_result = match stderr_reader.join() {
        Ok(result) => result.map_err(|error| {
            LargeFileCandidateScanError::Platform(format!(
                "unable to read Spotlight stderr: {error}"
            ))
        }),
        Err(_) => Err(LargeFileCandidateScanError::Platform(
            "Spotlight stderr reader panicked".to_string(),
        )),
    };

    // Join both pipe readers before propagating the primary error; returning early after a wait or
    // read failure would otherwise leave threads running in the background. Consumer failure and
    // cancellation are caller-initiated primary outcomes and take precedence over cleanup errors.
    // A successful consumption path then validates wait, stdout, and stderr in that order.
    consume_result?;
    let status = status_result.map_err(|error| {
        LargeFileCandidateScanError::Platform(format!(
            "unable to wait for the Spotlight large-file query: {error}"
        ))
    })?;
    stdout_result?;
    let stderr = stderr_result?;
    if !status.success() {
        let mut detail = String::from_utf8_lossy(&stderr.bytes).trim().to_string();
        if stderr.truncated {
            detail.push_str(" [truncated]");
        }
        return Err(LargeFileCandidateScanError::Platform(format!(
            "Spotlight large-file query failed: {detail}"
        )));
    }

    Ok(LargeFileCandidateSummary {
        candidate_count,
        // Spotlight does not expose the number of unindexed directories. Core validates stale or
        // inaccessible candidates at consumption time and includes them in its unified skip count.
        skipped_count: 0,
        consumer_elapsed_ms: consumer_elapsed.as_millis() as u64,
        producer_backpressure_ms: backpressure_nanos.load(Ordering::Relaxed) / 1_000_000,
        peak_in_flight_candidates: peak_in_flight.load(Ordering::Acquire),
        strategy: "spotlight_stream",
    })
}

/// Keeping NUL protocol parsing separate from the child lifecycle permits a tiny buffer to cover
/// records split across chunks. A false callback result means the downstream consumer stopped, so
/// the reader exits instead of reporting a disconnected channel as a Spotlight protocol error.
fn read_nul_candidate_stream(
    mut reader: impl BufRead,
    mut publish: impl FnMut(Vec<u8>) -> bool,
) -> Result<(), String> {
    let mut bytes = Vec::new();
    loop {
        bytes.clear();
        let read = (&mut reader)
            .take(SPOTLIGHT_MAX_PATH_BYTES.saturating_add(1))
            .read_until(0, &mut bytes)
            .map_err(|error| error.to_string())?;
        if read == 0 {
            return Ok(());
        }
        if bytes.last() != Some(&0) {
            if read as u64 > SPOTLIGHT_MAX_PATH_BYTES {
                return Err(format!(
                    "Spotlight path exceeds the {SPOTLIGHT_MAX_PATH_BYTES}-byte safety limit"
                ));
            }
            return Err("Spotlight output contains a path without a NUL terminator".to_string());
        }
        bytes.pop();
        if bytes.len() as u64 > SPOTLIGHT_MAX_PATH_BYTES {
            return Err(format!(
                "Spotlight path exceeds the {SPOTLIGHT_MAX_PATH_BYTES}-byte safety limit"
            ));
        }
        if bytes.is_empty() {
            continue;
        }
        if !publish(std::mem::take(&mut bytes)) {
            return Ok(());
        }
    }
}

/// stderr must be drained continuously to prevent a full pipe from deadlocking the child. Only a
/// fixed diagnostic prefix is retained so abnormal command output cannot violate the streaming
/// scan memory bound.
fn drain_diagnostic_stream(mut reader: impl Read) -> std::io::Result<BoundedDiagnostic> {
    let mut retained = Vec::with_capacity(COMMAND_DIAGNOSTIC_LIMIT_BYTES);
    let mut buffer = [0_u8; 8 * 1024];
    let mut truncated = false;
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        let available = COMMAND_DIAGNOSTIC_LIMIT_BYTES.saturating_sub(retained.len());
        let retained_bytes = available.min(read);
        retained.extend_from_slice(&buffer[..retained_bytes]);
        truncated |= retained_bytes < read;
    }
    Ok(BoundedDiagnostic {
        bytes: retained,
        truncated,
    })
}

fn spotlight_index_enabled(output: &Output) -> bool {
    let detail = command_detail(output).to_ascii_lowercase();
    detail.contains("indexing enabled")
        && !detail.contains("indexing disabled")
        && !detail.contains("unknown indexing state")
}

fn command_detail(output: &Output) -> String {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    format!("{stdout}\n{stderr}").trim().to_string()
}

/// Even though this currently runs the small `mdutil` status check, external output size must not
/// be trusted through `read_to_end`. Two readers continuously drain both pipes while retaining only
/// bounded diagnostic prefixes. The calling thread polls cancellation and owns the child lifecycle
/// so abnormal output cannot deadlock the process or violate the scan memory bound.
fn run_command_interruptible(
    mut command: Command,
    timeout: Duration,
    is_cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<Output, String> {
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| error.to_string())?;
    let stdout = match child.stdout.take() {
        Some(stdout) => stdout,
        None => {
            let _ = child.kill();
            let _ = child.wait();
            return Err("unable to capture command stdout".to_string());
        }
    };
    let stderr = match child.stderr.take() {
        Some(stderr) => stderr,
        None => {
            let _ = child.kill();
            let _ = child.wait();
            return Err("unable to capture command stderr".to_string());
        }
    };
    let stdout_reader = thread::spawn(move || drain_diagnostic_stream(stdout));
    let stderr_reader = thread::spawn(move || drain_diagnostic_stream(stderr));

    let mut cancelled = false;
    let mut timed_out = false;
    let started = Instant::now();
    let status_result = loop {
        if is_cancelled() {
            cancelled = true;
            let _ = child.kill();
            break child.wait();
        }
        if started.elapsed() >= timeout {
            timed_out = true;
            let _ = child.kill();
            break child.wait();
        }
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) => thread::sleep(Duration::from_millis(25)),
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                break Err(error);
            }
        }
    };
    let stdout = stdout_reader
        .join()
        .map_err(|_| "command stdout reader panicked".to_string())
        .and_then(|result| result.map_err(|error| error.to_string()));
    let stderr = stderr_reader
        .join()
        .map_err(|_| "command stderr reader panicked".to_string())
        .and_then(|result| result.map_err(|error| error.to_string()));
    // Reap both readers before returning cancellation or a wait failure; repeated scans could
    // otherwise accumulate background threads. Cancellation is the caller's primary outcome and
    // takes precedence over a secondary wait failure after killing the child.
    if cancelled {
        let _ = stdout;
        let _ = stderr;
        return Err("scan cancelled".to_string());
    }
    if timed_out {
        let _ = stdout;
        let _ = stderr;
        return Err(format!(
            "command timed out after {} ms",
            timeout.as_millis()
        ));
    }
    let status = status_result.map_err(|error| error.to_string())?;
    Ok(Output {
        status,
        stdout: stdout?.bytes,
        stderr: stderr?.bytes,
    })
}

#[cfg(test)]
mod tests {
    use std::{
        env, fs,
        io::{BufReader, Cursor, Read},
        os::unix::{fs::symlink, process::ExitStatusExt},
        sync::atomic::AtomicBool,
    };

    use super::*;

    #[test]
    fn dataless_entries_are_content_access_boundaries() {
        assert!(is_dataless_flags(SF_DATALESS));
        assert!(is_dataless_flags(SF_DATALESS | 0x20));
        assert!(!is_dataless_flags(0));
    }

    #[test]
    fn filesystem_identity_distinguishes_a_mounted_filesystem() {
        let platform = MacOsPlatform;
        let root =
            fs::symlink_metadata("/").expect("the system volume metadata should be readable");
        let user_directory =
            fs::symlink_metadata("/Users").expect("the user directory metadata should be readable");
        let device_filesystem = fs::symlink_metadata("/dev")
            .expect("the device filesystem metadata should be readable");

        assert!(
            platform.is_same_filesystem(&root, &user_directory),
            "macOS firmlinks must remain inside the selected system volume scope"
        );
        assert!(
            !platform.is_same_filesystem(&root, &device_filesystem),
            "a mounted filesystem must not be traversed as part of the selected volume"
        );
    }

    #[test]
    fn large_file_policy_prunes_system_runtime_data_without_hiding_user_data() {
        let platform = MacOsPlatform;
        let root = Path::new("/");

        assert_eq!(
            platform.should_skip(
                Path::new("/private/var/db/example.db"),
                root,
                ScanPurpose::LargeFiles
            ),
            Some(SkipReason::SystemCritical)
        );
        assert_eq!(
            platform.should_skip(
                Path::new("/private/var/db/example.db"),
                root,
                ScanPurpose::Analysis
            ),
            None,
            "disk analysis must continue measuring system runtime allocation"
        );
        assert_eq!(
            platform.should_skip(
                Path::new("/private/var/folders/example/cache.bin"),
                root,
                ScanPurpose::LargeFiles
            ),
            None,
            "per-user temporary application data must remain discoverable"
        );
        assert_eq!(
            platform.should_skip(
                Path::new("/Users/example/Library/Application Support/model/weights.bin"),
                root,
                ScanPurpose::LargeFiles
            ),
            None,
            "large-file discovery must retain user application data"
        );
    }

    #[test]
    fn macos_system_aliases_are_allowed_but_custom_links_are_rejected() {
        let platform = MacOsPlatform;
        platform
            .validate_path_no_links(&env::temp_dir())
            .expect("the macOS user temporary directory may traverse the system /var alias");
        platform
            .validate_path_no_links(Path::new("/tmp"))
            .expect("the macOS system /tmp alias may be used as a scan scope");

        let root =
            env::temp_dir().join(format!("mangodisk-path-alias-test-{}", std::process::id()));
        let target = root.join("target");
        let alias = root.join("custom-link");
        fs::create_dir_all(&target).expect("path-alias fixture directory should be created");
        symlink(&target, &alias).expect("fixture symbolic link should be created");

        let result = platform.validate_path_no_links(&alias);

        assert!(
            result.is_err(),
            "a user-created symbolic link must be rejected"
        );
        fs::remove_file(alias).expect("fixture symbolic link should be removed");
        fs::remove_dir_all(root).expect("path-alias fixture directory should be removed");
    }

    #[test]
    fn spotlight_index_status_requires_an_explicit_enabled_state() {
        let output = |message: &str| Output {
            status: std::process::ExitStatus::from_raw(0),
            stdout: message.as_bytes().to_vec(),
            stderr: Vec::new(),
        };

        assert!(spotlight_index_enabled(&output("Indexing enabled.")));
        assert!(!spotlight_index_enabled(&output("Indexing disabled.")));
        assert!(!spotlight_index_enabled(&output(
            "Error: unknown indexing state."
        )));
    }

    #[test]
    fn spotlight_nul_stream_preserves_cross_buffer_and_non_unicode_paths() {
        let input = b"/tmp/hello\nworld\0/tmp/\xff.bin\0";
        let reader = BufReader::with_capacity(3, Cursor::new(input));
        let mut records = Vec::new();

        read_nul_candidate_stream(reader, |record| {
            records.push(record);
            true
        })
        .expect("a NUL path split across buffers should be parsed completely");

        assert_eq!(
            records,
            vec![b"/tmp/hello\nworld".to_vec(), b"/tmp/\xff.bin".to_vec()]
        );
    }

    #[test]
    fn spotlight_nul_stream_rejects_unterminated_tail() {
        let mut records = Vec::new();
        let error =
            read_nul_candidate_stream(Cursor::new(b"/tmp/complete\0/tmp/partial"), |record| {
                records.push(record);
                true
            })
            .expect_err("an unterminated tail must not be accepted as a complete path");

        assert_eq!(records, vec![b"/tmp/complete".to_vec()]);
        assert!(error.contains("without a NUL terminator"));
    }

    #[test]
    fn spotlight_nul_stream_rejects_oversized_record_early() {
        let mut input = vec![b'x'; SPOTLIGHT_MAX_PATH_BYTES as usize + 1];
        input.push(0);

        let error = read_nul_candidate_stream(Cursor::new(input), |_| true)
            .expect_err("an oversized protocol record must stop at the fixed limit");

        assert!(error.contains("exceeds"));
    }

    #[test]
    fn spotlight_nul_stream_accepts_record_at_safety_limit() {
        let expected = vec![b'x'; SPOTLIGHT_MAX_PATH_BYTES as usize];
        let mut input = expected.clone();
        input.push(0);
        let mut records = Vec::new();

        read_nul_candidate_stream(Cursor::new(input), |record| {
            records.push(record);
            true
        })
        .expect("a complete record exactly at the path limit should be accepted");

        assert_eq!(records, vec![expected]);
    }

    #[test]
    fn spotlight_diagnostic_reader_drains_but_bounds_retained_output() {
        let input = vec![b'e'; COMMAND_DIAGNOSTIC_LIMIT_BYTES + 4_096];

        let diagnostic = drain_diagnostic_stream(Cursor::new(input))
            .expect("diagnostic reader should drain input");

        assert_eq!(diagnostic.bytes.len(), COMMAND_DIAGNOSTIC_LIMIT_BYTES);
        assert!(diagnostic.truncated);
    }

    #[test]
    fn interruptible_command_bounds_status_output() {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "head -c 70000 /dev/zero"]);

        let output = run_command_interruptible(command, Duration::from_secs(3), &|| false)
            .expect("status command should complete and drain output");

        assert!(output.status.success());
        assert_eq!(output.stdout.len(), COMMAND_DIAGNOSTIC_LIMIT_BYTES);
    }

    #[test]
    fn spotlight_nul_stream_stops_when_consumer_closes() {
        let mut published = 0;
        read_nul_candidate_stream(Cursor::new(b"/a\0/b\0/c\0"), |_| {
            published += 1;
            false
        })
        .expect("a downstream stop is not a protocol failure");

        assert_eq!(published, 1);
    }

    #[test]
    fn spotlight_process_stops_after_consumer_failure() {
        let mut command = Command::new("/usr/bin/printf");
        command.arg("/a\\0/b\\0/c\\0");
        let mut consumed = 0;

        let error = stream_nul_candidates(command, Duration::from_secs(3), &|| false, &mut |_| {
            consumed += 1;
            if consumed == 2 {
                Err("fixture consumer failure".to_string())
            } else {
                Ok(())
            }
        })
        .expect_err("consumer failure must stop candidate production");

        assert_eq!(consumed, 2);
        assert!(matches!(
            error,
            LargeFileCandidateScanError::Consumer(ref detail)
                if detail == "fixture consumer failure"
        ));
    }

    #[test]
    fn spotlight_process_cancellation_reaps_child_promptly() {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "while :; do printf '/fixture/path\\000'; done"]);
        let cancelled = Arc::new(AtomicBool::new(false));
        let cancellation_signal = Arc::clone(&cancelled);
        let canceller = thread::spawn(move || {
            thread::sleep(Duration::from_millis(25));
            cancellation_signal.store(true, Ordering::Release);
        });
        let started = Instant::now();

        let error = stream_nul_candidates(
            command,
            Duration::from_secs(3),
            &|| cancelled.load(Ordering::Acquire),
            &mut |_| Ok(()),
        )
        .expect_err("cancellation must terminate the Spotlight child");
        canceller
            .join()
            .expect("cancellation thread should finish normally");

        assert!(matches!(error, LargeFileCandidateScanError::Cancelled));
        assert!(
            started.elapsed() < Duration::from_millis(250),
            "cancellation and child reaping should finish within the 250 ms acceptance window"
        );
    }

    #[test]
    fn spotlight_cancellation_works_after_child_closes_stdout() {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "exec 1>&-; while :; do :; done"]);
        let cancelled = Arc::new(AtomicBool::new(false));
        let cancellation_signal = Arc::clone(&cancelled);
        let canceller = thread::spawn(move || {
            thread::sleep(Duration::from_millis(25));
            cancellation_signal.store(true, Ordering::Release);
        });
        let started = Instant::now();

        let error = stream_nul_candidates(
            command,
            Duration::from_secs(3),
            &|| cancelled.load(Ordering::Acquire),
            &mut |_| Ok(()),
        )
        .expect_err("cancellation must still work after stdout closes");
        canceller
            .join()
            .expect("cancellation thread should finish normally");

        assert!(matches!(error, LargeFileCandidateScanError::Cancelled));
        assert!(
            started.elapsed() < Duration::from_millis(250),
            "closing stdout must not enter an uncancellable blocking wait"
        );
    }

    #[test]
    fn spotlight_query_timeout_reaps_a_stalled_child() {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "exec 1>&-; while :; do :; done"]);
        let started = Instant::now();

        let error =
            stream_nul_candidates(command, Duration::from_millis(50), &|| false, &mut |_| {
                Ok(())
            })
            .expect_err("a stalled Spotlight query must be terminated at its deadline");

        assert!(matches!(
            error,
            LargeFileCandidateScanError::Platform(ref detail) if detail.contains("timed out")
        ));
        assert!(
            started.elapsed() < Duration::from_millis(250),
            "timeout and child reaping should finish within the 250 ms acceptance window"
        );
    }

    /// The explicit capacity baseline runs this test. The generator emits one million records
    /// byte-by-byte without constructing one million files or a path vector, so process RSS
    /// directly reflects whether the parser keeps a constant working set.
    #[test]
    #[ignore = "executed explicitly by the Spotlight candidate capacity baseline"]
    fn spotlight_million_candidate_stream_stays_incremental() {
        const FIXTURE_COUNT_ENV: &str = "MANGODISK_CANDIDATE_FIXTURE_COUNT";

        struct CandidateBytes {
            remaining_bytes: u64,
            next_is_nul: bool,
        }

        impl Read for CandidateBytes {
            fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
                if self.remaining_bytes == 0 || buffer.is_empty() {
                    return Ok(0);
                }
                let count = usize::try_from(self.remaining_bytes.min(buffer.len() as u64))
                    .unwrap_or(buffer.len());
                for byte in &mut buffer[..count] {
                    *byte = if self.next_is_nul { 0 } else { b'x' };
                    self.next_is_nul = !self.next_is_nul;
                }
                self.remaining_bytes -= count as u64;
                Ok(count)
            }
        }

        let candidate_count = std::env::var(FIXTURE_COUNT_ENV)
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(1_000_000);
        assert!(
            candidate_count > 0,
            "candidate capacity fixture must contain at least one record"
        );
        let reader = BufReader::with_capacity(
            17,
            CandidateBytes {
                remaining_bytes: candidate_count.saturating_mul(2),
                next_is_nul: false,
            },
        );
        let mut count = 0_u64;
        read_nul_candidate_stream(reader, |record| {
            assert_eq!(record, b"x");
            count += 1;
            true
        })
        .expect("synthetic candidate records should be parsed incrementally");

        println!("candidate_stream count={count} parser_buffer_bytes=17");
        assert_eq!(count, candidate_count);
    }
}
