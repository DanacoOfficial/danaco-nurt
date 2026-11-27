// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Checkout time of a large tree compared with `git`.
//!
//! The test is ignored by default because it creates 100 000 files:
//! `cargo test -p nurt-git-difftest --test checkout_perf -- --ignored --nocapture`.

#![allow(
    clippy::expect_used,
    reason = "a failed setup step of the measurement aborts the test"
)]

use std::path::Path;
use std::time::{Duration, Instant};

use nurt_contracts::journal::MutationTx;
use nurt_git::apply::testjournal::RecordingJournal;
use nurt_git::apply::{ApplyContext, apply_plan, plan_two_way};
use nurt_git_difftest::fsutil::copy_tree;
use nurt_git_difftest::git::{Availability, GitEnv, GitProgram};
use nurt_git_difftest::time::ScenarioClock;

const DIRECTORIES: usize = 500;
const FILES_PER_DIRECTORY: usize = 200;
/// Largest accepted ratio of the Nurt time to the `git` time.
const MAX_RATIO: f64 = 3.0;

fn write_files(root: &Path) {
    for directory in 0..DIRECTORIES {
        let dir = root.join(format!("d{directory:03}"));
        std::fs::create_dir_all(&dir).expect("directory");
        for file in 0..FILES_PER_DIRECTORY {
            let content = format!("file {directory} {file}\n");
            std::fs::write(dir.join(format!("f{file:03}.txt")), content).expect("file");
        }
    }
}

#[test]
#[ignore = "creates 100000 files; run with --ignored"]
fn checkout_of_100000_files_takes_at_most_three_times_the_time_of_git() {
    let Availability::Ready(git) = GitProgram::locate() else {
        return;
    };
    let scratch = tempfile::tempdir().expect("temporary directory");
    let home = scratch.path().join("home");
    std::fs::create_dir_all(&home).expect("home");
    std::fs::write(home.join(".gitconfig"), "").expect("config");
    let env = GitEnv::new(&home, ScenarioClock::default());
    let git_side = scratch.path().join("git");
    std::fs::create_dir_all(&git_side).expect("repository directory");
    for args in [
        &["init", "-q", "-b", "main"][..],
        // An automatic gc that git starts in the background after the commit
        // of the files packs loose objects while the repository is copied.
        &["config", "gc.auto", "0"],
        &["config", "maintenance.auto", "false"],
        &["commit", "-q", "--allow-empty", "-m", "empty"],
        &["switch", "-q", "-c", "big"],
    ] {
        git.run_ok(&env, &git_side, args).expect("git setup");
    }
    write_files(&git_side);
    for args in [
        &["add", "-A"][..],
        &["commit", "-q", "-m", "files"],
        &["switch", "-q", "main"],
    ] {
        git.run_ok(&env, &git_side, args).expect("git setup");
    }
    let nurt_side = scratch.path().join("nurt");
    copy_tree(&git_side, &nurt_side).expect("copy");

    let started = Instant::now();
    git.run_ok(&env, &git_side, &["switch", "-q", "big"])
        .expect("git switch");
    let git_time = started.elapsed();

    let repo = gix::open_opts(&nurt_side, gix::open::Options::isolated()).expect("open");
    let head = repo.rev_parse_single("main^{tree}").expect("head").detach();
    let target = repo
        .rev_parse_single("big^{tree}")
        .expect("target")
        .detach();
    let journal = RecordingJournal::new();
    let started = Instant::now();
    let ctx = ApplyContext::open(&repo).expect("context");
    let plan = plan_two_way(&ctx, Some(&head), &target).expect("plan");
    let planned = started.elapsed();
    let mut tx = journal.begin();
    apply_plan(&mut tx, &ctx, plan).expect("apply");
    Box::new(tx).commit().expect("commit");
    let nurt_time = started.elapsed();

    let ratio = nurt_time.as_secs_f64() / git_time.as_secs_f64().max(f64::EPSILON);
    println!(
        "git: {git_time:?}, nurt: {nurt_time:?} (plan {planned:?}, apply {:?}), ratio {ratio:.2}",
        nurt_time.saturating_sub(planned)
    );
    assert!(nurt_time > Duration::ZERO);
    assert!(
        ratio <= MAX_RATIO,
        "checkout took {ratio:.2} times the time of git"
    );
}
