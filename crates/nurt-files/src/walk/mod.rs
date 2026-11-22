// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Parallel directory walker over the read-only file system interface.
//!
//! A [`Walker`] lists one or more roots depth first. Directories are read by
//! worker threads, while entries reach the caller on the calling thread in a
//! deterministic order: roots in the order given, every directory followed
//! by its children sorted by the bytes of their names. The order does not
//! depend on the number of threads or on the order in which the file system
//! returns names.
//!
//! The root itself is never excluded. Every other entry passes the following
//! rules in turn; an entry excluded by one of them is not delivered and, if
//! it is a directory, is not entered, so a `!pattern` cannot re-include an
//! entry below an excluded directory:
//!
//! 1. `hidden: false` excludes names starting with `.` and, on Windows,
//!    entries with the hidden attribute;
//! 2. `exclude_presets` exclude directories by name ([`preset_names`]);
//! 3. `exclude` patterns in gitignore syntax, matched against the path
//!    relative to the root, where the last matching pattern decides and
//!    `!pattern` re-includes;
//! 4. with `gitignore: true`, the ignore files of the tree (see below).
//!
//! The kind filter, the depth range and the `globs` select which of the
//! remaining entries are delivered; they never prevent descending into a
//! directory.
//!
//! A directory below a root that cannot be read, and an entry whose metadata
//! cannot be read, is reported as [`WalkItem::Unreadable`] and the walk
//! continues. [`WalkSummary`] counts such entries; entries refused for lack
//! of permission are counted separately, with a sample of their paths.

mod filter;
mod rules;
mod schedule;
mod service;

use std::ffi::OsString;
use std::ops::ControlFlow;
use std::path::{Component, PathBuf};

use globset::GlobSet;
use ignore::gitignore::Gitignore;
use nurt_contracts::error::Result;
use nurt_contracts::fs::{AbsPath, FileKind, FileMeta, Fs, FsError};
use nurt_contracts::services::ExcludePreset;

pub use filter::{BUILD_DIRS, CACHE_DIRS, DEPS_DIRS, VCS_DIRS, preset_names};
pub use service::FileList;

/// Highest number of worker threads used when [`WalkOptions::threads`] is
/// left at its default.
pub const DEFAULT_MAX_THREADS: usize = 8;

/// Number of paths kept in each sample of [`WalkSummary`].
pub const SAMPLE_LIMIT: usize = 5;

/// Kind of entries delivered by a [`Walker`], as in the `type` parameter of
/// `list` and `find`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KindFilter {
    /// Regular files; with `follow_links`, also links to files.
    File,
    /// Directories; with `follow_links`, also links to directories.
    Directory,
    /// Symbolic links and junctions; with `follow_links`, only links whose
    /// target cannot be resolved.
    Link,
    /// Every entry, including devices, sockets and pipes.
    Any,
}

/// Settings of a [`Walker`].
///
/// The default value has the defaults of `find`: files only, unlimited depth,
/// hidden entries included, links not followed, no ignore files, no
/// exclusions and all globs matched case-sensitively.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalkOptions {
    /// Shallowest depth delivered; 0 delivers the roots themselves.
    pub min_depth: u32,
    /// Deepest depth delivered and entered; `None` is unlimited.
    pub max_depth: Option<u32>,
    /// Kind of entries delivered.
    pub kind: KindFilter,
    /// Deliver and enter hidden entries.
    pub hidden: bool,
    /// Enter symbolic links and junctions that resolve to directories.
    /// Links that lead back to a directory being walked are delivered but
    /// not entered; [`WalkSummary::link_loops`] counts them.
    pub follow_links: bool,
    /// Apply ignore files, read through the [`Fs`] interface.
    ///
    /// The walker reads `.ignore` files in every directory, and `.gitignore`
    /// files, `.git/info/exclude` and the global exclude file in directories
    /// of a git repository (a directory containing `.git`, and its
    /// descendants). Files in the ancestors of a root are read as well. Rules
    /// of a deeper directory take precedence over rules of a shallower one;
    /// `.ignore` takes precedence over `.gitignore`, which takes precedence
    /// over `.git/info/exclude`, which takes precedence over the global
    /// file. A nested repository does not inherit the `.gitignore` files of
    /// the enclosing one.
    pub gitignore: bool,
    /// Global exclude file of git (`core.excludesFile`); used only with
    /// `gitignore`.
    pub global_ignore_file: Option<AbsPath>,
    /// Exclusion patterns in gitignore syntax relative to the root.
    pub exclude: Vec<String>,
    /// Named groups of excluded directories.
    pub exclude_presets: Vec<ExcludePreset>,
    /// Glob patterns matched against the path relative to the root with `/`
    /// separators; an entry matching any of them is delivered. An empty list
    /// matches every entry. A root is matched by its own name.
    pub globs: Vec<String>,
    /// Match `globs` and `exclude` case-sensitively.
    pub case_sensitive: bool,
    /// Read the metadata of every delivered entry ([`Entry::meta`]).
    pub metadata: bool,
    /// Worker threads reading directories; 0 is treated as 1.
    pub threads: usize,
}

