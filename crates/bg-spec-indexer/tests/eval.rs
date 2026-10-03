//! Deterministic retrieval gates. Gold locators identify evidence, not normative conclusions.
use bg_spec_core::services::{ServiceSettings, Services, SourceResolution};
use bg_spec_indexer::{IndexOptions, Indexer, testing};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Gold {
    searches: Vec<SearchCase>,
    endpoints: Vec<EndpointCase>,
    requirements: Vec<RequirementCase>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchCase {
    query: String,
    version: String,
    source_id: String,
    locator: String,
    /// Worst acceptable 1-based rank; defaults to the evaluation cut-off.
    #[serde(default)]
    max_rank: Option<usize>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EndpointCase {
    version: String,
    path: String,
    source_id: String,
    resolved_path: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RequirementCase {
    id: String,
    source_id: String,
    locator: String,
}

async fn evaluate(official: bool) {
    let dir = tempfile::tempdir().unwrap();
    let corpus = if official {
        testing::workspace_corpus_dir()
    } else {
        testing::example_corpus_dir()
    };
    let config = testing::stage_corpus(&corpus, dir.path()).unwrap();
    Indexer::new(config.clone())
        .run(IndexOptions::default())
        .await
        .unwrap();
    let (catalog, search) = bg_spec_store::open_read_only(&config).await.unwrap();
    let svc = Services::new(catalog, search, ServiceSettings::from_config(&config));
    let gold: Gold = serde_saphyr::from_str(if official {
        include_str!("fixtures/eval-official.yaml")
    } else {
        include_str!("fixtures/eval.yaml")
    })
    .unwrap();
    assert!(!gold.searches.is_empty());
    assert!(!gold.endpoints.is_empty());
    assert!(!gold.requirements.is_empty());
    let k = 5;
    let mut found = 0;
    let mut reciprocal_rank = 0.0;
    let mut duplicates = 0;
    for case in &gold.searches {
        let response = svc
            .specification
            .search(&case.query, Some(&case.version), None, None, Some(k))
            .await
            .unwrap();
        let rank = response.results.iter().position(|hit| {
            (hit.source_id == case.source_id && hit.locator == case.locator)
                || hit
                    .also_found_in
                    .iter()
                    .any(|d| d.source_id == case.source_id && d.locator == case.locator)
        });
        if let Some(rank) = rank {
            found += 1;
            reciprocal_rank += 1.0 / (rank + 1) as f64;
        }
        assert!(
            rank.is_some(),
            "missing gold evidence for {}; top hits: {:#?}",
            case.query,
            response
                .results
                .iter()
                .map(|h| (h.locator.as_str(), h.relevance, &h.section_path))
                .collect::<Vec<_>>()
        );
        if let (Some(rank), Some(max)) = (rank, case.max_rank) {
            assert!(rank < max, "{} ranked {} (max {max})", case.query, rank + 1);
        }
        // Hits repeating earlier evidence: identical content, or another chunk of the same page section.
        let mut seen = std::collections::BTreeSet::new();
        for hit in &response.results {
            let content = (hit.version.to_string(), hit.sha256.clone());
            let page = (
                hit.source_id.clone(),
                hit.page
                    .map(|p| format!("{p}|{}", hit.title))
                    .unwrap_or_else(|| hit.locator.clone()),
            );
            let fresh_content = seen.insert(content);
            let fresh_page = seen.insert(page);
            if !fresh_content || !fresh_page {
                duplicates += 1;
            }
        }
        for hit in &response.results {
            let evidence = svc
                .specification
                .read_source(&hit.source_id, &hit.locator)
                .await
                .unwrap();
            assert!(
                evidence.provenance.iter().any(|p| p.sha256 == hit.sha256),
                "search citation does not resolve: {} {}",
                hit.source_id,
                hit.locator
            );
        }
    }
    let n = gold.searches.len() as f64;
    let recall = found as f64 / n;
    // Exactly one judged relevant locator per query; unjudged hits are not treated as relevant.
    let precision = found as f64 / (n * k as f64);
    let mrr = reciprocal_rank / n;
    assert_eq!(recall, 1.0);
    assert!(mrr >= if official { 0.8 } else { 1.0 });
    assert_eq!(precision, 0.2);
    for case in &gold.endpoints {
        let response = svc
            .specification
            .read_endpoint(Some(&case.version), &case.path, "GET")
            .await
            .unwrap();
        assert_eq!(response.operation.provenance.source_id, case.source_id);
        assert_eq!(response.operation.path, case.resolved_path);
    }
    for case in &gold.requirements {
        let trace = svc.requirements.trace(&case.id).await.unwrap();
        assert!(trace.sources.iter().any(|s| {
            s.source_id == case.source_id
                && s.locator == case.locator
                && s.status == SourceResolution::Resolved
        }));
    }
    println!(
        "recall@{k}={recall:.3} judged_precision@{k}={precision:.3} \
         MRR@{k}={mrr:.3} duplicate_hits@{k}={duplicates} \
         exact_lookup_accuracy=1.000 source_resolution_accuracy=1.000"
    );
}

#[tokio::test]
async fn fixture_retrieval_baseline() {
    evaluate(false).await;
}

#[tokio::test]
#[ignore = "indexes the official corpus; run with --release --test eval -- --ignored --nocapture"]
async fn official_retrieval_baseline() {
    evaluate(true).await;
}
