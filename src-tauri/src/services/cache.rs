//! Disposable response cache (perf only). Git/GitHub stay authoritative;
//! entries expire and are never persisted (spec §8.4).

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
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
    inner: Mutex<HashMap<String, Arc<Slot>>>,
}

pub struct Slot {
    gate: Mutex<()>,
    users: AtomicUsize,
}

impl Slot {
    pub fn enter(&self) -> std::sync::MutexGuard<'_, ()> {
        self.gate.lock().unwrap_or_else(|err| err.into_inner())
    }
}

impl Singleflight {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
        }
    }

    /// Join the in-flight group for `key`; the caller counts as a user until it
    /// calls [`Singleflight::release`]. Counting here, under the map lock, is
    /// what makes "last caller out drops the slot" exact rather than a guess
    /// at `Arc::strong_count`, which straddles `join` and release.
    pub fn join(&self, key: &str) -> Arc<Slot> {
        let mut slots = self.inner.lock().unwrap_or_else(|err| err.into_inner());
        let slot = slots.get(key).cloned().unwrap_or_else(|| {
            let slot = Arc::new(Slot {
                gate: Mutex::new(()),
                users: AtomicUsize::new(0),
            });
            slots.insert(key.to_string(), slot.clone());
            slot
        });
        slot.users.fetch_add(1, Ordering::SeqCst);
        slot
    }

    /// Leave the group and drop the slot when this was the last user, so the
    /// map stays small. Decrement and removal share the map lock, so a caller
    /// arriving meanwhile either keeps the slot alive or gets a fresh one.
    pub fn release(&self, key: &str, slot: &Arc<Slot>) {
        let mut slots = self.inner.lock().unwrap_or_else(|err| err.into_inner());
        if slot.users.fetch_sub(1, Ordering::SeqCst) > 1 {
            return;
        }
        if slots
            .get(key)
            .is_some_and(|current| Arc::ptr_eq(current, slot))
        {
            slots.remove(key);
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
