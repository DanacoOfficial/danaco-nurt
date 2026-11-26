// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! `create`: files, directories, links and junctions, and entries with a
//! unique name in a temporary directory.
//!
//! The plan step observes every path and plans one target per new entry;
//! missing parent directories are targets of their own, created before the
//! entry. An existing entry of the requested kind is left as it is
//! (`on_exists: skip`), fails the command (`error`) or gets the current time
//! as its access and modification time (`touch`); an existing entry of
//! another kind always fails with `E_EXISTS`.
//!
//! A symbolic link stores its target as written; a relative target is
//! relative to the directory of the link. A hard link and a junction need an
//! existing target, a file and a directory respectively. Junctions exist only
//! on Windows.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use nurt_contracts::env::Os;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::{
    AbsPath, FileKind, FileMeta, Follow, Fs, FsErrorKind, LinkKind, MetaChange,
};
use nurt_contracts::journal::{Expect, FileOp, MutationTx};
use nurt_contracts::report::{Notable, NotableEntry};
use nurt_contracts::stage::{
    BoundArgs, EffectOutcome, EffectPlan, EffectStage, ExecCtx, InputKinds, StageDescriptor,
    StageImpl, StageInput, StageOutput,
};
use nurt_contracts::value;

use super::write::{
    DIRECTORY_MODE, PlannedOp, apply_planned, choice, conflict, plan_parents, present, take_ops,
};
use crate::common::etag::{Observed, observe};
use crate::common::notable::at_path;
use crate::common::path::{path_field, path_list_arg_for_change};

const COMMAND: &str = "create";

const PATH: &str = "path";
const TYPE: &str = "type";
const TARGET: &str = "target";
const HARD: &str = "hard";
const TEMPORARY: &str = "temporary";
const PARENTS: &str = "parents";
const ON_EXISTS: &str = "on_exists";

const TYPE_VALUES: [&str; 4] = ["file", "directory", "link", "junction"];
const ON_EXISTS_VALUES: [&str; 3] = ["skip", "error", "touch"];

/// Start of the names of temporary entries.
const TEMPORARY_PREFIX: &str = "nurt-";
/// Names tried in one directory before `create` gives up.
const TEMPORARY_ATTEMPTS: u32 = 64;
/// Detail of the `unchanged` entry of an entry left as it is.
const EXISTS_DETAIL: &str = "exists";

/// Stage of `create`.
pub(crate) struct Create;

/// Stages of this module, for the registry of the engine.
pub(crate) fn stages() -> Vec<Arc<dyn StageImpl>> {
    vec![Arc::new(Create)]
}

impl StageImpl for Create {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: COMMAND.to_owned(),
            source: true,
            input: InputKinds::None,
        }
    }

    fn as_effect(&self) -> Option<&dyn EffectStage> {
        Some(self)
    }
}

impl EffectStage for Create {
    fn plan<'a>(
        &'a self,
        args: &'a BoundArgs,
        _input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<EffectPlan> {
        let spec = Spec::read(args, ctx)?;
        let paths = path_list_arg_for_change(args, PATH, ctx)?;
        let mut planner = Planner::new(ctx, spec);
        if planner.spec.temporary {
            planner.temporaries(paths)?;
        } else if paths.is_empty() {
            return Err(Error::new(
                ErrorCode::MissingParam,
                "`create` needs a `path`, or `temporary: true` for a new entry in the temporary directory",
            ));
        } else {
            for path in paths {
                ctx.checkpoint()?;
                planner.entry(path)?;
            }
        }
        Ok(planner.finish())
    }

    fn apply<'a>(
        &'a self,
        _args: &'a BoundArgs,
        mut plan: EffectPlan,
        tx: &mut dyn MutationTx,
        ctx: &'a ExecCtx,
    ) -> Result<EffectOutcome<'a>> {
        let ops = take_ops(&mut plan, COMMAND)?;
        let changes = apply_planned(tx, ops, ctx)?;
        Ok(EffectOutcome {
            changes,
            ..EffectOutcome::new(StageOutput::none())
        })
    }
}

