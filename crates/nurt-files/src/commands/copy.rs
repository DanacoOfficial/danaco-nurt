// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! `copy`: copies files, links and directory trees.
//!
//! Every copied entry is one target of the transaction: a created
//! directory, a file copied with [`Mutation::CopyFrom`] or a created link.
//! A directory tree is walked once and copied in one operation; an existing
//! destination directory receives the content of the source directory, and
//! conflicts are decided per file by `on_conflict`.

use std::collections::HashSet;
use std::sync::Arc;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::{AbsPath, FileKind, FileMeta, Follow, Fs, LinkKind};
use nurt_contracts::journal::{CloneMode, Expect, FileOp, Mutation, MutationTx, TargetAction};
use nurt_contracts::report::{Notable, NotableEntry};
use nurt_contracts::stage::{
    BoundArgs, EffectOutcome, EffectPlan, EffectStage, ExecCtx, InputKinds, StageDescriptor,
    StageImpl, StageInput,
};
use nurt_contracts::value::RecordKind;
use serde_json::Value as Json;

use super::conflict::{
    CHECKPOINT_INTERVAL, Destination, NEW_DIR_MODE, Occupant, OnConflict, Placer, Source, Step, TO,
    choice, destination, directory_exists_error, effect_output, exists_error, flag,
    into_itself_error, kind_mismatch_error, missing_state, on_conflict_param,
    require_shared_directory, run_steps, same_entry_error, sources, unknown_value, walk_tree,
};
use crate::common::mutation::fs_target;
use crate::common::path::PATH_FIELD;

const COMMAND: &str = "copy";
const FROM: &str = "from";
const PRESERVE: &str = "preserve";
const CLONE: &str = "clone";
const CLONE_VALUES: [&str; 3] = ["auto", "always", "never"];

/// Permission bits kept by `preserve`, including the set-id and sticky bits.
const PERMISSION_BITS: u32 = 0o7777;

/// Stage of `copy`.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Copy;

/// Stages of this module.
pub(crate) fn stages() -> Vec<Arc<dyn StageImpl>> {
    vec![Arc::new(Copy)]
}

impl StageImpl for Copy {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: COMMAND.to_owned(),
            source: true,
            input: InputKinds::Only(vec![RecordKind::Entry]),
        }
    }

    fn as_effect(&self) -> Option<&dyn EffectStage> {
        Some(self)
    }
}

/// Settings of one call.
#[derive(Debug, Clone, Copy)]
struct Settings {
    on_conflict: OnConflict,
    preserve: bool,
    clone: CloneMode,
}

impl Settings {
    fn read(args: &BoundArgs) -> Result<Self> {
        let clone = match choice(args, CLONE)? {
            None | Some("auto") => CloneMode::Auto,
            Some("always") => CloneMode::Always,
            Some("never") => CloneMode::Never,
            Some(other) => return Err(unknown_value(CLONE, other, &CLONE_VALUES)),
        };
        Ok(Self {
            on_conflict: on_conflict_param(args, OnConflict::Overwrite)?,
            preserve: flag(args, PRESERVE, true)?,
            clone,
        })
    }
}

/// Counters of the `effect` record.
#[derive(Debug, Clone, Copy, Default)]
struct Summary {
    files: u64,
    directories: u64,
    links: u64,
    overwritten: u64,
    skipped: u64,
}

