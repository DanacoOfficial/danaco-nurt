// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-git::show
// Responsibility:   The `git.show` command.
// Dependencies:     bstr, gix, nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! The `git.show` command.
//!
//! A commit is shown as a `commit` record followed by `hunk` records of its
//! changes against its first parent, with renamed files detected. A merge
//! is shown with the condensed combined difference against all its parents,
//! as `git show` writes it. An annotated tag is shown as a `git_tag` record
//! followed by the object it tags. With `file`, the content of one file at
//! the revision is returned as a `content` record instead.

use std::collections::BTreeMap;

use bstr::{BStr, BString, ByteSlice};
use gix::ObjectId;
use gix::objs::tree::EntryKind;
use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::record::{
    Content, GitFileState, GitTag, GitTagKind, Hunk, HunkStatus, LineEnding,
};
use nurt_contracts::report::{Notable, NotableEntry};
use nurt_contracts::stage::{
    BoundArgs, ExecCtx, InputKinds, ReadStage, RecordStream, StageDescriptor, StageImpl,
    StageInput, StageOutput,
};
use nurt_contracts::value::{Path, Range, Record, Size};

use crate::repo::records::raw_path_bytes;
use crate::revwalk::args;
use crate::revwalk::changes::{self, Detect, FileChange, Side};
use crate::revwalk::combined;
use crate::revwalk::lines;
use crate::revwalk::paths::{PathFilter, entry_at, excluded};
use crate::revwalk::record::{CommitRecords, SignedKind, instant, signature_of};
use crate::revwalk::repo::{OpenRepo, RevisionProblem, quote};

const COMMAND: &str = "git.show";

/// Unchanged lines around each change when `context` is not given.
const DEFAULT_CONTEXT: u32 = 3;

/// Byte order mark of UTF-8.
const UTF8_BOM: &[u8] = b"\xEF\xBB\xBF";

/// Implementation of `git.show`.
///
/// # Examples
///
/// ```
/// use nurt_contracts::stage::StageImpl;
/// use nurt_git::show::GitShow;
///
/// let descriptor = GitShow.descriptor();
/// assert_eq!(descriptor.command, "git.show");
/// assert!(descriptor.source);
/// ```
#[derive(Debug, Clone, Copy, Default)]
pub struct GitShow;

impl StageImpl for GitShow {
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

impl ReadStage for GitShow {
    /// Shows the objects named by `revision` and `revs`, or the content of
    /// `file` at `revision`.
    ///
    /// # Errors
    ///
    /// Fails with `E_GIT_NOT_REPOSITORY` when `repo` is not inside a
    /// repository, `E_GIT_REVISION` for an unknown revision or one that
    /// names a tree or a file, `E_NOT_FOUND` when `file` does not exist at
    /// the revision, `E_INVALID_VALUE` or `E_TYPE` for arguments that cannot
    /// be used together, `E_IO` when the repository cannot be read and
    /// `E_CANCELLED` when the program is stopped.
    fn execute<'a>(
        &'a self,
        args: &'a BoundArgs,
        _input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> nurt_contracts::error::Result<StageOutput<'a>> {
        let query = Query::from_args(args)?;
        let repo = OpenRepo::discover(&query.repo, ctx)?;
        let records = run(&repo, &query, ctx)?;
        Ok(StageOutput::records(RecordStream::from_records(records)))
    }
}

struct Query {
    repo: String,
    revisions: Vec<String>,
    file: Option<String>,
    paths: Vec<String>,
    offset: Option<u64>,
    limit: Option<u64>,
    stat: bool,
    context: u32,
    exclude: Vec<BString>,
}

