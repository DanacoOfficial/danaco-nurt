// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

//! Stage implementation interface, record streams and execution context.
//!
//! Every command of the language, built in or supplied by an extension, is
//! implemented through [`StageImpl`]. The executor drives a pipeline in fixed
//! execution steps ([`ExecStep`]); the phase that the binder assigned to a
//! bound stage selects the interface the executor calls:
//!
//! | Bound phase | Step | Interface | World access |
//! |---|---|---|---|
//! | `source`, `transform` | read (2) | [`ReadStage::execute`] | reads only |
//! | `check` | check (3) | [`ReadStage::execute`] | none; failure changes nothing |
//! | `mutate`, `run` | plan (4) | [`EffectStage::plan`] | reads only; `preview` stops here |
//! | `mutate`, `run` | apply (5) | [`EffectStage::apply`] | changes through [`MutationTx`] only |
//! | `after` | after (5a) | [`ReadStage::execute`] | none; reads the records of the effect |
//! | `verify` | verify (6) | executor | rolls the effect back on failure |
//!
//! Static checks ([`StageImpl::check`]) run after binding and before any
//! step, without access to the world.
//!
//! Records flow lazily: a [`RecordStream`] is pulled by the next stage, so a
//! pipeline never holds the whole input in memory unless a stage needs it
//! (for example `sort`), and then only within [`Limits::max_records`].
//! Cancellation ([`CancelToken`]) and the deadline of the program are checked
//! through [`ExecCtx::checkpoint`]. Progress and the output of launched
//! programs are reported as [`Event`]s through an [`EventSink`].
//!
//! The interface is synchronous and object safe: stages are held as
//! `Arc<dyn StageImpl>` and every interface is `Send + Sync`.

use std::any::{Any, TypeId};
use std::collections::{BTreeSet, HashMap};
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError, Weak};
use std::time::{Duration, Instant as StdInstant};

use indexmap::IndexMap;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::bound::{BoundExpr, BoundPipeline};
use crate::config::Config;
use crate::env::{Clock, Host};
use crate::envelope::RequestOptions;
use crate::error::{Error, ErrorCode, Result};
use crate::fs::Fs;
use crate::journal::{JournalStore, MutationTx};
use crate::kinds::{EffectKind, Implementation};
use crate::report::{Change, IrreversibleEntry, NotableEntry, ReportStatus};
use crate::services::{CodeService, DocService, FileListService, ProcessHost, Services};
use crate::spec::StageSpec;
use crate::value::{self, Record, RecordKind, Value};

fn contract_error(code: ErrorCode, message: impl Into<String>) -> Error {
    Error::new(code, message)
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Execution step of a program, in execution order.
///
/// The same type as the stage of an error position ([`crate::error::Stage`]):
/// the serialized form is the step number `"0"` to `"7"`, with `"5a"` for the
/// steps after the effect.
///
/// ```
/// use nurt_contracts::stage::ExecStep;
///
/// assert_eq!(ExecStep::After.as_str(), "5a");
/// assert!(ExecStep::Plan < ExecStep::Apply);
/// assert!(ExecStep::Apply.changes_world());
/// ```
pub type ExecStep = crate::error::Stage;

/// Why an execution was cancelled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum CancelReason {
    /// The caller cancelled the execution (`E_CANCELLED`).
    Requested,
    /// The time limit of the program elapsed (`E_TIMEOUT`).
    Timeout,
    /// A resource limit of the tool was reached (`E_LIMIT`).
    Limit,
    /// The engine is shutting down (`E_CANCELLED`).
    Shutdown,
}

impl CancelReason {
    /// Error code reported for this reason.
    #[must_use]
    pub fn error_code(self) -> ErrorCode {
        match self {
            Self::Requested | Self::Shutdown => ErrorCode::Cancelled,
            Self::Timeout => ErrorCode::Timeout,
            Self::Limit => ErrorCode::Limit,
        }
    }

    /// Error reported to the caller for this reason.
    #[must_use]
    pub fn to_error(self) -> Error {
        let message = match self {
            Self::Requested => "the execution was cancelled",
            Self::Timeout => "the time limit of the program elapsed",
            Self::Limit => "a resource limit was reached",
            Self::Shutdown => "the engine is shutting down",
        };
        contract_error(self.error_code(), message)
    }
}

struct CancelState {
    cancelled: AtomicBool,
    reason: Mutex<Option<CancelReason>>,
    wake: Condvar,
    children: Mutex<Vec<Weak<CancelState>>>,
}

impl CancelState {
    fn new() -> Self {
        Self {
            cancelled: AtomicBool::new(false),
            reason: Mutex::new(None),
            wake: Condvar::new(),
            children: Mutex::new(Vec::new()),
        }
    }

    fn cancel(&self, reason: CancelReason) {
        {
            let mut current = lock(&self.reason);
            if current.is_some() {
                return;
            }
            *current = Some(reason);
            self.cancelled.store(true, Ordering::Release);
            self.wake.notify_all();
        }
        let children: Vec<Arc<CancelState>> = lock(&self.children)
            .iter()
            .filter_map(Weak::upgrade)
            .collect();
        for child in children {
            child.cancel(reason);
        }
    }
}

/// Cooperative cancellation shared by all stages of one execution.
///
/// Cloning a token shares it. [`child`](CancelToken::child) creates a token
/// that is cancelled together with its parent but can also be cancelled on
/// its own, for example for one background job. The first reason wins.
///
/// ```
/// use nurt_contracts::stage::{CancelReason, CancelToken};
///
/// let root = CancelToken::new();
/// let job = root.child();
/// job.cancel(CancelReason::Limit);
/// assert!(job.is_cancelled());
/// assert!(!root.is_cancelled());
///
/// let other = root.child();
/// root.cancel(CancelReason::Requested);
/// assert_eq!(other.reason(), Some(CancelReason::Requested));
/// assert_eq!(job.reason(), Some(CancelReason::Limit));
/// ```
#[derive(Clone)]
pub struct CancelToken {
    state: Arc<CancelState>,
}

impl CancelToken {
    /// Creates a token that is not cancelled.
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: Arc::new(CancelState::new()),
        }
    }

    /// Creates a token cancelled together with this one.
    #[must_use]
    pub fn child(&self) -> Self {
        let child = Self::new();
        {
            let mut children = lock(&self.state.children);
            children.retain(|weak| weak.strong_count() > 0);
            children.push(Arc::downgrade(&child.state));
        }
        if let Some(reason) = self.reason() {
            child.cancel(reason);
        }
        child
    }

    /// Cancels the token and all its children. Has no effect when the token
    /// is already cancelled.
    pub fn cancel(&self, reason: CancelReason) {
        self.state.cancel(reason);
    }

    /// Returns `true` once the token is cancelled.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.state.cancelled.load(Ordering::Acquire)
    }

    /// Reason of the cancellation, once cancelled.
    #[must_use]
    pub fn reason(&self) -> Option<CancelReason> {
        *lock(&self.state.reason)
    }

    /// Checks whether the token is cancelled.
    ///
    /// # Errors
    ///
    /// Fails with the error of the cancellation reason
    /// ([`CancelReason::to_error`]) once the token is cancelled.
    pub fn check(&self) -> Result<()> {
        match self.reason() {
            Some(reason) => Err(reason.to_error()),
            None => Ok(()),
        }
    }

    /// Blocks the calling thread for no longer than `timeout` of real time.
    ///
    /// The thread wakes up as soon as the token is cancelled. Returns the
    /// reason when the token is cancelled, `None` when the time elapsed.
    /// Production clocks implement [`Clock::sleep`] with this method; virtual
    /// clocks do not block.
    #[must_use]
    pub fn wait_timeout(&self, timeout: Duration) -> Option<CancelReason> {
        let deadline = StdInstant::now().checked_add(timeout);
        let mut reason = lock(&self.state.reason);
        loop {
            if let Some(reason) = *reason {
                return Some(reason);
            }
            let remaining = match deadline {
                Some(deadline) => deadline.saturating_duration_since(StdInstant::now()),
                None => Duration::MAX,
            };
            if remaining.is_zero() {
                return None;
            }
            let (guard, _) = self
                .state
                .wake
                .wait_timeout(reason, remaining)
                .unwrap_or_else(PoisonError::into_inner);
            reason = guard;
        }
    }
}

