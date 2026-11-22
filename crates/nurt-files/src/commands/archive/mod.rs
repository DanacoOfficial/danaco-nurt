// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Stages of the archive commands: `archive.list`, `archive.extract` and
//! `archive.create`.
//!
//! The formats are read and written by [`nurt_archive`]: the format of an
//! existing archive is recognised from its content, members are checked
//! against the destination before anything is written, and the content of
//! members is streamed between the archive and the transaction one member at
//! a time.
//!
//! Patterns of `glob` and `only` use the glob syntax of the other file
//! commands: `*` and `?` do not match `/`, `**` matches any number of
//! components and `\` escapes. A pattern without `/` is matched against the
//! last component of the member path, a pattern with `/` against the whole
//! path.

use std::io;
use std::sync::Arc;

use globset::{GlobBuilder, GlobMatcher};
use nurt_archive::list::{ArchiveError, Format, Member, MemberKind};
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::AbsPath;
use nurt_contracts::journal::MutationTx;
use nurt_contracts::record::{ArchiveEntry, Change, Effect, EffectStatus, EntryType};
use nurt_contracts::report::NotableEntry;
use nurt_contracts::stage::{BoundArgs, ExecCtx, RecordStream, StageImpl, StageOutput};
use nurt_contracts::value::{Record, Size, Value};
use serde_json::Value as Json;
use unicode_normalization::UnicodeNormalization;

use crate::common::entry::mode_text;
use crate::common::path::{path_arg, path_field};

pub(crate) mod create;
pub(crate) mod extract;
pub(crate) mod list;

/// Name of the parameter that names the archive file.
const PATH: &str = "path";

/// Value of the `format` parameter and of the `format` field for 7z
/// archives.
const SEVEN_ZIP: &str = "seven_zip";

/// Separator of components in member paths.
const SEPARATOR: char = '/';

/// Separator between the archive file and the member in the language form
/// of a member path, as in `archive:/srv/a.zip!/dir/file`.
const MEMBER_SEPARATOR: &str = "!/";

/// Scheme of the language form of member paths.
const SCHEME: &str = "archive:";

/// Message of the I/O error that stops the archive library after a stage
/// error, which [`Carried`] keeps and returns instead.
const STOPPED: &str = "stopped by the stage";

/// Stages of this module.
pub(crate) fn stages() -> Vec<Arc<dyn StageImpl>> {
    vec![
        Arc::new(list::ArchiveList),
        Arc::new(extract::ArchiveExtract),
        Arc::new(create::ArchiveCreate),
    ]
}

/// Name of a format in the values of the `format` parameter and in the
/// `format` field of `archive_entry` records.
fn format_name(format: Format) -> &'static str {
    match format {
        Format::SevenZ => SEVEN_ZIP,
        other => other.name(),
    }
}

/// Converts an error of the archive library to the error of the language,
/// with the archive file in the details.
fn archive_error(error: &ArchiveError, archive: &AbsPath) -> Error {
    let converted = Error::new(error.code(), error.to_string())
        .with_detail("archive", Json::from(archive.to_language_string()));
    match error {
        ArchiveError::Limit {
            limit,
            allowed,
            actual,
        } => converted
            .with_detail("limit", Json::from(limit.name()))
            .with_expected(Json::from(*allowed))
            .with_actual(Json::from(*actual)),
        ArchiveError::Encrypted { path } if !path.is_empty() => {
            converted.with_detail("member", Json::from(path.as_str()))
        }
        ArchiveError::InvalidPath { path, .. }
        | ArchiveError::DuplicatePath { path }
        | ArchiveError::Changed { path, .. } => converted.with_found(path.as_str()),
        _ => converted,
    }
}

/// Error of a stage carried through a callback of the archive library.
///
/// A callback returns the result of [`Carried::stop`] to end the call;
/// [`Carried::resolve`] then returns the kept stage error instead of the
/// error the library reports.
#[derive(Debug, Default)]
struct Carried {
    error: Option<Error>,
}

impl Carried {
    /// Keeps `error` and returns the error that stops the library.
    fn stop(&mut self, error: Error) -> ArchiveError {
        ArchiveError::Io(self.keep(error))
    }

    /// Keeps `error` and returns the I/O error that stops a reader or
    /// writer callback of the library.
    fn keep(&mut self, error: Error) -> io::Error {
        self.error = Some(error);
        io::Error::other(STOPPED)
    }

    /// The kept stage error, otherwise `error` converted.
    fn resolve(&mut self, error: &ArchiveError, archive: &AbsPath) -> Error {
        self.error
            .take()
            .unwrap_or_else(|| archive_error(error, archive))
    }
}

/// Filter of member paths compiled from `glob` or `only`.
#[derive(Debug, Clone)]
struct PathFilter {
    matcher: GlobMatcher,
    name_only: bool,
}

impl PathFilter {
    /// Whether the member path is selected.
    fn matches(&self, path: &str) -> bool {
        let subject = if self.name_only {
            path.rsplit(SEPARATOR).next().unwrap_or(path)
        } else {
            path
        };
        self.matcher.is_match(subject)
    }
}

