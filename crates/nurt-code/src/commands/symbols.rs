// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-code::commands::symbols
// Responsibility:   The `code.symbols` stage.
// Dependencies:     globset, nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! The `code.symbols` stage.
//!
//! Definitions are searched by name across files and directories. With
//! `source: auto` and a name, the language server answers first
//! ([`SemanticProvider::workspace_symbols`]); without a usable server, without
//! a name, or with `exported_only`, the symbol index of each directory
//! answers ([`crate::index`]). Names are ranked as described in
//! [`crate::symbols::matching`]; records are ordered by rank, then by path
//! and position. Without a name every symbol is listed in source order.
//!
//! Directory searches cover programming languages unless `language` names
//! a language; `.git`, `node_modules`, `__pycache__`, `venv` and `.venv`
//! are always skipped, as are files larger than `code.file_bytes_max`.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use globset::{Glob, GlobMatcher};
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::{AbsPath, FileKind, Follow};
use nurt_contracts::record::Symbol;
use nurt_contracts::report::{Notable, NotableEntry};
use nurt_contracts::services::{
    ExcludePreset, LanguageId, ReferenceRole, SpanClass, SymbolInfo, SymbolKind,
};
use nurt_contracts::stage::{
    BoundArgs, ExecCtx, InputKinds, ReadStage, RecordStream, StageDescriptor, StageImpl,
    StageInput, StageOutput,
};
use nurt_contracts::value::{Record, Value};

use crate::commands::definition::shared::{self, Source};
use crate::index::{SourceFilter, index_tree, read_source};
use crate::local::{analyze, is_test_path};
use crate::registry::{Registry, unsupported};
use crate::semantic::{DocumentPosition, SemanticProvider, SemanticScope, SemanticSymbol};
use crate::symbols::matching::{MatchMode, NameMatcher};
use crate::symbols::records::{
    FileFacts, RecordFields, semantic_record, service_kind, symbol_record,
};
use crate::symbols::{extract, parse_kind};

const COMMAND: &str = "code.symbols";

/// Implementation of `code.symbols`.
///
/// ```
/// use nurt_code::commands::symbols::CodeSymbols;
/// use nurt_contracts::stage::StageImpl;
///
/// let stage = CodeSymbols::new(None);
/// let descriptor = stage.descriptor();
/// assert_eq!(descriptor.command, "code.symbols");
/// assert!(descriptor.source);
/// ```
pub struct CodeSymbols {
    semantic: Option<Arc<dyn SemanticProvider>>,
    registry: &'static Registry,
}

impl CodeSymbols {
    /// Stage that asks `semantic` first; without a provider every answer
    /// comes from syntax analysis.
    #[must_use]
    pub fn new(semantic: Option<Arc<dyn SemanticProvider>>) -> Self {
        Self {
            semantic,
            registry: Registry::builtin(),
        }
    }
}

/// Parameters of one execution.
struct Options {
    matchers: Vec<NameMatcher>,
    roots: Vec<PathBuf>,
    glob: Option<GlobMatcher>,
    filter: SourceFilter,
    skip_tests: bool,
    kind: Option<SymbolKind>,
    language: Option<LanguageId>,
    source: Source,
    with_signature: bool,
    with_references: bool,
    exported_only: bool,
}

/// Symbol found, before conversion to a record.
struct Found {
    rank: u8,
    record: Symbol,
}

/// File searched by syntax analysis, for reference counts.
struct Searched {
    path: PathBuf,
    language: LanguageId,
}

