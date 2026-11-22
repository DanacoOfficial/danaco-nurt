// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Helpers of the archive command tests: stages and arguments, archives
//! built in memory, tar archives with members the archive writer refuses,
//! and runs of effect stages through the recording journal of the virtual
//! file system.

// Every test binary that includes this module uses a different subset of
// the helpers.
#![allow(dead_code)]
// Tests report failures by panicking.
#![allow(clippy::expect_used)]

use std::collections::HashMap;
use std::sync::Arc;

use nurt_archive::create::{CreateOptions, Input, create};
use nurt_archive::list::Format;
use nurt_contracts::error::Result;
use nurt_contracts::fs::AbsPath;
use nurt_contracts::journal::{JournalStore, RollbackReason, TargetIndex, TargetPath};
use nurt_contracts::record::{ArchiveEntry, Change, Effect};
use nurt_contracts::report::NotableEntry;
use nurt_contracts::stage::{ArgValue, BoundArgs, StageImpl, StageInput};
use nurt_contracts::value::{Record, RecordKind, Value};
use nurt_testkit::ctx::TestContext;
use nurt_testkit::fixtures::begin_request;
use nurt_testkit::journal::JournalCall;
use nurt_testkit::vfs::test_path;

/// Size of a tar block.
const BLOCK: usize = 512;
/// Fields of a ustar header: offset and length.
const NAME: (usize, usize) = (0, 100);
const MODE: (usize, usize) = (100, 8);
const UID: (usize, usize) = (108, 8);
const GID: (usize, usize) = (116, 8);
const SIZE: (usize, usize) = (124, 12);
const MTIME: (usize, usize) = (136, 12);
const CHECKSUM: (usize, usize) = (148, 8);
const TYPE: usize = 156;
const LINK_NAME: (usize, usize) = (157, 100);
const SIGNATURE: (usize, usize) = (257, 8);
const USTAR_SIGNATURE: &[u8] = b"ustar\x0000";

/// Absolute path of the virtual file system.
pub(crate) fn abs(path: &str) -> AbsPath {
    test_path(path).expect("test paths are absolute")
}

/// Language form of a path of the virtual file system.
pub(crate) fn language(path: &str) -> String {
    abs(path).to_language_string()
}

/// Path value of an argument.
pub(crate) fn path(text: &str) -> Value {
    Value::Path(language(text))
}

/// Enumeration value of an argument.
pub(crate) fn choice(text: &str) -> Value {
    Value::Enum(text.to_owned())
}

/// Registered stage of a command.
pub(crate) fn stage(command: &str) -> Arc<dyn StageImpl> {
    nurt_files::stages()
        .into_iter()
        .find(|stage| stage.descriptor().command == command)
        .expect("the command is registered")
}

/// Arguments of a command.
pub(crate) fn args(command: &str, given: &[(&str, Value)]) -> BoundArgs {
    given
        .iter()
        .fold(BoundArgs::new(command), |args, (name, value)| {
            args.with(*name, ArgValue::Value(value.clone()), false)
        })
}

/// `entry` record of a path, with a `relative` field when given.
pub(crate) fn entry(path: &str, relative: Option<&str>) -> Record {
    let record = Record::new(RecordKind::Entry).with("path", Value::Path(language(path)));
    match relative {
        Some(relative) => record.with("relative", Value::Text(relative.to_owned())),
        None => record,
    }
}

/// Archive of the given format with the inputs; regular files take their
/// content from `contents` by path.
pub(crate) fn build(format: Format, inputs: Vec<Input>, contents: &[(&str, &[u8])]) -> Vec<u8> {
    let contents: HashMap<&str, &[u8]> = contents.iter().copied().collect();
    create(Vec::new(), &CreateOptions::new(format), inputs, |input| {
        let content = contents
            .get(input.path.as_str())
            .copied()
            .unwrap_or_default();
        Ok(Box::new(content))
    })
    .expect("the archive is written")
}

/// Archive of regular files with their content.
pub(crate) fn files(format: Format, contents: &[(&str, &[u8])]) -> Vec<u8> {
    let inputs = contents
        .iter()
        .map(|(path, content)| Input::file(*path, content.len() as u64))
        .collect();
    build(format, inputs, contents)
}

/// Member of a tar archive written as given, without any checks.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Raw<'a> {
    /// Regular file with content.
    File(&'a str, &'a [u8]),
    /// Directory.
    Directory(&'a str),
    /// Symbolic link with its target.
    Symlink(&'a str, &'a str),
    /// Hard link to another member or path.
    HardLink(&'a str, &'a str),
}

