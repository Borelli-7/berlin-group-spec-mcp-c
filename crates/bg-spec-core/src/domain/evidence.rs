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
    /// Lower-ranked hits collapsed into this one (identical content or the same page).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub also_found_in: Vec<CollapsedHit>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DuplicateReason {
    /// Same version and identical content hash (e.g. a schema repeated in several sources).
    IdenticalContent,
    /// Another chunk of the same page and section (title) of the same source.
    SamePage,
}

/// A search hit collapsed into a better-ranked one; still addressable with `read_source`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CollapsedHit {
    pub record_id: String,
    pub source_id: String,
    pub locator: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<u32>,
    pub relevance: f32,
    pub sha256: String,
    pub reason: DuplicateReason,
}

/// Collapses ranked hits that repeat earlier evidence, keeping the first (best-ranked) of each
/// group, then truncates to `limit`. Input order is preserved, so the result is deterministic.
pub fn collapse_duplicates(results: Vec<SearchResult>, limit: usize) -> Vec<SearchResult> {
    let mut out: Vec<SearchResult> = Vec::new();
    for hit in results {
        let target = out.iter().position(|kept| {
            kept.version == hit.version && !hit.sha256.is_empty() && kept.sha256 == hit.sha256
        });
        let (target, reason) = match target {
            Some(i) => (Some(i), DuplicateReason::IdenticalContent),
            None => (
                out.iter().position(|kept| {
                    hit.record_type == RecordType::Chunk
                        && kept.record_type == RecordType::Chunk
                        && hit.page.is_some()
                        && kept.page == hit.page
                        && kept.source_id == hit.source_id
                        && kept.title == hit.title
                }),
                DuplicateReason::SamePage,
            ),
        };
        match target {
            Some(i) => out[i].also_found_in.push(CollapsedHit {
                record_id: hit.record_id,
                source_id: hit.source_id,
                locator: hit.locator,
                page: hit.page,
                relevance: hit.relevance,
                sha256: hit.sha256,
                reason,
            }),
            None if out.len() < limit => out.push(hit),
            // Once full, later hits are only kept as duplicates of the retained ones.
            None => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(id: &str, source: &str, page: Option<u32>, sha: &str, rt: RecordType) -> SearchResult {
        SearchResult {
            record_id: id.into(),
            record_type: rt,
            source_id: source.into(),
            document_id: format!("{source}@1"),
            version: SpecificationVersion::new("openfinance-v2").unwrap(),
            kind: DocumentKind::Pdf,
            authority: DocumentAuthority::Normative,
            title: String::new(),
            locator: id.into(),
            page,
            evidence: String::new(),
            relevance: 1.0,
            sha256: sha.into(),
            classification: EvidenceClass::DiscoveredEvidence,
            also_found_in: Vec::new(),
        }
    }

    #[test]
    fn collapses_identical_content_and_same_page() {
        let hits = vec![
            hit("a", "s1", Some(3), "x", RecordType::Chunk),
            hit("b", "s1", Some(3), "y", RecordType::Chunk),
            hit("c", "s2", None, "z", RecordType::Schema),
            hit("d", "s3", None, "z", RecordType::Schema),
            hit("e", "s1", Some(4), "w", RecordType::Chunk),
            hit("f", "s2", Some(3), "v", RecordType::Chunk),
            hit("g", "s9", None, "u", RecordType::Schema),
            hit("h", "s1", Some(4), "t", RecordType::Chunk),
            {
                let mut other_section = hit("i", "s1", Some(3), "r", RecordType::Chunk);
                other_section.title = "Other section".into();
                other_section
            },
        ];
        let out = collapse_duplicates(hits, 4);
        let ids: Vec<_> = out.iter().map(|h| h.record_id.as_str()).collect();
        assert_eq!(ids, vec!["a", "c", "e", "f"]);
        assert_eq!(out[0].also_found_in[0].record_id, "b");
        assert_eq!(out[0].also_found_in[0].reason, DuplicateReason::SamePage);
        assert_eq!(
            out[1].also_found_in[0].reason,
            DuplicateReason::IdenticalContent
        );
        // A duplicate ranked after the cut-off is still attached to its kept hit.
        assert_eq!(out[2].also_found_in[0].record_id, "h");
        assert!(out[3].also_found_in.is_empty());
    }
}
