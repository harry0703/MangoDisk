#[cfg(target_os = "linux")]
pub use crate::linux::LinuxPlatform as CurrentPlatform;
#[cfg(target_os = "macos")]
pub use crate::macos::MacOsPlatform as CurrentPlatform;
#[cfg(windows)]
pub use crate::windows::WindowsPlatform as CurrentPlatform;

/// Returns the stateless adapter for the current operating system.
pub fn current_platform() -> CurrentPlatform {
    CurrentPlatform
}

/// Resolves private application roots for adapters that do not run inside
/// Tauri. The desktop adapter uses Tauri's path resolver directly.
pub fn application_directories(
    identifier: &str,
) -> crate::PlatformResult<crate::ApplicationDirectories> {
    if identifier.is_empty()
        || identifier.contains(['/', '\\'])
        || identifier == "."
        || identifier == ".."
    {
        return Err(crate::PlatformError::invalid_path(
            "application identifier is invalid",
        ));
    }
    #[cfg(target_os = "macos")]
    {
        crate::macos::application_directories(identifier)
    }
    #[cfg(windows)]
    {
        crate::windows::application_directories(identifier)
    }
    #[cfg(target_os = "linux")]
    {
        crate::linux::application_directories(identifier)
    }
}

#[cfg(test)]
mod tests {
    use super::application_directories;

    #[test]
    fn application_identifier_rejects_path_components() {
        for identifier in ["", ".", "..", "nested/name", r"nested\name"] {
            assert!(application_directories(identifier).is_err());
        }
    }
}
