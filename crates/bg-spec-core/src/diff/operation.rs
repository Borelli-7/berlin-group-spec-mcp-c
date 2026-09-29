use super::schema::{SchemaSet, diff_schemas};
use crate::{
    domain::{
        ChangeArea, ChangeSide, ChangeType, CompatibilityChange, MediaTypeSchema, NormalizedParameter,
        OpenApiOperation, SchemaChange,
    },
    openapi_path::template_params,
};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

/// One side of an operation comparison: the operation plus the schemas it can reference.
pub type OperationSide<'a> = Option<(&'a OpenApiOperation, &'a SchemaSet)>;

/// Compares two operations. `None` means the operation does not exist in that version.
pub fn diff_operations(old: OperationSide<'_>, new: OperationSide<'_>) -> Vec<CompatibilityChange> {
    let mut out = Vec::new();
    let (a, sa, b, sb) = match (old, new) {
        (None, None) => return out,
        (None, Some((b, _))) => {
            out.push(change(ChangeArea::Endpoint, ChangeSide::Endpoint, ChangeType::Added,
                endpoint_label(b), None, Some(json!(endpoint_label(b)))));
            return out;
        }
        (Some((a, _)), None) => {
            out.push(change(ChangeArea::Endpoint, ChangeSide::Endpoint, ChangeType::Removed,
                endpoint_label(a), Some(json!(endpoint_label(a))), None));
            return out;
        }
        (Some((a, sa)), Some((b, sb))) => (a, sa, b, sb),
    };

    out.push(change(ChangeArea::Endpoint, ChangeSide::Endpoint, ChangeType::Unchanged,
        endpoint_label(b), None, None));
    if a.path != b.path {
        out.push(change(ChangeArea::Path, ChangeSide::Endpoint, ChangeType::Modified, "path".into(),
            Some(json!(a.path)), Some(json!(b.path))));
    }
    out.push(scalar(ChangeArea::OperationId, ChangeSide::Endpoint, "operationId",
        a.operation_id.as_ref().map(|s| json!(s)), b.operation_id.as_ref().map(|s| json!(s))));
    if a.deprecated != b.deprecated {
        out.push(change(ChangeArea::Deprecated, ChangeSide::Endpoint, ChangeType::Modified, "deprecated".into(),
            Some(json!(a.deprecated)), Some(json!(b.deprecated))));
    }
    let (ta, tb): (BTreeSet<_>, BTreeSet<_>) = (a.tags.iter().collect(), b.tags.iter().collect());
    if ta != tb {
        out.push(change(ChangeArea::Tags, ChangeSide::Endpoint, ChangeType::Modified, "tags".into(),
            Some(json!(ta)), Some(json!(tb))));
    }

    diff_parameters(&mut out, a, sa, b, sb);
    diff_request_body(&mut out, a, sa, b, sb);
    diff_responses(&mut out, a, sa, b, sb);

    let norm = |op: &OpenApiOperation| {
        let mut v: Vec<Value> = op.security.iter().map(|s| json!(s)).collect();
        v.sort_by_key(|x| x.to_string());
        v
    };
    let (seca, secb) = (norm(a), norm(b));
    out.push(change(ChangeArea::Security, ChangeSide::Request,
        if seca == secb { ChangeType::Unchanged } else { ChangeType::Modified },
        "security".into(),
        (seca != secb).then(|| json!(seca)),
        (seca != secb).then(|| json!(secb))));
    out
}

fn endpoint_label(op: &OpenApiOperation) -> String {
    format!("{} {}", op.method, op.path)
}

fn change(area: ChangeArea, side: ChangeSide, change: ChangeType, subject: String,
    before: Option<Value>, after: Option<Value>) -> CompatibilityChange {
    CompatibilityChange { area, side, change, subject, before, after, schema_changes: Vec::new() }
}

fn scalar(area: ChangeArea, side: ChangeSide, subject: &str, a: Option<Value>, b: Option<Value>) -> CompatibilityChange {
    let kind = match (&a, &b) {
        (None, None) => ChangeType::Unchanged,
        (None, Some(_)) => ChangeType::Added,
        (Some(_), None) => ChangeType::Removed,
        (Some(x), Some(y)) if x == y => ChangeType::Unchanged,
        _ => ChangeType::Modified,
    };
    let differs = kind != ChangeType::Unchanged;
    change(area, side, kind, subject.to_owned(), a.filter(|_| differs), b.filter(|_| differs))
}

