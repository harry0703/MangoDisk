//! Apple Silicon active-state averages. Unknown tables or channel layouts fail closed.
use super::{valid_mhz, CpuFrequency, CpuFrequencySource};
use crate::PlatformResult;
use core_foundation::{
    array::CFArray,
    base::{CFType, CFTypeRef, TCFType},
    data::CFData,
    dictionary::{CFDictionary, CFMutableDictionaryRef},
    string::{CFString, CFStringRef},
};
use std::{
    ffi::c_char,
    ptr,
    time::{Duration, Instant},
};

#[link(name = "IOReport")]
unsafe extern "C" {
    fn IOReportCopyChannelsInGroup(
        group: CFStringRef,
        subgroup: CFStringRef,
        a: u64,
        b: u64,
        c: u64,
    ) -> core_foundation::dictionary::CFDictionaryRef;
    fn IOReportCreateSubscription(
        a: *const std::ffi::c_void,
        channels: CFMutableDictionaryRef,
        out: *mut CFMutableDictionaryRef,
        b: u64,
        c: CFTypeRef,
    ) -> CFTypeRef;
    fn IOReportCreateSamples(
        subscription: CFTypeRef,
        channels: CFMutableDictionaryRef,
        c: CFTypeRef,
    ) -> core_foundation::dictionary::CFDictionaryRef;
    fn IOReportCreateSamplesDelta(
        a: core_foundation::dictionary::CFDictionaryRef,
        b: core_foundation::dictionary::CFDictionaryRef,
        c: CFTypeRef,
    ) -> core_foundation::dictionary::CFDictionaryRef;
    fn IOReportChannelGetChannelName(
        channel: core_foundation::dictionary::CFDictionaryRef,
    ) -> CFStringRef;
    fn IOReportStateGetCount(channel: core_foundation::dictionary::CFDictionaryRef) -> i32;
    fn IOReportStateGetNameForIndex(
        channel: core_foundation::dictionary::CFDictionaryRef,
        index: i32,
    ) -> CFStringRef;
    fn IOReportStateGetResidency(
        channel: core_foundation::dictionary::CFDictionaryRef,
        index: i32,
    ) -> i64;
}
#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    fn IOServiceNameMatching(name: *const c_char) -> CFMutableDictionaryRef;
    fn IOServiceGetMatchingService(port: u32, matching: CFMutableDictionaryRef) -> u32;
    fn IOObjectRelease(object: u32) -> i32;
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
fn count(name: &std::ffi::CStr) -> Option<u32> {
    let mut value = 0u32;
    let mut len = std::mem::size_of::<u32>();
    (unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            (&mut value as *mut u32).cast(),
            &mut len,
            ptr::null_mut(),
            0,
        )
    } == 0
        && len == 4
        && value > 0)
        .then_some(value)
}
fn table(object: u32, key: &str, divisor: f64) -> Option<Vec<f64>> {
    let key = CFString::new(key);
    let raw = unsafe {
        IORegistryEntryCreateCFProperty(object, key.as_concrete_TypeRef(), ptr::null(), 0)
    };
    if raw.is_null() {
        return None;
    }
    let value = unsafe { CFType::wrap_under_create_rule(raw) };
    let data = value.downcast::<CFData>()?;
    decode_table(data.bytes(), divisor)
}
fn decode_table(bytes: &[u8], divisor: f64) -> Option<Vec<f64>> {
    if bytes.is_empty() || bytes.len() > 512 || !bytes.len().is_multiple_of(8) {
        return None;
    }
    let values: Option<Vec<f64>> = bytes
        .as_chunks::<8>()
        .0
        .iter()
        .map(|chunk| {
            let mhz = f64::from(u32::from_le_bytes(chunk[..4].try_into().ok()?)) / divisor;
            valid_mhz(mhz).filter(|value| *value >= 100.0)
        })
        .collect();
    let values = values?;
    values
        .windows(2)
        .all(|pair| pair[0] < pair[1])
        .then_some(values)
}
fn cluster(name: &str) -> Option<bool> {
    for (prefix, efficiency) in [("ECPU", true), ("PCPU", false)] {
        if let Some(suffix) = name.strip_prefix(prefix) {
            if suffix.bytes().all(|byte| byte.is_ascii_digit()) {
                return Some(efficiency);
            }
        }
    }
    None
}
fn active_average(states: &[(String, i64)], frequencies: &[f64]) -> Option<f64> {
    let mut active = Vec::new();
    for (name, residency) in states {
        if *residency < 0 {
            return None;
        }
        if matches!(name.as_str(), "IDLE" | "DOWN" | "OFF") {
            continue;
        }
        // Driver state ordinals must agree with the frequency table, not merely its length.
        let (voltage, performance) = name.strip_prefix('V')?.split_once('P')?;
        let index = voltage.parse::<usize>().ok()?;
        let level = performance.parse::<usize>().ok()?;
        if index != active.len() || level != frequencies.len().checked_sub(index + 1)? {
            return None;
        }
        active.push(*residency as f64);
    }
    if active.len() != frequencies.len() {
        return None;
    }
    let total: f64 = active.iter().sum();
    if total <= 0.0 {
        return None;
    }
    valid_mhz(
        active
            .iter()
            .zip(frequencies)
            .map(|(time, freq)| time / total * freq)
            .sum(),
    )
}
struct Subscription {
    channels: CFType,
    subscription: CFType,
    previous: Option<(CFType, Instant)>,
    efficiency: Vec<f64>,
    performance: Vec<f64>,
}
fn dictionary(value: &CFType) -> Option<CFDictionary<CFString, CFType>> {
    let value = value.downcast::<CFDictionary>()?;
    Some(unsafe { CFDictionary::wrap_under_get_rule(value.as_concrete_TypeRef()) })
}
impl Subscription {
    fn open() -> PlatformResult<Self> {
        let model = super::identity().model.ok_or_else(super::unsupported)?;
        let generation = model
            .strip_prefix("Apple M")
            .and_then(|text| text.split_whitespace().next())
            .and_then(|text| text.parse::<u32>().ok())
            .ok_or_else(super::unsupported)?;
        if count(c"hw.perflevel2.physicalcpu").is_some() {
            return Err(super::unsupported());
        }
        let divisor = match generation {
            1..=3 => 1_000_000.0,
            4 => 1000.0,
            _ => return Err(super::unsupported()),
        };
        let object = Object(unsafe {
            IOServiceGetMatchingService(0, IOServiceNameMatching(c"pmgr".as_ptr()))
        });
        if object.0 == 0 {
            return Err(super::unsupported());
        }
        let efficiency =
            table(object.0, "voltage-states1-sram", divisor).ok_or_else(super::unsupported)?;
        let performance =
            table(object.0, "voltage-states5-sram", divisor).ok_or_else(super::unsupported)?;
        let efficiency_count =
            count(c"hw.perflevel1.physicalcpu").ok_or_else(super::unsupported)?;
        let performance_count =
            count(c"hw.perflevel0.physicalcpu").ok_or_else(super::unsupported)?;
        let group = CFString::new("CPU Stats");
        let subgroup = CFString::new("CPU Complex Performance States");
        let raw = unsafe {
            IOReportCopyChannelsInGroup(
                group.as_concrete_TypeRef(),
                subgroup.as_concrete_TypeRef(),
                0,
                0,
                0,
            )
        };
        if raw.is_null() {
            return Err(super::unsupported());
        }
        let source = unsafe { CFType::wrap_under_create_rule(raw.cast()) };
        let channels = unsafe {
            core_foundation::dictionary::CFDictionaryCreateMutableCopy(
                ptr::null(),
                0,
                source.as_CFTypeRef().cast(),
            )
        };
        if channels.is_null() {
            return Err(super::unsupported());
        }
        let channels = unsafe { CFType::wrap_under_create_rule(channels.cast()) };
        let mut output = ptr::null_mut();
        let raw = unsafe {
            IOReportCreateSubscription(
                ptr::null(),
                channels.as_CFTypeRef().cast_mut().cast(),
                &mut output,
                0,
                ptr::null(),
            )
        };
        if !output.is_null() {
            drop(unsafe { CFType::wrap_under_create_rule(output.cast()) });
        }
        if raw.is_null() {
            return Err(super::unsupported());
        }
        let subscription = unsafe { CFType::wrap_under_create_rule(raw) };
        log::info!("cpu_frequency_capability source=ioreport_performance_states generation={generation} efficiency_states={} performance_states={} efficiency_cores={efficiency_count} performance_cores={performance_count}",efficiency.len(),performance.len());
        Ok(Self {
            channels,
            subscription,
            previous: None,
            efficiency,
            performance,
        })
    }
    fn read(&mut self) -> PlatformResult<Option<CpuFrequency>> {
        let raw = unsafe {
            IOReportCreateSamples(
                self.subscription.as_CFTypeRef(),
                self.channels.as_CFTypeRef().cast_mut().cast(),
                ptr::null(),
            )
        };
        if raw.is_null() {
            return Err(super::unsupported());
        }
        let current = unsafe { CFType::wrap_under_create_rule(raw.cast()) };
        let now = Instant::now();
        let Some((previous, at)) = self.previous.replace((current.clone(), now)) else {
            return Ok(None);
        };
        if !(100..=5000).contains(&now.duration_since(at).as_millis()) {
            return Ok(None);
        }
        let raw = unsafe {
            IOReportCreateSamplesDelta(
                previous.as_CFTypeRef().cast(),
                current.as_CFTypeRef().cast(),
                ptr::null(),
            )
        };
        if raw.is_null() {
            return Err(super::unsupported());
        }
        let delta = unsafe { CFType::wrap_under_create_rule(raw.cast()) };
        let dictionary = dictionary(&delta).ok_or_else(super::unsupported)?;
        let list = dictionary
            .find(CFString::new("IOReportChannels"))
            .and_then(|value| value.downcast::<CFArray>())
            .ok_or_else(super::unsupported)?;
        let mut efficiency = Vec::new();
        let mut performance = Vec::new();
        for index in 0..list.len() {
            let item = unsafe {
                core_foundation::array::CFArrayGetValueAtIndex(list.as_concrete_TypeRef(), index)
            };
            if item.is_null()
                || unsafe { core_foundation::base::CFGetTypeID(item) }
                    != CFDictionary::<CFType, CFType>::type_id()
            {
                return Err(super::unsupported());
            }
            let item = item.cast();
            let name = unsafe { IOReportChannelGetChannelName(item) };
            if name.is_null() {
                continue;
            }
            let name = unsafe { CFString::wrap_under_get_rule(name) }.to_string();
            let Some(is_efficiency) = cluster(&name) else {
                continue;
            };
            let count = unsafe { IOReportStateGetCount(item) };
            if !(1..=68).contains(&count) {
                return Err(super::unsupported());
            }
            let mut states = Vec::new();
            for state in 0..count {
                let name = unsafe { IOReportStateGetNameForIndex(item, state) };
                if name.is_null() {
                    return Err(super::unsupported());
                }
                states.push((
                    unsafe { CFString::wrap_under_get_rule(name) }.to_string(),
                    unsafe { IOReportStateGetResidency(item, state) },
                ));
            }
            if let Some(value) = active_average(
                &states,
                if is_efficiency {
                    &self.efficiency
                } else {
                    &self.performance
                },
            ) {
                if is_efficiency {
                    efficiency.push(value)
                } else {
                    performance.push(value)
                };
            }
        }
        fn mean(values: &[f64]) -> Option<f64> {
            (!values.is_empty()).then(|| values.iter().sum::<f64>() / values.len() as f64)
        }
        let efficiency_mhz = mean(&efficiency);
        let performance_mhz = mean(&performance);
        // Do not substitute the minimum clock when a cluster has no active-state residency.
        if efficiency_mhz.is_none() && performance_mhz.is_none() {
            return Ok(None);
        }
        Ok(Some(CpuFrequency {
            // Cluster activity differs; do not invent an overall core-weighted frequency.
            average_mhz: None,
            efficiency_mhz,
            performance_mhz,
            source: CpuFrequencySource::ApplePerformanceStates,
        }))
    }
}
#[derive(Default)]
pub(super) struct FrequencyReader {
    subscription: Option<Subscription>,
    retry_at: Option<Instant>,
    failure_logged: bool,
}
impl FrequencyReader {
    pub(super) fn pause(&mut self) {
        if let Some(subscription) = &mut self.subscription {
            subscription.previous = None;
        }
    }
    pub(super) fn read(&mut self) -> PlatformResult<Option<CpuFrequency>> {
        if self.retry_at.is_some_and(|at| Instant::now() < at) {
            return Err(super::unsupported());
        }
        let result = (|| {
            if self.subscription.is_none() {
                self.subscription = Some(Subscription::open()?);
            }
            self.subscription
                .as_mut()
                .expect("opened frequency subscription")
                .read()
        })();
        if let Err(error) = &result {
            if !self.failure_logged {
                log::info!("cpu_frequency_unavailable source=ioreport_performance_states error={} retry_seconds=30",crate::diagnostics::text(error));
                self.failure_logged = true;
            }
            self.subscription = None;
            self.retry_at = Some(Instant::now() + Duration::from_secs(30));
        }
        if result.is_ok() {
            self.failure_logged = false;
        }
        result
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn active_frequency_excludes_idle_and_rejects_unknown_state_layouts() {
        let mut states = vec![
            ("DOWN".into(), 200),
            ("IDLE".into(), 800),
            ("V0P1".into(), 25),
            ("V1P0".into(), 75),
        ];
        assert_eq!(active_average(&states, &[1000.0, 2000.0]), Some(1750.0));
        states[2].0 = "V1P1".into();
        assert_eq!(active_average(&states, &[1000.0, 2000.0]), None);
        assert_eq!(cluster("PCPU1"), Some(false));
        assert_eq!(cluster("PCPM_IDLE"), None);
        assert_eq!(decode_table(&[0; 8], 1_000_000.0), None);
    }
}
