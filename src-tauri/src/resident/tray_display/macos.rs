//! Retina-aware native image rendering. Keep AppKit work on Tauri's main thread.
use super::{
    format::DisplayEntry,
    macos_presentation::{self, Column},
};
use block2::RcBlock;
use objc2::{
    rc::Retained,
    runtime::{AnyObject, Bool},
    AnyThread, MainThreadMarker,
};
use objc2_app_kit::{
    NSAccessibility, NSAppearanceCustomization, NSCellImagePosition, NSColor,
    NSCompositingOperation, NSFont, NSFontAttributeName, NSForegroundColorAttributeName, NSImage,
    NSRectFillUsingOperation, NSStringDrawing,
};
use objc2_foundation::{NSData, NSDictionary, NSPoint, NSRect, NSSize, NSString};

const RATE_VALUE_WIDTH: f64 = 24.0;
const RATE_UNIT_WIDTH: f64 = 32.0;

#[derive(Default)]
pub struct Cache {
    columns: Vec<Column>,
    icon: Option<bool>,
    compact: bool,
    appearance: String,
    button: usize,
}

fn text(value: &str, mut rect: NSRect, size: f64, weight: f64, color: &NSColor, right: bool) {
    let font = NSFont::monospacedDigitSystemFontOfSize_weight(size, weight);
    // These AppKit attribute keys require an NSFont and NSColor respectively.
    let attributes: Retained<NSDictionary<NSString, AnyObject>> = unsafe {
        NSDictionary::from_slices(
            &[NSFontAttributeName, NSForegroundColorAttributeName],
            &[&*font, color],
        )
    };
    unsafe {
        let value = NSString::from_str(value);
        if right {
            // Measure with the same font used for drawing. Digits grow leftward
            // within their reserved field; the unit always keeps its own origin.
            let offset =
                (rect.size.width - value.sizeWithAttributes(Some(&attributes)).width).max(0.0);
            rect.origin.x += offset;
            rect.size.width -= offset;
        }
        value.drawInRect_withAttributes(rect, Some(&attributes));
    }
}

fn rect(x: f64, y: f64, width: f64, height: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(width, height))
}

fn image(columns: Vec<Column>, icon: bool, dark: bool, compact: bool) -> Retained<NSImage> {
    let size = NSSize::new(
        macos_presentation::width(&columns, icon, compact) as f64,
        macos_presentation::HEIGHT as f64,
    );
    let logo = icon
        .then(|| {
            NSImage::initWithData(
                NSImage::alloc(),
                &NSData::with_bytes(include_bytes!("../../../icons/tray-template.png")),
            )
        })
        .flatten();
    // A drawing-handler image renders at the actual backing scale instead of
    // stretching one bitmap. The button retains the image and its owned snapshot.
    let draw = RcBlock::new(move |_: NSRect| {
        let foreground = if dark {
            NSColor::whiteColor()
        } else {
            NSColor::blackColor()
        };
        let mut x = 0.0;
        if let Some(logo) = &logo {
            let area = rect(0.0, 2.0, 18.0, 18.0);
            logo.drawInRect(area);
            foreground.set();
            NSRectFillUsingOperation(area, NSCompositingOperation::SourceAtop);
            x = 18.0 + macos_presentation::gap(compact) as f64;
        }
        for column in &columns {
            if column.id == super::format::DisplayId::Upload {
                // Direction remains explicit for users who cannot distinguish the colors.
                text(
                    "↑",
                    rect(x, 0.0, 10.0, 11.0),
                    10.0,
                    0.3,
                    &NSColor::systemRedColor(),
                    false,
                );
                text(
                    "↓",
                    rect(x, 11.0, 10.0, 11.0),
                    10.0,
                    0.3,
                    &NSColor::systemBlueColor(),
                    false,
                );
                for (value, unit, y) in [
                    (&column.top, &column.top_unit, 0.0),
                    (&column.bottom, &column.bottom_unit, 11.0),
                ] {
                    text(
                        value,
                        rect(x + 11.0, y, RATE_VALUE_WIDTH, 11.0),
                        10.0,
                        0.0,
                        &foreground,
                        true,
                    );
                    text(
                        unit,
                        rect(
                            x + (if compact { 12.0 } else { 14.0 }) + RATE_VALUE_WIDTH,
                            y,
                            if compact { 18.0 } else { RATE_UNIT_WIDTH },
                            11.0,
                        ),
                        10.0,
                        0.0,
                        &foreground,
                        false,
                    );
                }
            } else {
                let [r, g, b] = column.tone.rgb(if dark { [255; 3] } else { [0; 3] });
                let value_color = NSColor::colorWithSRGBRed_green_blue_alpha(
                    f64::from(r) / 255.0,
                    f64::from(g) / 255.0,
                    f64::from(b) / 255.0,
                    1.0,
                );
                text(
                    &column.top,
                    rect(x, 0.0, column.width as f64, 9.0),
                    7.5,
                    0.0,
                    &foreground,
                    false,
                );
                text(
                    &column.bottom,
                    rect(x, 8.0, column.width as f64, 14.0),
                    if compact { 11.0 } else { 12.0 },
                    0.3,
                    &value_color,
                    false,
                );
            }
            x += (column.width + macos_presentation::gap(compact)) as f64;
        }
        Bool::YES
    });
    let image = NSImage::imageWithSize_flipped_drawingHandler(size, true, &draw);
    image.setTemplate(false);
    image
}

