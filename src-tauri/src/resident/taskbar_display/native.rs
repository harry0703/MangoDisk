//! The window and its GDI objects stay on one native thread. Other threads only
//! replace a bounded model and post a wake message; callbacks never create WebViews.
use super::{
    directwrite, draw,
    layout::{self, Bounds},
    position::{self, Edge, Environment},
    presentation::{self, Column},
    shell_events,
    surface::Surface,
    transparent,
    visibility::{self, Visibility},
    DisplayStatus, Service,
};
use crate::resident::{
    main_window, panel,
    tray_display::{labels::Labels, windows_bitmap},
};
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
        Graphics::Gdi::*,
        System::{SystemInformation::GetWindowsDirectoryW, Threading::*},
        UI::{
            Controls::WM_MOUSELEAVE, HiDpi::*, Input::KeyboardAndMouse::*, WindowsAndMessaging::*,
        },
    },
};
pub const UPDATE: u32 = WM_APP + 71;

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
    xaml_architecture_matches: Option<bool>,
    geometry_delayed: bool,
    reservation_failed: Option<super::reservation::Failure>,
    reservation_retry: std::time::Instant,
    placement_policy: Option<(
        crate::resident::preference_schema::TaskbarPosition,
        Environment,
        Edge,
    )>,
    foreground: usize,
    shell_surface: bool,
    appearance_checked: std::time::Instant,
}