impl Query {
    fn from_args(bound: &BoundArgs) -> Result<Self, Error> {
        let repo = args::required_path(bound, "repo")?;
        let mut revisions =
            vec![args::text(bound, "revision")?.unwrap_or_else(|| "HEAD".to_owned())];
        let more = args::texts(bound, "revs")?;
        let file = args::path(bound, "file")?;
        let paths = args::paths(bound, "paths")?;
        let offset = args::count(bound, "offset")?;
        let limit = args::count(bound, "limit")?;
        let stat = args::flag(bound, "stat")?;
        let context = match args::count(bound, "context")? {
            None => DEFAULT_CONTEXT,
            Some(lines) => u32::try_from(lines).map_err(|_| {
                args::invalid(
                    bound,
                    "context",
                    format!("`context` must be at most {}, not {lines}", u32::MAX),
                )
            })?,
        };
        if offset == Some(0) {
            return Err(args::invalid(
                bound,
                "offset",
                "`offset` counts lines from 1, give 1 or more",
            ));
        }
        if file.is_none() && (offset.is_some() || limit.is_some()) {
            let name = if offset.is_some() { "offset" } else { "limit" };
            return Err(args::invalid(
                bound,
                name,
                format!("`{name}` applies to the content of `file`, give `file` as well"),
            ));
        }
        if file.is_some() {
            let conflict = [
                ("revs", !more.is_empty()),
                ("paths", !paths.is_empty()),
                ("stat", stat),
            ];
            if let Some((name, _)) = conflict.iter().find(|(_, given)| *given) {
                return Err(args::invalid(
                    bound,
                    "file",
                    format!("`file` cannot be combined with `{name}`"),
                ));
            }
        }
        revisions.extend(more);
        Ok(Self {
            exclude: args::globs(bound, "exclude")?,
            repo,
            revisions,
            file,
            paths,
            offset,
            limit,
            stat,
            context,
        })
    }
}

fn run(repo: &OpenRepo, query: &Query, ctx: &ExecCtx) -> Result<Vec<Record>, Error> {
    if let Some(file) = &query.file {
        return file_content(repo, query, file).map(|content| vec![Record::from(content)]);
    }
    let relative: Vec<BString> = query
        .paths
        .iter()
        .map(|path| repo.relative(path))
        .collect::<Result<_, _>>()?;
    let mut show = Show {
        repo,
        records: CommitRecords::new(repo, &ctx.demand().fields),
        filter: (!relative.is_empty()).then(|| PathFilter::new(relative)),
        exclude: query.exclude.clone(),
        context: query.context,
        stat: query.stat,
        out: Vec::new(),
        excluded: 0,
    };
    for revision in &query.revisions {
        ctx.checkpoint()?;
        let id = repo.resolve_object(revision)?;
        show.object(revision, id)?;
    }
    if show.excluded > 0 {
        let mut note = NotableEntry::new(Notable::Excluded);
        note.count = Some(show.excluded);
        note.detail = Some(format!(
            "{} changed files left out by `exclude`",
            show.excluded
        ));
        ctx.note(note);
    }
    Ok(show.out)
}

/// Records of the objects shown so far.
struct Show<'r> {
    repo: &'r OpenRepo,
    records: CommitRecords<'r>,
    filter: Option<PathFilter>,
    exclude: Vec<BString>,
    context: u32,
    stat: bool,
    out: Vec<Record>,
    excluded: u64,
}

