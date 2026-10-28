//! Source-aware display color transitions are independent of native rendering.
use super::{
    display_entry::{DisplayEntry, DisplayId},
    usage_color,
};
use crate::resident::{
    color_rules::MetricColorRule,
    preference_schema::{DisplayMetricId, FanDisplayMode, ResidentPreferences},
};
use std::collections::HashMap;

#[derive(PartialEq, Eq, Hash)]
struct ColorKey {
    id: DisplayId,
    gpu_device: Option<(String, String)>,
}

#[derive(Default)]
pub(super) struct ColorState {
    rules: Option<(bool, Vec<MetricColorRule>)>,
    disk_volume: Option<String>,
    disk_temperature_volume: Option<String>,
    gpu_adapter: Option<String>,
    gpu_temperature_adapter: Option<String>,
    matches: HashMap<ColorKey, Option<usize>>,
    fan_source: Option<(FanDisplayMode, Vec<u8>)>,
    fan_matches: HashMap<u8, Option<usize>>,
}

impl ColorState {
    // Hysteresis belongs to the sampled volume, not the display slot. A newly
    // selected disk can arrive without an intervening unavailable frame.
    pub(super) fn apply(
        &mut self,
        entries: &mut [DisplayEntry],
        preferences: &ResidentPreferences,
        disk_volume: Option<&str>,
        disk_temperature_volume: Option<&str>,
        gpu_adapter: Option<&str>,
        gpu_temperature_adapter: Option<&str>,
    ) {
        let color_rules = (
            preferences.usage_colors && preferences.enabled,
            preferences.color_rules.clone(),
        );
        let fan_source = (preferences.fan_display, preferences.fan_ids.clone());
        let same_fans = self.fan_source.as_ref() == Some(&fan_source);
        let same_rules = self.rules.as_ref() == Some(&color_rules);
        if !same_fans || !same_rules {
            self.fan_matches.clear();
        }
        for entry in entries.iter_mut() {
            let id = match entry.id {
                DisplayId::Cpu => DisplayMetricId::Cpu,
                DisplayId::CpuTemperature => DisplayMetricId::CpuTemperature,
                DisplayId::Gpu => DisplayMetricId::Gpu,
                DisplayId::GpuTemperature => DisplayMetricId::GpuTemperature,
                DisplayId::BatteryTemperature => DisplayMetricId::BatteryTemperature,
                DisplayId::ChargingPower => DisplayMetricId::ChargingPower,
                DisplayId::CpuPower => DisplayMetricId::CpuPower,
                DisplayId::GpuPower => DisplayMetricId::GpuPower,
                DisplayId::SystemPower => DisplayMetricId::SystemPower,
                DisplayId::BatteryPower => DisplayMetricId::BatteryPower,
                DisplayId::FanSpeed => DisplayMetricId::FanSpeed,
                DisplayId::Memory => DisplayMetricId::Memory,
                DisplayId::Disk => DisplayMetricId::Disk,
                DisplayId::DiskTemperature => DisplayMetricId::DiskTemperature,
                _ => {
                    entry.tone = Default::default();
                    continue;
                }
            };
            let same_source = match entry.id {
                DisplayId::FanSpeed => same_fans,
                DisplayId::Disk => self.disk_volume.as_deref() == disk_volume,
                DisplayId::DiskTemperature => {
                    self.disk_temperature_volume.as_deref() == disk_temperature_volume
                }
                DisplayId::Gpu | DisplayId::GpuPower => self.gpu_adapter.as_deref() == gpu_adapter,
                DisplayId::GpuTemperature => {
                    self.gpu_temperature_adapter.as_deref() == gpu_temperature_adapter
                }
                _ => true,
            };
            let key = ColorKey {
                id: entry.id,
                gpu_device: entry
                    .gpu_device
                    .as_ref()
                    .map(|device| (device.id.clone(), device.name.clone())),
            };
            let previous = if same_rules && (entry.gpu_device.is_some() || same_source) {
                self.matches.get(&key).copied().flatten()
            } else {
                None
            };
            let rule = color_rules.1.iter().find(|rule| rule.id == id);
            let matched = rule
                .filter(|_| color_rules.0)
                .and_then(|rule| usage_color::next_rule(previous, entry.color_value, rule));
            entry.tone = matched
                .and_then(|index| rule.map(|rule| rule.rules[index].color().into()))
                .unwrap_or_default();
            self.matches.insert(key, matched);
            if let Some(rows) = &mut entry.fan_rows {
                for row in rows {
                    let previous = self.fan_matches.get(&row.id).copied().flatten();
                    let matched = rule
                        .filter(|_| color_rules.0)
                        .and_then(|rule| usage_color::next_rule(previous, row.color_value, rule));
                    row.tone = matched
                        .and_then(|index| rule.map(|rule| rule.rules[index].color().into()))
                        .unwrap_or_default();
                    self.fan_matches.insert(row.id, matched);
                }
            }
        }
        // Unavailable or hidden entries must not retain an older match.
        self.matches.retain(|key, _| {
            entries.iter().any(|entry| {
                entry.id == key.id
                    && entry
                        .gpu_device
                        .as_ref()
                        .map(|device| (&device.id, &device.name))
                        == key.gpu_device.as_ref().map(|(id, name)| (id, name))
            })
        });
        self.fan_matches.retain(|id, _| {
            entries
                .iter()
                .filter_map(|entry| entry.fan_rows.as_ref())
                .flatten()
                .any(|row| row.id == *id)
        });
        self.fan_source = entries
            .iter()
            .any(|entry| entry.id == DisplayId::FanSpeed)
            .then_some(fan_source);
        self.rules = Some(color_rules);
        self.disk_volume = disk_volume.map(str::to_owned);
        self.disk_temperature_volume = disk_temperature_volume.map(str::to_owned);
        self.gpu_adapter = gpu_adapter.map(str::to_owned);
        self.gpu_temperature_adapter = gpu_temperature_adapter.map(str::to_owned);
    }
}
