// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Tests of the `run` stage with a run host that starts the test program
//! directly.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use nurt_contracts::config::Config;
use nurt_contracts::envelope::RequestOptions;
use nurt_contracts::error::{ErrorCode, Result};
use nurt_contracts::record::{Exec, Job, JobState, KilledBy};
use nurt_contracts::report::Notable;
use nurt_contracts::spec::StageSpec;
use nurt_contracts::stage::{
    ArgValue, BoundArgs, CheckCtx, EffectOutcome, EffectStage, ExecCtx, RecordStream, StageData,
    StageImpl, StageInput,
};
use nurt_contracts::value::{self, Fields, Record, RecordKind, Value};

use super::RunStage;
use crate::api::{ToolKind, ToolOutcome, ToolRun};
use crate::iface::{RunHost, RunKind, StdinSource};
use crate::spawn::testing::{DirectRunHost, HostMode, NullTx, TestHost, context, fixture_path};

fn fixture() -> String {
    fixture_path().to_string_lossy().into_owned()
}

fn text(value: &str) -> Value {
    Value::Text(value.to_owned())
}

fn path(value: &Path) -> Value {
    Value::Path(value.to_string_lossy().into_owned())
}

/// Arguments of `run` with the test program as the program.
fn run_args(mode: &[&str], cwd: &Path, extra: Vec<(&str, Value)>) -> BoundArgs {
    let mut items = vec![text(&fixture())];
    items.extend(mode.iter().map(|arg| text(arg)));
    let mut args = BoundArgs::new("run")
        .with("command", ArgValue::Value(Value::List(items)), false)
        .with("cwd", ArgValue::Value(path(cwd)), false);
    for (name, value) in extra {
        args.insert(name, ArgValue::Value(value), false);
    }
    args
}

fn ctx(vars: &[(&str, &str)]) -> ExecCtx {
    context(
        TestHost::current(vars),
        Config::default(),
        RequestOptions::default(),
    )
}

fn records(outcome: EffectOutcome<'_>) -> Vec<Record> {
    match outcome.output.data {
        StageData::Records(stream) => stream.map(|record| record.unwrap()).collect(),
        _ => Vec::new(),
    }
}

/// Plans and applies the stage; returns the outcome as owned parts.
fn execute(
    host: &Arc<DirectRunHost>,
    args: &BoundArgs,
    input: StageInput<'_>,
    ctx: &ExecCtx,
) -> Result<(Vec<Record>, Vec<Notable>, bool)> {
    let stage = RunStage::with_host(Arc::clone(host) as Arc<dyn RunHost>);
    let plan = stage.plan(args, input, ctx)?;
    let mut tx = NullTx::default();
    let outcome = stage.apply(args, plan, &mut tx, ctx)?;
    let notable = outcome.notable.iter().map(|note| note.code).collect();
    let detached = outcome.detached;
    Ok((records(outcome), notable, detached))
}

fn exec_of(records: &[Record]) -> Exec {
    Exec::try_from(records[0].clone()).unwrap()
}

fn entries(paths: &[&str]) -> StageInput<'static> {
    let records: Vec<Record> = paths
        .iter()
        .map(|path| Record::new(RecordKind::Entry).with("path", Value::Path((*path).to_owned())))
        .collect();
    StageInput::records(RecordStream::from_records(records))
}

mod arguments {
    use super::*;

