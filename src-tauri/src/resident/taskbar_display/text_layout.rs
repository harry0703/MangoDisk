//! Fixed text fields shared by opaque drawing and transparent glyph coloring.
use super::{layout::Bounds, presentation::Column, surface::Surface};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Alignment {
    Left,
    Center,
    Right,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ink {
    Foreground,
    Upload,
    Download,
    Usage(crate::resident::tray_display::usage_color::UsageTone),
}
impl Ink {
    pub fn rgb(self, foreground: [u8; 3]) -> [u8; 3] {
        // Match the macOS menu bar and the frontend status color tokens.
        match self {
            Self::Foreground => foreground,
            Self::Usage(tone) => tone.rgb(foreground),
            Self::Upload => [255, 69, 58],
            Self::Download => [10, 132, 255],
        }
    }
}

/// Keep metric labels subordinate to values in both native renderers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextStyle {
    Label,
    Value,
}
impl TextStyle {
    pub fn pixels(self, dpi: u32) -> u32 {
        match self {
            Self::Label => 9 * dpi / 96,
            Self::Value => 12 * dpi / 96,
        }
    }
}

pub struct Run<'a> {
    pub text: &'a str,
    pub style: TextStyle,
    pub bounds: Bounds,
    pub alignment: Alignment,
    pub ink: Ink,
}

