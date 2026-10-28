//! Apple Silicon active-state averages. Unknown tables or channel layouts fail closed.
use super::{CpuFrequency, CpuFrequencyActivity, CpuFrequencyClass, CpuFrequencySource};
use crate::{PlatformError, PlatformErrorCode};

#[cfg(test)]
mod a18_pro_tests;
mod frequency_layout;
use crate::PlatformResult;
use core_foundation::{
    array::CFArray,
    base::{CFType, CFTypeRef, TCFType},
    data::CFData,
    dictionary::{CFDictionary, CFMutableDictionaryRef},
    string::{CFString, CFStringRef},
};
use frequency_layout::{
    active_average, activity_ratios, decode_table, CoreKind, FrequencyLayout, KINDS,
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
    fn IOServiceGetMatchingServices(
        port: u32,
        matching: CFMutableDictionaryRef,
        iterator: *mut u32,
    ) -> i32;
    fn IOIteratorNext(iterator: u32) -> u32;
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
        if self.0 != 0 {
            unsafe {
                IOObjectRelease(self.0);
            }
        }
    }
}
fn unavailable(stage: &str, reason: impl std::fmt::Display) -> PlatformError {
    PlatformError::new(
        PlatformErrorCode::Unsupported,
        format!("source=ioreport_performance_states stage={stage} {reason}"),
    )
}
fn property(object: u32, key: &str) -> Option<CFType> {
    let key = CFString::new(key);
    let raw = unsafe {
        IORegistryEntryCreateCFProperty(object, key.as_concrete_TypeRef(), ptr::null(), 0)
    };
    (!raw.is_null()).then(|| unsafe { CFType::wrap_under_create_rule(raw) })
}
fn table(object: u32, key: &str, divisor: f64) -> PlatformResult<Option<Vec<f64>>> {
    let Some(value) = property(object, key) else {
        return Ok(None);
    };
    let data = value
        .downcast::<CFData>()
        .ok_or_else(|| unavailable("decode_table", format!("key={key} reason=not_data")))?;
    decode_table(data.bytes(), divisor)
        .map(Some)
        .ok_or_else(|| {
            unavailable(
                "decode_table",
                format!(
                    "key={key} bytes={} divisor={divisor} reason=invalid_frequency_table",
                    data.len()
                ),
            )
        })
}
fn load_tables(layout: FrequencyLayout) -> PlatformResult<[Vec<f64>; 3]> {
    let mut tables: [Vec<f64>; 3] = Default::default();
    for node in [c"pmgr", c"pmgr-child"] {
        let mut iterator = 0;
        let code = unsafe {
            IOServiceGetMatchingServices(0, IOServiceNameMatching(node.as_ptr()), &mut iterator)
        };
        if code != 0 {
            return Err(unavailable(
                "match_power_manager",
                format!("node={} native_code={code}", node.to_string_lossy()),
            ));
        }
        let iterator = Object(iterator);
        loop {
            let entry = unsafe { IOIteratorNext(iterator.0) };
            if entry == 0 {
                break;
            }
            let entry = Object(entry);
            let clusters = layout
                .uses_cluster_metadata()
                .then(|| property(entry.0, "acc-clusters"))
                .flatten();
            let data = clusters
                .as_ref()
                .and_then(|value| value.downcast::<CFData>());
            if clusters.is_some() && data.is_none() {
                return Err(unavailable("decode_clusters", "reason=not_data"));
            }
            let keys = layout
                .table_keys(data.as_ref().map(|data| data.bytes()))
                .ok_or_else(|| {
                    unavailable(
                        "decode_clusters",
                        format!(
                            "node={} layout={layout:?} bytes={} reason=unknown_cluster_layout",
                            node.to_string_lossy(),
                            data.as_ref().map_or(0, |data| data.len()),
                        ),
                    )
                })?;
            for kind in KINDS {
                let index = kind as usize;
                if tables[index].is_empty() {
                    for slot in keys[index].into_iter().flatten() {
                        let key = format!("voltage-states{slot}-sram");
                        match table(entry.0, &key, layout.divisor()) {
                            Ok(Some(values)) => {
                                log::info!("cpu_frequency_table node={} kind={kind:?} key={key} states={} min_mhz={} max_mhz={}", node.to_string_lossy(), values.len(), values.iter().copied().fold(f64::INFINITY, f64::min), values.iter().copied().fold(f64::NEG_INFINITY, f64::max));
                                tables[index] = values;
                                break;
                            }
                            Ok(None) => {}
                            Err(error) => {
                                // An absent core tier may retain an empty firmware table. Only
                                // identified live channels require a table, checked in read().
                                log::info!("cpu_frequency_table_rejected node={} kind={kind:?} key={key} code={:?} error={} outcome=try_next_identified_table",node.to_string_lossy(),error.code(),crate::diagnostics::text(&error));
                            }
                        }
                    }
                }
            }
        }
    }
    if layout.complete_tables(&mut tables) {
        log::info!(
            "cpu_frequency_table kind=Performance source=shared_pacc generation=6 states={}",
            tables[CoreKind::Performance as usize].len()
        );
    }
    if tables.iter().all(Vec::is_empty) {
        return Err(unavailable(
            "load_tables",
            format!("layout={layout:?} nodes=pmgr,pmgr-child reason=no_identified_tables"),
        ));
    }
    Ok(tables)
}
pub(super) fn frequency_classes(model: &str) -> Vec<super::CpuFrequencyClass> {
    use super::CpuFrequencyClass;
    // Discover installed tiers from firmware, never from fluctuating live values.
    // This probe does not subscribe to IOReport or start frequency sampling.
    FrequencyLayout::identify(model)
        .and_then(|layout| match load_tables(layout) {
            Ok(tables) => Some(tables),
            Err(error) => {
                log::info!("cpu_frequency_classes_unavailable model={} code={:?} error={} outcome=no_selectable_classes",crate::diagnostics::text(model),error.code(),crate::diagnostics::text(&error));
                None
            }
        })
        .map(|tables| {
            [
                CpuFrequencyClass::Efficiency,
                CpuFrequencyClass::Performance,
                CpuFrequencyClass::Super,
            ]
            .into_iter()
            .zip(tables)
            .filter_map(|(kind, table)| (!table.is_empty()).then_some(kind))
            .collect()
        })
        .unwrap_or_default()
}

