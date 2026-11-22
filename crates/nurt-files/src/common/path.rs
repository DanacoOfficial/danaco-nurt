// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Paths of command arguments and input records.
//!
//! A path in a program or a record is absolute or relative to the base
//! directory: the `base` option of the request, otherwise the configuration
//! key `scope.base`. The language has no current directory, so a relative
//! path without a base directory fails with `E_PATH_RELATIVE`.

use std::path::PathBuf;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::AbsPath;
use nurt_contracts::stage::{BoundArgs, ExecCtx};
use nurt_contracts::value::{self, Record, Value};

/// Field of input records that holds the path of a file.
pub const PATH_FIELD: &str = "path";

/// Base directory for relative paths: the `base` option of the request,
/// otherwise `scope.base` of the configuration; `None` when neither is set.
///
/// # Errors
///
/// Fails with `E_CONFIG` when the base directory is not an absolute path.
pub fn base_dir(ctx: &ExecCtx) -> Result<Option<AbsPath>> {
    let (base, source) = match (
        ctx.options().base.as_deref(),
        ctx.config().scope.base.as_deref(),
    ) {
        (Some(base), _) => (base, "base"),
        (None, Some(base)) => (base, "scope.base"),
        (None, None) => return Ok(None),
    };
    let written = PathBuf::from(base);
    let absolute = written
        .is_absolute()
        .then(|| AbsPath::new(written).ok())
        .flatten();
    absolute.map(Some).ok_or_else(|| {
        Error::new(
            ErrorCode::Config,
            format!("`{source}` must be an absolute directory, not `{base}`"),
        )
        .with_found(base)
    })
}

/// Resolves a path written in a program or a record.
///
/// An absolute path is normalised; a relative path is joined to
/// [`base_dir`].
///
/// # Errors
///
/// Fails with `E_PATH_RELATIVE` for a relative path without a base
/// directory, with `E_INVALID_VALUE` for an empty path and for a path that
/// leads above the root directory, and as [`base_dir`].
pub fn resolve(ctx: &ExecCtx, text: &str) -> Result<AbsPath> {
    if text.is_empty() {
        return Err(Error::new(
            ErrorCode::InvalidValue,
            "a path cannot be empty: give a file or directory",
        ));
    }
    let written = PathBuf::from(text);
    let joined = if written.is_absolute() {
        written
    } else {
        let Some(base) = base_dir(ctx)? else {
            return Err(Error::new(
                ErrorCode::PathRelative,
                format!(
                    "relative path `{text}` needs a base directory: give an absolute path or set the `base` option"
                ),
            )
            .with_found(text));
        };
        base.as_path().join(written)
    };
    AbsPath::new(joined).map_err(|_| {
        Error::new(
            ErrorCode::InvalidValue,
            format!("path `{text}` leads above the root directory"),
        )
        .with_found(text)
    })
}

/// Path given for the parameter `name`; `None` when the parameter has no
/// value.
///
/// # Errors
///
/// Fails with `E_TYPE` when the value is neither a path nor a text, and as
/// [`resolve`].
pub fn path_arg(args: &BoundArgs, name: &str, ctx: &ExecCtx) -> Result<Option<AbsPath>> {
    match args.value(name) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => path_value(args.command(), name, value, ctx).map(Some),
    }
}

/// Paths given for a `list<path>` parameter, in order; a single path is a
/// list of one.
///
/// # Errors
///
/// Fails with `E_TYPE` when an item is neither a path nor a text, and as
/// [`resolve`].
pub fn path_list_arg(args: &BoundArgs, name: &str, ctx: &ExecCtx) -> Result<Vec<AbsPath>> {
    match args.value(name) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::List(items)) => items
            .iter()
            .map(|item| path_value(args.command(), name, item, ctx))
            .collect(),
        Some(value) => path_value(args.command(), name, value, ctx).map(|path| vec![path]),
    }
}

fn path_value(command: &str, name: &str, value: &Value, ctx: &ExecCtx) -> Result<AbsPath> {
    match value {
        Value::Path(text) | Value::Text(text) => resolve(ctx, text),
        _ => Err(Error::new(
            ErrorCode::Type,
            format!("`{command}` expects a path for `{name}`"),
        )),
    }
}

/// Path of an input record, from its [`PATH_FIELD`] field.
///
/// # Errors
///
/// Fails with `E_TYPE` when the record has no such field or the field is
/// not a path, and as [`resolve`].
pub fn record_path(record: &Record, ctx: &ExecCtx) -> Result<AbsPath> {
    let kind = record.kind.as_str();
    match record.get(PATH_FIELD) {
        Some(Value::Path(text) | Value::Text(text)) => resolve(ctx, text),
        Some(_) => Err(Error::new(
            ErrorCode::Type,
            format!("the `{PATH_FIELD}` field of a `{kind}` record is not a path"),
        )),
        None => Err(Error::new(
            ErrorCode::Type,
            format!("a `{kind}` record has no `{PATH_FIELD}` field: pass records of files"),
        )),
    }
}

