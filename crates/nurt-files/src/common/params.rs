// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-files::common::params
// Responsibility:   Parameters shared by several commands.
// Dependencies:     nurt_contracts, nurt_testkit, nurt_text
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Parameters shared by several commands.
//!
//! | Parameter | Commands |
//! |---|---|
//! | `encoding` | `read`, `write`, `convert` |
//! | `line_ending` | `write`, `convert` |
//! | `on_missing` | `remove`, `replace`, `edit` |
//! | `if_match` | `write`, `replace`, `insert`, `edit`, `patch` |
//! | `hidden`, `gitignore`, `exclude`, `exclude_preset` | `list`, `find`, `search`, `replace` |
//!
//! A parameter without a value takes the default of the catalog.

use std::borrow::Cow;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::{AbsPath, Etag};
use nurt_contracts::services::ExcludePreset;
use nurt_contracts::stage::{BoundArgs, ExecCtx};
use nurt_contracts::value::Value;
use nurt_text::codec::Codec;
use nurt_text::eol::{self, Terminator};

use super::etag::parse_etag;
use super::path::resolve;
use crate::walk::WalkOptions;

/// Name of the `encoding` parameter.
pub const ENCODING: &str = "encoding";
/// Name of the `line_ending` parameter.
pub const LINE_ENDING: &str = "line_ending";
/// Name of the `on_missing` parameter.
pub const ON_MISSING: &str = "on_missing";
/// Name of the `if_match` parameter.
pub const IF_MATCH: &str = "if_match";
/// Name of the `hidden` parameter of the walking commands.
pub const HIDDEN: &str = "hidden";
/// Name of the `gitignore` parameter.
pub const GITIGNORE: &str = "gitignore";
/// Name of the `exclude` parameter.
pub const EXCLUDE: &str = "exclude";
/// Name of the `exclude_preset` parameter.
pub const EXCLUDE_PRESET: &str = "exclude_preset";

/// Value of `encoding` that selects detection.
pub const AUTO_ENCODING: &str = "auto";

const LINE_ENDING_VALUES: [&str; 4] = ["lf", "crlf", "cr", "keep"];
const ON_MISSING_VALUES: [&str; 2] = ["skip", "error"];
const PRESET_VALUES: [&str; 4] = ["vcs", "deps", "build", "cache"];

/// Value of the `encoding` parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncodingParam {
    /// `auto`: detect the encoding, or keep the encoding of an existing
    /// file.
    Auto,
    /// A named encoding.
    Codec(Codec),
}

/// Reads `encoding`; `None` when the parameter has no value.
///
/// The label is matched as by [`Codec::from_label`], so aliases such as
/// `cp1250` and `latin2` are accepted.
///
/// # Errors
///
/// Fails with `E_TYPE` for a value that is not a text and with
/// `E_UNKNOWN_VALUE` for a label of no supported encoding.
pub fn encoding_param(args: &BoundArgs) -> Result<Option<EncodingParam>> {
    let Some(label) = args.text(ENCODING)? else {
        return Ok(None);
    };
    if label.trim().eq_ignore_ascii_case(AUTO_ENCODING) {
        return Ok(Some(EncodingParam::Auto));
    }
    Codec::from_label(label)
        .map(|codec| Some(EncodingParam::Codec(codec)))
        .map_err(|error| error.into_error(None))
}

/// Value of the `line_ending` parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineEndingParam {
    /// `keep`: leave the line endings as they are.
    Keep,
    /// Convert every line ending to the terminator.
    Convert(Terminator),
}

impl LineEndingParam {
    /// Applies the setting to a text.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_files::common::params::LineEndingParam;
    /// use nurt_text::eol::Terminator;
    ///
    /// assert_eq!(LineEndingParam::Convert(Terminator::CrLf).apply("a\nb"), "a\r\nb");
    /// assert_eq!(LineEndingParam::Keep.apply("a\nb"), "a\nb");
    /// ```
    #[must_use]
    pub fn apply(self, text: &str) -> Cow<'_, str> {
        match self {
            Self::Keep => Cow::Borrowed(text),
            Self::Convert(terminator) => eol::convert(text, terminator),
        }
    }
}

