use std::{collections::BTreeMap, fs, path::Path};

use super::*;

fn analyze(root: &Path) -> AnalysisResult {
    AnalysisService::analyze_with_progress(Some(root.to_string_lossy().into_owned()), true, |_| {})
        .unwrap()
}

fn projection(result: &AnalysisResult) -> BTreeMap<String, (u64, u64, u64)> {
    fn visit(
        nodes: &[super::super::AnalysisDirectoryNode],
        rows: &mut BTreeMap<String, (u64, u64, u64)>,
    ) {
        for node in nodes {
            rows.insert(
                node.path.clone(),
                (node.bytes, node.file_count, node.total_entry_count),
            );
            for file in &node.files {
                rows.insert(file.path.clone(), (file.bytes, file.file_count, 0));
            }
            visit(&node.children, rows);
        }
    }
    let mut rows = BTreeMap::new();
    visit(&result.directory_hierarchy, &mut rows);
    rows
}

#[cfg(unix)]
#[test]
fn evicted_index_reconciliation_restores_previously_zero_charge_nested_chart_nodes() {
    let _operation_lock = crate::shared::operation::test_operation_lock();
    cache::clear_all().unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(fixture.path()).unwrap();
    fs::create_dir_all(root.join("b/deep/nested")).unwrap();
    fs::write(root.join("a.bin"), vec![1; 8192]).unwrap();
    fs::hard_link(root.join("a.bin"), root.join("b/deep/nested/alias.bin")).unwrap();
    fs::write(root.join("b/ordinary.bin"), vec![2; 4096]).unwrap();
    let initial = analyze(&root);
    cache::clear_all().unwrap();
    let deleted = AnalysisService::delete_entry_permanently(
        initial.scan_id,
        root.join("a.bin").to_string_lossy().into_owned(),
    )
    .unwrap();
    assert!(!deleted.requires_rescan);
    let updated = deleted
        .updated_results
        .iter()
        .find(|result| result.scan_id == initial.scan_id)
        .unwrap();
    let fresh = analyze(&root);
    assert_eq!(updated.total_bytes, fresh.total_bytes);
    assert_eq!(projection(updated), projection(&fresh));
}

#[cfg(unix)]
#[test]
fn partial_linked_folder_deletion_expires_shared_sibling_sessions() {
    use std::os::unix::fs::PermissionsExt;
    let _operation_lock = crate::shared::operation::test_operation_lock();
    cache::clear_all().unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(fixture.path()).unwrap();
    let locked = root.join("a/locked");
    fs::create_dir_all(&locked).unwrap();
    fs::create_dir(root.join("b")).unwrap();
    fs::write(root.join("a/owner.bin"), vec![1; 8192]).unwrap();
    fs::hard_link(root.join("a/owner.bin"), root.join("b/alias.bin")).unwrap();
    fs::write(locked.join("retained.bin"), [1]).unwrap();
    let initial = analyze(&root);
    let sibling = AnalysisService::analyze_with_progress(
        Some(root.join("b").to_string_lossy().into_owned()),
        false,
        |_| {},
    )
    .unwrap();
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o0)).unwrap();
    let deleted = AnalysisService::delete_entry_permanently(
        initial.scan_id,
        root.join("a").to_string_lossy().into_owned(),
    );
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o700)).unwrap();
    let error = deleted.expect_err("unreadable descendants must stop deletion");
    assert_eq!(
        error.mutation_state(),
        mangodisk_platform::PlatformMutationState::MayHaveChanged
    );
    assert!(
        AnalysisService::resolve_open_target(sibling.scan_id, sibling.entries[0].path.clone())
            .is_err(),
        "shared sibling snapshots must not survive an uncertain partial deletion"
    );
}

