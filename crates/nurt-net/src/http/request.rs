// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The `http.request` command.
//!
//! The command sends one request per address and returns a `response`
//! record for each, or `row` records with `extract: tables`. HTML bodies are
//! converted to Markdown or plain text with an outline of long pages, and
//! parts of a page are selected with `section`, `find` and `query`.
//! Statuses of 400 and above are data in `response.status`. Methods other
//! than `get`, `head` and `options` change the remote side and are marked
//! irreversible; a body written with `save` goes through the journal.
//!
//! The console applies no network rules: every address, including private
//! and loopback addresses, is contacted. The static check reports the
//! normalized targets in the `network` list of its details, and an embedding
//! application may register a [`ConnectGate`](crate::client::ConnectGate)
//! that is consulted before every connection.

use std::io::Read as _;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use indexmap::IndexMap;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::AbsPath;
use nurt_contracts::journal::{Expect, Mutation, MutationTx, TargetAction, TargetPath, TargetSpec};
use nurt_contracts::record::{HttpResponse, Row};
use nurt_contracts::report::{Change, ChangeAction, Irreversible, IrreversibleEntry};
use nurt_contracts::stage::{
    BoundArgs, CheckCtx, EffectOutcome, EffectPlan, EffectStage, Event, ExecCtx, InputKinds,
    LogEvent, LogLevel, ReadStage, RecordStream, StageDescriptor, StageImpl, StageInput,
    StageOutput, StaticInfo,
};
use nurt_contracts::value::{self, Record, Size, Value};
use regex::{Regex, RegexBuilder};
use sha2::{Digest, Sha256};

use crate::cache::{CacheStore, CachedResponse};
use crate::check::{AddressFact, RequestFacts, describe};
use crate::client::{
    BodyEncoding, ClientSettings, HttpClient, Method, NameResolver, Pins, ProxyChoice,
    RequestBody, RequestSpec, ResponseBody, Target,
};
use crate::html::{
    self, ConvertOptions, ImageStyle, LinkStyle, Page, TextForm, convert_html, convert_markdown,
    convert_text, select,
};

/// Name of the command in the catalog.
pub const COMMAND: &str = "http.request";
/// Bytes of a body kept in memory when the request gives no limit.
pub const DEFAULT_MAX_SIZE: u64 = 2 * 1024 * 1024;
/// Requests sent at the same time for a list of addresses.
pub const PARALLEL: usize = 4;
/// Paragraphs returned by `query` when `query_limit` is not given.
pub const DEFAULT_QUERY_LIMIT: i64 = 5;
/// Largest `query_limit`.
pub const MAX_QUERY_LIMIT: i64 = 20;
/// Largest `find_context`.
pub const MAX_FIND_CONTEXT: i64 = 10;
/// Hexadecimal digits of the entity tag of a rendered body.
pub const ETAG_DIGITS: usize = 12;
/// Subdirectory of the state directory holding the full content of pages
/// returned as an outline.
pub const RESULTS_DIR: &str = "http-results";

/// Implementation of `http.request`.
#[derive(Debug, Default, Clone, Copy)]
pub struct HttpRequest;

impl StageImpl for HttpRequest {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: COMMAND.to_owned(),
            source: true,
            input: InputKinds::None,
        }
    }

    fn check(&self, args: &BoundArgs, ctx: &CheckCtx<'_>) -> Result<StaticInfo> {
        let params = Params::from_args(args, Addresses::Static)?;
        let mut info = StaticInfo::default();
        let mut entries = Vec::new();
        for address in &params.addresses {
            let facts = params.facts(address.clone());
            let described = describe(&facts, ctx.host(), ctx.config().http.proxy.as_deref(), None)?;
            info.network.extend(described.target);
            entries.push(described.entry);
        }
        if let Some(save) = &params.save {
            info.paths.push(save.to_language_string());
        }
        info.details
            .insert("network".to_owned(), Value::List(entries));
        Ok(info)
    }

    fn as_read(&self) -> Option<&dyn ReadStage> {
        Some(self)
    }

    fn as_effect(&self) -> Option<&dyn EffectStage> {
        Some(self)
    }
}

impl ReadStage for HttpRequest {
    fn execute<'a>(
        &'a self,
        args: &'a BoundArgs,
        _input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<StageOutput<'a>> {
        let params = Params::from_args(args, Addresses::Evaluated(ctx))?;
        if params.save.is_some() || !params.method.is_read_only() {
            return Err(Error::new(
                ErrorCode::Arg,
                "`http.request` with `save` or a changing method runs as the effect of the program",
            ));
        }
        let runner = Runner::new(&params, ctx);
        let records = runner.run_all(ctx)?;
        Ok(StageOutput::records(RecordStream::from_records(records)))
    }
}

impl EffectStage for HttpRequest {
    fn plan<'a>(
        &'a self,
        args: &'a BoundArgs,
        _input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<EffectPlan> {
        let params = Params::from_args(args, Addresses::Evaluated(ctx))?;
        let mut plan = EffectPlan::new();
        plan.irreversible.extend(params.irreversible());
        if let Some(save) = &params.save {
            let exists = ctx
                .fs()
                .and_then(|fs| fs.metadata(save, nurt_contracts::fs::Follow::Follow).ok())
                .is_some();
            plan.changes.push(save_change(save, exists, None, None));
        }
        Ok(plan)
    }

    fn apply<'a>(
        &'a self,
        args: &'a BoundArgs,
        _plan: EffectPlan,
        tx: &mut dyn MutationTx,
        ctx: &'a ExecCtx,
    ) -> Result<EffectOutcome<'a>> {
        let params = Params::from_args(args, Addresses::Evaluated(ctx))?;
        let irreversible = params.irreversible();
        for entry in &irreversible {
            tx.mark_irreversible(entry.clone());
        }
        let runner = Runner::new(&params, ctx);
        let (records, changes) = match &params.save {
            Some(save) => {
                let (records, change) = runner.run_saving(ctx, save, tx)?;
                (records, vec![change])
            }
            None => (runner.run_all(ctx)?, Vec::new()),
        };
        let mut outcome = EffectOutcome::new(StageOutput::records(RecordStream::from_records(
            records,
        )));
        outcome.changes = changes;
        outcome.irreversible = irreversible;
        Ok(outcome)
    }
}

