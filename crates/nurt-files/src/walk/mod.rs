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
//! `exclude` and `globs` compare the patterns and the relative paths in
//! Unicode normalization form C, so a name stored decomposed, as on macOS
//! volumes, matches a pattern written composed, and the reverse.
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
pub(crate) use filter::{compile_globs, glob_matches};
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
    /// Exclusion patterns in gitignore syntax relative to the root, matched
    /// in NFC.
    pub exclude: Vec<String>,
    /// Named groups of excluded directories.
    pub exclude_presets: Vec<ExcludePreset>,
    /// Glob patterns matched against the path relative to the root with `/`
    /// separators, in NFC; an entry matching any of them is delivered. An
    /// empty list matches every entry. A root is matched by its own name.
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

#[cfg(test)]
mod tests {
    use nurt_contracts::error::{Error, ErrorCode};
    use nurt_testkit::ctx::TestContext;
    use nurt_testkit::vfs::test_path;

    use super::*;

    fn abs(text: &str) -> AbsPath {
        test_path(text).expect("test paths are absolute")
    }

    fn tree(files: &[&str]) -> TestContext {
        let test = TestContext::new();
        for file in files {
            write(&test, file, "x");
        }
        test
    }

    fn write(test: &TestContext, file: &str, content: &str) {
        test.fs()
            .write_file(&abs(file), content.as_bytes())
            .expect("the virtual file system accepts the file");
    }

    fn walk_in(test: &TestContext, root: &str, options: WalkOptions) -> (Vec<String>, WalkSummary) {
        let walker = Walker::new(options).expect("the patterns compile");
        let mut delivered = Vec::new();
        let summary = walker
            .run(test.fs().as_ref(), &[abs(root)], &|| Ok(()), |item| {
                if let WalkItem::Entry(entry) = item {
                    delivered.push(entry.relative_string());
                }
                ControlFlow::Continue(())
            })
            .expect("the walk succeeds");
        (delivered, summary)
    }

    fn walk(test: &TestContext, options: WalkOptions) -> Vec<String> {
        walk_in(test, "/w", options).0
    }

    fn strings(list: &[&str]) -> Vec<String> {
        list.iter().map(|item| (*item).to_owned()).collect()
    }

    fn any_below_root() -> WalkOptions {
        WalkOptions {
            kind: KindFilter::Any,
            min_depth: 1,
            ..WalkOptions::default()
        }
    }

    #[test]
    fn entries_come_depth_first_in_byte_order_of_names() {
        let test = tree(&["/w/b/d/e.txt", "/w/b/c.txt", "/w/a.txt", "/w/B.txt"]);
        let (delivered, summary) = walk_in(&test, "/w", any_below_root());
        assert_eq!(
            delivered,
            strings(&["B.txt", "a.txt", "b", "b/c.txt", "b/d", "b/d/e.txt"])
        );
        assert_eq!(summary.entries, 6);
        assert!(summary.complete);
        assert_eq!(summary.unreadable, 0);
        assert_eq!(summary.permission_denied, 0);
    }

    #[test]
    fn the_order_does_not_depend_on_the_number_of_threads() {
        let test = tree(&["/w/b/d/e.txt", "/w/b/c.txt", "/w/a.txt", "/w/z/y/x.txt"]);
        let one = walk(
            &test,
            WalkOptions {
                threads: 1,
                ..any_below_root()
            },
        );
        let many = walk(
            &test,
            WalkOptions {
                threads: 8,
                ..any_below_root()
            },
        );
        assert_eq!(one, many);
        assert_eq!(one.len(), 8);
    }

    #[test]
    fn files_only_is_the_default_and_the_root_is_a_candidate_at_depth_zero() {
        let test = tree(&["/w/b/c.txt", "/w/a.txt"]);
        assert_eq!(
            walk(&test, WalkOptions::default()),
            strings(&["a.txt", "b/c.txt"])
        );
        let directories = WalkOptions {
            kind: KindFilter::Directory,
            ..WalkOptions::default()
        };
        assert_eq!(walk(&test, directories), strings(&["", "b"]));
    }

    #[test]
    fn depth_limits_select_levels_and_stop_the_descent() {
        let test = tree(&["/w/b/d/e.txt", "/w/b/c.txt", "/w/a.txt"]);
        let level_two = WalkOptions {
            min_depth: 2,
            max_depth: Some(2),
            ..any_below_root()
        };
        assert_eq!(walk(&test, level_two), strings(&["b/c.txt", "b/d"]));
        let top = WalkOptions {
            max_depth: Some(1),
            ..any_below_root()
        };
        assert_eq!(walk(&test, top), strings(&["a.txt", "b"]));
        let root_only = WalkOptions {
            max_depth: Some(0),
            kind: KindFilter::Any,
            ..WalkOptions::default()
        };
        assert_eq!(walk(&test, root_only), strings(&[""]));
    }

