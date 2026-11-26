// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Parameters of `run`: bound arguments with the configured defaults.
//!
//! A parameter the program did not give takes its default from the
//! configuration (`run.timeout`, `run.wait`, `run.idle_timeout`,
//! `run.track_changes`, `run.parallel`, `output.head`, `output.tail`, `output.max_bytes`), so the
//! defaults of the catalog apply only when the configuration keeps them.

use std::path::{Path, PathBuf};
use std::time::Duration;

use nurt_contracts::config::{Config, PreimageMode, TrackChanges};
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::kinds::Implementation;
use nurt_contracts::record::Launcher;
use nurt_contracts::services::EnvVar;
use nurt_contracts::stage::{ArgValue, BoundArgs, CheckCtx, ExecCtx, ExternalProgram, StaticInfo};
use nurt_contracts::value::{Fields, Record, Value};

use crate::iface::{
    OutputView, Priority, ProcessLimits, ShowMode, StdinSource, TrackMode, TrackSpec,
};
use crate::spawn::{InheritEnv, Platform, metacharacter_arguments};

/// Settings of one `run` stage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Settings {
    pub(super) platform: Platform,
    pub(super) cwd: PathBuf,
    pub(super) env: Vec<EnvVar>,
    pub(super) inherit: InheritEnv,
    pub(super) stdin: StdinSource,
    pub(super) merge_stderr: bool,
    pub(super) limits: ProcessLimits,
    /// Waiting time before the run moves to the background; `None` waits
    /// until the end.
    pub(super) wait: Option<Duration>,
    pub(super) view: OutputView,
    pub(super) track: TrackSpec,
    pub(super) label: Option<String>,
    pub(super) description: Option<String>,
    pub(super) batch: Option<usize>,
    pub(super) parallel: usize,
    pub(super) bail: bool,
}

impl Settings {
    /// Names of the variables given in `env`, in order.
    pub(super) fn env_keys(&self) -> Vec<String> {
        self.env.iter().map(|var| var.name.clone()).collect()
    }
}

/// Reads the settings of a `run` stage.
///
/// # Errors
///
/// Fails with `E_MISSING_PARAM` without `cwd` and `base`, `E_PATH_RELATIVE`
/// for a relative `cwd` without `base`, `E_TYPE` and `E_INVALID_VALUE` for
/// values the stage cannot use, and `E_NOT_FOUND` for a missing `stdin` file.
pub(super) fn read(args: &BoundArgs, ctx: &ExecCtx) -> Result<Settings> {
    let config = ctx.config();
    let platform = Platform::of(ctx.host().info().os);
    let cwd = working_directory(args, ctx)?;
    let timeout = duration_or(args, ctx, "timeout", config.run.timeout.as_std())?;
    let idle = duration_or(args, ctx, "idle_timeout", config.run.idle_timeout.as_std())?;
    let limits = ProcessLimits {
        timeout: Some(timeout),
        idle_timeout: non_zero(idle),
        kill_grace: config.run.kill_grace.as_std(),
        max_cpu: optional_duration(args, ctx, "max_cpu")?,
        max_memory: match given(args, ctx, "max_memory")? {
            Some(Value::Size(size)) => Some(size.bytes()),
            Some(other) => return Err(type_error("max_memory", "size", &other)),
            None => None,
        },
        priority: match text(args, ctx, "priority")?.as_deref() {
            Some("low") => Priority::Low,
            Some("high") => Priority::High,
            _ => Priority::Normal,
        },
    };
    let track = TrackSpec {
        mode: match given(args, ctx, "track_changes")? {
            Some(Value::Enum(mode) | Value::Text(mode)) => track_mode_named(&mode)?,
            None | Some(Value::Null) => track_mode(config.run.track_changes),
            Some(other) => return Err(type_error("track_changes", "text", &other)),
        },
        watch: watch_paths(args, ctx, &cwd)?,
        ignore: config.run.track_ignore.clone(),
        preserve: Vec::new(),
        exact: None,
        preimage: config.run.preimage == PreimageMode::Auto,
        scan_hash: false,
        timeout: limits.timeout,
    };
    Ok(Settings {
        platform,
        env: environment(args, ctx)?,
        inherit: match text(args, ctx, "inherit_env")?.as_deref() {
            Some("none") => InheritEnv::None,
            _ => InheritEnv::All,
        },
        stdin: stdin(args, ctx, &cwd)?,
        merge_stderr: flag(args, ctx, "merge_stderr")?.unwrap_or(false),
        limits,
        wait: wait(args, ctx, config)?,
        view: output_view(args, ctx, config)?,
        track,
        label: text(args, ctx, "label")?,
        description: text(args, ctx, "description")?,
        batch: count(args, ctx, "batch")?,
        parallel: count(args, ctx, "parallel")?
            .unwrap_or_else(|| usize::try_from(config.run.parallel.max(1)).unwrap_or(1)),
        bail: flag(args, ctx, "bail")?.unwrap_or(false),
        cwd,
    })
}

