// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-proc/tests/spawn_host.rs
// Responsibility:   Process host service and the stage list, through the public interface.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Process host service and the stage list, through the public interface.

mod support;

use std::io::{Read, Write};
use std::time::Duration;

use nurt_contracts::error::ErrorCode;
use nurt_contracts::services::ServiceSpawnSpec;
use nurt_contracts::value::RecordKind;

fn service(args: &[&str], dir: &std::path::Path) -> ServiceSpawnSpec {
    ServiceSpawnSpec {
        program: support::fixture_path().to_string_lossy().into_owned(),
        args: args.iter().map(|arg| (*arg).to_owned()).collect(),
        cwd: dir.to_path_buf(),
        env: Vec::new(),
        inherit_env: true,
        temp_dir: None,
    }
}

#[test]
fn the_stage_list_holds_run_for_files_and_matches() {
    let stages = nurt_proc::stages();
    let run = stages
        .iter()
        .map(|stage| stage.descriptor())
        .find(|descriptor| descriptor.command == "run")
        .expect("the stage list has no run stage");
    assert!(run.source);
    assert!(run.input.accepts(RecordKind::Entry));
    assert!(run.input.accepts(RecordKind::Match));
}

#[test]
fn resolves_a_program_path_and_reports_a_missing_program() {
    let dir = tempfile::tempdir().unwrap();
    let host = nurt_proc::process_host();
    let fixture = support::fixture_path();
    assert_eq!(
        host.resolve_program(&fixture.to_string_lossy(), dir.path())
            .unwrap(),
        fixture
    );
    let error = host
        .resolve_program("no-such-program-for-tests", dir.path())
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::ExternalMissing);
}

#[test]
fn a_helper_process_receives_its_arguments_and_input() {
    let dir = tempfile::tempdir().unwrap();
    let host = nurt_proc::process_host();
    let mut child = host
        .spawn_service(&service(&["print-argv", "a b", "&|"], dir.path()))
        .unwrap();
    let mut text = String::new();
    child
        .take_stdout()
        .unwrap()
        .read_to_string(&mut text)
        .unwrap();
    let argv: Vec<String> = serde_json::from_str(text.trim()).unwrap();
    assert_eq!(argv, vec!["a b".to_owned(), "&|".to_owned()]);
    let exit = child.wait_timeout(Duration::from_secs(10)).unwrap();
    assert_eq!(exit.and_then(|exit| exit.exit_code), Some(0));

    let mut counter = host
        .spawn_service(&service(&["count-input"], dir.path()))
        .unwrap();
    let mut stdin = counter.take_stdin().unwrap();
    stdin.write_all(b"1234").unwrap();
    drop(stdin);
    let mut count = String::new();
    counter
        .take_stdout()
        .unwrap()
        .read_to_string(&mut count)
        .unwrap();
    assert_eq!(count.trim(), "4");
}

#[test]
fn dropping_a_helper_process_stops_it() {
    let dir = tempfile::tempdir().unwrap();
    let host = nurt_proc::process_host();
    let mut child = host
        .spawn_service(&service(&["exit-with", "0", "30000"], dir.path()))
        .unwrap();
    assert!(child.try_wait().unwrap().is_none());
    let stdout = child.take_stdout().unwrap();
    drop(child);
    let mut rest = Vec::new();
    let mut stdout = stdout;
    stdout.read_to_end(&mut rest).unwrap();
    assert!(rest.is_empty());
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
