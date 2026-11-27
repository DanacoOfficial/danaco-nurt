// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-files::commands::conflict
// Responsibility:   Sources, destinations, conflicts and transaction steps shared by `copy`.
// Dependencies:     nurt_contracts, nurt_testkit, serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Sources, destinations, conflicts and transaction steps shared by `copy`,
//! `move`, `remove` and `rename`.
//!
//! A command reads its sources ([`sources`]), places every source at its
//! destination ([`Placer`]), decides what happens to an entry that already
//! occupies the destination ([`OnConflict`], [`Occupant`]) and plans one
//! [`Step`] per target. The apply step runs every step in one protocol pass
//! ([`run_steps`]) and returns the `effect` record ([`effect_output`]).

use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
use std::io::Read;
use std::ops::ControlFlow;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::{AbsPath, Etag, FileKind, FileMeta, Follow, Fs, FsErrorKind};
use nurt_contracts::journal::{FileOp, Mutation, MutationTx, TargetSpec};
use nurt_contracts::record::{Change, ChangeAction, Effect, EffectStatus};
use nurt_contracts::report::{Notable, NotableEntry};
use nurt_contracts::services::ExcludePreset;
use nurt_contracts::stage::{BoundArgs, ExecCtx, RecordStream, StageInput, StageOutput};
use nurt_contracts::value::{Record, RecordKind, Value};
use serde_json::Value as Json;

use crate::common::etag::{Observed, check_if_match, observe, parse_etag};
use crate::common::mutation::{applied_change, planned_change};
use crate::common::notable::at_path;
use crate::common::path::{
    PATH_FIELD, base_dir, path_arg_for_change, path_field, path_list_arg, path_list_arg_for_change,
    record_path, resolve_for_change,
};
use crate::walk::{Entry as WalkEntry, KindFilter, WalkItem, WalkOptions, Walker};

/// Name of the `to` parameter.
pub(crate) const TO: &str = "to";
/// Name of the `into` parameter.
pub(crate) const INTO: &str = "into";
/// Name of the `on_conflict` parameter.
pub(crate) const ON_CONFLICT: &str = "on_conflict";
/// Name of the `check_references` parameter.
pub(crate) const CHECK_REFERENCES: &str = "check_references";

/// Permission bits of directories created for a destination.
pub(crate) const NEW_DIR_MODE: u32 = 0o755;

/// Input records read between two checks of cancellation and deadline.
pub(crate) const CHECKPOINT_INTERVAL: usize = 256;

const ON_CONFLICT_VALUES: [&str; 3] = ["overwrite", "skip", "error"];
const EXISTS_FIELD: &str = "exists";
const ETAG_FIELD: &str = "etag";
const HOLDERS_DETAIL: &str = "holders";

/// Files read by the reference check, at most.
const REFERENCE_SCAN_FILES: usize = 5000;
/// Larger files are not read by the reference check.
const REFERENCE_FILE_BYTES: u64 = 1024 * 1024;
/// Locations listed in the `detail` of `references_left`, at most.
const REFERENCE_SAMPLE: usize = 20;

/// Value of the `on_conflict` parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OnConflict {
    /// Replace the existing entry; the journal keeps it.
    Overwrite,
    /// Keep the existing entry and leave the source as it is.
    Skip,
    /// Fail with `E_EXISTS`.
    Error,
}

/// Reads `on_conflict`; `default` without a value.
pub(crate) fn on_conflict_param(args: &BoundArgs, default: OnConflict) -> Result<OnConflict> {
    match choice(args, ON_CONFLICT)? {
        None => Ok(default),
        Some("overwrite") => Ok(OnConflict::Overwrite),
        Some("skip") => Ok(OnConflict::Skip),
        Some("error") => Ok(OnConflict::Error),
        Some(other) => Err(unknown_value(ON_CONFLICT, other, &ON_CONFLICT_VALUES)),
    }
}

/// Value of an enumeration parameter; `None` without a value.
pub(crate) fn choice<'a>(args: &'a BoundArgs, name: &str) -> Result<Option<&'a str>> {
    match args.value(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Enum(text) | Value::Text(text)) => Ok(Some(text.as_str())),
        Some(_) => Err(type_error(
            args.command(),
            name,
            "a value of the enumeration",
        )),
    }
}

/// Value of a boolean parameter; `default` without a value.
pub(crate) fn flag(args: &BoundArgs, name: &str, default: bool) -> Result<bool> {
    Ok(args.bool(name)?.unwrap_or(default))
}

/// `E_TYPE` for a parameter value of another type.
pub(crate) fn type_error(command: &str, name: &str, expected: &str) -> Error {
    Error::new(
        ErrorCode::Type,
        format!("`{command}` expects {expected} for `{name}`"),
    )
}

