// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-git::review
// Responsibility:   `git.review`: checks changes before a commit and reports `finding`.
// Dependencies:     collect, nurt_contracts, rules, subject
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! `git.review`: checks changes before a commit and reports `finding`
//! records.
//!
//! The reviewed changes are those of a [`Scope`] (the index against `HEAD`,
//! the working tree against the index, or the working tree against `HEAD`)
//! or of a revision range given by `base` and `target`. Untracked files that
//! are not ignored belong to the working tree scopes. Each [`Check`] looks at
//! the added lines or at whole files:
//!
//! | Check | Finding |
//! |---|---|
//! | `secrets` | an added line matches a rule of the review rule service |
//! | `conflict_markers` | an added line starts with `<<<<<<< ` or `>>>>>>> ` |
//! | `whitespace` | trailing white space (not in `.md` files), a space before a tab in the indentation, a new blank line at the end of the file |
//! | `debug_statements` | a debugging call in a source file |
//! | `final_newline` | the change leaves the file without a final line feed |
//! | `large_files` | the new content is larger than [`LARGE_FILE_BYTES`] |
//! | `binary` | the new content is binary |
//! | `tests` | a source file changed and no test of it changed |
//! | `subject` | the subject line of `message` breaks a rule |
//!
//! Findings that block a commit (severity `error`) come first, then the
//! warnings; within a severity the checks keep the order of the table and
//! the files their path order. Findings of the message have the target
//! `message` and the repository root as path. Line findings carry the hunk
//! identifier (`path#number:fingerprint`) when the reviewed difference is the
//! canonical difference that `git.stage` and `git.restore` accept.
//!
//! # Examples
//!
//! ```no_run
//! use nurt_git::review::{ReviewRequest, Scope, run_review};
//!
//! fn blocking(ctx: &nurt_contracts::stage::ExecCtx) -> nurt_contracts::error::Result<usize> {
//!     let mut request = ReviewRequest::new("/home/user/project");
//!     request.scope = Scope::Staged;
//!     request.message = Some("feat(parser): add numbers".to_owned());
//!     request.conventional = true;
//!     let output = run_review(&request, ctx)?;
//!     Ok(output
//!         .findings
//!         .iter()
//!         .filter(|finding| finding.severity == nurt_contracts::record::FindingSeverity::Error)
//!         .count())
//! }
//! ```

mod collect;
mod rules;
mod subject;
#[cfg(test)]
mod tests;

use std::collections::BTreeSet;
use std::path::PathBuf;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::record::{Finding, FindingKind, FindingSeverity};
use nurt_contracts::report::NotableEntry;
use nurt_contracts::stage::{
    BoundArgs, CheckCtx, ExecCtx, InputKinds, ReadStage, RecordStream, StageDescriptor, StageImpl,
    StageInput, StageOutput, StaticInfo,
};
use nurt_contracts::value::Record;

pub use collect::{Scope, UNTRACKED_MAX};
pub use rules::{
    CODE_EXTENSIONS, DEBUG_PATTERN, GENERIC_STEMS, LARGE_FILE_BYTES, TEST_DIRECTORIES,
};
pub use subject::{MessageFinding, SUBJECT_MAX, SUBJECT_MAX_MIN, check_message};

use crate::commit::args;
use crate::diff::open_repository;

/// Name of the command in the catalog.
pub const COMMAND: &str = "git.review";

/// A check of `git.review`, declared in report order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Check {
    /// Added lines matching a rule of the review rule service.
    Secrets,
    /// Leftover conflict markers.
    ConflictMarkers,
    /// White space errors.
    Whitespace,
    /// Debugging calls in source files.
    DebugStatements,
    /// A file left without a final line feed.
    FinalNewline,
    /// Files above [`LARGE_FILE_BYTES`].
    LargeFiles,
    /// Binary files.
    Binary,
    /// Source files changed without a change of their tests.
    Tests,
    /// Rules of the subject line of the commit message.
    Subject,
}

impl Check {
    /// Every check, in report order.
    pub const ALL: [Self; 9] = [
        Self::Secrets,
        Self::ConflictMarkers,
        Self::Whitespace,
        Self::DebugStatements,
        Self::FinalNewline,
        Self::LargeFiles,
        Self::Binary,
        Self::Tests,
        Self::Subject,
    ];

    /// The checks of the alias `style`.
    pub const STYLE: [Self; 4] = [
        Self::ConflictMarkers,
        Self::Whitespace,
        Self::DebugStatements,
        Self::FinalNewline,
    ];

