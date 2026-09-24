//! Whole-machine CPU sampling: native Windows percentages or cumulative Mach ticks.
//! Core validates observations and never publishes an unprimed interval as idle.

use crate::PlatformResult;
#[cfg(not(target_os = "linux"))]
use crate::{PlatformError, PlatformErrorCode};

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::CpuReader;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::CpuReader;

/// Native PDH percentages already represent an interval; never differentiate
/// them as cumulative ticks or publish its first, unprimed sample as zero.
#[derive(Debug, Clone, Copy)]
pub enum CpuSample {
    Counters(CpuCounters),
    Percent { used: f64, interval_ms: u64 },
    Baseline,
}

impl From<CpuCounters> for CpuSample {
    fn from(value: CpuCounters) -> Self {
        Self::Counters(value)
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
#[derive(Default)]
pub struct CpuReader;

#[cfg(not(any(windows, target_os = "linux")))]
impl CpuReader {
    pub fn read(&mut self) -> PlatformResult<CpuSample> {
        read().map(CpuSample::Counters)
    }

    pub fn reset(&mut self) {}
}

#[derive(Debug, Clone, Copy)]
pub struct CpuCounters {
    pub busy: u64,
    pub idle: u64,
}

#[cfg(target_os = "macos")]
pub fn read() -> PlatformResult<CpuCounters> {
    use std::mem::MaybeUninit;
    unsafe extern "C" {
        fn mach_host_self() -> libc::mach_port_t;
        fn mach_port_deallocate(
            task: libc::mach_port_t,
            name: libc::mach_port_t,
        ) -> libc::kern_return_t;
        static mach_task_self_: libc::mach_port_t;
    }
    // host_statistics returns aggregate ticks across processors, so the ratio
    // already has a whole-machine denominator rather than a single-core one.
    unsafe {
        let port = mach_host_self();
        let mut value = MaybeUninit::<libc::host_cpu_load_info>::zeroed();
        let mut count = libc::HOST_CPU_LOAD_INFO_COUNT;
        let result = libc::host_statistics(
            port,
            libc::HOST_CPU_LOAD_INFO,
            value.as_mut_ptr().cast(),
            &mut count,
        );
        mach_port_deallocate(mach_task_self_, port);
        if result != libc::KERN_SUCCESS || count != libc::HOST_CPU_LOAD_INFO_COUNT {
            return Err(unavailable());
        }
        let ticks = value.assume_init().cpu_ticks;
        Ok(CpuCounters {
            busy: u64::from(ticks[libc::CPU_STATE_USER as usize])
                + u64::from(ticks[libc::CPU_STATE_SYSTEM as usize])
                + u64::from(ticks[libc::CPU_STATE_NICE as usize]),
            idle: u64::from(ticks[libc::CPU_STATE_IDLE as usize]),
        })
    }
}

#[cfg(windows)]
pub fn read() -> PlatformResult<CpuCounters> {
    use windows_sys::Win32::{
        Foundation::FILETIME,
        System::Threading::{GetActiveProcessorGroupCount, GetSystemTimes},
    };
    fn ticks(value: FILETIME) -> u64 {
        (u64::from(value.dwHighDateTime) << 32) | u64::from(value.dwLowDateTime)
    }
    unsafe {
        // GetSystemTimes covers only the calling processor group. Until a
        // group-aware source is provided, never label a partial count as total CPU.
        if GetActiveProcessorGroupCount() > 1 {
            return Err(PlatformError::new(
                PlatformErrorCode::Unsupported,
                "multiple processor groups require a group-aware CPU source",
            ));
        }
        let mut idle = std::mem::zeroed();
        let mut kernel = std::mem::zeroed();
        let mut user = std::mem::zeroed();
        if GetSystemTimes(&mut idle, &mut kernel, &mut user) == 0 {
            return Err(unavailable());
        }
        // Windows kernel time includes idle time. Subtract it exactly once.
        let idle = ticks(idle);
        let busy = ticks(kernel)
            .checked_sub(idle)
            .and_then(|kernel| kernel.checked_add(ticks(user)))
            .ok_or_else(unavailable)?;
        Ok(CpuCounters { busy, idle })
    }
}

#[cfg(target_os = "linux")]
pub fn read() -> PlatformResult<CpuCounters> {
    linux::read()
}

#[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
pub fn read() -> PlatformResult<CpuCounters> {
    Err(unavailable())
}

#[cfg(not(target_os = "linux"))]
fn unavailable() -> PlatformError {
    PlatformError::new(
        PlatformErrorCode::OperationFailed,
        "cpu counters unavailable",
    )
}

#[cfg(test)]
mod tests {
    #[test]
    #[cfg(any(target_os = "macos", windows, target_os = "linux"))]
    fn native_cpu_counters_have_a_nonzero_machine_total() {
        #[cfg(windows)]
        if unsafe { windows_sys::Win32::System::Threading::GetActiveProcessorGroupCount() } > 1 {
            assert_eq!(
                super::read().unwrap_err().code(),
                crate::PlatformErrorCode::Unsupported
            );
            return;
        }
        let value = super::read().expect("native CPU counters must be readable");
        assert!(value.busy > 0 || value.idle > 0);
    }
}
