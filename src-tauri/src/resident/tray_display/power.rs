//! Battery and system power presentation using the shared native precision rules.
use super::{
    display_entry::{DisplayEntry, DisplayId},
    labels,
};
use crate::resident::preference_schema::DisplayMetricId;
use mangodisk_core::system_resources::{metrics::MetricStatus, readings::ResourceReadings};

use super::format::{displayed_power, power_digits};

pub(super) fn component_power_entry(
    values: &ResourceReadings,
    labels: &labels::Labels,
    id: DisplayMetricId,
) -> DisplayEntry {
    let cpu = id == DisplayMetricId::CpuPower;
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
                .filter(|power| {
                    values
                        .gpu_adapters
                        .iter()
                        .any(|adapter| adapter.id == power.adapter_id)
                })
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
    let name = labels.text(if cpu { "cpuPower" } else { "gpuPower" });
    let digits = watts.map(power_digits).unwrap_or_else(|| "—".into());
    DisplayEntry {
        frequency: None,
        gpu_device: None,
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
        tooltip: if let Some(watts) = watts {
            format!("{name} {watts:.1}W · {}", labels.text("componentPowerHint"))
        } else {
            format!(
                "{name} · {}",
                labels.status(if status == MetricStatus::Ready {
                    MetricStatus::Stale
                } else {
                    status
                })
            )
        },
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
