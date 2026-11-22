// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Errors of `git.pull` with ready replacements of the program.

use nurt_contracts::error::{Error, ErrorCode, Stage};
use nurt_contracts::record::GitRefUpdate;

use super::params::PullMode;

const HELP: &str = "git.pull";

/// Largest number of paths listed in the sample of an error.
const SAMPLE_LIMIT: usize = 20;

/// Program text of one call to `git.pull`, used to build replacements.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PullCall {
    repo: String,
    remote: Option<String>,
    branch: Option<String>,
}

impl PullCall {
    pub(crate) fn new(repo: &str, remote: Option<&str>, branch: Option<&str>) -> Self {
        Self {
            repo: repo.to_owned(),
            remote: remote.map(str::to_owned),
            branch: branch.map(str::to_owned),
        }
    }

    pub(crate) fn repo(&self) -> &str {
        &self.repo
    }

    /// The call with the given branch of the given remote.
    pub(crate) fn with_branch(&self, remote: &str, branch: &str) -> Self {
        Self {
            repo: self.repo.clone(),
            remote: Some(remote.to_owned()),
            branch: Some(branch.to_owned()),
        }
    }

    /// Program text of the call with the given mode.
    pub(crate) fn text(&self, mode: Option<PullMode>) -> String {
        let mut text = format!("git.pull {}", quote(&self.repo));
        if let Some(remote) = &self.remote {
            text.push_str(&format!(" remote: {}", quote(remote)));
        }
        if let Some(branch) = &self.branch {
            text.push_str(&format!(" branch: {}", quote(branch)));
        }
        if let Some(mode) = mode {
            text.push_str(&format!(" mode: {mode}"));
        }
        text
    }
}

/// Text literal of the language.
pub(crate) fn quote(text: &str) -> String {
    let mut quoted = String::with_capacity(text.len() + 2);
    quoted.push('"');
    for character in text.chars() {
        match character {
            '"' => quoted.push_str("\\\""),
            '\\' => quoted.push_str("\\\\"),
            '\n' => quoted.push_str("\\n"),
            '\t' => quoted.push_str("\\t"),
            other => quoted.push(other),
        }
    }
    quoted.push('"');
    quoted
}

fn plan_error(code: ErrorCode, problem: &str, message: String) -> Error {
    Error::new(code, message)
        .with_stage(Stage::Plan)
        .with_problem(problem)
        .with_help(HELP)
}

fn apply_error(code: ErrorCode, problem: &str, message: String) -> Error {
    Error::new(code, message)
        .with_stage(Stage::Apply)
        .with_problem(problem)
        .with_help(HELP)
}

fn with_samples(mut error: Error, paths: &[String]) -> Error {
    for path in paths.iter().take(SAMPLE_LIMIT) {
        error = error.with_sample(path.clone());
    }
    if paths.len() > SAMPLE_LIMIT {
        error = error.with_detail("total", serde_json::json!(paths.len()));
    }
    error
}

/// A configuration value that `git` rejects as well.
pub(crate) fn invalid_config(call: &PullCall, key: &str, value: &str, fixes: &[PullMode]) -> Error {
    let mut error = plan_error(
        ErrorCode::GitUnsupported,
        "invalid_config_value",
        format!("invalid value `{value}` of `{key}` in the repository configuration; correct it or give `mode`"),
    )
    .with_found(value)
    .with_detail("key", serde_json::json!(key));
    for mode in fixes {
        error = error.with_fix(call.text(Some(*mode)));
    }
    error
}

/// Unmerged index entries left by an earlier merge.
pub(crate) fn unmerged_files(call: &PullCall, paths: &[String]) -> Error {
    let repo = quote(call.repo());
    let error = plan_error(
        ErrorCode::GitInProgress,
        "unmerged_files",
        "pulling is not possible with unmerged files; resolve them and commit, or abort the merge"
            .to_owned(),
    )
    .with_fix(format!("git.merge {repo} action: abort"))
    .with_next_step(format!("git.stage {repo} action: add"))
    .with_next_step(format!("git.commit {repo}"));
    with_samples(error, paths)
}

