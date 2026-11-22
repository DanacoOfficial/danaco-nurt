// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Revisions to merge and the names merge messages give them.

use bstr::ByteSlice;
use gix_hash::ObjectId;
use nurt_contracts::error::{Error, ErrorCode, Result};

use super::message::{DEFAULT_SUPPRESSED_DESTINATIONS, MergeSource};
use super::trees::{io_error, revision_error};
use crate::state::quote;

/// Prefix of local branches.
pub(crate) const HEADS: &str = "refs/heads/";
/// Prefix of remote-tracking branches.
const REMOTES: &str = "refs/remotes/";
/// Prefix of tags.
const TAGS: &str = "refs/tags/";

/// The commit merged into the current branch and the names Git uses for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeTarget {
    /// Commit to merge.
    pub commit: ObjectId,
    /// Name written after the `>>>>>>>` conflict markers and in the reflog
    /// message `merge <label>`: the revision as given.
    pub label: String,
    /// Description used in the message of the merge commit.
    pub source: MergeSource,
}

impl MergeTarget {
    /// Resolves `revision` to a commit and describes it as
    /// `git fmt-merge-msg` does: a tag, a local branch, a remote-tracking
    /// branch or a commit.
    ///
    /// # Errors
    ///
    /// Fails with `E_GIT_REVISION` when `revision` names no commit and with
    /// `E_IO` when the references cannot be read.
    pub fn resolve(repo: &gix::Repository, revision: &str) -> Result<Self> {
        let commit = repo
            .rev_parse_single(revision.as_bytes().as_bstr())
            .ok()
            .and_then(|id| id.object().ok())
            .and_then(|object| object.peel_to_commit().ok())
            .map(|commit| commit.id)
            .ok_or_else(|| {
                Error::new(
                    ErrorCode::GitRevision,
                    format!("{revision} does not name a commit; give a branch, tag or commit"),
                )
                .with_problem("unknown")
                .with_found(revision.to_owned())
            })?;
        Ok(Self {
            commit,
            label: revision.to_owned(),
            source: describe(repo, revision)?,
        })
    }
}

/// Description of `revision` for the merge message, in the order in which
/// Git expands a short reference name.
fn describe(repo: &gix::Repository, revision: &str) -> Result<MergeSource> {
    let candidates = [
        (TAGS, Kind::Tag),
        (HEADS, Kind::Branch),
        (REMOTES, Kind::Remote),
    ];
    for (prefix, kind) in candidates {
        let short = revision;
        let full = if revision.starts_with(prefix) {
            revision.to_owned()
        } else {
            format!("{prefix}{revision}")
        };
        if !has_ref(repo, &full)? {
            continue;
        }
        return Ok(match kind {
            Kind::Tag => MergeSource::Tag {
                name: short.to_owned(),
                message: tag_message(repo, &full)?,
            },
            Kind::Branch => MergeSource::Branch(short.to_owned()),
            Kind::Remote => MergeSource::RemoteBranch(short.to_owned()),
        });
    }
    if let Some((branch, early)) = ancestor_of_branch(revision)
        && has_ref(repo, &format!("{HEADS}{branch}"))?
    {
        return Ok(if early {
            MergeSource::EarlyPart(branch.to_owned())
        } else {
            MergeSource::Branch(branch.to_owned())
        });
    }
    Ok(MergeSource::Commit(revision.to_owned()))
}

/// Branch name of `<name>^…` (an early part) or `<name>~<number>`, as
/// `git merge` recognises them.
fn ancestor_of_branch(revision: &str) -> Option<(&str, bool)> {
    let carets = revision.trim_end_matches('^');
    if carets.len() < revision.len() {
        return (!carets.is_empty()).then_some((carets, true));
    }
    let (branch, count) = revision.rsplit_once('~')?;
    let digits = count.bytes().all(|byte| byte.is_ascii_digit());
    (digits && !branch.is_empty()).then_some((branch, false))
}

#[derive(Clone, Copy)]
enum Kind {
    Tag,
    Branch,
    Remote,
}

fn has_ref(repo: &gix::Repository, name: &str) -> Result<bool> {
    if gix::refs::FullName::try_from(name).is_err() {
        return Ok(false);
    }
    repo.try_find_reference(name)
        .map(|found| found.is_some())
        .map_err(io_error("read references"))
}

/// Message of the annotated tag `full`; `None` for a lightweight tag.
fn tag_message(repo: &gix::Repository, full: &str) -> Result<Option<String>> {
    let Some(reference) = repo
        .try_find_reference(full)
        .map_err(io_error("read references"))?
    else {
        return Ok(None);
    };
    let Some(id) = reference.target().try_id().map(ToOwned::to_owned) else {
        return Ok(None);
    };
    let object = repo.find_object(id).map_err(revision_error("read a tag"))?;
    if object.kind != gix_object::Kind::Tag {
        return Ok(None);
    }
    let tag = object.into_tag();
    let decoded = tag.decode().map_err(revision_error("decode a tag"))?;
    Ok(Some(decoded.message.to_str_lossy().into_owned()))
}