/// Kind of the new entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    File,
    Directory,
    Link,
    Junction,
}

impl Kind {
    fn as_str(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Directory => "directory",
            Self::Link => "link",
            Self::Junction => "junction",
        }
    }
}

/// Behaviour for an entry that exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OnExists {
    Skip,
    Error,
    Touch,
}

/// Parameters of one `create`.
#[derive(Debug)]
struct Spec {
    kind: Kind,
    target: Option<String>,
    hard: bool,
    temporary: bool,
    parents: bool,
    on_exists: OnExists,
}

impl Spec {
    fn read(args: &BoundArgs, ctx: &ExecCtx) -> Result<Self> {
        let kind = match choice(args, TYPE, &TYPE_VALUES)? {
            Some("file") => Kind::File,
            Some("directory") => Kind::Directory,
            Some("link") => Kind::Link,
            Some("junction") => Kind::Junction,
            _ => return Err(missing_type(args, ctx)),
        };
        let on_exists = match choice(args, ON_EXISTS, &ON_EXISTS_VALUES)? {
            Some("error") => OnExists::Error,
            Some("touch") => OnExists::Touch,
            _ => OnExists::Skip,
        };
        let spec = Self {
            kind,
            target: args.text(TARGET)?.map(str::to_owned),
            hard: args.bool(HARD)?.unwrap_or(false),
            temporary: args.bool(TEMPORARY)?.unwrap_or(false),
            parents: args.bool(PARENTS)?.unwrap_or(true),
            on_exists,
        };
        spec.check(args, ctx)?;
        Ok(spec)
    }

    fn check(&self, args: &BoundArgs, ctx: &ExecCtx) -> Result<()> {
        let links = matches!(self.kind, Kind::Link | Kind::Junction);
        if !links && present(args, TARGET) {
            return Err(type_conflict(TARGET, self.kind));
        }
        if links && self.target.is_none() {
            return Err(Error::new(
                ErrorCode::MissingParam,
                format!(
                    "`type: {}` needs `target`: give the entry the link points to",
                    self.kind.as_str()
                ),
            ));
        }
        if self.hard && self.kind != Kind::Link {
            return Err(type_conflict(HARD, self.kind));
        }
        if self.temporary && links {
            return Err(conflict(COMMAND, TEMPORARY, "type: link"));
        }
        if self.kind == Kind::Junction && ctx.host().info().os != Os::Windows {
            return Err(Error::new(
                ErrorCode::OsOnly,
                "junctions exist only on Windows: create a link with `type: link`",
            ));
        }
        Ok(())
    }

    /// Whether an existing entry of `kind` is the entry requested.
    fn matches(&self, kind: FileKind) -> bool {
        match self.kind {
            Kind::File => kind == FileKind::File,
            Kind::Directory => kind == FileKind::Directory,
            Kind::Link if self.hard => kind == FileKind::File,
            Kind::Link => kind == FileKind::Symlink,
            Kind::Junction => kind == FileKind::Junction,
        }
    }

    fn target_text(&self) -> &str {
        self.target.as_deref().unwrap_or_default()
    }
}

fn missing_type(args: &BoundArgs, ctx: &ExecCtx) -> Error {
    let error = Error::new(
        ErrorCode::MissingParam,
        "`create` needs `type`: file, directory, link or junction; there is no default",
    );
    let Ok(paths) = path_list_arg_for_change(args, PATH, ctx) else {
        return error;
    };
    let Some(path) = paths.first() else {
        return error;
    };
    let path = path.to_language_string();
    ["file", "directory"]
        .into_iter()
        .fold(error, |error, kind| {
            error.with_fix(format!("create \"{path}\" type: {kind}"))
        })
}

fn type_conflict(name: &str, kind: Kind) -> Error {
    Error::new(
        ErrorCode::ParamConflict,
        format!(
            "`{name}` cannot be used with `type: {}`: remove `{name}` or use `type: link`",
            kind.as_str()
        ),
    )
}

