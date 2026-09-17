//! One set of cell rectangles drives painting, hover and panel anchors on every edge.
use super::{layout::Bounds, presentation::Column};
use crate::resident::tray_display::format::DisplayId;

#[derive(Default, PartialEq, Eq)]
pub struct Surface {
    pub cells: Vec<Bounds>,
    pub width: i32,
    pub height: i32,
    pub vertical: bool,
}

impl Surface {
    pub fn arrange(columns: &[Column], bar: Bounds, dpi: u32) -> Option<Self> {
        let vertical = bar.width() < bar.height();
        let gap = (4 * dpi / 96) as i32;
        let row = (36 * dpi / 96) as i32;
        let row = if vertical {
            row
        } else {
            row.min(bar.height() - gap)
        };
        let width = (bar.width() - gap * 2).min((112 * dpi / 96) as i32);
        if columns.is_empty()
            || (vertical && width < (48 * dpi / 96) as i32)
            || (!vertical && row < (24 * dpi / 96) as i32)
        {
            return None;
        }
        let mut surface = Self {
            vertical,
            ..Self::default()
        };
        for column in columns {
            let cell = if vertical {
                // Network rates use separate value/unit lines rather than ellipsizing
                // every measurement on the narrow Windows 10 side taskbar.
                let height = if column.id == DisplayId::Upload {
                    row * 2
                } else {
                    row
                };
                Bounds {
                    left: 0,
                    top: surface.height,
                    right: width,
                    bottom: surface.height + height,
                }
            } else {
                Bounds {
                    left: surface.width,
                    top: 0,
                    right: surface.width + column.width,
                    bottom: row,
                }
            };
            surface.width = surface.width.max(cell.right);
            surface.height = surface.height.max(cell.bottom);
            surface.cells.push(cell);
        }
        Some(surface)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn small_horizontal_taskbars_keep_the_existing_two_line_layout() {
        let column = Column {
            compact: false,
            tone: Default::default(),
            id: DisplayId::Cpu,
            first: "CPU".into(),
            second: "15%".into(),
            width: 50,
            network: None,
        };
        let surface = Surface::arrange(
            &[column],
            Bounds {
                left: 0,
                top: 0,
                right: 1920,
                bottom: 30,
            },
            96,
        )
        .unwrap();
        assert_eq!(surface.height, 26);
        assert!(!surface.vertical);
    }

    #[test]
    fn vertical_cells_keep_order_and_network_units_readable_at_multiple_scales() {
        for dpi in [96, 120, 192, 240] {
            let scale = dpi as i32;
            let bar = Bounds {
                left: 0,
                top: 0,
                right: 80 * scale / 96,
                bottom: 1080 * scale / 96,
            };
            let columns = vec![
                Column {
                    compact: false,
                    tone: Default::default(),
                    id: DisplayId::Cpu,
                    first: "CPU".into(),
                    second: "15%".into(),
                    width: 50 * scale / 96,
                    network: None,
                },
                Column {
                    compact: false,
                    tone: Default::default(),
                    id: DisplayId::Upload,
                    first: "↑ 999.9 GB/s".into(),
                    second: "↓ 1.2 MB/s".into(),
                    width: 112 * scale / 96,
                    network: None,
                },
            ];
            let surface = Surface::arrange(&columns, bar, dpi).unwrap();
            assert!(surface.width < bar.width());
            assert_eq!(surface.height, 108 * scale / 96);
            assert_eq!(surface.cells[0].bottom, surface.cells[1].top);
        }
    }
}
