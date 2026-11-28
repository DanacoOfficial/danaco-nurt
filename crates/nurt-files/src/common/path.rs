// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-files::common::path
// Responsibility:   Paths of command arguments and input records.
// Dependencies:     nurt_contracts, nurt_testkit
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Paths of command arguments and input records.
//!
//! A path in a program or a record is absolute or relative to the base
//! directory: the `base` option of the request, otherwise the configuration
//! key `scope.base`. The language has no current directory, so a relative
//! path without a base directory fails with `E_PATH_RELATIVE`.

use std::path::{Component, Path, PathBuf};

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::{AbsPath, FileKind, Follow, Fs};
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
    normalise(text, join_text(ctx, text)?)
}

/// Resolves a path that a changing command acts on.
///
/// As [`resolve`], and moreover refuses a path in which `..` follows a
/// symbolic link: the system continues from the target of the link, the
/// normalisation of the language from the directory that holds the link, so
/// the command could remove, replace or write another entry than the one
/// written. The error carries the path without `..` as its fix. Commands that
/// only read keep the lexical meaning of [`resolve`].
///
/// # Errors
///
/// Fails with `E_INVALID_VALUE` for a `..` after a link, and as [`resolve`].
pub fn resolve_for_change(ctx: &ExecCtx, text: &str) -> Result<AbsPath> {
    let joined = join_text(ctx, text)?;
    let lexical = normalise(text, joined.clone())?;
    if let Some(fs) = ctx.fs() {
        ensure_same_entry(fs, text, &joined, &lexical)?;
    }
    Ok(lexical)
}

/// Fails when a `..` of `joined` follows a symbolic link or a junction.
fn ensure_same_entry(fs: &dyn Fs, text: &str, joined: &Path, lexical: &AbsPath) -> Result<()> {
    let mut walked = PathBuf::new();
    for component in joined.components() {
        match component {
            Component::ParentDir => {
                if is_link(fs, &walked) {
                    return Err(Error::new(
                        ErrorCode::InvalidValue,
                        format!(
                            "`..` in `{text}` follows the link `{}`: the system continues from the target of the link, the language from the directory that holds it; write `{lexical}` or the path through the target",
                            walked.display()
                        ),
                    )
                    .with_found(text)
                    .with_fix(format!("\"{}\"", lexical.to_language_string())));
                }
                walked.pop();
            }
            Component::CurDir => {}
            other => walked.push(other.as_os_str()),
        }
    }
    Ok(())
}

fn is_link(fs: &dyn Fs, path: &Path) -> bool {
    AbsPath::new(path)
        .ok()
        .and_then(|abs| fs.metadata(&abs, Follow::NoFollow).ok())
        .is_some_and(|meta| matches!(meta.kind, FileKind::Symlink | FileKind::Junction))
}

/// Absolute path written in `text`, relative paths joined to the base
/// directory, before normalisation.
fn join_text(ctx: &ExecCtx, text: &str) -> Result<PathBuf> {
    if text.is_empty() {
        return Err(Error::new(
            ErrorCode::InvalidValue,
            "a path cannot be empty: give a file or directory",
        ));
    }
    if crate::registry::is_registry_path(text) {
        return Err(Error::new(
            ErrorCode::InvalidValue,
            format!(
                "`{text}` is a registry path, which only `list`, `find`, `read`, `info`, `write`, `remove`, `copy` and `move` take: give a file path here"
            ),
        )
        .with_found(text));
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
    Ok(joined)
}

fn normalise(text: &str, joined: PathBuf) -> Result<AbsPath> {
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
        Some(value) => path_value(args.command(), name, value, ctx, resolve).map(Some),
    }
}

/// As [`path_arg`], for a path a changing command acts on; see
/// [`resolve_for_change`].
///
/// # Errors
///
/// As [`path_arg`] and [`resolve_for_change`].
pub fn path_arg_for_change(args: &BoundArgs, name: &str, ctx: &ExecCtx) -> Result<Option<AbsPath>> {
    match args.value(name) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => path_value(args.command(), name, value, ctx, resolve_for_change).map(Some),
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
    path_list(args, name, ctx, resolve)
}

/// As [`path_list_arg`], for paths a changing command acts on; see
/// [`resolve_for_change`].
///
/// # Errors
///
/// As [`path_list_arg`] and [`resolve_for_change`].
pub fn path_list_arg_for_change(
    args: &BoundArgs,
    name: &str,
    ctx: &ExecCtx,
) -> Result<Vec<AbsPath>> {
    path_list(args, name, ctx, resolve_for_change)
}

