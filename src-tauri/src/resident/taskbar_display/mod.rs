//! Windows taskbar presentation owns only native UI. Resource collection and
//! preferences remain shared with the tray adapter. A companion owns reversible
//! shell layout leases independently of the monitor window.
#[cfg(any(windows, test))]
mod alpha;
#[cfg(windows)]
mod directwrite;
#[cfg(windows)]
mod draw;
#[cfg(windows)]
mod geometry;
#[cfg(windows)]
mod hosting;
#[cfg(any(windows, test))]
mod layout;
#[cfg(windows)]
mod native;
#[cfg(windows)]
mod peers;
#[cfg(any(windows, test))]
mod position;
#[cfg(any(windows, test))]
mod presentation;
#[cfg(windows)]
mod reservation;
#[cfg(any(windows, test))]
mod reservation_layout;
#[cfg(windows)]
mod reservation_windows;
#[cfg(windows)]
mod reservation_xaml;
#[cfg(windows)]
pub use reservation::run_helper_mode as run_layout_helper_mode;
#[cfg(windows)]
mod shell_events;
#[cfg(any(windows, test))]
mod surface;
#[cfg(any(windows, test))]
mod text_layout;
#[cfg(windows)]
mod transparent;
#[cfg(any(windows, test))]
mod visibility;

#[cfg(windows)]
mod identify;
pub mod monitors;
#[cfg(windows)]
mod monitors_windows;

use serde::Serialize;
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DisplayStatus {
    #[default]
    Tray,
    #[cfg(windows)]
    Taskbar,
    #[cfg(windows)]
    NoSpace,
    #[cfg(windows)]
    UnsupportedLayout,
    #[cfg(windows)]
    ShellUnavailable,
}

#[cfg(windows)]
use super::tray_display::display_entry::DisplayEntry;
#[cfg(windows)]
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc, Mutex,
};

#[cfg(windows)]
#[derive(Clone, Default)]
struct Model {
    entries: Vec<DisplayEntry>,
    locale: String,
    position: super::preference_schema::TaskbarPosition,
    monitor: Option<String>,
    background: bool,
    compact: bool,
}

#[cfg(windows)]
struct Service {
    app: tauri::AppHandle,
    requested: AtomicBool,
    window: AtomicUsize,
    model: Mutex<Model>,
    geometry: Mutex<Option<geometry::Geometry>>,
    bounds: Mutex<
        Vec<(
            super::tray_display::display_entry::DisplayId,
            layout::Bounds,
        )>,
    >,
    status: Mutex<DisplayStatus>,
}

#[cfg(windows)]
pub fn install(app: &tauri::AppHandle) {
    use tauri::Manager;
    let service = Arc::new(Service {
        app: app.clone(),
        requested: AtomicBool::new(false),
        window: AtomicUsize::new(0),
        model: Mutex::new(Model::default()),
        geometry: Mutex::new(None),
        bounds: Mutex::new(Vec::new()),
        status: Mutex::new(DisplayStatus::Tray),
    });
    app.manage(service.clone());
    geometry::start(service.clone());
    native::start(service);
}

pub fn status(app: &tauri::AppHandle) -> DisplayStatus {
    #[cfg(windows)]
    {
        use tauri::Manager;
        *app.state::<Arc<Service>>()
            .status
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }
    #[cfg(not(windows))]
    {
        let _ = app;
        DisplayStatus::Tray
    }
}

#[cfg(windows)]
pub fn update(
    app: &tauri::AppHandle,
    preferences: &super::preferences::ResidentPreferences,
    entries: &[DisplayEntry],
    locale: &str,
) -> bool {
    use tauri::Manager;
    let service = app.state::<Arc<Service>>();
    let requested = preferences.enabled
        && preferences.windows_display_mode
            != super::preference_schema::WindowsDisplayMode::Floating
        && !entries.is_empty();
    *service.model.lock().unwrap_or_else(|e| e.into_inner()) = Model {
        entries: entries.to_vec(),
        locale: locale.into(),
        position: preferences.taskbar_position,
        monitor: preferences.taskbar_monitor.clone(),
        background: preferences.taskbar_background,
        compact: preferences.taskbar_compact,
    };
    service.requested.store(requested, Ordering::Relaxed);
    let active = requested && status(app) == DisplayStatus::Taskbar;
    unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW(
            service.window.load(Ordering::Relaxed) as _,
            native::UPDATE,
            0,
            0,
        );
    }
    active
}

