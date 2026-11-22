// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! OpenPGP public keys from key files and GnuPG keyrings.
//!
//! GnuPG keeps public keys in one of three stores in its home directory:
//! `pubring.kbx` (keybox), `pubring.gpg` (a sequence of OpenPGP packets,
//! GnuPG before 2.1) and `public-keys.d/pubring.db` (SQLite database of
//! `keyboxd`, GnuPG 2.4 and later). All present stores are read.
//!
//! A keybox file is a sequence of blobs. Every blob starts with its length
//! (32-bit big-endian, including the header) and a type byte; type 2 holds
//! an OpenPGP key block whose offset and length (both from the start of the
//! blob) follow at bytes 8 and 12.

use std::path::{Path, PathBuf};

use nurt_contracts::error::Result;
use pgp::composed::{Deserializable, SignedPublicKey, SignedSecretKey};

use crate::sign::error::io_error;

const ARMOR_PREFIX: &[u8] = b"-----BEGIN PGP";
const KEYBOX_FILE: &str = "pubring.kbx";
const LEGACY_KEYRING_FILE: &str = "pubring.gpg";
const KEYBOXD_DIR: &str = "public-keys.d";
const KEYBOXD_DATABASE: &str = "pubring.db";
const KEYBOX_BLOB_OPENPGP: u8 = 2;
const KEYBOX_HEADER_LEN: usize = 16;
const KEYBOX_TYPE_OFFSET: usize = 4;
const KEYBOX_BLOCK_OFFSET: usize = 8;
const KEYBOX_BLOCK_LEN_OFFSET: usize = 12;
const KEYBOXD_QUERY: &str = "SELECT keyblob FROM pubkey WHERE type = 1";

/// Whether `bytes` start with an OpenPGP armor header line, after leading
/// white space.
pub(crate) fn is_armored(bytes: &[u8]) -> bool {
    let start = bytes
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .unwrap_or(bytes.len());
    bytes[start..].starts_with(ARMOR_PREFIX)
}

/// Keys of the configured key files; a secret key contributes its public
/// part.
///
/// A file that cannot be read is an error, because it was named
/// explicitly.
pub(crate) fn load_files(paths: &[PathBuf]) -> Result<Vec<SignedPublicKey>> {
    let mut certs = Vec::new();
    for path in paths {
        let bytes = std::fs::read(path).map_err(|source| io_error(path, &source))?;
        certs.extend(parse_certs(&bytes));
    }
    Ok(certs)
}

/// Keys of every public keyring in the GnuPG home directory `home`;
/// missing or unreadable stores contribute nothing.
pub(crate) fn load_gnupg(home: &Path) -> Vec<SignedPublicKey> {
    let mut certs = Vec::new();
    if let Ok(bytes) = std::fs::read(home.join(KEYBOX_FILE)) {
        for block in keybox_blocks(&bytes) {
            certs.extend(parse_certs(block));
        }
    }
    if let Ok(bytes) = std::fs::read(home.join(LEGACY_KEYRING_FILE)) {
        certs.extend(parse_certs(&bytes));
    }
    for block in keyboxd_blocks(&home.join(KEYBOXD_DIR).join(KEYBOXD_DATABASE)) {
        certs.extend(parse_certs(&block));
    }
    certs
}

/// Parses every transferable public or secret key in `bytes`, armored or
/// binary; malformed keys are skipped.
pub(crate) fn parse_certs(bytes: &[u8]) -> Vec<SignedPublicKey> {
    let public: Vec<SignedPublicKey> = if is_armored(bytes) {
        SignedPublicKey::from_armor_many(bytes)
            .map(|(keys, _)| keys.filter_map(std::result::Result::ok).collect())
            .unwrap_or_default()
    } else {
        SignedPublicKey::from_bytes_many(bytes)
            .map(|keys| keys.filter_map(std::result::Result::ok).collect())
            .unwrap_or_default()
    };
    if !public.is_empty() {
        return public;
    }
    let secret: Vec<SignedSecretKey> = if is_armored(bytes) {
        SignedSecretKey::from_armor_many(bytes)
            .map(|(keys, _)| keys.filter_map(std::result::Result::ok).collect())
            .unwrap_or_default()
    } else {
        SignedSecretKey::from_bytes_many(bytes)
            .map(|keys| keys.filter_map(std::result::Result::ok).collect())
            .unwrap_or_default()
    };
    secret.iter().map(SignedSecretKey::to_public_key).collect()
}

fn read_u32(bytes: &[u8], offset: usize) -> Option<usize> {
    let field: [u8; 4] = bytes.get(offset..offset + 4)?.try_into().ok()?;
    usize::try_from(u32::from_be_bytes(field)).ok()
}

/// The OpenPGP key blocks of a keybox file.
pub(crate) fn keybox_blocks(bytes: &[u8]) -> Vec<&[u8]> {
    let mut blocks = Vec::new();
    let mut offset = 0;
    while let Some(length) = read_u32(bytes, offset) {
        if length <= KEYBOX_TYPE_OFFSET || offset + length > bytes.len() {
            break;
        }
        let blob = &bytes[offset..offset + length];
        if blob[KEYBOX_TYPE_OFFSET] == KEYBOX_BLOB_OPENPGP && length >= KEYBOX_HEADER_LEN {
            let block = read_u32(blob, KEYBOX_BLOCK_OFFSET).zip(read_u32(blob, KEYBOX_BLOCK_LEN_OFFSET));
            if let Some(block) = block.and_then(|(start, len)| blob.get(start..start.checked_add(len)?)) {
                blocks.push(block);
            }
        }
        offset += length;
    }
    blocks
}

