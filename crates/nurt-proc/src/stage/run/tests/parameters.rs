// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-proc::stage::run::tests::parameters
// Responsibility:   Parameters of `run`: defaults of the configuration, the values passed to.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Parameters of `run`: defaults of the configuration, the values passed to
//! the run host and the rejection of values the stage cannot use.

use nurt_contracts::config::{DurationSetting, TrackChanges};
use nurt_contracts::stage::StaticInfo;

use super::*;
use crate::iface::{Priority, RunRequest, ShowMode, TrackMode};

fn config_ctx(config: Config) -> ExecCtx {
    context(TestHost::current(&[]), config, RequestOptions::default())
}

fn millis(value: i64) -> Value {
    Value::Duration(value::Duration::from_millis(value))
}

fn word(value: &str) -> Value {
    Value::Enum(value.to_owned())
}

/// Request and waiting time the stage hands to the run host; no program is
/// started.
fn planned(args: &BoundArgs, ctx: &ExecCtx) -> (RunRequest, Option<Duration>) {
    let host = DirectRunHost::new(HostMode::Background);
    execute(&host, args, StageInput::none(), ctx).unwrap();
    host.recorded().remove(0)
}

/// Code of the error of `args`; the run host receives nothing.
fn rejected(args: &BoundArgs, ctx: &ExecCtx) -> ErrorCode {
    let host = DirectRunHost::new(HostMode::Background);
    let code = execute(&host, args, StageInput::none(), ctx)
        .unwrap_err()
        .code();
    assert!(host.recorded().is_empty());
    code
}

fn static_info(args: &BoundArgs) -> Result<StaticInfo> {
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
    RunStage::default().check(args, &check)
}

#[test]
fn limits_and_output_lines_default_to_the_configuration() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = Config::default();
    config.run.timeout = DurationSetting::from_millis(7_000);
    config.run.idle_timeout = DurationSetting::from_millis(45_000);
    config.run.kill_grace = DurationSetting::from_millis(1_500);
    config.output.head = 7;
    config.output.tail = 9;
    let ctx = config_ctx(config);
    let (request, _) = planned(&run_args(&["print-argv"], dir.path(), Vec::new()), &ctx);
    let limits = &request.spawn.limits;
    assert_eq!(
        (limits.timeout, limits.idle_timeout, limits.kill_grace),
        (
            Some(Duration::from_secs(7)),
            Some(Duration::from_secs(45)),
            Duration::from_millis(1_500)
        )
    );
    assert_eq!((request.view.head, request.view.tail), (Some(7), Some(9)));
    let defaulted = run_args(&["print-argv"], dir.path(), Vec::new())
        .with("timeout", ArgValue::Value(millis(120_000)), true)
        .with("tail", ArgValue::Value(Value::Int(40)), true);
    let (request, _) = planned(&defaulted, &ctx);
    assert_eq!(request.spawn.limits.timeout, Some(Duration::from_secs(7)));
    assert_eq!(request.view.tail, Some(9));
    let given = run_args(
        &["print-argv"],
        dir.path(),
        vec![
            ("timeout", millis(1_500)),
            ("idle_timeout", millis(0)),
            ("head", Value::Int(0)),
            ("tail", Value::Int(3)),
        ],
    );
    let (request, _) = planned(&given, &ctx);
    assert_eq!(
        request.spawn.limits.timeout,
        Some(Duration::from_millis(1_500))
    );
    assert_eq!(request.spawn.limits.idle_timeout, None);
    assert_eq!((request.view.head, request.view.tail), (Some(0), Some(3)));
}

#[test]
fn a_zero_configured_wait_moves_every_run_to_the_background() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = Config::default();
    config.run.wait = DurationSetting::from_millis(0);
    let ctx = config_ctx(config);
    let args = run_args(&["print-argv"], dir.path(), Vec::new());
    let host = DirectRunHost::new(HostMode::Background);
    let (records, notable, detached) = execute(&host, &args, StageInput::none(), &ctx).unwrap();
    assert_eq!(host.recorded()[0].1, Some(Duration::ZERO));
    assert!(detached);
    assert_eq!(notable, vec![Notable::PostStagesDeferred]);
    assert_eq!(records[0].kind, RecordKind::Job);
    let waiting = run_args(&["print-argv"], dir.path(), vec![("wait", millis(250))]);
    assert_eq!(planned(&waiting, &ctx).1, Some(Duration::from_millis(250)));
}

