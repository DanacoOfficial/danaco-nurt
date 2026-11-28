// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-text::patch
// Responsibility:   Parsing and application of patches in unified and git format.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Parsing and application of patches in unified and git format.
//!
//! [`Patch::parse`] reads the file sections of a patch. [`apply_hunks`]
//! applies the hunks of one section to a text. [`plan`] applies a whole
//! patch to a set of files supplied by the caller and returns the resulting
//! file changes without writing anything, so the caller can perform them in
//! one transaction or reject the patch as a whole.
//!
//! A hunk is placed at the line given in its header, shifted by the offset of
//! the previous hunk. When the old lines are not found there, the hunk is
//! searched for at increasing distances. With a fuzz factor `n`, up to `n`
//! context lines at each end of the hunk may be ignored when no exact match
//! exists. Lines that differ only in a final carriage return match when no
//! exact match exists; added lines then take the line ending of the file.
//!
//! # Examples
//!
//! ```
//! use std::collections::BTreeMap;
//!
//! use nurt_text::patch::{ApplyOptions, FileChange, Patch, plan};
//!
//! let patch = Patch::parse(
//!     "--- a/hello.txt\n+++ b/hello.txt\n@@ -1 +1 @@\n-hello\n+hello, world\n",
//! )?;
//! let files = BTreeMap::from([("hello.txt".to_owned(), "hello\n".to_owned())]);
//! let options = ApplyOptions { strip: 1, ..ApplyOptions::default() };
//! let changes = plan(&patch, &options, |path| files.get(path).cloned())?;
//! assert_eq!(
//!     changes[0].change,
//!     FileChange::Modify {
//!         path: "hello.txt".to_owned(),
//!         content: "hello, world\n".to_owned(),
//!         mode: None,
//!     }
//! );
//! # Ok::<(), nurt_text::patch::PatchError>(())
//! ```

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use nurt_contracts::error::ErrorCode;

use crate::diff::{Hunk, HunkLine, Line, LineKind, split_lines};

/// Name that stands for a missing file in unified and git format.
const NULL_PATH: &str = "/dev/null";

/// Change a file section of a patch makes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FileOperation {
    /// Changes the content or the mode of an existing file.
    Modify,
    /// Creates a file.
    Create,
    /// Deletes a file.
    Delete,
    /// Renames a file and optionally changes its content.
    Rename,
    /// Creates a file from the content of another file.
    Copy,
}

/// File section of a patch.
///
/// Paths are stored as written in the patch; [`ApplyOptions::strip`] is
/// applied when the patch is planned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilePatch {
    /// Change the section makes.
    pub operation: FileOperation,
    /// The section has a `diff --git` header.
    pub git: bool,
    /// Old path from the `---` line or the `diff --git` header; `None` for
    /// `/dev/null`.
    pub old_path: Option<String>,
    /// New path from the `+++` line or the `diff --git` header; `None` for
    /// `/dev/null`.
    pub new_path: Option<String>,
    /// Source of a rename or copy from the `rename from` or `copy from` line.
    /// Git writes it without the `a/` prefix, so one component less is
    /// stripped from it.
    pub source: Option<String>,
    /// Target of a rename or copy from the `rename to` or `copy to` line.
    pub target: Option<String>,
    /// Mode of the old file, from `old mode` or `deleted file mode`.
    pub old_mode: Option<u32>,
    /// Mode of the new file, from `new mode` or `new file mode`.
    pub new_mode: Option<u32>,
    /// Hunks of the section in order.
    pub hunks: Vec<Hunk>,
    /// Line of the patch where the section starts, 1-based.
    pub line: usize,
}

/// Parsed patch: its file sections in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Patch {
    /// File sections in the order of the patch.
    pub files: Vec<FilePatch>,
}

impl Patch {
    /// Parses a patch in unified or git format.
    ///
    /// Text outside file sections, such as a commit message or `Index:`
    /// lines, is skipped. A section starts with a `diff --git` line, or with
    /// a `---` line followed by a `+++` line and a hunk. Git extended header
    /// lines `old mode`, `new mode`, `new file mode`, `deleted file mode`,
    /// `rename from`, `rename to`, `copy from`, `copy to`, `similarity index`
    /// and `index` are recognised.
    ///
    /// # Errors
    ///
    /// - [`PatchError::Malformed`] when a hunk header or a hunk line is
    ///   invalid, or a hunk is shorter or longer than its header states;
    /// - [`PatchError::Binary`] for a binary patch;
    /// - [`PatchError::NoChanges`] when the text contains no file section.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_text::patch::{FileOperation, Patch};
    ///
    /// let patch = Patch::parse(
    ///     "diff --git a/old.txt b/new.txt\n\
    ///      similarity index 100%\n\
    ///      rename from old.txt\n\
    ///      rename to new.txt\n",
    /// )?;
    /// assert_eq!(patch.files[0].operation, FileOperation::Rename);
    /// assert_eq!(patch.files[0].target.as_deref(), Some("new.txt"));
    /// # Ok::<(), nurt_text::patch::PatchError>(())
    /// ```
    pub fn parse(text: &str) -> Result<Self, PatchError> {
        Parser {
            lines: text.split_inclusive('\n').map(trim_newline).collect(),
            pos: 0,
        }
        .parse()
    }

    /// Returns the paths the patch reads or writes after `strip` is applied,
    /// sorted and without duplicates.
    ///
    /// # Errors
    ///
    /// [`PatchError::Strip`] when a path has fewer components than `strip`,
    /// [`PatchError::UnsafePath`] when a stripped path is absolute or contains
    /// `..`.
    pub fn paths(&self, strip: usize) -> Result<Vec<String>, PatchError> {
        let mut paths = BTreeSet::new();
        for file in &self.files {
            let names = [
                (&file.old_path, strip),
                (&file.new_path, strip),
                (&file.source, strip.saturating_sub(1)),
                (&file.target, strip.saturating_sub(1)),
            ];
            for (path, count) in names {
                if let Some(path) = path {
                    paths.insert(resolve_path(path, count)?);
                }
            }
        }
        Ok(paths.into_iter().collect())
    }
}

fn trim_newline(line: &str) -> &str {
    line.strip_suffix('\n').unwrap_or(line)
}

/// Options of [`plan`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ApplyOptions {
    /// Number of leading path components removed from each path, as with
    /// `patch -p`. Rename and copy paths lose one component less.
    pub strip: usize,
    /// Number of context lines that may be ignored at each end of a hunk.
    pub fuzz: usize,
    /// Applies the patch in reverse: undoes the changes it describes.
    pub reverse: bool,
}

/// Placement of an applied hunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HunkOutcome {
    /// Distance in lines between the line in the hunk header and the line
    /// where the hunk was applied.
    pub offset: isize,
    /// Number of context lines ignored at each end of the hunk.
    pub fuzz: usize,
}

