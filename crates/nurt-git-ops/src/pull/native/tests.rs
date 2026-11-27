// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-git-ops::pull::native::tests
// Responsibility:   Tests of the fetch and rebase adapters against repositories in temporary.
// Dependencies:     gix, nurt_contracts, nurt_git
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Tests of the fetch and rebase adapters against repositories in temporary
//! directories; the remote of the fetch is a second local repository.

use std::fs;

use gix::ObjectId;
use nurt_contracts::error::ErrorCode;
use nurt_git::apply::testjournal::RecordingJournal;
use nurt_git::repo::GitRepo;
use nurt_git::testutil::test_ctx;

use super::super::probe::tests::Fixture;
use super::*;

const BASE: &[(&str, &str)] = &[("a.txt", "a\n")];
const UPSTREAM: &[(&str, &str)] = &[("a.txt", "a\nb\n")];
const LOCAL: &[(&str, &str)] = &[("a.txt", "a\n"), ("l.txt", "l\n")];
const LOCAL_AND_SIDE: &[(&str, &str)] = &[("a.txt", "a\n"), ("l.txt", "l\n"), ("s.txt", "s\n")];
const CONFLICTING: &[(&str, &str)] = &[("a.txt", "a\nx\n")];

struct Pair {
    local: Fixture,
    remote: Fixture,
    base: ObjectId,
    upstream: ObjectId,
    tip: ObjectId,
}

impl Pair {
    /// Both repositories share `base`; the remote branch `main` has one more
    /// commit, the local branch `main` has `local_files` on top of `base`.
    fn new(local_files: &[(&str, &str)]) -> Self {
        let mut remote = Fixture::new();
        let base = remote.commit(BASE, &[], "base");
        let upstream = remote.commit(UPSTREAM, &[base], "upstream");
        remote.set_ref("refs/heads/main", upstream);
        let mut local = Fixture::new();
        assert_eq!(local.commit(BASE, &[], "base"), base);
        let tip = local.commit(local_files, &[base], "local");
        local.set_ref("refs/heads/main", tip);
        local.set_ref("refs/remotes/origin/main", base);
        local.config(&format!(
            "[remote \"origin\"]\n\turl = {}\n\tfetch = +refs/heads/*:refs/remotes/origin/*\n[user]\n\tname = Puller\n\temail = puller@example.com\n[branch \"main\"]\n\tremote = origin\n\tmerge = refs/heads/main\n",
            remote.root().display().to_string().replace('\\', "\\\\")
        ));
        local.checkout(tip);
        Self {
            local,
            remote,
            base,
            upstream,
            tip,
        }
    }

    fn repo(&self) -> GitRepo {
        GitRepo::open(self.local.root(), &test_ctx()).unwrap()
    }

    fn branch(&self) -> ObjectId {
        let repo = nurt_git::testutil::open_isolated(self.local.root()).unwrap();
        repo.find_reference("refs/heads/main")
            .unwrap()
            .into_fully_peeled_id()
            .unwrap()
            .detach()
    }

    fn read(&self, path: &str) -> String {
        fs::read_to_string(self.local.root().join(path)).unwrap()
    }
}

fn fetch_request(remote: &str) -> FetchRequest {
    FetchRequest {
        remote: remote.to_owned(),
        refspecs: Vec::new(),
        reflog_action: "pull".to_owned(),
        accept_host_key: None,
    }
}

fn rebase_request(pair: &Pair, kind: RebaseKind) -> RebaseRequest {
    RebaseRequest {
        repo: "/home/user/repo".to_owned(),
        onto: pair.upstream,
        upstream: pair.base,
        kind,
        autostash: false,
        reflog_action: "pull --rebase".to_owned(),
    }
}

fn rebase(pair: &Pair, kind: RebaseKind) -> std::result::Result<LocalOutcome, RebaseError> {
    let journal = RecordingJournal::new();
    let mut tx = journal.begin();
    let fetched = PlanFetch
        .fetch(&pair.repo(), &fetch_request("origin"), &mut tx, &test_ctx())
        .unwrap();
    let outcome = PlanRebase.rebase(
        &pair.repo(),
        &rebase_request(pair, kind),
        fetched.local_plan(),
        &mut tx,
        &test_ctx(),
    );
    Box::new(tx).commit().unwrap();
    outcome
}

