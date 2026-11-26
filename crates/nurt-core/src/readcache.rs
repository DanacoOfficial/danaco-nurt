// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Cache of file content seen by earlier reads.
//!
//! Commands that compare a file with the content seen by an earlier read
//! (`read since:`) keep that content here, keyed by path and entity tag. The
//! cache holds at most [`DEFAULT_CAPACITY`] bytes, evicts the least recently
//! used entries first and forgets entries after [`DEFAULT_TTL`]. With a
//! directory (the `readcache` area of the state directory) entries are also
//! written to disk, so that later processes find them. Writing to disk is
//! best effort: an entry that cannot be written stays in memory.
//!
//! The engine registers its cache in the extensions of every execution
//! context, where stages find it with
//! `ctx.extensions().get::<ReadCache>()`.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use nurt_contracts::env::{Clock, WallTime};
use nurt_contracts::fs::Etag;
use serde::{Deserialize, Serialize};

use crate::state::{read_optional, sha256_hex, write_atomic};

/// Default capacity of the cache in bytes (64 MiB).
pub const DEFAULT_CAPACITY: u64 = 64 * 1024 * 1024;

/// Default lifetime of an entry (one hour).
pub const DEFAULT_TTL: Duration = Duration::from_secs(60 * 60);

const FILE_NAME_DIGITS: usize = 32;
const DATA_EXTENSION: &str = "bin";
const META_EXTENSION: &str = "json";

type Key = (PathBuf, Etag);

struct Entry {
    data: Arc<[u8]>,
    stored: WallTime,
    tick: u64,
}

#[derive(Default)]
struct Lru {
    entries: HashMap<Key, Entry>,
    order: BTreeMap<u64, Key>,
    tick: u64,
    bytes: u64,
}

impl Lru {
    fn next_tick(&mut self) -> u64 {
        self.tick += 1;
        self.tick
    }

    fn remove(&mut self, key: &Key) -> Option<Entry> {
        let entry = self.entries.remove(key)?;
        self.order.remove(&entry.tick);
        self.bytes = self.bytes.saturating_sub(byte_len(&entry.data));
        Some(entry)
    }

    fn touch(&mut self, key: &Key) -> Option<Arc<[u8]>> {
        let tick = self.next_tick();
        let entry = self.entries.get_mut(key)?;
        self.order.remove(&entry.tick);
        entry.tick = tick;
        self.order.insert(tick, key.clone());
        Some(Arc::clone(&entry.data))
    }

    fn insert(&mut self, key: Key, data: Arc<[u8]>, stored: WallTime) {
        self.remove(&key);
        let tick = self.next_tick();
        self.bytes += byte_len(&data);
        self.order.insert(tick, key.clone());
        self.entries.insert(key, Entry { data, stored, tick });
    }

    fn evict_to(&mut self, capacity: u64) -> Vec<Key> {
        let mut evicted = Vec::new();
        while self.bytes > capacity {
            let Some((_, key)) = self.order.pop_first() else {
                break;
            };
            if let Some(entry) = self.entries.remove(&key) {
                self.bytes = self.bytes.saturating_sub(byte_len(&entry.data));
            }
            evicted.push(key);
        }
        evicted
    }
}

fn byte_len(data: &[u8]) -> u64 {
    u64::try_from(data.len()).unwrap_or(u64::MAX)
}

#[derive(Serialize, Deserialize)]
struct DiskMeta {
    path: PathBuf,
    etag: Etag,
    stored: WallTime,
}

