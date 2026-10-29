//! The window and its GDI objects stay on one native thread. Other threads only
//! replace a bounded model and post a wake message; callbacks never create WebViews.
mod foreground;
mod interaction;
mod messages;
mod rendering;
mod update;

use super::{
    directwrite,
    layout::Bounds,
    position::{Edge, Environment},
    presentation::Column,
    shell_events,
    surface::Surface,
    visibility::Visibility,
    DisplayStatus, Service,
};
use messages::procedure;
use std::{
    cell::RefCell,
    ptr,
    sync::{atomic::Ordering, Arc},
    time::Duration,
};
use windows_sys::{
    core::w,
    Win32::{
        Foundation::*,
        UI::{HiDpi::*, WindowsAndMessaging::*},
    },
};

pub(super) const UPDATE: u32 = WM_APP + 71;

struct Window {
    service: Arc<Service>,
    columns: Vec<Column>,
    bounds: Bounds,
    surface: Surface,
    dpi: u32,
    color: [u8; 3],
    hover: Option<usize>,
    visible: bool,
    parent: HWND,
    shell: HWND,
    visibility: Option<Visibility>,
    background: bool,
    paint_failed: bool,
    text_renderer: Option<directwrite::Renderer>,
    position_failed: bool,
    reservation: Option<super::reservation::Client>,
    releasing: Option<super::reservation::Release>,
    xaml_architecture_matches: Option<bool>,
    geometry_delayed: bool,
    reservation_failed: Option<super::reservation::Failure>,
    reservation_retry: std::time::Instant,
    placement_policy: Option<(
        crate::resident::preference_schema::TaskbarPosition,
        Environment,
        Edge,
        bool,
    )>,
    foreground: foreground::Identity,
    fullscreen_candidate: Option<foreground::FullscreenCandidate>,
    appearance_checked: std::time::Instant,
    layout_checked: std::time::Instant,
    layout_inputs: Option<(std::time::Instant, Bounds, Bounds, super::peers::Peers)>,
}