#[test]
fn every_linked_folder_deletion_order_matches_an_independent_scan_with_and_without_index() {
    let _operation_lock = crate::shared::operation::test_operation_lock();
    let mut scenarios = 0;
    for evict_index in [false, true] {
        for a in 0..4 {
            for b in 0..4 {
                for c in 0..4 {
                    for d in 0..4 {
                        let order = [a, b, c, d];
                        if order
                            .iter()
                            .copied()
                            .collect::<std::collections::BTreeSet<_>>()
                            .len()
                            != 4
                        {
                            continue;
                        }
                        cache::clear_all().unwrap();
                        let fixture = tempfile::tempdir().unwrap();
                        let root = fs::canonicalize(fixture.path()).unwrap();
                        let folders = ["a owner", "b \u{4e2d}\u{6587}", "c", "d"];
                        for name in folders {
                            fs::create_dir_all(root.join(name).join("deep/nested")).unwrap();
                        }
                        for (group, members) in [vec![0, 1, 2, 3], vec![0, 2], vec![1, 3]]
                            .iter()
                            .enumerate()
                        {
                            let owner = root
                                .join(folders[members[0]])
                                .join(format!("deep/nested/group{group}.bin"));
                            fs::write(&owner, vec![group as u8; (group + 1) * 8192]).unwrap();
                            for member in members.iter().skip(1) {
                                fs::hard_link(
                                    &owner,
                                    root.join(folders[*member])
                                        .join(format!("deep/nested/group{group}.bin")),
                                )
                                .unwrap();
                            }
                        }
                        // Empty files and directories must not be confused with uncharged aliases.
                        fs::write(root.join("d/ordinary.bin"), vec![3; 4096]).unwrap();
                        fs::write(root.join("c/empty.bin"), []).unwrap();
                        fs::create_dir(root.join("b \u{4e2d}\u{6587}/empty")).unwrap();
                        let initial = analyze(&root);
                        for index in order {
                            if evict_index {
                                cache::clear_all().unwrap();
                            }
                            // Destructive actions use the published path, not an OS canonicalization alias.
                            let selected_path = initial
                                .entries
                                .iter()
                                .find(|entry| entry.name == folders[index])
                                .expect("every fixture folder must be published")
                                .path
                                .clone();
                            let response = AnalysisService::delete_entry_permanently(
                                initial.scan_id,
                                selected_path.clone(),
                            )
                            .unwrap_or_else(|error| {
                                panic!(
                                    "order={order:?}, evicted={evict_index}, selected={selected_path:?}, error={error}"
                                )
                            });
                            assert!(
                                !response.requires_rescan,
                                "order={order:?}, evicted={evict_index}"
                            );
                            assert!(!response.invalidated_scan_ids.contains(&initial.scan_id));
                            let updated = response
                                .updated_results
                                .iter()
                                .find(|result| result.scan_id == initial.scan_id)
                                .unwrap();
                            // Diagnostics returns an independent result without replacing session authority.
                            let (fresh, _) = AnalysisService::analyze_with_diagnostics(
                                Some(root.to_string_lossy().into_owned()),
                                true,
                                |_| {},
                            )
                            .unwrap();
                            assert_eq!(
                                updated.total_bytes, fresh.total_bytes,
                                "order={order:?}, evicted={evict_index}"
                            );
                            assert_eq!(updated.total_entry_count, fresh.total_entry_count);
                            let rows = |result: &AnalysisResult| {
                                result
                                    .entries
                                    .iter()
                                    .map(|row| {
                                        (
                                            row.path.clone(),
                                            (row.bytes, row.file_count, row.is_directory),
                                        )
                                    })
                                    .collect::<BTreeMap<_, _>>()
                            };
                            assert_eq!(rows(updated), rows(&fresh));
                            assert_eq!(
                                projection(updated),
                                projection(&fresh),
                                "order={order:?}, evicted={evict_index}"
                            );
                        }
                        scenarios += 1;
                    }
                }
            }
        }
    }
    assert_eq!(scenarios, 48);
}

#[cfg(unix)]
#[test]
fn missing_or_symlink_replaced_aliases_do_not_receive_allocation() {
    use std::os::unix::fs::symlink;
    let _operation_lock = crate::shared::operation::test_operation_lock();
    for replacement in [false, true] {
        cache::clear_all().unwrap();
        let fixture = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(fixture.path()).unwrap();
        fs::write(root.join("a.bin"), vec![1; 8192]).unwrap();
        fs::hard_link(root.join("a.bin"), root.join("b.bin")).unwrap();
        fs::hard_link(root.join("a.bin"), root.join("c.bin")).unwrap();
        let initial = analyze(&root);
        fs::remove_file(root.join("b.bin")).unwrap();
        if replacement {
            symlink(root.join("c.bin"), root.join("b.bin")).unwrap();
        }
        cache::clear_all().unwrap();
        let response = AnalysisService::delete_entry_permanently(
            initial.scan_id,
            root.join("a.bin").to_string_lossy().into_owned(),
        )
        .unwrap();
        assert!(!response.requires_rescan);
        let updated = response
            .updated_results
            .iter()
            .find(|result| result.scan_id == initial.scan_id)
            .unwrap();
        assert_eq!(updated.total_bytes, initial.total_bytes);
        assert_eq!(
            updated
                .entries
                .iter()
                .find(|row| row.name == "c.bin")
                .unwrap()
                .bytes,
            initial.total_bytes
        );
        assert_eq!(
            updated
                .entries
                .iter()
                .find(|row| row.name == "b.bin")
                .unwrap()
                .bytes,
            0
        );
    }
}

