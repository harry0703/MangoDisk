use std::{
    io::{BufWriter, Write},
    path::Path,
    time::Instant,
};

use chrono::{TimeZone, Utc};
use mangodisk_platform::ScanOmissionCounts;
use serde::{ser::SerializeMap, Serialize, Serializer};

use crate::{
    storage::analysis::{AnalysisEntrySizeStatus, AnalysisScanMode},
    CoreError, CoreResult,
};

#[path = "analysis_markdown.rs"]
mod markdown;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, serde::Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AnalysisExportFormat {
    #[default]
    Json,
    Markdown,
}

impl AnalysisExportFormat {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Markdown => "md",
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Markdown => "markdown",
        }
    }
}

/// Version 5 is nested, write-only scan evidence, never deletion authority.
/// Consumers must reject unsupported versions; reports are not imported as app state.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisReport {
    schema_version: u8,
    byte_metric: ReportByteMetric,
    scanned_at_ms: u64,
    pub(crate) root: AnalysisReportEntry,
    #[serde(skip)]
    pub(crate) scan: AnalysisReportScan,
    #[serde(skip)]
    path_format: ReportPathFormat,
    #[serde(skip)]
    pub(crate) entry_count: usize,
    #[serde(skip)]
    format: AnalysisExportFormat,
}

enum ReportPathFormat {
    Relative,
    Absolute,
}
impl ReportPathFormat {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Relative => "relative",
            Self::Absolute => "absolute",
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
enum ReportByteMetric {
    AllocatedBytes,
    LogicalBytes,
}

pub(crate) struct AnalysisReportScan {
    pub id: u64,
    pub root_name: String,
    pub scanned_at_ms: u64,
    pub scan_mode: AnalysisScanMode,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AnalysisReportEntry {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub bytes: u64,
    pub is_directory: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modified_at_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scanned_at_ms: Option<u64>,
    #[serde(skip_serializing_if = "complete_size")]
    pub size_status: AnalysisEntrySizeStatus,
    #[serde(
        skip_serializing_if = "empty_omissions",
        serialize_with = "nonzero_omissions"
    )]
    pub omissions: ScanOmissionCounts,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub children: Option<Vec<AnalysisReportEntry>>,
    #[serde(skip_serializing_if = "AnalysisReportRemaining::is_empty")]
    pub remaining: AnalysisReportRemaining,
}

/// Each bucket represents disjoint omitted immediate children. File counts include
/// all scanned files in omitted directory subtrees; directory counts are direct children only.
#[derive(Default, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AnalysisReportTotals {
    pub bytes: u64,
    pub file_count: u64,
    #[serde(rename = "directDirectoryCount", skip_serializing_if = "zero")]
    pub directory_count: u64,
}

impl AnalysisReportTotals {
    pub(crate) fn is_empty(self) -> bool {
        self.bytes == 0 && self.file_count == 0 && self.directory_count == 0
    }
    pub(crate) fn add(&mut self, value: Self) -> CoreResult<()> {
        let overflow = || CoreError::invalid_input("analysis export accounting overflowed");
        self.bytes = self.bytes.checked_add(value.bytes).ok_or_else(overflow)?;
        self.file_count = self
            .file_count
            .checked_add(value.file_count)
            .ok_or_else(overflow)?;
        self.directory_count = self
            .directory_count
            .checked_add(value.directory_count)
            .ok_or_else(overflow)?;
        Ok(())
    }
}

fn zero(value: &u64) -> bool {
    *value == 0
}

#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AnalysisReportRemaining {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub depth_limit: Option<AnalysisReportTotals>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry_limit: Option<AnalysisReportTotals>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub list_limit: Option<AnalysisReportTotals>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details_not_retained: Option<AnalysisReportTotals>,
}
impl AnalysisReportRemaining {
    fn is_empty(&self) -> bool {
        self.depth_limit.is_none()
            && self.entry_limit.is_none()
            && self.list_limit.is_none()
            && self.details_not_retained.is_none()
    }
}

fn complete_size(status: &AnalysisEntrySizeStatus) -> bool {
    *status == AnalysisEntrySizeStatus::Complete
}

