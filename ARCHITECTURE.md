# Architecture

## 1. Principle

The server is a **retrieval and evidence system**, not a reasoning system.

```text
Specification MCP  = source retrieval + parsing + indexing + provenance + deterministic comparison
Requirements Agent = interpretation + engineering analysis + acceptance criteria
```

It never:
* makes business decisions
* invents requirement IDs
* resolves conflicts
* modifies sources or code
* calls an LLM

## 2. Layers and crates

```text
                ┌───────────────┐        ┌───────────────┐
                │  bg-spec-mcp  │        │  bg-spec-cli  │   composition roots (binaries)
                │ rmcp, stdio   │        │ index/doctor… │
                └──────┬────────┘        └──────┬────────┘
                       │ Services (DI: Arc<dyn Port>)│
                ┌──────▼──────────────────────────▼──────┐
                │              bg-spec-core               │  domain, services, diff, ports
                │  SpecificationService RequirementService│  (no I/O dependencies)
                │  CompatibilityService  diff::*          │
                │  ports: CatalogRepository SearchRepository
                └──────▲──────────────────────────▲──────┘
                       │ implements               │ uses
                ┌──────┴────────┐          ┌──────┴──────────┐
                │ bg-spec-store │◄─────────│ bg-spec-indexer │  write side (CLI only)
                │ SQLite+Tantivy│  writers │ pdf / openapi / │
                └───────────────┘          │ chunk / pipeline│
                                           └─────────────────┘
```

| Crate | Responsibility |
|---|---|
| `bg-spec-core` | Domain types, `SourceLocator` grammar, manifest and requirements parsing and validation, config, SHA-256 helpers, the structural diff engine, application services, and the `CatalogRepository` / `SearchRepository` ports. |
| `bg-spec-store` | `SqliteCatalog`, which opens read-write for the indexer and `mode=ro` for runtime. `TantivyWriter` / `TantivySearch`. Embedded migrations. |
| `bg-spec-indexer` | Corpus discovery with path confinement, hashing, the `PdfExtractor` trait with `PdfOxideExtractor`, OpenAPI parsing and normalization, page-aware chunking, and the incremental pipeline. |
| `bg-spec-mcp` | `BgSpecServer`: 10 `#[tool]` adapters over `Services`, error mapping, and a stdio `main`. Its only dependencies are core and store. |
| `bg-spec-cli` | `bg-spec index / doctor / stats / sources`. |

The MCP binary **does not link the indexer**. The runtime cannot parse documents or build indexes.

## 3. Domain model (bg-spec-core::domain)

| Type | Notes |
|---|---|
| `Document` | A catalogued source. Fields: `source_id`, `document_id` (`{source_id}@{sha12}`), version, kind, authority, precedence, relative path, sha256, status, pages, diagnostics, and metadata. |
| `DocumentKind` | `pdf`, `openapi`, `text` |
| `DocumentAuthority` | `normative`, `technical`, `informative`, `project`, `test`, `unknown`. Always declared in the manifest. |
| `SpecificationVersion` | A validated newtype, e.g. `openfinance-v2`. |
| `SourceLocator` | `page:N`, `page:N-M`, `section:X`, `chunk:ID`, `path:/p`, `op:METHOD /p`, `schema:Name`. Parsed, and `Display` round-trips. |
| `Provenance` | `source_id`, `document_id`, `version`, `kind`, `authority`, `precedence`, `locator`, `sha256` (the record), and `document_sha256`. |
| `EvidenceChunk` | A PDF or text chunk with page, section, ordinal, text, and provenance. |
| `OpenApiOperation` / `OpenApiSchema` | Normalized semantic records (see INDEXING.md). |
| `Requirement`, `RequirementSource`, `RequirementTrace` | The curated mapping and its trace. |
| `CompatibilityChange`, `SchemaChange` | Structural diff output: `added`, `removed`, `modified`, or `unchanged`, with JSON pointers. |
| `Conflict` | Kinds: `declared`, `dangling_reference`, `precedence_tie`, `stale_source`, `duplicate_definition`. Carries citations and provenance. |
| `SearchResult` | A BM25 hit, always classified `discovered_evidence`. |
| `EvidenceClass` | `authoritative_requirement`, `discovered_evidence`, or `source_content`. |

**Invariant:** every source-backed record returned by any tool carries a `Provenance`. No evidence is
returned without it.

## 4. Storage

### SQLite (`data/catalog.db`): authoritative metadata and content

Migration `crates/bg-spec-store/migrations/0001_init.sql`:

