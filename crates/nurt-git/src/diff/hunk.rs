// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Hunks of a unified difference in the layout of Git.
//!
//! Changes closer than twice the context (plus `diff.interHunkContext`) share
//! a hunk. Context lines come from the new text. The header text is the
//! nearest function line above the first line of the hunk in the old text.
//! With the function context, a hunk starts at the function line above its
//! first change (together with the non-blank lines directly above it) and
//! ends before the next function line, without the blank lines in front of
//! it.

use std::ops::Range;

use bstr::{BString, ByteSlice};
use nurt_contracts::record::{Hunk as HunkRecord, HunkStatus, MovedBlock as MovedRecord};
use nurt_contracts::value::{Bytes, Path, Record};
use sha2::{Digest, Sha256};

use super::funcname::FuncMatcher;
use super::id::HunkId;
use super::lines::{Change, LineOptions, changes, is_blank, line_body, split_lines};

/// Text of the line that marks a missing line feed at the end of a file.
pub const NO_NEWLINE: &str = "\\ No newline at end of file";

/// Number of hexadecimal digits of a hunk fingerprint.
pub const FINGERPRINT_LEN: usize = 6;

/// Kind of a line of a hunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LineKind {
    /// Unchanged line, prefix ` `.
    Context,
    /// Added line, prefix `+`.
    Added,
    /// Removed line, prefix `-`.
    Removed,
    /// The previous line has no line feed, written `\ No newline at end of file`.
    NoNewline,
}

impl LineKind {
    /// Prefix of the line in a unified difference.
    #[must_use]
    pub fn prefix(self) -> u8 {
        match self {
            Self::Context => b' ',
            Self::Added => b'+',
            Self::Removed => b'-',
            Self::NoNewline => b'\\',
        }
    }
}

/// Line of a hunk.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct HunkLine {
    /// Kind of the line.
    pub kind: LineKind,
    /// Text without prefix and line feed; empty for [`LineKind::NoNewline`].
    pub text: BString,
}

impl HunkLine {
    /// The line as written in a unified difference, without line feed.
    #[must_use]
    pub fn render(&self) -> BString {
        if self.kind == LineKind::NoNewline {
            return BString::from(NO_NEWLINE);
        }
        let mut out = Vec::with_capacity(self.text.len() + 1);
        out.push(self.kind.prefix());
        out.extend_from_slice(&self.text);
        out.into()
    }
}

/// Block of at least three lines removed at one place and added at another.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MovedBlock {
    /// File the lines were removed from.
    pub from_path: BString,
    /// First removed line in the old text, from 1.
    pub from_line: u32,
    /// File the lines were added to.
    pub to_path: BString,
    /// First added line in the new text, from 1.
    pub to_line: u32,
    /// Number of lines.
    pub lines: u32,
}

/// Hunk of one file with its position in the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hunk {
    /// File relative to the repository root, after the change.
    pub path: BString,
    /// File before a rename or copy.
    pub old_path: Option<BString>,
    /// Status of the file.
    pub status: HunkStatus,
    /// Position of the hunk in the file, from 1.
    pub number: u32,
    /// First old line as written in the header.
    pub old_start: u32,
    /// Number of old lines.
    pub old_len: u32,
    /// First new line as written in the header.
    pub new_start: u32,
    /// Number of new lines.
    pub new_len: u32,
    /// Function text after `@@`.
    pub header: Option<BString>,
    /// Lines in order.
    pub lines: Vec<HunkLine>,
    /// The file is binary; such a hunk has no lines.
    pub binary: bool,
    /// Mode before, in octal.
    pub old_mode: Option<String>,
    /// Mode after, in octal.
    pub new_mode: Option<String>,
    /// Moved blocks that touch the lines of this hunk.
    pub moved: Vec<MovedBlock>,
}

