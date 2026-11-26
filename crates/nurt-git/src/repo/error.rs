// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Conversion of repository failures into errors of the language.
//!
//! Every error built here carries a stable code, a short message that says
//! what to correct and, where a ready call exists, `fix` and `next_steps`
//! entries written as calls of the console.

use std::io;
use std::path::Path;

use nurt_contracts::error::{Candidate, Error, ErrorCode};
use serde_json::{Value, json};

/// Largest number of candidates listed by an error.
pub const MAX_CANDIDATES: usize = 8;

/// Writes `text` as a text literal of the language: double quotes, with `\`
/// and `"` escaped and control characters written as escapes.
///
/// # Examples
///
/// ```
/// use nurt_git::repo::error::quote;
///
/// assert_eq!(quote(r#"a "b" \c"#), r#""a \"b\" \\c""#);
/// ```
#[must_use]
pub fn quote(text: &str) -> String {
    let mut quoted = String::with_capacity(text.len() + 2);
    quoted.push('"');
    for ch in text.chars() {
        match ch {
            '"' => quoted.push_str("\\\""),
            '\\' => quoted.push_str("\\\\"),
            '\n' => quoted.push_str("\\n"),
            '\r' => quoted.push_str("\\r"),
            '\t' => quoted.push_str("\\t"),
            ch if ch.is_control() => quoted.push_str(&format!("\\u{{{:x}}}", u32::from(ch))),
            ch => quoted.push(ch),
        }
    }
    quoted.push('"');
    quoted
}

/// Writes a path as a text literal with `/` separators, the form paths take
/// in programs on every system. A backslash is a separator on Windows only;
/// in a name on another system it stays.
#[must_use]
pub fn quote_path(path: &Path) -> String {
    let text = path.to_string_lossy();
    if cfg!(windows) {
        quote(&text.replace('\\', "/"))
    } else {
        quote(&text)
    }
}

/// Name of the command used in ready calls when the running stage has no
/// command of the `git` group.
pub const DEFAULT_COMMAND: &str = "git.status";

/// The command `name` when it belongs to the `git` group, otherwise
/// [`DEFAULT_COMMAND`].
#[must_use]
pub fn git_command(name: &str) -> &str {
    if name.starts_with("git.") {
        name
    } else {
        DEFAULT_COMMAND
    }
}

/// `E_GIT_NOT_REPOSITORY`: `path` is not inside a repository.
///
/// `nearby` lists repositories found near the path; each becomes a ranked
/// candidate that repeats `command` with the repository as its first
/// argument.
#[must_use]
pub fn not_repository(path: &Path, nearby: &[std::path::PathBuf], command: &str) -> Error {
    let command = git_command(command);
    let mut error = Error::new(
        ErrorCode::GitNotRepository,
        format!(
            "not a git repository: {}; pass a path inside a repository",
            path.display()
        ),
    )
    .with_problem("not_repository")
    .with_detail("path", Value::from(path.to_string_lossy().into_owned()));
    for (rank, repo) in (1_u32..).zip(nearby.iter().take(MAX_CANDIDATES)) {
        error = error.with_candidate(Candidate {
            rank,
            reference: None,
            program: format!("{command} {}", quote_path(repo)),
            why: Some("repository near the given path".to_owned()),
        });
    }
    error
}

/// `E_GIT_NOT_REPOSITORY` with problem `path_not_found`: the given path does
/// not exist.
#[must_use]
pub fn path_not_found(path: &Path) -> Error {
    Error::new(
        ErrorCode::GitNotRepository,
        format!(
            "path does not exist: {}; pass an existing directory",
            path.display()
        ),
    )
    .with_problem("path_not_found")
    .with_detail("path", Value::from(path.to_string_lossy().into_owned()))
}

/// `E_GIT_NOT_REPOSITORY` with problem `unsafe_repository`: the repository
/// belongs to another account and `safe.directory` does not list it.
///
/// The fix adds the repository to `safe.directory` in the global
/// configuration file `global_config`.
#[must_use]
pub fn unsafe_repository(repo: &Path, global_config: Option<&Path>) -> Error {
    let mut error = Error::new(
        ErrorCode::GitNotRepository,
        format!(
            "repository {} belongs to another account; add it to safe.directory to trust it",
            repo.display()
        ),
    )
    .with_problem("unsafe_repository")
    .with_detail("path", Value::from(repo.to_string_lossy().into_owned()));
    if let Some(config) = global_config {
        error = error.with_fix(format!(
            "set {} key: \"safe.directory[+]\" value: {} format: ini",
            quote_path(config),
            quote_path(repo)
        ));
    }
    error
}

