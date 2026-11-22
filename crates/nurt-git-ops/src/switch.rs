// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The `git.switch` command.
//!
//! Switching resolves the target (a local branch, a branch created first, a
//! commit with `detach`, the previous branch with `-`), plans the update of
//! the working tree and the index with the two-tree rules of `git`, and
//! points `HEAD` at the target. The reference log of `HEAD` receives the
//! line `checkout: moving from <old> to <new>` also when the target is the
//! current branch, as with `git switch`.

use bstr::{BString, ByteSlice};
use gix::ObjectId;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::journal::{Expect, MutationTx};
use nurt_git::apply::{
    ApplyContext, ApplyPlan, ApplyReport, Notable, RefPlan, RefTarget, ReflogLine, ResetOptions,
    StateFilePlan, apply_plan, plan_reset, plan_two_way,
};

pub(crate) mod stage;

pub use stage::SwitchCommand;

/// Prefix of local branch references.
const HEADS: &str = "refs/heads/";
/// Prefix of remote-tracking references.
const REMOTES: &str = "refs/remotes/";
/// Name that selects the previously checked out branch.
const PREVIOUS: &str = "-";

/// Parameters of a switch.
#[derive(Debug, Clone)]
pub struct SwitchRequest {
    /// Branch to switch to, `-` for the previous branch, or with `detach` any
    /// revision; `None` with `detach` detaches at the current commit.
    pub branch: Option<String>,
    /// Create the branch first, at `start_point`.
    pub create: bool,
    /// Revision at which a created branch starts; `HEAD` when absent.
    pub start_point: Option<String>,
    /// Detach `HEAD` at the commit instead of switching to a branch.
    pub detach: bool,
    /// Discard local changes of tracked files instead of carrying them over;
    /// untracked files in the way still block the switch.
    pub discard_changes: bool,
    /// Create a local branch tracking the only remote-tracking branch of the
    /// same name when no local branch exists.
    pub guess: bool,
    /// Identity and time recorded in the reference logs.
    pub committer: gix::actor::Signature,
}

/// Planned switch; executed by [`apply_switch`] or shown as a preview.
#[derive(Debug, Clone)]
pub struct SwitchPlan {
    /// Changes of the working tree, the index, `HEAD`, the created branch
    /// and the configuration.
    pub apply: ApplyPlan,
    /// Commit of `HEAD` before the switch; `None` on an unborn branch.
    pub head_before: Option<ObjectId>,
    /// Commit of `HEAD` after the switch.
    pub head_after: ObjectId,
    /// Full name of the branch `HEAD` names after the switch; `None` when
    /// detached.
    pub branch: Option<String>,
    /// The branch is created by the switch.
    pub created: bool,
}

/// Result of an executed switch.
#[derive(Debug, Clone)]
pub struct SwitchOutcome {
    /// Changes made.
    pub report: ApplyReport,
    /// Commit of `HEAD` before the switch.
    pub head_before: Option<ObjectId>,
    /// Commit of `HEAD` after the switch.
    pub head_after: ObjectId,
    /// Branch named by `HEAD`; `None` when detached.
    pub branch: Option<String>,
    /// Paths whose local changes were carried over.
    pub carried: Vec<BString>,
}

struct Target {
    commit: ObjectId,
    branch: Option<String>,
    to_label: String,
    create_from: Option<String>,
    tracking: Option<(String, String)>,
}

