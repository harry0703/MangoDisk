use super::{
    placement::Placement, sizing::ItemMeasurement, LayoutMeasurement, Snapshot, EVENT, LABEL,
};
use crate::resident::preferences::ResidentPreferences;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex,
};
use tauri::{Emitter, Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent};

struct State {
    snapshot: Mutex<Option<Snapshot>>,
    creation: Mutex<()>,
    position: Mutex<Placement>,
    ready: AtomicBool,
    queued: AtomicBool,
    failed: AtomicBool,
    #[cfg(windows)]
    click_through: AtomicBool,
    measurements: Mutex<Option<LayoutMeasurement>>,
}

pub fn install(app: &tauri::AppHandle) {
    app.manage(State {
        snapshot: Mutex::new(None),
        creation: Mutex::new(()),
        position: Mutex::new(Placement::load(app)),
        ready: AtomicBool::new(false),
        queued: AtomicBool::new(false),
        failed: AtomicBool::new(false),
        #[cfg(windows)]
        click_through: AtomicBool::new(false),
        measurements: Mutex::new(None),
    });
}

pub fn prepare(app: &tauri::AppHandle, preferences: &ResidentPreferences) -> tauri::Result<()> {
    let state = app.state::<State>();
    let _creation = state.creation.lock().unwrap_or_else(|e| e.into_inner());
    if !requested(preferences) || app.get_webview_window(LABEL).is_some() {
        return Ok(());
    }
    // Document readiness and native placement cannot survive a WebView replacement.
    state.ready.store(false, Ordering::Relaxed);
    *state.measurements.lock().unwrap_or_else(|e| e.into_inner()) = None;
    state
        .position
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .invalidate_window();
    #[cfg(windows)]
    state.click_through.store(false, Ordering::Relaxed);
    let builder =
        WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("floating-monitor.html".into()))
            .title(localized_title(
                &crate::services::native_labels::NativeLabels::load(app).locale,
            ))
            .inner_size(114.0, 26.0)
            .decorations(false)
            .resizable(false)
            .maximizable(false)
            .minimizable(false)
            .visible(false)
            .focused(false)
            .skip_taskbar(true)
            .shadow(false)
            .transparent(true)
            .always_on_top(preferences.floating.always_on_top);
    #[cfg(target_os = "macos")]
    let builder = builder
        .accept_first_mouse(true)
        .visible_on_all_workspaces(true);
    let window = builder.build()?;
    #[cfg(windows)]
    crate::webview_recovery::install(&window);
    super::menu::install(&window);
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    super::native::configure(&window)?;
    window.on_window_event(move |event| {
        if let WindowEvent::CloseRequested { api, .. } = event {
            api.prevent_close();
        }
    });
    log::info!("resident_floating_created outcome=hidden_until_ready");
    Ok(())
}

#[cfg(windows)]
pub(crate) fn recreate_webview(app: &tauri::AppHandle) -> tauri::Result<()> {
    let resident = app.state::<std::sync::Arc<crate::resident::runtime::ResidentState>>();
    let preferences = resident
        .preferences
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    // Current preferences and fullscreen detection own visibility, not the failed handle.
    prepare(app, &preferences)?;
    resident.wake();
    // Recovery runs on a worker. Restore dependent panels only after their anchor
    // has real geometry; fullscreen-hidden monitors still complete placement.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while let Some(window) = app.get_webview_window(LABEL) {
        if app
            .state::<State>()
            .position
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_applied()
        {
            // This UI-thread getter also drains the render that applied placement.
            let _ = window.is_visible()?;
            break;
        }
        if std::time::Instant::now() >= deadline {
            log::warn!(
                "resident_floating_recovery_layout_timeout outcome=restore_remaining_windows"
            );
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    Ok(())
}

pub fn prewarm(app: &tauri::AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let preferences = app
            .state::<std::sync::Arc<crate::resident::runtime::ResidentState>>()
            .preferences
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if let Err(error) = prepare(&app, &preferences) {
            crate::resident::diagnostics::Failure::record("floating_startup", &error);
            log::warn!("resident_floating_startup_failed recovery=tray_icon");
        }
    });
}

