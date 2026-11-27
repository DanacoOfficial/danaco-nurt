// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Continuation of shortened results.
//!
//! A response that leaves out lines, records or bytes carries the field
//! `continuation` ([`Continuation`]) and its text ends with `more: next`.
//! The command `next` returns the next part. Stages that shorten their own
//! output describe what remains with a [`ContinuationRequest`] given to the
//! [`ContinuationSink`] of the execution; the engine keeps the remainder and
//! assigns the identifier.

use std::sync::{Mutex, MutexGuard, PoisonError};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::value::Record;

/// Name of the command that reads the next part.
pub const NEXT_COMMAND: &str = "next";

/// Unit of the parts of a continuation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ContinuationUnit {
    /// Lines of text.
    Lines,
    /// Records of a result.
    Records,
    /// Bytes of content.
    Bytes,
}

impl ContinuationUnit {
    /// Name of the unit as written in the response.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Lines => "lines",
            Self::Records => "records",
            Self::Bytes => "bytes",
        }
    }
}

/// Field `continuation` of a response that leaves something out.
///
/// # Examples
///
/// ```
/// use nurt_contracts::continuation::{Continuation, ContinuationUnit};
///
/// let continuation = Continuation {
///     id: "c1".to_owned(),
///     unit: ContinuationUnit::Lines,
///     shown: 25,
///     remaining: 975,
///     cut_lines: 0,
///     older: false,
/// };
/// assert_eq!(continuation.more_text(), "more: next");
/// let older = Continuation { older: true, ..continuation };
/// assert_eq!(older.more_text(), "more: next c1");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Continuation {
    /// Short identifier, `c1`, `c2`, … within the session of the console.
    pub id: String,
    /// Unit of the parts.
    pub unit: ContinuationUnit,
    /// Units in the responses given so far.
    pub shown: u64,
    /// Units not yet given; `-1` when not known in advance.
    pub remaining: i64,
    /// Lines of the shown part that were shortened by `max_line` or
    /// `max_chars`.
    #[serde(default)]
    pub cut_lines: u64,
    /// A newer continuation exists, so the hint names the identifier. Not
    /// part of the response; set by the engine for the text of the response.
    #[serde(default, skip)]
    #[schemars(skip)]
    pub older: bool,
}

impl Continuation {
    /// Whether something remains to read or some lines were cut.
    #[must_use]
    pub const fn has_more(&self) -> bool {
        self.remaining != 0 || self.cut_lines > 0
    }

    /// The hint that ends the one-line text of a response: `more: next` for
    /// the latest continuation, `more: next cN` for an older one. A last
    /// part (`remaining` 0, no cut lines) has no hint; see [`Self::has_more`].
    #[must_use]
    pub fn more_text(&self) -> String {
        if self.older {
            format!("more: {NEXT_COMMAND} {}", self.id)
        } else {
            format!("more: {NEXT_COMMAND}")
        }
    }
}

/// A line of a file that a response cut.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CutLine {
    /// File.
    pub path: String,
    /// Line number, from 1.
    pub no: u64,
    /// The whole text of the line at the time of the response, without the
    /// line ending.
    #[serde(default)]
    pub text: String,
}

/// Lines of one file that a response leaves out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineRange {
    /// File with the lines.
    pub path: String,
    /// First line not yet given.
    pub first_line: u64,
    /// Last line of the remainder.
    pub last_line: u64,
    /// Lines of the file that the shown part cut.
    #[serde(default)]
    pub cut: Vec<u64>,
}

/// What remains of a result and where the engine finds it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "snake_case")]
pub enum ContinuationSource {
    /// Lines of a file, from `first_line` to `last_line` (both from 1,
    /// inclusive): the spool file of a run; `next` holds the ranges of the
    /// other streams, read after this one.
    Lines {
        /// File with the lines.
        path: String,
        /// First line not yet given.
        first_line: u64,
        /// Last line of the remainder.
        last_line: u64,
        /// Lines of the file that the shown part cut, by line number.
        #[serde(default)]
        cut: Vec<u64>,
        /// Ranges of further files, read when this one is exhausted.
        #[serde(default)]
        next: Vec<LineRange>,
    },
    /// Records computed by the program and kept in the order of the
    /// response.
    Records {
        /// The records not yet given.
        rows: Vec<Record>,
        /// The lines of the shown part that the response cut; `next full`
        /// reads them whole from their files.
        #[serde(default)]
        cut: Vec<CutLine>,
        /// Cursor of a query that continues after the kept records.
        #[serde(default)]
        resume: Option<String>,
    },
    /// A query that continues after a cursor of its own: `logs`.
    Resume {
        /// Cursor of the query at the time of the response.
        cursor: String,
    },
    /// A continuation that already exists and that `next` advanced.
    Existing,
}

/// Description of one continuation, given by a stage or by the engine.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContinuationRequest {
    /// Identifier of an existing continuation (`next`); `None` for a new one.
    #[serde(default)]
    pub id: Option<String>,
    /// Where the remainder is.
    #[serde(flatten)]
    pub source: ContinuationSource,
    /// Unit of the parts.
    pub unit: ContinuationUnit,
    /// Units in the response.
    pub shown: u64,
    /// Units left; `-1` when not known.
    pub remaining: i64,
    /// Lines of the response that were shortened.
    #[serde(default)]
    pub cut_lines: u64,
}

/// Collects the continuations that the stages of one execution describe.
///
/// The engine adds one sink to the extensions of every execution and keeps
/// what the stages put in it.
#[derive(Debug, Default)]
pub struct ContinuationSink {
    requests: Mutex<Vec<ContinuationRequest>>,
}

impl ContinuationSink {
    /// An empty sink.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a description.
    pub fn push(&self, request: ContinuationRequest) {
        self.lock().push(request);
    }

    /// Takes the descriptions collected so far.
    #[must_use]
    pub fn take(&self) -> Vec<ContinuationRequest> {
        std::mem::take(&mut *self.lock())
    }

    fn lock(&self) -> MutexGuard<'_, Vec<ContinuationRequest>> {
        self.requests.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Position that a query given by `next` continues from; the engine puts it
/// in the extensions of the execution that repeats the program of the first
/// response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumeOverride {
    /// Cursor to continue after.
    pub cursor: String,
    /// Number of units of the part; the program's own size when `None`.
    pub count: Option<u64>,
    /// Options of the first call (`session`, `base`, `timeout`, …) as JSON,
    /// kept so that the query runs again under the same conditions.
    pub options: Option<serde_json::Value>,
}

/// A part read by running the program of a continuation again from its
/// cursor.
#[derive(Debug, Clone, PartialEq)]
pub struct ResumedPart {
    /// Records of the part.
    pub records: Vec<Record>,
    /// What remains after the part, when anything does.
    pub next: Option<ContinuationRequest>,
}

/// Runs the program of a continuation from a cursor; supplied by the engine.
pub trait ResumeRunner: Send + Sync {
    /// Runs `program` with `resume` and returns the records of the part.
    ///
    /// # Errors
    ///
    /// Fails with the error of the program.
    fn resume(&self, program: &str, resume: ResumeOverride) -> Result<ResumedPart>;
}
