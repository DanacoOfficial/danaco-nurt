// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

//! Stable error codes and the error structure of the language.
//!
//! Every error carries a stable code ([`ErrorCode`], written `E_UPPER_CASE`),
//! a message and, where known, the position of the error ([`At`]: the
//! execution stage and the place in the program), ready replacements of the
//! program text and a help topic. Errors of the stages `parse` and `bind` have
//! no effect on the world: they describe a mistake in the program and how to
//! correct it.
//!
//! The JSON form is the `error` member of the effect report; the list of
//! codes is published in the schema `nurt-value/1` and in the specification
//! of error codes, and both are checked against [`ErrorCode::ALL`].
//!
//! # Examples
//!
//! ```
//! use nurt_contracts::error::{At, Error, ErrorCode, Stage};
//!
//! let error = Error::new(ErrorCode::UnknownCommand, "unknown command `remvoe`")
//!     .with_stage(Stage::Bind)
//!     .with_at(At::new(1, 1, 6))
//!     .with_fix("remove \"/home/user/build/old.log\"")
//!     .with_help("commands");
//! assert_eq!(error.to_string(), "E_UNKNOWN_COMMAND: unknown command `remvoe`");
//! assert_eq!(error.stage(), Some(Stage::Bind));
//! let json = serde_json::to_value(&error).unwrap();
//! assert_eq!(json["code"], "E_UNKNOWN_COMMAND");
//! assert_eq!(json["at"]["stage"], "1");
//! assert_eq!(json["at"]["column"], 1);
//! ```

use std::borrow::Cow;
use std::fmt;
use std::ops::{Deref, DerefMut};
use std::str::FromStr;

use schemars::{JsonSchema, Schema, SchemaGenerator};
use serde::{Deserialize, Serialize};

/// Result of an operation that fails with a language [`Error`].
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Execution stage of a program, in execution order.
///
/// The JSON form is the stage number as text: `"0"` to `"7"` and `"5a"`. The
/// executor uses the same type for its execution steps
/// ([`crate::stage::ExecStep`]). The set of stages is fixed by the language.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
pub enum Stage {
    /// `0`, parse: text to surface tree with form correction; no effect.
    #[serde(rename = "0")]
    Parse,
    /// `1`, bind: binding to the catalog, types, defaults and the effect
    /// class; no effect.
    #[serde(rename = "1")]
    Bind,
    /// `2`, read: sources and transformations; reads only.
    #[serde(rename = "2")]
    Read,
    /// `3`, check: assertions before the effect; no effect.
    #[serde(rename = "3")]
    Check,
    /// `4`, plan: targets and the planned effect; previews end here.
    #[serde(rename = "4")]
    Plan,
    /// `5`, apply: the effect, recorded in the journal.
    #[serde(rename = "5")]
    Apply,
    /// `5a`, after: stages on the records of the effect; the effect stays.
    #[serde(rename = "5a")]
    After,
    /// `6`, verify: the verification pipeline; failure rolls the effect
    /// back.
    #[serde(rename = "6")]
    Verify,
    /// `7`, report: the effect report.
    #[serde(rename = "7")]
    Report,
}

impl Stage {
    /// Every stage in execution order.
    pub const ALL: &'static [Stage] = &[
        Stage::Parse,
        Stage::Bind,
        Stage::Read,
        Stage::Check,
        Stage::Plan,
        Stage::Apply,
        Stage::After,
        Stage::Verify,
        Stage::Report,
    ];

    /// Stage number as text: `0` to `7` and `5a`; the JSON form.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Parse => "0",
            Self::Bind => "1",
            Self::Read => "2",
            Self::Check => "3",
            Self::Plan => "4",
            Self::Apply => "5",
            Self::After => "5a",
            Self::Verify => "6",
            Self::Report => "7",
        }
    }

    /// Stage name: `parse`, `bind`, `read`, `check`, `plan`, `apply`,
    /// `after`, `verify` or `report`.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Parse => "parse",
            Self::Bind => "bind",
            Self::Read => "read",
            Self::Check => "check",
            Self::Plan => "plan",
            Self::Apply => "apply",
            Self::After => "after",
            Self::Verify => "verify",
            Self::Report => "report",
        }
    }

    /// Whether errors of this stage leave the world unchanged.
    #[must_use]
    pub const fn is_before_effect(self) -> bool {
        matches!(
            self,
            Self::Parse | Self::Bind | Self::Read | Self::Check | Self::Plan
        )
    }

    /// Whether the executor may change the world in this stage: apply, and
    /// verify, which rolls the effect back on failure.
    #[must_use]
    pub const fn changes_world(self) -> bool {
        matches!(self, Self::Apply | Self::Verify)
    }
}