pub fn apply(
    tray: &tauri::tray::TrayIcon,
    entries: &[DisplayEntry],
    icon: bool,
    compact: bool,
    summary: &str,
    cache: &mut Cache,
) -> tauri::Result<bool> {
    let columns = macos_presentation::columns(entries, compact);
    let changed = cache.columns != columns || cache.icon != Some(icon) || cache.compact != compact;
    let previous_appearance = cache.appearance.clone();
    let previous_button = cache.button;
    let content = columns.clone();
    let label = format!("MangoDisk\n{summary}");
    let result = tray.with_inner_tray_icon(move |tray| {
        let mtm = MainThreadMarker::new().expect("native tray rendering runs on the main thread");
        let item = tray.ns_status_item()?;
        let button = item.button(mtm)?;
        let appearance = button.effectiveAppearance().name().to_string();
        let identity = Retained::as_ptr(&button) as usize;
        let redraw = changed || appearance != previous_appearance || identity != previous_button;
        if redraw {
            button.setTitle(&NSString::from_str(""));
            button.setImage(Some(&image(
                content,
                icon,
                appearance.contains("Dark"),
                compact,
            )));
            button.setImagePosition(NSCellImagePosition::ImageOnly);
            // AppKit adds its normal status-item padding around the content image.
            item.setLength(-1.0);
        }
        button.setAccessibilityLabel(Some(&NSString::from_str(&label)));
        Some((appearance, identity, redraw))
    })?;
    let Some((appearance, button, redraw)) = result else {
        return Err(tauri::Error::Io(std::io::Error::other(
            "display_native_button",
        )));
    };
    if cache.icon != Some(icon)
        || cache.appearance != appearance
        || macos_presentation::width(&cache.columns, icon, cache.compact)
            != macos_presentation::width(&columns, icon, compact)
    {
        log::info!(
            "resident_macos_layout columns={} content_width_pt={} dark={} compact={compact}",
            columns.len(),
            macos_presentation::width(&columns, icon, compact),
            appearance.contains("Dark")
        );
    }
    cache.columns = columns;
    cache.icon = Some(icon);
    cache.compact = compact;
    cache.appearance = appearance;
    cache.button = button;
    Ok(redraw)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_fonts_fit_every_percentage_and_network_field() {
        fn measured(value: &str, size: f64, weight: f64) -> f64 {
            let font = NSFont::monospacedDigitSystemFontOfSize_weight(size, weight);
            let attributes: Retained<NSDictionary<NSString, AnyObject>> =
                unsafe { NSDictionary::from_slices(&[NSFontAttributeName], &[&*font]) };
            unsafe {
                NSString::from_str(value)
                    .sizeWithAttributes(Some(&attributes))
                    .width
            }
        }
        let column = macos_presentation::columns(
            &[DisplayEntry {
                tone: Default::default(),
                usage_percent: None,
                id: super::super::format::DisplayId::Cpu,
                digits: "100".into(),
                marker: "C".into(),
                text: String::new(),
                tooltip: String::new(),
            }],
            false,
        );
        let percentage_width = column[0].width as f64;
        // Real AppKit metrics catch regressions that string-length tests miss.
        for value in 0..=100 {
            assert!(
                measured(&format!("{value}%"), 11.0, 0.3) <= 34.0,
                "compact percentage {value} overflows"
            );
            assert!(
                measured(&format!("{value}%"), 12.0, 0.3) <= percentage_width,
                "percentage {value} overflows"
            );
        }
        for value in 0..=999 {
            assert!(
                measured(&value.to_string(), 10.0, 0.0) <= RATE_VALUE_WIDTH,
                "rate {value} overflows"
            );
        }
        for unit in ["B/s", "KB/s", "MB/s", "GB/s", "TB/s"] {
            assert!(
                measured(unit, 10.0, 0.0) <= RATE_UNIT_WIDTH,
                "unit {unit} overflows"
            );
        }
    }
}
