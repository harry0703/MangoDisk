//! A bounded handoff survives cold startup and reuses the ordinary analysis UI.
use serde::Serialize;
#[cfg(any(windows, test))]
use std::path::Path;
use std::sync::Mutex;
#[cfg(windows)]
use tauri::Emitter;
use tauri::Manager;

#[cfg(windows)]
pub const EVENT: &str = "analysis-directory-requested";
#[cfg(any(windows, test))]
const ARGUMENT: &str = "--analyze-directory";

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Request {
    schema_version: u32,
    path: String,
}

#[derive(Default)]
pub struct State(Mutex<Option<Request>>);

#[cfg(any(windows, test))]
pub fn parse(arguments: &[String]) -> Option<String> {
    // The elevated form is consumed only after startup validates the parent/child handshake.
    let path = match arguments {
        [_, argument, path] if argument == ARGUMENT => path,
        [_, restart, _, _, argument, path]
            if restart == "--mangodisk-admin-restart-v1" && argument == ARGUMENT =>
        {
            path
        }
        _ => return None,
    };
    if path.is_empty() || path.chars().any(char::is_control) || !Path::new(path).is_absolute() {
        return None;
    }
    Some(path.clone())
}

impl State {
    #[cfg(any(windows, test))]
    fn store(&self, path: String) {
        *self.0.lock().unwrap_or_else(|error| error.into_inner()) = Some(Request {
            schema_version: 1,
            path,
        });
    }

    fn take(&self) -> Option<Request> {
        self.0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take()
    }
}

#[cfg(windows)]
pub fn queue(app: &tauri::AppHandle, arguments: &[String]) -> bool {
    let Some(path) = parse(arguments) else {
        return false;
    };
    app.state::<State>().store(path);
    true
}

#[cfg(windows)]
pub fn receive(app: &tauri::AppHandle, arguments: &[String]) -> bool {
    if !queue(app, arguments) {
        return false;
    }
    crate::resident::main_window::request(
        app,
        crate::resident::main_window::Destination::Main,
        "explorer_analysis",
    );
    let app = app.clone();
    // A second launch can arrive while the first WebView is still being built.
    // Return from the native callback before touching the window event manager.
    tauri::async_runtime::spawn_blocking(move || {
        if let Err(error) = app.emit_to(crate::MAIN_WINDOW_LABEL, EVENT, ()) {
            // Startup still consumes the request if the listener is not ready.
            log::debug!("analysis_launch_notice_deferred error={error}");
        }
    });
    true
}

pub fn take(app: &tauri::AppHandle) -> Option<Request> {
    app.state::<State>().take()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pending_analysis_survives_startup_and_is_consumed_once() {
        let state = State::default();
        state.store("first".into());
        state.store("latest".into());
        let request = state.take().unwrap();
        assert_eq!(request.schema_version, 1);
        assert_eq!(request.path, "latest");
        assert!(state.take().is_none());
    }

    #[test]
    fn authenticated_administrator_restart_retains_one_analysis_request() {
        let path = std::env::temp_dir()
            .join("analysis & unicode-\u{4e2d}")
            .to_string_lossy()
            .into_owned();
        let args = vec![
            "app".into(),
            "--mangodisk-admin-restart-v1".into(),
            "123".into(),
            "nonce".into(),
            ARGUMENT.into(),
            path.clone(),
        ];
        assert_eq!(parse(&args), Some(path));
        let mut mixed = args.clone();
        mixed.push("--background".into());
        assert!(parse(&mixed).is_none());
    }
    #[test]
    fn analysis_launch_rejects_relative_and_mixed_helper_arguments() {
        for arguments in [
            vec!["app", ARGUMENT, "relative"],
            vec!["app", ARGUMENT],
            vec!["app", ARGUMENT, "/folder", "--background"],
            vec!["app", "--background", "/folder"],
        ] {
            assert!(parse(&arguments.into_iter().map(String::from).collect::<Vec<_>>()).is_none());
        }
    }
    #[test]
    fn analysis_launch_keeps_unicode_spaces_and_shell_characters_as_data() {
        let root = std::env::temp_dir()
            .join("\u{4e2d}\u{6587} & folder #1")
            .to_string_lossy()
            .into_owned();
        assert_eq!(
            parse(&["app".into(), ARGUMENT.into(), root.clone()]),
            Some(root)
        );
    }
}
