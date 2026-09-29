//! MCP protocol tests: a real rmcp client talks to `BgSpecServer` over an in-memory duplex
//! transport backed by an indexed copy of the example corpus.

use bg_spec_core::services::{ServiceSettings, Services};
use bg_spec_indexer::{IndexOptions, Indexer, testing};
use bg_spec_mcp::{BgSpecServer, TOOL_NAMES};
use rmcp::{
    ClientHandler, RoleClient, ServiceExt,
    model::{CallToolRequestParams, CallToolResult, ClientConfig},
    service::RunningService,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::path::Path;

#[derive(Clone, Default)]
struct TestClient;

impl ClientHandler for TestClient {
    fn get_info(&self) -> ClientConfig {
        ClientConfig::default()
    }
}

struct Session {
    dir: tempfile::TempDir,
    client: RunningService<RoleClient, TestClient>,
    server: tokio::task::JoinHandle<()>,
}

async fn session() -> Session {
    let dir = tempfile::tempdir().unwrap();
    let config = testing::stage_example_corpus(dir.path()).unwrap();
    Indexer::new(config.clone())
        .run(IndexOptions::default())
        .await
        .unwrap();
    let (catalog, search) = bg_spec_store::open_read_only(&config).await.unwrap();
    let server = BgSpecServer::new(Services::new(
        catalog,
        search,
        ServiceSettings::from_config(&config),
    ));

    let (server_io, client_io) = tokio::io::duplex(64 * 1024);
    let server = tokio::spawn(async move {
        let running = server.serve(server_io).await.expect("server start");
        let _ = running.waiting().await;
    });
    let client = TestClient
        .serve(client_io)
        .await
        .expect("client initialize");
    Session {
        dir,
        client,
        server,
    }
}

async fn call(s: &Session, name: &str, args: Value) -> CallToolResult {
    let map = args.as_object().cloned().unwrap_or_default();
    s.client
        .call_tool(CallToolRequestParams::new(name.to_owned()).with_arguments(map))
        .await
        .unwrap_or_else(|e| panic!("{name} failed at protocol level: {e}"))
}

fn structured(r: &CallToolResult) -> &Value {
    assert_ne!(r.is_error, Some(true), "unexpected tool error: {r:?}");
    r.structured_content.as_ref().expect("structured content")
}

fn error_code(r: &CallToolResult) -> String {
    assert_eq!(r.is_error, Some(true), "expected tool error: {r:?}");
    let text = r
        .content
        .first()
        .and_then(|c| c.as_text())
        .map(|t| t.text.clone())
        .expect("text content");
    let v: Value = serde_json::from_str(&text).expect("json error body");
    v["error"]["code"].as_str().unwrap().to_owned()
}

fn hash_tree(root: &Path) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for e in std::fs::read_dir(&dir).unwrap() {
            let p = e.unwrap().path();
            let name = p.file_name().unwrap().to_string_lossy().to_string();
            if p.is_dir() {
                stack.push(p);
            } else if !name.ends_with("-shm") && !name.ends_with("-wal") && !name.ends_with(".lock")
            {
                out.push((
                    p.display().to_string(),
                    hex::encode(Sha256::digest(std::fs::read(&p).unwrap())),
                ));
            }
        }
    }
    out.sort();
    out
}

#[tokio::test]
async fn initialize_and_server_info() {
    let s = session().await;
    let info = s.client.peer_info().expect("server info");
    assert_eq!(
        info.server_info.as_ref().map(|i| i.name.as_str()),
        Some("berlin-group-spec")
    );
    assert!(info.capabilities.tools.is_some());
    let instructions = info.instructions.as_deref().unwrap();
    assert!(instructions.contains("discovered_evidence") && instructions.contains("read-only"));
    s.client.cancel().await.unwrap();
    s.server.await.unwrap();
}