impl Default for CancelToken {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for CancelToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CancelToken")
            .field("reason", &self.reason())
            .finish()
    }
}

/// Lazy stream of records passed between stages.
///
/// A stream yields `Ok(record)` items until it ends. An `Err` item ends the
/// program; streams yield nothing after their first error. Items skipped
/// without an error (for example unreadable entries during a listing) are
/// reported with [`ExecCtx::note`].
///
/// ```
/// use nurt_contracts::stage::RecordStream;
///
/// let stream = RecordStream::empty();
/// assert_eq!(stream.count(), 0);
/// ```
pub struct RecordStream<'a> {
    inner: Box<dyn Iterator<Item = Result<Record>> + Send + 'a>,
}

impl<'a> RecordStream<'a> {
    /// Wraps an iterator of records.
    pub fn new<I>(iter: I) -> Self
    where
        I: Iterator<Item = Result<Record>> + Send + 'a,
    {
        Self {
            inner: Box::new(iter),
        }
    }

    /// A stream without records.
    #[must_use]
    pub fn empty() -> Self {
        Self::new(std::iter::empty())
    }

    /// A stream of one record.
    #[must_use]
    pub fn once(record: Record) -> Self {
        Self::new(std::iter::once(Ok(record)))
    }

    /// A stream over records held in memory.
    #[must_use]
    pub fn from_records(records: Vec<Record>) -> Self {
        Self::new(records.into_iter().map(Ok))
    }

    /// A stream that fails with `error` when pulled.
    #[must_use]
    pub fn failed(error: Error) -> Self {
        Self::new(std::iter::once(Err(error)))
    }

    /// Wraps the stream so that every pull first calls
    /// [`ExecCtx::checkpoint`]; after the first error the stream ends.
    #[must_use]
    pub fn checked(self, ctx: &'a ExecCtx) -> Self {
        Self::new(Checked {
            inner: self,
            ctx,
            done: false,
        })
    }

    /// Collects the whole stream into memory.
    ///
    /// Stages that must see all records before producing output (`sort`,
    /// `last`, `group`) use this with [`Limits::max_records`].
    ///
    /// # Errors
    ///
    /// Fails with `E_LIMIT` when the stream holds more than `max` records,
    /// and with the first error item of the stream.
    pub fn collect_bounded(self, max: u64) -> Result<Vec<Record>> {
        let mut records = Vec::new();
        for item in self {
            let record = item?;
            if u64::try_from(records.len()).unwrap_or(u64::MAX) >= max {
                return Err(contract_error(
                    ErrorCode::Limit,
                    format!("the stream holds more than {max} records"),
                ));
            }
            records.push(record);
        }
        Ok(records)
    }
}

impl Iterator for RecordStream<'_> {
    type Item = Result<Record>;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next()
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl fmt::Debug for RecordStream<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RecordStream")
            .field("size_hint", &self.inner.size_hint())
            .finish_non_exhaustive()
    }
}

struct Checked<'a> {
    inner: RecordStream<'a>,
    ctx: &'a ExecCtx,
    done: bool,
}

impl Iterator for Checked<'_> {
    type Item = Result<Record>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        if let Err(error) = self.ctx.checkpoint() {
            self.done = true;
            return Some(Err(error));
        }
        let item = self.inner.next();
        if !matches!(item, Some(Ok(_))) {
            self.done = true;
        }
        item
    }
}

/// Order of a stream known to its producer.
///
/// A stage that produces records in a known order declares it so that a
/// following `sort` by the same keys does not collect the stream.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
pub struct StreamOrder {
    /// Sort keys, the primary key first.
    pub keys: Vec<OrderKey>,
}

/// One key of a [`StreamOrder`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct OrderKey {
    /// Field name.
    pub field: String,
    /// Descending order.
    pub descending: bool,
}

/// Presentation hint for the text renderer. The JSON form of the result is
/// not affected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum RenderHint {
    /// Records are entries in depth-first order with a `depth` field; the
    /// renderer draws a tree.
    Tree,
    /// As [`RenderHint::Tree`], with directory size totals.
    Sizes,
}

/// Data passed between stages.
#[derive(Debug)]
#[non_exhaustive]
pub enum StageData<'a> {
    /// No data: the stage starts the pipeline.
    None,
    /// A stream of records.
    Records(RecordStream<'a>),
    /// A single value, for example the result of `count`.
    Value(Value),
}

/// Input of a stage: the output of the previous stage.
#[derive(Debug)]
pub struct StageInput<'a> {
    /// Data.
    pub data: StageData<'a>,
    /// Header record that describes the whole stream, for example the
    /// `test_run` record in front of `test_result` records.
    pub header: Option<Record>,
    /// Order of the stream known to its producer.
    pub order: Option<StreamOrder>,
    /// Presentation hint of the producer.
    pub render: Option<RenderHint>,
}

impl<'a> StageInput<'a> {
    /// Input of a source stage.
    #[must_use]
    pub fn none() -> Self {
        Self {
            data: StageData::None,
            header: None,
            order: None,
            render: None,
        }
    }

    /// Input from a stream of records.
    #[must_use]
    pub fn records(stream: RecordStream<'a>) -> Self {
        Self {
            data: StageData::Records(stream),
            ..Self::none()
        }
    }

    /// Input from a single value.
    #[must_use]
    pub fn value(value: Value) -> Self {
        Self {
            data: StageData::Value(value),
            ..Self::none()
        }
    }

    /// Returns `true` when there is no input.
    #[must_use]
    pub fn is_none(&self) -> bool {
        matches!(self.data, StageData::None)
    }

    /// The input records.
    ///
    /// # Errors
    ///
    /// Fails with `E_TYPE` when the input is not a record stream.
    pub fn into_records(self) -> Result<RecordStream<'a>> {
        match self.data {
            StageData::Records(stream) => Ok(stream),
            StageData::None => Err(contract_error(
                ErrorCode::Type,
                "the stage needs input records",
            )),
            StageData::Value(_) => Err(contract_error(
                ErrorCode::Type,
                "the stage needs input records, not a single value",
            )),
        }
    }

    /// Transforms the input records and keeps the header, order and
    /// presentation hint.
    ///
    /// # Errors
    ///
    /// Fails with `E_TYPE` when the input is not a record stream.
    pub fn map_records<F>(self, transform: F) -> Result<StageOutput<'a>>
    where
        F: FnOnce(RecordStream<'a>) -> RecordStream<'a>,
    {
        let header = self.header;
        let order = self.order;
        let render = self.render;
        let stream = Self {
            data: self.data,
            header: None,
            order: None,
            render: None,
        }
        .into_records()?;
        Ok(StageOutput {
            data: StageData::Records(transform(stream)),
            header,
            order,
            render,
        })
    }
}

