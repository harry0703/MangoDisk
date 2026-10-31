use std::{
    collections::{HashMap, HashSet},
    sync::{Mutex, MutexGuard, OnceLock},
};

use mangodisk_platform::{current_platform, Platform};

use super::{
    DuplicateEntryDeletePolicy, DuplicateFilesResult, DuplicateGroupKind, DuplicateGroupPage,
};
use crate::filesystem::PermanentDeleteCandidate;

pub(super) const DUPLICATE_RESULT_PAGE_SIZE: usize = 40;
const DUPLICATE_RESULT_PAGE_LIMIT: usize = 100;

static DUPLICATE_RESULT_SESSION: OnceLock<Mutex<Option<DuplicateResultSession>>> = OnceLock::new();

struct DuplicateResultSession {
    result: DuplicateFilesResult,
}

#[derive(Debug)]
pub(super) struct ValidatedDuplicateDeleteCandidate {
    pub(super) candidate: PermanentDeleteCandidate,
    pub(super) kind: DuplicateGroupKind,
    pub(super) expected_allocated_bytes: u64,
    pub(super) expected_file_count: u64,
    pub(super) scan_root: String,
    pub(super) protected_roots: Vec<String>,
}

fn result_session() -> &'static Mutex<Option<DuplicateResultSession>> {
    DUPLICATE_RESULT_SESSION.get_or_init(|| Mutex::new(None))
}

fn lock_result_session() -> Result<MutexGuard<'static, Option<DuplicateResultSession>>, String> {
    result_session()
        .lock()
        .map_err(|_| "the duplicate-file result session is unavailable".to_string())
}

pub(super) fn clear_result_session() -> Result<(), String> {
    *lock_result_session()? = None;
    Ok(())
}

pub(super) fn publish_result_session(
    mut result: DuplicateFilesResult,
) -> Result<DuplicateFilesResult, String> {
    let full_groups = std::mem::take(&mut result.groups);
    result.groups = full_groups
        .iter()
        .take(DUPLICATE_RESULT_PAGE_SIZE)
        .cloned()
        .collect();
    let mut session_result = result.clone();
    session_result.groups = full_groups;
    *lock_result_session()? = Some(DuplicateResultSession {
        result: session_result,
    });
    Ok(result)
}

pub(super) fn result_page(
    scan_id: u64,
    offset: u64,
    limit: u64,
) -> Result<DuplicateGroupPage, String> {
    if limit == 0 {
        return Err("the duplicate-file page size must be greater than zero".to_string());
    }
    let offset = usize::try_from(offset)
        .map_err(|_| "the duplicate-file page offset is out of range".to_string())?;
    let limit = usize::try_from(limit)
        .unwrap_or(DUPLICATE_RESULT_PAGE_LIMIT)
        .min(DUPLICATE_RESULT_PAGE_LIMIT);
    let session = lock_result_session()?;
    let result = &session
        .as_ref()
        .filter(|session| session.result.scan_id == scan_id)
        .ok_or_else(|| "the duplicate-file result session expired; scan again".to_string())?
        .result;
    if offset > result.groups.len() {
        return Err("the duplicate-file page offset exceeds the current result".to_string());
    }
    let end = offset.saturating_add(limit).min(result.groups.len());
    let groups = result.groups[offset..end].to_vec();
    let next_offset = (end < result.groups.len()).then(|| u64::try_from(end).unwrap_or(u64::MAX));
    Ok(DuplicateGroupPage {
        scan_id,
        offset: u64::try_from(offset).unwrap_or(u64::MAX),
        next_offset,
        total_count: u64::try_from(result.groups.len()).unwrap_or(u64::MAX),
        groups,
    })
}

pub(super) fn resolve_open_target(scan_id: u64, selected_path: &str) -> Result<String, String> {
    let session = lock_result_session()?;
    let result = &session
        .as_ref()
        .filter(|session| session.result.scan_id == scan_id)
        .ok_or_else(|| "the duplicate-file result session expired; scan again".to_string())?
        .result;
    result
        .groups
        .iter()
        .flat_map(|group| &group.entries)
        .find(|entry| entry.path == selected_path)
        .map(|entry| entry.path.clone())
        .ok_or_else(|| {
            "the selected item is not part of the current duplicate-file scan".to_string()
        })
}