impl fmt::Display for Stage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Stage {
    type Err = UnknownName;

    /// Parses a stage number (`5a`) or a stage name (`after`).
    fn from_str(text: &str) -> std::result::Result<Self, Self::Err> {
        Self::ALL
            .iter()
            .copied()
            .find(|stage| stage.as_str() == text || stage.name() == text)
            .ok_or_else(|| UnknownName::new("stage", text))
    }
}

/// Status of a function of the C interface (`nurt_status`).
///
/// Functions that return a response document (`nurt_run_json`,
/// `nurt_check`, `nurt_parse`, `nurt_correct`, `nurt_exec_wait`,
/// `nurt_page`, `nurt_history`, `nurt_space`) return [`AbiStatus::Ok`] for
/// every language error and carry the error in the document. Functions
/// without a response document report the error through the status given
/// by [`ErrorCode::abi`] and the detail through `nurt_last_error`.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
#[repr(u32)]
#[non_exhaustive]
pub enum AbiStatus {
    /// `NURT_OK` (0): the call completed; a language error, if any, is in
    /// the response document.
    Ok = 0,
    /// `NURT_E_TIMEOUT` (1): the wait ended before the execution finished.
    Timeout = 1,
    /// `NURT_E_CANCELLED` (2): the execution was cancelled.
    Cancelled = 2,
    /// `NURT_E_ARG` (3): an argument of the call is invalid: a null pointer,
    /// text that is not UTF-8, or a document that is not valid.
    Arg = 3,
    /// `NURT_E_ABI` (4): the library and the caller use incompatible
    /// interface versions; no language error maps to this status.
    Abi = 4,
    /// `NURT_E_IO` (5): the configuration, the journal or another file of
    /// the instance could not be opened, read or written.
    Io = 5,
    /// `NURT_E_INTERNAL` (6): an internal error of the library.
    Internal = 6,
}

impl AbiStatus {
    /// Every status in numeric order.
    pub const ALL: &'static [AbiStatus] = &[
        AbiStatus::Ok,
        AbiStatus::Timeout,
        AbiStatus::Cancelled,
        AbiStatus::Arg,
        AbiStatus::Abi,
        AbiStatus::Io,
        AbiStatus::Internal,
    ];

    /// Numeric value of the status.
    #[must_use]
    pub const fn code(self) -> u32 {
        self as u32
    }

    /// Name of the constant in `nurt.h`, for example `NURT_E_IO`.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Ok => "NURT_OK",
            Self::Timeout => "NURT_E_TIMEOUT",
            Self::Cancelled => "NURT_E_CANCELLED",
            Self::Arg => "NURT_E_ARG",
            Self::Abi => "NURT_E_ABI",
            Self::Io => "NURT_E_IO",
            Self::Internal => "NURT_E_INTERNAL",
        }
    }
}

/// Error returned for a name that is not a known stage or error code.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct UnknownName {
    /// What was being parsed: `stage` or `error code`.
    pub what: String,
    /// The rejected text.
    pub text: String,
}

impl UnknownName {
    fn new(what: &str, text: &str) -> Self {
        Self {
            what: what.to_owned(),
            text: text.to_owned(),
        }
    }
}

impl fmt::Display for UnknownName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unknown {} `{}`", self.what, self.text)
    }
}

impl std::error::Error for UnknownName {}