/// Reads `line_ending`; [`LineEndingParam::Keep`] without a value.
///
/// # Errors
///
/// Fails with `E_TYPE` for a value that is not an enumeration value and with
/// `E_UNKNOWN_VALUE` for a value other than `lf`, `crlf`, `cr` and `keep`.
pub fn line_ending_param(args: &BoundArgs) -> Result<LineEndingParam> {
    match choice(args, LINE_ENDING)? {
        None | Some("keep") => Ok(LineEndingParam::Keep),
        Some("lf") => Ok(LineEndingParam::Convert(Terminator::Lf)),
        Some("crlf") => Ok(LineEndingParam::Convert(Terminator::CrLf)),
        Some("cr") => Ok(LineEndingParam::Convert(Terminator::Cr)),
        Some(other) => Err(unknown_value(LINE_ENDING, other, &LINE_ENDING_VALUES)),
    }
}

/// Value of the `on_missing` parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OnMissing {
    /// `skip`: report the missing target as a notable effect and continue.
    #[default]
    Skip,
    /// `error`: fail the command.
    Error,
}

/// Reads `on_missing`; [`OnMissing::Skip`] without a value.
///
/// # Errors
///
/// Fails with `E_TYPE` for a value that is not an enumeration value and with
/// `E_UNKNOWN_VALUE` for a value other than `skip` and `error`.
pub fn on_missing_param(args: &BoundArgs) -> Result<OnMissing> {
    match choice(args, ON_MISSING)? {
        None | Some("skip") => Ok(OnMissing::Skip),
        Some("error") => Ok(OnMissing::Error),
        Some(other) => Err(unknown_value(ON_MISSING, other, &ON_MISSING_VALUES)),
    }
}

/// Expected entity tags given with `if_match`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IfMatch {
    /// One tag, expected of every target.
    One(Etag),
    /// Tags of the listed paths; other targets are not checked.
    PerPath(Vec<(AbsPath, Etag)>),
}

impl IfMatch {
    /// Tag expected of `path`, when any.
    #[must_use]
    pub fn expected(&self, path: &AbsPath) -> Option<&Etag> {
        match self {
            Self::One(tag) => Some(tag),
            Self::PerPath(tags) => tags
                .iter()
                .find(|(listed, _)| listed == path)
                .map(|(_, tag)| tag),
        }
    }
}

/// Reads `if_match`: an etag, or a map from path to etag whose paths are
/// resolved as by [`resolve`]; `None` when the parameter has no value.
///
/// # Errors
///
/// Fails with `E_TYPE` for a value of another type, with `E_INVALID_VALUE`
/// for a text that is not an etag, and as [`resolve`] for the paths of a
/// map.
pub fn if_match_param(args: &BoundArgs, ctx: &ExecCtx) -> Result<Option<IfMatch>> {
    let expected = "an etag or a map from path to etag";
    match args.value(IF_MATCH) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Text(text)) => parse_etag(text).map(|tag| Some(IfMatch::One(tag))),
        Some(Value::Map(entries)) => entries
            .iter()
            .map(|(path, tag)| match tag {
                Value::Text(text) => Ok((resolve(ctx, path)?, parse_etag(text)?)),
                _ => Err(type_error(args.command(), IF_MATCH, expected)),
            })
            .collect::<Result<Vec<_>>>()
            .map(|tags| Some(IfMatch::PerPath(tags))),
        Some(_) => Err(type_error(args.command(), IF_MATCH, expected)),
    }
}

/// Filters of the walking commands: `hidden`, `gitignore`, `exclude` and
/// `exclude_preset`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalkFilters {
    /// Include hidden entries.
    pub hidden: bool,
    /// Apply the rules of ignore files.
    pub gitignore: bool,
    /// Exclusion patterns in gitignore syntax.
    pub exclude: Vec<String>,
    /// Named groups of excluded directories.
    pub exclude_presets: Vec<ExcludePreset>,
}