#[test]
fn show_accepts_every_mode_of_the_catalog() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = ctx(&[]);
    let (request, _) = planned(&run_args(&["print-argv"], dir.path(), Vec::new()), &ctx);
    assert_eq!(request.view.show, ShowMode::Auto);
    for (name, mode) in [
        ("auto", ShowMode::Auto),
        ("both", ShowMode::Both),
        ("stdout", ShowMode::Stdout),
        ("stderr", ShowMode::Stderr),
        ("errors", ShowMode::Errors),
        ("summary", ShowMode::Summary),
        ("none", ShowMode::None),
    ] {
        let args = run_args(&["print-argv"], dir.path(), vec![("show", word(name))]);
        assert_eq!(planned(&args, &ctx).0.view.show, mode, "{name}");
    }
}

#[test]
fn the_selection_and_the_limits_reach_the_run_host() {
    let dir = tempfile::tempdir().unwrap();
    let args = run_args(
        &["print-argv"],
        dir.path(),
        vec![
            ("priority", word("high")),
            ("max_cpu", millis(2_000)),
            ("stdin", text("data")),
            ("filter", text("warn")),
            ("context", Value::Int(2)),
            ("dedupe", Value::Bool(true)),
            ("quiet_on_success", Value::Bool(true)),
            (
                "progress",
                Value::Regex(value::Regex::new("(\\d+)%", "").unwrap()),
            ),
            (
                "summary",
                Value::Regex(value::Regex::new("passed", "").unwrap()),
            ),
            ("output_encoding", text("cp1250")),
        ],
    );
    let (request, _) = planned(&args, &ctx(&[]));
    assert_eq!(request.spawn.limits.priority, Priority::High);
    assert_eq!(request.spawn.limits.max_cpu, Some(Duration::from_secs(2)));
    assert_eq!(request.spawn.stdin, StdinSource::Bytes(b"data".to_vec()));
    let view = &request.view;
    assert_eq!(
        (
            view.filter.as_deref(),
            view.context,
            view.dedupe,
            view.quiet_on_success,
            view.progress.as_deref(),
            view.summary.as_slice(),
            view.encoding.as_deref(),
        ),
        (
            Some("warn"),
            2,
            true,
            true,
            Some("(\\d+)%"),
            ["passed".to_owned()].as_slice(),
            Some("cp1250"),
        )
    );
}

#[test]
fn the_session_of_the_call_reaches_the_request() {
    let dir = tempfile::tempdir().unwrap();
    let options = RequestOptions {
        session: Some("s7".to_owned()),
        ..RequestOptions::default()
    };
    let ctx = context(TestHost::current(&[]), Config::default(), options);
    let (request, _) = planned(&run_args(&["print-argv"], dir.path(), Vec::new()), &ctx);
    assert_eq!(request.session.as_deref(), Some("s7"));
    assert_eq!(request.kind, RunKind::Run);
    assert!(request.retry_of.is_none());
}

