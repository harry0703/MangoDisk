use std::{
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

#[cfg(target_os = "macos")]
use std::time::Duration;

use super::*;

#[test]
fn retained_rows_preserve_empty_entries_at_the_visible_limit() {
    let _operation_lock = crate::shared::operation::test_operation_lock();
    for count in [499, 500, 501] {
        let fixture = tempfile::tempdir().unwrap();
        let root = current_platform()
            .canonicalize_no_links(fixture.path())
            .unwrap();
        fs::write(root.join("empty-file"), []).unwrap();
        for index in 0..count {
            fs::write(root.join(format!("file-{index:04}")), [1; 4096]).unwrap();
        }
        let progress = Arc::new(ProgressTracker::new(0, |_| {}, 0));
        let (aggregate, snapshot) = traverse_memory_only(
            &root,
            TraversalKind::Analysis(AnalysisScanMode::Standard),
            now_ms(),
            None,
            &progress,
            &AtomicBool::new(false),
            None,
        )
        .unwrap();
        let project = |aggregate| {
            cache::analysis_result_from_snapshot(
                &root,
                aggregate,
                &snapshot.directories,
                &snapshot.files,
                &[],
                &mangodisk_platform::NameExclusions::default(),
                1,
            )
            .unwrap()
        };
        let baseline = project(DirectoryAggregate {
            retained_file_limit: 0,
            ..aggregate
        });
        let retained = project(aggregate);
        assert_eq!(
            retained.truncated, baseline.truncated,
            "positive files={count}; zero-allocation entries still affect truncation"
        );
        assert_eq!(
            serde_json::to_value(retained.entries).unwrap(),
            serde_json::to_value(baseline.entries).unwrap()
        );
        assert_eq!(retained.total_entry_count, baseline.total_entry_count);
    }
}

#[test]
fn retained_directory_rows_preserve_empty_entries_at_the_visible_limit() {
    let _operation_lock = crate::shared::operation::test_operation_lock();
    for count in [499, 500, 501] {
        let fixture = tempfile::tempdir().unwrap();
        let root = current_platform()
            .canonicalize_no_links(fixture.path())
            .unwrap();
        fs::write(root.join("empty-file"), []).unwrap();
        for index in 0..count {
            let child = root.join(format!("directory-{index:04}"));
            fs::create_dir(&child).unwrap();
            fs::write(child.join("file"), [1; 4096]).unwrap();
        }
        let progress = Arc::new(ProgressTracker::new(0, |_| {}, 0));
        let (aggregate, snapshot) = traverse_memory_only(
            &root,
            TraversalKind::Analysis(AnalysisScanMode::Standard),
            now_ms(),
            None,
            &progress,
            &AtomicBool::new(false),
            None,
        )
        .unwrap();
        let result = cache::analysis_result_from_snapshot(
            &root,
            aggregate,
            &snapshot.directories,
            &snapshot.files,
            &[],
            &mangodisk_platform::NameExclusions::default(),
            1,
        )
        .unwrap();
        assert_eq!(result.total_entry_count, count);
        assert_eq!(
            result.truncated,
            count >= 500,
            "an omitted empty file still causes truncation at exactly 500 directories"
        );
        assert_eq!(result.entries.len(), (count + 1).min(500));
        assert_eq!(result.entries[0].name, "directory-0000");
        if count == 499 {
            assert_eq!(result.entries.last().unwrap().name, "empty-file");
        }
    }
}

#[cfg(target_os = "macos")]
#[test]
fn retained_child_navigation_preserves_refresh_and_no_follow_delete_boundaries() {
    let _operation_lock = crate::shared::operation::test_operation_lock();
    cache::clear_all().unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let child = fixture.path().join("child");
    fs::create_dir(&child).unwrap();
    for index in 0..600 {
        fs::write(child.join(format!("file-{index:04}")), [1; 4096]).unwrap();
    }
    crate::AnalysisService::analyze_with_progress(
        Some(current_platform().display_path(fixture.path())),
        true,
        |_| {},
    )
    .unwrap();
    let navigate = || {
        crate::AnalysisService::analyze_with_progress(
            Some(current_platform().display_path(&child)),
            false,
            |_| {},
        )
        .unwrap()
    };
    let initial = navigate();
    let selected = initial.entries[0].path.clone();
    let outside = fixture.path().join("outside");
    fs::write(&outside, [2; 8192]).unwrap();
    fs::remove_file(&selected).unwrap();
    std::os::unix::fs::symlink(&outside, &selected).unwrap();
    let cached = navigate();
    assert!(cached.entries.iter().any(|entry| entry.path == selected));
    let error = crate::AnalysisService::delete_entry_permanently(cached.scan_id, selected.clone())
        .expect_err("cached file facts must never authorize following a replacement symlink");
    assert_eq!(
        error.mutation_state(),
        mangodisk_platform::PlatformMutationState::NotAttempted
    );
    assert!(fs::symlink_metadata(&selected)
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(fs::read(&outside).unwrap(), [2; 8192]);
    let refreshed = crate::AnalysisService::analyze_with_progress(
        Some(current_platform().display_path(&child)),
        true,
        |_| {},
    )
    .unwrap();
    assert!(!refreshed.entries.iter().any(|entry| entry.path == selected));
    assert_eq!(refreshed.total_entry_count + 1, initial.total_entry_count);
    cache::clear_all().unwrap();
}

#[cfg(target_os = "linux")]
#[test]
fn retained_rows_match_display_path_order_for_non_utf8_names() {
    use std::os::unix::ffi::OsStringExt;
    let _operation_lock = crate::shared::operation::test_operation_lock();
    let fixture = tempfile::tempdir().unwrap();
    let root = current_platform()
        .canonicalize_no_links(fixture.path())
        .unwrap();
    for index in 0..501 {
        fs::write(root.join(format!("é-{index:04}")), [1; 4096]).unwrap();
    }
    for index in 0..1500 {
        let mut name = vec![0x80];
        name.extend_from_slice(format!("-{index:04}").as_bytes());
        fs::write(root.join(std::ffi::OsString::from_vec(name)), [1; 4096]).unwrap();
    }
    let progress = Arc::new(ProgressTracker::new(0, |_| {}, 0));
    let (aggregate, snapshot) = traverse_memory_only(
        &root,
        TraversalKind::Analysis(AnalysisScanMode::Standard),
        now_ms(),
        None,
        &progress,
        &AtomicBool::new(false),
        None,
    )
    .unwrap();
    let project = |aggregate| {
        cache::analysis_result_from_snapshot(
            &root,
            aggregate,
            &snapshot.directories,
            &snapshot.files,
            &[],
            &mangodisk_platform::NameExclusions::default(),
            1,
        )
        .unwrap()
    };
    let live = project(DirectoryAggregate {
        retained_file_limit: 0,
        ..aggregate
    });
    let cached = project(aggregate);
    let paths = |result: AnalysisResult| {
        result
            .entries
            .into_iter()
            .map(|entry| entry.path)
            .collect::<Vec<_>>()
    };
    assert_eq!(paths(cached), paths(live));
}

#[test]
fn retained_navigation_rows_match_independent_metadata_and_fall_back_after_eviction() {
    let _operation_lock = crate::shared::operation::test_operation_lock();
    let fixture = tempfile::tempdir().unwrap();
    let root = current_platform()
        .canonicalize_no_links(fixture.path())
        .unwrap();
    fs::create_dir(root.join("empty-dir")).unwrap();
    fs::create_dir(root.join("child")).unwrap();
    fs::write(root.join("child/nested"), [1; 4096]).unwrap();
    fs::write(root.join("empty"), []).unwrap();
    for index in 0..1100 {
        fs::write(
            root.join(format!("file-{index:04}")),
            vec![3; (index % 19 + 1) * 4096],
        )
        .unwrap();
    }
    #[cfg(unix)]
    {
        fs::hard_link(root.join("file-1099"), root.join("alias")).unwrap();
        std::os::unix::fs::symlink(root.join("file-0000"), root.join("symlink")).unwrap();
    }
    let progress = Arc::new(ProgressTracker::new(0, |_| {}, 0));
    let (aggregate, mut snapshot) = traverse_memory_only(
        &root,
        TraversalKind::Analysis(AnalysisScanMode::Standard),
        now_ms(),
        None,
        &progress,
        &AtomicBool::new(false),
        None,
    )
    .unwrap();
    assert_eq!(
        aggregate.retained_file_limit,
        super::index_sink::ANALYSIS_FILES_PER_DIRECTORY
    );
    let baseline_aggregate = DirectoryAggregate {
        retained_file_limit: 0,
        ..aggregate
    };
    let project = |aggregate, snapshot: &CompletedIndexSink| {
        cache::analysis_result_from_snapshot(
            &root,
            aggregate,
            &snapshot.directories,
            &snapshot.files,
            &[],
            &mangodisk_platform::NameExclusions::default(),
            1,
        )
        .unwrap()
    };
    let rows = |result: &AnalysisResult| {
        result
            .entries
            .iter()
            .map(|entry| {
                (
                    entry.path.clone(),
                    entry.bytes,
                    entry.logical_bytes,
                    entry.file_count,
                    entry.is_directory,
                )
            })
            .collect::<Vec<_>>()
    };
    let baseline = project(baseline_aggregate, &snapshot);
    let retained = project(aggregate, &snapshot);
    assert_eq!(rows(&retained), rows(&baseline));
    assert_eq!(retained.total_entry_count, baseline.total_entry_count);
    assert_eq!(retained.total_bytes, baseline.total_bytes);
    assert_eq!(retained.truncated, baseline.truncated);
    assert_eq!(
        retained.entries.len(),
        crate::storage::analysis::ANALYSIS_VISIBLE_ENTRY_LIMIT
    );
    // A read-only cached page stays coherent with its scan even if an external
    // mutation happens. Refresh and destructive preflight revalidate live state.
    let selected_file = retained
        .entries
        .iter()
        .find(|entry| !entry.is_directory && !entry.path.ends_with("/alias"))
        .unwrap();
    fs::remove_file(&selected_file.path).unwrap();
    assert_eq!(rows(&project(aggregate, &snapshot)), rows(&retained));
    // Losing enough retained rows must use real metadata rather than silently
    // publish an incomplete largest-file prefix or an incorrect remainder count.
    let removable: Vec<_> = snapshot
        .files
        .iter()
        .filter(|(_, file)| file.shared_identity.is_none())
        .take(700)
        .map(|(path, _)| path.clone())
        .collect();
    for path in removable {
        snapshot.files.remove(&path);
    }
    let fallback = project(aggregate, &snapshot);
    let live = project(baseline_aggregate, &snapshot);
    assert_eq!(rows(&fallback), rows(&live));
    assert_eq!(fallback.total_entry_count, live.total_entry_count);
    assert_eq!(fallback.total_entry_count + 1, baseline.total_entry_count);
}

#[cfg(unix)]
#[test]
fn complete_shared_alias_navigation_uses_snapshot_without_reopening_directory() {
    let _operation_lock = crate::shared::operation::test_operation_lock();
    cache::clear_all().unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let root = current_platform()
        .canonicalize_no_links(fixture.path())
        .unwrap();
    let owners = root.join("a-owners");
    let aliases = root.join("z-aliases");
    fs::create_dir(&owners).unwrap();
    fs::create_dir(&aliases).unwrap();
    for index in 0..600 {
        let name = format!("file-{index:04}");
        fs::write(owners.join(&name), [1; 4096]).unwrap();
        fs::hard_link(owners.join(&name), aliases.join(&name)).unwrap();
    }
    let progress = Arc::new(ProgressTracker::new(0, |_| {}, 0));
    let (_, snapshot) = traverse_memory_only(
        &root,
        TraversalKind::Analysis(AnalysisScanMode::Standard),
        now_ms(),
        None,
        &progress,
        &AtomicBool::new(false),
        None,
    )
    .unwrap();
    let project = || {
        cache::analysis_result_from_snapshot(
            &aliases,
            snapshot.directories[&aliases],
            &snapshot.directories,
            &snapshot.files,
            &[],
            &mangodisk_platform::NameExclusions::default(),
            1,
        )
        .unwrap()
    };
    let before = project();
    assert_eq!(before.total_bytes, 0);
    assert_eq!(before.total_entry_count, 0);
    assert_eq!(before.entries.len(), 500);
    assert!(before.truncated);
    assert_eq!(before.shared_allocations.len(), 600);
    // Removing the directory path cannot affect a complete read-only snapshot.
    // Explicit refresh and destructive preflight still inspect live state.
    fs::rename(&aliases, root.join("moved-aliases")).unwrap();
    let after = project();
    assert_eq!(
        serde_json::to_value(before.entries).unwrap(),
        serde_json::to_value(after.entries).unwrap()
    );
    cache::clear_all().unwrap();
}

#[test]
#[ignore = "requires MANGODISK_TEST_NAVIGATION_ROOT and MANGODISK_TEST_NAVIGATION_CHILD"]
fn manual_cached_navigation_latency() {
    struct BenchmarkLogger;
    impl log::Log for BenchmarkLogger {
        fn enabled(&self, _: &log::Metadata<'_>) -> bool {
            true
        }
        fn log(&self, record: &log::Record<'_>) {
            if record.target().starts_with("mangodisk_core::storage") {
                eprintln!("{}", record.args());
            }
        }
        fn flush(&self) {}
    }
    static LOGGER: BenchmarkLogger = BenchmarkLogger;
    let _ = log::set_logger(&LOGGER);
    log::set_max_level(log::LevelFilter::Info);
    let _operation_lock = crate::shared::operation::test_operation_lock();
    cache::clear_all().unwrap();
    let root = std::env::var("MANGODISK_TEST_NAVIGATION_ROOT").unwrap();
    let child = std::env::var("MANGODISK_TEST_NAVIGATION_CHILD").unwrap();
    let (_, scan) = StorageTraversal::analyze_path_with_diagnostics(Some(root), true, |_| {})
        .expect("scan the explicit read-only benchmark root");
    eprintln!("navigation_initial_scan diagnostics={scan:?}");
    let mut children = vec![child];
    if let Some(extra) = std::env::var_os("MANGODISK_TEST_NAVIGATION_CHILDREN") {
        children
            .extend(std::env::split_paths(&extra).map(|path| path.to_string_lossy().into_owned()));
    }
    let iterations = std::env::var("MANGODISK_TEST_NAVIGATION_ITERATIONS")
        .ok()
        .map(|value| {
            value
                .parse::<usize>()
                .expect("positive benchmark iteration count")
        })
        .unwrap_or(3);
    assert!(iterations > 0);
    for child in children {
        for iteration in 0..iterations {
            let started = Instant::now();
            let events = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let recorded = Arc::clone(&events);
            let (result, diagnostics) = StorageTraversal::analyze_path_with_diagnostics(
                Some(child.clone()),
                false,
                move |_| {
                    recorded.fetch_add(1, Ordering::Relaxed);
                },
            )
            .expect("open the cached child");
            assert_eq!(diagnostics.fast_path, "cache");
            assert_eq!(events.load(Ordering::Relaxed), 0);
            eprintln!("navigation_cached_child child={} iteration={iteration} elapsed_us={} entries={} total_entries={} diagnostics={diagnostics:?}", crate::filesystem::metadata::diagnostic_path(std::path::Path::new(&child)), started.elapsed().as_micros(), result.entries.len(), result.total_entry_count);
        }
    }
}

#[test]
fn cached_child_navigation_does_not_emit_scan_progress() {
    let _operation_lock = crate::shared::operation::test_operation_lock();
    cache::clear_all().expect("clear the analysis cache before navigation validation");
    let fixture = tempfile::tempdir().unwrap();
    let child = fixture.path().join("child");
    fs::create_dir(&child).unwrap();
    fs::write(child.join("sample.bin"), vec![1_u8; 4096]).unwrap();
    let scan_events = Arc::new(Mutex::new(Vec::new()));
    let events = Arc::clone(&scan_events);
    StorageTraversal::analyze_path_with_diagnostics(
        Some(current_platform().display_path(fixture.path())),
        true,
        move |progress| events.lock().unwrap().push(progress),
    )
    .expect("scan the parent fixture");
    assert!(!scan_events.lock().unwrap().is_empty());

    scan_events.lock().unwrap().clear();
    let events = Arc::clone(&scan_events);
    let (result, diagnostics) = StorageTraversal::analyze_path_with_diagnostics(
        Some(current_platform().display_path(&child)),
        false,
        move |progress| events.lock().unwrap().push(progress),
    )
    .expect("navigate to an unvisited child using the parent snapshot");
    assert_eq!(diagnostics.fast_path, "cache");
    assert_eq!(result.entries.len(), 1);
    assert!(scan_events.lock().unwrap().is_empty());
}

#[test]
fn analysis_root_validation_preserves_platform_error_code() {
    let _operation_lock = crate::shared::operation::test_operation_lock();
    let fixture = tempfile::tempdir().unwrap();
    let missing = fixture.path().join("missing");

    let error = StorageTraversal::analyze_path_with_progress(
        Some(missing.to_string_lossy().into_owned()),
        true,
        |_| {},
    )
    .expect_err("a missing analysis root should fail during path validation");

    assert_eq!(error.code(), crate::shared::CoreErrorCode::Platform);
}

#[test]
fn name_exclusions_apply_to_native_scans_cached_results_and_parent_deletion() {
    use crate::{
        AnalysisService, DuplicateFileService, DuplicateScanLocation, DuplicateScanLocationMode,
        ExcludedNameKind, LargeFileService, ScanExclusionOptions, ScanNameExclusion,
    };
    let _operation_lock = crate::shared::operation::test_operation_lock();
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    fs::create_dir_all(root.join("node_modules/package")).unwrap();
    fs::create_dir_all(root.join("parent/node_modules")).unwrap();
    for index in 0..1000 {
        fs::write(
            root.join(format!("node_modules/package/file-{index}.js")),
            [1; 32],
        )
        .unwrap();
    }
    let data = vec![5; LARGE_FILE_CANDIDATE_FLOOR_BYTES as usize + 4096];
    fs::write(root.join("first.bin"), &data).unwrap();
    fs::write(root.join("second.bin"), &data).unwrap();
    fs::write(root.join(".DS_Store"), [7; 16]).unwrap();
    fs::write(root.join("parent/source.txt"), b"source").unwrap();
    let root_text =
        current_platform().display_path(&current_platform().canonicalize_no_links(root).unwrap());
    let options = ScanExclusionOptions {
        paths: Vec::new(),
        names: vec![
            ScanNameExclusion {
                name: "node_modules".into(),
                kind: ExcludedNameKind::Folder,
            },
            ScanNameExclusion {
                name: ".DS_Store".into(),
                kind: ExcludedNameKind::File,
            },
        ],
    };
    let unfiltered = AnalysisService::analyze_with_exclusions_progress(
        Some(root_text.clone()),
        true,
        Vec::new(),
        |_| {},
    )
    .unwrap();
    assert!(unfiltered.entries.iter().any(|e| e.name == "node_modules"));
    for refresh in [false, false] {
        let result = AnalysisService::analyze_with_exclusions_progress(
            Some(root_text.clone()),
            refresh,
            options.clone(),
            |_| {},
        )
        .unwrap();
        assert!(!result
            .entries
            .iter()
            .any(|e| e.name == "node_modules" || e.name == ".DS_Store"));
        assert_eq!(result.entries.iter().map(|e| e.file_count).sum::<u64>(), 3);
        let parent = result.entries.iter().find(|e| e.name == "parent").unwrap();
        assert!(
            AnalysisService::delete_entry_permanently(result.scan_id, parent.path.clone()).is_err()
        );
        assert!(root.join("parent/source.txt").exists());
        assert!(
            root.join("parent/node_modules").is_dir(),
            "an empty excluded directory is still protected"
        );
    }
    let large = LargeFileService::find_with_progress(
        vec![root_text.clone()],
        1,
        LargeFileScanMode::Complete,
        options.clone(),
        |_| {},
    )
    .unwrap();
    assert_eq!(large.entries.len(), 2);
    let duplicates = DuplicateFileService::find_paged_with_locations_and_exclusions(
        vec![DuplicateScanLocation {
            path: root_text.clone(),
            mode: DuplicateScanLocationMode::Cleanable,
        }],
        options.clone(),
        1,
        |_| {},
        |_| {},
    )
    .unwrap();
    assert_eq!(duplicates.scanned_file_count, 3);
    assert_eq!(duplicates.groups.len(), 1);
    let excluded_root = root.join("node_modules").to_string_lossy().into_owned();
    assert!(AnalysisService::analyze_with_exclusions_progress(
        Some(excluded_root),
        true,
        options,
        |_| {}
    )
    .is_err());
    let restored = AnalysisService::analyze_with_exclusions_progress(
        Some(root_text),
        false,
        Vec::new(),
        |_| {},
    )
    .unwrap();
    assert!(restored.entries.iter().any(|e| e.name == "node_modules"));
}

struct DirectoryCleanup(PathBuf);

#[test]
fn duplicate_directory_aggregation_preserves_excluded_empty_folders() {
    let _operation_lock = crate::shared::operation::test_operation_lock();
    let fixture = tempfile::tempdir().unwrap();
    for copy in ["first", "second"] {
        fs::create_dir_all(fixture.path().join(copy).join("node_modules")).unwrap();
        fs::write(fixture.path().join(copy).join("source.txt"), b"same source").unwrap();
    }
    let root = current_platform()
        .canonicalize_no_links(fixture.path())
        .unwrap();
    let result = crate::DuplicateFileService::find_paged_with_locations_and_exclusions(
        vec![crate::DuplicateScanLocation {
            path: root.to_string_lossy().into_owned(),
            mode: crate::DuplicateScanLocationMode::Cleanable,
        }],
        crate::ScanExclusionOptions {
            paths: Vec::new(),
            names: vec![crate::ScanNameExclusion {
                name: "node_modules".into(),
                kind: crate::ExcludedNameKind::Folder,
            }],
        },
        1,
        |_| {},
        |_| {},
    )
    .unwrap();
    assert_eq!(result.groups.len(), 1);
    assert!(
        result
            .groups
            .iter()
            .all(|group| group.kind == crate::storage::duplicates::DuplicateGroupKind::File),
        "a parent containing an excluded empty folder must not become a deletable directory group"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn analysis_parent_deletion_preserves_exclusions_using_system_path_aliases() {
    let _operation_lock = crate::shared::operation::test_operation_lock();
    let fixture = tempfile::tempdir().unwrap();
    let root = current_platform()
        .canonicalize_no_links(fixture.path())
        .unwrap();
    let protected = root.join("parent/keep");
    fs::create_dir_all(&protected).unwrap();
    fs::write(root.join("parent/source.txt"), b"source").unwrap();
    let alias = Path::new("/var").join(protected.strip_prefix("/private/var").unwrap());
    let result = crate::AnalysisService::analyze_with_exclusions_progress(
        Some(root.to_string_lossy().into_owned()),
        true,
        vec![alias.to_string_lossy().into_owned()],
        |_| {},
    )
    .unwrap();
    let parent = result
        .entries
        .iter()
        .find(|entry| entry.name == "parent")
        .unwrap();
    let deletion =
        crate::AnalysisService::delete_entry_permanently(result.scan_id, parent.path.clone());
    assert!(
        deletion.is_err(),
        "the canonical parent must retain the excluded alias: {deletion:?}"
    );
    assert!(protected.is_dir());
}

#[test]
fn analysis_parent_deletion_preserves_restored_missing_exclusion() {
    let _operation_lock = crate::shared::operation::test_operation_lock();
    let fixture = tempfile::tempdir().unwrap();
    let root = current_platform()
        .canonicalize_no_links(fixture.path())
        .unwrap();
    fs::create_dir_all(root.join("parent")).unwrap();
    fs::create_dir_all(root.join("other")).unwrap();
    fs::write(root.join("parent/source.txt"), b"source").unwrap();
    let protected = root.join("parent/keep");
    let requested = root.join("other/../parent/keep");
    let result = crate::AnalysisService::analyze_with_exclusions_progress(
        Some(root.to_string_lossy().into_owned()),
        true,
        vec![requested.to_string_lossy().into_owned()],
        |_| {},
    )
    .unwrap();
    fs::create_dir(&protected).unwrap();
    fs::write(protected.join("keep.txt"), b"protected").unwrap();
    let parent = result
        .entries
        .iter()
        .find(|entry| entry.name == "parent")
        .unwrap();
    let deletion =
        crate::AnalysisService::delete_entry_permanently(result.scan_id, parent.path.clone());
    assert!(
        deletion.is_err(),
        "a restored excluded directory must block parent deletion: {deletion:?}"
    );
    assert!(protected.join("keep.txt").exists());
}

#[cfg(target_os = "macos")]
#[test]
fn analysis_parent_deletion_preserves_restored_missing_alias_exclusion() {
    let _operation_lock = crate::shared::operation::test_operation_lock();
    let fixture = tempfile::tempdir().unwrap();
    let root = current_platform()
        .canonicalize_no_links(fixture.path())
        .unwrap();
    fs::create_dir(root.join("parent")).unwrap();
    fs::write(root.join("parent/source.txt"), b"source").unwrap();
    let protected = root.join("parent/keep");
    let alias = Path::new("/var").join(protected.strip_prefix("/private/var").unwrap());
    let result = crate::AnalysisService::analyze_with_exclusions_progress(
        Some(root.to_string_lossy().into_owned()),
        true,
        vec![alias.to_string_lossy().into_owned()],
        |_| {},
    )
    .unwrap();
    fs::create_dir(&protected).unwrap();
    let parent = result
        .entries
        .iter()
        .find(|entry| entry.name == "parent")
        .unwrap();
    let deletion =
        crate::AnalysisService::delete_entry_permanently(result.scan_id, parent.path.clone());
    assert!(
        deletion.is_err(),
        "a restored alias exclusion must block parent deletion: {deletion:?}"
    );
    assert!(protected.is_dir());
}

#[test]
fn analysis_parent_deletion_preserves_resolved_exclusions_on_repeated_scans() {
    let _operation_lock = crate::shared::operation::test_operation_lock();
    let fixture = tempfile::tempdir().unwrap();
    let root = current_platform()
        .canonicalize_no_links(fixture.path())
        .unwrap();
    let protected = root.join("parent/keep");
    fs::create_dir_all(&protected).unwrap();
    fs::write(root.join("parent/source.txt"), b"source").unwrap();
    let requested = protected.join("..").join("keep");
    for refresh in [true, false] {
        let result = crate::AnalysisService::analyze_with_exclusions_progress(
            Some(root.to_string_lossy().into_owned()),
            refresh,
            vec![requested.to_string_lossy().into_owned()],
            |_| {},
        )
        .unwrap();
        let parent = result
            .entries
            .iter()
            .find(|entry| entry.name == "parent")
            .unwrap();
        assert!(crate::AnalysisService::delete_entry_permanently(
            result.scan_id,
            parent.path.clone()
        )
        .is_err());
        assert!(protected.is_dir());
        assert!(root.join("parent/source.txt").exists());
    }
}

#[cfg(windows)]
#[test]
#[ignore = "creates an isolated fixture under an explicitly supplied shared directory"]
fn real_redirected_share_supports_storage_scans() {
    use crate::storage::{
        analysis::AnalysisService,
        duplicates::{DuplicateFileService, DuplicateScanLocation, DuplicateScanLocationMode},
        large_files::LargeFileService,
    };
    let _operation_lock = crate::shared::operation::test_operation_lock();
    let parent = std::env::var("MANGODISK_TEST_SHARED_SCAN_PARENT")
        .expect("supply a redirected shared directory for isolated test files");
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let entry = PathBuf::from(parent).join(format!("MangoDisk-Shared-Scan-{unique}"));
    fs::create_dir(&entry).expect("create the isolated shared fixture");
    let _cleanup = DirectoryCleanup(entry.clone());
    let data = vec![0x5a_u8; LARGE_FILE_CANDIDATE_FLOOR_BYTES as usize + 4096];
    fs::write(entry.join("first.bin"), &data).unwrap();
    fs::write(entry.join("second.bin"), &data).unwrap();
    fs::create_dir(entry.join("nested")).unwrap();
    fs::write(entry.join("nested/unique.txt"), b"different content").unwrap();
    let excluded = entry.join("excluded-small-files");
    fs::create_dir(&excluded).expect("create the high-file-count excluded directory");
    for index in 0..2_000 {
        fs::write(excluded.join(format!("small-{index}.tmp")), [index as u8])
            .expect("write an excluded small-file fixture");
    }
    let canonical = current_platform()
        .resolve_directory_entry(&entry)
        .expect("resolve the redirected shared directory");
    let root = current_platform().display_path(&canonical);
    let excluded = current_platform()
        .resolve_directory_entry(&excluded)
        .map(|path| current_platform().display_path(&path))
        .expect("resolve the excluded high-file-count directory");
    let baseline_started = Instant::now();
    let unfiltered = DuplicateFileService::find_paged_with_locations_and_exclusions(
        vec![DuplicateScanLocation {
            path: root.clone(),
            mode: DuplicateScanLocationMode::Cleanable,
        }],
        Vec::new(),
        1,
        |_| {},
        |_| {},
    )
    .expect("scan the complete high-file-count fixture");
    let baseline_ms = baseline_started.elapsed().as_millis();
    assert_eq!(unfiltered.scanned_file_count, 2_003);
    let started = Instant::now();
    let analysis = AnalysisService::analyze_with_exclusions_progress(
        Some(root.clone()),
        true,
        vec![excluded.clone()],
        |_| {},
    )
    .unwrap();
    assert_eq!(
        analysis
            .entries
            .iter()
            .map(|entry| entry.file_count)
            .sum::<u64>(),
        3
    );
    assert!(analysis
        .entries
        .iter()
        .all(|entry| entry.name != "excluded-small-files"));
    let large = LargeFileService::find_with_progress(
        vec![root.clone()],
        1,
        LargeFileScanMode::Complete,
        vec![excluded.clone()],
        |_| {},
    )
    .unwrap();
    assert_eq!(large.entries.len(), 2);
    let filtered_duplicate_started = Instant::now();
    let duplicates = DuplicateFileService::find_paged_with_locations_and_exclusions(
        vec![DuplicateScanLocation {
            path: root,
            mode: DuplicateScanLocationMode::Cleanable,
        }],
        vec![excluded],
        1,
        |_| {},
        |_| {},
    )
    .unwrap();
    let filtered_duplicate_ms = filtered_duplicate_started.elapsed().as_millis();
    assert_eq!(duplicates.scanned_file_count, 3);
    assert_eq!(duplicates.groups.len(), 1);
    assert_eq!(duplicates.groups[0].entries.len(), 2);
    assert_eq!(duplicates.groups[0].bytes_per_file, data.len() as u64);
    println!(
        "shared_scan_exclusions_verified included_files=3 excluded_files=2000 large_files=2 duplicate_groups=1 unfiltered_duplicate_ms={baseline_ms} filtered_duplicate_ms={filtered_duplicate_ms} filtered_workflow_ms={}",
        started.elapsed().as_millis()
    );
}

impl Drop for DirectoryCleanup {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn saved_parent_exclusion_prunes_explicit_large_and_duplicate_scan_roots() {
    use crate::storage::duplicates::{
        DuplicateFileService, DuplicateScanLocation, DuplicateScanLocationMode,
    };

    let _operation_lock = crate::shared::operation::test_operation_lock();
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let parent = std::env::temp_dir().join(format!(
        "mangodisk-explicit-root-exclusion-{}-{unique}",
        std::process::id()
    ));
    let root = parent.join("selected");
    fs::create_dir_all(&root).expect("create the selected scan root");
    let _cleanup = DirectoryCleanup(parent.clone());
    let contents = vec![0x42_u8; LARGE_FILE_CANDIDATE_FLOOR_BYTES as usize + 1];
    fs::write(root.join("first.bin"), &contents).expect("create the first scan candidate");
    fs::write(root.join("second.bin"), &contents).expect("create the second scan candidate");
    let root_value = current_platform().display_path(&root);
    let parent_value = current_platform().display_path(&parent);

    let (large, diagnostics) = StorageTraversal::find_large_files_with_diagnostics(
        vec![root_value.clone()],
        1,
        LargeFileScanMode::Complete,
        vec![parent_value.clone()],
        |_| {},
    )
    .expect("exclude the selected large-file root");
    assert_eq!(large.total_count, 0);
    assert_eq!(diagnostics.candidate_strategy, "excluded_root");

    let duplicates = DuplicateFileService::find_paged_with_locations_and_exclusions(
        vec![DuplicateScanLocation {
            path: root_value,
            mode: DuplicateScanLocationMode::Cleanable,
        }],
        vec![parent_value],
        1,
        |_| {},
        |_| {},
    )
    .expect("exclude the selected duplicate-file root");
    assert_eq!(duplicates.scanned_file_count, 0);
    assert!(duplicates.groups.is_empty());
}

#[test]
fn cancellation_at_final_analysis_progress_does_not_publish_a_snapshot() {
    let _operation_lock = crate::shared::operation::test_operation_lock();
    cache::clear_all().unwrap();
    let fixture = tempfile::tempdir().unwrap();
    fs::write(fixture.path().join("sample.bin"), b"content").unwrap();
    let root = current_platform()
        .canonicalize_no_links(fixture.path())
        .unwrap();
    let displayed_root = current_platform().display_path(&root);
    let callback_root = displayed_root.clone();
    let error =
        StorageTraversal::analyze_path_with_progress(Some(displayed_root), true, move |progress| {
            if progress.current_path == callback_root && progress.items_scanned >= 1 {
                OperationGuard::cancel(CoordinatedOperationKind::Analysis);
            }
        })
        .expect_err("cancellation before publication must not complete the analysis");
    assert_eq!(
        error.code(),
        crate::shared::CoreErrorCode::OperationCancelled
    );
    assert!(cache::analysis_result(&root).unwrap().is_none());
}

#[test]
fn traversal_cancellation_preserves_the_typed_error_code() {
    let error = traversal_core_error(OPERATION_CANCELLED_ERROR.to_string());

    assert_eq!(
        error.code(),
        crate::shared::CoreErrorCode::OperationCancelled
    );
}

#[test]
fn native_worker_shutdown_preserves_the_retryable_busy_code() {
    // Other tests may hold the process-wide operation slot in parallel.
    let _operation_lock = crate::shared::operation::test_operation_lock();
    let operation = OperationGuard::start(CoordinatedOperationKind::Analysis)
        .expect("the isolated analysis operation should start");
    let error = analysis_stream_core_error(&operation, AnalysisStreamError::ResourcesReleasing);

    assert_eq!(error.code(), crate::shared::CoreErrorCode::OperationBusy);
    assert_eq!(
        error.reason(),
        Some(crate::shared::CoreErrorReason::ScanResourcesReleasing)
    );
}

#[test]
fn native_large_file_candidate_below_physical_threshold_is_not_skipped() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "MangoDisk-Large-Candidate-{}-{unique}",
        std::process::id()
    ));
    let _sandbox_cleanup = DirectoryCleanup(root.clone());
    fs::create_dir_all(&root).expect("create the large-file candidate fixture");
    let path = root.join("candidate.bin");
    fs::write(&path, [1_u8, 2, 3, 4]).expect("write the large-file candidate fixture");
    let metadata = fs::metadata(&path).expect("read the large-file candidate metadata");
    let allocated = current_platform()
        .file_space_usage(&path, &metadata)
        .allocated_bytes;
    let progress = Arc::new(ProgressTracker::new(0, |_| {}, 0));
    let cancelled = AtomicBool::new(false);
    let exclusions = StorageScanExclusions::resolve(&root, &[]).expect("prepare empty exclusions");
    let mut validation = LargeFileStreamValidation::new(
        &root,
        allocated.saturating_add(1),
        now_ms(),
        &progress,
        &cancelled,
        true,
        &exclusions,
    )
    .expect("prepare native large-file validation");
    let mut sink = IndexRecordSink::memory(None);

    validation
        .consume(path, &mut sink)
        .expect("filter the ineligible native candidate");

    assert_eq!(validation.valid_count, 0);
    assert_eq!(validation.aggregate.skipped_count, 0);
}

#[test]
fn duplicate_native_large_file_candidate_is_idempotent() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "MangoDisk-Duplicate-Large-Candidate-{}-{unique}",
        std::process::id()
    ));
    let _sandbox_cleanup = DirectoryCleanup(root.clone());
    fs::create_dir_all(&root).expect("create the duplicate candidate fixture");
    let path = root.join("candidate.bin");
    fs::write(&path, [1_u8, 2, 3, 4]).expect("write the duplicate candidate fixture");
    let metadata = fs::metadata(&path).expect("read the duplicate candidate metadata");
    let usage = current_platform().file_space_usage(&path, &metadata);
    let progress = Arc::new(ProgressTracker::new(0, |_| {}, 0));
    let cancelled = AtomicBool::new(false);
    let exclusions = StorageScanExclusions::resolve(&root, &[]).expect("prepare empty exclusions");
    let mut validation = LargeFileStreamValidation::new(
        &root,
        0,
        now_ms(),
        &progress,
        &cancelled,
        true,
        &exclusions,
    )
    .expect("prepare native large-file validation");
    let mut sink = IndexRecordSink::memory(None);

    validation
        .consume(path.clone(), &mut sink)
        .expect("accept the first native candidate");
    validation
        .consume(path, &mut sink)
        .expect("ignore a duplicate native candidate");

    assert_eq!(validation.valid_count, 1);
    assert_eq!(validation.aggregate.bytes, usage.allocated_bytes);
    assert_eq!(validation.aggregate.logical_bytes, usage.logical_bytes);
    assert_eq!(validation.aggregate.file_count, 1);
    assert_eq!(validation.aggregate.skipped_count, 0);
    assert_eq!(
        sink.finish()
            .expect("finish the deduplicated candidate index")
            .files
            .len(),
        1
    );
}

