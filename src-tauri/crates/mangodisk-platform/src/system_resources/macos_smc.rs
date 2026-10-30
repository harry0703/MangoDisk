//! Read-only Apple SMC transport shared by identified CPU, GPU, and battery sensors.
use crate::{PlatformError, PlatformErrorCode, PlatformResult};
use core_foundation::dictionary::CFMutableDictionaryRef;
use std::{ffi::c_void, mem::size_of};
fn unsupported() -> PlatformError {
    PlatformError::new(
        PlatformErrorCode::Unsupported,
        "Apple SMC sensors unavailable",
    )
}
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

/// Fixed, identified telemetry keys share the same read-only transport.
pub(crate) struct ValueReader {
    connection: Connection,
    metadata: std::cell::RefCell<std::collections::HashMap<u32, KeyInfo>>,
}
impl ValueReader {
    pub(crate) fn new() -> PlatformResult<Self> {
        Ok(Self {
            connection: Connection::open()?,
            metadata: Default::default(),
        })
    }
    pub(crate) fn read(&self, key: [u8; 4]) -> PlatformResult<Option<f64>> {
        let key = u32::from_be_bytes(key);
        let cached = self.metadata.borrow().get(&key).copied();
        let info = match cached {
            Some(info) => Some(info),
            None => {
                let info = self
                    .connection
                    .query(Packet {
                        key,
                        command: 9,
                        ..Default::default()
                    })?
                    .map(|packet| packet.info);
                // Retry absent keys: a missing response during wake is not a permanent capability result.
                if let Some(info) = info {
                    self.metadata.borrow_mut().insert(key, info);
                }
                info
            }
        };
        let Some(info) = info else {
            return Ok(None);
        };
        if !supported_encoding(info) {
            return Err(encoding_error(key, info));
        }
        let Some(value) = self.connection.query(Packet {
            key,
            info,
            command: 5,
            ..Default::default()
        })?
        else {
            return Ok(None);
        };
        Ok(decode(info, &value.bytes))
    }
}

pub(crate) struct TemperatureSensors {
    connection: Connection,
    discovery: SensorDiscovery,
}
impl TemperatureSensors {
    pub(crate) fn new(keys: &[[u8; 4]]) -> PlatformResult<Self> {
        let connection = Connection::open()?;
        let sensors = discover_with(keys, |key| temperature_metadata(&connection, key))?;
        Ok(Self {
            connection,
            discovery: SensorDiscovery {
                sensors,
                ..Default::default()
            },
        })
    }
    pub(crate) fn new_cpu(keys: &[[u8; 4]]) -> PlatformResult<Self> {
        let connection = Connection::open()?;
        let discovery = discover_cpu_with(keys, |key| temperature_metadata(&connection, key))?;
        Ok(Self {
            connection,
            discovery,
        })
    }
    #[cfg(target_arch = "x86_64")]
    pub(crate) fn select(&mut self, keys: &[[u8; 4]]) -> PlatformResult<()> {
        self.discovery =
            discover_cpu_with(keys, |key| temperature_metadata(&self.connection, key))?;
        Ok(())
    }
    pub(crate) fn len(&self) -> usize {
        self.discovery.sensors.len()
    }
    #[cfg(target_arch = "x86_64")]
    pub(crate) fn has_valid_readings(&mut self) -> PlatformResult<bool> {
        let samples = self.read_available();
        if !samples.values.is_empty() {
            return Ok(true);
        }
        if let Some((_, error)) = samples
            .failures
            .into_iter()
            .find(|(_, error)| error.code() != PlatformErrorCode::InvalidData)
        {
            return Err(error);
        }
        Ok(false)
    }
    fn read_sensor(&self, key: u32, info: KeyInfo) -> PlatformResult<Option<f64>> {
        Ok(self
            .read_raw_sensor(key, info)?
            .filter(|value| value.is_finite() && *value > 0.0 && *value <= 150.0))
    }
    fn read_raw_sensor(&self, key: u32, info: KeyInfo) -> PlatformResult<Option<f64>> {
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
                    format!("SMC sensor disappeared key={key:08x}"),
                )
            })?;
        Ok(decode(info, &packet.bytes))
    }
    pub(crate) fn read_available(&mut self) -> super::temperature_samples::TemperatureSamples {
        self.discovery
            .retry_with(|key| temperature_metadata(&self.connection, key));
        let mut samples = super::temperature_samples::TemperatureSamples::collect(
            self.discovery
                .sensors
                .iter()
                .map(|&(key, info)| (sensor_name(key), self.read_raw_sensor(key, info))),
        );
        samples.failures.extend(
            self.discovery
                .unavailable
                .iter()
                .map(|(key, error)| (sensor_name(*key), error.clone())),
        );
        samples
    }
    pub(crate) fn read(&self) -> PlatformResult<Vec<f64>> {
        // GPU readings keep a fixed denominator; CPU explicitly uses read_available.
        self.discovery
            .sensors
            .iter()
            .map(|&(key, info)| {
                self.read_sensor(key, info)?.ok_or_else(|| {
                    PlatformError::new(
                        PlatformErrorCode::InvalidData,
                        format!("SMC temperature invalid key={key:08x}"),
                    )
                })
            })
            .collect()
    }
}
fn sensor_name(key: u32) -> String {
    String::from_utf8_lossy(&key.to_be_bytes()).into_owned()
}
fn temperature_metadata(connection: &Connection, key: u32) -> PlatformResult<Option<KeyInfo>> {
    connection
        .query(Packet {
            key,
            command: 9,
            ..Default::default()
        })
        .map(|packet| packet.map(|packet| packet.info))
        .map_err(|error| {
            PlatformError::new(error.code(), format!("SMC metadata key={key:08x}: {error}"))
        })
}
fn discover_with(
    keys: &[[u8; 4]],
    mut query: impl FnMut(u32) -> PlatformResult<Option<KeyInfo>>,
) -> PlatformResult<Vec<(u32, KeyInfo)>> {
    let mut sensors = Vec::new();
    for key in keys {
        let key = u32::from_be_bytes(*key);
        let Some(info) = query(key)? else {
            continue;
        };
        if temperature_encoding(info) {
            sensors.push((key, info));
        }
    }
    Ok(sensors)
}