struct Subscription {
    channels: CFType,
    subscription: CFType,
    previous: Option<(CFType, Instant)>,
    layout: FrequencyLayout,
    expected_channels: [Vec<String>; 3],
    tables: [Vec<f64>; 3],
}
fn dictionary(value: &CFType) -> Option<CFDictionary<CFString, CFType>> {
    let value = value.downcast::<CFDictionary>()?;
    Some(unsafe { CFDictionary::wrap_under_get_rule(value.as_concrete_TypeRef()) })
}
fn channel_names(sample: &CFType, layout: FrequencyLayout) -> PlatformResult<[Vec<String>; 3]> {
    let dictionary = dictionary(sample)
        .ok_or_else(|| unavailable("identify_channels", "reason=not_dictionary"))?;
    let list = dictionary
        .find(CFString::new("IOReportChannels"))
        .and_then(|value| value.downcast::<CFArray>())
        .ok_or_else(|| unavailable("identify_channels", "reason=channels_missing_or_invalid"))?;
    let mut names: [Vec<String>; 3] = Default::default();
    for index in 0..list.len() {
        let item = unsafe {
            core_foundation::array::CFArrayGetValueAtIndex(list.as_concrete_TypeRef(), index)
        };
        if item.is_null()
            || unsafe { core_foundation::base::CFGetTypeID(item) }
                != CFDictionary::<CFType, CFType>::type_id()
        {
            return Err(unavailable("identify_channels", "reason=not_dictionary"));
        }
        let name = unsafe { IOReportChannelGetChannelName(item.cast()) };
        if !name.is_null() {
            let name = unsafe { CFString::wrap_under_get_rule(name) }.to_string();
            if let Some(kind) = layout.cluster(&name) {
                names[kind as usize].push(name);
            }
        }
    }
    Ok(names)
}
impl Subscription {
    fn open() -> PlatformResult<Self> {
        let model = super::identity()
            .model
            .ok_or_else(|| unavailable("identify_model", "reason=model_missing"))?;
        let layout = FrequencyLayout::identify(&model).ok_or_else(|| {
            unavailable(
                "identify_model",
                format!(
                    "model={} reason=unidentified_layout",
                    crate::diagnostics::text(&model)
                ),
            )
        })?;
        let tables = load_tables(layout)?;
        let group = CFString::new("CPU Stats");
        // Newer chips expose EACC/PACC complex names; their per-core channels retain the
        // ECPU/MCPU/PCPU identity needed to keep three distinct frequency classes.
        let subgroup = CFString::new(layout.subgroup());
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
            return Err(unavailable(
                "create_channels",
                format!(
                    "model={} subgroup={} reason=null_result",
                    crate::diagnostics::text(&model),
                    subgroup
                ),
            ));
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
            return Err(unavailable("copy_channels", "reason=null_result"));
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
            return Err(unavailable("subscribe", "reason=null_result"));
        }
        let subscription = unsafe { CFType::wrap_under_create_rule(raw) };
        let expected_channels = channel_names(&channels, layout)?;
        log::info!("cpu_frequency_capability model={} layout={layout:?} subgroup={} efficiency_states={} performance_states={} super_states={}",crate::diagnostics::text(&model),subgroup,tables[0].len(),tables[1].len(),tables[2].len());
        Ok(Self {
            channels,
            subscription,
            previous: None,
            layout,
            expected_channels,
            tables,
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
            return Err(unavailable("sample", "reason=null_result"));
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
            return Err(unavailable("sample_delta", "reason=null_result"));
        }
        let delta = unsafe { CFType::wrap_under_create_rule(raw.cast()) };
        let dictionary = dictionary(&delta)
            .ok_or_else(|| unavailable("decode_sample", "reason=not_dictionary"))?;
        let list = dictionary
            .find(CFString::new("IOReportChannels"))
            .and_then(|value| value.downcast::<CFArray>())
            .ok_or_else(|| unavailable("decode_channels", "reason=channels_missing_or_invalid"))?;
        let mut values: [Vec<f64>; 3] = Default::default();
        let mut activity: [Vec<Option<[f64; 4]>>; 3] = Default::default();
        let mut seen_channels: [Vec<String>; 3] = Default::default();
        let mut matched = 0;
        let mut observed_names = Vec::new();
        for index in 0..list.len() {
            let item = unsafe {
                core_foundation::array::CFArrayGetValueAtIndex(list.as_concrete_TypeRef(), index)
            };
            if item.is_null()
                || unsafe { core_foundation::base::CFGetTypeID(item) }
                    != CFDictionary::<CFType, CFType>::type_id()
            {
                return Err(unavailable(
                    "decode_channels",
                    format!("index={index} reason=not_dictionary"),
                ));
            }
            let item = item.cast();
            let name = unsafe { IOReportChannelGetChannelName(item) };
            if name.is_null() {
                continue;
            }
            let name = unsafe { CFString::wrap_under_get_rule(name) }.to_string();
            let Some(kind) = self.layout.cluster(&name) else {
                if observed_names.len() < 24 {
                    observed_names.push(crate::diagnostics::text(&name));
                }
                continue;
            };
            matched += 1;
            seen_channels[kind as usize].push(name.clone());
            let count = unsafe { IOReportStateGetCount(item) };
            if !(1..=68).contains(&count) {
                return Err(unavailable(
                    "decode_states",
                    format!(
                        "channel={} count={count} reason=invalid_state_count",
                        crate::diagnostics::text(&name)
                    ),
                ));
            }
            let mut states = Vec::new();
            for state in 0..count {
                let state_name = unsafe { IOReportStateGetNameForIndex(item, state) };
                if state_name.is_null() {
                    return Err(unavailable(
                        "decode_states",
                        format!(
                            "channel={} state={state} reason=state_name_missing",
                            crate::diagnostics::text(&name)
                        ),
                    ));
                }
                states.push((
                    unsafe { CFString::wrap_under_get_rule(state_name) }.to_string(),
                    unsafe { IOReportStateGetResidency(item, state) },
                ));
            }
            let frequencies = &self.tables[kind as usize];
            let value = active_average(&states, frequencies).map_err(|reason| {
                unavailable(
                    "decode_states",
                    format!(
                        "channel={} kind={kind:?} count={count} table_states={} state_names={:?} reason={reason}",
                        crate::diagnostics::text(&name),
                        frequencies.len(), states.iter().map(|(name, _)| name.as_str()).take(24).collect::<Vec<_>>()
                    ),
                )
            })?;
            activity[kind as usize].push(activity_ratios(&states));
            if let Some(value) = value {
                values[kind as usize].push(value);
            }
        }
        if matched == 0 {
            return Err(unavailable("decode_channels", format!("layout={:?} channel_count={} observed={observed_names:?} reason=no_identified_cpu_channels",self.layout,list.len())));
        }
        Ok(Some(complete_sample(
            values,
            activity,
            &self.expected_channels,
            &seen_channels,
        )))
    }
}

