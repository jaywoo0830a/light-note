//! The Pdfium engine: bind, measure, rasterize, annotate.
//!
//! `pdfium-render` binds to a **runtime** library, so the app can be built and
//! run without Pdfium at all — it just cannot open PDFs.  [`bind_pdfium`] tries,
//! in order:
//!
//! 1. `LIGHT_NOTE_PDFIUM` (a directory, or a full path to the library);
//! 2. next to the executable, and in `<exe dir>\pdfium`;
//! 3. `crates/light-note/pdfium` (so `cargo run` works from a checkout);
//! 4. the working directory, and `<cwd>\pdfium`;
//! 5. whatever the system can find (`Pdfium::default()`).
//!
//! Ink is written back as **vector paths** into a copy of the document, so an
//! exported note keeps the original text and vectors instead of becoming a
//! picture of a page.

use std::path::PathBuf;

use pdfium_render::prelude::*;
use vello_cpu::Pixmap;
use vello_cpu::peniko::color::PremulRgba8;

use super::{PageInk, PdfError};
use crate::geom::{Scale, Size};
use crate::shape::{spans_union, stroke_shape};

/// A bound Pdfium library.  `Send + Sync` (the `thread_safe` feature), so a
/// worker thread owns one and the UI thread never calls into it.
pub struct PdfEngine {
    pdfium: Pdfium,
}

/// Every place the library may live, in the order they are tried.
pub fn library_candidates() -> Vec<PathBuf> {
    let mut directories: Vec<PathBuf> = Vec::new();
    if let Some(configured) = std::env::var_os("LIGHT_NOTE_PDFIUM") {
        let path = PathBuf::from(configured);
        if path.is_file() {
            directories.push(path);
        } else {
            directories.push(path);
        }
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        directories.push(dir.to_path_buf());
        directories.push(dir.join("pdfium"));
    }
    directories.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("pdfium"));
    if let Ok(cwd) = std::env::current_dir() {
        directories.push(cwd.clone());
        directories.push(cwd.join("pdfium"));
    }
    directories
}

/// Binds Pdfium, or explains why it is not available.
pub fn bind_pdfium() -> Result<PdfEngine, PdfError> {
    for candidate in library_candidates() {
        let library = if candidate.is_file() {
            candidate
        } else {
            Pdfium::pdfium_platform_library_name_at_path(&candidate)
        };
        if !library.exists() {
            continue;
        }
        if let Ok(bindings) = Pdfium::bind_to_library(&library) {
            return Ok(PdfEngine {
                pdfium: Pdfium::new(bindings),
            });
        }
    }

    // The system search (or a library bound by an earlier call).
    match std::panic::catch_unwind(Pdfium::default) {
        Ok(pdfium) => Ok(PdfEngine { pdfium }),
        Err(_) => Err(PdfError::Unavailable(format!(
            "no pdfium library found (looked in {})",
            library_candidates()
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ))),
    }
}

impl PdfEngine {
    /// Opens a document from memory.
    ///
    /// The document borrows the engine *and* the bytes (Pdfium keeps a pointer
    /// into them), so both share the named lifetime.
    fn open<'a>(&'a self, bytes: &'a [u8]) -> Result<PdfDocument<'a>, PdfError> {
        self.pdfium
            .load_pdf_from_byte_slice(bytes, None)
            .map_err(|error| PdfError::Pdfium(error.to_string()))
    }

    /// Page sizes in pt (the top-left origin is the app's business, not Pdfium's).
    pub fn page_sizes(&self, bytes: &[u8]) -> Result<Vec<Size>, PdfError> {
        let document = self.open(bytes)?;
        let sizes: Vec<Size> = document
            .pages()
            .iter()
            .map(|page| Size::new(page.width().value, page.height().value))
            .collect();
        super::sizes_or_error(sizes)
    }

    /// Renders one page onto white paper at `scale`.
    pub fn render_page(&self, bytes: &[u8], index: usize, scale: Scale) -> Result<Pixmap, PdfError> {
        let document = self.open(bytes)?;
        let page = document
            .pages()
            .get(index as PdfPageIndex)
            .map_err(|_| PdfError::PageOutOfRange(index))?;

        let size = Size::new(page.width().value, page.height().value);
        let (width, height) = size.to_pixels(scale);
        let bitmap = page
            .render_with_config(
                &PdfRenderConfig::new()
                    .set_target_width(width as i32)
                    .set_target_height(height as i32),
            )
            .map_err(|error| PdfError::Pdfium(error.to_string()))?;

        Ok(pixmap_from_rgba_over_white(
            &bitmap.as_rgba_bytes(),
            width,
            height,
        ))
    }

