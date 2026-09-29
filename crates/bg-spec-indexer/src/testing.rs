//! Test support: stage the bundled example corpus into a scratch directory and build a
//! matching configuration. Used by integration and MCP tests; it never touches the
//! repository's own `data/` directory.

use bg_spec_core::{Result, config::Config, error::CoreError};
use std::path::{Path, PathBuf};

/// Path of the synthetic fixture corpus used by tests (independent of the real `corpus/`).
pub fn example_corpus_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/corpus")
}

/// Path of the workspace corpus with the official Berlin Group files.
pub fn workspace_corpus_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus")
}

/// Copies the example corpus into `dest/corpus` and returns a config whose data directory is
/// `dest/data`.
pub fn stage_example_corpus(dest: &Path) -> Result<Config> {
    stage_corpus(&example_corpus_dir(), dest)
}

/// Copies `src` into `dest/corpus` and returns a config whose data directory is `dest/data`.
pub fn stage_corpus(src: &Path, dest: &Path) -> Result<Config> {
    let corpus = dest.join("corpus");
    copy_dir(src, &corpus)?;
    config_for(dest, &corpus)
}

/// Builds a configuration equivalent to `config/config.toml` for the given corpus.
pub fn config_for(dest: &Path, corpus: &Path) -> Result<Config> {
    let raw = format!(
        r#"corpus_root = {corpus:?}
data_dir = {data:?}

[versions]
baseline = "nextgenpsd2-v1.3"
target = "openfinance-v2"

[versions.path_prefixes]
"nextgenpsd2-v1.3" = "/v1"
"openfinance-v2" = "/v2"
"#,
        corpus = corpus.display().to_string(),
        data = dest.join("data").display().to_string(),
    );
    Config::parse(&raw, dest)
}

fn copy_dir(src: &Path, dst: &Path) -> Result<()> {
    let io = |p: &Path, e| CoreError::io(p.display().to_string(), e);
    std::fs::create_dir_all(dst).map_err(|e| io(dst, e))?;
    for entry in std::fs::read_dir(src).map_err(|e| io(src, e))? {
        let entry = entry.map_err(|e| io(src, e))?;
        let (from, to) = (entry.path(), dst.join(entry.file_name()));
        if entry.file_type().map_err(|e| io(&from, e))?.is_dir() {
            copy_dir(&from, &to)?;
        } else {
            std::fs::copy(&from, &to).map_err(|e| io(&from, e))?;
        }
    }
    Ok(())
}