fn complete_sample(
    mut values: [Vec<f64>; 3],
    mut activity: [Vec<Option<[f64; 4]>>; 3],
    expected_channels: &[Vec<String>; 3],
    seen_channels: &[Vec<String>; 3],
) -> CpuFrequency {
    for kind in KINDS {
        let index = kind as usize;
        let expected = &expected_channels[index];
        let seen = &seen_channels[index];
        // A missing or zero-duration channel cannot establish either the clock
        // or inactivity of the whole class. Keep other complete classes usable.
        if !frequency_layout::complete_channel_coverage(expected, seen)
            || activity[index].iter().any(Option::is_none)
        {
            values[index].clear();
            activity[index].clear();
        }
    }
    let mut sample = cluster_frequency(values);
    sample.activity = summarize_activity(activity);
    sample
}

fn cluster_frequency(values: [Vec<f64>; 3]) -> CpuFrequency {
    fn mean(values: &[f64]) -> Option<f64> {
        (!values.is_empty()).then(|| values.iter().sum::<f64>() / values.len() as f64)
    }
    let efficiency_mhz = mean(&values[0]);
    let performance_mhz = mean(&values[1]);
    let super_mhz = mean(&values[2]);
    // A completed idle interval must clear old clocks; only a missing baseline
    // returns no sample. Do not replace absent active residency with a minimum.
    CpuFrequency {
        // Cluster activity differs; do not invent an overall core-weighted clock.
        average_mhz: None,
        efficiency_mhz,
        performance_mhz,
        super_mhz,
        source: CpuFrequencySource::ApplePerformanceStates,
        activity: Vec::new(),
    }
}

