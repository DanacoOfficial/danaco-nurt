// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-testkit::ctx
// Responsibility:   Execution contexts assembled from the doubles of this crate.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Execution contexts assembled from the doubles of this crate.

use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use nurt_contracts::report::NotableEntry;
use nurt_contracts::services::{ProcessHost, Services};
use nurt_contracts::stage::{
    Event, EventSink, ExecCtx, ExecCtxBuilder, ExprEvaluator, PipelineRunner,
};

use crate::clock::VirtualClock;
use crate::host::TestHost;
use crate::journal::RecordingJournal;
use crate::services::{ScriptedProcessHost, test_services};
use crate::vfs::VirtualFs;

/// Event sink that keeps every event in order.
#[derive(Debug, Default)]
pub struct RecordingEvents {
    events: Mutex<Vec<Event>>,
}

impl RecordingEvents {
    fn lock(&self) -> MutexGuard<'_, Vec<Event>> {
        self.events.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Events received so far.
    #[must_use]
    pub fn events(&self) -> Vec<Event> {
        self.lock().clone()
    }

    /// Removes and returns the events received so far.
    #[must_use]
    pub fn take(&self) -> Vec<Event> {
        std::mem::take(&mut *self.lock())
    }
}

impl EventSink for RecordingEvents {
    fn emit(&self, event: Event) {
        self.lock().push(event);
    }
}

/// Doubles of one test: a virtual clock, a host, a virtual file system on
/// that clock, a recording journal over that file system, a recording event
/// sink and the service doubles of [`test_services`].
///
/// [`TestContext::builder`] returns an [`ExecCtxBuilder`] with all of them
/// set, together with the expression evaluator and the pipeline runner when
/// the test supplied them; the test adds what it needs (configuration,
/// options, frame) and builds the context. The doubles stay accessible for
/// setup and assertions. Notes recorded by a stage ([`ExecCtx::note`]) are
/// read with [`TestContext::notes`].
///
/// ```
/// use std::time::Duration;
///
/// use nurt_contracts::fs::{Follow, Fs};
/// use nurt_testkit::ctx::TestContext;
/// use nurt_testkit::vfs::test_path;
///
/// let test = TestContext::new();
/// let path = test_path("/data/a.txt").unwrap();
/// test.fs().write_file(&path, b"text").unwrap();
///
/// let ctx = test.builder().timeout(Duration::from_secs(10)).build();
/// let fs = ctx.require_fs().unwrap();
/// assert_eq!(fs.metadata(&path, Follow::Follow).unwrap().size, 4);
///
/// test.clock().advance(Duration::from_secs(11));
/// assert!(ctx.checkpoint().is_err());
/// ```
pub struct TestContext {
    clock: Arc<VirtualClock>,
    host: Arc<TestHost>,
    fs: Arc<VirtualFs>,
    journal: Arc<RecordingJournal>,
    events: Arc<RecordingEvents>,
    services: Arc<Services>,
    evaluator: Option<Arc<dyn ExprEvaluator>>,
    pipelines: Option<Arc<dyn PipelineRunner>>,
}

impl TestContext {
    /// Doubles with their defaults; the host is [`TestHost::native`].
    #[must_use]
    pub fn new() -> Self {
        Self::with_host(TestHost::native())
    }

    /// Doubles with the given host.
    #[must_use]
    pub fn with_host(host: TestHost) -> Self {
        let clock = Arc::new(VirtualClock::new());
        let fs = Arc::new(VirtualFs::new(clock.clone()));
        let journal = Arc::new(RecordingJournal::new(fs.clone()));
        Self {
            clock,
            host: Arc::new(host),
            fs,
            journal,
            events: Arc::new(RecordingEvents::default()),
            services: Arc::new(test_services()),
            evaluator: None,
            pipelines: None,
        }
    }

    /// Replaces the journal by a [`RecordingJournal::keeping`] journal over
    /// the same file system, for tests of rollback and of
    /// `E_ROLLBACK_FAILED`.
    #[must_use]
    pub fn keeping(mut self) -> Self {
        self.journal = Arc::new(RecordingJournal::keeping(self.fs.clone()));
        self
    }

    /// Replaces the service registry, for tests that supply their own
    /// services.
    #[must_use]
    pub fn with_services(mut self, services: Services) -> Self {
        self.services = Arc::new(services);
        self
    }

    /// Registers `host` as the process host, keeping the other services.
    #[must_use]
    pub fn with_process_host(mut self, host: Arc<dyn ProcessHost>) -> Self {
        let services = self.services.as_ref().clone().with_process_host(host);
        self.services = Arc::new(services);
        self
    }

    /// Sets the evaluator of per-record expressions supplied by the test.
    #[must_use]
    pub fn with_evaluator(mut self, evaluator: Arc<dyn ExprEvaluator>) -> Self {
        self.evaluator = Some(evaluator);
        self
    }

    /// Sets the runner of pipeline arguments supplied by the test.
    #[must_use]
    pub fn with_pipelines(mut self, pipelines: Arc<dyn PipelineRunner>) -> Self {
        self.pipelines = Some(pipelines);
        self
    }

    /// Scripted process host on the clock and file system of this context;
    /// register it with [`TestContext::with_process_host`].
    #[must_use]
    pub fn scripted_processes(&self) -> Arc<ScriptedProcessHost> {
        Arc::new(ScriptedProcessHost::new(self.clock.clone()).with_fs(self.fs.clone()))
    }

