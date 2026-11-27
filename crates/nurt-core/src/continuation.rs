// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Kept remainders of shortened results and the parts that `next` reads.
//!
//! A response that leaves lines or records out registers a continuation:
//! the engine keeps the remainder in the state directory, below `results`,
//! outside the journal, and names it `c1`, `c2`, … `next` reads the parts
//! and never computes the result again. The remainder is one of three
//! kinds ([`ContinuationSource`]): lines of the spool file of a `run`, the
//! records computed by the program in the order of the response (followed,
//! for `logs`, by the cursor of the query) or the cursor of a query alone.
//!
//! Several processes and sessions share one state directory: a lock file
//! serializes the allocation of identifiers and the change of a result, a
//! continuation belongs to the session that made it, and the end of a session
//! removes only its own results.
//!
//! The limits of the section `[continuation]` bound the age, the number and
//! the space of kept results; the oldest continuation is removed first. A
//! removed continuation leaves a short note with the original call, which
//! `E_CONTINUATION_EXPIRED` offers in `next_steps`.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{self, BufRead, BufReader, Seek, SeekFrom};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use nurt_contracts::config::ContinuationConfig;
use nurt_contracts::continuation::{
    Continuation, ContinuationRequest, ContinuationSource, ContinuationUnit, CutLine,
    ResumeOverride, ResumeRunner,
};
use nurt_contracts::env::WallTime;
use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::record::{Line, LineEnding};
use nurt_contracts::value::{Path, Record};
use serde::{Deserialize, Serialize};

use crate::state::{read_optional, write_atomic};

const CONTINUATION_DIR: &str = "continuations";
const META_EXTENSION: &str = "meta";
const BODY_EXTENSION: &str = "body";
const GONE_EXTENSION: &str = "gone";
const COUNTER_FILE: &str = "counter";
const LOCK_FILE: &str = ".lock";
const ID_PREFIX: &str = "c";
const MAX_NOTES: usize = 500;
const MAX_ALL_RECORDS: usize = 1_000_000;
const FETCH_RETRIES: usize = 8;
const LOCK_WAIT: Duration = Duration::from_secs(10);
const LOCK_RETRY: Duration = Duration::from_millis(5);

/// What a caller gives when it registers a continuation.
#[derive(Debug, Clone, Default)]
pub struct Registration {
    /// Text of the call that produced the response, offered again when the
    /// continuation expires and used to run a query again from its cursor.
    pub program: Option<String>,
    /// Session of the console that made the call.
    pub session: Option<String>,
    /// Size of the first response, the default size of a part.
    pub part: u64,
    /// Options of the first call as JSON; a query that continues after a
    /// cursor runs again under them.
    pub options: Option<serde_json::Value>,
}

/// Key of the owner of the continuations of one execution: the console or
/// the session; given to `next` through the extensions of the execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionKey(pub Option<String>);

/// What `next` was asked for.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NextQuery {
    /// Identifier `cN`; the latest continuation of the session when `None`.
    pub id: Option<String>,
    /// Units of the part; the size of the first response when `None`.
    pub count: Option<u64>,
    /// Everything that is left.
    pub all: bool,
    /// The lines of the last shown part that were cut, whole.
    pub full: bool,
}