impl Hunk {
    /// Identifier `path#number` without fingerprint and rows.
    #[must_use]
    pub fn id(&self) -> HunkId {
        HunkId {
            path: self.path.to_str_lossy().into_owned(),
            number: self.number,
            fingerprint: None,
            rows: None,
        }
    }

    /// First six hexadecimal digits of the SHA-256 of the rendered lines
    /// joined by line feeds.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_git::diff::{Hunk, HunkLine, LineKind};
    ///
    /// let mut hunk = Hunk::binary_placeholder("a.txt".into());
    /// hunk.lines = vec![HunkLine { kind: LineKind::Added, text: "x".into() }];
    /// assert_eq!(hunk.fingerprint().len(), 6);
    /// ```
    #[must_use]
    pub fn fingerprint(&self) -> String {
        let mut hasher = Sha256::new();
        for (index, line) in self.lines.iter().enumerate() {
            if index > 0 {
                hasher.update(b"\n");
            }
            hasher.update(line.render().as_slice());
        }
        let digest = hasher.finalize();
        let mut text = hex::encode(digest);
        text.truncate(FINGERPRINT_LEN);
        text
    }

    /// The header line `@@ -a,b +c,d @@ text`.
    #[must_use]
    pub fn header_line(&self) -> String {
        let mut out = format!(
            "@@ -{} +{} @@",
            range_text(self.old_start, self.old_len),
            range_text(self.new_start, self.new_len)
        );
        if let Some(text) = &self.header {
            out.push(' ');
            out.push_str(&text.to_str_lossy());
        }
        out
    }

    /// Number of added and removed lines.
    #[must_use]
    pub fn counts(&self) -> (u64, u64) {
        self.lines
            .iter()
            .fold((0, 0), |(added, removed), line| match line.kind {
                LineKind::Added => (added + 1, removed),
                LineKind::Removed => (added, removed + 1),
                LineKind::Context | LineKind::NoNewline => (added, removed),
            })
    }

    /// A hunk without lines for a binary file.
    #[must_use]
    pub fn binary_placeholder(path: BString) -> Self {
        Self {
            path,
            old_path: None,
            status: HunkStatus::Modified,
            number: 1,
            old_start: 0,
            old_len: 0,
            new_start: 0,
            new_len: 0,
            header: None,
            lines: Vec::new(),
            binary: true,
            old_mode: None,
            new_mode: None,
            moved: Vec::new(),
        }
    }

    /// The `hunk` record; `with_id` adds the identifier and the fingerprint,
    /// which are valid only for the canonical difference.
    #[must_use]
    pub fn to_record(&self, with_id: bool) -> Record {
        Record::from(self.to_hunk_record(with_id))
    }

    /// The typed `hunk` record; `with_id` adds the identifier and the
    /// fingerprint.
    #[must_use]
    pub fn to_hunk_record(&self, with_id: bool) -> HunkRecord {
        HunkRecord {
            path: Path::from(self.path.to_str_lossy().into_owned()),
            raw_path: raw_bytes(&self.path),
            old_start: u64::from(self.old_start),
            old_len: u64::from(self.old_len),
            new_start: u64::from(self.new_start),
            new_len: u64::from(self.new_len),
            lines: self
                .lines
                .iter()
                .map(|line| line.render().to_str_lossy().into_owned())
                .collect(),
            id: with_id.then(|| self.id().to_string()),
            header: self
                .header
                .as_ref()
                .map(|text| text.to_str_lossy().into_owned()),
            old_path: self
                .old_path
                .as_ref()
                .map(|path| Path::from(path.to_str_lossy().into_owned())),
            raw_old_path: self.old_path.as_ref().and_then(raw_bytes),
            status: Some(self.status),
            binary: self.binary.then_some(true),
            old_mode: self.old_mode.clone(),
            new_mode: self.new_mode.clone(),
            ignore_whitespace: None,
            fingerprint: with_id.then(|| self.fingerprint()),
            staged: None,
            untracked: None,
            unit: None,
            moved: (!self.moved.is_empty()).then(|| self.moved.iter().map(moved_record).collect()),
        }
    }
}

