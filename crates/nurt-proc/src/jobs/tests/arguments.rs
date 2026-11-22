// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Arguments and static checks of the job commands, and the values the
//! commands pass on to output capture and to the running process.

use nurt_contracts::spec::StageSpec;
use nurt_contracts::stage::{CheckCtx, StaticInfo};

use super::*;

/// Catalog specification used by the static checks; the checks read only
/// the arguments.
fn spec(command: &str) -> StageSpec {
    serde_json::from_value(serde_json::json!({
        "name": command,
        "category": "run",
        "reversibility": "reversible_if_preserved",
        "effect": "process",
        "implementation": "user_program",
        "input": ["none"],
        "output": ["exec", "job"],
        "summary": "Job command.",
    }))
    .unwrap()
}

fn check(command: &str, args: &BoundArgs) -> Result<StaticInfo> {
    let dir = tempfile::tempdir().unwrap();
    let (ctx, _) = context(dir.path());
    let host = TestHost {
        info: ctx.host().info().clone(),
    };
    let config = Config::default();
    let spec = spec(command);
    let check = CheckCtx::new(&spec, &host, &config);
    stage(&components(), command).check(args, &check)
}

fn duration_value(millis: i64) -> ArgValue {
    ArgValue::Value(Value::Duration(value::Duration::from_millis(millis)))
}

fn int(number: i64) -> ArgValue {
    ArgValue::Value(Value::Int(number))
}

/// Output capture double that records every selection it renders.
struct RecordingStore {
    inner: Arc<dyn OutputStore>,
    views: Arc<Mutex<Vec<OutputView>>>,
}

impl OutputSink for RecordingStore {
    fn write(&self, stream: OutputStream, bytes: &[u8]) -> Result<()> {
        self.inner.write(stream, bytes)
    }

    fn finish(&self, stream: OutputStream) -> Result<()> {
        self.inner.finish(stream)
    }

    fn last_activity(&self) -> Option<Duration> {
        self.inner.last_activity()
    }
}

impl OutputStore for RecordingStore {
    fn render(&self, view: &OutputView, exit_code: Option<i32>) -> Result<RenderedOutput> {
        self.views.lock().unwrap().push(view.clone());
        self.inner.render(view, exit_code)
    }

    fn read_since(&self, cursor: Option<&str>, view: &OutputView) -> Result<RenderedOutput> {
        self.views.lock().unwrap().push(view.clone());
        self.inner.read_since(cursor, view)
    }
}

#[derive(Default)]
struct RecordingSpool {
    views: Arc<Mutex<Vec<OutputView>>>,
}

impl RecordingSpool {
    fn wrap(&self, inner: Arc<dyn OutputStore>) -> Arc<dyn OutputStore> {
        Arc::new(RecordingStore {
            inner,
            views: Arc::clone(&self.views),
        })
    }

    fn last(&self) -> OutputView {
        self.views.lock().unwrap().last().cloned().unwrap()
    }
}

impl SpoolFactory for RecordingSpool {
    fn create(&self, ctx: &ExecCtx, id: &str, merge_stderr: bool) -> Result<Arc<dyn OutputStore>> {
        Ok(self.wrap(FileSpool.create(ctx, id, merge_stderr)?))
    }

    fn open(&self, ctx: &ExecCtx, id: &str) -> Result<Arc<dyn OutputStore>> {
        Ok(self.wrap(FileSpool.open(ctx, id)?))
    }

    fn remove(&self, ctx: &ExecCtx, id: &str) -> Result<()> {
        FileSpool.remove(ctx, id)
    }
}

/// Process control double that records the signal and the grace time of
/// every stop.
#[derive(Default)]
struct StopRecorder {
    inner: FakeControl,
    stops: Arc<Mutex<Vec<(Signal, Duration)>>>,
}

struct RecordedStops {
    process: Box<dyn RunningProcess>,
    stops: Arc<Mutex<Vec<(Signal, Duration)>>>,
}

impl ProcessControl for StopRecorder {
    fn spawn(
        &self,
        request: SpawnRequest,
        sink: Arc<dyn OutputSink>,
        cancel: CancelToken,
    ) -> Result<Box<dyn RunningProcess>> {
        let process = self.inner.spawn(request, sink, cancel)?;
        Ok(Box::new(RecordedStops {
            process,
            stops: Arc::clone(&self.stops),
        }))
    }
}

impl RunningProcess for RecordedStops {
    fn pid(&self) -> u32 {
        self.process.pid()
    }

    fn started(&self) -> WallTime {
        self.process.started()
    }

