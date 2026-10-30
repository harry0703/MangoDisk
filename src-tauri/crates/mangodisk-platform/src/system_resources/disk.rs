//! Mounted local-volume capacity; no directory traversal or network-volume polling.
use serde::Serialize;

#[cfg(target_os = "macos")]
#[path = "disk/macos.rs"]
mod native;
#[cfg(windows)]
#[path = "disk/windows.rs"]
mod native;
#[cfg(target_os = "linux")]
#[path = "disk/linux.rs"]
mod native;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceDisk {
    /// Hardware identity when available; a volume-bound fallback never aliases a replacement.
    pub id: String,
    pub number: Option<u32>,
    pub model: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceVolume {
    /// Stable native identity, persisted locally and never logged.
    pub id: String,
    pub name: String,
    pub system: bool,
    pub file_system: Option<String>,
    pub mount_points: Vec<String>,
    pub devices: Vec<ResourceDisk>,
    #[serde(skip)]
    pub mount_point: String,
}

#[derive(Debug, Clone, Copy)]
pub struct VolumeCapacity {
    pub total_bytes: u64,
    /// Display availability includes system-reclaimable storage on macOS.
    /// This estimate is not a guarantee that an allocation will succeed.
    pub available_bytes: u64,
}

/// Completed filesystem mutations bypass the short macOS display cache.
pub fn invalidate_capacity_cache() {
    #[cfg(target_os = "macos")]
    crate::macos::volume_capacity::invalidate();
}

#[cfg(any(target_os = "macos", windows, target_os = "linux"))]
pub use native::capacity;

#[cfg(any(target_os = "macos", windows, target_os = "linux"))]
pub fn list() -> crate::PlatformResult<Vec<ResourceVolume>> {
    let mut volumes = native::list()?;
    #[cfg(any(target_os = "macos", windows))]
    for volume in &mut volumes {
        match physical_devices(volume) {
            Ok(devices) => volume.devices = devices,
            Err(error) => log::debug!(
                "disk_topology_unavailable volume={} code={:?} error={}",
                crate::diagnostics::text(&volume.name),
                error.code(),
                crate::diagnostics::text(&error)
            ),
        }
    }
    volumes.sort_by(|left, right| {
        right
            .system
            .cmp(&left.system)
            .then_with(|| left.mount_points.cmp(&right.mount_points))
            .then_with(|| left.name.cmp(&right.name))
    });
    Ok(volumes)
}

#[cfg(any(target_os = "macos", windows))]
pub(crate) fn physical_devices(
    volume: &ResourceVolume,
) -> crate::PlatformResult<Vec<ResourceDisk>> {
    super::disk_smart::physical_devices(volume)
}

#[cfg(target_os = "macos")]
pub(super) fn matches_mounted_identity(volume: &ResourceVolume) -> bool {
    native::matches_identity(volume)
}

#[cfg(any(target_os = "macos", windows))]
pub(super) fn device_identity(
    namespace: &str,
    model: Option<&str>,
    serial: Option<&str>,
    fallback: &str,
) -> String {
    if let Some(serial) = serial.filter(|value| !value.is_empty()) {
        // The hash is the protocol identity, not a replacement for diagnostic object names.
        let mut identity = blake3::Hasher::new();
        identity.update(model.unwrap_or_default().as_bytes());
        identity.update(&[0]);
        identity.update(serial.as_bytes());
        format!("{namespace}:{}", identity.finalize().to_hex())
    } else {
        format!("{namespace}:volume:{fallback}")
    }
}

#[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
pub fn list() -> crate::PlatformResult<Vec<ResourceVolume>> {
    Err(unavailable())
}
#[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
pub fn capacity(_volume: &ResourceVolume) -> crate::PlatformResult<VolumeCapacity> {
    Err(unavailable())
}

fn unavailable() -> crate::PlatformError {
    crate::PlatformError::new(
        crate::PlatformErrorCode::OperationFailed,
        "volume capacity unavailable",
    )
}

#[cfg(test)]
mod tests {
    #[test]
    #[cfg(any(target_os = "macos", windows, target_os = "linux"))]
    fn system_volume_is_identifiable_and_has_valid_capacity() {
        let volumes = super::list().expect("local volume enumeration must succeed");
        let system: Vec<_> = volumes.iter().filter(|volume| volume.system).collect();
        assert_eq!(
            system.len(),
            1,
            "system volume must have one unambiguous identity"
        );
        let capacity = super::capacity(system[0]).expect("system capacity must be readable");
        assert!(capacity.total_bytes > 0);
        assert!(capacity.available_bytes <= capacity.total_bytes);
    }
}
