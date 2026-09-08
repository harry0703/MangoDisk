use std::{
    fs,
    path::{Path, PathBuf},
};

use super::{
    ApplicationComponentAggregate, ApplicationComponentAggregateError, ApplicationProcessCloseMode,
    ApplicationProcessCloseResult, ApplicationProcessTarget, ApplicationUninstallExecutionOutcome,
    ApplicationUninstallPlatformError, ApplicationUninstallRegistration,
    ApplicationUninstallRegistrationState, DirectPhysicalDirectoryEnumeration,
    DirectoryEntryIdentities, DirectoryTreeAggregate, DirectoryTreeAggregateError,
    FastAnalysisQuery, FastAnalysisRecord, FastAnalysisScanError, FastAnalysisSummary,
    FileSpaceUsage, FilesystemChangeImpactError, FilesystemChangeImpactOutcome,
    FilesystemChangeMonitor, FilesystemChangeToken, LargeFileCandidateScanError,
    LargeFileCandidateSummary, PlatformCancellation, PlatformError, PlatformResult,
    ProjectMarkerCandidateProgress, ProjectMarkerCandidateQuery, ProjectMarkerCandidateScanError,
    ProjectMarkerCandidateSummary, RunningProcessIdentity, ScanPurpose, SkipReason,
    SystemInventory, UserDirectories, VolumeInfo,
};

