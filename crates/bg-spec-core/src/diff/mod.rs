//! Deterministic structural comparison of OpenAPI schemas and operations. No heuristics,
//! no compatibility verdicts: only concrete differences.

mod facts;
mod operation;
mod schema;

pub use facts::{CompatibilityFact, derive_facts};
pub use operation::diff_operations;
pub use schema::{SchemaSet, diff_schemas};

/// Escapes one JSON-pointer reference token.
pub(crate) fn escape_token(token: &str) -> String {
    token.replace('~', "~0").replace('/', "~1")
}
