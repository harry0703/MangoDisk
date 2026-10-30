//! Explicit application termination over the shared, identity-checked process adapter.
use std::path::Path;

use crate::{ApplicationProcessCloseResult, PlatformError, PlatformResult};

/// This policy is shared by capability publication and execution. OS-owned
/// images and shared hosts must not become force targets merely because they
/// happen to appear in a memory ranking.
pub fn allowed(path: &Path) -> bool {
    if !path.is_absolute()
        || path
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir))
    {
        return false;
    }
    #[cfg(windows)]
    {
        use std::{os::windows::ffi::OsStringExt, sync::OnceLock};
        static WINDOWS: OnceLock<Option<std::path::PathBuf>> = OnceLock::new();
        let root = WINDOWS.get_or_init(|| {
            let mut buffer = vec![0u16; 32768];
            let size = unsafe {
                windows_sys::Win32::System::SystemInformation::GetWindowsDirectoryW(
                    buffer.as_mut_ptr(),
                    buffer.len() as u32,
                )
            } as usize;
            (size > 0 && size < buffer.len())
                .then(|| std::ffi::OsString::from_wide(&buffer[..size]).into())
        });
        root.as_deref()
            .is_some_and(|root| windows_target_allowed(path, root))
    }
    #[cfg(target_os = "macos")]
    {
        path.extension().is_some_and(|ext| ext == "app")
            && !path.starts_with("/System/Library")
            && !path.starts_with("/Library/Apple")
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    false
}

#[cfg(windows)]
fn windows_target_allowed(path: &Path, windows: &Path) -> bool {
    use crate::{current_platform, Platform};
    path.extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("exe"))
        && !current_platform().path_is_same_or_child(path, windows)
        && !path.file_name().is_some_and(|name| {
            [
                "msedgewebview2.exe",
                "svchost.exe",
                "dllhost.exe",
                "rundll32.exe",
                "runtimebroker.exe",
            ]
            .iter()
            .any(|host| name.eq_ignore_ascii_case(host))
        })
}

pub fn execute(path: &Path) -> PlatformResult<ApplicationProcessCloseResult> {
    use crate::{
        current_platform, ApplicationProcessCloseMode, ApplicationProcessTarget, Platform,
    };
    if !allowed(path) {
        return Err(PlatformError::operation_failed(
            "the application image is protected from forced termination",
        ));
    }
    // Core supplies only a freshly resolved image/bundle, never a name alias.
    current_platform().close_application_processes(
        &ApplicationProcessTarget {
            executable_names: Vec::new(),
            executable_paths: vec![path.to_path_buf()],
        },
        ApplicationProcessCloseMode::Force,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn relative_and_parent_paths_cannot_authorize_force_quit() {
        assert!(!allowed(Path::new("app.exe")));
        assert!(!allowed(Path::new(
            "/Applications/../System/Library/Test.app"
        )));
    }
    #[cfg(windows)]
    #[test]
    fn system_paths_and_shared_hosts_are_blocked_with_component_boundaries() {
        let root = Path::new(r"C:\Windows");
        for path in [
            r"C:\Windows\explorer.exe",
            r"c:\WINDOWS\System32\notepad.exe",
            r"C:\Apps\msedgewebview2.exe",
            r"C:\Apps\SVCHOST.EXE",
        ] {
            assert!(
                !windows_target_allowed(Path::new(path), root),
                "protected target {path}"
            );
        }
        assert!(windows_target_allowed(
            Path::new(r"C:\WindowsApps\Example.exe"),
            root
        ));
        assert!(windows_target_allowed(
            Path::new(r"D:\Apps\Example.exe"),
            root
        ));
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn system_services_are_blocked_but_application_bundles_are_supported() {
        assert!(!allowed(Path::new(
            "/System/Library/CoreServices/Finder.app"
        )));
        assert!(!allowed(Path::new("/Library/Apple/Helper.app")));
        assert!(allowed(Path::new("/Applications/Example.app")));
        assert!(allowed(Path::new("/System/Applications/TextEdit.app")));
    }
}
