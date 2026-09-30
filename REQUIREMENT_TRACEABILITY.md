# Requirement traceability

The MCP server never creates requirements. **Curated mappings** in `corpus/requirements/requirements.yaml`
are the only `authoritative_requirement` records. All other records are `discovered_evidence`.

```text
Requirement ──► Source (+ locator, optional sha256 pin)
            ──► Endpoint (OpenAPI operation)
            ──► Schema(s) (transitive)
            ──► Acceptance criteria
            ──► Related evidence (search, discovered)
            ──► Conflicts (reported, never resolved)
```

## `requirements.yaml`

```yaml
schema_version: 1
requirements:
  - id: OFV2-AIS-TRANSACTIONS-001            # [A-Z0-9][A-Z0-9._-]*, ≤128 chars, unique
    version: openfinance-v2
    title: Read transaction list with mandatory bookingStatus and conditional dateFrom
    description: >-                          # optional
      ...
    sources:                                 # ≥1: no requirement without provenance
      - source_id: bg-openfinance-v2-xs2a-implementation-guidelines
        locator: section:4.4.4               # any locator (see MCP_TOOLS.md)
        note: Read Transaction List          # optional
      - source_id: bg-openfinance-v2-xs2a-implementation-guidelines
        locator: page:95-101                 # physical PDF pages, not the printed footer numbers
        sha256: 3f5a9c0d1e2b                 # optional pin (≥12 hex chars, prefix of the document hash)
      - source_id: bg-openfinance-v2-openapi-ais
        locator: op:GET /v2/accounts/{account-id}/transactions
    endpoints:                               # optional "METHOD /path"
      - GET /v2/accounts/{account-id}/transactions
    schemas: [accountReport, transactions]   # optional component schema names
    acceptance_criteria:
      - "A request without the bookingStatus query parameter is rejected with HTTP 400 FORMAT_ERROR."
    conflicts_with:                          # optional, never self-referencing
      - requirement_id: OFV2-EXAMPLE-002     # see OFV2-SCA-APPROACH-001 ↔ -002 for a real case
        note: why the two requirements contradict each other
    tags: [ais, transactions]
```

`page:N` locators count the physical pages of the PDF (as extracted by the indexer). Many Berlin Group
PDFs print a different number in the footer, for example physical page 95 of the XS2A Implementation
Guidelines 2.4 is printed as page 90.

The file is validated during indexing (for schema version, id format, duplicates, empty titles, missing sources,
locator syntax, pin format, and self-conflicts), so an invalid file fails `bg-spec index`. If a `source_id` is unknown to the
manifest, the indexer emits a `dangling_source` warning and the trace reports a `dangling_reference` conflict.

Every requirement is stored with **mapping provenance**: `{file, file_sha256, locator:"requirement:<ID>",
sha256}`. Acceptance criteria get stable ids (`<ID>/AC1`, `<ID>/AC2`, …) and carry the same mapping provenance.

## How requirements are linked to an endpoint

`get_endpoint_requirements` links a curated requirement to an operation when either of these is true:
* it lists the endpoint under `endpoints` (compared by canonical path key, so `{accountId}` ≡ `{account-id}`), or
* one of its sources uses an `op:` or `path:` locator on an OpenAPI source of that version.

The canonical path key strips the version prefix configured in `[versions.path_prefixes]`
(`/v1` for `nextgenpsd2-v1.3`, `/v2` for `openfinance-v2`), so `/v1/accounts/{account-id}/transactions`
and `/v2/accounts/{account-id}/transactions` share the key `/accounts/{}/transactions`. A lookup first
uses the prefix of the requested version and then the prefixes of the other versions, so a v1 path
finds the v2 operation and vice versa (`matched_by: canonical`). When several templates share one key
(e.g. `/{payment-service}/{payment-product}/{paymentId}` and
`/{resource-path}/{resourceId}/{authorisation-category}`), the operation with the same parameter names
is preferred. Structurally different paths (v1 `POST /v1/{payment-service}/{payment-product}` vs
v2 `POST /v2/payments/{payment-product}`, `/v1/consents` vs `/v2/consents/account-access`) are not
matched; cite both explicitly in the requirement if they belong together.

## Trace output (`trace_requirement`)

Each **source** is resolved to indexed content, with an excerpt and full provenance, and gets a status:

| status | meaning |
|---|---|
| `resolved` | The locator resolves to indexed content. |
| `stale` | The locator resolves, but the pinned `sha256` no longer matches the document. |
| `dangling` | The source exists but the locator does not resolve (e.g. the page is out of range, or the section is unknown). |
| `unknown_source` | The `source_id` is not in the catalog. |

