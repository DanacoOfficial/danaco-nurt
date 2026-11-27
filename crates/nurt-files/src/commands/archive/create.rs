// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-files::commands::archive::create
// Responsibility:   `archive.create`: creates an archive from the input entries.
// Dependencies:     nurt_archive, nurt_contracts, nurt_testkit, serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! `archive.create`: creates an archive from the input entries.
//!
//! Every input record names a file, a directory or a link; a directory adds
//! its whole tree. The name of a member is the path of the entry relative to
//! `base`; without `base` it is the `relative` field of the record (as
//! produced by `find` and `list`), otherwise the name of the entry. An entry
//! that occurs more than once, for example a directory and the files `find`
//! lists inside it, is stored once.
//!
//! Members are written in byte order of their names with the permissions
//! and modification times of the files, so the same tree always yields the
//! same archive. Symbolic links are stored as links; with
//! `follow_links: true` the files and directories they point to are stored
//! instead, and a link that leads back into a directory being stored is
//! skipped with the notable `link_loop_skipped`.
//!
//! The archive is a new file: it is written to a staged file of the
//! transaction, one input open at a time, and switched in when complete.

use std::collections::{BTreeMap, HashMap};
use std::ffi::OsString;
use std::io::{self, Read, Write};
use std::path::Component;

use nurt_archive::create::{CreateOptions, Input, create};
use nurt_archive::list::Format;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::{AbsPath, FileKind, FileMeta, Follow, Fs};
use nurt_contracts::journal::{Expect, Mutation, MutationTx, TargetAction, TargetSpec};
use nurt_contracts::record::EntryType;
use nurt_contracts::report::{Notable, NotableEntry};
use nurt_contracts::stage::{
    BoundArgs, EffectOutcome, EffectPlan, EffectStage, ExecCtx, InputKinds, StageDescriptor,
    StageImpl, StageInput,
};
use nurt_contracts::value::{Instant, Record, RecordKind, Value};
use serde_json::Value as Json;

use super::{
    Carried, SEPARATOR, SEVEN_ZIP, choice, effect_output, flag, missing_state, required_path,
    unknown_value,
};
use crate::common::etag::observe;
use crate::common::mutation::{applied_change, fs_target, planned_change};
use crate::common::notable::at_path;
use crate::common::path::{path_arg, record_path};

const COMMAND: &str = "archive.create";
const TO: &str = "to";
const FORMAT: &str = "format";
const BASE: &str = "base";
const LEVEL: &str = "level";
const FOLLOW_LINKS: &str = "follow_links";
const AUTO: &str = "auto";
const FORMAT_VALUES: [&str; 8] = [
    AUTO, "zip", "tar", "tar_gz", "tar_zst", "tar_xz", "tar_bz2", SEVEN_ZIP,
];

/// Field of `entry` records with the path relative to the listed
/// directory.
const RELATIVE_FIELD: &str = "relative";

/// Value of the `relative` field of the root of a listing.
const ROOT_RELATIVE: &str = ".";

/// Field of `entry` records with the type of the entry.
const TYPE_FIELD: &str = "type";

/// Permission bits stored, including the set-id and sticky bits.
const PERMISSION_BITS: u32 = 0o7777;

/// Stage of `archive.create`.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ArchiveCreate;

impl StageImpl for ArchiveCreate {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: COMMAND.to_owned(),
            source: false,
            input: InputKinds::Only(vec![RecordKind::Entry]),
        }
    }

    fn as_effect(&self) -> Option<&dyn EffectStage> {
        Some(self)
    }
}

/// Counters of the `effect` record.
#[derive(Debug, Clone, Copy, Default)]
struct Summary {
    files: u64,
    directories: u64,
    links: u64,
    bytes: u64,
}

