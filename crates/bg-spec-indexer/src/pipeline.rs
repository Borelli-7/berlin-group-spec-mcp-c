//! Incremental indexing pipeline.
//!
//! ```text
//! manifest entry → confined path → sha256 → unchanged? skip : extract → normalize → chunk
//!   → SQLite (one transaction per document) + Tantivy (delete by source, add) → requirements
//! ```

use crate::{
    chunk::{ChunkSettings, PageText, chunk_pages},
    discover::{CorpusRoot, orphan_files},
    openapi::{
        normalize, operation_identifiers, operation_search_text, parse_document,
        schema_identifiers, schema_search_text,
    },
    pdf::{ExtractedPage, PdfExtractor, PdfOxideExtractor},
};
use bg_spec_core::{
    CoreError, Result,
    config::Config,
    domain::{
        Diagnostic, Document, DocumentKind, DocumentStatus, EvidenceChunk, PageRecord, PageStatus,
        Provenance, RecordType, Severity,
    },
    hash::{sha256_hex, sha256_reader},
    manifest::{Manifest, ManifestSource},
    openapi_path,
    requirements::RequirementsFile,
    timing::Timer,
};
use bg_spec_store::{
    DocumentBundle, SearchDocument, SqliteCatalog, StoredOperation, TantivyWriter,
};
use serde::Serialize;
use serde_json::json;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::{Instant, SystemTime, UNIX_EPOCH},
};
use tracing::{info, warn};

/// Bumped when extraction/normalization output changes, forcing re-indexing.
pub const INDEXER_FORMAT_VERSION: u32 = 2;
const META_IN_PROGRESS: &str = "index_in_progress";

#[derive(Debug, Clone, Copy, Default)]
pub struct IndexOptions {
    /// Re-index every document regardless of hashes.
    pub force: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IndexAction {
    Indexed,
    Skipped,
    Missing,
    Failed,
    Removed,
}

#[derive(Debug, Clone, Serialize)]
pub struct DocumentReport {
    pub source_id: String,
    pub action: IndexAction,
    pub status: Option<DocumentStatus>,
    pub sha256: Option<String>,
    pub records: usize,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, Serialize)]
pub struct IndexReport {
    pub documents: Vec<DocumentReport>,
    pub requirements: usize,
    pub requirements_diagnostics: Vec<Diagnostic>,
    pub orphan_files: Vec<String>,
    pub full_rebuild: bool,
    pub index_generation: u64,
    pub duration_ms: u128,
}

impl IndexReport {
    pub fn count(&self, action: IndexAction) -> usize {
        self.documents.iter().filter(|d| d.action == action).count()
    }
}

/// Orchestrates indexing of a corpus described by a [`Config`].
pub struct Indexer {
    config: Config,
    extractor: Arc<dyn PdfExtractor>,
}

struct Processed {
    document: Document,
    bundle: DocumentBundle,
    search_docs: Vec<SearchDocument>,
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn join_err(e: tokio::task::JoinError) -> CoreError {
    CoreError::Integrity(format!("indexing task failed: {e}"))
}

impl Indexer {
    pub fn new(config: Config) -> Self {
        Self::with_extractor(config, Arc::new(PdfOxideExtractor))
    }

    /// Injects an alternative PDF extractor (e.g. a future Pdfium/OCR implementation).
    pub fn with_extractor(config: Config, extractor: Arc<dyn PdfExtractor>) -> Self {
        Self { config, extractor }
    }

    pub fn load_manifest(config: &Config) -> Result<Manifest> {
        let path = config.manifest_path();
        let raw = std::fs::read_to_string(&path)
            .map_err(|e| CoreError::io(path.display().to_string(), e))?;
        Manifest::parse(&raw)
    }

