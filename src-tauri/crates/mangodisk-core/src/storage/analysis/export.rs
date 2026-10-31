use std::{path::Path, time::Instant};

use serde::{Deserialize, Serialize};

use super::{AnalysisCoverage, AnalysisIndexSnapshot, AnalysisScanMode};
use crate::{
    filesystem::metadata::display_path,
    reporting::analysis::{
        AnalysisReport, AnalysisReportDirectory, AnalysisReportExclusions, AnalysisReportScan,
    },
    storage::index::cache::{self, IndexReadView},
    CoreError, CoreErrorReason, CoreResult,
};

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AnalysisExportScope {
    ScanRoot,
    CurrentDirectory,
}

#[derive(Debug, Default, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum AnalysisExportDetail {
    #[default]
    Summary,
    Full,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisExportScopeInfo {
    pub path: String,
    pub scanned_at_ms: u64,
    pub total_bytes: u64,
    pub file_count: u64,
    pub coverage: AnalysisCoverage,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisExportOptions {
    pub schema_version: u8,
    pub scan_id: u64,
    pub scan_mode: AnalysisScanMode,
    pub scan_root: AnalysisExportScopeInfo,
    pub current_directory: AnalysisExportScopeInfo,
}

/// Metadata needed to verify an export, without cloning the projected chart and file entries.
pub(super) struct AnalysisExportSnapshot {
    pub root: String,
    pub index_snapshot: Option<AnalysisIndexSnapshot>,
    pub total_bytes: u64,
    pub scanned_at_ms: u64,
    pub scan_mode: AnalysisScanMode,
}

pub(super) fn unavailable() -> CoreError {
    CoreError::invalid_input("the matching disk-analysis index is unavailable; analyze again")
        .with_reason(CoreErrorReason::AnalysisExportUnavailable)
}

fn latest_generation(cache: &IndexReadView<'_>, root: &Path) -> Option<u64> {
    cache
        .publish_generations
        .iter()
        .filter(|(scope, _)| root.starts_with(scope) || scope.starts_with(root))
        .map(|(_, generation)| *generation)
        .max()
}

pub(in crate::storage) fn index_snapshot(
    cache: &IndexReadView<'_>,
    root: &Path,
) -> Option<AnalysisIndexSnapshot> {
    Some(AnalysisIndexSnapshot {
        root: root.to_path_buf(),
        generation: latest_generation(cache, root)?,
        mutation_revision: cache.mutation_revision,
    })
}

/// No filesystem reads are permitted here: an unavailable snapshot must never trigger a rescan.
fn with_snapshot<T>(
    scan_id: u64,
    read: impl FnOnce(&IndexReadView<'_>, &Path, &Path, &crate::ScanExclusionOptions) -> CoreResult<T>,
) -> CoreResult<T> {
    let (result, exclusions) = super::session::export_result(scan_id)?;
    let reject = |reason: &str| {
        log::info!(
            "analysis_export_snapshot_unavailable scan_id={} root={} reason={}",
            scan_id,
            mangodisk_platform::diagnostics::text(&result.root),
            reason
        );
        unavailable()
    };
    let identity = result
        .index_snapshot
        .as_ref()
        .ok_or_else(|| reject("index_not_retained"))?;
    cache::read_snapshot(|cache| {
        if cache.mutation_revision != identity.mutation_revision
            || latest_generation(&cache, &identity.root) != Some(identity.generation)
        {
            log::info!("analysis_export_revision_changed scan_id={} expected_generation={} actual_generation={:?} expected_mutation_revision={} actual_mutation_revision={}", scan_id, identity.generation, latest_generation(&cache, &identity.root), identity.mutation_revision, cache.mutation_revision);
            return Err(reject("index_revision_changed"));
        }
        let aggregate = cache
            .directories
            .get(&identity.root)
            .ok_or_else(|| reject("directory_not_retained"))?;
        if aggregate.bytes != result.total_bytes
            || aggregate.scanned_at_ms != result.scanned_at_ms
            || AnalysisScanMode::from(aggregate.byte_metric) != result.scan_mode
        {
            return Err(reject("measurement_changed"));
        }
        let scan_root = cache
            .scan_roots
            .keys()
            .filter(|root| identity.root.starts_with(root))
            .max_by_key(|root| root.components().count())
            .ok_or_else(|| reject("scan_scope_not_retained"))?;
        // Exporting an ancestor must not silently incorporate a later sibling scan.
        if latest_generation(&cache, scan_root) != Some(identity.generation) {
            return Err(reject("scan_scope_revision_changed"));
        }
        read(&cache, scan_root, &identity.root, &exclusions)
    })?
}

fn scope_info(cache: &IndexReadView<'_>, root: &Path) -> CoreResult<AnalysisExportScopeInfo> {
    let aggregate = cache.directories.get(root).ok_or_else(unavailable)?;
    Ok(AnalysisExportScopeInfo {
        path: display_path(root),
        scanned_at_ms: aggregate.scanned_at_ms,
        total_bytes: aggregate.bytes,
        file_count: aggregate.file_count,
        coverage: AnalysisCoverage::new(aggregate.omissions, aggregate.skipped_count),
    })
}

pub(super) fn options(scan_id: u64) -> CoreResult<AnalysisExportOptions> {
    let started = Instant::now();
    let result = with_snapshot(scan_id, |cache, scan_root, current, _| {
        Ok(AnalysisExportOptions {
            schema_version: 1,
            scan_id,
            scan_mode: cache.directories[current].byte_metric.into(),
            scan_root: scope_info(cache, scan_root)?,
            current_directory: scope_info(cache, current)?,
        })
    });
    log::info!(
        "analysis_export_options_finished scan_id={scan_id} outcome={} elapsed_ms={}",
        if result.is_ok() {
            "ready"
        } else {
            "unavailable"
        },
        started.elapsed().as_millis()
    );
    result
}

pub(super) fn prepare(
    scan_id: u64,
    scope: AnalysisExportScope,
    include_full_paths: bool,
    detail: AnalysisExportDetail,
    min_directory_bytes: u64,
) -> CoreResult<AnalysisReport> {
    let started = Instant::now();
    log::info!("analysis_export_prepare_started scan_id={scan_id} scope={scope:?} include_full_paths={include_full_paths} detail={detail:?} min_directory_bytes={min_directory_bytes}");
    let report = with_snapshot(scan_id, |cache, scan_root, current, configured| {
        let root = match scope {
            AnalysisExportScope::ScanRoot => scan_root,
            AnalysisExportScope::CurrentDirectory => current,
        };
        let aggregate = cache.directories.get(root).ok_or_else(unavailable)?;
        let root_name = root
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| {
                if cfg!(windows) {
                    "disk".into()
                } else {
                    "/".into()
                }
            });
        let path =
            |path: &Path| -> CoreResult<String> { report_path(path, root, include_full_paths) };
        let retained: Vec<_> = cache.directories
            .iter()
            .filter(|(directory, _)| directory.starts_with(root))
            .map(|(directory, aggregate)| (directory.as_path(), aggregate))
            .collect();
        let selected = if detail == AnalysisExportDetail::Summary || min_directory_bytes > 0 {
            super::export_summary::select(
                root, &retained,
                if detail == AnalysisExportDetail::Summary { super::export_summary::SUMMARY_DIRECTORY_LIMIT } else { usize::MAX },
                min_directory_bytes,
            )?
        } else {
            (0..retained.len())
                .map(|index| super::export_summary::SelectedDirectory { index, collapsed: None, filtered: None })
                .collect()
        };
        let range = retained.iter().fold([u64::MAX, 0], |range, (_, aggregate)| {
            [range[0].min(aggregate.scanned_at_ms), range[1].max(aggregate.scanned_at_ms)]
        });
        let has_hidden_details = selected.iter().any(|row| row.collapsed.is_some() || row.filtered.is_some());
        let collapsed_count: usize = selected.iter().filter_map(|row| row.collapsed.as_ref()).map(|bucket| bucket.directory_count).sum();
        let filtered_count: usize = selected.iter().filter_map(|row| row.filtered.as_ref()).map(|bucket| bucket.directory_count).sum();
        let scanned_at_range_ms = (has_hidden_details && range[0] != range[1]).then_some(range);
        let mut directories = Vec::with_capacity(selected.len());
        for row in selected {
            let (directory, aggregate) = retained[row.index];
            directories.push(AnalysisReportDirectory {
                path: path(directory)?,
                bytes: aggregate.bytes,
                file_count: aggregate.file_count,
                scanned_at_ms: (aggregate.scanned_at_ms != cache.directories[root].scanned_at_ms)
                    .then_some(aggregate.scanned_at_ms),
                size_status: super::coverage::entry_size_status(*aggregate),
                omissions: aggregate.omissions.with_unclassified(aggregate.skipped_count),
                path_is_lossy: directory.to_str().is_none(),
                collapsed: row.collapsed,
                filtered: row.filtered,
            });
        }
        log::info!("analysis_export_detail_selected scan_id={scan_id} detail={detail:?} retained_directories={} exported_directories={} collapsed_directories={collapsed_count} filtered_directories={filtered_count} min_directory_bytes={min_directory_bytes}", retained.len(), directories.len());
        let mut exclusions = AnalysisReportExclusions::default();
        if let Some((paths, _)) = cache.scan_exclusions.get(scan_root) {
            exclusions.paths = paths
                .iter()
                .filter(|path| path.starts_with(root))
                .map(|p| path(p))
                .collect::<CoreResult<_>>()?;
            exclusions.name_rules = configured.names.clone();
        }
        let scan = AnalysisReportScan {
            id: scan_id,
            root: if include_full_paths {
                path(root)?
            } else {
                root_name.clone()
            },
            root_name,
            scanned_at_ms: aggregate.scanned_at_ms,
            scan_mode: aggregate.byte_metric.into(),
            total_bytes: aggregate.bytes,
            subtree_of_original_scan: root != scan_root,
        };
        Ok((scan, directories, exclusions, scanned_at_range_ms))
    })
    .map(|(scan, directories, exclusions, scanned_at_range_ms)| {
        AnalysisReport::new(
            scan, directories, exclusions, include_full_paths, scanned_at_range_ms, min_directory_bytes,
        )
    });
    match &report {
        Ok(report) => log::info!("analysis_export_prepare_finished scan_id={scan_id} directories={} total_bytes={} elapsed_ms={}", report.directories.len(), report.total_bytes(), started.elapsed().as_millis()),
        Err(error) => log::warn!("analysis_export_prepare_failed scan_id={scan_id} stage=snapshot code={:?} error={} elapsed_ms={}", error.code(), mangodisk_platform::diagnostics::text(error), started.elapsed().as_millis()),
    }
    report
}

fn report_path(path: &Path, root: &Path, full: bool) -> CoreResult<String> {
    if full {
        return Ok(display_path(path));
    }
    let relative = path.strip_prefix(root).map_err(|_| unavailable())?;
    if relative.as_os_str().is_empty() {
        return Ok(".".into());
    }
    Ok(relative
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/"))
}

#[cfg(test)]
#[path = "export_tests.rs"]
mod tests;