    #[test]
    fn the_program_receives_every_argument_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let sent = [
            "a b",
            "",
            "quote\"d",
            "back\\slash\\",
            "naïve",
            "日本語",
            "--x=1",
            "$HOME",
            "*",
            "'",
            "&|<>^%!",
        ];
        let mut mode = vec!["print-argv"];
        mode.extend(sent);
        let host = DirectRunHost::new(HostMode::Run);
        let (records, _, _) = execute(
            &host,
            &run_args(&mode, dir.path(), Vec::new()),
            StageInput::none(),
            &ctx(&[]),
        )
        .unwrap();
        let exec = exec_of(&records);
        let received: Vec<String> = serde_json::from_str(exec.stdout.trim()).unwrap();
        assert_eq!(received, sent);
        assert_eq!(exec.exit_code, 0);
        assert_eq!(
            exec.program.as_str(),
            crate::spawn::record_path(&fixture_path(), crate::spawn::Platform::current()).as_str()
        );
        assert_eq!(exec.command[1], "print-argv");
        assert_eq!(exec.job.as_deref(), Some("j1"));
    }

    #[test]
    fn the_program_runs_in_the_working_directory() {
        let dir = tempfile::tempdir().unwrap();
        let host = DirectRunHost::new(HostMode::Run);
        let (records, _, _) = execute(
            &host,
            &run_args(&["print-cwd"], dir.path(), Vec::new()),
            StageInput::none(),
            &ctx(&[]),
        )
        .unwrap();
        let printed = std::fs::canonicalize(exec_of(&records).stdout.trim()).unwrap();
        assert_eq!(printed, std::fs::canonicalize(dir.path()).unwrap());
    }

    #[test]
    fn a_relative_working_directory_is_taken_against_base() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        let options = RequestOptions {
            base: Some(dir.path().to_string_lossy().into_owned()),
            ..RequestOptions::default()
        };
        let ctx = context(TestHost::current(&[]), Config::default(), options);
        let args = run_args(&["print-cwd"], Path::new("sub"), Vec::new());
        let host = DirectRunHost::new(HostMode::Run);
        execute(&host, &args, StageInput::none(), &ctx).unwrap();
        let (request, _) = &host.recorded()[0];
        assert_eq!(request.spawn.cwd, dir.path().join("sub"));
        assert_eq!(request.track.watch, vec![dir.path().join("sub")]);
    }

    #[test]
    fn a_missing_working_directory_is_reported_before_the_start() {
        let dir = tempfile::tempdir().unwrap();
        let host = DirectRunHost::new(HostMode::Run);
        let error = execute(
            &host,
            &run_args(&["print-cwd"], &dir.path().join("missing"), Vec::new()),
            StageInput::none(),
            &ctx(&[]),
        )
        .unwrap_err();
        assert_eq!(error.code(), ErrorCode::Spawn);
        assert!(host.recorded().is_empty());
    }

    #[test]
    fn without_cwd_and_base_the_stage_asks_for_a_directory() {
        let host = DirectRunHost::new(HostMode::Run);
        let args = BoundArgs::new("run").with(
            "command",
            ArgValue::Value(Value::List(vec![text(&fixture())])),
            false,
        );
        let error = execute(&host, &args, StageInput::none(), &ctx(&[])).unwrap_err();
        assert_eq!(error.code(), ErrorCode::MissingParam);
    }
}

mod outcomes;
mod parameters;

mod environment {
    use super::*;

    /// Prefix of the variables that the profiling runtime of a test program
    /// built for coverage measurement sets in its own environment.
    const PROFILE_RUNTIME_PREFIX: &str = "__LLVM_PROFILE";

    fn child_env(args: &BoundArgs, ctx: &ExecCtx) -> BTreeMap<String, String> {
        let host = DirectRunHost::new(HostMode::Run);
        let (records, _, _) = execute(&host, args, StageInput::none(), ctx).unwrap();
        serde_json::from_str(exec_of(&records).stdout.trim()).unwrap()
    }

    fn env_map(items: &[(&str, Value)]) -> Value {
        let mut fields = Fields::new();
        for (name, value) in items {
            fields.insert((*name).to_owned(), value.clone());
        }
        Value::Map(fields)
    }

    #[test]
    fn inherits_the_console_environment_with_defaults_and_changes() {
        let dir = tempfile::tempdir().unwrap();
        let args = run_args(
            &["print-env"],
            dir.path(),
            vec![(
                "env",
                env_map(&[
                    ("ADDED", text("1")),
                    ("REMOVED", Value::Null),
                    ("EMPTY", text("")),
                ]),
            )],
        );
        let env = child_env(
            &args,
            &ctx(&[("KEPT", "yes"), ("REMOVED", "x"), ("TERM", "xterm")]),
        );
        assert_eq!(env.get("KEPT").map(String::as_str), Some("yes"));
        assert_eq!(env.get("ADDED").map(String::as_str), Some("1"));
        assert_eq!(env.get("EMPTY").map(String::as_str), Some(""));
        assert!(!env.contains_key("REMOVED"));
        assert_eq!(env.get("TERM").map(String::as_str), Some("dumb"));
        assert_eq!(env.get("NO_COLOR").map(String::as_str), Some("1"));
        assert_eq!(
            env.get("GIT_TERMINAL_PROMPT").map(String::as_str),
            Some("0")
        );
    }

