// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Bound golden trees of `tests/d1/golden/bound/<variant>/`.
//!
//! Every bound tree passes `validate_bound`, survives a CBOR round trip and
//! has its recorded hash. A program written in several syntax variants binds
//! to the same tree, so the files of one name in `bound/a`, `bound/b` and
//! `bound/c` have the same hash.

// A failed unwrap, expectation or panic is how a test reports a failure.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod hash_support;

use std::collections::BTreeMap;

use hash_support::{Format, Tree, d1_bound_trees, d1_golden, decode_cbor, decode_cbor_strict};
use nurt_syntax::validate::validate_bound;

/// Directories of the syntax variants of the bound golden trees.
const VARIANTS: [&str; 3] = ["a", "b", "c"];

#[test]
fn bound_golden_trees_use_the_variant_layout() {
    for tree in d1_bound_trees() {
        let variant = tree
            .path
            .parent()
            .and_then(|dir| dir.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        assert!(
            VARIANTS.contains(&variant.as_str()),
            "{}: bound trees belong to bound/a, bound/b or bound/c",
            tree.path.display()
        );
        assert!(
            tree.hash_path().is_file(),
            "{}: no .hash file",
            tree.path.display()
        );
    }
}

#[test]
fn bound_golden_trees_are_valid_and_survive_cbor() {
    for tree in d1_bound_trees() {
        let Tree::Bound(document) = tree.decode() else {
            panic!("{}: not a bound tree", tree.path.display());
        };
        validate_bound(&document)
            .unwrap_or_else(|error| panic!("{}: {error}", tree.path.display()));
        let decoded = Tree::Bound(document);
        let bytes = decoded.cbor();
        assert_eq!(decode_cbor(Format::Bound, &bytes).unwrap(), decoded);
        assert_eq!(decode_cbor_strict(Format::Bound, &bytes).unwrap(), decoded);
        assert_eq!(
            decode_cbor(Format::Bound, &bytes).unwrap().hash(),
            decoded.hash()
        );
    }
}

#[test]
fn variants_of_one_program_have_the_same_bound_hash() {
    let mut by_name: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
    for variant in VARIANTS {
        let dir = d1_golden().join("bound").join(variant);
        for tree in d1_bound_trees()
            .into_iter()
            .filter(|tree| tree.path.starts_with(&dir))
        {
            by_name
                .entry(tree.stem())
                .or_default()
                .push((variant.to_owned(), tree.decode().hash()));
        }
    }
    for (name, hashes) in by_name {
        let (first_variant, first_hash) = &hashes[0];
        for (variant, hash) in &hashes[1..] {
            assert_eq!(
                hash, first_hash,
                "{name}: the bound tree of variant {variant} differs from variant {first_variant}"
            );
        }
    }
}
