// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-proc::jobs::tests::tools
// Responsibility:   Background jobs of tool runs and the final records built by the finishers of the test and analysis commands.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Background jobs of tool runs and the final records built by the
//! finishers of the test and analysis commands.

use crate::api::{JobFinisher, JobFinishers, ToolKind, ToolResult};

use super::*;

/// Finisher that records the exit code of the tool and the run identifier
/// of its data, or fails as an unreadable report.
struct ReportFinisher {
    fails: bool,
}

impl JobFinisher for ReportFinisher {
    fn finish(
        &self,
        _ctx: &ExecCtx,
        result: &ToolResult,
        data: Option<&serde_json::Value>,
    ) -> Result<Record> {
        if self.fails {
            return Err(Error::new(
                ErrorCode::StageFailed,
                "the report of the tool cannot be read",
            ));
        }
        let run = data
            .and_then(|data| data.get("run"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        Ok(Record::new(RecordKind::Map)
            .with("exit_code", Value::Int(result.exec.exit_code))
            .with("run", Value::Text(run.to_owned())))
    }
}

fn finishers(kind: ToolKind, fails: bool) -> JobFinishers {
    let mut finishers = JobFinishers::new();
    finishers.insert(kind, Arc::new(ReportFinisher { fails }));
    finishers
}

fn tool_context(dir: &Path, finishers: JobFinishers) -> (ExecCtx, Arc<Recorder>) {
    let (plain, _) = context(dir);
    let recorder = Arc::new(Recorder::default());
    let mut extensions = Extensions::new();
    extensions.insert::<JobFinishers>(Arc::new(finishers));
    let ctx = ExecCtx::builder(
        Arc::new(SystemClock {
            origin: Instant::now(),
        }),
        Arc::new(TestHost {
            info: plain.host().info().clone(),
        }),
    )
    .state_dir(dir.to_path_buf())
    .config(Arc::new(Config::default()))
    .events(recorder.clone())
    .extensions(extensions)
    .build();
    (ctx, recorder)
}

fn tool_request(dir: &Path, kind: RunKind, code: i32) -> RunRequest {
    let mut request = request(dir, 0, code, 2);
    request.kind = kind;
    request.finish_data = Some(serde_json::json!({"run": "run-4"}));
    request
}

#[test]
fn a_background_test_job_ends_with_the_record_of_its_finisher() {
    let dir = tempfile::tempdir().unwrap();
    let (ctx, _) = tool_context(dir.path(), finishers(ToolKind::Test, false));
    let components = components();
    let job = background(
        &ctx,
        &components,
        tool_request(dir.path(), RunKind::Test, 3),
    );
    let file = wait_finished(dir.path(), &job.id);
    let record = file.final_record.clone().unwrap();
    assert_eq!(record.get("exit_code"), Some(&Value::Int(3)));
    assert_eq!(record.get("run"), Some(&Value::Text("run-4".to_owned())));
    let args = BoundArgs::new("jobs")
        .with("id", text(&job.id), false)
        .with("view", word("result"), false);
    let (jobs, header) = read_jobs(&components, &ctx, &args).unwrap();
    assert_eq!(jobs[0].state, JobState::Failed);
    assert_eq!(header, Some(record));
}

#[test]
fn a_finisher_error_leaves_the_job_without_a_final_record() {
    let dir = tempfile::tempdir().unwrap();
    let (ctx, recorder) = tool_context(dir.path(), finishers(ToolKind::Analyze, true));
    let job = background(
        &ctx,
        &components(),
        tool_request(dir.path(), RunKind::Analyze, 0),
    );
    let file = wait_finished(dir.path(), &job.id);
    assert_eq!(file.final_record, None);
    assert_eq!(file.record.state, JobState::Exited);
    assert!(recorder.events().iter().any(|event| matches!(
        event,
        Event::Log(log) if log.message.contains("the report of the tool cannot be read")
    )));
}

#[test]
fn only_tool_jobs_with_a_finisher_get_a_final_record() {
    let dir = tempfile::tempdir().unwrap();
    let (ctx, _) = tool_context(dir.path(), finishers(ToolKind::Analyze, false));
    let components = components();
    let plain = background(&ctx, &components, tool_request(dir.path(), RunKind::Run, 0));
    let test = background(
        &ctx,
        &components,
        tool_request(dir.path(), RunKind::Test, 0),
    );
    let analyze = background(
        &ctx,
        &components,
        tool_request(dir.path(), RunKind::Analyze, 0),
    );
    assert_eq!(wait_finished(dir.path(), &plain.id).final_record, None);
    assert_eq!(wait_finished(dir.path(), &test.id).final_record, None);
    assert!(
        wait_finished(dir.path(), &analyze.id)
            .final_record
            .is_some()
    );
    let registered = finishers(ToolKind::Test, false);
    assert!(registered.get(ToolKind::Test).is_some());
    assert!(registered.get(ToolKind::Analyze).is_none());
}

#[test]
fn a_tool_run_waited_for_to_its_end_has_no_final_record() {
    let dir = tempfile::tempdir().unwrap();
    let (ctx, _) = tool_context(dir.path(), finishers(ToolKind::Test, false));
    let RunWait::Finished(result) = run(
        &ctx,
        &components(),
        tool_request(dir.path(), RunKind::Test, 0),
        None,
    )
    .unwrap() else {
        panic!("the run moved to the background");
    };
    assert_eq!(result.job, None);
    let table = JobTable::at(dir.path().join("jobs")).unwrap();
    let id = table.ids().unwrap().last().cloned().unwrap();
    let file = job_file(dir.path(), &id);
    assert_eq!(file.final_record, None);
    assert_eq!(file.request.0.kind, RunKind::Test);
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
