//! End-to-end tests: stage the example corpus, index it and exercise every service through the
//! real SQLite catalog and Tantivy index.

use bg_spec_core::{
    CoreError,
    domain::{ConflictKind, DocumentStatus, EvidenceClass},
    services::{ServiceSettings, Services, SourceContent},
};
use bg_spec_indexer::{IndexAction, IndexOptions, Indexer, testing};
use serde_json::Value;

const ENDPOINT: &str = "/accounts/{accountId}/transactions";

struct Env {
    _dir: tempfile::TempDir,
    config: bg_spec_core::config::Config,
}

async fn indexed() -> Env {
    let dir = tempfile::tempdir().expect("tempdir");
    let config = testing::stage_example_corpus(dir.path()).expect("stage corpus");
    let report = Indexer::new(config.clone())
        .run(IndexOptions::default())
        .await
        .expect("index");
    assert_eq!(report.count(IndexAction::Indexed), 6, "{report:#?}");
    Env { _dir: dir, config }
}

async fn services(env: &Env) -> Services {
    let (catalog, search) = bg_spec_store::open_read_only(&env.config)
        .await
        .expect("open read-only");
    Services::new(catalog, search, ServiceSettings::from_config(&env.config))
}

/// Removes volatile values (index timestamps) so snapshots are deterministic.
fn stable(v: impl serde::Serialize) -> Value {
    fn walk(v: &mut Value) {
        match v {
            Value::Object(m) => {
                for (k, x) in m.iter_mut() {
                    if k == "indexed_at_unix" || k == "last_indexed_at_unix" {
                        *x = Value::from("[timestamp]");
                    } else {
                        walk(x);
                    }
                }
            }
            Value::Array(a) => a.iter_mut().for_each(walk),
            _ => {}
        }
    }
    let mut v = serde_json::to_value(v).expect("serialize");
    walk(&mut v);
    v
}

#[test]
fn committed_fixture_pdfs_are_reproducible() {
    for pdf in bg_spec_indexer::pdf::fixture_corpus::fixture_pdfs() {
        let committed =
            std::fs::read(testing::example_corpus_dir().join(pdf.path)).expect("committed fixture");
        assert_eq!(
            pdf.render(),
            committed,
            "{} differs from generator output",
            pdf.path
        );
    }
}

#[tokio::test]
async fn incremental_indexing_skips_unchanged_and_reindexes_changed() {
    let env = indexed().await;
    let indexer = Indexer::new(env.config.clone());

    let again = indexer.run(IndexOptions::default()).await.unwrap();
    assert_eq!(again.count(IndexAction::Skipped), 6);
    assert!(!again.full_rebuild);

    let errata = env.config.corpus_root.join("text/v2/errata.md");
    let mut text = std::fs::read_to_string(&errata).unwrap();
    text.push_str(
        "\n## E-99 Additional clarification\n\nThe ASPSP SHALL document this clarification.\n",
    );
    std::fs::write(&errata, text).unwrap();
    let changed = indexer.run(IndexOptions::default()).await.unwrap();
    assert_eq!(changed.count(IndexAction::Indexed), 1);
    assert_eq!(changed.count(IndexAction::Skipped), 5);

    let forced = indexer.run(IndexOptions { force: true }).await.unwrap();
    assert_eq!(forced.count(IndexAction::Indexed), 6);

    let svc = services(&env).await;
    let hit = svc
        .specification
        .search("E-99 clarification", None, None, None, Some(5))
        .await
        .unwrap();
    assert!(
        hit.results
            .iter()
            .any(|r| r.source_id == "bg-openfinance-v2-errata")
    );
}

#[tokio::test]
async fn removed_manifest_entries_are_dropped_from_both_stores() {
    let env = indexed().await;
    let manifest = env.config.corpus_root.join("manifest.yaml");
    let raw = std::fs::read_to_string(&manifest).unwrap();
    let start = raw
        .find("  - id: bg-openfinance-v2-errata")
        .expect("errata entry");
    let end = raw[start + 4..]
        .find("\n  - id:")
        .map(|i| start + 4 + i + 1)
        .unwrap_or(raw.len());
    std::fs::write(&manifest, format!("{}{}", &raw[..start], &raw[end..])).unwrap();

    let report = Indexer::new(env.config.clone())
        .run(IndexOptions::default())
        .await
        .unwrap();
    assert_eq!(report.count(IndexAction::Removed), 1, "{report:#?}");
    let svc = services(&env).await;
    let hits = svc
        .specification
        .search(
            "consent",
            None,
            None,
            Some("bg-openfinance-v2-errata"),
            None,
        )
        .await;
    assert!(matches!(hits, Err(CoreError::NotFound(_))) || hits.unwrap().results.is_empty());
}

