use super::Provenance;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum ConflictKind {
    /// Declared via `conflicts_with` in requirements.yaml.
    Declared,
    /// A curated mapping points to a source/endpoint/schema/page that does not exist or is not indexed.
    DanglingReference,
    /// Cited sources of the same version share precedence but differ in authority.
    PrecedenceTie,
    /// A pinned source hash no longer matches the indexed document.
    StaleSource,
    /// The same operation/schema is defined differently by several sources of one version.
    DuplicateDefinition,
}

/// A deterministic conflict finding. The server reports conflicts; it never resolves them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Conflict {
    pub kind: ConflictKind,
    pub description: String,
    pub requirement_ids: Vec<String>,
    /// Source citations involved (`source_id` + locator).
    pub citations: Vec<Citation>,
    /// Full provenance of involved indexed records, where they exist.
    pub provenance: Vec<Provenance>,
}

/// Lightweight source citation (may reference sources that are not indexed).
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
pub struct Citation {
    pub source_id: String,
    pub locator: String,
}