    #[test]
    fn an_explicit_variable_overrides_a_default() {
        let dir = tempfile::tempdir().unwrap();
        let args = run_args(
            &["print-env"],
            dir.path(),
            vec![("env", env_map(&[("TERM", text("xterm"))]))],
        );
        let env = child_env(&args, &ctx(&[]));
        assert_eq!(env.get("TERM").map(String::as_str), Some("xterm"));
    }

    #[cfg(unix)]
    #[test]
    fn an_empty_environment_holds_only_defaults_and_given_variables() {
        let dir = tempfile::tempdir().unwrap();
        let args = run_args(
            &["print-env"],
            dir.path(),
            vec![
                ("inherit_env", Value::Enum("none".to_owned())),
                ("env", env_map(&[("ONLY", text("this"))])),
            ],
        );
        let env = child_env(&args, &ctx(&[("KEPT", "yes")]));
        let mut expected: Vec<String> =
            Config::default().run.env_defaults.keys().cloned().collect();
        expected.push("ONLY".to_owned());
        expected.sort();
        let received: Vec<String> = env
            .keys()
            .filter(|name| !name.starts_with(PROFILE_RUNTIME_PREFIX))
            .cloned()
            .collect();
        assert_eq!(received, expected);
    }

    #[test]
    fn reports_the_names_but_not_the_values_of_given_variables() {
        let dir = tempfile::tempdir().unwrap();
        let args = run_args(
            &["print-argv"],
            dir.path(),
            vec![(
                "env",
                env_map(&[("TOKEN", text("secret-value")), ("B", text("2"))]),
            )],
        );
        let host = DirectRunHost::new(HostMode::Run);
        let (records, _, _) = execute(&host, &args, StageInput::none(), &ctx(&[])).unwrap();
        let exec = exec_of(&records);
        assert_eq!(
            exec.env_keys,
            Some(vec!["TOKEN".to_owned(), "B".to_owned()])
        );
        let json = serde_json::to_string(&exec).unwrap();
        assert!(!json.contains("secret-value"));
    }
}

mod input {
    use super::*;

    /// Ten mebibytes.
    const LARGE_INPUT: usize = 10 * 1024 * 1024;

    fn counted(extra: Vec<(&str, Value)>, dir: &Path) -> String {
        let host = DirectRunHost::new(HostMode::Run);
        let (records, _, _) = execute(
            &host,
            &run_args(&["count-input"], dir, extra),
            StageInput::none(),
            &ctx(&[]),
        )
        .unwrap();
        exec_of(&records).stdout.trim().to_owned()
    }

    #[test]
    fn writes_a_large_text_to_standard_input() {
        let dir = tempfile::tempdir().unwrap();
        let input = "x".repeat(LARGE_INPUT);
        assert_eq!(
            counted(vec![("stdin", text(&input))], dir.path()),
            LARGE_INPUT.to_string()
        );
    }

    #[test]
    fn opens_a_file_as_standard_input() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("input.txt"), "12345").unwrap();
        assert_eq!(
            counted(
                vec![("stdin", path(&dir.path().join("input.txt")))],
                dir.path()
            ),
            "5"
        );
        let host = DirectRunHost::new(HostMode::Run);
        let args = run_args(
            &["count-input"],
            dir.path(),
            vec![("stdin", path(&dir.path().join("input.txt")))],
        );
        execute(&host, &args, StageInput::none(), &ctx(&[])).unwrap();
        assert_eq!(
            host.recorded()[0].0.spawn.stdin,
            StdinSource::File(dir.path().join("input.txt"))
        );
    }

    #[test]
    fn standard_input_is_empty_without_stdin() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(counted(Vec::new(), dir.path()), "0");
    }

    #[test]
    fn a_missing_input_file_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let host = DirectRunHost::new(HostMode::Run);
        let args = run_args(
            &["count-input"],
            dir.path(),
            vec![("stdin", path(&dir.path().join("none")))],
        );
        let error = execute(&host, &args, StageInput::none(), &ctx(&[])).unwrap_err();
        assert_eq!(error.code(), ErrorCode::NotFound);
    }
}

