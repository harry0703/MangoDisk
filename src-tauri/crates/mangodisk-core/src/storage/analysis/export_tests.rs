use super::*;
use crate::{
    storage::{analysis::AnalysisService, index::cache},
    ScanExclusionOptions,
};
use serde_json::Value;
use std::path::Path;

fn analyze(root: &Path) -> super::super::AnalysisResult {
    AnalysisService::analyze_with_exclusions_progress(
        Some(root.to_string_lossy().into_owned()),
        true,
        ScanExclusionOptions::default(),
        |_| {},
    )
    .unwrap()
}
fn request(scan_id: u64, depth: u8, limit: usize) -> AnalysisExportRequest {
    AnalysisExportRequest {
        scan_id,
        max_depth: depth,
        max_entries: limit,
        include_full_paths: false,
        format: AnalysisExportFormat::Json,
    }
}
fn json(scan_id: u64, depth: u8, limit: usize) -> Value {
    serde_json::to_value(prepare(request(scan_id, depth, limit)).unwrap()).unwrap()
}
fn child<'a>(node: &'a Value, name: &str) -> &'a Value {
    node["children"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["name"] == name)
        .unwrap()
}
fn count(node: &Value) -> usize {
    node.get("children")
        .and_then(Value::as_array)
        .map_or(0, |rows| rows.iter().map(|row| 1 + count(row)).sum())
}
fn verify_totals(node: &Value) {
    if node["isDirectory"] != true {
        assert!(node.get("children").is_none());
        return;
    }
    let mut bytes = 0_u64;
    let mut files = 0_u64;
    if let Some(children) = node.get("children").and_then(Value::as_array) {
        for child in children {
            bytes += child["bytes"].as_u64().unwrap();
            files += if child["isDirectory"] == true {
                child["fileCount"].as_u64().unwrap()
            } else {
                1
            };
            verify_totals(child);
        }
    }
    if let Some(remaining) = node.get("remaining").and_then(Value::as_object) {
        for bucket in remaining.values() {
            bytes += bucket["bytes"].as_u64().unwrap();
            files += bucket["fileCount"].as_u64().unwrap();
        }
    }
    assert_eq!(
        node["bytes"].as_u64(),
        Some(bytes),
        "byte accounting for {}",
        node["name"]
    );
    assert_eq!(
        node["fileCount"].as_u64(),
        Some(files),
        "file accounting for {}",
        node["name"]
    );
}

#[test]
fn nested_export_reuses_child_file_records_and_preserves_every_parent_total() {
    let _lock = crate::shared::operation::test_operation_lock();
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("folder/deep")).unwrap();
    std::fs::write(root.path().join("root.bin"), vec![1; 8192]).unwrap();
    std::fs::write(root.path().join("folder/visible.bin"), vec![2; 4096]).unwrap();
    std::fs::write(
        root.path().join("folder/deep/secret.bin"),
        b"private content",
    )
    .unwrap();
    let result = analyze(root.path());
    let value = json(result.scan_id, 2, 500);
    assert_eq!(value["schemaVersion"], 5);
    assert_eq!(value["byteMetric"], "allocatedBytes");
    assert_eq!(value["root"]["bytes"], result.total_bytes);
    assert_eq!(child(&value["root"], "root.bin")["isDirectory"], false);
    let folder = child(&value["root"], "folder");
    assert_eq!(child(folder, "visible.bin")["isDirectory"], false);
    let deep = child(folder, "deep");
    assert!(deep.get("children").is_none());
    assert!(
        deep["remaining"]["depthLimit"]["fileCount"]
            .as_u64()
            .unwrap()
            > 0
    );
    verify_totals(&value["root"]);
    let text = serde_json::to_string(&value).unwrap();
    for forbidden in [
        "secret.bin",
        "private content",
        "exportedAt",
        "fingerprint",
        "deletionProtection",
        &root.path().to_string_lossy(),
    ] {
        assert!(!text.contains(forbidden), "unexpected detail: {forbidden}");
    }
    let full = serde_json::to_value(
        prepare(AnalysisExportRequest {
            include_full_paths: true,
            ..request(result.scan_id, 2, 500)
        })
        .unwrap(),
    )
    .unwrap();
    assert_eq!(full["root"]["path"], result.root);
    assert!(Path::new(child(&full["root"], "folder")["path"].as_str().unwrap()).is_absolute());
}

