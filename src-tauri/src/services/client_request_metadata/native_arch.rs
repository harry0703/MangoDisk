use windows_sys::{
    core::w,
    Win32::{
        Foundation::{GetLastError, HANDLE},
        System::{
            LibraryLoader::{GetModuleHandleW, GetProcAddress},
            SystemInformation::{IMAGE_FILE_MACHINE_AMD64, IMAGE_FILE_MACHINE_ARM64},
            Threading::GetCurrentProcess,
        },
    },
};

type IsWow64Process2Fn = unsafe extern "system" fn(HANDLE, *mut u16, *mut u16) -> i32;

pub(crate) fn telemetry_native_arch() -> Option<&'static str> {
    // Resolve at runtime so older Windows builds without this API still start the app.
    let module = unsafe { GetModuleHandleW(w!("kernel32.dll")) };
    let function = if module.is_null() {
        None
    } else {
        unsafe { GetProcAddress(module, c"IsWow64Process2".as_ptr().cast()) }
    };
    let Some(function) = function else {
        log::warn!(
            "client_metadata_native_arch_unavailable stage=resolve_api code={} outcome=header_omitted",
            unsafe { GetLastError() }
        );
        return None;
    };
    let query = unsafe {
        std::mem::transmute::<unsafe extern "system" fn() -> isize, IsWow64Process2Fn>(function)
    };
    let mut process_machine = 0;
    let mut native_machine = 0;
    // Query the host machine: the process machine can still be x86_64 under ARM emulation.
    if unsafe {
        query(
            GetCurrentProcess(),
            &mut process_machine,
            &mut native_machine,
        )
    } == 0
    {
        log::warn!(
            "client_metadata_native_arch_unavailable stage=is_wow64_process2 code={} outcome=header_omitted",
            unsafe { GetLastError() }
        );
        return None;
    }
    match native_machine {
        IMAGE_FILE_MACHINE_AMD64 => Some("x86_64"),
        IMAGE_FILE_MACHINE_ARM64 => Some("aarch64"),
        _ => {
            log::warn!(
                "client_metadata_native_arch_unavailable stage=map_machine native_machine={native_machine:#06x} outcome=header_omitted"
            );
            None
        }
    }
}
