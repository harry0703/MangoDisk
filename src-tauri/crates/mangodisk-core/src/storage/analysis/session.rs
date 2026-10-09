use std::{
    collections::VecDeque,
    path::Path,
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex, MutexGuard, OnceLock,
    },
};

use mangodisk_platform::{current_platform, Platform};

use super::{
    AnalysisDirectoryNode, AnalysisEntryCandidate, AnalysisRemainderParent, AnalysisResult,
    DirectoryEntryInfo,
};

const ANALYSIS_RESULT_SESSION_LIMIT: usize = 80;

static NEXT_ANALYSIS_SCAN_ID: AtomicU64 = AtomicU64::new(1);
static ANALYSIS_RESULT_SESSIONS: OnceLock<Mutex<VecDeque<AnalysisSession>>> = OnceLock::new();

struct AnalysisSession {
    result: AnalysisResult,
    exclusions: crate::filesystem::ScanExclusionOptions,
}

fn sessions() -> &'static Mutex<VecDeque<AnalysisSession>> {
    ANALYSIS_RESULT_SESSIONS.get_or_init(|| Mutex::new(VecDeque::new()))
}

fn lock_sessions() -> Result<MutexGuard<'static, VecDeque<AnalysisSession>>, String> {
    sessions()
        .lock()
        .map_err(|_| "the disk-analysis result session is unavailable".to_string())
}

/// Publishes an authoritative result snapshot used by trusted follow-up operations.
///
/// The UI keeps a bounded navigation cache, so Core retains the same number of recent snapshots.
/// A cached UI result therefore remains usable without trusting snapshots reconstructed by the
/// WebView.
pub(super) fn publish_result_session(result: AnalysisResult) -> Result<AnalysisResult, String> {
    publish_result_with_exclusions(result, Default::default())
}

pub(super) fn publish_result_with_exclusions(
    mut result: AnalysisResult,
    exclusions: crate::filesystem::ScanExclusionOptions,
) -> Result<AnalysisResult, String> {
    result.scan_id = NEXT_ANALYSIS_SCAN_ID.fetch_add(1, Ordering::Relaxed);
    let mut sessions = lock_sessions()?;
    sessions.retain(|session| {
        !current_platform().paths_equal(Path::new(&session.result.root), Path::new(&result.root))
    });
    sessions.push_front(AnalysisSession {
        result: result.clone(),
        exclusions,
    });
    sessions.truncate(ANALYSIS_RESULT_SESSION_LIMIT);
    Ok(result)
}

/// Resolves a UI selection back to the complete snapshot owned by Core.
pub(super) fn resolve_entry_candidate(
    scan_id: u64,
    selected_path: &str,
) -> Result<AnalysisEntryCandidate, String> {
    let sessions = lock_sessions()?;
    let result = sessions
        .iter()
        .find(|result| result.result.scan_id == scan_id)
        .ok_or_else(|| "the disk-analysis result session expired; scan again".to_string())?;
    let entry = result
        .result
        .entries
        .iter()
        .find(|entry| entry.path == selected_path)
        .ok_or_else(|| "the selected item is not part of the current disk analysis".to_string())?;
    Ok(AnalysisEntryCandidate {
        scan_mode: result.result.scan_mode,
        has_shared_allocation: result.result.shared_allocations.iter().any(|group| {
            group.files.iter().any(|file| {
                current_platform()
                    .path_is_same_or_child(Path::new(&file.path), Path::new(&entry.path))
            })
        }),
        exclusions: result.exclusions.clone(),
        root: result.result.root.clone(),
        path: entry.path.clone(),
        expected_logical_bytes: entry.logical_bytes,
        expected_displayed_bytes: entry.bytes,
        expected_file_count: entry.file_count,
        is_directory: entry.is_directory,
    })
}

fn find_directory<'a>(
    nodes: &'a [AnalysisDirectoryNode],
    path: &str,
) -> Option<&'a AnalysisDirectoryNode> {
    for node in nodes {
        if current_platform().paths_equal(Path::new(&node.path), Path::new(path)) {
            return Some(node);
        }
        if let Some(found) = find_directory(&node.children, path) {
            return Some(found);
        }
    }
    None
}

fn find_projected_file<'a>(
    nodes: &'a [AnalysisDirectoryNode],
    path: &str,
) -> Option<&'a DirectoryEntryInfo> {
    for node in nodes {
        if let Some(file) = node
            .files
            .iter()
            .find(|file| current_platform().paths_equal(Path::new(&file.path), Path::new(path)))
        {
            return Some(file);
        }
        if let Some(file) = find_projected_file(&node.children, path) {
            return Some(file);
        }
    }
    None
}

