use std::{ffi::CString, os::unix::fs::MetadataExt, path::Path};

use super::{unavailable, ResourceVolume, VolumeCapacity};
use crate::PlatformResult;

const SYSTEM_MOUNT: &str = "/";

fn identity(path: &Path) -> PlatformResult<String> {
    let metadata = std::fs::metadata(path).map_err(|_| unavailable())?;
    Ok(format!(
        "linux-volume:{}:{}",
        metadata.dev(),
        path.to_string_lossy()
    ))
}

pub fn list() -> PlatformResult<Vec<ResourceVolume>> {
    let mut result = crate::linux::volumes::local_volumes()?
        .into_iter()
        .filter_map(|volume| {
            let mount_point = std::path::PathBuf::from(&volume.mount_point);
            let id = identity(&mount_point).ok()?;
            let system = volume.mount_point == SYSTEM_MOUNT;
            let name = if system {
                "System".to_string()
            } else {
                mount_point
                    .file_name()
                    .filter(|name| !name.is_empty())
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| volume.mount_point.clone())
            };
            Some(ResourceVolume {
                id,
                name,
                system,
                mount_point: volume.mount_point,
            })
        })
        .collect::<Vec<_>>();
    result.sort_by(|left, right| {
        right
            .system
            .cmp(&left.system)
            .then_with(|| left.name.cmp(&right.name))
            .then_with(|| left.mount_point.cmp(&right.mount_point))
    });
    if result.is_empty() {
        return Err(unavailable());
    }
    Ok(result)
}

pub fn capacity(volume: &ResourceVolume) -> PlatformResult<VolumeCapacity> {
    let path = Path::new(&volume.mount_point);
    if volume.id != identity(path)? {
        return Err(unavailable());
    }
    let path = CString::new(volume.mount_point.as_bytes()).map_err(|_| unavailable())?;
    let mut stat = unsafe { std::mem::zeroed::<libc::statvfs>() };
    if unsafe { libc::statvfs(path.as_ptr(), &mut stat) } != 0 {
        return Err(unavailable());
    }
    let block_size = if stat.f_frsize == 0 {
        stat.f_bsize
    } else {
        stat.f_frsize
    };
    Ok(VolumeCapacity {
        total_bytes: stat.f_blocks.saturating_mul(block_size),
        available_bytes: stat.f_bavail.saturating_mul(block_size),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_volume_list_has_stable_unique_identities() {
        let volumes = list().unwrap();
        let identities = volumes
            .iter()
            .map(|volume| volume.id.as_str())
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(identities.len(), volumes.len());
        assert_eq!(volumes.iter().filter(|volume| volume.system).count(), 1);
    }
}
