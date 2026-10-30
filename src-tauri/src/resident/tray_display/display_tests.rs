use super::*;

#[test]
fn linux_both_mode_keeps_tray_readings_and_floating_only_hides_them() {
    use crate::resident::preference_schema::LinuxDisplayMode;
    let mut cpu = display_entry(DisplayId::Cpu, "CPU 27%");
    cpu.marker = "CPU".into();
    cpu.digits = "27".into();
    let entries = [cpu];
    for (compact, expected) in [(false, "CPU 27%"), (true, "CPU27%")] {
        let mut preferences = ResidentPreferences {
            taskbar_compact: compact,
            ..Default::default()
        };
        for mode in [LinuxDisplayMode::Tray, LinuxDisplayMode::Both] {
            preferences.linux_display_mode = mode;
            assert_eq!(
                indicator_title(&entries, &preferences).as_deref(),
                Some(expected)
            );
            assert!(indicator_title(&[], &preferences).is_none());
        }
        preferences.linux_display_mode = LinuxDisplayMode::Floating;
        assert!(indicator_title(&entries, &preferences).is_none());
    }
}

#[test]
fn disk_temperature_formats_fresh_samples_without_usage_or_history() {
    use mangodisk_core::system_resources::metrics::MetricReading;
    use mangodisk_platform::system_resources::disk_temperature::{
        DiskTemperature, DiskTemperatureSource,
    };
    let labels = super::super::labels::Labels::for_locale("en-US");
    let mut readings = ResourceReadings {
        observed_at_ms: 1000,
        volumes: vec![mangodisk_platform::system_resources::disk::ResourceVolume {
            devices: Vec::new(),
            mount_points: Vec::new(),
            file_system: None,
            id: "disk".into(),
            name: "System".into(),
            system: true,
            mount_point: String::new(),
        }]
        .into(),
        disk_temperature: MetricReading::ready(
            DiskTemperature {
                volume_id: "disk".into(),
                volume_name: "System".into(),
                celsius: 34.5,
                source: DiskTemperatureSource::NvmeSmart,
            },
            1000,
        ),
        ..Default::default()
    };
    let entry = disk_temperature_entry(&readings, &labels, None, None);
    assert_eq!(entry.digits, "35");
    assert_eq!(entry.text, "DISK 35°C");
    assert_eq!(entry.tooltip, "Disk temperature 35°C · System");
    assert_eq!(
        disk_temperature_entry(&readings, &labels, Some("another-disk"), None).digits,
        "—"
    );
    assert_eq!(
        disk_temperature_entry(&readings, &labels, Some("disk"), None).digits,
        "35"
    );
    readings.observed_at_ms = 91_001;
    assert_eq!(
        disk_temperature_entry(&readings, &labels, None, None).digits,
        "—"
    );
    readings.observed_at_ms = 999;
    assert_eq!(
        disk_temperature_entry(&readings, &labels, None, None).digits,
        "—"
    );
    readings.observed_at_ms = 1000;
    for status in [
        MetricStatus::Loading,
        MetricStatus::Failed,
        MetricStatus::Disconnected,
        MetricStatus::Unsupported,
    ] {
        readings.disk_temperature.status = status;
        assert_eq!(
            disk_temperature_entry(&readings, &labels, None, None).digits,
            "—"
        );
    }
}

