//! PDF extraction behind a replaceable interface.

pub mod fixture;
mod oxide;

pub use oxide::PdfOxideExtractor;

use bg_spec_core::Result;
use std::path::Path;

/// Raw text of a single PDF page as produced by an extractor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedPage {
    /// 1-based page number.
    pub number: u32,
    pub text: String,
    /// Set when the extractor failed on this page (other pages may still succeed).
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedPdf {
    pub pages: Vec<ExtractedPage>,
}

/// Page-aware text extraction. Implementations must be deterministic and must not
/// silently drop pages: every page is returned, with empty text if nothing was extracted.
///
/// A future Pdfium or OCR-backed implementation can be plugged in without changing the
/// indexing pipeline.
pub trait PdfExtractor: Send + Sync {
    /// Stable identifier recorded in the catalog (part of the document fingerprint).
    fn name(&self) -> &'static str;
    fn extract(&self, path: &Path) -> Result<ExtractedPdf>;
}
