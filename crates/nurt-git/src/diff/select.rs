// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Application of selected hunks and rows to a text.
//!
//! [`Direction::Forward`] applies the selection to the old text, as when
//! staging: a selected removed line is dropped, an unselected one stays, a
//! selected added line is inserted and an unselected one is not.
//! [`Direction::Reverse`] takes the selection back out of the new text, as
//! when discarding: a selected added line is dropped, an unselected one stays,
//! a selected removed line is restored and an unselected one is not. A
//! `\ No newline at end of file` row belongs to the line before it.

use std::collections::BTreeMap;

use bstr::ByteSlice;
use nurt_contracts::error::Error;
use nurt_contracts::record::HunkStatus;

use super::hunk::{Hunk, LineKind};
use super::id::{HunkId, HunkProblem, RowRange, hunk_error};
use super::lines::split_lines;

/// Direction in which a selection is applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Apply the selected changes to the old text.
    Forward,
    /// Take the selected changes out of the new text.
    Reverse,
}

/// Applies the hunks named by `ids` to `base`.
///
/// `hunks` are the canonical hunks of one file ([`super::hunks_for`]);
/// `base` is the old text for [`Direction::Forward`] and the new text for
/// [`Direction::Reverse`], in the form stored by Git. Hunks that are not
/// named stay as they are in `base`. Several identifiers of one hunk add up
/// their rows; an identifier without rows selects the whole hunk.
///
/// # Errors
///
/// Errors carry the problem names of hunk selections: `not_found` for a
/// number the file does not have, `changed` for a fingerprint the hunk no
/// longer has, `invalid_rows` for rows outside the hunk, `no_change_rows`
/// when the rows select no added or removed line,
/// `rows_on_added_or_deleted` for rows of an added or deleted file and
/// `apply_failed` when `base` does not match the hunks.
///
/// # Examples
///
/// ```
/// use nurt_git::diff::{apply_selected, text_hunks, Direction, HunkId};
///
/// let old = b"a\nb\nc\n";
/// let new = b"a\nB\nc\nd\n";
/// let hunks = text_hunks("f.txt", old, new);
/// let id: HunkId = "f.txt#1@2-3".parse()?;
/// let staged = apply_selected(old, &hunks, &[id], Direction::Forward)?;
/// assert_eq!(staged, b"a\nB\nc\n");
/// # Ok::<(), nurt_contracts::error::Error>(())
/// ```
pub fn apply_selected(base: &[u8], hunks: &[Hunk], ids: &[HunkId], direction: Direction) -> Result<Vec<u8>, Error> {
    let picks = collect_picks(hunks, ids)?;
    let base_lines = split_lines(base);
    let mut out = Vec::with_capacity(base.len());
    let mut cursor = 0;
    for (number, rows) in &picks {
        let hunk = hunks
            .iter()
            .find(|hunk| hunk.number == *number)
            .ok_or_else(|| not_found(hunks, *number))?;
        let (start, len) = match direction {
            Direction::Forward => (hunk.old_start, hunk.old_len),
            Direction::Reverse => (hunk.new_start, hunk.new_len),
        };
        let first = if len == 0 { start as usize } else { start as usize - 1 };
        if first < cursor || first > base_lines.len() {
            return Err(mismatch(hunk));
        }
        for line in &base_lines[cursor..first] {
            push(&mut out, line);
        }
        cursor = apply_hunk(&mut out, &base_lines, first, hunk, rows.as_deref(), direction)?;
    }
    for line in &base_lines[cursor..] {
        push(&mut out, line);
    }
    Ok(out)
}

type Picks = BTreeMap<u32, Option<Vec<RowRange>>>;

fn collect_picks(hunks: &[Hunk], ids: &[HunkId]) -> Result<Picks, Error> {
    let mut picks = Picks::new();
    for id in ids {
        let hunk = hunks
            .iter()
            .find(|hunk| hunk.number == id.number)
            .ok_or_else(|| not_found(hunks, id.number))?;
        if hunk.binary {
            return Err(hunk_error(
                HunkProblem::ApplyFailed,
                format!("`{id}` belongs to a binary file, select the whole path instead"),
            ));
        }
        if let Some(expected) = &id.fingerprint {
            if *expected != hunk.fingerprint() {
                return Err(hunk_error(
                    HunkProblem::Changed,
                    format!("hunk `{id}` changed since it was listed, list the hunks again"),
                ));
            }
        }
        if let Some(rows) = &id.rows {
            check_rows(hunk, id, rows)?;
        }
        let entry = picks.entry(id.number).or_insert_with(|| Some(Vec::new()));
        match (&id.rows, entry.as_mut()) {
            (Some(rows), Some(selected)) => selected.extend_from_slice(rows),
            _ => *entry = None,
        }
    }
    Ok(picks)
}

