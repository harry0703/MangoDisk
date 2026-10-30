use std::sync::{atomic::Ordering, Arc};

#[cfg(not(target_os = "macos"))]
use tauri::PhysicalPosition;
use tauri::{Emitter, Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent};

use super::{panel_page::PanelPage, runtime::ResidentState, PANEL_LABEL};

#[cfg(target_os = "macos")]
mod macos;

const WIDTH: f64 = 390.0;
const HEIGHT: f64 = 610.0;

pub const PAGE_EVENT: &str = "resident-panel-page";
pub const VISIBILITY_EVENT: &str = "resident-panel-visibility";

pub fn select_page(app: &tauri::AppHandle, page: PanelPage) {
    let page = set_page(app, page);
    let _ = app.emit_to(PANEL_LABEL, PAGE_EVENT, page);
}

/// A frontend selection already updates that panel; native shortcuts also notify it.
pub fn set_page(app: &tauri::AppHandle, page: PanelPage) -> PanelPage {
    let state = app.state::<Arc<ResidentState>>();
    let page = page.available_for_battery(
        state
            .reading
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .resources
            .battery_present,
    );
    *state
        .panel_page
        .lock()
        .unwrap_or_else(|error| error.into_inner()) = page;
    if page == PanelPage::Cpu && state.panel_open.load(Ordering::Relaxed) {
        state.refresh_application_processes();
    } else {
        state.wake();
    }
    if state.panel_open.load(Ordering::Relaxed) {
        log_detail_snapshot(&state, page);
    }
    page
}

fn log_detail_snapshot(state: &ResidentState, page: PanelPage) {
    let reading = state
        .published_reading
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let (status, sampled_at_ms, has_value) = match page {
        PanelPage::Power => {
            let metric = &reading.resources.battery_details;
            (metric.status, metric.sampled_at_ms, metric.value.is_some())
        }
        PanelPage::Disk => {
            let metric = &reading.resources.disk_health;
            (metric.status, metric.sampled_at_ms, metric.value.is_some())
        }
        _ => return,
    };
    log::info!(
        "resident_panel_detail_snapshot page={page:?} revision={} status={status:?} has_value={has_value} sample_age_ms={:?}",
        reading.revision,
        sampled_at_ms.and_then(|at| super::sampling_workers::timestamp_ms().checked_sub(at))
    );
}

pub fn toggle_from(
    app: &tauri::AppHandle,
    source: &str,
    page: Option<PanelPage>,
) -> tauri::Result<()> {
    toggle_entry(app, source, EntryRequest::Resume(page))
}

#[cfg(windows)]
pub fn toggle_page_from(
    app: &tauri::AppHandle,
    source: &str,
    page: PanelPage,
) -> tauri::Result<()> {
    toggle_entry(app, source, EntryRequest::Select(page))
}

#[derive(Clone, Copy)]
enum EntryRequest {
    Resume(Option<PanelPage>),
    #[cfg(any(windows, test))]
    Select(PanelPage),
}

impl EntryRequest {
    fn available_for_battery(self, present: Option<bool>) -> Self {
        match self {
            Self::Resume(page) => {
                Self::Resume(page.map(|page| page.available_for_battery(present)))
            }
            #[cfg(any(windows, test))]
            Self::Select(page) => Self::Select(page.available_for_battery(present)),
        }
    }

    fn selection(self, previous: PanelPage, is_open: bool) -> PanelPage {
        match self {
            Self::Resume(requested) => entry_selection(previous, requested, is_open),
            #[cfg(any(windows, test))]
            Self::Select(requested) => requested,
        }
    }

    fn repeats(self, previous: PanelPage) -> bool {
        match (self, previous) {
            (Self::Resume(_), _) => true,
            #[cfg(any(windows, test))]
            (Self::Select(requested), previous) => requested == previous,
        }
    }
}

fn toggle_entry(app: &tauri::AppHandle, source: &str, request: EntryRequest) -> tauri::Result<()> {
    let state = app.state::<Arc<ResidentState>>();
    // Resolve shortcuts before repeat detection so an unavailable Power entry
    // still toggles the CPU page it actually opened on a batteryless desktop.
    let battery_present = state
        .reading
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .resources
        .battery_present;
    let request = request.available_for_battery(battery_present);
    let _action = state
        .panel_action
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let same = state
        .panel_source
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .as_str()
        == source;
    let was_open = state.panel_open.load(Ordering::Relaxed);
    let previous = *state
        .panel_page
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if was_open && same && request.repeats(previous) {
        log::info!("resident_panel_entry source={source} action=hide reason=repeated_click");
        hide(app);
        return Ok(());
    }
    *state
        .panel_source
        .lock()
        .unwrap_or_else(|error| error.into_inner()) = source.into();
    let selected = request.selection(previous.available_for_battery(battery_present), was_open);
    let notify = match request {
        EntryRequest::Resume(page) => was_open && page.is_some(),
        // Explicit shortcuts must also leave the remembered overview tab.
        #[cfg(any(windows, test))]
        EntryRequest::Select(_) => true,
    };
    if selected != previous || notify {
        select_page(app, selected);
    }
    log::info!("resident_panel_entry source={source} switching={was_open} metric={selected:?}");
    open(app)
}

