//! Direct upgrades from shipped preference structures; development schemas are not contracts.
use super::{
    DisplayMetric, DisplayMetricId, GpuDisplaySelection, ResidentPreferences, DISPLAY_ORDER,
    PREFERENCE_SCHEMA_VERSION,
};
use crate::resident::color_rules::{self, ColorRule, DisplayColor, MetricColorRule};
use serde_json::{Map, Value};

// Release tags v1.1.1–v1.1.9 persisted only these five structures.
const RELEASED_VERSIONS: [u64; 5] = [7, 8, 10, 21, 25];

pub fn decode(mut value: Value) -> Result<ResidentPreferences, &'static str> {
    let version = value
        .get("schemaVersion")
        .and_then(Value::as_u64)
        .ok_or("preferences_version")?;
    if version == u64::from(PREFERENCE_SCHEMA_VERSION) {
        return serde_json::from_value::<ResidentPreferences>(value)
            .map_err(|_| "preferences_invalid")?
            .normalize();
    }
    if version == 41 {
        let object = value.as_object_mut().ok_or("preferences_invalid")?;
        object.insert("schemaVersion".into(), PREFERENCE_SCHEMA_VERSION.into());
        object.entry("taskbarMonitor").or_insert(Value::Null);
        return serde_json::from_value::<ResidentPreferences>(value)
            .map_err(|_| "preferences_invalid")?
            .normalize();
    }
    if version == 40 {
        let object = value.as_object_mut().ok_or("preferences_invalid")?;
        let network = object
            .get_mut("publicNetwork")
            .and_then(Value::as_object_mut)
            .ok_or("preferences_invalid")?;
        network
            .remove("refreshMinutes")
            .and_then(|value| value.as_u64())
            .filter(|minutes| [0, 1, 5, 15, 30, 60].contains(minutes))
            .ok_or("preferences_invalid")?;
        object.insert("schemaVersion".into(), PREFERENCE_SCHEMA_VERSION.into());
        return serde_json::from_value::<ResidentPreferences>(value)
            .map_err(|_| "preferences_invalid")?
            .normalize();
    }
    if matches!(version, 38 | 39) {
        value
            .as_object_mut()
            .ok_or("preferences_invalid")?
            .insert("schemaVersion".into(), PREFERENCE_SCHEMA_VERSION.into());
        return decode_upgraded(value);
    }
    if RELEASED_VERSIONS.contains(&version) || matches!(version, 28..=36) {
        let object = value.as_object_mut().ok_or("preferences_invalid")?;
        if object
            .insert("linuxDisplayMode".into(), "tray".into())
            .is_some()
        {
            return Err("preferences_invalid");
        }
    }
    if matches!(version, 28..=37) {
        add_component_defaults(&mut value)?;
        add_frequency_defaults(value.as_object_mut().ok_or("preferences_invalid")?)?;
    }
    if matches!(version, 34..=37) {
        let object = value.as_object_mut().ok_or("preferences_invalid")?;
        if version == 34 {
            add_macos_defaults(object)?;
        }
        if version < 36 {
            add_fullscreen_default(object)?;
        }
        object.insert("schemaVersion".into(), PREFERENCE_SCHEMA_VERSION.into());
        return decode_upgraded(value);
    }
    if matches!(version, 29..=33) {
        let object = value.as_object_mut().ok_or("preferences_invalid")?;
        let floating = object
            .get_mut("floating")
            .and_then(Value::as_object_mut)
            .ok_or("preferences_invalid")?;
        if version < 31 && floating.contains_key("snapToEdges") {
            return Err("preferences_invalid");
        }
        if version == 29 && floating.contains_key("backgroundOpacity") {
            return Err("preferences_invalid");
        }
        if version == 29 {
            floating.insert("backgroundOpacity".into(), 100.into());
        }
        if version < 31 {
            floating.insert("snapToEdges".into(), true.into());
        }
        if floating
            .insert("clickThrough".into(), false.into())
            .is_some()
        {
            return Err("preferences_invalid");
        }
        if version < 33 && floating.insert("dividers".into(), true.into()).is_some() {
            return Err("preferences_invalid");
        }
        if version < 32 {
            for (key, value) in [
                ("fontSize", Value::from("standard")),
                ("bold", Value::from(false)),
                ("background", Value::from("default")),
            ] {
                if floating.insert(key.into(), value).is_some() {
                    return Err("preferences_invalid");
                }
            }
        }
        add_macos_defaults(object)?;
        add_fullscreen_default(object)?;
        object.insert("schemaVersion".into(), PREFERENCE_SCHEMA_VERSION.into());
        return decode_upgraded(value);
    }
    if version == 28 {
        let object = value.as_object_mut().ok_or("preferences_invalid")?;
        add_floating_defaults(object)?;
        object.insert("schemaVersion".into(), PREFERENCE_SCHEMA_VERSION.into());
        return decode_upgraded(value);
    }
    if !RELEASED_VERSIONS.contains(&version) {
        return Err("preferences_version");
    }
    let object = value.as_object_mut().ok_or("preferences_invalid")?;
    // Reject fields absent from the declared release before adding current defaults.
    for field in [
        "gpuDisplay",
        "gpuTemperatureDisplay",
        "diskTemperatureDevice",
    ] {
        if object.contains_key(field) {
            return Err("preferences_invalid");
        }
    }
    if version < 25 {
        for field in ["fanDisplay", "fanIds"] {
            if object.contains_key(field) {
                return Err("preferences_invalid");
            }
        }
        object.insert("fanDisplay".into(), "highest".into());
        object.insert("fanIds".into(), serde_json::json!([0]));
    }
    if version <= 10 {
        upgrade_percentage_display(object, version)?;
    }
    upgrade_metrics(object, version)?;
    if version == 21 {
        upgrade_color_rules(object, version)?;
    }
    let selection = match object.get("gpuAdapter").and_then(Value::as_str) {
        Some(id) => GpuDisplaySelection::Selected {
            adapter_ids: vec![id.into()],
        },
        None => GpuDisplaySelection::Automatic,
    };
    object.insert(
        "gpuDisplay".into(),
        serde_json::to_value(selection).map_err(|_| "preferences_invalid")?,
    );
    // The former single GPU choice also controlled temperature; automatic temperature
    // now follows the same menu-bar selection without coupling it to detail navigation.
    object.insert(
        "gpuTemperatureDisplay".into(),
        serde_json::to_value(GpuDisplaySelection::Automatic).map_err(|_| "preferences_invalid")?,
    );
    object.insert("diskTemperatureDevice".into(), Value::Null);
    add_floating_defaults(object)?;
    object.insert("schemaVersion".into(), PREFERENCE_SCHEMA_VERSION.into());
    decode_upgraded(value)
}