fn save_change(path: &AbsPath, existed: bool, before: Option<u64>, after: Option<u64>) -> Change {
    Change {
        path: path.to_language_string(),
        action: if existed {
            ChangeAction::Modify
        } else {
            ChangeAction::Create
        },
        from: None,
        size_before: before,
        size_after: after,
        hash_before: None,
        hash_after: None,
        replacements: None,
        added: None,
        removed: None,
        before: None,
        after: None,
        flags: Vec::new(),
    }
}

/// Form of the returned body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Form {
    Raw,
    Text,
    Markdown,
}

/// Part of the page returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Extract {
    Content,
    Headers,
    Links,
    Headings,
    Meta,
    Tables,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CacheMode {
    Bypass,
    Use,
    Revalidate,
    Only,
}

impl CacheMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Bypass => "bypass",
            Self::Use => "use",
            Self::Revalidate => "revalidate",
            Self::Only => "only",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OutlineMode {
    Auto,
    Always,
    Never,
}

/// How addresses are read from the arguments.
enum Addresses<'a> {
    /// Before execution: expressions are reported as dynamic.
    Static,
    /// During execution: expressions are evaluated.
    Evaluated(&'a ExecCtx),
}

/// Arguments of one call, validated.
struct Params {
    addresses: Vec<AddressFact>,
    method: Method,
    headers: Vec<(String, String)>,
    body: RequestBody,
    body_kind: &'static str,
    save: Option<AbsPath>,
    form: Form,
    extract: Extract,
    table: usize,
    convert: ConvertOptions,
    section: Option<String>,
    outline: OutlineMode,
    query: Option<String>,
    query_limit: usize,
    find: Option<Regex>,
    find_context: usize,
    links_filter: Option<Regex>,
    same_host: bool,
    charset: Option<String>,
    offset: usize,
    cache: CacheMode,
    unchanged_since: Option<String>,
    timeout: Duration,
    follow_redirects: bool,
    max_redirects: u32,
    verify_tls: bool,
    retries: u32,
    fail_on_status: bool,
    on_error_record: bool,
    max_size: u64,
    pins: Pins,
    proxy: ProxyChoice,
}

fn conflict(message: impl Into<String>) -> Error {
    Error::new(ErrorCode::Arg, message).with_problem("param_conflict")
}

fn out_of_range(name: &str, value: i64, low: i64, high: i64) -> Error {
    Error::new(
        ErrorCode::Arg,
        format!("`{name}` must be between {low} and {high}"),
    )
    .with_expected(serde_json::json!({ "min": low, "max": high }))
    .with_actual(serde_json::json!(value))
}

fn word<'a>(args: &'a BoundArgs, name: &str) -> Result<Option<&'a str>> {
    match args.value(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Text(text) | Value::Enum(text)) => Ok(Some(text.as_str())),
        Some(other) => Err(Error::new(
            ErrorCode::Type,
            format!("`{name}` expects text, not {}", other.type_name()),
        )),
    }
}

fn choice<T: Copy>(args: &BoundArgs, name: &str, default: T, values: &[(&str, T)]) -> Result<T> {
    let Some(given) = word(args, name)? else {
        return Ok(default);
    };
    values
        .iter()
        .find(|(text, _)| *text == given)
        .map(|(_, value)| *value)
        .ok_or_else(|| {
            let names: Vec<&str> = values.iter().map(|(text, _)| *text).collect();
            Error::new(
                ErrorCode::UnknownValue,
                format!("`{given}` is not a value of `{name}`; use one of {}", names.join(", ")),
            )
            .with_expected(serde_json::json!(names))
        })
}

fn int(args: &BoundArgs, name: &str, default: i64) -> Result<i64> {
    Ok(args.int(name)?.unwrap_or(default))
}

fn flag(args: &BoundArgs, name: &str, default: bool) -> Result<bool> {
    Ok(args.bool(name)?.unwrap_or(default))
}

fn count(args: &BoundArgs, name: &str, default: i64, high: i64) -> Result<usize> {
    let value = int(args, name, default)?;
    if !(0..=high).contains(&value) {
        return Err(out_of_range(name, value, 0, high));
    }
    Ok(usize::try_from(value).unwrap_or_default())
}

fn regex_arg(args: &BoundArgs, name: &str) -> Result<Option<Regex>> {
    let (pattern, flags) = match args.value(name) {
        None | Some(Value::Null) => return Ok(None),
        Some(Value::Regex(regex)) => (regex.pattern.clone(), regex.flags.clone()),
        Some(Value::Text(text)) => (text.clone(), value::RegexFlags::default()),
        Some(other) => {
            return Err(Error::new(
                ErrorCode::Type,
                format!("`{name}` expects a regex, not {}", other.type_name()),
            ));
        }
    };
    RegexBuilder::new(&pattern)
        .case_insensitive(flags.case_insensitive)
        .multi_line(flags.multi_line)
        .dot_matches_new_line(flags.dot_matches_new_line)
        .ignore_whitespace(flags.extended)
        .build()
        .map(Some)
        .map_err(|error| {
            Error::new(
                ErrorCode::Regex,
                format!("the pattern of `{name}` is not valid: {error}"),
            )
            .with_found(pattern)
        })
}

fn duration(args: &BoundArgs, name: &str, default: Duration) -> Result<Duration> {
    match args.value(name) {
        None | Some(Value::Null) => Ok(default),
        Some(Value::Duration(value)) => value.to_std().ok_or_else(|| {
            Error::new(ErrorCode::Arg, format!("`{name}` must not be negative"))
        }),
        Some(other) => Err(Error::new(
            ErrorCode::Type,
            format!("`{name}` expects a duration, not {}", other.type_name()),
        )),
    }
}

fn size(args: &BoundArgs, name: &str, default: u64) -> Result<u64> {
    match args.value(name) {
        None | Some(Value::Null) => Ok(default),
        Some(Value::Size(value)) => Ok(value.bytes()),
        Some(Value::Int(value)) => u64::try_from(*value)
            .map_err(|_| Error::new(ErrorCode::Arg, format!("`{name}` must not be negative"))),
        Some(other) => Err(Error::new(
            ErrorCode::Type,
            format!("`{name}` expects a size, not {}", other.type_name()),
        )),
    }
}

