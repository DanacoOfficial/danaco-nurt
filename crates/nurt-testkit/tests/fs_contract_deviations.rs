// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The conformance checks against a file system that deviates from the
//! contract at one chosen call.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests report failures by panicking"
)]

use std::io::{self, Cursor, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use nurt_contracts::fs::{
    AbsPath, ContentHash, DirEntry, Durability, Etag, ExtendedMeta, FileKind, FileLock, FileMeta,
    Follow, Fs, FsError, FsErrorKind, FsResult, FsWrite, Holder, LinkKind, MetaChange,
    MetaTransferReport, PathKey, ReadHandle, RemoveStats, RenameMode, SpaceInfo, TxScope, VolumeId,
    VolumeInfo, WriteHandle,
};
use nurt_testkit::clock::VirtualClock;
use nurt_testkit::fs_contract::{CHECKS, CheckOutcome, FsContract};
use nurt_testkit::vfs::{VirtualFs, test_path};

type Dir<'a> = Box<dyn Iterator<Item = FsResult<DirEntry>> + Send + 'a>;

/// Wraps a virtual file system and deviates at the call with the number
/// `target`, counted over all calls, in the manner selected by `variant`.
struct Tamper {
    inner: VirtualFs,
    counter: AtomicUsize,
    target: usize,
    variant: u8,
}

fn io_error(op: &str) -> FsError {
    FsError::new(FsErrorKind::Io, op, None)
}

fn denied(op: &str) -> FsError {
    FsError::new(FsErrorKind::PermissionDenied, op, None)
}

fn tweak(variant: u8, mut meta: FileMeta) -> FileMeta {
    match variant {
        1 => meta.size += 1,
        2 => meta.links += 1,
        3 => {
            meta.kind = if meta.kind == FileKind::File {
                FileKind::Directory
            } else {
                FileKind::File
            }
        }
        4 => meta.modified_ns += 1,
        _ => meta.modified_ns = 0,
    }
    meta
}

struct Reader {
    data: Cursor<Vec<u8>>,
    meta: FileMeta,
}

impl Read for Reader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.data.read(buf)
    }
}

impl Seek for Reader {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.data.seek(position)
    }
}

impl ReadHandle for Reader {
    fn meta(&self) -> FsResult<FileMeta> {
        Ok(self.meta.clone())
    }
}

struct Writer {
    inner: Box<dyn WriteHandle>,
    variant: u8,
}

impl Write for Writer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.variant == 1 {
            Ok(buf.len())
        } else {
            self.inner.write(buf)
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

impl Seek for Writer {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.inner.seek(position)
    }
}

impl WriteHandle for Writer {
    fn path(&self) -> &AbsPath {
        self.inner.path()
    }

    fn sync(&mut self, durability: Durability) -> FsResult<()> {
        if self.variant == 2 {
            Err(io_error("sync"))
        } else {
            self.inner.sync(durability)
        }
    }

    fn set_length(&mut self, length: u64) -> FsResult<()> {
        if self.variant == 3 {
            Ok(())
        } else {
            self.inner.set_length(length)
        }
    }

    fn apply_meta(&mut self, meta: &ExtendedMeta) -> FsResult<MetaTransferReport> {
        self.inner.apply_meta(meta)
    }
}

struct Lock {
    path: AbsPath,
}

impl FileLock for Lock {
    fn path(&self) -> &AbsPath {
        &self.path
    }
}

impl Tamper {
    fn new(inner: VirtualFs, target: usize, variant: u8) -> Self {
        Self {
            inner,
            counter: AtomicUsize::new(0),
            target,
            variant,
        }
    }

    fn gate(&self) -> Option<u8> {
        let call = self.counter.fetch_add(1, Ordering::SeqCst);
        (call == self.target).then_some(self.variant)
    }

    fn mutate<T>(&self, op: &str, skipped: T, real: impl FnOnce() -> FsResult<T>) -> FsResult<T> {
        match self.gate() {
            Some(0) => Err(io_error(op)),
            Some(1) => Ok(skipped),
            Some(2) => Err(denied(op)),
            _ => real(),
        }
    }

    fn handle(&self, variant: Option<u8>, handle: Box<dyn WriteHandle>) -> Box<dyn WriteHandle> {
        match variant {
            Some(variant) => Box::new(Writer {
                inner: handle,
                variant,
            }),
            None => handle,
        }
    }
}