fn check_rows(hunk: &Hunk, id: &HunkId, rows: &[RowRange]) -> Result<(), Error> {
    if hunk.status != HunkStatus::Modified && hunk.status != HunkStatus::Renamed {
        return Err(hunk_error(
            HunkProblem::RowsOnAddedOrDeleted,
            format!("rows can be selected only in modified files, select the whole hunk `{}`", hunk.id()),
        ));
    }
    let count = u32::try_from(hunk.lines.len()).unwrap_or(u32::MAX);
    if rows.iter().any(|range| range.last > count) {
        return Err(hunk_error(
            HunkProblem::InvalidRows,
            format!("`{id}` selects rows beyond the {count} rows of the hunk"),
        ));
    }
    let changes_selected = hunk.lines.iter().enumerate().any(|(index, line)| {
        matches!(line.kind, LineKind::Added | LineKind::Removed) && row_selected(hunk, index, Some(rows))
    });
    if changes_selected {
        Ok(())
    } else {
        Err(hunk_error(
            HunkProblem::NoChangeRows,
            format!("`{id}` selects no added or removed line"),
        ))
    }
}

fn row_selected(hunk: &Hunk, index: usize, rows: Option<&[RowRange]>) -> bool {
    let Some(rows) = rows else {
        return true;
    };
    let hit = |position: usize| {
        let row = u32::try_from(position + 1).unwrap_or(u32::MAX);
        rows.iter().any(|range| range.contains(row))
    };
    let marker_follows = hunk
        .lines
        .get(index + 1)
        .is_some_and(|line| line.kind == LineKind::NoNewline);
    hit(index) || (marker_follows && hit(index + 1))
}

fn apply_hunk(
    out: &mut Vec<u8>,
    base: &[&[u8]],
    first: usize,
    hunk: &Hunk,
    rows: Option<&[RowRange]>,
    direction: Direction,
) -> Result<usize, Error> {
    let (kept_side, other_side) = match direction {
        Direction::Forward => (LineKind::Removed, LineKind::Added),
        Direction::Reverse => (LineKind::Added, LineKind::Removed),
    };
    let mut cursor = first;
    for (index, line) in hunk.lines.iter().enumerate() {
        if line.kind == LineKind::NoNewline {
            continue;
        }
        let selected = row_selected(hunk, index, rows);
        let terminated = hunk
            .lines
            .get(index + 1)
            .is_none_or(|next| next.kind != LineKind::NoNewline);
        if line.kind == LineKind::Context || line.kind == kept_side {
            let current = base.get(cursor).ok_or_else(|| mismatch(hunk))?;
            if current.strip_suffix(b"\n").unwrap_or(current) != line.text.as_slice() {
                return Err(mismatch(hunk));
            }
            cursor += 1;
            if line.kind == LineKind::Context || !selected {
                push(out, current);
            }
        } else if line.kind == other_side && selected {
            separate(out);
            out.extend_from_slice(&line.text);
            if terminated {
                out.push(b'\n');
            }
        }
    }
    Ok(cursor)
}

fn push(out: &mut Vec<u8>, line: &[u8]) {
    separate(out);
    out.extend_from_slice(line);
}

/// A line written after a line without line feed starts a new line.
fn separate(out: &mut Vec<u8>) {
    if out.last().is_some_and(|&byte| byte != b'\n') {
        out.push(b'\n');
    }
}

fn not_found(hunks: &[Hunk], number: u32) -> Error {
    let path = hunks
        .first()
        .map(|hunk| hunk.path.to_str_lossy().into_owned())
        .unwrap_or_default();
    hunk_error(
        HunkProblem::NotFound,
        format!(
            "`{path}` has no hunk {number} ({} hunks), list the hunks again",
            hunks.len()
        ),
    )
}

