// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Execution contexts assembled from the doubles of this crate.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use nurt_contracts::services::Services;
use nurt_contracts::stage::{Event, EventSink, ExecCtx, ExecCtxBuilder};

use crate::clock::VirtualClock;
use crate::host::TestHost;
use crate::journal::RecordingJournal;
use crate::services::test_services;
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
/// set; the test adds what it needs (configuration, options, frame) and
/// builds the context. The doubles stay accessible for setup and
/// assertions.
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
#[derive(Debug)]
pub struct TestContext {
    clock: Arc<VirtualClock>,
    host: Arc<TestHost>,
    fs: Arc<VirtualFs>,
    journal: Arc<RecordingJournal>,
    events: Arc<RecordingEvents>,
    services: Arc<Services>,
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
        }
    }

    /// Replaces the service registry, for tests that supply their own
    /// services.
    #[must_use]
    pub fn with_services(mut self, services: Services) -> Self {
        self.services = Arc::new(services);
        self
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

    /// Builder of an execution context with every double set.
    #[must_use]
    pub fn builder(&self) -> ExecCtxBuilder {
        ExecCtx::builder(self.clock.clone(), self.host.clone())
            .fs(self.fs.clone())
            .journal(self.journal.clone())
            .events(self.events.clone())
            .services(self.services.clone())
    }

    /// Execution context with every double set and defaults for the rest.
    #[must_use]
    pub fn ctx(&self) -> ExecCtx {
        self.builder().build()
    }
}

impl Default for TestContext {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use nurt_contracts::stage::{Event, LogEvent, LogLevel};

    use super::TestContext;

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
}
