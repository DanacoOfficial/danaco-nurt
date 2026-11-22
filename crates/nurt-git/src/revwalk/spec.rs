// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Revisions and ranges as written for `git log`: `A`, `A..B`, `A...B`,
//! `^A`, `A^@`, `A^!`, several separated by white space.

use gix::ObjectId;
use nurt_contracts::error::Error;

use super::repo::{OpenRepo, RevisionProblem};

/// Commits that start the traversal and commits whose ancestors are left out.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Tips {
    /// Commits listed with their ancestors.
    pub(crate) include: Vec<ObjectId>,
    /// Commits whose ancestors, themselves included, are left out.
    pub(crate) exclude: Vec<ObjectId>,
}

impl Tips {
    /// Whether nothing is to be listed.
    pub(crate) fn is_empty(&self) -> bool {
        self.include.is_empty()
    }
}

/// Resolves the revisions of `text`.
///
/// `HEAD` of a repository without commits resolves to nothing, so that the
/// history of a new repository is empty rather than an error.
pub(crate) fn resolve(repo: &OpenRepo, text: &str) -> Result<Tips, Error> {
    let mut tips = Tips::default();
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.is_empty() {
        return Err(repo.revision_error(text, RevisionProblem::InvalidSyntax));
    }
    for word in words {
        if word == "HEAD" && head_is_unborn(repo) {
            continue;
        }
        resolve_word(repo, word, &mut tips)?;
    }
    Ok(tips)
}

fn head_is_unborn(repo: &OpenRepo) -> bool {
    repo.repo.head().is_ok_and(|head| head.is_unborn())
}

fn resolve_word(repo: &OpenRepo, word: &str, tips: &mut Tips) -> Result<(), Error> {
    let spec = repo
        .repo
        .rev_parse(word)
        .map_err(|err| repo.revision_error(word, classify(&err.to_string())))?
        .detach();
    let commit = |id: ObjectId| {
        repo.peel_to_commit(id)
            .ok_or_else(|| repo.revision_error(word, RevisionProblem::NotACommit))
    };
    match spec {
        gix_revision::Spec::Include(id) => tips.include.push(commit(id)?),
        gix_revision::Spec::Exclude(id) => tips.exclude.push(commit(id)?),
        gix_revision::Spec::Range { from, to } => {
            tips.exclude.push(commit(from)?);
            tips.include.push(commit(to)?);
        }
        gix_revision::Spec::Merge { theirs, ours } => {
            let theirs = commit(theirs)?;
            let ours = commit(ours)?;
            tips.include.push(theirs);
            tips.include.push(ours);
            let bases = repo
                .repo
                .merge_bases_many(theirs, &[ours])
                .map_err(|err| repo.read_error(format_args!("the merge bases of `{word}`"), err))?;
            tips.exclude.extend(bases.into_iter().map(gix::Id::detach));
        }
        gix_revision::Spec::IncludeOnlyParents(id) => {
            let info = parents_of(repo, word, commit(id)?)?;
            tips.include.extend(info);
        }
        gix_revision::Spec::ExcludeParents(id) => {
            let id = commit(id)?;
            tips.exclude.extend(parents_of(repo, word, id)?);
            tips.include.push(id);
        }
    }
    Ok(())
}

fn parents_of(repo: &OpenRepo, word: &str, id: ObjectId) -> Result<Vec<ObjectId>, Error> {
    repo.commits
        .info(&repo.repo, id)
        .map(|info| info.parents.into_vec())
        .map_err(|err| repo.read_error(format_args!("the parents of `{word}`"), err))
}

fn classify(message: &str) -> RevisionProblem {
    if message.to_ascii_lowercase().contains("ambiguous") {
        RevisionProblem::Ambiguous
    } else {
        RevisionProblem::NotFound
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::revwalk::fixture::Fixture;
    use nurt_contracts::error::ErrorCode;

    fn branchy() -> (Fixture, [ObjectId; 4]) {
        let mut fixture = Fixture::new();
        let base = fixture.commit("base", &[("a.txt", Some("a\n"))]);
        let topic = fixture.commit_on(&[base], "topic", &[("b.txt", Some("b\n"))]);
        fixture.branch("topic", topic);
        let main = fixture.commit_on(&[base], "main", &[("c.txt", Some("c\n"))]);
        let merge = fixture.commit_on(&[main, topic], "merge", &[]);
        fixture.branch("main", merge);
        (fixture, [base, topic, main, merge])
    }

    #[test]
    fn resolves_ranges_and_exclusions() {
        let (fixture, [base, topic, main, merge]) = branchy();
        let repo = fixture.open();
        let two_dots = resolve(&repo, "topic..main").unwrap();
        assert_eq!(two_dots.include, vec![merge]);
        assert_eq!(two_dots.exclude, vec![topic]);
        let caret = resolve(&repo, "main ^topic").unwrap();
        assert_eq!(caret, two_dots);
        let symmetric = resolve(&repo, "topic...HEAD~1").unwrap();
        assert_eq!(symmetric.include, vec![topic, main]);
        assert_eq!(symmetric.exclude, vec![base]);
        let parents = resolve(&repo, "main^@").unwrap();
        assert_eq!(parents.include, vec![main, topic]);
        let only = resolve(&repo, "main^!").unwrap();
        assert_eq!(only.include, vec![merge]);
        assert_eq!(only.exclude, vec![main, topic]);
    }

    #[test]
    fn unborn_head_resolves_to_nothing() {
        let fixture = Fixture::new();
        let repo = fixture.open();
        assert!(resolve(&repo, "HEAD").unwrap().is_empty());
    }

    #[test]
    fn rejects_unknown_and_empty_revisions() {
        let (fixture, _) = branchy();
        let repo = fixture.open();
        assert_eq!(
            resolve(&repo, "nothing").unwrap_err().code(),
            ErrorCode::GitRevision
        );
        assert_eq!(
            resolve(&repo, "  ").unwrap_err().code(),
            ErrorCode::GitRevision
        );
        assert_eq!(
            resolve(&repo, "HEAD^{tree}").unwrap_err().code(),
            ErrorCode::GitRevision
        );
        assert_eq!(classify("prefix is ambiguous"), RevisionProblem::Ambiguous);
    }
}