pub(super) fn start(service: Arc<Service>) {
    std::thread::spawn(move || unsafe {
        // This affects only our native thread, never Explorer or Tauri's process DPI.
        SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let class = WNDCLASSW {
            lpfnWndProc: Some(procedure),
            lpszClassName: w!("MangoDiskTaskbarStatus"),
            hCursor: LoadCursorW(ptr::null_mut(), IDC_HAND),
            ..Default::default()
        };
        if RegisterClassW(&class) == 0 {
            log::warn!(
                "resident_taskbar_native_failed stage=register_class code={}",
                GetLastError()
            );
            service.publish(DisplayStatus::ShellUnavailable);
            return;
        }
        let mut host_failure = None;
        let mut pending_release = None;
        loop {
            clear_stale_quit();
            let requested_monitor = service
                .model
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .monitor
                .clone();
            let shell = service
                .geometry
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .as_ref()
                .filter(|geometry| geometry.requested_monitor == requested_monitor)
                .map_or(ptr::null_mut(), |geometry| geometry.shell as HWND);
            if shell.is_null() {
                service.publish(DisplayStatus::ShellUnavailable);
                std::thread::sleep(Duration::from_millis(200));
                continue;
            }
            // Only the monitor HWND is embedded. On Windows 10 the companion
            // owns space allocation/restoration separately from this UI thread.
            let parent = super::hosting::parent(shell);
            if parent.is_null() {
                service.publish(DisplayStatus::ShellUnavailable);
                std::thread::sleep(Duration::from_millis(200));
                continue;
            }
            let dpi_context = match super::hosting::DpiContext::enter(parent) {
                Ok(context) => context,
                Err((stage, value)) => {
                    let failure = (parent as usize, stage, value);
                    if host_failure != Some(failure) {
                        log::warn!("resident_taskbar_embedding_failed stage={stage} value={value} parent={parent:?} fallback=tray");
                        host_failure = Some(failure);
                    }
                    service.publish(DisplayStatus::ShellUnavailable);
                    std::thread::sleep(Duration::from_secs(2));
                    continue;
                }
            };
            let mut data = Box::new(RefCell::new(Window {
                service: service.clone(),
                columns: Vec::new(),
                bounds: Bounds::default(),
                surface: Surface::default(),
                dpi: 96,
                color: [0; 3],
                hover: None,
                visible: false,
                parent,
                shell,
                visibility: None,
                background: true,
                paint_failed: false,
                text_renderer: None,
                position_failed: false,
                reservation: None,
                releasing: None,
                xaml_architecture_matches: None,
                geometry_delayed: false,
                reservation_failed: None,
                reservation_retry: std::time::Instant::now(),
                placement_policy: None,
                foreground: foreground::Identity::default(),
                fullscreen_candidate: None,
                appearance_checked: std::time::Instant::now() - Duration::from_secs(2),
                layout_checked: std::time::Instant::now() - Duration::from_secs(2),
                layout_inputs: None,
            }));
            // Establish a real child relationship at creation, rather than
            // reparenting a live popup across processes with mismatched DPI.
            let hwnd = CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_LAYERED,
                w!("MangoDiskTaskbarStatus"),
                w!("MangoDisk Status"),
                WS_CHILD | WS_CLIPSIBLINGS,
                0,
                0,
                1,
                1,
                parent,
                ptr::null_mut(),
                ptr::null_mut(),
                (&mut *data as *mut RefCell<Window>).cast(),
            );
            if hwnd.is_null() {
                let code = GetLastError();
                let failure = (parent as usize, "create_window", code);
                if host_failure != Some(failure) {
                    log::warn!(
                        "resident_taskbar_native_failed stage=create_window code={code} parent={parent:?} parent_dpi={} parent_context={:?} thread_context={:?}",
                        GetDpiForWindow(parent),
                        GetWindowDpiAwarenessContext(parent),
                        GetThreadDpiAwarenessContext(),
                    );
                    host_failure = Some(failure);
                }
                // Explorer can replace/reconfigure its parent during creation.
                // Keep the adapter alive so a transient failure can recover.
                drop(dpi_context);
                service.publish(DisplayStatus::ShellUnavailable);
                std::thread::sleep(Duration::from_secs(2));
                continue;
            }
            if let Err(code) = super::hosting::background(hwnd, true) {
                log::warn!("resident_taskbar_native_failed stage=background_style code={code}");
                DestroyWindow(hwnd);
                service.publish(DisplayStatus::ShellUnavailable);
                return;
            }
            if host_failure.take().is_some() {
                log::info!("resident_taskbar_embedding_recovered parent={parent:?}");
            }
            log::info!("resident_taskbar_embedded hwnd={hwnd:?} parent={parent:?} shell={shell:?} dpi={} awareness={}", GetDpiForWindow(hwnd), GetAwarenessFromDpiAwarenessContext(GetWindowDpiAwarenessContext(hwnd)));
            drop(dpi_context);
            data.get_mut().releasing = pending_release.take();
            service.window.store(hwnd as usize, Ordering::Relaxed);
            let subscription = shell_events::Subscription::install(hwnd);
            PostMessageW(hwnd, UPDATE, 0, 0);
            let mut message = MSG::default();
            let mut result = GetMessageW(&mut message, ptr::null_mut(), 0, 0);
            while result > 0 {
                TranslateMessage(&message);
                DispatchMessageW(&message);
                result = GetMessageW(&mut message, ptr::null_mut(), 0, 0);
            }
            // Do not release the callback data while a native window can still
            // refer to it, including the exceptional GetMessage failure path.
            drop(subscription);
            if IsWindow(hwnd) != 0 {
                DestroyWindow(hwnd);
            }
            // Display changes destroy a healthy child while its old companion
            // may still be restoring Explorer. Carry the completion barrier
            // across windows so a rapid return cannot use the old reserved gap.
            data.get_mut().release_reservation();
            pending_release = data.get_mut().releasing.take();
            service.window.store(0, Ordering::Relaxed);
            service
                .bounds
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clear();
            service.publish(DisplayStatus::ShellUnavailable);
            if result < 0 {
                log::warn!("resident_taskbar_native_failed stage=message_loop");
                return;
            }
            // Explorer owns only this native surface. Recreate it after a shell
            // restart while retaining the sampler, model and existing WebViews.
            std::thread::sleep(Duration::from_millis(200));
        }
    });
}

/// A failed CreateWindowEx can still deliver WM_NCDESTROY. Its quit message
/// belongs to the failed window, not to the replacement created by this loop.
unsafe fn clear_stale_quit() {
    let mut message = MSG::default();
    while PeekMessageW(&mut message, ptr::null_mut(), WM_QUIT, WM_QUIT, PM_REMOVE) != 0 {}
}

#[cfg(test)]
#[path = "native_tests.rs"]
mod lifecycle_tests;
