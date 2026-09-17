//! Compact menu-bar geometry for native rendering.
use super::format::{DisplayEntry, DisplayId};
use serde::Serialize;

#[cfg(any(target_os = "macos", test))]
pub const HEIGHT: u32 = 22;
#[cfg(any(target_os = "macos", test))]
pub fn gap(compact: bool) -> u32 {
    if compact {
        3
    } else {
        6
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Column {
    pub id: DisplayId,
    pub tone: super::usage_color::UsageTone,
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
                    if compact { 54 } else { 70 },
                ),
                DisplayId::Download | DisplayId::App => return None,
                id => (
                    match id {
                        DisplayId::Cpu => "CPU",
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
            Some(Column {
                id: entry.id,
                tone: entry.tone,
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
    fn entry(id: DisplayId, digits: &str, marker: &str) -> DisplayEntry {
        DisplayEntry {
            tone: Default::default(),
            usage_percent: None,
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
        assert_eq!(width(&compact, true, true), 186);
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
        assert_eq!(width(&unavailable, true, true), 186);
    }

    #[test]
    fn two_rows_preserve_order_and_pair_network_directions_without_width_jitter() {
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
    fn all_metrics_and_logo_fit_in_220_points() {
        let columns = columns(
            &[
                entry(DisplayId::Cpu, "100", "C"),
                entry(DisplayId::Memory, "100", "M"),
                entry(DisplayId::Upload, "999", "↑K"),
                entry(DisplayId::Download, "999", "↓G"),
                entry(DisplayId::Disk, "100", "D"),
            ],
            false,
        );
        assert_eq!(width(&columns, true, false), 220);
        assert_eq!(width(&[], true, false), 18);
        assert_eq!(width(&[], false, false), 0);
        assert_eq!(HEIGHT, 22);
    }
}
