// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Manifest hashes and the hash chain.
//!
//! # Manifest digest
//!
//! The digest of a manifest (`integrity.manifest_sha256`) is the SHA-256 of
//! the canonical form of the manifest, computed as follows:
//!
//! 1. The manifest is converted to a JSON value.
//! 2. The member `manifest_sha256` of the top-level member `integrity` is
//!    removed, when present. Every other member, including `integrity.algo`
//!    and `integrity.chain`, is covered.
//! 3. Every integer whose magnitude exceeds 2^53 is replaced by a string with
//!    its decimal digits. The canonical form represents numbers as IEEE 754
//!    doubles and would otherwise lose the low digits of nanosecond
//!    timestamps, file identities and integer literals of the recorded
//!    program.
//! 4. The value is serialized by the JSON Canonicalization Scheme (RFC 8785)
//!    and the resulting bytes are hashed.
//!
//! A manifest whose recorded digest differs from the computed one is corrupt.
//!
//! # Chain
//!
//! The chain link of an entry (`integrity.chain`) is the digest of the
//! manifest of the entry that precedes it in the account index; the first
//! entry of the account has no link. A crash between the commit of an entry
//! and the append of its index row lets a later entry take the same
//! predecessor; the index then forks, which [`verify_chain`] reports without
//! treating either entry as corrupt.

use std::collections::HashMap;

use nurt_contracts::fs::{ContentHash, Fs, FsErrorKind, FsResult};
use nurt_contracts::journal::{EntryState, OpId};
use serde::Serialize;

use crate::layout::EntryLayout;
use crate::manifest::{self, ReadError, StateRecord};
use serde_json::Value;
use sha2::{Digest, Sha256};

/// Top-level manifest member holding the integrity fields.
const INTEGRITY: &str = "integrity";
/// Member of [`INTEGRITY`] holding the manifest digest.
const MANIFEST_SHA256: &str = "manifest_sha256";
/// Member of [`INTEGRITY`] naming the digest algorithm.
const DIGEST_METHOD: &str = "algo";
/// The only digest algorithm of manifest format version 1.
const DIGEST_SHA256: &str = "sha256";
/// Largest integer magnitude an IEEE 754 double represents exactly (2^53).
const EXACT_DOUBLE_LIMIT: u64 = 1 << 53;

/// Computes the digest of a manifest by the rule given in the module
/// documentation.
///
/// The recorded digest, if any, does not influence the result, so the same
/// function serves for writing and for verification.
///
/// # Errors
///
/// Returns the serialization error when the manifest cannot be represented
/// as JSON (for example a map with non-string keys).
pub(crate) fn manifest_sha256<T: Serialize + ?Sized>(
    manifest: &T,
) -> Result<ContentHash, serde_json::Error> {
    let mut value = serde_json::to_value(manifest)?;
    if let Some(Value::Object(integrity)) = value.get_mut(INTEGRITY) {
        integrity.remove(MANIFEST_SHA256);
    }
    widen_large_integers(&mut value);
    let canonical = serde_json_canonicalizer::to_vec(&value)?;
    Ok(ContentHash(Sha256::digest(&canonical).into()))
}

/// Replaces integers outside the exact range of doubles by decimal strings.
///
/// Iterative, so that the nesting depth is bounded by the parser and not by
/// the stack of this function.
fn widen_large_integers(value: &mut Value) {
    let mut pending = vec![value];
    while let Some(current) = pending.pop() {
        match current {
            Value::Number(number) => {
                let exact = number
                    .as_u64()
                    .map(|n| n <= EXACT_DOUBLE_LIMIT)
                    .or_else(|| {
                        number
                            .as_i64()
                            .map(|n| n.unsigned_abs() <= EXACT_DOUBLE_LIMIT)
                    })
                    .unwrap_or(true);
                if !exact {
                    *current = Value::String(number.to_string());
                }
            }
            Value::Array(items) => pending.extend(items.iter_mut()),
            Value::Object(members) => pending.extend(members.values_mut()),
            Value::Null | Value::Bool(_) | Value::String(_) => {}
        }
    }
}

