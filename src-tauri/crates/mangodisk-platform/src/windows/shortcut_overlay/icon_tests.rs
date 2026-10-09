use super::TRANSPARENT_ICON;

#[test]
fn overlay_png_preserves_low_nonzero_alpha_without_visible_artwork() {
    let data = TRANSPARENT_ICON;
    assert_eq!(&data[..6], &[0, 0, 1, 0, 1, 0]);
    assert_eq!(&data[6..14], &[0, 0, 0, 0, 1, 0, 32, 0]);
    let length = u32::from_le_bytes(data[14..18].try_into().unwrap()) as usize;
    let offset = u32::from_le_bytes(data[18..22].try_into().unwrap()) as usize;
    assert_eq!(offset, 22);
    assert_eq!(offset + length, data.len());
    let mut reader = png::Decoder::new(std::io::Cursor::new(&data[offset..]))
        .read_info()
        .expect("overlay PNG must decode");
    let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut pixels).unwrap();
    assert_eq!((info.width, info.height), (256, 256));
    assert_eq!(info.color_type, png::ColorType::Rgba);
    assert_eq!(info.bit_depth, png::BitDepth::Eight);
    assert!(pixels[..info.buffer_size()]
        .as_chunks::<4>()
        .0
        .iter()
        .all(|pixel| *pixel == [0, 0, 0, 1]));
}

#[cfg(windows)]
#[test]
fn windows_loads_and_blends_overlay_at_shell_icon_sizes() {
    use std::{ffi::OsStr, os::windows::ffi::OsStrExt, ptr};
    use windows_sys::Win32::{
        Graphics::Gdi::{
            CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GdiFlush, SelectObject,
            BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
        },
        UI::WindowsAndMessaging::{
            DestroyIcon, DrawIconEx, LoadImageW, DI_NORMAL, IMAGE_ICON, LR_LOADFROMFILE,
        },
    };

    let directory = std::env::temp_dir().join(format!(
        "mangodisk-overlay-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&directory).unwrap();
    let path = directory.join("overlay.ico");
    std::fs::write(&path, TRANSPARENT_ICON).unwrap();
    let wide: Vec<u16> = OsStr::new(&path).encode_wide().chain(Some(0)).collect();

    for size in [16, 24, 32, 48, 64, 96, 128, 256] {
        // SAFETY: each GDI object stays alive until it is deselected and destroyed. The DIB
        // allocation contains size*size BGRA pixels; GdiFlush precedes reading its memory.
        unsafe {
            let icon = LoadImageW(
                ptr::null_mut(),
                wide.as_ptr(),
                IMAGE_ICON,
                size,
                size,
                LR_LOADFROMFILE,
            );
            assert!(!icon.is_null(), "native icon loading failed at {size}px");
            let dc = CreateCompatibleDC(ptr::null_mut());
            assert!(!dc.is_null());
            let info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: size,
                    biHeight: -size,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits = ptr::null_mut();
            let bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, ptr::null_mut(), 0);
            assert!(!bitmap.is_null());
            let previous = SelectObject(dc, bitmap);
            let pixels =
                std::slice::from_raw_parts_mut(bits.cast::<u8>(), (size * size * 4) as usize);
            for background in [64_u8, 255] {
                pixels.fill(background);
                let drawn = DrawIconEx(dc, 0, 0, icon, size, size, 0, ptr::null_mut(), DI_NORMAL);
                GdiFlush();
                let min = pixels
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .flat_map(|pixel| &pixel[..3])
                    .min()
                    .copied();
                let max = pixels
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .flat_map(|pixel| &pixel[..3])
                    .max()
                    .copied();
                assert_ne!(drawn, 0, "native drawing failed at {size}px");
                assert!(
                    min.is_some_and(|v| v >= background - 1),
                    "opaque overlay at {size}px"
                );
                assert!(
                    max.is_some_and(|v| v <= background),
                    "unexpected artwork at {size}px"
                );
            }
            SelectObject(dc, previous);
            DeleteObject(bitmap);
            DeleteDC(dc);
            DestroyIcon(icon);
        }
    }
    std::fs::remove_file(&path).unwrap();
    std::fs::remove_dir(&directory).unwrap();
}
