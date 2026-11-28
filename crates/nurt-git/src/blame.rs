// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-git::blame
// Responsibility:   The `git.blame` command.
// Dependencies:     bstr, gix, nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! The `git.blame` command.
//!
//! Each line of a file is annotated with the commit that last changed it,
//! as `git blame` finds it: the history is followed through renames of the
//! whole file, while lines moved or copied between files are attributed to
//! the commit that moved them. Without `revision` the file of the working
//! tree is annotated, and its lines that differ from `HEAD` belong to no
//! commit yet. Root commits are reported as boundaries.

use std::borrow::Cow;
use std::collections::HashMap;

use bstr::{BString, ByteSlice};
use gix::ObjectId;
use gix::objs::tree::EntryKind;
use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::record::BlameLine;
use nurt_contracts::stage::{
    BoundArgs, ExecCtx, InputKinds, ReadStage, RecordStream, StageDescriptor, StageImpl,
    StageInput, StageOutput,
};
use nurt_contracts::value::{Instant, Range, Record};

use crate::diff::{Reader, open_repository};
use crate::revwalk::args;
use crate::revwalk::lines;
use crate::revwalk::moves::{Moves, Piece};
use crate::revwalk::paths::entry_at;
use crate::revwalk::record::{CommitRecords, instant};
use crate::revwalk::repo::{OpenRepo, quote};

const COMMAND: &str = "git.blame";

/// Author of lines that are not committed, as `git blame` names it.
const UNCOMMITTED_AUTHOR: &str = "Not Committed Yet";

/// E-mail of lines that are not committed, as `git blame` writes it.
const UNCOMMITTED_EMAIL: &str = "not.committed.yet";

/// Hexadecimal digits of an abbreviated identifier of no commit.
const SHORT_LEN: usize = 7;

/// Configuration keys of diff drivers that name programs.
const DIFF_PROGRAM_KEYS: [&str; 2] = ["textconv", "command"];

/// Implementation of `git.blame`.
///
/// # Examples
///
/// ```
/// use nurt_contracts::stage::StageImpl;
/// use nurt_git::blame::GitBlame;
///
/// let descriptor = GitBlame.descriptor();
/// assert_eq!(descriptor.command, "git.blame");
/// assert!(descriptor.source);
/// ```
#[derive(Debug, Clone, Copy, Default)]
pub struct GitBlame;

impl StageImpl for GitBlame {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: COMMAND.to_owned(),
            source: true,
            input: InputKinds::None,
        }
    }

    fn as_read(&self) -> Option<&dyn ReadStage> {
        Some(self)
    }
}

impl ReadStage for GitBlame {
    /// Annotates the lines of `path` as `blame_line` records, in line order.
    ///
    /// # Errors
    ///
    /// Fails with `E_GIT_NOT_REPOSITORY` when `path` is not inside a
    /// repository, `E_GIT_REVISION` for an unknown revision, `E_NOT_FOUND`
    /// when the file does not exist at the revision or is not tracked,
    /// `E_INVALID_VALUE` for a directory or for `lines` outside the file,
    /// `E_IO` when the repository cannot be read and `E_CANCELLED` when the
    /// program is stopped.
    fn execute<'a>(
        &'a self,
        args: &'a BoundArgs,
        _input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> nurt_contracts::error::Result<StageOutput<'a>> {
        let query = Query::from_args(args)?;
        let repo = OpenRepo::discover(&query.path, ctx)?;
        let relative = repo.relative(&query.path)?;
        if relative.is_empty() {
            return Err(args::invalid(
                args,
                "path",
                format!(
                    "`{}` is the root of the repository, give a file",
                    query.path
                ),
            ));
        }
        let start = Start::find(&repo, &query, &relative, ctx)?;
        let records = blame(&repo, &query, &relative, start, ctx)?;
        Ok(StageOutput::records(RecordStream::from_records(records)))
    }
}

