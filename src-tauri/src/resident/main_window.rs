//! Main-window presentation is independent of tray-panel visibility and Core tasks.
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{Emitter, Manager, WebviewWindowBuilder};

use crate::MAIN_WINDOW_LABEL;

pub const BACKGROUND_ARGUMENT: &str = "--background";

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Destination {
    Main,
    Cleanup,
    Applications,
    Settings,
    About,
}

#[derive(Default)]
struct Presentation {
    ready: bool,
    requested: bool,
    destination: Option<Destination>,
}

impl Presentation {
    fn request(&mut self, destination: Destination) {
        self.requested = true;
        self.destination = Some(destination);
    }

    fn hide(&mut self) {
        self.requested = false;
        self.destination = None;
    }

    fn take_destination(&mut self) -> Option<Destination> {
        if self.ready && self.requested {
            self.destination.take()
        } else {
            None
        }
    }
}

#[derive(Default)]
pub struct MainWindowState {
    creation: Mutex<()>,
    presentation: Mutex<Presentation>,
}

pub fn is_background_launch(arguments: impl IntoIterator<Item = impl AsRef<str>>) -> bool {
    arguments
        .into_iter()
        .skip(1)
        .any(|arg| arg.as_ref() == BACKGROUND_ARGUMENT)
}

/// Only a resident login launch has a usable foreground entry without the main UI.
pub fn start_hidden(login_launch: bool, resident_enabled: bool) -> bool {
    login_launch && resident_enabled
}

/// Native callbacks must return before WebView2 construction can process its messages.
pub fn request(app: &tauri::AppHandle, destination: Destination, source: &'static str) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(error) = open(&app, destination, source) {
            log::warn!("main_window_open_failed source={source} error={error}");
        }
    });
}

pub fn open(
    app: &tauri::AppHandle,
    destination: Destination,
    source: &'static str,
) -> tauri::Result<()> {
    super::panel::hide(app);
    let state = app.state::<MainWindowState>();
    let _creation = state
        .creation
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    state
        .presentation
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .request(destination);
    if app.get_webview_window(MAIN_WINDOW_LABEL).is_none() {
        let config = app
            .config()
            .app
            .windows
            .iter()
            .find(|window| window.label == MAIN_WINDOW_LABEL)
            .ok_or_else(|| std::io::Error::other("main window configuration is missing"))?;
        // The configured window is created only for an explicit foreground request.
        // WebKitGTK on Ubuntu/Wayland can leave a lazily shown window without
        // working input after it was created hidden. Linux background launches
        // do not create the main window, so showing this foreground window
        // immediately avoids that platform defect without a login flash.
        let initially_visible = cfg!(target_os = "linux");
        WebviewWindowBuilder::from_config(app, config)?
            .visible(initially_visible)
            .focused(initially_visible)
            .build()?;
        // The state plugin also locks its cache in the main-thread window-ready
        // callback. Restoring from this worker can hold that cache while waiting
        // for a native query, deadlocking the first window after background launch.
        let restore_app = app.clone();
        app.run_on_main_thread(move || crate::restore_main_window_state(&restore_app))?;
        log::info!("main_window_created source={source}");
    }
    let destination = state
        .presentation
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .take_destination();
    if let Some(destination) = destination {
        reveal(app)?;
        if destination == Destination::About {
            crate::events::emit(app, crate::events::OPEN_ABOUT, ());
        } else {
            app.emit_to(MAIN_WINDOW_LABEL, "resident-open-page", destination)?;
        }
    }
    log::info!("main_window_open_requested source={source}");
    Ok(())
}

/// The frontend subscribes before this handshake, so lazy creation cannot lose navigation.
pub fn ready(app: &tauri::AppHandle) -> tauri::Result<Option<Destination>> {
    let state = app.state::<MainWindowState>();
    let _creation = state
        .creation
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let destination = {
        let mut presentation = state
            .presentation
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        presentation.ready = true;
        presentation.take_destination()
    };
    if destination.is_some() {
        reveal(app)?;
    }
    Ok(destination)
}

fn reveal(app: &tauri::AppHandle) -> tauri::Result<()> {
    foreground_policy(app)?;
    if let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) {
        #[cfg(windows)]
        window.set_skip_taskbar(false)?;
        window.unminimize()?;
        window.show()?;
        window.set_focus()?;
    }
    log::info!("main_window_shown");
    Ok(())
}

pub fn foreground_policy(_app: &tauri::AppHandle) -> tauri::Result<()> {
    #[cfg(target_os = "macos")]
    _app.set_activation_policy(tauri::ActivationPolicy::Regular)?;
    Ok(())
}

pub fn background_policy(_app: &tauri::AppHandle) -> tauri::Result<()> {
    #[cfg(target_os = "macos")]
    _app.set_activation_policy(tauri::ActivationPolicy::Accessory)?;
    Ok(())
}

pub fn hide(app: &tauri::AppHandle, resident: bool) -> tauri::Result<()> {
    if let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) {
        window.hide()?;
        #[cfg(windows)]
        window.set_skip_taskbar(true)?;
    }
    app.state::<MainWindowState>()
        .presentation
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .hide();
    if resident {
        background_policy(app)?;
    }
    log::info!("main_window_hidden resident={resident}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_and_residency_combinations_always_leave_an_entry_point() {
        assert!(start_hidden(true, true));
        assert!(!start_hidden(true, false));
        assert!(!start_hidden(false, true));
        assert!(!start_hidden(false, false));
    }

    #[test]
    fn background_arguments_do_not_confuse_manual_launches_or_partial_matches() {
        assert!(is_background_launch(["MangoDisk", "--background"]));
        assert!(!is_background_launch(["MangoDisk"]));
        assert!(!is_background_launch([
            "--background",
            "--background=false"
        ]));
    }

    #[test]
    fn lazy_creation_delivers_the_latest_requested_destination_once() {
        let mut state = Presentation::default();
        state.request(Destination::Main);
        state.request(Destination::Settings);
        assert_eq!(state.take_destination(), None);
        state.ready = true;
        assert_eq!(state.take_destination(), Some(Destination::Settings));
        assert_eq!(state.take_destination(), None);
    }

    #[test]
    fn closing_before_mount_cancels_reveal_but_a_later_request_can_reopen() {
        let mut state = Presentation::default();
        state.request(Destination::Main);
        state.hide();
        state.ready = true;
        assert_eq!(state.take_destination(), None);
        state.request(Destination::Main);
        assert_eq!(state.take_destination(), Some(Destination::Main));
    }
}
