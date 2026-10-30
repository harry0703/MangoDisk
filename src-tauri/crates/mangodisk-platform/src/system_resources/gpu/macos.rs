//! Driver statistics are capability-probed; their keys are not a stable OS contract.
use super::details::{
    GpuActivity, GpuActivityKind, GpuDetails, GpuMemoryArchitecture, GpuMemoryStatus, GpuTelemetry,
};
use super::observation_diagnostics::ObservationDiagnostics;
use super::{failed, unsupported, GpuAdapter, GpuAdapterUsage, GpuSample, PlatformResult};
use crate::diagnostics::text;
use core_foundation::{
    base::{CFType, CFTypeRef, TCFType},
    data::CFData,
    dictionary::{CFDictionary, CFMutableDictionaryRef},
    number::CFNumber,
    string::{CFString, CFStringRef},
};
use std::{
    ffi::c_char,
    ptr,
    time::{Duration, Instant},
};

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    fn IOServiceMatching(name: *const c_char) -> CFMutableDictionaryRef;
    fn IOServiceGetMatchingServices(
        port: u32,
        matching: CFMutableDictionaryRef,
        iterator: *mut u32,
    ) -> i32;
    fn IOIteratorNext(iterator: u32) -> u32;
    fn IOObjectRelease(object: u32) -> i32;
    fn IOObjectConformsTo(object: u32, class_name: *const c_char) -> u32;
    fn IORegistryEntryGetParentEntry(entry: u32, plane: *const c_char, parent: *mut u32) -> i32;
    fn IORegistryEntryGetPath(entry: u32, plane: *const c_char, path: *mut c_char) -> i32;
    fn IORegistryEntryCreateCFProperty(
        entry: u32,
        key: CFStringRef,
        allocator: *const std::ffi::c_void,
        options: u32,
    ) -> CFTypeRef;
}

struct Object(u32);
impl Drop for Object {
    fn drop(&mut self) {
        unsafe {
            IOObjectRelease(self.0);
        }
    }
}
struct Adapter {
    object: Object,
    id: String,
    name: String,
    unified: bool,
    core_count: Option<u32>,
    class: String,
    #[cfg(target_arch = "x86_64")]
    acpi_path: Option<String>,
    temperature: Option<super::macos_temperature::Reader>,
    temperature_error: Option<(crate::PlatformError, Instant)>,
    temperature_diagnostic: Option<String>,
    temperature_failures: u8,
    temperature_retry_ms: u64,
    temperature_identified: bool,
}

impl Adapter {
    fn observe_temperature_error(&mut self, error: &crate::PlatformError, now: Instant) {
        if error.code() == crate::PlatformErrorCode::InvalidData {
            self.temperature_error = None;
            self.temperature_failures = 0;
            self.temperature_retry_ms = super::temperature::GPU_TEMPERATURE_INTERVAL_MS;
            return;
        }
        self.temperature = None;
        if self
            .temperature_error
            .as_ref()
            .is_some_and(|(previous, at)| now < *at && previous.code() == error.code())
        {
            return;
        }
        if self
            .temperature_error
            .as_ref()
            .is_none_or(|(previous, _)| previous.code() != error.code())
        {
            self.temperature_failures = 0;
        }
        self.temperature_failures = self.temperature_failures.saturating_add(1);
        self.temperature_retry_ms = crate::system_resources::temperature_retry::delay_ms(
            error.code(),
            self.temperature_failures,
            super::temperature::GPU_TEMPERATURE_INTERVAL_MS,
        );
        self.temperature_error = Some((
            error.clone(),
            now + Duration::from_millis(self.temperature_retry_ms),
        ));
    }