/// Facts of the static check: waiting, the program, named paths and the
/// environment the program receives besides the inherited one.
///
/// # Errors
///
/// Fails with `E_INVALID_VALUE` for `batch` or `parallel` below 1 and for
/// `batch` together with a `command` computed per input record.
pub(super) fn static_info(args: &BoundArgs, ctx: &CheckCtx<'_>) -> Result<StaticInfo> {
    let config = ctx.config();
    for name in ["batch", "parallel"] {
        if let Some(Value::Int(number)) = args.value(name)
            && *number < 1
        {
            return Err(invalid(name, "must be 1 or more"));
        }
    }
    if args.expr("command").is_some() && args.is_given("batch") {
        return Err(invalid(
            "batch",
            "takes its arguments from the `path` of the input records; remove `batch` or write `command` without record fields",
        ));
    }
    let mut info = StaticInfo {
        waits: waits(args, config),
        ..StaticInfo::default()
    };
    if let Some(command) = constant_command(args)
        && let Some(program) = command.first()
    {
        info.programs.push(ExternalProgram {
            program: program.clone(),
            implementation: Implementation::UserProgram,
        });
        let lower = program.to_ascii_lowercase();
        if lower.ends_with(".bat") || lower.ends_with(".cmd") {
            let positions = metacharacter_arguments(&command)
                .into_iter()
                .map(|index| Value::from(u64::try_from(index).unwrap_or(u64::MAX)))
                .collect::<Vec<_>>();
            info.details.insert(
                "launcher".to_owned(),
                Value::Enum(Launcher::Cmd.as_str().to_owned()),
            );
            info.details
                .insert("metacharacter_arguments".to_owned(), Value::List(positions));
        }
    }
    for name in ["cwd", "stdin"] {
        if let Some(Value::Path(path)) = args.value(name) {
            info.paths.push(path.clone());
        }
    }
    if let Some(Value::List(paths)) = args.value("watch") {
        info.paths
            .extend(paths.iter().filter_map(|path| match path {
                Value::Path(path) | Value::Text(path) => Some(path.clone()),
                _ => None,
            }));
    }
    let defaults: Fields = config
        .run
        .env_defaults
        .iter()
        .map(|(name, value)| (name.clone(), Value::Text(value.clone())))
        .collect();
    info.details
        .insert("env_defaults".to_owned(), Value::Map(defaults));
    if let Some(Value::Map(env)) = args.value("env") {
        info.details.insert(
            "env_keys".to_owned(),
            Value::List(env.keys().map(|name| Value::Text(name.clone())).collect()),
        );
    }
    let inherit = match args.value("inherit_env") {
        Some(Value::Enum(mode) | Value::Text(mode)) => mode.clone(),
        _ => "all".to_owned(),
    };
    info.details
        .insert("inherit_env".to_owned(), Value::Enum(inherit));
    Ok(info)
}

/// The command when it is known before the stage runs.
fn constant_command(args: &BoundArgs) -> Option<Vec<String>> {
    match args.value("command")? {
        Value::List(items) => items.iter().map(|item| element_text(item).ok()).collect(),
        _ => None,
    }
}

fn waits(args: &BoundArgs, config: &Config) -> bool {
    if matches!(args.value("background"), Some(Value::Bool(true))) {
        return false;
    }
    match args.value("wait") {
        Some(Value::Enum(word) | Value::Text(word)) => word == "forever",
        Some(Value::Duration(wait)) if !args.is_defaulted("wait") => wait
            .to_std()
            .is_some_and(|wait| wait > config.run.waits_threshold.as_std()),
        _ => config.run.wait.as_std() > config.run.waits_threshold.as_std(),
    }
}