#[test]
fn depths_one_through_six_have_exact_boundaries_including_files() {
    let _lock = crate::shared::operation::test_operation_lock();
    let root = tempfile::tempdir().unwrap();
    let mut path = root.path().to_path_buf();
    for level in 1..=7 {
        std::fs::write(path.join(format!("file-{level}")), vec![level; 4096]).unwrap();
        path.push(format!("dir-{level}"));
        std::fs::create_dir(&path).unwrap();
    }
    let result = analyze(root.path());
    for depth in 1..=6 {
        let value = json(result.scan_id, depth, 0);
        let mut node = &value["root"];
        for level in 1..=depth {
            assert!(
                child(node, &format!("file-{level}"))["bytes"]
                    .as_u64()
                    .unwrap()
                    > 0
            );
            node = child(node, &format!("dir-{level}"));
        }
        assert!(node.get("children").is_none());
        assert!(node.get("remaining").is_some());
        verify_totals(&value["root"]);
    }
}

#[test]
fn one_global_budget_preserves_the_current_layer_then_ranks_all_siblings_together() {
    let _lock = crate::shared::operation::test_operation_lock();
    let root = tempfile::tempdir().unwrap();
    for (folder, size) in [("a", 4096), ("b", 8192), ("c", 12288), ("d", 16384)] {
        std::fs::create_dir(root.path().join(folder)).unwrap();
        for index in 0..220 {
            std::fs::write(
                root.path().join(folder).join(format!("f-{index:03}")),
                vec![1; size],
            )
            .unwrap();
        }
    }
    let result = analyze(root.path());
    let report = prepare(request(result.scan_id, 2, 200)).unwrap();
    assert_eq!(report.entry_count, 200);
    let value = serde_json::to_value(report).unwrap();
    assert_eq!(count(&value["root"]), 200);
    let a = child(&value["root"], "a");
    let d = child(&value["root"], "d");
    let available_d = cache::read_snapshot(|cache| {
        cache
            .navigation
            .file_names(&result.index_snapshot.as_ref().unwrap().root.join("d"))
            .map_or(0, |names| names.len())
    })
    .unwrap();
    assert_eq!(
        d["children"].as_array().unwrap().len(),
        available_d.min(196)
    );
    assert!(a["remaining"]["entryLimit"]["fileCount"].as_u64().unwrap() > 0);
    verify_totals(&value["root"]);
}

#[test]
fn missing_file_records_and_budget_omissions_are_disjoint_including_zero_byte_files() {
    let _lock = crate::shared::operation::test_operation_lock();
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("files")).unwrap();
    for i in 0..1505 {
        std::fs::write(
            root.path().join("files").join(format!("file-{i:04}")),
            vec![1; 4096],
        )
        .unwrap();
    }
    for i in 0..5 {
        std::fs::write(root.path().join("files").join(format!("empty-{i}")), []).unwrap();
    }
    let result = analyze(root.path());
    let available = cache::read_snapshot(|cache| {
        cache
            .navigation
            .file_names(&result.index_snapshot.as_ref().unwrap().root.join("files"))
            .map_or(0, |names| names.len())
    })
    .unwrap();
    for limit in [200, 500, 1000, 2000, 0] {
        let value = json(result.scan_id, 2, limit);
        let directory = child(&value["root"], "files");
        assert_eq!(
            directory["remaining"]["detailsNotRetained"]["fileCount"],
            (1510 - available) as u64
        );
        assert!(directory.get("omissions").is_none());
        if limit != 0 && available > limit - 1 {
            assert!(directory["remaining"].get("entryLimit").is_some());
        } else {
            assert!(directory["remaining"].get("entryLimit").is_none());
        }
        verify_totals(&value["root"]);
    }
}