impl<'a> From<StageOutput<'a>> for StageInput<'a> {
    fn from(output: StageOutput<'a>) -> Self {
        Self {
            data: output.data,
            header: output.header,
            order: output.order,
            render: output.render,
        }
    }
}

/// Output of a stage.
#[derive(Debug)]
pub struct StageOutput<'a> {
    /// Data.
    pub data: StageData<'a>,
    /// Header record that describes the whole stream.
    pub header: Option<Record>,
    /// Order of the stream, when known.
    pub order: Option<StreamOrder>,
    /// Presentation hint.
    pub render: Option<RenderHint>,
}

impl<'a> StageOutput<'a> {
    /// Output without data.
    #[must_use]
    pub fn none() -> Self {
        Self {
            data: StageData::None,
            header: None,
            order: None,
            render: None,
        }
    }

    /// Output of a stream of records.
    #[must_use]
    pub fn records(stream: RecordStream<'a>) -> Self {
        Self {
            data: StageData::Records(stream),
            ..Self::none()
        }
    }

    /// Output of a single value.
    #[must_use]
    pub fn value(value: Value) -> Self {
        Self {
            data: StageData::Value(value),
            ..Self::none()
        }
    }

    /// Sets the header record.
    #[must_use]
    pub fn with_header(mut self, header: Record) -> Self {
        self.header = Some(header);
        self
    }

    /// Declares the order of the stream.
    #[must_use]
    pub fn with_order(mut self, order: StreamOrder) -> Self {
        self.order = Some(order);
        self
    }

    /// Sets the presentation hint.
    #[must_use]
    pub fn with_render(mut self, render: RenderHint) -> Self {
        self.render = Some(render);
        self
    }
}

/// Value of one bound argument as seen by a stage.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
#[non_exhaustive]
pub enum ArgValue {
    /// The parameter has no value and no default.
    Absent,
    /// A value known before the stage runs: a literal, a default, or a
    /// reference to a named result already computed.
    Value(Value),
    /// An expression evaluated per input record (`ƒ` parameters), through
    /// [`ExecCtx::evaluator`].
    Expr(BoundExpr),
    /// A pipeline argument (for example `verify` or `write from:`), run
    /// through [`ExecCtx::pipelines`].
    Pipeline(BoundPipeline),
}

/// Bound arguments of one stage, in signature order.
///
/// The executor builds them from the bound stage: every parameter of the
/// signature is present, defaults are explicit, constant expressions and
/// references to named results are already evaluated.
///
/// ```
/// use nurt_contracts::stage::{ArgValue, BoundArgs};
///
/// let args = BoundArgs::new("list").with("path", ArgValue::Absent, false);
/// assert_eq!(args.command(), "list");
/// assert!(!args.is_given("path"));
/// ```
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
pub struct BoundArgs {
    command: String,
    values: IndexMap<String, ArgValue>,
    defaulted: BTreeSet<String>,
}

impl BoundArgs {
    /// Creates empty arguments of `command`.
    #[must_use]
    pub fn new(command: impl Into<String>) -> Self {
        Self {
            command: command.into(),
            values: IndexMap::new(),
            defaulted: BTreeSet::new(),
        }
    }

    /// Adds an argument; `defaulted` marks a value inserted by the binder.
    #[must_use]
    pub fn with(mut self, name: impl Into<String>, value: ArgValue, defaulted: bool) -> Self {
        self.insert(name, value, defaulted);
        self
    }

    /// Adds or replaces an argument.
    pub fn insert(&mut self, name: impl Into<String>, value: ArgValue, defaulted: bool) {
        let name = name.into();
        if defaulted {
            self.defaulted.insert(name.clone());
        } else {
            self.defaulted.remove(&name);
        }
        self.values.insert(name, value);
    }

    /// Full name of the command.
    #[must_use]
    pub fn command(&self) -> &str {
        &self.command
    }

    /// Parameter names in signature order.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.values.keys().map(String::as_str)
    }

    /// Argument of a parameter.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&ArgValue> {
        self.values.get(name)
    }

    /// Returns `true` when the program gave the parameter a value.
    #[must_use]
    pub fn is_given(&self, name: &str) -> bool {
        !self.defaulted.contains(name)
            && self
                .values
                .get(name)
                .is_some_and(|value| !matches!(value, ArgValue::Absent))
    }

    /// Returns `true` when the binder inserted the default value.
    #[must_use]
    pub fn is_defaulted(&self, name: &str) -> bool {
        self.defaulted.contains(name)
    }

    /// Value known before the stage runs.
    #[must_use]
    pub fn value(&self, name: &str) -> Option<&Value> {
        match self.values.get(name) {
            Some(ArgValue::Value(value)) => Some(value),
            _ => None,
        }
    }

    /// Per-record expression.
    #[must_use]
    pub fn expr(&self, name: &str) -> Option<&BoundExpr> {
        match self.values.get(name) {
            Some(ArgValue::Expr(expr)) => Some(expr),
            _ => None,
        }
    }

    /// Pipeline argument.
    #[must_use]
    pub fn pipeline(&self, name: &str) -> Option<&BoundPipeline> {
        match self.values.get(name) {
            Some(ArgValue::Pipeline(pipeline)) => Some(pipeline),
            _ => None,
        }
    }

    /// Value that must be present.
    ///
    /// # Errors
    ///
    /// Fails with `E_MISSING_PARAM` when the parameter has no value known
    /// before the stage runs.
    pub fn require(&self, name: &str) -> Result<&Value> {
        self.value(name).ok_or_else(|| {
            contract_error(
                ErrorCode::MissingParam,
                format!("`{}` needs a value for `{name}`", self.command),
            )
        })
    }

    /// Integer value.
    ///
    /// # Errors
    ///
    /// Fails with `E_TYPE` when the value has another type.
    pub fn int(&self, name: &str) -> Result<Option<i64>> {
        match self.value(name) {
            None => Ok(None),
            Some(Value::Int(number)) => Ok(Some(*number)),
            Some(_) => Err(self.type_error(name, "int")),
        }
    }

    /// Boolean value.
    ///
    /// # Errors
    ///
    /// Fails with `E_TYPE` when the value has another type.
    pub fn bool(&self, name: &str) -> Result<Option<bool>> {
        match self.value(name) {
            None => Ok(None),
            Some(Value::Bool(flag)) => Ok(Some(*flag)),
            Some(_) => Err(self.type_error(name, "bool")),
        }
    }

    /// Text value.
    ///
    /// # Errors
    ///
    /// Fails with `E_TYPE` when the value has another type.
    pub fn text(&self, name: &str) -> Result<Option<&str>> {
        match self.value(name) {
            None => Ok(None),
            Some(Value::Text(text)) => Ok(Some(text.as_str())),
            Some(_) => Err(self.type_error(name, "text")),
        }
    }

    /// Value for one input record.
    ///
    /// Constant values are returned as they are; per-record expressions are
    /// evaluated on `record`.
    ///
    /// # Errors
    ///
    /// Fails with `E_TYPE` for a pipeline argument, with `E_STAGE_FAILED`
    /// when the context has no evaluator, and with the error of the
    /// evaluation.
    pub fn eval(
        &self,
        name: &str,
        record: Option<&Record>,
        ctx: &ExecCtx,
    ) -> Result<Option<Value>> {
        match self.values.get(name) {
            None | Some(ArgValue::Absent) => Ok(None),
            Some(ArgValue::Value(value)) => Ok(Some(value.clone())),
            Some(ArgValue::Expr(expr)) => ctx.evaluator()?.eval(expr, record, ctx).map(Some),
            Some(ArgValue::Pipeline(_)) => Err(self.type_error(name, "value")),
        }
    }

    fn type_error(&self, name: &str, expected: &str) -> Error {
        contract_error(
            ErrorCode::Type,
            format!("`{}` expects {expected} for `{name}`", self.command),
        )
    }
}