fn path_list(
    args: &BoundArgs,
    name: &str,
    ctx: &ExecCtx,
    resolver: fn(&ExecCtx, &str) -> Result<AbsPath>,
) -> Result<Vec<AbsPath>> {
    match args.value(name) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::List(items)) => items
            .iter()
            .map(|item| path_value(args.command(), name, item, ctx, resolver))
            .collect(),
        Some(value) => {
            path_value(args.command(), name, value, ctx, resolver).map(|path| vec![path])
        }
    }
}

fn path_value(
    command: &str,
    name: &str,
    value: &Value,
    ctx: &ExecCtx,
    resolver: fn(&ExecCtx, &str) -> Result<AbsPath>,
) -> Result<AbsPath> {
    match value {
        Value::Path(text) | Value::Text(text) => resolver(ctx, text),
        _ => Err(Error::new(
            ErrorCode::Type,
            format!("`{command}` expects a path for `{name}`"),
        )),
    }
}

/// Field of input records that holds the native bytes of a path that is not
/// valid Unicode; see [`PATH_FIELD`].
pub const RAW_PATH_FIELD: &str = "raw_path";

/// Path of the [`RAW_PATH_FIELD`] field of a record, when it is an absolute
/// path and the [`PATH_FIELD`] field, if the record has one, is still its
/// display form: a stage that wrote another `path` named another file.
fn raw_record_path(record: &Record) -> Option<AbsPath> {
    let Some(Value::Bytes(raw)) = record.get(RAW_PATH_FIELD) else {
        return None;
    };
    let path = AbsPath::from_raw_bytes(raw.as_slice()).ok()?;
    match record.get(PATH_FIELD) {
        Some(Value::Path(text) | Value::Text(text)) if *text != path.to_language_string() => None,
        _ => Some(path),
    }
}

/// Path of an input record, from its [`PATH_FIELD`] field, or from its
/// [`RAW_PATH_FIELD`] field when the record has one: the text of a path that is
/// not valid Unicode shows the invalid bytes as U+FFFD and does not lead back
/// to the entry.
///
/// # Errors
///
/// Fails with `E_TYPE` when the record has no such field or the field is
/// not a path, and as [`resolve`].
pub fn record_path(record: &Record, ctx: &ExecCtx) -> Result<AbsPath> {
    let kind = record.kind.as_str();
    if let Some(path) = raw_record_path(record) {
        return Ok(path);
    }
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

    fn world_with_a_link() -> TestContext {
        let test = TestContext::new();
        test.fs().create_dir_all(&abs("/real/inner")).unwrap();
        test.fs().create_dir_all(&abs("/area")).unwrap();
        test.fs().write_file(&abs("/area/b.txt"), b"b").unwrap();
        test.fs().write_file(&abs("/real/b.txt"), b"r").unwrap();
        test.fs()
            .symlink("/real/inner", &abs("/area/link"))
            .unwrap();
        test
    }

    #[test]
    fn a_changing_command_refuses_dot_dot_after_a_link_and_offers_the_path_without_it() {
        let test = world_with_a_link();
        let ctx = test.ctx();
        let written = format!("{}/link/../b.txt", language("/area"));
        assert_eq!(resolve(&ctx, &written).unwrap(), abs("/area/b.txt"));
        let error = resolve_for_change(&ctx, &written).unwrap_err();
        assert_eq!(error.code(), ErrorCode::InvalidValue);
        assert_eq!(error.found.as_deref(), Some(written.as_str()));
        assert_eq!(error.fix, [format!("\"{}\"", language("/area/b.txt"))]);
    }

    #[test]
    fn dot_dot_after_a_directory_or_without_a_link_is_kept_for_a_changing_command() {
        let test = world_with_a_link();
        let ctx = test.ctx();
        for written in [
            format!("{}/./b.txt", language("/area")),
            format!("{}/../b.txt", language("/real/inner")),
            format!("{}/link/x/../../b.txt", language("/area")),
            language("/area/b.txt"),
        ] {
            let outcome = resolve_for_change(&ctx, &written);
            if written.contains("/link/") {
                assert!(outcome.is_err(), "{written}");
            } else {
                assert!(outcome.is_ok(), "{written}: {outcome:?}");
            }
        }
    }

    #[test]
    fn path_arguments_for_change_use_the_same_rule() {
        let test = world_with_a_link();
        let ctx = test.ctx();
        let written = format!("{}/link/../b.txt", language("/area"));
        let given = args(Value::Path(written.clone()));
        assert!(path_arg(&given, "path", &ctx).is_ok());
        assert!(path_arg_for_change(&given, "path", &ctx).is_err());
        let listed = BoundArgs::new("remove").with(
            "path",
            ArgValue::Value(Value::List(vec![Value::Path(written)])),
            false,
        );
        assert!(path_list_arg(&listed, "path", &ctx).is_ok());
        assert!(path_list_arg_for_change(&listed, "path", &ctx).is_err());
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

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