    #[test]
    fn hidden_entries_and_their_content_are_left_out_on_request() {
        let test = tree(&["/w/.env", "/w/.cache/x.txt", "/w/a.txt"]);
        assert_eq!(
            walk(&test, WalkOptions::default()),
            strings(&[".cache/x.txt", ".env", "a.txt"])
        );
        let visible = WalkOptions {
            hidden: false,
            ..WalkOptions::default()
        };
        assert_eq!(walk(&test, visible), strings(&["a.txt"]));
    }

    #[test]
    fn presets_exclude_directories_but_not_files_of_the_same_name() {
        let test = tree(&[
            "/w/target/out.o",
            "/w/src/main.rs",
            "/w/vendor/lib.rs",
            "/w/dir/target",
        ]);
        let build = WalkOptions {
            exclude_presets: vec![ExcludePreset::Build],
            ..WalkOptions::default()
        };
        assert_eq!(
            walk(&test, build),
            strings(&["dir/target", "src/main.rs", "vendor/lib.rs"])
        );
        let both = WalkOptions {
            exclude_presets: vec![ExcludePreset::Build, ExcludePreset::Deps],
            ..WalkOptions::default()
        };
        assert_eq!(walk(&test, both), strings(&["dir/target", "src/main.rs"]));
    }

    #[test]
    fn exclusion_patterns_follow_the_last_matching_rule() {
        let test = tree(&["/w/a.log", "/w/keep.log", "/w/b.txt", "/w/sub/c.log"]);
        let options = WalkOptions {
            exclude: strings(&["*.log", "!keep.log"]),
            ..WalkOptions::default()
        };
        assert_eq!(walk(&test, options), strings(&["b.txt", "keep.log"]));
    }

    #[test]
    fn an_excluded_directory_is_not_entered_and_cannot_be_restored() {
        let test = tree(&["/w/out/keep.txt", "/w/a.txt"]);
        let options = WalkOptions {
            exclude: strings(&["out/", "!out/keep.txt"]),
            ..WalkOptions::default()
        };
        assert_eq!(walk(&test, options), strings(&["a.txt"]));
    }

    #[test]
    fn globs_match_the_relative_path_with_the_case_setting() {
        let test = tree(&["/w/a.txt", "/w/sub/b.txt", "/w/sub/c.md"]);
        let top = WalkOptions {
            globs: strings(&["*.txt"]),
            ..WalkOptions::default()
        };
        assert_eq!(walk(&test, top), strings(&["a.txt"]));
        let deep = WalkOptions {
            globs: strings(&["**/*.txt"]),
            ..WalkOptions::default()
        };
        assert_eq!(walk(&test, deep), strings(&["a.txt", "sub/b.txt"]));
        let sensitive = WalkOptions {
            globs: strings(&["*.TXT"]),
            ..WalkOptions::default()
        };
        assert!(walk(&test, sensitive).is_empty());
        let insensitive = WalkOptions {
            globs: strings(&["*.TXT"]),
            case_sensitive: false,
            ..WalkOptions::default()
        };
        assert_eq!(walk(&test, insensitive), strings(&["a.txt"]));
    }

    #[test]
    fn decomposed_names_match_composed_globs_and_keep_their_stored_form() {
        let test = tree(&["/w/cafe\u{301}.txt", "/w/cafe.txt"]);
        let options = WalkOptions {
            globs: strings(&["caf\u{e9}.txt"]),
            ..WalkOptions::default()
        };
        assert_eq!(walk(&test, options), strings(&["cafe\u{301}.txt"]));
    }

    #[test]
    fn links_are_listed_but_not_entered_without_follow_links() {
        let test = tree(&["/w/sub/a.txt"]);
        test.fs()
            .symlink("sub", &abs("/w/link"))
            .expect("the virtual file system accepts the link");
        assert_eq!(
            walk(&test, any_below_root()),
            strings(&["link", "sub", "sub/a.txt"])
        );
        let links = WalkOptions {
            kind: KindFilter::Link,
            ..WalkOptions::default()
        };
        assert_eq!(walk(&test, links), strings(&["link"]));
    }

    #[test]
    fn a_stopping_sink_ends_the_walk_before_its_end() {
        let test = tree(&["/w/a.txt", "/w/b.txt", "/w/c.txt"]);
        let walker = Walker::new(WalkOptions::default()).expect("no patterns to compile");
        let mut seen = 0;
        let summary = walker
            .run(test.fs().as_ref(), &[abs("/w")], &|| Ok(()), |_| {
                seen += 1;
                ControlFlow::Break(())
            })
            .expect("the walk succeeds");
        assert_eq!(seen, 1);
        assert!(!summary.complete);
        assert_eq!(summary.entries, 1);
    }

