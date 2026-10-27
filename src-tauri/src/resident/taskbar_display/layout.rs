//! Pure placement rules: never cover an occupied system control or leave the taskbar.
use super::position::Edge;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Bounds {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}
impl Bounds {
    pub fn transpose(self) -> Self {
        Self {
            left: self.top,
            top: self.left,
            right: self.bottom,
            bottom: self.right,
        }
    }
    pub fn width(self) -> i32 {
        self.right - self.left
    }
    pub fn height(self) -> i32 {
        self.bottom - self.top
    }
    pub fn fits_in(self, outer: Self) -> bool {
        self.left >= outer.left
            && self.top >= outer.top
            && self.right <= outer.right
            && self.bottom <= outer.bottom
    }
    pub fn intersection(self, other: Self) -> Option<Self> {
        let clipped = Self {
            left: self.left.max(other.left),
            top: self.top.max(other.top),
            right: self.right.min(other.right),
            bottom: self.bottom.min(other.bottom),
        };
        (clipped.width() > 0 && clipped.height() > 0).then_some(clipped)
    }
    pub fn contains(self, x: i32, y: i32) -> bool {
        x >= self.left && x < self.right && y >= self.top && y < self.bottom
    }
}

/// The shell HWND can include space above or beside its visible content.
/// Only borrow the content's cross axis; the layout lease still owns its edge.
pub fn content_band(host: Bounds, content: Option<Bounds>) -> Bounds {
    let Some(content) = content.filter(|r| r.width() > 0 && r.height() > 0) else {
        return host;
    };
    if host.width() < host.height() {
        if content.left >= host.left && content.right <= host.right {
            return Bounds {
                left: content.left,
                right: content.right,
                ..host
            };
        }
    } else if content.top >= host.top && content.bottom <= host.bottom {
        return Bounds {
            top: content.top,
            bottom: content.bottom,
            ..host
        };
    }
    host
}

/// Apply the same content alignment to reserved slots and ordinary safe gaps.
pub fn align_cross_axis(placed: Bounds, band: Bounds) -> Option<Bounds> {
    let aligned = if band.width() < band.height() {
        let left = band.left + (band.width() - placed.width()) / 2;
        Bounds {
            left,
            right: left + placed.width(),
            ..placed
        }
    } else {
        let top = band.top + (band.height() - placed.height()) / 2;
        Bounds {
            top,
            bottom: top + placed.height(),
            ..placed
        }
    };
    (placed.width() > 0 && placed.height() > 0 && aligned.fits_in(band)).then_some(aligned)
}