macro_rules! error_codes {
    ($(
        $(#[$meta:meta])*
        $variant:ident = $code:literal, [$($stage:ident),+], $abi:ident;
    )+) => {
        /// Stable error code of the language.
        ///
        /// Codes are only added within a major version of the language;
        /// the text form (`E_UNKNOWN_COMMAND`) is the JSON form.
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
            JsonSchema,
        )]
        #[non_exhaustive]
        pub enum ErrorCode {
            $( $(#[$meta])* #[serde(rename = $code)] $variant, )+
        }

        impl ErrorCode {
            /// Every error code in the order of the specification.
            pub const ALL: &'static [ErrorCode] = &[$(ErrorCode::$variant),+];

            /// Stable text of the code, for example `E_STALE`.
            ///
            /// ```
            /// use nurt_contracts::error::ErrorCode;
            ///
            /// assert_eq!(ErrorCode::JournalPartial.as_str(), "E_JOURNAL_PARTIAL");
            /// assert_eq!("E_JOURNAL_PARTIAL".parse(), Ok(ErrorCode::JournalPartial));
            /// ```
            #[must_use]
            pub const fn as_str(self) -> &'static str {
                match self {
                    $(ErrorCode::$variant => $code,)+
                }
            }

            /// Stages in which the error can occur.
            #[must_use]
            pub const fn stages(self) -> &'static [Stage] {
                match self {
                    $(ErrorCode::$variant => &[$(Stage::$stage),+],)+
                }
            }

            /// Status of a function of the C interface without a response
            /// document that ends with this error (see [`AbiStatus`]).
            ///
            /// | Codes | Status |
            /// |---|---|
            /// | `E_TIMEOUT` | `NURT_E_TIMEOUT` (1) |
            /// | `E_CANCELLED` | `NURT_E_CANCELLED` (2) |
            /// | errors of the program text, of documents and of arguments: lexical, syntax, binding, type and value errors, `E_ARG`, `E_AST_*` | `NURT_E_ARG` (3) |
            /// | file system and journal storage errors: `E_IO`, `E_PERMISSION`, `E_EXISTS`, `E_NOT_FOUND`, `E_IN_USE`, `E_PRIVILEGE`, `E_NO_SPACE`, `E_JOURNAL_CORRUPT` | `NURT_E_IO` (5) |
            /// | `E_INTERNAL`, `E_INTERNAL_CORRECTION` | `NURT_E_INTERNAL` (6) |
            /// | outcomes of an execution (assertions, stale files, rollbacks, journal, programs, network, repositories) | `NURT_OK` (0): they occur only in calls that return a response document |
            ///
            /// The complete table is in the specification of error codes.
            #[must_use]
            pub const fn abi(self) -> AbiStatus {
                match self {
                    $(ErrorCode::$variant => AbiStatus::$abi,)+
                }
            }

            /// Numeric `nurt_status` for this error; see [`ErrorCode::abi`].
            ///
            /// ```
            /// use nurt_contracts::error::ErrorCode;
            ///
            /// assert_eq!(ErrorCode::Timeout.abi_status(), 1);
            /// assert_eq!(ErrorCode::Syntax.abi_status(), 3);
            /// assert_eq!(ErrorCode::Assert.abi_status(), 0);
            /// ```
            #[must_use]
            pub const fn abi_status(self) -> u32 {
                self.abi().code()
            }
        }

        impl FromStr for ErrorCode {
            type Err = UnknownName;

            fn from_str(text: &str) -> std::result::Result<Self, Self::Err> {
                match text {
                    $($code => Ok(ErrorCode::$variant),)+
                    _ => Err(UnknownName::new("error code", text)),
                }
            }
        }
    };
}