#[test]
#[cfg(any(target_os = "macos", windows))]
fn physical_absence_keeps_selected_battery_placeholders_without_changing_preferences() {
    let mut preferences = ResidentPreferences::default();
    for metric in &mut preferences.metrics {
        metric.enabled = matches!(
            metric.id,
            DisplayMetricId::BatteryTemperature
                | DisplayMetricId::ChargingPower
                | DisplayMetricId::BatteryPower
                | DisplayMetricId::SystemPower
        );
    }
    let before = preferences.clone();
    let mut readings = ResourceReadings {
        battery_present: Some(false),
        ..Default::default()
    };
    let labels = super::super::labels::Labels::for_locale("en-US");
    let visible = entries(&preferences, &readings, &labels, 1000.0);
    assert_eq!(visible.len(), 4);
    for entry in visible
        .iter()
        .filter(|entry| entry.id != DisplayId::SystemPower)
    {
        assert_eq!(entry.digits, "—");
        assert!(entry.tooltip.contains("This device has no battery"));
    }
    for present in [None, Some(true)] {
        readings.battery_present = present;
        readings.battery_temperature.status = MetricStatus::Unsupported;
        assert_eq!(entries(&preferences, &readings, &labels, 1000.0).len(), 4);
    }
    assert_eq!(preferences, before);
}
#[test]
fn power_precision_preserves_sub_watt_ranges_and_rounds_once() {
    use mangodisk_core::system_resources::metrics::MetricReading;
    use mangodisk_platform::system_resources::battery::{
        BatteryPower, BatteryPowerState, BatterySource, ChargingPower,
    };
    let labels = super::super::labels::Labels::for_locale("en-US");
    for (watts, expected) in [
        (0.0, "0"),
        (0.01, "<1"),
        (0.99, "<1"),
        (1.0, "1"),
        (19.8, "20"),
        (99.94, "100"),
        (134.49, "134"),
        (134.5, "135"),
        (99.96, "100"),
        (100.0, "100"),
        (135.46, "135"),
        (135.5, "136"),
        (300.0, "300"),
    ] {
        let values = ResourceReadings {
            observed_at_ms: 1000,
            system_power: MetricReading::ready(
                mangodisk_platform::system_resources::power::SystemPower { watts },
                1000,
            ),
            charging_power: MetricReading::ready(
                ChargingPower {
                    watts,
                    charging: true,
                    external_connected: true,
                    source: BatterySource::AppleSmartBattery,
                },
                1000,
            ),
            battery_power: MetricReading::ready(
                BatteryPower {
                    watts,
                    state: BatteryPowerState::Charging,
                    source: BatterySource::AppleSmartBattery,
                },
                1000,
            ),
            ..Default::default()
        };
        for id in [
            DisplayMetricId::ChargingPower,
            DisplayMetricId::BatteryPower,
        ] {
            let entry = battery_entry(&values, &labels, id);
            assert_eq!(entry.digits, expected, "{id:?} at {watts} W");
            assert!(entry.tooltip.contains(&format!("{watts:.1}W")));
            assert_eq!(entry.color_value, Some(displayed_power(watts) as u16));
        }
        let entry = system_power_entry(&values, &labels);
        assert_eq!(entry.digits, expected, "system power at {watts} W");
        assert!(entry.tooltip.contains(&format!("{watts:.1}W")));
    }
}

#[test]
fn battery_power_preserves_direction_and_clears_it_when_stale() {
    use mangodisk_core::system_resources::metrics::MetricReading;
    use mangodisk_platform::system_resources::battery::{
        BatteryPower, BatteryPowerState, BatterySource,
    };
    for (state, watts, heading) in [
        (BatteryPowerState::Charging, 75.0, "BAT"),
        (BatteryPowerState::Discharging, 24.0, "BAT"),
        (BatteryPowerState::Idle, 0.0, "IDLE"),
    ] {
        let mut values = ResourceReadings {
            observed_at_ms: 1000,
            battery_power: MetricReading::ready(
                BatteryPower {
                    watts,
                    state,
                    source: BatterySource::AppleSmartBattery,
                },
                1000,
            ),
            ..Default::default()
        };
        let labels = super::super::labels::Labels::for_locale("en-US");
        let entry = battery_entry(&values, &labels, DisplayMetricId::BatteryPower);
        assert_eq!(entry.power_state, Some(state));
        let column = &super::super::macos_presentation::columns(&[entry], true)[0];
        assert_eq!(column.top, heading);
        assert_eq!(column.power_state, Some(state));
        values.observed_at_ms = 6001;
        let expired = battery_entry(&values, &labels, DisplayMetricId::BatteryPower);
        assert_eq!(expired.digits, "—");
        assert_eq!(expired.power_state, None);
    }
}

#[test]
fn battery_temperature_rounding_matches_the_other_temperature_items() {
    use mangodisk_core::system_resources::metrics::MetricReading;
    use mangodisk_platform::system_resources::battery::{BatterySource, BatteryTemperature};
    let values = ResourceReadings {
        observed_at_ms: 1000,
        battery_temperature: MetricReading::ready(
            BatteryTemperature {
                celsius: 48.5,
                source: BatterySource::AppleSmc,
            },
            1000,
        ),
        ..Default::default()
    };
    let entry = battery_entry(
        &values,
        &super::super::labels::Labels::for_locale("en-US"),
        DisplayMetricId::BatteryTemperature,
    );
    assert_eq!(entry.digits, "49");
    assert_eq!(entry.color_value, Some(49));
}
#[test]
fn charging_color_thresholds_match_the_displayed_integer_and_expire() {
    use mangodisk_core::system_resources::metrics::MetricReading;
    use mangodisk_platform::system_resources::battery::{BatterySource, ChargingPower};
    let mut values = ResourceReadings {
        observed_at_ms: 1000,
        charging_power: MetricReading::ready(
            ChargingPower {
                watts: 49.99,
                charging: true,
                external_connected: true,
                source: BatterySource::AppleSmc,
            },
            1000,
        ),
        ..Default::default()
    };
    let labels = super::super::labels::Labels::for_locale("en-US");
    let entry = battery_entry(&values, &labels, DisplayMetricId::ChargingPower);
    assert_eq!(entry.digits, "50");
    assert_eq!(entry.color_value, Some(50));
    values.observed_at_ms = 6001;
    assert_eq!(
        battery_entry(&values, &labels, DisplayMetricId::ChargingPower).digits,
        "—"
    );
    values.observed_at_ms = 999;
    assert_eq!(
        battery_entry(&values, &labels, DisplayMetricId::ChargingPower).color_value,
        None
    );
}

