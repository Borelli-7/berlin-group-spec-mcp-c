//! Bounded in-memory caches for read paths.
//!
//! A [`Services`](super::Services) instance serves exactly one immutable index generation (the MCP
//! server builds a new instance when a new generation is published), so cache entries never go
//! stale and need no invalidation. Only successful results are cached; cached values are clones of
//! what the uncached path returns, so responses stay byte-identical.

use super::{OperationLookup, SchemaClosure, requirement::ResolvedSources};
use std::{
    collections::{BTreeSet, HashMap},
    hash::Hash,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

/// Entries kept per cache before the least recently used one is evicted.
pub const CACHE_CAPACITY: usize = 512;

/// A small least-recently-used map. Eviction scans for the oldest entry, which is cheap at this
/// capacity and keeps the implementation dependency-free.
pub(crate) struct BoundedCache<K, V> {
    capacity: usize,
    entries: Mutex<HashMap<K, (V, u64)>>,
    clock: AtomicU64,
    hits: AtomicU64,
    misses: AtomicU64,
}

impl<K: Eq + Hash + Clone, V: Clone> BoundedCache<K, V> {
    pub(crate) fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            entries: Mutex::new(HashMap::new()),
            clock: AtomicU64::new(0),
            hits: AtomicU64::new(0),
            misses: AtomicU64::new(0),
        }
    }

    fn tick(&self) -> u64 {
        self.clock.fetch_add(1, Ordering::Relaxed)
    }

    pub(crate) fn get(&self, key: &K) -> Option<V> {
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        match entries.get_mut(key) {
            Some((value, used)) => {
                *used = self.tick();
                self.hits.fetch_add(1, Ordering::Relaxed);
                Some(value.clone())
            }
            None => {
                self.misses.fetch_add(1, Ordering::Relaxed);
                None
            }
        }
    }

    pub(crate) fn insert(&self, key: K, value: V) {
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        if !entries.contains_key(&key) && entries.len() >= self.capacity {
            let oldest = entries
                .iter()
                .min_by_key(|(_, (_, used))| *used)
                .map(|(k, _)| k.clone());
            if let Some(k) = oldest {
                entries.remove(&k);
            }
        }
        entries.insert(key, (value, self.tick()));
    }

    pub(crate) fn stats(&self) -> CacheStats {
        CacheStats {
            entries: self.entries.lock().unwrap_or_else(|e| e.into_inner()).len(),
            hits: self.hits.load(Ordering::Relaxed),
            misses: self.misses.load(Ordering::Relaxed),
        }
    }
}

/// Counters of one cache.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CacheStats {
    pub entries: usize,
    pub hits: u64,
    pub misses: u64,
}

/// Key of a schema closure: referencing source, version and the ordered roots.
pub(crate) type ClosureKey = (String, String, Vec<String>);
/// Key of an operation lookup: version, normalised method and requested path.
pub(crate) type LookupKey = (String, String, String);

/// Every cache of one service generation.
pub(crate) struct ServiceCaches {
    pub(crate) known_versions: Mutex<Option<Arc<BTreeSet<String>>>>,
    pub(crate) operations: BoundedCache<LookupKey, Option<OperationLookup>>,
    pub(crate) closures: BoundedCache<ClosureKey, SchemaClosure>,
    pub(crate) requirement_sources: BoundedCache<String, ResolvedSources>,
}

impl ServiceCaches {
    pub(crate) fn new(capacity: usize) -> Self {
        Self {
            known_versions: Mutex::new(None),
            operations: BoundedCache::new(capacity),
            closures: BoundedCache::new(capacity),
            requirement_sources: BoundedCache::new(capacity),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evicts_least_recently_used() {
        let c: BoundedCache<u32, u32> = BoundedCache::new(2);
        c.insert(1, 10);
        c.insert(2, 20);
        assert_eq!(c.get(&1), Some(10));
        c.insert(3, 30);
        assert_eq!(c.get(&2), None);
        assert_eq!(c.get(&1), Some(10));
        assert_eq!(c.get(&3), Some(30));
        c.insert(3, 31);
        assert_eq!(c.get(&3), Some(31));
        assert_eq!(
            c.stats(),
            CacheStats {
                entries: 2,
                hits: 4,
                misses: 1
            }
        );
    }
}
