// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-files/tests/differential_hash.rs
// Responsibility:   Differential tests of `nurt_text::hash` against `sha256sum`, `sha512sum`, `sha1sum`, `md5sum` and `b3sum`.
// Dependencies:     nurt_text, support_corpus, support_diff
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Differential tests of `nurt_text::hash` against `sha256sum`, `sha512sum`, `sha1sum`, `md5sum` and `b3sum`.

#![cfg(all(feature = "differential", unix))]
// Test helpers unwrap results so that a failure aborts the test with the original error.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support_corpus;
mod support_diff;

use std::fs;
use std::io::Cursor;

use nurt_text::hash::{HashAlgorithm, Hasher, hash_reader};
use support_corpus::Rng;
use support_diff::{require, run_expect};

/// Sizes around the block boundaries of the hash functions, and larger inputs.
const SIZES: [usize; 12] = [0, 1, 55, 56, 63, 64, 65, 127, 128, 129, 4_096, 1 << 20];
const SEED: u64 = 0x5EED_0001;
const CHUNK: usize = 1_000;

fn random_bytes(rng: &mut Rng, length: usize) -> Vec<u8> {
    (0..length).map(|_| rng.next_u64() as u8).collect()
}

fn check(algorithm: HashAlgorithm, program: &str) {
    let Some(path) = require(program) else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let mut rng = Rng::new(SEED);
    for size in SIZES {
        let data = random_bytes(&mut rng, size);
        fs::write(dir.path().join("input.bin"), &data).unwrap();
        let output = run_expect(&path, &["input.bin"], dir.path(), &[0]);
        let text = String::from_utf8(output.stdout).unwrap();
        let reference = text.split_whitespace().next().unwrap();

        let mut whole = Hasher::new(algorithm);
        whole.update(&data);
        assert_eq!(
            whole.finalize().to_hex(),
            reference,
            "{program}, {size} bytes"
        );

        let mut chunked = Hasher::new(algorithm);
        data.chunks(CHUNK).for_each(|chunk| chunked.update(chunk));
        assert_eq!(
            chunked.finalize().to_hex(),
            reference,
            "{program}, {size} bytes in chunks"
        );

        let streamed = hash_reader(algorithm, Cursor::new(&data)).unwrap();
        assert_eq!(
            streamed.to_hex(),
            reference,
            "{program}, {size} bytes from a reader"
        );
    }
}

#[test]
fn sha256_matches_sha256sum() {
    check(HashAlgorithm::Sha256, "sha256sum");
}

#[test]
fn sha512_matches_sha512sum() {
    check(HashAlgorithm::Sha512, "sha512sum");
}

#[test]
fn sha1_matches_sha1sum() {
    check(HashAlgorithm::Sha1, "sha1sum");
}

#[test]
fn md5_matches_md5sum() {
    check(HashAlgorithm::Md5, "md5sum");
}

#[test]
fn blake3_matches_b3sum() {
    check(HashAlgorithm::Blake3, "b3sum");
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