pub fn start(service: Arc<Service>) {
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
        loop {
            clear_stale_quit();
            let shell = FindWindowW(w!("Shell_TrayWnd"), ptr::null());
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
                xaml_architecture_matches: None,
                geometry_delayed: false,
                reservation_failed: None,
                reservation_retry: std::time::Instant::now(),
                placement_policy: None,
                foreground: 0,
                shell_surface: false,
                appearance_checked: std::time::Instant::now() - Duration::from_secs(2),
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

unsafe extern "system" fn procedure(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_NCCREATE {
        let create = &*(lparam as *const CREATESTRUCTW);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
    }
    let raw = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut RefCell<Window>;
    if raw.is_null() {
        return DefWindowProcW(hwnd, message, wparam, lparam);
    }
    if message == WM_NCDESTROY {
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
        PostQuitMessage(0);
        return 0;
    }
    // A popup menu can dispatch a wake while update() still holds its borrow.
    // Release the coalescing slot even then; the timer remains the safe fallback.
    if message == shell_events::WAKE {
        shell_events::dispatched();
    }
    // Parent-driven hiding need not pass through our logical visibility state.
    // Ignore synchronous notifications from our own ShowWindow calls.
    if message == WM_SHOWWINDOW {
        if let Ok(window) = (*raw).try_borrow() {
            log::info!(
                "resident_taskbar_external_visibility show={} reason={lparam} expected={:?} foreground={:?}",
                wparam != 0,
                window.visibility,
                GetForegroundWindow(),
            );
        }
    }
    // Default processing can synchronously reenter this callback. Borrow state
    // only for handled messages. Handle right release here so DefWindowProc
    // cannot forward our context menu to the Explorer parent.
    if !matches!(
        message,
        UPDATE
            | WM_TIMER
            | shell_events::WAKE
            | WM_PAINT
            | WM_MOUSEMOVE
            | WM_MOUSELEAVE
            | WM_LBUTTONUP
            | WM_RBUTTONUP
            | WM_CONTEXTMENU
    ) {
        return match message {
            WM_ERASEBKGND => 1,
            WM_MOUSEACTIVATE => MA_NOACTIVATE as _,
            WM_CLOSE => {
                ShowWindow(hwnd, SW_HIDE);
                0
            }
            _ => DefWindowProcW(hwnd, message, wparam, lparam),
        };
    }
    // ShowWindow/SetWindowPos and popup menus may synchronously reenter this
    // callback. Never manufacture overlapping mutable references to window state.
    let Ok(mut window) = (*raw).try_borrow_mut() else {
        return DefWindowProcW(hwnd, message, wparam, lparam);
    };
    match message {
        UPDATE | WM_TIMER | shell_events::WAKE => {
            if message == UPDATE {
                if window.service.requested.load(Ordering::Relaxed) {
                    SetTimer(hwnd, 1, 100, None);
                } else {
                    KillTimer(hwnd, 1);
                }
            }
            window.update(hwnd);
            0
        }
        WM_PAINT => {
            if window.background {
                draw::paint(
                    hwnd,
                    &window.columns,
                    &window.surface,
                    window.dpi,
                    window.color,
                    window.hover,
                );
            } else {
                // UpdateLayeredWindow owns the pixels; validate WM_PAINT without
                // trying to BitBlt to a layered window's redirected surface.
                ValidateRect(hwnd, ptr::null());
                window.paint_transparent(hwnd);
            }
            0
        }
        WM_MOUSEMOVE => {
            let mut tracking = TRACKMOUSEEVENT {
                cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                dwFlags: TME_LEAVE,
                hwndTrack: hwnd,
                dwHoverTime: 0,
            };
            TrackMouseEvent(&mut tracking);
            window.update_hover(hwnd);
            0
        }
        WM_MOUSELEAVE => {
            window.hover = None;
            InvalidateRect(hwnd, ptr::null(), 0);
            let app = window.service.app.clone();
            // Match notification-area leave handling after an abandoned press.
            // Dispatch outside this native callback to avoid WebView reentrancy.
            tauri::async_runtime::spawn(async move { panel::tray_pointer_left(&app) });
            0
        }
        WM_LBUTTONUP => {
            window.update_hover(hwnd);
            if let Some(column) = window.hover.and_then(|i| window.columns.get(i)) {
                let app = window.service.app.clone();
                let id = column.id;
                tauri::async_runtime::spawn(async move {
                    if let Err(error) =
                        panel::toggle_from(&app, &format!("taskbar-{}", id.tray_id()), id.metric())
                    {
                        crate::resident::diagnostics::Failure::record(
                            "taskbar_panel_entry",
                            &error,
                        );
                    }
                });
            }
            0
        }
        WM_RBUTTONUP | WM_CONTEXTMENU => {
            window.menu(hwnd);
            0
        }
        _ => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}

impl Window {
    unsafe fn paint_transparent(&mut self, hwnd: HWND) -> bool {
        let initializing = self.text_renderer.is_none();
        match transparent::paint(
            &mut self.text_renderer,
            hwnd,
            &self.columns,
            &self.surface,
            self.dpi,
            self.color,
            self.hover,
        ) {
            Ok(()) => {
                if initializing {
                    log::info!(
                        "resident_taskbar_text_renderer backend=directwrite antialias=grayscale"
                    );
                }
                if self.paint_failed {
                    log::info!("resident_taskbar_paint_recovered");
                }
                self.paint_failed = false;
                true
            }
            Err(error) => {
                // Discard device resources after a failed frame so the next poll
                // can recover without restarting the application.
                self.text_renderer = None;
                if !self.paint_failed {
                    log::warn!(
                        "resident_taskbar_native_failed stage={} code={}",
                        error.stage,
                        error.code
                    );
                }
                self.paint_failed = true;
                self.hide(hwnd, Visibility::PaintFailed);
                self.service.publish(DisplayStatus::ShellUnavailable);
                false
            }
        }
    }

    unsafe fn record_visibility(&mut self, state: Visibility) {
        if self.visibility != Some(state) {
            let foreground = GetForegroundWindow();
            let mut class = [0u16; 256];
            let length =
                GetClassNameW(foreground, class.as_mut_ptr(), class.len() as i32).max(0) as usize;
            let mut frame = RECT::default();
            let has_frame = GetWindowRect(foreground, &mut frame) != 0;
            log::info!(
                "resident_taskbar_visibility previous={:?} current={state:?} foreground={foreground:?} class={} style={:#x} maximized={} frame_available={has_frame} frame=({},{},{},{}) bounds={:?}",
                self.visibility,
                mangodisk_platform::diagnostics::text(&String::from_utf16_lossy(&class[..length])),
                GetWindowLongPtrW(foreground, GWL_STYLE),
                IsZoomed(foreground) != 0,
                frame.left, frame.top, frame.right, frame.bottom,
                self.bounds
            );
            self.visibility = Some(state);
        }
    }

    unsafe fn hide(&mut self, hwnd: HWND, reason: Visibility) {
        self.record_visibility(reason);
        // Native parent visibility can change independently of our cache.
        // Reconcile actual visibility so fullscreen/auto-hide stays respected.
        if self.visible || IsWindowVisible(hwnd) != 0 {
            ShowWindow(hwnd, SW_HIDE);
            self.visible = false;
        }
        self.service
            .bounds
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
    }
    unsafe fn update(&mut self, hwnd: HWND) {
        if !self.service.requested.load(Ordering::Relaxed) {
            self.reservation = None;
            self.hide(hwnd, Visibility::Disabled);
            self.service.publish(DisplayStatus::Tray);
            return;
        }
        if FindWindowW(w!("Shell_TrayWnd"), ptr::null()) != self.shell
            || GetParent(hwnd) != self.parent
            || IsWindow(self.parent) == 0
        {
            DestroyWindow(hwnd);
            return;
        }
        let geometry = self
            .service
            .geometry
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        // The sampler may still hold the old shell snapshot after a restart.
        // Wait for fresh geometry instead of repeatedly destroying the new child.
        let Some(geometry) = geometry.filter(|g| g.shell as HWND == self.shell) else {
            self.hide(hwnd, Visibility::GeometryUnavailable);
            self.service.publish(DisplayStatus::ShellUnavailable);
            return;
        };
        // UIA can lag under CPU load although the independent layout companion
        // still validates our reserved area. Reuse only that live lease, with
        // unchanged DPI/alignment and the live parent bounds checked below.
        // Gap placement continues to require fresh occupied-control rectangles.
        let delayed = geometry.sampled.elapsed() >= Duration::from_secs(3);
        if delayed
            && !(self.reservation.as_ref().is_some_and(|client| {
                client.has_recent_layout(self.shell as usize, self.parent as usize)
            }) && GetDpiForWindow(self.shell).max(96) == geometry.dpi
                && position::read_environment() == geometry.environment)
        {
            self.hide(hwnd, Visibility::GeometryUnavailable);
            self.service.publish(DisplayStatus::ShellUnavailable);
            return;
        }
        if delayed != self.geometry_delayed {
            log::info!(
                "resident_taskbar_geometry_delay delayed={delayed} age_ms={} placement=reserved",
                geometry.sampled.elapsed().as_millis()
            );
            self.geometry_delayed = delayed;
        }
        if geometry.hidden {
            self.hide(hwnd, Visibility::AutoHidden);
            self.service.publish(DisplayStatus::Taskbar);
            return;
        }
        let shell = geometry.shell as HWND;
        let mut current = RECT::default();
        if GetWindowRect(shell, &mut current) == 0 {
            self.hide(hwnd, Visibility::ShellUnavailable);
            self.service.publish(DisplayStatus::ShellUnavailable);
            return;
        }
        // During orientation/size changes, cached UIA bounds and the live parent
        // can describe different layouts. Wait for the sampler before gap checks
        // so that this transition does not incorrectly activate NoSpace fallback.
        if current.left != geometry.bar.left
            || current.top != geometry.bar.top
            || current.right != geometry.bar.right
            || current.bottom != geometry.bar.bottom
        {
            self.hide(hwnd, Visibility::ShellMoving);
            self.service.publish(DisplayStatus::Taskbar);
            return;
        }
        // Classify temporary shell visibility before allocating space. Fullscreen
        // UIA snapshots can report collapsed gaps; those are not NoSpace failures.
        // The taskbar bounds identify the monitor even before a strip is placed.
        let bounds = geometry.bar;
        let center = POINT {
            x: (bounds.left + bounds.right) / 2,
            y: (bounds.top + bounds.bottom) / 2,
        };
        let monitor = MonitorFromPoint(center, MONITOR_DEFAULTTONULL);
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as _,
            ..Default::default()
        };
        let inside = !monitor.is_null()
            && GetMonitorInfoW(monitor, &mut info) != 0
            && bounds.left >= info.rcMonitor.left
            && bounds.right <= info.rcMonitor.right
            && bounds.top >= info.rcMonitor.top
            && bounds.bottom <= info.rcMonitor.bottom;
        let foreground = GetForegroundWindow();
        if self.foreground != foreground as usize {
            self.foreground = foreground as usize;
            self.shell_surface = is_shell_surface(foreground, shell);
        }
        let mut front = RECT::default();
        let front_available = !foreground.is_null() && GetWindowRect(foreground, &mut front) != 0;
        let front_bounds = Bounds {
            left: front.left,
            top: front.top,
            right: front.right,
            bottom: front.bottom,
        };
        let covered = foreground != GetShellWindow()
            && !self.shell_surface
            && foreground != hwnd
            && foreground != shell
            && !foreground.is_null()
            && front_available
            && visibility::is_fullscreen(
                front_bounds,
                Bounds {
                    left: info.rcMonitor.left,
                    top: info.rcMonitor.top,
                    right: info.rcMonitor.right,
                    bottom: info.rcMonitor.bottom,
                },
                GetWindowLongPtrW(foreground, GWL_STYLE) & (WS_DLGFRAME | WS_THICKFRAME) as isize
                    != 0,
                IsZoomed(foreground) != 0,
            );
        let hidden = if !inside || IsWindowVisible(shell) == 0 {
            Some(Visibility::AutoHidden)
        } else if covered {
            Some(Visibility::Fullscreen)
        } else {
            None
        };
        if let Some(reason) = hidden {
            self.hide(hwnd, reason);
            self.service.publish(DisplayStatus::Taskbar);
            return;
        }
        let model = self
            .service
            .model
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let columns = presentation::columns(&model.entries, geometry.dpi, model.compact);
        let Some(surface) = Surface::arrange(&columns, geometry.bar, geometry.dpi) else {
            self.reservation = None;
            self.hide(hwnd, Visibility::UnsupportedLayout);
            self.service.publish(DisplayStatus::UnsupportedLayout);
            return;
        };
        let edge = position::resolve(model.position, geometry.environment);
        let policy = (model.position, geometry.environment, edge);
        if self.placement_policy != Some(policy) {
            log::info!(
                "resident_taskbar_position requested={:?} environment={:?} resolved={:?}",
                model.position,
                geometry.environment,
                edge
            );
            self.placement_policy = Some(policy);
        }
        let gap = (4 * geometry.dpi / 96) as i32;
        let Some(parent_bounds) = super::hosting::client_bounds(self.parent) else {
            self.hide(hwnd, Visibility::ShellUnavailable);
            self.service.publish(DisplayStatus::ShellUnavailable);
            return;
        };
        // Explorer may resize the rebar before moving Shell_TrayWnd during an
        // orientation change, including briefly setting its height to zero.
        // An empty host or one outside the sampled bar is a layout in progress,
        // not evidence that the user's metrics no longer fit.
        if parent_bounds.width() <= 0
            || parent_bounds.height() <= 0
            || !parent_bounds.fits_in(geometry.bar)
        {
            self.hide(hwnd, Visibility::ShellMoving);
            self.service.publish(DisplayStatus::Taskbar);
            return;
        }
        let find_gap = || {
            layout::place(
                parent_bounds,
                &geometry.occupied,
                surface.width,
                surface.height,
                gap,
                edge,
                model.position == crate::resident::preference_schema::TaskbarPosition::Auto,
            )
        };
        // Unknown environments must not cache support without probing. Defer
        // the Windows 11 check until a usable snapshot identifies its taskbar,
        // including when an earlier registry read failed and later recovered.
        let reservation_supported = match geometry.environment {
            Environment::Unknown => false,
            Environment::Windows10 => true,
            Environment::Windows11Centered | Environment::Windows11LeftAligned => *self
                .xaml_architecture_matches
                .get_or_insert_with(|| super::hosting::xaml_architecture_matches(shell)),
        };
        // Cross-architecture XAML cannot reserve space, but the embedded child
        // can still use a collision-checked gap, just like an unknown shell.
        let placed = if !reservation_supported {
            self.reservation = None;
            find_gap()
        } else {
            if self.reservation.is_none() && std::time::Instant::now() >= self.reservation_retry {
                match super::reservation::Client::start(&self.service.app) {
                    Ok(client) => self.reservation = Some(client),
                    Err(error) => {
                        if self.reservation_failed != Some(error) {
                            log::warn!(
                                "resident_taskbar_reservation_failed stage={:?} code={}",
                                error.stage,
                                error.code
                            );
                        }
                        self.reservation_failed = Some(error);
                        self.reservation_retry = std::time::Instant::now() + Duration::from_secs(2);
                    }
                }
            }
            let result = self.reservation.as_ref().and_then(|client| {
                client.request(super::reservation::Request::new(
                    self.shell as usize,
                    self.parent as usize,
                    parent_bounds,
                    (surface.width, surface.height),
                    gap,
                    edge,
                ))
            });
            match result {
                Some(Ok(bounds)) => {
                    if self.reservation_failed.take().is_some() {
                        log::info!("resident_taskbar_reservation_recovered");
                    }
                    Some(bounds)
                }
                Some(Err(error)) if error.stage != super::reservation::Stage::Geometry => {
                    if self.reservation_failed != Some(error) {
                        log::warn!(
                            "resident_taskbar_reservation_failed stage={:?} code={}",
                            error.stage,
                            error.code
                        );
                    }
                    self.reservation_failed = Some(error);
                    if error.stage == super::reservation::Stage::Space {
                        self.hide(hwnd, Visibility::NoSpace);
                        self.service.publish(DisplayStatus::NoSpace);
                        return;
                    }
                    if matches!(
                        error.stage,
                        super::reservation::Stage::Channel | super::reservation::Stage::Protocol
                    ) {
                        self.reservation = None;
                        self.reservation_retry = std::time::Instant::now() + Duration::from_secs(2);
                    }
                    self.hide(hwnd, Visibility::ShellUnavailable);
                    self.service.publish(DisplayStatus::ShellUnavailable);
                    return;
                }
                _ => {
                    self.hide(hwnd, Visibility::ShellMoving);
                    self.service.publish(DisplayStatus::Taskbar);
                    return;
                }
            }
        };
        let Some(bounds) = placed else {
            if self.visibility != Some(Visibility::NoSpace) {
                log::info!(
                    "resident_taskbar_no_space bar={:?} parent={parent_bounds:?} width={} height={} occupied_count={}",
                    geometry.bar,
                    surface.width,
                    surface.height,
                    geometry.occupied.len()
                );
            }
            self.hide(hwnd, Visibility::NoSpace);
            self.service.publish(DisplayStatus::NoSpace);
            return;
        };
        let color = if self.appearance_checked.elapsed() >= Duration::from_secs(1) {
            self.appearance_checked = std::time::Instant::now();
            windows_bitmap::appearance().color
        } else {
            self.color
        };
        let layout_changed = self.bounds != bounds || self.surface != surface;
        let changed = layout_changed
            || self.columns != columns
            || self.color != color
            || self.dpi != geometry.dpi
            || self.background != model.background
            || self.paint_failed;
        if self.bounds != bounds || self.surface.vertical != surface.vertical {
            log::info!(
                "resident_taskbar_layout vertical={} preference={:?} compact={} width={} height={} left={} top={} dpi={}",
                surface.vertical,
                model.position,
                model.compact,
                surface.width,
                surface.height,
                bounds.left,
                bounds.top,
                geometry.dpi
            );
        }
        if self.background != model.background {
            if let Err(code) = super::hosting::background(hwnd, model.background) {
                if !self.paint_failed {
                    log::warn!("resident_taskbar_native_failed stage=background_style code={code}");
                }
                self.paint_failed = true;
                self.hide(hwnd, Visibility::PaintFailed);
                self.service.publish(DisplayStatus::ShellUnavailable);
                return;
            }
            self.background = model.background;
            if self.paint_failed {
                log::info!("resident_taskbar_paint_recovered stage=background_style");
                self.paint_failed = false;
            }
            log::debug!("resident_taskbar_background enabled={}", self.background);
        }
        self.bounds = bounds;
        self.surface = surface;
        self.columns = columns;
        self.color = color;
        self.dpi = geometry.dpi;
        // Child visibility follows the taskbar. Never repair global TOPMOST
        // order: doing so would reintroduce Show Desktop/menu races. Only our
        // sibling order is set; Explorer's task buttons keep their original bounds.
        let restoring = !self.visible;
        if changed || restoring {
            let mut origin = POINT {
                x: bounds.left,
                y: bounds.top,
            };
            let converted = ScreenToClient(self.parent, &mut origin) != 0;
            let positioned = converted
                && SetWindowPos(
                    hwnd,
                    HWND_TOP,
                    origin.x,
                    origin.y,
                    bounds.width(),
                    bounds.height(),
                    SWP_NOACTIVATE | SWP_NOOWNERZORDER | SWP_SHOWWINDOW,
                ) != 0;
            if !positioned {
                if !self.position_failed {
                    log::warn!("resident_taskbar_position_failed mode=child converted={converted} code={} parent={:?} bounds={bounds:?}", GetLastError(), self.parent);
                }
                self.position_failed = true;
                self.hide(hwnd, Visibility::PositionFailed);
                self.service.publish(DisplayStatus::ShellUnavailable);
                return;
            }
            let radius = (8 * geometry.dpi / 96) as i32;
            let region = CreateRoundRectRgn(
                0,
                0,
                bounds.width() + 1,
                bounds.height() + 1,
                radius,
                radius,
            );
            if SetWindowRgn(hwnd, region, 1) == 0 {
                DeleteObject(region);
            }
            if !self.background && !self.paint_transparent(hwnd) {
                return;
            }
            if self.position_failed {
                log::info!("resident_taskbar_position_recovered mode=child");
            }
            self.position_failed = false;
            InvalidateRect(hwnd, ptr::null(), 0);
            self.visible = true;
        }
        *self
            .service
            .bounds
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = self
            .columns
            .iter()
            .zip(&self.surface.cells)
            .map(|(column, cell)| {
                (
                    column.id,
                    Bounds {
                        left: bounds.left + cell.left,
                        right: bounds.left + cell.right,
                        top: bounds.top + cell.top,
                        bottom: bounds.top + cell.bottom,
                    },
                )
            })
            .collect();
        self.record_visibility(Visibility::Visible);
        self.update_hover(hwnd);
        self.service.publish(DisplayStatus::Taskbar);
    }
    unsafe fn update_hover(&mut self, hwnd: HWND) {
        let mut point = POINT::default();
        GetCursorPos(&mut point);
        let hover =
            self.surface.cells.iter().position(|cell| {
                cell.contains(point.x - self.bounds.left, point.y - self.bounds.top)
            });
        if self.hover != hover {
            self.hover = hover;
            InvalidateRect(hwnd, ptr::null(), 0);
        }
    }
    unsafe fn menu(&self, hwnd: HWND) {
        let locale = self
            .service
            .model
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .locale
            .clone();
        let labels = Labels::for_locale(&locale);
        let menu = CreatePopupMenu();
        if menu.is_null() {
            return;
        }
        for (id, key) in [(1, "openMain"), (2, "settings"), (3, "quit")] {
            let text: Vec<u16> = labels.text(key).encode_utf16().chain(Some(0)).collect();
            AppendMenuW(menu, MF_STRING, id, text.as_ptr());
        }
        let mut point = POINT::default();
        GetCursorPos(&mut point);
        SetForegroundWindow(hwnd);
        let command = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON,
            point.x,
            point.y,
            0,
            hwnd,
            ptr::null(),
        );
        DestroyMenu(menu);
        PostMessageW(hwnd, WM_NULL, 0, 0);
        match command {
            1 => main_window::request(
                &self.service.app,
                main_window::Destination::Main,
                "taskbar_menu",
            ),
            2 => main_window::request(
                &self.service.app,
                main_window::Destination::Settings,
                "taskbar_menu",
            ),
            3 => self.service.app.exit(0),
            _ => {}
        }
    }
}

/// The shell desktop is not necessarily GetShellWindow(): Windows 10 commonly
/// activates a full-monitor WorkerW when the user clicks the wallpaper. Resolve
/// its class and shell ownership together, once per foreground-window change.
/// The separate system Start host is also a shell surface during its animation.
unsafe fn is_shell_surface(hwnd: HWND, shell: HWND) -> bool {
    let mut process_id = 0;
    let mut shell_process_id = 0;
    GetWindowThreadProcessId(hwnd, &mut process_id);
    GetWindowThreadProcessId(shell, &mut shell_process_id);
    let mut class = [0u16; 256];
    let length = GetClassNameW(hwnd, class.as_mut_ptr(), class.len() as i32).max(0) as usize;
    let class = String::from_utf16_lossy(&class[..length]);
    if visibility::is_shell_desktop(&class, process_id, shell_process_id) {
        return true;
    }
    if class != "Windows.UI.Core.CoreWindow" {
        return false;
    }
    // Resolve only on a foreground change, and only for the relevant class.
    // A process basename alone must not exempt a user-installed UWP application.
    let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, process_id);
    if process.is_null() {
        return false;
    }
    let mut image = [0u16; 32768];
    let mut length = image.len() as u32;
    let queried = QueryFullProcessImageNameW(process, 0, image.as_mut_ptr(), &mut length) != 0;
    CloseHandle(process);
    if !queried {
        return false;
    }
    let mut windows = [0u16; 32768];
    let count = GetWindowsDirectoryW(windows.as_mut_ptr(), windows.len() as u32) as usize;
    if count == 0 || count >= windows.len() {
        return false;
    }
    let image = String::from_utf16_lossy(&image[..length as usize]);
    let system_apps = format!(
        "{}\\SystemApps\\",
        String::from_utf16_lossy(&windows[..count])
    );
    let system_app = image
        .to_ascii_lowercase()
        .starts_with(&system_apps.to_ascii_lowercase());
    let name = image.rsplit('\\').next().unwrap_or_default();
    let flyout = visibility::is_shell_flyout(&class, name, system_app);
    if flyout {
        log::debug!("resident_taskbar_system_surface image={name} pid={process_id}");
    }
    flyout
}

