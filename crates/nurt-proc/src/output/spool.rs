// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-proc::output::spool
// Responsibility:   Spool files of one run and the factory that creates, reopens and removes.
// Dependencies:     nurt_contracts, serde, sha2
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Spool files of one run and the factory that creates, reopens and removes
//! them.
//!
//! The spool of a run named `id` lives in the `spool` directory of the state
//! directory: `<id>-stdout.txt` and `<id>-stderr.txt`, or `<id>-output.txt`
//! when standard error is merged into standard output, and the record
//! `<id>.json` with the counters, written at the start and when the output
//! ends. Bytes are written to the files as they arrive, without buffering,
//! so readers see every complete line at once.

use std::collections::{HashMap, VecDeque};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::{Duration, SystemTime};

use nurt_contracts::config::ByteLimit;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::stage::{ExecCtx, OutputStream};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::cursor::{Cursor, spool_tag};
use super::select::{self, Mode};
use crate::iface::{OutputSink, OutputStore, OutputView, RenderedOutput, SpoolFactory};

/// Directory of spool files within the state directory.
const SPOOL_DIR: &str = "spool";

/// Longest accepted spool name.
const MAX_ID_LENGTH: usize = 128;

/// Age after which the spool of a run that never ended is removed, when
/// `output.spool_ttl` is shorter; it covers runs of processes that ended
/// abnormally.
const ABANDONED_SPOOL_AGE: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// Largest part of `run.spool_max` kept from the end of the output. It is
/// held in memory until the stream ends, so it bounds the memory of a spool.
const MAX_TAIL_BYTES: u64 = 64 * 1024 * 1024;

/// Size of the buffer used to count lines of a file.
const COUNT_BUFFER_BYTES: usize = 64 * 1024;

/// Unix permissions of spool files: read and write for the account only.
#[cfg(unix)]
const PRIVATE_FILE_MODE: u32 = 0o600;

/// Unix permissions of the spool directory: access for the account only.
#[cfg(unix)]
const PRIVATE_DIR_MODE: u32 = 0o700;

/// Bytes and lines received on one stream.
///
/// Lines are counted by line feed bytes, plus one for a last line without a
/// line feed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct StreamCounters {
    /// Bytes received.
    pub(super) bytes: u64,
    newlines: u64,
    last_newline: bool,
}

impl StreamCounters {
    fn add(&mut self, bytes: &[u8]) {
        self.bytes += bytes.len() as u64;
        self.newlines += memchr::memchr_iter(b'\n', bytes).count() as u64;
        if let Some(last) = bytes.last() {
            self.last_newline = *last == b'\n';
        }
    }

    /// Lines received.
    pub(super) fn lines(&self) -> u64 {
        self.newlines + u64::from(self.bytes > 0 && !self.last_newline)
    }
}

/// Record of a spool kept next to its files.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Meta {
    merged: bool,
    ended: [bool; 2],
    counters: [StreamCounters; 2],
    dropped: u64,
    sha256: Option<String>,
}

impl Meta {
    /// Whether all output has arrived. Merged output ends with standard
    /// output; standard error counts only when bytes were written to it.
    fn is_complete(&self) -> bool {
        if self.merged {
            self.ended[0] && (self.ended[1] || self.counters[1].bytes == 0)
        } else {
            self.ended[0] && self.ended[1]
        }
    }
}

/// One stream file of a spool as seen by a reader.
#[derive(Debug, Clone)]
pub(super) struct Source {
    /// Stream the file is shown as; merged output is shown as standard
    /// output.
    pub(super) stream: OutputStream,
    /// Spool file.
    pub(super) path: PathBuf,
    /// No more bytes follow.
    pub(super) ended: bool,
}

