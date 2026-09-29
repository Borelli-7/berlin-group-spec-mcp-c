use super::SpecificationVersion;
use crate::{CoreError, Result};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};

macro_rules! string_enum {
    ($name:ident { $($variant:ident => $s:literal),+ $(,)? }) => {
        impl $name {
            pub const ALL: &'static [$name] = &[$($name::$variant),+];
            pub fn as_str(self) -> &'static str {
                match self { $($name::$variant => $s),+ }
            }
        }
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }
        impl FromStr for $name {
            type Err = CoreError;
            fn from_str(s: &str) -> Result<Self> {
                match s.trim().to_ascii_lowercase().as_str() {
                    $($s => Ok($name::$variant),)+
                    other => Err(CoreError::InvalidInput(format!(
                        concat!("unknown ", stringify!($name), " '{}' (expected one of: {})"),
                        other,
                        [$($s),+].join(", ")
                    ))),
                }
            }
        }
    };
}

/// Physical/semantic kind of a corpus document.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum DocumentKind {
    /// PDF specification (Implementation Guidelines, Operational Rules, ...).
    Pdf,
    /// OpenAPI 3.x contract (YAML or JSON).
    Openapi,
    /// Plain text or Markdown (clarifications, errata, curated notes).
    Text,
}
string_enum!(DocumentKind { Pdf => "pdf", Openapi => "openapi", Text => "text" });

/// Authority class declared explicitly in the manifest. Never inferred from file names.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum DocumentAuthority {
    Normative,
    Technical,
    Informative,
    Project,
    Test,
    Unknown,
}
string_enum!(DocumentAuthority {
    Normative => "normative",
    Technical => "technical",
    Informative => "informative",
    Project => "project",
    Test => "test",
    Unknown => "unknown",
});

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DocumentStatus {
    /// Fully indexed.
    Indexed,
    /// Indexed with gaps (for example pages requiring OCR).
    Partial,
    /// File declared in the manifest was not found.
    Missing,
    /// Extraction or parsing failed; no evidence is available.
    Failed,
}
string_enum!(DocumentStatus {
    Indexed => "indexed",
    Partial => "partial",
    Missing => "missing",
    Failed => "failed",
});

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Info,
    Warning,
    Error,
}

/// Diagnostic produced during indexing (stored with the document).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Diagnostic {
    pub severity: Severity,
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locator: Option<String>,
}

impl Diagnostic {
    pub fn new(severity: Severity, code: &str, message: impl Into<String>) -> Self {
        Self {
            severity,
            code: code.to_owned(),
            message: message.into(),
            locator: None,
        }
    }

    pub fn at(mut self, locator: impl Into<String>) -> Self {
        self.locator = Some(locator.into());
        self
    }
}

/// A catalogued corpus document (one indexed revision of a manifest source).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Document {
    /// Stable logical identifier from the manifest.
    pub source_id: String,
    /// Identifier of the indexed revision: `{source_id}@{sha256 prefix}`.
    pub document_id: String,
    pub kind: DocumentKind,
    pub version: SpecificationVersion,
    pub authority: DocumentAuthority,
    pub precedence: u32,
    pub title: String,
    /// Path relative to the corpus root (never absolute).
    pub path: String,
    pub sha256: Option<String>,
    pub size_bytes: Option<u64>,
    pub status: DocumentStatus,
    /// Fingerprint of manifest metadata + indexing settings (drives re-indexing).
    pub fingerprint: String,
    pub extractor: Option<String>,
    pub page_count: Option<u32>,
    pub metadata: serde_json::Value,
    pub diagnostics: Vec<Diagnostic>,
    pub indexed_at_unix: i64,
}

impl Document {
    pub fn document_id_for(source_id: &str, sha256: Option<&str>) -> String {
        match sha256 {
            Some(h) => format!("{source_id}@{}", crate::hash::short(h)),
            None => format!("{source_id}@unavailable"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PageStatus {
    /// Text extracted successfully.
    Extracted,
    /// No (or too little) extractable text: likely a scanned image. OCR required.
    OcrRequired,
    /// The extractor failed for this page.
    ExtractionFailed,
}
string_enum!(PageStatus {
    Extracted => "extracted",
    OcrRequired => "ocr_required",
    ExtractionFailed => "extraction_failed",
});

/// Full text of one page of a paged document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PageRecord {
    pub source_id: String,
    pub page: u32,
    pub status: PageStatus,
    pub text: String,
    pub char_count: u32,
    pub sha256: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_kinds_and_authorities() {
        assert_eq!("PDF".parse::<DocumentKind>().unwrap(), DocumentKind::Pdf);
        assert_eq!(
            "normative".parse::<DocumentAuthority>().unwrap(),
            DocumentAuthority::Normative
        );
        assert!("yaml".parse::<DocumentKind>().is_err());
        assert_eq!(DocumentAuthority::ALL.len(), 6);
    }

    #[test]
    fn document_id() {
        assert_eq!(
            Document::document_id_for("src", Some("0123456789abcdef0000")),
            "src@0123456789ab"
        );
    }
}
