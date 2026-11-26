// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Correction candidates that a caller can accept.
//!
//! When a program is ambiguous (`E_AMBIGUOUS`) or cannot be corrected safely
//! (`E_UNCORRECTABLE`), the error lists ranked candidate programs. The store
//! gives each candidate a short identifier (`c1`, `c2`, …) and keeps the list
//! for `correction.candidates_ttl` within the session of the call. A request
//! with `accept` runs the candidate program. An unknown or expired identifier
//! is the error `E_EXPIRED_CANDIDATE`. With a file, the lists are also kept
//! on disk, so that the next process of the command line finds them.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use nurt_contracts::config::DurationSetting;
use nurt_contracts::env::WallTime;
use nurt_contracts::error::{Candidate, Error, ErrorCode};
use serde::{Deserialize, Serialize};

use crate::session::SessionId;
use crate::state::{read_optional, write_atomic};

/// Prefix of candidate identifiers.
pub const CANDIDATE_PREFIX: &str = "c";

/// Key of the calls made without a session.
const ANONYMOUS: &str = "";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Offer {
    expires: WallTime,
    candidates: Vec<Candidate>,
}

type Offers = HashMap<String, Offer>;

/// Candidates offered to the sessions of one engine.
///
/// # Examples
///
/// ```
/// use std::time::Duration;
/// use nurt_contracts::env::WallTime;
/// use nurt_contracts::error::{Candidate, Error, ErrorCode};
/// use nurt_core::candidates::CandidateStore;
///
/// let store = CandidateStore::new(Duration::from_secs(600), None);
/// let mut error = Error::new(ErrorCode::Ambiguous, "`fnd` is ambiguous").with_candidate(Candidate {
///     rank: 1,
///     reference: None,
///     program: "find \"/srv\"".to_owned(),
///     why: None,
/// });
/// let now = WallTime::from_unix_nanos(0);
/// store.register(None, &mut error, now).unwrap();
/// assert_eq!(error.candidates[0].reference.as_deref(), Some("c1"));
/// assert_eq!(store.accept(None, "c1", now).unwrap().program, "find \"/srv\"");
/// ```
#[derive(Debug)]
pub struct CandidateStore {
    ttl: Duration,
    file: Option<PathBuf>,
    offers: Mutex<Offers>,
}

impl CandidateStore {
    /// Store keeping candidates for `ttl`, also in `file` when given.
    #[must_use]
    pub fn new(ttl: Duration, file: Option<PathBuf>) -> Self {
        Self {
            ttl,
            file,
            offers: Mutex::new(HashMap::new()),
        }
    }

    /// Whether `error` offers candidates that can be accepted.
    #[must_use]
    pub fn offers_candidates(error: &Error) -> bool {
        matches!(error.code, ErrorCode::Ambiguous | ErrorCode::Uncorrectable)
            && !error.candidates.is_empty()
    }

    /// Numbers the candidates of `error` by rank and keeps them for the
    /// session; an earlier offer of the session is replaced.
    ///
    /// Errors without candidates are left unchanged.
    ///
    /// # Errors
    ///
    /// Returns `E_IO` when the candidates cannot be written to the file; the
    /// candidates are then available only to this process.
    pub fn register(
        &self,
        session: Option<&SessionId>,
        error: &mut Error,
        now: WallTime,
    ) -> Result<(), Error> {
        if !Self::offers_candidates(error) {
            return Ok(());
        }
        error.candidates.sort_by_key(|candidate| candidate.rank);
        for (index, candidate) in error.candidates.iter_mut().enumerate() {
            candidate.reference = Some(format!("{CANDIDATE_PREFIX}{}", index + 1));
        }
        let offer = Offer {
            expires: now.saturating_add(self.ttl),
            candidates: error.candidates.clone(),
        };
        let mut offers = self.lock();
        self.reload(&mut offers);
        offers.retain(|_, offer| offer.expires > now);
        offers.insert(key(session), offer);
        self.persist(&offers)
    }

