// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The `code.rename` stage.
//!
//! The plan step locates the symbol, asks the language server for the
//! workspace edit (`source: lsp`, or `auto` with a server) or replaces the
//! whole identifier in code by syntax analysis (`source: syntax`, or `auto`
//! without a server), restricts the edit to the scope, checks the new name
//! for collisions and computes the references the change leaves behind. The
//! preview lists one `rename_edit` record per edit and a `map` record with
//! the plan summary.
//!
//! The apply step changes every file in one transaction: content changes and
//! the resource operations of the workspace edit (create, rename, delete). A
//! file changed after the plan fails with `E_STALE` before anything is
//! written. After the change, syntax errors and language server errors of
//! every written file are compared with those before the change; a new error
//! fails the stage with `E_ROLLED_BACK`, and the executor restores all files.

use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeMap, BTreeSet};
use std::hash::{Hash, Hasher};
use std::io::Read;
use std::path::{Path as FsPath, PathBuf};
use std::sync::Arc;

use nurt_contracts::error::{Candidate, Error, ErrorCode, Result};
use nurt_contracts::fs::{AbsPath, Etag, Follow};
use nurt_contracts::journal::{
    Expect, Mutation, MutationTx, TargetAction, TargetIndex, TargetPath, TargetSpec,
};
use nurt_contracts::record::{
    Effect, EffectStatus, RenameEdit, RenameEditKind, Symbol, SymbolSource,
};
use nurt_contracts::report::{Change, ChangeAction, Notable, NotableEntry};
use nurt_contracts::services::{
    CodeService, IssueSeverity, LanguageId, SpanClass, TextPosition, WalkSpec,
};
use nurt_contracts::stage::{
    BoundArgs, CheckCtx, EffectOutcome, EffectPlan, EffectStage, ExecCtx, InputKinds, RecordStream,
    StageDescriptor, StageImpl, StageInput, StageOutput, StaticInfo,
};
use nurt_contracts::value::{Fields, Path, Record, RecordKind, Value};

use crate::semantic::{
    DocumentChange, DocumentPosition, ResourceChange, SemanticError, SemanticErrorKind,
    SemanticProvider, SemanticScope, TextChange,
};

/// Command name in the catalog.
const COMMAND: &str = "code.rename";
/// Largest number of files one rename may change.
const MAX_CHANGED_FILES: usize = 500;
/// Number of locations quoted in the detail of a notable effect.
const SAMPLE: usize = 5;
/// Directories never searched for occurrences.
const SKIPPED_DIRS: &[&str] = &[".git", "node_modules", "__pycache__", "venv", ".venv"];
/// Files whose presence marks a project root, besides `.git`.
const ROOT_MARKERS: &[&str] = &[
    ".git",
    "package.json",
    "pyproject.toml",
    "setup.py",
    "go.mod",
    "Cargo.toml",
    "pom.xml",
    "build.gradle",
    "composer.json",
    "Gemfile",
    "compile_commands.json",
];
/// Extensions of documentation files, for the completeness check.
const DOC_EXTENSIONS: &[&str] = &["md", "markdown", "rst", "txt", "adoc"];
/// Extensions of configuration files, for the completeness check.
const CONFIG_EXTENSIONS: &[&str] = &[
    "json",
    "jsonc",
    "yaml",
    "yml",
    "toml",
    "ini",
    "cfg",
    "conf",
    "xml",
    "properties",
    "env",
];

/// Implementation of `code.rename`.
///
/// ```
/// use nurt_code::commands::rename::CodeRename;
/// use nurt_contracts::stage::StageImpl;
///
/// let stage = CodeRename::new(None);
/// assert_eq!(stage.descriptor().command, "code.rename");
/// assert!(stage.as_effect().is_some());
/// ```
pub struct CodeRename {
    semantic: Option<Arc<dyn SemanticProvider>>,
}

impl CodeRename {
    /// Stage that asks `semantic` for rename edits; without a provider the
    /// rename uses syntax analysis, and `source: lsp` fails with
    /// `E_EXTERNAL_MISSING`.
    #[must_use]
    pub fn new(semantic: Option<Arc<dyn SemanticProvider>>) -> Self {
        Self { semantic }
    }
}

impl StageImpl for CodeRename {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: COMMAND.to_owned(),
            source: true,
            input: InputKinds::Only(vec![RecordKind::Symbol]),
        }
    }

    fn check(&self, args: &BoundArgs, _ctx: &CheckCtx<'_>) -> Result<StaticInfo> {
        if let Some(to) = args.text("to")? {
            check_new_name(to)?;
        }
        Ok(StaticInfo::default())
    }

    fn as_effect(&self) -> Option<&dyn EffectStage> {
        Some(self)
    }
}

impl EffectStage for CodeRename {
    fn plan<'a>(
        &'a self,
        args: &'a BoundArgs,
        input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<EffectPlan> {
        let options = Options::from_args(args)?;
        let requests = requests(args, input)?;
        let mut planner = Planner {
            ctx,
            semantic: self.semantic.as_deref(),
            options: &options,
            notes: Vec::new(),
        };
        let mut state = planner.plan(&requests)?;
        state.plan_notes = planner.notes;
        let mut plan = EffectPlan::new();
        plan.changes = state.changes();
        plan.bulk = state.targets.len() > 1;
        plan.preview = state.preview_records();
        plan.notable = state.plan_notes.clone();
        plan.notable.extend(state.notable.iter().cloned());
        Ok(plan.with_state(state))
    }

    fn apply<'a>(
        &'a self,
        _args: &'a BoundArgs,
        mut plan: EffectPlan,
        tx: &mut dyn MutationTx,
        ctx: &'a ExecCtx,
    ) -> Result<EffectOutcome<'a>> {
        let state = plan.take_state::<RenameState>().ok_or_else(|| {
            Error::new(
                ErrorCode::StageFailed,
                "the plan of `code.rename` carries no rename; compute the plan again",
            )
        })?;
        let applier = Applier {
            ctx,
            semantic: self.semantic.as_deref(),
            state: &state,
        };
        applier.apply(tx)
    }
}

/// Scope of a rename.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scope {
    Project,
    File,
}

/// Analysis requested by `source`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Source {
    Auto,
    Lsp,
    Syntax,
}

/// Parameters of one call.
#[derive(Debug, Clone)]
struct Options {
    to: String,
    scope: Scope,
    paths: Vec<PathBuf>,
    comments: bool,
    strings: bool,
    source: Source,
    check_references: bool,
    language: Option<LanguageId>,
    root: Option<PathBuf>,
}

impl Options {
    fn from_args(args: &BoundArgs) -> Result<Self> {
        let to = args.text("to")?.ok_or_else(|| {
            Error::new(
                ErrorCode::MissingParam,
                "`code.rename` needs the new name in `to`",
            )
        })?;
        check_new_name(to)?;
        let scope = match word(args.value("scope")) {
            Some("file") => Scope::File,
            _ => Scope::Project,
        };
        let source = match word(args.value("source")) {
            Some("lsp") => Source::Lsp,
            Some("syntax") => Source::Syntax,
            _ => Source::Auto,
        };
        let language = match word(args.value("language")) {
            None | Some("auto") => None,
            Some(name) => Some(LanguageId::new(name).map_err(|error| {
                Error::new(ErrorCode::InvalidValue, format!("language: {error}"))
            })?),
        };
        Ok(Self {
            to: to.to_owned(),
            scope,
            paths: path_list(args.value("paths"))?,
            comments: args.bool("include_comments")?.unwrap_or(false),
            strings: args.bool("include_strings")?.unwrap_or(false),
            source,
            check_references: args.bool("check_references")?.unwrap_or(true),
            language,
            root: word(args.value("root")).map(PathBuf::from),
        })
    }
}

/// Text of a text, enumeration or path value.
fn word(value: Option<&Value>) -> Option<&str> {
    match value? {
        Value::Text(text) | Value::Enum(text) | Value::Path(text) => Some(text.as_str()),
        _ => None,
    }
}

fn path_list(value: Option<&Value>) -> Result<Vec<PathBuf>> {
    let items: Vec<&Value> = match value {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::List(items)) => items.iter().collect(),
        Some(other) => vec![other],
    };
    items
        .into_iter()
        .map(|item| {
            word(Some(item))
                .map(PathBuf::from)
                .ok_or_else(|| Error::new(ErrorCode::Type, "`paths` expects a list of paths"))
        })
        .collect()
}

