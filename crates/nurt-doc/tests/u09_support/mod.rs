// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Fixtures of the `query` and `set` tests.
//!
//! [`Fixture`] holds a virtual file system, an in-memory journal and the
//! format registry of the crate with two test formats added: `kv`, a flat
//! `key = value` format, and `kvc`, the same format whose syntax check
//! reports every line containing [`ERROR_MARK`] as an error. Effects run
//! through [`CheckingTx`], which refuses a declared target whose content no
//! longer has the expected hash, as the journal does.

// Test helpers report setup failures by panicking; clippy exempts only `#[test]` functions.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
// Each test file includes this module and uses a part of it.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::WriteHandle;
use nurt_contracts::journal::{
    Applied, ExemptWrite, Expect, ExternalChange, Mutation, MutationTx, OpHandle, OpId, OpKind,
    Preserved, RollbackReason, RollbackReport, TargetIndex, TargetPath, TargetSpec,
};
use nurt_contracts::record::Item;
use nurt_contracts::report::{Change, IrreversibleEntry, NotableEntry};
use nurt_contracts::services::{
    ByteRange, DocFormatId, IssueSeverity, SyntaxIssue, TextPosition, TextSpan,
};
use nurt_contracts::stage::{
    ArgValue, BoundArgs, ExecCtx, Limits, RecordStream, StageData, StageImpl, StageInput,
};
use nurt_contracts::value::{Record, Value};
use nurt_doc::commands::query::QueryStage;
use nurt_doc::model::DocHandle;
use nurt_doc::registry::{Confidence, DocFormat, DocOptions, Registry};
use nurt_doc::service::DocumentService;
use nurt_doc::testing::{KeyValueFormat, MemoryJournal};
use nurt_testkit::ctx::TestContext;
use nurt_testkit::hash::sha256;
use nurt_testkit::services::test_services;
use nurt_testkit::vfs::{VirtualFs, test_path};

/// Identifier of the flat test format.
pub(crate) const KV: &str = "kv";

/// Identifier of the flat test format with a syntax check.
pub(crate) const CHECKED_KV: &str = "kvc";

/// Text that the syntax check of `kvc` reports as an error.
pub(crate) const ERROR_MARK: &str = "!!";

/// The `kv` format with a syntax check that reports every line containing
/// [`ERROR_MARK`] as an error of the kind `mark`.
pub(crate) struct CheckedKeyValue {
    inner: KeyValueFormat,
}

impl CheckedKeyValue {
    fn new() -> Self {
        Self {
            inner: KeyValueFormat::new(DocFormatId::new(CHECKED_KV).unwrap()),
        }
    }
}

impl DocFormat for CheckedKeyValue {
    fn id(&self) -> DocFormatId {
        self.inner.id()
    }

    fn extensions(&self) -> &[&'static str] {
        &[CHECKED_KV]
    }

    fn sniff(&self, _content: &[u8]) -> Confidence {
        Confidence::None
    }

    fn open(&self, content: Vec<u8>, options: &DocOptions) -> Result<Box<dyn DocHandle>> {
        self.inner.open(content, options)
    }

    fn check_syntax(&self, content: &[u8], _options: &DocOptions) -> Vec<SyntaxIssue> {
        String::from_utf8_lossy(content)
            .lines()
            .enumerate()
            .filter(|(_, line)| line.contains(ERROR_MARK))
            .map(|(index, _)| {
                let position = TextPosition {
                    line: u32::try_from(index + 1).unwrap(),
                    column: 1,
                };
                SyntaxIssue {
                    severity: IssueSeverity::Error,
                    message: format!("`{ERROR_MARK}` is not allowed"),
                    span: TextSpan {
                        bytes: ByteRange::new(0, 0),
                        start: position,
                        end: position,
                    },
                    kind: Some("mark".to_owned()),
                }
            })
            .collect()
    }
}

/// Identifier of the flat test format that records the paths it is opened with.
pub(crate) const PROBED_KV: &str = "kvp";