#[test]
fn the_preview_shows_input_environment_and_change_detection() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("input.txt"), "abc").unwrap();
    let args = run_args(
        &["print-argv"],
        dir.path(),
        vec![
            ("stdin", Value::Path("input.txt".to_owned())),
            ("inherit_env", word("none")),
            ("track_changes", word("events")),
            (
                "watch",
                Value::List(vec![Value::Path("out".to_owned()), text("logs")]),
            ),
            ("wait", word("forever")),
            ("timeout", millis(9_000)),
        ],
    );
    let host = DirectRunHost::new(HostMode::Run);
    let stage = RunStage::with_host(Arc::clone(&host) as Arc<dyn RunHost>);
    let plan = stage.plan(&args, StageInput::none(), &ctx(&[])).unwrap();
    assert!(host.recorded().is_empty());
    let preview = &plan.preview[0];
    assert_eq!(preview.get("stdin"), Some(&word("file")));
    assert_eq!(preview.get("inherit_env"), Some(&word("none")));
    assert_eq!(preview.get("track_changes"), Some(&word("events")));
    assert_eq!(preview.get("wait"), Some(&word("forever")));
    assert_eq!(preview.get("timeout"), Some(&millis(9_000)));
    let Some(Value::List(watch)) = preview.get("watch") else {
        panic!("the preview has no watch list");
    };
    assert_eq!(watch.len(), 2);
    assert!(preview.get("batch").is_none());
    let (request, _) = planned(&args, &ctx(&[]));
    assert_eq!(
        request.spawn.stdin,
        StdinSource::File(dir.path().join("input.txt"))
    );
    assert_eq!(
        request.track.watch,
        vec![dir.path().join("out"), dir.path().join("logs")]
    );
    assert_eq!(request.track.mode, TrackMode::Events);
}

#[test]
fn change_detection_follows_the_argument_and_the_configuration() {
    let dir = tempfile::tempdir().unwrap();
    for (name, mode) in [
        ("auto", TrackMode::Auto),
        ("scan", TrackMode::Scan),
        ("events", TrackMode::Events),
        ("none", TrackMode::None),
    ] {
        let args = run_args(
            &["print-argv"],
            dir.path(),
            vec![("track_changes", word(name))],
        );
        assert_eq!(planned(&args, &ctx(&[])).0.track.mode, mode, "{name}");
    }
    for (configured, mode) in [
        (TrackChanges::Scan, TrackMode::Scan),
        (TrackChanges::Events, TrackMode::Events),
        (TrackChanges::Disabled, TrackMode::None),
        (TrackChanges::Auto, TrackMode::Auto),
    ] {
        let mut config = Config::default();
        config.run.track_changes = configured;
        let args = run_args(&["print-argv"], dir.path(), Vec::new());
        let (request, _) = planned(&args, &config_ctx(config));
        assert_eq!(request.track.mode, mode, "{configured:?}");
        assert_eq!(request.track.watch, vec![dir.path().to_path_buf()]);
    }
}

#[test]
fn rejects_values_the_stage_cannot_use() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = ctx(&[]);
    let mut nested = Fields::new();
    nested.insert("A".to_owned(), Value::List(Vec::new()));
    let cases = [
        ("timeout", text("2m"), ErrorCode::Type),
        ("timeout", millis(-1), ErrorCode::InvalidValue),
        ("idle_timeout", Value::Int(5), ErrorCode::Type),
        ("max_cpu", text("1s"), ErrorCode::Type),
        ("max_memory", Value::Int(1_024), ErrorCode::Type),
        ("merge_stderr", text("yes"), ErrorCode::Type),
        ("dedupe", text("no"), ErrorCode::Type),
        ("batch", text("2"), ErrorCode::Type),
        ("parallel", Value::Int(0), ErrorCode::InvalidValue),
        ("head", Value::Int(-1), ErrorCode::InvalidValue),
        ("tail", text("5"), ErrorCode::Type),
        ("context", Value::Int(-1), ErrorCode::InvalidValue),
        ("context", text("2"), ErrorCode::Type),
        ("env", text("A=1"), ErrorCode::Type),
        ("env", Value::Map(nested), ErrorCode::Type),
        ("stdin", Value::Int(1), ErrorCode::Type),
        ("watch", text("src"), ErrorCode::Type),
        ("watch", Value::List(vec![Value::Int(1)]), ErrorCode::Type),
        ("filter", Value::Int(1), ErrorCode::Type),
        ("summary", Value::Int(1), ErrorCode::Type),
        ("progress", Value::Bool(true), ErrorCode::Type),
        ("track_changes", word("sometimes"), ErrorCode::InvalidValue),
        ("show", word("full"), ErrorCode::InvalidValue),
        ("label", Value::Int(1), ErrorCode::Type),
        ("cwd", Value::Int(1), ErrorCode::Type),
        ("wait", text("soon"), ErrorCode::Type),
    ];
    for (name, value, code) in cases {
        let args = run_args(&["print-argv"], dir.path(), vec![(name, value)]);
        assert_eq!(rejected(&args, &ctx), code, "{name}");
    }
}