#[cfg(unix)]
#[test]
fn survivor_outside_scan_scope_and_zero_length_links_need_no_rescan() {
    let _operation_lock = crate::shared::operation::test_operation_lock();
    for bytes in [0, 8192] {
        cache::clear_all().unwrap();
        let fixture = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(fixture.path()).unwrap();
        fs::write(root.join("owner.bin"), vec![1; bytes]).unwrap();
        fs::hard_link(root.join("owner.bin"), outside.path().join("alias.bin")).unwrap();
        let initial = analyze(&root);
        cache::clear_all().unwrap();
        let response = AnalysisService::delete_entry_permanently(
            initial.scan_id,
            root.join("owner.bin").to_string_lossy().into_owned(),
        )
        .unwrap();
        assert!(!response.requires_rescan);
        let updated = response
            .updated_results
            .iter()
            .find(|result| result.scan_id == initial.scan_id)
            .unwrap();
        assert_eq!(updated.total_bytes, 0);
        assert_eq!(updated.total_entry_count, 0);
        assert!(updated.entries.is_empty());
        assert_eq!(
            fs::metadata(outside.path().join("alias.bin"))
                .unwrap()
                .len(),
            bytes as u64
        );
    }
}

#[cfg(unix)]
#[test]
fn deeply_promoted_links_preserve_counts_at_the_chart_depth_boundary() {
    let _operation_lock = crate::shared::operation::test_operation_lock();
    for depth in [5, 6, 7, 10] {
        cache::clear_all().unwrap();
        let fixture = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(fixture.path()).unwrap();
        let mut leaf = root.join("b");
        for _ in 1..depth {
            leaf.push("nested");
        }
        fs::create_dir_all(&leaf).unwrap();
        fs::write(root.join("a.bin"), vec![1; 8192]).unwrap();
        fs::hard_link(root.join("a.bin"), leaf.join("alias.bin")).unwrap();
        let initial = analyze(&root);
        cache::clear_all().unwrap();
        let response = AnalysisService::delete_entry_permanently(
            initial.scan_id,
            root.join("a.bin").to_string_lossy().into_owned(),
        )
        .unwrap();
        let updated = response
            .updated_results
            .iter()
            .find(|result| result.scan_id == initial.scan_id)
            .unwrap();
        let (fresh, _) = AnalysisService::analyze_with_diagnostics(
            Some(root.to_string_lossy().into_owned()),
            true,
            |_| {},
        )
        .unwrap();
        assert_eq!(projection(updated), projection(&fresh), "depth={depth}");
        assert!(!response.requires_rescan);
    }
}

#[cfg(unix)]
#[test]
fn promoted_aliases_share_the_chart_sibling_budget_with_ordinary_files() {
    let _operation_lock = crate::shared::operation::test_operation_lock();
    cache::clear_all().unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(fixture.path()).unwrap();
    fs::create_dir(root.join("a")).unwrap();
    fs::create_dir(root.join("b")).unwrap();
    for index in 0..80 {
        let owner = root.join(format!("a/{index:03}.bin"));
        fs::write(&owner, vec![1; 16384]).unwrap();
        fs::hard_link(owner, root.join(format!("b/{index:03}.bin"))).unwrap();
        fs::write(
            root.join(format!("b/ordinary{index:03}.bin")),
            vec![2; 4096],
        )
        .unwrap();
    }
    let initial = analyze(&root);
    cache::clear_all().unwrap();
    let response = AnalysisService::delete_entry_permanently(
        initial.scan_id,
        root.join("a").to_string_lossy().into_owned(),
    )
    .unwrap();
    let updated = response
        .updated_results
        .iter()
        .find(|result| result.scan_id == initial.scan_id)
        .unwrap();
    let (fresh, _) = AnalysisService::analyze_with_diagnostics(
        Some(root.to_string_lossy().into_owned()),
        true,
        |_| {},
    )
    .unwrap();
    assert_eq!(projection(updated), projection(&fresh));
    let node = &updated.directory_hierarchy[0];
    assert_eq!(node.total_entry_count, 160);
    assert_eq!(node.children.len() + node.files.len(), 64);
    assert!(!response.requires_rescan);
}

