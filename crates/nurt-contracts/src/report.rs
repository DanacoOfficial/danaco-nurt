// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

//! Effect report `nurt-report/1`.
//!
//! Every program returns one [`Report`]. The report describes the **effect**
//! of the program, not the fact that it ran: the targets, the changes with
//! sizes and hashes before and after, a sample of differences, a summary for
//! bulk operations, unusual effects ([`Notable`]) and irreversible effects
//! ([`Irreversible`]).
//!
//! The report never says whether, until when or how an operation can be
//! undone. It carries the operation number ([`Report::op`]) and nothing else
//! about the journal: no journal state, no expiry, no countdown. The field
//! [`Report::irreversible`] is present only when an effect is irreversible;
//! its absence carries no information about undo. The state of journal
//! entries is shown only by an explicit `history` query.
//!
//! The JSON form is described by the published schema
//! [`crate::schema::REPORT`]. Within major version 1 the format only gains
//! optional fields and new values of the enumerations marked as
//! non-exhaustive; readers ignore fields they do not know.
//!
//! # Examples
//!
//! ```
//! use nurt_contracts::report::{Notable, NotableEntry, Report, ReportStatus, Severity};
//!
//! let mut report = Report::new(ReportStatus::Ok, "replace");
//! report.op = Some(18);
//! report.notable.push(NotableEntry::new(Notable::ManyPerFile));
//! assert_eq!(report.notable[0].severity, Severity::Notice);
//! assert!(report.validate().is_ok());
//! ```

use std::fmt;
use std::str::FromStr;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::ast::AST_HASH_PREFIX;
use crate::error::Error;
use crate::value::{Bytes, Record, RecordKind};

pub use crate::record::{Change, ChangeAction, Hunk};

/// Note of a change whose target is a directory. The report reads it for the
/// kind of an entry that the journal did not describe, as in a preview or in
/// the restoring of an undo.
pub const FLAG_DIRECTORY: &str = "directory";
/// Prefix of the note with the number of files held by a directory.
const FILES_INSIDE_PREFIX: &str = "files_inside=";
/// Prefix of the note with the number of directories held by a directory.
const DIRECTORIES_INSIDE_PREFIX: &str = "directories_inside=";

/// Notes of a change of a directory with its content: the kind of the
/// target and the number of files (links included) and of directories
/// below it.
#[must_use]
pub fn tree_flags(files: u64, directories: u64) -> Vec<String> {
    vec![
        FLAG_DIRECTORY.to_owned(),
        format!("{FILES_INSIDE_PREFIX}{files}"),
        format!("{DIRECTORIES_INSIDE_PREFIX}{directories}"),
    ]
}

/// Number of files and of directories below a directory, from the notes
/// written by [`tree_flags`]; `None` when the notes carry no counts.
#[must_use]
pub fn tree_counts(flags: &[String]) -> Option<(u64, u64)> {
    let number = |prefix: &str| {
        flags
            .iter()
            .find_map(|flag| flag.strip_prefix(prefix))
            .and_then(|text| text.parse::<u64>().ok())
    };
    Some((
        number(FILES_INSIDE_PREFIX)?,
        number(DIRECTORIES_INSIDE_PREFIX)?,
    ))
}

#[cfg(test)]
mod tree_note_tests {
    use super::*;

    #[test]
    fn counts_survive_a_round_trip_through_the_notes() {
        let mut flags = vec!["recursive".to_owned()];
        flags.extend(tree_flags(4, 2));
        assert_eq!(tree_counts(&flags), Some((4, 2)));
        assert!(flags.iter().any(|flag| flag == FLAG_DIRECTORY));
    }

    #[test]
    fn notes_without_counts_or_with_damaged_counts_give_none() {
        assert_eq!(tree_counts(&[]), None);
        assert_eq!(tree_counts(&["recursive".to_owned()]), None);
        let damaged = vec![
            "files_inside=x".to_owned(),
            "directories_inside=1".to_owned(),
        ];
        assert_eq!(tree_counts(&damaged), None);
        assert_eq!(tree_counts(&["files_inside=3".to_owned()]), None);
    }

    #[test]
    fn zero_counts_are_kept() {
        assert_eq!(tree_counts(&tree_flags(0, 0)), Some((0, 0)));
    }
}

/// Number of hexadecimal digits of a SHA-256 digest.
const SHA256_HEX_DIGITS: usize = 64;

/// Name of the report format.
pub const REPORT_FORMAT: &str = "nurt-report/1";

/// Version written in the `nurt` field of every report produced by this
/// version of the contract.
pub const REPORT_VERSION: &str = "1.0";

/// Stable identifier (`$id`) of the published report schema.
pub const REPORT_SCHEMA_ID: &str = "https://nurt.danaco-group.pl/schema/nurt-report-1.schema.json";

/// Error returned when a text does not name a known value of a code
/// enumeration.
///
/// # Examples
///
/// ```
/// use nurt_contracts::report::Notable;
///
/// let error = "no_such_code".parse::<Notable>().unwrap_err();
/// assert_eq!(error.code(), "no_such_code");
/// assert_eq!(error.enumeration(), "Notable");
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownCode {
    enumeration: &'static str,
    code: String,
}

impl UnknownCode {
    /// Name of the enumeration that was asked for the code.
    #[must_use]
    pub fn enumeration(&self) -> &'static str {
        self.enumeration
    }

    /// The code that was not recognized.
    #[must_use]
    pub fn code(&self) -> &str {
        &self.code
    }
}

impl fmt::Display for UnknownCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unknown {} code `{}`", self.enumeration, self.code)
    }
}

impl std::error::Error for UnknownCode {}

/// Finds the value of a code enumeration whose stable code is `code`.
fn parse_code<T: Copy>(
    all: &[T],
    as_str: fn(T) -> &'static str,
    enumeration: &'static str,
    code: &str,
) -> Result<T, UnknownCode> {
    all.iter()
        .copied()
        .find(|value| as_str(*value) == code)
        .ok_or_else(|| UnknownCode {
            enumeration,
            code: code.to_owned(),
        })
}

/// Outcome of a program as a whole.
///
/// | Status | Meaning |
/// |---|---|
/// | `ok` | the program finished; a changing program applied its effect |
/// | `planned` | preview: targets and effect computed, nothing changed |
/// | `running` | a program started in the background is still running; the report carries the job |
/// | `partial` | the effect was applied to part of the targets; the rest is listed in `irreversible` or `error` |
/// | `rolled_back` | the effect was applied and then reverted because a later check failed |
/// | `failed` | the program stopped with an error; see `error` |
///
/// # Examples
///
/// ```
/// use nurt_contracts::report::ReportStatus;
///
/// assert_eq!(ReportStatus::RolledBack.as_str(), "rolled_back");
/// assert_eq!("planned".parse::<ReportStatus>(), Ok(ReportStatus::Planned));
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ReportStatus {
    /// The program finished.
    Ok,
    /// Preview: the effect was computed and nothing was changed.
    Planned,
    /// A background program is still running.
    Running,
    /// The effect was applied to part of the targets only.
    Partial,
    /// The effect was applied and then reverted.
    RolledBack,
    /// The program stopped with an error.
    Failed,
}