impl Options {
    fn parse(args: &BoundArgs, ctx: &ExecCtx, registry: &Registry) -> Result<Self> {
        let mode = match shared::word_arg(args, "match")? {
            Some(text) => text.parse::<MatchMode>()?,
            None => MatchMode::Fuzzy,
        };
        let (filter, skip_tests) = filter_args(args)?;
        Ok(Self {
            matchers: matchers(args.value("name"), mode)?,
            roots: roots(args, ctx)?,
            glob: glob_arg(args)?,
            filter,
            skip_tests,
            kind: kind_arg(args)?,
            language: language_arg(args, registry)?,
            source: Source::from_args(args)?,
            with_signature: args.bool("with_signature")?.unwrap_or(false),
            with_references: args.bool("with_references")?.unwrap_or(false),
            exported_only: args.bool("exported_only")?.unwrap_or(false),
        })
    }

    fn fields(&self) -> RecordFields {
        RecordFields {
            signature: self.with_signature,
            references: None,
        }
    }

    /// Best rank of `name` among the queries; 1 when there is no query.
    fn rank(&self, name: &str) -> u8 {
        if self.matchers.is_empty() {
            return 1;
        }
        self.matchers
            .iter()
            .map(|matcher| matcher.rank(name))
            .max()
            .unwrap_or(0)
    }

    fn keeps(&self, info: &SymbolInfo) -> bool {
        self.kind.is_none_or(|kind| kind == info.kind)
            && (!self.exported_only || info.exported == Some(true))
    }

    fn glob_matches(&self, root: &Path, path: &Path) -> bool {
        self.glob.as_ref().is_none_or(|glob| {
            let relative = path.strip_prefix(root).unwrap_or(path);
            glob.is_match(relative.to_string_lossy().replace('\\', "/"))
        })
    }
}

fn matchers(value: Option<&Value>, mode: MatchMode) -> Result<Vec<NameMatcher>> {
    match value {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Text(text)) if text.trim().is_empty() => Ok(Vec::new()),
        Some(Value::Text(text)) => Ok(vec![NameMatcher::new(text.trim(), mode)?]),
        Some(Value::Regex(regex)) => Ok(vec![NameMatcher::from_regex(regex)?]),
        Some(Value::List(items)) => items
            .iter()
            .map(|item| match item {
                Value::Text(text) => NameMatcher::new(text.trim(), mode),
                Value::Regex(regex) => NameMatcher::from_regex(regex),
                _ => Err(type_error(
                    "name",
                    "text, a regular expression or a list of text",
                )),
            })
            .collect(),
        Some(_) => Err(type_error(
            "name",
            "text, a regular expression or a list of text",
        )),
    }
}

fn type_error(name: &str, expected: &str) -> Error {
    Error::new(
        ErrorCode::Type,
        format!("`{COMMAND}` expects {expected} for `{name}`"),
    )
}

fn roots(args: &BoundArgs, ctx: &ExecCtx) -> Result<Vec<PathBuf>> {
    let given = shared::paths_arg(args, "path")?;
    if given.is_empty() {
        let base = ctx.options().base.clone().ok_or_else(|| {
            Error::new(
                ErrorCode::MissingParam,
                format!("`{COMMAND}` needs `path` when the request has no base directory"),
            )
        })?;
        return Ok(vec![PathBuf::from(base)]);
    }
    given
        .iter()
        .map(|text| shared::resolve_path(ctx, text))
        .collect()
}

fn glob_arg(args: &BoundArgs) -> Result<Option<GlobMatcher>> {
    let text = match args.value("glob") {
        None | Some(Value::Null) => return Ok(None),
        Some(Value::Glob(text) | Value::Text(text)) => text.clone(),
        Some(_) => return Err(type_error("glob", "a glob")),
    };
    if text == "**" {
        return Ok(None);
    }
    Glob::new(&text)
        .map(|glob| Some(glob.compile_matcher()))
        .map_err(|error| {
            Error::new(
                ErrorCode::InvalidValue,
                format!("invalid glob `{text}`: {error}"),
            )
        })
}