/// Text after the hunks of one file were applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Applied {
    /// Resulting text.
    pub text: String,
    /// Placement of each hunk in order.
    pub hunks: Vec<HunkOutcome>,
}

/// Hunk that does not apply.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Rejection {
    /// Number of the hunk within its file section, 1-based.
    pub hunk: usize,
    /// Line of the file where the hunk was expected, 1-based.
    pub line: usize,
    /// Old lines of the hunk: context and removed lines.
    pub expected: Vec<String>,
    /// Lines of the file at the expected place.
    pub actual: Vec<String>,
}

/// Change of one file planned by [`plan`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileChange {
    /// Creates `path` with `content`.
    Create {
        /// Path of the new file.
        path: String,
        /// Content of the new file.
        content: String,
        /// Mode of the new file, when the patch states one.
        mode: Option<u32>,
    },
    /// Replaces the content of `path`.
    Modify {
        /// Path of the file.
        path: String,
        /// New content of the file.
        content: String,
        /// New mode of the file, when the patch changes it.
        mode: Option<u32>,
    },
    /// Deletes `path`.
    Delete {
        /// Path of the file.
        path: String,
    },
    /// Renames `from` to `to` and writes `content` to it.
    Rename {
        /// Old path.
        from: String,
        /// New path.
        to: String,
        /// Content of the renamed file.
        content: String,
        /// New mode of the file, when the patch changes it.
        mode: Option<u32>,
    },
}

/// File change with the placement of the hunks that produced it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedChange {
    /// Change of the file.
    pub change: FileChange,
    /// Placement of each applied hunk in order.
    pub hunks: Vec<HunkOutcome>,
}

/// Error of patch parsing or application.
///
/// [`PatchError::code`] maps each error to the error code of the command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PatchError {
    /// The patch text is invalid at `line` (1-based).
    Malformed {
        /// Line of the patch, 1-based.
        line: usize,
        /// What is wrong.
        reason: &'static str,
    },
    /// The text contains no file section.
    NoChanges,
    /// The patch changes a binary file.
    Binary {
        /// Path or description of the binary file.
        path: String,
    },
    /// A file section names no file for the change it makes.
    MissingPath {
        /// Line of the patch where the section starts, 1-based.
        line: usize,
    },
    /// A path has fewer components than the strip count.
    Strip {
        /// Path as written in the patch.
        path: String,
        /// Number of components to remove.
        strip: usize,
    },
    /// A path is absolute or contains `..`.
    UnsafePath {
        /// Path after stripping.
        path: String,
    },
    /// A file to change does not exist.
    NotFound {
        /// Path of the file.
        path: String,
    },
    /// A file to create already exists.
    Exists {
        /// Path of the file.
        path: String,
    },
    /// A hunk does not match the file.
    Rejected {
        /// Path of the file.
        path: String,
        /// The hunk and the lines that differ.
        rejection: Rejection,
    },
    /// A file to delete has content other than the patch states.
    Leftover {
        /// Path of the file.
        path: String,
    },
}

impl PatchError {
    /// Returns the error code: `E_PATCH_REJECTED` for a hunk or a deletion
    /// that does not match, `E_NOT_FOUND` and `E_EXISTS` for missing and
    /// existing files, `E_FORMAT` for invalid or unsupported patches.
    #[must_use]
    pub const fn code(&self) -> ErrorCode {
        match self {
            Self::Rejected { .. } | Self::Leftover { .. } => ErrorCode::PatchRejected,
            Self::NotFound { .. } => ErrorCode::NotFound,
            Self::Exists { .. } => ErrorCode::Exists,
            Self::Malformed { .. }
            | Self::NoChanges
            | Self::Binary { .. }
            | Self::MissingPath { .. }
            | Self::Strip { .. }
            | Self::UnsafePath { .. } => ErrorCode::Format,
        }
    }
}

impl fmt::Display for PatchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Malformed { line, reason } => {
                write!(f, "malformed patch at line {line}: {reason}")
            }
            Self::NoChanges => {
                f.write_str("no file changes found; the input must be a unified or git diff")
            }
            Self::Binary { path } => {
                write!(
                    f,
                    "binary patch for {path} is not supported; provide a text diff"
                )
            }
            Self::MissingPath { line } => write!(
                f,
                "file section at line {line} names no file; add the --- and +++ lines"
            ),
            Self::Strip { path, strip } => {
                write!(
                    f,
                    "cannot strip {strip} path components from {path}; lower strip"
                )
            }
            Self::UnsafePath { path } => write!(
                f,
                "path {path} is absolute or contains '..'; use paths relative to the root"
            ),
            Self::NotFound { path } => {
                write!(
                    f,
                    "file to patch does not exist: {path}; check root and strip"
                )
            }
            Self::Exists { path } => write!(f, "file to create already exists: {path}"),
            Self::Rejected { path, rejection } => write!(
                f,
                "hunk {} does not apply to {path} at line {}; check the file version or raise fuzz",
                rejection.hunk, rejection.line
            ),
            Self::Leftover { path } => write!(
                f,
                "file to delete differs from the patch: {path}; check the file version"
            ),
        }
    }
}

impl std::error::Error for PatchError {}

struct Parser<'t> {
    lines: Vec<&'t str>,
    pos: usize,
}