/// `E_UNKNOWN_VALUE` for a value outside an enumeration.
pub(crate) fn unknown_value(name: &str, found: &str, expected: &[&str]) -> Error {
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

/// `E_STAGE_FAILED` for a plan that lost the private state of its stage.
pub(crate) fn missing_state(command: &str) -> Error {
    Error::new(
        ErrorCode::StageFailed,
        format!("the plan of `{command}` does not carry the state of its stage"),
    )
}

/// `E_TYPE` for an input record of a kind the stage does not accept.
pub(crate) fn input_kind_error(command: &str, record: &Record) -> Error {
    Error::new(
        ErrorCode::Type,
        format!(
            "`{command}` does not accept `{}` records here: pass records of files",
            record.kind.as_str()
        ),
    )
}

/// `E_NOT_FOUND` for a source that does not exist.
pub(crate) fn not_found_error(path: &AbsPath) -> Error {
    Error::new(
        ErrorCode::NotFound,
        format!("`{path}` does not exist: check the path"),
    )
    .with_detail(PATH_FIELD, Json::from(path.to_language_string()))
}

/// `E_EXISTS` for a destination occupied by an entry that `on_conflict:
/// error` keeps.
pub(crate) fn exists_error(path: &AbsPath) -> Error {
    Error::new(
        ErrorCode::Exists,
        format!(
            "`{path}` already exists: pass `on_conflict: overwrite` to replace it or `on_conflict: skip` to keep it"
        ),
    )
    .with_detail(PATH_FIELD, Json::from(path.to_language_string()))
}

/// `E_EXISTS` for a destination that is an existing directory, which is
/// never replaced.
pub(crate) fn directory_exists_error(path: &AbsPath) -> Error {
    Error::new(
        ErrorCode::Exists,
        format!(
            "`{path}` is an existing directory and is never replaced: remove it first or choose another destination"
        ),
    )
    .with_problem("directory_exists")
    .with_detail(PATH_FIELD, Json::from(path.to_language_string()))
}

/// `E_EXISTS` for a destination occupied by an entry of another kind than
/// the source, for example a directory copied over a file.
pub(crate) fn kind_mismatch_error(path: &AbsPath) -> Error {
    Error::new(
        ErrorCode::Exists,
        format!(
            "`{path}` exists and is not a directory: remove it first or choose another destination"
        ),
    )
    .with_problem("kind_mismatch")
    .with_detail(PATH_FIELD, Json::from(path.to_language_string()))
}

/// `E_INVALID_VALUE` for a directory moved or copied into itself.
pub(crate) fn into_itself_error(source: &AbsPath, destination: &AbsPath) -> Error {
    Error::new(
        ErrorCode::InvalidValue,
        format!("`{destination}` lies inside `{source}`: choose a destination outside the source"),
    )
    .with_detail(PATH_FIELD, Json::from(source.to_language_string()))
}

/// `E_INVALID_VALUE` for a source whose destination is the source itself.
pub(crate) fn same_entry_error(path: &AbsPath) -> Error {
    Error::new(
        ErrorCode::InvalidValue,
        format!("`{path}` is its own destination: choose another destination"),
    )
    .with_detail(PATH_FIELD, Json::from(path.to_language_string()))
}

/// Name of the command that only reads its sources.
const COPY_COMMAND: &str = "copy";

/// Entry the command works on.
#[derive(Debug, Clone)]
pub(crate) struct Source {
    /// Path of the entry.
    pub(crate) path: AbsPath,
    /// Input record, in the stage form.
    pub(crate) record: Option<Record>,
    /// Entity tag the input record carries; the entry must still have it.
    pub(crate) expected: Option<Etag>,
}

impl Source {
    /// Source given by a path.
    pub(crate) fn from_path(path: AbsPath) -> Self {
        Self {
            path,
            record: None,
            expected: None,
        }
    }

    /// Source given by an `entry` record: its path and entity tag. A record
    /// with `exists: false`, as produced by `info`, carries no tag.
    pub(crate) fn from_record(record: Record, ctx: &ExecCtx) -> Result<Self> {
        let path = record_path(&record, ctx)?;
        let missing = matches!(record.get(EXISTS_FIELD), Some(Value::Bool(false)));
        let expected = match record.get(ETAG_FIELD) {
            Some(Value::Text(text)) if !missing => Some(parse_etag(text)?),
            _ => None,
        };
        Ok(Self {
            path,
            record: Some(record),
            expected,
        })
    }

    /// Observes the entry itself, a link in the last component not followed,
    /// and checks the entity tag of the input record.
    ///
    /// # Errors
    ///
    /// Fails with `E_STALE` when the entry changed since the record was
    /// read, and with the errors of the file system.
    pub(crate) fn observe(&self, fs: &dyn Fs) -> Result<Observed> {
        let observed = observe(fs, &self.path)?;
        check_if_match(&self.path, &observed, self.expected.as_ref())?;
        Ok(observed)
    }

    /// Metadata of an entry that must exist.
    ///
    /// # Errors
    ///
    /// Fails as [`Source::observe`] and with `E_NOT_FOUND` for a missing
    /// entry.
    pub(crate) fn require(&self, fs: &dyn Fs) -> Result<FileMeta> {
        match self.observe(fs)? {
            Observed::Present(meta, _) => Ok(meta),
            Observed::Absent => Err(not_found_error(&self.path)),
        }
    }
}

/// Sources of a command: the paths of `param` in the source form, the
/// `entry` records in the stage form.
///
/// # Errors
///
/// Fails with `E_MISSING_PARAM` when the source form has no paths, with
/// `E_TYPE` for records of another kind, and as [`Source::from_record`].
///
/// An entry named more than once is one source: planned twice it would be
/// moved or copied twice and the second step would fail when applied.
pub(crate) fn sources(
    args: &BoundArgs,
    input: StageInput<'_>,
    ctx: &ExecCtx,
    param: &str,
) -> Result<Vec<Source>> {
    if input.is_none() {
        args.require(param)?;
        // A copy only reads its sources; every other command changes them.
        let paths = if args.command() == COPY_COMMAND {
            path_list_arg(args, param, ctx)?
        } else {
            path_list_arg_for_change(args, param, ctx)?
        };
        return Ok(first_of_each_path(
            paths.into_iter().map(Source::from_path).collect(),
        ));
    }
    let mut found = Vec::new();
    for (index, item) in input.into_records()?.enumerate() {
        if index % CHECKPOINT_INTERVAL == 0 {
            ctx.checkpoint()?;
        }
        let record = item?;
        if record.kind != RecordKind::Entry {
            return Err(input_kind_error(args.command(), &record));
        }
        found.push(Source::from_record(record, ctx)?);
    }
    Ok(first_of_each_path(found))
}

/// Keeps the first source of every path, in order.
fn first_of_each_path(found: Vec<Source>) -> Vec<Source> {
    let mut seen = HashSet::with_capacity(found.len());
    found
        .into_iter()
        .filter(|source| seen.insert(source.path.clone()))
        .collect()
}

/// Destination given by `to` or `into`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Destination {
    /// `to`: the new path; an existing directory receives the entry.
    To(AbsPath),
    /// `into`: the directory that receives the entry, created when missing.
    Into(AbsPath),
}

/// Destination of one source; `to` is evaluated on the input record.
///
/// # Errors
///
/// Fails with `E_PARAM_CONFLICT` when both `to` and `into` are given, with
/// `E_MISSING_PARAM` when neither is, with `E_TYPE` for a value that is not
/// a path, and as [`resolve`].
pub(crate) fn destination(
    args: &BoundArgs,
    record: Option<&Record>,
    ctx: &ExecCtx,
) -> Result<Destination> {
    let command = args.command();
    let into = path_arg_for_change(args, INTO, ctx)?;
    let to = match args.eval(TO, record, ctx)? {
        None | Some(Value::Null) => None,
        Some(Value::Path(text) | Value::Text(text)) => Some(resolve_for_change(ctx, &text)?),
        Some(_) => return Err(type_error(command, TO, "a path")),
    };
    match (to, into) {
        (Some(_), Some(_)) => Err(Error::new(
            ErrorCode::ParamConflict,
            format!("`{command}` takes either `to` or `into`, not both"),
        )),
        (Some(to), None) => Ok(Destination::To(to)),
        (None, Some(into)) => Ok(Destination::Into(into)),
        (None, None) => Err(Error::new(
            ErrorCode::MissingParam,
            format!("`{command}` needs a destination: give `to` or `into`"),
        )),
    }
}

/// What occupies a destination.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Occupant {
    /// Nothing.
    Free,
    /// The source itself, under its own name or a name that differs only in
    /// letter case on a volume that does not distinguish case, or another
    /// hard link of it.
    Same(FileMeta),
    /// An existing directory.
    Directory(FileMeta),
    /// Another existing entry: a file, a link or a special file.
    Other(FileMeta),
}

/// Returns `true` when `destination` names `source` in other letter case
/// in the same directory.
pub(crate) fn is_case_variant(source: &AbsPath, destination: &AbsPath) -> bool {
    let names = (source.file_name(), destination.file_name());
    let (Some(old), Some(new)) = names else {
        return false;
    };
    source.parent() == destination.parent()
        && old != new
        && old.to_string_lossy().to_lowercase() == new.to_string_lossy().to_lowercase()
}

/// Destination of one source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Placement {
    /// Path the entry receives.
    pub(crate) path: AbsPath,
    /// `to` named an existing directory, which receives the entry.
    pub(crate) to_is_dir: bool,
}

