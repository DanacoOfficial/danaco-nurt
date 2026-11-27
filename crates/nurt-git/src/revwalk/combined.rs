// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-git::revwalk::combined
// Responsibility:   Combined differences of a merge result against all of its parents, in the.
// Dependencies:     bstr
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Combined differences of a merge result against all of its parents, in the
//! condensed form that `git show` writes for merges.
//!
//! Every line of the result records the parents it is missing from, and
//! every position of the result records the lines that parents had there and
//! the result dropped; equal dropped lines of several parents are shown once.
//! A group of changes is left out when the parents hold at most two versions
//! of it and the result takes one of them unchanged.

use bstr::ByteSlice;

use super::lines::{self, NO_NEWLINE};

/// Largest number of parents whose lines can be told apart; one bit of a
/// mask stands for one parent.
pub(crate) const MAX_PARENTS: usize = 64;

/// Largest product of the lengths of two lists of dropped lines that are
/// matched line by line; longer lists are joined without matching.
const MATCH_LIMIT: usize = 1 << 20;

/// One hunk of a combined difference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CombinedHunk {
    /// First line and number of lines of each parent, written as in the
    /// unified format.
    pub(crate) parents: Vec<(u64, u64)>,
    /// First line of the result.
    pub(crate) new_start: u64,
    /// Number of lines of the result.
    pub(crate) new_len: u64,
    /// Lines with one prefix column per parent: `+` for a line the parent
    /// lacks, `-` for a line only the parent has, space otherwise.
    pub(crate) lines: Vec<String>,
}

/// A line that some parents have at a position and the result does not.
#[derive(Debug, Clone)]
struct Dropped {
    text: Vec<u8>,
    parents: u64,
}

/// A position of the result: the line at it, when it is not the end, and
/// the lines dropped immediately before it.
#[derive(Debug, Clone, Default)]
struct Slot {
    added: u64,
    dropped: Vec<Dropped>,
    /// Index in each parent of the line at this position, after the
    /// dropped lines.
    at: Vec<u32>,
}

/// Hunks of the condensed combined difference of `result` against
/// `parents`, with `context` unchanged lines around each change.
///
/// Returns no hunk when the result differs from the parents only in ways
/// that the condensed form leaves out, and when there are more parents than
/// [`MAX_PARENTS`].
pub(crate) fn combine(parents: &[&[u8]], result: &[u8], context: u32) -> Vec<CombinedHunk> {
    if parents.is_empty() || parents.len() > MAX_PARENTS {
        return Vec::new();
    }
    let result_lines = lines::split_lines(result);
    let count = result_lines.len();
    let mut slots = vec![
        Slot {
            at: vec![0; parents.len()],
            ..Slot::default()
        };
        count + 1
    ];
    for (index, parent) in parents.iter().enumerate() {
        add_parent(&mut slots, index, parent, result);
    }
    let all = all_parents(parents.len());
    let mut marked: Vec<bool> = slots.iter().map(is_changed).collect();
    drop_plain_choices(&slots, &mut marked, all, context);
    let (painted, quiet) = paint_context(&slots, &marked, context);
    runs(&painted)
        .into_iter()
        .map(|run| render(&slots, &result_lines, &quiet, run, parents.len()))
        .collect()
}

fn all_parents(count: usize) -> u64 {
    if count >= MAX_PARENTS {
        u64::MAX
    } else {
        (1u64 << count) - 1
    }
}

fn is_changed(slot: &Slot) -> bool {
    slot.added != 0 || !slot.dropped.is_empty()
}

/// Records the differences of one parent: lines of the result it lacks and
/// lines it has that the result dropped.
fn add_parent(slots: &mut [Slot], index: usize, parent: &[u8], result: &[u8]) {
    let bit = 1u64 << index;
    let parent_lines = lines::split_lines(parent);
    let count = slots.len() - 1;
    let mut position: u32 = 0;
    let mut line = 0usize;
    for edit in lines::edits(parent, result) {
        let start = edit.after.start as usize;
        while line < start {
            slots[line].at[index] = position;
            position += 1;
            line += 1;
        }
        let removed: Vec<&[u8]> = (edit.before.start..edit.before.end)
            .filter_map(|old| parent_lines.get(old as usize).copied())
            .collect();
        position += u32::try_from(removed.len()).unwrap_or(u32::MAX);
        merge_dropped(&mut slots[start].dropped, &removed, bit);
        for added in edit.after.start..edit.after.end {
            let slot = &mut slots[added as usize];
            slot.added |= bit;
            slot.at[index] = position;
        }
        line = edit.after.end as usize;
    }
    while line <= count {
        slots[line].at[index] = position;
        position += 1;
        line += 1;
    }
}