/// Record kinds a stage implementation accepts as input.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", content = "records", rename_all = "snake_case")]
#[non_exhaustive]
pub enum InputKinds {
    /// The implementation is a source only.
    None,
    /// Any record kind.
    Any,
    /// The listed record kinds.
    Only(Vec<RecordKind>),
}

impl InputKinds {
    /// Returns `true` when records of `kind` are accepted.
    #[must_use]
    pub fn accepts(&self, kind: RecordKind) -> bool {
        match self {
            Self::None => false,
            Self::Any => true,
            Self::Only(kinds) => kinds.contains(&kind),
        }
    }
}

/// Identity of a stage implementation, used by the registry.
///
/// One command may have several implementations, each for other input record
/// kinds and possibly from other crates; the registry selects the
/// implementation by command and input record kind.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct StageDescriptor {
    /// Full command name, as in the catalog.
    pub command: String,
    /// The implementation can start a pipeline (source form).
    pub source: bool,
    /// Record kinds accepted in the stage form.
    pub input: InputKinds,
}

/// Network endpoint a stage will contact, reported by the static check so
/// that an embedding application can apply its own rules before execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct NetworkTarget {
    /// Scheme, for example `"https"` or `"ssh"`.
    pub scheme: String,
    /// Host name or address as written.
    pub host: String,
    /// Port, when known.
    pub port: Option<u16>,
    /// Full address, when the stage has one.
    pub url: Option<String>,
}

/// External program a stage will start.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ExternalProgram {
    /// Program as written or as resolved.
    pub program: String,
    /// Why the stage uses an external program.
    pub implementation: Implementation,
}

/// Facts found by the static check of a stage, before any step runs.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
pub struct StaticInfo {
    /// The stage blocks the call by waiting.
    pub waits: bool,
    /// Network endpoints the stage will contact.
    pub network: Vec<NetworkTarget>,
    /// External programs the stage will start.
    pub programs: Vec<ExternalProgram>,
    /// Paths the stage names in its arguments.
    pub paths: Vec<String>,
    /// Further facts specific to the command, by name.
    pub details: IndexMap<String, Value>,
}

/// Context of the static check: no access to the world.
#[derive(Clone, Copy)]
pub struct CheckCtx<'a> {
    spec: &'a StageSpec,
    host: &'a dyn Host,
    config: &'a Config,
}

impl<'a> CheckCtx<'a> {
    /// Creates a check context.
    #[must_use]
    pub fn new(spec: &'a StageSpec, host: &'a dyn Host, config: &'a Config) -> Self {
        Self { spec, host, config }
    }

    /// Catalog specification of the command.
    #[must_use]
    pub fn spec(&self) -> &'a StageSpec {
        self.spec
    }

    /// Host facts.
    #[must_use]
    pub fn host(&self) -> &'a dyn Host {
        self.host
    }

    /// Effective configuration.
    #[must_use]
    pub fn config(&self) -> &'a Config {
        self.config
    }
}

impl fmt::Debug for CheckCtx<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CheckCtx").finish_non_exhaustive()
    }
}

/// A command implementation.
///
/// An implementation provides [`ReadStage`], [`EffectStage`] or both; the
/// executor selects the interface by the phase of the bound stage. A command
/// whose effect depends on its arguments (an action that only reads, another
/// that changes) provides both.
pub trait StageImpl: Send + Sync {
    /// Identity used by the registry.
    fn descriptor(&self) -> StageDescriptor;

    /// Checks the bound arguments beyond the catalog types and reports
    /// static facts.
    ///
    /// Runs after binding, before any step, without access to the world. The
    /// default accepts the arguments and reports nothing.
    ///
    /// # Errors
    ///
    /// Fails with the error that rejects the program, usually `E_TYPE` or
    /// `E_MISSING_PARAM` with the parameter concerned.
    fn check(&self, _args: &BoundArgs, _ctx: &CheckCtx<'_>) -> Result<StaticInfo> {
        Ok(StaticInfo::default())
    }

    /// The read interface, for the phases `source`, `transform`, `check` and
    /// `after`.
    fn as_read(&self) -> Option<&dyn ReadStage> {
        None
    }

    /// The effect interface, for the phases `mutate` and `run`.
    fn as_effect(&self) -> Option<&dyn EffectStage> {
        None
    }
}

/// Destination of the stage implementations supplied by a command crate.
///
/// Every command crate exposes one function of the form
/// `pub fn register(registrar: &mut dyn StageRegistrar) -> Result<()>` that
/// adds all its implementations. Registration does not depend on the order
/// in which command crates and extensions are registered: the registry of
/// the engine accepts an implementation of a command that is not in the
/// catalog yet and checks every implementation against the catalog in a
/// separate validation step after all registrations. A vector collects
/// implementations without checks.
///
/// ```
/// use std::sync::Arc;
/// use nurt_contracts::error::Result;
/// use nurt_contracts::stage::{InputKinds, StageDescriptor, StageImpl, StageRegistrar};
///
/// struct Hello;
///
/// impl StageImpl for Hello {
///     fn descriptor(&self) -> StageDescriptor {
///         StageDescriptor {
///             command: "hello".to_owned(),
///             source: true,
///             input: InputKinds::None,
///         }
///     }
/// }
///
/// fn register(registrar: &mut dyn StageRegistrar) -> Result<()> {
///     registrar.register(Arc::new(Hello))
/// }
///
/// let mut stages: Vec<Arc<dyn StageImpl>> = Vec::new();
/// assert!(register(&mut stages).is_ok());
/// assert_eq!(stages[0].descriptor().command, "hello");
/// ```
pub trait StageRegistrar {
    /// Adds an implementation described by its
    /// [`descriptor`](StageImpl::descriptor).
    ///
    /// An implementation of a command that is missing from the catalog, or
    /// that accepts record kinds the catalog does not declare for the
    /// command, is reported by the validation step of the registry, not by
    /// this method.
    ///
    /// # Errors
    ///
    /// Fails when the implementation cannot be added; the registry of the
    /// engine fails with `E_CATALOG` when another implementation of the same
    /// command already accepts one of the input kinds or already is a
    /// source. A registrar that checks implementations as they arrive may
    /// fail with `E_UNKNOWN_COMMAND` or `E_TYPE`.
    fn register(&mut self, stage: Arc<dyn StageImpl>) -> Result<()>;
}

impl StageRegistrar for Vec<Arc<dyn StageImpl>> {
    fn register(&mut self, stage: Arc<dyn StageImpl>) -> Result<()> {
        self.push(stage);
        Ok(())
    }
}

/// A stage that reads the world or transforms records and never changes the
/// world.
pub trait ReadStage: Send + Sync {
    /// Produces the output of the stage.
    ///
    /// The output stream may be lazy and borrow the arguments, the input and
    /// the context. Long loops call [`ExecCtx::checkpoint`] or wrap their
    /// stream with [`RecordStream::checked`].
    ///
    /// # Errors
    ///
    /// Fails with the error that ends the program before the first record;
    /// later errors are items of the output stream.
    fn execute<'a>(
        &'a self,
        args: &'a BoundArgs,
        input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<StageOutput<'a>>;
}

