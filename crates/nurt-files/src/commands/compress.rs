// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-files::commands::compress
// Responsibility:   `compress` and `decompress`: whole-file compression with gzip, zstd, xz.
// Dependencies:     nurt_archive, nurt_contracts, nurt_testkit
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! `compress` and `decompress`: whole-file compression with gzip, zstd, xz,
//! bzip2 or lz4.
//!
//! `compress` writes `<name>.<extension>` next to each source file and
//! `decompress` writes the name without the compression extension, with the
//! algorithm taken from the content of the file. An output that already
//! exists fails with `E_EXISTS` in the plan step, before anything changes.
//! The output of each file is streamed into a staged file of the transaction,
//! so memory use does not depend on the size of the file. With `keep: false`
//! the sources are removed in the same transaction, after every output has
//! been written, so a failure leaves the sources in place.
//!
//! The apply step reads each source again and fails with `E_STALE` when it
//! no longer has the entity tag seen by the plan.

use std::io::{self, Read, Write};
use std::sync::Arc;
use std::time::SystemTime;

use nurt_archive::codec::{self, Algorithm};
use nurt_archive::compress::{compress, compressed_name};
use nurt_archive::decompress::{decompress, decompressed_name};
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::{AbsPath, ContentHash, Etag, FileKind, FileMeta, Follow, Fs, ReadHandle};
use nurt_contracts::journal::{Expect, Mutation, MutationTx, TargetAction, TargetSpec};
use nurt_contracts::record::{Change, Effect, EffectStatus};
use nurt_contracts::report::{Notable, NotableEntry};
use nurt_contracts::stage::{
    BoundArgs, EffectOutcome, EffectPlan, EffectStage, ExecCtx, InputKinds, RecordStream,
    StageDescriptor, StageImpl, StageInput, StageOutput,
};
use nurt_contracts::value::{Record, RecordKind};

use crate::common::etag::{observe, stale_error};
use crate::common::mutation::{applied_change, fs_target, planned_change};
use crate::common::path::{path_list_arg_for_change, record_path};

/// Name of the `compress` command in the catalog.
const COMPRESS: &str = "compress";

/// Name of the `decompress` command in the catalog.
const DECOMPRESS: &str = "decompress";

/// Name of the `path` parameter.
const PATH: &str = "path";

/// Name of the `algorithm` parameter.
const ALGORITHM: &str = "algorithm";

/// Name of the `keep` parameter.
const KEEP: &str = "keep";

/// Name of the `level` parameter.
const LEVEL: &str = "level";

/// Algorithm of `compress` when `algorithm` is not given.
const DEFAULT_ALGORITHM: Algorithm = Algorithm::Gzip;

/// Largest number of bytes `decompress` writes for one file: one tebibyte.
const MAX_OUTPUT_BYTES: u64 = 1 << 40;

/// Number of leading bytes that identify every supported format.
const PREFIX_LEN: usize = 16;

/// Stage of `compress`: source form with `path`, stage form on `entry`
/// records.
pub(crate) struct Compress;

impl StageImpl for Compress {
    fn descriptor(&self) -> StageDescriptor {
        descriptor(COMPRESS)
    }

    fn as_effect(&self) -> Option<&dyn EffectStage> {
        Some(self)
    }
}

impl EffectStage for Compress {
    fn plan<'a>(
        &'a self,
        args: &'a BoundArgs,
        input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<EffectPlan> {
        let algorithm = algorithm_param(args)?;
        let level = level_param(args, algorithm)?;
        let keep = keep_param(args)?;
        let derive = |_: &ExecCtx, source: &AbsPath| {
            let name = file_name(source)?;
            let output = compressed_name(&name, algorithm).ok_or_else(|| empty_name(source))?;
            sibling(source, &output)
        };
        let (jobs, plan) = plan_jobs(COMPRESS, args, input, ctx, &derive)?;
        finish_plan(
            plan,
            PlanState {
                jobs,
                keep,
                algorithm,
                level,
            },
        )
    }

    fn apply<'a>(
        &'a self,
        _args: &'a BoundArgs,
        plan: EffectPlan,
        tx: &mut dyn MutationTx,
        ctx: &'a ExecCtx,
    ) -> Result<EffectOutcome<'a>> {
        let transform =
            |job: &Job, state: &PlanState, input: Box<dyn ReadHandle>, output: &mut dyn Write| {
                compress(state.algorithm, state.level, input, output)
                    .map(|_| ())
                    .map_err(|error| codec_error(&error, &job.source, COMPRESS))
            };
        apply_jobs(COMPRESS, plan, tx, ctx, &transform)
    }
}

/// Stage of `decompress`: source form with `path`, stage form on `entry`
/// records.
pub(crate) struct Decompress {
    /// Largest number of bytes written for one file.
    limit: u64,
}

impl Decompress {
    /// Stage with the production limit on the size of the output.
    const fn new() -> Self {
        Self {
            limit: MAX_OUTPUT_BYTES,
        }
    }
}

impl StageImpl for Decompress {
    fn descriptor(&self) -> StageDescriptor {
        descriptor(DECOMPRESS)
    }

    fn as_effect(&self) -> Option<&dyn EffectStage> {
        Some(self)
    }
}

