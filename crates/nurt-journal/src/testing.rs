// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-journal::testing
// Responsibility:   Setup of the recovery measurement of `tests/perf.rs`; it serves only the.
// Dependencies:     nurt_contracts, nurt_fs
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Setup of the recovery measurement of `tests/perf.rs`; it serves only the
//! performance tests and is not part of the interface of the crate.

// A failed expectation is how the setup of a test reports a failure.
#![allow(clippy::expect_used, clippy::missing_panics_doc)]

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use nurt_contracts::config::{
    AgeClock, AgeLimit, ByteLimit, ConfigLevel, Oversize, OverwriteStrategy, SizeLimit, StepLimit,
};
use nurt_contracts::env::{BootId, ClockStamp, WallTime};
use nurt_contracts::fs::{AbsPath, Follow, Fs, FsFeatures, VolumeId, VolumeInfo};
use nurt_contracts::journal::{
    Admission, AdmissionDecision, AdmissionRequest, EffectiveRetention, Eligibility, EntryState,
    OpId, OpKind, PreserveMode, ServiceLiveness, Sourced, TargetAction, TargetIndex, TargetPath,
    Thresholds, VolumeState, VolumeStatus,
};
use nurt_fs::clock::VirtualClock;
use nurt_fs::failpoint::NoFailpoints;
use nurt_fs::mem::{MemFs, VolumeSpec};

use crate::layout::{BlobId, BlobRole, JournalLayout, StateLayout};
use crate::manifest::{
    self, FORMAT_VERSION, Integrity, Manifest, PartRef, StateRecord, TargetRecord, TargetRole,
};
use crate::recovery::recover;
use crate::store::{JournalCtx, JournalLocation, RetentionLimits, Settings, VolumeJournals};

const OLD: &[u8] = b"previous content";
const NEW: &[u8] = b"new content of the operation";

struct Admit;

impl Admission for Admit {
    fn eligibility(&self, _volume: &VolumeInfo) -> Eligibility {
        Eligibility::Eligible
    }

    fn admit(&self, _request: &AdmissionRequest) -> AdmissionDecision {
        AdmissionDecision::Keep
    }

    fn after_operation(&self, _volume: &VolumeId) {}

    fn volume_status(&self, _volume: &VolumeId) -> VolumeStatus {
        VolumeStatus {
            state: VolumeState::Normal,
            thresholds: Thresholds {
                reserve: 0,
                soft: 0,
                margin: 0,
                hysteresis: 0,
            },
        }
    }

    fn liveness(&self) -> ServiceLiveness {
        ServiceLiveness::Alive
    }
}

struct Limits;

fn sourced<T>(value: T) -> Sourced<T> {
    Sourced {
        value,
        source: ConfigLevel::Builtin,
        raised_to_floor: false,
    }
}

impl RetentionLimits for Limits {
    fn effective(&self, _volume: &VolumeInfo) -> EffectiveRetention {
        EffectiveRetention {
            max_ops: sourced(StepLimit::Limited(30)),
            max_age: sourced(AgeLimit::Limited(
                nurt_contracts::config::DurationSetting::from_secs(1800),
            )),
            max_size: sourced(SizeLimit::Unlimited),
            max_size_bytes: None,
            oversize: sourced(Oversize::Skip),
            age_clock: sourced(AgeClock::Wall),
            max_op_size: sourced(ByteLimit::Unlimited),
        }
    }
}

struct Journals(HashMap<u64, JournalLocation>);

impl VolumeJournals for Journals {
    fn locate(&self, volume: &VolumeInfo) -> JournalLocation {
        self.0
            .get(&volume.id.dev)
            .cloned()
            .unwrap_or(JournalLocation::Inactive(
                nurt_contracts::journal::InactiveReason::NoAnchor,
            ))
    }

    fn active(&self) -> Vec<AbsPath> {
        self.0
            .values()
            .filter_map(|location| match location {
                JournalLocation::Active(dir) => Some(dir.clone()),
                JournalLocation::Inactive(_) => None,
            })
            .collect()
    }
}

fn abs(text: &str) -> AbsPath {
    AbsPath::for_test(text)
}

fn features() -> FsFeatures {
    FsFeatures {
        exchange: true,
        rename_noreplace: true,
        replace_with_backup: true,
        hardlink: true,
        reflink: true,
        xattr: true,
        acl: true,
        alternate_streams: false,
        durable_metadata: true,
        case_sensitive: true,
        decomposes_names: false,
    }
}

