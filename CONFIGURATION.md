# Configuration

## `config/config.toml`

Relative paths are resolved against the **directory that contains the config file**.

```toml
corpus_root  = "../corpus"                        # directory with manifest.yaml and the documents
data_dir     = "../data"                          # CURRENT and generations/<n>/{catalog.db,tantivy/}
manifest     = "manifest.yaml"                    # relative to corpus_root (default shown)
requirements = "requirements/requirements.yaml"   # relative to corpus_root (default shown)

[versions]
baseline = "nextgenpsd2-v1.3"   # default `from` for compare_v1_v2
target   = "openfinance-v2"     # default version for endpoint/schema tools and the `to` side of compare

[versions.path_prefixes]
# The version publishes paths under a prefix, and the prefix is stripped for cross-version matching.
"nextgenpsd2-v1.3" = "/v1"
"openfinance-v2"   = "/v2"

[chunking]
max_chars     = 6000   # >= 500
overlap_chars = 500    # < max_chars / 2

[pdf]
min_chars_per_page = 16   # pages with fewer non-whitespace characters are marked ocr_required

[index]
jobs = 0               # sources prepared concurrently; 0 = automatic (min(CPUs, 8)), max 64; `bg-spec index --jobs N` overrides

[search]
default_limit = 10
max_limit     = 50

[log]
level = "info"         # tracing filter; overridden by the BG_SPEC_LOG environment variable
```

Unknown keys are rejected. `manifest` and `requirements` must be relative paths without `..`.

Changing the chunking or PDF settings changes each document's fingerprint, so the next `bg-spec
index` re-indexes the affected documents automatically.

### Environment variables

| Variable | Meaning |
|---|---|
| `BG_SPEC_CONFIG` | Default for `--config` (both binaries). |
| `BG_SPEC_LOG` | `tracing` filter, e.g. `warn` or `bg_spec_core=debug`. Logs always go to stderr. `bg_spec::timing=debug` logs per-operation latency (`op`, `elapsed_us`) for service calls and indexer stages. |

No secrets are required.

## `corpus/manifest.yaml`

```yaml
schema_version: 1
sources:
  - id: bg-openfinance-v2-xs2a-implementation-guidelines   # stable id: [a-z0-9][a-z0-9._-]{0,127}
    kind: pdf                                              # pdf | openapi | text
    version: openfinance-v2
    authority: normative      # normative | technical | informative | project | test | unknown
    precedence: 100           # 0..=1000; higher wins when sources of one version overlap
    path: pdf/v2/XS2A API as PSD2 Interface implementation guide line-2.4.pdf   # relative to the corpus root
    title: openFinance XS2A API as PSD2 Interface - Implementation Guidelines 2.4 (2025-10-31)   # optional
    description: ...                                       # optional
    tags: [implementation-guidelines]                      # optional
    blank_pages: [2]                                       # optional, pdf/text only: intentionally blank pages
```

Rules:
* `authority` and `precedence` are **declared**. Nothing is inferred from file names.
* Ids must be unique. Paths must be relative, must not contain `..`, and must stay within the corpus root after symlink resolution.
* `text` sources are UTF-8 files (Markdown or plain text). Form feed (`\f`) separates pages, and
  numbered or Markdown headings become sections.
* `blank_pages` lists physical page numbers that are intentionally blank. Such a page is stored with
  `status: blank` instead of `ocr_required`, so it does not make the document `partial` or `health` `degraded`.
  Only declare a page after checking that it really has no content (e.g. `pdffonts`/`pdfimages -f N -l N`).
* Files under `openapi/`, `pdf/` or `text/` that are not listed in the manifest are reported as orphans,
  but are never indexed.

## Copilot CLI

Build the release binaries and index the corpus first:

```bash
cargo build --release
./target/release/bg-spec index --config config/config.toml
```

### User-wide configuration (`~/.copilot/mcp-config.json`)

`examples/copilot/install.sh` prints [`examples/copilot/mcp-config.json`](examples/copilot/mcp-config.json)
with absolute paths for your checkout. Merge its `mcpServers.berlin-group-spec` entry into
`~/.copilot/mcp-config.json`:

```json
{
  "mcpServers": {
    "berlin-group-spec": {
      "type": "local",
      "command": "/absolute/path/berlin-group-spec-mcp/target/release/bg-spec-mcp",
      "args": ["--config", "/absolute/path/berlin-group-spec-mcp/config/config.toml"],
      "env": { "BG_SPEC_LOG": "warn" },
      "tools": [
        "search_specification", "find_requirement", "read_source",
        "read_openapi_endpoint", "read_openapi_schema", "get_endpoint_requirements",
        "trace_requirement", "compare_v1_v2", "list_sources", "health"
      ]
    }
  }
}
```

### Repository configuration (`.mcp.json`)

Copy [`examples/copilot/.mcp.json`](examples/copilot/.mcp.json) to the root of `aspsp-xs2a` (or any of
the other repositories), then replace the `/absolute/path` placeholders.

### Verification

1. `bg-spec-mcp --config …` started in a terminal should wait silently on stdin. Stop it with Ctrl-D.
2. In Copilot CLI, `/mcp` lists `berlin-group-spec`. Ask the agent to call `health`.
3. With several Herdr agents, each agent starts its own read-only server process. SQLite in WAL
   mode and Tantivy both support any number of concurrent readers.

### Re-indexing while agents are running

`bg-spec index` can run while MCP servers are live. Running servers keep serving the previous
Tantivy commit until the new commit lands, after which the reader reloads automatically. Check that the
`index.index_generation` value reported by `health` has increased.
