// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

//! Patch identifiers and patch text of commits.
//!
//! The patch identifier follows the unstable mode of `git patch-id`: one
//! SHA-1 over the file headers and the lines of the patch with all white
//! space removed, without hunk headers. Two commits that introduce the same
//! change have the same identifier, which is how already applied commits are
//! recognised. The patch text is the content of `rebase-merge/patch`.

use std::ops::Range;

use bstr::{BStr, ByteSlice};
use gix::ObjectId;
use gix_imara_diff::{Algorithm, Diff, InternedInput};
use gix_object::tree::EntryMode;
use nurt_contracts::error::Result;
use sha1::{Digest, Sha1};

use super::objects::{self, Leaf, LeafChange};

/// Lines of context around a change.
const CONTEXT_LINES: u32 = 3;
/// Changes closer than this many lines share one hunk.
const MAX_COMMON_LINES: u32 = 2 * CONTEXT_LINES;
/// Number of leading bytes inspected to decide whether content is binary.
const BINARY_PROBE_BYTES: usize = 8000;
/// Longest function context shown after a hunk header.
const FUNCTION_CONTEXT_BYTES: usize = 80;
/// Minimal length of abbreviated object names in `index` lines.
const ABBREV_MIN: usize = 7;

/// Patch identifier of the change between two trees.
pub(super) fn patch_id(repo: &gix::Repository, old: ObjectId, new: ObjectId) -> Result<[u8; 20]> {
    let mut hasher = Sha1::new();
    for change in objects::diff_trees(repo, old, new)? {
        hash_change(repo, &change, &mut hasher)?;
    }
    let mut out = [0u8; 20];
    out.copy_from_slice(&hasher.finalize());
    Ok(out)
}

fn hash_change(repo: &gix::Repository, change: &LeafChange, hasher: &mut Sha1) -> Result<()> {
    let path = strip_space(&change.path);
    hasher.update(b"diff--git");
    hasher.update(b"a/");
    hasher.update(&path);
    hasher.update(b"b/");
    hasher.update(&path);
    match (change.old, change.new) {
        (None, Some(new)) => {
            hasher.update(b"newfilemode");
            hasher.update(objects::mode_text(new.mode).as_bytes());
        }
        (Some(old), None) => {
            hasher.update(b"deletedfilemode");
            hasher.update(objects::mode_text(old.mode).as_bytes());
        }
        (Some(old), Some(new)) if old.mode != new.mode => {
            hasher.update(b"oldmode");
            hasher.update(objects::mode_text(old.mode).as_bytes());
            hasher.update(b"newmode");
            hasher.update(objects::mode_text(new.mode).as_bytes());
        }
        _ => {}
    }
    let old_data = content(repo, change.old)?;
    let new_data = content(repo, change.new)?;
    if is_binary(&old_data) || is_binary(&new_data) {
        let null = ObjectId::null(repo.object_hash());
        hasher.update(change.old.map_or(null, |leaf| leaf.id).to_hex().to_string().as_bytes());
        hasher.update(change.new.map_or(null, |leaf| leaf.id).to_hex().to_string().as_bytes());
        return Ok(());
    }
    match (change.old, change.new) {
        (None, _) => {
            hasher.update(b"---/dev/null");
            hasher.update(b"+++b/");
            hasher.update(&path);
        }
        (_, None) => {
            hasher.update(b"---a/");
            hasher.update(&path);
            hasher.update(b"+++/dev/null");
        }
        _ => {
            hasher.update(b"---a/");
            hasher.update(&path);
            hasher.update(b"+++b/");
            hasher.update(&path);
        }
    }
    let diff = LineDiff::new(&old_data, &new_data);
    for hunk in diff.hunks() {
        diff.each_line(&hunk, |marker, line| {
            hasher.update([marker]);
            hasher.update(strip_space(line));
        });
    }
    Ok(())
}

/// Bytes with every ASCII white space character removed.
fn strip_space(bytes: &[u8]) -> Vec<u8> {
    bytes
        .iter()
        .copied()
        .filter(|byte| !is_space(*byte))
        .collect()
}

/// White space as classified by the C library in the C locale.
fn is_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c)
}

