// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-git-transport::send_pack::pack
// Responsibility:   Pack generation for push.
// Dependencies:     gix, gix_pack, nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Pack generation for push.
//!
//! Entries already stored as deltas in local packs are copied when their
//! base is part of the same pack; every other object is sent whole. No new
//! deltas are computed, so a push of many revisions of large text files is
//! larger than the pack `git` would send.

use std::io::Write;
use std::sync::atomic::AtomicBool;

use gix::ObjectId;
use gix_pack::data::output::{self, count::objects::ObjectExpansion, entry::iter_from_counts};
use nurt_contracts::error::Result;

use super::error::library_error;
use super::objects::MISSING_OBJECT;

/// Pack size from which the absence of new deltas is reported.
pub const DELTA_NOTICE_BYTES: u64 = 8 * 1024 * 1024;

/// Size and composition of a written pack.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PackSummary {
    /// Objects in the pack.
    pub objects: u64,
    /// Entries copied as deltas from local packs.
    pub deltas: u64,
    /// Bytes written, including header and trailer.
    pub bytes: u64,
}

impl PackSummary {
    /// Whether the pack is large enough for the missing delta compression to
    /// matter.
    #[must_use]
    pub fn without_deltas_notable(&self) -> bool {
        self.bytes >= DELTA_NOTICE_BYTES
    }
}

/// Writes a version 2 pack with the objects `ids` of `repo` to `out`.
///
/// # Errors
///
/// Fails with `E_IO` and problem `missing_object` when an object is absent,
/// and with `E_INTERNAL` when the pack cannot be produced.
pub fn write_pack(
    repo: &gix::Repository,
    ids: Vec<ObjectId>,
    out: &mut dyn Write,
    interrupt: &AtomicBool,
) -> Result<PackSummary> {
    // Packs must stay mapped while their entries are read: after an unload
    // `gix-odb` panics in `entry_by_location` on large histories.
    let mut db = repo
        .objects
        .clone()
        .into_arc()
        .map_err(|error| library_error("opening the object database failed", &error))?
        .into_inner();
    db.prevent_pack_unload();
    let mut input = ids.into_iter().map(Ok);
    let (counts, _) = output::count::objects_unthreaded(
        &db,
        &mut input,
        &gix::progress::Discard,
        interrupt,
        ObjectExpansion::AsIs,
    )
    .map_err(|error| missing_or_library("counting objects failed", &error))?;
    let entries = u32::try_from(counts.len())
        .map_err(|_| library_error("writing the pack failed", &"too many objects"))?;
    let options = output::entry::iter_from_counts::Options {
        thread_limit: Some(1),
        allow_thin_pack: false,
        version: gix_pack::data::Version::V2,
        ..Default::default()
    };
    let mut chunks = iter_from_counts(counts, db, Box::new(gix::progress::Discard), options)
        .map_err(|error| library_error("resolving pack entries failed", &error))?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|error| missing_or_library("resolving pack entries failed", &error))?;
    chunks.sort_by_key(|(sequence, _)| *sequence);
    let mut summary = PackSummary {
        objects: u64::from(entries),
        ..PackSummary::default()
    };
    for entry in chunks.iter().flat_map(|(_, entries)| entries) {
        if entry.is_invalid() {
            return Err(nurt_contracts::error::Error::new(
                nurt_contracts::error::ErrorCode::Io,
                format!("object {} is missing from the repository", entry.id),
            )
            .with_problem(MISSING_OBJECT));
        }
        if !matches!(entry.kind, output::entry::Kind::Base(_)) {
            summary.deltas += 1;
        }
    }
    let input = chunks.into_iter().map(|(_, entries)| Ok(entries));
    let writer = gix_pack::data::output::bytes::FromEntriesIter::new(
        input,
        out,
        entries,
        gix_pack::data::Version::V2,
        repo.object_hash(),
    );
    for written in writer {
        summary.bytes +=
            written.map_err(|error| library_error("writing the pack failed", &error))?;
    }
    Ok(summary)
}

fn missing_or_library(
    context: &str,
    error: &dyn std::fmt::Display,
) -> nurt_contracts::error::Error {
    let text = error.to_string();
    if text.contains("not found") || text.contains("could not be found") {
        return nurt_contracts::error::Error::new(
            nurt_contracts::error::ErrorCode::Io,
            format!("{context}: {text}"),
        )
        .with_problem(MISSING_OBJECT);
    }
    library_error(context, &text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::send_pack::objects::objects_to_send;
    use crate::send_pack::testing::Fixture;

    #[test]
    fn writes_a_pack_that_indexes_back_to_the_same_objects() {
        let fixture = Fixture::new();
        let base = fixture.commit("main", &[("a.txt", "one\n")], &[]);
        let repo = fixture.repo();
        let ids = objects_to_send(&repo, &[base], &[], &AtomicBool::new(false)).unwrap();
        let mut pack = Vec::new();
        let summary = write_pack(&repo, ids.clone(), &mut pack, &AtomicBool::new(false)).unwrap();
        assert_eq!(summary.objects, 3);
        assert_eq!(summary.bytes, pack.len() as u64);
        assert!(pack.starts_with(b"PACK"));
        assert!(!summary.without_deltas_notable());
        let target = Fixture::new();
        let written = target.index_pack(&pack);
        assert_eq!(written, 3);
    }

    #[test]
    fn writes_an_empty_pack() {
        let fixture = Fixture::new();
        fixture.commit("main", &[("a.txt", "one\n")], &[]);
        let mut pack = Vec::new();
        let summary = write_pack(
            &fixture.repo(),
            Vec::new(),
            &mut pack,
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(summary.objects, 0);
        assert_eq!(pack.len(), 32, "header and trailer");
    }

    #[test]
    fn fails_for_a_missing_object() {
        let fixture = Fixture::new();
        fixture.commit("main", &[("a.txt", "one\n")], &[]);
        let unknown = ObjectId::from_bytes_or_panic(&[9; 20]);
        let error = write_pack(
            &fixture.repo(),
            vec![unknown],
            &mut Vec::new(),
            &AtomicBool::new(false),
        )
        .unwrap_err();
        assert!(matches!(
            error.code(),
            nurt_contracts::error::ErrorCode::Io | nurt_contracts::error::ErrorCode::Internal
        ));
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