fn kind_name(kind: FileKind) -> &'static str {
    match kind {
        FileKind::File => "file",
        FileKind::Directory => "directory",
        FileKind::Symlink => "symbolic link",
        FileKind::Junction => "junction",
        _ => "special file",
    }
}

fn exists_error(path: &AbsPath) -> Error {
    Error::new(
        ErrorCode::Exists,
        format!("`{path}` already exists: use `on_exists: skip` to keep it or `touch` to update its time"),
    )
    .with_detail("path", serde_json::Value::from(path.to_language_string()))
}

/// Path a link target names: an absolute target as it is, a relative one
/// from the directory of the link.
fn link_target_path(link: &AbsPath, target: &str) -> Result<AbsPath> {
    let written = Path::new(target);
    let joined = if written.is_absolute() {
        written.to_path_buf()
    } else {
        let parent = link.parent().ok_or_else(|| {
            Error::new(
                ErrorCode::InvalidValue,
                format!("`{link}` is a root directory and cannot be a link"),
            )
        })?;
        parent.as_path().join(written)
    };
    AbsPath::new(joined).map_err(|_| {
        Error::new(
            ErrorCode::InvalidValue,
            format!("link target `{target}` leads above the root directory"),
        )
        .with_found(target)
    })
}

/// Collects the planned operations of one `create`.
struct Planner<'c> {
    ctx: &'c ExecCtx,
    spec: Spec,
    ops: Vec<PlannedOp>,
    notable: Vec<NotableEntry>,
    dirs: BTreeSet<AbsPath>,
    seen: BTreeSet<AbsPath>,
}

impl<'c> Planner<'c> {
    fn new(ctx: &'c ExecCtx, spec: Spec) -> Self {
        Self {
            ctx,
            spec,
            ops: Vec::new(),
            notable: Vec::new(),
            dirs: BTreeSet::new(),
            seen: BTreeSet::new(),
        }
    }

    fn finish(self) -> EffectPlan {
        let mut plan = EffectPlan::new();
        plan.changes = self.ops.iter().map(|op| op.change.clone()).collect();
        plan.notable = self.notable;
        plan.bulk = self.ops.len() > 1;
        plan.with_state(self.ops)
    }

    fn entry(&mut self, path: AbsPath) -> Result<()> {
        if !self.seen.insert(path.clone()) {
            return Err(Error::new(
                ErrorCode::ParamConflict,
                format!("`{path}` is given twice: give every path once"),
            ));
        }
        if self.spec.kind == Kind::Directory && self.dirs.contains(&path) {
            return Ok(());
        }
        let ctx = self.ctx;
        let fs = ctx.require_fs()?;
        match observe(fs, &path)? {
            Observed::Present(meta) => self.existing(&path, &meta),
            Observed::Absent => self.absent(fs, path),
        }
    }

    fn absent(&mut self, fs: &dyn Fs, path: AbsPath) -> Result<()> {
        let parents = plan_parents(fs, &path, self.spec.parents, &mut self.dirs)?;
        let op = self.new_entry(fs, &path)?;
        self.ops.extend(parents);
        self.ops.push(op);
        if self.spec.kind == Kind::Directory {
            self.dirs.insert(path);
        }
        Ok(())
    }

    fn existing(&mut self, path: &AbsPath, meta: &FileMeta) -> Result<()> {
        if !self.spec.matches(meta.kind) {
            return Err(Error::new(
                ErrorCode::Exists,
                format!(
                    "`{path}` exists and is a {}, not a {}: choose another path",
                    kind_name(meta.kind),
                    self.spec.kind.as_str()
                ),
            ));
        }
        match self.spec.on_exists {
            OnExists::Skip => {
                self.notable.push(NotableEntry {
                    detail: Some(EXISTS_DETAIL.to_owned()),
                    ..at_path(Notable::Unchanged, path)
                });
                Ok(())
            }
            OnExists::Error => Err(exists_error(path)),
            OnExists::Touch => {
                let now = now_ns(self.ctx)?;
                let change = MetaChange {
                    modified_ns: Some(now),
                    accessed_ns: Some(now),
                    ..MetaChange::default()
                };
                let op = FileOp::set_meta(path.clone(), change).expect(Expect::Etag(meta.etag()));
                self.ops.push(PlannedOp::from_op(op));
                Ok(())
            }
        }
    }