    fn wait(&mut self, timeout: Option<Duration>) -> Result<Option<ProcessEnd>> {
        self.process.wait(timeout)
    }

    fn stop(&mut self, signal: Signal, grace: Duration) -> Result<ProcessEnd> {
        self.stops.lock().unwrap().push((signal, grace));
        self.process.stop(signal, grace)
    }

    fn suspend(&mut self) -> Result<()> {
        self.process.suspend()
    }

    fn resume(&mut self) -> Result<()> {
        self.process.resume()
    }
}

#[test]
fn jobs_waits_only_with_a_positive_wait() {
    let jobs = BoundArgs::new("jobs");
    assert!(!check("jobs", &jobs).unwrap().waits);
    let waiting = jobs.clone().with("wait", duration_value(5_000), false);
    assert!(check("jobs", &waiting).unwrap().waits);
    let zero = jobs.clone().with("wait", duration_value(0), false);
    assert!(!check("jobs", &zero).unwrap().waits);
    let view = jobs.clone().with("view", word("summary"), false);
    assert_eq!(
        check("jobs", &view).unwrap_err().code,
        ErrorCode::InvalidValue
    );
    let state = jobs.with("state", word("paused"), false);
    let error = check("jobs", &state).unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidValue);
    assert_eq!(error.found.as_deref(), Some("paused"));
}

#[test]
fn stop_checks_its_signal_and_grace_time() {
    let unknown = BoundArgs::new("stop").with("signal", word("usr1"), false);
    assert_eq!(
        check("stop", &unknown).unwrap_err().code,
        ErrorCode::InvalidValue
    );
    let grace = BoundArgs::new("stop").with("grace", text("5s"), false);
    assert_eq!(check("stop", &grace).unwrap_err().code, ErrorCode::Type);
    let negative = BoundArgs::new("stop").with("grace", duration_value(-5), false);
    assert_eq!(
        check("stop", &negative).unwrap_err().code,
        ErrorCode::InvalidValue
    );
    for signal in ["terminate", "kill", "interrupt", "hangup"] {
        let args = BoundArgs::new("stop").with("signal", word(signal), false);
        assert!(!check("stop", &args).unwrap().waits, "{signal}");
    }
    let suspend = BoundArgs::new("suspend").with("signal", word("usr1"), false);
    assert!(check("suspend", &suspend).is_ok());
    assert!(check("resume", &BoundArgs::new("resume")).is_ok());
}

#[test]
fn retry_waits_unless_its_wait_is_zero() {
    assert!(check("retry", &BoundArgs::new("retry")).unwrap().waits);
    let forever = BoundArgs::new("retry").with("wait", word("forever"), false);
    assert!(check("retry", &forever).unwrap().waits);
    let zero = BoundArgs::new("retry").with("wait", duration_value(0), false);
    assert!(!check("retry", &zero).unwrap().waits);
    let timeout = BoundArgs::new("retry").with("timeout", text("1m"), false);
    assert_eq!(check("retry", &timeout).unwrap_err().code, ErrorCode::Type);
}

#[test]
fn jobs_rejects_values_of_another_type() {
    let dir = tempfile::tempdir().unwrap();
    let (ctx, _) = context(dir.path());
    let components = components();
    let job = background(&ctx, &components, request(dir.path(), 0, 0, 2));
    wait_finished(dir.path(), &job.id);
    let by_id = BoundArgs::new("jobs")
        .with("id", text(&job.id), false)
        .with("view", word("output"), false);
    let cases = [
        ("head", text("5"), ErrorCode::Type),
        ("head", int(-1), ErrorCode::InvalidValue),
        ("tail", ArgValue::Value(Value::Bool(true)), ErrorCode::Type),
        ("context", int(-2), ErrorCode::InvalidValue),
        ("filter", int(3), ErrorCode::Type),
        ("dedupe", int(1), ErrorCode::Type),
        ("wait", text("soon"), ErrorCode::Type),
        ("wait", duration_value(-10), ErrorCode::InvalidValue),
        ("since", int(4), ErrorCode::Type),
    ];
    for (name, value, code) in cases {
        let args = by_id.clone().with(name, value, false);
        let error = read_jobs(&components, &ctx, &args).unwrap_err();
        assert_eq!(error.code, code, "{name}");
    }
    let view = by_id.with("view", word("summary"), false);
    assert_eq!(
        read_jobs(&components, &ctx, &view).unwrap_err().code,
        ErrorCode::InvalidValue
    );
    let state = BoundArgs::new("jobs").with("state", word("paused"), false);
    assert_eq!(
        read_jobs(&components, &ctx, &state).unwrap_err().code,
        ErrorCode::InvalidValue
    );
    let id = BoundArgs::new("jobs").with("id", int(1), false);
    assert_eq!(
        read_jobs(&components, &ctx, &id).unwrap_err().code,
        ErrorCode::Type
    );
}

