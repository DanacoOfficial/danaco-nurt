// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Traversal of commits in the default order of `git log`.
//!
//! Commits wait in a queue ordered by committer time, newest first; commits
//! with equal times leave the queue in the order they entered it. Without
//! excluded revisions commits are produced as they leave the queue. With
//! excluded revisions the traversal first separates the listed commits from
//! the excluded ones, stopping once only excluded commits remain in the
//! queue for several steps, and then produces the listed commits in the
//! order they were reached.

use std::cmp::{Ordering, Reverse};
use std::collections::{BinaryHeap, HashMap, VecDeque};

use gix::ObjectId;
use nurt_contracts::error::Error;
use smallvec::SmallVec;

use super::graph::CommitInfo;
use super::paths::PathFilter;
use super::repo::OpenRepo;
use super::spec::Tips;

/// Steps the separation continues after the queue holds only excluded
/// commits, to tolerate commits whose times are out of order.
const SLOP: u32 = 5;

const SEEN: u8 = 1;
const UNINTERESTING: u8 = 2;
const ADDED: u8 = 4;
const TREESAME: u8 = 8;

/// Limits of the traversal.
#[derive(Debug, Clone, Default)]
pub(crate) struct WalkOptions {
    /// Hide commits that do not change these paths and follow a merge
    /// through a parent equal to it within them.
    pub(crate) simplify: Option<PathFilter>,
    /// Committer time, in seconds, before which the traversal stops.
    pub(crate) since: Option<i64>,
    /// Committer time, in seconds, after which commits are hidden.
    pub(crate) until: Option<i64>,
}

#[derive(Debug, PartialEq, Eq)]
struct Queued {
    time: i64,
    order: Reverse<u64>,
    id: ObjectId,
}

impl Ord for Queued {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.time, self.order).cmp(&(other.time, other.order))
    }
}

impl PartialOrd for Queued {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Iterator over the commits of a history.
pub(crate) struct Walk<'r> {
    repo: &'r OpenRepo,
    options: WalkOptions,
    queue: BinaryHeap<Queued>,
    next_order: u64,
    flags: HashMap<ObjectId, u8>,
    parents: HashMap<ObjectId, SmallVec<[ObjectId; 2]>>,
    separated: Option<VecDeque<ObjectId>>,
    failed: bool,
}

impl<'r> Walk<'r> {
    /// Starts a traversal of `tips`.
    pub(crate) fn new(
        repo: &'r OpenRepo,
        tips: &Tips,
        options: WalkOptions,
    ) -> Result<Self, Error> {
        let mut walk = Self {
            repo,
            options,
            queue: BinaryHeap::new(),
            next_order: 0,
            flags: HashMap::new(),
            parents: HashMap::new(),
            separated: None,
            failed: false,
        };
        for id in &tips.exclude {
            walk.mark_uninteresting(*id)?;
            walk.enqueue(*id, UNINTERESTING)?;
        }
        for id in &tips.include {
            walk.enqueue(*id, 0)?;
        }
        if !tips.exclude.is_empty() {
            walk.separate()?;
        }
        Ok(walk)
    }

    fn info(&self, id: ObjectId) -> Result<CommitInfo, Error> {
        self.repo
            .commits
            .info(&self.repo.repo, id)
            .map_err(|err| self.repo.read_error(format_args!("commit {id}"), err))
    }

    fn flag(&self, id: ObjectId) -> u8 {
        self.flags.get(&id).copied().unwrap_or(0)
    }

    fn enqueue(&mut self, id: ObjectId, flags: u8) -> Result<(), Error> {
        let entry = self.flags.entry(id).or_insert(0);
        let seen = *entry & SEEN != 0;
        *entry |= SEEN | flags;
        if !seen {
            let time = self.info(id)?.time;
            self.queue.push(Queued {
                time,
                order: Reverse(self.next_order),
                id,
            });
            self.next_order += 1;
        }
        Ok(())
    }

    /// Marks `id` and every ancestor already reached as excluded.
    fn mark_uninteresting(&mut self, id: ObjectId) -> Result<(), Error> {
        let mut stack = vec![id];
        while let Some(current) = stack.pop() {
            let entry = self.flags.entry(current).or_insert(0);
            let already = *entry & UNINTERESTING != 0 && current != id;
            *entry |= UNINTERESTING;
            if already || *entry & ADDED == 0 {
                continue;
            }
            match self.parents.get(&current) {
                Some(parents) => stack.extend(parents.iter().copied()),
                None => stack.extend(self.info(current)?.parents),
            }
        }
        Ok(())
    }