    fn new_entry(&self, fs: &dyn Fs, path: &AbsPath) -> Result<PlannedOp> {
        match self.spec.kind {
            Kind::File => Ok(PlannedOp::from_op(FileOp::create(path.clone(), Vec::new()))),
            Kind::Directory => Ok(PlannedOp::from_op(FileOp::create_dir(
                path.clone(),
                DIRECTORY_MODE,
            ))),
            Kind::Link if self.spec.hard => self.hard_link(fs, path),
            Kind::Link => Ok(self.symbolic_link(fs, path)),
            Kind::Junction => self.junction(fs, path),
        }
    }

    fn hard_link(&self, fs: &dyn Fs, path: &AbsPath) -> Result<PlannedOp> {
        let existing = link_target_path(path, self.spec.target_text())?;
        let meta = target_meta(fs, &existing, Follow::NoFollow)?;
        if meta.kind != FileKind::File {
            return Err(Error::new(
                ErrorCode::InvalidValue,
                format!(
                    "`{existing}` is a {}: a hard link needs a file as its target",
                    kind_name(meta.kind)
                ),
            ));
        }
        let mut op = PlannedOp::from_op(
            FileOp::create_hard_link(path.clone(), existing.clone()).expect(Expect::Absent),
        );
        op.change.link_target = Some(path_field(&existing));
        Ok(op)
    }

    fn symbolic_link(&self, fs: &dyn Fs, path: &AbsPath) -> PlannedOp {
        let target = self.spec.target_text();
        let directory = link_target_path(path, target).is_ok_and(|resolved| {
            fs.metadata(&resolved, Follow::Follow)
                .is_ok_and(|meta| meta.kind == FileKind::Directory)
        });
        let kind = if directory {
            LinkKind::SymlinkDirectory
        } else {
            LinkKind::SymlinkFile
        };
        let mut op = PlannedOp::from_op(
            FileOp::create_link(path.clone(), PathBuf::from(target), kind).expect(Expect::Absent),
        );
        op.change.link_target = Some(value::Path::new(target));
        op
    }

    fn junction(&self, fs: &dyn Fs, path: &AbsPath) -> Result<PlannedOp> {
        let resolved = link_target_path(path, self.spec.target_text())?;
        let meta = target_meta(fs, &resolved, Follow::Follow)?;
        if meta.kind != FileKind::Directory {
            return Err(Error::new(
                ErrorCode::InvalidValue,
                format!(
                    "`{resolved}` is a {}: a junction needs a directory as its target",
                    kind_name(meta.kind)
                ),
            ));
        }
        let stored = resolved.as_path().to_path_buf();
        let mut op = PlannedOp::from_op(
            FileOp::create_link(path.clone(), stored, LinkKind::Junction).expect(Expect::Absent),
        );
        op.change.link_target = Some(path_field(&resolved));
        Ok(op)
    }

    /// Plans one entry with a unique name in every directory; without
    /// directories, in the temporary directory of the host.
    fn temporaries(&mut self, dirs: Vec<AbsPath>) -> Result<()> {
        let dirs = if dirs.is_empty() {
            vec![temporary_dir(self.ctx)?]
        } else {
            dirs
        };
        let ctx = self.ctx;
        let fs = ctx.require_fs()?;
        for dir in dirs {
            ctx.checkpoint()?;
            let path = self.unique_name(fs, &dir)?;
            self.entry(path)?;
        }
        Ok(())
    }

