//! Short-lived, non-activating display numbers never alter Explorer or its layout.
use std::{
    ptr,
    time::{Duration, Instant},
};
use windows_sys::{
    core::w,
    Win32::{
        Foundation::*,
        Graphics::Gdi::*,
        UI::{HiDpi::*, WindowsAndMessaging::*},
    },
};

pub(super) fn show() -> Result<(), String> {
    // Enumerate before spawning so discovery failures remain actionable in settings.
    let screens = super::monitors_windows::enumerate()
        .map_err(|code| format!("taskbar_identify stage=enumerate code={code}"))?;
    std::thread::spawn(move || unsafe {
        let _context = super::hosting::DpiContext::physical();
        let mut windows = Vec::new();
        for screen in screens {
            let monitor = MonitorFromRect(
                &RECT {
                    left: screen.bounds.left,
                    top: screen.bounds.top,
                    right: screen.bounds.right,
                    bottom: screen.bounds.bottom,
                },
                MONITOR_DEFAULTTONULL,
            );
            let mut dpi = 96;
            let mut y_dpi = 96;
            let _ = GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi, &mut y_dpi);
            let size = (160 * dpi / 96) as i32;
            let label: Vec<u16> = screen
                .info
                .number
                .to_string()
                .encode_utf16()
                .chain(Some(0))
                .collect();
            // STATIC uses SS_CENTER | SS_CENTERIMAGE for numeric identification.
            let hwnd = CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                w!("STATIC"),
                label.as_ptr(),
                WS_POPUP | WS_VISIBLE | 0x0001 | 0x0200,
                screen.bounds.left + (screen.bounds.width() - size) / 2,
                screen.bounds.top + (screen.bounds.height() - size) / 2,
                size,
                size,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null(),
            );
            if hwnd.is_null() {
                log::warn!(
                    "resident_taskbar_identify_failed number={} stage=create_window code={}",
                    screen.info.number,
                    GetLastError()
                );
                continue;
            }
            let mut font = LOGFONTW {
                lfHeight: -(dpi as i32),
                lfWeight: 700,
                ..Default::default()
            };
            for (target, value) in font.lfFaceName.iter_mut().zip("Segoe UI".encode_utf16()) {
                *target = value;
            }
            let font = CreateFontIndirectW(&font);
            SendMessageW(hwnd, WM_SETFONT, font as usize, 1);
            windows.push((hwnd, font));
        }
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(3) {
            let mut message = MSG::default();
            while PeekMessageW(&mut message, ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
            MsgWaitForMultipleObjects(0, ptr::null(), 0, 50, QS_ALLINPUT);
        }
        for (window, font) in windows {
            DestroyWindow(window);
            if !font.is_null() {
                DeleteObject(font);
            }
        }
    });
    Ok(())
}