/// Checks that `name` is an identifier: a letter, `_` or `$`, followed by
/// letters, digits, `_` or `$`.
fn check_new_name(name: &str) -> Result<()> {
    let mut chars = name.chars();
    let valid = chars
        .next()
        .is_some_and(|first| first.is_alphabetic() || first == '_' || first == '$')
        && chars.all(is_ident_char);
    if valid {
        Ok(())
    } else {
        Err(Error::new(
            ErrorCode::InvalidValue,
            format!("`{name}` is not an identifier; give `to` a name of letters, digits and `_`"),
        ))
    }
}

fn is_ident_char(character: char) -> bool {
    character.is_alphanumeric() || character == '_' || character == '$'
}

/// Symbol to rename, as given by the source form or an input record.
#[derive(Debug, Clone)]
struct Request {
    path: PathBuf,
    line: Option<u32>,
    column: Option<u32>,
    symbol: Option<String>,
}

fn requests(args: &BoundArgs, input: StageInput<'_>) -> Result<Vec<Request>> {
    if input.is_none() {
        return Ok(vec![source_request(args)?]);
    }
    let mut requests = Vec::new();
    for record in input.into_records()? {
        let symbol = Symbol::try_from(record?)?;
        let name = match &symbol.container {
            Some(container) => format!("{container}.{}", symbol.name),
            None => symbol.name.clone(),
        };
        requests.push(Request {
            path: PathBuf::from(symbol.path.as_str()),
            line: u32::try_from(symbol.line).ok(),
            column: None,
            symbol: Some(name),
        });
    }
    Ok(requests)
}

fn source_request(args: &BoundArgs) -> Result<Request> {
    let path = word(args.value("path")).ok_or_else(|| {
        Error::new(
            ErrorCode::MissingParam,
            "`code.rename` needs `path` with `symbol` or `line`, or `symbol` records as input",
        )
    })?;
    let request = Request {
        path: PathBuf::from(path),
        line: position_arg(args, "line")?,
        column: position_arg(args, "column")?,
        symbol: args.text("symbol")?.map(str::to_owned),
    };
    if request.symbol.is_none() && request.line.is_none() {
        return Err(Error::new(
            ErrorCode::MissingParam,
            "`code.rename` needs `symbol` or `line` to find the symbol in `path`",
        ));
    }
    Ok(request)
}

fn position_arg(args: &BoundArgs, name: &str) -> Result<Option<u32>> {
    match args.int(name)? {
        None => Ok(None),
        Some(value) => u32::try_from(value)
            .ok()
            .filter(|value| *value >= 1)
            .map(Some)
            .ok_or_else(|| {
                Error::new(
                    ErrorCode::InvalidValue,
                    format!("`{name}` counts from 1; give a positive number"),
                )
            }),
    }
}

/// Symbol found in its file.
#[derive(Debug, Clone)]
struct Located {
    path: PathBuf,
    language: Option<LanguageId>,
    name: String,
    position: TextPosition,
}

/// Text file read for the plan.
#[derive(Debug, Clone)]
struct Loaded {
    text: String,
    etag: Etag,
}

/// One replacement in the content a file had before the change.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Edit {
    start: usize,
    end: usize,
    start_pos: TextPosition,
    end_pos: TextPosition,
    old_text: String,
    new_text: String,
    kind: RenameEditKind,
}

/// Where the content of a file before its edits comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Base {
    /// The file on disk at this path.
    Disk(PathBuf),
    /// A file created by the workspace edit.
    Empty,
}

/// Content of one path after the workspace edit.
#[derive(Debug, Clone)]
struct Pending {
    base: Base,
    changes: Vec<TextChange>,
}

/// One target of the transaction.
#[derive(Debug, Clone)]
struct Target {
    path: PathBuf,
    action: TargetKind,
    /// Entity tag of the file the target reads or replaces.
    etag: Option<(PathBuf, Etag)>,
    base: Base,
    edits: Vec<Edit>,
    language: Option<LanguageId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum TargetKind {
    Modify,
    Create,
    Delete,
    Rename { from: PathBuf, replace: bool },
}

/// Occurrence of the old name left after the change.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Leftover {
    path: PathBuf,
    line: u32,
    category: &'static str,
}

/// Rename plan carried from the plan step to the apply step.
#[derive(Debug, Clone)]
struct RenameState {
    id: String,
    from: String,
    to: String,
    source: SymbolSource,
    server: Option<String>,
    language: Option<LanguageId>,
    root: PathBuf,
    roots: Vec<PathBuf>,
    targets: Vec<Target>,
    alarms: Vec<String>,
    leftovers: Vec<Leftover>,
    check_references: bool,
    /// Effects of the completeness check.
    notable: Vec<NotableEntry>,
    /// Effects found while planning: fallback, edits outside the scope.
    plan_notes: Vec<NotableEntry>,
}

struct Planner<'a> {
    ctx: &'a ExecCtx,
    semantic: Option<&'a dyn SemanticProvider>,
    options: &'a Options,
    notes: Vec<NotableEntry>,
}