fn texts(args: &BoundArgs, name: &str) -> Result<Vec<String>> {
    let items = match args.value(name) {
        None | Some(Value::Null) => return Ok(Vec::new()),
        Some(Value::List(items)) => items.clone(),
        Some(other) => vec![other.clone()],
    };
    items
        .into_iter()
        .map(|item| match item {
            Value::Text(text) | Value::Glob(text) | Value::Enum(text) => Ok(text),
            _ => Err(type_error(name, "a list of text")),
        })
        .collect()
}

fn filter_args(args: &BoundArgs) -> Result<(SourceFilter, bool)> {
    let mut presets = Vec::new();
    let mut skip_tests = false;
    for name in texts(args, "exclude_preset")? {
        let preset = match name.as_str() {
            "vcs" => ExcludePreset::Vcs,
            "deps" => ExcludePreset::Deps,
            "build" => ExcludePreset::Build,
            "cache" => ExcludePreset::Cache,
            "tests" => {
                skip_tests = true;
                continue;
            }
            other => {
                return Err(Error::new(
                    ErrorCode::InvalidValue,
                    format!(
                        "unknown exclude_preset `{other}`; use vcs, deps, build, cache or tests"
                    ),
                ));
            }
        };
        presets.push(preset);
    }
    let filter = SourceFilter {
        globs: Vec::new(),
        exclude: texts(args, "exclude")?,
        presets,
    };
    Ok((filter, skip_tests))
}

fn kind_arg(args: &BoundArgs) -> Result<Option<SymbolKind>> {
    match shared::word_arg(args, "kind")?.as_deref() {
        None | Some("any") => Ok(None),
        Some(text) => parse_kind(text).map(Some).ok_or_else(|| {
            Error::new(
                ErrorCode::InvalidValue,
                format!("unknown symbol kind `{text}`; use a kind listed by the catalog or any"),
            )
        }),
    }
}

fn language_arg(args: &BoundArgs, registry: &Registry) -> Result<Option<LanguageId>> {
    match shared::word_arg(args, "language")?.as_deref() {
        None | Some("auto") => Ok(None),
        Some(name) => registry.resolve_name(name).map(Some).ok_or_else(|| {
            Error::new(
                ErrorCode::UnsupportedLanguage,
                format!("unknown language `{name}`; list the languages with `code.languages`"),
            )
        }),
    }
}

impl CodeSymbols {
    fn run(&self, ctx: &ExecCtx, options: &Options) -> Result<Vec<Record>> {
        let served = self.server_symbols(ctx, options)?;
        let (mut found, searched) = match served {
            Some(found) => (found, Vec::new()),
            None => self.syntax_symbols(ctx, options)?,
        };
        found.sort_by(|left, right| {
            right
                .rank
                .cmp(&left.rank)
                .then_with(|| left.record.path.as_str().cmp(right.record.path.as_str()))
                .then_with(|| {
                    (left.record.line, left.record.column)
                        .cmp(&(right.record.line, right.record.column))
                })
        });
        if options.with_references {
            self.count_references(ctx, &mut found, &searched)?;
        }
        Ok(found.into_iter().map(|item| item.record.into()).collect())
    }

    /// Symbols from the language server; `None` when syntax analysis
    /// answers.
    fn server_symbols(&self, ctx: &ExecCtx, options: &Options) -> Result<Option<Vec<Found>>> {
        if options.exported_only {
            if options.source == Source::Lsp {
                return Err(Error::new(
                    ErrorCode::ParamConflict,
                    "language servers do not report visibility; use exported_only with source: auto or syntax",
                ));
            }
            return Ok(None);
        }
        if options.matchers.is_empty() && options.source == Source::Auto {
            return Ok(None);
        }
        let queries: Vec<String> = match options.matchers.as_slice() {
            [] => vec![String::new()],
            matchers => matchers.iter().map(|m| m.query().to_owned()).collect(),
        };
        let mut found = Vec::new();
        for root in &options.roots {
            let scope = SemanticScope::new(project_dir(ctx, root));
            for query in &queries {
                let answer =
                    shared::ask(self.semantic.as_deref(), options.source, ctx, |provider| {
                        provider.workspace_symbols(ctx, &scope, query)
                    })?;
                let Some(symbols) = answer else {
                    return Ok(None);
                };
                found.extend(symbols);
            }
        }
        Ok(Some(self.server_records(ctx, options, found)))
    }