#[test]
fn the_command_must_be_a_list_of_words() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = ctx(&[]);
    let cwd = ArgValue::Value(path(dir.path()));
    let missing = BoundArgs::new("run").with("cwd", cwd.clone(), false);
    assert_eq!(rejected(&missing, &ctx), ErrorCode::MissingParam);
    let line = missing
        .clone()
        .with("command", ArgValue::Value(text("make test")), false);
    assert_eq!(rejected(&line, &ctx), ErrorCode::Type);
    let nested = missing.with(
        "command",
        ArgValue::Value(Value::List(vec![
            text(&fixture()),
            Value::Map(Fields::new()),
        ])),
        false,
    );
    assert_eq!(rejected(&nested, &ctx), ErrorCode::Type);
}

#[test]
fn a_relative_working_directory_needs_base() {
    let args = run_args(&["print-cwd"], Path::new("sub"), Vec::new());
    assert_eq!(rejected(&args, &ctx(&[])), ErrorCode::PathRelative);
}

#[test]
fn command_words_of_other_kinds_reach_the_program_as_text() {
    let dir = tempfile::tempdir().unwrap();
    let mut args = run_args(&[], dir.path(), Vec::new());
    args.insert(
        "command",
        ArgValue::Value(Value::List(vec![
            text(&fixture()),
            text("print-argv"),
            Value::Int(3),
            Value::Bool(true),
            word("fast"),
            Value::Path("out/report.txt".to_owned()),
        ])),
        false,
    );
    let host = DirectRunHost::new(HostMode::Run);
    let (records, _, _) = execute(&host, &args, StageInput::none(), &ctx(&[])).unwrap();
    let received: Vec<String> = serde_json::from_str(exec_of(&records).stdout.trim()).unwrap();
    assert_eq!(received, ["3", "true", "fast", "out/report.txt"]);
}

#[test]
fn the_environment_of_one_run_does_not_reach_the_next() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = ctx(&[]);
    let mut env = Fields::new();
    env.insert("ONE_RUN_ONLY".to_owned(), text("1"));
    let host = DirectRunHost::new(HostMode::Run);
    let first = run_args(&["print-env"], dir.path(), vec![("env", Value::Map(env))]);
    let second = run_args(&["print-env"], dir.path(), Vec::new());
    let mut printed = Vec::new();
    for args in [&first, &second] {
        let (records, _, _) = execute(&host, args, StageInput::none(), &ctx).unwrap();
        let env: BTreeMap<String, String> =
            serde_json::from_str(exec_of(&records).stdout.trim()).unwrap();
        printed.push(env.contains_key("ONE_RUN_ONLY"));
    }
    assert_eq!(printed, [true, false]);
}

#[test]
fn a_missing_program_fails_the_run_before_the_start() {
    let dir = tempfile::tempdir().unwrap();
    let mut args = run_args(&[], dir.path(), Vec::new());
    args.insert(
        "command",
        ArgValue::Value(Value::List(vec![text("no-such-program-for-tests")])),
        false,
    );
    assert_eq!(rejected(&args, &ctx(&[])), ErrorCode::ProgramNotFound);
}

#[test]
fn a_run_the_console_ended_names_the_limit_in_its_error() {
    let dir = tempfile::tempdir().unwrap();
    let cases = [
        (
            KilledBy::Timeout,
            vec![("timeout", millis(1_500))],
            "timeout",
            "of 1500ms",
        ),
        (
            KilledBy::Timeout,
            vec![("timeout", millis(2_000))],
            "timeout",
            "of 2s",
        ),
        (
            KilledBy::Idle,
            vec![("idle_timeout", millis(30_000))],
            "idle",
            "of 30s",
        ),
        (KilledBy::Limit, Vec::new(), "cpu", "max_cpu"),
        (KilledBy::Cancel, Vec::new(), "cancel", "cancelled"),
    ];
    for (by, extra, reason, text) in cases {
        let host = DirectRunHost::new(HostMode::KilledBy(by));
        let args = run_args(&["print-argv"], dir.path(), extra);
        let error = execute(&host, &args, StageInput::none(), &ctx(&[])).unwrap_err();
        let data = error.data();
        assert!(data.message.contains(text), "{}", data.message);
        assert_eq!(data.details["reason"], reason);
        let exec = &data.details["exec"];
        assert_eq!(exec["timed_out"], by == KilledBy::Timeout, "{reason}");
        assert_eq!(exec["stalled"], by == KilledBy::Idle, "{reason}");
    }
}