impl EffectStage for Decompress {
    fn plan<'a>(
        &'a self,
        args: &'a BoundArgs,
        input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<EffectPlan> {
        let keep = keep_param(args)?;
        let derive = |ctx: &ExecCtx, source: &AbsPath| {
            let name = file_name(source)?;
            let Some((stem, _)) = decompressed_name(&name) else {
                return Err(unknown_extension(source));
            };
            if sniff(ctx, source)?.is_none() {
                return Err(not_compressed(source));
            }
            sibling(source, &stem)
        };
        let (jobs, plan) = plan_jobs(DECOMPRESS, args, input, ctx, &derive)?;
        finish_plan(
            plan,
            PlanState {
                jobs,
                keep,
                algorithm: DEFAULT_ALGORITHM,
                level: None,
            },
        )
    }

    fn apply<'a>(
        &'a self,
        _args: &'a BoundArgs,
        plan: EffectPlan,
        tx: &mut dyn MutationTx,
        ctx: &'a ExecCtx,
    ) -> Result<EffectOutcome<'a>> {
        let limit = self.limit;
        let transform =
            move |job: &Job, _: &PlanState, input: Box<dyn ReadHandle>, output: &mut dyn Write| {
                decompress(input, output, Some(limit))
                    .map(|_| ())
                    .map_err(|error| codec_error(&error, &job.source, DECOMPRESS))
            };
        apply_jobs(DECOMPRESS, plan, tx, ctx, &transform)
    }
}

/// Stages of this module, for the registry.
pub(crate) fn stages() -> Vec<Arc<dyn StageImpl>> {
    vec![Arc::new(Compress), Arc::new(Decompress::new())]
}

fn descriptor(command: &str) -> StageDescriptor {
    StageDescriptor {
        command: command.to_owned(),
        source: true,
        input: InputKinds::Only(vec![RecordKind::Entry]),
    }
}

/// File converted by one command: the source, its state seen by the plan
/// and the output.
struct Job {
    source: AbsPath,
    etag: Etag,
    size: u64,
    output: AbsPath,
}

/// Private state of the plan.
struct PlanState {
    jobs: Vec<Job>,
    keep: bool,
    algorithm: Algorithm,
    level: Option<u32>,
}

/// Output of one file: reads `input`, writes `output`.
type Transform<'t> =
    dyn Fn(&Job, &PlanState, Box<dyn ReadHandle>, &mut dyn Write) -> Result<()> + 't;

/// Output path of a source, after the checks of the command.
type Derive<'t> = dyn Fn(&ExecCtx, &AbsPath) -> Result<AbsPath> + 't;

/// File named by the program or by an input record.
struct Target {
    path: AbsPath,
    from_record: bool,
}

fn algorithm_param(args: &BoundArgs) -> Result<Algorithm> {
    let name = match args.value(ALGORITHM) {
        None | Some(nurt_contracts::value::Value::Null) => return Ok(DEFAULT_ALGORITHM),
        Some(
            nurt_contracts::value::Value::Enum(name) | nurt_contracts::value::Value::Text(name),
        ) => name.as_str(),
        Some(_) => {
            return Err(Error::new(
                ErrorCode::Type,
                format!("`{ALGORITHM}` is not an algorithm name: give one of the five algorithms"),
            ));
        }
    };
    Algorithm::from_name(name).ok_or_else(|| {
        Error::new(
            ErrorCode::UnknownValue,
            format!("`{name}` is not a compression algorithm: use gzip, zstd, xz, bzip2 or lz4"),
        )
        .with_found(name)
    })
}

fn level_param(args: &BoundArgs, algorithm: Algorithm) -> Result<Option<u32>> {
    let Some(level) = args.int(LEVEL)? else {
        return Ok(None);
    };
    let (low, high) = algorithm.level_range();
    match u32::try_from(level) {
        Ok(level) if (low..=high).contains(&level) => Ok(Some(level)),
        _ => Err(Error::new(
            ErrorCode::InvalidValue,
            format!(
                "level {level} is outside {low}..={high} for {}: give a level in this range",
                algorithm.name()
            ),
        )
        .with_problem("level_out_of_range")
        .with_found(level.to_string())),
    }
}

fn keep_param(args: &BoundArgs) -> Result<bool> {
    Ok(args.bool(KEEP)?.unwrap_or(true))
}

/// Paths of the source form or of the input records, each path once, in
/// order.
fn sources(
    command: &str,
    args: &BoundArgs,
    input: StageInput<'_>,
    ctx: &ExecCtx,
) -> Result<Vec<Target>> {
    let mut targets: Vec<Target> = Vec::new();
    let mut push = |path: AbsPath, from_record: bool| {
        if !targets.iter().any(|target| target.path == path) {
            targets.push(Target { path, from_record });
        }
    };
    if input.is_none() {
        let paths = path_list_arg_for_change(args, PATH, ctx)?;
        if paths.is_empty() {
            return Err(Error::new(
                ErrorCode::MissingParam,
                format!("`{command}` needs a path: give the files or pass entry records"),
            ));
        }
        for path in paths {
            push(path, false);
        }
        return Ok(targets);
    }
    if args.is_given(PATH) {
        return Err(Error::new(
            ErrorCode::ParamConflict,
            format!(
                "`{PATH}` cannot be given when `{command}` receives records: remove `{PATH}` or start the pipeline with `{command}`"
            ),
        ));
    }
    for record in input
        .into_records()?
        .collect_bounded(ctx.limits().max_records)?
    {
        push(record_path(&record, ctx)?, true);
    }
    Ok(targets)
}