    fn server_records(
        &self,
        ctx: &ExecCtx,
        options: &Options,
        symbols: Vec<SemanticSymbol>,
    ) -> Vec<Found> {
        let mut etags: HashMap<PathBuf, String> = HashMap::new();
        let mut seen = BTreeSet::new();
        let mut found = Vec::new();
        for symbol in symbols {
            let path = symbol.location.path.clone();
            let Some(root) = options.roots.iter().find(|root| path.starts_with(root)) else {
                continue;
            };
            let language = self.registry.detect_by_name(&path);
            let kind_ok = options
                .kind
                .is_none_or(|kind| service_kind(symbol.kind) == Some(kind));
            let language_ok = options.language.is_none() || options.language == language;
            let rank = options.rank(&symbol.name);
            if !kind_ok || !language_ok || rank == 0 || !options.glob_matches(root, &path) {
                continue;
            }
            let position = (
                path.clone(),
                symbol.name_position.line,
                symbol.name_position.column,
            );
            if !seen.insert(position) {
                continue;
            }
            let etag = etags
                .entry(path.clone())
                .or_insert_with(|| {
                    read_source(ctx, &path)
                        .map(|source| source.etag.to_string())
                        .unwrap_or_default()
                })
                .clone();
            let language = language.map(|id| id.to_string()).unwrap_or_default();
            found.push(Found {
                rank,
                record: semantic_record(&symbol, &language, &etag, options.fields()),
            });
        }
        found
    }

    fn syntax_symbols(
        &self,
        ctx: &ExecCtx,
        options: &Options,
    ) -> Result<(Vec<Found>, Vec<Searched>)> {
        let mut found = Vec::new();
        let mut searched = Vec::new();
        for root in &options.roots {
            if is_directory(ctx, root)? {
                self.search_directory(ctx, options, root, &mut found, &mut searched)?;
            } else {
                self.search_file(ctx, options, root, &mut found, &mut searched)?;
            }
        }
        Ok((found, searched))
    }

    fn search_file(
        &self,
        ctx: &ExecCtx,
        options: &Options,
        path: &Path,
        found: &mut Vec<Found>,
        searched: &mut Vec<Searched>,
    ) -> Result<()> {
        let source = read_source(ctx, path)?;
        let language = match &options.language {
            Some(language) => language.clone(),
            None => self
                .registry
                .detect(Some(path), &source.bytes)
                .ok_or_else(|| {
                    Error::new(
                        ErrorCode::UnsupportedLanguage,
                        format!(
                            "the language of {} is not known; give it with `language`",
                            path.display()
                        ),
                    )
                })?,
        };
        if self.registry.spec(&language).is_none() {
            return Err(unsupported(&language));
        }
        let parsed = self.registry.parse(&language, &source.bytes)?;
        let etag = source.etag.to_string();
        let facts = FileFacts {
            path,
            language: &language,
            etag: &etag,
        };
        self.collect(options, &facts, &extract(&parsed), found);
        searched.push(Searched {
            path: path.to_path_buf(),
            language,
        });
        Ok(())
    }