/// Places sources at their destinations within one plan.
///
/// The placer plans the directories `into` creates, rejects two sources with
/// the same destination, and reports `to_is_dir` and `case_collision`.
pub(crate) struct Placer<'a> {
    fs: &'a dyn Fs,
    created: Vec<AbsPath>,
    created_set: HashSet<AbsPath>,
    claims: HashMap<String, AbsPath>,
    case_sensitive: HashMap<AbsPath, bool>,
    noted: HashSet<(Notable, AbsPath)>,
    notable: Vec<NotableEntry>,
}

impl<'a> Placer<'a> {
    /// Placer reading the file system `fs`.
    pub(crate) fn new(fs: &'a dyn Fs) -> Self {
        Self {
            fs,
            created: Vec::new(),
            created_set: HashSet::new(),
            claims: HashMap::new(),
            case_sensitive: HashMap::new(),
            noted: HashSet::new(),
            notable: Vec::new(),
        }
    }

    /// Path that `source` receives at `destination`.
    ///
    /// # Errors
    ///
    /// Fails with `E_NOT_FOUND` when the directory of `to` does not exist,
    /// with `E_INVALID_VALUE` when `into` or the directory of `to` is not a
    /// directory and for a source without a name, and with the errors of the
    /// file system.
    pub(crate) fn place(
        &mut self,
        source: &AbsPath,
        destination: &Destination,
    ) -> Result<Placement> {
        let name = source.file_name().ok_or_else(|| nameless_error(source))?;
        match destination {
            Destination::Into(dir) => {
                self.ensure_dir(dir)?;
                Ok(Placement {
                    path: child(dir, name)?,
                    to_is_dir: false,
                })
            }
            Destination::To(path) if self.is_dir(path)? => {
                self.note(Notable::ToIsDir, path, None);
                Ok(Placement {
                    path: child(path, name)?,
                    to_is_dir: true,
                })
            }
            Destination::To(path) => {
                self.require_parent(path)?;
                Ok(Placement {
                    path: path.clone(),
                    to_is_dir: false,
                })
            }
        }
    }

    /// Whether `path` is an existing directory, a link to one, or a
    /// directory this plan creates.
    ///
    /// # Errors
    ///
    /// Fails with the errors of the file system other than a missing entry.
    pub(crate) fn is_dir(&self, path: &AbsPath) -> Result<bool> {
        if self.created_set.contains(path) {
            return Ok(true);
        }
        match self.fs.metadata(path, Follow::Follow) {
            Ok(meta) => Ok(meta.kind == FileKind::Directory),
            Err(error) if error.kind == FsErrorKind::NotFound => Ok(false),
            Err(error) => Err(error.into()),
        }
    }

    fn require_parent(&self, path: &AbsPath) -> Result<()> {
        let parent = path.parent().ok_or_else(|| nameless_error(path))?;
        if self.is_dir(&parent)? {
            return Ok(());
        }
        match self.fs.metadata(&parent, Follow::Follow) {
            Err(error) if error.kind == FsErrorKind::NotFound => Err(Error::new(
                ErrorCode::NotFound,
                format!(
                    "the directory `{parent}` of `{path}` does not exist: create it or give the directory as `into`"
                ),
            )
            .with_detail(PATH_FIELD, Json::from(parent.to_language_string()))),
            _ => Err(not_a_directory_error(&parent)),
        }
    }

    fn ensure_dir(&mut self, dir: &AbsPath) -> Result<()> {
        let mut missing = Vec::new();
        let mut current = Some(dir.clone());
        while let Some(path) = current {
            if self.created_set.contains(&path) {
                break;
            }
            match self.fs.metadata(&path, Follow::Follow) {
                Ok(meta) if meta.kind == FileKind::Directory => break,
                Ok(_) => return Err(not_a_directory_error(&path)),
                Err(error) if error.kind == FsErrorKind::NotFound => {
                    current = path.parent();
                    missing.push(path);
                }
                Err(error) => return Err(error.into()),
            }
        }
        for path in missing.into_iter().rev() {
            self.created_set.insert(path.clone());
            self.created.push(path);
        }
        Ok(())
    }

    /// Whether names on the volume of `path` distinguish letter case.
    ///
    /// # Errors
    ///
    /// Fails with the errors of [`Fs::volume`].
    pub(crate) fn case_sensitive(&mut self, path: &AbsPath) -> Result<bool> {
        let dir = path.parent().unwrap_or_else(|| path.clone());
        if let Some(known) = self.case_sensitive.get(&dir) {
            return Ok(*known);
        }
        let sensitive = self.fs.volume(&dir)?.features.case_sensitive;
        self.case_sensitive.insert(dir, sensitive);
        Ok(sensitive)
    }

    /// Records that `source` goes to `destination`.
    ///
    /// # Errors
    ///
    /// Fails with `E_EXISTS` when another source of the plan goes to the
    /// same destination, also when the two destinations differ only in
    /// letter case on a volume that does not distinguish case.
    pub(crate) fn claim(&mut self, destination: &AbsPath, source: &AbsPath) -> Result<()> {
        let sensitive = self.case_sensitive(destination)?;
        let key = fold(destination, sensitive);
        match self.claims.get(&key).cloned() {
            Some(first) if first != *source => {
                if !sensitive {
                    self.note(Notable::CaseCollision, destination, None);
                }
                Err(Error::new(
                    ErrorCode::Exists,
                    format!(
                        "`{first}` and `{source}` both go to `{destination}`: give them different destinations"
                    ),
                )
                .with_problem("duplicate_destination")
                .with_detail(PATH_FIELD, Json::from(destination.to_language_string())))
            }
            _ => {
                self.claims.insert(key, source.clone());
                Ok(())
            }
        }
    }

    /// What occupies `destination`, compared with the source entry `source`.
    ///
    /// A destination inside a directory this plan creates is free. An
    /// existing entry whose stored name differs from `destination` only in
    /// letter case is reported as `case_collision`.
    ///
    /// # Errors
    ///
    /// Fails with the errors of the file system.
    pub(crate) fn occupant(
        &mut self,
        source: &FileMeta,
        destination: &AbsPath,
    ) -> Result<Occupant> {
        if destination
            .parent()
            .is_some_and(|parent| self.created_set.contains(&parent))
        {
            return Ok(Occupant::Free);
        }
        let Observed::Present(meta, _) = observe(self.fs, destination)? else {
            return Ok(Occupant::Free);
        };
        if meta.file_id == source.file_id {
            return Ok(Occupant::Same(meta));
        }
        self.note_case_collision(destination)?;
        Ok(if meta.kind == FileKind::Directory {
            Occupant::Directory(meta)
        } else {
            Occupant::Other(meta)
        })
    }

    fn note_case_collision(&mut self, destination: &AbsPath) -> Result<()> {
        if self.case_sensitive(destination)? {
            return Ok(());
        }
        let (Some(parent), Some(name)) = (destination.parent(), destination.file_name()) else {
            return Ok(());
        };
        let wanted = name.to_string_lossy().to_lowercase();
        let stored = self
            .fs
            .read_dir(&parent)?
            .filter_map(|item| item.ok())
            .find(|item| item.name != name && item.name.to_string_lossy().to_lowercase() == wanted);
        if let Some(stored) = stored {
            let detail = stored.name.to_string_lossy().into_owned();
            self.note(Notable::CaseCollision, destination, Some(detail));
        }
        Ok(())
    }

