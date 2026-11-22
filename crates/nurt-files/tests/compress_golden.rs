// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Examples of `docs/commands/compress.md` and `docs/commands/decompress.md`,
//! run on the virtual file system with the recording journal.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests report failures by panicking"
)]

use std::sync::Arc;

use nurt_archive::codec::{self, Algorithm};
use nurt_archive::compress::compress;
use nurt_archive::decompress::decompress;
use nurt_contracts::error::{ErrorCode, Result};
use nurt_contracts::fs::AbsPath;
use nurt_contracts::journal::{JournalStore, TargetIndex, TargetPath};
use nurt_contracts::record::{ChangeAction, Effect};
use nurt_contracts::stage::{ArgValue, BoundArgs, RecordStream, StageData, StageImpl, StageInput};
use nurt_contracts::value::{Record, RecordKind, Value};
use nurt_testkit::ctx::TestContext;
use nurt_testkit::fixtures::begin_request;
use nurt_testkit::journal::JournalCall;
use nurt_testkit::vfs::test_path;

const LOG: &[u8] = b"2026-01-01 start\n2026-01-01 stop\n2026-01-02 start\n";

fn stage(command: &str) -> Arc<dyn StageImpl> {
    nurt_files::stages()
        .into_iter()
        .find(|stage| stage.descriptor().command == command)
        .expect("the command is registered")
}

fn abs(text: &str) -> AbsPath {
    test_path(text).unwrap()
}

fn with_files(files: &[(&str, &[u8])]) -> TestContext {
    let test = TestContext::new();
    for (path, content) in files {
        test.fs().write_file(&abs(path), content).unwrap();
    }
    test
}

fn args(command: &str, paths: &[&str], params: &[(&str, Value)]) -> BoundArgs {
    let mut args = BoundArgs::new(command);
    let list = paths
        .iter()
        .map(|path| Value::Path(abs(path).to_language_string()))
        .collect();
    args.insert("path", ArgValue::Value(Value::List(list)), false);
    for (name, value) in params {
        args.insert(*name, ArgValue::Value(value.clone()), false);
    }
    args
}

fn choice(value: &str) -> Value {
    Value::Enum(value.to_owned())
}

fn packed(algorithm: Algorithm, data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    compress(algorithm, None, data, &mut out).unwrap();
    out
}

fn unpacked(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    decompress(data, &mut out, None).unwrap();
    out
}

/// Plan and apply in one transaction, as the executor does.
fn run(test: &TestContext, args: &BoundArgs, input: StageInput<'_>) -> Result<Effect> {
    let stage = stage(args.command());
    let effect_stage = stage.as_effect().expect("the command is an effect stage");
    let ctx = test.ctx();
    let plan = effect_stage.plan(args, input, &ctx)?;
    let mut tx = test.journal().begin(begin_request(args.command()));
    let outcome = effect_stage.apply(args, plan, tx.as_mut(), &ctx)?;
    let StageData::Records(stream) = outcome.output.data else {
        panic!("expected the effect record");
    };
    let records: Vec<Record> = stream.collect::<Result<_>>()?;
    tx.commit()?;
    Ok(Effect::try_from(records[0].clone()).unwrap())
}