/// Jobs of a command and the plan with their `change` records.
///
/// Fails with `E_EXISTS` for an output that exists or that another job
/// produces too.
fn plan_jobs(
    command: &str,
    args: &BoundArgs,
    input: StageInput<'_>,
    ctx: &ExecCtx,
    derive: &Derive<'_>,
) -> Result<(Vec<Job>, EffectPlan)> {
    let targets = sources(command, args, input, ctx)?;
    let mut plan = EffectPlan::new();
    if targets.is_empty() {
        plan.notable
            .push(NotableEntry::new(Notable::NothingMatched));
    }
    let fs = ctx.require_fs()?;
    let mut jobs: Vec<Job> = Vec::new();
    for target in targets {
        ctx.checkpoint()?;
        let kind = fs.metadata(&target.path, Follow::Follow)?.kind;
        if kind != FileKind::File {
            if target.from_record {
                continue;
            }
            return Err(not_a_file(&target.path, kind, command));
        }
        let observed = observe(fs, &target.path)?;
        let (Some(mut etag), Some(meta)) = (observed.etag(), observed.meta()) else {
            return Err(Error::new(
                ErrorCode::NotFound,
                format!("`{}` does not exist: check the path", target.path),
            ));
        };
        etag.hash = racy_hash(fs, &target.path, meta);
        let output = derive(ctx, &target.path)?;
        let taken = observe(fs, &output)?.exists() || jobs.iter().any(|job| job.output == output);
        if taken {
            return Err(exists_error(&output));
        }
        jobs.push(Job {
            size: meta.size,
            source: target.path,
            etag,
            output,
        });
    }
    Ok((jobs, plan))
}

/// Content hash of a source modified within the timestamp granularity of the
/// file system (`RACY_WINDOW`), whose size and modification time would not show
/// a same-size rewrite between the plan and the apply; the apply reads the
/// source whole anyway. `None` when the file is older or cannot be read.
fn racy_hash(fs: &dyn Fs, path: &AbsPath, meta: &FileMeta) -> Option<ContentHash> {
    if meta.is_racy(SystemTime::now()) {
        fs.content_hash(path).ok()
    } else {
        None
    }
}

/// Plan with the declared changes of the jobs and the state for the apply
/// step.
fn finish_plan(mut plan: EffectPlan, state: PlanState) -> Result<EffectPlan> {
    for target in targets_of(&state) {
        plan.changes.push(planned_change(&target));
    }
    plan.bulk = state.jobs.len() > 1;
    Ok(plan.with_state(state))
}

/// Declared targets of a plan: the outputs, then the sources removed with
/// `keep: false`.
fn targets_of(state: &PlanState) -> Vec<TargetSpec> {
    let mut targets: Vec<TargetSpec> = state
        .jobs
        .iter()
        .map(|job| fs_target(&job.output, TargetAction::Create, Expect::Absent))
        .collect();
    if !state.keep {
        targets.extend(state.jobs.iter().map(|job| TargetSpec {
            size_hint: Some(job.size),
            ..fs_target(
                &job.source,
                TargetAction::Delete,
                Expect::Etag(job.etag.clone()),
            )
        }));
    }
    targets
}

/// Declares every target, writes the outputs through staged files, removes
/// the sources when they are not kept and builds the effect record.
fn apply_jobs<'a>(
    command: &str,
    mut plan: EffectPlan,
    tx: &mut dyn MutationTx,
    ctx: &'a ExecCtx,
    transform: &Transform<'_>,
) -> Result<EffectOutcome<'a>> {
    let state = plan.take_state::<PlanState>().ok_or_else(|| {
        Error::new(
            ErrorCode::Internal,
            format!("the plan of `{command}` has no state: plan the command again"),
        )
    })?;
    let fs = ctx.require_fs()?;
    let specs = targets_of(&state);
    let preserved = tx.preserve(&specs)?;
    let mut changes: Vec<Change> = Vec::with_capacity(specs.len());
    for (position, job) in state.jobs.iter().enumerate() {
        ctx.checkpoint()?;
        let mut current = observe(fs, &job.source)?.etag();
        if job.etag.hash.is_some()
            && let Some(current) = current.as_mut()
        {
            current.hash = fs.content_hash(&job.source).ok();
        }
        if !current.as_ref().is_some_and(|etag| etag.matches(&job.etag)) {
            return Err(stale_error(&job.source, &job.etag, current.as_ref()));
        }
        let input = fs.open_read(&job.source)?;
        let mut handle = tx.stage(preserved[position].index)?;
        transform(job, &state, input, &mut *handle)?;
        handle
            .flush()
            .map_err(|error| io_error(&error, &job.output, command))?;
        drop(handle);
        let applied = tx.apply(preserved[position].index, Mutation::CommitStaged)?;
        changes.push(applied_change(
            &specs[position],
            &preserved[position],
            &applied,
        ));
    }
    for position in state.jobs.len()..specs.len() {
        ctx.checkpoint()?;
        let applied = tx.apply(preserved[position].index, Mutation::Remove)?;
        changes.push(applied_change(
            &specs[position],
            &preserved[position],
            &applied,
        ));
    }
    let effect = Effect {
        status: EffectStatus::Ok,
        kind: command.to_owned(),
        op: tx.op().map(|op| op.0),
        targets: Some(u64::try_from(state.jobs.len()).unwrap_or(u64::MAX)),
        changes: Some(changes.clone()),
        notable: (!plan.notable.is_empty()).then(|| {
            plan.notable
                .iter()
                .map(|entry| entry.code.as_str().to_owned())
                .collect()
        }),
        ..Effect::default()
    };
    let output = StageOutput::records(RecordStream::once(Record::from(effect)));
    Ok(EffectOutcome {
        changes,
        ..EffectOutcome::new(output)
    })
}

