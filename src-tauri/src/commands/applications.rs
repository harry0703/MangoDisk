use crate::services::application_icon::{ApplicationIcon, ApplicationIconService};
use crate::services::application_uninstall_catalog::ApplicationUninstallCatalogCache;
use mangodisk_core::{
    ApplicationCloseBatchResult, ApplicationLeftoverPlanItem, ApplicationLeftoverResult,
    ApplicationLeftoverScanResult, ApplicationLeftoverService, ApplicationUninstallBatchPlan,
    ApplicationUninstallBatchPreparation, ApplicationUninstallBatchResult,
    ApplicationUninstallBatchSelection, ApplicationUninstallCloseRequest,
    ApplicationUninstallScanResult, ApplicationUninstallService, CoreError,
};
use serde::Serialize;
use std::time::Instant;
use tauri::Manager;

use super::error::{run_blocking, run_blocking_value, CommandResult};
use crate::events;

#[tauri::command]
pub async fn get_application_icons(
    app: tauri::AppHandle,
    paths: Vec<String>,
) -> CommandResult<Vec<ApplicationIcon>> {
    let requested_count = paths.len().min(ApplicationIconService::MAX_REQUESTS);
    let cache_root = app
        .path()
        .app_cache_dir()
        .map(|path| path.join("cache").join("application-icons"))
        .map_err(|error| {
            log::warn!("application_icon_cache_path_unavailable error={error}");
            error
        })
        .ok();
    let started_at = Instant::now();
    let result = run_blocking_value("get_application_icons", move || {
        ApplicationIconService::load(paths, cache_root)
    })
    .await?;

    log::info!(
        "application_icons_loaded requested={} resolved={} cache_hits={} decoded={} elapsed_ms={}",
        requested_count,
        result.icons.len(),
        result.cache_hits,
        result.decoded_icons,
        started_at.elapsed().as_millis()
    );
    Ok(result.icons)
}

#[tauri::command]
pub async fn scan_application_leftovers() -> CommandResult<ApplicationLeftoverScanResult> {
    run_blocking(
        "scan_application_leftovers",
        ApplicationLeftoverService::scan,
    )
    .await
}

#[tauri::command]
pub async fn scan_application_uninstall_catalog(
    app: tauri::AppHandle,
) -> CommandResult<ApplicationUninstallScanResult> {
    run_blocking("scan_application_uninstall_catalog", move || {
        let event_app = app.clone();
        let result = ApplicationUninstallService::scan_with_progress(move |progress| {
            events::emit(&event_app, events::APPLICATION_UNINSTALL_PROGRESS, progress);
        })?;
        let catalog_cache = app.state::<ApplicationUninstallCatalogCache>();
        ApplicationUninstallCatalogCache::replace(&catalog_cache, &result);
        Ok::<ApplicationUninstallScanResult, CoreError>(result)
    })
    .await
}

