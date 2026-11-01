use crate::filesystem::metadata::modified_ms;
use std::fs;
#[test]
fn cached_child_shared_groups_preserve_sibling_owners_among_unrelated_groups() {
    use super::*;
    let scope = std::env::temp_dir().join("mangodisk-navigation-groups");
    let child = scope.join("selected");
    let sibling = scope.join("sibling");
    let mut files = HashMap::new();
    for index in 0..20_000 {
        let identity = mangodisk_platform::PhysicalFileIdentity { volume: 1, index };
        for (parent, bytes) in [(&sibling, 4096), (&scope, 0)] {
            files.insert(
                parent.join(format!("unrelated-{index}.bin")),
                IndexedFile {
                    shared_identity: Some(identity),
                    bytes,
                    logical_bytes: 4096,
                    modified_at_ms: None,
                },
            );
        }
    }
    let identity = mangodisk_platform::PhysicalFileIdentity {
        volume: 1,
        index: 20_000,
    };
    for (parent, bytes) in [(&sibling, 4096), (&child, 0)] {
        files.insert(
            parent.join("shared.bin"),
            IndexedFile {
                shared_identity: Some(identity),
                bytes,
                logical_bytes: 4096,
                modified_at_ms: None,
            },
        );
    }
    let started = std::time::Instant::now();
    for _ in 0..5 {
        let groups = shared_allocations(&child, &scope, &files);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].identity, identity);
        assert_eq!(groups[0].bytes, 4096);
        assert_eq!(groups[0].owner, display_path(&sibling.join("shared.bin")));
        assert_eq!(groups[0].files.len(), 2);
    }
    eprintln!(
        "navigation_group_projection files={} iterations=5 elapsed_us={}",
        files.len(),
        started.elapsed().as_micros()
    );
    let empty = scope.join("ordinary");
    assert!(shared_allocations(&empty, &scope, &files).is_empty());
}

#[test]
fn empty_and_sparse_files_do_not_form_shared_allocation_groups() {
    use super::*;
    let root = std::env::temp_dir().join("mangodisk-shared-allocation-fixture");
    let ordinary = IndexedFile {
        shared_identity: None,
        bytes: 0,
        logical_bytes: 0,
        modified_at_ms: None,
    };
    let files = HashMap::from([
        (root.join("empty.bin"), ordinary),
        (
            root.join("sparse.bin"),
            IndexedFile {
                logical_bytes: 8192,
                ..ordinary
            },
        ),
    ]);
    assert!(shared_allocations(&root, &root, &files).is_empty());
}

#[test]
fn bounded_analysis_ranking_matches_full_sort_at_limits_and_equal_byte_ties() {
    for count in [0, 1, 499, 500, 501, 30_000] {
        let mut entries = (0..count)
            .rev()
            .map(|index| super::DirectoryEntryInfo {
                size_status: Default::default(),
                name: format!("file-{index}"),
                path: format!("/fixture/file-{index:05}"),
                bytes: (index * 7919 % 97) as u64,
                logical_bytes: 0,
                file_count: 1,
                is_directory: false,
                modified_at_ms: None,
                content_fingerprint: None,
            })
            .collect::<Vec<_>>();
        let mut expected = entries.clone();
        expected.sort_by(|left, right| {
            right
                .bytes
                .cmp(&left.bytes)
                .then_with(|| left.path.cmp(&right.path))
        });
        expected.truncate(super::ANALYSIS_VISIBLE_ENTRY_LIMIT);
        super::super::query::rank_entries(&mut entries);
        assert_eq!(
            entries.iter().map(|entry| &entry.path).collect::<Vec<_>>(),
            expected.iter().map(|entry| &entry.path).collect::<Vec<_>>()
        );
    }
}

#[test]
#[ignore = "manual ranking benchmark on a fixed large direct-child workload"]
fn benchmark_bounded_analysis_ranking() {
    let source = (0..100_000)
        .map(|index| super::DirectoryEntryInfo {
            size_status: Default::default(),
            name: format!("file-{index}"),
            path: format!("/fixture/file-{index:06}"),
            bytes: (index * 7919 % 104729) as u64,
            logical_bytes: 0,
            file_count: 1,
            is_directory: false,
            modified_at_ms: None,
            content_fingerprint: None,
        })
        .collect::<Vec<_>>();
    for run in 0..10 {
        for bounded in if run % 2 == 0 {
            [false, true]
        } else {
            [true, false]
        } {
            let mut entries = source.clone();
            let start = std::time::Instant::now();
            if bounded {
                super::super::query::rank_entries(&mut entries);
            } else {
                entries.sort_by(|left, right| {
                    right
                        .bytes
                        .cmp(&left.bytes)
                        .then_with(|| left.path.cmp(&right.path))
                });
                entries.truncate(super::ANALYSIS_VISIBLE_ENTRY_LIMIT);
            }
            println!(
                "analysis_ranking bounded={bounded} run={run} elapsed_us={}",
                start.elapsed().as_micros()
            );
            assert_eq!(entries.len(), 500);
        }
    }
}

