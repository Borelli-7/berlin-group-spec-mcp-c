//! `config.toml` model. Relative paths resolve against the directory of the config file.

use crate::{CoreError, Result, domain::SpecificationVersion};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Root directory of the corpus. All sources must live below it.
    pub corpus_root: PathBuf,
    /// Directory holding the published index generations (`CURRENT`, `generations/<n>/`).
    pub data_dir: PathBuf,
    /// Manifest path relative to `corpus_root`.
    #[serde(default = "default_manifest")]
    pub manifest: PathBuf,
    /// Requirement mapping path relative to `corpus_root`.
    #[serde(default = "default_requirements")]
    pub requirements: PathBuf,
    pub versions: VersionsConfig,
    #[serde(default)]
    pub chunking: ChunkingConfig,
    #[serde(default)]
    pub pdf: PdfConfig,
    #[serde(default)]
    pub search: SearchConfig,
    #[serde(default)]
    pub index: IndexConfig,
    #[serde(default)]
    pub log: LogConfig,
}

/// Upper bound for `index.jobs`.
pub const MAX_INDEX_JOBS: usize = 64;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IndexConfig {
    /// Sources hashed, extracted and normalised concurrently. `0` (default) uses the available
    /// parallelism, capped at 8. Output is identical for every value.
    #[serde(default)]
    pub jobs: usize,
}

impl IndexConfig {
    /// Effective number of concurrent jobs.
    pub fn effective_jobs(&self) -> usize {
        match self.jobs {
            0 => std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(1)
                .min(8),
            n => n.min(MAX_INDEX_JOBS),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VersionsConfig {
    /// Version used as "V1" by `compare_v1_v2`.
    pub baseline: SpecificationVersion,
    /// Version used as "V2" and as default version for lookups.
    pub target: SpecificationVersion,
    /// Optional path prefixes stripped for cross-version endpoint matching (e.g. `/v1`).
    #[serde(default)]
    pub path_prefixes: BTreeMap<String, String>,
}

impl VersionsConfig {
    pub fn prefix_for(&self, version: &SpecificationVersion) -> Option<&str> {
        self.path_prefixes.get(version.as_str()).map(String::as_str)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChunkingConfig {
    pub max_chars: usize,
    pub overlap_chars: usize,
}

impl Default for ChunkingConfig {
    fn default() -> Self {
        Self {
            max_chars: 6000,
            overlap_chars: 500,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PdfConfig {
    /// Pages with fewer non-whitespace characters are flagged `ocr_required`.
    pub min_chars_per_page: usize,
}

impl Default for PdfConfig {
    fn default() -> Self {
        Self {
            min_chars_per_page: 16,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SearchConfig {
    pub default_limit: usize,
    pub max_limit: usize,
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            default_limit: 10,
            max_limit: 50,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LogConfig {
    pub level: String,
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            level: "info".to_owned(),
        }
    }
}

fn default_manifest() -> PathBuf {
    PathBuf::from("manifest.yaml")
}

fn default_requirements() -> PathBuf {
    PathBuf::from("requirements/requirements.yaml")
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let raw = std::fs::read_to_string(path)
            .map_err(|e| CoreError::io(path.display().to_string(), e))?;
        let base = path.parent().unwrap_or_else(|| Path::new("."));
        Self::parse(&raw, base)
    }

    pub fn parse(raw: &str, base_dir: &Path) -> Result<Self> {
        let mut cfg: Config = toml::from_str(raw).map_err(|e| CoreError::Config(e.to_string()))?;
        if cfg.corpus_root.is_relative() {
            cfg.corpus_root = base_dir.join(&cfg.corpus_root);
        }
        if cfg.data_dir.is_relative() {
            cfg.data_dir = base_dir.join(&cfg.data_dir);
        }
        cfg.validate()?;
        Ok(cfg)
    }

    fn validate(&self) -> Result<()> {
        for (label, p) in [
            ("manifest", &self.manifest),
            ("requirements", &self.requirements),
        ] {
            if p.is_absolute()
                || p.components()
                    .any(|c| matches!(c, std::path::Component::ParentDir))
            {
                return Err(CoreError::Config(format!(
                    "{label} path must be relative to corpus_root without '..'"
                )));
            }
        }
        if self.chunking.max_chars < 500 {
            return Err(CoreError::Config(
                "chunking.max_chars must be >= 500".into(),
            ));
        }
        if self.chunking.overlap_chars >= self.chunking.max_chars / 2 {
            return Err(CoreError::Config(
                "chunking.overlap_chars must be < max_chars / 2".into(),
            ));
        }
        if self.index.jobs > MAX_INDEX_JOBS {
            return Err(CoreError::Config(format!(
                "index.jobs must be <= {MAX_INDEX_JOBS} (0 = automatic)"
            )));
        }
        if self.search.default_limit == 0 || self.search.max_limit < self.search.default_limit {
            return Err(CoreError::Config(
                "search.default_limit must be > 0 and <= search.max_limit".into(),
            ));
        }
        for (v, prefix) in &self.versions.path_prefixes {
            SpecificationVersion::new(v.clone())?;
            if !prefix.starts_with('/') {
                return Err(CoreError::Config(format!(
                    "path prefix for {v} must start with '/'"
                )));
            }
        }
        Ok(())
    }

    pub fn manifest_path(&self) -> PathBuf {
        self.corpus_root.join(&self.manifest)
    }

    pub fn requirements_path(&self) -> PathBuf {
        self.corpus_root.join(&self.requirements)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
corpus_root = "../corpus"
data_dir = "../data"
[versions]
baseline = "nextgenpsd2-v1.3"
target = "openfinance-v2"
[versions.path_prefixes]
"nextgenpsd2-v1.3" = "/v1"
"#;

    #[test]
    fn parses_and_resolves_relative_paths() {
        let cfg = Config::parse(SAMPLE, Path::new("/opt/bg/config")).unwrap();
        assert_eq!(cfg.corpus_root, PathBuf::from("/opt/bg/config/../corpus"));
        assert_eq!(cfg.chunking.max_chars, 6000);
        assert_eq!(cfg.chunking.overlap_chars, 500);
        assert_eq!(cfg.versions.prefix_for(&cfg.versions.baseline), Some("/v1"));
        assert_eq!(cfg.data_dir, PathBuf::from("/opt/bg/config/../data"));
    }

    #[test]
    fn rejects_unknown_keys_and_bad_values() {
        assert!(Config::parse(&format!("{SAMPLE}\nfoo = 1\n"), Path::new("/")).is_err());
        let bad = SAMPLE.replace("[versions]", "manifest = \"../x.yaml\"\n[versions]");
        assert!(Config::parse(&bad, Path::new("/")).is_err());
    }
}
