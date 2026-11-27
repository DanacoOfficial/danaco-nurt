// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-ext::stage
// Responsibility:   Commands of extensions as stage implementations.
// Dependencies:     nurt_contracts, nurt_testkit
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Commands of extensions as stage implementations.
//!
//! [`ExtensionStage`] implements [`StageImpl`] for one `[[stage]]` entry of a
//! manifest on top of an [`ExtRuntime`] (a WebAssembly component or a
//! native library). A command without `writes` is a read stage: the input
//! records go to the extension and its output records come back. A command
//! with `writes` is an effect stage: during the plan step the extension runs
//! and the files it writes through the host are only recorded; the preview
//! shows them as changes, and the apply step writes them through the
//! journal transaction, so the change can be undone.

use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::{AbsPath, Follow, FsErrorKind};
use nurt_contracts::journal::{
    Expect, Mutation, MutationTx, TargetAction, TargetIndex, TargetPath, TargetSpec,
};
use nurt_contracts::record::{Change, ChangeAction};
use nurt_contracts::stage::{
    BoundArgs, EffectOutcome, EffectPlan, EffectStage, ExecCtx, InputKinds, ReadStage,
    RecordStream, StageData, StageDescriptor, StageImpl, StageInput, StageOutput,
};
use nurt_contracts::value::{Record, RecordKind, Size};

use crate::error::{error, stage_failed};
use crate::manifest::{Permissions, StageEntry};
use crate::policy::{PathPolicy, PlaceholderRoots};
use crate::store::InstalledExt;
use crate::wire::{args_to_json, record_from_json, record_to_json};

/// Name of the input kind that stands for every record kind.
pub const ANY_KIND: &str = "any";

/// One call of a command of an extension.
#[derive(Debug)]
pub struct GuestCall<'a> {
    /// Full command name.
    pub command: &'a str,
    /// Arguments as one JSON object.
    pub args: String,
    /// Input records in JSON form.
    pub input: Vec<String>,
    /// Paths the extension may reach.
    pub policy: &'a PathPolicy,
    /// The command may plan file writes.
    pub writes: bool,
}

/// A file write planned by an extension, already checked against the
/// manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedWrite {
    /// Target file.
    pub path: AbsPath,
    /// New content.
    pub content: Vec<u8>,
}

/// Result of one call of a command of an extension.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GuestOutput {
    /// Output records in JSON form.
    pub records: Vec<String>,
    /// Planned file writes, in the order of the calls.
    pub writes: Vec<PlannedWrite>,
}

/// Code that runs the commands of one extension.
pub trait ExtRuntime: Send + Sync {
    /// Runs one command.
    ///
    /// # Errors
    ///
    /// Fails with `E_STAGE_FAILED` when the extension reports a failure or
    /// traps, with `E_PERMISSION` when the failure follows a refused host
    /// call, and with `E_LIMIT` when the extension exceeds its resources.
    fn run(&self, call: GuestCall<'_>, ctx: &ExecCtx) -> Result<GuestOutput>;
}

/// A command of an extension registered as a stage implementation.
pub struct ExtensionStage {
    command: String,
    entry: StageEntry,
    extension: String,
    dir: PathBuf,
    permissions: Permissions,
    input: InputKinds,
    runtime: Arc<dyn ExtRuntime>,
}

impl std::fmt::Debug for ExtensionStage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExtensionStage")
            .field("command", &self.command)
            .field("input", &self.input)
            .finish_non_exhaustive()
    }
}

/// Input kinds of a stage entry.
///
/// # Errors
///
/// Fails with `E_CATALOG` for a name that is neither a record kind nor
/// [`ANY_KIND`].
pub fn input_kinds(entry: &StageEntry) -> Result<InputKinds> {
    if entry.input.is_empty() {
        return Ok(InputKinds::None);
    }
    if entry.input.iter().any(|kind| kind == ANY_KIND) {
        return Ok(InputKinds::Any);
    }
    entry
        .input
        .iter()
        .map(|kind| parse_kind(kind))
        .collect::<Result<Vec<_>>>()
        .map(InputKinds::Only)
}

