//! Storage adapters: SQLite catalog (authoritative metadata/provenance) and Tantivy
//! full-text index. Both implement the read ports of `bg-spec-core`; write APIs are used
//! by the indexer only.

pub mod search;
pub mod sqlite;

pub use search::{SearchDocument, TantivySearch, TantivyWriter};
pub use sqlite::{DocumentBundle, SqliteCatalog, StoredOperation, CATALOG_SCHEMA_VERSION};

use bg_spec_core::{Result, config::Config};
use std::sync::Arc;

/// Opens both stores read-only (MCP runtime). Never builds or migrates anything.
pub async fn open_read_only(config: &Config) -> Result<(Arc<SqliteCatalog>, Arc<TantivySearch>)> {
    let catalog = SqliteCatalog::open_read_only(&config.catalog_path()).await?;
    let search = TantivySearch::open(&config.tantivy_dir())?;
    Ok((Arc::new(catalog), Arc::new(search)))
}
