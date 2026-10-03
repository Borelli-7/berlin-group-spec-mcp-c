# MCP tools

Server name: `berlin-group-spec` (stdio). Every tool returns **structured JSON** (`structuredContent`,
mirrored as text) and has these annotations: `readOnlyHint: true`, `destructiveHint: false`,
`idempotentHint: true`, `openWorldHint: false`. Inputs reject unknown fields.

Complete requests are in [`examples/calls/`](examples/calls) and the responses captured from the
official corpus (`corpus/`) are in [`examples/responses/`](examples/responses). To regenerate them, run `python3
examples/capture.py target/release/bg-spec-mcp config/config.toml` after `bg-spec index`. Arrays longer than
10 items are cut in the captures and end with `{"_truncated_items": N}`; the server itself never truncates.

## Evidence classification

| `classification` | Meaning |
|---|---|
| `authoritative_requirement` | A curated entry from `requirements.yaml`, with its source mapping. |
| `discovered_evidence` | A full-text hit. It is **not** a requirement. Verify it with `read_source`. |
| `source_content` | Exact indexed content returned by `read_source`. |

Every evidence item carries **provenance**: `source_id`, `document_id`, `version`, `kind`,
`authority`, `precedence`, `locator`, `sha256` (of the record), and `document_sha256` (of the file).

## Locator grammar

| Locator | Meaning |
|---|---|
| `document` | The whole document (all pages, or the operation and schema list). |
| `page:N` / `page:N-M` | PDF or text pages (a range covers at most 50 pages). |
| `section:4.4.4` | Chunks whose section equals the value or is a subsection of it (`4.4`, `4.4.4`, `4.4.4 Title`; also `section:E-01`). |
| `chunk:<chunk_id>` | One chunk, e.g. `chunk:bg-openfinance-v2-xs2a-implementation-guidelines:p95:c0`. |
| `op:GET /path` | An OpenAPI operation. |
| `path:/path` | All operations on a path. |
| `schema:Name` | A component schema. |

## Errors

Client errors return a tool result with `isError: true` and this text body:

```json
{"error": {"code": "not_found", "message": "not found: unknown source_id '../../etc/passwd'"}}
```

| code | cause |
|---|---|
| `invalid_input` | Empty query, unknown version, bad method or path, or an unsupported kind. |
| `invalid_locator` | The locator does not match the grammar above, or does not fit the document kind. |
| `not_found` | Unknown source, requirement, schema, page, section, or chunk. |

Unknown or missing fields are rejected by rmcp with `failed to deserialize parameters: …`
(`isError: true`). Storage and search failures are returned as JSON-RPC internal errors.

Endpoint lookups first try an **exact** match on the path template, then a **canonical** match:
the configured version prefixes (`/v1`, `/v2`) are stripped and parameter names are ignored, so
`/v1/accounts/{account-id}` matches `/v2/accounts/{account-id}` and `/accounts/{accountId}`. The response
reports which one was used in `matched_by`. When distinct templates share one canonical key, the best
fit wins: identical path, then identical parameter names, then names equal ignoring case, `-` and `_`,
then source precedence. The other templates are listed in `other_templates` (omitted when empty), and
`ambiguous: true` is added when precedence alone decided between equally fitting templates.

---

## `search_specification`

BM25 full-text search over PDF and text chunks, OpenAPI operations, and schemas. Results are
**discovered evidence** and are never authoritative requirements.
Code-like identifiers (`operationId`, `METHOD /path`, header and schema names, codes such as `E-07`)
also match exactly and rank the defining record first.

Input: `query` (required, ≤1000 chars, phrases in quotes), `version?`, `kind?` (`pdf|openapi|text`),
`source_id?`, `limit?` (clamped to `search.max_limit`).

Output: `{query, filters, notice, total, results:[{record_id, record_type(chunk|operation|schema),
source_id, document_id, version, kind, authority, title, locator, page, evidence, relevance, sha256,
classification}]}`. See [06](examples/responses/06-search_specification.json).

## `find_requirement`

Searches curated requirements (`match_reason`: `id_exact`, `id_partial`, or `text`, where every query token
prefixes a token of the id, title, description, criteria, tags, or endpoints) **and**
the full-text index. It never invents requirement ids.

Input: `query`, `version?`, `limit?`.
Output: `{query, notice, requirements:[{requirement_id, classification:"authoritative_requirement",
version, title, match_reason, sources, related_endpoints, acceptance_criteria, mapping}],
discovered_evidence:[…search results…]}`. See [07](examples/responses/07-find_requirement.json).

## `read_source`

Returns exact indexed content by `source_id` and `locator`. Filesystem paths are not accepted.

Input: `source_id`, `locator`.
Output: `{source:{…catalog metadata, status, ocr_required_pages, diagnostics…}, locator,
classification:"source_content", content:{type:"pages"|"chunks"|"operations"|"schemas", …},
provenance:[…]}`. A page whose status is `ocr_required` or `blank` is returned with empty text and that
status. The server never substitutes invented text. See [04](examples/responses/04-read_source.json),
[13](examples/responses/13-read_source-section.json), and the error example [12](examples/responses/12-error-unknown-source.json).

