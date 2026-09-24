//! System-wide block-device counters. Volume capacity has a separate scope.
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;
#[cfg(target_os = "linux")]
pub use linux::DiskIoReader;
#[cfg(target_os = "macos")]
pub use macos::DiskIoReader;
#[cfg(windows)]
pub use windows::DiskIoReader;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceCounters {
    /// Private transient identity used to reject topology changes; never serialized or logged.
    pub id: String,
    pub read_bytes: u64,
    pub written_bytes: u64,
}

fn unavailable() -> crate::PlatformError {
    crate::PlatformError::new(
        crate::PlatformErrorCode::Unsupported,
        "disk activity counters unavailable",
    )
}

#[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
#[derive(Default)]
pub struct DiskIoReader {}
#[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
impl DiskIoReader {
    pub fn read(&mut self) -> crate::PlatformResult<Vec<DeviceCounters>> {
        Err(unavailable())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    #[cfg(any(target_os = "macos", windows, target_os = "linux"))]
    fn native_disk_counters_are_available_and_identifiable() {
        let counters = super::DiskIoReader::default()
            .read()
            .expect("native disk counters");
        assert!(!counters.is_empty());
        assert!(counters.iter().all(|device| !device.id.is_empty()));
        assert!(counters.windows(2).all(|pair| pair[0].id < pair[1].id));
    }
    #[test]
    #[ignore = "manual native sampler timing probe"]
    #[cfg(any(target_os = "macos", windows, target_os = "linux"))]
    fn disk_sampler_timing_probe() {
        let mut reader = super::DiskIoReader::default();
        for sample in 0..5 {
            let started = std::time::Instant::now();
            let counters = reader.read().expect("native disk counters");
            eprintln!(
                "disk_io_probe sample={sample} devices={} elapsed_us={}",
                counters.len(),
                started.elapsed().as_micros()
            );
            std::thread::sleep(std::time::Duration::from_millis(300));
        }
    }
}
