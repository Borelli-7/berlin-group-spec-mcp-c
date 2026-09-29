//! Write side of the catalog (indexer only).

use super::{SqliteCatalog, storage};
use bg_spec_core::{
    Result,
    domain::{Document, DocumentStatus, EvidenceChunk, OpenApiOperation, OpenApiSchema, PageRecord, RequirementRecord},
};

/// Operation plus its canonical path key used for cross-version lookup.
#[derive(Debug, Clone)]
pub struct StoredOperation {
    pub operation: OpenApiOperation,
    pub path_key: String,
}

/// Everything derived from one source document; replaced atomically.
#[derive(Debug, Clone, Default)]
pub struct DocumentBundle {
    pub pages: Vec<PageRecord>,
    pub chunks: Vec<EvidenceChunk>,
    pub operations: Vec<StoredOperation>,
    pub schemas: Vec<OpenApiSchema>,
}

fn to_json<T: serde::Serialize>(value: &T) -> Result<String> {
    serde_json::to_string(value).map_err(storage)
}

impl SqliteCatalog {
    /// Atomically replaces a document and all records derived from it.
    pub async fn replace_document(&self, document: &Document, bundle: &DocumentBundle) -> Result<()> {
        self.ensure_writable()?;
        let mut tx = self.pool().begin().await.map_err(storage)?;
        sqlx::query("DELETE FROM documents WHERE source_id = ?1")
            .bind(&document.source_id)
            .execute(&mut *tx)
            .await
            .map_err(storage)?;
        sqlx::query(
            "INSERT INTO documents (source_id, document_id, kind, version, authority, precedence, title, rel_path, \
             sha256, size_bytes, status, fingerprint, extractor, page_count, metadata_json, diagnostics_json, \
             indexed_at_unix, json) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18)",
        )
        .bind(&document.source_id)
        .bind(&document.document_id)
        .bind(document.kind.as_str())
        .bind(document.version.as_str())
        .bind(document.authority.as_str())
        .bind(i64::from(document.precedence))
        .bind(&document.title)
        .bind(&document.path)
        .bind(document.sha256.as_deref())
        .bind(document.size_bytes.map(|s| s as i64))
        .bind(document.status.as_str())
        .bind(&document.fingerprint)
        .bind(document.extractor.as_deref())
        .bind(document.page_count.map(i64::from))
        .bind(to_json(&document.metadata)?)
        .bind(to_json(&document.diagnostics)?)
        .bind(document.indexed_at_unix)
        .bind(to_json(document)?)
        .execute(&mut *tx)
        .await
        .map_err(storage)?;

        for page in &bundle.pages {
            sqlx::query("INSERT INTO pages (source_id, page, status, text, char_count, sha256) VALUES (?1,?2,?3,?4,?5,?6)")
                .bind(&page.source_id)
                .bind(i64::from(page.page))
                .bind(page.status.as_str())
                .bind(&page.text)
                .bind(i64::from(page.char_count))
                .bind(&page.sha256)
                .execute(&mut *tx)
                .await
                .map_err(storage)?;
        }
        for chunk in &bundle.chunks {
            sqlx::query(
                "INSERT INTO chunks (chunk_id, source_id, version, kind, page, section, ordinal, locator, text, sha256, json) \
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
            )
            .bind(&chunk.chunk_id)
            .bind(&chunk.provenance.source_id)
            .bind(chunk.provenance.version.as_str())
            .bind(chunk.provenance.kind.as_str())
            .bind(chunk.page.map(i64::from))
            .bind(chunk.section.as_deref())
            .bind(i64::from(chunk.ordinal))
            .bind(&chunk.provenance.locator)
            .bind(&chunk.text)
            .bind(&chunk.provenance.sha256)
            .bind(to_json(chunk)?)
            .execute(&mut *tx)
            .await
            .map_err(storage)?;
        }
        for stored in &bundle.operations {
            let op = &stored.operation;
            sqlx::query(
                "INSERT INTO openapi_operations (source_id, version, method, path, path_key, operation_id, locator, sha256, json) \
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            )
            .bind(&op.provenance.source_id)
            .bind(op.version.as_str())
            .bind(&op.method)
            .bind(&op.path)
            .bind(&stored.path_key)
            .bind(op.operation_id.as_deref())
            .bind(&op.provenance.locator)
            .bind(&op.provenance.sha256)
            .bind(to_json(op)?)
            .execute(&mut *tx)
            .await
            .map_err(storage)?;
        }
        for schema in &bundle.schemas {
            sqlx::query(
                "INSERT INTO openapi_schemas (source_id, version, name, locator, sha256, refs_json, json) \
                 VALUES (?1,?2,?3,?4,?5,?6,?7)",
            )
            .bind(&schema.provenance.source_id)
            .bind(schema.version.as_str())
            .bind(&schema.name)
            .bind(&schema.provenance.locator)
            .bind(&schema.provenance.sha256)
            .bind(to_json(&schema.referenced_schemas)?)
            .bind(to_json(schema)?)
            .execute(&mut *tx)
            .await
            .map_err(storage)?;
        }
        tx.commit().await.map_err(storage)
    }

    /// Removes a document and (via cascade) all derived records.
    pub async fn delete_document(&self, source_id: &str) -> Result<()> {
        self.ensure_writable()?;
        sqlx::query("DELETE FROM documents WHERE source_id = ?1")
            .bind(source_id)
            .execute(self.pool())
            .await
            .map_err(storage)?;
        Ok(())
    }

    /// Updates status/diagnostics without touching derived records.
    pub async fn update_document_status(&self, document: &Document) -> Result<()> {
        self.ensure_writable()?;
        sqlx::query(
            "UPDATE documents SET status = ?2, diagnostics_json = ?3, json = ?4, fingerprint = ?5 WHERE source_id = ?1",
        )
        .bind(&document.source_id)
        .bind(document.status.as_str())
        .bind(to_json(&document.diagnostics)?)
        .bind(to_json(document)?)
        .bind(&document.fingerprint)
        .execute(self.pool())
        .await
        .map_err(storage)?;
        Ok(())
    }

    /// Stored status of a document for incremental indexing decisions.
    pub async fn document_state(&self, source_id: &str) -> Result<Option<(Option<String>, String, DocumentStatus)>> {
        let row: Option<(Option<String>, String, String)> =
            sqlx::query_as("SELECT sha256, fingerprint, status FROM documents WHERE source_id = ?1")
                .bind(source_id)
                .fetch_optional(self.pool())
                .await
                .map_err(storage)?;
        row.map(|(sha, fp, status)| Ok((sha, fp, status.parse().map_err(storage)?)))
            .transpose()
    }

    pub async fn source_ids(&self) -> Result<Vec<String>> {
        sqlx::query_scalar("SELECT source_id FROM documents ORDER BY source_id")
            .fetch_all(self.pool())
            .await
            .map_err(storage)
    }

    /// Replaces the full set of curated requirements atomically.
    pub async fn replace_requirements(&self, records: &[RequirementRecord]) -> Result<()> {
        self.ensure_writable()?;
        let mut tx = self.pool().begin().await.map_err(storage)?;
        sqlx::query("DELETE FROM requirements").execute(&mut *tx).await.map_err(storage)?;
        for record in records {
            let req = &record.requirement;
            sqlx::query("INSERT INTO requirements (id, version, title, sha256, json) VALUES (?1,?2,?3,?4,?5)")
                .bind(&req.id)
                .bind(req.version.as_str())
                .bind(&req.title)
                .bind(&record.mapping.sha256)
                .bind(to_json(record)?)
                .execute(&mut *tx)
                .await
                .map_err(storage)?;
            for (i, src) in req.sources.iter().enumerate() {
                sqlx::query(
                    "INSERT INTO requirement_sources (requirement_id, ordinal, source_id, locator, sha256_pin) VALUES (?1,?2,?3,?4,?5)",
                )
                .bind(&req.id)
                .bind(i as i64)
                .bind(&src.source_id)
                .bind(&src.locator)
                .bind(src.sha256.as_deref())
                .execute(&mut *tx)
                .await
                .map_err(storage)?;
            }
            for ep in &req.endpoints {
                sqlx::query("INSERT OR IGNORE INTO requirement_endpoints (requirement_id, method, path) VALUES (?1,?2,?3)")
                    .bind(&req.id)
                    .bind(&ep.method)
                    .bind(&ep.path)
                    .execute(&mut *tx)
                    .await
                    .map_err(storage)?;
            }
        }
        tx.commit().await.map_err(storage)
    }

    /// Ensures WAL contents are checkpointed so read-only openers see a compact DB.
    pub async fn checkpoint(&self) -> Result<()> {
        self.ensure_writable()?;
        sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
            .execute(self.pool())
            .await
            .map_err(storage)?;
        Ok(())
    }
}
