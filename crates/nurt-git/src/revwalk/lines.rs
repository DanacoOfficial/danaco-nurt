// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Differences of two file contents by lines: changed blocks, line counts,
//! hunks of the unified format and the movement of a line range from a
//! file to its previous version.

use std::ops::Range;

use bstr::ByteSlice;
use gix_imara_diff::{Algorithm, Diff, InternedInput};

/// Bytes inspected for a NUL byte to recognise binary content, as `git`
/// does.
const BINARY_PROBE: usize = 8000;

/// Largest length of the section heading after `@@`.
const HEADING_MAX: usize = 80;

/// Marker line after a last line without a line break.
pub(crate) const NO_NEWLINE: &str = "\\ No newline at end of file";

/// Whether `data` is treated as binary.
pub(crate) fn is_binary(data: &[u8]) -> bool {
    data[..data.len().min(BINARY_PROBE)].contains(&0)
}

/// Lines of `data`, each with its line break.
pub(crate) fn split_lines(data: &[u8]) -> Vec<&[u8]> {
    data.lines_with_terminator().collect()
}

/// Text of a line without its line break.
pub(crate) fn line_text(line: &[u8]) -> String {
    let line = line.strip_suffix(b"\n").unwrap_or(line);
    let line = line.strip_suffix(b"\r").unwrap_or(line);
    line.to_str_lossy().into_owned()
}

/// A block of changed lines: `before` lines of the old content replaced by
/// `after` lines of the new content, both counted from 0.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Edit {
    /// Removed lines of the old content.
    pub(crate) before: Range<u32>,
    /// Added lines of the new content.
    pub(crate) after: Range<u32>,
}

/// Changed blocks between two contents, computed with the Myers algorithm
/// and the indentation heuristic for the placement of ambiguous blocks.
pub(crate) fn edits(old: &[u8], new: &[u8]) -> Vec<Edit> {
    let input = InternedInput::new(old, new);
    let mut diff = Diff::compute(Algorithm::Myers, &input);
    diff.postprocess_lines(&input);
    diff.hunks()
        .map(|hunk| Edit {
            before: hunk.before,
            after: hunk.after,
        })
        .collect()
}

/// Lines added and removed from `old` to `new`.
pub(crate) fn line_counts(old: &[u8], new: &[u8]) -> (u64, u64) {
    edits(old, new).iter().fold((0, 0), |(added, removed), edit| {
        (
            added + u64::from(edit.after.end - edit.after.start),
            removed + u64::from(edit.before.end - edit.before.start),
        )
    })
}

/// One hunk of a unified difference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UnifiedHunk {
    /// First old line as written after `@@ -`: from 1, or the line before
    /// the hunk when it has no old lines.
    pub(crate) old_start: u64,
    /// Number of old lines.
    pub(crate) old_len: u64,
    /// First new line, written as `old_start`.
    pub(crate) new_start: u64,
    /// Number of new lines.
    pub(crate) new_len: u64,
    /// Lines with the prefixes `-`, `+` and space, and the marker of a
    /// missing final line break.
    pub(crate) lines: Vec<String>,
    /// Section heading: the nearest line before the hunk that starts with a
    /// letter, `_` or `$`.
    pub(crate) header: Option<String>,
}

/// Hunks of the unified difference from `old` to `new` with `context`
/// unchanged lines around each change; changes separated by at most twice
/// the context share a hunk.
pub(crate) fn unified(old: &[u8], new: &[u8], context: u32) -> Vec<UnifiedHunk> {
    let old_lines = split_lines(old);
    let new_lines = split_lines(new);
    let all = edits(old, new);
    let mut hunks = Vec::new();
    let mut start = 0;
    while start < all.len() {
        let mut end = start + 1;
        while end < all.len() && all[end].before.start - all[end - 1].before.end <= context.saturating_mul(2) {
            end += 1;
        }
        hunks.push(build_hunk(&old_lines, &new_lines, &all[start..end], context));
        start = end;
    }
    hunks
}