/// `MERGE_HEAD` exists without unmerged entries.
pub(crate) fn merge_in_progress(call: &PullCall) -> Error {
    let repo = quote(call.repo());
    plan_error(
        ErrorCode::GitInProgress,
        "merge_in_progress",
        "a merge is not concluded; commit it or abort it before pulling".to_owned(),
    )
    .with_fix(format!("git.commit {repo}"))
    .with_fix(format!("git.merge {repo} action: abort"))
}

/// A rebase is stopped in the repository.
pub(crate) fn rebase_in_progress(call: &PullCall) -> Error {
    let repo = quote(call.repo());
    plan_error(
        ErrorCode::GitInProgress,
        "rebase_in_progress",
        "a rebase is in progress; continue or abort it before pulling".to_owned(),
    )
    .with_fix(format!("git.rebase {repo} action: continue"))
    .with_fix(format!("git.rebase {repo} action: abort"))
}

/// Uncommitted changes that block a pull with rebase.
pub(crate) fn uncommitted_changes(call: &PullCall, staged: bool, paths: &[String]) -> Error {
    let (problem, what) = if staged {
        ("staged_changes", "the index contains uncommitted changes")
    } else {
        ("unstaged_changes", "the working tree has unstaged changes")
    };
    let repo = quote(call.repo());
    let error = plan_error(
        ErrorCode::GitDirty,
        problem,
        format!("cannot pull with rebase: {what}; commit or stash them"),
    )
    .with_fix(format!("git.stash {repo} action: push"))
    .with_fix(call.text(Some(PullMode::Merge)))
    .with_next_step(call.text(Some(PullMode::Rebase)))
    .with_detail(
        "kind",
        serde_json::json!(if staged { "staged" } else { "modified" }),
    );
    with_samples(error, paths)
}

/// Diverged histories without a configured integration mode.
pub(crate) fn diverged(call: &PullCall) -> Error {
    apply_error(
        ErrorCode::GitNonFastForward,
        "diverged",
        "the current branch and the fetched branch diverged; give `mode: merge` or `mode: rebase`"
            .to_owned(),
    )
    .with_fix(call.text(Some(PullMode::Merge)))
    .with_fix(call.text(Some(PullMode::Rebase)))
}

/// Diverged histories where only a fast-forward is allowed.
pub(crate) fn not_fast_forward(call: &PullCall) -> Error {
    apply_error(
        ErrorCode::GitNonFastForward,
        "not_fast_forward",
        "not possible to fast-forward; give `mode: merge` or `mode: rebase`".to_owned(),
    )
    .with_fix(call.text(Some(PullMode::Merge)))
    .with_fix(call.text(Some(PullMode::Rebase)))
}

/// The current branch has no upstream branch.
pub(crate) fn no_upstream(call: &PullCall, branch: &str, remote: &str) -> Error {
    apply_error(
        ErrorCode::GitRef,
        "no_upstream",
        format!("branch `{branch}` has no upstream branch; give `branch` to pull"),
    )
    .with_fix(call.with_branch(remote, branch).text(None))
    .with_next_step(format!("git.branch {} remote: true", quote(call.repo())))
}

/// `HEAD` is detached and no branch was given.
pub(crate) fn detached_head(call: &PullCall) -> Error {
    let repo = quote(call.repo());
    apply_error(
        ErrorCode::GitRef,
        "detached_head",
        "not on a branch; switch to a branch or give `branch` to pull".to_owned(),
    )
    .with_next_step(format!("git.branch {repo}"))
    .with_next_step(format!("git.branch {repo} remote: true"))
}

/// A remote other than the upstream remote was given without a branch.
pub(crate) fn no_branch_for_remote(call: &PullCall, remote: &str, branch: &str) -> Error {
    apply_error(
        ErrorCode::GitRef,
        "branch_required",
        format!(
            "remote `{remote}` is not the upstream remote of the current branch; give `branch`"
        ),
    )
    .with_fix(call.with_branch(remote, branch).text(None))
    .with_next_step(format!("git.branch {} remote: true", quote(call.repo())))
}