fn mismatch(hunk: &Hunk) -> Error {
    hunk_error(
        HunkProblem::ApplyFailed,
        format!("hunk `{}` does not match the file, list the hunks again", hunk.id()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::text_hunks;

    fn ids(list: &[&str]) -> Vec<HunkId> {
        list.iter().map(|text| text.parse().unwrap()).collect()
    }

    const OLD: &[u8] = b"1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n11\n12\n";
    const NEW: &[u8] = b"1\nTWO\n3\n4\n5\n6\n7\n8\n9\n10\n11\nTWELVE\n13\n";

    #[test]
    fn forward_whole_hunk_stages_only_that_hunk() {
        let hunks = text_hunks("f", OLD, NEW);
        assert_eq!(hunks.len(), 2);
        let staged = apply_selected(OLD, &hunks, &ids(&["f#2"]), Direction::Forward).unwrap();
        assert_eq!(staged, b"1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n11\nTWELVE\n13\n");
        let all = apply_selected(OLD, &hunks, &ids(&["f#1", "f#2"]), Direction::Forward).unwrap();
        assert_eq!(all, NEW);
    }

    #[test]
    fn reverse_discards_selected_hunk_from_new_text() {
        let hunks = text_hunks("f", OLD, NEW);
        let restored = apply_selected(NEW, &hunks, &ids(&["f#1"]), Direction::Reverse).unwrap();
        assert_eq!(restored, b"1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n11\nTWELVE\n13\n");
        let none = apply_selected(NEW, &hunks, &ids(&["f#1", "f#2"]), Direction::Reverse).unwrap();
        assert_eq!(none, OLD);
    }

    #[test]
    fn rows_select_single_lines() {
        let hunks = text_hunks("f", OLD, NEW);
        let second = &hunks[1];
        let rendered: Vec<String> = second.lines.iter().map(|line| line.render().to_string()).collect();
        assert_eq!(rendered, [" 9", " 10", " 11", "-12", "+TWELVE", "+13"]);
        let only_added = apply_selected(OLD, &hunks, &ids(&["f#2@6"]), Direction::Forward).unwrap();
        assert_eq!(only_added, b"1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n11\n12\n13\n");
        let reverse = apply_selected(NEW, &hunks, &ids(&["f#2@4-5"]), Direction::Reverse).unwrap();
        assert_eq!(reverse, b"1\nTWO\n3\n4\n5\n6\n7\n8\n9\n10\n11\n12\n13\n");
    }

    #[test]
    fn handles_missing_final_line_feed() {
        let old = b"a\nb";
        let new = b"a\nb\nc\n";
        let hunks = text_hunks("f", old, new);
        let staged = apply_selected(old, &hunks, &ids(&["f#1"]), Direction::Forward).unwrap();
        assert_eq!(staged, new);
        let back = apply_selected(new, &hunks, &ids(&["f#1"]), Direction::Reverse).unwrap();
        assert_eq!(back, old);
    }

    #[test]
    fn reports_selection_problems() {
        let hunks = text_hunks("f", OLD, NEW);
        let problem = |list: &[&str]| {
            apply_selected(OLD, &hunks, &ids(list), Direction::Forward)
                .unwrap_err()
                .data()
                .problem
                .clone()
        };
        assert_eq!(problem(&["f#3"]).as_deref(), Some("not_found"));
        assert_eq!(problem(&["f#1:000000"]).as_deref(), Some("changed"));
        assert_eq!(problem(&["f#1@40"]).as_deref(), Some("invalid_rows"));
        assert_eq!(problem(&["f#1@1"]).as_deref(), Some("no_change_rows"));
        let fingerprint = hunks[0].fingerprint();
        let current = format!("f#1:{fingerprint}");
        assert!(apply_selected(OLD, &hunks, &ids(&[current.as_str()]), Direction::Forward).is_ok());
        let wrong_base = apply_selected(b"x\n", &hunks, &ids(&["f#1"]), Direction::Forward).unwrap_err();
        assert_eq!(wrong_base.data().problem.as_deref(), Some("apply_failed"));
    }

    #[test]
    fn rejects_rows_in_added_file() {
        let mut hunks = text_hunks("f", b"", b"a\nb\n");
        hunks[0].status = HunkStatus::Added;
        let error = apply_selected(b"", &hunks, &ids(&["f#1@1"]), Direction::Forward).unwrap_err();
        assert_eq!(error.data().problem.as_deref(), Some("rows_on_added_or_deleted"));
        let whole = apply_selected(b"", &hunks, &ids(&["f#1"]), Direction::Forward).unwrap();
        assert_eq!(whole, b"a\nb\n");
    }

    #[test]
    fn rejects_binary_hunk() {
        let hunks = vec![Hunk::binary_placeholder("f".into())];
        let error = apply_selected(b"", &hunks, &ids(&["f#1"]), Direction::Forward).unwrap_err();
        assert_eq!(error.data().problem.as_deref(), Some("apply_failed"));
    }
}
