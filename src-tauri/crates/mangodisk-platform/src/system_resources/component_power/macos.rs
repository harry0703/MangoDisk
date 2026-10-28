//! Nonblocking IOReport energy deltas; the resident worker owns the sample cadence.
use super::{
    channel_matches, energy_watts, unavailable, Component, ComponentPower, ComponentPowerSource,
};
use crate::{PlatformError, PlatformErrorCode, PlatformResult};
use core_foundation::{
    array::CFArray,
    base::{CFType, CFTypeRef, TCFType},
    dictionary::{CFDictionary, CFDictionaryRef, CFMutableDictionaryRef},
    string::{CFString, CFStringRef},
};
use std::{
    ptr,
    time::{Instant, SystemTime},
};

#[link(name = "IOReport")]
unsafe extern "C" {
    fn IOReportCopyChannelsInGroup(
        group: CFStringRef,
        subgroup: CFStringRef,
        a: u64,
        b: u64,
        c: u64,
    ) -> CFDictionaryRef;
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
    ) -> CFDictionaryRef;
    fn IOReportCreateSamplesDelta(
        a: CFDictionaryRef,
        b: CFDictionaryRef,
        c: CFTypeRef,
    ) -> CFDictionaryRef;
    fn IOReportChannelGetChannelName(channel: CFDictionaryRef) -> CFStringRef;
    fn IOReportChannelGetUnitLabel(channel: CFDictionaryRef) -> CFStringRef;
    fn IOReportSimpleGetIntegerValue(channel: CFDictionaryRef, index: i32) -> i64;
}

