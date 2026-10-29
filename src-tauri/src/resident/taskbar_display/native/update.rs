//! Reconcile sampled geometry, live shell visibility and reversible layout leases.
use super::super::{
    hosting,
    layout::{self, Bounds},
    peers,
    position::{self, Environment},
    presentation, reservation,
    surface::Surface,
    visibility::{self, Visibility},
    DisplayStatus,
};
use super::{
    foreground::{FullscreenCandidate, Identity, ShellSurface},
    Window,
};
use crate::resident::tray_display::windows_bitmap;
use std::{ptr, sync::atomic::Ordering, time::Duration};
use windows_sys::{
    core::w,
    Win32::{
        Foundation::*,
        Graphics::Gdi::*,
        UI::{HiDpi::*, WindowsAndMessaging::*},
    },
};

impl Window {
    pub(super) fn release_reservation(&mut self) {
        if let Some(client) = self.reservation.take() {
            self.releasing = Some(client.release());
        }
    }

    pub(super) unsafe fn update(&mut self, hwnd: HWND, force: bool) {
        let requested = self.service.requested.load(Ordering::Relaxed);
        let foreground = GetForegroundWindow();
        if (requested || self.visibility != Some(Visibility::Disabled))
            && !self.foreground.matches(foreground, self.shell)
        {
            self.foreground = Identity::read(foreground, self.shell);
        }
        if !requested {
            self.release_reservation();
            self.hide(hwnd, Visibility::Disabled);
            self.service.publish(DisplayStatus::Tray);
            return;
        }
        if !super::super::monitors_windows::is_taskbar(self.shell)
            || GetParent(hwnd) != self.parent
            || IsWindow(self.parent) == 0
        {
            DestroyWindow(hwnd);
            return;
        }
        let peers = peers::inspect(self.parent);
        let geometry = self
            .service
            .geometry
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if geometry
            .as_ref()
            .is_some_and(|g| !peers.allows_reservation(g.environment))
        {
            self.release_reservation();
        }
        // The sampler may still hold the old shell snapshot after a restart.
        // Wait for fresh geometry instead of repeatedly destroying the new child.
        let requested_monitor = self
            .service
            .model
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .monitor
            .clone();
        if geometry.as_ref().is_some_and(|g| {
            g.requested_monitor == requested_monitor && g.shell as HWND != self.shell
        }) {
            self.release_reservation();
            DestroyWindow(hwnd);
            return;
        }
        let Some(geometry) = geometry
            .filter(|g| g.shell as HWND == self.shell && g.requested_monitor == requested_monitor)
        else {
            self.release_reservation();
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
        let monitor_bounds = geometry.monitor_bounds;
        let inside = bounds.fits_in(monitor_bounds);
        let mut front = RECT::default();
        let front_available = !foreground.is_null() && GetWindowRect(foreground, &mut front) != 0;
        let front_bounds = Bounds {
            left: front.left,
            top: front.top,
            right: front.right,
            bottom: front.bottom,
        };
        let style = GetWindowLongPtrW(foreground, GWL_STYLE);
        let maximized = IsZoomed(foreground) != 0;
        let fullscreen = front_available
            && visibility::is_fullscreen(
                front_bounds,
                monitor_bounds,
                style & (WS_DLGFRAME | WS_THICKFRAME) as isize != 0,
                maximized,
            );
        let covered = foreground != GetShellWindow()
            && self.foreground.surface == ShellSurface::Application
            && foreground != hwnd
            && foreground != shell
            && !foreground.is_null()
            && fullscreen;
        let candidate = fullscreen.then_some(FullscreenCandidate {
            hwnd: foreground as usize,
            process_id: self.foreground.process_id,
            window: front_bounds,
            monitor: monitor_bounds,
            style,
            maximized,
            surface: self.foreground.surface,
        });
        if candidate != self.fullscreen_candidate {
            if let Some(candidate) = &candidate {
                log::info!(
                    "resident_taskbar_fullscreen_candidate {} window={:?} monitor={:?} style={style:#x} maximized={maximized} dpi={} fullscreen_hide={covered}",
                    self.foreground.diagnostic(), candidate.window, candidate.monitor, geometry.dpi,
                );
            }
            self.fullscreen_candidate = candidate;
        }
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
        let Some(parent_bounds) = hosting::client_bounds(self.parent) else {
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
        let visible_host = hosting::visible_bounds(shell, parent_bounds).and_then(|bounds| {
            if self.parent == shell {
                Some(bounds)
            } else {
                hosting::visible_bounds(self.parent, bounds)
            }
        });
        let Some(visible_host) = visible_host else {
            self.hide(hwnd, Visibility::ShellMoving);
            self.service.publish(DisplayStatus::Taskbar);
            return;
        };
        // Keep shell/foreground visibility, collision and live host checks at 100 ms.
        // Stable layout does not need columns, surface allocation or a companion
        // exchange at that rate. Publications and shell events bypass this gate.
        let inputs = (geometry.sampled, parent_bounds, visible_host, peers.clone());
        if !force
            && self.visible
            && !self.paint_failed
            && self.layout_inputs.as_ref() == Some(&inputs)
            && self.layout_checked.elapsed() < Duration::from_secs(1)
        {
            return;
        }
        self.layout_inputs = Some(inputs);
        self.layout_checked = std::time::Instant::now();
        let model = self
            .service
            .model
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let columns = presentation::columns(&model.entries, geometry.dpi, model.compact);
        let placement_host = layout::content_band(visible_host, geometry.content);
        let content_band = layout::content_band(geometry.bar, Some(placement_host));
        let Some(surface) = Surface::arrange(&columns, content_band, geometry.dpi) else {
            self.release_reservation();
            self.hide(hwnd, Visibility::UnsupportedLayout);
            self.service.publish(DisplayStatus::UnsupportedLayout);
            return;
        };
        let edge = position::resolve(model.position, geometry.environment);
        let policy = (model.position, geometry.environment, edge, peers.present);
        if self.placement_policy != Some(policy) {
            log::info!(
                "resident_taskbar_position requested={:?} environment={:?} resolved={:?} shared_host={}",
                model.position,
                geometry.environment,
                edge,
                peers.present
            );
            self.placement_policy = Some(policy);
        }
        let gap = (4 * geometry.dpi / 96) as i32;
        let peers_allow_reservation = peers.allows_reservation(geometry.environment);
        let mut occupied = geometry.occupied.clone();
        occupied.extend(peers.occupied);
        let find_gap = || {
            layout::place(
                placement_host,
                &occupied,
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
        // The embedded XAML adapter currently owns only the primary island.
        // Secondary taskbars use collision-checked free space and must never
        // reserve a slot in the primary taskbar on behalf of another display.
        let reservation_supported = shell == FindWindowW(w!("Shell_TrayWnd"), ptr::null())
            && peers_allow_reservation
            && hosting::reservation_visible(shell, self.parent, parent_bounds, placement_host)
            && match geometry.environment {
                Environment::Unknown => false,
                Environment::Windows10 => hosting::task_list(self.parent).is_some(),
                Environment::Windows11Centered | Environment::Windows11LeftAligned => *self
                    .xaml_architecture_matches
                    .get_or_insert_with(|| hosting::xaml_architecture_matches(shell)),
            };
        // Cross-architecture XAML cannot reserve space, but the embedded child
        // can still use a collision-checked gap, just like an unknown shell.
        if !reservation_supported {
            self.release_reservation();
        }
        // EOF and restoration happen on the worker. Neither gap placement nor
        // a replacement lease may use a snapshot from before that worker exits.
        if let Some(release) = self
            .releasing
            .as_ref()
            .filter(|r| !r.accepts(geometry.sampled))
        {
            let stalled = release.is_stalled();
            self.hide(
                hwnd,
                if stalled {
                    Visibility::ShellUnavailable
                } else {
                    Visibility::ShellMoving
                },
            );
            self.service.publish(if stalled {
                DisplayStatus::ShellUnavailable
            } else {
                DisplayStatus::Taskbar
            });
            return;
        }
        self.releasing = None;
        let placed = if !reservation_supported {
            find_gap()
        } else {
            if self.reservation.is_none() && std::time::Instant::now() >= self.reservation_retry {
                match reservation::Client::start(&self.service.app) {
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
                client.request(reservation::Request::new(
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
                Some(Err(error)) if error.stage == reservation::Stage::SharedHost => {
                    self.release_reservation();
                    self.hide(hwnd, Visibility::ShellMoving);
                    self.service.publish(DisplayStatus::Taskbar);
                    return;
                }
                Some(Err(error)) if error.stage != reservation::Stage::Geometry => {
                    if self.reservation_failed != Some(error) {
                        log::warn!(
                            "resident_taskbar_reservation_failed stage={:?} code={}",
                            error.stage,
                            error.code
                        );
                    }
                    self.reservation_failed = Some(error);
                    if error.stage == reservation::Stage::Space {
                        self.hide(hwnd, Visibility::NoSpace);
                        self.service.publish(DisplayStatus::NoSpace);
                        return;
                    }
                    if matches!(
                        error.stage,
                        reservation::Stage::Channel | reservation::Stage::Protocol
                    ) {
                        self.release_reservation();
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
        let Some(bounds) =
            placed.and_then(|bounds| layout::align_cross_axis(bounds, placement_host))
        else {
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
        if !hosting::region_contains(shell, bounds)
            || (self.parent != shell && !hosting::region_contains(self.parent, bounds))
        {
            if self.visibility != Some(Visibility::UnsupportedLayout) {
                log::info!("resident_taskbar_clipped bounds={bounds:?} bar={:?} host={parent_bounds:?} visible_host={visible_host:?} outcome=hidden", geometry.bar);
            }
            self.hide(hwnd, Visibility::UnsupportedLayout);
            self.service.publish(DisplayStatus::UnsupportedLayout);
            return;
        }
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
                "resident_taskbar_layout vertical={} preference={:?} compact={} width={} height={} left={} top={} dpi={} bar={:?} host={parent_bounds:?} visible_host={visible_host:?} content={:?} placement_host={placement_host:?}",
                surface.vertical,
                model.position,
                model.compact,
                surface.width,
                surface.height,
                bounds.left,
                bounds.top,
                geometry.dpi,
                geometry.bar,
                geometry.content
            );
        }
        if self.background != model.background {
            if let Err(code) = hosting::background(hwnd, model.background) {
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
            if layout_changed || restoring {
                let mut actual = RECT::default();
                if GetWindowRect(hwnd, &mut actual) != 0 {
                    log::info!(
                    "resident_taskbar_positioned requested={bounds:?} actual=({},{},{},{}) parent={:?} dpi={}",
                    actual.left,
                    actual.top,
                    actual.right,
                    actual.bottom,
                    self.parent,
                    GetDpiForWindow(hwnd)
                );
                } else {
                    log::warn!(
                        "resident_taskbar_position_inspection_failed code={} parent={:?}",
                        GetLastError(),
                        self.parent
                    );
                }
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
}