/// Content of a leaf as compared in patches; submodules are shown as the
/// line `Subproject commit <id>`.
fn content(repo: &gix::Repository, leaf: Option<Leaf>) -> Result<Vec<u8>> {
    match leaf {
        None => Ok(Vec::new()),
        Some(leaf) if leaf.mode.is_commit() => {
            Ok(format!("Subproject commit {}\n", leaf.id.to_hex()).into_bytes())
        }
        Some(leaf) => objects::read_blob(repo, leaf.id),
    }
}

/// Whether content is treated as binary: a NUL byte among the first bytes.
fn is_binary(data: &[u8]) -> bool {
    data.iter().take(BINARY_PROBE_BYTES).any(|byte| *byte == 0)
}

/// One hunk of a unified diff: line ranges of both sides with context and
/// the changes inside.
struct Hunk {
    old: Range<u32>,
    new: Range<u32>,
    changes: Vec<gix_imara_diff::Hunk>,
}

/// Line difference of two texts with hunks grouped like Git does.
struct LineDiff<'a> {
    old: Vec<&'a [u8]>,
    new: Vec<&'a [u8]>,
    changes: Vec<gix_imara_diff::Hunk>,
}

impl<'a> LineDiff<'a> {
    fn new(old: &'a [u8], new: &'a [u8]) -> Self {
        let old_lines: Vec<&[u8]> = old.split_inclusive(|byte| *byte == b'\n').collect();
        let new_lines: Vec<&[u8]> = new.split_inclusive(|byte| *byte == b'\n').collect();
        let input = InternedInput::new(old_lines.iter().copied(), new_lines.iter().copied());
        let mut diff = Diff::compute(Algorithm::Myers, &input);
        diff.postprocess_no_heuristic(&input);
        Self {
            old: old_lines,
            new: new_lines,
            changes: diff.hunks().collect(),
        }
    }

    /// Hunks with context; changes separated by at most twice the context
    /// share a hunk.
    fn hunks(&self) -> Vec<Hunk> {
        let mut out: Vec<Hunk> = Vec::new();
        for change in &self.changes {
            let joins = out.last().and_then(|hunk| hunk.changes.last()).is_some_and(|last| {
                change.before.start.saturating_sub(last.before.end) <= MAX_COMMON_LINES
            });
            if joins && let Some(hunk) = out.last_mut() {
                hunk.changes.push(change.clone());
                continue;
            }
            out.push(Hunk {
                old: 0..0,
                new: 0..0,
                changes: vec![change.clone()],
            });
        }
        for hunk in &mut out {
            self.frame(hunk);
        }
        out
    }

    /// Sets the line ranges of a hunk including its context.
    fn frame(&self, hunk: &mut Hunk) {
        let (Some(first), Some(last)) = (hunk.changes.first(), hunk.changes.last()) else {
            return;
        };
        let lead = first.before.start.min(CONTEXT_LINES);
        let old_len = u32::try_from(self.old.len()).unwrap_or(u32::MAX);
        let trail = old_len.saturating_sub(last.before.end).min(CONTEXT_LINES);
        hunk.old = first.before.start - lead..last.before.end + trail;
        hunk.new = first.after.start - lead..last.after.end + trail;
    }

    /// Calls `emit` with the marker (` `, `-`, `+`) and the text of every
    /// line of a hunk, in output order.
    fn each_line(&self, hunk: &Hunk, mut emit: impl FnMut(u8, &[u8])) {
        let mut old_at = hunk.old.start;
        for change in &hunk.changes {
            for line in old_at..change.before.start {
                emit(b' ', self.old_line(line));
            }
            for line in change.before.clone() {
                emit(b'-', self.old_line(line));
            }
            for line in change.after.clone() {
                emit(b'+', self.new_line(line));
            }
            old_at = change.before.end;
        }
        for line in old_at..hunk.old.end {
            emit(b' ', self.old_line(line));
        }
    }