/// State of a spool at one moment.
#[derive(Debug, Clone)]
pub(super) struct Snapshot {
    /// Standard error is merged into standard output.
    pub(super) merged: bool,
    /// Stream files.
    pub(super) sources: Vec<Source>,
    /// Counters of standard output and standard error.
    pub(super) counters: [StreamCounters; 2],
    /// Bytes received but not kept because of `run.spool_max`.
    pub(super) dropped: u64,
    /// Hash of the whole output, after it ended.
    pub(super) sha256: Option<String>,
    /// Tag of cursors of this spool.
    pub(super) tag: String,
    /// Default number of first lines shown.
    pub(super) head: u32,
    /// Default number of last lines shown.
    pub(super) tail: u32,
}

#[derive(Debug, Clone)]
struct SpoolPaths {
    dir: PathBuf,
    id: String,
}

impl SpoolPaths {
    fn new(ctx: &ExecCtx, id: &str) -> Result<Self> {
        validate_id(id)?;
        let state = ctx.state_dir().ok_or_else(|| {
            Error::new(
                ErrorCode::Config,
                "program output needs a state directory; configure one for the console",
            )
        })?;
        Ok(Self {
            dir: state.join(SPOOL_DIR),
            id: id.to_owned(),
        })
    }

    fn stream(&self, name: &str) -> PathBuf {
        self.dir.join(format!("{}-{name}.txt", self.id))
    }

    fn stdout(&self) -> PathBuf {
        self.stream("stdout")
    }

    fn stderr(&self) -> PathBuf {
        self.stream("stderr")
    }

    fn merged(&self) -> PathBuf {
        self.stream("output")
    }

    fn meta(&self) -> PathBuf {
        self.dir.join(format!("{}.json", self.id))
    }

    fn streams(&self) -> [PathBuf; 3] {
        [self.stdout(), self.stderr(), self.merged()]
    }
}

fn validate_id(id: &str) -> Result<()> {
    let valid = !id.is_empty()
        && id.len() <= MAX_ID_LENGTH
        && id.starts_with(|first: char| first.is_ascii_alphanumeric())
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'));
    if valid {
        Ok(())
    } else {
        Err(Error::new(
            ErrorCode::Arg,
            format!(
                "spool name {id} is not usable as a file name; use letters, digits, '_', '-' and '.'"
            ),
        ))
    }
}

/// Maps an I/O error on `path` to the error code of its cause.
pub(super) fn io_error(action: &str, path: &Path, error: &io::Error) -> Error {
    let code = match error.kind() {
        io::ErrorKind::NotFound => ErrorCode::NotFound,
        io::ErrorKind::PermissionDenied => ErrorCode::Permission,
        io::ErrorKind::AlreadyExists => ErrorCode::Exists,
        io::ErrorKind::StorageFull => ErrorCode::NoSpace,
        _ => ErrorCode::Io,
    };
    Error::new(code, format!("cannot {action} {}: {error}", path.display()))
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn slot(stream: OutputStream) -> usize {
    match stream {
        OutputStream::Stdout => 0,
        OutputStream::Stderr => 1,
    }
}

fn private_file(path: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(PRIVATE_FILE_MODE);
    }
    options
        .open(path)
        .map_err(|error| io_error("create", path, &error))
}

fn create_dir(dir: &Path) -> Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(PRIVATE_DIR_MODE);
    }
    builder
        .create(dir)
        .map_err(|error| io_error("create", dir, &error))
}

fn write_meta(path: &Path, meta: &Meta) -> Result<()> {
    let text = serde_json::to_vec(meta).map_err(|error| {
        Error::new(
            ErrorCode::Internal,
            format!("cannot encode output record: {error}"),
        )
    })?;
    let mut staged = path.as_os_str().to_owned();
    staged.push(".tmp");
    let staged = PathBuf::from(staged);
    let _ = fs::remove_file(&staged);
    let mut file = private_file(&staged)?;
    let written = file
        .write_all(&text)
        .map_err(|error| io_error("write", &staged, &error));
    drop(file);
    let stored = written
        .and_then(|()| fs::rename(&staged, path).map_err(|error| io_error("write", path, &error)));
    if stored.is_err() {
        let _ = fs::remove_file(&staged);
    }
    stored
}