pub(super) struct Reader {
    channels: CFType,
    subscription: CFType,
    previous: Option<(CFType, Instant, SystemTime)>,
    expected: Vec<String>,
    component: Component,
}
fn owned(raw: CFTypeRef, stage: &str) -> PlatformResult<CFType> {
    if raw.is_null() {
        Err(unavailable(stage))
    } else {
        Ok(unsafe { CFType::wrap_under_create_rule(raw) })
    }
}
fn channels(value: &CFType) -> PlatformResult<Vec<CFDictionary<CFType, CFType>>> {
    let dictionary = value
        .downcast::<CFDictionary>()
        .ok_or_else(|| unavailable("dictionary"))?;
    let dictionary: CFDictionary<CFString, CFType> =
        unsafe { CFDictionary::wrap_under_get_rule(dictionary.as_concrete_TypeRef()) };
    let list = dictionary
        .find(CFString::new("IOReportChannels"))
        .and_then(|v| v.downcast::<CFArray>())
        .ok_or_else(|| unavailable("channels"))?;
    let mut result = Vec::new();
    for index in 0..list.len() {
        let item = unsafe {
            core_foundation::array::CFArrayGetValueAtIndex(list.as_concrete_TypeRef(), index)
        };
        if item.is_null()
            || unsafe { core_foundation::base::CFGetTypeID(item) }
                != CFDictionary::<CFType, CFType>::type_id()
        {
            return Err(unavailable("channel_type"));
        }
        result.push(unsafe { CFDictionary::wrap_under_get_rule(item.cast()) });
    }
    Ok(result)
}
fn name(channel: &CFDictionary<CFType, CFType>) -> String {
    let raw = unsafe { IOReportChannelGetChannelName(channel.as_concrete_TypeRef()) };
    if raw.is_null() {
        String::new()
    } else {
        unsafe { CFString::wrap_under_get_rule(raw) }.to_string()
    }
}
impl Reader {
    pub(super) fn new(component: Component) -> PlatformResult<Self> {
        if !cfg!(target_arch = "aarch64") {
            return Err(unavailable("architecture"));
        }
        // macOS 27 can expose frozen legacy counters. CLPC needs a separately
        // validated channel mapping; do not publish plausible-looking zero power.
        if !legacy_energy_supported() {
            return Err(unavailable("legacy_energy_os_version"));
        }
        let group = CFString::new("Energy Model");
        let source = owned(
            unsafe {
                IOReportCopyChannelsInGroup(group.as_concrete_TypeRef(), ptr::null(), 0, 0, 0)
            }
            .cast(),
            "discover",
        )?;
        let selected: Vec<_> = channels(&source)?
            .into_iter()
            .filter(|item| channel_matches(component, &name(item)))
            .collect();
        let expected: Vec<_> = selected.iter().map(name).collect();
        if expected.is_empty() {
            return Err(unavailable("identify"));
        }
        let copied = unsafe {
            core_foundation::dictionary::CFDictionaryCreateMutableCopy(
                ptr::null(),
                0,
                source.as_CFTypeRef().cast(),
            )
        };
        let channels = owned(copied.cast(), "copy")?;
        // Subscribe only to the requested component, excluding ANE, memory and
        // unrelated energy rails from both native sampling and delta allocation.
        let key = CFString::new("IOReportChannels");
        let selected = CFArray::from_CFTypes(&selected);
        unsafe {
            core_foundation::dictionary::CFDictionarySetValue(
                copied,
                key.as_CFTypeRef(),
                selected.as_CFTypeRef(),
            );
        }
        let mut output = ptr::null_mut();
        let raw =
            unsafe { IOReportCreateSubscription(ptr::null(), copied, &mut output, 0, ptr::null()) };
        if !output.is_null() {
            drop(owned(output.cast(), "subscription_output")?);
        }
        let subscription = owned(raw, "subscribe")?;
        Ok(Self {
            channels,
            subscription,
            previous: None,
            expected,
            component,
        })
    }
    pub(super) fn read(&mut self) -> PlatformResult<Option<ComponentPower>> {
        let raw = unsafe {
            IOReportCreateSamples(
                self.subscription.as_CFTypeRef(),
                self.channels.as_CFTypeRef().cast_mut().cast(),
                ptr::null(),
            )
        };
        let current = owned(raw.cast(), "sample")?;
        let now = Instant::now();
        let Some((previous, at, wall_at)) =
            self.previous
                .replace((current.clone(), now, SystemTime::now()))
        else {
            return Ok(None);
        };
        let elapsed = now.duration_since(at);
        // Do not average energy over sleep, paused monitoring, or stalled workers.
        let Some(seconds) =
            super::sample_seconds(elapsed, SystemTime::now().duration_since(wall_at).ok())
        else {
            return Ok(None);
        };
        let raw = unsafe {
            IOReportCreateSamplesDelta(
                previous.as_CFTypeRef().cast(),
                current.as_CFTypeRef().cast(),
                ptr::null(),
            )
        };
        let delta = owned(raw.cast(), "delta")?;
        let mut seen = Vec::new();
        let mut watts = 0.0;
        for item in channels(&delta)? {
            let channel = name(&item);
            if !channel_matches(self.component, &channel) {
                continue;
            }
            let unit = unsafe { IOReportChannelGetUnitLabel(item.as_concrete_TypeRef()) };
            if unit.is_null() {
                return Err(unavailable("unit"));
            }
            let unit = unsafe { CFString::wrap_under_get_rule(unit) }.to_string();
            let energy = unsafe { IOReportSimpleGetIntegerValue(item.as_concrete_TypeRef(), 0) };
            watts += energy_watts(energy, &unit, seconds).ok_or_else(|| PlatformError::new(PlatformErrorCode::InvalidData, format!("source=ioreport_energy channel={channel} unit={unit} energy={energy} reason=invalid_delta")))?;
            seen.push(channel);
        }
        seen.sort();
        let mut expected = self.expected.clone();
        expected.sort();
        if seen != expected || watts > super::COMPONENT_POWER_MAX_WATTS {
            return Err(unavailable("channel_set_changed"));
        }
        Ok(Some(ComponentPower {
            watts,
            source: ComponentPowerSource::IoReport,
        }))
    }
}

fn legacy_energy_supported() -> bool {
    let mut bytes = [0u8; 64];
    let mut size = bytes.len();
    let code = unsafe {
        libc::sysctlbyname(
            c"kern.osproductversion".as_ptr(),
            bytes.as_mut_ptr().cast(),
            &mut size,
            ptr::null_mut(),
            0,
        )
    };
    if code != 0 || size > bytes.len() {
        return false;
    }
    std::str::from_utf8(&bytes[..size])
        .ok()
        .and_then(|version| version.split('.').next()?.parse::<u32>().ok())
        .is_some_and(|major| major < 27)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires native Apple Silicon Energy Model channels"]
    fn native_energy_baselines_resume_without_averaging_across_pauses() {
        let mut reader = Reader::new(Component::Cpu).unwrap();
        assert!(reader.read().unwrap().is_none());
        std::thread::sleep(std::time::Duration::from_millis(150));
        let value = reader.read().unwrap().unwrap();
        assert!(value.watts.is_finite() && value.watts >= 0.0);
        let (_, _, wall_at) = reader.previous.as_mut().unwrap();
        *wall_at = SystemTime::now() - std::time::Duration::from_secs(60);
        assert!(reader.read().unwrap().is_none());
        std::thread::sleep(std::time::Duration::from_millis(150));
        assert!(reader.read().unwrap().is_some());
        println!(
            "native_component_power_resume source=ioreport outcome=ready watts={}",
            value.watts
        );
    }
}
