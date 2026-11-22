// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Selection of hunks and lines by identifier and construction of the resulting blob.
//!
//! A hunk identifier has the form `path#number[:fingerprint][@lines]`, where
//! `number` counts the hunks of the file from 1 in the canonical difference
//! (`-U3`, no rename detection), `fingerprint` is the first six hexadecimal
//! digits of the SHA-256 of the hunk lines (prefixes `+`, `-`, space and `\`,
//! without the `@@` header) joined by line feeds, and `lines` is a list of
//! positions or ranges (`3-5,8`) counted from 1 within the hunk body. The line
//! `\ No newline at end of file` takes a position and belongs to the line
//! before it.
//!
//! [`apply_selection`] builds the new content of one side from the other
//! side and the selected changes, as `git apply --cached` does with a patch
//! that keeps only the selected lines: an unselected removed line stays as
//! context and an unselected added line is dropped. [`Direction::Reverse`]
//! undoes the selected changes instead.

use std::collections::BTreeMap;
use std::ops::RangeInclusive;

use nurt_contracts::error::{Error, ErrorCode, Result};
use sha2::Digest;

const FINGERPRINT_LEN: usize = 6;
const MAX_LINE_POSITION: usize = 5000;
const NO_NEWLINE: &[u8] = b"\\ No newline at end of file";

/// Kind of a line of a hunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    /// Unchanged line.
    Context,
    /// Line present only on the old side.
    Removed,
    /// Line present only on the new side.
    Added,
}

/// One line of a hunk; `text` ends with a line feed unless the line is the
/// last line of its side without one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffLine {
    /// Kind.
    pub kind: LineKind,
    /// Content with its line terminator.
    pub text: Vec<u8>,
}

/// Hunk of a unified difference of one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffHunk {
    /// First old line, from 1 (0 for an empty old side).
    pub old_start: usize,
    /// Lines, in order.
    pub lines: Vec<DiffLine>,
}

impl DiffHunk {
    /// Lines of the hunk body as printed in a patch, each with its prefix
    /// and without a line terminator, including `\ No newline at end of
    /// file` markers.
    #[must_use]
    pub fn body(&self) -> Vec<Vec<u8>> {
        let mut out = Vec::with_capacity(self.lines.len());
        for line in &self.lines {
            let prefix = match line.kind {
                LineKind::Context => b' ',
                LineKind::Removed => b'-',
                LineKind::Added => b'+',
            };
            let content = line.text.strip_suffix(b"\n").unwrap_or(&line.text);
            let mut printed = Vec::with_capacity(content.len() + 1);
            printed.push(prefix);
            printed.extend_from_slice(content);
            out.push(printed);
            if !line.text.ends_with(b"\n") {
                out.push(NO_NEWLINE.to_vec());
            }
        }
        out
    }

    /// Fingerprint of the hunk: six hexadecimal digits of SHA-256 of the
    /// body lines joined by line feeds.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_git::hunk_select::{DiffHunk, DiffLine, LineKind};
    ///
    /// let hunk = DiffHunk {
    ///     old_start: 1,
    ///     lines: vec![DiffLine { kind: LineKind::Added, text: b"x\n".to_vec() }],
    /// };
    /// assert_eq!(hunk.fingerprint().len(), 6);
    /// ```
    #[must_use]
    pub fn fingerprint(&self) -> String {
        let joined = self.body().join(&b'\n');
        let digest = sha2::Sha256::digest(&joined);
        hex::encode(digest)[..FINGERPRINT_LEN].to_owned()
    }

    /// Hunk line index of every body position, from 1.
    fn positions(&self) -> Vec<usize> {
        let mut out = Vec::new();
        for (index, line) in self.lines.iter().enumerate() {
            out.push(index);
            if !line.text.ends_with(b"\n") {
                out.push(index);
            }
        }
        out
    }
}

/// Parsed hunk identifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HunkId {
    /// Path relative to the root of the working tree, `/`-separated.
    pub path: String,
    /// Hunk number, from 1.
    pub number: usize,
    /// Expected fingerprint.
    pub fingerprint: Option<String>,
    /// Selected positions; `None` selects the whole hunk.
    pub lines: Option<Vec<RangeInclusive<usize>>>,
}

