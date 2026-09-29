use super::{DocumentAuthority, DocumentKind, SpecificationVersion};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Exact origin of a piece of evidence. Every source-backed record carries one.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub struct Provenance {
    /// Stable manifest source identifier.
    pub source_id: String,
    /// Indexed document revision (`{source_id}@{sha256 prefix}`).
    pub document_id: String,
    pub version: SpecificationVersion,
    pub kind: DocumentKind,
    pub authority: DocumentAuthority,
    pub precedence: u32,
    /// Locator within the source, e.g. `page:12`, `op:GET /accounts`, `schema:Transaction`.
    pub locator: String,
    /// SHA-256 of the record content (chunk text, page text, normalized JSON ...).
    pub sha256: String,
    /// SHA-256 of the whole source document file.
    pub document_sha256: String,
}
