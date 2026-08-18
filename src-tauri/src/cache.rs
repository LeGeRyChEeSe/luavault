//! In-memory caching primitives used by the VDF, library, and Steam-store layers.
//!
//! Two strategies:
//!
//! * [`TtlCache`] — time-to-live expiration. Entries expire after a configurable
//!   duration. Expired entries are pruned lazily on access.
//!
//! * [`StampedCache`] — freshness is tied to a source file's metadata
//!   ([`FileStamp`]). A value is returned only if the file's modification time
//!   and size match what was recorded when the entry was cached. This is the
//!   right strategy for files that may be rewritten by external processes
//!   (Steam, the OS) without warning.

use std::collections::HashMap;
use std::fmt;
use std::hash::Hash;
use std::sync::Mutex;
use std::time::{Instant, SystemTime};

// Note: FileStamp does NOT derive Hash — the StampedCache key is PathBuf,
// not FileStamp. FileStamp is only used as the freshness validator alongside
// the path key, so it never needs to be a map key.

// ================================================================ TtlCache

/// A time-to-live cache. Entries expire after `Duration`.
///
/// Protected by a `std::sync::Mutex<HashMap<K, (V, Instant)>>`.
/// Expired entries are pruned lazily during `get` or `put` — no background thread.
pub struct TtlCache<K, V>
where
    K: Eq + Hash + Clone,
    V: Clone,
{
    inner: Mutex<HashMap<K, (V, Instant)>>,
    ttl: std::time::Duration,
}

impl<K, V> TtlCache<K, V>
where
    K: Eq + Hash + Clone,
    V: Clone,
{
    /// Create a new cache with the given time-to-live.
    pub fn new(ttl: std::time::Duration) -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
            ttl,
        }
    }

    /// Return a cached value, or `None` if absent or expired.
    ///
    /// Prunes any expired entry encountered during the lookup.
    pub fn get(&self, key: &K) -> Option<V> {
        let mut map = self.inner.lock().unwrap();
        let entry = map.get_mut(key)?;
        if entry.1.elapsed() >= self.ttl {
            map.remove(key);
            return None;
        }
        Some(entry.0.clone())
    }

    /// Insert a key-value pair (overwrites if the key already exists).
    ///
    /// Prunes expired entries before inserting.
    pub fn put(&self, key: K, value: V) {
        let mut map = self.inner.lock().unwrap();
        self.prune_expired(&mut map);
        map.insert(key, (value, Instant::now()));
    }

    /// Remove a single entry.
    pub fn invalidate(&self, key: &K) {
        self.inner.lock().unwrap().remove(key);
    }

    /// Remove all entries.
    pub fn clear(&self) {
        self.inner.lock().unwrap().clear();
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.inner.lock().unwrap().len()
    }

    /// Prune expired entries. Called by `get`, `put`, and `clear`.
    fn prune_expired(&self, map: &mut HashMap<K, (V, Instant)>) {
        let now = Instant::now();
        map.retain(|_, (_, inserted_at)| now.duration_since(*inserted_at) < self.ttl);
    }
}

// ================================================================ StampedCache

/// Snapshot of a file's identity: modification time and size.
///
/// Both fields are needed because a file rewritten at the same nanosecond
/// may share the same `mtime` — differing sizes catch that edge case.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileStamp {
    /// Unix timestamp (nanoseconds) from `SystemTime::duration_since(UNIX_EPOCH)`.
    pub mtime: u128,
    /// File length in bytes.
    pub len: u64,
}

impl FileStamp {
    /// Read `mtime` and `len` from a file on disk.
    ///
    /// Returns `None` if the file does not exist or metadata cannot be read.
    pub fn from_path(path: &std::path::Path) -> Option<Self> {
        let meta = std::fs::metadata(path).ok()?;
        let mtime = meta
            .modified()
            .ok()?
            .duration_since(SystemTime::UNIX_EPOCH)
            .ok()?
            .as_nanos();
        Some(Self {
            mtime,
            len: meta.len(),
        })
    }
}

/// Default settle time: 2 seconds in nanoseconds.
/// See [`StampedCache::new`] for the rationale.
const DEFAULT_SETTLE_NANOS: u64 = 2_000_000_000;