/// A part read by `next`.
#[derive(Debug, Clone, PartialEq)]
pub struct NextPart {
    /// Records of the part.
    pub records: Vec<Record>,
    /// The state after the part; `remaining` 0 and no cut lines mean the part
    /// was the last one.
    pub continuation: Option<ContinuationRequest>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Meta {
    id: String,
    seq: u64,
    session: Option<String>,
    created: WallTime,
    program: Option<String>,
    size: u64,
    #[serde(default)]
    options: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Body {
    unit: ContinuationUnit,
    shown: u64,
    remaining: i64,
    part: u64,
    source: ContinuationSource,
    offset: Option<u64>,
    #[serde(default)]
    version: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Note {
    program: Option<String>,
    reason: String,
    at: WallTime,
    #[serde(default)]
    session: Option<String>,
}

#[derive(Debug)]
enum Backend {
    Disk(PathBuf),
    Memory(Mutex<BTreeMap<String, Vec<u8>>>),
}

impl Backend {
    fn write(&self, name: &str, bytes: &[u8]) -> bool {
        match self {
            Self::Disk(dir) => write_atomic(&dir.join(name), bytes).is_ok(),
            Self::Memory(files) => {
                lock(files).insert(name.to_owned(), bytes.to_vec());
                true
            }
        }
    }

    fn read(&self, name: &str) -> Option<Vec<u8>> {
        match self {
            Self::Disk(dir) => read_optional(&dir.join(name)).ok().flatten(),
            Self::Memory(files) => lock(files).get(name).cloned(),
        }
    }

    fn remove(&self, name: &str) {
        match self {
            Self::Disk(dir) => {
                let _ = fs::remove_file(dir.join(name));
            }
            Self::Memory(files) => {
                lock(files).remove(name);
            }
        }
    }

    fn names(&self) -> Vec<String> {
        match self {
            Self::Disk(dir) => fs::read_dir(dir)
                .map(|entries| {
                    entries
                        .filter_map(Result::ok)
                        .filter_map(|entry| entry.file_name().into_string().ok())
                        .collect()
                })
                .unwrap_or_default(),
            Self::Memory(files) => lock(files).keys().cloned().collect(),
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Why a result is not kept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotKept(pub String);

/// Kept remainders of one state directory.
///
/// # Examples
///
/// ```
/// use nurt_contracts::config::ContinuationConfig;
/// use nurt_contracts::continuation::{ContinuationRequest, ContinuationSource, ContinuationUnit};
/// use nurt_contracts::env::WallTime;
/// use nurt_core::continuation::{ContinuationStore, NextQuery, Registration};
///
/// let store = ContinuationStore::new(None);
/// let config = ContinuationConfig::default();
/// let now = WallTime::from_unix_nanos(0);
/// let request = ContinuationRequest {
///     id: None,
///     source: ContinuationSource::Records { rows: Vec::new(), cut: Vec::new(), resume: None },
///     unit: ContinuationUnit::Records,
///     shown: 50,
///     remaining: 0,
///     cut_lines: 0,
/// };
/// let continuation = store
///     .register(request, &Registration::default(), &config, now)
///     .unwrap();
/// assert_eq!(continuation.id, "c1");
/// let part = store
///     .read(&NextQuery::default(), None, None, &config, now)
///     .unwrap();
/// assert!(part.records.is_empty());
/// ```
#[derive(Debug)]
pub struct ContinuationStore {
    backend: Backend,
    counter: AtomicU64,
    gate: Mutex<()>,
}

/// Holder of the lock of the store; the operating system releases the file
/// lock when the file closes.
struct Guard<'a> {
    _gate: MutexGuard<'a, ()>,
    _file: Option<fs::File>,
}

fn lock_error(what: &str, error: &io::Error) -> Error {
    Error::new(
        ErrorCode::Io,
        format!("the store of continuations cannot be locked: {what}: {error}"),
    )
}

impl ContinuationStore {
    /// Store below `dir` (the `results` area of the state directory), or in
    /// memory when there is no state directory.
    #[must_use]
    pub fn new(dir: Option<PathBuf>) -> Self {
        let backend = match dir {
            Some(dir) => {
                let dir = dir.join(CONTINUATION_DIR);
                let _ = fs::create_dir_all(&dir);
                Backend::Disk(dir)
            }
            None => Backend::Memory(Mutex::new(BTreeMap::new())),
        };
        Self {
            backend,
            counter: AtomicU64::new(0),
            gate: Mutex::new(()),
        }
    }

    /// Takes the lock of the store: a mutex within the process and, for a
    /// state directory, an exclusive lock of the operating system on the
    /// lock file, waited for up to [`LOCK_WAIT`]. The lock is never taken
    /// over: a holder that does not release it makes the call fail.
    ///
    /// # Errors
    ///
    /// Returns an error when the lock file cannot be opened or the lock is
    /// not obtained in time.
    fn locked(&self) -> Result<Guard<'_>, Error> {
        let gate = lock(&self.gate);
        let Backend::Disk(dir) = &self.backend else {
            return Ok(Guard {
                _gate: gate,
                _file: None,
            });
        };
        let path = dir.join(LOCK_FILE);
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(|error| lock_error("open", &error))?;
        let started = Instant::now();
        loop {
            match file.try_lock() {
                Ok(()) => {
                    return Ok(Guard {
                        _gate: gate,
                        _file: Some(file),
                    });
                }
                Err(fs::TryLockError::WouldBlock) => {
                    if started.elapsed() > LOCK_WAIT {
                        return Err(lock_error(
                            "wait",
                            &io::Error::new(io::ErrorKind::TimedOut, "held by another process"),
                        ));
                    }
                    std::thread::sleep(LOCK_RETRY);
                }
                Err(fs::TryLockError::Error(error)) => return Err(lock_error("lock", &error)),
            }
        }
    }

    /// Keeps the remainder described by `request` and returns the field
    /// `continuation` of the response.
    ///
    /// A request of an existing continuation (`next`) is not kept again; its
    /// description is returned as it is.
    ///
    /// # Errors
    ///
    /// Returns the reason when the result is not kept: it is larger than
    /// `continuation.max_size` or cannot be written.
    pub fn register(
        &self,
        request: ContinuationRequest,
        registration: &Registration,
        config: &ContinuationConfig,
        now: WallTime,
    ) -> Result<Continuation, NotKept> {
        let _guard = self
            .locked()
            .map_err(|error| NotKept(error.message.clone()))?;
        if let (ContinuationSource::Existing, Some(id)) = (&request.source, &request.id) {
            return Ok(Continuation {
                id: id.clone(),
                unit: request.unit,
                shown: request.shown,
                remaining: request.remaining,
                cut_lines: request.cut_lines,
                older: self.is_older(id, registration.session.as_deref()),
            });
        }
        self.sweep_inner(config, now);
        let body = Body {
            unit: request.unit,
            shown: request.shown,
            remaining: request.remaining,
            part: registration.part.max(1),
            source: request.source,
            offset: None,
            version: 0,
        };
        let bytes = serde_json::to_vec(&body).unwrap_or_default();
        if count_of(bytes.len()) > config.max_size.bytes() {
            return Err(NotKept(
                "the result is larger than continuation.max_size".to_owned(),
            ));
        }
        let seq = self.next_seq();
        let id = format!("{ID_PREFIX}{seq}");
        let meta = Meta {
            id: id.clone(),
            seq,
            session: registration.session.clone(),
            created: now,
            program: registration.program.clone(),
            size: 0,
            options: registration.options.clone(),
        };
        if !self.store(meta, &body) {
            return Err(NotKept(
                "the result cannot be written to the state directory".to_owned(),
            ));
        }
        self.sweep_inner(config, now);
        Ok(Continuation {
            id,
            unit: request.unit,
            shown: request.shown,
            remaining: request.remaining,
            cut_lines: request.cut_lines,
            older: false,
        })
    }

    /// Reads the part that `query` asks for.
    ///
    /// # Errors
    ///
    /// Returns `E_CONTINUATION_EXPIRED` when no continuation is kept, the
    /// named one expired, was removed by the limits or belongs to another
    /// session, `E_INVALID_VALUE` for `full` on a result without cut lines
    /// to give, and the error of the program when a query is run again from
    /// its cursor.
    pub fn read(
        &self,
        query: &NextQuery,
        session: Option<&str>,
        runner: Option<&dyn ResumeRunner>,
        config: &ContinuationConfig,
        now: WallTime,
    ) -> Result<NextPart, Error> {
        for _ in 0..FETCH_RETRIES {
            let guard = self.locked()?;
            self.sweep_inner(config, now);
            let (meta, body) = self.find(query.id.as_deref(), session, now)?;
            let Some((cursor, need)) = plan_fetch(&body, query) else {
                return self.apply(&meta, body, query, None);
            };
            // The query that continues after a cursor can take long: run it
            // without the lock and apply it only to the version it was
            // planned for.
            let version = body.version;
            drop(guard);
            let runner = runner.ok_or_else(|| {
                Error::new(ErrorCode::Internal, "no engine to run the query again")
            })?;
            let fetched = fetch(runner, &meta, cursor, need, body.part.max(1), query.all)?;
            let _guard = self.locked()?;
            self.sweep_inner(config, now);
            let (meta, body) = self.find(query.id.as_deref(), session, now)?;
            if body.version == version {
                return self.apply(&meta, body, query, Some(fetched));
            }
        }
        Err(Error::new(
            ErrorCode::ContinuationExpired,
            "the continuation is read by another call at the same time; call next again",
        ))
    }

    #[allow(
        clippy::cognitive_complexity,
        reason = "one match over the three kinds of remainder, each a short loop"
    )]
    fn apply(
        &self,
        meta: &Meta,
        mut body: Body,
        query: &NextQuery,
        mut fetched: Option<Fetched>,
    ) -> Result<NextPart, Error> {
        let part = query.count.unwrap_or(body.part).max(1);
        let take = if query.all { u64::MAX } else { part };
        let mut records = Vec::new();
        let mut full = query.full;
        match &mut body.source {
            ContinuationSource::Records { rows, cut, resume } => {
                full = full || (rows.is_empty() && resume.is_none() && !cut.is_empty());
                if full {
                    records = read_cut_lines(cut);
                    cut.clear();
                } else {
                    let now_rows = rows.len().min(usize::try_from(take).unwrap_or(usize::MAX));
                    records = rows.drain(..now_rows).collect();
                    cut.clear();
                    if let Some(fetched) = fetched.take() {
                        records.extend(fetched.records);
                        *resume = fetched.next;
                    }
                    body.shown = body.shown.saturating_add(count_of(records.len()));
                    body.remaining = if resume.is_some() {
                        -1
                    } else {
                        count_signed(rows.len())
                    };
                }
            }
            ContinuationSource::Lines {
                path,
                first_line,
                last_line,
                cut,
                next,
            } => {
                let left = last_line.saturating_add(1).saturating_sub(*first_line);
                full = full || (left == 0 && next.is_empty() && !cut.is_empty());
                if full {
                    let wanted: BTreeSet<u64> = cut.iter().copied().collect();
                    records = read_lines(path, &wanted).map_err(|error| error.with_call(meta))?;
                    cut.clear();
                } else {
                    let mut want = take;
                    while want > 0 {
                        let left = last_line.saturating_add(1).saturating_sub(*first_line);
                        let step = left.min(want);
                        if step > 0 {
                            let (read, offset) = read_range(path, *first_line, step, body.offset)
                                .map_err(|error| io_error(path, &error, meta))?;
                            let given = count_of(read.len());
                            *first_line = first_line.saturating_add(given);
                            if !read.is_empty() {
                                body.offset = Some(offset);
                            }
                            if given < step {
                                *last_line = first_line.saturating_sub(1);
                            }
                            want -= given;
                            records.extend(read);
                        }
                        let left = last_line.saturating_add(1).saturating_sub(*first_line);
                        if left == 0 && !next.is_empty() {
                            let range = next.remove(0);
                            *path = range.path;
                            *first_line = range.first_line;
                            *last_line = range.last_line;
                            *cut = range.cut;
                            body.offset = None;
                        } else if step == 0 || left > 0 {
                            break;
                        }
                    }
                    let given = count_of(records.len());
                    body.shown = body.shown.saturating_add(given);
                    let in_next: u64 = next
                        .iter()
                        .map(|range| {
                            range
                                .last_line
                                .saturating_add(1)
                                .saturating_sub(range.first_line)
                        })
                        .sum();
                    body.remaining = count_signed_u64(
                        last_line
                            .saturating_add(1)
                            .saturating_sub(*first_line)
                            .saturating_add(in_next),
                    );
                    if !records.is_empty() {
                        cut.clear();
                    }
                }
            }
            ContinuationSource::Resume { cursor } => {
                if full {
                    return Err(Error::new(
                        ErrorCode::InvalidValue,
                        "this continuation has no cut lines to read whole",
                    ));
                }
                let Some(fetched) = fetched.take() else {
                    return Err(Error::new(
                        ErrorCode::Internal,
                        "the query of the continuation was not run",
                    ));
                };
                body.shown = body.shown.saturating_add(count_of(fetched.records.len()));
                records = fetched.records;
                match fetched.next {
                    Some(next) => {
                        *cursor = next;
                        body.remaining = -1;
                    }
                    None => body.remaining = 0,
                }
            }
            ContinuationSource::Existing => {}
        }
        body.version += 1;
        let continuation = self.after(meta, &body);
        Ok(NextPart {
            records,
            continuation,
        })
    }

    /// Removes the continuations of `session`, as at the end of a session of
    /// the console; the results of other sessions stay.
    pub fn end_session(&self, session: Option<&str>) {
        let Ok(_guard) = self.locked() else {
            return;
        };
        for meta in self.metas() {
            if meta.session.as_deref() == session {
                self.backend
                    .remove(&format!("{}.{BODY_EXTENSION}", meta.id));
                self.backend
                    .remove(&format!("{}.{META_EXTENSION}", meta.id));
                self.backend
                    .remove(&format!("{}.{GONE_EXTENSION}", meta.id));
            }
        }
        for name in self.backend.names() {
            if let Some(stem) = name.strip_suffix(&format!(".{GONE_EXTENSION}"))
                && self
                    .backend
                    .read(&name)
                    .and_then(|bytes| serde_json::from_slice::<Note>(&bytes).ok())
                    .is_some_and(|note| note.session.as_deref() == session)
            {
                let _ = stem;
                self.backend.remove(&name);
            }
        }
    }

    /// Removes the continuations that the limits of `config` do not allow at
    /// `now` and returns their number.
    pub fn sweep(&self, config: &ContinuationConfig, now: WallTime) -> usize {
        let Ok(_guard) = self.locked() else {
            return 0;
        };
        self.sweep_inner(config, now)
    }

    fn sweep_inner(&self, config: &ContinuationConfig, now: WallTime) -> usize {
        let mut metas = self.metas();
        metas.sort_by_key(|meta| meta.seq);
        let max_age = config.max_age.as_std();
        let mut removed = 0;
        let mut kept = Vec::new();
        for meta in metas {
            if now.saturating_since(meta.created) > max_age {
                self.remove(&meta, "it is older than continuation.max_age", now);
                removed += 1;
            } else {
                kept.push(meta);
            }
        }
        let max_count = usize::try_from(config.max_count).unwrap_or(usize::MAX);
        while kept.len() > max_count {
            let meta = kept.remove(0);
            self.remove(&meta, "more than continuation.max_count are kept", now);
            removed += 1;
        }
        let max_size = config.max_size.bytes();
        let mut total: u64 = kept.iter().map(|meta| meta.size).sum();
        while total > max_size && !kept.is_empty() {
            let meta = kept.remove(0);
            total = total.saturating_sub(meta.size);
            self.remove(&meta, "the kept results exceed continuation.max_size", now);
            removed += 1;
        }
        removed
    }

    /// Whether a continuation newer than `id` is kept for `session`.
    fn is_older(&self, id: &str, session: Option<&str>) -> bool {
        let Some(number) = id_number(id) else {
            return false;
        };
        self.metas()
            .iter()
            .any(|meta| meta.session.as_deref() == session && meta.seq > number)
    }

    /// Number of kept continuations.
    #[must_use]
    pub fn len(&self) -> usize {
        self.metas().len()
    }

    /// Whether no continuation is kept.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn after(&self, meta: &Meta, body: &Body) -> Option<ContinuationRequest> {
        self.store(meta.clone(), body);
        let cut = match &body.source {
            ContinuationSource::Lines { cut, next, .. } => {
                count_of(cut.len() + next.iter().map(|range| range.cut.len()).sum::<usize>())
            }
            ContinuationSource::Records { cut, .. } => count_of(cut.len()),
            _ => 0,
        };
        Some(ContinuationRequest {
            id: Some(meta.id.clone()),
            source: ContinuationSource::Existing,
            unit: body.unit,
            shown: body.shown,
            remaining: body.remaining,
            cut_lines: cut,
        })
    }

    fn find(
        &self,
        id: Option<&str>,
        session: Option<&str>,
        now: WallTime,
    ) -> Result<(Meta, Body), Error> {
        let _ = now;
        let id = match id {
            Some(id) => id.to_owned(),
            None => match self
                .metas()
                .into_iter()
                .filter(|meta| meta.session.as_deref() == session)
                .max_by_key(|meta| meta.seq)
            {
                Some(latest) => latest.id,
                None => return Err(self.none_kept(session)),
            },
        };
        let meta = self
            .backend
            .read(&format!("{id}.{META_EXTENSION}"))
            .and_then(|bytes| serde_json::from_slice::<Meta>(&bytes).ok());
        let body = self
            .backend
            .read(&format!("{id}.{BODY_EXTENSION}"))
            .and_then(|bytes| serde_json::from_slice::<Body>(&bytes).ok());
        match (meta, body) {
            (Some(meta), _) if meta.session.as_deref() != session => Err(Error::new(
                ErrorCode::ContinuationExpired,
                format!("continuation `{id}` belongs to another session"),
            )
            .with_problem("`next` reads only the continuations of its own session")),
            (Some(meta), Some(body)) => Ok((meta, body)),
            _ => {
                let note = self
                    .backend
                    .read(&format!("{id}.{GONE_EXTENSION}"))
                    .and_then(|bytes| serde_json::from_slice::<Note>(&bytes).ok());
                Err(match note {
                    Some(note) if note.session.as_deref() == session => {
                        expired(&id, note.program.as_deref(), &note.reason)
                    }
                    _ => Error::new(
                        ErrorCode::ContinuationExpired,
                        format!("continuation `{id}` is unknown"),
                    )
                    .with_problem("no such continuation was kept in this session"),
                })
            }
        }
    }

    /// The error of `next` when no continuation is kept: the newest removed
    /// one of the session, with its original call, or a plain explanation.
    fn none_kept(&self, session: Option<&str>) -> Error {
        let newest = self
            .backend
            .names()
            .into_iter()
            .filter_map(|name| {
                let stem = name.strip_suffix(&format!(".{GONE_EXTENSION}"))?.to_owned();
                Some((id_number(&stem)?, stem))
            })
            .filter(|(_, id)| {
                self.backend
                    .read(&format!("{id}.{GONE_EXTENSION}"))
                    .and_then(|bytes| serde_json::from_slice::<Note>(&bytes).ok())
                    .is_some_and(|note| note.session.as_deref() == session)
            })
            .max();
        if let Some((_, id)) = newest
            && let Some(note) = self
                .backend
                .read(&format!("{id}.{GONE_EXTENSION}"))
                .and_then(|bytes| serde_json::from_slice::<Note>(&bytes).ok())
        {
            return expired(&id, note.program.as_deref(), &note.reason);
        }
        Error::new(
            ErrorCode::ContinuationExpired,
            "no continuation is kept in this session",
        )
        .with_problem(
            "`next` continues a response that ends with `more: next`; run a command first",
        )
    }

    fn store(&self, mut meta: Meta, body: &Body) -> bool {
        let bytes = serde_json::to_vec(body).unwrap_or_default();
        meta.size = count_of(bytes.len());
        let wrote = self
            .backend
            .write(&format!("{}.{BODY_EXTENSION}", meta.id), &bytes);
        let meta_bytes = serde_json::to_vec(&meta).unwrap_or_default();
        wrote
            && self
                .backend
                .write(&format!("{}.{META_EXTENSION}", meta.id), &meta_bytes)
    }

    fn remove(&self, meta: &Meta, reason: &str, now: WallTime) {
        self.backend
            .remove(&format!("{}.{BODY_EXTENSION}", meta.id));
        self.backend
            .remove(&format!("{}.{META_EXTENSION}", meta.id));
        let note = Note {
            program: meta.program.clone(),
            reason: reason.to_owned(),
            at: now,
            session: meta.session.clone(),
        };
        self.backend.write(
            &format!("{}.{GONE_EXTENSION}", meta.id),
            &serde_json::to_vec(&note).unwrap_or_default(),
        );
        self.trim_notes();
    }

    fn trim_notes(&self) {
        let mut notes: Vec<(u64, String)> = self
            .backend
            .names()
            .into_iter()
            .filter_map(|name| {
                let stem = name.strip_suffix(&format!(".{GONE_EXTENSION}"))?;
                Some((id_number(stem)?, name))
            })
            .collect();
        if notes.len() > MAX_NOTES {
            notes.sort();
            for (_, name) in notes.drain(..notes.len() - MAX_NOTES) {
                self.backend.remove(&name);
            }
        }
    }

    fn metas(&self) -> Vec<Meta> {
        self.backend
            .names()
            .into_iter()
            .filter(|name| name.ends_with(&format!(".{META_EXTENSION}")))
            .filter_map(|name| self.backend.read(&name))
            .filter_map(|bytes| serde_json::from_slice::<Meta>(&bytes).ok())
            .collect()
    }

    /// The next number; the caller holds the lock.
    fn next_seq(&self) -> u64 {
        let stored = self
            .backend
            .read(COUNTER_FILE)
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .and_then(|text| text.trim().parse::<u64>().ok())
            .unwrap_or(0);
        let named = self
            .backend
            .names()
            .iter()
            .filter_map(|name| id_number(name.split('.').next()?))
            .max()
            .unwrap_or(0);
        let seq = stored
            .max(named)
            .max(self.counter.load(Ordering::Relaxed))
            .saturating_add(1);
        self.counter.store(seq, Ordering::Relaxed);
        self.backend.write(COUNTER_FILE, seq.to_string().as_bytes());
        seq
    }
}

/// Records and the cursor that a query run again from a cursor returned.
struct Fetched {
    records: Vec<Record>,
    next: Option<String>,
}

/// The cursor to run the query again from and the units still needed, when
/// reading this part needs the query.
fn plan_fetch(body: &Body, query: &NextQuery) -> Option<(String, u64)> {
    if query.full {
        return None;
    }
    let part = query.count.unwrap_or(body.part).max(1);
    let take = if query.all { u64::MAX } else { part };
    match &body.source {
        ContinuationSource::Records {
            rows,
            resume: Some(cursor),
            ..
        } if count_of(rows.len()) < take => {
            Some((cursor.clone(), take.saturating_sub(count_of(rows.len()))))
        }
        ContinuationSource::Resume { cursor } => Some((cursor.clone(), take)),
        _ => None,
    }
}

/// Runs the query again from `cursor`, in steps of `part`, until `need`
/// units are read or the query has no more.
fn fetch(
    runner: &dyn ResumeRunner,
    meta: &Meta,
    mut cursor: String,
    need: u64,
    part: u64,
    all: bool,
) -> Result<Fetched, Error> {
    let program = meta
        .program
        .clone()
        .ok_or_else(|| expired(&meta.id, None, "the call that produced it is not kept"))?;
    let mut records: Vec<Record> = Vec::new();
    loop {
        let want = need.saturating_sub(count_of(records.len())).min(part);
        let step = runner.resume(
            &program,
            ResumeOverride {
                cursor: cursor.clone(),
                count: Some(want),
                options: meta.options.clone(),
            },
        )?;
        records.extend(step.records);
        let next = match step.next {
            Some(ContinuationRequest {
                source: ContinuationSource::Resume { cursor },
                ..
            }) => Some(cursor),
            _ => None,
        };
        let done = next.is_none()
            || !all
            || records.len() >= MAX_ALL_RECORDS
            || count_of(records.len()) >= need;
        match next {
            Some(next) if !done => cursor = next,
            other => {
                return Ok(Fetched {
                    records,
                    next: other,
                });
            }
        }
    }
}

fn id_number(id: &str) -> Option<u64> {
    id.strip_prefix(ID_PREFIX)?.parse().ok()
}

fn count_of(length: usize) -> u64 {
    u64::try_from(length).unwrap_or(u64::MAX)
}

fn count_signed(length: usize) -> i64 {
    i64::try_from(length).unwrap_or(i64::MAX)
}

fn count_signed_u64(length: u64) -> i64 {
    i64::try_from(length).unwrap_or(i64::MAX)
}

fn expired(id: &str, program: Option<&str>, reason: &str) -> Error {
    let error = Error::new(
        ErrorCode::ContinuationExpired,
        format!("continuation `{id}` has expired or was removed"),
    )
    .with_problem(format!("it was removed because {reason}"));
    match program {
        Some(program) => error.with_next_step(program),
        None => error,
    }
}

/// An unreadable output file, with the original call as the next step.
fn io_error(path: &str, error: &io::Error, meta: &Meta) -> Error {
    let error = Error::new(
        ErrorCode::ContinuationExpired,
        format!("the file `{path}` cannot be read: {error}"),
    )
    .with_problem("the file was removed after output.spool_ttl; run the call again");
    match &meta.program {
        Some(program) => error.with_next_step(program),
        None => error,
    }
}

trait WithCall {
    fn with_call(self, meta: &Meta) -> Error;
}

impl WithCall for Error {
    fn with_call(self, meta: &Meta) -> Error {
        match &meta.program {
            Some(program) if self.next_steps.is_empty() => self.with_next_step(program),
            _ => self,
        }
    }
}

fn line_record(path: &str, no: u64, raw: &[u8], start: u64) -> Record {
    let (text, ending) = match raw.strip_suffix(b"\n") {
        Some(rest) => match rest.strip_suffix(b"\r") {
            Some(rest) => (rest, LineEnding::Crlf),
            None => (rest, LineEnding::Lf),
        },
        None => (raw, LineEnding::None),
    };
    Record::from(Line {
        path: Path::new(path),
        raw_path: None,
        no,
        text: String::from_utf8_lossy(text).into_owned(),
        line_ending: ending,
        etag: String::new(),
        start: Some(start),
        end: Some(start + count_of(text.len())),
        range_index: None,
        encoding: Some("utf-8".to_owned()),
    })
}

/// `count` lines of the file from line `first` (from 1); `offset` is the
/// byte offset of that line when a previous read left it. Returns the lines
/// and the byte offset after the last line read.
fn read_range(
    path: &str,
    first: u64,
    count: u64,
    offset: Option<u64>,
) -> io::Result<(Vec<Record>, u64)> {
    let mut reader = BufReader::new(fs::File::open(path)?);
    let (mut number, mut position) = match offset {
        Some(offset) => {
            reader.seek(SeekFrom::Start(offset))?;
            (first, offset)
        }
        None => (1, 0),
    };
    let mut records = Vec::new();
    let mut buffer = Vec::new();
    while count_of(records.len()) < count {
        buffer.clear();
        let read = reader.read_until(b'\n', &mut buffer)?;
        if read == 0 {
            break;
        }
        if number >= first {
            records.push(line_record(path, number, &buffer, position));
        }
        position += count_of(read);
        number += 1;
    }
    Ok((records, position))
}

/// The lines of the file whose numbers are in `wanted`, whole.
fn read_lines(path: &str, wanted: &BTreeSet<u64>) -> Result<Vec<Record>, Error> {
    let Some(&last) = wanted.last() else {
        return Ok(Vec::new());
    };
    let file = fs::File::open(path).map_err(|error| {
        Error::new(
            ErrorCode::ContinuationExpired,
            format!("the file `{path}` cannot be read: {error}"),
        )
        .with_problem("the file was removed after output.spool_ttl; run the call again")
    })?;
    let mut reader = BufReader::new(file);
    let mut records = Vec::new();
    let mut buffer = Vec::new();
    let mut position = 0;
    let mut number = 1;
    while number <= last {
        buffer.clear();
        let read = reader
            .read_until(b'\n', &mut buffer)
            .map_err(|error| Error::new(ErrorCode::Io, format!("{path}: {error}")))?;
        if read == 0 {
            break;
        }
        if wanted.contains(&number) {
            records.push(line_record(path, number, &buffer, position));
        }
        position += count_of(read);
        number += 1;
    }
    Ok(records)
}

/// The cut lines of results of reading commands, whole, as they were when
/// the response was made.
fn read_cut_lines(cut: &[CutLine]) -> Vec<Record> {
    cut.iter()
        .map(|line| {
            let mut raw = line.text.clone().into_bytes();
            raw.push(b'\n');
            line_record(&line.path, line.no, &raw, 0)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use nurt_contracts::config::{DurationSetting, SizeSetting};
    use nurt_contracts::continuation::ResumedPart;
    use nurt_contracts::error::Result as StageResult;
    use nurt_contracts::value::{RecordKind, Value};

    use super::*;

    fn row(number: i64) -> Record {
        Record::new(RecordKind::Row).with("n", Value::Int(number))
    }

    fn number(record: &Record) -> i64 {
        match record.get("n") {
            Some(Value::Int(number)) => *number,
            other => panic!("not a row: {other:?}"),
        }
    }

    fn at(seconds: i128) -> WallTime {
        WallTime::from_unix_nanos(seconds * 1_000_000_000)
    }

    fn records_request(from: i64, to: i64, shown: u64) -> ContinuationRequest {
        ContinuationRequest {
            id: None,
            source: ContinuationSource::Records {
                rows: (from..=to).map(row).collect(),
                cut: Vec::new(),
                resume: None,
            },
            unit: ContinuationUnit::Records,
            shown,
            remaining: to - from + 1,
            cut_lines: 0,
        }
    }

    fn registration(program: &str, part: u64) -> Registration {
        Registration {
            program: Some(program.to_owned()),
            session: None,
            part,
            options: None,
        }
    }

    fn read(store: &ContinuationStore, text: &str) -> Result<NextPart, Error> {
        let query = NextQuery {
            id: (!text.is_empty()).then(|| text.to_owned()),
            ..NextQuery::default()
        };
        store.read(&query, None, None, &ContinuationConfig::default(), at(0))
    }

    #[test]
    fn parts_of_the_kept_records_concatenate_to_the_rest() {
        let store = ContinuationStore::new(None);
        let config = ContinuationConfig::default();
        let first = store.register(
            records_request(51, 130, 50),
            &registration("list /d", 50),
            &config,
            at(0),
        );
        let first = first.unwrap();
        assert_eq!(first.id, "c1");
        assert_eq!((first.shown, first.remaining), (50, 80));
        let mut numbers = Vec::new();
        let mut parts = 0;
        loop {
            let part = read(&store, "").unwrap();
            parts += 1;
            numbers.extend(part.records.iter().map(number));
            let request = part.continuation.unwrap();
            assert_eq!(request.id.as_deref(), Some("c1"));
            if request.remaining == 0 {
                assert_eq!(request.shown, 130);
                break;
            }
        }
        assert_eq!(parts, 2);
        assert_eq!(numbers, (51..=130).collect::<Vec<_>>());
    }

    #[test]
    fn count_all_and_the_default_size_decide_the_part() {
        let store = ContinuationStore::new(None);
        let config = ContinuationConfig::default();
        let _ = store.register(
            records_request(11, 400, 10),
            &registration("p", 10),
            &config,
            at(0),
        );
        let query = |count, all| NextQuery {
            count,
            all,
            ..NextQuery::default()
        };
        let read = |query: NextQuery| {
            store
                .read(&query, None, None, &config, at(0))
                .unwrap()
                .records
                .len()
        };
        assert_eq!(read(query(None, false)), 10);
        assert_eq!(read(query(Some(200), false)), 200);
        assert_eq!(read(query(None, true)), 180);
        assert_eq!(read(query(None, false)), 0);
    }

    #[test]
    fn two_continuations_are_read_alternately_and_the_latest_is_the_default() {
        let store = ContinuationStore::new(None);
        let config = ContinuationConfig::default();
        let _ = store.register(
            records_request(11, 100, 10),
            &registration("one", 10),
            &config,
            at(0),
        );
        let _ = store.register(
            records_request(1011, 1100, 10),
            &registration("two", 10),
            &config,
            at(0),
        );
        let numbers = |text: &str| -> Vec<i64> {
            read(&store, text)
                .unwrap()
                .records
                .iter()
                .map(number)
                .collect()
        };
        assert_eq!(numbers("c1")[0], 11);
        assert_eq!(numbers("c2")[0], 1011);
        assert_eq!(numbers("c1")[0], 21);
        assert_eq!(numbers("")[0], 1021);
        let older = read(&store, "c1").unwrap().continuation.unwrap();
        assert_eq!(older.id.as_deref(), Some("c1"));
        let register = |request: ContinuationRequest| {
            store.register(request, &registration("x", 1), &config, at(0))
        };
        assert!(register(older.clone()).unwrap().older);
    }

    #[test]
    fn a_continuation_expires_by_age_with_the_original_call() {
        let store = ContinuationStore::new(None);
        let mut config = ContinuationConfig::default();
        config.max_age = DurationSetting::from_secs(60);
        let _ = store.register(
            records_request(2, 9, 1),
            &registration("find /srv", 1),
            &config,
            at(0),
        );
        assert!(
            store
                .read(&NextQuery::default(), None, None, &config, at(60))
                .is_ok()
        );
        let error = store
            .read(&NextQuery::default(), None, None, &config, at(61))
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::ContinuationExpired);
        assert_eq!(error.next_steps, ["find /srv"]);
        assert!(store.is_empty());
    }

    #[test]
    fn the_oldest_continuation_is_removed_by_count_and_by_size() {
        let store = ContinuationStore::new(None);
        let mut config = ContinuationConfig::default();
        config.max_count = 2;
        for number in 0..3 {
            let _ = store.register(
                records_request(1, 5, 1),
                &registration(&format!("call {number}"), 1),
                &config,
                at(0),
            );
        }
        assert_eq!(store.len(), 2);
        let error = read(&store, "c1").unwrap_err();
        assert_eq!(error.code(), ErrorCode::ContinuationExpired);
        assert_eq!(error.next_steps, ["call 0"]);
        assert!(read(&store, "c2").is_ok());
        let mut small = ContinuationConfig::default();
        small.max_size = SizeSetting::from_bytes(1);
        let refused = store.register(
            records_request(1, 5, 1),
            &registration("big", 1),
            &small,
            at(0),
        );
        assert!(refused.is_err());
        assert!(store.is_empty());
        let error = read(&store, "c4").unwrap_err();
        assert_eq!(error.code(), ErrorCode::ContinuationExpired);
    }

    #[test]
    fn an_unknown_continuation_and_an_empty_store_are_expired() {
        let store = ContinuationStore::new(None);
        let error = read(&store, "").unwrap_err();
        assert_eq!(error.code(), ErrorCode::ContinuationExpired);
        let error = read(&store, "c7").unwrap_err();
        assert_eq!(error.code(), ErrorCode::ContinuationExpired);
        assert!(error.next_steps.is_empty());
    }

    #[test]
    fn the_end_of_a_session_removes_what_is_kept() {
        let store = ContinuationStore::new(None);
        let config = ContinuationConfig::default();
        let _ = store.register(
            records_request(1, 5, 1),
            &registration("x", 1),
            &config,
            at(0),
        );
        store.end_session(None);
        assert!(store.is_empty());
        assert!(read(&store, "c1").is_err());
    }

    #[test]
    fn kept_results_are_shared_by_stores_of_one_state_directory() {
        let dir = tempfile::tempdir().unwrap();
        let config = ContinuationConfig::default();
        let first = ContinuationStore::new(Some(dir.path().to_path_buf()));
        let _ = first.register(
            records_request(11, 90, 10),
            &registration("list /d", 10),
            &config,
            at(0),
        );
        let second = ContinuationStore::new(Some(dir.path().to_path_buf()));
        let part = read(&second, "").unwrap();
        assert_eq!(part.records.len(), 10);
        let third = ContinuationStore::new(Some(dir.path().to_path_buf()));
        let id = third
            .register(
                records_request(1, 3, 1),
                &registration("again", 1),
                &config,
                at(0),
            )
            .unwrap()
            .id;
        assert_eq!(id, "c2");
        assert_eq!(
            read(&third, "c1")
                .unwrap()
                .records
                .iter()
                .map(number)
                .next(),
            Some(21)
        );
    }

    fn lines_request(
        path: &std::path::Path,
        first: u64,
        last: u64,
        cut: Vec<u64>,
    ) -> ContinuationRequest {
        ContinuationRequest {
            id: None,
            source: ContinuationSource::Lines {
                path: path.display().to_string(),
                first_line: first,
                last_line: last,
                cut,
                next: Vec::new(),
            },
            unit: ContinuationUnit::Lines,
            shown: 10,
            remaining: i64::try_from(last + 1 - first).unwrap(),
            cut_lines: 0,
        }
    }

    fn texts(part: &NextPart) -> Vec<String> {
        part.records
            .iter()
            .map(|record| match record.get("text") {
                Some(Value::Text(text)) => text.clone(),
                other => panic!("not a line: {other:?}"),
            })
            .collect()
    }

    #[test]
    fn lines_are_read_from_the_file_in_parts_that_concatenate_byte_for_byte() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("spool.txt");
        let content: String = (1..=500).map(|number| format!("line {number}\n")).collect();
        std::fs::write(&path, &content).unwrap();
        let store = ContinuationStore::new(None);
        let config = ContinuationConfig::default();
        let _ = store.register(
            lines_request(&path, 11, 490, Vec::new()),
            &registration("run", 60),
            &config,
            at(0),
        );
        let mut joined = String::new();
        loop {
            let part = read(&store, "").unwrap();
            for line in texts(&part) {
                joined.push_str(&line);
                joined.push('\n');
            }
            if part.continuation.unwrap().remaining == 0 {
                break;
            }
        }
        let expected: String = (11..=490)
            .map(|number| format!("line {number}\n"))
            .collect();
        assert_eq!(joined, expected);
    }

    #[test]
    fn cut_lines_are_returned_whole_by_full() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("spool.txt");
        let long = "x".repeat(900);
        std::fs::write(&path, format!("a\n{long}\nb\nc\n{long}\nd\n")).unwrap();
        let store = ContinuationStore::new(None);
        let config = ContinuationConfig::default();
        let mut request = lines_request(&path, 7, 6, vec![2, 5]);
        request.cut_lines = 2;
        request.remaining = 0;
        let _ = store.register(request, &registration("run", 6), &config, at(0));
        let query = NextQuery {
            full: true,
            ..NextQuery::default()
        };
        let part = store.read(&query, None, None, &config, at(0)).unwrap();
        assert_eq!(texts(&part), [long.clone(), long]);
        assert_eq!(part.continuation.unwrap().cut_lines, 0);
    }

