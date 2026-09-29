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
  - id: OFV2-TRANSACTIONS-001                # [A-Z0-9][A-Z0-9._-]*, ≤128 chars, unique
    version: openfinance-v2
    title: Read transaction list with mandatory bookingStatus and dateFrom
    description: >-                          # optional
      ...
    sources:                                 # ≥1: no requirement without provenance
      - source_id: bg-openfinance-v2-implementation-guidelines
        locator: section:4.2                 # any locator (see MCP_TOOLS.md)
        note: Read Transaction List          # optional
      - source_id: bg-openfinance-v2-implementation-guidelines
        locator: page:3
        sha256: 3f5a9c0d1e2b                 # optional pin (≥12 hex chars, prefix of the document hash)
      - source_id: bg-openfinance-v2-openapi
        locator: op:GET /accounts/{accountId}/transactions
    endpoints:                               # optional "METHOD /path"
      - GET /accounts/{accountId}/transactions
    schemas: [TransactionsResponse200Json]   # optional component schema names
    acceptance_criteria:
      - "A request without bookingStatus is rejected with HTTP 400."
    conflicts_with:                          # optional, never self-referencing
      - requirement_id: OFV2-TRANSACTIONS-003
        note: Operational rules vs errata E-07
    tags: [ais, transactions]
```

The file is validated during indexing (for schema version, id format, duplicates, empty titles, missing sources,
locator syntax, pin format, and self-conflicts), so an invalid file fails `bg-spec index`. If a `source_id` is unknown to the
manifest, the indexer emits a `dangling_source` warning and the trace reports a `dangling_reference` conflict.

Every requirement is stored with **mapping provenance**: `{file, file_sha256, locator:"requirement:<ID>",
sha256}`. Acceptance criteria get stable ids (`<ID>/AC1`, `<ID>/AC2`, …) and carry the same mapping provenance.

## How requirements are linked to an endpoint

`get_endpoint_requirements` links a curated requirement to an operation when either of these is true:
* it lists the endpoint under `endpoints` (compared by canonical path key, so `{accountId}` ≡ `{account-id}`), or
* one of its sources uses an `op:` or `path:` locator on an OpenAPI source of that version.

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

## Example-corpus scenarios

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