pub(super) fn requested(preferences: &ResidentPreferences) -> bool {
    if !preferences.enabled {
        return false;
    }
    #[cfg(windows)]
    {
        preferences.windows_display_mode
            != crate::resident::preference_schema::WindowsDisplayMode::Taskbar
    }
    #[cfg(target_os = "macos")]
    {
        preferences.macos_display_mode
            != crate::resident::preference_schema::MacOsDisplayMode::MenuBar
    }
    #[cfg(target_os = "linux")]
    {
        preferences.linux_display_mode != crate::resident::preference_schema::LinuxDisplayMode::Tray
    }
}

pub fn update(
    app: &tauri::AppHandle,
    preferences: &ResidentPreferences,
    entries: &[super::DisplayEntry],
) {
    let state = app.state::<State>();
    {
        let mut current = state.snapshot.lock().unwrap_or_else(|e| e.into_inner());
        let next = requested(preferences).then(|| {
            let floating = preferences.floating.clone();
            #[cfg(any(target_os = "macos", target_os = "linux"))]
            let floating = super::passive_preferences(floating);
            Snapshot {
                schema_version: 12,
                interactive: cfg!(windows),
                context_menu: true,
                entries: entries.to_vec(),
                preferences: floating,
                columns: None,
            }
        });
        // Default taskbar mode never creates a WebView or queues floating UI work.
        if current.is_none() && next.is_none() && !state.failed.load(Ordering::Relaxed) {
            return;
        }
        *current = next;
    }
    if state.queued.swap(true, Ordering::Relaxed) {
        return;
    }
    let target = app.clone();
    if let Err(error) = app.run_on_main_thread(move || {
        target
            .state::<State>()
            .queued
            .store(false, Ordering::Relaxed);
        let state = target.state::<State>();
        match render(&target) {
            Ok(()) if state.failed.swap(false, Ordering::Relaxed) => {
                log::info!("resident_floating_render_recovered")
            }
            Err(error) if !state.failed.swap(true, Ordering::Relaxed) => {
                crate::resident::diagnostics::Failure::record("floating_render", &error);
            }
            _ => {}
        }
    }) {
        state.queued.store(false, Ordering::Relaxed);
        crate::resident::diagnostics::Failure::record("floating_dispatch", &error);
    }
}

pub fn ready(app: &tauri::AppHandle) -> Option<Snapshot> {
    let state = app.state::<State>();
    log::info!("resident_floating_ready stage=measure_layout");
    let snapshot = state
        .snapshot
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    app.state::<std::sync::Arc<crate::resident::runtime::ResidentState>>()
        .wake();
    snapshot
}

pub fn set_layout(app: &tauri::AppHandle, measurement: LayoutMeasurement) -> bool {
    let state = app.state::<State>();
    let current = state.snapshot.lock().unwrap_or_else(|e| e.into_inner());
    let Some(snapshot) = current.as_ref() else {
        return false;
    };
    if !measurement.valid() || !measurement.matches(&snapshot.preferences, &snapshot.entries) {
        return false;
    }
    let entries = snapshot.entries.len();
    let mut measured = state.measurements.lock().unwrap_or_else(|e| e.into_inner());
    if state.ready.load(Ordering::Relaxed) && *measured == Some(measurement.clone()) {
        return true;
    }
    *measured = Some(measurement);
    drop(measured);
    state.ready.store(true, Ordering::Relaxed);
    drop(current);
    log::info!("resident_floating_layout_ready entries={entries} outcome=accepted");
    app.state::<std::sync::Arc<crate::resident::runtime::ResidentState>>()
        .wake();
    true
}

pub fn update_title(app: &tauri::AppHandle, locale: &str) -> tauri::Result<()> {
    if let Some(window) = app.get_webview_window(LABEL) {
        window.set_title(&localized_title(locale))?;
    }
    Ok(())
}

