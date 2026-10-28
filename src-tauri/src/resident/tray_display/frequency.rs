//! Optional live CPU clocks; native surfaces abbreviate units without changing the value.
use super::{
    display_entry::{DisplayEntry, DisplayId, FrequencyDisplayState, FrequencyPresentation},
    labels::Labels,
};
use crate::resident::preference_schema::DisplayMetricId;
use mangodisk_core::system_resources::{
    metrics::{MetricId, MetricStatus},
    readings::ResourceReadings,
};
use mangodisk_platform::system_resources::cpu::details::CpuFrequencyClass;

pub(super) fn entry(
    id: DisplayMetricId,
    values: &ResourceReadings,
    labels: &Labels,
) -> DisplayEntry {
    let (display, class, marker, key) = match id {
        DisplayMetricId::CpuFrequency => (
            DisplayId::CpuFrequency,
            CpuFrequencyClass::Average,
            "CPU",
            "cpuFrequency",
        ),
        DisplayMetricId::CpuEfficiencyFrequency => (
            DisplayId::CpuEfficiencyFrequency,
            CpuFrequencyClass::Efficiency,
            "CPUE",
            "cpuEfficiencyFrequency",
        ),
        DisplayMetricId::CpuPerformanceFrequency => (
            DisplayId::CpuPerformanceFrequency,
            CpuFrequencyClass::Performance,
            "CPUP",
            "cpuPerformanceFrequency",
        ),
        DisplayMetricId::CpuSuperFrequency => (
            DisplayId::CpuSuperFrequency,
            CpuFrequencyClass::Super,
            "CPUS",
            "cpuSuperFrequency",
        ),
        _ => unreachable!("frequency entry requires a frequency display item"),
    };
    let reading = &values.cpu_frequency;
    let classes = values
        .cpu_identity
        .as_ref()
        .map(|identity| &identity.frequency_classes);
    let supported = classes.is_some_and(|classes| classes.contains(&class));
    // Installed capabilities keep labels stable when a cluster is idle or hidden.
    let marker = if supported && classes.is_some_and(|classes| classes.len() == 1) {
        "CPU"
    } else {
        marker
    };
    let fresh = reading.status == MetricStatus::Ready
        && reading.sampled_at_ms.is_some_and(|sampled| {
            sampled <= values.observed_at_ms
                && values.observed_at_ms - sampled <= MetricId::Cpu.freshness_ms()
        });
    let raw_mhz = reading
        .value
        .as_ref()
        .filter(|_| fresh && supported)
        .and_then(|value| match class {
            CpuFrequencyClass::Average => value.average_mhz,
            CpuFrequencyClass::Efficiency => value.efficiency_mhz,
            CpuFrequencyClass::Performance => value.performance_mhz,
            CpuFrequencyClass::Super => value.super_mhz,
        });
    let mhz = raw_mhz.filter(|mhz| mhz.is_finite() && (1.0..=20_000.0).contains(mhz));
    let activity = reading
        .value
        .as_ref()
        .filter(|_| supported && fresh)
        .and_then(|value| {
            value
                .activity
                .iter()
                .find(|activity| activity.class == class)
        });
    let idle = raw_mhz.is_none() && activity.is_some_and(|activity| activity.is_idle());
    let idle_label = labels.text("frequencyIdle");
    let state = if mhz.is_some() {
        FrequencyDisplayState::Active
    } else if idle {
        FrequencyDisplayState::Idle
    } else {
        FrequencyDisplayState::Unavailable
    };
    let digits = mhz
        .map(|mhz| format!("{:.1}", mhz / 1000.0))
        .unwrap_or_else(|| {
            if idle {
                idle_label.into()
            } else {
                "—".into()
            }
        });
    let name = labels.text(key);
    let mut tooltip = if mhz.is_some() {
        format!("{name} {digits} GHz")
    } else if idle {
        format!(
            "{name} · {idle_label}\n{}",
            labels.text("frequencyIdleReason")
        )
    } else if supported && fresh && reading.value.is_some() && raw_mhz.is_none() {
        format!("{name} · {}", labels.text("frequencyNoSample"))
    } else {
        let status = if fresh && raw_mhz.is_some() {
            MetricStatus::Failed
        } else if values.cpu_identity.is_some() && !supported {
            MetricStatus::Unsupported
        } else if reading.status == MetricStatus::Ready {
            MetricStatus::Stale
        } else {
            reading.status
        };
        format!("{name} · {}", labels.status(status))
    };
    if let Some(activity) = activity {
        let states = [
            (labels.text("frequencyActive"), activity.active_ratio),
            ("IDLE", activity.idle_ratio),
            ("DOWN", activity.down_ratio),
            ("OFF", activity.off_ratio),
        ]
        .into_iter()
        .filter(|(_, ratio)| *ratio > 0.0)
        .map(|(name, ratio)| format!("{name} {:.1}%", ratio * 100.0))
        .collect::<Vec<_>>()
        .join(" · ");
        if !states.is_empty() {
            tooltip.push_str(&format!("\n{}: {states}", labels.text("frequencyInterval")));
        }
    }
    DisplayEntry {
        id: display,
        frequency: Some(FrequencyPresentation {
            idle_label: idle_label.into(),
            state,
        }),
        gpu_device: None,
        tone: Default::default(),
        power_state: None,
        fan_rows: None,
        color_value: None,
        marker: marker.into(),
        text: if mhz.is_some() {
            format!("{marker} {digits}G")
        } else {
            format!("{marker} {digits}")
        },
        digits,
        tooltip,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mangodisk_core::system_resources::metrics::MetricReading;
    use mangodisk_platform::system_resources::cpu::details::{
        CpuFrequency, CpuFrequencySource, CpuIdentity,
    };

    fn readings() -> ResourceReadings {
        ResourceReadings {
            observed_at_ms: 1000,
            cpu_identity: Some(CpuIdentity {
                frequency_classes: vec![
                    CpuFrequencyClass::Average,
                    CpuFrequencyClass::Efficiency,
                    CpuFrequencyClass::Performance,
                ],
                ..Default::default()
            }),
            cpu_frequency: MetricReading::ready(
                CpuFrequency {
                    average_mhz: Some(3850.0),
                    efficiency_mhz: Some(1500.0),
                    performance_mhz: Some(3200.0),
                    super_mhz: None,
                    source: CpuFrequencySource::WindowsPerformance,
                    activity: Vec::new(),
                },
                1000,
            ),
            ..Default::default()
        }
    }
    #[test]
    fn confirmed_idle_and_failed_sampling_never_share_a_display_value() {
        use mangodisk_platform::system_resources::cpu::details::CpuFrequencyActivity;
        for locale in [
            "en-US", "zh-CN", "zh-TW", "ja-JP", "ko-KR", "de-DE", "es-ES", "fr-FR", "pt-BR",
            "ru-RU", "tr-TR", "uk-UA",
        ] {
            let labels = Labels::for_locale(locale);
            let mut values = readings();
            let value = values.cpu_frequency.value.as_mut().unwrap();
            value.efficiency_mhz = None;
            value.activity = vec![CpuFrequencyActivity {
                class: CpuFrequencyClass::Efficiency,
                active_ratio: 0.0,
                idle_ratio: 0.25,
                down_ratio: 0.75,
                off_ratio: 0.0,
            }];
            let idle = entry(DisplayMetricId::CpuEfficiencyFrequency, &values, &labels);
            assert_eq!(idle.digits, labels.text("frequencyIdle"));
            assert_eq!(
                idle.frequency.as_ref().unwrap().state,
                FrequencyDisplayState::Idle
            );
            assert!(idle.tooltip.contains("IDLE 25.0%"));
            assert!(idle.tooltip.contains("DOWN 75.0%"));
            assert!(!idle.text.ends_with('G'));
            for status in [
                MetricStatus::Failed,
                MetricStatus::Stale,
                MetricStatus::Loading,
            ] {
                values.cpu_frequency.status = status;
                let failed = entry(DisplayMetricId::CpuEfficiencyFrequency, &values, &labels);
                assert_eq!(failed.digits, "—");
                assert!(!failed.tooltip.contains("IDLE"));
            }
            values.cpu_frequency.status = MetricStatus::Ready;
            values.cpu_frequency.value.as_mut().unwrap().efficiency_mhz = Some(2400.0);
            let active = entry(DisplayMetricId::CpuEfficiencyFrequency, &values, &labels);
            assert_eq!(active.digits, "2.4");
            assert_eq!(
                idle.frequency.unwrap().idle_label,
                active.frequency.unwrap().idle_label
            );
        }
    }

    #[test]
    fn frequency_units_and_core_classes_keep_their_meaning() {
        for locale in [
            "en-US", "zh-CN", "zh-TW", "ja-JP", "ko-KR", "de-DE", "es-ES", "fr-FR", "pt-BR",
            "ru-RU", "tr-TR", "uk-UA",
        ] {
            let labels = Labels::for_locale(locale);
            for (id, expected, marker) in [
                (DisplayMetricId::CpuFrequency, "3.9", "CPU"),
                (DisplayMetricId::CpuEfficiencyFrequency, "1.5", "CPUE"),
                (DisplayMetricId::CpuPerformanceFrequency, "3.2", "CPUP"),
                (DisplayMetricId::CpuSuperFrequency, "4.1", "CPUS"),
            ] {
                let mut values = readings();
                if id == DisplayMetricId::CpuSuperFrequency {
                    values
                        .cpu_identity
                        .as_mut()
                        .unwrap()
                        .frequency_classes
                        .push(CpuFrequencyClass::Super);
                    values.cpu_frequency.value.as_mut().unwrap().super_mhz = Some(4060.0);
                }
                let value = entry(id, &values, &labels);
                assert_eq!(value.digits, expected);
                assert_eq!(value.text, format!("{marker} {expected}G"));
                assert!(value.tooltip.ends_with(&format!("{expected} GHz")));
                assert!(!value.tooltip.contains('—'));
                assert!(value.color_value.is_none());
            }
        }
    }
    #[test]
    fn installed_frequency_types_control_labels_without_shortening_tooltips() {
        for (id, class, key, typed_marker) in [
            (
                DisplayMetricId::CpuFrequency,
                CpuFrequencyClass::Average,
                "cpuFrequency",
                "CPU",
            ),
            (
                DisplayMetricId::CpuEfficiencyFrequency,
                CpuFrequencyClass::Efficiency,
                "cpuEfficiencyFrequency",
                "CPUE",
            ),
            (
                DisplayMetricId::CpuPerformanceFrequency,
                CpuFrequencyClass::Performance,
                "cpuPerformanceFrequency",
                "CPUP",
            ),
            (
                DisplayMetricId::CpuSuperFrequency,
                CpuFrequencyClass::Super,
                "cpuSuperFrequency",
                "CPUS",
            ),
        ] {
            for locale in [
                "en-US", "zh-CN", "zh-TW", "ja-JP", "ko-KR", "de-DE", "es-ES", "fr-FR", "pt-BR",
                "ru-RU", "tr-TR", "uk-UA",
            ] {
                let labels = Labels::for_locale(locale);
                let mut values = readings();
                values.cpu_identity.as_mut().unwrap().frequency_classes = vec![class];
                values.cpu_frequency.value.as_mut().unwrap().super_mhz = Some(4060.0);
                let ready = entry(id, &values, &labels);
                assert_eq!(ready.marker, "CPU");
                assert_eq!(ready.text, format!("CPU {}G", ready.digits));
                assert!(ready.tooltip.starts_with(labels.text(key)));
                for status in [
                    MetricStatus::Loading,
                    MetricStatus::Stale,
                    MetricStatus::Failed,
                ] {
                    values.cpu_frequency.status = status;
                    let unavailable = entry(id, &values, &labels);
                    assert_eq!(unavailable.marker, ready.marker);
                    assert_eq!(unavailable.digits, "—");
                    assert!(unavailable.tooltip.starts_with(labels.text(key)));
                }
                values.cpu_identity = None;
                assert_eq!(entry(id, &values, &labels).marker, typed_marker);
            }
        }
        let labels = Labels::for_locale("en-US");
        let mut values = readings();
        values.cpu_identity.as_mut().unwrap().frequency_classes = vec![
            CpuFrequencyClass::Efficiency,
            CpuFrequencyClass::Performance,
        ];
        values.cpu_frequency.value.as_mut().unwrap().efficiency_mhz = None;
        assert_eq!(
            entry(DisplayMetricId::CpuEfficiencyFrequency, &values, &labels).marker,
            "CPUE"
        );
        assert_eq!(
            entry(DisplayMetricId::CpuPerformanceFrequency, &values, &labels).marker,
            "CPUP"
        );
        values.cpu_identity.as_mut().unwrap().frequency_classes =
            vec![CpuFrequencyClass::Performance];
        assert_eq!(
            entry(DisplayMetricId::CpuEfficiencyFrequency, &values, &labels).marker,
            "CPUE"
        );
    }

    #[test]
    fn unavailable_idle_expired_and_invalid_clocks_keep_an_empty_slot() {
        let labels = Labels::for_locale("en-US");
        let check = |values: &ResourceReadings| {
            let value = entry(DisplayMetricId::CpuFrequency, values, &labels);
            assert_eq!(value.digits, "—");
            assert!(value.text.ends_with(" —"));
            assert!(!value.tooltip.contains("0.00"));
        };
        let mut values = readings();
        values.observed_at_ms += MetricId::Cpu.freshness_ms() + 1;
        check(&values);
        values = readings();
        values.cpu_frequency.sampled_at_ms = Some(1001);
        check(&values);
        for mhz in [
            None,
            Some(0.0),
            Some(f64::NAN),
            Some(f64::INFINITY),
            Some(20_001.0),
        ] {
            values = readings();
            values.cpu_frequency.value.as_mut().unwrap().average_mhz = mhz;
            check(&values);
        }
        for status in [
            MetricStatus::Loading,
            MetricStatus::Stale,
            MetricStatus::Failed,
            MetricStatus::Unsupported,
        ] {
            values = readings();
            values.cpu_frequency.status = status;
            check(&values);
        }
        values = readings();
        values
            .cpu_identity
            .as_mut()
            .unwrap()
            .frequency_classes
            .clear();
        check(&values);
        values = readings();
        values.cpu_frequency.value.as_mut().unwrap().efficiency_mhz = None;
        let idle = entry(DisplayMetricId::CpuEfficiencyFrequency, &values, &labels);
        assert_eq!(idle.digits, "—");
        assert!(idle.tooltip.ends_with(labels.text("frequencyNoSample")));
        assert!(!idle.tooltip.ends_with(labels.status(MetricStatus::Stale)));
        let absent = entry(DisplayMetricId::CpuSuperFrequency, &readings(), &labels);
        assert_eq!(absent.digits, "—");
        assert!(absent
            .tooltip
            .ends_with(labels.status(MetricStatus::Unsupported)));
    }
}