impl Default for WalkFilters {
    /// The catalog defaults: hidden entries included, no ignore files and no
    /// exclusions.
    fn default() -> Self {
        Self {
            hidden: true,
            gitignore: false,
            exclude: Vec::new(),
            exclude_presets: Vec::new(),
        }
    }
}

impl WalkFilters {
    /// Sets the filters in the settings of a walker.
    pub fn apply(&self, options: &mut WalkOptions) {
        options.hidden = self.hidden;
        options.gitignore = self.gitignore;
        options.exclude.clone_from(&self.exclude);
        options.exclude_presets.clone_from(&self.exclude_presets);
    }
}

/// Reads the filters of a walking command; a parameter the command does not
/// have keeps its default.
///
/// # Errors
///
/// Fails with `E_TYPE` for values of another type and with
/// `E_UNKNOWN_VALUE` for an unknown preset.
pub fn walk_filters(args: &BoundArgs) -> Result<WalkFilters> {
    let defaults = WalkFilters::default();
    let exclude_presets = text_list(args, EXCLUDE_PRESET)?
        .iter()
        .map(|name| preset(name))
        .collect::<Result<Vec<_>>>()?;
    Ok(WalkFilters {
        hidden: args.bool(HIDDEN)?.unwrap_or(defaults.hidden),
        gitignore: args.bool(GITIGNORE)?.unwrap_or(defaults.gitignore),
        exclude: text_list(args, EXCLUDE)?,
        exclude_presets,
    })
}

fn preset(name: &str) -> Result<ExcludePreset> {
    match name {
        "vcs" => Ok(ExcludePreset::Vcs),
        "deps" => Ok(ExcludePreset::Deps),
        "build" => Ok(ExcludePreset::Build),
        "cache" => Ok(ExcludePreset::Cache),
        other => Err(unknown_value(EXCLUDE_PRESET, other, &PRESET_VALUES)),
    }
}

fn text_list(args: &BoundArgs, name: &str) -> Result<Vec<String>> {
    let item = |value: &Value| match value {
        Value::Glob(text) | Value::Text(text) | Value::Enum(text) => Ok(text.clone()),
        _ => Err(type_error(args.command(), name, "a list of patterns")),
    };
    match args.value(name) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::List(items)) => items.iter().map(item).collect(),
        Some(value) => item(value).map(|text| vec![text]),
    }
}

fn choice<'a>(args: &'a BoundArgs, name: &str) -> Result<Option<&'a str>> {
    match args.value(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Enum(text) | Value::Text(text)) => Ok(Some(text.as_str())),
        Some(_) => Err(type_error(
            args.command(),
            name,
            "a value of the enumeration",
        )),
    }
}

fn type_error(command: &str, name: &str, expected: &str) -> Error {
    Error::new(
        ErrorCode::Type,
        format!("`{command}` expects {expected} for `{name}`"),
    )
}