impl Summary {
    fn pairs(self) -> [(&'static str, u64); 5] {
        [
            ("files", self.files),
            ("directories", self.directories),
            ("links", self.links),
            ("overwritten", self.overwritten),
            ("skipped", self.skipped),
        ]
    }
}

/// Private state of the plan.
#[derive(Debug)]
struct CopyState {
    steps: Vec<Step>,
    summary: Summary,
}

impl EffectStage for Copy {
    fn plan<'a>(
        &'a self,
        args: &'a BoundArgs,
        input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<EffectPlan> {
        let fs = ctx.require_fs()?;
        let settings = Settings::read(args)?;
        let sources = sources(args, input, ctx, FROM)?;
        let shared = sources.len() > 1 && args.expr(TO).is_none();
        let mut planner = Planner {
            fs,
            ctx,
            settings,
            placer: Placer::new(fs),
            steps: Vec::new(),
            summary: Summary::default(),
        };
        for source in &sources {
            let destination = destination(args, source.record.as_ref(), ctx)?;
            if shared {
                require_shared_directory(&planner.placer, &destination, COMMAND)?;
            }
            planner.copy_root(source, &destination)?;
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
        let fs = ctx.require_fs()?;
        let state = plan
            .take_state::<CopyState>()
            .ok_or_else(|| missing_state(COMMAND))?;
        let changes = run_steps(tx, state.steps, fs)?;
        let output = effect_output(COMMAND, tx, &changes, &plan.notable, &state.summary.pairs());
        let mut outcome = EffectOutcome::new(output);
        outcome.changes = changes;
        Ok(outcome)
    }
}

struct Planner<'a> {
    fs: &'a dyn Fs,
    ctx: &'a ExecCtx,
    settings: Settings,
    placer: Placer<'a>,
    steps: Vec<Step>,
    summary: Summary,
}

impl Planner<'_> {
    fn copy_root(&mut self, source: &Source, destination: &Destination) -> Result<()> {
        let meta = source.require(self.fs)?;
        let target = self.placer.place(&source.path, destination)?.path;
        if meta.kind == FileKind::Directory && target.starts_with(&source.path) {
            return Err(into_itself_error(&source.path, &target));
        }
        self.check_clone(&source.path, &target)?;
        self.placer.claim(&target, &source.path)?;
        if meta.kind == FileKind::Directory {
            self.copy_tree(&source.path, &meta, target)
        } else {
            let occupant = self.placer.occupant(&meta, &target)?;
            self.copy_entry(&source.path, &meta, target, occupant)
        }
    }

    fn check_clone(&self, source: &AbsPath, target: &AbsPath) -> Result<()> {
        if self.settings.clone != CloneMode::Always {
            return Ok(());
        }
        let supported =
            self.fs.volume(source)?.features.reflink && self.fs.same_volume(source, target)?;
        if supported {
            return Ok(());
        }
        Err(Error::new(
            ErrorCode::Unsupported,
            format!(
                "`clone: always` needs block clones from `{source}` to `{target}`, which the volume does not offer: use `clone: auto`"
            ),
        )
        .with_detail(PATH_FIELD, Json::from(source.to_language_string())))
    }

    fn copy_tree(&mut self, root: &AbsPath, meta: &FileMeta, target: AbsPath) -> Result<()> {
        let mut fresh = HashSet::new();
        match self.placer.occupant(meta, &target)? {
            Occupant::Free => {
                self.create_dir(&target, meta);
                fresh.insert(target.clone());
            }
            Occupant::Directory(_) => {}
            Occupant::Same(_) => return Err(same_entry_error(root)),
            Occupant::Other(_) => return Err(kind_mismatch_error(&target)),
        }
        for (index, entry) in walk_tree(self.fs, self.ctx, root)?.into_iter().enumerate() {
            if index % CHECKPOINT_INTERVAL == 0 {
                self.ctx.checkpoint()?;
            }
            let destination = AbsPath::new(target.as_path().join(&entry.relative))?;
            let meta = match entry.meta {
                Some(meta) => meta,
                None => self.fs.metadata(&entry.path, Follow::NoFollow)?,
            };
            self.placer.claim(&destination, &entry.path)?;
            let occupant = if destination
                .parent()
                .is_some_and(|parent| fresh.contains(&parent))
            {
                Occupant::Free
            } else {
                self.placer.occupant(&meta, &destination)?
            };
            if meta.kind != FileKind::Directory {
                self.copy_entry(&entry.path, &meta, destination, occupant)?;
                continue;
            }
            match occupant {
                Occupant::Free => {
                    self.create_dir(&destination, &meta);
                    fresh.insert(destination);
                }
                Occupant::Directory(_) => {}
                Occupant::Same(_) => return Err(same_entry_error(&entry.path)),
                Occupant::Other(_) => return Err(kind_mismatch_error(&destination)),
            }
        }
        Ok(())
    }

    fn create_dir(&mut self, target: &AbsPath, source: &FileMeta) {
        let mode = if self.settings.preserve {
            source.mode & PERMISSION_BITS
        } else {
            NEW_DIR_MODE
        };
        self.steps
            .push(Step::op(FileOp::create_dir(target.clone(), mode)));
        self.summary.directories += 1;
    }