fn add_frequency_defaults(object: &mut Map<String, Value>) -> Result<(), &'static str> {
    let mut metrics: Vec<DisplayMetric> = serde_json::from_value(
        object
            .get("metrics")
            .cloned()
            .ok_or("preferences_invalid")?,
    )
    .map_err(|_| "preferences_invalid")?;
    let previous: Vec<_> = DISPLAY_ORDER
        .into_iter()
        .filter(|id| id.frequency_class().is_none())
        .collect();
    if metrics.len() != previous.len()
        || previous
            .iter()
            .any(|id| metrics.iter().filter(|metric| metric.id == *id).count() != 1)
    {
        return Err("preferences_invalid");
    }
    let index = metrics
        .iter()
        .position(|metric| metric.id == DisplayMetricId::Cpu)
        .ok_or("preferences_invalid")?
        + 1;
    metrics.splice(
        index..index,
        DISPLAY_ORDER
            .into_iter()
            .filter(|id| id.frequency_class().is_some())
            .map(|id| DisplayMetric { id, enabled: false }),
    );
    object.insert(
        "metrics".into(),
        serde_json::to_value(metrics).map_err(|_| "preferences_invalid")?,
    );
    Ok(())
}

fn upgrade_percentage_display(
    object: &mut Map<String, Value>,
    version: u64,
) -> Result<(), &'static str> {
    if object.contains_key("windowsCpuTemperatureEnabled") || object.contains_key("colorRules") {
        return Err("preferences_invalid");
    }
    match object
        .remove("windowsDisplayMode")
        .as_ref()
        .and_then(Value::as_str)
    {
        Some("tray" | "taskbar") => {}
        _ => return Err("preferences_invalid"),
    }
    object.insert("windowsCpuTemperatureEnabled".into(), false.into());
    if version < 10 {
        if object.contains_key("gpuAdapter") {
            return Err("preferences_invalid");
        }
        object.insert("gpuAdapter".into(), Value::Null);
    }
    if version == 7 {
        for field in [
            "menuBarCompact",
            "usageColors",
            "usageWarningPercent",
            "usageCriticalPercent",
        ] {
            if object.contains_key(field) {
                return Err("preferences_invalid");
            }
        }
        object.insert("menuBarCompact".into(), false.into());
        object.insert("usageColors".into(), true.into());
    }
    let (warning, critical) = if version == 7 {
        (70, 90)
    } else {
        let warning = object
            .remove("usageWarningPercent")
            .and_then(|v| v.as_u64())
            .ok_or("preferences_invalid")?;
        let critical = object
            .remove("usageCriticalPercent")
            .and_then(|v| v.as_u64())
            .ok_or("preferences_invalid")?;
        if warning < 1 || warning >= critical || critical > 100 {
            return Err("preferences_invalid");
        }
        (warning as u16, critical as u16)
    };
    object.insert(
        "colorRules".into(),
        serde_json::to_value(percentage_colors(warning, critical))
            .map_err(|_| "preferences_invalid")?,
    );
    Ok(())
}