/// Least recently used cache of file content, keyed by path and entity tag.
///
/// # Examples
///
/// ```
/// use std::path::Path;
/// use std::sync::Arc;
/// use nurt_contracts::fs::{Etag, FileId};
/// use nurt_core::readcache::ReadCache;
/// use nurt_testkit::clock::VirtualClock;
///
/// let cache = ReadCache::new(Arc::new(VirtualClock::new()));
/// let etag: Etag = serde_json::from_value(serde_json::json!({
///     "size": 5, "modified_ns": 1, "file_id": {"volume": 1, "low": 2, "high": 0}
/// }))?;
/// cache.put(Path::new("/srv/notes.txt"), etag.clone(), b"hello".to_vec());
/// assert_eq!(cache.get(Path::new("/srv/notes.txt"), &etag).as_deref(), Some(&b"hello"[..]));
/// # Ok::<(), serde_json::Error>(())
/// ```
pub struct ReadCache {
    clock: Arc<dyn Clock>,
    capacity: u64,
    ttl: Duration,
    dir: Option<PathBuf>,
    lru: Mutex<Lru>,
}

impl std::fmt::Debug for ReadCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReadCache")
            .field("capacity", &self.capacity)
            .field("ttl", &self.ttl)
            .field("dir", &self.dir)
            .field("entries", &self.entries())
            .finish_non_exhaustive()
    }
}

impl ReadCache {
    /// Cache in memory with the default capacity and lifetime, aging entries
    /// by `clock`.
    #[must_use]
    pub fn new(clock: Arc<dyn Clock>) -> Self {
        Self {
            clock,
            capacity: DEFAULT_CAPACITY,
            ttl: DEFAULT_TTL,
            dir: None,
            lru: Mutex::new(Lru::default()),
        }
    }

    /// The same cache holding at most `bytes` bytes.
    #[must_use]
    pub fn with_capacity(mut self, bytes: u64) -> Self {
        self.capacity = bytes;
        self
    }

    /// The same cache forgetting entries after `ttl`.
    #[must_use]
    pub fn with_ttl(mut self, ttl: Duration) -> Self {
        self.ttl = ttl;
        self
    }

    /// The same cache, also keeping entries in `dir`.
    #[must_use]
    pub fn with_dir(mut self, dir: PathBuf) -> Self {
        self.dir = Some(dir);
        self
    }

    /// Content kept for `path` in the version `etag`, when it has not
    /// expired.
    #[must_use]
    pub fn get(&self, path: &Path, etag: &Etag) -> Option<Arc<[u8]>> {
        let key = (path.to_path_buf(), etag.clone());
        let now = self.clock.now();
        {
            let mut lru = self.lock();
            let stored = lru.entries.get(&key).map(|entry| entry.stored);
            match stored {
                Some(stored) if self.is_fresh(stored, now) => return lru.touch(&key),
                Some(_) => {
                    lru.remove(&key);
                    self.remove_files(&key);
                    return None;
                }
                None => {}
            }
        }
        let (data, stored) = self.read_disk(&key)?;
        if !self.is_fresh(stored, now) {
            self.remove_files(&key);
            return None;
        }
        self.insert(key, Arc::clone(&data), stored);
        Some(data)
    }

    /// Keeps `data` as the content of `path` in the version `etag`.
    ///
    /// Content larger than the capacity is not kept.
    pub fn put(&self, path: &Path, etag: Etag, data: impl Into<Arc<[u8]>>) {
        let data = data.into();
        if byte_len(&data) > self.capacity {
            return;
        }
        let key = (path.to_path_buf(), etag);
        let stored = self.clock.now();
        self.write_disk(&key, &data, stored);
        self.insert(key, data, stored);
    }

    /// Forgets every version of `path`.
    pub fn remove(&self, path: &Path) {
        let mut lru = self.lock();
        let keys: Vec<Key> = lru
            .entries
            .keys()
            .filter(|(candidate, _)| candidate == path)
            .cloned()
            .collect();
        for key in keys {
            lru.remove(&key);
            self.remove_files(&key);
        }
    }

    /// Number of entries in memory.
    #[must_use]
    pub fn entries(&self) -> usize {
        self.lock().entries.len()
    }

    /// Bytes of content in memory.
    #[must_use]
    pub fn bytes(&self) -> u64 {
        self.lock().bytes
    }