fn addresses(args: &BoundArgs, mode: &Addresses<'_>) -> Result<Vec<AddressFact>> {
    let value = match (args.expr("url"), mode) {
        (Some(expr), Addresses::Static) => {
            return Ok(vec![AddressFact::Dynamic(format!("{expr:?}"))]);
        }
        (Some(_), Addresses::Evaluated(ctx)) => args.eval("url", None, ctx)?,
        (None, _) => args.value("url").cloned(),
    };
    let texts = match value {
        Some(Value::Text(text)) => vec![text],
        Some(Value::List(items)) => items
            .into_iter()
            .map(|item| match item {
                Value::Text(text) => Ok(text),
                other => Err(Error::new(
                    ErrorCode::Type,
                    format!("`url` expects text addresses, not {}", other.type_name()),
                )),
            })
            .collect::<Result<_>>()?,
        Some(other) => {
            return Err(Error::new(
                ErrorCode::Type,
                format!("`url` expects text or a list of text, not {}", other.type_name()),
            ));
        }
        None => args.require("url").map(|_| Vec::new())?,
    };
    if texts.is_empty() {
        return Err(Error::new(ErrorCode::Arg, "`url` needs at least one address"));
    }
    Ok(texts.into_iter().map(AddressFact::Fixed).collect())
}

fn headers(args: &BoundArgs) -> Result<Vec<(String, String)>> {
    match args.value("headers") {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Map(map)) => map
            .iter()
            .map(|(name, value)| match value {
                Value::Text(text) => Ok((name.clone(), text.clone())),
                Value::Int(number) => Ok((name.clone(), number.to_string())),
                other => Err(Error::new(
                    ErrorCode::Type,
                    format!("header `{name}` expects text, not {}", other.type_name()),
                )),
            })
            .collect(),
        Some(other) => Err(Error::new(
            ErrorCode::Type,
            format!("`headers` expects a map, not {}", other.type_name()),
        )),
    }
}

fn request_body(args: &BoundArgs, mode: &Addresses<'_>) -> Result<(RequestBody, &'static str)> {
    let body = args.value("body").filter(|value| !value.is_null());
    let json = args.value("json").filter(|value| !value.is_null());
    match (body, json) {
        (Some(_), Some(_)) => Err(conflict("give either `body` or `json`, not both")),
        (None, None) => Ok((RequestBody::None, "none")),
        (None, Some(json)) => {
            let data = serde_json::to_vec(&json.to_plain_json()).map_err(|error| {
                Error::new(ErrorCode::Arg, format!("`json` cannot be encoded: {error}"))
            })?;
            Ok((
                RequestBody::Bytes {
                    data,
                    content_type: Some("application/json".to_owned()),
                },
                "json",
            ))
        }
        (Some(Value::Path(path)), None) => Ok((read_body_file(path, mode)?, "file")),
        (Some(Value::Text(text)), None) => Ok((
            RequestBody::Bytes {
                data: text.clone().into_bytes(),
                content_type: Some("text/plain; charset=utf-8".to_owned()),
            },
            "text",
        )),
        (Some(other), None) => Err(Error::new(
            ErrorCode::Type,
            format!("`body` expects text or a path, not {}", other.type_name()),
        )),
    }
}

fn read_body_file(path: &str, mode: &Addresses<'_>) -> Result<RequestBody> {
    let Addresses::Evaluated(ctx) = mode else {
        return Ok(RequestBody::None);
    };
    let path = AbsPath::new(path)?;
    let mut handle = ctx.require_fs()?.open_read(&path)?;
    let mut data = Vec::new();
    handle.read_to_end(&mut data).map_err(|error| {
        Error::new(
            ErrorCode::Io,
            format!("reading `{}` failed: {error}", path.to_language_string()),
        )
    })?;
    Ok(RequestBody::Bytes {
        data,
        content_type: Some("application/octet-stream".to_owned()),
    })
}

fn pins(args: &BoundArgs) -> Result<Pins> {
    let mut pins = Pins::new();
    let Some(Value::Map(map)) = args.value("resolve") else {
        return Ok(pins);
    };
    for (authority, addresses) in map {
        let list: Vec<String> = match addresses {
            Value::Text(text) => vec![text.clone()],
            Value::List(items) => items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_owned))
                .collect(),
            other => {
                return Err(Error::new(
                    ErrorCode::Type,
                    format!("`resolve` expects lists of addresses, not {}", other.type_name()),
                ));
            }
        };
        pins.insert(authority, &list)?;
    }
    Ok(pins)
}

fn proxy(args: &BoundArgs) -> Result<ProxyChoice> {
    Ok(match word(args, "proxy")? {
        None | Some("system") => ProxyChoice::System,
        Some("none") => ProxyChoice::None,
        Some(url) => ProxyChoice::Url(url.to_owned()),
    })
}

fn save_path(args: &BoundArgs) -> Result<Option<AbsPath>> {
    match args.value("save") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Path(path) | Value::Text(path)) => Ok(Some(AbsPath::new(path.as_str())?)),
        Some(other) => Err(Error::new(
            ErrorCode::Type,
            format!("`save` expects a path, not {}", other.type_name()),
        )),
    }
}

