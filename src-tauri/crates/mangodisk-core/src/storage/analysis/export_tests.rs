use super::*;
use crate::{storage::analysis::AnalysisService, ScanExclusionOptions};

fn analyze(root: &Path, refresh: bool) -> super::super::AnalysisResult {
    AnalysisService::analyze_with_exclusions_progress(
        Some(root.to_string_lossy().into_owned()),
        refresh,
        ScanExclusionOptions::default(),
        |_| {},
    )
    .unwrap()
}

fn fixture() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

#[test]
fn export_keeps_all_directory_levels_and_zero_byte_rows_beyond_gui_limits() {
    let _lock = crate::shared::operation::test_operation_lock();
    let root = fixture();
    for i in 0..550 {
        std::fs::create_dir(root.path().join(format!("folder-{i:03}"))).unwrap();
    }
    let deep = root.path().join("deep/a/b/c/d/e/f/g/h/i");
    std::fs::create_dir_all(&deep).unwrap();
    std::fs::write(
        deep.join("private-file-name.txt"),
        b"private contents must never be exported",
    )
    .unwrap();
    let result = analyze(root.path(), true);
    let report = prepare(
        result.scan_id,
        AnalysisExportScope::ScanRoot,
        false,
        AnalysisExportDetail::Full,
        0,
    )
    .unwrap();
    let value = serde_json::to_value(&report).unwrap();
    assert_eq!(value["schemaVersion"], 4);
    assert_eq!(
        value["root"],
        root.path().file_name().unwrap().to_str().unwrap()
    );
    assert!(report
        .directories
        .iter()
        .all(|row| Path::new(&row.path).is_relative()));
    assert_eq!(value["directories"][0]["bytes"], result.total_bytes);
    let object = value.as_object().unwrap();
    assert_eq!(object.len(), 5, "only essential report fields are exported");
    assert!(object.contains_key("byteMetric"));
    let row = value["directories"][0].as_object().unwrap();
    assert_eq!(
        row.len(),
        3,
        "ordinary rows contain only path, bytes and fileCount"
    );
    assert_eq!(report.directories.len(), 561);
    assert!(report
        .directories
        .iter()
        .any(|row| row.path == "deep/a/b/c/d/e/f/g/h/i"));
    assert!(report
        .directories
        .iter()
        .any(|row| row.path == "folder-549" && row.bytes == 0));
    let json = serde_json::to_string(&report).unwrap();
    assert!(!json.contains("private-file-name"));
    assert!(!json.contains("private contents"));
    assert!(!json.contains(&root.path().to_string_lossy().to_string()));
}

#[test]
fn export_places_root_totals_before_punctuation_named_directories() {
    let _lock = crate::shared::operation::test_operation_lock();
    let root = fixture();
    std::fs::create_dir(root.path().join("!cache")).unwrap();
    std::fs::create_dir(root.path().join("-downloads")).unwrap();
    std::fs::write(root.path().join("!cache/data.bin"), b"sample").unwrap();
    let result = analyze(root.path(), true);
    for full_paths in [false, true] {
        let report = prepare(
            result.scan_id,
            AnalysisExportScope::ScanRoot,
            full_paths,
            AnalysisExportDetail::Full,
            0,
        )
        .unwrap();
        let expected = if full_paths {
            result.root.as_str()
        } else {
            "."
        };
        assert_eq!(report.directories[0].path, expected);
        assert_eq!(report.directories[0].bytes, result.total_bytes);
    }
}