/// The bytes of a path of the repository that is not valid UTF-8; the record
/// shows such a path with U+FFFD and carries the bytes beside it.
fn raw_bytes(path: &BString) -> Option<Bytes> {
    path.to_str().is_err().then(|| Bytes::new(path.to_vec()))
}

fn moved_record(block: &MovedBlock) -> MovedRecord {
    MovedRecord {
        from_path: Path::from(block.from_path.to_str_lossy().into_owned()),
        from_line: u64::from(block.from_line),
        to_path: Path::from(block.to_path.to_str_lossy().into_owned()),
        to_line: u64::from(block.to_line),
        lines: u64::from(block.lines),
    }
}

fn range_text(start: u32, len: u32) -> String {
    if len == 1 {
        start.to_string()
    } else {
        format!("{start},{len}")
    }
}

/// Shape of the hunks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layout {
    /// Context lines around each change.
    pub context: u32,
    /// Additional common lines that still join two changes.
    pub inter_hunk_context: u32,
    /// Extend each hunk to the whole enclosing function.
    pub function_context: bool,
}

impl Default for Layout {
    fn default() -> Self {
        Self {
            context: CANONICAL_CONTEXT,
            inter_hunk_context: 0,
            function_context: false,
        }
    }
}

/// Context of the canonical difference that numbers hunks.
pub const CANONICAL_CONTEXT: u32 = 3;

/// Line difference of two texts, before it is cut into hunks.
#[derive(Debug, Clone)]
pub struct TextDiff<'a> {
    pub(crate) old: Vec<&'a [u8]>,
    pub(crate) new: Vec<&'a [u8]>,
    pub(crate) changes: Vec<Change>,
}

impl<'a> TextDiff<'a> {
    /// Compares two texts.
    #[must_use]
    pub fn new(old: &'a [u8], new: &'a [u8], options: &LineOptions) -> Self {
        let old = split_lines(old);
        let new = split_lines(new);
        let changes = changes(&old, &new, options);
        Self { old, new, changes }
    }

    /// Number of added and removed lines.
    #[must_use]
    pub fn counts(&self) -> (u64, u64) {
        self.changes
            .iter()
            .fold((0, 0), |(added, removed), change| {
                (
                    added + change.new.len() as u64,
                    removed + change.old.len() as u64,
                )
            })
    }

    /// Keeps only the changes that touch `lines` of the new text (0-based,
    /// end exclusive); a removal touches the range when it sits inside it.
    pub fn retain_new_lines(&mut self, lines: &Range<usize>) {
        self.changes.retain(|change| {
            if change.new.is_empty() {
                change.new.start >= lines.start && change.new.start <= lines.end
            } else {
                change.new.start < lines.end && change.new.end > lines.start
            }
        });
    }

    /// The texts are equal under the comparison options.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }

    /// Cuts the changes into hunks numbered from 1.
    #[must_use]
    pub fn hunks(&self, path: &BString, layout: &Layout, funcs: &FuncMatcher) -> Vec<Hunk> {
        let groups = self.groups(layout, funcs);
        groups
            .into_iter()
            .enumerate()
            .map(|(index, group)| self.emit(path, index, &group, funcs))
            .collect()
    }

    fn groups(&self, layout: &Layout, funcs: &FuncMatcher) -> Vec<Group> {
        let mut groups = Vec::new();
        let mut index = 0;
        while index < self.changes.len() {
            let group = self.group_from(index, layout, funcs);
            index = group.changes.end;
            groups.push(group);
        }
        groups
    }

