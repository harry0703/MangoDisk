//! Prepare one health observation; later panel visits retain the slow cadence.
use mangodisk_platform::{
    system_resources::{
        disk::ResourceVolume,
        disk_health::{self, DiskHealth},
    },
    PlatformResult,
};

#[derive(Default)]
pub(super) struct HealthSampler {
    identity: Option<(String, String, Vec<String>)>,
    attempted_at: Option<u64>,
    retry_ms: u64,
    outcome: Option<String>,
    cached: Option<(u64, PlatformResult<DiskHealth>)>,
}

impl HealthSampler {
    fn needs_initial_read(&self, volume: &ResourceVolume) -> bool {
        self.cached.is_none() || self.identity.as_ref() != Some(&source_identity(volume))
    }

    pub fn observation(
        &mut self,
        volume: &ResourceVolume,
        now_ms: u64,
        refresh: bool,
    ) -> (u64, PlatformResult<DiskHealth>) {
        if refresh || self.needs_initial_read(volume) {
            self.read(volume, now_ms)
        } else {
            // Visibility changes can reject an in-flight result. Republish its
            // cached observation without reopening the device while hidden.
            self.cached
                .as_ref()
                .expect("initial health attempt cached")
                .clone()
        }
    }

    fn read(&mut self, volume: &ResourceVolume, now_ms: u64) -> (u64, PlatformResult<DiskHealth>) {
        let _ = self.read_with(volume, now_ms, || disk_health::read(volume));
        self.cached
            .as_ref()
            .expect("health attempt initializes cache")
            .clone()
    }

    fn read_with(
        &mut self,
        volume: &ResourceVolume,
        now_ms: u64,
        read: impl FnOnce() -> PlatformResult<DiskHealth>,
    ) -> Option<PlatformResult<DiskHealth>> {
        let identity = source_identity(volume);
        if self.identity.as_ref() != Some(&identity) {
            self.identity = Some(identity);
            self.attempted_at = None;
            self.outcome = None;
        }
        if self
            .attempted_at
            .and_then(|previous| now_ms.checked_sub(previous))
            .is_some_and(|elapsed| elapsed < self.retry_ms)
        {
            return None;
        }
        let started = std::time::Instant::now();
        let result = read();
        log::info!(
            "disk_health_read volume={} elapsed_ms={} outcome={}",
            mangodisk_platform::diagnostics::text(&volume.name),
            started.elapsed().as_millis(),
            if result.is_ok() {
                "ready"
            } else {
                "unavailable"
            }
        );
        self.cached = Some((now_ms, result.clone()));
        self.attempted_at = Some(now_ms);
        self.retry_ms = match &result {
            Ok(_) => 300_000,
            Err(error) if error.code() == mangodisk_platform::PlatformErrorCode::Unsupported => {
                300_000
            }
            Err(_) => 30_000,
        };
        let outcome = match &result {
            Ok(value) => format!(
                "{:?}:{:?}:{:?}",
                value.source,
                value.status,
                value.ata.as_ref().map(|ata| ata.profile)
            ),
            Err(error) => format!("{:?}:{}", error.code(), error.diagnostic()),
        };
        if self.outcome.as_ref() != Some(&outcome) {
            match &result {
                Ok(value) => log::info!(
                    "disk_health_state volume={} outcome=ready source={:?} status={:?} ata_profile={:?} attributes={} decoded={}",
                    mangodisk_platform::diagnostics::text(&volume.name),
                    value.source,
                    value.status,
                    value.ata.as_ref().map(|ata| ata.profile),
                    value.ata.as_ref().map_or(0, |ata| ata.attributes.len()),
                    value.ata.as_ref().map_or(0, |ata| ata.attributes.iter().filter(|value| value.decoded.is_some()).count())
                ),
                Err(error) => log::info!(
                    "disk_health_state volume={} outcome=unavailable code={:?} error={}",
                    mangodisk_platform::diagnostics::text(&volume.name),
                    error.code(),
                    mangodisk_platform::diagnostics::text(error)
                ),
            }
            self.outcome = Some(outcome);
        }
        Some(result)
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }
}

