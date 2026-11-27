// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-git::fetch::tests
// Responsibility:   Repository with three commits on `main`, a branch `topic` one commit behind, a branch `gone` and an annotated tag `v1` on the first commit.
// Dependencies:     gix, nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use gix::ObjectId;
use nurt_contracts::error::ErrorCode;
use nurt_contracts::record::GitRefStatus;

use super::*;
use crate::testutil::{Fixture, RecordingJournal, RepoBuilder, TreeEntry, snapshot, test_ctx};

/// Repository with three commits on `main`, a branch `topic` one commit
/// behind, a branch `gone` and an annotated tag `v1` on the first commit.
fn upstream() -> Fixture {
    RepoBuilder::new()
        .file("a.txt", "one\n")
        .commit("first")
        .file("a.txt", "two\n")
        .commit("second")
        .file("b.txt", "three\n")
        .commit("third")
        .reference("refs/heads/topic", "HEAD~1")
        .reference("refs/heads/gone", "HEAD~2")
        .annotated_tag("v1", "HEAD~2", "release")
        .build()
}

/// Empty repository whose remote `origin` is `upstream`.
fn downstream(upstream: &Fixture) -> Fixture {
    RepoBuilder::new()
        .config("remote.origin.url", &upstream.path().to_string_lossy())
        .config("remote.origin.fetch", "+refs/heads/*:refs/remotes/origin/*")
        .build()
}

fn open(fixture: &Fixture) -> GitRepo {
    GitRepo::open(fixture.path(), &test_ctx()).expect("repository opens")
}

fn id_of(fixture: &Fixture, name: &str) -> Option<ObjectId> {
    fixture
        .open()
        .try_find_reference(name)
        .expect("readable reference")
        .and_then(|reference| reference.target().try_id().map(ToOwned::to_owned))
}

fn run(fixture: &Fixture, request: &FetchRequest) -> (RecordingJournal, FetchPlan) {
    let repo = open(fixture);
    let journal = RecordingJournal::new();
    let mut tx = journal.begin();
    let plan = fetch(&repo, request, &mut *tx).expect("fetch succeeds");
    tx.commit().expect("transaction commits");
    (journal, plan)
}

fn set_ref(fixture: &Fixture, name: &str, id: Option<ObjectId>) {
    let path = name
        .split('/')
        .fold(fixture.git_dir(), |path, part| path.join(part));
    match id {
        Some(id) => std::fs::write(path, format!("{id}\n")).expect("reference written"),
        None => std::fs::remove_file(path).expect("reference removed"),
    }
}

fn status_of(plan: &FetchPlan, name: &str) -> Option<GitRefStatus> {
    plan.changes
        .iter()
        .find(|change| change.name == name)
        .map(|change| change.status)
}

/// State of the repository outside the object store, which only grows.
fn state(root: &Path) -> BTreeMap<PathBuf, TreeEntry> {
    snapshot(root)
        .into_iter()
        .filter(|(path, _)| !path.starts_with(".git/objects"))
        .collect()
}

#[test]
fn first_fetch_stores_branches_tags_and_fetch_head() {
    let up = upstream();
    let down = downstream(&up);
    let (_, plan) = run(&down, &FetchRequest::new("origin"));
    for branch in ["main", "topic", "gone"] {
        assert_eq!(
            id_of(&down, &format!("refs/remotes/origin/{branch}")),
            id_of(&up, &format!("refs/heads/{branch}")),
            "{branch}"
        );
        assert_eq!(
            status_of(&plan, &format!("refs/remotes/origin/{branch}")),
            Some(GitRefStatus::New)
        );
    }
    assert_eq!(id_of(&down, "refs/tags/v1"), id_of(&up, "refs/tags/v1"));
    let fetch_head = std::fs::read_to_string(down.git_dir().join(FETCH_HEAD)).unwrap();
    let url = up.path().to_string_lossy();
    assert!(fetch_head.contains(&format!("\tnot-for-merge\tbranch 'main' of {url}\n")));
    assert!(fetch_head.contains(&format!("\tnot-for-merge\ttag 'v1' of {url}\n")));
    assert!(plan.objects_received > 0);
    assert!(plan.bytes_received > 0);
    let log =
        std::fs::read_to_string(down.git_dir().join("logs/refs/remotes/origin/main")).unwrap();
    assert!(log.ends_with("\tfetch origin: storing head\n"), "{log}");
    assert!(!plan.is_up_to_date());
}