    fn old_line(&self, index: u32) -> &'a [u8] {
        usize::try_from(index)
            .ok()
            .and_then(|index| self.old.get(index))
            .copied()
            .unwrap_or_default()
    }

    fn new_line(&self, index: u32) -> &'a [u8] {
        usize::try_from(index)
            .ok()
            .and_then(|index| self.new.get(index))
            .copied()
            .unwrap_or_default()
    }

    /// Function context of a hunk: the nearest line before it that starts
    /// with a letter, `_` or `$`, without trailing white space.
    fn function_context(&self, hunk: &Hunk) -> Option<&'a [u8]> {
        let start = usize::try_from(hunk.old.start).ok()?;
        self.old[..start.min(self.old.len())]
            .iter()
            .rev()
            .find(|line| {
                line.first()
                    .is_some_and(|byte| byte.is_ascii_alphabetic() || *byte == b'_' || *byte == b'$')
            })
            .map(|line| {
                let line = &line[..line.len().min(FUNCTION_CONTEXT_BYTES)];
                line.trim_end_with(|c| c.is_ascii_whitespace())
            })
    }
}

/// Text of the patch between two trees in the format of `git log -p`.
pub(super) fn patch_text(
    repo: &gix::Repository,
    short: &dyn Fn(ObjectId) -> String,
    old: ObjectId,
    new: ObjectId,
) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    for change in objects::diff_trees(repo, old, new)? {
        write_change(repo, short, &change, &mut out)?;
    }
    Ok(out)
}

fn write_change(
    repo: &gix::Repository,
    short: &dyn Fn(ObjectId) -> String,
    change: &LeafChange,
    out: &mut Vec<u8>,
) -> Result<()> {
    let path = change.path.as_bstr();
    out.extend_from_slice(b"diff --git a/");
    out.extend_from_slice(path);
    out.extend_from_slice(b" b/");
    out.extend_from_slice(path);
    out.push(b'\n');
    write_mode_lines(change, out);
    let old_data = content(repo, change.old)?;
    let new_data = content(repo, change.new)?;
    let abbrev = |leaf: Option<Leaf>| match leaf {
        Some(leaf) => short(leaf.id),
        None => "0".repeat(ABBREV_MIN),
    };
    let same_content = change.old.map(|leaf| leaf.id) == change.new.map(|leaf| leaf.id);
    if !same_content {
        out.extend_from_slice(format!("index {}..{}", abbrev(change.old), abbrev(change.new)).as_bytes());
        if let (Some(old), Some(new)) = (change.old, change.new)
            && old.mode == new.mode
        {
            out.extend_from_slice(format!(" {}", objects::mode_text(new.mode)).as_bytes());
        }
        out.push(b'\n');
    }
    if same_content {
        return Ok(());
    }
    let (old_name, new_name) = side_names(path, change);
    if is_binary(&old_data) || is_binary(&new_data) {
        out.extend_from_slice(b"Binary files ");
        out.extend_from_slice(&old_name);
        out.extend_from_slice(b" and ");
        out.extend_from_slice(&new_name);
        out.extend_from_slice(b" differ\n");
        return Ok(());
    }
    out.extend_from_slice(b"--- ");
    out.extend_from_slice(&old_name);
    out.extend_from_slice(b"\n+++ ");
    out.extend_from_slice(&new_name);
    out.push(b'\n');
    write_hunks(&LineDiff::new(&old_data, &new_data), out);
    Ok(())
}

/// Names of both sides in `---`/`+++` lines.
fn side_names(path: &BStr, change: &LeafChange) -> (Vec<u8>, Vec<u8>) {
    let named = |prefix: &str| {
        let mut name = prefix.as_bytes().to_vec();
        name.extend_from_slice(path);
        name
    };
    let old = if change.old.is_some() {
        named("a/")
    } else {
        b"/dev/null".to_vec()
    };
    let new = if change.new.is_some() {
        named("b/")
    } else {
        b"/dev/null".to_vec()
    };
    (old, new)
}

fn write_mode_lines(change: &LeafChange, out: &mut Vec<u8>) {
    match (change.old, change.new) {
        (None, Some(new)) => {
            out.extend_from_slice(format!("new file mode {}\n", objects::mode_text(new.mode)).as_bytes());
        }
        (Some(old), None) => {
            out.extend_from_slice(format!("deleted file mode {}\n", objects::mode_text(old.mode)).as_bytes());
        }
        (Some(old), Some(new)) if old.mode != new.mode => {
            out.extend_from_slice(
                format!(
                    "old mode {}\nnew mode {}\n",
                    objects::mode_text(old.mode),
                    objects::mode_text(new.mode)
                )
                .as_bytes(),
            );
        }
        _ => {}
    }
}