error_codes! {
    /// A token is not valid: an unknown character, an unterminated text or
    /// a malformed number, size or duration.
    Lexical = "E_LEXICAL", [Parse], Arg;
    /// An escape sequence in a text is not valid; the help shows the
    /// accepted escapes and the raw text form.
    Escape = "E_ESCAPE", [Parse], Arg;
    /// The program does not follow the grammar.
    Syntax = "E_SYNTAX", [Parse], Arg;
    /// A correction is possible but touches a target of a change;
    /// `candidates` lists ranked ready programs.
    Ambiguous = "E_AMBIGUOUS", [Parse, Bind], Arg;
    /// The program cannot be corrected by the correction rules; `fix` and
    /// `help` show the way forward.
    Uncorrectable = "E_UNCORRECTABLE", [Parse, Bind], Arg;
    /// The correction violated an invariant; nothing is executed.
    InternalCorrection = "E_INTERNAL_CORRECTION", [Parse, Bind], Internal;
    /// `accept` names a correction candidate that expired or is unknown;
    /// the program must be sent again.
    ExpiredCandidate = "E_EXPIRED_CANDIDATE", [Parse, Bind], Arg;
    /// The version of a syntax tree is newer than the instance supports;
    /// the unknown nodes are listed.
    AstVersion = "E_AST_VERSION", [Parse], Arg;
    /// A syntax tree does not conform to its schema.
    AstSchema = "E_AST_SCHEMA", [Parse], Arg;
    /// Unknown command; `expected` lists up to eight nearest names.
    UnknownCommand = "E_UNKNOWN_COMMAND", [Bind], Arg;
    /// Unknown parameter of a command; `expected` lists up to eight nearest
    /// names.
    UnknownParam = "E_UNKNOWN_PARAM", [Bind], Arg;
    /// Unknown name `@name` of a `let` result.
    UnknownName = "E_UNKNOWN_NAME", [Bind], Arg;
    /// Unknown field of a record; `expected` lists the fields.
    UnknownField = "E_UNKNOWN_FIELD", [Bind, Read], Arg;
    /// Unknown value of an enumeration; `expected` lists the values.
    UnknownValue = "E_UNKNOWN_VALUE", [Bind], Arg;
    /// A required parameter is missing.
    MissingParam = "E_MISSING_PARAM", [Bind], Arg;
    /// Parameters that exclude each other were given together; `expected`
    /// lists the admissible combinations.
    ParamConflict = "E_PARAM_CONFLICT", [Bind], Arg;
    /// A value or a record stream has a type the place does not accept;
    /// `expected` lists the accepted types.
    Type = "E_TYPE", [Bind, Read], Arg;
    /// Wrong number of arguments.
    Arity = "E_ARITY", [Bind], Arg;
    /// A value has the right type but is not valid for the place, for
    /// example a cron expression, a time zone or a too short interval.
    InvalidValue = "E_INVALID_VALUE", [Bind], Arg;
    /// The configuration is not valid: an unknown key, a value of the wrong
    /// type or out of range, or a file that does not parse; `location`
    /// points into the file.
    Config = "E_CONFIG", [Parse, Bind], Arg;
    /// A relative path was given without the call parameter `base`.
    PathRelative = "E_PATH_RELATIVE", [Bind], Arg;
    /// A path is written in a form foreign to the system, for example a
    /// drive letter on Linux or a name Windows does not allow.
    PathForeign = "E_PATH_FOREIGN", [Bind], Arg;
    /// More than one changing or running stage, or a stage after the
    /// effect that is not allowed there; `next_steps` splits the program.
    OneEffect = "E_ONE_EFFECT", [Bind], Arg;
    /// The command exists only on another operating system.
    OsOnly = "E_OS_ONLY", [Bind], Arg;
    /// A parameter or a mode is not available on this system, for example a
    /// memory limit without kernel support.
    Unsupported = "E_UNSUPPORTED", [Bind, Apply], Arg;
    /// A regular expression or a replacement template is not valid.
    Regex = "E_REGEX", [Bind], Arg;
    /// A key path into a document is not valid.
    KeySyntax = "E_KEY_SYNTAX", [Bind], Arg;
    /// A syntax pattern does not parse in the target language.
    PatternSyntax = "E_PATTERN_SYNTAX", [Bind], Arg;
    /// A document query does not compile or fails to run.
    Query = "E_QUERY", [Bind, Read, Apply], Arg;
    /// An SQL statement cannot be prepared or executed, or changes data
    /// without `write: true`.
    Sql = "E_SQL", [Bind, Read, Apply], Arg;
    /// An argument of a call cannot be passed without changing its meaning,
    /// for example a NUL character in a process argument.
    Arg = "E_ARG", [Bind, Apply], Arg;
    /// A program required by the command is not installed; `fix` shows how
    /// to install it.
    ExternalMissing = "E_EXTERNAL_MISSING", [Bind, Apply], Ok;
    /// An external program used by the command, such as a language server,
    /// failed or broke its protocol.
    ExternalFailed = "E_EXTERNAL_FAILED", [Read, Apply], Ok;
    /// A program to run was not found in the search path; `candidates`
    /// lists similar names.
    ProgramNotFound = "E_PROGRAM_NOT_FOUND", [Bind, Apply], Ok;
    /// A program could not be started: no permission to execute, an unknown
    /// format, a missing interpreter or working directory.
    Spawn = "E_SPAWN", [Apply], Ok;
    /// An assertion does not hold. Before the effect nothing changed; after
    /// the effect the effect stays and the report is complete.
    Assert = "E_ASSERT", [Check, Plan, After], Ok;
    /// Binary content where text is required.
    Binary = "E_BINARY", [Read, Apply], Ok;
    /// A file is not a valid document of its format; `location` shows where.
    DocParse = "E_DOC_PARSE", [Read, Apply], Arg;
    /// A format is damaged or not supported, for example an archive or a
    /// notebook.
    Format = "E_FORMAT", [Read, Plan, Apply], Ok;
    /// An archive or a document is encrypted.
    Encrypted = "E_ENCRYPTED", [Read], Ok;
    /// A construct of a document cannot be changed without losing its
    /// layout; the construct is named in `found`; nothing changed.
    UnsupportedConstruct = "E_UNSUPPORTED_CONSTRUCT", [Plan, Apply], Ok;
    /// A key path does not exist in the document.
    KeyNotFound = "E_KEY_NOT_FOUND", [Read, Apply], Ok;
    /// A key path passes through a value of another kind, for example an
    /// index into a map.
    KeyConflict = "E_KEY_CONFLICT", [Apply], Ok;
    /// No grammar is available for the language of a file.
    UnsupportedLanguage = "E_UNSUPPORTED_LANGUAGE", [Read, Apply], Ok;
    /// A symbol has no definition.
    SymbolNotFound = "E_SYMBOL_NOT_FOUND", [Read, Apply], Ok;
    /// A symbol has several definitions; `candidates` lists ready programs.
    SymbolAmbiguous = "E_SYMBOL_AMBIGUOUS", [Read, Apply], Ok;
    /// A new name collides with an existing symbol in its scope.
    RenameConflict = "E_RENAME_CONFLICT", [Plan, Apply], Ok;
    /// A replacement or an edit found no match while the call required one.
    NoMatch = "E_NO_MATCH", [Plan], Ok;
    /// A hunk of a patch does not apply.
    PatchRejected = "E_PATCH_REJECTED", [Plan], Ok;
    /// A line number, a page or a position lies outside the content, for
    /// example a line after the end of the file.
    OutOfRange = "E_OUT_OF_RANGE", [Read, Plan], Ok;
    /// A file changed after it was read; the whole operation was rolled
    /// back.
    Stale = "E_STALE", [Apply], Ok;
    /// A character cannot be represented in the encoding of the file.
    Encoding = "E_ENCODING", [Apply], Ok;
    /// Input or output error of the file system.
    Io = "E_IO", [Read, Apply], Io;
    /// Access denied by permissions.
    Permission = "E_PERMISSION", [Read, Apply], Io;
    /// The target already exists.
    Exists = "E_EXISTS", [Read, Apply], Io;
    /// The target does not exist.
    NotFound = "E_NOT_FOUND", [Read, Apply], Io;
    /// The target is in use; the processes holding it are listed.
    InUse = "E_IN_USE", [Read, Apply], Io;
    /// The operation needs privileges that are not available: no elevation
    /// backend, or the system denied the elevation to the account; `backend`
    /// lists the tried backends and `next_steps` the way forward.
    Privilege = "E_PRIVILEGE", [Read, Apply], Io;
    /// Not enough space on the volume.
    NoSpace = "E_NO_SPACE", [Read, Apply], Io;
    /// A time limit was reached.
    Timeout = "E_TIMEOUT", [Read, Apply], Timeout;
    /// The execution was cancelled.
    Cancelled = "E_CANCELLED", [Read, Apply], Cancelled;
    /// A resource limit was reached.
    Limit = "E_LIMIT", [Read, Apply], Ok;
    /// A stage failed; `partial_read` in `details` may carry what was read.
    StageFailed = "E_STAGE_FAILED", [Read, Apply, Verify], Ok;
    /// The verification failed and the effect was rolled back.
    RolledBack = "E_ROLLED_BACK", [Read, Apply, Verify], Ok;
    /// A rollback could not restore everything; the files are listed with
    /// instructions.
    RollbackFailed = "E_ROLLBACK_FAILED", [Read, Apply, Verify], Ok;
    /// No such operation in the journal, or nothing to undo or redo.
    JournalNoop = "E_JOURNAL_NOOP", [Plan], Ok;
    /// An entry of the journal has expired; nothing changed.
    JournalExpired = "E_JOURNAL_EXPIRED", [Plan], Ok;
    /// The current state differs from the state after the operation; the
    /// differences and the choices are listed; nothing changed.
    JournalConflict = "E_JOURNAL_CONFLICT", [Plan], Ok;
    /// The entry is partial: some items were released; the call asked for
    /// the whole entry; nothing changed.
    JournalPartial = "E_JOURNAL_PARTIAL", [Plan], Ok;
    /// An entry of the journal failed its integrity check; its data is kept
    /// for repair.
    JournalCorrupt = "E_JOURNAL_CORRUPT", [Plan, Apply], Io;
    /// An unknown background job.
    UnknownJob = "E_UNKNOWN_JOB", [Read], Ok;
    /// The action is not allowed in the current state of the job, for
    /// example resuming a job that is not suspended.
    JobState = "E_JOB_STATE", [Apply], Ok;
    /// The test framework of the project was not recognised.
    TestNoFramework = "E_TEST_NO_FRAMEWORK", [Read], Ok;
    /// An unknown test run.
    UnknownRun = "E_UNKNOWN_RUN", [Read], Ok;
    /// An unknown analysis.
    UnknownAnalysis = "E_UNKNOWN_ANALYSIS", [Read], Ok;
    /// The output of a test framework or an analyser could not be read; the
    /// raw execution result is included.
    OutputParse = "E_OUTPUT_PARSE", [Apply], Ok;
    /// A network error: name resolution, connection, TLS, proxy, redirects
    /// or protocol.
    Network = "E_NETWORK", [Read, Apply], Ok;
    /// A downloaded file does not have the expected hash; it was not kept.
    HashMismatch = "E_HASH_MISMATCH", [Apply], Ok;
    /// The service manager reported an error.
    Service = "E_SERVICE", [Apply], Ok;
    /// The background service of the instance is not running.
    ServiceNotRunning = "E_SERVICE_NOT_RUNNING", [Read, Apply], Ok;
    /// A program catalog configured explicitly cannot be read.
    Catalog = "E_CATALOG", [Read], Ok;
    /// The path is not inside a repository; `candidates` lists nearby
    /// repositories.
    GitNotRepository = "E_GIT_NOT_REPOSITORY", [Read], Ok;
    /// A revision is unknown, ambiguous or malformed.
    GitRevision = "E_GIT_REVISION", [Read], Ok;
    /// A reference name is invalid, exists, is missing, is not merged or is
    /// checked out elsewhere.
    GitRef = "E_GIT_REF", [Read, Apply], Ok;
    /// Local changes would be overwritten; nothing changed.
    GitDirty = "E_GIT_DIRTY", [Plan], Ok;
    /// A merge ended with conflicts; the effect is applied and the conflicts
    /// are listed.
    GitConflict = "E_GIT_CONFLICT", [Apply], Ok;
    /// A merge or rebase is in progress, or none is when one is required.
    GitInProgress = "E_GIT_IN_PROGRESS", [Plan], Ok;
    /// A lock file of the repository exists.
    GitLocked = "E_GIT_LOCKED", [Plan, Apply], Ok;
    /// The histories diverged where only a fast-forward is allowed.
    GitNonFastForward = "E_GIT_NON_FAST_FORWARD", [Plan], Ok;
    /// There is nothing to commit.
    GitNothingToCommit = "E_GIT_NOTHING_TO_COMMIT", [Plan], Ok;
    /// A hook ended with a non-zero exit code; no commit was created.
    GitHook = "E_GIT_HOOK", [Apply], Ok;
    /// A commit or a tag could not be signed.
    GitSign = "E_GIT_SIGN", [Apply], Ok;
    /// Authentication is required or was rejected by the remote.
    GitAuth = "E_GIT_AUTH", [Apply], Ok;
    /// The host key of the remote is unknown or changed.
    GitHostKey = "E_GIT_HOST_KEY", [Apply], Ok;
    /// The remote rejected an update of at least one reference.
    GitPushRejected = "E_GIT_PUSH_REJECTED", [Apply], Ok;
    /// The remote broke the protocol or lacks a required capability.
    GitProtocol = "E_GIT_PROTOCOL", [Apply], Ok;
    /// A feature of the repository is not supported.
    GitUnsupported = "E_GIT_UNSUPPORTED", [Read, Plan], Ok;
    /// A hunk identifier is malformed or unknown, the hunk changed since it
    /// was listed, or the selected lines cannot be applied.
    GitHunk = "E_GIT_HUNK", [Plan, Apply], Ok;
    /// A checkpoint has no saved index, or its content does not match after
    /// a restore; the paths are listed.
    GitCheckpoint = "E_GIT_CHECKPOINT", [Plan, Verify], Ok;
    /// A worktree is unknown, is the main worktree, is locked or contains
    /// the working directory.
    GitWorktree = "E_GIT_WORKTREE", [Plan], Ok;
    /// The configuration of the repository is not valid: a value cannot be
    /// parsed or a required key is missing.
    GitConfig = "E_GIT_CONFIG", [Read, Plan, Apply], Ok;
    /// An internal error; the report is complete up to this point.
    Internal = "E_INTERNAL", [Parse, Bind, Read, Check, Plan, Apply, After, Verify, Report], Internal;
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Position of an error: the execution stage and the place in the program
/// text.
///
/// Lines and columns start at 1; columns and lengths count Unicode scalar
/// values. Errors raised while a stage runs (for example `E_STALE` in stage
/// `5`) carry the stage without a place in the text.
///
/// ```
/// use nurt_contracts::error::{At, Stage};
///
/// let at = At::new(2, 5, 3).with_stage(Stage::Bind);
/// assert_eq!(
///     serde_json::to_value(at).unwrap(),
///     serde_json::json!({"line": 2, "column": 5, "length": 3, "stage": "1"})
/// );
/// assert_eq!(At::stage(Stage::After).line, None);
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub struct At {
    /// Line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    /// Column.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub column: Option<u32>,
    /// Number of characters of the erroneous fragment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub length: Option<u32>,
    /// Execution stage in which the error occurred.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage: Option<Stage>,
}

impl At {
    /// Place of a fragment of the given length in the program text.
    #[must_use]
    pub const fn new(line: u32, column: u32, length: u32) -> Self {
        Self {
            line: Some(line),
            column: Some(column),
            length: Some(length),
            stage: None,
        }
    }

    /// Position with a stage and no place in the program text.
    #[must_use]
    pub const fn stage(stage: Stage) -> Self {
        Self {
            line: None,
            column: None,
            length: None,
            stage: Some(stage),
        }
    }

    /// Sets the stage.
    #[must_use]
    pub const fn with_stage(mut self, stage: Stage) -> Self {
        self.stage = Some(stage);
        self
    }
}

/// Position in a data file, as opposed to [`At`], which points into the
/// program.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub struct Location {
    /// File.
    pub path: String,
    /// Line, from 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u64>,
    /// Column, from 1, in Unicode scalar values.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub column: Option<u64>,
    /// Last line, inclusive.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_line: Option<u64>,
    /// Column directly after the fragment on the last line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_column: Option<u64>,
    /// Byte offset from the start of the file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub byte: Option<u64>,
}

