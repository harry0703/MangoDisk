use super::metrics::MetricStatus;
use super::readings::ResourceCache;
use mangodisk_platform::system_resources::{
    disk::{ResourceVolume, VolumeCapacity},
    disk_health::{DiskHealth, DiskHealthStatus, DISK_HEALTH_FRESHNESS_MS},
    disk_temperature::DiskTemperatureSource,
};

fn volume(id: &str) -> ResourceVolume {
    ResourceVolume {
        devices: Vec::new(),
        mount_points: Vec::new(),
        file_system: None,
        id: id.into(),
        name: "Disk".into(),
        system: true,
        mount_point: "/".into(),
    }
}

#[test]
fn disk_sensor_source_requires_a_confirmed_available_capacity_observation() {
    use super::metrics::MetricId;
    let mut cache = ResourceCache::default();
    assert!(cache.disk_volume().is_none());
    cache.disk(
        volume("one"),
        VolumeCapacity {
            total_bytes: 100,
            available_bytes: 50,
        },
        1000,
    );
    assert_eq!(cache.disk_volume().unwrap().id, "one");
    cache.fail(MetricId::Disk, MetricStatus::Disconnected);
    assert!(cache.disk_volume().is_none());
    cache.reset(MetricId::Disk);
    assert!(cache.disk_volume().is_none());
}
fn health(id: &str) -> DiskHealth {
    DiskHealth {
        volume_id: id.into(),
        volume_name: "Disk".into(),
        status: DiskHealthStatus::Healthy,
        source: DiskTemperatureSource::NvmeSmart,
        total_read_bytes: None,
        power_cycles: None,
        available_spare: None,
        critical_warning: None,
        percentage_used: Some(0),
        total_written_bytes: Some("123456789012345678901234".into()),
        power_on_hours: Some(0),
        media_errors: Some("0".into()),
        temperature_celsius: None,
        ata: None,
        nvme: None,
    }
}

#[test]
fn capacity_failures_preserve_independent_sensor_identity_and_removal_invalidates_details() {
    use mangodisk_platform::system_resources::disk_temperature::DiskTemperature;
    let mut cache = ResourceCache::default();
    cache.disk(
        volume("selected"),
        VolumeCapacity {
            total_bytes: 100,
            available_bytes: 50,
        },
        1000,
    );
    cache.disk_health(Ok(health("selected")), 1000);
    cache.disk_temperature(
        Ok(DiskTemperature {
            volume_id: "other".into(),
            volume_name: "Other disk".into(),
            celsius: 35.0,
            source: DiskTemperatureSource::NvmeSmart,
        }),
        1000,
    );
    cache.fail_disk_capacity(MetricStatus::Failed);
    let reading = cache.snapshot(1000);
    assert_eq!(reading.disk.status, MetricStatus::Failed);
    assert_eq!(reading.disk_health.status, MetricStatus::Ready);
    assert_eq!(reading.disk_temperature.status, MetricStatus::Ready);
    cache.fail_disk_capacity(MetricStatus::Disconnected);
    let reading = cache.snapshot(1000);
    assert_eq!(reading.disk_health.status, MetricStatus::Disconnected);
    assert!(reading.disk_health.value.is_none());
    assert_eq!(reading.disk_temperature.value.unwrap().volume_id, "other");
    assert_eq!(reading.disk_temperature.status, MetricStatus::Ready);
}
#[test]
fn health_has_independent_freshness_and_rejects_other_devices_or_failed_observations() {
    let mut cache = ResourceCache::default();
    let capacity = VolumeCapacity {
        total_bytes: 100,
        available_bytes: 50,
    };
    cache.disk(volume("one"), capacity, 1000);
    cache.disk_health(Ok(health("one")), 1000);
    let wire = serde_json::to_value(cache.snapshot(1000)).unwrap();
    assert_eq!(wire["schemaVersion"], 39);
    assert_eq!(
        wire["diskHealth"]["value"]["totalWrittenBytes"],
        "123456789012345678901234"
    );
    assert_eq!(
        cache
            .snapshot(1000 + DISK_HEALTH_FRESHNESS_MS)
            .disk_health
            .status,
        MetricStatus::Ready
    );
    assert_eq!(
        cache
            .snapshot(1001 + DISK_HEALTH_FRESHNESS_MS)
            .disk_health
            .status,
        MetricStatus::Stale
    );
    cache.disk_health(Ok(health("other")), 2000);
    assert!(cache.snapshot(2000).disk_health.value.is_none());
    cache.disk_health(Ok(health("one")), 3000);
    cache.disk(volume("other"), capacity, 3001);
    assert!(cache.snapshot(3001).disk_health.value.is_none());
    cache.disk_health(Ok(health("other")), 4000);
    cache.fail(super::metrics::MetricId::Disk, MetricStatus::Disconnected);
    assert_eq!(
        cache.snapshot(4001).disk_health.status,
        MetricStatus::Disconnected
    );
    assert!(cache.snapshot(4001).disk_health.value.is_none());
}

