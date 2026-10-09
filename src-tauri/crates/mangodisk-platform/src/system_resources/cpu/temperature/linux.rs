//! Keep the hwmon driver identity: generic thermal zones are not CPU sensors.
use super::{unsupported, valid_celsius, CpuTemperature, CpuTemperatureKind, CpuTemperatureSource};
use crate::{PlatformError, PlatformErrorCode, PlatformResult};

pub(super) struct Reader {
    components: sysinfo::Components,
    labels: Vec<String>,
    kind: CpuTemperatureKind,
}
impl Reader {
    pub(super) fn new() -> PlatformResult<Self> {
        let components = sysinfo::Components::new_with_refreshed_list();
        let kind = if components
            .iter()
            .any(|c| classify(c.label()) == Some(CpuTemperatureKind::Package))
        {
            CpuTemperatureKind::Package
        } else {
            CpuTemperatureKind::CoreMaximum
        };
        let labels = components
            .iter()
            .filter(|c| classify(c.label()) == Some(kind))
            .map(|c| c.label().to_owned())
            .collect::<Vec<_>>();
        if labels.is_empty() {
            return Err(unsupported());
        }
        Ok(Self {
            components,
            labels,
            kind,
        })
    }
    pub(super) fn read(&mut self) -> PlatformResult<CpuTemperature> {
        self.components.refresh(true);
        // Full-list refresh retains the last temperature if a new sysfs read
        // fails. Direct refresh clears that cached value instead.
        for component in self
            .components
            .iter_mut()
            .filter(|component| self.labels.iter().any(|label| label == component.label()))
        {
            component.refresh();
        }
        let mut maximum = f64::NEG_INFINITY;
        for label in &self.labels {
            let value = self
                .components
                .iter()
                .find(|c| c.label() == label)
                .and_then(|c| c.temperature())
                .map(f64::from)
                .filter(|v| valid_celsius(*v))
                .ok_or_else(|| {
                    PlatformError::new(
                        PlatformErrorCode::InvalidData,
                        "CPU hwmon sensor missing or invalid",
                    )
                })?;
            maximum = maximum.max(value);
        }
        Ok(CpuTemperature {
            celsius: maximum,
            kind: self.kind,
            source: CpuTemperatureSource::LinuxHwmon,
            sensor_count: self.labels.len() as u32,
        })
    }
}
fn classify(label: &str) -> Option<CpuTemperatureKind> {
    if label.starts_with("coretemp Package id ") || label == "k10temp Tdie" {
        Some(CpuTemperatureKind::Package)
    } else if label.starts_with("coretemp Core ") {
        Some(CpuTemperatureKind::CoreMaximum)
    } else {
        None
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn thermal_zones_and_amd_control_offsets_are_not_cpu_temperatures() {
        for label in [
            "acpitz temp1",
            "k10temp Tctl",
            "nvme Composite",
            "CPU Proximity",
        ] {
            assert!(classify(label).is_none());
        }
        assert_eq!(
            classify("coretemp Package id 0"),
            Some(CpuTemperatureKind::Package)
        );
        assert_eq!(classify("k10temp Tdie"), Some(CpuTemperatureKind::Package));
        assert_eq!(
            classify("coretemp Core 0"),
            Some(CpuTemperatureKind::CoreMaximum)
        );
    }
}