/// Plans a switch without changing anything.
///
/// # Errors
///
/// `E_GIT_IN_PROGRESS` during a merge, rebase, cherry-pick, revert or
/// `am`; `E_GIT_REF` when the branch exists for `create`, does not exist,
/// or names a commit without `detach`; `E_GIT_REVISION` for an unknown
/// revision; `E_GIT_DIRTY` when local changes or untracked files block the
/// update; `E_IO` when the repository cannot be read.
pub fn plan_switch(ctx: &ApplyContext<'_>, request: &SwitchRequest) -> Result<SwitchPlan> {
    let repo = ctx.repo();
    refuse_in_progress(repo)?;
    let head_name = repo
        .head_name()
        .map_err(|err| io_error(err, "read HEAD"))?
        .map(|name| name.as_bstr().to_string());
    let head_before = repo.head_id().ok().map(gix::Id::detach);
    let target = resolve(repo, request)?;
    let head_tree = head_before.map(|id| tree_of(repo, id)).transpose()?;
    let target_tree = tree_of(repo, target.commit)?;
    let mut apply = if request.discard_changes {
        plan_reset(ctx, &target_tree, ResetOptions::default())
    } else {
        plan_two_way(ctx, head_tree.as_ref().map(|id| id.as_ref()), &target_tree)
    }
    .map_err(|err| with_switch_fixes(err, request))?;
    let from_label = match (&head_name, head_before) {
        (Some(name), _) => short_branch(name).to_owned(),
        (None, Some(id)) => id.to_string(),
        (None, None) => "HEAD".to_owned(),
    };
    let null = ObjectId::null(repo.object_hash());
    if let Some(from) = &target.create_from
        && let Some(branch) = &target.branch
    {
        apply.refs.push(RefPlan::new(
            repo,
            branch,
            RefTarget::Object(target.commit),
            Some(line(
                request,
                null,
                target.commit,
                format!("branch: Created from {from}"),
            )),
        )?);
    }
    if let (Some((remote, merge)), Some(branch)) = (&target.tracking, &target.branch) {
        apply
            .state_files
            .push(tracking_config(repo, short_branch(branch), remote, merge)?);
    }
    let head_target = match &target.branch {
        Some(branch) => RefTarget::Symbolic(branch.clone()),
        None => RefTarget::Object(target.commit),
    };
    apply.refs.push(RefPlan::new(
        repo,
        "HEAD",
        head_target,
        Some(line(
            request,
            head_before.unwrap_or(null),
            target.commit,
            format!("checkout: moving from {from_label} to {}", target.to_label),
        )),
    )?);
    if target.branch.is_none() {
        apply.notable.push(Notable {
            kind: "detached_head".to_owned(),
            path: None,
        });
    }
    if !apply.carried.is_empty() {
        apply.notable.push(Notable {
            kind: "local_changes_carried".to_owned(),
            path: None,
        });
    }
    Ok(SwitchPlan {
        apply,
        head_before,
        head_after: target.commit,
        created: target.create_from.is_some(),
        branch: target.branch,
    })
}

/// Executes a planned switch in the transaction `tx`.
///
/// # Errors
///
/// The errors of [`apply_plan`]: `E_GIT_LOCKED`, `E_STALE` and the errors of
/// the transaction.
pub fn apply_switch(
    tx: &mut dyn MutationTx,
    ctx: &ApplyContext<'_>,
    plan: SwitchPlan,
) -> Result<SwitchOutcome> {
    let carried = plan.apply.carried.clone();
    let report = apply_plan(tx, ctx, plan.apply)?;
    Ok(SwitchOutcome {
        report,
        head_before: plan.head_before,
        head_after: plan.head_after,
        branch: plan.branch,
        carried,
    })
}

fn line(request: &SwitchRequest, old: ObjectId, new: ObjectId, message: String) -> ReflogLine {
    ReflogLine {
        old,
        new,
        committer: request.committer.clone(),
        message,
    }
}

fn io_error(err: impl std::fmt::Display, what: &str) -> Error {
    Error::new(ErrorCode::Io, format!("cannot {what}: {err}"))
}

fn refuse_in_progress(repo: &gix::Repository) -> Result<()> {
    use gix::state::InProgress;
    let operation = match repo.state() {
        None | Some(InProgress::Bisect) => return Ok(()),
        Some(InProgress::Merge) => "merging",
        Some(InProgress::Rebase | InProgress::RebaseInteractive) => "rebasing",
        Some(InProgress::CherryPick | InProgress::CherryPickSequence) => "cherry-picking",
        Some(InProgress::Revert | InProgress::RevertSequence) => "reverting",
        Some(_) => "applying patches",
    };
    Err(Error::new(
        ErrorCode::GitInProgress,
        format!("cannot switch branch while {operation}; finish or abort the operation first"),
    )
    .with_problem(operation))
}

