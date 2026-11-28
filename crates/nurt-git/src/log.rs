// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-git::log
// Responsibility:   The `git.log` command.
// Dependencies:     bstr, gix, nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! The `git.log` command.
//!
//! Commits are listed in the order of `git log`: newest first by committer
//! time, without topological sorting. With `path` the history is simplified
//! as `git log -- <path>` simplifies it. Filters that need the changes of a
//! commit (`follow`, `pickaxe`, `diff_regex`) consider commits with one
//! parent or none; merges have no changes of their own and never match them.
//! The files of those changes are paired into renames unless `find_renames`
//! is `false`, and into copies of files changed by the same commit with
//! `find_copies`.

use std::cmp::{Ordering, Reverse};
use std::collections::{BinaryHeap, HashMap, HashSet};
use std::ops::Range;
use std::path::Path;

use bstr::{BString, ByteSlice};
use gix::ObjectId;
use nurt_contracts::error::{Candidate, Error, ErrorCode};
use nurt_contracts::record::Commit;
use nurt_contracts::services::CodeService;
use nurt_contracts::stage::{
    BoundArgs, ExecCtx, InputKinds, ReadStage, RecordStream, StageDescriptor, StageImpl,
    StageInput, StageOutput,
};
use nurt_contracts::value::{Record, RecordKind, Value};

use crate::revwalk::args;
use crate::revwalk::changes::{self, Detect, FileChange};
use crate::revwalk::lines;
use crate::revwalk::paths::{PathFilter, entry_at, excluded};
use crate::revwalk::record::{CommitRecords, Person};
use crate::revwalk::repo::{OpenRepo, quote};
use crate::revwalk::spec::{self, Tips};
use crate::revwalk::walk::{Walk, WalkOptions};

const COMMAND: &str = "git.log";

/// Number of commits listed when `count` is not given.
const DEFAULT_COUNT: u64 = 20;

const SECONDS_PER_DAY: i64 = 86_400;

/// Implementation of `git.log`.
///
/// # Examples
///
/// ```
/// use nurt_contracts::stage::StageImpl;
/// use nurt_git::log::GitLog;
///
/// let descriptor = GitLog.descriptor();
/// assert_eq!(descriptor.command, "git.log");
/// assert!(descriptor.source);
/// ```
#[derive(Debug, Clone, Copy, Default)]
pub struct GitLog;