#[cfg(target_os = "macos")]
#[test]
fn analysis_filesystem_boundary_keeps_firmlinks_and_rejects_mounts() {
    let platform = current_platform();
    let root = fs::symlink_metadata("/").expect("the system volume metadata should be readable");
    let users =
        fs::symlink_metadata("/Users").expect("the user-directory metadata should be readable");
    let device_mount =
        fs::symlink_metadata("/dev").expect("the device mount metadata should be readable");

    assert!(platform.is_same_filesystem(&root, &users));
    assert!(!platform.is_same_filesystem(&root, &device_mount));
}

#[test]
fn isolated_analysis_scans_only_requested_root() {
    let _operation_lock = crate::shared::operation::test_operation_lock();
    cache::clear_all().expect("the memory cache should be cleared before the test");
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "MangoDisk-Analysis-{}-{unique}",
        std::process::id()
    ));
    let _sandbox_cleanup = DirectoryCleanup(root.clone());
    let fixture = root.join("fingerprint-test").join("sample.bin");
    fs::create_dir_all(
        fixture
            .parent()
            .expect("the fixture file should have a parent directory"),
    )
    .expect("the analysis fixture directory should be created");
    fs::write(&fixture, [1_u8, 2, 3, 4, 5, 6])
        .expect("the analysis fixture file should be written");
    let expected_allocated = current_platform()
        .file_space_usage(
            &fixture,
            &fs::metadata(&fixture).expect("the analysis fixture metadata should be readable"),
        )
        .allocated_bytes;

    let result = StorageTraversal::analyze_path_with_progress(
        Some(root.to_string_lossy().into_owned()),
        true,
        |_| {},
    )
    .expect("analysis of the isolated directory should succeed");

    assert_eq!(result.total_bytes, expected_allocated);
    assert_eq!(result.skipped_count, 0);
    assert_eq!(result.entries.len(), 1);
    assert_eq!(result.entries[0].name, "fingerprint-test");
    assert_eq!(
        cache::memory_entry_counts().expect("memory-cache counts should be readable"),
        (1, 2, usize::from(expected_allocated > 0)),
        "the directory snapshot should retain a bounded supplemental file candidate"
    );
}

