//! Grayscale glyph coverage avoids ClearType color fringes on an unknown backdrop.
use super::{alpha, directwrite, presentation::Column, surface::Surface, text_layout};
use std::ptr;
use windows_sys::Win32::{
    Foundation::{HWND, POINT, SIZE},
    Graphics::Gdi::*,
    UI::WindowsAndMessaging::{UpdateLayeredWindow, ULW_ALPHA},
};

/// Stage and native code identify initialization, drawing, and presentation
/// failures without logging user content or repeating one warning every sample.
pub struct Failure {
    pub stage: &'static str,
    pub code: i32,
}
impl Failure {
    fn win32(stage: &'static str) -> Self {
        Self {
            stage,
            code: unsafe { windows_sys::Win32::Foundation::GetLastError() } as i32,
        }
    }
    fn directwrite(stage: &'static str, error: windows::core::Error) -> Self {
        Self {
            stage,
            code: error.code().0,
        }
    }
}

struct Frame {
    dc: HDC,
    bitmap: HBITMAP,
    previous: HGDIOBJ,
}
impl Drop for Frame {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.previous);
            DeleteObject(self.bitmap);
            DeleteDC(self.dc);
        }
    }
}

pub unsafe fn paint(
    renderer: &mut Option<directwrite::Renderer>,
    hwnd: HWND,
    columns: &[Column],
    surface: &Surface,
    dpi: u32,
    color: [u8; 3],
    hover: Option<usize>,
) -> Result<(), Failure> {
    let dc = CreateCompatibleDC(ptr::null_mut());
    if dc.is_null() {
        return Err(Failure::win32("transparent_dc"));
    }
    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: surface.width,
            biHeight: -surface.height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut bits = ptr::null_mut();
    let bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, ptr::null_mut(), 0);
    if bitmap.is_null() || bits.is_null() {
        let failure = Failure::win32("transparent_bitmap");
        if !bitmap.is_null() {
            DeleteObject(bitmap);
        }
        DeleteDC(dc);
        return Err(failure);
    }
    let _frame = Frame {
        dc,
        bitmap,
        previous: SelectObject(dc, bitmap),
    };
    let length = surface.width as usize * surface.height as usize * 4;
    // DirectWrite produces premultiplied colored glyphs directly. Never treat
    // gamma-adjusted RGB intensity from a white-on-black GDI mask as coverage.
    ptr::write_bytes(bits, 0, length);
    if renderer.is_none() {
        *renderer = Some(
            directwrite::Renderer::new()
                .map_err(|error| Failure::directwrite("directwrite_init", error))?,
        );
    }
    renderer
        .as_mut()
        .ok_or(Failure {
            stage: "directwrite_init",
            code: 0,
        })?
        .paint(
            dc,
            surface,
            &text_layout::runs(columns, surface, dpi),
            dpi,
            color,
        )
        .map_err(|error| Failure::directwrite("directwrite_paint", error))?;
    GdiFlush();
    let pixels = std::slice::from_raw_parts_mut(bits.cast::<u8>(), length);
    let hovered = hover.and_then(|index| surface.cells.get(index));
    for (index, pixel) in pixels.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        let x = (index % surface.width as usize) as i32;
        let y = (index / surface.width as usize) as i32;
        let glyph = [pixel[0], pixel[1], pixel[2], pixel[3]];
        pixel.copy_from_slice(&alpha::over_background(
            glyph,
            color,
            hovered.is_some_and(|cell| cell.contains(x, y)),
        ));
    }
    let size = SIZE {
        cx: surface.width,
        cy: surface.height,
    };
    let source = POINT::default();
    let blend = BLENDFUNCTION {
        BlendOp: AC_SRC_OVER as u8,
        BlendFlags: 0,
        SourceConstantAlpha: 255,
        AlphaFormat: AC_SRC_ALPHA as u8,
    };
    if UpdateLayeredWindow(
        hwnd,
        ptr::null_mut(),
        ptr::null(),
        &size,
        dc,
        &source,
        0,
        &blend,
        ULW_ALPHA,
    ) == 0
    {
        return Err(Failure::win32("transparent_present"));
    }
    Ok(())
}
