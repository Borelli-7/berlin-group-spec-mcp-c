//! Runs the `bg-spec` binary against a staged copy of the example corpus.

use serde_json::Value;
use std::process::Command;

fn stage() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    bg_spec_indexer::testing::stage_example_corpus(dir.path()).unwrap();
    let corpus = dir.path().join("corpus");
    let config = dir.path().join("config.toml");
    std::fs::write(
        &config,
        format!(
            "corpus_root = {:?}\ndata_dir = {:?}\n\n[versions]\nbaseline = \"nextgenpsd2-v1.3\"\ntarget = \"openfinance-v2\"\n\n[versions.path_prefixes]\n\"nextgenpsd2-v1.3\" = \"/v1\"\n",
            corpus.display().to_string(),
            dir.path().join("data").display().to_string()
        ),
    )
    .unwrap();
    (dir, config)
}

fn run(config: &std::path::Path, args: &[&str]) -> (i32, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_bg-spec"))
        .arg("--config")
        .arg(config)
        .args(args)
        .env("BG_SPEC_LOG", "error")
        .output()
        .unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8(out.stdout).unwrap(),
    )
}

#[test]
fn index_stats_sources_doctor() {
    let (_dir, config) = stage();

    let (code, _) = run(&config, &["stats"]);
    assert_ne!(code, 0, "stats must fail before indexing");

    let (code, out) = run(&config, &["index", "--json"]);
    assert_eq!(code, 0, "{out}");
    let report: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(report["documents"].as_array().unwrap().len(), 6);

    let (code, out) = run(&config, &["index", "--json"]);
    assert_eq!(code, 0);
    let again: Value = serde_json::from_str(&out).unwrap();
    assert!(
        again["documents"]
            .as_array()
            .unwrap()
            .iter()
            .all(|d| d["action"] == "skipped")
    );

    let (code, out) = run(&config, &["stats", "--json"]);
    assert_eq!(code, 0);
    let stats: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(stats["catalog"]["requirements"], 6);
    assert_eq!(stats["catalog"]["ocr_required_pages"], 1);

    let (code, out) = run(&config, &["sources", "--json"]);
    assert_eq!(code, 0);
    let sources: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(sources["total"], 6);

    let (code, out) = run(&config, &["doctor", "--json"]);
    assert_eq!(code, 0, "{out}");
    let doctor: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(doctor["status"], "warn");
    let checks = doctor["checks"].as_array().unwrap();
    for check in ["ocr_required", "requirement_trace", "search_index"] {
        assert!(
            checks.iter().any(|c| c["check"] == check),
            "missing doctor check {check}"
        );
    }
}

#[test]
fn doctor_fails_on_missing_source_file() {
    let (dir, config) = stage();
    assert_eq!(run(&config, &["index"]).0, 0);
    std::fs::remove_file(dir.path().join("corpus/text/v2/errata.md")).unwrap();
    let (code, out) = run(&config, &["doctor", "--json"]);
    assert_eq!(code, 1, "{out}");
    let doctor: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(doctor["status"], "error");
}
