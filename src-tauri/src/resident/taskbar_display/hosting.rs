//! Child hosting and DPI conversion. Windows 10 space reservation is owned by
//! the separate companion lease, not by these window-parenting primitives.
use std::ptr;
use windows_sys::{
    core::w,
    Win32::{
        Foundation::*,
        System::Threading::*,
        UI::{HiDpi::*, WindowsAndMessaging::*},
    },
};

/// Child HWND hosting works across architectures, but an in-process XAML DLL
/// must match Explorer. Detect this before starting a companion that cannot attach.
pub unsafe fn xaml_architecture_matches(shell: HWND) -> bool {
    let mut shell_pid = 0;
    GetWindowThreadProcessId(shell, &mut shell_pid);
    let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, shell_pid);
    let result = if process.is_null() {
        Err(GetLastError())
    } else {
        let result = process_machine(GetCurrentProcess())
            .and_then(|app| process_machine(process).map(|shell| (app, shell)));
        CloseHandle(process);
        result
    };
    match result {
        Ok((app, shell)) => {
            let mode = if app == shell { "reserved" } else { "free_gap" };
            log::info!("resident_taskbar_architecture app_machine={app:#06x} shell_machine={shell:#06x} shell_pid={shell_pid} placement={mode}");
            app == shell
        }
        Err(code) => {
            log::warn!("resident_taskbar_architecture_unavailable shell_pid={shell_pid} code={code} fallback=free_gap");
            false
        }
    }
}

unsafe fn process_machine(process: HANDLE) -> Result<u16, u32> {
    // Windows 11's x64 emulation is not reported as WOW64. IsWow64Process2
    // can therefore report UNKNOWN plus native ARM64 for an x64 executable.
    // This information class reports the actual process architecture instead.
    let mut info = PROCESS_MACHINE_INFORMATION::default();
    if GetProcessInformation(
        process,
        ProcessMachineTypeInfo,
        (&mut info as *mut PROCESS_MACHINE_INFORMATION).cast(),
        std::mem::size_of_val(&info) as u32,
    ) == 0
    {
        return Err(GetLastError());
    }
    Ok(info.ProcessMachine)
}

pub unsafe fn parent(shell: HWND) -> HWND {
    // Windows 10's rebar shares clipping/order with the task buttons. Windows
    // 11 may retain a compatibility rebar limited to the centered icon area;
    // host under Shell_TrayWnd so its unused sides remain available.
    if super::position::read_environment() != super::position::Environment::Windows10 {
        return shell;
    }
    // Explorer creates the rebar after its top-level taskbar. Wait for that
    // host instead of permanently attaching to a transient startup hierarchy.
    FindWindowExW(shell, ptr::null_mut(), w!("ReBarWindow32"), ptr::null())
}

pub struct DpiContext(DPI_AWARENESS_CONTEXT);

impl DpiContext {
    pub unsafe fn enter(parent: HWND) -> Result<Self, (&'static str, u32)> {
        let context = GetWindowDpiAwarenessContext(parent);
        // All placement/model rectangles are physical pixels. Do not silently
        // embed in a DPI-virtualized replacement taskbar or reset process DPI.
        if GetAwarenessFromDpiAwarenessContext(context) != DPI_AWARENESS_PER_MONITOR_AWARE {
            return Err((
                "parent_dpi",
                GetAwarenessFromDpiAwarenessContext(context) as u32,
            ));
        }
        let previous = SetThreadDpiAwarenessContext(context);
        if previous.is_null() {
            Err(("thread_dpi", GetLastError()))
        } else {
            Ok(Self(previous))
        }
    }
}

impl Drop for DpiContext {
    fn drop(&mut self) {
        // Only the native hosting thread changes context. Tauri/WebView threads
        // retain their original DPI awareness throughout window creation.
        unsafe {
            SetThreadDpiAwarenessContext(self.0);
        }
    }
}

