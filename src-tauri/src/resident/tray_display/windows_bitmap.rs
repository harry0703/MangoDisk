//! Render native-size anti-aliased masks, then tint without leaking GDI handles.
use std::ptr;
use windows_sys::{
    core::w,
    Win32::{
        Foundation::RECT,
        Graphics::Gdi::*,
        System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD},
        UI::{
            Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW},
            HiDpi::GetDpiForWindow,
            WindowsAndMessaging::{FindWindowW, SystemParametersInfoW, SPI_GETHIGHCONTRAST},
        },
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Appearance {
    pub size: u32,
    pub color: [u8; 3],
}

pub fn appearance() -> Appearance {
    unsafe {
        let taskbar = FindWindowW(w!("Shell_TrayWnd"), ptr::null());
        let dpi = GetDpiForWindow(taskbar).max(96);
        let size = (16 * dpi / 96).clamp(16, 64);
        let mut high: HIGHCONTRASTW = std::mem::zeroed();
        high.cbSize = std::mem::size_of::<HIGHCONTRASTW>() as u32;
        let contrast = SystemParametersInfoW(
            SPI_GETHIGHCONTRAST,
            high.cbSize,
            (&mut high as *mut HIGHCONTRASTW).cast(),
            0,
        ) != 0
            && high.dwFlags & HCF_HIGHCONTRASTON != 0;
        let mut light = 0u32;
        let mut length = 4u32;
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            w!("SystemUsesLightTheme"),
            RRF_RT_REG_DWORD,
            ptr::null_mut(),
            (&mut light as *mut u32).cast(),
            &mut length,
        );
        let color = if contrast {
            let color = GetSysColor(COLOR_WINDOWTEXT);
            [color as u8, (color >> 8) as u8, (color >> 16) as u8]
        } else if light != 0 {
            [32, 35, 40]
        } else {
            [245, 245, 245]
        };
        Appearance { size, color }
    }
}

struct Surface {
    dc: HDC,
    bitmap: HBITMAP,
    previous: HGDIOBJ,
}
impl Drop for Surface {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.previous);
            DeleteObject(self.bitmap);
            DeleteDC(self.dc);
        }
    }
}

fn draw(
    surface: &Surface,
    text: &str,
    size: u32,
    height: i32,
    top: i32,
    bottom: i32,
) -> Result<(), &'static str> {
    unsafe {
        let font = CreateFontW(
            -height,
            0,
            0,
            0,
            600,
            0,
            0,
            0,
            DEFAULT_CHARSET as u32,
            OUT_DEFAULT_PRECIS as u32,
            CLIP_DEFAULT_PRECIS as u32,
            ANTIALIASED_QUALITY as u32,
            DEFAULT_PITCH as u32,
            w!("Segoe UI"),
        );
        if font.is_null() {
            return Err("display_font");
        }
        let previous = SelectObject(surface.dc, font);
        let mut rect = RECT {
            left: 0,
            top,
            right: size as i32,
            bottom,
        };
        let mut value: Vec<u16> = text.encode_utf16().collect();
        let drawn = DrawTextW(
            surface.dc,
            value.as_mut_ptr(),
            value.len() as i32,
            &mut rect,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
        );
        SelectObject(surface.dc, previous);
        DeleteObject(font);
        if drawn == 0 {
            return Err("display_text");
        }
    }
    Ok(())
}

pub fn render(marker: &str, digits: &str, appearance: Appearance) -> Result<Vec<u8>, &'static str> {
    render_colored(
        marker,
        digits,
        appearance,
        super::usage_color::UsageTone::Normal,
    )
}

pub fn render_colored(
    marker: &str,
    digits: &str,
    appearance: Appearance,
    tone: super::usage_color::UsageTone,
) -> Result<Vec<u8>, &'static str> {
    let size = appearance.size;
    unsafe {
        let dc = CreateCompatibleDC(ptr::null_mut());
        if dc.is_null() {
            return Err("display_dc");
        }
        let mut info: BITMAPINFO = std::mem::zeroed();
        info.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        info.bmiHeader.biWidth = size as i32;
        info.bmiHeader.biHeight = -(size as i32);
        info.bmiHeader.biPlanes = 1;
        info.bmiHeader.biBitCount = 32;
        info.bmiHeader.biCompression = BI_RGB;
        let mut bits = ptr::null_mut();
        let bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, ptr::null_mut(), 0);
        if bitmap.is_null() || bits.is_null() {
            DeleteDC(dc);
            return Err("display_bitmap");
        }
        let surface = Surface {
            dc,
            bitmap,
            previous: SelectObject(dc, bitmap),
        };
        let count = (size * size * 4) as usize;
        ptr::write_bytes(bits, 0, count);
        SetBkMode(dc, TRANSPARENT as i32);
        SetTextColor(dc, 0x00ff_ffff);
        let split = (size * 5 / 16) as i32;
        draw(&surface, marker, size, split.max(5), 0, split)?;
        let height = (size * if digits.chars().count() > 2 { 9 } else { 11 } / 16) as i32;
        draw(&surface, digits, size, height, split, size as i32)?;
        GdiFlush();
        let mask = std::slice::from_raw_parts(bits.cast::<u8>(), count);
        let mut rgba = vec![0; count];
        for (index, (source, destination)) in mask
            .chunks_exact(4)
            .zip(rgba.chunks_exact_mut(4))
            .enumerate()
        {
            let color = if index / size as usize >= split as usize {
                tone.rgb(appearance.color)
            } else {
                appearance.color
            };
            destination[..3].copy_from_slice(&color);
            destination[3] = source[0].max(source[1]).max(source[2]);
        }
        Ok(rgba)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tray_percentage_tones_keep_markers_in_the_system_foreground() {
        for size in [16, 20, 24, 32, 48, 64] {
            for foreground in [[32; 3], [245; 3]] {
                for tone in [
                    super::super::usage_color::UsageTone::Warning,
                    super::super::usage_color::UsageTone::Critical,
                ] {
                    let pixels = render_colored(
                        "C",
                        "90",
                        Appearance {
                            size,
                            color: foreground,
                        },
                        tone,
                    )
                    .unwrap();
                    let split = (size * 5 / 16 * size * 4) as usize;
                    assert!(pixels[..split]
                        .chunks_exact(4)
                        .any(|p| p[3] > 0 && p[..3] == foreground));
                    assert!(pixels[split..]
                        .chunks_exact(4)
                        .any(|p| p[3] > 0 && p[..3] == tone.rgb(foreground)));
                }
            }
        }
    }

    #[test]
    fn icon_masks_have_transparency_and_visible_pixels_at_supported_sizes() {
        for size in [16, 20, 24, 32] {
            for digits in ["0", "100", "999", "—"] {
                let pixels = render(
                    "M",
                    digits,
                    Appearance {
                        size,
                        color: [255; 3],
                    },
                )
                .unwrap();
                assert_eq!(pixels.len(), (size * size * 4) as usize);
                assert!(pixels.chunks_exact(4).any(|pixel| pixel[3] > 0));
                assert!(pixels.chunks_exact(4).any(|pixel| pixel[3] == 0));
            }
        }
    }
}
