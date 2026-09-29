//! Smoke test against the official Berlin Group files in the workspace `corpus/`.
//! Ignored by default (large PDFs); run with
//! `cargo test -p bg-spec-indexer --release --test real_corpus -- --ignored`.

use bg_spec_core::{
    domain::ConflictKind,
    services::{ServiceSettings, Services, SourceResolution},
};
use bg_spec_indexer::{IndexAction, IndexOptions, Indexer, testing};

#[tokio::test]
#[ignore = "indexes the full official corpus"]
async fn real_corpus_indexes_and_every_requirement_resolves() {
    let dir = tempfile::tempdir().expect("tempdir");
    let config =
        testing::stage_corpus(&testing::workspace_corpus_dir(), dir.path()).expect("stage corpus");
    let report = Indexer::new(config.clone())
        .run(IndexOptions::default())
        .await
        .expect("index");
    assert_eq!(report.count(IndexAction::Indexed), 28, "{report:#?}");
    assert_eq!(report.requirements, 14);

    let (catalog, search) = bg_spec_store::open_read_only(&config)
        .await
        .expect("open read-only");
    let svc = Services::new(catalog, search, ServiceSettings::from_config(&config));

    let file = std::fs::read_to_string(config.corpus_root.join("requirements/requirements.yaml"))
        .expect("requirements.yaml");
    let ids: Vec<&str> = file
        .lines()
        .filter_map(|l| l.trim().strip_prefix("- id: "))
        .collect();
    assert_eq!(ids.len(), 14);
    for id in ids {
        let trace = svc.requirements.trace(id).await.unwrap();
        for s in &trace.sources {
            assert_eq!(
                s.status,
                SourceResolution::Resolved,
                "{id}: {} {}",
                s.source_id,
                s.locator
            );
        }
        assert!(
            trace
                .conflicts
                .iter()
                .all(|c| c.kind == ConflictKind::Declared),
            "{id}: {:#?}",
            trace.conflicts
        );
    }

    let sca = svc
        .requirements
        .trace("OFV2-SCA-APPROACH-001")
        .await
        .unwrap();
    assert!(
        sca.conflicts
            .iter()
            .any(|c| c.kind == ConflictKind::Declared)
    );

    // v1 and v2 operations match through the configured /v1 and /v2 prefixes.
    for path in [
        "/accounts/{account-id}/transactions",
        "/v2/accounts/{account-id}/transactions",
        "/v1/accounts/{account-id}/balances",
    ] {
        let cmp = svc
            .compatibility
            .compare(path, "GET", None, None)
            .await
            .unwrap();
        assert!(
            cmp.from.found && cmp.to.found,
            "{path}: {:?} / {:?}",
            cmp.from,
            cmp.to
        );
    }
    let payment = svc
        .compatibility
        .compare(
            "/v1/{payment-service}/{payment-product}/{paymentId}",
            "GET",
            None,
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        payment.v2_operation.map(|o| o.path).as_deref(),
        Some("/v2/{payment-service}/{payment-product}/{paymentId}")
    );
    let bundle = svc
        .requirements
        .endpoint_requirements(
            Some("openfinance-v2"),
            "/accounts/{accountId}/transactions",
            "GET",
            None,
        )
        .await
        .unwrap();
    let ids: Vec<_> = bundle
        .requirements
        .iter()
        .map(|r| r.requirement_id.as_str())
        .collect();
    assert!(ids.contains(&"OFV2-AIS-TRANSACTIONS-001"), "{ids:?}");
}
