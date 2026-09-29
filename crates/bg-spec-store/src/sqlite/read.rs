//! `CatalogRepository` implementation.

use super::{SqliteCatalog, storage};
use async_trait::async_trait;
use bg_spec_core::{
    Result,
    domain::{Document, EvidenceChunk, OpenApiOperation, OpenApiSchema, PageRecord, RequirementRecord, SpecificationVersion},
    ports::{CatalogRepository, CatalogStats, IndexMeta},
};
use serde::de::DeserializeOwned;

fn from_json<T: DeserializeOwned>(s: &str) -> Result<T> {
    serde_json::from_str(s).map_err(storage)
}

fn decode_all<T: DeserializeOwned>(rows: Vec<String>) -> Result<Vec<T>> {
    rows.iter().map(|s| from_json(s)).collect()
}

impl SqliteCatalog {
    async fn json_rows(&self, sql: &'static str, binds: &[&str]) -> Result<Vec<String>> {
        let mut q = sqlx::query_scalar::<_, String>(sql);
        for b in binds {
            q = q.bind(*b);
        }
        q.fetch_all(self.pool()).await.map_err(storage)
    }

    async fn count(&self, sql: &'static str) -> Result<u64> {
        let n: i64 = sqlx::query_scalar(sql).fetch_one(self.pool()).await.map_err(storage)?;
        Ok(n.max(0) as u64)
    }

    /// Raw `index_meta` value.
    pub async fn meta_value(&self, key: &str) -> Result<Option<String>> {
        self.meta(key).await
    }

    async fn meta(&self, key: &str) -> Result<Option<String>> {
        sqlx::query_scalar("SELECT value FROM index_meta WHERE key = ?1")
            .bind(key)
            .fetch_optional(self.pool())
            .await
            .map_err(storage)
    }
}

#[async_trait]
impl CatalogRepository for SqliteCatalog {
    async fn list_documents(&self) -> Result<Vec<Document>> {
        let rows = self
            .json_rows("SELECT json FROM documents ORDER BY version, precedence DESC, source_id", &[])
            .await?;
        decode_all(rows)
    }

    async fn get_document(&self, source_id: &str) -> Result<Option<Document>> {
        let rows = self.json_rows("SELECT json FROM documents WHERE source_id = ?1", &[source_id]).await?;
        rows.first().map(|s| from_json(s)).transpose()
    }

    async fn get_pages(&self, source_id: &str, from: u32, to: u32) -> Result<Vec<PageRecord>> {
        let rows: Vec<(i64, String, String, i64, String)> = sqlx::query_as(
            "SELECT page, status, text, char_count, sha256 FROM pages WHERE source_id = ?1 AND page BETWEEN ?2 AND ?3 ORDER BY page",
        )
        .bind(source_id)
        .bind(i64::from(from))
        .bind(i64::from(to))
        .fetch_all(self.pool())
        .await
        .map_err(storage)?;
        rows.into_iter()
            .map(|(page, status, text, char_count, sha256)| {
                Ok(PageRecord {
                    source_id: source_id.to_owned(),
                    page: page as u32,
                    status: status.parse().map_err(storage)?,
                    text,
                    char_count: char_count as u32,
                    sha256,
                })
            })
            .collect()
    }

    async fn get_chunk(&self, chunk_id: &str) -> Result<Option<EvidenceChunk>> {
        let rows = self.json_rows("SELECT json FROM chunks WHERE chunk_id = ?1", &[chunk_id]).await?;
        rows.first().map(|s| from_json(s)).transpose()
    }

    async fn chunks_for_page(&self, source_id: &str, page: u32) -> Result<Vec<EvidenceChunk>> {
        let rows: Vec<String> =
            sqlx::query_scalar("SELECT json FROM chunks WHERE source_id = ?1 AND page = ?2 ORDER BY ordinal")
                .bind(source_id)
                .bind(i64::from(page))
                .fetch_all(self.pool())
                .await
                .map_err(storage)?;
        decode_all(rows)
    }