struct Query {
    path: String,
    lines: Option<Range>,
    /// The revision given by the program; absent when the working tree is
    /// annotated.
    revision: Option<String>,
    /// Lines moved within the file are attributed to the commit that wrote
    /// them (`-M`).
    find_moves: bool,
    /// Lines copied from other files changed by the same commit are
    /// attributed to the commit that wrote them (`-C`); implies `find_moves`.
    find_copies: bool,
}

impl Query {
    fn from_args(bound: &BoundArgs) -> Result<Self, Error> {
        let revision = if bound.is_defaulted("revision") {
            None
        } else {
            args::text(bound, "revision")?
        };
        let find_copies = args::flag(bound, "find_copies")?;
        Ok(Self {
            path: args::required_path(bound, "path")?,
            lines: args::range(bound, "lines")?,
            revision,
            find_moves: find_copies || args::flag(bound, "find_moves")?,
            find_copies,
        })
    }
}

/// What is annotated.
enum Start {
    /// The file in a commit.
    Commit { id: ObjectId, content: Vec<u8> },
    /// The file of the working tree, compared first with the commit of
    /// `HEAD` when the file is there.
    Worktree {
        content: Vec<u8>,
        head: Option<ObjectId>,
    },
}

impl Start {
    fn content(&self) -> &[u8] {
        match self {
            Self::Commit { content, .. } | Self::Worktree { content, .. } => content,
        }
    }

    fn find(
        repo: &OpenRepo,
        query: &Query,
        relative: &BString,
        ctx: &ExecCtx,
    ) -> Result<Self, Error> {
        if let Some(revision) = &query.revision {
            let id = repo.resolve_commit(revision)?;
            return match file_at(repo, id, relative, &query.path, revision)? {
                Some(content) => Ok(Self::Commit { id, content }),
                None => Err(not_found(repo, &query.path, revision)),
            };
        }
        let head = repo.repo.head_id().ok().map(gix::Id::detach);
        let committed = match head {
            Some(id) => file_at(repo, id, relative, &query.path, "HEAD")?,
            None => None,
        };
        let worktree = worktree_file(repo, relative, ctx)?;
        match (worktree, committed, head) {
            (Some(content), Some(_), head) => Ok(Self::Worktree { content, head }),
            (Some(content), None, _) if in_index(repo, relative) => Ok(Self::Worktree {
                content,
                head: None,
            }),
            (None, Some(content), Some(id)) => Ok(Self::Commit { id, content }),
            _ => Err(not_tracked(repo, &query.path)),
        }
    }
}

/// Content of the file at `relative` in a commit.
fn file_at(
    repo: &OpenRepo,
    commit: ObjectId,
    relative: &BString,
    given: &str,
    revision: &str,
) -> Result<Option<Vec<u8>>, Error> {
    let tree = repo.tree_of(commit)?;
    let entry = entry_at(repo, tree, relative.as_ref())
        .map_err(|err| repo.read_error(format_args!("`{given}`"), err))?;
    match entry {
        None => Ok(None),
        Some((mode, _)) if mode.kind() == EntryKind::Tree => Err(Error::new(
            ErrorCode::InvalidValue,
            format!("`{given}` is a directory at `{revision}`, give a file"),
        )
        .with_found(format!("{COMMAND} path:"))),
        Some((mode, _)) if mode.kind() == EntryKind::Commit => Err(Error::new(
            ErrorCode::InvalidValue,
            format!("`{given}` is a submodule at `{revision}`, give a file"),
        )
        .with_found(format!("{COMMAND} path:"))),
        Some((_, blob)) => repo.blob(blob).map(Some),
    }
}

/// The file of the working tree in the form stored in the repository; no
/// filter program is started.
fn worktree_file(
    repo: &OpenRepo,
    relative: &BString,
    ctx: &ExecCtx,
) -> Result<Option<Vec<u8>>, Error> {
    let Some(root) = repo.workdir() else {
        return Ok(None);
    };
    let opened = open_repository(root, ctx)?;
    for note in opened.notes {
        ctx.note(note);
    }
    let mut reader = Reader::new(opened.repo.gix())?;
    Ok(reader
        .worktree(relative.as_ref(), None)?
        .map(|(content, _)| content))
}

