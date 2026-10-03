use super::{COMPARISON_NOTICE, MatchedBy, SpecificationService};
use crate::{
    CoreError, Result,
    diff::{CompatibilityFact, derive_facts, diff_operations},
    domain::{
        ChangeType, CompatibilityChange, OpenApiOperation, Provenance, SchemaChange,
        SpecificationVersion,
    },
    openapi_path::{normalize_method, validate_path},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ChangeCounts {
    pub added: usize,
    pub removed: usize,
    pub modified: usize,
    pub unchanged: usize,
    pub schema_changes: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ComparedSide {
    pub version: SpecificationVersion,
    pub found: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matched_by: Option<MatchedBy>,
    pub unresolved_schemas: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ComparisonReport {
    pub method: String,
    pub path: String,
    pub from: ComparedSide,
    pub to: ComparedSide,
    pub notice: String,
    pub v1_operation: Option<OpenApiOperation>,
    pub v2_operation: Option<OpenApiOperation>,
    pub changes: Vec<CompatibilityChange>,
    /// All schema changes flattened (also nested under each change).
    pub schema_changes: Vec<SchemaChange>,
    pub counts: ChangeCounts,
    pub compatibility_relevant_facts: Vec<CompatibilityFact>,
    pub sources: Vec<Provenance>,
}

/// Deterministic V1/V2 structural comparison.
#[derive(Clone)]
pub struct CompatibilityService {
    spec: SpecificationService,
}

impl CompatibilityService {
    pub fn new(spec: SpecificationService) -> Self {
        Self { spec }
    }

    pub async fn compare(
        &self,
        path: &str,
        method: &str,
        from_version: Option<&str>,
        to_version: Option<&str>,
    ) -> Result<ComparisonReport> {
        let _timer = crate::timing::Timer::start("compatibility.compare");
        let method = normalize_method(method)?;
        let path = validate_path(path)?;
        let from = match from_version.map(str::trim).filter(|s| !s.is_empty()) {
            Some(v) => SpecificationVersion::new(v)?,
            None => self.spec.settings().baseline.clone(),
        };
        let to = self.spec.version_or_target(to_version)?;
        self.spec.ensure_known_version(&from).await?;
        self.spec.ensure_known_version(&to).await?;

        let a = self.spec.lookup_operation(&from, &method, &path).await?;
        let b = self.spec.lookup_operation(&to, &method, &path).await?;
        if a.is_none() && b.is_none() {
            return Err(CoreError::NotFound(format!(
                "{method} {path} exists in neither '{from}' nor '{to}'"
            )));
        }
        let closure_a = match &a {
            Some(l) => {
                self.spec
                    .schema_closure(
                        &l.primary.provenance.source_id,
                        &from,
                        l.primary.referenced_schemas.iter().cloned(),
                    )
                    .await?
            }
            None => Default::default(),
        };
        let closure_b = match &b {
            Some(l) => {
                self.spec
                    .schema_closure(
                        &l.primary.provenance.source_id,
                        &to,
                        l.primary.referenced_schemas.iter().cloned(),
                    )
                    .await?
            }
            None => Default::default(),
        };
        let (set_a, set_b) = (closure_a.schema_set(), closure_b.schema_set());
        let changes = diff_operations(
            a.as_ref().map(|l| (&l.primary, &set_a)),
            b.as_ref().map(|l| (&l.primary, &set_b)),
        );
        let schema_changes: Vec<SchemaChange> = changes
            .iter()
            .flat_map(|c| c.schema_changes.iter().cloned())
            .collect();
        let mut counts = ChangeCounts {
            schema_changes: schema_changes.len(),
            ..Default::default()
        };
        for c in &changes {
            match c.change {
                ChangeType::Added => counts.added += 1,
                ChangeType::Removed => counts.removed += 1,
                ChangeType::Modified => counts.modified += 1,
                ChangeType::Unchanged => counts.unchanged += 1,
            }
        }
        let facts = derive_facts(&changes, from.as_str(), to.as_str());
        let mut sources: Vec<Provenance> = Vec::new();
        for (l, c) in [(&a, &closure_a), (&b, &closure_b)] {
            if let Some(l) = l {
                sources.push(l.primary.provenance.clone());
                sources.extend(c.schemas.values().map(|s| s.provenance.clone()));
            }
        }
        Ok(ComparisonReport {
            method,
            path,
            from: ComparedSide {
                version: from,
                found: a.is_some(),
                matched_by: a.as_ref().map(|l| l.matched_by),
                unresolved_schemas: closure_a.unresolved.into_iter().collect(),
            },
            to: ComparedSide {
                version: to,
                found: b.is_some(),
                matched_by: b.as_ref().map(|l| l.matched_by),
                unresolved_schemas: closure_b.unresolved.into_iter().collect(),
            },
            notice: COMPARISON_NOTICE.to_owned(),
            v1_operation: a.map(|l| l.primary),
            v2_operation: b.map(|l| l.primary),
            changes,
            schema_changes,
            counts,
            compatibility_relevant_facts: facts,
            sources,
        })
    }
}
