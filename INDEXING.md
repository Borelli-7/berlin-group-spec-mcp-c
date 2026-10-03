# Indexing

`bg-spec index --config config/config.toml` is the **only** component that writes. The MCP server
never parses documents and never rebuilds the index when it starts.

```text
manifest.yaml ──► discover (path confinement) ──► SHA-256 + fingerprint ──► unchanged? ──► skip
                                                                    │ changed/new
                                                                    ▼
             ┌──────── pdf ────────┐   ┌──── openapi ────┐   ┌──── text ────┐
             │ PdfExtractor (pages)│   │ YAML/JSON → tree│   │ \f → pages   │
             │ OCR detection       │   │ normalize ops & │   │              │
             │ page→section→para→  │   │ schemas, $ref   │   │ same chunker │
             │ chunk               │   │                 │   │              │
             └─────────┬───────────┘   └────────┬────────┘   └──────┬───────┘
                       ▼                        ▼                   ▼
        SQLite (documents, pages, chunks, openapi_*)  +  Tantivy (chunk / operation / schema docs)
                                                    ▼
                   requirements.yaml → requirements, requirement_sources/endpoints/…
```

## Incremental indexing

For each manifest source, the indexer performs these steps:

1. It resolves the path inside the canonical corpus root. A missing file becomes `status: missing`, and a path that escapes the root is rejected.
2. It computes the streaming **SHA-256** of the file and a **fingerprint**. The fingerprint is a SHA-256 over the manifest entry,
   the chunking settings, `min_chars_per_page`, the version path prefix, the extractor name, and the indexer format
   version.
3. If the **same hash and the same fingerprint** are found, and the previous status is `indexed` or `partial`, the source is **skipped**.
4. Otherwise it is re-extracted, its SQLite rows are replaced in a transaction, and its Tantivy documents are
   deleted by `source_id` and re-added.

Sources removed from the manifest are deleted from both stores. Each run increments
`index_generation`. A run that was interrupted (the `index_in_progress` flag is still set) triggers a full
rebuild on the next run. `--force` always rebuilds everything, as does a missing or incompatible Tantivy schema.

## PDF

* `trait PdfExtractor { fn name(&self); fn extract(&self, path) -> Result<ExtractedPdf> }`. The
  default implementation is `PdfOxideExtractor` (`pdf_oxide`). It runs inside `spawn_blocking`. Every page is
  returned; a page that fails records an error but does not abort the document.
* A future Pdfium or OCR extractor only has to implement this trait. Its `name()` becomes part of the
  fingerprint, so switching extractors re-indexes the PDFs automatically.

### OCR detection

A page with fewer than `pdf.min_chars_per_page` non-whitespace characters (the default is 16) is stored with
`status: ocr_required` and produces an `ocr_required` diagnostic. **It produces no chunk**, so the server never
returns empty evidence. A document with some OCR pages becomes `partial`, and one with no extractable page
gets the error diagnostic `no_extractable_text`. `read_source` returns such pages with an explicit status,
`health` reports `degraded`, and `bg-spec doctor` lists them.

Pages declared in the manifest's `blank_pages` are the exception: when they have no text they are stored with
`status: blank`, produce no chunk and no diagnostic, and do not degrade the document. If a declared page does
contain text it is indexed normally with the warning `blank_page_has_text`, and a number beyond the page count
yields `blank_page_out_of_range`.

### Extraction quality

Page text served by `read_source page:N` (and its `sha256`) is always the verbatim extraction. Only the text
that is chunked and indexed for search is cleaned, and only for PDFs:

- **Running headers/footers**: for documents with at least 4 extracted pages, a line among the first or last
  two non-empty lines of a page is treated as a margin when its digit-normalised form (`Page 12 of 40` →
  `Page # of #`) appears in the margins of at least `max(3, ⌈pages/2⌉)` pages. Such lines are dropped from
  the search text.
- **Hyphenation**: `word-⏎continuation` is joined only when both fragments are lowercase letters and the left
  fragment has at least two letters, so identifiers such as `X-Request-ID` or `{account-id}` are never merged.
- **Garbled text**: a page whose share of replacement, control or private-use characters exceeds 5 %, or
  where more than half of at least 20 tokens are single letters, gets the warning `low_quality_text`
  (locator `page:N`). It is still indexed; there is no OCR.

