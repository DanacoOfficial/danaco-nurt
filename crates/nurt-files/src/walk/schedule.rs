// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Scheduling of a walk.
//!
//! Worker threads take directories from a shared stack, read and filter them
//! and store the result as a listing: the events of one directory in name
//! order, where an event is an entry, a problem or a reference to the listing
//! of a subdirectory. The calling thread replays the listings depth first,
//! waiting for each referenced listing in turn, so the delivered order is
//! independent of scheduling.
//!
//! Workers read ahead of the caller until [`READ_AHEAD_LIMIT`] events are
//! buffered; past that limit they read only the directory the caller is
//! waiting for, which bounds memory without blocking progress.

use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::mem;
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::{self, Scope};
use std::time::Duration;

use ignore::gitignore::Gitignore;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::{
    AbsPath, DirEntry, FileId, FileKind, FileMeta, Follow, Fs, FsError, FsErrorKind,
};

use super::filter::{self, GIT_DIR};
use super::rules::{self, Frame, GITIGNORE_FILE, GitMarker, IGNORE_FILE, Markers};
use super::{Entry, SAMPLE_LIMIT, Unreadable, WalkItem, WalkSummary, Walker};

/// Events buffered ahead of the caller before workers stop reading ahead.
const READ_AHEAD_LIMIT: usize = 65_536;

/// Longest wait for a listing before the checkpoint is called again.
const WAIT_SLICE: Duration = Duration::from_millis(50);

/// Delivered items between two calls of the checkpoint.
const CHECKPOINT_INTERVAL: u32 = 1024;

pub(super) fn run<F>(
    walker: &Walker,
    fs: &dyn Fs,
    roots: &[AbsPath],
    checkpoint: &dyn Fn() -> Result<()>,
    mut sink: F,
) -> Result<WalkSummary>
where
    F: FnMut(WalkItem) -> ControlFlow<()>,
{
    let options = walker.options();
    let mut pending = Vec::new();
    let global = match (&options.global_ignore_file, options.gitignore) {
        (Some(path), true) => rules::read_global(fs, path).unwrap_or_else(|error| {
            pending.push(Problem::Unreadable(Unreadable {
                path: path.clone(),
                depth: 0,
                error,
            }));
            None
        }),
        _ => None,
    };
    let context = Context {
        walker,
        fs,
        global: global.as_ref(),
        shared: Shared::default(),
    };
    let mut summary = WalkSummary {
        complete: true,
        ..WalkSummary::default()
    };
    let flow = thread::scope(|scope| {
        let _shutdown = ShutdownGuard(&context.shared);
        let mut consumer = Consumer {
            context: &context,
            scope,
            workers: 0,
            checkpoint,
            sink: &mut sink,
            summary: &mut summary,
            since_checkpoint: 0,
        };
        for problem in pending {
            if consumer.problem(problem)?.is_break() {
                return Ok(ControlFlow::Break(()));
            }
        }
        consumer.walk(roots)
    })?;
    summary.complete = flow.is_continue();
    Ok(summary)
}

struct Context<'a> {
    walker: &'a Walker,
    fs: &'a dyn Fs,
    global: Option<&'a Gitignore>,
    shared: Shared,
}

/// Directory to read.
struct Job {
    id: u64,
    dir: AbsPath,
    relative: PathBuf,
    depth: u32,
    root: usize,
    rules: Option<Arc<Frame>>,
    ancestors: Option<Arc<Ancestor>>,
}

/// Identity of a directory on the current path, for loop detection.
struct Ancestor {
    id: FileId,
    parent: Option<Arc<Ancestor>>,
}

fn has_ancestor(chain: Option<&Arc<Ancestor>>, id: FileId) -> bool {
    std::iter::successors(chain.map(Arc::as_ref), |ancestor| {
        ancestor.parent.as_deref()
    })
    .any(|ancestor| ancestor.id == id)
}

enum Problem {
    Unreadable(Unreadable),
    Loop(AbsPath),
}

enum Event {
    Entry(Entry),
    Problem(Problem),
    Descend(u64),
}

