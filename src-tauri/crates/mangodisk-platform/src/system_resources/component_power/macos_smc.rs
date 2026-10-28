//! Intel SMC domains stay distinct: package, discrete GPU and integrated GPU.
use super::{ComponentPower, ComponentPowerSource, COMPONENT_POWER_MAX_WATTS};
use crate::{
    system_resources::macos_smc::ValueReader, PlatformError, PlatformErrorCode, PlatformResult,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Sensor {
    CpuPackage,
    DiscreteGpu,
    IntegratedGpu,
}
impl Sensor {
    fn key(self) -> [u8; 4] {
        match self {
            Self::CpuPackage => *b"PCPC",
            Self::DiscreteGpu => *b"PG0C",
            Self::IntegratedGpu => *b"PCPG",
        }
    }
}
pub(crate) struct Reader {
    smc: ValueReader,
    sensor: Sensor,
}
impl Reader {
    pub(crate) fn new(sensor: Sensor) -> PlatformResult<Self> {
        let smc = ValueReader::new()?;
        log::info!(
            "component_power_sensor source=apple_smc sensor={sensor:?} key={} outcome=opened",
            String::from_utf8_lossy(&sensor.key())
        );
        Ok(Self { smc, sensor })
    }
    pub(crate) fn read(&self) -> PlatformResult<Option<ComponentPower>> {
        let key = self.sensor.key();
        let value = self.smc.read(key).map_err(|error| {
            PlatformError::new(
                error.code(),
                format!(
                    "source=apple_smc_power sensor={:?} key={} error={error}",
                    self.sensor,
                    String::from_utf8_lossy(&key)
                ),
            )
        })?;
        let watts = validate(value).map_err(|code| {
            PlatformError::new(
                code,
                format!(
                    "source=apple_smc_power sensor={:?} key={} value={value:?}",
                    self.sensor,
                    String::from_utf8_lossy(&key)
                ),
            )
        })?;
        Ok(Some(ComponentPower {
            watts,
            source: ComponentPowerSource::AppleSmc,
        }))
    }
}
fn validate(value: Option<f64>) -> Result<f64, PlatformErrorCode> {
    match value {
        Some(value) if value.is_finite() && (0.0..=COMPONENT_POWER_MAX_WATTS).contains(&value) => {
            Ok(value)
        }
        Some(_) => Err(PlatformErrorCode::InvalidData),
        None => Err(PlatformErrorCode::Unsupported),
    }
}

/// PG0C has no adapter ID. Bind it only to the unique firmware PEG/GFX0 device,
/// never to the first accelerator, a second GPU or a Thunderbolt PCI device.
pub(crate) fn gpu_sensor<'a>(
    adapters: impl IntoIterator<Item = (&'a str, Option<&'a str>)>,
    selected: Option<usize>,
) -> Option<(usize, Sensor)> {
    let adapters: Vec<_> = adapters.into_iter().collect();
    if let Some(index) = selected {
        let (class, path) = *adapters.get(index)?;
        if integrated_adapter(class, path)
            && adapters
                .iter()
                .filter(|(class, _)| class.contains("intel"))
                .count()
                == 1
        {
            return Some((index, Sensor::IntegratedGpu));
        }
    }
    let discrete: Vec<_> = adapters
        .iter()
        .enumerate()
        .filter(|(_, (class, _))| !class.contains("intel"))
        .collect();
    if let [(index, (class, Some(path)))] = discrete.as_slice() {
        if (class.contains("amd") || class.contains("nvidia") || class.contains("nvda"))
            && internal_discrete_path(path)
        {
            return selected
                .is_none_or(|selected| selected == *index)
                .then_some((*index, Sensor::DiscreteGpu));
        }
        return None;
    }
    if !discrete.is_empty()
        || adapters.len() != 1
        || !integrated_adapter(adapters[0].0, adapters[0].1)
    {
        return None;
    }
    Some((0, Sensor::IntegratedGpu))
}

fn integrated_adapter(class: &str, path: Option<&str>) -> bool {
    class == "intelaccelerator"
        && matches!(
            path,
            Some("IOACPIPlane:/_SB/PCI0@0/IGPU@20000" | "IOACPIPlane:/_SB/PCI0@0/GFX0@20000")
        )
}

