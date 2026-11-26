// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Stages of the `program` command group.
//!
//! | Command | Output |
//! |---|---|
//! | `program.find` | `program_location`, one per name, from `PATH` only |
//! | `program.search` | `program_card` with `score`, best first |
//! | `program.describe` | `program_card` with description, guide and help |
//! | `program.catalog` | `program_category`, or `program_card` of a category |
//!
//! The commands work without configuration on the programs of the
//! computer; external catalogs add entries. Warnings of providers are
//! emitted as log events of level `warn`; they never fail a command.
//! Fields beyond the `program_card`, `program_category` and
//! `program_location` contracts are added to the dynamic records.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use nurt_contracts::error::{Candidate, Error, ErrorCode, Result};
use nurt_contracts::kinds::Implementation;
use nurt_contracts::record::{
    ProgramCard, ProgramCategory, ProgramLocation, ProgramSource as RecordSource,
};
use nurt_contracts::stage::{
    BoundArgs, CheckCtx, Event, ExecCtx, ExternalProgram, InputKinds, LogEvent, LogLevel,
    ReadStage, RecordStream, StageDescriptor, StageImpl, StageInput, StageOutput, StaticInfo,
};
use nurt_contracts::value::{Fields, Instant, Path as ValuePath, Record, Value};
use sha2::{Digest, Sha256};

use crate::help::{HelpResult, read_help};
use crate::index::{Card, IndexCache, IndexOptions, ProgramIndex};
use crate::model::{DetailWant, EntryKey, EntryKind, LocalizedText, SectionKind};
use crate::search::{
    KindFilter, Query, SIMILAR_LIMIT, SearchOptions, Section, SourceFilter, Target, card_sections,
    find_section, guide_sections, match_sections, search, similar,
};
use crate::source::{HostEnv, SourceId, SourceKind, SourceRegistry, SourceWarning};
use crate::sources::path::resolve_from;
use crate::taxonomy::OTHER_SUBCATEGORY;

/// Command `program.find`.
pub const FIND: &str = "program.find";
/// Command `program.search`.
pub const SEARCH: &str = "program.search";
/// Command `program.describe`.
pub const DESCRIBE: &str = "program.describe";
/// Command `program.catalog`.
pub const CATALOG: &str = "program.catalog";
/// Shortest query.
pub const QUERY_MIN_CHARS: usize = 2;
/// Longest query.
pub const QUERY_MAX_CHARS: usize = 200;
/// Most queries of one search.
pub const MAX_QUERIES: usize = 8;
/// Most names of one description.
pub const MAX_NAMES: usize = 12;
/// Longest program name.
pub const NAME_MAX_CHARS: usize = 120;
/// Default number of search results.
pub const SEARCH_LIMIT: i64 = 15;
/// Default number of results of a query inside a category.
pub const CATEGORY_QUERY_LIMIT: i64 = 30;
/// Largest `limit`.
pub const MAX_LIMIT: i64 = 60;
/// Hexadecimal characters of an entity tag.
pub const ETAG_CHARS: usize = 8;
/// Title of the replacement subcategory.
pub const OTHER_TITLE: &str = "outside subcategories";
const LOG_SOURCE: &str = "program";

/// Configuration shared by the stages: providers, environment and index
/// cache. The defaults read the computer through the execution context.
#[derive(Clone, Default)]
pub struct ProgramsSetup {
    /// Providers; `None` uses the built-in providers enabled by the
    /// configuration.
    pub registry: Option<SourceRegistry>,
    /// Environment; `None` derives it from the execution context.
    pub env: Option<HostEnv>,
    /// Index options; `None` derives them from the configuration.
    pub options: Option<IndexOptions>,
    /// Cache of the index; `None` uses the cache of the process.
    pub cache: Option<Arc<IndexCache>>,
    /// Ranking options; `None` uses [`SearchOptions::FULL`].
    pub ranking: Option<SearchOptions>,
}

impl std::fmt::Debug for ProgramsSetup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProgramsSetup")
            .field("registry", &self.registry)
            .field("env", &self.env)
            .field("options", &self.options)
            .finish_non_exhaustive()
    }
}

impl ProgramsSetup {
    fn env(&self, ctx: &ExecCtx) -> HostEnv {
        self.env.clone().unwrap_or_else(|| HostEnv::from_ctx(ctx))
    }

    fn index(&self, ctx: &ExecCtx, env: &HostEnv) -> Arc<ProgramIndex> {
        let config = &ctx.config().programs;
        let registry = self
            .registry
            .clone()
            .unwrap_or_else(|| crate::sources::builtin(env, config));
        let options = self
            .options
            .clone()
            .unwrap_or_else(|| IndexOptions::from_config(config, env));
        let cache = self
            .cache
            .as_deref()
            .unwrap_or_else(|| IndexCache::global());
        let index = cache.get(&registry, env, &options, ctx.clock());
        emit_warnings(ctx, index.warnings());
        index
    }

    fn ranking(&self) -> SearchOptions {
        self.ranking.unwrap_or(SearchOptions::FULL)
    }
}

/// The four stages with the default setup.
#[must_use]
pub fn stages() -> Vec<Arc<dyn StageImpl>> {
    stages_with(ProgramsSetup::default())
}

