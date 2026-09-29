//! OpenAPI ingestion.
//!
//! * OpenAPI 3.0.x documents are parsed strictly with [`openapiv3::OpenAPI`]; the typed
//!   model validates the structure and supplies API metadata.
//! * Normalized records are projected from the raw JSON tree, so schema JSON is exactly
//!   what the source declares (typed re-serialization would drop keywords such as
//!   `const` or `examples` and break provenance fidelity).
//! * OpenAPI 3.1 documents (unsupported by `openapiv3`) and 3.0 documents rejected by the
//!   typed parser use the same projector and receive a diagnostic.

mod normalize;
mod text;

pub use normalize::{NormalizedApi, normalize};
pub use text::{operation_search_text, schema_search_text};

use bg_spec_core::{CoreError, Result};
use serde_json::Value;

/// Parses a YAML or JSON OpenAPI document into a JSON tree.
pub fn parse_document(raw: &str, file_name: &str) -> Result<Value> {
    let value: Value = if file_name.ends_with(".json") {
        serde_json::from_str(raw).map_err(|e| CoreError::Integrity(format!("{file_name}: invalid JSON: {e}")))?
    } else {
        serde_saphyr::from_str(raw).map_err(|e| CoreError::Integrity(format!("{file_name}: invalid YAML: {e}")))?
    };
    if !value.is_object() {
        return Err(CoreError::Integrity(format!("{file_name}: OpenAPI root must be a mapping")));
    }
    Ok(value)
}

/// OpenAPI dialect detected from the root `openapi` / `swagger` field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Dialect {
    V30(String),
    V31(String),
}

pub fn detect_dialect(root: &Value) -> Result<Dialect> {
    if root.get("swagger").is_some() {
        return Err(CoreError::Integrity("Swagger 2.0 documents are not supported; convert to OpenAPI 3".into()));
    }
    let v = root
        .get("openapi")
        .and_then(Value::as_str)
        .ok_or_else(|| CoreError::Integrity("missing 'openapi' version field".into()))?;
    if v.starts_with("3.0") {
        Ok(Dialect::V30(v.to_owned()))
    } else if v.starts_with("3.1") || v.starts_with("3.2") {
        Ok(Dialect::V31(v.to_owned()))
    } else {
        Err(CoreError::Integrity(format!("unsupported OpenAPI version '{v}'")))
    }
}