impl ReportStatus {
    /// Every status, in declaration order.
    pub const ALL: &'static [Self] = &[
        Self::Ok,
        Self::Planned,
        Self::Running,
        Self::Partial,
        Self::RolledBack,
        Self::Failed,
    ];

    /// Returns the stable code of the status, as written in JSON.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Planned => "planned",
            Self::Running => "running",
            Self::Partial => "partial",
            Self::RolledBack => "rolled_back",
            Self::Failed => "failed",
        }
    }

    /// Whether a report with this status must carry an error.
    #[must_use]
    pub const fn requires_error(self) -> bool {
        matches!(self, Self::Failed | Self::RolledBack)
    }
}

impl fmt::Display for ReportStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ReportStatus {
    type Err = UnknownCode;

    fn from_str(code: &str) -> Result<Self, Self::Err> {
        parse_code(Self::ALL, Self::as_str, "ReportStatus", code)
    }
}

/// Importance of an unusual effect, in increasing order of importance.
///
/// # Examples
///
/// ```
/// use nurt_contracts::report::Severity;
///
/// assert!(Severity::Alert > Severity::Warning);
/// assert_eq!(Severity::Notice.as_str(), "notice");
/// ```
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// Informational; the effect is as requested, with a detail worth knowing.
    Info,
    /// Worth noticing.
    Notice,
    /// Differs from the usual result of the command and needs attention.
    Warning,
    /// Touches a sensitive location or a large amount of data.
    Alert,
}

impl Severity {
    /// Every severity, in increasing order of importance.
    pub const ALL: &'static [Self] = &[Self::Info, Self::Notice, Self::Warning, Self::Alert];

    /// Returns the stable code of the severity, as written in JSON.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Notice => "notice",
            Self::Warning => "warning",
            Self::Alert => "alert",
        }
    }
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Severity {
    type Err = UnknownCode;

    fn from_str(code: &str) -> Result<Self, Self::Err> {
        parse_code(Self::ALL, Self::as_str, "Severity", code)
    }
}

/// Code of an unusual effect reported in [`Report::notable`].
///
/// A notable effect is a description only; nothing is blocked because of
/// it. Thresholds are configurable under `report.notable.*`. Codes for which
/// [`Notable::computed_by_report`] is true are derived by the report builder
/// from the list of changes; all other codes are emitted by the command that
/// observed the condition.
///
/// # Examples
///
/// ```
/// use nurt_contracts::report::{Notable, Severity};
///
/// assert_eq!(Notable::MassDelete.as_str(), "mass_delete");
/// assert_eq!(Notable::MassDelete.default_severity(), Severity::Warning);
/// assert!(Notable::MassDelete.computed_by_report());
/// assert!(Notable::NothingMatched.computed_by_report());
/// assert!(!Notable::Unchanged.computed_by_report());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Notable {
    /// A target lies outside every configured project root.
    OutsideProject,
    /// A target lies in a system directory.
    SystemPath,
    /// A target is a configuration file or directory in the home directory.
    HomeConfig,
    /// Many files or bytes, or a large share of a directory, are deleted.
    MassDelete,
    /// A non-empty directory is deleted recursively.
    DeleteTree,
    /// Existing files were replaced by a write, move or copy.
    OverwroteExisting,
    /// A file that had content is empty after the change.
    FileEmptied,
    /// The size of a file shrank by at least the configured ratio.
    SizeShrunk,
    /// A single file received at least the configured number of replacements.
    ManyPerFile,
    /// The number of targets reached the configured threshold.
    TargetCountHigh,
    /// A changing program received no targets.
    NothingMatched,
    /// A write produced content identical to the existing content.
    Unchanged,
    /// A file with binary content was changed.
    BinaryTarget,
    /// Files with binary content were skipped; `count` gives their number.
    BinarySkipped,
    /// A special file (device, pipe, socket, pseudo file system) was written.
    SpecialFile,
    /// A change went through a symbolic link to a target outside its directory.
    SymlinkFollowed,
    /// A file with several hard links was replaced, separating the links.
    HardlinkBroken,
    /// A file with several hard links was written in place, keeping the links.
    HardlinkSetPreserved,
    /// Mode, ownership or access control of a file changed.
    AttributesChanged,
    /// A file was written in its original encoding other than UTF-8.
    EncodingPreserved,
    /// A file was written with its original line endings other than `lf`.
    LineEndingPreserved,
    /// A file has mixed line endings.
    LineEndingMixed,
    /// The encoding of a file was converted.
    EncodingChanged,
    /// The line endings of a file were converted.
    LineEndingChanged,
    /// A byte order mark was added or removed.
    BomChanged,
    /// Long lines were shortened in the output.
    TruncatedLines,
    /// A regular expression hit the backtracking limit on some input.
    RegexBacktracking,
    /// Compressed content was decompressed transparently.
    Decompressed,
    /// A move crossed volumes and was performed as copy and delete.
    CrossDevice,
    /// The destination of a move or copy is an existing directory.
    ToIsDir,
    /// Names differing only in letter case collide.
    CaseCollision,
    /// A name was matched in a letter case different from the one given.
    CaseAlias,
    /// A name was matched in a Unicode normalization form different from the one given.
    NfdName,
    /// A name is not portable between operating systems.
    Portability,
    /// A target lies on a network or removable volume.
    ExternalVolume,
    /// Archive entries pointing outside the destination were skipped.
    ArchivePathEscape,
    /// Files without read permission were skipped.
    PermissionDeniedSkipped,
    /// Changes in paths excluded from tracking were not reported.
    Ignored,
    /// A node referenced by aliases was changed; the change is visible through every alias.
    AliasShared,
    /// Intermediate keys were created.
    KeysCreated,
    /// A document has a repeated key; the last occurrence was changed.
    DuplicateKey,
    /// A code operation fell back to syntax analysis; `detail` gives the cause.
    SemanticFallback,
    /// Textual references to the old name remain after a change.
    ReferencesLeft,
    /// A rename did not reach every reference.
    RenameIncomplete,
    /// Changes made by a program are detected only inside the watched directories.
    DetectionScope,
    /// A program finished with an exit code other than zero.
    ExitNonzero,
    /// Stages after the effect were skipped because the program moved to the background.
    PostStagesDeferred,
    /// A rollback restored only part of the changes.
    RollbackPartial,
    /// A program wrote binary data to an output stream.
    OutputBinary,
    /// The output of a program exceeded the spool limit.
    SpoolLimit,
    /// A program closed its standard input before all input was written.
    StdinClosedEarly,
    /// A background job lost its supervisor.
    JobLost,
    /// A requested analysis tool is not installed.
    ToolMissing,
    /// No tests were found or run.
    TestsNone,
    /// Trust settings in a project configuration file were ignored; a project
    /// cannot make itself trusted.
    TrustConfigIgnored,
    /// Keys of a project configuration file outside the keys allowed for
    /// projects were ignored.
    ProjectConfigIgnored,
    /// Journal settings in an untrusted project configuration file were ignored.
    JournalConfigIgnored,
    /// The operation is larger than the journal size limit.
    JournalOversize,
    /// A data source of the operating system was unavailable; results are incomplete.
    SourceUnavailable,
    /// A settings change was broadcast to running programs.
    SettingsBroadcast,
    /// Repository hooks were not run.
    HooksSkipped,
    /// Repository history was rewritten.
    HistoryRewritten,
    /// A remote branch was overwritten by a forced push.
    ForcePush,
    /// The repository is in detached head state.
    DetachedHead,
    /// A branch that was not merged was deleted.
    BranchDeletedUnmerged,
    /// A branch was updated by fast-forward.
    FastForward,
    /// The operation stopped with conflicts; `count` gives their number.
    Conflicts,
    /// Untracked files were removed.
    UntrackedRemoved,
    /// Uncommitted changes were carried to another branch.
    LocalChangesCarried,
    /// A content filter of the repository was not applied.
    FilterNotApplied,
    /// A configured credential helper was not run.
    CredentialHelperIgnored,
    /// A configured SSH command was not run.
    SshCommandIgnored,
    /// A configured file system monitor was not run.
    FsmonitorIgnored,
    /// Submodules were not updated.
    SubmodulesNotUpdated,
    /// The repository uses sparse checkout.
    SparseCheckout,
    /// The repository has shallow history.
    ShallowRepository,
    /// Line endings were converted by repository settings.
    LineEndingsConverted,
    /// An upstream branch was set.
    UpstreamSet,
    /// A stash entry was kept after a conflict.
    StashKept,
    /// A commit or tag was signed.
    Signed,
    /// The amended commit is reachable from a remote-tracking branch.
    AmendPublished,
    /// A commit was created on a detached `HEAD`.
    CommitDetachedHead,
    /// A commit was created while a rebase, cherry-pick, revert or bisection
    /// is in progress.
    OperationInProgress,
    /// Files matched by `exclude` were skipped; `count` gives the number.
    Excluded,
    /// Files were not reviewed: untracked files over the size limit, binary
    /// files or files over the count limit.
    ReviewSkipped,
    /// A checkpoint was created without the index because the index has
    /// conflicts.
    CheckpointWithoutIndex,
    /// A symbolic link that leads back to one of its parent directories was
    /// not followed while walking a directory tree.
    LinkLoopSkipped,
    /// Verification of TLS certificates is disabled by the repository
    /// configuration.
    TlsVerificationDisabled,
    /// An HTTP setting of the repository configuration is not supported and
    /// was ignored.
    HttpOptionIgnored,
    /// A push sent objects without new delta compression.
    PackWithoutDeltas,
    /// The remote did not report the outcome of a reference update.
    PushStatusUnknown,
    /// The host key of a remote was accepted without verification.
    HostKeyUnverified,
    /// A `Match exec` condition of the SSH configuration was not run.
    MatchExecIgnored,
    /// A signing program named by the repository configuration was not run;
    /// the signature was created without it.
    SigningProgramIgnored,
    /// Credentials written in the URL of a remote were used.
    CredentialsInUrl,
    /// The output stopped at the number of entries given by `limit`; `count`
    /// gives the number of entries left out.
    LimitReached,
    /// A program classified as interactive was run: an editor, a pager, a
    /// terminal monitor or an interpreter without a script. It can wait for
    /// terminal input.
    InteractiveProgram,
    /// A program ran a script given in its arguments, for example `bash -c`
    /// or `python -c`; the effects of the script are not classified.
    OpaqueScript,
}

