// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-syntax::hash
// Responsibility:   Tree hash `ast_hash`.
// Dependencies:     nurt_contracts, serde, sha2
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Tree hash `ast_hash`.
//!
//! ```text
//! ast_hash = "sha256:" + lowercase_hex(SHA-256(UTF-8(JCS(hash_form(document)))))
//! ```
//!
//! The hash is computed from the JSON produced by serializing a typed tree,
//! never from received bytes, so omitted optional members, members written
//! with their default value (`"fields": []`, `"waits": false`,
//! `"default": false`) and the order of members never change it. The hash
//! form removes the root member `meta` and the member `span` of every object
//! (`nurt_contracts::ast::strip_for_hash`). JCS is the JSON Canonicalization
//! Scheme of RFC 8785.

// `SyntaxError` is the error type shared by every module of the crate; its
// layout is defined with the parser diagnostics, not per module.
#![allow(clippy::result_large_err)]

use nurt_contracts::ast::{AST_HASH_PREFIX, AstDocument, strip_for_hash};
use nurt_contracts::bound::BoundDocument;
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::error::SyntaxError;
use crate::validate::violation;

mod sealed {
    pub trait Sealed {}

    impl Sealed for nurt_contracts::ast::AstDocument {}

    impl Sealed for nurt_contracts::bound::BoundDocument {}
}

/// A tree document whose hash is defined: [`AstDocument`] or
/// [`BoundDocument`].
pub trait TreeDocument: Serialize + sealed::Sealed {}

impl TreeDocument for AstDocument {}

impl TreeDocument for BoundDocument {}

/// Returns the canonical JSON text whose SHA-256 digest is the tree hash.
///
/// The document is expected to be valid. Every decoder of the crate validates
/// the documents it returns with [`crate::validate::validate_surface`] or
/// [`crate::validate::validate_bound`].
///
/// # Errors
///
/// Returns `E_AST_SCHEMA` when the document cannot be serialized as JSON.
///
/// # Examples
///
/// ```
/// use nurt_contracts::ast::AstDocument;
/// use nurt_syntax::hash::hash_input;
///
/// let document: AstDocument = serde_json::from_str(
///     r#"{"program":{"statements":[{"t":"pipeline","stages":[],
///         "head":{"t":"stage","name":"count","args":[],"span":[1,1,1,6]}}]},"nurt":"1.0"}"#,
/// )?;
/// assert_eq!(
///     hash_input(&document).ok().as_deref(),
///     Some(r#"{"nurt":"1.0","program":{"statements":[{"head":{"args":[],"name":"count","t":"stage"},"stages":[],"t":"pipeline"}]}}"#),
/// );
/// # Ok::<(), serde_json::Error>(())
/// ```
pub fn hash_input<T: TreeDocument>(document: &T) -> Result<String, SyntaxError> {
    serde_json::to_value(document)
        .and_then(|value| serde_json_canonicalizer::to_string(&strip_for_hash(&value)))
        .map_err(|error| violation("", format!("the tree cannot be serialized: {error}")))
}

/// Computes the tree hash, `sha256:` followed by 64 lowercase hex digits.
///
/// # Errors
///
/// Returns `E_AST_SCHEMA` when the document cannot be serialized as JSON.
///
/// # Examples
///
/// ```
/// use nurt_contracts::ast::AstDocument;
/// use nurt_syntax::hash::ast_hash;
///
/// let plain: AstDocument = serde_json::from_str(
///     r#"{"nurt":"1.0","program":{"statements":[{"t":"pipeline",
///         "head":{"t":"stage","name":"count","args":[]},"stages":[]}]}}"#,
/// )?;
/// let with_layout: AstDocument = serde_json::from_str(
///     r#"{"nurt":"1.0","meta":{"source":"count"},"program":{"statements":[{"t":"pipeline",
///         "head":{"t":"stage","name":"count","args":[],"span":[1,1,1,6]},"stages":[],
///         "span":[1,1,1,6]}]}}"#,
/// )?;
/// let hash = ast_hash(&plain).ok();
/// assert!(hash.as_deref().is_some_and(|hash| hash.starts_with("sha256:")));
/// assert_eq!(hash, ast_hash(&with_layout).ok());
/// # Ok::<(), serde_json::Error>(())
/// ```
pub fn ast_hash<T: TreeDocument>(document: &T) -> Result<String, SyntaxError> {
    let input = hash_input(document)?;
    let digest = Sha256::digest(input.as_bytes());
    Ok(format!("{AST_HASH_PREFIX}{}", hex::encode(digest)))
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
