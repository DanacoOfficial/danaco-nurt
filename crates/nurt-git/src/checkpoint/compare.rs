// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Comparing a checkpoint with another checkpoint, a revision or the
//! working tree.
//!
//! Hunks are computed with the Myers algorithm and the indent heuristic of
//! `git diff`, with the requested number of context lines.

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;
use std::path::Path;

use gix::bstr::{BString, ByteSlice};
use gix::hash::ObjectId;
use gix::imara::{Algorithm, Diff, InternedInput};
use gix::objs::tree::EntryKind;
use nurt_contracts::error::{ErrorCode, Result};
use nurt_contracts::record::{Change, ChangeAction, Hunk, HunkStatus};
use nurt_contracts::value::Path as RecordPath;

use super::snapshot::{Blobs, snapshot_worktree};
use super::support::{
    Filters, TreeEntry, display_path, flatten_tree, in_scope, open_repo, problem, repo_error,
    repo_relative, workdir,
};
use super::{parse_id, read_commit, resolve};

/// Bytes inspected when deciding whether content is binary, as `git` does.
const BINARY_PROBE: usize = 8000;

/// Marker line of content without a final line feed.
const NO_NEWLINE: &str = "\\ No newline at end of file";

/// Parameters of a comparison.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CompareRequest {
    /// Checkpoint on the old side, as `cp-<N>`.
    pub checkpoint: String,
    /// New side: a checkpoint or a revision; the working tree when absent.
    pub against: Option<String>,
    /// Report changed files with counts of lines instead of hunks.
    pub stat: bool,
    /// Lines of context around each change.
    pub context: u32,
    /// Files or directories to compare; all files when empty.
    pub paths: Vec<String>,
}

/// Result of a comparison.
#[derive(Debug, Clone, PartialEq)]
pub enum Comparison {
    /// Hunks of every changed file, in path order.
    Hunks(Vec<Hunk>),
    /// One change per changed file, in path order.
    Changes(Vec<Change>),
}

/// Compares checkpoint `request.checkpoint` with the new side of `request`.
///
/// The comparison reads the repository only: blobs of working tree files are
/// hashed in memory.
///
/// # Errors
///
/// Fails with `E_GIT_REF` and problem `not_found` for an unknown checkpoint.
/// Fails with `E_GIT_REVISION` when the new side is neither a checkpoint nor
/// a revision. Fails with `E_GIT_NOT_REPOSITORY` for a path outside the
/// working tree. Fails with `E_IO` when the repository cannot be read.
pub fn compare(repo_path: &Path, request: &CompareRequest) -> Result<Comparison> {
    let repo = open_repo(repo_path)?.with_object_memory();
    let root = workdir(&repo)?;
    let shown = display_path(repo_path);
    let scopes = request
        .paths
        .iter()
        .map(|path| repo_relative(&root, path))
        .collect::<Result<Vec<_>>>()?;
    let found = resolve(&repo, &request.checkpoint, &shown)?;
    let old = flatten_tree(&repo, read_commit(&repo, &found)?.tree)?;
    let new = match request.against.as_deref() {
        None => {
            let index = repo
                .index_or_empty()
                .map_err(|err| repo_error("read the index", err))?;
            let mut filters = Filters::new(&repo, &index)?;
            snapshot_worktree(&repo, &index, &mut filters, Blobs::Write)?.files
        }
        Some(id) if parse_id(id).is_some() => {
            let other = resolve(&repo, id, &shown)?;
            flatten_tree(&repo, read_commit(&repo, &other)?.tree)?
        }
        Some(revision) => flatten_tree(&repo, revision_tree(&repo, revision)?)?,
    };
    let changed = changed_paths(&old, &new, &scopes);
    if request.stat {
        let mut out = Vec::new();
        for path in changed {
            out.push(stat_of(&repo, &path, old.get(&path), new.get(&path))?);
        }
        return Ok(Comparison::Changes(out));
    }
    let mut out = Vec::new();
    for path in changed {
        out.extend(hunks_of(&repo, &path, old.get(&path), new.get(&path), request.context)?);
    }
    Ok(Comparison::Hunks(out))
}

