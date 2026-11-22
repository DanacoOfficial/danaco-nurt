// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Walker behaviour on the real file system, in temporary directories.

// Tests report failures by panicking.
#![allow(clippy::expect_used)]

mod support_fs;

use std::fs;
use std::ops::ControlFlow;
use std::path::Path;
use std::time::Instant;

use nurt_contracts::fs::AbsPath;
use nurt_contracts::services::ExcludePreset;
use nurt_files::walk::{KindFilter, WalkItem, WalkOptions, WalkSummary, Walker};
use support_fs::DiskFs;

/// Number of files of the local performance test.
const PERF_FILES: usize = 100_000;

/// Files per directory of the local performance test.
const PERF_FILES_PER_DIR: usize = 100;

fn write(root: &Path, relative: &str, content: &str) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().expect("path has a parent")).expect("directory is created");
    fs::write(path, content).expect("file is written");
}

fn walk(root: &Path, options: WalkOptions) -> (Vec<String>, WalkSummary) {
    let walker = Walker::new(options).expect("options are valid");
    let root = AbsPath::new(root).expect("temporary directory is absolute");
    let mut entries = Vec::new();
    let summary = walker
        .run(&DiskFs, std::slice::from_ref(&root), &|| Ok(()), |item| {
            if let WalkItem::Entry(entry) = item {
                entries.push(entry.relative_string());
            }
            ControlFlow::Continue(())
        })
        .expect("walk succeeds");
    (entries, summary)
}

fn all() -> WalkOptions {
    WalkOptions {
        min_depth: 1,
        kind: KindFilter::Any,
        ..WalkOptions::default()
    }
}

#[test]
fn lists_a_real_tree_in_name_order() {
    let temp = tempfile::tempdir().expect("temporary directory");
    for relative in ["b/2.txt", "b/1.txt", "a.txt", "c/d/e.txt"] {
        write(temp.path(), relative, "x");
    }
    let (entries, summary) = walk(temp.path(), all());
    assert_eq!(
        entries,
        ["a.txt", "b", "b/1.txt", "b/2.txt", "c", "c/d", "c/d/e.txt"]
    );
    assert!(summary.complete);
}

#[test]
fn applies_gitignore_of_a_real_repository() {
    let temp = tempfile::tempdir().expect("temporary directory");
    fs::create_dir_all(temp.path().join(".git/info")).expect("repository directory");
    write(temp.path(), ".git/info/exclude", "secret.txt\n");
    write(temp.path(), ".gitignore", "target/\n*.tmp\n");
    write(temp.path(), "src/lib.rs", "");
    write(temp.path(), "src/scratch.tmp", "");
    write(temp.path(), "target/debug/app", "");
    write(temp.path(), "secret.txt", "");
    let options = WalkOptions {
        gitignore: true,
        exclude_presets: vec![ExcludePreset::Vcs],
        kind: KindFilter::File,
        ..all()
    };
    let (entries, _) = walk(temp.path(), options);
    assert_eq!(entries, [".gitignore", "src/lib.rs"]);
}

#[cfg(unix)]
#[test]
fn detects_symbolic_link_loops_on_a_real_tree() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().expect("temporary directory");
    write(temp.path(), "a/file.txt", "");
    symlink("..", temp.path().join("a/parent")).expect("link is created");
    symlink("missing", temp.path().join("a/dangling")).expect("link is created");
    let options = WalkOptions {
        follow_links: true,
        ..all()
    };
    let (entries, summary) = walk(temp.path(), options);
    assert_eq!(entries, ["a", "a/dangling", "a/file.txt", "a/parent"]);
    assert_eq!(summary.link_loops, 1);
}

#[test]
fn entries_carry_metadata_from_the_disk() {
    let temp = tempfile::tempdir().expect("temporary directory");
    write(temp.path(), "five.txt", "12345");
    let walker = Walker::new(WalkOptions {
        metadata: true,
        ..all()
    })
    .expect("options are valid");
    let root = AbsPath::new(temp.path()).expect("temporary directory is absolute");
    let mut sizes = Vec::new();
    walker
        .run(&DiskFs, &[root], &|| Ok(()), |item| {
            if let WalkItem::Entry(entry) = item {
                sizes.push(entry.meta.map(|meta| meta.size));
            }
            ControlFlow::Continue(())
        })
        .expect("walk succeeds");
    assert_eq!(sizes, [Some(5)]);
}

/// Measures a walk of [`PERF_FILES`] files; run with `--ignored`. The final
/// comparison with other tools is made by the performance suite.
#[test]
#[ignore = "local performance measurement"]
fn walks_one_hundred_thousand_files() {
    let temp = tempfile::tempdir().expect("temporary directory");
    for index in 0..PERF_FILES {
        let dir = temp
            .path()
            .join(format!("d{:04}", index / PERF_FILES_PER_DIR));
        if index % PERF_FILES_PER_DIR == 0 {
            fs::create_dir(&dir).expect("directory is created");
        }
        fs::write(dir.join(format!("f{index}.txt")), "").expect("file is written");
    }
    let started = Instant::now();
    let (entries, _) = walk(
        temp.path(),
        WalkOptions {
            min_depth: 1,
            ..WalkOptions::default()
        },
    );
    let elapsed = started.elapsed();
    assert_eq!(entries.len(), PERF_FILES);
    assert!(elapsed.as_secs() < 30, "walk took {elapsed:?}");
}