/// The four stages with a setup.
#[must_use]
pub fn stages_with(setup: ProgramsSetup) -> Vec<Arc<dyn StageImpl>> {
    [
        Command::Find,
        Command::Search,
        Command::Describe,
        Command::Catalog,
    ]
    .into_iter()
    .map(|command| {
        Arc::new(ProgramStage {
            command,
            setup: setup.clone(),
        }) as Arc<dyn StageImpl>
    })
    .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Command {
    Find,
    Search,
    Describe,
    Catalog,
}

impl Command {
    fn name(self) -> &'static str {
        match self {
            Self::Find => FIND,
            Self::Search => SEARCH,
            Self::Describe => DESCRIBE,
            Self::Catalog => CATALOG,
        }
    }
}

/// One command of the group.
#[derive(Debug, Clone)]
pub struct ProgramStage {
    command: Command,
    setup: ProgramsSetup,
}

impl StageImpl for ProgramStage {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: self.command.name().to_owned(),
            source: true,
            input: InputKinds::None,
        }
    }

    fn check(&self, args: &BoundArgs, _ctx: &CheckCtx<'_>) -> Result<StaticInfo> {
        let mut info = StaticInfo::default();
        if self.command == Command::Describe && args.bool("help")?.unwrap_or(false) {
            for name in texts(args, "name")? {
                info.programs.push(ExternalProgram {
                    program: name,
                    implementation: Implementation::UserProgram,
                });
            }
        }
        Ok(info)
    }

    fn as_read(&self) -> Option<&dyn ReadStage> {
        Some(self)
    }
}

impl ReadStage for ProgramStage {
    fn execute<'a>(
        &'a self,
        args: &'a BoundArgs,
        _input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<StageOutput<'a>> {
        let records = match self.command {
            Command::Find => find(&self.setup, args, ctx)?,
            Command::Search => run_search(&self.setup, args, ctx)?,
            Command::Describe => describe(&self.setup, args, ctx)?,
            Command::Catalog => catalog(&self.setup, args, ctx)?,
        };
        Ok(StageOutput::records(RecordStream::from_records(records)))
    }
}

fn emit_warnings(ctx: &ExecCtx, warnings: &[SourceWarning]) {
    for warning in warnings {
        let place = warning
            .path
            .as_deref()
            .map(|path| format!(" ({path})"))
            .unwrap_or_default();
        ctx.emit(Event::Log(LogEvent {
            level: LogLevel::Warn,
            message: format!(
                "{}: {}: {}{place}",
                warning.code.as_str(),
                warning.source,
                warning.reason
            ),
            source: Some(LOG_SOURCE.to_owned()),
        }));
    }
}

fn warn(ctx: &ExecCtx, message: String) {
    ctx.emit(Event::Log(LogEvent {
        level: LogLevel::Warn,
        message,
        source: Some(LOG_SOURCE.to_owned()),
    }));
}

/// Texts of a `text` or `list<text>` argument.
fn texts(args: &BoundArgs, name: &str) -> Result<Vec<String>> {
    let mut out = Vec::new();
    collect_texts(args.value(name), &mut out);
    let mut unique: Vec<String> = Vec::new();
    for item in out
        .into_iter()
        .map(|item| item.trim().to_owned())
        .filter(|item| !item.is_empty())
    {
        if !unique.contains(&item) {
            unique.push(item);
        }
    }
    Ok(unique)
}

fn collect_texts(value: Option<&Value>, out: &mut Vec<String>) {
    match value {
        Some(Value::Text(text) | Value::Enum(text) | Value::Path(text)) => out.push(text.clone()),
        Some(Value::List(items)) => {
            for item in items {
                collect_texts(Some(item), out);
            }
        }
        _ => {}
    }
}

fn invalid(command: &str, message: String) -> Error {
    Error::new(ErrorCode::InvalidValue, format!("`{command}`: {message}"))
}

fn limit(args: &BoundArgs, command: &str, default: i64) -> Result<usize> {
    let value = args.int("limit")?.unwrap_or(default);
    if !(1..=MAX_LIMIT).contains(&value) {
        return Err(invalid(
            command,
            format!("limit must be between 1 and {MAX_LIMIT}, not {value}"),
        )
        .with_expected(serde_json::json!({ "min": 1, "max": MAX_LIMIT })));
    }
    Ok(usize::try_from(value).unwrap_or(1))
}

fn check_query(command: &str, query: &str) -> Result<()> {
    let length = query.chars().count();
    if (QUERY_MIN_CHARS..=QUERY_MAX_CHARS).contains(&length) {
        Ok(())
    } else {
        Err(Error::new(
            ErrorCode::Arg,
            format!(
                "`{command}`: query must have {QUERY_MIN_CHARS} to {QUERY_MAX_CHARS} characters"
            ),
        )
        .with_found(query.to_owned()))
    }
}

fn candidates(error: Error, programs: impl IntoIterator<Item = String>) -> Error {
    let mut error = error;
    for (rank, program) in programs.into_iter().enumerate() {
        error.candidates.push(Candidate {
            rank: u32::try_from(rank + 1).unwrap_or(u32::MAX),
            reference: None,
            program,
            why: None,
        });
    }
    error
}

fn quoted(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}