impl<'t> Parser<'t> {
    /// Returns the line at `index` without a final carriage return, for
    /// header lines that may come from a `crlf` patch.
    fn header(&self, index: usize) -> Option<&'t str> {
        self.lines
            .get(index)
            .map(|&line| line.strip_suffix('\r').unwrap_or(line))
    }

    fn malformed(&self, reason: &'static str) -> PatchError {
        PatchError::Malformed {
            line: self.pos + 1,
            reason,
        }
    }

    fn parse(mut self) -> Result<Patch, PatchError> {
        let mut files = Vec::new();
        while let Some(line) = self.header(self.pos) {
            if let Some(names) = line.strip_prefix("diff --git ") {
                files.push(self.git_section(names)?);
            } else if self.starts_unified_section() {
                files.push(self.unified_section()?);
            } else if let Some(description) = binary_description(line) {
                return Err(PatchError::Binary {
                    path: description.to_owned(),
                });
            } else {
                self.pos += 1;
            }
        }
        if files.is_empty() {
            Err(PatchError::NoChanges)
        } else {
            Ok(Patch { files })
        }
    }

    fn starts_unified_section(&self) -> bool {
        let at = |offset, prefix| {
            self.header(self.pos + offset)
                .is_some_and(|line| line.starts_with(prefix))
        };
        at(0, "--- ") && at(1, "+++ ") && at(2, "@@ -")
    }

    fn unified_section(&mut self) -> Result<FilePatch, PatchError> {
        let line = self.pos + 1;
        let (old_path, new_path) = self.labels()?;
        let operation = match (&old_path, &new_path) {
            (None, None) => return Err(self.malformed("both file names are /dev/null")),
            (None, Some(_)) => FileOperation::Create,
            (Some(_), None) => FileOperation::Delete,
            (Some(_), Some(_)) => FileOperation::Modify,
        };
        Ok(FilePatch {
            operation,
            git: false,
            old_path,
            new_path,
            source: None,
            target: None,
            old_mode: None,
            new_mode: None,
            hunks: self.hunks()?,
            line,
        })
    }

    /// Reads the `---` and `+++` lines at the current position.
    fn labels(&mut self) -> Result<(Option<String>, Option<String>), PatchError> {
        let old = self
            .header(self.pos)
            .and_then(|line| line.strip_prefix("--- "));
        let new = self
            .header(self.pos + 1)
            .and_then(|line| line.strip_prefix("+++ "));
        let (Some(old), Some(new)) = (old, new) else {
            return Err(self.malformed("expected --- and +++ lines"));
        };
        let old = parse_label(old).ok_or_else(|| self.malformed("invalid quoted file name"))?;
        self.pos += 1;
        let new = parse_label(new).ok_or_else(|| self.malformed("invalid quoted file name"))?;
        self.pos += 1;
        Ok((old, new))
    }

    fn git_section(&mut self, names: &str) -> Result<FilePatch, PatchError> {
        let line = self.pos + 1;
        let (old_path, new_path) = split_git_names(names).unzip();
        let mut file = FilePatch {
            operation: FileOperation::Modify,
            git: true,
            old_path,
            new_path,
            source: None,
            target: None,
            old_mode: None,
            new_mode: None,
            hunks: Vec::new(),
            line,
        };
        self.pos += 1;
        while let Some(header) = self.header(self.pos) {
            if !self.extended_header(header, &mut file)? {
                break;
            }
            self.pos += 1;
        }
        if self
            .header(self.pos)
            .is_some_and(|line| line.starts_with("--- "))
        {
            let (old_path, new_path) = self.labels()?;
            file.old_path = old_path;
            file.new_path = new_path;
        }
        match file.operation {
            FileOperation::Create => file.old_path = None,
            FileOperation::Delete => file.new_path = None,
            FileOperation::Modify | FileOperation::Rename | FileOperation::Copy => {}
        }
        file.hunks = self.hunks()?;
        Ok(file)
    }

    /// Applies one git extended header line to `file`; returns `false` when
    /// `header` is not an extended header line.
    fn extended_header(&self, header: &str, file: &mut FilePatch) -> Result<bool, PatchError> {
        let mode =
            |value: &str| parse_mode(value).ok_or_else(|| self.malformed("invalid file mode"));
        let name = |value: &str| {
            parse_name(value).ok_or_else(|| self.malformed("invalid quoted file name"))
        };
        if let Some(value) = header.strip_prefix("old mode ") {
            file.old_mode = Some(mode(value)?);
        } else if let Some(value) = header.strip_prefix("new mode ") {
            file.new_mode = Some(mode(value)?);
        } else if let Some(value) = header.strip_prefix("deleted file mode ") {
            file.operation = FileOperation::Delete;
            file.old_mode = Some(mode(value)?);
        } else if let Some(value) = header.strip_prefix("new file mode ") {
            file.operation = FileOperation::Create;
            file.new_mode = Some(mode(value)?);
        } else if let Some(value) = header
            .strip_prefix("rename from ")
            .or_else(|| header.strip_prefix("rename old "))
        {
            file.operation = FileOperation::Rename;
            file.source = Some(name(value)?);
        } else if let Some(value) = header
            .strip_prefix("rename to ")
            .or_else(|| header.strip_prefix("rename new "))
        {
            file.operation = FileOperation::Rename;
            file.target = Some(name(value)?);
        } else if let Some(value) = header.strip_prefix("copy from ") {
            file.operation = FileOperation::Copy;
            file.source = Some(name(value)?);
        } else if let Some(value) = header.strip_prefix("copy to ") {
            file.operation = FileOperation::Copy;
            file.target = Some(name(value)?);
        } else if header == "GIT binary patch" || binary_description(header).is_some() {
            let path = file.new_path.as_ref().or(file.old_path.as_ref());
            return Err(PatchError::Binary {
                path: path.cloned().unwrap_or_default(),
            });
        } else {
            let ignored = ["similarity index ", "dissimilarity index ", "index "];
            return Ok(ignored.iter().any(|prefix| header.starts_with(prefix)));
        }
        Ok(true)
    }

    fn hunks(&mut self) -> Result<Vec<Hunk>, PatchError> {
        let mut hunks = Vec::new();
        while self
            .header(self.pos)
            .is_some_and(|line| line.starts_with("@@ -"))
        {
            hunks.push(self.hunk()?);
        }
        Ok(hunks)
    }

    fn hunk(&mut self) -> Result<Hunk, PatchError> {
        let header = self.header(self.pos).unwrap_or_default();
        let (old_start, old_len, new_start, new_len, section) =
            parse_hunk_header(header).ok_or_else(|| self.malformed("invalid hunk header"))?;
        self.pos += 1;
        let (mut old_left, mut new_left) = (old_len, new_len);
        let mut lines: Vec<HunkLine> = Vec::with_capacity(old_len + new_len);
        while old_left > 0 || new_left > 0 {
            let Some(&raw) = self.lines.get(self.pos) else {
                return Err(self.malformed("hunk ends before the line counts of its header"));
            };
            let (kind, text) = match raw.as_bytes().first() {
                Some(b' ') => (LineKind::Context, &raw[1..]),
                Some(b'-') => (LineKind::Removed, &raw[1..]),
                Some(b'+') => (LineKind::Added, &raw[1..]),
                Some(b'\\') => {
                    self.mark_no_newline(&mut lines)?;
                    continue;
                }
                // An empty context line whose leading space was removed, for
                // example by an editor that trims trailing white space.
                None | Some(b'\r') if raw.len() <= 1 => (LineKind::Context, raw),
                _ => return Err(self.malformed("hunk line must start with ' ', '-' or '+'")),
            };
            if kind.in_old() {
                old_left = old_left.checked_sub(1).ok_or_else(|| {
                    self.malformed("hunk has more old lines than its header states")
                })?;
            }
            if kind.in_new() {
                new_left = new_left.checked_sub(1).ok_or_else(|| {
                    self.malformed("hunk has more new lines than its header states")
                })?;
            }
            lines.push(HunkLine::new(kind, text));
            self.pos += 1;
        }
        if self
            .lines
            .get(self.pos)
            .is_some_and(|line| line.starts_with('\\'))
        {
            self.mark_no_newline(&mut lines)?;
        }
        Ok(Hunk {
            old_start,
            old_len,
            new_start,
            new_len,
            header: section,
            lines,
        })
    }

    fn mark_no_newline(&mut self, lines: &mut [HunkLine]) -> Result<(), PatchError> {
        let last = lines
            .last_mut()
            .ok_or_else(|| self.malformed("marker line without a preceding line"))?;
        last.no_newline = true;
        self.pos += 1;
        Ok(())
    }
}

