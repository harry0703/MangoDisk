//! Enumerate active screens and bind only Explorer-owned taskbars to them.
use super::{layout::Bounds, monitors::Monitor};
use std::ptr;
use windows_sys::{
    core::w,
    Win32::{
        Foundation::*,
        Graphics::Gdi::*,
        UI::{Shell::*, WindowsAndMessaging::*},
    },
};

pub(super) struct Screen {
    pub info: Monitor,
    pub bounds: Bounds,
    pub shell: usize,
}

pub(super) fn enumerate() -> Result<Vec<Screen>, u32> {
    unsafe {
        let context = super::hosting::DpiContext::physical();
        let mut screens = Vec::<Screen>::new();
        if EnumDisplayMonitors(
            ptr::null_mut(),
            ptr::null(),
            Some(visit),
            (&mut screens as *mut Vec<Screen>) as isize,
        ) == 0
        {
            return Err(GetLastError());
        }
        drop(context);
        screens.sort_by(|a, b| {
            a.info
                .number
                .cmp(&b.info.number)
                .then(a.info.id.cmp(&b.info.id))
        });
        // Some virtual display drivers expose identical interface IDs. A fixed
        // choice must not silently resolve to the wrong member of such a set.
        let mut ids = std::collections::HashSet::new();
        if screens
            .iter()
            .any(|screen| !ids.insert(screen.info.id.clone()))
        {
            return Err(ERROR_DUP_NAME);
        }
        Ok(screens)
    }
}

unsafe extern "system" fn visit(monitor: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> i32 {
    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
    if GetMonitorInfoW(monitor, (&mut info as *mut MONITORINFOEXW).cast()) == 0 {
        return 0;
    }
    let device_name = string(&info.szDevice);
    let mut device = DISPLAY_DEVICEW {
        cb: std::mem::size_of::<DISPLAY_DEVICEW>() as u32,
        ..Default::default()
    };
    let found = EnumDisplayDevicesW(
        info.szDevice.as_ptr(),
        0,
        &mut device,
        EDD_GET_DEVICE_INTERFACE_NAME,
    ) != 0;
    let interface = if found {
        string(&device.DeviceID)
    } else {
        String::new()
    };
    let name = if found {
        string(&device.DeviceString)
    } else {
        String::new()
    };
    let bounds = info.monitorInfo.rcMonitor;
    let primary = info.monitorInfo.dwFlags & MONITORINFOF_PRIMARY != 0;
    let shell = taskbar(monitor, primary, bounds);
    let screen = Screen {
        info: Monitor {
            // RDP has no physical monitor interface on some Windows builds.
            // Its session-local display name is the only available identity.
            id: if interface.is_empty() {
                format!("session:{device_name}")
            } else {
                interface.to_ascii_lowercase()
            },
            name: if name.is_empty() {
                device_name.clone()
            } else {
                name
            },
            number: device_name
                .rsplit("DISPLAY")
                .next()
                .and_then(|number| number.parse().ok())
                .unwrap_or(unsafe { (*(data as *const Vec<Screen>)).len() as u32 + 1 }),
            primary,
            has_taskbar: !shell.is_null(),
            width: bounds.right - bounds.left,
            height: bounds.bottom - bounds.top,
        },
        bounds: Bounds {
            left: bounds.left,
            top: bounds.top,
            right: bounds.right,
            bottom: bounds.bottom,
        },
        shell: shell as usize,
    };
    (*(data as *mut Vec<Screen>)).push(screen);
    1
}

unsafe fn taskbar(monitor: HMONITOR, primary: bool, bounds: RECT) -> HWND {
    if primary {
        return FindWindowW(w!("Shell_TrayWnd"), ptr::null());
    }
    // An auto-hidden bar can extend into an adjacent monitor. Query its actual
    // appbar registration before using the visible window rectangle as evidence.
    for edge in [ABE_LEFT, ABE_TOP, ABE_RIGHT, ABE_BOTTOM] {
        let mut data = APPBARDATA {
            cbSize: std::mem::size_of::<APPBARDATA>() as u32,
            uEdge: edge,
            rc: bounds,
            ..Default::default()
        };
        let window = SHAppBarMessage(ABM_GETAUTOHIDEBAREX, &mut data) as HWND;
        if is_taskbar(window) {
            return window;
        }
    }
    let mut window = ptr::null_mut();
    loop {
        window = FindWindowExW(
            ptr::null_mut(),
            window,
            w!("Shell_SecondaryTrayWnd"),
            ptr::null(),
        );
        if window.is_null() {
            return window;
        }
        if is_taskbar(window) && MonitorFromWindow(window, MONITOR_DEFAULTTONULL) == monitor {
            return window;
        }
    }
}

pub(super) unsafe fn is_taskbar(window: HWND) -> bool {
    if window.is_null() || IsWindow(window) == 0 {
        return false;
    }
    let primary = FindWindowW(w!("Shell_TrayWnd"), ptr::null());
    if window == primary {
        return true;
    }
    let mut pid = 0;
    let mut primary_pid = 0;
    GetWindowThreadProcessId(window, &mut pid);
    GetWindowThreadProcessId(primary, &mut primary_pid);
    let mut class = [0u16; 64];
    let count = GetClassNameW(window, class.as_mut_ptr(), class.len() as i32);
    pid != 0 && pid == primary_pid && count > 0 && string(&class) == "Shell_SecondaryTrayWnd"
}

fn string(value: &[u16]) -> String {
    String::from_utf16_lossy(
        &value[..value
            .iter()
            .position(|unit| *unit == 0)
            .unwrap_or(value.len())],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn secondary_taskbar_class_requires_the_current_explorer_process() {
        unsafe {
            assert!(!is_taskbar(ptr::null_mut()));
            let class = WNDCLASSW {
                lpfnWndProc: Some(DefWindowProcW),
                lpszClassName: w!("Shell_SecondaryTrayWnd"),
                ..Default::default()
            };
            assert_ne!(RegisterClassW(&class), 0);
            let window = CreateWindowExW(
                0,
                class.lpszClassName,
                ptr::null(),
                WS_POPUP,
                0,
                0,
                100,
                40,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null(),
            );
            assert!(!window.is_null());
            assert!(!is_taskbar(window));
            DestroyWindow(window);
            UnregisterClassW(class.lpszClassName, ptr::null_mut());
        }
    }
}