fn write_hunks(diff: &LineDiff<'_>, out: &mut Vec<u8>) {
    for hunk in diff.hunks() {
        out.extend_from_slice(b"@@ -");
        out.extend_from_slice(range_text(&hunk.old).as_bytes());
        out.extend_from_slice(b" +");
        out.extend_from_slice(range_text(&hunk.new).as_bytes());
        out.extend_from_slice(b" @@");
        if let Some(function) = diff.function_context(&hunk) {
            out.push(b' ');
            out.extend_from_slice(function);
        }
        out.push(b'\n');
        diff.each_line(&hunk, |marker, line| {
            out.push(marker);
            out.extend_from_slice(line);
            if !line.ends_with(b"\n") {
                out.extend_from_slice(b"\n\\ No newline at end of file\n");
            }
        });
    }
}

/// Range of a hunk header: `start,count` with the count omitted when it is
/// one and the start shifted back when the range is empty.
fn range_text(range: &Range<u32>) -> String {
    let count = range.end - range.start;
    match count {
        0 => format!("{},0", range.start),
        1 => format!("{}", range.start + 1),
        _ => format!("{},{}", range.start + 1, count),
    }
}

/// Whether two modes describe the same kind of entry.
#[cfg(test)]
fn same_kind(a: EntryMode, b: EntryMode) -> bool {
    a.kind() == b.kind()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines_of(old: &str, new: &str) -> Vec<String> {
        let diff = LineDiff::new(old.as_bytes(), new.as_bytes());
        let mut out = Vec::new();
        for hunk in diff.hunks() {
            out.push(format!("@@ -{} +{} @@", range_text(&hunk.old), range_text(&hunk.new)));
            diff.each_line(&hunk, |marker, line| {
                out.push(format!("{}{}", marker as char, String::from_utf8_lossy(line).trim_end()));
            });
        }
        out
    }

    #[test]
    fn single_change_has_three_lines_of_context() {
        let old = "1\n2\n3\n4\n5\n6\n7\n8\n9\n";
        let new = "1\n2\n3\n4\nX\n6\n7\n8\n9\n";
        assert_eq!(
            lines_of(old, new),
            vec!["@@ -2,7 +2,7 @@", " 2", " 3", " 4", "-5", "+X", " 6", " 7", " 8"]
        );
    }

    #[test]
    fn changes_six_lines_apart_share_a_hunk() {
        let old = "a\n1\n2\n3\n4\n5\n6\nb\n";
        let new = "A\n1\n2\n3\n4\n5\n6\nB\n";
        let lines = lines_of(old, new);
        assert_eq!(lines.iter().filter(|line| line.starts_with("@@")).count(), 1);
    }

    #[test]
    fn changes_seven_lines_apart_use_two_hunks() {
        let old = "a\n1\n2\n3\n4\n5\n6\n7\nb\n";
        let new = "A\n1\n2\n3\n4\n5\n6\n7\nB\n";
        let lines = lines_of(old, new);
        assert_eq!(lines.iter().filter(|line| line.starts_with("@@")).count(), 2);
    }

    #[test]
    fn empty_old_range_starts_at_zero() {
        assert_eq!(lines_of("", "a\n")[0], "@@ -0,0 +1 @@");
    }

    #[test]
    fn white_space_is_removed_for_patch_ids() {
        assert_eq!(strip_space(b" a\tb \n"), b"ab".to_vec());
    }

    #[test]
    fn nul_bytes_mark_binary_content() {
        assert!(is_binary(b"a\0b"));
        assert!(!is_binary(b"text\n"));
    }

    #[test]
    fn function_context_is_the_nearest_identifier_line() {
        let old = "fn a() {\n  x\n  y\n  z\n  w\n  v\n}\n";
        let new = "fn a() {\n  x\n  y\n  z\n  w\n  V\n}\n";
        let diff = LineDiff::new(old.as_bytes(), new.as_bytes());
        let hunks = diff.hunks();
        assert_eq!(diff.function_context(&hunks[0]), Some(&b"fn a() {"[..]));
    }

    #[test]
    fn kinds_compare_by_entry_kind() {
        assert!(same_kind(EntryMode::from(gix_object::tree::EntryKind::Blob), EntryMode::from(gix_object::tree::EntryKind::Blob)));
    }
}
