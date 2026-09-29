//! Tantivy full-text index (BM25) over chunks, OpenAPI operations and schemas.

use async_trait::async_trait;
use bg_spec_core::{
    CoreError, Result,
    domain::{
        DocumentAuthority, DocumentKind, EvidenceClass, RecordType, SearchQuery, SearchResult,
        SpecificationVersion,
    },
    ports::SearchRepository,
};
use std::{
    cmp::Ordering,
    path::{Path, PathBuf},
};
use tantivy::{
    Index, IndexReader, IndexWriter, ReloadPolicy, TantivyDocument, Term,
    collector::TopDocs,
    query::{BooleanQuery, Occur, Query, QueryParser, TermQuery},
    schema::{
        Field, INDEXED, IndexRecordOption, STORED, STRING, Schema, TextFieldIndexing, TextOptions,
        Value,
    },
    snippet::SnippetGenerator,
};

/// Bumped whenever the Tantivy schema changes; mismatches force a rebuild by the indexer.
pub const SEARCH_SCHEMA_VERSION: u32 = 1;
const MARKER_FILE: &str = "bg-spec-search.version";
const TOKENIZER: &str = "en_stem";
const SNIPPET_CHARS: usize = 400;
const WRITER_HEAP_BYTES: usize = 64 * 1024 * 1024;

fn search_err(e: impl std::fmt::Display) -> CoreError {
    CoreError::Search(e.to_string())
}

#[derive(Clone, Copy)]
struct Fields {
    record_id: Field,
    record_type: Field,
    source_id: Field,
    document_id: Field,
    version: Field,
    kind: Field,
    authority: Field,
    locator: Field,
    page: Field,
    title: Field,
    content: Field,
    sha256: Field,
}

fn build_schema() -> (Schema, Fields) {
    let mut b = Schema::builder();
    let text = TextOptions::default()
        .set_indexing_options(
            TextFieldIndexing::default()
                .set_tokenizer(TOKENIZER)
                .set_index_option(IndexRecordOption::WithFreqsAndPositions),
        )
        .set_stored();
    let fields = Fields {
        record_id: b.add_text_field("record_id", STRING | STORED),
        record_type: b.add_text_field("record_type", STRING | STORED),
        source_id: b.add_text_field("source_id", STRING | STORED),
        document_id: b.add_text_field("document_id", STRING | STORED),
        version: b.add_text_field("version", STRING | STORED),
        kind: b.add_text_field("kind", STRING | STORED),
        authority: b.add_text_field("authority", STRING | STORED),
        locator: b.add_text_field("locator", STRING | STORED),
        page: b.add_u64_field("page", INDEXED | STORED),
        title: b.add_text_field("title", text.clone()),
        content: b.add_text_field("content", text),
        sha256: b.add_text_field("sha256", STRING | STORED),
    };
    (b.build(), fields)
}

fn fields_of(schema: &Schema) -> Result<Fields> {
    let f = |name: &str| schema.get_field(name).map_err(search_err);
    Ok(Fields {
        record_id: f("record_id")?,
        record_type: f("record_type")?,
        source_id: f("source_id")?,
        document_id: f("document_id")?,
        version: f("version")?,
        kind: f("kind")?,
        authority: f("authority")?,
        locator: f("locator")?,
        page: f("page")?,
        title: f("title")?,
        content: f("content")?,
        sha256: f("sha256")?,
    })
}

fn marker_ok(dir: &Path) -> bool {
    std::fs::read_to_string(dir.join(MARKER_FILE))
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok())
        == Some(SEARCH_SCHEMA_VERSION)
}

/// A record to be indexed for full-text retrieval.
#[derive(Debug, Clone)]
pub struct SearchDocument {
    pub record_id: String,
    pub record_type: RecordType,
    pub source_id: String,
    pub document_id: String,
    pub version: SpecificationVersion,
    pub kind: DocumentKind,
    pub authority: DocumentAuthority,
    pub locator: String,
    pub page: Option<u32>,
    pub title: String,
    pub content: String,
    pub sha256: String,
}