impl Show<'_> {
    /// Shows an object; tags are shown together with the objects they tag.
    fn object(&mut self, revision: &str, id: ObjectId) -> Result<(), Error> {
        let mut current = id;
        loop {
            let object = self
                .repo
                .repo
                .find_object(current)
                .map_err(|err| self.repo.read_error(format_args!("object {current}"), err))?;
            match object.kind {
                gix::object::Kind::Commit => return self.commit(current),
                gix::object::Kind::Tag => {
                    let (record, target) = self.tag(current, &object.data)?;
                    self.out.push(record.into());
                    current = target;
                }
                gix::object::Kind::Tree | gix::object::Kind::Blob if current == id => {
                    return Err(self
                        .repo
                        .revision_error(revision, RevisionProblem::NotACommit)
                        .with_fix(format!(
                            "git.show {} revision: \"HEAD\" file: \"<file>\"",
                            quote(self.repo.label())
                        )));
                }
                gix::object::Kind::Tree | gix::object::Kind::Blob => return Ok(()),
            }
        }
    }

    fn tag(&self, id: ObjectId, data: &[u8]) -> Result<(GitTag, ObjectId), Error> {
        let tag = gix::objs::TagRef::from_bytes(data, self.repo.repo.object_hash())
            .map_err(|err| self.repo.read_error(format_args!("tag {id}"), err))?;
        let tagger = tag
            .tagger()
            .map_err(|err| self.repo.read_error(format_args!("tag {id}"), err))?
            .map(|signature| self.records.person(signature));
        let message = tag.message.to_str_lossy().trim_end_matches('\n').to_owned();
        let record = GitTag {
            name: tag.name.to_str_lossy().into_owned(),
            target: tag.target().to_string(),
            kind: GitTagKind::Annotated,
            tagger: tagger
                .as_ref()
                .map(|person| format!("{} <{}>", person.name, person.email)),
            time: tagger.as_ref().map(|person| instant(person.seconds)),
            message: Some(message),
            signature: self
                .records
                .signatures()
                .then(|| signature_of(self.repo, SignedKind::Tag, data, tag.signature)),
        };
        Ok((record, tag.target()))
    }

    fn commit(&mut self, id: ObjectId) -> Result<(), Error> {
        let repo = self.repo;
        let mut commit = self.records.commit(id)?;
        let info = repo
            .commits
            .info(&repo.repo, id)
            .map_err(|err| repo.read_error(format_args!("commit {id}"), err))?;
        let parent_trees: Vec<ObjectId> = info
            .parents
            .iter()
            .map(|parent| repo.tree_of(*parent))
            .collect::<Result<_, _>>()?;
        if self.stat {
            let changes = self.changes(parent_trees.first().copied(), info.tree)?;
            let (files, stats) = self.records.file_stats(&changes)?;
            commit.files = Some(files);
            commit.stats = Some(stats);
            self.out.push(commit.into());
            return Ok(());
        }
        self.out.push(commit.into());
        if parent_trees.len() > 1 {
            return self.combined(&parent_trees, info.tree);
        }
        for change in self.changes(parent_trees.first().copied(), info.tree)? {
            self.file_hunks(&change)?;
        }
        Ok(())
    }

    /// Changed files against one parent, within `paths` and outside
    /// `exclude`.
    fn changes(&mut self, old: Option<ObjectId>, new: ObjectId) -> Result<Vec<FileChange>, Error> {
        let found =
            changes::tree_changes(self.repo, old, new, Detect::Renames, self.filter.as_ref())?;
        Ok(found
            .into_iter()
            .filter(|change| self.keep(change.path.as_ref()))
            .collect())
    }

    /// Whether a changed file is shown; files left out by `exclude` are
    /// counted.
    fn keep(&mut self, path: &BStr) -> bool {
        if excluded(&self.exclude, path) {
            self.excluded += 1;
            false
        } else {
            true
        }
    }

    fn file_hunks(&mut self, change: &FileChange) -> Result<(), Error> {
        let old = self.records.content(change.old)?;
        let new = self.records.content(change.new)?;
        let base = Hunk {
            path: Path::from(change.path.to_str_lossy().into_owned()),
            raw_path: raw_path_bytes(change.path.as_bstr()),
            old_path: change
                .old_path
                .as_ref()
                .map(|path| Path::from(path.to_str_lossy().into_owned())),
            raw_old_path: change
                .old_path
                .as_ref()
                .and_then(|path| raw_path_bytes(path.as_bstr())),
            status: Some(hunk_status(change.status)),
            old_mode: None,
            new_mode: None,
            ..Hunk::default()
        };
        let base = with_modes(base, change.old, change.new);
        if lines::is_binary(&old) || lines::is_binary(&new) {
            let status = if change.status == GitFileState::Modified {
                HunkStatus::Binary
            } else {
                hunk_status(change.status)
            };
            self.out.push(
                Hunk {
                    status: Some(status),
                    binary: Some(true),
                    ..base
                }
                .into(),
            );
            return Ok(());
        }
        let hunks = lines::unified(&old, &new, self.context);
        if hunks.is_empty() {
            self.out.push(base.into());
            return Ok(());
        }
        for hunk in hunks {
            self.out.push(
                Hunk {
                    old_start: hunk.old_start,
                    old_len: hunk.old_len,
                    new_start: hunk.new_start,
                    new_len: hunk.new_len,
                    lines: hunk.lines,
                    header: hunk.header,
                    ..base.clone()
                }
                .into(),
            );
        }
        Ok(())
    }

    /// Files of a merge that differ from every parent, with the condensed
    /// combined difference; the ranges of the first parent are given as the
    /// old side.
    fn combined(&mut self, parent_trees: &[ObjectId], tree: ObjectId) -> Result<(), Error> {
        let mut per_parent: Vec<BTreeMap<BString, FileChange>> =
            Vec::with_capacity(parent_trees.len());
        for parent in parent_trees {
            let found = changes::tree_changes(
                self.repo,
                Some(*parent),
                tree,
                Detect::Nothing,
                self.filter.as_ref(),
            )?;
            per_parent.push(
                found
                    .into_iter()
                    .map(|change| (change.path.clone(), change))
                    .collect(),
            );
        }
        let Some((first, rest)) = per_parent.split_first() else {
            return Ok(());
        };
        let mut files = Vec::new();
        for (path, change) in first {
            if !rest.iter().all(|changes| changes.contains_key(path)) {
                continue;
            }
            let sides: Vec<Option<Side>> = per_parent
                .iter()
                .map(|changes| changes.get(path).and_then(|change| change.old))
                .collect();
            files.push((path.clone(), sides, change.new));
        }
        for (path, sides, result) in files {
            if self.keep(path.as_ref()) {
                self.combined_file(&path, &sides, result)?;
            }
        }
        Ok(())
    }

    fn combined_file(
        &mut self,
        path: &BString,
        sides: &[Option<Side>],
        result: Option<Side>,
    ) -> Result<(), Error> {
        let new = self.records.content(result)?;
        let parents: Vec<Vec<u8>> = sides
            .iter()
            .map(|side| self.records.content(*side))
            .collect::<Result<_, _>>()?;
        let status = if result.is_none() {
            HunkStatus::Deleted
        } else if sides.iter().all(Option::is_none) {
            HunkStatus::Added
        } else {
            HunkStatus::Modified
        };
        let base = Hunk {
            path: Path::from(path.to_str_lossy().into_owned()),
            raw_path: raw_path_bytes(path.as_bstr()),
            status: Some(status),
            ..Hunk::default()
        };
        if lines::is_binary(&new) || parents.iter().any(|parent| lines::is_binary(parent)) {
            self.out.push(
                Hunk {
                    binary: Some(true),
                    ..base
                }
                .into(),
            );
            return Ok(());
        }
        let slices: Vec<&[u8]> = parents.iter().map(Vec::as_slice).collect();
        for hunk in combined::combine(&slices, &new, self.context) {
            let (old_start, old_len) = hunk.parents.first().copied().unwrap_or((0, 0));
            self.out.push(
                Hunk {
                    old_start,
                    old_len,
                    new_start: hunk.new_start,
                    new_len: hunk.new_len,
                    lines: hunk.lines,
                    ..base.clone()
                }
                .into(),
            );
        }
        Ok(())
    }
}

