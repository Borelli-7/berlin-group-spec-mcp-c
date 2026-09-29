//! Domain model. Every source-backed record carries a [`Provenance`].

mod compat;
mod conflict;
mod document;
mod evidence;
mod locator;
mod openapi;
mod provenance;
mod requirement;
mod version;

pub use compat::*;
pub use conflict::*;
pub use document::*;
pub use evidence::*;
pub use locator::*;
pub use openapi::*;
pub use provenance::*;
pub use requirement::*;
pub use version::*;
