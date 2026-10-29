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
        Foundation::RECT,
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
    pub requested_monitor: Option<String>,
    pub monitor_id: String,
    pub monitor_bounds: Bounds,
    pub fallback: super::monitors::Fallback,
    pub bar: Bounds,
    pub content: Option<Bounds>,
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
                let mut previous_target = None;
                loop {
                    let mut delay = Duration::from_secs(1);
                    if service.requested.load(Ordering::Relaxed) {
                        let started = Instant::now();
                        let requested = service
                            .model
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .monitor
                            .clone();
                        match inspect(&automation, requested.as_deref()) {
                            Ok(geometry) => {
                                let target = (
                                    requested.clone(),
                                    geometry.monitor_id.clone(),
                                    geometry.shell,
                                    geometry.fallback,
                                );
                                if previous_target.as_ref() != Some(&target) {
                                    log::info!("resident_taskbar_monitor requested={} resolved={} shell=0x{:x} bounds={:?} dpi={} fallback={:?}",
                                        mangodisk_platform::diagnostics::text(requested.as_deref().unwrap_or("primary")),
                                        mangodisk_platform::diagnostics::text(&geometry.monitor_id), geometry.shell,
                                        geometry.monitor_bounds, geometry.dpi, geometry.fallback);
                                    previous_target = Some(target);
                                }
                                if geometry.hidden {
                                    // Hidden inspection skips accessibility controls. Poll cheaply
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
                                        "resident_taskbar_geometry_failed stage=shell_controls requested={} code={}",
                                        mangodisk_platform::diagnostics::text(requested.as_deref().unwrap_or("primary")), error.code().0
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

unsafe fn inspect(
    automation: &IUIAutomation,
    requested: Option<&str>,
) -> windows::core::Result<Geometry> {
    // Measure snapshot age from collection start, so a slow UIA query cannot
    // make old rectangles appear fresh merely because it completed recently.
    let sampled = Instant::now();
    let screens = super::monitors_windows::enumerate().map_err(|code| {
        windows::core::Error::from_hresult(windows::core::HRESULT::from_win32(code))
    })?;
    let monitors: Vec<_> = screens.iter().map(|screen| screen.info.clone()).collect();
    let (index, fallback) = super::monitors::select(&monitors, requested).ok_or_else(|| {
        windows::core::Error::from_hresult(windows::core::HRESULT::from_win32(
            windows_sys::Win32::Foundation::ERROR_NOT_FOUND,
        ))
    })?;
    let screen = &screens[index];
    let shell = screen.shell as windows_sys::Win32::Foundation::HWND;
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
    let environment = super::position::read_environment();
    let hidden = !bounds.fits_in(screen.bounds);
    if hidden {
        // Auto-hidden taskbars deliberately stop exposing usable controls. This
        // is a normal visibility state, not an accessibility failure or no-space fallback.
        return Ok(Geometry {
            sampled,
            shell: shell as usize,
            requested_monitor: requested.map(str::to_owned),
            monitor_id: screen.info.id.clone(),
            monitor_bounds: screen.bounds,
            fallback,
            bar: bounds,
            content: None,
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
    let mut content = None;
    if !tray.is_null() {
        let mut r = RECT::default();
        if GetWindowRect(tray, &mut r) != 0 {
            let tray_bounds = Bounds {
                left: r.left,
                top: r.top,
                right: r.right,
                bottom: r.bottom,
            };
            occupied.push(tray_bounds);
            if tray_bounds.width() > 0 && tray_bounds.height() > 0 && tray_bounds.fits_in(bounds) {
                content = Some(tray_bounds);
            }
        }
    }
    Ok(Geometry {
        sampled,
        shell: shell as usize,
        requested_monitor: requested.map(str::to_owned),
        monitor_id: screen.info.id.clone(),
        monitor_bounds: screen.bounds,
        fallback,
        bar: Bounds {
            left: bar.left,
            top: bar.top,
            right: bar.right,
            bottom: bar.bottom,
        },
        occupied,
        content,
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
            let geometry = inspect(&automation, None).unwrap();
            // Geometry only: never dump control names, window titles or tray tooltips.
            println!(
                "bar={:?} content={:?} dpi={} occupied={:?}",
                geometry.bar, geometry.content, geometry.dpi, geometry.occupied
            );
            assert!(geometry.bar.width() > 0 && geometry.bar.height() > 0);
            drop(automation);
            CoUninitialize();
        }
    }
}