    fn process_parents(&mut self, id: ObjectId, info: &CommitInfo) -> Result<(), Error> {
        if self.flag(id) & ADDED != 0 {
            return Ok(());
        }
        *self.flags.entry(id).or_insert(0) |= ADDED;
        if self.flag(id) & UNINTERESTING != 0 {
            for parent in info.parents.clone() {
                self.mark_uninteresting(parent)?;
                self.enqueue(parent, UNINTERESTING)?;
            }
            self.parents.insert(id, info.parents.clone());
            return Ok(());
        }
        let followed = self.simplify(id, info)?;
        for parent in &followed {
            self.enqueue(*parent, 0)?;
        }
        self.parents.insert(id, followed);
        Ok(())
    }

    /// Decides whether `id` changes the paths and which parents to follow.
    fn simplify(
        &mut self,
        id: ObjectId,
        info: &CommitInfo,
    ) -> Result<SmallVec<[ObjectId; 2]>, Error> {
        let Some(filter) = self.options.simplify.clone() else {
            return Ok(info.parents.clone());
        };
        let same = |old: Option<ObjectId>| {
            filter.same_in(self.repo, old, info.tree).map_err(|err| {
                self.repo
                    .read_error(format_args!("the tree of commit {id}"), err)
            })
        };
        if info.parents.is_empty() {
            if same(None)? {
                *self.flags.entry(id).or_insert(0) |= TREESAME;
            }
            return Ok(SmallVec::new());
        }
        let mut relevant = 0;
        let mut relevant_changed = false;
        let mut other_changed = false;
        for parent in &info.parents {
            let is_relevant = self.flag(*parent) & UNINTERESTING == 0;
            if is_relevant {
                relevant += 1;
            }
            let parent_tree = self.info(*parent)?.tree;
            if same(Some(parent_tree))? {
                if is_relevant {
                    *self.flags.entry(id).or_insert(0) |= TREESAME;
                    return Ok(SmallVec::from_slice(&[*parent]));
                }
            } else if is_relevant {
                relevant_changed = true;
            } else {
                other_changed = true;
            }
        }
        let changed = if relevant > 0 {
            relevant_changed
        } else {
            other_changed
        };
        if !changed {
            *self.flags.entry(id).or_insert(0) |= TREESAME;
        }
        Ok(info.parents.clone())
    }

    fn too_old(&self, info: &CommitInfo) -> bool {
        self.options.since.is_some_and(|since| info.time < since)
    }

    fn shown(&self, id: ObjectId, info: &CommitInfo) -> bool {
        let flags = self.flag(id);
        if flags & UNINTERESTING != 0 {
            return false;
        }
        if self.options.simplify.is_some() && flags & TREESAME != 0 {
            return false;
        }
        !self.options.until.is_some_and(|until| info.time > until)
    }

    /// Separates listed from excluded commits before anything is produced.
    fn separate(&mut self) -> Result<(), Error> {
        let mut listed = VecDeque::new();
        let mut date = i64::MAX;
        let mut slop = SLOP;
        while let Some(Queued { id, .. }) = self.queue.pop() {
            let info = self.info(id)?;
            if self.too_old(&info) {
                *self.flags.entry(id).or_insert(0) |= UNINTERESTING;
            }
            self.process_parents(id, &info)?;
            if self.flag(id) & UNINTERESTING != 0 {
                self.mark_uninteresting(id)?;
                slop = self.still_interesting(date, slop);
                if slop > 0 {
                    continue;
                }
                break;
            }
            if self.options.until.is_some_and(|until| info.time > until) {
                continue;
            }
            date = info.time;
            listed.push_back(id);
        }
        self.separated = Some(listed);
        Ok(())
    }

    fn still_interesting(&self, date: i64, slop: u32) -> u32 {
        let Some(newest) = self.queue.peek() else {
            return 0;
        };
        if date <= newest.time {
            return SLOP;
        }
        if self
            .queue
            .iter()
            .any(|queued| self.flag(queued.id) & UNINTERESTING == 0)
        {
            return SLOP;
        }
        slop - 1
    }

    fn next_separated(&mut self) -> Option<Result<ObjectId, Error>> {
        loop {
            let id = self.separated.as_mut()?.pop_front()?;
            match self.info(id) {
                Ok(info) if self.shown(id, &info) => return Some(Ok(id)),
                Ok(_) => {}
                Err(err) => return Some(Err(err)),
            }
        }
    }

    fn next_queued(&mut self) -> Result<Option<ObjectId>, Error> {
        while let Some(Queued { id, .. }) = self.queue.pop() {
            let info = self.info(id)?;
            if self.too_old(&info) {
                continue;
            }
            self.process_parents(id, &info)?;
            if self.shown(id, &info) {
                return Ok(Some(id));
            }
        }
        Ok(None)
    }
}