#[test]
fn the_root_stays_scoped_to_the_current_list_even_with_a_larger_export_budget() {
    let _lock = crate::shared::operation::test_operation_lock();
    let root = tempfile::tempdir().unwrap();
    for i in 0..550 {
        std::fs::write(root.path().join(format!("file-{i:03}")), vec![1; 4096]).unwrap();
    }
    let result = analyze(root.path());
    assert_eq!(result.entries.len(), 500);
    for limit in [200, 500, 1000, 0] {
        let value = json(result.scan_id, 2, limit);
        let remaining = &value["root"]["remaining"];
        assert_eq!(
            remaining["listLimit"]["fileCount"].as_u64().unwrap_or(0)
                + remaining["detailsNotRetained"]["fileCount"]
                    .as_u64()
                    .unwrap_or(0),
            50
        );
        assert_eq!(count(&value["root"]), if limit == 200 { 200 } else { 500 });

        verify_totals(&value["root"]);
    }
}

#[test]
fn source_changes_never_refresh_export_and_evicted_or_replaced_indexes_fail_closed() {
    let _lock = crate::shared::operation::test_operation_lock();
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("child")).unwrap();
    let file = root.path().join("child/before.bin");
    std::fs::write(&file, b"scan data").unwrap();
    let result = analyze(root.path());
    let report = prepare(request(result.scan_id, 2, 500)).unwrap();
    let before = serde_json::to_value(&report).unwrap();
    std::fs::remove_file(file).unwrap();
    std::fs::write(root.path().join("child/after.bin"), b"not displayed").unwrap();
    assert_eq!(
        json(result.scan_id, 2, 500),
        before,
        "export must not enumerate or stat changed source files"
    );
    let _child = analyze(&root.path().join("child"));
    assert_eq!(
        prepare(request(result.scan_id, 2, 500))
            .err()
            .unwrap()
            .reason(),
        Some(CoreErrorReason::AnalysisExportUnavailable)
    );
    assert_eq!(
        serde_json::to_value(&report).unwrap(),
        before,
        "prepared report remains immutable across the picker"
    );
    let latest = analyze(root.path());
    cache::clear_all().unwrap();
    assert_eq!(
        prepare(request(latest.scan_id, 2, 500))
            .err()
            .unwrap()
            .reason(),
        Some(CoreErrorReason::AnalysisExportUnavailable)
    );
}

#[test]
fn empty_lists_and_atomic_save_failures_keep_the_destination_intact() {
    let _lock = crate::shared::operation::test_operation_lock();
    let root = tempfile::tempdir().unwrap();
    let result = analyze(root.path());
    let report = prepare(request(result.scan_id, 2, 500)).unwrap();
    let output = tempfile::tempdir().unwrap();
    let path = output.path().join("report.json");
    std::fs::write(&path, b"old report").unwrap();
    let receipt = report.save(&path).unwrap();
    assert_eq!(receipt.schema_version, 2);
    assert_eq!(receipt.entry_count, 0);
    assert_eq!(receipt.file_bytes, std::fs::metadata(&path).unwrap().len());
    let value: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(value["root"]["children"], serde_json::json!([]));
    verify_totals(&value["root"]);
    assert!(report.save(Path::new("relative.json")).is_err());
    let blocked = output.path().join("blocked.json");
    std::fs::create_dir(&blocked).unwrap();
    assert!(report.save(&blocked).is_err());
    assert!(blocked.is_dir());
    assert_eq!(std::fs::read_dir(output.path()).unwrap().count(), 2);
}

#[cfg(unix)]
#[test]
fn hard_link_aliases_do_not_duplicate_space() {
    let _lock = crate::shared::operation::test_operation_lock();
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("linked");
    std::fs::create_dir(&dir).unwrap();
    let file = dir.join("owner.bin");
    std::fs::write(&file, vec![1; 8192]).unwrap();
    std::fs::hard_link(&file, dir.join("alias.bin")).unwrap();
    let result = analyze(root.path());
    let value = json(result.scan_id, 2, 500);
    let dir = &value["root"]["children"][0];
    assert_eq!(dir["children"].as_array().unwrap().len(), 2);
    assert_eq!(dir["fileCount"], 2);
    verify_totals(&value["root"]);
}