/// Returns the description of a `Binary files A and B differ` line.
fn binary_description(line: &str) -> Option<&str> {
    line.strip_prefix("Binary files ")?.strip_suffix(" differ")
}

type HunkHeader = (usize, usize, usize, usize, Option<String>);

/// Parses `@@ -l[,s] +l[,s] @@[ section]`.
fn parse_hunk_header(line: &str) -> Option<HunkHeader> {
    let rest = line.strip_prefix("@@ -")?;
    let (old, rest) = rest.split_once(" +")?;
    let (new, rest) = rest.split_once(" @@")?;
    let (old_start, old_len) = parse_range(old)?;
    let (new_start, new_len) = parse_range(new)?;
    let section = rest
        .strip_prefix(' ')
        .filter(|text| !text.is_empty())
        .map(str::to_owned);
    Some((old_start, old_len, new_start, new_len, section))
}

fn parse_range(text: &str) -> Option<(usize, usize)> {
    match text.split_once(',') {
        Some((start, len)) => Some((start.parse().ok()?, len.parse().ok()?)),
        None => Some((text.parse().ok()?, 1)),
    }
}

fn parse_mode(text: &str) -> Option<u32> {
    u32::from_str_radix(text.trim(), 8).ok()
}

/// Parses the file name of a `---` or `+++` line: a quoted name, or the text
/// up to a tab that starts a time stamp. Returns `Some(None)` for
/// `/dev/null` and `None` for an invalid quoted name.
fn parse_label(text: &str) -> Option<Option<String>> {
    let name = if text.starts_with('"') {
        unquote(text)?.0
    } else {
        text.split_once('\t')
            .map_or(text, |(name, _)| name)
            .to_owned()
    };
    Some((name != NULL_PATH).then_some(name))
}

/// Parses the name of a git extended header line, quoted or not.
fn parse_name(text: &str) -> Option<String> {
    if text.starts_with('"') {
        unquote(text).map(|(name, _)| name)
    } else {
        Some(text.to_owned())
    }
}

/// Splits the names of a `diff --git` line.
///
/// Unquoted names may contain spaces; the split is taken where both names
/// agree after their first component, as git writes them.
fn split_git_names(text: &str) -> Option<(String, String)> {
    if text.starts_with('"') {
        let (old, rest) = unquote(text)?;
        let new = parse_name(rest.strip_prefix(' ')?)?;
        return Some((old, new));
    }
    if let Some(index) = text.find(" \"") {
        let (new, _) = unquote(&text[index + 1..])?;
        return Some((text[..index].to_owned(), new));
    }
    let without_prefix = |name: &str| name.split_once('/').map(|(_, rest)| rest.to_owned());
    text.match_indices(' ').find_map(|(index, _)| {
        let (old, new) = (&text[..index], &text[index + 1..]);
        let agree =
            old == new || without_prefix(old).is_some_and(|rest| Some(rest) == without_prefix(new));
        (agree && !old.is_empty()).then(|| (old.to_owned(), new.to_owned()))
    })
}

/// Decodes a name in C quotes as git writes it; returns the name and the
/// text after the closing quote.
fn unquote(text: &str) -> Option<(String, &str)> {
    let body = text.strip_prefix('"')?;
    let mut bytes = Vec::with_capacity(body.len());
    let mut iter = body.bytes().enumerate();
    while let Some((index, byte)) = iter.next() {
        match byte {
            b'"' => return Some((String::from_utf8(bytes).ok()?, &body[index + 1..])),
            b'\\' => {
                let (_, escaped) = iter.next()?;
                let decoded = match escaped {
                    b'a' => 0x07,
                    b'b' => 0x08,
                    b'f' => 0x0C,
                    b'n' => b'\n',
                    b'r' => b'\r',
                    b't' => b'\t',
                    b'v' => 0x0B,
                    b'0'..=b'7' => {
                        let (_, second) = iter.next()?;
                        let (_, third) = iter.next()?;
                        let digits = [escaped, second, third];
                        u8::from_str_radix(std::str::from_utf8(&digits).ok()?, 8).ok()?
                    }
                    other => other,
                };
                bytes.push(decoded);
            }
            other => bytes.push(other),
        }
    }
    None
}

/// Removes `count` leading components from `path` and checks that the rest
/// is a relative path inside the root; returns it with `/` separators,
/// without empty and `.` components.
fn resolve_path(path: &str, count: usize) -> Result<String, PatchError> {
    let mut rest = path;
    for _ in 0..count {
        let (_, tail) = rest.split_once('/').ok_or_else(|| PatchError::Strip {
            path: path.to_owned(),
            strip: count,
        })?;
        rest = tail.trim_start_matches('/');
    }
    let unsafe_path = || PatchError::UnsafePath {
        path: rest.to_owned(),
    };
    let bytes = rest.as_bytes();
    let drive = bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':';
    if rest.starts_with(['/', '\\']) || drive {
        return Err(unsafe_path());
    }
    let parts: Vec<&str> = rest
        .split('/')
        .filter(|part| !part.is_empty() && *part != ".")
        .collect();
    if parts.is_empty()
        || parts
            .iter()
            .any(|part| part.split('\\').any(|piece| piece == ".."))
    {
        return Err(unsafe_path());
    }
    Ok(parts.join("/"))
}

