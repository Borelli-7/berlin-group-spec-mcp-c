//! Read-only MCP adapter for the Berlin Group specification corpus.
//!
//! The server is intentionally thin: every tool validates its typed input, delegates to a
//! `bg_spec_core` service and returns structured JSON. No tool writes, executes or reads
//! caller-supplied filesystem paths.

#![forbid(unsafe_code)]

mod error;
mod inputs;
mod server;

pub use error::ToolError;
pub use inputs::*;
pub use server::{BgSpecServer, SERVER_INSTRUCTIONS, TOOL_NAMES};
