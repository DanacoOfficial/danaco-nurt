// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
use super::ByteEdit;
use crate::model::Span;

/// Largest number of inserted and deleted lines for which the line
/// alignment is computed; beyond it the changed region becomes one edit.
const MAX_LINE_EDITS: usize = 1024;

/// Run of equal lines: start in the old lines, start in the new lines,
/// length.
type Run = (usize, usize, usize);

/// Byte edits that turn `original` into `new`.
///
/// Used by formats whose library rewrites the text instead of reporting
/// edits: the edits are then checked by the scope guard like any other.
/// Lines are aligned first (Myers' algorithm), and every changed group of
/// lines is narrowed to the bytes that differ, so a change of two separate
/// values gives two small edits.
///
/// ```
/// use nurt_doc::edit::{apply, diff};
///
/// let old = b"a = 1\nb = 2\nc = 3\n";
/// let new = b"a = 10\nb = 2\nc = 30\n";
/// let edits = diff(old, new);
/// assert_eq!(edits.len(), 2);
/// assert_eq!(apply(old, &edits)?.bytes, new);
/// # Ok::<(), nurt_contracts::error::Error>(())
/// ```
#[must_use]
pub fn diff(original: &[u8], new: &[u8]) -> Vec<ByteEdit> {
    let prefix = common_prefix(original, new);
    let suffix = common_suffix(&original[prefix..], &new[prefix..]);
    let old_mid = &original[prefix..original.len() - suffix];
    let new_mid = &new[prefix..new.len() - suffix];
    if old_mid.is_empty() && new_mid.is_empty() {
        return Vec::new();
    }
    let old_starts = line_starts(old_mid);
    let new_starts = line_starts(new_mid);
    let old_lines = split(old_mid, &old_starts);
    let new_lines = split(new_mid, &new_starts);
    let Some(runs) = align(&old_lines, &new_lines) else {
        return vec![ByteEdit::replace(
            Span::new(prefix, prefix + old_mid.len()),
            new_mid.to_vec(),
        )];
    };
    let mut edits = Vec::new();
    let (mut old_line, mut new_line) = (0, 0);
    let end = (old_lines.len(), new_lines.len(), 0);
    for (old_start, new_start, len) in runs.into_iter().chain(std::iter::once(end)) {
        if old_start > old_line || new_start > new_line {
            let old_span = byte_span(&old_starts, old_line, old_start, old_mid.len());
            let new_span = byte_span(&new_starts, new_line, new_start, new_mid.len());
            edits.push(narrowed(old_mid, new_mid, old_span, new_span, prefix));
        }
        old_line = old_start + len;
        new_line = new_start + len;
    }
    edits
}

fn common_prefix(left: &[u8], right: &[u8]) -> usize {
    left.iter().zip(right).take_while(|(a, b)| a == b).count()
}

fn common_suffix(left: &[u8], right: &[u8]) -> usize {
    left.iter()
        .rev()
        .zip(right.iter().rev())
        .take_while(|(a, b)| a == b)
        .count()
}

/// Start offsets of the lines of a non-empty `text`.
fn line_starts(text: &[u8]) -> Vec<usize> {
    let mut starts = vec![0];
    starts.extend(
        text.iter()
            .enumerate()
            .filter(|(_, byte)| **byte == b'\n')
            .map(|(offset, _)| offset + 1)
            .filter(|offset| *offset < text.len()),
    );
    if text.is_empty() {
        starts.clear();
    }
    starts
}

/// Lines of `text` with their terminators.
fn split<'t>(text: &'t [u8], starts: &[usize]) -> Vec<&'t [u8]> {
    starts
        .iter()
        .enumerate()
        .map(|(line, start)| {
            let end = starts.get(line + 1).copied().unwrap_or(text.len());
            &text[*start..end]
        })
        .collect()
}

