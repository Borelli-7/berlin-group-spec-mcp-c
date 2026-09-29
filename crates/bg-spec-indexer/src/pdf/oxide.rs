use super::{ExtractedPage, ExtractedPdf, PdfExtractor};
use bg_spec_core::{CoreError, Result};
use pdf_oxide::document::PdfDocument;
use std::path::Path;

/// [`PdfExtractor`] backed by the pure-Rust `pdf_oxide` crate.
#[derive(Debug, Default, Clone, Copy)]
pub struct PdfOxideExtractor;

impl PdfExtractor for PdfOxideExtractor {
    fn name(&self) -> &'static str {
        "pdf_oxide-0.3"
    }

    fn extract(&self, path: &Path) -> Result<ExtractedPdf> {
        let display = path.display().to_string();
        let doc = PdfDocument::open(path)
            .map_err(|e| CoreError::Integrity(format!("cannot open PDF {display}: {e}")))?;
        let count = doc.page_count().map_err(|e| {
            CoreError::Integrity(format!("cannot read page tree of {display}: {e}"))
        })?;
        let pages = (0..count)
            .map(|index| {
                let number = u32::try_from(index + 1).unwrap_or(u32::MAX);
                match doc.extract_text(index) {
                    Ok(text) => ExtractedPage {
                        number,
                        text,
                        error: None,
                    },
                    Err(e) => ExtractedPage {
                        number,
                        text: String::new(),
                        error: Some(e.to_string()),
                    },
                }
            })
            .collect();
        Ok(ExtractedPdf { pages })
    }
}