/// Paths given to [`DocFormat::open_at`], in order of the calls.
pub(crate) type OpenedPaths = Arc<Mutex<Vec<Option<PathBuf>>>>;

/// The `kv` format that records the path of every document it opens.
struct ProbedKeyValue {
    inner: KeyValueFormat,
    opened: OpenedPaths,
}

impl DocFormat for ProbedKeyValue {
    fn id(&self) -> DocFormatId {
        self.inner.id()
    }

    fn extensions(&self) -> &[&'static str] {
        &[PROBED_KV]
    }

    fn sniff(&self, _content: &[u8]) -> Confidence {
        Confidence::None
    }

    fn open(&self, content: Vec<u8>, options: &DocOptions) -> Result<Box<dyn DocHandle>> {
        self.inner.open(content, options)
    }

    fn open_at(
        &self,
        path: Option<&Path>,
        content: Vec<u8>,
        options: &DocOptions,
    ) -> Result<Box<dyn DocHandle>> {
        self.opened
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(path.map(Path::to_path_buf));
        self.inner.open(content, options)
    }
}

/// Built-in formats of the crate with `kv` and `kvc` added.
pub(crate) fn registry() -> Registry {
    registry_probing(OpenedPaths::default())
}

/// [`registry`] with the `kvp` format that records its paths in `opened`.
pub(crate) fn registry_probing(opened: OpenedPaths) -> Registry {
    let mut registry = Registry::builtin();
    registry.register(Box::new(KeyValueFormat::new(DocFormatId::new(KV).unwrap())));
    registry.register(Box::new(CheckedKeyValue::new()));
    registry.register(Box::new(ProbedKeyValue {
        inner: KeyValueFormat::new(DocFormatId::new(PROBED_KV).unwrap()),
        opened,
    }));
    registry
}

/// Doubles of one test.
pub(crate) struct Fixture {
    /// Clock, host, virtual file system and event sink.
    pub(crate) test: TestContext,
    /// Journal that receives the writes of effects.
    pub(crate) journal: MemoryJournal,
    /// Registry given to the stages.
    pub(crate) registry: Arc<Registry>,
    /// Paths with which the registry of the fixture opened `kvp` documents.
    pub(crate) opened: OpenedPaths,
    max_records: Option<u64>,
}

impl Fixture {
    /// Doubles without files.
    pub(crate) fn new() -> Self {
        let opened = OpenedPaths::default();
        Self {
            test: TestContext::new(),
            journal: MemoryJournal::new(),
            registry: Arc::new(registry_probing(Arc::clone(&opened))),
            opened,
            max_records: None,
        }
    }

    /// The doubles with a limit on the number of records.
    pub(crate) fn with_max_records(mut self, max_records: u64) -> Self {
        self.max_records = Some(max_records);
        self
    }

    /// Execution context with the document service of the crate.
    pub(crate) fn ctx(&self) -> ExecCtx {
        let service = DocumentService::new(registry(), DocOptions::default());
        let services = test_services().with_doc(Arc::new(service));
        let builder = self.test.builder().services(Arc::new(services));
        match self.max_records {
            Some(max_records) => builder
                .limits(Limits {
                    max_records,
                    ..Limits::default()
                })
                .build(),
            None => builder.build(),
        }
    }

    /// Creates or replaces a file in the file system and in the journal.
    pub(crate) fn file(&self, path: &str, content: &[u8]) {
        let fs = self.test.fs();
        if let Some(parent) = Path::new(path).parent() {
            fs.create_dir_all(&test_path(&parent.to_string_lossy()).unwrap())
                .unwrap();
        }
        let abs = test_path(path).unwrap();
        fs.write_file(&abs, content).unwrap();
        let _ = self
            .journal
            .clone()
            .with_file(abs.as_path(), content.to_vec());
    }

    /// Content of a file of the file system.
    pub(crate) fn content(&self, path: &str) -> Option<Vec<u8>> {
        self.test.fs().read_file(&test_path(path).unwrap()).ok()
    }