impl Planner<'_> {
    fn plan(&mut self, requests: &[Request]) -> Result<RenameState> {
        let mut located = Vec::new();
        for request in requests {
            self.ctx.checkpoint()?;
            located.push(self.locate(request)?);
        }
        let first = located.first().cloned().ok_or_else(|| {
            Error::new(
                ErrorCode::NoMatch,
                "`code.rename` received no symbol; give `path` and `symbol` or `line`",
            )
        })?;
        if first.name == self.options.to {
            return Err(Error::new(
                ErrorCode::InvalidValue,
                format!(
                    "the symbol is already named `{}`; give `to` another name",
                    first.name
                ),
            ));
        }
        let root = self.root_of(&first.path)?;
        let roots = self.roots(&root, &first.path);
        let mut source = SymbolSource::Syntax;
        let mut server = None;
        let mut pending = BTreeMap::new();
        let mut resources = Vec::new();
        for target in &located {
            match self.semantic_rename(target, &root)? {
                Some((name, changes)) => {
                    source = SymbolSource::Lsp;
                    server = Some(name);
                    split_changes(changes, &mut pending, &mut resources);
                }
                None => self.syntax_rename(target, &roots, &mut pending)?,
            }
        }
        let scope = ScopeFilter::new(self.options, &root, &first.path);
        let dropped = scope.restrict(&mut pending, &mut resources);
        self.note_dropped(&dropped);
        let mut targets = self.targets(pending, &resources, &first)?;
        if source == SymbolSource::Lsp && (self.options.comments || self.options.strings) {
            self.add_text_occurrences(&first, &roots, &scope, &mut targets)?;
        }
        if targets.len() > MAX_CHANGED_FILES {
            return Err(Error::new(
                ErrorCode::Limit,
                format!(
                    "the rename changes {} files, more than {MAX_CHANGED_FILES}; narrow it with `paths` or `scope: file`",
                    targets.len()
                ),
            ));
        }
        self.check_collisions(&targets)?;
        if source == SymbolSource::Syntax {
            self.check_other_definitions(&first, &targets)?;
        }
        let alarms = unexpected_edits(&targets, &first.name, &self.options.to);
        let mut state = RenameState {
            id: String::new(),
            from: first.name.clone(),
            to: self.options.to.clone(),
            source,
            server,
            language: first.language.clone(),
            root,
            roots,
            targets,
            alarms,
            leftovers: Vec::new(),
            check_references: self.options.check_references,
            notable: Vec::new(),
            plan_notes: Vec::new(),
        };
        state.id = plan_id(&state);
        if self.options.check_references {
            let overrides = self.final_contents(&state)?;
            state.leftovers = leftovers(self.ctx, &state, &overrides)?;
            state.notable = leftover_notes(&state);
        }
        Ok(state)
    }

    fn locate(&self, request: &Request) -> Result<Located> {
        let loaded = read_text(self.ctx, &request.path)?;
        let code = self.ctx.code();
        let language = self.options.language.clone().or_else(|| {
            code.and_then(|code| code.language_of(Some(&request.path), loaded.text.as_bytes()))
        });
        let starts = line_starts(&loaded.text);
        let offset = match &request.symbol {
            Some(symbol) => self.symbol_offset(request, symbol, &loaded.text, language.as_ref())?,
            None => position_offset(request, &loaded.text, &starts, code, language.as_ref())?,
        };
        let (start, end) = identifier_at(&loaded.text, offset).ok_or_else(|| {
            Error::new(
                ErrorCode::SymbolNotFound,
                format!(
                    "no identifier at {}:{}; give `line` and `column` of the name or `symbol`",
                    request.path.display(),
                    request.line.unwrap_or(1)
                ),
            )
        })?;
        Ok(Located {
            path: request.path.clone(),
            language,
            name: loaded.text[start..end].to_owned(),
            position: position_at(&loaded.text, &starts, start),
        })
    }

    fn symbol_offset(
        &self,
        request: &Request,
        symbol: &str,
        text: &str,
        language: Option<&LanguageId>,
    ) -> Result<usize> {
        let (code, language) = syntax_service(self.ctx, language, &request.path)?;
        let found = code.find_symbol(text.as_bytes(), language, symbol)?;
        let narrowed: Vec<_> = match request.line {
            Some(line) if found.len() > 1 => found
                .iter()
                .filter(|info| info.span.start.line == line || info.name_span.start.line == line)
                .cloned()
                .collect(),
            _ => found,
        };
        match narrowed.as_slice() {
            [] => Err(Error::new(
                ErrorCode::SymbolNotFound,
                format!(
                    "`{symbol}` is not defined in {}; check the name or give `line` and `column`",
                    request.path.display()
                ),
            )),
            [only] => usize::try_from(only.name_span.bytes.start).map_err(|_| limit_error()),
            several => Err(ambiguous(symbol, &request.path, several, &self.options.to)),
        }
    }

    fn root_of(&self, file: &FsPath) -> Result<PathBuf> {
        if let Some(root) = &self.options.root {
            return Ok(root.clone());
        }
        let fs = self.ctx.require_fs()?;
        let mut dir = file.parent();
        while let Some(current) = dir {
            let marked = ROOT_MARKERS.iter().any(|marker| {
                AbsPath::new(current.join(marker))
                    .is_ok_and(|path| fs.metadata(&path, Follow::Follow).is_ok())
            });
            if marked {
                return Ok(current.to_path_buf());
            }
            dir = current.parent();
        }
        Ok(file
            .parent()
            .map_or_else(|| file.to_path_buf(), FsPath::to_path_buf))
    }

    fn roots(&self, root: &FsPath, file: &FsPath) -> Vec<PathBuf> {
        match self.options.scope {
            Scope::File => vec![file.to_path_buf()],
            Scope::Project if self.options.paths.is_empty() => vec![root.to_path_buf()],
            Scope::Project => self.options.paths.clone(),
        }
    }

    /// Edits of the language server, or `None` when syntax analysis is
    /// used instead.
    fn semantic_rename(
        &mut self,
        target: &Located,
        root: &FsPath,
    ) -> Result<Option<(String, Vec<DocumentChange>)>> {
        if self.options.source == Source::Syntax {
            return Ok(None);
        }
        let Some(semantic) = self.semantic else {
            return match self.options.source {
                Source::Lsp => Err(SemanticError::new(
                    SemanticErrorKind::Unavailable,
                    "no language server is available for `code.rename`",
                )
                .with_fix("install the language server of the language or use `source: syntax`")
                .to_error()),
                _ => Ok(None),
            };
        };
        let scope = SemanticScope {
            root: root.to_path_buf(),
            language: target.language.clone(),
            timeout: None,
        };
        let at = DocumentPosition {
            path: target.path.clone(),
            position: target.position,
        };
        match semantic.rename(self.ctx, &scope, &at, &self.options.to) {
            Ok(change) if !change.changes.is_empty() => {
                Ok(Some((self.server_name(semantic, target), change.changes)))
            }
            Ok(_) => self.fallback(
                SemanticError::new(
                    SemanticErrorKind::Failed,
                    format!(
                        "the language server found nothing to rename at `{}`",
                        target.name
                    ),
                ),
                ErrorCode::SymbolNotFound,
            ),
            Err(error) => self.fallback(error, ErrorCode::ExternalFailed),
        }
    }

    fn fallback(
        &mut self,
        error: SemanticError,
        empty_code: ErrorCode,
    ) -> Result<Option<(String, Vec<DocumentChange>)>> {
        if self.options.source == Source::Lsp || !error.allows_fallback() {
            let failure = error.to_error();
            return Err(if error.kind == SemanticErrorKind::Failed {
                Error::new(empty_code, failure.message.clone())
            } else {
                failure
            });
        }
        let mut note = NotableEntry::new(Notable::SemanticFallback);
        note.detail = Some(error.to_string());
        self.notes.push(note);
        Ok(None)
    }

    fn server_name(&self, semantic: &dyn SemanticProvider, target: &Located) -> String {
        semantic
            .server_status(self.ctx, target.language.as_ref())
            .into_iter()
            .find_map(|status| status.server)
            .unwrap_or_else(|| "language server".to_owned())
    }

    fn syntax_rename(
        &self,
        target: &Located,
        roots: &[PathBuf],
        pending: &mut BTreeMap<PathBuf, Pending>,
    ) -> Result<()> {
        let (code, language) = syntax_service(self.ctx, target.language.as_ref(), &target.path)?;
        let classes = self.classes(true);
        for path in candidates(self.ctx, roots, &target.path)? {
            self.ctx.checkpoint()?;
            let Some(changes) = occurrence_changes(
                self.ctx,
                code,
                &path,
                language,
                &target.name,
                &self.options.to,
                &classes,
            )?
            else {
                continue;
            };
            if !changes.is_empty() {
                pending
                    .entry(path.clone())
                    .or_insert_with(|| Pending {
                        base: Base::Disk(path),
                        changes: Vec::new(),
                    })
                    .changes
                    .extend(changes);
            }
        }
        Ok(())
    }

    fn classes(&self, code: bool) -> Vec<SpanClass> {
        let mut classes = Vec::new();
        if code {
            classes.push(SpanClass::Code);
        }
        if self.options.comments {
            classes.push(SpanClass::Comment);
        }
        if self.options.strings {
            classes.push(SpanClass::String);
        }
        classes
    }

    /// Adds occurrences in comments and strings to the edits of a server.
    fn add_text_occurrences(
        &self,
        first: &Located,
        roots: &[PathBuf],
        scope: &ScopeFilter,
        targets: &mut Vec<Target>,
    ) -> Result<()> {
        let Ok((code, language)) = syntax_service(self.ctx, first.language.as_ref(), &first.path)
        else {
            return Ok(());
        };
        let classes = self.classes(false);
        for path in candidates(self.ctx, roots, &first.path)? {
            if !scope.allows(&path) {
                continue;
            }
            let Some(changes) = occurrence_changes(
                self.ctx,
                code,
                &path,
                language,
                &first.name,
                &self.options.to,
                &classes,
            )?
            else {
                continue;
            };
            if changes.is_empty() {
                continue;
            }
            let loaded = read_text(self.ctx, &path)?;
            let extra = edits_for(&loaded.text, &changes, &first.name, Some((code, language)));
            merge_into(targets, &path, loaded, extra, Some(language.clone()));
        }
        Ok(())
    }

    fn note_dropped(&mut self, dropped: &Dropped) {
        if !dropped.outside.is_empty() {
            let mut note = NotableEntry::new(Notable::OutsideProject);
            note.count = Some(count(dropped.outside.len()));
            note.detail = Some(format!(
                "edits outside the rename scope were not applied: {}; widen `paths` or `root` to include them",
                sample_paths(&dropped.outside)
            ));
            note.path = dropped
                .outside
                .first()
                .map(|path| path.display().to_string());
            self.notes.push(note);
        }
        if !dropped.other_files.is_empty() {
            let mut note = NotableEntry::new(Notable::RenameIncomplete);
            note.count = Some(count(dropped.other_files.len()));
            note.detail = Some(format!(
                "`scope: file` leaves references in other files unchanged: {}",
                sample_paths(&dropped.other_files)
            ));
            self.notes.push(note);
        }
    }

    fn targets(
        &self,
        pending: BTreeMap<PathBuf, Pending>,
        resources: &[ResourceChange],
        first: &Located,
    ) -> Result<Vec<Target>> {
        let mut builder = TargetBuilder {
            ctx: self.ctx,
            name: &first.name,
            targets: Vec::new(),
        };
        builder.resources(resources, &pending)?;
        for (path, pending) in pending {
            builder.content(path, pending)?;
        }
        Ok(builder.targets)
    }

    fn check_collisions(&self, targets: &[Target]) -> Result<()> {
        let to = &self.options.to;
        for target in targets.iter().filter(|target| !target.edits.is_empty()) {
            let Base::Disk(base) = &target.base else {
                continue;
            };
            let text = read_text(self.ctx, base)?.text;
            let starts = line_starts(&text);
            let hit = code_occurrences(self.ctx, target.language.as_ref(), &text, to)
                .into_iter()
                .next();
            if let Some(offset) = hit {
                let at = position_at(&text, &starts, offset);
                return Err(Error::new(
                    ErrorCode::RenameConflict,
                    format!(
                        "`{to}` is already used in {}:{}; choose another name or rename the existing `{to}` first",
                        base.display(),
                        at.line
                    ),
                )
                .with_found(format!("{}:{}:{}", base.display(), at.line, at.column)));
            }
        }
        Ok(())
    }

    /// Syntax analysis renames every occurrence of the name; when the
    /// changed files define the name more than once, the other definitions
    /// would be renamed with the symbol.
    fn check_other_definitions(&self, first: &Located, targets: &[Target]) -> Result<()> {
        let Some(code) = self.ctx.code() else {
            return Ok(());
        };
        let mut others = Vec::new();
        for target in targets {
            let (Base::Disk(base), Some(language)) = (&target.base, &target.language) else {
                continue;
            };
            let text = read_text(self.ctx, base)?.text;
            for info in code.find_symbol(text.as_bytes(), language, &first.name)? {
                others.push((base.clone(), info));
            }
        }
        if others.len() < 2 {
            return Ok(());
        }
        let mut error = Error::new(
            ErrorCode::SymbolAmbiguous,
            format!(
                "`{}` has {} definitions that syntax analysis would rename together; narrow the rename with `paths` or `scope: file`, or use `source: lsp`",
                first.name,
                others.len()
            ),
        );
        for (rank, (path, info)) in others.iter().take(SAMPLE).enumerate() {
            error = error.with_candidate(Candidate {
                rank: u32::try_from(rank + 1).unwrap_or(u32::MAX),
                reference: None,
                program: format!(
                    "code.rename \"{}\" path: \"{}\" line: {} column: {}",
                    self.options.to,
                    path.display(),
                    info.name_span.start.line,
                    info.name_span.start.column
                ),
                why: Some("another definition with the same name".to_owned()),
            });
        }
        Err(error)
    }

    /// Content of every changed file after the rename, by final path.
    fn final_contents(&self, state: &RenameState) -> Result<BTreeMap<PathBuf, Option<String>>> {
        let mut contents = BTreeMap::new();
        for target in &state.targets {
            match &target.action {
                TargetKind::Delete => {
                    contents.insert(target.path.clone(), None);
                }
                TargetKind::Rename { from, .. } => {
                    contents.insert(from.clone(), None);
                    let text = read_text(self.ctx, from).ok().map(|loaded| loaded.text);
                    contents.insert(target.path.clone(), text);
                }
                TargetKind::Modify | TargetKind::Create => {
                    let base = base_text(self.ctx, &target.base)?;
                    contents.insert(target.path.clone(), Some(apply_edits(&base, &target.edits)));
                }
            }
        }
        Ok(contents)
    }
}