/// Keep opaque GDI painting composed above Explorer too. Switching from
/// SetLayeredWindowAttributes to UpdateLayeredWindow requires clearing the bit.
pub unsafe fn background(hwnd: HWND, enabled: bool) -> Result<(), u32> {
    let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
    for value in [
        style & !(WS_EX_LAYERED as isize),
        style | WS_EX_LAYERED as isize,
    ] {
        SetLastError(0);
        if SetWindowLongPtrW(hwnd, GWL_EXSTYLE, value) == 0 && GetLastError() != 0 {
            return Err(GetLastError());
        }
    }
    if enabled && SetLayeredWindowAttributes(hwnd, 0, 255, LWA_ALPHA) == 0 {
        return Err(GetLastError());
    }
    Ok(())
}

/// A rebar may be narrower than the taskbar. Never place pixels outside its
/// client area: the OS would clip them even if SetWindowPos reports success.
pub unsafe fn client_bounds(parent: HWND) -> Option<super::layout::Bounds> {
    let mut rect = RECT::default();
    let mut origin = POINT::default();
    if GetClientRect(parent, &mut rect) == 0
        || windows_sys::Win32::Graphics::Gdi::ClientToScreen(parent, &mut origin) == 0
    {
        return None;
    }
    Some(super::layout::Bounds {
        left: origin.x + rect.left,
        top: origin.y + rect.top,
        right: origin.x + rect.right,
        bottom: origin.y + rect.bottom,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_process_machine_reports_binary_architecture_even_under_emulation() {
        use windows_sys::Win32::System::SystemInformation::{
            IMAGE_FILE_MACHINE_AMD64, IMAGE_FILE_MACHINE_ARM64,
        };
        let expected = if cfg!(target_arch = "aarch64") {
            IMAGE_FILE_MACHINE_ARM64
        } else {
            IMAGE_FILE_MACHINE_AMD64
        };
        let actual = unsafe { process_machine(GetCurrentProcess()) };
        if super::super::position::read_environment()
            == super::super::position::Environment::Windows10
        {
            // The production caller only probes Windows 11 XAML taskbars.
            assert!(actual.is_err(), "Windows 10 has no ProcessMachineTypeInfo");
        } else {
            assert_eq!(actual, Ok(expected));
        }
    }

    #[test]
    fn child_hosting_preserves_parent_geometry_and_thread_dpi() {
        unsafe {
            // Exercise real Win32 state in hidden windows owned by this test;
            // never touch Explorer or depend on an interactive desktop. Layered
            // presentation is exercised using the manifest-bearing application
            // because Cargo library test executables do not embed that resource.
            let original = SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
            assert!(!original.is_null());
            let restore = DpiContext(original);
            let parent = CreateWindowExW(
                0,
                w!("STATIC"),
                ptr::null(),
                WS_POPUP,
                80,
                120,
                500,
                80,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null(),
            );
            assert!(!parent.is_null());
            let before = client_bounds(parent).unwrap();
            let entered = DpiContext::enter(parent).unwrap();
            let child = CreateWindowExW(
                WS_EX_NOACTIVATE,
                w!("STATIC"),
                ptr::null(),
                WS_CHILD | WS_CLIPSIBLINGS,
                4,
                4,
                100,
                40,
                parent,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null(),
            );
            assert!(
                !child.is_null(),
                "child creation failed: {}",
                GetLastError()
            );
            drop(entered);
            assert_ne!(
                AreDpiAwarenessContextsEqual(
                    GetThreadDpiAwarenessContext(),
                    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2
                ),
                0
            );
            assert_eq!(GetParent(child), parent);
            assert_eq!(
                GetWindowLongPtrW(child, GWL_EXSTYLE) & WS_EX_TOPMOST as isize,
                0
            );
            assert_eq!(client_bounds(parent), Some(before));
            DestroyWindow(child);
            assert_eq!(
                client_bounds(parent),
                Some(before),
                "removal must not leave reserved space"
            );
            DestroyWindow(parent);
            drop(restore);
        }
    }
}
