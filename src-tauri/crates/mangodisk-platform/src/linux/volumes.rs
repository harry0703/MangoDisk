use std::path::Path;

use crate::{PlatformError, PlatformResult, ScanConcurrency, VolumeInfo};

pub(crate) fn system_volume() -> PlatformResult<VolumeInfo> {
    volume_for_path(Path::new("/"))
}

pub(crate) fn volumes() -> PlatformResult<Vec<VolumeInfo>> {
    let mut result = Vec::new();
    let content = std::fs::read_to_string("/proc/mounts")
        .map_err(|error| PlatformError::io("read /proc/mounts", &error))?;

    for line in content.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 2 {
            continue;
        }
        let mount_point = parts[1];
        // Skip virtual filesystems
        if is_virtual_filesystem(parts[0]) {
            continue;
        }
        if let Ok(info) = stat_volume(mount_point) {
            result.push(info);
        }
    }

    if result.is_empty() {
        return Err(PlatformError::operation_failed(
            "no mountable volumes found",
        ));
    }

    Ok(result)
}

fn volume_for_path(path: &Path) -> PlatformResult<VolumeInfo> {
    let mount_point = find_mount_point(path)?;
    stat_volume(&mount_point)
}

fn find_mount_point(path: &Path) -> PlatformResult<String> {
    let canonical = std::fs::canonicalize(path)
        .map_err(|error| PlatformError::io("canonicalize path", &error))?;
    let canonical_str = canonical.to_string_lossy().to_string();

    let content = std::fs::read_to_string("/proc/mounts")
        .map_err(|error| PlatformError::io("read /proc/mounts", &error))?;

    let mut best_match = String::from("/");
    for line in content.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 2 {
            continue;
        }
        let mount_point = parts[1];
        if canonical_str.starts_with(mount_point) && mount_point.len() > best_match.len() {
            best_match = mount_point.to_string();
        }
    }

    Ok(best_match)
}

fn stat_volume(mount_point: &str) -> PlatformResult<VolumeInfo> {
    let mut stat: libc::statfs = unsafe { std::mem::zeroed() };
    let c_path = std::ffi::CString::new(mount_point).map_err(|error| {
        PlatformError::operation_failed(format!("invalid mount point: {error}"))
    })?;

    let result = unsafe { libc::statfs(c_path.as_ptr(), &mut stat) };
    if result != 0 {
        return Err(PlatformError::io(
            "statfs",
            &std::io::Error::last_os_error(),
        ));
    }

    let total_bytes = (stat.f_blocks as u64) * (stat.f_bsize as u64);
    let available_bytes = (stat.f_bavail as u64) * (stat.f_bsize as u64);
    let used_bytes = total_bytes.saturating_sub(available_bytes);

    Ok(VolumeInfo {
        name: mount_point.to_string(),
        mount_point: mount_point.to_string(),
        total_bytes,
        available_bytes,
        used_bytes,
        scan_concurrency: ScanConcurrency::solid_state(),
    })
}

fn is_virtual_filesystem(fstype: &str) -> bool {
    matches!(
        fstype,
        "proc"
            | "sysfs"
            | "devtmpfs"
            | "devpts"
            | "tmpfs"
            | "securityfs"
            | "cgroup"
            | "cgroup2"
            | "pstore"
            | "bpf"
            | "debugfs"
            | "tracefs"
            | "fusectl"
            | "configfs"
            | "binfmt_misc"
            | "autofs"
            | "overlay"
            | "nsfs"
            | "rpc_pipefs"
            | "nfsd"
    )
}
