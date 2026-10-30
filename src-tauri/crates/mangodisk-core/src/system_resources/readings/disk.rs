use super::ResourceCache;
use crate::system_resources::{
    disk,
    metrics::{MetricReading, MetricStatus},
};
use mangodisk_platform::system_resources::disk::{ResourceVolume, VolumeCapacity};

impl ResourceCache {
    pub fn volumes(&mut self, volumes: Vec<ResourceVolume>) {
        self.readings.volumes = volumes;
    }

    pub fn disk(&mut self, volume: ResourceVolume, capacity: VolumeCapacity, timestamp_ms: u64) {
        if let Some(value) = disk::usage(volume, capacity) {
            if self
                .readings
                .disk_health
                .value
                .as_ref()
                .is_some_and(|health| health.volume_id != value.volume.id)
            {
                self.readings.disk_health = MetricReading::default();
                self.readings.disk_detail_temperature = MetricReading::default();
            }
            self.readings.disk = MetricReading::ready(value, timestamp_ms);
        } else {
            self.fail_disk_capacity(MetricStatus::Failed);
        }
    }

    /// Expose only the confirmed cached source, without cloning histories or process lists.
    pub fn disk_volume(&self) -> Option<&ResourceVolume> {
        matches!(
            self.readings.disk.status,
            MetricStatus::Ready | MetricStatus::Stale
        )
        .then(|| self.readings.disk.value.as_ref().map(|disk| &disk.volume))
        .flatten()
    }

    /// Capacity failures cannot invalidate independently sampled physical sensors.
    pub fn fail_disk_capacity(&mut self, status: MetricStatus) {
        self.readings.disk.status = status;
        if matches!(
            status,
            MetricStatus::Disconnected | MetricStatus::Unsupported
        ) {
            self.readings.disk_health.mark_unavailable(status);
            self.readings
                .disk_detail_temperature
                .mark_unavailable(status);
        }
    }

    pub fn disk_health(
        &mut self,
        result: mangodisk_platform::PlatformResult<
            mangodisk_platform::system_resources::disk_health::DiskHealth,
        >,
        timestamp_ms: u64,
    ) {
        match result {
            Ok(value)
                if !value.volume_id.is_empty()
                    && self
                        .readings
                        .disk
                        .value
                        .as_ref()
                        .is_some_and(|disk| disk.volume.id == value.volume_id) =>
            {
                self.readings.disk_health = MetricReading::ready(value, timestamp_ms)
            }
            result => {
                // An unrelated completion cannot be reused as this disk's observation.
                let status = match result {
                    Err(error)
                        if error.code() == mangodisk_platform::PlatformErrorCode::Unsupported =>
                    {
                        MetricStatus::Unsupported
                    }
                    Err(error)
                        if error.code() == mangodisk_platform::PlatformErrorCode::ItemChanged =>
                    {
                        MetricStatus::Disconnected
                    }
                    Err(_) => MetricStatus::Failed,
                    Ok(_) => MetricStatus::Disconnected,
                };
                self.readings.disk_health.mark_unavailable(status);
            }
        }
    }

    pub fn clear_disk_temperature(&mut self) {
        self.readings.disk_temperature = MetricReading::default();
    }

    pub fn disk_temperature(
        &mut self,
        result: mangodisk_platform::PlatformResult<
            mangodisk_platform::system_resources::disk_temperature::DiskTemperature,
        >,
        timestamp_ms: u64,
    ) {
        self.readings.disk_temperature = Self::temperature_reading(result, timestamp_ms);
    }

    pub fn disk_detail_temperature(
        &mut self,
        result: mangodisk_platform::PlatformResult<
            mangodisk_platform::system_resources::disk_temperature::DiskTemperature,
        >,
        timestamp_ms: u64,
    ) {
        self.readings.disk_detail_temperature = Self::temperature_reading(result, timestamp_ms);
    }

    fn temperature_reading(
        result: mangodisk_platform::PlatformResult<
            mangodisk_platform::system_resources::disk_temperature::DiskTemperature,
        >,
        timestamp_ms: u64,
    ) -> MetricReading<mangodisk_platform::system_resources::disk_temperature::DiskTemperature>
    {
        let unavailable = |status| MetricReading {
            status,
            sampled_at_ms: None,
            value: None,
        };
        match result {
            Ok(value) if value.valid() => MetricReading::ready(value, timestamp_ms),
            Ok(_) => unavailable(MetricStatus::Failed),
            Err(error) => unavailable(
                if error.code() == mangodisk_platform::PlatformErrorCode::Unsupported {
                    MetricStatus::Unsupported
                } else if error.code() == mangodisk_platform::PlatformErrorCode::ItemChanged {
                    MetricStatus::Disconnected
                } else {
                    MetricStatus::Failed
                },
            ),
        }
    }
}
