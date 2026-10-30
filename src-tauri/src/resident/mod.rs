mod application_icons;
pub mod autostart;
mod battery_sampling;
mod color_rules;
pub(crate) mod diagnostics;
mod disk_health_sampling;
mod disk_sensor_sampling;
mod disk_temperature_sampling;
pub mod floating_monitor;
pub mod main_window;
mod memory_automatic;
pub mod memory_preferences;
pub mod memory_release;
pub mod memory_window;
pub mod network_traffic;
pub mod panel;
pub mod panel_page;
mod power_sensor_sampling;
mod preference_file;
mod preference_schema;
pub mod preferences;
mod presentation;
mod process_cpu_sampling;
pub mod public_network;
pub mod runtime;
mod sampling_diagnostics;
mod sampling_dispatch;
mod sampling_policy;
mod sampling_reading;
mod sampling_schedule;
mod sampling_workers;
#[cfg(windows)]
mod windows_frame;

use std::sync::Arc;
use tauri::Manager;
pub mod taskbar_display;
pub mod tray_display;

pub const PANEL_LABEL: &str = "tray-panel";
pub const TRAY_ID: &str = "resident";

pub fn install(app: &tauri::AppHandle) -> tauri::Result<()> {
    #[cfg(target_os = "linux")]
    application_icons::install();
    #[cfg(windows)]
    taskbar_display::install(app);
    #[cfg(any(windows, target_os = "macos", target_os = "linux"))]
    floating_monitor::install(app);
    tray_display::install(app)?;
    memory_preferences::install(app);
    let preferences = preferences::load(app);
    let state = runtime::start(app, preferences.clone());
    public_network::install(app, &preferences);
    let reading = state
        .reading
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clone();
    tray_display::apply_preferences(app, &preferences, &reading)?;
    state
        .display_initialized
        .store(true, std::sync::atomic::Ordering::Release);
    state.wake();
    #[cfg(any(windows, target_os = "macos", target_os = "linux"))]
    floating_monitor::prewarm(app);
    log::info!("resident_started enabled={}", preferences.enabled);
    Ok(())
}

/// macOS keeps its native close/reopen convention; Windows stays resident only
/// while the tray feature is enabled. Explicit Quit never enters this handler.
pub fn handle_window_event(window: &tauri::Window, event: &tauri::WindowEvent) {
    if window.label() != crate::MAIN_WINDOW_LABEL {
        return;
    }
    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
        let enabled = window
            .try_state::<Arc<runtime::ResidentState>>()
            .is_some_and(|state| state.enabled());
        if cfg!(target_os = "macos") || enabled {
            // A hidden tray cannot provide a reopening affordance on Windows.
            // macOS retains Dock reopening even when monitoring is disabled.
            api.prevent_close();
            match main_window::hide(window.app_handle(), enabled) {
                Ok(()) => log::info!("main_window_hidden reason=close_requested"),
                Err(error) => log::warn!("main_window_hide_failed error={error}"),
            }
        } else {
            // A previously opened, hidden panel is still a native window. Closing
            // the main window must therefore explicitly quit when residency is off.
            window.app_handle().exit(0);
        }
    }
}