fn short_branch(name: &str) -> &str {
    name.strip_prefix(HEADS).unwrap_or(name)
}

fn tree_of(repo: &gix::Repository, commit: ObjectId) -> Result<ObjectId> {
    let what = format!("read the tree of commit {commit}");
    repo.find_commit(commit)
        .map_err(|err| io_error(err, &what))?
        .tree_id()
        .map(gix::Id::detach)
        .map_err(|err| io_error(err, &what))
}

fn commit_of(repo: &gix::Repository, spec: &str) -> Result<ObjectId> {
    let id = repo
        .rev_parse_single(spec.as_bytes().as_bstr())
        .map_err(|_| {
            Error::new(
                ErrorCode::GitRevision,
                format!("invalid reference: {spec}; give a branch, tag or commit that exists"),
            )
            .with_problem("unknown")
            .with_found(spec.to_owned())
        })?;
    let object = id
        .object()
        .map_err(|err| io_error(err, &format!("read {spec}")))?;
    object
        .peel_to_commit()
        .map(|commit| commit.id)
        .map_err(|_| {
            Error::new(
                ErrorCode::GitRevision,
                format!("'{spec}' does not name a commit; give a branch, tag or commit"),
            )
            .with_problem("not_a_commit")
            .with_found(spec.to_owned())
        })
}

fn branch_exists(repo: &gix::Repository, full: &str) -> Result<bool> {
    repo.try_find_reference(full)
        .map(|found| found.is_some())
        .map_err(|err| io_error(err, &format!("read {full}")))
}

fn resolve(repo: &gix::Repository, request: &SwitchRequest) -> Result<Target> {
    let name = match request.branch.as_deref() {
        Some(PREVIOUS) => Some(previous_branch(repo)?),
        other => other.map(ToOwned::to_owned),
    };
    if request.detach {
        let spec = name.unwrap_or_else(|| "HEAD".to_owned());
        return Ok(Target {
            commit: commit_of(repo, &spec)?,
            branch: None,
            to_label: spec,
            create_from: None,
            tracking: None,
        });
    }
    let name = name.ok_or_else(|| {
        Error::new(
            ErrorCode::MissingParam,
            "a branch is required unless detach is true; give branch: \"<name>\"",
        )
    })?;
    let full = format!("{HEADS}{name}");
    if gix::refs::FullName::try_from(full.as_str()).is_err() {
        return Err(Error::new(
            ErrorCode::GitRef,
            format!(
                "'{name}' is not a valid branch name; use letters, digits, '-', '_', '/' and '.'"
            ),
        )
        .with_problem("invalid_name")
        .with_found(name));
    }
    if request.create {
        return create_target(repo, request, name, full);
    }
    if branch_exists(repo, &full)? {
        let commit = commit_of(repo, &full)?;
        return Ok(Target {
            commit,
            branch: Some(full),
            to_label: name,
            create_from: None,
            tracking: None,
        });
    }
    if request.guess
        && let Some(remote) = only_remote_branch(repo, &name)?
    {
        let remote_ref = format!("{REMOTES}{remote}/{name}");
        return Ok(Target {
            commit: commit_of(repo, &remote_ref)?,
            branch: Some(full.clone()),
            to_label: name,
            create_from: Some(remote_ref),
            tracking: Some((remote, full)),
        });
    }
    Err(not_a_branch(repo, &name))
}

