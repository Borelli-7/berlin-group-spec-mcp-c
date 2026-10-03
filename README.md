# Berlin Group Specification MCP

A local, **read-only**, deterministic [Model Context Protocol](https://modelcontextprotocol.io) server
written in Rust. It serves the **Berlin Group NextGenPSD2 / Open Finance** specification corpus
(OpenAPI YAML, PDF specifications, errata and curated requirement mappings) to GitHub Copilot CLI agents
running under Herdr.

The server is the **specification-evidence layer** for the multi-agent workflow that implements
Open Finance v2 on top of NextGenPSD2 v1.3 in `aspsp-xs2a`, `ledgers`, `xs2a-connector-examples` and
`psd2-dynamic-sandbox`:

```text
Official/curated specification corpus
        ↓
Indexing pipeline            (bg-spec index)
        ↓
SQLite provenance store + Tantivy search index
        ↓
Read-only Rust MCP           (bg-spec-mcp, stdio)
        ↓
Copilot CLI
        ↓
Requirements / Developer / Tester / Architect agents
```

It **retrieves, parses, indexes, compares and cites**. It never interprets requirements, never
resolves conflicts, never produces a compatibility verdict and never calls an LLM.

| Guarantee | How |
|---|---|
| Deterministic | Pure parsing and structural diffing; stable ordering; snapshot-tested output. |
| Source-traceable | Every record carries `source_id`, `document_id`, `version`, `kind`, `locator`, `sha256`. |
| Evidence ≠ requirement | Search hits are `discovered_evidence`; only curated mappings are `authoritative_requirement`. |
| Read-only | No write/exec tools; SQLite opened `mode=ro`; no caller-supplied paths. |
| Fast startup | Opens existing indexes only; never parses documents at startup. |

## Quick start

```bash
cd berlin-group-spec-mcp
cargo build --release                                   # bg-spec, bg-spec-mcp
./target/release/bg-spec index  --config config/config.toml
./target/release/bg-spec doctor --config config/config.toml
./target/release/bg-spec stats  --config config/config.toml
./target/release/bg-spec sources --config config/config.toml
```

Register the server with Copilot CLI (see [CONFIGURATION.md](CONFIGURATION.md#copilot-cli)):

```bash
examples/copilot/install.sh        # prints the config and merges it into ~/.copilot/mcp-config.json
# alternatively, copy examples/copilot/.mcp.json into a repository
```

Then, inside Copilot CLI, `/mcp` should list `berlin-group-spec` with 10 tools.

## Tools

| Tool | Purpose |
|---|---|
| `get_endpoint_requirements` | **Primary Requirements Agent tool**: evidence bundle for one endpoint. |
| `read_openapi_endpoint` | Normalized OpenAPI operation. |
| `read_openapi_schema` | Component schema + referenced schemas. |
| `trace_requirement` | Requirement → sources → endpoint → schemas → acceptance criteria → evidence → conflicts. |
| `find_requirement` | Curated requirements (authoritative) + discovered evidence (separately). |
| `read_source` | Exact indexed content by `source_id` + locator. |
| `search_specification` | BM25 full-text search with version/kind/source filters. |
| `compare_v1_v2` | Structural endpoint/schema diff between versions; no verdict. |
| `list_sources` | Catalog of sources with authority, precedence, hash and status. |
| `health` | Operational status. |

Full reference, example calls and captured responses: [MCP_TOOLS.md](MCP_TOOLS.md),
[`examples/calls`](examples/calls), [`examples/responses`](examples/responses).

Recommended Requirements Agent flow (the acceptance scenario, tested in
`crates/bg-spec-indexer/tests/integration.rs::acceptance_scenario_transactions_endpoint`):

```text
get_endpoint_requirements → read_openapi_endpoint → trace_requirement → read_source → compare_v1_v2
```

## Corpus

`corpus/` contains the **official Berlin Group publications** (NextGenPSD2 1.3.16 and openFinance API
Framework v2.x PDFs and OpenAPI files, 28 sources), published by the Berlin Group mostly under the
Creative Commons Attribution-NoDerivatives 4.0 license; they are redistributed unmodified. Authority and
precedence are **declared** in `corpus/manifest.yaml`, never inferred from file names.
`corpus/requirements/requirements.yaml` holds 102 curated requirements, and `corpus/text/v2/errata.md`
records project clarifications on discrepancies between the official files (not a Berlin Group publication).

Tests use a separate **synthetic fixture corpus** in `crates/bg-spec-indexer/tests/fixtures/corpus/`
(original test text with deliberate differences, an image-only PDF page and curation conflicts).
See [INDEXING.md](INDEXING.md) and [REQUIREMENT_TRACEABILITY.md](REQUIREMENT_TRACEABILITY.md).

## Workspace

```text
crates/
  bg-spec-core     domain model, manifest/requirements parsing, diff engine, services, ports
  bg-spec-store    SQLite catalog (sqlx) + Tantivy search (implements the ports)
  bg-spec-indexer  discovery, SHA-256, PDF extraction (pdf_oxide), OpenAPI normalization, chunking
  bg-spec-mcp      rmcp stdio server (thin adapters)            → binary bg-spec-mcp
  bg-spec-cli      index / doctor / stats / sources             → binary bg-spec
corpus/            official corpus: manifest.yaml, openapi/, pdf/, text/, requirements/
crates/bg-spec-indexer/tests/fixtures/corpus/   synthetic test corpus
config/config.toml
data/              CURRENT, generations/<n>/{catalog.db,tantivy/}   (generated; git-ignored)
examples/          Copilot configs, example calls and responses
```

## Documentation

* [ARCHITECTURE.md](ARCHITECTURE.md) – layers, data model, flows, design decisions
* [CONFIGURATION.md](CONFIGURATION.md) – `config.toml`, manifest, Copilot CLI setup
* [INDEXING.md](INDEXING.md) – pipeline, PDF/OpenAPI strategy, incremental indexing, OCR
* [MCP_TOOLS.md](MCP_TOOLS.md) – tool reference with examples
* [REQUIREMENT_TRACEABILITY.md](REQUIREMENT_TRACEABILITY.md) – `requirements.yaml`, tracing, conflicts
* [SECURITY.md](SECURITY.md) – threat model and read-only guarantees
* [CONTRIBUTING.md](CONTRIBUTING.md) – development workflow and quality gate

## Troubleshooting

| Symptom | Fix |
|---|---|
| `catalog not found at …; run bg-spec index` | Run `bg-spec index --config …` before starting the MCP. |
| `catalog schema version … != expected` / `incompatible schema` | The index was built by another version: `bg-spec index --force`. |
| `doctor` reports `hash_drift` | A corpus file changed since indexing: `bg-spec index`. |
| `doctor` reports `ocr_required` | The PDF page is image-only; see [INDEXING.md](INDEXING.md#ocr-detection). |
| Copilot shows no tools | Check absolute paths in `mcp-config.json`; run the command manually; check stderr logs. |
| Garbled MCP traffic | Nothing but MCP frames is written to stdout; logs go to stderr (`BG_SPEC_LOG=debug`). |
| Build error in `office_oxide` | Keep the committed `Cargo.lock` (see [CONTRIBUTING.md](CONTRIBUTING.md#dependency-pins)). |
