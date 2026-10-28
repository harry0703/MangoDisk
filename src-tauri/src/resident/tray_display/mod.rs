mod color_state;
pub mod display;
pub mod display_entry;
mod fan;
mod format;
mod frequency;
mod gpu;
pub mod labels;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
mod macos_interaction;
#[cfg(any(target_os = "macos", test))]
mod macos_presentation;
#[cfg(target_os = "macos")]
pub(super) mod macos_thread;
mod power;
mod temperature;
pub mod usage_color;
#[cfg(windows)]
pub mod windows_bitmap;

use crate::resident::{
    main_window, panel, preferences::ResidentPreferences, runtime::ResidentReading,
};
use display_entry::{DisplayEntry, DisplayId};
use std::{collections::HashMap, sync::Mutex};
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Manager,
};

#[derive(Default)]
pub struct DisplayState {
    entries: HashMap<DisplayId, DisplayEntry>,
    locale: String,
    visible: Vec<DisplayId>,
    summary: String,
    pub renders: u64,
    #[cfg(target_os = "macos")]
    macos: macos::Cache,
    #[cfg(windows)]
    appearance: Option<windows_bitmap::Appearance>,
    #[cfg(target_os = "linux")]
    indicator_title: Option<String>,
    failed: bool,
    #[cfg(windows)]
    menu_preferences: Option<(
        u64,
        bool,
        crate::resident::preference_schema::WindowsDisplayMode,
        bool,
    )>,
    colors: color_state::ColorState,
    floating_colors: color_state::ColorState,
}

pub fn install(app: &tauri::AppHandle) -> tauri::Result<()> {
    app.manage(Mutex::new(DisplayState::default()));
    #[cfg(windows)]
    app.on_menu_event(|app, event| {
        if event.id().as_ref() != "resident-click-through" {
            return;
        }
        // Tray builders receive global menu events. Register a toggle only once,
        // otherwise retained metric icons could toggle the setting repeatedly.
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            app.state::<Mutex<DisplayState>>()
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .menu_preferences = None;
            if let Err(error) = super::floating_monitor::toggle_click_through(&app) {
                super::diagnostics::Failure::record("floating_click_through_toggle", &error);
                main_window::request(
                    &app,
                    main_window::Destination::DisplaySettings,
                    "tray_toggle_failed",
                );
            }
        });
    });
    #[cfg(target_os = "macos")]
    {
        macos_thread::call(app, |app, _mtm| {
            ensure(app, DisplayId::App, &labels::Labels::load(app))
        })
    }
    #[cfg(not(target_os = "macos"))]
    ensure(app, DisplayId::App, &labels::Labels::load(app))
}

pub fn log_summary(app: &tauri::AppHandle, loop_max_ms: u128) {
    #[cfg(target_os = "macos")]
    {
        let target = app.clone();
        // Diagnostics must not make the sampling loop wait for native menu
        // tracking. Only the queued callback acquires native tray handles.
        if let Err(error) = app.run_on_main_thread(move || summary(&target, loop_max_ms)) {
            super::diagnostics::Failure::record("display_summary_dispatch", &error);
        }
    }
    #[cfg(not(target_os = "macos"))]
    summary(app, loop_max_ms);
}

fn summary(app: &tauri::AppHandle, loop_max_ms: u128) {
    #[cfg(target_os = "macos")]
    let _mtm = objc2::MainThreadMarker::new().expect("tray diagnostics run on the main thread");
    let renders = app
        .state::<Mutex<DisplayState>>()
        .try_lock()
        .ok()
        .map(|state| state.renders);
    let handles = DisplayId::ALL
        .iter()
        .filter(|id| app.tray_by_id(id.tray_id()).is_some())
        .count();
    log::info!("resident_runtime_summary tray_renders={renders:?} tray_handles={handles} loop_max_ms={loop_max_ms}");
}

fn menu(
    app: &tauri::AppHandle,
    labels: &labels::Labels,
    _preferences: &ResidentPreferences,
) -> tauri::Result<Menu<tauri::Wry>> {
    // AppIndicator does not emit tray clicks on Linux. Keep the detail panel
    // reachable through the native menu that the desktop opens instead.
    #[cfg(target_os = "linux")]
    let status = MenuItem::with_id(
        app,
        "resident-status",
        labels.text("details"),
        true,
        None::<&str>,
    )?;
    let open = MenuItem::with_id(
        app,
        "resident-open",
        labels.text("openMain"),
        true,
        None::<&str>,
    )?;
    let settings = MenuItem::with_id(
        app,
        "resident-settings",
        labels.text("settings"),
        true,
        None::<&str>,
    )?;
    let quit = MenuItem::with_id(
        app,
        "resident-quit",
        labels.text("quit"),
        true,
        None::<&str>,
    )?;
    let menu = Menu::new(app)?;
    #[cfg(target_os = "linux")]
    menu.append(&status)?;
    menu.append_items(&[&open, &settings])?;
    #[cfg(windows)]
    {
        let labels = crate::services::native_labels::NativeLabels::for_locale(&labels.locale);
        menu.append(&tauri::menu::CheckMenuItem::with_id(
            app,
            "resident-click-through",
            labels.text("/floatingMonitor/clickThrough"),
            _preferences.enabled
                && _preferences.windows_display_mode
                    != crate::resident::preference_schema::WindowsDisplayMode::Taskbar,
            _preferences.floating.click_through,
            None::<&str>,
        )?)?;
    }
    menu.append(&quit)?;
    Ok(menu)
}