fn read_meta(path: &Path) -> Result<Meta> {
    let bytes = fs::read(path).map_err(|error| io_error("read", path, &error))?;
    serde_json::from_slice(&bytes).map_err(|error| {
        Error::new(
            ErrorCode::Io,
            format!("output record {} is damaged: {error}", path.display()),
        )
    })
}

/// Counters of a file written by another process.
fn count_file(path: &Path) -> Result<StreamCounters> {
    let mut counters = StreamCounters::default();
    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(counters),
        Err(error) => return Err(io_error("read", path, &error)),
    };
    let mut buffer = vec![0; COUNT_BUFFER_BYTES];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| io_error("read", path, &error))?;
        if read == 0 {
            return Ok(counters);
        }
        counters.add(&buffer[..read]);
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Last bytes of a stream that did not fit in the head of the spool.
#[derive(Default)]
struct Tail {
    bytes: VecDeque<u8>,
    /// Bytes pushed out of the tail, which are in neither part of the file.
    omitted: u64,
}

impl Tail {
    /// Appends `bytes` and keeps the last `capacity` bytes; returns the number
    /// of bytes this call pushed out.
    fn push(&mut self, bytes: &[u8], capacity: usize) -> u64 {
        self.bytes.extend(bytes);
        let over = self.bytes.len().saturating_sub(capacity);
        self.bytes.drain(..over);
        self.omitted += over as u64;
        over as u64
    }
}

/// Parts of the output of a program that are kept when `run.spool_max` is
/// reached: the first `head` bytes of the program and, per file, the last
/// `tail` bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Keep {
    pub(super) head: u64,
    pub(super) tail: usize,
}

impl Keep {
    /// A quarter of the limit, at most [`MAX_TAIL_BYTES`], is kept from the
    /// end, shared by the files of the spool; the rest is the head.
    pub(super) fn of(limit: Option<u64>, merged: bool) -> Self {
        let Some(limit) = limit else {
            return Self {
                head: u64::MAX,
                tail: 0,
            };
        };
        let tail = (limit / 4).min(MAX_TAIL_BYTES);
        let files = if merged { 1 } else { 2 };
        Self {
            head: limit - tail,
            tail: usize::try_from(tail / files).unwrap_or(usize::MAX),
        }
    }
}

/// Writing side of a spool.
struct Writer {
    files: [Option<File>; 2],
    meta: Meta,
    stored: u64,
    hasher: Sha256,
    tails: [Tail; 2],
    last_kept: [Option<u8>; 2],
}

/// Captured output of one run.
pub(super) struct Spool {
    paths: SpoolPaths,
    merged: bool,
    tag: String,
    keep: Keep,
    head: u32,
    tail: u32,
    ctx: ExecCtx,
    writer: Option<Mutex<Writer>>,
    activity: AtomicU64,
}

impl Spool {
    /// Creates the files of a spool; the files created before a failure are
    /// removed, files of another run with the same name are not touched.
    fn create(ctx: &ExecCtx, paths: SpoolPaths, merged: bool) -> Result<Self> {
        let mut created: Vec<PathBuf> = Vec::new();
        let opened = Self::create_files(&paths, merged, &mut created);
        let (files, meta) = match opened {
            Ok(opened) => opened,
            Err(error) => {
                for path in &created {
                    let _ = fs::remove_file(path);
                }
                return Err(error);
            }
        };
        let limit = match ctx.config().run.spool_max {
            ByteLimit::Bytes(size) => Some(size.bytes()),
            ByteLimit::Unlimited => None,
        };
        let writer = Writer {
            files,
            meta,
            stored: 0,
            hasher: Sha256::new(),
            tails: [Tail::default(), Tail::default()],
            last_kept: [None, None],
        };
        Ok(Self::with(ctx, paths, merged, limit, Some(writer)))
    }

