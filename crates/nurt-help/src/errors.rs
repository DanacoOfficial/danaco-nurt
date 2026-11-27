// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-help::errors
// Responsibility:   Meanings and fixes of the error codes, and the help reference attached.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Meanings and fixes of the error codes, and the help reference attached
//! to an error.

use nurt_contracts::error::{Error, ErrorCode};

/// Meaning and typical fix of one error code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ErrorNote {
    /// The code.
    pub code: ErrorCode,
    /// What the error means.
    pub meaning: &'static str,
    /// What to change in the program or the situation.
    pub fix: &'static str,
}

const fn note(code: ErrorCode, meaning: &'static str, fix: &'static str) -> ErrorNote {
    ErrorNote { code, meaning, fix }
}

/// Notes of all error codes, in the order of the codes.
pub const NOTES: &[ErrorNote] = &[
    note(
        ErrorCode::Lexical,
        "a token is not valid: an unknown character, an unterminated text or a malformed number, size or duration",
        "fix the token shown at the position; see help syntax for the literals",
    ),
    note(
        ErrorCode::Escape,
        "an escape sequence in a text is not valid",
        "use \\\\ \\\" \\n \\t \\u{..} or the raw form r\"...\"",
    ),
    note(
        ErrorCode::Syntax,
        "the program does not follow the grammar",
        "compare with the fix shown or with help syntax",
    ),
    note(
        ErrorCode::Ambiguous,
        "a correction is possible but touches the target of a change",
        "send one of the candidates as shown, or write the program in canonical form",
    ),
    note(
        ErrorCode::Uncorrectable,
        "the program cannot be corrected by the correction rules",
        "use the fix or help shown with the error; see help corrections",
    ),
    note(
        ErrorCode::InternalCorrection,
        "a correction violated an invariant; nothing was executed",
        "send the program in canonical form and report the case",
    ),
    note(
        ErrorCode::ExpiredCandidate,
        "accept names a candidate that expired or is unknown",
        "send the program again to obtain new candidates",
    ),
    note(
        ErrorCode::AstVersion,
        "the version of a syntax tree is newer than the instance supports",
        "use a tree of a supported version or update the instance",
    ),
    note(
        ErrorCode::AstSchema,
        "a syntax tree does not conform to its schema",
        "validate the tree against the published schema",
    ),
    note(
        ErrorCode::UnknownCommand,
        "the command is not in the catalog",
        "choose a name from expected; help lists the commands",
    ),
    note(
        ErrorCode::UnknownParam,
        "the command has no such parameter",
        "choose a name from expected; help <command> lists the parameters",
    ),
    note(
        ErrorCode::UnknownName,
        "the name @name is not a result of a let in the program",
        "define it with let before use or choose a name from expected",
    ),
    note(
        ErrorCode::UnknownField,
        "the records have no such field",
        "choose a field from expected; describe lists the fields",
    ),
    note(
        ErrorCode::UnknownValue,
        "the value is not one of the enumeration values",
        "choose a value from expected",
    ),
    note(
        ErrorCode::MissingParam,
        "a required parameter is missing",
        "give the parameter; help <command> shows which are required",
    ),
    note(
        ErrorCode::ParamConflict,
        "parameters that exclude each other were given together",
        "keep one of the parameters listed in expected",
    ),
    note(
        ErrorCode::Type,
        "a value or a record stream has a type the place does not accept",
        "convert the value or change the preceding stage; expected lists the accepted types",
    ),
    note(
        ErrorCode::Arity,
        "the number of arguments is wrong",
        "give the arguments the signature lists",
    ),
    note(
        ErrorCode::InvalidValue,
        "the value has the right type but is not valid for the place",
        "correct the value, for example a cron expression or a time zone",
    ),
    note(
        ErrorCode::Config,
        "the configuration is not valid",
        "fix the key at the location shown",
    ),
    note(
        ErrorCode::PathRelative,
        "a relative path was given without the call parameter base",
        "write an absolute path or give base",
    ),
    note(
        ErrorCode::PathForeign,
        "the path is written in a form foreign to the system",
        "write the path in the notation of the host",
    ),
    note(
        ErrorCode::OneEffect,
        "the program has more than one changing or running stage, or a stage after the effect that is not allowed there",
        "send the parts listed in next_steps as separate calls",
    ),
    note(
        ErrorCode::OsOnly,
        "the command exists only on another operating system",
        "use the equivalent command of this system",
    ),
    note(
        ErrorCode::Unsupported,
        "the parameter or mode is not available on this system",
        "omit the parameter or use another mode",
    ),
    note(
        ErrorCode::Regex,
        "a regular expression or a replacement template is not valid",
        "fix the expression; the syntax is RE2",
    ),
    note(
        ErrorCode::KeySyntax,
        "a key path into a document is not valid",
        "write the key as a.b, a[0].b or a[*]",
    ),
    note(
        ErrorCode::PatternSyntax,
        "a syntax pattern does not parse in the target language",
        "write a pattern that is valid code of the language",
    ),
    note(
        ErrorCode::Query,
        "a document query does not compile or fails to run",
        "fix the query expression",
    ),
    note(
        ErrorCode::Sql,
        "an SQL statement cannot be prepared or executed, or changes data with write: false",
        "fix the statement or remove write: false",
    ),
    note(
        ErrorCode::Arg,
        "an argument cannot be passed without changing its meaning",
        "remove the character the error names",
    ),
    note(
        ErrorCode::ContinuationExpired,
        "the continuation read by next expired or was removed, or none is kept",
        "run the call from next_steps again",
    ),
    note(
        ErrorCode::ExternalMissing,
        "a program the command requires is not installed",
        "install the program; fix shows how",
    ),
    note(
        ErrorCode::ExternalFailed,
        "an external program used by the command failed or broke its protocol",
        "check the program and run the command again",
    ),
    note(
        ErrorCode::ProgramNotFound,
        "the program to run was not found in the search path",
        "choose a name from candidates or give the path of the program",
    ),
    note(
        ErrorCode::Spawn,
        "the program could not be started",
        "check permission to execute, the interpreter and the working directory",
    ),
    note(
        ErrorCode::Assert,
        "an assertion does not hold; before the effect nothing changed, after the effect the effect stays",
        "inspect the actual value in the error and correct the program or the expectation",
    ),
    note(
        ErrorCode::Binary,
        "the content is binary where text is required",
        "use view: hex or a command for binary content",
    ),
    note(
        ErrorCode::DocParse,
        "the file is not a valid document of its format",
        "fix the document at the location shown",
    ),
    note(
        ErrorCode::Format,
        "a format is damaged or not supported",
        "check the file; convert it to a supported format",
    ),
    note(
        ErrorCode::Encrypted,
        "an archive or a document is encrypted",
        "decrypt it with a program outside the console",
    ),
    note(
        ErrorCode::UnsupportedConstruct,
        "a construct of the document cannot be changed without losing its layout",
        "change the construct named in found by another command",
    ),
    note(
        ErrorCode::KeyNotFound,
        "the key path does not exist in the document",
        "check the key with read or create it with create: true",
    ),
    note(
        ErrorCode::KeyConflict,
        "the key path passes through a value of another kind",
        "use a key path that matches the structure",
    ),
    note(
        ErrorCode::UnsupportedLanguage,
        "no grammar is available for the language of the file",
        "use text commands for this file",
    ),
    note(
        ErrorCode::SymbolNotFound,
        "the symbol has no definition",
        "check the name with code.symbols",
    ),
    note(
        ErrorCode::SymbolAmbiguous,
        "the symbol has several definitions",
        "send one of the candidates",
    ),
    note(
        ErrorCode::RenameConflict,
        "the new name collides with an existing symbol in its scope",
        "choose another name",
    ),
    note(
        ErrorCode::NoMatch,
        "a replacement or an edit found no match while the call required one",
        "check the pattern with search; relax expect",
    ),
    note(
        ErrorCode::PatchRejected,
        "a hunk of the patch does not apply",
        "regenerate the patch against the current content",
    ),
    note(
        ErrorCode::OutOfRange,
        "a line number, a page or a position lies outside the content",
        "use a position inside the content; read the file to see its length",
    ),
    note(
        ErrorCode::Stale,
        "a file changed after it was read; the whole operation was rolled back",
        "read the files again and repeat the call",
    ),
    note(
        ErrorCode::Encoding,
        "a character cannot be represented in the encoding of the file",
        "convert the file or choose another encoding",
    ),
    note(
        ErrorCode::Io,
        "input or output error of the file system",
        "check the device and repeat the call",
    ),
    note(
        ErrorCode::Permission,
        "access denied by permissions",
        "change the permissions with attributes or choose another path",
    ),
    note(
        ErrorCode::Exists,
        "the target already exists",
        "use on_conflict or choose another target",
    ),
    note(
        ErrorCode::NotFound,
        "the target does not exist",
        "check the path with info",
    ),
    note(
        ErrorCode::InUse,
        "the target is in use; the processes holding it are listed",
        "stop the processes or choose another target",
    ),
    note(
        ErrorCode::Privilege,
        "the operation requires a privilege the account does not have",
        "run the console with the required privilege",
    ),
    note(
        ErrorCode::NoSpace,
        "not enough space on the volume",
        "free space; journal.space shows what the journal holds",
    ),
    note(
        ErrorCode::Timeout,
        "a time limit was reached",
        "raise timeout or reduce the work",
    ),
    note(
        ErrorCode::Cancelled,
        "the execution was cancelled",
        "repeat the call when needed",
    ),
    note(
        ErrorCode::Limit,
        "a resource limit was reached",
        "narrow the targets or raise the limit",
    ),
    note(
        ErrorCode::StageFailed,
        "a stage failed",
        "read the cause in the error and the partial result, if any",
    ),
    note(
        ErrorCode::RolledBack,
        "the verification failed and the effect was rolled back",
        "inspect the verification result and correct the change",
    ),
    note(
        ErrorCode::RollbackFailed,
        "a rollback could not restore everything; the files are listed with instructions",
        "follow the instructions in the error",
    ),
    note(
        ErrorCode::JournalNoop,
        "no such operation in the journal, or nothing to undo or redo",
        "check the identifier with history",
    ),
    note(
        ErrorCode::JournalExpired,
        "an entry of the journal has expired; nothing changed",
        "restore the content from another source",
    ),
    note(
        ErrorCode::JournalConflict,
        "the current state differs from the state after the operation; nothing changed",
        "choose an on_conflict value from the listed choices",
    ),
    note(
        ErrorCode::JournalPartial,
        "the entry is partial: some items were released",
        "undo selected files with paths: or give on_conflict: partial",
    ),
    note(
        ErrorCode::JournalCorrupt,
        "an entry of the journal failed its integrity check",
        "keep the data and report the entry",
    ),
    note(
        ErrorCode::UnknownJob,
        "the background job is unknown",
        "list the jobs with jobs",
    ),
    note(
        ErrorCode::JobState,
        "the action is not allowed in the current state of the job",
        "check the state with jobs",
    ),
    note(
        ErrorCode::TestNoFramework,
        "the test framework of the project was not recognised",
        "give framework: or command:",
    ),
    note(
        ErrorCode::UnknownRun,
        "the test run is unknown",
        "list the runs with test.list",
    ),
    note(
        ErrorCode::UnknownAnalysis,
        "the analysis is unknown",
        "run analyze.check first",
    ),
    note(
        ErrorCode::OutputParse,
        "the output of a test framework or an analyser could not be read",
        "use the raw execution result included in the error",
    ),
    note(
        ErrorCode::Network,
        "a network error occurred: name resolution, connection, TLS, proxy, redirects or protocol",
        "check the address and the connection; set retries",
    ),
    note(
        ErrorCode::HashMismatch,
        "the downloaded file does not have the expected hash; it was not kept",
        "check the source and the expected hash",
    ),
    note(
        ErrorCode::Service,
        "the service manager reported an error",
        "read the message of the service manager in the error",
    ),
    note(
        ErrorCode::ServiceNotRunning,
        "the background service of the instance is not running",
        "start nurt service",
    ),
    note(
        ErrorCode::Catalog,
        "a program catalog configured explicitly cannot be read",
        "check the path in the configuration",
    ),
    note(
        ErrorCode::GitNotRepository,
        "the path is not inside a repository",
        "choose a path from candidates",
    ),
    note(
        ErrorCode::GitRevision,
        "the revision is unknown, ambiguous or malformed",
        "check the name with git.log or git.branch",
    ),
    note(
        ErrorCode::GitRef,
        "the reference name is invalid, exists, is missing, is not merged or is checked out elsewhere",
        "read problem in the error and choose another name",
    ),
    note(
        ErrorCode::GitDirty,
        "local changes would be overwritten; nothing changed",
        "commit or checkpoint the changes first",
    ),
    note(
        ErrorCode::GitConflict,
        "a merge ended with conflicts; the effect is applied and the conflicts are listed",
        "resolve the conflicts and continue",
    ),
    note(
        ErrorCode::GitInProgress,
        "a merge or rebase is in progress, or none is when one is required",
        "continue or abort the operation in progress",
    ),
    note(
        ErrorCode::GitLocked,
        "a lock file of the repository exists",
        "wait for the other operation or remove the stale lock",
    ),
    note(
        ErrorCode::GitNonFastForward,
        "the histories diverged where only a fast-forward is allowed",
        "fetch and merge or rebase first",
    ),
    note(
        ErrorCode::GitNothingToCommit,
        "there is nothing to commit",
        "stage changes or give paths:",
    ),
    note(
        ErrorCode::GitHook,
        "a hook ended with a non-zero exit code; no commit was created",
        "fix the problem the hook reports",
    ),
    note(
        ErrorCode::GitSign,
        "a commit or a tag could not be signed",
        "check the signing configuration",
    ),
    note(
        ErrorCode::GitAuth,
        "authentication is required or was rejected by the remote",
        "configure credentials for the remote",
    ),
    note(
        ErrorCode::GitHostKey,
        "the host key of the remote is unknown or changed",
        "verify the host key outside the console",
    ),
    note(
        ErrorCode::GitPushRejected,
        "the remote rejected an update of at least one reference",
        "fetch and integrate the remote changes",
    ),
    note(
        ErrorCode::GitProtocol,
        "the remote broke the protocol or lacks a required capability",
        "check the remote",
    ),
    note(
        ErrorCode::GitUnsupported,
        "a feature of the repository is not supported",
        "use a repository without the feature",
    ),
    note(
        ErrorCode::GitHunk,
        "a hunk identifier is malformed or unknown, or the hunk changed since it was listed",
        "list the hunks again",
    ),
    note(
        ErrorCode::GitCheckpoint,
        "a checkpoint has no saved index, or its content does not match after a restore",
        "list the checkpoints with git.checkpoint",
    ),
    note(
        ErrorCode::GitWorktree,
        "the worktree is unknown, is the main worktree, is locked or contains the working directory",
        "list the worktrees with git.worktree",
    ),
    note(
        ErrorCode::GitConfig,
        "the configuration of the repository is not valid: a value cannot be parsed or a required key is missing",
        "correct the value or set the missing key in the repository configuration",
    ),
    note(
        ErrorCode::Internal,
        "an internal error; the report is complete up to this point",
        "report the case",
    ),
];