/// Algorithm of the leading bytes of a file; `None` when they carry no
/// signature of a supported format.
fn sniff(ctx: &ExecCtx, path: &AbsPath) -> Result<Option<Algorithm>> {
    let mut handle = ctx.require_fs()?.open_read(path)?;
    let mut prefix = [0_u8; PREFIX_LEN];
    let mut filled = 0;
    while filled < PREFIX_LEN {
        match handle.read(&mut prefix[filled..]) {
            Ok(0) => break,
            Ok(count) => filled += count,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(io_error(&error, path, DECOMPRESS)),
        }
    }
    Ok(codec::detect(&prefix[..filled]))
}

fn file_name(path: &AbsPath) -> Result<String> {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .ok_or_else(|| empty_name(path))
}

fn sibling(path: &AbsPath, name: &str) -> Result<AbsPath> {
    let parent = path.parent().ok_or_else(|| empty_name(path))?;
    Ok(parent.join_name(name)?)
}

fn empty_name(path: &AbsPath) -> Error {
    Error::new(
        ErrorCode::InvalidValue,
        format!("`{path}` has no file name: give a file"),
    )
    .with_found(path.to_language_string())
}

fn unknown_extension(path: &AbsPath) -> Error {
    Error::new(
        ErrorCode::InvalidValue,
        format!(
            "`{path}` has no compression extension: rename it to end in .gz, .zst, .xz, .bz2 or .lz4"
        ),
    )
    .with_problem("unknown_extension")
    .with_found(path.to_language_string())
}

fn not_compressed(path: &AbsPath) -> Error {
    Error::new(
        ErrorCode::Format,
        format!("`{path}` is not gzip, zstd, xz, bzip2 or lz4 data: check the file"),
    )
    .with_problem("not_compressed")
    .with_found(path.to_language_string())
}

fn exists_error(path: &AbsPath) -> Error {
    Error::new(
        ErrorCode::Exists,
        format!("`{path}` exists: remove it or choose another file"),
    )
    .with_problem("output_exists")
    .with_found(path.to_language_string())
    .with_detail("path", serde_json::Value::from(path.to_language_string()))
}

fn not_a_file(path: &AbsPath, kind: FileKind, command: &str) -> Error {
    let message = if kind == FileKind::Directory {
        format!("`{path}` is a directory: give its files with `find` and `|> {command}`")
    } else {
        format!("`{path}` is not a regular file: give a file")
    };
    Error::new(ErrorCode::InvalidValue, message)
        .with_problem("not_a_file")
        .with_found(path.to_language_string())
        .with_detail("path", serde_json::Value::from(path.to_language_string()))
}

fn io_error(error: &io::Error, path: &AbsPath, command: &str) -> Error {
    Error::new(
        ErrorCode::Io,
        format!("`{command}` failed on `{path}`: {error}; check the device and repeat the call"),
    )
    .with_detail("path", serde_json::Value::from(path.to_language_string()))
}

/// Error of a stream operation of the archive library.
fn codec_error(error: &codec::Error, path: &AbsPath, command: &str) -> Error {
    let found = serde_json::Value::from(path.to_language_string());
    match error {
        codec::Error::Format(reason) => Error::new(
            ErrorCode::Format,
            format!("`{path}` holds damaged data ({reason}): restore the file from its source"),
        )
        .with_found(path.to_language_string())
        .with_detail("path", found),
        codec::Error::Limit { limit } => Error::new(
            ErrorCode::Limit,
            format!("`{path}` expands to more than {limit} bytes: check the file before decompressing it"),
        )
        .with_problem("decompressed_size")
        .with_detail("path", found)
        .with_detail("limit", serde_json::Value::from(*limit)),
        codec::Error::Io(error) => io_error(error, path, command),
        codec::Error::InvalidLevel {
            algorithm,
            level,
            low,
            high,
        } => Error::new(
            ErrorCode::InvalidValue,
            format!(
                "level {level} is outside {low}..={high} for {}: give a level in this range",
                algorithm.name()
            ),
        )
        .with_problem("level_out_of_range"),
    }
}

#[cfg(test)]
mod tests {
    use nurt_contracts::journal::{JournalStore, TargetIndex, TargetPath};
    use nurt_contracts::record::ChangeAction;
    use nurt_contracts::stage::{ArgValue, StageData};
    use nurt_contracts::value::Value;
    use nurt_testkit::ctx::TestContext;
    use nurt_testkit::fixtures::{begin_request, racy_time_ns};
    use nurt_testkit::journal::JournalCall;
    use nurt_testkit::vfs::test_path;

    use super::*;

    const TEXT: &[u8] = b"Zielona \xc5\x82\xc4\x85ka\nZielona \xc5\x82\xc4\x85ka\n";