    fn create_files(
        paths: &SpoolPaths,
        merged: bool,
        created: &mut Vec<PathBuf>,
    ) -> Result<([Option<File>; 2], Meta)> {
        let mut open = |path: PathBuf| {
            let file = private_file(&path)?;
            created.push(path);
            Ok::<File, Error>(file)
        };
        let files = if merged {
            [Some(open(paths.merged())?), None]
        } else {
            [Some(open(paths.stdout())?), Some(open(paths.stderr())?)]
        };
        let meta = Meta {
            merged,
            ..Meta::default()
        };
        created.push(paths.meta());
        write_meta(&paths.meta(), &meta)?;
        Ok((files, meta))
    }

    fn open(ctx: &ExecCtx, paths: SpoolPaths) -> Result<Self> {
        let meta = read_meta(&paths.meta()).map_err(|error| {
            if error.code() == ErrorCode::NotFound {
                Error::new(
                    ErrorCode::NotFound,
                    format!(
                        "no output is kept for {}; spool files are removed after output.spool_ttl",
                        paths.id
                    ),
                )
            } else {
                error
            }
        })?;
        Ok(Self::with(ctx, paths, meta.merged, None, None))
    }

    fn with(
        ctx: &ExecCtx,
        paths: SpoolPaths,
        merged: bool,
        limit: Option<u64>,
        writer: Option<Writer>,
    ) -> Self {
        let output = &ctx.config().output;
        Self {
            tag: spool_tag(&paths.id),
            paths,
            merged,
            keep: Keep::of(limit, merged),
            head: output.head,
            tail: output.tail,
            ctx: ctx.clone(),
            writer: writer.map(Mutex::new),
            activity: AtomicU64::new(0),
        }
    }

    /// Appends the held-back end of the output to the file of `slot`, after
    /// a line that says how many bytes lie between it and the head.
    fn seal(&self, writer: &mut Writer, slot: usize) -> Result<()> {
        let tail = std::mem::take(&mut writer.tails[slot]);
        if tail.bytes.is_empty() && tail.omitted == 0 {
            return Ok(());
        }
        let mut text: Vec<u8> = Vec::with_capacity(tail.bytes.len() + 64);
        if tail.omitted > 0 {
            if writer.last_kept[slot].is_some_and(|byte| byte != b'\n') {
                text.push(b'\n');
            }
            text.extend(format!("[... {} bytes omitted ...]\n", tail.omitted).bytes());
        }
        let (front, back) = tail.bytes.as_slices();
        text.extend_from_slice(front);
        text.extend_from_slice(back);
        if let Some(file) = writer.files[slot].as_mut() {
            file.write_all(&text)
                .map_err(|error| io_error("write", &self.file_path(slot), &error))?;
        }
        Ok(())
    }

    fn file_path(&self, slot: usize) -> PathBuf {
        match (self.merged, slot) {
            (true, _) => self.paths.merged(),
            (false, 0) => self.paths.stdout(),
            (false, _) => self.paths.stderr(),
        }
    }

    fn writer(&self) -> Result<MutexGuard<'_, Writer>> {
        self.writer
            .as_ref()
            .map(lock)
            .ok_or_else(|| Error::new(ErrorCode::Internal, "output of an earlier run is read only"))
    }

    fn snapshot(&self) -> Result<Snapshot> {
        let (meta, counted) = match &self.writer {
            Some(writer) => (lock(writer).meta.clone(), true),
            None => {
                let meta = read_meta(&self.paths.meta())?;
                let counted = meta.is_complete();
                (meta, counted)
            }
        };
        let mut counters = meta.counters;
        if !counted {
            counters = [
                count_file(&self.file_path(0))?,
                if self.merged {
                    StreamCounters::default()
                } else {
                    count_file(&self.file_path(1))?
                },
            ];
        }
        let sources = if self.merged {
            vec![Source {
                stream: OutputStream::Stdout,
                path: self.paths.merged(),
                ended: meta.is_complete(),
            }]
        } else {
            vec![
                Source {
                    stream: OutputStream::Stdout,
                    path: self.paths.stdout(),
                    ended: meta.ended[0],
                },
                Source {
                    stream: OutputStream::Stderr,
                    path: self.paths.stderr(),
                    ended: meta.ended[1],
                },
            ]
        };
        Ok(Snapshot {
            merged: self.merged,
            sources,
            counters,
            dropped: meta.dropped,
            sha256: meta.sha256,
            tag: self.tag.clone(),
            head: self.head,
            tail: self.tail,
        })
    }
}

