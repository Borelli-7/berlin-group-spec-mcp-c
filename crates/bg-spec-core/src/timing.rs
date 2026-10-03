//! Lightweight operation timing. Emitted at `debug` level on the `bg_spec::timing` target, so it
//! goes to the configured stderr subscriber and never to the MCP stdio channel.
//! Enable with `BG_SPEC_LOG=bg_spec::timing=debug`.

use std::time::Instant;

pub const TARGET: &str = "bg_spec::timing";

/// Logs the elapsed time of an operation when dropped (including on early `?` returns).
#[must_use = "the timer measures until it is dropped"]
pub struct Timer {
    op: &'static str,
    started: Instant,
}

impl Timer {
    pub fn start(op: &'static str) -> Self {
        Self {
            op,
            started: Instant::now(),
        }
    }
}

impl Drop for Timer {
    fn drop(&mut self) {
        tracing::debug!(
            target: TARGET,
            op = self.op,
            elapsed_us = self.started.elapsed().as_micros() as u64,
            "timing"
        );
    }
}