impl HunkId {
    /// Parses `path#number[:fingerprint][@lines]`.
    ///
    /// # Errors
    ///
    /// `E_VALUE` with problem `invalid_hunk_id` for a malformed identifier
    /// or a position above 5000.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_git::hunk_select::HunkId;
    ///
    /// let id = HunkId::parse("src/a.rs#2:0a1b2c@3-5,8").unwrap();
    /// assert_eq!(id.path, "src/a.rs");
    /// assert_eq!(id.number, 2);
    /// assert_eq!(id.fingerprint.as_deref(), Some("0a1b2c"));
    /// assert_eq!(id.lines, Some(vec![3..=5, 8..=8]));
    /// ```
    pub fn parse(text: &str) -> Result<Self> {
        let invalid = |reason: &str| {
            Error::new(
                ErrorCode::Value,
                format!("{text} is not a hunk identifier: {reason}; use path#number or path#number@3-5,8"),
            )
            .with_problem("invalid_hunk_id")
            .with_found(text.to_owned())
        };
        let (path, rest) = text.rsplit_once('#').ok_or_else(|| invalid("missing #"))?;
        if path.is_empty() {
            return Err(invalid("missing path"));
        }
        let (head, lines) = match rest.split_once('@') {
            Some((head, lines)) => (head, Some(lines)),
            None => (rest, None),
        };
        let (number, fingerprint) = match head.split_once(':') {
            Some((number, fingerprint)) => (number, Some(fingerprint)),
            None => (head, None),
        };
        let number: usize = number.parse().map_err(|_| invalid("the hunk number is not a positive integer"))?;
        if number == 0 {
            return Err(invalid("hunks are numbered from 1"));
        }
        let fingerprint = match fingerprint {
            Some(value) if value.len() == FINGERPRINT_LEN && value.bytes().all(|b| b.is_ascii_hexdigit()) => {
                Some(value.to_ascii_lowercase())
            }
            Some(_) => return Err(invalid("the fingerprint must have six hexadecimal digits")),
            None => None,
        };
        let lines = lines.map(|spec| parse_ranges(spec).ok_or_else(|| invalid("malformed line list"))).transpose()?;
        Ok(Self {
            path: path.to_owned(),
            number,
            fingerprint,
            lines,
        })
    }
}

fn parse_ranges(spec: &str) -> Option<Vec<RangeInclusive<usize>>> {
    let mut out = Vec::new();
    for part in spec.split(',') {
        let (start, end) = match part.split_once('-') {
            Some((start, end)) => (start.parse().ok()?, end.parse().ok()?),
            None => {
                let single: usize = part.parse().ok()?;
                (single, single)
            }
        };
        if start == 0 || end < start || end > MAX_LINE_POSITION {
            return None;
        }
        out.push(start..=end);
    }
    Some(out)
}

/// Whether selected changes are applied or undone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Build the new side from the old side (`git.stage add`).
    Forward,
    /// Build the old side from the new side (`git.stage unstage`).
    Reverse,
}

/// Selection of whole hunks or of lines within hunks of one file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Selection {
    /// Selected hunk lines by hunk index from 0; `None` selects the whole
    /// hunk.
    hunks: BTreeMap<usize, Option<Vec<usize>>>,
}

impl Selection {
    /// Resolves identifiers of one file against its hunks.
    ///
    /// # Errors
    ///
    /// `E_GIT_REF` with problem `hunk_not_found` for a number beyond the last
    /// hunk, `hunk_changed` when a fingerprint does not match, and
    /// `line_out_of_range` for a position beyond the hunk body.
    pub fn resolve(ids: &[HunkId], hunks: &[DiffHunk]) -> Result<Self> {
        let mut selection = Self::default();
        for id in ids {
            let hunk = hunks.get(id.number - 1).ok_or_else(|| {
                hunk_error(
                    "hunk_not_found",
                    id,
                    format!("{}#{} does not exist; the file has {} hunks, list them again", id.path, id.number, hunks.len()),
                )
            })?;
            if let Some(expected) = &id.fingerprint
                && *expected != hunk.fingerprint()
            {
                return Err(hunk_error(
                    "hunk_changed",
                    id,
                    format!("{}#{} changed since it was listed; list the hunks again", id.path, id.number),
                ));
            }
            selection.add(id, hunk)?;
        }
        Ok(selection)
    }