/// Plain ustar archive with the members exactly as given.
pub(crate) fn raw_tar(members: &[Raw<'_>]) -> Vec<u8> {
    let mut out = Vec::new();
    for member in members {
        let (name, kind, content, link) = match *member {
            Raw::File(name, content) => (name, b'0', content, ""),
            Raw::Directory(name) => (name, b'5', &[][..], ""),
            Raw::Symlink(name, target) => (name, b'2', &[][..], target),
            Raw::HardLink(name, target) => (name, b'1', &[][..], target),
        };
        let mut header = [0_u8; BLOCK];
        put(&mut header, NAME, name.as_bytes());
        put(&mut header, MODE, b"0000644\0");
        put(&mut header, UID, b"0000000\0");
        put(&mut header, GID, b"0000000\0");
        put(
            &mut header,
            SIZE,
            format!("{:011o}\0", content.len()).as_bytes(),
        );
        put(&mut header, MTIME, b"00000000000\0");
        put(&mut header, CHECKSUM, b"        ");
        header[TYPE] = kind;
        put(&mut header, LINK_NAME, link.as_bytes());
        put(&mut header, SIGNATURE, USTAR_SIGNATURE);
        let sum: u32 = header.iter().map(|byte| u32::from(*byte)).sum();
        put(&mut header, CHECKSUM, format!("{sum:06o}\0 ").as_bytes());
        out.extend_from_slice(&header);
        out.extend_from_slice(content);
        out.resize(out.len().div_ceil(BLOCK) * BLOCK, 0);
    }
    out.resize(out.len() + 2 * BLOCK, 0);
    out
}

fn put(header: &mut [u8; BLOCK], (offset, length): (usize, usize), bytes: &[u8]) {
    assert!(bytes.len() <= length, "a tar header field is too long");
    header[offset..offset + bytes.len()].copy_from_slice(bytes);
}

/// Result of an applied effect stage.
#[derive(Debug)]
pub(crate) struct Outcome {
    /// Changes as applied.
    pub(crate) changes: Vec<Change>,
    /// The `effect` record of the output.
    pub(crate) effect: Effect,
    /// Notable effects of the plan.
    pub(crate) notable: Vec<NotableEntry>,
}

impl Outcome {
    /// Counter of the summary of the `effect` record.
    pub(crate) fn counter(&self, name: &str) -> u64 {
        self.effect
            .summary
            .as_ref()
            .and_then(|summary| summary.get(name).copied())
            .unwrap_or_default()
    }

    /// Notable entries with the given code name.
    pub(crate) fn notable(&self, code: &str) -> Vec<&NotableEntry> {
        self.notable
            .iter()
            .filter(|entry| entry.code.as_str() == code)
            .collect()
    }
}

/// Plans and applies an effect stage in one transaction and commits it; a
/// failed apply step rolls the transaction back.
pub(crate) fn run_effect(
    test: &TestContext,
    command: &str,
    args: &BoundArgs,
    input: StageInput<'_>,
) -> Result<Outcome> {
    let ctx = test.ctx();
    let stage = stage(command);
    let effect = stage.as_effect().expect("the command changes files");
    let plan = effect.plan(args, input, &ctx)?;
    let notable = plan.notable.clone();
    let mut tx = test.journal().begin(begin_request(command));
    let outcome = match effect.apply(args, plan, tx.as_mut(), &ctx) {
        Ok(outcome) => outcome,
        Err(error) => {
            tx.rollback(RollbackReason::TargetFailed)
                .expect("the rollback succeeds");
            return Err(error);
        }
    };
    let changes = outcome.changes;
    let record = StageInput::from(outcome.output)
        .into_records()?
        .next()
        .expect("the stage produces an effect record")?;
    let effect = Effect::try_from(record).expect("the record is an effect");
    tx.commit()?;
    Ok(Outcome {
        changes,
        effect,
        notable,
    })
}

/// Runs `archive.list` and returns its records.
pub(crate) fn list_archive(test: &TestContext, args: &BoundArgs) -> Result<Vec<ArchiveEntry>> {
    let ctx = test.ctx();
    let stage = stage("archive.list");
    let output =
        stage
            .as_read()
            .expect("archive.list reads")
            .execute(args, StageInput::none(), &ctx)?;
    StageInput::from(output)
        .into_records()?
        .map(
            |record| Ok(ArchiveEntry::try_from(record?).expect("every record is an archive entry")),
        )
        .collect()
}

/// Targets declared to the journal, in order.
pub(crate) fn declared(test: &TestContext) -> Vec<TargetPath> {
    test.journal()
        .calls()
        .into_iter()
        .filter_map(|call| match call {
            JournalCall::Preserve { targets, .. } => Some(targets),
            _ => None,
        })
        .flatten()
        .collect()
}

/// Targets changed successfully, in order.
pub(crate) fn applied_ok(test: &TestContext) -> Vec<TargetIndex> {
    test.journal()
        .calls()
        .into_iter()
        .filter_map(|call| match call {
            JournalCall::Apply {
                index, ok: true, ..
            } => Some(index),
            _ => None,
        })
        .collect()
}

/// Indexes `0..count` of the declared targets.
pub(crate) fn indexes(count: usize) -> Vec<TargetIndex> {
    (0..count)
        .map(|index| TargetIndex(u32::try_from(index).expect("few targets")))
        .collect()
}

/// Paths of every mutation made on the virtual file system so far.
pub(crate) fn mutated_paths(test: &TestContext) -> Vec<AbsPath> {
    test.fs()
        .mutations()
        .into_iter()
        .flat_map(|record| record.paths)
        .collect()
}
