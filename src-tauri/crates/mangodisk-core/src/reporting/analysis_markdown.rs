use std::io::{self, Write};

use chrono::{TimeZone, Utc};

use super::{AnalysisEntrySizeStatus, AnalysisReport, AnalysisReportEntry, ReportByteMetric};

/// Stream the same selected tree as JSON, without a second report or a full text buffer.
pub(super) fn write(report: &AnalysisReport, writer: &mut impl Write) -> io::Result<()> {
    writeln!(writer, "# MangoDisk disk analysis\n")?;
    write!(writer, "- Scanned at: ")?;
    if let Some(time) = i64::try_from(report.scanned_at_ms)
        .ok()
        .and_then(|ms| Utc.timestamp_millis_opt(ms).single())
    {
        write!(writer, "{}; ", time.to_rfc3339())?;
    }
    writeln!(writer, "{} ms since Unix epoch", report.scanned_at_ms)?;
    writeln!(
        writer,
        "- Size metric: {}",
        match report.byte_metric {
            ReportByteMetric::AllocatedBytes => "allocated bytes (disk usage)",
            ReportByteMetric::LogicalBytes => "logical bytes (file size)",
        }
    )?;
    writeln!(
        writer,
        "- Exported entries: {} (root and summaries excluded)\n",
        report.entry_count
    )?;
    writeln!(writer, "## Directory tree\n")?;
    node(writer, &report.root, 0)
}

fn indent(writer: &mut impl Write, depth: usize) -> io::Result<()> {
    for _ in 0..depth {
        writer.write_all(b"    ")?;
    }
    Ok(())
}

/// JSON escaping keeps control characters on one line; a longer code delimiter keeps
/// backticks, links and raw HTML inert even in adversarial filesystem names.
fn literal(writer: &mut impl Write, value: &str) -> io::Result<()> {
    let longest = value.split(|c| c != '`').map(str::len).max().unwrap_or(0);
    for _ in 0..=longest {
        writer.write_all(b"`")?;
    }
    writer.write_all(b" ")?;
    serde_json::to_writer(&mut *writer, value).map_err(io::Error::other)?;
    writer.write_all(b" ")?;
    for _ in 0..=longest {
        writer.write_all(b"`")?;
    }
    Ok(())
}

fn node(writer: &mut impl Write, entry: &AnalysisReportEntry, depth: usize) -> io::Result<()> {
    indent(writer, depth)?;
    write!(
        writer,
        "- **{}** ",
        if entry.is_directory {
            "Directory"
        } else {
            "File"
        }
    )?;
    literal(writer, &entry.name)?;
    write!(writer, " — {} bytes", entry.bytes)?;
    if let Some(count) = entry.file_count {
        write!(writer, "; {count} files")?;
    }
    if let Some(time) = entry.modified_at_ms {
        write!(writer, "; modifiedAtMs={time}")?;
    }
    if let Some(time) = entry.scanned_at_ms {
        write!(writer, "; scannedAtMs={time}")?;
    }
    match entry.size_status {
        AnalysisEntrySizeStatus::Complete => {}
        AnalysisEntrySizeStatus::Partial => write!(writer, "; size status: partial")?,
        AnalysisEntrySizeStatus::Unavailable => write!(writer, "; size status: unavailable")?,
    }
    writeln!(writer)?;
    if let Some(path) = &entry.path {
        indent(writer, depth + 1)?;
        write!(writer, "- Path: ")?;
        literal(writer, path)?;
        writeln!(writer)?;
    }
    if let Some(children) = &entry.children {
        for child in children {
            node(writer, child, depth + 1)?;
        }
    }
    for (label, totals) in [
        ("Not expanded (depth limit)", entry.remaining.depth_limit),
        ("Not exported (entry limit)", entry.remaining.entry_limit),
        ("Outside the current list", entry.remaining.list_limit),
        ("Details not retained", entry.remaining.details_not_retained),
    ] {
        if let Some(totals) = totals {
            indent(writer, depth + 1)?;
            write!(
                writer,
                "- **Summary — {label}**: {} bytes; {} files",
                totals.bytes, totals.file_count
            )?;
            if totals.directory_count > 0 {
                write!(writer, "; {} direct directories", totals.directory_count)?;
            }
            writeln!(writer)?;
        }
    }
    if entry.omissions.total() > 0 {
        indent(writer, depth + 1)?;
        writeln!(
            writer,
            "- **Scan omissions** (encountered entries; descendants may be unknown):"
        )?;
        for (reason, count) in [
            ("permissionDenied", entry.omissions.permission_denied),
            ("readErrors", entry.omissions.read_errors),
            ("excluded", entry.omissions.excluded),
            ("linkOrMount", entry.omissions.link_or_mount),
            ("other", entry.omissions.other),
        ] {
            if count > 0 {
                indent(writer, depth + 2)?;
                writeln!(writer, "- {reason}: {count}")?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "analysis_markdown_tests.rs"]
mod tests;