    /// The candidate `reference` offered to the session.
    ///
    /// # Errors
    ///
    /// Returns `E_EXPIRED_CANDIDATE` when the session received no such
    /// candidate or its offer has expired.
    pub fn accept(
        &self,
        session: Option<&SessionId>,
        reference: &str,
        now: WallTime,
    ) -> Result<Candidate, Error> {
        let mut offers = self.lock();
        let key = key(session);
        if !offers.contains_key(&key) {
            self.reload(&mut offers);
        }
        offers
            .get(&key)
            .filter(|offer| offer.expires > now)
            .and_then(|offer| {
                offer
                    .candidates
                    .iter()
                    .find(|candidate| candidate.reference.as_deref() == Some(reference))
                    .cloned()
            })
            .ok_or_else(|| expired(reference, self.ttl))
    }

    fn lock(&self) -> MutexGuard<'_, Offers> {
        self.offers.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Adds offers from the file that this process does not hold; a missing
    /// or unreadable file adds nothing, because candidates can always be
    /// obtained again by running the program.
    fn reload(&self, offers: &mut Offers) {
        let Some(file) = &self.file else {
            return;
        };
        let Ok(Some(bytes)) = read_optional(file) else {
            return;
        };
        if let Ok(stored) = serde_json::from_slice::<Offers>(&bytes) {
            for (session, offer) in stored {
                offers.entry(session).or_insert(offer);
            }
        }
    }

    fn persist(&self, offers: &Offers) -> Result<(), Error> {
        let Some(file) = &self.file else {
            return Ok(());
        };
        let bytes = serde_json::to_vec(offers)
            .map_err(|error| Error::new(ErrorCode::Internal, error.to_string()))?;
        write_atomic(file, &bytes).map_err(|error| {
            Error::new(
                ErrorCode::Io,
                format!("cannot keep candidates in `{}`: {error}", file.display()),
            )
        })
    }
}

fn key(session: Option<&SessionId>) -> String {
    session.map_or_else(|| ANONYMOUS.to_owned(), |id| id.as_str().to_owned())
}