#[cfg(windows)]
impl Service {
    fn publish(&self, status: DisplayStatus) {
        use tauri::{Emitter, Manager};
        let mut previous = self.status.lock().unwrap_or_else(|e| e.into_inner());
        if *previous == status {
            return;
        }
        log::info!(
            "resident_taskbar_state previous={:?} current={status:?}",
            *previous
        );
        *previous = status;
        drop(previous);
        let _ = self.app.emit("resident-display-status", status);
        if let Some(state) = self.app.try_state::<Arc<super::runtime::ResidentState>>() {
            state.wake();
        }
    }
}

#[cfg(windows)]
pub fn contains_point(app: &tauri::AppHandle, x: f64, y: f64) -> bool {
    use tauri::Manager;
    app.state::<Arc<Service>>()
        .bounds
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .any(|(_, bounds)| bounds.contains(x as i32, y as i32))
}

#[cfg(windows)]
pub fn anchor(app: &tauri::AppHandle, source: Option<&str>) -> Option<(f64, f64)> {
    use tauri::Manager;
    let service = app.state::<Arc<Service>>();
    let bounds = service.bounds.lock().unwrap_or_else(|e| e.into_inner());
    anchor_in_bounds(&bounds, source)
}

#[cfg(any(windows, test))]
fn anchor_in_bounds(
    bounds: &[(
        super::tray_display::display_entry::DisplayId,
        layout::Bounds,
    )],
    source: Option<&str>,
) -> Option<(f64, f64)> {
    bounds
        .iter()
        .filter(|(_, r)| r.right > r.left && r.bottom > r.top)
        .find(|(id, _)| source.is_none_or(|source| source == format!("taskbar-{}", id.tray_id())))
        .map(|(_, r)| {
            (
                (r.left + r.right) as f64 / 2.0,
                (r.top + r.bottom) as f64 / 2.0,
            )
        })
}

/// Read-only display discovery is separate from shell accessibility inspection.
pub fn monitors() -> Result<monitors::Catalogue, String> {
    #[cfg(windows)]
    let monitors = monitors_windows::enumerate()
        .map_err(|code| format!("taskbar_monitors stage=enumerate code={code}"))?
        .into_iter()
        .map(|screen| screen.info)
        .collect();
    #[cfg(not(windows))]
    let monitors = Vec::new();
    Ok(monitors::Catalogue {
        schema_version: 1,
        monitors,
    })
}

pub fn identify_monitors() -> Result<(), String> {
    #[cfg(windows)]
    return identify::show();
    #[cfg(not(windows))]
    Ok(())
}

#[cfg(test)]
mod anchor_tests {
    use super::{anchor_in_bounds, layout::Bounds};
    use crate::resident::tray_display::display_entry::DisplayId;

    #[test]
    fn removed_or_hidden_entries_can_fall_back_to_an_existing_taskbar_item() {
        let bounds = [
            (DisplayId::App, Bounds::default()),
            (
                DisplayId::Memory,
                Bounds {
                    left: 1500,
                    top: 1040,
                    right: 1600,
                    bottom: 1080,
                },
            ),
        ];
        assert_eq!(
            anchor_in_bounds(&bounds, Some("taskbar-resident-memory")),
            Some((1550.0, 1060.0))
        );
        assert_eq!(
            anchor_in_bounds(&bounds, Some("taskbar-resident-gpu")),
            None
        );
        assert_eq!(anchor_in_bounds(&bounds, None), Some((1550.0, 1060.0)));
        assert_eq!(anchor_in_bounds(&[], None), None);
        assert_eq!(anchor_in_bounds(&bounds[..1], None), None);
    }
}
