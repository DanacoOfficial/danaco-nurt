// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-syntax/tests/hash_support/mod.rs
// Responsibility:   Helpers shared by the golden-file tests of the tree codecs and the tree.
// Dependencies:     nurt_contracts, serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Helpers shared by the golden-file tests of the tree codecs and the tree
//! hash.
//!
//! Golden trees are the `*.json` files under `spec/golden/{ast,bound}` and
//! `tests/d1/golden/{ast,bound}`; the recorded hash of `<name>.json` is the
//! file `<name>.hash`. Setting `NURT_BLESS=1` makes the tests write the
//! recorded hashes and the hash index of `tests/d1/golden` instead of
//! comparing them; the files under `spec/` are never written.

// Every test crate includes this module and uses a subset of the helpers.
#![allow(dead_code)]
// A failed unwrap, expectation or panic is how a test reports a failure.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::path::{Path, PathBuf};

use nurt_contracts::ast::{AST_FORMAT, AstDocument};
use nurt_contracts::bound::{BOUND_FORMAT, BoundDocument};
use nurt_syntax::error::SyntaxError;
use nurt_syntax::hash::ast_hash;
use nurt_syntax::json;
use nurt_syntax::validate::{validate_bound, validate_surface};
use serde_json::Value;

/// Environment variable that switches the golden tests to regeneration.
pub(crate) const BLESS_VARIABLE: &str = "NURT_BLESS";

/// File name of the generated schema kept next to the golden trees.
pub(crate) const GENERATED_SCHEMA: &str = "schema.generated.json";

/// File name of the hash index of `tests/d1/golden`.
pub(crate) const HASH_INDEX: &str = "hashes.tsv";

/// Header line of the hash index.
pub(crate) const HASH_INDEX_HEADER: &str = "file\tformat\tast_hash";

/// Returns `true` when the golden files are to be regenerated.
pub(crate) fn bless() -> bool {
    std::env::var(BLESS_VARIABLE).is_ok_and(|value| value == "1")
}

/// Root of the workspace.
pub(crate) fn workspace() -> PathBuf {
    Path::new(
        std::env::var("CARGO_MANIFEST_DIR")
            .as_deref()
            .unwrap_or(env!("CARGO_MANIFEST_DIR")),
    )
    .join("../..")
}

/// Conformance files of the specification.
pub(crate) fn spec_golden() -> PathBuf {
    workspace().join("spec/golden")
}

/// Golden files of the language crates.
pub(crate) fn d1_golden() -> PathBuf {
    workspace().join("tests/d1/golden")
}

/// Published schemas.
pub(crate) fn spec_schemas() -> PathBuf {
    workspace().join("spec/schemas")
}

/// Kind of a tree document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Format {
    /// Surface tree, `nurt-ast/1`.
    Surface,
    /// Bound tree, `nurt-bound/1`.
    Bound,
}

impl Format {
    /// Format name written into the hash index.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Surface => AST_FORMAT,
            Self::Bound => BOUND_FORMAT,
        }
    }
}

/// A decoded golden tree.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Tree {
    /// Surface tree.
    Surface(AstDocument),
    /// Bound tree.
    Bound(BoundDocument),
}

impl Tree {
    /// Tree hash of the document.
    pub(crate) fn hash(&self) -> String {
        match self {
            Self::Surface(document) => ast_hash(document),
            Self::Bound(document) => ast_hash(document),
        }
        .unwrap_or_else(|error| panic!("hash: {error}"))
    }

    /// Deterministic CBOR encoding of the document.
    pub(crate) fn cbor(&self) -> Vec<u8> {
        match self {
            Self::Surface(document) => nurt_syntax::cbor::encode(document),
            Self::Bound(document) => nurt_syntax::cbor::encode(document),
        }
        .unwrap_or_else(|error| panic!("encode: {error}"))
    }

    /// JSON value of the document.
    pub(crate) fn to_json(&self) -> Value {
        match self {
            Self::Surface(document) => serde_json::to_value(document),
            Self::Bound(document) => serde_json::to_value(document),
        }
        .unwrap_or_else(|error| panic!("serialize: {error}"))
    }
}