impl Notable {
    /// Every code, in declaration order.
    pub const ALL: &'static [Self] = &[
        Self::OutsideProject,
        Self::SystemPath,
        Self::HomeConfig,
        Self::MassDelete,
        Self::DeleteTree,
        Self::OverwroteExisting,
        Self::FileEmptied,
        Self::SizeShrunk,
        Self::ManyPerFile,
        Self::TargetCountHigh,
        Self::NothingMatched,
        Self::Unchanged,
        Self::BinaryTarget,
        Self::BinarySkipped,
        Self::SpecialFile,
        Self::SymlinkFollowed,
        Self::HardlinkBroken,
        Self::HardlinkSetPreserved,
        Self::AttributesChanged,
        Self::EncodingPreserved,
        Self::LineEndingPreserved,
        Self::LineEndingMixed,
        Self::EncodingChanged,
        Self::LineEndingChanged,
        Self::BomChanged,
        Self::TruncatedLines,
        Self::RegexBacktracking,
        Self::Decompressed,
        Self::CrossDevice,
        Self::ToIsDir,
        Self::CaseCollision,
        Self::CaseAlias,
        Self::NfdName,
        Self::Portability,
        Self::ExternalVolume,
        Self::ArchivePathEscape,
        Self::PermissionDeniedSkipped,
        Self::Ignored,
        Self::AliasShared,
        Self::KeysCreated,
        Self::DuplicateKey,
        Self::SemanticFallback,
        Self::ReferencesLeft,
        Self::RenameIncomplete,
        Self::DetectionScope,
        Self::ExitNonzero,
        Self::PostStagesDeferred,
        Self::RollbackPartial,
        Self::OutputBinary,
        Self::SpoolLimit,
        Self::StdinClosedEarly,
        Self::JobLost,
        Self::ToolMissing,
        Self::TestsNone,
        Self::TrustConfigIgnored,
        Self::ProjectConfigIgnored,
        Self::JournalConfigIgnored,
        Self::JournalOversize,
        Self::SourceUnavailable,
        Self::SettingsBroadcast,
        Self::HooksSkipped,
        Self::HistoryRewritten,
        Self::ForcePush,
        Self::DetachedHead,
        Self::BranchDeletedUnmerged,
        Self::FastForward,
        Self::Conflicts,
        Self::UntrackedRemoved,
        Self::LocalChangesCarried,
        Self::FilterNotApplied,
        Self::CredentialHelperIgnored,
        Self::SshCommandIgnored,
        Self::FsmonitorIgnored,
        Self::SubmodulesNotUpdated,
        Self::SparseCheckout,
        Self::ShallowRepository,
        Self::LineEndingsConverted,
        Self::UpstreamSet,
        Self::StashKept,
        Self::Signed,
        Self::AmendPublished,
        Self::CommitDetachedHead,
        Self::OperationInProgress,
        Self::Excluded,
        Self::ReviewSkipped,
        Self::CheckpointWithoutIndex,
        Self::LinkLoopSkipped,
        Self::TlsVerificationDisabled,
        Self::HttpOptionIgnored,
        Self::PackWithoutDeltas,
        Self::PushStatusUnknown,
        Self::HostKeyUnverified,
        Self::MatchExecIgnored,
        Self::SigningProgramIgnored,
        Self::CredentialsInUrl,
        Self::LimitReached,
        Self::InteractiveProgram,
        Self::OpaqueScript,
    ];

    /// Returns the stable code, as written in JSON.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OutsideProject => "outside_project",
            Self::SystemPath => "system_path",
            Self::HomeConfig => "home_config",
            Self::MassDelete => "mass_delete",
            Self::DeleteTree => "delete_tree",
            Self::OverwroteExisting => "overwrote_existing",
            Self::FileEmptied => "file_emptied",
            Self::SizeShrunk => "size_shrunk",
            Self::ManyPerFile => "many_per_file",
            Self::TargetCountHigh => "target_count_high",
            Self::NothingMatched => "nothing_matched",
            Self::Unchanged => "unchanged",
            Self::BinaryTarget => "binary_target",
            Self::BinarySkipped => "binary_skipped",
            Self::SpecialFile => "special_file",
            Self::SymlinkFollowed => "symlink_followed",
            Self::HardlinkBroken => "hardlink_broken",
            Self::HardlinkSetPreserved => "hardlink_set_preserved",
            Self::AttributesChanged => "attributes_changed",
            Self::EncodingPreserved => "encoding_preserved",
            Self::LineEndingPreserved => "line_ending_preserved",
            Self::LineEndingMixed => "line_ending_mixed",
            Self::EncodingChanged => "encoding_changed",
            Self::LineEndingChanged => "line_ending_changed",
            Self::BomChanged => "bom_changed",
            Self::TruncatedLines => "truncated_lines",
            Self::RegexBacktracking => "regex_backtracking",
            Self::Decompressed => "decompressed",
            Self::CrossDevice => "cross_device",
            Self::ToIsDir => "to_is_dir",
            Self::CaseCollision => "case_collision",
            Self::CaseAlias => "case_alias",
            Self::NfdName => "nfd_name",
            Self::Portability => "portability",
            Self::ExternalVolume => "external_volume",
            Self::ArchivePathEscape => "archive_path_escape",
            Self::PermissionDeniedSkipped => "permission_denied_skipped",
            Self::Ignored => "ignored",
            Self::AliasShared => "alias_shared",
            Self::KeysCreated => "keys_created",
            Self::DuplicateKey => "duplicate_key",
            Self::SemanticFallback => "semantic_fallback",
            Self::ReferencesLeft => "references_left",
            Self::RenameIncomplete => "rename_incomplete",
            Self::DetectionScope => "detection_scope",
            Self::ExitNonzero => "exit_nonzero",
            Self::PostStagesDeferred => "post_stages_deferred",
            Self::RollbackPartial => "rollback_partial",
            Self::OutputBinary => "output_binary",
            Self::SpoolLimit => "spool_limit",
            Self::StdinClosedEarly => "stdin_closed_early",
            Self::JobLost => "job_lost",
            Self::ToolMissing => "tool_missing",
            Self::TestsNone => "tests_none",
            Self::TrustConfigIgnored => "trust_config_ignored",
            Self::ProjectConfigIgnored => "project_config_ignored",
            Self::JournalConfigIgnored => "journal_config_ignored",
            Self::JournalOversize => "journal_oversize",
            Self::SourceUnavailable => "source_unavailable",
            Self::SettingsBroadcast => "settings_broadcast",
            Self::HooksSkipped => "hooks_skipped",
            Self::HistoryRewritten => "history_rewritten",
            Self::ForcePush => "force_push",
            Self::DetachedHead => "detached_head",
            Self::BranchDeletedUnmerged => "branch_deleted_unmerged",
            Self::FastForward => "fast_forward",
            Self::Conflicts => "conflicts",
            Self::UntrackedRemoved => "untracked_removed",
            Self::LocalChangesCarried => "local_changes_carried",
            Self::FilterNotApplied => "filter_not_applied",
            Self::CredentialHelperIgnored => "credential_helper_ignored",
            Self::SshCommandIgnored => "ssh_command_ignored",
            Self::FsmonitorIgnored => "fsmonitor_ignored",
            Self::SubmodulesNotUpdated => "submodules_not_updated",
            Self::SparseCheckout => "sparse_checkout",
            Self::ShallowRepository => "shallow_repository",
            Self::LineEndingsConverted => "line_endings_converted",
            Self::UpstreamSet => "upstream_set",
            Self::StashKept => "stash_kept",
            Self::Signed => "signed",
            Self::AmendPublished => "amend_published",
            Self::CommitDetachedHead => "commit_detached_head",
            Self::OperationInProgress => "operation_in_progress",
            Self::Excluded => "excluded",
            Self::ReviewSkipped => "review_skipped",
            Self::CheckpointWithoutIndex => "checkpoint_without_index",
            Self::LinkLoopSkipped => "link_loop_skipped",
            Self::TlsVerificationDisabled => "tls_verification_disabled",
            Self::HttpOptionIgnored => "http_option_ignored",
            Self::PackWithoutDeltas => "pack_without_deltas",
            Self::PushStatusUnknown => "push_status_unknown",
            Self::HostKeyUnverified => "host_key_unverified",
            Self::MatchExecIgnored => "match_exec_ignored",
            Self::SigningProgramIgnored => "signing_program_ignored",
            Self::CredentialsInUrl => "credentials_in_url",
            Self::LimitReached => "limit_reached",
            Self::InteractiveProgram => "interactive_program",
            Self::OpaqueScript => "opaque_script",
        }
    }

    /// Severity used when the condition holds at its default threshold.
    ///
    /// The report builder may raise the severity; for example `mass_delete`
    /// at ten times its threshold is an alert.
    #[must_use]
    pub const fn default_severity(self) -> Severity {
        match self {
            Self::OutsideProject | Self::SystemPath | Self::ForcePush | Self::AmendPublished => {
                Severity::Alert
            }
            Self::CommitDetachedHead | Self::OperationInProgress => Severity::Warning,
            Self::ReviewSkipped | Self::CheckpointWithoutIndex => Severity::Notice,
            Self::Excluded => Severity::Info,
            Self::HomeConfig
            | Self::MassDelete
            | Self::OverwroteExisting
            | Self::FileEmptied
            | Self::SizeShrunk
            | Self::BinaryTarget
            | Self::SpecialFile
            | Self::SymlinkFollowed
            | Self::RegexBacktracking
            | Self::ArchivePathEscape
            | Self::PermissionDeniedSkipped
            | Self::AliasShared
            | Self::DuplicateKey
            | Self::RenameIncomplete
            | Self::RollbackPartial
            | Self::SpoolLimit
            | Self::JobLost
            | Self::TrustConfigIgnored
            | Self::JournalOversize
            | Self::SourceUnavailable
            | Self::HistoryRewritten
            | Self::BranchDeletedUnmerged
            | Self::Conflicts
            | Self::UntrackedRemoved
            | Self::FilterNotApplied
            | Self::TlsVerificationDisabled
            | Self::PushStatusUnknown
            | Self::HostKeyUnverified
            | Self::CredentialsInUrl => Severity::Warning,
            Self::DeleteTree
            | Self::ManyPerFile
            | Self::TargetCountHigh
            | Self::NothingMatched
            | Self::BinarySkipped
            | Self::HardlinkBroken
            | Self::AttributesChanged
            | Self::LineEndingMixed
            | Self::EncodingChanged
            | Self::LineEndingChanged
            | Self::BomChanged
            | Self::TruncatedLines
            | Self::CrossDevice
            | Self::ToIsDir
            | Self::CaseCollision
            | Self::CaseAlias
            | Self::NfdName
            | Self::Portability
            | Self::ExternalVolume
            | Self::SemanticFallback
            | Self::ReferencesLeft
            | Self::ExitNonzero
            | Self::OutputBinary
            | Self::ToolMissing
            | Self::TestsNone
            | Self::ProjectConfigIgnored
            | Self::JournalConfigIgnored
            | Self::SettingsBroadcast
            | Self::HooksSkipped
            | Self::DetachedHead
            | Self::CredentialHelperIgnored
            | Self::SshCommandIgnored
            | Self::FsmonitorIgnored
            | Self::SubmodulesNotUpdated
            | Self::StashKept
            | Self::LinkLoopSkipped
            | Self::HttpOptionIgnored
            | Self::MatchExecIgnored
            | Self::SigningProgramIgnored
            | Self::InteractiveProgram
            | Self::OpaqueScript => Severity::Notice,
            Self::Unchanged
            | Self::HardlinkSetPreserved
            | Self::EncodingPreserved
            | Self::LineEndingPreserved
            | Self::Decompressed
            | Self::Ignored
            | Self::KeysCreated
            | Self::DetectionScope
            | Self::PostStagesDeferred
            | Self::StdinClosedEarly
            | Self::FastForward
            | Self::LocalChangesCarried
            | Self::SparseCheckout
            | Self::ShallowRepository
            | Self::LineEndingsConverted
            | Self::UpstreamSet
            | Self::Signed
            | Self::PackWithoutDeltas
            | Self::LimitReached => Severity::Info,
        }
    }

    /// Whether the code is derived by the report builder from the list of
    /// changes, the number of targets and the configured thresholds.
    ///
    /// All other codes are emitted by the command that observed the
    /// condition. An entry of a derived code emitted by a command is merged
    /// with the derived entry.
    #[must_use]
    pub const fn computed_by_report(self) -> bool {
        matches!(
            self,
            Self::OutsideProject
                | Self::SystemPath
                | Self::HomeConfig
                | Self::MassDelete
                | Self::DeleteTree
                | Self::OverwroteExisting
                | Self::FileEmptied
                | Self::SizeShrunk
                | Self::ManyPerFile
                | Self::TargetCountHigh
                | Self::NothingMatched
        )
    }
}