#[cfg(unix)]
#[test]
fn inaccessible_surviving_alias_fails_closed_and_expires_every_session() {
    use std::os::unix::fs::PermissionsExt;
    let _operation_lock = crate::shared::operation::test_operation_lock();
    cache::clear_all().unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(fixture.path()).unwrap();
    fs::create_dir(root.join("b")).unwrap();
    fs::write(root.join("a.bin"), vec![1; 8192]).unwrap();
    fs::hard_link(root.join("a.bin"), root.join("b/alias.bin")).unwrap();
    let initial = analyze(&root);
    let sibling = AnalysisService::analyze_with_progress(
        Some(root.join("b").to_string_lossy().into_owned()),
        false,
        |_| {},
    )
    .unwrap();
    fs::set_permissions(root.join("b"), fs::Permissions::from_mode(0o0)).unwrap();
    let response = AnalysisService::delete_entry_permanently(
        initial.scan_id,
        root.join("a.bin").to_string_lossy().into_owned(),
    );
    fs::set_permissions(root.join("b"), fs::Permissions::from_mode(0o700)).unwrap();
    let error = response.expect_err("identity verification must fail closed on permission errors");
    assert_eq!(
        error.mutation_state(),
        mangodisk_platform::PlatformMutationState::MayHaveChanged
    );
    assert!(!root.join("a.bin").exists());
    assert!(
        AnalysisService::resolve_open_target(sibling.scan_id, sibling.entries[0].path.clone())
            .is_err()
    );
    assert!(cache::analysis_result(&root).unwrap().is_none());
}

#[cfg(unix)]
#[test]
fn cached_and_independently_refreshed_siblings_are_not_double_charged() {
    let _operation_lock = crate::shared::operation::test_operation_lock();
    for refresh_sibling in [false, true] {
        cache::clear_all().unwrap();
        let fixture = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(fixture.path()).unwrap();
        fs::create_dir(root.join("a")).unwrap();
        fs::create_dir(root.join("b")).unwrap();
        fs::write(root.join("a/owner.bin"), vec![1; 8192]).unwrap();
        fs::hard_link(root.join("a/owner.bin"), root.join("b/alias.bin")).unwrap();
        let parent = analyze(&root);
        let owner = AnalysisService::analyze_with_progress(
            Some(root.join("a").to_string_lossy().into_owned()),
            false,
            |_| {},
        )
        .unwrap();
        let sibling = AnalysisService::analyze_with_progress(
            Some(root.join("b").to_string_lossy().into_owned()),
            refresh_sibling,
            |_| {},
        )
        .unwrap();
        assert_eq!(
            sibling.total_bytes,
            if refresh_sibling {
                parent.total_bytes
            } else {
                0
            }
        );
        let response =
            AnalysisService::delete_entry_permanently(owner.scan_id, owner.entries[0].path.clone())
                .unwrap();
        assert!(!response.requires_rescan);
        assert!(response.invalidated_scan_ids.contains(&parent.scan_id));
        assert!(!response.invalidated_scan_ids.contains(&sibling.scan_id));
        let cached = cache::analysis_result(&root.join("b")).unwrap().unwrap();
        assert_eq!(
            cached.total_bytes, parent.total_bytes,
            "refresh_sibling={refresh_sibling}"
        );
        assert_eq!(cached.entries[0].bytes, parent.total_bytes);
        let candidate = resolve_entry_candidate(sibling.scan_id, &sibling.entries[0].path).unwrap();
        assert_eq!(candidate.expected_displayed_bytes, parent.total_bytes);
        let last = AnalysisService::delete_entry_permanently(
            sibling.scan_id,
            sibling.entries[0].path.clone(),
        )
        .unwrap();
        assert!(!last.requires_rescan);
        assert_eq!(
            last.updated_results
                .iter()
                .find(|result| result.scan_id == sibling.scan_id)
                .unwrap()
                .total_bytes,
            0
        );
    }
}