/// Patterns of `merge.suppressDest`; an empty value clears the earlier
/// ones, and without any value the default branches are suppressed.
pub(crate) fn suppressed_destinations(repo: &gix::Repository) -> Vec<String> {
    let config = repo.config_snapshot();
    let values: Vec<String> = config
        .plumbing()
        .strings("merge.suppressDest")
        .unwrap_or_default()
        .into_iter()
        .map(|value| value.to_str_lossy().into_owned())
        .collect();
    if values.is_empty() {
        return DEFAULT_SUPPRESSED_DESTINATIONS
            .iter()
            .map(|name| (*name).to_owned())
            .collect();
    }
    let start = values
        .iter()
        .rposition(String::is_empty)
        .map_or(0, |at| at + 1);
    values[start..].to_vec()
}

/// Comment character of commit messages (`core.commentChar`); `auto` and
/// unset select `#`.
pub(crate) fn comment_char(repo: &gix::Repository) -> char {
    const DEFAULT: char = '#';
    repo.config_snapshot()
        .string("core.commentChar")
        .and_then(|value| value.to_str().ok().map(str::to_owned))
        .filter(|text| text != "auto")
        .and_then(|text| text.chars().next())
        .unwrap_or(DEFAULT)
}

/// Branch name without `refs/heads/`.
pub(crate) fn short_branch(full: &str) -> &str {
    full.strip_prefix(HEADS).unwrap_or(full)
}

/// `E_MISSING_PARAM` for a merge started without a revision.
pub(crate) fn missing_branch(repo: &str) -> Error {
    Error::new(
        ErrorCode::MissingParam,
        "a branch to merge is required when action is start; give branch: \"<name>\"",
    )
    .with_fix(format!("git.merge {} branch: \"<name>\"", quote(repo)))
}

#[cfg(test)]
mod tests {
    use super::super::fixture::Fixture;
    use super::*;

    fn write(fixture: &Fixture, name: &str, content: &str) {
        let path = fixture.repo.git_dir().join(name);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("directories");
        std::fs::write(path, content).expect("file");
    }

    #[test]
    fn describes_branches_tags_remotes_and_commits() {
        let fixture = Fixture::new();
        let tree = fixture.tree(&[("a", "a\n")]);
        let commit = fixture.commit(tree, &[], "c");
        write(&fixture, "refs/heads/topic", &format!("{commit}\n"));
        write(&fixture, "refs/remotes/origin/dev", &format!("{commit}\n"));
        write(&fixture, "refs/tags/v1", &format!("{commit}\n"));
        let cases = [
            ("topic", MergeSource::Branch("topic".into())),
            (
                "refs/heads/topic",
                MergeSource::Branch("refs/heads/topic".into()),
            ),
            ("origin/dev", MergeSource::RemoteBranch("origin/dev".into())),
            (
                "v1",
                MergeSource::Tag {
                    name: "v1".into(),
                    message: None,
                },
            ),
        ];
        for (revision, source) in cases {
            let target = MergeTarget::resolve(&fixture.repo, revision).expect("resolve");
            assert_eq!(target.commit, commit);
            assert_eq!(target.label, revision);
            assert_eq!(target.source, source);
        }
        let hex = commit.to_hex().to_string();
        let target = MergeTarget::resolve(&fixture.repo, &hex).expect("resolve");
        assert_eq!(target.source, MergeSource::Commit(hex));
        let child = fixture.commit(tree, &[commit], "child");
        write(&fixture, "refs/heads/next", &format!("{child}\n"));
        let tilde = MergeTarget::resolve(&fixture.repo, "next~1").expect("resolve");
        assert_eq!(tilde.commit, commit);
        assert_eq!(tilde.source, MergeSource::Branch("next".into()));
        let caret = MergeTarget::resolve(&fixture.repo, "next^").expect("resolve");
        assert_eq!(caret.source, MergeSource::EarlyPart("next".into()));
    }

    #[test]
    fn recognises_ancestors_of_branches_like_git() {
        assert_eq!(ancestor_of_branch("topic^^"), Some(("topic", true)));
        assert_eq!(ancestor_of_branch("topic~2"), Some(("topic", false)));
        assert_eq!(ancestor_of_branch("topic~x"), None);
        assert_eq!(ancestor_of_branch("^"), None);
        assert_eq!(ancestor_of_branch("topic"), None);
    }

    #[test]
    fn rejects_revision_without_commit() {
        let fixture = Fixture::new();
        let error = MergeTarget::resolve(&fixture.repo, "nothing").expect_err("unknown");
        assert_eq!(error.code(), ErrorCode::GitRevision);
        assert_eq!(error.data().problem.as_deref(), Some("unknown"));
    }

    #[test]
    fn reads_suppressed_destinations_and_comment_character() {
        let fixture = Fixture::new();
        assert_eq!(suppressed_destinations(&fixture.repo), ["main", "master"]);
        assert_eq!(comment_char(&fixture.repo), '#');
        assert_eq!(short_branch("refs/heads/a/b"), "a/b");
        assert_eq!(short_branch("HEAD"), "HEAD");
        assert_eq!(missing_branch("/r").code(), ErrorCode::MissingParam);
    }
}
