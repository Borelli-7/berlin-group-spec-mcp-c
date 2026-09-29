use super::escape_token;
use crate::domain::{ChangeType, SchemaChange, SchemaChangeKind, schema_ref_name};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, HashSet};

/// Named component schemas available for `$ref` resolution on one side of a comparison.
pub type SchemaSet = BTreeMap<String, Value>;

const MAX_REF_CHAIN: usize = 32;
const MAX_DEPTH: usize = 64;

const CONSTRAINT_KEYS: &[&str] = &[
    "minLength",
    "maxLength",
    "pattern",
    "minimum",
    "maximum",
    "exclusiveMinimum",
    "exclusiveMaximum",
    "multipleOf",
    "minItems",
    "maxItems",
    "uniqueItems",
    "minProperties",
    "maxProperties",
    "const",
    "default",
    "readOnly",
    "writeOnly",
];

const COMPOSITION_KEYS: &[&str] = &["allOf", "oneOf", "anyOf"];

/// Compares two schemas structurally, following `$ref`s into the provided schema sets.
pub fn diff_schemas(old: &Value, new: &Value, old_set: &SchemaSet, new_set: &SchemaSet) -> Vec<SchemaChange> {
    let mut d = Differ {
        old_set,
        new_set,
        visited: HashSet::new(),
        out: Vec::new(),
    };
    d.diff("", None, old, new, 0);
    d.out
}

struct Differ<'a> {
    old_set: &'a SchemaSet,
    new_set: &'a SchemaSet,
    visited: HashSet<(String, String)>,
    out: Vec<SchemaChange>,
}

/// Follows a `$ref` chain. Returns the last referenced name and the resolved schema.
/// Unresolvable references resolve to the reference object itself.
fn resolve<'v>(set: &'v SchemaSet, mut v: &'v Value) -> (Option<&'v str>, &'v Value) {
    let mut name = None;
    for _ in 0..MAX_REF_CHAIN {
        let Some(n) = v.get("$ref").and_then(Value::as_str).and_then(schema_ref_name) else {
            break;
        };
        match set.get_key_value(n) {
            Some((k, target)) => {
                name = Some(k.as_str());
                v = target;
            }
            None => {
                name = Some(n);
                break;
            }
        }
    }
    (name, v)
}

fn type_set(v: &Value) -> BTreeSet<String> {
    match v.get("type") {
        Some(Value::String(t)) if t != "null" => BTreeSet::from([t.clone()]),
        Some(Value::Array(ts)) => ts
            .iter()
            .filter_map(Value::as_str)
            .filter(|t| *t != "null")
            .map(str::to_owned)
            .collect(),
        _ => BTreeSet::new(),
    }
}

/// OpenAPI 3.0 `nullable: true` or 3.1 `type: [..., "null"]`.
fn is_nullable(v: &Value) -> bool {
    v.get("nullable").and_then(Value::as_bool).unwrap_or(false)
        || matches!(v.get("type"), Some(Value::Array(ts)) if ts.iter().any(|t| t == "null"))
}

fn required_set(v: &Value) -> BTreeSet<String> {
    v.get("required")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).map(str::to_owned).collect())
        .unwrap_or_default()
}

fn empty_map() -> &'static serde_json::Map<String, Value> {
    static EMPTY: std::sync::OnceLock<serde_json::Map<String, Value>> = std::sync::OnceLock::new();
    EMPTY.get_or_init(serde_json::Map::new)
}

fn properties(v: &Value) -> &serde_json::Map<String, Value> {
    v.get("properties").and_then(Value::as_object).unwrap_or_else(|| empty_map())
}

fn ref_label(name: Option<&str>, raw: &Value) -> Value {
    match name {
        Some(n) => json!(format!("#/components/schemas/{n}")),
        None if raw.get("$ref").is_some() => raw["$ref"].clone(),
        None => json!("inline"),
    }
}

