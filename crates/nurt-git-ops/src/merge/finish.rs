// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Conclusion of a stopped merge by a commit, shared by `git.merge` with
//! `action: continue` and by `git.commit`.

use std::path::Path;

use gix_hash::ObjectId;
use gix_index::entry::Stage;
use nurt_contracts::error::{Error, ErrorCode, Result};

use super::message::{Cleanup, clean_message, reflog_commit_merge};
use crate::state::{self, StateChange, StateFile, quote};

/// What a commit needs to conclude a stopped merge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeCompletion {
    /// Parents after `HEAD`: the commits of `MERGE_HEAD`.
    pub parents: Vec<ObjectId>,
    /// Message prepared in `MERGE_MSG`, unchanged.
    pub prepared_message: Option<String>,
    /// `MERGE_MODE` requires a commit even when a fast-forward is possible.
    pub no_ff: bool,
    /// Removal of the merge state files, to be applied in the transaction
    /// of the commit.
    pub state_changes: Vec<StateChange>,
}

impl MergeCompletion {
    /// Message of the concluding commit: `explicit` when given, otherwise the
    /// prepared message, cleaned up with `cleanup` (`git merge --continue`
    /// strips comments, `git commit --no-edit` keeps them). An explicit
    /// message only loses surplus white space, as a message given with `-m`
    /// does, so a line such as `#42 fix` stays.
    #[must_use]
    pub fn message(&self, explicit: Option<&str>, cleanup: Cleanup, comment: char) -> String {
        let (text, cleanup) = match explicit {
            Some(text) => (text, Cleanup::Whitespace),
            None => (
                self.prepared_message.as_deref().unwrap_or_default(),
                cleanup,
            ),
        };
        clean_message(text, cleanup, comment)
    }

    /// Reflog message of the concluding commit (`commit (merge): <subject>`).
    #[must_use]
    pub fn reflog_message(message: &str) -> String {
        reflog_commit_merge(message)
    }
}

/// Reads the stopped merge of the repository with Git directory `git_dir`
/// and checks that `index` has no unresolved paths; `None` when no merge is
/// in progress.
///
/// `repo` is the repository as given by the user, used in the calls of
/// `fix`.
///
/// # Errors
///
/// Fails with `E_GIT_CONFLICT` (problem `unmerged_paths`, effect not
/// applied) when the index still has conflict stages, and with `E_IO` or
/// `E_GIT_REVISION` when the state files cannot be read.
pub fn prepare_completion(
    git_dir: &Path,
    index: &gix_index::State,
    repo: &str,
) -> Result<Option<MergeCompletion>> {
    let Some(merge) = state::read_merge_state(git_dir)? else {
        return Ok(None);
    };
    let unmerged = unmerged_paths(index);
    if !unmerged.is_empty() {
        return Err(unmerged_error(&unmerged, repo));
    }
    Ok(Some(MergeCompletion {
        parents: merge.heads,
        prepared_message: merge
            .message
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned()),
        no_ff: merge.no_ff,
        state_changes: state::removals(git_dir, StateFile::MERGE_STATE)?,
    }))
}

/// Paths of `index` with conflict stages, in index order, without
/// repetition.
#[must_use]
pub fn unmerged_paths(index: &gix_index::State) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for entry in index.entries() {
        if entry.stage() == Stage::Unconflicted {
            continue;
        }
        let path = entry.path(index).to_string();
        if out.last() != Some(&path) {
            out.push(path);
        }
    }
    out
}

/// `E_GIT_CONFLICT` for a commit attempted while `paths` are unresolved.
#[must_use]
pub fn unmerged_error(paths: &[String], repo: &str) -> Error {
    let mut error = Error::new(
        ErrorCode::GitConflict,
        "committing is not possible because of unmerged files; resolve them and stage the result",
    )
    .with_problem("unmerged_paths")
    .with_effect_applied(false);
    for path in paths {
        error = error.with_sample(path.clone());
    }
    error
        .with_fix(format!(
            "git.stage {} action: add paths: [{}]",
            quote(repo),
            quoted_list(paths)
        ))
        .with_next_step(format!("git.merge {} action: abort", quote(repo)))
}

fn quoted_list(paths: &[String]) -> String {
    paths
        .iter()
        .map(|path| quote(path))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    const ONE: &str = "1111111111111111111111111111111111111111";

    fn empty_index() -> gix_index::State {
        gix_index::State::new(gix_hash::Kind::Sha1)
    }

    #[test]
    fn returns_none_without_merge_head() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            prepare_completion(dir.path(), &empty_index(), "/r").unwrap(),
            None
        );
    }

    #[test]
    fn prepares_parents_message_and_state_removals() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("MERGE_HEAD"), format!("{ONE}\n")).unwrap();
        fs::write(
            dir.path().join("MERGE_MSG"),
            "Merge branch 'topic'\n\n# Conflicts:\n#\tf\n",
        )
        .unwrap();
        fs::write(dir.path().join("MERGE_MODE"), "").unwrap();
        fs::write(dir.path().join("AUTO_MERGE"), format!("{ONE}\n")).unwrap();
        let completion = prepare_completion(dir.path(), &empty_index(), "/r")
            .unwrap()
            .unwrap();
        assert_eq!(completion.parents.len(), 1);
        assert!(!completion.no_ff);
        assert_eq!(completion.state_changes.len(), 4);
        assert_eq!(
            completion.message(None, Cleanup::Strip, '#'),
            "Merge branch 'topic'\n"
        );
        assert_eq!(
            completion.message(None, Cleanup::Whitespace, '#'),
            "Merge branch 'topic'\n\n# Conflicts:\n#\tf\n"
        );
        assert_eq!(
            completion.message(Some("own\n"), Cleanup::Strip, '#'),
            "own\n"
        );
        assert_eq!(
            completion.message(Some("#42 resolve\n\nbody\n"), Cleanup::Strip, '#'),
            "#42 resolve\n\nbody\n"
        );
        assert_eq!(
            MergeCompletion::reflog_message("Merge branch 'topic'\n"),
            "commit (merge): Merge branch 'topic'"
        );
    }

    #[test]
    fn refuses_completion_with_unmerged_paths() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("MERGE_HEAD"), format!("{ONE}\n")).unwrap();
        let mut index = empty_index();
        let id = ObjectId::empty_blob(gix_hash::Kind::Sha1);
        for stage in [
            gix_index::entry::Stage::Ours,
            gix_index::entry::Stage::Theirs,
        ] {
            index.dangerously_push_entry(
                Default::default(),
                id,
                stage.into(),
                gix_index::entry::Mode::FILE,
                "f.txt".into(),
            );
        }
        index.sort_entries();
        let error = prepare_completion(dir.path(), &index, "/r").unwrap_err();
        assert_eq!(error.code(), ErrorCode::GitConflict);
        assert_eq!(error.data().sample, vec!["f.txt".to_owned()]);
        assert_eq!(
            error.data().fix,
            vec!["git.stage \"/r\" action: add paths: [\"f.txt\"]".to_owned()]
        );
    }
}
