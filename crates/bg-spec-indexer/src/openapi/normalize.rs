use super::{Dialect, detect_dialect};
use bg_spec_core::{
    Result,
    domain::{
        Diagnostic, Document, MediaTypeSchema, NormalizedParameter, NormalizedRequestBody, NormalizedResponse,
        OpenApiOperation, OpenApiSchema, Provenance, SchemaSlot, SecurityOrigin, SecurityRequirement, Severity,
        collect_schema_refs, schema_ref_name,
    },
    hash::sha256_hex,
    openapi_path,
};
use serde_json::{Map, Value, json};
use std::collections::BTreeSet;

const METHODS: [&str; 8] = ["get", "put", "post", "delete", "options", "head", "patch", "trace"];
const MAX_REF_DEPTH: usize = 16;

/// Normalized content of one OpenAPI document.
#[derive(Debug, Clone, Default)]
pub struct NormalizedApi {
    pub operations: Vec<OpenApiOperation>,
    pub schemas: Vec<OpenApiSchema>,
    pub metadata: Value,
    pub diagnostics: Vec<Diagnostic>,
}

fn pointer_escape(s: &str) -> String {
    s.replace('~', "~0").replace('/', "~1")
}

fn canonical_sha(value: &Value) -> String {
    sha256_hex(serde_json::to_vec(value).unwrap_or_default())
}

fn str_field(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::to_owned)
}

struct Resolver<'a> {
    root: &'a Value,
}

impl<'a> Resolver<'a> {
    /// Follows local `$ref`s. Returns the target and the first reference seen.
    fn resolve(&self, value: &'a Value) -> std::result::Result<(&'a Value, Option<String>), String> {
        let mut current = value;
        let mut first_ref = None;
        for _ in 0..MAX_REF_DEPTH {
            let Some(r) = current.get("$ref").and_then(Value::as_str) else {
                return Ok((current, first_ref));
            };
            let pointer = r
                .strip_prefix('#')
                .ok_or_else(|| format!("external reference '{r}' is not supported"))?;
            first_ref.get_or_insert_with(|| r.to_owned());
            current = self.root.pointer(pointer).ok_or_else(|| format!("unresolved reference '{r}'"))?;
        }
        Err("reference chain too deep".into())
    }
}

fn schema_slot(schema: &Value) -> SchemaSlot {
    SchemaSlot {
        schema_ref: schema.get("$ref").and_then(Value::as_str).and_then(schema_ref_name).map(str::to_owned),
        schema: schema.clone(),
    }
}

fn media_types(content: Option<&Value>) -> Vec<MediaTypeSchema> {
    content
        .and_then(Value::as_object)
        .map(|m| {
            m.iter()
                .map(|(media_type, mt)| MediaTypeSchema {
                    media_type: media_type.clone(),
                    schema: mt.get("schema").map(schema_slot),
                })
                .collect()
        })
        .unwrap_or_default()
}

fn security_list(v: &Value) -> Vec<SecurityRequirement> {
    v.as_array()
        .map(|alts| {
            alts.iter()
                .filter_map(Value::as_object)
                .map(|alt| {
                    alt.iter()
                        .map(|(scheme, scopes)| {
                            let scopes = scopes
                                .as_array()
                                .map(|s| s.iter().filter_map(Value::as_str).map(str::to_owned).collect())
                                .unwrap_or_default();
                            (scheme.clone(), scopes)
                        })
                        .collect()
                })
                .collect()
        })
        .unwrap_or_default()
}

struct Projector<'a> {
    doc: &'a Document,
    resolver: Resolver<'a>,
    diagnostics: Vec<Diagnostic>,
}

