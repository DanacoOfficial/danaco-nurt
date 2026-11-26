// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! `move`: moves files, links and directory trees.
//!
//! Every moved entry is one target of the transaction, declared as a rename
//! to its destination. The journal renames within a volume and copies and
//! deletes between volumes; the plan reports the latter as `cross_device`.
//! A directory tree is moved as a whole.

use std::sync::Arc;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::{AbsPath, FileKind, Fs};
use nurt_contracts::journal::{Expect, FileOp, MutationTx};
use nurt_contracts::report::{Notable, NotableEntry};
use nurt_contracts::stage::{
    BoundArgs, EffectOutcome, EffectPlan, EffectStage, ExecCtx, InputKinds, StageDescriptor,
    StageImpl, StageInput,
};
use nurt_contracts::value::RecordKind;
use serde_json::Value as Json;

use super::conflict::{
    Destination, Occupant, OnConflict, Placer, Source, Step, TO, destination,
    directory_exists_error, effect_output, exists_error, into_itself_error, is_case_variant,
    kind_mismatch_error, missing_state, on_conflict_param, require_shared_directory, run_steps,
    same_entry_error, sources,
};
use crate::common::path::PATH_FIELD;

const COMMAND: &str = "move";
const FROM: &str = "from";

/// Stage of `move`.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Move;

/// Stages of this module.
pub(crate) fn stages() -> Vec<Arc<dyn StageImpl>> {
    vec![Arc::new(Move)]
}

impl StageImpl for Move {
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

/// Counters of the `effect` record.
#[derive(Debug, Clone, Copy, Default)]
struct Summary {
    moved: u64,
    overwritten: u64,
    skipped: u64,
    cross_device: u64,
}

impl Summary {
    fn pairs(self) -> [(&'static str, u64); 4] {
        [
            ("moved", self.moved),
            ("overwritten", self.overwritten),
            ("skipped", self.skipped),
            ("cross_device", self.cross_device),
        ]
    }
}

/// Private state of the plan.
#[derive(Debug)]
struct MoveState {
    steps: Vec<Step>,
    summary: Summary,
}

impl EffectStage for Move {
    fn plan<'a>(
        &'a self,
        args: &'a BoundArgs,
        input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<EffectPlan> {
        let fs = ctx.require_fs()?;
        let on_conflict = on_conflict_param(args, OnConflict::Overwrite)?;
        let sources = sources(args, input, ctx, FROM)?;
        reject_nested(&sources)?;
        let shared = sources.len() > 1 && args.expr(TO).is_none();
        let mut planner = Planner {
            fs,
            on_conflict,
            placer: Placer::new(fs),
            steps: Vec::new(),
            summary: Summary::default(),
        };
        for (index, source) in sources.iter().enumerate() {
            if index % super::conflict::CHECKPOINT_INTERVAL == 0 {
                ctx.checkpoint()?;
            }
            let destination = destination(args, source.record.as_ref(), ctx)?;
            if shared {
                require_shared_directory(&planner.placer, &destination, COMMAND)?;
            }
            planner.move_entry(source, &destination)?;
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
            .take_state::<MoveState>()
            .ok_or_else(|| missing_state(COMMAND))?;
        let changes = run_steps(tx, state.steps, fs)?;
        let output = effect_output(COMMAND, tx, &changes, &plan.notable, &state.summary.pairs());
        let mut outcome = EffectOutcome::new(output);
        outcome.changes = changes;
        Ok(outcome)
    }
}

/// Rejects a source that lies inside another source, which would no longer
/// exist where it was read once the enclosing directory is moved.
fn reject_nested(sources: &[Source]) -> Result<()> {
    let mut paths: Vec<&AbsPath> = sources.iter().map(|source| &source.path).collect();
    paths.sort();
    for pair in paths.windows(2) {
        if let [outer, inner] = pair
            && inner.starts_with(outer)
            && inner != outer
        {
            return Err(Error::new(
                ErrorCode::InvalidValue,
                format!(
                    "`{inner}` lies inside `{outer}`, which is moved as well: move only `{outer}`"
                ),
            )
            .with_detail(PATH_FIELD, Json::from(inner.to_language_string())));
        }
    }
    Ok(())
}

struct Planner<'a> {
    fs: &'a dyn Fs,
    on_conflict: OnConflict,
    placer: Placer<'a>,
    steps: Vec<Step>,
    summary: Summary,
}

impl Planner<'_> {
    fn move_entry(&mut self, source: &Source, destination: &Destination) -> Result<()> {
        let meta = source.require(self.fs)?;
        let target = self.placer.place(&source.path, destination)?.path;
        if target == source.path {
            return Err(same_entry_error(&source.path));
        }
        if meta.kind == FileKind::Directory && target.starts_with(&source.path) {
            return Err(into_itself_error(&source.path, &target));
        }
        self.placer.claim(&target, &source.path)?;
        let replace = match self.placer.occupant(&meta, &target)? {
            Occupant::Free => false,
            Occupant::Same(_) if is_case_variant(&source.path, &target) => false,
            Occupant::Directory(_) => return Err(directory_exists_error(&target)),
            Occupant::Same(_) | Occupant::Other(_) if meta.kind == FileKind::Directory => {
                return Err(kind_mismatch_error(&target));
            }
            Occupant::Same(_) | Occupant::Other(_) => match self.on_conflict {
                OnConflict::Overwrite => {
                    self.summary.overwritten += 1;
                    true
                }
                OnConflict::Skip => {
                    self.summary.skipped += 1;
                    return Ok(());
                }
                OnConflict::Error => return Err(exists_error(&target)),
            },
        };
        if !self.fs.same_volume(&source.path, &target)? {
            self.summary.cross_device += 1;
            self.placer.note(
                Notable::CrossDevice,
                &source.path,
                Some(target.to_language_string()),
            );
        }
        let op = FileOp::rename(source.path.clone(), target, replace)
            .expect(Expect::Etag(meta.etag()))
            .size_hint(meta.size);
        self.steps.push(Step::op(op));
        self.summary.moved += 1;
        Ok(())
    }