/// Command of one process: the `command` argument, evaluated for `record`
/// when it depends on the record.
///
/// # Errors
///
/// Fails with `E_MISSING_PARAM` without a command and `E_TYPE` for an
/// element that is not text.
pub(super) fn command(
    args: &BoundArgs,
    record: Option<&Record>,
    ctx: &ExecCtx,
) -> Result<Vec<String>> {
    match args.eval("command", record, ctx)? {
        Some(Value::List(items)) => items.iter().map(element_text).collect(),
        Some(other) => Err(type_error("command", "list<text>", &other)),
        None => Err(Error::new(
            ErrorCode::MissingParam,
            "`run` needs a command; give the program and its arguments as a list",
        )),
    }
}

/// Text of one element of a command; numbers, truth values and paths are
/// written as text.
fn element_text(value: &Value) -> Result<String> {
    match value {
        Value::Text(text)
        | Value::Path(text)
        | Value::Glob(text)
        | Value::Key(text)
        | Value::Enum(text)
        | Value::Decimal(text)
        | Value::BigInt(text) => Ok(text.clone()),
        Value::Int(number) => Ok(number.to_string()),
        Value::Number(number) => Ok(number.to_string()),
        Value::Bool(flag) => Ok(flag.to_string()),
        other => Err(type_error("command", "text elements", other)),
    }
}

/// Path given by a record of the input stream.
///
/// # Errors
///
/// Fails with `E_TYPE` when the record has no `path`.
pub(super) fn record_argument(record: &Record) -> Result<String> {
    match record.get("path") {
        Some(Value::Path(path) | Value::Text(path)) => Ok(path.clone()),
        _ => Err(Error::new(
            ErrorCode::Type,
            format!(
                "an input record of kind `{}` has no `path`; pass records of files or matches to `run`",
                record.kind
            ),
        )),
    }
}

/// Change detection mode of the configuration.
pub(super) fn track_mode(mode: TrackChanges) -> TrackMode {
    match mode {
        TrackChanges::Scan => TrackMode::Scan,
        TrackChanges::Events => TrackMode::Events,
        TrackChanges::Disabled => TrackMode::None,
        _ => TrackMode::Auto,
    }
}

fn track_mode_named(mode: &str) -> Result<TrackMode> {
    match mode {
        "auto" => Ok(TrackMode::Auto),
        "scan" => Ok(TrackMode::Scan),
        "events" => Ok(TrackMode::Events),
        "none" => Ok(TrackMode::None),
        other => Err(invalid(
            "track_changes",
            &format!("does not accept `{other}`; use auto, scan, events or none"),
        )),
    }
}

/// `None` for a zero duration, which disables a limit.
pub(super) fn non_zero(duration: Duration) -> Option<Duration> {
    (!duration.is_zero()).then_some(duration)
}

/// Value of a parameter the program gave; a default inserted by the binder
/// counts as not given, so the configuration applies.
fn given(args: &BoundArgs, ctx: &ExecCtx, name: &str) -> Result<Option<Value>> {
    if args.is_defaulted(name) {
        return Ok(None);
    }
    match args.get(name) {
        None | Some(ArgValue::Absent) => Ok(None),
        Some(_) => args.eval(name, None, ctx),
    }
}

fn text(args: &BoundArgs, ctx: &ExecCtx, name: &str) -> Result<Option<String>> {
    let value = match args.get(name) {
        Some(ArgValue::Absent) | None => None,
        Some(_) => args.eval(name, None, ctx)?,
    };
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Text(text) | Value::Enum(text)) => Ok(Some(text)),
        Some(other) => Err(type_error(name, "text", &other)),
    }
}

fn flag(args: &BoundArgs, ctx: &ExecCtx, name: &str) -> Result<Option<bool>> {
    match args.eval(name, None, ctx)? {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(flag)) => Ok(Some(flag)),
        Some(other) => Err(type_error(name, "bool", &other)),
    }
}