/// Applies `hunks` to `text` in order.
///
/// Each hunk is tried at fuzz levels from zero to `fuzz`; at level `n` up to
/// `n` context lines are ignored at each end of the hunk. At each level an
/// exact match is preferred to one that ignores final carriage returns. At
/// level zero a hunk with less leading than trailing context must match at
/// the start of the text, and one with less trailing context at its end.
///
/// # Errors
///
/// [`Rejection`] for the first hunk whose old lines are not found.
///
/// # Examples
///
/// ```
/// use nurt_text::diff::unified_diff;
/// use nurt_text::patch::apply_hunks;
///
/// let hunks = unified_diff("a\nb\nc\n", "a\nB\nc\n", 3, false);
/// let applied = apply_hunks("a\r\nb\r\nc\r\n", &hunks, 0)?;
/// assert_eq!(applied.text, "a\r\nB\r\nc\r\n");
/// # Ok::<(), nurt_text::patch::Rejection>(())
/// ```
pub fn apply_hunks(text: &str, hunks: &[Hunk], fuzz: usize) -> Result<Applied, Rejection> {
    let lines = split_lines(text);
    let mut out = Output {
        text: String::with_capacity(text.len()),
        crlf: crlf_style(&lines),
        open: false,
    };
    let mut outcomes = Vec::with_capacity(hunks.len());
    let mut cursor = 0;
    let mut offset: isize = 0;
    for (index, hunk) in hunks.iter().enumerate() {
        let base = to_isize(hunk_base(hunk));
        let expected = base.saturating_add(offset);
        let Some(place) = locate(&lines, cursor, hunk, expected, fuzz) else {
            return Err(rejection(index, hunk, &lines, expected));
        };
        lines[cursor..place.start]
            .iter()
            .for_each(|line| out.existing(line, line.newline));
        // Added lines take the line ending of the file only when the match
        // ignored carriage returns; without that evidence they stay as given.
        let convert = place.tolerant;
        let mut position = place.start;
        for line in &hunk.lines[place.top..hunk.lines.len() - place.bottom] {
            match line.kind {
                LineKind::Context => {
                    if let Some(existing) = lines.get(position) {
                        out.existing(existing, existing.newline || !line.no_newline);
                    }
                    position += 1;
                }
                LineKind::Removed => position += 1,
                LineKind::Added => out.added(line, convert),
            }
        }
        offset = to_isize(place.start) - base - to_isize(place.top);
        outcomes.push(HunkOutcome {
            offset,
            fuzz: place.top.max(place.bottom),
        });
        cursor = position;
    }
    lines[cursor..]
        .iter()
        .for_each(|line| out.existing(line, line.newline));
    Ok(Applied {
        text: out.text,
        hunks: outcomes,
    })
}

/// Index of the first old line of a hunk in the text, 0-based.
const fn hunk_base(hunk: &Hunk) -> usize {
    if hunk.old_len == 0 {
        hunk.old_start
    } else {
        hunk.old_start.saturating_sub(1)
    }
}

fn to_isize(value: usize) -> isize {
    isize::try_from(value).unwrap_or(isize::MAX)
}

/// Line ending of a text: `Some(true)` for `crlf`, `Some(false)` for `lf`,
/// `None` for a text without line feeds. The first line feed decides.
fn crlf_style(lines: &[Line<'_>]) -> Option<bool> {
    lines
        .iter()
        .find(|line| line.newline)
        .map(|line| line.text.ends_with('\r'))
}

struct Output {
    text: String,
    crlf: Option<bool>,
    /// The last written line has no line feed.
    open: bool,
}

impl Output {
    fn line_ending(&self) -> &'static str {
        if self.crlf == Some(true) {
            "\r\n"
        } else {
            "\n"
        }
    }

    /// Closes a line without a line feed before another line follows it.
    fn close(&mut self) {
        if self.open {
            self.text.push_str(self.line_ending());
            self.open = false;
        }
    }

    fn existing(&mut self, line: &Line<'_>, newline: bool) {
        self.close();
        self.text.push_str(line.text);
        if line.newline {
            self.text.push('\n');
        } else if newline {
            self.text.push_str(self.line_ending());
        } else {
            self.open = true;
        }
    }

    fn added(&mut self, line: &HunkLine, convert: bool) {
        self.close();
        if !convert {
            self.text.push_str(&line.text);
            if line.no_newline {
                self.open = true;
            } else {
                self.text.push('\n');
            }
            return;
        }
        self.text.push_str(strip_cr(&line.text));
        if line.no_newline {
            self.open = true;
        } else {
            self.text.push_str(self.line_ending());
        }
    }
}

fn strip_cr(text: &str) -> &str {
    text.strip_suffix('\r').unwrap_or(text)
}

/// Position of a hunk in a text.
struct Place {
    /// Index of the first compared old line.
    start: usize,
    /// Context lines ignored at the start of the hunk.
    top: usize,
    /// Context lines ignored at the end of the hunk.
    bottom: usize,
    /// The match ignores final carriage returns.
    tolerant: bool,
}

