//! `bg-spec`: indexing, diagnostics and maintenance CLI for the Berlin Group spec MCP.

#![forbid(unsafe_code)]

mod doctor;

use anyhow::{Context, Result};
use bg_spec_core::{
    config::Config, ports::CatalogRepository, ports::SearchRepository, services::ServiceSettings,
};
use bg_spec_indexer::{IndexAction, IndexOptions, Indexer};
use clap::{Parser, Subcommand};
use std::{path::PathBuf, process::ExitCode, sync::Arc};

#[derive(Parser)]
#[command(
    name = "bg-spec",
    version,
    about = "Berlin Group specification corpus indexer and diagnostics"
)]
struct Cli {
    /// Path to config.toml.
    #[arg(
        long,
        global = true,
        default_value = "config/config.toml",
        env = "BG_SPEC_CONFIG"
    )]
    config: PathBuf,
    /// Emit machine-readable JSON on stdout.
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Index the corpus incrementally (unchanged documents are skipped).
    Index {
        /// Re-index every document regardless of hashes.
        #[arg(long)]
        force: bool,
    },
    /// Check configuration, corpus, catalog and search index consistency.
    Doctor,
    /// Show catalog and index statistics.
    Stats,
    /// List catalogued sources.
    Sources,
}

fn init_tracing(level: &str) {
    let filter = tracing_subscriber::EnvFilter::try_from_env("BG_SPEC_LOG")
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(format!("{level},tantivy=warn")));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .try_init();
}

fn print_json(value: &impl serde::Serialize) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli).await {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::from(2)
        }
    }
}

async fn run(cli: Cli) -> Result<ExitCode> {
    let config =
        Config::load(&cli.config).with_context(|| format!("loading {}", cli.config.display()))?;
    init_tracing(&config.log.level);
    match cli.command {
        Command::Index { force } => index(&config, force, cli.json).await,
        Command::Doctor => doctor::run(&config, cli.json).await,
        Command::Stats => stats(&config, cli.json).await,
        Command::Sources => sources(&config, cli.json).await,
    }
}

async fn index(config: &Config, force: bool, json: bool) -> Result<ExitCode> {
    let report = Indexer::new(config.clone())
        .run(IndexOptions { force })
        .await?;
    if json {
        print_json(&report)?;
    } else {
        for d in &report.documents {
            println!(
                "{:<9} {:<48} {:<8} records={:<4} diagnostics={}",
                format!("{:?}", d.action).to_lowercase(),
                d.source_id,
                d.status.map(|s| s.as_str()).unwrap_or("-"),
                d.records,
                d.diagnostics.len()
            );
            for diag in d
                .diagnostics
                .iter()
                .filter(|x| x.severity != bg_spec_core::domain::Severity::Info)
            {
                println!(
                    "          {:?} {} {}",
                    diag.severity,
                    diag.code,
                    diag.locator.as_deref().unwrap_or("")
                );
            }
        }
        println!(
            "indexed={} skipped={} missing={} failed={} removed={} requirements={} generation={} full_rebuild={} ({} ms)",
            report.count(IndexAction::Indexed),
            report.count(IndexAction::Skipped),
            report.count(IndexAction::Missing),
            report.count(IndexAction::Failed),
            report.count(IndexAction::Removed),
            report.requirements,
            report.index_generation,
            report.full_rebuild,
            report.duration_ms
        );
        for d in &report.requirements_diagnostics {
            println!("requirements: {:?} {} {}", d.severity, d.code, d.message);
        }
        for f in &report.orphan_files {
            println!("orphan file (not in manifest): {f}");
        }
    }
    let failed = report.count(IndexAction::Failed) + report.count(IndexAction::Missing);
    Ok(if failed > 0 {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    })
}

async fn open_services(config: &Config) -> Result<bg_spec_core::services::Services> {
    let (catalog, search) = bg_spec_store::open_read_only(config).await?;
    Ok(bg_spec_core::services::Services::new(
        catalog,
        search,
        ServiceSettings::from_config(config),
    ))
}

async fn stats(config: &Config, json: bool) -> Result<ExitCode> {
    let (catalog, search) = bg_spec_store::open_read_only(config).await?;
    let stats = catalog.stats().await?;
    let meta = catalog.index_meta().await?;
    let search_docs = search.num_docs().await?;
    let docs = Arc::clone(&catalog).list_documents().await?;
    let mut by_version: std::collections::BTreeMap<
        String,
        std::collections::BTreeMap<String, usize>,
    > = Default::default();
    for d in &docs {
        *by_version
            .entry(d.version.to_string())
            .or_default()
            .entry(d.kind.to_string())
            .or_default() += 1;
    }
    let out = serde_json::json!({
        "catalog": stats,
        "index": meta,
        "search_documents": search_docs,
        "documents_by_version": by_version,
    });
    if json {
        print_json(&out)?;
    } else {
        println!("documents          {}", stats.documents);
        println!(
            "pages              {} ({} ocr_required)",
            stats.pages, stats.ocr_required_pages
        );
        println!("chunks             {}", stats.chunks);
        println!("openapi operations {}", stats.operations);
        println!("openapi schemas    {}", stats.schemas);
        println!("requirements       {}", stats.requirements);
        println!("search documents   {search_docs}");
        println!("index generation   {}", meta.index_generation.unwrap_or(0));
        println!(
            "last indexed (unix){:>12}",
            meta.last_indexed_at_unix.unwrap_or(0)
        );
        for (v, kinds) in by_version {
            println!("version {v}: {kinds:?}");
        }
    }
    Ok(ExitCode::SUCCESS)
}

async fn sources(config: &Config, json: bool) -> Result<ExitCode> {
    let services = open_services(config).await?;
    let listing = services.specification.list_sources(None, None).await?;
    if json {
        print_json(&listing)?;
    } else {
        println!(
            "{:<48} {:<18} {:<8} {:<12} {:>4} {:<9} {:<14} path",
            "source_id", "version", "kind", "authority", "prec", "status", "sha256"
        );
        for s in &listing.sources {
            println!(
                "{:<48} {:<18} {:<8} {:<12} {:>4} {:<9} {:<14} {}",
                s.source_id,
                s.version.to_string(),
                s.kind.to_string(),
                s.authority.to_string(),
                s.precedence,
                s.status.to_string(),
                s.sha256
                    .as_deref()
                    .map(|h| &h[..12.min(h.len())])
                    .unwrap_or("-"),
                s.path
            );
        }
    }
    Ok(ExitCode::SUCCESS)
}
