use super::{Provenance, SpecificationVersion};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// Schema reference or inline schema attached to a parameter / media type.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SchemaSlot {
    /// Component schema name when the slot is a `$ref` to `#/components/schemas/<name>`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_ref: Option<String>,
    /// Schema JSON exactly as declared (may contain `$ref`s).
    pub schema: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct NormalizedParameter {
    pub name: String,
    /// `path`, `query`, `header` or `cookie`.
    pub location: String,
    pub required: bool,
    pub deprecated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<SchemaSlot>,
    /// `#/components/parameters/<name>` when the parameter was referenced.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub component: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct MediaTypeSchema {
    pub media_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<SchemaSlot>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct NormalizedRequestBody {
    pub required: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub content: Vec<MediaTypeSchema>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub component: Option<String>,
}

/// A response header with its `$ref` resolved.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct NormalizedHeader {
    pub name: String,
    pub required: bool,
    pub deprecated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<SchemaSlot>,
    /// `#/components/headers/<name>` when the header was referenced.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub component: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct NormalizedResponse {
    /// Status code (`200`, `4XX`) or `default`.
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Header names (sorted); full definitions are in `header_definitions`.
    pub headers: Vec<String>,
    /// Header definitions in the order of `headers`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub header_definitions: Vec<NormalizedHeader>,
    pub content: Vec<MediaTypeSchema>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub component: Option<String>,
}

/// A `$ref` inside an operation that could not be resolved; the referencing element is omitted.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct UnresolvedReference {
    /// Where the reference occurs, e.g. `parameters`, `responses/404`, `responses/200/headers/X-Request-ID`.
    pub location: String,
    pub reason: String,
}

/// One security requirement alternative: scheme name -> scopes.
pub type SecurityRequirement = BTreeMap<String, Vec<String>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SecurityOrigin {
    Operation,
    Global,
    None,
}

/// A normalized OpenAPI operation, indexed as a semantic unit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct OpenApiOperation {
    pub version: SpecificationVersion,
    pub path: String,
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub tags: Vec<String>,
    pub deprecated: bool,
    /// Path-level and operation-level parameters merged (operation wins).
    pub parameters: Vec<NormalizedParameter>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_body: Option<NormalizedRequestBody>,
    pub responses: Vec<NormalizedResponse>,
    /// Effective security requirements (operation-level, or inherited global).
    pub security: Vec<SecurityRequirement>,
    pub security_origin: SecurityOrigin,
    /// Component schemas directly referenced by this operation.
    pub referenced_schemas: Vec<String>,
    /// References that could not be resolved (the affected elements are omitted above).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unresolved_references: Vec<UnresolvedReference>,
    /// JSON pointer of the operation inside the source document.
    pub json_pointer: String,
    pub provenance: Provenance,
}

/// A reusable OpenAPI component schema, indexed separately.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct OpenApiSchema {
    pub version: SpecificationVersion,
    pub name: String,
    pub schema: Value,
    /// Component schemas directly referenced by this schema.
    pub referenced_schemas: Vec<String>,
    pub json_pointer: String,
    pub provenance: Provenance,
}

/// Collects `#/components/schemas/<name>` references found anywhere in `value`.
pub fn collect_schema_refs(value: &Value, out: &mut std::collections::BTreeSet<String>) {
    match value {
        Value::Object(map) => {
            if let Some(Value::String(r)) = map.get("$ref")
                && let Some(name) = schema_ref_name(r)
            {
                out.insert(name.to_owned());
            }
            for v in map.values() {
                collect_schema_refs(v, out);
            }
        }
        Value::Array(items) => items.iter().for_each(|v| collect_schema_refs(v, out)),
        _ => {}
    }
}

/// Returns the component name of a local schema reference.
pub fn schema_ref_name(reference: &str) -> Option<&str> {
    reference
        .strip_prefix("#/components/schemas/")
        .or_else(|| reference.strip_prefix("#/definitions/"))
        .filter(|n| !n.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn collects_refs() {
        let v = json!({"allOf":[{"$ref":"#/components/schemas/A"}],
            "properties":{"b":{"items":{"$ref":"#/components/schemas/B"}}, "c":{"$ref":"#/components/parameters/X"}}});
        let mut out = std::collections::BTreeSet::new();
        collect_schema_refs(&v, &mut out);
        assert_eq!(out.into_iter().collect::<Vec<_>>(), vec!["A", "B"]);
    }
}