    fn fingerprint(&self, source: &ManifestSource) -> String {
        let settings = json!({
            "format": INDEXER_FORMAT_VERSION,
            "source": source,
            "chunking": [self.config.chunking.max_chars, self.config.chunking.overlap_chars],
            "min_chars_per_page": self.config.pdf.min_chars_per_page,
            "path_prefix": self.config.versions.prefix_for(&source.version),
            "extractor": self.extractor.name(),
        });
        sha256_hex(settings.to_string())
    }

    pub async fn run(&self, options: IndexOptions) -> Result<IndexReport> {
        let _timer = Timer::start("indexer.run");
        let started = Instant::now();
        let manifest = Self::load_manifest(&self.config)?;
        let root = CorpusRoot::open(&self.config.corpus_root)?;
        let catalog = SqliteCatalog::open_read_write(&self.config.catalog_path()).await?;
        let tantivy_dir = self.config.tantivy_dir();
        let mut writer =
            tokio::task::spawn_blocking(move || TantivyWriter::open_or_create(&tantivy_dir))
                .await
                .map_err(join_err)??;

        let meta = bg_spec_core::ports::CatalogRepository::index_meta(&catalog).await?;
        let interrupted = sqlx_meta_flag(&catalog).await?;
        let full_rebuild = options.force || writer.was_rebuilt() || interrupted;
        if interrupted {
            warn!("previous indexing run did not complete; performing a full rebuild");
        }
        if full_rebuild {
            writer.delete_all()?;
        }
        catalog.set_meta(META_IN_PROGRESS, "1").await?;

        let mut reports = Vec::new();
        // Documents no longer present in the manifest.
        for sid in catalog.source_ids().await? {
            if manifest.source(&sid).is_none() {
                catalog.delete_document(&sid).await?;
                writer.delete_source(&sid);
                reports.push(DocumentReport {
                    source_id: sid,
                    action: IndexAction::Removed,
                    status: None,
                    sha256: None,
                    records: 0,
                    diagnostics: vec![],
                });
            }
        }

        for source in &manifest.sources {
            let _doc_timer = Timer::start("indexer.index_source");
            let report = self
                .index_source(&root, source, &catalog, &writer, full_rebuild)
                .await?;
            info!(source_id = %report.source_id, action = ?report.action, records = report.records, "document processed");
            reports.push(report);
        }

        let generation = meta.index_generation.unwrap_or(0) + 1;
        let commit_timer = Timer::start("indexer.search_commit");
        tokio::task::spawn_blocking(move || writer.commit())
            .await
            .map_err(join_err)??;
        drop(commit_timer);

        let requirements_timer = Timer::start("indexer.requirements");
        let (requirements, requirements_diagnostics, req_sha) =
            self.index_requirements(&manifest, &catalog).await?;
        drop(requirements_timer);
        catalog
            .set_meta("last_indexed_at_unix", &now_unix().to_string())
            .await?;
        catalog
            .set_meta("index_generation", &generation.to_string())
            .await?;
        catalog
            .set_meta("requirements_file_sha256", req_sha.as_deref().unwrap_or(""))
            .await?;
        catalog
            .set_meta(
                "indexer_format_version",
                &INDEXER_FORMAT_VERSION.to_string(),
            )
            .await?;
        catalog.set_meta(META_IN_PROGRESS, "0").await?;
        catalog.checkpoint().await?;
        catalog.close().await;

        Ok(IndexReport {
            documents: reports,
            requirements,
            requirements_diagnostics,
            orphan_files: orphan_files(&root, &manifest),
            full_rebuild,
            index_generation: generation,
            duration_ms: started.elapsed().as_millis(),
        })
    }