#[test]
fn child_export_uses_relative_paths_and_original_scan_accounting() {
    let _lock = crate::shared::operation::test_operation_lock();
    let root = fixture();
    let child = root.path().join("Downloads");
    std::fs::create_dir_all(child.join("archives")).unwrap();
    std::fs::write(child.join("archives/item.bin"), b"sample").unwrap();
    let parent = analyze(root.path(), true);
    let result = analyze(&child, false);
    let options = options(result.scan_id).unwrap();
    assert_eq!(options.scan_root.path, parent.root);
    assert_eq!(options.current_directory.path, result.root);
    let report = prepare(
        result.scan_id,
        AnalysisExportScope::CurrentDirectory,
        false,
        AnalysisExportDetail::Full,
        0,
    )
    .unwrap();
    let value = serde_json::to_value(&report).unwrap();
    assert_eq!(value["root"], "Downloads");
    assert_eq!(value["subtreeOfOriginalScan"], true);
    assert_eq!(value["directories"][0]["bytes"], result.total_bytes);
    assert_eq!(report.directories.len(), 2);
    assert_eq!(report.directories[0].path, ".");
    assert_eq!(report.directories[1].path, "archives");
    let full = prepare(
        result.scan_id,
        AnalysisExportScope::CurrentDirectory,
        true,
        AnalysisExportDetail::Full,
        0,
    )
    .unwrap();
    assert_eq!(serde_json::to_value(full).unwrap()["root"], result.root);
}

#[test]
fn snapshot_export_never_reads_the_source_again_or_rescans_it() {
    let _lock = crate::shared::operation::test_operation_lock();
    let root = fixture();
    let source = root.path().join("source");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(source.join("file.bin"), b"snapshot").unwrap();
    let result = analyze(&source, true);
    std::fs::remove_dir_all(&source).unwrap();
    let report = prepare(
        result.scan_id,
        AnalysisExportScope::ScanRoot,
        false,
        AnalysisExportDetail::Full,
        0,
    )
    .unwrap();
    let destination = root.path().join("analysis.json");
    let receipt = report.save(&destination).unwrap();
    let bytes = std::fs::read(&destination).unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(receipt.file_bytes, bytes.len() as u64);
    assert_eq!(value["directories"][0]["bytes"], result.total_bytes);
    assert_eq!(value["scannedAtMs"], result.scanned_at_ms);
    assert!(!source.exists());
}

#[test]
fn expired_or_replaced_indexes_cannot_export_an_old_result() {
    let _lock = crate::shared::operation::test_operation_lock();
    let root = fixture();
    let original = analyze(root.path(), true);
    cache::clear_all().unwrap();
    assert_eq!(
        options(original.scan_id).unwrap_err().reason(),
        Some(CoreErrorReason::AnalysisExportUnavailable)
    );
    let replacement = analyze(root.path(), true);
    assert!(options(replacement.scan_id).is_ok());
    assert!(options(original.scan_id).is_err());
    assert!(options(u64::MAX).is_err());
}

#[test]
fn newer_overlapping_index_is_rejected_even_with_unchanged_totals() {
    let _lock = crate::shared::operation::test_operation_lock();
    let root = fixture();
    std::fs::create_dir(root.path().join("child")).unwrap();
    let parent = analyze(root.path(), true);
    let child = analyze(&root.path().join("child"), true);
    assert_eq!(parent.total_bytes, child.total_bytes);
    assert!(options(parent.scan_id).is_err());
    assert!(options(child.scan_id).is_ok());
}

#[test]
fn failed_publication_preserves_destination_and_removes_temporary_report() {
    let _lock = crate::shared::operation::test_operation_lock();
    let root = fixture();
    let result = analyze(root.path(), true);
    let report = prepare(
        result.scan_id,
        AnalysisExportScope::ScanRoot,
        false,
        AnalysisExportDetail::Full,
        0,
    )
    .unwrap();
    let output = fixture();
    let destination = output.path().join("existing");
    std::fs::create_dir(&destination).unwrap();
    std::fs::write(destination.join("keep.txt"), b"keep").unwrap();
    assert_eq!(
        report.save(&destination).unwrap_err().code(),
        crate::CoreErrorCode::Persistence
    );
    assert_eq!(
        std::fs::read(destination.join("keep.txt")).unwrap(),
        b"keep"
    );
    assert_eq!(std::fs::read_dir(output.path()).unwrap().count(), 1);
    let file = output.path().join("report.json");
    std::fs::write(&file, b"old report").unwrap();
    report.save(&file).unwrap();
    assert!(serde_json::from_slice::<serde_json::Value>(&std::fs::read(file).unwrap()).is_ok());
}