#[derive(Default)]
struct SensorDiscovery {
    sensors: Vec<(u32, KeyInfo)>,
    unavailable: Vec<(u32, PlatformError)>,
}
impl SensorDiscovery {
    fn retry_with(&mut self, mut query: impl FnMut(u32) -> PlatformResult<Option<KeyInfo>>) {
        // Only failed metadata is retried. Absent model variants do not add
        // recurring native calls; recovered keys join the next CPU observation.
        let mut pending = Vec::new();
        for (key, _) in self.unavailable.drain(..) {
            match query(key) {
                Ok(Some(info)) if temperature_encoding(info) => self.sensors.push((key, info)),
                Ok(Some(_)) => {}
                Ok(None) => pending.push((
                    key,
                    PlatformError::new(
                        PlatformErrorCode::OperationFailed,
                        format!("SMC metadata disappeared key={key:08x}"),
                    ),
                )),
                Err(error) => pending.push((key, error)),
            }
        }
        self.unavailable = pending;
    }
}
fn discover_cpu_with(
    keys: &[[u8; 4]],
    mut query: impl FnMut(u32) -> PlatformResult<Option<KeyInfo>>,
) -> PlatformResult<SensorDiscovery> {
    let mut discovery = SensorDiscovery::default();
    for key in keys {
        let key = u32::from_be_bytes(*key);
        match query(key) {
            Ok(Some(info)) if temperature_encoding(info) => discovery.sensors.push((key, info)),
            Ok(_) => {}
            Err(error) => discovery.unavailable.push((key, error)),
        }
    }
    if discovery.sensors.is_empty() && !discovery.unavailable.is_empty() {
        super::temperature_samples::TemperatureSamples::collect(
            discovery
                .unavailable
                .iter()
                .map(|(key, error)| (sensor_name(*key), Err(error.clone()))),
        )
        .into_values()?;
    }
    Ok(discovery)
}