fn entry_selection(previous: PanelPage, requested: Option<PanelPage>, is_open: bool) -> PanelPage {
    // Reopening resumes the user's tab. Explicit metric shortcuts can still
    // navigate an already open panel, without resetting the next visit to CPU.
    if is_open {
        requested.unwrap_or(previous)
    } else {
        previous
    }
}

pub fn open(app: &tauri::AppHandle) -> tauri::Result<()> {
    let state = app.state::<Arc<ResidentState>>();
    if !state.enabled() {
        return Ok(());
    }
    #[cfg(windows)]
    let needs_focus = !native_panel_is_focused(app);
    #[cfg(not(windows))]
    let needs_focus = true;
    *state
        .panel_requested_at
        .lock()
        .unwrap_or_else(|error| error.into_inner()) = needs_focus.then(std::time::Instant::now);
    log::info!(
        "resident_panel_open_requested ready={}",
        state.panel_ready.load(Ordering::Relaxed)
    );
    state.panel_open.store(true, Ordering::Relaxed);
    let page = *state.panel_page.lock().unwrap_or_else(|e| e.into_inner());
    log_detail_snapshot(&state, page);
    if page == PanelPage::Cpu {
        state.refresh_application_processes();
    }
    // Showing never waits for Vue, preference I/O, samples, or icon resolution.
    // Usually startup has already prepared the hidden WebView; an early click
    // still reveals its first frame while frontend initialization completes.
    if let Err(error) = ensure_created(app).and_then(|()| show(app)) {
        // Failed native creation/positioning must not retain open intent: it
        // would keep detailed sampling active and consume the next click as hide.
        hide(app);
        return Err(error);
    }
    state.wake();
    Ok(())
}

/// Prepare one hidden WebView without changing foreground or sampling intent.
/// Leave the native callback before creating it to keep WebView2 responsive.
pub fn prewarm(app: &tauri::AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if app.state::<Arc<ResidentState>>().enabled() {
            if let Err(error) = ensure_created(&app) {
                log::warn!("resident_panel_prewarm_failed error={error}");
            }
        }
    });
}

fn ensure_created(app: &tauri::AppHandle) -> tauri::Result<()> {
    let state = app.state::<Arc<ResidentState>>();
    // Startup preparation and an early click share one creation lock.
    let creation = state
        .panel_creation
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if app.get_webview_window(PANEL_LABEL).is_none() {
        #[cfg(windows)]
        state.panel_hide_blurs.store(0, Ordering::Relaxed);
        let started = std::time::Instant::now();
        let builder =
            WebviewWindowBuilder::new(app, PANEL_LABEL, WebviewUrl::App("tray-panel.html".into()))
                .title("MangoDisk")
                .inner_size(WIDTH, HEIGHT)
                .resizable(false)
                .decorations(false)
                .visible(false)
                .focused(false)
                .skip_taskbar(true)
                .always_on_top(true)
                .shadow(true);
        // A titled macOS window keeps the system's rounded frame and shadow.
        // Overlay lets the panel content fill it without a separate title bar.
        #[cfg(target_os = "macos")]
        let builder = builder
            // The tray panel may appear while another application is active.
            // Deliver that first click to its controls instead of consuming it
            // only for activation while the OS completes the focus transition.
            .accept_first_mouse(true)
            .decorations(true)
            .title_bar_style(tauri::TitleBarStyle::Overlay)
            .hidden_title(true);
        let window = builder.build()?;
        #[cfg(windows)]
        crate::webview_recovery::install(&window);
        #[cfg(target_os = "macos")]
        {
            let app = app.clone();
            window.with_webview(move |webview| {
                // Tauri owns both native objects for the duration of this main-thread callback.
                let native = unsafe { &*webview.ns_window().cast::<objc2_app_kit::NSWindow>() };
                let view = unsafe { &*webview.inner().cast::<objc2_app_kit::NSView>() };
                macos::create(&app, native, view);
            })?;
        }
        let app = app.clone();
        window.on_window_event(move |event| match event {
            WindowEvent::Focused(false) => {
                log::info!("resident_panel_focus_changed focused=false");
                #[cfg(windows)]
                {
                    let state = app.state::<Arc<ResidentState>>();
                    if ignore_queued_blur(&state.panel_hide_blurs, native_panel_is_focused(&app)) {
                        log::info!(
                            "resident_panel_blur_ignored reason=programmatic_hide_or_newer_focus"
                        );
                        return;
                    }
                }
                // Native entry presses can take focus before delivering the tray
                // event. Preserve open intent so that press closes, rather than
                // immediately reopening, the panel.
                if tray_owns_focus(&app) {
                    log::info!("resident_panel_blur_deferred reason=entry_interaction");
                } else {
                    hide(&app);
                }
            }
            WindowEvent::Focused(true) => {
                #[cfg(windows)]
                if !native_panel_is_focused(&app) {
                    log::info!("resident_panel_focus_ignored reason=stale_focus");
                    return;
                }
                focused(&app);
            }
            WindowEvent::CloseRequested { api, .. } => {
                api.prevent_close();
                hide(&app);
            }
            _ => {}
        });
        log::info!(
            "resident_panel_created elapsed_ms={}",
            started.elapsed().as_millis()
        );
    }
    drop(creation);
    Ok(())
}