/// `E_GIT_UNSUPPORTED`: a feature of the repository is not supported;
/// `feature` names it, for example `extensions.refStorage=reftable`.
#[must_use]
pub fn unsupported(feature: &str, reason: &str) -> Error {
    Error::new(
        ErrorCode::GitUnsupported,
        format!("unsupported repository feature `{feature}`: {reason}"),
    )
    .with_problem("unsupported_feature")
    .with_found(feature)
}

/// `E_GIT_LOCKED`: the lock file `lock` exists, so another program is
/// changing the repository or a previous run ended without removing it.
#[must_use]
pub fn locked(lock: &Path) -> Error {
    Error::new(
        ErrorCode::GitLocked,
        format!(
            "lock file exists: {}; wait for the other git process or remove the stale lock",
            lock.display()
        ),
    )
    .with_problem("lock_exists")
    .with_detail("path", Value::from(lock.to_string_lossy().into_owned()))
    .with_fix(format!("remove {}", quote_path(lock)))
}

/// Problem of a revision that could not be resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RevisionProblem {
    /// No object or reference matches.
    NotFound,
    /// Several objects match an abbreviated identifier.
    Ambiguous,
    /// The text is not a valid revision.
    InvalidSyntax,
}

impl RevisionProblem {
    /// Name of the problem in the `problem` field.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NotFound => "not_found",
            Self::Ambiguous => "ambiguous",
            Self::InvalidSyntax => "invalid_syntax",
        }
    }
}

/// `E_GIT_REVISION`: `spec` cannot be resolved in the repository `repo`.
///
/// `candidates` are revisions to use instead, best first: nearby reference
/// names for an unknown name, longer identifiers for an ambiguous one. Each
/// becomes a ranked call of `git.log` that shows the candidate.
#[must_use]
pub fn revision(
    problem: RevisionProblem,
    spec: &str,
    repo: &Path,
    candidates: &[(String, String)],
    detail: &str,
) -> Error {
    let message = match problem {
        RevisionProblem::NotFound => {
            format!("unknown revision `{spec}`; use a branch, tag or commit identifier")
        }
        RevisionProblem::Ambiguous => {
            format!("ambiguous revision `{spec}`; use a longer identifier")
        }
        RevisionProblem::InvalidSyntax => {
            format!("invalid revision `{spec}`: {detail}")
        }
    };
    let mut error = Error::new(ErrorCode::GitRevision, message)
        .with_problem(problem.as_str())
        .with_found(spec);
    if !candidates.is_empty() {
        let names: Vec<Value> = candidates
            .iter()
            .take(MAX_CANDIDATES)
            .map(|(name, _)| Value::from(name.as_str()))
            .collect();
        error = error.with_expected(Value::Array(names));
    }
    for (rank, (name, why)) in (1_u32..).zip(candidates.iter().take(MAX_CANDIDATES)) {
        error = error.with_candidate(Candidate {
            rank,
            reference: None,
            program: format!(
                "git.log {} revision: {} count: 1",
                quote_path(repo),
                quote(name)
            ),
            why: (!why.is_empty()).then(|| why.clone()),
        });
    }
    error
}

/// Converts an I/O error on `path` into `E_NOT_FOUND`, `E_PERMISSION`,
/// `E_NO_SPACE` or `E_IO`.
#[must_use]
pub fn io(path: &Path, error: &io::Error) -> Error {
    let (code, problem) = io_code(error.kind());
    let mut converted = Error::new(code, format!("{}: {error}", path.display()))
        .with_problem(problem)
        .with_detail("path", Value::from(path.to_string_lossy().into_owned()));
    if let Some(os_error) = error.raw_os_error() {
        converted = converted.with_detail("os_error", json!(os_error));
    }
    converted
}

/// Converts an error of the repository library raised while doing `what`.
///
/// The classification of the library selects the code: missing items give
/// `E_NOT_FOUND`, denied access `E_PERMISSION`, cancellation `E_CANCELLED`,
/// unsupported features `E_GIT_UNSUPPORTED`; anything else, including
/// damaged repository data, is `E_IO`. I/O errors found in the cause chain
/// are converted with [`io`].
#[must_use]
pub fn from_gix(what: &str, error: &gix::Error) -> Error {
    if let Some(io_error) = error.downcast_any_ref::<io::Error>() {
        let (code, problem) = io_code(io_error.kind());
        return Error::new(code, format!("{what} failed: {io_error}"))
            .with_problem(problem)
            .with_detail("operation", Value::from(what));
    }
    let (code, problem) = if error.is_cancelled() {
        (ErrorCode::Cancelled, "cancelled")
    } else if error.is_permission_denied() {
        (ErrorCode::Permission, "permission_denied")
    } else if error.is_unsupported() {
        (ErrorCode::GitUnsupported, "unsupported_feature")
    } else if error.is_not_found() {
        (ErrorCode::NotFound, "not_found")
    } else if error.is_corrupted() {
        (ErrorCode::Io, "corrupt_repository")
    } else {
        (ErrorCode::Io, "repository_error")
    };
    Error::new(
        code,
        format!("{what} failed: {}", first_line(&error.to_string())),
    )
    .with_problem(problem)
    .with_detail("operation", Value::from(what))
}