fn hunk_status(status: GitFileState) -> HunkStatus {
    match status {
        GitFileState::Added => HunkStatus::Added,
        GitFileState::Deleted => HunkStatus::Deleted,
        GitFileState::Renamed | GitFileState::Copied => HunkStatus::Renamed,
        _ => HunkStatus::Modified,
    }
}

/// Modes as `git` reports them: the mode of an added or deleted file, and
/// both modes when they differ.
fn with_modes(mut hunk: Hunk, old: Option<Side>, new: Option<Side>) -> Hunk {
    let text = |side: Side| format!("{:06o}", side.mode.value());
    match (old, new) {
        (None, Some(new)) => hunk.new_mode = Some(text(new)),
        (Some(old), None) => hunk.old_mode = Some(text(old)),
        (Some(old), Some(new)) if old.mode != new.mode => {
            hunk.old_mode = Some(text(old));
            hunk.new_mode = Some(text(new));
        }
        _ => {}
    }
    hunk
}

fn file_content(repo: &OpenRepo, query: &Query, file: &str) -> Result<Content, Error> {
    let revision = query.revisions.first().map_or("HEAD", String::as_str);
    let id = repo.resolve_object(revision)?;
    let commit = repo
        .peel_to_commit(id)
        .ok_or_else(|| repo.revision_error(revision, RevisionProblem::NotACommit))?;
    let tree = repo.tree_of(commit)?;
    let relative = repo.relative(file)?;
    let entry = entry_at(repo, tree, relative.as_ref())
        .map_err(|err| repo.read_error(format_args!("`{file}`"), err))?;
    let Some((mode, blob)) = entry else {
        return Err(Error::new(
            ErrorCode::NotFound,
            format!("`{file}` does not exist at `{revision}`"),
        )
        .with_found(file)
        .with_next_step(format!(
            "git.log {} path: {} count: 5",
            quote(repo.label()),
            quote(file)
        )));
    };
    match mode.kind() {
        EntryKind::Tree => {
            return Err(Error::new(
                ErrorCode::InvalidValue,
                format!("`{file}` is a directory at `{revision}`, give a file"),
            )
            .with_found(format!("{COMMAND} file:"))
            .with_next_step(format!(
                "git.show {} revision: {} paths: [{}]",
                quote(repo.label()),
                quote(revision),
                quote(file)
            )));
        }
        EntryKind::Commit => {
            return Err(Error::new(
                ErrorCode::InvalidValue,
                format!("`{file}` is a submodule at `{revision}`, give a file"),
            )
            .with_found(format!("{COMMAND} file:")));
        }
        EntryKind::Blob | EntryKind::BlobExecutable | EntryKind::Link => {}
    }
    let bytes = repo.blob(blob)?;
    Ok(content_record(
        repo.absolute(relative.as_ref()),
        &bytes,
        blob,
        commit,
        query.offset,
        query.limit,
    ))
}

