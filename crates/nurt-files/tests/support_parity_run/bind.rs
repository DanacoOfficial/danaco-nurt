// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Binding of parsed stages to the signatures of the catalog.
//!
//! The binder stands in for the binder of the engine: it assigns positional
//! values to positional parameters, converts every value to the type the
//! catalog declares and separates the request options. Values that the
//! engine would reject become errors of the report; parameters that the
//! catalog does not know make the scenario pending.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::stage::{ArgValue, BoundArgs};
use nurt_contracts::value::{self, Fields, Range, Value};
use serde::Deserialize;
use serde_json::{Map, Value as Json};

use super::program::{Call, Lit};

/// Request options passed to the context of the stage.
pub(crate) const REQUEST_OPTIONS: [&str; 7] = [
    "preview",
    "format",
    "verbosity",
    "diff",
    "diff_context",
    "timeout",
    "base",
];

/// Request options applied by the executor after the stages ran.
pub(crate) const EXECUTOR_OPTIONS: [&str; 5] =
    ["limit", "max_chars", "fields", "cursor", "save_to"];

/// Placeholder value of an entity tag that matches no file.
pub(crate) const UNKNOWN_ETAG: &str = "0-0-0-0-0";

/// Parameter of a command.
#[derive(Clone, Debug, Deserialize)]
pub(crate) struct ParamSpec {
    /// Name.
    pub(crate) name: String,
    /// Type in the notation of the catalog.
    #[serde(rename = "type")]
    pub(crate) ty: String,
    /// Position of a positional parameter, from 1.
    pub(crate) position: Option<u32>,
    /// The parameter needs a value in the source form.
    #[serde(default)]
    pub(crate) required: bool,
    /// The parameter takes the remaining positional values.
    #[serde(default)]
    pub(crate) repeated: bool,
}

/// Command of the catalog.
#[derive(Clone, Debug, Deserialize)]
pub(crate) struct CommandSpec {
    /// Full name.
    pub(crate) name: String,
    /// Effect class: `read` for commands that change nothing.
    pub(crate) effect: String,
    /// Parameters in signature order.
    #[serde(default)]
    pub(crate) param: Vec<ParamSpec>,
}

#[derive(Deserialize)]
struct CatalogFile {
    #[serde(default)]
    command: Vec<CommandSpec>,
}

#[derive(Deserialize)]
struct IndexEntry {
    file: String,
}

#[derive(Deserialize)]
struct IndexFile {
    file: Vec<IndexEntry>,
}

/// Commands of every group of the catalog.
#[derive(Debug)]
pub(crate) struct Catalog {
    commands: BTreeMap<String, CommandSpec>,
}

impl Catalog {
    /// Loads the groups listed in `_index.toml` below `dir`.
    ///
    /// # Errors
    ///
    /// Returns a message naming the file that cannot be read or parsed.
    pub(crate) fn load(dir: &Path) -> Result<Self, String> {
        let read = |name: &str| {
            let path = dir.join(name);
            fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))
        };
        let index: IndexFile =
            toml::from_str(&read("_index.toml")?).map_err(|e| format!("_index.toml: {e}"))?;
        let mut commands = BTreeMap::new();
        for entry in &index.file {
            let name = entry.file.clone();
            let file: CatalogFile =
                toml::from_str(&read(&name)?).map_err(|e| format!("{name}: {e}"))?;
            for command in file.command {
                commands.insert(command.name.clone(), command);
            }
        }
        Ok(Self { commands })
    }

    /// Command named `name`.
    pub(crate) fn command(&self, name: &str) -> Option<&CommandSpec> {
        self.commands.get(name)
    }
}

/// Arguments of one stage and the request options it names.
#[derive(Debug)]
pub(crate) struct Bound {
    /// Arguments for the stage.
    pub(crate) args: BoundArgs,
    /// Request options as JSON fields of the request envelope.
    pub(crate) options: Map<String, Json>,
}

/// Reason the binder did not produce arguments.
#[derive(Debug)]
pub(crate) enum BindFailure {
    /// The engine would reject the program with this error.
    Error(Error),
    /// The scenario cannot run on the stages of this crate.
    Pending(String),
}