    /// Name in the catalog.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Secrets => "secrets",
            Self::ConflictMarkers => "conflict_markers",
            Self::Whitespace => "whitespace",
            Self::DebugStatements => "debug_statements",
            Self::FinalNewline => "final_newline",
            Self::LargeFiles => "large_files",
            Self::Binary => "binary",
            Self::Tests => "tests",
            Self::Subject => "subject",
        }
    }

    /// Kind of the findings of the check.
    #[must_use]
    pub fn kind(self) -> FindingKind {
        match self {
            Self::Secrets => FindingKind::Secrets,
            Self::ConflictMarkers => FindingKind::ConflictMarkers,
            Self::Whitespace | Self::FinalNewline => FindingKind::Whitespace,
            Self::LargeFiles => FindingKind::LargeFiles,
            Self::Binary => FindingKind::Binary,
            Self::Subject => FindingKind::Subject,
            Self::DebugStatements | Self::Tests => FindingKind::Other,
        }
    }

    /// The checks a name selects: a catalog name, `style` for
    /// [`Check::STYLE`] or `message` for [`Check::Subject`].
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_git::review::Check;
    ///
    /// assert_eq!(Check::named("binary"), Some(vec![Check::Binary]));
    /// assert_eq!(Check::named("message"), Some(vec![Check::Subject]));
    /// assert_eq!(Check::named("style").map(|checks| checks.len()), Some(4));
    /// assert_eq!(Check::named("lint"), None);
    /// ```
    #[must_use]
    pub fn named(name: &str) -> Option<Vec<Self>> {
        match name {
            "style" => Some(Self::STYLE.to_vec()),
            "message" => Some(vec![Self::Subject]),
            _ => Self::ALL
                .into_iter()
                .find(|check| check.name() == name)
                .map(|check| vec![check]),
        }
    }

    /// The check reads the changed files.
    fn reads_files(self) -> bool {
        self != Self::Subject
    }
}

/// The checks selected by `names`; every check when `names` is empty.
///
/// # Errors
///
/// `E_INVALID_VALUE` with problem `unknown_check` for a name that is neither
/// a check nor an alias.
pub fn parse_checks(names: &[String]) -> Result<BTreeSet<Check>> {
    if names.is_empty() {
        return Ok(Check::ALL.into_iter().collect());
    }
    let mut checks = BTreeSet::new();
    for name in names {
        let selected = Check::named(name.trim()).ok_or_else(|| {
            let known: Vec<&str> = Check::ALL.iter().map(|check| check.name()).collect();
            Error::new(
                ErrorCode::InvalidValue,
                format!(
                    "`{name}` is not a check of `{COMMAND}`; use {}, `style` or `message`",
                    known
                        .iter()
                        .map(|name| format!("`{name}`"))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            )
            .with_problem("unknown_check")
            .with_found(name.clone())
        })?;
        checks.extend(selected);
    }
    Ok(checks)
}

/// Arguments of one review.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewRequest {
    /// Repository or a path inside it.
    pub repo: PathBuf,
    /// Changes reviewed when no `base` is given.
    pub scope: Scope,
    /// Only changes in these files or directories.
    pub paths: Vec<String>,
    /// Patterns of paths left out.
    pub exclude: Vec<String>,
    /// Base revision of a range reviewed instead of uncommitted changes.
    pub base: Option<String>,
    /// End revision of the range; `HEAD` when absent.
    pub target: Option<String>,
    /// Planned commit message.
    pub message: Option<String>,
    /// Checks to run.
    pub checks: BTreeSet<Check>,
    /// Longest allowed subject line, in characters.
    pub subject_max: usize,
    /// Require a Conventional Commits subject line.
    pub conventional: bool,
}

impl ReviewRequest {
    /// Request for `repo` with the defaults of the catalog: scope `all`,
    /// every check, subject lines up to [`SUBJECT_MAX`] characters.
    #[must_use]
    pub fn new(repo: impl Into<PathBuf>) -> Self {
        Self {
            repo: repo.into(),
            scope: Scope::All,
            paths: Vec::new(),
            exclude: Vec::new(),
            base: None,
            target: None,
            message: None,
            checks: Check::ALL.into_iter().collect(),
            subject_max: SUBJECT_MAX,
            conventional: false,
        }
    }

    /// Builds the request from bound arguments.
    ///
    /// # Errors
    ///
    /// `E_MISSING_PARAM` without `repo`, `E_TYPE` for an argument of another
    /// type, `E_INVALID_VALUE` for an unknown scope or check and for a
    /// negative `subject_max`.
    pub fn from_args(args: &BoundArgs) -> Result<Self> {
        let mut request = Self::new(args::repo(args)?);
        if let Some(name) = args::text(args, "scope")? {
            request.scope = Scope::from_name(&name).ok_or_else(|| {
                Error::new(
                    ErrorCode::InvalidValue,
                    format!("`{name}` is not a scope; use `all`, `staged` or `working`"),
                )
                .with_problem("unknown_scope")
                .with_found(name.clone())
            })?;
        }
        request.paths = args::texts(args, "paths")?;
        request.exclude = args::texts(args, "exclude")?;
        request.base = args::text(args, "base")?;
        request.target = args::text(args, "target")?;
        request.message = args::text(args, "message")?;
        request.checks = parse_checks(&args::texts(args, "checks")?)?;
        if let Some(limit) = args.int("subject_max")? {
            request.subject_max = usize::try_from(limit).map_err(|_| subject_max_error(limit))?;
        }
        request.conventional = args.bool("conventional")?.unwrap_or(false);
        Ok(request)
    }