| Table | Content |
|---|---|
| `index_meta` | Key/value pairs: schema_version, index_generation, last_indexed_at_unix, requirements_file_sha256, indexer_format_version, index_in_progress. |
| `documents` | One row per manifest source. Holds status, sha256, fingerprint, and the full `Document` JSON. |
| `pages` | Per-page status (`extracted`, `ocr_required`, `extraction_failed`, `blank`), text, and sha256. |
| `chunks` | `chunk_id`, `source_id`, `document_id`, version, page, section, ordinal, locator, text, sha256, and JSON. |
| `openapi_operations` | Unique on (source, method, path). Also stores a canonical `path_key` for cross-version matching, plus the JSON. |
| `openapi_schemas` | Unique on (source, name). Stores the JSON and the referenced schema names. |
| `requirements` | The curated requirement JSON plus mapping provenance (file and file sha256). |
| `requirement_sources` | Deliberately has **no FK** to `documents`, so dangling references stay visible. |
| `requirement_endpoints` | The (method, path) links. |

The indexer opens the database read-write with WAL and foreign keys, and truncates the WAL
checkpoint at the end of each run. The MCP opens it `read_only(true)`, checks `schema_version`, and never migrates.

### Tantivy (`data/tantivy/`): full-text retrieval

| Field | Options |
|---|---|
| `record_id`, `record_type`, `source_id`, `document_id`, `version`, `kind`, `authority`, `locator`, `sha256` | `STRING \| STORED`. These are exact-match filters. |
| `page` | `u64`, indexed and stored |
| `title`, `content` | `TEXT` with `en_stem`, stored. Titles get a 2× boost. |

The index holds chunk, OpenAPI operation, and OpenAPI schema records. Queries are parsed leniently
(`parse_query_lenient`). Filters are `TermQuery` must-clauses. Results are ordered by BM25 score, then
`record_id`, so the order is stable. A marker file `bg-spec-search.version` guards schema compatibility.

## 5. Flows

### Indexing (`bg-spec index`)

```text
manifest.yaml ─► validate ─► for each source:
   resolve path inside corpus root (reject .., absolute, escaping symlinks)
   sha256(file) + fingerprint(format version, manifest entry, chunking, extractor)
   unchanged & indexed? ─► skip
   else extract (spawn_blocking) ─► normalize/chunk ─► SQLite txn replace ─► Tantivy delete+add
removed from manifest ─► delete from both stores
requirements.yaml ─► validate ─► replace requirement tables
commit Tantivy, checkpoint SQLite, bump index_generation
```

An `index_in_progress` flag forces a full rebuild if a previous run was interrupted.

### Runtime (`bg-spec-mcp`)

```text
start ─► load config ─► open SQLite (ro) + Tantivy reader ─► serve stdio
tool call ─► typed input (serde, deny_unknown_fields) ─► service ─► repositories ─► JSON + provenance
```

The Tantivy reader reloads on commit, so a running server picks up a re-index without a restart.
`health` shows the `index_generation`.

## 6. Services

* **SpecificationService**: `search`, `read_source`, `read_endpoint`, `read_schema` (with the transitive
  `$ref` closure), `list_sources`, `health`. Resolves endpoint paths across versions through the
  configured prefixes (`/v1`) and a canonical key that ignores parameter names.
* **RequirementService**: `find` returns curated matches, with search hits kept separate. `trace` and
  `endpoint_requirements` implement the seven-step bundle and detect conflicts.
* **CompatibilityService**: `compare` runs `diff::diff_operations`, flattens the schema changes, and derives
  factual `compatibility_relevant_facts`. It never produces a verdict.

## 7. Key design decisions

| Decision | Rationale |
|---|---|
| Content is served from SQLite, not re-read from files. | Runtime needs no file parsing, content is consistent with the hashes, and startup is fast. |
| Search and catalog are separate stores. | Tantivy handles ranking; SQLite is the authoritative record, provenance, and exact-read store. |
| No compatibility verdict. | "Backward compatible" depends on client behaviour, so the tool reports facts only. |
| Conflicts are reported, not resolved. | Resolution is an engineering decision for the agents and humans. |
| OpenAPI 3.1 goes through the generic JSON path. | `openapiv3` only models 3.0. A diagnostic is emitted and the same normalized records are produced. |
| `PdfExtractor` is a trait. | A Pdfium or OCR implementation can be plugged in without changing the pipeline. |
| The MCP does not depend on the indexer. | This enforces the read-only runtime at compile time. |