/// Stable comparison key. Path parameters are matched by template position so that a
/// renamed placeholder (`{account-id}` -> `{accountId}`) is reported as a modification.
fn param_key(op: &OpenApiOperation, p: &NormalizedParameter) -> String {
    if p.location == "path"
        && let Some(pos) = template_params(&op.path).iter().position(|n| n == &p.name)
    {
        return format!("path:#{pos}");
    }
    let name = if p.location == "header" { p.name.to_ascii_lowercase() } else { p.name.clone() };
    format!("{}:{name}", p.location)
}

fn param_summary(p: &NormalizedParameter) -> Value {
    json!({"name": p.name, "in": p.location, "required": p.required, "deprecated": p.deprecated,
        "schema": p.schema.as_ref().map(|s| &s.schema)})
}

fn slot_diff(a: Option<&Value>, b: Option<&Value>, sa: &SchemaSet, sb: &SchemaSet) -> Vec<SchemaChange> {
    let empty = json!({});
    diff_schemas(a.unwrap_or(&empty), b.unwrap_or(&empty), sa, sb)
}

fn diff_parameters(out: &mut Vec<CompatibilityChange>, a: &OpenApiOperation, sa: &SchemaSet,
    b: &OpenApiOperation, sb: &SchemaSet) {
    let pa: BTreeMap<String, &NormalizedParameter> = a.parameters.iter().map(|p| (param_key(a, p), p)).collect();
    let pb: BTreeMap<String, &NormalizedParameter> = b.parameters.iter().map(|p| (param_key(b, p), p)).collect();
    let keys: BTreeSet<&String> = pa.keys().chain(pb.keys()).collect();
    for k in keys {
        match (pa.get(k), pb.get(k)) {
            (None, Some(p)) => out.push(change(ChangeArea::Parameter, ChangeSide::Request, ChangeType::Added,
                format!("{}:{}", p.location, p.name), None, Some(param_summary(p)))),
            (Some(p), None) => out.push(change(ChangeArea::Parameter, ChangeSide::Request, ChangeType::Removed,
                format!("{}:{}", p.location, p.name), Some(param_summary(p)), None)),
            (Some(x), Some(y)) => {
                let schema_changes = slot_diff(x.schema.as_ref().map(|s| &s.schema),
                    y.schema.as_ref().map(|s| &s.schema), sa, sb);
                let meta_differs = x.name != y.name || x.required != y.required || x.deprecated != y.deprecated;
                let mut c = change(ChangeArea::Parameter, ChangeSide::Request,
                    if meta_differs || !schema_changes.is_empty() { ChangeType::Modified } else { ChangeType::Unchanged },
                    format!("{}:{}", y.location, y.name), None, None);
                if meta_differs {
                    c.before = Some(json!({"name": x.name, "required": x.required, "deprecated": x.deprecated}));
                    c.after = Some(json!({"name": y.name, "required": y.required, "deprecated": y.deprecated}));
                }
                c.schema_changes = schema_changes;
                out.push(c);
            }
            (None, None) => {}
        }
    }
}

fn diff_content(out: &mut Vec<CompatibilityChange>, side: ChangeSide, prefix: &str,
    ca: &[MediaTypeSchema], cb: &[MediaTypeSchema], sa: &SchemaSet, sb: &SchemaSet) -> bool {
    let ma: BTreeMap<&str, &MediaTypeSchema> = ca.iter().map(|m| (m.media_type.as_str(), m)).collect();
    let mb: BTreeMap<&str, &MediaTypeSchema> = cb.iter().map(|m| (m.media_type.as_str(), m)).collect();
    let keys: BTreeSet<&&str> = ma.keys().chain(mb.keys()).collect();
    let mut changed = false;
    for k in keys {
        let subject = format!("{prefix} {k}");
        match (ma.get(*k), mb.get(*k)) {
            (None, Some(m)) => {
                changed = true;
                out.push(change(ChangeArea::MediaType, side, ChangeType::Added, subject, None,
                    Some(json!(m.schema.as_ref().map(|s| &s.schema)))));
            }
            (Some(m), None) => {
                changed = true;
                out.push(change(ChangeArea::MediaType, side, ChangeType::Removed, subject,
                    Some(json!(m.schema.as_ref().map(|s| &s.schema))), None));
            }
            (Some(x), Some(y)) => {
                let sc = slot_diff(x.schema.as_ref().map(|s| &s.schema), y.schema.as_ref().map(|s| &s.schema), sa, sb);
                if !sc.is_empty() {
                    changed = true;
                    let mut c = change(ChangeArea::MediaType, side, ChangeType::Modified, subject, None, None);
                    c.schema_changes = sc;
                    out.push(c);
                }
            }
            (None, None) => {}
        }
    }
    changed
}

