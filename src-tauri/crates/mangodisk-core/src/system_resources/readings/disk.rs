use super::ResourceCache;
use crate::system_resources::metrics::{MetricReading, MetricStatus};
use mangodisk_platform::system_resources::disk::{ResourceVolume, VolumeCapacity};
use std::sync::Arc;

impl ResourceCache {
    pub fn volumes(&mut self, volumes: Vec<ResourceVolume>) {
        let temperature_source_invalid =
            self.readings
                .disk_temperature
                .value
                .as_ref()
                .is_some_and(|reading| {
                    let next = volumes.iter().find(|volume| volume.id == reading.volume_id);
                    let previous = self
                        .readings
                        .volumes
                        .iter()
                        .find(|volume| volume.id == reading.volume_id);
                    next.is_none_or(|next| {
                        previous.is_some_and(|previous| !same_source(previous, next))
                    })
                });
        if temperature_source_invalid {
            self.clear_disk_temperature();
        }
        if let Some(previous) = self.readings.disk.value.as_ref() {
            if let Some(next) = volumes
                .iter()
                .find(|volume| volume.id == previous.volume.id)
            {
                self.invalidate_disk_source(next);
            }
        }
        self.readings.volumes = volumes.into();
    }

    pub fn volume_catalogue(&self) -> Arc<[ResourceVolume]> {
        Arc::clone(&self.readings.volumes)
    }

    pub fn disk(&mut self, volume: ResourceVolume, capacity: VolumeCapacity, timestamp_ms: u64) {
        self.invalidate_disk_source(&volume);
        if let Some(value) = crate::system_resources::disk::usage(volume, capacity) {
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
    pub fn reset_disk_selection(&mut self) {
        self.readings.disk = MetricReading::default();
        self.readings.disk_health = MetricReading::default();
        self.readings.disk_detail_temperature = MetricReading::default();
    }

    fn invalidate_disk_source(&mut self, next: &ResourceVolume) {
        let Some(previous) = self.readings.disk.value.as_ref() else {
            return;
        };
        let same_volume = previous.volume.id == next.id;
        let replaced = same_volume && !same_source(&previous.volume, next);
        if !same_volume || replaced {
            self.readings.disk_health = MetricReading::default();
            self.readings.disk_detail_temperature = MetricReading::default();
        }
        if replaced
            && self
                .readings
                .disk_temperature
                .value
                .as_ref()
                .is_some_and(|reading| reading.volume_id == next.id)
        {
            self.clear_disk_temperature();
        }
    }

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

fn device_ids(volume: &ResourceVolume) -> Vec<&str> {
    let mut ids: Vec<_> = volume.devices.iter().map(|disk| disk.id.as_str()).collect();
    ids.sort();
    ids
}

fn same_source(previous: &ResourceVolume, next: &ResourceVolume) -> bool {
    previous.id == next.id
        && previous.mount_point == next.mount_point
        && device_ids(previous) == device_ids(next)
}