#[test]
fn gpu_temperature_preserves_celsius_and_rounded_color_value() {
    use mangodisk_core::system_resources::metrics::MetricReading;
    use mangodisk_platform::system_resources::gpu::temperature::{
        GpuTemperature, GpuTemperatureKind, GpuTemperatureSource,
    };
    let labels = super::super::labels::Labels::for_locale("en-US");
    let mut readings = ResourceReadings {
        observed_at_ms: 1000,
        gpu_temperature: MetricReading::ready(
            GpuTemperature {
                adapter_id: "a".into(),
                adapter_name: "Apple M3 Max".into(),
                celsius: 48.5,
                kind: GpuTemperatureKind::SensorAverage,
                source: GpuTemperatureSource::AppleSmc,
                sensor_count: 8,
            },
            1000,
        ),
        ..Default::default()
    };
    let mut prefs = ResidentPreferences {
        show_icon: false,
        ..Default::default()
    };
    for metric in &mut prefs.metrics {
        metric.enabled = metric.id == DisplayMetricId::GpuTemperature;
    }
    let pending = gpu_temperature_entry(&readings, &labels, Some("different-device"));
    assert_eq!(pending.digits, "—");
    assert!(pending.color_value.is_none());
    assert!(pending
        .tooltip
        .contains(labels.status(MetricStatus::Loading)));
    let entry = gpu_temperature_entry(&readings, &labels, None);
    assert_eq!(entry.text, "GPU 49°C");
    assert_eq!(entry.color_value, Some(49));
    assert!(entry.tooltip.contains("Apple M3 Max"));
    assert_eq!(entries(&prefs, &readings, &labels, 1000.0).len(), 1);
    readings.observed_at_ms = 11001;
    assert_eq!(gpu_temperature_entry(&readings, &labels, None).digits, "—");
    readings.observed_at_ms = 999;
    assert_eq!(gpu_temperature_entry(&readings, &labels, None).digits, "—");
}

#[test]
fn temperature_rounding_matches_panel_at_half_degree_boundaries() {
    use mangodisk_core::system_resources::metrics::MetricReading;
    use mangodisk_platform::system_resources::cpu::temperature::{
        CpuTemperature, CpuTemperatureKind, CpuTemperatureSource,
    };
    let labels = super::super::labels::Labels::for_locale("en-US");
    for (celsius, expected) in [(48.5, "49"), (49.5, "50"), (50.5, "51"), (148.5, "149")] {
        let readings = ResourceReadings {
            observed_at_ms: 1_000,
            cpu_temperature: MetricReading::ready(
                CpuTemperature {
                    celsius,
                    kind: CpuTemperatureKind::CoreAverage,
                    source: CpuTemperatureSource::AppleSmc,
                    sensor_count: 16,
                    sampled_at_ms: None,
                },
                1_000,
            ),
            ..Default::default()
        };
        let entry = temperature_entry(&readings, &labels);
        assert_eq!(entry.digits, expected, "celsius={celsius}");
        assert!(entry.tooltip.contains(&format!("{expected}°C")));
    }
}