impl Summary {
    fn pairs(self) -> [(&'static str, u64); 4] {
        [
            ("files", self.files),
            ("directories", self.directories),
            ("links", self.links),
            ("bytes", self.bytes),
        ]
    }
}

/// Private state of the plan.
#[derive(Debug)]
struct CreateState {
    target: TargetSpec,
    to: AbsPath,
    options: CreateOptions,
    inputs: Vec<Input>,
    sources: HashMap<String, AbsPath>,
    summary: Summary,
}

impl EffectStage for ArchiveCreate {
    fn plan<'a>(
        &'a self,
        args: &'a BoundArgs,
        input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<EffectPlan> {
        let fs = ctx.require_fs()?;
        let to = required_path(args, TO, ctx)?;
        let options = CreateOptions {
            format: format_param(args, &to)?,
            level: level_param(args)?,
        };
        create(io::sink(), &options, Vec::new(), |_| {
            Ok(Box::new(io::empty()) as Box<dyn Read>)
        })
        .map_err(|error| super::archive_error(&error, &to))?;
        let base = path_arg(args, BASE, ctx)?;
        check_destination(fs, &to)?;

        let records = input
            .into_records()?
            .collect_bounded(ctx.limits().max_records)?;
        let mut collector = Collector {
            fs,
            ctx,
            follow_links: flag(args, FOLLOW_LINKS, false)?,
            members: BTreeMap::new(),
            sources: HashMap::new(),
            descended: Vec::new(),
            notable: Vec::new(),
            summary: Summary::default(),
        };
        for record in &records {
            let path = record_path(record, ctx)?;
            let name = member_name(record, &path, base.as_ref())?;
            collector.add(&path, name.as_deref())?;
        }

        let target = fs_target(&to, TargetAction::Create, Expect::Absent);
        let mut plan = EffectPlan::new();
        plan.changes = vec![planned_change(&target)];
        plan.notable = collector.notable;
        Ok(plan.with_state(CreateState {
            target,
            to,
            options,
            inputs: collector.members.into_values().collect(),
            sources: collector.sources,
            summary: collector.summary,
        }))
    }

    fn apply<'a>(
        &'a self,
        _args: &'a BoundArgs,
        mut plan: EffectPlan,
        tx: &mut dyn MutationTx,
        ctx: &'a ExecCtx,
    ) -> Result<EffectOutcome<'a>> {
        let fs = ctx.require_fs()?;
        let state = plan
            .take_state::<CreateState>()
            .ok_or_else(|| missing_state(COMMAND))?;
        let preserved = tx.preserve(std::slice::from_ref(&state.target))?;
        let Some(preserved) = preserved.into_iter().next() else {
            return Err(missing_state(COMMAND));
        };
        let mut handle = tx.stage(preserved.index)?;
        let mut carried = Carried::default();
        let sources = &state.sources;
        let written = create(&mut handle, &state.options, state.inputs, |input| {
            ctx.checkpoint().map_err(|error| carried.keep(error))?;
            let Some(source) = sources.get(&input.path) else {
                return Err(carried.keep(missing_state(COMMAND)));
            };
            fs.open_read(source)
                .map(|reader| Box::new(reader) as Box<dyn Read>)
                .map_err(|error| carried.keep(error.into()))
        });
        written.map_err(|error| carried.resolve(&error, &state.to))?;
        handle.flush().map_err(|error| {
            Error::new(
                ErrorCode::Io,
                format!("cannot write `{}`: {error}", state.to),
            )
        })?;
        drop(handle);
        let applied = tx.apply(preserved.index, Mutation::CommitStaged)?;
        let changes = vec![applied_change(&state.target, &preserved, &applied)];
        let output = effect_output(
            COMMAND,
            &*tx,
            &changes,
            &plan.notable,
            &state.summary.pairs(),
        );
        let mut outcome = EffectOutcome::new(output);
        outcome.changes = changes;
        Ok(outcome)
    }
}

/// Reads `format`; `auto` selects the format from the extension of `to`.
fn format_param(args: &BoundArgs, to: &AbsPath) -> Result<Format> {
    match choice(args, FORMAT)? {
        None | Some(AUTO) => {
            let name = to
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            Format::from_file_name(&name).ok_or_else(|| {
                Error::new(
                    ErrorCode::InvalidValue,
                    format!(
                        "the archive format cannot be told from the name `{name}`: set `{FORMAT}` or use an extension such as .zip or .tar.gz"
                    ),
                )
                .with_found(name.as_str())
            })
        }
        Some(SEVEN_ZIP) => Ok(Format::SevenZ),
        Some(value) => FORMAT_VALUES
            .contains(&value)
            .then(|| Format::from_name(value))
            .flatten()
            .ok_or_else(|| unknown_value(FORMAT, value, &FORMAT_VALUES)),
    }
}