fn io_code(kind: io::ErrorKind) -> (ErrorCode, &'static str) {
    match kind {
        io::ErrorKind::NotFound => (ErrorCode::NotFound, "not_found"),
        io::ErrorKind::PermissionDenied => (ErrorCode::Permission, "permission_denied"),
        io::ErrorKind::AlreadyExists => (ErrorCode::Exists, "exists"),
        io::ErrorKind::StorageFull => (ErrorCode::NoSpace, "no_space"),
        _ => (ErrorCode::Io, "io"),
    }
}

fn first_line(text: &str) -> &str {
    text.lines().next().unwrap_or(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quote_escapes_control_characters() {
        assert_eq!(quote("a\tb\n"), r#""a\tb\n""#);
        assert_eq!(quote("\u{1}"), r#""\u{1}""#);
    }

    #[cfg(windows)]
    #[test]
    fn quote_path_uses_forward_slashes() {
        assert_eq!(quote_path(Path::new(r"C:\repo\x")), r#""C:/repo/x""#);
    }

    #[cfg(not(windows))]
    #[test]
    fn quote_path_keeps_a_backslash_in_a_name() {
        assert_eq!(quote_path(Path::new(r"/r/a\b")), r#""/r/a\\b""#);
        assert_eq!(quote_path(Path::new("/r/a/b")), r#""/r/a/b""#);
    }

    #[test]
    fn not_repository_offers_nearby_repositories_as_calls() {
        let error = not_repository(
            Path::new("/work"),
            &["/work/app".into(), "/work/lib".into()],
            "git.log",
        );
        assert_eq!(error.code, ErrorCode::GitNotRepository);
        assert_eq!(error.candidates.len(), 2);
        assert_eq!(error.candidates[0].program, r#"git.log "/work/app""#);
        assert_eq!(error.candidates[1].rank, 2);
    }

    #[test]
    fn not_repository_uses_status_outside_the_git_group() {
        let error = not_repository(Path::new("/w"), &["/w/a".into()], "find");
        assert_eq!(error.candidates[0].program, r#"git.status "/w/a""#);
    }

    #[test]
    fn locked_offers_explicit_removal() {
        let error = locked(Path::new("/r/.git/index.lock"));
        assert_eq!(error.code, ErrorCode::GitLocked);
        assert_eq!(error.fix, vec![r#"remove "/r/.git/index.lock""#.to_owned()]);
    }

    #[test]
    fn revision_lists_candidates_as_log_calls() {
        let error = revision(
            RevisionProblem::NotFound,
            "mian",
            Path::new("/r"),
            &[("main".to_owned(), "branch".to_owned())],
            "",
        );
        assert_eq!(error.problem.as_deref(), Some("not_found"));
        assert_eq!(
            error.candidates[0].program,
            r#"git.log "/r" revision: "main" count: 1"#
        );
        assert_eq!(error.expected, Some(json!(["main"])));
    }

    #[test]
    fn io_maps_error_kinds_to_codes() {
        let missing = io(Path::new("/x"), &io::Error::from(io::ErrorKind::NotFound));
        assert_eq!(missing.code, ErrorCode::NotFound);
        let denied = io(
            Path::new("/x"),
            &io::Error::from(io::ErrorKind::PermissionDenied),
        );
        assert_eq!(denied.code, ErrorCode::Permission);
        let other = io(Path::new("/x"), &io::Error::other("broken"));
        assert_eq!(other.code, ErrorCode::Io);
    }

    #[test]
    fn unsafe_repository_offers_safe_directory_entry() {
        let error = unsafe_repository(
            Path::new("/srv/r"),
            Some(Path::new("/home/user/.gitconfig")),
        );
        assert_eq!(error.problem.as_deref(), Some("unsafe_repository"));
        assert_eq!(
            error.fix[0],
            r#"set "/home/user/.gitconfig" key: "safe.directory[+]" value: "/srv/r" format: ini"#
        );
    }
}