impl StageImpl for GitLog {
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

impl ReadStage for GitLog {
    /// Lists the commits selected by the arguments as `commit` records, or
    /// as `map` records `{key, count}` with `group_by`.
    ///
    /// # Errors
    ///
    /// `E_GIT_NOT_REPOSITORY` when `repo` is not inside a repository,
    /// `E_GIT_REVISION` for an unknown revision, `E_REGEX` for an invalid
    /// pattern, `E_INVALID_VALUE` or `E_MISSING_PARAM` for arguments that
    /// cannot be combined, `E_SYMBOL_NOT_FOUND`, `E_SYMBOL_AMBIGUOUS` and
    /// `E_UNSUPPORTED_LANGUAGE` for `symbol`, `E_IO` when the repository
    /// cannot be read and `E_CANCELLED` when the program is stopped.
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

/// Content filter on the changes of a commit.
enum Pickaxe {
    /// The number of occurrences of a text differs between the sides.
    Text(Vec<u8>),
    /// The number of matches of a pattern differs between the sides.
    Pattern(regex::bytes::Regex),
    /// An added or removed line matches a pattern.
    Lines(regex::bytes::Regex),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GroupBy {
    Type,
    Author,
    Day,
}

struct Query {
    repo: String,
    revision: String,
    paths: Vec<String>,
    count: u64,
    skip: u64,
    author: Option<regex::Regex>,
    message: Option<regex::Regex>,
    since: Option<i64>,
    until: Option<i64>,
    pickaxe: Option<Pickaxe>,
    follow: bool,
    symbol: Option<String>,
    stat: bool,
    group_by: Option<GroupBy>,
    exclude: Vec<BString>,
    /// Detection of moved files in the changes of a commit.
    detect: Detect,
}

impl Query {
    fn from_args(bound: &BoundArgs) -> Result<Self, Error> {
        let repo = args::required_path(bound, "repo")?;
        let paths = args::paths(bound, "path")?;
        let follow = args::flag(bound, "follow")?;
        let symbol = args::text(bound, "symbol")?;
        let stat = args::flag(bound, "stat")?;
        let group_by = match args::text(bound, "group_by")?.as_deref() {
            None => None,
            Some("type") => Some(GroupBy::Type),
            Some("author") => Some(GroupBy::Author),
            Some("day") => Some(GroupBy::Day),
            Some(other) => {
                return Err(args::invalid(
                    bound,
                    "group_by",
                    format!("`group_by` is `type`, `author` or `day`, not `{other}`"),
                ));
            }
        };
        if follow || symbol.is_some() {
            let name = if follow { "follow" } else { "symbol" };
            if paths.is_empty() {
                return Err(Error::new(
                    ErrorCode::MissingParam,
                    format!("`{name}` needs one file in `path`"),
                )
                .with_fix(format!(
                    "git.log {} path: \"<file>\" {name}: …",
                    quote(&repo)
                )));
            }
            if paths.len() > 1 {
                return Err(args::invalid(
                    bound,
                    "path",
                    format!(
                        "`{name}` needs exactly one file in `path`, not {}",
                        paths.len()
                    ),
                ));
            }
        }
        if symbol.is_some() && (follow || stat) {
            let other = if follow { "follow" } else { "stat" };
            return Err(args::invalid(
                bound,
                "symbol",
                format!("`symbol` cannot be combined with `{other}`"),
            ));
        }
        let detect = if args::flag(bound, "find_copies")? {
            Detect::Copies
        } else if args::flag_or(bound, "find_renames", true)? {
            Detect::Renames
        } else {
            Detect::Nothing
        };
        Ok(Self {
            revision: args::text(bound, "revision")?.unwrap_or_else(|| "HEAD".to_owned()),
            count: args::count(bound, "count")?.unwrap_or(DEFAULT_COUNT),
            skip: args::count(bound, "skip")?.unwrap_or(0),
            author: args::regex(bound, "author")?,
            message: args::regex(bound, "message")?,
            since: args::seconds(bound, "since")?,
            until: args::seconds(bound, "until")?,
            pickaxe: pickaxe(bound)?,
            repo,
            paths,
            follow,
            symbol,
            stat,
            group_by,
            exclude: args::globs(bound, "exclude")?,
            detect,
        })
    }
}

fn pickaxe(bound: &BoundArgs) -> Result<Option<Pickaxe>, Error> {
    let text = match bound.value("pickaxe") {
        Some(Value::Regex(regex)) => Some(Pickaxe::Pattern(args::bytes_regex(
            bound, "pickaxe", regex,
        )?)),
        Some(Value::Text(text)) if text.is_empty() => {
            return Err(args::invalid(
                bound,
                "pickaxe",
                "`pickaxe` needs a non-empty text",
            ));
        }
        Some(Value::Text(text)) => Some(Pickaxe::Text(text.clone().into_bytes())),
        None | Some(Value::Null) => None,
        Some(_) => {
            return Err(Error::new(
                ErrorCode::Type,
                "`git.log` expects a text or a regular expression for `pickaxe`",
            ));
        }
    };
    let lines = match args::pattern(bound, "diff_regex")? {
        Some(source) => Some(Pickaxe::Lines(args::bytes_regex(
            bound,
            "diff_regex",
            &source,
        )?)),
        None => None,
    };
    if text.is_some() && lines.is_some() {
        return Err(args::invalid(
            bound,
            "diff_regex",
            "`diff_regex` cannot be combined with `pickaxe`",
        ));
    }
    Ok(text.or(lines))
}

fn run(repo: &OpenRepo, query: &Query, ctx: &ExecCtx) -> Result<Vec<Record>, Error> {
    let tips = spec::resolve(repo, &query.revision)?;
    if tips.is_empty() {
        return Ok(Vec::new());
    }
    let relative: Vec<BString> = query
        .paths
        .iter()
        .map(|path| repo.relative(path))
        .collect::<Result<_, _>>()?;
    let filter = (!relative.is_empty()).then(|| PathFilter::new(relative.clone()));
    let demand = ctx.demand();
    let mut wanted = query.count;
    if query.group_by.is_none()
        && let Some(limit) = demand.limit
    {
        wanted = wanted.min(limit);
    }
    let records = CommitRecords::new(repo, &demand.fields);
    let mut selector = Selector {
        records: &records,
        query,
        follow: query.follow.then(|| relative[0].clone()),
        filter: if query.follow { None } else { filter.clone() },
    };
    let commits = match &query.symbol {
        Some(symbol) => {
            let path = relative[0].clone();
            let range = symbol_range(repo, &tips, &query.paths[0], &path, symbol, ctx.code())?;
            let trace = LineTrace::new(repo, &tips, path, range, query.since)?;
            select(trace, &mut selector, wanted, query.skip, ctx)?
        }
        None => {
            let options = WalkOptions {
                simplify: if query.follow { None } else { filter },
                since: query.since,
                until: query.until,
            };
            let walk = Walk::new(repo, &tips, options)?;
            select(walk, &mut selector, wanted, query.skip, ctx)?
        }
    };
    match query.group_by {
        Some(group) => group_records(&records, &commits, group),
        None => Ok(commits
            .into_iter()
            .map(|(_, commit)| Record::from(commit))
            .collect()),
    }
}

fn select(
    ids: impl Iterator<Item = Result<ObjectId, Error>>,
    selector: &mut Selector<'_, '_>,
    wanted: u64,
    skip: u64,
    ctx: &ExecCtx,
) -> Result<Vec<(ObjectId, Commit)>, Error> {
    let mut out = Vec::new();
    let mut skipped = 0;
    if wanted == 0 {
        return Ok(out);
    }
    for id in ids {
        ctx.checkpoint()?;
        let id = id?;
        let Some(commit) = selector.select(id)? else {
            continue;
        };
        if skipped < skip {
            skipped += 1;
            continue;
        }
        out.push((id, commit));
        if out.len() as u64 >= wanted {
            break;
        }
    }
    Ok(out)
}

/// Applies the filters of a query to one commit at a time, in history
/// order; `follow` changes the followed path as renames are found.
struct Selector<'q, 'r> {
    records: &'q CommitRecords<'r>,
    query: &'q Query,
    follow: Option<BString>,
    filter: Option<PathFilter>,
}

impl Selector<'_, '_> {
    fn select(&mut self, id: ObjectId) -> Result<Option<Commit>, Error> {
        let mut commit = self.records.commit(id)?;
        if !self.text_matches(&commit) {
            return Ok(None);
        }
        if self.query.until.is_some_and(|until| {
            commit
                .commit_time
                .is_some_and(|time| time.as_unix_nanos().div_euclid(1_000_000_000) > until)
        }) {
            return Ok(None);
        }
        let needs_changes =
            self.follow.is_some() || self.query.pickaxe.is_some() || self.query.stat;
        if !needs_changes {
            return Ok(Some(commit));
        }
        let repo = self.records.repo();
        let info = repo
            .commits
            .info(&repo.repo, id)
            .map_err(|err| repo.read_error(format_args!("commit {id}"), err))?;
        if info.parents.len() > 1 {
            let filtered = self.follow.is_some() || self.query.pickaxe.is_some();
            return Ok((!filtered).then_some(commit));
        }
        let old = match info.parents.first() {
            Some(parent) => Some(repo.tree_of(*parent)?),
            None => None,
        };
        let mut found = match &self.follow {
            Some(path) => {
                let changes = self.followed_changes(old, info.tree, path.clone())?;
                if changes.is_empty() {
                    return Ok(None);
                }
                changes
            }
            None => changes::tree_changes(
                repo,
                old,
                info.tree,
                self.query.detect,
                self.filter.as_ref(),
            )?,
        };
        found.retain(|change| !excluded(&self.query.exclude, change.path.as_ref()));
        if let Some(pickaxe) = &self.query.pickaxe {
            found = self.picked(pickaxe, found)?;
            if found.is_empty() {
                return Ok(None);
            }
        }
        if self.query.stat {
            let (files, stats) = self.records.file_stats(&found)?;
            commit.files = Some(files);
            commit.stats = Some(stats);
        }
        Ok(Some(commit))
    }