    /// Content of a file of the file system as text.
    pub(crate) fn text(&self, path: &str) -> String {
        String::from_utf8(self.content(path).unwrap()).unwrap()
    }

    /// Content of a file of the journal.
    pub(crate) fn journal_content(&self, path: &str) -> Option<Vec<u8>> {
        self.journal.file(test_path(path).unwrap().as_path())
    }

    /// Number of changes made through the file system interface.
    pub(crate) fn fs_mutations(&self) -> usize {
        self.test.fs().mutations().len()
    }
}

/// Entity tag of a content.
pub(crate) fn etag(content: &[u8]) -> String {
    sha256(content).to_string()
}

/// Text value.
pub(crate) fn text(value: &str) -> Value {
    Value::Text(value.to_owned())
}

/// Path value.
pub(crate) fn path(value: &str) -> Value {
    Value::Path(value.to_owned())
}

/// Map value of the given entries, in order.
pub(crate) fn map(entries: &[(&str, Value)]) -> Value {
    Value::Map(
        entries
            .iter()
            .map(|(name, value)| ((*name).to_owned(), value.clone()))
            .collect(),
    )
}

/// Bound arguments with the given values.
pub(crate) fn args(command: &str, values: &[(&str, Value)]) -> BoundArgs {
    values
        .iter()
        .fold(BoundArgs::new(command), |args, (name, value)| {
            args.with(*name, ArgValue::Value(value.clone()), false)
        })
}

/// Input of a stage: none, or the given records.
pub(crate) fn stage_input<'a>(input: Option<Vec<Record>>) -> StageInput<'a> {
    input.map_or_else(StageInput::none, |records| {
        StageInput::records(RecordStream::new(records.into_iter().map(Ok)))
    })
}

/// Every element of the output of `query`, errors included.
pub(crate) fn query_all(
    fixture: &Fixture,
    args: &BoundArgs,
    input: Option<Vec<Record>>,
) -> Vec<Result<Record>> {
    let ctx = fixture.ctx();
    let stage = QueryStage::new(Arc::clone(&fixture.registry));
    let output = match stage
        .as_read()
        .unwrap()
        .execute(args, stage_input(input), &ctx)
    {
        Ok(output) => output,
        Err(error) => return vec![Err(error)],
    };
    match output.data {
        StageData::Records(stream) => stream.collect(),
        other => panic!("`query` returned {other:?} instead of records"),
    }
}

/// Records of `query`, or its first error.
pub(crate) fn query(
    fixture: &Fixture,
    args: &BoundArgs,
    input: Option<Vec<Record>>,
) -> Result<Vec<Record>> {
    query_all(fixture, args, input).into_iter().collect()
}

/// Items of a source form `query` on `file`.
pub(crate) fn query_file(fixture: &Fixture, file: &str, expression: &str) -> Result<Vec<Item>> {
    let args = args(
        "query",
        &[("expression", text(expression)), ("path", path(file))],
    );
    query(fixture, &args, None).map(|records| items(&records))
}

/// Typed `item` records.
pub(crate) fn items(records: &[Record]) -> Vec<Item> {
    records
        .iter()
        .map(|record| Item::try_from(record.clone()).unwrap())
        .collect()
}

/// Error of a failed result.
pub(crate) fn failure<T>(result: Result<T>) -> Error {
    match result {
        Ok(_) => panic!("the call succeeded"),
        Err(error) => error,
    }
}

/// Result of an effect stage.
pub(crate) struct Outcome {
    /// Records of the output.
    pub(crate) records: Vec<Record>,
    /// Changes reported by the stage.
    pub(crate) changes: Vec<Change>,
    /// Notable events reported by the stage.
    pub(crate) notable: Vec<NotableEntry>,
    /// Targets declared to the transaction.
    pub(crate) declared: Vec<TargetSpec>,
}

/// Runs the plan and apply steps of an effect stage and commits the
/// transaction; the committed contents are copied to the file system.
pub(crate) fn effect(
    fixture: &Fixture,
    stage: &dyn StageImpl,
    args: &BoundArgs,
    input: Option<Vec<Record>>,
) -> Result<Outcome> {
    effect_with(fixture, stage, args, input, |_| {})
}

