//! Curated requirement mappings (`corpus/requirements/requirements.yaml`).

use crate::{
    CoreError, Result,
    domain::{MappingProvenance, Requirement, RequirementRecord, SourceLocator},
    hash::sha256_hex,
};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequirementsFile {
    #[serde(default = "one")]
    pub schema_version: u32,
    pub requirements: Vec<Requirement>,
}

fn one() -> u32 {
    1
}

/// Valid requirement identifier: `[A-Z0-9][A-Z0-9._-]*` (upper case).
pub fn is_valid_requirement_id(id: &str) -> bool {
    let mut chars = id.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_uppercase() || c.is_ascii_digit())
        && id.len() <= 128
        && chars
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-'))
}

impl RequirementsFile {
    /// Parses and structurally validates the mapping file.
    ///
    /// References to unknown sources/endpoints are *not* rejected here: they are surfaced
    /// at query time as `dangling_reference` conflicts so that curators can see them.
    pub fn parse(yaml: &str) -> Result<Self> {
        let file: RequirementsFile =
            serde_saphyr::from_str(yaml).map_err(|e| CoreError::Requirements(e.to_string()))?;
        file.validate()?;
        Ok(file)
    }

    fn validate(&self) -> Result<()> {
        if self.schema_version != 1 {
            return Err(CoreError::Requirements(format!(
                "unsupported schema_version {}",
                self.schema_version
            )));
        }
        let mut ids = HashSet::new();
        for r in &self.requirements {
            let err = |msg: String| CoreError::Requirements(format!("{}: {msg}", r.id));
            if !is_valid_requirement_id(&r.id) {
                return Err(err(
                    "invalid requirement id (expected [A-Z0-9][A-Z0-9._-]*)".into(),
                ));
            }
            if !ids.insert(r.id.as_str()) {
                return Err(err("duplicate requirement id".into()));
            }
            if r.title.trim().is_empty() {
                return Err(err("title must not be empty".into()));
            }
            if r.sources.is_empty() {
                return Err(err(
                    "at least one source is required (no requirement without provenance)".into(),
                ));
            }
            for s in &r.sources {
                s.locator
                    .parse::<SourceLocator>()
                    .map_err(|e| err(format!("source {}: {e}", s.source_id)))?;
                if let Some(pin) = &s.sha256
                    && (pin.len() < 12 || !pin.chars().all(|c| c.is_ascii_hexdigit()))
                {
                    return Err(err(format!(
                        "source {}: sha256 pin must be >= 12 hex chars",
                        s.source_id
                    )));
                }
            }
            for c in &r.conflicts_with {
                if c.requirement_id == r.id {
                    return Err(err("conflicts_with must not reference itself".into()));
                }
            }
        }
        Ok(())
    }

    /// Attaches mapping provenance to each requirement.
    pub fn into_records(self, file: &str, file_sha256: &str) -> Result<Vec<RequirementRecord>> {
        self.requirements
            .into_iter()
            .map(|r| {
                let json =
                    serde_json::to_vec(&r).map_err(|e| CoreError::Requirements(e.to_string()))?;
                Ok(RequirementRecord {
                    mapping: MappingProvenance {
                        file: file.to_owned(),
                        file_sha256: file_sha256.to_owned(),
                        locator: format!("requirement:{}", r.id),
                        sha256: sha256_hex(json),
                    },
                    requirement: r,
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const YAML: &str = r#"
requirements:
  - id: OFV2-TRANSACTIONS-001
    version: openfinance-v2
    title: Transaction endpoint requirement
    sources:
      - source_id: bg-openfinance-v2-implementation-guidelines
        locator: page:3
      - source_id: bg-openfinance-v2-openapi
        locator: path:/accounts/{accountId}/transactions
    endpoints:
      - GET /accounts/{accountId}/transactions
    acceptance_criteria:
      - "dateFrom is mandatory"
"#;

    #[test]
    fn parses() {
        let f = RequirementsFile::parse(YAML).unwrap();
        let r = &f.requirements[0];
        assert_eq!(r.id, "OFV2-TRANSACTIONS-001");
        assert_eq!(r.endpoints[0].method, "GET");
        let recs = f
            .clone()
            .into_records("requirements/requirements.yaml", "abc")
            .unwrap();
        assert_eq!(recs[0].mapping.locator, "requirement:OFV2-TRANSACTIONS-001");
        assert_eq!(recs[0].mapping.sha256.len(), 64);
    }

    #[test]
    fn rejects_invalid() {
        assert!(RequirementsFile::parse(&YAML.replace("page:3", "page:zero")).is_err());
        assert!(RequirementsFile::parse(&YAML.replace("OFV2-TRANSACTIONS-001", "bad id")).is_err());
        let no_sources = r#"
requirements:
  - id: X-1
    version: openfinance-v2
    title: t
    sources: []
"#;
        assert!(RequirementsFile::parse(no_sources).is_err());
        assert!(RequirementsFile::parse(&YAML.replace("dateFrom", "x\"\n    bogus: [")).is_err());
    }
}