fn create_target(
    repo: &gix::Repository,
    request: &SwitchRequest,
    name: String,
    full: String,
) -> Result<Target> {
    if branch_exists(repo, &full)? {
        return Err(Error::new(
            ErrorCode::GitRef,
            format!("a branch named '{name}' already exists; switch to it without create"),
        )
        .with_problem("exists")
        .with_found(name));
    }
    let start = request
        .start_point
        .clone()
        .unwrap_or_else(|| "HEAD".to_owned());
    Ok(Target {
        commit: commit_of(repo, &start)?,
        branch: Some(full),
        to_label: name,
        create_from: Some(start),
        tracking: None,
    })
}

fn not_a_branch(repo: &gix::Repository, name: &str) -> Error {
    if commit_of(repo, name).is_ok() {
        return Error::new(
            ErrorCode::GitRef,
            format!(
                "a branch is expected, got '{name}'; switch with detach: true to check out a commit"
            ),
        )
        .with_problem("expected_branch")
        .with_found(name.to_owned());
    }
    Error::new(
        ErrorCode::GitRevision,
        format!("invalid reference: {name}; create the branch with create: true or check the name"),
    )
    .with_problem("unknown")
    .with_found(name.to_owned())
}

/// Remote of the only remote-tracking branch named `name`.
fn only_remote_branch(repo: &gix::Repository, name: &str) -> Result<Option<String>> {
    let references = repo
        .references()
        .map_err(|err| io_error(err, "list references"))?;
    let iter = references
        .remote_branches()
        .map_err(|err| io_error(err, "list references"))?;
    let mut remotes = Vec::new();
    for reference in iter.flatten() {
        let full = reference.name().as_bstr().to_string();
        if let Some((remote, branch)) = full
            .strip_prefix(REMOTES)
            .and_then(|rest| rest.split_once('/'))
            && branch == name
        {
            remotes.push(remote.to_owned());
        }
    }
    Ok(if remotes.len() == 1 {
        remotes.pop()
    } else {
        None
    })
}

/// Branch left by the last `checkout: moving from <a> to <b>` line of the
/// log of `HEAD`.
fn previous_branch(repo: &gix::Repository) -> Result<String> {
    const MARKER: &str = "\tcheckout: moving from ";
    let path = repo.git_dir().join("logs").join("HEAD");
    let log = std::fs::read(&path).unwrap_or_default();
    log.lines()
        .rev()
        .find_map(|line| {
            let line = line.to_str().ok()?;
            let rest = &line[line.find(MARKER)? + MARKER.len()..];
            rest.rsplit_once(" to ").map(|(from, _)| from.to_owned())
        })
        .ok_or_else(|| {
            Error::new(
                ErrorCode::GitRevision,
                "there is no previous branch in the log of HEAD; give the branch name",
            )
            .with_problem("no_previous")
        })
}

fn tracking_config(
    repo: &gix::Repository,
    branch: &str,
    remote: &str,
    merge: &str,
) -> Result<StateFilePlan> {
    let path = repo.git_dir().join("config");
    let mut content =
        std::fs::read(&path).map_err(|err| io_error(err, "read the configuration"))?;
    if !content.is_empty() && !content.ends_with(b"\n") {
        content.push(b'\n');
    }
    content.extend_from_slice(
        format!("[branch \"{branch}\"]\n\tremote = {remote}\n\tmerge = {merge}\n").as_bytes(),
    );
    Ok(StateFilePlan {
        path: "config".to_owned(),
        content: Some(content),
        expect: Expect::Unchecked,
    })
}