#[test]
fn invalid_limits_and_expired_sessions_never_fall_back_to_scanning() {
    for (depth, limit) in [(0, 500), (7, 500), (2, 123)] {
        assert!(prepare(request(u64::MAX, depth, limit)).is_err());
    }
    assert_eq!(
        prepare(request(u64::MAX, 2, 500)).err().unwrap().reason(),
        Some(CoreErrorReason::AnalysisExportUnavailable)
    );
    assert!(serde_json::from_value::<AnalysisExportRequest>(serde_json::json!({"scanId":1,"maxDepth":2,"maxEntries":500,"includeFullPaths":false,"extra":1})).is_err());
    let legacy: AnalysisExportRequest = serde_json::from_value(
        serde_json::json!({"scanId":1,"maxDepth":2,"maxEntries":500,"includeFullPaths":false}),
    )
    .unwrap();
    assert_eq!(legacy.format, AnalysisExportFormat::Json);
    assert!(serde_json::from_value::<AnalysisExportRequest>(serde_json::json!({"scanId":1,"maxDepth":2,"maxEntries":500,"includeFullPaths":false,"format":"csv"})).is_err());
}

#[test]
fn markdown_and_json_share_one_frozen_selection_after_the_source_disappears() {
    let _lock = crate::shared::operation::test_operation_lock();
    let parent = tempfile::tempdir().unwrap();
    let root = parent.path().join("source");
    std::fs::create_dir_all(root.join("child/deep")).unwrap();
    std::fs::write(root.join("child/file.bin"), b"private file content marker").unwrap();
    std::fs::write(root.join("child/deep/hidden.bin"), vec![1; 4096]).unwrap();
    let result = analyze(&root);
    let params = AnalysisExportRequest {
        include_full_paths: true,
        ..request(result.scan_id, 2, 500)
    };
    let json_report = prepare(params).unwrap();
    let markdown = prepare(AnalysisExportRequest {
        format: AnalysisExportFormat::Markdown,
        ..params
    })
    .unwrap();
    assert_eq!(
        serde_json::to_value(&json_report).unwrap(),
        serde_json::to_value(&markdown).unwrap()
    );
    assert!(json_report.suggested_file_name().ends_with(".json"));
    assert!(markdown.suggested_file_name().ends_with(".md"));
    std::fs::rename(&root, parent.path().join("moved")).unwrap();
    cache::clear_all().unwrap();
    let output = tempfile::tempdir().unwrap();
    let destination = output.path().join("report.md");
    std::fs::write(&destination, b"previous report").unwrap();
    let receipt = markdown.save(&destination).unwrap();
    let text = std::fs::read_to_string(&destination).unwrap();
    assert!(text.starts_with("# MangoDisk disk analysis\n"));
    assert!(text.contains("**Summary — Not expanded (depth limit)**"));
    assert!(text.contains("**File** ` \"file.bin\" `"));
    assert!(!text.contains("private file content marker"));
    assert_eq!(receipt.entry_count, json_report.entry_count);
    assert_eq!(receipt.file_bytes, text.len() as u64);
    assert_eq!(receipt.schema_version, 2);
    assert!(markdown.save(Path::new("relative.md")).is_err());
    let blocked = output.path().join("blocked.md");
    std::fs::create_dir(&blocked).unwrap();
    assert!(markdown.save(&blocked).is_err());
    assert_eq!(std::fs::read_dir(output.path()).unwrap().count(), 2);
}

#[cfg(target_os = "linux")]
#[test]
fn non_utf8_directory_names_use_native_keys_without_losing_nested_files() {
    use std::os::unix::ffi::OsStringExt;
    let _lock = crate::shared::operation::test_operation_lock();
    let root = tempfile::tempdir().unwrap();
    let dir = root
        .path()
        .join(std::ffi::OsString::from_vec(b"dir-\xff".to_vec()));
    std::fs::create_dir(&dir).unwrap();
    std::fs::write(dir.join("child.bin"), vec![1; 4096]).unwrap();
    let result = analyze(root.path());
    let value = json(result.scan_id, 2, 500);
    let dir = &value["root"]["children"][0];
    assert_eq!(dir["children"][0]["name"], "child.bin");
    verify_totals(&value["root"]);
}