/// Why a manifest cannot be verified at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ManifestDefect {
    /// The bytes are not a JSON document.
    NotJson,
    /// The document is not a JSON object.
    NotObject,
    /// The `integrity` member or its digest is missing.
    MissingDigest,
    /// The recorded digest is not in the text form of a content hash.
    MalformedDigest,
    /// The digest algorithm is not SHA-256.
    UnknownAlgorithm,
    /// The document cannot be brought into canonical form.
    NotCanonical,
}

impl ManifestDefect {
    /// Stable snake-case name, used in error details and salvage reports.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::NotJson => "not_json",
            Self::NotObject => "not_object",
            Self::MissingDigest => "missing_digest",
            Self::MalformedDigest => "malformed_digest",
            Self::UnknownAlgorithm => "unknown_algorithm",
            Self::NotCanonical => "not_canonical",
        }
    }
}

/// Result of verifying the bytes of a manifest.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ManifestVerdict {
    /// The recorded digest matches the content.
    Intact {
        /// Parsed manifest, ready for typed decoding.
        manifest: Value,
        /// Digest of the manifest.
        digest: ContentHash,
    },
    /// The recorded digest differs from the content.
    Mismatch {
        /// Digest recorded in the manifest.
        recorded: ContentHash,
        /// Digest computed from the content.
        computed: ContentHash,
    },
    /// The manifest cannot be verified.
    Unreadable(ManifestDefect),
}

impl ManifestVerdict {
    /// Whether the manifest passed verification.
    pub(crate) fn is_intact(&self) -> bool {
        matches!(self, Self::Intact { .. })
    }
}

/// Verifies the bytes of a manifest against its recorded digest.
///
/// Accepts arbitrary bytes and never panics; nesting deeper than the limit of
/// the JSON parser yields [`ManifestDefect::NotJson`].
pub(crate) fn verify_manifest(bytes: &[u8]) -> ManifestVerdict {
    let Ok(manifest) = serde_json::from_slice::<Value>(bytes) else {
        return ManifestVerdict::Unreadable(ManifestDefect::NotJson);
    };
    let Some(members) = manifest.as_object() else {
        return ManifestVerdict::Unreadable(ManifestDefect::NotObject);
    };
    let Some(integrity) = members.get(INTEGRITY).and_then(Value::as_object) else {
        return ManifestVerdict::Unreadable(ManifestDefect::MissingDigest);
    };
    if integrity.get(DIGEST_METHOD).and_then(Value::as_str) != Some(DIGEST_SHA256) {
        return ManifestVerdict::Unreadable(ManifestDefect::UnknownAlgorithm);
    }
    let Some(recorded) = integrity.get(MANIFEST_SHA256).and_then(Value::as_str) else {
        return ManifestVerdict::Unreadable(ManifestDefect::MissingDigest);
    };
    let Ok(recorded) = ContentHash::parse(recorded) else {
        return ManifestVerdict::Unreadable(ManifestDefect::MalformedDigest);
    };
    let Ok(computed) = manifest_sha256(&manifest) else {
        return ManifestVerdict::Unreadable(ManifestDefect::NotCanonical);
    };
    if computed == recorded {
        ManifestVerdict::Intact {
            manifest,
            digest: computed,
        }
    } else {
        ManifestVerdict::Mismatch { recorded, computed }
    }
}

/// Chain link recorded by an entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RecordedLink {
    /// Neither the manifest nor a tombstone carrying the link is available.
    Unavailable,
    /// The entry records no predecessor.
    First,
    /// The entry records the digest of its predecessor.
    After(ContentHash),
}

/// One entry of the account index, in index order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ChainEntry {
    /// Entry.
    pub(crate) op: OpId,
    /// Manifest digest recorded in the index row.
    pub(crate) digest: ContentHash,
    /// Link recorded by the entry itself.
    pub(crate) link: RecordedLink,
}

/// Verification status of one link.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum LinkStatus {
    /// The link names the immediately preceding index row, or the entry is
    /// the first of the account and records no predecessor.
    Verified,
    /// The link cannot be checked: the entry no longer has its link, or it
    /// names a predecessor older than the oldest retained index row.
    Unverifiable,
    /// The link names an earlier index row that is not the immediate
    /// predecessor; the chain forked, for example after a crash between a
    /// commit and the append of its index row.
    Forked {
        /// Entry named by the link.
        predecessor: OpId,
    },
    /// The link names no retained entry although the immediate predecessor
    /// is available, or an entry other than the first records no
    /// predecessor.
    Broken,
}