/// Ranked ready program offered instead of an ambiguous or uncorrectable
/// one.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub struct Candidate {
    /// Rank, from 1 for the most likely candidate.
    pub rank: u32,
    /// Identifier for `accept`, valid for a limited time.
    #[serde(rename = "ref", default, skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
    /// Complete program text of the candidate.
    pub program: String,
    /// Why the candidate is offered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
}

/// Content of an [`Error`].
///
/// [`Error`] dereferences to this structure, so the fields are read as
/// `error.code`, `error.message` and so on.
///
/// A reader ignores fields it does not know: a later minor version of the
/// format may add optional error fields.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ErrorData {
    /// Stable code.
    pub code: ErrorCode,
    /// Message in English, one or two sentences.
    pub message: String,
    /// Stage in which the error occurred and position in the program.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<At>,
    /// Position in a data file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<Location>,
    /// The fragment that was found.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub found: Option<String>,
    /// Short name of the specific problem, for example `not_merged`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub problem: Option<String>,
    /// Ready replacements of the program text, best first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fix: Vec<String>,
    /// Help topic with the rules that apply.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub help: Option<String>,
    /// What was expected: names, types, counts or hashes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected: Option<serde_json::Value>,
    /// What was found instead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actual: Option<serde_json::Value>,
    /// Sample of the records or paths concerned.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sample: Vec<String>,
    /// Ranked ready programs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub candidates: Vec<Candidate>,
    /// Further calls that complete the task, for example the parts of a
    /// program split by `E_ONE_EFFECT`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub next_steps: Vec<String>,
    /// Whether the effect of the program is in the world despite the error,
    /// as after a failed assertion in stage `5a` or a merge with conflicts.
    #[serde(default, skip_serializing_if = "is_false")]
    pub effect_applied: bool,
    /// Data specific to the code, for example `path`, `os_error`,
    /// `holders` or `partial_read`.
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub details: serde_json::Map<String, serde_json::Value>,
    /// Error that caused this one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cause: Option<Error>,
}

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde passes fields by reference"
)]
const fn is_false(value: &bool) -> bool {
    !*value
}