/// Positive count; values below 1 are rejected.
fn count(args: &BoundArgs, ctx: &ExecCtx, name: &str) -> Result<Option<usize>> {
    let value = if name == "parallel" {
        given(args, ctx, name)?
    } else {
        args.eval(name, None, ctx)?
    };
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Int(number)) if number >= 1 => {
            Ok(Some(usize::try_from(number).unwrap_or(usize::MAX)))
        }
        Some(Value::Int(_)) => Err(invalid(name, "must be 1 or more")),
        Some(other) => Err(type_error(name, "int", &other)),
    }
}

fn lines(args: &BoundArgs, ctx: &ExecCtx, name: &str, configured: u32) -> Result<u32> {
    match given(args, ctx, name)? {
        None => Ok(configured),
        Some(Value::Int(number)) => {
            u32::try_from(number).map_err(|_| invalid(name, "must be 0 or more"))
        }
        Some(other) => Err(type_error(name, "int", &other)),
    }
}

fn std_duration(name: &str, value: &Value) -> Result<Duration> {
    match value {
        Value::Duration(duration) => duration
            .to_std()
            .ok_or_else(|| invalid(name, "must not be negative")),
        other => Err(type_error(name, "duration", other)),
    }
}

fn duration_or(
    args: &BoundArgs,
    ctx: &ExecCtx,
    name: &str,
    configured: Duration,
) -> Result<Duration> {
    match given(args, ctx, name)? {
        Some(value) => std_duration(name, &value),
        None => Ok(configured),
    }
}

fn optional_duration(args: &BoundArgs, ctx: &ExecCtx, name: &str) -> Result<Option<Duration>> {
    match args.eval(name, None, ctx)? {
        None | Some(Value::Null) => Ok(None),
        Some(value) => std_duration(name, &value).map(Some),
    }
}

fn wait(args: &BoundArgs, ctx: &ExecCtx, config: &Config) -> Result<Option<Duration>> {
    if flag(args, ctx, "background")? == Some(true) {
        return Ok(Some(Duration::ZERO));
    }
    match given(args, ctx, "wait")? {
        Some(Value::Enum(word) | Value::Text(word)) if word == "forever" => Ok(None),
        Some(value) => std_duration("wait", &value).map(Some),
        None => Ok(Some(config.run.wait.as_std())),
    }
}

/// Absolute path of a value; a relative path is taken against `base`.
fn path_value(name: &str, value: &Value, base: Option<&Path>) -> Result<PathBuf> {
    let text = match value {
        Value::Path(text) | Value::Text(text) => text,
        other => return Err(type_error(name, "path", other)),
    };
    let path = PathBuf::from(text);
    if path.is_absolute() {
        return Ok(path);
    }
    match base {
        Some(base) => Ok(base.join(path)),
        None => Err(Error::new(
            ErrorCode::PathRelative,
            format!(
                "`{name}` is the relative path {text}; give an absolute path or the call parameter `base`"
            ),
        )),
    }
}

fn working_directory(args: &BoundArgs, ctx: &ExecCtx) -> Result<PathBuf> {
    let base = ctx.options().base.as_deref().map(PathBuf::from);
    match args.eval("cwd", None, ctx)? {
        Some(Value::Null) | None => base.ok_or_else(|| {
            Error::new(
                ErrorCode::MissingParam,
                "`run` needs a working directory; give `cwd` or the call parameter `base`",
            )
        }),
        Some(value) => path_value("cwd", &value, base.as_deref()),
    }
}

fn environment(args: &BoundArgs, ctx: &ExecCtx) -> Result<Vec<EnvVar>> {
    match args.eval("env", None, ctx)? {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Map(fields)) => fields
            .into_iter()
            .map(|(name, value)| {
                let value = match value {
                    Value::Null => None,
                    other => Some(
                        element_text(&other)
                            .map_err(|_| type_error("env", "text values or null", &other))?,
                    ),
                };
                Ok(EnvVar { name, value })
            })
            .collect(),
        Some(other) => Err(type_error("env", "map<text>", &other)),
    }
}

