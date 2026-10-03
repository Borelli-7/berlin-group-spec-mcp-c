//! Domain model, repository ports and application services for the Berlin Group
//! specification evidence system.
//!
//! This crate is free of storage, parsing and transport concerns. Storage adapters live
//! in `bg-spec-store`, ingestion in `bg-spec-indexer` and the MCP adapter in `bg-spec-mcp`.

#![forbid(unsafe_code)]

pub mod config;
pub mod diff;
pub mod domain;
pub mod error;
pub mod hash;
pub mod manifest;
pub mod openapi_path;
pub mod ports;
pub mod requirements;
pub mod services;
pub mod timing;

pub use error::{CoreError, Result};