#[test]
fn analysis_exclusions_prune_results_and_invalidate_the_previous_snapshot() {
    let _operation_lock = crate::shared::operation::test_operation_lock();
    cache::clear_all().expect("clear the analysis cache before exclusion validation");
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "MangoDisk-Analysis-Exclusions-{}-{unique}",
        std::process::id()
    ));
    let _sandbox_cleanup = DirectoryCleanup(root.clone());
    let included = root.join("included");
    let excluded = root.join("excluded");
    fs::create_dir_all(&included).expect("create the included analysis directory");
    fs::create_dir_all(&excluded).expect("create the excluded analysis directory");
    fs::write(included.join("kept.bin"), vec![1_u8; 4096])
        .expect("write the included analysis fixture");
    fs::write(excluded.join("omitted.bin"), vec![2_u8; 8192])
        .expect("write the excluded analysis fixture");
    let root_value = current_platform().display_path(&root);
    let canonical_root = current_platform()
        .canonicalize_no_links(&root)
        .expect("canonicalize the analysis exclusion root");
    let canonical_excluded = current_platform()
        .canonicalize_no_links(&excluded)
        .expect("canonicalize the excluded analysis directory");
    let validated_exclusions = StorageScanExclusions::resolve(
        &canonical_root,
        &[current_platform().display_path(&excluded)],
    )
    .expect("resolve the analysis exclusion fixture");
    assert!(
        validated_exclusions.matches(&canonical_excluded),
        "resolved exclusions do not cover the fixture: {:?}",
        validated_exclusions.roots()
    );

    let complete = crate::AnalysisService::analyze_with_exclusions_progress(
        Some(root_value.clone()),
        true,
        Vec::new(),
        |_| {},
    )
    .expect("scan the complete fixture");
    assert!(complete
        .entries
        .iter()
        .any(|entry| entry.name == "excluded"));

    let (filtered, diagnostics) = StorageTraversal::analyze_path_with_exclusions_diagnostics(
        Some(root_value),
        false,
        vec![current_platform().display_path(&excluded)],
        |_| {},
    )
    .expect("rescan after the exclusion configuration changes");
    assert_ne!(
        diagnostics.fast_path, "cache",
        "a changed exclusion configuration must not reuse the previous snapshot"
    );
    assert!(filtered
        .entries
        .iter()
        .any(|entry| entry.name == "included"));
    assert!(
        filtered
            .entries
            .iter()
            .all(|entry| entry.name != "excluded"),
        "excluded analysis entry survived: {:?}",
        filtered
            .entries
            .iter()
            .map(|entry| (&entry.name, &entry.path, entry.bytes))
            .collect::<Vec<_>>()
    );
    assert!(filtered.total_bytes < complete.total_bytes);

    let cached = cache::analysis_result(&canonical_root)
        .expect("read the exclusion-aware analysis snapshot")
        .expect("the completed analysis snapshot should be cached");
    assert!(cached.entries.iter().all(|entry| entry.name != "excluded"));
}

