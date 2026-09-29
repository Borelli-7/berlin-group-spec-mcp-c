//! SQLite catalog repository.

mod read;
mod write;

pub use write::{DocumentBundle, StoredOperation};

use bg_spec_core::{CoreError, Result};
use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
};
use std::{path::Path, time::Duration};

/// Version of the catalog layout; bumped with incompatible migrations.
pub const CATALOG_SCHEMA_VERSION: u32 = 1;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

pub(crate) fn storage(e: impl std::fmt::Display) -> CoreError {
    CoreError::Storage(e.to_string())
}

/// SQLite-backed catalog. Opened read-write by the indexer and read-only by the MCP server.
#[derive(Clone)]
pub struct SqliteCatalog {
    pool: SqlitePool,
    read_only: bool,
}

impl SqliteCatalog {
    /// Opens (creating if needed) and migrates the catalog for indexing.
    pub async fn open_read_write(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)
                .map_err(|e| CoreError::io(dir.display().to_string(), e))?;
        }
        let opts = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .foreign_keys(true)
            .busy_timeout(Duration::from_secs(10));
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .map_err(storage)?;
        MIGRATOR.run(&pool).await.map_err(storage)?;
        let catalog = Self {
            pool,
            read_only: false,
        };
        catalog
            .set_meta("schema_version", &CATALOG_SCHEMA_VERSION.to_string())
            .await?;
        Ok(catalog)
    }

    /// Opens an existing catalog read-only. Fails if it was never indexed or has an
    /// incompatible schema. Never migrates.
    pub async fn open_read_only(path: &Path) -> Result<Self> {
        if !path.is_file() {
            return Err(CoreError::Config(format!(
                "catalog not found at {}; run `bg-spec index --config <config.toml>` first",
                path.display()
            )));
        }
        let opts = SqliteConnectOptions::new()
            .filename(path)
            .read_only(true)
            .create_if_missing(false)
            .foreign_keys(true)
            .busy_timeout(Duration::from_secs(10));
        let pool = SqlitePoolOptions::new()
            .max_connections(8)
            .connect_with(opts)
            .await
            .map_err(storage)?;
        let catalog = Self {
            pool,
            read_only: true,
        };
        let version: Option<String> =
            sqlx::query_scalar("SELECT value FROM index_meta WHERE key = 'schema_version'")
                .fetch_optional(&catalog.pool)
                .await
                .map_err(|e| {
                    CoreError::Config(format!(
                        "catalog is not initialized ({e}); run `bg-spec index`"
                    ))
                })?;
        match version.as_deref().and_then(|v| v.parse::<u32>().ok()) {
            Some(CATALOG_SCHEMA_VERSION) => Ok(catalog),
            other => Err(CoreError::Config(format!(
                "catalog schema version {other:?} != expected {CATALOG_SCHEMA_VERSION}; run `bg-spec index --force`"
            ))),
        }
    }

    pub fn is_read_only(&self) -> bool {
        self.read_only
    }

    pub(crate) fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    pub(crate) fn ensure_writable(&self) -> Result<()> {
        if self.read_only {
            Err(CoreError::Storage("catalog is opened read-only".into()))
        } else {
            Ok(())
        }
    }

    pub async fn set_meta(&self, key: &str, value: &str) -> Result<()> {
        self.ensure_writable()?;
        sqlx::query("INSERT INTO index_meta(key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value")
            .bind(key)
            .bind(value)
            .execute(&self.pool)
            .await
            .map_err(storage)?;
        Ok(())
    }

    pub async fn close(&self) {
        self.pool.close().await;
    }
}
