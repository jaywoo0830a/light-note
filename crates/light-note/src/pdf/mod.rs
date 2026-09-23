//! PDF through Pdfium (`pdfium-render`).
//!
//! The engine is bound **at run time**: `pdfium.dll` is loaded on first use, and
//! the app keeps working (blank paper) when it is not there — see
//! [`bind_pdfium`] for the search order.
//!
//! Everything here runs on the PDF worker thread, never on the UI thread:
//! opening a document, measuring a page, rasterizing a page and writing ink into
//! a copy of the document are all blocking calls into a C++ library.

mod engine;

pub use engine::{PdfEngine, bind_pdfium, library_candidates};

use crate::geom::Size;
use crate::ink::Stroke;

/// What went wrong on the PDF side.
#[derive(Debug, thiserror::Error)]
pub enum PdfError {
    /// No Pdfium library could be loaded (the app still runs, without PDFs).
    #[error("pdfium is not available: {0}")]
    Unavailable(String),
    /// The file could not be read.
    #[error("could not read {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    /// Pdfium refused the document.
    #[error("pdfium could not open the document: {0}")]
    Pdfium(String),
    /// The document has no pages at all.
    #[error("the PDF has no pages")]
    NoPages,
    /// The page index does not exist in this document.
    #[error("page {0} does not exist")]
    PageOutOfRange(usize),
}

/// Ink to write into one page of the exported PDF.
#[derive(Clone, Debug, PartialEq)]
pub struct PageInk {
    /// Page index (0-based).
    pub index: usize,
    /// The strokes drawn on that page.
    pub strokes: Vec<Stroke>,
}

/// Page sizes in pt, for a document that is already in memory.
pub fn sizes_or_error(sizes: Vec<Size>) -> Result<Vec<Size>, PdfError> {
    if sizes.is_empty() {
        Err(PdfError::NoPages)
    } else {
        Ok(sizes)
    }
}