fn in_index(repo: &OpenRepo, relative: &BString) -> bool {
    repo.repo
        .index_or_empty()
        .is_ok_and(|index| index.entry_by_path(relative.as_bstr()).is_some())
}

fn not_found(repo: &OpenRepo, path: &str, revision: &str) -> Error {
    Error::new(
        ErrorCode::NotFound,
        format!("`{path}` does not exist at `{revision}`"),
    )
    .with_found(path)
    .with_next_step(format!(
        "git.log {} path: {} follow: true count: 5",
        quote(repo.label()),
        quote(path)
    ))
}

fn not_tracked(repo: &OpenRepo, path: &str) -> Error {
    Error::new(
        ErrorCode::NotFound,
        format!("`{path}` is not tracked in the repository, so it has no history"),
    )
    .with_found(path)
    .with_next_step(format!("git.status {}", quote(repo.label())))
}

/// First and last line, from 1, of the lines to annotate in a file of
/// `total` lines; the end is cut to the file.
fn line_bounds(bound: Option<Range>, total: u32, path: &str) -> Result<(u32, u32), Error> {
    let Some(range) = bound else {
        return Ok((1, total));
    };
    let start = range.start().unwrap_or(1);
    let end = range.end().unwrap_or(i64::from(total));
    if start < 1 || end < start {
        return Err(Error::new(
            ErrorCode::InvalidValue,
            format!("`lines` must count from 1 and end at or after its start, not {start}..{end}"),
        )
        .with_found(format!("{COMMAND} lines:")));
    }
    if start > i64::from(total) {
        return Err(Error::new(
            ErrorCode::InvalidValue,
            format!("`lines` starts at line {start}, but `{path}` has {total} lines"),
        )
        .with_found(format!("{COMMAND} lines:"))
        .with_fix(format!("{COMMAND} {} lines: 1..{total}", quote(path))));
    }
    let start = u32::try_from(start).unwrap_or(u32::MAX);
    let end = u32::try_from(end.min(i64::from(total))).unwrap_or(total);
    Ok((start, end))
}

/// Author, time and boundary of one commit.
struct Origin {
    id: String,
    short_id: String,
    author: String,
    email: String,
    time: Instant,
    boundary: bool,
}