/// Choose the first/last available gap. Exclude a margin around controls, including
/// widgets and task buttons, rather than relying on undocumented Explorer widths.
pub fn place(
    bar: Bounds,
    occupied: &[Bounds],
    width: i32,
    height: i32,
    gap: i32,
    preference: Edge,
    allow_fallback: bool,
) -> Option<Bounds> {
    // Transposition reuses the same collision rules on vertical taskbars. Left/right
    // preferences become top/bottom; no Explorer child is moved to manufacture space.
    if bar.width() < bar.height() {
        return place(
            bar.transpose(),
            &occupied.iter().map(|r| r.transpose()).collect::<Vec<_>>(),
            height,
            width,
            gap,
            preference,
            allow_fallback,
        )
        .map(Bounds::transpose);
    }
    if width <= 0 || height <= 0 || height > bar.height() {
        return None;
    }
    let mut ranges: Vec<_> = occupied
        .iter()
        .filter(|r| r.bottom > bar.top && r.top < bar.bottom && r.width() > 0)
        .map(|r| ((r.left - gap).max(bar.left), (r.right + gap).min(bar.right)))
        .filter(|(left, right)| left < right)
        .collect();
    ranges.sort_unstable();
    let mut left = bar.left + gap;
    let mut candidate = None;
    for (start, end) in ranges
        .into_iter()
        .chain(std::iter::once((bar.right - gap, bar.right)))
    {
        if start > left && (!allow_fallback || start - left >= width) {
            // Anchor to the requested screen edge within the safe gap. Aligning
            // both modes to the gap's end makes Left follow centered task buttons.
            candidate = Some((
                match preference {
                    Edge::Left => left,
                    Edge::Right => start - width,
                },
                start - left,
            ));
            if preference == Edge::Left {
                break;
            }
        }
        left = left.max(end);
    }
    // A manual edge selects the outer gap even when it is too narrow. Do not
    // silently jump across the task buttons to the opposite side of the screen.
    // Automatic mode may still choose another fitting gap when the shell is busy.
    candidate
        .filter(|(_, capacity)| *capacity >= width)
        .map(|(left, _)| Bounds {
            left,
            right: left + width,
            top: bar.top + (bar.height() - height) / 2,
            bottom: bar.top + (bar.height() - height) / 2 + height,
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn rect(left: i32, right: i32) -> Bounds {
        Bounds {
            left,
            right,
            top: 1040,
            bottom: 1080,
        }
    }
    #[test]
    fn clipped_shell_alignment_uses_the_visible_band_even_when_the_tray_stays_expanded() {
        for dpi in [96, 120, 144, 192, 240] {
            let px = |value| value * dpi / 96;
            let shell = Bounds {
                left: px(-1920),
                top: px(1008),
                right: 0,
                bottom: px(1080),
            };
            let region = Bounds {
                top: px(1032),
                ..shell
            };
            let tray = Bounds {
                left: px(-300),
                ..shell
            };
            let visible = shell.intersection(region).unwrap();
            let band = content_band(visible, Some(tray));
            assert_eq!(band, region);
            let original = place(shell, &[], px(120), px(36), px(4), Edge::Right, false).unwrap();
            assert!(
                !original.fits_in(visible),
                "expanded centering clips the first text row"
            );
            let aligned = align_cross_axis(original, band).unwrap();
            assert_eq!(
                aligned.top,
                region.top + (region.height() - original.height()) / 2
            );
            assert_eq!(
                (aligned.left, aligned.right),
                (original.left, original.right)
            );
            assert!(aligned.fits_in(visible));
            assert_eq!(
                shell.transpose().intersection(region.transpose()),
                Some(visible.transpose())
            );
            assert_eq!(
                align_cross_axis(original.transpose(), band.transpose()),
                Some(aligned.transpose())
            );
        }
        assert!(rect(0, 100).intersection(rect(100, 200)).is_none());
        assert!(rect(0, 100).intersection(Bounds::default()).is_none());
    }
    #[test]
    fn content_alignment_corrects_inset_shells_at_fractional_and_integer_dpi() {
        for dpi in [96, 120, 144, 192, 240] {
            let px = |value| value * dpi / 96;
            let shell = Bounds {
                left: px(-1920),
                top: px(1008),
                right: 0,
                bottom: px(1080),
            };
            let tray = Bounds {
                left: px(-300),
                top: px(1032),
                ..shell
            };
            let placed = place(shell, &[], px(120), px(36), px(4), Edge::Right, false).unwrap();
            let band = content_band(shell, Some(tray));
            let aligned = align_cross_axis(placed, band).unwrap();
            assert_eq!(
                aligned.top,
                tray.top + (tray.height() - placed.height()) / 2
            );
            assert_ne!(aligned.top, placed.top);
            assert_eq!((aligned.left, aligned.right), (placed.left, placed.right));
            assert!(aligned.fits_in(shell));
            assert_eq!(
                align_cross_axis(placed.transpose(), band.transpose()),
                Some(aligned.transpose())
            );
        }
    }

    #[test]
    fn content_alignment_preserves_normal_hosts_and_rejects_unusable_geometry() {
        let host = rect(0, 1920);
        let placed = place(host, &[], 120, 36, 4, Edge::Left, false).unwrap();
        for content in [
            None,
            Some(host),
            Some(Bounds::default()),
            Some(Bounds { top: 1030, ..host }),
        ] {
            assert_eq!(content_band(host, content), host);
            assert_eq!(
                align_cross_axis(placed, content_band(host, content)),
                Some(placed)
            );
        }
        let narrow = Bounds { top: 1060, ..host };
        assert!(align_cross_axis(placed, narrow).is_none());
    }
    #[test]
    fn centered_shell_gaps_exclude_foreign_panels_at_either_edge() {
        let bar = rect(0, 2912);
        let system = [rect(1014, 2000), rect(2364, 2912)];
        for (peer, edge) in [(rect(2, 201), Edge::Left), (rect(2150, 2350), Edge::Right)] {
            let occupied = [system[0], system[1], peer];
            let placed = place(bar, &occupied, 200, 36, 8, edge, false);
            if edge == Edge::Left {
                assert_eq!(placed.unwrap().left, 209);
            } else {
                // The remaining manual-right gap cannot fit both panels. Do not
                // paint over the peer or silently cross the centered task buttons.
                assert!(placed.is_none());
                let automatic = place(bar, &occupied, 200, 36, 8, edge, true).unwrap();
                assert!(automatic.right <= system[0].left - 8);
            }
        }
        // Closing the peer restores the outer gap without retaining its bounds.
        assert_eq!(
            place(bar, &system, 200, 36, 8, Edge::Left, false)
                .unwrap()
                .left,
            8
        );
    }

    #[test]
    fn foreign_monitor_surface_is_avoided_even_without_accessible_controls() {
        // A competing reservation can leave a small task-button area and a
        // large empty tail. The monitor must use that tail without shrinking
        // Explorer again or drawing over the peer's owner-drawn surface.
        let bar = rect(0, 2200);
        let occupied = [rect(0, 437), rect(439, 638)];
        for edge in [Edge::Left, Edge::Right] {
            let bounds = place(bar, &occupied, 344, 36, 8, edge, true).unwrap();
            assert!(bounds.left >= 646);
            assert!(bounds.right <= 2192);
            let vertical: Vec<_> = occupied.iter().map(|r| r.transpose()).collect();
            assert_eq!(
                place(bar.transpose(), &vertical, 36, 344, 8, edge, true),
                Some(bounds.transpose())
            );
        }
        assert!(place(rect(0, 900), &occupied, 344, 36, 8, Edge::Right, true).is_none());
    }

    #[test]
    fn auto_hidden_taskbars_are_outside_the_monitor_even_when_a_thin_edge_remains() {
        let monitor = Bounds {
            left: -1920,
            top: 0,
            right: 0,
            bottom: 1080,
        };
        assert!(rect(-1920, 0).fits_in(monitor));
        assert!(!Bounds {
            top: 1078,
            bottom: 1118,
            ..rect(-1920, 0)
        }
        .fits_in(monitor));
        assert!(!Bounds {
            top: -38,
            bottom: 2,
            ..rect(-1920, 0)
        }
        .fits_in(monitor));
    }
    #[test]
    fn hit_testing_excludes_right_and_bottom_edges() {
        let bounds = rect(10, 30);
        assert!(bounds.contains(10, 1040));
        assert!(!bounds.contains(30, 1040));
        assert!(!bounds.contains(10, 1080));
    }
    #[test]
    fn placement_uses_rightmost_free_gap_without_covering_controls() {
        let bar = rect(0, 1920);
        let occupied = [rect(0, 200), rect(700, 1200), rect(1550, 1920)];
        assert_eq!(
            place(bar, &occupied, 300, 36, 6, Edge::Right, true),
            Some(Bounds {
                left: 1244,
                right: 1544,
                top: 1042,
                bottom: 1078
            })
        );
        assert_eq!(
            place(bar, &occupied, 400, 36, 6, Edge::Right, true)
                .unwrap()
                .left,
            294
        );
        assert!(place(bar, &occupied, 500, 36, 6, Edge::Right, true).is_none());
    }
    #[test]
    fn manual_right_never_falls_back_to_the_left_of_centered_tasks() {
        let bar = rect(0, 1570);
        let occupied = [rect(565, 1006), rect(1232, 1570)];
        assert!(place(bar, &occupied, 262, 36, 4, Edge::Right, false).is_none());
        let compact = place(bar, &occupied, 202, 36, 4, Edge::Right, false).unwrap();
        assert_eq!(compact.right, 1228);
        assert!(compact.left >= 1010);
        assert!(place(bar, &occupied, 262, 36, 4, Edge::Right, true).is_some());
        let vertical: Vec<_> = occupied.iter().map(|r| r.transpose()).collect();
        assert!(place(bar.transpose(), &vertical, 36, 262, 4, Edge::Right, false).is_none());
        assert_eq!(
            place(bar.transpose(), &vertical, 36, 202, 4, Edge::Right, false),
            Some(compact.transpose())
        );
    }

    #[test]
    fn left_anchors_to_the_first_free_edge_and_avoids_system_controls() {
        let bar = rect(0, 1920);
        let occupied = [rect(0, 200), rect(700, 1200), rect(1550, 1920)];
        assert_eq!(
            place(bar, &occupied, 300, 36, 6, Edge::Left, true)
                .unwrap()
                .left,
            206
        );
        let aligned = [rect(0, 1200), rect(1550, 1920)];
        assert_eq!(
            place(bar, &aligned, 300, 36, 6, Edge::Left, true)
                .unwrap()
                .left,
            1206
        );
    }

    #[test]
    fn left_stays_at_the_screen_margin_when_centered_tasks_move() {
        for task_start in [700, 900, 1100] {
            let occupied = [rect(task_start, 1300), rect(1550, 1920)];
            let placed = place(rect(0, 1920), &occupied, 300, 36, 6, Edge::Left, true).unwrap();
            assert_eq!(placed.left, 6);
            assert_eq!(placed.right, 306);
        }
        let negative = place(rect(-1920, 0), &[], 300, 36, 6, Edge::Left, true).unwrap();
        assert_eq!(negative.left, -1914);
    }

    #[test]
    fn side_taskbars_reuse_collision_and_preference_rules_by_transposition() {
        let bar = rect(0, 1920).transpose();
        let occupied = [
            rect(0, 200).transpose(),
            rect(700, 1200).transpose(),
            rect(1550, 1920).transpose(),
        ];
        for preference in [Edge::Left, Edge::Right] {
            let expected = place(
                bar.transpose(),
                &occupied.iter().map(|r| r.transpose()).collect::<Vec<_>>(),
                300,
                36,
                6,
                preference,
                true,
            )
            .unwrap()
            .transpose();
            let actual = place(bar, &occupied, 36, 300, 6, preference, true).unwrap();
            assert_eq!(actual, expected);
            assert_eq!(
                actual.top,
                if preference == Edge::Left { 206 } else { 1244 }
            );
            assert!(actual.fits_in(bar));
            assert!(occupied
                .iter()
                .all(|r| actual.bottom <= r.top || actual.top >= r.bottom));
        }
    }

    #[test]
    fn overlap_negative_coordinates_and_side_taskbars_fail_safely() {
        assert!(place(
            rect(-1920, 0),
            &[rect(-1920, -900), rect(-1000, 0)],
            100,
            30,
            4,
            Edge::Right,
            true
        )
        .is_none());
        let placed = place(
            rect(-1920, 0),
            &[rect(-1920, -1800), rect(-300, 0)],
            300,
            30,
            4,
            Edge::Right,
            true,
        )
        .unwrap();
        assert_eq!(placed.left, -604);
        assert!(place(
            Bounds {
                left: 0,
                top: 0,
                right: 48,
                bottom: 1080
            },
            &[],
            100,
            30,
            4,
            Edge::Right,
            true
        )
        .is_none());
        assert!(place(rect(0, 1920), &[], 100, 50, 4, Edge::Right, true).is_none());
    }
}
