// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Tests of three-way merges of trees and commits.

use std::sync::Arc;

use gix_object::tree::EntryKind;
use nurt_contracts::error::ErrorCode;
use nurt_contracts::record::GitConflictKind;
use nurt_contracts::report::Notable;
use nurt_contracts::services::ProcessHost;

use super::fixture::{DriverHost, Fixture, context};
use super::*;

const BASE: &str = "a\nb\nc\n";
const OURS: &str = "a\nb2\nc\n";
const THEIRS: &str = "a\nB\nc\n";

fn options(theirs: &str) -> MergeOptions {
    MergeOptions {
        labels: MergeLabels::head_and(theirs),
        ..MergeOptions::default()
    }
}

#[test]
fn merges_disjoint_changes_cleanly() {
    let fx = Fixture::new();
    let base = fx.tree(&[("f", BASE), ("g", "x\n")]);
    let ours = fx.tree(&[("f", OURS), ("g", "x\n")]);
    let theirs = fx.tree(&[("f", BASE), ("g", "y\n"), ("new/h", "h\n")]);
    let out = merge_trees(
        &fx.repo,
        &base,
        &ours,
        &theirs,
        &options("topic"),
        DriverMode::Defer,
    )
    .unwrap();
    assert!(out.is_clean());
    assert_eq!(fx.read(out.tree, "f").as_deref(), Some(OURS));
    assert_eq!(fx.read(out.tree, "g").as_deref(), Some("y\n"));
    assert_eq!(fx.read(out.tree, "new/h").as_deref(), Some("h\n"));
    assert_eq!(out.index.entries().len(), 3);
    assert_eq!(out.base_tree, base);
    assert!(out.merge_bases.is_empty());
}

#[test]
fn writes_conflict_markers_like_git_in_merge_style() {
    let fx = Fixture::new();
    let base = fx.tree(&[("f", BASE)]);
    let ours = fx.tree(&[("f", OURS)]);
    let theirs = fx.tree(&[("f", THEIRS)]);
    let out = merge_trees(
        &fx.repo,
        &base,
        &ours,
        &theirs,
        &options("topic"),
        DriverMode::Defer,
    )
    .unwrap();
    assert_eq!(
        fx.read(out.tree, "f").as_deref(),
        Some("a\n<<<<<<< HEAD\nb2\n=======\nB\n>>>>>>> topic\nc\n")
    );
    assert_eq!(out.conflicts.len(), 1);
    let conflict = &out.conflicts[0];
    assert_eq!(conflict.path, "f");
    assert_eq!(conflict.kind, GitConflictKind::BothModified);
    assert!(conflict.markers);
    assert!(conflict.base.is_some() && conflict.ours.is_some() && conflict.theirs.is_some());
    let stages: Vec<u8> = out
        .index
        .entries()
        .iter()
        .map(|e| e.stage_raw() as u8)
        .collect();
    assert_eq!(stages, vec![1, 2, 3]);
    assert_eq!(out.conflicted_paths(), vec!["f"]);
    assert_eq!(conflict.to_record().kind, GitConflictKind::BothModified);
}

#[test]
fn writes_base_section_in_diff3_and_zdiff3_styles() {
    let fx = Fixture::new();
    let base = fx.tree(&[("f", BASE)]);
    let ours = fx.tree(&[("f", OURS)]);
    let theirs = fx.tree(&[("f", THEIRS)]);
    for style in [ConflictStyle::Diff3, ConflictStyle::Zdiff3] {
        let mut opts = options("topic");
        opts.conflict_style = style;
        opts.labels.base = Some("9057a25".to_owned());
        let out = merge_trees(&fx.repo, &base, &ours, &theirs, &opts, DriverMode::Defer).unwrap();
        assert_eq!(
            fx.read(out.tree, "f").as_deref(),
            Some("a\n<<<<<<< HEAD\nb2\n||||||| 9057a25\nb\n=======\nB\n>>>>>>> topic\nc\n"),
            "style {}",
            style.as_str()
        );
    }
}

