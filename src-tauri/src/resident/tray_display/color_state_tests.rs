use super::*;
use usage_color::UsageTone;

#[cfg(not(windows))]
#[test]
fn fan_colors_keep_rows_independent_and_clear_history_for_missing_hidden_or_changed_sources() {
    use crate::resident::preference_schema::{DisplayMetricId, FanDisplayMode};
    use mangodisk_core::system_resources::{metrics::MetricReading, readings::ResourceReadings};
    use mangodisk_platform::system_resources::fans::{FanSample, FanSpeed};
    let labels = labels::Labels::for_locale("en-US");
    let mut state = DisplayState::default();
    let mut preferences = ResidentPreferences::default();
    let frame = |preferences: &ResidentPreferences, rpm: [Option<f64>; 2]| {
        vec![fan::entry(
            &ResourceReadings {
                observed_at_ms: 1000,
                fan_speed: MetricReading::ready(
                    FanSample {
                        fans: rpm
                            .into_iter()
                            .enumerate()
                            .map(|(id, rpm)| FanSpeed { id: id as u8, rpm })
                            .collect(),
                    },
                    1000,
                ),
                ..Default::default()
            },
            &labels,
            preferences,
        )]
    };
    let paint = |state: &mut DisplayState,
                 preferences: &ResidentPreferences,
                 entries: &mut [DisplayEntry]| {
        state
            .colors
            .apply(entries, preferences, None, None, None, None)
    };
    let mut entries = frame(&preferences, [Some(4000.0), Some(0.0)]);
    paint(&mut state, &preferences, &mut entries);
    assert_eq!(entries[0].tone, UsageTone::Critical);
    entries = frame(&preferences, [Some(3999.0), Some(0.0)]);
    paint(&mut state, &preferences, &mut entries);
    assert_eq!(entries[0].tone, UsageTone::Critical);
    preferences.fan_display = FanDisplayMode::Average;
    entries = frame(&preferences, [Some(3999.0), Some(3999.0)]);
    paint(&mut state, &preferences, &mut entries);
    assert_eq!(entries[0].tone, UsageTone::Warning);
    preferences.fan_display = FanDisplayMode::Selected;
    preferences.fan_ids = vec![0, 1];
    entries = frame(&preferences, [Some(4000.0), Some(0.0)]);
    paint(&mut state, &preferences, &mut entries);
    assert_eq!(
        entries[0]
            .fan_rows
            .as_ref()
            .unwrap()
            .each_ref()
            .map(|row| row.tone),
        [UsageTone::Critical, UsageTone::Green]
    );
    entries = frame(&preferences, [Some(3999.0), Some(2000.0)]);
    paint(&mut state, &preferences, &mut entries);
    assert_eq!(
        entries[0]
            .fan_rows
            .as_ref()
            .unwrap()
            .each_ref()
            .map(|row| row.tone),
        [UsageTone::Critical, UsageTone::Warning]
    );
    entries = frame(&preferences, [None, Some(1900.0)]);
    paint(&mut state, &preferences, &mut entries);
    assert_eq!(
        entries[0]
            .fan_rows
            .as_ref()
            .unwrap()
            .each_ref()
            .map(|row| row.tone),
        [UsageTone::Normal, UsageTone::Green]
    );
    entries = frame(&preferences, [Some(3999.0), Some(0.0)]);
    paint(&mut state, &preferences, &mut entries);
    assert_eq!(
        entries[0].fan_rows.as_ref().unwrap()[0].tone,
        UsageTone::Warning
    );
    preferences.usage_colors = false;
    paint(&mut state, &preferences, &mut entries);
    assert!(entries[0]
        .fan_rows
        .as_ref()
        .unwrap()
        .iter()
        .all(|row| row.tone == UsageTone::Normal));
    preferences.usage_colors = true;
    preferences
        .color_rules
        .iter_mut()
        .find(|rule| rule.id == DisplayMetricId::FanSpeed)
        .unwrap()
        .enabled = false;
    paint(&mut state, &preferences, &mut entries);
    assert!(entries[0]
        .fan_rows
        .as_ref()
        .unwrap()
        .iter()
        .all(|row| row.tone == UsageTone::Normal));
    preferences
        .color_rules
        .iter_mut()
        .find(|rule| rule.id == DisplayMetricId::FanSpeed)
        .unwrap()
        .enabled = true;
    entries = frame(&preferences, [Some(4000.0), Some(0.0)]);
    paint(&mut state, &preferences, &mut entries);
    paint(&mut state, &preferences, &mut []);
    entries = frame(&preferences, [Some(3999.0), Some(0.0)]);
    paint(&mut state, &preferences, &mut entries);
    assert_eq!(
        entries[0].fan_rows.as_ref().unwrap()[0].tone,
        UsageTone::Warning
    );
    preferences.fan_ids = vec![0];
    entries = frame(&preferences, [Some(4000.0), Some(3999.0)]);
    paint(&mut state, &preferences, &mut entries);
    assert_eq!(entries[0].tone, UsageTone::Critical);
    preferences.fan_ids = vec![1];
    entries = frame(&preferences, [Some(4000.0), Some(3999.0)]);
    paint(&mut state, &preferences, &mut entries);
    assert_eq!(entries[0].tone, UsageTone::Warning);
    preferences
        .color_rules
        .iter_mut()
        .find(|rule| rule.id == DisplayMetricId::FanSpeed)
        .unwrap()
        .rules = vec![crate::resident::color_rules::ColorRule::AtLeast {
        value: 0,
        color: crate::resident::color_rules::DisplayColor::Blue,
    }];
    paint(&mut state, &preferences, &mut entries);
    assert_eq!(entries[0].tone, UsageTone::Blue);
}