fn locate(
    lines: &[Line<'_>],
    cursor: usize,
    hunk: &Hunk,
    expected: isize,
    fuzz: usize,
) -> Option<Place> {
    let leading = hunk
        .lines
        .iter()
        .take_while(|line| line.kind == LineKind::Context)
        .count();
    let trailing = hunk
        .lines
        .iter()
        .rev()
        .take_while(|line| line.kind == LineKind::Context)
        .count();
    let total = hunk.lines.len();
    for level in 0..=fuzz.min(leading.max(trailing)) {
        let top = level.min(leading);
        let bottom = level.min(trailing).min(total - top);
        let old: Vec<&HunkLine> = hunk.lines[top..total - bottom]
            .iter()
            .filter(|line| line.kind.in_old())
            .collect();
        let anchor = match level {
            0 if leading < trailing => Anchor::Start,
            0 if trailing < leading => Anchor::End,
            _ => Anchor::None,
        };
        for tolerant in [false, true] {
            let target = expected.saturating_add(to_isize(top));
            if let Some(start) = search(lines, cursor, &old, target, anchor, tolerant) {
                return Some(Place {
                    start,
                    top,
                    bottom,
                    tolerant,
                });
            }
        }
    }
    None
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Anchor {
    None,
    Start,
    End,
}

/// Finds the old lines in `lines` at or after `cursor`, trying `expected`
/// first and then positions at increasing distance, later before earlier.
fn search(
    lines: &[Line<'_>],
    cursor: usize,
    old: &[&HunkLine],
    expected: isize,
    anchor: Anchor,
    tolerant: bool,
) -> Option<usize> {
    let last = lines.len().checked_sub(old.len())?;
    if last < cursor {
        return None;
    }
    let fits = |start: usize| {
        old.iter().zip(&lines[start..]).all(|(want, have)| {
            if tolerant {
                strip_cr(&want.text) == strip_cr(have.text)
            } else {
                want.text == have.text
            }
        })
    };
    match anchor {
        Anchor::Start => return (cursor == 0 && fits(0)).then_some(0),
        Anchor::End => return fits(last).then_some(last),
        Anchor::None => {}
    }
    let first = usize::try_from(expected).unwrap_or(0).clamp(cursor, last);
    (0..=last - cursor).find_map(|distance| {
        let after = first.checked_add(distance).filter(|&start| start <= last);
        let before = first
            .checked_sub(distance)
            .filter(|&start| start >= cursor && distance > 0);
        after
            .filter(|&start| fits(start))
            .or_else(|| before.filter(|&start| fits(start)))
    })
}

fn rejection(index: usize, hunk: &Hunk, lines: &[Line<'_>], expected: isize) -> Rejection {
    let expected_lines: Vec<String> = hunk
        .lines
        .iter()
        .filter(|line| line.kind.in_old())
        .map(|line| line.text.clone())
        .collect();
    let start = usize::try_from(expected).unwrap_or(0).min(lines.len());
    let actual = lines[start..]
        .iter()
        .take(expected_lines.len())
        .map(|line| line.text.to_owned())
        .collect();
    Rejection {
        hunk: index + 1,
        line: start + 1,
        expected: expected_lines,
        actual,
    }
}

/// Applies a patch to files supplied by `read` and returns the resulting
/// changes in patch order, without writing anything.
///
/// `read` returns the content of a stripped path, or `None` for a missing
/// file; [`Patch::paths`] lists the paths. Later sections see the result of
/// earlier ones. A unified section changes its old path when that file
/// exists, otherwise its new path, which is created when every hunk only
/// adds lines.
///
/// # Errors
///
/// [`PatchError::Rejected`] for a hunk that does not match,
/// [`PatchError::Leftover`] for a file to delete with other content,
/// [`PatchError::NotFound`] and [`PatchError::Exists`] for missing and
/// existing files, [`PatchError::Strip`], [`PatchError::UnsafePath`] and
/// [`PatchError::MissingPath`] for unusable paths.
///
/// # Examples
///
/// ```
/// use nurt_text::patch::{ApplyOptions, FileChange, Patch, plan};
///
/// let patch = Patch::parse(
///     "diff --git a/run.sh b/run.sh\n\
///      new file mode 100755\n\
///      --- /dev/null\n\
///      +++ b/run.sh\n\
///      @@ -0,0 +1 @@\n\
///      +echo ok\n",
/// )?;
/// let options = ApplyOptions { strip: 1, ..ApplyOptions::default() };
/// let changes = plan(&patch, &options, |_| None)?;
/// assert_eq!(
///     changes[0].change,
///     FileChange::Create {
///         path: "run.sh".to_owned(),
///         content: "echo ok\n".to_owned(),
///         mode: Some(0o100_755),
///     }
/// );
/// # Ok::<(), nurt_text::patch::PatchError>(())
/// ```
pub fn plan<F>(
    patch: &Patch,
    options: &ApplyOptions,
    read: F,
) -> Result<Vec<PlannedChange>, PatchError>
where
    F: FnMut(&str) -> Option<String>,
{
    let mut tree = Tree {
        read,
        overlay: BTreeMap::new(),
    };
    let mut changes = Vec::with_capacity(patch.files.len());
    for file in &patch.files {
        let hunks: Cow<'_, [Hunk]> = if options.reverse {
            Cow::Owned(file.hunks.iter().map(Hunk::reversed).collect())
        } else {
            Cow::Borrowed(&file.hunks)
        };
        let (old_mode, new_mode) = if options.reverse {
            (file.new_mode, file.old_mode)
        } else {
            (file.old_mode, file.new_mode)
        };
        let action = resolve_action(file, options, &hunks, &mut tree)?;
        if let Some(change) = tree.perform(action, &hunks, options.fuzz, old_mode, new_mode)? {
            changes.push(change);
        }
    }
    Ok(changes)
}

/// File operation of a section after paths are resolved and the direction is
/// applied.
enum Action {
    Create(String),
    Delete(String),
    Modify(String),
    Rename {
        from: String,
        to: String,
    },
    Copy {
        from: String,
        to: String,
    },
    /// Reverse of a copy: deletes `copy` if it equals `source` with the hunks.
    Uncopy {
        copy: String,
        source: String,
    },
}

fn resolve_action<F>(
    file: &FilePatch,
    options: &ApplyOptions,
    hunks: &[Hunk],
    tree: &mut Tree<F>,
) -> Result<Action, PatchError>
where
    F: FnMut(&str) -> Option<String>,
{
    let full = |path: &Option<String>| {
        path.as_deref()
            .map(|path| resolve_path(path, options.strip))
            .transpose()
    };
    let short = |path: &Option<String>| {
        path.as_deref()
            .map(|path| resolve_path(path, options.strip.saturating_sub(1)))
            .transpose()
    };
    let missing = || PatchError::MissingPath { line: file.line };
    let old = full(&file.old_path)?;
    let new = full(&file.new_path)?;
    let action = match file.operation {
        FileOperation::Create => {
            let path = new.or(old).ok_or_else(missing)?;
            if options.reverse {
                Action::Delete(path)
            } else {
                Action::Create(path)
            }
        }
        FileOperation::Delete => {
            let path = old.or(new).ok_or_else(missing)?;
            if options.reverse {
                Action::Create(path)
            } else {
                Action::Delete(path)
            }
        }
        FileOperation::Modify if file.git => Action::Modify(new.or(old).ok_or_else(missing)?),
        FileOperation::Modify => {
            let old_exists = old.as_ref().is_some_and(|path| tree.get(path).is_some());
            let new_exists = new.as_ref().is_some_and(|path| tree.get(path).is_some());
            let only_adds = hunks.iter().all(|hunk| hunk.old_len == 0);
            match (old, new) {
                (Some(path), _) if old_exists => Action::Modify(path),
                (_, Some(path)) if new_exists => Action::Modify(path),
                (old, new) if only_adds => Action::Create(new.or(old).ok_or_else(missing)?),
                (old, new) => Action::Modify(old.or(new).ok_or_else(missing)?),
            }
        }
        FileOperation::Rename | FileOperation::Copy => {
            let from = short(&file.source)?.or(old).ok_or_else(missing)?;
            let to = short(&file.target)?.or(new).ok_or_else(missing)?;
            match (file.operation, options.reverse) {
                (FileOperation::Copy, false) => Action::Copy { from, to },
                (FileOperation::Copy, true) => Action::Uncopy {
                    copy: to,
                    source: from,
                },
                (_, false) => Action::Rename { from, to },
                (_, true) => Action::Rename { from: to, to: from },
            }
        }
    };
    Ok(action)
}

/// Files as seen by a plan: changes of earlier sections over `read`.
struct Tree<F> {
    read: F,
    overlay: BTreeMap<String, Option<String>>,
}

impl<F> Tree<F>
where
    F: FnMut(&str) -> Option<String>,
{
    fn get(&mut self, path: &str) -> Option<String> {
        match self.overlay.get(path) {
            Some(content) => content.clone(),
            None => (self.read)(path),
        }
    }

    fn existing(&mut self, path: &str) -> Result<String, PatchError> {
        self.get(path).ok_or_else(|| PatchError::NotFound {
            path: path.to_owned(),
        })
    }

    fn absent(&mut self, path: &str) -> Result<(), PatchError> {
        match self.get(path) {
            Some(_) => Err(PatchError::Exists {
                path: path.to_owned(),
            }),
            None => Ok(()),
        }
    }

    fn perform(
        &mut self,
        action: Action,
        hunks: &[Hunk],
        fuzz: usize,
        old_mode: Option<u32>,
        new_mode: Option<u32>,
    ) -> Result<Option<PlannedChange>, PatchError> {
        let changed_mode = new_mode.filter(|&mode| Some(mode) != old_mode);
        let (change, outcomes) = match action {
            Action::Create(path) => {
                self.absent(&path)?;
                let applied = apply_file(&path, "", hunks, fuzz)?;
                self.overlay
                    .insert(path.clone(), Some(applied.text.clone()));
                (
                    FileChange::Create {
                        path,
                        content: applied.text,
                        mode: new_mode,
                    },
                    applied.hunks,
                )
            }
            Action::Delete(path) => {
                let applied = apply_file(&path, &self.existing(&path)?, hunks, fuzz)?;
                if !applied.text.is_empty() {
                    return Err(PatchError::Leftover { path });
                }
                self.overlay.insert(path.clone(), None);
                (FileChange::Delete { path }, applied.hunks)
            }
            Action::Modify(path) => {
                if hunks.is_empty() && changed_mode.is_none() {
                    return Ok(None);
                }
                let applied = apply_file(&path, &self.existing(&path)?, hunks, fuzz)?;
                self.overlay
                    .insert(path.clone(), Some(applied.text.clone()));
                (
                    FileChange::Modify {
                        path,
                        content: applied.text,
                        mode: changed_mode,
                    },
                    applied.hunks,
                )
            }
            Action::Rename { from, to } => {
                let content = self.existing(&from)?;
                if to != from {
                    self.absent(&to)?;
                }
                let applied = apply_file(&from, &content, hunks, fuzz)?;
                self.overlay.insert(from.clone(), None);
                self.overlay.insert(to.clone(), Some(applied.text.clone()));
                let change = FileChange::Rename {
                    from,
                    to,
                    content: applied.text,
                    mode: changed_mode,
                };
                (change, applied.hunks)
            }
            Action::Copy { from, to } => {
                let content = self.existing(&from)?;
                self.absent(&to)?;
                let applied = apply_file(&from, &content, hunks, fuzz)?;
                self.overlay.insert(to.clone(), Some(applied.text.clone()));
                (
                    FileChange::Create {
                        path: to,
                        content: applied.text,
                        mode: new_mode,
                    },
                    applied.hunks,
                )
            }
            Action::Uncopy { copy, source } => {
                let applied = apply_file(&copy, &self.existing(&copy)?, hunks, fuzz)?;
                if applied.text != self.existing(&source)? {
                    return Err(PatchError::Leftover { path: copy });
                }
                self.overlay.insert(copy.clone(), None);
                (FileChange::Delete { path: copy }, applied.hunks)
            }
        };
        Ok(Some(PlannedChange {
            change,
            hunks: outcomes,
        }))
    }
}

fn apply_file(path: &str, text: &str, hunks: &[Hunk], fuzz: usize) -> Result<Applied, PatchError> {
    apply_hunks(text, hunks, fuzz).map_err(|rejection| PatchError::Rejected {
        path: path.to_owned(),
        rejection,
    })
}

impl fmt::Display for Rejection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "hunk {} does not apply at line {}", self.hunk, self.line)
    }
}