#[test]
fn display_temperature_and_detail_temperature_remain_independent_and_expire_separately() {
    use mangodisk_platform::system_resources::disk_temperature::DiskTemperature;
    let mut cache = ResourceCache::default();
    let sample = |id: &str, celsius| DiskTemperature {
        volume_id: id.into(),
        volume_name: id.into(),
        celsius,
        source: DiskTemperatureSource::NvmeSmart,
    };
    cache.disk_temperature(Ok(sample("display", 40.0)), 1000);
    cache.disk_detail_temperature(Ok(sample("detail", 35.0)), 2000);
    let snapshot = cache.snapshot(2000);
    assert_eq!(
        snapshot.disk_temperature.value.unwrap().volume_id,
        "display"
    );
    assert_eq!(
        snapshot.disk_detail_temperature.value.unwrap().volume_id,
        "detail"
    );
    cache.clear_disk_temperature();
    assert_eq!(
        cache.snapshot(2000).disk_detail_temperature.status,
        MetricStatus::Ready
    );
    assert!(cache.expire(92_001));
    assert_eq!(
        cache.snapshot(92_001).disk_detail_temperature.status,
        MetricStatus::Stale
    );
}

#[test]
fn health_cache_survives_transient_failures_without_changing_observation_time() {
    use mangodisk_platform::{PlatformError, PlatformErrorCode};
    let mut cache = ResourceCache::default();
    cache.disk(
        volume("one"),
        VolumeCapacity {
            total_bytes: 100,
            available_bytes: 50,
        },
        1000,
    );
    cache.disk_health(Ok(health("one")), 1000);
    cache.fail(super::metrics::MetricId::Disk, MetricStatus::Stale);
    let expired = cache.snapshot(1500);
    assert_eq!(expired.disk_health.status, MetricStatus::Stale);
    assert_eq!(expired.disk_health.sampled_at_ms, Some(1000));
    assert!(expired.disk_health.value.is_some());
    cache.disk_health(
        Err(PlatformError::new(
            PlatformErrorCode::Io,
            "temporary failure",
        )),
        2000,
    );
    let snapshot = cache.snapshot(2000);
    assert_eq!(snapshot.disk_health.status, MetricStatus::Failed);
    assert_eq!(snapshot.disk_health.sampled_at_ms, Some(1000));
    assert_eq!(snapshot.disk_health.value.unwrap().volume_id, "one");
    cache.fail(super::metrics::MetricId::Disk, MetricStatus::Failed);
    assert!(cache.snapshot(3000).disk_health.value.is_some());
    cache.disk_health(Err(PlatformError::item_changed("removed")), 4000);
    assert!(cache.snapshot(4000).disk_health.value.is_none());
}

#[test]
fn changing_physical_devices_invalidates_health_even_with_the_same_volume_identity() {
    use mangodisk_platform::system_resources::disk::ResourceDisk;
    let mut previous = volume("same-volume");
    previous.devices = vec![ResourceDisk {
        id: "old-device".into(),
        number: Some(0),
        model: None,
    }];
    let mut cache = ResourceCache::default();
    cache.volumes(vec![previous.clone()]);
    cache.disk(
        previous.clone(),
        VolumeCapacity {
            total_bytes: 100,
            available_bytes: 50,
        },
        1000,
    );
    cache.disk_health(Ok(health("same-volume")), 1000);
    let mut replacement = previous;
    replacement.devices[0].id = "new-device".into();
    cache.volumes(vec![replacement.clone()]);
    cache.disk(
        replacement,
        VolumeCapacity {
            total_bytes: 100,
            available_bytes: 50,
        },
        2000,
    );
    assert!(
        cache.snapshot(2000).disk_health.value.is_none(),
        "a cloned volume ID must not restore another physical disk's health"
    );
}

#[test]
fn catalogue_removal_or_replacement_invalidates_an_independent_temperature_source() {
    use mangodisk_platform::system_resources::{
        disk::ResourceDisk, disk_temperature::DiskTemperature,
    };
    for removed in [false, true] {
        let mut cache = ResourceCache::default();
        let selected = volume("selected");
        let mut fixed = volume("fixed");
        fixed.devices = vec![ResourceDisk {
            id: "physical-one".into(),
            number: None,
            model: None,
        }];
        cache.volumes(vec![selected.clone(), fixed.clone()]);
        cache.disk(
            selected.clone(),
            VolumeCapacity {
                total_bytes: 100,
                available_bytes: 50,
            },
            1000,
        );
        cache.disk_temperature(
            Ok(DiskTemperature {
                volume_id: fixed.id.clone(),
                volume_name: fixed.name.clone(),
                celsius: 35.0,
                source: DiskTemperatureSource::NvmeSmart,
            }),
            1000,
        );
        fixed.devices[0].id = "physical-replacement".into();
        cache.volumes(if removed {
            vec![selected]
        } else {
            vec![selected, fixed]
        });
        let snapshot = cache.snapshot(2000);
        assert!(
            snapshot.disk_temperature.value.is_none(),
            "a fixed temperature cannot survive its own catalogue removal or physical replacement"
        );
        assert_eq!(snapshot.disk.status, MetricStatus::Ready);
        assert_eq!(snapshot.disk.sampled_at_ms, Some(1000));
    }
}