/// Note of an error code.
#[must_use]
pub fn error_note(code: ErrorCode) -> Option<&'static ErrorNote> {
    NOTES.iter().find(|note| note.code == code)
}

/// Topic of `help` for an error that needs no command name.
fn fixed_topic(code: ErrorCode) -> Option<&'static str> {
    match code {
        ErrorCode::Ambiguous
        | ErrorCode::Uncorrectable
        | ErrorCode::InternalCorrection
        | ErrorCode::ExpiredCandidate => Some("corrections"),
        ErrorCode::Lexical
        | ErrorCode::Escape
        | ErrorCode::Syntax
        | ErrorCode::OneEffect
        | ErrorCode::PathRelative
        | ErrorCode::PathForeign => Some("syntax"),
        ErrorCode::Assert => Some("assert"),
        ErrorCode::UnknownCommand => Some("commands"),
        _ => None,
    }
}

/// Codes whose reference is the help of the command that raised them.
const COMMAND_CODES: &[ErrorCode] = &[
    ErrorCode::Type,
    ErrorCode::UnknownParam,
    ErrorCode::UnknownValue,
    ErrorCode::UnknownField,
    ErrorCode::MissingParam,
    ErrorCode::ParamConflict,
    ErrorCode::Arity,
    ErrorCode::InvalidValue,
];