#[test]
fn the_static_check_names_the_paths_and_variables_of_the_run() {
    let dir = tempfile::tempdir().unwrap();
    let mut env = Fields::new();
    env.insert("TOKEN".to_owned(), text("x"));
    let args = run_args(
        &["print-argv"],
        dir.path(),
        vec![
            ("stdin", Value::Path("input.txt".to_owned())),
            (
                "watch",
                Value::List(vec![
                    Value::Path("out".to_owned()),
                    text("logs"),
                    Value::Int(1),
                ]),
            ),
            ("env", Value::Map(env)),
            ("inherit_env", word("none")),
        ],
    );
    let info = static_info(&args).unwrap();
    let cwd = dir.path().to_string_lossy().into_owned();
    assert_eq!(
        info.paths,
        vec![
            cwd,
            "input.txt".to_owned(),
            "out".to_owned(),
            "logs".to_owned()
        ]
    );
    assert_eq!(
        info.details.get("env_keys"),
        Some(&Value::List(vec![text("TOKEN")]))
    );
    assert_eq!(info.details.get("inherit_env"), Some(&word("none")));
    assert!(info.details.get("launcher").is_none());
    let background = run_args(
        &["print-argv"],
        dir.path(),
        vec![("background", Value::Bool(true)), ("wait", word("forever"))],
    );
    assert!(!static_info(&background).unwrap().waits);
    let long = run_args(
        &["print-argv"],
        dir.path(),
        vec![("wait", millis(3_600_000))],
    );
    assert!(static_info(&long).unwrap().waits);
    let parallel = run_args(
        &["print-argv"],
        dir.path(),
        vec![("parallel", Value::Int(0))],
    );
    assert_eq!(
        static_info(&parallel).unwrap_err().code(),
        ErrorCode::InvalidValue
    );
}

#[test]
fn a_change_detection_mode_inserted_as_a_default_yields_to_the_configuration() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = Config::default();
    config.run.track_changes = TrackChanges::Scan;
    let ctx = config_ctx(config);
    let defaulted = run_args(&["print-argv"], dir.path(), Vec::new()).with(
        "track_changes",
        ArgValue::Value(word("auto")),
        true,
    );
    assert_eq!(planned(&defaulted, &ctx).0.track.mode, TrackMode::Scan);
    let given = run_args(&["print-argv"], dir.path(), Vec::new()).with(
        "track_changes",
        ArgValue::Value(word("auto")),
        false,
    );
    assert_eq!(planned(&given, &ctx).0.track.mode, TrackMode::Auto);
}

#[cfg(unix)]
#[test]
fn a_device_file_is_accepted_as_standard_input() {
    let dir = tempfile::tempdir().unwrap();
    let args = run_args(
        &["print-argv"],
        dir.path(),
        vec![("stdin", Value::Path("/dev/null".to_owned()))],
    );
    let (request, _) = planned(&args, &ctx(&[]));
    assert_eq!(
        request.spawn.stdin,
        StdinSource::File(std::path::PathBuf::from("/dev/null"))
    );
}

#[test]
fn a_directory_is_rejected_as_standard_input() {
    let dir = tempfile::tempdir().unwrap();
    let args = run_args(
        &["print-argv"],
        dir.path(),
        vec![("stdin", Value::Path(".".to_owned()))],
    );
    assert_eq!(rejected(&args, &ctx(&[])), ErrorCode::NotFound);
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