#[test]
fn fetch_plans_the_tracking_update_and_fetch_head() {
    let pair = Pair::new(LOCAL);
    let journal = RecordingJournal::new();
    let mut tx = journal.begin();
    let outcome = PlanFetch
        .fetch(&pair.repo(), &fetch_request("origin"), &mut tx, &test_ctx())
        .unwrap();
    Box::new(tx).commit().unwrap();

    assert!(
        outcome
            .url
            .contains(&pair.remote.root().display().to_string())
    );
    assert_eq!(outcome.for_merge.len(), 1);
    assert_eq!(outcome.for_merge[0].id, pair.upstream);
    assert_eq!(outcome.for_merge[0].remote_ref, "refs/heads/main");
    assert_eq!(outcome.ref_updates.len(), 1);
    assert_eq!(outcome.ref_updates[0].reference, "refs/remotes/origin/main");
    assert_eq!(
        outcome.refs.len(),
        2,
        "the tracking branch and the new remote HEAD"
    );
    assert!(
        outcome
            .state_files
            .iter()
            .any(|file| file.path == super::super::steps::FETCH_HEAD)
    );
    assert!(outcome.objects_received > 0);
    assert!(
        nurt_git::testutil::open_isolated(pair.local.root())
            .unwrap()
            .has_object(pair.upstream)
    );
    let tracking = pair.local.git_dir().join("refs/remotes/origin/main");
    assert_eq!(
        fs::read_to_string(tracking).unwrap().trim(),
        pair.base.to_string(),
        "the plan does not write the tracking reference"
    );
}

#[test]
fn fetch_rejects_an_unknown_remote() {
    let pair = Pair::new(LOCAL);
    let journal = RecordingJournal::new();
    let mut tx = journal.begin();
    let error = PlanFetch
        .fetch(
            &pair.repo(),
            &fetch_request("nowhere"),
            &mut tx,
            &test_ctx(),
        )
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::GitRef);
}

#[test]
fn rebase_treats_merge_preserving_mode_as_plain_for_linear_history() {
    let pair = Pair::new(LOCAL);
    rebase(&pair, RebaseKind::Merges).unwrap();
    assert_eq!(pair.read("l.txt"), "l\n");
}

#[test]
fn rebase_refuses_merge_preserving_mode_before_any_write() {
    let mut pair = Pair::new(LOCAL);
    let side = pair
        .local
        .commit(&[("a.txt", "a\n"), ("s.txt", "s\n")], &[pair.base], "side");
    let merge = pair
        .local
        .commit(LOCAL_AND_SIDE, &[pair.tip, side], "merge");
    pair.local.set_ref("refs/heads/main", merge);
    pair.tip = merge;
    let Err(RebaseError::Refused(error)) = rebase(&pair, RebaseKind::Merges) else {
        panic!("expected a refusal");
    };
    assert_eq!(error.code(), ErrorCode::GitUnsupported);
    assert_eq!(error.problem.as_deref(), Some("rebase_merges"));
    assert_eq!(pair.branch(), pair.tip);
}

#[test]
fn rebase_replays_local_commits_onto_the_fetched_head() {
    let pair = Pair::new(LOCAL);
    let outcome = rebase(&pair, RebaseKind::Plain).unwrap();

    let repo = nurt_git::testutil::open_isolated(pair.local.root()).unwrap();
    let rebased = repo.find_commit(pair.branch()).unwrap();
    assert_eq!(rebased.parent_ids().next().unwrap().detach(), pair.upstream);
    assert_eq!(pair.read("a.txt"), "a\nb\n");
    assert_eq!(pair.read("l.txt"), "l\n");
    assert!(!outcome.changes.is_empty());
}

#[test]
fn rebase_stops_with_a_conflict_when_changes_overlap() {
    let pair = Pair::new(CONFLICTING);
    let Err(RebaseError::Failed(error)) = rebase(&pair, RebaseKind::Plain) else {
        panic!("expected a stopped rebase");
    };
    assert_eq!(error.code(), ErrorCode::GitConflict);
}

#[test]
fn tracking_refspec_maps_a_branch_through_the_configured_refspec() {
    let pair = Pair::new(LOCAL);
    let repo = pair.repo();
    assert_eq!(
        tracking_refspec(&repo, "origin", "dev"),
        "refs/heads/dev:refs/remotes/origin/dev"
    );
    assert_eq!(
        tracking_refspec(&repo, "origin", "refs/heads/dev"),
        "refs/heads/dev:refs/remotes/origin/dev"
    );
}

#[test]
fn tracking_refspec_keeps_specs_without_a_configured_mapping() {
    let pair = Pair::new(LOCAL);
    let repo = pair.repo();
    assert_eq!(
        tracking_refspec(&repo, "origin", "refs/heads/dev:refs/heads/x"),
        "refs/heads/dev:refs/heads/x"
    );
    assert_eq!(
        tracking_refspec(&repo, "origin", "refs/tags/v1"),
        "refs/tags/v1"
    );
    assert_eq!(tracking_refspec(&repo, "nowhere", "dev"), "dev");
}

#[test]
fn tracking_refspec_maps_an_exact_configured_refspec() {
    let pair = Pair::new(LOCAL);
    pair.local
        .config("[remote \"exact\"]\n\tfetch = +refs/heads/dev:refs/remotes/exact/dev\n");
    assert_eq!(
        tracking_refspec(&pair.repo(), "exact", "dev"),
        "refs/heads/dev:refs/remotes/exact/dev"
    );
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
