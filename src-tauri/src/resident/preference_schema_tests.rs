use super::*;

#[test]
fn current_rules_round_trip_empty_single_and_sparse_conditions() {
    let mut preferences = ResidentPreferences::default();
    preferences.color_rules[0].rules.clear();
    preferences.color_rules[1].rules.truncate(1);
    preferences.color_rules[2].rules.remove(1);
    assert_eq!(
        decode(serde_json::to_value(&preferences).unwrap()).unwrap(),
        preferences
    );
    let mut malformed = serde_json::to_value(&preferences).unwrap();
    malformed["colorRules"][1]["rules"][0]["from"] = 1.into();
    assert!(decode(malformed).is_err());
}

#[test]
fn current_rules_reject_unknown_colors_missing_metrics_and_legacy_fields() {
    let current = serde_json::to_value(ResidentPreferences::default()).unwrap();
    assert_eq!(
        decode(current.clone()).unwrap().schema_version,
        PREFERENCE_SCHEMA_VERSION
    );
    let mut value = current.clone();
    value["colorRules"][0]["rules"][0]["color"] = "purple".into();
    assert!(decode(value).is_err());
    let mut value = current.clone();
    value["colorRules"].as_array_mut().unwrap().pop();
    assert!(decode(value).is_err());
    let mut value = current.clone();
    value["usageWarningPercent"] = 70.into();
    assert!(decode(value).is_err());
    let mut value = current;
    value["schemaVersion"] = 14.into();
    value["usageWarningPercent"] = 70.into();
    value["usageCriticalPercent"] = 90.into();
    assert!(decode(value).is_err());
}

#[test]
fn unconsented_windows_temperature_retains_logo_recovery() {
    let mut prefs = ResidentPreferences {
        show_icon: false,
        ..Default::default()
    };
    for metric in &mut prefs.metrics {
        metric.enabled = metric.id == DisplayMetricId::CpuTemperature;
    }
    assert_eq!(prefs.effective_icon(), cfg!(windows));
    prefs
        .metrics
        .iter_mut()
        .for_each(|metric| metric.enabled = false);
    assert!(prefs.effective_icon());
}

#[test]
fn fresh_preferences_show_cpu_memory_disk_network_with_automatic_selection() {
    let preferences = ResidentPreferences::default();
    assert!(preferences.enabled && preferences.show_icon && preferences.taskbar_background);
    assert_eq!(preferences.taskbar_position, TaskbarPosition::Auto);
    assert_eq!(
        preferences
            .metrics
            .iter()
            .filter(|metric| metric.enabled)
            .map(|metric| metric.id)
            .collect::<Vec<_>>(),
        vec![
            DisplayMetricId::Cpu,
            DisplayMetricId::Memory,
            DisplayMetricId::Disk,
            DisplayMetricId::Network,
        ]
    );
    assert!(preferences.network_interface.is_none() && preferences.disk_volume.is_none());
}

#[test]
fn saved_preferences_are_not_replaced_by_new_install_defaults() {
    let mut saved = ResidentPreferences {
        taskbar_position: TaskbarPosition::Right,
        enabled: false,
        show_icon: false,
        taskbar_background: false,
        ..Default::default()
    };
    for metric in &mut saved.metrics {
        metric.enabled = matches!(metric.id, DisplayMetricId::Cpu | DisplayMetricId::Memory);
    }
    assert_eq!(
        decode(serde_json::to_value(&saved).unwrap()).unwrap(),
        saved
    );
}

#[test]
fn every_display_combination_retains_an_entry() {
    for bits in 0..(1 << DISPLAY_ORDER.len()) {
        for show_icon in [false, true] {
            let mut preferences = ResidentPreferences {
                show_icon,
                ..Default::default()
            };
            for (index, metric) in preferences.metrics.iter_mut().enumerate() {
                metric.enabled = bits & (1 << index) != 0;
            }
            assert_eq!(
                preferences.effective_icon(),
                show_icon
                    || !preferences.metrics.iter().any(|m| {
                        m.enabled && (!cfg!(windows) || m.id != DisplayMetricId::CpuTemperature)
                    })
            );
            assert!(
                preferences.effective_icon()
                    || preferences.metrics.iter().any(|metric| metric.enabled)
            );
        }
    }
}

#[test]
fn current_schema_rejects_development_versions_unknown_fields_and_missing_fields() {
    let current = serde_json::to_value(ResidentPreferences::default()).unwrap();
    for version in (0..PREFERENCE_SCHEMA_VERSION)
        .filter(|v| {
            ![
                7, 8, 10, 21, 25, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41,
            ]
            .contains(v)
        })
        .chain([PREFERENCE_SCHEMA_VERSION + 1])
    {
        let mut value = current.clone();
        value["schemaVersion"] = version.into();
        assert_eq!(decode(value).unwrap_err(), "preferences_version");
    }
    for field in [
        "gpuTemperatureAdapter",
        "gpuAllAdapters",
        "gpuTemperatureAllAdapters",
        "fanId",
    ] {
        let mut value = current.clone();
        value[field] = serde_json::Value::Null;
        assert!(decode(value).is_err(), "field={field}");
    }
    for field in [
        "gpuDisplay",
        "gpuTemperatureDisplay",
        "fanIds",
        "metrics",
        "windowsDisplayMode",
        "macosDisplayMode",
        "floating",
    ] {
        let mut value = current.clone();
        value.as_object_mut().unwrap().remove(field);
        assert!(decode(value).is_err(), "field={field}");
    }
    for mode in ["all", "followSelection"] {
        let mut value = current.clone();
        value["gpuDisplay"] = serde_json::json!({"mode":mode});
        assert!(decode(value).is_err());
    }
}