#[test]
fn honours_marker_size() {
    let fx = Fixture::new();
    let base = fx.tree(&[("f", BASE)]);
    let ours = fx.tree(&[("f", OURS)]);
    let theirs = fx.tree(&[("f", THEIRS)]);
    let mut opts = options("t");
    opts.marker_size = std::num::NonZeroU8::new(3).unwrap();
    let out = merge_trees(&fx.repo, &base, &ours, &theirs, &opts, DriverMode::Defer).unwrap();
    assert_eq!(
        fx.read(out.tree, "f").as_deref(),
        Some("a\n<<< HEAD\nb2\n===\nB\n>>> t\nc\n")
    );
}

#[test]
fn favor_resolves_conflicting_hunks() {
    let fx = Fixture::new();
    let base = fx.tree(&[("f", BASE)]);
    let ours = fx.tree(&[("f", OURS)]);
    let theirs = fx.tree(&[("f", THEIRS)]);
    for (favor, expected) in [(Favor::Ours, OURS), (Favor::Theirs, THEIRS)] {
        let mut opts = options("topic");
        opts.favor = Some(favor);
        let out = merge_trees(&fx.repo, &base, &ours, &theirs, &opts, DriverMode::Defer).unwrap();
        assert!(out.is_clean());
        assert_eq!(fx.read(out.tree, "f").as_deref(), Some(expected));
    }
    assert_eq!(Favor::parse("ours"), Some(Favor::Ours));
    assert_eq!(Favor::parse("theirs"), Some(Favor::Theirs));
    assert_eq!(Favor::parse("both"), None);
}

#[test]
fn keeps_modified_side_of_modify_delete_conflict() {
    let fx = Fixture::new();
    let base = fx.tree(&[("f", BASE), ("keep", "k\n")]);
    let ours = fx.tree(&[("f", OURS), ("keep", "k\n")]);
    let theirs = fx.tree(&[("keep", "k\n")]);
    let out = merge_trees(
        &fx.repo,
        &base,
        &ours,
        &theirs,
        &options("topic"),
        DriverMode::Defer,
    )
    .unwrap();
    assert_eq!(fx.read(out.tree, "f").as_deref(), Some(OURS));
    assert_eq!(out.conflicts.len(), 1);
    assert_eq!(out.conflicts[0].kind, GitConflictKind::DeletedByThem);
    assert!(!out.conflicts[0].markers);
    assert!(out.conflicts[0].theirs.is_none());

    let reverse = merge_trees(
        &fx.repo,
        &base,
        &theirs,
        &ours,
        &options("topic"),
        DriverMode::Defer,
    )
    .unwrap();
    assert_eq!(reverse.conflicts[0].kind, GitConflictKind::DeletedByUs);
    assert_eq!(fx.read(reverse.tree, "f").as_deref(), Some(OURS));
}

#[test]
fn reports_add_add_conflict_with_empty_base() {
    let fx = Fixture::new();
    let base = fx.tree(&[("x", "x\n")]);
    let ours = fx.tree(&[("x", "x\n"), ("n", "one\n")]);
    let theirs = fx.tree(&[("x", "x\n"), ("n", "two\n")]);
    let out = merge_trees(
        &fx.repo,
        &base,
        &ours,
        &theirs,
        &options("topic"),
        DriverMode::Defer,
    )
    .unwrap();
    assert_eq!(out.conflicts.len(), 1);
    assert_eq!(out.conflicts[0].kind, GitConflictKind::BothAdded);
    assert!(out.conflicts[0].markers);
    assert_eq!(
        fx.read(out.tree, "n").as_deref(),
        Some("<<<<<<< HEAD\none\n=======\ntwo\n>>>>>>> topic\n")
    );
}

