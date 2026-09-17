//! Shared taskbar text and stable column widths for native rendering.
use crate::resident::tray_display::format::DisplayId;
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Column {
    pub id: DisplayId,
    pub compact: bool,
    pub tone: crate::resident::tray_display::usage_color::UsageTone,
    pub first: String,
    pub second: String,
    pub width: i32,
    pub network: Option<[Rate; 2]>,
}
/// Separate rate fields let renderers anchor units independently of digit count.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Rate {
    pub value: String,
    pub unit: String,
}
impl Rate {
    fn from_title(text: &str) -> Self {
        // The shared formatter owns this fixed arrow/value/unit grammar. Do not
        // parse localized tooltips or diagnostics to determine display behavior.
        // A five-character value fills the formatter's entire padded field,
        // leaving no space after the arrow (for example, "↑123.4 KB/s").
        let mut fields = text.trim_start_matches(['↑', '↓']).split_whitespace();
        let value = fields.next().unwrap_or("—");
        // Large rates do not need tenths in the narrow taskbar. Keep the shared
        // unit and rounding semantics, including a possible rounded 1000, rather
        // than clipping values to 999 or changing the byte base at this boundary.
        // Small fractional rates remain visible; exact zero loses its decimal.
        let value = value
            .parse::<f64>()
            .ok()
            .filter(|v| v.is_finite() && *v >= 0.0)
            .map(|value| {
                if value >= 100.0 {
                    format!("{value:.0}")
                } else {
                    format!("{value:.1}").trim_end_matches(".0").to_owned()
                }
            })
            .unwrap_or_else(|| "—".into());
        let unit = fields.next().unwrap_or_default();
        Self {
            unit: if value == "—" { "" } else { unit }.into(),
            value,
        }
    }
}