fn upgrade_metrics(object: &mut Map<String, Value>, version: u64) -> Result<(), &'static str> {
    let mut metrics: Vec<DisplayMetric> = serde_json::from_value(
        object
            .get("metrics")
            .cloned()
            .ok_or("preferences_invalid")?,
    )
    .map_err(|_| "preferences_invalid")?;
    let expected: Vec<_> = DISPLAY_ORDER
        .into_iter()
        .filter(|id| released_metric(version, *id))
        .collect();
    if metrics.len() != expected.len()
        || expected
            .iter()
            .any(|id| metrics.iter().filter(|m| m.id == *id).count() != 1)
    {
        return Err("preferences_invalid");
    }
    // Keep every saved metric's relative position and enabled state. Newly introduced
    // metrics stay off and sit before a trailing network column where possible.
    for id in DISPLAY_ORDER {
        if !expected.contains(&id) {
            let index = if id == DisplayMetricId::DiskTemperature {
                metrics
                    .iter()
                    .position(|m| m.id == DisplayMetricId::Disk)
                    .map_or(metrics.len(), |i| i + 1)
            } else if metrics
                .last()
                .is_some_and(|m| m.id == DisplayMetricId::Network)
            {
                metrics.len() - 1
            } else {
                metrics.len()
            };
            metrics.insert(index, DisplayMetric { id, enabled: false });
        }
    }
    object.insert(
        "metrics".into(),
        serde_json::to_value(metrics).map_err(|_| "preferences_invalid")?,
    );
    Ok(())
}

fn upgrade_color_rules(object: &mut Map<String, Value>, version: u64) -> Result<(), &'static str> {
    let mut rules: Vec<MetricColorRule> = serde_json::from_value(
        object
            .get("colorRules")
            .cloned()
            .ok_or("preferences_invalid")?,
    )
    .map_err(|_| "preferences_invalid")?;
    let expected: Vec<_> = color_rules::COLOR_METRICS
        .into_iter()
        .filter(|id| released_metric(version, *id))
        .collect();
    if rules.len() != expected.len()
        || expected
            .iter()
            .any(|id| rules.iter().filter(|r| r.id == *id).count() != 1)
        || rules.iter().any(|rule| !rule.valid())
    {
        return Err("preferences_invalid");
    }
    rules.extend(
        [DisplayMetricId::DiskTemperature, DisplayMetricId::FanSpeed]
            .map(MetricColorRule::defaults),
    );
    object.insert(
        "colorRules".into(),
        serde_json::to_value(rules).map_err(|_| "preferences_invalid")?,
    );
    Ok(())
}

fn released_metric(version: u64, id: DisplayMetricId) -> bool {
    if matches!(id, DisplayMetricId::CpuPower | DisplayMetricId::GpuPower) {
        return false;
    }
    if id.frequency_class().is_some() {
        return false;
    }
    match version {
        7 | 8 => matches!(
            id,
            DisplayMetricId::Cpu
                | DisplayMetricId::Memory
                | DisplayMetricId::Disk
                | DisplayMetricId::Network
        ),
        10 => matches!(
            id,
            DisplayMetricId::Cpu
                | DisplayMetricId::Gpu
                | DisplayMetricId::Memory
                | DisplayMetricId::Disk
                | DisplayMetricId::Network
        ),
        21 => !matches!(
            id,
            DisplayMetricId::DiskTemperature | DisplayMetricId::FanSpeed
        ),
        25 => true,
        _ => false,
    }
}