    fn base_document(
        &self,
        source: &ManifestSource,
        sha256: Option<String>,
        size: Option<u64>,
        status: DocumentStatus,
    ) -> Document {
        Document {
            document_id: Document::document_id_for(&source.id, sha256.as_deref()),
            source_id: source.id.clone(),
            kind: source.kind,
            version: source.version.clone(),
            authority: source.authority,
            precedence: source.precedence,
            title: source.display_title().to_owned(),
            path: source.path.clone(),
            sha256,
            size_bytes: size,
            status,
            fingerprint: self.fingerprint(source),
            extractor: None,
            page_count: None,
            metadata: json!({"description": source.description, "tags": source.tags}),
            diagnostics: Vec::new(),
            indexed_at_unix: now_unix(),
        }
    }

    async fn store_empty(
        &self,
        catalog: &SqliteCatalog,
        writer: &TantivyWriter,
        document: Document,
        action: IndexAction,
    ) -> Result<DocumentReport> {
        catalog
            .replace_document(&document, &DocumentBundle::default())
            .await?;
        writer.delete_source(&document.source_id);
        Ok(DocumentReport {
            source_id: document.source_id.clone(),
            action,
            status: Some(document.status),
            sha256: document.sha256.clone(),
            records: 0,
            diagnostics: document.diagnostics,
        })
    }

    async fn index_source(
        &self,
        root: &CorpusRoot,
        source: &ManifestSource,
        catalog: &SqliteCatalog,
        writer: &TantivyWriter,
        full_rebuild: bool,
    ) -> Result<DocumentReport> {
        let path = match root.resolve(&source.path) {
            Ok(Some(p)) => p,
            Ok(None) => {
                let mut doc = self.base_document(source, None, None, DocumentStatus::Missing);
                doc.diagnostics.push(Diagnostic::new(
                    Severity::Error,
                    "file_missing",
                    format!(
                        "file '{}' declared in the manifest does not exist",
                        source.path
                    ),
                ));
                return self
                    .store_empty(catalog, writer, doc, IndexAction::Missing)
                    .await;
            }
            Err(e) => {
                let mut doc = self.base_document(source, None, None, DocumentStatus::Failed);
                doc.diagnostics.push(Diagnostic::new(
                    Severity::Error,
                    "path_rejected",
                    e.to_string(),
                ));
                return self
                    .store_empty(catalog, writer, doc, IndexAction::Failed)
                    .await;
            }
        };

        let hash_path = path.clone();
        let (sha, size) = tokio::task::spawn_blocking(move || -> Result<(String, u64)> {
            let file = std::fs::File::open(&hash_path)
                .map_err(|e| CoreError::io(hash_path.display().to_string(), e))?;
            let size = file.metadata().map(|m| m.len()).unwrap_or(0);
            let sha = sha256_reader(std::io::BufReader::new(file))
                .map_err(|e| CoreError::io(hash_path.display().to_string(), e))?;
            Ok((sha, size))
        })
        .await
        .map_err(join_err)??;

        let fingerprint = self.fingerprint(source);
        if !full_rebuild
            && let Some((Some(old_sha), old_fp, status)) =
                catalog.document_state(&source.id).await?
            && old_sha == sha
            && old_fp == fingerprint
            && matches!(status, DocumentStatus::Indexed | DocumentStatus::Partial)
        {
            return Ok(DocumentReport {
                source_id: source.id.clone(),
                action: IndexAction::Skipped,
                status: Some(status),
                sha256: Some(sha),
                records: 0,
                diagnostics: vec![],
            });
        }

        let document = self.base_document(source, Some(sha), Some(size), DocumentStatus::Indexed);
        let processed = match self
            .process(document.clone(), path, source.blank_pages.clone())
            .await
        {
            Ok(p) => p,
            Err(e) => {
                let mut doc = document;
                doc.status = DocumentStatus::Failed;
                doc.diagnostics.push(Diagnostic::new(
                    Severity::Error,
                    "processing_failed",
                    e.to_string(),
                ));
                return self
                    .store_empty(catalog, writer, doc, IndexAction::Failed)
                    .await;
            }
        };
        catalog
            .replace_document(&processed.document, &processed.bundle)
            .await?;
        writer.delete_source(&source.id);
        for d in &processed.search_docs {
            writer.add(d)?;
        }
        Ok(DocumentReport {
            source_id: source.id.clone(),
            action: IndexAction::Indexed,
            status: Some(processed.document.status),
            sha256: processed.document.sha256.clone(),
            records: processed.search_docs.len(),
            diagnostics: processed.document.diagnostics,
        })
    }