/// Code service and language for syntax analysis of `path`.
fn syntax_service<'c>(
    ctx: &'c ExecCtx,
    language: Option<&'c LanguageId>,
    path: &FsPath,
) -> Result<(&'c dyn CodeService, &'c LanguageId)> {
    match (ctx.code(), language) {
        (Some(code), Some(language)) => Ok((code, language)),
        _ => Err(Error::new(
            ErrorCode::UnsupportedLanguage,
            format!(
                "no syntax analysis is available for {}; give `language` or use a language server",
                path.display()
            ),
        )),
    }
}

fn ambiguous(
    symbol: &str,
    path: &FsPath,
    found: &[nurt_contracts::services::SymbolInfo],
    to: &str,
) -> Error {
    let mut error = Error::new(
        ErrorCode::SymbolAmbiguous,
        format!(
            "`{symbol}` has {} definitions in {}; give `line` and `column` or a qualified name",
            found.len(),
            path.display()
        ),
    );
    for (rank, info) in found.iter().take(SAMPLE).enumerate() {
        error = error.with_candidate(Candidate {
            rank: u32::try_from(rank + 1).unwrap_or(u32::MAX),
            reference: None,
            program: format!(
                "code.rename \"{to}\" path: \"{}\" line: {} column: {}",
                path.display(),
                info.name_span.start.line,
                info.name_span.start.column
            ),
            why: info
                .container
                .as_ref()
                .map(|container| format!("in `{container}`")),
        });
    }
    error
}

fn limit_error() -> Error {
    Error::new(
        ErrorCode::Limit,
        "the file is too large to address on this platform",
    )
}

fn count(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

fn sample_paths(paths: &[PathBuf]) -> String {
    let mut text: Vec<String> = paths
        .iter()
        .take(SAMPLE)
        .map(|path| path.display().to_string())
        .collect();
    if paths.len() > SAMPLE {
        text.push(format!("and {} more", paths.len() - SAMPLE));
    }
    text.join(", ")
}

/// Splits the changes of a server into text changes by path, in order, and
/// resource operations, recording where the content of each path comes
/// from.
fn split_changes(
    changes: Vec<DocumentChange>,
    pending: &mut BTreeMap<PathBuf, Pending>,
    resources: &mut Vec<ResourceChange>,
) {
    for change in changes {
        match change {
            DocumentChange::Text(text) => {
                let path = text.range.path.clone();
                pending
                    .entry(path.clone())
                    .or_insert_with(|| Pending {
                        base: Base::Disk(path),
                        changes: Vec::new(),
                    })
                    .changes
                    .push(text);
            }
            DocumentChange::Resource(resource) => {
                move_pending(&resource, pending);
                resources.push(resource);
            }
        }
    }
}

/// Follows a resource operation with the pending content of the paths.
fn move_pending(resource: &ResourceChange, pending: &mut BTreeMap<PathBuf, Pending>) {
    match resource {
        ResourceChange::Create { path, .. } => {
            pending.insert(
                path.clone(),
                Pending {
                    base: Base::Empty,
                    changes: Vec::new(),
                },
            );
        }
        ResourceChange::Rename { from, to, .. } => {
            let moved = pending.remove(from).unwrap_or_else(|| Pending {
                base: Base::Disk(from.clone()),
                changes: Vec::new(),
            });
            pending.insert(to.clone(), moved);
        }
        ResourceChange::Delete { path, .. } => {
            pending.remove(path);
        }
    }
}

/// Edits dropped by the scope.
#[derive(Debug, Default)]
struct Dropped {
    outside: Vec<PathBuf>,
    other_files: Vec<PathBuf>,
}

struct ScopeFilter {
    file_only: Option<PathBuf>,
    roots: Vec<PathBuf>,
    root: PathBuf,
}

impl ScopeFilter {
    fn new(options: &Options, root: &FsPath, file: &FsPath) -> Self {
        Self {
            file_only: (options.scope == Scope::File).then(|| file.to_path_buf()),
            roots: if options.paths.is_empty() {
                vec![root.to_path_buf()]
            } else {
                options.paths.clone()
            },
            root: root.to_path_buf(),
        }
    }

    fn in_project(&self, path: &FsPath) -> bool {
        path.starts_with(&self.root) || self.roots.iter().any(|root| path.starts_with(root))
    }

    fn in_paths(&self, path: &FsPath) -> bool {
        self.roots.iter().any(|root| path.starts_with(root))
    }

    fn allows(&self, path: &FsPath) -> bool {
        match &self.file_only {
            Some(file) => path == file,
            None => self.in_paths(path),
        }
    }

    fn restrict(
        &self,
        pending: &mut BTreeMap<PathBuf, Pending>,
        resources: &mut Vec<ResourceChange>,
    ) -> Dropped {
        let mut dropped = Dropped::default();
        let mut sort = |path: &FsPath| {
            if self.file_only.is_some() && self.in_project(path) {
                dropped.other_files.push(path.to_path_buf());
            } else {
                dropped.outside.push(path.to_path_buf());
            }
        };
        pending.retain(|path, _| {
            let keep = self.allows(path);
            if !keep {
                sort(path);
            }
            keep
        });
        resources.retain(|resource| {
            let keep = resource_paths(resource)
                .iter()
                .all(|path| self.allows(path));
            if !keep {
                for path in resource_paths(resource) {
                    sort(path);
                }
            }
            keep
        });
        dropped.outside.sort();
        dropped.outside.dedup();
        dropped.other_files.sort();
        dropped.other_files.dedup();
        dropped
    }
}

fn resource_paths(resource: &ResourceChange) -> Vec<&FsPath> {
    match resource {
        ResourceChange::Create { path, .. } | ResourceChange::Delete { path, .. } => {
            vec![path.as_path()]
        }
        ResourceChange::Rename { from, to, .. } => vec![from.as_path(), to.as_path()],
    }
}

struct TargetBuilder<'a> {
    ctx: &'a ExecCtx,
    name: &'a str,
    targets: Vec<Target>,
}

