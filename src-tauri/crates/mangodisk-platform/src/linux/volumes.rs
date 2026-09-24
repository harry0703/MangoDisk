use std::{
    collections::BTreeSet,
    ffi::{CString, OsString},
    os::unix::ffi::{OsStrExt, OsStringExt},
    path::{Path, PathBuf},
};

use crate::{PlatformError, PlatformResult, ScanConcurrency, ScanDeviceClass, VolumeInfo};

#[derive(Debug, Clone, Eq, PartialEq)]
struct MountRecord {
    device_id: String,
    mount_point: PathBuf,
    filesystem: String,
}

pub(crate) fn system_volume() -> PlatformResult<VolumeInfo> {
    volume_for_path(Path::new("/"))
}

pub(crate) fn volumes() -> PlatformResult<Vec<VolumeInfo>> {
    enumerate_volumes(true, true)
}

pub(crate) fn local_volumes() -> PlatformResult<Vec<VolumeInfo>> {
    enumerate_volumes(false, false)
}

fn enumerate_volumes(
    include_network: bool,
    emit_diagnostics: bool,
) -> PlatformResult<Vec<VolumeInfo>> {
    let started = std::time::Instant::now();
    let records = mount_records()?;
    let mut result = Vec::new();
    let mut seen = BTreeSet::new();
    let mut filtered_count = 0_u64;
    let mut stat_failure_count = 0_u64;

    for record in records {
        if record.mount_point != Path::new("/")
            && (is_virtual_filesystem(&record.filesystem)
                || (!include_network && is_network_filesystem(&record.filesystem))
                || is_virtual_mount_point(&record.mount_point))
        {
            filtered_count = filtered_count.saturating_add(1);
            continue;
        }
        if !seen.insert(record.mount_point.clone()) {
            filtered_count = filtered_count.saturating_add(1);
            continue;
        }
        match stat_volume(&record) {
            Ok(info) => result.push(info),
            Err(error) => {
                stat_failure_count = stat_failure_count.saturating_add(1);
                if emit_diagnostics {
                    log::warn!(
                        "linux_volume_stat_failed mount_point={:?} code={:?} diagnostic={:?}",
                        record.mount_point,
                        error.code(),
                        error.diagnostic()
                    );
                }
            }
        }
    }

    if result.is_empty() {
        return Err(PlatformError::operation_failed(
            "no mountable volumes found",
        ));
    }

    result.sort_by(|left, right| left.mount_point.cmp(&right.mount_point));
    if emit_diagnostics {
        log::info!(
            "linux_volume_inventory_ready volume_count={} filtered_count={} stat_failure_count={} elapsed_ms={}",
            result.len(),
            filtered_count,
            stat_failure_count,
            started.elapsed().as_millis()
        );
    }
    Ok(result)
}

pub(crate) fn system_filesystem_kind() -> PlatformResult<String> {
    let records = mount_records()?;
    records
        .iter()
        .find(|record| record.mount_point == Path::new("/"))
        .map(|record| record.filesystem.clone())
        .ok_or_else(|| PlatformError::operation_failed("system filesystem is unavailable"))
}

pub(super) fn volume_for_path(path: &Path) -> PlatformResult<VolumeInfo> {
    let canonical = std::fs::canonicalize(path)
        .map_err(|error| PlatformError::io("canonicalize path", &error))?;
    let records = mount_records()?;
    let mount = select_mount(&records, &canonical)
        .ok_or_else(|| PlatformError::operation_failed("no containing mount point found"))?;
    stat_volume(mount)
}

fn mount_records() -> PlatformResult<Vec<MountRecord>> {
    let content = std::fs::read_to_string("/proc/self/mountinfo")
        .map_err(|error| PlatformError::io("read /proc/self/mountinfo", &error))?;
    Ok(content.lines().filter_map(parse_mountinfo_line).collect())
}

fn parse_mountinfo_line(line: &str) -> Option<MountRecord> {
    let (mount_fields, filesystem_fields) = line.split_once(" - ")?;
    let mut mount_fields = mount_fields.split_whitespace();
    let device_id = mount_fields.nth(2)?.to_string();
    let mount_point = decode_mount_field(mount_fields.nth(1)?);
    let filesystem = filesystem_fields.split_whitespace().next()?.to_string();
    Some(MountRecord {
        device_id,
        mount_point: PathBuf::from(mount_point),
        filesystem,
    })
}

fn decode_mount_field(field: &str) -> OsString {
    let bytes = field.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'\\'
            && index + 3 < bytes.len()
            && bytes[index + 1..=index + 3]
                .iter()
                .all(|byte| matches!(byte, b'0'..=b'7'))
        {
            let value = (bytes[index + 1] - b'0') * 64
                + (bytes[index + 2] - b'0') * 8
                + (bytes[index + 3] - b'0');
            decoded.push(value);
            index += 4;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    OsString::from_vec(decoded)
}

fn select_mount<'a>(records: &'a [MountRecord], path: &Path) -> Option<&'a MountRecord> {
    records
        .iter()
        .filter(|record| path.starts_with(&record.mount_point))
        .max_by_key(|record| record.mount_point.components().count())
}