use super::*;

#[test]
fn hierarchy_projection_preserves_byte_floor_and_bounded_sorted_branches() {
    let root = Path::new("fixture");
    let mut directories = HashMap::new();
    directories.insert(
        root.to_path_buf(),
        DirectoryAggregate {
            bytes: 2_000_000,
            ..DirectoryAggregate::default()
        },
    );
    for index in 0..96 {
        let branch = root.join(format!("branch-{index:02}"));
        directories.insert(
            branch.clone(),
            DirectoryAggregate {
                bytes: 20_000 + index,
                file_count: 2,
                ..DirectoryAggregate::default()
            },
        );
        for (name, bytes) in [("visible", 1_000), ("small", 999)] {
            directories.insert(
                branch.join(name),
                DirectoryAggregate {
                    bytes,
                    file_count: 1,
                    ..DirectoryAggregate::default()
                },
            );
        }
    }
    directories.insert(
        PathBuf::from("unrelated/large"),
        DirectoryAggregate {
            bytes: 3_000_000,
            ..DirectoryAggregate::default()
        },
    );
    let nodes = build_directory_hierarchy(root, &directories, &HashMap::new());
    assert_eq!(nodes.len(), 64);
    for (offset, node) in nodes.iter().enumerate() {
        assert_eq!(node.name, format!("branch-{:02}", 95 - offset));
        assert_eq!(node.total_entry_count, 2);
        assert_eq!(node.children.len(), 1);
        assert_eq!(node.children[0].name, "visible");
        assert_eq!(node.children[0].bytes, 1_000);
    }
}

#[test]
fn parallel_initial_rows_preserve_native_usage_stale_files_and_zero_charge_aliases() {
    let root = tempfile::tempdir().unwrap();
    for index in 0..600 {
        fs::write(
            root.path().join(format!("file-{index:04}")),
            vec![0_u8; index + 1],
        )
        .unwrap();
    }
    #[cfg(windows)]
    {
        let path = root.path().join("wof.bin");
        fs::write(&path, vec![0_u8; 1024 * 1024]).unwrap();
        let output = std::process::Command::new("compact.exe")
            .args(["/C", "/F", "/EXE:XPRESS4K"])
            .arg(&path)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "WOF fixture compression must succeed"
        );
        let metadata = fs::symlink_metadata(&path).unwrap();
        let usage = current_platform().file_space_usage(&path, &metadata);
        assert!(
            usage.allocated_bytes < usage.logical_bytes,
            "WOF fixture must have native compressed usage before comparing scan results"
        );
    }
    let files = HashMap::from([
        (
            root.path().join("file-0000"),
            IndexedFile {
                shared_identity: None,
                bytes: 0,
                logical_bytes: 1,
                modified_at_ms: None,
            },
        ),
        (
            root.path().join("file-0001"),
            IndexedFile {
                shared_identity: None,
                bytes: 9999,
                logical_bytes: 9999,
                modified_at_ms: None,
            },
        ),
    ]);
    let build = |workers| {
        analysis_result_from_snapshot(
            root.path(),
            DirectoryAggregate {
                scanned_at_ms: 100,
                ..DirectoryAggregate::default()
            },
            &HashMap::new(),
            &files,
            &[],
            &mangodisk_platform::NameExclusions::default(),
            workers,
        )
        .unwrap()
    };
    let expected = serde_json::to_value(build(1)).unwrap();
    for workers in [2, 4, 8] {
        assert_eq!(serde_json::to_value(build(workers)).unwrap(), expected);
    }
}