    async fn process(
        &self,
        document: Document,
        path: PathBuf,
        blank_pages: Vec<u32>,
    ) -> Result<Processed> {
        let settings = ChunkSettings {
            max_chars: self.config.chunking.max_chars,
            overlap_chars: self.config.chunking.overlap_chars,
        };
        let min_chars = self.config.pdf.min_chars_per_page;
        let prefix = self
            .config
            .versions
            .prefix_for(&document.version)
            .map(str::to_owned);
        let extractor = Arc::clone(&self.extractor);
        tokio::task::spawn_blocking(move || match document.kind {
            DocumentKind::Pdf => {
                let extracted = extractor.extract(&path)?;
                let mut document = document;
                document.extractor = Some(extractor.name().to_owned());
                Ok(process_paged(
                    document,
                    extracted.pages,
                    settings,
                    min_chars,
                    &blank_pages,
                ))
            }
            DocumentKind::Text => {
                let raw = read_text(&path)?;
                let pages = raw
                    .split('\u{c}')
                    .enumerate()
                    .map(|(i, text)| ExtractedPage {
                        number: i as u32 + 1,
                        text: text.to_owned(),
                        error: None,
                    })
                    .collect();
                let mut document = document;
                document.extractor = Some("text".into());
                Ok(process_paged(
                    document,
                    pages,
                    settings,
                    min_chars,
                    &blank_pages,
                ))
            }
            DocumentKind::Openapi => process_openapi(document, &path, prefix.as_deref()),
        })
        .await
        .map_err(join_err)?
    }