/// The `content` record of a file stored in the repository.
fn content_record(
    path: String,
    bytes: &[u8],
    blob: ObjectId,
    commit: ObjectId,
    offset: Option<u64>,
    limit: Option<u64>,
) -> Content {
    let bom = bytes.starts_with(UTF8_BOM);
    let body = if bom { &bytes[UTF8_BOM.len()..] } else { bytes };
    let mut record = Content {
        path: Path::from(path),
        size: Size::from_bytes(bytes.len() as u64),
        etag: blob.to_string(),
        bom,
        line_ending: line_ending(body),
        binary: Some(lines::is_binary(bytes)),
        revision: Some(commit.to_string()),
        ..Content::default()
    };
    if lines::is_binary(bytes) {
        record.encoding = "binary".to_owned();
        return record;
    }
    let all = lines::split_lines(body);
    let first = offset.unwrap_or(1);
    let start = usize::try_from(first.saturating_sub(1))
        .unwrap_or(usize::MAX)
        .min(all.len());
    let end = match limit {
        Some(count) => start
            .saturating_add(usize::try_from(count).unwrap_or(usize::MAX))
            .min(all.len()),
        None => all.len(),
    };
    let selected = all[start..end].concat();
    let (text, encoding) = decode(body.to_str().is_ok(), &selected);
    record.text = text;
    record.encoding = encoding.to_owned();
    record.lines = (end - start) as u64;
    record.total_lines = Some(all.len() as u64);
    if offset.is_some() || limit.is_some() {
        let shown = (end - start) as u64;
        record.range = (shown > 0)
            .then(|| {
                Range::new(
                    i64::try_from(first).ok(),
                    i64::try_from(first + shown - 1).ok(),
                )
                .ok()
            })
            .flatten();
        record.truncated = Some(end < all.len());
    }
    record
}

/// Text of the selected lines: UTF-8 when the whole file is valid UTF-8,
/// ISO 8859-1 otherwise, so that every byte maps to one character.
fn decode(utf8: bool, bytes: &[u8]) -> (String, &'static str) {
    if utf8 {
        (bytes.to_str_lossy().into_owned(), "utf-8")
    } else {
        (
            bytes.iter().map(|byte| char::from(*byte)).collect(),
            "iso-8859-1",
        )
    }
}

