//! Child hosting and DPI conversion. Windows 10 space reservation is owned by
//! the separate companion lease, not by these window-parenting primitives.
use std::ptr;
use windows_sys::{
    core::w,
    Win32::{
        Foundation::*,
        Graphics::Gdi::*,
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
    let rebar = FindWindowExW(shell, ptr::null_mut(), w!("ReBarWindow32"), ptr::null());
    if rebar.is_null()
        && super::monitors_windows::is_taskbar(shell)
        && shell != FindWindowW(w!("Shell_TrayWnd"), ptr::null())
    {
        shell
    } else {
        rebar
    }
}

/// Identify Explorer's Windows 10 pagination container and inner button list.
/// Reservation resizes the container so Explorer can update overflow controls;
/// unsupported hierarchies retain the read-only gap path.
pub unsafe fn task_list(rebar: HWND) -> Option<(HWND, HWND)> {
    if rebar.is_null() {
        return None;
    }
    let container = FindWindowExW(rebar, ptr::null_mut(), w!("MSTaskSwWClass"), ptr::null());
    if container.is_null() {
        return None;
    }
    let list = FindWindowExW(
        container,
        ptr::null_mut(),
        w!("MSTaskListWClass"),
        ptr::null(),
    );
    (!list.is_null()).then_some((container, list))
}

pub struct DpiContext(DPI_AWARENESS_CONTEXT);

impl DpiContext {
    pub unsafe fn physical() -> Self {
        Self(SetThreadDpiAwarenessContext(
            DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        ))
    }
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
        // Restore the caller's awareness after native hosting or discovery.
        // Other threads retain their own DPI context throughout this operation.
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

/// Explorer can clip an expanded taskbar without changing either HWND's bounds.
/// Window regions use the window origin, which can differ from its client origin.
pub unsafe fn visible_bounds(
    window: HWND,
    client: super::layout::Bounds,
) -> Option<super::layout::Bounds> {
    let mut region = RECT::default();
    match GetWindowRgnBox(window, &mut region) {
        0 => Some(client), // A window without an explicit region uses its full bounds.
        NULLREGION => None,
        _ => {
            let mut origin = RECT::default();
            if GetWindowRect(window, &mut origin) == 0 {
                return None;
            }
            client.intersection(super::layout::Bounds {
                left: origin.left + region.left,
                top: origin.top + region.top,
                right: origin.left + region.right,
                bottom: origin.top + region.bottom,
            })
        }
    }
}

/// A complex region's bounding box can contain holes between floating taskbar
/// sections. Require the entire strip to survive clipping before displaying it.
pub unsafe fn region_contains(window: HWND, bounds: super::layout::Bounds) -> bool {
    let mut box_rect = RECT::default();
    let kind = GetWindowRgnBox(window, &mut box_rect);
    match kind {
        0 => return true,
        NULLREGION => return false,
        _ => {}
    }
    let mut origin = RECT::default();
    if GetWindowRect(window, &mut origin) == 0 {
        return false;
    }
    if kind == SIMPLEREGION {
        return bounds.fits_in(super::layout::Bounds {
            left: origin.left + box_rect.left,
            top: origin.top + box_rect.top,
            right: origin.left + box_rect.right,
            bottom: origin.top + box_rect.bottom,
        });
    }
    let region = CreateRectRgn(0, 0, 0, 0);
    let remaining = CreateRectRgn(
        bounds.left - origin.left,
        bounds.top - origin.top,
        bounds.right - origin.left,
        bounds.bottom - origin.top,
    );
    let contained = !region.is_null()
        && !remaining.is_null()
        && GetWindowRgn(window, region) != 0
        && CombineRgn(remaining, remaining, region, RGN_DIFF) == NULLREGION;
    if !region.is_null() {
        DeleteObject(region);
    }
    if !remaining.is_null() {
        DeleteObject(remaining);
    }
    contained
}

/// A lease can move task buttons anywhere along the host's original long axis.
/// Preserve cross-axis clipping, but use gaps when that entire lane is not visible.
pub unsafe fn reservation_visible(
    shell: HWND,
    parent: HWND,
    host: super::layout::Bounds,
    content: super::layout::Bounds,
) -> bool {
    let lane = super::layout::content_band(host, Some(content));
    region_contains(shell, lane) && (parent == shell || region_contains(parent, lane))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_regions_clip_at_the_window_origin_and_reject_complex_holes() {
        unsafe {
            let original = SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
            let restore = DpiContext(original);
            let window = CreateWindowExW(
                0,
                w!("STATIC"),
                ptr::null(),
                WS_POPUP | WS_BORDER,
                80,
                120,
                500,
                80,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null(),
            );
            assert!(!window.is_null());
            let client = client_bounds(window).unwrap();
            assert_eq!(visible_bounds(window, client), Some(client));
            let lower_band = CreateRectRgn(0, 24, 500, 80);
            assert_ne!(SetWindowRgn(window, lower_band, 0), 0);
            let visible = visible_bounds(window, client).unwrap();
            assert_eq!(visible.top, 144, "region coordinates use the window origin");
            assert_eq!(visible.left, client.left);
            assert!(region_contains(window, visible));
            assert!(!region_contains(window, client));
            assert!(reservation_visible(window, window, client, visible));

            assert_ne!(SetWindowRgn(window, CreateRectRgn(40, 24, 460, 80), 0), 0);
            let inset = visible_bounds(window, client).unwrap();
            assert!(region_contains(window, inset));
            assert!(
                !reservation_visible(window, window, client, inset),
                "a long-axis inset cannot authorize a lease in the original host"
            );

            let islands = CreateRectRgn(0, 24, 160, 80);
            let right = CreateRectRgn(300, 24, 500, 80);
            assert_eq!(CombineRgn(islands, islands, right, RGN_OR), COMPLEXREGION);
            DeleteObject(right);
            assert_ne!(SetWindowRgn(window, islands, 0), 0);
            assert_eq!(visible_bounds(window, client), Some(visible));
            assert!(!reservation_visible(window, window, client, visible));
            assert!(
                !region_contains(window, visible),
                "the bounding box includes a hole"
            );
            assert!(region_contains(
                window,
                super::super::layout::Bounds {
                    right: 230,
                    ..visible
                }
            ));
            assert!(!region_contains(
                window,
                super::super::layout::Bounds {
                    left: 250,
                    right: 350,
                    ..visible
                }
            ));

            assert_ne!(SetWindowRgn(window, CreateRectRgn(0, 0, 0, 0), 0), 0);
            assert!(visible_bounds(window, client).is_none());
            assert!(!region_contains(window, visible));
            assert_ne!(SetWindowRgn(window, ptr::null_mut(), 0), 0);
            assert_eq!(visible_bounds(window, client), Some(client));
            assert!(region_contains(window, client));
            assert!(reservation_visible(window, window, client, client));
            DestroyWindow(window);
            drop(restore);
        }
    }

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
