// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Diagnostics published by a language server.
//!
//! Servers without `diagnosticProvider` push diagnostics with
//! `textDocument/publishDiagnostics` after a document is opened or changed.
//! Some servers do not publish again when a change leaves the diagnostics
//! unchanged, so a wait for a newer publication ends after a quiet period
//! without work in progress and then returns the last publication of the
//! document.
//!
//! # Examples
//!
//! ```
//! use std::time::Duration;
//! use nurt_contracts::stage::CancelToken;
//! use nurt_lspclient::diagnostics::{DiagnosticStore, QuietWait};
//! use serde_json::json;
//!
//! let store = DiagnosticStore::new();
//! store.publish(&json!({"uri": "file:///a.py", "version": 1, "diagnostics": []}));
//! let wait = QuietWait {
//!     timeout: Duration::from_secs(1),
//!     quiet: Duration::from_millis(50),
//!     cancel: &CancelToken::new(),
//! };
//! assert_eq!(store.wait("file:///a.py", 1, &wait, || false).map(|items| items.len()), Some(0));
//! ```

use std::collections::HashMap;
use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use lsp_types::Diagnostic;
use nurt_contracts::stage::CancelToken;
use serde_json::Value;

/// Longest single wait between checks of the cancellation token.
const CANCEL_POLL: Duration = Duration::from_millis(25);

#[derive(Debug, Clone)]
struct Published {
    version: i64,
    generation: u64,
    items: Vec<Diagnostic>,
}

#[derive(Debug, Default)]
struct State {
    documents: HashMap<String, Published>,
    generation: u64,
    closed: bool,
}

/// Limits of a wait for diagnostics.
#[derive(Debug, Clone, Copy)]
pub struct QuietWait<'a> {
    /// Longest wait.
    pub timeout: Duration,
    /// Silence of a server without work in progress after which the last
    /// publication applies.
    pub quiet: Duration,
    /// Ends the wait early.
    pub cancel: &'a CancelToken,
}

/// Latest diagnostics of every document, by URI.
#[derive(Debug, Default)]
pub struct DiagnosticStore {
    state: Mutex<State>,
    changed: Condvar,
}

impl DiagnosticStore {
    /// Store without diagnostics.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Applies the parameters of a `textDocument/publishDiagnostics`
    /// notification. Items that do not parse as diagnostics are skipped.
    pub fn publish(&self, params: &Value) {
        let Some(uri) = params.get("uri").and_then(Value::as_str) else {
            return;
        };
        let uri = key(uri);
        let uri = uri.as_str();
        let items = params
            .get("diagnostics")
            .and_then(Value::as_array)
            .map(|items| parse_items(items))
            .unwrap_or_default();
        let mut state = self.lock();
        let version = params
            .get("version")
            .and_then(Value::as_i64)
            .or_else(|| state.documents.get(uri).map(|known| known.version))
            .unwrap_or(0);
        state.generation += 1;
        let generation = state.generation;
        state.documents.insert(
            uri.to_owned(),
            Published {
                version,
                generation,
                items,
            },
        );
        drop(state);
        self.changed.notify_all();
    }

    /// Drops the diagnostics of a closed document.
    pub fn forget(&self, uri: &str) {
        self.lock().documents.remove(&key(uri));
    }

    /// Ends every wait; used when the server ends.
    pub fn close(&self) {
        self.lock().closed = true;
        self.changed.notify_all();
    }

