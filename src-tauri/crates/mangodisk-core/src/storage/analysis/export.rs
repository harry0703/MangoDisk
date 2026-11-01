use std::time::Instant;

use serde::Deserialize;

use super::{AnalysisCoverage, AnalysisIndexSnapshot, AnalysisScanMode, DirectoryEntryInfo};
use crate::{
    reporting::analysis::{AnalysisExportFormat, AnalysisReport, AnalysisReportScan},
    storage::index::cache,
    CoreError, CoreErrorReason, CoreResult,
};

/// Zero means all available detail within max_depth, never a filesystem refresh.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AnalysisExportRequest {
    pub scan_id: u64,
    pub max_depth: u8,
    pub max_entries: usize,
    pub include_full_paths: bool,
    #[serde(default)]
    pub format: AnalysisExportFormat,
}

pub(super) struct AnalysisExportSnapshot {
    pub root: String,
    pub index_snapshot: Option<AnalysisIndexSnapshot>,
    pub total_bytes: u64,
    pub scanned_at_ms: u64,
    pub scan_mode: AnalysisScanMode,
    pub coverage: AnalysisCoverage,
    pub entries: Vec<DirectoryEntryInfo>,
}

pub(super) fn unavailable() -> CoreError {
    CoreError::invalid_input("the matching disk-analysis snapshot is unavailable; analyze again")
        .with_reason(CoreErrorReason::AnalysisExportUnavailable)
}

/// The retained session and index must refer to one immutable measurement. No
/// filesystem validation or fallback enumeration belongs on the export path.
pub(super) fn prepare(request: AnalysisExportRequest) -> CoreResult<AnalysisReport> {
    if !(1..=6).contains(&request.max_depth)
        || ![0, 200, 500, 1000, 2000].contains(&request.max_entries)
    {
        return Err(CoreError::invalid_input(
            "unsupported analysis export depth or entry limit",
        ));
    }
    let started = Instant::now();
    log::info!("analysis_export_prepare_started scan_id={} max_depth={} max_entries={} include_full_paths={} format={}", request.scan_id, request.max_depth, request.max_entries, request.include_full_paths, request.format.as_str());
    let result = (|| -> CoreResult<AnalysisReport> {
        let snapshot = super::session::export_result(request.scan_id)?;
        let identity = snapshot.index_snapshot.as_ref().ok_or_else(unavailable)?;
        cache::read_snapshot(|cache| {
            if !super::query::matches_snapshot(&cache, identity) {
                log::info!(
                    "analysis_export_snapshot_rejected scan_id={} reason=index_revision_changed",
                    request.scan_id
                );
                return Err(unavailable());
            }
            let aggregate = cache
                .directories
                .get(&identity.root)
                .ok_or_else(unavailable)?;
            if aggregate.bytes != snapshot.total_bytes
                || aggregate.scanned_at_ms != snapshot.scanned_at_ms
                || AnalysisScanMode::from(aggregate.byte_metric) != snapshot.scan_mode
            {
                return Err(unavailable());
            }
            let (root, entry_count) = super::export_selection::select(&cache, &snapshot, request)
                .map_err(|error| {
                error.with_reason(CoreErrorReason::AnalysisExportUnavailable)
            })?;
            log::info!("analysis_export_selection_finished scan_id={} entries={} max_depth={} max_entries={}", request.scan_id, entry_count, request.max_depth, request.max_entries);
            Ok(AnalysisReport::new(
                AnalysisReportScan {
                    id: request.scan_id,
                    root_name: root.name.clone(),
                    scanned_at_ms: snapshot.scanned_at_ms,
                    scan_mode: snapshot.scan_mode,
                },
                root,
                entry_count,
                request.include_full_paths,
                request.format,
            ))
        })?
    })();
    match &result {
        Ok(report) => log::info!("analysis_export_prepare_finished scan_id={} entries={} total_bytes={} elapsed_ms={}", request.scan_id, report.entry_count, report.total_bytes(), started.elapsed().as_millis()),
        Err(error) => log::warn!("analysis_export_prepare_failed scan_id={} stage=snapshot code={:?} error={} elapsed_ms={}", request.scan_id, error.code(), mangodisk_platform::diagnostics::text(error), started.elapsed().as_millis()),
    }
    result
}

#[cfg(test)]
#[path = "export_tests.rs"]
mod tests;