impl Differ<'_> {
    #[allow(clippy::too_many_arguments)]
    fn push(
        &mut self,
        pointer: &str,
        schema_name: Option<&str>,
        kind: SchemaChangeKind,
        change: ChangeType,
        before: Option<Value>,
        after: Option<Value>,
    ) {
        self.out.push(SchemaChange {
            pointer: if pointer.is_empty() { "/".to_owned() } else { pointer.to_owned() },
            kind,
            change,
            before,
            after,
            schema_name: schema_name.map(str::to_owned),
        });
    }

    fn diff(&mut self, ptr: &str, ctx_name: Option<&str>, a_raw: &Value, b_raw: &Value, depth: usize) {
        if depth > MAX_DEPTH {
            return;
        }
        let (na, a) = resolve(self.old_set, a_raw);
        let (nb, b) = resolve(self.new_set, b_raw);

        if na != nb {
            self.push(
                ptr,
                ctx_name,
                SchemaChangeKind::RefChanged,
                ChangeType::Modified,
                Some(ref_label(na, a_raw)),
                Some(ref_label(nb, b_raw)),
            );
        }
        if let (Some(x), Some(y)) = (na, nb)
            && !self.visited.insert((x.to_owned(), y.to_owned()))
        {
            return;
        }
        let owned_name = nb.or(na).map(str::to_owned);
        let name = owned_name.as_deref().or(ctx_name);

        // type
        let (ta, tb) = (type_set(a), type_set(b));
        if ta != tb {
            self.push(ptr, name, SchemaChangeKind::TypeChanged, ChangeType::Modified,
                Some(json!(ta)), Some(json!(tb)));
        }
        // format
        let (fa, fb) = (a.get("format"), b.get("format"));
        if fa != fb {
            let change = match (fa, fb) {
                (None, Some(_)) => ChangeType::Added,
                (Some(_), None) => ChangeType::Removed,
                _ => ChangeType::Modified,
            };
            self.push(&format!("{ptr}/format"), name, SchemaChangeKind::FormatChanged, change,
                fa.cloned(), fb.cloned());
        }
        // nullable
        let (nla, nlb) = (is_nullable(a), is_nullable(b));
        if nla != nlb {
            self.push(ptr, name, SchemaChangeKind::NullableChanged, ChangeType::Modified,
                Some(json!(nla)), Some(json!(nlb)));
        }
        self.diff_enum(ptr, name, a, b);
        for key in CONSTRAINT_KEYS {
            let (x, y) = (a.get(*key), b.get(*key));
            if x != y {
                let change = match (x, y) {
                    (None, Some(_)) => ChangeType::Added,
                    (Some(_), None) => ChangeType::Removed,
                    _ => ChangeType::Modified,
                };
                self.push(&format!("{ptr}/{key}"), name, SchemaChangeKind::ConstraintChanged, change,
                    x.cloned(), y.cloned());
            }
        }
        self.diff_object(ptr, name, a, b, depth);
        // items
        match (a.get("items"), b.get("items")) {
            (Some(x), Some(y)) => self.diff(&format!("{ptr}/items"), name, x, y, depth + 1),
            (None, Some(y)) => self.push(&format!("{ptr}/items"), name, SchemaChangeKind::ConstraintChanged,
                ChangeType::Added, None, Some(y.clone())),
            (Some(x), None) => self.push(&format!("{ptr}/items"), name, SchemaChangeKind::ConstraintChanged,
                ChangeType::Removed, Some(x.clone()), None),
            (None, None) => {}
        }
        self.diff_composition(ptr, name, a, b, depth);
    }

    fn diff_enum(&mut self, ptr: &str, name: Option<&str>, a: &Value, b: &Value) {
        let p = format!("{ptr}/enum");
        match (a.get("enum").and_then(Value::as_array), b.get("enum").and_then(Value::as_array)) {
            (Some(x), Some(y)) => {
                let added: Vec<Value> = y.iter().filter(|v| !x.contains(v)).cloned().collect();
                let removed: Vec<Value> = x.iter().filter(|v| !y.contains(v)).cloned().collect();
                if !added.is_empty() {
                    self.push(&p, name, SchemaChangeKind::EnumValuesAdded, ChangeType::Added,
                        None, Some(Value::Array(added)));
                }
                if !removed.is_empty() {
                    self.push(&p, name, SchemaChangeKind::EnumValuesRemoved, ChangeType::Removed,
                        Some(Value::Array(removed)), None);
                }
            }
            (None, Some(y)) => self.push(&p, name, SchemaChangeKind::EnumConstraintChanged,
                ChangeType::Added, None, Some(Value::Array(y.clone()))),
            (Some(x), None) => self.push(&p, name, SchemaChangeKind::EnumConstraintChanged,
                ChangeType::Removed, Some(Value::Array(x.clone())), None),
            (None, None) => {}
        }
    }

    fn diff_object(&mut self, ptr: &str, name: Option<&str>, a: &Value, b: &Value, depth: usize) {
        let (ra, rb) = (required_set(a), required_set(b));
        let (pa, pb) = (properties(a), properties(b));
        let keys: BTreeSet<&String> = pa.keys().chain(pb.keys()).collect();
        for k in keys {
            let p = format!("{ptr}/properties/{}", escape_token(k));
            match (pa.get(k), pb.get(k)) {
                (None, Some(v)) => self.push(&p, name, SchemaChangeKind::PropertyAdded, ChangeType::Added,
                    None, Some(json!({"required": rb.contains(k), "schema": v}))),
                (Some(v), None) => self.push(&p, name, SchemaChangeKind::PropertyRemoved, ChangeType::Removed,
                    Some(json!({"required": ra.contains(k), "schema": v})), None),
                (Some(x), Some(y)) => self.diff(&p, name, x, y, depth + 1),
                (None, None) => {}
            }
        }
        // Required-ness changes (including for properties declared only via allOf siblings).
        for k in rb.difference(&ra) {
            if pa.contains_key(k) || !pb.contains_key(k) {
                self.push(&format!("{ptr}/properties/{}", escape_token(k)), name,
                    SchemaChangeKind::RequiredAdded, ChangeType::Modified, Some(json!(false)), Some(json!(true)));
            }
        }
        for k in ra.difference(&rb) {
            if pb.contains_key(k) || !pa.contains_key(k) {
                self.push(&format!("{ptr}/properties/{}", escape_token(k)), name,
                    SchemaChangeKind::RequiredRemoved, ChangeType::Modified, Some(json!(true)), Some(json!(false)));
            }
        }
        let (xa, xb) = (a.get("additionalProperties"), b.get("additionalProperties"));
        match (xa, xb) {
            (Some(x @ Value::Object(_)), Some(y @ Value::Object(_))) => {
                self.diff(&format!("{ptr}/additionalProperties"), name, x, y, depth + 1)
            }
            _ if xa != xb => self.push(&format!("{ptr}/additionalProperties"), name,
                SchemaChangeKind::AdditionalPropertiesChanged, ChangeType::Modified, xa.cloned(), xb.cloned()),
            _ => {}
        }
    }

    fn diff_composition(&mut self, ptr: &str, name: Option<&str>, a: &Value, b: &Value, depth: usize) {
        for key in COMPOSITION_KEYS {
            let (xa, xb) = (
                a.get(*key).and_then(Value::as_array),
                b.get(*key).and_then(Value::as_array),
            );
            let p = format!("{ptr}/{key}");
            match (xa, xb) {
                (Some(x), Some(y)) => {
                    if x.len() != y.len() {
                        self.push(&p, name, SchemaChangeKind::CompositionChanged, ChangeType::Modified,
                            Some(json!(x.len())), Some(json!(y.len())));
                    }
                    for (i, (sx, sy)) in x.iter().zip(y).enumerate() {
                        self.diff(&format!("{p}/{i}"), name, sx, sy, depth + 1);
                    }
                }
                (None, Some(y)) => self.push(&p, name, SchemaChangeKind::CompositionChanged, ChangeType::Added,
                    None, Some(Value::Array(y.clone()))),
                (Some(x), None) => self.push(&p, name, SchemaChangeKind::CompositionChanged, ChangeType::Removed,
                    Some(Value::Array(x.clone())), None),
                (None, None) => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use SchemaChangeKind as K;

    fn kinds(changes: &[SchemaChange]) -> Vec<(K, String)> {
        changes.iter().map(|c| (c.kind, c.pointer.clone())).collect()
    }

    #[test]
    fn identical_schemas_have_no_changes() {
        let s = json!({"type":"object","properties":{"a":{"type":"string"}},"required":["a"]});
        assert!(diff_schemas(&s, &s, &SchemaSet::new(), &SchemaSet::new()).is_empty());
    }

    #[test]
    fn detects_property_type_format_required_enum_nullable() {
        let a = json!({"type":"object","required":["id"],"properties":{
            "id":{"type":"string"},
            "amount":{"type":"string","format":"decimal"},
            "status":{"type":"string","enum":["booked","pending"]},
            "old":{"type":"integer"},
            "note":{"type":"string"}
        }});
        let b = json!({"type":"object","required":["id","status"],"properties":{
            "id":{"type":"integer"},
            "amount":{"type":"string","format":"amount"},
            "status":{"type":"string","enum":["booked","information"]},
            "note":{"type":"string","nullable":true},
            "added":{"type":"boolean"}
        }});
        let c = diff_schemas(&a, &b, &SchemaSet::new(), &SchemaSet::new());
        let k = kinds(&c);
        assert!(k.contains(&(K::TypeChanged, "/properties/id".into())));
        assert!(k.contains(&(K::FormatChanged, "/properties/amount/format".into())));
        assert!(k.contains(&(K::EnumValuesAdded, "/properties/status/enum".into())));
        assert!(k.contains(&(K::EnumValuesRemoved, "/properties/status/enum".into())));
        assert!(k.contains(&(K::RequiredAdded, "/properties/status".into())));
        assert!(k.contains(&(K::NullableChanged, "/properties/note".into())));
        assert!(k.contains(&(K::PropertyRemoved, "/properties/old".into())));
        assert!(k.contains(&(K::PropertyAdded, "/properties/added".into())));
        assert_eq!(c.len(), 8, "{c:#?}");
    }

    #[test]
    fn follows_refs_and_survives_cycles() {
        let old: SchemaSet = [
            ("Node".to_owned(), json!({"type":"object","properties":{"next":{"$ref":"#/components/schemas/Node"},"v":{"type":"string"}}})),
        ].into();
        let new: SchemaSet = [
            ("Node".to_owned(), json!({"type":"object","properties":{"next":{"$ref":"#/components/schemas/Node"},"v":{"type":"integer"}}})),
        ].into();
        let r = json!({"$ref":"#/components/schemas/Node"});
        let c = diff_schemas(&r, &r, &old, &new);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].kind, K::TypeChanged);
        assert_eq!(c[0].schema_name.as_deref(), Some("Node"));
    }

    #[test]
    fn detects_ref_change_and_openapi31_nullable() {
        let old: SchemaSet = [("A".to_owned(), json!({"type":"string"}))].into();
        let new: SchemaSet = [("B".to_owned(), json!({"type":["string","null"]}))].into();
        let c = diff_schemas(&json!({"$ref":"#/components/schemas/A"}), &json!({"$ref":"#/components/schemas/B"}), &old, &new);
        let k: Vec<K> = c.iter().map(|c| c.kind).collect();
        assert_eq!(k, vec![K::RefChanged, K::NullableChanged]);
    }

    #[test]
    fn detects_enum_constraint_and_composition() {
        let a = json!({"oneOf":[{"type":"string"}]});
        let b = json!({"oneOf":[{"type":"string"},{"type":"integer"}], "enum":["x"]});
        let k: Vec<K> = diff_schemas(&a, &b, &SchemaSet::new(), &SchemaSet::new()).iter().map(|c| c.kind).collect();
        assert_eq!(k, vec![K::EnumConstraintChanged, K::CompositionChanged]);
    }
}