impl TargetBuilder<'_> {
    /// Targets of the resource operations: deletions and renames of entries
    /// whose content does not change. A renamed file that is also edited
    /// becomes a deletion of the old path and a creation of the new one.
    fn resources(
        &mut self,
        resources: &[ResourceChange],
        pending: &BTreeMap<PathBuf, Pending>,
    ) -> Result<()> {
        for resource in resources {
            match resource {
                ResourceChange::Create { .. } => {}
                ResourceChange::Rename {
                    from,
                    to,
                    overwrite,
                    ..
                } => {
                    let edited = pending
                        .get(to)
                        .is_some_and(|pending| !pending.changes.is_empty());
                    let etag = Some((from.clone(), etag_of(self.ctx, from)?));
                    let action = if edited {
                        TargetKind::Delete
                    } else {
                        TargetKind::Rename {
                            from: from.clone(),
                            replace: *overwrite,
                        }
                    };
                    let path = if edited { from.clone() } else { to.clone() };
                    self.push(
                        path,
                        action,
                        etag,
                        Base::Disk(from.clone()),
                        Vec::new(),
                        None,
                    );
                }
                ResourceChange::Delete {
                    path,
                    ignore_if_not_exists,
                    ..
                } => match etag_of(self.ctx, path) {
                    Ok(etag) => self.push(
                        path.clone(),
                        TargetKind::Delete,
                        Some((path.clone(), etag)),
                        Base::Disk(path.clone()),
                        Vec::new(),
                        None,
                    ),
                    Err(_) if *ignore_if_not_exists => {}
                    Err(error) => return Err(error),
                },
            }
        }
        Ok(())
    }

    fn content(&mut self, path: PathBuf, pending: Pending) -> Result<()> {
        let moved = pending.base != Base::Disk(path.clone());
        if !moved && pending.changes.is_empty() {
            return Ok(());
        }
        if matches!(&pending.base, Base::Disk(_)) && pending.changes.is_empty() {
            return Ok(());
        }
        let text = base_text(self.ctx, &pending.base)?;
        let language = self
            .ctx
            .code()
            .and_then(|code| code.language_of(Some(&path), text.as_bytes()));
        let service = self.ctx.code().zip(language.as_ref());
        let edits = edits_for(&text, &pending.changes, self.name, service);
        let (action, etag) = match etag_of(self.ctx, &path) {
            Ok(etag) => (TargetKind::Modify, Some((path.clone(), etag))),
            Err(_) => (TargetKind::Create, None),
        };
        self.push(path, action, etag, pending.base, edits, language);
        Ok(())
    }

    fn push(
        &mut self,
        path: PathBuf,
        action: TargetKind,
        etag: Option<(PathBuf, Etag)>,
        base: Base,
        edits: Vec<Edit>,
        language: Option<LanguageId>,
    ) {
        self.targets.push(Target {
            path,
            action,
            etag,
            base,
            edits,
            language,
        });
    }
}

/// Adds edits to the target of `path`, skipping edits that overlap edits
/// already planned.
fn merge_into(
    targets: &mut Vec<Target>,
    path: &FsPath,
    loaded: Loaded,
    extra: Vec<Edit>,
    language: Option<LanguageId>,
) {
    if let Some(target) = targets
        .iter_mut()
        .find(|target| target.path == path && target.base == Base::Disk(path.to_path_buf()))
    {
        for edit in extra {
            let overlaps = target
                .edits
                .iter()
                .any(|known| edit.start < known.end && known.start < edit.end);
            if !overlaps {
                target.edits.push(edit);
            }
        }
        target.edits.sort_by_key(|edit| edit.start);
        return;
    }
    targets.push(Target {
        path: path.to_path_buf(),
        action: TargetKind::Modify,
        etag: Some((path.to_path_buf(), loaded.etag)),
        base: Base::Disk(path.to_path_buf()),
        edits: extra,
        language,
    });
}

/// Replacements of the occurrences of `name` in the listed classes, or
/// `None` when the file is not text of the language family.
fn occurrence_changes(
    ctx: &ExecCtx,
    code: &dyn CodeService,
    path: &FsPath,
    language: &LanguageId,
    name: &str,
    to: &str,
    classes: &[SpanClass],
) -> Result<Option<Vec<TextChange>>> {
    let Ok(loaded) = read_text(ctx, path) else {
        return Ok(None);
    };
    if !loaded.text.contains(name) {
        return Ok(None);
    }
    let detected = code.language_of(Some(path), loaded.text.as_bytes());
    if detected.as_ref().map(family) != Some(family(language)) {
        return Ok(None);
    }
    let language = detected.as_ref().unwrap_or(language);
    let changes = code
        .references(loaded.text.as_bytes(), language, name)?
        .into_iter()
        .filter(|reference| classes.contains(&reference.class))
        .map(|reference| TextChange {
            range: crate::semantic::Location {
                path: path.to_path_buf(),
                start: reference.span.start,
                end: reference.span.end,
            },
            new_text: to.to_owned(),
        })
        .collect();
    Ok(Some(changes))
}

/// Languages whose files refer to each other's symbols.
fn family(language: &LanguageId) -> &str {
    match language.as_str() {
        "typescript" | "tsx" | "javascript" | "jsx" => "javascript",
        "c" | "cpp" => "c",
        other => other,
    }
}

/// Converts text changes on `text` to byte edits with the class of the
/// replaced text.
fn edits_for(
    text: &str,
    changes: &[TextChange],
    name: &str,
    service: Option<(&dyn CodeService, &LanguageId)>,
) -> Vec<Edit> {
    let starts = line_starts(text);
    let spans = service
        .and_then(|(code, language)| code.classify_spans(text.as_bytes(), language).ok())
        .unwrap_or_default();
    let mut edits: Vec<Edit> = changes
        .iter()
        .map(|change| {
            let start = offset_at(text, &starts, change.range.start);
            let end = offset_at(text, &starts, change.range.end).max(start);
            let class = spans
                .iter()
                .find(|span| span.bytes.contains(count(start)))
                .map_or(SpanClass::Code, |span| span.class);
            Edit {
                start,
                end,
                start_pos: change.range.start,
                end_pos: change.range.end,
                old_text: text[start..end].to_owned(),
                new_text: change.new_text.clone(),
                kind: edit_kind(class, text, &starts, start, name),
            }
        })
        .collect();
    edits.sort_by_key(|edit| edit.start);
    edits.dedup_by(|later, earlier| later.start < earlier.end);
    edits
}

fn edit_kind(
    class: SpanClass,
    text: &str,
    starts: &[usize],
    offset: usize,
    name: &str,
) -> RenameEditKind {
    match class {
        SpanClass::Comment => RenameEditKind::Comment,
        SpanClass::String => RenameEditKind::String,
        _ => {
            let line = line_text(text, starts, offset).trim_start();
            let import = ["import ", "from ", "use ", "#include", "require"]
                .iter()
                .any(|keyword| line.starts_with(keyword))
                && line.contains(name);
            if import {
                RenameEditKind::Import
            } else {
                RenameEditKind::Code
            }
        }
    }
}

/// Server edits that do more than replace the old name by the new one.
fn unexpected_edits(targets: &[Target], from: &str, to: &str) -> Vec<String> {
    let mut alarms = Vec::new();
    for target in targets {
        for edit in &target.edits {
            if edit.old_text != from || edit.new_text != to {
                alarms.push(format!(
                    "{}:{} replaces `{}` with `{}`",
                    target.path.display(),
                    edit.start_pos.line,
                    edit.old_text,
                    edit.new_text
                ));
            }
        }
    }
    alarms
}

/// Byte offsets of the occurrences of the identifier `name` in code.
fn code_occurrences(
    ctx: &ExecCtx,
    language: Option<&LanguageId>,
    text: &str,
    name: &str,
) -> Vec<usize> {
    if let (Some(code), Some(language)) = (ctx.code(), language)
        && let Ok(found) = code.references(text.as_bytes(), language, name)
    {
        return found
            .into_iter()
            .filter(|reference| reference.class == SpanClass::Code)
            .filter_map(|reference| usize::try_from(reference.span.bytes.start).ok())
            .collect();
    }
    whole_words(text, name)
}