/// Record kind named in a manifest.
///
/// # Errors
///
/// Fails with `E_CATALOG` for an unknown kind.
pub fn parse_kind(kind: &str) -> Result<RecordKind> {
    RecordKind::from_str(kind).map_err(|_| {
        error(
            ErrorCode::Catalog,
            format!("`{kind}` is not a record kind; use a kind of the catalog or `{ANY_KIND}`"),
        )
    })
}

struct PlannedEffect {
    writes: Vec<(PlannedWrite, TargetAction, Expect)>,
    records: Vec<Record>,
}

impl ExtensionStage {
    /// Stage implementation of `entry` of the installed extension
    /// `extension`, run by `runtime`.
    ///
    /// # Errors
    ///
    /// Fails with `E_CATALOG` for an unknown input kind.
    pub fn new(
        extension: &InstalledExt,
        entry: &StageEntry,
        runtime: Arc<dyn ExtRuntime>,
    ) -> Result<Self> {
        Ok(Self {
            command: format!("{}.{}", extension.name(), entry.name),
            entry: entry.clone(),
            extension: extension.name().to_owned(),
            dir: extension.dir.clone(),
            permissions: extension.manifest.permissions.clone(),
            input: input_kinds(entry)?,
            runtime,
        })
    }

    /// Full command name.
    #[must_use]
    pub fn command(&self) -> &str {
        &self.command
    }

    fn call(
        &self,
        args: &BoundArgs,
        input: StageInput<'_>,
        ctx: &ExecCtx,
    ) -> Result<(Vec<Record>, Vec<PlannedWrite>)> {
        let records = match input.data {
            StageData::None => Vec::new(),
            StageData::Records(stream) => stream.collect_bounded(ctx.limits().max_records)?,
            StageData::Value(_) => {
                return Err(error(
                    ErrorCode::Type,
                    format!("`{}` needs input records, not a single value", self.command),
                ));
            }
            _ => Vec::new(),
        };
        let policy = PathPolicy::new(
            &self.extension,
            &self.permissions,
            &PlaceholderRoots::from_ctx(ctx, &self.dir),
        );
        let call = GuestCall {
            command: &self.command,
            args: args_to_json(args, ctx)?,
            input: records.iter().map(record_to_json).collect(),
            policy: &policy,
            writes: self.entry.writes,
        };
        let output = self.runtime.run(call, ctx)?;
        let records = output
            .records
            .iter()
            .map(|text| {
                record_from_json(text).map_err(|problem| {
                    stage_failed(
                        &self.command,
                        format!("it returned an invalid record: {problem}"),
                    )
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok((records, output.writes))
    }
}

impl StageImpl for ExtensionStage {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: self.command.clone(),
            source: self.entry.source,
            input: self.input.clone(),
        }
    }

    fn as_read(&self) -> Option<&dyn ReadStage> {
        (!self.entry.writes).then_some(self as &dyn ReadStage)
    }

    fn as_effect(&self) -> Option<&dyn EffectStage> {
        self.entry.writes.then_some(self as &dyn EffectStage)
    }
}

impl ReadStage for ExtensionStage {
    fn execute<'a>(
        &'a self,
        args: &'a BoundArgs,
        input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<StageOutput<'a>> {
        let (records, writes) = self.call(args, input, ctx)?;
        if !writes.is_empty() {
            return Err(stage_failed(
                &self.command,
                "it planned file writes, but its manifest does not declare `writes = true`",
            ));
        }
        Ok(StageOutput::records(RecordStream::from_records(records)))
    }
}

/// The writes of one call with one write per path: when an extension writes a
/// file twice, the last content counts and the file keeps the place of its
/// first write.
fn last_write_per_path(writes: Vec<PlannedWrite>) -> Vec<PlannedWrite> {
    let mut kept: Vec<PlannedWrite> = Vec::with_capacity(writes.len());
    for write in writes {
        match kept.iter_mut().find(|known| known.path == write.path) {
            Some(known) => known.content = write.content,
            None => kept.push(write),
        }
    }
    kept
}

fn planned_change(write: &PlannedWrite, ctx: &ExecCtx) -> Result<(Change, TargetAction, Expect)> {
    let fs = ctx.require_fs()?;
    let content_size = Size::from(u64::try_from(write.content.len()).unwrap_or(u64::MAX));
    match fs.metadata(&write.path, Follow::NoFollow) {
        Ok(meta) => {
            let mut change = Change::new(write.path.to_language_string(), ChangeAction::Modify);
            change.size_before = Some(Size::from(meta.size));
            change.size_after = Some(content_size);
            Ok((change, TargetAction::Modify, Expect::Etag(meta.etag())))
        }
        Err(problem) if problem.kind == FsErrorKind::NotFound => {
            let mut change = Change::new(write.path.to_language_string(), ChangeAction::Create);
            change.size_after = Some(content_size);
            Ok((change, TargetAction::Create, Expect::Absent))
        }
        Err(problem) => Err(Error::from(problem)),
    }
}

impl EffectStage for ExtensionStage {
    fn plan<'a>(
        &'a self,
        args: &'a BoundArgs,
        input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<EffectPlan> {
        let (records, writes) = self.call(args, input, ctx)?;
        let mut plan = EffectPlan::new();
        let mut planned = Vec::with_capacity(writes.len());
        for write in last_write_per_path(writes) {
            let (change, action, expect) = planned_change(&write, ctx)?;
            plan.changes.push(change);
            planned.push((write, action, expect));
        }
        plan.bulk = planned.len() > 1;
        plan.preview.clone_from(&records);
        Ok(plan.with_state(PlannedEffect {
            writes: planned,
            records,
        }))
    }