impl std::error::Error for Rejection {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::unified_diff;

    fn modify(path: &str, content: &str) -> FileChange {
        FileChange::Modify {
            path: path.to_owned(),
            content: content.to_owned(),
            mode: None,
        }
    }

    #[test]
    fn parses_unified_section_with_time_stamps() {
        let patch = Patch::parse(
            "--- old.txt\t2026-01-02 03:04:05.000000000 +0000\n\
             +++ new.txt\t2026-01-02 03:04:06.000000000 +0000\n\
             @@ -1,2 +1,2 @@\n a\n-b\n+c\n",
        )
        .unwrap();
        let file = &patch.files[0];
        assert_eq!(file.operation, FileOperation::Modify);
        assert_eq!(file.old_path.as_deref(), Some("old.txt"));
        assert_eq!(file.new_path.as_deref(), Some("new.txt"));
        assert_eq!(file.hunks[0].prefixed_lines(), [" a", "-b", "+c"]);
    }

    #[test]
    fn skips_text_around_sections() {
        let patch = Patch::parse(
            "Subject: fix\n\nIndex: a\n--- a\n+++ a\n@@ -1 +1 @@\n-x\n+y\n-- \n2.43\n",
        )
        .unwrap();
        assert_eq!(patch.files.len(), 1);
    }

    #[test]
    fn rejects_text_without_sections() {
        assert_eq!(Patch::parse("hello\n"), Err(PatchError::NoChanges));
        assert_eq!(PatchError::NoChanges.code(), ErrorCode::Format);
    }

    #[test]
    fn rejects_truncated_hunk() {
        let error = Patch::parse("--- a\n+++ a\n@@ -1,2 +1,2 @@\n a\n").unwrap_err();
        assert_eq!(
            error,
            PatchError::Malformed {
                line: 5,
                reason: "hunk ends before the line counts of its header"
            }
        );
    }

    #[test]
    fn rejects_invalid_hunk_line() {
        let error = Patch::parse("--- a\n+++ a\n@@ -1 +1 @@\n*x\n+y\n").unwrap_err();
        assert!(matches!(error, PatchError::Malformed { line: 4, .. }));
    }

    #[test]
    fn reads_marker_of_missing_line_feed() {
        let patch =
            Patch::parse("--- a\n+++ a\n@@ -1 +1 @@\n-x\n\\ No newline at end of file\n+x\n")
                .unwrap();
        let lines = &patch.files[0].hunks[0].lines;
        assert!(lines[0].no_newline);
        assert!(!lines[1].no_newline);
    }

    #[test]
    fn accepts_context_line_without_leading_space() {
        let patch = Patch::parse("--- a\n+++ a\n@@ -1,3 +1,3 @@\n a\n\n-b\n+c\n").unwrap();
        assert_eq!(
            patch.files[0].hunks[0].lines[1],
            HunkLine::new(LineKind::Context, "")
        );
    }

    #[test]
    fn rejects_binary_patch() {
        let error = Patch::parse(
            "diff --git a/x.png b/x.png\nindex 1..2 100644\nGIT binary patch\nliteral 1\n",
        )
        .unwrap_err();
        assert_eq!(
            error,
            PatchError::Binary {
                path: "b/x.png".to_owned()
            }
        );
        let error = Patch::parse("Binary files a/x.png and b/x.png differ\n").unwrap_err();
        assert_eq!(error.code(), ErrorCode::Format);
    }

    #[test]
    fn decodes_quoted_names() {
        let patch = Patch::parse(
            "diff --git \"a/t\\303\\251st\\tx\" \"b/t\\303\\251st\\tx\"\nnew file mode 100644\n",
        )
        .unwrap();
        assert_eq!(patch.files[0].new_path.as_deref(), Some("b/t\u{e9}st\tx"));
    }