    fn group_from(&self, first: usize, layout: &Layout, funcs: &FuncMatcher) -> Group {
        let context = layout.context as usize;
        let join = 2 * context + layout.inter_hunk_context as usize;
        let mut last = first;
        while let Some(next) = self.changes.get(last + 1) {
            if next.old.start - self.changes[last].old.end > join {
                break;
            }
            last += 1;
        }
        let head = &self.changes[first];
        let mut old_start = head.old.start.saturating_sub(context);
        let mut new_start = head.new.start.saturating_sub(context);
        if layout.function_context
            && let Some(start) = self.function_start(head, funcs)
            && start < old_start
        {
            new_start = new_start.saturating_sub(old_start - start);
            old_start = start;
        }
        let (old_end, new_end, last) = self.group_end(last, layout, funcs);
        Group {
            changes: first..last + 1,
            old: old_start..old_end,
            new: new_start..new_end,
        }
    }

    fn group_end(
        &self,
        mut last: usize,
        layout: &Layout,
        funcs: &FuncMatcher,
    ) -> (usize, usize, usize) {
        let context = layout.context as usize;
        loop {
            let tail = &self.changes[last];
            let after = context
                .min(self.old.len() - tail.old.end)
                .min(self.new.len() - tail.new.end);
            let mut old_end = tail.old.end + after;
            let mut new_end = tail.new.end + after;
            if !layout.function_context {
                return (old_end, new_end, last);
            }
            let function_end = self.function_end(tail.old.end, funcs);
            if function_end > old_end {
                new_end = (new_end + (function_end - old_end)).min(self.new.len());
                old_end = function_end;
            }
            let Some(next) = self.changes.get(last + 1) else {
                return (old_end, new_end, last);
            };
            let probe = next.old.start.min(self.old.len().saturating_sub(1));
            let separated = probe.saturating_sub(context) > old_end
                && (old_end + 1..=probe).any(|index| funcs.is_function(self.old[index]));
            if separated {
                return (old_end, new_end, last);
            }
            last += 1;
        }
    }

    fn function_start(&self, head: &Change, funcs: &FuncMatcher) -> Option<usize> {
        let mut probe = head.old.start;
        if probe >= self.old.len() {
            let whole_function_added = self.new[head.new.start..]
                .iter()
                .any(|line| funcs.is_function(line));
            if whole_function_added {
                return None;
            }
            probe = self.old.len().checked_sub(1)?;
        }
        let mut start = funcs.find_backward(&self.old, probe).unwrap_or(0);
        while start > 0 && !is_blank(self.old[start - 1]) && !funcs.is_function(self.old[start - 1])
        {
            start -= 1;
        }
        Some(start)
    }

    fn function_end(&self, from: usize, funcs: &FuncMatcher) -> usize {
        let found = (from..self.old.len()).find(|&index| funcs.is_function(self.old[index]));
        match found {
            Some(mut end) => {
                while end > 0 && is_blank(self.old[end - 1]) {
                    end -= 1;
                }
                end
            }
            None => self.old.len(),
        }
    }

    fn emit(&self, path: &BString, index: usize, group: &Group, funcs: &FuncMatcher) -> Hunk {
        let mut lines = Vec::new();
        let mut new_pos = group.new.start;
        for change in &self.changes[group.changes.clone()] {
            while new_pos < change.new.start {
                push_line(&mut lines, LineKind::Context, self.new[new_pos]);
                new_pos += 1;
            }
            for line in &self.old[change.old.clone()] {
                push_line(&mut lines, LineKind::Removed, line);
            }
            for line in &self.new[change.new.clone()] {
                push_line(&mut lines, LineKind::Added, line);
            }
            new_pos = change.new.end;
        }
        while new_pos < group.new.end {
            push_line(&mut lines, LineKind::Context, self.new[new_pos]);
            new_pos += 1;
        }
        let header = group
            .old
            .start
            .checked_sub(1)
            .and_then(|start| funcs.find_backward(&self.old, start))
            .and_then(|found| funcs.function_text(self.old[found]))
            .filter(|text| !text.is_empty())
            .map(BString::from);
        Hunk {
            path: path.clone(),
            old_path: None,
            status: HunkStatus::Modified,
            number: u32::try_from(index + 1).unwrap_or(u32::MAX),
            old_start: header_start(group.old.start, group.old.len()),
            old_len: to_u32(group.old.len()),
            new_start: header_start(group.new.start, group.new.len()),
            new_len: to_u32(group.new.len()),
            header,
            lines,
            binary: false,
            old_mode: None,
            new_mode: None,
            moved: Vec::new(),
        }
    }
}

