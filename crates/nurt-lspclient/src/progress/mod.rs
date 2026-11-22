// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Work in progress reported by a language server.
//!
//! Servers report project loading and indexing through `$/progress`
//! (`begin`, `report`, `end`) and, for rust-analyzer, through
//! `experimental/serverStatus` with `quiescent: false` while work is pending.
//! The tracker counts started work so that a caller can tell whether a
//! response was produced while the server was loading.
//!
//! # Examples
//!
//! ```
//! use nurt_lspclient::progress::ProgressTracker;
//! use serde_json::json;
//!
//! let tracker = ProgressTracker::new();
//! tracker.on_progress(&json!({"token": "t", "value": {"kind": "begin", "title": "indexing"}}));
//! assert!(tracker.busy());
//! tracker.on_progress(&json!({"token": "t", "value": {"kind": "end"}}));
//! assert!(!tracker.busy());
//! assert_eq!(tracker.loads(), 1);
//! ```

use std::collections::HashSet;
use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use nurt_contracts::stage::CancelToken;
use serde_json::Value;

/// Longest single wait between checks of the cancellation token.
const CANCEL_POLL: Duration = Duration::from_millis(25);

#[derive(Debug, Default)]
struct State {
    tokens: HashSet<String>,
    quiescent: bool,
    loads: u64,
    closed: bool,
    last_note: Option<String>,
}

/// Tracker of the work in progress of one server.
#[derive(Debug)]
pub struct ProgressTracker {
    state: Mutex<State>,
    changed: Condvar,
}

impl Default for ProgressTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl ProgressTracker {
    /// Tracker of an idle server.
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: Mutex::new(State {
                quiescent: true,
                ..State::default()
            }),
            changed: Condvar::new(),
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Applies the parameters of a `$/progress` notification.
    pub fn on_progress(&self, params: &Value) {
        let token = match params.get("token") {
            Some(Value::String(text)) => text.clone(),
            Some(other) => other.to_string(),
            None => String::new(),
        };
        let value = params.get("value");
        let kind = value
            .and_then(|value| value.get("kind"))
            .and_then(Value::as_str);
        let mut state = self.lock();
        match kind {
            Some("begin") => {
                state.tokens.insert(token);
                state.loads += 1;
            }
            Some("end") => {
                state.tokens.remove(&token);
            }
            _ => {}
        }
        if let Some(note) = value.and_then(progress_note) {
            state.last_note = Some(note);
        }
        drop(state);
        self.changed.notify_all();
    }

    /// Applies the parameters of an `experimental/serverStatus` notification.
    pub fn on_server_status(&self, params: &Value) {
        let quiescent = params.get("quiescent").and_then(Value::as_bool) != Some(false);
        let mut state = self.lock();
        if !quiescent {
            state.loads += 1;
        }
        state.quiescent = quiescent;
        drop(state);
        self.changed.notify_all();
    }

    /// Marks the server as ended; waits end at once.
    pub fn close(&self) {
        let mut state = self.lock();
        state.closed = true;
        state.tokens.clear();
        state.quiescent = true;
        drop(state);
        self.changed.notify_all();
    }

    /// Returns `true` while the server reports work in progress.
    #[must_use]
    pub fn busy(&self) -> bool {
        let state = self.lock();
        !state.closed && (!state.tokens.is_empty() || !state.quiescent)
    }

    /// Number of work items started so far; a change between a request and
    /// its response means the response was produced while loading.
    #[must_use]
    pub fn loads(&self) -> u64 {
        self.lock().loads
    }

    /// Title, message and percentage of the latest progress report.
    #[must_use]
    pub fn last_note(&self) -> Option<String> {
        self.lock().last_note.clone()
    }

    /// Waits until the server has no work in progress. Returns `true` when
    /// the work ended, `false` when `timeout` elapsed or `cancel` was
    /// cancelled first.
    #[must_use]
    pub fn wait_settled(&self, timeout: Duration, cancel: &CancelToken) -> bool {
        let deadline = Instant::now().checked_add(timeout);
        let mut state = self.lock();
        loop {
            if state.closed || (state.tokens.is_empty() && state.quiescent) {
                return true;
            }
            if cancel.is_cancelled() {
                return false;
            }
            let remaining = deadline.map_or(CANCEL_POLL, |deadline| {
                deadline.saturating_duration_since(Instant::now())
            });
            if remaining.is_zero() {
                return false;
            }
            state = self
                .changed
                .wait_timeout(state, remaining.min(CANCEL_POLL))
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }
}

fn progress_note(value: &Value) -> Option<String> {
    let mut parts = Vec::new();
    for key in ["title", "message"] {
        if let Some(text) = value.get(key).and_then(Value::as_str)
            && !text.is_empty()
        {
            parts.push(text.to_owned());
        }
    }
    if let Some(percentage) = value.get("percentage").and_then(Value::as_u64) {
        parts.push(format!("{percentage}%"));
    }
    (!parts.is_empty()).then(|| parts.join(" "))
}