impl Default for WalkOptions {
    fn default() -> Self {
        let threads = std::thread::available_parallelism()
            .map_or(1, std::num::NonZero::get)
            .min(DEFAULT_MAX_THREADS);
        Self {
            min_depth: 0,
            max_depth: None,
            kind: KindFilter::File,
            hidden: true,
            follow_links: false,
            gitignore: false,
            global_ignore_file: None,
            exclude: Vec::new(),
            exclude_presets: Vec::new(),
            globs: Vec::new(),
            case_sensitive: true,
            metadata: false,
            threads,
        }
    }
}

/// Entry delivered by a [`Walker`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Absolute path.
    pub path: AbsPath,
    /// Path relative to the root; empty for a root.
    pub relative: PathBuf,
    /// Name as stored by the file system; empty for a file system root.
    pub name: OsString,
    /// Levels below the root; 0 for a root.
    pub depth: u32,
    /// Position of the root in the list passed to [`Walker::run`].
    pub root: usize,
    /// Kind of the entry itself, links not followed.
    pub kind: FileKind,
    /// Kind of the link target, when the entry is a link that was resolved
    /// because `follow_links` is set.
    pub target_kind: Option<FileKind>,
    /// Metadata, when [`WalkOptions::metadata`] is set; for a resolved link,
    /// the metadata of the target.
    pub meta: Option<FileMeta>,
}

impl Entry {
    /// Kind used by the kind filter: the target kind of a resolved link,
    /// otherwise the kind of the entry.
    #[must_use]
    pub fn effective_kind(&self) -> FileKind {
        self.target_kind.unwrap_or(self.kind)
    }

    /// Returns `true` for a directory or a resolved link to a directory.
    #[must_use]
    pub fn is_dir(&self) -> bool {
        self.effective_kind() == FileKind::Directory
    }

    /// Relative path in language form: components joined by `/`, names that
    /// are not valid Unicode converted lossily.
    ///
    /// ```
    /// use std::path::PathBuf;
    ///
    /// # use nurt_contracts::fs::{AbsPath, FileKind};
    /// # use nurt_files::walk::Entry;
    /// # let root = std::env::temp_dir();
    /// let entry = Entry {
    ///     path: AbsPath::new(root.join("src").join("lib.rs"))?,
    ///     relative: PathBuf::from("src").join("lib.rs"),
    ///     name: "lib.rs".into(),
    ///     depth: 2,
    ///     root: 0,
    ///     kind: FileKind::File,
    ///     target_kind: None,
    ///     meta: None,
    /// };
    /// assert_eq!(entry.relative_string(), "src/lib.rs");
    /// # Ok::<(), nurt_contracts::fs::FsError>(())
    /// ```
    #[must_use]
    pub fn relative_string(&self) -> String {
        let mut text = String::new();
        for component in self.relative.components() {
            if let Component::Normal(part) = component {
                if !text.is_empty() {
                    text.push('/');
                }
                text.push_str(&part.to_string_lossy());
            }
        }
        text
    }
}

/// Entry that could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unreadable {
    /// Absolute path of the entry.
    pub path: AbsPath,
    /// Levels below the root.
    pub depth: u32,
    /// The failure.
    pub error: FsError,
}

/// Item delivered by [`Walker::run`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WalkItem {
    /// An entry selected by the options.
    Entry(Entry),
    /// A directory below a root, or an entry, that could not be read; for a
    /// directory, its contents are missing from the walk.
    Unreadable(Unreadable),
}

