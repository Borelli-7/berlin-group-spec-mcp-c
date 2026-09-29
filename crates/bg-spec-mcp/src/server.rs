use crate::{error::ToolError, inputs::*};
use bg_spec_core::services::{
    ComparisonReport, EndpointEvidenceBundle, EndpointResponse, FindRequirementResponse, HealthReport,
    ListSourcesResponse, ReadSourceResponse, RequirementTrace, SchemaResponse, SearchResponse, Services,
};
use rmcp::{
    ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::{Json, Parameters}},
    model::{Implementation, ServerCapabilities, ServerConfig},
    tool, tool_handler, tool_router,
};

type ToolResult<T> = Result<Json<T>, ToolError>;

/// Names of every tool exposed by the server (all read-only).
pub const TOOL_NAMES: [&str; 10] = [
    "search_specification",
    "find_requirement",
    "read_source",
    "read_openapi_endpoint",
    "read_openapi_schema",
    "get_endpoint_requirements",
    "trace_requirement",
    "compare_v1_v2",
    "list_sources",
    "health",
];

pub const SERVER_INSTRUCTIONS: &str = "Berlin Group NextGenPSD2 / Open Finance specification evidence server (read-only, deterministic, no LLM). \
Every record carries provenance (source_id, document_id, version, kind, locator, sha256). \
Only curated requirement mappings (classification 'authoritative_requirement') are requirements; \
full-text search hits are 'discovered_evidence' and must be verified with read_source before being relied upon. \
Conflicts are reported, never resolved. compare_v1_v2 reports structural facts only, never a compatibility verdict. \
Recommended flow: get_endpoint_requirements -> read_openapi_endpoint -> trace_requirement -> read_source -> compare_v1_v2.";

/// The MCP server. Holds only service handles; all logic lives in `bg_spec_core::services`.
#[derive(Clone)]
pub struct BgSpecServer {
    services: Services,
    tool_router: ToolRouter<Self>,
}

#[tool_router(router = tool_router)]
impl BgSpecServer {
    pub fn new(services: Services) -> Self {
        Self { services, tool_router: Self::tool_router() }
    }