#[test]
fn output_view_passes_head_tail_filter_context_and_dedupe() {
    let dir = tempfile::tempdir().unwrap();
    let (ctx, _) = context(dir.path());
    let spool = Arc::new(RecordingSpool::default());
    let components = Components {
        control: Arc::new(FakeControl::default()),
        spool: Arc::clone(&spool) as Arc<dyn SpoolFactory>,
        tracker: Arc::new(FakeTracker),
    };
    let job = background(&ctx, &components, request(dir.path(), 0, 0, 6));
    wait_finished(dir.path(), &job.id);
    let output = BoundArgs::new("jobs")
        .with("id", text(&job.id), false)
        .with("view", word("output"), false)
        .with("stream", word("stdout"), false)
        .with("head", int(2), false)
        .with("tail", int(3), false)
        .with(
            "filter",
            ArgValue::Value(Value::Regex(value::Regex::new("line", "i").unwrap())),
            false,
        )
        .with("context", int(1), false)
        .with("dedupe", ArgValue::Value(Value::Bool(true)), false);
    read_jobs(&components, &ctx, &output).unwrap();
    let view = spool.last();
    assert_eq!(
        (
            view.show,
            view.head,
            view.tail,
            view.filter.as_deref(),
            view.context,
            view.dedupe,
        ),
        (
            ShowMode::Stdout,
            Some(2),
            Some(3),
            Some("(?i)line"),
            1,
            true
        )
    );
    let result = BoundArgs::new("jobs")
        .with("id", text(&job.id), false)
        .with("view", word("result"), false)
        .with("stream", word("stderr"), false)
        .with("filter", text("failure"), false);
    read_jobs(&components, &ctx, &result).unwrap();
    let view = spool.last();
    assert_eq!(view.show, ShowMode::Stderr);
    assert_eq!(view.filter.as_deref(), Some("failure"));
    assert_eq!((view.head, view.tail, view.context), (None, None, 0));
    assert!(!view.dedupe);
}

#[test]
fn stop_passes_the_signal_and_grace_time_to_the_process() {
    let dir = tempfile::tempdir().unwrap();
    let (ctx, _) = context(dir.path());
    let control = Arc::new(StopRecorder::default());
    let components = Components {
        control: Arc::clone(&control) as Arc<dyn ProcessControl>,
        spool: Arc::new(FileSpool),
        tracker: Arc::new(FakeTracker),
    };
    let first = background(&ctx, &components, request(dir.path(), 10_000, 0, 0));
    let hangup = BoundArgs::new("stop")
        .with("signal", word("hangup"), false)
        .with("grace", millis(2_000), false);
    effect_on(&components, &ctx, "stop", &hangup, vec![first]).unwrap();
    let second = background(&ctx, &components, request(dir.path(), 10_000, 0, 0));
    effect_on(
        &components,
        &ctx,
        "stop",
        &BoundArgs::new("stop"),
        vec![second.clone()],
    )
    .unwrap();
    assert_eq!(
        control.stops.lock().unwrap().clone(),
        vec![
            (Signal::Hangup, Duration::from_secs(2)),
            (Signal::Terminate, Duration::from_secs(5)),
        ]
    );
    let stopped = job_file(dir.path(), &second.id).record;
    assert_eq!(stopped.state, JobState::Stopped);
    assert_eq!(stopped.signal, Some(Signal::Terminate));
}

#[test]
fn stopping_several_jobs_is_one_bulk_plan() {
    let dir = tempfile::tempdir().unwrap();
    let (ctx, _) = context(dir.path());
    let components = components();
    let jobs: Vec<Job> = (0..2)
        .map(|_| background(&ctx, &components, request(dir.path(), 10_000, 0, 0)))
        .collect();
    let stop = stage(&components, "stop");
    let input = StageInput::records(RecordStream::from_records(
        jobs.iter().cloned().map(Record::from).collect(),
    ));
    let plan = stop
        .as_effect()
        .unwrap()
        .plan(&BoundArgs::new("stop"), input, &ctx)
        .unwrap();
    assert!(plan.bulk);
    assert_eq!(plan.preview.len(), 2);
    assert_eq!(plan.irreversible.len(), 2);
    assert_eq!(
        plan.irreversible[0].detail.as_deref(),
        Some(format!("job {}", jobs[0].id).as_str())
    );
    stop_all(&ctx, &components);
}