/// Index writer used by the indexer. Holds the Tantivy writer lock while alive.
pub struct TantivyWriter {
    writer: IndexWriter<TantivyDocument>,
    fields: Fields,
    rebuilt: bool,
}

impl TantivyWriter {
    /// Opens the index for writing, (re)creating it if absent or schema-incompatible.
    pub fn open_or_create(dir: &Path) -> Result<Self> {
        let has_index = dir.join("meta.json").is_file();
        let (index, rebuilt) = if has_index && marker_ok(dir) {
            (Index::open_in_dir(dir).map_err(search_err)?, false)
        } else {
            if dir.exists() {
                std::fs::remove_dir_all(dir)
                    .map_err(|e| CoreError::io(dir.display().to_string(), e))?;
            }
            std::fs::create_dir_all(dir)
                .map_err(|e| CoreError::io(dir.display().to_string(), e))?;
            let (schema, _) = build_schema();
            let index = Index::create_in_dir(dir, schema).map_err(search_err)?;
            std::fs::write(dir.join(MARKER_FILE), SEARCH_SCHEMA_VERSION.to_string())
                .map_err(|e| CoreError::io(dir.display().to_string(), e))?;
            (index, true)
        };
        let fields = fields_of(&index.schema())?;
        let writer = index
            .writer_with_num_threads(1, WRITER_HEAP_BYTES)
            .map_err(search_err)?;
        Ok(Self {
            writer,
            fields,
            rebuilt,
        })
    }

    /// True if the index was (re)created, meaning every document must be re-added.
    pub fn was_rebuilt(&self) -> bool {
        self.rebuilt
    }

    pub fn delete_source(&self, source_id: &str) {
        self.writer
            .delete_term(Term::from_field_text(self.fields.source_id, source_id));
    }

    pub fn delete_all(&mut self) -> Result<()> {
        self.writer.delete_all_documents().map_err(search_err)?;
        Ok(())
    }

    pub fn add(&self, doc: &SearchDocument) -> Result<()> {
        let f = &self.fields;
        let mut d = TantivyDocument::default();
        d.add_text(f.record_id, &doc.record_id);
        d.add_text(f.record_type, doc.record_type.as_str());
        d.add_text(f.source_id, &doc.source_id);
        d.add_text(f.document_id, &doc.document_id);
        d.add_text(f.version, doc.version.as_str());
        d.add_text(f.kind, doc.kind.as_str());
        d.add_text(f.authority, doc.authority.as_str());
        d.add_text(f.locator, &doc.locator);
        if let Some(page) = doc.page {
            d.add_u64(f.page, u64::from(page));
        }
        d.add_text(f.title, &doc.title);
        d.add_text(f.content, &doc.content);
        d.add_text(f.sha256, &doc.sha256);
        self.writer.add_document(d).map_err(search_err)?;
        Ok(())
    }

    /// Commits pending changes and waits for merges; returns the commit opstamp.
    pub fn commit(mut self) -> Result<u64> {
        let stamp = self.writer.commit().map_err(search_err)?;
        self.writer.wait_merging_threads().map_err(search_err)?;
        Ok(stamp)
    }
}

/// Read-only search repository used by the MCP runtime.
#[derive(Clone)]
pub struct TantivySearch {
    index: Index,
    reader: IndexReader,
    fields: Fields,
    dir: PathBuf,
}