fn find(setup: &ProgramsSetup, args: &BoundArgs, ctx: &ExecCtx) -> Result<Vec<Record>> {
    let names = texts(args, "name")?;
    if names.is_empty() {
        return Err(Error::new(
            ErrorCode::MissingParam,
            "`program.find` needs at least one name",
        ));
    }
    let versions = args.bool("versions")?.unwrap_or(false);
    let all = args.bool("all")?.unwrap_or(false);
    let env = setup.env(ctx);
    let index = versions.then(|| setup.index(ctx, &env));
    let base = base_directory(ctx);
    let mut records = Vec::with_capacity(names.len());
    for name in names {
        ctx.checkpoint()?;
        let resolution = resolve_from(&env, &name, base.as_deref());
        let first = resolution
            .paths
            .first()
            .map(|path| path.to_string_lossy().into_owned());
        let located = index
            .as_deref()
            .and_then(|index| location_version(index, &name, first.as_deref()));
        let version = located.as_ref().map(|(version, _)| version.clone());
        let location = ProgramLocation {
            name: name.clone(),
            found: first.is_some(),
            path: first.clone().map(ValuePath::from),
            version: version.clone(),
            source: first.as_ref().map(|_| RecordSource::Path),
            all_paths: all.then(|| {
                resolution
                    .paths
                    .iter()
                    .map(|path| ValuePath::from(path.to_string_lossy().into_owned()))
                    .collect()
            }),
        };
        let mut record = Record::from(location);
        if let Some((_, source)) = located {
            record.set("version_source", Value::Enum(source.as_str().to_owned()));
        }
        if let Some(alias) = resolution.alias_of {
            record.set("alias_of", Value::Text(alias));
        }
        records.push(record);
    }
    Ok(records)
}

/// Directory a relative name with a path separator is taken relative to: the
/// `base` option of the request, otherwise `scope.base` of the configuration;
/// only an absolute directory counts.
fn base_directory(ctx: &ExecCtx) -> Option<PathBuf> {
    ctx.options()
        .base
        .as_deref()
        .or(ctx.config().scope.base.as_deref())
        .map(PathBuf::from)
        .filter(|base| base.is_absolute())
}

/// Version of the program at `path` and the kind of provider that gave it.
fn location_version(
    index: &ProgramIndex,
    name: &str,
    path: Option<&str>,
) -> Option<(String, SourceKind)> {
    let path = path?;
    let (position, _) = index.resolve(name)?;
    let card = index.card(position)?;
    if card.path.as_deref() != Some(path) && !card.alternatives.iter().any(|known| known == path) {
        return None;
    }
    Some((card.version.clone()?, card.version_source?))
}

fn source_filter(index: &ProgramIndex, args: &BoundArgs, command: &str) -> Result<SourceFilter> {
    let mut filter = SourceFilter::default();
    for value in texts(args, "source")? {
        match value.as_str() {
            "path" => filter.kinds.push(SourceKind::Path),
            "package" => filter.kinds.push(SourceKind::Package),
            "app" => filter.kinds.push(SourceKind::App),
            "catalog" => filter.kinds.push(SourceKind::Catalog),
            "man" => filter.kinds.push(SourceKind::Description),
            id if index.source(&SourceId::new(id)).is_some() => filter.ids.push(id.to_owned()),
            other => {
                let known: Vec<String> = ["path", "package", "app", "catalog"]
                    .into_iter()
                    .map(str::to_owned)
                    .chain(index.sources().iter().map(|source| source.id().to_string()))
                    .collect();
                let error = invalid(
                    command,
                    format!("unknown source `{other}`; use one of the listed sources"),
                )
                .with_expected(serde_json::json!(known));
                return Err(if other.starts_with(SourceId::CATALOG_PREFIX) {
                    Error::new(
                        ErrorCode::Catalog,
                        format!(
                            "`{command}`: catalog `{other}` is not configured or cannot be read"
                        ),
                    )
                    .with_expected(serde_json::json!(known))
                } else {
                    error
                });
            }
        }
    }
    let all_kinds = [
        SourceKind::Path,
        SourceKind::Package,
        SourceKind::App,
        SourceKind::Catalog,
    ];
    if filter.ids.is_empty() && all_kinds.iter().all(|kind| filter.kinds.contains(kind)) {
        filter.kinds.clear();
    }
    Ok(filter)
}

fn category_error(index: &ProgramIndex, command: &str, requested: &str) -> Error {
    let ids: Vec<String> = visible_categories(index, &SourceFilter::default())
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    let mut ranked: Vec<(usize, &String)> = ids
        .iter()
        .map(|id| {
            (
                crate::text::edit_distance(&crate::text::fold(requested), id),
                id,
            )
        })
        .collect();
    ranked.sort();
    let error = invalid(
        command,
        format!("unknown category `{requested}`; use one of the listed categories"),
    )
    .with_expected(serde_json::json!(ids));
    candidates(
        error,
        ranked
            .into_iter()
            .take(SIMILAR_LIMIT)
            .map(|(_, id)| format!("{CATALOG} {}", quoted(id))),
    )
}