fn revision_tree(repo: &gix::Repository, revision: &str) -> Result<ObjectId> {
    let not_found = |err: gix::Error| {
        problem(
            ErrorCode::GitRevision,
            "not_found",
            format!("`{revision}` is neither a checkpoint nor a revision: {err}"),
        )
    };
    let id = repo.rev_parse_single(revision).map_err(not_found)?;
    let object = id.object().map_err(not_found)?;
    let tree = object.peel_to_tree().map_err(not_found)?;
    Ok(tree.id)
}

fn changed_paths(
    old: &BTreeMap<BString, TreeEntry>,
    new: &BTreeMap<BString, TreeEntry>,
    scopes: &[BString],
) -> Vec<BString> {
    let all: BTreeSet<&BString> = old.keys().chain(new.keys()).collect();
    all.into_iter()
        .filter(|path| in_scope(path.as_bstr(), scopes) && old.get(*path) != new.get(*path))
        .cloned()
        .collect()
}

struct Sides {
    old: Vec<u8>,
    new: Vec<u8>,
    status: HunkStatus,
    old_mode: Option<String>,
    new_mode: Option<String>,
}

fn load_sides(
    repo: &gix::Repository,
    old: Option<&TreeEntry>,
    new: Option<&TreeEntry>,
) -> Result<Sides> {
    let read = |entry: Option<&TreeEntry>| -> Result<Vec<u8>> {
        match entry {
            Some((EntryKind::Commit, id)) => Ok(format!("Subproject commit {id}\n").into_bytes()),
            Some((_, id)) => repo
                .find_blob(*id)
                .map(|blob| blob.data.clone())
                .map_err(|err| repo_error(&format!("read blob {id}"), err)),
            None => Ok(Vec::new()),
        }
    };
    let old_bytes = read(old)?;
    let new_bytes = read(new)?;
    let binary = is_binary(&old_bytes) || is_binary(&new_bytes);
    let status = match (old, new, binary) {
        (_, _, true) => HunkStatus::Binary,
        (None, _, false) => HunkStatus::Added,
        (_, None, false) => HunkStatus::Deleted,
        _ => HunkStatus::Modified,
    };
    let mode = |entry: Option<&TreeEntry>| entry.map(|(kind, _)| format!("{:06o}", *kind as u32));
    Ok(Sides {
        old: old_bytes,
        new: new_bytes,
        status,
        old_mode: mode(old),
        new_mode: mode(new),
    })
}

fn is_binary(content: &[u8]) -> bool {
    content[..content.len().min(BINARY_PROBE)].contains(&0)
}

fn stat_of(
    repo: &gix::Repository,
    path: &BString,
    old: Option<&TreeEntry>,
    new: Option<&TreeEntry>,
) -> Result<Change> {
    let sides = load_sides(repo, old, new)?;
    let action = match (old, new) {
        (None, _) => ChangeAction::Create,
        (_, None) => ChangeAction::Delete,
        _ => ChangeAction::Modify,
    };
    let (added, removed, flags) = if sides.status == HunkStatus::Binary {
        (None, None, Some(vec!["binary".to_owned()]))
    } else {
        let input = InternedInput::new(sides.old.as_slice(), sides.new.as_slice());
        let diff = line_diff(&input);
        (
            Some(u64::from(diff.count_additions())),
            Some(u64::from(diff.count_removals())),
            None,
        )
    };
    Ok(Change {
        path: RecordPath::new(path.to_str_lossy().into_owned()),
        action,
        added,
        removed,
        flags,
        ..Change::default()
    })
}

fn line_diff(input: &InternedInput<&[u8]>) -> Diff {
    let mut diff = Diff::compute(Algorithm::Myers, input);
    diff.postprocess_lines(input);
    diff
}