/// The effect stage of a pipeline: a change, an external action or a
/// launched program.
///
/// The same [`EffectPlan`] serves the preview and the execution, so the
/// targets shown by a preview are the targets changed by the execution.
pub trait EffectStage: Send + Sync {
    /// Computes the targets and the planned effect without changing the
    /// world.
    ///
    /// The stage consumes its input here and records the entity tag of every
    /// target as read. Read-only access, including read-only network
    /// requests needed to compute the plan, is allowed; nothing may be
    /// created, changed, sent or started. With `preview` the execution ends
    /// after this step.
    ///
    /// # Errors
    ///
    /// Fails with the error that ends the program; the world is unchanged.
    fn plan<'a>(
        &'a self,
        args: &'a BoundArgs,
        input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<EffectPlan>;

    /// Performs the planned effect through the transaction.
    ///
    /// Every change of user data goes through `tx`: the stage declares and
    /// preserves all targets before the first change, then applies the
    /// changes; the journal checks entity tags. The executor admits the
    /// operation, opens the transaction before this call and commits or rolls
    /// it back after it; the stage never commits. Effects outside the file
    /// system are marked irreversible on `tx`. The output holds the records
    /// of the effect (`effect`, `exec`, `job`, …) read by the stages after
    /// the effect; it cannot borrow the transaction.
    ///
    /// # Errors
    ///
    /// Fails with the error of the transaction (for example `E_STALE`) or of
    /// the effect; the executor then rolls the transaction back.
    fn apply<'a>(
        &'a self,
        args: &'a BoundArgs,
        plan: EffectPlan,
        tx: &mut dyn MutationTx,
        ctx: &'a ExecCtx,
    ) -> Result<EffectOutcome<'a>>;
}

/// Planned effect of an effect stage, produced without changing the world.
pub struct EffectPlan {
    /// Planned changes, one per target, with differences, in target order.
    pub changes: Vec<Change>,
    /// Unusual effects known before execution.
    pub notable: Vec<NotableEntry>,
    /// Effects known to be irreversible before execution.
    pub irreversible: Vec<IrreversibleEntry>,
    /// Records shown by a preview when the effect has no file targets, for
    /// example the command line and directory of a program to run.
    pub preview: Vec<Record>,
    /// The operation has more than one target and is undone as a whole.
    pub bulk: bool,
    state: Option<Box<dyn Any + Send>>,
}

impl EffectPlan {
    /// A plan without changes.
    #[must_use]
    pub fn new() -> Self {
        Self {
            changes: Vec::new(),
            notable: Vec::new(),
            irreversible: Vec::new(),
            preview: Vec::new(),
            bulk: false,
            state: None,
        }
    }

    /// Attaches private state of the implementation, read back in
    /// [`EffectStage::apply`]. The state should hold what is needed to
    /// produce new content, not the content itself, so that large changes do
    /// not stay in memory between the steps.
    #[must_use]
    pub fn with_state<T: Any + Send>(mut self, state: T) -> Self {
        self.state = Some(Box::new(state));
        self
    }

    /// Private state, when it has type `T`.
    #[must_use]
    pub fn state<T: Any>(&self) -> Option<&T> {
        self.state.as_ref()?.downcast_ref::<T>()
    }

    /// Takes the private state out of the plan, when it has type `T`.
    pub fn take_state<T: Any>(&mut self) -> Option<T> {
        let state = self.state.take()?;
        match state.downcast::<T>() {
            Ok(state) => Some(*state),
            Err(state) => {
                self.state = Some(state);
                None
            }
        }
    }
}

impl Default for EffectPlan {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for EffectPlan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EffectPlan")
            .field("changes", &self.changes)
            .field("notable", &self.notable)
            .field("irreversible", &self.irreversible)
            .field("preview", &self.preview)
            .field("bulk", &self.bulk)
            .field("state", &self.state.is_some())
            .finish()
    }
}

/// Result of an applied effect.
#[derive(Debug)]
pub struct EffectOutcome<'a> {
    /// Records of the effect, read by the stages after the effect.
    pub output: StageOutput<'a>,
    /// Changes as applied, one per target, in target order.
    pub changes: Vec<Change>,
    /// Unusual effects observed during execution.
    pub notable: Vec<NotableEntry>,
    /// Irreversible effects observed during execution.
    pub irreversible: Vec<IrreversibleEntry>,
    /// The effect continues in the background (a launched program passed its
    /// `wait`); the output holds a `job` record and the stages after the
    /// effect are not run.
    pub detached: bool,
}

impl<'a> EffectOutcome<'a> {
    /// An outcome with the given output and nothing else to report.
    #[must_use]
    pub fn new(output: StageOutput<'a>) -> Self {
        Self {
            output,
            changes: Vec::new(),
            notable: Vec::new(),
            irreversible: Vec::new(),
            detached: false,
        }
    }
}

/// Evaluates per-record expressions of bound arguments.
pub trait ExprEvaluator: Send + Sync {
    /// Value of `expr` for `record`; `record` is `None` outside a record
    /// stream.
    ///
    /// # Errors
    ///
    /// Fails with the error of the evaluation, for example `E_TYPE` for an
    /// operand of another type.
    fn eval(&self, expr: &BoundExpr, record: Option<&Record>, ctx: &ExecCtx) -> Result<Value>;
}

/// Runs pipeline arguments.
pub trait PipelineRunner: Send + Sync {
    /// Runs `pipeline` on `input` and returns its output.
    ///
    /// # Errors
    ///
    /// Fails with the first error of a stage of the pipeline.
    fn run<'a>(
        &'a self,
        pipeline: &'a BoundPipeline,
        input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<StageOutput<'a>>;
}

/// Shared objects registered by type, for facilities without a dedicated
/// accessor on [`ExecCtx`] (for example a shared thread pool or a read
/// cache).
///
/// ```
/// use std::sync::Arc;
/// use nurt_contracts::stage::Extensions;
///
/// trait Greeter: Send + Sync {
///     fn greet(&self) -> String;
/// }
/// struct English;
/// impl Greeter for English {
///     fn greet(&self) -> String {
///         "hello".to_owned()
///     }
/// }
///
/// let mut extensions = Extensions::new();
/// extensions.insert::<dyn Greeter>(Arc::new(English));
/// let greeter = extensions.get::<dyn Greeter>().map(|g| g.greet());
/// assert_eq!(greeter.as_deref(), Some("hello"));
/// ```
#[derive(Default)]
pub struct Extensions {
    entries: HashMap<TypeId, Box<dyn Any + Send + Sync>>,
}

impl Extensions {
    /// An empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers `object` under its type `T`, which may be a trait object
    /// type; replaces an earlier object of the same type.
    pub fn insert<T: ?Sized + Send + Sync + 'static>(&mut self, object: Arc<T>) {
        self.entries.insert(TypeId::of::<T>(), Box::new(object));
    }

    /// The object registered under type `T`.
    #[must_use]
    pub fn get<T: ?Sized + Send + Sync + 'static>(&self) -> Option<Arc<T>> {
        let entry: &(dyn Any + Send + Sync) = self.entries.get(&TypeId::of::<T>())?.as_ref();
        entry.downcast_ref::<Arc<T>>().cloned()
    }

    /// Returns `true` when an object is registered under type `T`.
    #[must_use]
    pub fn contains<T: ?Sized + Send + Sync + 'static>(&self) -> bool {
        self.entries.contains_key(&TypeId::of::<T>())
    }

    /// Number of registered objects.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns `true` when nothing is registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl fmt::Debug for Extensions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Extensions")
            .field("len", &self.entries.len())
            .finish()
    }
}

/// Fields of the output records used by the rest of the pipeline.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", content = "fields", rename_all = "snake_case")]
pub enum FieldDemand {
    /// All fields may be used.
    #[default]
    All,
    /// Only the listed fields are used; fields that are expensive to compute
    /// and not listed may be left out.
    Only(BTreeSet<String>),
}