    #[test]
    fn splits_git_names_with_spaces() {
        assert_eq!(
            split_git_names("a/my file.txt b/my file.txt"),
            Some(("a/my file.txt".to_owned(), "b/my file.txt".to_owned()))
        );
    }

    #[test]
    fn strips_leading_components() {
        assert_eq!(resolve_path("a//src/main.rs", 1).unwrap(), "src/main.rs");
        assert_eq!(resolve_path("/abs/x", 1).unwrap(), "abs/x");
        assert!(matches!(
            resolve_path("x", 1),
            Err(PatchError::Strip { .. })
        ));
        assert!(matches!(
            resolve_path("a/../x", 0),
            Err(PatchError::UnsafePath { .. })
        ));
        assert!(matches!(
            resolve_path("/etc/passwd", 0),
            Err(PatchError::UnsafePath { .. })
        ));
        assert!(matches!(
            resolve_path("C:/x", 0),
            Err(PatchError::UnsafePath { .. })
        ));
        assert!(matches!(
            resolve_path("a\\..\\x", 0),
            Err(PatchError::UnsafePath { .. })
        ));
    }

    #[test]
    fn applies_hunk_with_offset() {
        let hunks = unified_diff("a\nb\nc\nd\n", "a\nb\nC\nd\n", 1, false);
        let applied = apply_hunks("0\n1\na\nb\nc\nd\n", &hunks, 0).unwrap();
        assert_eq!(applied.text, "0\n1\na\nb\nC\nd\n");
        assert_eq!(applied.hunks, [HunkOutcome { offset: 2, fuzz: 0 }]);
    }

    #[test]
    fn applies_hunk_with_fuzz_only_when_allowed() {
        let hunks = unified_diff("a\nb\nc\nd\ne\n", "a\nb\nC\nd\ne\n", 2, false);
        let changed = "a\nX\nc\nd\ne\n";
        let rejection = apply_hunks(changed, &hunks, 0).unwrap_err();
        assert_eq!(rejection.hunk, 1);
        assert_eq!(rejection.expected, ["a", "b", "c", "d", "e"]);
        assert_eq!(rejection.actual, ["a", "X", "c", "d", "e"]);
        let applied = apply_hunks(changed, &hunks, 1).unwrap_err();
        assert_eq!(applied.hunk, 1);
        let applied = apply_hunks(changed, &hunks, 2).unwrap();
        assert_eq!(applied.text, "a\nX\nC\nd\ne\n");
        assert_eq!(applied.hunks[0].fuzz, 2);
    }

    #[test]
    fn anchors_hunk_at_start_of_file() {
        let hunks = unified_diff("a\nb\nc\nd\n", "A\nb\nc\nd\n", 3, false);
        assert!(apply_hunks("z\nb\nc\nd\na\nb\nc\nd\n", &hunks, 0).is_err());
        assert_eq!(
            apply_hunks("z\na\nb\nc\nd\n", &hunks, 1).unwrap().text,
            "z\nA\nb\nc\nd\n"
        );
    }

    #[test]
    fn keeps_crlf_of_file_for_lf_patch() {
        let hunks = unified_diff("a\nb\n", "a\nb\nc\n", 3, false);
        assert_eq!(
            apply_hunks("a\r\nb\r\n", &hunks, 0).unwrap().text,
            "a\r\nb\r\nc\r\n"
        );
        let hunks = unified_diff("", "x\ny\n", 3, false);
        assert_eq!(apply_hunks("", &hunks, 0).unwrap().text, "x\ny\n");
    }

    #[test]
    fn keeps_lf_of_file_for_crlf_patch() {
        let hunks = unified_diff("a\r\nb\r\n", "a\r\nB\r\n", 3, false);
        assert_eq!(apply_hunks("a\nb\n", &hunks, 0).unwrap().text, "a\nB\n");
    }

    #[test]
    fn adds_line_feed_before_appended_lines() {
        let patch = Patch::parse("--- a\n+++ a\n@@ -1 +1,2 @@\n x\n+y\n").unwrap();
        assert_eq!(
            apply_hunks("x", &patch.files[0].hunks, 0).unwrap().text,
            "x\ny\n"
        );
    }

    #[test]
    fn plans_reverse_creation_as_deletion() {
        let patch = Patch::parse("--- /dev/null\n+++ b/n.txt\n@@ -0,0 +1 @@\n+n\n").unwrap();
        let options = ApplyOptions {
            strip: 1,
            reverse: true,
            ..ApplyOptions::default()
        };
        let changes = plan(&patch, &options, |_| Some("n\n".to_owned())).unwrap();
        assert_eq!(
            changes[0].change,
            FileChange::Delete {
                path: "n.txt".to_owned()
            }
        );
        let error = plan(&patch, &options, |_| Some("other\n".to_owned())).unwrap_err();
        assert_eq!(error.code(), ErrorCode::PatchRejected);
    }

    #[test]
    fn creates_file_from_unified_section_with_empty_old_range() {
        let patch = Patch::parse("--- n.txt\n+++ n.txt\n@@ -0,0 +1 @@\n+n\n").unwrap();
        let changes = plan(&patch, &ApplyOptions::default(), |_| None).unwrap();
        assert!(matches!(&changes[0].change, FileChange::Create { path, .. } if path == "n.txt"));
    }

    #[test]
    fn sees_earlier_sections_of_same_patch() {
        let text = "--- a\n+++ a\n@@ -1 +1 @@\n-1\n+2\n--- a\n+++ a\n@@ -1 +1 @@\n-2\n+3\n";
        let patch = Patch::parse(text).unwrap();
        let changes = plan(&patch, &ApplyOptions::default(), |_| Some("1\n".to_owned())).unwrap();
        assert_eq!(changes[1].change, modify("a", "3\n"));
    }

    #[test]
    fn reports_missing_and_existing_files() {
        let patch = Patch::parse("--- a\n+++ a\n@@ -1 +1 @@\n-1\n+2\n").unwrap();
        let error = plan(&patch, &ApplyOptions::default(), |_| None).unwrap_err();
        assert_eq!(
            error,
            PatchError::NotFound {
                path: "a".to_owned()
            }
        );
        assert_eq!(error.code(), ErrorCode::NotFound);
        let patch = Patch::parse("--- /dev/null\n+++ a\n@@ -0,0 +1 @@\n+1\n").unwrap();
        let error = plan(&patch, &ApplyOptions::default(), |_| Some(String::new())).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Exists);
    }

    #[test]
    fn reports_rejected_hunk_with_path() {
        let patch = Patch::parse("--- a\n+++ a\n@@ -1 +1 @@\n-1\n+2\n").unwrap();
        let error = plan(&patch, &ApplyOptions::default(), |_| Some("9\n".to_owned())).unwrap_err();
        assert_eq!(
            error.to_string(),
            "hunk 1 does not apply to a at line 1; check the file version or raise fuzz"
        );
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