    /// Adds a notable entry for `path` once per code and path.
    pub(crate) fn note(&mut self, code: Notable, path: &AbsPath, detail: Option<String>) {
        if self.noted.insert((code, path.clone())) {
            self.notable.push(NotableEntry {
                detail,
                ..at_path(code, path)
            });
        }
    }

    /// Steps that create the directories of `into`, parents first, and the
    /// notable entries of the placement.
    pub(crate) fn finish(self) -> (Vec<Step>, Vec<NotableEntry>) {
        let steps = self
            .created
            .into_iter()
            .map(|dir| Step::op(FileOp::create_dir(dir, NEW_DIR_MODE)))
            .collect();
        (steps, self.notable)
    }
}

/// Requires a directory as the shared destination of several sources:
/// `into`, or `to` naming an existing directory.
///
/// # Errors
///
/// Fails with `E_PARAM_CONFLICT` when `to` names no existing directory.
pub(crate) fn require_shared_directory(
    placer: &Placer<'_>,
    destination: &Destination,
    command: &str,
) -> Result<()> {
    match destination {
        Destination::To(path) if !placer.is_dir(path)? => Err(Error::new(
            ErrorCode::ParamConflict,
            format!(
                "`{command}` of several entries needs a directory: give `into: {path}` or an existing directory as `to`"
            ),
        )
        .with_detail(PATH_FIELD, Json::from(path.to_language_string()))),
        _ => Ok(()),
    }
}

fn nameless_error(path: &AbsPath) -> Error {
    Error::new(
        ErrorCode::InvalidValue,
        format!("`{path}` has no name: give a file or directory below the root"),
    )
}

fn not_a_directory_error(path: &AbsPath) -> Error {
    Error::new(
        ErrorCode::InvalidValue,
        format!("`{path}` is not a directory: give a directory as the destination"),
    )
    .with_detail(PATH_FIELD, Json::from(path.to_language_string()))
}

/// Path of the entry `name` in `dir`.
///
/// # Errors
///
/// Fails with `E_INVALID_VALUE` when the joined path is not valid.
pub(crate) fn child(dir: &AbsPath, name: &OsStr) -> Result<AbsPath> {
    Ok(AbsPath::new(dir.as_path().join(name))?)
}

/// Comparison key of a path: the language form, in lower case on volumes
/// that do not distinguish letter case.
pub(crate) fn fold(path: &AbsPath, case_sensitive: bool) -> String {
    let text = path.to_language_string();
    if case_sensitive {
        text
    } else {
        text.to_lowercase()
    }
}

/// Entries below `root`, links not followed, sorted by their path relative
/// to the root, so that every directory comes before its content.
///
/// # Errors
///
/// Fails with the errors of the walk, of the context checkpoint, and with
/// the error of the first entry that cannot be read.
pub(crate) fn walk_tree(fs: &dyn Fs, ctx: &ExecCtx, root: &AbsPath) -> Result<Vec<WalkEntry>> {
    let walker = Walker::new(WalkOptions {
        min_depth: 1,
        kind: KindFilter::Any,
        hidden: true,
        follow_links: false,
        metadata: true,
        ..WalkOptions::default()
    })?;
    let mut entries = Vec::new();
    let mut failure = None;
    walker.run(
        fs,
        std::slice::from_ref(root),
        &|| ctx.checkpoint(),
        |item| match item {
            WalkItem::Entry(entry) => {
                entries.push(entry);
                ControlFlow::Continue(())
            }
            WalkItem::Unreadable(unreadable) => {
                failure = Some(unreadable.error);
                ControlFlow::Break(())
            }
        },
    )?;
    if let Some(error) = failure {
        return Err(error.into());
    }
    entries.sort_by(|left, right| left.relative.cmp(&right.relative));
    Ok(entries)
}

/// One target of a plan and the mutations applied to it, in order.
#[derive(Debug, Clone)]
pub(crate) struct Step {
    /// Declaration of the target.
    pub(crate) target: TargetSpec,
    /// Mutations applied after every target of the pass is preserved.
    pub(crate) mutations: Vec<Mutation>,
    /// Source of a copy, reported as `from`.
    pub(crate) from: Option<AbsPath>,
    /// `change` record with the details known in the plan; the journal
    /// adds sizes, hashes and the way the previous state was kept.
    pub(crate) detail: Option<Change>,
}

impl Step {
    /// Step of one file operation.
    pub(crate) fn op(op: FileOp) -> Self {
        Self {
            target: op.target,
            mutations: vec![op.mutation],
            from: None,
            detail: None,
        }
    }

    /// Step with an explicit declaration and mutations.
    pub(crate) fn new(target: TargetSpec, mutations: Vec<Mutation>) -> Self {
        Self {
            target,
            mutations,
            from: None,
            detail: None,
        }
    }

    /// The same step, reporting `source` as the `from` of its change.
    pub(crate) fn with_source(mut self, source: &AbsPath) -> Self {
        self.from = Some(source.clone());
        self
    }

    /// The same step, with the `change` record known in the plan.
    pub(crate) fn with_detail(mut self, detail: Change) -> Self {
        self.detail = Some(detail);
        self
    }

    /// Planned `change` record.
    pub(crate) fn change(&self) -> Change {
        let mut change = self
            .detail
            .clone()
            .unwrap_or_else(|| planned_change(&self.target));
        if let Some(source) = &self.from {
            change.from = Some(path_field(source));
            change.raw_from = source.raw_bytes();
        }
        if change.action == ChangeAction::Rename && change.from.is_none() {
            // A detail that names only the source and the destination: the path of
            // the record is the path after the change.
            change.from = Some(change.path.clone());
            change.raw_from = change.raw_path.clone();
            if let Some(to) = &change.to {
                change.path = to.clone();
                change.raw_path = change.raw_to.clone();
            }
        }
        change
    }
}

/// Runs every step as one protocol pass: declares and preserves all targets,
/// then applies the mutations of each target in order. Returns the `change`
/// record of every target.
///
/// # Errors
///
/// Fails with the errors of the transaction; `E_IN_USE` lists the processes
/// that hold the entry open ([`with_holders`]).
pub(crate) fn run_steps(
    tx: &mut dyn MutationTx,
    steps: Vec<Step>,
    fs: &dyn Fs,
) -> Result<Vec<Change>> {
    let targets: Vec<TargetSpec> = steps.iter().map(|step| step.target.clone()).collect();
    let kept = tx
        .preserve(&targets)
        .map_err(|error| with_holders(error, fs))?;
    let mut changes = Vec::with_capacity(steps.len());
    for (step, kept) in steps.into_iter().zip(kept) {
        let planned = step.change();
        let mut last = None;
        for mutation in step.mutations {
            let applied = tx
                .apply(kept.index, mutation)
                .map_err(|error| with_holders(error, fs))?;
            last = Some(applied);
        }
        if let Some(applied) = last {
            let journal = applied_change(&step.target, &kept, &applied);
            changes.push(Change {
                size_before: journal.size_before.or(planned.size_before),
                size_after: journal.size_after,
                hash_before: journal.hash_before,
                hash_after: journal.hash_after,
                store: journal.store,
                irreversible: journal.irreversible,
                ..planned
            });
        }
    }
    Ok(changes)
}

