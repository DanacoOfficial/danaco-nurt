// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Sessions of callers.
//!
//! A session groups the calls of one caller: an interactive shell, a
//! protocol connection, a tool-calling integration or a scheduled task
//! (`schedule:nightly`). The identifier is recorded in the journal, scopes
//! the correction candidates of the caller and carries settings made for the
//! session, which expire after `session.ttl` without calls.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use nurt_contracts::env::WallTime;
use nurt_contracts::error::{Error, ErrorCode};
use serde_json::{Map, Value as Json};

/// Largest number of characters of a session identifier.
pub const MAX_SESSION_ID_LEN: usize = 128;

/// Identifier of a session: 1 to [`MAX_SESSION_ID_LEN`] characters without
/// control characters.
///
/// # Examples
///
/// ```
/// use nurt_core::session::SessionId;
///
/// assert_eq!(SessionId::new("schedule:nightly").unwrap().as_str(), "schedule:nightly");
/// assert!(SessionId::new("").is_err());
/// assert!(SessionId::new("a\nb").is_err());
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SessionId(String);

impl SessionId {
    /// Checks and wraps an identifier.
    ///
    /// # Errors
    ///
    /// Returns `E_ARG` when the identifier is empty, too long or contains a
    /// control character.
    pub fn new(id: impl Into<String>) -> Result<Self, Error> {
        let id = id.into();
        let length = id.chars().count();
        if length == 0 || length > MAX_SESSION_ID_LEN || id.chars().any(char::is_control) {
            return Err(Error::new(
                ErrorCode::Arg,
                format!(
                    "session identifier must have 1 to {MAX_SESSION_ID_LEN} characters without control characters; given `{}`",
                    id.escape_default()
                ),
            ));
        }
        Ok(Self(id))
    }

    /// The identifier.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// State of one session.
#[derive(Debug, Clone, PartialEq)]
pub struct Session {
    id: SessionId,
    started: WallTime,
    last_used: WallTime,
    settings: Map<String, Json>,
}

impl Session {
    /// Identifier of the session.
    #[must_use]
    pub fn id(&self) -> &SessionId {
        &self.id
    }

    /// When the session started or last restarted after expiring.
    #[must_use]
    pub fn started(&self) -> WallTime {
        self.started
    }

    /// Time of the latest call of the session.
    #[must_use]
    pub fn last_used(&self) -> WallTime {
        self.last_used
    }

    /// Settings made for the session, as configuration tables.
    #[must_use]
    pub fn settings(&self) -> &Map<String, Json> {
        &self.settings
    }
}

/// Sessions of one engine.
///
/// # Examples
///
/// ```
/// use std::time::Duration;
/// use nurt_contracts::env::WallTime;
/// use nurt_core::session::{SessionId, Sessions};
///
/// let sessions = Sessions::new(Duration::from_secs(60));
/// let id = SessionId::new("s-1").unwrap();
/// let start = WallTime::from_unix_nanos(0);
/// sessions.touch(&id, start);
/// assert!(sessions.get(&id, start.saturating_add(Duration::from_secs(30))).is_some());
/// assert!(sessions.get(&id, start.saturating_add(Duration::from_secs(61))).is_none());
/// ```
#[derive(Debug)]
pub struct Sessions {
    ttl: Duration,
    entries: Mutex<HashMap<SessionId, Session>>,
}

impl Sessions {
    /// Sessions that expire after `ttl` without calls.
    #[must_use]
    pub fn new(ttl: Duration) -> Self {
        Self {
            ttl,
            entries: Mutex::new(HashMap::new()),
        }
    }

    /// Time without calls after which a session expires.
    #[must_use]
    pub fn ttl(&self) -> Duration {
        self.ttl
    }

    /// Records a call of the session at `now`; an absent or expired session
    /// starts again without settings.
    pub fn touch(&self, id: &SessionId, now: WallTime) -> Session {
        let mut entries = self.lock();
        let session = entries
            .entry(id.clone())
            .and_modify(|session| {
                if expired(session, now, self.ttl) {
                    *session = fresh(id, now);
                }
            })
            .or_insert_with(|| fresh(id, now));
        session.last_used = now;
        session.clone()
    }