/// Facts the binder needs from the tree.
pub(crate) struct BindCx<'a> {
    /// Language form of the root directory of the scenario.
    pub(crate) root: &'a str,
    /// Entity tag text of a path, for the placeholders of the scenarios.
    pub(crate) etag_of: &'a dyn Fn(&str) -> String,
}

enum Conv {
    Mismatch,
    Invalid(Error),
    Unsupported(String),
}

fn invalid(code: ErrorCode, message: String) -> Conv {
    Conv::Invalid(Error::new(code, message))
}

/// Replaces `{root}` by the root directory.
pub(crate) fn with_root(text: &str, root: &str) -> String {
    text.replace("{root}", root)
}

/// Binds `call` to the signature of `spec`; `source` marks the first stage
/// of a pipeline, where required parameters must have a value.
///
/// # Errors
///
/// Returns [`BindFailure::Error`] for values the engine rejects and
/// [`BindFailure::Pending`] for parameters outside the catalog and types the
/// binder does not convert.
pub(crate) fn bind(
    call: &Call,
    spec: &CommandSpec,
    source: bool,
    cx: &BindCx<'_>,
) -> Result<Bound, BindFailure> {
    let mut args = BoundArgs::new(spec.name.as_str());
    let mut options = Map::new();
    let context_path = call.positional.iter().find_map(|lit| match lit {
        Lit::Str(text) if text.contains("{root}") => Some(with_root(text, cx.root)),
        _ => None,
    });
    let context = Context {
        cx,
        path: context_path.as_deref(),
    };
    bind_positional(call, spec, &context, &mut args)?;
    for (name, lit) in &call.named {
        if let Some(param) = spec.param.iter().find(|p| &p.name == name) {
            let value = convert(lit, &param.ty, &context, name)?;
            args.insert(name.as_str(), ArgValue::Value(value), false);
        } else if REQUEST_OPTIONS.contains(&name.as_str()) {
            options.insert(name.clone(), option_json(lit, cx.root));
        } else if EXECUTOR_OPTIONS.contains(&name.as_str()) {
            return Err(BindFailure::Pending(format!(
                "option `{name}` is applied by the executor"
            )));
        } else {
            return Err(BindFailure::Pending(format!(
                "parameter `{name}` is not in the catalog of `{}`",
                spec.name
            )));
        }
    }
    if source {
        for param in spec.param.iter().filter(|p| p.required) {
            if !args.is_given(&param.name) {
                return Err(BindFailure::Error(Error::new(
                    ErrorCode::MissingParam,
                    format!("`{}` needs a value for `{}`", spec.name, param.name),
                )));
            }
        }
    }
    Ok(Bound { args, options })
}

struct Context<'a> {
    cx: &'a BindCx<'a>,
    path: Option<&'a str>,
}

fn bind_positional(
    call: &Call,
    spec: &CommandSpec,
    context: &Context<'_>,
    args: &mut BoundArgs,
) -> Result<(), BindFailure> {
    let mut positional: Vec<&ParamSpec> =
        spec.param.iter().filter(|p| p.position.is_some()).collect();
    positional.sort_by_key(|p| p.position);
    let mut values = call.positional.iter().peekable();
    for (index, param) in positional.iter().enumerate() {
        let remaining = values.len();
        if remaining == 0 {
            break;
        }
        let later = positional.len() - index - 1;
        let take = if param.repeated {
            remaining.saturating_sub(later).max(1)
        } else {
            1
        };
        let taken: Vec<Lit> = values.by_ref().take(take).cloned().collect();
        let lit = if taken.len() == 1 {
            taken.into_iter().next().unwrap_or(Lit::List(Vec::new()))
        } else {
            Lit::List(taken)
        };
        let value = convert(&lit, &param.ty, context, &param.name)?;
        args.insert(param.name.as_str(), ArgValue::Value(value), false);
    }
    if values.peek().is_some() {
        return Err(BindFailure::Error(Error::new(
            ErrorCode::Arity,
            format!(
                "`{}` takes {} positional values",
                spec.name,
                positional.len()
            ),
        )));
    }
    Ok(())
}