fn focused(app: &tauri::AppHandle) {
    log::info!("resident_panel_focus_changed focused=true");
    let state = app.state::<Arc<ResidentState>>();
    if let Some(started) = state
        .panel_requested_at
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .take()
    {
        log::info!(
            "resident_panel_focused elapsed_ms={}",
            started.elapsed().as_millis()
        );
    };
}

#[cfg(target_os = "macos")]
pub(crate) fn shutdown(app: &tauri::AppHandle) {
    app.state::<Arc<ResidentState>>()
        .panel_open
        .store(false, Ordering::Relaxed);
    macos::destroy();
}

pub fn is_visible(app: &tauri::AppHandle) -> tauri::Result<bool> {
    #[cfg(target_os = "macos")]
    {
        macos::query(app, macos::is_visible)
    }
    #[cfg(not(target_os = "macos"))]
    {
        app.get_webview_window(PANEL_LABEL)
            .map_or(Ok(false), |window| window.is_visible())
    }
}

#[cfg(windows)]
pub(crate) fn recreate_webview(app: &tauri::AppHandle, visible: bool) -> tauri::Result<()> {
    let state = app.state::<Arc<ResidentState>>();
    state.panel_ready.store(false, Ordering::Relaxed);
    if visible {
        open(app)
    } else {
        hide(app);
        ensure_created(app)
    }
}

#[cfg(target_os = "macos")]
fn macos_panel_behavior(
    current: objc2_app_kit::NSWindowCollectionBehavior,
    joins_applications: bool,
) -> objc2_app_kit::NSWindowCollectionBehavior {
    use objc2_app_kit::NSWindowCollectionBehavior as Behavior;

    // The panel belongs to the clicked entry's Space, including another app's
    // fullscreen Space. Stationary also keeps Show Desktop from sweeping it away.
    let mut behavior = (current
        & !(Behavior::Managed
            | Behavior::Transient
            | Behavior::MoveToActiveSpace
            | Behavior::FullScreenPrimary
            | Behavior::FullScreenNone
            | Behavior::Primary
            | Behavior::Auxiliary
            | Behavior::CanJoinAllApplications))
        | Behavior::Stationary
        | Behavior::CanJoinAllSpaces
        | Behavior::FullScreenAuxiliary;
    // macOS 13 added an explicit cross-application overlay policy. Older systems
    // retain the supported Spaces/fullscreen auxiliary flags.
    if joins_applications {
        behavior |= Behavior::CanJoinAllApplications;
    }
    behavior
}

#[cfg(windows)]
fn native_panel_is_focused(app: &tauri::AppHandle) -> bool {
    app.get_webview_window(PANEL_LABEL)
        .and_then(|window| window.hwnd().ok())
        .is_some_and(|hwnd| unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::GetForegroundWindow() == hwnd.0.cast()
        })
}

#[cfg(any(windows, test))]
fn ignore_queued_blur(pending: &std::sync::atomic::AtomicU32, focused: bool) -> bool {
    let mut count = pending.load(Ordering::Relaxed);
    while count > 0 {
        match pending.compare_exchange_weak(count, count - 1, Ordering::Relaxed, Ordering::Relaxed)
        {
            Ok(_) => return true,
            Err(actual) => count = actual,
        }
    }
    focused
}