impl fmt::Display for Notable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Notable {
    type Err = UnknownCode;

    fn from_str(code: &str) -> Result<Self, Self::Err> {
        parse_code(Self::ALL, Self::as_str, "Notable", code)
    }
}

/// Reason why an effect is irreversible, reported in
/// [`Report::irreversible`].
///
/// # Examples
///
/// ```
/// use nurt_contracts::report::Irreversible;
///
/// assert_eq!(Irreversible::RemoteRefUpdated.as_str(), "remote_ref_updated");
/// assert_eq!(
///     "previous_content_not_kept".parse::<Irreversible>(),
///     Ok(Irreversible::PreviousContentNotKept)
/// );
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Irreversible {
    /// A program with an effect outside the local file system was run.
    ExternalEffect,
    /// References of a remote repository were updated or fetched.
    RemoteRefUpdated,
    /// A network request that changes remote state was sent.
    Network,
    /// A database was changed without a preserved copy.
    Database,
    /// A process or job was stopped.
    ProcessStopped,
    /// The state of a system service was changed.
    ServiceChanged,
    /// System packages were installed, removed or upgraded.
    PackageChanged,
    /// A program changed a file whose previous content was not preserved.
    PreviousContentNotKept,
    /// There was no space for a copy of the previous content.
    NoSpaceForCopy,
    /// The journal is disabled or its space service is not running.
    JournalInactive,
    /// The target lies on a network volume without a journal.
    NetworkVolume,
    /// The target lies on a removable volume without a journal.
    RemovableVolume,
    /// Journal entries were purged.
    Purged,
    /// A special file (device, pipe, socket, pseudo file system) was written.
    SpecialFile,
    /// The operation is larger than the limit for a single journal entry.
    TooLargeForJournal,
    /// Preserved content of part of a bulk operation was released to free disk space.
    ReleasedForSpace,
    /// The kept output of a background job was removed together with the job.
    JobOutputRemoved,
    /// The state of a running process was changed: it was suspended, resumed,
    /// signalled or given another priority.
    ProcessStateChanged,
    /// Journal entries of earlier operations expired because of this
    /// operation, for example when the journal limits were lowered.
    JournalEntriesExpired,
    /// The journal failed while recording the operation; the change was made
    /// without preserved previous content.
    JournalError,
}