    /// Diagnostics of `uri` for document version `version`.
    ///
    /// A publication for that version or a later one is returned at once.
    /// Otherwise the wait ends with the next publication of the document,
    /// or, when an older publication exists, after `wait.quiet` without a
    /// publication while `busy` returns `false`. `None` when the time limit
    /// elapsed, the wait was cancelled or the server ended.
    pub fn wait(
        &self,
        uri: &str,
        version: i64,
        wait: &QuietWait<'_>,
        busy: impl Fn() -> bool,
    ) -> Option<Vec<Diagnostic>> {
        let uri = key(uri);
        let uri = uri.as_str();
        let started = Instant::now();
        let mut state = self.lock();
        let baseline = state.documents.get(uri).cloned();
        if let Some(known) = &baseline
            && known.version >= version
        {
            return Some(known.items.clone());
        }
        let mut quiet_from = Instant::now();
        loop {
            if state.closed || wait.cancel.is_cancelled() {
                return None;
            }
            if let Some(current) = state.documents.get(uri)
                && baseline
                    .as_ref()
                    .is_none_or(|known| current.generation > known.generation)
            {
                return Some(current.items.clone());
            }
            let now = Instant::now();
            if baseline.is_some() && now.duration_since(quiet_from) >= wait.quiet {
                if busy() {
                    quiet_from = now;
                } else {
                    return baseline.map(|known| known.items);
                }
            }
            let elapsed = now.duration_since(started);
            if elapsed >= wait.timeout {
                return None;
            }
            let pause = (wait.timeout - elapsed).min(CANCEL_POLL);
            state = self
                .changed
                .wait_timeout(state, pause)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }
}

/// Key of a document: the `file:` URI written by this client for the same
/// path, so that spellings of the server (`file:///c%3A/x` and
/// `file:///C:/x`, `%7E` and `~`) meet; other URIs stay as they are.
fn key(uri: &str) -> String {
    crate::sync::uri_to_path(uri)
        .and_then(|path| crate::sync::path_to_uri(&path))
        .unwrap_or_else(|| uri.to_owned())
}

/// Diagnostics of a pull report or a publication; items that do not parse
/// are skipped.
#[must_use]
pub fn parse_items(items: &[Value]) -> Vec<Diagnostic> {
    items
        .iter()
        .filter_map(|item| serde_json::from_value::<Diagnostic>(item.clone()).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn quick<'a>(cancel: &'a CancelToken) -> QuietWait<'a> {
        QuietWait {
            timeout: Duration::from_millis(100),
            quiet: Duration::from_millis(10),
            cancel,
        }
    }

    #[cfg(unix)]
    #[test]
    fn spellings_of_one_file_uri_share_a_key() {
        let store = DiagnosticStore::new();
        store.publish(&json!({"uri": "file:///p/a%7Eb.py", "version": 2, "diagnostics": []}));
        let cancel = CancelToken::new();
        let wait = quick(&cancel);
        let found = store.wait("file:///p/a~b.py", 2, &wait, || false);
        assert_eq!(found.map(|items| items.len()), Some(0));
        store.forget("file:///p/a~b.py");
        assert!(store.wait("file:///p/a~b.py", 2, &wait, || false).is_none());
    }

    #[test]
    fn uris_of_other_schemes_stay_as_they_are() {
        assert_eq!(key("untitled:Untitled-1"), "untitled:Untitled-1");
        assert_eq!(key("not a uri"), "not a uri");
    }

    #[test]
    fn items_that_do_not_parse_are_skipped_and_the_version_is_kept() {
        let store = DiagnosticStore::new();
        let good = json!({
            "range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 1}},
            "message": "za\u{17c}\u{f3}\u{142}\u{107} \u{1f600}"
        });
        store.publish(&json!({"uri": "file:///a.py", "version": 3, "diagnostics": [good, {"x": 1}]}));
        store.publish(&json!({"uri": "file:///a.py", "diagnostics": []}));
        let cancel = CancelToken::new();
        let wait = quick(&cancel);
        let found = store.wait("file:///a.py", 3, &wait, || false);
        assert_eq!(found.map(|items| items.len()), Some(0));
        store.publish(&json!({"diagnostics": []}));
    }

    #[test]
    fn publication_with_a_good_item_keeps_the_text_of_the_message() {
        let store = DiagnosticStore::new();
        let good = json!({
            "range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 1}},
            "message": "za\u{17c}\u{f3}\u{142}\u{107}"
        });
        store.publish(&json!({"uri": "file:///a.py", "version": 1, "diagnostics": [good, 7]}));
        let cancel = CancelToken::new();
        let items = store
            .wait("file:///a.py", 1, &quick(&cancel), || false)
            .expect("publication");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].message, "za\u{17c}\u{f3}\u{142}\u{107}");
    }

    #[test]
    fn closed_store_and_cancelled_wait_return_nothing() {
        let store = DiagnosticStore::new();
        let cancel = CancelToken::new();
        store.close();
        assert!(store.wait("file:///a.py", 1, &quick(&cancel), || false).is_none());
        let open = DiagnosticStore::new();
        cancel.cancel(nurt_contracts::stage::CancelReason::Requested);
        assert!(open.wait("file:///a.py", 1, &quick(&cancel), || false).is_none());
    }

    #[test]
    fn older_publication_is_returned_after_the_quiet_period_unless_the_server_is_busy() {
        let store = DiagnosticStore::new();
        store.publish(&json!({"uri": "file:///a.py", "version": 1, "diagnostics": []}));
        let cancel = CancelToken::new();
        let found = store.wait("file:///a.py", 2, &quick(&cancel), || false);
        assert_eq!(found.map(|items| items.len()), Some(0));
        assert!(
            store
                .wait("file:///a.py", 2, &quick(&cancel), || true)
                .is_none()
        );
    }
}