/// Adds the processes that hold an entry open to an `E_IN_USE` error that
/// does not list them, read through [`Fs::holders`].
pub(crate) fn with_holders(mut error: Error, fs: &dyn Fs) -> Error {
    if error.code() != ErrorCode::InUse || error.details.contains_key(HOLDERS_DETAIL) {
        return error;
    }
    let path = error
        .details
        .get(PATH_FIELD)
        .and_then(Json::as_str)
        .and_then(|text| AbsPath::new(text).ok());
    let Some(path) = path else {
        return error;
    };
    let holders = fs.holders(&path).unwrap_or_default();
    if holders.is_empty() {
        return error;
    }
    let names: Vec<String> = holders
        .iter()
        .map(|holder| format!("{} ({})", holder.name, holder.pid))
        .collect();
    error.message = format!(
        "`{path}` is open in {}: close it there and run the command again",
        names.join(", ")
    );
    let listed: Vec<Json> = holders
        .iter()
        .map(|holder| serde_json::json!({ "pid": holder.pid, "name": holder.name }))
        .collect();
    error
        .details
        .insert(HOLDERS_DETAIL.to_owned(), Json::Array(listed));
    error
}

/// Output of an effect stage: one `effect` record with the operation, the
/// changes, the codes of the notable entries and the counters of `summary`.
pub(crate) fn effect_output<'a>(
    command: &str,
    tx: &dyn MutationTx,
    changes: &[Change],
    notable: &[NotableEntry],
    summary: &[(&str, u64)],
) -> StageOutput<'a> {
    let codes: Vec<String> = notable
        .iter()
        .map(|entry| entry.code.as_str().to_owned())
        .collect();
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

/// Number of items as a counter.
pub(crate) fn count(items: usize) -> u64 {
    u64::try_from(items).unwrap_or(u64::MAX)
}

/// `references_left` for the textual references to the names of `targets`
/// that remain in the files below the base directory, or below the
/// directory of the first target without a base directory.
///
/// A reference is a whole-word occurrence of the name of a target or of the
/// name without its last extension. Hidden entries, version control,
/// dependency, build and cache directories, files larger than 1 MiB, files
/// with binary content and the targets themselves are not read; at most 5000
/// files are read. The `detail` lists up to 20 locations as `path:line`.
///
/// # Errors
///
/// Fails with the errors of the walk and of the context checkpoint.
pub(crate) fn references_left(ctx: &ExecCtx, targets: &[AbsPath]) -> Result<Option<NotableEntry>> {
    let Some(first) = targets.first() else {
        return Ok(None);
    };
    let names = reference_names(targets);
    if names.is_empty() {
        return Ok(None);
    }
    let root = match base_dir(ctx)? {
        Some(base) => base,
        None => first.parent().unwrap_or_else(|| first.clone()),
    };
    let fs = ctx.require_fs()?;
    let files = reference_files(fs, ctx, &root, targets)?;
    let mut found = 0_u64;
    let mut sample = Vec::new();
    for (index, file) in files.iter().enumerate() {
        if index % CHECKPOINT_INTERVAL == 0 {
            ctx.checkpoint()?;
        }
        let Some(text) = small_text(fs, file) else {
            continue;
        };
        for (number, line) in text.lines().enumerate() {
            if names.iter().any(|name| contains_word(line, name)) {
                found = found.saturating_add(1);
                if sample.len() < REFERENCE_SAMPLE {
                    sample.push(format!("{}:{}", file.to_language_string(), number + 1));
                }
            }
        }
    }
    Ok((found > 0).then(|| NotableEntry {
        count: Some(found),
        detail: Some(sample.join(", ")),
        ..NotableEntry::new(Notable::ReferencesLeft)
    }))
}

fn reference_names(targets: &[AbsPath]) -> Vec<String> {
    let mut names = Vec::new();
    for target in targets {
        let Some(name) = target.file_name().and_then(OsStr::to_str) else {
            continue;
        };
        let mut candidates = vec![name.to_owned()];
        if let Some((stem, _)) = name.rsplit_once('.')
            && !stem.is_empty()
        {
            candidates.push(stem.to_owned());
        }
        for candidate in candidates {
            if !names.contains(&candidate) {
                names.push(candidate);
            }
        }
    }
    names
}