impl OutputSink for Spool {
    /// Appends `bytes` to the spool file of `stream`, or to the merged file.
    ///
    /// The first bytes of the program are written at once. Beyond the head
    /// allowed by `run.spool_max` the last bytes are held back and written to
    /// the end of the file when the stream ends; the bytes between them are
    /// counted and hashed but not kept.
    fn write(&self, stream: OutputStream, bytes: &[u8]) -> Result<()> {
        if bytes.is_empty() {
            return Ok(());
        }
        let now = self.ctx.clock().monotonic();
        self.activity.store(
            u64::try_from(now.as_nanos())
                .unwrap_or(u64::MAX)
                .saturating_add(1),
            Ordering::Relaxed,
        );
        let mut writer = self.writer()?;
        let slot = slot(stream);
        if writer.meta.ended[slot] {
            return Err(Error::new(
                ErrorCode::Internal,
                "output written after the end of its stream",
            ));
        }
        writer.meta.counters[slot].add(bytes);
        writer.hasher.update(bytes);
        let room = self.keep.head.saturating_sub(writer.stored);
        let keep = usize::try_from(room).unwrap_or(usize::MAX).min(bytes.len());
        let file_slot = if self.merged { 0 } else { slot };
        if keep > 0
            && let Some(file) = writer.files[file_slot].as_mut()
        {
            file.write_all(&bytes[..keep])
                .map_err(|error| io_error("write", &self.file_path(file_slot), &error))?;
            writer.last_kept[file_slot] = Some(bytes[keep - 1]);
        }
        writer.stored += keep as u64;
        if keep < bytes.len() {
            let pushed_out = writer.tails[file_slot].push(&bytes[keep..], self.keep.tail);
            writer.meta.dropped += pushed_out;
        }
        Ok(())
    }

    /// Marks the end of `stream`. After the end of all streams the files are
    /// closed, the hash of the output is fixed and the record is written.
    fn finish(&self, stream: OutputStream) -> Result<()> {
        let mut writer = self.writer()?;
        let slot = slot(stream);
        if writer.meta.ended[slot] {
            return Ok(());
        }
        writer.meta.ended[slot] = true;
        if !self.merged {
            self.seal(&mut writer, slot)?;
            writer.files[slot] = None;
        }
        if writer.meta.is_complete() {
            if self.merged {
                self.seal(&mut writer, 0)?;
            }
            writer.files = [None, None];
            writer.meta.sha256 = Some(hex(&writer.hasher.clone().finalize()));
            write_meta(&self.paths.meta(), &writer.meta)?;
        }
        Ok(())
    }

    /// Reading of the monotonic clock of the context at the last byte.
    fn last_activity(&self) -> Option<Duration> {
        match self.activity.load(Ordering::Relaxed) {
            0 => None,
            stamp => Some(Duration::from_nanos(stamp - 1)),
        }
    }
}

impl OutputStore for Spool {
    fn render(&self, view: &OutputView, exit_code: Option<i32>) -> Result<RenderedOutput> {
        select::respond(&self.snapshot()?, view, Mode::Whole { exit_code })
    }

    fn read_since(&self, cursor: Option<&str>, view: &OutputView) -> Result<RenderedOutput> {
        let from = cursor
            .map(|text| Cursor::parse(text, &self.tag))
            .transpose()?;
        select::respond(&self.snapshot()?, view, Mode::Since { from })
    }
}