/// Read-only open actions can target published hierarchy files and directories. The
/// complete entry resolver remains the only authority for destructive actions.
pub(super) fn resolve_open_path(scan_id: u64, selected_path: &str) -> Result<String, String> {
    let sessions = lock_sessions()?;
    let result = &sessions
        .iter()
        .find(|session| session.result.scan_id == scan_id)
        .ok_or_else(|| "the disk-analysis result session expired; scan again".to_string())?
        .result;
    result
        .entries
        .iter()
        .find(|entry| entry.path == selected_path)
        .map(|entry| entry.path.clone())
        .or_else(|| {
            find_directory(&result.directory_hierarchy, selected_path).map(|node| node.path.clone())
        })
        .or_else(|| {
            find_projected_file(&result.directory_hierarchy, selected_path)
                .map(|file| file.path.clone())
        })
        .ok_or_else(|| "the selected item is not part of the current disk analysis".to_string())
}

/// Only published parents can supply read-only remainder details.
pub(super) fn resolve_remainder_parent(
    scan_id: u64,
    path: &str,
) -> Result<AnalysisRemainderParent, String> {
    let sessions = lock_sessions()?;
    let session = sessions
        .iter()
        .find(|session| session.result.scan_id == scan_id)
        .ok_or_else(|| "the disk-analysis result session expired; scan again".to_string())?;
    let result = &session.result;
    let (path, bytes) = if current_platform().paths_equal(Path::new(&result.root), Path::new(path))
    {
        (result.root.clone(), result.total_bytes)
    } else {
        find_directory(&result.directory_hierarchy, path)
            .map(|node| (node.path.clone(), node.bytes))
            .ok_or_else(|| {
                "the remainder parent is not part of the current disk analysis".to_string()
            })?
    };
    Ok(AnalysisRemainderParent {
        scan_mode: result.scan_mode,
        path,
        bytes,
        exclusions: session.exclusions.clone(),
    })
}

/// Expires authoritative snapshots whose contents may have changed after a failed delete.
pub(super) fn invalidate_changed_path(changed_path: &Path) -> Result<(), String> {
    let mut sessions = lock_sessions()?;
    sessions.retain(|session| {
        let root = Path::new(&session.result.root);
        !current_platform().path_is_same_or_child(root, changed_path)
            && !current_platform().path_is_same_or_child(changed_path, root)
    });
    Ok(())
}

/// Hard-link ownership can move between sibling snapshots, not just ancestors.
pub(super) fn invalidate_all() -> Result<(), String> {
    lock_sessions()?.clear();
    Ok(())
}

pub(super) struct AnalysisDeleteSynchronization {
    pub(super) updated_results: Vec<AnalysisResult>,
    pub(super) invalidated_scan_ids: Vec<u64>,
    pub(super) credits: Vec<DirectoryEntryInfo>,
}

/// Reconcile the current view and affected sibling snapshots. Ancestors and
/// descendants expire because their aggregate file counts may have changed.
pub(super) fn synchronize_removed_path(
    source_scan_id: u64,
    removed_path: &Path,
) -> Result<AnalysisDeleteSynchronization, String> {
    let mut sessions = lock_sessions()?;
    let source = sessions
        .iter()
        .find(|session| session.result.scan_id == source_scan_id)
        .ok_or_else(|| "the disk-analysis result session expired; scan again".to_string())?;
    let source_exclusions = source.exclusions.clone();
    let source_mode = source.result.scan_mode;
    let mut synchronization = AnalysisDeleteSynchronization {
        updated_results: Vec::new(),
        invalidated_scan_ids: Vec::new(),
        credits: Vec::new(),
    };
    for session in sessions.iter_mut() {
        let result = &mut session.result;
        if result.scan_id != source_scan_id {
            let root = Path::new(&result.root);
            if current_platform().path_is_same_or_child(root, removed_path)
                || current_platform().path_is_same_or_child(removed_path, root)
            {
                synchronization.invalidated_scan_ids.push(result.scan_id);
                continue;
            }
        }
        let touched = result.shared_allocations.iter().any(|group| {
            group.files.iter().any(|file| {
                current_platform().path_is_same_or_child(Path::new(&file.path), removed_path)
            })
        });
        if result.scan_id != source_scan_id && !touched {
            continue;
        }
        // A deletion response belongs to one scan configuration. Returning an
        // older sibling could revive excluded content or mix byte metrics.
        if session.exclusions != source_exclusions || result.scan_mode != source_mode {
            synchronization.invalidated_scan_ids.push(result.scan_id);
            continue;
        }
        let credits = super::allocation::reconcile_shared_allocations(result, removed_path)?;
        if result.scan_id == source_scan_id {
            super::allocation::remove_direct_entry(result, removed_path);
        }
        super::allocation::apply_allocation_credits(result, &credits);
        synchronization.credits.extend(credits);
        synchronization.updated_results.push(result.clone());
    }
    sessions.retain(|session| {
        !synchronization
            .invalidated_scan_ids
            .contains(&session.result.scan_id)
    });
    synchronization
        .credits
        .sort_unstable_by(|left, right| left.path.cmp(&right.path));
    synchronization
        .credits
        .dedup_by(|left, right| left.path == right.path);
    Ok(synchronization)
}