/// The OpenPGP key blocks of a `keyboxd` database; empty when the database
/// is absent or cannot be read.
fn keyboxd_blocks(path: &Path) -> Vec<Vec<u8>> {
    if !path.is_file() {
        return Vec::new();
    }
    let flags = rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let Ok(connection) = rusqlite::Connection::open_with_flags(path, flags) else {
        return Vec::new();
    };
    let Ok(mut statement) = connection.prepare(KEYBOXD_QUERY) else {
        return Vec::new();
    };
    statement
        .query_map([], |row| row.get::<_, Vec<u8>>(0))
        .map(|rows| rows.filter_map(std::result::Result::ok).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::super::fixtures::{ED25519_FINGERPRINT, ED25519_PUBLIC, RSA_FINGERPRINT, RSA_PUBLIC};
    use super::super::fingerprint_hex;
    use super::*;
    use pgp::types::KeyDetails;

    fn binary(armored: &str) -> Vec<u8> {
        let (cert, _) = SignedPublicKey::from_armor_single(armored.as_bytes()).expect("fixture");
        pgp::ser::Serialize::to_bytes(&cert).expect("serialize")
    }

    /// A keybox with a header blob, one OpenPGP blob and one X.509 blob.
    fn keybox(block: &[u8]) -> Vec<u8> {
        let mut file = Vec::new();
        let mut header = vec![0, 0, 0, 32, 1, 1, 0, 2];
        header.extend_from_slice(b"KBXf");
        header.resize(32, 0);
        file.extend_from_slice(&header);
        let block_offset = 32_u32;
        let blob_len = block_offset as usize + block.len();
        let mut blob = Vec::new();
        blob.extend_from_slice(&u32::try_from(blob_len).expect("length").to_be_bytes());
        blob.extend_from_slice(&[KEYBOX_BLOB_OPENPGP, 1, 0, 0]);
        blob.extend_from_slice(&block_offset.to_be_bytes());
        blob.extend_from_slice(&u32::try_from(block.len()).expect("length").to_be_bytes());
        blob.resize(block_offset as usize, 0);
        blob.extend_from_slice(block);
        file.extend_from_slice(&blob);
        file.extend_from_slice(&[0, 0, 0, 8, 3, 1, 0, 0]);
        file
    }

    #[test]
    fn reads_keys_from_keybox_and_legacy_keyring() {
        let home = tempfile::tempdir().expect("tempdir");
        std::fs::write(home.path().join(KEYBOX_FILE), keybox(&binary(ED25519_PUBLIC))).expect("write");
        std::fs::write(home.path().join(LEGACY_KEYRING_FILE), binary(RSA_PUBLIC)).expect("write");
        let fingerprints: Vec<String> = load_gnupg(home.path())
            .iter()
            .map(|cert| fingerprint_hex(&cert.primary_key.fingerprint()))
            .collect();
        assert_eq!(fingerprints, [ED25519_FINGERPRINT, RSA_FINGERPRINT]);
    }

    #[test]
    fn reads_keys_from_keyboxd_database() {
        let home = tempfile::tempdir().expect("tempdir");
        let dir = home.path().join(KEYBOXD_DIR);
        std::fs::create_dir(&dir).expect("mkdir");
        let connection = rusqlite::Connection::open(dir.join(KEYBOXD_DATABASE)).expect("open");
        connection
            .execute_batch("CREATE TABLE pubkey (ubid BLOB PRIMARY KEY, type INTEGER, keyblob BLOB);")
            .expect("schema");
        connection
            .execute(
                "INSERT INTO pubkey (ubid, type, keyblob) VALUES (?1, 1, ?2)",
                rusqlite::params![vec![1_u8; 20], binary(RSA_PUBLIC)],
            )
            .expect("insert");
        drop(connection);
        let certs = load_gnupg(home.path());
        assert_eq!(certs.len(), 1);
        assert_eq!(fingerprint_hex(&certs[0].primary_key.fingerprint()), RSA_FINGERPRINT);
    }

    #[test]
    fn stops_at_truncated_keybox_blob() {
        let mut file = keybox(&binary(ED25519_PUBLIC));
        file.extend_from_slice(&[0, 0, 1, 0, 2]);
        assert_eq!(keybox_blocks(&file).len(), 1);
        assert!(keybox_blocks(&[0, 0, 0, 2]).is_empty());
    }

    #[test]
    fn treats_unreadable_key_file_as_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let error = load_files(&[dir.path().join("absent.asc")]).expect_err("missing");
        assert_eq!(error.code().as_str(), "E_NOT_FOUND");
    }

    #[test]
    fn recognizes_armor_after_white_space() {
        assert!(is_armored(b"\n  -----BEGIN PGP PUBLIC KEY BLOCK-----"));
        assert!(!is_armored(&[0x99, 0x01]));
    }
}