fn hunks_of(
    repo: &gix::Repository,
    path: &BString,
    old: Option<&TreeEntry>,
    new: Option<&TreeEntry>,
    context: u32,
) -> Result<Vec<Hunk>> {
    let sides = load_sides(repo, old, new)?;
    let record_path = RecordPath::new(path.to_str_lossy().into_owned());
    let modes = (sides.old_mode.clone(), sides.new_mode.clone());
    let base = Hunk {
        path: record_path,
        status: Some(sides.status),
        old_mode: modes.0.filter(|old| Some(old) != modes.1.as_ref()),
        new_mode: modes.1.filter(|new| Some(new) != sides.old_mode.as_ref()),
        ..Hunk::default()
    };
    if sides.status == HunkStatus::Binary {
        return Ok(vec![Hunk {
            binary: Some(true),
            id: Some(format!("{path}#1")),
            ..base
        }]);
    }
    let input = InternedInput::new(sides.old.as_slice(), sides.new.as_slice());
    let diff = line_diff(&input);
    let changes: Vec<(Range<u32>, Range<u32>)> =
        diff.hunks().map(|hunk| (hunk.before, hunk.after)).collect();
    if changes.is_empty() {
        return Ok(vec![Hunk {
            id: Some(format!("{path}#1")),
            ..base
        }]);
    }
    let old_lines: Vec<&[u8]> = sides.old.lines_with_terminator().collect();
    let new_lines: Vec<&[u8]> = sides.new.lines_with_terminator().collect();
    let groups = group_changes(&changes, context, old_lines.len());
    let mut out = Vec::new();
    for (number, group) in groups.iter().enumerate() {
        let lines = render_group(group, &changes, &old_lines, &new_lines, context);
        out.push(Hunk {
            old_start: header_start(lines.old.start, lines.old.len()),
            old_len: lines.old.len() as u64,
            new_start: header_start(lines.new.start, lines.new.len()),
            new_len: lines.new.len() as u64,
            lines: lines.text,
            id: Some(format!("{path}#{}", number + 1)),
            ..base.clone()
        });
    }
    Ok(out)
}

/// Start line of a hunk header: from 1, or the line before an empty range.
fn header_start(start: usize, len: usize) -> u64 {
    if len == 0 {
        start as u64
    } else {
        start as u64 + 1
    }
}

/// Groups changes whose context overlaps or touches, as unified diffs do.
fn group_changes(
    changes: &[(Range<u32>, Range<u32>)],
    context: u32,
    old_len: usize,
) -> Vec<Range<usize>> {
    let mut groups: Vec<Range<usize>> = Vec::new();
    for (index, (before, _)) in changes.iter().enumerate() {
        if let Some(last) = groups.last_mut() {
            let previous_end = changes[last.end - 1].0.end;
            if before.start.saturating_sub(previous_end) <= 2 * context {
                last.end = index + 1;
                continue;
            }
        }
        groups.push(index..index + 1);
    }
    let _ = old_len;
    groups
}

struct GroupLines {
    old: Range<usize>,
    new: Range<usize>,
    text: Vec<String>,
}

fn render_group(
    group: &Range<usize>,
    changes: &[(Range<u32>, Range<u32>)],
    old_lines: &[&[u8]],
    new_lines: &[&[u8]],
    context: u32,
) -> GroupLines {
    let context = context as usize;
    let first = &changes[group.start];
    let last = &changes[group.end - 1];
    let old_start = (first.0.start as usize).saturating_sub(context);
    let old_end = (last.0.end as usize + context).min(old_lines.len());
    let lead = first.0.start as usize - old_start;
    let new_start = first.1.start as usize - lead;
    let trail = old_end - last.0.end as usize;
    let new_end = last.1.end as usize + trail;
    let mut text = Vec::new();
    let mut old_at = old_start;
    for (before, after) in &changes[group.clone()] {
        while old_at < before.start as usize {
            push_line(&mut text, ' ', old_lines[old_at]);
            old_at += 1;
        }
        for line in &old_lines[before.start as usize..before.end as usize] {
            push_line(&mut text, '-', line);
        }
        for line in &new_lines[after.start as usize..after.end as usize] {
            push_line(&mut text, '+', line);
        }
        old_at = before.end as usize;
    }
    while old_at < old_end {
        push_line(&mut text, ' ', old_lines[old_at]);
        old_at += 1;
    }
    GroupLines {
        old: old_start..old_end,
        new: new_start..new_end.min(new_lines.len()),
        text,
    }
}