fn declared(test: &TestContext) -> Vec<TargetPath> {
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

fn applied_ok(test: &TestContext) -> Vec<TargetIndex> {
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

fn actions(effect: &Effect) -> Vec<ChangeAction> {
    effect
        .changes
        .as_ref()
        .unwrap()
        .iter()
        .map(|change| change.action)
        .collect()
}

/// Example `compress "/srv/app/app.log"` writes a gzip file and keeps the
/// source.
#[test]
fn compress_a_log_file_with_gzip_and_keep_it() {
    let test = with_files(&[("/srv/app/app.log", LOG)]);
    let effect = run(
        &test,
        &args("compress", &["/srv/app/app.log"], &[]),
        StageInput::none(),
    )
    .unwrap();
    assert_eq!(actions(&effect), [ChangeAction::Create]);
    assert_eq!(test.fs().read_file(&abs("/srv/app/app.log")).unwrap(), LOG);
    let output = test.fs().read_file(&abs("/srv/app/app.log.gz")).unwrap();
    assert_eq!(codec::detect(&output), Some(Algorithm::Gzip));
    assert_eq!(unpacked(&output), LOG);
    assert_eq!(
        declared(&test),
        [TargetPath::Fs(abs("/srv/app/app.log.gz"))]
    );
    assert_eq!(applied_ok(&test), [TargetIndex(0)]);
}

/// Example `compress "/srv/app/a.csv" "/srv/app/b.csv" algorithm: "zstd"
/// level: 19 keep: false` replaces two files with their zstd forms.
#[test]
fn compress_two_files_with_zstd_and_remove_the_originals() {
    let test = with_files(&[
        ("/srv/app/a.csv", b"a,b\n1,2\n"),
        ("/srv/app/b.csv", b"c,d\n3,4\n"),
    ]);
    let given = args(
        "compress",
        &["/srv/app/a.csv", "/srv/app/b.csv"],
        &[
            ("algorithm", choice("zstd")),
            ("level", Value::Int(19)),
            ("keep", Value::Bool(false)),
        ],
    );
    let effect = run(&test, &given, StageInput::none()).unwrap();
    assert_eq!(
        actions(&effect),
        [
            ChangeAction::Create,
            ChangeAction::Create,
            ChangeAction::Delete,
            ChangeAction::Delete
        ]
    );
    assert!(!test.fs().exists(&abs("/srv/app/a.csv")));
    assert!(!test.fs().exists(&abs("/srv/app/b.csv")));
    let first = test.fs().read_file(&abs("/srv/app/a.csv.zst")).unwrap();
    assert_eq!(codec::detect(&first), Some(Algorithm::Zstd));
    assert_eq!(unpacked(&first), b"a,b\n1,2\n");
    assert_eq!(declared(&test).len(), applied_ok(&test).len());
}

/// Example `find ... |> compress algorithm: "xz"` compresses the listed
/// files.
#[test]
fn compress_the_listed_files_with_xz() {
    let test = with_files(&[
        ("/srv/app/logs/2026-01.log", LOG),
        ("/srv/app/logs/sub/2026-02.log", LOG),
    ]);
    let records = ["/srv/app/logs", "/srv/app/logs/2026-01.log"]
        .iter()
        .map(|path| {
            Record::new(RecordKind::Entry).with("path", Value::Path(abs(path).to_language_string()))
        })
        .collect();
    let mut given = BoundArgs::new("compress");
    given.insert("algorithm", ArgValue::Value(choice("xz")), false);
    let input = StageInput::records(RecordStream::from_records(records));
    let effect = run(&test, &given, input).unwrap();
    assert_eq!(actions(&effect), [ChangeAction::Create]);
    let output = test
        .fs()
        .read_file(&abs("/srv/app/logs/2026-01.log.xz"))
        .unwrap();
    assert_eq!(codec::detect(&output), Some(Algorithm::Xz));
    assert_eq!(unpacked(&output), LOG);
    assert!(!test.fs().exists(&abs("/srv/app/logs/sub/2026-02.log.xz")));
}

/// Example `compress ... level: 99` is rejected before any change.
#[test]
fn compress_rejects_a_level_outside_the_range() {
    let test = with_files(&[("/srv/app/app.log", LOG)]);
    let given = args(
        "compress",
        &["/srv/app/app.log"],
        &[("level", Value::Int(99))],
    );
    let error = run(&test, &given, StageInput::none()).unwrap_err();
    assert_eq!(error.code(), ErrorCode::InvalidValue);
    assert!(test.journal().calls().is_empty());
}

/// Example `decompress "/srv/app/app.log.gz"` writes the log and keeps the
/// compressed file.
#[test]
fn decompress_a_gzip_file_and_keep_it() {
    let compressed = packed(Algorithm::Gzip, LOG);
    let test = with_files(&[("/srv/app/app.log.gz", &compressed)]);
    let effect = run(
        &test,
        &args("decompress", &["/srv/app/app.log.gz"], &[]),
        StageInput::none(),
    )
    .unwrap();
    assert_eq!(actions(&effect), [ChangeAction::Create]);
    assert_eq!(test.fs().read_file(&abs("/srv/app/app.log")).unwrap(), LOG);
    assert_eq!(
        test.fs().read_file(&abs("/srv/app/app.log.gz")).unwrap(),
        compressed
    );
    assert_eq!(declared(&test), [TargetPath::Fs(abs("/srv/app/app.log"))]);
}

/// Example `decompress "/srv/app/data.csv.zst" keep: false` replaces the
/// compressed file with its content.
#[test]
fn decompress_a_zstd_file_and_remove_the_compressed_file() {
    let test = with_files(&[("/srv/app/data.csv.zst", &packed(Algorithm::Zstd, b"a,b\n"))]);
    let given = args(
        "decompress",
        &["/srv/app/data.csv.zst"],
        &[("keep", Value::Bool(false))],
    );
    let effect = run(&test, &given, StageInput::none()).unwrap();
    assert_eq!(
        actions(&effect),
        [ChangeAction::Create, ChangeAction::Delete]
    );
    assert!(!test.fs().exists(&abs("/srv/app/data.csv.zst")));
    assert_eq!(
        test.fs().read_file(&abs("/srv/app/data.csv")).unwrap(),
        b"a,b\n"
    );
    assert_eq!(declared(&test).len(), applied_ok(&test).len());
}

/// Example `decompress "/srv/app/backup.tgz"` writes `backup.tar`.
#[test]
fn decompress_a_compressed_tar_file_to_a_tar_file() {
    let tar = vec![0x61_u8; 1024];
    let test = with_files(&[("/srv/app/backup.tgz", &packed(Algorithm::Gzip, &tar))]);
    run(
        &test,
        &args("decompress", &["/srv/app/backup.tgz"], &[]),
        StageInput::none(),
    )
    .unwrap();
    assert_eq!(
        test.fs().read_file(&abs("/srv/app/backup.tar")).unwrap(),
        tar
    );
}

/// Every algorithm survives `compress` followed by `decompress`.
#[test]
fn every_algorithm_round_trips_through_the_registered_stages() {
    for algorithm in codec::ALGORITHMS {
        let test = with_files(&[("/srv/app/a.txt", LOG)]);
        let given = args(
            "compress",
            &["/srv/app/a.txt"],
            &[
                ("algorithm", choice(algorithm.name())),
                ("keep", Value::Bool(false)),
            ],
        );
        run(&test, &given, StageInput::none()).unwrap();
        let name = format!("/srv/app/a.txt.{}", algorithm.extension());
        run(
            &test,
            &args("decompress", &[&name], &[]),
            StageInput::none(),
        )
        .unwrap();
        assert_eq!(test.fs().read_file(&abs("/srv/app/a.txt")).unwrap(), LOG);
    }
}
