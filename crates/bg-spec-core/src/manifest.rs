//! Corpus manifest (`corpus/manifest.yaml`). Authority is always explicit.

use crate::{
    CoreError, Result,
    domain::{DocumentAuthority, DocumentKind, SpecificationVersion},
};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Component, Path};

pub const MAX_PRECEDENCE: u32 = 1000;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    #[serde(default = "one")]
    pub schema_version: u32,
    pub sources: Vec<ManifestSource>,
}

fn one() -> u32 {
    1
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManifestSource {
    pub id: String,
    pub kind: DocumentKind,
    pub version: SpecificationVersion,
    pub authority: DocumentAuthority,
    pub precedence: u32,
    /// Path relative to the corpus root.
    pub path: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
}

impl ManifestSource {
    pub fn display_title(&self) -> &str {
        self.title.as_deref().unwrap_or(&self.id)
    }
}

/// Valid stable source identifier: `[a-z0-9][a-z0-9._-]{0,127}`.
pub fn is_valid_source_id(id: &str) -> bool {
    let mut chars = id.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_lowercase() || c.is_ascii_digit())
        && id.len() <= 128
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-'))
}

/// Validates that `path` is relative and cannot escape its root lexically.
pub fn validate_relative_path(path: &str) -> Result<()> {
    let p = Path::new(path);
    if path.is_empty()
        || p.is_absolute()
        || path.contains('\\')
        || p.components().any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err(CoreError::Manifest(format!(
            "path '{path}' must be a relative path without '.', '..' or root components"
        )));
    }
    Ok(())
}

impl Manifest {
    pub fn parse(yaml: &str) -> Result<Self> {
        let manifest: Manifest =
            serde_saphyr::from_str(yaml).map_err(|e| CoreError::Manifest(e.to_string()))?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn validate(&self) -> Result<()> {
        if self.schema_version != 1 {
            return Err(CoreError::Manifest(format!(
                "unsupported manifest schema_version {}",
                self.schema_version
            )));
        }
        let mut ids = HashSet::new();
        let mut paths = HashSet::new();
        for s in &self.sources {
            if !is_valid_source_id(&s.id) {
                return Err(CoreError::Manifest(format!(
                    "invalid source id '{}' (expected [a-z0-9][a-z0-9._-]*)",
                    s.id
                )));
            }
            if !ids.insert(s.id.as_str()) {
                return Err(CoreError::Manifest(format!("duplicate source id '{}'", s.id)));
            }
            validate_relative_path(&s.path)?;
            if !paths.insert(s.path.as_str()) {
                return Err(CoreError::Manifest(format!(
                    "path '{}' is declared by more than one source",
                    s.path
                )));
            }
            if s.precedence > MAX_PRECEDENCE {
                return Err(CoreError::Manifest(format!(
                    "source '{}': precedence must be 0..={MAX_PRECEDENCE}",
                    s.id
                )));
            }
        }
        Ok(())
    }

    pub fn source(&self, id: &str) -> Option<&ManifestSource> {
        self.sources.iter().find(|s| s.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const YAML: &str = r#"
sources:
  - id: bg-openfinance-v2-implementation-guidelines
    kind: pdf
    version: openfinance-v2
    authority: normative
    precedence: 100
    path: pdf/v2/implementation-guidelines.pdf
  - id: bg-openfinance-v2-openapi
    kind: openapi
    version: openfinance-v2
    authority: technical
    precedence: 90
    path: openapi/v2/openfinance.yaml
    title: Open Finance V2 OpenAPI
"#;

    #[test]
    fn parses_valid_manifest() {
        let m = Manifest::parse(YAML).unwrap();
        assert_eq!(m.sources.len(), 2);
        let s = m.source("bg-openfinance-v2-openapi").unwrap();
        assert_eq!(s.kind, DocumentKind::Openapi);
        assert_eq!(s.authority, DocumentAuthority::Technical);
        assert_eq!(s.display_title(), "Open Finance V2 OpenAPI");
        assert_eq!(
            m.sources[0].display_title(),
            "bg-openfinance-v2-implementation-guidelines"
        );
    }

    #[test]
    fn authority_is_mandatory() {
        let y = YAML.replace("    authority: normative\n", "");
        assert!(Manifest::parse(&y).is_err());
    }

    #[test]
    fn rejects_unknown_authority_and_fields() {
        assert!(Manifest::parse(&YAML.replace("normative", "official")).is_err());
        assert!(Manifest::parse(&YAML.replace("precedence: 90", "precedence: 90\n    color: red")).is_err());
    }

    #[test]
    fn rejects_escaping_paths() {
        for p in ["../secret.pdf", "/etc/passwd", "a/../../b", "./a.pdf", "a\\b.pdf"] {
            let y = YAML.replace("pdf/v2/implementation-guidelines.pdf", p);
            assert!(Manifest::parse(&y).is_err(), "{p}");
        }
    }

    #[test]
    fn rejects_duplicates_and_bad_ids() {
        let dup = YAML.replace("bg-openfinance-v2-openapi", "bg-openfinance-v2-implementation-guidelines");
        assert!(Manifest::parse(&dup).is_err());
        assert!(Manifest::parse(&YAML.replace("bg-openfinance-v2-openapi", "Bad ID")).is_err());
        assert!(Manifest::parse(&YAML.replace("precedence: 90", "precedence: 5000")).is_err());
    }
}