fn supported_encoding(info: KeyInfo) -> bool {
    temperature_encoding(info)
        || (info.kind == u32::from_be_bytes(*b"sp96") && info.size == 2)
        || (info.kind == u32::from_be_bytes(*b"sp87") && info.size == 2)
        || (info.kind == u32::from_be_bytes(*b"fpe2") && info.size == 2)
        || (info.kind == u32::from_be_bytes(*b"ui8 ") && info.size == 1)
}
fn temperature_encoding(info: KeyInfo) -> bool {
    (info.kind == u32::from_be_bytes(*b"flt ") && info.size == 4)
        || (info.kind == u32::from_be_bytes(*b"sp78") && info.size == 2)
}
fn encoding_error(key: u32, info: KeyInfo) -> PlatformError {
    PlatformError::new(
        PlatformErrorCode::Unsupported,
        format!(
            "SMC encoding unsupported sensor={} encoding={} size={}",
            crate::diagnostics::text(&String::from_utf8_lossy(&key.to_be_bytes())),
            crate::diagnostics::text(&String::from_utf8_lossy(&info.kind.to_be_bytes())),
            info.size
        ),
    )
}
fn decode(info: KeyInfo, bytes: &[u8; 32]) -> Option<f64> {
    if !supported_encoding(info) {
        return None;
    }
    if info.kind == u32::from_be_bytes(*b"ui8 ") {
        Some(f64::from(bytes[0]))
    } else if info.kind == u32::from_be_bytes(*b"fpe2") {
        Some(f64::from(u16::from_be_bytes(bytes[..2].try_into().ok()?)) / 4.0)
    } else if info.size == 4 {
        Some(f64::from(f32::from_le_bytes(bytes[..4].try_into().ok()?)))
    } else {
        let divisor = if info.kind == u32::from_be_bytes(*b"sp96") {
            64.0
        } else if info.kind == u32::from_be_bytes(*b"sp87") {
            128.0
        } else {
            256.0
        };
        Some(f64::from(i16::from_be_bytes(bytes[..2].try_into().ok()?)) / divisor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn one_cpu_metadata_failure_keeps_other_identified_temperature_sensors() {
        let discovery = discover_cpu_with(&[*b"Tp0Y", *b"Te05"], |key| {
            if key == u32::from_be_bytes(*b"Tp0Y") {
                Err(PlatformError::new(
                    PlatformErrorCode::OperationFailed,
                    "SMC key metadata unavailable",
                ))
            } else {
                Ok(Some(KeyInfo {
                    size: 4,
                    kind: u32::from_be_bytes(*b"flt "),
                    ..Default::default()
                }))
            }
        })
        .unwrap();
        assert_eq!(discovery.sensors.len(), 1);
        assert_eq!(discovery.unavailable.len(), 1);
    }
    fn float_metadata() -> KeyInfo {
        KeyInfo {
            size: 4,
            kind: u32::from_be_bytes(*b"flt "),
            ..Default::default()
        }
    }
    #[test]
    fn failed_cpu_metadata_retries_until_recovery_without_duplicate_sensors() {
        let failed = u32::from_be_bytes(*b"Tp0Y");
        let mut discovery = discover_cpu_with(&[*b"Tp0Y", *b"Te05"], |key| {
            if key == failed {
                Err(PlatformError::operation_failed("metadata native_code=1"))
            } else {
                Ok(Some(float_metadata()))
            }
        })
        .unwrap();
        discovery.retry_with(|key| {
            assert_eq!(key, failed);
            Ok(None)
        });
        assert_eq!(discovery.sensors.len(), 1);
        assert_eq!(discovery.unavailable.len(), 1);
        discovery.retry_with(|key| {
            assert_eq!(key, failed);
            Ok(Some(float_metadata()))
        });
        assert_eq!(discovery.sensors.len(), 2);
        assert!(discovery.unavailable.is_empty());
        discovery.retry_with(|_| panic!("recovered metadata must not be queried again"));
        assert_eq!(discovery.sensors.len(), 2);
    }
    #[test]
    fn absent_or_unsupported_cpu_keys_are_not_retried_and_all_native_errors_propagate() {
        let discovery = discover_cpu_with(&[*b"Tp0Y", *b"Te05"], |key| {
            if key == u32::from_be_bytes(*b"Tp0Y") {
                Ok(None)
            } else {
                Ok(Some(KeyInfo {
                    size: 1,
                    kind: u32::from_be_bytes(*b"ui8 "),
                    ..Default::default()
                }))
            }
        })
        .unwrap();
        assert!(discovery.sensors.is_empty() && discovery.unavailable.is_empty());
        let error = discover_cpu_with(&[*b"Tp0Y"], |_| {
            Err(PlatformError::new(
                PlatformErrorCode::AccessDenied,
                "metadata denied native_code=1234",
            ))
        })
        .err()
        .unwrap();
        assert_eq!(error.code(), PlatformErrorCode::AccessDenied);
        assert!(
            error.diagnostic().contains("Tp0Y") && error.diagnostic().contains("native_code=1234")
        );
        assert!(
            discover_with(&[*b"Tp0Y", *b"Te05"], |key| {
                if key == u32::from_be_bytes(*b"Tp0Y") {
                    Ok(Some(float_metadata()))
                } else {
                    Err(PlatformError::operation_failed("GPU metadata failure"))
                }
            })
            .is_err(),
            "GPU discovery must keep its strict semantics"
        );
    }
    #[test]
    #[ignore = "requires an Apple M3 Mac with readable Te05 and Te0L CPU sensors"]
    fn native_cpu_metadata_failure_recovers_on_the_same_connection() {
        let connection = Connection::open().unwrap();
        let discovery = discover_cpu_with(&[*b"Te05", *b"Te0L"], |key| {
            if key == u32::from_be_bytes(*b"Te05") {
                Err(PlatformError::operation_failed("injected metadata failure"))
            } else {
                temperature_metadata(&connection, key)
            }
        })
        .unwrap();
        assert_eq!(discovery.sensors.len(), 1);
        assert_eq!(discovery.unavailable.len(), 1);
        let mut sensors = TemperatureSensors {
            connection,
            discovery,
        };
        let (key, info) = sensors.discovery.sensors[0];
        let initial = sensors.read_raw_sensor(key, info).unwrap().unwrap();
        assert!(initial.is_finite() && initial > 0.0 && initial <= 150.0);
        println!("native_cpu_metadata_partial valid=1 invalid=1 celsius={initial}");
        let samples = sensors.read_available();
        assert_eq!(samples.values.len(), 2);
        assert!(samples.failures.is_empty());
        assert!(sensors.discovery.unavailable.is_empty());
        println!(
            "native_cpu_metadata_recovered valid=2 values={:?}",
            samples.values
        );
    }
    #[test]
    fn macbook_package_power_sp87_preserves_fraction_sign_and_bounds() {
        let info = KeyInfo {
            kind: u32::from_be_bytes(*b"sp87"),
            size: 2,
            ..Default::default()
        };
        let mut bytes = [0; 32];
        for (raw, expected) in [
            (0i16, 0.0),
            (1281, 10.0078125),
            (-128, -1.0),
            (i16::MAX, 255.9921875),
            (i16::MIN, -256.0),
        ] {
            bytes[..2].copy_from_slice(&raw.to_be_bytes());
            assert_eq!(decode(info, &bytes), Some(expected));
        }
        assert!(!temperature_encoding(info));
        assert!(decode(KeyInfo { size: 4, ..info }, &bytes).is_none());
    }
    #[test]
    fn fan_count_and_unsigned_fixed_point_preserve_zero_and_fractional_rpm() {
        let mut bytes = [0; 32];
        let count = KeyInfo {
            kind: u32::from_be_bytes(*b"ui8 "),
            size: 1,
            ..Default::default()
        };
        assert_eq!(decode(count, &bytes), Some(0.0));
        bytes[0] = 2;
        assert_eq!(decode(count, &bytes), Some(2.0));
        assert!(decode(KeyInfo { size: 2, ..count }, &bytes).is_none());
        let speed = KeyInfo {
            kind: u32::from_be_bytes(*b"fpe2"),
            size: 2,
            ..Default::default()
        };
        for (raw, rpm) in [(0u16, 0.0), (5401, 1350.25), (u16::MAX, 16383.75)] {
            bytes[..2].copy_from_slice(&raw.to_be_bytes());
            assert_eq!(decode(speed, &bytes), Some(rpm));
        }
        assert!(!temperature_encoding(speed));
        assert!(decode(KeyInfo { size: 4, ..speed }, &bytes).is_none());
    }

    #[test]
    fn intel_power_fixed_point_preserves_fraction_and_sign() {
        let info = KeyInfo {
            kind: u32::from_be_bytes(*b"sp96"),
            size: 2,
            ..Default::default()
        };
        let mut bytes = [0; 32];
        for (encoded, expected) in [(0x176d, 93.703125), (-32, -0.5), (0, 0.0)] {
            bytes[..2].copy_from_slice(&(encoded as i16).to_be_bytes());
            assert_eq!(decode(info, &bytes), Some(expected));
        }
        assert!(!temperature_encoding(info));
        assert!(decode(KeyInfo { size: 4, ..info }, &bytes).is_none());
    }
    #[test]
    fn unsupported_encoding_diagnostics_do_not_claim_the_sensor_is_absent() {
        let error = encoding_error(
            u32::from_be_bytes(*b"PSTR"),
            KeyInfo {
                kind: u32::from_be_bytes(*b"fpe2"),
                size: 2,
                ..Default::default()
            },
        );
        assert_eq!(error.code(), PlatformErrorCode::Unsupported);
        assert!(error
            .to_string()
            .contains("sensor=\"PSTR\" encoding=\"fpe2\" size=2"));
        assert!(!error.to_string().contains("absent"));
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
}