    /// Checks the combination of arguments.
    ///
    /// # Errors
    ///
    /// `E_MISSING_PARAM` with problem `missing_base` for `target` without
    /// `base`, and `E_INVALID_VALUE` with problem `out_of_range` for
    /// `subject_max` below [`SUBJECT_MAX_MIN`].
    pub fn validate(&self) -> Result<()> {
        if self.target.is_some() && self.base.is_none() {
            return Err(Error::new(
                ErrorCode::MissingParam,
                "`target` ends a range and needs `base`; give the base revision as `base`",
            )
            .with_problem("missing_base"));
        }
        if self.subject_max < SUBJECT_MAX_MIN {
            return Err(subject_max_error(
                i64::try_from(self.subject_max).unwrap_or(i64::MAX),
            ));
        }
        Ok(())
    }
}

fn subject_max_error(value: i64) -> Error {
    Error::new(
        ErrorCode::InvalidValue,
        format!("`subject_max` {value} is too small; use at least {SUBJECT_MAX_MIN}"),
    )
    .with_problem("out_of_range")
    .with_found(value.to_string())
}

/// Findings and notes of a review.
#[derive(Debug, Clone, Default)]
pub struct ReviewOutput {
    /// Findings in report order.
    pub findings: Vec<Finding>,
    /// Unusual effects for the report: excluded and skipped files, checks
    /// without rules.
    pub notes: Vec<NotableEntry>,
}

/// Runs a review.
///
/// # Errors
///
/// * the errors of [`ReviewRequest::validate`];
/// * `E_GIT_NOT_REPOSITORY` when `repo` is not in a repository or a path is
///   outside it;
/// * `E_GIT_REVISION` for an unknown `base` or `target`;
/// * `E_GIT_UNSUPPORTED` for a scope of the working tree in a bare
///   repository;
/// * `E_CANCELLED` when the call is cancelled;
/// * `E_IO` or `E_STAGE_FAILED` when the repository cannot be read.
pub fn run_review(request: &ReviewRequest, ctx: &ExecCtx) -> Result<ReviewOutput> {
    request.validate()?;
    let opened = open_repository(&request.repo, ctx)?;
    let repo = opened.repo;
    let mut notes = opened.notes;
    let mut findings = Vec::new();
    if request.checks.iter().any(|check| check.reads_files()) {
        let collected = collect::collect(&repo, request, &mut notes)?;
        let secrets = if request.checks.contains(&Check::Secrets) {
            rules::secret_rules(ctx.services().review_rules(), &mut notes)
        } else {
            Vec::new()
        };
        let settings = rules::Settings {
            checks: &request.checks,
            secrets: &secrets,
            tracked: &collected.tracked,
        };
        findings.extend(rules::check_files(&collected.changes, &settings));
    }
    if request.checks.contains(&Check::Subject)
        && let Some(message) = &request.message
    {
        let root = repo.root().display().to_string();
        findings.extend(
            check_message(message, request.subject_max, request.conventional)
                .into_iter()
                .map(|finding| (Check::Subject, finding.into_finding(&root))),
        );
    }
    findings.sort_by_key(|(check, finding)| (severity_rank(finding.severity), *check));
    Ok(ReviewOutput {
        findings: findings.into_iter().map(|(_, finding)| finding).collect(),
        notes,
    })
}

fn severity_rank(severity: FindingSeverity) -> u8 {
    match severity {
        FindingSeverity::Error => 0,
        FindingSeverity::Warning => 1,
        _ => 2,
    }
}

/// The `git.review` command.
#[derive(Debug, Clone, Copy, Default)]
pub struct GitReview;

impl StageImpl for GitReview {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: COMMAND.to_owned(),
            source: true,
            input: InputKinds::None,
        }
    }

    fn check(&self, args: &BoundArgs, _ctx: &CheckCtx<'_>) -> Result<StaticInfo> {
        ReviewRequest::from_args(args)?.validate()?;
        Ok(StaticInfo::default())
    }

    fn as_read(&self) -> Option<&dyn ReadStage> {
        Some(self)
    }
}

impl ReadStage for GitReview {
    fn execute<'a>(
        &'a self,
        args: &'a BoundArgs,
        _input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<StageOutput<'a>> {
        let request = ReviewRequest::from_args(args)?;
        let output = run_review(&request, ctx)?;
        for note in output.notes {
            ctx.note(note);
        }
        let records: Vec<Record> = output.findings.into_iter().map(Record::from).collect();
        Ok(StageOutput::records(
            RecordStream::from_records(records).checked(ctx),
        ))
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