struct Listing {
    dir: AbsPath,
    depth: u32,
    events: Vec<Event>,
    failure: Option<FsError>,
}

#[derive(Default)]
struct State {
    queue: Vec<Job>,
    done: HashMap<u64, Listing>,
    buffered: usize,
    wanted: Option<u64>,
}

impl State {
    fn pick(&mut self) -> Option<Job> {
        if let Some(wanted) = self.wanted
            && let Some(position) = self.queue.iter().rposition(|job| job.id == wanted)
        {
            return Some(self.queue.remove(position));
        }
        if self.buffered < READ_AHEAD_LIMIT {
            self.queue.pop()
        } else {
            None
        }
    }
}

#[derive(Default)]
struct Shared {
    state: Mutex<State>,
    work: Condvar,
    ready: Condvar,
    stop: AtomicBool,
    next_id: AtomicU64,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn next_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    fn stopped(&self) -> bool {
        self.stop.load(Ordering::Acquire)
    }

    /// Queues jobs so that the first one is taken first.
    fn push(&self, jobs: Vec<Job>) {
        if jobs.is_empty() {
            return;
        }
        self.lock().queue.extend(jobs.into_iter().rev());
        self.work.notify_all();
    }

    fn next_job(&self) -> Option<Job> {
        let mut state = self.lock();
        loop {
            if self.stopped() {
                return None;
            }
            if let Some(job) = state.pick() {
                return Some(job);
            }
            state = self
                .work
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }

    fn finish(&self, id: u64, listing: Listing, jobs: Vec<Job>) {
        {
            let mut state = self.lock();
            state.buffered += listing.events.len();
            state.done.insert(id, listing);
        }
        self.ready.notify_all();
        self.push(jobs);
    }

    fn shutdown(&self) {
        self.stop.store(true, Ordering::Release);
        drop(self.lock());
        self.work.notify_all();
    }
}

/// Stops the workers when the caller leaves the scope, also on an error or
/// a panic, so that the scope can join them.
struct ShutdownGuard<'a>(&'a Shared);

impl Drop for ShutdownGuard<'_> {
    fn drop(&mut self) {
        self.0.shutdown();
    }
}

struct Consumer<'c, 'scope, 'env, F> {
    context: &'env Context<'env>,
    scope: &'scope Scope<'scope, 'env>,
    workers: usize,
    checkpoint: &'c dyn Fn() -> Result<()>,
    sink: &'c mut F,
    summary: &'c mut WalkSummary,
    since_checkpoint: u32,
}