    fn copy_entry(
        &mut self,
        source: &AbsPath,
        meta: &FileMeta,
        target: AbsPath,
        occupant: Occupant,
    ) -> Result<()> {
        let (mutation, created) = match meta.kind {
            FileKind::File => (
                Mutation::CopyFrom {
                    source: source.clone(),
                    clone: self.settings.clone,
                },
                TargetAction::Create,
            ),
            FileKind::Symlink | FileKind::Junction => {
                (self.link_of(source, meta)?, TargetAction::Link)
            }
            _ => return Err(special_error(source)),
        };
        let link = created == TargetAction::Link;
        let step = match occupant {
            Occupant::Free => {
                Step::new(fs_target(&target, created, Expect::Absent), vec![mutation])
            }
            Occupant::Same(_) => return Err(same_entry_error(source)),
            Occupant::Directory(_) => return Err(directory_exists_error(&target)),
            Occupant::Other(existing) => match self.settings.on_conflict {
                OnConflict::Overwrite => {
                    self.summary.overwritten += 1;
                    overwrite(&target, &existing, mutation)
                }
                OnConflict::Skip => {
                    self.summary.skipped += 1;
                    return Ok(());
                }
                OnConflict::Error => return Err(exists_error(&target)),
            },
        };
        if link {
            self.summary.links += 1;
        } else {
            self.summary.files += 1;
        }
        self.steps.push(step.with_source(source));
        Ok(())
    }

    fn link_of(&self, source: &AbsPath, meta: &FileMeta) -> Result<Mutation> {
        let target = self.fs.read_link(source)?;
        let kind = if meta.kind == FileKind::Junction {
            LinkKind::Junction
        } else {
            match self.fs.metadata(source, Follow::Follow) {
                Ok(resolved) if resolved.kind == FileKind::Directory => LinkKind::SymlinkDirectory,
                _ => LinkKind::SymlinkFile,
            }
        };
        Ok(Mutation::CreateLink { target, kind })
    }

    fn finish(self) -> EffectPlan {
        let (mut steps, mut notable) = self.placer.finish();
        steps.extend(self.steps);
        if steps.is_empty() && self.summary.skipped == 0 {
            notable.push(NotableEntry::new(Notable::NothingMatched));
        }
        let changes = steps.iter().map(Step::change).collect();
        let bulk = steps.len() > 1;
        let mut plan = EffectPlan::new().with_state(CopyState {
            steps,
            summary: self.summary,
        });
        plan.changes = changes;
        plan.notable = notable;
        plan.bulk = bulk;
        plan
    }
}

/// Step that replaces an existing file or link: a file copied over a file
/// replaces its content, anything else replaces the entry as a whole.
fn overwrite(target: &AbsPath, existing: &FileMeta, mutation: Mutation) -> Step {
    let expect = Expect::Etag(existing.etag());
    match mutation {
        Mutation::CopyFrom { source, clone } if existing.kind == FileKind::File => Step::op(
            FileOp::copy_over(source, target.clone(), clone)
                .expect(expect)
                .size_hint(existing.size),
        ),
        mutation => {
            let mut declaration = fs_target(target, TargetAction::Replace, expect);
            declaration.size_hint = Some(existing.size);
            Step::new(declaration, vec![Mutation::Remove, mutation])
        }
    }
}

fn special_error(path: &AbsPath) -> Error {
    Error::new(
        ErrorCode::Unsupported,
        format!(
            "`{path}` is a device, pipe or socket and is not copied: exclude it from the sources"
        ),
    )
    .with_detail(PATH_FIELD, Json::from(path.to_language_string()))
}

#[cfg(test)]
mod tests {
    use nurt_contracts::record::ChangeAction;
    use nurt_contracts::stage::{ArgValue, StageImpl};
    use nurt_contracts::value::Value;
    use nurt_testkit::ctx::TestContext;
    use nurt_testkit::vfs::VolumeConfig;

    use super::super::conflict::testing::{
        abs, assert_journaled, assert_untouched, choice_arg, dir, entry, file, path_arg, paths_arg,
        run,
    };
    use super::super::conflict::{INTO, ON_CONFLICT as ON_CONFLICT_PARAM};
    use super::*;

    fn args(from: &[&AbsPath]) -> BoundArgs {
        BoundArgs::new(COMMAND).with(FROM, paths_arg(from), false)
    }

    fn to(args: BoundArgs, path: &AbsPath) -> BoundArgs {
        args.with(TO, path_arg(path), false)
    }

