use super::{unsupported, CpuTemperature, CpuTemperatureKind, CpuTemperatureSource};
use crate::{PlatformError, PlatformErrorCode, PlatformResult};
use mangodisk_privileged::{
    cpu_temperature::CpuTemperatureSensor,
    windows::{client, deployment},
};
use mangodisk_privileged_protocol::{CapabilityStatus, Operation, Reply};

pub(super) struct Reader {
    source: CpuTemperatureSource,
}
impl Reader {
    pub(super) fn new() -> PlatformResult<Self> {
        let source = match deployment::temperature_sensor().ok_or_else(unsupported)? {
            CpuTemperatureSensor::IntelPackageDts => CpuTemperatureSource::IntelDtsPawnIo,
            CpuTemperatureSensor::AmdZen { .. } => CpuTemperatureSource::AmdZenPawnIo,
        };
        Ok(Self { source })
    }
    pub(super) fn read(&mut self) -> PlatformResult<CpuTemperature> {
        let response = client::request(Operation::CpuTemperature).map_err(|error| {
            PlatformError::new(
                PlatformErrorCode::Io,
                format!("privileged CPU temperature unavailable: {error}"),
            )
        })?;
        match response.reply {
            Reply::CpuTemperature {
                status: CapabilityStatus::Ready,
                reading: Some(reading),
            } => {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64;
                reading.validate(now).map_err(|error| {
                    PlatformError::new(PlatformErrorCode::InvalidData, error.to_string())
                })?;
                Ok(CpuTemperature {
                    celsius: reading.celsius,
                    kind: CpuTemperatureKind::Package,
                    source: self.source,
                    sensor_count: 1,
                    sampled_at_ms: Some(reading.sampled_at_ms),
                })
            }
            Reply::CpuTemperature {
                status: CapabilityStatus::UnsupportedHardware,
                ..
            } => Err(unsupported()),
            Reply::CpuTemperature {
                status: CapabilityStatus::DriverAccessDenied,
                ..
            } => Err(PlatformError::new(
                PlatformErrorCode::AccessDenied,
                "privileged CPU driver access denied",
            )),
            Reply::CpuTemperature {
                status: CapabilityStatus::DriverMissing,
                ..
            } => Err(PlatformError::new(
                PlatformErrorCode::Io,
                "privileged CPU driver unavailable",
            )
            .with_failure_reason(crate::PlatformFailureReason::DependencyUnavailable)),
            Reply::CpuTemperature {
                status: CapabilityStatus::SensorUnavailable,
                ..
            } => Err(PlatformError::new(
                PlatformErrorCode::OperationFailed,
                "privileged CPU sensor temporarily unavailable",
            )),
            _ => Err(PlatformError::new(
                PlatformErrorCode::InvalidData,
                "unexpected CPU temperature reply",
            )),
        }
    }
}
