// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-programs/tests/perf.rs
// Responsibility:   Timing targets of the `program` commands on a synthetic system: 2000.
// Dependencies:     nurt_contracts, support
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Timing targets of the `program` commands on a synthetic system: 2000
//! executables in `PATH`, 300 dpkg packages and 300 compressed manual pages.
//!
//! The limits are the targets of release builds; debug builds multiply them
//! by [`DEBUG_FACTOR`]. A failed assertion reports the measured time.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code fails loudly on broken fixtures"
)]

mod support;

use std::io::Write;
use std::sync::Arc;
use std::time::{Duration, Instant};

use nurt_contracts::stage::BoundArgs;
use nurt_contracts::value::Value;
use nurt_programs::index::{IndexCache, IndexOptions};
use nurt_programs::stages::{DESCRIBE, FIND, ProgramsSetup, SEARCH};
use support::{Fixture, args, run, text, texts};

/// Executables in `/usr/bin`.
const USR_BIN_PROGRAMS: usize = 1500;
/// Executables in `/usr/local/bin`.
const LOCAL_BIN_PROGRAMS: usize = 500;
/// Packages of the dpkg status file; each provides five executables of
/// `/usr/bin`.
const PACKAGES: usize = 300;
/// Executables per package.
const PER_PACKAGE: usize = 5;
/// Manual pages in section 1.
const MAN_PAGES: usize = 300;
/// Multiplier of the limits in debug builds.
const DEBUG_FACTOR: u32 = 25;

/// Targets of release builds.
const FIND_TARGET: Duration = Duration::from_millis(5);
const COLD_SEARCH_TARGET: Duration = Duration::from_secs(1);
const DISK_CACHE_SEARCH_TARGET: Duration = Duration::from_millis(40);
const WARM_SEARCH_TARGET: Duration = Duration::from_millis(10);
const DESCRIBE_TARGET: Duration = Duration::from_millis(10);

fn limit(target: Duration) -> Duration {
    if cfg!(debug_assertions) {
        target * DEBUG_FACTOR
    } else {
        target
    }
}

fn program_name(index: usize) -> String {
    format!("tool{index:04}")
}

fn gzip_bytes(text: &[u8]) -> Vec<u8> {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    encoder.write_all(text).unwrap();
    encoder.finish().unwrap()
}

fn system() -> Fixture {
    let fixture = Fixture::new();
    for index in 0..USR_BIN_PROGRAMS {
        fixture.executable(&format!("/usr/bin/{}", program_name(index)));
    }
    for index in USR_BIN_PROGRAMS..USR_BIN_PROGRAMS + LOCAL_BIN_PROGRAMS {
        fixture.executable(&format!("/usr/local/bin/{}", program_name(index)));
    }
    let mut status = String::new();
    for package in 0..PACKAGES {
        status.push_str(&format!(
            "Package: suite{package:03}\nStatus: install ok installed\nVersion: 1.{package}-1\nSection: utils\nDescription: Utility suite number {package} for archive and network tasks\n\n"
        ));
        let files: String = (0..PER_PACKAGE)
            .map(|offset| {
                format!(
                    "/usr/bin/{}\n",
                    program_name(package * PER_PACKAGE + offset)
                )
            })
            .collect();
        fixture.write(
            &format!("/var/lib/dpkg/info/suite{package:03}.list"),
            files.as_bytes(),
        );
    }
    fixture.write("/var/lib/dpkg/status", status.as_bytes());
    for index in 0..MAN_PAGES {
        let name = program_name(index);
        let page =
            format!(".TH X 1\n.SH NAME\n{name} \\- converts video streams, variant {index}\n");
        fixture.write(
            &format!("/usr/share/man/man1/{name}.1.gz"),
            &gzip_bytes(page.as_bytes()),
        );
    }
    fixture
}

fn setup(fixture: &Fixture, cache: Arc<IndexCache>) -> ProgramsSetup {
    let mut setup = fixture.setup(Vec::new());
    setup.options = Some(IndexOptions {
        cache_dir: Some(fixture.path().join("cache")),
        ..IndexOptions::default()
    });
    setup.cache = Some(cache);
    setup
}

fn timed(setup: &ProgramsSetup, fixture: &Fixture, bound: BoundArgs) -> (Duration, usize) {
    let ctx = fixture.ctx();
    let start = Instant::now();
    let records = run(setup, &ctx, bound).unwrap();
    (start.elapsed(), records.len())
}

fn check(scenario: &str, elapsed: Duration, target: Duration) {
    let allowed = limit(target);
    assert!(
        elapsed <= allowed,
        "{scenario}: {elapsed:?} exceeds {allowed:?}"
    );
}

fn search_args() -> BoundArgs {
    args(
        SEARCH,
        &[("query", text("convert video")), ("limit", Value::Int(15))],
    )
}

#[test]
fn find_five_names_within_target() {
    let fixture = system();
    let setup = setup(&fixture, Arc::new(IndexCache::new()));
    let names = [
        program_name(1),
        program_name(700),
        program_name(1600),
        program_name(1999),
        "absent".to_owned(),
    ];
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    let (elapsed, count) = timed(&setup, &fixture, args(FIND, &[("name", texts(&names))]));
    assert_eq!(count, names.len());
    check("program.find of five names", elapsed, FIND_TARGET);
}

#[test]
fn search_cold_disk_cache_and_warm_within_targets() {
    let fixture = system();
    let cold = setup(&fixture, Arc::new(IndexCache::new()));
    let (elapsed, count) = timed(&cold, &fixture, search_args());
    assert_eq!(count, 15);
    check(
        "first program.search without cache",
        elapsed,
        COLD_SEARCH_TARGET,
    );

    let (elapsed, _) = timed(&cold, &fixture, search_args());
    check(
        "program.search in the same process",
        elapsed,
        WARM_SEARCH_TARGET,
    );

    let (elapsed, _) = timed(
        &cold,
        &fixture,
        args(DESCRIBE, &[("name", text(&program_name(42)))]),
    );
    check(
        "program.describe in the same process",
        elapsed,
        DESCRIBE_TARGET,
    );

    let new_process = setup(&fixture, Arc::new(IndexCache::new()));
    let (elapsed, count) = timed(&new_process, &fixture, search_args());
    assert_eq!(count, 15);
    check(
        "program.search with the disk cache in a new process",
        elapsed,
        DISK_CACHE_SEARCH_TARGET,
    );
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