#[cfg(windows)]
fn tray_owns_focus(app: &tauri::AppHandle) -> bool {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetAncestor, GetForegroundWindow, WindowFromPoint, GA_ROOT,
    };
    let Some(window) = app.get_webview_window(PANEL_LABEL) else {
        return false;
    };
    let Ok(cursor) = window.cursor_position() else {
        return false;
    };
    let over_entry = super::tray_display::display_entry::DisplayId::ALL
        .into_iter()
        .any(|id| {
            app.tray_by_id(id.tray_id())
                .and_then(|tray| tray.rect().ok().flatten())
                .is_some_and(|rect| {
                    let position = rect.position.to_physical::<f64>(1.0);
                    let size = rect.size.to_physical::<f64>(1.0);
                    contains(
                        (cursor.x, cursor.y),
                        (position.x, position.y, size.width, size.height),
                    )
                })
        });
    // Cursor position alone is insufficient: Alt+Tab must still dismiss the
    // panel when the pointer happens to remain over the tray icon.
    unsafe {
        let surface = WindowFromPoint(windows_sys::Win32::Foundation::POINT {
            x: cursor.x as i32,
            y: cursor.y as i32,
        });
        let root = GetAncestor(surface, GA_ROOT);
        // Match the actual native window, not its rectangle: an occluded strip
        // must not retain the panel when the user activates another application.
        let over_floating = app
            .get_webview_window(super::floating_monitor::LABEL)
            .and_then(|window| window.hwnd().ok())
            .is_some_and(|hwnd| root == hwnd.0.cast());
        if !over_entry
            && !super::taskbar_display::contains_point(app, cursor.x, cursor.y)
            && !over_floating
        {
            return false;
        }
        // Embedded no-activate children can receive the press before Explorer
        // becomes foreground. Keep open intent until button-up consumes it.
        // A pointer merely parked here must not prevent keyboard dismissal.
        let deferred = !surface.is_null()
            && defer_entry_blur(
                root == GetForegroundWindow(),
                GetAsyncKeyState(VK_LBUTTON as i32) < 0,
            );
        if deferred && over_floating {
            settle_floating_blur(app);
        }
        deferred
    }
}

#[cfg(windows)]
fn settle_floating_blur(app: &tauri::AppHandle) {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON};

    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        // A metric click will toggle or refocus the panel on button-up. Blank
        // cells and window dragging have no metric command, so dismiss their
        // deferred blur after release instead of leaving an unfocused panel open.
        while unsafe { GetAsyncKeyState(VK_LBUTTON as i32) } < 0 {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        tray_pointer_left(&app);
    });
}

#[cfg(any(windows, test))]
fn defer_entry_blur(entry_has_focus: bool, left_pressed: bool) -> bool {
    entry_has_focus || left_pressed
}

#[cfg(target_os = "macos")]
fn tray_owns_focus(app: &tauri::AppHandle) -> bool {
    use objc2_app_kit::NSEvent;

    let Some(window) = app.get_webview_window(PANEL_LABEL) else {
        return false;
    };
    let Ok(cursor) = window.cursor_position() else {
        return false;
    };
    let over_entry = super::tray_display::macos_thread::rect(app, super::TRAY_ID)
        .ok()
        .flatten()
        .is_some_and(|rect| {
            let position = rect.position.to_physical::<f64>(1.0);
            let size = rect.size.to_physical::<f64>(1.0);
            contains(
                (cursor.x, cursor.y),
                (position.x, position.y, size.width, size.height),
            )
        });
    // A parked pointer must not suppress Cmd+Tab or another app taking focus.
    // Bit zero is the primary button; secondary presses close via the menu path.
    defer_macos_entry_blur(over_entry, NSEvent::pressedMouseButtons() & 1 != 0)
}

#[cfg(any(target_os = "macos", test))]
fn defer_macos_entry_blur(over_entry: bool, left_pressed: bool) -> bool {
    over_entry && left_pressed
}

#[cfg(not(any(windows, target_os = "macos")))]
fn tray_owns_focus(_app: &tauri::AppHandle) -> bool {
    false
}

#[cfg(windows)]
pub fn tray_pointer_left(app: &tauri::AppHandle) {
    // Dismiss a deferred blur if a tray press was abandoned or a context menu
    // took focus. A focused panel stays open when the pointer enters it.
    // A newly opened WebView may not have received its first focus event yet.
    if app
        .state::<Arc<ResidentState>>()
        .panel_requested_at
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .is_some()
    {
        return;
    }
    if let Some(window) = app.get_webview_window(PANEL_LABEL) {
        if !window.is_focused().unwrap_or(false) {
            hide(app);
        }
    }
}