impl Fs for Tamper {
    fn metadata(&self, path: &AbsPath, follow: Follow) -> FsResult<FileMeta> {
        match self.gate() {
            Some(0) => Err(io_error("metadata")),
            Some(variant) => self.inner.metadata(path, follow).map(|m| tweak(variant, m)),
            None => self.inner.metadata(path, follow),
        }
    }

    fn extended_metadata(&self, path: &AbsPath, follow: Follow) -> FsResult<ExtendedMeta> {
        let mut meta = self.inner.extended_metadata(path, follow)?;
        match self.gate() {
            Some(0) => return Err(io_error("extended_metadata")),
            Some(1) => meta.link_target = None,
            Some(_) => meta.link_target = Some(PathBuf::from("/elsewhere")),
            None => {}
        }
        Ok(meta)
    }

    fn open_read(&self, path: &AbsPath) -> FsResult<Box<dyn ReadHandle>> {
        let gate = self.gate();
        let handle = self.inner.open_read(path)?;
        let Some(variant) = gate else {
            return Ok(handle);
        };
        let mut data = Vec::new();
        let mut source = handle;
        source
            .read_to_end(&mut data)
            .map_err(|_| io_error("read"))?;
        let mut meta = source.meta()?;
        match variant {
            0 => return Err(io_error("open_read")),
            1 => data.push(b'!'),
            _ => meta.size += 1,
        }
        Ok(Box::new(Reader {
            data: Cursor::new(data),
            meta,
        }))
    }

    fn open_stream_read(&self, path: &AbsPath, stream: &str) -> FsResult<Box<dyn ReadHandle>> {
        self.inner.open_stream_read(path, stream)
    }

    fn read_dir<'a>(&'a self, path: &AbsPath) -> FsResult<Dir<'a>> {
        let gate = self.gate();
        let entries = self.inner.read_dir(path)?;
        match gate {
            Some(0) => Err(io_error("read_dir")),
            Some(1) => Ok(Box::new(entries.chain(std::iter::once(Ok(DirEntry {
                name: "zz".into(),
                kind: None,
            }))))),
            Some(2) => Ok(Box::new(
                entries.chain(std::iter::once(Err(io_error("entry")))),
            )),
            Some(_) => Ok(Box::new(std::iter::empty())),
            None => Ok(entries),
        }
    }

    fn read_link(&self, path: &AbsPath) -> FsResult<PathBuf> {
        match self.gate() {
            Some(0) => Err(io_error("read_link")),
            Some(_) => Ok(PathBuf::from("/elsewhere")),
            None => self.inner.read_link(path),
        }
    }

    fn real_path(&self, path: &AbsPath) -> FsResult<AbsPath> {
        match self.gate() {
            Some(0) => Err(io_error("real_path")),
            Some(_) => test_path("/elsewhere"),
            None => self.inner.real_path(path),
        }
    }

    fn content_hash(&self, path: &AbsPath) -> FsResult<ContentHash> {
        match self.gate() {
            Some(0) => Err(io_error("content_hash")),
            Some(_) => Ok(ContentHash([0; 32])),
            None => self.inner.content_hash(path),
        }
    }

    fn volume(&self, path: &AbsPath) -> FsResult<VolumeInfo> {
        let mut info = self.inner.volume(path)?;
        match self.gate() {
            Some(0) => return Err(io_error("volume")),
            Some(1) => info.features.hardlink = !info.features.hardlink,
            Some(_) => info.features.rename_noreplace = !info.features.rename_noreplace,
            None => {}
        }
        Ok(info)
    }

    fn space(&self, volume: &VolumeId) -> FsResult<SpaceInfo> {
        let mut space = self.inner.space(volume)?;
        match self.gate() {
            Some(0) => return Err(io_error("space")),
            Some(_) => space.available = space.total + 1,
            None => {}
        }
        Ok(space)
    }

    fn path_key(&self, path: &AbsPath) -> FsResult<PathKey> {
        match self.gate() {
            Some(0) => Err(io_error("path_key")),
            Some(1) => Ok(PathKey(vec![1])),
            Some(_) => Ok(PathKey(vec![
                u8::try_from(self.counter.load(Ordering::SeqCst)).unwrap_or(0),
            ])),
            None => self.inner.path_key(path),
        }
    }

    fn holders(&self, path: &AbsPath) -> FsResult<Vec<Holder>> {
        match self.gate() {
            Some(0) => Err(io_error("holders")),
            Some(1) => Ok(Vec::new()),
            Some(_) => Err(FsError::new(FsErrorKind::NotFound, "holders", None)),
            None => self.inner.holders(path),
        }
    }

    fn can_write_dir(&self, dir: &AbsPath) -> FsResult<bool> {
        match self.gate() {
            Some(0) => Err(io_error("can_write_dir")),
            Some(_) => Ok(false),
            None => self.inner.can_write_dir(dir),
        }
    }

    fn etag(&self, path: &AbsPath, follow: Follow) -> FsResult<Etag> {
        match self.gate() {
            Some(0) => Err(io_error("etag")),
            Some(variant) => Ok(tweak(variant, self.inner.metadata(path, follow)?).etag()),
            None => self.inner.etag(path, follow),
        }
    }

    fn same_volume(&self, a: &AbsPath, b: &AbsPath) -> FsResult<bool> {
        match self.gate() {
            Some(0) => Err(io_error("same_volume")),
            Some(_) => Ok(false),
            None => self.inner.same_volume(a, b),
        }
    }
}

