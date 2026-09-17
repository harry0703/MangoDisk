//! Real Windows raster checks; optional BMP evidence stays outside production source.
use super::*;
use crate::resident::{
    taskbar_display::{alpha, draw, layout::Bounds, presentation, text_layout},
    tray_display::format::{DisplayEntry, DisplayId},
};
use windows_sys::Win32::Graphics::Gdi::*;

struct Bitmap {
    dc: windows_sys::Win32::Graphics::Gdi::HDC,
    bitmap: HBITMAP,
    old: HGDIOBJ,
    bits: *mut u8,
    length: usize,
}
impl Bitmap {
    unsafe fn new(width: i32, height: i32) -> Self {
        let dc = CreateCompatibleDC(std::ptr::null_mut());
        assert!(!dc.is_null());
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        let bitmap = CreateDIBSection(
            dc,
            &info,
            DIB_RGB_COLORS,
            &mut bits,
            std::ptr::null_mut(),
            0,
        );
        assert!(!bitmap.is_null() && !bits.is_null());
        let length = (width * height * 4) as usize;
        std::ptr::write_bytes(bits, 0, length);
        Self {
            dc,
            bitmap,
            old: SelectObject(dc, bitmap),
            bits: bits.cast(),
            length,
        }
    }
    unsafe fn pixels(&mut self) -> &mut [u8] {
        GdiFlush();
        std::slice::from_raw_parts_mut(self.bits, self.length)
    }
}
impl Drop for Bitmap {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.old);
            DeleteObject(self.bitmap);
            DeleteDC(self.dc);
        }
    }
}