pub fn hide(app: &tauri::AppHandle) {
    let state = app.state::<Arc<ResidentState>>();
    state.panel_open.store(false, Ordering::Relaxed);
    *state
        .panel_requested_at
        .lock()
        .unwrap_or_else(|error| error.into_inner()) = None;
    if let Some(window) = app.get_webview_window(PANEL_LABEL) {
        #[cfg(windows)]
        // This getter drains earlier queued show/focus requests before Hide.
        let expects_blur = window.is_focused().unwrap_or(false);
        // Each focused native hide queues one blur, possibly after a newer open.
        #[cfg(windows)]
        if expects_blur {
            state.panel_hide_blurs.fetch_add(1, Ordering::Relaxed);
        }
        #[cfg(target_os = "macos")]
        let result = {
            let app = app.clone();
            // with_webview can hold its dispatcher lock during a synchronous IPC
            // callback; emitting visibility from there would reenter that lock.
            window.run_on_main_thread(move || macos::hide(&app))
        };
        #[cfg(not(target_os = "macos"))]
        let result = window.hide();
        if let Err(error) = result {
            #[cfg(windows)]
            if expects_blur {
                ignore_queued_blur(&state.panel_hide_blurs, false);
            }
            log::warn!("resident_panel_hide_failed error={error}");
        } else {
            #[cfg(not(target_os = "macos"))]
            let _ = app.emit_to(PANEL_LABEL, VISIBILITY_EVENT, false);
        }
    }
    state.wake();
}

pub fn ready(app: &tauri::AppHandle) {
    let state = app.state::<Arc<ResidentState>>();
    state.panel_ready.store(true, Ordering::Relaxed);
    // A prewarmed page becoming ready must not reveal itself or steal focus.
    log::info!("resident_panel_ready");
}

fn show(app: &tauri::AppHandle) -> tauri::Result<()> {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        let target = app.clone();
        // Monitor geometry and native placement belong to the UI thread. Keep
        // their snapshot and presentation ordered, including cancelled opens.
        app.run_on_main_thread(move || {
            if let Err(error) = show_window(&target) {
                hide(&target);
                super::diagnostics::Failure::record("panel_show", &error);
            }
        })
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    show_window(app)
}