impl Projector<'_> {
    fn warn(&mut self, code: &str, msg: String, locator: String) {
        self.diagnostics.push(Diagnostic::new(Severity::Warning, code, msg).at(locator));
    }

    fn parameter(&mut self, raw: &Value, locator: &str) -> Option<NormalizedParameter> {
        let (p, component) = match self.resolver.resolve(raw) {
            Ok(r) => r,
            Err(e) => {
                self.warn("unresolved_ref", e, locator.to_owned());
                return None;
            }
        };
        let name = str_field(p, "name")?;
        let location = str_field(p, "in")?;
        let schema = p.get("schema").map(schema_slot).or_else(|| {
            media_types(p.get("content")).into_iter().next().and_then(|m| m.schema)
        });
        Some(NormalizedParameter {
            required: p.get("required").and_then(Value::as_bool).unwrap_or(location == "path"),
            deprecated: p.get("deprecated").and_then(Value::as_bool).unwrap_or(false),
            description: str_field(p, "description"),
            name,
            location,
            schema,
            component,
        })
    }

    fn parameters(&mut self, path_level: Option<&Value>, op_level: Option<&Value>, locator: &str) -> Vec<NormalizedParameter> {
        let mut merged: Vec<NormalizedParameter> = Vec::new();
        for list in [path_level, op_level].into_iter().flatten() {
            for raw in list.as_array().into_iter().flatten() {
                if let Some(p) = self.parameter(raw, locator) {
                    match merged.iter_mut().find(|q| q.name == p.name && q.location == p.location) {
                        Some(existing) => *existing = p,
                        None => merged.push(p),
                    }
                }
            }
        }
        merged
    }

    fn request_body(&mut self, raw: Option<&Value>, locator: &str) -> Option<NormalizedRequestBody> {
        let (body, component) = match self.resolver.resolve(raw?) {
            Ok(r) => r,
            Err(e) => {
                self.warn("unresolved_ref", e, locator.to_owned());
                return None;
            }
        };
        Some(NormalizedRequestBody {
            required: body.get("required").and_then(Value::as_bool).unwrap_or(false),
            description: str_field(body, "description"),
            content: media_types(body.get("content")),
            component,
        })
    }

    fn responses(&mut self, raw: Option<&Value>, locator: &str) -> Vec<NormalizedResponse> {
        let mut out = Vec::new();
        for (status, raw) in raw.and_then(Value::as_object).into_iter().flatten() {
            if status.starts_with("x-") {
                continue;
            }
            let (resp, component) = match self.resolver.resolve(raw) {
                Ok(r) => r,
                Err(e) => {
                    self.warn("unresolved_ref", e, locator.to_owned());
                    continue;
                }
            };
            let mut headers: Vec<String> = resp
                .get("headers")
                .and_then(Value::as_object)
                .map(|h| h.keys().cloned().collect())
                .unwrap_or_default();
            headers.sort();
            out.push(NormalizedResponse {
                status: status.clone(),
                description: str_field(resp, "description"),
                headers,
                content: media_types(resp.get("content")),
                component,
            });
        }
        out
    }

    fn operation(&mut self, path: &str, method: &str, path_item: &Value, raw: &Value, global_security: Option<&Value>) -> OpenApiOperation {
        let method_upper = method.to_ascii_uppercase();
        let locator = format!("op:{method_upper} {path}");
        let parameters = self.parameters(path_item.get("parameters"), raw.get("parameters"), &locator);
        let request_body = self.request_body(raw.get("requestBody"), &locator);
        let responses = self.responses(raw.get("responses"), &locator);
        let (security, security_origin) = match (raw.get("security"), global_security) {
            (Some(s), _) => (security_list(s), SecurityOrigin::Operation),
            (None, Some(g)) => (security_list(g), SecurityOrigin::Global),
            (None, None) => (Vec::new(), SecurityOrigin::None),
        };

        let mut refs = BTreeSet::new();
        for v in [
            serde_json::to_value(&parameters).unwrap_or_default(),
            serde_json::to_value(&request_body).unwrap_or_default(),
            serde_json::to_value(&responses).unwrap_or_default(),
        ] {
            collect_schema_refs(&v, &mut refs);
        }
        OpenApiOperation {
            version: self.doc.version.clone(),
            path: path.to_owned(),
            method: method_upper,
            operation_id: str_field(raw, "operationId"),
            summary: str_field(raw, "summary"),
            description: str_field(raw, "description"),
            tags: raw
                .get("tags")
                .and_then(Value::as_array)
                .map(|t| t.iter().filter_map(Value::as_str).map(str::to_owned).collect())
                .unwrap_or_default(),
            deprecated: raw.get("deprecated").and_then(Value::as_bool).unwrap_or(false),
            parameters,
            request_body,
            responses,
            security,
            security_origin,
            referenced_schemas: refs.into_iter().collect(),
            json_pointer: format!("/paths/{}/{method}", pointer_escape(path)),
            provenance: Provenance::for_document(self.doc, locator, canonical_sha(&json!({"path_item_parameters": path_item.get("parameters"), "operation": raw}))),
        }
    }
}

fn metadata(root: &Value, dialect: &Dialect, parser: &str, operations: usize, schemas: usize) -> Value {
    let info = root.get("info").cloned().unwrap_or(Value::Null);
    let servers: Vec<Value> = root
        .get("servers")
        .and_then(Value::as_array)
        .map(|s| s.iter().filter_map(|x| x.get("url").cloned()).collect())
        .unwrap_or_default();
    let tags: Vec<Value> = root
        .get("tags")
        .and_then(Value::as_array)
        .map(|t| t.iter().filter_map(|x| x.get("name").cloned()).collect())
        .unwrap_or_default();
    let components = root.get("components").and_then(Value::as_object);
    let mut component_counts = Map::new();
    let mut security_schemes = Map::new();
    if let Some(c) = components {
        for (k, v) in c {
            if let Some(o) = v.as_object() {
                component_counts.insert(k.clone(), json!(o.len()));
            }
        }
        if let Some(schemes) = c.get("securitySchemes").and_then(Value::as_object) {
            for (name, s) in schemes {
                security_schemes.insert(name.clone(), s.get("type").cloned().unwrap_or(Value::Null));
            }
        }
    }
    let openapi = match dialect {
        Dialect::V30(v) | Dialect::V31(v) => v.clone(),
    };
    json!({
        "openapi": openapi,
        "parser": parser,
        "info": {
            "title": info.get("title"),
            "version": info.get("version"),
            "description": info.get("description"),
        },
        "servers": servers,
        "tags": tags,
        "security_schemes": security_schemes,
        "global_security": root.get("security"),
        "components": component_counts,
        "operation_count": operations,
        "schema_count": schemas,
    })
}