#[test]
fn native_grayscale_preserves_alpha_colors_and_field_fit_across_dpi() {
    unsafe {
        let mut renderer = Renderer::new().unwrap();
        for dpi in [96, 120, 144, 168, 192, 240, 288] {
            for compact in [false, true] {
                let entries: Vec<_> = [
                    (DisplayId::Memory, "MEM 18%", "18"),
                    (DisplayId::Cpu, "CPU 100%", "100"),
                    (DisplayId::Upload, "↑999.9 TB/s", "999.9"),
                    (DisplayId::Download, "↓ 99.9 MB/s", "99.9"),
                    (DisplayId::Disk, "DISK 0%", "0"),
                ]
                .into_iter()
                .map(|(id, text, digits)| DisplayEntry {
                    tone: match id {
                        DisplayId::Cpu => {
                            crate::resident::tray_display::usage_color::UsageTone::Critical
                        }
                        DisplayId::Memory => {
                            crate::resident::tray_display::usage_color::UsageTone::Warning
                        }
                        _ => Default::default(),
                    },
                    usage_percent: None,
                    id,
                    text: text.into(),
                    digits: digits.into(),
                    marker: String::new(),
                    tooltip: String::new(),
                })
                .collect();
                let columns = presentation::columns(&entries, dpi, compact);
                let surface = Surface::arrange(
                    &columns,
                    Bounds {
                        left: 0,
                        top: 0,
                        right: 3840,
                        bottom: (48 * dpi / 96) as i32,
                    },
                    dpi,
                )
                .unwrap();
                let runs = text_layout::runs(&columns, &surface, dpi);
                for foreground in [[32, 32, 32], [240, 240, 240]] {
                    let mut frame = Bitmap::new(surface.width, surface.height);
                    renderer
                        .paint(frame.dc, &surface, &runs, dpi, foreground)
                        .unwrap();
                    let pixels = frame.pixels();
                    assert!(
                        pixels.chunks_exact(4).any(|p| p[3] > 0 && p[3] < 255),
                        "missing antialiased edges dpi={dpi}"
                    );
                    assert!(
                        pixels.chunks_exact(4).any(|p| p[3] == 0),
                        "transparent background lost dpi={dpi}"
                    );
                    assert!(
                        pixels
                            .chunks_exact(4)
                            .all(|p| p[..3].iter().all(|c| *c <= p[3])),
                        "invalid premultiplied alpha dpi={dpi}"
                    );
                    for tone in [
                        crate::resident::tray_display::usage_color::UsageTone::Warning,
                        crate::resident::tray_display::usage_color::UsageTone::Critical,
                    ] {
                        let [r, g, b] = tone.rgb(foreground);
                        assert!(
                            pixels.chunks_exact(4).any(|p| p == [b, g, r, 255]),
                            "missing usage color dpi={dpi} tone={tone:?}"
                        );
                    }
                    for run in &runs {
                        let layout = renderer
                            .write
                            .CreateTextLayout(
                                &run.text.encode_utf16().collect::<Vec<_>>(),
                                renderer.format(run.style),
                                1000.0,
                                1000.0,
                            )
                            .unwrap();
                        let mut metrics = DWRITE_TEXT_METRICS::default();
                        layout.GetMetrics(&mut metrics).unwrap();
                        assert!(
                            metrics.width <= run.bounds.width() as f32,
                            "text={} dpi={dpi} width={} field={}",
                            run.text,
                            metrics.width,
                            run.bounds.width()
                        );
                    }
                    if let Ok(directory) = std::env::var("MANGODISK_TEXT_EVIDENCE") {
                        std::fs::create_dir_all(&directory).unwrap();
                        let bg = if foreground[0] < 128 {
                            [225, 238, 246]
                        } else {
                            [37, 37, 37]
                        };
                        let theme = if foreground[0] < 128 { "light" } else { "dark" };
                        let mut rows = Vec::new();
                        // Row 1 is the previous GDI coverage conversion, row 2 is
                        // DirectWrite, row 3 is the existing opaque ClearType reference.
                        let mut old = Bitmap::new(surface.width, surface.height);
                        assert!(draw::text(old.dc, &runs, dpi, None, ANTIALIASED_QUALITY));
                        for (index, pixel) in old.pixels().chunks_exact(4).enumerate() {
                            let x = (index % surface.width as usize) as i32;
                            let y = (index / surface.width as usize) as i32;
                            let color = runs
                                .iter()
                                .find(|run| run.bounds.contains(x, y))
                                .map(|run| run.ink.rgb(foreground))
                                .unwrap_or(foreground);
                            rows.extend(composite(
                                alpha::pixel(pixel[0].max(pixel[1]).max(pixel[2]), color, false),
                                bg,
                            ));
                        }
                        for pixel in pixels.chunks_exact(4) {
                            rows.extend(composite([pixel[0], pixel[1], pixel[2], pixel[3]], bg));
                        }
                        for pixel in old.pixels().chunks_exact_mut(4) {
                            pixel.copy_from_slice(&[bg[2], bg[1], bg[0], 255]);
                        }
                        assert!(draw::text(
                            old.dc,
                            &runs,
                            dpi,
                            Some(foreground),
                            CLEARTYPE_QUALITY
                        ));
                        rows.extend_from_slice(old.pixels());
                        save_bmp(
                            &std::path::Path::new(&directory)
                                .join(format!("dpi-{dpi}-{compact}-{theme}.bmp")),
                            surface.width,
                            surface.height * 3,
                            &rows,
                        );
                    }
                }
                let dc = Bitmap::new(surface.width, surface.height);
                let start = std::time::Instant::now();
                for _ in 0..100 {
                    assert!(draw::text(dc.dc, &runs, dpi, None, ANTIALIASED_QUALITY));
                    GdiFlush();
                }
                eprintln!(
                    "gdi_text dpi={dpi} compact={compact} average_us={}",
                    start.elapsed().as_micros() / 100
                );
                let start = std::time::Instant::now();
                for _ in 0..100 {
                    renderer
                        .paint(dc.dc, &surface, &runs, dpi, [32; 3])
                        .unwrap();
                }
                eprintln!(
                    "directwrite_frame dpi={dpi} compact={compact} average_us={}",
                    start.elapsed().as_micros() / 100
                );
            }
        }
    }
}

fn composite(pixel: [u8; 4], background: [u8; 3]) -> [u8; 4] {
    let mut result = [0, 0, 0, 255];
    for i in 0..3 {
        result[i] = (u32::from(pixel[i])
            + (u32::from(background[2 - i]) * (255 - u32::from(pixel[3])) + 127) / 255)
            as u8;
    }
    result
}

fn save_bmp(path: &std::path::Path, width: i32, height: i32, pixels: &[u8]) {
    let mut data = Vec::new();
    data.extend(b"BM");
    data.extend((54 + pixels.len() as u32).to_le_bytes());
    data.extend([0; 4]);
    data.extend(54u32.to_le_bytes());
    data.extend(40u32.to_le_bytes());
    data.extend(width.to_le_bytes());
    data.extend((-height).to_le_bytes());
    data.extend(1u16.to_le_bytes());
    data.extend(32u16.to_le_bytes());
    data.extend([0; 24]);
    data.extend(pixels);
    std::fs::write(path, data).unwrap();
}