struct Group {
    changes: Range<usize>,
    old: Range<usize>,
    new: Range<usize>,
}

fn to_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

/// Git writes an empty range as the line before it and a non-empty range
/// from its first line, counting from 1.
fn header_start(start: usize, len: usize) -> u32 {
    if len == 0 {
        to_u32(start)
    } else {
        to_u32(start + 1)
    }
}

fn push_line(lines: &mut Vec<HunkLine>, kind: LineKind, line: &[u8]) {
    let body = line_body(line);
    lines.push(HunkLine {
        kind,
        text: BString::from(body),
    });
    if body.len() == line.len() {
        lines.push(HunkLine {
            kind: LineKind::NoNewline,
            text: BString::default(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::text_hunks;

    #[test]
    fn deleting_every_line_writes_an_empty_new_range() {
        let hunks = text_hunks("f", b"a\nb\n", b"");
        assert_eq!(hunks[0].header_line(), "@@ -1,2 +0,0 @@");
        assert_eq!(hunks[0].counts(), (0, 2));
    }

    #[test]
    fn adding_to_an_empty_file_writes_an_empty_old_range() {
        let hunks = text_hunks("f", b"", b"a\n");
        assert_eq!(hunks[0].header_line(), "@@ -0,0 +1 @@");
        assert_eq!(hunks[0].counts(), (1, 0));
    }

    #[test]
    fn a_missing_final_line_feed_adds_a_marker_row() {
        let with = text_hunks("f", b"a\nb\n", b"a\nc\n");
        let without = text_hunks("f", b"a\nb\n", b"a\nc");
        let last = without[0].lines.last().unwrap();
        assert_eq!(last.kind, LineKind::NoNewline);
        assert_eq!(last.render(), NO_NEWLINE);
        assert_ne!(with[0].fingerprint(), without[0].fingerprint());
        assert_eq!(without[0].counts(), (1, 1));
    }

    #[test]
    fn carriage_returns_stay_in_the_text_of_a_line() {
        let hunks = text_hunks("f", b"a\r\nb\r\n", b"a\r\nc\r\n");
        let added = hunks[0]
            .lines
            .iter()
            .find(|line| line.kind == LineKind::Added)
            .unwrap();
        assert_eq!(added.text, "c\r");
        assert!(
            hunks[0]
                .lines
                .iter()
                .all(|line| line.kind != LineKind::NoNewline)
        );
    }

    #[test]
    fn retains_only_changes_that_touch_the_given_new_lines() {
        let old = b"a\nb\nc\nd\ne\n";
        let new = b"A\nb\nc\nd\nE\n";
        let mut diff = TextDiff::new(old, new, &LineOptions::default());
        assert_eq!(diff.counts(), (2, 2));
        diff.retain_new_lines(&(4..5));
        assert_eq!(diff.counts(), (1, 1));
        assert!(!diff.is_empty());
        diff.retain_new_lines(&(1..3));
        assert!(diff.is_empty());
    }

    #[test]
    fn header_text_names_the_function_line_above_the_hunk() {
        let old: String = ["fn one() {\n"]
            .into_iter()
            .map(str::to_owned)
            .chain((1..=9).map(|number| format!("    line{number}\n")))
            .collect();
        let new = old.replace("line9", "other");
        let hunks = text_hunks("f", old.as_bytes(), new.as_bytes());
        assert_eq!(hunks[0].header_line(), "@@ -7,4 +7,4 @@ fn one() {");
    }
}