#[test]
fn exclusions_and_missing_data_are_exported_without_private_path_prefixes() {
    let _lock = crate::shared::operation::test_operation_lock();
    let root = fixture();
    let excluded = root.path().join("excluded");
    std::fs::create_dir(&excluded).unwrap();
    let result = AnalysisService::analyze_with_exclusions_progress(
        Some(root.path().to_string_lossy().into_owned()),
        true,
        ScanExclusionOptions {
            paths: vec![excluded.to_string_lossy().into_owned()],
            names: Vec::new(),
        },
        |_| {},
    )
    .unwrap();
    let report = prepare(
        result.scan_id,
        AnalysisExportScope::ScanRoot,
        false,
        AnalysisExportDetail::Full,
        0,
    )
    .unwrap();
    let value = serde_json::to_value(&report).unwrap();
    assert_eq!(
        value["exclusions"]["paths"],
        serde_json::json!(["excluded"])
    );
    assert!(
        value["directories"][0]["omissions"]["excluded"]
            .as_u64()
            .unwrap()
            > 0
    );
    assert_eq!(
        value["directories"][0]["omissions"]
            .as_object()
            .unwrap()
            .len(),
        1
    );
    assert!(!serde_json::to_string(&report)
        .unwrap()
        .contains(&root.path().to_string_lossy().to_string()));
}

#[test]
fn report_serialization_preserves_integers_beyond_javascript_precision() {
    let _lock = crate::shared::operation::test_operation_lock();
    let root = fixture();
    let result = analyze(root.path(), true);
    let mut report = prepare(
        result.scan_id,
        AnalysisExportScope::ScanRoot,
        false,
        AnalysisExportDetail::Full,
        0,
    )
    .unwrap();
    report.scan.total_bytes = 9_007_199_254_740_993;
    report.directories[0].bytes = report.scan.total_bytes;
    let json = serde_json::to_string(&report).unwrap();
    assert!(json.contains("\"bytes\":9007199254740993"));
}

#[test]
fn shared_hard_link_allocation_keeps_gui_totals_without_double_charging() {
    let _lock = crate::shared::operation::test_operation_lock();
    let root = fixture();
    std::fs::create_dir(root.path().join("A")).unwrap();
    std::fs::create_dir(root.path().join("B")).unwrap();
    let first = root.path().join("A/data.bin");
    std::fs::write(&first, vec![7; 8192]).unwrap();
    std::fs::hard_link(first, root.path().join("B/alias.bin")).unwrap();
    let result = analyze(root.path(), true);
    let report = prepare(
        result.scan_id,
        AnalysisExportScope::ScanRoot,
        false,
        AnalysisExportDetail::Full,
        0,
    )
    .unwrap();
    assert_eq!(report.scan.total_bytes, result.total_bytes);
    assert_eq!(report.directories[0].file_count, 2);
    let children: u64 = report
        .directories
        .iter()
        .filter(|row| row.path != "." && !row.path.contains('/'))
        .map(|row| row.bytes)
        .sum();
    assert!(children <= result.total_bytes);
    assert_eq!(report.directories.len(), 3);
}