#[tauri::command]
pub fn cancel_application_uninstall_catalog_scan() {
    ApplicationUninstallService::cancel_scan();
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplicationUninstallCloseResponse {
    close_result: ApplicationCloseBatchResult,
    catalog: ApplicationUninstallScanResult,
}

#[tauri::command]
pub async fn close_application_uninstall_applications(
    app: tauri::AppHandle,
    request: ApplicationUninstallCloseRequest,
    catalog_revision: String,
) -> CommandResult<ApplicationUninstallCloseResponse> {
    run_blocking("close_application_uninstall_applications", move || {
        let catalog_cache = app.state::<ApplicationUninstallCatalogCache>();
        let mut catalog = ApplicationUninstallCatalogCache::find(&catalog_cache, &catalog_revision)
            .ok_or_else(|| CoreError::operation_failed("application uninstall catalog changed"))?;
        let close_result =
            ApplicationUninstallService::close_applications_from_catalog(request, &mut catalog)?;
        ApplicationUninstallCatalogCache::replace(&catalog_cache, &catalog);
        Ok::<ApplicationUninstallCloseResponse, CoreError>(ApplicationUninstallCloseResponse {
            close_result,
            catalog,
        })
    })
    .await
}

#[tauri::command]
pub async fn prepare_application_uninstall_batch(
    app: tauri::AppHandle,
    selections: Vec<ApplicationUninstallBatchSelection>,
    catalog_revision: String,
) -> CommandResult<ApplicationUninstallBatchPreparation> {
    run_blocking("prepare_application_uninstall_batch", move || {
        let catalog_cache = app.state::<ApplicationUninstallCatalogCache>();
        if let Some(catalog) =
            ApplicationUninstallCatalogCache::find(&catalog_cache, &catalog_revision)
        {
            log::info!("application_uninstall_prepare_catalog_cache_hit");
            return ApplicationUninstallService::prepare_batch_from_catalog(&selections, &catalog);
        }
        log::info!("application_uninstall_prepare_catalog_cache_miss");
        ApplicationUninstallService::prepare_batch(&selections)
    })
    .await
}

#[tauri::command]
pub async fn execute_application_uninstall_batch(
    app: tauri::AppHandle,
    plan: ApplicationUninstallBatchPlan,
    dry_run: bool,
    authorization_prompt: String,
) -> CommandResult<ApplicationUninstallBatchResult> {
    run_blocking("execute_application_uninstall_batch", move || {
        ApplicationUninstallService::execute_batch_with_progress(
            plan,
            dry_run,
            Some(&authorization_prompt),
            move |progress| {
                events::emit(
                    &app,
                    events::APPLICATION_UNINSTALL_EXECUTION_PROGRESS,
                    progress,
                );
            },
        )
    })
    .await
}

#[tauri::command]
pub fn cancel_application_uninstall_execution() {
    ApplicationUninstallService::cancel_execution();
}

#[tauri::command]
pub async fn execute_application_leftovers(
    reviewed_items: Vec<ApplicationLeftoverPlanItem>,
    dry_run: bool,
    deep_cleanup_operation_id: String,
) -> CommandResult<ApplicationLeftoverResult> {
    run_blocking("execute_application_leftovers", move || {
        ApplicationLeftoverService::execute_reviewed(
            reviewed_items,
            dry_run,
            deep_cleanup_operation_id,
        )
    })
    .await
}

#[tauri::command]
pub fn cancel_application_leftovers() {
    ApplicationLeftoverService::cancel();
}

/// Keeps the system fallback limited to one fixed destination. The WebView cannot supply a
/// command, registry key, or custom URI, and opening Settings never counts as an uninstall.
#[tauri::command]
pub async fn open_windows_installed_apps() -> CommandResult<()> {
    run_blocking("open_windows_installed_apps", open_installed_apps_settings).await
}

#[cfg(windows)]
fn open_installed_apps_settings() -> Result<(), CoreError> {
    log::info!("windows_installed_apps_open_requested destination=appsfeatures");
    tauri_plugin_opener::open_url("ms-settings:appsfeatures", None::<&str>).map_err(|error| {
        log::warn!(
            "windows_installed_apps_open_failed error_digest={}",
            blake3::hash(error.to_string().as_bytes()).to_hex()
        );
        CoreError::operation_failed("Windows installed apps settings could not be opened")
    })?;
    log::info!("windows_installed_apps_open_finished outcome=opened");
    Ok(())
}

#[cfg(not(windows))]
fn open_installed_apps_settings() -> Result<(), CoreError> {
    Err(CoreError::operation_failed(
        "Windows installed apps settings are unavailable on this platform",
    ))
}

#[tauri::command]
pub async fn remove_application_record(
    app: tauri::AppHandle,
    application_id: String,
) -> CommandResult<()> {
    run_blocking("remove_application_record", move || {
        let result = ApplicationUninstallService::remove_application_record(&application_id, false);
        // Verification can fail after a committed native deletion. Never keep an adapter cache
        // that would present the old record as fresh after either a success or an uncertain result.
        app.state::<ApplicationUninstallCatalogCache>().clear();
        result
    })
    .await
}

/// Correlate the item the user opened with native diagnostics without displaying support IDs.
/// Only IDs from the current Core-produced catalog are logged; caller-supplied text is discarded.
#[tauri::command]
pub fn log_application_uninstall_details(
    app: tauri::AppHandle,
    application_id: String,
    catalog_revision: String,
) {
    let cache = app.state::<ApplicationUninstallCatalogCache>();
    let Some(catalog) = cache.find(&catalog_revision) else {
        return;
    };
    if let Some(candidate) = catalog
        .candidates
        .iter()
        .find(|candidate| candidate.application_id == application_id)
    {
        log::info!("application_uninstall_details_opened application_id={} capability={:?} record_state={:?} reason={}", candidate.application_id, candidate.capability, candidate.record_state, candidate.uninstall_diagnostic.map_or("none", |reason| reason.stable_code()));
    }
}