pub fn brand_icon() -> tauri::image::Image<'static> {
    #[cfg(target_os = "macos")]
    {
        tauri::include_image!("icons/tray-template.png")
    }
    #[cfg(not(target_os = "macos"))]
    {
        tauri::include_image!("icons/tray-color.png")
    }
}

fn ensure(app: &tauri::AppHandle, id: DisplayId, labels: &labels::Labels) -> tauri::Result<()> {
    #[cfg(target_os = "macos")]
    let _mtm = objc2::MainThreadMarker::new().expect("tray handles stay on the main thread");
    if app.tray_by_id(id.tray_id()).is_some() {
        return Ok(());
    }
    #[cfg(windows)]
    let icon = if let Some(metric) = id.metric() {
        let marker = match metric {
            mangodisk_core::system_resources::metrics::MetricId::Cpu => "C",
            mangodisk_core::system_resources::metrics::MetricId::Gpu => "G",
            mangodisk_core::system_resources::metrics::MetricId::Memory => "M",
            mangodisk_core::system_resources::metrics::MetricId::Disk => "D",
            _ => {
                if id == DisplayId::Upload {
                    "↑"
                } else {
                    "↓"
                }
            }
        };
        let appearance = windows_bitmap::appearance();
        let rgba = windows_bitmap::render(marker, "—", appearance)
            .map_err(|stage| tauri::Error::Io(std::io::Error::other(stage)))?;
        tauri::image::Image::new_owned(rgba, appearance.size, appearance.size)
    } else {
        brand_icon()
    };
    #[cfg(not(windows))]
    let icon = brand_icon();
    let builder = TrayIconBuilder::with_id(id.tray_id())
        .icon(icon)
        .icon_as_template(cfg!(target_os = "macos"))
        .tooltip("MangoDisk")
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            #[cfg(target_os = "linux")]
            "resident-status" => {
                let app = app.clone();
                // Menu activation is also a native callback; leave it before
                // creating or focusing the panel's WebView.
                tauri::async_runtime::spawn(async move {
                    if let Err(error) = panel::open(&app) {
                        super::diagnostics::Failure::record("panel_menu_entry", &error);
                    }
                });
            }
            "resident-open" => {
                main_window::request(app, main_window::Destination::Main, "tray_menu")
            }
            "resident-settings" => {
                main_window::request(app, main_window::Destination::Settings, "tray_menu")
            }
            "resident-quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(move |tray, event| {
            #[cfg(target_os = "macos")]
            if let TrayIconEvent::Click {
                button,
                button_state,
                ..
            } = event
            {
                macos_interaction::click(tray, button, button_state);
                return;
            }
            #[cfg(windows)]
            if matches!(event, TrayIconEvent::Leave { .. }) {
                panel::tray_pointer_left(tray.app_handle());
            }
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                }
            ) {
                let app = tray.app_handle().clone();
                // Never create WebView2 in the native event callback.
                tauri::async_runtime::spawn(async move {
                    if let Err(error) = panel::toggle_from(&app, id.tray_id(), id.page()) {
                        super::diagnostics::Failure::record("panel_entry", &error);
                    }
                });
            }
        });
    // tray-icon 0.25 retains the menu and attaches it only during native
    // tracking, preserving left-click delivery on macOS 27 (#365).
    let builder = builder.menu(&menu(app, labels, &ResidentPreferences::default())?);
    let tray = builder.build(app)?;
    tray.set_visible(false)?;
    Ok(())
}

/// Apply a complete desired set before persistence. On rejection, the caller
/// reapplies the last committed preferences so settings and native entries agree.
pub fn apply_preferences(
    app: &tauri::AppHandle,
    preferences: &ResidentPreferences,
    reading: &ResidentReading,
) -> tauri::Result<()> {
    let request = DisplayRequest::new(app, preferences, reading);
    #[cfg(target_os = "macos")]
    return macos_thread::call(app, move |app, _mtm| apply_request(app, request));
    #[cfg(not(target_os = "macos"))]
    apply_request(app, request)
}

