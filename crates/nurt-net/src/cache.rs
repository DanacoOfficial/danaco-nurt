// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! HTTP response cache.
//!
//! The cache lives in the state directory of the engine, outside the
//! journal of operations. An entry holds the decompressed body of a `200`
//! response to `GET`, limited to the size requested when it was stored, and
//! the headers needed to revalidate it. The key covers the method, the
//! normalized address and the client identity (`User-Agent` and the
//! additional certificate file), so changing either makes earlier entries
//! unreachable. Entries are written atomically by renaming a complete
//! temporary file; the least recently used entries are removed when the
//! cache grows over its size limit.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Subdirectory of the state directory holding the cache.
pub const DIR_NAME: &str = "http-cache";
/// Time an entry is used without revalidation.
pub const DEFAULT_TTL: Duration = Duration::from_secs(15 * 60);
/// Largest total size of the cached bodies.
pub const DEFAULT_MAX_SIZE: u64 = 64 * 1024 * 1024;

const META_SUFFIX: &str = ".meta.json";
const BODY_SUFFIX: &str = ".body";

/// A cached response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CachedResponse {
    /// Address of the request.
    pub url: String,
    /// Address that gave the response, after redirects.
    pub final_url: String,
    /// Status code.
    pub status: u16,
    /// Headers, names in lower case.
    pub headers: Vec<(String, String)>,
    /// Decompressed body.
    #[serde(skip)]
    pub body: Vec<u8>,
    /// The body was cut at `max_size`.
    pub truncated: bool,
    /// Size limit in force when the body was read.
    pub max_size: u64,
    /// Time the entry was stored or last revalidated, in seconds since the
    /// Unix epoch.
    pub stored: i64,
    /// Time the entry was last used, in seconds since the Unix epoch.
    pub used: i64,
}

impl CachedResponse {
    /// First value of a header.
    #[must_use]
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(header, _)| header.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    /// Whether the entry is younger than `ttl` at `now`.
    #[must_use]
    pub fn is_fresh(&self, now: i64, ttl: Duration) -> bool {
        let ttl = i64::try_from(ttl.as_secs()).unwrap_or(i64::MAX);
        now.saturating_sub(self.stored) < ttl
    }

    /// Whether the entry can answer a request that reads up to `max_size`
    /// bytes: a cut body cannot answer a request with a larger limit.
    #[must_use]
    pub fn serves(&self, max_size: u64) -> bool {
        !self.truncated || max_size <= self.max_size
    }
}

/// Cache directory with its limits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheStore {
    dir: PathBuf,
    ttl: Duration,
    max_size: u64,
}

impl CacheStore {
    /// A cache in `dir`.
    #[must_use]
    pub fn new(dir: PathBuf, ttl: Duration, max_size: u64) -> Self {
        Self { dir, ttl, max_size }
    }

    /// The cache of a state directory with the default limits.
    #[must_use]
    pub fn in_state_dir(state: &Path) -> Self {
        Self::new(state.join(DIR_NAME), DEFAULT_TTL, DEFAULT_MAX_SIZE)
    }

    /// Time an entry is used without revalidation.
    #[must_use]
    pub fn ttl(&self) -> Duration {
        self.ttl
    }

    /// Key of a request: SHA-256 of the method, the normalized address and
    /// the client identity, in hexadecimal.
    ///
    /// ```
    /// use nurt_net::cache::CacheStore;
    ///
    /// let a = CacheStore::key("get", "https://a.test/", "nurt/0.1.0");
    /// assert_eq!(a.len(), 64);
    /// assert_ne!(a, CacheStore::key("get", "https://a.test/", "other"));
    /// ```
    #[must_use]
    pub fn key(method: &str, url: &str, identity: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(method.as_bytes());
        hasher.update(b"\n");
        hasher.update(url.as_bytes());
        hasher.update(b"\n");
        hasher.update(identity.as_bytes());
        hex::encode(hasher.finalize())
    }

    fn path(&self, key: &str, suffix: &str) -> PathBuf {
        self.dir.join(format!("{key}{suffix}"))
    }

    /// The entry stored under `key`; damaged or incomplete entries count as
    /// absent.
    #[must_use]
    pub fn load(&self, key: &str) -> Option<CachedResponse> {
        let mut entry = self.load_meta(key)?;
        entry.body = fs::read(self.path(key, BODY_SUFFIX)).ok()?;
        Some(entry)
    }

    /// The entry stored under `key` without its body, which stays on disk:
    /// cleaning the cache reads the use times of all entries and must not
    /// read all bodies for it.
    fn load_meta(&self, key: &str) -> Option<CachedResponse> {
        let meta = fs::read(self.path(key, META_SUFFIX)).ok()?;
        serde_json::from_slice(&meta).ok()
    }

    /// Stores an entry, then removes the least recently used entries while
    /// the bodies exceed the size limit.
    ///
    /// # Errors
    ///
    /// The I/O error of the cache directory.
    pub fn store(&self, key: &str, entry: &CachedResponse) -> io::Result<()> {
        fs::create_dir_all(&self.dir)?;
        self.write_atomic(&self.path(key, BODY_SUFFIX), &entry.body)?;
        self.write_meta(key, entry)?;
        self.prune()
    }

