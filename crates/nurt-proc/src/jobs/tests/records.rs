// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Job identifiers, record times, expiry of finished jobs and the output
//! events of a running job.

use std::sync::atomic::{AtomicUsize, Ordering};

use super::super::events::OutputEvents;
use super::super::record::{duration_value, with_elapsed};
use super::super::table::{instant, number, wall_time};
use super::*;

#[test]
fn identifiers_are_a_letter_and_a_number() {
    assert_eq!(number("j1"), Some(1));
    assert_eq!(number("j007"), Some(7));
    assert_eq!(number("j18446744073709551615"), Some(u64::MAX));
    for invalid in [
        "",
        "j",
        "7",
        "J7",
        "j7x",
        "j-1",
        "j+1",
        "j 1",
        "j1.input",
        "../j1",
        "j١",
        "j18446744073709551616",
    ] {
        assert_eq!(number(invalid), None, "{invalid:?}");
    }
}

#[test]
fn record_times_are_clamped_to_the_range_of_an_instant() {
    let normal = WallTime::from_unix_nanos(1_700_000_000_123_456_789);
    assert_eq!(wall_time(instant(normal)), normal);
    assert_eq!(
        instant(WallTime::from_unix_nanos(i128::MAX)).as_unix_nanos(),
        i64::MAX
    );
    assert_eq!(
        instant(WallTime::from_unix_nanos(i128::MIN)).as_unix_nanos(),
        i64::MIN
    );
    let before_epoch = WallTime::from_unix_nanos(-5);
    assert_eq!(wall_time(instant(before_epoch)), before_epoch);
}

#[test]
fn durations_that_do_not_fit_a_record_saturate() {
    assert_eq!(
        duration_value(Duration::from_millis(1500)),
        value::Duration::from_millis(1500)
    );
    assert_eq!(
        duration_value(Duration::MAX),
        value::Duration::from_millis(i64::MAX)
    );
}

#[test]
fn the_elapsed_time_of_a_running_job_is_measured_at_the_given_time() {
    let started = WallTime::from_unix_nanos(1_000_000_000_000);
    let running = Job {
        started: instant(started),
        ..Job::default()
    };
    let now = started.saturating_add(Duration::from_secs(90));
    assert_eq!(
        with_elapsed(&running, now).elapsed,
        value::Duration::from_millis(90_000)
    );
    assert_eq!(
        with_elapsed(&running, WallTime::UNIX_EPOCH).elapsed,
        value::Duration::ZERO
    );
}

#[test]
fn a_finished_job_keeps_its_elapsed_time() {
    let started = WallTime::from_unix_nanos(1_000_000_000_000);
    let finished = Job {
        started: instant(started),
        finished: Some(instant(started.saturating_add(Duration::from_secs(5)))),
        elapsed: value::Duration::from_millis(5_000),
        ..Job::default()
    };
    let much_later = started.saturating_add(Duration::from_secs(86_400));
    assert_eq!(with_elapsed(&finished, much_later), finished);
}

#[test]
fn finished_jobs_expire_only_after_the_lifetime() {
    let dir = tempfile::tempdir().unwrap();
    let table = JobTable::at(dir.path().join("jobs")).unwrap();
    let ended = WallTime::from_unix_nanos(5_000_000_000_000);
    let mut finished = sample_file(dir.path(), "j1", JobState::Exited);
    finished.record.finished = Some(instant(ended));
    table.write(&finished).unwrap();
    let _lock = table.lock_job("j2").unwrap();
    table
        .write(&sample_file(dir.path(), "j2", JobState::Running))
        .unwrap();
    let ttl = Duration::from_secs(10);
    let on_time = ended.saturating_add(ttl);
    assert!(table.expired(on_time, ttl).unwrap().is_empty());
    let late = on_time.saturating_add(Duration::from_nanos(1));
    assert_eq!(table.expired(late, ttl).unwrap(), ["j1"]);
    assert!(table.expired(WallTime::UNIX_EPOCH, ttl).unwrap().is_empty());
}