    fn into(args: BoundArgs, path: &AbsPath) -> BoundArgs {
        args.with(INTO, path_arg(path), false)
    }

    fn with(args: BoundArgs, name: &str, value: ArgValue) -> BoundArgs {
        args.with(name, value, false)
    }

    #[test]
    fn descriptor_names_the_command_and_its_input() {
        let descriptor = Copy.descriptor();
        assert_eq!(descriptor.command, "copy");
        assert!(descriptor.source);
        assert!(descriptor.input.accepts(RecordKind::Entry));
        assert!(Copy.as_effect().is_some());
    }

    #[test]
    fn copies_a_file_to_a_new_path_through_the_transaction() {
        let test = TestContext::new();
        let source = file(&test, "/data/a.txt", b"alpha");
        let target = abs("/data/b.txt");
        let run = run(&Copy, &test, &to(args(&[&source]), &target), None).unwrap();
        assert_eq!(test.fs().read_file(&target).unwrap(), b"alpha");
        assert_eq!(test.fs().read_file(&source).unwrap(), b"alpha");
        assert_eq!(run.changes.len(), 1);
        assert_eq!(run.changes[0].action, ChangeAction::Create);
        assert_eq!(
            run.changes[0]
                .from
                .as_ref()
                .map(|path| path.as_str().to_owned()),
            Some(source.to_language_string())
        );
        assert_eq!(run.planned.len(), 1);
        assert_eq!(run.summary("files"), Some(1));
        assert_journaled(&test);
    }

    #[test]
    fn to_an_existing_directory_copies_inside_it() {
        let test = TestContext::new();
        let source = file(&test, "/data/a.txt", b"a");
        let target = dir(&test, "/out");
        let run = run(&Copy, &test, &to(args(&[&source]), &target), None).unwrap();
        assert!(test.fs().exists(&abs("/out/a.txt")));
        assert_eq!(run.codes(), [Notable::ToIsDir]);
    }

    #[test]
    fn conflict_matrix_for_an_existing_file() {
        let cases = [
            ("overwrite", Some(b"new".as_slice()), None),
            ("skip", Some(b"old".as_slice()), None),
            ("error", None, Some(ErrorCode::Exists)),
        ];
        for (value, content, error) in cases {
            let test = TestContext::new();
            let source = file(&test, "/data/a.txt", b"new");
            let target = file(&test, "/data/b.txt", b"old");
            let call = with(
                to(args(&[&source]), &target),
                ON_CONFLICT_PARAM,
                choice_arg(value),
            );
            let result = run(&Copy, &test, &call, None);
            match (content, error) {
                (Some(content), _) => {
                    result.unwrap();
                    assert_eq!(test.fs().read_file(&target).unwrap(), content, "{value}");
                }
                (None, Some(code)) => {
                    assert_eq!(result.unwrap_err().code(), code, "{value}");
                    assert_eq!(test.fs().read_file(&target).unwrap(), b"old");
                    assert_untouched(&test);
                }
                (None, None) => unreachable!("every case expects content or an error"),
            }
        }
    }

    #[test]
    fn an_existing_directory_is_never_replaced_by_a_file() {
        let test = TestContext::new();
        let source = file(&test, "/data/a.txt", b"a");
        let blocker = dir(&test, "/out/a.txt");
        for value in ["overwrite", "skip", "error"] {
            let call = with(
                to(args(&[&source]), &abs("/out")),
                ON_CONFLICT_PARAM,
                choice_arg(value),
            );
            let error = run(&Copy, &test, &call, None).unwrap_err();
            assert_eq!(error.code(), ErrorCode::Exists, "{value}");
            assert_eq!(error.problem.as_deref(), Some("directory_exists"));
        }
        assert!(test.fs().exists(&blocker));
    }

    #[test]
    fn several_sources_need_a_directory() {
        let test = TestContext::new();
        let first = file(&test, "/data/a.txt", b"a");
        let second = file(&test, "/data/b.txt", b"b");
        let error = run(
            &Copy,
            &test,
            &to(args(&[&first, &second]), &abs("/out")),
            None,
        )
        .unwrap_err();
        assert_eq!(error.code(), ErrorCode::ParamConflict);
        let run = run(
            &Copy,
            &test,
            &into(args(&[&first, &second]), &abs("/out/new")),
            None,
        )
        .unwrap();
        assert!(test.fs().exists(&abs("/out/new/a.txt")));
        assert!(test.fs().exists(&abs("/out/new/b.txt")));
        let created: Vec<ChangeAction> = run.changes.iter().map(|change| change.action).collect();
        assert_eq!(created.len(), 4);
        assert_journaled(&test);
    }