#[tokio::test]
async fn ocr_required_pages_are_reported_not_emitted_as_evidence() {
    let env = indexed().await;
    let svc = services(&env).await;
    let listing = svc
        .specification
        .list_sources(Some("openfinance-v2"), Some("pdf"))
        .await
        .unwrap();
    let ig = listing
        .sources
        .iter()
        .find(|s| s.source_id == "bg-openfinance-v2-implementation-guidelines")
        .unwrap();
    assert_eq!(ig.status, DocumentStatus::Partial);
    assert_eq!(ig.ocr_required_pages, vec![4]);

    let page = svc
        .specification
        .read_source(&ig.source_id, "page:4")
        .await
        .unwrap();
    let json = serde_json::to_value(&page).unwrap();
    assert_eq!(json["content"]["pages"][0]["status"], "ocr_required");
    assert!(
        json["content"]["pages"][0]["warning"]
            .as_str()
            .unwrap()
            .contains("no extractable text")
    );

    let hits = svc
        .specification
        .search("transaction", None, None, Some(&ig.source_id), Some(50))
        .await
        .unwrap();
    assert!(hits.results.iter().all(|r| r.page != Some(4)));
}

#[tokio::test]
async fn search_filters_and_provenance() {
    let env = indexed().await;
    let svc = services(&env).await;
    let res = svc
        .specification
        .search(
            "transaction list dateFrom",
            Some("openfinance-v2"),
            Some("pdf"),
            None,
            Some(5),
        )
        .await
        .unwrap();
    assert!(!res.results.is_empty());
    for r in &res.results {
        assert_eq!(r.version.as_str(), "openfinance-v2");
        assert_eq!(r.kind.to_string(), "pdf");
        assert_eq!(r.classification, EvidenceClass::DiscoveredEvidence);
        assert!(!r.sha256.is_empty() && !r.locator.is_empty() && !r.document_id.is_empty());
    }
    assert!(res.notice.contains("NOT authoritative"));

    let openapi = svc
        .specification
        .search(
            "getTransactionList",
            Some("nextgenpsd2-v1.3"),
            Some("openapi"),
            None,
            None,
        )
        .await
        .unwrap();
    assert!(
        openapi
            .results
            .iter()
            .any(|r| r.locator.starts_with("op:GET /v1/accounts"))
    );

    let only = svc
        .specification
        .search(
            "transaction",
            None,
            None,
            Some("bg-openfinance-v2-operational-rules"),
            None,
        )
        .await
        .unwrap();
    assert!(
        only.results
            .iter()
            .all(|r| r.source_id == "bg-openfinance-v2-operational-rules")
    );

    assert!(matches!(
        svc.specification.search("  ", None, None, None, None).await,
        Err(CoreError::InvalidInput(_))
    ));
    assert!(
        svc.specification
            .search("x", Some("bogus-v9"), None, None, None)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn read_source_locators() {
    let env = indexed().await;
    let svc = services(&env).await;
    let ig = "bg-openfinance-v2-implementation-guidelines";

    let page = svc.specification.read_source(ig, "page:3").await.unwrap();
    let SourceContent::Pages { pages } = &page.content else {
        panic!("expected pages")
    };
    assert!(pages[0].text.contains("dateFrom is mandatory"));
    assert!(!page.provenance.is_empty());

    let range = svc.specification.read_source(ig, "page:2-3").await.unwrap();
    let SourceContent::Pages { pages } = &range.content else {
        panic!("expected pages")
    };
    assert_eq!(pages.iter().map(|p| p.page).collect::<Vec<_>>(), vec![2, 3]);

    let section = svc
        .specification
        .read_source(ig, "section:4.2")
        .await
        .unwrap();
    let SourceContent::Chunks { chunks } = &section.content else {
        panic!("expected chunks")
    };
    assert!(
        chunks
            .iter()
            .all(|c| c.section.as_deref().is_some_and(|s| s.starts_with("4.2 ")))
    );

    let chunk_id = &chunks[0].chunk_id;
    let by_chunk = svc
        .specification
        .read_source(ig, &format!("chunk:{chunk_id}"))
        .await
        .unwrap();
    assert_eq!(by_chunk.provenance[0].sha256, chunks[0].provenance.sha256);

    let errata = svc
        .specification
        .read_source("bg-openfinance-v2-errata", "section:E-07")
        .await
        .unwrap();
    assert!(
        serde_json::to_string(&errata)
            .unwrap()
            .contains("per consent only")
    );

    let op = svc
        .specification
        .read_source("bg-openfinance-v2-openapi", &format!("op:GET {ENDPOINT}"))
        .await
        .unwrap();
    assert!(matches!(op.content, SourceContent::Operations { .. }));
    let path = svc
        .specification
        .read_source("bg-openfinance-v2-openapi", &format!("path:{ENDPOINT}"))
        .await
        .unwrap();
    assert!(!path.provenance.is_empty());
    let schema = svc
        .specification
        .read_source("bg-openfinance-v2-openapi", "schema:Transactions")
        .await
        .unwrap();
    assert!(matches!(schema.content, SourceContent::Schema { .. }));

    // Errors: unknown source, filesystem path, malformed and out-of-range locators.
    assert!(matches!(
        svc.specification.read_source("nope", "page:1").await,
        Err(CoreError::NotFound(_))
    ));
    assert!(
        svc.specification
            .read_source("/etc/passwd", "page:1")
            .await
            .is_err()
    );
    assert!(
        svc.specification
            .read_source("../corpus/manifest.yaml", "page:1")
            .await
            .is_err()
    );
    assert!(matches!(
        svc.specification.read_source(ig, "line:7").await,
        Err(CoreError::InvalidLocator { .. })
    ));
    assert!(matches!(
        svc.specification.read_source(ig, "page:99").await,
        Err(CoreError::NotFound(_))
    ));
}

#[tokio::test]
async fn endpoint_and_schema_lookup() {
    let env = indexed().await;
    let svc = services(&env).await;
    let ep = svc
        .specification
        .read_endpoint(None, ENDPOINT, "get")
        .await
        .unwrap();
    assert_eq!(ep.version.as_str(), "openfinance-v2");
    assert_eq!(
        ep.operation.operation_id.as_deref(),
        Some("getTransactionList")
    );
    assert_eq!(
        ep.operation.provenance.locator,
        format!("op:GET {ENDPOINT}")
    );
    insta::assert_json_snapshot!("v2_transactions_operation", stable(&ep));

    // v1 lookup through the configured `/v1` prefix and canonical parameter names.
    let v1 = svc
        .specification
        .read_endpoint(Some("nextgenpsd2-v1.3"), ENDPOINT, "GET")
        .await
        .unwrap();
    assert_eq!(v1.operation.path, "/v1/accounts/{account-id}/transactions");

    let schema = svc
        .specification
        .read_schema(None, "Transactions")
        .await
        .unwrap();
    assert_eq!(schema.schema.provenance.locator, "schema:Transactions");
    assert!(
        schema
            .transitive_referenced_schemas
            .contains(&"Amount".to_owned())
    );

    assert!(matches!(
        svc.specification.read_endpoint(None, "/nope", "GET").await,
        Err(CoreError::NotFound(_))
    ));
    assert!(matches!(
        svc.specification
            .read_endpoint(None, ENDPOINT, "FETCH")
            .await,
        Err(CoreError::InvalidInput(_))
    ));
    assert!(matches!(
        svc.specification.read_schema(None, "Nope").await,
        Err(CoreError::NotFound(_))
    ));
}

#[tokio::test]
async fn requirement_find_and_trace() {
    let env = indexed().await;
    let svc = services(&env).await;
    let found = svc
        .requirements
        .find("OFV2-TRANSACTIONS-001", None, None)
        .await
        .unwrap();
    let json = serde_json::to_value(&found).unwrap();
    assert_eq!(
        json["requirements"][0]["requirement_id"],
        "OFV2-TRANSACTIONS-001"
    );
    assert_eq!(
        json["requirements"][0]["classification"],
        "authoritative_requirement"
    );

    let free = svc
        .requirements
        .find("standing order information", None, None)
        .await
        .unwrap();
    let json = serde_json::to_value(&free).unwrap();
    for e in json["discovered_evidence"].as_array().unwrap() {
        assert_eq!(e["classification"], "discovered_evidence");
    }
    let none = svc
        .requirements
        .find("OFV2-DOES-NOT-EXIST-999", None, None)
        .await
        .unwrap();
    assert!(
        serde_json::to_value(&none).unwrap()["requirements"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    let trace = svc
        .requirements
        .trace("OFV2-TRANSACTIONS-001")
        .await
        .unwrap();
    assert_eq!(
        trace.classification,
        EvidenceClass::AuthoritativeRequirement
    );
    assert!(!trace.acceptance_criteria.is_empty());
    assert!(!trace.endpoints.is_empty() && !trace.schemas.is_empty());
    assert!(trace.conflicts.is_empty(), "{:#?}", trace.conflicts);
    insta::assert_json_snapshot!("trace_ofv2_transactions_001", stable(&trace));

    assert!(matches!(
        svc.requirements.trace("OFV2-NOPE-1").await,
        Err(CoreError::NotFound(_))
    ));
}

#[tokio::test]
async fn conflicts_are_detected_deterministically() {
    let env = indexed().await;
    let svc = services(&env).await;
    let kinds = |t: &bg_spec_core::services::RequirementTrace| {
        t.conflicts.iter().map(|c| c.kind).collect::<Vec<_>>()
    };

    let t3 = svc
        .requirements
        .trace("OFV2-TRANSACTIONS-003")
        .await
        .unwrap();
    let k3 = kinds(&t3);
    assert!(k3.contains(&ConflictKind::Declared), "{k3:?}");
    assert!(k3.contains(&ConflictKind::PrecedenceTie), "{k3:?}");
    assert!(k3.contains(&ConflictKind::StaleSource), "{k3:?}");
    assert!(t3.conflicts.iter().all(|c| !c.citations.is_empty()));

    let bal = svc.requirements.trace("OFV2-BALANCES-001").await.unwrap();
    assert!(
        kinds(&bal)
            .iter()
            .all(|k| *k == ConflictKind::DanglingReference)
    );
    assert!(kinds(&bal).len() >= 2);
}

#[tokio::test]
async fn v1_v2_comparison_reports_concrete_changes() {
    let env = indexed().await;
    let svc = services(&env).await;
    let report = svc
        .compatibility
        .compare(ENDPOINT, "GET", None, None)
        .await
        .unwrap();
    let json = stable(&report);
    let facts = serde_json::to_string(&json["compatibility_relevant_facts"]).unwrap();
    for needle in [
        "Consent-ID",
        "pageSize",
        "dateFrom",
        "response:429",
        "creditorName",
        "date-time",
        "nullable",
        "BearerAuthOAuth",
    ] {
        assert!(facts.contains(needle), "missing fact about {needle}");
    }
    let text = serde_json::to_string(&json).unwrap().to_lowercase();
    for verdict in ["backward-compatible", "breaking change", "\"compatible\""] {
        assert!(
            !text.contains(verdict),
            "comparison must not contain a verdict ({verdict})"
        );
    }
    insta::assert_json_snapshot!("compare_transactions_v13_v2", json);

    let missing = svc
        .compatibility
        .compare("/accounts/{accountId}/balances", "GET", None, None)
        .await
        .unwrap();
    let j = serde_json::to_value(&missing).unwrap();
    assert_eq!(j["from"]["found"], false);
    assert_eq!(j["to"]["found"], true);
    assert_eq!(j["changes"][0]["change"], "added");

    assert!(matches!(
        svc.compatibility.compare("/nope", "GET", None, None).await,
        Err(CoreError::NotFound(_))
    ));
}

/// Acceptance scenario (spec §29): the full Requirements Agent chain for
/// `GET /accounts/{accountId}/transactions`.
#[tokio::test]
async fn acceptance_scenario_transactions_endpoint() {
    let env = indexed().await;
    let svc = services(&env).await;

    // 1. get_endpoint_requirements
    let bundle = svc
        .requirements
        .endpoint_requirements(Some("openfinance-v2"), ENDPOINT, "GET", None)
        .await
        .unwrap();
    let b = stable(&bundle);
    assert_eq!(b["endpoint"]["found"], true);
    let req_ids: Vec<_> = b["requirements"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["requirement_id"].as_str().unwrap().to_owned())
        .collect();
    assert!(req_ids.contains(&"OFV2-TRANSACTIONS-001".to_owned()));
    for r in b["requirements"].as_array().unwrap() {
        assert_eq!(r["classification"], "authoritative_requirement");
    }
    for e in b["related_evidence"].as_array().unwrap() {
        assert_eq!(e["classification"], "discovered_evidence");
        assert!(e["source_id"].is_string() && e["locator"].is_string() && e["sha256"].is_string());
    }
    assert!(!b["schemas"].as_array().unwrap().is_empty());
    assert!(!b["sources"].as_array().unwrap().is_empty());
    assert!(!b["conflicts"].as_array().unwrap().is_empty());
    insta::assert_json_snapshot!("endpoint_requirements_transactions", b);

    // 2. read_openapi_endpoint
    let ep = svc
        .specification
        .read_endpoint(Some("openfinance-v2"), ENDPOINT, "GET")
        .await
        .unwrap();
    assert!(
        ep.operation
            .parameters
            .iter()
            .any(|p| p.name == "dateFrom" && p.required)
    );

    // 3. trace_requirement
    let trace = svc
        .requirements
        .trace("OFV2-TRANSACTIONS-001")
        .await
        .unwrap();
    let citation = trace
        .sources
        .iter()
        .find(|s| s.locator == "page:3")
        .expect("page citation");

    // 4. read_source for the cited locator
    let src = svc
        .specification
        .read_source(&citation.source_id, &citation.locator)
        .await
        .unwrap();
    assert!(
        serde_json::to_string(&src)
            .unwrap()
            .contains("SHALL reject a\\nrequest without dateFrom")
    );

    // 5. compare_v1_v2
    let cmp = svc
        .compatibility
        .compare(ENDPOINT, "GET", None, None)
        .await
        .unwrap();
    assert!(!cmp.changes.is_empty());
    assert!(cmp.sources.len() >= 2);
}