#[test]
fn second_fetch_without_changes_is_up_to_date() {
    let up = upstream();
    let down = downstream(&up);
    run(&down, &FetchRequest::new("origin"));
    let (_, plan) = run(&down, &FetchRequest::new("origin"));
    assert!(plan.is_up_to_date());
    assert!(
        plan.changes
            .iter()
            .all(|change| change.status == GitRefStatus::UpToDate)
    );
    assert_eq!(plan.ref_updates().len(), plan.changes.len());
}

#[test]
fn undo_restores_references_reflogs_and_fetch_head() {
    let up = upstream();
    let down = downstream(&up);
    let before = state(down.path());
    let (journal, _) = run(&down, &FetchRequest::new("origin"));
    assert_ne!(state(down.path()), before);
    journal.undo_last();
    assert_eq!(state(down.path()), before);
}

#[test]
fn updates_are_fast_forward_forced_and_pruned() {
    let up = upstream();
    let down = downstream(&up);
    run(&down, &FetchRequest::new("origin"));
    let main = id_of(&up, "refs/heads/main");
    let topic = id_of(&up, "refs/heads/topic");
    set_ref(&up, "refs/heads/topic", main);
    set_ref(&up, "refs/heads/main", topic);
    set_ref(&up, "refs/heads/gone", None);
    let mut request = FetchRequest::new("origin");
    request.prune = Some(true);
    let before = state(down.path());
    let (journal, plan) = run(&down, &request);
    assert_eq!(
        status_of(&plan, "refs/remotes/origin/topic"),
        Some(GitRefStatus::Updated)
    );
    assert_eq!(
        status_of(&plan, "refs/remotes/origin/main"),
        Some(GitRefStatus::Forced)
    );
    assert_eq!(
        status_of(&plan, "refs/remotes/origin/gone"),
        Some(GitRefStatus::Deleted)
    );
    assert_eq!(id_of(&down, "refs/remotes/origin/gone"), None);
    assert_eq!(id_of(&down, "refs/remotes/origin/main"), topic);
    let log =
        std::fs::read_to_string(down.git_dir().join("logs/refs/remotes/origin/main")).unwrap();
    assert!(
        log.ends_with("\tfetch --prune origin: forced-update\n"),
        "{log}"
    );
    let log =
        std::fs::read_to_string(down.git_dir().join("logs/refs/remotes/origin/topic")).unwrap();
    assert!(
        log.ends_with("\tfetch --prune origin: fast-forward\n"),
        "{log}"
    );
    journal.undo_last();
    assert_eq!(state(down.path()), before);
}

#[test]
fn non_fast_forward_without_plus_is_rejected() {
    let up = upstream();
    let down = downstream(&up);
    run(&down, &FetchRequest::new("origin"));
    let before = id_of(&down, "refs/remotes/origin/main");
    set_ref(&up, "refs/heads/main", id_of(&up, "refs/heads/topic"));
    let mut request = FetchRequest::new("origin");
    request.refspecs = vec!["refs/heads/main:refs/remotes/origin/main".to_owned()];
    let (_, plan) = run(&down, &request);
    let change = &plan.changes[0];
    assert_eq!(change.status, GitRefStatus::Rejected);
    assert_eq!(change.reason.as_deref(), Some("non_fast_forward"));
    assert_eq!(id_of(&down, "refs/remotes/origin/main"), before);
    assert_eq!(plan.for_merge.len(), 1);
}