fn run_search(setup: &ProgramsSetup, args: &BoundArgs, ctx: &ExecCtx) -> Result<Vec<Record>> {
    let queries = texts(args, "query")?;
    if queries.is_empty() {
        return Err(Error::new(
            ErrorCode::MissingParam,
            "`program.search` needs a query",
        ));
    }
    if queries.len() > MAX_QUERIES {
        return Err(invalid(
            SEARCH,
            format!("at most {MAX_QUERIES} queries are accepted"),
        ));
    }
    for query in &queries {
        check_query(SEARCH, query)?;
    }
    let limit = limit(args, SEARCH, SEARCH_LIMIT)?;
    let kind_text = args.text("kind")?.unwrap_or("all");
    let kind = KindFilter::parse(kind_text).ok_or_else(|| {
        invalid(
            SEARCH,
            format!("unknown kind `{kind_text}`; use all, programs, apps or guides"),
        )
    })?;
    let installed = args.bool("installed")?;
    let env = setup.env(ctx);
    let index = setup.index(ctx, &env);
    let filter = source_filter(&index, args, SEARCH)?;
    let category = args.text("category")?.filter(|text| !text.is_empty());
    let category = match category {
        Some(requested) => Some(
            index
                .category(requested)
                .map(|found| found.id.clone())
                .ok_or_else(|| category_error(&index, SEARCH, requested))?,
        ),
        None => None,
    };
    let mut records = Vec::new();
    for text in &queries {
        let query = Query {
            text,
            category: category.as_deref(),
            kind,
            sources: &filter,
            installed,
            limit,
        };
        for hit in search(&index, &query, setup.ranking()) {
            let mut record = hit_record(&index, &hit.target, env.languages());
            record.set("score", Value::Number(f64::from(hit.points)));
            if queries.len() > 1 {
                record.set("query", Value::Text(text.clone()));
            }
            if let Some(instead) = hit.instead {
                record.set("instead", Value::Text(instead));
            }
            records.push(record);
        }
    }
    Ok(records)
}

fn hit_record(index: &ProgramIndex, target: &Target, languages: &[String]) -> Record {
    match *target {
        Target::Card(position) => index.card(position).map_or_else(
            || Record::from(ProgramCard::default()),
            |card| card_record(index, card, languages),
        ),
        Target::Guide(position) => guide_record(index, position, languages),
    }
}

fn pick(text: &LocalizedText, languages: &[String]) -> String {
    text.pick(languages).0.to_owned()
}

fn list(values: impl IntoIterator<Item = String>) -> Value {
    Value::List(values.into_iter().map(Value::Text).collect())
}

fn date_instant(date: &str) -> Option<Instant> {
    format!("{date}T00:00:00Z").parse().ok()
}

fn record_source(card: &Card) -> RecordSource {
    match card.source {
        Some(SourceKind::Package) => RecordSource::Package,
        Some(SourceKind::App) => RecordSource::App,
        Some(SourceKind::Catalog) => RecordSource::Catalog,
        _ => RecordSource::Path,
    }
}

fn shared_with(index: &ProgramIndex, card: &Card) -> Vec<String> {
    let Some((_, guide)) = &card.guide else {
        return Vec::new();
    };
    index
        .cards()
        .iter()
        .filter(|other| other.name != card.name)
        .filter(|other| {
            other
                .guide
                .as_ref()
                .is_some_and(|(_, known)| known.name == guide.name)
        })
        .map(|other| other.name.clone())
        .collect()
}

/// Record of a card with the fields shown in listings.
fn card_record(index: &ProgramIndex, card: &Card, languages: &[String]) -> Record {
    let shared = shared_with(index, card);
    let typed = ProgramCard {
        name: card.name.clone(),
        summary: pick(&card.summary, languages),
        source: record_source(card),
        installed: card.installed,
        guide_optional: card.guide_optional,
        sources: card.sources.iter().map(ToString::to_string).collect(),
        category: Some(card.category.clone()),
        subcategory: card.subcategory.clone(),
        path: card.path.clone().map(ValuePath::from),
        version: card.version.clone(),
        guide_shared: card
            .guide
            .as_ref()
            .filter(|_| !shared.is_empty())
            .map(|(_, guide)| guide.name.clone()),
        package: card.package.as_ref().map(|package| package.name.clone()),
        homepage: card.homepage.clone(),
        license: card.license.clone(),
        aliases: (!card.aliases.is_empty()).then(|| {
            card.aliases
                .iter()
                .map(|alias| alias.name.clone())
                .collect()
        }),
        verified: card.verified.as_deref().and_then(date_instant),
        tags: (!card.tags.is_empty()).then(|| card.tags.clone()),
        platforms: (!card.platforms.is_empty()).then(|| card.platforms.clone()),
        ..ProgramCard::default()
    };
    let mut record = Record::from(typed);
    record.set("kind", Value::Enum(card.kind.as_str().to_owned()));
    if let Some(source) = card.category_source {
        record.set("category_source", Value::Enum(source.as_str().to_owned()));
    }
    record.set("has_guide", Value::Bool(card.guide.is_some()));
    if card.guide_required {
        record.set("guide_required", Value::Bool(true));
    }
    if let Some((_, guide)) = &card.guide {
        record.set("guide_name", Value::Text(guide.name.clone()));
    }
    if !shared.is_empty() {
        record.set("guide_shared_with", list(shared));
    }
    if let Some(overview) = &card.overview {
        record.set("overview", Value::Text(pick(overview, languages)));
    }
    if !card.alternatives.is_empty() {
        record.set("alternatives", list(card.alternatives.clone()));
    }
    if let Some(path) = &card.catalog_path {
        record.set("catalog_path", Value::Text(path.clone()));
    }
    if let Some(package) = &card.package {
        record.set("manager", Value::Text(package.manager.to_string()));
    }
    if let Some(app_id) = &card.app_id {
        record.set("app_id", Value::Text(app_id.clone()));
    }
    if let Some(lang) = card.summary.pick(languages).1 {
        record.set("language", Value::Text(lang.to_owned()));
    }
    record
}

