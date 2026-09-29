use crate::{CoreError, Result};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::fmt;

/// Identifier of a specification version, e.g. `nextgenpsd2-v1.3` or `openfinance-v2`.
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(try_from = "String", into = "String")]
pub struct SpecificationVersion(String);

impl SpecificationVersion {
    pub fn new(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        let valid = !value.is_empty()
            && value.len() <= 64
            && value
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'));
        if valid {
            Ok(Self(value))
        } else {
            Err(CoreError::InvalidInput(format!(
                "invalid specification version '{value}' (expected 1-64 chars of [A-Za-z0-9._-])"
            )))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for SpecificationVersion {
    type Error = CoreError;
    fn try_from(value: String) -> Result<Self> {
        Self::new(value)
    }
}

impl From<SpecificationVersion> for String {
    fn from(v: SpecificationVersion) -> Self {
        v.0
    }
}

impl fmt::Display for SpecificationVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates() {
        assert!(SpecificationVersion::new("openfinance-v2").is_ok());
        assert!(SpecificationVersion::new("nextgenpsd2-v1.3").is_ok());
        assert!(SpecificationVersion::new("").is_err());
        assert!(SpecificationVersion::new("../etc").is_err());
        assert!(SpecificationVersion::new("a b").is_err());
    }
}