/// Validates permanent-delete input against the authoritative duplicate scan session.
///
/// The WebView owns selection state, but it is not a safety boundary. Permanent deletion must
/// therefore reject stale or fabricated entries and must preserve at least one file in every
/// affected duplicate group.
pub(super) fn validate_permanent_delete_candidates(
    scan_id: u64,
    candidates: Vec<PermanentDeleteCandidate>,
) -> Result<Vec<ValidatedDuplicateDeleteCandidate>, String> {
    if candidates.is_empty() {
        return Err("at least one duplicate file must be selected".to_string());
    }

    let session = lock_result_session()?;
    let result = &session
        .as_ref()
        .filter(|session| session.result.scan_id == scan_id)
        .ok_or_else(|| "the duplicate-file result session expired; scan again".to_string())?
        .result;
    let mut candidates_by_path = HashMap::with_capacity(candidates.len());
    for candidate in &candidates {
        if candidates_by_path
            .insert(candidate.path.as_str(), candidate)
            .is_some()
        {
            return Err("a duplicate file was selected more than once".to_string());
        }
    }

    let mut matched_paths = HashSet::with_capacity(candidates.len());
    let mut validated = Vec::with_capacity(candidates.len());
    for group in &result.groups {
        let selected_count = group
            .entries
            .iter()
            .filter(|entry| candidates_by_path.contains_key(entry.path.as_str()))
            .count();
        if selected_count == group.entries.len() && selected_count > 0 {
            return Err("at least one file must remain in every duplicate group".to_string());
        }
        for entry in &group.entries {
            let Some(candidate) = candidates_by_path.get(entry.path.as_str()).copied() else {
                continue;
            };
            if candidate.expected_bytes != entry.bytes
                || candidate.expected_modified_at_ms != entry.modified_at_ms
            {
                return Err("a duplicate file no longer matches the scan result".to_string());
            }
            let path = std::path::Path::new(&entry.path);
            let intersects_protected_root = result.protected_roots.iter().any(|root| {
                let root = std::path::Path::new(root);
                current_platform().path_is_same_or_child(path, root)
                    || (group.kind == DuplicateGroupKind::Directory
                        && current_platform().path_is_same_or_child(root, path))
            });
            if entry.delete_policy == DuplicateEntryDeletePolicy::Protected
                || intersects_protected_root
            {
                return Err("a protected duplicate item cannot be deleted".to_string());
            }
            matched_paths.insert(entry.path.as_str());
            let scan_root = result
                .roots
                .iter()
                .filter(|root| {
                    current_platform().path_is_same_or_child(
                        std::path::Path::new(&entry.path),
                        std::path::Path::new(root),
                    )
                })
                .max_by_key(|root| std::path::Path::new(root).components().count())
                .cloned()
                .ok_or_else(|| "a duplicate item is outside the current scan roots".to_string())?;
            validated.push(ValidatedDuplicateDeleteCandidate {
                candidate: candidate.clone(),
                kind: group.kind,
                expected_allocated_bytes: entry.allocated_bytes,
                expected_file_count: group.file_count_per_entry,
                scan_root,
                protected_roots: result.protected_roots.clone(),
            });
        }
    }
    if matched_paths.len() != candidates.len() {
        return Err("a selected file is not part of the current duplicate scan".to_string());
    }

    Ok(validated)
}