    /// Events newest first with identifiers that never repeat; the cursor is
    /// the identifier of the last event given, as a journal cursor is.
    struct Journal {
        events: Mutex<Vec<i64>>,
    }

    impl ResumeRunner for Journal {
        fn resume(&self, _program: &str, resume: ResumeOverride) -> StageResult<ResumedPart> {
            let before: i64 = resume.cursor.parse().unwrap();
            let limit = usize::try_from(resume.count.unwrap()).unwrap();
            let events = lock(&self.events);
            let older: Vec<i64> = events.iter().copied().filter(|id| *id < before).collect();
            let part: Vec<i64> = older.iter().copied().take(limit).collect();
            let next = (older.len() > part.len()).then(|| ContinuationRequest {
                id: None,
                source: ContinuationSource::Resume {
                    cursor: part.last().unwrap().to_string(),
                },
                unit: ContinuationUnit::Records,
                shown: count_of(part.len()),
                remaining: -1,
                cut_lines: 0,
            });
            Ok(ResumedPart {
                records: part.into_iter().map(row).collect(),
                next,
            })
        }
    }

    #[test]
    fn a_query_continues_after_its_cursor_without_overlap_or_gap() {
        let journal = Journal {
            events: Mutex::new((1..=100).rev().collect()),
        };
        let store = ContinuationStore::new(None);
        let config = ContinuationConfig::default();
        let request = ContinuationRequest {
            id: None,
            source: ContinuationSource::Resume {
                cursor: "91".to_owned(),
            },
            unit: ContinuationUnit::Records,
            shown: 10,
            remaining: -1,
            cut_lines: 0,
        };
        let _ = store.register(request, &registration("logs", 10), &config, at(0));
        let mut seen = Vec::new();
        let mut arrived = 100;
        loop {
            let part = store
                .read(&NextQuery::default(), None, Some(&journal), &config, at(0))
                .unwrap();
            seen.extend(part.records.iter().map(number));
            arrived += 1;
            lock(&journal.events).insert(0, arrived);
            if part.continuation.unwrap().remaining == 0 {
                break;
            }
        }
        assert_eq!(seen, (1..=90).rev().collect::<Vec<_>>());
    }

