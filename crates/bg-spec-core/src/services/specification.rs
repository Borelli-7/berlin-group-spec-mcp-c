use super::{EVIDENCE_NOTICE, ServiceSettings};
use crate::{
    CoreError, Result,
    domain::{
        Document, DocumentAuthority, DocumentKind, DocumentStatus, EvidenceChunk, EvidenceClass,
        OpenApiOperation, OpenApiSchema, PageStatus, Provenance, SearchQuery, SearchResult,
        SourceLocator, SpecificationVersion, collapse_duplicates,
    },
    openapi_path::{normalize_method, path_key, template_score, validate_path},
    ports::{CatalogRepository, CatalogStats, IndexMeta, SearchRepository},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

const MAX_QUERY_CHARS: usize = 1000;
const MAX_SCHEMA_CLOSURE: usize = 2000;

const RELATED_MAX_TERMS: usize = 40;
const STOPWORDS: &[&str] = &[
    "about", "after", "all", "also", "and", "any", "are", "been", "but", "can", "case", "cases",
    "does", "each", "for", "from", "has", "have", "into", "its", "may", "might", "more", "must",
    "not", "only", "other", "same", "shall", "should", "such", "than", "that", "the", "then",
    "there", "these", "this", "those", "through", "under", "used", "using", "via", "was", "were",
    "when", "where", "which", "while", "will", "with", "within", "would", "you", "your",
];

/// Query terms for "related evidence" of a requirement statement: code-like identifiers first
/// (kept intact, e.g. `PSU-ID`), then distinct non-stopword words in order of appearance.
fn related_terms(text: &str) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut identifiers = Vec::new();
    let mut words = Vec::new();
    for raw in text.split_whitespace() {
        let token = raw.trim_matches(|c: char| !c.is_alphanumeric());
        if crate::identifiers::is_code_like(token) {
            if seen.insert(token.to_lowercase()) {
                identifiers.push(token.to_string());
            }
            continue;
        }
        for w in token.split(|c: char| !c.is_alphanumeric()) {
            let lower = w.to_lowercase();
            if w.chars().count() > 2 && !STOPWORDS.contains(&lower.as_str()) && seen.insert(lower) {
                words.push(w.to_string());
            }
        }
    }
    identifiers.extend(words);
    identifiers.truncate(RELATED_MAX_TERMS);
    identifiers
}

#[cfg(test)]
mod related_terms_tests {
    #[test]
    fn prefers_identifiers_and_drops_stopwords() {
        let terms = super::related_terms(
            "The ASPSP shall reject the request when the PSU-ID header and the psuId are missing; the ASPSP shall respond.",
        );
        assert_eq!(&terms[..2], ["PSU-ID", "psuId"]);
        assert!(
            !terms
                .iter()
                .any(|t| t.eq_ignore_ascii_case("the") || t == "shall")
        );
        assert_eq!(terms.iter().filter(|t| *t == "ASPSP").count(), 1);
    }
}

