//! Corpus discovery and path confinement.

use bg_spec_core::{CoreError, Result, manifest::Manifest};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};
use walkdir::WalkDir;

/// Canonical corpus root. Every source path must resolve inside it.
#[derive(Debug, Clone)]
pub struct CorpusRoot(PathBuf);

impl CorpusRoot {
    pub fn open(path: &Path) -> Result<Self> {
        let canonical = path
            .canonicalize()
            .map_err(|e| CoreError::Config(format!("corpus root {} is not accessible: {e}", path.display())))?;
        if !canonical.is_dir() {
            return Err(CoreError::Config(format!("corpus root {} is not a directory", canonical.display())));
        }
        Ok(Self(canonical))
    }

    pub fn path(&self) -> &Path {
        &self.0
    }

    /// Resolves a manifest-relative path to a canonical file path inside the corpus.
    /// Rejects absolute paths, `..`, and symlinks escaping the corpus root.
    pub fn resolve(&self, relative: &str) -> Result<Option<PathBuf>> {
        bg_spec_core::manifest::validate_relative_path(relative)?;
        let joined = self.0.join(relative);
        if !joined.exists() {
            return Ok(None);
        }
        let canonical = joined
            .canonicalize()
            .map_err(|e| CoreError::io(relative.to_owned(), e))?;
        if !canonical.starts_with(&self.0) {
            return Err(CoreError::Integrity(format!("path '{relative}' resolves outside the corpus root")));
        }
        if !canonical.is_file() {
            return Err(CoreError::Integrity(format!("path '{relative}' is not a regular file")));
        }
        Ok(Some(canonical))
    }
}

/// Files under `openapi/`, `pdf/` or `text/` that are not referenced by the manifest.
pub fn orphan_files(root: &CorpusRoot, manifest: &Manifest) -> Vec<String> {
    let referenced: BTreeSet<&str> = manifest.sources.iter().map(|s| s.path.as_str()).collect();
    let mut out = Vec::new();
    for dir in ["openapi", "pdf", "text"] {
        let base = root.path().join(dir);
        for entry in WalkDir::new(&base).follow_links(false).sort_by_file_name().into_iter().flatten() {
            if !entry.file_type().is_file() || entry.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            if let Ok(rel) = entry.path().strip_prefix(root.path()) {
                let rel = rel.to_string_lossy().replace('\\', "/");
                if !referenced.contains(rel.as_str()) {
                    out.push(rel);
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn confines_paths() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("corpus/pdf")).unwrap();
        std::fs::write(dir.path().join("corpus/pdf/a.pdf"), b"x").unwrap();
        std::fs::write(dir.path().join("secret.txt"), b"x").unwrap();
        let root = CorpusRoot::open(&dir.path().join("corpus")).unwrap();
        assert!(root.resolve("pdf/a.pdf").unwrap().is_some());
        assert!(root.resolve("pdf/missing.pdf").unwrap().is_none());
        assert!(root.resolve("../secret.txt").is_err());
        assert!(root.resolve("/etc/passwd").is_err());
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(dir.path().join("secret.txt"), dir.path().join("corpus/pdf/link.pdf")).unwrap();
            assert!(root.resolve("pdf/link.pdf").is_err());
        }
    }
}