/// Several entries linked to the same predecessor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ChainFork {
    /// Common predecessor.
    pub predecessor: OpId,
    /// Entries linked to it, in index order.
    pub successors: Vec<OpId>,
}

/// Result of verifying the chain of an account index.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct ChainReport {
    /// Status of every entry, in index order.
    pub links: Vec<(OpId, LinkStatus)>,
    /// Forks, ordered by the index position of the predecessor.
    pub forks: Vec<ChainFork>,
}

impl ChainReport {
    /// Whether no link is broken. Forks and unverifiable links are reported
    /// but do not make a chain broken.
    pub fn is_unbroken(&self) -> bool {
        self.links
            .iter()
            .all(|(_, status)| *status != LinkStatus::Broken)
    }
}

/// Verifies the chain of an account index.
///
/// `entries` are the rows of the index in append order. The chain is
/// evidence, not a validity condition of single entries: a broken or forked
/// link never changes the state of an entry whose manifest is intact.
pub(crate) fn verify_chain(entries: &[ChainEntry]) -> ChainReport {
    let mut position: HashMap<ContentHash, usize> = HashMap::with_capacity(entries.len());
    for (index, entry) in entries.iter().enumerate() {
        position.entry(entry.digest).or_insert(index);
    }
    let mut successors: HashMap<usize, Vec<OpId>> = HashMap::new();
    let mut links = Vec::with_capacity(entries.len());
    for (index, entry) in entries.iter().enumerate() {
        let status = match entry.link {
            RecordedLink::Unavailable => LinkStatus::Unverifiable,
            RecordedLink::First if index == 0 => LinkStatus::Verified,
            RecordedLink::First => LinkStatus::Broken,
            RecordedLink::After(digest) => match position.get(&digest) {
                Some(&named) if named < index => {
                    successors.entry(named).or_default().push(entry.op);
                    if named + 1 == index {
                        LinkStatus::Verified
                    } else {
                        let predecessor = entries.get(named).map_or(entry.op, |row| row.op);
                        LinkStatus::Forked { predecessor }
                    }
                }
                Some(_) => LinkStatus::Broken,
                None if index == 0 => LinkStatus::Unverifiable,
                None => LinkStatus::Broken,
            },
        };
        links.push((entry.op, status));
    }
    let mut forks: Vec<(usize, Vec<OpId>)> = successors
        .into_iter()
        .filter(|(_, ops)| ops.len() > 1)
        .collect();
    forks.sort_unstable_by_key(|(named, _)| *named);
    let forks = forks
        .into_iter()
        .filter_map(|(named, successors)| {
            entries.get(named).map(|row| ChainFork {
                predecessor: row.op,
                successors,
            })
        })
        .collect();
    ChainReport { links, forks }
}

/// Integrity of the manifest of one entry part.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ManifestStatus {
    /// The manifest matches its digest.
    Intact(ContentHash),
    /// The recorded digest differs from the content.
    Mismatch,
    /// The manifest cannot be verified.
    Unreadable(ManifestDefect),
    /// The manifest file does not exist.
    Missing,
}

impl ManifestStatus {
    /// Stable snake-case name of a defect; `None` for an intact manifest.
    pub(crate) fn problem(self) -> Option<&'static str> {
        match self {
            Self::Intact(_) => None,
            Self::Mismatch => Some("digest_mismatch"),
            Self::Unreadable(defect) => Some(defect.as_str()),
            Self::Missing => Some("missing"),
        }
    }
}

/// Result of checking the files of one entry part.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EntryCheck {
    /// State; `None` when the state file is missing or unreadable.
    pub(crate) state: Option<StateRecord>,
    /// Integrity of the manifest.
    pub(crate) manifest: ManifestStatus,
    /// The item state file is absent or passes its checksum. When it fails,
    /// every item of the entry is treated as unavailable.
    pub(crate) items_intact: bool,
}

impl EntryCheck {
    /// Whether the entry must not be used for undo or details
    /// (`E_JOURNAL_CORRUPT`): it is marked corrupt, its state is unreadable
    /// or its manifest fails verification.
    pub(crate) fn is_corrupt(&self) -> bool {
        let marked = self
            .state
            .is_none_or(|record| record.state == EntryState::Corrupt);
        marked || !matches!(self.manifest, ManifestStatus::Intact(_))
    }
}