    #[test]
    fn a_query_without_its_call_cannot_be_continued() {
        let journal = Journal {
            events: Mutex::new(Vec::new()),
        };
        let store = ContinuationStore::new(None);
        let config = ContinuationConfig::default();
        let request = ContinuationRequest {
            id: None,
            source: ContinuationSource::Resume {
                cursor: "5".to_owned(),
            },
            unit: ContinuationUnit::Records,
            shown: 1,
            remaining: -1,
            cut_lines: 0,
        };
        let _ = store.register(
            request,
            &Registration {
                part: 1,
                options: None,
                ..Registration::default()
            },
            &config,
            at(0),
        );
        let error = store
            .read(&NextQuery::default(), None, Some(&journal), &config, at(0))
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::ContinuationExpired);
        let _ = Duration::ZERO;
    }
}

#[cfg(test)]
mod session_tests {
    use std::sync::Arc;

    use nurt_contracts::continuation::ResumedPart;
    use nurt_contracts::error::Result as StageResult;
    use nurt_contracts::value::{RecordKind, Value};

    use super::*;

    fn row(number: i64) -> Record {
        Record::new(RecordKind::Row).with("n", Value::Int(number))
    }

    fn number(record: &Record) -> i64 {
        match record.get("n") {
            Some(Value::Int(number)) => *number,
            other => panic!("not a row: {other:?}"),
        }
    }