    /// Virtual clock of the context.
    #[must_use]
    pub fn clock(&self) -> &Arc<VirtualClock> {
        &self.clock
    }

    /// Host of the context.
    #[must_use]
    pub fn host(&self) -> &Arc<TestHost> {
        &self.host
    }

    /// Virtual file system of the context.
    #[must_use]
    pub fn fs(&self) -> &Arc<VirtualFs> {
        &self.fs
    }

    /// Journal of the context.
    #[must_use]
    pub fn journal(&self) -> &Arc<RecordingJournal> {
        &self.journal
    }

    /// Event sink of the context.
    #[must_use]
    pub fn events(&self) -> &Arc<RecordingEvents> {
        &self.events
    }

    /// Service registry of the context.
    #[must_use]
    pub fn services(&self) -> &Arc<Services> {
        &self.services
    }

    /// Builder of an execution context with every double set.
    #[must_use]
    pub fn builder(&self) -> ExecCtxBuilder {
        let mut builder = ExecCtx::builder(self.clock.clone(), self.host.clone())
            .fs(self.fs.clone())
            .journal(self.journal.clone())
            .events(self.events.clone())
            .services(self.services.clone());
        if let Some(evaluator) = &self.evaluator {
            builder = builder.evaluator(evaluator.clone());
        }
        if let Some(pipelines) = &self.pipelines {
            builder = builder.pipelines(pipelines.clone());
        }
        builder
    }

    /// Execution context with every double set and defaults for the rest.
    #[must_use]
    pub fn ctx(&self) -> ExecCtx {
        self.builder().build()
    }

    /// Takes the notes recorded through `ctx` and every context derived
    /// from it with [`ExecCtx::for_stage`], in recording order.
    #[must_use]
    pub fn notes(&self, ctx: &ExecCtx) -> Vec<NotableEntry> {
        ctx.take_notes()
    }
}

impl Default for TestContext {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for TestContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TestContext")
            .field("clock", &self.clock)
            .field("fs", &self.fs)
            .field("journal", &self.journal)
            .field("services", &self.services)
            .field("evaluator", &self.evaluator.is_some())
            .field("pipelines", &self.pipelines.is_some())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use nurt_contracts::bound::BoundExpr;
    use nurt_contracts::error::Result;
    use nurt_contracts::report::{Notable, NotableEntry};
    use nurt_contracts::services::ProgramSpec;
    use nurt_contracts::stage::{Event, ExecCtx, ExprEvaluator, LogEvent, LogLevel, StageFrame};
    use nurt_contracts::value::{Record, Value};

    use super::TestContext;
    use crate::services::ProgramScript;

    struct Constant;

    impl ExprEvaluator for Constant {
        fn eval(
            &self,
            _expr: &BoundExpr,
            _record: Option<&Record>,
            _ctx: &ExecCtx,
        ) -> Result<Value> {
            Ok(Value::Int(42))
        }
    }

    #[test]
    fn context_carries_every_double() {
        let test = TestContext::new();
        let ctx = test.ctx();
        assert!(ctx.fs().is_some());
        assert!(ctx.journal().is_some());
        assert!(ctx.doc().is_some());
        assert!(ctx.code().is_some());
        assert!(ctx.process().is_some());
        assert!(ctx.files().is_some());
        assert!(ctx.evaluator().is_err());
        assert!(ctx.pipelines().is_err());
    }

    #[test]
    fn events_are_recorded_in_order() {
        let test = TestContext::new();
        let ctx = test.ctx();
        ctx.progress("scan", 1, Some(2));
        ctx.emit(Event::Log(LogEvent {
            level: LogLevel::Info,
            message: "done".to_owned(),
            source: None,
        }));
        let events = test.events().take();
        assert_eq!(events.len(), 2);
        assert!(matches!(events[0], Event::Progress(_)));
        assert!(test.events().events().is_empty());
    }

    #[test]
    fn notes_of_derived_contexts_are_collected() {
        let test = TestContext::new();
        let ctx = test.ctx();
        let stage = ctx.for_stage(StageFrame::default());
        stage.note(NotableEntry::new(Notable::NothingMatched));
        let notes = test.notes(&ctx);
        assert_eq!(notes.len(), 1);
        assert!(test.notes(&ctx).is_empty());
    }

    #[test]
    fn supplied_evaluator_reaches_the_context() {
        let test = TestContext::new().with_evaluator(Arc::new(Constant));
        let ctx = test.ctx();
        assert!(ctx.evaluator().is_ok());
        assert!(format!("{test:?}").contains("evaluator: true"));
    }

    #[test]
    fn scripted_processes_run_on_the_context_clock() {
        let test = TestContext::new();
        let host = test.scripted_processes();
        host.script(ProgramScript::new("make").runs_for(Duration::from_secs(4)));
        let test = test.with_process_host(host.clone());
        let ctx = test.ctx();
        let process = ctx.process().unwrap();
        let spec = ProgramSpec::new("make", Vec::new(), "/");
        let outcome = process.run_program(&spec, ctx.cancel()).unwrap();
        assert_eq!(outcome.duration_ms, 4_000);
        assert_eq!(test.clock().sleeps(), [Duration::from_secs(4)]);
        assert!(ctx.doc().is_some());
        assert_eq!(host.runs().len(), 1);
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