fn guide_record(index: &ProgramIndex, position: usize, languages: &[String]) -> Record {
    let Some(entry) = index.guides().get(position) else {
        return Record::from(ProgramCard::default());
    };
    let guide = &entry.guide;
    let typed = ProgramCard {
        name: guide.name.clone(),
        summary: guide
            .summary
            .as_ref()
            .map(|text| pick(text, languages))
            .unwrap_or_default(),
        source: RecordSource::Catalog,
        installed: false,
        guide_optional: false,
        sources: vec![entry.source.to_string()],
        category: guide.category.clone(),
        tags: (!guide.tags.is_empty()).then(|| guide.tags.clone()),
        ..ProgramCard::default()
    };
    let mut record = Record::from(typed);
    record.set("kind", Value::Enum(EntryKind::Guide.as_str().to_owned()));
    record.set("has_guide", Value::Bool(true));
    record.set("guide_name", Value::Text(guide.name.clone()));
    record
}

struct Resolved {
    requested: String,
    target: Target,
    alias_kind: Option<crate::model::AliasKind>,
}

fn resolve_name(index: &ProgramIndex, name: &str, filter: &SourceFilter) -> Option<Resolved> {
    if let Some((position, alias_kind)) = index.resolve(name) {
        let card = index.card(position)?;
        if filter.admits(card, index) {
            return Some(Resolved {
                requested: name.to_owned(),
                target: Target::Card(position),
                alias_kind,
            });
        }
    }
    let key = crate::text::fold(name);
    index
        .guides()
        .iter()
        .position(|guide| guide.folded_name == key && filter.admits_guide(guide))
        .map(|position| Resolved {
            requested: name.to_owned(),
            target: Target::Guide(position),
            alias_kind: None,
        })
}

struct DescribeOptions {
    include_guide: bool,
    section: Option<String>,
    query: Option<String>,
    unchanged_since: Option<String>,
    help: bool,
    help_timeout: Duration,
}

fn describe(setup: &ProgramsSetup, args: &BoundArgs, ctx: &ExecCtx) -> Result<Vec<Record>> {
    let names = texts(args, "name")?;
    if names.is_empty() {
        return Err(Error::new(
            ErrorCode::MissingParam,
            "`program.describe` needs a program name",
        ));
    }
    if names.len() > MAX_NAMES {
        return Err(invalid(
            DESCRIBE,
            format!("at most {MAX_NAMES} names are accepted"),
        ));
    }
    if let Some(long) = names
        .iter()
        .find(|name| name.chars().count() > NAME_MAX_CHARS)
    {
        return Err(invalid(
            DESCRIBE,
            format!("names have at most {NAME_MAX_CHARS} characters"),
        )
        .with_found(long.clone()));
    }
    let query = args
        .text("query")?
        .filter(|text| !text.is_empty())
        .map(str::to_owned);
    if let Some(query) = &query {
        check_query(DESCRIBE, query)?;
    }
    let options = DescribeOptions {
        include_guide: args.bool("include_guide")?.unwrap_or(true),
        section: args
            .text("section")?
            .filter(|text| !text.is_empty())
            .map(str::to_owned),
        query,
        unchanged_since: args.text("unchanged_since")?.map(str::to_owned),
        help: args.bool("help")?.unwrap_or(false),
        help_timeout: ctx.config().programs.help_timeout.as_std(),
    };
    let env = setup.env(ctx);
    let index = setup.index(ctx, &env);
    let filter = source_filter(&index, args, DESCRIBE)?;
    let mut records = Vec::new();
    let mut unknown = Vec::new();
    for name in &names {
        ctx.checkpoint()?;
        match resolve_name(&index, name, &filter) {
            Some(resolved) => records.push(describe_one(&index, &env, &resolved, &options, ctx)),
            None => unknown.push(name.clone()),
        }
    }
    if records.is_empty() {
        let name = unknown.first().cloned().unwrap_or_default();
        let similar = similar(&index, &name, SIMILAR_LIMIT);
        let error = Error::new(ErrorCode::NotFound, format!("`{DESCRIBE}`: no program, application or guide named `{name}`; check the name with program.search"))
            .with_found(name)
            .with_expected(serde_json::json!(similar));
        return Err(candidates(
            error,
            similar
                .iter()
                .map(|candidate| format!("{DESCRIBE} {}", quoted(candidate))),
        ));
    }
    for name in unknown {
        warn(
            ctx,
            format!("no program, application or guide named `{name}`"),
        );
    }
    Ok(records)
}

/// Guide text of a card or guide, read from its provider.
fn guide_body(
    index: &ProgramIndex,
    source: &SourceId,
    path: &str,
) -> Option<crate::model::GuideBody> {
    let provider = index.source(source)?;
    let want = DetailWant {
        guide: true,
        ..DetailWant::default()
    };
    provider
        .detail(index.env(), &EntryKey::new(source, path.to_owned()), &want)
        .ok()
        .flatten()
        .and_then(|detail| detail.guide)
}