/// Byte offsets of the whole-word occurrences of `name`.
fn whole_words(text: &str, name: &str) -> Vec<usize> {
    if name.is_empty() {
        return Vec::new();
    }
    text.match_indices(name)
        .filter(|(start, _)| {
            let before = text[..*start].chars().next_back();
            let after = text[start + name.len()..].chars().next();
            !before.is_some_and(is_ident_char) && !after.is_some_and(is_ident_char)
        })
        .map(|(start, _)| start)
        .collect()
}

/// Files under the roots, sorted, with the source file first.
fn candidates(ctx: &ExecCtx, roots: &[PathBuf], file: &FsPath) -> Result<Vec<PathBuf>> {
    let limit = usize::try_from(ctx.config().code.index_max_files).unwrap_or(usize::MAX);
    let mut found = BTreeSet::new();
    for root in roots {
        if root == file {
            continue;
        }
        if let Some(files) = ctx.files() {
            for entry in files.walk(&WalkSpec::source_tree(root.clone()))? {
                found.insert(entry?.path);
                if found.len() >= limit {
                    break;
                }
            }
        } else {
            walk(ctx, root, &mut found, limit)?;
        }
    }
    found.remove(file);
    let mut list = vec![file.to_path_buf()];
    list.extend(found);
    Ok(list)
}

/// Lists files below `root` through the read-only file system.
fn walk(ctx: &ExecCtx, root: &FsPath, found: &mut BTreeSet<PathBuf>, limit: usize) -> Result<()> {
    let fs = ctx.require_fs()?;
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        ctx.checkpoint()?;
        let Ok(abs) = AbsPath::new(dir.clone()) else {
            continue;
        };
        let Ok(entries) = fs.read_dir(&abs) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.name.to_string_lossy();
            let path = dir.join(&*name);
            let Ok(abs) = AbsPath::new(path.clone()) else {
                continue;
            };
            match fs.metadata(&abs, Follow::NoFollow) {
                Ok(meta) if meta.kind == nurt_contracts::fs::FileKind::Directory => {
                    if !SKIPPED_DIRS.contains(&name.as_ref()) {
                        pending.push(path);
                    }
                }
                Ok(meta) if meta.kind == nurt_contracts::fs::FileKind::File => {
                    found.insert(path);
                    if found.len() >= limit {
                        return Ok(());
                    }
                }
                _ => {}
            }
        }
    }
    Ok(())
}

/// Reads a UTF-8 text file with its entity tag.
fn read_text(ctx: &ExecCtx, path: &FsPath) -> Result<Loaded> {
    let fs = ctx.require_fs()?;
    let abs = AbsPath::new(path.to_path_buf())?;
    let meta = fs.metadata(&abs, Follow::Follow)?;
    let max = ctx.config().code.file_bytes_max.bytes();
    if meta.size > max {
        return Err(Error::new(
            ErrorCode::Limit,
            format!(
                "{} is larger than `code.file_bytes_max` ({max} bytes); raise the limit to rename in it",
                path.display()
            ),
        ));
    }
    let mut bytes = Vec::new();
    fs.open_read(&abs)?
        .read_to_end(&mut bytes)
        .map_err(|error| {
            Error::new(
                ErrorCode::Io,
                format!("cannot read {}: {error}", path.display()),
            )
        })?;
    let text = String::from_utf8(bytes).map_err(|_| {
        Error::new(
            ErrorCode::Encoding,
            format!(
                "{} is not UTF-8 text; convert it before renaming",
                path.display()
            ),
        )
    })?;
    Ok(Loaded {
        text,
        etag: meta.etag(),
    })
}

fn etag_of(ctx: &ExecCtx, path: &FsPath) -> Result<Etag> {
    let fs = ctx.require_fs()?;
    Ok(fs.etag(&AbsPath::new(path.to_path_buf())?, Follow::Follow)?)
}

fn base_text(ctx: &ExecCtx, base: &Base) -> Result<String> {
    match base {
        Base::Disk(path) => Ok(read_text(ctx, path)?.text),
        Base::Empty => Ok(String::new()),
    }
}

fn apply_edits(text: &str, edits: &[Edit]) -> String {
    let mut result = String::with_capacity(text.len());
    let mut cursor = 0;
    for edit in edits {
        result.push_str(&text[cursor..edit.start]);
        result.push_str(&edit.new_text);
        cursor = edit.end;
    }
    result.push_str(&text[cursor..]);
    result
}

fn line_starts(text: &str) -> Vec<usize> {
    std::iter::once(0)
        .chain(text.match_indices('\n').map(|(index, _)| index + 1))
        .collect()
}

/// Byte offset of a one-based position counted in characters; positions
/// past the end of a line or of the text are clamped to it.
fn offset_at(text: &str, starts: &[usize], at: TextPosition) -> usize {
    let index = usize::try_from(at.line.saturating_sub(1)).unwrap_or(usize::MAX);
    let Some(&start) = starts.get(index) else {
        return text.len();
    };
    let line = line_text(text, starts, start);
    let column = usize::try_from(at.column.saturating_sub(1)).unwrap_or(usize::MAX);
    start
        + line
            .char_indices()
            .nth(column)
            .map_or(line.len(), |(offset, _)| offset)
}

fn position_at(text: &str, starts: &[usize], offset: usize) -> TextPosition {
    let index = starts
        .partition_point(|start| *start <= offset)
        .saturating_sub(1);
    let start = starts.get(index).copied().unwrap_or(0);
    let column = text
        .get(start..offset)
        .map_or(0, |prefix| prefix.chars().count());
    TextPosition {
        line: u32::try_from(index + 1).unwrap_or(u32::MAX),
        column: u32::try_from(column + 1).unwrap_or(u32::MAX),
    }
}

/// The line holding `offset`, without its line ending.
fn line_text<'t>(text: &'t str, starts: &[usize], offset: usize) -> &'t str {
    let index = starts
        .partition_point(|start| *start <= offset)
        .saturating_sub(1);
    let start = starts.get(index).copied().unwrap_or(0);
    let end = starts.get(index + 1).map_or(text.len(), |next| next - 1);
    text.get(start..end)
        .unwrap_or_default()
        .trim_end_matches('\r')
}

/// The identifier containing `offset` or ending at it.
fn identifier_at(text: &str, offset: usize) -> Option<(usize, usize)> {
    let offset = offset.min(text.len());
    let at = text[offset..].chars().next().filter(|c| is_ident_char(*c));
    let anchor = match at {
        Some(_) => offset,
        None => {
            let (index, previous) = text[..offset].char_indices().next_back()?;
            if !is_ident_char(previous) {
                return None;
            }
            index
        }
    };
    let start = text[..anchor]
        .char_indices()
        .rev()
        .take_while(|(_, c)| is_ident_char(*c))
        .last()
        .map_or(anchor, |(index, _)| index);
    let end = text[anchor..]
        .char_indices()
        .find(|(_, c)| !is_ident_char(*c))
        .map_or(text.len(), |(index, _)| anchor + index);
    let first = text[start..end].chars().next()?;
    (!first.is_ascii_digit()).then_some((start, end))
}

/// Offset of the symbol given by `line` and `column`; without `column`, the
/// first identifier of the line.
fn position_offset(
    request: &Request,
    text: &str,
    starts: &[usize],
    code: Option<&dyn CodeService>,
    language: Option<&LanguageId>,
) -> Result<usize> {
    let line = request.line.unwrap_or(1);
    if let Some(column) = request.column {
        return Ok(offset_at(text, starts, TextPosition { line, column }));
    }
    let index = usize::try_from(line - 1).unwrap_or(usize::MAX);
    let start = *starts.get(index).ok_or_else(|| {
        Error::new(
            ErrorCode::InvalidValue,
            format!(
                "{} has {} lines; give a `line` inside the file",
                request.path.display(),
                starts.len()
            ),
        )
    })?;
    let end = start + line_text(text, starts, start).len();
    if let (Some(code), Some(language)) = (code, language)
        && let Ok(tokens) = code.tokens(text.as_bytes(), language)
    {
        let first = tokens.into_iter().find(|token| {
            let at = usize::try_from(token.bytes.start).unwrap_or(usize::MAX);
            at >= start && at < end && token.kind.ends_with("identifier")
        });
        if let Some(token) = first {
            return usize::try_from(token.bytes.start).map_err(|_| limit_error());
        }
    }
    Ok(text[start..end]
        .char_indices()
        .find(|(_, c)| c.is_alphabetic() || *c == '_')
        .map_or(start, |(index, _)| start + index))
}