/// Rebuilding a projection from the already reconciled index is a direct-child
/// read, not a recursive scan. It restores omitted rows and bounded chart nodes.
pub(super) fn replace_reconciled_result(
    mut result: AnalysisResult,
    scan_id: u64,
) -> Result<AnalysisResult, String> {
    let mut sessions = lock_sessions()?;
    let session = sessions
        .iter_mut()
        .find(|session| session.result.scan_id == scan_id)
        .ok_or_else(|| "the disk-analysis result session expired; scan again".to_string())?;
    result.scan_id = scan_id;
    session.result = result.clone();
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::analysis::DirectoryEntryInfo;

    fn result(path: &str) -> AnalysisResult {
        AnalysisResult {
            scan_mode: Default::default(),
            scan_id: 0,
            root: "/fixture".to_string(),
            scanned_at_ms: 1,
            total_bytes: 4,
            total_entry_count: 1,
            skipped_count: 0,
            truncated: false,
            directory_hierarchy: Vec::new(),
            shared_directories: Default::default(),
            shared_allocations: Default::default(),
            shared_entries: Default::default(),
            entries: vec![DirectoryEntryInfo {
                name: "sample.bin".to_string(),
                path: path.to_string(),
                bytes: 4,
                logical_bytes: 12,
                file_count: 1,
                is_directory: false,
                modified_at_ms: Some(7),
                content_fingerprint: None,
            }],
        }
    }

    #[test]
    fn entry_candidate_must_belong_to_the_authoritative_analysis_result() {
        let _operation_lock = crate::shared::operation::test_operation_lock();
        let result = publish_result_session(result("/fixture/sample.bin"))
            .expect("publish the analysis fixture");

        let candidate = resolve_entry_candidate(result.scan_id, "/fixture/sample.bin")
            .expect("resolve the published entry");
        assert_eq!(candidate.expected_logical_bytes, 12);
        assert_eq!(candidate.expected_displayed_bytes, 4);
        assert!(
            resolve_entry_candidate(result.scan_id, "/fixture/not-scanned.bin").is_err(),
            "a fabricated path must not cross the analysis-result boundary"
        );
        assert!(
            resolve_entry_candidate(result.scan_id.saturating_add(10_000), &candidate.path)
                .is_err(),
            "an unknown scan identifier must be rejected"
        );
    }

    #[test]
    fn hierarchy_open_targets_do_not_authorize_deletion_or_unpublished_descendants() {
        let _operation_lock = crate::shared::operation::test_operation_lock();
        let mut fixture = result("/context-fixture/sample.bin");
        fixture.root = "/context-fixture".into();
        fixture.directory_hierarchy = vec![AnalysisDirectoryNode {
            name: "A".into(),
            path: "/context-fixture/A".into(),
            bytes: 4,
            file_count: 1,
            total_entry_count: 1,
            children: vec![AnalysisDirectoryNode {
                name: "B".into(),
                path: "/context-fixture/A/B".into(),
                bytes: 4,
                file_count: 1,
                total_entry_count: 0,
                children: vec![],
                files: vec![],
            }],
            files: vec![],
        }];
        let published = publish_result_session(fixture).unwrap();
        assert_eq!(
            resolve_open_path(published.scan_id, "/context-fixture/sample.bin").unwrap(),
            "/context-fixture/sample.bin"
        );
        assert_eq!(
            resolve_open_path(published.scan_id, "/context-fixture/A/B").unwrap(),
            "/context-fixture/A/B"
        );
        assert!(resolve_entry_candidate(published.scan_id, "/context-fixture/A/B").is_err());
        for path in [
            "/context-fixture/A/B/unpublished.bin",
            "/context-fixture/A/B/../unknown",
            "/outside",
            "/context-fixture",
        ] {
            assert!(
                resolve_open_path(published.scan_id, path).is_err(),
                "unpublished target must fail: {path}"
            );
        }
        assert!(resolve_open_path(published.scan_id + 10000, "/context-fixture/A/B").is_err());
        invalidate_changed_path(Path::new("/context-fixture/A")).unwrap();
        assert!(resolve_open_path(published.scan_id, "/context-fixture/A/B").is_err());
    }

    #[cfg(windows)]
    #[test]
    fn canonical_deleted_path_updates_display_path_session() {
        let _operation_lock = crate::shared::operation::test_operation_lock();
        let root = std::env::temp_dir().join(format!(
            "mangodisk-analysis-session-{}-{}",
            std::process::id(),
            crate::filesystem::metadata::now_ms()
        ));
        std::fs::create_dir_all(&root).expect("the analysis session fixture should be created");
        let file = root.join("sample.bin");
        std::fs::write(&file, b"fixture").expect("the analysis session file should be written");
        let mut fixture = result(&file.to_string_lossy());
        fixture.root = root.to_string_lossy().into_owned();
        let published = publish_result_session(fixture).expect("publish the analysis session");
        let canonical =
            std::fs::canonicalize(&file).expect("the analysis session file should canonicalize");

        synchronize_removed_path(published.scan_id, &canonical)
            .expect("the canonical deletion should update the display session");

        assert!(resolve_entry_candidate(published.scan_id, &file.to_string_lossy()).is_err());
        std::fs::remove_dir_all(root).expect("the analysis session fixture should be removed");
    }
}