#[test]
fn battery_power_color_rules_apply_to_their_own_slot() {
    use crate::resident::color_rules::{ColorRule, DisplayColor};
    use crate::resident::preference_schema::DisplayMetricId;
    let mut preferences = ResidentPreferences::default();
    let rule = preferences
        .color_rules
        .iter_mut()
        .find(|rule| rule.id == DisplayMetricId::BatteryPower)
        .unwrap();
    rule.enabled = true;
    rule.rules = vec![ColorRule::Below {
        value: 100,
        color: DisplayColor::Blue,
    }];
    let mut entries = vec![DisplayEntry {
        frequency: None,
        gpu_device: None,
        id: DisplayId::BatteryPower,
        tone: Default::default(),
        power_state: None,
        fan_rows: None,
        color_value: Some(50),
        marker: "W".into(),
        digits: "50.0".into(),
        text: String::new(),
        tooltip: String::new(),
    }];
    DisplayState::default()
        .colors
        .apply(&mut entries, &preferences, None, None, None, None);
    assert_eq!(entries[0].tone, UsageTone::Blue);
}

#[test]
fn temperature_and_percentage_rules_are_independent_and_unavailable_is_neutral() {
    let mut state = DisplayState::default();
    let preferences = ResidentPreferences::default();
    let mut entries: Vec<_> = [
        DisplayId::Cpu,
        DisplayId::CpuTemperature,
        DisplayId::GpuTemperature,
    ]
    .into_iter()
    .map(|id| DisplayEntry {
        frequency: None,
        gpu_device: None,
        id,
        tone: Default::default(),
        power_state: None,
        fan_rows: None,
        color_value: Some(65),
        marker: String::new(),
        digits: String::new(),
        text: String::new(),
        tooltip: String::new(),
    })
    .collect();
    state
        .colors
        .apply(&mut entries, &preferences, None, None, Some("a"), Some("a"));
    assert_eq!(entries[0].tone, UsageTone::Green);
    assert_eq!(entries[1].tone, UsageTone::Warning);
    assert_eq!(entries[2].tone, UsageTone::Warning);
    entries[2].color_value = Some(80);
    state
        .colors
        .apply(&mut entries, &preferences, None, None, Some("a"), Some("a"));
    assert_eq!(entries[2].tone, UsageTone::Critical);
    entries[2].color_value = Some(79);
    state
        .colors
        .apply(&mut entries, &preferences, None, None, Some("a"), Some("a"));
    assert_eq!(entries[2].tone, UsageTone::Critical);
    state
        .colors
        .apply(&mut entries, &preferences, None, None, Some("b"), Some("b"));
    assert_eq!(entries[2].tone, UsageTone::Warning);
    entries[1].color_value = None;
    state
        .colors
        .apply(&mut entries, &preferences, None, None, Some("b"), Some("b"));
    assert_eq!(entries[1].tone, UsageTone::Normal);
}

