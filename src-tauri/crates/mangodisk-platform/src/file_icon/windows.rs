use std::{
    ffi::OsStr,
    fs,
    io::Cursor,
    mem::size_of,
    os::windows::ffi::OsStrExt,
    path::Path,
    ptr::{null, null_mut},
    time::UNIX_EPOCH,
};

use windows_sys::Win32::{
    Foundation::S_OK,
    Graphics::Gdi::{
        CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, SelectObject, BITMAPINFO,
        BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
    },
    Storage::FileSystem::{FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL},
    UI::{
        Shell::{
            AssocQueryStringW, SHGetFileInfoW, ASSOCF_NONE, ASSOCSTR_EXECUTABLE, SHFILEINFOW,
            SHGFI_ICON, SHGFI_LARGEICON, SHGFI_USEFILEATTRIBUTES,
        },
        WindowsAndMessaging::{DestroyIcon, DrawIconEx, DI_NORMAL, HICON},
    },
};

use super::IconQuery;

const ICON_PIXELS: i32 = 64;
const ASSOCIATION_BUFFER_CHARS: usize = 32_768;

/// Includes the current default handler in the type cache identity. Windows
/// updates file icons when associations change, so stale PNG files naturally
/// stop matching without clearing the entire cache.
pub(super) fn provider_variant(query: &IconQuery) -> Vec<u8> {
    let IconQuery::Type {
        extension: Some(extension),
        ..
    } = query
    else {
        return Vec::new();
    };
    let mut association = format!(".{extension}")
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let mut output = vec![0_u16; ASSOCIATION_BUFFER_CHARS];
    let mut output_len = output.len() as u32;
    // SAFETY: Both UTF-16 buffers are NUL-terminated and remain alive for the
    // complete call. `output_len` accurately describes the writable buffer.
    let status = unsafe {
        AssocQueryStringW(
            ASSOCF_NONE,
            ASSOCSTR_EXECUTABLE,
            association.as_mut_ptr(),
            null(),
            output.as_mut_ptr(),
            &mut output_len,
        )
    };
    if status != S_OK || output_len == 0 {
        return Vec::new();
    }

    let used = output_len.saturating_sub(1) as usize;
    let handler = String::from_utf16_lossy(&output[..used.min(output.len())]);
    let mut variant = handler.as_bytes().to_vec();
    append_path_metadata(&mut variant, Path::new(&handler));
    variant
}

pub(super) fn load_png(query: &IconQuery) -> Option<Vec<u8>> {
    let (path, attributes, use_file_attributes) = shell_query(query)?;
    let wide_path = OsStr::new(&path)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let mut info = SHFILEINFOW::default();
    let flags = SHGFI_ICON
        | SHGFI_LARGEICON
        | if use_file_attributes {
            SHGFI_USEFILEATTRIBUTES
        } else {
            0
        };
    // SAFETY: `wide_path` is NUL-terminated, `info` is a valid writable
    // structure, and the flags request an owned HICON documented for cleanup
    // with `DestroyIcon`.
    let result = unsafe {
        SHGetFileInfoW(
            wide_path.as_ptr(),
            attributes,
            &mut info,
            size_of::<SHFILEINFOW>() as u32,
            flags,
        )
    };
    if result == 0 || info.hIcon.is_null() {
        return None;
    }

    let png = rasterize_icon(info.hIcon);
    // SAFETY: SHGetFileInfoW returned this owned icon handle to the caller.
    unsafe {
        DestroyIcon(info.hIcon);
    }
    png
}

fn shell_query(query: &IconQuery) -> Option<(String, u32, bool)> {
    match query {
        IconQuery::Type { key, .. } if key == "kind:folder" => {
            Some(("folder".to_string(), FILE_ATTRIBUTE_DIRECTORY, true))
        }
        IconQuery::Type {
            extension: Some(extension),
            ..
        } => Some((format!("file.{extension}"), FILE_ATTRIBUTE_NORMAL, true)),
        IconQuery::Type { .. } => Some(("file".to_string(), FILE_ATTRIBUTE_NORMAL, true)),
        IconQuery::Path { path, .. } => {
            Some((path.to_str()?.to_string(), FILE_ATTRIBUTE_NORMAL, false))
        }
    }
}

fn rasterize_icon(icon: HICON) -> Option<Vec<u8>> {
    // Rendering against two known backgrounds reconstructs alpha without
    // mistaking opaque black outlines for transparent pixels. This is required
    // for legacy icons whose 32-bit color bitmap leaves every alpha byte at
    // zero and relies on the separate Windows icon mask.
    let black = render_icon_on_background(icon, 0)?;
    let white = render_icon_on_background(icon, 255)?;
    encode_png(&reconstruct_rgba(&black, &white))
}