fn build_hunk(old: &[&[u8]], new: &[&[u8]], group: &[Edit], context: u32) -> UnifiedHunk {
    let (Some(first), Some(last)) = (group.first(), group.last()) else {
        return UnifiedHunk {
            old_start: 0,
            old_len: 0,
            new_start: 0,
            new_len: 0,
            lines: Vec::new(),
            header: None,
        };
    };
    let lead = context.min(first.before.start);
    let old_total = len32(old.len());
    let trail = context.min(old_total - last.before.end);
    let old_begin = first.before.start - lead;
    let new_begin = first.after.start - lead;
    let old_end = last.before.end + trail;
    let new_end = last.after.end + trail;
    let mut lines = Vec::new();
    let mut position = old_begin;
    for edit in group {
        push_context(&mut lines, old, position..edit.before.start);
        push_lines(&mut lines, '-', old, edit.before.clone());
        push_lines(&mut lines, '+', new, edit.after.clone());
        position = edit.before.end;
    }
    push_context(&mut lines, old, position..old_end);
    UnifiedHunk {
        old_start: header_start(old_begin, old_end),
        old_len: u64::from(old_end - old_begin),
        new_start: header_start(new_begin, new_end),
        new_len: u64::from(new_end - new_begin),
        lines,
        header: section_heading(old, old_begin),
    }
}

fn header_start(begin: u32, end: u32) -> u64 {
    if end > begin { u64::from(begin) + 1 } else { u64::from(begin) }
}

fn push_context(out: &mut Vec<String>, lines: &[&[u8]], range: Range<u32>) {
    push_lines(out, ' ', lines, range);
}

fn push_lines(out: &mut Vec<String>, prefix: char, lines: &[&[u8]], range: Range<u32>) {
    for index in range {
        let Some(line) = lines.get(index as usize) else {
            continue;
        };
        let body = line.strip_suffix(b"\n").unwrap_or(line);
        let mut text = String::with_capacity(body.len() + 1);
        text.push(prefix);
        text.push_str(&body.to_str_lossy());
        out.push(text);
        if !line.ends_with(b"\n") {
            out.push(NO_NEWLINE.to_owned());
        }
    }
}

/// The nearest line before `before` that starts with a letter, `_` or `$`,
/// cut to the length `git` writes and without trailing white space.
pub(crate) fn section_heading(lines: &[&[u8]], before: u32) -> Option<String> {
    let before = (before as usize).min(lines.len());
    lines[..before].iter().rev().find_map(|line| {
        let first = *line.first()?;
        if !(first.is_ascii_alphabetic() || first == b'_' || first == b'$') {
            return None;
        }
        let cut = &line[..line.len().min(HEADING_MAX)];
        Some(cut.trim_end().to_str_lossy().into_owned())
    })
}

/// The range of the old content that a range of the new content comes from,
/// and whether a change touches the range.
///
/// A range end inside a changed block extends to the whole old side of the
/// block; lines removed strictly inside the range touch it.
pub(crate) fn map_to_old(edits: &[Edit], range: Range<u32>) -> (Range<u32>, bool) {
    let touched = edits.iter().any(|edit| {
        if edit.after.is_empty() {
            range.start < edit.after.start && edit.after.start < range.end
        } else {
            edit.after.start < range.end && range.start < edit.after.end
        }
    });
    if range.is_empty() {
        return (0..0, touched);
    }
    let start = map_line(edits, range.start, true);
    let end = map_line(edits, range.end - 1, false);
    (start..end.max(start), touched)
}

fn map_line(edits: &[Edit], line: u32, start: bool) -> u32 {
    let mut shift: i64 = 0;
    for edit in edits {
        if edit.after.start <= line && line < edit.after.end {
            return if start { edit.before.start } else { edit.before.end };
        }
        if edit.after.end <= line {
            shift += i64::from(edit.before.end - edit.before.start) - i64::from(edit.after.end - edit.after.start);
        } else {
            break;
        }
    }
    let mapped = (i64::from(line) + shift).max(0);
    let mapped = u32::try_from(mapped).unwrap_or(u32::MAX);
    if start { mapped } else { mapped.saturating_add(1) }
}

