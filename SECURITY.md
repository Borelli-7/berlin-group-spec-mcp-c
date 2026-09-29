# Security

## Threat model

Several AI coding agents under Herdr and Copilot CLI launch `bg-spec-mcp` and send it arbitrary tool
arguments. Those agents may be confused, prompt-injected, or buggy, so **every tool argument is
untrusted input**. The server holds no secrets and needs no network access.

## Guarantees

| Guarantee | Enforcement |
|---|---|
| **Read-only tools only** | All 10 tools are annotated `readOnlyHint: true` and `destructiveHint: false`. There are no write, delete, exec, or shell tools. Tested in `crates/bg-spec-mcp/tests/mcp.rs`. |
| **No file writes at runtime** | SQLite is opened with `read_only(true)` and migrations never run. Tantivy is opened with a reader only. The MCP crate does not link the indexer. A test hashes every file under the corpus and data directories before and after an MCP session and asserts they are unchanged. |
| **No arbitrary file reads** | No tool takes a filesystem path. `read_source` accepts a `source_id` and a locator. The id is looked up in the catalog, and content is served from the indexed records. Unknown ids return `not_found`, including values like `../../etc/passwd`. |
| **Path confinement at index time** | Manifest paths must be relative with no `..`. They are canonicalized and must stay under the canonical corpus root, so symlinks that escape the root are rejected. Only regular files are read. See `crates/bg-spec-indexer/src/discover.rs`. |
| **Strict input validation** | Inputs are typed and `deny_unknown_fields` is set. Locators are parsed against a fixed grammar. Versions must match `[A-Za-z0-9._-]{1,64}`. HTTP methods come from a fixed set. Query length and result limits are clamped. `$ref` resolution is depth-limited (16) and guarded against cycles. |
| **No panics on bad input** | Expected errors become `CoreError` and are returned as tool errors (`isError: true`, `{error:{code,message}}`). Internal failures become JSON-RPC internal errors. |
| **Clean stdio** | stdout carries only MCP frames. All logs go to stderr (`BG_SPEC_LOG` controls the level). |
| **No LLM, no network** | No HTTP client is used at runtime, and there is no model invocation. |
| **No `unsafe`** | Every crate root declares `#![forbid(unsafe_code)]`. |

## Recommended process isolation

The process only needs access to the following:

| Path | Access |
|---|---|
| `config/config.toml` | read |
| `corpus/` | read (the `health` and `doctor` commands only read metadata) |
| `data/catalog.db`, `data/tantivy/` | read. SQLite may need to create `-shm` for WAL readers, so give the directory read-write access if WAL files are absent. |

Optionally run the server under a restricted user, a read-only bind mount, or a sandbox (`systemd-run
--property=ProtectSystem=strict`, `bwrap`, or a container). Run `bg-spec index` as a separate, trusted step.

## Evidence integrity

* Every record has a SHA-256, and every document has a file SHA-256. Both are included in provenance.
* A requirement can pin a source hash (`sha256:`). A mismatch is reported as a `stale_source` conflict.
* `bg-spec doctor` detects hash drift between the corpus files and the index.

## Reporting a vulnerability

Please report privately to the maintainers rather than opening a public issue. Include the tool call,
the arguments, and the observed behaviour.
