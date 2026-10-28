//! Native painting and visibility retain their failure recovery on the native window thread.
use super::super::{transparent, visibility::Visibility, DisplayStatus};
use super::Window;
use windows_sys::Win32::{Foundation::*, UI::WindowsAndMessaging::*};

impl Window {
    pub(super) unsafe fn paint_transparent(&mut self, hwnd: HWND) -> bool {
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

    pub(super) unsafe fn record_visibility(&mut self, state: Visibility) {
        if self.visibility != Some(state) {
            let foreground = self.foreground.hwnd as HWND;
            let mut frame = RECT::default();
            let has_frame = GetWindowRect(foreground, &mut frame) != 0;
            let frame_code = if has_frame { 0 } else { GetLastError() };
            log::info!(
                "resident_taskbar_visibility previous={:?} current={state:?} {} style={:#x} maximized={} frame_available={has_frame} frame_code={frame_code} frame=({},{},{},{}) bounds={:?}",
                self.visibility,
                self.foreground.diagnostic(),
                GetWindowLongPtrW(foreground, GWL_STYLE),
                IsZoomed(foreground) != 0,
                frame.left, frame.top, frame.right, frame.bottom,
                self.bounds
            );
            self.visibility = Some(state);
        }
    }

    pub(super) unsafe fn hide(&mut self, hwnd: HWND, reason: Visibility) {
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
}
