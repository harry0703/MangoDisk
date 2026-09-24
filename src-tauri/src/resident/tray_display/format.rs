//! Shared text for native resident surfaces.
use mangodisk_core::system_resources::{
    metrics::{MetricId, MetricStatus},
    readings::ResourceReadings,
};
use serde::Serialize;

use crate::resident::preferences::ResidentPreferences;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DisplayId {
    App,
    Cpu,
    Memory,
    Upload,
    Download,
    Disk,
}

impl DisplayId {
    pub const ALL: [Self; 6] = [
        Self::App,
        Self::Cpu,
        Self::Memory,
        Self::Upload,
        Self::Download,
        Self::Disk,
    ];
    pub fn tray_id(self) -> &'static str {
        match self {
            Self::App => "resident",
            Self::Cpu => "resident-cpu",
            Self::Memory => "resident-memory",
            Self::Upload => "resident-upload",
            Self::Download => "resident-download",
            Self::Disk => "resident-disk",
        }
    }
    pub fn metric(self) -> Option<MetricId> {
        match self {
            Self::App => None,
            Self::Cpu => Some(MetricId::Cpu),
            Self::Memory => Some(MetricId::Memory),
            Self::Upload | Self::Download => Some(MetricId::Network),
            Self::Disk => Some(MetricId::Disk),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DisplayEntry {
    pub id: DisplayId,
    pub tone: super::usage_color::UsageTone,
    pub usage_percent: Option<u8>,
    pub marker: String,
    pub digits: String,
    pub text: String,
    pub tooltip: String,
}

/// Linux AppIndicator exposes frequently changing status through its title
/// label. Keep the icon stable and compose only the enabled metric fields.
#[cfg(target_os = "linux")]
pub fn indicator_title(entries: &[DisplayEntry], compact: bool) -> Option<String> {
    let title = entries
        .iter()
        .filter_map(|entry| {
            let text = if compact {
                match entry.id {
                    DisplayId::Cpu | DisplayId::Memory | DisplayId::Disk => {
                        format!("{}{}%", entry.marker, entry.digits)
                    }
                    DisplayId::Upload | DisplayId::Download => {
                        let mut marker = entry.marker.chars();
                        let direction = marker.next()?;
                        format!("{direction}{}{}", entry.digits, marker.as_str())
                    }
                    DisplayId::App => return None,
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
        .filter(|id| match id.metric() {
            Some(metric) => preferences.shows(metric),
            None => preferences.effective_icon(),
        })
        .collect()
}

#[cfg(any(windows, test))]
pub fn windows_desired(preferences: &ResidentPreferences, taskbar_active: bool) -> Vec<DisplayId> {
    if !preferences.enabled {
        return Vec::new();
    }
    if taskbar_active {
        if preferences.effective_icon() {
            vec![DisplayId::App]
        } else {
            Vec::new()
        }
    } else {
        desired(preferences)
    }
}

pub fn compact_rate(bytes: f64, base: f64) -> (String, &'static str) {
    if !bytes.is_finite() || bytes < 0.0 {
        return ("—".into(), "");
    }
    let units = ["B", "K", "M", "G", "T"];
    let mut number = bytes;
    let mut unit = 0;
    while number >= base - 0.5 && unit < units.len() - 1 {
        number /= base;
        unit += 1;
    }
    // Three digits fit at the normal numeric font size. Never shrink until an
    // arbitrary value fits; the last unit explicitly caps exceptionally large rates.
    (format!("{:.0}", number.min(999.0)), units[unit])
}

pub fn byte_text(bytes: f64, base: f64) -> String {
    let mut value = bytes;
    let units = ["B", "KB", "MB", "GB", "TB"];
    let mut index = 0;
    while value >= base && index < units.len() - 1 {
        value /= base;
        index += 1;
    }
    format!("{value:.1} {}", units[index])
}

/// Constant-width fields keep native status-bar layout stable across unit changes.
fn rate_title(bytes: f64, base: f64) -> String {
    let units = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes;
    let mut index = 0;
    while (value * 10.0).round() / 10.0 >= base && index < units.len() - 1 {
        value /= base;
        index += 1;
    }
    format!("{:>5.1} {:>2}/s", value.min(999.9), units[index])
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
        let (ids, status) = match metric.id {
            MetricId::Cpu => (vec![DisplayId::Cpu], values.cpu.status),
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
                DisplayId::App => unreachable!(),
            };
            // Invalid samples must not become NaN%, infinity, or a misleading
            // valid-looking rate. Keep the normal unavailable presentation.
            let status = if status == MetricStatus::Ready
                && value.is_some_and(|value| {
                    !value.is_finite() || (metric.id == MetricId::Network && value < 0.0)
                }) {
                MetricStatus::Failed
            } else {
                status
            };
            let name = labels.metric(metric.id);
            if status != MetricStatus::Ready || value.is_none() {
                entries.push(DisplayEntry {
                    tone: Default::default(),
                    usage_percent: None,
                    id,
                    marker: marker.into(),
                    digits: "—".into(),
                    text: format!(
                        "{} —",
                        match id {
                            DisplayId::Cpu => "CPU",
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
            let entry = if metric.id == MetricId::Network {
                let (digits, unit) = compact_rate(value, base);
                let speed = byte_text(value, base);
                let interface = values
                    .network
                    .value
                    .as_ref()
                    .map(|value| value.interface.name.as_str())
                    .unwrap_or("");
                DisplayEntry {
                    tone: Default::default(),
                    usage_percent: None,
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
                let details = match metric.id {
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
                                byte_text(value.used_bytes as f64, base),
                                byte_text(value.total_bytes as f64, base)
                            )
                        })
                        .unwrap_or_default(),
                    _ => String::new(),
                };
                let short = match metric.id {
                    MetricId::Cpu => "CPU",
                    MetricId::Memory => "MEM",
                    _ => "DISK",
                };
                DisplayEntry {
                    tone: Default::default(),
                    usage_percent: Some(percent),
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
mod tests {
    use super::*;

    #[cfg(target_os = "linux")]
    fn display_entry(id: DisplayId, text: &str) -> DisplayEntry {
        DisplayEntry {
            id,
            tone: Default::default(),
            usage_percent: None,
            marker: String::new(),
            digits: String::new(),
            text: text.to_string(),
            tooltip: String::new(),
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn indicator_title_contains_only_non_empty_enabled_metrics() {
        assert_eq!(indicator_title(&[], false), None);
        assert_eq!(
            indicator_title(
                &[
                    display_entry(DisplayId::Cpu, "CPU  12%"),
                    display_entry(DisplayId::Memory, "MEM  48%"),
                    display_entry(DisplayId::Disk, ""),
                ],
                false,
            ),
            Some("CPU  12%  MEM  48%".to_string())
        );
    }

    #[cfg(target_os = "linux")]
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

        let standard =
            indicator_title(&[cpu.clone(), memory.clone(), upload.clone()], false).unwrap();
        let compact = indicator_title(&[cpu, memory, upload], true).unwrap();

        assert_eq!(compact, "C12% M48% ↑82K");
        assert!(compact.len() < standard.len());
    }

    #[test]
    fn all_thirty_two_combinations_have_unique_entries_and_paired_network_directions() {
        for bits in 0..16 {
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
                assert!(ids.len() <= 6);
                assert_eq!(
                    ids.contains(&DisplayId::Upload),
                    prefs.shows(MetricId::Network)
                );
                assert_eq!(
                    ids.contains(&DisplayId::Download),
                    prefs.shows(MetricId::Network)
                );
                assert_eq!(ids.contains(&DisplayId::App), show_icon || bits == 0);
                prefs.enabled = false;
                assert!(desired(&prefs).is_empty());
            }
        }
    }

    #[test]
    fn taskbar_fallback_restores_all_metrics_and_never_loses_the_last_entry() {
        for bits in 0..16 {
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
                assert_eq!(active.contains(&DisplayId::App), show_icon || bits == 0);
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
        assert_eq!(desired(&prefs), DisplayId::ALL);
        assert_eq!(DisplayId::Upload.metric(), DisplayId::Download.metric());
        prefs.enabled = false;
        assert!(desired(&prefs).is_empty());
    }
    #[test]
    fn invalid_ready_samples_use_the_unavailable_presentation() {
        use mangodisk_core::system_resources::metrics::{CpuUsage, MetricReading};
        let mut preferences = ResidentPreferences::default();
        for metric in &mut preferences.metrics {
            metric.enabled = metric.id == MetricId::Cpu;
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
            assert_eq!(cpu.usage_percent, Some(expected));
            assert_eq!(UsageTone::Normal.next(cpu.usage_percent, 75, 85), tone);
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
}