fn source_identity(volume: &ResourceVolume) -> (String, String, Vec<String>) {
    let mut devices: Vec<_> = volume
        .devices
        .iter()
        .map(|device| device.id.clone())
        .collect();
    devices.sort();
    (volume.id.clone(), volume.mount_point.clone(), devices)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mangodisk_platform::{PlatformError, PlatformErrorCode};
    #[test]
    fn unsupported_reopens_are_throttled_and_changed_devices_or_clocks_retry() {
        let mut reader = HealthSampler::default();
        let mut volume = ResourceVolume {
            devices: Vec::new(),
            mount_points: Vec::new(),
            file_system: None,
            id: "one".into(),
            name: "Disk".into(),
            system: true,
            mount_point: "/".into(),
        };
        let missing = || {
            Err(PlatformError::new(
                PlatformErrorCode::Unsupported,
                "not supported",
            ))
        };
        assert!(reader.read_with(&volume, 1000, missing).is_some());
        assert_eq!(reader.cached.as_ref().unwrap().0, 1000);
        assert!(reader
            .read_with(&volume, 1001, || panic!(
                "cached capability must not reopen storage"
            ))
            .is_none());
        assert_eq!(
            reader.cached.as_ref().unwrap().0,
            1000,
            "cache hits retain the native observation timestamp for late-result recovery"
        );
        assert!(reader.read_with(&volume, 301_000, missing).is_some());
        volume.mount_point = "/other".into();
        assert!(reader.read_with(&volume, 301_001, missing).is_some());
        assert!(reader.read_with(&volume, 10, missing).is_some());
        reader.clear();
        assert!(reader.read_with(&volume, 11, missing).is_some());
    }
    #[test]
    fn transient_failures_wait_thirty_seconds() {
        let mut reader = HealthSampler::default();
        let volume = ResourceVolume {
            devices: Vec::new(),
            mount_points: Vec::new(),
            file_system: None,
            id: "disk".into(),
            name: "Disk".into(),
            system: true,
            mount_point: "/".into(),
        };
        let denied = || {
            Err(PlatformError::new(
                PlatformErrorCode::AccessDenied,
                "denied",
            ))
        };
        assert!(reader.read_with(&volume, 1000, denied).is_some());
        assert!(reader.read_with(&volume, 30_999, denied).is_none());
        assert!(reader.read_with(&volume, 31_000, denied).is_some());
    }

    #[test]
    fn successful_health_reads_survive_reopening_until_the_five_minute_deadline() {
        let mut reader = HealthSampler::default();
        let volume = ResourceVolume {
            devices: Vec::new(),
            mount_points: Vec::new(),
            file_system: None,
            id: "disk".into(),
            name: "Disk".into(),
            system: true,
            mount_point: "/".into(),
        };
        let calls = std::cell::Cell::new(0);
        let read = || {
            calls.set(calls.get() + 1);
            Ok(DiskHealth {
                volume_id: volume.id.clone(),
                volume_name: volume.name.clone(),
                status: disk_health::DiskHealthStatus::Healthy,
                source: mangodisk_platform::system_resources::disk_temperature::DiskTemperatureSource::NvmeSmart,
                percentage_used: Some(11),
                total_written_bytes: None,
                total_read_bytes: None,
                power_cycles: None,
                available_spare: None,
                critical_warning: None,
                power_on_hours: None,
                media_errors: None,
                temperature_celsius: Some(34.0),
                ata: None,
                nvme: None,
            })
        };
        assert!(reader.needs_initial_read(&volume));
        for now in [1000, 1001, 30_000, 60_000, 300_999, 301_000] {
            let _ = reader.read_with(&volume, now, read);
            assert!(!reader.needs_initial_read(&volume));
            assert_eq!(
                reader.cached.as_ref().unwrap().0,
                if now < 301_000 { 1000 } else { 301_000 }
            );
            assert!(reader.cached.as_ref().unwrap().1.is_ok());
        }
        assert_eq!(calls.get(), 2);
        let replay = reader.observation(&volume, 900_000, false);
        assert_eq!(
            replay.0, 301_000,
            "hidden replay must retain the original timestamp without a new native query"
        );
        assert!(replay.1.is_ok());
        let mut changed = volume.clone();
        changed.mount_point = "/reconnected".into();
        assert!(reader.needs_initial_read(&changed));
        reader.clear();
        assert!(reader.needs_initial_read(&volume));
    }
}