fn diff_request_body(out: &mut Vec<CompatibilityChange>, a: &OpenApiOperation, sa: &SchemaSet,
    b: &OpenApiOperation, sb: &SchemaSet) {
    match (&a.request_body, &b.request_body) {
        (None, None) => {}
        (None, Some(rb)) => out.push(change(ChangeArea::RequestBody, ChangeSide::Request, ChangeType::Added,
            "requestBody".into(), None, Some(json!({"required": rb.required, "media_types": rb.content.iter().map(|m| &m.media_type).collect::<Vec<_>>()})))),
        (Some(ra), None) => out.push(change(ChangeArea::RequestBody, ChangeSide::Request, ChangeType::Removed,
            "requestBody".into(), Some(json!({"required": ra.required})), None)),
        (Some(ra), Some(rb)) => {
            let content_changed = diff_content(out, ChangeSide::Request, "requestBody", &ra.content, &rb.content, sa, sb);
            if ra.required != rb.required {
                out.push(change(ChangeArea::RequestBody, ChangeSide::Request, ChangeType::Modified, "requestBody.required".into(),
                    Some(json!(ra.required)), Some(json!(rb.required))));
            } else if !content_changed {
                out.push(change(ChangeArea::RequestBody, ChangeSide::Request, ChangeType::Unchanged, "requestBody".into(), None, None));
            }
        }
    }
}

fn diff_responses(out: &mut Vec<CompatibilityChange>, a: &OpenApiOperation, sa: &SchemaSet,
    b: &OpenApiOperation, sb: &SchemaSet) {
    let ra: BTreeMap<&str, _> = a.responses.iter().map(|r| (r.status.as_str(), r)).collect();
    let rb: BTreeMap<&str, _> = b.responses.iter().map(|r| (r.status.as_str(), r)).collect();
    let keys: BTreeSet<&&str> = ra.keys().chain(rb.keys()).collect();
    for k in keys {
        let subject = format!("response:{k}");
        match (ra.get(*k), rb.get(*k)) {
            (None, Some(r)) => out.push(change(ChangeArea::Response, ChangeSide::Response, ChangeType::Added, subject, None,
                Some(json!({"description": r.description, "media_types": r.content.iter().map(|m| &m.media_type).collect::<Vec<_>>()})))),
            (Some(r), None) => out.push(change(ChangeArea::Response, ChangeSide::Response, ChangeType::Removed, subject,
                Some(json!({"description": r.description})), None)),
            (Some(x), Some(y)) => {
                let mut changed = diff_content(out, ChangeSide::Response, &subject, &x.content, &y.content, sa, sb);
                let ha: BTreeSet<String> = x.headers.iter().map(|h| h.to_ascii_lowercase()).collect();
                let hb: BTreeSet<String> = y.headers.iter().map(|h| h.to_ascii_lowercase()).collect();
                for h in hb.difference(&ha) {
                    changed = true;
                    out.push(change(ChangeArea::ResponseHeader, ChangeSide::Response, ChangeType::Added,
                        format!("{subject} header:{h}"), None, Some(json!(h))));
                }
                for h in ha.difference(&hb) {
                    changed = true;
                    out.push(change(ChangeArea::ResponseHeader, ChangeSide::Response, ChangeType::Removed,
                        format!("{subject} header:{h}"), Some(json!(h)), None));
                }
                if !changed {
                    out.push(change(ChangeArea::Response, ChangeSide::Response, ChangeType::Unchanged, subject, None, None));
                }
            }
            (None, None) => {}
        }
    }
}