mod batches {
    use super::*;

    #[test]
    fn splits_input_paths_into_batches_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let host = DirectRunHost::new(HostMode::Run);
        let args = run_args(&["print-argv"], dir.path(), vec![("batch", Value::Int(2))]);
        let (records, _, _) =
            execute(&host, &args, entries(&["a", "b", "c", "d", "e"]), &ctx(&[])).unwrap();
        let exec = exec_of(&records);
        let batches = exec.batches.clone().unwrap();
        assert_eq!(batches.len(), 3);
        assert_eq!(batches[0].args, vec!["a".to_owned(), "b".to_owned()]);
        assert_eq!(batches[2].args, vec!["e".to_owned()]);
        assert_eq!(batches[2].no, 3);
        assert_eq!(exec.stdout, "[\"a\",\"b\"]\n[\"c\",\"d\"]\n[\"e\"]\n");
        assert_eq!(exec.exit_code, 0);
        assert_eq!(exec.failed_batches, Some(0));
        assert_eq!(exec.command.len(), 2);
    }

    #[test]
    fn without_batch_one_process_runs_per_record() {
        let dir = tempfile::tempdir().unwrap();
        let host = DirectRunHost::new(HostMode::Run);
        let args = run_args(&["print-argv"], dir.path(), Vec::new());
        let (records, _, _) = execute(&host, &args, entries(&["a", "b"]), &ctx(&[])).unwrap();
        assert_eq!(
            exec_of(&records).batches.map(|batches| batches.len()),
            Some(2)
        );
        assert!(host.recorded().iter().all(|(_, wait)| wait.is_none()));
    }

    #[test]
    fn keeps_batch_order_when_batches_run_in_parallel() {
        let dir = tempfile::tempdir().unwrap();
        let host = DirectRunHost::new(HostMode::Run);
        let args = run_args(
            &["exit-with", "0"],
            dir.path(),
            vec![("parallel", Value::Int(3))],
        );
        let (records, _, _) =
            execute(&host, &args, entries(&["300", "150", "0"]), &ctx(&[])).unwrap();
        let exec = exec_of(&records);
        let numbers: Vec<u64> = exec.batches.unwrap().iter().map(|batch| batch.no).collect();
        assert_eq!(numbers, vec![1, 2, 3]);
        assert_eq!(exec.stdout, "exit 0\nexit 0\nexit 0\n");
    }

    #[test]
    fn the_exit_code_is_the_code_of_the_first_failed_batch() {
        let dir = tempfile::tempdir().unwrap();
        let host = DirectRunHost::new(HostMode::Run);
        let args = run_args(&["exit-with"], dir.path(), Vec::new());
        let (records, notable, _) =
            execute(&host, &args, entries(&["0", "5", "0", "7"]), &ctx(&[])).unwrap();
        let exec = exec_of(&records);
        assert_eq!(exec.exit_code, 5);
        assert_eq!(exec.failed_batches, Some(2));
        assert_eq!(exec.batches.map(|batches| batches.len()), Some(4));
        assert!(notable.contains(&Notable::ExitNonzero));
    }

    #[test]
    fn bail_starts_no_batch_after_the_first_failure() {
        let dir = tempfile::tempdir().unwrap();
        let host = DirectRunHost::new(HostMode::Run);
        let args = run_args(
            &["exit-with"],
            dir.path(),
            vec![("bail", Value::Bool(true))],
        );
        let (records, _, _) = execute(&host, &args, entries(&["0", "3", "0"]), &ctx(&[])).unwrap();
        let exec = exec_of(&records);
        assert_eq!(exec.exit_code, 3);
        assert_eq!(exec.batches.map(|batches| batches.len()), Some(2));
        assert_eq!(host.recorded().len(), 2);
    }

    #[test]
    fn an_empty_input_starts_no_process() {
        let dir = tempfile::tempdir().unwrap();
        let host = DirectRunHost::new(HostMode::Run);
        let (records, _, _) = execute(
            &host,
            &run_args(&["print-argv"], dir.path(), Vec::new()),
            entries(&[]),
            &ctx(&[]),
        )
        .unwrap();
        assert!(records.is_empty());
        assert!(host.recorded().is_empty());
    }

    #[test]
    fn a_record_without_path_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let host = DirectRunHost::new(HostMode::Run);
        let input = StageInput::records(RecordStream::once(Record::new(RecordKind::Entry)));
        let error = execute(
            &host,
            &run_args(&["print-argv"], dir.path(), Vec::new()),
            input,
            &ctx(&[]),
        )
        .unwrap_err();
        assert_eq!(error.code(), ErrorCode::Type);
    }
}