impl Params {
    fn from_args(args: &BoundArgs, mode: Addresses<'_>) -> Result<Self> {
        let (body, body_kind) = request_body(args, &mode)?;
        let params = Self {
            addresses: addresses(args, &mode)?,
            method: Method::parse(word(args, "method")?.unwrap_or("get")).ok_or_else(|| {
                Error::new(ErrorCode::UnknownValue, "unknown HTTP method; use get, head, post, put, patch, delete or options")
            })?,
            headers: headers(args)?,
            body,
            body_kind,
            save: save_path(args)?,
            form: choice(args, "as", Form::Raw, &[("raw", Form::Raw), ("text", Form::Text), ("markdown", Form::Markdown)])?,
            extract: choice(
                args,
                "extract",
                Extract::Content,
                &[
                    ("content", Extract::Content),
                    ("headers", Extract::Headers),
                    ("links", Extract::Links),
                    ("headings", Extract::Headings),
                    ("meta", Extract::Meta),
                    ("tables", Extract::Tables),
                ],
            )?,
            table: count(args, "table", 0, i64::MAX)?,
            convert: ConvertOptions {
                main_only: flag(args, "main_only", true)?,
                links: choice(args, "links", LinkStyle::Inline, &[("inline", LinkStyle::Inline), ("list", LinkStyle::List), ("none", LinkStyle::None)])?,
                images: choice(args, "images", ImageStyle::Alt, &[("drop", ImageStyle::Drop), ("alt", ImageStyle::Alt)])?,
                base: None,
            },
            section: word(args, "section")?.map(str::to_owned),
            outline: choice(args, "outline", OutlineMode::Auto, &[("auto", OutlineMode::Auto), ("always", OutlineMode::Always), ("never", OutlineMode::Never)])?,
            query: word(args, "query")?.map(str::to_owned),
            query_limit: query_limit(args)?,
            find: regex_arg(args, "find")?,
            find_context: count(args, "find_context", 1, MAX_FIND_CONTEXT)?,
            links_filter: regex_arg(args, "links_filter")?,
            same_host: flag(args, "same_host", false)?,
            charset: word(args, "charset")?.filter(|name| *name != "auto").map(str::to_owned),
            offset: count(args, "offset", 0, i64::MAX)?,
            cache: choice(
                args,
                "cache",
                CacheMode::Bypass,
                &[("bypass", CacheMode::Bypass), ("off", CacheMode::Bypass), ("use", CacheMode::Use), ("revalidate", CacheMode::Revalidate), ("only", CacheMode::Only)],
            )?,
            unchanged_since: word(args, "unchanged_since")?.map(str::to_owned),
            timeout: duration(args, "timeout", crate::client::DEFAULT_TIMEOUT)?,
            follow_redirects: flag(args, "follow_redirects", true)?,
            max_redirects: u32::try_from(count(args, "max_redirects", 10, i64::from(u32::MAX))?).unwrap_or(u32::MAX),
            verify_tls: flag(args, "verify_tls", true)?,
            retries: u32::try_from(count(args, "retries", 0, i64::from(u32::MAX))?).unwrap_or(u32::MAX),
            fail_on_status: flag(args, "fail_on_status", false)?,
            on_error_record: word(args, "on_error")? == Some("record"),
            max_size: size(args, "max_size", DEFAULT_MAX_SIZE)?,
            pins: pins(args)?,
            proxy: proxy(args)?,
        };
        params.validate(args)?;
        Ok(params)
    }

    fn validate(&self, args: &BoundArgs) -> Result<()> {
        if args.is_given("query_limit") && self.query.is_none() {
            return Err(conflict("`query_limit` applies only together with `query`"));
        }
        if self.table > 0 && self.extract != Extract::Tables {
            return Err(conflict("`table` applies only with `extract: tables`"));
        }
        let link_options = self.links_filter.is_some() || self.same_host;
        if link_options && self.extract != Extract::Links {
            return Err(conflict("`links_filter` and `same_host` apply only with `extract: links`"));
        }
        let selection = self.section.is_some() || self.find.is_some() || self.query.is_some();
        if selection && (self.form == Form::Raw || self.extract != Extract::Content) {
            return Err(conflict(
                "`section`, `find` and `query` apply to `as: text` or `as: markdown` with `extract: content`",
            ));
        }
        if self.method == Method::Head && self.body_kind != "none" {
            return Err(conflict("a `head` request has no body"));
        }
        if self.save.is_some() && self.addresses.len() > 1 {
            return Err(conflict("`save` writes one file; give one address"));
        }
        if self.save.is_some() && self.cache == CacheMode::Only {
            return Err(conflict("`save` needs the network; remove `cache: only`"));
        }
        Ok(())
    }

    fn facts(&self, address: AddressFact) -> RequestFacts {
        RequestFacts {
            stage: 0,
            command: COMMAND.to_owned(),
            deferred: false,
            method: self.method.as_str().to_owned(),
            address,
            follow_redirects: self.follow_redirects,
            max_redirects: self.max_redirects,
            retries: self.retries,
            proxy: self.proxy.clone(),
            pins: self.pins.clone(),
            verify_tls: self.verify_tls,
            header_names: self.headers.iter().map(|(name, _)| name.clone()).collect(),
            body: self.body_kind.to_owned(),
            save: self.save.as_ref().map(AbsPath::to_language_string),
            cache: self.cache.as_str().to_owned(),
        }
    }

    fn irreversible(&self) -> Vec<IrreversibleEntry> {
        if self.method.is_read_only() {
            return Vec::new();
        }
        self.addresses
            .iter()
            .map(|address| {
                let url = match address {
                    AddressFact::Fixed(url) | AddressFact::Dynamic(url) => url.as_str(),
                };
                let mut entry = IrreversibleEntry::new(Irreversible::Network);
                entry.detail = Some(format!("{} {url}", self.method.as_str()));
                entry
            })
            .collect()
    }

    fn textual_form(&self) -> TextForm {
        if self.form == Form::Text {
            TextForm::Text
        } else {
            TextForm::Markdown
        }
    }
}

fn query_limit(args: &BoundArgs) -> Result<usize> {
    let value = int(args, "query_limit", DEFAULT_QUERY_LIMIT)?;
    if !(1..=MAX_QUERY_LIMIT).contains(&value) {
        return Err(out_of_range("query_limit", value, 1, MAX_QUERY_LIMIT));
    }
    Ok(usize::try_from(value).unwrap_or(1))
}

/// Facts about a response used to build its record.
struct Head {
    status: u16,
    headers: Vec<(String, String)>,
    final_url: String,
    redirects: Vec<String>,
    remote: Option<String>,
    encoding: BodyEncoding,
    from_cache: bool,
}

impl Head {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(header, _)| header.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    fn media_type(&self) -> Option<String> {
        let media = self.header("content-type")?.split(';').next()?.trim();
        (!media.is_empty()).then(|| media.to_ascii_lowercase())
    }

    fn charset(&self) -> Option<String> {
        self.header("content-type")?
            .split(';')
            .skip(1)
            .filter_map(|part| part.split_once('='))
            .find(|(name, _)| name.trim().eq_ignore_ascii_case("charset"))
            .map(|(_, value)| value.trim().trim_matches('"').to_ascii_lowercase())
    }