#[test]
fn keeps_current_side_of_binary_conflict() {
    let fx = Fixture::new();
    let bin = |text: &str| -> Vec<u8> {
        let mut data = vec![0u8, 1, 2];
        data.extend_from_slice(text.as_bytes());
        data
    };
    let (b, o, t) = (bin("base"), bin("ours"), bin("theirs"));
    let base = fx.tree_with(&[("bin", EntryKind::Blob, &b)]);
    let ours = fx.tree_with(&[("bin", EntryKind::Blob, &o)]);
    let theirs = fx.tree_with(&[("bin", EntryKind::Blob, &t)]);
    let out = merge_trees(
        &fx.repo,
        &base,
        &ours,
        &theirs,
        &options("topic"),
        DriverMode::Defer,
    )
    .unwrap();
    assert_eq!(out.conflicts.len(), 1);
    assert_eq!(out.conflicts[0].kind, GitConflictKind::Binary);
    assert!(!out.conflicts[0].markers);
    assert_eq!(out.conflicts[0].ours, Some(fx.blob(&o)));
}

#[test]
fn merges_executable_bit_from_either_side() {
    let fx = Fixture::new();
    let base = fx.tree_with(&[
        ("s", EntryKind::Blob, b"run\n"),
        ("t", EntryKind::Blob, b"t\n"),
    ]);
    let ours = fx.tree_with(&[
        ("s", EntryKind::Blob, b"run\n"),
        ("t", EntryKind::Blob, b"t2\n"),
    ]);
    let theirs = fx.tree_with(&[
        ("s", EntryKind::BlobExecutable, b"run\n"),
        ("t", EntryKind::Blob, b"t\n"),
    ]);
    let out = merge_trees(
        &fx.repo,
        &base,
        &ours,
        &theirs,
        &options("topic"),
        DriverMode::Defer,
    )
    .unwrap();
    assert!(out.is_clean());
    assert_eq!(fx.mode(out.tree, "s"), Some(EntryKind::BlobExecutable));
    assert_eq!(fx.read(out.tree, "t").as_deref(), Some("t2\n"));
}

#[test]
fn follows_rename_on_one_side_with_change_on_the_other() {
    let fx = Fixture::new();
    let text = "line 1\nline 2\nline 3\nline 4\nline 5\nline 6\nline 7\nline 8\n";
    let changed = "line 1\nline 2\nline 3\nline 4\nline 5\nline 6\nline 7\nline 8 changed\n";
    let base = fx.tree(&[("old.txt", text)]);
    let ours = fx.tree(&[("new.txt", text)]);
    let theirs = fx.tree(&[("old.txt", changed)]);
    let out = merge_trees(
        &fx.repo,
        &base,
        &ours,
        &theirs,
        &options("topic"),
        DriverMode::Defer,
    )
    .unwrap();
    assert!(out.is_clean());
    assert_eq!(fx.read(out.tree, "new.txt").as_deref(), Some(changed));
    assert_eq!(fx.read(out.tree, "old.txt"), None);

    let mut no_renames = options("topic");
    no_renames.renames = None;
    let out = merge_trees(
        &fx.repo,
        &base,
        &ours,
        &theirs,
        &no_renames,
        DriverMode::Defer,
    )
    .unwrap();
    assert_eq!(out.conflicts.len(), 1);
    assert_eq!(out.conflicts[0].kind, GitConflictKind::DeletedByUs);
}

#[test]
fn reports_rename_rename_conflict() {
    let fx = Fixture::new();
    let text = "1\n2\n3\n4\n5\n6\n7\n8\n9\n";
    let base = fx.tree(&[("a", text)]);
    let ours = fx.tree(&[("b", text)]);
    let theirs = fx.tree(&[("c", text)]);
    let out = merge_trees(
        &fx.repo,
        &base,
        &ours,
        &theirs,
        &options("topic"),
        DriverMode::Defer,
    )
    .unwrap();
    assert!(!out.is_clean());
    assert!(
        out.conflicts
            .iter()
            .any(|conflict| conflict.kind == GitConflictKind::RenameRename),
        "{:?}",
        out.conflicts
    );
}