fn convert(lit: &Lit, ty: &str, context: &Context<'_>, name: &str) -> Result<Value, BindFailure> {
    match convert_type(lit, ty, context) {
        Ok(value) => Ok(value),
        Err(Conv::Invalid(error)) => Err(BindFailure::Error(error)),
        Err(Conv::Unsupported(what)) => Err(BindFailure::Pending(format!(
            "parameter `{name}` needs {what}, which the scenario runner does not bind"
        ))),
        Err(Conv::Mismatch) => Err(BindFailure::Error(Error::new(
            ErrorCode::Type,
            format!("`{name}` expects {ty}"),
        ))),
    }
}

/// Splits `text` at `separator` outside angle brackets and parentheses.
pub(crate) fn split_top(text: &str, separator: char) -> Vec<String> {
    let mut parts = Vec::new();
    let mut depth = 0_i32;
    let mut current = String::new();
    for c in text.chars() {
        match c {
            '<' | '(' => depth += 1,
            '>' | ')' => depth -= 1,
            _ => {}
        }
        if c == separator && depth == 0 {
            parts.push(current.trim().to_owned());
            current.clear();
        } else {
            current.push(c);
        }
    }
    parts.push(current.trim().to_owned());
    parts
}

fn convert_type(lit: &Lit, ty: &str, context: &Context<'_>) -> Result<Value, Conv> {
    let mut failure = Conv::Mismatch;
    for alternative in split_top(ty, '|') {
        match convert_one(lit, &alternative, context) {
            Ok(value) => return Ok(value),
            Err(Conv::Mismatch) => {}
            Err(Conv::Invalid(error)) => failure = Conv::Invalid(error),
            Err(Conv::Unsupported(what)) => {
                if matches!(failure, Conv::Mismatch) {
                    failure = Conv::Unsupported(what);
                }
            }
        }
    }
    Err(failure)
}

fn inner<'t>(ty: &'t str, prefix: &str) -> Option<&'t str> {
    ty.strip_prefix(prefix)?.strip_suffix('>')
}

fn text_of(lit: &Lit, context: &Context<'_>, key: Option<&str>) -> Option<String> {
    match lit {
        Lit::Str(text) => Some(resolve_text(text, context, key)),
        Lit::Ident(word) => Some(word.clone()),
        _ => None,
    }
}

/// Resolves `{root}` and the entity tag placeholders `<...>` of the scenarios.
fn resolve_text(text: &str, context: &Context<'_>, key: Option<&str>) -> String {
    let placeholder = text.starts_with('<') && text.ends_with('>');
    if !placeholder {
        return with_root(text, context.cx.root);
    }
    if text.contains("unknown") {
        return UNKNOWN_ETAG.to_owned();
    }
    key.or(context.path).map_or_else(
        || UNKNOWN_ETAG.to_owned(),
        |path| (context.cx.etag_of)(path),
    )
}