    fn finish(self) -> EffectPlan {
        let (mut steps, mut notable) = self.placer.finish();
        steps.extend(self.steps);
        if steps.is_empty() && self.summary.skipped == 0 {
            notable.push(NotableEntry::new(Notable::NothingMatched));
        }
        let changes = steps.iter().map(Step::change).collect();
        let bulk = steps.len() > 1;
        let mut plan = EffectPlan::new().with_state(MoveState {
            steps,
            summary: self.summary,
        });
        plan.changes = changes;
        plan.notable = notable;
        plan.bulk = bulk;
        plan
    }
}

#[cfg(test)]
mod tests {
    use nurt_contracts::fs::Follow;
    use nurt_contracts::record::ChangeAction;
    use nurt_contracts::value::Value;
    use nurt_testkit::ctx::TestContext;
    use nurt_testkit::vfs::VolumeConfig;

    use super::super::conflict::testing::{
        abs, assert_journaled, assert_untouched, choice_arg, dir, entry, file, path_arg, paths_arg,
        run,
    };
    use super::super::conflict::{INTO, ON_CONFLICT};
    use super::*;

    fn args(from: &[&AbsPath]) -> BoundArgs {
        BoundArgs::new(COMMAND).with(FROM, paths_arg(from), false)
    }

    fn to(args: BoundArgs, path: &AbsPath) -> BoundArgs {
        args.with(TO, path_arg(path), false)
    }

    #[test]
    fn moves_a_file_through_one_rename() {
        let test = TestContext::new();
        let source = file(&test, "/data/a.txt", b"alpha");
        let target = abs("/data/b.txt");
        let run = run(&Move, &test, &to(args(&[&source]), &target), None).unwrap();
        assert!(!test.fs().exists(&source));
        assert_eq!(test.fs().read_file(&target).unwrap(), b"alpha");
        assert_eq!(run.changes[0].action, ChangeAction::Rename);
        assert_eq!(run.changes[0].path.as_str(), source.to_language_string());
        assert_eq!(
            run.changes[0]
                .to
                .as_ref()
                .map(|path| path.as_str().to_owned()),
            Some(target.to_language_string())
        );
        assert!(run.codes().is_empty());
        assert_journaled(&test);
    }

    #[test]
    fn every_rename_change_gives_the_previous_path() {
        let test = TestContext::new();
        let first = file(&test, "/data/a.txt", b"a");
        let second = dir(&test, "/data/sub");
        file(&test, "/data/sub/inner.txt", b"i");
        let call = args(&[&first, &second]).with(INTO, path_arg(&abs("/out")), false);
        let run = run(&Move, &test, &call, None).unwrap();
        let renames: Vec<_> = run
            .changes
            .iter()
            .filter(|change| change.action == ChangeAction::Rename)
            .collect();
        assert_eq!(renames.len(), 2);
        for change in renames {
            assert_eq!(
                change.from.as_ref().map(|path| path.as_str().to_owned()),
                Some(change.path.as_str().to_owned())
            );
        }
    }