/// Identifier of a plan: a digest of the edits and of the entity tags of
/// the files they read.
fn plan_id(state: &RenameState) -> String {
    let mut hasher = DefaultHasher::new();
    state.from.hash(&mut hasher);
    state.to.hash(&mut hasher);
    for target in &state.targets {
        target.path.hash(&mut hasher);
        if let Some((path, etag)) = &target.etag {
            path.hash(&mut hasher);
            etag.hash(&mut hasher);
        }
        for edit in &target.edits {
            (edit.start, edit.end, &edit.new_text).hash(&mut hasher);
        }
    }
    format!("plan-{:016x}", hasher.finish())
}

/// Occurrences of the old name in the files of the scope after the change,
/// with `overrides` giving the content of changed files.
fn leftovers(
    ctx: &ExecCtx,
    state: &RenameState,
    overrides: &BTreeMap<PathBuf, Option<String>>,
) -> Result<Vec<Leftover>> {
    let mut found = Vec::new();
    let first = state
        .targets
        .first()
        .map_or_else(|| state.root.clone(), |target| target.path.clone());
    let mut paths: BTreeSet<PathBuf> = candidates(ctx, &state.roots, &first)?.into_iter().collect();
    paths.extend(overrides.keys().cloned());
    for path in paths {
        ctx.checkpoint()?;
        let text = match overrides.get(&path) {
            Some(text) => text.clone(),
            None => read_text(ctx, &path).ok().map(|loaded| loaded.text),
        };
        if let Some(text) = text {
            found.extend(leftovers_in(ctx, state, &path, &text));
        }
        let file_name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned());
        if file_name.is_some_and(|name| !whole_words(&name, &state.from).is_empty()) {
            found.push(Leftover {
                path,
                line: 0,
                category: "filename",
            });
        }
    }
    Ok(found)
}

fn leftovers_in(ctx: &ExecCtx, state: &RenameState, path: &FsPath, text: &str) -> Vec<Leftover> {
    if !text.contains(&state.from) {
        return Vec::new();
    }
    let starts = line_starts(text);
    let language = ctx
        .code()
        .and_then(|code| code.language_of(Some(path), text.as_bytes()));
    let classified = match (ctx.code(), &language) {
        (Some(code), Some(language)) => {
            code.references(text.as_bytes(), language, &state.from).ok()
        }
        _ => None,
    };
    if let Some(references) = classified {
        let same_family = language.as_ref().map(family) == state.language.as_ref().map(family);
        return references
            .into_iter()
            .map(|reference| Leftover {
                path: path.to_path_buf(),
                line: reference.span.start.line,
                category: match reference.class {
                    SpanClass::Comment => "comment",
                    SpanClass::String => "string",
                    _ if same_family => "code",
                    _ => "other_code",
                },
            })
            .collect();
    }
    let category = text_category(path);
    whole_words(text, &state.from)
        .into_iter()
        .map(|offset| Leftover {
            path: path.to_path_buf(),
            line: position_at(text, &starts, offset).line,
            category,
        })
        .collect()
}

fn text_category(path: &FsPath) -> &'static str {
    let extension = path
        .extension()
        .map(|extension| extension.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    if DOC_EXTENSIONS.contains(&extension.as_str()) {
        "docs"
    } else if CONFIG_EXTENSIONS.contains(&extension.as_str()) {
        "config"
    } else {
        "text"
    }
}

/// Notable effects of the completeness check.
fn leftover_notes(state: &RenameState) -> Vec<NotableEntry> {
    if state.leftovers.is_empty() {
        return Vec::new();
    }
    let mut by_category: BTreeMap<&str, usize> = BTreeMap::new();
    for leftover in &state.leftovers {
        *by_category.entry(leftover.category).or_default() += 1;
    }
    let counts: Vec<String> = by_category
        .iter()
        .map(|(category, number)| format!("{category} {number}"))
        .collect();
    let sample: Vec<String> = state
        .leftovers
        .iter()
        .take(SAMPLE)
        .map(|leftover| {
            format!(
                "{}:{} ({})",
                leftover.path.display(),
                leftover.line,
                leftover.category
            )
        })
        .collect();
    let mut left = NotableEntry::new(Notable::ReferencesLeft);
    left.count = Some(count(state.leftovers.len()));
    left.path = state
        .leftovers
        .first()
        .map(|leftover| leftover.path.display().to_string());
    left.detail = Some(format!(
        "`{}` remains {} times ({}): {}",
        state.from,
        state.leftovers.len(),
        counts.join(", "),
        sample.join(", ")
    ));
    let mut notes = vec![left];
    let in_code: Vec<&Leftover> = state
        .leftovers
        .iter()
        .filter(|leftover| leftover.category == "code")
        .collect();
    if !in_code.is_empty() {
        let mut incomplete = NotableEntry::new(Notable::RenameIncomplete);
        incomplete.count = Some(count(in_code.len()));
        incomplete.path = in_code
            .first()
            .map(|leftover| leftover.path.display().to_string());
        incomplete.detail = Some(format!(
            "{} occurrences of `{}` in code were not renamed; check them before relying on the change",
            in_code.len(),
            state.from
        ));
        notes.push(incomplete);
    }
    notes
}

impl RenameState {
    fn changes(&self) -> Vec<Change> {
        self.targets.iter().map(Target::change).collect()
    }

    fn total(&self) -> usize {
        self.targets.iter().map(|target| target.edits.len()).sum()
    }

    fn preview_records(&self) -> Vec<Record> {
        let mut records: Vec<Record> = self
            .targets
            .iter()
            .flat_map(|target| {
                target.edits.iter().map(|edit| {
                    Record::from(RenameEdit {
                        path: Path::new(target.path.to_string_lossy()),
                        line: u64::from(edit.start_pos.line),
                        column: u64::from(edit.start_pos.column),
                        end_line: u64::from(edit.end_pos.line),
                        end_column: u64::from(edit.end_pos.column),
                        old_text: edit.old_text.clone(),
                        new_text: edit.new_text.clone(),
                        kind: edit.kind,
                        source: self.source,
                    })
                })
            })
            .collect();
        records.push(self.summary());
        records
    }

    /// Summary of the plan as a `map` record.
    fn summary(&self) -> Record {
        let mut fields = Fields::new();
        let text = |value: &str| Value::Text(value.to_owned());
        fields.insert("plan".to_owned(), text(&self.id));
        fields.insert("from".to_owned(), text(&self.from));
        fields.insert("to".to_owned(), text(&self.to));
        fields.insert(
            "source".to_owned(),
            Value::Enum(self.source.as_str().to_owned()),
        );
        if let Some(server) = &self.server {
            fields.insert("server".to_owned(), text(server));
        }
        let confidence = match self.source {
            SymbolSource::Lsp => "semantic",
            _ => "identifier",
        };
        fields.insert("confidence".to_owned(), Value::Enum(confidence.to_owned()));
        fields.insert(
            "root".to_owned(),
            Value::Path(self.root.to_string_lossy().into_owned()),
        );
        fields.insert("files".to_owned(), Value::Int(int(self.targets.len())));
        fields.insert("total".to_owned(), Value::Int(int(self.total())));
        fields.insert(
            "alarms".to_owned(),
            Value::List(self.alarms.iter().map(|alarm| text(alarm)).collect()),
        );
        fields.insert(
            "leftovers".to_owned(),
            Value::List(
                self.leftovers
                    .iter()
                    .map(|leftover| {
                        text(&format!(
                            "{}:{} {}",
                            leftover.path.display(),
                            leftover.line,
                            leftover.category
                        ))
                    })
                    .collect(),
            ),
        );
        Record {
            kind: RecordKind::Map,
            fields,
        }
    }
}

fn int(value: usize) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

impl Target {
    fn change(&self) -> Change {
        let (action, from) = match &self.action {
            TargetKind::Modify => (ChangeAction::Modify, None),
            TargetKind::Create => (ChangeAction::Create, None),
            TargetKind::Delete => (ChangeAction::Delete, None),
            TargetKind::Rename { from, .. } => (
                ChangeAction::Rename,
                Some(Path::new(from.display().to_string())),
            ),
        };
        let mut change = Change::new(self.path.display().to_string(), action);
        change.from = from;
        if !self.edits.is_empty() {
            change.replacements = Some(count(self.edits.len()));
            let lines: BTreeSet<u32> = self.edits.iter().map(|edit| edit.start_pos.line).collect();
            change.added = Some(count(lines.len()));
            change.removed = Some(count(lines.len()));
        }
        change
    }