    fn text_matches(&self, commit: &Commit) -> bool {
        if let Some(author) = &self.query.author {
            let identity = format!(
                "{} <{}>",
                commit.author,
                commit.author_email.as_deref().unwrap_or_default()
            );
            if !author.is_match(&identity) {
                return false;
            }
        }
        self.query
            .message
            .as_ref()
            .is_none_or(|message| commit.message.lines().any(|line| message.is_match(line)))
    }

    /// Changes of the followed file; when the file is new in the commit and
    /// was renamed or copied from another file, the other file is followed
    /// from here on.
    fn followed_changes(
        &mut self,
        old: Option<ObjectId>,
        new: ObjectId,
        path: BString,
    ) -> Result<Vec<FileChange>, Error> {
        let repo = self.records.repo();
        let limit = PathFilter::new(vec![path.clone()]);
        let mut found = changes::tree_changes(repo, old, new, Detect::Nothing, Some(&limit))?;
        let added = found
            .iter()
            .any(|change| change.path == path && change.old.is_none());
        if added && let Some(origin) = changes::origin_of(repo, old, new, path.as_ref())? {
            self.follow = Some(origin.source_path().to_owned());
            found = vec![origin];
        }
        Ok(found)
    }

    fn picked(&self, pickaxe: &Pickaxe, found: Vec<FileChange>) -> Result<Vec<FileChange>, Error> {
        let mut kept = Vec::new();
        for change in found {
            if is_submodule(change.old) || is_submodule(change.new) {
                continue;
            }
            let old = self.records.content(change.old)?;
            let new = self.records.content(change.new)?;
            if lines::is_binary(&old) || lines::is_binary(&new) {
                continue;
            }
            if pickaxe_matches(pickaxe, &old, &new) {
                kept.push(change);
            }
        }
        Ok(kept)
    }
}

fn is_submodule(side: Option<changes::Side>) -> bool {
    side.is_some_and(|side| side.mode.kind() == gix::objs::tree::EntryKind::Commit)
}

fn pickaxe_matches(pickaxe: &Pickaxe, old: &[u8], new: &[u8]) -> bool {
    match pickaxe {
        Pickaxe::Text(needle) => old.find_iter(needle).count() != new.find_iter(needle).count(),
        Pickaxe::Pattern(pattern) => {
            pattern.find_iter(old).count() != pattern.find_iter(new).count()
        }
        Pickaxe::Lines(pattern) => {
            let old_lines = lines::split_lines(old);
            let new_lines = lines::split_lines(new);
            let strip =
                |line: &[u8]| -> Vec<u8> { line.strip_suffix(b"\n").unwrap_or(line).to_vec() };
            lines::edits(old, new).iter().any(|edit| {
                let removed = edit
                    .before
                    .clone()
                    .filter_map(|index| old_lines.get(index as usize));
                let added = edit
                    .after
                    .clone()
                    .filter_map(|index| new_lines.get(index as usize));
                removed
                    .chain(added)
                    .any(|line| pattern.is_match(&strip(line)))
            })
        }
    }
}

fn group_records(
    records: &CommitRecords<'_>,
    commits: &[(ObjectId, Commit)],
    group: GroupBy,
) -> Result<Vec<Record>, Error> {
    let mut counts: HashMap<String, i64> = HashMap::new();
    for (id, commit) in commits {
        let key = match group {
            GroupBy::Type => commit_type(&commit.subject),
            GroupBy::Author => commit.author.clone(),
            GroupBy::Day => day_of(&records.author(*id)?),
        };
        *counts.entry(key).or_insert(0) += 1;
    }
    let mut sorted: Vec<(String, i64)> = counts.into_iter().collect();
    sorted.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    Ok(sorted
        .into_iter()
        .map(|(key, count)| {
            Record::new(RecordKind::Map)
                .with("key", Value::Text(key))
                .with("count", Value::Int(count))
        })
        .collect())
}

/// Type of a Conventional Commits subject, `other` for other subjects.
fn commit_type(subject: &str) -> String {
    let head = subject.split(':').next().unwrap_or_default();
    if head.len() == subject.len() {
        return "other".to_owned();
    }
    let head = head.strip_suffix('!').unwrap_or(head);
    let name = match head.find('(') {
        Some(open) if head.ends_with(')') => &head[..open],
        Some(_) => return "other".to_owned(),
        None => head,
    };
    if !name.is_empty() && name.chars().all(|ch| ch.is_ascii_alphabetic()) {
        name.to_ascii_lowercase()
    } else {
        "other".to_owned()
    }
}

/// Date of the author time at the UTC offset recorded with it, `YYYY-MM-DD`.
fn day_of(person: &Person) -> String {
    let local = person.seconds + i64::from(person.offset);
    let days = local.div_euclid(SECONDS_PER_DAY);
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02}")
}

/// Proleptic Gregorian date of a day counted from 1970-01-01.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = u32::try_from(doy - (153 * mp + 2) / 5 + 1).unwrap_or(1);
    let month = u32::try_from(if mp < 10 { mp + 3 } else { mp - 9 }).unwrap_or(1);
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// Lines, counted from 0 with an exclusive end, of the definition of
/// `symbol` in `path` at the first listed revision.
fn symbol_range(
    repo: &OpenRepo,
    tips: &Tips,
    given: &str,
    path: &BString,
    symbol: &str,
    code: Option<&dyn CodeService>,
) -> Result<Range<u32>, Error> {
    let code = code.ok_or_else(|| {
        Error::new(
            ErrorCode::Unsupported,
            "`symbol` needs the code service, which this host does not provide",
        )
        .with_next_step(format!(
            "git.log {} path: {}",
            quote(repo.label()),
            quote(given)
        ))
    })?;
    let tip = tips.include[0];
    let tree = repo.tree_of(tip)?;
    let entry =
        entry_at(repo, tree, path.as_ref()).map_err(|err| repo.read_error("the file", err))?;
    let Some((_, blob)) = entry.filter(|(mode, _)| !mode.is_tree()) else {
        return Err(Error::new(
            ErrorCode::NotFound,
            format!("`{given}` does not exist at the listed revision"),
        )
        .with_next_step(format!(
            "git.log {} path: {}",
            quote(repo.label()),
            quote(given)
        )));
    };
    let bytes = repo.blob(blob)?;
    let file = Path::new(given);
    let language = code.language_of(Some(file), &bytes).ok_or_else(|| {
        Error::new(
            ErrorCode::UnsupportedLanguage,
            format!("the language of `{given}` has no grammar, so `symbol` cannot be found"),
        )
        .with_next_step(format!(
            "git.log {} path: {}",
            quote(repo.label()),
            quote(given)
        ))
    })?;
    let found = code.find_symbol(&bytes, &language, symbol)?;
    match found.as_slice() {
        [] => Err(Error::new(
            ErrorCode::SymbolNotFound,
            format!("`{symbol}` is not defined in `{given}`"),
        )
        .with_found(symbol)
        .with_next_step(format!("read {} view: outline", quote(given)))),
        [only] => {
            let start = only.span.start.line.saturating_sub(1);
            let end = if only.span.end.column == 1 && only.span.end.line > only.span.start.line {
                only.span.end.line - 1
            } else {
                only.span.end.line
            };
            Ok(start..end.max(start + 1))
        }
        many => {
            let mut error = Error::new(
                ErrorCode::SymbolAmbiguous,
                format!("`{symbol}` is defined {} times in `{given}`", many.len()),
            )
            .with_found(symbol);
            for (rank, candidate) in many.iter().enumerate() {
                let qualified = candidate.container.as_ref().map_or_else(
                    || candidate.name.clone(),
                    |container| format!("{container}.{}", candidate.name),
                );
                error = error.with_candidate(Candidate {
                    rank: u32::try_from(rank + 1).unwrap_or(u32::MAX),
                    reference: Some(format!("{given}:{}", candidate.span.start.line)),
                    program: format!(
                        "git.log {} path: {} symbol: {}",
                        quote(repo.label()),
                        quote(given),
                        quote(&qualified)
                    ),
                    why: None,
                });
            }
            Err(error)
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
struct Traced {
    time: i64,
    order: Reverse<u64>,
    id: ObjectId,
}

impl Ord for Traced {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.time, self.order).cmp(&(other.time, other.order))
    }
}

impl PartialOrd for Traced {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// History of a range of lines, as `git log -L` follows it: a commit is
/// listed when it changes the lines; the lines are carried to the parents
/// through the differences, and through a single parent when a merge took
/// them unchanged from it.
struct LineTrace<'r> {
    repo: &'r OpenRepo,
    queue: BinaryHeap<Traced>,
    next_order: u64,
    ranges: HashMap<ObjectId, (BString, Vec<Range<u32>>)>,
    hidden: HashSet<ObjectId>,
    since: Option<i64>,
    failed: bool,
}

impl<'r> LineTrace<'r> {
    fn new(
        repo: &'r OpenRepo,
        tips: &Tips,
        path: BString,
        range: Range<u32>,
        since: Option<i64>,
    ) -> Result<Self, Error> {
        let mut hidden = HashSet::new();
        if !tips.exclude.is_empty() {
            let excluded = Tips {
                include: tips.exclude.clone(),
                exclude: Vec::new(),
            };
            for id in Walk::new(repo, &excluded, WalkOptions::default())? {
                hidden.insert(id?);
            }
        }
        let mut trace = Self {
            repo,
            queue: BinaryHeap::new(),
            next_order: 0,
            ranges: HashMap::new(),
            hidden,
            since,
            failed: false,
        };
        trace.give(tips.include[0], path, vec![range])?;
        Ok(trace)
    }

    fn give(&mut self, id: ObjectId, path: BString, ranges: Vec<Range<u32>>) -> Result<(), Error> {
        if ranges.is_empty() || self.hidden.contains(&id) {
            return Ok(());
        }
        match self.ranges.get_mut(&id) {
            Some((_, existing)) => {
                existing.extend(ranges);
                *existing = union(std::mem::take(existing));
            }
            None => {
                let time = self
                    .repo
                    .commits
                    .info(&self.repo.repo, id)
                    .map_err(|err| self.repo.read_error(format_args!("commit {id}"), err))?
                    .time;
                self.ranges.insert(id, (path, union(ranges)));
                self.queue.push(Traced {
                    time,
                    order: Reverse(self.next_order),
                    id,
                });
                self.next_order += 1;
            }
        }
        Ok(())
    }

    fn step(&mut self) -> Result<Option<ObjectId>, Error> {
        while let Some(Traced { id, time, .. }) = self.queue.pop() {
            if self.since.is_some_and(|since| time < since) {
                continue;
            }
            let Some((path, ranges)) = self.ranges.get(&id).cloned() else {
                continue;
            };
            if self.process(id, &path, &ranges)? {
                return Ok(Some(id));
            }
        }
        Ok(None)
    }

    /// Carries the ranges of `id` to its parents and tells whether `id`
    /// changed them.
    fn process(
        &mut self,
        id: ObjectId,
        path: &BString,
        ranges: &[Range<u32>],
    ) -> Result<bool, Error> {
        let repo = self.repo;
        let info = repo
            .commits
            .info(&repo.repo, id)
            .map_err(|err| repo.read_error(format_args!("commit {id}"), err))?;
        let new_blob = match entry_at(repo, info.tree, path.as_ref())
            .map_err(|err| repo.read_error("the file", err))?
        {
            Some((_, blob)) => blob,
            None => return Ok(false),
        };
        if info.parents.is_empty() {
            return Ok(true);
        }
        let new = repo.blob(new_blob)?;
        let mut carried = Vec::with_capacity(info.parents.len());
        for parent in &info.parents {
            let parent_tree = repo.tree_of(*parent)?;
            let (old_path, old_blob) = match entry_at(repo, parent_tree, path.as_ref())
                .map_err(|err| repo.read_error("the file", err))?
            {
                Some((_, blob)) => (path.clone(), Some(blob)),
                None => {
                    match changes::origin_of(repo, Some(parent_tree), info.tree, path.as_ref())? {
                        Some(origin) => (
                            origin.source_path().to_owned(),
                            origin.old.map(|side| side.id),
                        ),
                        None => (path.clone(), None),
                    }
                }
            };
            let Some(old_blob) = old_blob else {
                carried.push((*parent, old_path, Vec::new(), true));
                continue;
            };
            if old_blob == new_blob {
                carried.push((*parent, old_path, ranges.to_vec(), false));
                continue;
            }
            let edits = lines::edits(&repo.blob(old_blob)?, &new);
            let mut touched = false;
            let mut mapped = Vec::new();
            for range in ranges {
                let (old_range, changed) = lines::map_to_old(&edits, range.clone());
                touched |= changed;
                if !old_range.is_empty() {
                    mapped.push(old_range);
                }
            }
            carried.push((*parent, old_path, mapped, touched));
        }
        if let Some((parent, old_path, mapped, _)) =
            carried.iter().find(|(_, _, _, touched)| !touched)
        {
            self.give(*parent, old_path.clone(), mapped.clone())?;
            return Ok(false);
        }
        for (parent, old_path, mapped, _) in carried {
            self.give(parent, old_path, mapped)?;
        }
        Ok(true)
    }
}

impl Iterator for LineTrace<'_> {
    type Item = Result<ObjectId, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.failed {
            return None;
        }
        let item = self.step().transpose();
        if matches!(item, Some(Err(_))) {
            self.failed = true;
        }
        item
    }
}

fn union(mut ranges: Vec<Range<u32>>) -> Vec<Range<u32>> {
    ranges.sort_by_key(|range| (range.start, range.end));
    let mut out: Vec<Range<u32>> = Vec::with_capacity(ranges.len());
    for range in ranges {
        match out.last_mut() {
            Some(last) if range.start <= last.end => last.end = last.end.max(range.end),
            _ => out.push(range),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::revwalk::fixture::{Fixture, LineCode, args as bound, exec_ctx, run as execute};
    use nurt_contracts::value::{Instant, Regex};
    use std::sync::Arc;

    fn log(fixture: &Fixture, values: &[(&str, Value)]) -> Vec<Record> {
        let mut all = vec![("repo", Value::Path(fixture.root()))];
        all.extend(values.iter().cloned());
        let args = bound(COMMAND, &all);
        execute(&GitLog, &args, &exec_ctx(Some(Arc::new(LineCode)))).unwrap()
    }

    fn log_error(fixture: &Fixture, values: &[(&str, Value)]) -> ErrorCode {
        let mut all = vec![("repo", Value::Path(fixture.root()))];
        all.extend(values.iter().cloned());
        let args = bound(COMMAND, &all);
        execute(&GitLog, &args, &exec_ctx(None)).unwrap_err().code()
    }

    fn subjects(records: &[Record]) -> Vec<String> {
        records
            .iter()
            .map(|record| match record.get("subject") {
                Some(Value::Text(text)) => text.clone(),
                other => panic!("subject missing: {other:?}"),
            })
            .collect()
    }

    fn text(value: &str) -> Value {
        Value::Text(value.to_owned())
    }

    fn linear() -> Fixture {
        let mut fixture = Fixture::new();
        fixture.commit(
            "feat: add a",
            &[(
                "a.txt",
                Some("one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\n"),
            )],
        );
        fixture.commit_as(
            ("Bob", "bob@example.com"),
            "fix(core): copy b",
            &[(
                "b.txt",
                Some("one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\n"),
            )],
        );
        fixture.commit(
            "docs: change a",
            &[(
                "a.txt",
                Some("one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine foo\n"),
            )],
        );
        fixture.commit(
            "rename a to z",
            &[
                ("a.txt", None),
                (
                    "z.txt",
                    Some("one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine foo\n"),
                ),
            ],
        );
        fixture.commit(
            "change z",
            &[(
                "z.txt",
                Some("one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine foo\nten\n"),
            )],
        );
        fixture
    }

    #[test]
    fn lists_commits_newest_first_with_count_and_skip() {
        let fixture = linear();
        assert_eq!(
            subjects(&log(&fixture, &[])),
            [
                "change z",
                "rename a to z",
                "docs: change a",
                "fix(core): copy b",
                "feat: add a"
            ]
        );
        assert_eq!(
            subjects(&log(
                &fixture,
                &[("count", Value::Int(2)), ("skip", Value::Int(1))]
            )),
            ["rename a to z", "docs: change a"]
        );
        assert!(log(&fixture, &[("count", Value::Int(0))]).is_empty());
        assert_eq!(
            subjects(&log(&fixture, &[("revision", text("HEAD~3..HEAD~1"))])),
            ["rename a to z", "docs: change a"]
        );
    }

    #[test]
    fn empty_repository_has_no_commits() {
        let fixture = Fixture::new();
        assert!(log(&fixture, &[]).is_empty());
    }

    #[test]
    fn limits_history_to_paths() {
        let fixture = linear();
        let z = fixture.path("z.txt");
        assert_eq!(
            subjects(&log(
                &fixture,
                &[("path", Value::List(vec![Value::Path(z.clone())]))]
            )),
            ["change z", "rename a to z"]
        );
        assert_eq!(
            subjects(&log(
                &fixture,
                &[("path", Value::Path(z)), ("follow", Value::Bool(true))]
            )),
            ["change z", "rename a to z", "docs: change a", "feat: add a"]
        );
        let b = fixture.path("b.txt");
        assert_eq!(
            subjects(&log(
                &fixture,
                &[("path", Value::Path(b)), ("follow", Value::Bool(true))]
            )),
            ["fix(core): copy b", "feat: add a"]
        );
    }

    #[test]
    fn filters_by_author_message_and_dates() {
        let fixture = linear();
        assert_eq!(
            subjects(&log(&fixture, &[("author", text("^Bob <"))])),
            ["fix(core): copy b"]
        );
        assert_eq!(
            subjects(&log(
                &fixture,
                &[("message", Value::Regex(Regex::new("CHANGE", "i").unwrap()))]
            )),
            ["change z", "docs: change a"]
        );
        let second = fixture.resolve("HEAD~3");
        let time = fixture.time_of(second);
        let since = Value::Instant(Instant::from_unix_seconds(time).unwrap());
        assert_eq!(log(&fixture, &[("since", since.clone())]).len(), 4);
        assert_eq!(
            subjects(&log(&fixture, &[("until", since)])),
            ["fix(core): copy b", "feat: add a"]
        );
    }

    #[test]
    fn pickaxe_finds_commits_that_change_occurrences() {
        let fixture = linear();
        assert_eq!(
            subjects(&log(&fixture, &[("pickaxe", text("foo"))])),
            ["docs: change a"]
        );
        assert_eq!(
            subjects(&log(
                &fixture,
                &[("pickaxe", Value::Regex(Regex::new("^ten$", "m").unwrap()))]
            )),
            ["change z"]
        );
        assert_eq!(
            subjects(&log(&fixture, &[("diff_regex", text("^nine"))])),
            ["docs: change a"]
        );
        assert_eq!(
            log_error(&fixture, &[("pickaxe", text(""))]),
            ErrorCode::InvalidValue
        );
        assert_eq!(
            log_error(
                &fixture,
                &[("pickaxe", text("a")), ("diff_regex", text("b"))]
            ),
            ErrorCode::InvalidValue
        );
    }

    #[test]
    fn stat_lists_changed_files() {
        let fixture = linear();
        let records = log(
            &fixture,
            &[("stat", Value::Bool(true)), ("count", Value::Int(2))],
        );
        let commit = Commit::try_from(records[1].clone()).unwrap();
        let files = commit.files.unwrap();
        assert_eq!(files[0].path, "z.txt");
        assert_eq!(files[0].old_path.as_deref(), Some("a.txt"));
        assert_eq!(commit.stats.unwrap().files, 1);
    }

    #[test]
    fn find_renames_and_find_copies_control_the_pairing_of_files() {
        let mut fixture = Fixture::new();
        let text = "one line of text\ntwo lines of text\nthree lines of text\nfour lines of text\n";
        fixture.commit("first", &[("a.txt", Some(text)), ("b.txt", Some("b\n"))]);
        fixture.commit(
            "second",
            &[
                ("a.txt", Some(&format!("{text}more\n"))),
                ("c.txt", Some(text)),
                ("b.txt", None),
                ("d.txt", Some("d\n")),
            ],
        );
        let files = |values: &[(&str, Value)]| {
            let mut all = vec![("stat", Value::Bool(true)), ("count", Value::Int(1))];
            all.extend(values.iter().cloned());
            let commit = Commit::try_from(log(&fixture, &all)[0].clone()).unwrap();
            commit
                .files
                .unwrap()
                .into_iter()
                .map(|file| (file.path, file.status))
                .collect::<Vec<_>>()
        };
        let added = (
            "c.txt".to_owned(),
            nurt_contracts::record::GitFileState::Added,
        );
        let copied = (
            "c.txt".to_owned(),
            nurt_contracts::record::GitFileState::Copied,
        );
        assert!(files(&[]).contains(&added));
        assert!(files(&[("find_copies", Value::Bool(true))]).contains(&copied));
        assert!(files(&[("find_renames", Value::Bool(false))]).contains(&added));
    }

    /// Number of commits of the history used to compare the speed of the
    /// command with `git log`.
    const LARGE_HISTORY: u32 = 100_000;

    /// Fields of a `commit` record that `git log` is asked for: identifiers,
    /// authors, times, parents, tree, names of references and message.
    const LOG_FORMAT: &str =
        "--format=%H%x00%h%x00%an%x00%ae%x00%at%x00%cn%x00%ce%x00%ct%x00%P%x00%T%x00%D%x00%s%x00%B";

    /// Slowest accepted time of the command, as a multiple of the time of
    /// `git log`.
    const SLOWDOWN_LIMIT: f64 = 2.0;

    /// A linear history of `LARGE_HISTORY` commits, each changing one file,
    /// created with `git fast-import` and indexed by a commit graph.
    #[allow(
        clippy::disallowed_methods,
        reason = "the measurement compares with the git program, which the test starts directly"
    )]
    fn large_repository(root: &std::path::Path) {
        use std::io::Write as _;
        use std::process::{Command, Stdio};

        let git = |args: &[&str]| {
            let status = Command::new("git")
                .args(args)
                .current_dir(root)
                .env("LC_ALL", "C")
                .status()
                .unwrap();
            assert!(status.success(), "git {args:?} failed");
        };
        git(&["init", "-q", "-b", "main"]);
        let mut stream = Vec::new();
        for number in 1..=LARGE_HISTORY {
            let message = format!("commit {number}");
            let content = format!("line {number}\n");
            let parent = if number > 1 {
                format!("from :{}\n", number - 1)
            } else {
                String::new()
            };
            write!(
                stream,
                "commit refs/heads/main\nmark :{number}\ncommitter A <a@example.com> {} +0000\ndata {}\n{message}\n{parent}M 100644 inline f{}.txt\ndata {}\n{content}\n",
                1_700_000_000 + i64::from(number),
                message.len(),
                number % 50,
                content.len(),
            )
            .unwrap();
        }
        let mut import = Command::new("git")
            .args(["fast-import", "--quiet"])
            .current_dir(root)
            .stdin(Stdio::piped())
            .spawn()
            .unwrap();
        import.stdin.take().unwrap().write_all(&stream).unwrap();
        assert!(import.wait().unwrap().success());
        git(&["commit-graph", "write", "--reachable"]);
    }

    #[test]
    #[ignore = "creates a history of 100000 commits and needs git; run with --release --ignored"]
    #[allow(
        clippy::disallowed_methods,
        reason = "the measurement compares with the git program, which the test starts directly"
    )]
    fn lists_a_history_of_one_hundred_thousand_commits_within_twice_the_time_of_git() {
        use std::time::Instant as Clock;

        let dir = tempfile::tempdir().unwrap();
        large_repository(dir.path());
        let root = dir.path().display().to_string();
        let measure = |run: &dyn Fn()| {
            run();
            let start = Clock::now();
            run();
            start.elapsed().as_secs_f64()
        };
        let git_seconds = measure(&|| {
            let output = std::process::Command::new("git")
                .args(["log", LOG_FORMAT])
                .current_dir(dir.path())
                .output()
                .unwrap();
            assert!(output.status.success());
        });
        let args = bound(
            COMMAND,
            &[
                ("repo", Value::Path(root)),
                ("count", Value::Int(i64::from(LARGE_HISTORY))),
            ],
        );
        let nurt_seconds = measure(&|| {
            let records = execute(&GitLog, &args, &exec_ctx(None)).unwrap();
            assert_eq!(records.len(), LARGE_HISTORY as usize);
        });
        eprintln!(
            "git log: {git_seconds:.3} s, git.log: {nurt_seconds:.3} s, ratio {:.2}",
            nurt_seconds / git_seconds
        );
        assert!(
            nurt_seconds <= git_seconds * SLOWDOWN_LIMIT,
            "git.log took {nurt_seconds:.3} s, git log {git_seconds:.3} s"
        );
    }

    #[test]
    fn exclude_leaves_files_out_of_stat_and_pickaxe() {
        let fixture = linear();
        let exclude = Value::List(vec![Value::Glob("z.*".into())]);
        assert_eq!(
            subjects(&log(&fixture, &[("pickaxe", text("ten"))])),
            ["change z"]
        );
        assert!(
            log(
                &fixture,
                &[("pickaxe", text("ten")), ("exclude", exclude.clone())]
            )
            .is_empty()
        );
        let records = log(
            &fixture,
            &[
                ("stat", Value::Bool(true)),
                ("count", Value::Int(1)),
                ("exclude", exclude),
            ],
        );
        let commit = Commit::try_from(records[0].clone()).unwrap();
        assert!(commit.files.unwrap().is_empty());
    }

    #[test]
    fn groups_commits() {
        let fixture = linear();
        let groups = log(&fixture, &[("group_by", Value::Enum("type".into()))]);
        let pairs: Vec<(Value, Value)> = groups
            .iter()
            .map(|g| {
                (
                    g.get("key").cloned().unwrap(),
                    g.get("count").cloned().unwrap(),
                )
            })
            .collect();
        assert_eq!(pairs[0], (text("other"), Value::Int(2)));
        assert_eq!(pairs.len(), 4);
        let authors = log(&fixture, &[("group_by", Value::Enum("author".into()))]);
        assert_eq!(authors[0].get("key"), Some(&text("Author")));
        let days = log(&fixture, &[("group_by", Value::Enum("day".into()))]);
        assert_eq!(days[0].get("key"), Some(&text("2026-01-01")));
    }

    #[test]
    fn rejects_inconsistent_arguments() {
        let fixture = linear();
        assert_eq!(
            log_error(&fixture, &[("follow", Value::Bool(true))]),
            ErrorCode::MissingParam
        );
        let two = Value::List(vec![
            Value::Path(fixture.path("a")),
            Value::Path(fixture.path("b")),
        ]);
        assert_eq!(
            log_error(&fixture, &[("follow", Value::Bool(true)), ("path", two)]),
            ErrorCode::InvalidValue
        );
        let one = Value::Path(fixture.path("z.txt"));
        assert_eq!(
            log_error(
                &fixture,
                &[
                    ("symbol", text("f")),
                    ("path", one.clone()),
                    ("stat", Value::Bool(true))
                ]
            ),
            ErrorCode::InvalidValue
        );
        assert_eq!(
            log_error(&fixture, &[("symbol", text("f")), ("path", one)]),
            ErrorCode::Unsupported
        );
        assert_eq!(
            log_error(&fixture, &[("group_by", Value::Enum("week".into()))]),
            ErrorCode::InvalidValue
        );
        assert_eq!(
            log_error(&fixture, &[("revision", text("missing"))]),
            ErrorCode::GitRevision
        );
        assert_eq!(
            log_error(&fixture, &[("pickaxe", Value::Int(1))]),
            ErrorCode::Type
        );
    }

    #[test]
    fn symbol_follows_the_lines_of_a_function() {
        let mut fixture = Fixture::new();
        fixture.commit(
            "add",
            &[("lib.rs", Some("fn a() {\n    1\n}\n\nfn b() {\n    2\n}\n"))],
        );
        fixture.commit(
            "change b",
            &[("lib.rs", Some("fn a() {\n    1\n}\n\nfn b() {\n    3\n}\n"))],
        );
        fixture.commit(
            "change a",
            &[(
                "lib.rs",
                Some("fn a() {\n    10\n}\n\nfn b() {\n    3\n}\n"),
            )],
        );
        fixture.commit(
            "move",
            &[(
                "lib.rs",
                Some("// top\nfn a() {\n    10\n}\n\nfn b() {\n    3\n}\n"),
            )],
        );
        let path = Value::Path(fixture.path("lib.rs"));
        assert_eq!(
            subjects(&log(
                &fixture,
                &[("path", path.clone()), ("symbol", text("b"))]
            )),
            ["change b", "add"]
        );
        assert_eq!(
            subjects(&log(
                &fixture,
                &[("path", path.clone()), ("symbol", text("a"))]
            )),
            ["change a", "add"]
        );
        assert_eq!(
            log_error_with_code(&fixture, &[("path", path), ("symbol", text("missing"))]),
            ErrorCode::SymbolNotFound
        );
    }

    fn log_error_with_code(fixture: &Fixture, values: &[(&str, Value)]) -> ErrorCode {
        let mut all = vec![("repo", Value::Path(fixture.root()))];
        all.extend(values.iter().cloned());
        let args = bound(COMMAND, &all);
        execute(&GitLog, &args, &exec_ctx(Some(Arc::new(LineCode))))
            .unwrap_err()
            .code()
    }

    #[test]
    fn classifies_conventional_commit_types() {
        assert_eq!(commit_type("feat: x"), "feat");
        assert_eq!(commit_type("Fix(parser)!: x"), "fix");
        assert_eq!(commit_type("feat(a: x"), "other");
        assert_eq!(commit_type("release 1.0"), "other");
        assert_eq!(commit_type("v1.0: notes"), "other");
    }

    #[test]
    fn computes_local_days() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(-1), (1969, 12, 31));
        assert_eq!(civil_from_days(20_454), (2026, 1, 1));
        let person = Person {
            name: String::new(),
            email: String::new(),
            seconds: 20_454 * SECONDS_PER_DAY - 3600,
            offset: 7200,
        };
        assert_eq!(day_of(&person), "2026-01-01");
    }

    #[test]
    fn merges_overlapping_ranges() {
        assert_eq!(union(vec![5..7, 0..2, 1..3, 7..8]), vec![0..3, 5..8]);
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