#[test]
fn existing_tags_are_not_moved_without_force() {
    let up = upstream();
    let down = downstream(&up);
    run(&down, &FetchRequest::new("origin"));
    set_ref(&down, "refs/tags/v1", id_of(&up, "refs/heads/main"));
    let mut request = FetchRequest::new("origin");
    request.tags = Some(TagMode::All);
    let (_, plan) = run(&down, &request);
    let change = plan
        .changes
        .iter()
        .find(|change| change.name == "refs/tags/v1")
        .unwrap();
    assert_eq!(change.status, GitRefStatus::Rejected);
    assert_eq!(change.reason.as_deref(), Some("would_clobber_existing_tag"));
}

#[test]
fn no_tags_mode_fetches_no_tag() {
    let up = upstream();
    let down = downstream(&up);
    let mut request = FetchRequest::new("origin");
    request.tags = Some(TagMode::None);
    let (_, plan) = run(&down, &request);
    assert_eq!(id_of(&down, "refs/tags/v1"), None);
    assert!(
        !String::from_utf8_lossy(&plan.fetch_head).contains("tag 'v1'"),
        "no tag line expected"
    );
}

#[test]
fn explicit_refspec_without_destination_is_merged_only() {
    let up = upstream();
    let down = downstream(&up);
    let mut request = FetchRequest::new("origin");
    request.refspecs = vec!["refs/heads/topic".to_owned()];
    let (_, plan) = run(&down, &request);
    assert!(plan.changes.is_empty());
    assert_eq!(plan.for_merge.len(), 1);
    assert_eq!(Some(plan.for_merge[0].id), id_of(&up, "refs/heads/topic"));
    assert_eq!(id_of(&down, "refs/remotes/origin/topic"), None);
    let text = String::from_utf8(plan.fetch_head.clone()).unwrap();
    assert_eq!(text.lines().count(), 1);
    assert!(text.contains("\t\tbranch 'topic' of "));
}

#[test]
fn branch_merge_configuration_selects_the_head_to_merge() {
    let up = upstream();
    let down = RepoBuilder::new()
        .config("remote.origin.url", &up.path().to_string_lossy())
        .config("remote.origin.fetch", "+refs/heads/*:refs/remotes/origin/*")
        .config("branch.main.remote", "origin")
        .config("branch.main.merge", "refs/heads/main")
        .build();
    let (_, plan) = run(&down, &FetchRequest::new("origin"));
    assert_eq!(plan.for_merge.len(), 1);
    assert_eq!(plan.for_merge[0].remote_ref, "refs/heads/main");
    let text = String::from_utf8(plan.fetch_head.clone()).unwrap();
    assert!(
        text.lines()
            .next()
            .unwrap()
            .contains("\t\tbranch 'main' of ")
    );
}

#[test]
fn address_fetches_head_into_fetch_head() {
    let up = upstream();
    let down = RepoBuilder::new().build();
    let url = up.path().to_string_lossy().into_owned();
    let (_, plan) = run(&down, &FetchRequest::new(url.clone()));
    assert!(plan.changes.is_empty());
    assert_eq!(plan.remote, None);
    assert_eq!(
        String::from_utf8(plan.fetch_head.clone()).unwrap(),
        format!("{}\t\t{url}\n", id_of(&up, "refs/heads/main").unwrap())
    );
}

#[test]
fn unknown_remote_is_rejected() {
    let up = upstream();
    let down = downstream(&up);
    let journal = RecordingJournal::new();
    let mut tx = journal.begin();
    let error = fetch(&open(&down), &FetchRequest::new("nowhere"), &mut *tx).unwrap_err();
    assert_eq!(error.code, ErrorCode::GitRef);
    assert_eq!(error.problem.as_deref(), Some("unknown_remote"));
}