    #[test]
    fn a_directory_tree_is_moved_as_one_rename() {
        let test = TestContext::new();
        let tree = dir(&test, "/data/tree");
        file(&test, "/data/tree/sub/a.txt", b"a");
        let target = abs("/data/moved");
        let run = run(&Move, &test, &to(args(&[&tree]), &target), None).unwrap();
        assert_eq!(run.changes.len(), 1);
        assert_eq!(
            test.fs().read_file(&abs("/data/moved/sub/a.txt")).unwrap(),
            b"a"
        );
        assert!(!test.fs().exists(&tree));
        assert_eq!(run.summary("moved"), Some(1));
        assert_journaled(&test);
    }

    #[test]
    fn to_an_existing_directory_moves_inside_it_and_is_notable() {
        let test = TestContext::new();
        let source = file(&test, "/data/a.txt", b"a");
        let target = dir(&test, "/out");
        let run = run(&Move, &test, &to(args(&[&source]), &target), None).unwrap();
        assert!(test.fs().exists(&abs("/out/a.txt")));
        assert_eq!(run.codes(), [Notable::ToIsDir]);
    }

    #[test]
    fn empty_input_moves_nothing() {
        let test = TestContext::new();
        let call = to(BoundArgs::new(COMMAND), &abs("/data/b"));
        let run = run(&Move, &test, &call, Some(Vec::new())).unwrap();
        assert_eq!(run.codes(), [Notable::NothingMatched]);
        assert!(run.changes.is_empty());
    }

    #[test]
    fn sources_with_the_same_name_cannot_share_a_destination() {
        let test = TestContext::new();
        let first = file(&test, "/a/x.txt", b"1");
        let second = file(&test, "/b/x.txt", b"2");
        let call = args(&[&first, &second]).with(INTO, path_arg(&abs("/out")), false);
        let error = run(&Move, &test, &call, None).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Exists);
        assert_eq!(error.problem.as_deref(), Some("duplicate_destination"));
        assert!(test.fs().exists(&first));
        assert_untouched(&test);
    }

    #[test]
    fn conflict_matrix_for_files_directories_and_free_destinations() {
        let cases = [
            ("overwrite", "file", Ok(b"new".as_slice())),
            ("skip", "file", Ok(b"old".as_slice())),
            ("error", "file", Err(ErrorCode::Exists)),
            ("overwrite", "directory", Err(ErrorCode::Exists)),
            ("error", "none", Ok(b"new".as_slice())),
        ];
        for (value, occupant, expected) in cases {
            let test = TestContext::new();
            let source = file(&test, "/data/a.txt", b"new");
            let target = abs("/data/b.txt");
            match occupant {
                "file" => {
                    test.fs().write_file(&target, b"old").unwrap();
                }
                "directory" => {
                    test.fs()
                        .create_dir_all(&abs("/data/b.txt/a.txt/x"))
                        .unwrap();
                }
                _ => {}
            }
            let call = to(args(&[&source]), &target).with(ON_CONFLICT, choice_arg(value), false);
            match (run(&Move, &test, &call, None), expected) {
                (Ok(_), Ok(content)) => {
                    assert_eq!(
                        test.fs().read_file(&target).unwrap(),
                        content,
                        "{value} {occupant}"
                    );
                    assert_eq!(test.fs().exists(&source), value == "skip");
                }
                (Err(error), Err(code)) => {
                    assert_eq!(error.code(), code, "{value} {occupant}");
                    assert!(test.fs().exists(&source));
                    assert_untouched(&test);
                }
                (result, expected) => panic!("{value} {occupant}: {result:?} {expected:?}"),
            }
        }
    }

    #[test]
    fn several_sources_go_into_a_created_directory() {
        let test = TestContext::new();
        let first = file(&test, "/data/a.txt", b"a");
        let second = dir(&test, "/data/sub");
        file(&test, "/data/sub/inner.txt", b"i");
        let call = args(&[&first, &second]).with(INTO, path_arg(&abs("/out/new")), false);
        let moved = run(&Move, &test, &call, None).unwrap();
        assert!(test.fs().exists(&abs("/out/new/a.txt")));
        assert!(test.fs().exists(&abs("/out/new/sub/inner.txt")));
        assert!(!test.fs().exists(&second));
        assert_eq!(moved.summary("moved"), Some(2));
        assert_journaled(&test);
        let test = TestContext::new();
        let first = file(&test, "/data/a.txt", b"a");
        let second = file(&test, "/data/b.txt", b"b");
        let error = run(
            &Move,
            &test,
            &to(args(&[&first, &second]), &abs("/data/c")),
            None,
        )
        .unwrap_err();
        assert_eq!(error.code(), ErrorCode::ParamConflict);
    }

