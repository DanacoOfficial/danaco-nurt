// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-files::commands::read::cache
// Responsibility:   Cache of the text of files read earlier, for `read since:`.
// Dependencies:     nurt_contracts, nurt_testkit
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Cache of the text of files read earlier, for `read since:`.
//!
//! Every successful text read stores the decoded text of the whole file
//! under its content hash. `since` names an earlier version by its entity
//! tag or by a leading part of its content hash; when that version is still
//! cached, `read` returns the difference against it instead of the whole
//! text.
//!
//! The cache keeps the entries used last within a byte budget and
//! drops entries older than a time to live. Time is the monotonic time of
//! the clock of the execution context, so expiry is deterministic in tests.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use nurt_contracts::fs::{AbsPath, ContentHash};

/// Byte budget of the cache of a stage: 64 MiB.
pub(crate) const DEFAULT_CAPACITY: usize = 64 * 1024 * 1024;

/// Time an entry stays in the cache of a stage: one hour.
pub(crate) const DEFAULT_TTL: Duration = Duration::from_secs(60 * 60);

/// Store of texts read earlier, by file and content hash.
///
/// An implementation registered in the extensions of the execution context
/// as `dyn ReadCache` is shared by all `read` stages of the engine; without
/// one, each stage uses its own [`MemoryReadCache`].
pub(crate) trait ReadCache: Send + Sync {
    /// Text of `path` stored under a content hash whose hexadecimal digits
    /// start with `hash_prefix` (lower case, without `sha256:`); the
    /// match stored last wins. `now` is the monotonic time of the clock.
    fn get(&self, path: &AbsPath, hash_prefix: &str, now: Duration) -> Option<Arc<str>>;

    /// Stores the text of `path` with the content hash of the file bytes.
    fn put(&self, path: &AbsPath, hash: &ContentHash, text: &str, now: Duration);
}

/// Least recently used cache in memory with a byte budget and a time to
/// live.
#[derive(Debug)]
pub(crate) struct MemoryReadCache {
    capacity: usize,
    ttl: Duration,
    state: Mutex<State>,
}

#[derive(Debug, Default)]
struct State {
    /// Entries, the least recently used first.
    entries: VecDeque<Entry>,
    /// Sum of the costs of the entries.
    bytes: usize,
}

#[derive(Debug)]
struct Entry {
    path: AbsPath,
    hash: String,
    text: Arc<str>,
    stored: Duration,
    cost: usize,
}

