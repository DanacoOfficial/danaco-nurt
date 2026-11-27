// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-git::apply::resolve_undo
// Responsibility:   The `REUC` (resolve-undo) index extension.
// Dependencies:     bstr, gix, gix_index
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! The `REUC` (resolve-undo) index extension.
//!
//! `gix-index` decodes the extension but does not write it. When an update
//! resolves the conflict stages of a path, `git` records the stages in the
//! extension so that `git checkout -m` and `git update-index --unresolve`
//! can restore them; the extension is appended to the serialized index here.

use std::collections::{BTreeMap, BTreeSet};

use bstr::{BString, ByteSlice};
use gix::ObjectId;

const SIGNATURE: &[u8; 4] = b"REUC";
const STAGE_COUNT: usize = 3;

/// Stages of one path as recorded by the extension: mode and object id for
/// stage 1 (base), 2 (ours) and 3 (theirs).
type Stages = [Option<(u32, ObjectId)>; STAGE_COUNT];

/// Payload of the extension for the paths whose conflict stages exist in
/// `before` and not in `after`; empty when no path was resolved.
pub(super) fn payload(before: &gix_index::State, after: &gix_index::State) -> Vec<u8> {
    let still_conflicted: BTreeSet<&[u8]> = after
        .entries()
        .iter()
        .filter(|entry| entry.stage_raw() != 0)
        .map(|entry| entry.path(after).as_bytes())
        .collect();
    let mut resolved: BTreeMap<BString, Stages> = BTreeMap::new();
    for entry in before.entries() {
        let stage = usize::try_from(entry.stage_raw()).unwrap_or(0);
        let path = entry.path(before);
        if stage == 0 || still_conflicted.contains(path.as_bytes()) {
            continue;
        }
        if let Some(slot) = resolved
            .entry(path.to_owned())
            .or_default()
            .get_mut(stage - 1)
        {
            *slot = Some((entry.mode.bits(), entry.id));
        }
    }
    let mut out = Vec::new();
    for (path, stages) in &resolved {
        out.extend_from_slice(path);
        out.push(0);
        for stage in stages {
            let mode = stage.map_or(0, |(mode, _)| mode);
            out.extend_from_slice(format!("{mode:o}").as_bytes());
            out.push(0);
        }
        for (_, id) in stages.iter().flatten() {
            out.extend_from_slice(id.as_bytes());
        }
    }
    out
}

/// Appends the extension to a serialized index and recomputes the checksum.
pub(super) fn append(index: &mut Vec<u8>, payload: &[u8], kind: gix::hash::Kind) {
    if payload.is_empty() {
        return;
    }
    let body_len = index.len().saturating_sub(kind.len_in_bytes());
    index.truncate(body_len);
    index.extend_from_slice(SIGNATURE);
    index.extend_from_slice(
        &u32::try_from(payload.len())
            .unwrap_or(u32::MAX)
            .to_be_bytes(),
    );
    index.extend_from_slice(payload);
    let mut hasher = gix::hash::hasher(kind);
    hasher.update(index);
    if let Ok(checksum) = hasher.try_finalize() {
        index.extend_from_slice(checksum.as_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gix_index::entry::{Flags, Mode, Stat};

    fn state(entries: &[(&str, u8, u8)]) -> gix_index::State {
        let mut state = gix_index::State::new(gix::hash::Kind::Sha1);
        for (path, byte, stage) in entries {
            let id = ObjectId::from_bytes_or_panic(&[*byte; 20]);
            let flags = Flags::from_bits_retain(u32::from(*stage) << 12);
            state.dangerously_push_entry(
                Stat::default(),
                id,
                flags,
                Mode::FILE,
                path.as_bytes().as_bstr(),
            );
        }
        state.sort_entries();
        state
    }

    #[test]
    fn records_stages_of_resolved_path() {
        let before = state(&[("a", 1, 1), ("a", 2, 2), ("a", 3, 3), ("b", 9, 0)]);
        let after = state(&[("a", 2, 0), ("b", 9, 0)]);
        let payload = payload(&before, &after);
        assert!(payload.starts_with(b"a\x00100644\x00100644\x00100644\x00"));
        assert_eq!(
            payload.len(),
            b"a\x00100644\x00100644\x00100644\x00".len() + 60
        );
    }

    #[test]
    fn omits_missing_stage_and_keeps_unresolved_paths_out() {
        let before = state(&[("a", 2, 2), ("a", 3, 3), ("c", 4, 2), ("c", 5, 3)]);
        let after = state(&[("a", 2, 0), ("c", 4, 2), ("c", 5, 3)]);
        let payload = payload(&before, &after);
        assert!(payload.starts_with(b"a\x000\x00100644\x00100644\x00"));
        assert_eq!(payload.len(), "a\x000\x00100644\x00100644\x00".len() + 40);
    }

    #[test]
    fn appended_extension_carries_valid_checksum() {
        let before = state(&[("a", 2, 2), ("a", 3, 3)]);
        let after = state(&[("a", 2, 0)]);
        let mut bytes = Vec::new();
        gix_index::File::from_state(after.clone(), std::path::PathBuf::from("index"))
            .write_to(&mut bytes, gix_index::write::Options::default())
            .expect("serialize");
        let payload = payload(&before, &after);
        append(&mut bytes, &payload, gix::hash::Kind::Sha1);
        let (body, checksum) = bytes.split_at(bytes.len() - 20);
        let mut hasher = gix::hash::hasher(gix::hash::Kind::Sha1);
        hasher.update(body);
        assert_eq!(hasher.try_finalize().expect("hash").as_bytes(), checksum);
        assert!(body.windows(4).any(|window| window == SIGNATURE));
    }

    #[test]
    fn leaves_index_unchanged_without_payload() {
        let mut bytes = vec![1, 2, 3];
        append(&mut bytes, &[], gix::hash::Kind::Sha1);
        assert_eq!(bytes, [1, 2, 3]);
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
