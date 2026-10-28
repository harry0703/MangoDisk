//! Shared text for native resident surfaces.
use mangodisk_core::system_resources::{
    metrics::{MetricId, MetricStatus},
    readings::ResourceReadings,
};

#[cfg(test)]
use super::format::displayed_power;
use super::{
    display_entry::{DisplayEntry, DisplayId},
    format::{byte_text, byte_text_with_precision, compact_rate, rate_title},
    power::{battery_entry, component_power_entry, system_power_entry},
    temperature::{disk_temperature_entry, gpu_temperature_entry, temperature_entry},
};
use crate::resident::{preference_schema::DisplayMetricId, preferences::ResidentPreferences};

/// Linux AppIndicator exposes frequently changing status through its title
/// label. Keep the icon stable and compose only the enabled metric fields.
#[cfg(any(target_os = "linux", test))]
pub fn indicator_title(
    entries: &[DisplayEntry],
    preferences: &ResidentPreferences,
) -> Option<String> {
    if preferences.linux_display_mode
        == crate::resident::preference_schema::LinuxDisplayMode::Floating
    {
        return None;
    }
    let compact = preferences.taskbar_compact;
    let title = entries
        .iter()
        .filter_map(|entry| {
            let text = if compact {
                match entry.id {
                    DisplayId::Cpu | DisplayId::Gpu | DisplayId::Memory | DisplayId::Disk => {
                        format!("{}{}%", entry.marker, entry.digits)
                    }
                    DisplayId::Upload | DisplayId::Download => {
                        let mut marker = entry.marker.chars();
                        let direction = marker.next()?;
                        format!("{direction}{}{}", entry.digits, marker.as_str())
                    }
                    DisplayId::CpuFrequency
                    | DisplayId::CpuEfficiencyFrequency
                    | DisplayId::CpuPerformanceFrequency
                    | DisplayId::CpuSuperFrequency
                    | DisplayId::App
                    | DisplayId::CpuTemperature
                    | DisplayId::GpuTemperature
                    | DisplayId::DiskTemperature
                    | DisplayId::BatteryTemperature
                    | DisplayId::ChargingPower
                    | DisplayId::BatteryPower
                    | DisplayId::CpuPower
                    | DisplayId::GpuPower
                    | DisplayId::SystemPower
                    | DisplayId::FanSpeed => entry.text.clone(),
                }
            } else {
                entry.text.trim().to_string()
            };
            (!text.is_empty()).then_some(text)
        })
        .collect::<Vec<_>>()
        .join(if compact { " " } else { "  " });
    (!title.is_empty()).then_some(title)
}

pub fn desired(preferences: &ResidentPreferences) -> Vec<DisplayId> {
    if !preferences.enabled {
        return Vec::new();
    }
    DisplayId::ALL
        .into_iter()
        .filter(|id| match id {
            DisplayId::CpuFrequency => preferences.shows_display(DisplayMetricId::CpuFrequency),
            DisplayId::CpuEfficiencyFrequency => {
                preferences.shows_display(DisplayMetricId::CpuEfficiencyFrequency)
            }
            DisplayId::CpuPerformanceFrequency => {
                preferences.shows_display(DisplayMetricId::CpuPerformanceFrequency)
            }
            DisplayId::CpuSuperFrequency => {
                preferences.shows_display(DisplayMetricId::CpuSuperFrequency)
            }
            DisplayId::CpuTemperature => preferences.shows_cpu_temperature(),
            DisplayId::GpuTemperature => preferences.shows_gpu_temperature(),
            DisplayId::DiskTemperature => preferences.shows_disk_temperature(),
            DisplayId::BatteryTemperature => {
                preferences.shows_battery_item(DisplayMetricId::BatteryTemperature)
            }
            DisplayId::CpuPower => preferences.shows_display(DisplayMetricId::CpuPower),
            DisplayId::GpuPower => preferences.shows_display(DisplayMetricId::GpuPower),
            DisplayId::SystemPower => preferences.shows_system_power(),
            DisplayId::FanSpeed => preferences.shows_fan_speed(),
            DisplayId::ChargingPower => {
                preferences.shows_battery_item(DisplayMetricId::ChargingPower)
            }
            DisplayId::BatteryPower => {
                preferences.shows_battery_item(DisplayMetricId::BatteryPower)
            }
            id => match id.metric() {
                Some(metric) => preferences.shows(metric),
                None => preferences.effective_icon(),
            },
        })
        .collect()
}