#[test]
fn analysis_rejects_a_root_covered_by_an_exclusion() {
    let _operation_lock = crate::shared::operation::test_operation_lock();
    let fixture = tempfile::tempdir().expect("create an isolated analysis fixture");
    let root = fixture.path().join("excluded-root");
    fs::create_dir(&root).expect("create the excluded analysis root");
    fs::write(root.join("hidden.bin"), vec![1_u8; 4096]).expect("create excluded content");
    let path = current_platform().display_path(&root);

    for excluded in [
        path.clone(),
        current_platform().display_path(fixture.path()),
    ] {
        let error = crate::AnalysisService::analyze_with_exclusions_progress(
            Some(path.clone()),
            true,
            vec![excluded],
            |_| {},
        )
        .expect_err("an excluded analysis root should not reach filesystem traversal");

        assert_eq!(error.code(), crate::CoreErrorCode::InvalidInput);
        assert_eq!(
            error.reason(),
            Some(crate::CoreErrorReason::AnalysisRootExcluded)
        );
    }
}

#[test]
fn fast_analysis_contract_validates_record_counts_before_publish() {
    let root = Path::new("/fixture");
    let progress = Arc::new(ProgressTracker::new(1, |_| {}, 0));
    let cancelled = AtomicBool::new(false);
    let exclusions =
        StorageScanExclusions::resolve(root, &[]).expect("prepare empty analysis exclusions");
    let mut validation =
        FastAnalysisStreamValidation::new(root, 100, &progress, &cancelled, &exclusions);
    let mut sink = IndexRecordSink::memory(None);
    validation
        .consume(
            FastAnalysisRecord::Directory {
                retained_file_limit: 0,
                path: root.join("child"),
                logical_bytes: 5,
                allocated_bytes: 5,
                file_count: 1,
                direct_file_count: 1,
                skipped_count: 0,
            },
            &mut sink,
        )
        .expect("the child-directory record should be written");
    validation
        .consume(
            FastAnalysisRecord::Directory {
                retained_file_limit: 0,
                path: root.to_path_buf(),
                logical_bytes: 5,
                allocated_bytes: 5,
                file_count: 1,
                direct_file_count: 1,
                skipped_count: 0,
            },
            &mut sink,
        )
        .expect("the root-directory record should be written");
    validation
        .consume(
            FastAnalysisRecord::LargeFileCandidate(root.join("missing.bin")),
            &mut sink,
        )
        .expect("a candidate disappearing after enumeration should not invalidate directories");
    let mut summary = FastAnalysisSummary {
        root_logical_bytes: 5,
        root_allocated_bytes: 5,
        root_file_count: 1,
        root_skipped_count: 0,
        page_count: 1,
        entry_count: 3,
        directory_count: 1,
        candidate_count: 1,
        returned_bytes: 128,
        consumer_elapsed_ms: 0,
        strategy: "test",
    };

    assert!(
        validation.complete(&summary).is_err(),
        "a snapshot must not publish when directory records disagree with the summary"
    );
    summary.directory_count = 2;
    assert_eq!(
        validation
            .complete(&summary)
            .expect("a complete record stream should satisfy contract validation")
            .bytes,
        5
    );
}

