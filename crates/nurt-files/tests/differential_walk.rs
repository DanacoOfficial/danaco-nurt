// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-files/tests/differential_walk.rs
// Responsibility:   Differential tests of the walker against `fd`.
// Dependencies:     nurt_contracts, support_corpus, support_diff, support_walk
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Differential tests of the walker against `fd`.

#![cfg(all(feature = "differential", unix))]
// Test helpers unwrap results so that a failure aborts the test with the original error.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support_corpus;
mod support_diff;
mod support_fs;
#[path = "support_diff/walk.rs"]
mod support_walk;

use std::path::Path;

use nurt_contracts::services::ExcludePreset;
use nurt_files::walk::{KindFilter, WalkOptions};
use support_corpus::{Corpus, CorpusSpec};
use support_diff::{assert_same, normalize_nul_paths, normalize_path, require, run_expect};
use support_walk::walk_relative;

const SEEDS: [u64; 2] = [0xFD_0001, 0xFD_0002];
const FILES: usize = 1_500;
const DEPTH_LIMIT: u32 = 2;

/// One compared listing: Nurt options and the equivalent `fd` arguments.
struct Spec {
    label: &'static str,
    options: WalkOptions,
    fd: Vec<&'static str>,
}

fn base() -> WalkOptions {
    WalkOptions {
        min_depth: 1,
        kind: KindFilter::File,
        hidden: true,
        gitignore: false,
        exclude_presets: vec![ExcludePreset::Vcs],
        ..WalkOptions::default()
    }
}

fn specs() -> Vec<Spec> {
    let with = |change: fn(&mut WalkOptions)| {
        let mut options = base();
        change(&mut options);
        options
    };
    vec![
        Spec {
            label: "files",
            options: base(),
            fd: vec!["-t", "f", "-H", "-I"],
        },
        Spec {
            label: "directories",
            options: with(|o| o.kind = KindFilter::Directory),
            fd: vec!["-t", "d", "-H", "-I"],
        },
        Spec {
            label: "links",
            options: with(|o| o.kind = KindFilter::Link),
            fd: vec!["-t", "l", "-H", "-I"],
        },
        Spec {
            label: "visible entries only",
            options: with(|o| o.hidden = false),
            fd: vec!["-t", "f", "-I"],
        },
        Spec {
            label: "ignore files applied",
            options: with(|o| o.gitignore = true),
            fd: vec!["-t", "f", "-H"],
        },
        Spec {
            label: "depth limit",
            options: with(|o| o.max_depth = Some(DEPTH_LIMIT)),
            fd: vec!["-t", "f", "-H", "-I", "-d", "2"],
        },
        Spec {
            label: "follow links to files",
            options: with(|o| o.follow_links = true),
            fd: vec!["-t", "f", "-H", "-I", "-L"],
        },
        Spec {
            label: "follow links to directories",
            options: with(|o| {
                o.follow_links = true;
                o.kind = KindFilter::Directory;
            }),
            fd: vec!["-t", "d", "-H", "-I", "-L"],
        },
        Spec {
            label: "exclude pattern",
            options: with(|o| o.exclude = vec!["cache_*".to_owned()]),
            fd: vec!["-t", "f", "-H", "-I", "-E", "cache_*"],
        },
        Spec {
            label: "glob",
            options: with(|o| o.globs = vec!["**/*.txt".to_owned()]),
            fd: vec!["-t", "f", "-H", "-I", "-g", "*.txt"],
        },
    ]
}

fn fd_list(fd: &Path, root: &Path, spec: &Spec) -> Vec<String> {
    let mut args: Vec<&str> = vec![
        "--no-config",
        "--color",
        "never",
        "--print0",
        "--no-global-ignore-file",
        "-E",
        ".git",
    ];
    args.extend(&spec.fd);
    let output = run_expect(fd, &args, root, &[0]);
    normalize_nul_paths(&output.stdout, root).unwrap()
}

fn nurt_list(root: &Path, spec: &Spec) -> Vec<String> {
    let mut paths: Vec<String> = walk_relative(root, spec.options.clone())
        .iter()
        .map(|path| normalize_path(path, root))
        .collect();
    paths.sort();
    paths
}

#[test]
fn listings_match_fd_for_every_mapped_option() {
    let Some(fd) = require("fd") else {
        return;
    };
    for seed in SEEDS {
        let dir = tempfile::tempdir().unwrap();
        Corpus::generate(dir.path(), &CorpusSpec::new(seed, FILES)).unwrap();
        for spec in specs() {
            let reference = fd_list(&fd, dir.path(), &spec);
            let nurt = nurt_list(dir.path(), &spec);
            assert!(
                !reference.is_empty(),
                "{}: the corpus has no result",
                spec.label
            );
            assert_same(&format!("{} (seed {seed})", spec.label), &nurt, &reference);
        }
    }
}

#[test]
fn hidden_entries_are_included_by_default_unlike_fd() {
    let Some(fd) = require("fd") else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    Corpus::generate(dir.path(), &CorpusSpec::new(SEEDS[0], FILES)).unwrap();
    let default_options = WalkOptions {
        min_depth: 1,
        kind: KindFilter::File,
        exclude_presets: vec![ExcludePreset::Vcs],
        ..WalkOptions::default()
    };
    assert!(
        default_options.hidden,
        "the default of Nurt includes hidden entries"
    );
    let nurt = nurt_list(
        dir.path(),
        &Spec {
            label: "default",
            options: default_options,
            fd: Vec::new(),
        },
    );
    let visible = fd_list(
        &fd,
        dir.path(),
        &Spec {
            label: "visible",
            options: base(),
            fd: vec!["-t", "f", "-I"],
        },
    );
    let hidden = fd_list(
        &fd,
        dir.path(),
        &Spec {
            label: "all",
            options: base(),
            fd: vec!["-t", "f", "-H", "-I"],
        },
    );
    assert!(
        hidden.len() > visible.len(),
        "the corpus has no hidden file"
    );
    assert_same("default walk equals fd with hidden entries", &nurt, &hidden);
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