/// Reads `level`; `None` selects the default level of the format.
fn level_param(args: &BoundArgs) -> Result<Option<u32>> {
    args.int(LEVEL)?
        .map(|level| {
            u32::try_from(level).map_err(|_| {
                Error::new(
                    ErrorCode::InvalidValue,
                    format!("`{LEVEL}` must be 0 or more, not {level}"),
                )
                .with_found(level.to_string())
            })
        })
        .transpose()
}

/// Checks that `to` does not exist and that its directory does.
fn check_destination(fs: &dyn Fs, to: &AbsPath) -> Result<()> {
    if observe(fs, to)?.exists() {
        return Err(Error::new(
            ErrorCode::Exists,
            format!("`{to}` exists: remove it first or choose another archive name"),
        )
        .with_detail("path", Json::from(to.to_language_string())));
    }
    let parent_is_directory = to.parent().is_some_and(|parent| {
        fs.metadata(&parent, Follow::Follow)
            .is_ok_and(|meta| meta.kind == FileKind::Directory)
    });
    if parent_is_directory {
        Ok(())
    } else {
        Err(Error::new(
            ErrorCode::NotFound,
            format!("the directory of `{to}` does not exist: create it first"),
        )
        .with_detail("path", Json::from(to.to_language_string())))
    }
}

/// Name of the member of an input entry; `None` when the entry is `base`
/// itself, whose content is stored without it.
fn member_name(record: &Record, path: &AbsPath, base: Option<&AbsPath>) -> Result<Option<String>> {
    if let Some(base) = base {
        let relative = path.as_path().strip_prefix(base.as_path()).map_err(|_| {
            Error::new(
                ErrorCode::InvalidValue,
                format!("`{path}` is not inside the base directory `{base}`: change `{BASE}`"),
            )
            .with_found(path.to_language_string())
        })?;
        let mut parts = Vec::new();
        for component in relative.components() {
            if let Component::Normal(part) = component {
                parts.push(unicode_name(part.to_os_string(), path)?);
            }
        }
        return Ok((!parts.is_empty()).then(|| parts.join("/")));
    }
    let relative = match record.get(RELATIVE_FIELD) {
        Some(Value::Text(text) | Value::Path(text)) if !text.is_empty() => Some(text.clone()),
        _ => None,
    };
    let is_root = relative.as_deref() == Some(ROOT_RELATIVE);
    if is_root && is_directory_record(record) {
        return Ok(None);
    }
    let name = match relative.filter(|_| !is_root) {
        Some(relative) => relative,
        None => {
            let Some(name) = path.file_name() else {
                return Err(Error::new(
                    ErrorCode::InvalidValue,
                    format!("`{path}` has no name to store: set `{BASE}`"),
                ));
            };
            unicode_name(name.to_os_string(), path)?
        }
    };
    normalised_name(&name, path).map(Some)
}

/// Whether an input record is a directory, by its `type` field.
fn is_directory_record(record: &Record) -> bool {
    let directory = EntryType::Directory.as_str();
    matches!(
        record.get(TYPE_FIELD),
        Some(Value::Enum(kind) | Value::Text(kind)) if kind == directory
    )
}

/// Relative member name without `.` and empty components.
fn normalised_name(name: &str, path: &AbsPath) -> Result<String> {
    let parts: Vec<&str> = name
        .split(SEPARATOR)
        .filter(|part| !part.is_empty() && *part != ".")
        .collect();
    if name.starts_with(SEPARATOR) || parts.contains(&"..") || parts.is_empty() {
        return Err(Error::new(
            ErrorCode::InvalidValue,
            format!("`{name}` cannot name a member of `{path}`: give a relative path without `..`"),
        )
        .with_found(name));
    }
    Ok(parts.join("/"))
}

fn unicode_name(name: OsString, path: &AbsPath) -> Result<String> {
    name.into_string().map_err(|_| {
        Error::new(
            ErrorCode::InvalidValue,
            format!("the name of `{path}` is not valid Unicode and cannot be stored in an archive"),
        )
    })
}