/// Error of the language.
///
/// The content is boxed so that results stay small; the fields of
/// [`ErrorData`] are reachable through dereferencing.
///
/// ```
/// use nurt_contracts::error::{Error, ErrorCode, Stage};
///
/// let mut error = Error::new(ErrorCode::Assert, "expected 12 paths, found 14")
///     .with_stage(Stage::Check)
///     .with_expected(serde_json::json!(12))
///     .with_actual(serde_json::json!(14));
/// assert_eq!(error.code, ErrorCode::Assert);
/// error.sample.push("/home/user/src/a.go".to_owned());
/// assert_eq!(error.sample.len(), 1);
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Error(Box<ErrorData>);

impl Error {
    /// Error with the code and the message.
    #[must_use]
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self(Box::new(ErrorData {
            code,
            message: message.into(),
            at: None,
            location: None,
            found: None,
            problem: None,
            fix: Vec::new(),
            help: None,
            expected: None,
            actual: None,
            sample: Vec::new(),
            candidates: Vec::new(),
            next_steps: Vec::new(),
            effect_applied: false,
            details: serde_json::Map::new(),
            cause: None,
        }))
    }

    /// The code.
    #[must_use]
    pub fn code(&self) -> ErrorCode {
        self.0.code
    }

    /// The content.
    #[must_use]
    pub fn data(&self) -> &ErrorData {
        &self.0
    }

    /// Unwraps the content.
    #[must_use]
    pub fn into_data(self) -> ErrorData {
        *self.0
    }

    /// Stage in which the error occurred (`at.stage`).
    #[must_use]
    pub fn stage(&self) -> Option<Stage> {
        self.0.at.and_then(|at| at.stage)
    }

    /// Sets the stage, keeping the place in the program.
    #[must_use]
    pub fn with_stage(mut self, stage: Stage) -> Self {
        self.0.at = Some(self.0.at.unwrap_or_default().with_stage(stage));
        self
    }

    /// Sets the place in the program; a stage already set is kept when `at`
    /// has none.
    #[must_use]
    pub fn with_at(mut self, at: At) -> Self {
        let stage = at.stage.or_else(|| self.stage());
        self.0.at = Some(At { stage, ..at });
        self
    }

    /// Sets the position in a data file.
    #[must_use]
    pub fn with_location(mut self, location: Location) -> Self {
        self.0.location = Some(location);
        self
    }

    /// Sets the fragment that was found.
    #[must_use]
    pub fn with_found(mut self, found: impl Into<String>) -> Self {
        self.0.found = Some(found.into());
        self
    }

    /// Sets the name of the specific problem.
    #[must_use]
    pub fn with_problem(mut self, problem: impl Into<String>) -> Self {
        self.0.problem = Some(problem.into());
        self
    }

    /// Adds a ready replacement of the program text.
    #[must_use]
    pub fn with_fix(mut self, program: impl Into<String>) -> Self {
        self.0.fix.push(program.into());
        self
    }

    /// Sets the help topic.
    #[must_use]
    pub fn with_help(mut self, topic: impl Into<String>) -> Self {
        self.0.help = Some(topic.into());
        self
    }

    /// Sets what was expected.
    #[must_use]
    pub fn with_expected(mut self, expected: serde_json::Value) -> Self {
        self.0.expected = Some(expected);
        self
    }

    /// Sets what was found instead.
    #[must_use]
    pub fn with_actual(mut self, actual: serde_json::Value) -> Self {
        self.0.actual = Some(actual);
        self
    }

    /// Adds an item of the sample.
    #[must_use]
    pub fn with_sample(mut self, item: impl Into<String>) -> Self {
        self.0.sample.push(item.into());
        self
    }

    /// Adds a ranked candidate.
    #[must_use]
    pub fn with_candidate(mut self, candidate: Candidate) -> Self {
        self.0.candidates.push(candidate);
        self
    }

    /// Adds a further call.
    #[must_use]
    pub fn with_next_step(mut self, program: impl Into<String>) -> Self {
        self.0.next_steps.push(program.into());
        self
    }

    /// Marks that the effect of the program is in the world.
    #[must_use]
    pub fn with_effect_applied(mut self, applied: bool) -> Self {
        self.0.effect_applied = applied;
        self
    }

    /// Sets a code-specific detail.
    #[must_use]
    pub fn with_detail(mut self, name: &str, value: serde_json::Value) -> Self {
        self.0.details.insert(name.to_owned(), value);
        self
    }

    /// Sets the error that caused this one.
    #[must_use]
    pub fn with_cause(mut self, cause: Self) -> Self {
        self.0.cause = Some(cause);
        self
    }
}

