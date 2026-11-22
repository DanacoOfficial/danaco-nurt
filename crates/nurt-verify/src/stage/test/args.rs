// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Reading of the bound arguments of the test commands.

use std::path::PathBuf;
use std::time::Duration;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::stage::{BoundArgs, ExecCtx};
use nurt_contracts::value::Value;
use nurt_proc::ToolKind;

pub(super) fn text(args: &BoundArgs, name: &str) -> Result<Option<String>> {
    match args.value(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Text(text) | Value::Enum(text)) => Ok(Some(text.clone())),
        Some(_) => Err(invalid(args, name, "text")),
    }
}

pub(super) fn flag(args: &BoundArgs, name: &str) -> Result<bool> {
    Ok(args.bool(name)?.unwrap_or(false))
}

/// Count parameter with a default; negative values are rejected.
pub(super) fn count(args: &BoundArgs, name: &str, default: usize) -> Result<usize> {
    match args.int(name)? {
        None => Ok(default),
        Some(value) => usize::try_from(value).map_err(|_| {
            Error::new(
                ErrorCode::InvalidValue,
                format!("`{name}` must not be negative"),
            )
            .with_fix(format!("{name}: {default}"))
        }),
    }
}

pub(super) fn list(args: &BoundArgs, name: &str) -> Result<Vec<String>> {
    match args.value(name) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::List(items)) => items
            .iter()
            .map(|item| {
                item.as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| invalid(args, name, "list<text>"))
            })
            .collect(),
        Some(_) => Err(invalid(args, name, "list<text>")),
    }
}

pub(super) fn duration(args: &BoundArgs, name: &str) -> Result<Option<Duration>> {
    match args.value(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Duration(value)) => Ok(value.to_std()),
        Some(_) => Err(invalid(args, name, "duration")),
    }
}

pub(super) fn path(args: &BoundArgs, name: &str) -> Result<Option<PathBuf>> {
    match args.value(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Path(path) | Value::Text(path)) => Ok(Some(PathBuf::from(path))),
        Some(_) => Err(invalid(args, name, "path")),
    }
}

/// Project directory: the `path` argument, else the base directory of the
/// request or the configuration, else the working directory.
pub(super) fn project_dir(ctx: &ExecCtx, args: &BoundArgs) -> Result<PathBuf> {
    if let Some(path) = path(args, "path")? {
        return Ok(path);
    }
    base_dir(ctx)
}

pub(super) fn base_dir(ctx: &ExecCtx) -> Result<PathBuf> {
    if let Some(base) = ctx.options().base.as_ref().or(ctx.config().scope.base.as_ref()) {
        return Ok(PathBuf::from(base));
    }
    std::env::current_dir().map_err(|failure| {
        Error::new(
            ErrorCode::Io,
            format!("cannot read the working directory: {failure}"),
        )
        .with_fix("set scope.base")
    })
}

/// Targets must name tests, not options of the framework.
pub(super) fn check_targets(targets: &[String]) -> Result<()> {
    match targets.iter().find(|target| target.starts_with('-')) {
        Some(target) => Err(Error::new(
            ErrorCode::InvalidValue,
            format!("target `{target}` looks like an option"),
        )
        .with_fix("pass options of the framework through command:")),
        None => Ok(()),
    }
}

/// Kind of the tool runs of the test commands.
pub(super) const TOOL_KIND: ToolKind = ToolKind::Test;

fn invalid(args: &BoundArgs, name: &str, expected: &str) -> Error {
    Error::new(
        ErrorCode::Type,
        format!("`{}` expects {expected} for `{name}`", args.command()),
    )
}