Per-document counts are stored in the document metadata as `extraction_quality` (`low_quality_pages`,
`mean_garbage_ratio`, `repeated_margin_patterns`, `removed_margin_lines`, `hyphenation_repairs`) and are
aggregated by `bg-spec stats` (and `--json`). `bg-spec doctor` lists `low_quality_text` pages as warnings.
Changing these rules bumps `INDEXER_FORMAT_VERSION`, so the next `bg-spec index` re-processes every document.

### Chunking

```text
page → section (heading) → paragraph → sentence → chunk
```

* Chunks **never cross pages**, so every chunk has an exact `page:N` citation.
* Headings are detected in these forms: numbered (`4.2.1 Read Transaction List`, depth ≤ 6), `Annex`/`Appendix`,
  and Markdown `#`. The current section carries over to the following pages.
* Paragraphs are packed up to `max_chars` (6000). A paragraph that is too long is split at **sentence**
  boundaries. A sentence, including any normative MUST/SHALL/REQUIRED statement, is only
  hard-split if it alone exceeds `max_chars`.
* The overlap (500) is sentence-aligned and taken from the tail of the previous chunk.
* The chunk id is `{source_id}:p{page}:c{ordinal}` and the locator is `chunk:{id}`. The `section` and `page` are stored
  with the chunk's own SHA-256.

## OpenAPI

* YAML is parsed with `serde-saphyr` and JSON with `serde_json`.
* **3.0.x** is validated with `openapiv3::OpenAPI`, which also supplies the API metadata. If the typed
  parse fails, the indexer emits `openapi_typed_parse_failed` and continues generically.
* **3.1/3.2** use the generic JSON projector and receive the diagnostic `openapi_31_fallback`.
* Records are projected from the raw JSON tree, so schema JSON is **verbatim**:
  * **Operations**, one per path and method, with the locator `op:GET /path`. They include `operationId`, summary,
    description, tags, deprecated, path-level and operation-level parameters with `$ref` resolved, the request body,
    responses (content, header names and full header definitions with `$ref` resolved), the security requirement with
    `security_origin` (operation or global), the referenced schema names, the `unresolved_references` (location and
    reason; the affected element is omitted and an `unresolved_ref` diagnostic is recorded), and a JSON pointer.
  * **Schemas**, one per `components.schemas` entry, with the locator `schema:Name` and their direct `$ref` names.
* `path_key` is the canonical path: the version prefix is stripped and parameters are replaced with `{}`. It enables
  cross-version matching (`/v1/accounts/{account-id}` ≡ `/v2/accounts/{account-id}` ≡ `/accounts/{accountId}`).

## Text sources

UTF-8 Markdown or plain text. Form feed (`\f`) separates pages, and the files go through the same chunker as PDFs.

## Tantivy

Each chunk, operation, and schema is one search document.

| Field | Options |
|---|---|
| `record_id`, `record_type`, `source_id`, `document_id`, `version`, `kind`, `authority`, `locator`, `sha256` | `STRING \| STORED` (exact filters) |
| `page` | `u64 INDEXED \| STORED` |
| `title`, `content` | `TEXT \| STORED` (BM25) |
| `identifiers` | exact, lower-cased identifiers (`bg_ident` tokenizer), not stored |

`identifiers` holds, per record type:

* operation: `operationId`, path, `METHOD path`, parameter names and response header names;
* schema: the component name;
* chunk: code-like tokens of the text (`PSU-IP-Address`, `E-07`, `bookingStatus`, `/v2/...` paths and
  `METHOD /path` pairs), at most 256 per chunk.

Queries combine BM25 over `title` and `content` with boosted exact `identifiers` terms (the whole query and each
token) and `TermQuery` filters on `version`, `kind`, and `source_id`. Every hit carries `source_id` and `locator`, so the exact record can be fetched from SQLite with `read_source`.

## SQLite

Migrations are embedded from `crates/bg-spec-store/migrations/`. The main tables are `documents`, `pages`, `chunks`,
`openapi_operations`, `openapi_schemas`, `requirements`, `requirement_sources`, `requirement_endpoints`,
and `index_meta`. The database is in WAL mode, and the MCP opens it read-only and checks the `schema_version`.

## Diagnostics

`file_missing`, `path_rejected`, `processing_failed`, `page_extraction_failed`, `ocr_required`, `low_quality_text`,
`blank_page_has_text`, `blank_page_out_of_range`,
`no_extractable_text`, `openapi_typed_parse_failed`, `openapi_31_fallback`,
`openapi_operation_count_mismatch`, `requirements_missing`, and `dangling_source`. They are shown by `bg-spec index`,
`bg-spec sources`, `bg-spec doctor`, and the `diagnostics` field of `list_sources`.
