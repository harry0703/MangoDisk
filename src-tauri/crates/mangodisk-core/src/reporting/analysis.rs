use std::{
    io::{BufWriter, Write},
    path::Path,
    time::Instant,
};

use chrono::{TimeZone, Utc};
use mangodisk_platform::{ScanNameExclusion, ScanOmissionCounts};
use serde::{ser::SerializeMap, Serialize, Serializer};

use crate::{
    storage::analysis::{AnalysisEntrySizeStatus, AnalysisScanMode},
    CoreError, CoreResult,
};

/// Standalone, versioned evidence for external tools. It is never accepted as deletion authority.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisReport {
    schema_version: u8,
    #[serde(skip)]
    path_format: ReportPathFormat,
    byte_metric: ReportByteMetric,
    #[serde(flatten)]
    pub(crate) scan: AnalysisReportScan,
    #[serde(skip_serializing_if = "AnalysisReportExclusions::is_empty")]
    exclusions: AnalysisReportExclusions,
    #[serde(skip_serializing_if = "Option::is_none")]
    scanned_at_range_ms: Option<[u64; 2]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    min_directory_bytes: Option<u64>,
    pub(crate) directories: Vec<AnalysisReportDirectory>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
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

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AnalysisReportScan {
    #[serde(skip)]
    pub id: u64,
    #[serde(skip)]
    pub root_name: String,
    pub root: String,
    pub scanned_at_ms: u64,
    #[serde(skip)]
    pub scan_mode: AnalysisScanMode,
    #[serde(skip)]
    pub total_bytes: u64,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub subtree_of_original_scan: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AnalysisReportDirectory {
    pub path: String,
    pub bytes: u64,
    pub file_count: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scanned_at_ms: Option<u64>,
    #[serde(skip_serializing_if = "complete_size")]
    pub size_status: AnalysisEntrySizeStatus,
    #[serde(
        skip_serializing_if = "empty_omissions",
        serialize_with = "nonzero_omissions"
    )]
    pub omissions: ScanOmissionCounts,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub path_is_lossy: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub collapsed: Option<AnalysisReportSubtreeTotals>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filtered: Option<AnalysisReportSubtreeTotals>,
}

#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AnalysisReportSubtreeTotals {
    pub directory_count: usize,
    pub bytes: u64,
    pub file_count: u64,
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

#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AnalysisReportExclusions {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub paths: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub name_rules: Vec<ScanNameExclusion>,
}

impl AnalysisReportExclusions {
    fn is_empty(&self) -> bool {
        self.paths.is_empty() && self.name_rules.is_empty()
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisExportReceipt {
    pub schema_version: u8,
    pub path: String,
    pub directory_count: usize,
    pub file_bytes: u64,
}

impl AnalysisReport {
    pub(crate) fn new(
        scan: AnalysisReportScan,
        mut directories: Vec<AnalysisReportDirectory>,
        mut exclusions: AnalysisReportExclusions,
        full_paths: bool,
        scanned_at_range_ms: Option<[u64; 2]>,
        min_directory_bytes: u64,
    ) -> Self {
        let root_path = if full_paths { scan.root.as_str() } else { "." };
        directories.sort_by(|left, right| {
            (left.path != root_path)
                .cmp(&(right.path != root_path))
                .then_with(|| left.path.cmp(&right.path))
        });
        exclusions.paths.sort();
        Self {
            schema_version: 4,
            path_format: if full_paths {
                ReportPathFormat::Absolute
            } else {
                ReportPathFormat::Relative
            },
            byte_metric: if scan.scan_mode == AnalysisScanMode::Fast {
                ReportByteMetric::LogicalBytes
            } else {
                ReportByteMetric::AllocatedBytes
            },
            scan,
            exclusions,
            directories,
            scanned_at_range_ms,
            min_directory_bytes: (min_directory_bytes > 0).then_some(min_directory_bytes),
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
        format!("mangodisk-{name}-{timestamp}.json")
    }

    pub(crate) fn total_bytes(&self) -> u64 {
        self.scan.total_bytes
    }

    /// Publish only a complete report. A failed serialization or write leaves the destination intact.
    pub fn save(&self, destination: &Path) -> CoreResult<AnalysisExportReceipt> {
        let started = Instant::now();
        let diagnostic = mangodisk_platform::diagnostics::text(&destination.display());
        log::info!(
            "analysis_export_save_started scan_id={} destination={} directories={} path_format={}",
            self.scan.id,
            diagnostic,
            self.directories.len(),
            self.path_format.as_str()
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
                serde_json::to_writer(&mut writer, self)?;
                writer.write_all(b"\n")?;
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
                log::info!("analysis_export_save_finished scan_id={} destination={} file_bytes={} directories={} elapsed_ms={}", self.scan.id, diagnostic, file_bytes, self.directories.len(), started.elapsed().as_millis());
                Ok(AnalysisExportReceipt {
                    schema_version: 1,
                    path: destination.to_string_lossy().into_owned(),
                    directory_count: self.directories.len(),
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