fn man_line(index: &ProgramIndex, card: &Card) -> Option<String> {
    let want = DetailWant {
        man: true,
        ..DetailWant::default()
    };
    card.keys
        .iter()
        .filter(|key| key.source.as_str() == SourceId::MAN)
        .find_map(|key| {
            index
                .source(&key.source)?
                .detail(index.env(), key, &want)
                .ok()
                .flatten()?
                .man
        })
}

fn etag(parts: &[&str]) -> String {
    let digest = Sha256::digest(parts.join("\n").as_bytes());
    hex::encode(digest).chars().take(ETAG_CHARS).collect()
}

fn describe_one(
    index: &ProgramIndex,
    env: &HostEnv,
    resolved: &Resolved,
    options: &DescribeOptions,
    ctx: &ExecCtx,
) -> Record {
    let languages = env.languages();
    let (mut record, body, card) = match resolved.target {
        Target::Card(position) => {
            let card = index.card(position);
            let body = card
                .and_then(|card| card.guide.as_ref())
                .and_then(|(source, guide)| guide_body(index, source, &guide.path));
            (hit_record(index, &resolved.target, languages), body, card)
        }
        Target::Guide(position) => {
            let body = index
                .guides()
                .get(position)
                .and_then(|guide| guide_body(index, &guide.source, &guide.guide.path));
            (hit_record(index, &resolved.target, languages), body, None)
        }
    };
    let description = card
        .and_then(|card| {
            card.description
                .as_ref()
                .or(card.package_description.as_ref())
        })
        .map(|text| pick(text, languages));
    let guide_text = body
        .as_ref()
        .map(|body| body.text.clone())
        .unwrap_or_default();
    let tag = etag(&[
        description.as_deref().unwrap_or_default(),
        card.and_then(|card| card.path.as_deref())
            .unwrap_or_default(),
        card.and_then(|card| card.version.as_deref())
            .unwrap_or_default(),
        card.and_then(|card| card.verified.as_deref())
            .unwrap_or_default(),
        &guide_text,
    ]);
    record.set("etag", Value::Text(tag.clone()));
    if resolved.requested != record_name(&record) {
        record.set("requested", Value::Text(resolved.requested.clone()));
        if let Some(kind) = resolved.alias_kind {
            record.set("alias_kind", Value::Enum(kind.as_str().to_owned()));
        }
    }
    if options.unchanged_since.as_deref() == Some(tag.as_str()) {
        record.set("unchanged", Value::Bool(true));
        return record;
    }
    let mut sections: Vec<Section> = guide_sections(&guide_text);
    if let Some(card) = card {
        sections.extend(card_sections(card, languages));
        add_card_details(index, card, &mut record, languages, description.clone());
        if options.help {
            add_help(&mut record, card, env, options, ctx);
        }
    }
    if let Some(body) = &body {
        record.set("guide_sections", list(body.sections.clone()));
        if !body.files.is_empty() {
            record.set("guide_files", list(body.files.clone()));
        }
        record.set("guide_etag", Value::Text(etag(&[&body.text])));
    }
    add_fragments(&mut record, &sections, &guide_text, options, body.is_some());
    record
}

fn record_name(record: &Record) -> String {
    match record.get("name") {
        Some(Value::Text(name)) => name.clone(),
        _ => String::new(),
    }
}

fn add_card_details(
    index: &ProgramIndex,
    card: &Card,
    record: &mut Record,
    languages: &[String],
    description: Option<String>,
) {
    if let Some(description) = description {
        record.set("description", Value::Text(description));
    }
    if !card.when_to_use.is_empty() {
        record.set(
            "when_to_use",
            list(card.when_to_use.iter().map(|text| pick(text, languages))),
        );
    }
    if !card.not_for.is_empty() {
        let items = card
            .not_for
            .iter()
            .map(|item| {
                let mut fields = Fields::new();
                fields.insert("task".to_owned(), Value::Text(pick(&item.task, languages)));
                if let Some(instead) = &item.instead {
                    fields.insert("use".to_owned(), Value::Text(instead.clone()));
                }
                Value::Map(fields)
            })
            .collect();
        record.set("not_for", Value::List(items));
    }
    if let Some(example) = &card.example {
        record.set("example", Value::Text(example.clone()));
    }
    if let Some(pitfalls) = card
        .sections
        .iter()
        .find(|section| section.kind == SectionKind::Pitfalls)
    {
        record.set("pitfalls", Value::Text(pick(&pitfalls.text, languages)));
    }
    if let Some(man) = man_line(index, card) {
        record.set("man", Value::Text(man));
    }
}

fn add_help(
    record: &mut Record,
    card: &Card,
    env: &HostEnv,
    options: &DescribeOptions,
    ctx: &ExecCtx,
) {
    let result = match &card.path {
        Some(path) => read_help(ctx, env, Path::new(path), options.help_timeout),
        None => HelpResult {
            text: None,
            status: crate::help::HelpStatus::Refused,
            executed: None,
            reason: Some("the program is not installed on this computer".to_owned()),
            changed: Vec::new(),
        },
    };
    if let Some(text) = result.text {
        record.set("help", Value::Text(text));
    }
    record.set(
        "help_status",
        Value::Enum(result.status.as_str().to_owned()),
    );
    if let Some(executed) = result.executed {
        record.set("executed", list(executed));
    }
    if let Some(reason) = result.reason {
        warn(ctx, format!("help of `{}` not read: {reason}", card.name));
    }
    if !result.changed.is_empty() {
        warn(
            ctx,
            format!(
                "`{} --help` changed {} files in its working directory",
                card.name,
                result.changed.len()
            ),
        );
    }
}

