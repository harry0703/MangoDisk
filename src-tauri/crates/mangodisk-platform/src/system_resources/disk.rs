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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceVolume {
    /// Stable native identity, persisted locally and never logged.
    pub id: String,
    pub name: String,
    pub system: bool,
    #[serde(skip)]
    pub mount_point: String,
}

#[derive(Debug, Clone, Copy)]
pub struct VolumeCapacity {
    pub total_bytes: u64,
    pub available_bytes: u64,
}

#[cfg(any(target_os = "macos", windows, target_os = "linux"))]
pub use native::{capacity, list};

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