    fn search_directory(
        &self,
        ctx: &ExecCtx,
        options: &Options,
        root: &Path,
        found: &mut Vec<Found>,
        searched: &mut Vec<Searched>,
    ) -> Result<()> {
        let registry = self.registry;
        let accept = |path: &Path| self.accepts(options, root, path);
        let (index, stats) = index_tree(ctx, registry, root, &options.filter, &accept)?;
        if stats.truncated {
            let mut note = NotableEntry::new(Notable::SourceUnavailable);
            note.path = Some(root.to_string_lossy().into_owned());
            note.detail = Some(format!(
                "the symbol index stopped at code.index_max_files ({}) files; narrow `path` or `glob`, or raise the limit",
                ctx.config().code.index_max_files
            ));
            ctx.note(note);
        }
        for file in index.files() {
            let Some(language) = &file.language else {
                continue;
            };
            if !self.language_wanted(options, language) {
                continue;
            }
            let facts = FileFacts {
                path: &file.path,
                language,
                etag: &file.etag,
            };
            self.collect(options, &facts, &file.symbols, found);
            searched.push(Searched {
                path: file.path.clone(),
                language: language.clone(),
            });
        }
        Ok(())
    }

    /// Files listed for a directory search: matching the glob, not tests
    /// when excluded, and of a wanted language by name, or without an
    /// extension (scripts identified by their shebang line).
    fn accepts(&self, options: &Options, root: &Path, path: &Path) -> bool {
        if !options.glob_matches(root, path) {
            return false;
        }
        let language = self.registry.detect_by_name(path);
        if options.skip_tests && is_test_path(path, language.as_ref()) {
            return false;
        }
        match language {
            Some(language) => self.language_wanted(options, &language),
            None => path.extension().is_none(),
        }
    }

    fn language_wanted(&self, options: &Options, language: &LanguageId) -> bool {
        match &options.language {
            Some(wanted) => wanted == language,
            None => self
                .registry
                .spec(language)
                .is_some_and(|spec| spec.programming),
        }
    }

    fn collect(
        &self,
        options: &Options,
        facts: &FileFacts<'_>,
        symbols: &[SymbolInfo],
        found: &mut Vec<Found>,
    ) {
        for info in symbols {
            if !options.keeps(info) {
                continue;
            }
            let rank = options.rank(&info.name);
            if rank == 0 {
                continue;
            }
            found.push(Found {
                rank,
                record: symbol_record(info, facts, options.fields()),
            });
        }
    }

    fn count_references(
        &self,
        ctx: &ExecCtx,
        found: &mut [Found],
        searched: &[Searched],
    ) -> Result<()> {
        if found
            .iter()
            .any(|item| item.record.source == nurt_contracts::record::SymbolSource::Lsp)
        {
            self.count_with_server(ctx, found);
            return Ok(());
        }
        let mut wanted: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for item in found.iter() {
            wanted
                .entry(item.record.language.clone())
                .or_default()
                .insert(item.record.name.clone());
        }
        let counts = count_occurrences(ctx, self.registry, searched, &wanted)?;
        for item in found.iter_mut() {
            let key = (item.record.language.clone(), item.record.name.clone());
            item.record.references = Some(counts.get(&key).copied().unwrap_or(0));
        }
        Ok(())
    }

    fn count_with_server(&self, ctx: &ExecCtx, found: &mut [Found]) {
        let Some(provider) = self.semantic.as_deref() else {
            return;
        };
        for item in found.iter_mut() {
            let path = PathBuf::from(item.record.path.as_str());
            let scope = SemanticScope::new(project_dir(ctx, &path));
            let at = DocumentPosition {
                path,
                position: nurt_contracts::services::TextPosition {
                    line: u32::try_from(item.record.line).unwrap_or(u32::MAX),
                    column: u32::try_from(item.record.column).unwrap_or(u32::MAX),
                },
            };
            item.record.references = provider
                .references(ctx, &scope, &at, false)
                .ok()
                .map(|locations| u64::try_from(locations.len()).unwrap_or(u64::MAX));
        }
    }
}