    fn abs(text: &str) -> AbsPath {
        test_path(text).unwrap()
    }

    fn with_files(files: &[(&str, &[u8])]) -> TestContext {
        let test = TestContext::new();
        for (path, content) in files {
            test.fs().write_file(&abs(path), content).unwrap();
        }
        test
    }

    fn args(command: &str, paths: &[&str], params: &[(&str, Value)]) -> BoundArgs {
        let mut args = BoundArgs::new(command);
        if !paths.is_empty() {
            let list = paths
                .iter()
                .map(|path| Value::Path(abs(path).to_language_string()))
                .collect();
            args.insert(PATH, ArgValue::Value(Value::List(list)), false);
        }
        for (name, value) in params {
            args.insert(*name, ArgValue::Value(value.clone()), false);
        }
        args
    }

    fn choice(value: &str) -> Value {
        Value::Enum(value.to_owned())
    }

    fn entries(paths: &[&str]) -> StageInput<'static> {
        let records = paths
            .iter()
            .map(|path| {
                Record::new(RecordKind::Entry)
                    .with("path", Value::Path(abs(path).to_language_string()))
            })
            .collect();
        StageInput::records(RecordStream::from_records(records))
    }

    fn packed(algorithm: Algorithm, data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        compress(algorithm, None, data, &mut out).unwrap();
        out
    }

    #[derive(Debug)]
    struct Run {
        planned: Vec<Change>,
        changes: Vec<Change>,
        effect: Effect,
    }

    fn execute(
        stage: &dyn EffectStage,
        test: &TestContext,
        args: &BoundArgs,
        input: StageInput<'_>,
    ) -> Result<Run> {
        let ctx = test.ctx();
        let plan = stage.plan(args, input, &ctx)?;
        let planned = plan.changes.clone();
        let mut tx = test.journal().begin(begin_request(args.command()));
        let outcome = stage.apply(args, plan, tx.as_mut(), &ctx)?;
        let StageData::Records(stream) = outcome.output.data else {
            panic!("expected an effect record");
        };
        let records: Vec<Record> = stream.collect::<Result<_>>()?;
        tx.commit()?;
        Ok(Run {
            planned,
            changes: outcome.changes,
            effect: Effect::try_from(records[0].clone()).unwrap(),
        })
    }

    fn compress_run(test: &TestContext, args: &BoundArgs) -> Result<Run> {
        execute(&Compress, test, args, StageInput::none())
    }

    fn decompress_run(test: &TestContext, args: &BoundArgs) -> Result<Run> {
        execute(&Decompress::new(), test, args, StageInput::none())
    }

    fn declared(test: &TestContext) -> Vec<TargetPath> {
        test.journal()
            .calls()
            .into_iter()
            .filter_map(|call| match call {
                JournalCall::Preserve { targets, .. } => Some(targets),
                _ => None,
            })
            .flatten()
            .collect()
    }

    fn applied_ok(test: &TestContext) -> Vec<TargetIndex> {
        test.journal()
            .calls()
            .into_iter()
            .filter_map(|call| match call {
                JournalCall::Apply {
                    index, ok: true, ..
                } => Some(index),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn descriptors_register_effect_stages() {
        let registered = stages();
        assert_eq!(registered.len(), 2);
        let names: Vec<String> = registered
            .iter()
            .map(|stage| stage.descriptor().command)
            .collect();
        assert_eq!(names, ["compress", "decompress"]);
        for stage in &registered {
            assert!(stage.descriptor().source);
            assert!(stage.descriptor().input.accepts(RecordKind::Entry));
            assert!(stage.as_effect().is_some());
            assert!(stage.as_read().is_none());
        }
    }

    #[test]
    fn compress_writes_gzip_next_to_the_source_and_keeps_it() {
        let test = with_files(&[("/data/log.txt", TEXT)]);
        let run = compress_run(&test, &args(COMPRESS, &["/data/log.txt"], &[])).unwrap();
        assert_eq!(test.fs().read_file(&abs("/data/log.txt")).unwrap(), TEXT);
        let output = test.fs().read_file(&abs("/data/log.txt.gz")).unwrap();
        assert_eq!(codec::detect(&output), Some(Algorithm::Gzip));
        let mut back = Vec::new();
        decompress(output.as_slice(), &mut back, None).unwrap();
        assert_eq!(back, TEXT);
        assert_eq!(run.changes.len(), 1);
        assert_eq!(run.changes[0].action, ChangeAction::Create);
        assert_eq!(run.planned[0].action, ChangeAction::Create);
        assert_eq!(run.effect.kind, "compress");
        assert_eq!(run.effect.targets, Some(1));
    }

    #[test]
    fn every_algorithm_round_trips_through_both_commands() {
        for algorithm in codec::ALGORITHMS {
            let test = with_files(&[("/data/a.txt", TEXT)]);
            let given = args(
                COMPRESS,
                &["/data/a.txt"],
                &[
                    (ALGORITHM, choice(algorithm.name())),
                    (KEEP, Value::Bool(false)),
                ],
            );
            compress_run(&test, &given).unwrap();
            let name = format!("/data/a.txt.{}", algorithm.extension());
            let stored = test.fs().read_file(&abs(&name)).unwrap();
            assert_eq!(codec::detect(&stored), Some(algorithm));
            assert!(!test.fs().exists(&abs("/data/a.txt")));
            let back = args(DECOMPRESS, &[&name], &[(KEEP, Value::Bool(false))]);
            decompress_run(&test, &back).unwrap();
            assert_eq!(test.fs().read_file(&abs("/data/a.txt")).unwrap(), TEXT);
            assert!(!test.fs().exists(&abs(&name)));
        }
    }

    #[test]
    fn decompress_restores_the_name_and_content_of_every_algorithm() {
        for algorithm in codec::ALGORITHMS {
            let name = format!("/data/doc.txt.{}", algorithm.extension());
            let test = with_files(&[(name.as_str(), &packed(algorithm, TEXT))]);
            let run = decompress_run(&test, &args(DECOMPRESS, &[&name], &[])).unwrap();
            assert_eq!(test.fs().read_file(&abs("/data/doc.txt")).unwrap(), TEXT);
            assert!(test.fs().exists(&abs(&name)));
            assert_eq!(run.effect.kind, "decompress");
        }
    }

    #[test]
    fn keep_false_removes_the_sources_in_the_same_transaction() {
        let test = with_files(&[("/data/a.txt", TEXT), ("/data/b.txt", b"second\n")]);
        let given = args(
            COMPRESS,
            &["/data/a.txt", "/data/b.txt"],
            &[(KEEP, Value::Bool(false)), (ALGORITHM, choice("zstd"))],
        );
        let run = compress_run(&test, &given).unwrap();
        assert!(!test.fs().exists(&abs("/data/a.txt")));
        assert!(!test.fs().exists(&abs("/data/b.txt")));
        assert!(test.fs().exists(&abs("/data/a.txt.zst")));
        assert!(test.fs().exists(&abs("/data/b.txt.zst")));
        let actions: Vec<ChangeAction> = run.changes.iter().map(|change| change.action).collect();
        assert_eq!(
            actions,
            [
                ChangeAction::Create,
                ChangeAction::Create,
                ChangeAction::Delete,
                ChangeAction::Delete
            ]
        );
        assert_eq!(run.effect.targets, Some(2));
    }

    #[test]
    fn decompress_with_keep_false_removes_the_compressed_file() {
        let test = with_files(&[("/data/a.txt.xz", &packed(Algorithm::Xz, TEXT))]);
        let given = args(
            DECOMPRESS,
            &["/data/a.txt.xz"],
            &[(KEEP, Value::Bool(false))],
        );
        decompress_run(&test, &given).unwrap();
        assert!(!test.fs().exists(&abs("/data/a.txt.xz")));
        assert_eq!(test.fs().read_file(&abs("/data/a.txt")).unwrap(), TEXT);
    }

    #[test]
    fn every_mutation_goes_through_the_transaction() {
        let test = with_files(&[("/data/a.txt", TEXT), ("/data/b.txt", b"b\n")]);
        let given = args(
            COMPRESS,
            &["/data/a.txt", "/data/b.txt"],
            &[(KEEP, Value::Bool(false))],
        );
        let ctx = test.ctx();
        let plan = Compress.plan(&given, StageInput::none(), &ctx).unwrap();
        assert!(plan.bulk);
        let mut tx = test.journal().begin(begin_request(COMPRESS));
        let outcome = Compress.apply(&given, plan, tx.as_mut(), &ctx).unwrap();
        tx.commit().unwrap();
        let targets = declared(&test);
        assert_eq!(
            targets,
            [
                TargetPath::Fs(abs("/data/a.txt.gz")),
                TargetPath::Fs(abs("/data/b.txt.gz")),
                TargetPath::Fs(abs("/data/a.txt")),
                TargetPath::Fs(abs("/data/b.txt")),
            ]
        );
        assert_eq!(applied_ok(&test).len(), targets.len());
        assert_eq!(outcome.changes.len(), targets.len());
    }

    #[test]
    fn decompress_declares_each_target_once_through_the_transaction() {
        let test = with_files(&[("/data/a.txt.gz", &packed(Algorithm::Gzip, TEXT))]);
        decompress_run(&test, &args(DECOMPRESS, &["/data/a.txt.gz"], &[])).unwrap();
        assert_eq!(declared(&test), [TargetPath::Fs(abs("/data/a.txt"))]);
        assert_eq!(applied_ok(&test), [TargetIndex(0)]);
    }

    #[test]
    fn an_existing_output_fails_with_exists_before_any_change() {
        let test = with_files(&[("/data/a.txt", TEXT), ("/data/a.txt.gz", b"older")]);
        let error = compress_run(&test, &args(COMPRESS, &["/data/a.txt"], &[])).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Exists);
        assert_eq!(error.problem.as_deref(), Some("output_exists"));
        assert_eq!(
            test.fs().read_file(&abs("/data/a.txt.gz")).unwrap(),
            b"older"
        );
        assert!(test.journal().calls().is_empty());

        let test = with_files(&[
            ("/data/a.txt.gz", &packed(Algorithm::Gzip, TEXT)),
            ("/data/a.txt", b"mine"),
        ]);
        let error = decompress_run(&test, &args(DECOMPRESS, &["/data/a.txt.gz"], &[])).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Exists);
        assert_eq!(test.fs().read_file(&abs("/data/a.txt")).unwrap(), b"mine");
    }

    #[test]
    fn two_sources_with_the_same_output_fail_with_exists() {
        let test = with_files(&[
            ("/data/a.gz", &packed(Algorithm::Gzip, TEXT)),
            ("/data/a.zst", &packed(Algorithm::Zstd, TEXT)),
        ]);
        let given = args(DECOMPRESS, &["/data/a.gz", "/data/a.zst"], &[]);
        let error = decompress_run(&test, &given).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Exists);
        assert!(!test.fs().exists(&abs("/data/a")));
    }

    #[test]
    fn decompress_detects_the_algorithm_by_content() {
        let test = with_files(&[("/data/a.txt.gz", &packed(Algorithm::Zstd, TEXT))]);
        decompress_run(&test, &args(DECOMPRESS, &["/data/a.txt.gz"], &[])).unwrap();
        assert_eq!(test.fs().read_file(&abs("/data/a.txt")).unwrap(), TEXT);
    }

    #[test]
    fn decompress_reads_every_member_of_a_multi_member_stream() {
        let mut stream = packed(Algorithm::Gzip, b"first\n");
        stream.extend(packed(Algorithm::Gzip, b"second\n"));
        let test = with_files(&[("/data/a.txt.gz", &stream)]);
        decompress_run(&test, &args(DECOMPRESS, &["/data/a.txt.gz"], &[])).unwrap();
        assert_eq!(
            test.fs().read_file(&abs("/data/a.txt")).unwrap(),
            b"first\nsecond\n"
        );
    }

    #[test]
    fn damaged_data_fails_with_format_and_leaves_no_output() {
        let mut data = packed(Algorithm::Zstd, &TEXT.repeat(50));
        let middle = data.len() / 2;
        data[middle] ^= 0xff;
        data.truncate(data.len() - 3);
        let test = with_files(&[("/data/a.txt.zst", &data)]);
        let given = args(DECOMPRESS, &["/data/a.txt.zst"], &[]);
        let error = decompress_run(&test, &given).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Format);
        assert!(!test.fs().exists(&abs("/data/a.txt")));
        assert!(test.fs().exists(&abs("/data/a.txt.zst")));
    }

    #[test]
    fn data_without_a_signature_fails_with_format_in_the_plan() {
        let test = with_files(&[("/data/a.txt.gz", b"plain text\n")]);
        let error = decompress_run(&test, &args(DECOMPRESS, &["/data/a.txt.gz"], &[])).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Format);
        assert_eq!(error.problem.as_deref(), Some("not_compressed"));
        assert!(test.journal().calls().is_empty());
    }

    #[test]
    fn a_name_without_a_compression_extension_is_rejected() {
        let test = with_files(&[("/data/notes.txt", &packed(Algorithm::Gzip, TEXT))]);
        let error =
            decompress_run(&test, &args(DECOMPRESS, &["/data/notes.txt"], &[])).unwrap_err();
        assert_eq!(error.code(), ErrorCode::InvalidValue);
        assert_eq!(error.problem.as_deref(), Some("unknown_extension"));
    }

    #[test]
    fn output_above_the_limit_fails_with_limit_and_changes_nothing() {
        let bomb = packed(Algorithm::Gzip, &vec![0_u8; 1 << 20]);
        let test = with_files(&[("/data/zeros.gz", &bomb)]);
        let stage = Decompress { limit: 4096 };
        let error = execute(
            &stage,
            &test,
            &args(DECOMPRESS, &["/data/zeros.gz"], &[]),
            StageInput::none(),
        )
        .unwrap_err();
        assert_eq!(error.code(), ErrorCode::Limit);
        assert!(!test.fs().exists(&abs("/data/zeros")));
        assert!(test.fs().exists(&abs("/data/zeros.gz")));
    }

    #[test]
    fn level_is_applied_and_checked_against_the_range_of_the_algorithm() {
        let test = with_files(&[("/data/a.txt", &TEXT.repeat(200))]);
        let low = args(
            COMPRESS,
            &["/data/a.txt"],
            &[(ALGORITHM, choice("bzip2")), (LEVEL, Value::Int(1))],
        );
        compress_run(&test, &low).unwrap();
        let output = test.fs().read_file(&abs("/data/a.txt.bz2")).unwrap();
        assert_eq!(codec::detect(&output), Some(Algorithm::Bzip2));

        for level in [0, 10, -1] {
            let bad = args(
                COMPRESS,
                &["/data/a.txt"],
                &[(ALGORITHM, choice("bzip2")), (LEVEL, Value::Int(level))],
            );
            let error = Compress
                .plan(&bad, StageInput::none(), &test.ctx())
                .unwrap_err();
            assert_eq!(error.code(), ErrorCode::InvalidValue, "level {level}");
            assert_eq!(error.problem.as_deref(), Some("level_out_of_range"));
        }
    }

    #[test]
    fn unknown_algorithm_and_missing_path_are_rejected() {
        let test = with_files(&[("/data/a.txt", TEXT)]);
        let unknown = args(COMPRESS, &["/data/a.txt"], &[(ALGORITHM, choice("rar"))]);
        let error = Compress
            .plan(&unknown, StageInput::none(), &test.ctx())
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::UnknownValue);
        let error = Compress
            .plan(&args(COMPRESS, &[], &[]), StageInput::none(), &test.ctx())
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::MissingParam);
    }

    #[test]
    fn source_form_rejects_directories_and_missing_files() {
        let test = TestContext::new();
        test.fs().create_dir_all(&abs("/data/dir")).unwrap();
        let error = Compress
            .plan(
                &args(COMPRESS, &["/data/dir"], &[]),
                StageInput::none(),
                &test.ctx(),
            )
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::InvalidValue);
        assert_eq!(error.problem.as_deref(), Some("not_a_file"));
        let error = Compress
            .plan(
                &args(COMPRESS, &["/data/none.txt"], &[]),
                StageInput::none(),
                &test.ctx(),
            )
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::NotFound);
    }

    #[test]
    fn stage_form_skips_directories_and_rejects_a_path() {
        let test = with_files(&[("/data/dir/a.txt", TEXT)]);
        let input = entries(&["/data/dir", "/data/dir/a.txt"]);
        let run = execute(&Compress, &test, &args(COMPRESS, &[], &[]), input).unwrap();
        assert_eq!(run.changes.len(), 1);
        assert!(test.fs().exists(&abs("/data/dir/a.txt.gz")));

        let error = Compress
            .plan(
                &args(COMPRESS, &["/data/dir/a.txt"], &[]),
                entries(&["/data/dir/a.txt"]),
                &test.ctx(),
            )
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::ParamConflict);

        let plan = Compress
            .plan(&args(COMPRESS, &[], &[]), entries(&[]), &test.ctx())
            .unwrap();
        assert_eq!(plan.notable[0].code, Notable::NothingMatched);
        assert!(plan.changes.is_empty());
    }

    #[test]
    fn a_same_size_rewrite_that_kept_the_modification_time_fails_with_stale() {
        let test = with_files(&[("/data/a.txt", TEXT)]);
        let path = abs("/data/a.txt");
        let racy = racy_time_ns();
        test.fs().set_modified(&path, racy).unwrap();
        let given = args(COMPRESS, &["/data/a.txt"], &[]);
        let ctx = test.ctx();
        let plan = Compress.plan(&given, StageInput::none(), &ctx).unwrap();
        let mut rewritten = TEXT.to_vec();
        rewritten[0] ^= 1;
        test.fs().write_file(&path, &rewritten).unwrap();
        test.fs().set_modified(&path, racy).unwrap();
        let mut tx = test.journal().begin(begin_request(COMPRESS));
        let error = Compress.apply(&given, plan, tx.as_mut(), &ctx).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Stale);
        assert!(!test.fs().exists(&abs("/data/a.txt.gz")));
    }

    #[test]
    fn a_source_changed_after_the_plan_fails_with_stale() {
        let test = with_files(&[("/data/a.txt", TEXT)]);
        let given = args(COMPRESS, &["/data/a.txt"], &[]);
        let ctx = test.ctx();
        let plan = Compress.plan(&given, StageInput::none(), &ctx).unwrap();
        test.fs()
            .write_file(&abs("/data/a.txt"), b"changed after the plan\n")
            .unwrap();
        let mut tx = test.journal().begin(begin_request(COMPRESS));
        let error = Compress.apply(&given, plan, tx.as_mut(), &ctx).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Stale);
        assert!(!test.fs().exists(&abs("/data/a.txt.gz")));
    }

    #[test]
    fn apply_without_the_state_of_the_plan_fails() {
        let test = TestContext::new();
        let mut tx = test.journal().begin(begin_request(COMPRESS));
        let error = Compress
            .apply(
                &args(COMPRESS, &[], &[]),
                EffectPlan::new(),
                tx.as_mut(),
                &test.ctx(),
            )
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::Internal);
    }

    #[test]
    fn repeated_paths_are_compressed_once() {
        let test = with_files(&[("/data/a.txt", TEXT)]);
        let given = args(COMPRESS, &["/data/a.txt", "/data/a.txt"], &[]);
        let run = compress_run(&test, &given).unwrap();
        assert_eq!(run.changes.len(), 1);
        assert_eq!(declared(&test).len(), 1);
    }

    #[test]
    fn an_empty_file_round_trips_through_both_commands() {
        let test = with_files(&[("/data/empty.txt", b"")]);
        let pack = args(
            COMPRESS,
            &["/data/empty.txt"],
            &[(KEEP, Value::Bool(false))],
        );
        compress_run(&test, &pack).unwrap();
        assert!(!test.fs().exists(&abs("/data/empty.txt")));
        assert!(test.fs().exists(&abs("/data/empty.txt.gz")));
        let unpack = args(DECOMPRESS, &["/data/empty.txt.gz"], &[]);
        decompress_run(&test, &unpack).unwrap();
        assert_eq!(test.fs().read_file(&abs("/data/empty.txt")).unwrap(), b"");
    }

    #[test]
    fn an_algorithm_that_is_not_a_name_is_rejected() {
        let test = with_files(&[("/data/a.txt", TEXT)]);
        let given = args(COMPRESS, &["/data/a.txt"], &[(ALGORITHM, Value::Int(3))]);
        let error = Compress
            .plan(&given, StageInput::none(), &test.ctx())
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::Type);
    }

    #[test]
    fn io_errors_name_the_command_and_the_path() {
        let error = io_error(&io::Error::other("disk"), &abs("/data/a.txt"), COMPRESS);
        assert_eq!(error.code(), ErrorCode::Io);
        let message = error.to_string();
        assert!(message.contains("compress"));
        assert!(message.contains("a.txt"));
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
