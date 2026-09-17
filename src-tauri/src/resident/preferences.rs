use super::{diagnostics::Failure, runtime::ResidentState};
use std::sync::Arc;
use tauri::Manager;
use tauri_plugin_store::StoreExt;

use super::preference_schema::decode;
pub use super::preference_schema::ResidentPreferences;
const FILE: &str = "resident.json";

pub fn load(app: &tauri::AppHandle) -> ResidentPreferences {
    let value = match app.store_builder(FILE).disable_auto_save().build() {
        Ok(store) => store.get("preferences"),
        Err(error) => {
            Failure::record("preferences_load", &error);
            return ResidentPreferences::default();
        }
    };
    match value {
        None => ResidentPreferences::default(),
        Some(value) => match decode(value) {
            Ok(preferences) => preferences,
            _ => {
                log::warn!("resident_preferences_invalid");
                ResidentPreferences::default()
            }
        },
    }
}

fn save(app: &tauri::AppHandle, preferences: &ResidentPreferences) -> Result<(), Failure> {
    if preferences.schema_version != 8 {
        return Err(Failure::state("preferences_version"));
    }
    let store = app
        .store_builder(FILE)
        .disable_auto_save()
        .build()
        .map_err(|error| Failure::record("preferences_open", &error))?;
    let previous = store.get("preferences");
    if let Some(value) = &previous {
        decode(value.clone()).map_err(Failure::state)?;
    }
    store.set(
        "preferences",
        serde_json::to_value(preferences)
            .map_err(|error| Failure::record("preferences_encode", &error))?,
    );
    if let Err(error) = store.save() {
        // Restore the plugin cache too: otherwise a later save could persist a
        // preference that the UI correctly reported as rejected.
        if let Some(previous) = previous {
            store.set("preferences", previous);
        } else {
            store.delete("preferences");
        }
        return Err(Failure::record("preferences_save", &error));
    }
    Ok(())
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
        .reading
        .lock()
        .map_err(|_| Failure::state("preferences_reading"))?
        .clone();
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
        }
    }
    if preferences.enabled {
        super::panel::prewarm(app);
    } else {
        super::panel::hide(app);
    }
    state.wake();
    log::info!(
        "resident_preferences_saved enabled={} revision={} show_icon={} metrics={:?} mode={:?} position={:?} background={} compact={} menu_bar_compact={} usage_colors={} warning_percent={} critical_percent={} network_manual={} disk_manual={} elapsed_ms={}",
        preferences.enabled,
        preferences.revision,
        preferences.effective_icon(),
        preferences
            .metrics
            .iter()
            .filter(|metric| metric.enabled)
            .map(|metric| metric.id)
            .collect::<Vec<_>>(),
        preferences.windows_display_mode,
        preferences.taskbar_position,
        preferences.taskbar_background,
        preferences.taskbar_compact,
        preferences.menu_bar_compact,
        preferences.usage_colors,
        preferences.usage_warning_percent,
        preferences.usage_critical_percent,
        preferences.network_interface.is_some(),
        preferences.disk_volume.is_some(),
        started.elapsed().as_millis()
    );
    Ok(preferences)
}
