//! Battery and system power presentation using the shared native precision rules.
use super::{
    display_entry::{DisplayEntry, DisplayId, GpuDisplayDevice},
    labels,
};
use crate::resident::preference_schema::DisplayMetricId;
use mangodisk_core::system_resources::{metrics::MetricStatus, readings::ResourceReadings};

use super::format::{displayed_power, power_digits};

/// Preserve validated fractional readings for the floating surface before compact rounding.
pub(super) fn floating_power_digits(
    entries: &[DisplayEntry],
    values: &ResourceReadings,
) -> std::collections::HashMap<DisplayId, String> {
    entries
        .iter()
        .filter(|entry| entry.digits != "—")
        .filter_map(|entry| {
            let watts = match entry.id {
                DisplayId::CpuPower => values.cpu_power.value.as_ref().map(|power| power.watts),
                DisplayId::GpuPower => values
                    .gpu_power
                    .value
                    .as_ref()
                    .map(|power| power.power.watts),
                DisplayId::SystemPower => {
                    values.system_power.value.as_ref().map(|power| power.watts)
                }
                DisplayId::BatteryPower => {
                    values.battery_power.value.as_ref().map(|power| power.watts)
                }
                DisplayId::ChargingPower => values
                    .charging_power
                    .value
                    .as_ref()
                    .map(|power| power.watts),
                _ => None,
            }?;
            (watts.is_finite() && watts >= 0.0).then(|| (entry.id, format!("{watts:.1}")))
        })
        .collect()
}

/// The decimal surface classifies its own displayed value, not the compact rounding.
pub(super) fn floating_entries(
    entries: &[DisplayEntry],
    digits: &std::collections::HashMap<DisplayId, String>,
) -> Vec<DisplayEntry> {
    entries
        .iter()
        .cloned()
        .map(|mut entry| {
            if let Some(value) = digits.get(&entry.id) {
                // These strings come only from this module's fixed-decimal formatter.
                // The integer part is sufficient for the existing whole-watt rules.
                entry.color_value = value
                    .split_once('.')
                    .and_then(|(whole, _)| whole.parse().ok());
                entry.digits = value.clone();
            }
            entry
        })
        .collect()
}

pub(super) fn component_power_entry(
    values: &ResourceReadings,
    labels: &labels::Labels,
    id: DisplayMetricId,
) -> DisplayEntry {
    let cpu = id == DisplayMetricId::CpuPower;
    let gpu_device = if cpu {
        None
    } else {
        values.gpu_power.value.as_ref().and_then(|power| {
            values
                .gpu_adapters
                .iter()
                .enumerate()
                .find(|(_, adapter)| adapter.id == power.adapter_id)
                .map(|(index, adapter)| GpuDisplayDevice {
                    id: adapter.id.clone(),
                    name: adapter.name.clone(),
                    label: if values.gpu_adapters.len() == 1 {
                        "GPU".into()
                    } else {
                        format!("GPU{index}")
                    },
                })
        })
    };
    let (status, at, watts) = if cpu {
        let reading = &values.cpu_power;
        (
            reading.status,
            reading.sampled_at_ms,
            reading.value.as_ref().map(|power| power.watts),
        )
    } else {
        let reading = &values.gpu_power;
        (
            reading.status,
            reading.sampled_at_ms,
            reading
                .value
                .as_ref()
                .filter(|_| gpu_device.is_some())
                .map(|power| power.power.watts),
        )
    };
    let watts = watts.filter(|watts| {
        status == MetricStatus::Ready
            && watts.is_finite()
            && (0.0..=1_000.0).contains(watts)
            && at
                .and_then(|at| values.observed_at_ms.checked_sub(at))
                .is_some_and(|age| age <= 5_000)
    });
    let metric_name = labels.text(if cpu { "cpuPower" } else { "gpuPower" });
    let name = gpu_device
        .as_ref()
        .filter(|device| device.label != "GPU")
        .map_or_else(
            || metric_name.to_string(),
            |device| format!("{} · {metric_name}", device.label),
        );
    let digits = watts.map(power_digits).unwrap_or_else(|| "—".into());
    let tooltip = if let Some(watts) = watts {
        format!(
            "{metric_name} {watts:.1}W · {}",
            labels.text("componentPowerHint")
        )
    } else {
        format!(
            "{metric_name} · {}",
            labels.status(if status == MetricStatus::Ready {
                MetricStatus::Stale
            } else {
                status
            })
        )
    };
    let tooltip = match &gpu_device {
        Some(device) => format!("{} · {tooltip}", device.name),
        None => tooltip,
    };
    DisplayEntry {
        frequency: None,
        gpu_device,
        id: if cpu {
            DisplayId::CpuPower
        } else {
            DisplayId::GpuPower
        },
        tone: Default::default(),
        power_state: None,
        fan_rows: None,
        color_value: watts.map(|watts| displayed_power(watts).floor() as u16),
        marker: "W".into(),
        text: if watts.is_some() {
            format!("{name} {digits}W")
        } else {
            format!("{name} —")
        },
        tooltip,
        digits,
    }
}