    /// The session, when it exists and has not expired at `now`.
    #[must_use]
    pub fn get(&self, id: &SessionId, now: WallTime) -> Option<Session> {
        self.lock()
            .get(id)
            .filter(|session| !expired(session, now, self.ttl))
            .cloned()
    }

    /// Replaces the settings of the session and records a call at `now`.
    pub fn set_settings(
        &self,
        id: &SessionId,
        settings: Map<String, Json>,
        now: WallTime,
    ) -> Session {
        self.touch(id, now);
        let mut entries = self.lock();
        let session = entries.entry(id.clone()).or_insert_with(|| fresh(id, now));
        session.settings = settings;
        session.clone()
    }

    /// Removes sessions expired at `now` and returns their number.
    pub fn sweep(&self, now: WallTime) -> usize {
        let mut entries = self.lock();
        let before = entries.len();
        entries.retain(|_, session| !expired(session, now, self.ttl));
        before - entries.len()
    }

    /// Number of sessions that have not expired at `now`.
    #[must_use]
    pub fn active(&self, now: WallTime) -> usize {
        self.lock()
            .values()
            .filter(|session| !expired(session, now, self.ttl))
            .count()
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<SessionId, Session>> {
        self.entries.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

fn fresh(id: &SessionId, now: WallTime) -> Session {
    Session {
        id: id.clone(),
        started: now,
        last_used: now,
        settings: Map::new(),
    }
}

fn expired(session: &Session, now: WallTime, ttl: Duration) -> bool {
    now.saturating_since(session.last_used) >= ttl
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    const TTL: Duration = Duration::from_secs(24 * 3600);

    fn at(secs: u64) -> WallTime {
        WallTime::from_unix_nanos(0).saturating_add(Duration::from_secs(secs))
    }

    #[test]
    fn identifier_length_is_limited() {
        assert!(SessionId::new("x".repeat(MAX_SESSION_ID_LEN)).is_ok());
        let error = SessionId::new("x".repeat(MAX_SESSION_ID_LEN + 1)).unwrap_err();
        assert_eq!(error.code, ErrorCode::Arg);
        assert_eq!(SessionId::new("s").unwrap().to_string(), "s");
    }

    #[test]
    fn calls_keep_a_session_alive() {
        let sessions = Sessions::new(TTL);
        let id = SessionId::new("s-1").unwrap();
        let first = sessions.touch(&id, at(0));
        sessions.touch(&id, at(20 * 3600));
        let later = sessions.touch(&id, at(40 * 3600));
        assert_eq!(later.started(), first.started());
        assert_eq!(later.last_used(), at(40 * 3600));
        assert_eq!(later.id(), &id);
        assert_eq!(sessions.ttl(), TTL);
    }

    #[test]
    fn expired_session_restarts_without_settings() {
        let sessions = Sessions::new(TTL);
        let id = SessionId::new("s-1").unwrap();
        let mut settings = Map::new();
        settings.insert("journal".to_owned(), json!({"max_ops": 50}));
        sessions.set_settings(&id, settings, at(0));
        assert_eq!(sessions.get(&id, at(3600)).unwrap().settings().len(), 1);
        let restarted = sessions.touch(&id, at(25 * 3600));
        assert!(restarted.settings().is_empty());
        assert_eq!(restarted.started(), at(25 * 3600));
    }

    #[test]
    fn sweep_removes_expired_sessions() {
        let sessions = Sessions::new(TTL);
        sessions.touch(&SessionId::new("old").unwrap(), at(0));
        sessions.touch(&SessionId::new("new").unwrap(), at(23 * 3600));
        assert_eq!(sessions.active(at(24 * 3600)), 1);
        assert_eq!(sessions.sweep(at(24 * 3600)), 1);
        assert_eq!(sessions.active(at(24 * 3600)), 1);
    }
}