    fn at() -> WallTime {
        WallTime::from_unix_nanos(0)
    }

    fn request(rows: std::ops::RangeInclusive<i64>, resume: Option<&str>) -> ContinuationRequest {
        ContinuationRequest {
            id: None,
            unit: ContinuationUnit::Records,
            shown: 50,
            remaining: if resume.is_some() { -1 } else { 10 },
            cut_lines: 0,
            source: ContinuationSource::Records {
                rows: rows.map(row).collect(),
                cut: Vec::new(),
                resume: resume.map(str::to_owned),
            },
        }
    }

    fn registration(session: Option<&str>) -> Registration {
        Registration {
            program: Some("find /data".to_owned()),
            session: session.map(str::to_owned),
            part: 50,
            options: None,
        }
    }

    /// Events newest first; the cursor is the identifier of the last event
    /// given, so events that arrive later never shift the parts.
    struct Events(Mutex<Vec<i64>>);

    impl ResumeRunner for Events {
        fn resume(&self, _program: &str, resume: ResumeOverride) -> StageResult<ResumedPart> {
            let before: i64 = resume.cursor.parse().unwrap();
            let limit = usize::try_from(resume.count.unwrap()).unwrap();
            let older: Vec<i64> = lock(&self.0)
                .iter()
                .copied()
                .filter(|id| *id < before)
                .collect();
            let part: Vec<i64> = older.iter().copied().take(limit).collect();
            let next = (older.len() > part.len()).then(|| ContinuationRequest {
                id: None,
                source: ContinuationSource::Resume {
                    cursor: part.last().unwrap().to_string(),
                },
                unit: ContinuationUnit::Records,
                shown: count_of(part.len()),
                remaining: -1,
                cut_lines: 0,
            });
            Ok(ResumedPart {
                records: part.into_iter().map(row).collect(),
                next,
            })
        }
    }

