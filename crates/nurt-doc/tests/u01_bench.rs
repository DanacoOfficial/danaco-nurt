// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-doc/tests/u01_bench.rs
// Responsibility:   Time and peak memory of opening a 10 MiB document and changing one value.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Time and peak memory of opening a 10 MiB document and changing one value
//! through the guards. Run with `--ignored --nocapture`; the measurement is
//! written to standard error.
#![cfg(feature = "testing")]

use std::fmt::Write as _;
use std::io::Write as _;
use std::time::{Duration, Instant};

use nurt_contracts::keypath::KeyPath;
use nurt_contracts::services::DocFormatId;
use nurt_contracts::value::Value;
use nurt_doc::model::SetOptions;
use nurt_doc::registry::{DocOptions, open_with};
use nurt_doc::testing::KeyValueFormat;

/// Size of the generated document.
const DOCUMENT_BYTES: usize = 10 * 1024 * 1024;
/// Upper bound of opening and one change, for unoptimized builds.
const TIME_LIMIT: Duration = Duration::from_secs(120);

/// Peak resident memory of the process in KiB, where the system reports it.
fn peak_memory_kib() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    status
        .lines()
        .find_map(|line| line.strip_prefix("VmHWM:"))?
        .trim()
        .trim_end_matches("kB")
        .trim()
        .parse()
        .ok()
}

#[test]
#[ignore = "benchmark of a 10 MiB document"]
fn open_and_set_on_ten_mebibyte_document() {
    let mut text = String::with_capacity(DOCUMENT_BYTES + 64);
    let mut entries = 0_usize;
    while text.len() < DOCUMENT_BYTES {
        writeln!(text, "key{entries} = value{entries}").expect("writing to a string succeeds");
        entries += 1;
    }
    let format = KeyValueFormat::new(DocFormatId::new("kv").expect("valid identifier"));
    let middle = KeyPath::parse(&format!("key{}", entries / 2)).expect("valid key path");
    let start = Instant::now();
    let mut document =
        open_with(&format, text.as_bytes(), &DocOptions::default()).expect("document opens");
    let opened = start.elapsed();
    let report = document
        .handle
        .set(&middle, &Value::Int(1), &SetOptions::default())
        .expect("value is set");
    let total = start.elapsed();
    assert_eq!(report.changed.len(), 1);
    assert!(document.bytes().len() < text.len());
    let peak =
        peak_memory_kib().map_or_else(|| "not reported".to_owned(), |kib| format!("{kib} KiB"));
    writeln!(
        std::io::stderr(),
        "{} bytes, {entries} entries: open {opened:?}, open and set {total:?}, peak resident memory {peak}",
        text.len()
    )
    .expect("standard error is writable");
    assert!(total < TIME_LIMIT, "open and set took {total:?}");
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
