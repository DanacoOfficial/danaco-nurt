// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Local integration of the fetched heads in the transaction of the
//! transfer.
//!
//! A merge is planned completely with [`plan_merge`]; the updates of the
//! remote-tracking references and `FETCH_HEAD` are added to that plan, and
//! [`apply_merge`] writes everything with one preservation of all targets.
//! When the integration is refused before any write, the transfer is
//! written alone, as `git pull` keeps the fetched state when the merge
//! does not start.

use bstr::BString;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::journal::{Expect, MutationTx};
use nurt_contracts::stage::ExecCtx;
use nurt_git::apply::{ApplyContext, StateFilePlan, apply_plan};
use nurt_git::repo::GitRepo;

use super::settings::FfPolicy;
use super::steps::{FetchOutcome, LocalOutcome};
use crate::merge::{FastForward, MergeRequest, MergeResult, MergeTarget, apply_merge, plan_merge};
use crate::reset::file_changes;

/// Reflog message `git` writes when a pull creates the current branch.
pub(crate) const INITIAL_PULL: &str = "initial pull";

/// Nanoseconds per second of the clock of the execution context.
const NANOSECONDS_PER_SECOND: i128 = 1_000_000_000;

/// File of the repository directory that keeps the commit before an update.
const ORIG_HEAD: &str = "ORIG_HEAD";

/// Prefix of the full names of local branches.
const HEADS: &str = "refs/heads/";

/// Why the integration did not complete.
#[derive(Debug)]
pub(crate) enum Failure {
    /// Refused before any target was declared; the transfer is written
    /// alone.
    Refused(Error),
    /// Stopped or failed after the transaction was used; reported as it is.
    Failed(Error),
}

/// A merge of the single fetched head into the current branch.
pub(crate) struct MergeInput<'a> {
    /// Repository as given by the program.
    pub(crate) label: &'a str,
    /// Result of the transfer; its first head is merged.
    pub(crate) fetched: &'a FetchOutcome,
    /// Message of a merge commit.
    pub(crate) message: String,
    /// Fast-forward policy.
    pub(crate) fast_forward: FfPolicy,
    /// Prefix of the reflog messages.
    pub(crate) reflog_action: &'a str,
    /// The current branch has no commit: the reflog says `initial pull`.
    pub(crate) initial: bool,
}

fn fast_forward(policy: FfPolicy) -> FastForward {
    match policy {
        FfPolicy::Allow => FastForward::Auto,
        FfPolicy::Never => FastForward::Never,
        FfPolicy::Only => FastForward::Only,
    }
}

/// Merges the first fetched head, writing the transfer in the same
/// transaction.
///
/// # Errors
///
/// [`Failure::Refused`] with the planning errors of [`plan_merge`]
/// (`E_GIT_DIRTY`, `E_GIT_NON_FAST_FORWARD`, `E_GIT_IN_PROGRESS`);
/// [`Failure::Failed`] with `E_GIT_CONFLICT` and `effect_applied` when the
/// merge stopped with conflicts, or with the errors of [`apply_merge`].
pub(crate) fn merge(
    repo: &GitRepo,
    input: &MergeInput<'_>,
    tx: &mut dyn MutationTx,
    ctx: &ExecCtx,
) -> std::result::Result<LocalOutcome, Failure> {
    let request = merge_request(repo, input, ctx).map_err(Failure::Refused)?;
    let apply_ctx = ApplyContext::open(repo.gix()).map_err(Failure::Failed)?;
    let mut plan = plan_merge(&apply_ctx, &request).map_err(Failure::Refused)?;
    if input.initial {
        for reference in &mut plan.apply.refs {
            if let Some(line) = reference.reflog.as_mut() {
                INITIAL_PULL.clone_into(&mut line.message);
            }
        }
    }
    plan.apply.refs.extend(input.fetched.refs.iter().cloned());
    plan.apply
        .state_files
        .extend(input.fetched.state_files.iter().cloned());
    let result = apply_merge(tx, &apply_ctx, plan).map_err(Failure::Failed)?;
    if let Some(error) = result.conflict_error(input.label) {
        return Err(Failure::Failed(error));
    }
    Ok(local_outcome(repo, &result))
}

fn merge_request(repo: &GitRepo, input: &MergeInput<'_>, ctx: &ExecCtx) -> Result<MergeRequest> {
    let head = input.fetched.for_merge.first().ok_or_else(|| {
        Error::new(
            nurt_contracts::error::ErrorCode::Internal,
            "the transfer marked no head for merging",
        )
    })?;
    let branch = head
        .remote_ref
        .strip_prefix(HEADS)
        .unwrap_or(&head.remote_ref)
        .to_owned();
    let target = MergeTarget {
        commit: head.id,
        label: head.id.to_string(),
        source: crate::merge::MergeSource::Fetched {
            branch,
            url: input.fetched.url.clone(),
        },
    };
    let (author, committer) = identity(repo, ctx);
    let mut request = MergeRequest::start(input.label, target, committer);
    request.author = author;
    request.fast_forward = fast_forward(input.fast_forward);
    request.message = Some(input.message.clone());
    request.reflog_action = Some(input.reflog_action.to_owned());
    Ok(request)
}

fn text(path: &BString) -> String {
    String::from_utf8_lossy(path).into_owned()
}

fn local_outcome(repo: &GitRepo, result: &MergeResult) -> LocalOutcome {
    let root = repo.root();
    let written: Vec<String> = result.report.written.iter().map(text).collect();
    let removed: Vec<String> = result.report.removed.iter().map(text).collect();
    LocalOutcome {
        effect: result.effect(root),
        changes: file_changes(root, &written, &removed),
        notable: result.notes.clone(),
    }
}