fn internal_discrete_path(path: &str) -> bool {
    let Some(peg) = path
        .strip_prefix("IOACPIPlane:/_SB/PCI0@0/PEG")
        .and_then(|path| path.strip_suffix("/GFX0@0"))
    else {
        return false;
    };
    let Some((index, address)) = peg.split_once('@') else {
        return false;
    };
    !index.is_empty()
        && index.bytes().all(|byte| byte.is_ascii_digit())
        && !address.is_empty()
        && address.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn smc_domains_never_substitute_or_sum_other_rails() {
        assert_eq!(Sensor::CpuPackage.key(), *b"PCPC");
        assert_eq!(Sensor::DiscreteGpu.key(), *b"PG0C");
        assert_eq!(Sensor::IntegratedGpu.key(), *b"PCPG");
        assert_eq!(validate(Some(0.0)), Ok(0.0));
        assert_eq!(validate(None), Err(PlatformErrorCode::Unsupported));
        for value in [-1.0, f64::NAN, f64::INFINITY, 1001.0] {
            assert_eq!(validate(Some(value)), Err(PlatformErrorCode::InvalidData));
        }
    }
    #[test]
    fn gpu_identity_excludes_external_unknown_and_multiple_devices() {
        let intel = (
            "intelaccelerator",
            Some("IOACPIPlane:/_SB/PCI0@0/IGPU@20000"),
        );
        let amd = (
            "amdradeonx4000_amdbaffingraphicsaccelerator",
            Some("IOACPIPlane:/_SB/PCI0@0/PEG0@10000/GFX0@0"),
        );
        assert_eq!(
            gpu_sensor([intel, amd], None),
            Some((1, Sensor::DiscreteGpu))
        );
        assert_eq!(
            gpu_sensor([amd, intel], None),
            Some((0, Sensor::DiscreteGpu))
        );
        assert_eq!(gpu_sensor([intel], None), Some((0, Sensor::IntegratedGpu)));
        assert_eq!(gpu_sensor([amd, amd], None), None);
        assert_eq!(gpu_sensor([intel, intel], None), None);
        assert_eq!(gpu_sensor([("amd", None), intel], None), None);
        assert_eq!(
            gpu_sensor(
                [("amd", Some("IOACPIPlane:/_SB/PCI0@0/RP01@0/GFX0@0"))],
                None
            ),
            None
        );
        assert_eq!(gpu_sensor([("unknown", amd.1)], None), None);
        assert_eq!(gpu_sensor([("intelaccelerator", None)], None), None);
        assert_eq!(gpu_sensor([("intelarc", intel.1)], None), None);
        assert!(!internal_discrete_path(
            "IOACPIPlane:/_SB/PCI0@0/PEG0@10000/THUNDERBOLT/GFX0@0"
        ));
        assert_eq!(gpu_sensor([], None), None);
    }
    #[test]
    fn hybrid_gpu_selection_uses_the_requested_sensor_without_falling_back() {
        let intel = (
            "intelaccelerator",
            Some("IOACPIPlane:/_SB/PCI0@0/IGPU@20000"),
        );
        let amd = (
            "amdradeonx4000",
            Some("IOACPIPlane:/_SB/PCI0@0/PEG0@10000/GFX0@0"),
        );
        assert_eq!(
            gpu_sensor([intel, amd], Some(0)),
            Some((0, Sensor::IntegratedGpu))
        );
        assert_eq!(
            gpu_sensor([intel, amd], Some(1)),
            Some((1, Sensor::DiscreteGpu))
        );
        assert_eq!(gpu_sensor([intel, amd], Some(2)), None);
        assert_eq!(gpu_sensor([("intelaccelerator", None), amd], Some(0)), None);
        assert_eq!(gpu_sensor([intel, intel, amd], Some(0)), None);
        assert_eq!(
            gpu_sensor([intel, amd, amd], Some(0)),
            Some((0, Sensor::IntegratedGpu))
        );
        assert_eq!(gpu_sensor([intel, amd, amd], Some(1)), None);
    }
    #[test]
    #[ignore = "requires Intel Mac with readable integrated and discrete SMC power"]
    fn native_hybrid_gpu_power_follows_the_requested_adapter() {
        let mut reader = crate::system_resources::gpu::GpuReader::default();
        let adapters = reader.catalogue().unwrap();
        for adapter in adapters.iter().chain(adapters.iter().rev()) {
            let value = reader.read_power(Some(&adapter.id)).unwrap().unwrap();
            assert_eq!(value.adapter_id, adapter.id);
            assert!(matches!(value.power.source, ComponentPowerSource::AppleSmc));
            println!(
                "adapter={} watts={} source={:?}",
                adapter.name, value.power.watts, value.power.source
            );
        }
        assert!(reader.read_power(Some("missing-adapter")).unwrap().is_err());
    }
    #[test]
    #[ignore = "requires Intel Mac SMC package and GPU power sensors"]
    fn native_intel_power_tracks_package_and_identified_gpu() {
        let smc = Reader::new(Sensor::CpuPackage).unwrap();
        assert!(smc.read().unwrap().is_some());
        let mut cpu = super::super::ComponentPowerReader::default();
        let mut gpu = crate::system_resources::gpu::GpuReader::default();
        println!("adapters={:?}", gpu.catalogue().unwrap());
        for sample in 0..8 {
            let cpu = cpu.read().unwrap().unwrap();
            let gpu = gpu.read_power(None).unwrap().unwrap();
            println!("sample={sample} cpu={cpu:?} gpu={gpu:?}");
            std::thread::sleep(std::time::Duration::from_secs(1));
        }
    }
}
