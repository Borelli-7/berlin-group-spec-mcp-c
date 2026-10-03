//! Storage adapters: SQLite catalog (authoritative metadata/provenance) and Tantivy
//! full-text index. Both implement the read ports of `bg-spec-core`; write APIs are used
//! by the indexer only.

#![forbid(unsafe_code)]

pub mod layout;
pub mod search;
pub mod sqlite;

pub use layout::IndexLayout;
pub use search::{SearchDocument, TantivySearch, TantivyWriter};
pub use sqlite::{CATALOG_SCHEMA_VERSION, DocumentBundle, SqliteCatalog, StoredOperation};

use bg_spec_core::{Result, config::Config};
use std::sync::Arc;

/// Opens both stores of the published generation read-only (MCP runtime). Never builds or
/// migrates anything.
pub async fn open_read_only(config: &Config) -> Result<(Arc<SqliteCatalog>, Arc<TantivySearch>)> {
    let (catalog, search, _) = open_current(config).await?;
    Ok((catalog, search))
}

/// Like [`open_read_only`], also returning the layout that was opened.
pub async fn open_current(
    config: &Config,
) -> Result<(Arc<SqliteCatalog>, Arc<TantivySearch>, IndexLayout)> {
    let layout = layout::current_layout(&config.data_dir)?;
    let catalog = SqliteCatalog::open_read_only(&layout.catalog_path).await?;
    let search = TantivySearch::open(&layout.tantivy_dir)?;
    Ok((Arc::new(catalog), Arc::new(search), layout))
}