    fn from_entry(entry: &CachedResponse) -> Self {
        Self {
            status: entry.status,
            headers: entry.headers.clone(),
            final_url: entry.final_url.clone(),
            redirects: Vec::new(),
            remote: None,
            encoding: BodyEncoding::Identity,
            from_cache: true,
        }
    }
}

/// A response with its body in memory, or the size of the body saved.
struct Obtained {
    head: Head,
    bytes: Vec<u8>,
    truncated: bool,
    saved: Option<u64>,
}

type Saver<'s> = &'s mut dyn FnMut(&mut ResponseBody) -> Result<u64>;

struct Runner<'p> {
    params: &'p Params,
    client: HttpClient,
    cache: Option<CacheStore>,
    identity: String,
}

impl<'p> Runner<'p> {
    fn new(params: &'p Params, ctx: &ExecCtx) -> Self {
        let settings = ClientSettings::from_config(ctx.config());
        let identity = format!(
            "{}\n{}",
            settings.user_agent,
            settings
                .ca_file
                .as_ref()
                .map(|path| path.display().to_string())
                .unwrap_or_default()
        );
        let mut client = HttpClient::new(settings);
        if let Some(resolver) = ctx.extensions().get::<dyn NameResolver>() {
            client = client.with_resolver(resolver);
        }
        Self {
            params,
            client,
            cache: ctx.state_dir().map(CacheStore::in_state_dir),
            identity,
        }
    }

    fn run_all(&self, ctx: &ExecCtx) -> Result<Vec<Record>> {
        let targets = self.targets(ctx)?;
        let results = self.fetch_parallel(&targets, ctx);
        let mut records = Vec::new();
        let mut first_error = None;
        let mut failures = 0;
        for (target, result) in targets.iter().zip(results) {
            match result {
                Ok(found) => records.extend(found),
                Err(error) if self.params.on_error_record => {
                    failures += 1;
                    report_failure(ctx, target, &error);
                    first_error.get_or_insert(error);
                }
                Err(error) => return Err(error),
            }
        }
        match first_error {
            Some(error) if failures == targets.len() => Err(error),
            _ => Ok(records),
        }
    }

    fn targets(&self, ctx: &ExecCtx) -> Result<Vec<Target>> {
        self.params
            .addresses
            .iter()
            .map(|address| {
                let text = match address {
                    AddressFact::Fixed(text) | AddressFact::Dynamic(text) => text,
                };
                let target = Target::parse(text)?;
                if target.scheme_assumed() {
                    log(ctx, LogLevel::Info, format!("scheme_assumed: `{text}` is requested as `{}`", target.as_str()));
                }
                Ok(target)
            })
            .collect()
    }

    fn fetch_parallel(&self, targets: &[Target], ctx: &ExecCtx) -> Vec<Result<Vec<Record>>> {
        if targets.len() == 1 {
            return vec![self.fetch(&targets[0], ctx, None)];
        }
        let slots: Mutex<Vec<Option<Result<Vec<Record>>>>> =
            Mutex::new((0..targets.len()).map(|_| None).collect());
        let next = AtomicUsize::new(0);
        std::thread::scope(|scope| {
            for _ in 0..PARALLEL.min(targets.len()) {
                scope.spawn(|| loop {
                    let index = next.fetch_add(1, Ordering::SeqCst);
                    let Some(target) = targets.get(index) else {
                        return;
                    };
                    let result = self.fetch(target, ctx, None);
                    slots.lock().unwrap_or_else(PoisonError::into_inner)[index] = Some(result);
                });
            }
        });
        slots
            .into_inner()
            .unwrap_or_else(PoisonError::into_inner)
            .into_iter()
            .map(|slot| slot.unwrap_or_else(|| Err(Error::new(ErrorCode::Internal, "a request was not run"))))
            .collect()
    }

    fn run_saving(&self, ctx: &ExecCtx, path: &AbsPath, tx: &mut dyn MutationTx) -> Result<(Vec<Record>, Change)> {
        let targets = self.targets(ctx)?;
        let target = targets.first().ok_or_else(|| Error::new(ErrorCode::Arg, "`url` needs an address"))?;
        let preserved = tx.preserve(&[TargetSpec {
            path: TargetPath::Fs(path.clone()),
            action: TargetAction::Modify,
            expect: Expect::Unchecked,
            size_hint: None,
        }])?;
        let first = preserved.first().ok_or_else(|| Error::new(ErrorCode::Internal, "the journal declared no target"))?;
        let index = first.index;
        let before = first.before.clone();
        let mut saver = |body: &mut ResponseBody| -> Result<u64> {
            let mut handle = tx.stage(index)?;
            let written = body.copy_to(&mut *handle)?;
            drop(handle);
            tx.apply(index, Mutation::CommitStaged)?;
            Ok(written)
        };
        let records = self.fetch(target, ctx, Some(&mut saver))?;
        let written = records
            .first()
            .and_then(|record| record.get("bytes"))
            .and_then(|bytes| match bytes {
                Value::Size(size) => Some(size.bytes()),
                _ => None,
            });
        Ok((records, save_change(path, before.exists, before.size, written)))
    }

    fn fetch(&self, target: &Target, ctx: &ExecCtx, saver: Option<Saver<'_>>) -> Result<Vec<Record>> {
        let started = Instant::now();
        let saving = saver.is_some();
        let obtained = self.obtain(target, ctx, saver)?;
        if self.params.fail_on_status && obtained.head.status >= 400 {
            return Err(Error::new(
                ErrorCode::Network,
                format!("`{}` answered with status {}", target.as_str(), obtained.head.status),
            )
            .with_problem("status")
            .with_actual(serde_json::json!(obtained.head.status)));
        }
        if obtained.truncated {
            log(ctx, LogLevel::Warn, format!("body_truncated: only the first {} bytes of `{}` were read", obtained.bytes.len(), target.as_str()));
        }
        let elapsed = started.elapsed();
        let render = Render { params: self.params, target, ctx };
        if saving {
            return Ok(vec![render.saved_record(&obtained, elapsed)]);
        }
        render.records(&obtained, elapsed)
    }

    fn cache_key(&self, target: &Target) -> String {
        CacheStore::key(self.params.method.as_str(), target.as_str(), &self.identity)
    }