    fn spec(&self) -> Result<TargetSpec> {
        let expect = match &self.etag {
            Some((path, etag)) if *path == self.path => Expect::Etag(etag.clone()),
            _ if self.action == TargetKind::Create => Expect::Absent,
            _ => Expect::Unchecked,
        };
        let (path, action) = match &self.action {
            TargetKind::Modify => (&self.path, TargetAction::Modify),
            TargetKind::Create => (&self.path, TargetAction::Create),
            TargetKind::Delete => (&self.path, TargetAction::Delete),
            TargetKind::Rename { from, replace } => (
                from,
                TargetAction::Rename {
                    to: AbsPath::new(self.path.clone())?,
                    replace: *replace,
                },
            ),
        };
        let expect = match (&self.action, &self.etag) {
            (TargetKind::Rename { .. }, Some((_, etag))) => Expect::Etag(etag.clone()),
            _ => expect,
        };
        Ok(TargetSpec {
            path: TargetPath::Fs(AbsPath::new(path.clone())?),
            action,
            expect,
            size_hint: None,
        })
    }
}

struct Applier<'a> {
    ctx: &'a ExecCtx,
    semantic: Option<&'a dyn SemanticProvider>,
    state: &'a RenameState,
}

/// Error found by a check of one file.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Problem {
    line: u32,
    column: u32,
    message: String,
}

impl Applier<'_> {
    fn apply(&self, tx: &mut dyn MutationTx) -> Result<EffectOutcome<'static>> {
        self.check_fresh()?;
        let mut contents = Vec::new();
        for target in &self.state.targets {
            self.ctx.checkpoint()?;
            let content = match target.action {
                TargetKind::Modify | TargetKind::Create => {
                    let base = base_text(self.ctx, &target.base)?;
                    let after = apply_edits(&base, &target.edits);
                    let before = self.problems(target, &base);
                    Some((after, before))
                }
                TargetKind::Delete | TargetKind::Rename { .. } => None,
            };
            contents.push(content);
        }
        let specs = self
            .state
            .targets
            .iter()
            .map(Target::spec)
            .collect::<Result<Vec<_>>>()?;
        tx.preserve(&specs)?;
        self.mutate(tx, &contents)?;
        self.verify(&contents)?;
        Ok(self.outcome())
    }

    /// Fails with `E_STALE` when a file read by the plan changed since.
    fn check_fresh(&self) -> Result<()> {
        for target in &self.state.targets {
            if let Some((path, planned)) = &target.etag {
                let current = etag_of(self.ctx, path).ok();
                if current
                    .as_ref()
                    .is_none_or(|current| !current.matches(planned))
                {
                    return Err(Error::new(
                        ErrorCode::Stale,
                        format!(
                            "{} changed after the rename was planned; run `code.rename` again",
                            path.display()
                        ),
                    )
                    .with_found(path.display().to_string()));
                }
            }
        }
        Ok(())
    }

    fn mutate(
        &self,
        tx: &mut dyn MutationTx,
        contents: &[Option<(String, Vec<Problem>)>],
    ) -> Result<()> {
        let order = [0_u8, 1, 2];
        for phase in order {
            for (index, target) in self.state.targets.iter().enumerate() {
                let mutation = match (&target.action, &contents[index]) {
                    (TargetKind::Rename { .. }, _) if phase == 0 => Mutation::Rename,
                    (TargetKind::Delete, _) if phase == 1 => Mutation::Remove,
                    (TargetKind::Modify | TargetKind::Create, Some((after, _))) if phase == 2 => {
                        Mutation::WriteBytes(after.clone().into_bytes())
                    }
                    _ => continue,
                };
                let index = TargetIndex(u32::try_from(index).map_err(|_| limit_error())?);
                tx.apply(index, mutation)?;
            }
        }
        Ok(())
    }

    /// Compares errors before and after the change; a new error fails with
    /// `E_ROLLED_BACK`, after which the executor restores every file.
    fn verify(&self, contents: &[Option<(String, Vec<Problem>)>]) -> Result<()> {
        for (target, content) in self.state.targets.iter().zip(contents) {
            let Some((after, before)) = content else {
                continue;
            };
            let after_problems = self.problems(target, after);
            let new = new_problems(before, &after_problems, &self.state.from, &self.state.to);
            if let Some(problem) = new.first() {
                return Err(Error::new(
                    ErrorCode::RolledBack,
                    format!(
                        "renaming `{}` to `{}` introduced an error at {}:{}:{}: {}; every file was restored",
                        self.state.from,
                        self.state.to,
                        target.path.display(),
                        problem.line,
                        problem.column,
                        problem.message
                    ),
                )
                .with_found(format!("{}:{}:{}", target.path.display(), problem.line, problem.column))
                .with_effect_applied(false));
            }
        }
        Ok(())
    }

    /// Syntax errors and language server errors of `text` as the content of
    /// the target.
    fn problems(&self, target: &Target, text: &str) -> Vec<Problem> {
        let mut problems = Vec::new();
        if let (Some(code), Some(language)) = (self.ctx.code(), &target.language)
            && let Ok(issues) = code.check_syntax(text.as_bytes(), language)
        {
            problems.extend(
                issues
                    .into_iter()
                    .filter(|issue| issue.severity == IssueSeverity::Error)
                    .map(|issue| Problem {
                        line: issue.span.start.line,
                        column: issue.span.start.column,
                        message: issue.message,
                    }),
            );
        }
        if let (Some(semantic), SymbolSource::Lsp) = (self.semantic, self.state.source) {
            let scope = SemanticScope {
                root: self.state.root.clone(),
                language: target.language.clone(),
                timeout: None,
            };
            if let Ok(found) = semantic.diagnostics(self.ctx, &scope, &target.path, Some(text)) {
                problems.extend(
                    found
                        .into_iter()
                        .filter(|found| {
                            found.severity == nurt_contracts::record::DiagnosticSeverity::Error
                        })
                        .map(|found| Problem {
                            line: found.range.start.line,
                            column: found.range.start.column,
                            message: found.message,
                        }),
                );
            }
        }
        problems
    }

    fn outcome(&self) -> EffectOutcome<'static> {
        let state = self.state;
        let changes = state.changes();
        let mut notable = state.plan_notes.clone();
        if state.check_references {
            let actual = leftovers(self.ctx, state, &BTreeMap::new()).unwrap_or_default();
            let checked = RenameState {
                leftovers: actual,
                ..state.clone()
            };
            notable.extend(leftover_notes(&checked));
        }
        let mut summary = indexmap::IndexMap::new();
        for target in &state.targets {
            let key = match target.action {
                TargetKind::Modify => "files_modified",
                TargetKind::Create => "files_created",
                TargetKind::Delete => "files_deleted",
                TargetKind::Rename { .. } => "files_renamed",
            };
            *summary.entry(key.to_owned()).or_insert(0) += 1;
        }
        summary.insert("replacements".to_owned(), count(state.total()));
        let effect = Effect {
            status: EffectStatus::Ok,
            kind: COMMAND.to_owned(),
            targets: Some(count(state.targets.len())),
            summary: Some(summary),
            changes: Some(changes.clone()),
            notable: (!notable.is_empty()).then(|| {
                notable
                    .iter()
                    .map(|entry| entry.code.as_str().to_owned())
                    .collect()
            }),
            ..Effect::default()
        };
        let mut outcome = EffectOutcome::new(StageOutput::records(RecordStream::once(
            Record::from(effect),
        )));
        outcome.changes = changes;
        outcome.notable = notable;
        outcome
    }
}

/// Problems of `after` that `before` does not have. Messages are compared
/// with the new name read as the old one, so a message that names the
/// symbol is not counted as new.
fn new_problems(before: &[Problem], after: &[Problem], from: &str, to: &str) -> Vec<Problem> {
    let mut known: BTreeMap<String, usize> = BTreeMap::new();
    for problem in before {
        *known.entry(problem.message.clone()).or_default() += 1;
    }
    let mut new = Vec::new();
    for problem in after {
        let key = problem.message.replace(to, from);
        match known.get_mut(&key) {
            Some(left) if *left > 0 => *left -= 1,
            _ => new.push(problem.clone()),
        }
    }
    new
}
