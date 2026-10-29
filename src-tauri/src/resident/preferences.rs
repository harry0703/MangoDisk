use super::{diagnostics::Failure, runtime::ResidentState};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use tauri::{Emitter, Manager};

pub use super::preference_schema::ResidentPreferences;
use super::preference_schema::{decode, PREFERENCE_SCHEMA_VERSION};
const FILE: &str = "resident.json";

pub fn load(app: &tauri::AppHandle) -> ResidentPreferences {
    let path = match path(app) {
        Ok(path) => path,
        Err(_) => return ResidentPreferences::default(),
    };
    let value = match super::preference_file::read(&path) {
        Ok(mut document) => document.remove("preferences"),
        Err(error) => {
            Failure::record("preferences_load", &error);
            return ResidentPreferences::default();
        }
    };
    match value {
        None => ResidentPreferences::default(),
        Some(value) => {
            let previous_version = value.get("schemaVersion").and_then(|v| v.as_u64());
            match decode(value) {
                Ok(preferences) => {
                    if previous_version != Some(u64::from(PREFERENCE_SCHEMA_VERSION)) {
                        log::info!("resident_preferences_migrated from_schema={previous_version:?} to_schema={} persisted=false outcome=loaded_current save_policy=next_explicit_save",
                            preferences.schema_version);
                    }
                    preferences
                }
                Err(reason) => {
                    log::warn!("resident_preferences_invalid schema={previous_version:?} reason={reason} outcome=runtime_defaults_original_preserved");
                    ResidentPreferences::default()
                }
            }
        }
    }
}

fn save(app: &tauri::AppHandle, preferences: &ResidentPreferences) -> Result<(), Failure> {
    save_to_path(&path(app)?, preferences)
}

fn save_to_path(path: &Path, preferences: &ResidentPreferences) -> Result<(), Failure> {
    if preferences.schema_version != PREFERENCE_SCHEMA_VERSION {
        return Err(Failure::state("preferences_version"));
    }
    let mut document = super::preference_file::read(path)
        .map_err(|error| Failure::record("preferences_open", &error))?;
    let previous = document.get("preferences");
    if let Some(value) = previous {
        if let Err(reason) = decode(value.clone()) {
            let stored_version = value.get("schemaVersion").and_then(|value| value.as_u64());
            if reason != "preferences_version" || stored_version.is_none() {
                log::warn!("resident_preferences_save_rejected stored_schema={stored_version:?} requested_schema={} reason={reason} outcome=original_preserved", preferences.schema_version);
                return Err(Failure::state(reason));
            }
            log::warn!("resident_preferences_unknown_schema stored_schema={stored_version:?} requested_schema={} action=replace_on_explicit_save", preferences.schema_version);
        }
    }
    document.insert(
        "preferences".into(),
        serde_json::to_value(preferences)
            .map_err(|error| Failure::record("preferences_encode", &error))?,
    );
    let content = serde_json::to_vec_pretty(&document)
        .map_err(|error| Failure::record("preferences_encode", &error))?;
    super::preference_file::write(path, &content)
        .map_err(|error| Failure::record("preferences_save", &error))
}

fn path(app: &tauri::AppHandle) -> Result<PathBuf, Failure> {
    app.path()
        .app_data_dir()
        .map(|directory| directory.join(FILE))
        .map_err(|error| Failure::record("preferences_path", &error))
}