    fn apply<'a>(
        &'a self,
        _args: &'a BoundArgs,
        mut plan: EffectPlan,
        tx: &mut dyn MutationTx,
        ctx: &'a ExecCtx,
    ) -> Result<EffectOutcome<'a>> {
        let state = plan.take_state::<PlannedEffect>().ok_or_else(|| {
            error(
                ErrorCode::Internal,
                format!("the plan of `{}` lost its planned writes", self.command),
            )
        })?;
        // The grants were checked when the extension asked for the writes;
        // a path may have been replaced by a link since, so they are checked
        // again before anything is written.
        let policy = PathPolicy::new(
            &self.extension,
            &self.permissions,
            &PlaceholderRoots::from_ctx(ctx, &self.dir),
        );
        if !state.writes.is_empty() {
            let fs = ctx.require_fs()?;
            for (write, _, _) in &state.writes {
                policy.check_write(&write.path.as_path().to_string_lossy(), fs)?;
            }
        }
        let targets: Vec<TargetSpec> = state
            .writes
            .iter()
            .map(|(write, action, expect)| TargetSpec {
                path: TargetPath::Fs(write.path.clone()),
                action: action.clone(),
                expect: expect.clone(),
                size_hint: u64::try_from(write.content.len()).ok(),
            })
            .collect();
        if !targets.is_empty() {
            tx.preserve(&targets)?;
        }
        for (index, (write, _, _)) in state.writes.into_iter().enumerate() {
            let index = u32::try_from(index)
                .map_err(|_| error(ErrorCode::Limit, "too many planned writes"))?;
            tx.apply(TargetIndex(index), Mutation::WriteBytes(write.content))?;
        }
        let mut outcome = EffectOutcome::new(StageOutput::records(RecordStream::from_records(
            state.records,
        )));
        outcome.changes = std::mem::take(&mut plan.changes);
        Ok(outcome)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use nurt_contracts::error::{ErrorCode, Result};
    use nurt_contracts::fs::AbsPath;
    use nurt_contracts::journal::JournalStore;
    use nurt_contracts::stage::{
        BoundArgs, ExecCtx, InputKinds, RecordStream, StageData, StageImpl, StageInput,
    };
    use nurt_contracts::value::{Record, RecordKind, Value};
    use nurt_testkit::ctx::TestContext;
    use nurt_testkit::fixtures::begin_request;
    use nurt_testkit::vfs::test_path;

    use super::{ExtRuntime, ExtensionStage, GuestCall, GuestOutput, PlannedWrite};
    use crate::manifest::Manifest;
    use crate::store::InstalledExt;

    const MANIFEST: &str = r#"
[ext]
name = "acme"
version = "1.0.0"
api = "nurt:ext@1.0"
kind = "wasm"
wasm = "acme.wasm"

[[stage]]
name = "tag"
input = ["item"]

[[stage]]
name = "fix"
input = ["any"]
writes = true

[[stage]]
name = "make"
source = true

[permissions]
write = ["/project"]
"#;

