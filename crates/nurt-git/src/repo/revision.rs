// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Resolution of revisions written in the syntax of `git rev-parse`.
//!
//! The full syntax of the repository library is accepted: `~`, `^`,
//! `@{n}`, `@{upstream}`, `@{push}`, `:/text`, `<rev>:<path>` and
//! `^{type}`. A failure becomes `E_GIT_REVISION` with the problem
//! `not_found`, `ambiguous` or `invalid_syntax`; unknown names come with the
//! nearest reference names and ambiguous identifiers with the matching
//! objects.

use bstr::ByteSlice;
use nurt_contracts::error::Error;

use super::error::{self, MAX_CANDIDATES, RevisionProblem};
use super::GitRepo;

impl GitRepo {
    /// Resolves `spec` to an object identifier.
    ///
    /// # Errors
    ///
    /// `E_GIT_REVISION` when the revision is unknown, ambiguous or
    /// malformed; `E_IO` when the repository cannot be read.
    pub fn resolve_rev(&self, spec: &str) -> Result<gix::ObjectId, Error> {
        if spec.trim().is_empty() {
            return Err(error::revision(
                RevisionProblem::InvalidSyntax,
                spec,
                self.root(),
                &[],
                "the revision is empty",
            ));
        }
        self.gix()
            .rev_parse_single(spec)
            .map(gix::Id::detach)
            .map_err(|source| self.revision_error(spec, &source))
    }

    /// Resolves `spec` to a commit, peeling tags.
    ///
    /// # Errors
    ///
    /// As [`GitRepo::resolve_rev`]; `E_GIT_REVISION` with the problem
    /// `not_a_commit` when the object is a tree or a blob.
    pub fn resolve_commit(&self, spec: &str) -> Result<gix::ObjectId, Error> {
        let id = self.resolve_rev(spec)?;
        let object = self
            .gix()
            .find_object(id)
            .map_err(|source| error::from_gix("read an object", &source))?;
        let commit = object.peel_to_kind(gix::object::Kind::Commit).map_err(|_| {
            error::revision(
                RevisionProblem::InvalidSyntax,
                spec,
                self.root(),
                &[],
                "the revision does not name a commit",
            )
            .with_problem("not_a_commit")
        })?;
        Ok(commit.id)
    }

    fn revision_error(&self, spec: &str, source: &gix::Error) -> Error {
        use gix::revision::spec::parse::Error as ParseError;
        match source.downcast_any_ref::<ParseError>() {
            Some(ParseError::AmbiguousPrefix { candidates, .. })
            | Some(ParseError::AmbiguousRefAndObject { candidates, .. }) => {
                let listed: Vec<(String, String)> = candidates
                    .iter()
                    .map(|(prefix, info)| (prefix.to_string(), info.to_string()))
                    .collect();
                error::revision(RevisionProblem::Ambiguous, spec, self.root(), &listed, "")
            }
            Some(ParseError::MissingReference { .. }) => self.not_found(spec),
            _ if source.is_not_found() => self.not_found(spec),
            _ if source.is_validation() => error::revision(
                RevisionProblem::InvalidSyntax,
                spec,
                self.root(),
                &[],
                &first_line(&source.to_string()),
            ),
            _ => error::from_gix("resolve a revision", source),
        }
    }

    fn not_found(&self, spec: &str) -> Error {
        let names = self.reference_names();
        let candidates: Vec<(String, String)> = nearest_names(spec, &names)
            .into_iter()
            .map(|name| (name, "reference with a similar name".to_owned()))
            .collect();
        error::revision(RevisionProblem::NotFound, spec, self.root(), &candidates, "")
    }

    /// Short names of all branches, remote-tracking branches and tags.
    #[must_use]
    pub fn reference_names(&self) -> Vec<String> {
        let Ok(platform) = self.gix().references() else {
            return Vec::new();
        };
        let Ok(all) = platform.all() else {
            return Vec::new();
        };
        all.filter_map(Result::ok)
            .map(|reference| {
                let full = reference.name().as_bstr().to_str_lossy().into_owned();
                super::records::short_ref_name(&full).to_owned()
            })
            .collect()
    }
}

fn first_line(text: &str) -> String {
    text.lines().next().unwrap_or(text).to_owned()
}