#[test]
fn fast_analysis_progress_accumulates_batches_without_repeating_final_totals() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let captured = Arc::clone(&events);
    let progress = Arc::new(ProgressTracker::new(
        1,
        move |event| {
            captured
                .lock()
                .expect("the progress event lock should remain valid")
                .push(event)
        },
        0,
    ));
    let root = Path::new("/fixture");
    let mut validation = FastAnalysisProgressValidation::new(root, &progress);

    validation.observe(&root.join("first"), 2, 30);
    validation.observe(&root.join("second"), 3, 70);
    validation
        .complete(DirectoryAggregate {
            bytes: 100,
            file_count: 5,
            ..DirectoryAggregate::default()
        })
        .expect("matching progress batches should satisfy the final aggregate");
    progress.finish(TraversalStage::Analyzing, root);

    let events = events
        .lock()
        .expect("the progress event lock should remain valid");
    let final_event = events.last().expect("final progress should be published");
    assert_eq!(final_event.items_scanned, 5);
    assert_eq!(final_event.bytes_scanned, 100);
}

#[test]
fn memory_index_rejects_duplicate_stream_records() {
    let path = PathBuf::from("/fixture");
    let aggregate = DirectoryAggregate {
        bytes: 1,
        file_count: 1,
        ..DirectoryAggregate::default()
    };
    let mut sink = IndexRecordSink::memory(None);

    sink.push_directory(path.clone(), aggregate)
        .expect("the first directory record should be written");
    assert!(
        sink.push_directory(path, aggregate).is_err(),
        "an in-memory retry must not hide duplicate platform records by overwriting them"
    );
}