#[cfg(any(windows, test))]
pub fn windows_desired(preferences: &ResidentPreferences, taskbar_active: bool) -> Vec<DisplayId> {
    if !preferences.enabled {
        return Vec::new();
    }
    if taskbar_active
        || preferences.windows_display_mode
            == crate::resident::preference_schema::WindowsDisplayMode::Floating
    {
        if preferences.effective_icon() {
            vec![DisplayId::App]
        } else {
            Vec::new()
        }
    } else {
        desired(preferences)
    }
}

pub fn entries(
    preferences: &ResidentPreferences,
    values: &ResourceReadings,
    labels: &super::labels::Labels,
    base: f64,
) -> Vec<DisplayEntry> {
    let mut entries = Vec::new();
    for metric in &preferences.metrics {
        if !metric.enabled {
            continue;
        }
        if metric.id.frequency_class().is_some() && metric.id.supported_on_platform() {
            entries.push(super::frequency::entry(metric.id, values, labels));
            continue;
        }
        if !metric.id.supported_on_platform() {
            entries.push(platform_placeholder(metric.id, labels));
            continue;
        }
        if matches!(
            metric.id,
            DisplayMetricId::Gpu | DisplayMetricId::GpuTemperature
        ) {
            let temperature = metric.id == DisplayMetricId::GpuTemperature;
            let selection = if temperature
                && matches!(
                    preferences.gpu_temperature_display,
                    crate::resident::preference_schema::GpuDisplaySelection::Automatic
                ) {
                &preferences.gpu_display
            } else if temperature {
                &preferences.gpu_temperature_display
            } else {
                &preferences.gpu_display
            };
            if (!values.gpu_devices.is_empty()
                || matches!(
                    selection,
                    crate::resident::preference_schema::GpuDisplaySelection::Selected { .. }
                ))
                && (!temperature || preferences.shows_gpu_temperature())
            {
                entries.extend(super::gpu::entries(values, labels, temperature, selection));
                continue;
            }
        }

        if metric.id == DisplayMetricId::CpuTemperature {
            if preferences.shows_cpu_temperature() {
                entries.push(temperature_entry(values, labels));
            }
            continue;
        }
        if metric.id == DisplayMetricId::DiskTemperature {
            if preferences.shows_disk_temperature() {
                entries.push(disk_temperature_entry(
                    values,
                    labels,
                    preferences.disk_volume.as_deref(),
                    preferences.disk_temperature_device.as_deref(),
                ));
            }
            continue;
        }
        if metric.id == DisplayMetricId::GpuTemperature {
            if preferences.shows_gpu_temperature() {
                entries.push(gpu_temperature_entry(
                    values,
                    labels,
                    values
                        .gpu
                        .value
                        .as_ref()
                        .filter(|_| values.gpu.status == MetricStatus::Ready)
                        .map(|gpu| gpu.adapter_id.as_str()),
                ));
            }
            continue;
        }
        if matches!(
            metric.id,
            DisplayMetricId::BatteryTemperature
                | DisplayMetricId::ChargingPower
                | DisplayMetricId::BatteryPower
        ) {
            if preferences.shows_battery_item(metric.id) {
                entries.push(battery_entry(values, labels, metric.id));
            }
            continue;
        }
        if metric.id == DisplayMetricId::FanSpeed {
            if preferences.shows_fan_speed() {
                entries.push(super::fan::entry(values, labels, preferences));
            }
            continue;
        }
        if matches!(
            metric.id,
            DisplayMetricId::CpuPower | DisplayMetricId::GpuPower
        ) {
            entries.push(component_power_entry(values, labels, metric.id));
            continue;
        }
        if metric.id == DisplayMetricId::SystemPower {
            if preferences.shows_system_power() {
                entries.push(system_power_entry(values, labels));
            }
            continue;
        }
        let Some(metric_id) = metric.id.metric() else {
            continue;
        };
        let (ids, status) = match metric_id {
            MetricId::Cpu => (vec![DisplayId::Cpu], values.cpu.status),
            MetricId::Gpu => (vec![DisplayId::Gpu], values.gpu.status),
            MetricId::Memory => (vec![DisplayId::Memory], values.memory.status),
            MetricId::Network => (
                vec![DisplayId::Upload, DisplayId::Download],
                values.network.status,
            ),
            MetricId::Disk => (vec![DisplayId::Disk], values.disk.status),
        };
        for id in ids {
            let (marker, value) = match id {
                DisplayId::Cpu => (
                    "C",
                    values.cpu.value.as_ref().map(|value| value.used_percent),
                ),
                DisplayId::Gpu => (
                    "G",
                    values.gpu.value.as_ref().map(|value| value.used_percent),
                ),
                DisplayId::Memory => (
                    "M",
                    values
                        .memory
                        .value
                        .as_ref()
                        .map(|value| f64::from(value.memory.used_percent)),
                ),
                DisplayId::Disk => (
                    "D",
                    values.disk.value.as_ref().map(|value| value.used_percent),
                ),
                DisplayId::Upload => (
                    "↑",
                    values
                        .network
                        .value
                        .as_ref()
                        .map(|value| value.transmitted_bytes_per_second),
                ),
                DisplayId::Download => (
                    "↓",
                    values
                        .network
                        .value
                        .as_ref()
                        .map(|value| value.received_bytes_per_second),
                ),
                DisplayId::CpuFrequency
                | DisplayId::CpuEfficiencyFrequency
                | DisplayId::CpuPerformanceFrequency
                | DisplayId::CpuSuperFrequency
                | DisplayId::App
                | DisplayId::CpuTemperature
                | DisplayId::GpuTemperature
                | DisplayId::DiskTemperature
                | DisplayId::BatteryTemperature
                | DisplayId::ChargingPower
                | DisplayId::BatteryPower
                | DisplayId::CpuPower
                | DisplayId::GpuPower
                | DisplayId::SystemPower
                | DisplayId::FanSpeed => {
                    unreachable!()
                }
            };
            // Invalid samples must not become NaN%, infinity, or a misleading
            // valid-looking rate. Keep the normal unavailable presentation.
            let status = if status == MetricStatus::Ready
                && value.is_some_and(|value| {
                    !value.is_finite() || (metric_id == MetricId::Network && value < 0.0)
                }) {
                MetricStatus::Failed
            } else {
                status
            };
            let name = labels.metric(metric_id);
            if status != MetricStatus::Ready || value.is_none() {
                entries.push(DisplayEntry {
                    frequency: None,
                    gpu_device: None,
                    tone: Default::default(),
                    power_state: None,
                    fan_rows: None,
                    color_value: None,
                    id,
                    marker: marker.into(),
                    digits: "—".into(),
                    text: format!(
                        "{} —",
                        match id {
                            DisplayId::Cpu => "CPU",
                            DisplayId::Gpu => "GPU",
                            DisplayId::Memory => "MEM",
                            DisplayId::Disk => "DISK",
                            _ => marker,
                        }
                    ),
                    tooltip: format!("{name} · {}", labels.status(status)),
                });
                continue;
            }
            let value = value.unwrap_or_default();
            let entry = if metric_id == MetricId::Network {
                let (digits, unit) = compact_rate(value, base);
                let speed = byte_text(value, base);
                let interface = values
                    .network
                    .value
                    .as_ref()
                    .map(|value| value.interface.name.as_str())
                    .unwrap_or("");
                DisplayEntry {
                    frequency: None,
                    gpu_device: None,
                    tone: Default::default(),
                    power_state: None,
                    fan_rows: None,
                    color_value: None,
                    id,
                    marker: format!("{marker}{unit}"),
                    digits,
                    text: format!("{marker}{}", rate_title(value, base)),
                    tooltip: format!("{name} {marker} {speed}/s · {interface}"),
                }
            } else {
                // Display and threshold classification must share one rounded value.
                let percent = value.clamp(0.0, 100.0).round() as u8;
                let digits = percent.to_string();
                let details = match metric_id {
                    MetricId::Gpu => values
                        .gpu
                        .value
                        .as_ref()
                        .map(|value| {
                            format!(
                                " · {}",
                                mangodisk_platform::diagnostics::text(&value.adapter_name)
                            )
                        })
                        .unwrap_or_default(),
                    MetricId::Memory => values
                        .memory
                        .value
                        .as_ref()
                        .map(|value| {
                            format!(
                                " · {} / {}",
                                byte_text(value.memory.used_bytes as f64, 1024.0),
                                byte_text(value.memory.total_bytes as f64, 1024.0)
                            )
                        })
                        .unwrap_or_default(),
                    MetricId::Disk => values
                        .disk
                        .value
                        .as_ref()
                        .map(|value| {
                            format!(
                                " · {} · {} / {}",
                                if value.volume.system {
                                    labels.text("systemDisk")
                                } else {
                                    &value.volume.name
                                },
                                byte_text_with_precision(value.used_bytes as f64, base, 2),
                                byte_text_with_precision(value.total_bytes as f64, base, 2)
                            )
                        })
                        .unwrap_or_default(),
                    _ => String::new(),
                };
                let short = match metric_id {
                    MetricId::Cpu => "CPU",
                    MetricId::Gpu => "GPU",
                    MetricId::Memory => "MEM",
                    _ => "DISK",
                };
                DisplayEntry {
                    frequency: None,
                    gpu_device: None,
                    tone: Default::default(),
                    power_state: None,
                    fan_rows: None,
                    color_value: Some(u16::from(percent)),
                    id,
                    marker: marker.into(),
                    text: format!("{short} {digits:>3}%"),
                    tooltip: format!("{name} {digits}%{details}"),
                    digits,
                }
            };
            entries.push(entry);
        }
    }
    entries
}