#[test]
fn temperature_formatting_rejects_expired_invalid_and_future_samples() {
    use mangodisk_core::system_resources::metrics::MetricReading;
    use mangodisk_platform::system_resources::cpu::temperature::{
        CpuTemperature, CpuTemperatureKind, CpuTemperatureSource,
    };
    let mut readings = ResourceReadings {
        observed_at_ms: 5_000,
        cpu_temperature: MetricReading::ready(
            CpuTemperature {
                celsius: 49.4,
                kind: CpuTemperatureKind::CoreAverage,
                source: CpuTemperatureSource::AppleSmc,
                sensor_count: 16,
                sampled_at_ms: None,
            },
            1_000,
        ),
        ..Default::default()
    };
    let labels = super::super::labels::Labels::for_locale("en-US");
    let ready = temperature_entry(&readings, &labels);
    assert_eq!(ready.digits, "49");
    assert_eq!(ready.tooltip, "CPU temperature 49°C");
    assert_eq!(ready.color_value, Some(49));
    assert_eq!(ready.tone, Default::default());
    for status in [
        MetricStatus::Stale,
        MetricStatus::Failed,
        MetricStatus::Loading,
        MetricStatus::Unsupported,
    ] {
        readings.cpu_temperature.status = status;
        assert_eq!(temperature_entry(&readings, &labels).digits, "—");
    }
    readings.cpu_temperature.status = MetricStatus::Ready;
    for timestamp in [0, 999, 11_001] {
        readings.observed_at_ms = timestamp;
        assert_eq!(temperature_entry(&readings, &labels).digits, "—");
    }
    readings.observed_at_ms = 11_000;
    assert_eq!(temperature_entry(&readings, &labels).digits, "49");
    for celsius in [f64::NAN, f64::INFINITY, 0.0, -1.0, 151.0] {
        readings.cpu_temperature.value.as_mut().unwrap().celsius = celsius;
        assert_eq!(temperature_entry(&readings, &labels).digits, "—");
    }
    readings.cpu_temperature.value.as_mut().unwrap().celsius = 49.0;
    readings
        .cpu_temperature
        .value
        .as_mut()
        .unwrap()
        .sensor_count = 0;
    assert_eq!(temperature_entry(&readings, &labels).digits, "—");
}

#[test]
fn temperature_selection_preserves_windows_consent_and_independence_from_cpu() {
    let mut prefs = ResidentPreferences {
        show_icon: false,
        ..Default::default()
    };
    for metric in &mut prefs.metrics {
        metric.enabled = metric.id == DisplayMetricId::CpuTemperature;
    }
    prefs.metrics.reverse();
    let ids = desired(&prefs);
    if !cfg!(windows) {
        assert_eq!(ids, [DisplayId::CpuTemperature]);
        let display = entries(
            &prefs,
            &ResourceReadings::default(),
            &super::super::labels::Labels::for_locale("zh-CN"),
            1024.0,
        );
        assert_eq!(display.len(), 1);
        assert_eq!(display[0].id.metric(), Some(MetricId::Cpu));
        assert!(display[0]
            .tooltip
            .starts_with(super::super::labels::Labels::for_locale("zh-CN").temperature("label")));
    } else {
        assert_eq!(ids, [DisplayId::App]);
        assert!(entries(
            &prefs,
            &ResourceReadings::default(),
            &super::super::labels::Labels::for_locale("en-US"),
            1024.0
        )
        .is_empty());
    }
}

#[cfg(target_os = "macos")]
#[test]
fn temperature_entries_follow_saved_order_including_without_cpu_usage() {
    let labels = super::super::labels::Labels::for_locale("en-US");
    let mut prefs = ResidentPreferences::default();
    for metric in &mut prefs.metrics {
        metric.enabled = matches!(
            metric.id,
            DisplayMetricId::CpuTemperature | DisplayMetricId::Memory | DisplayMetricId::Cpu
        );
    }
    prefs.metrics.reverse();
    let ids = entries(&prefs, &ResourceReadings::default(), &labels, 1024.0)
        .into_iter()
        .map(|entry| entry.id)
        .collect::<Vec<_>>();
    assert_eq!(
        ids,
        [DisplayId::Memory, DisplayId::CpuTemperature, DisplayId::Cpu]
    );
    prefs
        .metrics
        .iter_mut()
        .find(|metric| metric.id == DisplayMetricId::Cpu)
        .unwrap()
        .enabled = false;
    let ids = entries(&prefs, &ResourceReadings::default(), &labels, 1024.0)
        .into_iter()
        .map(|entry| entry.id)
        .collect::<Vec<_>>();
    assert_eq!(ids, [DisplayId::Memory, DisplayId::CpuTemperature]);
}

fn display_entry(id: DisplayId, text: &str) -> DisplayEntry {
    DisplayEntry {
        frequency: None,
        gpu_device: None,
        id,
        tone: Default::default(),
        power_state: None,
        fan_rows: None,
        color_value: None,
        marker: String::new(),
        digits: String::new(),
        text: text.to_string(),
        tooltip: String::new(),
    }
}