    /// Writes `pages`' ink into a copy of `bytes` and returns the new document.
    pub fn annotate(&self, bytes: &[u8], pages: &[PageInk]) -> Result<Vec<u8>, PdfError> {
        let document = self.open(bytes)?;
        let mut paths: Vec<(usize, PdfPagePathObject<'_>)> = Vec::new();

        for page_ink in pages {
            let Ok(page) = document.pages().get(page_ink.index as PdfPageIndex) else {
                return Err(PdfError::PageOutOfRange(page_ink.index));
            };
            let height = page.height().value;
            for stroke in &page_ink.strokes {
                // The export path works in pt, so the shape is built at 1 px per
                // pt (not at the screen's 1.5) and flipped into PDF space.
                let Some(shape) = stroke_shape(stroke, Scale::new(1.0)) else {
                    continue;
                };
                let Some(spans) = shape.spans() else {
                    continue;
                };
                if spans.is_empty() {
                    continue;
                }
                let color = stroke.style().color.over_white();
                let mut path = PdfPagePathObject::new(
                    &document,
                    PdfPoints::new(0.0),
                    PdfPoints::new(0.0),
                    None,
                    None,
                    Some(PdfColor::new(color.r, color.g, color.b, 255)),
                )
                .map_err(|error| PdfError::Pdfium(error.to_string()))?;

                // The union outline, so a translucent stroke is one shape.
                for element in spans_union(&spans).elements() {
                    let point = |x: f64, y: f64| {
                        (PdfPoints::new(x as f32), PdfPoints::new(height - y as f32))
                    };
                    let result = match element {
                        vello_cpu::kurbo::PathEl::MoveTo(at) => {
                            let (x, y) = point(at.x, at.y);
                            path.move_to(x, y)
                        }
                        vello_cpu::kurbo::PathEl::LineTo(at) => {
                            let (x, y) = point(at.x, at.y);
                            path.line_to(x, y)
                        }
                        vello_cpu::kurbo::PathEl::QuadTo(control, at) => {
                            let (cx, cy) = point(control.x, control.y);
                            let (x, y) = point(at.x, at.y);
                            path.bezier_to(cx, cy, cx, cy, x, y)
                        }
                        vello_cpu::kurbo::PathEl::CurveTo(first, second, at) => {
                            let (ax, ay) = point(first.x, first.y);
                            let (bx, by) = point(second.x, second.y);
                            let (x, y) = point(at.x, at.y);
                            path.bezier_to(ax, ay, bx, by, x, y)
                        }
                        vello_cpu::kurbo::PathEl::ClosePath => path.close_path(),
                    };
                    result.map_err(|error| PdfError::Pdfium(error.to_string()))?;
                }
                paths.push((page_ink.index, path));
            }
        }

        for (index, path) in paths {
            let mut page = document
                .pages()
                .get(index as PdfPageIndex)
                .map_err(|_| PdfError::PageOutOfRange(index))?;
            page.objects_mut()
                .add_path_object(path)
                .map_err(|error| PdfError::Pdfium(error.to_string()))?;
        }

        document
            .save_to_bytes()
            .map_err(|error| PdfError::Pdfium(error.to_string()))
    }
}

/// Pdfium hands back straight RGBA; the rasterizer wants premultiplied pixels
/// over white paper (a page with no background is transparent, not white).
fn pixmap_from_rgba_over_white(rgba: &[u8], width: u16, height: u16) -> Pixmap {
    let mut pixels = Vec::with_capacity(width as usize * height as usize);
    for chunk in rgba.chunks_exact(4) {
        let alpha = chunk[3] as u32;
        let over_white = |channel: u8| ((channel as u32 * alpha + 255 * (255 - alpha)) / 255) as u8;
        pixels.push(PremulRgba8 {
            r: over_white(chunk[0]),
            g: over_white(chunk[1]),
            b: over_white(chunk[2]),
            a: 255,
        });
    }
    Pixmap::from_parts(pixels, width, height)
}