    #[test]
    fn copies_a_tree_with_links_and_merges_into_an_existing_directory() {
        let test = TestContext::new();
        file(&test, "/src/tree/a.txt", b"a");
        file(&test, "/src/tree/sub/b.txt", b"b");
        test.fs().symlink("a.txt", &abs("/src/tree/link")).unwrap();
        file(&test, "/out/tree/keep.txt", b"k");
        let call = into(args(&[&abs("/src/tree")]), &abs("/out"));
        let run = run(&Copy, &test, &call, None).unwrap();
        assert_eq!(
            test.fs().read_file(&abs("/out/tree/sub/b.txt")).unwrap(),
            b"b"
        );
        assert!(test.fs().exists(&abs("/out/tree/keep.txt")));
        let link = test.fs().read_link(&abs("/out/tree/link")).unwrap();
        assert_eq!(link, std::path::PathBuf::from("a.txt"));
        assert_eq!(run.summary("links"), Some(1));
        assert_eq!(run.summary("directories"), Some(1));
        assert_eq!(run.summary("files"), Some(2));
        assert_journaled(&test);
    }

    #[test]
    fn a_directory_cannot_be_copied_into_itself() {
        let test = TestContext::new();
        let source = dir(&test, "/src/tree");
        let error = run(
            &Copy,
            &test,
            &into(args(&[&source]), &abs("/src/tree/inner")),
            None,
        )
        .unwrap_err();
        assert_eq!(error.code(), ErrorCode::InvalidValue);
        let file_source = file(&test, "/src/a.txt", b"a");
        let error = run(&Copy, &test, &to(args(&[&file_source]), &file_source), None).unwrap_err();
        assert_eq!(error.code(), ErrorCode::InvalidValue);
    }