#[test]
fn analysis_result_bounds_large_projections_and_preserves_exact_totals() {
    let root = tempfile::tempdir().unwrap();
    for index in 1..=ANALYSIS_VISIBLE_ENTRY_LIMIT {
        fs::write(root.path().join(format!("{index:05}.bin")), []).unwrap();
    }
    let build = || {
        let children = read_analysis_children(
            root.path(),
            &[],
            &mangodisk_platform::NameExclusions::default(),
        )
        .unwrap();
        let entries = build_analysis_entries(
            children,
            super::super::AnalysisScanMode::Standard,
            |_| None,
            |path| {
                let bytes = path.file_stem()?.to_str()?.parse().ok()?;
                Some(IndexedFile {
                    shared_identity: None,
                    bytes,
                    logical_bytes: 0,
                    modified_at_ms: modified_ms(&fs::metadata(path).unwrap()),
                })
            },
        );
        let count = entries.iter().filter(|entry| entry.bytes > 0).count();
        build_analysis_result_from_entries(
            root.path(),
            DirectoryAggregate::default(),
            entries,
            Default::default(),
            count,
        )
    };
    let exact = build();
    assert_eq!(exact.entries.len(), ANALYSIS_VISIBLE_ENTRY_LIMIT);
    assert!(!exact.truncated);
    assert_eq!(exact.total_entry_count, ANALYSIS_VISIBLE_ENTRY_LIMIT);

    fs::write(
        root.path()
            .join(format!("{:05}.bin", ANALYSIS_VISIBLE_ENTRY_LIMIT + 1)),
        [],
    )
    .unwrap();
    let limited = build();
    assert_eq!(limited.entries.len(), ANALYSIS_VISIBLE_ENTRY_LIMIT);
    assert!(limited.truncated);
    assert_eq!(limited.total_entry_count, ANALYSIS_VISIBLE_ENTRY_LIMIT + 1);
    assert_eq!(
        limited.entries.first().unwrap().bytes,
        (ANALYSIS_VISIBLE_ENTRY_LIMIT + 1) as u64
    );
    assert_eq!(limited.entries.last().unwrap().name, "00002.bin");
    fs::write(root.path().join("000.bin"), []).unwrap();
    assert_eq!(
        build().total_entry_count,
        ANALYSIS_VISIBLE_ENTRY_LIMIT + 1,
        "zero-byte items are not part of Other"
    );
    assert_eq!(
        serde_json::to_value(limited).unwrap()["totalEntryCount"],
        ANALYSIS_VISIBLE_ENTRY_LIMIT + 1
    );
}

#[test]
fn unreadable_directory_status_does_not_contaminate_known_empty_siblings() {
    let fixture = tempfile::tempdir().unwrap();
    let root = current_platform()
        .canonicalize_no_links(fixture.path())
        .unwrap();
    let empty = root.join("empty");
    let unavailable = root.join("unavailable");
    fs::create_dir(&empty).unwrap();
    fs::create_dir(&unavailable).unwrap();
    let denied = DirectoryAggregate {
        skipped_count: 1,
        omissions: mangodisk_platform::ScanOmissionCounts {
            permission_denied: 1,
            ..Default::default()
        },
        ..Default::default()
    };
    let directories = HashMap::from([
        (empty, DirectoryAggregate::default()),
        (unavailable.clone(), denied),
    ]);
    for retained_file_limit in [0, 1000] {
        let result = analysis_result_from_snapshot(
            &root,
            DirectoryAggregate {
                retained_file_limit,
                ..denied
            },
            &directories,
            &HashMap::new(),
            &[],
            &Default::default(),
            1,
        )
        .unwrap();
        assert_eq!(
            result
                .entries
                .iter()
                .find(|entry| entry.name == "empty")
                .unwrap()
                .size_status,
            crate::AnalysisEntrySizeStatus::Complete
        );
        assert_eq!(
            result
                .entries
                .iter()
                .find(|entry| entry.name == "unavailable")
                .unwrap()
                .size_status,
            crate::AnalysisEntrySizeStatus::Unavailable
        );
    }
}

#[cfg(unix)]
#[test]
fn colliding_display_names_preserve_each_directorys_own_chart_count() {
    use std::os::unix::ffi::OsStringExt;
    let root = Path::new("/fixture");
    let directories = HashMap::from([
        (
            root.to_path_buf(),
            DirectoryAggregate {
                bytes: 32768,
                ..Default::default()
            },
        ),
        (
            root.join(std::ffi::OsString::from_vec(vec![b'd', 0x80])),
            DirectoryAggregate {
                bytes: 8192,
                file_count: 2,
                direct_file_count: 2,
                ..Default::default()
            },
        ),
        (
            root.join(std::ffi::OsString::from_vec(vec![b'd', 0x81])),
            DirectoryAggregate {
                bytes: 12288,
                file_count: 5,
                direct_file_count: 5,
                ..Default::default()
            },
        ),
    ]);
    for _ in 0..8 {
        let nodes = build_directory_hierarchy(root, &directories, &HashMap::new());
        assert_eq!(nodes.len(), 2);
        assert_eq!(nodes[0].total_entry_count, 5);
        assert_eq!(nodes[1].total_entry_count, 2);
    }
}