fn apply_request(app: &tauri::AppHandle, request: DisplayRequest) -> tauri::Result<()> {
    let state = app.state::<Mutex<DisplayState>>();
    let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
    render(app, request, &mut state)
}

pub fn refresh(
    app: &tauri::AppHandle,
    preferences: &ResidentPreferences,
    reading: &ResidentReading,
) {
    let request = DisplayRequest::new(app, preferences, reading);
    #[cfg(target_os = "macos")]
    let result = macos_thread::call(app, move |app, _mtm| refresh_request(app, request));
    #[cfg(not(target_os = "macos"))]
    let result = refresh_request(app, request);
    if let Err(error) = result {
        super::diagnostics::Failure::record("display_dispatch", &error);
    }
}

fn refresh_request(app: &tauri::AppHandle, request: DisplayRequest) -> tauri::Result<()> {
    let state = app.state::<Mutex<DisplayState>>();
    let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
    match render(app, request, &mut state) {
        Ok(()) => {
            if state.failed {
                log::info!("resident_display_recovered");
            }
            state.failed = false;
        }
        Err(error) => {
            if !state.failed {
                super::diagnostics::Failure::record("display_refresh", &error);
            }
            state.failed = true;
        }
    }
    Ok(())
}

// Prepare only the data native rendering needs. Detailed process lists and
// resource histories stay on the presentation worker rather than being cloned
// again for main-thread dispatch.
struct DisplayRequest {
    preferences: ResidentPreferences,
    labels: labels::Labels,
    entries: Vec<DisplayEntry>,
    floating_power_digits: std::collections::HashMap<DisplayId, String>,
    disk_volume: Option<String>,
    disk_temperature_volume: Option<String>,
    gpu_adapter: Option<String>,
    gpu_temperature_adapter: Option<String>,
}

impl DisplayRequest {
    fn new(
        app: &tauri::AppHandle,
        preferences: &ResidentPreferences,
        reading: &ResidentReading,
    ) -> Self {
        let labels = labels::Labels::load(app);
        let entries = display::entries(
            preferences,
            &reading.resources,
            &labels,
            if cfg!(windows) { 1024.0 } else { 1000.0 },
        );
        Self {
            preferences: preferences.clone(),
            floating_power_digits: power::floating_power_digits(&entries, &reading.resources),
            labels,
            entries,
            gpu_adapter: reading
                .resources
                .gpu
                .value
                .as_ref()
                .map(|gpu| gpu.adapter_id.clone()),
            gpu_temperature_adapter: reading
                .resources
                .gpu_temperature
                .value
                .as_ref()
                .map(|gpu| gpu.adapter_id.clone()),
            disk_temperature_volume: reading
                .resources
                .disk_temperature
                .value
                .as_ref()
                .map(|disk| disk.volume_id.clone()),
            disk_volume: reading
                .resources
                .disk
                .value
                .as_ref()
                .map(|disk| disk.volume.id.clone()),
        }
    }
}