pub(super) fn system_power_entry(
    values: &ResourceReadings,
    labels: &labels::Labels,
) -> DisplayEntry {
    use mangodisk_platform::system_resources::power::{
        SYSTEM_POWER_FRESHNESS_MS, SYSTEM_POWER_MAX_WATTS,
    };
    let reading = &values.system_power;
    let value = reading
        .value
        .as_ref()
        .filter(|value| {
            reading.status == MetricStatus::Ready
                && value.watts.is_finite()
                && (0.0..=SYSTEM_POWER_MAX_WATTS).contains(&value.watts)
                && reading
                    .sampled_at_ms
                    .and_then(|at| values.observed_at_ms.checked_sub(at))
                    .is_some_and(|age| age <= SYSTEM_POWER_FRESHNESS_MS)
        })
        .map(|value| value.watts);
    let displayed = value.map(displayed_power);
    let digits = value.map(power_digits).unwrap_or_else(|| "—".into());
    let name = labels.text("systemPower");
    let text = if displayed.is_some() {
        format!("{name} {digits}W")
    } else {
        format!("{name} —")
    };
    let tooltip = if let Some(value) = value {
        format!("{name} {value:.1}W")
    } else {
        format!(
            "{name} · {}",
            labels.status(if reading.status == MetricStatus::Ready {
                MetricStatus::Stale
            } else {
                reading.status
            })
        )
    };
    DisplayEntry {
        frequency: None,
        gpu_device: None,
        id: DisplayId::SystemPower,
        tone: Default::default(),
        power_state: None,
        fan_rows: None,
        color_value: displayed.map(|value| value.floor() as u16),
        marker: "W".into(),
        digits,
        text,
        tooltip,
    }
}

