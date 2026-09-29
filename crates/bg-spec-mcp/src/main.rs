//! `bg-spec-mcp`: read-only MCP server over stdio.
//!
//! Startup only opens the existing SQLite catalog (read-only) and Tantivy index; it never parses
//! documents or rebuilds indexes. All logs go to stderr so stdout carries only MCP frames.

use anyhow::{Context, Result};
use bg_spec_core::{
    config::Config,
    services::{ServiceSettings, Services},
};
use bg_spec_mcp::BgSpecServer;
use clap::Parser;
use rmcp::ServiceExt;
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "bg-spec-mcp",
    version,
    about = "Read-only Berlin Group specification MCP server (stdio)"
)]
struct Args {
    /// Path to config.toml.
    #[arg(long, default_value = "config/config.toml", env = "BG_SPEC_CONFIG")]
    config: PathBuf,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let config =
        Config::load(&args.config).with_context(|| format!("loading {}", args.config.display()))?;
    let filter = tracing_subscriber::EnvFilter::try_from_env("BG_SPEC_LOG").unwrap_or_else(|_| {
        tracing_subscriber::EnvFilter::new(format!("{},tantivy=warn", config.log.level))
    });
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();

    let (catalog, search) = bg_spec_store::open_read_only(&config)
        .await
        .context("opening index (run `bg-spec index --config <config>` first)")?;
    let services = Services::new(catalog, search, ServiceSettings::from_config(&config));
    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        "bg-spec-mcp serving on stdio"
    );

    let service = BgSpecServer::new(services)
        .serve(rmcp::transport::stdio())
        .await?;
    service.waiting().await?;
    Ok(())
}