#[cfg(windows)]
#[test]
fn windows_fast_export_declares_logical_bytes_instead_of_disk_usage() {
    let _lock = crate::shared::operation::test_operation_lock();
    let root = fixture();
    std::fs::write(root.path().join("small.bin"), b"logical").unwrap();
    let result = AnalysisService::analyze_with_mode_progress(
        Some(root.path().to_string_lossy().into_owned()),
        true,
        ScanExclusionOptions::default(),
        AnalysisScanMode::Fast,
        |_| {},
    )
    .unwrap();
    let report = prepare(
        result.scan_id,
        AnalysisExportScope::ScanRoot,
        false,
        AnalysisExportDetail::Full,
        0,
    )
    .unwrap();
    let json = serde_json::to_value(report).unwrap();
    assert_eq!(json["byteMetric"], "logicalBytes");
    assert_eq!(json["directories"][0]["bytes"], 7);
}

#[test]
fn summary_export_preserves_full_scan_statistics_and_exclusions_without_rescanning() {
    let _lock = crate::shared::operation::test_operation_lock();
    let root = fixture();
    let source = root.path().join("source");
    std::fs::create_dir_all(source.join("excluded")).unwrap();
    for index in 0..550 {
        std::fs::create_dir(source.join(format!("folder-{index:03}"))).unwrap();
    }
    std::fs::write(source.join("direct.bin"), b"direct data").unwrap();
    std::fs::write(source.join("folder-000/child.bin"), b"child data").unwrap();
    let result = AnalysisService::analyze_with_exclusions_progress(
        Some(source.to_string_lossy().into_owned()),
        true,
        ScanExclusionOptions {
            paths: vec![source.join("excluded").to_string_lossy().into_owned()],
            names: Vec::new(),
        },
        |_| {},
    )
    .unwrap();
    std::fs::remove_dir_all(&source).unwrap();
    let full = serde_json::to_value(
        prepare(
            result.scan_id,
            AnalysisExportScope::ScanRoot,
            false,
            AnalysisExportDetail::Full,
            0,
        )
        .unwrap(),
    )
    .unwrap();
    for full_paths in [false, true] {
        let summary = prepare(
            result.scan_id,
            AnalysisExportScope::ScanRoot,
            full_paths,
            AnalysisExportDetail::Summary,
            0,
        )
        .unwrap();
        assert_eq!(summary.directories.len(), 500);
        let destination = root.path().join("summary.json");
        summary.save(&destination).unwrap();
        let value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(destination).unwrap()).unwrap();
        for key in ["bytes", "fileCount", "sizeStatus", "omissions"] {
            assert_eq!(
                value["directories"][0][key], full["directories"][0][key],
                "root {key} is unchanged by folding"
            );
        }
        for key in ["byteMetric", "scannedAtMs"] {
            assert_eq!(value[key], full[key]);
        }
        let collapsed = &value["directories"][0]["collapsed"];
        assert_eq!(
            collapsed["directoryCount"],
            full["directories"].as_array().unwrap().len() - 500
        );
        assert_eq!(
            collapsed["bytes"], 0,
            "zero-byte directories are folded explicitly"
        );
        assert_eq!(value["directories"][0]["omissions"]["excluded"], 1);
        if !full_paths {
            assert_eq!(value["exclusions"], full["exclusions"]);
            assert!(!serde_json::to_string(&value)
                .unwrap()
                .contains(&source.to_string_lossy().to_string()));
        }
        assert!(value.get("scannedAtRangeMs").is_none());
        assert!(!source.exists());
    }
}

