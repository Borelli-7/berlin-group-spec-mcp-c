//! Application services. They orchestrate repositories and pure domain logic; they never
//! interpret requirements or produce verdicts.

mod compatibility;
mod requirement;
mod specification;

pub use compatibility::*;
pub use requirement::*;
pub use specification::*;

use crate::{
    config::Config,
    domain::SpecificationVersion,
    ports::{CatalogRepository, SearchRepository},
};
use std::{collections::BTreeMap, sync::Arc};

/// Notice attached to search output: search hits are evidence, not requirements.
pub const EVIDENCE_NOTICE: &str = "Search results are discovered evidence retrieved by full-text relevance. \
They are NOT authoritative requirements. Only curated requirement mappings (classification \
'authoritative_requirement') are requirements; verify evidence with read_source before relying on it.";

/// Notice attached to comparisons: facts only, no verdict.
pub const COMPARISON_NOTICE: &str = "Structural differences only. No backward-compatibility verdict is made; \
interpretation is the responsibility of the consuming agent.";

/// Service-level settings derived from the configuration.
#[derive(Debug, Clone)]
pub struct ServiceSettings {
    pub baseline: SpecificationVersion,
    pub target: SpecificationVersion,
    pub path_prefixes: BTreeMap<String, String>,
    pub default_limit: usize,
    pub max_limit: usize,
}

impl ServiceSettings {
    pub fn from_config(cfg: &Config) -> Self {
        Self {
            baseline: cfg.versions.baseline.clone(),
            target: cfg.versions.target.clone(),
            path_prefixes: cfg.versions.path_prefixes.clone(),
            default_limit: cfg.search.default_limit,
            max_limit: cfg.search.max_limit,
        }
    }

    pub fn prefix_for(&self, v: &SpecificationVersion) -> Option<&str> {
        self.path_prefixes.get(v.as_str()).map(String::as_str)
    }
}

/// All services wired to one pair of repositories.
#[derive(Clone)]
pub struct Services {
    pub specification: SpecificationService,
    pub requirements: RequirementService,
    pub compatibility: CompatibilityService,
}

impl Services {
    pub fn new(
        catalog: Arc<dyn CatalogRepository>,
        search: Arc<dyn SearchRepository>,
        settings: ServiceSettings,
    ) -> Self {
        let specification = SpecificationService::new(catalog, search, Arc::new(settings));
        Self {
            requirements: RequirementService::new(specification.clone()),
            compatibility: CompatibilityService::new(specification.clone()),
            specification,
        }
    }
}
