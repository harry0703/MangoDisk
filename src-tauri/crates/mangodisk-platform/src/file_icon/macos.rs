use std::{
    fs,
    path::Path,
    sync::{Mutex, MutexGuard, Once},
    time::{Instant, UNIX_EPOCH},
};

use objc2::{
    rc::{autoreleasepool, Retained},
    runtime::AnyObject,
    AnyThread, Message,
};
use objc2_app_kit::{
    NSBitmapImageFileType, NSBitmapImageRep, NSCalibratedRGBColorSpace, NSCompositingOperation,
    NSGraphicsContext, NSImage, NSImageInterpolation, NSWorkspace,
};
use objc2_foundation::{NSDictionary, NSPoint, NSRect, NSSize, NSString};
use objc2_uniform_type_identifiers::{UTType, UTTypeApplicationBundle, UTTypeData, UTTypeFolder};

use super::IconQuery;

// Covers the application's icons up to 64 logical pixels at 2x (and 40 at 3x).
// Keep this in the provider identity so old, oversized PNGs are never reused.
const ICON_PIXELS: isize = 128;
const RASTER_VARIANT: &[u8] = b"macos-raster-128-v2";
static EXTRACTION_GATE: Mutex<()> = Mutex::new(());

/// Warm-up and WebViews share the disk cache. Recheck it under this gate before
/// extraction so simultaneous misses cannot decode the same AppKit asset twice.
/// Only the background icon service takes this lock, never the UI thread.
pub(super) fn extraction_guard() -> MutexGuard<'static, ()> {
    EXTRACTION_GATE
        .lock()
        .unwrap_or_else(|error| error.into_inner())
}

/// Captures the default application behind a file type. macOS can change the
/// icon shown by Finder after a user changes the default opener, so this value
/// participates in the persistent cache key alongside the raster format.
pub(super) fn provider_variant(query: &IconQuery) -> Vec<u8> {
    autoreleasepool(|_| {
        let mut variant = RASTER_VARIANT.to_vec();
        variant.extend_from_slice(&provider_variant_in_pool(query));
        variant
    })
}

fn provider_variant_in_pool(query: &IconQuery) -> Vec<u8> {
    let IconQuery::Type { .. } = query else {
        return Vec::new();
    };
    let Some(content_type) = content_type(query) else {
        return Vec::new();
    };

    let workspace = NSWorkspace::sharedWorkspace();
    let mut variant = content_type.identifier().to_string().into_bytes();
    if let Some(application_url) = workspace.URLForApplicationToOpenContentType(&content_type) {
        if let Some(path) = application_url.path() {
            let path = path.to_string();
            variant.extend_from_slice(path.as_bytes());
            append_path_metadata(&mut variant, Path::new(&path));
        }
    }
    variant
}

/// Uses the same AppKit workspace APIs that Finder relies on. Type queries do
/// not require touching a real document, while path queries preserve bundle,
/// volume, and custom-folder icons that cannot safely share an extension key.
pub(super) fn load_png(query: &IconQuery) -> Option<Vec<u8>> {
    static ANNOUNCED: Once = Once::new();
    ANNOUNCED.call_once(|| {
        log::info!("file_icon_rasterizer_ready platform=macos pixels={ICON_PIXELS} cache_variant=macos-raster-128-v2");
    });
    let started = Instant::now();
    let result = autoreleasepool(|_| load_png_in_pool(query));
    match result {
        Ok(png) => {
            if started.elapsed().as_millis() >= 250 {
                log::info!(
                    "file_icon_extraction_slow object={} pixels={ICON_PIXELS} png_bytes={} elapsed_ms={} outcome=ready",
                    diagnostic_identity(query), png.len(), started.elapsed().as_millis()
                );
            }
            Some(png)
        }
        Err(stage) => {
            log::warn!(
                "file_icon_extraction_failed object={} reason={stage} pixels={ICON_PIXELS} elapsed_ms={} outcome=fallback",
                diagnostic_identity(query), started.elapsed().as_millis()
            );
            None
        }
    }
}