/// Name of the detail of an error that holds the command.
const COMMAND_DETAIL: &str = "command";

/// Prefix that some errors write before the topic, as in `help syntax`.
const HELP_PREFIX: &str = "help ";

/// The line `help: help "<topic>"` that points an error at the part of the
/// help that applies.
///
/// The topic is always written as a quoted text, because the parameter
/// `topic` of `help` is a text and a bare word is rejected with `E_TYPE`.
///
/// An explicit help topic of the error wins. Syntax and correction errors
/// point at `syntax` and `corrections`, `E_ASSERT` at `assert`, an unknown
/// command at `commands`, and the errors of a command (`E_TYPE`,
/// `E_MISSING_PARAM`, …) at the command named in the detail `command`. Every
/// other code points at its own entry.
///
/// # Examples
///
/// ```
/// use nurt_contracts::error::{Error, ErrorCode};
/// use nurt_help::errors::help_reference;
///
/// let error = Error::new(ErrorCode::Assert, "expected 12 paths, found 14");
/// assert_eq!(help_reference(&error), "help: help \"assert\"");
/// ```
#[must_use]
pub fn help_reference(error: &Error) -> String {
    let code = error.code();
    let topic = if let Some(topic) = error.help.as_deref() {
        topic.strip_prefix(HELP_PREFIX).unwrap_or(topic).to_owned()
    } else if COMMAND_CODES.contains(&code) {
        error
            .details
            .get(COMMAND_DETAIL)
            .and_then(serde_json::Value::as_str)
            .map_or_else(|| code.as_str().to_owned(), str::to_owned)
    } else {
        fixed_topic(code).map_or_else(|| code.as_str().to_owned(), str::to_owned)
    };
    if topic.is_empty() {
        "help: help".to_owned()
    } else {
        let escaped = topic.replace('\\', "\\\\").replace('"', "\\\"");
        format!("help: help \"{escaped}\"")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_error_code_has_a_note() {
        for code in ErrorCode::ALL {
            assert!(error_note(*code).is_some(), "{code} has no note");
        }
        assert_eq!(NOTES.len(), ErrorCode::ALL.len());
    }

    #[test]
    fn notes_are_written_in_the_order_of_the_codes() {
        let codes: Vec<ErrorCode> = NOTES.iter().map(|note| note.code).collect();
        assert_eq!(codes, ErrorCode::ALL);
    }

    #[test]
    fn assertion_points_at_assert() {
        let error = Error::new(ErrorCode::Assert, "failed");
        assert_eq!(help_reference(&error), "help: help \"assert\"");
    }

    #[test]
    fn type_error_points_at_the_command_of_the_error() {
        let error = Error::new(ErrorCode::Type, "wrong input")
            .with_detail("command", serde_json::Value::from("remove"));
        assert_eq!(help_reference(&error), "help: help \"remove\"");
    }

    #[test]
    fn type_error_without_a_command_points_at_its_own_entry() {
        let error = Error::new(ErrorCode::Type, "wrong input");
        assert_eq!(help_reference(&error), "help: help \"E_TYPE\"");
    }

    #[test]
    fn ambiguity_points_at_corrections() {
        let error = Error::new(ErrorCode::Ambiguous, "two readings");
        assert_eq!(help_reference(&error), "help: help \"corrections\"");
    }

    #[test]
    fn explicit_topic_of_the_error_wins() {
        let error = Error::new(ErrorCode::Syntax, "bad").with_help("syntax");
        assert_eq!(help_reference(&error), "help: help \"syntax\"");
        let error = Error::new(ErrorCode::Stale, "changed").with_help("undo");
        assert_eq!(help_reference(&error), "help: help \"undo\"");
    }

    #[test]
    fn unknown_command_points_at_the_list() {
        let error = Error::new(ErrorCode::UnknownCommand, "no such command");
        assert_eq!(help_reference(&error), "help: help \"commands\"");
    }

    #[test]
    fn other_codes_point_at_their_own_entry() {
        let error = Error::new(ErrorCode::Stale, "changed");
        assert_eq!(help_reference(&error), "help: help \"E_STALE\"");
    }

    #[test]
    fn topic_written_as_a_call_loses_its_prefix() {
        let error = Error::new(ErrorCode::Syntax, "bad").with_help("help syntax");
        assert_eq!(help_reference(&error), "help: help \"syntax\"");
    }

    #[test]
    fn topic_is_a_quoted_text_that_a_program_can_run() {
        let error = Error::new(ErrorCode::Type, "wrong").with_help("help");
        assert_eq!(help_reference(&error), "help: help \"help\"");
        let error = Error::new(ErrorCode::Type, "wrong").with_help("a\"b\\c");
        assert_eq!(help_reference(&error), "help: help \"a\\\"b\\\\c\"");
        let error = Error::new(ErrorCode::Type, "wrong").with_help("");
        assert_eq!(help_reference(&error), "help: help");
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