fn stdin(args: &BoundArgs, ctx: &ExecCtx, cwd: &Path) -> Result<StdinSource> {
    match args.eval("stdin", None, ctx)? {
        None | Some(Value::Null) => Ok(StdinSource::Null),
        Some(Value::Text(text)) => Ok(StdinSource::Bytes(text.into_bytes())),
        Some(Value::Bytes(bytes)) => Ok(StdinSource::Bytes(bytes.into_vec())),
        Some(value @ Value::Path(_)) => {
            let path = path_value("stdin", &value, Some(cwd))?;
            if path.exists() && !path.is_dir() {
                Ok(StdinSource::File(path))
            } else {
                Err(Error::new(
                    ErrorCode::NotFound,
                    format!(
                        "the `stdin` file {} does not exist; give an existing file or text",
                        path.display()
                    ),
                ))
            }
        }
        Some(other) => Err(type_error("stdin", "text or path", &other)),
    }
}

fn watch_paths(args: &BoundArgs, ctx: &ExecCtx, cwd: &Path) -> Result<Vec<PathBuf>> {
    match args.eval("watch", None, ctx)? {
        None | Some(Value::Null) => Ok(vec![cwd.to_path_buf()]),
        Some(Value::List(paths)) => paths
            .iter()
            .map(|path| path_value("watch", path, Some(cwd)))
            .collect(),
        Some(other) => Err(type_error("watch", "list<path>", &other)),
    }
}

fn regex_text(name: &str, value: &Value) -> Result<String> {
    match value {
        Value::Regex(regex) if regex.flags.is_empty() => Ok(regex.pattern.clone()),
        Value::Regex(regex) => Ok(format!("(?{}){}", regex.flags, regex.pattern)),
        Value::Text(text) => Ok(text.clone()),
        other => Err(type_error(name, "regex", other)),
    }
}

fn optional_regex(args: &BoundArgs, ctx: &ExecCtx, name: &str) -> Result<Option<String>> {
    match args.eval(name, None, ctx)? {
        None | Some(Value::Null) => Ok(None),
        Some(value) => regex_text(name, &value).map(Some),
    }
}

fn output_view(args: &BoundArgs, ctx: &ExecCtx, config: &Config) -> Result<OutputView> {
    let show = match text(args, ctx, "show")?.as_deref() {
        None | Some("auto") => ShowMode::Auto,
        Some("both") => ShowMode::Both,
        Some("stdout") => ShowMode::Stdout,
        Some("stderr") => ShowMode::Stderr,
        Some("errors") => ShowMode::Errors,
        Some("summary") => ShowMode::Summary,
        Some("none") => ShowMode::None,
        Some(other) => {
            return Err(invalid(
                "show",
                &format!(
                    "does not accept `{other}`; use auto, both, stdout, stderr, errors, summary or none"
                ),
            ));
        }
    };
    let summary = match args.eval("summary", None, ctx)? {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::List(patterns)) => patterns
            .iter()
            .map(|pattern| regex_text("summary", pattern))
            .collect::<Result<_>>()?,
        Some(other) => vec![regex_text("summary", &other)?],
    };
    let context = match args.eval("context", None, ctx)? {
        None | Some(Value::Null) => 0,
        Some(Value::Int(number)) => {
            u32::try_from(number).map_err(|_| invalid("context", "must be 0 or more"))?
        }
        Some(other) => return Err(type_error("context", "int", &other)),
    };
    let compact = show == ShowMode::Auto
        && given(args, ctx, "head")?.is_none()
        && given(args, ctx, "tail")?.is_none()
        && optional_regex(args, ctx, "filter")?.is_none();
    Ok(OutputView {
        show,
        compact,
        max_bytes: lines(args, ctx, "max_bytes", config.output.max_bytes)?,
        head: Some(lines(args, ctx, "head", config.output.head)?),
        tail: Some(lines(args, ctx, "tail", config.output.tail)?),
        filter: optional_regex(args, ctx, "filter")?,
        context,
        dedupe: flag(args, ctx, "dedupe")?.unwrap_or(true),
        quiet_on_success: flag(args, ctx, "quiet_on_success")?.unwrap_or(false),
        summary,
        progress: optional_regex(args, ctx, "progress")?,
        encoding: text(args, ctx, "output_encoding")?.filter(|encoding| encoding != "auto"),
    })
}

fn type_error(name: &str, expected: &str, found: &Value) -> Error {
    Error::new(
        ErrorCode::Type,
        format!(
            "`run` expects {expected} for `{name}`, not {}; change the value",
            found.type_name()
        ),
    )
}

fn invalid(name: &str, reason: &str) -> Error {
    Error::new(ErrorCode::InvalidValue, format!("`{name}` {reason}"))
}