fn render_icon_on_background(icon: HICON, background: u8) -> Option<Vec<u8>> {
    // A negative DIB height creates a top-down pixel buffer, avoiding a second
    // vertical flip before alpha reconstruction and PNG encoding.
    let bitmap_info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: ICON_PIXELS,
            biHeight: -ICON_PIXELS,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            ..BITMAPINFOHEADER::default()
        },
        ..BITMAPINFO::default()
    };

    // SAFETY: GDI receives initialized structures, and every successfully
    // created handle is restored or released before this function returns.
    unsafe {
        let device_context = CreateCompatibleDC(null_mut());
        if device_context.is_null() {
            return None;
        }
        let mut pixels = null_mut();
        let bitmap = CreateDIBSection(
            device_context,
            &bitmap_info,
            DIB_RGB_COLORS,
            &mut pixels,
            null_mut(),
            0,
        );
        if bitmap.is_null() || pixels.is_null() {
            if !bitmap.is_null() {
                DeleteObject(bitmap);
            }
            DeleteDC(device_context);
            return None;
        }
        let byte_len = (ICON_PIXELS * ICON_PIXELS * 4) as usize;
        std::ptr::write_bytes(pixels.cast::<u8>(), background, byte_len);
        let previous = SelectObject(device_context, bitmap);
        let drawn = DrawIconEx(
            device_context,
            0,
            0,
            icon,
            ICON_PIXELS,
            ICON_PIXELS,
            0,
            null_mut(),
            DI_NORMAL,
        ) != 0;

        let rendered =
            drawn.then(|| std::slice::from_raw_parts(pixels.cast::<u8>(), byte_len).to_vec());

        SelectObject(device_context, previous);
        DeleteObject(bitmap);
        DeleteDC(device_context);
        rendered
    }
}

fn reconstruct_rgba(black: &[u8], white: &[u8]) -> Vec<u8> {
    debug_assert_eq!(black.len(), white.len());
    let mut rgba = Vec::with_capacity(black.len());
    for (black_pixel, white_pixel) in black
        .as_chunks::<4>()
        .0
        .iter()
        .zip(white.as_chunks::<4>().0.iter())
    {
        // For a source channel C and alpha A:
        // black = A*C, white = A*C + (1-A)*255.
        // The largest channel delta is resilient to integer rounding in GDI.
        let background_delta = (0..3)
            .map(|index| white_pixel[index].saturating_sub(black_pixel[index]))
            .max()
            .unwrap_or(255);
        let alpha = 255_u8.saturating_sub(background_delta);
        if alpha == 0 {
            rgba.extend_from_slice(&[0, 0, 0, 0]);
            continue;
        }

        let unpremultiply = |channel: u8| {
            ((u16::from(channel) * 255 + u16::from(alpha) / 2) / u16::from(alpha)).min(255) as u8
        };
        rgba.extend_from_slice(&[
            unpremultiply(black_pixel[2]),
            unpremultiply(black_pixel[1]),
            unpremultiply(black_pixel[0]),
            alpha,
        ]);
    }
    rgba
}

fn encode_png(rgba: &[u8]) -> Option<Vec<u8>> {
    let mut output = Vec::new();
    {
        let mut encoder = png::Encoder::new(
            Cursor::new(&mut output),
            ICON_PIXELS as u32,
            ICON_PIXELS as u32,
        );
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().ok()?;
        writer.write_image_data(rgba).ok()?;
    }
    Some(output)
}

fn append_path_metadata(variant: &mut Vec<u8>, path: &Path) {
    let Ok(metadata) = fs::metadata(path) else {
        return;
    };
    variant.extend_from_slice(&metadata.len().to_le_bytes());
    if let Ok(modified) = metadata.modified() {
        if let Ok(duration) = modified.duration_since(UNIX_EPOCH) {
            variant.extend_from_slice(&duration.as_secs().to_le_bytes());
            variant.extend_from_slice(&duration.subsec_nanos().to_le_bytes());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::reconstruct_rgba;

    #[test]
    fn alpha_reconstruction_preserves_opaque_black_pixels() {
        let black_background = [0, 0, 0, 255, 0, 0, 0, 255];
        let white_background = [0, 0, 0, 255, 255, 255, 255, 255];

        assert_eq!(
            reconstruct_rgba(&black_background, &white_background),
            [0, 0, 0, 255, 0, 0, 0, 0]
        );
    }

    #[test]
    fn alpha_reconstruction_restores_translucent_colors() {
        // A 50% red pixel becomes BGRA (0, 0, 128) on black and
        // BGRA (127, 127, 255) on white.
        let black_background = [0, 0, 128, 255];
        let white_background = [127, 127, 255, 255];

        assert_eq!(
            reconstruct_rgba(&black_background, &white_background),
            [255, 0, 0, 128]
        );
    }
}
