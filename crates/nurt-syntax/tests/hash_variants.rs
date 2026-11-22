// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Equality of the surface hash of variants A and B.
//!
//! Variant B writes the same surface tree as variant A, so the text
//! `tests/d1/golden/ast/b/<name>.nurt` parsed as variant B has the hash
//! recorded in `spec/golden/ast/<name>.hash`. Variant C names every argument
//! and is compared on the bound tree (`hash_bound`).

mod hash_support;

use hash_support::{Tree, d1_golden, files, read_text, spec_golden};
use nurt_syntax::parse::{ParseOptions, Variant, parse};

/// Number of reference tasks written in variant B.
const REFERENCE_TASKS: usize = 14;

#[test]
fn variant_b_texts_have_the_surface_hash_of_variant_a() {
    let texts = files(&d1_golden().join("ast/b"), ".nurt");
    let mut tasks = 0;
    for path in texts {
        let name = path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default();
        let recorded = spec_golden().join("ast").join(format!("{name}.hash"));
        if !recorded.is_file() {
            continue;
        }
        let document = parse(&read_text(&path), Variant::B, &ParseOptions::default())
            .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        assert_eq!(
            Tree::Surface(document).hash(),
            read_text(&recorded).trim_end(),
            "{}",
            path.display()
        );
        if name.starts_with("task-") {
            tasks += 1;
        }
    }
    assert_eq!(tasks, REFERENCE_TASKS);
}

#[test]
fn variant_a_texts_have_their_recorded_surface_hash() {
    let mut tasks = 0;
    for path in files(&spec_golden().join("ast"), ".nurt") {
        let recorded = path.with_extension("hash");
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        if !name.starts_with("task-") || !recorded.is_file() {
            continue;
        }
        let document = parse(&read_text(&path), Variant::A, &ParseOptions::default())
            .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        assert_eq!(
            Tree::Surface(document).hash(),
            read_text(&recorded).trim_end(),
            "{}",
            path.display()
        );
        tasks += 1;
    }
    assert_eq!(tasks, REFERENCE_TASKS);
}
