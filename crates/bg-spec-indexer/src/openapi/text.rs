//! Search text for OpenAPI records (indexed in Tantivy alongside PDF chunks).

use bg_spec_core::domain::{OpenApiOperation, OpenApiSchema};
use serde_json::Value;
use std::fmt::Write;

pub fn operation_search_text(op: &OpenApiOperation) -> (String, String) {
    let label = op.summary.as_deref().or(op.operation_id.as_deref()).unwrap_or_default();
    let title = format!("{} {} {label}", op.method, op.path).trim_end().to_owned();
    let mut s = format!("{} {}\n", op.method, op.path);
    for v in [&op.operation_id, &op.summary, &op.description].into_iter().flatten() {
        let _ = writeln!(s, "{v}");
    }
    if !op.tags.is_empty() {
        let _ = writeln!(s, "Tags: {}", op.tags.join(", "));
    }
    for p in &op.parameters {
        let _ = writeln!(
            s,
            "Parameter {} ({}, {}){}",
            p.name,
            p.location,
            if p.required { "required" } else { "optional" },
            p.description.as_deref().map(|d| format!(": {d}")).unwrap_or_default()
        );
    }
    if let Some(body) = &op.request_body {
        let media: Vec<_> = body.content.iter().map(|m| m.media_type.as_str()).collect();
        let _ = writeln!(s, "Request body ({}): {}", if body.required { "required" } else { "optional" }, media.join(", "));
    }
    for r in &op.responses {
        let _ = writeln!(s, "Response {}: {}", r.status, r.description.as_deref().unwrap_or_default());
    }
    if !op.referenced_schemas.is_empty() {
        let _ = writeln!(s, "Schemas: {}", op.referenced_schemas.join(", "));
    }
    (title, s)
}

fn collect_text(v: &Value, key: Option<&str>, out: &mut String) {
    match v {
        Value::Object(m) => {
            for (k, v) in m {
                if k == "properties" {
                    if let Some(props) = v.as_object() {
                        for (name, prop) in props {
                            let _ = writeln!(out, "property {name}");
                            collect_text(prop, None, out);
                        }
                    }
                } else {
                    collect_text(v, Some(k), out);
                }
            }
        }
        Value::Array(items) => items.iter().for_each(|i| collect_text(i, key, out)),
        Value::String(s) if matches!(key, Some("description" | "title" | "enum" | "format" | "type" | "pattern" | "$ref")) => {
            let _ = writeln!(out, "{s}");
        }
        _ => {}
    }
}

pub fn schema_search_text(schema: &OpenApiSchema) -> (String, String) {
    let mut s = format!("Schema {}\n", schema.name);
    collect_text(&schema.schema, None, &mut s);
    (format!("Schema {}", schema.name), s)
}