#[test]
fn merges_commits_with_their_best_common_ancestor() {
    let fx = Fixture::new();
    let root = fx.commit(fx.tree(&[("f", BASE)]), &[], "root");
    let ours = fx.commit(fx.tree(&[("f", OURS)]), &[root], "ours");
    let theirs = fx.commit(fx.tree(&[("f", BASE), ("g", "g\n")]), &[root], "theirs");
    assert_eq!(merge_bases(&fx.repo, ours, theirs).unwrap(), vec![root]);
    let out = merge_commits(&fx.repo, ours, theirs, &options("topic"), DriverMode::Defer).unwrap();
    assert!(out.is_clean());
    assert_eq!(out.merge_bases, vec![root]);
    assert!(out.virtual_bases.is_empty());
    assert_eq!(fx.read(out.tree, "f").as_deref(), Some(OURS));
    assert_eq!(fx.read(out.tree, "g").as_deref(), Some("g\n"));
}

#[test]
fn labels_base_with_abbreviated_commit_in_diff3() {
    let fx = Fixture::new();
    let root = fx.commit(fx.tree(&[("f", BASE)]), &[], "root");
    let ours = fx.commit(fx.tree(&[("f", OURS)]), &[root], "ours");
    let theirs = fx.commit(fx.tree(&[("f", THEIRS)]), &[root], "theirs");
    let mut opts = options("topic");
    opts.conflict_style = ConflictStyle::Diff3;
    let out = merge_commits(&fx.repo, ours, theirs, &opts, DriverMode::Defer).unwrap();
    let short = root.to_hex_with_len(7).to_string();
    assert_eq!(
        fx.read(out.tree, "f").unwrap(),
        format!("a\n<<<<<<< HEAD\nb2\n||||||| {short}\nb\n=======\nB\n>>>>>>> topic\nc\n")
    );
}

#[test]
fn merges_criss_cross_history_through_virtual_base() {
    let fx = Fixture::new();
    let root = fx.commit(fx.tree(&[("f", "0\n"), ("g", "0\n")]), &[], "root");
    let a1 = fx.commit(fx.tree(&[("f", "1\n"), ("g", "0\n")]), &[root], "a1");
    let b1 = fx.commit(fx.tree(&[("f", "0\n"), ("g", "1\n")]), &[root], "b1");
    let a2 = fx.commit(fx.tree(&[("f", "1\n"), ("g", "1\n")]), &[a1, b1], "a2");
    let b2 = fx.commit(
        fx.tree(&[("f", "1\n"), ("g", "1\n"), ("h", "h\n")]),
        &[b1, a1],
        "b2",
    );
    let bases = merge_bases(&fx.repo, a2, b2).unwrap();
    assert_eq!(bases.len(), 2);
    let out = merge_commits(&fx.repo, a2, b2, &options("b2"), DriverMode::Defer).unwrap();
    assert!(out.is_clean());
    assert_eq!(out.virtual_bases.len(), 1);
    assert_eq!(fx.read(out.tree, "h").as_deref(), Some("h\n"));
    let virtual_tree = merge_base_virtual(&fx.repo, &bases, &options("b2")).unwrap();
    assert_eq!(fx.read(virtual_tree, "f").as_deref(), Some("1\n"));
}

#[test]
fn refuses_unrelated_histories_unless_allowed() {
    let fx = Fixture::new();
    let ours = fx.commit(fx.tree(&[("a", "a\n")]), &[], "a");
    let theirs = fx.commit(fx.tree(&[("b", "b\n")]), &[], "b");
    let error =
        merge_commits(&fx.repo, ours, theirs, &options("other"), DriverMode::Defer).unwrap_err();
    assert_eq!(error.code(), ErrorCode::GitNonFastForward);
    assert_eq!(error.data().problem.as_deref(), Some("unrelated_histories"));
    let mut opts = options("other");
    opts.allow_unrelated = true;
    let out = merge_commits(&fx.repo, ours, theirs, &opts, DriverMode::Defer).unwrap();
    assert!(out.is_clean());
    assert_eq!(
        out.base_tree,
        gix_hash::ObjectId::empty_tree(gix_hash::Kind::Sha1)
    );
    assert_eq!(fx.read(out.tree, "a").as_deref(), Some("a\n"));
    assert_eq!(fx.read(out.tree, "b").as_deref(), Some("b\n"));
}