fn percentage_colors(warning: u16, critical: u16) -> Vec<MetricColorRule> {
    color_rules::defaults()
        .into_iter()
        .map(|mut rule| {
            if matches!(
                rule.id,
                DisplayMetricId::CpuTemperature | DisplayMetricId::GpuTemperature
            ) {
                rule.enabled = false;
            } else if matches!(
                rule.id,
                DisplayMetricId::Cpu
                    | DisplayMetricId::Gpu
                    | DisplayMetricId::Memory
                    | DisplayMetricId::Disk
            ) {
                rule.rules = vec![
                    ColorRule::Below {
                        value: warning,
                        color: DisplayColor::Default,
                    },
                    ColorRule::Range {
                        from: warning,
                        to: critical,
                        color: DisplayColor::Orange,
                    },
                    ColorRule::AtLeast {
                        value: critical,
                        color: DisplayColor::Red,
                    },
                ];
            }
            rule
        })
        .collect()
}

fn add_floating_defaults(object: &mut Map<String, Value>) -> Result<(), &'static str> {
    add_macos_defaults(object)?;
    if object.contains_key("windowsDisplayMode") || object.contains_key("floating") {
        return Err("preferences_invalid");
    }
    object.insert("windowsDisplayMode".into(), "taskbar".into());
    object.insert(
        "floating".into(),
        serde_json::to_value(super::FloatingPreferences::default())
            .map_err(|_| "preferences_invalid")?,
    );
    Ok(())
}

fn add_macos_defaults(object: &mut Map<String, Value>) -> Result<(), &'static str> {
    if object
        .insert("macosDisplayMode".into(), "menuBar".into())
        .is_some()
    {
        return Err("preferences_invalid");
    }
    Ok(())
}

fn add_fullscreen_default(object: &mut Map<String, Value>) -> Result<(), &'static str> {
    let floating = object
        .get_mut("floating")
        .and_then(Value::as_object_mut)
        .ok_or("preferences_invalid")?;
    if floating
        .insert("hideOnFullscreen".into(), true.into())
        .is_some()
    {
        return Err("preferences_invalid");
    }
    Ok(())
}

fn add_component_defaults(value: &mut Value) -> Result<(), &'static str> {
    for key in ["metrics", "colorRules"] {
        let rows = value
            .get_mut(key)
            .and_then(Value::as_array_mut)
            .ok_or("preferences_invalid")?;
        for (id, anchor) in [
            (DisplayMetricId::CpuPower, DisplayMetricId::CpuTemperature),
            (DisplayMetricId::GpuPower, DisplayMetricId::GpuTemperature),
        ] {
            let id_value = serde_json::to_value(id).map_err(|_| "preferences_invalid")?;
            if !rows.iter().any(|row| row.get("id") == Some(&id_value)) {
                if key == "metrics" {
                    let anchor = serde_json::to_value(anchor).map_err(|_| "preferences_invalid")?;
                    let index = rows
                        .iter()
                        .position(|row| row.get("id") == Some(&anchor))
                        .ok_or("preferences_invalid")?
                        + 1;
                    rows.insert(index, serde_json::json!({"id": id_value, "enabled": false}));
                } else {
                    rows.push(serde_json::json!({"id": id_value, "enabled": false, "rules": []}));
                }
            }
        }
    }
    Ok(())
}
fn decode_upgraded(mut value: Value) -> Result<ResidentPreferences, &'static str> {
    value.as_object_mut().ok_or("preferences_invalid")?.insert(
        "publicNetwork".into(),
        serde_json::to_value(super::PublicNetworkPreferences::default())
            .map_err(|_| "preferences_invalid")?,
    );
    value
        .as_object_mut()
        .ok_or("preferences_invalid")?
        .entry("taskbarMonitor")
        .or_insert(Value::Null);
    add_component_defaults(&mut value)?;
    serde_json::from_value::<ResidentPreferences>(value)
        .map_err(|_| "preferences_invalid")?
        .normalize()
}
