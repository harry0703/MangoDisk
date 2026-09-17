use std::path::Path;

use crate::{PlatformError, PlatformResult, UserDirectories};

pub(crate) fn user_directories() -> PlatformResult<UserDirectories> {
    let home = dirs::home_dir()
        .ok_or_else(|| PlatformError::operation_failed("unable to determine home directory"))?;

    let temp = std::env::temp_dir();
    let cache = dirs::cache_dir().unwrap_or_else(|| home.join(".cache"));

    let application_data_directories = [
        dirs::config_dir().unwrap_or_else(|| home.join(".config")),
        dirs::data_dir().unwrap_or_else(|| home.join(".local/share")),
    ];

    Ok(UserDirectories::new(
        home,
        temp,
        cache,
        application_data_directories,
    ))
}

/// Returns whether a path belongs to a system-critical Linux directory.
pub(crate) fn is_system_critical(path: &Path) -> bool {
    let Some(first) = path.components().next() else {
        return false;
    };
    let first_str = first.as_os_str().to_string_lossy();

    matches!(first_str.as_ref(), "/proc" | "/sys" | "/dev" | "/run")
        || path.starts_with("/etc")
        || path.starts_with("/boot")
        || path.starts_with("/usr/lib")
        || path.starts_with("/usr/share")
        || path.starts_with("/usr/libexec")
        || path.starts_with("/sbin")
        || path.starts_with("/bin")
}

/// Returns whether a path must never be used as a cleanup root.
pub(crate) fn is_protected_cleanup_path(path: &Path) -> bool {
    let protected = [
        "/etc", "/boot", "/usr", "/lib", "/lib64", "/sbin", "/bin", "/proc", "/sys", "/dev",
        "/run", "/srv", "/opt",
    ];
    for prefix in &protected {
        if path == Path::new(prefix) || path.starts_with(prefix) {
            return true;
        }
    }
    false
}
