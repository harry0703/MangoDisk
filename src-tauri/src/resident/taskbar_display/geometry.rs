//! Read-only shell inspection on a dedicated COM thread. A stalled accessibility
//! provider must never delay a click, the Tauri event loop, or resource sampling.
use super::{layout::Bounds, Service};
use std::{
    ptr,
    sync::{atomic::Ordering, Arc},
    time::{Duration, Instant},
};
use windows::Win32::{Foundation::HWND, System::Com::*, UI::Accessibility::*};
use windows_sys::{
    core::w,
    Win32::{
        Foundation::{POINT, RECT},
        Graphics::Gdi::{GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTOPRIMARY},
        UI::{
            HiDpi::{
                GetDpiForWindow, SetThreadDpiAwarenessContext,
                DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
            },
            WindowsAndMessaging::*,
        },
    },
};

#[derive(Clone)]
pub struct Geometry {
    pub sampled: Instant,
    pub shell: usize,
    pub bar: Bounds,
    pub occupied: Vec<Bounds>,
    pub dpi: u32,
    pub hidden: bool,
    pub environment: super::position::Environment,
}

pub fn start(service: Arc<Service>) {
    std::thread::spawn(move || unsafe {
        SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        if CoInitializeEx(None, COINIT_MULTITHREADED).is_err() {
            log::warn!("resident_taskbar_geometry_failed stage=com_initialize");
            return;
        }
        let automation: Result<IUIAutomation, _> =
            CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER);
        match automation {
            Ok(automation) => {
                let mut failed = false;
                loop {
                    let mut delay = Duration::from_secs(1);
                    if service.requested.load(Ordering::Relaxed) {
                        let started = Instant::now();
                        match inspect(&automation) {
                            Ok(geometry) => {
                                if geometry.hidden {
                                    // Hidden inspection reads only shell bounds. Poll cheaply
                                    // so revealing the taskbar does not wait a full sample period.
                                    delay = Duration::from_millis(200);
                                }
                                *service.geometry.lock().unwrap_or_else(|e| e.into_inner()) =
                                    Some(geometry);
                                if failed {
                                    log::info!("resident_taskbar_geometry_recovered");
                                }
                                failed = false;
                            }
                            Err(error) => {
                                if !failed {
                                    log::warn!(
                                        "resident_taskbar_geometry_failed stage=shell_controls code={}",
                                        error.code().0
                                    );
                                }
                                failed = true;
                            }
                        }
                        if started.elapsed() > Duration::from_millis(500) && !failed {
                            log::debug!(
                                "resident_taskbar_geometry_slow elapsed_ms={}",
                                started.elapsed().as_millis()
                            );
                        }
                    }
                    std::thread::sleep(delay);
                }
            }
            Err(_) => log::warn!("resident_taskbar_geometry_failed stage=automation_create"),
        }
        CoUninitialize();
    });
}

unsafe fn inspect(automation: &IUIAutomation) -> windows::core::Result<Geometry> {
    // Measure snapshot age from collection start, so a slow UIA query cannot
    // make old rectangles appear fresh merely because it completed recently.
    let sampled = Instant::now();
    let shell = FindWindowW(w!("Shell_TrayWnd"), ptr::null());
    if shell.is_null() {
        return Err(windows::core::Error::from_thread());
    }
    let mut bar = RECT::default();
    if GetWindowRect(shell, &mut bar) == 0 {
        return Err(windows::core::Error::from_thread());
    }
    let bounds = Bounds {
        left: bar.left,
        top: bar.top,
        right: bar.right,
        bottom: bar.bottom,
    };
    // Shell_TrayWnd is the primary taskbar. A hidden bar may extend onto an
    // adjacent display, so its offscreen rectangle must not choose that display.
    let monitor = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as _,
        ..Default::default()
    };
    if GetMonitorInfoW(monitor, &mut info) == 0 {
        return Err(windows::core::Error::from_thread());
    }
    let environment = super::position::read_environment();
    let hidden = !bounds.fits_in(Bounds {
        left: info.rcMonitor.left,
        top: info.rcMonitor.top,
        right: info.rcMonitor.right,
        bottom: info.rcMonitor.bottom,
    });
    if hidden {
        // Auto-hidden taskbars deliberately stop exposing usable controls. This
        // is a normal visibility state, not an accessibility failure or no-space fallback.
        return Ok(Geometry {
            sampled,
            shell: shell as usize,
            bar: bounds,
            occupied: Vec::new(),
            dpi: GetDpiForWindow(shell).max(96),
            hidden,
            environment,
        });
    }
    let root = automation.ElementFromHandle(HWND(shell))?;
    let cache = automation.CreateCacheRequest()?;
    cache.AddProperty(UIA_BoundingRectanglePropertyId)?;
    cache.AddProperty(UIA_ControlTypePropertyId)?;
    cache.AddProperty(UIA_IsOffscreenPropertyId)?;
    let elements = root.FindAllBuildCache(
        TreeScope_Descendants,
        &automation.CreateTrueCondition()?,
        &cache,
    )?;
    let mut occupied = Vec::new();
    for index in 0..elements.Length()?.min(2048) {
        let element = elements.GetElement(index)?;
        if element.CachedIsOffscreen()?.as_bool() {
            continue;
        }
        let kind = element.CachedControlType()?;
        if [
            UIA_ButtonControlTypeId,
            UIA_ListItemControlTypeId,
            UIA_MenuItemControlTypeId,
            UIA_EditControlTypeId,
            UIA_TextControlTypeId,
            UIA_CheckBoxControlTypeId,
            UIA_SliderControlTypeId,
            UIA_SplitButtonControlTypeId,
        ]
        .contains(&kind)
        {
            let r = element.CachedBoundingRectangle()?;
            occupied.push(Bounds {
                left: r.left,
                top: r.top,
                right: r.right,
                bottom: r.bottom,
            });
        }
    }
    // A partially initialized Explorer provider is not evidence of empty space.
    if occupied.is_empty() {
        return Err(windows::core::Error::from_hresult(windows::core::HRESULT(
            0x80004005u32 as i32,
        )));
    }
    // Notification containers include blank inter-icon spacing and the clock.
    let tray = FindWindowExW(shell, ptr::null_mut(), w!("TrayNotifyWnd"), ptr::null());
    if !tray.is_null() {
        let mut r = RECT::default();
        if GetWindowRect(tray, &mut r) != 0 {
            occupied.push(Bounds {
                left: r.left,
                top: r.top,
                right: r.right,
                bottom: r.bottom,
            });
        }
    }
    Ok(Geometry {
        sampled,
        shell: shell as usize,
        bar: Bounds {
            left: bar.left,
            top: bar.top,
            right: bar.right,
            bottom: bar.bottom,
        },
        occupied,
        hidden: false,
        environment,
        dpi: GetDpiForWindow(shell).max(96),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires an interactive Windows desktop with Explorer running"]
    fn inspect_live_taskbar_geometry() {
        unsafe {
            SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
            CoInitializeEx(None, COINIT_MULTITHREADED).unwrap();
            let automation: IUIAutomation =
                CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).unwrap();
            let geometry = inspect(&automation).unwrap();
            // Geometry only: never dump control names, window titles or tray tooltips.
            println!(
                "bar={:?} dpi={} occupied={:?}",
                geometry.bar, geometry.dpi, geometry.occupied
            );
            assert!(geometry.bar.width() > 0 && geometry.bar.height() > 0);
            drop(automation);
            CoUninitialize();
        }
    }
}
