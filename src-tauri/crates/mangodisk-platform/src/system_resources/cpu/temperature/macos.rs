//! Read-only SMC queries. Only documented community CPU key mappings are selected.
//! Key identities: https://github.com/exelban/stats/blob/master/Modules/Sensors/values.swift
use super::{unsupported, valid_celsius, CpuTemperature, CpuTemperatureKind, CpuTemperatureSource};
use crate::{PlatformError, PlatformErrorCode, PlatformResult};
use core_foundation::dictionary::CFMutableDictionaryRef;
use std::{ffi::c_void, mem::size_of};

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    fn IOServiceMatching(name: *const std::ffi::c_char) -> CFMutableDictionaryRef;
    fn IOServiceGetMatchingServices(
        port: u32,
        matching: CFMutableDictionaryRef,
        iterator: *mut u32,
    ) -> i32;
    fn IOIteratorNext(iterator: u32) -> u32;
    fn IORegistryEntryGetName(entry: u32, name: *mut std::ffi::c_char) -> i32;
    fn IOObjectRelease(object: u32) -> i32;
    fn IOServiceOpen(service: u32, task: u32, kind: u32, connection: *mut u32) -> i32;
    fn IOServiceClose(connection: u32) -> i32;
    fn IOConnectCallStructMethod(
        connection: u32,
        selector: u32,
        input: *const c_void,
        input_size: usize,
        output: *mut c_void,
        output_size: *mut usize,
    ) -> i32;
}
unsafe extern "C" {
    fn mach_task_self() -> u32;
}

