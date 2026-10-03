//! On-disk layout of published index generations.
//!
//! Every `bg-spec index` run builds a complete catalog and search index in a private staging
//! directory and then publishes it as `generations/<n>/`. The `CURRENT` file names the published
//! generation and is replaced atomically (write + fsync + rename), so readers always open a
//! matching SQLite/Tantivy pair and never observe a half-written index.
//!
//! ```text
//! data_dir/
//!   CURRENT                  # "<n>\n"
//!   generations/<n>/catalog.db
//!   generations/<n>/tantivy/
//! ```
//!
//! A data directory without `CURRENT` uses the legacy layout (`catalog.db` and `tantivy/` directly
//! in `data_dir`); the next index run migrates it.

use bg_spec_core::{CoreError, Result};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

pub const CURRENT_FILE: &str = "CURRENT";
pub const GENERATIONS_DIR: &str = "generations";
const CATALOG_FILE: &str = "catalog.db";
const TANTIVY_DIR: &str = "tantivy";
const STAGING_PREFIX: &str = ".staging-";
/// Published generations kept on disk: the current one and its predecessor, so a reader that
/// opened the previous generation keeps working until it reloads.
pub const KEEP_GENERATIONS: usize = 2;

/// Paths of one catalog/search index pair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexLayout {
    /// Published generation, or `None` for the legacy (unversioned) layout.
    pub generation: Option<u64>,
    pub catalog_path: PathBuf,
    pub tantivy_dir: PathBuf,
}

impl IndexLayout {
    pub fn in_dir(dir: &Path, generation: Option<u64>) -> Self {
        Self {
            generation,
            catalog_path: dir.join(CATALOG_FILE),
            tantivy_dir: dir.join(TANTIVY_DIR),
        }
    }
}

fn io_err(path: &Path, e: std::io::Error) -> CoreError {
    CoreError::io(path.display().to_string(), e)
}

fn generations_dir(data_dir: &Path) -> PathBuf {
    data_dir.join(GENERATIONS_DIR)
}

pub fn generation_dir(data_dir: &Path, generation: u64) -> PathBuf {
    generations_dir(data_dir).join(generation.to_string())
}