mod stage {
    use super::*;

    #[test]
    fn a_preview_shows_the_launch_without_starting_it() {
        let dir = tempfile::tempdir().unwrap();
        let mut env = Fields::new();
        env.insert("A".to_owned(), text("1"));
        let args = run_args(
            &["print-argv", "x"],
            dir.path(),
            vec![("env", Value::Map(env))],
        );
        let host = DirectRunHost::new(HostMode::Run);
        let stage = RunStage::with_host(Arc::clone(&host) as Arc<dyn RunHost>);
        let ctx = ctx(&[]);
        let plan = stage.plan(&args, StageInput::none(), &ctx).unwrap();
        assert!(host.recorded().is_empty());
        assert!(!plan.bulk);
        let preview = &plan.preview[0];
        assert_eq!(
            preview.get("launcher"),
            Some(&Value::Enum("direct".to_owned()))
        );
        assert_eq!(preview.get("env_keys"), Some(&Value::List(vec![text("A")])));
        assert_eq!(preview.get("stdin"), Some(&Value::Enum("none".to_owned())));
        assert_eq!(
            preview.get("wait"),
            Some(&Value::Duration(value::Duration::from_millis(3000)))
        );
        let Some(Value::List(argv)) = preview.get("argv") else {
            panic!("the preview has no argv");
        };
        assert_eq!(argv.len(), 3);
        assert!(matches!(preview.get("program"), Some(Value::Path(_))));
    }

    #[test]
    fn a_run_past_its_wait_continues_as_a_job() {
        let dir = tempfile::tempdir().unwrap();
        let host = DirectRunHost::new(HostMode::Background);
        let (records, notable, detached) = execute(
            &host,
            &run_args(&["print-argv"], dir.path(), Vec::new()),
            StageInput::none(),
            &ctx(&[]),
        )
        .unwrap();
        let job = Job::try_from(records[0].clone()).unwrap();
        assert_eq!(job.state, JobState::Running);
        assert!(detached);
        assert_eq!(notable, vec![Notable::PostStagesDeferred]);
        assert_eq!(host.recorded()[0].1, Some(Duration::from_secs(3)));
    }

    #[test]
    fn background_and_wait_select_the_waiting_time() {
        let dir = tempfile::tempdir().unwrap();
        let cases = [
            (
                vec![("background", Value::Bool(true))],
                Some(Duration::ZERO),
            ),
            (vec![("wait", Value::Enum("forever".to_owned()))], None),
            (
                vec![("wait", Value::Duration(value::Duration::from_millis(250)))],
                Some(Duration::from_millis(250)),
            ),
        ];
        for (extra, expected) in cases {
            let host = DirectRunHost::new(HostMode::Background);
            execute(
                &host,
                &run_args(&["print-argv"], dir.path(), extra),
                StageInput::none(),
                &ctx(&[]),
            )
            .unwrap();
            assert_eq!(host.recorded()[0].1, expected);
        }
    }