#[test]
fn switching_disks_resets_hysteresis_without_resetting_cpu() {
    let mut state = DisplayState::default();
    let preferences = ResidentPreferences::default();
    let mut entries: Vec<_> = [DisplayId::Cpu, DisplayId::Disk]
        .into_iter()
        .map(|id| DisplayEntry {
            frequency: None,
            gpu_device: None,
            id,
            tone: UsageTone::Normal,
            power_state: None,
            fan_rows: None,
            color_value: Some(95),
            marker: String::new(),
            digits: String::new(),
            text: String::new(),
            tooltip: String::new(),
        })
        .collect();
    state
        .colors
        .apply(&mut entries, &preferences, Some("disk-a"), None, None, None);
    assert!(entries
        .iter()
        .all(|entry| entry.tone == UsageTone::Critical));
    state.entries = entries
        .iter()
        .cloned()
        .map(|entry| (entry.id, entry))
        .collect();
    for entry in &mut entries {
        entry.color_value = Some(89);
    }
    state
        .colors
        .apply(&mut entries, &preferences, Some("disk-a"), None, None, None);
    assert!(entries
        .iter()
        .all(|entry| entry.tone == UsageTone::Critical));
    state
        .colors
        .apply(&mut entries, &preferences, Some("disk-b"), None, None, None);
    assert_eq!(entries[0].tone, UsageTone::Critical);
    assert_eq!(entries[1].tone, UsageTone::Warning);
    // Simulate a failed native frame by leaving the paint cache unchanged.
    state
        .colors
        .apply(&mut entries, &preferences, Some("disk-b"), None, None, None);
    assert_eq!(entries[1].tone, UsageTone::Warning);
    let mut higher_threshold = preferences;
    for rule in &mut higher_threshold.color_rules {
        rule.rules = vec![
            super::super::color_rules::ColorRule::Below {
                value: 70,
                color: super::super::color_rules::DisplayColor::Default,
            },
            super::super::color_rules::ColorRule::Range {
                from: 70,
                to: 91,
                color: super::super::color_rules::DisplayColor::Orange,
            },
            super::super::color_rules::ColorRule::AtLeast {
                value: 91,
                color: super::super::color_rules::DisplayColor::Red,
            },
        ];
    }
    state.colors.apply(
        &mut entries,
        &higher_threshold,
        Some("disk-b"),
        None,
        None,
        None,
    );
    state.colors.apply(
        &mut entries,
        &higher_threshold,
        Some("disk-b"),
        None,
        None,
        None,
    );
    assert!(entries.iter().all(|entry| entry.tone == UsageTone::Warning));
}