    fn unique_name(&self, fs: &dyn Fs, dir: &AbsPath) -> Result<AbsPath> {
        let stamp = self.ctx.clock().now().unix_nanos();
        for attempt in 0..TEMPORARY_ATTEMPTS {
            let path = dir.join_name(&format!("{TEMPORARY_PREFIX}{stamp:x}-{attempt}"))?;
            if !self.seen.contains(&path) && !observe(fs, &path)?.exists() {
                return Ok(path);
            }
        }
        Err(Error::new(
            ErrorCode::Exists,
            format!("no free temporary name is left in `{dir}`: remove old temporary entries"),
        ))
    }
}

fn target_meta(fs: &dyn Fs, path: &AbsPath, follow: Follow) -> Result<FileMeta> {
    fs.metadata(path, follow).map_err(|error| match error.kind {
        FsErrorKind::NotFound => Error::new(
            ErrorCode::NotFound,
            format!("link target `{path}` does not exist: create it first"),
        )
        .with_detail("path", serde_json::Value::from(path.to_language_string())),
        _ => error.into(),
    })
}

fn temporary_dir(ctx: &ExecCtx) -> Result<AbsPath> {
    AbsPath::new(ctx.host().info().temp.clone()).map_err(|_| {
        Error::new(
            ErrorCode::Config,
            "the temporary directory of the host is not an absolute path: give a directory in `path`",
        )
    })
}

