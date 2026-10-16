//! Disposable response cache (perf only). Git/GitHub stay authoritative;
//! entries expire and are never persisted (spec §8.4).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::github::models::{CacheStats, DataSource};

const TTL: Duration = Duration::from_secs(90);

struct Entry {
    inserted: Instant,
    value: String,
}

struct Inner {
    entries: HashMap<String, Entry>,
    capacity: usize,
    hits: u64,
    misses: u64,
}

pub struct Cache {
    inner: Mutex<Inner>,
}

/// Coalesces concurrent identical live reads: the first command to miss the
/// [`Cache`] runs the clone+read while latecomers wait on the same key and
/// then hit the freshly populated entry, so N simultaneous tab mounts cost
/// one clone+call instead of N.
pub struct Singleflight {
    inner: Mutex<HashMap<String, Arc<Mutex<()>>>>,
}

impl Singleflight {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
        }
    }

    pub fn lock_for(&self, key: &str) -> Arc<Mutex<()>> {
        let mut slots = self.inner.lock().unwrap_or_else(|err| err.into_inner());
        slots.get(key).cloned().unwrap_or_else(|| {
            let slot = Arc::new(Mutex::new(()));
            slots.insert(key.to_string(), slot.clone());
            slot
        })
    }

    /// Drop the slot once nobody is waiting on it, so the map stays small.
    /// A waiter that arrived before the removal holds its own `Arc` and still
    /// serializes correctly; newcomers create a fresh slot and hit the cache.
    pub fn remove_if_unused(&self, key: &str) {
        let mut slots = self.inner.lock().unwrap_or_else(|err| err.into_inner());
        if let Some(slot) = slots.get(key) {
            if Arc::strong_count(slot) <= 2 {
                slots.remove(key);
            }
        }
    }

    #[cfg(test)]
    pub fn len_for_tests(&self) -> usize {
        self.inner.lock().map(|slots| slots.len()).unwrap_or(0)
    }
}

impl Default for Singleflight {
    fn default() -> Self {
        Self::new()
    }
}

impl Cache {
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: Mutex::new(Inner {
                entries: HashMap::new(),
                capacity,
                hits: 0,
                misses: 0,
            }),
        }
    }

    pub fn get(&self, key: &str) -> Option<String> {
        let mut inner = self.inner.lock().ok()?;
        let fresh = inner
            .entries
            .get(key)
            .map(|entry| entry.inserted.elapsed() < TTL)
            .unwrap_or(false);
        if fresh {
            inner.hits += 1;
            return inner.entries.get(key).map(|entry| entry.value.clone());
        }
        inner.misses += 1;
        if inner.entries.contains_key(key) {
            inner.entries.remove(key);
        }
        None
    }

    pub fn put(&self, key: &str, value: String) {
        let Ok(mut inner) = self.inner.lock() else {
            return;
        };
        if inner.entries.len() >= inner.capacity && !inner.entries.contains_key(key) {
            if let Some(oldest) = inner
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.inserted)
                .map(|(k, _)| k.clone())
            {
                inner.entries.remove(&oldest);
            }
        }
        inner.entries.insert(
            key.to_string(),
            Entry {
                inserted: Instant::now(),
                value,
            },
        );
    }

    pub fn clear(&self) -> usize {
        let Ok(mut inner) = self.inner.lock() else {
            return 0;
        };
        let removed = inner.entries.len();
        inner.entries.clear();
        removed
    }

    pub fn stats(&self) -> CacheStats {
        let fallback = CacheStats {
            entries: 0,
            capacity: 0,
            hits: 0,
            misses: 0,
            oldest_entry_at: None,
            rebuildable: true,
            data_source: DataSource::Fixture,
            note: String::new(),
        };
        let Ok(inner) = self.inner.lock() else {
            return fallback;
        };
        let oldest = inner
            .entries
            .values()
            .map(|entry| entry.inserted.elapsed())
            .max()
            .map(|elapsed| elapsed.as_secs());

        CacheStats {
            entries: inner.entries.len(),
            capacity: inner.capacity,
            hits: inner.hits,
            misses: inner.misses,
            oldest_entry_at: oldest.map(|secs| format!("{secs}s ago")),
            rebuildable: true,
            data_source: DataSource::Fixture,
            note: format!(
                "Disposable {}-second cache for commit pages and diffs. Git history and GitHub are authoritative; clearing it only costs a refetch.",
                TTL.as_secs()
            ),
        }
    }
}