/// The published generation named by `CURRENT`, if any.
pub fn read_current(data_dir: &Path) -> Result<Option<u64>> {
    let path = data_dir.join(CURRENT_FILE);
    match fs::read_to_string(&path) {
        Ok(raw) => raw.trim().parse::<u64>().map(Some).map_err(|_| {
            CoreError::Config(format!(
                "{} does not name a generation ({:?}); run `bg-spec index --force`",
                path.display(),
                raw.trim()
            ))
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(io_err(&path, e)),
    }
}

/// Layout readers should open: the published generation, or the legacy layout.
pub fn current_layout(data_dir: &Path) -> Result<IndexLayout> {
    Ok(match read_current(data_dir)? {
        Some(n) => IndexLayout::in_dir(&generation_dir(data_dir, n), Some(n)),
        None => IndexLayout::in_dir(data_dir, None),
    })
}

/// A fresh, empty staging directory for one index run.
pub fn create_staging(data_dir: &Path) -> Result<PathBuf> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir =
        generations_dir(data_dir).join(format!("{STAGING_PREFIX}{}-{nanos}", std::process::id()));
    fs::create_dir_all(&dir).map_err(|e| io_err(&dir, e))?;
    Ok(dir)
}

/// Copies the catalog and search index of `from` into the staging directory, so an incremental
/// run starts from the published state. Missing files are skipped (first run).
pub fn seed_staging(from: &IndexLayout, staging: &Path) -> Result<()> {
    let to = IndexLayout::in_dir(staging, None);
    for suffix in ["", "-wal"] {
        let src = PathBuf::from(format!("{}{suffix}", from.catalog_path.display()));
        if src.is_file() {
            let dst = PathBuf::from(format!("{}{suffix}", to.catalog_path.display()));
            fs::copy(&src, &dst).map_err(|e| io_err(&src, e))?;
        }
    }
    if from.tantivy_dir.is_dir() {
        copy_dir(&from.tantivy_dir, &to.tantivy_dir)?;
    }
    Ok(())
}

fn copy_dir(from: &Path, to: &Path) -> Result<()> {
    fs::create_dir_all(to).map_err(|e| io_err(to, e))?;
    for entry in fs::read_dir(from).map_err(|e| io_err(from, e))? {
        let entry = entry.map_err(|e| io_err(from, e))?;
        let path = entry.path();
        let name = entry.file_name();
        // Tantivy lock files belong to the process that created them.
        if name.to_string_lossy().ends_with(".lock") {
            continue;
        }
        let target = to.join(&name);
        if path.is_dir() {
            copy_dir(&path, &target)?;
        } else {
            fs::copy(&path, &target).map_err(|e| io_err(&path, e))?;
        }
    }
    Ok(())
}

fn sync_dir(dir: &Path) {
    // Directory fsync is best-effort (not supported on every platform).
    if let Ok(f) = fs::File::open(dir) {
        let _ = f.sync_all();
    }
}

/// Publishes a finished staging directory as `generation` and points `CURRENT` at it.
pub fn publish(data_dir: &Path, staging: &Path, generation: u64) -> Result<IndexLayout> {
    let target = generation_dir(data_dir, generation);
    if target.exists() {
        // Left over from a run that crashed after renaming but before publishing.
        fs::remove_dir_all(&target).map_err(|e| io_err(&target, e))?;
    }
    fs::rename(staging, &target).map_err(|e| io_err(staging, e))?;
    sync_dir(&generations_dir(data_dir));

    let tmp = data_dir.join(format!("{CURRENT_FILE}.tmp-{}", std::process::id()));
    {
        let mut f = fs::File::create(&tmp).map_err(|e| io_err(&tmp, e))?;
        writeln!(f, "{generation}").map_err(|e| io_err(&tmp, e))?;
        f.sync_all().map_err(|e| io_err(&tmp, e))?;
    }
    let current = data_dir.join(CURRENT_FILE);
    fs::rename(&tmp, &current).map_err(|e| io_err(&current, e))?;
    sync_dir(data_dir);
    Ok(IndexLayout::in_dir(&target, Some(generation)))
}

/// Removes unpublished staging directories (other than `keep_staging`), generations older than
/// the newest [`KEEP_GENERATIONS`], and the legacy layout once a generation is published.
/// Returns the removed paths in sorted order.
pub fn collect_garbage(data_dir: &Path, keep_staging: Option<&Path>) -> Result<Vec<PathBuf>> {
    let Some(current) = read_current(data_dir)? else {
        return Ok(Vec::new());
    };
    let mut removed = Vec::new();
    let root = generations_dir(data_dir);
    let mut published = Vec::new();
    for entry in fs::read_dir(&root).map_err(|e| io_err(&root, e))? {
        let entry = entry.map_err(|e| io_err(&root, e))?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with(STAGING_PREFIX) {
            if Some(path.as_path()) != keep_staging {
                removed.push(path);
            }
        } else if let Ok(n) = name.parse::<u64>() {
            published.push(n);
        }
    }
    published.sort_unstable_by(|a, b| b.cmp(a));
    let keep: Vec<u64> = published
        .iter()
        .copied()
        .filter(|n| *n <= current)
        .take(KEEP_GENERATIONS)
        .collect();
    for n in published {
        if n != current && !keep.contains(&n) {
            removed.push(generation_dir(data_dir, n));
        }
    }
    for legacy in [
        data_dir.join(CATALOG_FILE),
        data_dir.join(format!("{CATALOG_FILE}-wal")),
        data_dir.join(format!("{CATALOG_FILE}-shm")),
        data_dir.join(TANTIVY_DIR),
    ] {
        if legacy.exists() {
            removed.push(legacy);
        }
    }
    removed.sort();
    for path in &removed {
        let result = if path.is_dir() {
            fs::remove_dir_all(path)
        } else {
            fs::remove_file(path)
        };
        if let Err(e) = result {
            tracing::warn!(path = %path.display(), error = %e, "could not remove old index data");
        }
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(path: &Path) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, b"x").unwrap();
    }

    #[test]
    fn legacy_layout_without_current() {
        let dir = tempfile::tempdir().unwrap();
        let layout = current_layout(dir.path()).unwrap();
        assert_eq!(layout, IndexLayout::in_dir(dir.path(), None));
    }

    #[test]
    fn publish_switches_current_and_gc_keeps_two_generations() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path();
        touch(&data.join("catalog.db"));
        touch(&data.join("tantivy/meta.json"));
        touch(&data.join("tantivy/.tantivy-writer.lock"));

        let mut stale = None;
        for n in 1..=3u64 {
            let staging = create_staging(data).unwrap();
            seed_staging(&current_layout(data).unwrap(), &staging).unwrap();
            assert!(staging.join("catalog.db").is_file());
            assert!(staging.join("tantivy/meta.json").is_file());
            assert!(!staging.join("tantivy/.tantivy-writer.lock").exists());
            let layout = publish(data, &staging, n).unwrap();
            assert_eq!(layout.generation, Some(n));
            assert_eq!(read_current(data).unwrap(), Some(n));
            assert_eq!(current_layout(data).unwrap(), layout);
            if n == 2 {
                stale = Some(create_staging(data).unwrap());
            }
            collect_garbage(data, None).unwrap();
        }
        assert!(!stale.unwrap().exists());
        assert!(!data.join("catalog.db").exists());
        assert!(!data.join("tantivy").exists());
        assert!(!generation_dir(data, 1).exists());
        assert!(generation_dir(data, 2).is_dir());
        assert!(generation_dir(data, 3).is_dir());
    }

    #[test]
    fn invalid_current_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(CURRENT_FILE), "nope").unwrap();
        assert!(current_layout(dir.path()).is_err());
    }
}