    async fn chunks_for_section(&self, source_id: &str, section: &str) -> Result<Vec<EvidenceChunk>> {
        let like = format!("{}%", section.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_"));
        let rows: Vec<(String, Option<String>)> = sqlx::query_as(
            "SELECT json, section FROM chunks WHERE source_id = ?1 AND section LIKE ?2 ESCAPE '\\' ORDER BY page, ordinal",
        )
        .bind(source_id)
        .bind(like)
        .fetch_all(self.pool())
        .await
        .map_err(storage)?;
        rows.into_iter()
            .filter(|(_, s)| {
                s.as_deref().is_some_and(|s| {
                    s == section
                        || s.strip_prefix(section)
                            .is_some_and(|rest| rest.starts_with(['.', ' ', '/']))
                })
            })
            .map(|(json, _)| from_json(&json))
            .collect()
    }

    async fn find_operations(
        &self,
        version: &SpecificationVersion,
        method: &str,
        path_key: &str,
    ) -> Result<Vec<OpenApiOperation>> {
        let rows = self
            .json_rows(
                "SELECT o.json FROM openapi_operations o JOIN documents d ON d.source_id = o.source_id \
                 WHERE o.version = ?1 AND o.method = ?2 AND o.path_key = ?3 ORDER BY d.precedence DESC, o.source_id",
                &[version.as_str(), method, path_key],
            )
            .await?;
        decode_all(rows)
    }

    async fn get_source_operation(&self, source_id: &str, method: &str, path: &str) -> Result<Option<OpenApiOperation>> {
        let rows = self
            .json_rows(
                "SELECT json FROM openapi_operations WHERE source_id = ?1 AND method = ?2 AND path = ?3",
                &[source_id, method, path],
            )
            .await?;
        rows.first().map(|s| from_json(s)).transpose()
    }

    async fn source_operations_by_path(&self, source_id: &str, path: &str) -> Result<Vec<OpenApiOperation>> {
        let rows = self
            .json_rows(
                "SELECT json FROM openapi_operations WHERE source_id = ?1 AND path = ?2 ORDER BY method",
                &[source_id, path],
            )
            .await?;
        decode_all(rows)
    }

    async fn find_schemas(&self, version: &SpecificationVersion, name: &str) -> Result<Vec<OpenApiSchema>> {
        let rows = self
            .json_rows(
                "SELECT s.json FROM openapi_schemas s JOIN documents d ON d.source_id = s.source_id \
                 WHERE s.version = ?1 AND s.name = ?2 ORDER BY d.precedence DESC, s.source_id",
                &[version.as_str(), name],
            )
            .await?;
        decode_all(rows)
    }

    async fn get_source_schema(&self, source_id: &str, name: &str) -> Result<Option<OpenApiSchema>> {
        let rows = self
            .json_rows("SELECT json FROM openapi_schemas WHERE source_id = ?1 AND name = ?2", &[source_id, name])
            .await?;
        rows.first().map(|s| from_json(s)).transpose()
    }

    async fn list_requirements(&self) -> Result<Vec<RequirementRecord>> {
        let rows = self.json_rows("SELECT json FROM requirements ORDER BY id", &[]).await?;
        decode_all(rows)
    }

    async fn get_requirement(&self, id: &str) -> Result<Option<RequirementRecord>> {
        let rows = self.json_rows("SELECT json FROM requirements WHERE id = ?1", &[id]).await?;
        rows.first().map(|s| from_json(s)).transpose()
    }

    async fn stats(&self) -> Result<CatalogStats> {
        Ok(CatalogStats {
            documents: self.count("SELECT COUNT(*) FROM documents").await?,
            pages: self.count("SELECT COUNT(*) FROM pages").await?,
            ocr_required_pages: self.count("SELECT COUNT(*) FROM pages WHERE status = 'ocr_required'").await?,
            chunks: self.count("SELECT COUNT(*) FROM chunks").await?,
            operations: self.count("SELECT COUNT(*) FROM openapi_operations").await?,
            schemas: self.count("SELECT COUNT(*) FROM openapi_schemas").await?,
            requirements: self.count("SELECT COUNT(*) FROM requirements").await?,
        })
    }

    async fn index_meta(&self) -> Result<IndexMeta> {
        Ok(IndexMeta {
            schema_version: self.meta("schema_version").await?.and_then(|v| v.parse().ok()),
            last_indexed_at_unix: self.meta("last_indexed_at_unix").await?.and_then(|v| v.parse().ok()),
            index_generation: self.meta("index_generation").await?.and_then(|v| v.parse().ok()),
            requirements_file_sha256: self.meta("requirements_file_sha256").await?,
        })
    }
}