#[test]
fn indicator_title_contains_only_non_empty_enabled_metrics() {
    let preferences = ResidentPreferences::default();
    assert_eq!(indicator_title(&[], &preferences), None);
    assert_eq!(
        indicator_title(
            &[
                display_entry(DisplayId::Cpu, "CPU  12%"),
                display_entry(DisplayId::Memory, "MEM  48%"),
                display_entry(DisplayId::Disk, ""),
            ],
            &preferences,
        ),
        Some("CPU  12%  MEM  48%".to_string())
    );
}

#[test]
fn compact_indicator_title_reduces_width_and_keeps_values() {
    let mut cpu = display_entry(DisplayId::Cpu, "CPU  12%");
    cpu.marker = "C".into();
    cpu.digits = "12".into();
    let mut memory = display_entry(DisplayId::Memory, "MEM  48%");
    memory.marker = "M".into();
    memory.digits = "48".into();
    let mut upload = display_entry(DisplayId::Upload, "↑ 82.0 KB/s");
    upload.marker = "↑K".into();
    upload.digits = "82".into();

    let mut preferences = ResidentPreferences::default();
    let standard =
        indicator_title(&[cpu.clone(), memory.clone(), upload.clone()], &preferences).unwrap();
    preferences.taskbar_compact = true;
    let compact = indicator_title(&[cpu, memory, upload], &preferences).unwrap();

    assert_eq!(compact, "C12% M48% ↑82K");
    assert!(compact.len() < standard.len());
}

#[test]
fn all_display_combinations_have_unique_entries_and_paired_network_directions() {
    for bits in 0..(1 << ResidentPreferences::default().metrics.len()) {
        for show_icon in [false, true] {
            let mut prefs = ResidentPreferences {
                show_icon,
                ..Default::default()
            };
            for (index, metric) in prefs.metrics.iter_mut().enumerate() {
                metric.enabled = bits & (1 << index) != 0;
            }
            let ids = desired(&prefs);
            assert_eq!(
                ids.iter().collect::<std::collections::HashSet<_>>().len(),
                ids.len()
            );
            assert!(!ids.is_empty());
            assert!(ids.len() <= DisplayId::ALL.len());
            assert_eq!(
                ids.contains(&DisplayId::Upload),
                prefs.shows(MetricId::Network)
            );
            assert_eq!(
                ids.contains(&DisplayId::Download),
                prefs.shows(MetricId::Network)
            );
            assert_eq!(ids.contains(&DisplayId::App), prefs.effective_icon());
            prefs.enabled = false;
            assert!(desired(&prefs).is_empty());
        }
    }
}

#[test]
fn disk_tooltips_use_shared_capacity_precision_without_explanatory_text() {
    use mangodisk_core::system_resources::{disk, metrics::MetricReading};
    use mangodisk_platform::system_resources::disk::{ResourceVolume, VolumeCapacity};
    let value = disk::usage(
        ResourceVolume {
            devices: Vec::new(),
            mount_points: Vec::new(),
            file_system: None,
            id: "test".into(),
            name: "Test".into(),
            system: true,
            mount_point: "/".into(),
        },
        VolumeCapacity {
            total_bytes: 100_000_000_000,
            available_bytes: 40_000_000_000,
        },
    )
    .unwrap();
    let readings = ResourceReadings {
        disk: MetricReading::ready(value, 0),
        ..Default::default()
    };
    let mut preferences = ResidentPreferences::default();
    for metric in &mut preferences.metrics {
        metric.enabled = metric.id == DisplayMetricId::Disk;
    }
    for locale in [
        "en-US", "zh-CN", "zh-TW", "ja-JP", "ko-KR", "tr-TR", "pt-BR",
    ] {
        let labels = super::super::labels::Labels::for_locale(locale);
        let entries = entries(&preferences, &readings, &labels, 1000.0);
        let disk = entries
            .iter()
            .find(|entry| entry.id == DisplayId::Disk)
            .unwrap();
        assert_eq!(disk.color_value, Some(60));
        assert!(disk.tooltip.contains("60.00 GB / 100.00 GB"));
        assert!(!disk.tooltip.contains(labels.text("diskCapacityHint")));
        assert!(!disk.tooltip.contains('\n'));
    }
}