/// Bytes of lines `from..to`.
fn byte_span(starts: &[usize], from: usize, to: usize, len: usize) -> Span {
    let at = |line: usize| starts.get(line).copied().unwrap_or(len);
    Span::new(at(from), at(to))
}

/// Edit replacing `old_span` of `old` by `new_span` of `new`, without the
/// bytes both share at the start and at the end.
fn narrowed(old: &[u8], new: &[u8], old_span: Span, new_span: Span, base: usize) -> ByteEdit {
    let old_part = &old[old_span.start..old_span.end];
    let new_part = &new[new_span.start..new_span.end];
    let head = common_prefix(old_part, new_part);
    let tail = common_suffix(&old_part[head..], &new_part[head..]);
    ByteEdit::replace(
        Span::new(base + old_span.start + head, base + old_span.end - tail),
        new_part[head..new_part.len() - tail].to_vec(),
    )
}

/// Index of diagonal `k` in the row of edit distance `d`.
fn slot(k: isize, d: usize) -> usize {
    usize::try_from(k + isize::try_from(d).unwrap_or(isize::MAX)).unwrap_or(0)
}

fn signed(value: usize) -> isize {
    isize::try_from(value).unwrap_or(isize::MAX)
}

/// Whether the path to diagonal `k` at distance `d` comes from diagonal
/// `k + 1` (an inserted line) rather than `k - 1` (a deleted line).
fn from_above(previous: &[usize], k: isize, d: usize) -> bool {
    let d_signed = signed(d);
    k == -d_signed
        || (k != d_signed && previous[slot(k - 1, d - 1)] < previous[slot(k + 1, d - 1)])
}

/// Runs of equal lines on a shortest edit path (Myers), `None` when more
/// than [`MAX_LINE_EDITS`] lines differ.
fn align(old: &[&[u8]], new: &[&[u8]]) -> Option<Vec<Run>> {
    let (n, m) = (old.len(), new.len());
    let follow = |mut x: usize, mut y: usize| {
        while x < n && y < m && old[x] == new[y] {
            x += 1;
            y += 1;
        }
        (x, y)
    };
    let mut trace: Vec<Vec<usize>> = Vec::new();
    for d in 0..=MAX_LINE_EDITS.min(n + m) {
        let mut row = vec![0; 2 * d + 1];
        for step in 0..=d {
            let k = signed(2 * step) - signed(d);
            let x = match trace.last() {
                None => 0,
                Some(previous) if from_above(previous, k, d) => previous[slot(k + 1, d - 1)],
                Some(previous) => previous[slot(k - 1, d - 1)] + 1,
            };
            let y = usize::try_from(signed(x) - k).unwrap_or(usize::MAX);
            let (x, y) = if x <= n && y <= m { follow(x, y) } else { (x, y) };
            row[slot(k, d)] = x;
            if x == n && y == m {
                trace.push(row);
                return Some(backtrack(&trace, n, m));
            }
        }
        trace.push(row);
    }
    None
}

fn backtrack(trace: &[Vec<usize>], n: usize, m: usize) -> Vec<Run> {
    let mut runs = Vec::new();
    let (mut x, mut y) = (n, m);
    for d in (1..trace.len()).rev() {
        let k = signed(x) - signed(y);
        let previous = &trace[d - 1];
        let above = from_above(previous, k, d);
        let previous_k = if above { k + 1 } else { k - 1 };
        let previous_x = previous[slot(previous_k, d - 1)];
        let previous_y = usize::try_from(signed(previous_x) - previous_k).unwrap_or(0);
        let (mid_x, mid_y) = if above {
            (previous_x, previous_y + 1)
        } else {
            (previous_x + 1, previous_y)
        };
        if x > mid_x {
            runs.push((mid_x, mid_y, x - mid_x));
        }
        x = previous_x;
        y = previous_y;
    }
    if x > 0 {
        runs.push((0, 0, x));
    }
    runs.reverse();
    runs
}