fn summarize_activity(channels: [Vec<Option<[f64; 4]>>; 3]) -> Vec<CpuFrequencyActivity> {
    [
        CpuFrequencyClass::Efficiency,
        CpuFrequencyClass::Performance,
        CpuFrequencyClass::Super,
    ]
    .into_iter()
    .zip(channels)
    .filter_map(|(class, channels)| {
        if channels.is_empty() {
            return None;
        }
        // Every observed channel must have a usable interval before calling
        // the whole class idle. One missing duration cannot prove inactivity.
        let ratios: Option<Vec<_>> = channels.into_iter().collect();
        let ratios = ratios?;
        let mut mean = [0.0; 4];
        for ratio in &ratios {
            for (sum, value) in mean.iter_mut().zip(ratio) {
                *sum += value / ratios.len() as f64;
            }
        }
        Some(CpuFrequencyActivity {
            class,
            active_ratio: mean[0],
            idle_ratio: mean[1],
            down_ratio: mean[2],
            off_ratio: mean[3],
        })
    })
    .collect()
}

#[derive(Default)]
pub(super) struct FrequencyReader {
    subscription: Option<Subscription>,
    retry_at: Option<Instant>,
    previous_error: Option<String>,
}
impl FrequencyReader {
    pub(super) fn pause(&mut self) {
        if let Some(subscription) = &mut self.subscription {
            subscription.previous = None;
        }
    }
    pub(super) fn read(&mut self) -> PlatformResult<Option<CpuFrequency>> {
        if self.retry_at.is_some_and(|at| Instant::now() < at) {
            return Err(unavailable("retry_backoff", "retry_seconds=30"));
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
            if self.previous_error.as_deref() != Some(error.diagnostic()) {
                log::info!("cpu_frequency_unavailable source=ioreport_performance_states model={} code={:?} error={} retry_seconds=30",crate::diagnostics::text(super::identity().model.as_deref().unwrap_or("unknown")),error.code(),crate::diagnostics::text(error));
                self.previous_error = Some(error.diagnostic().to_string());
            }
            self.subscription = None;
            self.retry_at = Some(Instant::now() + Duration::from_secs(30));
        }
        if matches!(result, Ok(Some(_))) && self.previous_error.take().is_some() {
            log::info!("cpu_frequency_recovered source=ioreport_performance_states");
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires physical Apple Silicon and live IOReport channels"]
    fn native_frequency_sampling_preserves_installed_classes_and_idle_intervals() {
        let identity = super::super::identity();
        let layout = FrequencyLayout::identify(identity.model.as_deref().unwrap()).unwrap();
        let mut subscription = Subscription::open().expect("native IOReport subscription");
        assert_eq!(subscription.layout, layout);
        assert!(subscription.read().unwrap().is_none());
        let mut completed = 0;
        for _ in 0..8 {
            std::thread::sleep(Duration::from_millis(150));
            let sample = subscription
                .read()
                .unwrap()
                .expect("completed native interval");
            assert_eq!(sample.source, CpuFrequencySource::ApplePerformanceStates);
            assert!(sample.average_mhz.is_none());
            assert!(!sample.activity.is_empty());
            for (kind, value, class) in [
                (
                    CoreKind::Efficiency,
                    sample.efficiency_mhz,
                    CpuFrequencyClass::Efficiency,
                ),
                (
                    CoreKind::Performance,
                    sample.performance_mhz,
                    CpuFrequencyClass::Performance,
                ),
                (CoreKind::Super, sample.super_mhz, CpuFrequencyClass::Super),
            ] {
                if let Some(value) = value {
                    assert!(identity.frequency_classes.contains(&class));
                    assert!(subscription.tables[kind as usize]
                        .iter()
                        .any(|mhz| *mhz <= value + 0.001));
                    assert!(subscription.tables[kind as usize]
                        .iter()
                        .any(|mhz| *mhz >= value - 0.001));
                }
                if sample
                    .activity
                    .iter()
                    .any(|entry| entry.class == class && entry.is_idle())
                {
                    assert!(
                        value.is_none(),
                        "idle cores must not publish a minimum clock"
                    );
                }
            }
            println!("model={:?} layout={layout:?} efficiency_mhz={:?} performance_mhz={:?} super_mhz={:?} activity={:?}", identity.model, sample.efficiency_mhz, sample.performance_mhz, sample.super_mhz, sample.activity);
            completed += 1;
        }
        assert_eq!(completed, 8);
    }

    #[test]
    fn incomplete_intervals_do_not_publish_a_partial_class_frequency() {
        let expected = [
            vec!["ECPU".into()],
            vec!["PCPU".into(), "PCPU1".into()],
            vec![],
        ];
        for (seen, activity) in [
            (
                [vec!["ECPU".into()], vec!["PCPU".into()], vec![]],
                [
                    vec![Some([1.0, 0.0, 0.0, 0.0])],
                    vec![Some([1.0, 0.0, 0.0, 0.0])],
                    vec![],
                ],
            ),
            (
                expected.clone(),
                [
                    vec![Some([1.0, 0.0, 0.0, 0.0])],
                    vec![Some([1.0, 0.0, 0.0, 0.0]), None],
                    vec![],
                ],
            ),
        ] {
            let sample = complete_sample(
                [vec![1500.0], vec![3000.0], vec![]],
                activity,
                &expected,
                &seen,
            );
            assert_eq!(sample.efficiency_mhz, Some(1500.0));
            assert!(
                sample.performance_mhz.is_none(),
                "An incomplete class must not expose a partial average"
            );
            assert!(!sample
                .activity
                .iter()
                .any(|value| value.class == CpuFrequencyClass::Performance));
        }
        let sample = complete_sample(
            [vec![1500.0], vec![3000.0, 4000.0], vec![]],
            [
                vec![Some([1.0, 0.0, 0.0, 0.0])],
                vec![Some([1.0, 0.0, 0.0, 0.0]); 2],
                vec![],
            ],
            &expected,
            &expected,
        );
        assert_eq!(sample.performance_mhz, Some(3500.0));
        let idle = complete_sample(
            [vec![1500.0], vec![], vec![]],
            [
                vec![Some([1.0, 0.0, 0.0, 0.0])],
                vec![Some([0.0, 0.4, 0.6, 0.0]); 2],
                vec![],
            ],
            &expected,
            &expected,
        );
        assert!(idle.performance_mhz.is_none());
        assert!(idle
            .activity
            .iter()
            .any(|value| { value.class == CpuFrequencyClass::Performance && value.is_idle() }));
    }

    #[test]
    fn a_whole_core_type_is_idle_only_with_complete_positive_duration_channels() {
        let activity = summarize_activity([
            vec![Some([0.0, 0.4, 0.6, 0.0]), Some([0.0, 1.0, 0.0, 0.0])],
            vec![],
            vec![Some([0.0, 1.0, 0.0, 0.0]), None],
        ]);
        assert_eq!(activity.len(), 1);
        assert_eq!(activity[0].class, CpuFrequencyClass::Efficiency);
        assert!(activity[0].is_idle());
        let mixed = summarize_activity([
            vec![Some([0.0, 1.0, 0.0, 0.0]), Some([0.5, 0.5, 0.0, 0.0])],
            vec![],
            vec![],
        ]);
        assert!(!mixed[0].is_idle());
        assert_eq!(mixed[0].active_ratio, 0.25);
        assert!(summarize_activity(Default::default()).is_empty());
    }

    #[test]
    fn completed_idle_interval_is_a_fresh_empty_frequency_sample() {
        let sample = cluster_frequency(Default::default());
        assert!(sample.average_mhz.is_none());
        assert!(sample.efficiency_mhz.is_none());
        assert!(sample.performance_mhz.is_none());
        assert!(sample.super_mhz.is_none());
    }

    #[test]
    fn complex_channels_include_the_unnumbered_first_performance_cluster() {
        let mut values: [Vec<f64>; 3] = Default::default();
        for (name, residencies) in [("ECPU", [75, 25]), ("PCPU", [25, 75]), ("PCPU1", [50, 50])] {
            let kind = FrequencyLayout::M(3)
                .cluster(name)
                .expect("The native channel must be classified");
            let states = vec![
                ("IDLE".into(), 1000),
                ("V0P1".into(), residencies[0]),
                ("V1P0".into(), residencies[1]),
            ];
            if let Some(value) = active_average(&states, &[1000.0, 2000.0]).unwrap() {
                values[kind as usize].push(value);
            }
        }
        assert_eq!(values[CoreKind::Performance as usize].len(), 2);
        let sample = cluster_frequency(values);
        assert_eq!(sample.efficiency_mhz, Some(1250.0));
        assert_eq!(sample.performance_mhz, Some(1625.0));
    }
}
