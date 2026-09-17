use crate::services::app_updates::{AppUpdates, CheckSource, Notice};
use serde::Serialize;
use std::sync::Arc;
use tauri::Manager;

#[tauri::command]
pub(crate) fn get_app_update_notice(state: tauri::State<'_, Arc<AppUpdates>>) -> Notice {
    state.snapshot()
}

/// A WebView-owned clone preserves the plugin's signed download/install flow
/// while native discovery keeps its own cached update across window lifetimes.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpdateResource {
    rid: tauri::ResourceId,
    current_version: String,
    version: String,
    date: Option<String>,
    body: Option<String>,
    raw_json: serde_json::Value,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AcquiredUpdate {
    schema_version: u32,
    update: Option<UpdateResource>,
}

#[tauri::command]
pub(crate) async fn acquire_app_update(
    webview: tauri::Webview,
    state: tauri::State<'_, Arc<AppUpdates>>,
    refresh: bool,
) -> Result<AcquiredUpdate, String> {
    if webview.label() != "main" {
        return Err("update_actions_require_main_window".into());
    }
    let source = if refresh {
        CheckSource::Manual
    } else {
        CheckSource::Cached
    };
    let update = state.check(webview.app_handle(), source).await?;
    let update = update.map(|update| {
        let current_version = update.current_version.clone();
        let version = update.version.clone();
        let date = update
            .raw_json
            .get("pub_date")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned);
        let body = update.body.clone();
        let raw_json = update.raw_json.clone();
        let rid = webview.resources_table().add(update);
        UpdateResource {
            rid,
            current_version,
            version,
            date,
            body,
            raw_json,
        }
    });
    log::info!(
        "app_update_resource_acquired source={source:?} version={}",
        update.as_ref().map_or("none", |u| u.version.as_str())
    );
    Ok(AcquiredUpdate {
        schema_version: 1,
        update,
    })
}
