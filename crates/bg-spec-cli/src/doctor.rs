//! `bg-spec doctor`: read-only consistency checks.

use anyhow::Result;
use bg_spec_core::{
    config::Config,
    domain::{ConflictKind, DocumentStatus},
    hash::{sha256_hex, sha256_reader},
    ports::{CatalogRepository, SearchRepository},
    requirements::RequirementsFile,
    services::{ServiceSettings, Services},
};
use bg_spec_indexer::{
    Indexer,
    discover::{CorpusRoot, orphan_files},
};
use serde::Serialize;
use std::process::ExitCode;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
enum Level {
    Ok,
    Warn,
    Error,
}

#[derive(Debug, Serialize)]
struct Check {
    level: Level,
    check: &'static str,
    message: String,
}

#[derive(Default)]
struct Checks(Vec<Check>);

impl Checks {
    fn push(&mut self, level: Level, check: &'static str, message: impl Into<String>) {
        self.0.push(Check {
            level,
            check,
            message: message.into(),
        });
    }
}

pub async fn run(config: &Config, json: bool) -> Result<ExitCode> {
    let mut c = Checks::default();
    c.push(
        Level::Ok,
        "config",
        format!(
            "configuration loaded (corpus_root={})",
            config.corpus_root.display()
        ),
    );

    let root = match CorpusRoot::open(&config.corpus_root) {
        Ok(r) => Some(r),
        Err(e) => {
            c.push(Level::Error, "corpus", e.to_string());
            None
        }
    };
    let manifest = match Indexer::load_manifest(config) {
        Ok(m) => {
            c.push(
                Level::Ok,
                "manifest",
                format!("{} sources declared", m.sources.len()),
            );
            Some(m)
        }
        Err(e) => {
            c.push(Level::Error, "manifest", e.to_string());
            None
        }
    };

    let mut disk_hashes = std::collections::BTreeMap::new();
    if let (Some(root), Some(manifest)) = (&root, &manifest) {
        for s in &manifest.sources {
            match root.resolve(&s.path) {
                Ok(Some(p)) => {
                    let sha = std::fs::File::open(&p)
                        .and_then(|f| sha256_reader(std::io::BufReader::new(f)));
                    match sha {
                        Ok(h) => {
                            disk_hashes.insert(s.id.clone(), h);
                        }
                        Err(e) => c.push(Level::Error, "source_file", format!("{}: {e}", s.id)),
                    }
                }
                Ok(None) => c.push(
                    Level::Error,
                    "source_file",
                    format!("{}: file '{}' is missing", s.id, s.path),
                ),
                Err(e) => c.push(Level::Error, "source_file", format!("{}: {e}", s.id)),
            }
        }
        for f in orphan_files(root, manifest) {
            c.push(
                Level::Warn,
                "orphan_file",
                format!("{f} is not referenced by the manifest"),
            );
        }
    }

    let requirements_raw = std::fs::read_to_string(config.requirements_path());
    match &requirements_raw {
        Ok(raw) => match RequirementsFile::parse(raw) {
            Ok(file) => c.push(
                Level::Ok,
                "requirements",
                format!("{} curated requirements parse", file.requirements.len()),
            ),
            Err(e) => c.push(Level::Error, "requirements", e.to_string()),
        },
        Err(e) => c.push(
            Level::Warn,
            "requirements",
            format!("requirements file not readable: {e}"),
        ),
    }

    match bg_spec_store::open_read_only(config).await {
        Err(e) => c.push(Level::Error, "index", e.to_string()),
        Ok((catalog, search)) => {
            let docs = catalog.list_documents().await?;
            let stats = catalog.stats().await?;
            let meta = catalog.index_meta().await?;
            c.push(
                Level::Ok,
                "catalog",
                format!(
                    "{} documents, generation {}",
                    docs.len(),
                    meta.index_generation.unwrap_or(0)
                ),
            );
            if let Some(manifest) = &manifest {
                for s in &manifest.sources {
                    match docs.iter().find(|d| d.source_id == s.id) {
                        None => c.push(
                            Level::Error,
                            "catalog",
                            format!("{} is not indexed; run `bg-spec index`", s.id),
                        ),
                        Some(d) => {
                            match d.status {
                                DocumentStatus::Missing | DocumentStatus::Failed => c.push(
                                    Level::Error,
                                    "document_status",
                                    format!(
                                        "{} has status {}: {:?}",
                                        d.source_id,
                                        d.status,
                                        d.diagnostics.first().map(|x| &x.message)
                                    ),
                                ),
                                DocumentStatus::Partial => c.push(
                                    Level::Warn,
                                    "document_status",
                                    format!(
                                        "{} is partially indexed ({} diagnostics)",
                                        d.source_id,
                                        d.diagnostics.len()
                                    ),
                                ),
                                DocumentStatus::Indexed => {}
                            }
                            for diag in d.diagnostics.iter().filter(|x| x.code == "ocr_required") {
                                c.push(
                                    Level::Warn,
                                    "ocr_required",
                                    format!(
                                        "{} {}: no extractable text",
                                        d.source_id,
                                        diag.locator.as_deref().unwrap_or("")
                                    ),
                                );
                            }
                            for diag in d
                                .diagnostics
                                .iter()
                                .filter(|x| x.code == "low_quality_text")
                            {
                                c.push(
                                    Level::Warn,
                                    "low_quality_text",
                                    format!(
                                        "{} {}: extracted text looks garbled",
                                        d.source_id,
                                        diag.locator.as_deref().unwrap_or("")
                                    ),
                                );
                            }
                            if let (Some(disk), Some(indexed)) = (disk_hashes.get(&s.id), &d.sha256)
                                && disk != indexed
                            {
                                c.push(
                                    Level::Warn,
                                    "hash_drift",
                                    format!(
                                        "{} changed on disk since indexing; run `bg-spec index`",
                                        s.id
                                    ),
                                );
                            }
                        }
                    }
                }
                for d in &docs {
                    if manifest.source(&d.source_id).is_none() {
                        c.push(
                            Level::Warn,
                            "catalog",
                            format!("{} is indexed but no longer in the manifest", d.source_id),
                        );
                    }
                }
            }
            if let Ok(raw) = &requirements_raw
                && meta.requirements_file_sha256.as_deref()
                    != Some(sha256_hex(raw.as_bytes()).as_str())
            {
                c.push(
                    Level::Warn,
                    "requirements",
                    "requirements.yaml changed since indexing; run `bg-spec index`",
                );
            }
            let search_docs = search.num_docs().await?;
            let expected = stats.chunks + stats.operations + stats.schemas;
            if search_docs == expected {
                c.push(
                    Level::Ok,
                    "search_index",
                    format!("{search_docs} search documents match the catalog"),
                );
            } else {
                c.push(
                    Level::Error,
                    "search_index",
                    format!("search index has {search_docs} documents but catalog expects {expected}; run `bg-spec index --force`"),
                );
            }
            let services = Services::new(
                catalog.clone(),
                search,
                ServiceSettings::from_config(config),
            );
            for rec in catalog.list_requirements().await? {
                let id = rec.requirement.id;
                match services.requirements.trace(&id).await {
                    Ok(trace) => {
                        for conflict in trace.conflicts {
                            let level = match conflict.kind {
                                ConflictKind::DanglingReference | ConflictKind::StaleSource => {
                                    Level::Warn
                                }
                                _ => Level::Ok,
                            };
                            c.push(
                                level,
                                "requirement_trace",
                                format!("{id}: {:?} - {}", conflict.kind, conflict.description),
                            );
                        }
                    }
                    Err(e) => c.push(Level::Error, "requirement_trace", format!("{id}: {e}")),
                }
            }
        }
    }

    let worst = c.0.iter().map(|x| x.level).max().unwrap_or(Level::Ok);
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({ "status": worst, "checks": c.0 }))?
        );
    } else {
        for x in &c.0 {
            let tag = match x.level {
                Level::Ok => "ok   ",
                Level::Warn => "WARN ",
                Level::Error => "ERROR",
            };
            println!("[{tag}] {:<18} {}", x.check, x.message);
        }
        println!("doctor: {worst:?}");
    }
    Ok(if worst == Level::Error {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    })
}