/// A golden tree file.
#[derive(Debug, Clone)]
pub(crate) struct GoldenFile {
    /// Path of the `.json` file.
    pub(crate) path: PathBuf,
    /// Kind of the document.
    pub(crate) format: Format,
}

impl GoldenFile {
    /// Path of the recorded hash.
    pub(crate) fn hash_path(&self) -> PathBuf {
        self.path.with_extension("hash")
    }

    /// File name without the extension `.json`.
    pub(crate) fn stem(&self) -> String {
        self.path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    /// JSON text of the file.
    pub(crate) fn text(&self) -> String {
        read_text(&self.path)
    }

    /// JSON value of the file.
    pub(crate) fn json(&self) -> Value {
        serde_json::from_str(&self.text())
            .unwrap_or_else(|error| panic!("{}: {error}", self.path.display()))
    }

    /// Decodes the file with the JSON decoder of the crate.
    pub(crate) fn decode(&self) -> Tree {
        decode_json(self.format, &self.text())
            .unwrap_or_else(|error| panic!("{}: {error}", self.path.display()))
    }
}

/// Checks the invariants of a decoded tree.
pub(crate) fn validated(tree: Tree) -> Result<Tree, SyntaxError> {
    match &tree {
        Tree::Surface(document) => validate_surface(document)?,
        Tree::Bound(document) => validate_bound(document)?,
    }
    Ok(tree)
}

/// Decodes JSON text as a tree of the given kind and checks its invariants.
pub(crate) fn decode_json(format: Format, text: &str) -> Result<Tree, SyntaxError> {
    validated(match format {
        Format::Surface => json::decode_surface(text).map(Tree::Surface)?,
        Format::Bound => json::decode_bound(text).map(Tree::Bound)?,
    })
}

/// Decodes CBOR as a tree of the given kind and checks its invariants.
pub(crate) fn decode_cbor(format: Format, bytes: &[u8]) -> Result<Tree, SyntaxError> {
    validated(match format {
        Format::Surface => nurt_syntax::cbor::decode_surface(bytes).map(Tree::Surface)?,
        Format::Bound => nurt_syntax::cbor::decode_bound(bytes).map(Tree::Bound)?,
    })
}

/// Decodes CBOR as a tree of the given kind, requires the deterministic
/// encoding and checks the invariants.
pub(crate) fn decode_cbor_strict(format: Format, bytes: &[u8]) -> Result<Tree, SyntaxError> {
    validated(match format {
        Format::Surface => nurt_syntax::cbor::decode_surface_strict(bytes).map(Tree::Surface)?,
        Format::Bound => nurt_syntax::cbor::decode_bound_strict(bytes).map(Tree::Bound)?,
    })
}

/// Reads a UTF-8 file.
pub(crate) fn read_text(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

/// Returns the files of a directory whose names end with `suffix`, sorted by
/// name; a missing directory has no files.
pub(crate) fn files(dir: &Path, suffix: &str) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found: Vec<PathBuf> = entries
        .map(|entry| {
            entry
                .unwrap_or_else(|error| panic!("{}: {error}", dir.display()))
                .path()
        })
        .filter(|path| {
            path.is_file()
                && path
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().ends_with(suffix))
        })
        .collect();
    found.sort();
    found
}

fn trees_in(dir: &Path, format: Format) -> Vec<GoldenFile> {
    files(dir, ".json")
        .into_iter()
        .filter(|path| {
            path.file_name()
                .is_some_and(|name| name != GENERATED_SCHEMA)
        })
        .map(|path| GoldenFile { path, format })
        .collect()
}

/// Subdirectories of a directory, sorted; a missing directory has none.
pub(crate) fn subdirectories(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found: Vec<PathBuf> = entries
        .map(|entry| {
            entry
                .unwrap_or_else(|error| panic!("{}: {error}", dir.display()))
                .path()
        })
        .filter(|path| path.is_dir())
        .collect();
    found.sort();
    found
}

/// Golden trees of the specification.
pub(crate) fn spec_trees() -> Vec<GoldenFile> {
    let mut trees = trees_in(&spec_golden().join("ast"), Format::Surface);
    trees.extend(trees_in(&spec_golden().join("bound"), Format::Bound));
    trees
}