    fn lock(&self) -> MutexGuard<'_, Lru> {
        self.lru.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn is_fresh(&self, stored: WallTime, now: WallTime) -> bool {
        now.saturating_since(stored) < self.ttl
    }

    fn insert(&self, key: Key, data: Arc<[u8]>, stored: WallTime) {
        let mut lru = self.lock();
        lru.insert(key, data, stored);
        for evicted in lru.evict_to(self.capacity) {
            self.remove_files(&evicted);
        }
    }

    fn file_stem(&self, key: &Key) -> Option<PathBuf> {
        let dir = self.dir.as_ref()?;
        let mut seed = key.0.as_os_str().as_encoded_bytes().to_vec();
        seed.extend(serde_json::to_vec(&key.1).ok()?);
        Some(dir.join(&sha256_hex(&seed)[..FILE_NAME_DIGITS]))
    }

    fn write_disk(&self, key: &Key, data: &[u8], stored: WallTime) {
        let Some(stem) = self.file_stem(key) else {
            return;
        };
        let meta = DiskMeta {
            path: key.0.clone(),
            etag: key.1.clone(),
            stored,
        };
        let Ok(meta) = serde_json::to_vec(&meta) else {
            return;
        };
        if write_atomic(&stem.with_extension(DATA_EXTENSION), data).is_ok() {
            let _ = write_atomic(&stem.with_extension(META_EXTENSION), &meta);
        }
    }

    fn read_disk(&self, key: &Key) -> Option<(Arc<[u8]>, WallTime)> {
        let stem = self.file_stem(key)?;
        let meta = read_optional(&stem.with_extension(META_EXTENSION)).ok()??;
        let meta: DiskMeta = serde_json::from_slice(&meta).ok()?;
        if meta.path != key.0 || meta.etag != key.1 {
            return None;
        }
        let data = read_optional(&stem.with_extension(DATA_EXTENSION)).ok()??;
        Some((Arc::from(data), meta.stored))
    }

    fn remove_files(&self, key: &Key) {
        if let Some(stem) = self.file_stem(key) {
            let _ = fs::remove_file(stem.with_extension(META_EXTENSION));
            let _ = fs::remove_file(stem.with_extension(DATA_EXTENSION));
        }
    }
}

#[cfg(test)]
mod tests {
    use nurt_testkit::clock::VirtualClock;
    use serde_json::json;

    use super::*;

    fn etag(size: u64) -> Etag {
        serde_json::from_value(json!({
            "size": size, "modified_ns": 1, "file_id": {"volume": 1, "low": 2, "high": 0}
        }))
        .unwrap()
    }

    fn cache(clock: &Arc<VirtualClock>) -> ReadCache {
        ReadCache::new(Arc::clone(clock) as Arc<dyn Clock>)
    }

    #[test]
    fn content_is_found_by_path_and_tag() {
        let clock = Arc::new(VirtualClock::new());
        let cache = cache(&clock);
        let path = Path::new("/srv/a.txt");
        cache.put(path, etag(1), b"one".to_vec());
        cache.put(path, etag(2), b"two".to_vec());
        assert_eq!(cache.get(path, &etag(1)).as_deref(), Some(&b"one"[..]));
        assert_eq!(cache.get(path, &etag(2)).as_deref(), Some(&b"two"[..]));
        assert!(cache.get(path, &etag(3)).is_none());
        assert!(cache.get(Path::new("/srv/b.txt"), &etag(1)).is_none());
        assert_eq!(cache.entries(), 2);
        assert_eq!(cache.bytes(), 6);
    }

