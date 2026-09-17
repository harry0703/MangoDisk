//! Native grayscale text on a premultiplied transparent surface.
use super::{
    surface::Surface,
    text_layout::{Alignment, Run, TextStyle},
};
use windows::{
    core::{w, Result},
    Win32::{
        Foundation::RECT,
        Graphics::{
            Direct2D::{Common::*, *},
            DirectWrite::*,
            Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM,
            Gdi::HDC,
        },
    },
};

/// Owned by the taskbar's native thread. Reuse factories and the software target
/// across samples; an RDP session or VM must not require a working GPU device.
pub struct Renderer {
    target: ID2D1DCRenderTarget,
    write: IDWriteFactory,
    formats: Option<(u32, [IDWriteTextFormat; 2])>,
}
impl Renderer {
    pub unsafe fn new() -> Result<Self> {
        let factory: ID2D1Factory = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
        let target = factory.CreateDCRenderTarget(&D2D1_RENDER_TARGET_PROPERTIES {
            r#type: D2D1_RENDER_TARGET_TYPE_SOFTWARE,
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: 96.0,
            dpiY: 96.0,
            ..Default::default()
        })?;
        target.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
        Ok(Self {
            target,
            write: DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?,
            formats: None,
        })
    }

    fn format(&self, style: TextStyle) -> &IDWriteTextFormat {
        &self.formats.as_ref().expect("text formats initialized").1
            [if style == TextStyle::Label { 0 } else { 1 }]
    }

    pub unsafe fn paint(
        &mut self,
        dc: windows_sys::Win32::Graphics::Gdi::HDC,
        surface: &Surface,
        runs: &[Run<'_>],
        dpi: u32,
        foreground: [u8; 3],
    ) -> Result<()> {
        if self.formats.as_ref().map(|(scale, _)| *scale) != Some(dpi) {
            // D2D uses physical pixels at 96 DPI, matching GDI's font heights.
            let create = |style: TextStyle| -> Result<IDWriteTextFormat> {
                let format = self.write.CreateTextFormat(
                    w!("Segoe UI"),
                    None,
                    DWRITE_FONT_WEIGHT_NORMAL,
                    DWRITE_FONT_STYLE_NORMAL,
                    DWRITE_FONT_STRETCH_NORMAL,
                    style.pixels(dpi) as f32,
                    w!("en-US"),
                )?;
                format.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
                format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
                Ok(format)
            };
            self.formats = Some((dpi, [create(TextStyle::Label)?, create(TextStyle::Value)?]));
        }
        self.target.BindDC(
            HDC(dc),
            &RECT {
                left: 0,
                top: 0,
                right: surface.width,
                bottom: surface.height,
            },
        )?;
        let brush = self
            .target
            .CreateSolidColorBrush(&D2D1_COLOR_F::default(), None)?;
        self.target.BeginDraw();
        self.target.Clear(None);
        // EndDraw must run even if a per-run formatting call fails, otherwise
        // this reusable target would remain inside an unfinished drawing batch.
        let result = (|| -> Result<()> {
            for run in runs {
                if run.text.is_empty() {
                    continue;
                }
                let format = self.format(run.style);
                format.SetTextAlignment(match run.alignment {
                    Alignment::Left => DWRITE_TEXT_ALIGNMENT_LEADING,
                    Alignment::Center => DWRITE_TEXT_ALIGNMENT_CENTER,
                    Alignment::Right => DWRITE_TEXT_ALIGNMENT_TRAILING,
                })?;
                let [r, g, b] = run.ink.rgb(foreground);
                brush.SetColor(&D2D1_COLOR_F {
                    r: f32::from(r) / 255.0,
                    g: f32::from(g) / 255.0,
                    b: f32::from(b) / 255.0,
                    a: 1.0,
                });
                self.target.DrawText(
                    &run.text.encode_utf16().collect::<Vec<_>>(),
                    format,
                    &D2D_RECT_F {
                        left: run.bounds.left as f32,
                        top: run.bounds.top as f32,
                        right: run.bounds.right as f32,
                        bottom: run.bounds.bottom as f32,
                    },
                    &brush,
                    D2D1_DRAW_TEXT_OPTIONS_CLIP,
                    DWRITE_MEASURING_MODE_NATURAL,
                );
            }
            Ok(())
        })();
        let finished = self.target.EndDraw(None, None);
        result.and(finished)
    }
}

#[cfg(test)]
#[path = "directwrite_tests.rs"]
mod tests;