#[test]
fn previous_windows_floating_preferences_gain_independent_macos_menu_bar_default() {
    let expected = ResidentPreferences {
        revision: 187,
        windows_display_mode: WindowsDisplayMode::Both,
        floating: FloatingPreferences {
            layout: FloatingLayout::Vertical,
            font_size: FloatingFontSize::Large,
            bold: true,
            background_opacity: 70,
            click_through: true,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut previous = serde_json::to_value(&expected).unwrap();
    previous["schemaVersion"] = 34.into();
    remove_frequency_items(&mut previous);
    previous.as_object_mut().unwrap().remove("linuxDisplayMode");
    previous["floating"]
        .as_object_mut()
        .unwrap()
        .remove("hideOnFullscreen");
    previous.as_object_mut().unwrap().remove("macosDisplayMode");
    assert_eq!(decode(previous.clone()).unwrap(), expected);
    previous["macosDisplayMode"] = "both".into();
    assert!(decode(previous).is_err());

    for mode in [
        MacOsDisplayMode::MenuBar,
        MacOsDisplayMode::Floating,
        MacOsDisplayMode::Both,
    ] {
        let mut preferences = expected.clone();
        preferences.macos_display_mode = mode;
        assert_eq!(
            decode(serde_json::to_value(&preferences).unwrap()).unwrap(),
            preferences
        );
    }
    let mut malformed = serde_json::to_value(expected).unwrap();
    malformed["macosDisplayMode"] = "taskbar".into();
    assert!(decode(malformed).is_err());
}

#[test]
fn gpu_display_selections_are_independent_bounded_and_round_trip() {
    let mut preferences = ResidentPreferences {
        gpu_adapter: Some("detail".into()),
        gpu_display: GpuDisplaySelection::Selected {
            adapter_ids: vec!["a".into(), "b".into()],
        },
        ..Default::default()
    };
    assert_eq!(
        decode(serde_json::to_value(&preferences).unwrap()).unwrap(),
        preferences
    );
    for ids in [
        vec![],
        vec!["".into()],
        vec!["a".into(), "a".into()],
        vec!["a\0b".into()],
        vec!["a".repeat(1025)],
        (0..33).map(|id| id.to_string()).collect(),
    ] {
        preferences.gpu_temperature_display = GpuDisplaySelection::Selected { adapter_ids: ids };
        assert!(preferences.clone().normalize().is_err());
    }
    preferences.gpu_temperature_display = GpuDisplaySelection::Selected {
        adapter_ids: (0..32).map(|id| id.to_string()).collect(),
    };
    assert!(preferences.normalize().is_ok());
}

#[test]
fn incomplete_or_duplicate_metrics_and_invalid_fan_ids_are_rejected() {
    let mut preferences = ResidentPreferences::default();
    preferences.metrics.pop();
    assert!(preferences.normalize().is_err());
    let mut preferences = ResidentPreferences::default();
    preferences.metrics[1] = preferences.metrics[0].clone();
    assert!(preferences.normalize().is_err());
    for ids in [vec![], vec![0, 0], vec![0, 1, 2], vec![10]] {
        assert!(ResidentPreferences {
            fan_ids: ids,
            ..Default::default()
        }
        .normalize()
        .is_err());
    }
    let normalized = ResidentPreferences {
        fan_ids: vec![1, 0],
        ..Default::default()
    }
    .normalize()
    .unwrap();
    assert_eq!(normalized.fan_ids, [0, 1]);
}

#[test]
fn released_preferences_upgrade_directly_without_losing_saved_choices() {
    for fixture in [
        include_str!("preference_schema/fixtures/v1.1.1.json"),
        include_str!("preference_schema/fixtures/v1.1.3.json"),
        include_str!("preference_schema/fixtures/v1.1.7.json"),
        include_str!("preference_schema/fixtures/v1.1.8.json"),
        include_str!("preference_schema/fixtures/v1.1.9.json"),
    ] {
        let old: serde_json::Value = serde_json::from_str(fixture).unwrap();
        let migrated = decode(old.clone()).unwrap();
        let current = serde_json::to_value(&migrated).unwrap();
        assert_eq!(migrated.schema_version, PREFERENCE_SCHEMA_VERSION);
        for key in [
            "revision",
            "enabled",
            "showIcon",
            "taskbarPosition",
            "taskbarBackground",
            "taskbarCompact",
            "networkInterface",
            "diskVolume",
            "gpuAdapter",
            "fanDisplay",
            "fanIds",
            "menuBarCompact",
            "usageColors",
            "windowsCpuTemperatureEnabled",
        ] {
            if let Some(value) = old.get(key) {
                assert_eq!(
                    &current[key], value,
                    "schema={} key={key}",
                    old["schemaVersion"]
                );
            }
        }
        let old_metrics = old["metrics"].as_array().unwrap();
        let retained: Vec<_> = current["metrics"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|metric| old_metrics.iter().any(|old| old["id"] == metric["id"]))
            .cloned()
            .collect();
        assert_eq!(&retained, old_metrics);
        for metric in current["metrics"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|metric| !old_metrics.iter().any(|old| old["id"] == metric["id"]))
        {
            assert_eq!(metric["enabled"], false);
        }
        if let Some(rules) = old.get("colorRules") {
            for rule in rules.as_array().unwrap() {
                assert!(current["colorRules"].as_array().unwrap().contains(rule));
            }
        }
        let expected_gpu = if old["gpuAdapter"].is_string() {
            serde_json::json!({"mode":"selected", "adapterIds":[old["gpuAdapter"].clone()]})
        } else {
            serde_json::json!({"mode":"automatic"})
        };
        assert_eq!(current["gpuDisplay"], expected_gpu);
        assert_eq!(
            current["gpuTemperatureDisplay"],
            serde_json::json!({"mode":"automatic"})
        );
        assert_eq!(decode(current).unwrap(), migrated);
    }
}

#[test]
fn release_upgrades_reject_corrupt_payloads_and_development_fields() {
    for fixture in [
        include_str!("preference_schema/fixtures/v1.1.1.json"),
        include_str!("preference_schema/fixtures/v1.1.3.json"),
        include_str!("preference_schema/fixtures/v1.1.7.json"),
        include_str!("preference_schema/fixtures/v1.1.8.json"),
        include_str!("preference_schema/fixtures/v1.1.9.json"),
    ] {
        let old: serde_json::Value = serde_json::from_str(fixture).unwrap();
        for field in [
            "unknown",
            "gpuDisplay",
            "gpuTemperatureDisplay",
            "gpuTemperatureAdapter",
            "gpuAllAdapters",
            "fanId",
        ] {
            let mut invalid = old.clone();
            invalid[field] = serde_json::Value::Null;
            assert_eq!(
                decode(invalid).unwrap_err(),
                "preferences_invalid",
                "schema={} field={field}",
                old["schemaVersion"]
            );
        }
        let mut invalid = old.clone();
        invalid["metrics"].as_array_mut().unwrap().pop();
        assert!(decode(invalid).is_err());
        let mut invalid = old.clone();
        invalid["metrics"][1] = invalid["metrics"][0].clone();
        assert!(decode(invalid).is_err());
        if old["schemaVersion"].as_u64().unwrap() >= 21 {
            for mutation in 0..3 {
                let mut invalid = old.clone();
                match mutation {
                    0 => {
                        invalid["colorRules"].as_array_mut().unwrap().pop();
                    }
                    1 => {
                        invalid["colorRules"][1] = invalid["colorRules"][0].clone();
                    }
                    _ => {
                        invalid["colorRules"][0]["rules"][0]["value"] = 101.into();
                    }
                }
                assert!(decode(invalid).is_err());
            }
        }
    }
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("preference_schema/fixtures/v1.1.1.json")).unwrap();
    let mut invalid = fixture.clone();
    invalid["windowsCpuTemperatureEnabled"] = true.into();
    assert!(decode(invalid).is_err());
    let mut invalid = fixture;
    invalid["gpuAdapter"] = serde_json::Value::Null;
    assert!(decode(invalid).is_err());
}

#[test]
fn released_percentage_colors_preserve_thresholds_and_privilege_defaults() {
    use super::super::color_rules::{ColorRule, DisplayColor};
    let old: serde_json::Value =
        serde_json::from_str(include_str!("preference_schema/fixtures/v1.1.3.json")).unwrap();
    let migrated = decode(old.clone()).unwrap();
    assert!(!migrated.windows_cpu_temperature_enabled);
    assert_eq!(migrated.fan_display, FanDisplayMode::Highest);
    assert_eq!(migrated.fan_ids, [0]);
    let rule = migrated
        .color_rules
        .iter()
        .find(|rule| rule.id == DisplayMetricId::Cpu)
        .unwrap();
    for value in 0..=100 {
        let expected = if value < 55 {
            DisplayColor::Default
        } else if value < 85 {
            DisplayColor::Orange
        } else {
            DisplayColor::Red
        };
        let actual: Vec<_> = rule
            .rules
            .iter()
            .filter(|rule| rule.matches(value))
            .map(ColorRule::color)
            .collect();
        assert_eq!(actual, [expected]);
    }
    for (warning, critical) in [(0, 90), (90, 70), (70, 101)] {
        let mut invalid = old.clone();
        invalid["usageWarningPercent"] = warning.into();
        invalid["usageCriticalPercent"] = critical.into();
        assert!(decode(invalid).is_err());
    }
}

#[test]
fn disk_temperature_device_is_independent_and_released_preferences_follow_the_selected_volume() {
    let mut preferences = ResidentPreferences {
        disk_volume: Some("saved-volume".into()),
        disk_temperature_device: Some("physical-device".into()),
        ..Default::default()
    };
    let encoded = serde_json::to_value(&preferences).unwrap();
    assert_eq!(decode(encoded).unwrap(), preferences);
    preferences.disk_temperature_device = Some("".into());
    assert!(preferences.normalize().is_err());
    for text in [
        include_str!("preference_schema/fixtures/v1.1.1.json"),
        include_str!("preference_schema/fixtures/v1.1.9.json"),
    ] {
        let mut value: serde_json::Value = serde_json::from_str(text).unwrap();
        let migrated = decode(value.clone()).unwrap();
        assert!(migrated.disk_temperature_device.is_none());
        assert_eq!(migrated.disk_volume.as_deref(), Some("saved-volume"));
        value["diskTemperatureDevice"] = serde_json::json!("development-value");
        assert!(decode(value).is_err());
    }
}

#[test]
fn previous_monitor_settings_upgrade_without_changing_user_choices() {
    let mut previous = serde_json::to_value(ResidentPreferences::default()).unwrap();
    previous["schemaVersion"] = 28.into();
    remove_frequency_items(&mut previous);
    previous.as_object_mut().unwrap().remove("linuxDisplayMode");
    previous.as_object_mut().unwrap().remove("macosDisplayMode");
    previous["revision"] = 91.into();
    previous["enabled"] = false.into();
    previous["taskbarPosition"] = "left".into();
    let object = previous.as_object_mut().unwrap();
    object.remove("windowsDisplayMode");
    object.remove("floating");
    let upgraded = decode(previous.clone()).unwrap();
    assert_eq!(upgraded.schema_version, PREFERENCE_SCHEMA_VERSION);
    assert_eq!(upgraded.revision, 91);
    assert!(!upgraded.enabled);
    assert_eq!(upgraded.taskbar_position, TaskbarPosition::Left);
    assert_eq!(upgraded.windows_display_mode, WindowsDisplayMode::Taskbar);
    assert_eq!(upgraded.floating, FloatingPreferences::default());
    previous["floating"] = serde_json::json!({"layout":"vertical"});
    assert!(
        decode(previous).is_err(),
        "old schemas must reject unrecognized fields"
    );
}

#[test]
fn floating_options_reject_unknown_modes_and_preserve_a_recovery_icon() {
    let mut preferences = ResidentPreferences {
        show_icon: false,
        windows_display_mode: WindowsDisplayMode::Floating,
        ..Default::default()
    };
    assert_eq!(preferences.effective_icon(), cfg!(windows));
    preferences.floating.layout = FloatingLayout::Vertical;
    preferences.floating.locked = true;
    let value = serde_json::to_value(&preferences).unwrap();
    assert_eq!(decode(value.clone()).unwrap(), preferences);
    let mut malformed = value.clone();
    malformed["windowsDisplayMode"] = "desktop".into();
    assert!(decode(malformed).is_err());
    let mut malformed = value;
    malformed["floating"]["layout"] = "grid".into();
    assert!(decode(malformed).is_err());
}

#[test]
fn previous_floating_settings_keep_layout_and_position_options_with_opaque_background() {
    let mut previous = serde_json::to_value(ResidentPreferences::default()).unwrap();
    previous["schemaVersion"] = 29.into();
    remove_frequency_items(&mut previous);
    previous.as_object_mut().unwrap().remove("linuxDisplayMode");
    previous["floating"]
        .as_object_mut()
        .unwrap()
        .remove("hideOnFullscreen");
    previous.as_object_mut().unwrap().remove("macosDisplayMode");
    for field in ["fontSize", "bold", "background", "dividers", "clickThrough"] {
        previous["floating"].as_object_mut().unwrap().remove(field);
    }
    previous["revision"] = 92.into();
    previous["windowsDisplayMode"] = "both".into();
    previous["floating"]["layout"] = "vertical".into();
    previous["floating"]["locked"] = true.into();
    previous["floating"]
        .as_object_mut()
        .unwrap()
        .remove("backgroundOpacity");
    previous["floating"]
        .as_object_mut()
        .unwrap()
        .remove("snapToEdges");
    let migrated = decode(previous.clone()).unwrap();
    assert_eq!(migrated.schema_version, PREFERENCE_SCHEMA_VERSION);
    assert_eq!(migrated.revision, 92);
    assert_eq!(migrated.windows_display_mode, WindowsDisplayMode::Both);
    assert_eq!(migrated.floating.layout, FloatingLayout::Vertical);
    assert!(migrated.floating.locked);
    assert_eq!(migrated.floating.background_opacity, 100);
    assert!(migrated.floating.snap_to_edges);
    previous["floating"]["backgroundOpacity"] = 75.into();
    assert!(
        decode(previous).is_err(),
        "old schemas must reject future fields"
    );
}

#[test]
fn floating_background_opacity_requires_a_bounded_integer_percentage() {
    let value = serde_json::to_value(ResidentPreferences::default()).unwrap();
    for percentage in [50, 75, 100] {
        let mut changed = value.clone();
        changed["floating"]["backgroundOpacity"] = percentage.into();
        assert_eq!(
            decode(changed).unwrap().floating.background_opacity,
            percentage as u8
        );
    }
    for invalid in [
        serde_json::json!(0),
        serde_json::json!(49),
        serde_json::json!(101),
        serde_json::json!(-1),
        serde_json::json!(60.5),
        serde_json::json!("60"),
    ] {
        let mut changed = value.clone();
        changed["floating"]["backgroundOpacity"] = invalid;
        assert!(decode(changed).is_err());
    }
    let mut missing = value;
    missing["floating"]
        .as_object_mut()
        .unwrap()
        .remove("backgroundOpacity");
    assert!(decode(missing).is_err());
}

#[test]
fn previous_opacity_settings_enable_snapping_without_resetting_other_choices() {
    let mut previous = serde_json::to_value(ResidentPreferences::default()).unwrap();
    previous["schemaVersion"] = 30.into();
    remove_frequency_items(&mut previous);
    previous.as_object_mut().unwrap().remove("linuxDisplayMode");
    previous["floating"]
        .as_object_mut()
        .unwrap()
        .remove("hideOnFullscreen");
    previous.as_object_mut().unwrap().remove("macosDisplayMode");
    for field in ["fontSize", "bold", "background", "dividers", "clickThrough"] {
        previous["floating"].as_object_mut().unwrap().remove(field);
    }
    previous["revision"] = 94.into();
    previous["floating"]["backgroundOpacity"] = 60.into();
    previous["floating"]["layout"] = "vertical".into();
    previous["floating"]["locked"] = true.into();
    previous["floating"]
        .as_object_mut()
        .unwrap()
        .remove("snapToEdges");
    let migrated = decode(previous.clone()).unwrap();
    assert_eq!(migrated.revision, 94);
    assert_eq!(migrated.floating.background_opacity, 60);
    assert_eq!(migrated.floating.layout, FloatingLayout::Vertical);
    assert!(migrated.floating.locked && migrated.floating.snap_to_edges);
    previous["floating"]["snapToEdges"] = false.into();
    assert!(
        decode(previous).is_err(),
        "previous schemas reject future fields"
    );
}

#[test]
fn snapping_requires_an_explicit_boolean_in_the_current_schema() {
    let value = serde_json::to_value(ResidentPreferences::default()).unwrap();
    for enabled in [true, false] {
        let mut changed = value.clone();
        changed["floating"]["snapToEdges"] = enabled.into();
        assert_eq!(decode(changed).unwrap().floating.snap_to_edges, enabled);
    }
    for invalid in [
        serde_json::json!(0),
        serde_json::json!("false"),
        serde_json::Value::Null,
    ] {
        let mut changed = value.clone();
        changed["floating"]["snapToEdges"] = invalid;
        assert!(decode(changed).is_err());
    }
    let mut missing = value;
    missing["floating"]
        .as_object_mut()
        .unwrap()
        .remove("snapToEdges");
    assert!(decode(missing).is_err());
}

#[test]
fn previous_floating_appearance_keeps_opacity_and_defaults_new_options() {
    let mut previous = serde_json::to_value(ResidentPreferences::default()).unwrap();
    previous["schemaVersion"] = 31.into();
    remove_frequency_items(&mut previous);
    previous.as_object_mut().unwrap().remove("linuxDisplayMode");
    previous["floating"]
        .as_object_mut()
        .unwrap()
        .remove("hideOnFullscreen");
    previous.as_object_mut().unwrap().remove("macosDisplayMode");
    previous["floating"]["backgroundOpacity"] = 60.into();
    previous["floating"]["snapToEdges"] = false.into();
    for field in ["fontSize", "bold", "background", "dividers", "clickThrough"] {
        previous["floating"].as_object_mut().unwrap().remove(field);
    }
    let migrated = decode(previous.clone()).unwrap();
    assert_eq!(migrated.floating.background_opacity, 60);
    assert!(!migrated.floating.snap_to_edges);
    assert_eq!(migrated.floating.font_size, FloatingFontSize::Standard);
    assert!(!migrated.floating.bold);
    assert_eq!(migrated.floating.background, FloatingBackground::Default);
    previous["floating"]["bold"] = false.into();
    assert!(decode(previous).is_err());
}

#[test]
fn floating_appearance_requires_known_presets_and_explicit_fields() {
    let current = serde_json::to_value(ResidentPreferences::default()).unwrap();
    for (field, values) in [
        (
            "fontSize",
            vec!["standard", "larger", "large", "extraLarge"],
        ),
        (
            "background",
            vec![
                "default",
                "darkGray",
                "inkBlue",
                "warmWhite",
                "lightGray",
                "black",
                "white",
            ],
        ),
    ] {
        for value in values {
            let mut changed = current.clone();
            changed["floating"][field] = value.into();
            assert!(decode(changed).is_ok());
        }
        let mut invalid = current.clone();
        invalid["floating"][field] = "unknown".into();
        assert!(decode(invalid).is_err());
    }
    for field in ["fontSize", "bold", "background"] {
        let mut missing = current.clone();
        missing["floating"].as_object_mut().unwrap().remove(field);
        assert!(decode(missing).is_err());
    }
    for value in [
        serde_json::json!(1),
        serde_json::json!("false"),
        serde_json::Value::Null,
    ] {
        let mut invalid = current.clone();
        invalid["floating"]["bold"] = value;
        assert!(decode(invalid).is_err());
    }
}

#[test]
fn previous_floating_appearance_adds_dividers_without_changing_saved_choices() {
    let expected = ResidentPreferences {
        revision: 97,
        floating: FloatingPreferences {
            layout: FloatingLayout::Vertical,
            background_opacity: 60,
            snap_to_edges: false,
            bold: true,
            font_size: FloatingFontSize::ExtraLarge,
            background: FloatingBackground::InkBlue,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut previous = serde_json::to_value(&expected).unwrap();
    previous["schemaVersion"] = 32.into();
    remove_frequency_items(&mut previous);
    previous.as_object_mut().unwrap().remove("linuxDisplayMode");
    previous["floating"]
        .as_object_mut()
        .unwrap()
        .remove("hideOnFullscreen");
    previous.as_object_mut().unwrap().remove("macosDisplayMode");
    previous["floating"]
        .as_object_mut()
        .unwrap()
        .remove("dividers");
    previous["floating"]
        .as_object_mut()
        .unwrap()
        .remove("clickThrough");
    assert_eq!(decode(previous.clone()).unwrap(), expected);
    previous["floating"]["dividers"] = false.into();
    assert!(
        decode(previous).is_err(),
        "older schemas reject future fields"
    );
}

#[test]
fn floating_dividers_require_an_explicit_boolean_and_round_trip() {
    let current = serde_json::to_value(ResidentPreferences::default()).unwrap();
    for enabled in [true, false] {
        let mut changed = current.clone();
        changed["floating"]["dividers"] = enabled.into();
        let decoded = decode(changed).unwrap();
        assert_eq!(decoded.floating.dividers, enabled);
        assert_eq!(
            decode(serde_json::to_value(&decoded).unwrap()).unwrap(),
            decoded
        );
    }
    for invalid in [
        serde_json::json!(0),
        serde_json::json!("false"),
        serde_json::Value::Null,
    ] {
        let mut changed = current.clone();
        changed["floating"]["dividers"] = invalid;
        assert!(decode(changed).is_err());
    }
    let mut missing = current;
    missing["floating"]
        .as_object_mut()
        .unwrap()
        .remove("dividers");
    assert!(decode(missing).is_err());
}

#[test]
fn click_through_migrates_disabled_and_preserves_previous_choices() {
    let expected = ResidentPreferences {
        revision: 123,
        windows_display_mode: WindowsDisplayMode::Both,
        show_icon: false,
        floating: FloatingPreferences {
            dividers: false,
            locked: true,
            layout: FloatingLayout::Vertical,
            background_opacity: 60,
            background: FloatingBackground::Black,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut previous = serde_json::to_value(&expected).unwrap();
    previous["schemaVersion"] = 33.into();
    remove_frequency_items(&mut previous);
    previous.as_object_mut().unwrap().remove("linuxDisplayMode");
    previous["floating"]
        .as_object_mut()
        .unwrap()
        .remove("hideOnFullscreen");
    previous.as_object_mut().unwrap().remove("macosDisplayMode");
    previous["floating"]
        .as_object_mut()
        .unwrap()
        .remove("clickThrough");
    assert_eq!(decode(previous.clone()).unwrap(), expected);
    previous["floating"]["clickThrough"] = true.into();
    assert!(
        decode(previous).is_err(),
        "previous schemas reject future fields"
    );
}

#[test]
fn click_through_requires_an_explicit_boolean_and_retains_recovery() {
    let current = serde_json::to_value(ResidentPreferences::default()).unwrap();
    assert!(!ResidentPreferences::default().floating.click_through);
    for enabled in [false, true] {
        let mut value = current.clone();
        value["floating"]["clickThrough"] = enabled.into();
        let mut preferences = decode(value).unwrap();
        assert_eq!(preferences.floating.click_through, enabled);
        preferences.show_icon = false;
        for mode in [WindowsDisplayMode::Floating, WindowsDisplayMode::Both] {
            preferences.windows_display_mode = mode;
            assert_eq!(preferences.effective_icon(), cfg!(windows));
        }
        assert_eq!(
            decode(serde_json::to_value(&preferences).unwrap()).unwrap(),
            preferences
        );
    }
    for invalid in [
        serde_json::json!(null),
        serde_json::json!(1),
        serde_json::json!("true"),
    ] {
        let mut value = current.clone();
        value["floating"]["clickThrough"] = invalid;
        assert!(decode(value).is_err());
    }
    let mut missing = current;
    missing["floating"]
        .as_object_mut()
        .unwrap()
        .remove("clickThrough");
    assert!(decode(missing).is_err());
}

#[test]
fn fullscreen_hiding_migrates_enabled_without_resetting_other_choices() {
    let expected = ResidentPreferences {
        revision: 125,
        macos_display_mode: MacOsDisplayMode::Both,
        floating: FloatingPreferences {
            background: FloatingBackground::InkBlue,
            locked: true,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut previous = serde_json::to_value(&expected).unwrap();
    previous["schemaVersion"] = 35.into();
    remove_frequency_items(&mut previous);
    previous.as_object_mut().unwrap().remove("linuxDisplayMode");
    previous["floating"]
        .as_object_mut()
        .unwrap()
        .remove("hideOnFullscreen");
    assert_eq!(decode(previous.clone()).unwrap(), expected);
    previous["floating"]["hideOnFullscreen"] = false.into();
    assert!(
        decode(previous).is_err(),
        "previous schemas reject future fields"
    );
}

#[test]
fn fullscreen_hiding_requires_a_boolean_and_preserves_both_values() {
    let current = serde_json::to_value(ResidentPreferences::default()).unwrap();
    assert!(ResidentPreferences::default().floating.hide_on_fullscreen);
    for enabled in [false, true] {
        let mut changed = current.clone();
        changed["floating"]["hideOnFullscreen"] = enabled.into();
        let preferences = decode(changed).unwrap();
        assert_eq!(preferences.floating.hide_on_fullscreen, enabled);
        assert_eq!(
            decode(serde_json::to_value(&preferences).unwrap()).unwrap(),
            preferences
        );
    }
    for invalid in [
        serde_json::json!(1),
        serde_json::json!("true"),
        serde_json::Value::Null,
    ] {
        let mut changed = current.clone();
        changed["floating"]["hideOnFullscreen"] = invalid;
        assert!(decode(changed).is_err());
    }
    let mut missing = current;
    missing["floating"]
        .as_object_mut()
        .unwrap()
        .remove("hideOnFullscreen");
    assert!(decode(missing).is_err());
}

#[test]
fn linux_display_migrates_to_tray_and_preserves_platform_preferences() {
    let expected = ResidentPreferences {
        revision: 93,
        windows_display_mode: WindowsDisplayMode::Both,
        macos_display_mode: MacOsDisplayMode::Both,
        floating: FloatingPreferences {
            bold: true,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut old = serde_json::to_value(&expected).unwrap();
    old["schemaVersion"] = 36.into();
    remove_frequency_items(&mut old);
    old.as_object_mut().unwrap().remove("linuxDisplayMode");
    assert_eq!(decode(old.clone()).unwrap(), expected);
    old["linuxDisplayMode"] = "floating".into();
    assert!(decode(old).is_err(), "old schemas reject future fields");
    for mode in [
        LinuxDisplayMode::Tray,
        LinuxDisplayMode::Floating,
        LinuxDisplayMode::Both,
    ] {
        let mut preferences = expected.clone();
        preferences.linux_display_mode = mode;
        assert_eq!(
            decode(serde_json::to_value(&preferences).unwrap()).unwrap(),
            preferences
        );
    }
    for value in [
        serde_json::json!("taskbar"),
        serde_json::json!("menuBar"),
        serde_json::json!(true),
        serde_json::Value::Null,
    ] {
        let mut invalid = serde_json::to_value(&expected).unwrap();
        invalid["linuxDisplayMode"] = value;
        assert!(decode(invalid).is_err());
    }
    let mut missing = serde_json::to_value(&expected).unwrap();
    missing.as_object_mut().unwrap().remove("linuxDisplayMode");
    assert!(decode(missing).is_err());
}

fn remove_frequency_items(value: &mut serde_json::Value) {
    value["metrics"]
        .as_array_mut()
        .unwrap()
        .retain(|item| !item["id"].as_str().unwrap().ends_with("Frequency"));
}

#[test]
fn frequency_display_upgrade_preserves_order_and_defaults_new_items_off() {
    let mut expected = ResidentPreferences {
        revision: 128,
        show_icon: false,
        ..Default::default()
    };
    expected.metrics.reverse();
    let mut previous = serde_json::to_value(&expected).unwrap();
    previous["schemaVersion"] = 37.into();
    remove_frequency_items(&mut previous);
    let migrated = decode(previous.clone()).unwrap();
    let original: Vec<_> = expected
        .metrics
        .iter()
        .filter(|item| item.id.frequency_class().is_none())
        .collect();
    let preserved: Vec<_> = migrated
        .metrics
        .iter()
        .filter(|item| item.id.frequency_class().is_none())
        .collect();
    assert_eq!(original, preserved);
    assert_eq!(migrated.revision, 128);
    assert!(!migrated.show_icon);
    assert_eq!(
        migrated
            .metrics
            .iter()
            .filter(|item| item.id.frequency_class().is_some())
            .count(),
        4
    );
    assert!(migrated
        .metrics
        .iter()
        .filter(|item| item.id.frequency_class().is_some())
        .all(|item| !item.enabled));
    previous["metrics"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({"id":"cpuFrequency","enabled":true}));
    assert!(decode(previous).is_err());
}

#[test]
fn version_38_adds_component_power_without_enabling_or_reordering_existing_metrics() {
    let mut original = serde_json::to_value(ResidentPreferences::default()).unwrap();
    original["schemaVersion"] = 38.into();
    for key in ["metrics", "colorRules"] {
        original[key]
            .as_array_mut()
            .unwrap()
            .retain(|row| !matches!(row["id"].as_str(), Some("cpuPower" | "gpuPower")));
    }
    original["metrics"].as_array_mut().unwrap().reverse();
    let expected = original["metrics"].clone();
    let decoded = super::decode(original).unwrap();
    let upgraded = serde_json::to_value(&decoded).unwrap();
    assert_eq!(upgraded["schemaVersion"], PREFERENCE_SCHEMA_VERSION);
    let existing: Vec<_> = upgraded["metrics"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| !matches!(row["id"].as_str(), Some("cpuPower" | "gpuPower")))
        .cloned()
        .collect();
    assert_eq!(&existing, expected.as_array().unwrap());
    for (id, anchor) in [
        (DisplayMetricId::CpuPower, DisplayMetricId::CpuTemperature),
        (DisplayMetricId::GpuPower, DisplayMetricId::GpuTemperature),
    ] {
        let index = decoded
            .metrics
            .iter()
            .position(|row| row.id == anchor)
            .unwrap();
        assert_eq!(decoded.metrics[index + 1].id, id);
    }
    for id in [DisplayMetricId::CpuPower, DisplayMetricId::GpuPower] {
        assert!(
            !decoded
                .metrics
                .iter()
                .find(|row| row.id == id)
                .unwrap()
                .enabled
        );
        assert!(
            !decoded
                .color_rules
                .iter()
                .find(|row| row.id == id)
                .unwrap()
                .enabled
        );
    }
}

#[test]
fn public_network_preferences_default_to_enabled_and_migrate_without_resetting_choices() {
    let defaults = ResidentPreferences::default();
    assert!(defaults.public_network.enabled);
    let mut old = serde_json::to_value(defaults).unwrap();
    old["schemaVersion"] = 39.into();
    old["revision"] = 27.into();
    old["networkInterface"] = "saved-wired-adapter".into();
    old.as_object_mut().unwrap().remove("publicNetwork");
    let migrated = decode(old).unwrap();
    assert_eq!(migrated.revision, 27);
    assert_eq!(
        migrated.network_interface.as_deref(),
        Some("saved-wired-adapter")
    );
    assert!(migrated.public_network.enabled);
    let mut saved = migrated;
    saved.public_network.enabled = false;
    assert_eq!(
        decode(serde_json::to_value(&saved).unwrap()).unwrap(),
        saved
    );
}

#[test]
fn public_network_preferences_remove_preview_intervals_and_preserve_disabled_choices() {
    for minutes in [0, 1, 5, 15, 30, 60] {
        let mut preview = serde_json::to_value(ResidentPreferences::default()).unwrap();
        preview["schemaVersion"] = 40.into();
        preview["revision"] = 27.into();
        preview["networkInterface"] = "saved-wired-adapter".into();
        preview["publicNetwork"] =
            serde_json::json!({ "enabled": false, "refreshMinutes": minutes });
        let upgraded = decode(preview).unwrap();
        assert_eq!(upgraded.revision, 27);
        assert_eq!(
            upgraded.network_interface.as_deref(),
            Some("saved-wired-adapter")
        );
        assert!(!upgraded.public_network.enabled);
        let value = serde_json::to_value(upgraded).unwrap();
        assert_eq!(value["schemaVersion"], 42);
        assert_eq!(
            value["publicNetwork"],
            serde_json::json!({ "enabled": false })
        );
    }
}

#[test]
fn public_network_preferences_reject_malformed_values_and_obsolete_fields_in_current_schema() {
    let value = serde_json::to_value(ResidentPreferences::default()).unwrap();
    for invalid in [
        serde_json::json!("true"),
        serde_json::Value::Null,
        serde_json::json!(1),
    ] {
        let mut malformed = value.clone();
        malformed["publicNetwork"]["enabled"] = invalid;
        assert!(decode(malformed).is_err());
    }
    let mut obsolete = value.clone();
    obsolete["publicNetwork"]["refreshMinutes"] = 5.into();
    assert!(decode(obsolete).is_err());
    for invalid in [
        serde_json::json!(2),
        serde_json::json!(-1),
        serde_json::json!("5"),
        serde_json::Value::Null,
    ] {
        let mut preview = value.clone();
        preview["schemaVersion"] = 40.into();
        preview["publicNetwork"]["refreshMinutes"] = invalid;
        assert!(decode(preview).is_err());
    }
}

#[test]
fn display_choice_upgrades_and_preserves_public_network_and_disconnected_identity() {
    for version in [39, 41] {
        let mut old = serde_json::to_value(ResidentPreferences::default()).unwrap();
        old["schemaVersion"] = version.into();
        old.as_object_mut().unwrap().remove("taskbarMonitor");
        old["publicNetwork"]["enabled"] = false.into();
        let migrated = decode(old).unwrap();
        assert_eq!(migrated.taskbar_monitor, None);
        if version == 41 {
            assert!(!migrated.public_network.enabled);
        }
    }
    let fixed = ResidentPreferences {
        taskbar_monitor: Some("display-interface-42".into()),
        ..Default::default()
    };
    assert_eq!(
        decode(serde_json::to_value(&fixed).unwrap()).unwrap(),
        fixed
    );
    for id in ["".to_string(), "x".repeat(1025), "a\0b".to_string()] {
        assert!(ResidentPreferences {
            taskbar_monitor: Some(id),
            ..Default::default()
        }
        .normalize()
        .is_err());
    }
}