fn with_switch_fixes(error: Error, request: &SwitchRequest) -> Error {
    if error.code() != ErrorCode::GitDirty {
        return error;
    }
    let branch = request.branch.clone().unwrap_or_default();
    error
        .with_fix("git.stash \".\" action: push".to_owned())
        .with_fix(format!(
            "git.switch \".\" branch: \"{branch}\" discard_changes: true"
        ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use gix::object::tree::EntryKind;
    use nurt_git::apply::testjournal::{RecordingJournal, tree_state};

    fn signature() -> gix::actor::Signature {
        gix::actor::Signature {
            name: "Test".into(),
            email: "test@example.com".into(),
            time: gix::date::Time::new(1_700_000_000, 0),
        }
    }

    fn request(branch: &str) -> SwitchRequest {
        SwitchRequest {
            branch: Some(branch.to_owned()),
            create: false,
            start_point: None,
            detach: false,
            discard_changes: false,
            guess: false,
            committer: signature(),
        }
    }

    struct Fixture {
        _dir: tempfile::TempDir,
        repo: gix::Repository,
        journal: RecordingJournal,
        main: ObjectId,
        side: ObjectId,
    }

    fn commit(repo: &gix::Repository, files: &[(&str, &str)], parents: &[ObjectId]) -> ObjectId {
        let mut editor = repo.edit_tree(repo.empty_tree().id).expect("editor");
        for (path, content) in files {
            let blob = repo.write_blob(content.as_bytes()).expect("blob").detach();
            editor.upsert(*path, EntryKind::Blob, blob).expect("upsert");
        }
        let tree = editor.write().expect("tree").detach();
        repo.new_commit_as(
            signature().to_ref(&mut Default::default()),
            signature().to_ref(&mut Default::default()),
            "c",
            tree,
            parents.iter().copied(),
        )
        .expect("commit")
        .id
    }

    fn write_ref(repo: &gix::Repository, name: &str, content: &str) {
        let path = repo.git_dir().join(name);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("dirs");
        std::fs::write(path, content).expect("ref");
    }

    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().expect("temporary directory");
        let repo = gix::ThreadSafeRepository::init_opts(
            dir.path(),
            gix::create::Kind::WithWorktree,
            gix::create::Options::default(),
            gix::open::Options::isolated(),
        )
        .expect("init")
        .to_thread_local();
        let main = commit(&repo, &[("a.txt", "a\n"), ("same.txt", "s\n")], &[]);
        let side = commit(
            &repo,
            &[("a.txt", "side\n"), ("same.txt", "s\n"), ("b.txt", "b\n")],
            &[main],
        );
        write_ref(&repo, "refs/heads/main", &format!("{main}\n"));
        write_ref(&repo, "refs/heads/side", &format!("{side}\n"));
        write_ref(&repo, "HEAD", "ref: refs/heads/main\n");
        let journal = RecordingJournal::new();
        let ctx = ApplyContext::open(&repo).expect("context");
        let plan = nurt_git::apply::plan_new_tree(&ctx, &tree_of(&repo, main).expect("tree"))
            .expect("plan");
        let mut tx = journal.begin();
        apply_plan(&mut tx, &ctx, plan).expect("apply");
        Box::new(tx).commit().expect("commit");
        Fixture {
            _dir: dir,
            repo,
            journal,
            main,
            side,
        }
    }

    fn switch(fixture: &Fixture, request: &SwitchRequest) -> Result<SwitchOutcome> {
        let ctx = ApplyContext::open(&fixture.repo)?;
        let plan = plan_switch(&ctx, request)?;
        let mut tx = fixture.journal.begin();
        let outcome = apply_switch(&mut tx, &ctx, plan)?;
        Box::new(tx).commit()?;
        Ok(outcome)
    }

    fn git_file(fixture: &Fixture, name: &str) -> String {
        std::fs::read_to_string(fixture.repo.git_dir().join(name)).expect("read")
    }

    #[test]
    fn switch_moves_head_and_writes_reflog() {
        let fixture = fixture();
        let outcome = switch(&fixture, &request("side")).expect("switch");
        assert_eq!(outcome.head_after, fixture.side);
        assert_eq!(git_file(&fixture, "HEAD"), "ref: refs/heads/side\n");
        let log = git_file(&fixture, "logs/HEAD");
        assert!(
            log.ends_with("\tcheckout: moving from main to side\n"),
            "{log}"
        );
        let root = fixture.repo.workdir().expect("workdir");
        assert_eq!(
            std::fs::read_to_string(root.join("b.txt")).expect("b"),
            "b\n"
        );
    }

    #[test]
    fn switch_to_current_branch_still_logs_the_move() {
        let fixture = fixture();
        switch(&fixture, &request("main")).expect("switch");
        assert!(git_file(&fixture, "logs/HEAD").ends_with("moving from main to main\n"));
    }

    #[test]
    fn switch_create_records_branch_creation() {
        let fixture = fixture();
        let mut create = request("topic");
        create.create = true;
        switch(&fixture, &create).expect("switch");
        assert_eq!(
            git_file(&fixture, "refs/heads/topic"),
            format!("{}\n", fixture.main)
        );
        assert!(
            git_file(&fixture, "logs/refs/heads/topic").ends_with("\tbranch: Created from HEAD\n")
        );
        let error = switch(&fixture, &create).expect_err("exists");
        assert_eq!(error.code(), ErrorCode::GitRef);
    }

    #[test]
    fn switch_detach_points_head_at_commit() {
        let fixture = fixture();
        let mut detach = request("side");
        detach.detach = true;
        let outcome = switch(&fixture, &detach).expect("switch");
        assert!(
            outcome
                .report
                .notable
                .iter()
                .any(|item| item.kind == "detached_head")
        );
        assert_eq!(git_file(&fixture, "HEAD"), format!("{}\n", fixture.side));
        switch(&fixture, &request(PREVIOUS)).expect("previous");
        assert_eq!(git_file(&fixture, "HEAD"), "ref: refs/heads/main\n");
    }

    #[test]
    fn switch_refuses_commit_without_detach_and_unknown_names() {
        let fixture = fixture();
        let error = switch(&fixture, &request(&fixture.side.to_string())).expect_err("commit");
        assert_eq!(error.data().problem.as_deref(), Some("expected_branch"));
        let error = switch(&fixture, &request("nope")).expect_err("unknown");
        assert_eq!(error.code(), ErrorCode::GitRevision);
    }

    #[test]
    fn switch_refuses_local_change_and_discard_changes_overrides() {
        let fixture = fixture();
        let root = fixture.repo.workdir().expect("workdir").to_path_buf();
        std::fs::write(root.join("a.txt"), "local\n").expect("write");
        let error = switch(&fixture, &request("side")).expect_err("dirty");
        assert_eq!(error.code(), ErrorCode::GitDirty);
        assert!(!error.data().fix.is_empty());
        let mut discard = request("side");
        discard.discard_changes = true;
        switch(&fixture, &discard).expect("switch");
        assert_eq!(
            std::fs::read_to_string(root.join("a.txt")).expect("a"),
            "side\n"
        );
    }

    #[test]
    fn switch_refuses_during_merge() {
        let fixture = fixture();
        write_ref(&fixture.repo, "MERGE_HEAD", &format!("{}\n", fixture.side));
        let error = switch(&fixture, &request("side")).expect_err("in progress");
        assert_eq!(error.code(), ErrorCode::GitInProgress);
    }

    #[test]
    fn switch_guess_creates_tracking_branch() {
        let fixture = fixture();
        write_ref(
            &fixture.repo,
            "refs/remotes/origin/feature",
            &format!("{}\n", fixture.side),
        );
        let mut guess = request("feature");
        assert!(switch(&fixture, &guess).is_err(), "no guessing by default");
        guess.guess = true;
        switch(&fixture, &guess).expect("switch");
        assert!(
            git_file(&fixture, "config").contains(
                "[branch \"feature\"]\n\tremote = origin\n\tmerge = refs/heads/feature\n"
            )
        );
        assert!(
            git_file(&fixture, "logs/refs/heads/feature")
                .ends_with("branch: Created from refs/remotes/origin/feature\n")
        );
    }

    #[test]
    fn undo_of_switch_restores_previous_state() {
        let fixture = fixture();
        let root = fixture.repo.workdir().expect("workdir").to_path_buf();
        let before = tree_state(&root);
        switch(&fixture, &request("side")).expect("switch");
        fixture.journal.undo_last();
        assert_eq!(tree_state(&root), before);
    }
}