fn blame(
    repo: &OpenRepo,
    query: &Query,
    relative: &BString,
    start: Start,
    ctx: &ExecCtx,
) -> Result<Vec<Record>, Error> {
    let content = start.content();
    let total = u32::try_from(lines::split_lines(content).len()).unwrap_or(u32::MAX);
    if total == 0 {
        return Ok(Vec::new());
    }
    let (first, last) = line_bounds(query.lines, total, &query.path)?;
    let path = relative.to_str_lossy().into_owned();
    let start = match start {
        Start::Worktree {
            content,
            head: None,
        } => {
            return Ok(uncommitted_lines(&content, &path, first, last, ctx));
        }
        Start::Worktree {
            content,
            head: Some(head),
        } => gix_blame::Start::Contents {
            first_suspect: head,
            contents: Cow::Owned(content),
        },
        Start::Commit { id, .. } => gix_blame::Start::Commit(id),
    };
    let reading = |err: &dyn std::fmt::Display| {
        repo.read_error(format_args!("the history of `{}`", query.path), err)
    };
    let source = without_diff_programs(&repo.repo).map_err(|err| reading(&err))?;
    let mut cache = source
        .diff_resource_cache_for_tree_diff()
        .map_err(|err| reading(&err))?;
    let ranges = gix_blame::BlameRanges::from_one_based_inclusive_range(first..=last)
        .map_err(|err| reading(&err))?;
    let options = gix_blame::Options {
        diff_algorithm: source
            .diff_algorithm()
            .unwrap_or(gix::diff::blob::Algorithm::Myers),
        ranges,
        since: None,
        rewrites: Some(gix::diff::Rewrites::default()),
        debug_track_path: false,
    };
    let graph = source.commit_graph_if_enabled().ok().flatten();
    let outcome = gix_blame::file(
        &source.objects,
        start,
        graph,
        &mut cache,
        relative.as_bstr(),
        options,
    )
    .map_err(|err| reading(&err))?;
    let mut pieces: Vec<Piece> = outcome
        .entries
        .iter()
        .map(|entry| Piece {
            blamed_start: entry.start_in_blamed_file,
            len: entry.len.get(),
            commit: entry.commit_id,
            path: entry
                .source_file_name
                .clone()
                .unwrap_or_else(|| relative.clone()),
            source_start: entry.start_in_source_file,
        })
        .collect();
    if query.find_moves {
        pieces = Moves::new(repo, &source, &mut cache, query.find_copies).refine(pieces, 0)?;
    }
    let records = CommitRecords::new(repo, &ctx.demand().fields);
    let blamed = lines::split_lines(&outcome.blob);
    let mut origins: HashMap<ObjectId, Origin> = HashMap::new();
    let mut out: Vec<BlameLine> = Vec::new();
    for entry in &pieces {
        ctx.checkpoint()?;
        if let std::collections::hash_map::Entry::Vacant(e) = origins.entry(entry.commit) {
            let origin = origin_of(&records, entry.commit, ctx)?;
            e.insert(origin);
        }
        let Some(origin) = origins.get(&entry.commit) else {
            continue;
        };
        let source_path = entry.path.to_str_lossy().into_owned();
        for offset in 0..entry.len {
            let line = entry.blamed_start + offset;
            out.push(BlameLine {
                path: path.clone(),
                no: u64::from(line) + 1,
                commit: origin.id.clone(),
                short_id: origin.short_id.clone(),
                author: origin.author.clone(),
                time: origin.time,
                text: blamed
                    .get(line as usize)
                    .map(|text| lines::line_text(text))
                    .unwrap_or_default(),
                author_email: Some(origin.email.clone()),
                original_path: Some(source_path.clone()),
                original_no: Some(u64::from(entry.source_start + offset) + 1),
                boundary: Some(origin.boundary),
            });
        }
    }
    out.sort_by_key(|line| line.no);
    Ok(out.into_iter().map(Record::from).collect())
}

fn origin_of(records: &CommitRecords<'_>, id: ObjectId, ctx: &ExecCtx) -> Result<Origin, Error> {
    if id.is_null() {
        return Ok(uncommitted_origin(id, ctx));
    }
    let repo = records.repo();
    let author = records.author(id)?;
    let info = repo
        .commits
        .info(&repo.repo, id)
        .map_err(|err| repo.read_error(format_args!("commit {id}"), err))?;
    Ok(Origin {
        id: id.to_string(),
        short_id: repo.short_id(id),
        author: author.name,
        email: author.email,
        time: instant(author.seconds),
        boundary: info.parents.is_empty(),
    })
}

fn uncommitted_origin(id: ObjectId, ctx: &ExecCtx) -> Origin {
    let nanos = ctx.clock().now().unix_nanos();
    Origin {
        id: id.to_string(),
        short_id: id.to_hex_with_len(SHORT_LEN).to_string(),
        author: UNCOMMITTED_AUTHOR.to_owned(),
        email: UNCOMMITTED_EMAIL.to_owned(),
        time: Instant::from_unix_nanos(i64::try_from(nanos).unwrap_or(i64::MAX)),
        boundary: false,
    }
}