/// Writes the remote-tracking references and `FETCH_HEAD` alone.
///
/// A refusal because of local changes also records the current commit in
/// `ORIG_HEAD`, as `git` does before it checks out the merge result.
///
/// # Errors
///
/// The errors of [`apply_plan`]: `E_GIT_LOCKED`, `E_STALE` and the errors
/// of the transaction.
pub(crate) fn write_transfer(
    repo: &GitRepo,
    fetched: &FetchOutcome,
    refusal: &Error,
    tx: &mut dyn MutationTx,
) -> Result<()> {
    let apply_ctx = ApplyContext::open(repo.gix())?;
    let mut plan = fetched.local_plan();
    if refusal.code() == ErrorCode::GitDirty
        && let Ok(head) = repo.gix().head_id()
    {
        plan.state_files.push(StateFilePlan {
            path: ORIG_HEAD.to_owned(),
            content: Some(format!("{head}\n").into_bytes()),
            expect: Expect::Unchecked,
        });
    }
    apply_plan(tx, &apply_ctx, plan).map(|_| ())
}

/// Author and committer of a merge commit: `GIT_AUTHOR_*` and
/// `GIT_COMMITTER_*` of the environment of the context, then `user.name`
/// and `user.email`, then the account and host names; the time is the
/// clock of the context with the local offset of the host.
pub(super) fn identity(
    repo: &GitRepo,
    ctx: &ExecCtx,
) -> (gix::actor::Signature, gix::actor::Signature) {
    let host = ctx.host();
    let info = host.info();
    let var = |name: &str| -> Option<String> {
        host.env_var(name)
            .and_then(|value| value.into_string().ok())
            .filter(|value| !value.is_empty())
    };
    let default_name = repo
        .config_string("user.name")
        .unwrap_or_else(|| info.user.clone());
    let default_email = repo
        .config_string("user.email")
        .unwrap_or_else(|| format!("{}@{}", info.user, info.hostname));
    let seconds = i64::try_from(
        ctx.clock()
            .now()
            .unix_nanos()
            .div_euclid(NANOSECONDS_PER_SECOND),
    )
    .unwrap_or_default();
    let offset = jiff::tz::TimeZone::get(&info.timezone)
        .ok()
        .zip(jiff::Timestamp::from_second(seconds).ok())
        .map_or(0, |(zone, instant)| zone.to_offset(instant).seconds());
    let now = gix::date::Time::new(seconds, offset);
    let pick = |role: &str| gix::actor::Signature {
        name: var(&format!("GIT_{role}_NAME"))
            .unwrap_or_else(|| default_name.clone())
            .into(),
        email: var(&format!("GIT_{role}_EMAIL"))
            .unwrap_or_else(|| default_email.clone())
            .into(),
        time: var(&format!("GIT_{role}_DATE"))
            .and_then(|value| gix::date::parse(&value, None).ok())
            .unwrap_or(now),
    };
    (pick("AUTHOR"), pick("COMMITTER"))
}

#[cfg(test)]
mod tests {
    use nurt_git::testutil::{TestHost, test_ctx, test_ctx_with};

    use super::super::probe::tests::Fixture;
    use super::*;

    #[test]
    fn maps_the_fast_forward_policies_of_a_pull() {
        assert_eq!(fast_forward(FfPolicy::Allow), FastForward::Auto);
        assert_eq!(fast_forward(FfPolicy::Never), FastForward::Never);
        assert_eq!(fast_forward(FfPolicy::Only), FastForward::Only);
    }

    #[test]
    fn takes_the_identity_from_the_configuration_and_the_clock() {
        let fixture = Fixture::new();
        fixture.config("[user]\n\tname = Puller\n\temail = puller@example.com\n");
        let ctx = test_ctx();
        let repo = GitRepo::open(fixture.root(), &ctx).unwrap();
        let (author, committer) = identity(&repo, &ctx);
        assert_eq!(author.name, "Puller");
        assert_eq!(committer.email, "puller@example.com");
        assert_eq!(committer.time.seconds, 1_767_268_800);
    }

    #[test]
    fn prefers_the_identity_variables_of_the_environment() {
        let fixture = Fixture::new();
        fixture.config("[user]\n\tname = Puller\n\temail = puller@example.com\n");
        let host = TestHost::new("/nonexistent-home")
            .with_var("GIT_AUTHOR_NAME", "Author")
            .with_var("GIT_COMMITTER_EMAIL", "committer@example.com")
            .with_var("GIT_COMMITTER_DATE", "1700000000 +0200");
        let ctx = test_ctx_with(host);
        let repo = GitRepo::open(fixture.root(), &ctx).unwrap();
        let (author, committer) = identity(&repo, &ctx);
        assert_eq!(author.name, "Author");
        assert_eq!(author.email, "puller@example.com");
        assert_eq!(committer.name, "Puller");
        assert_eq!(committer.email, "committer@example.com");
        assert_eq!(committer.time.seconds, 1_700_000_000);
    }

    #[test]
    fn refuses_a_merge_without_a_fetched_head() {
        let fixture = Fixture::new();
        let ctx = test_ctx();
        let repo = GitRepo::open(fixture.root(), &ctx).unwrap();
        let fetched = FetchOutcome::default();
        let input = MergeInput {
            label: "/work/repo",
            fetched: &fetched,
            message: String::new(),
            fast_forward: FfPolicy::Allow,
            reflog_action: "pull",
            initial: false,
        };
        let error = merge_request(&repo, &input, &ctx).unwrap_err();
        assert_eq!(error.code, nurt_contracts::error::ErrorCode::Internal);
    }
}
