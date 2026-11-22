// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Exchange of control requests between a requester and the process that
//! supervises a job.

use nurt_contracts::stage::CancelReason;

use super::*;

/// Job table with a running job `j1` whose supervisor holds its lock.
fn supervised(dir: &Path) -> (JobTable, impl Sized, JobFile) {
    let table = JobTable::at(dir.join("jobs")).unwrap();
    let lock = table.lock_job("j1").unwrap();
    let file = sample_file(dir, "j1", JobState::Running);
    table.write(&file).unwrap();
    (table, lock, file)
}

fn request_files(table: &JobTable) -> usize {
    fs::read_dir(table.control_dir("j1"))
        .map(|entries| {
            entries
                .filter_map(|entry| entry.ok())
                .filter(|entry| {
                    entry
                        .file_name()
                        .to_string_lossy()
                        .ends_with(".request.json")
                })
                .count()
        })
        .unwrap_or(0)
}

#[test]
fn a_posted_request_is_taken_once_in_order() {
    let dir = tempfile::tempdir().unwrap();
    let (table, _lock, _) = supervised(dir.path());
    control::post(&table, "j1", ControlAction::Cancel).unwrap();
    control::post(&table, "j1", ControlAction::Resume).unwrap();
    let taken: Vec<ControlAction> = control::pending(&table, "j1")
        .into_iter()
        .map(|(_, action)| action)
        .collect();
    assert_eq!(taken, [ControlAction::Cancel, ControlAction::Resume]);
    assert!(control::pending(&table, "j1").is_empty());
}

#[test]
fn a_request_of_another_protocol_is_dropped() {
    let dir = tempfile::tempdir().unwrap();
    let (table, _lock, _) = supervised(dir.path());
    let control_dir = table.control_dir("j1");
    fs::create_dir_all(&control_dir).unwrap();
    fs::write(
        control_dir.join("1-0.request.json"),
        br#"{"protocol": 0, "action": {"action": "suspend"}}"#,
    )
    .unwrap();
    fs::write(control_dir.join("1-1.request.json"), b"not json").unwrap();
    assert!(control::pending(&table, "j1").is_empty());
    assert_eq!(request_files(&table), 1);
}

#[test]
fn a_failed_action_reaches_the_requester() {
    let dir = tempfile::tempdir().unwrap();
    let (ctx, _) = context(dir.path());
    let (table, _lock, _) = supervised(dir.path());
    let requester = {
        let (ctx, table) = (ctx.clone(), table.clone());
        std::thread::spawn(move || control::send(&ctx, &table, "j1", ControlAction::Suspend))
    };
    let mut taken = Vec::new();
    wait_until(|| {
        taken.extend(control::pending(&table, "j1"));
        !taken.is_empty()
    });
    assert_eq!(taken[0].1, ControlAction::Suspend);
    control::reply(
        &table,
        "j1",
        &taken[0].0,
        Err(Error::new(
            ErrorCode::Unsupported,
            "the process cannot be suspended",
        )),
    );
    let error = requester.join().unwrap().unwrap_err();
    assert_eq!(error.code, ErrorCode::Unsupported);
}

#[test]
fn a_request_to_a_job_that_ended_is_withdrawn() {
    let dir = tempfile::tempdir().unwrap();
    let (ctx, _) = context(dir.path());
    let (table, _lock, mut file) = supervised(dir.path());
    let requester = {
        let (ctx, table) = (ctx.clone(), table.clone());
        std::thread::spawn(move || control::send(&ctx, &table, "j1", ControlAction::Suspend))
    };
    wait_until(|| request_files(&table) == 1);
    file.record.state = JobState::Exited;
    table.write(&file).unwrap();
    let error = requester.join().unwrap().unwrap_err();
    assert_eq!(error.code, ErrorCode::JobState);
    assert_eq!(request_files(&table), 0);
}

#[test]
fn a_cancelled_call_stops_waiting_for_the_reply() {
    let dir = tempfile::tempdir().unwrap();
    let (ctx, _) = context(dir.path());
    let (table, _lock, _) = supervised(dir.path());
    ctx.cancel().cancel(CancelReason::Requested);
    let stop = ControlAction::Stop {
        signal: Signal::Terminate,
        grace: Duration::ZERO,
    };
    let error = control::send(&ctx, &table, "j1", stop).unwrap_err();
    assert_eq!(error.code, ErrorCode::Cancelled);
    assert_eq!(request_files(&table), 0);
}

#[test]
fn a_request_needs_a_job_in_a_matching_state() {
    let dir = tempfile::tempdir().unwrap();
    let (ctx, _) = context(dir.path());
    let (table, _lock, mut file) = supervised(dir.path());
    file.record.state = JobState::Suspended;
    table.write(&file).unwrap();
    let error = control::send(&ctx, &table, "j1", ControlAction::Suspend).unwrap_err();
    assert_eq!(error.code, ErrorCode::JobState);
    let unknown = control::send(&ctx, &table, "j9", ControlAction::Resume).unwrap_err();
    assert_eq!(unknown.code, ErrorCode::UnknownJob);
    assert_eq!(request_files(&table), 0);
}