impl From<ErrorData> for Error {
    fn from(data: ErrorData) -> Self {
        Self(Box::new(data))
    }
}

impl From<crate::fs::FsError> for Error {
    /// Converts a file system error: the code follows the kind of the error;
    /// the kind, the operation, the path, the system error code and the
    /// processes holding the file become details.
    fn from(error: crate::fs::FsError) -> Self {
        let code = ErrorCode::from_str(error.code_id()).unwrap_or(ErrorCode::Io);
        let kind = error.kind.as_str();
        let message = match &error.path {
            Some(path) => format!("{} failed ({kind}): {}", error.op, path.display()),
            None => format!("{} failed ({kind})", error.op),
        };
        let mut converted = Self::new(code, message)
            .with_detail("kind", serde_json::Value::from(kind))
            .with_detail("op", serde_json::Value::from(error.op.as_str()));
        if let Some(path) = &error.path {
            let path = path.to_string_lossy().into_owned();
            converted = converted.with_detail("path", serde_json::Value::from(path));
        }
        if let Some(os_code) = error.os_code {
            converted = converted.with_detail("os_error", serde_json::Value::from(os_code));
        }
        if !error.holders.is_empty() {
            let holders = serde_json::to_value(&error.holders).unwrap_or_default();
            converted = converted.with_detail("holders", holders);
        }
        converted
    }
}

impl Deref for Error {
    type Target = ErrorData;

    fn deref(&self) -> &ErrorData {
        &self.0
    }
}

impl DerefMut for Error {
    fn deref_mut(&mut self) -> &mut ErrorData {
        &mut self.0
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.0.code, self.0.message)
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.0
            .cause
            .as_ref()
            .map(|cause| cause as &(dyn std::error::Error + 'static))
    }
}

impl JsonSchema for Error {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("Error")
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        generator.subschema_for::<ErrorData>()
    }
}
