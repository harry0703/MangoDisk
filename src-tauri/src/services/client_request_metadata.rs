//! Desktop facts for first-party HTTP requests; missing telemetry never blocks a service.
use mangodisk_core::http_client::OfficialRequestMetadata;
use tauri_plugin_store::StoreExt;

#[cfg(target_os = "windows")]
mod native_arch;
mod os_version;

pub(crate) fn collect(app: &tauri::AppHandle) -> OfficialRequestMetadata {
    let metadata = OfficialRequestMetadata {
        locale: super::native_labels::NativeLabels::load(app)
            .locale
            .to_string(),
        distribution: crate::commands::app_distribution::current()
            .diagnostic_name()
            .into(),
        os_version: Some(os_version::telemetry_os_version()),
        system_locale: tauri_plugin_os::locale(),
        install_id: read_install_id(app, std::path::Path::new("installation.json")),
        request_id: Some(uuid::Uuid::new_v4().to_string()),
        ..Default::default()
    };
    #[cfg(target_os = "windows")]
    let metadata = OfficialRequestMetadata {
        native_arch: native_arch::telemetry_native_arch().map(str::to_owned),
        ..metadata
    };
    metadata
}

pub(crate) fn log_request(service: &'static str, metadata: &OfficialRequestMetadata) {
    log::info!(
        "official_http_request_started service={service} request_id={} install_id_present={}",
        mangodisk_platform::diagnostics::text(metadata.request_id.as_deref().unwrap_or("none")),
        metadata.install_id.is_some()
    );
}

fn read_install_id<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    path: &std::path::Path,
) -> Option<String> {
    // Read only: the frontend owns identity creation, including its first-write transaction.
    if let Some(store) = app.get_store(path) {
        return install_id(store.get("identity").as_ref());
    }
    let path = tauri_plugin_store::resolve_store_path(app, path).ok()?;
    let value: serde_json::Value = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    install_id(value.get("identity"))
}
fn install_id(value: Option<&serde_json::Value>) -> Option<String> {
    let value = value?;
    if value.get("schemaVersion")?.as_u64()? != 1 {
        return None;
    }
    uuid::Uuid::parse_str(value.get("installId")?.as_str()?)
        .ok()
        .map(|id| id.to_string())
}

#[cfg(test)]
#[path = "client_request_metadata_tests.rs"]
pub(crate) mod tests;