impl FsWrite for Tamper {
    fn create_temp_in(
        &self,
        scope: &TxScope,
        dir: &AbsPath,
        name_hint: &str,
    ) -> FsResult<Box<dyn WriteHandle>> {
        let gate = self.gate();
        if gate == Some(0) {
            return Err(io_error("create_temp_in"));
        }
        let handle = self.inner.create_temp_in(scope, dir, name_hint)?;
        Ok(self.handle(gate.map(|variant| variant - 1), handle))
    }

    fn create_new(&self, scope: &TxScope, path: &AbsPath) -> FsResult<Box<dyn WriteHandle>> {
        match self.gate() {
            Some(0) => Err(io_error("create_new")),
            Some(1) => self
                .inner
                .create_new(scope, path)
                .or_else(|_| self.inner.open_in_place(scope, path)),
            Some(2) => Err(denied("create_new")),
            Some(variant) => {
                let handle = self.inner.create_new(scope, path)?;
                Ok(self.handle(Some(variant - 2), handle))
            }
            None => self.inner.create_new(scope, path),
        }
    }

    fn open_in_place(&self, scope: &TxScope, path: &AbsPath) -> FsResult<Box<dyn WriteHandle>> {
        let gate = self.gate();
        if gate == Some(0) {
            return Err(io_error("open_in_place"));
        }
        let handle = self.inner.open_in_place(scope, path)?;
        Ok(self.handle(gate.map(|variant| variant - 1), handle))
    }

    fn open_stream_write(
        &self,
        scope: &TxScope,
        path: &AbsPath,
        stream: &str,
    ) -> FsResult<Box<dyn WriteHandle>> {
        self.inner.open_stream_write(scope, path, stream)
    }

    fn rename(
        &self,
        scope: &TxScope,
        from: &AbsPath,
        to: &AbsPath,
        mode: RenameMode,
    ) -> FsResult<()> {
        self.mutate("rename", (), || self.inner.rename(scope, from, to, mode))
    }

    fn exchange(&self, scope: &TxScope, a: &AbsPath, b: &AbsPath) -> FsResult<()> {
        self.inner.exchange(scope, a, b)
    }

    fn replace_with_backup(
        &self,
        scope: &TxScope,
        target: &AbsPath,
        replacement: &AbsPath,
        backup: &AbsPath,
    ) -> FsResult<()> {
        self.inner
            .replace_with_backup(scope, target, replacement, backup)
    }

    fn hard_link(&self, scope: &TxScope, existing: &AbsPath, new: &AbsPath) -> FsResult<()> {
        self.mutate("hard_link", (), || {
            self.inner.hard_link(scope, existing, new)
        })
    }

    fn clone_file(&self, scope: &TxScope, src: &AbsPath, dst: &AbsPath) -> FsResult<()> {
        self.inner.clone_file(scope, src, dst)
    }

    fn copy_file(&self, scope: &TxScope, src: &AbsPath, dst: &AbsPath) -> FsResult<u64> {
        self.mutate("copy_file", 0, || self.inner.copy_file(scope, src, dst))
    }

    fn remove_file(&self, scope: &TxScope, path: &AbsPath) -> FsResult<()> {
        self.mutate("remove_file", (), || self.inner.remove_file(scope, path))
    }