    /// Records a revalidation or a use of an entry.
    ///
    /// # Errors
    ///
    /// The I/O error of the cache directory.
    pub fn touch(&self, key: &str, stored: Option<i64>, used: i64) -> io::Result<()> {
        let meta = fs::read(self.path(key, META_SUFFIX))?;
        let mut entry: CachedResponse = serde_json::from_slice(&meta)?;
        if let Some(stored) = stored {
            entry.stored = stored;
        }
        entry.used = used;
        self.write_meta(key, &entry)
    }

    fn write_meta(&self, key: &str, entry: &CachedResponse) -> io::Result<()> {
        let meta = serde_json::to_vec(entry)?;
        self.write_atomic(&self.path(key, META_SUFFIX), &meta)
    }

    fn write_atomic(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        static SEQUENCE: AtomicU64 = AtomicU64::new(0);
        let temp = self.dir.join(format!(
            ".tmp-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let written = fs::write(&temp, bytes).and_then(|()| fs::rename(&temp, path));
        if written.is_err() {
            let _ = fs::remove_file(&temp);
        }
        written
    }

    fn prune(&self) -> io::Result<()> {
        let mut entries = Vec::new();
        let mut total = 0_u64;
        for item in fs::read_dir(&self.dir)? {
            let path = item?.path();
            let Some(key) = path
                .file_name()
                .and_then(|name| name.to_str())
                .and_then(|name| name.strip_suffix(META_SUFFIX))
            else {
                continue;
            };
            let body = fs::metadata(self.path(key, BODY_SUFFIX));
            let size = body.as_ref().map_or(0, |meta| meta.len());
            // An entry without a body is damaged and goes first.
            let used = body
                .ok()
                .and_then(|_| self.load_meta(key))
                .map_or(i64::MIN, |entry| entry.used);
            total += size;
            entries.push((used, key.to_owned(), size));
        }
        entries.sort();
        for (_, key, size) in entries {
            if total <= self.max_size {
                break;
            }
            let _ = fs::remove_file(self.path(&key, META_SUFFIX));
            let _ = fs::remove_file(self.path(&key, BODY_SUFFIX));
            total = total.saturating_sub(size);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(body: &[u8], used: i64) -> CachedResponse {
        CachedResponse {
            url: "https://a.test/".to_owned(),
            final_url: "https://a.test/".to_owned(),
            status: 200,
            headers: vec![("etag".to_owned(), "\"v1\"".to_owned())],
            body: body.to_vec(),
            truncated: false,
            max_size: 100,
            stored: used,
            used,
        }
    }

    #[test]
    fn stores_loads_and_touches_entries() {
        let dir = tempfile::tempdir().unwrap();
        let store = CacheStore::in_state_dir(dir.path());
        store.store("k", &entry(b"body", 10)).unwrap();
        let loaded = store.load("k").unwrap();
        assert_eq!(loaded.body, b"body");
        assert_eq!(loaded.header("ETag"), Some("\"v1\""));
        assert!(loaded.is_fresh(10 + 899, DEFAULT_TTL));
        assert!(!loaded.is_fresh(10 + 900, DEFAULT_TTL));
        store.touch("k", Some(50), 60).unwrap();
        let touched = store.load("k").unwrap();
        assert_eq!((touched.stored, touched.used), (50, 60));
        assert!(store.load("missing").is_none());
    }

    #[test]
    fn truncated_entries_do_not_serve_larger_limits() {
        let mut cut = entry(b"abc", 0);
        cut.truncated = true;
        assert!(cut.serves(100));
        assert!(!cut.serves(101));
        assert!(entry(b"abc", 0).serves(1000));
    }

    #[test]
    fn the_metadata_of_an_entry_is_read_without_its_body() {
        let dir = tempfile::tempdir().unwrap();
        let store = CacheStore::in_state_dir(dir.path());
        store.store("k", &entry(b"body", 10)).unwrap();
        fs::remove_file(store.path("k", BODY_SUFFIX)).unwrap();
        let meta = store.load_meta("k").unwrap();
        assert_eq!(meta.used, 10);
        assert!(meta.body.is_empty());
        assert!(store.load("k").is_none());
    }

    #[test]
    fn an_entry_without_a_body_is_evicted_before_a_used_one() {
        let dir = tempfile::tempdir().unwrap();
        let store = CacheStore::new(dir.path().to_path_buf(), DEFAULT_TTL, 10);
        store.store("damaged", &entry(b"1234", 100)).unwrap();
        fs::remove_file(store.path("damaged", BODY_SUFFIX)).unwrap();
        store.store("older", &entry(b"123456", 1)).unwrap();
        store.store("newer", &entry(b"123456", 2)).unwrap();
        assert!(store.load_meta("damaged").is_none());
        assert!(store.load("newer").is_some());
    }

    #[test]
    fn evicts_least_recently_used_entries() {
        let dir = tempfile::tempdir().unwrap();
        let store = CacheStore::new(dir.path().to_path_buf(), DEFAULT_TTL, 10);
        store.store("old", &entry(b"123456", 1)).unwrap();
        store.store("new", &entry(b"123456", 2)).unwrap();
        assert!(store.load("old").is_none());
        assert!(store.load("new").is_some());
    }
}
