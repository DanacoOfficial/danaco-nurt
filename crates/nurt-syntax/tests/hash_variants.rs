// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-syntax/tests/hash_variants.rs
// Responsibility:   Equality of the surface hash of variants A and B.
// Dependencies:     hash_support
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Equality of the surface hash of variants A and B.
//!
//! Variant B writes the same surface tree as variant A, so a variant B text
//! of a program has the hash recorded in `spec/golden/ast/<name>.hash`. The
//! variant B texts are `spec/golden/ast/<name>.b.nurt` and
//! `tests/d1/golden/ast/b/<name>.nurt`. Variant C names every argument, so
//! its surface tree differs; it is compared on the bound tree (`hash_bound`).

// A failed unwrap, expectation or panic is how a test reports a failure.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod hash_support;

use std::path::{Path, PathBuf};

use hash_support::{Tree, d1_golden, files, read_text, spec_golden};
use nurt_syntax::parse::{ParseOptions, Variant, parse};

/// Number of reference tasks written in every variant.
const REFERENCE_TASKS: usize = 14;

/// Prefix of the file names of the reference tasks.
const TASK_PREFIX: &str = "task-";

/// Extension of program texts.
const TEXT_EXTENSION: &str = ".nurt";

/// Name of a program text without the extension: `task-01-x.b` for
/// `task-01-x.b.nurt`.
fn text_stem(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .and_then(|name| name.strip_suffix(TEXT_EXTENSION).map(str::to_owned))
        .unwrap_or_default()
}

/// Splits a specification text stem into the program name and the variant
/// marked by the suffix `.b` or `.c`; a stem without a suffix is variant A.
fn spec_variant(stem: &str) -> (&str, Variant) {
    if let Some(name) = stem.strip_suffix(".b") {
        (name, Variant::B)
    } else if let Some(name) = stem.strip_suffix(".c") {
        (name, Variant::C)
    } else {
        (stem, Variant::A)
    }
}

/// Recorded surface hash of the variant A tree of a program.
fn recorded_hash(name: &str) -> PathBuf {
    spec_golden().join("ast").join(format!("{name}.hash"))
}

/// Parses `path` as `variant` and compares its hash with the recorded hash
/// of `name`; returns `true` when `name` is a reference task.
fn check_text(path: &Path, variant: Variant, name: &str) -> bool {
    let recorded = recorded_hash(name);
    let document = parse(&read_text(path), variant, &ParseOptions::default())
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    assert_eq!(
        Tree::Surface(document).hash(),
        read_text(&recorded).trim_end(),
        "{}",
        path.display()
    );
    name.starts_with(TASK_PREFIX)
}

/// Specification texts of `variant` that have a recorded variant A hash, as
/// pairs of path and program name.
fn spec_texts(variant: Variant) -> Vec<(PathBuf, String)> {
    files(&spec_golden().join("ast"), TEXT_EXTENSION)
        .into_iter()
        .filter_map(|path| {
            let stem = text_stem(&path);
            let (name, found) = spec_variant(&stem);
            let name = name.to_owned();
            (found == variant && recorded_hash(&name).is_file()).then_some((path, name))
        })
        .collect()
}

#[test]
fn variant_a_texts_have_their_recorded_surface_hash() {
    let tasks = spec_texts(Variant::A)
        .iter()
        .filter(|(path, name)| check_text(path, Variant::A, name))
        .count();
    assert_eq!(tasks, REFERENCE_TASKS);
}

#[test]
fn specification_variant_b_texts_have_the_surface_hash_of_variant_a() {
    let tasks = spec_texts(Variant::B)
        .iter()
        .filter(|(path, name)| check_text(path, Variant::B, name))
        .count();
    assert_eq!(tasks, REFERENCE_TASKS);
}

#[test]
fn language_variant_b_texts_have_the_surface_hash_of_variant_a() {
    let tasks = files(&d1_golden().join("ast/b"), TEXT_EXTENSION)
        .iter()
        .filter_map(|path| {
            let name = text_stem(path);
            recorded_hash(&name)
                .is_file()
                .then(|| check_text(path, Variant::B, &name))
        })
        .filter(|task| *task)
        .count();
    assert_eq!(tasks, REFERENCE_TASKS);
}

#[test]
fn variant_suffixes_select_the_variant() {
    assert_eq!(spec_variant("task-01-x"), ("task-01-x", Variant::A));
    assert_eq!(spec_variant("task-01-x.b"), ("task-01-x", Variant::B));
    assert_eq!(spec_variant("task-01-x.c"), ("task-01-x", Variant::C));
    assert_eq!(
        text_stem(Path::new("ast/task-01-x.b.nurt")),
        "task-01-x.b".to_owned()
    );
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
