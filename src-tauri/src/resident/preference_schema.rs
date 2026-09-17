//! Versioned user preferences. Unknown or malformed persisted data is never overwritten.
use mangodisk_core::system_resources::metrics::MetricId;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WindowsDisplayMode {
    Tray,
    #[default]
    Taskbar,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TaskbarPosition {
    #[default]
    Auto,
    Left,
    Right,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DisplayMetric {
    pub id: MetricId,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResidentPreferences {
    pub schema_version: u32,
    pub revision: u64,
    pub enabled: bool,
    pub show_icon: bool,
    pub windows_display_mode: WindowsDisplayMode,
    pub taskbar_position: TaskbarPosition,
    pub taskbar_background: bool,
    pub taskbar_compact: bool,
    pub menu_bar_compact: bool,
    pub usage_colors: bool,
    pub usage_warning_percent: u8,
    pub usage_critical_percent: u8,
    pub metrics: Vec<DisplayMetric>,
    pub network_interface: Option<String>,
    pub disk_volume: Option<String>,
}

const DISPLAY_ORDER: [MetricId; 4] = [
    MetricId::Cpu,
    MetricId::Memory,
    MetricId::Disk,
    MetricId::Network,
];

impl Default for ResidentPreferences {
    fn default() -> Self {
        Self {
            schema_version: 8,
            revision: 0,
            enabled: true,
            show_icon: true,
            windows_display_mode: WindowsDisplayMode::default(),
            taskbar_position: TaskbarPosition::default(),
            taskbar_background: true,
            taskbar_compact: false,
            menu_bar_compact: false,
            usage_colors: true,
            usage_warning_percent: 70,
            usage_critical_percent: 90,
            metrics: DISPLAY_ORDER
                .into_iter()
                .map(|id| DisplayMetric {
                    id,
                    enabled: matches!(id, MetricId::Cpu | MetricId::Memory),
                })
                .collect(),
            network_interface: None,
            disk_volume: None,
        }
    }
}

impl ResidentPreferences {
    pub fn shows(&self, id: MetricId) -> bool {
        self.metrics
            .iter()
            .any(|metric| metric.id == id && metric.enabled)
    }

    pub fn effective_icon(&self) -> bool {
        self.show_icon || !self.metrics.iter().any(|metric| metric.enabled)
    }

    pub fn normalize(mut self) -> Result<Self, &'static str> {
        if self.schema_version != 8 {
            return Err("preferences_version");
        }
        if self.usage_warning_percent < 1
            || self.usage_warning_percent >= self.usage_critical_percent
            || self.usage_critical_percent > 100
            || self.metrics.len() > 64
            || [&self.network_interface, &self.disk_volume]
                .iter()
                .any(|id| {
                    id.as_ref()
                        .is_some_and(|id| id.is_empty() || id.len() > 1024 || id.contains('\0'))
                })
        {
            return Err("preferences_invalid");
        }
        let mut seen = Vec::new();
        self.metrics.retain(|metric| {
            if seen.contains(&metric.id) {
                return false;
            }
            seen.push(metric.id);
            true
        });
        for id in DISPLAY_ORDER {
            if !seen.contains(&id) {
                self.metrics.push(DisplayMetric { id, enabled: false });
            }
        }
        Ok(self)
    }
}

pub fn decode(mut value: serde_json::Value) -> Result<ResidentPreferences, &'static str> {
    let version = value.get("schemaVersion").and_then(|v| v.as_u64());
    if matches!(version, Some(2..=7)) {
        for field in [
            "menuBarCompact",
            "usageColors",
            "usageWarningPercent",
            "usageCriticalPercent",
        ] {
            if value.get(field).is_some() {
                return Err("preferences_invalid");
            }
        }
        // Upgrade only the former default sequence; keep deliberate custom order.
        if let Some(metrics) = value.get_mut("metrics").and_then(|v| v.as_array_mut()) {
            let ids: Vec<_> = metrics
                .iter()
                .filter_map(|m| m.get("id").and_then(|id| id.as_str()))
                .collect();
            if ids == ["cpu", "memory", "network", "disk"] {
                metrics.swap(2, 3);
            }
        }
        value["menuBarCompact"] = false.into();
        value["usageColors"] = true.into();
        value["usageWarningPercent"] = 70.into();
        value["usageCriticalPercent"] = 90.into();
    }
    // Versions 2–6 had no density preference. Preserve their appearance and
    // reject fields that did not belong to the declared legacy protocol.
    if matches!(
        value.get("schemaVersion").and_then(|v| v.as_u64()),
        Some(2..=6)
    ) {
        if value.get("taskbarCompact").is_some() {
            return Err("preferences_invalid");
        }
        value["taskbarCompact"] = false.into();
    }
    match value
        .get("schemaVersion")
        .and_then(serde_json::Value::as_u64)
    {
        Some(1) => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Previous {
                schema_version: u32,
                enabled: bool,
                show_memory: bool,
            }
            let previous: Previous =
                serde_json::from_value(value).map_err(|_| "preferences_invalid")?;
            if previous.schema_version != 1 {
                return Err("preferences_version");
            }
            let mut migrated = ResidentPreferences {
                enabled: previous.enabled,
                // Migration preserves the legacy presentation independently of
                // fresh-install defaults, including fields absent in version 1.
                windows_display_mode: WindowsDisplayMode::Tray,
                taskbar_position: TaskbarPosition::Right,
                ..Default::default()
            };
            for metric in &mut migrated.metrics {
                metric.enabled = metric.id == MetricId::Memory && previous.show_memory;
            }
            Ok(migrated)
        }
        Some(2) => {
            // Version 2 has no taskbar mode. Retain every saved selection and the
            // original tray presentation; never opt an existing installation in.
            if value.get("windowsDisplayMode").is_some()
                || value.get("taskbarPosition").is_some()
                || value.get("taskbarBackground").is_some()
            {
                return Err("preferences_invalid");
            }
            value["schemaVersion"] = 8.into();
            value["taskbarBackground"] = true.into();
            value["windowsDisplayMode"] = "tray".into();
            value["taskbarPosition"] = "right".into();
            serde_json::from_value::<ResidentPreferences>(value)
                .map_err(|_| "preferences_invalid")?
                .normalize()
        }
        Some(3) => {
            if value.get("taskbarPosition").is_some() || value.get("taskbarBackground").is_some() {
                return Err("preferences_invalid");
            }
            value["schemaVersion"] = 8.into();
            value["taskbarBackground"] = true.into();
            value["taskbarPosition"] = "right".into();
            serde_json::from_value::<ResidentPreferences>(value)
                .map_err(|_| "preferences_invalid")?
                .normalize()
        }
        Some(4) => {
            if !matches!(
                value.get("taskbarPosition").and_then(|v| v.as_str()),
                Some("left" | "right")
            ) {
                return Err("preferences_invalid");
            }
            if value.get("taskbarBackground").is_some() {
                return Err("preferences_invalid");
            }
            // Existing installations keep the opaque presentation until the
            // user explicitly selects transparency.
            value["schemaVersion"] = 8.into();
            value["taskbarBackground"] = true.into();
            serde_json::from_value::<ResidentPreferences>(value)
                .map_err(|_| "preferences_invalid")?
                .normalize()
        }
        Some(5) => {
            // Version 5 predates automatic placement. Do not infer whether a saved
            // left/right value was a default or an explicit user decision.
            if !matches!(
                value.get("taskbarPosition").and_then(|v| v.as_str()),
                Some("left" | "right")
            ) {
                return Err("preferences_invalid");
            }
            value["schemaVersion"] = 8.into();
            serde_json::from_value::<ResidentPreferences>(value)
                .map_err(|_| "preferences_invalid")?
                .normalize()
        }
        Some(6) => {
            value["schemaVersion"] = 8.into();
            serde_json::from_value::<ResidentPreferences>(value)
                .map_err(|_| "preferences_invalid")?
                .normalize()
        }
        Some(7 | 8) => {
            value["schemaVersion"] = 8.into();
            serde_json::from_value::<ResidentPreferences>(value)
                .map_err(|_| "preferences_invalid")?
                .normalize()
        }
        _ => Err("preferences_version"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn legacy_preferences() -> serde_json::Value {
        let mut value = serde_json::to_value(ResidentPreferences::default()).unwrap();
        value["schemaVersion"] = 7.into();
        for field in [
            "menuBarCompact",
            "usageColors",
            "usageWarningPercent",
            "usageCriticalPercent",
        ] {
            value.as_object_mut().unwrap().remove(field);
        }
        value
    }

    #[test]
    fn usage_colors_default_on_without_overwriting_saved_opt_out() {
        let defaults = ResidentPreferences::default();
        assert!(defaults.usage_colors);
        assert_eq!(defaults.usage_warning_percent, 70);
        assert_eq!(defaults.usage_critical_percent, 90);
        let saved = ResidentPreferences {
            usage_colors: false,
            usage_warning_percent: 55,
            usage_critical_percent: 80,
            ..defaults
        };
        let restored = decode(serde_json::to_value(&saved).unwrap()).unwrap();
        assert_eq!(restored, saved);
        assert_eq!(
            decode(serde_json::to_value(&restored).unwrap()).unwrap(),
            saved
        );
    }

    #[test]
    fn version_seven_preserves_order_and_adds_display_options() {
        let mut old = legacy_preferences();
        old["metrics"].as_array_mut().unwrap().reverse();
        let order = old["metrics"].clone();
        let migrated = decode(old).unwrap();
        assert_eq!(migrated.schema_version, 8);
        assert!(!migrated.menu_bar_compact);
        assert!(migrated.usage_colors);
        assert_eq!(serde_json::to_value(migrated).unwrap()["metrics"], order);
        assert_eq!(
            ResidentPreferences::default()
                .metrics
                .iter()
                .map(|m| m.id)
                .collect::<Vec<_>>(),
            DISPLAY_ORDER
        );
    }

    #[test]
    fn legacy_default_order_moves_network_last_without_changing_enabled_flags() {
        let mut old = legacy_preferences();
        old["metrics"] = serde_json::json!([
            {"id":"cpu", "enabled":false}, {"id":"memory", "enabled":true},
            {"id":"network", "enabled":true}, {"id":"disk", "enabled":false}
        ]);
        let migrated = decode(old).unwrap();
        assert_eq!(
            migrated.metrics.iter().map(|m| m.id).collect::<Vec<_>>(),
            DISPLAY_ORDER
        );
        assert!(migrated.shows(MetricId::Network));
        assert!(!migrated.shows(MetricId::Disk));
    }

    #[test]
    fn usage_thresholds_reject_invalid_ranges_and_round_trip() {
        for (warning, critical) in [(0, 90), (90, 90), (95, 90), (70, 101)] {
            assert!(ResidentPreferences {
                usage_warning_percent: warning,
                usage_critical_percent: critical,
                ..Default::default()
            }
            .normalize()
            .is_err());
        }
        let settings = ResidentPreferences {
            menu_bar_compact: true,
            usage_warning_percent: 60,
            usage_critical_percent: 85,
            ..Default::default()
        };
        assert_eq!(
            decode(serde_json::to_value(&settings).unwrap()).unwrap(),
            settings
        );
        let mut malformed = legacy_preferences();
        malformed["usageColors"] = true.into();
        assert!(decode(malformed).is_err());
    }

    #[test]
    fn version_five_preserves_manual_positions_and_current_auto_round_trips() {
        for position in ["left", "right"] {
            let mut old = legacy_preferences();
            old["schemaVersion"] = 5.into();
            old.as_object_mut().unwrap().remove("taskbarCompact");
            old["taskbarPosition"] = position.into();
            let migrated = decode(old).unwrap();
            assert_eq!(migrated.schema_version, 8);
            assert_eq!(
                serde_json::to_value(migrated).unwrap()["taskbarPosition"],
                position
            );
        }
        let current = ResidentPreferences::default();
        assert_eq!(
            decode(serde_json::to_value(&current).unwrap()).unwrap(),
            current
        );
    }

    #[test]
    fn version_six_preserves_automatic_position_and_defaults_to_standard_density() {
        let mut old = legacy_preferences();
        old["schemaVersion"] = 6.into();
        old.as_object_mut().unwrap().remove("taskbarCompact");
        let migrated = decode(old).unwrap();
        assert_eq!(migrated.taskbar_position, TaskbarPosition::Auto);
        assert!(!migrated.taskbar_compact);
        let mut compact = migrated;
        compact.taskbar_compact = true;
        assert_eq!(
            decode(serde_json::to_value(&compact).unwrap()).unwrap(),
            compact
        );
    }

    #[test]
    fn fresh_preferences_show_logo_cpu_memory_with_automatic_taskbar_position() {
        let preferences = ResidentPreferences::default();
        assert!(preferences.enabled && preferences.show_icon && preferences.taskbar_background);
        assert_eq!(
            preferences.windows_display_mode,
            WindowsDisplayMode::Taskbar
        );
        assert_eq!(preferences.taskbar_position, TaskbarPosition::Auto);
        assert_eq!(
            preferences
                .metrics
                .iter()
                .filter(|metric| metric.enabled)
                .map(|metric| metric.id)
                .collect::<Vec<_>>(),
            vec![MetricId::Cpu, MetricId::Memory]
        );
        assert!(preferences.network_interface.is_none() && preferences.disk_volume.is_none());
    }

    #[test]
    fn saved_preferences_are_not_replaced_by_new_install_defaults() {
        let mut saved = ResidentPreferences {
            windows_display_mode: WindowsDisplayMode::Tray,
            taskbar_position: TaskbarPosition::Right,
            enabled: false,
            show_icon: false,
            taskbar_background: false,
            ..Default::default()
        };
        for metric in &mut saved.metrics {
            metric.enabled = matches!(metric.id, MetricId::Network | MetricId::Disk);
        }
        assert_eq!(
            decode(serde_json::to_value(&saved).unwrap()).unwrap(),
            saved
        );
    }

    #[test]
    fn migration_preserves_old_background_and_memory_choices() {
        for enabled in [false, true] {
            for memory in [false, true] {
                let migrated = decode(
                    serde_json::json!({"schemaVersion":1,"enabled":enabled,"showMemory":memory}),
                )
                .unwrap();
                assert_eq!(migrated.windows_display_mode, WindowsDisplayMode::Tray);
                assert_eq!(migrated.taskbar_position, TaskbarPosition::Right);
                assert_eq!(migrated.enabled, enabled);
                assert_eq!(migrated.shows(MetricId::Memory), memory);
                assert!(migrated.show_icon);
                assert!(!migrated.shows(MetricId::Cpu));
                assert_eq!(migrated.schema_version, 8);
            }
        }
    }

    #[test]
    fn version_two_migrates_to_tray_without_changing_choices() {
        let mut old = legacy_preferences();
        old["schemaVersion"] = 2.into();
        old.as_object_mut().unwrap().remove("taskbarCompact");
        old.as_object_mut().unwrap().remove("taskbarBackground");
        old["revision"] = 17.into();
        old["showIcon"] = false.into();
        old.as_object_mut().unwrap().remove("windowsDisplayMode");
        old.as_object_mut().unwrap().remove("taskbarPosition");
        let migrated = decode(old).unwrap();
        assert_eq!(migrated.windows_display_mode, WindowsDisplayMode::Tray);
        assert_eq!(migrated.revision, 17);
        assert!(!migrated.show_icon);
        assert_eq!(
            decode(serde_json::to_value(&migrated).unwrap()).unwrap(),
            migrated
        );
    }

    #[test]
    fn version_three_preserves_taskbar_mode_and_defaults_to_right() {
        let mut old = legacy_preferences();
        old["schemaVersion"] = 3.into();
        old.as_object_mut().unwrap().remove("taskbarCompact");
        old.as_object_mut().unwrap().remove("taskbarBackground");
        old["windowsDisplayMode"] = "taskbar".into();
        old["revision"] = 42.into();
        old.as_object_mut().unwrap().remove("taskbarPosition");
        let migrated = decode(old).unwrap();
        assert_eq!(migrated.taskbar_position, TaskbarPosition::Right);
        assert_eq!(migrated.windows_display_mode, WindowsDisplayMode::Taskbar);
        assert_eq!(migrated.revision, 42);
        let mut current = serde_json::to_value(migrated).unwrap();
        current["taskbarPosition"] = "left".into();
        assert_eq!(
            decode(current.clone()).unwrap().taskbar_position,
            TaskbarPosition::Left
        );
        current["taskbarPosition"] = "middle".into();
        assert!(decode(current).is_err());
    }

    #[test]
    fn version_four_keeps_background_and_current_preserves_transparency() {
        let mut old = legacy_preferences();
        old["schemaVersion"] = 4.into();
        old.as_object_mut().unwrap().remove("taskbarCompact");
        old["taskbarPosition"] = "left".into();
        old["windowsDisplayMode"] = "taskbar".into();
        old.as_object_mut().unwrap().remove("taskbarBackground");
        let migrated = decode(old.clone()).unwrap();
        assert!(migrated.taskbar_background);
        assert_eq!(migrated.taskbar_position, TaskbarPosition::Left);
        assert_eq!(migrated.windows_display_mode, WindowsDisplayMode::Taskbar);
        let mut current = serde_json::to_value(migrated).unwrap();
        current["taskbarBackground"] = false.into();
        assert!(!decode(current.clone()).unwrap().taskbar_background);
        current["taskbarBackground"] = "false".into();
        assert!(decode(current).is_err());
        old["taskbarBackground"] = false.into();
        assert!(decode(old).is_err());
    }

    #[test]
    fn duplicate_ids_preserve_first_order_and_missing_items_are_disabled() {
        let preferences = ResidentPreferences {
            metrics: vec![
                DisplayMetric {
                    id: MetricId::Network,
                    enabled: true,
                },
                DisplayMetric {
                    id: MetricId::Network,
                    enabled: false,
                },
            ],
            ..Default::default()
        }
        .normalize()
        .unwrap();
        assert_eq!(preferences.metrics.len(), 4);
        assert_eq!(preferences.metrics[0].id, MetricId::Network);
        assert!(preferences.shows(MetricId::Network));
        assert!(!preferences.shows(MetricId::Memory));
    }

    #[test]
    fn unknown_versions_and_invalid_types_are_rejected_without_migration() {
        for value in [
            serde_json::json!({"schemaVersion":99}),
            serde_json::json!({"schemaVersion":1,"enabled":"true","showMemory":false}),
            serde_json::json!({"schemaVersion":1,"enabled":true}),
        ] {
            assert!(decode(value).is_err());
        }
        let mut value = serde_json::to_value(ResidentPreferences::default()).unwrap();
        value["metrics"][0]["id"] = "gpu".into();
        assert!(decode(value).is_err());
    }

    #[test]
    fn every_display_combination_retains_an_entry() {
        for bits in 0..16 {
            for show_icon in [false, true] {
                let mut preferences = ResidentPreferences {
                    show_icon,
                    ..Default::default()
                };
                for (index, metric) in preferences.metrics.iter_mut().enumerate() {
                    metric.enabled = bits & (1 << index) != 0;
                }
                assert_eq!(preferences.effective_icon(), show_icon || bits == 0);
                assert!(
                    preferences.effective_icon()
                        || preferences.metrics.iter().any(|metric| metric.enabled)
                );
            }
        }
    }
}
