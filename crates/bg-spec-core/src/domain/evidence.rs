use super::{DocumentAuthority, DocumentKind, Provenance, SpecificationVersion};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// How a returned item must be interpreted by the consuming agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceClass {
    /// A curated requirement mapping (requirements.yaml). The only authoritative requirement form.
    AuthoritativeRequirement,
    /// Retrieved by search/heuristics. Evidence only; never a requirement by itself.
    DiscoveredEvidence,
    /// Exact source content addressed by source id + locator.
    SourceContent,
}

/// An indexed piece of text evidence (part of a page / section).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct EvidenceChunk {
    pub chunk_id: String,
    pub page: Option<u32>,
    pub section: Option<String>,
    /// Position of the chunk within its page.
    pub ordinal: u32,
    pub text: String,
    pub provenance: Provenance,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RecordType {
    Chunk,
    Operation,
    Schema,
}

impl RecordType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Chunk => "chunk",
            Self::Operation => "operation",
            Self::Schema => "schema",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "chunk" => Some(Self::Chunk),
            "operation" => Some(Self::Operation),
            "schema" => Some(Self::Schema),
            _ => None,
        }
    }
}

/// Full-text search request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchQuery {
    pub text: String,
    pub version: Option<SpecificationVersion>,
    /// Empty = all kinds.
    pub kinds: Vec<DocumentKind>,
    pub source_id: Option<String>,
    pub limit: usize,
}

/// One full-text search hit. Always `discovered_evidence`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SearchResult {
    /// Chunk id, or record id for OpenAPI operations/schemas. Use with `read_source`.
    pub record_id: String,
    pub record_type: RecordType,
    pub source_id: String,
    pub document_id: String,
    pub version: SpecificationVersion,
    pub kind: DocumentKind,
    pub authority: DocumentAuthority,
    pub title: String,
    /// Locator usable with `read_source` (e.g. `page:12`, `op:GET /x`).
    pub locator: String,
    pub page: Option<u32>,
    /// Text excerpt of the matched evidence.
    pub evidence: String,
    /// BM25 relevance score (higher is more relevant; not a truth measure).
    pub relevance: f32,
    /// SHA-256 of the matched record content.
    pub sha256: String,
    pub classification: EvidenceClass,
}
