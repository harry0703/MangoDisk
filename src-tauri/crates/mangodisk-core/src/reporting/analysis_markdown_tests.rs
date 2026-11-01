use super::super::{
    AnalysisExportFormat, AnalysisReportRemaining, AnalysisReportScan, AnalysisReportTotals,
};
use super::*;
use crate::storage::analysis::AnalysisScanMode;

fn file(name: &str, bytes: u64) -> AnalysisReportEntry {
    AnalysisReportEntry {
        name: name.into(),
        path: None,
        bytes,
        is_directory: false,
        file_count: None,
        modified_at_ms: None,
        scanned_at_ms: None,
        size_status: AnalysisEntrySizeStatus::Complete,
        omissions: Default::default(),
        children: None,
        remaining: Default::default(),
    }
}

fn fixture() -> AnalysisReport {
    let mut child = file("folder", 8);
    child.is_directory = true;
    child.file_count = Some(1);
    child.scanned_at_ms = Some(1_700_000_000_123);
    let mut leaf = file("leaf.bin", 8);
    leaf.modified_at_ms = Some(1_700_000_000_010);
    child.children = Some(vec![leaf]);
    let mut unreadable = file("unreadable", 0);
    unreadable.is_directory = true;
    unreadable.file_count = Some(0);
    unreadable.size_status = AnalysisEntrySizeStatus::Unavailable;
    let mut root = file("fixture", (1_u64 << 53) + 7 + 8 + 16);
    root.is_directory = true;
    root.file_count = Some(6);
    root.size_status = AnalysisEntrySizeStatus::Partial;
    root.children = Some(vec![
        file("large.bin", (1_u64 << 53) + 7),
        child,
        unreadable,
    ]);
    let bucket = Some(AnalysisReportTotals {
        bytes: 4,
        file_count: 1,
        directory_count: 0,
    });
    root.remaining = AnalysisReportRemaining {
        depth_limit: bucket,
        entry_limit: bucket,
        list_limit: bucket,
        details_not_retained: bucket,
    };
    root.omissions = mangodisk_platform::ScanOmissionCounts {
        permission_denied: 1,
        read_errors: 2,
        excluded: 3,
        link_or_mount: 4,
        other: 5,
    };
    AnalysisReport::new(
        AnalysisReportScan {
            id: 1,
            root_name: root.name.clone(),
            scanned_at_ms: 1_700_000_000_000,
            scan_mode: AnalysisScanMode::Standard,
        },
        root,
        4,
        false,
        AnalysisExportFormat::Markdown,
    )
}

#[test]
fn markdown_preserves_nested_facts_and_disjoint_coverage_without_rounding() {
    let mut report = fixture();
    let mut output = Vec::new();
    write(&report, &mut output).unwrap();
    let text = String::from_utf8(output).unwrap();
    assert!(text.contains("9007199254740999 bytes"));
    assert!(
        text.contains("        - **File** ` \"leaf.bin\" ` — 8 bytes; modifiedAtMs=1700000000010")
    );
    assert!(text.contains("scannedAtMs=1700000000123"));
    assert!(text.contains("size status: partial"));
    assert!(text.contains("size status: unavailable"));
    assert_eq!(text.matches("**Summary —").count(), 4);
    assert!(text.contains("**Scan omissions**"));
    for (reason, count) in [
        ("permissionDenied", 1),
        ("readErrors", 2),
        ("excluded", 3),
        ("linkOrMount", 4),
        ("other", 5),
    ] {
        assert!(text.contains(&format!("- {reason}: {count}")));
    }
    assert!(text.contains("allocated bytes (disk usage)"));
    let value = serde_json::to_value(&report).unwrap();
    report.format = AnalysisExportFormat::Json;
    assert_eq!(serde_json::to_value(&report).unwrap(), value);
    report.byte_metric = ReportByteMetric::LogicalBytes;
    let mut output = Vec::new();
    write(&report, &mut output).unwrap();
    assert!(String::from_utf8(output)
        .unwrap()
        .contains("logical bytes (file size)"));
}

#[test]
fn names_and_paths_cannot_inject_markdown_blocks_links_or_raw_html() {
    let mut report = fixture();
    let hostile =
        "``` [link](https://example.invalid)\n# injected\r\n<img src=x onerror=alert(1)>\\name\t";
    report.root.name = hostile.into();
    report.root.path = Some(hostile.into());
    let mut output = Vec::new();
    write(&report, &mut output).unwrap();
    let text = String::from_utf8(output).unwrap();
    let quoted = serde_json::to_string(hostile).unwrap();
    assert_eq!(text.matches(&format!("```` {quoted} ````")).count(), 2);
    assert!(!text.contains("\n# injected"));
    assert!(!text.contains('\r'));
    assert!(!text.contains('\t'));
}

#[test]
fn markdown_propagates_writer_failures_without_claiming_success() {
    struct FailedWriter;
    impl Write for FailedWriter {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::Error::other("injected write failure"))
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    assert!(write(&fixture(), &mut FailedWriter)
        .unwrap_err()
        .to_string()
        .contains("injected write failure"));
}