/// Members collected from the input entries.
struct Collector<'a> {
    fs: &'a dyn Fs,
    ctx: &'a ExecCtx,
    follow_links: bool,
    members: BTreeMap<String, Input>,
    sources: HashMap<String, AbsPath>,
    /// Real paths of the directories being stored, outermost first, used to
    /// detect followed links that lead back into them.
    descended: Vec<AbsPath>,
    notable: Vec<NotableEntry>,
    summary: Summary,
}

impl Collector<'_> {
    fn add(&mut self, path: &AbsPath, name: Option<&str>) -> Result<()> {
        self.ctx.checkpoint()?;
        let meta = self.fs.metadata(path, Follow::NoFollow)?;
        match meta.kind {
            FileKind::Symlink | FileKind::Junction if self.follow_links => {
                let followed = self.fs.metadata(path, Follow::Follow)?;
                self.add_followed(path, name, &followed)
            }
            FileKind::Symlink | FileKind::Junction => {
                let name = required_name(name, path)?;
                let target = self.fs.read_link(path)?;
                let Some(text) = target.to_str() else {
                    return Err(Error::new(
                        ErrorCode::InvalidValue,
                        format!("the target of `{path}` is not valid Unicode and cannot be stored"),
                    ));
                };
                let stored = if cfg!(windows) {
                    text.replace('\\', "/")
                } else {
                    text.to_owned()
                };
                if self.insert(name, with_meta(Input::symlink(name, stored), &meta), None) {
                    self.summary.links += 1;
                }
                Ok(())
            }
            _ => self.add_followed(path, name, &meta),
        }
    }

    /// Adds a file or a directory, `meta` being the metadata of the entry or
    /// of the target of a followed link.
    fn add_followed(&mut self, path: &AbsPath, name: Option<&str>, meta: &FileMeta) -> Result<()> {
        match meta.kind {
            FileKind::File => {
                let name = required_name(name, path)?;
                let input = with_meta(Input::file(name, meta.size), meta);
                if self.insert(name, input, Some(path)) {
                    self.summary.files += 1;
                    self.summary.bytes = self.summary.bytes.saturating_add(meta.size);
                }
                Ok(())
            }
            FileKind::Directory => self.add_directory(path, name, meta),
            _ => {
                self.notable.push(at_path(Notable::SpecialFile, path));
                Ok(())
            }
        }
    }

    fn add_directory(&mut self, path: &AbsPath, name: Option<&str>, meta: &FileMeta) -> Result<()> {
        let real = if self.follow_links {
            let real = self.fs.real_path(path)?;
            if self.descended.contains(&real) {
                self.notable.push(at_path(Notable::LinkLoopSkipped, path));
                return Ok(());
            }
            Some(real)
        } else {
            None
        };
        if let Some(name) = name {
            if !self.insert(name, with_meta(Input::directory(name), meta), None) {
                return Ok(());
            }
            self.summary.directories += 1;
        }
        let mut children: Vec<OsString> = Vec::new();
        for child in self.fs.read_dir(path)? {
            children.push(child?.name);
        }
        children.sort();
        if let Some(real) = real {
            self.descended.push(real);
        }
        let result = children.into_iter().try_for_each(|child| {
            let child_name = unicode_name(child, path)?;
            let child_path = path.join_name(&child_name)?;
            let member = match name {
                Some(name) => format!("{name}/{child_name}"),
                None => child_name,
            };
            self.add(&child_path, Some(&member))
        });
        if self.follow_links {
            self.descended.pop();
        }
        result
    }

    /// Adds a member unless one with the same name exists; returns whether
    /// it was added.
    fn insert(&mut self, name: &str, input: Input, source: Option<&AbsPath>) -> bool {
        if self.members.contains_key(name) {
            return false;
        }
        if let Some(source) = source {
            self.sources.insert(name.to_owned(), source.clone());
        }
        self.members.insert(name.to_owned(), input);
        true
    }
}

fn required_name<'n>(name: Option<&'n str>, path: &AbsPath) -> Result<&'n str> {
    name.ok_or_else(|| {
        Error::new(
            ErrorCode::InvalidValue,
            format!(
                "`{path}` is the base directory but not a directory: choose a parent as `{BASE}`"
            ),
        )
    })
}