#[test]
fn multiple_gpu_colors_follow_device_identity_across_reordering_and_removal() {
    use display_entry::GpuDisplayDevice;
    let preferences = ResidentPreferences::default();
    let mut state = color_state::ColorState::default();
    let entry = |id: &str, value: u16| DisplayEntry {
        frequency: None,
        gpu_device: Some(GpuDisplayDevice {
            id: id.into(),
            name: id.into(),
            label: "GPU0".into(),
        }),
        id: DisplayId::Gpu,
        tone: Default::default(),
        power_state: None,
        fan_rows: None,
        color_value: Some(value),
        marker: "G".into(),
        digits: value.to_string(),
        text: String::new(),
        tooltip: String::new(),
    };
    let mut first = vec![entry("a", 90), entry("b", 10)];
    state.apply(&mut first, &preferences, None, None, None, None);
    assert_eq!(first[0].tone, UsageTone::Critical);
    let mut reordered = vec![entry("b", 10), entry("a", 89)];
    state.apply(&mut reordered, &preferences, None, None, None, None);
    assert_eq!(reordered[1].tone, UsageTone::Critical);
    let mut removed = vec![entry("b", 10)];
    state.apply(&mut removed, &preferences, None, None, None, None);
    let mut returned = vec![entry("a", 89), entry("b", 10)];
    state.apply(&mut returned, &preferences, None, None, None, None);
    assert_eq!(returned[0].tone, UsageTone::Warning);
}

#[cfg(windows)]
#[test]
fn windows_fans_use_aggregate_values_and_clear_missing_colors() {
    use crate::resident::preference_schema::FanDisplayMode;
    use mangodisk_core::system_resources::{metrics::MetricReading, readings::ResourceReadings};
    use mangodisk_platform::system_resources::fans::{FanSample, FanSpeed};
    let labels = labels::Labels::for_locale("en-US");
    let mut preferences = ResidentPreferences {
        fan_display: FanDisplayMode::Selected,
        fan_ids: vec![0, 1],
        ..Default::default()
    };
    let mut readings = ResourceReadings {
        observed_at_ms: 1000,
        fan_speed: MetricReading::ready(
            FanSample {
                fans: vec![
                    FanSpeed {
                        id: 0,
                        rpm: Some(4000.0),
                    },
                    FanSpeed {
                        id: 1,
                        rpm: Some(0.0),
                    },
                ],
            },
            1000,
        ),
        ..Default::default()
    };
    let mut state = DisplayState::default();
    for (mode, digits, tone) in [
        (FanDisplayMode::Selected, "4000", UsageTone::Critical),
        (FanDisplayMode::Average, "2000", UsageTone::Warning),
    ] {
        preferences.fan_display = mode;
        let mut entries = vec![fan::entry(&readings, &labels, &preferences)];
        state
            .colors
            .apply(&mut entries, &preferences, None, None, None, None);
        assert_eq!(entries[0].digits, digits);
        assert_eq!(entries[0].tone, tone);
        assert!(entries[0].fan_rows.is_none());
        assert!(entries[0].tooltip.contains(labels.text("gpuFanSpeed")));
        assert_eq!(entries[0].marker, "GFAN");
    }
    readings.fan_speed = MetricReading {
        status: mangodisk_core::system_resources::metrics::MetricStatus::Unsupported,
        ..Default::default()
    };
    let mut entries = vec![fan::entry(&readings, &labels, &preferences)];
    state
        .colors
        .apply(&mut entries, &preferences, None, None, None, None);
    assert_eq!(entries[0].digits, "—");
    assert_eq!(entries[0].tone, UsageTone::Normal);
}