    fn add(&mut self, id: &HunkId, hunk: &DiffHunk) -> Result<()> {
        let index = id.number - 1;
        let Some(ranges) = &id.lines else {
            self.hunks.insert(index, None);
            return Ok(());
        };
        let positions = hunk.positions();
        let mut chosen = Vec::new();
        for range in ranges {
            for position in range.clone() {
                let line = positions.get(position - 1).ok_or_else(|| {
                    hunk_error(
                        "line_out_of_range",
                        id,
                        format!(
                            "line {position} is outside {}#{}, which has {} lines",
                            id.path,
                            id.number,
                            positions.len()
                        ),
                    )
                })?;
                chosen.push(*line);
            }
        }
        match self.hunks.entry(index).or_insert_with(|| Some(Vec::new())) {
            Some(lines) => lines.extend(chosen),
            None => {}
        }
        Ok(())
    }

    /// Whether nothing is selected.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.hunks.is_empty()
    }

    fn is_selected(&self, hunk: usize, line: usize) -> bool {
        match self.hunks.get(&hunk) {
            None => false,
            Some(None) => true,
            Some(Some(lines)) => lines.contains(&line),
        }
    }
}

fn hunk_error(problem: &str, id: &HunkId, message: String) -> Error {
    Error::new(ErrorCode::GitRef, message)
        .with_problem(problem)
        .with_found(format!("{}#{}", id.path, id.number))
}

/// Builds the content of one side from `base` and the selected changes.
///
/// With [`Direction::Forward`], `base` is the old side of `hunks` and the
/// result contains the selected changes; with [`Direction::Reverse`],
/// `base` is the new side and the result lacks the selected changes.
///
/// # Errors
///
/// `E_GIT_REF` with problem `hunk_changed` when `base` does not contain the
/// lines the hunks expect.
///
/// # Examples
///
/// ```
/// use nurt_git::hunk_select::{apply_selection, DiffHunk, DiffLine, Direction, HunkId, LineKind, Selection};
///
/// let hunk = DiffHunk {
///     old_start: 1,
///     lines: vec![
///         DiffLine { kind: LineKind::Removed, text: b"a\n".to_vec() },
///         DiffLine { kind: LineKind::Added, text: b"b\n".to_vec() },
///         DiffLine { kind: LineKind::Added, text: b"c\n".to_vec() },
///     ],
/// };
/// let ids = [HunkId::parse("f#1@2").unwrap()];
/// let selection = Selection::resolve(&ids, std::slice::from_ref(&hunk)).unwrap();
/// let out = apply_selection(b"a\n", &[hunk], &selection, Direction::Forward).unwrap();
/// assert_eq!(out, b"a\nb\n");
/// ```
pub fn apply_selection(
    base: &[u8],
    hunks: &[DiffHunk],
    selection: &Selection,
    direction: Direction,
) -> Result<Vec<u8>> {
    let base_lines = split_lines(base);
    let mut out = Vec::with_capacity(base.len());
    let mut next = 0usize;
    let mut offset: isize = 0;
    for (index, hunk) in hunks.iter().enumerate() {
        let start = base_start(hunk, direction, offset);
        copy_lines(&base_lines, next, start, &mut out)?;
        next = start;
        for (position, line) in hunk.lines.iter().enumerate() {
            let selected = selection.is_selected(index, position);
            let (consumes, emits) = line_effect(line.kind, selected, direction);
            if consumes {
                let found = base_lines.get(next).ok_or_else(|| mismatch(index))?;
                if *found != line.text.as_slice() && trim_eol(found) != trim_eol(&line.text) {
                    return Err(mismatch(index));
                }
                next += 1;
            }
            if emits {
                out.extend_from_slice(&line.text);
                if !line.text.ends_with(b"\n") && position + 1 < hunk.lines.len() {
                    out.push(b'\n');
                }
            }
        }
        offset += side_delta(hunk, direction);
    }
    copy_lines(&base_lines, next, base_lines.len(), &mut out)?;
    Ok(out)
}

/// Whether a line is read from the base and whether it is written out.
fn line_effect(kind: LineKind, selected: bool, direction: Direction) -> (bool, bool) {
    match (kind, direction) {
        (LineKind::Context, _) => (true, true),
        (LineKind::Removed, Direction::Forward) => (true, !selected),
        (LineKind::Added, Direction::Forward) => (false, selected),
        (LineKind::Added, Direction::Reverse) => (true, !selected),
        (LineKind::Removed, Direction::Reverse) => (false, selected),
    }
}