/// Input with the permissions and modification time of a file.
fn with_meta(input: Input, meta: &FileMeta) -> Input {
    input
        .with_mode(meta.mode & PERMISSION_BITS)
        .with_modified(Instant::from_unix_nanos(meta.modified_ns))
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use nurt_archive::codec::Algorithm;
    use nurt_archive::list::list;
    use nurt_contracts::journal::JournalStore;
    use nurt_contracts::stage::{ArgValue, RecordStream};
    use nurt_testkit::ctx::TestContext;
    use nurt_testkit::fixtures::begin_request;

    use super::*;

    fn abs(path: &str) -> nurt_contracts::fs::AbsPath {
        nurt_testkit::vfs::test_path(path).unwrap()
    }

    fn args(to: &str, given: &[(&str, Value)]) -> BoundArgs {
        let base = BoundArgs::new(COMMAND).with(
            TO,
            ArgValue::Value(Value::Path(abs(to).to_language_string())),
            false,
        );
        given.iter().fold(base, |args, (name, value)| {
            args.with(*name, ArgValue::Value(value.clone()), false)
        })
    }

    fn entry(path: &str) -> Record {
        Record::new(RecordKind::Entry).with("path", Value::Path(abs(path).to_language_string()))
    }

    fn run(test: &TestContext, args: &BoundArgs, records: Vec<Record>) -> Result<Vec<String>> {
        let ctx = test.ctx();
        let input = StageInput::records(RecordStream::from_records(records));
        let plan = ArchiveCreate.plan(args, input, &ctx)?;
        let mut tx = test.journal().begin(begin_request(COMMAND));
        ArchiveCreate.apply(args, plan, tx.as_mut(), &ctx)?;
        tx.commit()?;
        let to = path_arg(args, TO, &ctx)?.unwrap_or_else(|| abs("/"));
        let bytes = test.fs().read_file(&to).unwrap();
        let mut names = Vec::new();
        list(Cursor::new(bytes), None, |member| {
            names.push(member.path);
            Ok(())
        })
        .unwrap();
        Ok(names)
    }

    fn tree(test: &TestContext) {
        test.fs().write_file(&abs("/src/app/a.txt"), b"a").unwrap();
        test.fs()
            .write_file(&abs("/src/app/sub/b.txt"), b"bb")
            .unwrap();
    }

    #[test]
    fn directories_add_their_whole_tree() {
        let test = TestContext::new();
        tree(&test);
        let names = run(&test, &args("/out/app.zip", &[]), vec![entry("/src/app")]);
        assert!(names.is_err());
        test.fs().create_dir_all(&abs("/out")).unwrap();
        let names = run(&test, &args("/out/app.zip", &[]), vec![entry("/src/app")]).unwrap();
        assert_eq!(names, ["app", "app/a.txt", "app/sub", "app/sub/b.txt"]);
    }

    #[test]
    fn base_names_members_relative_to_it() {
        let test = TestContext::new();
        tree(&test);
        let given = [(BASE, Value::Path(abs("/src/app").to_language_string()))];
        let names = run(
            &test,
            &args("/src/app.tar", &given),
            vec![entry("/src/app"), entry("/src/app/a.txt")],
        )
        .unwrap();
        assert_eq!(names, ["a.txt", "sub", "sub/b.txt"]);
        let outside = run(
            &test,
            &args("/src/x.tar", &given),
            vec![entry("/src/other")],
        );
        assert_eq!(outside.unwrap_err().code(), ErrorCode::InvalidValue);
    }

    #[test]
    fn relative_fields_name_members_without_base() {
        let test = TestContext::new();
        tree(&test);
        let record =
            entry("/src/app/sub/b.txt").with("relative", Value::Text("sub/b.txt".to_owned()));
        let names = run(&test, &args("/src/b.tgz", &[]), vec![record]).unwrap();
        assert_eq!(names, ["sub/b.txt"]);
        let escaping = entry("/src/app/a.txt").with("relative", Value::Text("../a.txt".to_owned()));
        let error = run(&test, &args("/src/c.tgz", &[]), vec![escaping]).unwrap_err();
        assert_eq!(error.code(), ErrorCode::InvalidValue);
    }

    #[test]
    fn the_root_record_of_a_listing_adds_its_content_without_a_member() {
        let test = TestContext::new();
        tree(&test);
        let root = entry("/src/app")
            .with("relative", Value::Text(".".to_owned()))
            .with("type", Value::Enum("directory".to_owned()));
        let child = entry("/src/app/a.txt").with("relative", Value::Text("a.txt".to_owned()));
        let names = run(&test, &args("/src/dot.tar", &[]), vec![root, child]).unwrap();
        assert_eq!(names, ["a.txt", "sub", "sub/b.txt"]);
    }

    #[test]
    fn a_root_file_is_stored_under_its_own_name() {
        let test = TestContext::new();
        tree(&test);
        let record = entry("/src/app/a.txt")
            .with("relative", Value::Text(".".to_owned()))
            .with("type", Value::Enum("file".to_owned()));
        let names = run(&test, &args("/src/file.tar", &[]), vec![record]).unwrap();
        assert_eq!(names, ["a.txt"]);
    }

    #[test]
    fn links_are_stored_or_followed() {
        let test = TestContext::new();
        tree(&test);
        test.fs().symlink("sub", &abs("/src/app/current")).unwrap();
        test.fs().symlink("..", &abs("/src/app/sub/up")).unwrap();
        let stored = run(&test, &args("/src/l.zip", &[]), vec![entry("/src/app")]).unwrap();
        assert!(stored.contains(&"app/current".to_owned()));
        let followed = run(
            &test,
            &args("/src/f.zip", &[(FOLLOW_LINKS, Value::Bool(true))]),
            vec![entry("/src/app")],
        )
        .unwrap();
        assert!(followed.contains(&"app/current/b.txt".to_owned()));
        assert!(!followed.iter().any(|name| name.starts_with("app/sub/up")));
    }

    #[test]
    fn formats_and_levels_are_checked() {
        let test = TestContext::new();
        tree(&test);
        let unknown = run(&test, &args("/src/app.data", &[]), vec![entry("/src/app")]);
        assert_eq!(unknown.unwrap_err().code(), ErrorCode::InvalidValue);
        let level = run(
            &test,
            &args("/src/app.zip", &[(LEVEL, Value::Int(12))]),
            vec![entry("/src/app")],
        );
        assert_eq!(level.unwrap_err().code(), ErrorCode::InvalidValue);
        let negative = run(
            &test,
            &args("/src/app.zip", &[(LEVEL, Value::Int(-1))]),
            vec![entry("/src/app")],
        );
        assert_eq!(negative.unwrap_err().code(), ErrorCode::InvalidValue);
        let named = args(
            "/src/app.data",
            &[(FORMAT, Value::Enum("tar_xz".to_owned()))],
        );
        assert_eq!(
            format_param(&named, &abs("/src/app.data")).unwrap(),
            Format::Tar(Some(Algorithm::Xz))
        );
        let seven = args(
            "/src/app.data",
            &[(FORMAT, Value::Enum(SEVEN_ZIP.to_owned()))],
        );
        assert_eq!(
            format_param(&seven, &abs("/src/app.data")).unwrap(),
            Format::SevenZ
        );
        let lz4 = args(
            "/src/app.data",
            &[(FORMAT, Value::Enum("tar_lz4".to_owned()))],
        );
        assert_eq!(
            format_param(&lz4, &abs("/src/app.data"))
                .unwrap_err()
                .code(),
            ErrorCode::UnknownValue
        );
    }

    #[test]
    fn an_existing_archive_is_not_replaced() {
        let test = TestContext::new();
        tree(&test);
        test.fs().write_file(&abs("/src/app.zip"), b"old").unwrap();
        let error = run(&test, &args("/src/app.zip", &[]), vec![entry("/src/app")]).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Exists);
        assert_eq!(test.fs().read_file(&abs("/src/app.zip")).unwrap(), b"old");
    }

    #[test]
    fn dangling_links_fail_when_followed() {
        let test = TestContext::new();
        tree(&test);
        test.fs()
            .symlink("missing", &abs("/src/app/broken"))
            .unwrap();
        let error = run(
            &test,
            &args("/src/b.zip", &[(FOLLOW_LINKS, Value::Bool(true))]),
            vec![entry("/src/app")],
        )
        .unwrap_err();
        assert_eq!(error.code(), ErrorCode::NotFound);
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