/// Hits fetched before duplicates are collapsed, so the requested number of distinct hits remains.
fn overfetch(limit: usize) -> usize {
    limit.saturating_mul(3).max(limit.saturating_add(10))
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct SearchFilters {
    pub version: Option<String>,
    pub kind: Option<String>,
    pub source_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SearchResponse {
    pub query: String,
    pub filters: SearchFilters,
    pub notice: String,
    pub total: usize,
    pub results: Vec<SearchResult>,
}

/// Catalog view of a source (no absolute filesystem paths).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SourceListing {
    pub source_id: String,
    pub document_id: String,
    pub version: SpecificationVersion,
    pub kind: DocumentKind,
    pub title: String,
    pub authority: DocumentAuthority,
    pub precedence: u32,
    /// Path relative to the corpus root.
    pub path: String,
    pub sha256: Option<String>,
    /// `indexed`, `partial`, `missing` or `failed`.
    pub status: DocumentStatus,
    pub page_count: Option<u32>,
    pub ocr_required_pages: Vec<u32>,
    pub diagnostics: usize,
    pub indexed_at_unix: i64,
}

impl From<&Document> for SourceListing {
    fn from(d: &Document) -> Self {
        Self {
            source_id: d.source_id.clone(),
            document_id: d.document_id.clone(),
            version: d.version.clone(),
            kind: d.kind,
            title: d.title.clone(),
            authority: d.authority,
            precedence: d.precedence,
            path: d.path.clone(),
            sha256: d.sha256.clone(),
            status: d.status,
            page_count: d.page_count,
            ocr_required_pages: d
                .diagnostics
                .iter()
                .filter(|x| x.code == "ocr_required")
                .filter_map(|x| x.locator.as_deref()?.strip_prefix("page:")?.parse().ok())
                .collect(),
            diagnostics: d.diagnostics.len(),
            indexed_at_unix: d.indexed_at_unix,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ListSourcesResponse {
    pub total: usize,
    pub sources: Vec<SourceListing>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PageContent {
    pub page: u32,
    pub status: PageStatus,
    pub text: String,
    pub sha256: String,
    /// Set when the page carries no usable text (e.g. `ocr_required`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PageOverview {
    pub page: u32,
    pub status: PageStatus,
    pub char_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SourceContent {
    Pages {
        pages: Vec<PageContent>,
    },
    Chunks {
        chunks: Vec<EvidenceChunk>,
    },
    Operations {
        operations: Vec<OpenApiOperation>,
    },
    Schema {
        schema: Box<OpenApiSchema>,
    },
    Document {
        metadata: serde_json::Value,
        diagnostics: Vec<crate::domain::Diagnostic>,
        pages: Vec<PageOverview>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ReadSourceResponse {
    pub source: SourceListing,
    /// Normalized locator that was read.
    pub locator: String,
    pub classification: EvidenceClass,
    pub content: SourceContent,
    /// Provenance of every returned record.
    pub provenance: Vec<Provenance>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MatchedBy {
    /// Path template matched exactly.
    Exact,
    /// Matched via canonical key (version prefix stripped, parameter names ignored).
    Canonical,
}

/// Result of resolving an endpoint in one version.
#[derive(Debug, Clone)]
pub struct OperationLookup {
    pub primary: OpenApiOperation,
    pub matched_by: MatchedBy,
    /// Other sources of the same version defining the same path template.
    pub others: Vec<OpenApiOperation>,
    /// Distinct path templates sharing the canonical key (highest precedence per template).
    pub other_templates: Vec<OpenApiOperation>,
    /// True when several templates fit equally well and precedence decided.
    pub ambiguous: bool,
}

impl OperationLookup {
    /// Whether a curated endpoint/path reference denotes the resolved template. When other
    /// templates share the canonical key, parameter names must match the resolved one.
    pub fn denotes(&self, path: &str) -> bool {
        self.other_templates.is_empty()
            || template_score(path, &self.primary.path) > 0
                && self.other_templates.iter().all(|o| {
                    template_score(path, &o.path) <= template_score(path, &self.primary.path)
                })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct EndpointResponse {
    pub version: SpecificationVersion,
    pub requested_method: String,
    pub requested_path: String,
    pub matched_by: MatchedBy,
    pub operation: OpenApiOperation,
    /// Other sources of the same version defining this operation (highest precedence wins).
    pub alternatives: Vec<Provenance>,
    /// True when the requested path fits several templates equally and precedence decided.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub ambiguous: bool,
    /// Other path templates sharing the canonical key (e.g. differing only in parameter names).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub other_templates: Vec<Provenance>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SchemaResponse {
    pub version: SpecificationVersion,
    pub name: String,
    pub schema: OpenApiSchema,
    /// Directly referenced component schemas.
    pub referenced_schemas: Vec<String>,
    /// Transitively referenced component schemas (excluding the schema itself).
    pub transitive_referenced_schemas: Vec<String>,
    pub unresolved_references: Vec<String>,
    pub alternatives: Vec<Provenance>,
    /// References resolved in another source of the same version.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cross_source_references: Vec<CrossSourceReference>,
    /// Schemas of the closure that take part in a reference cycle.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cyclic_schemas: Vec<String>,
    /// True when the closure stopped at its size cap.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub truncated: bool,
}

/// A schema name the referencing source does not define, resolved in another source instead.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CrossSourceReference {
    pub name: String,
    /// Source whose operation or schema referenced the name.
    pub referenced_from: String,
    /// The schema that was used (highest precedence in the version).
    pub resolved: Provenance,
    /// Number of sources of the version defining the name.
    pub candidates: usize,
}

/// Transitive closure of schemas reachable from a set of roots.
#[derive(Debug, Clone, Default)]
pub struct SchemaClosure {
    pub schemas: BTreeMap<String, OpenApiSchema>,
    pub unresolved: BTreeSet<String>,
    pub cross_source: Vec<CrossSourceReference>,
    pub truncated: bool,
}

impl SchemaClosure {
    pub fn schema_set(&self) -> crate::diff::SchemaSet {
        self.schemas
            .iter()
            .map(|(k, v)| (k.clone(), v.schema.clone()))
            .collect()
    }

    /// Schemas of the closure that lie on a reference cycle (Tarjan SCC; size > 1 or self-loop).
    pub fn cyclic(&self) -> BTreeSet<String> {
        struct Tarjan<'a> {
            graph: &'a BTreeMap<String, OpenApiSchema>,
            index: BTreeMap<&'a str, (usize, usize)>,
            stack: Vec<&'a str>,
            on_stack: BTreeSet<&'a str>,
            next: usize,
            out: BTreeSet<String>,
        }
        impl<'a> Tarjan<'a> {
            fn visit(&mut self, v: &'a str) {
                self.index.insert(v, (self.next, self.next));
                self.next += 1;
                self.stack.push(v);
                self.on_stack.insert(v);
                for w in &self.graph[v].referenced_schemas {
                    let w = w.as_str();
                    let Some((wk, _)) = self.graph.get_key_value(w) else {
                        continue;
                    };
                    let w = wk.as_str();
                    if !self.index.contains_key(w) {
                        self.visit(w);
                        let low = self.index[v].1.min(self.index[w].1);
                        self.index.get_mut(v).unwrap().1 = low;
                    } else if self.on_stack.contains(w) {
                        let low = self.index[v].1.min(self.index[w].0);
                        self.index.get_mut(v).unwrap().1 = low;
                    }
                }
                if self.index[v].0 == self.index[v].1 {
                    let mut scc = Vec::new();
                    while let Some(w) = self.stack.pop() {
                        self.on_stack.remove(w);
                        scc.push(w);
                        if w == v {
                            break;
                        }
                    }
                    let self_loop = self.graph[v].referenced_schemas.iter().any(|r| r == v);
                    if scc.len() > 1 || self_loop {
                        self.out.extend(scc.into_iter().map(str::to_owned));
                    }
                }
            }
        }
        let mut t = Tarjan {
            graph: &self.schemas,
            index: BTreeMap::new(),
            stack: Vec::new(),
            on_stack: BTreeSet::new(),
            next: 0,
            out: BTreeSet::new(),
        };
        for k in self.schemas.keys() {
            if !t.index.contains_key(k.as_str()) {
                t.visit(k);
            }
        }
        t.out
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct HealthReport {
    pub status: String,
    pub read_only: bool,
    pub server_version: String,
    pub baseline_version: SpecificationVersion,
    pub target_version: SpecificationVersion,
    pub catalog: CatalogStats,
    pub index: IndexMeta,
    pub search_documents: u64,
    pub versions: Vec<String>,
    pub problems: Vec<String>,
}

/// Source retrieval, OpenAPI lookup and full-text search.
#[derive(Clone)]
pub struct SpecificationService {
    catalog: Arc<dyn CatalogRepository>,
    search: Arc<dyn SearchRepository>,
    settings: Arc<ServiceSettings>,
}

pub(crate) fn provenance(doc: &Document, locator: String, sha256: String) -> Provenance {
    Provenance::for_document(doc, locator, sha256)
}

impl SpecificationService {
    pub fn new(
        catalog: Arc<dyn CatalogRepository>,
        search: Arc<dyn SearchRepository>,
        settings: Arc<ServiceSettings>,
    ) -> Self {
        Self {
            catalog,
            search,
            settings,
        }
    }

    pub fn settings(&self) -> &ServiceSettings {
        &self.settings
    }

    pub(crate) fn catalog(&self) -> &dyn CatalogRepository {
        self.catalog.as_ref()
    }

    /// Parses an optional version, defaulting to the configured target version.
    pub fn version_or_target(&self, v: Option<&str>) -> Result<SpecificationVersion> {
        match v.map(str::trim).filter(|s| !s.is_empty()) {
            Some(v) => SpecificationVersion::new(v),
            None => Ok(self.settings.target.clone()),
        }
    }

    pub fn clamp_limit(&self, limit: Option<usize>) -> usize {
        limit
            .unwrap_or(self.settings.default_limit)
            .clamp(1, self.settings.max_limit)
    }

    pub fn path_key_for(&self, version: &SpecificationVersion, path: &str) -> String {
        path_key(path, self.settings.prefix_for(version))
    }

    /// Lookup keys in priority order: the version's own prefix first, then every other
    /// configured prefix, so `/v1/...` and `/v2/...` paths resolve in either version.
    pub fn candidate_path_keys(&self, version: &SpecificationVersion, path: &str) -> Vec<String> {
        let mut keys = vec![self.path_key_for(version, path)];
        for prefix in self.settings.path_prefixes.values() {
            let key = path_key(path, Some(prefix));
            if !keys.contains(&key) {
                keys.push(key);
            }
        }
        keys
    }

    async fn known_versions(&self) -> Result<BTreeSet<String>> {
        Ok(self
            .catalog
            .list_documents()
            .await?
            .into_iter()
            .map(|d| d.version.to_string())
            .collect())
    }

    /// Ensures the version exists in the catalog, returning a helpful error otherwise.
    pub async fn ensure_known_version(&self, v: &SpecificationVersion) -> Result<()> {
        let known = self.known_versions().await?;
        if known.contains(v.as_str()) {
            Ok(())
        } else {
            Err(CoreError::InvalidInput(format!(
                "unknown version '{v}'; indexed versions: {}",
                known.into_iter().collect::<Vec<_>>().join(", ")
            )))
        }
    }

    pub async fn search(
        &self,
        query: &str,
        version: Option<&str>,
        kind: Option<&str>,
        source_id: Option<&str>,
        limit: Option<usize>,
    ) -> Result<SearchResponse> {
        let _timer = crate::timing::Timer::start("specification.search");
        let text = query.trim();
        if text.is_empty() {
            return Err(CoreError::InvalidInput("query must not be empty".into()));
        }
        if text.chars().count() > MAX_QUERY_CHARS {
            return Err(CoreError::InvalidInput(format!(
                "query longer than {MAX_QUERY_CHARS} characters"
            )));
        }
        let version = match version.map(str::trim).filter(|s| !s.is_empty()) {
            Some(v) => {
                let v = SpecificationVersion::new(v)?;
                self.ensure_known_version(&v).await?;
                Some(v)
            }
            None => None,
        };
        let kinds = match kind.map(str::trim).filter(|s| !s.is_empty()) {
            Some(k) => vec![k.parse::<DocumentKind>()?],
            None => Vec::new(),
        };
        let source_id = source_id.map(str::trim).filter(|s| !s.is_empty());
        if let Some(sid) = source_id
            && self.catalog.get_document(sid).await?.is_none()
        {
            return Err(CoreError::NotFound(format!("unknown source_id '{sid}'")));
        }
        let q = SearchQuery {
            text: text.to_owned(),
            version: version.clone(),
            kinds: kinds.clone(),
            source_id: source_id.map(str::to_owned),
            limit: overfetch(self.clamp_limit(limit)),
        };
        let results = collapse_duplicates(self.search.search(&q).await?, self.clamp_limit(limit));
        Ok(SearchResponse {
            query: text.to_owned(),
            filters: SearchFilters {
                version: version.map(|v| v.to_string()),
                kind: kinds.first().map(|k| k.to_string()),
                source_id: source_id.map(str::to_owned),
            },
            notice: EVIDENCE_NOTICE.to_owned(),
            total: results.len(),
            results,
        })
    }

    /// Internal evidence search used by bundle builders (no user-facing validation).
    pub(crate) async fn related_evidence(
        &self,
        text: &str,
        version: &SpecificationVersion,
        limit: usize,
    ) -> Result<Vec<SearchResult>> {
        let _timer = crate::timing::Timer::start("specification.related_evidence");
        let words = related_terms(text);
        if words.is_empty() || limit == 0 {
            return Ok(Vec::new());
        }
        let limit = limit.min(self.settings.max_limit);
        let hits = self
            .search
            .search(&SearchQuery {
                text: words.join(" "),
                version: Some(version.clone()),
                kinds: vec![DocumentKind::Pdf, DocumentKind::Text],
                source_id: None,
                limit: overfetch(limit),
            })
            .await?;
        Ok(collapse_duplicates(hits, limit))
    }

    pub async fn list_sources(
        &self,
        version: Option<&str>,
        kind: Option<&str>,
    ) -> Result<ListSourcesResponse> {
        let version = version
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(SpecificationVersion::new)
            .transpose()?;
        let kind = kind
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::parse::<DocumentKind>)
            .transpose()?;
        let sources: Vec<SourceListing> = self
            .catalog
            .list_documents()
            .await?
            .iter()
            .filter(|d| version.as_ref().is_none_or(|v| &d.version == v))
            .filter(|d| kind.is_none_or(|k| d.kind == k))
            .map(SourceListing::from)
            .collect();
        Ok(ListSourcesResponse {
            total: sources.len(),
            sources,
        })
    }

    async fn indexed_document(&self, source_id: &str) -> Result<Document> {
        let doc = self
            .catalog
            .get_document(source_id)
            .await?
            .ok_or_else(|| CoreError::NotFound(format!("unknown source_id '{source_id}'")))?;
        Ok(doc)
    }

    pub async fn read_source(&self, source_id: &str, locator: &str) -> Result<ReadSourceResponse> {
        let _timer = crate::timing::Timer::start("specification.read_source");
        let locator: SourceLocator = locator.parse()?;
        let doc = self.indexed_document(source_id.trim()).await?;
        self.read_loaded_source(doc, locator).await
    }

    /// [`Self::read_source`] for a document the caller has already loaded.
    pub(crate) async fn read_loaded_source(
        &self,
        doc: Document,
        locator: SourceLocator,
    ) -> Result<ReadSourceResponse> {
        let listing = SourceListing::from(&doc);
        if !matches!(locator, SourceLocator::Document)
            && matches!(doc.status, DocumentStatus::Missing | DocumentStatus::Failed)
        {
            return Err(CoreError::NotFound(format!(
                "source '{}' has status '{}'; no evidence is available (run `bg-spec doctor`)",
                doc.source_id, doc.status
            )));
        }
        let paged = matches!(doc.kind, DocumentKind::Pdf | DocumentKind::Text);
        let wrong_kind = |what: &str| {
            CoreError::InvalidInput(format!(
                "locator '{locator}' addresses {what} but source '{}' is of kind '{}'",
                doc.source_id, doc.kind
            ))
        };
        let (content, provenance) = match &locator {
            SourceLocator::Document => {
                let pages = match doc.page_count {
                    Some(n) if n > 0 => self.catalog.get_pages(&doc.source_id, 1, n).await?,
                    _ => Vec::new(),
                };
                let content = SourceContent::Document {
                    metadata: doc.metadata.clone(),
                    diagnostics: doc.diagnostics.clone(),
                    pages: pages
                        .iter()
                        .map(|p| PageOverview {
                            page: p.page,
                            status: p.status,
                            char_count: p.char_count,
                        })
                        .collect(),
                };
                let prov = provenance(
                    &doc,
                    "document".into(),
                    doc.sha256.clone().unwrap_or_default(),
                );
                (content, vec![prov])
            }
            SourceLocator::Page(a) | SourceLocator::PageRange(a, _) => {
                if !paged {
                    return Err(wrong_kind("pages"));
                }
                let b = match locator {
                    SourceLocator::PageRange(_, b) => b,
                    _ => *a,
                };
                let max = doc.page_count.unwrap_or(0);
                if *a > max {
                    return Err(CoreError::NotFound(format!(
                        "page {a} out of range: source '{}' has {max} page(s)",
                        doc.source_id
                    )));
                }
                let pages = self
                    .catalog
                    .get_pages(&doc.source_id, *a, b.min(max))
                    .await?;
                let prov = pages
                    .iter()
                    .map(|p| provenance(&doc, format!("page:{}", p.page), p.sha256.clone()))
                    .collect();
                let pages = pages
                    .into_iter()
                    .map(|p| PageContent {
                        warning: match p.status {
                            PageStatus::Extracted => None,
                            PageStatus::Blank => Some(format!(
                                "page {} is declared intentionally blank in the manifest",
                                p.page
                            )),
                            _ => Some(format!(
                                "page {} has status '{}'; no extractable text is available",
                                p.page, p.status
                            )),
                        },
                        page: p.page,
                        status: p.status,
                        text: p.text,
                        sha256: p.sha256,
                    })
                    .collect();
                (SourceContent::Pages { pages }, prov)
            }
            SourceLocator::Section(s) => {
                if !paged {
                    return Err(wrong_kind("a section"));
                }
                let chunks = self.catalog.chunks_for_section(&doc.source_id, s).await?;
                if chunks.is_empty() {
                    return Err(CoreError::NotFound(format!(
                        "section '{s}' not found in '{}'",
                        doc.source_id
                    )));
                }
                let prov = chunks.iter().map(|c| c.provenance.clone()).collect();
                (SourceContent::Chunks { chunks }, prov)
            }
            SourceLocator::Chunk(id) => {
                let chunk = self
                    .catalog
                    .get_chunk(id)
                    .await?
                    .filter(|c| c.provenance.source_id == doc.source_id)
                    .ok_or_else(|| {
                        CoreError::NotFound(format!(
                            "chunk '{id}' not found in '{}'",
                            doc.source_id
                        ))
                    })?;
                let prov = vec![chunk.provenance.clone()];
                (
                    SourceContent::Chunks {
                        chunks: vec![chunk],
                    },
                    prov,
                )
            }
            SourceLocator::Operation { method, path } => {
                if doc.kind != DocumentKind::Openapi {
                    return Err(wrong_kind("an OpenAPI operation"));
                }
                let op = self
                    .catalog
                    .get_source_operation(&doc.source_id, method, path)
                    .await?
                    .ok_or_else(|| {
                        CoreError::NotFound(format!(
                            "operation {method} {path} not found in '{}'",
                            doc.source_id
                        ))
                    })?;
                let prov = vec![op.provenance.clone()];
                (
                    SourceContent::Operations {
                        operations: vec![op],
                    },
                    prov,
                )
            }
            SourceLocator::Path(path) => {
                if doc.kind != DocumentKind::Openapi {
                    return Err(wrong_kind("an OpenAPI path"));
                }
                let ops = self
                    .catalog
                    .source_operations_by_path(&doc.source_id, path)
                    .await?;
                if ops.is_empty() {
                    return Err(CoreError::NotFound(format!(
                        "path {path} not found in '{}'",
                        doc.source_id
                    )));
                }
                let prov = ops.iter().map(|o| o.provenance.clone()).collect();
                (SourceContent::Operations { operations: ops }, prov)
            }
            SourceLocator::Schema(name) => {
                if doc.kind != DocumentKind::Openapi {
                    return Err(wrong_kind("an OpenAPI schema"));
                }
                let schema = self
                    .catalog
                    .get_source_schema(&doc.source_id, name)
                    .await?
                    .ok_or_else(|| {
                        CoreError::NotFound(format!(
                            "schema '{name}' not found in '{}'",
                            doc.source_id
                        ))
                    })?;
                let prov = vec![schema.provenance.clone()];
                (
                    SourceContent::Schema {
                        schema: Box::new(schema),
                    },
                    prov,
                )
            }
        };
        Ok(ReadSourceResponse {
            source: listing,
            locator: locator.to_string(),
            classification: EvidenceClass::SourceContent,
            content,
            provenance,
        })
    }

    /// Resolves an operation in a version (exact template first, then canonical key).
    pub async fn lookup_operation(
        &self,
        version: &SpecificationVersion,
        method: &str,
        path: &str,
    ) -> Result<Option<OperationLookup>> {
        let _timer = crate::timing::Timer::start("specification.lookup_operation");
        let method = normalize_method(method)?;
        let path = validate_path(path)?;
        let mut ops = Vec::new();
        for key in self.candidate_path_keys(version, &path) {
            ops = self.catalog.find_operations(version, &method, &key).await?;
            if !ops.is_empty() {
                break;
            }
        }
        if ops.is_empty() {
            return Ok(None);
        }
        // `ops` is in precedence order; the first operation with the best score wins.
        let scores: Vec<u8> = ops.iter().map(|o| template_score(&path, &o.path)).collect();
        let best = scores.iter().copied().max().unwrap_or(0);
        let idx = scores.iter().position(|s| *s == best).unwrap_or(0);
        let primary = ops.remove(idx);
        let matched_by = if primary.path == path {
            MatchedBy::Exact
        } else {
            MatchedBy::Canonical
        };
        let (others, rest): (Vec<_>, Vec<_>) =
            ops.into_iter().partition(|o| o.path == primary.path);
        let mut other_templates: Vec<OpenApiOperation> = Vec::new();
        for o in rest {
            if !other_templates.iter().any(|t| t.path == o.path) {
                other_templates.push(o);
            }
        }
        let ambiguous = other_templates
            .iter()
            .any(|o| template_score(&path, &o.path) == best);
        Ok(Some(OperationLookup {
            primary,
            matched_by,
            others,
            other_templates,
            ambiguous,
        }))
    }

    pub async fn read_endpoint(
        &self,
        version: Option<&str>,
        path: &str,
        method: &str,
    ) -> Result<EndpointResponse> {
        let version = self.version_or_target(version)?;
        self.ensure_known_version(&version).await?;
        let lookup = self
            .lookup_operation(&version, method, path)
            .await?
            .ok_or_else(|| {
                CoreError::NotFound(format!(
                    "no OpenAPI operation {} {path} in version '{version}'",
                    method.to_ascii_uppercase()
                ))
            })?;
        Ok(EndpointResponse {
            version,
            requested_method: lookup.primary.method.clone(),
            requested_path: path.trim().to_owned(),
            matched_by: lookup.matched_by,
            alternatives: lookup.others.iter().map(|o| o.provenance.clone()).collect(),
            ambiguous: lookup.ambiguous,
            other_templates: lookup
                .other_templates
                .iter()
                .map(|o| o.provenance.clone())
                .collect(),
            operation: lookup.primary,
        })
    }

    pub async fn read_schema(&self, version: Option<&str>, name: &str) -> Result<SchemaResponse> {
        let _timer = crate::timing::Timer::start("specification.read_schema");
        let version = self.version_or_target(version)?;
        let name = name.trim();
        if name.is_empty() || name.len() > 256 {
            return Err(CoreError::InvalidInput(
                "schema name must be 1-256 characters".into(),
            ));
        }
        self.ensure_known_version(&version).await?;
        let mut found = self.catalog.find_schemas(&version, name).await?;
        if found.is_empty() {
            return Err(CoreError::NotFound(format!(
                "no schema '{name}' in version '{version}'"
            )));
        }
        let schema = found.remove(0);
        let closure = self
            .schema_closure(
                &schema.provenance.source_id,
                &version,
                schema.referenced_schemas.iter().cloned(),
            )
            .await?;
        Ok(SchemaResponse {
            version,
            name: schema.name.clone(),
            referenced_schemas: schema.referenced_schemas.clone(),
            transitive_referenced_schemas: closure
                .schemas
                .keys()
                .filter(|k| *k != &schema.name)
                .cloned()
                .collect(),
            cyclic_schemas: closure.cyclic().into_iter().collect(),
            unresolved_references: closure.unresolved.into_iter().collect(),
            alternatives: found.iter().map(|s| s.provenance.clone()).collect(),
            cross_source_references: closure.cross_source,
            truncated: closure.truncated,
            schema,
        })
    }

    /// Breadth-first transitive closure. Each reference is resolved in the source of the
    /// referencing schema first, then (reported in `cross_source`) by precedence in the version.
    pub async fn schema_closure(
        &self,
        source_id: &str,
        version: &SpecificationVersion,
        roots: impl IntoIterator<Item = String>,
    ) -> Result<SchemaClosure> {
        let _timer = crate::timing::Timer::start("specification.schema_closure");
        let mut closure = SchemaClosure::default();
        // Each reference is resolved in the source of the schema/operation that made it.
        // Breadth-first, one level at a time: each level is fetched with one query per referencing
        // source plus one for cross-source fallbacks, then applied in FIFO order.
        let mut level: Vec<(String, String)> = roots
            .into_iter()
            .map(|n| (n, source_id.to_owned()))
            .collect();
        while !level.is_empty() {
            let pending: Vec<&(String, String)> = level
                .iter()
                .filter(|(n, _)| {
                    !closure.schemas.contains_key(n) && !closure.unresolved.contains(n)
                })
                .collect();
            let mut by_source: BTreeMap<&str, Vec<String>> = BTreeMap::new();
            for (name, from) in &pending {
                by_source.entry(from).or_default().push(name.clone());
            }
            let mut local: BTreeMap<(String, String), OpenApiSchema> = BTreeMap::new();
            for (from, names) in by_source {
                for s in self.catalog.get_source_schemas(from, &names).await? {
                    local.insert((from.to_owned(), s.name.clone()), s);
                }
            }
            let mut misses: Vec<String> = pending
                .iter()
                .filter(|(n, f)| !local.contains_key(&(f.clone(), n.clone())))
                .map(|(n, _)| n.clone())
                .collect();
            misses.sort();
            misses.dedup();
            let mut fallback: BTreeMap<String, Vec<OpenApiSchema>> = BTreeMap::new();
            for s in self.catalog.find_schemas_by_names(version, &misses).await? {
                fallback.entry(s.name.clone()).or_default().push(s);
            }
            let mut next = Vec::new();
            for (name, from) in std::mem::take(&mut level) {
                if closure.schemas.contains_key(&name) || closure.unresolved.contains(&name) {
                    continue;
                }
                if closure.schemas.len() >= MAX_SCHEMA_CLOSURE {
                    closure.truncated = true;
                    return Ok(closure);
                }
                let schema = match local.remove(&(from.clone(), name.clone())) {
                    Some(s) => Some(s),
                    None => fallback.get(&name).and_then(|all| {
                        let s = all.first()?.clone();
                        closure.cross_source.push(CrossSourceReference {
                            name: name.clone(),
                            referenced_from: from.clone(),
                            resolved: s.provenance.clone(),
                            candidates: all.len(),
                        });
                        Some(s)
                    }),
                };
                match schema {
                    Some(s) => {
                        next.extend(
                            s.referenced_schemas
                                .iter()
                                .map(|r| (r.clone(), s.provenance.source_id.clone())),
                        );
                        closure.schemas.insert(name, s);
                    }
                    None => {
                        closure.unresolved.insert(name);
                    }
                }
            }
            level = next;
        }
        Ok(closure)
    }

    pub async fn health(&self) -> Result<HealthReport> {
        let catalog = self.catalog.stats().await?;
        let index = self.catalog.index_meta().await?;
        let search_documents = self.search.num_docs().await?;
        let docs = self.catalog.list_documents().await?;
        let mut problems = Vec::new();
        if index.schema_version.is_none() {
            problems.push("catalog has never been indexed; run `bg-spec index`".to_owned());
        }
        for d in &docs {
            if matches!(d.status, DocumentStatus::Missing | DocumentStatus::Failed) {
                problems.push(format!(
                    "source '{}' has status '{}'",
                    d.source_id, d.status
                ));
            }
        }
        if catalog.ocr_required_pages > 0 {
            problems.push(format!(
                "{} page(s) require OCR (no extractable text)",
                catalog.ocr_required_pages
            ));
        }
        Ok(HealthReport {
            status: if problems.is_empty() {
                "ok"
            } else {
                "degraded"
            }
            .to_owned(),
            read_only: true,
            server_version: env!("CARGO_PKG_VERSION").to_owned(),
            baseline_version: self.settings.baseline.clone(),
            target_version: self.settings.target.clone(),
            versions: docs
                .iter()
                .map(|d| d.version.to_string())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect(),
            catalog,
            index,
            search_documents,
            problems,
        })
    }
}