#[test]
fn reads_options_from_repository_configuration() {
    let fx = Fixture::new();
    let config = fx.repo.git_dir().join("config");
    let mut text = std::fs::read_to_string(&config).unwrap();
    text.push_str(
        "[merge]\n\tconflictStyle = zdiff3\n\trenames = false\n\trenormalize = true\n\tdefault = keep\n[merge \"keep\"]\n\tname = keep ours\n\tdriver = keep %A\n",
    );
    std::fs::write(&config, text).unwrap();
    let repo = gix::open(fx.repo.git_dir()).unwrap();
    let opts = MergeOptions::from_repo(&repo).unwrap();
    assert_eq!(opts.conflict_style, ConflictStyle::Zdiff3);
    assert!(opts.renames.is_none());
    assert!(opts.renormalize);
    assert_eq!(opts.default_driver.as_deref(), Some("keep"));
    assert_eq!(
        opts.drivers,
        vec![ExternalDriver {
            name: "keep".to_owned(),
            command: "keep %A".to_owned()
        }]
    );
}

#[test]
fn default_options_use_ort_rename_limit_and_git_marker_size() {
    let fx = Fixture::new();
    let opts = MergeOptions::from_repo(&fx.repo).unwrap();
    assert_eq!(opts.conflict_style, ConflictStyle::Merge);
    assert_eq!(opts.marker_size.get(), DEFAULT_MARKER_SIZE);
    assert_eq!(opts.renames.map(|r| r.limit), Some(DEFAULT_RENAME_LIMIT));
    assert!(opts.drivers.is_empty());
    assert_eq!(ConflictStyle::parse("diff3"), Some(ConflictStyle::Diff3));
    assert_eq!(ConflictStyle::Merge.as_str(), "merge");
}

#[test]
fn rejects_unknown_conflict_style_in_configuration() {
    let fx = Fixture::new();
    let config = fx.repo.git_dir().join("config");
    let mut text = std::fs::read_to_string(&config).unwrap();
    text.push_str("[merge]\n\tconflictStyle = fancy\n");
    std::fs::write(&config, text).unwrap();
    let repo = gix::open(fx.repo.git_dir()).unwrap();
    let error = MergeOptions::from_repo(&repo).unwrap_err();
    assert_eq!(error.code(), ErrorCode::Config);
}

fn driver_fixture() -> (
    Fixture,
    gix_hash::ObjectId,
    gix_hash::ObjectId,
    gix_hash::ObjectId,
    MergeOptions,
) {
    let fx = Fixture::new();
    std::fs::write(fx.workdir().join(".gitattributes"), "*.db merge=custom\n").unwrap();
    let base = fx.tree(&[("data.db", BASE), ("plain", BASE)]);
    let ours = fx.tree(&[("data.db", OURS), ("plain", OURS)]);
    let theirs = fx.tree(&[("data.db", THEIRS), ("plain", THEIRS)]);
    let mut opts = options("topic");
    opts.drivers = vec![ExternalDriver {
        name: "custom".to_owned(),
        command: "custom-merge %A %O %B %P".to_owned(),
    }];
    (fx, base, ours, theirs, opts)
}

#[test]
fn defers_configured_driver_programs_when_planning() {
    let (fx, base, ours, theirs, opts) = driver_fixture();
    let out = merge_trees(&fx.repo, &base, &ours, &theirs, &opts, DriverMode::Defer).unwrap();
    assert_eq!(out.pending_drivers, vec![bstr::BString::from("data.db")]);
    assert!(out.drivers_run.is_empty());
    assert_eq!(out.conflicts.len(), 2);
}

