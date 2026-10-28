//! Independent, non-overlapping color conditions with metric-specific units.
use super::preference_schema::DisplayMetricId;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DisplayColor {
    #[default]
    Default,
    Green,
    Yellow,
    Orange,
    Red,
    Blue,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "condition", rename_all = "camelCase", deny_unknown_fields)]
pub enum ColorRule {
    Below {
        value: u16,
        color: DisplayColor,
    },
    Range {
        from: u16,
        to: u16,
        color: DisplayColor,
    },
    AtLeast {
        value: u16,
        color: DisplayColor,
    },
}

impl ColorRule {
    pub fn bounds(&self) -> (u16, Option<u16>) {
        match *self {
            Self::Below { value, .. } => (0, Some(value)),
            Self::Range { from, to, .. } => (from, Some(to)),
            Self::AtLeast { value, .. } => (value, None),
        }
    }

    pub fn color(&self) -> DisplayColor {
        match *self {
            Self::Below { color, .. } | Self::Range { color, .. } | Self::AtLeast { color, .. } => {
                color
            }
        }
    }

    pub fn matches(&self, value: u16) -> bool {
        let (from, to) = self.bounds();
        value >= from && to.is_none_or(|to| value < to)
    }

    fn valid(&self, limit: u16) -> bool {
        let (from, to) = self.bounds();
        from <= limit && to.is_none_or(|to| to <= limit && from < to)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MetricColorRule {
    pub id: DisplayMetricId,
    pub enabled: bool,
    pub rules: Vec<ColorRule>,
}

pub const MAX_COLOR_RULES: usize = 16;
pub const COLOR_METRICS: [DisplayMetricId; 14] = [
    DisplayMetricId::Cpu,
    DisplayMetricId::CpuTemperature,
    DisplayMetricId::Gpu,
    DisplayMetricId::GpuTemperature,
    DisplayMetricId::Memory,
    DisplayMetricId::Disk,
    DisplayMetricId::DiskTemperature,
    DisplayMetricId::BatteryTemperature,
    DisplayMetricId::ChargingPower,
    DisplayMetricId::BatteryPower,
    DisplayMetricId::SystemPower,
    DisplayMetricId::CpuPower,
    DisplayMetricId::GpuPower,
    DisplayMetricId::FanSpeed,
];

pub fn temperature(id: DisplayMetricId) -> bool {
    matches!(
        id,
        DisplayMetricId::DiskTemperature
            | DisplayMetricId::CpuTemperature
            | DisplayMetricId::GpuTemperature
            | DisplayMetricId::BatteryTemperature
    )
}

pub fn limit(id: DisplayMetricId) -> u16 {
    match id {
        DisplayMetricId::FanSpeed => 30000,
        DisplayMetricId::CpuPower | DisplayMetricId::GpuPower => 1000,
        DisplayMetricId::SystemPower => 3000,
        DisplayMetricId::ChargingPower | DisplayMetricId::BatteryPower => 300,
        DisplayMetricId::BatteryTemperature => 100,
        id if temperature(id) => 150,
        _ => 100,
    }
}

impl MetricColorRule {
    pub fn defaults(id: DisplayMetricId) -> Self {
        if matches!(
            id,
            DisplayMetricId::ChargingPower
                | DisplayMetricId::BatteryPower
                | DisplayMetricId::CpuPower
                | DisplayMetricId::GpuPower
                | DisplayMetricId::SystemPower
        ) {
            return Self {
                id,
                enabled: false,
                rules: Vec::new(),
            };
        }
        let (middle, high) = if id == DisplayMetricId::FanSpeed {
            (2000, 4000)
        } else if id == DisplayMetricId::DiskTemperature {
            (50, 60)
        } else if id == DisplayMetricId::BatteryTemperature {
            (40, 45)
        } else if temperature(id) {
            (60, 80)
        } else {
            (70, 90)
        };
        Self {
            id,
            enabled: true,
            rules: vec![
                ColorRule::Below {
                    value: middle,
                    color: DisplayColor::Green,
                },
                ColorRule::Range {
                    from: middle,
                    to: high,
                    color: DisplayColor::Orange,
                },
                ColorRule::AtLeast {
                    value: high,
                    color: DisplayColor::Red,
                },
            ],
        }
    }

    pub fn valid(&self) -> bool {
        let limit = limit(self.id);
        COLOR_METRICS.contains(&self.id)
            && self.rules.len() <= MAX_COLOR_RULES
            && self.rules.iter().all(|rule| rule.valid(limit))
            && self.rules.iter().enumerate().all(|(index, rule)| {
                let (from, to) = rule.bounds();
                self.rules[index + 1..].iter().all(|other| {
                    let (other_from, other_to) = other.bounds();
                    to.is_some_and(|to| to <= other_from) || other_to.is_some_and(|to| to <= from)
                })
            })
    }
}

pub fn defaults() -> Vec<MetricColorRule> {
    COLOR_METRICS
        .into_iter()
        .map(MetricColorRule::defaults)
        .collect()
}

pub fn valid(rules: &[MetricColorRule]) -> bool {
    rules.len() == COLOR_METRICS.len()
        && COLOR_METRICS
            .iter()
            .all(|id| rules.iter().filter(|rule| rule.id == *id).count() == 1)
        && rules.iter().all(MetricColorRule::valid)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fan_defaults_cover_all_supported_rpm_and_reject_out_of_range_thresholds() {
        let mut rule = MetricColorRule::defaults(DisplayMetricId::FanSpeed);
        assert!(rule.enabled && rule.valid());
        for rpm in 0..=30000 {
            let matches = rule
                .rules
                .iter()
                .filter(|rule| rule.matches(rpm))
                .collect::<Vec<_>>();
            assert_eq!(matches.len(), 1, "rpm={rpm}");
            assert_eq!(
                matches[0].color(),
                if rpm < 2000 {
                    DisplayColor::Green
                } else if rpm < 4000 {
                    DisplayColor::Orange
                } else {
                    DisplayColor::Red
                },
                "rpm={rpm}"
            );
        }
        rule.rules = vec![ColorRule::AtLeast {
            value: 30001,
            color: DisplayColor::Red,
        }];
        assert!(!rule.valid());
    }

    #[test]
    fn disk_temperature_defaults_cover_every_celsius_without_overlaps_or_gaps() {
        let defaults = MetricColorRule::defaults(DisplayMetricId::DiskTemperature);
        assert!(defaults.enabled && defaults.valid());
        for value in 0..=150 {
            let matches = defaults
                .rules
                .iter()
                .filter(|rule| rule.matches(value))
                .collect::<Vec<_>>();
            assert_eq!(matches.len(), 1, "celsius={value}");
            let expected = if value < 50 {
                DisplayColor::Green
            } else if value < 60 {
                DisplayColor::Orange
            } else {
                DisplayColor::Red
            };
            assert_eq!(matches[0].color(), expected, "celsius={value}");
        }
    }

    #[test]
    fn battery_temperature_defaults_cover_boundaries_with_independent_power_rules() {
        let rule = MetricColorRule::defaults(DisplayMetricId::BatteryTemperature);
        assert!(rule.enabled && rule.valid());
        for (value, expected) in [
            (0, DisplayColor::Green),
            (39, DisplayColor::Green),
            (40, DisplayColor::Orange),
            (44, DisplayColor::Orange),
            (45, DisplayColor::Red),
            (100, DisplayColor::Red),
        ] {
            assert_eq!(
                rule.rules
                    .iter()
                    .find(|rule| rule.matches(value))
                    .unwrap()
                    .color(),
                expected
            );
        }
        for id in [
            DisplayMetricId::ChargingPower,
            DisplayMetricId::BatteryPower,
            DisplayMetricId::SystemPower,
        ] {
            let power = MetricColorRule::defaults(id);
            assert!(!power.enabled && power.rules.is_empty());
        }
    }

    #[test]
    fn conditions_accept_empty_single_gapped_and_reordered_rules() {
        let mut rule = MetricColorRule::defaults(DisplayMetricId::Cpu);
        assert!(rule.valid());
        rule.rules.reverse();
        assert!(rule.valid());
        rule.rules = vec![
            ColorRule::Below {
                value: 20,
                color: DisplayColor::Green,
            },
            ColorRule::AtLeast {
                value: 90,
                color: DisplayColor::Red,
            },
        ];
        assert!(rule.valid());
        assert!(!rule.rules.iter().any(|rule| rule.matches(70)));
        rule.rules.remove(0);
        assert!(rule.valid());
        rule.rules.clear();
        assert!(rule.valid());
    }

    #[test]
    fn conditions_reject_overlap_empty_ranges_duplicate_metrics_and_wrong_units() {
        let mut rules = defaults();
        rules[0].rules[0] = ColorRule::Below {
            value: 71,
            color: DisplayColor::Green,
        };
        assert!(!valid(&rules));
        rules = defaults();
        rules[0].rules = vec![ColorRule::Range {
            from: 70,
            to: 70,
            color: DisplayColor::Green,
        }];
        assert!(!valid(&rules));
        rules[0].rules = vec![ColorRule::AtLeast {
            value: 120,
            color: DisplayColor::Red,
        }];
        assert!(!valid(&rules));
        rules[1].rules = rules[0].rules.clone();
        assert!(rules[1].valid());
        rules = defaults();
        rules[0].id = DisplayMetricId::Disk;
        assert!(!valid(&rules));
    }
}