    struct Scripted {
        output: GuestOutput,
        seen: Mutex<Vec<String>>,
    }

    impl ExtRuntime for Scripted {
        fn run(&self, call: GuestCall<'_>, _ctx: &ExecCtx) -> Result<GuestOutput> {
            self.seen.lock().unwrap().push(format!(
                "{} {} {}",
                call.command,
                call.args,
                call.input.len()
            ));
            Ok(self.output.clone())
        }
    }

    fn extension() -> InstalledExt {
        InstalledExt {
            // The granted directory is `/project` with the root of
            // `nurt_testkit::paths`.
            manifest: Manifest::parse(&MANIFEST.replace(
                "\"/project\"",
                &format!("\"{}\"", nurt_testkit::paths::abs("/project")),
            ))
            .unwrap(),
            dir: "/ext/acme".into(),
        }
    }

    fn stage(index: usize, output: GuestOutput) -> (ExtensionStage, Arc<Scripted>) {
        let runtime = Arc::new(Scripted {
            output,
            seen: Mutex::new(Vec::new()),
        });
        let extension = extension();
        let stage = ExtensionStage::new(
            &extension,
            &extension.manifest.stage[index],
            runtime.clone(),
        )
        .unwrap();
        (stage, runtime)
    }

    fn items() -> StageInput<'static> {
        StageInput::records(RecordStream::from_records(vec![
            Record::new(RecordKind::Item).with("name", Value::from("a")),
        ]))
    }

    #[test]
    fn descriptors_follow_the_manifest() {
        let (tag, _) = stage(0, GuestOutput::default());
        let descriptor = tag.descriptor();
        assert_eq!(descriptor.command, "acme.tag");
        assert_eq!(descriptor.input, InputKinds::Only(vec![RecordKind::Item]));
        assert!(tag.as_read().is_some() && tag.as_effect().is_none());
        let (fix, _) = stage(1, GuestOutput::default());
        assert_eq!(fix.descriptor().input, InputKinds::Any);
        assert!(fix.as_read().is_none() && fix.as_effect().is_some());
        let (make, _) = stage(2, GuestOutput::default());
        assert!(make.descriptor().source);
        assert_eq!(make.descriptor().input, InputKinds::None);
        assert_eq!(make.command(), "acme.make");
    }

    #[test]
    fn read_stages_return_the_records_of_the_extension() {
        let output = GuestOutput {
            records: vec![r#"{"kind":"finding","fields":{"line":1}}"#.to_owned()],
            writes: Vec::new(),
        };
        let (tag, runtime) = stage(0, output);
        let context = TestContext::new();
        let ctx = context.ctx();
        let args = BoundArgs::new("acme.tag");
        let result = tag
            .as_read()
            .unwrap()
            .execute(&args, items(), &ctx)
            .unwrap();
        let StageData::Records(stream) = result.data else {
            panic!("records expected");
        };
        let records: Vec<Record> = stream.map(Result::unwrap).collect();
        assert_eq!(records[0].kind, RecordKind::Finding);
        assert_eq!(runtime.seen.lock().unwrap().as_slice(), ["acme.tag {} 1"]);
        let value =
            tag.as_read()
                .unwrap()
                .execute(&args, StageInput::value(Value::from(1_i64)), &ctx);
        assert_eq!(value.unwrap_err().code(), ErrorCode::Type);
    }

    #[test]
    fn invalid_output_and_undeclared_writes_fail_the_stage() {
        let context = TestContext::new();
        let ctx = context.ctx();
        let args = BoundArgs::new("acme.tag");
        let invalid = GuestOutput {
            records: vec!["{}".to_owned()],
            writes: Vec::new(),
        };
        let (tag, _) = stage(0, invalid);
        let error = tag
            .as_read()
            .unwrap()
            .execute(&args, items(), &ctx)
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::StageFailed);
        assert!(error.message.contains("invalid record"));
        let writes = GuestOutput {
            records: Vec::new(),
            writes: vec![PlannedWrite {
                path: test_path("/project/a").unwrap(),
                content: Vec::new(),
            }],
        };
        let (tag, _) = stage(0, writes);
        let error = tag
            .as_read()
            .unwrap()
            .execute(&args, items(), &ctx)
            .unwrap_err();
        assert!(error.message.contains("writes = true"));
    }

    #[test]
    fn a_file_written_twice_is_planned_once_with_the_last_content() {
        let context = TestContext::new();
        context
            .fs()
            .create_dir_all(&test_path("/project").unwrap())
            .unwrap();
        let path = test_path("/project/twice.txt").unwrap();
        let output = GuestOutput {
            records: Vec::new(),
            writes: vec![
                PlannedWrite {
                    path: path.clone(),
                    content: b"first".to_vec(),
                },
                PlannedWrite {
                    path: test_path("/project/other.txt").unwrap(),
                    content: b"o".to_vec(),
                },
                PlannedWrite {
                    path: path.clone(),
                    content: b"second".to_vec(),
                },
            ],
        };
        let (fix, _) = stage(1, output);
        let ctx = context.ctx();
        let args = BoundArgs::new("acme.fix");
        let effect = fix.as_effect().unwrap();
        let plan = effect.plan(&args, items(), &ctx).unwrap();
        assert_eq!(plan.changes.len(), 2);
        let mut tx = context.journal().begin(begin_request("acme.fix"));
        effect.apply(&args, plan, tx.as_mut(), &ctx).unwrap();
        tx.commit().unwrap();
        assert_eq!(context.fs().read_file(&path).unwrap(), b"second");
    }

    #[test]
    fn a_write_outside_the_grants_is_refused_when_it_is_applied() {
        let context = TestContext::new();
        let outside = test_path("/secret/key").unwrap();
        let output = GuestOutput {
            records: Vec::new(),
            writes: vec![PlannedWrite {
                path: outside.clone(),
                content: b"x".to_vec(),
            }],
        };
        let (fix, _) = stage(1, output);
        let ctx = context.ctx();
        let args = BoundArgs::new("acme.fix");
        let effect = fix.as_effect().unwrap();
        let plan = effect.plan(&args, items(), &ctx).unwrap();
        let mut tx = context.journal().begin(begin_request("acme.fix"));
        let error = effect.apply(&args, plan, tx.as_mut(), &ctx).err().unwrap();
        assert_eq!(error.code(), ErrorCode::Permission);
        assert!(!context.fs().exists(&outside));
    }

    #[test]
    fn last_write_per_path_keeps_the_order_of_first_writes() {
        let write = |name: &str, content: &str| PlannedWrite {
            path: test_path(name).unwrap(),
            content: content.as_bytes().to_vec(),
        };
        let kept =
            super::last_write_per_path(vec![write("/a", "1"), write("/b", "2"), write("/a", "3")]);
        assert_eq!(kept, [write("/a", "3"), write("/b", "2")]);
        assert!(super::last_write_per_path(Vec::new()).is_empty());
    }

    #[test]
    fn effect_stages_plan_and_apply_writes_through_the_journal() {
        let context = TestContext::new();
        let existing: AbsPath = test_path("/project/old.txt").unwrap();
        context.fs().write_file(&existing, b"old").unwrap();
        let output = GuestOutput {
            records: vec![r#"{"kind":"item","fields":{}}"#.to_owned()],
            writes: vec![
                PlannedWrite {
                    path: existing.clone(),
                    content: b"new content".to_vec(),
                },
                PlannedWrite {
                    path: test_path("/project/new.txt").unwrap(),
                    content: b"x".to_vec(),
                },
            ],
        };
        let (fix, _) = stage(1, output);
        let ctx = context.ctx();
        let args = BoundArgs::new("acme.fix");
        let effect = fix.as_effect().unwrap();
        let plan = effect.plan(&args, items(), &ctx).unwrap();
        assert_eq!(plan.changes.len(), 2);
        assert!(plan.bulk);
        assert_eq!(plan.preview.len(), 1);
        assert_eq!(context.fs().read_file(&existing).unwrap(), b"old");

        let mut tx = context.journal().begin(begin_request("acme.fix"));
        let outcome = effect.apply(&args, plan, tx.as_mut(), &ctx).unwrap();
        assert_eq!(outcome.changes.len(), 2);
        tx.commit().unwrap();
        assert_eq!(context.fs().read_file(&existing).unwrap(), b"new content");
        assert_eq!(
            context
                .fs()
                .read_file(&test_path("/project/new.txt").unwrap())
                .unwrap(),
            b"x"
        );
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