impl Irreversible {
    /// Every reason, in declaration order.
    pub const ALL: &'static [Self] = &[
        Self::ExternalEffect,
        Self::RemoteRefUpdated,
        Self::Network,
        Self::Database,
        Self::ProcessStopped,
        Self::ServiceChanged,
        Self::PackageChanged,
        Self::PreviousContentNotKept,
        Self::NoSpaceForCopy,
        Self::JournalInactive,
        Self::NetworkVolume,
        Self::RemovableVolume,
        Self::Purged,
        Self::SpecialFile,
        Self::TooLargeForJournal,
        Self::ReleasedForSpace,
        Self::JobOutputRemoved,
        Self::ProcessStateChanged,
        Self::JournalEntriesExpired,
        Self::JournalError,
    ];

    /// Returns the stable code of the reason, as written in JSON.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ExternalEffect => "external_effect",
            Self::RemoteRefUpdated => "remote_ref_updated",
            Self::Network => "network",
            Self::Database => "database",
            Self::ProcessStopped => "process_stopped",
            Self::ServiceChanged => "service_changed",
            Self::PackageChanged => "package_changed",
            Self::PreviousContentNotKept => "previous_content_not_kept",
            Self::NoSpaceForCopy => "no_space_for_copy",
            Self::JournalInactive => "journal_inactive",
            Self::NetworkVolume => "network_volume",
            Self::RemovableVolume => "removable_volume",
            Self::Purged => "purged",
            Self::SpecialFile => "special_file",
            Self::TooLargeForJournal => "too_large_for_journal",
            Self::ReleasedForSpace => "released_for_space",
            Self::JobOutputRemoved => "job_output_removed",
            Self::ProcessStateChanged => "process_state_changed",
            Self::JournalEntriesExpired => "journal_entries_expired",
            Self::JournalError => "journal_error",
        }
    }
}