    #[test]
    fn a_missing_root_fails_and_a_file_root_is_a_single_entry() {
        let test = tree(&["/w/a.txt"]);
        let walker = Walker::new(WalkOptions::default()).expect("no patterns to compile");
        let error = walker
            .run(test.fs().as_ref(), &[abs("/missing")], &|| Ok(()), |_| {
                ControlFlow::Continue(())
            })
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::NotFound);
        let (delivered, summary) = walk_in(&test, "/w/a.txt", WalkOptions::default());
        assert_eq!(delivered, strings(&[""]));
        assert_eq!(summary.entries, 1);
    }

    #[test]
    fn invalid_patterns_are_rejected_when_the_walker_is_built() {
        let error = Walker::new(WalkOptions {
            globs: strings(&["[a-"]),
            ..WalkOptions::default()
        })
        .unwrap_err();
        assert_eq!(error.code(), ErrorCode::InvalidValue);
        assert_eq!(error.found.as_deref(), Some("[a-"));
    }

    #[test]
    fn the_error_of_the_checkpoint_ends_the_walk() {
        let test = tree(&["/w/a.txt"]);
        let walker = Walker::new(WalkOptions::default()).expect("no patterns to compile");
        let error = walker
            .run(
                test.fs().as_ref(),
                &[abs("/w")],
                &|| Err(Error::new(ErrorCode::Cancelled, "cancelled")),
                |_| ControlFlow::Continue(()),
            )
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::Cancelled);
    }

    #[test]
    fn ignore_files_apply_inside_repositories_and_deeper_rules_win() {
        let test = tree(&[
            "/w/.git/HEAD",
            "/w/a.log",
            "/w/b.txt",
            "/w/build/x.o",
            "/w/sub/keep.log",
            "/w/sub/other.log",
        ]);
        write(&test, "/w/.gitignore", "*.log\n/build/\n");
        write(&test, "/w/sub/.gitignore", "!keep.log\n");
        let plain = WalkOptions {
            exclude_presets: vec![ExcludePreset::Vcs],
            ..WalkOptions::default()
        };
        assert_eq!(walk(&test, plain.clone()).len(), 7);
        let ignoring = WalkOptions {
            gitignore: true,
            ..plain
        };
        assert_eq!(
            walk(&test, ignoring),
            strings(&[".gitignore", "b.txt", "sub/.gitignore", "sub/keep.log"])
        );
    }

    #[test]
    fn gitignore_files_outside_repositories_are_not_applied_but_ignore_files_are() {
        let test = tree(&["/p/a.txt", "/p/secret.key", "/p/b.md"]);
        write(&test, "/p/.gitignore", "*.txt\n");
        let options = WalkOptions {
            gitignore: true,
            ..WalkOptions::default()
        };
        let (delivered, _) = walk_in(&test, "/p", options.clone());
        assert_eq!(
            delivered,
            strings(&[".gitignore", "a.txt", "b.md", "secret.key"])
        );
        write(&test, "/p/.ignore", "*.key\n");
        let (delivered, _) = walk_in(&test, "/p", options);
        assert_eq!(
            delivered,
            strings(&[".gitignore", ".ignore", "a.txt", "b.md"])
        );
    }

    #[test]
    fn ignore_files_are_not_read_without_the_gitignore_option() {
        let test = tree(&["/p/a.txt"]);
        write(&test, "/p/.ignore", "a.txt\n");
        let (delivered, _) = walk_in(&test, "/p", WalkOptions::default());
        assert_eq!(delivered, strings(&[".ignore", "a.txt"]));
    }

    #[test]
    fn entries_report_depth_root_and_effective_kind() {
        let test = tree(&["/w/b/c.txt"]);
        let walker = Walker::new(any_below_root()).expect("no patterns to compile");
        let mut found = Vec::new();
        walker
            .run(
                test.fs().as_ref(),
                &[abs("/w"), abs("/w/b")],
                &|| Ok(()),
                |item| {
                    if let WalkItem::Entry(entry) = item {
                        found.push((
                            entry.relative_string(),
                            entry.depth,
                            entry.root,
                            entry.is_dir(),
                            entry.effective_kind(),
                        ));
                    }
                    ControlFlow::Continue(())
                },
            )
            .expect("the walk succeeds");
        assert_eq!(
            found,
            [
                ("b".to_owned(), 1, 0, true, FileKind::Directory),
                ("b/c.txt".to_owned(), 2, 0, false, FileKind::File),
                ("c.txt".to_owned(), 1, 1, false, FileKind::File),
            ]
        );
    }
}