impl Iterator for Walk<'_> {
    type Item = Result<ObjectId, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.failed {
            return None;
        }
        let item = if self.separated.is_some() {
            self.next_separated()
        } else {
            self.next_queued().transpose()
        };
        if matches!(item, Some(Err(_))) {
            self.failed = true;
        }
        item
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::revwalk::fixture::Fixture;
    use crate::revwalk::spec;
    use bstr::BString;

    fn walk(fixture: &Fixture, revision: &str, options: WalkOptions) -> Vec<String> {
        let repo = fixture.open();
        let tips = spec::resolve(&repo, revision).unwrap();
        let ids: Vec<ObjectId> = Walk::new(&repo, &tips, options)
            .unwrap()
            .map(Result::unwrap)
            .collect();
        ids.iter().map(|id| fixture.message_of(*id)).collect()
    }

    fn paths(list: &[&str]) -> WalkOptions {
        WalkOptions {
            simplify: Some(PathFilter::new(
                list.iter().map(|path| BString::from(*path)).collect(),
            )),
            ..WalkOptions::default()
        }
    }

    /// History in which `main other` and `topic f` both follow `base` and
    /// `merge` joins them, taking `f` from `topic f`.
    fn merged() -> Fixture {
        let mut fixture = Fixture::new();
        let base = fixture.commit("base", &[("f", Some("base\n")), ("other", Some("o\n"))]);
        let topic = fixture.commit_on(&[base], "topic f", &[("f", Some("base\nfoo\n"))]);
        fixture.branch("topic", topic);
        let main = fixture.commit_on(&[base], "main other", &[("other", Some("o\nm\n"))]);
        let merge = fixture.commit_on(&[main, topic], "merge", &[("f", Some("base\nfoo\n"))]);
        fixture.branch("main", merge);
        fixture
    }

    #[test]
    fn lists_newest_first_with_ties_in_reach_order() {
        let fixture = merged();
        assert_eq!(
            walk(&fixture, "HEAD", WalkOptions::default()),
            ["merge", "main other", "topic f", "base"]
        );
        let mut same_time = Fixture::new();
        same_time.freeze_clock();
        let base = same_time.commit("base", &[("a", Some("a\n"))]);
        let left = same_time.commit_on(&[base], "left", &[("b", Some("b\n"))]);
        let right = same_time.commit_on(&[base], "right", &[("c", Some("c\n"))]);
        let merge = same_time.commit_on(&[left, right], "merge", &[]);
        same_time.branch("main", merge);
        assert_eq!(
            walk(&same_time, "HEAD", WalkOptions::default()),
            ["merge", "left", "right", "base"]
        );
    }

    #[test]
    fn excluded_revisions_hide_their_ancestors() {
        let fixture = merged();
        assert!(walk(&fixture, "main..topic", WalkOptions::default()).is_empty());
        assert_eq!(
            walk(&fixture, "topic..main", WalkOptions::default()),
            ["merge", "main other"]
        );
        assert_eq!(
            walk(&fixture, "topic...main", WalkOptions::default()),
            ["merge", "main other"]
        );
        assert_eq!(walk(&fixture, "main^!", WalkOptions::default()), ["merge"]);
    }

    #[test]
    fn paths_hide_commits_that_do_not_change_them() {
        let fixture = merged();
        assert_eq!(walk(&fixture, "HEAD", paths(&["f"])), ["topic f", "base"]);
        assert_eq!(
            walk(&fixture, "HEAD", paths(&["other"])),
            ["main other", "base"]
        );
        assert_eq!(
            walk(&fixture, "HEAD", paths(&["missing"])),
            Vec::<String>::new()
        );
        assert_eq!(
            walk(&fixture, "topic..main", paths(&["other"])),
            ["main other"]
        );
    }

    #[test]
    fn merge_that_differs_from_every_parent_is_shown() {
        let mut fixture = merged();
        let head = fixture.head();
        let topic = fixture.resolve("topic");
        let main = fixture.resolve("HEAD~1");
        let evil = fixture.commit_on(&[main, topic], "evil merge", &[("f", Some("resolved\n"))]);
        fixture.branch("main", evil);
        assert_ne!(head, evil);
        assert_eq!(
            walk(&fixture, "HEAD", paths(&["f"])),
            ["evil merge", "topic f", "base"]
        );
    }

    #[test]
    fn since_stops_and_until_hides() {
        let fixture = merged();
        let topic = fixture.resolve("topic");
        let main_other = fixture.resolve("HEAD~1");
        let since = WalkOptions {
            since: Some(fixture.time_of(topic)),
            ..WalkOptions::default()
        };
        assert_eq!(
            walk(&fixture, "HEAD", since.clone()),
            ["merge", "main other", "topic f"]
        );
        assert_eq!(
            walk(&fixture, "topic..HEAD", since),
            ["merge", "main other"]
        );
        let until = WalkOptions {
            until: Some(fixture.time_of(main_other) - 1),
            ..WalkOptions::default()
        };
        assert_eq!(walk(&fixture, "HEAD", until.clone()), ["topic f", "base"]);
        assert_eq!(walk(&fixture, "HEAD ^topic", until), Vec::<String>::new());
    }
}