fn push_line(out: &mut Vec<String>, prefix: char, line: &[u8]) {
    let content = line.strip_suffix(b"\n").unwrap_or(line);
    let content = content.strip_suffix(b"\r").map_or(content, |trimmed| {
        if line.ends_with(b"\r\n") { trimmed } else { content }
    });
    let mut text = String::with_capacity(content.len() + 1);
    text.push(prefix);
    text.push_str(&String::from_utf8_lossy(content));
    if line.ends_with(b"\r\n") {
        text.push('\r');
    }
    out.push(text);
    if !line.ends_with(b"\n") {
        out.push(NO_NEWLINE.to_owned());
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_journal::{Fixture, RecordingJournal, test_ctx};
    use super::super::{CreateRequest, apply_create, plan_create};
    use super::*;

    fn checkpoint(fixture: &Fixture, journal: &RecordingJournal) {
        let plan = plan_create(&fixture.root, &CreateRequest::default(), &test_ctx()).unwrap();
        let mut tx = journal.begin(fixture.base());
        apply_create(&plan, tx.as_mut()).unwrap();
        journal.commit(tx);
    }

    fn request(against: Option<&str>, stat: bool, paths: &[&str]) -> CompareRequest {
        CompareRequest {
            checkpoint: "cp-1".into(),
            against: against.map(str::to_owned),
            stat,
            context: 3,
            paths: paths.iter().map(|path| (*path).to_owned()).collect(),
        }
    }

    fn numbered(count: usize, changed: &[(usize, &str)]) -> String {
        (1..=count)
            .map(|line| {
                changed
                    .iter()
                    .find(|(at, _)| *at == line)
                    .map_or_else(|| format!("line {line}\n"), |(_, text)| format!("{text}\n"))
            })
            .collect()
    }

    #[test]
    fn compare_with_working_tree_produces_unified_hunks() {
        let fixture = Fixture::new(&[("src/a.ts", "export const a = 2;\nexport const b = 1;\n")]);
        let journal = RecordingJournal::default();
        checkpoint(&fixture, &journal);
        fixture.write("src/a.ts", "export const a = 3;\nexport const b = 1;\n");
        let Comparison::Hunks(hunks) = compare(&fixture.root, &request(None, false, &[])).unwrap() else {
            panic!("expected hunks");
        };
        assert_eq!(hunks.len(), 1);
        let hunk = &hunks[0];
        assert_eq!((hunk.old_start, hunk.old_len, hunk.new_start, hunk.new_len), (1, 2, 1, 2));
        assert_eq!(
            hunk.lines,
            ["-export const a = 2;", "+export const a = 3;", " export const b = 1;"]
        );
        assert_eq!(hunk.id.as_deref(), Some("src/a.ts#1"));
        assert_eq!(hunk.status, Some(HunkStatus::Modified));
    }

    #[test]
    fn distant_changes_form_separate_hunks_with_context() {
        let original = numbered(30, &[]);
        let fixture = Fixture::new(&[("long.txt", original.as_str())]);
        let journal = RecordingJournal::default();
        checkpoint(&fixture, &journal);
        fixture.write("long.txt", &numbered(30, &[(2, "two"), (29, "twenty-nine")]));
        let Comparison::Hunks(hunks) = compare(&fixture.root, &request(None, false, &[])).unwrap() else {
            panic!("expected hunks");
        };
        assert_eq!(hunks.len(), 2);
        assert_eq!((hunks[0].old_start, hunks[0].old_len), (1, 5));
        assert_eq!((hunks[1].old_start, hunks[1].old_len, hunks[1].new_start), (26, 5, 26));
        assert_eq!(hunks[1].id.as_deref(), Some("long.txt#2"));
        assert_eq!(hunks[1].lines[3], "-line 29");
    }

    #[test]
    fn stat_lists_created_deleted_and_modified_files_within_paths() {
        let fixture = Fixture::new(&[("src/a.txt", "one\n"), ("src/b.txt", "gone\n"), ("c.txt", "c\n")]);
        let journal = RecordingJournal::default();
        checkpoint(&fixture, &journal);
        fixture.write("src/a.txt", "one\ntwo\n");
        fixture.remove("src/b.txt");
        fixture.write("src/new.txt", "new\n");
        fixture.write("c.txt", "changed\n");
        let Comparison::Changes(changes) = compare(&fixture.root, &request(None, true, &["src"])).unwrap() else {
            panic!("expected changes");
        };
        let summary: Vec<(String, ChangeAction, Option<u64>, Option<u64>)> = changes
            .into_iter()
            .map(|change| (change.path.as_str().to_owned(), change.action, change.added, change.removed))
            .collect();
        assert_eq!(
            summary,
            [
                ("src/a.txt".to_owned(), ChangeAction::Modify, Some(1), Some(0)),
                ("src/b.txt".to_owned(), ChangeAction::Delete, Some(0), Some(1)),
                ("src/new.txt".to_owned(), ChangeAction::Create, Some(1), Some(0)),
            ]
        );
    }

    #[test]
    fn compare_against_checkpoint_and_revision() {
        let fixture = Fixture::new(&[("a.txt", "one\n")]);
        let journal = RecordingJournal::default();
        checkpoint(&fixture, &journal);
        fixture.write("a.txt", "two\n");
        checkpoint(&fixture, &journal);
        let Comparison::Changes(changes) = compare(&fixture.root, &request(Some("cp-2"), true, &[])).unwrap() else {
            panic!("expected changes");
        };
        assert_eq!(changes.len(), 1);
        let Comparison::Hunks(hunks) = compare(&fixture.root, &request(Some("HEAD"), false, &[])).unwrap() else {
            panic!("expected hunks");
        };
        assert!(hunks.is_empty());
        let err = compare(&fixture.root, &request(Some("no-such-rev"), false, &[])).unwrap_err();
        assert_eq!(err.code(), ErrorCode::GitRevision);
        let err = compare(&fixture.root, &request(Some("cp-9"), false, &[])).unwrap_err();
        assert_eq!(err.code(), ErrorCode::GitRef);
    }

    #[test]
    fn missing_final_newline_and_binary_content_are_marked() {
        let fixture = Fixture::new(&[("t.txt", "a\nb"), ("bin.dat", "x")]);
        let journal = RecordingJournal::default();
        checkpoint(&fixture, &journal);
        fixture.write("t.txt", "a\nc");
        fixture.write_bytes("bin.dat", b"\0\x01");
        let Comparison::Hunks(hunks) = compare(&fixture.root, &request(None, false, &[])).unwrap() else {
            panic!("expected hunks");
        };
        let binary = hunks.iter().find(|hunk| hunk.path.as_str() == "bin.dat").unwrap();
        assert_eq!(binary.binary, Some(true));
        let text = hunks.iter().find(|hunk| hunk.path.as_str() == "t.txt").unwrap();
        assert_eq!(text.lines, [" a", "-b", NO_NEWLINE, "+c", NO_NEWLINE]);
    }

    #[test]
    fn header_start_of_empty_range_is_the_line_before() {
        assert_eq!(header_start(0, 0), 0);
        assert_eq!(header_start(4, 0), 4);
        assert_eq!(header_start(4, 2), 5);
    }
}
