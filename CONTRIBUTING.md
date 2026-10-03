# Contributing

## Quality gate (required before every commit)

```bash
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --workspace
cargo build --release
```

The MSRV is Rust 1.91 (edition 2024), and `rust-toolchain.toml` tracks stable.

## Architecture rules

* `bg-spec-core` holds the domain types, ports, and services. It must not depend on SQLite, Tantivy, pdf_oxide or rmcp.
* `bg-spec-store` implements the ports, and `bg-spec-indexer` is the **only** writer.
* `bg-spec-mcp` must **never** depend on `bg-spec-indexer` outside `[dev-dependencies]`. Its tools must stay
  read-only (the `read_only_hint = true` annotation), must accept no filesystem paths, and must contain no business logic.
  Put logic in a core service and add a thin adapter.
* Never add an LLM call or network access to any crate.
* Every returned evidence item must carry `Provenance`.
* No `unsafe`: every crate declares `#![forbid(unsafe_code)]`.
* Expected input errors are `CoreError` variants and must never cause a panic.

## Adding a tool

1. Add the service method (with unit tests) to `bg-spec-core`.
2. Add a typed input to `crates/bg-spec-mcp/src/inputs.rs` (`deny_unknown_fields`, doc comments
   become the JSON schema).
3. Add a `#[tool(annotations(read_only_hint = true, …))]` method that returns `Result<Json<T>, ToolError>`.
   The literal return type is required for rmcp to detect the output schema.
4. Add the tool to `TOOL_NAMES`, the MCP tests, `MCP_TOOLS.md`, and the Copilot config examples.

## Tests and snapshots

* Unit tests live next to the code. Integration tests are in `crates/bg-spec-indexer/tests/`, MCP
  protocol tests use an in-memory duplex in `crates/bg-spec-mcp/tests/`, and CLI tests are in `crates/bg-spec-cli/tests/`.
* Integration and MCP tests copy the synthetic fixture corpus `crates/bg-spec-indexer/tests/fixtures/corpus/`
  into a temp dir (`bg_spec_indexer::testing`); they never read the official files in `corpus/`.
  `cargo test -p bg-spec-indexer --release --test real_corpus -- --ignored` smoke-tests `corpus/`.
* Snapshots use `insta`. To accept intended changes:
  ```bash
  INSTA_UPDATE=always cargo test -p bg-spec-indexer --test integration   # or: cargo insta review
  ```
  Review the diff. Snapshots must not contain absolute paths or timestamps.
* Retrieval gold cases live in `crates/bg-spec-indexer/tests/fixtures/eval*.yaml`.
  `cargo test -p bg-spec-indexer --test eval -- --nocapture` gates fixture recall@5,
  judged precision@5, MRR@5, endpoint identity and citation resolution. Each query has
  one judged relevant locator; precision does not claim exhaustive relevance labels.
  Run the official-corpus counterpart with
  `cargo test -p bg-spec-indexer --release --test eval -- --ignored --nocapture`.
  Review labels when adding cases; never lower a baseline to accommodate a regression.

## Fixtures

The corpus in `corpus/` is **synthetic**. Never commit copyrighted Berlin Group documents.
To regenerate the fixture PDFs, which are deterministic minimal PDFs that include one blank page simulating a scanned page:

```bash
cargo run -p bg-spec-indexer --example generate_fixtures -- corpus
```

After changing the corpus, re-run the snapshots and recapture the example responses:

```bash
cargo build --release && ./target/release/bg-spec index --config config/config.toml
python3 examples/capture.py target/release/bg-spec-mcp config/config.toml
```

## Dependency pins

`pdf_oxide 0.3.78` does not build with newer `office_oxide` releases, so `Cargo.lock` pins
`office_oxide = 0.1.9`. **Do not run a bare `cargo update`**. Instead, update specific crates with
`cargo update -p <crate>` and re-run the quality gate.

## Commits

Use small, focused commits with an imperative subject line. Mention any snapshot updates in the body.