fn add_fragments(
    record: &mut Record,
    sections: &[Section],
    guide_text: &str,
    options: &DescribeOptions,
    has_guide: bool,
) {
    let chosen: Option<Vec<&Section>> = if let Some(section) = &options.section {
        Some(find_section(sections, section).into_iter().collect())
    } else {
        options
            .query
            .as_ref()
            .map(|query| match_sections(sections, query))
    };
    match chosen {
        Some(chosen) => {
            record.set(
                "guide_sections",
                list(chosen.iter().map(|section| section.heading.clone())),
            );
            let text: Vec<&str> = chosen.iter().map(|section| section.body.trim()).collect();
            if !text.is_empty() {
                record.set("guide", Value::Text(text.join("\n\n")));
            }
        }
        None if options.include_guide && has_guide => {
            record.set("guide", Value::Text(guide_text.to_owned()));
        }
        None => {}
    }
}

/// Categories with visible cards and their counts, by count descending and
/// id ascending.
fn visible_categories(index: &ProgramIndex, filter: &SourceFilter) -> Vec<(String, usize)> {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for card in index
        .cards()
        .iter()
        .filter(|card| filter.admits(card, index))
    {
        *counts.entry(card.category.clone()).or_default() += 1;
    }
    let mut out: Vec<(String, usize)> = counts.into_iter().collect();
    out.sort_by(|left, right| right.1.cmp(&left.1).then(left.0.cmp(&right.0)));
    out
}

fn category_record(
    index: &ProgramIndex,
    id: &str,
    count: usize,
    parent: Option<&str>,
    languages: &[String],
    cards: &[&Card],
) -> Record {
    let category = index
        .categories()
        .iter()
        .find(|category| category.id == id && category.parent.as_deref() == parent)
        .or_else(|| index.categories().iter().find(|category| category.id == id));
    let subcategories: Vec<String> = if parent.is_none() {
        visible_subcategories(index, id, cards)
            .into_iter()
            .map(|(sub, _)| sub)
            .collect()
    } else {
        Vec::new()
    };
    let typed = ProgramCategory {
        name: id.to_owned(),
        count: count as u64,
        source: category
            .and_then(|category| category.sources.last().cloned())
            .unwrap_or_else(|| crate::index::TAXONOMY_SOURCE.to_owned()),
        title: category.map(|category| pick(&category.title, languages)),
        description: category
            .and_then(|category| category.description.as_ref().or(category.summary.as_ref()))
            .map(|text| pick(text, languages)),
        subcategories: (!subcategories.is_empty()).then(|| subcategories.clone()),
    };
    let mut record = Record::from(typed);
    record.set(
        "level",
        Value::Enum(
            if parent.is_some() {
                "subcategory"
            } else {
                "category"
            }
            .to_owned(),
        ),
    );
    if let Some(parent) = parent {
        record.set("parent", Value::Text(parent.to_owned()));
    }
    if let Some(summary) = category.and_then(|category| category.summary.as_ref()) {
        record.set("summary", Value::Text(pick(summary, languages)));
    }
    if let Some(category) = category {
        if !category.aliases.is_empty() {
            record.set("aliases", list(category.aliases.clone()));
        }
        record.set("sources", list(category.sources.clone()));
    }
    let mut by_source: BTreeMap<String, i64> = BTreeMap::new();
    for card in cards {
        for source in &card.sources {
            *by_source.entry(source.to_string()).or_default() += 1;
        }
    }
    let mut fields = Fields::new();
    for (source, count) in by_source {
        fields.insert(source, Value::Int(count));
    }
    record.set("counts_by_source", Value::Map(fields));
    record
}

/// Subcategories of a category that hold visible cards, in catalog order.
fn visible_subcategories(
    index: &ProgramIndex,
    category: &str,
    cards: &[&Card],
) -> Vec<(String, usize)> {
    let mut out = Vec::new();
    for sub in index
        .categories()
        .iter()
        .filter(|sub| sub.parent.as_deref() == Some(category))
    {
        let count = cards
            .iter()
            .filter(|card| card.subcategory.as_deref() == Some(sub.id.as_str()))
            .count();
        if count > 0 && !out.iter().any(|(id, _): &(String, usize)| *id == sub.id) {
            out.push((sub.id.clone(), count));
        }
    }
    out
}

fn sorted_cards(mut cards: Vec<&Card>) -> Vec<&Card> {
    cards.sort_by(|left, right| left.name.cmp(&right.name));
    cards
}