#[test]
fn intentional_scan_exclusions_are_not_export_folding_or_missing_file_detail() {
    let _lock = crate::shared::operation::test_operation_lock();
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("visible/excluded")).unwrap();
    std::fs::write(
        root.path().join("visible/excluded/private-name.bin"),
        vec![1; 4096],
    )
    .unwrap();
    std::fs::write(root.path().join("visible/kept.bin"), vec![1; 4096]).unwrap();
    let result = AnalysisService::analyze_with_exclusions_progress(
        Some(root.path().to_string_lossy().into_owned()),
        true,
        ScanExclusionOptions {
            paths: vec![root
                .path()
                .join("visible/excluded")
                .to_string_lossy()
                .into_owned()],
            names: vec![],
        },
        |_| {},
    )
    .unwrap();
    let value = json(result.scan_id, 6, 0);
    let dir = child(&value["root"], "visible");
    assert!(dir["omissions"]["excluded"].as_u64().unwrap() > 0);
    assert!(dir.get("remaining").is_none());
    assert!(!serde_json::to_string(&value)
        .unwrap()
        .contains("private-name"));
    verify_totals(&value["root"]);
}

#[test]
fn native_u64_totals_and_metric_survive_serialization_without_javascript_rounding() {
    let _lock = crate::shared::operation::test_operation_lock();
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("file.bin"), vec![1; 4096]).unwrap();
    let result = analyze(root.path());
    let report = prepare(request(result.scan_id, 2, 500)).unwrap();
    let mut root_node = report.root;
    root_node.bytes = (1_u64 << 53) + 7;
    root_node.children.as_mut().unwrap()[0].bytes = root_node.bytes;
    let report = AnalysisReport::new(
        AnalysisReportScan {
            scan_mode: AnalysisScanMode::Fast,
            ..report.scan
        },
        root_node,
        1,
        false,
        AnalysisExportFormat::Json,
    );
    let value = serde_json::to_value(report).unwrap();
    assert_eq!(value["byteMetric"], "logicalBytes");
    assert_eq!(value["root"]["bytes"].as_u64(), Some((1_u64 << 53) + 7));
    verify_totals(&value["root"]);
}

#[test]
fn refreshed_subtrees_preserve_the_measurement_times_of_untouched_siblings() {
    let _lock = crate::shared::operation::test_operation_lock();
    let root = tempfile::tempdir().unwrap();
    for name in ["a", "b"] {
        std::fs::create_dir(root.path().join(name)).unwrap();
        std::fs::write(root.path().join(name).join("file.bin"), vec![1; 4096]).unwrap();
    }
    let original = analyze(root.path());
    let native_root = original.index_snapshot.as_ref().unwrap().root.clone();
    // Ensure the refreshed subtree has a distinct wall-clock measurement time.
    std::thread::sleep(std::time::Duration::from_millis(2));
    let refreshed = analyze(&root.path().join("a"));
    assert_ne!(original.scanned_at_ms, refreshed.scanned_at_ms);
    let rebuilt = super::super::index_projection::analysis_result(&native_root)
        .unwrap()
        .unwrap();
    let current = super::super::session::publish_result_session(rebuilt).unwrap();
    let value = json(current.scan_id, 2, 500);
    assert_eq!(value["scannedAtMs"], refreshed.scanned_at_ms);
    assert_eq!(
        child(&value["root"], "b")["scannedAtMs"],
        original.scanned_at_ms
    );
    verify_totals(&value["root"]);
}

#[cfg(target_os = "linux")]
#[test]
fn distinct_native_directories_with_colliding_display_names_keep_their_own_totals() {
    use std::os::unix::ffi::OsStringExt;
    let _lock = crate::shared::operation::test_operation_lock();
    // Navigation's hash order is deliberately independent of the list's size order.
    for _ in 0..16 {
        let root = tempfile::tempdir().unwrap();
        for (suffix, size) in [(0x80, 4096), (0x81, 8192)] {
            let dir = root
                .path()
                .join(std::ffi::OsString::from_vec(vec![b'd', suffix]));
            std::fs::create_dir(&dir).unwrap();
            std::fs::write(dir.join("file.bin"), vec![1; size]).unwrap();
        }
        let result = analyze(root.path());
        let value = json(result.scan_id, 2, 500);
        assert_eq!(value["root"]["children"].as_array().unwrap().len(), 2);
        verify_totals(&value["root"]);
    }
}