/// Adds lines dropped by one parent to the lines dropped by earlier parents
/// at the same position; a line equal to one already there in the longest
/// common order is shared instead of repeated.
fn merge_dropped(list: &mut Vec<Dropped>, removed: &[&[u8]], bit: u64) {
    if removed.is_empty() {
        return;
    }
    let fresh = |text: &[u8]| Dropped {
        text: text.to_vec(),
        parents: bit,
    };
    let (n, m) = (list.len(), removed.len());
    if n == 0 || n.saturating_mul(m) > MATCH_LIMIT {
        list.extend(removed.iter().map(|text| fresh(text)));
        return;
    }
    let width = m + 1;
    let mut common = vec![0u32; (n + 1) * width];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            common[i * width + j] = if list[i].text == removed[j] {
                common[(i + 1) * width + j + 1] + 1
            } else {
                common[(i + 1) * width + j].max(common[i * width + j + 1])
            };
        }
    }
    let existing = std::mem::take(list);
    let mut merged = Vec::with_capacity(n + m);
    let (mut i, mut j) = (0, 0);
    let mut existing = existing.into_iter().peekable();
    while i < n || j < m {
        let same = i < n && j < m && existing.peek().is_some_and(|item| item.text == removed[j]);
        if same && common[i * width + j] == common[(i + 1) * width + j + 1] + 1 {
            if let Some(mut item) = existing.next() {
                item.parents |= bit;
                merged.push(item);
            }
            i += 1;
            j += 1;
        } else if i < n && (j == m || common[(i + 1) * width + j] >= common[i * width + j + 1]) {
            merged.extend(existing.next());
            i += 1;
        } else {
            merged.push(fresh(removed[j]));
            j += 1;
        }
    }
    *list = merged;
}

/// Position of the end of a group for the count of trailing context: a
/// last position that only carries dropped lines shows its own line as the
/// first context line.
fn trailing_end(slots: &[Slot], begin: usize, end: usize) -> usize {
    if end > begin && slots[end - 1].added == 0 {
        end - 1
    } else {
        end
    }
}

/// Unmarks groups of changes in which every changed or dropped line differs
/// from the same set of parents, short of all of them: the parents then hold
/// two versions and the result is one of them.
fn drop_plain_choices(slots: &[Slot], marked: &mut [bool], all: u64, context: u32) {
    let last = slots.len();
    let context = context as usize;
    let mut begin = 0;
    while begin < last {
        if !marked[begin] {
            begin += 1;
            continue;
        }
        let mut end = begin + 1;
        loop {
            while end < last && marked[end] {
                end += 1;
            }
            if end >= last {
                break;
            }
            let reach = (trailing_end(slots, begin, end) + context).min(last);
            match (end..reach).rev().find(|&index| marked[index]) {
                Some(next) => end = next + 1,
                None => break,
            }
        }
        if !has_several_versions(&slots[begin..end]) && group_mask(&slots[begin..end]) != all {
            marked[begin..end].iter_mut().for_each(|mark| *mark = false);
        }
        begin = end;
    }
}

fn masks(slots: &[Slot]) -> impl Iterator<Item = u64> + '_ {
    slots.iter().flat_map(|slot| {
        let added = (slot.added != 0).then_some(slot.added);
        added
            .into_iter()
            .chain(slot.dropped.iter().map(|dropped| dropped.parents))
    })
}

fn has_several_versions(group: &[Slot]) -> bool {
    let mut seen = masks(group);
    let Some(first) = seen.next() else {
        return false;
    };
    seen.any(|mask| mask != first)
}

fn group_mask(group: &[Slot]) -> u64 {
    masks(group).next().unwrap_or(0)
}

