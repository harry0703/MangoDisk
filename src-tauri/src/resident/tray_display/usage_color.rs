//! Shared usage tones; native adapters own the actual glyph rendering.
use serde::Serialize;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum UsageTone {
    #[default]
    Normal,
    Warning,
    Critical,
}

impl UsageTone {
    /// Enter a higher band immediately, but require a three-point fall before
    /// clearing it. This avoids flickering when samples straddle a threshold.
    pub fn next(self, percent: Option<u8>, warning: u8, critical: u8) -> Self {
        let Some(percent) = percent else {
            return Self::Normal;
        };
        if percent == 0 {
            return Self::Normal;
        }
        if percent >= critical || (self == Self::Critical && percent > critical.saturating_sub(3)) {
            Self::Critical
        } else if percent >= warning
            || (self != Self::Normal && percent > warning.saturating_sub(3))
        {
            Self::Warning
        } else {
            Self::Normal
        }
    }

    #[cfg(any(test, windows, target_os = "macos"))]
    pub fn rgb(self, foreground: [u8; 3]) -> [u8; 3] {
        // Darker hues stay legible on light taskbars; lighter hues serve dark
        // surfaces. Normal and unavailable readings keep the system foreground.
        let dark =
            u16::from(foreground[0]) + u16::from(foreground[1]) + u16::from(foreground[2]) > 384;
        match (self, dark) {
            (Self::Normal, _) => foreground,
            (Self::Warning, false) => [166, 77, 0],
            (Self::Warning, true) => [255, 190, 92],
            (Self::Critical, false) => [194, 34, 42],
            (Self::Critical, true) => [255, 112, 117],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::UsageTone::*;
    #[test]
    fn threshold_crossings_have_hysteresis_and_unavailable_resets() {
        assert_eq!(Normal.next(Some(69), 70, 90), Normal);
        assert_eq!(Normal.next(Some(70), 70, 90), Warning);
        assert_eq!(Warning.next(Some(69), 70, 90), Warning);
        assert_eq!(Warning.next(Some(68), 70, 90), Warning);
        assert_eq!(Warning.next(Some(67), 70, 90), Normal);
        assert_eq!(Normal.next(Some(90), 70, 90), Critical);
        assert_eq!(Critical.next(Some(88), 70, 90), Critical);
        assert_eq!(Critical.next(Some(87), 70, 90), Warning);
        assert_eq!(Critical.next(Some(10), 70, 90), Normal);
        assert_eq!(Critical.next(None, 70, 90), Normal);
        assert_eq!(Normal.next(Some(100), 1, 100), Critical);
        assert_eq!(Warning.next(Some(0), 1, 90), Normal);
        assert_eq!(Critical.next(Some(0), 1, 2), Normal);
    }
    #[test]
    #[cfg(any(test, windows, target_os = "macos"))]
    fn normal_colors_follow_the_theme_and_bands_remain_distinct() {
        for foreground in [[32; 3], [245; 3]] {
            assert_eq!(Normal.rgb(foreground), foreground);
            assert_ne!(Warning.rgb(foreground), Critical.rgb(foreground));
            assert_ne!(Warning.rgb(foreground), foreground);
        }
    }
}