fn now_ns(ctx: &ExecCtx) -> Result<i64> {
    i64::try_from(ctx.clock().now().unix_nanos()).map_err(|_| {
        Error::new(
            ErrorCode::InvalidValue,
            "the current time is outside the range of file times",
        )
    })
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use nurt_contracts::env::Clock;
    use nurt_contracts::journal::{JournalStore, Mutation, TargetAction};
    use nurt_contracts::record::{Change, ChangeAction};
    use nurt_contracts::stage::ArgValue;
    use nurt_contracts::value::Value;
    use nurt_testkit::ctx::TestContext;
    use nurt_testkit::fixtures::begin_request;
    use nurt_testkit::host::TestHost;
    use nurt_testkit::journal::JournalCall;

    use super::*;
    use crate::commands::write::Work;

    fn abs(text: &str) -> AbsPath {
        nurt_testkit::vfs::test_path(text).unwrap()
    }

    fn with_file(path: &str, content: &[u8]) -> (TestContext, AbsPath) {
        let test = TestContext::new();
        let path = abs(path);
        test.fs().create_dir_all(&path.parent().unwrap()).unwrap();
        test.fs().write_file(&path, content).unwrap();
        (test, path)
    }

    #[derive(Debug)]
    struct Run {
        changes: Vec<Change>,
        notable: Vec<Notable>,
        bulk: bool,
    }

    fn set(args: BoundArgs, name: &str, value: Value) -> BoundArgs {
        args.with(name, ArgValue::Value(value), false)
    }

    fn args(paths: &[&str], kind: &str) -> BoundArgs {
        let list = paths
            .iter()
            .map(|path| Value::Path(abs(path).to_language_string()))
            .collect();
        let given = set(BoundArgs::new(COMMAND), PATH, Value::List(list));
        set(given, TYPE, Value::Enum(kind.to_owned()))
    }

    fn run(test: &TestContext, args: &BoundArgs) -> Result<Run> {
        run_in(test, &test.ctx(), args)
    }

    fn run_in(test: &TestContext, ctx: &ExecCtx, args: &BoundArgs) -> Result<Run> {
        let plan = Create.plan(args, StageInput::none(), ctx)?;
        let notable = plan.notable.iter().map(|entry| entry.code).collect();
        let bulk = plan.bulk;
        let mut tx = test.journal().begin(begin_request(COMMAND));
        let changes = Create.apply(args, plan, tx.as_mut(), ctx)?.changes;
        tx.commit()?;
        Ok(Run {
            changes,
            notable,
            bulk,
        })
    }

    fn kind_of(test: &TestContext, path: &str) -> FileKind {
        test.fs()
            .metadata(&abs(path), Follow::NoFollow)
            .unwrap()
            .kind
    }

    fn assert_every_change_declared(test: &TestContext) {
        let calls = test.journal().calls();
        let declared: usize = calls
            .iter()
            .map(|call| match call {
                JournalCall::Preserve { targets, .. } => targets.len(),
                _ => 0,
            })
            .sum();
        let applied = calls
            .iter()
            .filter(|call| matches!(call, JournalCall::Apply { ok: true, .. }))
            .count();
        assert!(declared > 0);
        assert_eq!(declared, applied);
    }

    #[test]
    fn directories_are_created_with_their_parents() {
        let test = TestContext::new();
        let run = run(&test, &args(&["/data/a/b"], "directory")).unwrap();
        let actions: Vec<ChangeAction> = run.changes.iter().map(|change| change.action).collect();
        assert_eq!(actions, [ChangeAction::Create; 3]);
        assert!(run.bulk);
        assert_eq!(kind_of(&test, "/data/a/b"), FileKind::Directory);
        assert_every_change_declared(&test);
    }

    #[test]
    fn nested_new_directories_are_created_once() {
        let test = TestContext::new();
        let run = run(&test, &args(&["/data/a/b", "/data/a"], "directory")).unwrap();
        assert_eq!(run.changes.len(), 3);
        assert_eq!(kind_of(&test, "/data/a"), FileKind::Directory);
        assert_every_change_declared(&test);
    }

    #[test]
    fn missing_parents_fail_without_parents() {
        let test = TestContext::new();
        let given = set(args(&["/data/a"], "directory"), PARENTS, Value::Bool(false));
        let error = run(&test, &given).unwrap_err();
        assert_eq!(error.code(), ErrorCode::NotFound);
        assert!(test.journal().calls().is_empty());
    }

    #[test]
    fn files_are_created_empty() {
        let test = TestContext::new();
        test.fs().create_dir_all(&abs("/data")).unwrap();
        let run = run(&test, &args(&["/data/a.txt", "/data/b.txt"], "file")).unwrap();
        assert_eq!(run.changes.len(), 2);
        assert!(test.fs().read_file(&abs("/data/a.txt")).unwrap().is_empty());
        assert!(test.fs().exists(&abs("/data/b.txt")));
        assert_every_change_declared(&test);
    }

    #[test]
    fn existing_entries_are_skipped_by_default() {
        let (test, _) = with_file("/data/a.txt", b"kept");
        let run = run(&test, &args(&["/data/a.txt"], "file")).unwrap();
        assert!(run.changes.is_empty());
        assert_eq!(run.notable, [Notable::Unchanged]);
        assert_eq!(test.fs().read_file(&abs("/data/a.txt")).unwrap(), b"kept");
    }

    #[test]
    fn existing_entries_fail_with_on_exists_error_or_another_kind() {
        let (test, _) = with_file("/data/a.txt", b"kept");
        let error_mode = set(
            args(&["/data/a.txt"], "file"),
            ON_EXISTS,
            Value::Enum("error".to_owned()),
        );
        assert_eq!(
            run(&test, &error_mode).unwrap_err().code(),
            ErrorCode::Exists
        );
        let other_kind = run(&test, &args(&["/data/a.txt"], "directory")).unwrap_err();
        assert_eq!(other_kind.code(), ErrorCode::Exists);
        assert!(other_kind.message.contains("is a file"));
        assert!(test.journal().calls().is_empty());
    }

    #[test]
    fn touch_sets_the_current_time() {
        let (test, path) = with_file("/data/a.txt", b"kept");
        test.clock().advance(Duration::from_secs(60));
        let given = set(
            args(&["/data/a.txt"], "file"),
            ON_EXISTS,
            Value::Enum("touch".to_owned()),
        );
        let run = run(&test, &given).unwrap();
        assert_eq!(run.changes[0].action, ChangeAction::Attributes);
        let meta = test.fs().metadata(&path, Follow::NoFollow).unwrap();
        let now = i64::try_from(test.clock().now().unix_nanos()).unwrap();
        assert_eq!(meta.modified_ns, now);
        assert_eq!(test.fs().read_file(&path).unwrap(), b"kept");
        assert_every_change_declared(&test);
    }

    #[test]
    fn an_existing_link_of_the_requested_kind_is_left_as_it_is() {
        let (test, _) = with_file("/data/a.txt", b"a");
        test.fs().symlink("a.txt", &abs("/data/link")).unwrap();
        let given = set(
            args(&["/data/link"], "link"),
            TARGET,
            Value::Text("a.txt".to_owned()),
        );
        let kept = run(&test, &given).unwrap();
        assert!(kept.changes.is_empty());
        assert_eq!(kept.notable, [Notable::Unchanged]);
        let wrong = run(&test, &args(&["/data/link"], "file")).unwrap_err();
        assert_eq!(wrong.code(), ErrorCode::Exists);
        assert!(wrong.message.contains("symbolic link"));
        assert_eq!(kind_of(&test, "/data/link"), FileKind::Symlink);
    }

    #[test]
    fn a_path_given_twice_with_a_unicode_name_is_rejected() {
        let test = TestContext::new();
        let name = "/data/\u{142}\u{f3}d\u{17a}";
        let given = args(&[name, name], "directory");
        assert_eq!(
            run(&test, &given).unwrap_err().code(),
            ErrorCode::ParamConflict
        );
        assert!(test.journal().calls().is_empty());
        let once = run(&test, &args(&[name], "directory")).unwrap();
        assert_eq!(once.changes.len(), 2);
    }

    #[test]
    fn symbolic_links_store_the_target_as_written() {
        let (test, _) = with_file("/data/releases/v2/app", b"bin");
        let given = set(
            args(&["/data/current"], "link"),
            TARGET,
            Value::Text("releases/v2".to_owned()),
        );
        let run = run(&test, &given).unwrap();
        assert_eq!(run.changes[0].action, ChangeAction::Link);
        assert_eq!(
            run.changes[0].link_target.as_ref().map(value::Path::as_str),
            Some("releases/v2")
        );
        assert_eq!(kind_of(&test, "/data/current"), FileKind::Symlink);
        assert_eq!(
            test.fs().read_link(&abs("/data/current")).unwrap(),
            PathBuf::from("releases/v2")
        );
        assert_eq!(
            test.fs().read_file(&abs("/data/current/app")).unwrap(),
            b"bin"
        );
        assert_every_change_declared(&test);
    }

    #[test]
    fn hard_links_name_the_same_file() {
        let (test, real) = with_file("/data/real.txt", b"shared");
        let given = set(
            set(args(&["/data/alias.txt"], "link"), HARD, Value::Bool(true)),
            TARGET,
            Value::Text(real.to_language_string()),
        );
        run(&test, &given).unwrap();
        assert_eq!(
            test.fs().read_file(&abs("/data/alias.txt")).unwrap(),
            b"shared"
        );
        let meta = test.fs().metadata(&real, Follow::NoFollow).unwrap();
        assert_eq!(meta.links, 2);

        let missing = set(
            set(args(&["/data/other.txt"], "link"), HARD, Value::Bool(true)),
            TARGET,
            Value::Text("none.txt".to_owned()),
        );
        assert_eq!(
            run(&test, &missing).unwrap_err().code(),
            ErrorCode::NotFound
        );
        let directory = set(
            set(args(&["/data/dir-link"], "link"), HARD, Value::Bool(true)),
            TARGET,
            Value::Text("/".to_owned()),
        );
        assert_eq!(
            run(&test, &directory).unwrap_err().code(),
            ErrorCode::InvalidValue
        );
    }

    #[test]
    fn junctions_exist_only_on_windows() {
        let linux = TestContext::with_host(TestHost::linux());
        let given = set(
            args(&["/data/j"], "junction"),
            TARGET,
            Value::Text("target".to_owned()),
        );
        let error = Create
            .plan(&given, StageInput::none(), &linux.ctx())
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::OsOnly);

        let windows = TestContext::with_host(TestHost::windows());
        windows.fs().create_dir_all(&abs("/data/target")).unwrap();
        let mut plan = Create
            .plan(&given, StageInput::none(), &windows.ctx())
            .unwrap();
        let ops = take_ops(&mut plan, COMMAND).unwrap();
        assert_eq!(ops[0].target.action, TargetAction::Link);
        assert!(matches!(
            &ops[0].work,
            Work::Apply(Mutation::CreateLink {
                kind: LinkKind::Junction,
                ..
            })
        ));
        let file_target = set(
            args(&["/data/k"], "junction"),
            TARGET,
            Value::Text("missing".to_owned()),
        );
        let missing = Create
            .plan(&file_target, StageInput::none(), &windows.ctx())
            .unwrap_err();
        assert_eq!(missing.code(), ErrorCode::NotFound);
    }

    #[test]
    fn temporary_entries_get_unique_names() {
        let test = TestContext::new();
        let given = set(
            args(&["/data/tmp", "/data/tmp"], "file"),
            TEMPORARY,
            Value::Bool(true),
        );
        let run = run(&test, &given).unwrap();
        let created: Vec<&Change> = run
            .changes
            .iter()
            .filter(|change| change.path.as_str().contains(TEMPORARY_PREFIX))
            .collect();
        assert_eq!(created.len(), 2);
        assert_ne!(created[0].path, created[1].path);
        for change in created {
            let path = AbsPath::new(change.path.as_str()).unwrap();
            assert!(test.fs().exists(&path), "{}", change.path.as_str());
        }
        assert_every_change_declared(&test);
    }

    #[test]
    fn temporary_entries_default_to_the_temporary_directory_of_the_host() {
        let test = TestContext::new();
        let ctx = test.ctx();
        let temp = AbsPath::new(ctx.host().info().temp.clone()).unwrap();
        let given = set(
            set(
                BoundArgs::new(COMMAND),
                TYPE,
                Value::Enum("directory".to_owned()),
            ),
            TEMPORARY,
            Value::Bool(true),
        );
        let run = run_in(&test, &ctx, &given).unwrap();
        let last = run.changes.last().unwrap();
        assert!(last.path.as_str().starts_with(&temp.to_language_string()));
        assert_eq!(last.action, ChangeAction::Create);
    }

    #[test]
    fn invalid_combinations_are_rejected() {
        let test = TestContext::new();
        let ctx = test.ctx();
        let plan = |given: &BoundArgs| {
            Create
                .plan(given, StageInput::none(), &ctx)
                .unwrap_err()
                .code()
        };
        let target_with_file = set(
            args(&["/data/a"], "file"),
            TARGET,
            Value::Text("b".to_owned()),
        );
        assert_eq!(plan(&target_with_file), ErrorCode::ParamConflict);
        assert_eq!(plan(&args(&["/data/a"], "link")), ErrorCode::MissingParam);
        let hard_directory = set(args(&["/data/a"], "directory"), HARD, Value::Bool(true));
        assert_eq!(plan(&hard_directory), ErrorCode::ParamConflict);
        let temporary_link = set(
            set(
                args(&["/data"], "link"),
                TARGET,
                Value::Text("b".to_owned()),
            ),
            TEMPORARY,
            Value::Bool(true),
        );
        assert_eq!(plan(&temporary_link), ErrorCode::ParamConflict);
        let untyped = set(
            BoundArgs::new(COMMAND),
            PATH,
            Value::Path(abs("/data/a").to_language_string()),
        );
        let error = Create.plan(&untyped, StageInput::none(), &ctx).unwrap_err();
        assert_eq!(error.code(), ErrorCode::MissingParam);
        assert_eq!(error.fix.len(), 2);
        assert_eq!(plan(&args(&[], "file")), ErrorCode::MissingParam);
        assert_eq!(
            plan(&args(&["/data/a", "/data/a"], "file")),
            ErrorCode::ParamConflict
        );
        assert!(test.journal().calls().is_empty());
    }

    #[test]
    fn stage_is_registered_as_an_effect() {
        let stages = stages();
        let descriptor = stages[0].descriptor();
        assert_eq!(descriptor.command, COMMAND);
        assert_eq!(descriptor.input, InputKinds::None);
        assert!(stages[0].as_effect().is_some());
    }
}