fn render(app: &tauri::AppHandle) -> tauri::Result<()> {
    let state = app.state::<State>();
    let Some(window) = app.get_webview_window(LABEL) else {
        return Ok(());
    };
    let snapshot = state
        .snapshot
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    let Some(mut snapshot) = snapshot else {
        if window.is_visible()? {
            super::native::set_visible(&window, false)?;
        }
        return Ok(());
    };
    #[cfg(windows)]
    super::interaction::apply_cursor_events(
        &window,
        &state.click_through,
        snapshot.preferences.click_through,
    )?;
    if !state.ready.load(Ordering::Relaxed) {
        app.emit_to(LABEL, EVENT, &snapshot)?;
        return Ok(());
    }
    let measurements = state
        .measurements
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    if measurements
        .as_ref()
        .is_some_and(|measurement| !measurement.matches(&snapshot.preferences, &snapshot.entries))
    {
        app.emit_to(LABEL, EVENT, &snapshot)?;
        return Ok(());
    }
    let widths = measurements
        .and_then(|measurement| measurement.items)
        .map(|items| items.iter().map(ItemMeasurement::width).collect::<Vec<_>>())
        .unwrap_or_else(|| {
            vec![108.0 * snapshot.preferences.font_size.row_height() / 20.0; snapshot.entries.len()]
        });
    window.set_always_on_top(snapshot.preferences.always_on_top)?;
    snapshot.columns = super::placement::apply(&window, &snapshot, &state.position, &widths)?;
    if snapshot.columns.is_none() {
        return Ok(());
    }
    #[cfg(windows)]
    let fullscreen = snapshot.preferences.hide_on_fullscreen && super::native::fullscreen(&window);
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    let fullscreen = super::native::fullscreen(&window);
    let visible = !fullscreen;
    if visible {
        app.emit_to(LABEL, EVENT, &snapshot)?;
    }
    if visible != window.is_visible()? {
        if visible {
            super::native::set_visible(&window, true)?;
        } else {
            super::native::set_visible(&window, false)?;
        }
        log::info!("resident_floating_visibility visible={visible} fullscreen={fullscreen}");
    }
    Ok(())
}

pub fn show(app: &tauri::AppHandle) {
    if app.get_webview_window(LABEL).is_none() {
        let preferences = app
            .state::<std::sync::Arc<crate::resident::runtime::ResidentState>>()
            .preferences
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if let Err(error) = prepare(app, &preferences) {
            crate::resident::diagnostics::Failure::record("floating_reopen", &error);
            return;
        }
    }
    if let Some(window) = app.get_webview_window(LABEL) {
        if let Err(error) = window.unminimize() {
            crate::resident::diagnostics::Failure::record("floating_restore", &error);
        }
    }
    if let Some(state) = app.try_state::<std::sync::Arc<crate::resident::runtime::ResidentState>>()
    {
        state.wake();
    }
}

pub(super) fn finish_drag(app: &tauri::AppHandle) {
    // Native Moved events also follow DPI and work-area changes. Only an
    // explicit drag gesture may replace the saved anchor.
    app.state::<State>()
        .position
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .finish_drag();
}

pub(super) fn reset(app: &tauri::AppHandle) {
    let state = app.state::<State>();
    let mut position = state.position.lock().unwrap_or_else(|e| e.into_inner());
    position.reset();
    drop(position);
    show(app);
}

fn localized_title(locale: &str) -> String {
    let labels = crate::services::native_labels::NativeLabels::for_locale(locale);
    format!("MangoDisk · {}", labels.text("/floatingMonitor/title"))
}

#[cfg(test)]
mod tests {
    use super::localized_title;
    use crate::services::native_labels::NativeLabels;

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_tray_mode_hides_the_surface_and_both_mode_retains_recovery() {
        use crate::resident::preference_schema::LinuxDisplayMode;
        for (mode, visible) in [
            (LinuxDisplayMode::Tray, false),
            (LinuxDisplayMode::Floating, true),
            (LinuxDisplayMode::Both, true),
        ] {
            let mut preferences = super::ResidentPreferences {
                linux_display_mode: mode,
                show_icon: false,
                ..Default::default()
            };
            assert_eq!(super::requested(&preferences), visible);
            assert_eq!(preferences.effective_icon(), visible);
            preferences.enabled = false;
            assert!(!super::requested(&preferences));
        }
    }

    #[test]
    fn localized_titles_differ_even_when_metric_labels_are_identical() {
        let english = NativeLabels::for_locale("en-US");
        let chinese = NativeLabels::for_locale("zh-CN");
        assert_eq!(
            english.text("/floatingMonitor/labels/cpu"),
            chinese.text("/floatingMonitor/labels/cpu")
        );
        assert_eq!(localized_title("en-US"), "MangoDisk · System status");
        let chinese_title = localized_title("zh-CN");
        assert_ne!(chinese_title, localized_title("en-US"));
        assert_ne!(chinese_title, "MangoDisk · —");
    }
}