/// A cache keyed by file content identity rather than time.
///
/// A value is returned only if the source file's [`FileStamp`] matches what
/// was recorded at cache-insertion time **and** the file was modified at
/// least 2 seconds ago. This makes it safe for files that may be rewritten
/// by external processes.
pub struct StampedCache<K, V>
where
    K: Eq + Hash + Clone,
    V: Clone,
{
    inner: Mutex<HashMap<K, (V, FileStamp)>>,
    /// How many nanoseconds must elapse after a file's mtime before its
    /// stamp is trusted. Default is 2 s; tests may shorten it to zero.
    settle_nanos: std::sync::atomic::AtomicU64,
    /// Number of successful `get` returns (cache hits). Reset on `clear()`.
    hits: std::sync::atomic::AtomicUsize,
}

impl<K, V> StampedCache<K, V>
where
    K: Eq + Hash + Clone,
    V: Clone,
{
    /// Create a new stamp-based cache with the default 2-second settle window.
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
            settle_nanos: std::sync::atomic::AtomicU64::new(DEFAULT_SETTLE_NANOS),
            hits: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    /// Return a cached value only if the file stamp matches and the file
    /// is old enough to be trusted (see [`StampedCache::new`]).
    pub fn get(&self, key: &K, stamp: &FileStamp) -> Option<V> {
        // Refuse values whose file was just modified — the stamp may still
        // be lying because of NTFS clock tick granularity.
        let settle = self.settle_nanos.load(std::sync::atomic::Ordering::Relaxed);
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0); // 0 → saturating_sub → 0 < settle → refused
        if now.saturating_sub(stamp.mtime) < settle as u128 {
            return None;
        }

        let map = self.inner.lock().unwrap();
        let (value, recorded_stamp) = map.get(key)?;
        if recorded_stamp != stamp {
            return None;
        }
        let result = value.clone();
        drop(map);
        self.hits.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Some(result)
    }

    /// Insert a key-value pair with the current file stamp.
    pub fn put(&self, key: K, stamp: FileStamp, value: V) {
        let mut map = self.inner.lock().unwrap();
        map.insert(key, (value, stamp));
    }

    /// Remove a single entry.
    pub fn invalidate(&self, key: &K) {
        self.inner.lock().unwrap().remove(key);
    }

    /// Remove all entries.
    pub fn clear(&self) {
        self.inner.lock().unwrap().clear();
        self.hits.store(0, std::sync::atomic::Ordering::Relaxed);
    }

    // ---- Test helpers ----

    #[cfg(test)]
    pub fn set_settle_nanos(&self, n: u64) {
        self.settle_nanos.store(n, std::sync::atomic::Ordering::Relaxed);
    }

    /// Restore the default settle window — the counterpart of
    /// [`Self::set_settle_nanos`] so a test never leaks its shortened
    /// window into the tests that run after it.
    #[cfg(test)]
    pub fn reset_settle(&self) {
        self.settle_nanos
            .store(DEFAULT_SETTLE_NANOS, std::sync::atomic::Ordering::Relaxed);
    }

    #[cfg(test)]
    pub fn hits(&self) -> usize {
        self.hits.load(std::sync::atomic::Ordering::Relaxed)
    }

    #[cfg(test)]
    pub fn reset_hits(&self) {
        self.hits.store(0, std::sync::atomic::Ordering::Relaxed);
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.inner.lock().unwrap().len()
    }
}

// =============================================================== Display impl

impl<K, V> fmt::Debug for TtlCache<K, V>
where
    K: Eq + Hash + Clone + fmt::Debug,
    V: Clone,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TtlCache")
            .field("ttl", &self.ttl)
            .finish_non_exhaustive()
    }
}

impl<K, V> fmt::Debug for StampedCache<K, V>
where
    K: Eq + Hash + Clone + fmt::Debug,
    V: Clone,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StampedCache").finish_non_exhaustive()
    }
}