impl fmt::Display for Irreversible {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Irreversible {
    type Err = UnknownCode;

    fn from_str(code: &str) -> Result<Self, Self::Err> {
        parse_code(Self::ALL, Self::as_str, "Irreversible", code)
    }
}

/// Certainty class of a correction of the program text.
///
/// # Examples
///
/// ```
/// use nurt_contracts::report::CorrectionClass;
///
/// assert_eq!(CorrectionClass::Fuzzy.as_str(), "F");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub enum CorrectionClass {
    /// Certain equivalence: the text differs in notation only.
    #[serde(rename = "C")]
    Certain,
    /// Structural: a syntax character was added or removed where the
    /// structure is unambiguous.
    #[serde(rename = "S")]
    Structural,
    /// Fuzzy name: a name was matched by edit distance.
    #[serde(rename = "F")]
    Fuzzy,
    /// Probable: touches the bounds of a literal or resolves an ambiguity.
    #[serde(rename = "P")]
    Probable,
}

impl CorrectionClass {
    /// Every class, in declaration order.
    pub const ALL: &'static [Self] =
        &[Self::Certain, Self::Structural, Self::Fuzzy, Self::Probable];

    /// Returns the stable code of the class, as written in JSON.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Certain => "C",
            Self::Structural => "S",
            Self::Fuzzy => "F",
            Self::Probable => "P",
        }
    }
}

impl fmt::Display for CorrectionClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for CorrectionClass {
    type Err = UnknownCode;

    fn from_str(code: &str) -> Result<Self, Self::Err> {
        parse_code(Self::ALL, Self::as_str, "CorrectionClass", code)
    }
}

/// Position in the program text, counted from 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub struct TextPosition {
    /// Line number, from 1.
    pub line: u32,
    /// Column in Unicode scalar values, from 1.
    pub column: u32,
    /// Length of the corrected text in Unicode scalar values.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub length: Option<u32>,
}

/// A correction applied to the program text before it ran.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Correction {
    /// Identifier of the correction within the report, for example `C1`.
    pub id: String,
    /// Identifier of the correction rule: a letter of the rule table
    /// (`L`, `T`, `V`, `B`, `P`) and two digits, for example `L01`.
    pub rule: String,
    /// Certainty class of the correction.
    pub class: CorrectionClass,
    /// Position of the corrected text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<TextPosition>,
    /// Text as written.
    pub from: String,
    /// Text after the correction.
    pub to: String,
    /// Kind of corrected element, for example `command` or `param`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// Other candidates that were considered.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub alternatives: Vec<String>,
}

impl Correction {
    /// Whether `rule` has the form of a correction rule identifier.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_contracts::report::Correction;
    ///
    /// assert!(Correction::is_rule_id("L01"));
    /// assert!(!Correction::is_rule_id("X1"));
    /// ```
    #[must_use]
    pub fn is_rule_id(rule: &str) -> bool {
        matches!(
            rule.as_bytes(),
            [b'L' | b'T' | b'V' | b'B' | b'P', tens, units]
                if tens.is_ascii_digit() && units.is_ascii_digit()
        )
    }
}

/// Targets enumerated by the program.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
pub struct Targets {
    /// Number of targets.
    pub count: u64,
    /// Number of files among the targets.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub files: Option<u64>,
    /// Number of directories among the targets.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub directories: Option<u64>,
    /// Total size of the targets in bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes: Option<u64>,
    /// Number of entries in [`Report::changes`] on this page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub listed: Option<u64>,
}

/// Totals of a changing operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
pub struct Summary {
    /// Files created.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub files_created: Option<u64>,
    /// Files whose content changed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub files_modified: Option<u64>,
    /// Files deleted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub files_deleted: Option<u64>,
    /// Files renamed or moved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub files_renamed: Option<u64>,
    /// Directories created.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub directories_created: Option<u64>,
    /// Directories deleted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub directories_deleted: Option<u64>,
    /// Replacements made.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replacements: Option<u64>,
    /// Lines added.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lines_added: Option<u64>,
    /// Lines removed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lines_removed: Option<u64>,
    /// Total size of the changed targets before the change, in bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes_before: Option<u64>,
    /// Total size of the changed targets after the change, in bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes_after: Option<u64>,
    /// Bytes freed by deletion.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes_freed: Option<u64>,
}

/// Number of targets in one directory of a bulk operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DirectoryCount {
    /// Directory in canonical form.
    pub directory: String,
    /// Native bytes of the directory; present only when it is not valid
    /// Unicode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_directory: Option<Bytes>,
    /// Number of targets in the directory.
    pub count: u64,
    /// Total size of the targets in bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes: Option<u64>,
}

/// Number of targets with one file name extension in a bulk operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ExtensionCount {
    /// Extension with the leading dot, for example `.tmp`; empty for names
    /// without an extension.
    pub extension: String,
    /// Number of targets with the extension.
    pub count: u64,
    /// Total size of the targets in bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes: Option<u64>,
}

/// Summary of a bulk operation (at least `report.mass_threshold` targets).
///
/// The full list of targets stays available through [`Report::next_cursor`].
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
pub struct Bulk {
    /// Directories ordered by the number of targets, largest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub by_directory: Vec<DirectoryCount>,
    /// Extensions ordered by the number of targets, largest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub by_extension: Vec<ExtensionCount>,
    /// First targets in enumeration order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sample_head: Vec<String>,
    /// Native bytes of the targets of `sample_head`, item by item, `null` for
    /// a target that is valid Unicode; omitted when all of them are.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub raw_sample_head: Vec<Option<Bytes>>,
    /// Last targets in enumeration order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sample_tail: Vec<String>,
    /// Native bytes of the targets of `sample_tail`, item by item, `null` for
    /// a target that is valid Unicode; omitted when all of them are.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub raw_sample_tail: Vec<Option<Bytes>>,
}

/// Records returned by a reading program.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RecordsPage {
    /// Record kind of the rows, for example `entry` or `match`.
    #[serde(rename = "type")]
    pub record_type: RecordKind,
    /// Number of records produced by the program before paging.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matched: Option<u64>,
    /// Header record of a stream that has one, for example a test run before
    /// its test results.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub header: Option<Record>,
    /// Records on this page.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rows: Vec<Record>,
}

/// An unusual effect.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct NotableEntry {
    /// Code of the effect.
    pub code: Notable,
    /// Importance of the effect.
    pub severity: Severity,
    /// Target concerned, when the effect concerns one target.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Native bytes of `path`; present only when it is not valid Unicode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_path: Option<Bytes>,
    /// Number of targets or occurrences concerned.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count: Option<u64>,
    /// Short description of the effect.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl NotableEntry {
    /// Creates an entry with the default severity of the code.
    #[must_use]
    pub fn new(code: Notable) -> Self {
        Self {
            code,
            severity: code.default_severity(),
            path: None,
            raw_path: None,
            count: None,
            detail: None,
        }
    }
}