    fn obtain(&self, target: &Target, ctx: &ExecCtx, saver: Option<Saver<'_>>) -> Result<Obtained> {
        let params = self.params;
        let cache = self
            .cache
            .as_ref()
            .filter(|_| params.method == Method::Get && saver.is_none() && params.cache != CacheMode::Bypass);
        let usable = cache.filter(|_| params.headers.is_empty() || params.cache == CacheMode::Only);
        let key = self.cache_key(target);
        let now = unix_seconds(ctx);
        let cached = usable.and_then(|store| store.load(&key)).filter(|entry| entry.serves(params.max_size));
        if let Some(found) = self.answer_from_cache(target, cached.as_ref(), now)? {
            if let Some(store) = usable {
                let _ = store.touch(&key, None, now);
            }
            return Ok(found);
        }
        let mut spec = self.spec(target);
        let revalidating = params.cache == CacheMode::Revalidate && cached.is_some();
        if let (true, Some(entry)) = (revalidating, cached.as_ref()) {
            conditional_headers(&mut spec, entry);
        }
        let mut exchange = self.client.send(&spec, ctx)?;
        if let (true, Some(entry), 304) = (revalidating, cached.as_ref(), exchange.head.status) {
            if let Some(store) = usable {
                let _ = store.touch(&key, Some(now), now);
            }
            return Ok(from_entry(entry));
        }
        let head = Head {
            status: exchange.head.status,
            headers: exchange.head.headers.clone(),
            final_url: exchange.head.final_url.clone(),
            redirects: exchange.head.redirects.iter().map(|step| step.location.clone()).collect(),
            remote: exchange.head.remote_address.map(|address| address.to_string()),
            encoding: exchange.body.encoding().clone(),
            from_cache: false,
        };
        if let Some(saver) = saver {
            let written = saver(&mut exchange.body)?;
            return Ok(Obtained { head, bytes: Vec::new(), truncated: false, saved: Some(written) });
        }
        let body = if params.method == Method::Head {
            crate::client::LimitedBody { bytes: Vec::new(), truncated: false }
        } else {
            exchange.body.read_limited(params.max_size)?
        };
        let storable = params.headers.is_empty()
            && head.status == 200
            && !matches!(head.encoding, BodyEncoding::Unsupported(_));
        if let (true, Some(store)) = (storable, usable) {
            let entry = CachedResponse {
                url: target.as_str().to_owned(),
                final_url: head.final_url.clone(),
                status: head.status,
                headers: head.headers.clone(),
                body: body.bytes.clone(),
                truncated: body.truncated,
                max_size: params.max_size,
                stored: now,
                used: now,
            };
            if let Err(error) = store.store(&key, &entry) {
                log(ctx, LogLevel::Warn, format!("the response of `{}` was not cached: {error}", target.as_str()));
            }
        }
        Ok(Obtained { head, bytes: body.bytes, truncated: body.truncated, saved: None })
    }

    fn answer_from_cache(&self, target: &Target, cached: Option<&CachedResponse>, now: i64) -> Result<Option<Obtained>> {
        let params = self.params;
        match (params.cache, cached) {
            (CacheMode::Only, Some(entry)) => Ok(Some(from_entry(entry))),
            (CacheMode::Only, None) => Err(Error::new(
                ErrorCode::NotFound,
                format!("no cached response for `{}`", target.as_str()),
            )
            .with_fix(format!("http.request \"{}\" cache: use", target.as_str()))),
            (CacheMode::Use, Some(entry)) if entry.is_fresh(now, self.cache_ttl()) => Ok(Some(from_entry(entry))),
            _ => Ok(None),
        }
    }

    fn cache_ttl(&self) -> Duration {
        self.cache.as_ref().map_or(crate::cache::DEFAULT_TTL, CacheStore::ttl)
    }

    fn spec(&self, target: &Target) -> RequestSpec {
        let params = self.params;
        let mut spec = RequestSpec::new(params.method, target.clone());
        spec.headers = params.headers.clone();
        spec.body = params.body.clone();
        spec.timeout = params.timeout;
        spec.follow_redirects = params.follow_redirects;
        spec.max_redirects = params.max_redirects;
        spec.retries = params.retries;
        spec.verify_tls = params.verify_tls;
        spec.pins = params.pins.clone();
        spec.proxy = params.proxy.clone();
        spec
    }
}

fn conditional_headers(spec: &mut RequestSpec, entry: &CachedResponse) {
    if let Some(etag) = entry.header("etag") {
        spec.headers.push(("If-None-Match".to_owned(), etag.to_owned()));
    }
    if let Some(modified) = entry.header("last-modified") {
        spec.headers.push(("If-Modified-Since".to_owned(), modified.to_owned()));
    }
}

fn from_entry(entry: &CachedResponse) -> Obtained {
    Obtained {
        head: Head::from_entry(entry),
        bytes: entry.body.clone(),
        truncated: entry.truncated,
        saved: None,
    }
}

fn unix_seconds(ctx: &ExecCtx) -> i64 {
    const NANOS: i128 = 1_000_000_000;
    i64::try_from(ctx.clock().now().0 / NANOS).unwrap_or(i64::MAX)
}

fn log(ctx: &ExecCtx, level: LogLevel, message: String) {
    ctx.emit(Event::Log(LogEvent {
        level,
        message,
        source: Some(COMMAND.to_owned()),
    }));
}

fn report_failure(ctx: &ExecCtx, target: &Target, error: &Error) {
    log(
        ctx,
        LogLevel::Error,
        format!("{}: {}: {}", target.as_str(), error.code(), error.data().message),
    );
}

/// Entity tag of a rendered body: the first digits of its SHA-256.
///
/// ```
/// use nurt_net::http::request::body_etag;
///
/// assert_eq!(body_etag(""), "e3b0c44298fc");
/// ```
#[must_use]
pub fn body_etag(body: &str) -> String {
    let digest = hex::encode(Sha256::digest(body.as_bytes()));
    digest[..ETAG_DIGITS].to_owned()
}

/// Kind of a document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DocKind {
    Html,
    Markdown,
    Plain,
}