/// Index of the first base line of `hunk`; on the reverse side the start is
/// shifted by the line count differences of the earlier hunks.
fn base_start(hunk: &DiffHunk, direction: Direction, offset: isize) -> usize {
    let old_start = hunk.old_start.saturating_sub(usize::from(hunk.old_start > 0));
    let has_old = hunk.lines.iter().any(|line| line.kind != LineKind::Added);
    let old_index = if has_old { old_start } else { hunk.old_start };
    match direction {
        Direction::Forward => old_index,
        Direction::Reverse => old_index.saturating_add_signed(offset),
    }
}

/// Lines added minus lines removed by `hunk`, as seen from the new side.
fn side_delta(hunk: &DiffHunk, direction: Direction) -> isize {
    if direction == Direction::Forward {
        return 0;
    }
    hunk.lines
        .iter()
        .map(|line| match line.kind {
            LineKind::Added => 1,
            LineKind::Removed => -1,
            LineKind::Context => 0,
        })
        .sum()
}

fn copy_lines(lines: &[&[u8]], from: usize, to: usize, out: &mut Vec<u8>) -> Result<()> {
    let slice = lines.get(from..to).ok_or_else(|| mismatch(0))?;
    for line in slice {
        out.extend_from_slice(line);
    }
    Ok(())
}

fn trim_eol(line: &[u8]) -> &[u8] {
    line.strip_suffix(b"\n").unwrap_or(line)
}

fn mismatch(index: usize) -> Error {
    Error::new(
        ErrorCode::GitRef,
        format!("hunk {} does not match the current content; list the hunks again", index + 1),
    )
    .with_problem("hunk_changed")
}