#[test]
fn taskbar_fallback_restores_all_metrics_and_never_loses_the_last_entry() {
    for bits in 0..(1 << ResidentPreferences::default().metrics.len()) {
        for show_icon in [false, true] {
            let mut prefs = ResidentPreferences {
                show_icon,
                ..Default::default()
            };
            for (i, item) in prefs.metrics.iter_mut().enumerate() {
                item.enabled = bits & (1 << i) != 0;
            }
            assert_eq!(windows_desired(&prefs, false), desired(&prefs));
            let active = windows_desired(&prefs, true);
            assert_eq!(active.contains(&DisplayId::App), prefs.effective_icon());
            assert!(active.len() <= 1);
            prefs.enabled = false;
            assert!(windows_desired(&prefs, true).is_empty());
            assert!(windows_desired(&prefs, false).is_empty());
        }
    }
}
#[test]
fn native_ids_are_stable_and_network_contributes_two_entries() {
    let mut prefs = ResidentPreferences::default();
    for metric in &mut prefs.metrics {
        metric.enabled = true;
    }
    assert_eq!(
        desired(&prefs),
        DisplayId::ALL
            .into_iter()
            .filter(|id| !cfg!(windows) || *id != DisplayId::CpuTemperature)
            .collect::<Vec<_>>()
    );
    assert_eq!(DisplayId::Upload.metric(), DisplayId::Download.metric());
    prefs.enabled = false;
    assert!(desired(&prefs).is_empty());
}
#[test]
fn invalid_ready_samples_use_the_unavailable_presentation() {
    use mangodisk_core::system_resources::metrics::{CpuUsage, MetricReading};
    let mut preferences = ResidentPreferences::default();
    for metric in &mut preferences.metrics {
        metric.enabled = metric.id == DisplayMetricId::Cpu;
    }
    let labels = super::super::labels::Labels::for_locale("en-US");
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let readings = ResourceReadings {
            cpu: MetricReading::ready(
                CpuUsage {
                    used_percent: value,
                },
                0,
            ),
            ..Default::default()
        };
        let entries = entries(&preferences, &readings, &labels, 1000.0);
        assert_eq!(entries[0].digits, "—");
        assert!(!entries[0].tooltip.contains("NaN"));
        assert!(!entries[0].tooltip.contains("inf"));
    }
}

#[test]
fn displayed_percentages_and_color_thresholds_use_the_same_rounding() {
    use super::super::usage_color::UsageTone;
    use mangodisk_core::system_resources::metrics::{CpuUsage, MetricReading};
    let preferences = ResidentPreferences::default();
    let labels = super::super::labels::Labels::for_locale("en-US");
    for (value, expected, tone) in [
        (74.49, 74, UsageTone::Normal),
        (74.5, 75, UsageTone::Warning),
        (84.5, 85, UsageTone::Critical),
        (100.5, 100, UsageTone::Critical),
        (-0.5, 0, UsageTone::Normal),
    ] {
        let readings = ResourceReadings {
            cpu: MetricReading::ready(
                CpuUsage {
                    used_percent: value,
                },
                0,
            ),
            ..Default::default()
        };
        let entries = entries(&preferences, &readings, &labels, 1000.0);
        let cpu = entries
            .iter()
            .find(|entry| entry.id == DisplayId::Cpu)
            .unwrap();
        assert_eq!(cpu.digits, expected.to_string());
        assert_eq!(cpu.color_value, Some(expected));
        let mut rule = crate::resident::color_rules::MetricColorRule::defaults(
            crate::resident::preference_schema::DisplayMetricId::Cpu,
        );
        rule.rules = vec![
            crate::resident::color_rules::ColorRule::Below {
                value: 75,
                color: crate::resident::color_rules::DisplayColor::Default,
            },
            crate::resident::color_rules::ColorRule::Range {
                from: 75,
                to: 85,
                color: crate::resident::color_rules::DisplayColor::Orange,
            },
            crate::resident::color_rules::ColorRule::AtLeast {
                value: 85,
                color: crate::resident::color_rules::DisplayColor::Red,
            },
        ];
        let matched = super::super::usage_color::next_rule(None, cpu.color_value, &rule).unwrap();
        assert_eq!(UsageTone::from(rule.rules[matched].color()), tone);
    }
}

#[test]
fn network_unit_boundaries_stay_within_three_digits() {
    let units = ["B", "K", "M", "G", "T"];
    for base in [1000.0_f64, 1024.0] {
        for index in 0..4 {
            let scale = base.powi(index as i32);
            assert_eq!(compact_rate((base - 0.51) * scale, base).1, units[index]);
            assert_eq!(
                compact_rate((base - 0.49) * scale, base),
                ("1".into(), units[index + 1])
            );
        }
        for value in [
            0.0,
            0.49,
            0.51,
            9.49,
            9.51,
            99.49,
            99.51,
            999.49,
            999.51,
            f64::MAX,
        ] {
            assert!(compact_rate(value, base).0.len() <= 3);
        }
    }
    for invalid in [-1.0, f64::NAN, f64::INFINITY] {
        assert_eq!(compact_rate(invalid, 1000.0).0, "—");
    }
}