#[test]
fn missing_exact_refspec_is_rejected() {
    let up = upstream();
    let down = downstream(&up);
    let journal = RecordingJournal::new();
    let mut tx = journal.begin();
    let mut request = FetchRequest::new("origin");
    request.refspecs = vec!["refs/heads/missing".to_owned()];
    let error = fetch(&open(&down), &request, &mut *tx).unwrap_err();
    assert_eq!(error.problem.as_deref(), Some("no_match"));
}

#[test]
fn checked_out_branch_is_never_updated() {
    let up = upstream();
    let down = RepoBuilder::new()
        .file("local.txt", "local\n")
        .commit("local")
        .config("remote.origin.url", &up.path().to_string_lossy())
        .build();
    let journal = RecordingJournal::new();
    let mut tx = journal.begin();
    let mut request = FetchRequest::new("origin");
    request.refspecs = vec!["+refs/heads/main:refs/heads/main".to_owned()];
    let error = fetch(&open(&down), &request, &mut *tx).unwrap_err();
    assert_eq!(error.problem.as_deref(), Some("checked_out"));
}

#[test]
fn depth_writes_the_shallow_file_and_undo_removes_it() {
    let up = upstream();
    let down = downstream(&up);
    let mut request = FetchRequest::new("origin");
    request.depth = Some(1);
    let (journal, plan) = run(&down, &request);
    let shallow = std::fs::read_to_string(down.git_dir().join(SHALLOW)).unwrap();
    assert!(shallow.contains(&id_of(&up, "refs/heads/main").unwrap().to_string()));
    assert!(plan.shallow.is_some());
    journal.undo_last();
    assert!(!down.git_dir().join(SHALLOW).exists());
}

#[test]
fn zero_depth_is_invalid() {
    let up = upstream();
    let down = downstream(&up);
    let journal = RecordingJournal::new();
    let mut tx = journal.begin();
    let mut request = FetchRequest::new("origin");
    request.depth = Some(0);
    let error = fetch(&open(&down), &request, &mut *tx).unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidValue);
}

#[test]
fn plan_writes_only_the_object_store() {
    let up = upstream();
    let down = downstream(&up);
    let repo = open(&down);
    let journal = RecordingJournal::new();
    let mut tx = journal.begin();
    let plan = plan_fetch(&repo, &FetchRequest::new("origin"), &mut *tx).unwrap();
    tx.commit().unwrap();
    assert!(journal.last_targets().is_empty());
    assert!(!journal.exempt().is_empty());
    assert_eq!(id_of(&down, "refs/remotes/origin/main"), None);
    let committer = crate::checkpoint::support::committer(repo.gix(), repo.ctx());
    let plans = plan.ref_plans(repo.gix(), &committer).unwrap();
    assert_eq!(plans.len(), 4);
    assert_eq!(plan.state_files()[0].path, FETCH_HEAD);
    assert_eq!(
        plan.remote_head,
        Some((
            "refs/remotes/origin/HEAD".to_owned(),
            "refs/remotes/origin/main".to_owned()
        ))
    );
    assert_eq!(plan.ref_edits().unwrap().len(), 5);
}

#[test]
fn reflog_action_names_options_and_remote() {
    let mut request = FetchRequest::new("https://u:p@example.com/r");
    request.prune = Some(true);
    request.tags = Some(TagMode::All);
    request.depth = Some(3);
    assert_eq!(
        reflog_action(&request, true),
        "fetch --prune --tags --depth=3 https://example.com/r"
    );
    assert_eq!(reflog_action(&FetchRequest::new("origin"), false), "fetch");
}

#[test]
fn shallow_commits_reads_listed_identifiers_only() {
    let first = "8a45cf70a2d766acccaddafc8d74b7ef939807a8";
    let content = format!("{first}\nnot an identifier\n");
    let commits = update::shallow_commits(content.as_bytes());
    assert_eq!(commits.len(), 1);
    assert!(commits.contains(&ObjectId::from_hex(first.as_bytes()).unwrap()));
    assert!(update::shallow_commits(b"").is_empty());
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