/// Normalizes an OpenAPI document tree into semantic operation and schema records.
pub fn normalize(root: &Value, doc: &Document) -> Result<NormalizedApi> {
    let dialect = detect_dialect(root)?;
    let mut diagnostics = Vec::new();
    let mut typed: Option<openapiv3::OpenAPI> = None;
    let parser = match &dialect {
        Dialect::V30(_) => match serde_json::from_value::<openapiv3::OpenAPI>(root.clone()) {
            Ok(api) => {
                typed = Some(api);
                "openapiv3"
            }
            Err(e) => {
                diagnostics.push(Diagnostic::new(
                    Severity::Warning,
                    "openapi_typed_parse_failed",
                    format!("strict OpenAPI 3.0 parse failed ({e}); using generic JSON normalization"),
                ));
                "generic-json"
            }
        },
        Dialect::V31(v) => {
            diagnostics.push(Diagnostic::new(
                Severity::Info,
                "openapi_31_fallback",
                format!("OpenAPI {v} is not supported by the typed parser; using generic JSON normalization"),
            ));
            "generic-json"
        }
    };

    let mut projector = Projector { doc, resolver: Resolver { root }, diagnostics };
    let global_security = root.get("security");
    let mut operations = Vec::new();
    for (path, item) in root.get("paths").and_then(Value::as_object).into_iter().flatten() {
        if path.starts_with("x-") {
            continue;
        }
        if let Err(e) = openapi_path::validate_path(path) {
            projector.warn("invalid_path", e.to_string(), format!("path:{path}"));
            continue;
        }
        let item = match projector.resolver.resolve(item) {
            Ok((item, _)) => item,
            Err(e) => {
                projector.warn("unresolved_ref", e, format!("path:{path}"));
                continue;
            }
        };
        for method in METHODS {
            if let Some(raw) = item.get(method) {
                operations.push(projector.operation(path, method, item, raw, global_security));
            }
        }
    }

    let mut schemas = Vec::new();
    let components = root.pointer("/components/schemas").and_then(Value::as_object);
    for (name, schema) in components.into_iter().flatten() {
        let mut refs = BTreeSet::new();
        collect_schema_refs(schema, &mut refs);
        schemas.push(OpenApiSchema {
            version: doc.version.clone(),
            name: name.clone(),
            schema: schema.clone(),
            referenced_schemas: refs.into_iter().collect(),
            json_pointer: format!("/components/schemas/{}", pointer_escape(name)),
            provenance: Provenance::for_document(doc, format!("schema:{name}"), canonical_sha(schema)),
        });
    }

    if let Some(api) = &typed {
        let typed_ops: usize = api.paths.iter().filter_map(|(_, p)| p.as_item()).map(|p| p.iter().count()).sum();
        if typed_ops != operations.len() {
            projector.diagnostics.push(Diagnostic::new(
                Severity::Warning,
                "openapi_operation_count_mismatch",
                format!("typed parser found {typed_ops} operations, projector {}", operations.len()),
            ));
        }
    }

    let metadata = metadata(root, &dialect, parser, operations.len(), schemas.len());
    Ok(NormalizedApi { operations, schemas, metadata, diagnostics: projector.diagnostics })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openapi::parse_document;
    use bg_spec_core::domain::{DocumentAuthority, DocumentKind, DocumentStatus, SpecificationVersion};

    pub(crate) fn test_doc() -> Document {
        Document {
            source_id: "api".into(),
            document_id: "api@000000000000".into(),
            kind: DocumentKind::Openapi,
            version: SpecificationVersion::new("openfinance-v2").unwrap(),
            authority: DocumentAuthority::Technical,
            precedence: 90,
            title: "API".into(),
            path: "openapi/api.yaml".into(),
            sha256: Some("0".repeat(64)),
            size_bytes: None,
            status: DocumentStatus::Indexed,
            fingerprint: String::new(),
            extractor: None,
            page_count: None,
            metadata: Value::Null,
            diagnostics: vec![],
            indexed_at_unix: 0,
        }
    }

    const YAML: &str = r##"
openapi: 3.0.3
info: {title: Test, version: "2.0"}
security:
  - bearer: []
paths:
  /accounts/{accountId}/transactions:
    parameters:
      - $ref: '#/components/parameters/AccountId'
    get:
      operationId: getTransactionList
      summary: Read transaction list
      tags: [Accounts]
      parameters:
        - name: bookingStatus
          in: query
          required: true
          schema: {$ref: '#/components/schemas/BookingStatus'}
      responses:
        200:
          description: OK
          headers:
            X-Request-ID: {schema: {type: string}}
          content:
            application/json:
              schema: {$ref: '#/components/schemas/TransactionsResponse'}
        '400':
          $ref: '#/components/responses/Error400'
    post:
      security: []
      requestBody:
        required: true
        content:
          application/json:
            schema: {type: object}
      responses:
        '201': {description: Created}
components:
  parameters:
    AccountId: {name: accountId, in: path, required: true, schema: {type: string}}
  responses:
    Error400:
      description: Bad request
      content:
        application/json:
          schema: {$ref: '#/components/schemas/Error'}
  securitySchemes:
    bearer: {type: http, scheme: bearer}
  schemas:
    BookingStatus: {type: string, enum: [booked, pending]}
    TransactionsResponse:
      type: object
      properties:
        items: {type: array, items: {$ref: '#/components/schemas/Transaction'}}
    Transaction: {type: object, required: [amount], properties: {amount: {type: string}}}
    Error: {type: object}
"##;

    #[test]
    fn normalizes_typed_30() {
        let root = parse_document(YAML, "api.yaml").unwrap();
        let api = normalize(&root, &test_doc()).unwrap();
        assert!(api.diagnostics.is_empty(), "{:?}", api.diagnostics);
        assert_eq!(api.metadata["parser"], "openapiv3");
        assert_eq!(api.operations.len(), 2);
        let get = &api.operations[0];
        assert_eq!(get.method, "GET");
        assert_eq!(get.operation_id.as_deref(), Some("getTransactionList"));
        assert_eq!(get.provenance.locator, "op:GET /accounts/{accountId}/transactions");
        assert_eq!(get.json_pointer, "/paths/~1accounts~1{accountId}~1transactions/get");
        let names: Vec<_> = get.parameters.iter().map(|p| (p.name.as_str(), p.location.as_str(), p.required)).collect();
        assert_eq!(names, vec![("accountId", "path", true), ("bookingStatus", "query", true)]);
        assert_eq!(get.parameters[0].component.as_deref(), Some("#/components/parameters/AccountId"));
        let statuses: Vec<_> = get.responses.iter().map(|r| r.status.as_str()).collect();
        assert_eq!(statuses, vec!["200", "400"]);
        assert_eq!(get.responses[0].headers, vec!["X-Request-ID"]);
        assert_eq!(get.responses[1].description.as_deref(), Some("Bad request"));
        assert_eq!(get.referenced_schemas, vec!["BookingStatus", "Error", "TransactionsResponse"]);
        assert_eq!(get.security_origin, SecurityOrigin::Global);
        assert_eq!(get.security.len(), 1);
        let post = &api.operations[1];
        assert_eq!(post.security_origin, SecurityOrigin::Operation);
        assert!(post.security.is_empty());
        assert!(post.request_body.as_ref().unwrap().required);
        assert_eq!(api.schemas.len(), 4);
        let resp = api.schemas.iter().find(|s| s.name == "TransactionsResponse").unwrap();
        assert_eq!(resp.referenced_schemas, vec!["Transaction"]);
        assert_eq!(resp.provenance.locator, "schema:TransactionsResponse");
    }

    #[test]
    fn falls_back_for_31_with_diagnostic() {
        let yaml = YAML.replace("openapi: 3.0.3", "openapi: 3.1.0");
        let root = parse_document(&yaml, "api.yaml").unwrap();
        let api = normalize(&root, &test_doc()).unwrap();
        assert_eq!(api.metadata["parser"], "generic-json");
        assert!(api.diagnostics.iter().any(|d| d.code == "openapi_31_fallback"));
        assert_eq!(api.operations.len(), 2);
    }

    #[test]
    fn reports_unresolved_refs() {
        let yaml = YAML.replace("$ref: '#/components/responses/Error400'", "$ref: '#/components/responses/Missing'");
        let root = parse_document(&yaml, "api.yaml").unwrap();
        let api = normalize(&root, &test_doc()).unwrap();
        assert!(api.diagnostics.iter().any(|d| d.code == "unresolved_ref"));
    }

    #[test]
    fn rejects_swagger_and_non_mapping() {
        assert!(normalize(&json!({"swagger": "2.0"}), &test_doc()).is_err());
        assert!(parse_document("- a", "x.yaml").is_err());
    }
}
