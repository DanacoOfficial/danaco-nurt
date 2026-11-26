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
fn requests_are_taken_in_the_order_of_their_counters() {
    let dir = tempfile::tempdir().unwrap();
    let (table, _lock, _) = supervised(dir.path());
    let control_dir = table.control_dir("j1");
    fs::create_dir_all(&control_dir).unwrap();
    let body =
        |action: &str| format!(r#"{{"protocol": {PROTOCOL}, "action": {{"action": "{action}"}}}}"#);
    for (name, action) in [
        ("42-10", "resume"),
        ("42-9", "suspend"),
        ("7-100", "cancel"),
        ("42-100", "cancel"),
    ] {
        fs::write(
            control_dir.join(format!("{name}.request.json")),
            body(action),
        )
        .unwrap();
    }
    let taken: Vec<(String, ControlAction)> = control::pending(&table, "j1");
    let names: Vec<&str> = taken.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(names, ["7-100", "42-9", "42-10", "42-100"]);
    assert_eq!(taken[1].1, ControlAction::Suspend);
    assert_eq!(taken[2].1, ControlAction::Resume);
}

#[test]
fn request_names_without_a_counter_are_taken_last_in_text_order() {
    let order = control::request_order;
    assert_eq!(order("12-3"), (12, 3));
    assert!(order("12-3") < order("12-10"));
    assert!(order("9-100") < order("12-1"));
    assert_eq!(order("odd"), (u64::MAX, u64::MAX));
    assert_eq!(order("a-b"), (u64::MAX, u64::MAX));
    assert!(order("12-3") < order("odd"));
}

#[test]
fn start_requests_are_listed_in_the_order_of_their_counters() {
    let dir = tempfile::tempdir().unwrap();
    let table = JobTable::at(dir.path().join("jobs")).unwrap();
    for name in ["5-10", "5-9", "5-100", "5-1"] {
        fs::write(table.inbox().join(format!("{name}.request.json")), b"{}").unwrap();
    }
    fs::write(table.inbox().join("5-0.reply.json"), b"{}").unwrap();
    let names: Vec<String> = super::super::host::inbox_requests(&table)
        .unwrap()
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    assert_eq!(names, ["5-1", "5-9", "5-10", "5-100"]);
}

#[test]
fn a_request_belongs_to_whoever_removes_its_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("1-0.request.json");
    fs::write(&path, b"{}").unwrap();
    assert!(control::withdraw(&path).unwrap());
    assert!(!path.exists());
    assert!(!control::withdraw(&path).unwrap());
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
fn a_grace_time_beyond_the_clock_does_not_overflow_the_deadline() {
    let dir = tempfile::tempdir().unwrap();
    let (ctx, _) = context(dir.path());
    let (table, _lock, _) = supervised(dir.path());
    ctx.cancel().cancel(CancelReason::Requested);
    let stop = ControlAction::Stop {
        signal: Signal::Terminate,
        grace: Duration::MAX,
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