#[cfg(unix)]
#[test]
fn changed_exclusions_do_not_restore_older_shared_sibling_authority() {
    let _operation_lock = crate::shared::operation::test_operation_lock();
    for exclude_by_name in [false, true] {
        cache::clear_all().unwrap();
        let fixture = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(fixture.path()).unwrap();
        fs::create_dir_all(root.join("b/excluded-items")).unwrap();
        fs::write(root.join("a.bin"), vec![1; 8192]).unwrap();
        fs::hard_link(root.join("a.bin"), root.join("b/alias.bin")).unwrap();
        fs::write(root.join("b/excluded-items/keep.bin"), vec![2; 4096]).unwrap();
        analyze(&root);
        let sibling = AnalysisService::analyze_with_progress(
            Some(root.join("b").to_string_lossy().into_owned()),
            false,
            |_| {},
        )
        .unwrap();
        let exclusions = if exclude_by_name {
            ScanExclusionOptions {
                paths: Vec::new(),
                names: vec![mangodisk_platform::ScanNameExclusion {
                    name: "excluded-items".into(),
                    kind: mangodisk_platform::ExcludedNameKind::Folder,
                }],
            }
        } else {
            ScanExclusionOptions::from(vec![root
                .join("b/excluded-items")
                .to_string_lossy()
                .into_owned()])
        };
        let current = AnalysisService::analyze_with_exclusions_progress(
            Some(root.to_string_lossy().into_owned()),
            true,
            exclusions,
            |_| {},
        )
        .unwrap();
        cache::clear_all().unwrap();
        let response = AnalysisService::delete_entry_permanently(
            current.scan_id,
            root.join("a.bin").to_string_lossy().into_owned(),
        )
        .unwrap();
        assert!(!response.requires_rescan);
        assert!(
            response.invalidated_scan_ids.contains(&sibling.scan_id),
            "a sibling from older exclusions must expire rather than reenter the current UI"
        );
        assert!(response
            .updated_results
            .iter()
            .all(|result| result.scan_id != sibling.scan_id));
        assert!(AnalysisService::delete_entry_permanently(
            sibling.scan_id,
            root.join("b/excluded-items").to_string_lossy().into_owned()
        )
        .is_err());
        assert!(AnalysisService::resolve_open_target(
            sibling.scan_id,
            root.join("b/excluded-items").to_string_lossy().into_owned()
        )
        .is_err());
        assert!(root.join("b/excluded-items/keep.bin").exists());
    }
}

#[cfg(unix)]
#[test]
fn changed_hard_link_content_keeps_transferred_snapshot_totals_consistent() {
    let _operation_lock = crate::shared::operation::test_operation_lock();
    for evict_index in [false, true] {
        cache::clear_all().unwrap();
        let fixture = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(fixture.path()).unwrap();
        fs::write(root.join("a.bin"), vec![1; 8192]).unwrap();
        fs::hard_link(root.join("a.bin"), root.join("b.bin")).unwrap();
        let initial = analyze(&root);
        fs::write(root.join("b.bin"), vec![2; 65536]).unwrap();
        if evict_index {
            cache::clear_all().unwrap();
        }
        let response = AnalysisService::delete_entry_permanently(
            initial.scan_id,
            root.join("a.bin").to_string_lossy().into_owned(),
        )
        .unwrap();
        assert!(!response.requires_rescan);
        let updated = response
            .updated_results
            .iter()
            .find(|result| result.scan_id == initial.scan_id)
            .unwrap();
        assert_eq!(updated.entries.iter().map(|entry| entry.bytes).sum::<u64>(), updated.total_bytes,
            "transferred scan bytes must not be mixed with live file measurements, evicted={evict_index}");
        assert_eq!(updated.total_bytes, initial.total_bytes);
        assert_eq!(fs::metadata(root.join("b.bin")).unwrap().len(), 65536);
        let (fresh, _) = AnalysisService::analyze_with_diagnostics(
            Some(root.to_string_lossy().into_owned()),
            true,
            |_| {},
        )
        .unwrap();
        assert!(
            fresh.total_bytes > updated.total_bytes,
            "explicit refresh must expose the new live allocation"
        );
        let last = AnalysisService::delete_entry_permanently(
            initial.scan_id,
            root.join("b.bin").to_string_lossy().into_owned(),
        )
        .unwrap();
        assert!(!last.requires_rescan);
        assert_eq!(
            last.updated_results
                .iter()
                .find(|result| result.scan_id == initial.scan_id)
                .unwrap()
                .total_bytes,
            0
        );
    }
}