fn reference_files(
    fs: &dyn Fs,
    ctx: &ExecCtx,
    root: &AbsPath,
    targets: &[AbsPath],
) -> Result<Vec<AbsPath>> {
    let walker = Walker::new(WalkOptions {
        kind: KindFilter::File,
        hidden: false,
        metadata: true,
        exclude_presets: vec![
            ExcludePreset::Vcs,
            ExcludePreset::Deps,
            ExcludePreset::Build,
            ExcludePreset::Cache,
        ],
        ..WalkOptions::default()
    })?;
    let mut files = Vec::new();
    walker.run(
        fs,
        std::slice::from_ref(root),
        &|| ctx.checkpoint(),
        |item| {
            if let WalkItem::Entry(entry) = item {
                let small = entry
                    .meta
                    .as_ref()
                    .is_some_and(|meta| meta.size <= REFERENCE_FILE_BYTES);
                let target = targets.iter().any(|target| entry.path.starts_with(target));
                if small && !target {
                    files.push(entry.path);
                }
            }
            if files.len() >= REFERENCE_SCAN_FILES {
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        },
    )?;
    files.sort();
    Ok(files)
}

fn small_text(fs: &dyn Fs, path: &AbsPath) -> Option<String> {
    let mut handle = fs.open_read(path).ok()?;
    let mut bytes = Vec::new();
    handle
        .by_ref()
        .take(REFERENCE_FILE_BYTES)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.contains(&0) {
        return None;
    }
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

/// Whether `line` contains `word` not preceded nor followed by a letter, a
/// digit or `_`.
pub(crate) fn contains_word(line: &str, word: &str) -> bool {
    let is_word = |character: char| character.is_alphanumeric() || character == '_';
    line.match_indices(word).any(|(start, found)| {
        let before = line[..start].chars().next_back();
        let after = line[start + found.len()..].chars().next();
        !before.is_some_and(is_word) && !after.is_some_and(is_word)
    })
}

#[cfg(test)]
pub(crate) mod testing {
    //! Doubles and assertions shared by the tests of the commands.

    use std::collections::BTreeSet;

    use nurt_contracts::error::Result;
    use nurt_contracts::fs::AbsPath;
    use nurt_contracts::journal::JournalStore;
    use nurt_contracts::record::Change;
    use nurt_contracts::report::{Notable, NotableEntry};
    use nurt_contracts::stage::{
        ArgValue, BoundArgs, EffectStage, RecordStream, StageData, StageInput,
    };
    use nurt_contracts::value::{Record, RecordKind, Value};
    use nurt_testkit::ctx::TestContext;
    use nurt_testkit::fixtures::begin_request;
    use nurt_testkit::journal::JournalCall;
    use nurt_testkit::vfs::test_path;

    /// Absolute path of the virtual file system.
    pub(crate) fn abs(text: &str) -> AbsPath {
        test_path(text).expect("test paths are absolute")
    }

    /// Creates a file with its parent directories.
    pub(crate) fn file(test: &TestContext, path: &str, content: &[u8]) -> AbsPath {
        let path = abs(path);
        if let Some(parent) = path.parent() {
            test.fs().create_dir_all(&parent).expect("parent created");
        }
        test.fs().write_file(&path, content).expect("file written");
        path
    }

    /// Creates a directory with its parents.
    pub(crate) fn dir(test: &TestContext, path: &str) -> AbsPath {
        let path = abs(path);
        test.fs().create_dir_all(&path).expect("directory created");
        path
    }

    /// `entry` record with a path only.
    pub(crate) fn entry(path: &AbsPath) -> Record {
        Record::new(RecordKind::Entry).with("path", Value::Path(path.to_language_string()))
    }

    /// Argument with a path value.
    pub(crate) fn path_arg(path: &AbsPath) -> ArgValue {
        ArgValue::Value(Value::Path(path.to_language_string()))
    }

    /// Argument with a list of paths.
    pub(crate) fn paths_arg(paths: &[&AbsPath]) -> ArgValue {
        ArgValue::Value(Value::List(
            paths
                .iter()
                .map(|path| Value::Path(path.to_language_string()))
                .collect(),
        ))
    }

    /// Argument with an enumeration value.
    pub(crate) fn choice_arg(value: &str) -> ArgValue {
        ArgValue::Value(Value::Enum(value.to_owned()))
    }

    /// Argument with a text value.
    pub(crate) fn text_arg(value: &str) -> ArgValue {
        ArgValue::Value(Value::Text(value.to_owned()))
    }

    /// Result of one planned and applied stage.
    #[derive(Debug)]
    pub(crate) struct Run {
        /// Changes of the plan.
        pub(crate) planned: Vec<Change>,
        /// Notable entries of the plan.
        pub(crate) notable: Vec<NotableEntry>,
        /// Changes as applied.
        pub(crate) changes: Vec<Change>,
        /// Output records.
        pub(crate) output: Vec<Record>,
    }

    impl Run {
        /// Codes of the notable entries.
        pub(crate) fn codes(&self) -> Vec<Notable> {
            self.notable.iter().map(|entry| entry.code).collect()
        }

        /// Counter of the `summary` of the `effect` record.
        pub(crate) fn summary(&self, name: &str) -> Option<i64> {
            let Some(Value::Map(summary)) = self.output.first()?.get("summary") else {
                return None;
            };
            match summary.get(name) {
                Some(Value::Int(value)) => Some(*value),
                _ => None,
            }
        }
    }

    /// Plans and applies `stage` on the context of `test` and commits the
    /// transaction; `input` `None` runs the source form.
    pub(crate) fn run(
        stage: &dyn EffectStage,
        test: &TestContext,
        args: &BoundArgs,
        input: Option<Vec<Record>>,
    ) -> Result<Run> {
        let ctx = test.ctx();
        let input = match input {
            Some(records) => StageInput::records(RecordStream::from_records(records)),
            None => StageInput::none(),
        };
        let plan = stage.plan(args, input, &ctx)?;
        let planned = plan.changes.clone();
        let notable = plan.notable.clone();
        let mut tx = test.journal().begin(begin_request(args.command()));
        let outcome = stage.apply(args, plan, tx.as_mut(), &ctx)?;
        let StageData::Records(stream) = outcome.output.data else {
            panic!("an effect stage returns records");
        };
        let output = stream.collect::<Result<Vec<_>>>()?;
        let changes = outcome.changes;
        tx.commit()?;
        Ok(Run {
            planned,
            notable,
            changes,
            output,
        })
    }

    /// Asserts that every declared target was changed through the
    /// transaction and that every change was declared.
    pub(crate) fn assert_journaled(test: &TestContext) {
        let calls = test.journal().calls();
        let declared: usize = calls
            .iter()
            .map(|call| match call {
                JournalCall::Preserve { targets, .. } => targets.len(),
                _ => 0,
            })
            .sum();
        let applied: BTreeSet<u32> = calls
            .iter()
            .filter_map(|call| match call {
                JournalCall::Apply {
                    index, ok: true, ..
                } => Some(index.0),
                _ => None,
            })
            .collect();
        let failed = calls
            .iter()
            .any(|call| matches!(call, JournalCall::Apply { ok: false, .. }));
        assert!(declared > 0, "no target was declared");
        assert!(!failed, "a mutation failed");
        assert_eq!(
            applied.len(),
            declared,
            "declared targets and changed targets differ"
        );
    }

    /// Asserts that nothing was declared or changed.
    pub(crate) fn assert_untouched(test: &TestContext) {
        let changed = test.journal().calls().iter().any(|call| {
            matches!(
                call,
                JournalCall::Preserve { .. } | JournalCall::Apply { ok: true, .. }
            )
        });
        assert!(!changed, "the journal recorded a change");
    }
}

#[cfg(test)]
mod tests {
    use nurt_contracts::fs::{FileId, Holder};
    use nurt_contracts::journal::{TargetAction, TargetPath};
    use nurt_contracts::stage::ArgValue;
    use nurt_testkit::ctx::TestContext;
    use nurt_testkit::vfs::VolumeConfig;

    use super::testing::{abs, dir, entry, file, path_arg};
    use super::*;

    fn meta(test: &TestContext, path: &AbsPath) -> FileMeta {
        test.fs().metadata(path, Follow::NoFollow).unwrap()
    }

    #[test]
    fn on_conflict_reads_every_value_and_the_default() {
        let read = |value: &str| {
            let args = BoundArgs::new("copy").with(
                ON_CONFLICT,
                ArgValue::Value(Value::Enum(value.to_owned())),
                false,
            );
            on_conflict_param(&args, OnConflict::Error)
        };
        assert_eq!(read("overwrite").unwrap(), OnConflict::Overwrite);
        assert_eq!(read("skip").unwrap(), OnConflict::Skip);
        assert_eq!(read("error").unwrap(), OnConflict::Error);
        assert_eq!(read("merge").unwrap_err().code(), ErrorCode::UnknownValue);
        let none = BoundArgs::new("copy");
        assert_eq!(
            on_conflict_param(&none, OnConflict::Overwrite).unwrap(),
            OnConflict::Overwrite
        );
    }

    #[test]
    fn destination_needs_exactly_one_of_to_and_into() {
        let test = TestContext::new();
        let ctx = test.ctx();
        let target = abs("/data/out");
        let both = BoundArgs::new("copy")
            .with(TO, path_arg(&target), false)
            .with(INTO, path_arg(&target), false);
        assert_eq!(
            destination(&both, None, &ctx).unwrap_err().code(),
            ErrorCode::ParamConflict
        );
        let neither = BoundArgs::new("copy");
        assert_eq!(
            destination(&neither, None, &ctx).unwrap_err().code(),
            ErrorCode::MissingParam
        );
        let into = BoundArgs::new("copy").with(INTO, path_arg(&target), false);
        assert_eq!(
            destination(&into, None, &ctx).unwrap(),
            Destination::Into(target)
        );
    }

    #[test]
    fn sources_read_paths_and_entry_records() {
        let test = TestContext::new();
        let ctx = test.ctx();
        let path = file(&test, "/data/a.txt", b"a");
        let args = BoundArgs::new("remove").with("path", path_arg(&path), false);
        let given = sources(&args, StageInput::none(), &ctx, "path").unwrap();
        assert_eq!(given[0].path, path);
        assert_eq!(given[0].expected, None);

        let missing = BoundArgs::new("remove");
        assert_eq!(
            sources(&missing, StageInput::none(), &ctx, "path")
                .unwrap_err()
                .code(),
            ErrorCode::MissingParam
        );

        let record =
            Record::new(RecordKind::Line).with("path", Value::Path(path.to_language_string()));
        let input = StageInput::records(RecordStream::from_records(vec![record]));
        assert_eq!(
            sources(&args, input, &ctx, "path").unwrap_err().code(),
            ErrorCode::Type
        );
    }

    #[test]
    fn a_source_with_an_old_etag_is_stale() {
        let test = TestContext::new();
        let ctx = test.ctx();
        let path = file(&test, "/data/a.txt", b"a");
        let old = crate::common::etag::etag_text(&meta(&test, &path).etag());
        test.fs().write_file(&path, b"changed").unwrap();
        let record = entry(&path).with("etag", Value::Text(old));
        let source = Source::from_record(record, &ctx).unwrap();
        assert_eq!(
            source.observe(test.fs().as_ref()).unwrap_err().code(),
            ErrorCode::Stale
        );
        let missing = entry(&abs("/data/gone")).with("exists", Value::Bool(false));
        let source = Source::from_record(missing, &ctx).unwrap();
        assert_eq!(
            source.require(test.fs().as_ref()).unwrap_err().code(),
            ErrorCode::NotFound
        );
    }

    #[test]
    fn into_plans_missing_directories_parents_first() {
        let test = TestContext::new();
        dir(&test, "/data");
        let source = file(&test, "/src/a.txt", b"a");
        let mut placer = Placer::new(test.fs().as_ref());
        let placement = placer
            .place(&source, &Destination::Into(abs("/data/x/y")))
            .unwrap();
        assert_eq!(placement.path, abs("/data/x/y/a.txt"));
        let (steps, notable) = placer.finish();
        let created: Vec<TargetPath> = steps.iter().map(|step| step.target.path.clone()).collect();
        assert_eq!(
            created,
            [abs("/data/x"), abs("/data/x/y")].map(TargetPath::Fs)
        );
        assert!(
            steps
                .iter()
                .all(|step| step.target.action == TargetAction::Create)
        );
        assert!(notable.is_empty());
    }

    #[test]
    fn to_an_existing_directory_places_inside_and_is_notable() {
        let test = TestContext::new();
        let target = dir(&test, "/data/out");
        let source = file(&test, "/src/a.txt", b"a");
        let mut placer = Placer::new(test.fs().as_ref());
        let placement = placer
            .place(&source, &Destination::To(target.clone()))
            .unwrap();
        assert!(placement.to_is_dir);
        assert_eq!(placement.path, abs("/data/out/a.txt"));
        let (_, notable) = placer.finish();
        assert_eq!(notable[0].code, Notable::ToIsDir);
    }

    #[test]
    fn to_with_a_missing_directory_fails() {
        let test = TestContext::new();
        let source = file(&test, "/src/a.txt", b"a");
        let mut placer = Placer::new(test.fs().as_ref());
        let error = placer
            .place(&source, &Destination::To(abs("/nowhere/b.txt")))
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::NotFound);
        let blocker = file(&test, "/data/file", b"x");
        let error = placer
            .place(&source, &Destination::Into(blocker))
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::InvalidValue);
    }

    #[test]
    fn two_sources_cannot_share_a_destination() {
        let test = TestContext::new();
        let mut placer = Placer::new(test.fs().as_ref());
        let destination = abs("/data/a.txt");
        placer.claim(&destination, &abs("/x/a.txt")).unwrap();
        placer.claim(&destination, &abs("/x/a.txt")).unwrap();
        let error = placer.claim(&destination, &abs("/y/a.txt")).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Exists);
        assert_eq!(error.problem.as_deref(), Some("duplicate_destination"));
    }

    #[test]
    fn letter_case_collides_on_insensitive_volumes() {
        let test = TestContext::new();
        let mut config = VolumeConfig::default();
        config.features.case_sensitive = false;
        test.fs().add_volume(&abs("/ci"), config).unwrap();
        let existing = file(&test, "/ci/Report.txt", b"r");
        let source = file(&test, "/src/report.txt", b"s");
        let mut placer = Placer::new(test.fs().as_ref());
        let requested = abs("/ci/report.txt");
        let occupant = placer.occupant(&meta(&test, &source), &requested).unwrap();
        assert!(matches!(occupant, Occupant::Other(_)));
        placer.claim(&requested, &source).unwrap();
        let error = placer
            .claim(&abs("/ci/REPORT.txt"), &abs("/src/REPORT.txt"))
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::Exists);
        let (_, notable) = placer.finish();
        assert_eq!(notable[0].code, Notable::CaseCollision);
        assert_eq!(notable[0].detail.as_deref(), Some("Report.txt"));
        assert!(test.fs().exists(&existing));
    }

    #[test]
    fn the_source_itself_is_recognised() {
        let test = TestContext::new();
        let source = file(&test, "/data/a.txt", b"a");
        let mut placer = Placer::new(test.fs().as_ref());
        let occupant = placer.occupant(&meta(&test, &source), &source).unwrap();
        assert!(matches!(occupant, Occupant::Same(_)));
        let other = dir(&test, "/data/d");
        let occupant = placer.occupant(&meta(&test, &source), &other).unwrap();
        assert!(matches!(occupant, Occupant::Directory(_)));
        assert!(is_case_variant(&abs("/d/a.txt"), &abs("/d/A.txt")));
        assert!(!is_case_variant(&abs("/d/a.txt"), &abs("/e/A.txt")));
        assert!(!is_case_variant(&abs("/d/a.txt"), &abs("/d/a.txt")));
    }

    #[test]
    fn in_use_errors_list_the_holders() {
        let test = TestContext::new();
        let path = file(&test, "/data/a.txt", b"a");
        let holders = vec![Holder {
            pid: 42,
            name: "editor".to_owned(),
        }];
        test.fs().set_holders(&path, holders);
        let error = Error::new(ErrorCode::InUse, "in use").with_detail(
            PATH_FIELD,
            Json::from(path.as_path().to_string_lossy().into_owned()),
        );
        let error = with_holders(error, test.fs().as_ref());
        assert!(error.message.contains("editor (42)"));
        assert_eq!(
            error.details.get(HOLDERS_DETAIL),
            Some(&serde_json::json!([{ "pid": 42, "name": "editor" }]))
        );
        let other = with_holders(Error::new(ErrorCode::Io, "io"), test.fs().as_ref());
        assert_eq!(other.message, "io");
    }

    #[test]
    fn words_are_matched_on_boundaries() {
        assert!(contains_word("import utils", "utils"));
        assert!(contains_word("see utils.py:", "utils.py"));
        assert!(!contains_word("import utilsx", "utils"));
        assert!(!contains_word("my_utils", "utils"));
        assert_eq!(
            reference_names(&[abs("/a/utils.py"), abs("/a/.env")]),
            ["utils.py", "utils", ".env"]
        );
    }

    #[test]
    fn words_next_to_multibyte_letters_are_not_references() {
        assert!(contains_word("\u{e9} utils \u{fc}", "utils"));
        assert!(!contains_word("\u{e9}utils", "utils"));
        assert!(!contains_word("utils\u{fc}", "utils"));
        assert!(contains_word("(utils)", "utils"));
    }

    #[test]
    fn large_and_hidden_files_are_not_read_for_references() {
        let test = TestContext::new();
        let target = file(&test, "/p/utils.py", b"x\n");
        let mut big = b"import utils\n".to_vec();
        big.resize(usize::try_from(REFERENCE_FILE_BYTES).unwrap() + 1, b' ');
        file(&test, "/p/big.py", &big);
        file(&test, "/p/.hidden.py", b"import utils\n");
        let ctx = test.ctx();
        assert_eq!(references_left(&ctx, &[target]).unwrap(), None);
    }

    #[test]
    fn choices_and_flags_read_values_and_defaults() {
        let given = BoundArgs::new("copy")
            .with("preserve", ArgValue::Value(Value::Bool(true)), false)
            .with(
                "clone",
                ArgValue::Value(Value::Enum("never".to_owned())),
                false,
            );
        assert!(flag(&given, "preserve", false).unwrap());
        assert_eq!(choice(&given, "clone").unwrap(), Some("never"));
        let empty = BoundArgs::new("copy");
        assert!(!flag(&empty, "preserve", false).unwrap());
        assert!(flag(&empty, "preserve", true).unwrap());
        assert_eq!(choice(&empty, "clone").unwrap(), None);
        let number = BoundArgs::new("copy").with("clone", ArgValue::Value(Value::Int(1)), false);
        assert_eq!(
            choice(&number, "clone").unwrap_err().code(),
            ErrorCode::Type
        );
    }

    #[test]
    fn an_unknown_value_names_the_accepted_ones() {
        let error = unknown_value(ON_CONFLICT, "merge", &ON_CONFLICT_VALUES);
        assert_eq!(error.code(), ErrorCode::UnknownValue);
        assert_eq!(error.found.as_deref(), Some("merge"));
        assert!(error.message.contains("overwrite, skip, error"));
    }

    #[test]
    fn walk_tree_lists_directories_before_their_content_and_keeps_links() {
        let test = TestContext::new();
        file(&test, "/t/b/inner.txt", b"i");
        file(&test, "/t/a.txt", b"a");
        test.fs().symlink("/elsewhere", &abs("/t/link")).unwrap();
        let ctx = test.ctx();
        let entries = walk_tree(test.fs().as_ref(), &ctx, &abs("/t")).unwrap();
        let relative: Vec<String> = entries.iter().map(WalkEntry::relative_string).collect();
        assert_eq!(relative, ["a.txt", "b", "b/inner.txt", "link"]);
    }

    #[test]
    fn in_use_errors_without_holders_stay_as_they_are() {
        let test = TestContext::new();
        let plain = with_holders(Error::new(ErrorCode::InUse, "in use"), test.fs().as_ref());
        assert_eq!(plain.message, "in use");
        let path = file(&test, "/data/a.txt", b"a");
        let free = Error::new(ErrorCode::InUse, "in use").with_detail(
            PATH_FIELD,
            Json::from(path.as_path().to_string_lossy().into_owned()),
        );
        let free = with_holders(free, test.fs().as_ref());
        assert_eq!(free.message, "in use");
        assert!(!free.details.contains_key(HOLDERS_DETAIL));
    }

    #[test]
    fn remaining_references_are_counted_with_locations() {
        let test = TestContext::new();
        let target = file(&test, "/p/utils.py", b"def f(): pass\n");
        file(&test, "/p/main.py", b"import utils\nprint(1)\nutils.f()\n");
        file(&test, "/p/.git/config", b"utils\n");
        file(&test, "/p/blob.bin", b"utils\0");
        let ctx = test.ctx();
        let found = references_left(&ctx, &[target]).unwrap().unwrap();
        assert_eq!(found.code, Notable::ReferencesLeft);
        assert_eq!(found.count, Some(2));
        let main = abs("/p/main.py").to_language_string();
        assert_eq!(found.detail, Some(format!("{main}:1, {main}:3")));
        assert_eq!(references_left(&ctx, &[]).unwrap(), None);
    }

    #[test]
    fn a_rename_detail_that_names_only_its_source_and_destination_gets_the_new_path() {
        let source = abs("/a.txt");
        let target = abs("/b.txt");
        let step = Step::op(FileOp::rename(source.clone(), target.clone(), false));
        let mut detail = Change::new(source.to_language_string(), ChangeAction::Rename);
        detail.to = Some(path_field(&target));
        let change = step.with_detail(detail).change();
        assert_eq!(change.path.as_str(), target.to_language_string());
        assert_eq!(
            change.from.map(|path| path.as_str().to_owned()),
            Some(source.to_language_string())
        );
    }

    #[test]
    fn a_rename_step_gives_the_previous_path_and_the_destination() {
        let source = abs("/a.txt");
        let target = abs("/b.txt");
        let change = Step::op(FileOp::rename(source.clone(), target.clone(), false)).change();
        assert_eq!(change.action, ChangeAction::Rename);
        assert_eq!(change.path.as_str(), target.to_language_string());
        assert_eq!(
            change.from.map(|path| path.as_str().to_owned()),
            Some(source.to_language_string())
        );
        assert_eq!(
            change.to.map(|path| path.as_str().to_owned()),
            Some(target.to_language_string())
        );
    }

    #[test]
    fn steps_report_the_source_and_planned_details() {
        let source = abs("/a.txt");
        let target = abs("/b.txt");
        let step = Step::op(FileOp::create(target.clone(), Vec::new())).with_source(&source);
        let change = step.change();
        assert_eq!(change.path.as_str(), target.to_language_string());
        assert_eq!(
            change.from.map(|path| path.as_str().to_owned()),
            Some(source.to_language_string())
        );
        let detail = Change {
            removed: Some(2),
            ..planned_change(&step.target)
        };
        assert_eq!(step.with_detail(detail).change().removed, Some(2));
        let id = FileId {
            volume: 1,
            low: 1,
            high: 0,
        };
        assert_eq!(id.low, 1);
        assert_eq!(
            fold(&abs("/A/b"), false),
            abs("/a/b").to_language_string().to_lowercase()
        );
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