fn unknown_value(name: &str, found: &str, expected: &[&str]) -> Error {
    Error::new(
        ErrorCode::UnknownValue,
        format!(
            "`{found}` is not a value of `{name}`: use one of {}",
            expected.join(", ")
        ),
    )
    .with_found(found)
    .with_expected(serde_json::json!(expected))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use nurt_contracts::envelope::RequestOptions;
    use nurt_contracts::fs::FileId;
    use nurt_contracts::stage::ArgValue;
    use nurt_contracts::value::Fields;
    use nurt_testkit::ctx::TestContext;

    use super::*;
    use crate::common::etag::etag_text;
    use crate::common::fixture::abs;

    fn args(name: &str, value: Value) -> BoundArgs {
        BoundArgs::new("write").with(name, ArgValue::Value(value), false)
    }

    fn text(value: &str) -> Value {
        Value::Text(value.to_owned())
    }

    fn choice_value(value: &str) -> Value {
        Value::Enum(value.to_owned())
    }

    fn tag(size: u64) -> Etag {
        Etag {
            size,
            modified_ns: 1,
            file_id: FileId {
                volume: 1,
                low: 2,
                high: 0,
            },
            hash: None,
        }
    }

    fn ctx_with_base() -> ExecCtx {
        let options = RequestOptions {
            base: Some(abs("/base").to_language_string()),
            ..RequestOptions::default()
        };
        TestContext::new()
            .builder()
            .options(Arc::new(options))
            .build()
    }

    #[test]
    fn encoding_accepts_auto_and_labels() {
        assert_eq!(
            encoding_param(&args(ENCODING, text("auto"))).unwrap(),
            Some(EncodingParam::Auto)
        );
        assert_eq!(
            encoding_param(&args(ENCODING, text(" AUTO "))).unwrap(),
            Some(EncodingParam::Auto)
        );
        assert_eq!(
            encoding_param(&args(ENCODING, text("cp1250"))).unwrap(),
            Some(EncodingParam::Codec(Codec::Windows1250))
        );
        assert_eq!(encoding_param(&BoundArgs::new("convert")).unwrap(), None);
    }

    #[test]
    fn encoding_rejects_unknown_labels_and_other_types() {
        let unknown = encoding_param(&args(ENCODING, text("koi8-r"))).unwrap_err();
        assert_eq!(unknown.code(), ErrorCode::UnknownValue);
        let wrong = encoding_param(&args(ENCODING, Value::Int(8))).unwrap_err();
        assert_eq!(wrong.code(), ErrorCode::Type);
    }

    #[test]
    fn line_ending_reads_every_value() {
        let read = |value: &str| line_ending_param(&args(LINE_ENDING, choice_value(value)));
        assert_eq!(read("keep").unwrap(), LineEndingParam::Keep);
        assert_eq!(
            read("lf").unwrap(),
            LineEndingParam::Convert(Terminator::Lf)
        );
        assert_eq!(
            read("crlf").unwrap(),
            LineEndingParam::Convert(Terminator::CrLf)
        );
        assert_eq!(
            read("cr").unwrap(),
            LineEndingParam::Convert(Terminator::Cr)
        );
        assert_eq!(
            line_ending_param(&BoundArgs::new("write")).unwrap(),
            LineEndingParam::Keep
        );
        let error = read("dos").unwrap_err();
        assert_eq!(error.code(), ErrorCode::UnknownValue);
        assert_eq!(error.found.as_deref(), Some("dos"));
        assert_eq!(
            line_ending_param(&args(LINE_ENDING, Value::Bool(true)))
                .unwrap_err()
                .code(),
            ErrorCode::Type
        );
    }

    #[test]
    fn line_ending_setting_converts_text() {
        assert_eq!(
            LineEndingParam::Convert(Terminator::Lf).apply("a\r\nb\r"),
            "a\nb\n"
        );
        assert!(matches!(
            LineEndingParam::Keep.apply("a\r\n"),
            Cow::Borrowed("a\r\n")
        ));
    }

    #[test]
    fn on_missing_defaults_to_skip() {
        assert_eq!(
            on_missing_param(&BoundArgs::new("remove")).unwrap(),
            OnMissing::Skip
        );
        assert_eq!(
            on_missing_param(&args(ON_MISSING, choice_value("skip"))).unwrap(),
            OnMissing::Skip
        );
        assert_eq!(
            on_missing_param(&args(ON_MISSING, text("error"))).unwrap(),
            OnMissing::Error
        );
        assert_eq!(
            on_missing_param(&args(ON_MISSING, choice_value("ignore")))
                .unwrap_err()
                .code(),
            ErrorCode::UnknownValue
        );
    }

    #[test]
    fn if_match_reads_one_tag() {
        let ctx = ctx_with_base();
        let given = args(IF_MATCH, Value::Text(etag_text(&tag(4))));
        let parsed = if_match_param(&given, &ctx).unwrap().unwrap();
        assert_eq!(parsed, IfMatch::One(tag(4)));
        assert_eq!(parsed.expected(&abs("/any")), Some(&tag(4)));
        assert_eq!(if_match_param(&BoundArgs::new("edit"), &ctx).unwrap(), None);
    }

    #[test]
    fn if_match_reads_tags_per_path() {
        let ctx = ctx_with_base();
        let mut entries = Fields::new();
        entries.insert("a.txt".to_owned(), Value::Text(etag_text(&tag(1))));
        entries.insert(
            abs("/data/b.txt").to_language_string(),
            Value::Text(etag_text(&tag(2))),
        );
        let parsed = if_match_param(&args(IF_MATCH, Value::Map(entries)), &ctx)
            .unwrap()
            .unwrap();
        assert_eq!(parsed.expected(&abs("/base/a.txt")), Some(&tag(1)));
        assert_eq!(parsed.expected(&abs("/data/b.txt")), Some(&tag(2)));
        assert_eq!(parsed.expected(&abs("/data/c.txt")), None);
    }

    #[test]
    fn if_match_rejects_malformed_values() {
        let ctx = ctx_with_base();
        let malformed = if_match_param(&args(IF_MATCH, text("not-an-etag")), &ctx).unwrap_err();
        assert_eq!(malformed.code(), ErrorCode::InvalidValue);
        let mut entries = Fields::new();
        entries.insert("a.txt".to_owned(), Value::Int(1));
        assert_eq!(
            if_match_param(&args(IF_MATCH, Value::Map(entries)), &ctx)
                .unwrap_err()
                .code(),
            ErrorCode::Type
        );
        assert_eq!(
            if_match_param(&args(IF_MATCH, Value::Int(1)), &ctx)
                .unwrap_err()
                .code(),
            ErrorCode::Type
        );
    }

    #[test]
    fn walk_filters_have_catalog_defaults() {
        let filters = walk_filters(&BoundArgs::new("list")).unwrap();
        assert_eq!(filters, WalkFilters::default());
        assert!(filters.hidden);
        assert!(!filters.gitignore);
    }

    #[test]
    fn walk_filters_read_every_parameter() {
        let given = BoundArgs::new("find")
            .with(HIDDEN, ArgValue::Value(Value::Bool(false)), false)
            .with(GITIGNORE, ArgValue::Value(Value::Bool(true)), false)
            .with(
                EXCLUDE,
                ArgValue::Value(Value::List(vec![
                    Value::Glob("*.log".to_owned()),
                    text("!keep.log"),
                ])),
                false,
            )
            .with(
                EXCLUDE_PRESET,
                ArgValue::Value(Value::List(vec![
                    choice_value("vcs"),
                    choice_value("deps"),
                    choice_value("build"),
                    choice_value("cache"),
                ])),
                false,
            );
        let filters = walk_filters(&given).unwrap();
        assert!(!filters.hidden);
        assert!(filters.gitignore);
        assert_eq!(filters.exclude, ["*.log", "!keep.log"]);
        assert_eq!(
            filters.exclude_presets,
            [
                ExcludePreset::Vcs,
                ExcludePreset::Deps,
                ExcludePreset::Build,
                ExcludePreset::Cache
            ]
        );
        let mut options = WalkOptions::default();
        filters.apply(&mut options);
        assert!(!options.hidden);
        assert!(options.gitignore);
        assert_eq!(options.exclude, filters.exclude);
        assert_eq!(options.exclude_presets, filters.exclude_presets);
    }

    #[test]
    fn walk_filters_accept_a_single_pattern() {
        let filters = walk_filters(&args(EXCLUDE, Value::Glob("target".to_owned()))).unwrap();
        assert_eq!(filters.exclude, ["target"]);
    }

    #[test]
    fn walk_filters_reject_unknown_presets_and_types() {
        let unknown = walk_filters(&args(
            EXCLUDE_PRESET,
            Value::List(vec![choice_value("docs")]),
        ))
        .unwrap_err();
        assert_eq!(unknown.code(), ErrorCode::UnknownValue);
        let wrong = walk_filters(&args(EXCLUDE, Value::List(vec![Value::Int(1)]))).unwrap_err();
        assert_eq!(wrong.code(), ErrorCode::Type);
        assert_eq!(
            walk_filters(&args(HIDDEN, text("yes"))).unwrap_err().code(),
            ErrorCode::Type
        );
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