impl FieldDemand {
    /// Returns `true` when `field` is used later.
    #[must_use]
    pub fn wants(&self, field: &str) -> bool {
        match self {
            Self::All => true,
            Self::Only(fields) => fields.contains(field),
        }
    }
}

/// What the rest of the pipeline needs from the output of a stage.
///
/// A stage may use it to avoid work; the output must be correct for any
/// demand.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Demand {
    /// Fields used later.
    pub fields: FieldDemand,
    /// An effect stage or an entity-tag check follows: records of files must
    /// carry `etag`.
    pub etag: bool,
    /// No more than this many records are used later (`first`, page limit);
    /// the stage may stop early.
    pub limit: Option<u64>,
}

impl Default for Demand {
    fn default() -> Self {
        Self {
            fields: FieldDemand::All,
            etag: true,
            limit: None,
        }
    }
}

/// Resource limits of one execution.
///
/// The default allows 5 000 000 records in memory, text files of up to 1 GiB
/// and one thread per stage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Limits {
    /// Largest number of records a stage may hold in memory (`E_LIMIT` when
    /// exceeded; records are never dropped silently).
    pub max_records: u64,
    /// Largest file read as text, in bytes.
    pub max_file_bytes_text: u64,
    /// Threads a stage may use for parallel work; results stay in a
    /// deterministic order.
    pub threads: u32,
}

const DEFAULT_MAX_RECORDS: u64 = 5_000_000;
const DEFAULT_MAX_FILE_BYTES_TEXT: u64 = 1 << 30;
const DEFAULT_THREADS: u32 = 1;

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_records: DEFAULT_MAX_RECORDS,
            max_file_bytes_text: DEFAULT_MAX_FILE_BYTES_TEXT,
            threads: DEFAULT_THREADS,
        }
    }
}

/// Position of the running stage in its pipeline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct StageFrame {
    /// Index of the stage in the pipeline, from 0 for the head.
    pub index: u32,
    /// Full command name.
    pub command: String,
    /// Execution step.
    pub step: ExecStep,
    /// What the rest of the pipeline needs.
    pub demand: Demand,
}

impl Default for StageFrame {
    fn default() -> Self {
        Self {
            index: 0,
            command: String::new(),
            step: ExecStep::Read,
            demand: Demand::default(),
        }
    }
}

/// Output stream of a launched program.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OutputStream {
    /// Standard output.
    Stdout,
    /// Standard error.
    Stderr,
}

/// Final state of a job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum JobEnd {
    /// The program exited by itself.
    Exited,
    /// The program could not run or ended abnormally.
    Failed,
    /// The program was stopped.
    Stopped,
    /// The program exceeded its time limit.
    TimedOut,
}

/// Severity of a log event.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum LogLevel {
    /// An error that does not end the execution.
    Error,
    /// A warning.
    Warn,
    /// Information.
    Info,
    /// Diagnostic detail.
    Debug,
    /// Fine diagnostic detail.
    Trace,
}

/// Progress of a long stage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Progress {
    /// Command reporting progress.
    pub command: String,
    /// Phase of the work, for example `"scan"`, `"receive"` or `"checkout"`.
    pub phase: String,
    /// Units done.
    pub done: u64,
    /// Total units, when known.
    pub total: Option<u64>,
    /// Bytes transferred or processed, when meaningful.
    pub bytes: Option<u64>,
    /// Short human-readable description.
    pub message: Option<String>,
}

/// A launched program started.
///
/// Every launched program has a job identifier from the start, so the same
/// events describe programs that finish in the foreground and programs that
/// continue in the background.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct JobStarted {
    /// Job identifier, for example `"j7"`.
    pub id: String,
    /// Label given by the program.
    pub label: Option<String>,
    /// Command line, program first.
    pub command: Vec<String>,
    /// Working directory.
    pub cwd: String,
    /// Process identifier.
    pub pid: u32,
    /// Start time.
    pub started: value::Instant,
}

/// New output lines of a launched program.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct JobOutput {
    /// Job identifier.
    pub id: String,
    /// Stream the lines come from.
    pub stream: OutputStream,
    /// Cursor after these lines, accepted by `jobs … since:`.
    pub cursor: String,
    /// Decoded lines without line endings.
    pub lines: Vec<String>,
    /// Bytes of output these lines represent.
    pub bytes: u64,
}

/// Progress of a launched program, from its `progress:` pattern.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct JobProgress {
    /// Job identifier.
    pub id: String,
    /// Percentage, when the line gives one.
    pub percent: Option<f64>,
    /// Matching output line.
    pub line: Option<String>,
}

/// A launched program ended.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct JobFinished {
    /// Job identifier.
    pub id: String,
    /// Final state.
    pub state: JobEnd,
    /// Exit code, when the program exited.
    pub exit_code: Option<i32>,
    /// Signal that ended the program, when any.
    pub signal: Option<String>,
    /// Running time.
    pub elapsed: value::Duration,
    /// File holding the full output.
    pub output_file: Option<String>,
    /// Number of detected file changes.
    pub changes: u64,
}

/// Diagnostic message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LogEvent {
    /// Severity.
    pub level: LogLevel,
    /// Message.
    pub message: String,
    /// Component that wrote the message.
    pub source: Option<String>,
}

/// Start of the execution of a request; the first event of an execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ExecStarted {
    /// Hash of the executed program, when it was parsed.
    pub ast_hash: Option<String>,
    /// Strongest effect class of the program.
    pub effect: EffectKind,
}

/// End of the execution of a request; the last event of an execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ExecFinished {
    /// Status of the report.
    pub status: ReportStatus,
    /// Code of the error that ended the execution.
    pub error: Option<ErrorCode>,
}

/// Event emitted while a program executes.
///
/// The JSON form carries the event type in the member `"t"`. The engine adds
/// the sequence number, the time and the request identifier when it delivers
/// the event to the embedding application ([`DeliveredEvent`]).
///
/// ```
/// use nurt_contracts::stage::{Event, LogEvent, LogLevel};
///
/// let event = Event::Log(LogEvent {
///     level: LogLevel::Warn,
///     message: "slow volume".to_owned(),
///     source: None,
/// });
/// let json = serde_json::to_value(&event).unwrap_or_default();
/// assert_eq!(json["t"], "log");
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "t")]
#[non_exhaustive]
pub enum Event {
    /// Progress of a long stage.
    #[serde(rename = "progress")]
    Progress(Progress),
    /// A launched program started.
    #[serde(rename = "job.started")]
    JobStarted(JobStarted),
    /// New output of a launched program.
    #[serde(rename = "job.output")]
    JobOutput(JobOutput),
    /// Progress of a launched program.
    #[serde(rename = "job.progress")]
    JobProgress(JobProgress),
    /// A launched program ended.
    #[serde(rename = "job.finished")]
    JobFinished(JobFinished),
    /// Diagnostic message.
    #[serde(rename = "log")]
    Log(LogEvent),
    /// The execution of a request started.
    #[serde(rename = "exec.started")]
    ExecStarted(ExecStarted),
    /// The execution of a request ended.
    #[serde(rename = "exec.finished")]
    ExecFinished(ExecFinished),
}