Endpoints and schemas carry `via`, which records how they were reached, and `found`, each with provenance:
* endpoints: `endpoints` (declared) or `source:<source_id> <locator>` (from an `op:`/`path:` citation);
* schemas: `explicit` (declared), `operation:<METHOD /path>` (referenced by a linked operation) or
  `transitive:<Parent>` (reached through `$ref`).

## Conflict detection (deterministic)

| kind | rule |
|---|---|
| `declared` | `conflicts_with` in either direction. Citations are the sources of both requirements. |
| `dangling_reference` | A cited source, locator, endpoint, or schema does not exist or is not indexed. |
| `precedence_tie` | Cited sources of the **same version** share the same `precedence` but differ in `authority`. |
| `stale_source` | A pinned `sha256` is not a prefix of the indexed document hash. |
| `duplicate_definition` | The same operation or schema is defined with different content by several sources of one version. |

The server **reports** conflicts with citations and provenance. Deciding which source prevails
(for example by comparing precedence and authority) is left to the consuming agent or a human.

## Corpus scenarios

### Official corpus (`corpus/`)

`corpus/requirements/requirements.yaml` maps 101 requirements (72 `OFV2-*`, 29 `NGPSD2-*` baseline) onto the
official Berlin Group files. Every openFinance v2 OpenAPI operation is linked to at least one requirement;
NextGenPSD2 1.3 operations are covered where a v2 counterpart exists, and endpoint-less requirements cover
protocol functions, operational rules, administrative services and the Data Dictionary message codes.
Discrepancies between those files are recorded as project errata in
`corpus/text/v2/errata.md` (source `project-openfinance-v2-errata`, authority `project`, precedence 10).

| Requirement | Demonstrates |
|---|---|
| `OFV2-AIS-TRANSACTIONS-001` | A clean trace: IG 2.4 section 4.4.4 + pages 95-101 + Operational Rules 4.10 + Data Dictionary + OpenAPI operation → schemas → 7 criteria |
| `OFV2-SCA-APPROACH-001` ↔ `-002` | `declared` conflict: Protocol Functions 8.4.2 lists `SIGNATURE`, the OpenAPI enum omits it (errata E-01) |
| `OFV2-AIS-TRANSACTIONS-002`, `OFV2-AIS-FREQUENCY-001` | Normative sources complemented by project errata (E-03 `pageSize`, E-07 access counting) |
| `NGPSD2-AIS-TRANSACTIONS-001`, `NGPSD2-AIS-BALANCES-001` | v1.3 baseline reached from the v2 path through the `/v1` ↔ `/v2` canonical key |
| `OFV2-PIS-STATUS-CODES-001`, `OFV2-SCA-REQUEST-HEADERS-001` | Normative sources complemented by project errata (E-09 `PACT`/`PATC`, E-10 header names in change logs) |
| `OFV2-PIS-GET-PAYMENT-001`, `OFV2-AUTH-SUBRESOURCES-001` | Linked through `op:` citations only, because `GET /v2/{payment-service}/{payment-product}/{paymentId}` and `GET /v2/{resource-path}/{resourceId}/{authorisation-category}` share one canonical key |

The ignored smoke test `cargo test -p bg-spec-indexer --release --test real_corpus -- --ignored` indexes
this corpus and asserts that every cited locator resolves, that only declared conflicts remain and that
every openFinance v2 OpenAPI operation returns at least one curated requirement from `get_endpoint_requirements`.

### Test fixture corpus (`crates/bg-spec-indexer/tests/fixtures/corpus/`)

The integration and MCP tests use a small synthetic corpus that deliberately contains curation problems:

| Requirement | Demonstrates |
|---|---|
| `OFV2-TRANSACTIONS-001` | A clean trace: IG section 4.2 + page 3 + OpenAPI operation → schemas → 5 criteria |
| `OFV2-TRANSACTIONS-002` ↔ `-003` | `declared` conflict (operational rules vs errata E-07) |
| `OFV2-TRANSACTIONS-003` | `precedence_tie` and a `stale_source` pin |
| `OFV2-BALANCES-001` | `dangling_reference` (a page out of range and an unknown schema) |

Run `bg-spec doctor` to list all conflicts across the mapping file.

## Guidance for curators

* Cite the **narrowest** locator: prefer `section:` or `page:` over `document`.
* Pin `sha256` for sources whose wording you depend on. After an official update, `stale_source`
  flags the requirement for review.
* Put the endpoint in `endpoints` **and** cite the OpenAPI operation with `op:` so the contract has provenance.
* Record known contradictions with `conflicts_with` rather than deleting one side.