fn len32(len: usize) -> u32 {
    u32::try_from(len).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_binary_content_by_nul_bytes() {
        assert!(is_binary(b"a\0b"));
        assert!(!is_binary(b"plain text\n"));
        let mut late = vec![b'a'; BINARY_PROBE];
        late.push(0);
        assert!(!is_binary(&late));
    }

    #[test]
    fn strips_line_breaks_of_both_styles() {
        assert_eq!(line_text(b"text\r\n"), "text");
        assert_eq!(line_text(b"text\n"), "text");
        assert_eq!(line_text(b"text"), "text");
        assert_eq!(split_lines(b"a\r\nb\nc"), vec![&b"a\r\n"[..], b"b\n", b"c"]);
    }

    #[test]
    fn counts_added_and_removed_lines() {
        assert_eq!(line_counts(b"a\nb\nc\n", b"a\nB\nc\nd\n"), (2, 1));
        assert_eq!(line_counts(b"", b"x\ny\n"), (2, 0));
        assert_eq!(line_counts(b"a", b"a\n"), (1, 1));
    }

    #[test]
    fn builds_hunks_with_context_and_headings() {
        let old = b"fn one() {\n  1\n  2\n  3\n  4\n  5\n  6\n  7\n}\n";
        let new = b"fn one() {\n  1\n  2\n  3\n  4\n  X\n  6\n  7\n}\n";
        let hunks = unified(old, new, 3);
        assert_eq!(hunks.len(), 1);
        let hunk = &hunks[0];
        assert_eq!((hunk.old_start, hunk.old_len, hunk.new_start, hunk.new_len), (3, 7, 3, 7));
        assert_eq!(hunk.lines, ["   2", "   3", "   4", "-  5", "+  X", "   6", "   7", " }"]);
        assert_eq!(hunk.header.as_deref(), Some("fn one() {"));
    }

    #[test]
    fn separates_distant_changes_and_joins_close_ones() {
        let old: Vec<u8> = (1..=20).flat_map(|n| format!("{n}\n").into_bytes()).collect();
        let mut new = old.clone();
        new[0] = b'X';
        let changed = String::from_utf8(new.clone()).unwrap().replace("\n19\n", "\nY\n");
        assert_eq!(unified(&old, changed.as_bytes(), 3).len(), 2);
        assert_eq!(unified(&old, changed.as_bytes(), 10).len(), 1);
        let zero = unified(&old, changed.as_bytes(), 0);
        assert_eq!(zero[0].lines, ["-1", "+X"]);
    }

    #[test]
    fn marks_missing_final_line_break_and_new_files() {
        let hunks = unified(b"", b"a\nb", 3);
        assert_eq!((hunks[0].old_start, hunks[0].old_len, hunks[0].new_start), (0, 0, 1));
        assert_eq!(hunks[0].lines, ["+a", "+b", NO_NEWLINE]);
        assert_eq!(hunks[0].header, None);
        let removed = unified(b"a\nb\n", b"", 3);
        assert_eq!((removed[0].new_start, removed[0].new_len), (0, 0));
        let context = unified(b"x\nend", b"y\nend", 3);
        assert_eq!(context[0].lines, ["-x", "+y", " end", NO_NEWLINE]);
    }

    #[test]
    fn heading_is_cut_and_trimmed() {
        let long = format!("{}  \n", "a".repeat(100));
        let lines: Vec<&[u8]> = vec![long.as_bytes(), b" body\n"];
        assert_eq!(section_heading(&lines, 2).unwrap().len(), HEADING_MAX);
        let short: Vec<&[u8]> = vec![b"$var  \n", b"\tbody\n"];
        assert_eq!(section_heading(&short, 2).as_deref(), Some("$var"));
        assert_eq!(section_heading(&short, 0), None);
    }

    #[test]
    fn maps_ranges_to_the_old_content() {
        let old = b"a\nb\nc\nd\ne\n";
        let new = b"a\nNEW\nb\nc\nD\ne\n";
        let changes = edits(old, new);
        assert_eq!(map_to_old(&changes, 2..4), (1..3, false));
        assert_eq!(map_to_old(&changes, 4..5), (3..4, true));
        assert_eq!(map_to_old(&changes, 1..2), (1..1, true));
        assert_eq!(map_to_old(&changes, 5..6), (4..5, false));
        let removal = edits(b"a\nb\nc\n", b"a\nc\n");
        assert_eq!(map_to_old(&removal, 0..2), (0..3, true));
        assert_eq!(map_to_old(&removal, 1..2), (2..3, false));
        assert_eq!(map_to_old(&removal, 0..0), (0..0, false));
    }
}