/// Golden trees of `tests/d1/golden`: surface trees under `ast/<variant>/`
/// and bound trees under `bound/<variant>/`.
pub(crate) fn d1_trees() -> Vec<GoldenFile> {
    let mut trees = Vec::new();
    for (dir, format) in [("ast", Format::Surface), ("bound", Format::Bound)] {
        for variant in subdirectories(&d1_golden().join(dir)) {
            trees.extend(trees_in(&variant, format));
        }
    }
    trees
}

/// Bound golden trees of `tests/d1/golden/bound/<variant>/`.
pub(crate) fn d1_bound_trees() -> Vec<GoldenFile> {
    d1_trees()
        .into_iter()
        .filter(|tree| tree.format == Format::Bound)
        .collect()
}

/// Every golden tree.
pub(crate) fn all_trees() -> Vec<GoldenFile> {
    let mut trees = spec_trees();
    trees.extend(d1_trees());
    trees
}

/// Path relative to `tests/d1/golden`, with `/` separators.
pub(crate) fn d1_relative(path: &Path) -> String {
    path.strip_prefix(d1_golden())
        .unwrap_or(path)
        .components()
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

/// Returns the JSON nesting depth of a value; the root is at depth 1 and every
/// member or element is one level deeper than its container.
pub(crate) fn json_depth(value: &Value) -> usize {
    let mut deepest = 0;
    let mut pending = vec![(value, 1_usize)];
    while let Some((current, level)) = pending.pop() {
        deepest = deepest.max(level);
        match current {
            Value::Object(members) => pending.extend(members.values().map(|v| (v, level + 1))),
            Value::Array(items) => pending.extend(items.iter().map(|v| (v, level + 1))),
            _ => {}
        }
    }
    deepest
}

/// Returns a copy of a JSON value without the member `span` of every object
/// and without the root member `meta`.
pub(crate) fn without_layout(value: &Value) -> Value {
    let mut copy = value.clone();
    if let Value::Object(root) = &mut copy {
        root.remove("meta");
    }
    let mut pending = vec![&mut copy];
    while let Some(current) = pending.pop() {
        match current {
            Value::Object(members) => {
                members.remove("span");
                pending.extend(members.values_mut());
            }
            Value::Array(items) => pending.extend(items.iter_mut()),
            _ => {}
        }
    }
    copy
}

/// Deterministic pseudo-random numbers (SplitMix64) for reproducible
/// mutation tests.
#[derive(Debug, Clone)]
pub(crate) struct SplitMix(u64);

impl SplitMix {
    /// Generator with the given seed.
    pub(crate) fn new(seed: u64) -> Self {
        Self(seed)
    }

    /// Next 64 random bits.
    pub(crate) fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// Uniform number below `bound`; `bound` must not be zero.
    pub(crate) fn below(&mut self, bound: usize) -> usize {
        usize::try_from(self.next_u64() % u64::try_from(bound).unwrap()).unwrap()
    }

    /// Random byte.
    pub(crate) fn byte(&mut self) -> u8 {
        self.next_u64().to_le_bytes()[0]
    }
}

/// Applies one random mutation to a byte string: a bit flip, a byte
/// replacement, an insertion, a deletion, a truncation or the duplication of
/// a slice.
pub(crate) fn mutate(rng: &mut SplitMix, input: &[u8]) -> Vec<u8> {
    let mut bytes = input.to_vec();
    if bytes.is_empty() {
        bytes.push(rng.byte());
        return bytes;
    }
    let position = rng.below(bytes.len());
    match rng.below(6) {
        0 => bytes[position] ^= 1 << rng.below(8),
        1 => bytes[position] = rng.byte(),
        2 => bytes.insert(position, rng.byte()),
        3 => {
            bytes.remove(position);
        }
        4 => bytes.truncate(position),
        _ => {
            let end = (position + 1 + rng.below(16)).min(bytes.len());
            let slice = bytes[position..end].to_vec();
            let at = rng.below(bytes.len() + 1);
            bytes.splice(at..at, slice);
        }
    }
    bytes
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