pub fn runs<'a>(columns: &'a [Column], surface: &Surface, dpi: u32) -> Vec<Run<'a>> {
    let scale = |value: i32| value * dpi as i32 / 96;
    let mut result = Vec::new();
    for (column, cell) in columns.iter().zip(&surface.cells) {
        if let Some(rates) = &column.network {
            for (index, rate) in rates.iter().enumerate() {
                let top = cell.top + index as i32 * cell.height() / 2;
                let bottom = cell.top + (index as i32 + 1) * cell.height() / 2;
                let (arrow, value, unit) = if surface.vertical {
                    // Side taskbars keep four lines. The arrow has its own field;
                    // values and units share a stable right edge even at 48 DIP.
                    let middle = (top + bottom) / 2;
                    (
                        Bounds {
                            left: cell.left + scale(3),
                            top,
                            right: cell.left + scale(13),
                            bottom: middle,
                        },
                        Bounds {
                            left: cell.left + scale(13),
                            top,
                            right: cell.right - scale(3),
                            bottom: middle,
                        },
                        Bounds {
                            left: cell.left + scale(3),
                            top: middle,
                            right: cell.right - scale(3),
                            bottom,
                        },
                    )
                } else {
                    // Anchor the unit/value fields from the cell's right edge.
                    // Reserve 28 DIP for four numeric characters, including 99.9
                    // and rounded 1000. This keeps zero close to the arrow while
                    // preserving a stable unit anchor as speed changes.
                    let compact = column.compact;
                    let padding = scale(if compact { 3 } else { 5 });
                    let unit_width = scale(if compact { 12 } else { 33 });
                    let spacing = scale(if compact { 2 } else { 3 });
                    let unit_left = cell.right - padding - unit_width;
                    (
                        Bounds {
                            left: cell.left + padding,
                            top,
                            right: cell.left + padding + scale(10),
                            bottom,
                        },
                        Bounds {
                            left: cell.left + padding + scale(10),
                            top,
                            right: unit_left - spacing,
                            bottom,
                        },
                        Bounds {
                            left: unit_left,
                            top,
                            right: cell.right - padding,
                            bottom,
                        },
                    )
                };
                result.extend([
                    Run {
                        style: TextStyle::Value,
                        text: if index == 0 { "↑" } else { "↓" },
                        bounds: arrow,
                        alignment: Alignment::Left,
                        ink: if index == 0 {
                            Ink::Upload
                        } else {
                            Ink::Download
                        },
                    },
                    Run {
                        style: TextStyle::Value,
                        text: &rate.value,
                        bounds: value,
                        alignment: Alignment::Right,
                        ink: Ink::Foreground,
                    },
                    Run {
                        style: TextStyle::Value,
                        text: &rate.unit,
                        bounds: unit,
                        alignment: if surface.vertical {
                            Alignment::Right
                        } else {
                            Alignment::Left
                        },
                        ink: Ink::Foreground,
                    },
                ]);
            }
        } else {
            let split = cell.top + cell.height() * 5 / 12;
            for (index, text) in [&column.first, &column.second].into_iter().enumerate() {
                result.push(Run {
                    text,
                    style: if index == 0 {
                        TextStyle::Label
                    } else {
                        TextStyle::Value
                    },
                    bounds: Bounds {
                        left: cell.left + scale(3),
                        top: if index == 0 { cell.top } else { split },
                        right: cell.right - scale(3),
                        bottom: if index == 0 { split } else { cell.bottom },
                    },
                    alignment: Alignment::Center,
                    ink: if index == 1 {
                        Ink::Usage(column.tone)
                    } else {
                        Ink::Foreground
                    },
                });
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resident::tray_display::format::{DisplayEntry, DisplayId};

    #[test]
    fn percentage_tone_colors_only_values_for_all_percentage_metrics() {
        use crate::resident::tray_display::usage_color::UsageTone;
        for id in [DisplayId::Cpu, DisplayId::Memory, DisplayId::Disk] {
            for tone in [UsageTone::Normal, UsageTone::Warning, UsageTone::Critical] {
                let columns = super::super::presentation::columns(
                    &[DisplayEntry {
                        id,
                        tone,
                        usage_percent: Some(90),
                        digits: "90".into(),
                        marker: String::new(),
                        text: String::new(),
                        tooltip: String::new(),
                    }],
                    192,
                    true,
                );
                let surface = Surface::arrange(
                    &columns,
                    Bounds {
                        left: 0,
                        top: 0,
                        right: 1920,
                        bottom: 80,
                    },
                    192,
                )
                .unwrap();
                let runs = runs(&columns, &surface, 192);
                assert_eq!(runs[0].ink, Ink::Foreground);
                assert!(runs[0].style.pixels(96) < runs[1].style.pixels(96));
                assert_eq!(runs[1].ink, Ink::Usage(tone));
                assert_eq!(runs[1].text, "90%");
            }
        }
    }

    #[test]
    fn rate_boundaries_never_change_field_geometry_or_lose_direction_colors() {
        let mut baseline = Vec::new();
        for dpi in [96, 120, 144, 192, 240] {
            for (vertical, compact) in [(false, false), (false, true), (true, false), (true, true)]
            {
                baseline.clear();
                for title in [
                    "↑ —",
                    "↑ 0.0 B/s",
                    "↑ 9.9 KB/s",
                    "↑ 99.9 MB/s",
                    "↑ 999.9 GB/s",
                    "↑ 999.9 TB/s",
                ] {
                    let columns = super::super::presentation::columns(
                        &[DisplayEntry {
                            tone: Default::default(),
                            usage_percent: None,
                            id: DisplayId::Upload,
                            text: title.into(),
                            marker: String::new(),
                            digits: String::new(),
                            tooltip: String::new(),
                        }],
                        dpi,
                        compact,
                    );
                    let bar = if vertical {
                        Bounds {
                            left: 0,
                            top: 0,
                            right: (56 * dpi / 96) as i32,
                            bottom: 1080,
                        }
                    } else {
                        Bounds {
                            left: 0,
                            top: 0,
                            right: 1920,
                            bottom: (40 * dpi / 96) as i32,
                        }
                    };
                    let surface = Surface::arrange(&columns, bar, dpi).unwrap();
                    let runs = runs(&columns, &surface, dpi);
                    let bounds: Vec<_> = runs.iter().map(|run| run.bounds).collect();
                    if baseline.is_empty() {
                        baseline.clone_from(&bounds);
                    }
                    assert_eq!(bounds, baseline);
                    assert!(runs.iter().all(|run| run.bounds.fits_in(surface.cells[0])));
                    assert_eq!(runs[0].ink.rgb([0, 0, 0]), [255, 69, 58]);
                    assert_eq!(runs[3].ink.rgb([255, 255, 255]), [10, 132, 255]);
                    assert_eq!(runs[1].alignment, Alignment::Right);
                    assert_eq!(runs[4].text, "—");
                    assert!(runs[5].text.is_empty());
                }
            }
        }
    }
}