/// An irreversible effect.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct IrreversibleEntry {
    /// Reason why the effect is irreversible.
    pub reason: Irreversible,
    /// Target concerned, when the effect concerns one target.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Native bytes of `path`; present only when it is not valid Unicode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_path: Option<Bytes>,
    /// Number of targets concerned.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count: Option<u64>,
    /// Short description, for example the program that caused the effect.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl IrreversibleEntry {
    /// Creates an entry with a reason only.
    #[must_use]
    pub fn new(reason: Irreversible) -> Self {
        Self {
            reason,
            path: None,
            raw_path: None,
            count: None,
            detail: None,
        }
    }
}

/// Durations of the processing steps, in milliseconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
pub struct Timing {
    /// Parsing, correction and binding.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parse_ms: Option<u64>,
    /// Reading sources and transforming records.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read_ms: Option<u64>,
    /// Planning the effect.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_ms: Option<u64>,
    /// Applying the effect.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub apply_ms: Option<u64>,
}

/// Effect report of one program (`nurt-report/1`).
///
/// Optional fields are omitted from JSON when absent and lists are omitted
/// when empty, so that the report of an ordinary effect stays short.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Report {
    /// Format version, `MAJOR.MINOR`; see [`REPORT_VERSION`].
    pub nurt: String,
    /// Outcome of the program.
    pub status: ReportStatus,
    /// Kind of operation: the name of the changing command, or of the source
    /// command of a reading program (for example `replace`, `remove`, `run`,
    /// `find`).
    pub kind: String,
    /// Number of the journal entry of the operation, from 1; absent for
    /// reading programs, previews and changing programs without targets.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub op: Option<u64>,
    /// Canonical text of the program that ran; present only when corrections
    /// were applied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ran: Option<String>,
    /// Hash of the syntax tree: `sha256:` followed by 64 lowercase
    /// hexadecimal digits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ast_hash: Option<String>,
    /// Path of the file that holds the full result (UTF-8), when the result
    /// was written to a file (`save_to` or `output.results_dir`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_file: Option<String>,
    /// Corrections applied to the program text.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub corrections: Vec<Correction>,
    /// Targets enumerated by the program.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub targets: Option<Targets>,
    /// Totals of a changing operation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<Summary>,
    /// Changed targets on this page.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub changes: Vec<Change>,
    /// Sample of differences on this page.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diff: Vec<Hunk>,
    /// Summary of a bulk operation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bulk: Option<Bulk>,
    /// Record of kind `exec` describing a program that ran.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exec: Option<Record>,
    /// Record of kind `job` describing a program that continues in the
    /// background; present when the status is `running`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job: Option<Record>,
    /// Record of kind `test_run` describing a test run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub test: Option<Record>,
    /// Record of kind `analysis` describing an analysis verdict.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub analysis: Option<Record>,
    /// Record describing the effect on a repository: head before and after,
    /// reference updates and conflicts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git: Option<Record>,
    /// Records returned by a reading program.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub records: Option<RecordsPage>,
    /// Unusual effects, in descending order of severity.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notable: Vec<NotableEntry>,
    /// Irreversible effects; present only when an effect is irreversible.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub irreversible: Vec<IrreversibleEntry>,
    /// Error that stopped the program, or a check that failed after the
    /// effect was applied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<Error>,
    /// Cursor of the next page; absent on the last page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    /// Durations of the processing steps (verbosity `detailed`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timing: Option<Timing>,
}

impl Report {
    /// Creates a report with the current format version and no details.
    #[must_use]
    pub fn new(status: ReportStatus, kind: impl Into<String>) -> Self {
        Self {
            nurt: REPORT_VERSION.to_owned(),
            status,
            kind: kind.into(),
            op: None,
            ran: None,
            ast_hash: None,
            result_file: None,
            corrections: Vec::new(),
            targets: None,
            summary: None,
            changes: Vec::new(),
            diff: Vec::new(),
            bulk: None,
            exec: None,
            job: None,
            test: None,
            analysis: None,
            git: None,
            records: None,
            notable: Vec::new(),
            irreversible: Vec::new(),
            error: None,
            next_cursor: None,
            timing: None,
        }
    }

    /// Whether there is a further page of the result.
    #[must_use]
    pub fn has_more(&self) -> bool {
        self.next_cursor.is_some()
    }

    /// Sorts [`Report::notable`] in descending order of severity, keeping
    /// the relative order of entries of equal severity.
    pub fn sort_notable(&mut self) {
        self.notable
            .sort_by_key(|entry| std::cmp::Reverse(entry.severity));
    }

    /// Checks the rules of the format that the JSON schema does not express.
    ///
    /// # Errors
    ///
    /// Returns the first rule that the report breaks.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_contracts::report::{Report, ReportStatus, ReportViolation};
    ///
    /// let mut report = Report::new(ReportStatus::Planned, "remove");
    /// report.op = Some(3);
    /// assert_eq!(report.validate(), Err(ReportViolation::OpInPreview));
    /// ```
    pub fn validate(&self) -> Result<(), ReportViolation> {
        self.validate_header()?;
        self.validate_status()?;
        self.validate_corrections()?;
        self.validate_changes()?;
        self.validate_notable()
    }

    fn validate_header(&self) -> Result<(), ReportViolation> {
        if !is_supported_version(&self.nurt) {
            return Err(ReportViolation::UnsupportedVersion {
                found: self.nurt.clone(),
            });
        }
        if self.kind.is_empty() {
            return Err(ReportViolation::EmptyKind);
        }
        if self.op == Some(0) {
            return Err(ReportViolation::ZeroOp);
        }
        if let Some(hash) = self.ast_hash.as_deref().filter(|hash| !is_ast_hash(hash)) {
            return Err(ReportViolation::InvalidAstHash {
                found: hash.to_owned(),
            });
        }
        if self.result_file.as_deref() == Some("") {
            return Err(ReportViolation::EmptyResultFile);
        }
        if self.next_cursor.as_deref() == Some("") {
            return Err(ReportViolation::EmptyCursor);
        }
        Ok(())
    }

    fn validate_status(&self) -> Result<(), ReportViolation> {
        let status = self.status;
        if status.requires_error() && self.error.is_none() {
            return Err(ReportViolation::MissingError { status });
        }
        match status {
            ReportStatus::Planned if self.op.is_some() => Err(ReportViolation::OpInPreview),
            ReportStatus::Running if self.job.is_none() => Err(ReportViolation::MissingJob),
            ReportStatus::Partial if self.op.is_none() => {
                Err(ReportViolation::MissingOp { status })
            }
            ReportStatus::Partial if self.irreversible.is_empty() && self.error.is_none() => {
                Err(ReportViolation::PartialWithoutRemainder)
            }
            _ => Ok(()),
        }
    }