/// Desktop policy stays here; IPC only transports the requested preference value.
pub fn apply(
    app: &tauri::AppHandle,
    preferences: ResidentPreferences,
) -> Result<ResidentPreferences, Failure> {
    let started = std::time::Instant::now();
    let mut preferences = preferences.normalize().map_err(Failure::state)?;
    let state = app.state::<Arc<ResidentState>>();
    // Native calls and persistence may block. Serialize writers separately so
    // samplers continue using the last committed settings throughout the update.
    // Presentation takes this same gate before reading settings, preventing a
    // queued refresh from restoring the old display after commit or rollback.
    let _update = state
        .preference_update
        .lock()
        .map_err(|_| Failure::state("preferences_update_lock"))?;
    let current = state
        .preferences
        .lock()
        .map_err(|_| Failure::state("preferences_lock"))?
        .clone();
    if preferences.revision != current.revision {
        return Err(Failure::state("preferences_conflict"));
    }
    preferences.revision = current
        .revision
        .checked_add(1)
        .ok_or_else(|| Failure::state("preferences_revision"))?;
    if !preferences.enabled {
        super::main_window::open(
            app,
            super::main_window::Destination::Main,
            "disable_resident",
        )
        .map_err(|error| Failure::record("preferences_foreground", &error))?;
    }
    let reading = state
        .published_reading
        .lock()
        .map_err(|_| Failure::state("preferences_reading"))?
        .clone();
    #[cfg(any(windows, target_os = "macos", target_os = "linux"))]
    super::floating_monitor::prepare(app, &preferences)
        .map_err(|error| Failure::record("floating_prepare", &error))?;
    if let Err(error) = super::tray_display::apply_preferences(app, &preferences, &reading) {
        if let Err(rollback) = super::tray_display::apply_preferences(app, &current, &reading) {
            Failure::record("preferences_display_rollback", &rollback);
            super::main_window::request(
                app,
                super::main_window::Destination::Main,
                "display_rollback",
            );
        }
        return Err(Failure::record("preferences_display", &error));
    }
    if let Err(error) = save(app, &preferences) {
        if let Err(rollback) = super::tray_display::apply_preferences(app, &current, &reading) {
            Failure::record("preferences_display_rollback", &rollback);
        }
        return Err(error);
    }
    {
        // Publish only after both native application and persistence succeed.
        // No platform calls or file I/O may run while holding this snapshot lock.
        let mut committed = state
            .preferences
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        *committed = preferences.clone();
        if !preferences.enabled {
            let mut reading = state
                .reading
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            reading.revision += 1;
            reading.resources = Default::default();
            *state
                .published_reading
                .lock()
                .unwrap_or_else(|error| error.into_inner()) = reading.clone();
        }
    }
    #[cfg(windows)]
    if preferences.enabled
        && (!current.enabled || current.windows_display_mode != preferences.windows_display_mode)
    {
        super::floating_monitor::show(app);
    }
    #[cfg(target_os = "macos")]
    if preferences.enabled
        && (!current.enabled || current.macos_display_mode != preferences.macos_display_mode)
    {
        super::floating_monitor::show(app);
    }
    #[cfg(target_os = "linux")]
    if preferences.enabled
        && (!current.enabled || current.linux_display_mode != preferences.linux_display_mode)
    {
        super::floating_monitor::show(app);
    }
    if preferences.enabled {
        super::panel::prewarm(app);
    } else {
        super::panel::hide(app);
    }
    if let Some(network) = app.try_state::<Arc<super::public_network::PublicNetwork>>() {
        network.configure(&preferences);
    }
    let _ = app.emit("resident-preferences-changed", preferences.revision);
    state.wake();
    log::info!(
        "resident_preferences_saved enabled={} revision={} show_icon={} windows_mode={:?} macos_mode={:?} linux_mode={:?} floating_layout={:?} floating_top={} floating_locked={} floating_click_through={} floating_background_opacity={} floating_snap_to_edges={} floating_font_size={:?} floating_bold={} floating_background={:?} metrics={:?} position={:?} background={} compact={} menu_bar_compact={} usage_colors={} color_rules={:?} network_manual={} disk_manual={} gpu_adapter={} gpu_display={:?} gpu_temperature_display={:?} elapsed_ms={}",
        preferences.enabled,
        preferences.revision,
        preferences.effective_icon(),
        preferences.windows_display_mode,
        preferences.macos_display_mode,
        preferences.linux_display_mode,
        preferences.floating.layout,
        preferences.floating.always_on_top,
        preferences.floating.locked,
        preferences.floating.click_through,
        preferences.floating.background_opacity, preferences.floating.snap_to_edges,
        preferences.floating.font_size, preferences.floating.bold, preferences.floating.background,
        preferences
            .metrics
            .iter()
            .filter(|metric| metric.enabled)
            .map(|metric| metric.id)
            .collect::<Vec<_>>(),
        preferences.taskbar_position,
        preferences.taskbar_background,
        preferences.taskbar_compact,
        preferences.menu_bar_compact,
        preferences.usage_colors,
        preferences.color_rules,
        preferences.network_interface.is_some(),
        preferences.disk_volume.is_some(),
        mangodisk_platform::diagnostics::text(preferences.gpu_adapter.as_deref().unwrap_or("automatic")),
        preferences.gpu_display,
        preferences.gpu_temperature_display,
        started.elapsed().as_millis()
    );
    Ok(preferences)
}

#[cfg(test)]
#[path = "preferences_tests.rs"]
mod tests;
