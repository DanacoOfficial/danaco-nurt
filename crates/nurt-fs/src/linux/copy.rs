// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Block clones (`FICLONE`) and byte copies (`copy_file_range`).
//!
//! The destination is always a new file created with `O_EXCL`, with the
//! permission bits of the source. A clone or copy that fails removes the
//! destination it created, so no partial file remains.
//!
//! Copies keep holes: data ranges of the source are found with
//! `SEEK_DATA`/`SEEK_HOLE` and only they are transferred, then the
//! destination is extended to the source length. Within a range,
//! `copy_file_range` is used; when the kernel refuses it for the pair of
//! files (`EXDEV`, `ENOSYS`, `EINVAL`, `EOPNOTSUPP`), the copy continues with
//! `pread` and `pwrite`.

use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::path::Path;

use nurt_contracts::fs::{FileKind, FsError, FsErrorKind, FsResult};
use rustix::fs::{AtFlags, CWD, Mode, OFlags, SeekFrom};
use rustix::io::Errno;

use super::meta::{PERMISSION_BITS, stat_fd};
use super::sys::{kind_error, retry, sys_error};

/// Largest length passed to one `copy_file_range` call.
const COPY_CHUNK: usize = 1 << 30;
/// Buffer of the `pread`/`pwrite` fallback.
const FALLBACK_BUFFER: usize = 1 << 20;
/// Permission bits applied to a destination before the umask.
const ACCESS_BITS: u32 = 0o777;

/// Opened source and created destination of a clone or copy.
struct Pair {
    source: OwnedFd,
    destination: OwnedFd,
    size: u64,
}

fn open_pair(src: &Path, dst: &Path, op: &str) -> FsResult<Pair> {
    let source = retry(|| {
        rustix::fs::openat(
            CWD,
            src,
            OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOCTTY,
            Mode::empty(),
        )
    })
    .map_err(|errno| sys_error(errno, op, src))?;
    let meta = stat_fd(source.as_fd(), op, src)?;
    if meta.kind != FileKind::File {
        let kind = if meta.kind == FileKind::Directory {
            FsErrorKind::IsADirectory
        } else {
            FsErrorKind::Unsupported
        };
        return Err(FsError::new(kind, op, Some(src)));
    }
    let mode = Mode::from_raw_mode(meta.mode & ACCESS_BITS & PERMISSION_BITS);
    let destination = retry(|| {
        rustix::fs::openat(
            CWD,
            dst,
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::CLOEXEC,
            mode,
        )
    })
    .map_err(|errno| sys_error(errno, op, dst))?;
    Ok(Pair {
        source,
        destination,
        size: meta.size,
    })
}

/// Removes a destination created by this module after a failure; the
/// original error is reported, not a failure of the removal.
fn discard(dst: &Path) {
    let _ = retry(|| rustix::fs::unlinkat(CWD, dst, AtFlags::empty()));
}

pub(super) fn clone_file(src: &Path, dst: &Path) -> FsResult<()> {
    const OP: &str = "clone_file";
    let pair = open_pair(src, dst, OP)?;
    match retry(|| rustix::fs::ioctl_ficlone(pair.destination.as_fd(), pair.source.as_fd())) {
        Ok(()) => Ok(()),
        Err(errno) => {
            drop(pair);
            discard(dst);
            Err(match errno {
                Errno::OPNOTSUPP | Errno::XDEV | Errno::INVAL | Errno::NOTTY | Errno::NOSYS => {
                    kind_error(FsErrorKind::Unsupported, errno, OP, dst)
                }
                errno => sys_error(errno, OP, dst),
            })
        }
    }
}

pub(super) fn copy_file(src: &Path, dst: &Path) -> FsResult<u64> {
    const OP: &str = "copy_file";
    let pair = open_pair(src, dst, OP)?;
    let mut copier = Copier {
        source: pair.source.as_fd(),
        destination: pair.destination.as_fd(),
        in_kernel: true,
    };
    let result = copier
        .copy_data(pair.size)
        .and_then(|()| retry(|| rustix::fs::ftruncate(pair.destination.as_fd(), pair.size)));
    match result {
        Ok(()) => Ok(pair.size),
        Err(errno) => {
            drop(pair);
            discard(dst);
            Err(sys_error(errno, OP, dst))
        }
    }
}

struct Copier<'a> {
    source: BorrowedFd<'a>,
    destination: BorrowedFd<'a>,
    in_kernel: bool,
}