#[test]
fn a_retry_without_wait_takes_the_configured_wait() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = Config::default();
    config.run.wait = DurationSetting::from_millis(0);
    let (ctx, _) = context_with(dir.path(), config, None);
    let components = components();
    let mut slow = request(dir.path(), 10_000, 0, 0);
    slow.spawn.limits.timeout = Some(Duration::from_millis(300));
    let job = background(&ctx, &components, slow);
    wait_finished(dir.path(), &job.id);
    let (records, outcome, _) = effect_on(
        &components,
        &ctx,
        "retry",
        &BoundArgs::new("retry"),
        vec![job.clone()],
    )
    .unwrap();
    assert!(outcome.detached);
    let retried = Job::try_from(records[0].clone()).unwrap();
    assert_eq!(retried.retry_of.as_deref(), Some(job.id.as_str()));
    let file = wait_finished(dir.path(), &retried.id);
    assert_eq!(file.record.state, JobState::TimedOut);
}

#[test]
fn a_retry_of_several_jobs_reports_each_nonzero_exit_code() {
    let dir = tempfile::tempdir().unwrap();
    let (ctx, _) = context(dir.path());
    let components = components();
    let failed = background(&ctx, &components, request(dir.path(), 0, 3, 0));
    let passed = background(&ctx, &components, request(dir.path(), 0, 0, 1));
    wait_finished(dir.path(), &failed.id);
    wait_finished(dir.path(), &passed.id);
    let args = BoundArgs::new("retry").with("wait", word("forever"), false);
    let (records, outcome, _) = effect_on(
        &components,
        &ctx,
        "retry",
        &args,
        vec![failed.clone(), passed],
    )
    .unwrap();
    assert_eq!(records.len(), 2);
    let notes: Vec<_> = outcome
        .notable
        .iter()
        .filter(|entry| entry.code == Notable::ExitNonzero)
        .collect();
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].count, Some(3));
    assert_eq!(
        notes[0].detail.as_deref(),
        Some(format!("retry of job {}", failed.id).as_str())
    );
}

#[test]
fn label_and_description_reach_the_job_record() {
    let dir = tempfile::tempdir().unwrap();
    let (ctx, _) = context(dir.path());
    let components = components();
    let mut described = request(dir.path(), 0, 0, 0);
    described.description = Some("release build of the workspace".to_owned());
    let job = background(&ctx, &components, described);
    let file = wait_finished(dir.path(), &job.id);
    assert_eq!(file.record.label.as_deref(), Some("build"));
    assert_eq!(
        file.record.description.as_deref(),
        Some("release build of the workspace")
    );
    assert_eq!(file.record.session.as_deref(), Some("s1"));
    let (jobs, _) = read_jobs(
        &components,
        &ctx,
        &BoundArgs::new("jobs").with("id", text(&job.id), false),
    )
    .unwrap();
    assert_eq!(jobs[0].label.as_deref(), Some("build"));
    assert!(jobs[0].pid >= 4000);
}

#[test]
fn status_of_a_finished_job_reports_both_streams() {
    let dir = tempfile::tempdir().unwrap();
    let (ctx, _) = context(dir.path());
    let components = components();
    let job = background(&ctx, &components, request(dir.path(), 0, 2, 3));
    wait_finished(dir.path(), &job.id);
    let args = BoundArgs::new("jobs").with("id", text(&job.id), false);
    let (jobs, _) = read_jobs(&components, &ctx, &args).unwrap();
    assert_eq!(jobs[0].state, JobState::Failed);
    assert_eq!(jobs[0].exit_code, Some(2));
    assert_eq!(jobs[0].stdout_lines, Some(3));
    assert_eq!(jobs[0].stderr.as_deref(), Some("failure\n"));
}

#[test]
fn a_retry_of_a_command_with_an_external_effect_is_irreversible() {
    let dir = tempfile::tempdir().unwrap();
    let (ctx, _) = context(dir.path());
    let components = components();
    let mut publishing = request(dir.path(), 0, 0, 0);
    publishing.external_effect = vec!["git push".to_owned(), "npm publish".to_owned()];
    let job = background(&ctx, &components, publishing);
    wait_finished(dir.path(), &job.id);
    let args = BoundArgs::new("retry").with("wait", word("forever"), false);
    let (_, outcome, _) = effect_on(&components, &ctx, "retry", &args, vec![job]).unwrap();
    assert_eq!(outcome.irreversible.len(), 1);
    assert_eq!(outcome.irreversible[0].reason, Irreversible::ExternalEffect);
    assert_eq!(
        outcome.irreversible[0].detail.as_deref(),
        Some("git push, npm publish")
    );
}
