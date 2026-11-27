// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-files::walk::service
// Responsibility:   Items passed from the walking thread to the iterator before the walking.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

use std::fmt;
use std::ops::ControlFlow;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};
use std::thread;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::{AbsPath, FileKind, Follow, Fs, FsError, FsErrorKind};
use nurt_contracts::services::{FileListService, WalkEntry, WalkIter, WalkKind, WalkSpec};

use super::{KindFilter, WalkItem, WalkOptions, Walker};

/// Items passed from the walking thread to the iterator before the walking
/// thread waits for the consumer.
const CHANNEL_CAPACITY: usize = 1024;

/// File listing service of the file commands, backed by [`Walker`].
///
/// [`FileListService::walk`] checks the root on the calling thread and then
/// walks on a background thread; the returned iterator receives the entries
/// in walk order. Only files and directories are listed, links counted by
/// their resolved target when `follow_links` is set. Dropping the iterator
/// stops the walk. Every entry below the
/// root that cannot be read, including one refused for lack of permission,
/// is an error item of the iterator, and the listing continues after it.
///
/// # Examples
///
/// ```no_run
/// use std::sync::Arc;
///
/// use nurt_contracts::fs::Fs;
/// use nurt_contracts::services::{FileListService, WalkSpec};
/// use nurt_files::walk::FileList;
///
/// fn count_sources(fs: Arc<dyn Fs>) -> nurt_contracts::error::Result<usize> {
///     let service = FileList::new(fs);
///     let mut count = 0;
///     for entry in service.walk(&WalkSpec::source_tree("/srv/app"))? {
///         entry?;
///         count += 1;
///     }
///     Ok(count)
/// }
/// ```
#[derive(Clone)]
pub struct FileList {
    fs: Arc<dyn Fs>,
    threads: Option<usize>,
    global_ignore_file: Option<AbsPath>,
}

impl FileList {
    /// Service reading through `fs` with the default number of threads and
    /// without a global exclude file.
    #[must_use]
    pub fn new(fs: Arc<dyn Fs>) -> Self {
        Self {
            fs,
            threads: None,
            global_ignore_file: None,
        }
    }

    /// Sets the number of threads reading directories.
    #[must_use]
    pub fn with_threads(mut self, threads: usize) -> Self {
        self.threads = Some(threads);
        self
    }

    /// Sets the global exclude file of git, applied to requests with
    /// `gitignore` set.
    #[must_use]
    pub fn with_global_ignore_file(mut self, path: AbsPath) -> Self {
        self.global_ignore_file = Some(path);
        self
    }

    fn options(&self, spec: &WalkSpec) -> WalkOptions {
        let defaults = WalkOptions::default();
        WalkOptions {
            min_depth: 1,
            max_depth: spec.max_depth,
            kind: match spec.kind {
                WalkKind::File => KindFilter::File,
                WalkKind::Directory => KindFilter::Directory,
                _ => KindFilter::Any,
            },
            hidden: spec.hidden,
            follow_links: spec.follow_links,
            gitignore: spec.gitignore,
            global_ignore_file: self.global_ignore_file.clone(),
            exclude: spec.exclude.clone(),
            exclude_presets: spec.exclude_presets.clone(),
            globs: spec.globs.clone(),
            case_sensitive: spec.case_sensitive,
            metadata: false,
            threads: self.threads.unwrap_or(defaults.threads),
        }
    }
}

impl fmt::Debug for FileList {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FileList")
            .field("threads", &self.threads)
            .field("global_ignore_file", &self.global_ignore_file)
            .finish_non_exhaustive()
    }
}

impl FileListService for FileList {
    /// Lists entries under `spec.root`; depth 1 denotes the direct children.
    ///
    /// # Errors
    ///
    /// Fails with `E_PATH_RELATIVE` for a relative root, with
    /// `E_INVALID_VALUE` for an invalid glob or exclusion pattern, with
    /// `E_NOT_FOUND`, `E_PERMISSION` or `E_IO` when the root is missing, is
    /// not a directory or cannot be read, and with `E_INTERNAL` when the
    /// walking thread cannot be started.
    fn walk<'a>(&'a self, spec: &WalkSpec) -> Result<WalkIter<'a>> {
        if !spec.root.is_absolute() {
            return Err(Error::new(
                ErrorCode::PathRelative,
                format!(
                    "walk root `{}` is relative; pass an absolute path",
                    spec.root.display()
                ),
            )
            .with_found(spec.root.to_string_lossy()));
        }
        let root = AbsPath::new(spec.root.clone())?;
        let meta = self.fs.metadata(&root, Follow::Follow)?;
        if meta.kind != FileKind::Directory {
            return Err(
                FsError::new(FsErrorKind::NotADirectory, "walk", Some(root.as_path())).into(),
            );
        }
        drop(self.fs.read_dir(&root)?);
        let walker = Walker::new(self.options(spec))?;
        let (sender, receiver) = mpsc::sync_channel(CHANNEL_CAPACITY);
        let fs = Arc::clone(&self.fs);
        thread::Builder::new()
            .name("nurt-file-list".to_owned())
            .spawn(move || {
                let delivered = walker.run(fs.as_ref(), &[root], &|| Ok(()), |item| {
                    let message = match item {
                        WalkItem::Entry(entry)
                            if !matches!(
                                entry.effective_kind(),
                                FileKind::File | FileKind::Directory
                            ) =>
                        {
                            return ControlFlow::Continue(());
                        }
                        WalkItem::Entry(entry) => Ok(WalkEntry {
                            is_dir: entry.is_dir(),
                            path: entry.path.as_path().to_path_buf(),
                            depth: entry.depth,
                        }),
                        WalkItem::Unreadable(unreadable) => Err(Error::from(unreadable.error)),
                    };
                    match sender.send(message) {
                        Ok(()) => ControlFlow::Continue(()),
                        Err(_) => ControlFlow::Break(()),
                    }
                });
                if let Err(error) = delivered {
                    // A send fails only after the iterator was dropped, when
                    // nobody is left to receive the error.
                    let _ = sender.send(Err(error));
                }
            })
            .map_err(|_| Error::new(ErrorCode::Internal, "cannot start a thread to list files"))?;
        Ok(Box::new(Entries { receiver }))
    }
}

/// Iterator over the items sent by the walking thread.
struct Entries {
    receiver: Receiver<Result<WalkEntry>>,
}

impl Iterator for Entries {
    type Item = Result<WalkEntry>;

    fn next(&mut self) -> Option<Self::Item> {
        self.receiver.recv().ok()
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