pub(super) fn battery_entry(
    values: &ResourceReadings,
    labels: &labels::Labels,
    id: DisplayMetricId,
) -> DisplayEntry {
    use mangodisk_platform::system_resources::battery::BATTERY_FRESHNESS_MS;
    let temperature = id == DisplayMetricId::BatteryTemperature;
    let battery_power = id == DisplayMetricId::BatteryPower;
    let (status, sampled_at, value) = if temperature {
        (
            values.battery_temperature.status,
            values.battery_temperature.sampled_at_ms,
            values
                .battery_temperature
                .value
                .as_ref()
                .map(|value| value.celsius),
        )
    } else if battery_power {
        (
            values.battery_power.status,
            values.battery_power.sampled_at_ms,
            values.battery_power.value.as_ref().map(|value| value.watts),
        )
    } else {
        (
            values.charging_power.status,
            values.charging_power.sampled_at_ms,
            values
                .charging_power
                .value
                .as_ref()
                .map(|value| value.watts),
        )
    };
    let value = value.filter(|value| {
        values.battery_present != Some(false)
            && status == MetricStatus::Ready
            && value.is_finite()
            && (0.0..=if temperature { 100.0 } else { 300.0 }).contains(value)
            && sampled_at
                .and_then(|at| values.observed_at_ms.checked_sub(at))
                .is_some_and(|age| age <= BATTERY_FRESHNESS_MS)
    });
    let displayed = value.map(|value| {
        if temperature {
            value.round()
        } else {
            displayed_power(value)
        }
    });
    let digits = value
        .map(|value| {
            if temperature {
                format!("{:.0}", value.round())
            } else {
                power_digits(value)
            }
        })
        .unwrap_or_else(|| "—".into());
    let unit = if temperature { "°C" } else { "W" };
    let name = labels.text(if temperature {
        "batteryTemperature"
    } else if battery_power {
        "batteryPower"
    } else {
        "chargingPower"
    });
    let power_state = battery_power
        .then(|| values.battery_power.value.as_ref().map(|value| value.state))
        .flatten()
        .filter(|_| value.is_some());
    let state_name = power_state.map(|state| {
        use mangodisk_platform::system_resources::battery::BatteryPowerState;
        labels.text(match state {
            BatteryPowerState::Charging => "batteryCharging",
            BatteryPowerState::Discharging => "batteryDischarging",
            BatteryPowerState::Idle => "batteryIdle",
        })
    });
    let format_reading = |digits: &str| {
        let reading = format!("{name} {digits}{unit}");
        if let Some(state) = state_name {
            format!("{reading} · {state}")
        } else {
            reading
        }
    };
    let text = if value.is_some() {
        format_reading(&digits)
    } else {
        format!("{name} —")
    };
    let tooltip = if let Some(value) = value {
        if temperature {
            text.clone()
        } else {
            format_reading(&format!("{value:.1}"))
        }
    } else {
        format!(
            "{name} · {}",
            if values.battery_present == Some(false) {
                labels.text("noBattery")
            } else {
                labels.status(if status == MetricStatus::Ready {
                    MetricStatus::Stale
                } else {
                    status
                })
            }
        )
    };
    DisplayEntry {
        frequency: None,
        gpu_device: None,
        id: if temperature {
            DisplayId::BatteryTemperature
        } else if battery_power {
            DisplayId::BatteryPower
        } else {
            DisplayId::ChargingPower
        },
        tone: Default::default(),
        power_state,
        fan_rows: None,
        color_value: displayed.map(|value| value.floor() as u16),
        marker: unit.into(),
        digits,
        text,
        tooltip,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mangodisk_core::system_resources::metrics::MetricReading;
    use mangodisk_platform::system_resources::{
        component_power::{ComponentPower, ComponentPowerSource, GpuPower},
        gpu::GpuAdapter,
    };
    #[test]
    fn hybrid_power_entries_preserve_the_sampler_device_identity() {
        let labels = labels::Labels::for_locale("en-US");
        let mut values = ResourceReadings {
            observed_at_ms: 1000,
            ..Default::default()
        };
        values.gpu_adapters = vec![
            GpuAdapter {
                id: "amd".into(),
                name: "Radeon Pro".into(),
            },
            GpuAdapter {
                id: "intel".into(),
                name: "Intel Graphics".into(),
            },
        ];
        values.gpu = MetricReading::ready(
            mangodisk_core::system_resources::gpu::GpuUsage {
                adapter_id: "amd".into(),
                adapter_name: "Radeon Pro".into(),
                used_percent: 80.0,
                details: None,
            },
            1000,
        );
        values.gpu_power = MetricReading::ready(
            GpuPower {
                adapter_id: "intel".into(),
                power: ComponentPower {
                    watts: 0.4,
                    source: ComponentPowerSource::AppleSmc,
                },
            },
            1000,
        );
        let entry = component_power_entry(&values, &labels, DisplayMetricId::GpuPower);
        let device = entry
            .gpu_device
            .as_ref()
            .expect("GPU power must retain the sampled adapter");
        assert_eq!(device.id, "intel");
        assert_eq!(device.label, "GPU1");
        #[cfg(target_os = "macos")]
        assert_eq!(
            super::super::macos_presentation::columns(std::slice::from_ref(&entry), false)[0].top,
            "GPU1"
        );
        assert!(entry.tooltip.contains("Intel Graphics"));
        assert!(entry.text.contains("GPU1"));
        values.gpu_power.value.as_mut().unwrap().adapter_id = "amd".into();
        let entry = component_power_entry(&values, &labels, DisplayMetricId::GpuPower);
        assert_eq!(entry.gpu_device.as_ref().unwrap().label, "GPU0");
        assert!(entry.tooltip.contains("Radeon Pro"));
        values.gpu_adapters.remove(1);
        let entry = component_power_entry(&values, &labels, DisplayMetricId::GpuPower);
        assert_eq!(entry.gpu_device.as_ref().unwrap().label, "GPU");
        assert!(!entry.text.contains("GPU0"));
    }
    #[test]
    fn floating_power_keeps_tenths_without_changing_compact_or_missing_readings() {
        let labels = labels::Labels::for_locale("en-US");
        let mut values = ResourceReadings {
            observed_at_ms: 1000,
            ..Default::default()
        };
        for watts in [0.0, 0.4, 6.1, 6.2, 999.9] {
            values.cpu_power = MetricReading::ready(
                ComponentPower {
                    watts,
                    source: ComponentPowerSource::IoReport,
                },
                1000,
            );
            let entry = component_power_entry(&values, &labels, DisplayMetricId::CpuPower);
            let digits = floating_power_digits(std::slice::from_ref(&entry), &values);
            assert_eq!(
                digits.get(&DisplayId::CpuPower).unwrap(),
                &format!("{watts:.1}")
            );
            assert_eq!(entry.digits, power_digits(watts));
        }
        values.observed_at_ms = 6001;
        let entry = component_power_entry(&values, &labels, DisplayMetricId::CpuPower);
        assert_eq!(entry.digits, "—");
        assert!(floating_power_digits(&[entry], &values).is_empty());
        values.observed_at_ms = 1000;
        values.system_power = MetricReading::ready(
            mangodisk_platform::system_resources::power::SystemPower { watts: 3000.0 },
            1000,
        );
        let entry = system_power_entry(&values, &labels);
        assert_eq!(
            floating_power_digits(&[entry], &values)
                .get(&DisplayId::SystemPower)
                .unwrap(),
            "3000.0"
        );
    }

    #[test]
    fn component_entries_preserve_freshness_identity_and_panel_destination() {
        let labels = labels::Labels::for_locale("en-US");
        let mut values = ResourceReadings {
            observed_at_ms: 1000,
            ..Default::default()
        };
        let power = |watts| ComponentPower {
            watts,
            source: ComponentPowerSource::IoReport,
        };
        values.cpu_power = MetricReading::ready(power(0.0), 1000);
        values.gpu_power = MetricReading::ready(
            GpuPower {
                adapter_id: "apple".into(),
                power: power(25.5),
            },
            1000,
        );
        let cpu = component_power_entry(&values, &labels, DisplayMetricId::CpuPower);
        assert_ne!(cpu.digits, "—");
        assert!(cpu.text.contains("CPU"));
        assert_eq!(
            cpu.id.metric(),
            Some(mangodisk_core::system_resources::metrics::MetricId::Cpu)
        );
        assert_eq!(
            component_power_entry(&values, &labels, DisplayMetricId::GpuPower).digits,
            "—"
        );
        assert_eq!(
            cpu.id.page(),
            Some(crate::resident::panel_page::PanelPage::Cpu)
        );
        let gpu = component_power_entry(&values, &labels, DisplayMetricId::GpuPower);
        assert_eq!(
            gpu.id.page(),
            Some(crate::resident::panel_page::PanelPage::Gpu)
        );
        values.gpu_adapters.push(GpuAdapter {
            id: "apple".into(),
            name: "Apple GPU".into(),
        });
        assert_ne!(
            component_power_entry(&values, &labels, DisplayMetricId::GpuPower).digits,
            "—"
        );
        values.observed_at_ms = 6001;
        assert_eq!(
            component_power_entry(&values, &labels, DisplayMetricId::CpuPower).digits,
            "—"
        );
        values.observed_at_ms = 999;
        assert_eq!(
            component_power_entry(&values, &labels, DisplayMetricId::GpuPower).digits,
            "—"
        );
    }
}