fn nonzero_omissions<S: Serializer>(
    counts: &ScanOmissionCounts,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    let mut map = serializer.serialize_map(None)?;
    for (reason, count) in [
        ("permissionDenied", counts.permission_denied),
        ("readErrors", counts.read_errors),
        ("excluded", counts.excluded),
        ("linkOrMount", counts.link_or_mount),
        ("other", counts.other),
    ] {
        if count > 0 {
            map.serialize_entry(reason, &count)?;
        }
    }
    map.end()
}

fn empty_omissions(counts: &ScanOmissionCounts) -> bool {
    counts.total() == 0
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisExportReceipt {
    pub schema_version: u8,
    pub path: String,
    pub entry_count: usize,
    pub file_bytes: u64,
}

impl AnalysisReport {
    pub(crate) fn new(
        scan: AnalysisReportScan,
        root: AnalysisReportEntry,
        entry_count: usize,
        full_paths: bool,
        format: AnalysisExportFormat,
    ) -> Self {
        Self {
            schema_version: 5,
            byte_metric: if scan.scan_mode == AnalysisScanMode::Fast {
                ReportByteMetric::LogicalBytes
            } else {
                ReportByteMetric::AllocatedBytes
            },
            scanned_at_ms: scan.scanned_at_ms,
            root,
            scan,
            entry_count,
            format,
            path_format: if full_paths {
                ReportPathFormat::Absolute
            } else {
                ReportPathFormat::Relative
            },
        }
    }

    pub fn suggested_file_name(&self) -> String {
        let name: String = self
            .scan
            .root_name
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || matches!(c, '-' | '_') {
                    c
                } else {
                    '-'
                }
            })
            .take(48)
            .collect();
        let name = if name.trim_matches('-').is_empty() {
            "disk"
        } else {
            name.trim_matches('-')
        };
        let timestamp = Utc
            .timestamp_millis_opt(self.scan.scanned_at_ms as i64)
            .single()
            .map(|time| time.format("%Y%m%d-%H%M%SZ").to_string())
            .unwrap_or_else(|| "scan".into());
        format!("mangodisk-{name}-{timestamp}.{}", self.format.extension())
    }

    pub fn format(&self) -> AnalysisExportFormat {
        self.format
    }

    pub(crate) fn total_bytes(&self) -> u64 {
        self.root.bytes
    }

    /// Publish only a complete report. A failed serialization or write leaves the destination intact.
    pub fn save(&self, destination: &Path) -> CoreResult<AnalysisExportReceipt> {
        let started = Instant::now();
        let diagnostic = mangodisk_platform::diagnostics::text(&destination.display());
        log::info!(
            "analysis_export_save_started scan_id={} destination={} entries={} path_format={} format={}",
            self.scan.id,
            diagnostic,
            self.entry_count,
            self.path_format.as_str(),
            self.format.as_str()
        );
        let mut stage = "create_temporary";
        let result = (|| -> Result<u64, Box<dyn std::error::Error>> {
            let parent = destination
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
                .ok_or("an absolute report destination is required")?;
            if !destination.is_absolute() {
                return Err("an absolute report destination is required".into());
            }
            let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
            stage = "serialize_and_write";
            {
                let mut writer = BufWriter::new(temporary.as_file_mut());
                match self.format {
                    AnalysisExportFormat::Json => {
                        serde_json::to_writer(&mut writer, self)?;
                        writer.write_all(b"\n")?;
                    }
                    AnalysisExportFormat::Markdown => markdown::write(self, &mut writer)?,
                }
                writer.flush()?;
            }
            stage = "sync";
            temporary.as_file().sync_all()?;
            let bytes = temporary.as_file().metadata()?.len();
            stage = "publish";
            temporary.persist(destination)?;
            Ok(bytes)
        })();
        match result {
            Ok(file_bytes) => {
                log::info!("analysis_export_save_finished scan_id={} destination={} file_bytes={} entries={} elapsed_ms={}", self.scan.id, diagnostic, file_bytes, self.entry_count, started.elapsed().as_millis());
                Ok(AnalysisExportReceipt {
                    schema_version: 2,
                    path: destination.to_string_lossy().into_owned(),
                    entry_count: self.entry_count,
                    file_bytes,
                })
            }
            Err(error) => {
                log::warn!("analysis_export_save_failed scan_id={} destination={} stage={} error={} elapsed_ms={}", self.scan.id, diagnostic, stage, mangodisk_platform::diagnostics::text(&error), started.elapsed().as_millis());
                Err(CoreError::persistence(format!(
                    "analysis report save failed at {stage}: {error}"
                )))
            }
        }
    }
}