/// Filter given for the parameter `name`; `None` without a value.
///
/// # Errors
///
/// Fails with `E_TYPE` for a value that is not a pattern and with
/// `E_INVALID_VALUE` for a pattern that is not a valid glob.
fn path_filter(args: &BoundArgs, name: &str) -> Result<Option<PathFilter>> {
    let pattern = match args.value(name) {
        None | Some(Value::Null) => return Ok(None),
        Some(Value::Glob(text) | Value::Text(text)) => text.as_str(),
        Some(_) => {
            return Err(Error::new(
                ErrorCode::Type,
                format!("`{}` expects a glob for `{name}`", args.command()),
            ));
        }
    };
    let glob = GlobBuilder::new(pattern)
        .literal_separator(true)
        .backslash_escape(true)
        .build()
        .map_err(|error| {
            Error::new(
                ErrorCode::InvalidValue,
                format!("glob `{pattern}` is not valid: {}", error.kind()),
            )
            .with_found(pattern)
        })?;
    Ok(Some(PathFilter {
        matcher: glob.compile_matcher(),
        name_only: !pattern.contains(SEPARATOR),
    }))
}

/// Path given for a required parameter.
///
/// # Errors
///
/// Fails with `E_MISSING_PARAM` without a value and as
/// [`path_arg`](crate::common::path::path_arg).
fn required_path(args: &BoundArgs, name: &str, ctx: &ExecCtx) -> Result<AbsPath> {
    args.require(name)?;
    path_arg(args, name, ctx)?.ok_or_else(|| {
        Error::new(
            ErrorCode::MissingParam,
            format!("`{}` needs a value for `{name}`", args.command()),
        )
    })
}

/// Value of an enumeration parameter; `None` without a value.
///
/// # Errors
///
/// Fails with `E_TYPE` for a value that is not an enumeration value.
fn choice<'a>(args: &'a BoundArgs, name: &str) -> Result<Option<&'a str>> {
    match args.value(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Enum(text) | Value::Text(text)) => Ok(Some(text.as_str())),
        Some(_) => Err(Error::new(
            ErrorCode::Type,
            format!(
                "`{}` expects a value of the enumeration for `{name}`",
                args.command()
            ),
        )),
    }
}

/// Value of a boolean parameter, `default` without a value.
///
/// # Errors
///
/// Fails with `E_TYPE` for a value that is not a boolean.
fn flag(args: &BoundArgs, name: &str, default: bool) -> Result<bool> {
    Ok(args.bool(name)?.unwrap_or(default))
}

/// `E_UNKNOWN_VALUE` for a value outside an enumeration.
fn unknown_value(name: &str, found: &str, expected: &[&str]) -> Error {
    Error::new(
        ErrorCode::UnknownValue,
        format!(
            "`{found}` is not a value of `{name}`: use one of {}",
            expected.join(", ")
        ),
    )
    .with_found(found)
    .with_expected(serde_json::json!(expected))
}

/// `E_STAGE_FAILED` for an apply step called without the state of its plan.
fn missing_state(command: &str) -> Error {
    Error::new(
        ErrorCode::StageFailed,
        format!("the plan of `{command}` carries no state: plan the stage again"),
    )
}

/// Number of items as a counter.
fn count(items: usize) -> u64 {
    u64::try_from(items).unwrap_or(u64::MAX)
}

/// Language form of a member path: `archive:<archive>!/<member>`.
fn member_path(archive: &AbsPath, member: &str) -> String {
    format!(
        "{SCHEME}{}{MEMBER_SEPARATOR}{member}",
        archive.to_language_string()
    )
}

/// Type of a record entry for a type of archive member.
fn entry_type(kind: MemberKind) -> EntryType {
    match kind {
        MemberKind::File => EntryType::File,
        MemberKind::Directory => EntryType::Directory,
        MemberKind::Symlink | MemberKind::HardLink => EntryType::Link,
        MemberKind::Other => EntryType::Other,
    }
}

/// `archive_entry` record of a member; the name and the path are in NFC.
fn archive_entry(archive: &AbsPath, format: Format, member: Member) -> ArchiveEntry {
    ArchiveEntry {
        archive: path_field(archive),
        name: member.name().nfc().collect(),
        size: Size::from_bytes(member.size),
        kind: entry_type(member.kind),
        path: Some(member.path.nfc().collect()),
        packed: member.packed.map(Size::from_bytes),
        modified: member.modified,
        mode: member.mode.map(mode_text),
        link_target: member.link_target,
        compression: member.compression,
        encrypted: Some(member.encrypted),
        crc32: member.crc32.map(|crc| format!("{crc:08x}")),
        format: Some(format_name(format).to_owned()),
    }
}

