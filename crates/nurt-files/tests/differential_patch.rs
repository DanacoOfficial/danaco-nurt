// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Differential tests of `nurt_text::patch` against `patch -p1`.

#![cfg(all(feature = "differential", unix))]
// Test helpers unwrap results so that a failure aborts the test with the original error.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support_corpus;
mod support_diff;

use std::fs;
use std::path::{Path, PathBuf};

use nurt_text::patch::{ApplyOptions, FileChange, Patch, plan};
use support_corpus::Rng;
use support_diff::{require, run_expect};

const SEEDS: u64 = 40;
const MIN_LINES: u64 = 8;
const LINE_SPREAD: u64 = 40;
const EDIT_ODDS: u64 = 24;
const STRIP: usize = 1;
const TARGET: &str = "f.txt";
const PATCH_FILE: &str = "change.diff";
const WORDS: [&str; 6] = ["alpha", "bravo", "report", "index", "stream", "buffer"];

fn text_pair(seed: u64) -> (String, String) {
    let mut rng = Rng::new(seed);
    let count = MIN_LINES + rng.below(LINE_SPREAD);
    let mut old = String::new();
    let mut new = String::new();
    for index in 0..count {
        let word = WORDS[rng.below(WORDS.len() as u64) as usize];
        let line = format!("line {index} {word}\n");
        old.push_str(&line);
        match rng.below(EDIT_ODDS) {
            0 | 1 if index % 2 == 0 => {}
            2..=4 => new.push_str(&format!("changed {index} {word}\n")),
            5 | 6 => {
                new.push_str(&line);
                new.push_str(&format!("added {index}\n"));
            }
            _ => new.push_str(&line),
        }
    }
    (old, new)
}

/// Directory with `a/` and `b/` holding the two sides, and the patch of `diff -u`.
struct Fixture {
    dir: tempfile::TempDir,
    patch: String,
}

fn fixture(diff: &Path, files: &[(&str, Option<&str>, Option<&str>)]) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    for (name, old, new) in files {
        for (side, content) in [("a", old), ("b", new)] {
            if let Some(content) = content {
                let path = dir.path().join(side).join(name);
                fs::create_dir_all(path.parent().unwrap()).unwrap();
                fs::write(path, content).unwrap();
            }
        }
    }
    fs::create_dir_all(dir.path().join("a")).unwrap();
    fs::create_dir_all(dir.path().join("b")).unwrap();
    let output = run_expect(diff, &["-urN", "a", "b"], dir.path(), &[0, 1]);
    Fixture {
        dir,
        patch: String::from_utf8(output.stdout).unwrap(),
    }
}

/// Result of `patch` in a directory holding `files`; `None` for a removed file.
fn reference_apply(
    patch: &Path,
    text: &str,
    files: &[(&str, &str)],
    reverse: bool,
) -> Vec<(String, Option<String>)> {
    let work = tempfile::tempdir().unwrap();
    for (name, content) in files {
        let path = work.path().join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }
    fs::write(work.path().join(PATCH_FILE), text).unwrap();
    let mut args = vec![
        "-p1",
        "--batch",
        "--no-backup-if-mismatch",
        "-i",
        PATCH_FILE,
    ];
    if reverse {
        args.push("-R");
    }
    run_expect(patch, &args, work.path(), &[0]);
    let mut names: Vec<&str> = files.iter().map(|(name, _)| *name).collect();
    let patched = Patch::parse(text).unwrap().paths(STRIP).unwrap();
    names.extend(patched.iter().map(String::as_str));
    names.sort_unstable();
    names.dedup();
    names
        .into_iter()
        .map(|name| {
            let path: PathBuf = work.path().join(name);
            (name.to_owned(), fs::read_to_string(path).ok())
        })
        .collect()
}

fn nurt_apply(text: &str, files: &[(&str, &str)], reverse: bool) -> Vec<(String, Option<String>)> {
    let patch = Patch::parse(text).unwrap();
    let options = ApplyOptions {
        strip: STRIP,
        reverse,
        ..ApplyOptions::default()
    };
    let read = |path: &str| {
        files
            .iter()
            .find(|(name, _)| *name == path)
            .map(|(_, content)| (*content).to_owned())
    };
    let mut state: Vec<(String, Option<String>)> = files
        .iter()
        .map(|(name, content)| ((*name).to_owned(), Some((*content).to_owned())))
        .collect();
    let mut set = |name: &str, content: Option<String>| match state
        .iter_mut()
        .find(|(existing, _)| existing == name)
    {
        Some(slot) => slot.1 = content,
        None => state.push((name.to_owned(), content)),
    };
    for planned in plan(&patch, &options, read).unwrap() {
        match planned.change {
            FileChange::Create { path, content, .. } | FileChange::Modify { path, content, .. } => {
                set(&path, Some(content));
            }
            FileChange::Delete { path } => set(&path, None),
            FileChange::Rename { from, to, .. } => {
                let moved = files
                    .iter()
                    .find(|(name, _)| *name == from)
                    .map(|(_, content)| (*content).to_owned());
                set(&from, None);
                set(&to, moved);
            }
        }
    }
    state.sort();
    state
}

fn reference_sorted(mut state: Vec<(String, Option<String>)>) -> Vec<(String, Option<String>)> {
    state.sort();
    state
}

