use super::{
    CrossSourceReference, EVIDENCE_NOTICE, MatchedBy, SourceContent, SpecificationService,
    specification::provenance,
};
use crate::{
    CoreError, Result,
    domain::{
        Citation, Conflict, ConflictKind, DocumentAuthority, DocumentKind, DocumentStatus,
        EndpointRef, EvidenceClass, MappingProvenance, OpenApiOperation, OpenApiSchema, Provenance,
        Requirement, RequirementRecord, SearchResult, SourceLocator, SpecificationVersion,
    },
    openapi_path::{normalize_method, validate_path},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

const EXCERPT_CHARS: usize = 1200;
const DEFAULT_EVIDENCE_LIMIT: usize = 8;

/// Summary of a cited, indexed source document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DocumentRef {
    pub title: String,
    pub kind: DocumentKind,
    pub version: SpecificationVersion,
    pub authority: DocumentAuthority,
    pub precedence: u32,
    pub sha256: Option<String>,
    pub status: DocumentStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SourceResolution {
    /// Locator resolved to indexed content.
    Resolved,
    /// Resolved, but the pinned hash no longer matches the indexed document.
    Stale,
    /// Source exists but the locator does not resolve.
    Dangling,
    /// `source_id` is not in the catalog.
    UnknownSource,
    /// Source is catalogued but missing/failed; no evidence available.
    Unavailable,
}

/// A curated requirement citation resolved against the catalog.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TracedSource {
    pub source_id: String,
    pub locator: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    pub status: SourceResolution,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document: Option<DocumentRef>,
    /// Beginning of the resolved content (use `read_source` for the full text).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub excerpt: Option<String>,
    pub provenance: Vec<Provenance>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AcceptanceCriterion {
    /// `<REQUIREMENT-ID>/AC<n>` (1-based, order of the curated mapping).
    pub id: String,
    pub requirement_id: String,
    pub text: String,
    pub mapping: MappingProvenance,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct OperationSummary {
    pub method: String,
    pub path: String,
    pub operation_id: Option<String>,
    pub summary: Option<String>,
    pub response_statuses: Vec<String>,
    pub referenced_schemas: Vec<String>,
}

impl From<&OpenApiOperation> for OperationSummary {
    fn from(o: &OpenApiOperation) -> Self {
        Self {
            method: o.method.clone(),
            path: o.path.clone(),
            operation_id: o.operation_id.clone(),
            summary: o.summary.clone(),
            response_statuses: o.responses.iter().map(|r| r.status.clone()).collect(),
            referenced_schemas: o.referenced_schemas.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TracedEndpoint {
    /// `METHOD /path` as referenced by the mapping.
    pub endpoint: String,
    /// `endpoints` (explicit list) or `source:<source_id> <locator>`.
    pub via: String,
    pub found: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation: Option<OperationSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance: Option<Provenance>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TracedSchema {
    pub name: String,
    /// `explicit`, `source:<id>`, `operation:<METHOD path>` or `transitive`.
    pub via: String,
    pub found: bool,
    pub referenced_schemas: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance: Option<Provenance>,
}

/// Requirement -> sources -> endpoints -> schemas -> acceptance criteria -> evidence.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RequirementTrace {
    pub requirement_id: String,
    pub classification: EvidenceClass,
    pub requirement: Requirement,
    pub mapping: MappingProvenance,
    pub sources: Vec<TracedSource>,
    pub endpoints: Vec<TracedEndpoint>,
    pub schemas: Vec<TracedSchema>,
    pub acceptance_criteria: Vec<AcceptanceCriterion>,
    /// Full-text matches for the requirement title: discovered evidence only.
    pub related_evidence: Vec<SearchResult>,
    pub conflicts: Vec<Conflict>,
    pub notice: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RequirementMatch {
    pub requirement_id: String,
    pub classification: EvidenceClass,
    pub version: SpecificationVersion,
    pub title: String,
    /// `id_exact`, `id_partial` or `text`.
    pub match_reason: String,
    pub sources: Vec<Citation>,
    pub related_endpoints: Vec<String>,
    pub acceptance_criteria: Vec<AcceptanceCriterion>,
    pub mapping: MappingProvenance,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct FindRequirementResponse {
    pub query: String,
    pub notice: String,
    /// Curated requirement mappings only (authoritative).
    pub requirements: Vec<RequirementMatch>,
    /// Full-text search hits (discovered evidence, NOT requirements).
    pub discovered_evidence: Vec<SearchResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct EndpointDescriptor {
    pub version: SpecificationVersion,
    pub method: String,
    pub requested_path: String,
    pub found: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matched_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matched_by: Option<MatchedBy>,
    /// True when the requested path fits several templates equally and precedence decided.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub ambiguous: bool,
    /// Other path templates sharing the canonical key; requirements linked to them are excluded.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub other_templates: Vec<Provenance>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct EndpointRequirement {
    pub requirement_id: String,
    pub classification: EvidenceClass,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// How the requirement is linked to the endpoint.
    pub linked_via: Vec<String>,
    pub acceptance_criteria: Vec<AcceptanceCriterion>,
    pub sources: Vec<TracedSource>,
    pub mapping: MappingProvenance,
}

/// Role of a citation inside an evidence bundle.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum CitationRole {
    OpenapiOperation,
    OpenapiSchema,
    RequirementSource,
    DiscoveredEvidence,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SourceCitation {
    pub role: CitationRole,
    pub source_id: String,
    pub locator: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authority: Option<DocumentAuthority>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub precedence: Option<u32>,
}

/// Structured evidence bundle for one endpoint (primary Requirements Agent tool).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct EndpointEvidenceBundle {
    pub endpoint: EndpointDescriptor,
    pub notice: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub openapi: Option<OpenApiOperation>,
    pub requirements: Vec<EndpointRequirement>,
    /// Transitive closure of schemas referenced by the operation.
    pub schemas: Vec<OpenApiSchema>,
    pub unresolved_schemas: Vec<String>,
    /// Schema references resolved in another source of the same version.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cross_source_schemas: Vec<CrossSourceReference>,
    pub related_evidence: Vec<SearchResult>,
    pub conflicts: Vec<Conflict>,
    pub sources: Vec<SourceCitation>,
}

/// Curated requirement lookup, tracing and endpoint evidence bundles.
#[derive(Clone)]
pub struct RequirementService {
    spec: SpecificationService,
}

fn excerpt(text: &str) -> String {
    if text.len() <= EXCERPT_CHARS {
        text.to_owned()
    } else {
        format!("{}…", &text[..text.floor_char_boundary(EXCERPT_CHARS)])
    }
}

fn content_text(c: &SourceContent) -> String {
    match c {
        SourceContent::Pages { pages } => pages
            .iter()
            .map(|p| p.text.as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        SourceContent::Chunks { chunks } => chunks
            .iter()
            .map(|c| c.text.as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        SourceContent::Operations { operations } => operations
            .iter()
            .map(|o| {
                format!(
                    "{} {} {}",
                    o.method,
                    o.path,
                    o.summary.as_deref().unwrap_or_default()
                )
            })
            .collect::<Vec<_>>()
            .join("\n"),
        SourceContent::Schema { schema } => schema.schema.to_string(),
        SourceContent::Document { metadata, .. } => metadata.to_string(),
    }
}

fn acceptance(rec: &RequirementRecord) -> Vec<AcceptanceCriterion> {
    rec.requirement
        .acceptance_criteria
        .iter()
        .enumerate()
        .map(|(i, t)| AcceptanceCriterion {
            id: format!("{}/AC{}", rec.requirement.id, i + 1),
            requirement_id: rec.requirement.id.clone(),
            text: t.clone(),
            mapping: rec.mapping.clone(),
        })
        .collect()
}

fn tokens(s: &str) -> Vec<String> {
    s.split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() >= 2)
        .map(str::to_lowercase)
        .collect()
}

fn dedupe_conflicts(conflicts: &mut Vec<Conflict>) {
    let mut seen = BTreeSet::new();
    conflicts.retain(|c| seen.insert((c.kind, c.description.clone())));
    conflicts.sort_by(|a, b| (a.kind, &a.description).cmp(&(b.kind, &b.description)));
}

#[derive(Clone)]
pub(crate) struct ResolvedSources {
    sources: Vec<TracedSource>,
    conflicts: Vec<Conflict>,
}

impl RequirementService {
    pub fn new(spec: SpecificationService) -> Self {
        Self { spec }
    }

    async fn requirement(&self, id: &str) -> Result<RequirementRecord> {
        let id = id.trim();
        if id.is_empty() {
            return Err(CoreError::InvalidInput(
                "requirement_id must not be empty".into(),
            ));
        }
        match self.spec.catalog().get_requirement(id).await? {
            Some(r) => Ok(r),
            None => match self
                .spec
                .catalog()
                .get_requirement(&id.to_ascii_uppercase())
                .await?
            {
                Some(r) => Ok(r),
                None => Err(CoreError::NotFound(format!(
                    "no curated requirement mapping with id '{id}' (requirement ids are never invented; use find_requirement)"
                ))),
            },
        }
    }

    /// Resolves every curated source citation and derives source-level conflicts (cached per
    /// requirement for the lifetime of this index generation).
    async fn resolve_sources(&self, rec: &RequirementRecord) -> Result<ResolvedSources> {
        let cache = &self.spec.caches().requirement_sources;
        if let Some(hit) = cache.get(&rec.requirement.id) {
            return Ok(hit);
        }
        let resolved = self.resolve_sources_uncached(rec).await?;
        cache.insert(rec.requirement.id.clone(), resolved.clone());
        Ok(resolved)
    }

    async fn resolve_sources_uncached(&self, rec: &RequirementRecord) -> Result<ResolvedSources> {
        let _timer = crate::timing::Timer::start("requirement.resolve_sources");
        let req = &rec.requirement;
        let mut sources = Vec::new();
        let mut conflicts = Vec::new();
        let mut cited_docs: BTreeMap<String, DocumentRef> = BTreeMap::new();
        let mut ids: Vec<String> = req.sources.iter().map(|s| s.source_id.clone()).collect();
        ids.sort();
        ids.dedup();
        let documents: BTreeMap<String, crate::domain::Document> = self
            .spec
            .catalog()
            .get_documents(&ids)
            .await?
            .into_iter()
            .map(|d| (d.source_id.clone(), d))
            .collect();
        for s in &req.sources {
            let citation = Citation {
                source_id: s.source_id.clone(),
                locator: s.locator.clone(),
            };
            let mut traced = TracedSource {
                source_id: s.source_id.clone(),
                locator: s.locator.clone(),
                note: s.note.clone(),
                status: SourceResolution::Resolved,
                document: None,
                excerpt: None,
                provenance: Vec::new(),
                detail: None,
            };
            let Some(doc) = documents.get(&s.source_id) else {
                traced.status = SourceResolution::UnknownSource;
                traced.detail = Some("source_id is not declared in the manifest / catalog".into());
                conflicts.push(Conflict {
                    kind: ConflictKind::DanglingReference,
                    description: format!("{} cites unknown source '{}'", req.id, s.source_id),
                    requirement_ids: vec![req.id.clone()],
                    citations: vec![citation],
                    provenance: vec![],
                });
                sources.push(traced);
                continue;
            };
            let dref = DocumentRef {
                title: doc.title.clone(),
                kind: doc.kind,
                version: doc.version.clone(),
                authority: doc.authority,
                precedence: doc.precedence,
                sha256: doc.sha256.clone(),
                status: doc.status,
            };
            traced.document = Some(dref.clone());
            if matches!(doc.status, DocumentStatus::Missing | DocumentStatus::Failed) {
                traced.status = SourceResolution::Unavailable;
                traced.detail = Some(format!("source status is '{}'", doc.status));
                conflicts.push(Conflict {
                    kind: ConflictKind::DanglingReference,
                    description: format!(
                        "{} cites '{}' whose status is '{}'",
                        req.id, s.source_id, doc.status
                    ),
                    requirement_ids: vec![req.id.clone()],
                    citations: vec![citation],
                    provenance: vec![],
                });
                sources.push(traced);
                continue;
            }
            cited_docs.insert(doc.source_id.clone(), dref);
            let read = match s.locator.parse::<SourceLocator>() {
                Ok(locator) => self.spec.read_loaded_source(doc.clone(), locator).await,
                Err(e) => Err(e),
            };
            match read {
                Ok(read) => {
                    traced.excerpt = Some(excerpt(&content_text(&read.content)));
                    traced.provenance = read.provenance;
                }
                Err(e) if e.is_client_error() => {
                    traced.status = SourceResolution::Dangling;
                    traced.detail = Some(e.to_string());
                    conflicts.push(Conflict {
                        kind: ConflictKind::DanglingReference,
                        description: format!(
                            "{} cites {} {} which does not resolve: {e}",
                            req.id, s.source_id, s.locator
                        ),
                        requirement_ids: vec![req.id.clone()],
                        citations: vec![citation.clone()],
                        provenance: vec![provenance(
                            doc,
                            "document".into(),
                            doc.sha256.clone().unwrap_or_default(),
                        )],
                    });
                }
                Err(e) => return Err(e),
            }
            if let Some(pin) = &s.sha256 {
                let actual = doc.sha256.as_deref().unwrap_or_default();
                if !actual.starts_with(&pin.to_ascii_lowercase()) {
                    if traced.status == SourceResolution::Resolved {
                        traced.status = SourceResolution::Stale;
                    }
                    conflicts.push(Conflict {
                        kind: ConflictKind::StaleSource,
                        description: format!(
                            "{} pins {} to sha256 {pin} but the indexed document is {actual}",
                            req.id, s.source_id
                        ),
                        requirement_ids: vec![req.id.clone()],
                        citations: vec![citation],
                        provenance: vec![provenance(doc, "document".into(), actual.to_owned())],
                    });
                }
            }
            sources.push(traced);
        }
        // Precedence ties: same version + same precedence + different authority.
        let mut groups: BTreeMap<(String, u32), Vec<(&String, &DocumentRef)>> = BTreeMap::new();
        for (sid, d) in &cited_docs {
            groups
                .entry((d.version.to_string(), d.precedence))
                .or_default()
                .push((sid, d));
        }
        for ((version, precedence), docs) in groups {
            let authorities: BTreeSet<_> = docs.iter().map(|(_, d)| d.authority).collect();
            if authorities.len() > 1 {
                conflicts.push(Conflict {
                    kind: ConflictKind::PrecedenceTie,
                    description: format!(
                        "{}: sources {} of version {version} share precedence {precedence} but differ in authority ({})",
                        req.id,
                        docs.iter().map(|(s, _)| s.as_str()).collect::<Vec<_>>().join(", "),
                        authorities.iter().map(|a| a.as_str()).collect::<Vec<_>>().join(" vs ")
                    ),
                    requirement_ids: vec![req.id.clone()],
                    citations: req
                        .sources
                        .iter()
                        .filter(|s| docs.iter().any(|(sid, _)| **sid == s.source_id))
                        .map(|s| Citation { source_id: s.source_id.clone(), locator: s.locator.clone() })
                        .collect(),
                    provenance: vec![],
                });
            }
        }
        Ok(ResolvedSources { sources, conflicts })
    }

    /// Declared conflicts in both directions. Citations list the curated sources of every
    /// requirement involved so the conflict can be verified with `read_source`.
    /// Everything [`Self::declared_conflicts`] needs: the requirement, the requirements it declares
    /// conflicts with and those declaring a conflict with it, loaded by indexed query.
    async fn conflict_partners(&self, rec: &RequirementRecord) -> Result<Vec<RequirementRecord>> {
        let mut ids: Vec<String> = rec
            .requirement
            .conflicts_with
            .iter()
            .map(|c| c.requirement_id.clone())
            .collect();
        ids.push(rec.requirement.id.clone());
        ids.sort();
        ids.dedup();
        self.spec
            .catalog()
            .requirement_conflict_partners(&rec.requirement.id, &ids)
            .await
    }

    fn declared_conflicts(
        &self,
        rec: &RequirementRecord,
        all: &[RequirementRecord],
    ) -> Vec<Conflict> {
        let id = &rec.requirement.id;
        let cite = |ids: [&str; 2]| -> Vec<Citation> {
            let mut c: Vec<Citation> = all
                .iter()
                .filter(|r| ids.contains(&r.requirement.id.as_str()))
                .flat_map(|r| r.requirement.sources.iter())
                .map(|s| Citation {
                    source_id: s.source_id.clone(),
                    locator: s.locator.clone(),
                })
                .collect();
            c.sort();
            c.dedup();
            c
        };
        let mut out = Vec::new();
        for c in &rec.requirement.conflicts_with {
            let exists = all.iter().any(|r| r.requirement.id == c.requirement_id);
            out.push(Conflict {
                kind: if exists {
                    ConflictKind::Declared
                } else {
                    ConflictKind::DanglingReference
                },
                description: if exists {
                    format!(
                        "{id} declares a conflict with {}{}",
                        c.requirement_id,
                        c.note
                            .as_deref()
                            .map(|n| format!(": {n}"))
                            .unwrap_or_default()
                    )
                } else {
                    format!(
                        "{id} declares a conflict with unknown requirement {}",
                        c.requirement_id
                    )
                },
                requirement_ids: vec![id.clone(), c.requirement_id.clone()],
                citations: cite([id, &c.requirement_id]),
                provenance: vec![],
            });
        }
        for other in all {
            for c in &other.requirement.conflicts_with {
                if &c.requirement_id == id {
                    out.push(Conflict {
                        kind: ConflictKind::Declared,
                        description: format!(
                            "{} declares a conflict with {id}{}",
                            other.requirement.id,
                            c.note
                                .as_deref()
                                .map(|n| format!(": {n}"))
                                .unwrap_or_default()
                        ),
                        requirement_ids: vec![other.requirement.id.clone(), id.clone()],
                        citations: cite([&other.requirement.id, id]),
                        provenance: vec![],
                    });
                }
            }
        }
        out
    }

    fn duplicate_conflict(
        what: &str,
        primary: &Provenance,
        others: &[Provenance],
    ) -> Option<Conflict> {
        let differing: Vec<&Provenance> = others
            .iter()
            .filter(|o| o.sha256 != primary.sha256)
            .collect();
        if differing.is_empty() {
            return None;
        }
        Some(Conflict {
            kind: ConflictKind::DuplicateDefinition,
            description: format!(
                "{what} is defined differently by {} sources of version {}: {}",
                differing.len() + 1,
                primary.version,
                std::iter::once(primary)
                    .chain(differing.iter().copied())
                    .map(|p| p.source_id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            requirement_ids: vec![],
            citations: std::iter::once(primary)
                .chain(differing.iter().copied())
                .map(|p| Citation {
                    source_id: p.source_id.clone(),
                    locator: p.locator.clone(),
                })
                .collect(),
            provenance: std::iter::once(primary).chain(differing).cloned().collect(),
        })
    }

    pub async fn find(
        &self,
        query: &str,
        version: Option<&str>,
        limit: Option<usize>,
    ) -> Result<FindRequirementResponse> {
        let _timer = crate::timing::Timer::start("requirement.find");
        let q = query.trim();
        if q.is_empty() {
            return Err(CoreError::InvalidInput("query must not be empty".into()));
        }
        let version = version
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(SpecificationVersion::new)
            .transpose()?;
        let limit = self.spec.clamp_limit(limit);
        let all = match &version {
            Some(v) => self.spec.catalog().requirements_by_version(v).await?,
            None => self.spec.catalog().list_requirements().await?,
        };
        let q_upper = q.to_ascii_uppercase();
        let q_tokens = tokens(q);
        let mut matches: Vec<(u8, RequirementMatch)> = Vec::new();
        for rec in &all {
            let r = &rec.requirement;
            if version.as_ref().is_some_and(|v| &r.version != v) {
                continue;
            }
            let reason = if r.id == q_upper {
                Some((0, "id_exact"))
            } else if r.id.contains(&q_upper) {
                Some((1, "id_partial"))
            } else {
                let hay: BTreeSet<String> = tokens(&format!(
                    "{} {} {} {} {} {}",
                    r.id,
                    r.title,
                    r.description.as_deref().unwrap_or_default(),
                    r.acceptance_criteria.join(" "),
                    r.tags.join(" "),
                    r.endpoints
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(" ")
                ))
                .into_iter()
                .collect();
                (!q_tokens.is_empty()
                    && q_tokens
                        .iter()
                        .all(|t| hay.iter().any(|h| h.starts_with(t.as_str()))))
                .then_some((2, "text"))
            };
            if let Some((rank, reason)) = reason {
                matches.push((
                    rank,
                    RequirementMatch {
                        requirement_id: r.id.clone(),
                        classification: EvidenceClass::AuthoritativeRequirement,
                        version: r.version.clone(),
                        title: r.title.clone(),
                        match_reason: reason.to_owned(),
                        sources: r
                            .sources
                            .iter()
                            .map(|s| Citation {
                                source_id: s.source_id.clone(),
                                locator: s.locator.clone(),
                            })
                            .collect(),
                        related_endpoints: self
                            .endpoint_refs(r)
                            .into_iter()
                            .map(|(e, _)| e.to_string())
                            .collect(),
                        acceptance_criteria: acceptance(rec),
                        mapping: rec.mapping.clone(),
                    },
                ));
            }
        }
        matches.sort_by(|a, b| (a.0, &a.1.requirement_id).cmp(&(b.0, &b.1.requirement_id)));
        let requirements: Vec<RequirementMatch> =
            matches.into_iter().take(limit).map(|(_, m)| m).collect();
        let discovered_evidence = self
            .spec
            .search(
                q,
                version.as_ref().map(SpecificationVersion::as_str),
                None,
                None,
                Some(limit),
            )
            .await
            .map(|r| r.results)
            .or_else(|e| {
                if e.is_client_error() {
                    Ok(Vec::new())
                } else {
                    Err(e)
                }
            })?;
        Ok(FindRequirementResponse {
            query: q.to_owned(),
            notice: format!(
                "'requirements' contains curated requirement mappings only (authoritative_requirement). \
                 {EVIDENCE_NOTICE} No requirement id is ever generated from search hits."
            ),
            requirements,
            discovered_evidence,
        })
    }

    /// Endpoints linked by a requirement: explicit list plus `op:`/`path:` source locators.
    fn endpoint_refs(&self, r: &Requirement) -> Vec<(EndpointRef, String)> {
        let mut out: Vec<(EndpointRef, String)> = r
            .endpoints
            .iter()
            .map(|e| (e.clone(), "endpoints".to_owned()))
            .collect();
        for s in &r.sources {
            if let Ok(SourceLocator::Operation { method, path }) = s.locator.parse() {
                out.push((
                    EndpointRef { method, path },
                    format!("source:{} {}", s.source_id, s.locator),
                ));
            }
        }
        out
    }

    fn path_refs(r: &Requirement) -> Vec<(String, String)> {
        r.sources
            .iter()
            .filter_map(|s| match s.locator.parse() {
                Ok(SourceLocator::Path(p)) => {
                    Some((p, format!("source:{} {}", s.source_id, s.locator)))
                }
                _ => None,
            })
            .collect()
    }

    pub async fn trace(&self, requirement_id: &str) -> Result<RequirementTrace> {
        let _timer = crate::timing::Timer::start("requirement.trace");
        let rec = self.requirement(requirement_id).await?;
        let all = self.conflict_partners(&rec).await?;
        let req = &rec.requirement;
        let resolved = self.resolve_sources(&rec).await?;
        let mut conflicts = resolved.conflicts;
        conflicts.extend(self.declared_conflicts(&rec, &all));

        // Endpoints.
        let mut endpoints = Vec::new();
        let mut ops: Vec<OpenApiOperation> = Vec::new();
        for (e, via) in self.endpoint_refs(req) {
            let lookup = if let Some(sid) = via
                .strip_prefix("source:")
                .and_then(|v| v.split_whitespace().next())
            {
                self.spec
                    .catalog()
                    .get_source_operation(sid, &e.method, &e.path)
                    .await?
                    .map(|op| (op, Vec::new()))
            } else {
                self.spec
                    .lookup_operation(&req.version, &e.method, &e.path)
                    .await?
                    .map(|l| (l.primary, l.others))
            };
            match lookup {
                Some((op, others)) => {
                    let other_prov: Vec<Provenance> =
                        others.iter().map(|o| o.provenance.clone()).collect();
                    conflicts.extend(Self::duplicate_conflict(
                        &format!("operation {e}"),
                        &op.provenance,
                        &other_prov,
                    ));
                    endpoints.push(TracedEndpoint {
                        endpoint: e.to_string(),
                        via,
                        found: true,
                        operation: Some(OperationSummary::from(&op)),
                        provenance: Some(op.provenance.clone()),
                    });
                    ops.push(op);
                }
                None => {
                    // Already reported by resolve_sources for `source:` links.
                    if via == "endpoints" {
                        conflicts.push(Conflict {
                            kind: ConflictKind::DanglingReference,
                            description: format!(
                                "{} links endpoint {e} which is not defined in version {}",
                                req.id, req.version
                            ),
                            requirement_ids: vec![req.id.clone()],
                            citations: vec![],
                            provenance: vec![],
                        });
                    }
                    endpoints.push(TracedEndpoint {
                        endpoint: e.to_string(),
                        via,
                        found: false,
                        operation: None,
                        provenance: None,
                    });
                }
            }
        }
        for (path, via) in Self::path_refs(req) {
            let sid = via
                .trim_start_matches("source:")
                .split_whitespace()
                .next()
                .unwrap_or_default()
                .to_owned();
            for op in self
                .spec
                .catalog()
                .source_operations_by_path(&sid, &path)
                .await?
            {
                if ops.iter().any(|o| o.provenance == op.provenance) {
                    continue;
                }
                endpoints.push(TracedEndpoint {
                    endpoint: format!("{} {}", op.method, op.path),
                    via: via.clone(),
                    found: true,
                    operation: Some(OperationSummary::from(&op)),
                    provenance: Some(op.provenance.clone()),
                });
                ops.push(op);
            }
        }

        // Schemas: explicit, schema: locators, operation references, transitive closure.
        let mut schemas: Vec<TracedSchema> = Vec::new();
        let mut seen = BTreeSet::new();
        let mut roots: Vec<(String, String, Option<String>)> = req
            .schemas
            .iter()
            .map(|s| (s.clone(), "explicit".to_owned(), None))
            .collect();
        for s in &req.sources {
            if let Ok(SourceLocator::Schema(n)) = s.locator.parse() {
                roots.push((
                    n,
                    format!("source:{}", s.source_id),
                    Some(s.source_id.clone()),
                ));
            }
        }
        for op in &ops {
            for n in &op.referenced_schemas {
                roots.push((
                    n.clone(),
                    format!("operation:{} {}", op.method, op.path),
                    Some(op.provenance.source_id.clone()),
                ));
            }
        }
        for (name, via, sid) in roots {
            if !seen.insert(name.clone()) {
                continue;
            }
            let found = match &sid {
                Some(sid) => self.spec.catalog().get_source_schema(sid, &name).await?,
                None => self
                    .spec
                    .catalog()
                    .find_schemas(&req.version, &name)
                    .await?
                    .into_iter()
                    .next(),
            };
            match found {
                Some(s) => {
                    let closure = self
                        .spec
                        .schema_closure(
                            &s.provenance.source_id,
                            &req.version,
                            s.referenced_schemas.iter().cloned(),
                        )
                        .await?;
                    schemas.push(TracedSchema {
                        name: name.clone(),
                        via,
                        found: true,
                        referenced_schemas: s.referenced_schemas.clone(),
                        provenance: Some(s.provenance.clone()),
                    });
                    for (n, t) in closure.schemas {
                        if seen.insert(n.clone()) {
                            schemas.push(TracedSchema {
                                name: n,
                                via: format!("transitive:{name}"),
                                found: true,
                                referenced_schemas: t.referenced_schemas.clone(),
                                provenance: Some(t.provenance),
                            });
                        }
                    }
                }
                None => {
                    if via == "explicit" {
                        conflicts.push(Conflict {
                            kind: ConflictKind::DanglingReference,
                            description: format!(
                                "{} links schema '{name}' which is not defined in version {}",
                                req.id, req.version
                            ),
                            requirement_ids: vec![req.id.clone()],
                            citations: vec![],
                            provenance: vec![],
                        });
                    }
                    schemas.push(TracedSchema {
                        name,
                        via,
                        found: false,
                        referenced_schemas: vec![],
                        provenance: None,
                    });
                }
            }
        }

        let related_evidence = self
            .spec
            .related_evidence(
                &format!(
                    "{} {}",
                    req.title,
                    req.description.as_deref().unwrap_or_default()
                ),
                &req.version,
                5,
            )
            .await?;
        dedupe_conflicts(&mut conflicts);
        Ok(RequirementTrace {
            requirement_id: req.id.clone(),
            classification: EvidenceClass::AuthoritativeRequirement,
            acceptance_criteria: acceptance(&rec),
            requirement: rec.requirement.clone(),
            mapping: rec.mapping.clone(),
            sources: resolved.sources,
            endpoints,
            schemas,
            related_evidence,
            conflicts,
            notice: "The requirement and acceptance criteria come from the curated mapping. \
                     'related_evidence' is discovered evidence only."
                .to_owned(),
        })
    }

    /// Builds the endpoint evidence bundle (steps 1-7 of the Requirements Agent workflow).
    pub async fn endpoint_requirements(
        &self,
        version: Option<&str>,
        path: &str,
        method: &str,
        evidence_limit: Option<usize>,
    ) -> Result<EndpointEvidenceBundle> {
        let _timer = crate::timing::Timer::start("requirement.endpoint_requirements");
        let version = self.spec.version_or_target(version)?;
        self.spec.ensure_known_version(&version).await?;
        let method = normalize_method(method)?;
        let path = validate_path(path)?;
        let mut conflicts = Vec::new();
        let mut citations: Vec<SourceCitation> = Vec::new();

        // 1. OpenAPI operation.
        let lookup = self.spec.lookup_operation(&version, &method, &path).await?;
        let key = match &lookup {
            Some(l) => self.spec.path_key_for(&version, &l.primary.path),
            None => self.spec.path_key_for(&version, &path),
        };
        if let Some(l) = &lookup {
            let others: Vec<Provenance> = l.others.iter().map(|o| o.provenance.clone()).collect();
            conflicts.extend(Self::duplicate_conflict(
                &format!("operation {method} {}", l.primary.path),
                &l.primary.provenance,
                &others,
            ));
            citations.push(cite(CitationRole::OpenapiOperation, &l.primary.provenance));
        }

        // 2. Referenced schemas (transitive).
        let (schemas, unresolved, cross_source_schemas) = match &lookup {
            Some(l) => {
                let c = self
                    .spec
                    .schema_closure(
                        &l.primary.provenance.source_id,
                        &version,
                        l.primary.referenced_schemas.iter().cloned(),
                    )
                    .await?;
                (
                    c.schemas.into_values().collect::<Vec<_>>(),
                    c.unresolved.into_iter().collect::<Vec<_>>(),
                    c.cross_source,
                )
            }
            None => (Vec::new(), Vec::new(), Vec::new()),
        };
        citations.extend(
            schemas
                .iter()
                .map(|s| cite(CitationRole::OpenapiSchema, &s.provenance)),
        );

        // 3. Curated requirements linked to this endpoint (indexed candidates, exact match below).
        let candidates = self
            .spec
            .catalog()
            .requirements_for_method(&version, &method)
            .await?;
        let mut requirements = Vec::new();
        for rec in &candidates {
            let r = &rec.requirement;
            if r.version != version {
                continue;
            }
            let mut linked_via: Vec<String> = self
                .endpoint_refs(r)
                .into_iter()
                .filter(|(e, _)| {
                    e.method == method
                        && self.spec.path_key_for(&version, &e.path) == key
                        && lookup.as_ref().is_none_or(|l| l.denotes(&e.path))
                })
                .map(|(_, via)| via)
                .collect();
            linked_via.extend(
                Self::path_refs(r)
                    .into_iter()
                    .filter(|(p, _)| {
                        self.spec.path_key_for(&version, p) == key
                            && lookup.as_ref().is_none_or(|l| l.denotes(p))
                    })
                    .map(|(_, v)| v),
            );
            if linked_via.is_empty() {
                continue;
            }
            linked_via.sort();
            linked_via.dedup();
            let resolved = self.resolve_sources(rec).await?;
            conflicts.extend(resolved.conflicts);
            let partners = self.conflict_partners(rec).await?;
            conflicts.extend(self.declared_conflicts(rec, &partners));
            for s in &resolved.sources {
                if s.provenance.is_empty() {
                    citations.push(SourceCitation {
                        role: CitationRole::RequirementSource,
                        source_id: s.source_id.clone(),
                        locator: s.locator.clone(),
                        sha256: None,
                        document_sha256: None,
                        authority: None,
                        precedence: None,
                    });
                }
                citations.extend(
                    s.provenance
                        .iter()
                        .map(|p| cite(CitationRole::RequirementSource, p)),
                );
            }
            requirements.push(EndpointRequirement {
                requirement_id: r.id.clone(),
                classification: EvidenceClass::AuthoritativeRequirement,
                title: r.title.clone(),
                description: r.description.clone(),
                linked_via,
                acceptance_criteria: acceptance(rec),
                sources: resolved.sources,
                mapping: rec.mapping.clone(),
            });
        }

        if lookup.is_none() {
            if requirements.is_empty() {
                return Err(CoreError::NotFound(format!(
                    "no OpenAPI operation {method} {path} and no curated requirement for it in version '{version}'"
                )));
            }
            conflicts.push(Conflict {
                kind: ConflictKind::DanglingReference,
                description: format!("curated requirements reference {method} {path} but no OpenAPI operation defines it in version {version}"),
                requirement_ids: requirements.iter().map(|r| r.requirement_id.clone()).collect(),
                citations: vec![],
                provenance: vec![],
            });
        }

        // 4. Related PDF/text evidence.
        let query = match &lookup {
            Some(l) => {
                let op = &l.primary;
                let op_id = op
                    .operation_id
                    .as_deref()
                    .map(split_camel)
                    .unwrap_or_default();
                let segs: Vec<&str> = op
                    .path
                    .split('/')
                    .filter(|s| !s.is_empty() && !s.starts_with('{'))
                    .collect();
                format!(
                    "{} {} {} {}",
                    op.summary.as_deref().unwrap_or_default(),
                    op_id,
                    segs.join(" "),
                    op.tags.join(" ")
                )
            }
            None => path.replace(['/', '{', '}'], " "),
        };
        let related_evidence = self
            .spec
            .related_evidence(
                &query,
                &version,
                evidence_limit.unwrap_or(DEFAULT_EVIDENCE_LIMIT),
            )
            .await?;
        citations.extend(related_evidence.iter().map(|e| SourceCitation {
            role: CitationRole::DiscoveredEvidence,
            source_id: e.source_id.clone(),
            locator: e.locator.clone(),
            sha256: Some(e.sha256.clone()),
            document_sha256: None,
            authority: Some(e.authority),
            precedence: None,
        }));

        // 5./6. Citations and conflicts, deterministic order.
        let mut seen = BTreeSet::new();
        citations.retain(|c| {
            seen.insert((
                c.role,
                c.source_id.clone(),
                c.locator.clone(),
                c.sha256.clone(),
            ))
        });
        dedupe_conflicts(&mut conflicts);

        // 7. Bundle.
        Ok(EndpointEvidenceBundle {
            endpoint: EndpointDescriptor {
                version,
                method,
                requested_path: path,
                found: lookup.is_some(),
                matched_path: lookup.as_ref().map(|l| l.primary.path.clone()),
                matched_by: lookup.as_ref().map(|l| l.matched_by),
                ambiguous: lookup.as_ref().is_some_and(|l| l.ambiguous),
                other_templates: lookup
                    .as_ref()
                    .map(|l| {
                        l.other_templates
                            .iter()
                            .map(|o| o.provenance.clone())
                            .collect()
                    })
                    .unwrap_or_default(),
            },
            notice: format!(
                "Only entries in 'requirements' (classification authoritative_requirement) are curated requirements. {EVIDENCE_NOTICE} \
                 Conflicts are reported, never resolved."
            ),
            openapi: lookup.map(|l| l.primary),
            requirements,
            schemas,
            unresolved_schemas: unresolved,
            cross_source_schemas,
            related_evidence,
            conflicts,
            sources: citations,
        })
    }
}

fn cite(role: CitationRole, p: &Provenance) -> SourceCitation {
    SourceCitation {
        role,
        source_id: p.source_id.clone(),
        locator: p.locator.clone(),
        sha256: Some(p.sha256.clone()),
        document_sha256: Some(p.document_sha256.clone()),
        authority: Some(p.authority),
        precedence: Some(p.precedence),
    }
}

/// `getTransactionList` -> `get Transaction List`.
fn split_camel(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for (i, c) in s.chars().enumerate() {
        if i > 0 && c.is_uppercase() {
            out.push(' ');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camel_split() {
        assert_eq!(split_camel("getTransactionList"), "get Transaction List");
    }

    #[test]
    fn excerpt_is_bounded() {
        let s = "é".repeat(2000);
        assert!(excerpt(&s).len() <= EXCERPT_CHARS + 4);
    }
}