fn catalog(setup: &ProgramsSetup, args: &BoundArgs, ctx: &ExecCtx) -> Result<Vec<Record>> {
    let requested = args.text("category")?.filter(|text| !text.is_empty());
    let subcategory = args.text("subcategory")?.filter(|text| !text.is_empty());
    let query = args.text("query")?.filter(|text| !text.trim().is_empty());
    let Some(requested) = requested else {
        if subcategory.is_some() {
            return Err(Error::new(
                ErrorCode::Arg,
                "`program.catalog`: subcategory needs category; the list of categories is the result without arguments",
            ));
        }
        return list_categories(setup, args, ctx);
    };
    let env = setup.env(ctx);
    let languages = env.languages().to_vec();
    let index = setup.index(ctx, &env);
    let filter = source_filter(&index, args, CATALOG)?;
    let category = index
        .category(requested)
        .map(|found| found.parent.clone().unwrap_or_else(|| found.id.clone()))
        .filter(|id| {
            index
                .cards()
                .iter()
                .any(|card| card.category == *id && filter.admits(card, &index))
        })
        .ok_or_else(|| category_error(&index, CATALOG, requested))?;
    let cards: Vec<&Card> = index
        .cards()
        .iter()
        .filter(|card| card.category == category && filter.admits(card, &index))
        .collect();
    if let Some(query) = query {
        check_query(CATALOG, query)?;
        let limit = limit(args, CATALOG, CATEGORY_QUERY_LIMIT)?;
        let search_query = Query {
            text: query,
            category: Some(&category),
            kind: KindFilter::All,
            sources: &filter,
            installed: None,
            limit,
        };
        return Ok(search(&index, &search_query, setup.ranking())
            .into_iter()
            .map(|hit| {
                let mut record = hit_record(&index, &hit.target, &languages);
                record.set("score", Value::Number(f64::from(hit.points)));
                record
            })
            .collect());
    }
    let subcategories = visible_subcategories(&index, &category, &cards);
    if let Some(wanted) = subcategory {
        return subcategory_cards(
            &index,
            &category,
            &subcategories,
            &cards,
            wanted,
            &languages,
        );
    }
    if subcategories.is_empty() {
        return Ok(sorted_cards(cards)
            .into_iter()
            .map(|card| card_record(&index, card, &languages))
            .collect());
    }
    let mut records: Vec<Record> = subcategories
        .iter()
        .map(|(id, count)| {
            let members: Vec<&Card> = cards
                .iter()
                .copied()
                .filter(|card| card.subcategory.as_deref() == Some(id.as_str()))
                .collect();
            category_record(&index, id, *count, Some(&category), &languages, &members)
        })
        .collect();
    let rest: Vec<&Card> = cards
        .iter()
        .copied()
        .filter(|card| card.subcategory.is_none())
        .collect();
    if !rest.is_empty() {
        let mut record = category_record(
            &index,
            OTHER_SUBCATEGORY,
            rest.len(),
            Some(&category),
            &languages,
            &rest,
        );
        record.set("title", Value::Text(OTHER_TITLE.to_owned()));
        record.set(
            "source",
            Value::Text(crate::index::TAXONOMY_SOURCE.to_owned()),
        );
        records.push(record);
    }
    Ok(records)
}

fn subcategory_cards(
    index: &ProgramIndex,
    category: &str,
    subcategories: &[(String, usize)],
    cards: &[&Card],
    wanted: &str,
    languages: &[String],
) -> Result<Vec<Record>> {
    let key = crate::text::fold(wanted);
    let has_rest = cards.iter().any(|card| card.subcategory.is_none());
    if subcategories.is_empty() {
        return Err(Error::new(
            ErrorCode::Arg,
            format!(
                "`{CATALOG}`: category `{category}` has no subcategories; call it without subcategory"
            ),
        ));
    }
    let members: Vec<&Card> = if key == OTHER_SUBCATEGORY && has_rest {
        cards
            .iter()
            .copied()
            .filter(|card| card.subcategory.is_none())
            .collect()
    } else {
        let found = subcategories.iter().find(|(id, _)| {
            crate::text::fold(id) == key
                || index.categories().iter().any(|sub| {
                    sub.id == *id
                        && sub
                            .aliases
                            .iter()
                            .any(|alias| crate::text::fold(alias) == key)
                })
        });
        let Some((id, _)) = found else {
            let mut known: Vec<String> = subcategories.iter().map(|(id, _)| id.clone()).collect();
            if has_rest {
                known.push(OTHER_SUBCATEGORY.to_owned());
            }
            let error = Error::new(ErrorCode::Arg, format!("`{CATALOG}`: unknown subcategory `{wanted}` of `{category}`; use one of the listed subcategories"))
                .with_expected(serde_json::json!(known));
            return Err(candidates(
                error,
                known.iter().map(|sub| {
                    format!(
                        "{CATALOG} {} subcategory: {}",
                        quoted(category),
                        quoted(sub)
                    )
                }),
            ));
        };
        cards
            .iter()
            .copied()
            .filter(|card| card.subcategory.as_deref() == Some(id.as_str()))
            .collect()
    };
    Ok(sorted_cards(members)
        .into_iter()
        .map(|card| card_record(index, card, languages))
        .collect())
}

fn list_categories(setup: &ProgramsSetup, args: &BoundArgs, ctx: &ExecCtx) -> Result<Vec<Record>> {
    let env = setup.env(ctx);
    let languages = env.languages().to_vec();
    let index = setup.index(ctx, &env);
    let filter = source_filter(&index, args, CATALOG)?;
    if filter.kinds.contains(&SourceKind::Catalog)
        && !index
            .sources()
            .iter()
            .any(|source| source.kind() == SourceKind::Catalog)
    {
        warn(
            ctx,
            "no_catalogs: no program catalog is configured".to_owned(),
        );
    }
    Ok(visible_categories(&index, &filter)
        .into_iter()
        .map(|(id, count)| {
            let members: Vec<&Card> = index
                .cards()
                .iter()
                .filter(|card| card.category == id && filter.admits(card, &index))
                .collect();
            category_record(&index, &id, count, None, &languages, &members)
        })
        .collect())
}