/// Path as the value of a record field, in language form.
#[must_use]
pub fn path_field(path: &AbsPath) -> value::Path {
    value::Path::new(path.to_language_string())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use nurt_contracts::config::Config;
    use nurt_contracts::envelope::RequestOptions;
    use nurt_contracts::stage::ArgValue;
    use nurt_contracts::value::RecordKind;
    use nurt_testkit::ctx::TestContext;

    use super::*;
    use crate::common::fixture::abs;

    fn ctx_with_base(option: Option<&str>, config: Option<&str>) -> ExecCtx {
        let options = RequestOptions {
            base: option.map(str::to_owned),
            ..RequestOptions::default()
        };
        let mut settings = Config::default();
        settings.scope.base = config.map(str::to_owned);
        TestContext::new()
            .builder()
            .options(Arc::new(options))
            .config(Arc::new(settings))
            .build()
    }

    fn language(text: &str) -> String {
        abs(text).to_language_string()
    }

    fn args(value: Value) -> BoundArgs {
        BoundArgs::new("info").with("path", ArgValue::Value(value), false)
    }

    #[test]
    fn absolute_paths_are_normalised() {
        let ctx = ctx_with_base(None, None);
        let written = format!("{}/./b/../c.txt", language("/data"));
        assert_eq!(resolve(&ctx, &written).unwrap(), abs("/data/c.txt"));
    }

    #[test]
    fn relative_paths_use_the_base_option_first() {
        let ctx = ctx_with_base(Some(&language("/request")), Some(&language("/config")));
        assert_eq!(
            resolve(&ctx, "sub/a.txt").unwrap(),
            abs("/request/sub/a.txt")
        );
        assert_eq!(base_dir(&ctx).unwrap(), Some(abs("/request")));
    }

    #[test]
    fn relative_paths_fall_back_to_the_configured_base() {
        let ctx = ctx_with_base(None, Some(&language("/config")));
        assert_eq!(resolve(&ctx, "a.txt").unwrap(), abs("/config/a.txt"));
    }

    #[test]
    fn relative_path_without_base_is_rejected() {
        let ctx = ctx_with_base(None, None);
        let error = resolve(&ctx, "a.txt").unwrap_err();
        assert_eq!(error.code(), ErrorCode::PathRelative);
        assert_eq!(error.found.as_deref(), Some("a.txt"));
        assert_eq!(base_dir(&ctx).unwrap(), None);
    }

    #[test]
    fn relative_base_is_a_configuration_error() {
        let ctx = ctx_with_base(None, Some("projects"));
        let error = resolve(&ctx, "a.txt").unwrap_err();
        assert_eq!(error.code(), ErrorCode::Config);
        assert!(error.message.contains("scope.base"));
    }

    #[test]
    fn empty_and_escaping_paths_are_invalid() {
        let ctx = ctx_with_base(Some(&language("/base")), None);
        assert_eq!(
            resolve(&ctx, "").unwrap_err().code(),
            ErrorCode::InvalidValue
        );
        let escaping = format!("{}/../..", language("/"));
        assert_eq!(
            resolve(&ctx, &escaping).unwrap_err().code(),
            ErrorCode::InvalidValue
        );
    }

    #[test]
    fn path_arguments_accept_paths_and_texts() {
        let ctx = ctx_with_base(Some(&language("/base")), None);
        let given = args(Value::Path(language("/data/a.txt")));
        assert_eq!(
            path_arg(&given, "path", &ctx).unwrap(),
            Some(abs("/data/a.txt"))
        );
        let text = args(Value::Text("b.txt".to_owned()));
        assert_eq!(
            path_arg(&text, "path", &ctx).unwrap(),
            Some(abs("/base/b.txt"))
        );
        assert_eq!(path_arg(&args(Value::Null), "path", &ctx).unwrap(), None);
        assert_eq!(path_arg(&given, "other", &ctx).unwrap(), None);
        let wrong = path_arg(&args(Value::Int(1)), "path", &ctx).unwrap_err();
        assert_eq!(wrong.code(), ErrorCode::Type);
        assert!(wrong.message.contains("`info`"));
    }

    #[test]
    fn path_lists_accept_lists_and_single_paths() {
        let ctx = ctx_with_base(Some(&language("/base")), None);
        let list = args(Value::List(vec![
            Value::Text("a".to_owned()),
            Value::Path(language("/data/b")),
        ]));
        assert_eq!(
            path_list_arg(&list, "path", &ctx).unwrap(),
            [abs("/base/a"), abs("/data/b")]
        );
        let single = args(Value::Text("c".to_owned()));
        assert_eq!(
            path_list_arg(&single, "path", &ctx).unwrap(),
            [abs("/base/c")]
        );
        assert!(
            path_list_arg(&args(Value::Null), "path", &ctx)
                .unwrap()
                .is_empty()
        );
        let bad = args(Value::List(vec![Value::Bool(true)]));
        assert_eq!(
            path_list_arg(&bad, "path", &ctx).unwrap_err().code(),
            ErrorCode::Type
        );
    }

    #[test]
    fn record_paths_are_read_from_the_path_field() {
        let ctx = ctx_with_base(Some(&language("/base")), None);
        let entry = Record::new(RecordKind::Entry).with("path", Value::Path(language("/x")));
        assert_eq!(record_path(&entry, &ctx).unwrap(), abs("/x"));
        let text = Record::new(RecordKind::Entry).with("path", Value::Text("y".to_owned()));
        assert_eq!(record_path(&text, &ctx).unwrap(), abs("/base/y"));
        let wrong = Record::new(RecordKind::Entry).with("path", Value::Int(3));
        assert_eq!(
            record_path(&wrong, &ctx).unwrap_err().code(),
            ErrorCode::Type
        );
        let missing = record_path(&Record::new(RecordKind::Entry), &ctx).unwrap_err();
        assert_eq!(missing.code(), ErrorCode::Type);
        assert!(missing.message.contains("`entry`"));
    }

    #[test]
    fn path_fields_use_the_language_form() {
        assert_eq!(
            path_field(&abs("/data/a.txt")).as_str(),
            language("/data/a.txt")
        );
    }
}