impl<'scope, 'env, F> Consumer<'_, 'scope, 'env, F>
where
    F: FnMut(WalkItem) -> ControlFlow<()>,
{
    fn walk(&mut self, roots: &[AbsPath]) -> Result<ControlFlow<()>> {
        for (index, root) in roots.iter().enumerate() {
            (self.checkpoint)()?;
            if self.walk_root(index, root)?.is_break() {
                return Ok(ControlFlow::Break(()));
            }
        }
        Ok(ControlFlow::Continue(()))
    }

    fn walk_root(&mut self, index: usize, root: &AbsPath) -> Result<ControlFlow<()>> {
        let context = self.context;
        let options = context.walker.options();
        let stored = context.fs.metadata(root, Follow::NoFollow)?;
        let resolved = if is_link(stored.kind) {
            match context.fs.metadata(root, Follow::Follow) {
                Ok(meta) => Some(meta),
                Err(error) if is_unresolved(&error) => None,
                Err(error) => return Err(error.into()),
            }
        } else {
            None
        };
        let directory = if is_link(stored.kind) {
            resolved.as_ref()
        } else {
            Some(&stored)
        }
        .filter(|meta| meta.kind == FileKind::Directory);
        let directory_id = directory.map(|meta| meta.file_id);
        let target_kind = if options.follow_links {
            resolved.as_ref().map(|meta| meta.kind)
        } else {
            None
        };
        let name = root
            .file_name()
            .map(OsStr::to_os_string)
            .unwrap_or_default();
        let delivered = options.min_depth == 0
            && filter::kind_matches(options.kind, stored.kind, target_kind)
            && filter::glob_matches(context.walker.globs.as_ref(), Path::new(&name));
        if delivered {
            let meta = if !options.metadata {
                None
            } else if target_kind.is_some() {
                resolved
            } else {
                Some(stored.clone())
            };
            let entry = Entry {
                path: root.clone(),
                relative: PathBuf::new(),
                name,
                depth: 0,
                root: index,
                kind: stored.kind,
                target_kind,
                meta,
            };
            if self.entry(entry)?.is_break() {
                return Ok(ControlFlow::Break(()));
            }
        }
        if directory_id.is_none() || options.max_depth == Some(0) {
            return Ok(ControlFlow::Continue(()));
        }
        let mut problems = Vec::new();
        let rules = if options.gitignore {
            rules::ancestor_frames(context.fs, root, &mut problems)
        } else {
            None
        };
        for (path, error) in problems {
            let problem = Problem::Unreadable(Unreadable {
                path,
                depth: 0,
                error,
            });
            if self.problem(problem)?.is_break() {
                return Ok(ControlFlow::Break(()));
            }
        }
        let ancestors = directory_id
            .filter(|_| options.follow_links)
            .map(|id| Arc::new(Ancestor { id, parent: None }));
        let job = Job {
            id: context.shared.next_id(),
            dir: root.clone(),
            relative: PathBuf::new(),
            depth: 0,
            root: index,
            rules,
            ancestors,
        };
        let id = job.id;
        self.ensure_workers()?;
        context.shared.push(vec![job]);
        let listing = self.wait_for(id)?;
        if let Some(error) = listing.failure {
            return Err(error.into());
        }
        self.replay(listing.events)
    }

    /// Delivers the events of a listing and of every listing it refers to,
    /// depth first.
    fn replay(&mut self, events: Vec<Event>) -> Result<ControlFlow<()>> {
        let mut stack = Vec::new();
        let mut current = events.into_iter();
        loop {
            let Some(event) = current.next() else {
                match stack.pop() {
                    Some(parent) => {
                        current = parent;
                        continue;
                    }
                    None => return Ok(ControlFlow::Continue(())),
                }
            };
            let flow = match event {
                Event::Entry(entry) => self.entry(entry)?,
                Event::Problem(problem) => self.problem(problem)?,
                Event::Descend(id) => {
                    let listing = self.wait_for(id)?;
                    match listing.failure {
                        Some(error) => self.problem(Problem::Unreadable(Unreadable {
                            path: listing.dir,
                            depth: listing.depth,
                            error,
                        }))?,
                        None => {
                            stack.push(mem::replace(&mut current, listing.events.into_iter()));
                            ControlFlow::Continue(())
                        }
                    }
                }
            };
            if flow.is_break() {
                return Ok(flow);
            }
        }
    }

    fn entry(&mut self, entry: Entry) -> Result<ControlFlow<()>> {
        self.summary.entries += 1;
        self.since_checkpoint += 1;
        if self.since_checkpoint >= CHECKPOINT_INTERVAL {
            self.since_checkpoint = 0;
            (self.checkpoint)()?;
        }
        Ok((self.sink)(WalkItem::Entry(entry)))
    }

    fn problem(&mut self, problem: Problem) -> Result<ControlFlow<()>> {
        match problem {
            Problem::Loop(path) => {
                self.summary.link_loops += 1;
                push_sample(&mut self.summary.link_loop_sample, path);
                Ok(ControlFlow::Continue(()))
            }
            Problem::Unreadable(unreadable) => {
                if unreadable.error.kind == FsErrorKind::PermissionDenied {
                    self.summary.permission_denied += 1;
                    push_sample(
                        &mut self.summary.permission_denied_sample,
                        unreadable.path.clone(),
                    );
                } else {
                    self.summary.unreadable += 1;
                }
                Ok((self.sink)(WalkItem::Unreadable(unreadable)))
            }
        }
    }

    fn wait_for(&mut self, id: u64) -> Result<Listing> {
        let shared = &self.context.shared;
        let mut state = shared.lock();
        state.wanted = Some(id);
        shared.work.notify_all();
        loop {
            if let Some(listing) = state.done.remove(&id) {
                state.buffered = state.buffered.saturating_sub(listing.events.len());
                state.wanted = None;
                drop(state);
                shared.work.notify_all();
                return Ok(listing);
            }
            let (guard, wait) = shared
                .ready
                .wait_timeout(state, WAIT_SLICE)
                .unwrap_or_else(PoisonError::into_inner);
            state = guard;
            if wait.timed_out() {
                drop(state);
                (self.checkpoint)()?;
                state = shared.lock();
            }
        }
    }

    fn ensure_workers(&mut self) -> Result<()> {
        if self.workers > 0 {
            return Ok(());
        }
        let context = self.context;
        let count = context.walker.options().threads.max(1);
        for index in 0..count {
            let spawned = thread::Builder::new()
                .name(format!("nurt-walk-{index}"))
                .spawn_scoped(self.scope, move || context.work());
            if spawned.is_err() {
                break;
            }
            self.workers += 1;
        }
        if self.workers == 0 {
            return Err(Error::new(
                ErrorCode::Internal,
                "cannot start a thread to read directories",
            ));
        }
        Ok(())
    }
}