#[cfg(test)]
#[path = "display_tests.rs"]
mod tests;

#[cfg(any(windows, test))]
pub fn notification_entries(entries: &[DisplayEntry]) -> Vec<DisplayEntry> {
    let mut result: Vec<DisplayEntry> = Vec::new();
    for entry in entries {
        if let Some(previous) = result.iter_mut().find(|previous| previous.id == entry.id) {
            previous.tooltip.push('\n');
            previous.tooltip.push_str(&entry.tooltip);
        } else {
            result.push(entry.clone());
        }
    }
    result
}

/// Unsupported platforms retain selected columns without starting native providers.
fn platform_placeholder(id: DisplayMetricId, labels: &super::labels::Labels) -> DisplayEntry {
    let (id, marker, name) = match id {
        DisplayMetricId::CpuFrequency => {
            (DisplayId::CpuFrequency, "CPU", labels.text("cpuFrequency"))
        }
        DisplayMetricId::CpuEfficiencyFrequency => (
            DisplayId::CpuEfficiencyFrequency,
            "CPUE",
            labels.text("cpuEfficiencyFrequency"),
        ),
        DisplayMetricId::CpuPerformanceFrequency => (
            DisplayId::CpuPerformanceFrequency,
            "CPUP",
            labels.text("cpuPerformanceFrequency"),
        ),
        DisplayMetricId::CpuSuperFrequency => (
            DisplayId::CpuSuperFrequency,
            "CPUS",
            labels.text("cpuSuperFrequency"),
        ),
        DisplayMetricId::CpuTemperature => {
            (DisplayId::CpuTemperature, "°C", labels.temperature("label"))
        }
        DisplayMetricId::Gpu => (DisplayId::Gpu, "G", labels.text("gpu")),
        DisplayMetricId::GpuTemperature => (
            DisplayId::GpuTemperature,
            "°C",
            labels.gpu_temperature("label"),
        ),
        DisplayMetricId::DiskTemperature => (
            DisplayId::DiskTemperature,
            "°C",
            labels.text("diskTemperature"),
        ),
        DisplayMetricId::BatteryTemperature => (
            DisplayId::BatteryTemperature,
            "°C",
            labels.text("batteryTemperature"),
        ),
        DisplayMetricId::ChargingPower => {
            (DisplayId::ChargingPower, "W", labels.text("chargingPower"))
        }
        DisplayMetricId::BatteryPower => {
            (DisplayId::BatteryPower, "W", labels.text("batteryPower"))
        }
        DisplayMetricId::CpuPower => (DisplayId::CpuPower, "W", labels.text("cpuPower")),
        DisplayMetricId::GpuPower => (DisplayId::GpuPower, "W", labels.text("gpuPower")),
        DisplayMetricId::SystemPower => (DisplayId::SystemPower, "W", labels.text("systemPower")),
        DisplayMetricId::FanSpeed => (DisplayId::FanSpeed, "FMAX", labels.text("fanSpeed")),
        DisplayMetricId::Cpu
        | DisplayMetricId::Memory
        | DisplayMetricId::Network
        | DisplayMetricId::Disk => unreachable!(),
    };
    DisplayEntry {
        frequency: None,
        gpu_device: None,
        id,
        tone: Default::default(),
        power_state: None,
        fan_rows: None,
        color_value: None,
        marker: marker.into(),
        digits: "—".into(),
        text: format!("{name} —"),
        tooltip: format!("{name} · {}", labels.text("platformUnsupported")),
    }
}