    #[test]
    fn a_nonzero_exit_code_is_a_note_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let host = DirectRunHost::new(HostMode::Run);
        let (records, notable, detached) = execute(
            &host,
            &run_args(&["exit-with", "3"], dir.path(), Vec::new()),
            StageInput::none(),
            &ctx(&[]),
        )
        .unwrap();
        assert_eq!(exec_of(&records).exit_code, 3);
        assert_eq!(notable, vec![Notable::ExitNonzero]);
        assert!(!detached);
    }

    #[test]
    fn a_program_ended_by_a_limit_fails_with_the_exec_record() {
        let dir = tempfile::tempdir().unwrap();
        for (by, code) in [
            (KilledBy::Timeout, ErrorCode::Timeout),
            (KilledBy::Idle, ErrorCode::Timeout),
            (KilledBy::Limit, ErrorCode::Limit),
            (KilledBy::Cancel, ErrorCode::Cancelled),
        ] {
            let host = DirectRunHost::new(HostMode::KilledBy(by));
            let error = execute(
                &host,
                &run_args(&["print-argv"], dir.path(), Vec::new()),
                StageInput::none(),
                &ctx(&[]),
            )
            .unwrap_err();
            assert_eq!(error.code(), code);
            assert!(error.data().effect_applied);
            let exec = error.data().details.get("exec").unwrap();
            assert_eq!(exec["exit_code"], -1);
        }
    }

    #[test]
    fn a_program_stopped_by_stop_is_an_outcome() {
        let dir = tempfile::tempdir().unwrap();
        let host = DirectRunHost::new(HostMode::KilledBy(KilledBy::Stop));
        let (records, _, _) = execute(
            &host,
            &run_args(&["print-argv"], dir.path(), Vec::new()),
            StageInput::none(),
            &ctx(&[]),
        )
        .unwrap();
        assert_eq!(exec_of(&records).killed_by, Some(KilledBy::Stop));
    }

    #[test]
    fn passes_limits_view_and_job_fields_to_the_run_host() {
        let dir = tempfile::tempdir().unwrap();
        let host = DirectRunHost::new(HostMode::Background);
        let args = run_args(
            &["print-argv"],
            dir.path(),
            vec![
                (
                    "timeout",
                    Value::Duration(value::Duration::from_millis(5000)),
                ),
                (
                    "idle_timeout",
                    Value::Duration(value::Duration::from_millis(0)),
                ),
                ("max_memory", Value::Size(value::Size::from_bytes(1 << 20))),
                ("priority", Value::Enum("low".to_owned())),
                ("label", text("build")),
                ("description", text("release build")),
                ("show", Value::Enum("errors".to_owned())),
                ("head", Value::Int(5)),
                (
                    "filter",
                    Value::Regex(value::Regex::new("err", "i").unwrap()),
                ),
                (
                    "summary",
                    Value::List(vec![Value::Regex(value::Regex::new("passed", "").unwrap())]),
                ),
                ("output_encoding", text("auto")),
                ("merge_stderr", Value::Bool(true)),
                ("track_changes", Value::Enum("none".to_owned())),
            ],
        );
        execute(&host, &args, StageInput::none(), &ctx(&[])).unwrap();
        let (request, _) = &host.recorded()[0];
        let limits = &request.spawn.limits;
        assert_eq!(
            (
                limits.timeout,
                limits.idle_timeout,
                limits.max_memory,
                limits.priority,
                limits.kill_grace,
            ),
            (
                Some(Duration::from_secs(5)),
                None,
                Some(1 << 20),
                crate::iface::Priority::Low,
                Duration::from_secs(5),
            )
        );
        let view = &request.view;
        assert_eq!(
            (
                view.show,
                view.head,
                view.tail,
                view.filter.as_deref(),
                view.summary.as_slice(),
                view.encoding.as_deref(),
            ),
            (
                crate::iface::ShowMode::Errors,
                Some(5),
                Some(40),
                Some("(?i)err"),
                ["passed".to_owned()].as_slice(),
                None,
            )
        );
        assert_eq!(
            (
                request.label.as_deref(),
                request.description.as_deref(),
                request.spawn.merge_stderr,
                request.track.mode,
                request.kind,
                request.finish_data.is_none(),
            ),
            (
                Some("build"),
                Some("release build"),
                true,
                crate::iface::TrackMode::None,
                RunKind::Run,
                true,
            )
        );
    }

    #[test]
    fn the_static_check_reports_waiting_programs_and_environment_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let spec: StageSpec = serde_json::from_value(serde_json::json!({
            "name": "run",
            "category": "run",
            "reversibility": "reversible_if_preserved",
            "effect": "process",
            "implementation": "user_program",
            "input": ["none"],
            "output": ["exec", "job"],
            "summary": "Run a program without a shell.",
        }))
        .unwrap();
        let host = TestHost::current(&[]);
        let config = Config::default();
        let check = CheckCtx::new(&spec, &host, &config);
        let stage = RunStage::default();
        let args = run_args(
            &["print-argv"],
            dir.path(),
            vec![("wait", Value::Enum("forever".to_owned()))],
        );
        let info = stage.check(&args, &check).unwrap();
        assert!(info.waits);
        assert_eq!(info.programs[0].program, fixture());
        let Some(Value::Map(defaults)) = info.details.get("env_defaults") else {
            panic!("the check has no env_defaults");
        };
        assert_eq!(defaults.get("TERM"), Some(&text("dumb")));
        let short = run_args(&["print-argv"], dir.path(), Vec::new());
        assert!(!stage.check(&short, &check).unwrap().waits);
        let batch_file = BoundArgs::new("run").with(
            "command",
            ArgValue::Value(Value::List(vec![
                text("build.cmd"),
                text("a&b"),
                text("plain"),
            ])),
            false,
        );
        let info = stage.check(&batch_file, &check).unwrap();
        assert_eq!(
            info.details.get("metacharacter_arguments"),
            Some(&Value::List(vec![Value::from(1_u64)]))
        );
        let zero = run_args(&["print-argv"], dir.path(), vec![("batch", Value::Int(0))]);
        assert_eq!(
            stage.check(&zero, &check).unwrap_err().code(),
            ErrorCode::InvalidValue
        );
    }

    #[test]
    fn the_stage_accepts_file_and_match_records() {
        let descriptor = RunStage::default().descriptor();
        assert_eq!(descriptor.command, "run");
        assert!(descriptor.source);
        assert!(descriptor.input.accepts(RecordKind::Entry));
        assert!(descriptor.input.accepts(RecordKind::Match));
        assert!(!descriptor.input.accepts(RecordKind::Exec));
    }
}