#[repr(C)]
#[derive(Default)]
struct Version {
    major: u8,
    minor: u8,
    build: u8,
    reserved: u8,
    release: u16,
}
#[repr(C)]
#[derive(Default)]
struct Limits {
    version: u16,
    length: u16,
    cpu: u32,
    gpu: u32,
    memory: u32,
}
#[repr(C)]
#[derive(Default, Clone, Copy)]
struct KeyInfo {
    size: u32,
    kind: u32,
    attributes: u8,
}
#[repr(C)]
#[derive(Default)]
struct Packet {
    key: u32,
    version: Version,
    limits: Limits,
    info: KeyInfo,
    result: u8,
    status: u8,
    command: u8,
    index: u32,
    bytes: [u8; 32],
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
struct Connection(u32);
impl Drop for Connection {
    fn drop(&mut self) {
        unsafe {
            IOServiceClose(self.0);
        }
    }
}
impl Connection {
    fn open() -> PlatformResult<Self> {
        let mut iterator = 0;
        // Matching consumes the dictionary; the iterator and each service are owned.
        let code = unsafe {
            IOServiceGetMatchingServices(0, IOServiceMatching(c"AppleSMC".as_ptr()), &mut iterator)
        };
        let iterator = Object(iterator);
        if code != 0 {
            return Err(native_error("enumerate", code));
        }
        loop {
            let service = Object(unsafe { IOIteratorNext(iterator.0) });
            if service.0 == 0 {
                return Err(unsupported());
            }
            let mut name = [0i8; 128];
            if unsafe { IORegistryEntryGetName(service.0, name.as_mut_ptr()) } != 0 {
                continue;
            }
            let name = name
                .into_iter()
                .take_while(|byte| *byte != 0)
                .map(|byte| byte as u8)
                .collect::<Vec<_>>();
            let expected = if cfg!(target_arch = "aarch64") {
                b"AppleSMCKeysEndpoint".as_slice()
            } else {
                b"AppleSMC".as_slice()
            };
            if name != expected {
                continue;
            }
            let mut connection = 0;
            let code = unsafe { IOServiceOpen(service.0, mach_task_self(), 0, &mut connection) };
            if code != 0 {
                return Err(native_error("open", code));
            }
            if connection == 0 {
                return Err(unsupported());
            }
            return Ok(Self(connection));
        }
    }
    fn query(&self, input: Packet) -> PlatformResult<Option<Packet>> {
        let mut output = Packet::default();
        let mut size = size_of::<Packet>();
        // Packet is the fixed C ABI and both buffers stay alive throughout the call.
        let code = unsafe {
            IOConnectCallStructMethod(
                self.0,
                2,
                (&input as *const Packet).cast(),
                size_of::<Packet>(),
                (&mut output as *mut Packet).cast(),
                &mut size,
            )
        };
        if code != 0 {
            return Err(native_error("read", code));
        }
        if size != size_of::<Packet>() {
            return Err(PlatformError::new(
                PlatformErrorCode::InvalidData,
                format!("SMC response size={size}"),
            ));
        }
        if output.result == 132 {
            return Ok(None);
        }
        if output.result != 0 {
            return Err(PlatformError::new(
                PlatformErrorCode::OperationFailed,
                format!(
                    "SMC read key={:08x} result={} status={}",
                    input.key, output.result, output.status
                ),
            ));
        }
        Ok(Some(output))
    }
}
fn native_error(stage: &str, code: i32) -> PlatformError {
    PlatformError::new(
        PlatformErrorCode::OperationFailed,
        format!("SMC {stage} native_code=0x{:08x}", code as u32),
    )
}

pub(super) struct Reader {
    connection: Connection,
    sensors: Vec<(u32, KeyInfo)>,
    kind: CpuTemperatureKind,
}
impl Reader {
    pub(super) fn new(model: Option<&str>) -> PlatformResult<Self> {
        let (keys, kind) = cpu_keys(model).ok_or_else(unsupported)?;
        let connection = Connection::open()?;
        let sensors = discover(&connection, keys)?;
        #[cfg(target_arch = "x86_64")]
        let (sensors, kind) = if sensors.is_empty() {
            let keys = intel_core_keys(intel_physical_cores()?).ok_or_else(unsupported)?;
            let sensors = discover(&connection, keys)?;
            // Intel exposes unused core slots with plausible-looking sentinel
            // values. Use only the actual physical cores and require the full set.
            if sensors.len() != keys.len() {
                return Err(unsupported());
            }
            (sensors, CpuTemperatureKind::CoreMaximum)
        } else {
            (sensors, kind)
        };
        if sensors.is_empty() {
            return Err(unsupported());
        }
        Ok(Self {
            connection,
            sensors,
            kind,
        })
    }
    pub(super) fn read(&mut self) -> PlatformResult<CpuTemperature> {
        let mut sum = 0.0;
        let mut maximum = f64::NEG_INFINITY;
        // Missing known sensors are excluded during discovery. Once selected, every
        // sensor must be readable: silently changing the denominator changes meaning.
        for &(key, info) in &self.sensors {
            let packet = self
                .connection
                .query(Packet {
                    key,
                    info,
                    command: 5,
                    ..Default::default()
                })?
                .ok_or_else(|| {
                    PlatformError::new(
                        PlatformErrorCode::OperationFailed,
                        format!("SMC CPU sensor disappeared key={key:08x}"),
                    )
                })?;
            let value = decode(info, &packet.bytes)
                .filter(|value| valid_celsius(*value))
                .ok_or_else(|| {
                    PlatformError::new(
                        PlatformErrorCode::InvalidData,
                        format!("SMC CPU sensor invalid key={key:08x}"),
                    )
                })?;
            sum += value;
            maximum = maximum.max(value);
        }
        Ok(CpuTemperature {
            celsius: if self.kind == CpuTemperatureKind::CoreMaximum {
                maximum
            } else {
                sum / self.sensors.len() as f64
            },
            sensor_count: self.sensors.len() as u32,
            kind: self.kind,
            source: CpuTemperatureSource::AppleSmc,
        })
    }
}
fn discover(connection: &Connection, keys: &[[u8; 4]]) -> PlatformResult<Vec<(u32, KeyInfo)>> {
    let mut sensors = Vec::new();
    for key in keys {
        let key = u32::from_be_bytes(*key);
        let Some(packet) = connection.query(Packet {
            key,
            command: 9,
            ..Default::default()
        })?
        else {
            continue;
        };
        if supported_encoding(packet.info) {
            sensors.push((key, packet.info));
        }
    }
    Ok(sensors)
}

#[cfg(target_arch = "x86_64")]
fn intel_physical_cores() -> PlatformResult<usize> {
    let mut count = 0u32;
    let mut size = size_of::<u32>();
    let code = unsafe {
        libc::sysctlbyname(
            c"hw.physicalcpu_max".as_ptr(),
            (&mut count as *mut u32).cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if code != 0 || size != size_of::<u32>() {
        return Err(PlatformError::new(
            PlatformErrorCode::OperationFailed,
            format!(
                "CPU physical core count read failed: {}",
                std::io::Error::last_os_error()
            ),
        ));
    }
    Ok(count as usize)
}

#[cfg(target_arch = "x86_64")]
fn intel_core_keys(count: usize) -> Option<&'static [[u8; 4]]> {
    // TC1C through TC8C are identified CPU core sensors; do not enumerate other
    // TC-prefixed keys, which include proximity and integrated GPU temperatures.
    const KEYS: [[u8; 4]; 8] = [
        *b"TC1C", *b"TC2C", *b"TC3C", *b"TC4C", *b"TC5C", *b"TC6C", *b"TC7C", *b"TC8C",
    ];
    (1..=KEYS.len()).contains(&count).then(|| &KEYS[..count])
}
fn supported_encoding(info: KeyInfo) -> bool {
    (info.kind == u32::from_be_bytes(*b"flt ") && info.size == 4)
        || (info.kind == u32::from_be_bytes(*b"sp78") && info.size == 2)
}
fn decode(info: KeyInfo, bytes: &[u8; 32]) -> Option<f64> {
    if !supported_encoding(info) {
        return None;
    }
    if info.size == 4 {
        Some(f64::from(f32::from_le_bytes(bytes[..4].try_into().ok()?)))
    } else {
        Some(f64::from(i16::from_be_bytes(bytes[..2].try_into().ok()?)) / 256.0)
    }
}

fn cpu_keys(model: Option<&str>) -> Option<(&'static [[u8; 4]], CpuTemperatureKind)> {
    #[cfg(target_arch = "x86_64")]
    {
        let _ = model;
        Some((const { &[*b"TCAD"] }, CpuTemperatureKind::Package))
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        // A prefix alone is not an identity: Tf includes both CPU and GPU sensors,
        // and Tp0P is a power-board sensor on Intel. Unknown chips fail closed.
        let generation = model?
            .strip_prefix("Apple M")?
            .split_whitespace()
            .next()?
            .parse::<u32>()
            .ok()?;
        let keys: &'static [[u8; 4]] = match generation {
            1 => {
                const {
                    &[
                        *b"Tp09", *b"Tp0T", *b"Tp01", *b"Tp05", *b"Tp0D", *b"Tp0H", *b"Tp0L",
                        *b"Tp0P", *b"Tp0X", *b"Tp0b",
                    ]
                }
            }
            2 => {
                const {
                    &[
                        *b"Tp1h", *b"Tp1t", *b"Tp1p", *b"Tp1l", *b"Tp01", *b"Tp05", *b"Tp09",
                        *b"Tp0D", *b"Tp0X", *b"Tp0b", *b"Tp0f", *b"Tp0j",
                    ]
                }
            }
            3 => {
                const {
                    &[
                        *b"Te05", *b"Te0L", *b"Te0P", *b"Te0S", *b"Tf04", *b"Tf09", *b"Tf0A",
                        *b"Tf0B", *b"Tf0D", *b"Tf0E", *b"Tf44", *b"Tf49", *b"Tf4A", *b"Tf4B",
                        *b"Tf4D", *b"Tf4E",
                    ]
                }
            }
            4 => {
                const {
                    &[
                        *b"Te05", *b"Te0S", *b"Te09", *b"Te0H", *b"Tp01", *b"Tp05", *b"Tp09",
                        *b"Tp0D", *b"Tp0V", *b"Tp0Y", *b"Tp0b", *b"Tp0e",
                    ]
                }
            }
            5 => {
                const {
                    &[
                        *b"Tp00", *b"Tp04", *b"Tp08", *b"Tp0C", *b"Tp0G", *b"Tp0K", *b"Tp0O",
                        *b"Tp0R", *b"Tp0U", *b"Tp0X", *b"Tp0a", *b"Tp0d", *b"Tp0g", *b"Tp0j",
                        *b"Tp0m", *b"Tp0p", *b"Tp0u", *b"Tp0y",
                    ]
                }
            }
            _ => return None,
        };
        Some((keys, CpuTemperatureKind::CoreAverage))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_arch = "x86_64")]
    #[test]
    fn intel_fallback_excludes_unused_slots_and_unidentified_topologies() {
        let keys = intel_core_keys(6).unwrap();
        assert_eq!(keys.len(), 6);
        assert_eq!(keys.last(), Some(b"TC6C"));
        for key in [b"TC7C", b"TC8C", b"TC0P", b"TCGC"] {
            assert!(!keys.contains(key));
        }
        assert!(intel_core_keys(0).is_none());
        assert!(intel_core_keys(9).is_none());
        assert_eq!(cpu_keys(None).unwrap().1, CpuTemperatureKind::Package);
    }
    #[test]
    fn smc_abi_and_signed_decoding_are_checked() {
        assert_eq!(size_of::<Packet>(), 80);
        let mut bytes = [0; 32];
        bytes[..2].copy_from_slice(&(-128i16).to_be_bytes());
        assert_eq!(
            decode(
                KeyInfo {
                    kind: u32::from_be_bytes(*b"sp78"),
                    size: 2,
                    ..Default::default()
                },
                &bytes
            ),
            Some(-0.5)
        );
        bytes[..4].copy_from_slice(&48.5f32.to_le_bytes());
        assert_eq!(
            decode(
                KeyInfo {
                    kind: u32::from_be_bytes(*b"flt "),
                    size: 4,
                    ..Default::default()
                },
                &bytes
            ),
            Some(48.5)
        );
        assert!(decode(
            KeyInfo {
                kind: u32::from_be_bytes(*b"flt "),
                size: 32,
                ..Default::default()
            },
            &bytes
        )
        .is_none());
    }
    #[cfg(target_arch = "aarch64")]
    #[test]
    fn model_specific_keys_exclude_gpu_and_pmu_sensors() {
        let (keys, kind) = cpu_keys(Some("Apple M3 Max")).unwrap();
        assert_eq!(keys.len(), 16);
        assert_eq!(kind, CpuTemperatureKind::CoreAverage);
        assert!(!keys.contains(b"Tf14"));
        assert!(!keys.contains(b"Tp0P"));
        assert!(cpu_keys(Some("Apple M6")).is_none());
        assert!(cpu_keys(None).is_none());
        assert!(cpu_keys(Some("VirtualApple")).is_none());
    }
}