    fn read_temperature(
        &mut self,
        statistics_key: &CFString,
        unambiguous: bool,
    ) -> super::temperature::GpuTemperatureSample {
        use super::temperature::{
            GpuTemperature, GpuTemperatureKind, GpuTemperatureSample, GpuTemperatureSource,
        };
        let driver = property(self.object.0, statistics_key)
            .map(|value| details(&value, self.unified, self.core_count));
        let reading = (|| {
            if let Some(celsius) = driver
                .and_then(|value| value.telemetry.temperature_celsius)
                .filter(|value| *value != 128.0)
            {
                Ok(GpuTemperature {
                    adapter_id: self.id.clone(),
                    adapter_name: self.name.clone(),
                    celsius,
                    kind: GpuTemperatureKind::Device,
                    source: GpuTemperatureSource::Driver,
                    sensor_count: 1,
                })
            } else {
                if let Some((error, retry_at)) = &self.temperature_error {
                    if Instant::now() < *retry_at {
                        return Err(error.clone());
                    }
                }
                if self.temperature.is_none() {
                    self.temperature = Some(super::macos_temperature::Reader::new(
                        &self.name,
                        &self.class,
                        unambiguous,
                    )?);
                }
                self.temperature.as_mut().unwrap().read().map(
                    |(celsius, sensor_count, kind, source)| GpuTemperature {
                        adapter_id: self.id.clone(),
                        adapter_name: self.name.clone(),
                        celsius,
                        sensor_count,
                        kind,
                        source,
                    },
                )
            }
        })();
        let reading = reading.map_err(|error| {
            if self.temperature_identified && error.code() == crate::PlatformErrorCode::Unsupported
            {
                crate::PlatformError::new(
                    crate::PlatformErrorCode::OperationFailed,
                    format!("identified GPU temperature temporarily unavailable: {error}"),
                )
            } else {
                error
            }
        });
        let diagnostic = match &reading {
            Ok(value) => {
                self.temperature_error = None;
                self.temperature_failures = 0;
                self.temperature_identified = true;
                format!(
                    "ready:{:?}:{:?}:{}",
                    value.source, value.kind, value.sensor_count
                )
            }
            Err(error) => {
                self.observe_temperature_error(error, Instant::now());
                format!("{:?}:{}:{}", error.code(), error, self.temperature_retry_ms)
            }
        };
        if self.temperature_diagnostic.as_ref() != Some(&diagnostic) {
            match &reading {
                Ok(value) => {
                    log::info!(
                        "gpu_temperature_ready adapter={} name={} source={:?} kind={:?} sensor_count={}",
                        text(&self.id), text(&self.name), value.source, value.kind, value.sensor_count
                    );
                }
                Err(error) if error.code() == crate::PlatformErrorCode::Unsupported => {
                    log::info!(
                        "gpu_temperature_unsupported adapter={} name={} code={:?} reason={} retry_after_ms=30000",
                        text(&self.id),
                        text(&self.name),
                        error.code(),
                        text(error)
                    );
                }
                Err(error) => {
                    let retry_after_ms = self.temperature_retry_ms;
                    log::warn!(
                        "gpu_temperature_read_failed adapter={} name={} code={:?} error={} retry_after_ms={retry_after_ms}",
                        text(&self.id), text(&self.name), error.code(), text(error)
                    );
                }
            }
            self.temperature_diagnostic = Some(diagnostic);
        }
        GpuTemperatureSample {
            adapter_id: self.id.clone(),
            reading,
        }
    }
}