mod tools {
    use super::*;

    fn tool_run(dir: &Path) -> ToolRun {
        ToolRun {
            command: vec![fixture(), "print-argv".to_owned(), "--json".to_owned()],
            cwd: dir.to_path_buf(),
            env: Vec::new(),
            timeout: Duration::from_secs(30),
            watch: vec![dir.to_path_buf()],
            exact: None,
            kind: ToolKind::Test,
            label: Some("tests".to_owned()),
            finish_data: Some(serde_json::json!({"run": "run-1"})),
        }
    }

    #[test]
    fn a_tool_run_returns_the_exec_record() {
        let dir = tempfile::tempdir().unwrap();
        let host = DirectRunHost::new(HostMode::Run);
        let shared = Arc::clone(&host) as Arc<dyn RunHost>;
        let outcome =
            super::super::run_tool(&ctx(&[]), &shared, tool_run(dir.path()), None).unwrap();
        let ToolOutcome::Finished(result) = outcome else {
            panic!("the tool moved to the background");
        };
        assert_eq!(result.exec.stdout.trim(), "[\"--json\"]");
        let (request, _) = &host.recorded()[0];
        assert_eq!(request.kind, RunKind::Test);
        assert_eq!(
            request.finish_data,
            Some(serde_json::json!({"run": "run-1"}))
        );
        assert_eq!(request.view.show, crate::iface::ShowMode::Both);
    }

    #[test]
    fn a_tool_run_past_its_wait_returns_the_job() {
        let dir = tempfile::tempdir().unwrap();
        let host = DirectRunHost::new(HostMode::Background);
        let shared = Arc::clone(&host) as Arc<dyn RunHost>;
        let mut run = tool_run(dir.path());
        run.kind = ToolKind::Analyze;
        let outcome =
            super::super::run_tool(&ctx(&[]), &shared, run, Some(Duration::ZERO)).unwrap();
        assert!(matches!(outcome, ToolOutcome::Background(_)));
        assert_eq!(host.recorded()[0].0.kind, RunKind::Analyze);
    }
}