pub trait Platform: Send + Sync {
    fn os_name(&self) -> &'static str;
    fn system_volume_path(&self) -> PathBuf;
    fn system_volume(&self) -> PlatformResult<VolumeInfo>;
    fn volumes(&self) -> PlatformResult<Vec<VolumeInfo>>;
    fn user_directories(&self) -> PlatformResult<UserDirectories>;
    /// Reads only the revision of application sources without enumerating
    /// applications. Core uses it to validate a session cache without rebuilding
    /// inventory on every scan or retaining stale install state indefinitely.
    fn system_inventory_revision(&self) -> PlatformResult<String>;
    fn system_inventory_revision_with_cancellation(
        &self,
        cancellation: &PlatformCancellation,
    ) -> PlatformResult<String> {
        if cancellation.is_cancelled() {
            return Err(PlatformError::operation_failed(
                "application inventory revision capture was cancelled",
            ));
        }
        self.system_inventory_revision()
    }
    fn system_inventory(&self) -> PlatformResult<SystemInventory>;
    /// Captures application inventory while allowing adapters to stop native
    /// enumeration and child processes. Platforms without a cancellable path
    /// retain the original contract but still fail before starting new work.
    fn system_inventory_with_cancellation(
        &self,
        cancellation: &PlatformCancellation,
    ) -> PlatformResult<SystemInventory> {
        if cancellation.is_cancelled() {
            return Err(PlatformError::operation_failed(
                "application inventory capture was cancelled",
            ));
        }
        self.system_inventory()
    }
    fn application_uninstall_registration_state(
        &self,
        _registration: &ApplicationUninstallRegistration,
    ) -> Result<ApplicationUninstallRegistrationState, ApplicationUninstallPlatformError> {
        Err(ApplicationUninstallPlatformError::Unsupported)
    }
    fn execute_application_uninstall_registration(
        &self,
        _registration: &ApplicationUninstallRegistration,
    ) -> Result<ApplicationUninstallExecutionOutcome, ApplicationUninstallPlatformError> {
        Err(ApplicationUninstallPlatformError::Unsupported)
    }
    /// Removes explicitly selected installation metadata and its owned registry children.
    /// Resolve only fixed registration sources, preserve files, and verify the entry is absent.
    /// Application state and uninstall capability do not restrict this independent action.
    fn remove_application_record(
        &self,
        _application_id: &str,
        _dry_run: bool,
    ) -> PlatformResult<()> {
        Err(PlatformError::new(
            super::PlatformErrorCode::Unsupported,
            "application record removal is unavailable",
        ))
    }
    /// Process state changes too frequently to share the inventory cache.
    /// Callers capture snapshots at the safety boundary appropriate to each
    /// scan or destructive rule and may refresh them during long operations.
    fn running_process_names(&self) -> PlatformResult<Vec<String>>;
    fn running_process_names_with_cancellation(
        &self,
        cancellation: &PlatformCancellation,
    ) -> PlatformResult<Vec<String>> {
        if cancellation.is_cancelled() {
            return Err(PlatformError::operation_failed(
                "process snapshot capture was cancelled",
            ));
        }
        self.running_process_names()
    }
    /// Captures process names and paths as separate facts.
    ///
    /// The default adapter preserves compatibility with platforms that expose
    /// command output as strings. Native implementations should override this
    /// method when a process name remains available after path lookup fails.
    fn running_process_identities_with_cancellation(
        &self,
        cancellation: &PlatformCancellation,
    ) -> PlatformResult<Vec<RunningProcessIdentity>> {
        self.running_process_names_with_cancellation(cancellation)
            .map(|processes| {
                processes
                    .into_iter()
                    .map(|value| {
                        let path = PathBuf::from(&value);
                        let executable_path = path.is_absolute().then(|| path.clone());
                        let executable_name = executable_path
                            .as_deref()
                            .and_then(Path::file_name)
                            .map(|name| name.to_string_lossy().into_owned())
                            .unwrap_or(value);
                        RunningProcessIdentity {
                            executable_name,
                            executable_path,
                        }
                    })
                    .collect()
            })
    }
    /// Requests a bounded close operation for process identities that Core
    /// resolved from trusted product metadata. Implementations must never
    /// terminate MangoDisk itself and must recheck the target after waiting.
    fn close_application_processes(
        &self,
        _target: &ApplicationProcessTarget,
        _mode: ApplicationProcessCloseMode,
    ) -> PlatformResult<ApplicationProcessCloseResult> {
        Err(PlatformError::new(
            super::PlatformErrorCode::Unsupported,
            "application process control is unsupported",
        ))
    }
    /// Closes several trusted targets within one bounded platform operation.
    ///
    /// Results preserve request order and isolate target-specific failures so
    /// Core can report partial effects instead of discarding earlier outcomes.
    /// Native implementations should override this method to share process
    /// snapshots and one wait deadline across the complete user selection.
    fn close_application_processes_many(
        &self,
        targets: &[ApplicationProcessTarget],
        mode: ApplicationProcessCloseMode,
    ) -> Vec<PlatformResult<ApplicationProcessCloseResult>> {
        targets
            .iter()
            .map(|target| self.close_application_processes(target, mode))
            .collect()
    }
    /// Classifies entries that must not be opened as ordinary resident files.
    ///
    /// Symbolic links and reparse points can escape the selected scope, while cloud placeholders
    /// can materialize remote content when opened. The shared safety predicate intentionally
    /// groups both cases so every scan and mutation boundary fails closed without another metadata
    /// lookup in its hot path.
    fn is_link_like(&self, metadata: &fs::Metadata) -> bool;
    /// Reports whether two entries belong to the same mounted filesystem.
    ///
    /// Core owns the product rule that a scan stays inside its selected volume;
    /// platform adapters expose only the filesystem identity fact needed to
    /// enforce that rule without path-name heuristics or an extra metadata call.
    fn is_same_filesystem(&self, _root: &fs::Metadata, _candidate: &fs::Metadata) -> bool {
        true
    }
    /// Returns both identity and volume-usage sizes from one filesystem snapshot.
    ///
    /// The portable fallback preserves logical size when a platform cannot expose
    /// allocation without a substantially more expensive lookup. Native analysis
    /// implementations should report exact allocation from their batch metadata.
    fn file_space_usage(&self, _path: &Path, metadata: &fs::Metadata) -> FileSpaceUsage {
        FileSpaceUsage::logical_only(metadata.len())
    }
    /// Reports whether an already-open file has any physically allocated content range.
    ///
    /// `Some(false)` is an authoritative platform guarantee that the complete logical range is a
    /// hole and therefore reads as zero. `Some(true)` means at least one allocated range exists;
    /// it does not classify the remaining ranges. `None` selects ordinary content reading. Core
    /// validates the open object and current path before trusting this layout-only fact.
    fn file_has_allocated_content(
        &self,
        _file: &fs::File,
        _logical_bytes: u64,
    ) -> PlatformResult<Option<bool>> {
        Ok(None)
    }
    /// Returns stable physical identities for immediate directory entries in one native batch.
    ///
    /// Core treats these values only as hints and retains live metadata, link, and open-handle
    /// validation. `None` selects the portable per-file identity lookup. A platform error also
    /// falls back safely and must not make an otherwise readable directory disappear from scans.
    /// Implementations must check cancellation between bounded native batches.
    fn directory_entry_identities(
        &self,
        _directory: &Path,
        _cancellation: &PlatformCancellation,
    ) -> PlatformResult<Option<DirectoryEntryIdentities>> {
        Ok(None)
    }
    /// Some systems expose stable compatibility aliases such as macOS
    /// `/var -> /private/var`. Platform-owned aliases with fixed targets may be
    /// traversed as ancestors; user-created links remain rejected so operations
    /// cannot escape the selected physical scope.
    fn is_allowed_system_path_alias(&self, _path: &Path) -> bool {
        false
    }
    /// Converts a native path to the stable representation exposed across adapter boundaries.
    /// The default preserves Unix path text; Windows removes canonicalization-only prefixes.
    fn display_path(&self, path: &Path) -> String {
        path.to_string_lossy().into_owned()
    }
    /// Produces a stable lexical identity key for sorting, hashing, and deduplication.
    /// The key is platform-specific and does not resolve links or access the filesystem.
    fn path_identity_key(&self, path: &Path) -> String {
        let value = self.display_path(path);
        let trimmed = value.trim_end_matches(std::path::MAIN_SEPARATOR);
        if trimmed.is_empty() && !value.is_empty() {
            std::path::MAIN_SEPARATOR.to_string()
        } else {
            trimmed.to_string()
        }
    }
    /// Reports whether two paths identify the same lexical platform location.
    /// This comparison does not access the filesystem or resolve links.
    fn paths_equal(&self, left: &Path, right: &Path) -> bool {
        self.path_identity_key(left) == self.path_identity_key(right)
    }
    /// Reports whether a path is equal to or contained by a root according to
    /// platform path identity rules. Mutation boundaries use this instead of
    /// lexical string comparison because Windows paths are case-insensitive and
    /// may carry a verbatim prefix after canonicalization.
    fn path_is_same_or_child(&self, path: &Path, root: &Path) -> bool {
        path.starts_with(root)
    }
    /// Returns the lexical path below `root` using the same platform identity
    /// semantics as `path_is_same_or_child`.
    ///
    /// Core uses this at policy and scan-plan boundaries so Windows verbatim
    /// prefixes and case folding are never reimplemented by individual domains.
    fn relative_path(&self, path: &Path, root: &Path) -> Option<PathBuf> {
        if self.paths_equal(path, root) {
            return Some(PathBuf::new());
        }
        path.strip_prefix(root).ok().map(Path::to_path_buf)
    }
    fn validate_path_no_links(&self, path: &Path) -> PlatformResult<()> {
        let absolute = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()
                .map_err(|error| PlatformError::io("locate current directory", &error))?
                .join(path)
        };
        for component_path in absolute.ancestors().collect::<Vec<_>>().into_iter().rev() {
            if component_path.as_os_str().is_empty() {
                continue;
            }
            let metadata = fs::symlink_metadata(component_path)
                .map_err(|error| PlatformError::io("validate path metadata", &error))?;
            if self.is_link_like(&metadata) && !self.is_allowed_system_path_alias(component_path) {
                return Err(PlatformError::invalid_path(
                    "path contains a link or reparse point",
                ));
            }
        }
        Ok(())
    }
    /// Resolves an explicitly selected directory or an OS-provided folder shortcut.
    /// Callers must retain the returned target, not the original alias. This entry
    /// boundary does not relax link checks during traversal or deletion.
    fn resolve_directory_entry(&self, path: &Path) -> PlatformResult<PathBuf> {
        self.canonicalize_no_links(path)
    }
    fn canonicalize_no_links(&self, path: &Path) -> PlatformResult<PathBuf> {
        self.validate_path_no_links(path)?;
        let canonical =
            fs::canonicalize(path).map_err(|error| PlatformError::io("access path", &error))?;
        self.validate_path_no_links(path)?;
        let verified =
            fs::canonicalize(path).map_err(|error| PlatformError::io("revalidate path", &error))?;
        if canonical != verified {
            return Err(PlatformError::invalid_path(
                "path changed during validation",
            ));
        }
        Ok(canonical)
    }
    fn should_skip(
        &self,
        path: &Path,
        scan_root: &Path,
        purpose: ScanPurpose,
    ) -> Option<SkipReason>;
    fn validate_cleanup_root(&self, path: &Path) -> PlatformResult<()>;
    /// Validates a cleanup root chosen explicitly by the user.
    ///
    /// Implementations may allow personal-content directories while retaining
    /// volume, operating-system, and application-installation boundaries.
    fn validate_user_selected_cleanup_root(&self, path: &Path) -> PlatformResult<()> {
        self.validate_cleanup_root(path)
    }

    /// Measures a complete directory through a platform-native bulk traversal.
    ///
    /// `None` selects the generic Core walker. Native implementations must use
    /// logical file lengths, reject links, retain direct-child source
    /// aggregates, and report every unreadable or unsupported entry as skipped
    /// so the fast path has the same safety semantics as the fallback.
    ///
    /// `report_progress` publishes already-measured file and byte batches.
    /// Implementations must report each completed file at most once, flush the
    /// final partial batch before success, and use bounded batches rather than
    /// one callback per entry. Callers own rollback when a native scan fails
    /// and falls back to the portable walker.
    fn fast_directory_tree_aggregate(
        &self,
        _root: &Path,
        _is_cancelled: &(dyn Fn() -> bool + Sync),
        _report_progress: &(dyn Fn(&Path, u64, u64) + Sync),
    ) -> Result<Option<DirectoryTreeAggregate>, DirectoryTreeAggregateError> {
        Ok(None)
    }

    /// Measures one generated project-artifact tree through a native bulk traversal.
    ///
    /// Unlike ordinary cleanup roots, generated dependency trees frequently contain symbolic
    /// links. The project-artifact domain counts each link's own metadata as one entry but never
    /// follows its target. Keeping this as a separate capability prevents the ordinary cleanup
    /// aggregate from silently changing its stricter link-rejection semantics.
    fn fast_project_artifact_tree_aggregate(
        &self,
        _root: &Path,
        _is_cancelled: &(dyn Fn() -> bool + Sync),
        _report_progress: &(dyn Fn(&Path, u64, u64) + Sync),
    ) -> Result<Option<DirectoryTreeAggregate>, DirectoryTreeAggregateError> {
        Ok(None)
    }

    /// Measures one application component without retaining paths or creating a safety snapshot.
    ///
    /// Implementations count only ordinary-file logical lengths. Links and reparse points are
    /// observed but neither followed nor reported as skipped because an application bundle may
    /// legitimately contain them and the existing catalog treats such components as complete.
    /// Every unreadable, malformed, unsupported, or concurrently removed ordinary entry must be
    /// reflected in `skipped_count`. `None` selects Core's semantics-equivalent portable walker.
    fn fast_application_component_aggregate(
        &self,
        _root: &Path,
        _is_cancelled: &(dyn Fn() -> bool + Sync),
        _report_progress: &(dyn Fn(&Path, u64, u64) + Sync),
    ) -> Result<Option<ApplicationComponentAggregate>, ApplicationComponentAggregateError> {
        Ok(None)
    }

    /// Enumerates physical direct-child directories without a metadata syscall per entry.
    ///
    /// Reparse points and symbolic links must never be returned. `maximum_entries` limits all
    /// observed directory entries, not only accepted directories, so native and portable callers
    /// inspect the same bounded prefix. `None` selects the portable implementation.
    fn fast_direct_physical_directories(
        &self,
        _root: &Path,
        _maximum_entries: usize,
        _is_cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<Option<DirectPhysicalDirectoryEnumeration>, DirectoryTreeAggregateError> {
        Ok(None)
    }

    /// Captures an event boundary before a full scan. `None` means the platform
    /// has no reliable implementation; callers may build a snapshot but cannot
    /// report the unsupported capability as verified unchanged state.
    fn capture_filesystem_change_token(
        &self,
        _root: &Path,
    ) -> PlatformResult<Option<FilesystemChangeToken>> {
        Ok(None)
    }

    /// Catches up history from a token and monitors the current session. `None`
    /// means callers must perform a full scan instead of inferring validity from
    /// a time window.
    fn start_filesystem_change_monitor(
        &self,
        _root: &Path,
        _token: &FilesystemChangeToken,
        _is_cancelled: &(dyn Fn() -> bool + Sync),
    ) -> PlatformResult<Option<FilesystemChangeMonitor>> {
        Ok(None)
    }

    /// Reports whether the monitor continues after history catch-up.
    ///
    /// Core uses this capability to decide whether monitoring must start before
    /// scanning. FSEvents must cover the scan window, while Windows USN queries a
    /// fixed upper boundary after scanning. Declaring the behavior avoids OS-name
    /// checks and redundant journal queries.
    fn filesystem_change_monitor_is_continuous(&self) -> bool {
        false
    }

    /// Computes directories requiring remeasurement between a stable token and
    /// the fixed upper boundary captured when the call starts.
    ///
    /// `None` means unsupported. `Unavailable` means the request was recognized
    /// but a complete impact scope could not be proven. Both require full-scan
    /// fallback. This interface plans changes and never mutates snapshots.
    fn filesystem_change_impact_plan(
        &self,
        _root: &Path,
        _token: &FilesystemChangeToken,
        _is_cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<Option<FilesystemChangeImpactOutcome>, FilesystemChangeImpactError> {
        Ok(None)
    }

    /// Streams size-qualified candidates from a platform fast path.
    ///
    /// `None` selects generic traversal. `Some` means candidate production
    /// completed, not that type, scope, size, or modification time is trusted.
    /// The core consumer revalidates every candidate. This contract supports
    /// Spotlight today and indexed Windows implementations without exposing
    /// platform commands to business code.
    fn fast_large_file_candidates(
        &self,
        _root: &Path,
        _minimum_bytes: u64,
        _is_cancelled: &(dyn Fn() -> bool + Sync),
        _consumer: &mut dyn FnMut(PathBuf) -> Result<(), String>,
    ) -> Result<Option<LargeFileCandidateSummary>, LargeFileCandidateScanError> {
        Ok(None)
    }

    /// Reports whether the native candidate source covers every reachable file in the scope.
    ///
    /// Advisory indexes such as Spotlight keep the default `false`. Authoritative filesystem
    /// layout readers override this so complete scans can avoid a second enumeration pipeline.
    fn fast_large_file_candidates_are_complete(&self) -> bool {
        false
    }

    /// Streams files whose names may identify a development project.
    ///
    /// `None` means the platform has no reliable native index for this scope.
    /// `Some` means candidate production completed, not that every candidate is
    /// valid. The core engine must enforce scan scope, reject links and generated
    /// artifact subtrees, and evaluate the full declarative project rule.
    fn fast_project_marker_candidates(
        &self,
        _query: ProjectMarkerCandidateQuery<'_>,
        _is_cancelled: &(dyn Fn() -> bool + Sync),
        _report_progress: &(dyn Fn(ProjectMarkerCandidateProgress) + Sync),
        _consumer: &mut dyn FnMut(PathBuf) -> Result<(), String>,
    ) -> Result<Option<ProjectMarkerCandidateSummary>, ProjectMarkerCandidateScanError> {
        Ok(None)
    }

    /// Streams complete directory aggregates and large-file index candidates.
    ///
    /// `None` means the root, filesystem, or permissions do not support native
    /// aggregation. Native implementations may stream post-order records while
    /// traversal is still in progress only when a later platform failure causes
    /// the caller to roll back every staged record before using the generic
    /// fallback. Progress batches contain direct files only; implementations
    /// must not report descendant aggregates because Core accumulates every
    /// batch. A successful `Some` result certifies a complete dataset.
    fn fast_analysis_records(
        &self,
        _query: FastAnalysisQuery<'_>,
        _is_cancelled: &(dyn Fn() -> bool + Sync),
        _report_progress: &mut dyn FnMut(&Path, u64, u64),
        _consumer: &mut dyn FnMut(FastAnalysisRecord) -> Result<(), String>,
    ) -> Result<Option<FastAnalysisSummary>, FastAnalysisScanError> {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::Platform;
    use crate::current::current_platform;

    #[test]
    fn path_validation_errors_do_not_expose_private_paths() {
        let private_marker = format!("mangodisk-private-path-{}", std::process::id());
        let missing_path = std::env::temp_dir().join(&private_marker);

        let error = current_platform()
            .canonicalize_no_links(&missing_path)
            .expect_err("a missing path should fail validation");

        assert!(
            !error.to_string().contains(&private_marker),
            "path validation diagnostics must not expose private path components"
        );
    }
}