fn doc_kind(media: Option<&str>, url: &str, text: &str) -> DocKind {
    match media {
        Some("text/html" | "application/xhtml+xml") => DocKind::Html,
        Some("text/markdown" | "text/x-markdown") => DocKind::Markdown,
        Some(_) => DocKind::Plain,
        None => {
            let start = text.trim_start().get(..15).unwrap_or_default().to_ascii_lowercase();
            if start.starts_with("<!doctype html") || start.starts_with("<html") {
                DocKind::Html
            } else if url.ends_with(".md") {
                DocKind::Markdown
            } else {
                DocKind::Plain
            }
        }
    }
}

/// Building of the records of one response.
struct Render<'a> {
    params: &'a Params,
    target: &'a Target,
    ctx: &'a ExecCtx,
}

/// Body text with the facts that go into the record.
struct BodyText {
    text: String,
    charset: Option<String>,
    total_chars: Option<u64>,
}

impl Render<'_> {
    fn base(&self, obtained: &Obtained, elapsed: Duration) -> HttpResponse {
        let head = &obtained.head;
        HttpResponse {
            url: self.target.as_str().to_owned(),
            status: u64::from(head.status),
            headers: record_headers(&head.headers),
            body: String::new(),
            bytes: Size::from_bytes(obtained.saved.unwrap_or(obtained.bytes.len() as u64)),
            elapsed: value::Duration::from_std(elapsed).unwrap_or(value::Duration::ZERO),
            final_url: head.final_url.clone(),
            saved: None,
            content_type: head.media_type(),
            charset: head.charset(),
            truncated: Some(obtained.truncated),
            offset: None,
            total_chars: None,
            redirects: (!head.redirects.is_empty()).then(|| head.redirects.clone()),
            from_cache: Some(head.from_cache),
            unchanged: None,
            etag: None,
            last_modified: head.header("last-modified").and_then(parse_http_date),
            remote_address: head.remote.clone(),
        }
    }

    fn saved_record(&self, obtained: &Obtained, elapsed: Duration) -> Record {
        let mut record = self.base(obtained, elapsed);
        record.saved = self.params.save.as_ref().map(|path| value::Path::from(path.to_language_string().as_str()));
        Record::from(record)
    }

    fn records(&self, obtained: &Obtained, elapsed: Duration) -> Result<Vec<Record>> {
        let mut record = self.base(obtained, elapsed);
        let empty = self.params.method == Method::Head || obtained.bytes.is_empty();
        if self.params.extract == Extract::Headers || empty {
            return Ok(vec![self.finish(record, String::new())]);
        }
        let decoded = self.decode(obtained)?;
        record.charset = decoded.charset.clone().or(record.charset);
        if self.params.extract == Extract::Tables {
            let page = self.page(&decoded.text, &obtained.head);
            return self.rows(&page, &obtained.head);
        }
        let body = self.body(&decoded, &obtained.head)?;
        record.total_chars = body.total_chars;
        if self.params.offset > 0 {
            record.offset = Some(self.params.offset as u64);
        }
        Ok(vec![self.finish(record, body.text)])
    }

    fn finish(&self, mut record: HttpResponse, body: String) -> Record {
        let etag = body_etag(&body);
        let unchanged = self.params.unchanged_since.as_deref() == Some(etag.as_str());
        record.body = if unchanged { String::new() } else { body };
        record.unchanged = Some(unchanged);
        record.etag = Some(etag);
        Record::from(record)
    }

    fn decode(&self, obtained: &Obtained) -> Result<html::Decoded> {
        let head = &obtained.head;
        let media = head.media_type();
        let textual = !matches!(head.encoding, BodyEncoding::Unsupported(_))
            && html::is_textual(media.as_deref(), &obtained.bytes);
        if !textual {
            return Err(Error::new(
                ErrorCode::Binary,
                format!(
                    "the body of `{}` is not text ({})",
                    self.target.as_str(),
                    media.as_deref().unwrap_or("unknown type")
                ),
            )
            .with_fix(format!("http.request \"{}\" save: \"/tmp/body\"", self.target.as_str())));
        }
        let is_html = doc_kind(media.as_deref(), self.target.as_str(), "") == DocKind::Html;
        html::decode(&obtained.bytes, head.charset().as_deref(), self.params.charset.as_deref(), is_html || media.is_none())
    }

    fn page(&self, text: &str, head: &Head) -> Page {
        let media = head.media_type();
        match doc_kind(media.as_deref(), &head.final_url, text) {
            DocKind::Html => {
                let options = ConvertOptions {
                    base: url::Url::parse(&head.final_url).ok(),
                    ..self.params.convert.clone()
                };
                convert_html(text, &options)
            }
            DocKind::Markdown => convert_markdown(text),
            DocKind::Plain => convert_text(text),
        }
    }

    fn body(&self, decoded: &html::Decoded, head: &Head) -> Result<BodyText> {
        let full = match (self.params.extract, self.params.form) {
            (Extract::Content, Form::Raw) => decoded.text.clone(),
            (Extract::Content, _) => self.content(&self.page(&decoded.text, head))?,
            (Extract::Links, _) => self.links(&self.page(&decoded.text, head)),
            (Extract::Headings, _) => self.headings(&self.page(&decoded.text, head)),
            (Extract::Meta, _) => meta(&self.page(&decoded.text, head)),
            (Extract::Headers | Extract::Tables, _) => String::new(),
        };
        let total = full.chars().count();
        if self.params.offset > total {
            return Err(Error::new(
                ErrorCode::Arg,
                format!("`offset` {} is past the end of the body of {total} characters", self.params.offset),
            )
            .with_actual(serde_json::json!(total)));
        }
        Ok(BodyText {
            text: full.chars().skip(self.params.offset).collect(),
            charset: Some(decoded.charset.clone()),
            total_chars: Some(total as u64),
        })
    }

    fn content(&self, page: &Page) -> Result<String> {
        let params = self.params;
        let form = params.textual_form();
        let mut blocks = page.blocks.clone();
        let mut selected = false;
        if let Some(title) = &params.section {
            blocks = select::section(&blocks, title)
                .map_err(|candidates| section_missing(title, &candidates))?
                .to_vec();
            selected = true;
        }
        if let Some(pattern) = &params.find {
            blocks = select::find(&blocks, pattern, params.find_context);
            selected = true;
        }
        if let Some(query) = &params.query {
            blocks = select::query(&blocks, query, params.query_limit);
            selected = true;
        }
        let rendered = html::render(&blocks, form, params.convert.links);
        let max_chars = usize::try_from(self.ctx.options().max_chars()).unwrap_or(usize::MAX);
        let long = max_chars > 0 && rendered.chars().count() > max_chars;
        let outline = match params.outline {
            OutlineMode::Always => true,
            OutlineMode::Auto => long && params.offset == 0,
            OutlineMode::Never => false,
        };
        if selected || !outline {
            return Ok(rendered);
        }
        let saved = self.save_full(&rendered, form);
        let entries = select::sections(&blocks, form);
        Ok(select::outline(page.metadata.title.as_deref(), &rendered, &entries, saved.as_deref()))
    }

    fn save_full(&self, content: &str, form: TextForm) -> Option<String> {
        let dir = self.ctx.state_dir()?.join(RESULTS_DIR);
        let extension = if form == TextForm::Markdown { "md" } else { "txt" };
        let path = dir.join(format!("{}.{extension}", body_etag(content)));
        let written = std::fs::create_dir_all(&dir).and_then(|()| std::fs::write(&path, content));
        match written {
            Ok(()) => Some(path.display().to_string()),
            Err(error) => {
                log(self.ctx, LogLevel::Warn, format!("the full content was not kept: {error}"));
                None
            }
        }
    }

    fn links(&self, page: &Page) -> String {
        let host = url::Url::parse(self.target.as_str()).ok().and_then(|url| url.host_str().map(str::to_owned));
        page.links
            .iter()
            .filter(|link| self.params.links_filter.as_ref().is_none_or(|pattern| pattern.is_match(&link.url)))
            .filter(|link| {
                !self.params.same_host
                    || url::Url::parse(&link.url).ok().and_then(|url| url.host_str().map(str::to_owned)) == host
            })
            .map(|link| match self.params.textual_form() {
                TextForm::Markdown => format!("- [{}]({})", link.text, link.url),
                TextForm::Text => format!("{} {}", link.text, link.url).trim().to_owned(),
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn headings(&self, page: &Page) -> String {
        select::sections(&page.blocks, self.params.textual_form())
            .iter()
            .map(|entry| format!("{} {} ({})", "#".repeat(usize::from(entry.level)), entry.title, entry.chars))
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn rows(&self, page: &Page, head: &Head) -> Result<Vec<Record>> {
        let wanted = self.params.table;
        if wanted > page.tables.len() {
            return Err(Error::new(
                ErrorCode::NotFound,
                format!("table {wanted} does not exist; the page has {} tables", page.tables.len()),
            )
            .with_actual(serde_json::json!(page.tables.len())));
        }
        let mut records = Vec::new();
        for (index, table) in page.tables.iter().enumerate() {
            let number = index + 1;
            if wanted != 0 && wanted != number {
                continue;
            }
            for (row_index, row) in table.rows.iter().enumerate() {
                let cells: IndexMap<String, Value> = table
                    .columns
                    .iter()
                    .enumerate()
                    .map(|(column, name)| (name.clone(), Value::Text(row.get(column).cloned().unwrap_or_default())))
                    .collect();
                records.push(Record::from(Row {
                    path: value::Path::from(head.final_url.as_str()),
                    no: row_index as u64 + 1,
                    cells,
                    table: Some(number.to_string()),
                }));
            }
        }
        Ok(records)
    }
}

fn section_missing(title: &str, candidates: &[String]) -> Error {
    Error::new(
        ErrorCode::NotFound,
        format!("the page has no section `{title}`"),
    )
    .with_expected(serde_json::json!(candidates))
}

fn meta(page: &Page) -> String {
    let fields = [
        ("title", &page.metadata.title),
        ("description", &page.metadata.description),
        ("language", &page.metadata.language),
        ("canonical", &page.metadata.canonical),
    ];
    fields
        .iter()
        .filter_map(|(name, value)| value.as_ref().map(|value| format!("{name}: {value}")))
        .collect::<Vec<_>>()
        .join("\n")
}

fn record_headers(headers: &[(String, String)]) -> IndexMap<String, Value> {
    let mut out: IndexMap<String, Value> = IndexMap::new();
    for (name, value) in headers {
        let name = name.to_ascii_lowercase();
        if name == "set-cookie" {
            let entry = out.entry(name).or_insert_with(|| Value::List(Vec::new()));
            if let Value::List(items) = entry {
                items.push(Value::Text(value.clone()));
            }
            continue;
        }
        match out.get_mut(&name) {
            Some(Value::Text(existing)) => {
                existing.push_str(", ");
                existing.push_str(value);
            }
            _ => {
                out.insert(name, Value::Text(value.clone()));
            }
        }
    }
    out
}

fn parse_http_date(text: &str) -> Option<value::Instant> {
    let zoned = jiff::fmt::rfc2822::parse(text.trim()).ok()?;
    let nanos = i64::try_from(zoned.timestamp().as_nanosecond()).ok()?;
    Some(value::Instant::from_unix_nanos(nanos))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn joins_repeated_headers_and_lists_cookies() {
        let headers = vec![
            ("Vary".to_owned(), "a".to_owned()),
            ("vary".to_owned(), "b".to_owned()),
            ("Set-Cookie".to_owned(), "x=1".to_owned()),
            ("set-cookie".to_owned(), "y=2".to_owned()),
        ];
        let map = record_headers(&headers);
        assert_eq!(map["vary"], Value::Text("a, b".to_owned()));
        assert_eq!(
            map["set-cookie"],
            Value::List(vec![Value::Text("x=1".to_owned()), Value::Text("y=2".to_owned())])
        );
    }

    #[test]
    fn parses_http_dates() {
        let instant = parse_http_date("Sun, 06 Nov 1994 08:49:37 GMT").unwrap();
        assert_eq!(instant.as_unix_nanos(), 784_111_777_000_000_000);
        assert!(parse_http_date("yesterday").is_none());
    }

    #[test]
    fn detects_document_kinds() {
        assert_eq!(doc_kind(Some("text/html"), "", ""), DocKind::Html);
        assert_eq!(doc_kind(None, "", "  <!DOCTYPE html><p>"), DocKind::Html);
        assert_eq!(doc_kind(None, "https://a.test/README.md", "# x"), DocKind::Markdown);
        assert_eq!(doc_kind(Some("text/plain"), "", "<html>"), DocKind::Plain);
    }
}
