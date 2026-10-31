//! Analysis selection and scan-snapshot accounting over the shared deletion boundary.
use super::{AnalysisDeleteResult, AnalysisEntryCandidate};
use crate::filesystem::{
    metadata::diagnostic_path,
    permanent_delete::{
        delete_directory_tree_with_name_exclusions, delete_path_permanently,
        delete_prepared_directory_tree_from_snapshot, prepare_path_for_permanent_delete,
        reject_system_managed_file, PermanentDeleteError,
    },
};
use mangodisk_platform::{current_platform, FileSpaceUsage, Platform, ScanPurpose};
use std::{
    path::{Path, PathBuf},
    time::Instant,
};

/// Read-only analysis may display reserved entries; destructive actions use the
/// platform deletion policy for system and application-owned directory scopes.
pub(super) fn deletion_protection(
    path: &Path,
) -> Option<super::models::AnalysisDeletionProtection> {
    use super::models::AnalysisDeletionProtection;
    let platform = current_platform();
    if platform.is_system_managed_file(path) {
        Some(AnalysisDeletionProtection::SystemManagedFile)
    } else if platform.is_protected_deletion_path(path) {
        Some(AnalysisDeletionProtection::ProtectedPath)
    } else {
        None
    }
}

pub(crate) struct AnalysisDeleteOutcome {
    pub(crate) target: PathBuf,
    pub(crate) removed_usage: FileSpaceUsage,
    pub(crate) result: AnalysisDeleteResult,
}

/// Validates and permanently deletes one analysis entry without updating a domain cache.
/// The service owns session and index synchronization after this domain preflight.
/// Filesystem primitives own direct removal and partial-progress reporting.
pub(crate) fn delete_candidate(
    candidate: AnalysisEntryCandidate,
) -> Result<AnalysisDeleteOutcome, PermanentDeleteError> {
    let validation_started = Instant::now();
    reject_system_managed_file(Path::new(&candidate.path))?;
    if deletion_protection(Path::new(&candidate.path)).is_some() {
        return Err(
            "MangoDisk cannot process a protected system or application item"
                .to_string()
                .into(),
        );
    }
    let requested_root = PathBuf::from(&candidate.root);
    let root = current_platform()
        .canonicalize_no_links(&requested_root)
        .map_err(|error| format!("failed to access the current analysis directory: {error}"))?;
    let requested_target = PathBuf::from(&candidate.path);
    let prepared = prepare_path_for_permanent_delete(&requested_target)?;
    let requested_metadata = prepared.metadata();
    // Canonicalization follows symbolic links. Reject links and reparse
    // points before resolving the path so the target cannot be moved instead
    // of the link itself.
    if current_platform().is_link_like(requested_metadata) {
        return Err("MangoDisk cannot process a link or reparse point"
            .to_string()
            .into());
    }
    let target = current_platform()
        .canonicalize_no_links(&requested_target)
        .map_err(|error| format!("failed to access the requested item: {error}"))?;

    // The UI displays only direct children of the current directory. Enforce
    // the same boundary in Core so a modified request cannot target the scan
    // root or an arbitrary system path.
    if target.parent() != Some(root.as_path()) {
        return Err(
            "only direct children of the current analysis directory can be processed"
                .to_string()
                .into(),
        );
    }

    let metadata = prepared.metadata();
    let target_type_matches = if candidate.is_directory {
        metadata.is_dir()
    } else {
        metadata.is_file()
    };
    if !target_type_matches {
        return Err("the item type changed after scanning".to_string().into());
    }
    // The explicit confirmation authorizes deletion of the current regular
    // file or directory at this direct-child path. Scan metadata is retained
    // only for result accounting: rejecting normal changes between analysis
    // and confirmation made a user-requested delete appear broken. A lightweight
    // identity check precedes direct removal; later concurrent mutation is accepted.
    if current_platform()
        .should_skip(
            &target,
            &current_platform().system_volume_path(),
            ScanPurpose::Analysis,
        )
        .is_some()
        || deletion_protection(&target).is_some()
    {
        return Err(
            "MangoDisk cannot process a protected system or application item"
                .to_string()
                .into(),
        );
    }

    let mut names = mangodisk_platform::NameExclusions::compile(&candidate.exclusions.names)?;
    names.protect_folder_names(current_platform().protected_deletion_folder_names());
    let overlaps_path = candidate.exclusions.paths.iter().any(|excluded| {
        let excluded = Path::new(excluded);
        current_platform().path_is_same_or_child(&target, excluded)
            || (candidate.is_directory
                && current_platform().path_is_same_or_child(excluded, &target))
    });
    if overlaps_path || names.intersects_tree(&target, &|| false)? {
        log::info!(
            "analysis_delete_excluded path={} outcome=retained",
            diagnostic_path(&target)
        );
        return Err(PermanentDeleteError::before_mutation(
            "the item contains content excluded from this scan",
        ));
    }
    log::info!(
        "permanent_delete_stage_finished path={} stage=validate outcome=completed elapsed_ms={}",
        diagnostic_path(&target),
        validation_started.elapsed().as_millis()
    );
    if candidate.is_directory && !names.is_empty() {
        delete_directory_tree_with_name_exclusions(
            prepared,
            candidate.expected_logical_bytes,
            candidate.expected_file_count,
            &names,
            &|| false,
        )?;
    } else if candidate.is_directory {
        delete_prepared_directory_tree_from_snapshot(
            prepared,
            candidate.expected_logical_bytes,
            candidate.expected_file_count,
        )?;
    } else {
        delete_path_permanently(
            prepared,
            candidate.expected_logical_bytes,
            candidate.expected_file_count,
        )?;
    }
    let removed_usage = FileSpaceUsage {
        logical_bytes: candidate.expected_logical_bytes,
        allocated_bytes: candidate.expected_displayed_bytes,
    };
    Ok(AnalysisDeleteOutcome {
        target,
        removed_usage,
        result: AnalysisDeleteResult {
            schema_version: AnalysisDeleteResult::SCHEMA_VERSION,
            requires_rescan: false,
            removed_path: candidate.path,
            released_bytes: candidate.expected_displayed_bytes,
            removed_file_count: candidate.expected_file_count,
            updated_results: Vec::new(),
            invalidated_scan_ids: Vec::new(),
        },
    })
}

#[cfg(test)]
#[path = "deletion_tests.rs"]
mod tests;
