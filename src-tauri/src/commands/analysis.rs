use mangodisk_core::{
    AnalysisExportFormat, AnalysisExportReceipt, AnalysisExportRequest, AnalysisRemainderPage,
    AnalysisRemainderRequest, AnalysisResult, AnalysisScanMode, AnalysisService,
};
use mangodisk_core::{ScanExclusionOptions, ScanNameExclusion};
use tauri_plugin_dialog::DialogExt;

use crate::events;

use super::error::{run_blocking, CommandResult};

#[tauri::command]
pub async fn export_analysis_result(
    window: tauri::WebviewWindow,
    request: AnalysisExportRequest,
    dialog_title: String,
) -> CommandResult<Option<AnalysisExportReceipt>> {
    // Retain one immutable report across the native picker; navigation and later scans cannot
    // replace its data while the user selects a destination. No write path comes from the WebView.
    let scan_id = request.scan_id;
    let report = run_blocking("prepare_analysis_export", move || {
        AnalysisService::prepare_export(request)
    })
    .await?;
    let format = report.format();
    let (sender, receiver) = tokio::sync::oneshot::channel();
    window
        .dialog()
        .file()
        .set_parent(&window)
        .set_title(dialog_title)
        .set_file_name(report.suggested_file_name())
        .add_filter(
            match format {
                AnalysisExportFormat::Json => "JSON",
                AnalysisExportFormat::Markdown => "Markdown",
            },
            &[format.extension()],
        )
        .save_file(move |path| {
            let _ = sender.send(path);
        });
    let selected = super::error::into_command_result("analysis_export_dialog", receiver.await)?;
    let Some(selected) = selected else {
        log::info!(
            "analysis_export_cancelled scan_id={scan_id} stage=file_dialog outcome=not_saved"
        );
        return Ok(None);
    };
    let path =
        super::error::into_command_result("analysis_export_destination", selected.into_path())?;
    run_blocking("save_analysis_export", move || report.save(&path).map(Some)).await
}

#[tauri::command]
pub async fn analyze_path(
    app: tauri::AppHandle,
    path: Option<String>,
    refresh: bool,
    excluded_paths: Vec<String>,
    excluded_names: Option<Vec<ScanNameExclusion>>,
    scan_mode: Option<AnalysisScanMode>,
) -> CommandResult<AnalysisResult> {
    run_blocking("analyze_path", move || {
        AnalysisService::analyze_with_mode_progress(
            path,
            refresh,
            ScanExclusionOptions {
                paths: excluded_paths,
                names: excluded_names.unwrap_or_default(),
            },
            scan_mode.unwrap_or_default(),
            move |progress| {
                events::emit(&app, events::ANALYSIS_PROGRESS, progress);
            },
        )
    })
    .await
}

#[tauri::command]
pub fn cancel_analysis() {
    AnalysisService::cancel();
}

#[tauri::command]
pub async fn list_analysis_remainder(
    request: AnalysisRemainderRequest,
) -> CommandResult<AnalysisRemainderPage> {
    run_blocking("list_analysis_remainder", move || {
        AnalysisService::list_remainder(request)
    })
    .await
}

#[tauri::command]
pub async fn release_analysis_remainder(snapshot_id: u64) -> CommandResult<()> {
    run_blocking("release_analysis_remainder", move || {
        AnalysisService::release_remainder(snapshot_id)
    })
    .await
}