/// Positions shown, and positions shown as leading context whose dropped
/// lines are not written.
fn paint_context(slots: &[Slot], marked: &[bool], context: u32) -> (Vec<bool>, Vec<bool>) {
    let last = slots.len();
    let context = context as usize;
    let mut painted = marked.to_vec();
    let mut quiet = vec![false; last];
    let next_marked =
        |from: usize, painted: &[bool]| (from..last).find(|&index| painted[index]).unwrap_or(last);
    let mut current = next_marked(0, &painted);
    while current < last {
        for index in current.saturating_sub(context)..current {
            if !painted[index] {
                quiet[index] = true;
                painted[index] = true;
            }
        }
        loop {
            let gap = (current..last)
                .find(|&index| !painted[index])
                .unwrap_or(last);
            if gap >= last {
                return (painted, quiet);
            }
            let next = next_marked(gap, &painted);
            let tail = trailing_end(slots, current, gap);
            if next < tail + context {
                painted[tail..next].iter_mut().for_each(|mark| *mark = true);
                current = next;
                continue;
            }
            painted[tail..(tail + context).min(last)]
                .iter_mut()
                .for_each(|mark| *mark = true);
            current = next;
            break;
        }
    }
    (painted, quiet)
}

fn runs(painted: &[bool]) -> Vec<std::ops::Range<usize>> {
    let mut out = Vec::new();
    let mut index = 0;
    while index < painted.len() {
        if painted[index] {
            let start = index;
            while index < painted.len() && painted[index] {
                index += 1;
            }
            out.push(start..index);
        } else {
            index += 1;
        }
    }
    out
}

fn render(
    slots: &[Slot],
    result: &[&[u8]],
    quiet: &[bool],
    run: std::ops::Range<usize>,
    parents: usize,
) -> CombinedHunk {
    let count = result.len();
    let mut out = Vec::new();
    let mut parent_len = vec![0u64; parents];
    let mut parent_first: Vec<Option<u64>> = vec![None; parents];
    for index in run.clone() {
        let slot = &slots[index];
        if !quiet[index] {
            let mut seen = vec![0u32; parents];
            for dropped in &slot.dropped {
                let mut prefix = String::with_capacity(parents);
                for (parent, len) in parent_len.iter_mut().enumerate() {
                    if dropped.parents & (1u64 << parent) != 0 {
                        prefix.push('-');
                        let lost_here = lost_count(slot, parent);
                        let first = slot.at[parent] - lost_here + seen[parent];
                        parent_first[parent].get_or_insert(u64::from(first));
                        seen[parent] += 1;
                        *len += 1;
                    } else {
                        prefix.push(' ');
                    }
                }
                push_line(&mut out, &prefix, &dropped.text);
            }
        }
        if index < count {
            let mut prefix = String::with_capacity(parents);
            for (parent, len) in parent_len.iter_mut().enumerate() {
                if slot.added & (1u64 << parent) != 0 {
                    prefix.push('+');
                } else {
                    prefix.push(' ');
                    parent_first[parent].get_or_insert(u64::from(slot.at[parent]));
                    *len += 1;
                }
            }
            push_line(&mut out, &prefix, result[index]);
        }
    }
    let new_begin = run.start.min(count);
    let new_end = run.end.min(count);
    let parents_ranges = (0..parents)
        .map(|parent| {
            let len = parent_len[parent];
            let first =
                parent_first[parent].unwrap_or_else(|| u64::from(slots[run.start].at[parent]));
            (if len > 0 { first + 1 } else { first }, len)
        })
        .collect();
    let new_len = (new_end - new_begin) as u64;
    CombinedHunk {
        parents: parents_ranges,
        new_start: if new_len > 0 {
            new_begin as u64 + 1
        } else {
            new_begin as u64
        },
        new_len,
        lines: out,
    }
}

fn lost_count(slot: &Slot, parent: usize) -> u32 {
    let bit = 1u64 << parent;
    u32::try_from(
        slot.dropped
            .iter()
            .filter(|dropped| dropped.parents & bit != 0)
            .count(),
    )
    .unwrap_or(u32::MAX)
}