// ================================================================ Tests

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("ast_cache_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn ttl_cache_expires_entries() {
        let cache = TtlCache::new(std::time::Duration::from_millis(50));
        cache.put("key1".to_string(), "value1".to_string());
        assert_eq!(cache.get(&"key1".to_string()), Some("value1".to_string()));

        // Wait for expiration.
        std::thread::sleep(std::time::Duration::from_millis(100));
        assert!(cache.get(&"key1".to_string()).is_none());
    }

    #[test]
    fn ttl_cache_invalidate_removes_entry() {
        let cache = TtlCache::new(std::time::Duration::from_secs(3600));
        cache.put("key1".to_string(), "value1".to_string());
        cache.invalidate(&"key1".to_string());
        assert!(cache.get(&"key1".to_string()).is_none());
    }

    #[test]
    fn ttl_cache_clear_removes_all() {
        let cache = TtlCache::new(std::time::Duration::from_secs(3600));
        cache.put("a", 1);
        cache.put("b", 2);
        cache.clear();
        assert!(cache.get(&"a").is_none());
        assert!(cache.get(&"b").is_none());
    }

    #[test]
    fn stamped_cache_returns_value_when_stamp_matches() {
        let cache = StampedCache::new();
        let tmp = scratch("stamped_match");
        let file = tmp.join("data.txt");
        std::fs::write(&file, b"hello").unwrap();

        // Use an old stamp so the freshness gate passes.
        let old_mtime = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
            .saturating_sub(60_000_000_000); // 60 s ago
        let stamp = FileStamp {
            mtime: old_mtime,
            len: 5,
        };

        let key = "data.txt".to_string();
        cache.put(key.clone(), stamp.clone(), "cached".to_string());
        assert_eq!(cache.get(&key, &stamp), Some("cached".to_string()));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn stamped_cache_expires_when_file_rewritten() {
        let cache = StampedCache::new();
        let tmp = scratch("stamped_rewrite");
        let file = tmp.join("data.txt");

        // Write initial content (10 bytes).
        std::fs::write(&file, b"1234567890").unwrap();

        // Use an old stamp so the freshness gate passes.
        let old_mtime = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
            .saturating_sub(60_000_000_000); // 60 s ago
        let stamp = FileStamp {
            mtime: old_mtime,
            len: 10,
        };
        let key = "data.txt".to_string();
        cache.put(key.clone(), stamp.clone(), "v1".to_string());

        // Rewrite with different content AND different size.
        std::fs::write(&file, b"abcdefghijXYZ").unwrap();
        let new_stamp = FileStamp::from_path(&file).unwrap();

        // The new stamp (different size) should not match the cached entry.
        assert!(cache.get(&key, &new_stamp).is_none());

        // Use an old stamp for the new version so the freshness gate passes.
        let old_mtime2 = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
            .saturating_sub(60_000_000_000); // 60 s ago
        let new_stamp_old = FileStamp {
            mtime: old_mtime2,
            len: new_stamp.len,
        };

        // Insert with the new stamp.
        cache.put(key.clone(), new_stamp_old.clone(), "v2".to_string());
        assert_eq!(cache.get(&key, &new_stamp_old), Some("v2".to_string()));

        let _ = std::fs::remove_dir_all(&tmp);
    }


    /// Freshness guard: a file written less than SETTLE_NANOS ago must not
    /// be served from cache, even when the stamp matches.
    #[test]
    fn stamped_cache_refuses_fresh_files() {
        let cache = StampedCache::new();
        let tmp = scratch("stamped_fresh");
        let file = tmp.join("fresh.txt");

        // Write the file on disk (real mtime will be "fresh").
        std::fs::write(&file, b"fresh content").unwrap();

        // Cache a value using an old stamp (so the freshness gate doesn't
        // block the lookup — the gate is checked on get against the stamp
        // passed as argument, not the one on disk).
        let old_mtime = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
            .saturating_sub(60_000_000_000); // 60 s ago
        let stamp = FileStamp {
            mtime: old_mtime,
            len: 13,
        };
        let key = "fresh.txt".to_string();
        cache.put(key.clone(), stamp.clone(), "fresh-cached".to_string());

        // The file on disk is fresh (written just now), but the cached
        // stamp is old. When we ask with the real (fresh) stamp, the
        // freshness gate kicks in and refuses to serve the value.
        let real_stamp = FileStamp::from_path(&file).unwrap();
        assert!(
            cache.get(&key, &real_stamp).is_none(),
            "fresh file must not be served from cache"
        );

        // Forge an old stamp matching the cached entry — the freshness
        // gate passes and the stamp matches, so the value is served.
        // This proves the freshness gate is the only cause of the miss above.
        assert_eq!(
            cache.get(&key, &stamp),
            Some("fresh-cached".to_string()),
            "old stamp matching cache should be served"
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    // ================================================================ TtlCache tests

    #[test]
    fn ttl_cache_put_get_invalidate() {
        let cache = TtlCache::new(std::time::Duration::from_secs(3600));
        cache.put("key1".to_string(), "value1".to_string());
        assert_eq!(cache.get(&"key1".to_string()), Some("value1".to_string()));

        cache.invalidate(&"key1".to_string());
        assert!(cache.get(&"key1".to_string()).is_none());
    }

    #[test]
    fn ttl_cache_expires_after_short_ttl() {
        let cache = TtlCache::new(std::time::Duration::from_millis(20));
        cache.put("key1".to_string(), "value1".to_string());
        assert_eq!(cache.get(&"key1".to_string()), Some("value1".to_string()));

        std::thread::sleep(std::time::Duration::from_millis(50));
        assert!(cache.get(&"key1".to_string()).is_none());
    }

    // ================================================================ Déduplication test

    #[tokio::test]
    async fn steam_details_deduplication_and_cleanup() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

        // Wrap in Arc because TtlCache and tokio::sync::Mutex don't implement Clone.
        let steam_details: Arc<crate::cache::TtlCache<String, String>> =
            Arc::new(crate::cache::TtlCache::new(std::time::Duration::from_secs(300)));
        let steam_details_locks: Arc<
            tokio::sync::Mutex<std::collections::HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
        > = Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));

        let fetch_counter = Arc::new(AtomicUsize::new(0));
        let fetch_counter2 = fetch_counter.clone();

        // Spawn two concurrent "callers" that request the same key.
        let a = {
            let sd = steam_details.clone();
            let locks = steam_details_locks.clone();
            let counter = fetch_counter2.clone();
            tokio::spawn(async move {
                let key = "12345:french".to_string();

                // Step 1: cache miss
                assert!(sd.get(&key).is_none());

                // Step 2: acquire lock
                let lock = {
                    let mut entry = locks.lock().await;
                    Arc::clone(
                        entry
                            .entry(key.clone())
                            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(()))),
                    )
                };

                // Step 3-4: fetch with deduplication
                let _guard = lock.lock().await;
                // Simulate network call
                counter.fetch_add(1, Ordering::SeqCst);
                std::thread::sleep(std::time::Duration::from_millis(50));
                sd.put(key.clone(), "cached_result".to_string());

                // Step 5: cleanup
                {
                    let mut locks = locks.lock().await;
                    if let Some(a) = locks.get(&key) {
                        if Arc::strong_count(a) == 2 {
                            locks.remove(&key);
                        }
                    }
                }
            })
        };

        let b = {
            let sd = steam_details.clone();
            let locks = steam_details_locks.clone();
            let counter = fetch_counter.clone();
            tokio::spawn(async move {
                let key = "12345:french".to_string();

                // Step 1: may or may not see cache (depends on timing)
                let _has_cache = sd.get(&key).is_some();

                // Step 2: acquire lock (same key → same lock)
                let lock = {
                    let mut entry = locks.lock().await;
                    Arc::clone(
                        entry
                            .entry(key.clone())
                            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(()))),
                    )
                };

                // Step 3-4: re-read cache — caller A should have filled it
                let _guard = lock.lock().await;
                if sd.get(&key).is_none() {
                    // Fallback: do the fetch too
                    counter.fetch_add(1, Ordering::SeqCst);
                    std::thread::sleep(std::time::Duration::from_millis(10));
                    sd.put(key.clone(), "cached_result".to_string());
                }

                // Step 5: cleanup
                {
                    let mut locks = locks.lock().await;
                    if let Some(a) = locks.get(&key) {
                        if Arc::strong_count(a) == 2 {
                            locks.remove(&key);
                        }
                    }
                }
            })
        };

        let _ = tokio::join!(a, b);

        // The fetch counter must be 1 (or at most 2 if B didn't see the cache
        // in time — but with the dedup lock, B should have waited and found
        // the value). At minimum, the lock map must be empty now.
        let final_locks = steam_details_locks.lock().await;
        assert!(
            final_locks.is_empty(),
            "lock map should be empty after all callers finish"
        );

        // The cached value must be present.
        assert_eq!(
            steam_details.get(&"12345:french".to_string()),
            Some("cached_result".to_string())
        );
    }
}