/// Output store double that counts the reads and fails them on request.
struct CountingStore {
    reads: AtomicUsize,
    failure: Option<&'static str>,
}

impl OutputSink for CountingStore {
    fn write(&self, _stream: OutputStream, _bytes: &[u8]) -> Result<()> {
        Ok(())
    }

    fn finish(&self, _stream: OutputStream) -> Result<()> {
        Ok(())
    }

    fn last_activity(&self) -> Option<Duration> {
        None
    }
}

impl OutputStore for CountingStore {
    fn render(&self, _view: &OutputView, _exit_code: Option<i32>) -> Result<RenderedOutput> {
        Ok(RenderedOutput::default())
    }

    fn read_since(&self, _cursor: Option<&str>, _view: &OutputView) -> Result<RenderedOutput> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        match self.failure {
            Some(message) => Err(Error::new(ErrorCode::Io, message)),
            None => Ok(RenderedOutput::default()),
        }
    }
}

#[test]
fn output_events_are_read_once_per_interval_unless_forced() {
    let dir = tempfile::tempdir().unwrap();
    let (ctx, _) = context(dir.path());
    let store = CountingStore {
        reads: AtomicUsize::new(0),
        failure: None,
    };
    let mut events = OutputEvents::new("j1", None);
    events.pump(&ctx, &store, false);
    events.pump(&ctx, &store, false);
    assert_eq!(store.reads.load(Ordering::SeqCst), 1);
    events.pump(&ctx, &store, true);
    assert_eq!(store.reads.load(Ordering::SeqCst), 2);
}

#[test]
fn a_failed_read_ends_the_output_events_after_one_log_event() {
    let dir = tempfile::tempdir().unwrap();
    let (ctx, recorder) = context(dir.path());
    let store = CountingStore {
        reads: AtomicUsize::new(0),
        failure: Some("the spool is gone"),
    };
    let mut events = OutputEvents::new("j1", None);
    events.pump(&ctx, &store, true);
    events.pump(&ctx, &store, true);
    assert_eq!(store.reads.load(Ordering::SeqCst), 1);
    let logs: Vec<String> = recorder
        .events()
        .into_iter()
        .filter_map(|event| match event {
            Event::Log(log) => Some(log.message),
            _ => None,
        })
        .collect();
    assert_eq!(logs.len(), 1);
    assert!(logs[0].contains("j1") && logs[0].contains("the spool is gone"));
}

#[test]
fn the_time_limit_of_a_run_given_to_the_program_covers_the_preparation() {
    let dir = tempfile::tempdir().unwrap();
    let (ctx, _) = context(dir.path());
    let control = Arc::new(FakeControl::default());
    let components = components_with(control.clone());
    let RunWait::Finished(_) = run(&ctx, &components, request(dir.path(), 0, 0, 0), None).unwrap()
    else {
        panic!("the run moved to the background");
    };
    let spawned = control.spawned.lock().unwrap();
    let given = spawned[0].limits.timeout.unwrap();
    assert!(given <= Duration::from_secs(60));
    assert!(given > Duration::from_secs(50), "{given:?}");
}

#[test]
fn a_stored_request_without_a_scan_time_limit_is_still_read() {
    let dir = tempfile::tempdir().unwrap();
    let mut stored = request(dir.path(), 0, 0, 0);
    stored.track.timeout = Some(Duration::from_secs(7));
    let mut json = serde_json::to_value(StoredRequest(stored.clone())).unwrap();
    assert!(json["track"]["timeout"].is_object() || json["track"]["timeout"].is_array());
    json["track"].as_object_mut().unwrap().remove("timeout");
    let read: StoredRequest = serde_json::from_value(json).unwrap();
    assert_eq!(read.0.track.timeout, None);
    stored.track.timeout = None;
    assert_eq!(read.0, stored);
}