fn convert_one(lit: &Lit, ty: &str, context: &Context<'_>) -> Result<Value, Conv> {
    if let Some(element) = inner(ty, "list<") {
        return match lit {
            Lit::List(items) => items
                .iter()
                .map(|item| convert_type(item, element, context))
                .collect::<Result<Vec<_>, _>>()
                .map(Value::List),
            single => convert_type(single, element, context).map(|value| Value::List(vec![value])),
        };
    }
    if let Some(element) = inner(ty, "map<") {
        let Lit::Map(entries) = lit else {
            return Err(Conv::Mismatch);
        };
        let mut fields = Fields::new();
        for (key, item) in entries {
            let key = with_root(key, context.cx.root);
            let value = match item {
                Lit::Str(text) => {
                    let text = resolve_text(text, context, Some(&key));
                    convert_type(
                        &Lit::Str(text),
                        element,
                        &Context {
                            path: None,
                            ..*context
                        },
                    )?
                }
                other => convert_type(other, element, context)?,
            };
            fields.insert(key, value);
        }
        return Ok(Value::Map(fields));
    }
    if let Some(choices) = ty.strip_prefix("enum(").and_then(|t| t.strip_suffix(')')) {
        let Some(word) = text_of(lit, context, None) else {
            return Err(Conv::Mismatch);
        };
        return if choices.split('|').any(|choice| choice == word) {
            Ok(Value::Enum(word))
        } else {
            Err(invalid(
                ErrorCode::UnknownValue,
                format!("`{word}` is not one of {choices}"),
            ))
        };
    }
    match (ty, lit) {
        ("path", Lit::Str(text)) => Ok(Value::Path(with_root(text, context.cx.root))),
        ("text", Lit::Str(_) | Lit::Ident(_)) => text_of(lit, context, None)
            .map(Value::Text)
            .ok_or(Conv::Mismatch),
        ("glob", Lit::Str(text)) => Ok(Value::Glob(with_root(text, context.cx.root))),
        ("key", Lit::Str(text) | Lit::Ident(text)) => Ok(Value::Key(text.clone())),
        ("regex", Lit::Regex(pattern, flags)) => regex(pattern, flags),
        ("regex", Lit::Str(pattern)) => regex(pattern, ""),
        ("int", Lit::Int(number)) => Ok(Value::Int(*number)),
        ("bool", Lit::Bool(flag)) => Ok(Value::Bool(*flag)),
        ("range", Lit::Range(start, end)) => range(*start, *end),
        ("range", Lit::Int(line)) => range(Some(*line), Some(*line)),
        ("any", _) => any(lit, context),
        ("path" | "text" | "glob" | "key" | "regex" | "int" | "bool" | "range", _) => {
            Err(Conv::Mismatch)
        }
        (other, _) => Err(Conv::Unsupported(format!("type `{other}`"))),
    }
}

fn regex(pattern: &str, flags: &str) -> Result<Value, Conv> {
    value::Regex::new(pattern, flags)
        .map(Value::Regex)
        .map_err(|e| {
            invalid(
                ErrorCode::Regex,
                format!("invalid regular expression: {e:?}"),
            )
        })
}

fn range(start: Option<i64>, end: Option<i64>) -> Result<Value, Conv> {
    Range::new(start, end)
        .map(Value::Range)
        .map_err(|e| invalid(ErrorCode::InvalidValue, format!("invalid range: {e:?}")))
}

fn any(lit: &Lit, context: &Context<'_>) -> Result<Value, Conv> {
    match lit {
        Lit::Str(_) | Lit::Ident(_) | Lit::Raw(_) => Ok(Value::Text(
            text_of(lit, context, None).unwrap_or_else(|| match lit {
                Lit::Raw(raw) => raw.clone(),
                _ => String::new(),
            }),
        )),
        Lit::Int(number) => Ok(Value::Int(*number)),
        Lit::Float(number) => Ok(Value::Number(*number)),
        Lit::Bool(flag) => Ok(Value::Bool(*flag)),
        Lit::Range(start, end) => range(*start, *end),
        Lit::Regex(pattern, flags) => regex(pattern, flags),
        Lit::List(items) => items
            .iter()
            .map(|item| any(item, context))
            .collect::<Result<Vec<_>, _>>()
            .map(Value::List),
        Lit::Map(_) => convert_one(lit, "map<any>", context),
    }
}

fn option_json(lit: &Lit, root: &str) -> Json {
    match lit {
        Lit::Str(text) => Json::String(with_root(text, root)),
        Lit::Ident(word) | Lit::Raw(word) => Json::String(word.clone()),
        Lit::Int(number) => Json::from(*number),
        Lit::Float(number) => Json::from(*number),
        Lit::Bool(flag) => Json::Bool(*flag),
        Lit::List(items) => Json::Array(items.iter().map(|item| option_json(item, root)).collect()),
        Lit::Map(entries) => Json::Object(
            entries
                .iter()
                .map(|(key, item)| (key.clone(), option_json(item, root)))
                .collect(),
        ),
        Lit::Range(start, end) => Json::String(format!(
            "{}..{}",
            start.map(|n| n.to_string()).unwrap_or_default(),
            end.map(|n| n.to_string()).unwrap_or_default()
        )),
        Lit::Regex(pattern, _) => Json::String(pattern.clone()),
    }
}
