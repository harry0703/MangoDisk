//! Compact menu-bar geometry for native rendering.
use super::display_entry::{DisplayEntry, DisplayId};
use serde::Serialize;

#[cfg(any(target_os = "macos", test))]
pub const HEIGHT: u32 = 22;
#[cfg(any(target_os = "macos", test))]
pub fn gap(compact: bool) -> u32 {
    if compact {
        2
    } else {
        6
    }
}

pub struct RateLayout {
    pub value_offset: u32,
    pub value_width: u32,
    pub unit_offset: u32,
    pub unit_width: u32,
}

pub fn rate_layout(compact: bool) -> RateLayout {
    if compact {
        RateLayout {
            value_offset: 10,
            value_width: 21,
            unit_offset: 32,
            unit_width: 10,
        }
    } else {
        RateLayout {
            value_offset: 11,
            value_width: 24,
            unit_offset: 38,
            unit_width: 32,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Column {
    pub id: DisplayId,
    pub tone: super::usage_color::UsageTone,
    pub power_state: Option<mangodisk_platform::system_resources::battery::BatteryPowerState>,
    pub fan_tones: Option<[super::usage_color::UsageTone; 2]>,
    pub top: String,
    pub bottom: String,
    pub width: u32,
    pub top_unit: String,
    pub bottom_unit: String,
}

fn rate_unit(entry: &DisplayEntry, compact: bool) -> String {
    if entry.digits == "—" {
        return String::new();
    }
    if compact {
        return entry
            .marker
            .chars()
            .last()
            .filter(|c| matches!(c, 'K' | 'M' | 'G' | 'T'))
            .unwrap_or('B')
            .to_string();
    }
    match entry.marker.chars().last() {
        Some('K') => "KB/s",
        Some('M') => "MB/s",
        Some('G') => "GB/s",
        Some('T') => "TB/s",
        _ => "B/s",
    }
    .into()
}

pub fn columns(entries: &[DisplayEntry], compact: bool) -> Vec<Column> {
    let rate = rate_layout(compact);
    entries
        .iter()
        .filter_map(|entry| {
            let (top, bottom, width) = match entry.id {
                DisplayId::Upload => (
                    entry.digits.clone(),
                    entries
                        .iter()
                        .find(|e| e.id == DisplayId::Download)
                        .map(|entry| entry.digits.clone())
                        .unwrap_or_else(|| "—".into()),
                    rate.unit_offset + rate.unit_width,
                ),
                DisplayId::Download | DisplayId::App => return None,
                DisplayId::CpuTemperature
                | DisplayId::GpuTemperature
                | DisplayId::DiskTemperature
                | DisplayId::BatteryTemperature => (
                    if entry.id == DisplayId::DiskTemperature {
                        "DISK"
                    } else if entry.id == DisplayId::GpuTemperature {
                        "GPU"
                    } else if entry.id == DisplayId::BatteryTemperature {
                        "BAT"
                    } else {
                        "CPU"
                    }
                    .into(),
                    if entry.digits == "—" {
                        "—".into()
                    } else {
                        format!("{}°C", entry.digits)
                    },
                    if compact { 37 } else { 46 },
                ),
                DisplayId::CpuFrequency
                | DisplayId::CpuEfficiencyFrequency
                | DisplayId::CpuPerformanceFrequency
                | DisplayId::CpuSuperFrequency => (
                    entry.marker.clone(),
                    if entry.frequency.as_ref().is_some_and(|value| {
                        value.state != super::display_entry::FrequencyDisplayState::Active
                    }) || entry.digits == "—"
                    {
                        entry.digits.clone()
                    } else {
                        format!("{}G", entry.digits)
                    },
                    (if compact { 37 } else { 40 }).max(entry.frequency.as_ref().map_or(
                        0,
                        |value| {
                            value
                                .idle_label
                                .chars()
                                .map(|c| if (c as u32) < 0x2e80 { 8 } else { 12 })
                                .sum::<u32>()
                        },
                    )),
                ),
                DisplayId::FanSpeed => match &entry.fan_rows {
                    Some(rows) => (
                        rows[0].text.clone(),
                        rows[1].text.clone(),
                        if compact { 58 } else { 64 },
                    ),
                    None => (
                        entry.marker.clone(),
                        entry.digits.clone(),
                        if compact { 38 } else { 42 },
                    ),
                },
                DisplayId::CpuPower
                | DisplayId::GpuPower
                | DisplayId::ChargingPower
                | DisplayId::BatteryPower
                | DisplayId::SystemPower => (
                    if entry.id == DisplayId::CpuPower {
                        "CPU"
                    } else if entry.id == DisplayId::GpuPower {
                        "GPU"
                    } else if entry.id == DisplayId::SystemPower {
                        "SYS"
                    } else if entry.id == DisplayId::ChargingPower {
                        "CHG"
                    } else {
                        use mangodisk_platform::system_resources::battery::BatteryPowerState;
                        match entry.power_state {
                            Some(BatteryPowerState::Charging) => "BAT",
                            Some(BatteryPowerState::Discharging) => "BAT",
                            Some(BatteryPowerState::Idle) => "IDLE",
                            None => "BAT",
                        }
                    }
                    .into(),
                    if entry.digits == "—" {
                        "—".into()
                    } else {
                        format!("{}W", entry.digits)
                    },
                    // Three-digit fields stay fixed across ordinary samples.
                    // Whole-system readings may reach 3000 W; expand only when
                    // the rounded display actually requires a fourth digit.
                    if entry.digits.len() >= 4 {
                        if compact {
                            41
                        } else {
                            45
                        }
                    } else if compact {
                        34
                    } else {
                        37
                    },
                ),
                id => (
                    match id {
                        DisplayId::Cpu => "CPU",
                        DisplayId::Gpu => "GPU",
                        DisplayId::Memory => "MEM",
                        _ => "DISK",
                    }
                    .into(),
                    if entry.digits == "—" {
                        "—".into()
                    } else {
                        format!("{}%", entry.digits)
                    },
                    // Reserve the measured width of 100% at the native 12-point font.
                    if compact { 34 } else { 36 },
                ),
            };
            let top = entry
                .gpu_device
                .as_ref()
                .map(|device| device.label.clone())
                .unwrap_or(top);
            let width = width.max((top.chars().count() as u32) * 7);
            Some(Column {
                id: entry.id,
                tone: entry.tone,
                power_state: entry.power_state,
                fan_tones: entry
                    .fan_rows
                    .as_ref()
                    .map(|rows| rows.each_ref().map(|row| row.tone)),
                top,
                bottom,
                width,
                // Units are separate fields so both renderers can anchor them
                // independently of the number of digits or the transfer scale.
                top_unit: if entry.id == DisplayId::Upload {
                    rate_unit(entry, compact)
                } else {
                    String::new()
                },
                bottom_unit: if entry.id == DisplayId::Upload {
                    entries
                        .iter()
                        .find(|e| e.id == DisplayId::Download)
                        .map(|entry| rate_unit(entry, compact))
                        .unwrap_or_default()
                } else {
                    String::new()
                },
            })
        })
        .collect()
}

#[cfg(any(target_os = "macos", test))]
pub fn width(columns: &[Column], icon: bool, compact: bool) -> u32 {
    let gap = gap(compact);
    let brand = if icon {
        18 + if columns.is_empty() { 0 } else { gap }
    } else {
        0
    };
    brand
        + columns.iter().map(|c| c.width).sum::<u32>()
        + columns.len().saturating_sub(1) as u32 * gap
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fan_pair_uses_one_column_and_keeps_width_for_missing_or_maximum_values() {
        for compact in [false, true] {
            let mut fan = entry(DisplayId::FanSpeed, "0", "FMAX");
            let single = columns(&[fan.clone()], compact);
            assert_eq!(single[0].top, "FMAX");
            assert_eq!(single[0].bottom, "0");
            for rows in [
                ["F1 0", "F2 2400"],
                ["F1 —", "F2 —"],
                ["F9 30000", "F10 30000"],
            ] {
                fan.fan_rows = Some([0, 1].map(|index| {
                    crate::resident::tray_display::display_entry::FanRow {
                        id: index as u8,
                        text: rows[index].into(),
                        color_value: None,
                        tone: Default::default(),
                    }
                }));
                let pair = columns(&[fan.clone()], compact);
                assert_eq!(pair.len(), 1);
                assert_eq!(pair[0].fan_tones, Some([Default::default(); 2]));
                fan.fan_rows.as_mut().unwrap()[1].tone =
                    super::super::usage_color::UsageTone::Critical;
                let colored = columns(&[fan.clone()], compact);
                assert_ne!(colored, pair);
                assert_eq!(
                    colored[0].fan_tones,
                    Some([
                        super::super::usage_color::UsageTone::Normal,
                        super::super::usage_color::UsageTone::Critical
                    ])
                );
                assert_eq!(pair[0].width, if compact { 58 } else { 64 });
                assert!(!pair[0].top.contains("RPM"));
                assert!(!pair[0].bottom.contains("RPM"));
            }
        }
    }
    #[test]
    fn disk_temperature_columns_keep_celsius_and_unavailable_width() {
        for compact in [false, true] {
            let ready = columns(&[entry(DisplayId::DiskTemperature, "150", "°C")], compact);
            let unavailable = columns(&[entry(DisplayId::DiskTemperature, "—", "°C")], compact);
            assert_eq!(ready[0].top, "DISK");
            assert_eq!(ready[0].bottom, "150°C");
            assert_eq!(unavailable[0].bottom, "—");
            assert_eq!(ready[0].width, unavailable[0].width);
        }
    }
    #[test]
    fn temperature_columns_keep_celsius_and_fixed_width() {
        for compact in [false, true] {
            let maximum = columns(&[entry(DisplayId::CpuTemperature, "150", "°C")], compact);
            assert_eq!(maximum[0].top, "CPU");
            assert_eq!(maximum[0].bottom, "150°C");
            assert_eq!(maximum[0].tone, Default::default());
            for digits in ["9", "99", "100", "—"] {
                let current = columns(&[entry(DisplayId::CpuTemperature, digits, "°C")], compact);
                assert_eq!(current[0].width, maximum[0].width);
                assert!(!current[0].bottom.contains('%'));
            }
        }
    }
    fn entry(id: DisplayId, digits: &str, marker: &str) -> DisplayEntry {
        DisplayEntry {
            frequency: None,
            gpu_device: None,
            tone: Default::default(),
            power_state: None,
            fan_rows: None,
            color_value: None,
            id,
            digits: digits.into(),
            marker: marker.into(),
            text: String::new(),
            tooltip: String::new(),
        }
    }
    #[test]
    fn compact_columns_keep_labels_values_and_reduce_width() {
        let mut entries = vec![
            entry(DisplayId::Cpu, "100", "C"),
            entry(DisplayId::Memory, "0", "M"),
            entry(DisplayId::Disk, "99", "D"),
            entry(DisplayId::Upload, "999", "↑G"),
            entry(DisplayId::Download, "1", "↓K"),
        ];
        let standard = columns(&entries, false);
        let compact = columns(&entries, true);
        assert_eq!(width(&compact, true, true), 170);
        assert_eq!(width(&standard, true, false), 220);
        assert_eq!(compact[0].bottom, "100%");
        assert_eq!(compact[2].top, "DISK");
        assert_eq!(compact[3].top_unit, "G");
        assert_eq!(compact[3].bottom_unit, "K");
        entries[0].digits = "—".into();
        entries[3].digits = "—".into();
        let unavailable = columns(&entries, true);
        assert_eq!(unavailable[0].bottom, "—");
        assert!(unavailable[3].top_unit.is_empty());
        assert_eq!(width(&unavailable, true, true), 170);
    }

    #[test]
    fn two_rows_preserve_order_and_pair_network_directions_without_width_jitter() {
        for compact in [false, true] {
            let rate = rate_layout(compact);
            assert!(rate.value_offset + rate.value_width < rate.unit_offset);
        }
        let mut entries = vec![
            entry(DisplayId::Upload, "9", "↑K"),
            entry(DisplayId::Download, "5", "↓K"),
            entry(DisplayId::Cpu, "16", "C"),
        ];
        let first = columns(&entries, false);
        assert_eq!(first.len(), 2);
        assert_eq!((&*first[0].top, &*first[0].bottom), ("9", "5"));
        assert_eq!((&*first[1].top, &*first[1].bottom), ("CPU", "16%"));
        entries[0] = entry(DisplayId::Upload, "999", "↑T");
        entries[2].digits = "100".into();
        assert_eq!(
            width(&first, true, false),
            width(&columns(&entries, false), true, false)
        );
        assert_eq!(columns(&entries, false)[0].top_unit, "TB/s");
        assert_eq!(first[0].bottom_unit, "KB/s");
        entries[0].digits = "—".into();
        assert_eq!(columns(&entries, false)[0].top, "—");
        assert!(columns(&entries, false)[0].top_unit.is_empty());
    }
    #[test]
    fn all_metrics_and_logo_have_stable_compact_widths() {
        let mut entries = vec![
            entry(DisplayId::Cpu, "100", "C"),
            entry(DisplayId::Gpu, "100", "G"),
            entry(DisplayId::Memory, "100", "M"),
            entry(DisplayId::Upload, "999", "↑K"),
            entry(DisplayId::Download, "999", "↓G"),
            entry(DisplayId::Disk, "100", "D"),
        ];
        let columns = columns(&entries, false);
        assert_eq!(width(&columns, true, false), 262);
        entries.extend([
            entry(DisplayId::CpuTemperature, "150", "°C"),
            entry(DisplayId::GpuTemperature, "150", "°C"),
        ]);
        assert_eq!(width(&super::columns(&entries, false), true, false), 366);
        assert_eq!(width(&super::columns(&entries, true), true, true), 284);
        assert_eq!(width(&[], true, false), 18);
        assert_eq!(width(&[], false, false), 0);
        assert_eq!(HEIGHT, 22);
    }
}
