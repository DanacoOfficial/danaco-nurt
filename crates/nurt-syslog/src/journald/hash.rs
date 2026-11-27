// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-syslog::journald::hash
// Responsibility:   Hash functions of the journal file format.
// Dependencies:     siphasher
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Hash functions of the journal file format.
//!
//! Files without the flag `KEYED_HASH` hash payloads with Bob Jenkins'
//! lookup3 (`hashlittle2`, public domain), taking the first returned word as
//! the high half of the 64-bit value. Files with the flag use SipHash-2-4
//! keyed with the 16 bytes of the file identifier. The `xor_hash` of an
//! entry always uses lookup3.

use siphasher::sip::SipHasher24;

const LOOKUP3_INIT: u32 = 0xdead_beef;
const WORD: usize = 4;
const BLOCK: usize = 12;
const HALF_BITS: u32 = 32;

/// Computes the lookup3 `hashlittle2` of `data` with seeds `pc` and `pb`
/// and returns the pair `(pc, pb)`.
///
/// ```
/// use nurt_syslog::journald::hash::hashlittle2;
///
/// assert_eq!(hashlittle2(b"Four score and seven years ago", 0, 0), (0x1777_0551, 0xce72_26e6));
/// ```
#[must_use]
pub fn hashlittle2(data: &[u8], pc: u32, pb: u32) -> (u32, u32) {
    let length = u32::try_from(data.len()).unwrap_or(u32::MAX);
    let start = LOOKUP3_INIT.wrapping_add(length).wrapping_add(pc);
    let (mut a, mut b, mut c) = (start, start, start.wrapping_add(pb));
    let (blocks, tail) = data.as_chunks::<BLOCK>();
    let (full, last) = match (blocks.split_last(), tail.is_empty()) {
        (Some((last, full)), true) => (full, Some(last.as_slice())),
        _ => (blocks, (!tail.is_empty()).then_some(tail)),
    };
    for block in full {
        let [x, y, z] = words(block);
        a = a.wrapping_add(x);
        b = b.wrapping_add(y);
        c = c.wrapping_add(z);
        (a, b, c) = mix(a, b, c);
    }
    let Some(last) = last else {
        return (c, b);
    };
    let mut padded = [0_u8; BLOCK];
    for (slot, byte) in padded.iter_mut().zip(last) {
        *slot = *byte;
    }
    let [x, y, z] = words(&padded);
    a = a.wrapping_add(x);
    b = b.wrapping_add(y);
    c = c.wrapping_add(z);
    (_, b, c) = final_mix(a, b, c);
    (c, b)
}

/// Computes the 64-bit lookup3 hash used by journal files without
/// `KEYED_HASH`.
///
/// ```
/// use nurt_syslog::journald::hash::lookup3;
///
/// assert_eq!(lookup3(b""), 0xdead_beef_dead_beef);
/// ```
#[must_use]
pub fn lookup3(data: &[u8]) -> u64 {
    let (high, low) = hashlittle2(data, 0, 0);
    (u64::from(high) << HALF_BITS) | u64::from(low)
}

fn words(block: &[u8; BLOCK]) -> [u32; 3] {
    let (chunks, _) = block.as_chunks::<WORD>();
    let mut out = [0_u32; 3];
    for (slot, chunk) in out.iter_mut().zip(chunks) {
        *slot = u32::from_le_bytes(*chunk);
    }
    out
}

const fn mix(mut a: u32, mut b: u32, mut c: u32) -> (u32, u32, u32) {
    a = a.wrapping_sub(c);
    a ^= c.rotate_left(4);
    c = c.wrapping_add(b);
    b = b.wrapping_sub(a);
    b ^= a.rotate_left(6);
    a = a.wrapping_add(c);
    c = c.wrapping_sub(b);
    c ^= b.rotate_left(8);
    b = b.wrapping_add(a);
    a = a.wrapping_sub(c);
    a ^= c.rotate_left(16);
    c = c.wrapping_add(b);
    b = b.wrapping_sub(a);
    b ^= a.rotate_left(19);
    a = a.wrapping_add(c);
    c = c.wrapping_sub(b);
    c ^= b.rotate_left(4);
    b = b.wrapping_add(a);
    (a, b, c)
}

const fn final_mix(mut a: u32, mut b: u32, mut c: u32) -> (u32, u32, u32) {
    c ^= b;
    c = c.wrapping_sub(b.rotate_left(14));
    a ^= c;
    a = a.wrapping_sub(c.rotate_left(11));
    b ^= a;
    b = b.wrapping_sub(a.rotate_left(25));
    c ^= b;
    c = c.wrapping_sub(b.rotate_left(16));
    a ^= c;
    a = a.wrapping_sub(c.rotate_left(4));
    b ^= a;
    b = b.wrapping_sub(a.rotate_left(14));
    c ^= b;
    c = c.wrapping_sub(b.rotate_left(24));
    (a, b, c)
}

/// SipHash-2-4 of `data` with a 16-byte key read as two little-endian
/// words, the hash of journal files with `KEYED_HASH`.
///
/// ```
/// use nurt_syslog::journald::hash::siphash24;
///
/// let key: [u8; 16] = std::array::from_fn(|index| index as u8);
/// assert_eq!(siphash24(&key, b""), 0x726f_db47_dd0e_0e31);
/// ```
#[must_use]
pub fn siphash24(key: &[u8; 16], data: &[u8]) -> u64 {
    SipHasher24::new_with_key(key).hash(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PHRASE: &[u8] = b"Four score and seven years ago";

    fn reference_key() -> [u8; 16] {
        std::array::from_fn(|index| u8::try_from(index).unwrap())
    }

    #[test]
    fn lookup3_matches_published_vectors() {
        assert_eq!(hashlittle2(b"", 0, 0), (0xdead_beef, 0xdead_beef));
        assert_eq!(hashlittle2(PHRASE, 0, 0), (0x1777_0551, 0xce72_26e6));
        assert_eq!(hashlittle2(PHRASE, 0, 1), (0xe360_7cae, 0xbd37_1de4));
        assert_eq!(hashlittle2(PHRASE, 1, 0), (0xcd62_8161, 0x6cbe_a4b3));
    }

    #[test]
    fn lookup3_of_journal_payloads() {
        assert_eq!(lookup3(b"MESSAGE=hello"), 0x87dd_eff2_fd1b_d06d);
        assert_eq!(lookup3(b"_SYSTEMD_UNIT=ssh.service"), 0x2c67_c8bc_2e32_9bc3);
    }

    #[test]
    fn lookup3_handles_every_tail_length() {
        let data: Vec<u8> = (0..40).collect();
        let hashes: Vec<u64> = (0..data.len()).map(|end| lookup3(&data[..end])).collect();
        let mut unique = hashes.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), hashes.len());
    }

    #[test]
    fn siphash_matches_reference_vectors() {
        let key = reference_key();
        let message: Vec<u8> = (0..15).collect();
        assert_eq!(siphash24(&key, b""), 0x726f_db47_dd0e_0e31);
        assert_eq!(siphash24(&key, &message[..8]), 0x93f5_f579_9a93_2462);
        assert_eq!(siphash24(&key, &message), 0xa129_ca61_49be_45e5);
        assert_eq!(siphash24(&key, b"MESSAGE=hello"), 0xc1e4_7240_469d_2e88);
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