fn expired(reference: &str, ttl: Duration) -> Error {
    let ttl = DurationSetting::from_millis(u64::try_from(ttl.as_millis()).unwrap_or(u64::MAX));
    Error::new(
        ErrorCode::ExpiredCandidate,
        format!("candidate `{reference}` is unknown or expired"),
    )
    .with_problem(format!(
        "candidates are kept for {ttl} in the session that received them; run the program again to receive new candidates"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TTL: Duration = Duration::from_secs(600);

    fn candidate(rank: u32, program: &str) -> Candidate {
        Candidate {
            rank,
            reference: None,
            program: program.to_owned(),
            why: None,
        }
    }

    fn ambiguous() -> Error {
        Error::new(ErrorCode::Ambiguous, "`fnd` matches several commands")
            .with_candidate(candidate(2, "fold \"/srv\""))
            .with_candidate(candidate(1, "find \"/srv\""))
    }

    fn at(secs: u64) -> WallTime {
        WallTime::from_unix_nanos(0).saturating_add(Duration::from_secs(secs))
    }

    #[test]
    fn candidates_are_numbered_by_rank() {
        let store = CandidateStore::new(TTL, None);
        let mut error = ambiguous();
        store.register(None, &mut error, at(0)).unwrap();
        assert_eq!(error.candidates[0].program, "find \"/srv\"");
        assert_eq!(error.candidates[0].reference.as_deref(), Some("c1"));
        assert_eq!(error.candidates[1].reference.as_deref(), Some("c2"));
        assert_eq!(
            store.accept(None, "c2", at(1)).unwrap().program,
            "fold \"/srv\""
        );
    }

    #[test]
    fn expired_candidate_is_reported() {
        let store = CandidateStore::new(TTL, None);
        let mut error = ambiguous();
        store.register(None, &mut error, at(0)).unwrap();
        assert!(store.accept(None, "c1", at(599)).is_ok());
        let expired = store.accept(None, "c1", at(600)).unwrap_err();
        assert_eq!(expired.code, ErrorCode::ExpiredCandidate);
        assert!(
            expired
                .problem
                .as_deref()
                .unwrap_or_default()
                .contains("10m")
        );
    }

    #[test]
    fn candidates_belong_to_their_session() {
        let store = CandidateStore::new(TTL, None);
        let first = SessionId::new("s-1").unwrap();
        let second = SessionId::new("s-2").unwrap();
        let mut error = ambiguous();
        store.register(Some(&first), &mut error, at(0)).unwrap();
        assert!(store.accept(Some(&first), "c1", at(1)).is_ok());
        let foreign = store.accept(Some(&second), "c1", at(1)).unwrap_err();
        assert_eq!(foreign.code, ErrorCode::ExpiredCandidate);
        assert!(store.accept(None, "c1", at(1)).is_err());
    }

    #[test]
    fn unknown_reference_is_expired_candidate() {
        let store = CandidateStore::new(TTL, None);
        let mut error = ambiguous();
        store.register(None, &mut error, at(0)).unwrap();
        assert_eq!(
            store.accept(None, "c9", at(1)).unwrap_err().code,
            ErrorCode::ExpiredCandidate
        );
    }

    #[test]
    fn a_new_offer_of_a_session_replaces_the_earlier_one() {
        let store = CandidateStore::new(TTL, None);
        let mut first = ambiguous();
        store.register(None, &mut first, at(0)).unwrap();
        let mut second = Error::new(ErrorCode::Uncorrectable, "no safe correction")
            .with_candidate(candidate(1, "list \"/srv\""));
        store.register(None, &mut second, at(1)).unwrap();
        assert_eq!(
            store.accept(None, "c1", at(2)).unwrap().program,
            "list \"/srv\""
        );
        assert!(store.accept(None, "c2", at(2)).is_err());
    }

    #[test]
    fn an_offer_ends_with_its_lifetime_in_every_session() {
        let store = CandidateStore::new(TTL, None);
        let named = SessionId::new("s-ż").unwrap();
        let mut error = ambiguous();
        store.register(Some(&named), &mut error, at(0)).unwrap();
        assert!(store.accept(Some(&named), "c1", at(599)).is_ok());
        assert_eq!(
            store.accept(Some(&named), "c1", at(600)).unwrap_err().code,
            ErrorCode::ExpiredCandidate
        );
    }

    #[test]
    fn errors_without_candidates_are_unchanged() {
        let store = CandidateStore::new(TTL, None);
        let mut error = Error::new(ErrorCode::Syntax, "unclosed list");
        store.register(None, &mut error, at(0)).unwrap();
        assert!(error.candidates.is_empty());
        assert!(!CandidateStore::offers_candidates(&error));
    }

    #[test]
    fn candidates_survive_a_new_process() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("candidates.json");
        let first = CandidateStore::new(TTL, Some(file.clone()));
        let mut error = ambiguous();
        first.register(None, &mut error, at(0)).unwrap();
        let second = CandidateStore::new(TTL, Some(file));
        assert_eq!(
            second.accept(None, "c1", at(5)).unwrap().program,
            "find \"/srv\""
        );
    }

    #[test]
    fn unwritable_file_is_reported() {
        let temp = tempfile::tempdir().unwrap();
        let blocker = temp.path().join("file");
        std::fs::write(&blocker, b"x").unwrap();
        let store = CandidateStore::new(TTL, Some(blocker.join("candidates.json")));
        let mut error = ambiguous();
        let failure = store.register(None, &mut error, at(0)).unwrap_err();
        assert_eq!(failure.code, ErrorCode::Io);
        assert!(store.accept(None, "c1", at(1)).is_ok());
    }
}