    async fn index_requirements(
        &self,
        manifest: &Manifest,
        catalog: &SqliteCatalog,
    ) -> Result<(usize, Vec<Diagnostic>, Option<String>)> {
        let path = self.config.requirements_path();
        let mut diagnostics = Vec::new();
        if !path.is_file() {
            diagnostics.push(Diagnostic::new(
                Severity::Warning,
                "requirements_missing",
                format!(
                    "requirements mapping {} not found; no curated requirements available",
                    path.display()
                ),
            ));
            catalog.replace_requirements(&[]).await?;
            return Ok((0, diagnostics, None));
        }
        let raw = read_text(&path)?;
        let sha = sha256_hex(raw.as_bytes());
        let file = RequirementsFile::parse(&raw)?;
        for r in &file.requirements {
            for s in &r.sources {
                if manifest.source(&s.source_id).is_none() {
                    diagnostics.push(
                        Diagnostic::new(
                            Severity::Warning,
                            "dangling_source",
                            format!(
                                "requirement {} cites unknown source '{}'",
                                r.id, s.source_id
                            ),
                        )
                        .at(format!("requirement:{}", r.id)),
                    );
                }
            }
        }
        let display = relative_display(&path, &self.config.corpus_root);
        let records = file.into_records(&display, &sha)?;
        catalog.replace_requirements(&records).await?;
        Ok((records.len(), diagnostics, Some(sha)))
    }
}

async fn sqlx_meta_flag(catalog: &SqliteCatalog) -> Result<bool> {
    Ok(catalog.meta_value(META_IN_PROGRESS).await?.as_deref() == Some("1"))
}

fn relative_display(path: &Path, base: &Path) -> String {
    let (p, b) = (path.canonicalize().ok(), base.canonicalize().ok());
    match (p, b) {
        (Some(p), Some(b)) => p
            .strip_prefix(&b)
            .map(|r| r.to_string_lossy().replace('\\', "/"))
            .ok(),
        _ => None,
    }
    .unwrap_or_else(|| {
        path.file_name()
            .map(|f| f.to_string_lossy().into_owned())
            .unwrap_or_default()
    })
}

fn read_text(path: &Path) -> Result<String> {
    std::fs::read_to_string(path).map_err(|e| CoreError::io(path.display().to_string(), e))
}

fn process_paged(
    mut document: Document,
    pages: Vec<ExtractedPage>,
    settings: ChunkSettings,
    min_chars: usize,
    blank_pages: &[u32],
) -> Processed {
    let mut records = Vec::with_capacity(pages.len());
    for page in pages {
        let text = page
            .text
            .lines()
            .map(str::trim_end)
            .collect::<Vec<_>>()
            .join("\n")
            .trim()
            .to_owned();
        let significant = text.chars().filter(|c| !c.is_whitespace()).count();
        let declared_blank = blank_pages.contains(&page.number);
        let status = if page.error.is_some() {
            PageStatus::ExtractionFailed
        } else if significant < min_chars && declared_blank {
            PageStatus::Blank
        } else if significant < min_chars {
            PageStatus::OcrRequired
        } else {
            PageStatus::Extracted
        };
        match (&page.error, status) {
            (Some(e), _) => document.diagnostics.push(
                Diagnostic::new(Severity::Warning, "page_extraction_failed", format!("page {}: {e}", page.number))
                    .at(format!("page:{}", page.number)),
            ),
            (None, PageStatus::Extracted) if declared_blank => document.diagnostics.push(
                Diagnostic::new(
                    Severity::Warning,
                    "blank_page_has_text",
                    format!(
                        "page {} is declared in blank_pages but has {significant} extractable characters; indexed as text",
                        page.number
                    ),
                )
                .at(format!("page:{}", page.number)),
            ),
            (None, PageStatus::OcrRequired) => document.diagnostics.push(
                Diagnostic::new(
                    Severity::Warning,
                    "ocr_required",
                    format!(
                        "page {} has {significant} extractable characters (< {min_chars}); likely scanned, OCR required",
                        page.number
                    ),
                )
                .at(format!("page:{}", page.number)),
            ),
            _ => {}
        }
        records.push(PageRecord {
            source_id: document.source_id.clone(),
            page: page.number,
            status,
            char_count: text.chars().count() as u32,
            sha256: sha256_hex(text.as_bytes()),
            text,
        });
    }

    let extracted: Vec<PageText<'_>> = records
        .iter()
        .filter(|p| p.status == PageStatus::Extracted)
        .map(|p| PageText {
            number: p.page,
            text: &p.text,
        })
        .collect();
    let raw_chunks = chunk_pages(&extracted, settings);
    let extracted_count = extracted.len();
    let total = records.len();
    let blank = records
        .iter()
        .filter(|p| p.status == PageStatus::Blank)
        .count();
    for &p in blank_pages.iter().filter(|&&p| p as usize > total) {
        document.diagnostics.push(
            Diagnostic::new(
                Severity::Warning,
                "blank_page_out_of_range",
                format!("blank_pages entry {p} exceeds the document's {total} page(s)"),
            )
            .at(format!("page:{p}")),
        );
    }
    document.page_count = Some(total as u32);
    document.status = if extracted_count + blank == total && extracted_count > 0 {
        DocumentStatus::Indexed
    } else {
        DocumentStatus::Partial
    };
    if extracted_count == 0 {
        document.diagnostics.push(Diagnostic::new(
            Severity::Error,
            "no_extractable_text",
            "no page contains extractable text; the document provides no searchable evidence (OCR required)",
        ));
    }

    let chunks: Vec<EvidenceChunk> = raw_chunks
        .into_iter()
        .map(|c| {
            let chunk_id = format!("{}:p{}:c{}", document.source_id, c.page, c.ordinal);
            let provenance = Provenance::for_document(
                &document,
                format!("chunk:{chunk_id}"),
                sha256_hex(c.text.as_bytes()),
            );
            EvidenceChunk {
                chunk_id,
                page: Some(c.page),
                section: c.section,
                ordinal: c.ordinal,
                text: c.text,
                provenance,
            }
        })
        .collect();
    let search_docs = chunks
        .iter()
        .map(|c| SearchDocument {
            record_id: c.chunk_id.clone(),
            record_type: RecordType::Chunk,
            source_id: document.source_id.clone(),
            document_id: document.document_id.clone(),
            version: document.version.clone(),
            kind: document.kind,
            authority: document.authority,
            locator: c.provenance.locator.clone(),
            page: c.page,
            title: match &c.section {
                Some(s) => format!("{} — {s}", document.title),
                None => document.title.clone(),
            },
            content: c.text.clone(),
            sha256: c.provenance.sha256.clone(),
            identifiers: bg_spec_core::identifiers::from_text(&c.text),
        })
        .collect();
    let ocr = records
        .iter()
        .filter(|p| p.status == PageStatus::OcrRequired)
        .count();
    if let Some(m) = document.metadata.as_object_mut() {
        m.insert("pages".into(), json!(total));
        m.insert("extracted_pages".into(), json!(extracted_count));
        m.insert("ocr_required_pages".into(), json!(ocr));
        m.insert("blank_pages".into(), json!(blank));
        m.insert("chunks".into(), json!(chunks.len()));
    }
    Processed {
        document,
        bundle: DocumentBundle {
            pages: records,
            chunks,
            ..Default::default()
        },
        search_docs,
    }
}

fn process_openapi(mut document: Document, path: &Path, prefix: Option<&str>) -> Result<Processed> {
    let raw = read_text(path)?;
    let name = path
        .file_name()
        .map(|f| f.to_string_lossy().into_owned())
        .unwrap_or_default();
    let root = parse_document(&raw, &name)?;
    let api = normalize(&root, &document)?;
    document.diagnostics.extend(api.diagnostics);
    if document
        .diagnostics
        .iter()
        .any(|d| d.severity != Severity::Info)
    {
        document.status = DocumentStatus::Partial;
    }
    if let (Some(m), Some(api_meta)) = (document.metadata.as_object_mut(), api.metadata.as_object())
    {
        m.extend(api_meta.clone());
    }

    let mut search_docs = Vec::with_capacity(api.operations.len() + api.schemas.len());
    let base = |record_id: String,
                record_type,
                locator: String,
                title: String,
                content: String,
                sha256: String,
                identifiers: Vec<String>| SearchDocument {
        record_id,
        record_type,
        source_id: document.source_id.clone(),
        document_id: document.document_id.clone(),
        version: document.version.clone(),
        kind: document.kind,
        authority: document.authority,
        locator,
        page: None,
        title,
        content,
        sha256,
        identifiers,
    };
    for op in &api.operations {
        let (title, content) = operation_search_text(op);
        search_docs.push(base(
            format!("{}:{}", document.source_id, op.provenance.locator),
            RecordType::Operation,
            op.provenance.locator.clone(),
            title,
            content,
            op.provenance.sha256.clone(),
            operation_identifiers(op),
        ));
    }
    for schema in &api.schemas {
        let (title, content) = schema_search_text(schema);
        search_docs.push(base(
            format!("{}:{}", document.source_id, schema.provenance.locator),
            RecordType::Schema,
            schema.provenance.locator.clone(),
            title,
            content,
            schema.provenance.sha256.clone(),
            schema_identifiers(schema),
        ));
    }
    let operations = api
        .operations
        .into_iter()
        .map(|operation| StoredOperation {
            path_key: openapi_path::path_key(&operation.path, prefix),
            operation,
        })
        .collect();
    Ok(Processed {
        document,
        bundle: DocumentBundle {
            operations,
            schemas: api.schemas,
            ..Default::default()
        },
        search_docs,
    })
}