impl TantivySearch {
    pub fn open(dir: &Path) -> Result<Self> {
        if !dir.join("meta.json").is_file() {
            return Err(CoreError::Config(format!(
                "search index not found at {}; run `bg-spec index --config <config.toml>` first",
                dir.display()
            )));
        }
        if !marker_ok(dir) {
            return Err(CoreError::Config(format!(
                "search index at {} has an incompatible schema; run `bg-spec index --force`",
                dir.display()
            )));
        }
        let index = Index::open_in_dir(dir).map_err(search_err)?;
        let fields = fields_of(&index.schema())?;
        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::OnCommitWithDelay)
            .try_into()
            .map_err(search_err)?;
        Ok(Self {
            index,
            reader,
            fields,
            dir: dir.to_path_buf(),
        })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn build_query(&self, q: &SearchQuery) -> Result<Box<dyn Query>> {
        let f = &self.fields;
        let mut parser = QueryParser::for_index(&self.index, vec![f.title, f.content]);
        parser.set_field_boost(f.title, 2.0);
        let (text_query, _errors) = parser.parse_query_lenient(&q.text);
        let mut clauses: Vec<(Occur, Box<dyn Query>)> = vec![(Occur::Must, text_query)];
        let term = |field: Field, value: &str| -> Box<dyn Query> {
            Box::new(TermQuery::new(
                Term::from_field_text(field, value),
                IndexRecordOption::Basic,
            ))
        };
        if let Some(v) = &q.version {
            clauses.push((Occur::Must, term(f.version, v.as_str())));
        }
        if let Some(s) = &q.source_id {
            clauses.push((Occur::Must, term(f.source_id, s)));
        }
        if !q.kinds.is_empty() {
            let kinds = q
                .kinds
                .iter()
                .map(|k| (Occur::Should, term(f.kind, k.as_str())))
                .collect();
            clauses.push((Occur::Must, Box::new(BooleanQuery::new(kinds))));
        }
        Ok(Box::new(BooleanQuery::new(clauses)))
    }

    fn search_blocking(&self, q: &SearchQuery) -> Result<Vec<SearchResult>> {
        if q.limit == 0 || q.text.trim().is_empty() {
            return Ok(Vec::new());
        }
        let query = self.build_query(q)?;
        let searcher = self.reader.searcher();
        // Over-fetch so ties at the cut-off are resolved deterministically by record id.
        let fetch = q.limit.saturating_add(16);
        let top = searcher
            .search(&query, &TopDocs::with_limit(fetch).order_by_score())
            .map_err(search_err)?;
        let mut snippets = SnippetGenerator::create(&searcher, &*query, self.fields.content)
            .map_err(search_err)?;
        snippets.set_max_num_chars(SNIPPET_CHARS);

        let f = &self.fields;
        let mut results = Vec::with_capacity(top.len());
        for (score, addr) in top {
            let doc: TantivyDocument = searcher.doc(addr).map_err(search_err)?;
            let text = |field: Field| -> String {
                doc.get_first(field)
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_owned()
            };
            let snippet = snippets.snippet_from_doc(&doc);
            let evidence = if snippet.fragment().trim().is_empty() {
                let content = text(f.content);
                let end = content.floor_char_boundary(SNIPPET_CHARS.min(content.len()));
                content[..end].to_owned()
            } else {
                snippet.fragment().to_owned()
            };
            let record_type = RecordType::parse(&text(f.record_type))
                .ok_or_else(|| CoreError::Search("corrupt record_type in index".into()))?;
            results.push(SearchResult {
                record_id: text(f.record_id),
                record_type,
                source_id: text(f.source_id),
                document_id: text(f.document_id),
                version: SpecificationVersion::new(text(f.version))?,
                kind: text(f.kind).parse().map_err(search_err)?,
                authority: text(f.authority).parse().map_err(search_err)?,
                title: text(f.title),
                locator: text(f.locator),
                page: doc
                    .get_first(f.page)
                    .and_then(|v| v.as_u64())
                    .map(|p| p as u32),
                evidence,
                relevance: (score * 1000.0).round() / 1000.0,
                sha256: text(f.sha256),
                classification: EvidenceClass::DiscoveredEvidence,
            });
        }
        results.sort_by(|a, b| {
            b.relevance
                .partial_cmp(&a.relevance)
                .unwrap_or(Ordering::Equal)
                .then_with(|| a.record_id.cmp(&b.record_id))
        });
        results.truncate(q.limit);
        Ok(results)
    }
}

#[async_trait]
impl SearchRepository for TantivySearch {
    async fn search(&self, query: &SearchQuery) -> Result<Vec<SearchResult>> {
        let this = self.clone();
        let query = query.clone();
        tokio::task::spawn_blocking(move || this.search_blocking(&query))
            .await
            .map_err(search_err)?
    }

    async fn num_docs(&self) -> Result<u64> {
        Ok(self.reader.searcher().num_docs())
    }
}