    #[test]
    fn entries_expire_after_the_lifetime() {
        let clock = Arc::new(VirtualClock::new());
        let cache = cache(&clock).with_ttl(Duration::from_secs(60));
        let path = Path::new("/srv/a.txt");
        cache.put(path, etag(1), b"one".to_vec());
        clock.advance(Duration::from_secs(59));
        assert!(cache.get(path, &etag(1)).is_some());
        clock.advance(Duration::from_secs(1));
        assert!(cache.get(path, &etag(1)).is_none());
        assert_eq!(cache.entries(), 0);
    }

    #[test]
    fn least_recently_used_entries_are_evicted() {
        let clock = Arc::new(VirtualClock::new());
        let cache = cache(&clock).with_capacity(8);
        cache.put(Path::new("/a"), etag(1), vec![0; 4]);
        cache.put(Path::new("/b"), etag(1), vec![0; 4]);
        assert!(cache.get(Path::new("/a"), &etag(1)).is_some());
        cache.put(Path::new("/c"), etag(1), vec![0; 4]);
        assert!(cache.get(Path::new("/a"), &etag(1)).is_some());
        assert!(cache.get(Path::new("/b"), &etag(1)).is_none());
        assert!(cache.get(Path::new("/c"), &etag(1)).is_some());
        assert_eq!(cache.bytes(), 8);
    }

    #[test]
    fn content_larger_than_the_capacity_is_not_kept() {
        let clock = Arc::new(VirtualClock::new());
        let cache = cache(&clock).with_capacity(4);
        cache.put(Path::new("/a"), etag(1), vec![0; 5]);
        assert_eq!(cache.entries(), 0);
    }

    #[test]
    fn putting_the_same_version_again_replaces_its_content() {
        let clock = Arc::new(VirtualClock::new());
        let cache = cache(&clock);
        let path = Path::new("/srv/a.txt");
        cache.put(path, etag(1), b"ab".to_vec());
        cache.put(path, etag(1), "ąę".as_bytes().to_vec());
        assert_eq!(cache.entries(), 1);
        assert_eq!(cache.bytes(), 4);
        assert_eq!(cache.get(path, &etag(1)).as_deref(), Some("ąę".as_bytes()));
    }

    #[test]
    fn content_of_exactly_the_capacity_is_kept_and_empty_content_is_allowed() {
        let clock = Arc::new(VirtualClock::new());
        let cache = cache(&clock).with_capacity(4);
        cache.put(Path::new("/a"), etag(1), vec![0; 4]);
        cache.put(Path::new("/b"), etag(1), Vec::<u8>::new());
        assert_eq!(cache.entries(), 2);
        assert_eq!(cache.bytes(), 4);
        assert_eq!(
            cache.get(Path::new("/b"), &etag(1)).as_deref(),
            Some(&b""[..])
        );
    }

    #[test]
    fn remove_forgets_every_version() {
        let clock = Arc::new(VirtualClock::new());
        let cache = cache(&clock);
        cache.put(Path::new("/a"), etag(1), b"1".to_vec());
        cache.put(Path::new("/a"), etag(2), b"2".to_vec());
        cache.put(Path::new("/b"), etag(1), b"3".to_vec());
        cache.remove(Path::new("/a"));
        assert_eq!(cache.entries(), 1);
    }

    #[test]
    fn disk_entries_serve_a_new_process() {
        let temp = tempfile::tempdir().unwrap();
        let clock = Arc::new(VirtualClock::new());
        let first = cache(&clock).with_dir(temp.path().to_path_buf());
        first.put(Path::new("/srv/a.txt"), etag(1), b"kept".to_vec());
        let second = cache(&clock).with_dir(temp.path().to_path_buf());
        assert_eq!(
            second.get(Path::new("/srv/a.txt"), &etag(1)).as_deref(),
            Some(&b"kept"[..])
        );
        clock.advance(DEFAULT_TTL);
        let third = cache(&clock).with_dir(temp.path().to_path_buf());
        assert!(third.get(Path::new("/srv/a.txt"), &etag(1)).is_none());
        assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 0);
        assert!(format!("{third:?}").contains("ReadCache"));
    }
}
