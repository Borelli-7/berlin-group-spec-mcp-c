//! Regenerates the synthetic fixture PDFs of the test fixture corpus.
//!
//! ```bash
//! cargo run -p bg-spec-indexer --example generate_fixtures -- crates/bg-spec-indexer/tests/fixtures/corpus
//! ```

use bg_spec_indexer::pdf::fixture_corpus::fixture_pdfs;
use std::path::PathBuf;

fn main() -> std::io::Result<()> {
    let root = PathBuf::from(
        std::env::args()
            .nth(1)
            .unwrap_or_else(|| "crates/bg-spec-indexer/tests/fixtures/corpus".into()),
    );
    for pdf in fixture_pdfs() {
        let target = root.join(pdf.path);
        if let Some(dir) = target.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&target, pdf.render())?;
        eprintln!("wrote {}", target.display());
    }
    Ok(())
}