pub(super) fn synchronize_result_session(
    scan_id: u64,
    removed_paths: Vec<String>,
) -> Result<DuplicateFilesResult, String> {
    let removed_paths = removed_paths.into_iter().collect::<HashSet<_>>();
    let mut session = lock_result_session()?;
    let result = &mut session
        .as_mut()
        .filter(|session| session.result.scan_id == scan_id)
        .ok_or_else(|| "the duplicate-file result session expired; scan again".to_string())?
        .result;
    result.groups.retain_mut(|group| {
        group
            .entries
            .retain(|entry| !removed_paths.contains(&entry.path));
        let remains_duplicate = group.entries.len() > 1;
        if remains_duplicate {
            group.refresh_reclaimable_bytes();
        }
        remains_duplicate
    });
    // Page offsets refer to the surviving scan order. Reordering a group after
    // deletion can move an unseen group behind the caller's consumed prefix.
    result.duplicate_file_count = result
        .groups
        .iter()
        .map(|group| {
            u64::try_from(group.entries.len())
                .unwrap_or(u64::MAX)
                .saturating_mul(group.file_count_per_entry)
        })
        .sum();
    result.total_duplicate_bytes = result
        .groups
        .iter()
        .map(|group| group.total_allocated_bytes())
        .fold(0_u64, u64::saturating_add);
    result.reclaimable_bytes = result
        .groups
        .iter()
        .map(|group| group.reclaimable_bytes)
        .fold(0_u64, u64::saturating_add);
    result.total_group_count = u64::try_from(result.groups.len()).unwrap_or(u64::MAX);
    result.returned_group_count = u64::try_from(result.groups.len()).unwrap_or(u64::MAX);
    result.truncated = result.returned_group_count < result.total_group_count;

    let mut response = result.clone();
    response.groups.truncate(DUPLICATE_RESULT_PAGE_SIZE);
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared::operation::test_operation_lock;
    use crate::storage::duplicates::{DuplicateFileEntry, DuplicateGroup};

    fn result() -> DuplicateFilesResult {
        let entries = ["a", "b", "c"]
            .into_iter()
            .map(|name| DuplicateFileEntry {
                name: format!("{name}.bin"),
                path: format!("/fixture/{name}.bin"),
                parent_path: "/fixture".to_string(),
                bytes: 10,
                allocated_bytes: 10,
                modified_at_ms: Some(1),
                delete_policy: DuplicateEntryDeletePolicy::Cleanable,
            })
            .collect::<Vec<_>>();
        DuplicateFilesResult {
            scan_id: 42,
            roots: vec!["/fixture".to_string()],
            protected_roots: Vec::new(),
            scanned_at_ms: 1,
            scanned_file_count: 3,
            skipped_count: 0,
            duplicate_file_count: 3,
            total_duplicate_bytes: 30,
            reclaimable_bytes: 20,
            total_group_count: 1,
            returned_group_count: 1,
            truncated: false,
            groups: vec![DuplicateGroup {
                id: "group".to_string(),
                hash: "hash".to_string(),
                kind: DuplicateGroupKind::File,
                bytes_per_file: 10,
                file_count_per_entry: 1,
                reclaimable_bytes: 20,
                entries,
            }],
        }
    }

    fn candidate(path: &str) -> PermanentDeleteCandidate {
        PermanentDeleteCandidate {
            path: path.to_string(),
            expected_bytes: 10,
            expected_modified_at_ms: Some(1),
        }
    }

    #[test]
    fn deletion_preserves_the_next_page_after_a_loaded_group_shrinks_or_disappears() {
        let _operation_lock = test_operation_lock();
        for removed_count in [1, 2] {
            let mut fixture = result();
            let template = fixture.groups[0].clone();
            fixture.groups = (0..=DUPLICATE_RESULT_PAGE_SIZE)
                .map(|index| {
                    let mut group = template.clone();
                    group.id = format!("group-{index:02}");
                    group.hash = group.id.clone();
                    if index != 0 {
                        group.entries.truncate(2);
                    }
                    for (copy, entry) in group.entries.iter_mut().enumerate() {
                        entry.path = format!("/fixture/{index}-{copy}.bin");
                        entry.bytes = if index == 0 { 100 } else { 150 };
                        entry.allocated_bytes = entry.bytes;
                    }
                    group.refresh_reclaimable_bytes();
                    group
                })
                .collect();
            fixture.total_group_count = fixture.groups.len() as u64;
            fixture.returned_group_count = fixture.total_group_count;
            fixture.scanned_file_count = fixture
                .groups
                .iter()
                .map(|group| group.entries.len() as u64)
                .sum();
            fixture.duplicate_file_count = fixture.scanned_file_count;
            fixture.total_duplicate_bytes = fixture
                .groups
                .iter()
                .map(DuplicateGroup::total_allocated_bytes)
                .sum();
            fixture.reclaimable_bytes = fixture
                .groups
                .iter()
                .map(|group| group.reclaimable_bytes)
                .sum();
            let first = publish_result_session(fixture).unwrap();
            assert_eq!(first.groups.len(), DUPLICATE_RESULT_PAGE_SIZE);
            let removed_paths = first.groups[0].entries[..removed_count]
                .iter()
                .map(|entry| entry.path.clone())
                .collect();
            synchronize_result_session(first.scan_id, removed_paths).unwrap();

            let resolved_groups = usize::from(removed_count == 2);
            let offset = first.groups.len() - resolved_groups;
            let next = result_page(first.scan_id, offset as u64, 40).unwrap();
            clear_result_session().unwrap();
            assert_eq!(next.groups.len(), 1);
            assert_eq!(next.groups[0].id, "group-40");
            assert_eq!(next.next_offset, None);
        }
    }

    #[test]
    fn permanent_delete_validation_preserves_one_file_per_group() {
        let _operation_lock = test_operation_lock();
        publish_result_session(result()).expect("publish the duplicate fixture");

        let allowed = validate_permanent_delete_candidates(
            42,
            vec![candidate("/fixture/a.bin"), candidate("/fixture/b.bin")],
        )
        .expect("deleting duplicate copies while keeping one should be allowed");
        assert_eq!(allowed.len(), 2);
        assert_eq!(allowed[0].kind, DuplicateGroupKind::File);

        let error = validate_permanent_delete_candidates(
            42,
            vec![
                candidate("/fixture/a.bin"),
                candidate("/fixture/b.bin"),
                candidate("/fixture/c.bin"),
            ],
        )
        .expect_err("deleting every file in a duplicate group must be rejected");
        assert!(error.contains("at least one file must remain"));
        clear_result_session().expect("clear the duplicate fixture");
    }

    #[test]
    fn permanent_delete_validation_rejects_entries_outside_the_scan() {
        let _operation_lock = test_operation_lock();
        publish_result_session(result()).expect("publish the duplicate fixture");

        let error =
            validate_permanent_delete_candidates(42, vec![candidate("/fixture/not-scanned.bin")])
                .expect_err("an entry outside the scan must be rejected");
        assert!(error.contains("not part of the current duplicate scan"));
        clear_result_session().expect("clear the duplicate fixture");
    }

    #[test]
    fn permanent_delete_validation_rejects_authoritative_entries_outside_the_scan_root() {
        let _operation_lock = test_operation_lock();
        let mut fixture = result();
        fixture.groups[0].entries[0].path = "/outside/a.bin".to_string();
        fixture.groups[0].entries[0].parent_path = "/outside".to_string();
        publish_result_session(fixture).expect("publish the out-of-scope duplicate fixture");

        let error = validate_permanent_delete_candidates(42, vec![candidate("/outside/a.bin")])
            .expect_err("an authoritative entry outside every scan root must be rejected");
        assert!(error.contains("outside the current scan roots"));
        clear_result_session().expect("clear the out-of-scope duplicate fixture");
    }

    #[test]
    fn permanent_delete_validation_rejects_directories_containing_protected_roots() {
        let _operation_lock = test_operation_lock();
        let mut fixture = result();
        fixture.protected_roots = vec!["/fixture/work/important".to_string()];
        fixture.groups[0].kind = DuplicateGroupKind::Directory;
        fixture.groups[0].entries[0].name = "work".to_string();
        fixture.groups[0].entries[0].path = "/fixture/work".to_string();
        fixture.groups[0].entries[0].parent_path = "/fixture".to_string();
        fixture.groups[0].entries[1].name = "backup".to_string();
        fixture.groups[0].entries[1].path = "/fixture/backup".to_string();
        fixture.groups[0].entries[1].parent_path = "/fixture".to_string();
        fixture.groups[0].entries.truncate(2);
        publish_result_session(fixture).expect("publish the protected directory fixture");

        let error = validate_permanent_delete_candidates(42, vec![candidate("/fixture/work")])
            .expect_err("a directory containing a protected root must not be deleted");
        assert!(error.contains("protected duplicate item"));
        clear_result_session().expect("clear the protected directory fixture");
    }

    #[test]
    fn duplicate_open_target_is_bound_to_the_scan_result() {
        let _operation_lock = test_operation_lock();
        publish_result_session(result()).expect("publish the duplicate fixture");

        let target = resolve_open_target(42, "/fixture/a.bin")
            .expect("resolve the published duplicate file");
        assert_eq!(target, "/fixture/a.bin");
        assert!(resolve_open_target(42, "/fixture/not-scanned.bin").is_err());
        assert!(resolve_open_target(41, "/fixture/a.bin").is_err());
        clear_result_session().expect("clear the duplicate fixture");
    }

    #[test]
    fn result_synchronization_recomputes_physical_duplicate_totals() {
        let _operation_lock = test_operation_lock();
        let mut fixture = result();
        for (entry, allocated_bytes) in fixture.groups[0].entries.iter_mut().zip([4_u64, 8, 12]) {
            entry.allocated_bytes = allocated_bytes;
        }
        fixture.total_duplicate_bytes = 24;
        fixture.reclaimable_bytes = 20;
        fixture.groups[0].reclaimable_bytes = 20;
        publish_result_session(fixture).expect("publish the physical-size duplicate fixture");

        let updated = synchronize_result_session(42, vec!["/fixture/c.bin".to_string()])
            .expect("synchronize the removed duplicate");

        assert_eq!(updated.total_duplicate_bytes, 12);
        assert_eq!(updated.reclaimable_bytes, 8);
        clear_result_session().expect("clear the physical-size duplicate fixture");
    }
}