fn diagnostic_identity(query: &IconQuery) -> String {
    match query {
        IconQuery::Path { path, .. } => crate::diagnostics::text(&path.display()),
        IconQuery::Type { key, .. } => crate::diagnostics::text(key),
    }
}

fn load_png_in_pool(query: &IconQuery) -> Result<Vec<u8>, &'static str> {
    let workspace = NSWorkspace::sharedWorkspace();
    let image = match query {
        IconQuery::Type { .. } => {
            let content_type = content_type(query).ok_or("content_type")?;
            workspace.iconForContentType(&content_type)
        }
        IconQuery::Path { path, .. } => {
            let path = path.to_str().ok_or("path_encoding")?;
            workspace.iconForFile(&NSString::from_str(path))
        }
    };
    encode_png(&image)
}

fn content_type(query: &IconQuery) -> Option<Retained<UTType>> {
    let IconQuery::Type { key, extension } = query else {
        return None;
    };
    if key == "kind:folder" {
        // SAFETY: UniformTypeIdentifiers exports these immutable, process-wide
        // constants on every supported macOS version.
        return Some(unsafe { UTTypeFolder.retain() });
    }
    let Some(extension) = extension else {
        // SAFETY: See the constant-lifetime explanation above.
        return Some(unsafe { UTTypeData.retain() });
    };
    if extension == "app" {
        // Extension lookup resolves .app to the legacy application-file type on
        // macOS. Bundle identity selects the native grid placeholder used by Finder.
        // SAFETY: This immutable constant is available on every supported macOS version.
        return Some(unsafe { UTTypeApplicationBundle.retain() });
    }
    UTType::typeWithFilenameExtension(&NSString::from_str(extension))
        // SAFETY: See the constant-lifetime explanation above.
        .or_else(|| Some(unsafe { UTTypeData.retain() }))
}

fn encode_png(image: &NSImage) -> Result<Vec<u8>, &'static str> {
    // TIFFRepresentation serializes every representation of modern macOS icons;
    // a small row icon can produce a 70 MiB TIFF and hundreds of MiB of scratch
    // memory. Draw directly into a bounded bitmap instead, retaining alpha.
    // SAFETY: Null planes request AppKit-owned storage; dimensions, channel
    // count, and color space describe a packed, eight-bit RGBA bitmap.
    let bitmap = unsafe {
        NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bytesPerRow_bitsPerPixel(
            NSBitmapImageRep::alloc(), std::ptr::null_mut(), ICON_PIXELS, ICON_PIXELS,
            8, 4, true, false, NSCalibratedRGBColorSpace, 0, 0,
        )
    }.ok_or("bitmap_allocation")?;
    let pixels = bitmap.bitmapData();
    if pixels.is_null() {
        return Err("bitmap_storage");
    }
    // SAFETY: AppKit allocated this non-planar bitmap with the requested height.
    // Clear padding as well, so non-square icons retain transparent borders.
    unsafe { pixels.write_bytes(0, bitmap.bytesPerRow() as usize * ICON_PIXELS as usize) };
    let context =
        NSGraphicsContext::graphicsContextWithBitmapImageRep(&bitmap).ok_or("graphics_context")?;
    let size = image.size();
    if !size.width.is_finite()
        || !size.height.is_finite()
        || size.width <= 0.0
        || size.height <= 0.0
    {
        return Err("image_dimensions");
    }
    let scale = (ICON_PIXELS as f64 / size.width).min(ICON_PIXELS as f64 / size.height);
    let width = size.width * scale;
    let height = size.height * scale;
    let rect = NSRect::new(
        NSPoint::new(
            (ICON_PIXELS as f64 - width) / 2.0,
            (ICON_PIXELS as f64 - height) / 2.0,
        ),
        NSSize::new(width, height),
    );
    {
        NSGraphicsContext::saveGraphicsState_class();
        let _restore = GraphicsStateRestore;
        NSGraphicsContext::setCurrentContext(Some(&context));
        context.setImageInterpolation(NSImageInterpolation::High);
        image.drawInRect_fromRect_operation_fraction(
            rect,
            NSRect::ZERO,
            NSCompositingOperation::Copy,
            1.0,
        );
    }
    let properties = NSDictionary::<objc2_app_kit::NSBitmapImageRepPropertyKey, AnyObject>::new();
    // SAFETY: AppKit accepts an empty, correctly typed PNG property dictionary.
    let data = unsafe {
        bitmap.representationUsingType_properties(NSBitmapImageFileType::PNG, &properties)
    }
    .ok_or("png_encoding")?;
    Ok(data.to_vec())
}