#[test]
fn floating_power_does_not_cross_color_threshold_before_its_displayed_value() {
    use crate::resident::color_rules::{ColorRule, DisplayColor};
    use crate::resident::preference_schema::DisplayMetricId;
    use mangodisk_core::system_resources::{metrics::MetricReading, readings::ResourceReadings};
    use mangodisk_platform::system_resources::component_power::{
        ComponentPower, ComponentPowerSource,
    };
    let labels = labels::Labels::for_locale("en-US");
    let mut preferences = ResidentPreferences::default();
    preferences
        .metrics
        .iter_mut()
        .find(|metric| metric.id == DisplayMetricId::CpuPower)
        .unwrap()
        .enabled = true;
    let rule = preferences
        .color_rules
        .iter_mut()
        .find(|rule| rule.id == DisplayMetricId::CpuPower)
        .unwrap();
    rule.enabled = true;
    rule.rules = vec![
        ColorRule::Below {
            value: 7,
            color: DisplayColor::Green,
        },
        ColorRule::AtLeast {
            value: 7,
            color: DisplayColor::Red,
        },
    ];
    let values = ResourceReadings {
        observed_at_ms: 1000,
        cpu_power: MetricReading::ready(
            ComponentPower {
                watts: 6.7,
                source: ComponentPowerSource::IoReport,
            },
            1000,
        ),
        ..Default::default()
    };
    let mut menu = vec![power::component_power_entry(
        &values,
        &labels,
        DisplayMetricId::CpuPower,
    )];
    let mut state = DisplayState::default();
    state
        .colors
        .apply(&mut menu, &preferences, None, None, None, None);
    assert_eq!(menu[0].digits, "7");
    assert_eq!(menu[0].tone, UsageTone::Critical);
    let precision = power::floating_power_digits(&menu, &values);
    let mut floating = power::floating_entries(&menu, &precision);
    state
        .floating_colors
        .apply(&mut floating, &preferences, None, None, None, None);
    assert_eq!(floating[0].digits, "6.7");
    assert_eq!(
        floating[0].tone,
        UsageTone::Green,
        "6.7 W must stay below a 7 W threshold on a fresh surface"
    );
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[test]
#[ignore = "requires native Apple Silicon energy counters and a fractional observation"]
fn native_component_power_precision_uses_independent_surface_colors() {
    use crate::resident::{
        color_rules::{ColorRule, DisplayColor},
        preference_schema::DisplayMetricId,
    };
    use mangodisk_core::system_resources::{metrics::MetricReading, readings::ResourceReadings};
    use mangodisk_platform::system_resources::component_power::ComponentPowerReader;
    let labels = labels::Labels::for_locale("en-US");
    let mut reader = ComponentPowerReader::default();
    assert!(
        reader.read().is_none(),
        "native subscription establishes a baseline first"
    );
    for _ in 0..40 {
        std::thread::sleep(std::time::Duration::from_millis(200));
        let reading = reader
            .read()
            .expect("native power sample")
            .expect("readable component energy");
        let values = ResourceReadings {
            observed_at_ms: 1000,
            cpu_power: MetricReading::ready(reading, 1000),
            ..Default::default()
        };
        let mut menu = vec![power::component_power_entry(
            &values,
            &labels,
            DisplayMetricId::CpuPower,
        )];
        let precision = power::floating_power_digits(&menu, &values);
        let displayed = precision[&DisplayId::CpuPower].parse::<f64>().unwrap();
        let threshold = displayed.ceil() as u16;
        if displayed.fract() == 0.0 || menu[0].digits.parse::<u16>().ok() != Some(threshold) {
            continue;
        }
        let mut preferences = ResidentPreferences::default();
        preferences
            .metrics
            .iter_mut()
            .find(|metric| metric.id == DisplayMetricId::CpuPower)
            .unwrap()
            .enabled = true;
        let rule = preferences
            .color_rules
            .iter_mut()
            .find(|rule| rule.id == DisplayMetricId::CpuPower)
            .unwrap();
        rule.enabled = true;
        rule.rules = vec![
            ColorRule::Below {
                value: threshold,
                color: DisplayColor::Green,
            },
            ColorRule::AtLeast {
                value: threshold,
                color: DisplayColor::Red,
            },
        ];
        let mut state = DisplayState::default();
        state
            .colors
            .apply(&mut menu, &preferences, None, None, None, None);
        let mut floating = power::floating_entries(&menu, &precision);
        state
            .floating_colors
            .apply(&mut floating, &preferences, None, None, None, None);
        assert_eq!(menu[0].tone, UsageTone::Critical);
        assert_eq!(floating[0].tone, UsageTone::Green);
        println!("native_power_precision watts={:.5} threshold={} menu={} menu_tone={:?} floating={} floating_tone={:?}", values.cpu_power.value.as_ref().unwrap().watts, threshold, menu[0].digits, menu[0].tone, floating[0].digits, floating[0].tone);
        return;
    }
    panic!("no suitable fractional native power sample within eight seconds");
}
