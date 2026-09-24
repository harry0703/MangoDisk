use std::{ffi::CString, os::unix::ffi::OsStrExt, path::Path};

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
    path.starts_with("/proc")
        || path.starts_with("/sys")
        || path.starts_with("/dev")
        || path.starts_with("/run")
        || path.starts_with("/etc")
        || path.starts_with("/boot")
        || path.starts_with("/usr/lib")
        || path.starts_with("/usr/share")
        || path.starts_with("/usr/libexec")
        || path.starts_with("/sbin")
        || path.starts_with("/bin")
}

/// Returns whether a path lives inside a universal package manager's own tree.
///
/// Snap mounts each revision as a read-only squashfs image under `/snap` and
/// `/var/lib/snapd/snap`, and Flatpak lays out shared runtimes and app exports
/// under `/var/lib/flatpak`. Both formats intentionally duplicate identical
/// files across revisions and across unrelated packages (shared themes, icon
/// sets, runtimes), so large-file and duplicate-file discovery must not offer
/// their contents as reclaimable: the files are not user data, most of the
/// tree is read-only, and any writable remainder is package-manager state
/// whose removal can corrupt an installed package instead of freeing space.
pub(crate) fn is_package_manager_owned(path: &Path) -> bool {
    path.starts_with("/snap")
        || path.starts_with("/var/lib/snapd")
        || path.starts_with("/var/lib/flatpak")
}

/// Returns whether the current process cannot write to a directory.
///
/// Deleting a file requires write and search permission on its containing
/// directory, not just on the file itself. Distributions scatter root-owned
/// or other-user-owned trees outside the fixed system paths above (locally
/// installed software under `/usr/local`, package caches under `/var/cache`,
/// service state under `/var/lib`, `/opt` installs), and desktop sessions run
/// unprivileged. Pruning large-file and duplicate discovery at the first
/// unwritable directory keeps results limited to files this process can
/// actually remove instead of offering deletions that fail with an
/// unexplained permission error. This is intentionally conservative: a
/// writable subdirectory nested below an unwritable one is skipped too,
/// since MangoDisk cannot request elevation for this flow yet.
pub(crate) fn is_unwritable_by_current_user(path: &Path) -> bool {
    // A path that does not exist on disk cannot justify pruning: storage tests
    // and in-memory snapshots query fake roots, and a vanished directory is
    // invisible to the ongoing scan anyway. Only existing directories are judged.
    if !path.is_dir() {
        return false;
    }
    let Ok(candidate) = CString::new(path.as_os_str().as_bytes()) else {
        return true;
    };
    // SAFETY: `candidate` is a valid NUL-terminated C string for the lifetime of this call.
    unsafe { libc::access(candidate.as_ptr(), libc::W_OK | libc::X_OK) != 0 }
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

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    #[test]
    fn system_critical_paths_use_component_boundaries() {
        for path in [
            "/proc/1/status",
            "/sys/class/block",
            "/dev/disk/by-id",
            "/run/user/1000",
            "/etc/hosts",
            "/usr/lib/libc.so",
        ] {
            assert!(is_system_critical(Path::new(path)), "{path}");
        }
        for path in [
            "/process-data/report",
            "/system-backup/archive",
            "/developer/project",
            "/runtime-notes/file",
            "/home/user/document",
        ] {
            assert!(!is_system_critical(Path::new(path)), "{path}");
        }
    }

    #[test]
    fn package_manager_scope_covers_snap_and_flatpak_without_matching_similar_names() {
        for path in [
            "/snap/gtk-common-themes/1535/share/icons/Yaru",
            "/var/lib/snapd/snap/snap-store/1419/bin/lib/libflutter_linux_gtk.so",
            "/var/lib/flatpak/app/org.mozilla.firefox/current/active/files",
        ] {
            assert!(is_package_manager_owned(Path::new(path)), "{path}");
        }
        for path in [
            "/snapshot-backup/report.pdf",
            "/var/lib/snapd-cache/report.pdf",
            "/home/user/snap-notes.txt",
        ] {
            assert!(!is_package_manager_owned(Path::new(path)), "{path}");
        }
    }

    #[test]
    fn unwritable_directories_are_detected_without_flagging_owned_ones() {
        // Root bypasses the discretionary permission checks `access` reports,
        // so this boundary cannot be observed while running as root.
        if unsafe { libc::geteuid() } == 0 {
            return;
        }
        let root = std::env::temp_dir().join(format!(
            "mangodisk-unwritable-directory-test-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).expect("fixture root should be created");

        let writable = root.join("writable");
        std::fs::create_dir(&writable).expect("writable child directory should be created");
        assert!(!is_unwritable_by_current_user(&writable));

        let readonly = root.join("readonly");
        std::fs::create_dir(&readonly).expect("readonly child directory should be created");
        std::fs::set_permissions(&readonly, std::fs::Permissions::from_mode(0o555))
            .expect("write permission should be removable");
        assert!(is_unwritable_by_current_user(&readonly));

        // Restore write permission before cleanup, otherwise removing this directory would fail too.
        std::fs::set_permissions(&readonly, std::fs::Permissions::from_mode(0o755))
            .expect("write permission should be restorable");
        std::fs::remove_dir_all(&root).expect("fixture root should be removed");
    }
}