fn render(
    app: &tauri::AppHandle,
    request: DisplayRequest,
    state: &mut DisplayState,
) -> tauri::Result<()> {
    #[cfg(target_os = "macos")]
    let _mtm = objc2::MainThreadMarker::new().expect("tray rendering runs on the main thread");
    let DisplayRequest {
        preferences,
        labels,
        mut entries,
        floating_power_digits,
        disk_volume,
        disk_temperature_volume,
        gpu_adapter,
        gpu_temperature_adapter,
    } = request;
    let preferences = &preferences;
    let changed_locale = state.locale != labels.locale;
    state.colors.apply(
        &mut entries,
        preferences,
        disk_volume.as_deref(),
        disk_temperature_volume.as_deref(),
        gpu_adapter.as_deref(),
        gpu_temperature_adapter.as_deref(),
    );
    #[cfg(not(windows))]
    let all_desired = display::desired(preferences);
    #[cfg(windows)]
    let desired = display::windows_desired(
        preferences,
        super::taskbar_display::update(app, preferences, &entries, &labels.locale),
    );
    #[cfg(not(windows))]
    let desired = if !all_desired.is_empty() {
        vec![DisplayId::App]
    } else {
        Vec::new()
    };
    for id in &desired {
        ensure(app, *id, &labels)?;
    }
    #[cfg(windows)]
    let menu_preferences = (
        preferences.revision,
        preferences.enabled,
        preferences.windows_display_mode,
        preferences.floating.click_through,
    );
    #[cfg(windows)]
    let changed_menu = state.menu_preferences != Some(menu_preferences);
    #[cfg(not(windows))]
    let changed_menu = false;
    for id in DisplayId::ALL {
        if let Some(tray) = app.tray_by_id(id.tray_id()) {
            // Handles are retained and reused, with at most six for the process.
            // Hiding removes the entry from the system notification area.
            if state.visible.contains(&id) != desired.contains(&id) {
                tray.set_visible(desired.contains(&id))?;
                #[cfg(target_os = "macos")]
                {
                    state.macos = macos::Cache::default();
                }
                state.visible.retain(|visible| *visible != id);
                if desired.contains(&id) {
                    state.visible.push(id);
                    state.entries.remove(&id);
                }
            }
            if changed_locale || changed_menu {
                tray.set_menu(Some(menu(app, &labels, preferences)?))?;
            }
        }
    }
    state.visible = desired;
    let mut floating_entries = power::floating_entries(&entries, &floating_power_digits);
    state.floating_colors.apply(
        &mut floating_entries,
        preferences,
        disk_volume.as_deref(),
        disk_temperature_volume.as_deref(),
        gpu_adapter.as_deref(),
        gpu_temperature_adapter.as_deref(),
    );
    #[cfg(windows)]
    {
        state.menu_preferences = Some(menu_preferences);
        // Queue inert window updates only after the tray recovery surface is ready.
        super::floating_monitor::update(app, preferences, &floating_entries);
    }
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    super::floating_monitor::update(app, preferences, &floating_entries);
    #[cfg(any(windows, target_os = "macos", target_os = "linux"))]
    if changed_locale {
        // A translated title must update even when metric labels keep the same width.
        super::floating_monitor::update_title(app, &labels.locale)?;
    }
    if !preferences.enabled {
        state.locale = labels.locale;
        return Ok(());
    }
    let summary = entries
        .iter()
        .map(|entry| entry.tooltip.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    // Windows removes hidden icons from the notification area, so NIM_MODIFY
    // fails for the retained brand handle. Keep its tooltip cache unchanged
    // while hidden and apply the latest summary when that entry is shown again.
    if state.visible.contains(&DisplayId::App) && state.summary != summary {
        if let Some(tray) = app.tray_by_id(DisplayId::App.tray_id()) {
            tray.set_tooltip(Some(format!("MangoDisk\n{summary}")))?;
        }
        state.summary = summary.clone();
    }
    #[cfg(target_os = "linux")]
    if state.visible.contains(&DisplayId::App) {
        let title = display::indicator_title(&entries, preferences);
        if state.indicator_title != title {
            if let Some(tray) = app.tray_by_id(DisplayId::App.tray_id()) {
                tray.set_title(title.as_deref())?;
                state.renders += 1;
            }
            state.indicator_title = title;
        }
    }
    #[cfg(target_os = "macos")]
    if let Some(tray) = app.tray_by_id(DisplayId::App.tray_id()) {
        if macos::apply(
            &tray,
            if preferences.macos_display_mode
                == crate::resident::preference_schema::MacOsDisplayMode::Floating
            {
                &[]
            } else {
                &entries
            },
            preferences.effective_icon() || entries.is_empty(),
            preferences.menu_bar_compact,
            &summary,
            &mut state.macos,
        )? {
            state.renders += 1;
        }
    }
    #[cfg(windows)]
    {
        let appearance = windows_bitmap::appearance();
        // Notification icons have one retained handle per metric. The taskbar
        // renders every device; its fallback retains the first device and lists
        // all devices in the tooltip without repeatedly overwriting one handle.
        entries = display::notification_entries(&entries);
        for entry in &entries {
            if !state.visible.contains(&entry.id) {
                continue;
            }
            if let Some(tray) = app.tray_by_id(entry.id.tray_id()) {
                let previous = state.entries.get(&entry.id);
                if state.appearance != Some(appearance)
                    || !previous.is_some_and(|old| {
                        old.marker == entry.marker
                            && old.digits == entry.digits
                            && old.tone == entry.tone
                    })
                {
                    let rgba = windows_bitmap::render_colored(
                        &entry.marker,
                        &entry.digits,
                        appearance,
                        entry.tone,
                    )
                    .map_err(|stage| tauri::Error::Io(std::io::Error::other(stage)))?;
                    tray.set_icon(Some(tauri::image::Image::new_owned(
                        rgba,
                        appearance.size,
                        appearance.size,
                    )))?;
                    state.renders += 1;
                }
                if previous.is_none_or(|old| old.tooltip != entry.tooltip) {
                    tray.set_tooltip(Some(&entry.tooltip))?;
                }
            }
        }
        state.appearance = Some(appearance);
    }
    state.entries = entries.into_iter().map(|entry| (entry.id, entry)).collect();
    state.locale = labels.locale;
    Ok(())
}

#[cfg(test)]
#[path = "color_state_tests.rs"]
mod tests;
