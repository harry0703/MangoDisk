pub mod format;
pub mod labels;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(any(target_os = "macos", test))]
mod macos_presentation;
pub mod usage_color;
#[cfg(windows)]
pub mod windows_bitmap;

use crate::resident::{
    main_window, panel, preferences::ResidentPreferences, runtime::ResidentReading,
};
use format::{DisplayEntry, DisplayId};
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
    failed: bool,
    color_rules: Option<(bool, u8, u8)>,
    color_disk_volume: Option<String>,
    color_tones: HashMap<DisplayId, usage_color::UsageTone>,
}

impl DisplayState {
    // Hysteresis belongs to the sampled volume, not the display slot. A newly
    // selected disk can arrive without an intervening unavailable frame.
    fn apply_colors(
        &mut self,
        entries: &mut [DisplayEntry],
        preferences: &ResidentPreferences,
        disk_volume: Option<&str>,
    ) {
        let color_rules = (
            preferences.usage_colors && preferences.enabled,
            preferences.usage_warning_percent,
            preferences.usage_critical_percent,
        );
        for entry in entries.iter_mut() {
            let same_source =
                entry.id != DisplayId::Disk || self.color_disk_volume.as_deref() == disk_volume;
            let previous = if self.color_rules == Some(color_rules) && same_source {
                self.color_tones.get(&entry.id).copied().unwrap_or_default()
            } else {
                usage_color::UsageTone::Normal
            };
            entry.tone = if color_rules.0 {
                previous.next(entry.usage_percent, color_rules.1, color_rules.2)
            } else {
                usage_color::UsageTone::Normal
            };
        }
        // Keep classification separate from the successful-paint cache: native
        // presentation failures must not restore tones from an older rule/source.
        self.color_tones = entries.iter().map(|entry| (entry.id, entry.tone)).collect();
        self.color_rules = Some(color_rules);
        self.color_disk_volume = disk_volume.map(str::to_owned);
    }
}

pub fn install(app: &tauri::AppHandle) -> tauri::Result<()> {
    app.manage(Mutex::new(DisplayState::default()));
    ensure(app, DisplayId::App, &labels::Labels::load(app))
}

fn menu(app: &tauri::AppHandle, labels: &labels::Labels) -> tauri::Result<Menu<tauri::Wry>> {
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
    Menu::with_items(app, &[&open, &settings, &quit])
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
    if app.tray_by_id(id.tray_id()).is_some() {
        return Ok(());
    }
    #[cfg(windows)]
    let icon = if let Some(metric) = id.metric() {
        let marker = match metric {
            mangodisk_core::system_resources::metrics::MetricId::Cpu => "C",
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
    let tray = TrayIconBuilder::with_id(id.tray_id())
        .icon(icon)
        .icon_as_template(cfg!(target_os = "macos"))
        .tooltip("MangoDisk")
        .menu(&menu(app, labels)?)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
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
                    if let Err(error) = panel::toggle_from(&app, id.tray_id(), id.metric()) {
                        super::diagnostics::Failure::record("panel_entry", &error);
                    }
                });
            }
        })
        .build(app)?;
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
    let state = app.state::<Mutex<DisplayState>>();
    let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
    render(app, preferences, reading, &mut state)
}

pub fn refresh(
    app: &tauri::AppHandle,
    preferences: &ResidentPreferences,
    reading: &ResidentReading,
) {
    let state = app.state::<Mutex<DisplayState>>();
    let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
    match render(app, preferences, reading, &mut state) {
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
}

fn render(
    app: &tauri::AppHandle,
    preferences: &ResidentPreferences,
    reading: &ResidentReading,
    state: &mut DisplayState,
) -> tauri::Result<()> {
    let labels = labels::Labels::load(app);
    let changed_locale = state.locale != labels.locale;
    let mut entries = format::entries(
        preferences,
        &reading.resources,
        &labels,
        if cfg!(target_os = "macos") {
            1000.0
        } else {
            1024.0
        },
    );
    state.apply_colors(
        &mut entries,
        preferences,
        reading
            .resources
            .disk
            .value
            .as_ref()
            .map(|disk| disk.volume.id.as_str()),
    );
    #[cfg(not(windows))]
    let all_desired = format::desired(preferences);
    #[cfg(windows)]
    let desired = format::windows_desired(
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
            if changed_locale {
                tray.set_menu(Some(menu(app, &labels)?))?;
            }
        }
    }
    state.visible = desired;
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
    #[cfg(target_os = "macos")]
    if let Some(tray) = app.tray_by_id(DisplayId::App.tray_id()) {
        if macos::apply(
            &tray,
            &entries,
            preferences.effective_icon(),
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
mod tests {
    use super::*;
    use usage_color::UsageTone;

    #[test]
    fn switching_disks_resets_hysteresis_without_resetting_cpu() {
        let mut state = DisplayState::default();
        let preferences = ResidentPreferences::default();
        let mut entries: Vec<_> = [DisplayId::Cpu, DisplayId::Disk]
            .into_iter()
            .map(|id| DisplayEntry {
                id,
                tone: UsageTone::Normal,
                usage_percent: Some(95),
                marker: String::new(),
                digits: String::new(),
                text: String::new(),
                tooltip: String::new(),
            })
            .collect();
        state.apply_colors(&mut entries, &preferences, Some("disk-a"));
        assert!(entries
            .iter()
            .all(|entry| entry.tone == UsageTone::Critical));
        state.entries = entries
            .iter()
            .cloned()
            .map(|entry| (entry.id, entry))
            .collect();
        for entry in &mut entries {
            entry.usage_percent = Some(89);
        }
        state.apply_colors(&mut entries, &preferences, Some("disk-a"));
        assert!(entries
            .iter()
            .all(|entry| entry.tone == UsageTone::Critical));
        state.apply_colors(&mut entries, &preferences, Some("disk-b"));
        assert_eq!(entries[0].tone, UsageTone::Critical);
        assert_eq!(entries[1].tone, UsageTone::Warning);
        // Simulate a failed native frame by leaving the paint cache unchanged.
        state.apply_colors(&mut entries, &preferences, Some("disk-b"));
        assert_eq!(entries[1].tone, UsageTone::Warning);
        let higher_threshold = ResidentPreferences {
            usage_critical_percent: 91,
            ..preferences
        };
        state.apply_colors(&mut entries, &higher_threshold, Some("disk-b"));
        state.apply_colors(&mut entries, &higher_threshold, Some("disk-b"));
        assert!(entries.iter().all(|entry| entry.tone == UsageTone::Warning));
    }
}