    #[test]
    fn kept_records_are_followed_by_the_cursor_without_gap_or_overlap() {
        let events = Events(Mutex::new((1..=250).rev().collect()));
        let store = ContinuationStore::new(None);
        let config = ContinuationConfig::default();
        // The first response showed events 250..201; 200..151 are kept, and the
        // query continues after event 151.
        let kept = request(151..=200, Some("151"));
        store
            .register(kept, &registration(None), &config, at())
            .unwrap();
        let mut seen = Vec::new();
        let mut arrived = 250;
        loop {
            let part = store
                .read(&NextQuery::default(), None, Some(&events), &config, at())
                .unwrap();
            seen.extend(part.records.iter().map(number));
            arrived += 1;
            lock(&events.0).insert(0, arrived);
            if part.continuation.unwrap().remaining == 0 {
                break;
            }
        }
        let expected: Vec<i64> = (151..=200).chain((1..=150).rev()).collect();
        let mut sorted = seen.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), seen.len(), "an event came twice");
        assert_eq!(seen.len(), expected.len());
        assert_eq!(
            seen.iter().copied().collect::<BTreeSet<_>>(),
            (1..=200).collect::<BTreeSet<_>>()
        );
    }

    #[test]
    fn a_continuation_of_another_session_is_not_read() {
        let store = ContinuationStore::new(None);
        let config = ContinuationConfig::default();
        store
            .register(
                request(1..=9, None),
                &registration(Some("a")),
                &config,
                at(),
            )
            .unwrap();
        let query = NextQuery {
            id: Some("c1".to_owned()),
            ..NextQuery::default()
        };
        let error = store
            .read(&query, Some("b"), None, &config, at())
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::ContinuationExpired);
        assert!(store.read(&query, Some("a"), None, &config, at()).is_ok());
    }

    #[test]
    fn the_end_of_a_session_removes_only_its_own_results() {
        let store = ContinuationStore::new(None);
        let config = ContinuationConfig::default();
        for session in [Some("a"), Some("b"), None] {
            store
                .register(request(1..=9, None), &registration(session), &config, at())
                .unwrap();
        }
        store.end_session(Some("a"));
        assert_eq!(store.len(), 2);
        let query = |id: &str| NextQuery {
            id: Some(id.to_owned()),
            ..NextQuery::default()
        };
        assert!(
            store
                .read(&query("c2"), Some("b"), None, &config, at())
                .is_ok()
        );
        assert!(store.read(&query("c3"), None, None, &config, at()).is_ok());
    }

    #[test]
    fn registrations_of_several_stores_of_one_directory_get_distinct_identifiers() {
        let dir = tempfile::tempdir().unwrap();
        let config = Arc::new(ContinuationConfig::default());
        let path = dir.path().to_path_buf();
        let handles: Vec<_> = (0..4)
            .map(|worker| {
                let path = path.clone();
                let config = Arc::clone(&config);
                std::thread::spawn(move || {
                    let store = ContinuationStore::new(Some(path));
                    (0..10)
                        .map(|_| {
                            store
                                .register(
                                    request(1..=3, None),
                                    &registration(Some(&format!("s{worker}"))),
                                    &config,
                                    at(),
                                )
                                .unwrap()
                                .id
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        let mut ids: Vec<String> = handles
            .into_iter()
            .flat_map(|handle| handle.join().unwrap())
            .collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), 40);
    }

    #[test]
    fn two_readers_of_one_continuation_never_get_the_same_part() {
        let dir = tempfile::tempdir().unwrap();
        let config = Arc::new(ContinuationConfig::default());
        let store = ContinuationStore::new(Some(dir.path().to_path_buf()));
        let mut kept = request(1..=100, None);
        kept.shown = 10;
        store
            .register(kept, &registration(None), &config, at())
            .unwrap();
        let handles: Vec<_> = (0..4)
            .map(|_| {
                let path = dir.path().to_path_buf();
                let config = Arc::clone(&config);
                std::thread::spawn(move || {
                    let store = ContinuationStore::new(Some(path));
                    (0..5)
                        .flat_map(|_| {
                            store
                                .read(&NextQuery::default(), None, None, &config, at())
                                .unwrap()
                                .records
                                .iter()
                                .map(number)
                                .collect::<Vec<_>>()
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        let mut all: Vec<i64> = handles
            .into_iter()
            .flat_map(|handle| handle.join().unwrap())
            .collect();
        all.sort_unstable();
        assert_eq!(all, (1..=100).collect::<Vec<_>>());
    }

    #[test]
    fn a_result_above_max_size_is_not_kept_and_says_why() {
        let store = ContinuationStore::new(None);
        let mut config = ContinuationConfig::default();
        config.max_size = nurt_contracts::config::SizeSetting::from_bytes(10);
        let refused = store
            .register(request(1..=50, None), &registration(None), &config, at())
            .unwrap_err();
        assert!(refused.0.contains("max_size"), "{refused:?}");
        assert!(store.is_empty());
    }

    #[test]
    fn a_removed_spool_file_names_the_original_call() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gone.txt");
        std::fs::write(&path, "a\nb\n").unwrap();
        let store = ContinuationStore::new(None);
        let config = ContinuationConfig::default();
        let request = ContinuationRequest {
            id: None,
            source: ContinuationSource::Lines {
                path: path.display().to_string(),
                first_line: 1,
                last_line: 2,
                cut: Vec::new(),
                next: Vec::new(),
            },
            unit: ContinuationUnit::Lines,
            shown: 1,
            remaining: 2,
            cut_lines: 0,
        };
        store
            .register(
                request,
                &Registration {
                    program: Some("run [\"make\"]".to_owned()),
                    session: None,
                    part: 1,
                    options: None,
                },
                &config,
                at(),
            )
            .unwrap();
        std::fs::remove_file(&path).unwrap();
        let error = store
            .read(&NextQuery::default(), None, None, &config, at())
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::ContinuationExpired);
        assert_eq!(error.next_steps, ["run [\"make\"]"]);
    }

    #[test]
    fn the_ranges_of_two_streams_are_read_one_after_the_other() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("out.txt");
        let err = dir.path().join("err.txt");
        std::fs::write(&out, "o1\no2\no3\n").unwrap();
        std::fs::write(&err, "e1\ne2\n").unwrap();
        let store = ContinuationStore::new(None);
        let config = ContinuationConfig::default();
        let request = ContinuationRequest {
            id: None,
            source: ContinuationSource::Lines {
                path: out.display().to_string(),
                first_line: 2,
                last_line: 3,
                cut: Vec::new(),
                next: vec![nurt_contracts::continuation::LineRange {
                    path: err.display().to_string(),
                    first_line: 1,
                    last_line: 2,
                    cut: Vec::new(),
                }],
            },
            unit: ContinuationUnit::Lines,
            shown: 2,
            remaining: 4,
            cut_lines: 0,
        };
        store
            .register(request, &registration(None), &config, at())
            .unwrap();
        let query = NextQuery {
            all: true,
            ..NextQuery::default()
        };
        let part = store.read(&query, None, None, &config, at()).unwrap();
        let texts: Vec<String> = part
            .records
            .iter()
            .map(|record| match record.get("text") {
                Some(Value::Text(text)) => text.clone(),
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(texts, ["o2", "o3", "e1", "e2"]);
        assert_eq!(part.continuation.unwrap().remaining, 0);
    }
}

#[cfg(test)]
mod lock_tests {
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use nurt_contracts::continuation::ResumedPart;
    use nurt_contracts::error::Result as StageResult;
    use nurt_contracts::value::{RecordKind, Value};

    use super::*;

    fn row(number: i64) -> Record {
        Record::new(RecordKind::Row).with("n", Value::Int(number))
    }

    fn at() -> WallTime {
        WallTime::from_unix_nanos(0)
    }

    fn resume_request(cursor: &str) -> ContinuationRequest {
        ContinuationRequest {
            id: None,
            source: ContinuationSource::Resume {
                cursor: cursor.to_owned(),
            },
            unit: ContinuationUnit::Records,
            shown: 5,
            remaining: -1,
            cut_lines: 0,
        }
    }

    fn registration() -> Registration {
        Registration {
            program: Some("logs".to_owned()),
            session: None,
            part: 5,
            options: None,
        }
    }

    /// Events 100 down to 1; every call takes `delay`.
    struct Slow {
        delay: Duration,
    }

    impl ResumeRunner for Slow {
        fn resume(&self, _program: &str, resume: ResumeOverride) -> StageResult<ResumedPart> {
            std::thread::sleep(self.delay);
            let before: i64 = resume.cursor.parse().unwrap();
            let limit = usize::try_from(resume.count.unwrap()).unwrap();
            let part: Vec<i64> = (1..before).rev().take(limit).collect();
            let next = part
                .last()
                .filter(|last| **last > 1)
                .map(|last| ContinuationRequest {
                    id: None,
                    source: ContinuationSource::Resume {
                        cursor: last.to_string(),
                    },
                    unit: ContinuationUnit::Records,
                    shown: 0,
                    remaining: -1,
                    cut_lines: 0,
                });
            Ok(ResumedPart {
                records: part.into_iter().map(row).collect(),
                next,
            })
        }
    }

    #[test]
    fn a_slow_query_does_not_hold_the_lock_and_registrations_get_distinct_identifiers() {
        let dir = tempfile::tempdir().unwrap();
        let config = Arc::new(ContinuationConfig::default());
        let store = Arc::new(ContinuationStore::new(Some(dir.path().to_path_buf())));
        store
            .register(resume_request("101"), &registration(), &config, at())
            .unwrap();
        let reader = {
            let store = Arc::clone(&store);
            let config = Arc::clone(&config);
            std::thread::spawn(move || {
                let runner = Slow {
                    delay: Duration::from_millis(800),
                };
                store
                    .read(&NextQuery::default(), None, Some(&runner), &config, at())
                    .unwrap()
            })
        };
        std::thread::sleep(Duration::from_millis(150));
        let started = Instant::now();
        let other = ContinuationStore::new(Some(dir.path().to_path_buf()));
        let mut ids = vec!["c1".to_owned()];
        for _ in 0..5 {
            ids.push(
                other
                    .register(resume_request("50"), &registration(), &config, at())
                    .unwrap()
                    .id,
            );
        }
        assert!(started.elapsed() < Duration::from_millis(500));
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), 6);
        let part = reader.join().unwrap();
        assert_eq!(part.records.len(), 5);
    }

    #[test]
    fn two_readers_of_a_query_continuation_never_get_the_same_events() {
        let dir = tempfile::tempdir().unwrap();
        let config = Arc::new(ContinuationConfig::default());
        let store = ContinuationStore::new(Some(dir.path().to_path_buf()));
        store
            .register(resume_request("101"), &registration(), &config, at())
            .unwrap();
        let handles: Vec<_> = (0..3)
            .map(|_| {
                let path = dir.path().to_path_buf();
                let config = Arc::clone(&config);
                std::thread::spawn(move || {
                    let store = ContinuationStore::new(Some(path));
                    let runner = Slow {
                        delay: Duration::from_millis(60),
                    };
                    (0..4)
                        .flat_map(|_| {
                            store
                                .read(&NextQuery::default(), None, Some(&runner), &config, at())
                                .unwrap()
                                .records
                        })
                        .map(|record| match record.get("n") {
                            Some(Value::Int(n)) => *n,
                            other => panic!("{other:?}"),
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        let mut all: Vec<i64> = handles
            .into_iter()
            .flat_map(|handle| handle.join().unwrap())
            .collect();
        all.sort_unstable();
        let before = all.len();
        all.dedup();
        assert_eq!(all.len(), before, "an event was given twice");
    }

    #[test]
    fn cut_lines_come_from_the_text_kept_with_the_response() {
        let store = ContinuationStore::new(None);
        let config = ContinuationConfig::default();
        let request = ContinuationRequest {
            id: None,
            source: ContinuationSource::Records {
                rows: Vec::new(),
                cut: vec![CutLine {
                    path: "/gone/file.txt".to_owned(),
                    no: 7,
                    text: "the whole original line".to_owned(),
                }],
                resume: None,
            },
            unit: ContinuationUnit::Records,
            shown: 1,
            remaining: 0,
            cut_lines: 1,
        };
        store
            .register(request, &registration(), &config, at())
            .unwrap();
        let part = store
            .read(
                &NextQuery {
                    full: true,
                    ..NextQuery::default()
                },
                None,
                None,
                &config,
                at(),
            )
            .unwrap();
        match part.records[0].get("text") {
            Some(Value::Text(text)) => assert_eq!(text, "the whole original line"),
            other => panic!("{other:?}"),
        }
    }
}