/// Names from `names` close to `spec` by edit distance, nearest first, at
/// most [`MAX_CANDIDATES`].
///
/// The distance limit grows with the length of `spec`: one edit for every
/// three characters, at least one and at most three.
#[must_use]
pub fn nearest_names(spec: &str, names: &[String]) -> Vec<String> {
    const CHARS_PER_EDIT: usize = 3;
    const MAX_EDITS: usize = 3;
    let limit = (spec.chars().count() / CHARS_PER_EDIT).clamp(1, MAX_EDITS);
    let mut scored: Vec<(usize, &String)> = names
        .iter()
        .map(|name| (edit_distance(spec, name), name))
        .filter(|(distance, _)| *distance <= limit)
        .collect();
    scored.sort();
    scored.dedup_by(|a, b| a.1 == b.1);
    scored
        .into_iter()
        .take(MAX_CANDIDATES)
        .map(|(_, name)| name.clone())
        .collect()
}

/// Levenshtein distance between two texts, counted in characters.
#[must_use]
pub fn edit_distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    let mut current = vec![0; b.len() + 1];
    for (i, ca) in a.chars().enumerate() {
        current[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let substitution = previous[j] + usize::from(ca != *cb);
            current[j + 1] = substitution.min(previous[j + 1] + 1).min(current[j] + 1);
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{RepoBuilder, test_ctx};
    use nurt_contracts::error::ErrorCode;

    fn history() -> crate::testutil::Fixture {
        RepoBuilder::new()
            .file("a.txt", "1\n")
            .commit("one")
            .file("a.txt", "2\n")
            .commit("two")
            .reference("refs/heads/feature", "HEAD")
            .file("a.txt", "3\n")
            .commit("three")
            .build()
    }

    #[test]
    fn ancestry_and_branch_names_resolve() {
        let fixture = history();
        let repo = GitRepo::open(fixture.path(), &test_ctx()).unwrap();
        let head = repo.resolve_rev("HEAD").unwrap();
        assert_eq!(repo.resolve_rev("main").unwrap(), head);
        let feature = repo.resolve_rev("feature").unwrap();
        assert_eq!(repo.resolve_rev("HEAD~1").unwrap(), feature);
        assert_eq!(repo.resolve_rev("HEAD^").unwrap(), feature);
        assert_ne!(repo.resolve_rev("HEAD~2").unwrap(), feature);
    }

    #[test]
    fn message_search_finds_the_newest_matching_commit() {
        let fixture = history();
        let repo = GitRepo::open(fixture.path(), &test_ctx()).unwrap();
        assert_eq!(repo.resolve_rev(":/two").unwrap(), repo.resolve_rev("feature").unwrap());
    }

    #[test]
    fn unknown_name_lists_similar_references() {
        let fixture = history();
        let repo = GitRepo::open(fixture.path(), &test_ctx()).unwrap();
        let error = repo.resolve_rev("featrue").unwrap_err();
        assert_eq!(error.code, ErrorCode::GitRevision);
        assert_eq!(error.problem.as_deref(), Some("not_found"));
        assert_eq!(
            error.candidates[0].program,
            format!(
                "git.log {} revision: \"feature\" count: 1",
                crate::repo::error::quote_path(fixture.path())
            )
        );
    }

    #[test]
    fn malformed_and_empty_revisions_are_invalid_syntax() {
        let fixture = history();
        let repo = GitRepo::open(fixture.path(), &test_ctx()).unwrap();
        let empty = repo.resolve_rev("  ").unwrap_err();
        assert_eq!(empty.problem.as_deref(), Some("invalid_syntax"));
        let malformed = repo.resolve_rev("HEAD~~^{nothing}").unwrap_err();
        assert_eq!(malformed.code, ErrorCode::GitRevision);
    }

    #[test]
    fn tree_revision_is_not_a_commit() {
        let fixture = history();
        let repo = GitRepo::open(fixture.path(), &test_ctx()).unwrap();
        assert!(repo.resolve_rev("HEAD^{tree}").is_ok());
        let error = repo.resolve_commit("HEAD^{tree}").unwrap_err();
        assert_eq!(error.problem.as_deref(), Some("not_a_commit"));
    }

    #[test]
    fn edit_distance_counts_insertions_deletions_and_substitutions() {
        assert_eq!(edit_distance("main", "main"), 0);
        assert_eq!(edit_distance("mian", "main"), 2);
        assert_eq!(edit_distance("mai", "main"), 1);
        assert_eq!(edit_distance("", "abc"), 3);
    }

    #[test]
    fn nearest_names_are_sorted_and_limited() {
        let names = vec!["main".to_owned(), "maint".to_owned(), "develop".to_owned()];
        assert_eq!(nearest_names("mian", &names), vec!["main".to_owned()]);
        assert_eq!(nearest_names("xyz", &names), Vec::<String>::new());
    }
}
