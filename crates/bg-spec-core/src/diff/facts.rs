use crate::domain::{ChangeArea, ChangeSide, ChangeType, CompatibilityChange, SchemaChangeKind};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A neutral, factual statement derived mechanically from a change. Not a verdict.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CompatibilityFact {
    pub side: ChangeSide,
    pub subject: String,
    pub statement: String,
}

fn kind_text(k: SchemaChangeKind) -> &'static str {
    use SchemaChangeKind::*;
    match k {
        SchemaAdded => "schema added",
        SchemaRemoved => "schema removed",
        PropertyAdded => "property added",
        PropertyRemoved => "property removed",
        TypeChanged => "type changed",
        FormatChanged => "format changed",
        NullableChanged => "nullable changed",
        RequiredAdded => "property became required",
        RequiredRemoved => "property became optional",
        EnumValuesAdded => "enum values added",
        EnumValuesRemoved => "enum values removed",
        EnumConstraintChanged => "enum constraint added/removed",
        RefChanged => "referenced schema changed",
        CompositionChanged => "composition (allOf/oneOf/anyOf) changed",
        ConstraintChanged => "constraint changed",
        AdditionalPropertiesChanged => "additionalProperties changed",
    }
}

fn side_text(s: ChangeSide) -> &'static str {
    match s {
        ChangeSide::Endpoint => "endpoint",
        ChangeSide::Request => "request",
        ChangeSide::Response => "response",
    }
}

fn compact(v: &Option<serde_json::Value>) -> String {
    match v {
        Some(v) => {
            let s = v.to_string();
            if s.len() > 160 {
                format!("{}...", &s[..s.floor_char_boundary(157)])
            } else {
                s
            }
        }
        None => "-".to_owned(),
    }
}

/// Derives factual statements from every non-`unchanged` change (and nested schema change).
pub fn derive_facts(
    changes: &[CompatibilityChange],
    from: &str,
    to: &str,
) -> Vec<CompatibilityFact> {
    let mut facts = Vec::new();
    for c in changes {
        let side = side_text(c.side);
        match c.change {
            ChangeType::Unchanged => continue,
            ChangeType::Added if c.area == ChangeArea::Parameter => {
                let req = c
                    .after
                    .as_ref()
                    .and_then(|v| v.get("required"))
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                facts.push(CompatibilityFact {
                    side: c.side,
                    subject: c.subject.clone(),
                    statement: format!(
                        "{} {} parameter '{}' exists in {to} but not in {from}.",
                        if req { "Required" } else { "Optional" },
                        side,
                        c.subject
                    ),
                });
            }
            ChangeType::Added => facts.push(CompatibilityFact {
                side: c.side,
                subject: c.subject.clone(),
                statement: format!(
                    "{:?} '{}' exists in {to} but not in {from}.",
                    c.area, c.subject
                ),
            }),
            ChangeType::Removed => facts.push(CompatibilityFact {
                side: c.side,
                subject: c.subject.clone(),
                statement: format!(
                    "{:?} '{}' exists in {from} but not in {to}.",
                    c.area, c.subject
                ),
            }),
            ChangeType::Modified if c.before.is_some() || c.after.is_some() => {
                facts.push(CompatibilityFact {
                    side: c.side,
                    subject: c.subject.clone(),
                    statement: format!(
                        "{:?} '{}' changed from {} ({from}) to {} ({to}).",
                        c.area,
                        c.subject,
                        compact(&c.before),
                        compact(&c.after)
                    ),
                });
            }
            ChangeType::Modified => {}
        }
        for s in &c.schema_changes {
            let within = s
                .schema_name
                .as_deref()
                .map(|n| format!(" (schema {n})"))
                .unwrap_or_default();
            facts.push(CompatibilityFact {
                side: c.side,
                subject: c.subject.clone(),
                statement: format!(
                    "{} schema of '{}' at {}{}: {}; {from}={} {to}={}.",
                    side,
                    c.subject,
                    s.pointer,
                    within,
                    kind_text(s.kind),
                    compact(&s.before),
                    compact(&s.after)
                ),
            });
        }
    }
    facts
}