/// [`effect`] with `between` run after the plan and before the apply step.
pub(crate) fn effect_with(
    fixture: &Fixture,
    stage: &dyn StageImpl,
    args: &BoundArgs,
    input: Option<Vec<Record>>,
    between: impl FnOnce(&Fixture),
) -> Result<Outcome> {
    let ctx = fixture.ctx();
    let effect = stage.as_effect().unwrap();
    let plan = effect.plan(args, stage_input(input), &ctx)?;
    between(fixture);
    let declared = Arc::new(Mutex::new(Vec::new()));
    let mut tx = Box::new(CheckingTx {
        inner: fixture
            .journal
            .begin(OpKind::command(&stage.descriptor().command)),
        fs: Arc::clone(fixture.test.fs()),
        declared: Arc::clone(&declared),
    });
    let outcome = effect.apply(args, plan, tx.as_mut(), &ctx)?;
    let records = match outcome.output.data {
        StageData::Records(stream) => stream.collect::<Result<Vec<_>>>()?,
        other => panic!("the effect returned {other:?} instead of records"),
    };
    let changes = outcome.changes;
    let notable = outcome.notable;
    let handle = tx.commit()?;
    for target in &handle.targets {
        if let TargetPath::Fs(path) = &target.path
            && let Some(content) = fixture.journal.file(path.as_path())
        {
            fixture.test.fs().write_file(path, &content).unwrap();
        }
    }
    let declared = declared
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    Ok(Outcome {
        records,
        changes,
        notable,
        declared,
    })
}

/// Transaction that checks the expected state of every declared file
/// against the virtual file system before it delegates to the journal.
pub(crate) struct CheckingTx {
    inner: Box<dyn MutationTx>,
    fs: Arc<VirtualFs>,
    declared: Arc<Mutex<Vec<TargetSpec>>>,
}

impl CheckingTx {
    fn check(&self, spec: &TargetSpec) -> Result<()> {
        let TargetPath::Fs(path) = &spec.path else {
            return Ok(());
        };
        let current = self.fs.read_file(path).ok();
        let shown = path.as_path().display();
        match &spec.expect {
            Expect::Hash(expected) if current.as_deref().map(sha256).as_ref() != Some(expected) => {
                Err(Error::new(
                    ErrorCode::Stale,
                    format!("{shown} changed after it was read"),
                ))
            }
            Expect::Absent if current.is_some() => {
                Err(Error::new(ErrorCode::Exists, format!("{shown} exists")))
            }
            _ => Ok(()),
        }
    }
}

impl MutationTx for CheckingTx {
    fn op(&self) -> Option<OpId> {
        self.inner.op()
    }

    fn preserve(&mut self, targets: &[TargetSpec]) -> Result<Vec<Preserved>> {
        for spec in targets {
            self.check(spec)?;
        }
        self.declared
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(targets);
        self.inner.preserve(targets)
    }

    fn stage(&mut self, target: TargetIndex) -> Result<Box<dyn WriteHandle>> {
        self.inner.stage(target)
    }

    fn apply(&mut self, target: TargetIndex, mutation: Mutation) -> Result<Applied> {
        self.inner.apply(target, mutation)
    }

    fn observe_external(&mut self, changes: &[ExternalChange]) -> Result<Vec<Applied>> {
        self.inner.observe_external(changes)
    }

    fn mark_irreversible(&mut self, marker: IrreversibleEntry) {
        self.inner.mark_irreversible(marker);
    }

    fn record_exempt(&mut self, write: ExemptWrite) {
        self.inner.record_exempt(write);
    }

    fn commit(self: Box<Self>) -> Result<OpHandle> {
        self.inner.commit()
    }

    fn rollback(self: Box<Self>, reason: RollbackReason) -> Result<RollbackReport> {
        self.inner.rollback(reason)
    }
}