/// Output of an effect stage: one `effect` record with the operation, the
/// changes, the codes of the notable effects and the counters of the
/// summary.
fn effect_output<'a>(
    command: &str,
    tx: &dyn MutationTx,
    changes: &[Change],
    notable: &[NotableEntry],
    summary: &[(&str, u64)],
) -> StageOutput<'a> {
    let mut codes: Vec<String> = notable
        .iter()
        .map(|entry| entry.code.as_str().to_owned())
        .collect();
    codes.dedup();
    let effect = Effect {
        status: EffectStatus::Ok,
        kind: command.to_owned(),
        op: tx.op().map(|op| op.0),
        targets: Some(count(changes.len())),
        summary: Some(
            summary
                .iter()
                .map(|(name, value)| ((*name).to_owned(), *value))
                .collect(),
        ),
        changes: Some(changes.to_vec()),
        notable: (!codes.is_empty()).then_some(codes),
        ..Effect::default()
    };
    StageOutput::records(RecordStream::once(Record::from(effect)))
}

#[cfg(test)]
mod tests {
    use nurt_archive::codec::Algorithm;
    use nurt_archive::list::LimitKind;
    use nurt_contracts::stage::ArgValue;

    use super::*;

    fn abs(path: &str) -> nurt_contracts::fs::AbsPath {
        nurt_testkit::vfs::test_path(path).unwrap()
    }

    fn args(name: &str, value: Value) -> BoundArgs {
        BoundArgs::new("archive.list").with(name, ArgValue::Value(value), false)
    }

    #[test]
    fn format_names_follow_the_catalog() {
        assert_eq!(format_name(Format::SevenZ), "seven_zip");
        assert_eq!(format_name(Format::Zip), "zip");
        assert_eq!(format_name(Format::Tar(Some(Algorithm::Xz))), "tar_xz");
    }

    #[test]
    fn patterns_without_separator_match_the_name() {
        let filter = path_filter(&args("glob", Value::Glob("*.rs".to_owned())), "glob")
            .unwrap()
            .unwrap();
        assert!(filter.matches("src/lib.rs"));
        assert!(filter.matches("main.rs"));
        assert!(!filter.matches("src/lib.rs.bak"));
    }

    #[test]
    fn patterns_with_separator_match_the_whole_path() {
        let filter = path_filter(&args("glob", Value::Glob("src/*.rs".to_owned())), "glob")
            .unwrap()
            .unwrap();
        assert!(filter.matches("src/lib.rs"));
        assert!(!filter.matches("src/a/lib.rs"));
        let deep = path_filter(&args("glob", Value::Text("src/**/*.rs".to_owned())), "glob")
            .unwrap()
            .unwrap();
        assert!(deep.matches("src/a/b/lib.rs"));
    }

    #[test]
    fn invalid_patterns_and_types_are_rejected() {
        let invalid = path_filter(&args("glob", Value::Glob("[a".to_owned())), "glob").unwrap_err();
        assert_eq!(invalid.code(), ErrorCode::InvalidValue);
        assert_eq!(invalid.found.as_deref(), Some("[a"));
        let wrong = path_filter(&args("glob", Value::Int(1)), "glob").unwrap_err();
        assert_eq!(wrong.code(), ErrorCode::Type);
        assert!(
            path_filter(&BoundArgs::new("archive.list"), "glob")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn limit_errors_carry_the_limit_and_values() {
        let error = archive_error(
            &ArchiveError::Limit {
                limit: LimitKind::Ratio,
                allowed: 1000,
                actual: 5000,
            },
            &abs("/data/bomb.zip"),
        );
        assert_eq!(error.code(), ErrorCode::Limit);
        assert_eq!(error.details.get("limit"), Some(&Json::from("ratio")));
        assert_eq!(error.expected, Some(Json::from(1000)));
        assert_eq!(error.actual, Some(Json::from(5000)));
    }

    #[test]
    fn carried_errors_replace_the_library_error() {
        let mut carried = Carried::default();
        let stopped = carried.stop(Error::new(ErrorCode::Cancelled, "cancelled"));
        let archive = abs("/data/a.zip");
        assert_eq!(
            carried.resolve(&stopped, &archive).code(),
            ErrorCode::Cancelled
        );
        assert_eq!(carried.resolve(&stopped, &archive).code(), ErrorCode::Io);
    }

    #[test]
    fn member_paths_use_the_archive_scheme() {
        let archive = abs("/srv/a.zip");
        assert_eq!(
            member_path(&archive, "dir/file"),
            format!("archive:{}!/dir/file", archive.to_language_string())
        );
    }

    #[test]
    fn enumeration_and_flag_values_are_read() {
        let given = args("on_conflict", Value::Enum("skip".to_owned()));
        assert_eq!(choice(&given, "on_conflict").unwrap(), Some("skip"));
        assert_eq!(choice(&given, "other").unwrap(), None);
        assert_eq!(
            choice(&args("on_conflict", Value::Int(1)), "on_conflict")
                .unwrap_err()
                .code(),
            ErrorCode::Type
        );
        assert!(flag(&given, "allow_outside", true).unwrap());
        assert_eq!(
            unknown_value("format", "rar", &["zip"]).code(),
            ErrorCode::UnknownValue
        );
        assert_eq!(
            missing_state("archive.extract").code(),
            ErrorCode::StageFailed
        );
    }
}