#[test]
fn modification_matches_patch_for_random_edits() {
    let (Some(diff), Some(patch)) = (require("diff"), require("patch")) else {
        return;
    };
    for seed in 0..SEEDS {
        let (old, new) = text_pair(seed);
        let fixture = fixture(&diff, &[(TARGET, Some(&old), Some(&new))]);
        let files = [(TARGET, old.as_str())];
        let reference = reference_sorted(reference_apply(&patch, &fixture.patch, &files, false));
        let nurt = nurt_apply(&fixture.patch, &files, false);
        assert_eq!(nurt, reference, "seed {seed}\n{}", fixture.patch);
        assert_eq!(nurt[0].1.as_deref(), Some(new.as_str()), "seed {seed}");
        drop(fixture.dir);
    }
}

#[test]
fn reverse_application_matches_patch_dash_r() {
    let (Some(diff), Some(patch)) = (require("diff"), require("patch")) else {
        return;
    };
    for seed in 0..SEEDS {
        let (old, new) = text_pair(seed);
        let fixture = fixture(&diff, &[(TARGET, Some(&old), Some(&new))]);
        let files = [(TARGET, new.as_str())];
        let reference = reference_sorted(reference_apply(&patch, &fixture.patch, &files, true));
        let nurt = nurt_apply(&fixture.patch, &files, true);
        assert_eq!(nurt, reference, "seed {seed}");
        assert_eq!(nurt[0].1.as_deref(), Some(old.as_str()), "seed {seed}");
    }
}

#[test]
fn shifted_target_is_patched_at_the_found_offset() {
    let (Some(diff), Some(patch)) = (require("diff"), require("patch")) else {
        return;
    };
    let mut rejected = Vec::new();
    for seed in 0..SEEDS {
        // Unchanged leading lines keep every hunk away from the start of the file,
        // where a hunk is anchored to the first line.
        let head = "head 0\nhead 1\nhead 2\nhead 3\n";
        let (old, new) = text_pair(seed);
        let (old, new) = (format!("{head}{old}"), format!("{head}{new}"));
        let fixture = fixture(&diff, &[(TARGET, Some(&old), Some(&new))]);
        let shifted = format!("first extra line\nsecond extra line\n{old}");
        let files = [(TARGET, shifted.as_str())];
        let reference = reference_sorted(reference_apply(&patch, &fixture.patch, &files, false));
        let parsed = Patch::parse(&fixture.patch).unwrap();
        let options = ApplyOptions {
            strip: STRIP,
            ..ApplyOptions::default()
        };
        if plan(&parsed, &options, |_| Some(shifted.clone())).is_err() {
            rejected.push(seed);
            continue;
        }
        let nurt = nurt_apply(&fixture.patch, &files, false);
        assert_eq!(nurt, reference, "seed {seed}");
    }
    assert!(
        rejected.is_empty(),
        "Nurt rejected shifted targets that patch accepts, seeds {rejected:?}"
    );
}

#[test]
fn creation_and_deletion_match_patch() {
    let (Some(diff), Some(patch)) = (require("diff"), require("patch")) else {
        return;
    };
    let fixture = fixture(
        &diff,
        &[
            ("new/created.txt", None, Some("first\nsecond\n")),
            ("gone.txt", Some("one\ntwo\n"), None),
            ("kept.txt", Some("x\ny\nz\n"), Some("x\nY\nz\n")),
        ],
    );
    let files = [("gone.txt", "one\ntwo\n"), ("kept.txt", "x\ny\nz\n")];
    let mut reference = reference_sorted(reference_apply(&patch, &fixture.patch, &files, false));
    let mut nurt = nurt_apply(&fixture.patch, &files, false);
    // GNU patch removes a file that a timestamp-style patch empties, with an epoch time
    // stamp on the new side; Nurt leaves the empty file (listed in the intended differences).
    assert_eq!(reference[0], ("gone.txt".to_owned(), None));
    assert_eq!(nurt[0], ("gone.txt".to_owned(), Some(String::new())));
    reference.remove(0);
    nurt.remove(0);
    assert_eq!(nurt, reference, "{}", fixture.patch);
}

#[test]
fn rejected_hunk_fails_in_both_programs() {
    let (Some(diff), Some(patch)) = (require("diff"), require("patch")) else {
        return;
    };
    let fixture = fixture(&diff, &[(TARGET, Some("a\nb\nc\n"), Some("a\nB\nc\n"))]);
    let unrelated = "p\nq\nr\n";
    let work = tempfile::tempdir().unwrap();
    fs::write(work.path().join(TARGET), unrelated).unwrap();
    fs::write(work.path().join(PATCH_FILE), &fixture.patch).unwrap();
    let output = support_diff::run(
        &patch,
        &[
            "-p1",
            "--batch",
            "--no-backup-if-mismatch",
            "-i",
            PATCH_FILE,
        ],
        work.path(),
        support_diff::DEFAULT_TIMEOUT,
    )
    .unwrap();
    assert_ne!(
        output.code,
        Some(0),
        "patch applied a hunk that does not match"
    );
    let parsed = Patch::parse(&fixture.patch).unwrap();
    let options = ApplyOptions {
        strip: STRIP,
        ..ApplyOptions::default()
    };
    let result = plan(&parsed, &options, |_| Some(unrelated.to_owned()));
    assert!(result.is_err(), "Nurt applied a hunk that does not match");
}
