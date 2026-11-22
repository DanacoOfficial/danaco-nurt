// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The `analyze.check`, `analyze.show`, `analyze.format` and `analyze.tools`
//! commands.

use std::path::PathBuf;
use std::sync::Arc;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::record::FindingSeverity;
use nurt_contracts::stage::{BoundArgs, ExecCtx, StageImpl};
use nurt_contracts::value::Value;

use crate::stage::{analyze_check, analyze_format, analyze_show, analyze_tools};

/// Stages of the analysis commands.
pub(crate) fn stages() -> Vec<Arc<dyn StageImpl>> {
    vec![
        Arc::new(analyze_check::Check),
        Arc::new(analyze_show::Show),
        Arc::new(analyze_format::Format),
        Arc::new(analyze_tools::Tools),
    ]
}

/// Project directory: the base of the request or the working directory.
pub(crate) fn base_dir(ctx: &ExecCtx) -> Result<PathBuf> {
    match &ctx.options().base {
        Some(base) => Ok(PathBuf::from(base)),
        None => std::env::current_dir().map_err(|error| {
            Error::new(
                ErrorCode::Arg,
                format!("cannot read the working directory: {error}"),
            )
        }),
    }
}

/// Text of an argument given as text, enum or path.
pub(crate) fn text_arg(args: &BoundArgs, name: &str) -> Option<String> {
    match args.value(name)? {
        Value::Text(text) | Value::Enum(text) | Value::Path(text) => Some(text.clone()),
        _ => None,
    }
}

/// Elements of a list argument, or the single value of the argument.
pub(crate) fn list_arg(args: &BoundArgs, name: &str) -> Vec<String> {
    match args.value(name) {
        Some(Value::List(items)) => items
            .iter()
            .filter_map(|item| match item {
                Value::Text(text) | Value::Enum(text) | Value::Path(text) => Some(text.clone()),
                _ => None,
            })
            .collect(),
        Some(Value::Text(text) | Value::Enum(text) | Value::Path(text)) => vec![text.clone()],
        _ => Vec::new(),
    }
}

/// Boolean argument with a default.
pub(crate) fn bool_arg(args: &BoundArgs, name: &str, default: bool) -> bool {
    args.value(name).and_then(Value::as_bool).unwrap_or(default)
}

/// Non-negative integer argument with a default.
pub(crate) fn count_arg(args: &BoundArgs, name: &str, default: usize) -> usize {
    args.value(name)
        .and_then(Value::as_int)
        .and_then(|number| usize::try_from(number).ok())
        .unwrap_or(default)
}

/// Severity argument with a default.
pub(crate) fn severity_arg(
    args: &BoundArgs,
    name: &str,
    default: FindingSeverity,
) -> FindingSeverity {
    text_arg(args, name)
        .and_then(|text| text.parse().ok())
        .unwrap_or(default)
}