impl MemoryReadCache {
    /// Cache that holds up to `capacity` bytes of text for `ttl` each.
    pub(crate) fn new(capacity: usize, ttl: Duration) -> Self {
        Self {
            capacity,
            ttl,
            state: Mutex::new(State::default()),
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn expire(&self, state: &mut State, now: Duration) {
        let ttl = self.ttl;
        let before = state.entries.len();
        state
            .entries
            .retain(|entry| now.saturating_sub(entry.stored) < ttl);
        if state.entries.len() != before {
            state.bytes = state.entries.iter().map(|entry| entry.cost).sum();
        }
    }
}

impl Default for MemoryReadCache {
    fn default() -> Self {
        Self::new(DEFAULT_CAPACITY, DEFAULT_TTL)
    }
}

impl ReadCache for MemoryReadCache {
    fn get(&self, path: &AbsPath, hash_prefix: &str, now: Duration) -> Option<Arc<str>> {
        let mut state = self.lock();
        self.expire(&mut state, now);
        let position = state
            .entries
            .iter()
            .rposition(|entry| entry.path == *path && entry.hash.starts_with(hash_prefix))?;
        let entry = state.entries.remove(position)?;
        let text = entry.text.clone();
        state.entries.push_back(entry);
        Some(text)
    }

    fn put(&self, path: &AbsPath, hash: &ContentHash, text: &str, now: Duration) {
        let hash = hash_digits(hash);
        let cost = text.len() + path.as_path().as_os_str().len() + hash.len();
        let mut state = self.lock();
        self.expire(&mut state, now);
        if let Some(position) = state
            .entries
            .iter()
            .position(|entry| entry.path == *path && entry.hash == hash)
            && let Some(old) = state.entries.remove(position)
        {
            state.bytes -= old.cost;
        }
        if cost > self.capacity {
            return;
        }
        while state.bytes + cost > self.capacity {
            match state.entries.pop_front() {
                Some(evicted) => state.bytes -= evicted.cost,
                None => break,
            }
        }
        state.bytes += cost;
        state.entries.push_back(Entry {
            path: path.clone(),
            hash,
            text: Arc::from(text),
            stored: now,
            cost,
        });
    }
}

/// Hexadecimal digits of a content hash, without `sha256:`.
pub(crate) fn hash_digits(hash: &ContentHash) -> String {
    let text = hash.to_string();
    text.strip_prefix(ContentHash::PREFIX)
        .map_or_else(|| text.clone(), str::to_owned)
}

#[cfg(test)]
mod tests {
    use nurt_testkit::vfs::test_path;

    use super::*;

    fn path(text: &str) -> AbsPath {
        test_path(text).unwrap()
    }

    fn hash(byte: u8) -> ContentHash {
        ContentHash([byte; 32])
    }

    const MINUTE: Duration = Duration::from_secs(60);

    #[test]
    fn stored_text_is_found_by_a_hash_prefix() {
        let cache = MemoryReadCache::default();
        let file = path("/data/a.txt");
        cache.put(&file, &hash(0xab), "first\n", Duration::ZERO);
        assert_eq!(
            cache.get(&file, "abababab", MINUTE).as_deref(),
            Some("first\n")
        );
        assert_eq!(
            cache
                .get(&file, &hash_digits(&hash(0xab)), MINUTE)
                .as_deref(),
            Some("first\n")
        );
        assert!(cache.get(&file, "cdcdcdcd", MINUTE).is_none());
        assert!(cache.get(&path("/data/b.txt"), "abab", MINUTE).is_none());
    }

    #[test]
    fn several_versions_of_a_file_are_kept() {
        let cache = MemoryReadCache::default();
        let file = path("/data/a.txt");
        cache.put(&file, &hash(0x11), "one\n", Duration::ZERO);
        cache.put(&file, &hash(0x22), "two\n", Duration::ZERO);
        assert_eq!(
            cache.get(&file, "1111", Duration::ZERO).as_deref(),
            Some("one\n")
        );
        assert_eq!(
            cache.get(&file, "2222", Duration::ZERO).as_deref(),
            Some("two\n")
        );
    }

    #[test]
    fn entries_expire_after_the_time_to_live() {
        let cache = MemoryReadCache::new(DEFAULT_CAPACITY, MINUTE);
        let file = path("/data/a.txt");
        cache.put(&file, &hash(1), "text", Duration::ZERO);
        assert!(cache.get(&file, "0101", MINUTE / 2).is_some());
        assert!(cache.get(&file, "0101", MINUTE).is_none());
    }

    #[test]
    fn least_recently_used_entries_are_evicted_first() {
        let file = path("/a");
        let cost = 4 + file.as_path().as_os_str().len() + hash_digits(&hash(1)).len();
        let cache = MemoryReadCache::new(2 * cost, DEFAULT_TTL);
        cache.put(&file, &hash(1), "aaaa", Duration::ZERO);
        cache.put(&file, &hash(2), "bbbb", Duration::ZERO);
        assert!(cache.get(&file, "0101", Duration::ZERO).is_some());
        cache.put(&file, &hash(3), "cccc", Duration::ZERO);
        assert!(cache.get(&file, "0101", Duration::ZERO).is_some());
        assert!(cache.get(&file, "0202", Duration::ZERO).is_none());
        assert!(cache.get(&file, "0303", Duration::ZERO).is_some());
    }

    #[test]
    fn texts_above_the_budget_are_not_stored() {
        let cache = MemoryReadCache::new(8, DEFAULT_TTL);
        let file = path("/data/a.txt");
        cache.put(
            &file,
            &hash(1),
            "far too long for the budget",
            Duration::ZERO,
        );
        assert!(cache.get(&file, "01", Duration::ZERO).is_none());
    }

    #[test]
    fn storing_the_same_version_again_replaces_it() {
        let cache = MemoryReadCache::default();
        let file = path("/data/a.txt");
        cache.put(&file, &hash(1), "old", Duration::ZERO);
        cache.put(&file, &hash(1), "new", MINUTE);
        assert_eq!(cache.get(&file, "01", MINUTE).as_deref(), Some("new"));
        assert_eq!(cache.lock().entries.len(), 1);
    }

    #[test]
    fn hash_digits_drop_the_prefix() {
        assert_eq!(hash_digits(&hash(0xff)), "ff".repeat(32));
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