    #[test]
    fn moving_between_volumes_is_reported_as_cross_device() {
        let test = TestContext::new();
        test.fs()
            .add_volume(&abs("/mnt"), VolumeConfig::default())
            .unwrap();
        let source = file(&test, "/data/a.txt", b"alpha");
        let target = abs("/mnt/a.txt");
        let run = run(&Move, &test, &to(args(&[&source]), &target), None).unwrap();
        assert_eq!(test.fs().read_file(&target).unwrap(), b"alpha");
        assert!(!test.fs().exists(&source));
        assert_eq!(run.codes(), [Notable::CrossDevice]);
        assert_eq!(run.summary("cross_device"), Some(1));
        assert_journaled(&test);
    }

    #[test]
    fn a_case_only_move_renames_the_entry_itself() {
        let test = TestContext::new();
        let mut config = VolumeConfig::default();
        config.features.case_sensitive = false;
        test.fs().add_volume(&abs("/ci"), config).unwrap();
        let source = file(&test, "/ci/readme.md", b"r");
        let target = abs("/ci/README.md");
        run(&Move, &test, &to(args(&[&source]), &target), None).unwrap();
        let names: Vec<String> = test
            .fs()
            .read_dir(&abs("/ci"))
            .unwrap()
            .map(|item| item.unwrap().name.to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["README.md"]);
        assert_journaled(&test);
    }

    #[test]
    fn directories_cannot_move_into_themselves_or_with_their_content() {
        let test = TestContext::new();
        let tree = dir(&test, "/data/tree");
        let inner = file(&test, "/data/tree/a.txt", b"a");
        let into_self = to(args(&[&tree]), &abs("/data/tree/sub"));
        assert_eq!(
            run(&Move, &test, &into_self, None).unwrap_err().code(),
            ErrorCode::InvalidValue
        );
        let into_existing = to(args(&[&tree]), &tree);
        assert_eq!(
            run(&Move, &test, &into_existing, None).unwrap_err().code(),
            ErrorCode::InvalidValue
        );
        let nested = args(&[&tree, &inner]).with(INTO, path_arg(&abs("/out")), false);
        assert_eq!(
            run(&Move, &test, &nested, None).unwrap_err().code(),
            ErrorCode::InvalidValue
        );
        assert_untouched(&test);
    }

    #[test]
    fn a_link_is_moved_as_a_link() {
        let test = TestContext::new();
        file(&test, "/data/target.txt", b"t");
        test.fs().symlink("target.txt", &abs("/data/link")).unwrap();
        run(
            &Move,
            &test,
            &to(args(&[&abs("/data/link")]), &abs("/data/moved")),
            None,
        )
        .unwrap();
        let meta = test
            .fs()
            .metadata(&abs("/data/moved"), Follow::NoFollow)
            .unwrap();
        assert_eq!(meta.kind, FileKind::Symlink);
        assert!(test.fs().exists(&abs("/data/target.txt")));
    }

    #[test]
    fn entries_with_an_old_etag_are_stale_and_nothing_moves() {
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
        let error = run(&Move, &test, &call, Some(vec![record])).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Stale);
        assert!(test.fs().exists(&source));
        assert_untouched(&test);
    }

    #[test]
    fn a_file_held_open_fails_with_the_holders() {
        use nurt_contracts::fs::Holder;
        use nurt_testkit::vfs::Fault;

        let test = TestContext::new();
        let source = file(&test, "/data/a.txt", b"a");
        let holders = vec![Holder {
            pid: 7,
            name: "viewer".to_owned(),
        }];
        test.fs().set_holders(&source, holders.clone());
        test.fs().inject(Fault::in_use(&source, "rename", holders));
        let error = run(
            &Move,
            &test,
            &to(args(&[&source]), &abs("/data/b.txt")),
            None,
        )
        .unwrap_err();
        assert_eq!(error.code(), ErrorCode::InUse);
        assert!(error.message.contains("viewer (7)"), "{}", error.message);
        assert!(test.fs().exists(&source));
    }
}