/// Event as delivered to the embedding application.
///
/// Events of one execution are delivered in the order of `seq`, from
/// [`Event::ExecStarted`] to [`Event::ExecFinished`]. The JSON form is the
/// form of the event with the members `seq`, `time` and `request` added.
///
/// ```
/// use nurt_contracts::kinds::EffectKind;
/// use nurt_contracts::stage::{DeliveredEvent, Event, ExecStarted};
/// use nurt_contracts::value::Instant;
///
/// let delivered = DeliveredEvent {
///     seq: 1,
///     time: Instant::UNIX_EPOCH,
///     request: Some("r1".to_owned()),
///     event: Event::ExecStarted(ExecStarted {
///         ast_hash: None,
///         effect: EffectKind::Read,
///     }),
/// };
/// let json = serde_json::to_value(&delivered).unwrap();
/// assert_eq!(json["t"], "exec.started");
/// assert_eq!(json["seq"], 1);
/// assert_eq!(json["time"], "1970-01-01T00:00:00Z");
/// let back: DeliveredEvent = serde_json::from_value(json).unwrap();
/// assert_eq!(back, delivered);
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DeliveredEvent {
    /// Sequence number, increasing within one execution, starting at 1.
    pub seq: u64,
    /// Time of emission.
    pub time: value::Instant,
    /// Identifier of the request, when the request has one.
    pub request: Option<String>,
    /// The event.
    #[serde(flatten)]
    pub event: Event,
}

/// Receiver of events.
///
/// Emitting never blocks the execution for long: implementations buffer
/// events and deliver them in order from their own thread.
pub trait EventSink: Send + Sync {
    /// Accepts one event.
    fn emit(&self, event: Event);
}

/// Event sink that drops all events.
#[derive(Debug, Clone, Copy, Default)]
pub struct DiscardEvents;

impl EventSink for DiscardEvents {
    fn emit(&self, _event: Event) {}
}

/// Execution context of a stage.
///
/// The context gives a stage everything it may use: the read-only file
/// system, the journal store, clock, host, configuration, request options,
/// cancellation and deadline, limits, events and the registry of services of
/// other command groups ([`Services`]). A stage never reaches the world any
/// other way. Cloning is cheap; the executor derives one context per stage
/// with [`for_stage`](ExecCtx::for_stage).
#[derive(Clone)]
pub struct ExecCtx {
    clock: Arc<dyn Clock>,
    host: Arc<dyn Host>,
    fs: Option<Arc<dyn Fs>>,
    journal: Option<Arc<dyn JournalStore>>,
    config: Arc<Config>,
    options: Arc<RequestOptions>,
    events: Arc<dyn EventSink>,
    cancel: CancelToken,
    deadline: Option<Duration>,
    limits: Limits,
    state_dir: Option<PathBuf>,
    evaluator: Option<Arc<dyn ExprEvaluator>>,
    pipelines: Option<Arc<dyn PipelineRunner>>,
    services: Arc<Services>,
    extensions: Arc<Extensions>,
    frame: StageFrame,
    notes: Arc<Mutex<Vec<NotableEntry>>>,
}

impl ExecCtx {
    /// Starts building a context from the two facilities every context has.
    #[must_use]
    pub fn builder(clock: Arc<dyn Clock>, host: Arc<dyn Host>) -> ExecCtxBuilder {
        ExecCtxBuilder {
            clock,
            host,
            fs: None,
            journal: None,
            config: None,
            options: None,
            events: None,
            cancel: None,
            timeout: None,
            limits: Limits::default(),
            state_dir: None,
            evaluator: None,
            pipelines: None,
            services: Arc::default(),
            extensions: Extensions::new(),
            frame: StageFrame::default(),
        }
    }

    /// The same context for another stage.
    #[must_use]
    pub fn for_stage(&self, frame: StageFrame) -> Self {
        Self {
            frame,
            ..self.clone()
        }
    }

    /// Clock.
    #[must_use]
    pub fn clock(&self) -> &dyn Clock {
        self.clock.as_ref()
    }

    /// Host facts and the process environment.
    #[must_use]
    pub fn host(&self) -> &dyn Host {
        self.host.as_ref()
    }

    /// Read-only file system; present in every context built for execution.
    #[must_use]
    pub fn fs(&self) -> Option<&dyn Fs> {
        self.fs.as_deref()
    }

    /// Read-only file system that must be present.
    ///
    /// # Errors
    ///
    /// Fails with `E_STAGE_FAILED` when the context has no file system.
    pub fn require_fs(&self) -> Result<&dyn Fs> {
        self.fs().ok_or_else(|| self.missing("file system"))
    }

    /// Journal store, for the journal commands.
    #[must_use]
    pub fn journal(&self) -> Option<&dyn JournalStore> {
        self.journal.as_deref()
    }

    /// Effective configuration.
    #[must_use]
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Options of the request.
    #[must_use]
    pub fn options(&self) -> &RequestOptions {
        &self.options
    }

    /// Cancellation token of the execution.
    #[must_use]
    pub fn cancel(&self) -> &CancelToken {
        &self.cancel
    }

    /// Deadline of the program on the monotonic clock.
    #[must_use]
    pub fn deadline(&self) -> Option<Duration> {
        self.deadline
    }

    /// Time left until the deadline.
    #[must_use]
    pub fn remaining(&self) -> Option<Duration> {
        self.deadline
            .map(|deadline| deadline.saturating_sub(self.clock.monotonic()))
    }

    /// Checks cancellation and the deadline.
    ///
    /// Stages call it in long loops and before blocking work. A passed
    /// deadline cancels the token with [`CancelReason::Timeout`].
    ///
    /// # Errors
    ///
    /// Fails with `E_CANCELLED`, `E_TIMEOUT` or `E_LIMIT` when the execution
    /// was cancelled or the deadline passed.
    pub fn checkpoint(&self) -> Result<()> {
        self.cancel.check()?;
        if self
            .deadline
            .is_some_and(|deadline| self.clock.monotonic() >= deadline)
        {
            self.cancel.cancel(CancelReason::Timeout);
            return self.cancel.check();
        }
        Ok(())
    }

    /// Waits for `duration` on the context clock, never past the deadline
    /// and only while the execution is not cancelled.
    ///
    /// # Errors
    ///
    /// Fails as [`ExecCtx::checkpoint`] when the execution is cancelled
    /// before or during the wait, and with `E_TIMEOUT` when the wait reaches
    /// the deadline.
    pub fn sleep(&self, duration: Duration) -> Result<()> {
        self.checkpoint()?;
        let (bounded, cut) = match self.remaining() {
            Some(remaining) if remaining < duration => (remaining, true),
            _ => (duration, false),
        };
        self.clock
            .sleep(bounded, &self.cancel)
            .map_err(CancelReason::to_error)?;
        if cut {
            self.cancel.cancel(CancelReason::Timeout);
            return self.cancel.check();
        }
        Ok(())
    }

    /// Resource limits.
    #[must_use]
    pub fn limits(&self) -> &Limits {
        &self.limits
    }

    /// State directory of the engine (spool, jobs, caches), when configured.
    #[must_use]
    pub fn state_dir(&self) -> Option<&Path> {
        self.state_dir.as_deref()
    }

    /// Evaluator of per-record expressions.
    ///
    /// # Errors
    ///
    /// Fails with `E_STAGE_FAILED` when the context has no evaluator.
    pub fn evaluator(&self) -> Result<&dyn ExprEvaluator> {
        self.evaluator
            .as_deref()
            .ok_or_else(|| self.missing("expression evaluator"))
    }

    /// Runner of pipeline arguments.
    ///
    /// # Errors
    ///
    /// Fails with `E_STAGE_FAILED` when the context has no pipeline runner.
    pub fn pipelines(&self) -> Result<&dyn PipelineRunner> {
        self.pipelines
            .as_deref()
            .ok_or_else(|| self.missing("pipeline runner"))
    }