/// Lines of a file that no commit has, all attributed to no commit.
fn uncommitted_lines(
    content: &[u8],
    path: &str,
    first: u32,
    last: u32,
    ctx: &ExecCtx,
) -> Vec<Record> {
    let origin = uncommitted_origin(ObjectId::null(gix::hash::Kind::Sha1), ctx);
    lines::split_lines(content)
        .iter()
        .enumerate()
        .skip(first as usize - 1)
        .take((last - first + 1) as usize)
        .map(|(index, text)| {
            let no = index as u64 + 1;
            Record::from(BlameLine {
                path: path.to_owned(),
                no,
                commit: origin.id.clone(),
                short_id: origin.short_id.clone(),
                author: origin.author.clone(),
                time: origin.time,
                text: lines::line_text(text),
                author_email: Some(origin.email.clone()),
                original_path: Some(path.to_owned()),
                original_no: Some(no),
                boundary: Some(false),
            })
        })
        .collect()
}

/// A copy of the repository whose diff drivers that name programs are
/// removed, so that no conversion program is started while lines are
/// compared.
fn without_diff_programs(repo: &gix::Repository) -> Result<gix::Repository, String> {
    let mut copy = repo.clone();
    let mut snapshot = copy.config_snapshot_mut();
    let ids: Vec<_> = snapshot
        .sections_and_ids_by_name("diff")
        .map(|sections| {
            sections
                .filter(|(section, _)| {
                    section.header().subsection_name().is_some()
                        && DIFF_PROGRAM_KEYS
                            .iter()
                            .any(|key| section.value(key).is_some())
                })
                .map(|(_, id)| id)
                .collect()
        })
        .unwrap_or_default();
    for id in ids {
        snapshot.remove_section_by_id(id);
    }
    snapshot.commit().map_err(|err| err.to_string())?;
    Ok(copy)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::revwalk::fixture::{Fixture, args as bound, exec_ctx, run as execute};
    use nurt_contracts::value::Value;

    fn blame_lines(fixture: &Fixture, file: &str, values: &[(&str, Value)]) -> Vec<BlameLine> {
        let mut all = vec![("path", Value::Path(fixture.path(file)))];
        all.extend(values.iter().cloned());
        execute(&GitBlame, &bound(COMMAND, &all), &exec_ctx(None))
            .unwrap()
            .into_iter()
            .map(|record| BlameLine::try_from(record).unwrap())
            .collect()
    }

    fn blame_error(fixture: &Fixture, file: &str, values: &[(&str, Value)]) -> ErrorCode {
        let mut all = vec![("path", Value::Path(fixture.path(file)))];
        all.extend(values.iter().cloned());
        execute(&GitBlame, &bound(COMMAND, &all), &exec_ctx(None))
            .unwrap_err()
            .code()
    }

    fn text(value: &str) -> Value {
        Value::Text(value.to_owned())
    }

    fn commits(lines: &[BlameLine]) -> Vec<String> {
        lines.iter().map(|line| line.commit.clone()).collect()
    }

    #[test]
    fn attributes_each_line_to_the_commit_that_changed_it() {
        let mut fixture = Fixture::new();
        let first = fixture.commit("first", &[("a.txt", Some("one\ntwo\nthree\n"))]);
        let second = fixture.commit("second", &[("a.txt", Some("one\nTWO\nthree\n"))]);
        let lines = blame_lines(&fixture, "a.txt", &[("revision", text("HEAD"))]);
        assert_eq!(
            commits(&lines),
            [first.to_string(), second.to_string(), first.to_string()]
        );
        assert_eq!(lines[1].text, "TWO");
        assert_eq!(lines[1].no, 2);
        assert_eq!(lines[1].author, "Author");
        assert_eq!(lines[1].author_email.as_deref(), Some("author@example.com"));
        assert_eq!(lines[1].time, instant(fixture.time_of(second)));
        assert_eq!(lines[0].boundary, Some(true));
        assert_eq!(lines[1].boundary, Some(false));
        assert_eq!(lines[0].short_id.len(), SHORT_LEN);
        let worktree = blame_lines(&fixture, "a.txt", &[]);
        assert_eq!(commits(&worktree), commits(&lines));
        let older = blame_lines(&fixture, "a.txt", &[("revision", text("HEAD~1"))]);
        assert_eq!(commits(&older), vec![first.to_string(); 3]);
    }

    #[test]
    fn follows_renames_of_the_whole_file() {
        let mut fixture = Fixture::new();
        let text_lines = "one\ntwo\nthree\nfour\nfive\nsix\n";
        let first = fixture.commit("first", &[("old.txt", Some(text_lines))]);
        let second = fixture.commit(
            "rename",
            &[
                ("old.txt", None),
                ("new.txt", Some("one\ntwo\nthree\nfour\nfive\nsix\nseven\n")),
            ],
        );
        let lines = blame_lines(&fixture, "new.txt", &[("revision", text("HEAD"))]);
        assert_eq!(lines[0].commit, first.to_string());
        assert_eq!(lines[0].original_path.as_deref(), Some("old.txt"));
        assert_eq!(lines[6].commit, second.to_string());
        assert_eq!(lines[6].original_path.as_deref(), Some("new.txt"));
        assert_eq!(lines[6].path, "new.txt");
    }

    #[test]
    fn changes_of_the_working_tree_are_not_committed_yet() {
        let mut fixture = Fixture::new();
        let first = fixture.commit("first", &[("a.txt", Some("one\ntwo\n"))]);
        fixture.write_file("a.txt", "one\nchanged\n");
        let lines = blame_lines(&fixture, "a.txt", &[]);
        assert_eq!(lines[0].commit, first.to_string());
        assert!(lines[1].commit.chars().all(|ch| ch == '0'));
        assert_eq!(lines[1].author, UNCOMMITTED_AUTHOR);
        assert_eq!(lines[1].short_id, "0000000");
        let committed = blame_lines(&fixture, "a.txt", &[("revision", text("HEAD"))]);
        assert_eq!(committed[1].text, "two");
    }

    #[test]
    fn limits_the_lines() {
        let mut fixture = Fixture::new();
        fixture.commit("first", &[("a.txt", Some("1\n2\n3\n4\n5\n"))]);
        let range = Value::Range(Range::new(Some(2), Some(3)).unwrap());
        let lines = blame_lines(
            &fixture,
            "a.txt",
            &[("revision", text("HEAD")), ("lines", range)],
        );
        assert_eq!(lines.iter().map(|line| line.no).collect::<Vec<_>>(), [2, 3]);
        let open = Value::Range(Range::new(Some(4), Some(40)).unwrap());
        assert_eq!(blame_lines(&fixture, "a.txt", &[("lines", open)]).len(), 2);
        let single = blame_lines(&fixture, "a.txt", &[("lines", Value::Int(5))]);
        assert_eq!(single[0].text, "5");
        let beyond = Value::Range(Range::new(Some(9), Some(10)).unwrap());
        assert_eq!(
            blame_error(&fixture, "a.txt", &[("lines", beyond)]),
            ErrorCode::InvalidValue
        );
        let zero = Value::Range(Range::new(Some(0), Some(1)).unwrap());
        assert_eq!(
            blame_error(&fixture, "a.txt", &[("lines", zero)]),
            ErrorCode::InvalidValue
        );
    }

    #[test]
    fn strips_carriage_returns_from_line_text() {
        let mut fixture = Fixture::new();
        fixture.commit("crlf", &[("w.txt", Some("a\r\nb\r\n"))]);
        let lines = blame_lines(&fixture, "w.txt", &[("revision", text("HEAD"))]);
        assert_eq!(
            lines
                .iter()
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>(),
            ["a", "b"]
        );
    }

    #[test]
    fn rejects_missing_untracked_and_directory_paths() {
        let mut fixture = Fixture::new();
        fixture.commit("first", &[("dir/a.txt", Some("a\n"))]);
        fixture.write_file("untracked.txt", "u\n");
        assert_eq!(
            blame_error(&fixture, "missing.txt", &[("revision", text("HEAD"))]),
            ErrorCode::NotFound
        );
        assert_eq!(
            blame_error(&fixture, "untracked.txt", &[]),
            ErrorCode::NotFound
        );
        assert_eq!(
            blame_error(&fixture, "dir", &[("revision", text("HEAD"))]),
            ErrorCode::InvalidValue
        );
        assert_eq!(
            blame_error(&fixture, "dir/a.txt", &[("revision", text("nothing"))]),
            ErrorCode::GitRevision
        );
        let root = vec![("path", Value::Path(fixture.root()))];
        assert_eq!(
            execute(&GitBlame, &bound(COMMAND, &root), &exec_ctx(None))
                .unwrap_err()
                .code(),
            ErrorCode::InvalidValue
        );
    }

    const BLOCK: &str =
        "first block line with text\nsecond block line with text\nthird block line with text\n";

    #[test]
    fn moved_lines_belong_to_the_commit_that_wrote_them_with_find_moves() {
        let mut fixture = Fixture::new();
        let first = fixture.commit(
            "first",
            &[(
                "a.txt",
                Some(&format!("{BLOCK}middle\nfiller one\nfiller two\n")),
            )],
        );
        let second = fixture.commit(
            "move",
            &[(
                "a.txt",
                Some(&format!("middle\nfiller one\nfiller two\n{BLOCK}")),
            )],
        );
        let plain = blame_lines(&fixture, "a.txt", &[("revision", text("HEAD"))]);
        assert_eq!(plain[3].commit, second.to_string());
        let moved = blame_lines(
            &fixture,
            "a.txt",
            &[
                ("revision", text("HEAD")),
                ("find_moves", Value::Bool(true)),
            ],
        );
        assert_eq!(moved[3].commit, first.to_string());
        assert_eq!(moved[3].original_no, Some(1));
        assert_eq!(moved[5].original_no, Some(3));
        assert_eq!(moved[0].commit, first.to_string());
    }

    #[test]
    fn copied_lines_belong_to_the_commit_that_wrote_them_with_find_copies() {
        let mut fixture = Fixture::new();
        let long = "alpha line of the original file\nbeta line of the original file\ngamma line of the original file\ndelta line of the original file\n";
        let first = fixture.commit("first", &[("src.txt", Some(long))]);
        let second = fixture.commit(
            "copy",
            &[
                ("src.txt", Some(&format!("{long}extra\n"))),
                ("dst.txt", Some(long)),
            ],
        );
        let plain = blame_lines(&fixture, "dst.txt", &[("revision", text("HEAD"))]);
        assert_eq!(plain[0].commit, second.to_string());
        let moves = blame_lines(
            &fixture,
            "dst.txt",
            &[
                ("revision", text("HEAD")),
                ("find_moves", Value::Bool(true)),
            ],
        );
        assert_eq!(moves[0].commit, second.to_string());
        let copied = blame_lines(
            &fixture,
            "dst.txt",
            &[
                ("revision", text("HEAD")),
                ("find_copies", Value::Bool(true)),
            ],
        );
        assert_eq!(copied[0].commit, first.to_string());
        assert_eq!(copied[0].original_path.as_deref(), Some("src.txt"));
        assert_eq!(copied[3].original_no, Some(4));
    }

    #[test]
    fn empty_file_has_no_lines() {
        let mut fixture = Fixture::new();
        fixture.commit("empty", &[("e", Some(""))]);
        assert!(blame_lines(&fixture, "e", &[("revision", text("HEAD"))]).is_empty());
    }

    #[test]
    fn maps_authors_through_mailmap() {
        let mut fixture = Fixture::new();
        fixture.commit(
            "first",
            &[
                ("a.txt", Some("a\n")),
                (
                    ".mailmap",
                    Some("Proper <proper@example.com> <author@example.com>\n"),
                ),
            ],
        );
        let lines = blame_lines(&fixture, "a.txt", &[("revision", text("HEAD"))]);
        assert_eq!(lines[0].author, "Proper");
        assert_eq!(lines[0].author_email.as_deref(), Some("proper@example.com"));
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