## `read_openapi_endpoint`

Input: `version?` (defaults to the target), `path`, `method`.
Output: `{version, requested_method, requested_path, matched_by, operation:{path, method, operation_id,
summary, description, tags, deprecated, parameters[{name, location, required, schema, component}],
request_body (only when declared), responses[{status, description, headers, header_definitions?[{name, required,
deprecated, description, schema, component}], content}], security, security_origin (operation|global),
referenced_schemas, unresolved_references?[{location, reason}], json_pointer, provenance}, alternatives, ambiguous?,
other_templates?}`.
`alternatives` lists other sources of the same template; `other_templates` lists different templates with the same canonical key.
`$ref` values are resolved for parameters, request bodies, responses, and response headers. Schemas remain
`$ref` values, and their names (including header schemas) are listed in `referenced_schemas`. A reference that
cannot be resolved omits the affected element and is listed in `unresolved_references`. See [02](examples/responses/02-read_openapi_endpoint.json).

## `read_openapi_schema`

Input: `version?`, `name`.
Output: `{version, name, schema:{schema (verbatim JSON), referenced_schemas, json_pointer, provenance},
referenced_schemas, transitive_referenced_schemas, unresolved_references, alternatives}`.
See [08](examples/responses/08-read_openapi_schema.json).

## `get_endpoint_requirements` (primary Requirements Agent tool)

Deterministic 7-step bundle: operation → transitive schemas → curated requirements (by declared
endpoint **and** by `path:`/`op:` source locators) → related evidence (search built from the
summary, split operationId, literal path segments, and tags) → citations → conflict detection → bundle.

Input: `version?`, `path`, `method`, `evidence_limit?`.
Output: `{endpoint:{version, method, requested_path, found, matched_path, matched_by, ambiguous?, other_templates?}, notice, openapi,
requirements, schemas, unresolved_schemas, related_evidence, conflicts, sources}`. `sources` is the
deduplicated list of citations. If the endpoint does not exist, `found` is `false`. That is not an error.
See [01](examples/responses/01-get_endpoint_requirements.json).

## `trace_requirement`

Input: `requirement_id`.
Output: `{requirement_id, classification, requirement, mapping:{file, file_sha256, locator, sha256},
sources:[{source_id, locator, status, document, excerpt, provenance}], endpoints:[{endpoint, via,
found, operation, provenance}], schemas:[{name, via, found, referenced_schemas, provenance}],
acceptance_criteria:[{id, requirement_id, text, mapping}], related_evidence, conflicts, notice}`.
See [03](examples/responses/03-trace_requirement.json). The conflicts example is
[11](examples/responses/11-trace_requirement-conflicts.json).

## `compare_v1_v2`

Structural, deterministic diff. It **never returns a compatibility verdict**.

Input: `path`, `method`, `from_version?` (defaults to the baseline), `to_version?` (defaults to the target).
Output: `{method, path, from:{version, found, matched_by, unresolved_schemas, ambiguous?}, to:{…}, notice,
v1_operation, v2_operation, changes:[{area, side, change(added|removed|modified|unchanged), subject,
before?, after?}], schema_changes:[{schema_name, pointer, kind(schema_added|schema_removed|
property_added|property_removed|type_changed|format_changed|nullable_changed|required_added|
required_removed|enum_values_added|enum_values_removed|…), change, before?, after?}], counts, compatibility_relevant_facts:[{side, subject, statement}], sources}`.
See [05](examples/responses/05-compare_v1_v2.json).

## `list_sources`

Input: `version?`, `kind?`.
Output: `{total, sources:[{source_id, document_id, version, kind, title, authority, precedence, path
(relative to the corpus), sha256, status(indexed|partial|failed|missing), page_count,
ocr_required_pages, diagnostics, indexed_at_unix}]}`.

## `health`

Input: `{}`.
Output: `{status(ok|degraded), read_only:true, server_version, baseline_version, target_version,
catalog:{documents, pages, ocr_required_pages, blank_pages, chunks, operations, schemas, requirements},
index:{schema_version, last_indexed_at_unix, index_generation, requirements_file_sha256},
search_documents, versions, problems}`. See [10](examples/responses/10-health.json).

## Acceptance workflow (Requirements Agent)

```text
get_endpoint_requirements {path:"/v2/accounts/{account-id}/transactions", method:"GET"}
  → read_openapi_endpoint   (contract)
  → trace_requirement       {requirement_id:"OFV2-AIS-TRANSACTIONS-001"}
  → read_source             {source_id:"bg-openfinance-v2-xs2a-implementation-guidelines", locator:"section:4.4.4"}
  → compare_v1_v2           {path:"/v1/accounts/{account-id}/transactions", method:"GET"}
```