/// Occurrences in code, outside definitions, of the wanted names in the
/// searched files, by language and name.
fn count_occurrences(
    ctx: &ExecCtx,
    registry: &Registry,
    searched: &[Searched],
    wanted: &BTreeMap<String, BTreeSet<String>>,
) -> Result<HashMap<(String, String), u64>> {
    let mut counts: HashMap<(String, String), u64> = HashMap::new();
    for file in searched {
        ctx.checkpoint()?;
        let Some(names) = wanted.get(file.language.as_str()) else {
            continue;
        };
        let Ok(source) = read_source(ctx, &file.path) else {
            continue;
        };
        let text = String::from_utf8_lossy(&source.bytes);
        let present: Vec<&String> = names
            .iter()
            .filter(|name| text.contains(name.as_str()))
            .collect();
        if present.is_empty() {
            continue;
        }
        let Ok(analysis) = analyze(registry, &source.bytes, &file.language) else {
            continue;
        };
        for name in present {
            let uses = analysis
                .references(name)
                .iter()
                .filter(|item| {
                    item.class == SpanClass::Code && item.role != ReferenceRole::Definition
                })
                .count();
            *counts
                .entry((file.language.to_string(), name.clone()))
                .or_default() += u64::try_from(uses).unwrap_or(u64::MAX);
        }
    }
    Ok(counts)
}

fn is_directory(ctx: &ExecCtx, path: &Path) -> Result<bool> {
    let fs = ctx.require_fs()?;
    let absolute = AbsPath::new(path).map_err(Error::from)?;
    let meta = fs
        .metadata(&absolute, Follow::Follow)
        .map_err(Error::from)?;
    Ok(meta.kind == FileKind::Directory)
}

/// Directory a server is started for: the root itself, or the directory of
/// a file root.
fn project_dir(ctx: &ExecCtx, root: &Path) -> PathBuf {
    match is_directory(ctx, root) {
        Ok(true) => root.to_path_buf(),
        _ => root
            .parent()
            .map_or_else(|| root.to_path_buf(), Path::to_path_buf),
    }
}

impl StageImpl for CodeSymbols {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: COMMAND.to_owned(),
            source: true,
            input: InputKinds::None,
        }
    }

    fn as_read(&self) -> Option<&dyn ReadStage> {
        Some(self)
    }
}

impl ReadStage for CodeSymbols {
    fn execute<'a>(
        &'a self,
        args: &'a BoundArgs,
        _input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<StageOutput<'a>> {
        let options = Options::parse(args, ctx, self.registry)?;
        let records = self.run(ctx, &options)?;
        Ok(StageOutput::records(RecordStream::from_records(records)))
    }
}

#[cfg(test)]
mod tests {
    use nurt_contracts::value::Regex;

    use super::*;

    #[test]
    fn name_list_gives_one_matcher_per_name() {
        let list = Value::List(vec![
            Value::Text("parse".to_owned()),
            Value::Text("load".to_owned()),
        ]);
        let found = matchers(Some(&list), MatchMode::Prefix).unwrap();
        assert_eq!(found.len(), 2);
        assert_eq!(found[1].query(), "load");
        let regex = Value::Regex(Regex::new("^get", "").unwrap());
        assert_eq!(
            matchers(Some(&regex), MatchMode::Fuzzy).unwrap()[0].mode(),
            MatchMode::Regex
        );
        assert!(
            matchers(Some(&Value::Text("  ".to_owned())), MatchMode::Fuzzy)
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            matchers(Some(&Value::Int(1)), MatchMode::Fuzzy)
                .unwrap_err()
                .code(),
            ErrorCode::Type
        );
    }

    #[test]
    fn exclude_preset_tests_is_a_test_file_filter() {
        let args = BoundArgs::new(COMMAND).with(
            "exclude_preset",
            nurt_contracts::stage::ArgValue::Value(Value::List(vec![
                Value::Enum("tests".to_owned()),
                Value::Enum("build".to_owned()),
            ])),
            false,
        );
        let (filter, skip_tests) = filter_args(&args).unwrap();
        assert!(skip_tests);
        assert_eq!(filter.presets, [ExcludePreset::Build]);
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