#[tokio::test]
async fn list_tools_exposes_exactly_the_read_only_tool_set() {
    let s = session().await;
    let tools = s.client.list_all_tools().await.unwrap();
    let mut names: Vec<_> = tools.iter().map(|t| t.name.to_string()).collect();
    names.sort();
    let mut expected: Vec<_> = TOOL_NAMES.iter().map(|n| n.to_string()).collect();
    expected.sort();
    assert_eq!(names, expected);

    for t in &tools {
        let ann = t
            .annotations
            .as_ref()
            .unwrap_or_else(|| panic!("{} has no annotations", t.name));
        assert_eq!(ann.read_only_hint, Some(true), "{}", t.name);
        assert_eq!(ann.destructive_hint, Some(false), "{}", t.name);
        assert_eq!(
            t.input_schema.get("type").and_then(Value::as_str),
            Some("object"),
            "{}",
            t.name
        );
        assert!(
            t.output_schema.is_some(),
            "{} lacks an output schema",
            t.name
        );
        for forbidden in [
            "write",
            "delete",
            "exec",
            "shell",
            "file_path",
            "filesystem",
        ] {
            assert!(!t.name.contains(forbidden), "{}", t.name);
        }
    }

    let schema_of = |name: &str| {
        let t = tools.iter().find(|t| t.name == name).unwrap();
        serde_json::to_value(t.input_schema.as_ref()).unwrap()
    };
    let search = schema_of("search_specification");
    assert_eq!(search["required"], json!(["query"]));
    for p in ["query", "version", "kind", "source_id", "limit"] {
        assert!(
            search["properties"].get(p).is_some(),
            "search_specification.{p}"
        );
    }
    let read = schema_of("read_source");
    let mut req: Vec<_> = read["required"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    req.sort();
    assert_eq!(req, ["locator", "source_id"]);
    assert!(
        read["properties"].get("path").is_none(),
        "read_source must not accept filesystem paths"
    );

    let desc = tools
        .iter()
        .find(|t| t.name == "search_specification")
        .unwrap()
        .description
        .as_deref()
        .unwrap();
    assert!(desc.contains("must NOT automatically be interpreted as authoritative requirements"));
}

#[tokio::test]
async fn valid_invocations_return_structured_provenance() {
    let s = session().await;
    let ep = "/accounts/{accountId}/transactions";

    let r = call(&s, "search_specification", json!({"query": "transaction list", "version": "openfinance-v2", "kind": "pdf", "limit": 3})).await;
    let v = structured(&r);
    assert!(!v["results"].as_array().unwrap().is_empty());
    for hit in v["results"].as_array().unwrap() {
        assert_eq!(hit["classification"], "discovered_evidence");
        for k in [
            "source_id",
            "version",
            "kind",
            "title",
            "locator",
            "evidence",
            "relevance",
            "sha256",
        ] {
            assert!(!hit[k].is_null(), "missing {k}");
        }
    }

    let v = structured(
        &call(
            &s,
            "get_endpoint_requirements",
            json!({"version": "openfinance-v2", "path": ep, "method": "GET"}),
        )
        .await,
    )
    .clone();
    for k in [
        "endpoint",
        "openapi",
        "requirements",
        "schemas",
        "related_evidence",
        "conflicts",
        "sources",
    ] {
        assert!(v.get(k).is_some(), "bundle.{k}");
    }

    let v = structured(
        &call(
            &s,
            "read_openapi_endpoint",
            json!({"version": "openfinance-v2", "path": ep, "method": "GET"}),
        )
        .await,
    )
    .clone();
    assert_eq!(v["operation"]["operation_id"], "getTransactionList");

    let v = structured(
        &call(
            &s,
            "trace_requirement",
            json!({"requirement_id": "OFV2-TRANSACTIONS-001"}),
        )
        .await,
    )
    .clone();
    assert_eq!(v["classification"], "authoritative_requirement");
    let src = v["sources"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["locator"] == "page:3")
        .unwrap()
        .clone();

    let v = structured(
        &call(
            &s,
            "read_source",
            json!({"source_id": src["source_id"], "locator": "page:3"}),
        )
        .await,
    )
    .clone();
    assert!(
        v["content"]["pages"][0]["text"]
            .as_str()
            .unwrap()
            .contains("SHALL")
    );
    assert!(!v["provenance"].as_array().unwrap().is_empty());

    let v =
        structured(&call(&s, "compare_v1_v2", json!({"path": ep, "method": "GET"})).await).clone();
    assert!(!v["changes"].as_array().unwrap().is_empty());
    assert!(
        !v["compatibility_relevant_facts"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    let v =
        structured(&call(&s, "read_openapi_schema", json!({"name": "Transactions"})).await).clone();
    assert_eq!(v["schema"]["provenance"]["locator"], "schema:Transactions");

    let v = structured(
        &call(
            &s,
            "find_requirement",
            json!({"query": "OFV2-TRANSACTIONS-001"}),
        )
        .await,
    )
    .clone();
    assert_eq!(
        v["requirements"][0]["requirement_id"],
        "OFV2-TRANSACTIONS-001"
    );

    let v = structured(&call(&s, "list_sources", json!({})).await).clone();
    assert_eq!(v["total"], 6);

    let v = structured(&call(&s, "health", json!({})).await).clone();
    assert_eq!(v["read_only"], true);
}

#[tokio::test]
async fn invalid_invocations_are_tool_errors_not_crashes() {
    let s = session().await;
    let cases = [
        (
            "read_source",
            json!({"source_id": "../../etc/passwd", "locator": "page:1"}),
            "not_found",
        ),
        (
            "read_source",
            json!({"source_id": "bg-openfinance-v2-implementation-guidelines", "locator": "file:/etc/passwd"}),
            "invalid_locator",
        ),
        (
            "read_source",
            json!({"source_id": "bg-openfinance-v2-implementation-guidelines", "locator": "page:0"}),
            "invalid_locator",
        ),
        (
            "read_openapi_endpoint",
            json!({"path": "/does/not/exist", "method": "GET"}),
            "not_found",
        ),
        (
            "read_openapi_endpoint",
            json!({"path": "/accounts", "method": "TRACEX"}),
            "invalid_input",
        ),
        (
            "read_openapi_endpoint",
            json!({"version": "no-such-version", "path": "/accounts", "method": "GET"}),
            "invalid_input",
        ),
        (
            "read_openapi_schema",
            json!({"name": "NoSuchSchema"}),
            "not_found",
        ),
        (
            "trace_requirement",
            json!({"requirement_id": "OFV2-INVENTED-999"}),
            "not_found",
        ),
        (
            "search_specification",
            json!({"query": ""}),
            "invalid_input",
        ),
    ];
    for (tool, args, code) in cases {
        let r = call(&s, tool, args.clone()).await;
        assert_eq!(error_code(&r), code, "{tool} {args}");
    }

    // Schema violations (unknown or missing fields) are rejected before reaching services.
    for (tool, args) in [
        ("search_specification", json!({"qury": "typo"})),
        ("read_source", json!({"path": "/etc/passwd"})),
        ("trace_requirement", json!({})),
    ] {
        let map = args.as_object().cloned().unwrap_or_default();
        let r = s
            .client
            .call_tool(CallToolRequestParams::new(tool.to_owned()).with_arguments(map))
            .await;
        // Either a protocol-level invalid-params error or a tool error is acceptable.
        if let Ok(res) = r {
            assert_eq!(res.is_error, Some(true), "{tool} {args}");
        }
    }

    let unknown = s
        .client
        .call_tool(CallToolRequestParams::new("write_file".to_owned()))
        .await;
    assert!(unknown.is_err() || unknown.unwrap().is_error == Some(true));

    // The server is still healthy after all failures.
    assert_eq!(
        structured(&call(&s, "health", json!({})).await)["read_only"],
        true
    );
}

#[tokio::test]
async fn session_does_not_modify_catalog_index_or_corpus() {
    let s = session().await;
    let before = hash_tree(s.dir.path());
    for (tool, args) in [
        ("search_specification", json!({"query": "consent"})),
        (
            "get_endpoint_requirements",
            json!({"path": "/accounts/{accountId}/transactions", "method": "GET"}),
        ),
        (
            "trace_requirement",
            json!({"requirement_id": "OFV2-TRANSACTIONS-003"}),
        ),
        (
            "compare_v1_v2",
            json!({"path": "/accounts/{accountId}/transactions", "method": "GET"}),
        ),
        ("list_sources", json!({})),
        ("health", json!({})),
    ] {
        call(&s, tool, args).await;
    }
    let after = hash_tree(s.dir.path());
    assert_eq!(before, after, "an MCP session must not modify any file");
}
