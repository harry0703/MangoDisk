use super::{CpuCounters, CpuSample};
use crate::{PlatformError, PlatformErrorCode, PlatformResult};

#[derive(Default)]
pub struct CpuReader;

impl CpuReader {
    pub fn read(&mut self) -> PlatformResult<CpuSample> {
        read().map(CpuSample::Counters)
    }

    pub fn reset(&mut self) {}
}

pub fn read() -> PlatformResult<CpuCounters> {
    let content = std::fs::read_to_string("/proc/stat").map_err(|error| {
        PlatformError::new(
            PlatformErrorCode::OperationFailed,
            format!("read /proc/stat failed: {error}"),
        )
    })?;
    let fields = content
        .lines()
        .find(|line| line.starts_with("cpu "))
        .ok_or_else(unavailable)?
        .split_whitespace()
        .skip(1)
        .map(str::parse::<u64>)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| unavailable())?;
    if fields.len() < 5 {
        return Err(unavailable());
    }
    let busy = fields[0]
        .saturating_add(fields[1])
        .saturating_add(fields[2])
        .saturating_add(*fields.get(5).unwrap_or(&0))
        .saturating_add(*fields.get(6).unwrap_or(&0))
        .saturating_add(*fields.get(7).unwrap_or(&0));
    Ok(CpuCounters {
        busy,
        idle: fields[3].saturating_add(fields[4]),
    })
}

fn unavailable() -> PlatformError {
    PlatformError::new(
        PlatformErrorCode::OperationFailed,
        "Linux CPU counters are unavailable",
    )
}