    /// Registry of the services of other command groups.
    #[must_use]
    pub fn services(&self) -> &Services {
        &self.services
    }

    /// Structured document service, when its command group is loaded; the
    /// same as [`Services::doc`].
    #[must_use]
    pub fn doc(&self) -> Option<&dyn DocService> {
        self.services.doc()
    }

    /// Source code service, when its command group is loaded; the same as
    /// [`Services::code`].
    #[must_use]
    pub fn code(&self) -> Option<&dyn CodeService> {
        self.services.code()
    }

    /// Program launching service, when available; the same as
    /// [`Services::process_host`].
    #[must_use]
    pub fn process(&self) -> Option<&dyn ProcessHost> {
        self.services.process_host()
    }

    /// File listing service, when available; the same as
    /// [`Services::file_list`].
    #[must_use]
    pub fn files(&self) -> Option<&dyn FileListService> {
        self.services.file_list()
    }

    /// Shared objects registered by type.
    #[must_use]
    pub fn extensions(&self) -> &Extensions {
        &self.extensions
    }

    /// Position of the running stage.
    #[must_use]
    pub fn frame(&self) -> &StageFrame {
        &self.frame
    }

    /// Execution step of the running stage.
    #[must_use]
    pub fn step(&self) -> ExecStep {
        self.frame.step
    }

    /// What the rest of the pipeline needs from the output.
    #[must_use]
    pub fn demand(&self) -> &Demand {
        &self.frame.demand
    }

    /// Emits an event.
    pub fn emit(&self, event: Event) {
        self.events.emit(event);
    }

    /// Emits a progress event of the running stage.
    pub fn progress(&self, phase: &str, done: u64, total: Option<u64>) {
        self.emit(Event::Progress(Progress {
            command: self.frame.command.clone(),
            phase: phase.to_owned(),
            done,
            total,
            bytes: None,
            message: None,
        }));
    }

    /// Records an unusual effect or a skipped item for the report, without
    /// ending the program.
    pub fn note(&self, notable: NotableEntry) {
        lock(&self.notes).push(notable);
    }

    /// Takes all notes recorded so far, in recording order.
    #[must_use]
    pub fn take_notes(&self) -> Vec<NotableEntry> {
        std::mem::take(&mut *lock(&self.notes))
    }

    fn missing(&self, what: &str) -> Error {
        contract_error(
            ErrorCode::StageFailed,
            format!(
                "the execution context of `{}` has no {what}",
                self.frame.command
            ),
        )
    }
}

impl fmt::Debug for ExecCtx {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExecCtx")
            .field("frame", &self.frame)
            .field("cancel", &self.cancel)
            .field("deadline", &self.deadline)
            .field("limits", &self.limits)
            .field("fs", &self.fs.is_some())
            .field("journal", &self.journal.is_some())
            .field("services", &self.services)
            .finish_non_exhaustive()
    }
}

/// Builder of an [`ExecCtx`].
///
/// Facilities that are not set are absent ([`ExecCtx::fs`],
/// [`ExecCtx::journal`], evaluator, pipeline runner), use their defaults
/// (configuration, request options, limits, frame, a registry without
/// services) or are fresh (a new cancellation token, an event sink that drops
/// events).
pub struct ExecCtxBuilder {
    clock: Arc<dyn Clock>,
    host: Arc<dyn Host>,
    fs: Option<Arc<dyn Fs>>,
    journal: Option<Arc<dyn JournalStore>>,
    config: Option<Arc<Config>>,
    options: Option<Arc<RequestOptions>>,
    events: Option<Arc<dyn EventSink>>,
    cancel: Option<CancelToken>,
    timeout: Option<Duration>,
    limits: Limits,
    state_dir: Option<PathBuf>,
    evaluator: Option<Arc<dyn ExprEvaluator>>,
    pipelines: Option<Arc<dyn PipelineRunner>>,
    services: Arc<Services>,
    extensions: Extensions,
    frame: StageFrame,
}

impl ExecCtxBuilder {
    /// Sets the read-only file system.
    #[must_use]
    pub fn fs(mut self, fs: Arc<dyn Fs>) -> Self {
        self.fs = Some(fs);
        self
    }

    /// Sets the journal store.
    #[must_use]
    pub fn journal(mut self, journal: Arc<dyn JournalStore>) -> Self {
        self.journal = Some(journal);
        self
    }

    /// Sets the configuration; the default configuration otherwise.
    #[must_use]
    pub fn config(mut self, config: Arc<Config>) -> Self {
        self.config = Some(config);
        self
    }

    /// Sets the request options; the default options otherwise.
    #[must_use]
    pub fn options(mut self, options: Arc<RequestOptions>) -> Self {
        self.options = Some(options);
        self
    }

    /// Sets the event sink.
    #[must_use]
    pub fn events(mut self, events: Arc<dyn EventSink>) -> Self {
        self.events = Some(events);
        self
    }

    /// Sets the cancellation token.
    #[must_use]
    pub fn cancel(mut self, cancel: CancelToken) -> Self {
        self.cancel = Some(cancel);
        self
    }

    /// Sets the time limit of the program, measured from
    /// [`build`](ExecCtxBuilder::build) on the context clock.
    #[must_use]
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Sets the resource limits.
    #[must_use]
    pub fn limits(mut self, limits: Limits) -> Self {
        self.limits = limits;
        self
    }

    /// Sets the state directory.
    #[must_use]
    pub fn state_dir(mut self, dir: PathBuf) -> Self {
        self.state_dir = Some(dir);
        self
    }

    /// Sets the evaluator of per-record expressions.
    #[must_use]
    pub fn evaluator(mut self, evaluator: Arc<dyn ExprEvaluator>) -> Self {
        self.evaluator = Some(evaluator);
        self
    }

    /// Sets the runner of pipeline arguments.
    #[must_use]
    pub fn pipelines(mut self, pipelines: Arc<dyn PipelineRunner>) -> Self {
        self.pipelines = Some(pipelines);
        self
    }

    /// Sets the registry of services; a registry without services otherwise.
    #[must_use]
    pub fn services(mut self, services: Arc<Services>) -> Self {
        self.services = services;
        self
    }

    /// Sets the registry of shared objects.
    #[must_use]
    pub fn extensions(mut self, extensions: Extensions) -> Self {
        self.extensions = extensions;
        self
    }

    /// Sets the stage frame.
    #[must_use]
    pub fn frame(mut self, frame: StageFrame) -> Self {
        self.frame = frame;
        self
    }

    /// Builds the context.
    #[must_use]
    pub fn build(self) -> ExecCtx {
        let deadline = self
            .timeout
            .map(|timeout| self.clock.monotonic().saturating_add(timeout));
        ExecCtx {
            clock: self.clock,
            host: self.host,
            fs: self.fs,
            journal: self.journal,
            config: self.config.unwrap_or_default(),
            options: self.options.unwrap_or_default(),
            events: self.events.unwrap_or_else(|| Arc::new(DiscardEvents)),
            cancel: self.cancel.unwrap_or_default(),
            deadline,
            limits: self.limits,
            state_dir: self.state_dir,
            evaluator: self.evaluator,
            pipelines: self.pipelines,
            services: self.services,
            extensions: Arc::new(self.extensions),
            frame: self.frame,
            notes: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl fmt::Debug for ExecCtxBuilder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExecCtxBuilder")
            .field("frame", &self.frame)
            .field("limits", &self.limits)
            .field("timeout", &self.timeout)
            .finish_non_exhaustive()
    }
}