/// Counters of a finished walk.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WalkSummary {
    /// Entries delivered.
    pub entries: u64,
    /// Entries skipped because access was refused.
    pub permission_denied: u64,
    /// Paths of the first entries skipped for lack of permission, in walk
    /// order; up to [`SAMPLE_LIMIT`] paths.
    pub permission_denied_sample: Vec<AbsPath>,
    /// Links not entered because they lead back to a directory being walked.
    pub link_loops: u64,
    /// Paths of the first such links, in walk order; up to [`SAMPLE_LIMIT`]
    /// paths.
    pub link_loop_sample: Vec<AbsPath>,
    /// Entries skipped because of other read errors.
    pub unreadable: u64,
    /// `false` when the sink stopped the walk before its end.
    pub complete: bool,
}

/// Directory walker with compiled patterns.
///
/// A walker is immutable and can run any number of walks, also from several
/// threads at once.
#[derive(Debug, Clone)]
pub struct Walker {
    options: WalkOptions,
    globs: Option<GlobSet>,
    exclude: Option<Gitignore>,
    presets: Vec<&'static str>,
}

impl Walker {
    /// Compiles the patterns of the options.
    ///
    /// # Errors
    ///
    /// Fails with `E_INVALID_VALUE` when a glob or an exclusion pattern is
    /// not valid; `found` holds the pattern.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_contracts::error::ErrorCode;
    /// use nurt_files::walk::{WalkOptions, Walker};
    ///
    /// let options = WalkOptions { globs: vec!["src/[a-".to_owned()], ..WalkOptions::default() };
    /// let error = Walker::new(options).unwrap_err();
    /// assert_eq!(error.code(), ErrorCode::InvalidValue);
    /// ```
    pub fn new(options: WalkOptions) -> Result<Self> {
        let globs = filter::compile_globs(&options.globs, options.case_sensitive)?;
        let exclude = filter::compile_exclude(&options.exclude, options.case_sensitive)?;
        let presets = filter::preset_list(&options.exclude_presets);
        Ok(Self {
            options,
            globs,
            exclude,
            presets,
        })
    }

    /// Settings of the walker.
    #[must_use]
    pub fn options(&self) -> &WalkOptions {
        &self.options
    }

    /// Walks `roots` in order and passes every item to `sink` on the calling
    /// thread. The walk stops when `sink` returns [`ControlFlow::Break`].
    ///
    /// The checkpoint function is called regularly on the calling thread,
    /// also while it waits for a directory; an error returned by it ends the
    /// walk with that error. Pass `&|| cancel.check()` for a
    /// [`CancelToken`](nurt_contracts::stage::CancelToken), or the
    /// checkpoint of the execution context, which also enforces the
    /// deadline.
    ///
    /// # Errors
    ///
    /// Fails with the error of `checkpoint`, and with the code of the file
    /// system error (`E_NOT_FOUND`, `E_PERMISSION`, `E_IO`) when a root does
    /// not exist or a root directory cannot be read. Failures below a root
    /// are delivered as [`WalkItem::Unreadable`] instead. Fails with
    /// `E_INTERNAL` when no worker thread can be started.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::ops::ControlFlow;
    ///
    /// use nurt_contracts::error::Result;
    /// use nurt_contracts::fs::{AbsPath, Fs};
    /// use nurt_contracts::stage::CancelToken;
    /// use nurt_files::walk::{KindFilter, WalkItem, WalkOptions, Walker};
    ///
    /// fn rust_files(fs: &dyn Fs, root: &AbsPath, cancel: &CancelToken) -> Result<Vec<AbsPath>> {
    ///     let walker = Walker::new(WalkOptions {
    ///         globs: vec!["**/*.rs".to_owned()],
    ///         kind: KindFilter::File,
    ///         ..WalkOptions::default()
    ///     })?;
    ///     let mut found = Vec::new();
    ///     walker.run(fs, std::slice::from_ref(root), &|| cancel.check(), |item| {
    ///         if let WalkItem::Entry(entry) = item {
    ///             found.push(entry.path);
    ///         }
    ///         ControlFlow::Continue(())
    ///     })?;
    ///     Ok(found)
    /// }
    /// ```
    pub fn run<F>(
        &self,
        fs: &dyn Fs,
        roots: &[AbsPath],
        checkpoint: &dyn Fn() -> Result<()>,
        sink: F,
    ) -> Result<WalkSummary>
    where
        F: FnMut(WalkItem) -> ControlFlow<()>,
    {
        schedule::run(self, fs, roots, checkpoint, sink)
    }
}