/// Seeds `empty_entries` empty entry directories next to one prepared entry
/// that overwrites a file, recovers the journal and returns the time of the
/// recovery. Panics when the recovery fails or does not roll the entry back.
pub fn time_recovery_among_empty_entries(empty_entries: u64) -> Duration {
    let fs = Arc::new(
        MemFs::builder()
            .volume(VolumeSpec::posix(abs("/a")).with_features(features()))
            .build()
            .expect("the volume is valid"),
    );
    let journal = JournalLayout::new(abs("/a/.nurt-journal"));
    for dir in journal.structure().expect("the layout has a structure") {
        fs.seed_dir(&dir).expect("the directory is seeded");
    }
    fs.seed_dir(&abs("/a/state"))
        .expect("the directory is seeded");
    let volume = fs.volume(&abs("/a")).expect("the volume exists").id;
    let mut locations = HashMap::new();
    locations.insert(volume.dev, JournalLocation::Active(journal.root().clone()));
    let ctx = JournalCtx {
        fs: fs.clone(),
        providers: Vec::new(),
        admission: Arc::new(Admit),
        limits: Arc::new(Limits),
        clock: Arc::new(VirtualClock::new(VirtualClock::DEFAULT_START)),
        journals: Arc::new(Journals(locations)),
        state: StateLayout::new(abs("/a/state")),
        failpoints: Arc::new(NoFailpoints),
        settings: Settings {
            overwrite: OverwriteStrategy::Exchange,
            lock_timeout: Duration::from_millis(50),
            reference_schemes: Vec::new(),
        },
        space: None,
        startup: OnceLock::new(),
    };
    for op in 10..10 + empty_entries {
        fs.seed_dir(journal.entry(OpId(op)).expect("the entry has a path").dir())
            .expect("the directory is seeded");
    }
    seed_prepared_entry(&fs, &journal, volume);
    let started = Instant::now();
    recover(&ctx).expect("the recovery succeeds");
    let elapsed = started.elapsed();
    let restored = manifest::read_bytes(&*fs, &abs("/a/f")).expect("the file exists");
    assert_eq!(restored, OLD, "the entry was not rolled back");
    elapsed
}

fn seed_prepared_entry(fs: &MemFs, journal: &JournalLayout, volume: VolumeId) {
    fs.seed_file(&abs("/a/f"), OLD).expect("the file is seeded");
    let meta = fs
        .metadata(&abs("/a/f"), Follow::NoFollow)
        .expect("the file has metadata");
    let id = BlobId::new(TargetIndex(0), BlobRole::Previous);
    let before = manifest::BeforeRecord {
        exists: true,
        kind: Some(meta.kind),
        size: Some(meta.size),
        mode: Some(meta.mode),
        uid: meta.uid,
        gid: meta.gid,
        modified_ns: Some(meta.modified_ns),
        hash: None,
        links: Some(meta.links),
        file_id: Some(meta.file_id),
        store: PreserveMode::Exchange,
        blob: Some(id.content_name()),
        meta: Some(id.meta_name()),
        inline: None,
    };
    let wall = WallTime(1_700_000_000_000_000_000);
    let mut entry = Manifest {
        v: FORMAT_VERSION,
        op: OpId(1),
        uuid: uuid::Uuid::from_u128(1),
        time: wall,
        time_basis: ClockStamp {
            wall,
            boot: BootId("boot".to_owned()),
            since_boot_with_suspend: Duration::from_secs(60),
            since_boot_active: Duration::from_secs(60),
        },
        kind: OpKind::command("write"),
        session: None,
        account: None,
        ast_hash: manifest::hash_bytes(b"program"),
        ast: serde_json::json!({}),
        corrections: Vec::new(),
        bulk: false,
        counts_as_step: true,
        targets: vec![TargetRecord {
            index: TargetIndex(0),
            path: TargetPath::Fs(abs("/a/f")),
            action: TargetAction::Modify,
            role: TargetRole::Whole,
            before,
            replaced: None,
            after: None,
            in_place: false,
            bytes_held: 0,
            irreversible: None,
            notable: Vec::new(),
        }],
        parts: vec![PartRef {
            vol: volume,
            dir: journal.root().clone(),
        }],
        bytes_held: 0,
        irreversible: Vec::new(),
        notable: Vec::new(),
        exempt: Vec::new(),
        expires: None,
        undo_of: Vec::new(),
        redo_of: None,
        scope: None,
        items: Vec::new(),
        integrity: Integrity::default(),
    };
    let digest_dir = journal.entry(entry.op).expect("the entry has a path");
    entry.seal().expect("the manifest is sealed");
    fs.seed_file(
        &digest_dir.manifest().expect("the manifest has a path"),
        &entry.encode().expect("the manifest is encoded"),
    )
    .expect("the manifest is seeded");
    fs.seed_file(
        &digest_dir.state().expect("the state has a path"),
        &StateRecord::new(EntryState::Prepared).encode(),
    )
    .expect("the state is seeded");
    let _ = fs.seed_dir(&digest_dir.blobs().expect("the blobs have a path"));
    fs.seed_file(
        &journal.pending(entry.op).expect("the marker has a path"),
        b"",
    )
    .expect("the marker is seeded");
    let target = abs("/a/f");
    let staged = target
        .parent()
        .expect("the file has a parent")
        .join_name(&crate::layout::staged_name(OpId(1), TargetIndex(0)))
        .expect("the staged name is valid");
    fs.seed_file(&staged, NEW)
        .expect("the staged file is seeded");
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