pub struct GpuReader {
    adapters: Vec<Adapter>,
    discovered_at: Option<Instant>,
    statistics_key: CFString,
    temperature_due: Option<Instant>,
    power: crate::system_resources::component_power::ComponentPowerReader,
    power_adapter: Option<String>,
    diagnostics: ObservationDiagnostics,
    discovery_failure: Option<(&'static str, String)>,
}
impl Default for GpuReader {
    fn default() -> Self {
        Self {
            adapters: Vec::new(),
            discovered_at: None,
            statistics_key: CFString::new("PerformanceStatistics"),
            temperature_due: None,
            power: crate::system_resources::component_power::ComponentPowerReader::new(
                crate::system_resources::component_power::Component::Gpu,
            ),
            power_adapter: None,
            diagnostics: ObservationDiagnostics::default(),
            discovery_failure: None,
        }
    }
}

fn property(object: u32, key: &CFString) -> Option<CFType> {
    let raw = unsafe {
        IORegistryEntryCreateCFProperty(object, key.as_concrete_TypeRef(), ptr::null(), 0)
    };
    (!raw.is_null()).then(|| unsafe { CFType::wrap_under_create_rule(raw) })
}

fn model_name(value: &CFType) -> Option<String> {
    value
        .downcast::<CFString>()
        .map(|value| value.to_string())
        .or_else(|| {
            let data = value.downcast::<CFData>()?;
            std::str::from_utf8(data.bytes()).ok().map(str::to_owned)
        })
        .map(|value| value.trim_end_matches('\0').trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn adapter_model(object: u32, key: &CFString) -> Option<String> {
    if let Some(name) = property(object, key).as_ref().and_then(model_name) {
        return Some(name);
    }
    pci_property(object, key).as_ref().and_then(model_name)
}

fn pci_property(object: u32, key: &CFString) -> Option<CFType> {
    // Intel and AMD expose the model on the PCI device, sometimes behind a
    // controller node. Restrict the fallback to the nearest PCI device instead
    // of a recursive property search that could return the computer model.
    let mut parent: Option<Object> = None;
    for _ in 0..4 {
        let current = parent.as_ref().map_or(object, |parent| parent.0);
        let mut entry = 0;
        if unsafe { IORegistryEntryGetParentEntry(current, c"IOService".as_ptr(), &mut entry) } != 0
        {
            return None;
        }
        parent = Some(Object(entry));
        if unsafe { IOObjectConformsTo(entry, c"IOPCIDevice".as_ptr()) } != 0 {
            return property(entry, key);
        }
    }
    None
}

fn activity(property: &CFType) -> Option<(f64, &'static str)> {
    let untyped = property.downcast::<CFDictionary>()?;
    let dictionary: CFDictionary<CFString, CFType> =
        unsafe { CFDictionary::wrap_under_get_rule(untyped.as_concrete_TypeRef()) };
    ["Device Utilization %", "GPU Activity(%)"]
        .into_iter()
        .find_map(|key| percentage(&dictionary, key).map(|value| (value, key)))
}

fn percentage(dictionary: &CFDictionary<CFString, CFType>, key: &str) -> Option<f64> {
    let value = dictionary
        .find(CFString::new(key))?
        .downcast::<CFNumber>()?
        .to_f64()?;
    (value.is_finite() && (0.0..=100.0).contains(&value)).then_some(value)
}

fn statistic(dictionary: &CFDictionary<CFString, CFType>, key: &str, maximum: f64) -> Option<f64> {
    let value = dictionary
        .find(CFString::new(key))?
        .downcast::<CFNumber>()?
        .to_f64()?;
    (value.is_finite() && value > 0.0 && value <= maximum).then_some(value)
}

fn details(property: &CFType, unified: bool, core_count: Option<u32>) -> GpuDetails {
    let mut activities = Vec::new();
    let mut telemetry = GpuTelemetry {
        core_count,
        ..Default::default()
    };
    if let Some(untyped) = property.downcast::<CFDictionary>() {
        let dictionary: CFDictionary<CFString, CFType> =
            unsafe { CFDictionary::wrap_under_get_rule(untyped.as_concrete_TypeRef()) };
        // Optional driver facts share the existing property read. Zero clocks or
        // temperature are unavailable sentinels, while an idle fan may be zero.
        telemetry.temperature_celsius = statistic(&dictionary, "Temperature(C)", 150.0);
        telemetry.core_clock_mhz = statistic(&dictionary, "Core Clock(MHz)", 100_000.0);
        telemetry.memory_clock_mhz = statistic(&dictionary, "Memory Clock(MHz)", 100_000.0);
        telemetry.fan_percent = percentage(&dictionary, "Fan Speed(%)");
        for (key, kind, id) in [
            (
                "Renderer Utilization %",
                GpuActivityKind::Renderer,
                "renderer",
            ),
            ("Tiler Utilization %", GpuActivityKind::Tiler, "tiler"),
        ] {
            if let Some(used_percent) = percentage(&dictionary, key) {
                activities.push(GpuActivity {
                    id: id.into(),
                    kind,
                    name: None,
                    used_percent,
                    included_in_summary: false,
                });
            }
        }
    }
    GpuDetails {
        activities,
        telemetry,
        memory_architecture: if unified {
            GpuMemoryArchitecture::Unified
        } else {
            GpuMemoryArchitecture::Unknown
        },
        memory_status: GpuMemoryStatus::Unsupported,
        memory: None,
    }
}

impl GpuReader {
    fn discovery_error(
        &mut self,
        stage: &'static str,
        code: impl std::fmt::Display,
    ) -> crate::PlatformError {
        let code = code.to_string();
        if self
            .discovery_failure
            .as_ref()
            .is_none_or(|previous| previous.0 != stage || previous.1 != code)
        {
            log::warn!("gpu_inventory_unavailable source=ioreg_ioaccelerator stage={stage} native_code={} outcome=retain_previous_adapters", text(&code));
        }
        self.discovery_failure = Some((stage, code.clone()));
        failed(stage, code)
    }
    fn discover(&mut self) -> PlatformResult<()> {
        let matching = unsafe { IOServiceMatching(c"IOAccelerator".as_ptr()) };
        if matching.is_null() {
            return Err(self.discovery_error("matching", "null_dictionary"));
        }
        let mut iterator = 0;
        let result = unsafe { IOServiceGetMatchingServices(0, matching, &mut iterator) };
        if result != 0 {
            return Err(self.discovery_error("enumerate", result));
        }
        let iterator = Object(iterator);
        let model_key = CFString::new("model");
        let mut adapters = Vec::new();
        loop {
            let object = unsafe { IOIteratorNext(iterator.0) };
            if object == 0 {
                break;
            }
            let object = Object(object);
            let mut path = [0i8; 512];
            let result = unsafe {
                IORegistryEntryGetPath(object.0, c"IOService".as_ptr(), path.as_mut_ptr())
            };
            if result != 0 {
                return Err(self.discovery_error("identity", result));
            }
            let id = unsafe { std::ffi::CStr::from_ptr(path.as_ptr()) }
                .to_string_lossy()
                .into_owned();
            let name = adapter_model(object.0, &model_key)
                .unwrap_or_else(|| format!("GPU {}", adapters.len()));
            let class = property(object.0, &CFString::new("IOClass"))
                .and_then(|value| value.downcast::<CFString>())
                .map(|value| value.to_string().to_ascii_lowercase())
                .unwrap_or_default();
            let unified = class.contains("agx");
            let core_count = property(object.0, &CFString::new("gpu-core-count"))
                .and_then(|value| value.downcast::<CFNumber>())
                .and_then(|value| value.to_i64())
                .and_then(|value| u32::try_from(value).ok())
                .filter(|value| *value > 0 && *value <= 16_384);
            #[cfg(target_arch = "x86_64")]
            let acpi_path = pci_property(object.0, &CFString::new("acpi-path"))
                .as_ref()
                .and_then(model_name);
            adapters.push(Adapter {
                object,
                id,
                name,
                unified,
                core_count,
                class,
                #[cfg(target_arch = "x86_64")]
                acpi_path,
                temperature: None,
                temperature_error: None,
                temperature_failures: 0,
                temperature_retry_ms: 0,
                temperature_identified: false,
                temperature_diagnostic: None,
            });
        }
        if self.discovered_at.is_none() || adapters.len() != self.adapters.len() {
            log::info!(
                "gpu_inventory source=ioreg_ioaccelerator hardware_adapters={}",
                adapters.len()
            );
        }
        for adapter in &adapters {
            if !self.adapters.iter().any(|previous| {
                previous.id == adapter.id
                    && previous.name == adapter.name
                    && previous.unified == adapter.unified
                    && previous.core_count == adapter.core_count
            }) {
                log::info!("gpu_adapter_discovered source=ioreg_ioaccelerator adapter={} name={} unified={} core_count={:?} summary_policy=driver_device_utilization", text(&adapter.id), text(&adapter.name), adapter.unified, adapter.core_count);
            }
        }
        for previous in &self.adapters {
            if !adapters.iter().any(|adapter| adapter.id == previous.id) {
                log::info!(
                    "gpu_adapter_removed source=ioreg_ioaccelerator adapter={} name={}",
                    text(&previous.id),
                    text(&previous.name)
                );
            }
        }
        self.diagnostics
            .retain(|id| adapters.iter().any(|adapter| adapter.id == id));
        if self.discovery_failure.take().is_some() {
            log::info!("gpu_inventory_recovered source=ioreg_ioaccelerator");
        }
        // Retain read-only SMC connections across inventory refreshes only when
        // the accelerator inventory is unchanged, preserving sensor identity.
        if adapters.len() == self.adapters.len()
            && adapters.iter().all(|adapter| {
                self.adapters.iter().any(|old| {
                    old.id == adapter.id && old.name == adapter.name && old.class == adapter.class
                })
            })
        {
            for adapter in &mut adapters {
                if let Some(previous) = self.adapters.iter_mut().find(|old| old.id == adapter.id) {
                    adapter.temperature = previous.temperature.take();
                    adapter.temperature_error = previous.temperature_error.take();
                    adapter.temperature_diagnostic = previous.temperature_diagnostic.take();
                }
            }
        }
        self.adapters = adapters;
        self.discovered_at = Some(Instant::now());
        Ok(())
    }

    pub fn reset(&mut self) {
        // These driver values are instantaneous, with no interval baseline to reset.
        // Demand generations also change on tab switches; preserve metadata and diagnostics
        // until their regular refresh or the owning worker releases the reader.
    }

    fn refresh(&mut self) -> PlatformResult<()> {
        if self
            .discovered_at
            .is_none_or(|at| at.elapsed() >= Duration::from_secs(30))
        {
            self.discover()?;
        }
        Ok(())
    }
    pub fn catalogue(&mut self) -> PlatformResult<Vec<GpuAdapter>> {
        self.refresh()?;
        Ok(self.adapters())
    }
    pub fn adapters(&self) -> Vec<GpuAdapter> {
        self.adapters
            .iter()
            .enumerate()
            .map(|(index, adapter)| GpuAdapter {
                id: adapter.id.clone(),
                name: format!("GPU{index} · {}", adapter.name),
            })
            .collect()
    }
    pub fn read(&mut self) -> PlatformResult<GpuSample> {
        self.sample()
    }
    pub fn read_detailed(&mut self) -> PlatformResult<GpuSample> {
        self.read()
    }
    pub fn read_power(
        &mut self,
        selected_adapter: Option<&str>,
    ) -> Option<PlatformResult<crate::system_resources::component_power::GpuPower>> {
        use crate::system_resources::component_power::{Component, ComponentPowerReader, GpuPower};
        if let Err(error) = self.refresh() {
            return Some(Err(error));
        }
        #[cfg(target_arch = "aarch64")]
        let (id, smc_integrated) = {
            let mut adapters = self
                .adapters
                .iter()
                .filter(|adapter| adapter.class.contains("agx"));
            let Some(adapter) = adapters.next() else {
                return Some(Err(super::unsupported()));
            };
            if adapters.next().is_some() {
                return Some(Err(super::unsupported()));
            }
            if selected_adapter.is_some_and(|id| id != adapter.id) {
                return Some(Err(super::unsupported()));
            }
            (adapter.id.clone(), false)
        };
        #[cfg(target_arch = "x86_64")]
        let (id, smc_integrated) = {
            use crate::system_resources::component_power::macos_smc::{gpu_sensor, Sensor};
            let selected = match selected_adapter {
                Some(id) => {
                    let Some(index) = self.adapters.iter().position(|adapter| adapter.id == id)
                    else {
                        return Some(Err(super::unsupported()));
                    };
                    Some(index)
                }
                None => None,
            };
            let Some((index, sensor)) = gpu_sensor(
                self.adapters
                    .iter()
                    .map(|adapter| (adapter.class.as_str(), adapter.acpi_path.as_deref())),
                selected,
            ) else {
                return Some(Err(super::unsupported()));
            };
            (
                self.adapters[index].id.clone(),
                sensor == Sensor::IntegratedGpu,
            )
        };
        if self.power_adapter.as_ref() != Some(&id) {
            self.power = ComponentPowerReader::new(Component::Gpu);
            #[cfg(target_arch = "x86_64")]
            if smc_integrated {
                self.power = ComponentPowerReader::integrated_gpu();
            }
            log::info!(
                "gpu_power_identity adapter={} smc_integrated={smc_integrated} outcome=bound",
                text(&id)
            );
            self.power_adapter = Some(id.clone());
        }
        self.power.read().map(|result| {
            result.map(|power| GpuPower {
                adapter_id: id,
                power,
            })
        })
    }
    pub fn read_temperatures(
        &mut self,
    ) -> Option<PlatformResult<Vec<super::temperature::GpuTemperatureSample>>> {
        use super::temperature::GPU_TEMPERATURE_INTERVAL_MS;
        if self.temperature_due.is_some_and(|due| Instant::now() < due) {
            return None;
        }
        self.temperature_due =
            Some(Instant::now() + Duration::from_millis(GPU_TEMPERATURE_INTERVAL_MS));
        if let Err(error) = self.refresh() {
            return Some(Err(error));
        }
        let counts = |vendor: &str| {
            self.adapters
                .iter()
                .filter(|adapter| adapter.class.contains(vendor))
                .count()
        };
        let apple_count = counts("agx");
        let intel_count = counts("intel");
        Some(Ok(self
            .adapters
            .iter_mut()
            .map(|adapter| {
                let unambiguous = (adapter.class.contains("agx") && apple_count == 1)
                    || (adapter.class.contains("intel") && intel_count == 1);
                adapter.read_temperature(&self.statistics_key, unambiguous)
            })
            .collect()))
    }
    fn sample(&mut self) -> PlatformResult<GpuSample> {
        self.refresh()?;
        let diagnostics = &mut self.diagnostics;
        let values: Vec<_> = self
            .adapters
            .iter()
            .filter_map(|adapter| {
                let Some(statistics) = property(adapter.object.0, &self.statistics_key) else {
                    diagnostics.observe(
                        &adapter.id,
                        "ioreg_performance_statistics",
                        Err("statistics_missing"),
                    );
                    return None;
                };
                let Some((used_percent, source)) = activity(&statistics) else {
                    diagnostics.observe(
                        &adapter.id,
                        "ioreg_performance_statistics",
                        Err("utilization_missing_or_invalid"),
                    );
                    return None;
                };
                let details = details(&statistics, adapter.unified, adapter.core_count);
                diagnostics.observe(&adapter.id, source, Ok(Some(&details)));
                Some(GpuAdapterUsage {
                    id: adapter.id.clone(),
                    name: adapter.name.clone(),
                    used_percent,
                    // Engine history must continue while the detail panel is hidden.
                    // These facts reuse the same native property read as total usage.
                    details: Some(details),
                })
            })
            .collect();
        if values.is_empty() {
            Err(unsupported())
        } else {
            Ok(GpuSample::Usage(values))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_gpu_catalogue_contains_unique_adapter_identities() {
        let adapters = GpuReader::default().catalogue().unwrap();
        let identities: std::collections::HashSet<_> =
            adapters.iter().map(|adapter| &adapter.id).collect();
        assert_eq!(
            identities.len(),
            adapters.len(),
            "duplicate IOAccelerator matches must not become multiple physical GPUs: {adapters:?}"
        );
    }

    #[test]
    #[ignore = "requires an Apple Silicon Mac with readable GPU SMC sensors"]
    fn native_gpu_temperature_recovers_on_the_next_interval_after_invalid_readings() {
        let mut reader = GpuReader::default();
        reader.catalogue().unwrap();
        let adapter = reader
            .adapters
            .iter_mut()
            .find(|adapter| adapter.class.contains("agx"))
            .unwrap();
        let initial = adapter
            .read_temperature(&reader.statistics_key, true)
            .reading
            .unwrap();
        assert_eq!(
            initial.source,
            super::super::temperature::GpuTemperatureSource::AppleSmc
        );
        assert!(adapter.temperature.is_some());

        // Inject the same typed failure produced by an A18 Pro gated sample;
        // the adapter and subsequent SMC observation remain the host's real GPU.
        let now = Instant::now();
        let invalid = crate::PlatformError::new(
            crate::PlatformErrorCode::InvalidData,
            "test near-zero temperature interval",
        );
        adapter.observe_temperature_error(&invalid, now);
        let next =
            now + Duration::from_millis(super::super::temperature::GPU_TEMPERATURE_INTERVAL_MS);
        assert!(
            adapter
                .temperature_error
                .as_ref()
                .is_none_or(|(_, at)| *at <= next),
            "a transient invalid reading must not block the next temperature interval"
        );
        assert!(
            adapter.temperature.is_some(),
            "invalid samples must retain the healthy SMC connection"
        );
        let recovered = adapter
            .read_temperature(&reader.statistics_key, true)
            .reading
            .unwrap();
        assert!(recovered.valid());
        assert!(adapter.temperature_error.is_none());

        let failure = crate::PlatformError::new(
            crate::PlatformErrorCode::OperationFailed,
            "test SMC transport failure",
        );
        adapter.observe_temperature_error(&failure, now);
        assert!(adapter.temperature.is_none());
        assert_eq!(
            adapter.temperature_error.as_ref().unwrap().1,
            now + Duration::from_secs(4)
        );
        adapter.observe_temperature_error(&failure, now + Duration::from_secs(2));
        assert_eq!(
            adapter.temperature_error.as_ref().unwrap().1,
            now + Duration::from_secs(4),
            "a cached failure must not extend its retry deadline"
        );
        let blocked = adapter
            .read_temperature(&reader.statistics_key, true)
            .reading
            .unwrap_err();
        assert_eq!(blocked.code(), crate::PlatformErrorCode::OperationFailed);
        adapter.temperature_error.as_mut().unwrap().1 = now - Duration::from_secs(1);
        let reopened = adapter
            .read_temperature(&reader.statistics_key, true)
            .reading
            .unwrap();
        assert!(reopened.valid());
        assert!(adapter.temperature.is_some());
        assert!(adapter.temperature_error.is_none());
        println!("native_gpu_temperature_recovery model={} initial_celsius={} recovered_celsius={} reopened_celsius={} invalid_retry_ms={} transport_retry_ms=4000", adapter.name, initial.celsius, recovered.celsius, reopened.celsius, super::super::temperature::GPU_TEMPERATURE_INTERVAL_MS);
    }
    #[test]
    fn driver_models_accept_strings_and_pci_bytes_without_empty_or_invalid_labels() {
        assert_eq!(
            model_name(&CFData::from_buffer(b"Intel UHD Graphics 630\0").as_CFType()),
            Some("Intel UHD Graphics 630".into())
        );
        assert_eq!(
            model_name(&CFData::from_buffer(b"Radeon Pro 560X").as_CFType()),
            Some("Radeon Pro 560X".into())
        );
        assert_eq!(
            model_name(&CFString::new("Apple M3 Max").as_CFType()),
            Some("Apple M3 Max".into())
        );
        assert!(model_name(&CFData::from_buffer(b"\xff").as_CFType()).is_none());
        assert!(model_name(&CFString::new(" \0").as_CFType()).is_none());
        assert!(model_name(&CFNumber::from(1).as_CFType()).is_none());
    }
    #[test]
    fn demand_resets_preserve_cached_native_adapter_handles() {
        let mut reader = GpuReader::default();
        reader.catalogue().unwrap();
        let discovered_at = reader.discovered_at;
        let objects: Vec<_> = reader
            .adapters
            .iter()
            .map(|adapter| adapter.object.0)
            .collect();
        reader.reset();
        assert_eq!(reader.discovered_at, discovered_at);
        assert_eq!(
            reader
                .adapters
                .iter()
                .map(|adapter| adapter.object.0)
                .collect::<Vec<_>>(),
            objects
        );
        reader.catalogue().unwrap();
        assert_eq!(reader.discovered_at, discovered_at);
    }
    #[test]
    fn optional_engines_never_infer_a_separate_vram_capacity() {
        let statistics = CFDictionary::from_CFType_pairs(&[
            (
                CFString::new("Renderer Utilization %"),
                CFNumber::from(12.0).as_CFType(),
            ),
            (
                CFString::new("Tiler Utilization %"),
                CFNumber::from(f64::NAN).as_CFType(),
            ),
            (
                CFString::new("In use system memory"),
                CFNumber::from(1024).as_CFType(),
            ),
        ])
        .as_CFType();
        let value = details(&statistics, true, Some(40));
        assert_eq!(value.activities.len(), 1);
        assert_eq!(value.activities[0].kind, GpuActivityKind::Renderer);
        assert_eq!(value.memory_architecture, GpuMemoryArchitecture::Unified);
        assert!(value.memory.is_none());
        assert_eq!(value.memory_status, GpuMemoryStatus::Unsupported);
    }

    #[test]
    fn optional_driver_telemetry_rejects_sentinels_and_invalid_values() {
        let statistics = CFDictionary::from_CFType_pairs(&[
            (
                CFString::new("Temperature(C)"),
                CFNumber::from(0.0).as_CFType(),
            ),
            (
                CFString::new("Core Clock(MHz)"),
                CFNumber::from(f64::INFINITY).as_CFType(),
            ),
            (
                CFString::new("Memory Clock(MHz)"),
                CFNumber::from(1500.0).as_CFType(),
            ),
            (
                CFString::new("Fan Speed(%)"),
                CFNumber::from(0.0).as_CFType(),
            ),
        ])
        .as_CFType();
        let value = details(&statistics, false, Some(40));
        assert_eq!(value.telemetry.core_count, Some(40));
        assert!(value.telemetry.temperature_celsius.is_none());
        assert!(value.telemetry.core_clock_mhz.is_none());
        assert_eq!(value.telemetry.memory_clock_mhz, Some(1500.0));
        assert_eq!(value.telemetry.fan_percent, Some(0.0));
    }

    #[test]
    fn malformed_statistics_never_become_idle() {
        let value = |number: f64| {
            CFDictionary::from_CFType_pairs(&[(
                CFString::new("Device Utilization %"),
                CFNumber::from(number).as_CFType(),
            )])
            .as_CFType()
        };
        assert_eq!(activity(&value(0.0)), Some((0.0, "Device Utilization %")));
        assert_eq!(
            activity(&value(100.0)),
            Some((100.0, "Device Utilization %"))
        );
        assert_eq!(activity(&value(-1.0)), None);
        assert_eq!(activity(&value(f64::NAN)), None);
        assert_eq!(activity(&value(101.0)), None);
        assert_eq!(activity(&CFString::new("invalid").as_CFType()), None);
    }
}