#[cfg(target_os = "macos")]
fn analyze_until_total_bytes(path: Option<String>, expected: u64) -> AnalysisResult {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let result = StorageTraversal::analyze_path_with_progress(path.clone(), false, |_| {})
            .expect("analysis should succeed after an FSEvents change");
        if result.total_bytes == expected {
            return result;
        }
        assert!(
                Instant::now() < deadline,
                "FSEvents did not invalidate the cache before the deadline: actual={} expected={expected}",
                result.total_bytes
            );
        // FSEvents delivery is asynchronous by design. Polling the real cache entry verifies
        // eventual monitor behavior without imposing the full timeout on fast machines.
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[cfg(target_os = "macos")]
#[test]
#[ignore = "requires an explicit real FSEvents cache-invalidation diagnostic"]
fn macos_file_create_modify_and_delete_invalidate_analysis_snapshot() {
    let _operation_lock = crate::shared::operation::test_operation_lock();
    cache::clear_all().expect("the memory cache should be cleared before the test");
    let root = std::env::temp_dir().join(format!(
        "mangodisk-fsevents-cache-{}-{}",
        std::process::id(),
        now_ms()
    ));
    let _sandbox_cleanup = DirectoryCleanup(root.clone());
    fs::create_dir_all(&root).expect("the FSEvents cache fixture should be created");
    fs::write(root.join("stable.bin"), [1_u8; 6])
        .expect("the initial fixture file should be written");
    let path = Some(root.to_string_lossy().into_owned());

    let initial = StorageTraversal::analyze_path_with_progress(path.clone(), true, |_| {})
        .expect("the initial analysis should succeed");
    assert_eq!(initial.total_bytes, 6);

    let changed = root.join("changed.bin");
    let file = fs::File::create(&changed).expect("the new fixture file should be created");
    file.set_len(4)
        .expect("the new fixture file size should be set");
    file.sync_all()
        .expect("the new fixture file should be synchronized");
    let after_create = analyze_until_total_bytes(path.clone(), 10);
    assert_eq!(after_create.total_bytes, 10);

    let file = fs::OpenOptions::new()
        .write(true)
        .open(&changed)
        .expect("the fixture file should open for modification");
    file.set_len(8)
        .expect("the fixture file size should be modified");
    file.sync_all()
        .expect("the modified fixture file should be synchronized");
    let after_modify = analyze_until_total_bytes(path.clone(), 14);
    assert_eq!(after_modify.total_bytes, 14);

    fs::remove_file(changed).expect("the fixture file should be removed");
    let after_delete = analyze_until_total_bytes(path, 6);
    assert_eq!(after_delete.total_bytes, 6);
}

#[test]
fn large_file_session_supports_switching_from_high_threshold_to_candidate_floor() {
    let root = std::env::temp_dir().join(format!(
        "mangodisk-large-file-cache-{}-{}",
        std::process::id(),
        now_ms()
    ));
    let files = [("60-mb.bin", 60), ("120-mb.bin", 120), ("600-mb.bin", 600)]
        .into_iter()
        .map(|(name, mebibytes)| {
            let bytes = mebibytes * 1024 * 1024;
            (
                root.join(name),
                IndexedFile {
                    shared_identity: None,
                    bytes,
                    logical_bytes: bytes,
                    modified_at_ms: None,
                },
            )
        })
        .collect::<std::collections::HashMap<_, _>>();
    let aggregate = DirectoryAggregate {
        bytes: files.values().map(|file| file.bytes).sum(),
        logical_bytes: files.values().map(|file| file.logical_bytes).sum(),
        file_count: files.len() as u64,
        scanned_at_ms: now_ms(),
        ..DirectoryAggregate::default()
    };
    let retained_entries = cache::large_file_entries_from_snapshot(&root, &files);
    let result = LargeFilesResult::from_retained_entries(
        vec![current_platform().display_path(&root)],
        aggregate.scanned_at_ms,
        LargeFileScanMode::Complete,
        500 * 1024 * 1024,
        0,
        retained_entries,
    );
    let high_threshold = result.filtered(500 * 1024 * 1024);
    assert_eq!(high_threshold.entries.len(), 1);

    let low_threshold = result.filtered(LARGE_FILE_CANDIDATE_FLOOR_BYTES);
    assert_eq!(low_threshold.entries.len(), 3);
}

/// Validates platform fast scanning and recursive fallback against a real directory selected
/// through the development environment variable. The test is ignored by default so CI cannot
/// traverse a large disk accidentally. Local runs must set `MANGODISK_ANALYSIS_ROOT` explicitly.
#[test]
#[ignore = "requires an explicit real scan root in MANGODISK_ANALYSIS_ROOT"]
fn real_large_file_scan_completes_fast_path_or_recursive_fallback() {
    let _operation_lock = crate::shared::operation::test_operation_lock();
    let root = std::env::var(ANALYSIS_ROOT_ENV)
        .expect("MANGODISK_ANALYSIS_ROOT must be set before a real large-file scan");
    let (result, diagnostics) = StorageTraversal::find_large_files_with_diagnostics(
        vec![root],
        LARGE_FILE_CANDIDATE_FLOOR_BYTES,
        LargeFileScanMode::Complete,
        vec![],
        |_| {},
    )
    .expect("the real large-file scan should succeed");

    println!(
            "real_large_file_scan results={} bytes={} skipped={} fast_path={} strategy={} candidates={} peak_in_flight={} discovery_ms={} validation_ms={}",
            result.total_count,
            result.total_bytes,
            result.skipped_count,
            diagnostics.fast_path,
            diagnostics.candidate_strategy,
            diagnostics.candidate_count,
            diagnostics.candidate_peak_in_flight,
            diagnostics.candidate_discovery_ms,
            diagnostics.validation_or_traversal_ms
        );
    assert!(result
        .entries
        .iter()
        .all(|entry| entry.bytes >= LARGE_FILE_CANDIDATE_FLOOR_BYTES));
    if diagnostics.fast_path == "used" {
        assert!(
            diagnostics.candidate_count >= result.total_count,
            "platform candidates must cover every valid result"
        );
    }
}

/// Exercises the explicit indexed mode against a real root. This diagnostic intentionally does
/// not compare it with complete mode because files created before Spotlight catches up are an
/// expected product distinction, not a correctness failure.
#[test]
#[ignore = "requires an explicit real scan root in MANGODISK_ANALYSIS_ROOT"]
fn real_quick_large_file_scan_uses_platform_index() {
    let _operation_lock = crate::shared::operation::test_operation_lock();
    let root = std::env::var(ANALYSIS_ROOT_ENV)
        .expect("MANGODISK_ANALYSIS_ROOT must be set before a real quick large-file scan");
    let (result, diagnostics) = StorageTraversal::find_large_files_with_diagnostics(
        vec![root],
        LARGE_FILE_CANDIDATE_FLOOR_BYTES,
        LargeFileScanMode::Quick,
        vec![],
        |_| {},
    )
    .expect("the platform index should serve the real quick scan");

    println!(
        "real_quick_large_file_scan results={} bytes={} skipped={} strategy={} candidates={} peak_in_flight={} discovery_ms={}",
        result.total_count,
        result.total_bytes,
        result.skipped_count,
        diagnostics.candidate_strategy,
        diagnostics.candidate_count,
        diagnostics.candidate_peak_in_flight,
        diagnostics.candidate_discovery_ms
    );
    assert_eq!(result.scan_mode, LargeFileScanMode::Quick);
    assert_eq!(diagnostics.fast_path, "used");
    assert!(!diagnostics.candidate_strategy.is_empty());
    if diagnostics.candidate_count > 0 {
        assert!(diagnostics.candidate_peak_in_flight > 0);
    }
}

/// Measures the complete in-memory analysis representation for a real directory tree.
#[test]
#[ignore = "requires an explicit real scan root in MANGODISK_ANALYSIS_ROOT"]
fn real_analysis_materializes_complete_memory_index() {
    let _operation_lock = crate::shared::operation::test_operation_lock();
    let root = std::env::var(ANALYSIS_ROOT_ENV)
        .expect("MANGODISK_ANALYSIS_ROOT must be set before real in-memory analysis");
    let canonical_root = current_platform()
        .canonicalize_no_links(Path::new(&root))
        .expect("the real scan root should be safely accessible");
    let progress = Arc::new(ProgressTracker::new(0, |_| {}, 0));
    let cancelled = AtomicBool::new(false);
    let started = Instant::now();
    let mut sink = IndexRecordSink::memory(None);
    let exclusions = StorageScanExclusions::resolve(&canonical_root, &[])
        .expect("the empty exclusion set should resolve");
    let (aggregate, summary) = stream_fast_analysis_once(
        &canonical_root,
        now_ms(),
        &progress,
        &cancelled,
        &exclusions,
        &mut sink,
    )
    .expect("the real in-memory analysis fast path should not fail")
    .expect("the platform must support native analysis for this diagnostic");
    let elapsed_ms = started.elapsed().as_millis();
    let CompletedIndexSink {
        directories, files, ..
    } = sink
        .finish()
        .expect("the in-memory analysis sink should finish");

    assert_eq!(directories.len() as u64, summary.directory_count);
    assert!(
        files.len() as u64 <= summary.candidate_count,
        "live allocation validation may discard logical-size candidates"
    );
    assert_eq!(aggregate.bytes, summary.root_allocated_bytes);
    println!(
        "real_analysis_memory strategy={} directories={} candidates={} bytes={} elapsed_ms={}",
        summary.strategy,
        directories.len(),
        files.len(),
        aggregate.bytes,
        elapsed_ms
    );

    // Keep both maps live through the final print so an external process monitor observes the
    // actual steady-state footprint instead of a value after Rust has already released the tree.
    std::hint::black_box((&directories, &files));
}

/// Complete large-file scans own their candidate snapshot and leave disk-analysis navigation
/// intact. This keeps scan ownership explicit without retaining millions of unrelated directory
/// aggregates in the large-file session.
#[test]
#[ignore = "requires a real MANGODISK_ANALYSIS_ROOT volume with change history"]
fn complete_large_file_scan_preserves_real_analysis_snapshot() {
    let _operation_lock = crate::shared::operation::test_operation_lock();
    let root = std::env::var(ANALYSIS_ROOT_ENV)
        .expect("MANGODISK_ANALYSIS_ROOT must be set before shared-snapshot validation");
    let canonical_root = current_platform()
        .canonicalize_no_links(Path::new(&root))
        .expect("the validation root should resolve");
    let (_, analysis_diagnostics) =
        StorageTraversal::analyze_path_with_diagnostics(Some(root.clone()), true, |_| {})
            .expect("the real analysis should succeed");
    let (large_files, large_diagnostics) = StorageTraversal::find_large_files_with_diagnostics(
        vec![root],
        LARGE_FILE_CANDIDATE_FLOOR_BYTES,
        LargeFileScanMode::Complete,
        vec![],
        |_| {},
    )
    .expect("the large-file query should succeed");

    assert_eq!(
        large_diagnostics.fast_path, "used",
        "a complete large-file request must perform its own candidate scan"
    );
    assert!(large_files
        .entries
        .iter()
        .all(|entry| entry.bytes >= LARGE_FILE_CANDIDATE_FLOOR_BYTES));
    assert!(
        cache::analysis_result(&canonical_root)
            .expect("the analysis cache should remain readable")
            .is_some(),
        "a large-file scan must not evict the independent disk-analysis snapshot"
    );
    println!(
            "independent_large_file_snapshot analysis_fast_path={} strategy={} large_files={} bytes={} result_build_ms={}",
            analysis_diagnostics.fast_path,
            analysis_diagnostics.strategy,
            large_files.total_count,
            large_files.total_bytes,
            large_diagnostics.result_build_ms
        );
}

#[cfg(target_os = "macos")]
#[test]
fn explicit_nested_filesystem_root_is_not_covered_by_its_parent() {
    let parent = Path::new("/");
    let mounted = Path::new("/dev");
    assert!(!current_platform().is_same_filesystem(
        &fs::metadata(parent).unwrap(),
        &fs::metadata(mounted).unwrap()
    ));
    let roots = normalize_large_file_roots(vec![
        parent.display().to_string(),
        mounted.display().to_string(),
    ])
    .unwrap();
    assert_eq!(
        roots.len(),
        2,
        "a mounted filesystem must remain an explicit scan root"
    );
}

#[cfg(unix)]
#[test]
fn analysis_charges_hard_links_once_with_stable_native_and_fallback_ownership() {
    let _operation_lock = crate::shared::operation::test_operation_lock();
    cache::clear_all().unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let root = current_platform()
        .canonicalize_no_links(fixture.path())
        .unwrap();
    fs::create_dir(root.join("a")).unwrap();
    fs::create_dir(root.join("b")).unwrap();
    let owner = root.join("a/owner.bin");
    fs::write(&owner, vec![3; 1024 * 1024]).unwrap();
    fs::hard_link(&owner, root.join("b/alias.bin")).unwrap();
    fs::hard_link(&owner, root.join("a/alias.bin")).unwrap();
    let allocation = current_platform()
        .file_space_usage(&owner, &fs::metadata(&owner).unwrap())
        .allocated_bytes;
    let root_text = current_platform().display_path(&root);
    for _ in 0..3 {
        let (result, diagnostics) =
            StorageTraversal::analyze_path_with_diagnostics(Some(root_text.clone()), true, |_| {})
                .unwrap();
        assert_eq!(diagnostics.fast_path, "used");
        assert_eq!(
            result
                .directory_hierarchy
                .iter()
                .find(|node| node.name == "a")
                .unwrap()
                .total_entry_count,
            1
        );
        let owner_node = result
            .directory_hierarchy
            .iter()
            .find(|node| node.name == "a")
            .unwrap();
        assert_eq!(owner_node.files.len(), 1);
        assert_eq!(owner_node.files[0].name, "alias.bin");
        assert_eq!(result.total_bytes, allocation);
        assert_eq!(
            result.entries.iter().map(|entry| entry.bytes).sum::<u64>(),
            allocation
        );
        assert_eq!(
            result
                .entries
                .iter()
                .find(|entry| entry.name == "a")
                .unwrap()
                .bytes,
            allocation
        );
        assert_eq!(
            result
                .entries
                .iter()
                .find(|entry| entry.name == "b")
                .unwrap()
                .bytes,
            0
        );
        let child = cache::analysis_result(&root.join("a")).unwrap().unwrap();
        assert_eq!(
            child.entries.iter().map(|entry| entry.bytes).sum::<u64>(),
            allocation
        );
        assert_eq!(
            child.entries.iter().filter(|entry| entry.bytes > 0).count(),
            1
        );
    }
    let progress = Arc::new(ProgressTracker::new(0, |_| {}, 0));
    let (aggregate, snapshot) = traverse_memory_only(
        &root,
        TraversalKind::Analysis(AnalysisScanMode::Standard),
        now_ms(),
        None,
        &progress,
        &AtomicBool::new(false),
        None,
    )
    .unwrap();
    assert_eq!(aggregate.bytes, allocation);
    assert_eq!(aggregate.file_count, 3);
    assert_eq!(snapshot.directories[&root.join("a")].direct_file_count, 1);
    assert_eq!(snapshot.directories[&root.join("b")].direct_file_count, 0);
    assert_eq!(aggregate.logical_bytes, 3 * 1024 * 1024);
    assert_eq!(snapshot.files[&root.join("a/owner.bin")].bytes, 0);
    assert_eq!(snapshot.files[&root.join("b/alias.bin")].bytes, 0);
    let fallback = cache::analysis_result_from_snapshot(
        &root,
        aggregate,
        &snapshot.directories,
        &snapshot.files,
        &[],
        &mangodisk_platform::NameExclusions::default(),
        1,
    )
    .unwrap();
    let native = cache::analysis_result(&root).unwrap().unwrap();
    let ownership = |result: &AnalysisResult| {
        result
            .shared_allocations
            .iter()
            .map(|group| {
                (
                    group.owner.clone(),
                    group.bytes,
                    group
                        .files
                        .iter()
                        .map(|file| (file.path.clone(), file.bytes))
                        .collect::<Vec<_>>(),
                )
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(ownership(&fallback), ownership(&native));
    assert_eq!(
        fallback.shared_directories.len(),
        native.shared_directories.len()
    );

    let outside = tempfile::tempdir().unwrap();
    fs::hard_link(&owner, outside.path().join("outside.bin")).unwrap();
    // A link outside the selected root does not suppress the allocation inside it.
    let child = StorageTraversal::analyze_path_with_diagnostics(
        Some(current_platform().display_path(&root.join("b"))),
        true,
        |_| {},
    )
    .unwrap()
    .0;
    assert_eq!(child.total_bytes, allocation);
    // A child refresh must evict the ancestor rather than add shared allocation a second time.
    assert!(cache::analysis_result(&root).unwrap().is_none());
    let parent = StorageTraversal::analyze_path_with_diagnostics(Some(root_text), false, |_| {})
        .unwrap()
        .0;
    assert_eq!(parent.total_bytes, allocation);
    // Deleting the charged alias requires a full snapshot rebuild, so the surviving link owns it.
    fs::remove_file(root.join("a/alias.bin")).unwrap();
    cache::remove_entry(
        &root.join("a/alias.bin"),
        mangodisk_platform::FileSpaceUsage {
            allocated_bytes: allocation,
            logical_bytes: 1024 * 1024,
        },
        1,
        false,
    );
    assert!(cache::analysis_result(&root).unwrap().is_none());
    let refreshed = StorageTraversal::analyze_path_with_diagnostics(
        Some(current_platform().display_path(&root)),
        false,
        |_| {},
    )
    .unwrap()
    .0;
    assert_eq!(refreshed.total_bytes, allocation);
    cache::clear_all().unwrap();
}

#[test]
fn hierarchy_counts_small_direct_items_without_recursive_descendants_or_empty_files() {
    let _operation_lock = crate::shared::operation::test_operation_lock();
    cache::clear_all().unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let root = current_platform()
        .canonicalize_no_links(fixture.path())
        .unwrap();
    let branch = root.join("branch");
    fs::create_dir_all(branch.join("child")).unwrap();
    fs::create_dir(branch.join("empty-dir")).unwrap();
    fs::write(branch.join("large.bin"), vec![5; 1024 * 1024]).unwrap();
    fs::write(branch.join("empty.bin"), []).unwrap();
    for index in 0..80 {
        fs::write(branch.join(format!("small-{index}")), [3; 32]).unwrap();
        fs::write(
            branch.join("child").join(format!("nested-{index}")),
            [4; 32],
        )
        .unwrap();
    }
    let result = StorageTraversal::analyze_path_with_progress(
        Some(current_platform().display_path(&root)),
        true,
        |_| {},
    )
    .unwrap();
    let node = result
        .directory_hierarchy
        .iter()
        .find(|node| node.name == "branch")
        .unwrap();
    assert_eq!(
        node.total_entry_count, 82,
        "81 nonempty direct files plus one nonempty child directory"
    );
    assert!(node.files.len() + node.children.len() <= 64);
    let child = cache::analysis_result(&branch).unwrap().unwrap();
    assert_eq!(node.total_entry_count, child.total_entry_count as u64);
    let progress = Arc::new(ProgressTracker::new(0, |_| {}, 0));
    let (_, snapshot) = traverse_memory_only(
        &root,
        TraversalKind::Analysis(AnalysisScanMode::Standard),
        now_ms(),
        None,
        &progress,
        &AtomicBool::new(false),
        None,
    )
    .unwrap();
    assert_eq!(snapshot.directories[&branch].direct_file_count, 81);
    assert_eq!(
        snapshot.directories[&branch.join("child")].direct_file_count,
        80
    );
}

#[test]
fn hierarchy_projects_medium_files_below_large_file_floor_in_native_and_fallback_scans() {
    let _operation_lock = crate::shared::operation::test_operation_lock();
    cache::clear_all().unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let root = current_platform()
        .canonicalize_no_links(fixture.path())
        .unwrap();
    let branch = root.join("branch");
    fs::create_dir(&branch).unwrap();
    for index in 0..96 {
        fs::write(
            branch.join(format!("file-{index:03}")),
            vec![3; 256 * 1024 + index * 4096],
        )
        .unwrap();
    }
    let result = StorageTraversal::analyze_path_with_progress(
        Some(current_platform().display_path(&root)),
        true,
        |_| {},
    )
    .unwrap();
    let node = result
        .directory_hierarchy
        .iter()
        .find(|node| node.name == "branch")
        .unwrap();
    assert_eq!(
        node.files.len(),
        64,
        "medium files must be available before area filtering"
    );
    assert_eq!(node.total_entry_count, 96);
    assert_eq!(node.files[0].name, "file-095");
    assert_eq!(node.files[63].name, "file-032");
    assert!(node
        .files
        .iter()
        .all(|file| file.bytes < LARGE_FILE_CANDIDATE_FLOOR_BYTES));
    let progress = Arc::new(ProgressTracker::new(0, |_| {}, 0));
    let (aggregate, snapshot) = traverse_memory_only(
        &root,
        TraversalKind::Analysis(AnalysisScanMode::Standard),
        now_ms(),
        None,
        &progress,
        &AtomicBool::new(false),
        None,
    )
    .unwrap();
    let fallback = cache::analysis_result_from_snapshot(
        &root,
        aggregate,
        &snapshot.directories,
        &snapshot.files,
        &[],
        &mangodisk_platform::NameExclusions::default(),
        1,
    )
    .unwrap();
    let fallback_node = fallback
        .directory_hierarchy
        .iter()
        .find(|node| node.name == "branch")
        .unwrap();
    assert_eq!(fallback_node.files.len(), 64);
    assert_eq!(fallback_node.files[0].name, "file-095");
    assert_eq!(fallback_node.total_entry_count, 96);
    assert_eq!(fallback.total_bytes, result.total_bytes);
    cache::clear_all().unwrap();
}
