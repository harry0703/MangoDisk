//! Resolve foreground ownership once per window/process change, never per paint.
use super::super::{layout::Bounds, visibility};
use mangodisk_platform::diagnostics::text;
use windows_sys::Win32::{
    Foundation::*,
    System::{SystemInformation::GetWindowsDirectoryW, Threading::*},
    UI::WindowsAndMessaging::*,
};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) enum ShellSurface {
    #[default]
    Application,
    Desktop,
    TaskbarPreview,
    StartOrSearch,
}

#[derive(Debug)]
pub(super) struct QueryFailure {
    stage: &'static str,
    code: u32,
}

#[derive(Default)]
pub(super) struct Identity {
    pub hwnd: usize,
    pub process_id: u32,
    pub shell_process_id: u32,
    pub class: String,
    pub image: String,
    pub surface: ShellSurface,
    pub failure: Option<QueryFailure>,
}

impl Identity {
    pub unsafe fn matches(&self, hwnd: HWND, shell: HWND) -> bool {
        self.hwnd == hwnd as usize
            && self.process_id == process_id(hwnd)
            && self.shell_process_id == process_id(shell)
    }

    pub unsafe fn read(hwnd: HWND, shell: HWND) -> Self {
        let mut identity = Self {
            hwnd: hwnd as usize,
            process_id: process_id(hwnd),
            shell_process_id: process_id(shell),
            ..Default::default()
        };
        if hwnd.is_null() {
            return identity;
        }
        let mut class = [0u16; 256];
        let length = GetClassNameW(hwnd, class.as_mut_ptr(), class.len() as i32);
        if length == 0 {
            identity.failure = Some(QueryFailure {
                stage: "window_class",
                code: GetLastError(),
            });
        } else {
            identity.class = String::from_utf16_lossy(&class[..length as usize]);
        }
        match process_image(identity.process_id) {
            Ok(image) => identity.image = image,
            Err(error) => identity.failure = Some(error),
        }
        identity.surface = if visibility::is_shell_desktop(
            &identity.class,
            identity.process_id,
            identity.shell_process_id,
        ) {
            ShellSurface::Desktop
        } else if visibility::is_shell_preview(
            &identity.class,
            identity.process_id,
            identity.shell_process_id,
        ) {
            ShellSurface::TaskbarPreview
        } else if identity.class == "Windows.UI.Core.CoreWindow" {
            match is_system_app(&identity.image) {
                Ok(system_app)
                    if visibility::is_shell_flyout(
                        &identity.class,
                        identity.image.rsplit('\\').next().unwrap_or_default(),
                        system_app,
                    ) =>
                {
                    ShellSurface::StartOrSearch
                }
                Err(error) => {
                    identity.failure = Some(error);
                    ShellSurface::Application
                }
                _ => ShellSurface::Application,
            }
        } else {
            ShellSurface::Application
        };
        identity
    }

    pub fn diagnostic(&self) -> String {
        let (stage, code) = self
            .failure
            .as_ref()
            .map_or(("none", 0), |failure| (failure.stage, failure.code));
        format!(
            "foreground=0x{:x} pid={} shell_pid={} class={} image={} shell_surface={:?} query_stage={stage} query_code={code}",
            self.hwnd, self.process_id, self.shell_process_id,
            text(&self.class), text(&self.image), self.surface,
        )
    }
}

unsafe fn process_id(hwnd: HWND) -> u32 {
    let mut process_id = 0;
    if !hwnd.is_null() {
        GetWindowThreadProcessId(hwnd, &mut process_id);
    }
    process_id
}

unsafe fn process_image(process_id: u32) -> Result<String, QueryFailure> {
    let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, process_id);
    if process.is_null() {
        return Err(QueryFailure {
            stage: "open_process",
            code: GetLastError(),
        });
    }
    let mut image = vec![0u16; 32768];
    let mut length = image.len() as u32;
    let queried = QueryFullProcessImageNameW(process, 0, image.as_mut_ptr(), &mut length);
    let code = GetLastError();
    CloseHandle(process);
    if queried == 0 {
        return Err(QueryFailure {
            stage: "process_image",
            code,
        });
    }
    Ok(String::from_utf16_lossy(&image[..length as usize]))
}

unsafe fn is_system_app(image: &str) -> Result<bool, QueryFailure> {
    let mut windows = vec![0u16; 32768];
    let count = GetWindowsDirectoryW(windows.as_mut_ptr(), windows.len() as u32) as usize;
    if count == 0 || count >= windows.len() {
        return Err(QueryFailure {
            stage: "windows_directory",
            code: GetLastError(),
        });
    }
    let system_apps = format!(
        "{}\\SystemApps\\",
        String::from_utf16_lossy(&windows[..count])
    );
    Ok(image
        .to_ascii_lowercase()
        .starts_with(&system_apps.to_ascii_lowercase()))
}

/// Deduplicate full-monitor decisions while the same preview or fullscreen app
/// remains active. Ordinary focus changes do not need their own INFO records.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct FullscreenCandidate {
    pub hwnd: usize,
    pub process_id: u32,
    pub window: Bounds,
    pub monitor: Bounds,
    pub style: isize,
    pub maximized: bool,
    pub surface: ShellSurface,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_image_query_preserves_native_failure_details() {
        unsafe {
            let image = process_image(GetCurrentProcessId()).unwrap();
            assert!(std::path::Path::new(&image).is_absolute());
            let failure = process_image(0).unwrap_err();
            assert_eq!(failure.stage, "open_process");
            assert_ne!(failure.code, 0);
        }
    }

    #[test]
    fn visibility_diagnostic_escapes_process_and_class_fields() {
        let identity = Identity {
            hwnd: 42,
            process_id: 7,
            shell_process_id: 9,
            class: "untrusted\nclass".into(),
            image: "C:\\test\rimage.exe".into(),
            failure: Some(QueryFailure {
                stage: "process_image",
                code: 5,
            }),
            ..Default::default()
        };
        let diagnostic = identity.diagnostic();
        assert!(!diagnostic.contains(['\r', '\n']));
        assert!(diagnostic.contains("pid=7 shell_pid=9"));
        assert!(diagnostic.contains("query_stage=process_image query_code=5"));
        assert!(diagnostic.contains("untrusted\\nclass"));
    }
}
