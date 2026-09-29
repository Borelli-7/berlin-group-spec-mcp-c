//! Typed tool inputs. Unknown fields are rejected so that typos surface as errors instead of
//! silently widening a query.

use schemars::JsonSchema;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SearchSpecificationInput {
    /// Full-text query (BM25). Supports phrases in double quotes.
    pub query: String,
    /// Specification version filter, e.g. `openfinance-v2` or `nextgenpsd2-v1.3`.
    #[serde(default)]
    pub version: Option<String>,
    /// Kind filter: `pdf`, `openapi`, `text` or `requirements`.
    #[serde(default)]
    pub kind: Option<String>,
    /// Restrict to one source id (see `list_sources`).
    #[serde(default)]
    pub source_id: Option<String>,
    /// Maximum number of results (clamped to the configured maximum).
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FindRequirementInput {
    /// Requirement id (e.g. `OFV2-TRANSACTIONS-001`) or free text.
    pub query: String,
    /// Optional version filter.
    #[serde(default)]
    pub version: Option<String>,
    /// Maximum number of discovered-evidence results.
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReadSourceInput {
    /// Stable source id from the corpus manifest. Filesystem paths are not accepted.
    pub source_id: String,
    /// Locator: `page:N`, `page:N-M`, `section:4.2`, `chunk:<id>`, `op:GET /path`,
    /// `path:/path` or `schema:Name`.
    pub locator: String,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EndpointInput {
    /// Specification version; defaults to the configured target version.
    #[serde(default)]
    pub version: Option<String>,
    /// OpenAPI path template, e.g. `/accounts/{accountId}/transactions`.
    pub path: String,
    /// HTTP method, e.g. `GET`.
    pub method: String,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EndpointRequirementsInput {
    /// Specification version; defaults to the configured target version.
    #[serde(default)]
    pub version: Option<String>,
    /// OpenAPI path template, e.g. `/accounts/{accountId}/transactions`.
    pub path: String,
    /// HTTP method, e.g. `GET`.
    pub method: String,
    /// Maximum number of related full-text evidence items.
    #[serde(default)]
    pub evidence_limit: Option<usize>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SchemaInput {
    /// Specification version; defaults to the configured target version.
    #[serde(default)]
    pub version: Option<String>,
    /// Component schema name, e.g. `Transactions`.
    pub name: String,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TraceRequirementInput {
    /// Curated requirement id, e.g. `OFV2-TRANSACTIONS-001`.
    pub requirement_id: String,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CompareInput {
    /// Path template in the target version, e.g. `/accounts/{accountId}/transactions`.
    /// Configured version path prefixes (e.g. `/v1`) are applied automatically.
    pub path: String,
    /// HTTP method, e.g. `GET`.
    pub method: String,
    /// Baseline version; defaults to the configured baseline (`nextgenpsd2-v1.3`).
    #[serde(default)]
    pub from_version: Option<String>,
    /// Target version; defaults to the configured target (`openfinance-v2`).
    #[serde(default)]
    pub to_version: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListSourcesInput {
    /// Optional version filter.
    #[serde(default)]
    pub version: Option<String>,
    /// Optional kind filter.
    #[serde(default)]
    pub kind: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HealthInput {}