    fn validate_corrections(&self) -> Result<(), ReportViolation> {
        if self.ran.is_some() && self.corrections.is_empty() {
            return Err(ReportViolation::RanWithoutCorrections);
        }
        match self
            .corrections
            .iter()
            .find(|correction| !Correction::is_rule_id(&correction.rule))
        {
            Some(correction) => Err(ReportViolation::InvalidCorrectionRule {
                rule: correction.rule.clone(),
            }),
            None => Ok(()),
        }
    }

    fn validate_changes(&self) -> Result<(), ReportViolation> {
        if let Some(listed) = self.targets.and_then(|targets| targets.listed)
            && usize::try_from(listed).ok() != Some(self.changes.len())
        {
            return Err(ReportViolation::ListedMismatch {
                listed,
                changes: self.changes.len(),
            });
        }
        if let Some(change) = self
            .changes
            .iter()
            .find(|change| change.action == ChangeAction::Rename && change.from.is_none())
        {
            return Err(ReportViolation::RenameWithoutFrom {
                path: change.path.to_string(),
            });
        }
        for hunk in &self.diff {
            if let Some(line) = hunk.first_invalid_line() {
                return Err(ReportViolation::InvalidHunkLine {
                    path: hunk.path.to_string(),
                    line,
                });
            }
        }
        Ok(())
    }

    fn validate_notable(&self) -> Result<(), ReportViolation> {
        match self
            .notable
            .windows(2)
            .position(|pair| pair[0].severity < pair[1].severity)
        {
            Some(index) => Err(ReportViolation::NotableOrder { index: index + 1 }),
            None => Ok(()),
        }
    }
}

/// A rule of the report format broken by a [`Report`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ReportViolation {
    /// The `nurt` field does not hold a supported version.
    UnsupportedVersion {
        /// Version found in the report.
        found: String,
    },
    /// The `kind` field is empty.
    EmptyKind,
    /// The operation number is zero; numbers start at 1.
    ZeroOp,
    /// The syntax tree hash does not have the form `sha256:<64 hex digits>`.
    InvalidAstHash {
        /// Hash found in the report.
        found: String,
    },
    /// The result file path is empty.
    EmptyResultFile,
    /// The cursor of the next page is empty.
    EmptyCursor,
    /// The status requires an error and none is present.
    MissingError {
        /// Status of the report.
        status: ReportStatus,
    },
    /// A preview report carries an operation number.
    OpInPreview,
    /// A report with status `running` does not carry a job.
    MissingJob,
    /// The status requires an operation number and none is present.
    MissingOp {
        /// Status of the report.
        status: ReportStatus,
    },
    /// A report with status `partial` lists neither irreversible effects nor
    /// an error for the targets left unchanged.
    PartialWithoutRemainder,
    /// The canonical program text is present although no correction was
    /// applied.
    RanWithoutCorrections,
    /// A correction rule identifier does not have the form of a rule.
    InvalidCorrectionRule {
        /// Rule identifier found in the report.
        rule: String,
    },
    /// `targets.listed` differs from the number of changes on the page.
    ListedMismatch {
        /// Value of `targets.listed`.
        listed: u64,
        /// Number of entries in `changes`.
        changes: usize,
    },
    /// A rename does not give the previous path.
    RenameWithoutFrom {
        /// Path of the renamed target.
        path: String,
    },
    /// A line of a hunk has no valid prefix.
    InvalidHunkLine {
        /// Path of the hunk.
        path: String,
        /// Index of the line within the hunk, from 0.
        line: usize,
    },
    /// Notable effects are not in descending order of severity.
    NotableOrder {
        /// Index of the first entry more important than its predecessor.
        index: usize,
    },
}

impl fmt::Display for ReportViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedVersion { found } => {
                write!(
                    f,
                    "unsupported report version `{found}`; expected `1.<minor>`"
                )
            }
            Self::EmptyKind => f.write_str("the operation kind is empty"),
            Self::ZeroOp => f.write_str("operation numbers start at 1"),
            Self::InvalidAstHash { found } => write!(
                f,
                "invalid syntax tree hash `{found}`; expected `sha256:<64 hex digits>`"
            ),
            Self::EmptyResultFile => f.write_str("the result file path is empty"),
            Self::EmptyCursor => f.write_str("the cursor of the next page is empty"),
            Self::MissingError { status } => {
                write!(f, "a report with status `{status}` must carry an error")
            }
            Self::OpInPreview => f.write_str("a preview report must not carry an operation number"),
            Self::MissingJob => f.write_str("a report with status `running` must carry a job"),
            Self::MissingOp { status } => {
                write!(
                    f,
                    "a report with status `{status}` must carry an operation number"
                )
            }
            Self::PartialWithoutRemainder => f.write_str(
                "a report with status `partial` must list irreversible effects or an error",
            ),
            Self::RanWithoutCorrections => {
                f.write_str("the canonical program text is present without corrections")
            }
            Self::InvalidCorrectionRule { rule } => {
                write!(f, "invalid correction rule identifier `{rule}`")
            }
            Self::ListedMismatch { listed, changes } => write!(
                f,
                "targets.listed is {listed} but the page lists {changes} changes"
            ),
            Self::RenameWithoutFrom { path } => {
                write!(f, "the rename of `{path}` does not give the previous path")
            }
            Self::InvalidHunkLine { path, line } => write!(
                f,
                "line {line} of a hunk of `{path}` has no `+`, `-` or space prefix"
            ),
            Self::NotableOrder { index } => write!(
                f,
                "notable entry {index} is more important than the entry before it"
            ),
        }
    }
}

impl std::error::Error for ReportViolation {}

/// Whether `version` is a supported value of the `nurt` field: `1.<minor>`.
///
/// # Examples
///
/// ```
/// use nurt_contracts::report::is_supported_version;
///
/// assert!(is_supported_version("1.0"));
/// assert!(is_supported_version("1.12"));
/// assert!(!is_supported_version("2.0"));
/// assert!(!is_supported_version("1."));
/// ```
#[must_use]
pub fn is_supported_version(version: &str) -> bool {
    version
        .strip_prefix("1.")
        .is_some_and(|minor| !minor.is_empty() && minor.bytes().all(|b| b.is_ascii_digit()))
}

/// Whether `hash` has the form of a syntax tree hash.
///
/// The form is `sha256:` followed by 64 lowercase hexadecimal digits.
///
/// # Examples
///
/// ```
/// use nurt_contracts::report::is_ast_hash;
///
/// assert!(is_ast_hash(&format!("sha256:{}", "0".repeat(64))));
/// assert!(!is_ast_hash("sha256:ABC"));
/// ```
#[must_use]
pub fn is_ast_hash(hash: &str) -> bool {
    hash.strip_prefix(AST_HASH_PREFIX).is_some_and(|digits| {
        digits.len() == SHA256_HEX_DIGITS
            && digits
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}