fn line_ending(bytes: &[u8]) -> LineEnding {
    let (mut crlf, mut lf, mut cr) = (false, false, false);
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'\r' if bytes.get(index + 1) == Some(&b'\n') => {
                crlf = true;
                index += 1;
            }
            b'\r' => cr = true,
            b'\n' => lf = true,
            _ => {}
        }
        index += 1;
    }
    match (crlf, lf, cr) {
        (false, false, false) => LineEnding::None,
        (true, false, false) => LineEnding::Crlf,
        (false, true, false) => LineEnding::Lf,
        (false, false, true) => LineEnding::Cr,
        _ => LineEnding::Mixed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::revwalk::fixture::{Fixture, args as bound, exec_ctx, run as execute};
    use nurt_contracts::record::Commit;
    use nurt_contracts::value::Value;

    fn show_with(fixture: &Fixture, values: &[(&str, Value)], ctx: &ExecCtx) -> Vec<Record> {
        let mut all = vec![("repo", Value::Path(fixture.root()))];
        all.extend(values.iter().cloned());
        execute(&GitShow, &bound(COMMAND, &all), ctx).unwrap()
    }

    fn show(fixture: &Fixture, values: &[(&str, Value)]) -> Vec<Record> {
        show_with(fixture, values, &exec_ctx(None))
    }

    fn show_error(fixture: &Fixture, values: &[(&str, Value)]) -> ErrorCode {
        let mut all = vec![("repo", Value::Path(fixture.root()))];
        all.extend(values.iter().cloned());
        execute(&GitShow, &bound(COMMAND, &all), &exec_ctx(None))
            .unwrap_err()
            .code()
    }

    fn text(value: &str) -> Value {
        Value::Text(value.to_owned())
    }

    fn hunks(records: &[Record]) -> Vec<Hunk> {
        records
            .iter()
            .filter_map(|record| Hunk::try_from(record.clone()).ok())
            .collect()
    }

    fn content(records: &[Record]) -> Content {
        Content::try_from(records[0].clone()).unwrap()
    }

    /// History of three commits: `a.txt` added, a line changed, then the
    /// file renamed with one more line.
    fn linear() -> Fixture {
        let mut fixture = Fixture::new();
        fixture.commit(
            "add a",
            &[("a.txt", Some("one\ntwo\nthree\nfour\nfive\nsix\n"))],
        );
        fixture.commit(
            "change a",
            &[("a.txt", Some("one\nTWO\nthree\nfour\nfive\nsix\n"))],
        );
        fixture.commit(
            "rename a",
            &[
                ("a.txt", None),
                ("b.txt", Some("one\nTWO\nthree\nfour\nfive\nsix\nseven\n")),
            ],
        );
        fixture
    }

    /// A merge of two branches that both changed the second line of `f`.
    fn conflicted(resolution: &str) -> Fixture {
        let mut fixture = Fixture::new();
        let base = fixture.commit("base", &[("f", Some("a\nb\nc\n")), ("g", Some("g\n"))]);
        let left = fixture.commit_on(
            &[base],
            "left",
            &[("f", Some("a\nL\nc\n")), ("g", Some("g2\n"))],
        );
        let right = fixture.commit_on(&[base], "right", &[("f", Some("a\nR\nc\n"))]);
        let merge = fixture.commit_on(&[left, right], "merge", &[("f", Some(resolution))]);
        fixture.branch("main", merge);
        fixture
    }

    #[test]
    fn shows_a_commit_with_hunks_against_its_parent() {
        let fixture = linear();
        let records = show(&fixture, &[("revision", text("HEAD~1"))]);
        let commit = Commit::try_from(records[0].clone()).unwrap();
        assert_eq!(commit.subject, "change a");
        let found = hunks(&records);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].path, Path::from("a.txt"));
        assert_eq!(
            found[0].lines,
            [" one", "-two", "+TWO", " three", " four", " five"]
        );
        assert_eq!(
            (
                found[0].old_start,
                found[0].old_len,
                found[0].new_start,
                found[0].new_len
            ),
            (1, 5, 1, 5)
        );
        assert_eq!(found[0].status, Some(HunkStatus::Modified));
        let narrow = hunks(&show(
            &fixture,
            &[("revision", text("HEAD~1")), ("context", Value::Int(0))],
        ));
        assert_eq!(narrow[0].lines, ["-two", "+TWO"]);
    }

    #[test]
    fn root_commit_adds_every_line() {
        let fixture = linear();
        let found = hunks(&show(&fixture, &[("revision", text("HEAD~2"))]));
        assert_eq!(found[0].status, Some(HunkStatus::Added));
        assert_eq!(found[0].new_mode.as_deref(), Some("100644"));
        assert_eq!(
            (found[0].old_start, found[0].old_len, found[0].new_len),
            (0, 0, 6)
        );
    }

    #[test]
    fn renamed_file_keeps_its_old_path() {
        let fixture = linear();
        let found = hunks(&show(&fixture, &[("revision", text("HEAD"))]));
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].status, Some(HunkStatus::Renamed));
        assert_eq!(found[0].old_path, Some(Path::from("a.txt")));
        assert_eq!(found[0].lines.last().map(String::as_str), Some("+seven"));
    }

    #[test]
    fn stat_lists_files_without_hunks() {
        let fixture = linear();
        let records = show(
            &fixture,
            &[("revision", text("HEAD")), ("stat", Value::Bool(true))],
        );
        assert_eq!(records.len(), 1);
        let commit = Commit::try_from(records[0].clone()).unwrap();
        assert_eq!(commit.stats.unwrap().added, 1);
        assert_eq!(commit.files.unwrap()[0].path, "b.txt");
    }

    #[test]
    fn paths_and_exclude_limit_the_files() {
        let mut fixture = Fixture::new();
        fixture.commit(
            "two files",
            &[
                ("src/a.rs", Some("a\n")),
                ("Cargo.lock", Some("lock\n")),
                ("docs/b.md", Some("b\n")),
            ],
        );
        let src = Value::List(vec![Value::Path(fixture.path("src"))]);
        let found = hunks(&show(
            &fixture,
            &[("revision", text("HEAD")), ("paths", src)],
        ));
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].path, Path::from("src/a.rs"));
        let ctx = exec_ctx(None);
        let exclude = Value::List(vec![
            Value::Glob("*.lock".into()),
            Value::Glob("docs/*".into()),
        ]);
        let found = hunks(&show_with(
            &fixture,
            &[("revision", text("HEAD")), ("exclude", exclude)],
            &ctx,
        ));
        assert_eq!(found.len(), 1);
        let notes = ctx.take_notes();
        assert_eq!(notes[0].code, Notable::Excluded);
        assert_eq!(notes[0].count, Some(2));
    }

    #[test]
    fn merge_shows_the_combined_difference() {
        let fixture = conflicted("a\nM\nc\n");
        let records = show(&fixture, &[("revision", text("HEAD"))]);
        let found = hunks(&records);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].path, Path::from("f"));
        assert_eq!(found[0].lines, ["  a", "- L", " -R", "++M", "  c"]);
        assert_eq!(
            (
                found[0].old_start,
                found[0].old_len,
                found[0].new_start,
                found[0].new_len
            ),
            (1, 3, 1, 3)
        );
        let taken = conflicted("a\nL\nc\n");
        assert!(hunks(&show(&taken, &[("revision", text("HEAD"))])).is_empty());
        let stat = show(
            &fixture,
            &[("revision", text("HEAD")), ("stat", Value::Bool(true))],
        );
        let commit = Commit::try_from(stat[0].clone()).unwrap();
        assert_eq!(commit.files.unwrap()[0].path, "f");
    }

    #[test]
    fn annotated_tag_is_shown_before_its_commit() {
        let mut fixture = linear();
        let head = fixture.head();
        fixture.annotated_tag("v1", head, "release one");
        let records = show(&fixture, &[("revision", text("v1"))]);
        let tag = GitTag::try_from(records[0].clone()).unwrap();
        assert_eq!(tag.name, "v1");
        assert_eq!(tag.target, head.to_string());
        assert_eq!(tag.kind, GitTagKind::Annotated);
        assert_eq!(tag.tagger.as_deref(), Some("Tagger <tagger@example.com>"));
        assert_eq!(tag.message.as_deref(), Some("release one"));
        assert!(tag.signature.is_none());
        assert_eq!(
            Commit::try_from(records[1].clone()).unwrap().id,
            head.to_string()
        );
        assert_eq!(hunks(&records).len(), 1);
    }

    #[test]
    fn shows_several_revisions_in_order() {
        let fixture = linear();
        let records = show(
            &fixture,
            &[
                ("revision", text("HEAD")),
                ("revs", Value::List(vec![text("HEAD~2")])),
                ("stat", Value::Bool(true)),
            ],
        );
        let subjects: Vec<String> = records
            .iter()
            .map(|record| Commit::try_from(record.clone()).unwrap().subject)
            .collect();
        assert_eq!(subjects, ["rename a", "add a"]);
    }

    #[test]
    fn binary_and_mode_changes_have_no_lines() {
        let mut fixture = Fixture::new();
        fixture.commit("first", &[("bin", Some("x\0y")), ("t", Some("target\n"))]);
        fixture.commit("binary", &[("bin", Some("x\0z"))]);
        let found = hunks(&show(&fixture, &[("revision", text("HEAD"))]));
        assert_eq!(found[0].binary, Some(true));
        assert_eq!(found[0].status, Some(HunkStatus::Binary));
        assert!(found[0].lines.is_empty());
        fixture.commit_link("link", "t", "elsewhere");
        let link = hunks(&show(&fixture, &[("revision", text("HEAD"))]));
        assert_eq!(link[0].old_mode.as_deref(), Some("100644"));
        assert_eq!(link[0].new_mode.as_deref(), Some("120000"));
        assert!(link[0].lines.contains(&"+elsewhere".to_owned()));
    }

    #[test]
    fn rejects_trees_and_unknown_revisions() {
        let fixture = linear();
        assert_eq!(
            show_error(&fixture, &[("revision", text("HEAD^{tree}"))]),
            ErrorCode::GitRevision
        );
        assert_eq!(
            show_error(&fixture, &[("revision", text("missing"))]),
            ErrorCode::GitRevision
        );
        assert_eq!(
            show_error(
                &fixture,
                &[("revision", text("HEAD")), ("exclude", Value::Int(1))]
            ),
            ErrorCode::Type
        );
    }

    #[test]
    fn returns_file_content_with_offset_and_limit() {
        let fixture = linear();
        let file = Value::Path(fixture.path("a.txt"));
        let whole = content(&show(
            &fixture,
            &[("revision", text("HEAD~1")), ("file", file.clone())],
        ));
        assert_eq!(whole.text, "one\nTWO\nthree\nfour\nfive\nsix\n");
        assert_eq!((whole.lines, whole.total_lines), (6, Some(6)));
        assert_eq!(whole.line_ending, LineEnding::Lf);
        assert_eq!(whole.encoding, "utf-8");
        assert_eq!(whole.revision, Some(fixture.resolve("HEAD~1").to_string()));
        assert!(whole.range.is_none());
        let part = content(&show(
            &fixture,
            &[
                ("revision", text("HEAD~1")),
                ("file", file.clone()),
                ("offset", Value::Int(2)),
                ("limit", Value::Int(2)),
            ],
        ));
        assert_eq!(part.text, "TWO\nthree\n");
        assert_eq!(part.range, Range::new(Some(2), Some(3)).ok());
        assert_eq!(part.truncated, Some(true));
        let tail = content(&show(
            &fixture,
            &[
                ("revision", text("HEAD~2")),
                ("file", file.clone()),
                ("offset", Value::Int(10)),
            ],
        ));
        assert_eq!(tail.text, "");
        assert_eq!(tail.truncated, Some(false));
        assert_eq!(
            show_error(&fixture, &[("revision", text("HEAD")), ("file", file)]),
            ErrorCode::NotFound
        );
    }

    #[test]
    fn file_content_of_binary_files_and_directories() {
        let mut fixture = Fixture::new();
        fixture.commit(
            "files",
            &[("bin", Some("x\0y")), ("dir/a", Some("a\r\nb\r\n"))],
        );
        let binary = content(&show(
            &fixture,
            &[
                ("revision", text("HEAD")),
                ("file", Value::Path(fixture.path("bin"))),
            ],
        ));
        assert_eq!(binary.binary, Some(true));
        assert_eq!(binary.encoding, "binary");
        assert_eq!(binary.text, "");
        let crlf = content(&show(
            &fixture,
            &[
                ("revision", text("HEAD")),
                ("file", Value::Path(fixture.path("dir/a"))),
            ],
        ));
        assert_eq!(crlf.line_ending, LineEnding::Crlf);
        assert_eq!(
            show_error(
                &fixture,
                &[
                    ("revision", text("HEAD")),
                    ("file", Value::Path(fixture.path("dir")))
                ]
            ),
            ErrorCode::InvalidValue
        );
    }

    #[test]
    fn rejects_arguments_that_cannot_be_combined() {
        let fixture = linear();
        let file = Value::Path(fixture.path("b.txt"));
        assert_eq!(
            show_error(
                &fixture,
                &[("revision", text("HEAD")), ("offset", Value::Int(1))]
            ),
            ErrorCode::InvalidValue
        );
        assert_eq!(
            show_error(
                &fixture,
                &[
                    ("revision", text("HEAD")),
                    ("file", file.clone()),
                    ("offset", Value::Int(0))
                ]
            ),
            ErrorCode::InvalidValue
        );
        assert_eq!(
            show_error(
                &fixture,
                &[
                    ("revision", text("HEAD")),
                    ("file", file.clone()),
                    ("stat", Value::Bool(true))
                ]
            ),
            ErrorCode::InvalidValue
        );
        assert_eq!(
            show_error(
                &fixture,
                &[
                    ("revision", text("HEAD")),
                    ("file", file),
                    ("revs", text("HEAD~1"))
                ]
            ),
            ErrorCode::InvalidValue
        );
    }

    #[test]
    fn decodes_text_and_line_endings() {
        assert_eq!(line_ending(b"a\r\nb\n"), LineEnding::Mixed);
        assert_eq!(line_ending(b"a\rb\r"), LineEnding::Cr);
        assert_eq!(line_ending(b"no break"), LineEnding::None);
        assert_eq!(
            decode(false, b"caf\xe9"),
            ("caf\u{e9}".to_owned(), "iso-8859-1")
        );
        let bom = content_record(
            "/r/f".to_owned(),
            b"\xEF\xBB\xBFtext\n",
            ObjectId::null(gix::hash::Kind::Sha1),
            ObjectId::null(gix::hash::Kind::Sha1),
            None,
            None,
        );
        assert!(bom.bom);
        assert_eq!(bom.text, "text\n");
        assert_eq!(bom.size, Size::from_bytes(8));
    }

    #[test]
    fn exclusion_patterns_match_names_and_paths() {
        let patterns = vec![BString::from("*.lock"), BString::from("vendor/**")];
        assert!(excluded(&patterns, "deep/dir/Cargo.lock".into()));
        assert!(excluded(&patterns, "vendor/a/b.rs".into()));
        assert!(!excluded(&patterns, "src/vendor/a.rs".into()));
        assert!(!excluded(&patterns, "lockfile".into()));
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
