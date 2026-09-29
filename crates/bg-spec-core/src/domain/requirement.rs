use super::SpecificationVersion;
use crate::{CoreError, Result, openapi_path};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::fmt;

/// Endpoint reference `METHOD /path` used in curated mappings.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(try_from = "String", into = "String")]
#[schemars(with = "String")]
pub struct EndpointRef {
    pub method: String,
    pub path: String,
}

impl EndpointRef {
    pub fn new(method: &str, path: &str) -> Result<Self> {
        Ok(Self {
            method: openapi_path::normalize_method(method)?,
            path: openapi_path::validate_path(path)?,
        })
    }
}

impl TryFrom<String> for EndpointRef {
    type Error = CoreError;
    fn try_from(s: String) -> Result<Self> {
        let (m, p) = s.trim().split_once(char::is_whitespace).ok_or_else(|| {
            CoreError::InvalidInput(format!("endpoint '{s}' must be '<METHOD> <path>'"))
        })?;
        Self::new(m, p.trim())
    }
}

impl From<EndpointRef> for String {
    fn from(e: EndpointRef) -> Self {
        e.to_string()
    }
}

impl fmt::Display for EndpointRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.method, self.path)
    }
}

/// Citation from a curated requirement to a specification source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RequirementSource {
    pub source_id: String,
    /// Locator string (see `SourceLocator`).
    pub locator: String,
    /// Optional pin: SHA-256 (or >= 12 char prefix) of the source document at curation time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// Explicitly curated conflict between two requirements.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeclaredConflict {
    pub requirement_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// A curated requirement mapping (authoritative bridge requirement -> source -> endpoint -> AC).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Requirement {
    pub id: String,
    pub version: SpecificationVersion,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub sources: Vec<RequirementSource>,
    /// Explicit endpoint links (in addition to `op:`/`path:` source locators).
    #[serde(default)]
    pub endpoints: Vec<EndpointRef>,
    /// Explicit schema links (in addition to `schema:` source locators).
    #[serde(default)]
    pub schemas: Vec<String>,
    #[serde(default)]
    pub acceptance_criteria: Vec<String>,
    #[serde(default)]
    pub conflicts_with: Vec<DeclaredConflict>,
    #[serde(default)]
    pub tags: Vec<String>,
}

/// Provenance of a curated mapping record inside the requirements file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MappingProvenance {
    /// Corpus-relative path of the requirements file.
    pub file: String,
    /// SHA-256 of the requirements file.
    pub file_sha256: String,
    /// `requirement:<ID>`
    pub locator: String,
    /// SHA-256 of the normalized requirement record.
    pub sha256: String,
}

/// A curated requirement together with its mapping provenance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RequirementRecord {
    pub requirement: Requirement,
    pub mapping: MappingProvenance,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_ref_parses() {
        let e = EndpointRef::try_from("get /accounts/{accountId}".to_owned()).unwrap();
        assert_eq!(e.to_string(), "GET /accounts/{accountId}");
        assert!(EndpointRef::try_from("/accounts".to_owned()).is_err());
    }
}