fn show_window(app: &tauri::AppHandle) -> tauri::Result<()> {
    let state = app.state::<Arc<ResidentState>>();
    if !state.panel_open.load(Ordering::Relaxed) || !state.enabled() {
        return Ok(());
    }
    #[cfg(target_os = "macos")]
    if app.get_webview_window(PANEL_LABEL).is_none() {
        return Ok(());
    }
    #[cfg(not(target_os = "macos"))]
    let Some(window) = app.get_webview_window(PANEL_LABEL) else {
        return Ok(());
    };
    let source = state
        .panel_source
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clone();
    #[cfg(target_os = "macos")]
    let geometry = super::tray_display::macos_thread::geometry(app, &source)?;
    #[cfg(target_os = "macos")]
    let anchor_scale = geometry.as_ref().map(|geometry| geometry.scale_factor);
    #[cfg(target_os = "macos")]
    let anchor = geometry.map(|geometry| geometry.bounds);
    #[cfg(not(target_os = "macos"))]
    let anchor_scale = None;
    #[cfg(not(target_os = "macos"))]
    let anchor = app
        .tray_by_id(&source)
        .and_then(|tray| tray.rect().ok().flatten());
    let anchor = anchor.map(|rect| {
        let position = rect.position.to_physical::<f64>(1.0);
        let size = rect.size.to_physical::<f64>(1.0);
        (
            position.x + size.width / 2.0,
            position.y + size.height / 2.0,
        )
    });
    #[cfg(windows)]
    let (anchor, anchor_source) = if let Some(point) = (source == super::floating_monitor::LABEL)
        .then(|| super::floating_monitor::anchor(app))
        .flatten()
    {
        (Some(point), "floating_monitor")
    } else if let Some(point) = super::taskbar_display::anchor(app, Some(&source)) {
        (Some(point), "taskbar_entry")
    } else if let Some(point) = anchor {
        (Some(point), "tray_entry")
    } else if let Some(point) = super::taskbar_display::anchor(app, None) {
        // Settings can open the panel after its previous entry was hidden or removed.
        // Keep that popup near the remaining taskbar display rather than the screen top.
        (Some(point), "taskbar_fallback")
    } else {
        (None, "monitor_fallback")
    };
    #[cfg(not(windows))]
    let anchor_source = if anchor.is_some() {
        "tray_entry"
    } else {
        "monitor_fallback"
    };
    // macOS scales global origins by each display's backing factor. Bounds can
    // overlap across different scales, so retain the tray window's factor too.
    let monitor = match anchor {
        Some(point) => app.available_monitors()?.into_iter().find(|monitor| {
            monitor_contains_anchor(
                point,
                (
                    monitor.position().x as f64,
                    monitor.position().y as f64,
                    monitor.size().width as f64,
                    monitor.size().height as f64,
                ),
                monitor.scale_factor(),
                anchor_scale,
            )
        }),
        None => None,
    };
    let monitor = match monitor {
        Some(monitor) => Some(monitor),
        None => app.primary_monitor()?,
    };
    #[cfg(target_os = "macos")]
    let mut native_placement = None;
    if let Some(monitor) = monitor {
        let area = monitor.work_area();
        let scale = monitor.scale_factor();
        let width = (WIDTH * scale).min(area.size.width as f64);
        let height = (HEIGHT * scale).min(area.size.height as f64);
        #[cfg(not(any(windows, target_os = "macos")))]
        window.set_size(tauri::LogicalSize::new(width / scale, height / scale))?;
        let work = (
            area.position.x as f64,
            area.position.y as f64,
            area.size.width as f64,
            area.size.height as f64,
        );
        let anchor = anchor.unwrap_or((work.0 + work.2 - width / 2.0, work.1));
        #[cfg(windows)]
        let floating_position = (source == super::floating_monitor::LABEL)
            .then(|| {
                super::floating_monitor::panel_position(app, work, (width, height), 6.0 * scale)
            })
            .flatten();
        #[cfg(windows)]
        let (x, y) = floating_position
            .unwrap_or_else(|| position(anchor, work, (width, height), 6.0 * scale));
        #[cfg(not(windows))]
        let (x, y) = position(anchor, work, (width, height), 6.0 * scale);
        #[cfg(not(target_os = "macos"))]
        window.set_position(PhysicalPosition::new(x.round() as i32, y.round() as i32))?;
        #[cfg(target_os = "macos")]
        {
            native_placement = Some(macos::Placement::from_physical(
                (x, y),
                (width, height),
                scale,
            ));
        }
        // A hidden Windows panel can retain the previous display's DPI. Move
        // first, then apply the target monitor's physical size so logical-size
        // conversion cannot clip the WebView after a scale/display change.
        #[cfg(windows)]
        {
            super::windows_frame::refresh(&window)?;
            window.set_size(tauri::PhysicalSize::new(
                width.round() as u32,
                height.round() as u32,
            ))?;
        }
        log::info!(
            "resident_panel_positioned source={} anchor_source={anchor_source} anchor={anchor:?} scale={scale} x={x} y={y} width={width} height={height}",
            mangodisk_platform::diagnostics::text(&source)
        );
    }
    #[cfg(not(target_os = "macos"))]
    window.show()?;
    #[cfg(not(target_os = "macos"))]
    let _ = app.emit_to(PANEL_LABEL, VISIBILITY_EVENT, true);
    #[cfg(target_os = "macos")]
    {
        macos::show(app, native_placement);
    }
    #[cfg(target_os = "linux")]
    {
        use gtk::prelude::GtkWindowExt;

        let app = app.clone();
        let window = window.clone();
        // Tao queues GTK visibility changes and ignores focus while still hidden.
        // Run after those requests, restoring focusability before the first draw
        // of a prewarmed panel. A cancelled open must never reveal it again.
        gtk::glib::idle_add_local_once(move || {
            let state = app.state::<Arc<ResidentState>>();
            if !state.panel_open.load(Ordering::Relaxed) || !state.enabled() {
                return;
            }
            match window.gtk_window() {
                Ok(native) => {
                    native.set_accept_focus(true);
                    native.present();
                }
                Err(error) => {
                    hide(&app);
                    super::diagnostics::Failure::record("panel_focus", &error);
                }
            }
        });
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    window.set_focus()?;
    if let Some(started) = state
        .panel_requested_at
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .as_ref()
    {
        log::info!(
            "resident_panel_show_dispatched elapsed_ms={}",
            started.elapsed().as_millis()
        );
    }
    Ok(())
}

fn contains(point: (f64, f64), bounds: (f64, f64, f64, f64)) -> bool {
    point.0 >= bounds.0
        && point.0 < bounds.0 + bounds.2
        && point.1 >= bounds.1
        && point.1 < bounds.1 + bounds.3
}

fn monitor_contains_anchor(
    point: (f64, f64),
    bounds: (f64, f64, f64, f64),
    monitor_scale: f64,
    anchor_scale: Option<f64>,
) -> bool {
    anchor_scale.is_none_or(|scale| scale == monitor_scale) && contains(point, bounds)
}

/// Use physical coordinates consistently across Retina/non-Retina displays and taskbar edges.
fn position(
    anchor: (f64, f64),
    area: (f64, f64, f64, f64),
    size: (f64, f64),
    gap: f64,
) -> (f64, f64) {
    let (left, top, width, height) = area;
    // Side-taskbar anchors lie outside the work area horizontally. Center the
    // panel on the clicked metric while keeping the entire panel on the desktop.
    if (anchor.0 < left || anchor.0 >= left + width) && anchor.1 >= top && anchor.1 < top + height {
        let x = if anchor.0 < left {
            left + gap
        } else {
            left + width - size.0 - gap
        };
        let y = (anchor.1 - size.1 / 2.0).clamp(top, top + (height - size.1).max(0.0));
        return (x.clamp(left, left + (width - size.0).max(0.0)), y);
    }
    let x = (anchor.0 - size.0 / 2.0).clamp(left, left + (width - size.0).max(0.0));
    let y = if anchor.1 < top + height / 2.0 {
        top + gap
    } else {
        top + height - size.1 - gap
    };
    (x, y.clamp(top, top + (height - size.1).max(0.0)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_panel_joins_fullscreen_spaces_without_conflicting_collection_flags() {
        use objc2_app_kit::NSWindowCollectionBehavior as Behavior;

        let required =
            Behavior::Stationary | Behavior::CanJoinAllSpaces | Behavior::FullScreenAuxiliary;
        let conflicting = Behavior::Managed
            | Behavior::Transient
            | Behavior::MoveToActiveSpace
            | Behavior::FullScreenPrimary
            | Behavior::FullScreenNone;
        for joins_applications in [false, true] {
            for previous in [
                Behavior::Default,
                conflicting | Behavior::Primary,
                required | Behavior::Auxiliary,
                required | Behavior::CanJoinAllApplications,
            ] {
                let result =
                    macos_panel_behavior(previous | Behavior::IgnoresCycle, joins_applications);
                assert!(result.contains(required));
                assert!(!result.intersects(conflicting | Behavior::Primary | Behavior::Auxiliary));
                assert!(result.contains(Behavior::IgnoresCycle));
                assert_eq!(
                    result.contains(Behavior::CanJoinAllApplications),
                    joins_applications
                );
                assert_eq!(macos_panel_behavior(result, joins_applications), result);
            }
        }
    }

    #[test]
    fn macos_entry_press_preserves_toggle_but_keyboard_and_outside_blur_dismiss() {
        assert!(defer_macos_entry_blur(true, true));
        assert!(!defer_macos_entry_blur(true, false));
        assert!(!defer_macos_entry_blur(false, true));
        assert!(!defer_macos_entry_blur(false, false));
    }

    #[test]
    fn entry_press_preserves_toggle_intent_during_foreground_transition() {
        assert!(defer_entry_blur(false, true));
        assert!(defer_entry_blur(true, false));
        assert!(!defer_entry_blur(false, false));
    }

    #[test]
    fn reopening_remembers_the_tab_while_open_metric_shortcuts_still_navigate() {
        for previous in [
            PanelPage::Cpu,
            PanelPage::Memory,
            PanelPage::Power,
            PanelPage::Overview,
        ] {
            for requested in [
                None,
                Some(PanelPage::Cpu),
                Some(PanelPage::Memory),
                Some(PanelPage::Network),
            ] {
                assert_eq!(entry_selection(previous, requested, false), previous);
            }
        }
        assert_eq!(
            entry_selection(PanelPage::Memory, None, true),
            PanelPage::Memory
        );
        assert_eq!(
            entry_selection(PanelPage::Memory, Some(PanelPage::Cpu), true),
            PanelPage::Cpu
        );
        assert_eq!(
            entry_selection(PanelPage::Cpu, Some(PanelPage::Memory), true),
            PanelPage::Memory
        );
    }

    #[test]
    fn explicit_resource_entries_toggle_the_current_metric_and_select_it_on_reopening() {
        for metric in [
            PanelPage::Cpu,
            PanelPage::Gpu,
            PanelPage::Memory,
            PanelPage::Disk,
            PanelPage::Network,
        ] {
            let request = EntryRequest::Select(metric);
            for previous in [PanelPage::Cpu, PanelPage::Memory, PanelPage::Power] {
                assert_eq!(request.selection(previous, false), metric);
                assert_eq!(request.selection(previous, true), metric);
                assert_eq!(request.repeats(previous), metric == previous);
            }
        }
        // Tray entries retain their existing source-based toggle and remembered tab.
        let tray = EntryRequest::Resume(Some(PanelPage::Cpu));
        assert!(tray.repeats(PanelPage::Memory));
        assert_eq!(tray.selection(PanelPage::Memory, false), PanelPage::Memory);
        assert_eq!(tray.selection(PanelPage::Memory, true), PanelPage::Cpu);
    }

    #[test]
    fn batteryless_power_shortcuts_repeat_the_resolved_cpu_page() {
        let request = EntryRequest::Select(PanelPage::Power).available_for_battery(Some(false));
        assert_eq!(request.selection(PanelPage::Memory, false), PanelPage::Cpu);
        assert!(request.repeats(PanelPage::Cpu));
        assert!(!request.repeats(PanelPage::Memory));
        let tray = EntryRequest::Resume(Some(PanelPage::Power)).available_for_battery(Some(false));
        assert_eq!(tray.selection(PanelPage::Memory, true), PanelPage::Cpu);
        assert_eq!(
            EntryRequest::Resume(None)
                .selection(PanelPage::Power.available_for_battery(Some(false)), false),
            PanelPage::Cpu
        );
        for present in [None, Some(true)] {
            assert_eq!(
                EntryRequest::Select(PanelPage::Power)
                    .available_for_battery(present)
                    .selection(PanelPage::Cpu, false),
                PanelPage::Power
            );
        }
    }

    #[test]
    fn queued_native_hides_preserve_a_reopened_panel_but_the_next_outside_blur_dismisses() {
        let pending = std::sync::atomic::AtomicU32::new(2);
        // Two older hide/show cycles may deliver their blurs after the next open.
        assert!(ignore_queued_blur(&pending, false));
        assert!(ignore_queued_blur(&pending, false));
        // A stale blur cannot override the OS reporting this panel as focused.
        assert!(ignore_queued_blur(&pending, true));
        // The queue is drained: a real outside click must still dismiss it.
        assert!(!ignore_queued_blur(&pending, false));
        assert_eq!(pending.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn physical_monitor_selection_handles_retina_edges_and_negative_origins() {
        assert!(contains((2248.0, 24.0), (0.0, 0.0, 3840.0, 2160.0)));
        assert!(!contains((3840.0, 24.0), (0.0, 0.0, 3840.0, 2160.0)));
        assert!(contains((-100.0, 24.0), (-1920.0, 0.0, 1920.0, 1080.0)));
        assert!(!contains((0.0, -1.0), (0.0, 0.0, 3840.0, 2160.0)));
    }

    #[test]
    fn mixed_scale_tray_anchor_selects_the_screen_that_owns_the_entry() {
        // Tao and tray-icon scale global logical origins with each screen's
        // backing factor. Adjacent screens can therefore overlap in these pixels.
        let monitors = [
            ((0.0, 0.0, 3840.0, 2160.0), 2.0),
            ((1920.0, 0.0, 1920.0, 1080.0), 1.0),
        ];
        let selected = monitors.iter().position(|(bounds, scale)| {
            monitor_contains_anchor((2100.0, 12.0), *bounds, *scale, Some(1.0))
        });
        assert_eq!(selected, Some(1));
        assert!(monitor_contains_anchor(
            (2100.0, 24.0),
            monitors[0].0,
            2.0,
            Some(2.0)
        ));
    }

    #[test]
    fn tray_monitor_matching_preserves_negative_origins_and_unscaled_platforms() {
        let bounds = (-3840.0, -200.0, 3840.0, 2160.0);
        assert!(monitor_contains_anchor(
            (-100.0, 24.0),
            bounds,
            2.0,
            Some(2.0)
        ));
        assert!(!monitor_contains_anchor(
            (-100.0, 24.0),
            bounds,
            2.0,
            Some(1.0)
        ));
        assert!(!monitor_contains_anchor(
            (0.0, 24.0),
            bounds,
            2.0,
            Some(2.0)
        ));
        assert!(monitor_contains_anchor((-100.0, 24.0), bounds, 2.0, None));
    }

    #[test]
    fn side_taskbar_panels_open_inside_the_work_area_at_the_selected_metric() {
        assert_eq!(
            position(
                (40.0, 500.0),
                (80.0, 0.0, 1840.0, 1080.0),
                (390.0, 610.0),
                6.0
            ),
            (86.0, 195.0)
        );
        assert_eq!(
            position(
                (1880.0, 500.0),
                (0.0, 0.0, 1840.0, 1080.0),
                (390.0, 610.0),
                6.0
            ),
            (1444.0, 195.0)
        );
    }

    #[test]
    fn panel_fits_top_bottom_and_negative_origin_monitors() {
        assert_eq!(
            position(
                (1900.0, 10.0),
                (0.0, 24.0, 1920.0, 1056.0),
                (390.0, 610.0),
                6.0
            ),
            (1530.0, 30.0)
        );
        assert_eq!(
            position(
                (20.0, 1080.0),
                (0.0, 0.0, 1920.0, 1040.0),
                (390.0, 610.0),
                6.0
            ),
            (0.0, 424.0)
        );
        assert_eq!(
            position(
                (-10.0, 0.0),
                (-1920.0, 24.0, 1920.0, 1056.0),
                (780.0, 900.0),
                12.0
            ),
            (-780.0, 36.0)
        );
        assert_eq!(
            position((0.0, 0.0), (0.0, 0.0, 300.0, 400.0), (300.0, 400.0), 6.0),
            (0.0, 0.0)
        );
    }
}