#[test]
fn summary_preserves_measurement_range_when_older_rows_are_folded() {
    let _lock = crate::shared::operation::test_operation_lock();
    let root = fixture();
    for index in 0..550 {
        std::fs::create_dir(root.path().join(format!("folder-{index:03}"))).unwrap();
    }
    let result = analyze(root.path(), true);
    let older = result.scanned_at_ms - 1;
    let (native_root, mut directories, generation, revision) =
        with_snapshot(result.scan_id, |cache, _, root, _| {
            Ok((
                root.to_path_buf(),
                cache
                    .directories
                    .iter()
                    .filter(|(path, _)| path.starts_with(root))
                    .map(|(path, aggregate)| (path.clone(), *aggregate))
                    .collect::<std::collections::HashMap<_, _>>(),
                latest_generation(cache, root).ok_or_else(unavailable)?,
                cache.mutation_revision,
            ))
        })
        .unwrap();
    let root = native_root.as_path();
    directories
        .get_mut(&root.join("folder-549"))
        .unwrap()
        .scanned_at_ms = older;
    cache::store_memory_only(
        root,
        directories[root],
        directories.clone(),
        Default::default(),
        cache::SnapshotPublication::new(
            mangodisk_platform::ScanPurpose::Analysis,
            true,
            None,
            generation,
            revision,
        ),
    )
    .unwrap();
    let summary = serde_json::to_value(
        prepare(
            result.scan_id,
            AnalysisExportScope::ScanRoot,
            false,
            AnalysisExportDetail::Summary,
            0,
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        summary["scannedAtRangeMs"],
        serde_json::json!([older, result.scanned_at_ms])
    );
    assert!(summary["directories"]
        .as_array()
        .unwrap()
        .iter()
        .all(|row| row["path"] != "folder-549"));
    let filtered = serde_json::to_value(
        prepare(
            result.scan_id,
            AnalysisExportScope::ScanRoot,
            false,
            AnalysisExportDetail::Full,
            1,
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(filtered["scannedAtRangeMs"], summary["scannedAtRangeMs"]);
    assert_eq!(
        filtered["directories"][0]["filtered"]["directoryCount"],
        550
    );
    assert!(filtered["directories"][0].get("omissions").is_none());
    let full = serde_json::to_value(
        prepare(
            result.scan_id,
            AnalysisExportScope::ScanRoot,
            false,
            AnalysisExportDetail::Full,
            0,
        )
        .unwrap(),
    )
    .unwrap();
    assert!(full.get("scannedAtRangeMs").is_none());
    assert_eq!(
        full["directories"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["path"] == "folder-549")
            .unwrap()["scannedAtMs"],
        older
    );
}

#[test]
fn both_export_modes_filter_existing_results_without_changing_scan_totals_or_omissions() {
    let _lock = crate::shared::operation::test_operation_lock();
    let temp = fixture();
    let source = temp.path().join("source");
    std::fs::create_dir_all(source.join("measured")).unwrap();
    std::fs::create_dir(source.join("excluded")).unwrap();
    for index in 0..600 {
        std::fs::create_dir(source.join(format!("empty-{index:03}"))).unwrap();
    }
    std::fs::write(source.join("measured/private.bin"), vec![5; 8192]).unwrap();
    std::fs::write(source.join("direct.bin"), b"direct").unwrap();
    let result = AnalysisService::analyze_with_exclusions_progress(
        Some(source.to_string_lossy().into_owned()),
        true,
        ScanExclusionOptions {
            paths: vec![source.join("excluded").to_string_lossy().into_owned()],
            names: Vec::new(),
        },
        |_| {},
    )
    .unwrap();
    let baseline = serde_json::to_value(
        prepare(
            result.scan_id,
            AnalysisExportScope::ScanRoot,
            false,
            AnalysisExportDetail::Full,
            0,
        )
        .unwrap(),
    )
    .unwrap();
    std::fs::remove_dir_all(&source).unwrap();
    for detail in [AnalysisExportDetail::Summary, AnalysisExportDetail::Full] {
        for absolute in [false, true] {
            let report = prepare(
                result.scan_id,
                AnalysisExportScope::ScanRoot,
                absolute,
                detail,
                1,
            )
            .unwrap();
            let destination = temp.path().join("filtered.json");
            report.save(&destination).unwrap();
            let value: serde_json::Value =
                serde_json::from_slice(&std::fs::read(destination).unwrap()).unwrap();
            assert_eq!(value["schemaVersion"], 4);
            assert_eq!(value["minDirectoryBytes"], 1);
            assert_eq!(value["byteMetric"], baseline["byteMetric"]);
            assert_eq!(value["scannedAtMs"], baseline["scannedAtMs"]);
            for key in ["bytes", "fileCount", "sizeStatus", "omissions"] {
                assert_eq!(
                    value["directories"][0][key], baseline["directories"][0][key],
                    "{key} is unchanged by filtering"
                );
            }
            let rows = value["directories"].as_array().unwrap();
            assert!(rows.len() < 10, "known empty directories are filtered");
            assert!(rows.iter().all(|row| row.get("collapsed").is_none()));
            let filtered: usize = rows
                .iter()
                .map(|row| row["filtered"]["directoryCount"].as_u64().unwrap_or(0) as usize)
                .sum();
            assert_eq!(
                filtered + rows.len(),
                baseline["directories"].as_array().unwrap().len()
            );
            assert_eq!(value["directories"][0]["omissions"]["excluded"], 1);
            if !absolute {
                assert_eq!(value["exclusions"], baseline["exclusions"]);
                assert!(!serde_json::to_string(&value)
                    .unwrap()
                    .contains(&source.to_string_lossy().to_string()));
            }
            assert!(!source.exists());
        }
    }
    let root_only = serde_json::to_value(
        prepare(
            result.scan_id,
            AnalysisExportScope::ScanRoot,
            false,
            AnalysisExportDetail::Full,
            u64::MAX,
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(root_only["minDirectoryBytes"].as_u64(), Some(u64::MAX));
    assert_eq!(root_only["directories"][0]["bytes"], result.total_bytes);
    assert_eq!(root_only["directories"][0]["fileCount"], 2);
    assert!(baseline.get("minDirectoryBytes").is_none());
}

#[cfg(target_os = "linux")]
#[test]
fn linux_exports_keep_known_omissions_distinct_from_size_filtering() {
    use std::os::unix::fs::{symlink, PermissionsExt};

    let _lock = crate::shared::operation::test_operation_lock();
    let root = fixture();
    let denied = root.path().join("denied");
    let small = root.path().join("small");
    std::fs::create_dir(&denied).unwrap();
    std::fs::create_dir(&small).unwrap();
    std::fs::write(denied.join("private.bin"), b"not scanned").unwrap();
    std::fs::write(small.join("item.bin"), b"small").unwrap();
    std::fs::write(root.path().join("direct.bin"), b"direct").unwrap();
    symlink(&small, root.path().join("link")).unwrap();
    std::fs::set_permissions(&denied, std::fs::Permissions::from_mode(0o000)).unwrap();
    let result = AnalysisService::analyze_with_exclusions_progress(
        Some(root.path().to_string_lossy().into_owned()),
        true,
        ScanExclusionOptions::default(),
        |_| {},
    );
    std::fs::set_permissions(&denied, std::fs::Permissions::from_mode(0o700)).unwrap();
    let result = result.unwrap();
    assert_eq!(result.coverage.omissions.permission_denied, 1);
    assert_eq!(result.coverage.omissions.link_or_mount, 1);
    assert_eq!(result.coverage.omissions.other, 0);
    for detail in [AnalysisExportDetail::Summary, AnalysisExportDetail::Full] {
        let value = serde_json::to_value(
            prepare(
                result.scan_id,
                AnalysisExportScope::ScanRoot,
                false,
                detail,
                u64::MAX,
            )
            .unwrap(),
        )
        .unwrap();
        let rows = value["directories"].as_array().unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["bytes"], result.total_bytes);
        assert_eq!(rows[0]["fileCount"], 2);
        assert_eq!(
            rows[0]["omissions"],
            serde_json::json!({"permissionDenied": 1, "linkOrMount": 1})
        );
        assert_eq!(rows[0]["filtered"]["directoryCount"], 1);
        assert_eq!(rows[1]["path"], "denied");
        assert_eq!(rows[1]["sizeStatus"], "unavailable");
        assert_eq!(rows[1]["omissions"]["permissionDenied"], 1);
    }
}
