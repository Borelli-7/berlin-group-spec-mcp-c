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
    responses (content, headers), the security requirement with `security_origin` (operation or global), the referenced
    schema names, and a JSON pointer.
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

Queries combine BM25 over `title` and `content` with `TermQuery` filters on `version`, `kind`, and
`source_id`. Every hit carries `source_id` and `locator`, so the exact record can be fetched from SQLite with `read_source`.

## SQLite

Migrations are embedded from `crates/bg-spec-store/migrations/`. The main tables are `documents`, `pages`, `chunks`,
`openapi_operations`, `openapi_schemas`, `requirements`, `requirement_sources`, `requirement_endpoints`,
and `index_meta`. The database is in WAL mode, and the MCP opens it read-only and checks the `schema_version`.

## Diagnostics

`file_missing`, `path_rejected`, `processing_failed`, `page_extraction_failed`, `ocr_required`,
`blank_page_has_text`, `blank_page_out_of_range`,
`no_extractable_text`, `openapi_typed_parse_failed`, `openapi_31_fallback`,
`openapi_operation_count_mismatch`, `requirements_missing`, and `dangling_source`. They are shown by `bg-spec index`,
`bg-spec sources`, `bg-spec doctor`, and the `diagnostics` field of `list_sources`.