#[test]
fn compact_rates_keep_units_and_three_digit_bounds() {
    assert_eq!(compact_rate(0.0, 1000.0), ("0".into(), "B"));
    assert_eq!(compact_rate(999_000.0, 1000.0), ("999".into(), "K"));
    assert_eq!(compact_rate(1_000_000.0, 1000.0), ("1".into(), "M"));
    assert_eq!(compact_rate(f64::NAN, 1000.0).0, "—");
    assert_eq!(compact_rate(1024.0, 1024.0), ("1".into(), "K"));
}
#[test]
fn menu_bar_rate_fields_keep_their_width_across_rounding_and_unit_changes() {
    for base in [1000.0, 1024.0] {
        for value in [0.0, 99.9, 999.94, 999.95, 1023.96, 1_000_000.0, f64::MAX] {
            assert_eq!(rate_title(value, base).chars().count(), 10);
        }
    }
    assert_eq!(rate_title(999.95, 1000.0), "  1.0 KB/s");
    assert_eq!(rate_title(0.0, 1000.0), "  0.0  B/s");
}

#[test]
fn all_gpu_displays_keep_device_identity_values_and_unavailable_temperatures_separate() {
    use mangodisk_core::system_resources::{
        gpu::GpuUsage, metrics::MetricReading, readings::GpuDeviceReading,
    };
    let mut preferences = ResidentPreferences {
        gpu_display: crate::resident::preference_schema::GpuDisplaySelection::Selected {
            adapter_ids: vec!["a".into(), "b".into()],
        },
        ..Default::default()
    };
    for metric in &mut preferences.metrics {
        metric.enabled = matches!(
            metric.id,
            DisplayMetricId::Gpu | DisplayMetricId::GpuTemperature
        );
    }
    let mut values = ResourceReadings {
        observed_at_ms: 1000,
        ..Default::default()
    };
    values.gpu_devices = [("a", 90.0), ("b", 10.0)]
        .into_iter()
        .map(|(id, used_percent)| GpuDeviceReading {
            adapter_id: id.into(),
            adapter_name: id.into(),
            usage: MetricReading::ready(
                GpuUsage {
                    adapter_id: id.into(),
                    adapter_name: id.into(),
                    used_percent,
                    details: None,
                },
                1000,
            ),
            usage_history: Default::default(),
            temperature: MetricReading {
                status: MetricStatus::Unsupported,
                sampled_at_ms: None,
                value: None,
            },
            temperature_history: Default::default(),
        })
        .collect();
    let labels = super::super::labels::Labels::for_locale("en-US");
    values.gpu_adapters = values
        .gpu_devices
        .iter()
        .map(
            |device| mangodisk_platform::system_resources::gpu::GpuAdapter {
                id: device.adapter_id.clone(),
                name: device.adapter_name.clone(),
            },
        )
        .collect();
    let entries = entries(&preferences, &values, &labels, 1000.0);
    if !DisplayMetricId::Gpu.supported_on_platform() {
        assert_eq!(entries.len(), 2);
        assert!(entries
            .iter()
            .all(|entry| entry.digits == "—" && entry.gpu_device.is_none()));
        return;
    }
    assert_eq!(
        entries.len(),
        if cfg!(any(target_os = "macos", windows)) {
            4
        } else {
            2
        }
    );
    let notifications = notification_entries(&entries);
    assert_eq!(
        notifications.len(),
        if cfg!(any(target_os = "macos", windows)) {
            2
        } else {
            1
        }
    );
    assert!(notifications[0].tooltip.contains("a"));
    assert!(notifications[0].tooltip.contains("b"));
    assert_eq!(notifications[0].digits, "90");
    assert_eq!(entries[0].digits, "90");
    assert_eq!(entries[1].digits, "10");
    assert_eq!(entries[0].gpu_device.as_ref().unwrap().label, "GPU0");
    assert_eq!(entries[1].gpu_device.as_ref().unwrap().id, "b");
    for entry in entries.iter().skip(2) {
        assert_eq!(entry.digits, "—");
        assert!(entry.color_value.is_none());
    }
    #[cfg(target_os = "macos")]
    {
        let columns = super::super::macos_presentation::columns(&entries, true);
        assert_eq!(columns[0].top, "GPU0");
        assert_eq!(columns[1].top, "GPU1");
        assert_eq!(columns[2].top, "GPU0");
    }
}