    #[test]
    fn clone_always_without_block_clones_is_rejected() {
        let test = TestContext::new();
        let source = file(&test, "/data/a.txt", b"a");
        let call = with(
            to(args(&[&source]), &abs("/data/b.txt")),
            CLONE,
            choice_arg("always"),
        );
        let error = run(&Copy, &test, &call, None).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Unsupported);
        assert!(error.message.contains("clone: auto"));
        let mut config = VolumeConfig::default();
        config.features.reflink = true;
        test.fs().add_volume(&abs("/fast"), config).unwrap();
        let source = file(&test, "/fast/a.txt", b"a");
        let call = with(
            to(args(&[&source]), &abs("/fast/b.txt")),
            CLONE,
            choice_arg("always"),
        );
        run(&Copy, &test, &call, None).unwrap();
        assert!(test.fs().exists(&abs("/fast/b.txt")));
    }

    #[test]
    fn preserve_keeps_the_mode_of_directories() {
        let test = TestContext::new();
        let source = dir(&test, "/src/private");
        test.fs().set_mode(&source, 0o700).unwrap();
        run(&Copy, &test, &to(args(&[&source]), &abs("/kept")), None).unwrap();
        let kept = test.fs().metadata(&abs("/kept"), Follow::NoFollow).unwrap();
        assert_eq!(kept.mode & PERMISSION_BITS, 0o700);
        let call = with(
            to(args(&[&source]), &abs("/plain")),
            PRESERVE,
            ArgValue::Value(Value::Bool(false)),
        );
        run(&Copy, &test, &call, None).unwrap();
        let plain = test
            .fs()
            .metadata(&abs("/plain"), Follow::NoFollow)
            .unwrap();
        assert_eq!(plain.mode & PERMISSION_BITS, NEW_DIR_MODE);
    }

    #[test]
    fn two_sources_with_one_name_cannot_share_a_destination() {
        let test = TestContext::new();
        let first = file(&test, "/a/x.txt", b"1");
        let second = file(&test, "/b/x.txt", b"2");
        let call = into(args(&[&first, &second]), &abs("/out"));
        let error = run(&Copy, &test, &call, None).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Exists);
        assert_eq!(error.problem.as_deref(), Some("duplicate_destination"));
        assert_untouched(&test);
    }

    #[test]
    fn on_conflict_skip_keeps_the_existing_files_of_a_merged_tree() {
        let test = TestContext::new();
        file(&test, "/src/tree/a.txt", b"new-a");
        file(&test, "/src/tree/b.txt", b"new-b");
        file(&test, "/out/tree/a.txt", b"old-a");
        let call = with(
            into(args(&[&abs("/src/tree")]), &abs("/out")),
            ON_CONFLICT_PARAM,
            choice_arg("skip"),
        );
        let run = run(&Copy, &test, &call, None).unwrap();
        assert_eq!(test.fs().read_file(&abs("/out/tree/a.txt")).unwrap(), b"old-a");
        assert_eq!(test.fs().read_file(&abs("/out/tree/b.txt")).unwrap(), b"new-b");
        assert_eq!(run.summary("skipped"), Some(1));
        assert_eq!(run.summary("files"), Some(1));
        assert_eq!(run.summary("directories"), Some(0));
    }

    #[test]
    fn a_directory_is_never_copied_over_a_file() {
        let test = TestContext::new();
        file(&test, "/src/tree/a.txt", b"a");
        let blocker = file(&test, "/out/blocker", b"file");
        let error = run(
            &Copy,
            &test,
            &to(args(&[&abs("/src/tree")]), &blocker),
            None,
        )
        .unwrap_err();
        assert_eq!(error.code(), ErrorCode::Exists);
        assert_eq!(error.problem.as_deref(), Some("kind_mismatch"));
        assert_eq!(test.fs().read_file(&blocker).unwrap(), b"file");
        assert_untouched(&test);
    }

    #[test]
    fn entries_with_an_old_etag_are_stale_and_nothing_changes() {
        let test = TestContext::new();
        let source = file(&test, "/data/a.txt", b"a");
        let tag = test
            .fs()
            .metadata(&source, Follow::NoFollow)
            .unwrap()
            .etag();
        test.fs().write_file(&source, b"changed").unwrap();
        let record = entry(&source).with("etag", Value::Text(crate::common::etag::etag_text(&tag)));
        let call = to(BoundArgs::new(COMMAND), &abs("/data/b.txt"));
        let error = run(&Copy, &test, &call, Some(vec![record])).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Stale);
        assert!(!test.fs().exists(&abs("/data/b.txt")));
        assert_untouched(&test);
    }

    #[test]
    fn a_missing_source_is_not_found() {
        let test = TestContext::new();
        let call = to(args(&[&abs("/data/none")]), &abs("/data/b"));
        assert_eq!(
            run(&Copy, &test, &call, None).unwrap_err().code(),
            ErrorCode::NotFound
        );
        let empty = to(BoundArgs::new(COMMAND), &abs("/data/b"));
        let run = run(&Copy, &test, &empty, Some(Vec::new())).unwrap();
        assert_eq!(run.codes(), [Notable::NothingMatched]);
    }

    #[test]
    fn a_link_over_an_existing_file_replaces_the_entry() {
        let test = TestContext::new();
        test.fs().create_dir_all(&abs("/src")).unwrap();
        test.fs().symlink("target", &abs("/src/link")).unwrap();
        let target = file(&test, "/out/link", b"file");
        let run = run(&Copy, &test, &to(args(&[&abs("/src/link")]), &target), None).unwrap();
        assert_eq!(
            test.fs().read_link(&target).unwrap(),
            std::path::PathBuf::from("target")
        );
        assert_eq!(run.summary("overwritten"), Some(1));
        assert_journaled(&test);
    }

    #[test]
    fn copies_ten_thousand_files_as_one_operation() {
        let test = TestContext::new();
        let directories = 100_i64;
        let files = 100_i64;
        for outer in 0..directories {
            test.fs()
                .create_dir_all(&abs(&format!("/big/d{outer}")))
                .unwrap();
            for inner in 0..files {
                test.fs()
                    .write_file(&abs(&format!("/big/d{outer}/f{inner}")), b"x")
                    .unwrap();
            }
        }
        let run = run(
            &Copy,
            &test,
            &to(args(&[&abs("/big")]), &abs("/copy")),
            None,
        )
        .unwrap();
        assert_eq!(run.summary("files"), Some(directories * files));
        assert_eq!(run.summary("directories"), Some(directories + 1));
        assert_eq!(run.changes.len(), run.planned.len());
        assert!(test.fs().exists(&abs("/copy/d99/f99")));
        assert_journaled(&test);
    }
}