fn push_line(out: &mut Vec<String>, prefix: &str, line: &[u8]) {
    let body = line.strip_suffix(b"\n").unwrap_or(line);
    let mut text = String::with_capacity(prefix.len() + body.len());
    text.push_str(prefix);
    text.push_str(&body.to_str_lossy());
    out.push(text);
    if !line.ends_with(b"\n") {
        out.push(NO_NEWLINE.to_owned());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(lines: &[&str]) -> Vec<u8> {
        lines
            .iter()
            .flat_map(|line| format!("{line}\n").into_bytes())
            .collect()
    }

    #[test]
    fn result_taken_from_one_parent_has_no_hunks() {
        let first = text(&["a", "b", "c"]);
        let second = text(&["a", "B", "c"]);
        assert!(combine(&[&first, &second], &second, 3).is_empty());
        assert!(combine(&[&first, &second], &first, 3).is_empty());
    }

    #[test]
    fn conflict_resolution_shows_both_parents() {
        let first = text(&["a", "ours", "c"]);
        let second = text(&["a", "theirs", "c"]);
        let result = text(&["a", "merged", "c"]);
        let hunks = combine(&[&first, &second], &result, 3);
        assert_eq!(hunks.len(), 1);
        let hunk = &hunks[0];
        assert_eq!(hunk.lines, ["  a", "- ours", " -theirs", "++merged", "  c"]);
        assert_eq!(hunk.parents, vec![(1, 3), (1, 3)]);
        assert_eq!((hunk.new_start, hunk.new_len), (1, 3));
    }

    #[test]
    fn equal_dropped_lines_are_shared() {
        let first = text(&["a", "old", "x"]);
        let second = text(&["a", "old", "y"]);
        let result = text(&["a", "new", "z"]);
        let hunks = combine(&[&first, &second], &result, 1);
        assert_eq!(
            hunks[0].lines,
            ["  a", "--old", "- x", " -y", "++new", "++z"]
        );
    }

    #[test]
    fn line_added_to_both_sides_is_shown() {
        let first = text(&["a", "b"]);
        let second = text(&["a", "c"]);
        let result = text(&["a", "b", "c", "new"]);
        let hunks = combine(&[&first, &second], &result, 3);
        assert_eq!(hunks.len(), 1);
        assert!(hunks[0].lines.contains(&"++new".to_owned()));
        assert!(hunks[0].lines.contains(&" +b".to_owned()));
        assert!(hunks[0].lines.contains(&"+ c".to_owned()));
    }

    #[test]
    fn distant_changes_form_separate_hunks() {
        let base: Vec<String> = (1..=30).map(|n| n.to_string()).collect();
        let base: Vec<&str> = base.iter().map(String::as_str).collect();
        let mut first = base.clone();
        first[1] = "first";
        first[25] = "one";
        let mut second = base.clone();
        second[1] = "second";
        second[25] = "two";
        let mut result = base.clone();
        result[1] = "both";
        result[25] = "both again";
        let hunks = combine(&[&text(&first), &text(&second)], &text(&result), 3);
        assert_eq!(hunks.len(), 2);
        assert_eq!(hunks[1].new_start, 23);
        assert_eq!(hunks[1].parents[0], (23, 7));
    }

    #[test]
    fn missing_final_line_break_is_marked() {
        let first = b"a\nx".to_vec();
        let second = b"a\ny".to_vec();
        let result = b"a\nz".to_vec();
        let hunks = combine(&[&first, &second], &result, 3);
        assert!(hunks[0].lines.iter().any(|line| line == NO_NEWLINE));
        assert_eq!(hunks[0].lines.last().map(String::as_str), Some(NO_NEWLINE));
    }

    #[test]
    fn new_file_in_every_parent_is_all_added() {
        let result = text(&["x", "y"]);
        let hunks = combine(&[b"", b""], &result, 3);
        assert_eq!(hunks[0].lines, ["++x", "++y"]);
        assert_eq!(hunks[0].parents, vec![(0, 0), (0, 0)]);
    }

    #[test]
    fn deletion_against_both_parents_lists_dropped_lines() {
        let first = text(&["keep", "gone one"]);
        let second = text(&["keep", "gone two"]);
        let result = text(&["keep"]);
        let hunks = combine(&[&first, &second], &result, 3);
        assert_eq!(hunks[0].lines, ["  keep", "- gone one", " -gone two"]);
        assert_eq!(hunks[0].parents, vec![(1, 2), (1, 2)]);
        assert_eq!((hunks[0].new_start, hunks[0].new_len), (1, 1));
    }

    #[test]
    fn rejects_too_many_parents() {
        let parents: Vec<&[u8]> = vec![b"a\n"; MAX_PARENTS + 1];
        assert!(combine(&parents, b"b\n", 3).is_empty());
        assert!(combine(&[], b"b\n", 3).is_empty());
        assert_eq!(all_parents(MAX_PARENTS), u64::MAX);
    }

    #[test]
    fn later_parents_share_equal_dropped_lines() {
        let mut list = vec![Dropped {
            text: b"a\n".to_vec(),
            parents: 1,
        }];
        merge_dropped(&mut list, &[b"a\n", b"b\n"], 2);
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].parents, 3);
        assert_eq!(list[1].parents, 2);
        merge_dropped(&mut list, &[], 4);
        assert_eq!(list.len(), 2);
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