struct GraphicsStateRestore;

impl Drop for GraphicsStateRestore {
    fn drop(&mut self) {
        NSGraphicsContext::restoreGraphicsState_class();
    }
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
    use super::*;

    #[test]
    fn diagnostics_use_the_path_instead_of_the_cache_identity() {
        let query = IconQuery::Path {
            key: "path:opaque-cache-key".into(),
            path: "/Applications/Example\nApp.app".into(),
        };
        let identity = diagnostic_identity(&query);
        assert!(identity.contains("/Applications/Example\\nApp.app"));
        assert!(!identity.contains("opaque-cache-key"));
        assert!(!identity.contains('\n'));
    }

    #[test]
    fn raster_variant_invalidates_oversized_path_and_type_cache_entries() {
        let path = IconQuery::Path {
            key: "path:/App.app".into(),
            path: "/App.app".into(),
        };
        let folder = IconQuery::Type {
            key: "kind:folder".into(),
            extension: None,
        };
        assert!(provider_variant(&path).starts_with(RASTER_VARIANT));
        assert!(provider_variant(&folder).starts_with(RASTER_VARIANT));
    }

    #[test]
    fn raster_preserves_aspect_alpha_and_previous_graphics_context() {
        autoreleasepool(|_| {
            // Two opaque pixels make a non-square source without shell services.
            // SAFETY: AppKit owns a two-pixel RGBA allocation.
            let source = unsafe {
                NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bytesPerRow_bitsPerPixel(
                    NSBitmapImageRep::alloc(), std::ptr::null_mut(), 2, 1,
                    8, 4, true, false, NSCalibratedRGBColorSpace, 0, 0,
                )
            }.unwrap();
            assert!(!source.bitmapData().is_null());
            // SAFETY: The bitmap owns at least eight bytes for these two pixels.
            unsafe { source.bitmapData().write_bytes(255, 8) };
            let image = NSImage::initWithSize(NSImage::alloc(), NSSize::new(2.0, 1.0));
            image.addRepresentation(&source);
            let previous = NSGraphicsContext::currentContext();
            let png = encode_png(&image).unwrap();
            assert_eq!(NSGraphicsContext::currentContext(), previous);
            assert!(super::super::cache::valid_png(&png));
            let decoded =
                NSBitmapImageRep::imageRepWithData(&objc2_foundation::NSData::with_bytes(&png))
                    .unwrap();
            assert_eq!((decoded.pixelsWide(), decoded.pixelsHigh()), (128, 128));
            assert_eq!(decoded.samplesPerPixel(), 4);
            let mut corner = [0_usize; 4];
            let mut center = [0_usize; 4];
            // SAFETY: Each output contains four slots and coordinates are in bounds.
            unsafe {
                decoded.getPixel_atX_y(std::ptr::NonNull::new(corner.as_mut_ptr()).unwrap(), 0, 0);
                decoded.getPixel_atX_y(
                    std::ptr::NonNull::new(center.as_mut_ptr()).unwrap(),
                    64,
                    64,
                );
            }
            assert_eq!(
                corner[3], 0,
                "non-square artwork must retain transparent margins"
            );
            assert_eq!(
                center, [255; 4],
                "the bounded raster must contain the artwork"
            );
        });
    }

    #[test]
    fn generic_application_uses_bundle_type_instead_of_legacy_application_file() {
        let query = IconQuery::Type {
            key: "ext:app".to_string(),
            extension: Some("app".to_string()),
        };
        assert_eq!(
            content_type(&query).unwrap().identifier().to_string(),
            "com.apple.application-bundle"
        );
    }
}
