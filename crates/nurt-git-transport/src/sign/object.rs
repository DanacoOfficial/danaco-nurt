// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-git-transport::sign::object
// Responsibility:   Signatures inside commit and tag objects.
// Dependencies:     bstr, gix_object, nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Signatures inside commit and tag objects.
//!
//! A commit carries its signature in the header `gpgsig` (`gpgsig-sha256`
//! when the tree is named by a SHA-256 object ID), written after all other
//! headers; the signed payload is the commit without that header. An
//! annotated tag carries its signature after the message; the signed payload
//! is the tag object up to and including the line feed that ends the
//! message.

use bstr::BString;
use gix_object::{CommitRefIter, TagRefIter, WriteTo};
use nurt_contracts::error::{Error, ErrorCode, Result};

use super::{CommitSignature, Signature, SignatureStatus, Signer, SigningConfig, VerifyConfig};

/// Signs `commit` with `signer` and stores the signature in its header.
///
/// An existing signature header is replaced. On failure `commit` is left
/// unchanged.
///
/// # Errors
///
/// The errors of [`Signer::sign`], and `E_INVALID_VALUE` when the commit
/// cannot be serialized.
pub fn sign_commit(
    commit: &mut gix_object::Commit,
    signer: &dyn Signer,
    config: &SigningConfig,
) -> Result<Signature> {
    let field = gix_object::commit::signature_field_name(commit.tree.kind());
    let mut unsigned = commit.clone();
    unsigned.extra_headers.retain(|(name, _)| name != field);
    let payload = serialize(&unsigned, "commit")?;
    let signature = signer.sign(&payload, config)?;
    unsigned
        .extra_headers
        .push((field.into(), BString::from(signature.armored.clone())));
    *commit = unsigned;
    Ok(signature)
}

/// Signs the annotated tag `tag` with `signer` and appends the signature to
/// its message.
///
/// The message ends with exactly one line feed before the signature, as
/// Git writes it. An existing signature is replaced. On failure `tag` is
/// left unchanged.
///
/// # Errors
///
/// The errors of [`Signer::sign`], and `E_INVALID_VALUE` when the tag cannot
/// be serialized.
pub fn sign_tag(
    tag: &mut gix_object::Tag,
    signer: &dyn Signer,
    config: &SigningConfig,
) -> Result<Signature> {
    let mut unsigned = tag.clone();
    if unsigned.message.ends_with(b"\n") {
        unsigned.message.pop();
    }
    unsigned.signature = Some(BString::default());
    let payload = serialize(&unsigned, "tag")?;
    let signature = signer.sign(&payload, config)?;
    unsigned.signature = Some(BString::from(signature.armored.clone()));
    *tag = unsigned;
    Ok(signature)
}

/// Verifies the signature of the commit object `data` whose object IDs use
/// `hash_kind`.
///
/// When `config` names no time, the committer time is checked against the
/// validity period of SSH allowed signers.
///
/// # Errors
///
/// The errors of [`Signer::verify`], and `E_INVALID_VALUE` when `data` is
/// not a commit object.
///
/// # Examples
///
/// ```
/// use nurt_git_transport::sign::{verify_commit, BuiltinSigner, SignatureStatus, VerifyConfig};
///
/// let commit = b"tree 4b825dc642cb6eb9a060e54bf8d69288fbee4904\n\
///     author A <a@example.test> 1767225600 +0000\n\
///     committer A <a@example.test> 1767225600 +0000\n\nmessage\n";
/// let result = verify_commit(commit, gix_hash::Kind::Sha1, &BuiltinSigner, &VerifyConfig::default())?;
/// assert_eq!(result.status, SignatureStatus::None);
/// # Ok::<(), nurt_contracts::error::Error>(())
/// ```
pub fn verify_commit(
    data: &[u8],
    hash_kind: gix_hash::Kind,
    signer: &dyn Signer,
    config: &VerifyConfig,
) -> Result<CommitSignature> {
    let found = CommitRefIter::signature(data, hash_kind)
        .map_err(|error| malformed("commit", &error.to_string()))?;
    let Some((signature, signed)) = found else {
        return Ok(unsigned());
    };
    let mut config = config.clone();
    if config.signed_at.is_none() {
        config.signed_at = CommitRefIter::from_bytes(data, hash_kind)
            .committer()
            .ok()
            .and_then(|committer| committer.time().ok())
            .map(|time| time.seconds);
    }
    signer.verify(&signed.to_bstring(), &with_newline(&signature), &config)
}

/// Verifies the signature of the annotated tag object `data` whose object
/// IDs use `hash_kind`.
///
/// When `config` names no time, the tagger time is checked against the
/// validity period of SSH allowed signers.
///
/// # Errors
///
/// The errors of [`Signer::verify`].
///
/// # Examples
///
/// ```
/// use nurt_git_transport::sign::{verify_tag, BuiltinSigner, SignatureStatus, VerifyConfig};
///
/// let tag = b"object 4b825dc642cb6eb9a060e54bf8d69288fbee4904\ntype tree\ntag v1\n\nrelease\n";
/// let result = verify_tag(tag, gix_hash::Kind::Sha1, &BuiltinSigner, &VerifyConfig::default())?;
/// assert_eq!(result.status, SignatureStatus::None);
/// # Ok::<(), nurt_contracts::error::Error>(())
/// ```
pub fn verify_tag(
    data: &[u8],
    hash_kind: gix_hash::Kind,
    signer: &dyn Signer,
    config: &VerifyConfig,
) -> Result<CommitSignature> {
    let Some((signature, signed)) = TagRefIter::signature(data) else {
        return Ok(unsigned());
    };
    let mut config = config.clone();
    if config.signed_at.is_none() {
        config.signed_at = TagRefIter::from_bytes(data, hash_kind)
            .tagger()
            .ok()
            .flatten()
            .and_then(|tagger| tagger.time().ok())
            .map(|time| time.seconds);
    }
    signer.verify(&signed.to_bstring(), &with_newline(signature.data), &config)
}

fn serialize(object: &impl WriteTo, kind: &str) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    object.write_to(&mut out).map_err(|error| {
        Error::new(
            ErrorCode::InvalidValue,
            format!(
                "the {kind} cannot be serialized for signing: {error}; correct the {kind} fields"
            ),
        )
    })?;
    Ok(out)
}

fn malformed(kind: &str, detail: &str) -> Error {
    Error::new(
        ErrorCode::InvalidValue,
        format!("the {kind} object is malformed: {detail}; check the repository with `git.fsck`"),
    )
}

fn with_newline(signature: &[u8]) -> Vec<u8> {
    let mut armored = signature.to_vec();
    if !armored.ends_with(b"\n") {
        armored.push(b'\n');
    }
    armored
}

fn unsigned() -> CommitSignature {
    CommitSignature {
        status: SignatureStatus::None,
        format: None,
        key: None,
        signer: None,
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