    #[tool(
        name = "search_specification",
        description = "Full-text (BM25) search over indexed specification evidence: PDF pages/sections, text documents and OpenAPI operations/schemas. \
Filter by version, kind and source_id. Returns source_id, version, kind, title, locator, page, evidence snippet, relevance and sha256. \
IMPORTANT: results are DISCOVERED EVIDENCE ranked by text relevance and must NOT automatically be interpreted as authoritative requirements. \
Verify with read_source and prefer curated requirements (find_requirement / trace_requirement).",
        annotations(title = "Search specification evidence", read_only_hint = true, destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn search_specification(&self, Parameters(p): Parameters<SearchSpecificationInput>) -> ToolResult<SearchResponse> {
        let r = self
            .services
            .specification
            .search(&p.query, p.version.as_deref(), p.kind.as_deref(), p.source_id.as_deref(), p.limit)
            .await?;
        Ok(Json(r))
    }

    #[tool(
        name = "find_requirement",
        description = "Find requirements by curated id, title or text. Curated mappings from requirements.yaml are returned as 'authoritative_requirement' \
with sources, locators, acceptance criteria and related endpoints. Full-text matches are returned separately as 'discovered_evidence'. \
Requirement ids are never invented: if no curated mapping exists, the requirements list is empty.",
        annotations(title = "Find requirement", read_only_hint = true, destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn find_requirement(&self, Parameters(p): Parameters<FindRequirementInput>) -> ToolResult<FindRequirementResponse> {
        Ok(Json(self.services.requirements.find(&p.query, p.version.as_deref(), p.limit).await?))
    }

    #[tool(
        name = "read_source",
        description = "Read exact indexed content of a source by stable source_id and locator (page:N, page:N-M, section:X, chunk:ID, op:METHOD /path, path:/path, schema:Name). \
Returns source metadata, the resolved location, content and hashes. Arbitrary filesystem paths are not accepted.",
        annotations(title = "Read source evidence", read_only_hint = true, destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn read_source(&self, Parameters(p): Parameters<ReadSourceInput>) -> ToolResult<ReadSourceResponse> {
        Ok(Json(self.services.specification.read_source(&p.source_id, &p.locator).await?))
    }

    #[tool(
        name = "read_openapi_endpoint",
        description = "Return the normalized OpenAPI operation (path, method, operationId, parameters, request body, responses, security, tags) \
for a version, path template and HTTP method, with source provenance. $refs in parameters are resolved; referenced schema names are listed.",
        annotations(title = "Read OpenAPI endpoint", read_only_hint = true, destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn read_openapi_endpoint(&self, Parameters(p): Parameters<EndpointInput>) -> ToolResult<EndpointResponse> {
        Ok(Json(self.services.specification.read_endpoint(p.version.as_deref(), &p.path, &p.method).await?))
    }

    #[tool(
        name = "read_openapi_schema",
        description = "Return a component schema (raw JSON) by version and name, with provenance and the transitive closure of referenced schemas.",
        annotations(title = "Read OpenAPI schema", read_only_hint = true, destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn read_openapi_schema(&self, Parameters(p): Parameters<SchemaInput>) -> ToolResult<SchemaResponse> {
        Ok(Json(self.services.specification.read_schema(p.version.as_deref(), &p.name).await?))
    }

    #[tool(
        name = "get_endpoint_requirements",
        description = "PRIMARY Requirements Agent tool. Builds a deterministic evidence bundle for one endpoint: the OpenAPI operation, resolved schemas, \
curated requirements linked to the endpoint (authoritative_requirement), related full-text evidence (discovered_evidence), \
detected conflicts (declared, dangling reference, precedence tie, stale source) and a deduplicated citation list. \
Nothing is generated or interpreted; conflicts are reported, not resolved.",
        annotations(title = "Endpoint requirements evidence bundle", read_only_hint = true, destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn get_endpoint_requirements(
        &self,
        Parameters(p): Parameters<EndpointRequirementsInput>,
    ) -> ToolResult<EndpointEvidenceBundle> {
        let r = self
            .services
            .requirements
            .endpoint_requirements(p.version.as_deref(), &p.path, &p.method, p.evidence_limit)
            .await?;
        Ok(Json(r))
    }

    #[tool(
        name = "trace_requirement",
        description = "Trace a curated requirement: requirement -> specification sources (with resolved excerpts) -> OpenAPI endpoints -> schemas -> \
acceptance criteria -> related evidence -> conflicts. Every relationship carries provenance. Unknown ids return not_found (ids are never invented).",
        annotations(title = "Trace requirement", read_only_hint = true, destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn trace_requirement(&self, Parameters(p): Parameters<TraceRequirementInput>) -> ToolResult<RequirementTrace> {
        Ok(Json(self.services.requirements.trace(&p.requirement_id).await?))
    }

    #[tool(
        name = "compare_v1_v2",
        description = "Deterministic structural comparison of one endpoint between two versions (default: configured baseline -> target). \
Reports endpoint existence, operationId, parameters, request body, response codes, response schemas, security, and schema property/required/type/format/enum/nullable changes \
as added/removed/modified/unchanged with JSON pointers, plus factual compatibility-relevant facts and sources. No backward-compatibility verdict is produced.",
        annotations(title = "Compare endpoint across versions", read_only_hint = true, destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn compare_v1_v2(&self, Parameters(p): Parameters<CompareInput>) -> ToolResult<ComparisonReport> {
        let r = self
            .services
            .compatibility
            .compare(&p.path, &p.method, p.from_version.as_deref(), p.to_version.as_deref())
            .await?;
        Ok(Json(r))
    }

    #[tool(
        name = "list_sources",
        description = "List catalogued sources: source_id, version, kind, title, authority, precedence, corpus-relative path, sha256 and indexed status (including OCR-required pages).",
        annotations(title = "List sources", read_only_hint = true, destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn list_sources(&self, Parameters(p): Parameters<ListSourcesInput>) -> ToolResult<ListSourcesResponse> {
        Ok(Json(self.services.specification.list_sources(p.version.as_deref(), p.kind.as_deref()).await?))
    }

    #[tool(
        name = "health",
        description = "Operational status: read-only mode, index/schema versions, index generation and time, record counts and detected problems.",
        annotations(title = "Health", read_only_hint = true, destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn health(&self, Parameters(_): Parameters<HealthInput>) -> ToolResult<HealthReport> {
        Ok(Json(self.services.specification.health().await?))
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for BgSpecServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("berlin-group-spec", env!("CARGO_PKG_VERSION")))
            .with_instructions(SERVER_INSTRUCTIONS)
    }
}