/// Checks the state, manifest and item state files of an entry part.
///
/// Arbitrary content of the three files never causes a panic.
///
/// # Errors
///
/// Returns the file system error of reading a file, other than a missing
/// file.
pub(crate) fn check_entry(fs: &dyn Fs, entry: &EntryLayout) -> FsResult<EntryCheck> {
    let state = match manifest::read_state(fs, &entry.state()?) {
        Ok(record) => Some(record),
        Err(ReadError::Fs(error)) if error.kind != FsErrorKind::NotFound => return Err(error),
        Err(_) => None,
    };
    let manifest = match manifest::read_bytes(fs, &entry.manifest()?) {
        Ok(bytes) => match verify_manifest(&bytes) {
            ManifestVerdict::Intact { digest, .. } => ManifestStatus::Intact(digest),
            ManifestVerdict::Mismatch { .. } => ManifestStatus::Mismatch,
            ManifestVerdict::Unreadable(defect) => ManifestStatus::Unreadable(defect),
        },
        Err(error) if error.kind == FsErrorKind::NotFound => ManifestStatus::Missing,
        Err(error) => return Err(error),
    };
    let items_intact = match manifest::read_items(fs, &entry.items()?) {
        Ok(_) => true,
        Err(ReadError::Fs(error)) => return Err(error),
        Err(ReadError::Format(_)) => false,
    };
    Ok(EntryCheck {
        state,
        manifest,
        items_intact,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use serde_json::json;

    /// Minimal manifest: only the members that matter for the digest rule.
    fn sample_manifest() -> Value {
        json!({
            "v": 1,
            "op": 18,
            "kind": { "command": "write" },
            "targets": [ { "path": "/home/user/a.txt", "before": { "modified_ns": 1_763_720_031_123_456_789_u64 } } ],
            "integrity": { "algo": "sha256", "chain": null }
        })
    }

    fn with_digest(mut manifest: Value) -> Value {
        let digest = manifest_sha256(&manifest).unwrap();
        manifest["integrity"]["manifest_sha256"] = json!(digest.to_string());
        manifest
    }

    fn hash(byte: u8) -> ContentHash {
        ContentHash([byte; 32])
    }

    /// Canonical form of [`sample_manifest`]; the expected digest is the
    /// SHA-256 of exactly these bytes.
    const SAMPLE_CANONICAL: &str = concat!(
        r#"{"integrity":{"algo":"sha256","chain":null},"kind":{"command":"write"},"op":18,"#,
        r#""targets":[{"before":{"modified_ns":"1763720031123456789"},"path":"/home/user/a.txt"}],"#,
        r#""v":1}"#
    );

    #[test]
    fn digest_matches_fixed_vector() {
        let expected = ContentHash(Sha256::digest(SAMPLE_CANONICAL.as_bytes()).into());
        let digest = manifest_sha256(&sample_manifest()).unwrap();
        assert_eq!(digest, expected);
        assert_eq!(
            digest.to_string(),
            "sha256:419052ded471c447bfa367cfbbe302368d57005adb1700333d57bd8a3793940a"
        );
    }

    #[test]
    fn digest_ignores_recorded_digest() {
        let plain = sample_manifest();
        let recorded = with_digest(plain.clone());
        assert_eq!(
            manifest_sha256(&plain).unwrap(),
            manifest_sha256(&recorded).unwrap()
        );
    }

    #[test]
    fn digest_covers_chain_and_low_digits_of_large_integers() {
        let base = manifest_sha256(&sample_manifest()).unwrap();
        let mut chained = sample_manifest();
        chained["integrity"]["chain"] = json!(hash(1).to_string());
        assert_ne!(manifest_sha256(&chained).unwrap(), base);
        let mut shifted = sample_manifest();
        shifted["targets"][0]["before"]["modified_ns"] = json!(1_763_720_031_123_456_788_u64);
        assert_ne!(manifest_sha256(&shifted).unwrap(), base);
        let mut negative = sample_manifest();
        negative["targets"][0]["before"]["modified_ns"] = json!(-1_763_720_031_123_456_789_i64);
        assert_ne!(manifest_sha256(&negative).unwrap(), base);
    }

    #[test]
    fn digest_does_not_depend_on_member_order_or_whitespace() {
        let text = r#"{ "op": 18, "v": 1,
            "integrity": {"chain": null, "algo": "sha256"},
            "targets": [{"path": "/home/user/a.txt", "before": {"modified_ns": 1763720031123456789}}],
            "kind": {"command": "write"} }"#;
        let parsed: Value = serde_json::from_str(text).unwrap();
        assert_eq!(
            manifest_sha256(&parsed).unwrap(),
            manifest_sha256(&sample_manifest()).unwrap()
        );
    }

    #[test]
    fn intact_manifest_verifies() {
        let bytes = serde_json::to_vec(&with_digest(sample_manifest())).unwrap();
        let verdict = verify_manifest(&bytes);
        assert!(verdict.is_intact());
        if let ManifestVerdict::Intact { manifest, digest } = verdict {
            assert_eq!(manifest["op"], json!(18));
            assert_eq!(digest, manifest_sha256(&sample_manifest()).unwrap());
        }
    }

    #[test]
    fn changed_member_is_a_mismatch() {
        let mut manifest = with_digest(sample_manifest());
        manifest["targets"][0]["path"] = json!("/home/user/b.txt");
        let verdict = verify_manifest(&serde_json::to_vec(&manifest).unwrap());
        assert!(
            matches!(verdict, ManifestVerdict::Mismatch { recorded, computed } if recorded != computed)
        );
    }

    #[test]
    fn defects_are_classified() {
        let cases: [(&[u8], ManifestDefect); 6] = [
            (b"\x00\xff garbage", ManifestDefect::NotJson),
            (b"[1, 2]", ManifestDefect::NotObject),
            (br#"{"v": 1}"#, ManifestDefect::MissingDigest),
            (
                br#"{"integrity": {"algo": "md5", "manifest_sha256": "x"}}"#,
                ManifestDefect::UnknownAlgorithm,
            ),
            (
                br#"{"integrity": {"algo": "sha256"}}"#,
                ManifestDefect::MissingDigest,
            ),
            (
                br#"{"integrity": {"algo": "sha256", "manifest_sha256": "sha256:00"}}"#,
                ManifestDefect::MalformedDigest,
            ),
        ];
        for (bytes, defect) in cases {
            assert_eq!(verify_manifest(bytes), ManifestVerdict::Unreadable(defect));
            assert!(!defect.as_str().is_empty());
        }
    }

    #[test]
    fn deep_nesting_is_rejected_without_overflow() {
        let depth = 100_000;
        let mut text = "[".repeat(depth);
        text.push_str(&"]".repeat(depth));
        assert_eq!(
            verify_manifest(text.as_bytes()),
            ManifestVerdict::Unreadable(ManifestDefect::NotJson)
        );
    }

    fn row(op: u64, digest: u8, link: RecordedLink) -> ChainEntry {
        ChainEntry {
            op: OpId(op),
            digest: hash(digest),
            link,
        }
    }

    #[test]
    fn linear_chain_verifies() {
        let rows = [
            row(1, 1, RecordedLink::First),
            row(2, 2, RecordedLink::After(hash(1))),
            row(3, 3, RecordedLink::After(hash(2))),
        ];
        let report = verify_chain(&rows);
        assert!(report.is_unbroken());
        assert!(report.forks.is_empty());
        assert!(
            report
                .links
                .iter()
                .all(|(_, status)| *status == LinkStatus::Verified)
        );
    }

    #[test]
    fn missing_link_or_trimmed_predecessor_is_unverifiable() {
        let rows = [
            row(7, 7, RecordedLink::After(hash(6))),
            row(8, 8, RecordedLink::Unavailable),
            row(9, 9, RecordedLink::After(hash(8))),
        ];
        let report = verify_chain(&rows);
        assert_eq!(
            report.links,
            vec![
                (OpId(7), LinkStatus::Unverifiable),
                (OpId(8), LinkStatus::Unverifiable),
                (OpId(9), LinkStatus::Verified),
            ]
        );
        assert!(report.is_unbroken());
    }

    #[test]
    fn reappended_row_after_crash_is_a_fork() {
        // Entry 2 committed with link to 1, crashed before its index row was
        // appended; entry 3 then linked to 1 as well; recovery appended 2.
        let rows = [
            row(1, 1, RecordedLink::First),
            row(3, 3, RecordedLink::After(hash(1))),
            row(2, 2, RecordedLink::After(hash(1))),
        ];
        let report = verify_chain(&rows);
        assert_eq!(report.links[1], (OpId(3), LinkStatus::Verified));
        assert_eq!(
            report.links[2],
            (
                OpId(2),
                LinkStatus::Forked {
                    predecessor: OpId(1)
                }
            )
        );
        assert_eq!(
            report.forks,
            vec![ChainFork {
                predecessor: OpId(1),
                successors: vec![OpId(3), OpId(2)],
            }]
        );
        assert!(report.is_unbroken());
    }

    #[test]
    fn unknown_or_forward_link_and_late_first_are_broken() {
        let rows = [
            row(1, 1, RecordedLink::First),
            row(2, 2, RecordedLink::After(hash(9))),
            row(3, 3, RecordedLink::After(hash(4))),
            row(4, 4, RecordedLink::First),
        ];
        let report = verify_chain(&rows);
        assert_eq!(
            report.links,
            vec![
                (OpId(1), LinkStatus::Verified),
                (OpId(2), LinkStatus::Broken),
                (OpId(3), LinkStatus::Broken),
                (OpId(4), LinkStatus::Broken),
            ]
        );
        assert!(!report.is_unbroken());
    }

    #[test]
    fn integers_are_widened_only_beyond_two_to_the_fifty_third() {
        let mut value = json!({
            "a": 9_007_199_254_740_992_u64,
            "b": 9_007_199_254_740_993_u64,
            "c": -9_007_199_254_740_992_i64,
            "d": -9_007_199_254_740_993_i64,
            "e": [9_007_199_254_740_993_u64, { "f": 18_446_744_073_709_551_615_u64 }],
            "g": 1.5,
            "h": "x",
            "i": null,
            "j": true
        });
        widen_large_integers(&mut value);
        assert_eq!(value["a"], json!(9_007_199_254_740_992_u64));
        assert_eq!(value["b"], json!("9007199254740993"));
        assert_eq!(value["c"], json!(-9_007_199_254_740_992_i64));
        assert_eq!(value["d"], json!("-9007199254740993"));
        assert_eq!(value["e"][0], json!("9007199254740993"));
        assert_eq!(value["e"][1]["f"], json!("18446744073709551615"));
        assert_eq!(value["g"], json!(1.5));
        assert_eq!(value["h"], json!("x"));
        assert!(value["i"].is_null());
        assert_eq!(value["j"], json!(true));
    }

    #[test]
    fn the_digest_distinguishes_neighbouring_large_integers_in_nested_members() {
        let with = |low: u64| {
            json!({
                "integrity": { "algo": "sha256", "chain": null },
                "targets": [ { "before": { "modified_ns": 1_763_720_031_123_456_000_u64 + low } } ]
            })
        };
        let first = manifest_sha256(&with(1)).unwrap();
        assert_eq!(first, manifest_sha256(&with(1)).unwrap());
        assert_ne!(first, manifest_sha256(&with(2)).unwrap());
    }

    #[test]
    fn manifest_problems_have_stable_names() {
        assert_eq!(ManifestStatus::Intact(hash(1)).problem(), None);
        assert_eq!(ManifestStatus::Mismatch.problem(), Some("digest_mismatch"));
        assert_eq!(ManifestStatus::Missing.problem(), Some("missing"));
        for (defect, name) in [
            (ManifestDefect::NotJson, "not_json"),
            (ManifestDefect::NotObject, "not_object"),
            (ManifestDefect::MissingDigest, "missing_digest"),
            (ManifestDefect::MalformedDigest, "malformed_digest"),
            (ManifestDefect::UnknownAlgorithm, "unknown_algorithm"),
            (ManifestDefect::NotCanonical, "not_canonical"),
        ] {
            assert_eq!(defect.as_str(), name);
            assert_eq!(ManifestStatus::Unreadable(defect).problem(), Some(name));
        }
    }

    #[test]
    fn an_empty_index_has_an_unbroken_chain_and_a_self_link_is_broken() {
        let empty = verify_chain(&[]);
        assert!(empty.links.is_empty() && empty.forks.is_empty());
        assert!(empty.is_unbroken());
        let rows = [
            row(1, 1, RecordedLink::First),
            row(2, 2, RecordedLink::After(hash(2))),
        ];
        let report = verify_chain(&rows);
        assert_eq!(report.links[1], (OpId(2), LinkStatus::Broken));
        assert!(!report.is_unbroken());
    }

    #[test]
    fn rows_with_the_same_digest_resolve_a_link_to_the_first_of_them() {
        let rows = [
            row(1, 1, RecordedLink::First),
            row(2, 1, RecordedLink::After(hash(1))),
            row(3, 3, RecordedLink::After(hash(1))),
        ];
        let report = verify_chain(&rows);
        assert_eq!(report.links[1], (OpId(2), LinkStatus::Verified));
        assert_eq!(
            report.links[2],
            (
                OpId(3),
                LinkStatus::Forked {
                    predecessor: OpId(1)
                }
            )
        );
        assert_eq!(report.forks.len(), 1);
        assert_eq!(report.forks[0].successors, [OpId(2), OpId(3)]);
    }

    fn entry_files(state: &[u8], manifest_bytes: &[u8], items: &[u8]) -> EntryCheck {
        use nurt_contracts::fs::AbsPath;
        use nurt_fs::mem::{MemFs, VolumeSpec};
        let mount = AbsPath::new("/v").unwrap();
        let fs = MemFs::builder()
            .volume(VolumeSpec::posix(mount))
            .build()
            .unwrap();
        let entry = EntryLayout::new(AbsPath::new("/v/ops/000000000001").unwrap());
        fs.seed_file(&entry.state().unwrap(), state).unwrap();
        fs.seed_file(&entry.manifest().unwrap(), manifest_bytes)
            .unwrap();
        fs.seed_file(&entry.items().unwrap(), items).unwrap();
        check_entry(&fs, &entry).unwrap()
    }

    #[test]
    fn intact_entry_passes_and_damaged_files_are_reported() {
        let manifest_bytes = serde_json::to_vec(&with_digest(sample_manifest())).unwrap();
        let items = crate::manifest::Items::default().encode().unwrap();
        let intact = entry_files(b"committed\n", &manifest_bytes, &items);
        assert!(!intact.is_corrupt());
        assert!(intact.items_intact);
        assert_eq!(intact.manifest.problem(), None);
        let damaged = entry_files(b"committed\n", b"{}", b"garbage");
        assert!(damaged.is_corrupt());
        assert!(!damaged.items_intact);
        assert_eq!(damaged.manifest.problem(), Some("missing_digest"));
        let marked = entry_files(b"corrupt\n", &manifest_bytes, &items);
        assert!(marked.is_corrupt());
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]

        #[test]
        fn entry_check_never_panics_on_arbitrary_files(
            state in proptest::collection::vec(any::<u8>(), 0..64),
            manifest_bytes in proptest::collection::vec(any::<u8>(), 0..256),
            items in proptest::collection::vec(any::<u8>(), 0..128),
        ) {
            let check = entry_files(&state, &manifest_bytes, &items);
            prop_assert!(check.is_corrupt());
        }
    }

    proptest! {
        #[test]
        fn manifest_verification_never_panics(bytes in proptest::collection::vec(any::<u8>(), 0..512)) {
            let _ = verify_manifest(&bytes);
        }

        #[test]
        fn json_shaped_input_never_panics(text in r#"[\[\]{}",:0-9a-z\-\. ]{0,256}"#) {
            let _ = verify_manifest(text.as_bytes());
        }

        #[test]
        fn chain_verification_never_panics(
            rows in proptest::collection::vec((0_u64..8, 0_u8..6, 0_u8..8), 0..24)
        ) {
            let entries: Vec<ChainEntry> = rows
                .into_iter()
                .map(|(op, digest, link)| ChainEntry {
                    op: OpId(op),
                    digest: hash(digest),
                    link: match link {
                        0 => RecordedLink::Unavailable,
                        1 => RecordedLink::First,
                        other => RecordedLink::After(hash(other)),
                    },
                })
                .collect();
            let report = verify_chain(&entries);
            prop_assert_eq!(report.links.len(), entries.len());
        }
    }
}