impl Copier<'_> {
    /// Copies every data range of the first `size` bytes of the source.
    fn copy_data(&mut self, size: u64) -> rustix::io::Result<()> {
        let mut offset = 0;
        let mut sparse = true;
        while offset < size {
            let (start, end) = if sparse {
                match self.next_data(offset, size) {
                    Ok(Some(range)) => range,
                    Ok(None) => return Ok(()),
                    Err(Errno::INVAL | Errno::OPNOTSUPP) => {
                        sparse = false;
                        (offset, size)
                    }
                    Err(errno) => return Err(errno),
                }
            } else {
                (offset, size)
            };
            if self.copy_range(start, end)? {
                return Ok(());
            }
            offset = end;
        }
        Ok(())
    }

    /// Next data range at or after `offset`, ending at the next hole or at
    /// `size`; `None` when only a hole remains.
    fn next_data(&self, offset: u64, size: u64) -> rustix::io::Result<Option<(u64, u64)>> {
        let start = match retry(|| rustix::fs::seek(self.source, SeekFrom::Data(offset))) {
            Ok(start) => start,
            Err(Errno::NXIO) => return Ok(None),
            Err(errno) => return Err(errno),
        };
        if start >= size {
            return Ok(None);
        }
        let end = retry(|| rustix::fs::seek(self.source, SeekFrom::Hole(start)))?;
        Ok(Some((start, end.min(size))))
    }

    /// Copies `start..end`; returns `true` when the source ended early (it
    /// shrank during the copy).
    fn copy_range(&mut self, start: u64, end: u64) -> rustix::io::Result<bool> {
        let mut position = start;
        while position < end {
            let remaining = usize::try_from(end - position)
                .unwrap_or(usize::MAX)
                .min(COPY_CHUNK);
            let copied = if self.in_kernel {
                let mut from = position;
                let mut to = position;
                match retry(|| {
                    rustix::fs::copy_file_range(
                        self.source,
                        Some(&mut from),
                        self.destination,
                        Some(&mut to),
                        remaining,
                    )
                }) {
                    Ok(copied) => copied,
                    Err(Errno::XDEV | Errno::NOSYS | Errno::INVAL | Errno::OPNOTSUPP) => {
                        self.in_kernel = false;
                        continue;
                    }
                    Err(errno) => return Err(errno),
                }
            } else {
                self.copy_buffered(position, remaining)?
            };
            if copied == 0 {
                return Ok(true);
            }
            position += copied as u64;
        }
        Ok(false)
    }

    fn copy_buffered(&self, position: u64, length: usize) -> rustix::io::Result<usize> {
        let mut buffer = vec![0; length.min(FALLBACK_BUFFER)];
        let read = retry(|| rustix::io::pread(self.source, &mut buffer[..], position))?;
        let mut written = 0;
        while written < read {
            let offset = position + written as u64;
            written +=
                retry(|| rustix::io::pwrite(self.destination, &buffer[written..read], offset))?;
        }
        Ok(read)
    }
}

#[cfg(test)]
mod tests {
    use std::fs::{self, File};
    use std::io::{Seek, SeekFrom, Write};

    use super::*;

    /// Source of 3 MiB + 5 bytes with a hole in the middle; larger than the
    /// fallback buffer so that the buffered copy loops.
    fn sparse_source(dir: &Path) -> (std::path::PathBuf, Vec<u8>) {
        let path = dir.join("source");
        let mut file = File::create(&path).expect("source");
        let length = 3 * FALLBACK_BUFFER as u64 + 5;
        file.set_len(length).expect("length");
        file.write_all(b"head").expect("head");
        file.seek(SeekFrom::End(-4)).expect("seek");
        file.write_all(b"tail").expect("tail");
        drop(file);
        let content = fs::read(&path).expect("content");
        (path, content)
    }

    #[test]
    fn buffered_fallback_copies_every_range() {
        let dir = tempfile::tempdir().expect("directory");
        let (source, content) = sparse_source(dir.path());
        let target = dir.path().join("target");
        let pair = open_pair(&source, &target, "copy_file").expect("pair");
        let mut copier = Copier {
            source: pair.source.as_fd(),
            destination: pair.destination.as_fd(),
            in_kernel: false,
        };
        copier.copy_data(pair.size).expect("copy");
        rustix::fs::ftruncate(pair.destination.as_fd(), pair.size).expect("length");
        drop(pair);
        assert_eq!(fs::read(&target).expect("target"), content);
    }

    #[test]
    fn copy_without_hole_detection_copies_whole_length() {
        let dir = tempfile::tempdir().expect("directory");
        let (source, content) = sparse_source(dir.path());
        let target = dir.path().join("target");
        let pair = open_pair(&source, &target, "copy_file").expect("pair");
        let mut copier = Copier {
            source: pair.source.as_fd(),
            destination: pair.destination.as_fd(),
            in_kernel: true,
        };
        assert_eq!(copier.copy_range(0, pair.size), Ok(false));
        assert_eq!(copier.copy_range(pair.size, pair.size + 10), Ok(true));
        drop(pair);
        assert_eq!(fs::read(&target).expect("target"), content);
    }

    #[test]
    fn copies_file_consisting_of_one_hole() {
        let dir = tempfile::tempdir().expect("directory");
        let source = dir.path().join("hole");
        let length = 3 * FALLBACK_BUFFER;
        File::create(&source)
            .expect("source")
            .set_len(length as u64)
            .expect("length");
        let target = dir.path().join("target");
        let copied = copy_file(&source, &target).expect("copy");
        assert_eq!(copied, length as u64);
        assert_eq!(fs::read(&target).expect("target"), vec![0; length]);
    }

    #[test]
    fn failed_copy_reports_missing_source_and_leaves_no_destination() {
        let dir = tempfile::tempdir().expect("directory");
        let target = dir.path().join("target");
        let error = copy_file(&dir.path().join("missing"), &target).expect_err("missing source");
        assert_eq!(error.kind, FsErrorKind::NotFound);
        assert!(!target.exists());
        let source = dir.path().join("source");
        fs::write(&source, b"x").expect("source");
        let error = copy_file(&source, &dir.path().join("missing").join("target"))
            .expect_err("missing directory");
        assert_eq!(error.kind, FsErrorKind::NotFound);
    }
}