#[test]
fn automatic_gpu_display_is_independent_of_detail_selection_and_missing_fixed_ids() {
    use crate::resident::preference_schema::GpuDisplaySelection;
    use mangodisk_core::system_resources::{
        gpu::GpuUsage, metrics::MetricReading, readings::GpuDeviceReading,
    };
    let mut preferences = ResidentPreferences {
        gpu_adapter: Some("b".into()),
        ..Default::default()
    };
    for metric in &mut preferences.metrics {
        metric.enabled = metric.id == DisplayMetricId::Gpu;
    }
    let values = ResourceReadings {
        observed_at_ms: 1000,
        gpu_devices: [("a", 90.0), ("b", 10.0)]
            .into_iter()
            .map(|(id, used_percent)| GpuDeviceReading {
                adapter_id: id.into(),
                adapter_name: id.into(),
                usage: MetricReading::ready(
                    GpuUsage {
                        adapter_id: id.into(),
                        adapter_name: id.into(),
                        used_percent,
                        details: None,
                    },
                    1000,
                ),
                usage_history: Default::default(),
                temperature: Default::default(),
                temperature_history: Default::default(),
            })
            .collect(),
        ..Default::default()
    };
    let labels = super::super::labels::Labels::for_locale("en-US");
    let automatic = entries(&preferences, &values, &labels, 1000.0);
    assert_eq!(automatic.len(), 1);
    if !DisplayMetricId::Gpu.supported_on_platform() {
        assert_eq!(automatic[0].digits, "—");
        assert!(automatic[0].gpu_device.is_none());
        return;
    }
    assert_eq!(automatic[0].gpu_device.as_ref().unwrap().id, "a");
    preferences.gpu_display = GpuDisplaySelection::Selected {
        adapter_ids: vec!["gone".into()],
    };
    let missing = entries(&preferences, &values, &labels, 1000.0);
    assert_eq!(missing.len(), 1);
    assert_eq!(missing[0].digits, "—");
    assert_eq!(missing[0].gpu_device.as_ref().unwrap().id, "gone");
    assert!(missing[0].color_value.is_none());
}

#[test]
fn unsupported_platform_choices_remain_visible_without_an_icon_fallback() {
    let labels = super::super::labels::Labels::for_locale("en-US");
    for id in ResidentPreferences::default()
        .metrics
        .into_iter()
        .map(|metric| metric.id)
        .filter(|id| !id.supported_on_platform())
    {
        let mut preferences = ResidentPreferences {
            show_icon: false,
            ..Default::default()
        };
        for metric in &mut preferences.metrics {
            metric.enabled = metric.id == id;
        }
        assert!(!preferences.effective_icon());
        let visible = entries(&preferences, &ResourceReadings::default(), &labels, 1000.0);
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].digits, "—");
        assert!(visible[0]
            .tooltip
            .ends_with("Not supported on this platform"));
        assert!(desired(&preferences).contains(&visible[0].id));
    }
}

#[test]
fn confirmed_battery_removal_never_reuses_a_fresh_power_value() {
    use mangodisk_core::system_resources::metrics::MetricReading;
    use mangodisk_platform::system_resources::battery::{
        BatteryPower, BatteryPowerState, BatterySource,
    };
    let values = ResourceReadings {
        observed_at_ms: 1000,
        battery_present: Some(false),
        battery_power: MetricReading::ready(
            BatteryPower {
                watts: 12.0,
                state: BatteryPowerState::Discharging,
                source: BatterySource::AppleSmartBattery,
            },
            1000,
        ),
        ..Default::default()
    };
    let entry = battery_entry(
        &values,
        &super::super::labels::Labels::for_locale("en-US"),
        DisplayMetricId::BatteryPower,
    );
    assert_eq!(entry.digits, "—");
    assert_eq!(entry.power_state, None);
    assert_eq!(entry.color_value, None);
    assert!(entry.tooltip.ends_with("This device has no battery"));
}

#[test]
fn floating_only_never_falls_back_to_notification_area_metrics() {
    use crate::resident::preference_schema::WindowsDisplayMode;
    let preferences = ResidentPreferences {
        windows_display_mode: WindowsDisplayMode::Floating,
        ..Default::default()
    };
    assert_eq!(windows_desired(&preferences, false), vec![DisplayId::App]);
    assert_eq!(windows_desired(&preferences, true), vec![DisplayId::App]);
    let disabled = ResidentPreferences {
        enabled: false,
        ..preferences
    };
    assert!(windows_desired(&disabled, false).is_empty());
}
