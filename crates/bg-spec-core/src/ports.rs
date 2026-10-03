//! Repository ports. Storage adapters implement these; services depend only on them.

use crate::{
    Result,
    domain::{
        Document, EvidenceChunk, OpenApiOperation, OpenApiSchema, PageRecord, RequirementRecord,
        SearchQuery, SearchResult, SpecificationVersion,
    },
};
use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CatalogStats {
    pub documents: u64,
    pub pages: u64,
    pub ocr_required_pages: u64,
    pub blank_pages: u64,
    pub chunks: u64,
    pub operations: u64,
    pub schemas: u64,
    pub requirements: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct IndexMeta {
    pub schema_version: Option<u32>,
    pub last_indexed_at_unix: Option<i64>,
    pub index_generation: Option<u64>,
    pub requirements_file_sha256: Option<String>,
}

/// Read access to the authoritative metadata/provenance catalog.
#[async_trait]
pub trait CatalogRepository: Send + Sync {
    async fn list_documents(&self) -> Result<Vec<Document>>;
    async fn get_document(&self, source_id: &str) -> Result<Option<Document>>;
    /// Documents with any of these source ids (unknown ids are skipped).
    async fn get_documents(&self, source_ids: &[String]) -> Result<Vec<Document>>;
    async fn get_pages(&self, source_id: &str, from: u32, to: u32) -> Result<Vec<PageRecord>>;
    async fn get_chunk(&self, chunk_id: &str) -> Result<Option<EvidenceChunk>>;
    async fn chunks_for_page(&self, source_id: &str, page: u32) -> Result<Vec<EvidenceChunk>>;
    /// Chunks whose section equals `section` or starts with `section` + separator.
    async fn chunks_for_section(
        &self,
        source_id: &str,
        section: &str,
    ) -> Result<Vec<EvidenceChunk>>;
    /// Operations of a version matching method + canonical path key, highest precedence first.
    async fn find_operations(
        &self,
        version: &SpecificationVersion,
        method: &str,
        path_key: &str,
    ) -> Result<Vec<OpenApiOperation>>;
    async fn get_source_operation(
        &self,
        source_id: &str,
        method: &str,
        path: &str,
    ) -> Result<Option<OpenApiOperation>>;
    async fn source_operations_by_path(
        &self,
        source_id: &str,
        path: &str,
    ) -> Result<Vec<OpenApiOperation>>;
    /// Schemas of a version with this name, highest precedence first.
    async fn find_schemas(
        &self,
        version: &SpecificationVersion,
        name: &str,
    ) -> Result<Vec<OpenApiSchema>>;
    async fn get_source_schema(&self, source_id: &str, name: &str)
    -> Result<Option<OpenApiSchema>>;
    /// Schemas of one source with any of these names (missing names are skipped).
    async fn get_source_schemas(
        &self,
        source_id: &str,
        names: &[String],
    ) -> Result<Vec<OpenApiSchema>>;
    /// Schemas of a version with any of these names, grouped by name and highest precedence first
    /// within a name (the order of [`Self::find_schemas`]).
    async fn find_schemas_by_names(
        &self,
        version: &SpecificationVersion,
        names: &[String],
    ) -> Result<Vec<OpenApiSchema>>;
    async fn list_requirements(&self) -> Result<Vec<RequirementRecord>>;
    async fn get_requirement(&self, id: &str) -> Result<Option<RequirementRecord>>;
    /// Requirements with one of the `ids`, plus every requirement whose `conflicts_with` names
    /// `target`; ordered by id. Unknown ids are skipped.
    async fn requirement_conflict_partners(
        &self,
        target: &str,
        ids: &[String],
    ) -> Result<Vec<RequirementRecord>>;
    /// Requirements of one version, ordered by id.
    async fn requirements_by_version(
        &self,
        version: &SpecificationVersion,
    ) -> Result<Vec<RequirementRecord>>;
    /// Requirements of `version` that may link an endpoint with this (normalised) method: a declared
    /// endpoint with the method, or any `op:`/`path:` source locator. A superset; callers match
    /// paths exactly.
    async fn requirements_for_method(
        &self,
        version: &SpecificationVersion,
        method: &str,
    ) -> Result<Vec<RequirementRecord>>;
    async fn stats(&self) -> Result<CatalogStats>;
    async fn index_meta(&self) -> Result<IndexMeta>;
}

/// Full-text retrieval over indexed evidence.
#[async_trait]
pub trait SearchRepository: Send + Sync {
    async fn search(&self, query: &SearchQuery) -> Result<Vec<SearchResult>>;
    async fn num_docs(&self) -> Result<u64>;
}