#[test]
fn runs_configured_driver_through_program_service() {
    let (fx, base, ours, theirs, opts) = driver_fixture();
    let state = tempfile::tempdir().unwrap();
    let host = Arc::new(DriverHost::new("resolved\n", 0));
    let ctx = context(
        Some(state.path().to_path_buf()),
        Some(Arc::clone(&host) as Arc<dyn ProcessHost>),
        &[],
    );
    let out = merge_trees(
        &fx.repo,
        &base,
        &ours,
        &theirs,
        &opts,
        DriverMode::Run(&ctx),
    )
    .unwrap();
    assert_eq!(fx.read(out.tree, "data.db").as_deref(), Some("resolved\n"));
    assert_eq!(out.conflicts.len(), 1);
    assert_eq!(out.conflicts[0].path, "plain");
    assert_eq!(
        out.drivers_run,
        vec![("custom".to_owned(), bstr::BString::from("data.db"))]
    );
    let commands = host.commands.lock().unwrap().clone();
    assert_eq!(commands.len(), 1);
    assert!(commands[0].starts_with("custom-merge '"));
    assert!(commands[0].ends_with("'data.db'"));
    assert_eq!(
        std::fs::read_dir(state.path().join("git-merge-driver"))
            .unwrap()
            .count(),
        0
    );
}

#[test]
fn driver_exit_code_other_than_zero_leaves_conflict() {
    let (fx, base, ours, theirs, opts) = driver_fixture();
    let state = tempfile::tempdir().unwrap();
    let host: Arc<dyn ProcessHost> = Arc::new(DriverHost::new("partial\n", 1));
    let ctx = context(Some(state.path().to_path_buf()), Some(host), &[]);
    let out = merge_trees(
        &fx.repo,
        &base,
        &ours,
        &theirs,
        &opts,
        DriverMode::Run(&ctx),
    )
    .unwrap();
    assert_eq!(fx.read(out.tree, "data.db").as_deref(), Some("partial\n"));
    let driven = out.conflicts.iter().find(|c| c.path == "data.db").unwrap();
    assert_eq!(driven.kind, GitConflictKind::BothModified);
    assert!(driven.markers);
}

#[test]
fn falls_back_to_text_driver_without_program_service() {
    let (fx, base, ours, theirs, opts) = driver_fixture();
    let ctx = context(None, None, &[]);
    let out = merge_trees(
        &fx.repo,
        &base,
        &ours,
        &theirs,
        &opts,
        DriverMode::Run(&ctx),
    )
    .unwrap();
    assert!(out.drivers_run.is_empty());
    assert_eq!(out.notes.len(), 1);
    assert_eq!(out.notes[0].code, Notable::FilterNotApplied);
    assert_eq!(out.notes[0].path.as_deref(), Some("data.db"));
    assert!(
        fx.read(out.tree, "data.db")
            .unwrap()
            .contains("<<<<<<< HEAD")
    );
}

#[test]
fn falls_back_to_text_driver_when_shell_cannot_start() {
    let (fx, base, ours, theirs, opts) = driver_fixture();
    let state = tempfile::tempdir().unwrap();
    let mut host = DriverHost::new("x\n", 0);
    host.fail_to_start = true;
    let ctx = context(Some(state.path().to_path_buf()), Some(Arc::new(host)), &[]);
    let out = merge_trees(
        &fx.repo,
        &base,
        &ours,
        &theirs,
        &opts,
        DriverMode::Run(&ctx),
    )
    .unwrap();
    assert_eq!(out.notes.len(), 1);
    assert!(
        fx.read(out.tree, "data.db")
            .unwrap()
            .contains("<<<<<<< HEAD")
    );
}

#[test]
fn driver_mode_formats_for_debugging() {
    let ctx = context(None, None, &[]);
    assert_eq!(format!("{:?}", DriverMode::Defer), "Defer");
    assert_eq!(format!("{:?}", DriverMode::Run(&ctx)), "Run");
}