fn push_sample(sample: &mut Vec<AbsPath>, path: AbsPath) {
    if sample.len() < SAMPLE_LIMIT {
        sample.push(path);
    }
}

fn is_link(kind: FileKind) -> bool {
    matches!(kind, FileKind::Symlink | FileKind::Junction)
}

/// Failures that mean a link has no usable target: the target is missing, a
/// component of it is not a directory, or the links form a cycle.
fn is_unresolved(error: &FsError) -> bool {
    matches!(
        error.kind,
        FsErrorKind::NotFound | FsErrorKind::NotADirectory | FsErrorKind::Loop
    )
}

impl Context<'_> {
    fn work(&self) {
        while let Some(job) = self.shared.next_job() {
            let id = job.id;
            let (listing, jobs) = self.read(job);
            self.shared.finish(id, listing, jobs);
        }
    }

    fn read(&self, job: Job) -> (Listing, Vec<Job>) {
        let mut listing = Listing {
            dir: job.dir.clone(),
            depth: job.depth,
            events: Vec::new(),
            failure: None,
        };
        let mut jobs = Vec::new();
        let entries = match self.list(&job.dir, job.depth + 1, &mut listing.events) {
            Ok(entries) => entries,
            Err(error) => {
                listing.failure = Some(error);
                return (listing, jobs);
            }
        };
        let rules = if self.walker.options().gitignore {
            let markers = self.markers(&job.dir, &entries);
            let mut problems = Vec::new();
            let frame = rules::frame_for(
                self.fs,
                job.rules.as_ref(),
                &job.dir,
                markers,
                &mut problems,
            );
            for (path, error) in problems {
                listing
                    .events
                    .push(Event::Problem(Problem::Unreadable(Unreadable {
                        path,
                        depth: job.depth + 1,
                        error,
                    })));
            }
            frame
        } else {
            None
        };
        for entry in entries {
            if self.shared.stopped() {
                break;
            }
            self.visit(&job, rules.as_ref(), entry, &mut listing.events, &mut jobs);
        }
        (listing, jobs)
    }

    /// Reads a directory and sorts its entries by name. Failures of single
    /// entries become problems, sorted by path.
    fn list(
        &self,
        dir: &AbsPath,
        depth: u32,
        events: &mut Vec<Event>,
    ) -> std::result::Result<Vec<DirEntry>, FsError> {
        let mut entries = Vec::new();
        let mut failures = Vec::new();
        for item in self.fs.read_dir(dir)? {
            match item {
                Ok(entry) => entries.push(entry),
                Err(error) => failures.push(error),
            }
        }
        entries.sort_unstable_by(|a, b| a.name.cmp(&b.name));
        failures.sort_by(|a, b| a.path.cmp(&b.path));
        for error in failures {
            let path = error
                .path
                .as_ref()
                .and_then(|path| AbsPath::new(path.clone()).ok())
                .unwrap_or_else(|| dir.clone());
            events.push(Event::Problem(Problem::Unreadable(Unreadable {
                path,
                depth,
                error,
            })));
        }
        Ok(entries)
    }

    fn markers(&self, dir: &AbsPath, entries: &[DirEntry]) -> Markers {
        let mut markers = Markers::default();
        for entry in entries {
            let Some(name) = entry.name.to_str() else {
                continue;
            };
            match name {
                GIT_DIR => {
                    let kind = entry.kind.or_else(|| {
                        let path = dir.join_name(GIT_DIR).ok()?;
                        self.fs
                            .metadata(&path, Follow::NoFollow)
                            .ok()
                            .map(|meta| meta.kind)
                    });
                    markers.git = Some(if kind == Some(FileKind::Directory) {
                        GitMarker::Directory
                    } else {
                        GitMarker::File
                    });
                }
                IGNORE_FILE => markers.ignore = entry.kind != Some(FileKind::Directory),
                GITIGNORE_FILE => markers.gitignore = entry.kind != Some(FileKind::Directory),
                _ => {}
            }
        }
        markers
    }

    /// Applies the rules to one entry of a listing and records the result.
    fn visit(
        &self,
        job: &Job,
        rules: Option<&Arc<Frame>>,
        dir_entry: DirEntry,
        events: &mut Vec<Event>,
        jobs: &mut Vec<Job>,
    ) {
        let options = self.walker.options();
        let depth = job.depth + 1;
        let Some(mut candidate) = self.inspect(job, dir_entry, events) else {
            return;
        };
        let is_dir = candidate.effective_kind() == FileKind::Directory;
        let relative = job.relative.join(&candidate.name);
        if self.excluded(rules, &candidate, &relative, is_dir) {
            return;
        }
        let descent = if is_dir && options.max_depth.is_none_or(|max| depth < max) {
            self.descent(job, &mut candidate, events)
        } else {
            Descent::Stop
        };
        let delivered = depth >= options.min_depth
            && filter::kind_matches(options.kind, candidate.kind, candidate.target_kind())
            && filter::glob_matches(self.walker.globs.as_ref(), &relative);
        if delivered {
            let target_kind = candidate.target_kind();
            let meta = self.entry_meta(&mut candidate, events);
            events.push(Event::Entry(Entry {
                path: candidate.path.clone(),
                relative: relative.clone(),
                name: candidate.name,
                depth,
                root: job.root,
                kind: candidate.kind,
                target_kind,
                meta,
            }));
        }
        match descent {
            Descent::Stop => {}
            Descent::Loop => events.push(Event::Problem(Problem::Loop(candidate.path))),
            Descent::Enter(ancestors) => {
                let id = self.shared.next_id();
                events.push(Event::Descend(id));
                jobs.push(Job {
                    id,
                    dir: candidate.path,
                    relative,
                    depth,
                    root: job.root,
                    rules: rules.cloned(),
                    ancestors,
                });
            }
        }
    }

    /// Reads what the rules need to know about an entry: its kind, its
    /// metadata when the listing does not suffice, and the target of a link
    /// that is followed. Returns `None` for an entry that is hidden or
    /// cannot be read.
    fn inspect(
        &self,
        job: &Job,
        dir_entry: DirEntry,
        events: &mut Vec<Event>,
    ) -> Option<Candidate> {
        let options = self.walker.options();
        let depth = job.depth + 1;
        let path = match AbsPath::new(job.dir.as_path().join(&dir_entry.name)) {
            Ok(path) => path,
            Err(error) => {
                record_failure(events, job.dir.clone(), depth, error);
                return None;
            }
        };
        let mut stored = None;
        let kind = match dir_entry.kind {
            Some(kind) if !filter::hidden_needs_metadata(options.hidden) => kind,
            _ => match self.fs.metadata(&path, Follow::NoFollow) {
                Ok(meta) => stored.insert(meta).kind,
                Err(error) => {
                    record_failure(events, path, depth, error);
                    return None;
                }
            },
        };
        if !options.hidden && filter::is_hidden(&dir_entry.name, stored.as_ref()) {
            return None;
        }
        let target = if options.follow_links && is_link(kind) {
            match self.fs.metadata(&path, Follow::Follow) {
                Ok(meta) => Some(meta),
                Err(error) => {
                    if !is_unresolved(&error) {
                        record_failure(events, path.clone(), depth, error);
                    }
                    None
                }
            }
        } else {
            None
        };
        Some(Candidate {
            path,
            depth,
            name: dir_entry.name,
            kind,
            stored,
            target,
        })
    }

    fn excluded(
        &self,
        rules: Option<&Arc<Frame>>,
        candidate: &Candidate,
        relative: &Path,
        is_dir: bool,
    ) -> bool {
        filter::excluded_by_preset(&self.walker.presets, &candidate.name, is_dir)
            || filter::excluded_by_patterns(self.walker.exclude.as_ref(), relative, is_dir)
            || rules.is_some_and(|frame| {
                frame.is_ignored(candidate.path.as_path(), is_dir, self.global)
            })
    }

    /// Decides whether to enter a directory. With `follow_links`, a
    /// directory whose identity matches a directory on the current path is
    /// a loop and is not entered.
    fn descent(&self, job: &Job, candidate: &mut Candidate, events: &mut Vec<Event>) -> Descent {
        if !self.walker.options().follow_links {
            return Descent::Enter(None);
        }
        let identity = match candidate.target.as_ref().or(candidate.stored.as_ref()) {
            Some(meta) => Ok(meta.file_id),
            None => self
                .fs
                .metadata(&candidate.path, Follow::NoFollow)
                .map(|meta| candidate.stored.insert(meta).file_id),
        };
        match identity {
            Ok(id) if has_ancestor(job.ancestors.as_ref(), id) => Descent::Loop,
            Ok(id) => Descent::Enter(Some(Arc::new(Ancestor {
                id,
                parent: job.ancestors.clone(),
            }))),
            Err(error) => {
                record_failure(events, candidate.path.clone(), candidate.depth, error);
                Descent::Stop
            }
        }
    }

    /// Metadata of a delivered entry, when the options request it: the
    /// target of a followed link, otherwise the entry itself.
    fn entry_meta(&self, candidate: &mut Candidate, events: &mut Vec<Event>) -> Option<FileMeta> {
        if !self.walker.options().metadata {
            return None;
        }
        if let Some(meta) = candidate.target.take().or_else(|| candidate.stored.take()) {
            return Some(meta);
        }
        match self.fs.metadata(&candidate.path, Follow::NoFollow) {
            Ok(meta) => Some(meta),
            Err(error) => {
                record_failure(events, candidate.path.clone(), candidate.depth, error);
                None
            }
        }
    }
}

/// Entry of a listing while the rules are applied to it.
struct Candidate {
    path: AbsPath,
    depth: u32,
    name: OsString,
    kind: FileKind,
    stored: Option<FileMeta>,
    target: Option<FileMeta>,
}

impl Candidate {
    fn target_kind(&self) -> Option<FileKind> {
        self.target.as_ref().map(|meta| meta.kind)
    }

    fn effective_kind(&self) -> FileKind {
        self.target_kind().unwrap_or(self.kind)
    }
}

/// Outcome of [`Context::descent`].
enum Descent {
    /// The entry is not entered.
    Stop,
    /// The entry leads back to a directory on the current path.
    Loop,
    /// The entry is entered; with `follow_links`, the identities of the
    /// directories on the new path.
    Enter(Option<Arc<Ancestor>>),
}

/// Records a failure to read an entry. An entry removed after its directory
/// was listed is skipped without a problem, as if it had not been listed.
fn record_failure(events: &mut Vec<Event>, path: AbsPath, depth: u32, error: FsError) {
    if error.kind != FsErrorKind::NotFound {
        events.push(Event::Problem(Problem::Unreadable(Unreadable {
            path,
            depth,
            error,
        })));
    }
}