    fn remove_dir(&self, scope: &TxScope, path: &AbsPath) -> FsResult<()> {
        self.mutate("remove_dir", (), || self.inner.remove_dir(scope, path))
    }

    fn remove_tree(
        &self,
        scope: &TxScope,
        path: &AbsPath,
        parallelism: usize,
    ) -> FsResult<RemoveStats> {
        let skipped = RemoveStats::default();
        self.mutate("remove_tree", skipped, || {
            self.inner.remove_tree(scope, path, parallelism)
        })
    }

    fn create_dir(&self, scope: &TxScope, path: &AbsPath, mode: u32) -> FsResult<()> {
        self.mutate("create_dir", (), || {
            self.inner.create_dir(scope, path, mode)
        })
    }

    fn create_link(
        &self,
        scope: &TxScope,
        target: &Path,
        link: &AbsPath,
        kind: LinkKind,
    ) -> FsResult<()> {
        match self.gate() {
            Some(0) => Err(io_error("create_link")),
            Some(1) => Ok(()),
            Some(_) => Err(FsError::new(FsErrorKind::Privilege, "create_link", None)),
            None => self.inner.create_link(scope, target, link, kind),
        }
    }

    fn set_meta(&self, scope: &TxScope, path: &AbsPath, change: &MetaChange) -> FsResult<()> {
        self.mutate("set_meta", (), || self.inner.set_meta(scope, path, change))
    }

    fn restore_meta(
        &self,
        scope: &TxScope,
        path: &AbsPath,
        meta: &ExtendedMeta,
    ) -> FsResult<MetaTransferReport> {
        self.inner.restore_meta(scope, path, meta)
    }

    fn truncate(&self, scope: &TxScope, path: &AbsPath, length: u64) -> FsResult<()> {
        self.mutate("truncate", (), || self.inner.truncate(scope, path, length))
    }

    fn sync_dir(&self, scope: &TxScope, dir: &AbsPath) -> FsResult<()> {
        self.inner.sync_dir(scope, dir)
    }

    fn lock_exclusive(&self, path: &AbsPath, timeout: Duration) -> FsResult<Box<dyn FileLock>> {
        match self.gate() {
            Some(0) => Err(io_error("lock_exclusive")),
            Some(1) => Ok(Box::new(Lock {
                path: test_path("/elsewhere")?,
            })),
            Some(_) => self.inner.lock_exclusive(path, timeout),
            None => self.inner.lock_exclusive(path, timeout),
        }
    }

    fn mark_hidden(&self, scope: &TxScope, path: &AbsPath) -> FsResult<()> {
        self.inner.mark_hidden(scope, path)
    }
}

fn tampered(target: usize, variant: u8) -> Tamper {
    let inner = VirtualFs::new(Arc::new(VirtualClock::new()));
    inner
        .create_dir_all(&test_path("/contract").unwrap())
        .unwrap();
    Tamper::new(inner, target, variant)
}

const CALL_LIMIT: usize = 130;
const VARIANTS: u8 = 6;

#[test]
fn a_single_deviation_never_goes_unnoticed_by_every_check() {
    let scope = FsContract::scope();
    let mut failed = 0_usize;
    let mut skipped = 0_usize;
    for (name, _) in CHECKS {
        for variant in 0..VARIANTS {
            for target in 0..CALL_LIMIT {
                let fs = tampered(target, variant);
                let contract = FsContract::new(&fs, test_path("/contract").unwrap(), &scope);
                match contract.run_check(name).unwrap() {
                    CheckOutcome::Failed { message } => {
                        assert!(!message.is_empty());
                        failed += 1;
                    }
                    CheckOutcome::Skipped { .. } => skipped += 1,
                    CheckOutcome::Passed => {}
                }
            }
        }
    }
    assert!(failed > 300, "{failed} failed runs");
    assert!(skipped > 0);
}

#[test]
fn a_report_names_the_failed_check_and_the_deviation() {
    let scope = FsContract::scope();
    let fs = tampered(3, 1);
    let report = FsContract::new(&fs, test_path("/contract").unwrap(), &scope).run();
    assert!(!report.is_success());
    let text = report.to_string();
    assert!(text.contains("failed"), "{text}");
    let first = report.failures().next().unwrap();
    assert!(text.contains(first.name), "{text}");
}