/// The given branch was not fetched.
pub(crate) fn no_merge_candidates(call: &PullCall, refspecs: &[String]) -> Error {
    let error = apply_error(
        ErrorCode::GitRef,
        "no_merge_candidates",
        "the remote has no branch to integrate; check the branch name".to_owned(),
    )
    .with_next_step(format!("git.branch {} remote: true", quote(call.repo())));
    with_samples(error, refspecs)
}

/// The configured upstream branch was not fetched.
pub(crate) fn upstream_gone(call: &PullCall, merge_ref: &str) -> Error {
    apply_error(
        ErrorCode::GitRef,
        "upstream_gone",
        format!(
            "the upstream branch `{merge_ref}` does not exist on the remote; give another `branch`"
        ),
    )
    .with_found(merge_ref)
    .with_next_step(format!("git.branch {} remote: true", quote(call.repo())))
}

/// Several fetched heads where only one can be integrated.
pub(crate) fn multiple_heads(call: &PullCall, what: &str) -> Error {
    apply_error(
        ErrorCode::GitRef,
        "multiple_heads",
        format!("cannot {what} several branches; give one `branch`"),
    )
    .with_fix(call.text(Some(PullMode::Merge)))
}

/// Several fetched heads to merge at once (an octopus merge).
pub(crate) fn octopus_merge(call: &PullCall) -> Error {
    apply_error(
        ErrorCode::GitUnsupported,
        "octopus_merge",
        "merging several fetched branches at once is not supported; give one `branch`".to_owned(),
    )
    .with_next_step(format!("git.branch {} remote: true", quote(call.repo())))
}

/// A rebase is required and no rebase engine is available.
pub(crate) fn rebase_unavailable(call: &PullCall) -> Error {
    apply_error(
        ErrorCode::GitUnsupported,
        "rebase_unavailable",
        "a pull with rebase is not available in this installation; give `mode: merge`".to_owned(),
    )
    .with_fix(call.text(Some(PullMode::Merge)))
}

/// Marks an error that happened after the transfer: the fetched references
/// and `FETCH_HEAD` stay updated.
pub(crate) fn after_transfer(error: Error, ref_updates: &[GitRefUpdate]) -> Error {
    let stage = error.stage().unwrap_or(Stage::Apply);
    let updates: Vec<serde_json::Value> = ref_updates
        .iter()
        .map(|update| {
            serde_json::json!({
                "ref": update.reference,
                "status": update.status.as_str(),
                "old": update.old,
                "new": update.new,
            })
        })
        .collect();
    error
        .with_stage(stage)
        .with_effect_applied(true)
        .with_detail("ref_updates", serde_json::Value::Array(updates))
}

#[cfg(test)]
mod tests {
    use nurt_contracts::record::GitRefStatus;

    use super::*;

    fn call() -> PullCall {
        PullCall::new("/work/repo", None, None)
    }

    #[test]
    fn quotes_text_literals() {
        assert_eq!(quote("plain"), "\"plain\"");
        assert_eq!(quote("a\"b\\c\nd\te"), "\"a\\\"b\\\\c\\nd\\te\"");
    }

    #[test]
    fn writes_calls_with_given_parameters_only() {
        assert_eq!(call().text(None), "git.pull \"/work/repo\"");
        let full = PullCall::new("/work/repo", Some("up"), Some("dev"));
        assert_eq!(
            full.text(Some(PullMode::Rebase)),
            "git.pull \"/work/repo\" remote: \"up\" branch: \"dev\" mode: rebase"
        );
    }

    #[test]
    fn offers_merge_and_rebase_for_diverged_histories() {
        let error = diverged(&call());
        assert_eq!(error.code, ErrorCode::GitNonFastForward);
        assert_eq!(error.problem.as_deref(), Some("diverged"));
        assert_eq!(
            error.fix,
            [
                "git.pull \"/work/repo\" mode: merge",
                "git.pull \"/work/repo\" mode: rebase"
            ]
        );
        let refused = not_fast_forward(&call());
        assert_eq!(refused.problem.as_deref(), Some("not_fast_forward"));
        assert_eq!(refused.fix.len(), 2);
    }