#[cfg(test)]
mod lifecycle_tests {
    use super::*;

    #[test]
    fn failed_creation_does_not_stop_the_replacement_message_loop() {
        std::thread::spawn(|| unsafe {
            unsafe extern "system" fn fail_creation(
                window: HWND,
                message: u32,
                wparam: WPARAM,
                lparam: LPARAM,
            ) -> LRESULT {
                if message == WM_CREATE {
                    return -1;
                }
                if message == WM_NCDESTROY {
                    PostQuitMessage(0);
                }
                DefWindowProcW(window, message, wparam, lparam)
            }
            let class = WNDCLASSW {
                lpfnWndProc: Some(fail_creation),
                lpszClassName: w!("MangoDiskFailedCreationTest"),
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
                1,
                1,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null(),
            );
            assert!(window.is_null());
            let mut message = MSG::default();
            assert_ne!(
                PeekMessageW(&mut message, ptr::null_mut(), WM_QUIT, WM_QUIT, PM_NOREMOVE),
                0,
                "failed creation must reproduce the pending destruction message"
            );
            clear_stale_quit();
            PostQuitMessage(17);
            assert_eq!(GetMessageW(&mut message, ptr::null_mut(), 0, 0), 0);
            assert_eq!(
                message.wParam, 17,
                "replacement must receive only its own exit"
            );
            UnregisterClassW(class.lpszClassName, ptr::null_mut());
        })
        .join()
        .unwrap();
    }
}