pub fn columns(
    entries: &[crate::resident::tray_display::format::DisplayEntry],
    dpi: u32,
    compact: bool,
) -> Vec<Column> {
    entries
        .iter()
        .filter(|e| e.id != DisplayId::Download)
        .map(|entry| {
            let network = entry.id == DisplayId::Upload;
            Column {
                id: entry.id,
                compact,
                tone: entry.tone,
                first: if network {
                    entry.text.split_whitespace().collect::<Vec<_>>().join(" ")
                } else {
                    match entry.id {
                        DisplayId::Cpu => "CPU",
                        DisplayId::Memory => "MEM",
                        _ => "DISK",
                    }
                    .into()
                },
                second: if network {
                    entries
                        .iter()
                        .find(|e| e.id == DisplayId::Download)
                        .map(|e| e.text.split_whitespace().collect::<Vec<_>>().join(" "))
                        .unwrap_or_else(|| "↓ —".into())
                } else {
                    format!(
                        "{}{}",
                        entry.digits,
                        if entry.digits == "—" { "" } else { "%" }
                    )
                },
                // Compact units save width without discarding numeric precision.
                // Four numeric characters cover 99.9 and a rounded 1000.
                width: ((match (compact, network) {
                    (true, true) => 58,
                    (true, false) => 38,
                    (false, true) => 84,
                    (false, false) => 50,
                }) * dpi
                    / 96) as i32,
                network: network.then(|| {
                    let mut rates = [
                        Rate::from_title(&entry.text),
                        Rate::from_title(
                            entries
                                .iter()
                                .find(|e| e.id == DisplayId::Download)
                                .map(|e| e.text.as_str())
                                .unwrap_or("↓ —"),
                        ),
                    ];
                    if compact {
                        for rate in &mut rates {
                            rate.unit = rate
                                .unit
                                .chars()
                                .next()
                                .map(|unit| unit.to_string())
                                .unwrap_or_default();
                        }
                    }
                    rates
                }),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resident::tray_display::format::DisplayEntry;
    #[test]
    fn full_width_rate_does_not_drop_the_number_attached_to_arrow() {
        for text in ["↑123.4 KB/s", "↓999.9 TB/s", "↑  0.0  B/s", "↓ —"] {
            let rate = Rate::from_title(text);
            let expected = if text.contains("123.4") {
                ("123", "KB/s")
            } else if text.contains("999.9") {
                ("1000", "TB/s")
            } else if text.contains("0.0") {
                ("0", "B/s")
            } else {
                ("—", "")
            };
            assert_eq!((rate.value.as_str(), rate.unit.as_str()), expected);
        }
    }
    #[test]
    fn rate_precision_stays_bounded_across_unit_and_digit_transitions() {
        for unit in ["B/s", "KB/s", "MB/s", "GB/s", "TB/s"] {
            for (input, expected) in [
                ("0.0", "0"),
                ("0.1", "0.1"),
                ("9.9", "9.9"),
                ("10.0", "10"),
                ("99.9", "99.9"),
                ("100.0", "100"),
                ("999.0", "999"),
                ("999.9", "1000"),
            ] {
                let rate = Rate::from_title(&format!("↑{input} {unit}"));
                assert_eq!(rate.value, expected);
                assert_eq!(rate.unit, unit);
                assert!(rate.value.len() <= 4);
            }
        }
        for input in ["↑ —", "↑NaN KB/s", "↑-1.0 B/s", "↑inf TB/s"] {
            let rate = Rate::from_title(input);
            assert_eq!(rate.value, "—");
            assert!(rate.unit.is_empty());
        }
    }
    fn entry(id: DisplayId, text: &str, digits: &str) -> DisplayEntry {
        DisplayEntry {
            tone: Default::default(),
            usage_percent: None,
            id,
            marker: String::new(),
            digits: digits.into(),
            text: text.into(),
            tooltip: String::new(),
        }
    }
    #[test]
    fn paired_network_keeps_user_order_and_fixed_width_at_every_scale() {
        let entries = [
            entry(DisplayId::Upload, "↑ 24.0 KB/s", "24"),
            entry(DisplayId::Download, "↓ 1.2 MB/s", "1"),
            entry(DisplayId::Memory, "MEM 62%", "62"),
        ];
        for dpi in [96, 120, 144, 192, 240] {
            let result = columns(&entries, dpi, false);
            assert_eq!(result.len(), 2);
            assert_eq!(result[0].id, DisplayId::Upload);
            assert_eq!(result[0].second, "↓ 1.2 MB/s");
            assert_eq!(result[0].width, (84 * dpi / 96) as i32);
            assert_eq!(result[1].second, "62%");
        }
        let mut changed = entries.clone();
        changed[0].text = "↑ 999.9 GB/s".into();
        assert_eq!(
            columns(&entries, 96, false)[0].width,
            columns(&changed, 96, false)[0].width
        );
    }
    #[test]
    fn compact_density_reduces_space_without_changing_values() {
        let entries = [
            entry(DisplayId::Cpu, "CPU 100%", "100"),
            entry(DisplayId::Memory, "MEM 0%", "0"),
            entry(DisplayId::Upload, "↑999.9 TB/s", "999.9"),
            entry(DisplayId::Download, "↓ 0.0 B/s", "0.0"),
            entry(DisplayId::Disk, "DISK —", "—"),
        ];
        for dpi in [96, 120, 144, 192, 240] {
            let standard = columns(&entries, dpi, false);
            let compact = columns(&entries, dpi, true);
            assert_eq!(
                compact.iter().map(|c| c.width).sum::<i32>(),
                (3 * (38 * dpi / 96) + 58 * dpi / 96) as i32
            );
            for (full, small) in standard.iter().zip(&compact) {
                assert!(small.width < full.width);
                assert_eq!(small.first, full.first);
                assert_eq!(small.second, full.second);
                if let (Some(small), Some(full)) = (&small.network, &full.network) {
                    for (small, full) in small.iter().zip(full) {
                        assert_eq!(small.value, full.value);
                        assert_eq!(
                            small.unit,
                            full.unit
                                .chars()
                                .next()
                                .map(|c| c.to_string())
                                .unwrap_or_default()
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn unavailable_values_do_not_masquerade_as_zero_percent() {
        assert_eq!(
            columns(&[entry(DisplayId::Cpu, "CPU —", "—")], 96, false)[0].second,
            "—"
        );
    }
}