    #[test]
    fn suggests_the_branch_of_the_same_name_without_upstream() {
        let error = no_upstream(&call(), "feature", "origin");
        assert_eq!(error.code, ErrorCode::GitRef);
        assert_eq!(error.problem.as_deref(), Some("no_upstream"));
        assert_eq!(
            error.fix,
            ["git.pull \"/work/repo\" remote: \"origin\" branch: \"feature\""]
        );
        let other = no_branch_for_remote(&call(), "other", "main");
        assert_eq!(other.problem.as_deref(), Some("branch_required"));
        assert_eq!(
            other.fix,
            ["git.pull \"/work/repo\" remote: \"other\" branch: \"main\""]
        );
    }

    #[test]
    fn limits_the_path_sample() {
        let paths: Vec<String> = (0..25).map(|index| format!("f{index}.txt")).collect();
        let error = unmerged_files(&call(), &paths);
        assert_eq!(error.sample.len(), SAMPLE_LIMIT);
        assert_eq!(error.details["total"], serde_json::json!(25));
        assert_eq!(error.code, ErrorCode::GitInProgress);
        assert!(!error.next_steps.is_empty());
    }

    #[test]
    fn describes_blocking_changes_before_a_rebase() {
        let staged = uncommitted_changes(&call(), true, &["a.txt".to_owned()]);
        assert_eq!(staged.code, ErrorCode::GitDirty);
        assert_eq!(staged.problem.as_deref(), Some("staged_changes"));
        assert_eq!(staged.details["kind"], serde_json::json!("staged"));
        assert_eq!(staged.fix[0], "git.stash \"/work/repo\" action: push");
        let unstaged = uncommitted_changes(&call(), false, &[]);
        assert_eq!(unstaged.problem.as_deref(), Some("unstaged_changes"));
        assert_eq!(unstaged.stage(), Some(Stage::Plan));
    }

    #[test]
    fn offers_ways_out_of_unfinished_operations() {
        assert_eq!(merge_in_progress(&call()).fix.len(), 2);
        assert_eq!(
            rebase_in_progress(&call()).fix[1],
            "git.rebase \"/work/repo\" action: abort"
        );
        let invalid = invalid_config(&call(), "pull.ff", "maybe", &[PullMode::FastForwardOnly]);
        assert_eq!(invalid.found.as_deref(), Some("maybe"));
        assert_eq!(
            invalid.fix,
            ["git.pull \"/work/repo\" mode: fast_forward_only"]
        );
    }

    #[test]
    fn reports_missing_candidates() {
        let error = no_merge_candidates(&call(), &["dev".to_owned()]);
        assert_eq!(error.sample, ["dev"]);
        assert_eq!(
            upstream_gone(&call(), "refs/heads/rel").found.as_deref(),
            Some("refs/heads/rel")
        );
        assert_eq!(
            detached_head(&call()).problem.as_deref(),
            Some("detached_head")
        );
        assert_eq!(
            multiple_heads(&call(), "rebase onto").problem.as_deref(),
            Some("multiple_heads")
        );
    }

    #[test]
    fn reports_integrations_that_are_not_available() {
        let octopus = octopus_merge(&call());
        assert_eq!(octopus.code, ErrorCode::GitUnsupported);
        assert_eq!(octopus.problem.as_deref(), Some("octopus_merge"));
        let rebase = rebase_unavailable(&call());
        assert_eq!(rebase.problem.as_deref(), Some("rebase_unavailable"));
        assert_eq!(rebase.fix, ["git.pull \"/work/repo\" mode: merge"]);
    }

    #[test]
    fn keeps_the_transfer_in_errors_after_it() {
        let update = GitRefUpdate {
            reference: "refs/remotes/origin/main".to_owned(),
            status: GitRefStatus::Updated,
            remote: "origin".to_owned(),
            old: Some("1".repeat(40)),
            new: Some("2".repeat(40)),
            ..GitRefUpdate::default()
        };
        let error = after_transfer(diverged(&call()), &[update]);
        assert!(error.effect_applied);
        assert_eq!(error.stage(), Some(Stage::Apply));
        assert_eq!(
            error.details["ref_updates"][0]["status"],
            serde_json::json!("updated")
        );
    }
}