/// Lines of `content`, each with its line feed.
fn split_lines(content: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    let mut start = 0;
    for (index, byte) in content.iter().enumerate() {
        if *byte == b'\n' {
            out.push(&content[start..=index]);
            start = index + 1;
        }
    }
    if start < content.len() {
        out.push(&content[start..]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(kind: LineKind, text: &str) -> DiffLine {
        DiffLine {
            kind,
            text: text.as_bytes().to_vec(),
        }
    }

    use LineKind::{Added as A, Context as C, Removed as R};

    /// Old: a b c d e f g h i j; new: a B c d e f g h I j k (two hunks with
    /// context 1 for brevity).
    fn two_hunks() -> (Vec<u8>, Vec<u8>, Vec<DiffHunk>) {
        let old = b"a\nb\nc\nd\ne\nf\ng\nh\ni\nj\n".to_vec();
        let new = b"a\nB\nc\nd\ne\nf\ng\nh\nI\nj\nk\n".to_vec();
        let hunks = vec![
            DiffHunk {
                old_start: 1,
                lines: vec![line(C, "a\n"), line(R, "b\n"), line(A, "B\n"), line(C, "c\n")],
            },
            DiffHunk {
                old_start: 8,
                lines: vec![
                    line(C, "h\n"),
                    line(R, "i\n"),
                    line(A, "I\n"),
                    line(C, "j\n"),
                    line(A, "k\n"),
                ],
            },
        ];
        (old, new, hunks)
    }

    fn select(ids: &[&str], hunks: &[DiffHunk]) -> Selection {
        let ids: Vec<HunkId> = ids.iter().map(|id| HunkId::parse(id).unwrap()).collect();
        Selection::resolve(&ids, hunks).unwrap()
    }

    #[test]
    fn whole_hunks_forward_and_reverse() {
        let (old, new, hunks) = two_hunks();
        let all = select(&["f#1", "f#2"], &hunks);
        assert_eq!(apply_selection(&old, &hunks, &all, Direction::Forward).unwrap(), new);
        assert_eq!(apply_selection(&new, &hunks, &all, Direction::Reverse).unwrap(), old);
        let second = select(&["f#2"], &hunks);
        assert_eq!(
            apply_selection(&old, &hunks, &second, Direction::Forward).unwrap(),
            b"a\nb\nc\nd\ne\nf\ng\nh\nI\nj\nk\n"
        );
        assert_eq!(
            apply_selection(&new, &hunks, &second, Direction::Reverse).unwrap(),
            b"a\nB\nc\nd\ne\nf\ng\nh\ni\nj\n"
        );
    }

    #[test]
    fn unselected_removed_lines_stay_and_unselected_added_lines_are_dropped() {
        let (old, _, hunks) = two_hunks();
        let only_add = select(&["f#1@3"], &hunks);
        assert_eq!(
            apply_selection(&old, &hunks, &only_add, Direction::Forward).unwrap(),
            b"a\nb\nB\nc\nd\ne\nf\ng\nh\ni\nj\n"
        );
        let only_remove = select(&["f#1@2"], &hunks);
        assert_eq!(
            apply_selection(&old, &hunks, &only_remove, Direction::Forward).unwrap(),
            b"a\nc\nd\ne\nf\ng\nh\ni\nj\n"
        );
        let ranges = select(&["f#2@2-3,5"], &hunks);
        assert_eq!(
            apply_selection(&old, &hunks, &ranges, Direction::Forward).unwrap(),
            b"a\nb\nc\nd\ne\nf\ng\nh\nI\nj\nk\n"
        );
    }

    #[test]
    fn missing_final_newline_is_kept_and_marker_selects_the_line() {
        let old = b"x\ny".to_vec();
        let hunk = DiffHunk {
            old_start: 1,
            lines: vec![line(C, "x\n"), line(R, "y"), line(A, "y\n"), line(A, "z")],
        };
        let body = hunk.body();
        assert_eq!(body[2], NO_NEWLINE);
        assert_eq!(body.len(), 6);
        let hunks = vec![hunk];
        let all = select(&["f#1"], &hunks);
        assert_eq!(apply_selection(&old, &hunks, &all, Direction::Forward).unwrap(), b"x\ny\nz");
        let marker = select(&["f#1@3"], &hunks);
        assert_eq!(apply_selection(&old, &hunks, &marker, Direction::Forward).unwrap(), b"x\n");
    }

    #[test]
    fn new_file_can_be_added_partially() {
        let hunk = DiffHunk {
            old_start: 0,
            lines: vec![line(A, "one\n"), line(A, "two\n"), line(A, "three\n")],
        };
        let hunks = vec![hunk];
        let some = select(&["new.txt#1@1,3"], &hunks);
        assert_eq!(apply_selection(b"", &hunks, &some, Direction::Forward).unwrap(), b"one\nthree\n");
    }

    #[test]
    fn crlf_lines_are_kept_byte_for_byte() {
        let old = b"a\r\nb\r\n".to_vec();
        let hunk = DiffHunk {
            old_start: 1,
            lines: vec![line(C, "a\r\n"), line(R, "b\r\n"), line(A, "c\r\n")],
        };
        let hunks = vec![hunk];
        let all = select(&["f#1"], &hunks);
        assert_eq!(apply_selection(&old, &hunks, &all, Direction::Forward).unwrap(), b"a\r\nc\r\n");
    }

    #[test]
    fn identifiers_are_parsed_and_validated() {
        let id = HunkId::parse("dir/a#b.txt#3").unwrap();
        assert_eq!((id.path.as_str(), id.number), ("dir/a#b.txt", 3));
        for bad in ["a", "#1", "a#0", "a#x", "a#1:zz", "a#1@", "a#1@0", "a#1@5-3", "a#1@5001"] {
            let error = HunkId::parse(bad).unwrap_err();
            assert_eq!(error.problem.as_deref(), Some("invalid_hunk_id"), "{bad}");
        }
    }

    #[test]
    fn selection_errors_name_the_problem() {
        let (_, _, hunks) = two_hunks();
        let resolve = |id: &str| Selection::resolve(&[HunkId::parse(id).unwrap()], &hunks).unwrap_err();
        assert_eq!(resolve("f#3").problem.as_deref(), Some("hunk_not_found"));
        assert_eq!(resolve("f#1:000000").problem.as_deref(), Some("hunk_changed"));
        assert_eq!(resolve("f#1@9").problem.as_deref(), Some("line_out_of_range"));
        let fingerprint = hunks[0].fingerprint();
        let ok = Selection::resolve(&[HunkId::parse(&format!("f#1:{fingerprint}")).unwrap()], &hunks).unwrap();
        assert!(!ok.is_empty());
    }

    #[test]
    fn fingerprint_is_sha256_of_the_printed_body() {
        let hunk = DiffHunk {
            old_start: 1,
            lines: vec![line(C, "a\n"), line(A, "b")],
        };
        let expected = hex::encode(sha2::Sha256::digest(b" a\n+b\n\\ No newline at end of file"));
        assert_eq!(hunk.fingerprint(), expected[..6]);
    }

    #[test]
    fn content_that_does_not_match_is_rejected() {
        let (_, _, hunks) = two_hunks();
        let all = select(&["f#1"], &hunks);
        let error = apply_selection(b"q\nq\nq\n", &hunks, &all, Direction::Forward).unwrap_err();
        assert_eq!(error.problem.as_deref(), Some("hunk_changed"));
    }
}
