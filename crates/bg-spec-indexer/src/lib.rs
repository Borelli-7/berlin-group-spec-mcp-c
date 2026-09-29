//! Ingestion pipeline: discovery, hashing, PDF extraction, OpenAPI normalization,
//! chunking and indexing into the SQLite catalog and Tantivy index.

#![forbid(unsafe_code)]

pub mod chunk;
pub mod discover;
pub mod openapi;
pub mod pdf;
pub mod pipeline;
pub mod testing;

pub use pipeline::{DocumentReport, IndexAction, IndexOptions, IndexReport, Indexer};