/// Spool files in the state directory.
#[derive(Default)]
pub(super) struct SpoolDir {
    live: Mutex<HashMap<PathBuf, Weak<Spool>>>,
}

impl SpoolFactory for SpoolDir {
    /// Creates the spool and removes spools older than `output.spool_ttl`.
    fn create(&self, ctx: &ExecCtx, id: &str, merge_stderr: bool) -> Result<Arc<dyn OutputStore>> {
        let paths = SpoolPaths::new(ctx, id)?;
        create_dir(&paths.dir)?;
        let mut live = lock(&self.live);
        live.retain(|_, spool| spool.strong_count() > 0);
        remove_expired(ctx, &paths.dir, &live);
        let spool = Arc::new(Spool::create(ctx, paths.clone(), merge_stderr)?);
        live.insert(paths.meta(), Arc::downgrade(&spool));
        Ok(spool)
    }

    /// Returns the spool of a run of this process, or reads the files of a
    /// run of another process.
    fn open(&self, ctx: &ExecCtx, id: &str) -> Result<Arc<dyn OutputStore>> {
        let paths = SpoolPaths::new(ctx, id)?;
        if let Some(spool) = lock(&self.live).get(&paths.meta()).and_then(Weak::upgrade) {
            return Ok(spool);
        }
        Ok(Arc::new(Spool::open(ctx, paths)?))
    }

    fn remove(&self, ctx: &ExecCtx, id: &str) -> Result<()> {
        let paths = SpoolPaths::new(ctx, id)?;
        lock(&self.live).remove(&paths.meta());
        remove_files(&paths)
    }
}

/// Removes the stream files first and the record last, so that an
/// interrupted removal leaves a record the next cleanup finds.
fn remove_files(paths: &SpoolPaths) -> Result<()> {
    for path in paths.streams().iter().chain([&paths.meta()]) {
        match fs::remove_file(path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(io_error("remove", path, &error)),
        }
    }
    Ok(())
}

fn wall_clock(ctx: &ExecCtx) -> SystemTime {
    let nanos = u64::try_from(ctx.clock().now().unix_nanos()).unwrap_or(0);
    SystemTime::UNIX_EPOCH + Duration::from_nanos(nanos)
}

/// Removes ended spools older than `output.spool_ttl` and spools that never
/// ended older than [`ABANDONED_SPOOL_AGE`]. Spools of this process that are
/// still in use stay; a spool that cannot be removed is left for the next
/// run.
fn remove_expired(ctx: &ExecCtx, dir: &Path, live: &HashMap<PathBuf, Weak<Spool>>) {
    let ttl = ctx.config().output.spool_ttl.as_std();
    let now = wall_clock(ctx);
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for path in entries.flatten().map(|entry| entry.path()) {
        let Some(id) = path
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.strip_suffix(".json"))
        else {
            continue;
        };
        if live.contains_key(&path) || validate_id(id).is_err() {
            continue;
        }
        let paths = SpoolPaths {
            dir: dir.to_path_buf(),
            id: id.to_owned(),
        };
        if is_expired(&paths, now, ttl) {
            let _ = remove_files(&paths);
        }
    }
}

fn is_expired(paths: &SpoolPaths, now: SystemTime, ttl: Duration) -> bool {
    let newest = paths
        .streams()
        .iter()
        .chain([&paths.meta()])
        .filter_map(|path| fs::metadata(path).and_then(|meta| meta.modified()).ok())
        .max();
    let Some(newest) = newest else {
        return false;
    };
    let age = now.duration_since(newest).unwrap_or_default();
    if age <= ttl {
        // No spool is removed before `ttl`; the record of a recent spool is
        // not read.
        return false;
    }
    let complete = read_meta(&paths.meta()).is_ok_and(|meta| meta.is_complete());
    let limit = if complete {
        ttl
    } else {
        ttl.max(ABANDONED_SPOOL_AGE)
    };
    age > limit
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