fn stat_volume(record: &MountRecord) -> PlatformResult<VolumeInfo> {
    let mut stat: libc::statfs = unsafe { std::mem::zeroed() };
    let c_path = CString::new(record.mount_point.as_os_str().as_bytes()).map_err(|error| {
        PlatformError::operation_failed(format!("invalid mount point: {error}"))
    })?;

    let result = unsafe { libc::statfs(c_path.as_ptr(), &mut stat) };
    if result != 0 {
        return Err(PlatformError::io(
            "statfs",
            &std::io::Error::last_os_error(),
        ));
    }

    let block_size = u64::try_from(stat.f_bsize)
        .map_err(|_| PlatformError::operation_failed("filesystem block size is invalid"))?;
    let total_bytes = stat.f_blocks.saturating_mul(block_size);
    let available_bytes = stat.f_bavail.saturating_mul(block_size);
    let used_bytes = total_bytes.saturating_sub(available_bytes);
    let mount_point = record.mount_point.to_string_lossy().into_owned();

    Ok(VolumeInfo {
        name: mount_point.clone(),
        mount_point,
        total_bytes,
        available_bytes,
        used_bytes,
        scan_concurrency: scan_concurrency(record),
    })
}

fn scan_concurrency(record: &MountRecord) -> ScanConcurrency {
    if is_network_filesystem(&record.filesystem) {
        return ScanConcurrency::conservative(ScanDeviceClass::Network);
    }
    let Ok(device) = std::fs::canonicalize(format!("/sys/dev/block/{}", record.device_id)) else {
        return ScanConcurrency::conservative(ScanDeviceClass::Unknown);
    };
    for ancestor in device.ancestors() {
        if read_flag(ancestor.join("removable")) == Some(true) {
            return ScanConcurrency::conservative(ScanDeviceClass::Removable);
        }
        match read_flag(ancestor.join("queue/rotational")) {
            Some(true) => return ScanConcurrency::rotational(),
            Some(false) => return ScanConcurrency::solid_state(),
            None => {}
        }
    }
    ScanConcurrency::conservative(ScanDeviceClass::Unknown)
}

fn read_flag(path: PathBuf) -> Option<bool> {
    match std::fs::read_to_string(path).ok()?.trim() {
        "0" => Some(false),
        "1" => Some(true),
        _ => None,
    }
}

fn is_network_filesystem(filesystem: &str) -> bool {
    matches!(
        filesystem,
        "9p" | "ceph" | "cifs" | "davfs" | "fuse.sshfs" | "glusterfs" | "nfs" | "nfs4" | "smb3"
    )
}

fn is_virtual_filesystem(filesystem: &str) -> bool {
    matches!(
        filesystem,
        "autofs"
            | "binfmt_misc"
            | "bpf"
            | "cgroup"
            | "cgroup2"
            | "configfs"
            | "debugfs"
            | "devpts"
            | "devtmpfs"
            | "fuse.gvfsd-fuse"
            | "fuse.portal"
            | "fusectl"
            | "hugetlbfs"
            | "mqueue"
            | "nfsd"
            | "nsfs"
            | "overlay"
            | "proc"
            | "pstore"
            | "ramfs"
            | "rpc_pipefs"
            | "securityfs"
            | "sysfs"
            | "tmpfs"
            | "tracefs"
    )
}

fn is_virtual_mount_point(path: &Path) -> bool {
    // A host may bind a non-virtual filesystem below these system trees.
    // Its filesystem type alone must not expose that mount as a user disk.
    ["/proc", "/sys", "/dev", "/snap", "/var/lib/snapd"]
        .iter()
        .any(|root| path.starts_with(root))
        || path.ends_with("gvfs")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(path: &str) -> MountRecord {
        MountRecord {
            device_id: "8:1".to_string(),
            mount_point: PathBuf::from(path),
            filesystem: "ext4".to_string(),
        }
    }

    #[test]
    fn mountinfo_parser_uses_the_filesystem_field_and_decodes_paths() {
        let parsed = parse_mountinfo_line(
            "37 25 11:1 / /media/example/My\\040Disk ro,nosuid - iso9660 /dev/sr0 ro",
        )
        .unwrap();
        assert_eq!(parsed.device_id, "11:1");
        assert_eq!(parsed.mount_point, Path::new("/media/example/My Disk"));
        assert_eq!(parsed.filesystem, "iso9660");
    }

    #[test]
    fn mount_selection_observes_path_component_boundaries() {
        let records = vec![record("/"), record("/home/a"), record("/home/ab")];
        assert_eq!(
            select_mount(&records, Path::new("/home/ab/file"))
                .unwrap()
                .mount_point,
            Path::new("/home/ab")
        );
        assert_eq!(
            select_mount(&records, Path::new("/home/abc/file"))
                .unwrap()
                .mount_point,
            Path::new("/")
        );
    }

    #[test]
    fn virtual_and_network_filesystems_are_classified_explicitly() {
        assert!(is_virtual_filesystem("proc"));
        assert!(is_virtual_filesystem("fuse.gvfsd-fuse"));
        assert!(is_virtual_filesystem("fuse.portal"));
        assert!(!is_virtual_filesystem("ext4"));
        assert!(is_network_filesystem("nfs4"));
        assert!(!is_network_filesystem("btrfs"));
        assert!(is_virtual_mount_point(Path::new("/proc/runner")));
        assert!(is_virtual_mount_point(Path::new("/sys/firmware")));
        assert!(!is_virtual_mount_point(Path::new("/proc-data")));
        assert!(!is_virtual_mount_point(Path::new("/run/media/drive")));
    }

    #[test]
    fn actual_inventory_keeps_root_and_excludes_virtual_and_snap_mounts() {
        let volumes = super::volumes().unwrap();
        assert_eq!(
            volumes
                .iter()
                .filter(|volume| volume.mount_point == "/")
                .count(),
            1
        );
        assert!(
            volumes
                .iter()
                .all(|volume| !is_virtual_mount_point(Path::new(&volume.mount_point))),
            "inventory exposed a system mount: {:?}",
            volumes
                .iter()
                .map(|volume| &volume.mount_point)
                .collect::<Vec<_>>()
        );
    }
}
