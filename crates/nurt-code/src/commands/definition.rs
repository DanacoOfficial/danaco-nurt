// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-code::commands::definition
// Responsibility:   The `code.definition` stage.
// Dependencies:     nurt_contracts, regex
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! The `code.definition` stage.
//!
//! The stage answers from the language server of the file
//! ([`SemanticProvider::definition`]). Under `source: auto` a missing,
//! failing or slow server, or an empty answer, gives way to syntax analysis:
//! definitions of the identifier at the position in the file and then in
//! the files of the project in the same language, with a `semantic_fallback`
//! note that gives the cause.

use std::sync::Arc;

use nurt_contracts::error::Result;
use nurt_contracts::record::SymbolSource;
use nurt_contracts::stage::{
    BoundArgs, ExecCtx, InputKinds, ReadStage, RecordStream, StageDescriptor, StageImpl,
    StageInput, StageOutput,
};
use nurt_contracts::value::Record;

use crate::semantic::SemanticProvider;

use self::shared::{Files, Source, Target};

const COMMAND: &str = "code.definition";

/// Implementation of `code.definition`.
///
/// ```
/// use nurt_code::commands::definition::CodeDefinition;
/// use nurt_contracts::stage::StageImpl;
///
/// let stage = CodeDefinition::new(None);
/// assert_eq!(stage.descriptor().command, "code.definition");
/// assert!(stage.as_read().is_some());
/// ```
pub struct CodeDefinition {
    semantic: Option<Arc<dyn SemanticProvider>>,
}

impl CodeDefinition {
    /// Stage that asks `semantic` first; without a provider every answer
    /// comes from syntax analysis.
    #[must_use]
    pub fn new(semantic: Option<Arc<dyn SemanticProvider>>) -> Self {
        Self { semantic }
    }

    fn records(&self, args: &BoundArgs, ctx: &ExecCtx) -> Result<Vec<Record>> {
        let files = Files::new(ctx);
        let source = Source::from_args(args)?;
        let target = Target::resolve(args, ctx, &files, COMMAND)?;
        let served = shared::ask(self.semantic.as_deref(), source, ctx, |provider| {
            provider.definition(ctx, &target.scope(), &target.at())
        })?;
        match served {
            Some(locations) if !locations.is_empty() || source == Source::Lsp => {
                return Ok(locations
                    .iter()
                    .map(|location| {
                        shared::symbol_record(&files, &target, location, SymbolSource::Lsp).into()
                    })
                    .collect());
            }
            Some(_) => shared::note_fallback(ctx, "the language server found no definition"),
            None => {}
        }
        Ok(shared::local_definitions(ctx, &files, &target)?
            .iter()
            .map(|(path, info)| shared::local_symbol_record(&files, &target, path, info).into())
            .collect())
    }
}

impl StageImpl for CodeDefinition {
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

impl ReadStage for CodeDefinition {
    fn execute<'a>(
        &'a self,
        args: &'a BoundArgs,
        _input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<StageOutput<'a>> {
        let records = self.records(args, ctx)?;
        Ok(StageOutput::records(RecordStream::from_records(records)))
    }
}

/// Parts shared by the navigation and diagnostics stages of the code group:
/// arguments, file access through the context, the target position, the
/// call of the semantic provider with the syntax fallback, and record
/// construction.
pub(crate) mod shared {
    use std::cell::RefCell;
    use std::collections::{HashMap, HashSet};
    use std::path::{Path, PathBuf};
    use std::rc::Rc;
    use std::sync::LazyLock;

    use nurt_contracts::error::{Error, ErrorCode, Result};
    use nurt_contracts::fs::{AbsPath, FileKind, Follow};
    use nurt_contracts::record::{
        ReferenceRole as RecordRole, Symbol, SymbolKind as RecordKind, SymbolSource,
    };
    use nurt_contracts::report::{Notable, NotableEntry};
    use nurt_contracts::services::{
        CodeService, LanguageId, ReferenceRole, SpanClass, SymbolInfo, SymbolKind, TextPosition,
    };
    use nurt_contracts::stage::{BoundArgs, ExecCtx};
    use nurt_contracts::value::{Path as RecordPath, Value};
    use regex::Regex;

    use crate::index::{content_hash, read_bounded};
    use crate::semantic::{
        DocumentPosition, Location, SemanticProvider, SemanticResult, SemanticScope,
    };

    /// Directories never searched for project files.
    pub(crate) const SKIPPED_DIRECTORIES: &[&str] =
        &[".git", "node_modules", "__pycache__", "venv", ".venv"];

    /// Files that mark the root of a project, nearest first.
    const ROOT_MARKERS: &[&str] = &[
        "pyproject.toml",
        "setup.py",
        "package.json",
        "tsconfig.json",
        "go.mod",
        "Cargo.toml",
        "compile_commands.json",
        ".git",
    ];

    /// Longest signature kept in a record, in characters.
    pub(crate) const SIGNATURE_MAX: usize = 240;

    /// Longest documentation sentence kept by the short form, in characters.
    pub(crate) const SENTENCE_MAX: usize = 200;

    /// Marker appended to text shortened by [`clip`].
    const ELLIPSIS: &str = "...";

    /// Most definitions listed as samples of an ambiguity error.
    const AMBIGUITY_SAMPLES: usize = 8;

    static TEST_PATH: LazyLock<Option<Regex>> = LazyLock::new(|| {
        Regex::new(
            r"(?:^|/)(?:tests?|__tests__|spec|specs)/|[._-](?:test|spec)\.[^/]+$|(?:^|/)test_[^/]+$|_test\.[^/]+$",
        )
        .ok()
    });

    static IMPORT_LINE: LazyLock<Option<Regex>> = LazyLock::new(|| {
        Regex::new(
            r"^\s*(?:import\b|from\s+\S+\s+import\b|export\s[^=]*\bfrom\b|use\s|#\s*include\b|using\s|require\b)|\brequire\s*\(|\bimport\s*\(",
        )
        .ok()
    });

    /// Analysis selected by the `source` parameter.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(crate) enum Source {
        /// The language server when available, syntax analysis otherwise.
        Auto,
        /// The language server only.
        Lsp,
        /// Syntax analysis only.
        Syntax,
    }

    impl Source {
        pub(crate) fn from_args(args: &BoundArgs) -> Result<Self> {
            match word_arg(args, "source")?.as_deref() {
                None | Some("auto") => Ok(Self::Auto),
                Some("lsp") => Ok(Self::Lsp),
                Some("syntax") => Ok(Self::Syntax),
                Some(other) => Err(Error::new(
                    ErrorCode::InvalidValue,
                    format!("unknown source `{other}`; use auto, lsp or syntax"),
                )),
            }
        }
    }

    /// Text or enumeration value of a parameter.
    pub(crate) fn word_arg(args: &BoundArgs, name: &str) -> Result<Option<String>> {
        match args.value(name) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::Text(text) | Value::Enum(text)) => Ok(Some(text.clone())),
            Some(_) => Err(Error::new(
                ErrorCode::Type,
                format!("`{}` expects text for `{name}`", args.command()),
            )),
        }
    }

    /// Path value of a parameter.
    pub(crate) fn path_arg(args: &BoundArgs, name: &str) -> Result<Option<String>> {
        match args.value(name) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::Text(text) | Value::Path(text)) => Ok(Some(text.clone())),
            Some(_) => Err(Error::new(
                ErrorCode::Type,
                format!("`{}` expects a path for `{name}`", args.command()),
            )),
        }
    }

    /// Path values of a list parameter (a single path is accepted too).
    pub(crate) fn paths_arg(args: &BoundArgs, name: &str) -> Result<Vec<String>> {
        let items = match args.value(name) {
            None | Some(Value::Null) => return Ok(Vec::new()),
            Some(Value::List(items)) => items.clone(),
            Some(other) => vec![other.clone()],
        };
        items
            .into_iter()
            .map(|item| match item {
                Value::Text(text) | Value::Path(text) => Ok(text),
                _ => Err(Error::new(
                    ErrorCode::Type,
                    format!("`{}` expects paths for `{name}`", args.command()),
                )),
            })
            .collect()
    }

    /// Positive line or column value of a parameter.
    pub(crate) fn number_arg(args: &BoundArgs, name: &str) -> Result<Option<u32>> {
        let Some(number) = args.int(name)? else {
            return Ok(None);
        };
        u32::try_from(number)
            .ok()
            .filter(|number| *number >= 1)
            .map(Some)
            .ok_or_else(|| {
                Error::new(
                    ErrorCode::InvalidValue,
                    format!("`{name}` must be a positive number, not {number}"),
                )
            })
    }

    /// Absolute form of a path argument, relative to the base directory.
    pub(crate) fn resolve_path(ctx: &ExecCtx, text: &str) -> Result<PathBuf> {
        let path = Path::new(text);
        if path.is_absolute() {
            return Ok(path.to_path_buf());
        }
        match ctx.options().base.as_deref() {
            Some(base) => Ok(Path::new(base).join(path)),
            None => Err(Error::new(
                ErrorCode::Arg,
                format!("relative path `{text}` needs a base directory; give an absolute path"),
            )),
        }
    }

    /// Path in the form of the language, for record fields.
    pub(crate) fn record_path(path: &Path) -> RecordPath {
        let text = AbsPath::new(path).map_or_else(
            |_| path.to_string_lossy().into_owned(),
            |absolute| absolute.to_language_string(),
        );
        RecordPath::new(text)
    }

    /// Content of a source file read through the context.
    #[derive(Debug)]
    pub(crate) struct FileText {
        pub(crate) text: String,
        pub(crate) etag: String,
    }

    impl FileText {
        pub(crate) fn line(&self, line: u32) -> Option<&str> {
            line_text(&self.text, line)
        }
    }

    /// Files read during one execution.
    pub(crate) struct Files<'a> {
        ctx: &'a ExecCtx,
        cache: RefCell<HashMap<PathBuf, Option<Rc<FileText>>>>,
    }

    impl<'a> Files<'a> {
        pub(crate) fn new(ctx: &'a ExecCtx) -> Self {
            Self {
                ctx,
                cache: RefCell::new(HashMap::new()),
            }
        }

        /// Content of a file.
        pub(crate) fn read(&self, path: &Path) -> Result<Rc<FileText>> {
            if let Some(Some(known)) = self.cache.borrow().get(path) {
                return Ok(Rc::clone(known));
            }
            let read = read_file(self.ctx, path).map(Rc::new);
            self.cache
                .borrow_mut()
                .insert(path.to_path_buf(), read.as_ref().ok().cloned());
            read
        }

        /// Content of a file; `None` when it cannot be read.
        pub(crate) fn get(&self, path: &Path) -> Option<Rc<FileText>> {
            if let Some(known) = self.cache.borrow().get(path) {
                return known.clone();
            }
            self.read(path).ok()
        }

        /// Returns `true` when `path` exists.
        pub(crate) fn exists(&self, path: &Path) -> bool {
            let Ok(fs) = self.ctx.require_fs() else {
                return false;
            };
            AbsPath::new(path).is_ok_and(|path| fs.metadata(&path, Follow::Follow).is_ok())
        }

        /// Returns `true` when `path` is a directory.
        pub(crate) fn is_dir(&self, path: &Path) -> bool {
            let Ok(fs) = self.ctx.require_fs() else {
                return false;
            };
            AbsPath::new(path).is_ok_and(|path| {
                fs.metadata(&path, Follow::Follow)
                    .is_ok_and(|meta| meta.kind == FileKind::Directory)
            })
        }

        /// Files under `root` accepted by `accept`, in path order, skipping
        /// [`SKIPPED_DIRECTORIES`], no more than `code.index_max_files`; the
        /// flag is `true` when accepted files were left out because of the
        /// limit, and then a `source_unavailable` note is recorded.
        ///
        /// # Errors
        ///
        /// Fails when the execution is cancelled.
        pub(crate) fn walk(
            &self,
            root: &Path,
            accept: &dyn Fn(&Path) -> bool,
        ) -> Result<(Vec<PathBuf>, bool)> {
            let found = self.walk_limited(root, accept)?;
            if found.1 {
                let mut note = NotableEntry::new(Notable::SourceUnavailable);
                note.path = Some(root.to_string_lossy().into_owned());
                note.detail = Some(format!(
                    "the file listing stopped at code.index_max_files ({}) files; narrow the search or raise the limit",
                    self.ctx.config().code.index_max_files
                ));
                self.ctx.note(note);
            }
            Ok(found)
        }

        fn walk_limited(
            &self,
            root: &Path,
            accept: &dyn Fn(&Path) -> bool,
        ) -> Result<(Vec<PathBuf>, bool)> {
            let max = usize::try_from(self.ctx.config().code.index_max_files).unwrap_or(usize::MAX);
            let Ok(fs) = self.ctx.require_fs() else {
                return Ok((Vec::new(), false));
            };
            let mut found = Vec::new();
            let mut pending = vec![root.to_path_buf()];
            while let Some(directory) = pending.pop() {
                self.ctx.checkpoint()?;
                let Ok(absolute) = AbsPath::new(&directory) else {
                    continue;
                };
                let Ok(entries) = fs.read_dir(&absolute) else {
                    continue;
                };
                let mut entries: Vec<_> = entries.filter_map(std::result::Result::ok).collect();
                entries.sort_by(|left, right| left.name.cmp(&right.name));
                let mut directories = Vec::new();
                for entry in entries {
                    let path = directory.join(&entry.name);
                    let kind = entry.kind.or_else(|| {
                        let absolute = AbsPath::new(&path).ok()?;
                        fs.metadata(&absolute, Follow::NoFollow)
                            .ok()
                            .map(|meta| meta.kind)
                    });
                    match kind {
                        Some(FileKind::Directory) => {
                            if !SKIPPED_DIRECTORIES.iter().any(|name| entry.name == *name) {
                                directories.push(path);
                            }
                        }
                        Some(FileKind::File) if accept(&path) => {
                            if found.len() >= max {
                                return Ok((found, true));
                            }
                            found.push(path);
                        }
                        _ => {}
                    }
                }
                pending.extend(directories.into_iter().rev());
            }
            Ok((found, false))
        }
    }

    fn read_file(ctx: &ExecCtx, path: &Path) -> Result<FileText> {
        let fs = ctx.require_fs()?;
        let absolute = AbsPath::new(path).map_err(Error::from)?;
        let mut handle = fs.open_read(&absolute).map_err(Error::from)?;
        let meta = handle.meta().map_err(Error::from)?;
        let max = ctx.config().code.file_bytes_max.bytes();
        let too_large = |size: u64| {
            Error::new(
                ErrorCode::Limit,
                format!(
                    "{} has {size} bytes, more than code.file_bytes_max ({max}); raise the limit to analyse it",
                    path.display()
                ),
            )
        };
        if meta.size > max {
            return Err(too_large(meta.size));
        }
        let read = read_bounded(&mut handle, max).map_err(|error| {
            Error::new(
                ErrorCode::Io,
                format!("cannot read {}: {error}", path.display()),
            )
        })?;
        let Some(bytes) = read else {
            return Err(too_large(max.saturating_add(1)));
        };
        let etag = content_hash(&bytes).to_string();
        let text = String::from_utf8(bytes).map_err(|_| {
            Error::new(
                ErrorCode::Encoding,
                format!(
                    "{} is not UTF-8 text; code commands read UTF-8 sources",
                    path.display()
                ),
            )
        })?;
        Ok(FileText { text, etag })
    }

    /// Line `line` (from 1) of `text`, without the line ending.
    pub(crate) fn line_text(text: &str, line: u32) -> Option<&str> {
        let index = usize::try_from(line.checked_sub(1)?).ok()?;
        text.split('\n')
            .nth(index)
            .map(|line| line.strip_suffix('\r').unwrap_or(line))
    }

    fn is_ident(character: char) -> bool {
        character.is_alphanumeric() || character == '_' || character == '$'
    }

    /// Identifier that contains or ends at a position.
    pub(crate) fn word_at(text: &str, position: TextPosition) -> Option<String> {
        let characters: Vec<char> = line_text(text, position.line)?.chars().collect();
        let mut index = usize::try_from(position.column.checked_sub(1)?).ok()?;
        if !characters.get(index).copied().is_some_and(is_ident) {
            index = index
                .checked_sub(1)
                .filter(|before| characters.get(*before).copied().is_some_and(is_ident))?;
        }
        let start = characters[..index]
            .iter()
            .rposition(|character| !is_ident(*character))
            .map_or(0, |found| found + 1);
        let end = characters[index..]
            .iter()
            .position(|character| !is_ident(*character))
            .map_or(characters.len(), |found| index + found);
        Some(characters[start..end].iter().collect())
    }

    /// Column of the first identifier of a line, 1 when there is none.
    pub(crate) fn first_word_column(line: &str) -> u32 {
        line.chars()
            .position(|character| character.is_alphabetic() || character == '_' || character == '$')
            .and_then(|index| u32::try_from(index + 1).ok())
            .unwrap_or(1)
    }

    /// Byte offset of a position in `text`.
    pub(crate) fn offset_of(text: &str, position: TextPosition) -> Option<u64> {
        let mut offset = 0usize;
        for (index, line) in text.split('\n').enumerate() {
            if u32::try_from(index + 1).ok()? == position.line {
                let column = usize::try_from(position.column.checked_sub(1)?).ok()?;
                let within: usize = line.chars().take(column).map(char::len_utf8).sum();
                return u64::try_from(offset + within).ok();
            }
            offset += line.len() + 1;
        }
        None
    }

    /// Project root of a file: the nearest directory with a project marker,
    /// or the directory of the file.
    pub(crate) fn project_root(files: &Files<'_>, path: &Path) -> PathBuf {
        let start = path
            .parent()
            .map_or_else(|| path.to_path_buf(), Path::to_path_buf);
        start
            .ancestors()
            .find(|directory| {
                ROOT_MARKERS
                    .iter()
                    .any(|marker| files.exists(&directory.join(marker)))
            })
            .map_or_else(|| start.clone(), Path::to_path_buf)
    }

    /// Returns `true` for paths of test files by common conventions.
    pub(crate) fn is_test_path(path: &Path) -> bool {
        let text = path.to_string_lossy().replace('\\', "/");
        TEST_PATH
            .as_ref()
            .is_some_and(|pattern| pattern.is_match(&text))
    }

    /// Returns `true` for import and include lines.
    pub(crate) fn is_import_line(line: &str) -> bool {
        IMPORT_LINE
            .as_ref()
            .is_some_and(|pattern| pattern.is_match(line))
    }

    /// Text shortened to `max` characters.
    pub(crate) fn clip(text: &str, max: usize) -> String {
        if text.chars().count() <= max {
            return text.to_owned();
        }
        let mut clipped: String = text
            .chars()
            .take(max.saturating_sub(ELLIPSIS.len()))
            .collect();
        clipped.push_str(ELLIPSIS);
        clipped
    }

    /// File, language, project and position of a positional command.
    pub(crate) struct Target {
        pub(crate) path: PathBuf,
        pub(crate) file: Rc<FileText>,
        /// Language given by the `language` parameter.
        pub(crate) requested: Option<LanguageId>,
        /// Language of the grammar used by syntax analysis.
        pub(crate) language: Option<LanguageId>,
        pub(crate) root: PathBuf,
        pub(crate) position: TextPosition,
        pub(crate) word: Option<String>,
    }

    impl Target {
        /// Target of a command with `path`, `line`, `column`, `symbol`,
        /// `language` and `root`.
        pub(crate) fn resolve(
            args: &BoundArgs,
            ctx: &ExecCtx,
            files: &Files<'_>,
            command: &str,
        ) -> Result<Self> {
            let text = path_arg(args, "path")?.ok_or_else(|| {
                Error::new(
                    ErrorCode::MissingParam,
                    format!("`{command}` needs a value for `path`"),
                )
            })?;
            let path = resolve_path(ctx, &text)?;
            let requested = requested_language(args)?;
            let line = number_arg(args, "line")?;
            let column = number_arg(args, "column")?;
            let symbol = word_arg(args, "symbol")?;
            let root = root_arg(args, ctx, files, &path)?;
            Self::at_position(
                ctx,
                files,
                path,
                requested,
                root,
                (line, column, symbol.as_deref()),
                command,
            )
        }

        /// Target of a symbol record of the input.
        pub(crate) fn from_parts(
            ctx: &ExecCtx,
            files: &Files<'_>,
            path: PathBuf,
            requested: Option<LanguageId>,
            root: Option<PathBuf>,
            position: (Option<u32>, Option<u32>, Option<&str>),
            command: &str,
        ) -> Result<Self> {
            let root = root.unwrap_or_else(|| project_root(files, &path));
            Self::at_position(ctx, files, path, requested, root, position, command)
        }

        fn at_position(
            ctx: &ExecCtx,
            files: &Files<'_>,
            path: PathBuf,
            requested: Option<LanguageId>,
            root: PathBuf,
            (line, column, symbol): (Option<u32>, Option<u32>, Option<&str>),
            command: &str,
        ) -> Result<Self> {
            let file = files.read(&path)?;
            let language = requested.clone().or_else(|| {
                ctx.code()
                    .and_then(|code| code.language_of(Some(&path), file.text.as_bytes()))
            });
            let position = match (symbol, line) {
                (Some(symbol), _) => symbol_position(ctx, &path, &file, language.as_ref(), symbol)?,
                (None, Some(line)) => {
                    let text = file.line(line).ok_or_else(|| {
                        Error::new(
                            ErrorCode::InvalidValue,
                            format!("{} has fewer than {line} lines", path.display()),
                        )
                    })?;
                    TextPosition {
                        line,
                        column: column.unwrap_or_else(|| first_word_column(text)),
                    }
                }
                (None, None) => {
                    return Err(Error::new(
                        ErrorCode::MissingParam,
                        format!("`{command}` needs `line` or `symbol`"),
                    ));
                }
            };
            let word = word_at(&file.text, position);
            Ok(Self {
                path,
                file,
                requested,
                language,
                root,
                position,
                word,
            })
        }

        pub(crate) fn scope(&self) -> SemanticScope {
            SemanticScope {
                root: self.root.clone(),
                language: self.requested.clone(),
                timeout: None,
            }
        }

        pub(crate) fn at(&self) -> DocumentPosition {
            DocumentPosition {
                path: self.path.clone(),
                position: self.position,
            }
        }

        /// Name of the language for records.
        pub(crate) fn language_name(&self) -> String {
            self.language
                .as_ref()
                .map(|language| language.as_str().to_owned())
                .or_else(|| {
                    self.path
                        .extension()
                        .map(|extension| extension.to_string_lossy().into_owned())
                })
                .unwrap_or_default()
        }
    }

    /// Language given by the `language` parameter; `auto` is no language.
    pub(crate) fn requested_language(args: &BoundArgs) -> Result<Option<LanguageId>> {
        match word_arg(args, "language")? {
            None => Ok(None),
            Some(name) if name == "auto" => Ok(None),
            Some(name) => LanguageId::new(name.clone()).map(Some).map_err(|_| {
                Error::new(
                    ErrorCode::InvalidValue,
                    format!("`{name}` is not a language name; list languages with code.languages"),
                )
            }),
        }
    }

    /// Project root from the `root` parameter, or detected from `path`.
    pub(crate) fn root_arg(
        args: &BoundArgs,
        ctx: &ExecCtx,
        files: &Files<'_>,
        path: &Path,
    ) -> Result<PathBuf> {
        let Some(text) = path_arg(args, "root")? else {
            return Ok(project_root(files, path));
        };
        let root = resolve_path(ctx, &text)?;
        if !files.is_dir(&root) {
            return Err(Error::new(
                ErrorCode::InvalidValue,
                format!("root {} is not a directory", root.display()),
            ));
        }
        Ok(root)
    }

    fn require_code<'c>(
        ctx: &'c ExecCtx,
        language: Option<&LanguageId>,
        path: &Path,
    ) -> Result<(&'c dyn CodeService, LanguageId)> {
        let unsupported = || {
            Error::new(
                ErrorCode::UnsupportedLanguage,
                format!(
                    "no grammar is available for {}; set language",
                    path.display()
                ),
            )
        };
        let code = ctx.code().ok_or_else(unsupported)?;
        let language = language.cloned().ok_or_else(unsupported)?;
        Ok((code, language))
    }

    fn symbol_position(
        ctx: &ExecCtx,
        path: &Path,
        file: &FileText,
        language: Option<&LanguageId>,
        symbol: &str,
    ) -> Result<TextPosition> {
        let (code, language) = require_code(ctx, language, path)?;
        let found = code.find_symbol(file.text.as_bytes(), &language, symbol)?;
        match found.as_slice() {
            [] => Err(Error::new(
                ErrorCode::SymbolNotFound,
                format!(
                    "no definition of `{symbol}` in {}; check the name with code.symbols or give line and column",
                    path.display()
                ),
            )),
            [only] => Ok(only.name_span.start),
            several => {
                let mut error = Error::new(
                    ErrorCode::SymbolAmbiguous,
                    format!(
                        "`{symbol}` has {} definitions in {}; qualify the name as Container.name or give line and column",
                        several.len(),
                        path.display()
                    ),
                );
                for info in several.iter().take(AMBIGUITY_SAMPLES) {
                    error = error.with_sample(describe(info));
                }
                Err(error)
            }
        }
    }

    fn describe(info: &SymbolInfo) -> String {
        let name = match &info.container {
            Some(container) => format!("{container}.{}", info.name),
            None => info.name.clone(),
        };
        format!(
            "{} {name} line {}",
            kind_name(info.kind),
            info.name_span.start.line
        )
    }

    fn kind_name(kind: SymbolKind) -> &'static str {
        record_kind(kind).as_str()
    }

    /// Record kind of a syntax symbol kind.
    pub(crate) fn record_kind(kind: SymbolKind) -> RecordKind {
        match kind {
            SymbolKind::Function => RecordKind::Function,
            SymbolKind::Method => RecordKind::Method,
            SymbolKind::Class => RecordKind::Class,
            SymbolKind::Struct => RecordKind::Struct,
            SymbolKind::Interface => RecordKind::Interface,
            SymbolKind::Enum => RecordKind::Enum,
            SymbolKind::Variable => RecordKind::Variable,
            SymbolKind::Constant => RecordKind::Constant,
            SymbolKind::Field => RecordKind::Field,
            SymbolKind::Module => RecordKind::Module,
            SymbolKind::Type => RecordKind::Type,
            SymbolKind::Macro => RecordKind::Macro,
            _ => RecordKind::Other,
        }
    }

    /// Records a `semantic_fallback` note with the cause.
    pub(crate) fn note_fallback(ctx: &ExecCtx, detail: &str) {
        let mut entry = NotableEntry::new(Notable::SemanticFallback);
        entry.detail = Some(detail.to_owned());
        ctx.note(entry);
    }

    /// Calls the provider under the rules of `source`.
    ///
    /// `Ok(None)` means syntax analysis applies: `source: syntax`, or
    /// `source: auto` without a provider or with a provider error that
    /// allows a fallback (then a `semantic_fallback` note gives the cause).
    /// With `source: lsp` every provider error is the error of the command.
    pub(crate) fn ask<T>(
        provider: Option<&dyn SemanticProvider>,
        source: Source,
        ctx: &ExecCtx,
        call: impl FnOnce(&dyn SemanticProvider) -> SemanticResult<T>,
    ) -> Result<Option<T>> {
        if source == Source::Syntax {
            return Ok(None);
        }
        let Some(provider) = provider else {
            if source == Source::Lsp {
                return Err(Error::new(
                    ErrorCode::ExternalMissing,
                    "language servers are not available in this engine; use source: syntax",
                ));
            }
            note_fallback(ctx, "language servers are not available");
            return Ok(None);
        };
        match call(provider) {
            Ok(value) => Ok(Some(value)),
            Err(error) if source == Source::Auto && error.allows_fallback() => {
                note_fallback(ctx, &error.to_string());
                Ok(None)
            }
            Err(error) => Err(error.to_error()),
        }
    }

    /// Calls the provider for a command that has no syntax analysis:
    /// every failure, and the absence of a provider, is an error.
    pub(crate) fn require_server<T>(
        provider: Option<&dyn SemanticProvider>,
        source: Source,
        command: &str,
        call: impl FnOnce(&dyn SemanticProvider) -> SemanticResult<T>,
    ) -> Result<T> {
        let missing = || {
            Error::new(
                ErrorCode::ExternalMissing,
                format!("{command} needs a language server; install the server of the language"),
            )
        };
        if source == Source::Syntax {
            return Err(missing());
        }
        let provider = provider.ok_or_else(missing)?;
        call(provider).map_err(|error| error.to_error())
    }

    /// Syntax symbol whose name starts at `position`, from the outline.
    pub(crate) fn symbol_at(
        ctx: &ExecCtx,
        files: &Files<'_>,
        path: &Path,
        position: TextPosition,
    ) -> Option<SymbolInfo> {
        let file = files.get(path)?;
        let code = ctx.code()?;
        let language = code.language_of(Some(path), file.text.as_bytes())?;
        let outline = code.outline(file.text.as_bytes(), &language, None).ok()?;
        outline
            .iter()
            .find(|info| info.name_span.start == position)
            .or_else(|| {
                outline
                    .iter()
                    .find(|info| info.name_span.start.line == position.line)
            })
            .cloned()
    }

    /// Symbol record of a location answered by a server.
    pub(crate) fn symbol_record(
        files: &Files<'_>,
        target: &Target,
        location: &Location,
        source: SymbolSource,
    ) -> Symbol {
        let info = symbol_at(files.ctx, files, &location.path, location.start);
        let file = files.get(&location.path);
        let text = file
            .as_ref()
            .and_then(|file| file.line(location.start.line))
            .map(|line| line.trim().to_owned());
        Symbol {
            name: info
                .as_ref()
                .map(|info| info.name.clone())
                .or_else(|| target.word.clone())
                .unwrap_or_default(),
            kind: info
                .as_ref()
                .map_or(RecordKind::Other, |info| record_kind(info.kind)),
            path: record_path(&location.path),
            line: u64::from(location.start.line),
            column: u64::from(location.start.column),
            end_line: u64::from(
                info.as_ref()
                    .map_or(location.end.line, |info| info.span.end.line),
            ),
            language: target.language_name(),
            source,
            etag: file.map(|file| file.etag.clone()).unwrap_or_default(),
            end_column: Some(u64::from(
                info.as_ref()
                    .map_or(location.end.column, |info| info.span.end.column),
            )),
            container: info.as_ref().and_then(|info| info.container.clone()),
            signature: info
                .as_ref()
                .and_then(|info| info.signature.as_deref())
                .map(|signature| clip(signature, SIGNATURE_MAX)),
            text,
            exported: info.and_then(|info| info.exported),
            references: None,
        }
    }

    /// Symbol record of a definition found by syntax analysis.
    pub(crate) fn local_symbol_record(
        files: &Files<'_>,
        target: &Target,
        path: &Path,
        info: &SymbolInfo,
    ) -> Symbol {
        let file = files.get(path);
        Symbol {
            name: info.name.clone(),
            kind: record_kind(info.kind),
            path: record_path(path),
            line: u64::from(info.name_span.start.line),
            column: u64::from(info.name_span.start.column),
            end_line: u64::from(info.span.end.line),
            language: target.language_name(),
            source: SymbolSource::Syntax,
            etag: file
                .as_ref()
                .map(|file| file.etag.clone())
                .unwrap_or_default(),
            end_column: Some(u64::from(info.span.end.column)),
            container: info.container.clone(),
            signature: info
                .signature
                .as_deref()
                .map(|signature| clip(signature, SIGNATURE_MAX)),
            text: file.and_then(|file| {
                file.line(info.name_span.start.line)
                    .map(|line| line.trim().to_owned())
            }),
            exported: info.exported,
            references: None,
        }
    }

    /// Files of the project in the language of the target.
    ///
    /// # Errors
    ///
    /// Fails when the execution is cancelled.
    pub(crate) fn project_files(
        ctx: &ExecCtx,
        files: &Files<'_>,
        target: &Target,
    ) -> Result<Vec<PathBuf>> {
        let (Some(code), Some(language)) = (ctx.code(), target.language.as_ref()) else {
            return Ok(vec![target.path.clone()]);
        };
        let accept = |path: &Path| code.language_of(Some(path), &[]).as_ref() == Some(language);
        let (mut paths, _) = files.walk(&target.root, &accept)?;
        if !paths.contains(&target.path) {
            paths.insert(0, target.path.clone());
        }
        Ok(paths)
    }

    /// Definitions of the identifier at the target found by syntax
    /// analysis: in the file first, then in the project.
    pub(crate) fn local_definitions(
        ctx: &ExecCtx,
        files: &Files<'_>,
        target: &Target,
    ) -> Result<Vec<(PathBuf, SymbolInfo)>> {
        let Some(word) = target.word.as_deref() else {
            return Ok(Vec::new());
        };
        let (code, language) = require_code(ctx, target.language.as_ref(), &target.path)?;
        let mut found = Vec::new();
        let mut seen = HashSet::new();
        for path in project_files(ctx, files, target)? {
            ctx.checkpoint()?;
            let Some(file) = files.get(&path) else {
                continue;
            };
            if !file.text.contains(word) {
                continue;
            }
            for info in code
                .find_symbol(file.text.as_bytes(), &language, word)
                .unwrap_or_default()
            {
                if seen.insert((path.clone(), info.name_span.start)) {
                    found.push((path.clone(), info));
                }
            }
        }
        Ok(found)
    }

    /// Reference found in a file, with its role.
    pub(crate) struct Occurrence {
        pub(crate) path: PathBuf,
        pub(crate) start: TextPosition,
        pub(crate) end: TextPosition,
        pub(crate) offset: u64,
        pub(crate) role: RecordRole,
        pub(crate) container: Option<String>,
        pub(crate) text: String,
    }

    /// Occurrences in code of the identifier at the target found by syntax
    /// analysis, in the file only or in the whole project.
    pub(crate) fn local_references(
        ctx: &ExecCtx,
        files: &Files<'_>,
        target: &Target,
        whole_project: bool,
    ) -> Result<Vec<Occurrence>> {
        let Some(word) = target.word.as_deref() else {
            return Ok(Vec::new());
        };
        let (code, language) = require_code(ctx, target.language.as_ref(), &target.path)?;
        let paths = if whole_project {
            project_files(ctx, files, target)?
        } else {
            vec![target.path.clone()]
        };
        let mut found = Vec::new();
        for path in paths {
            ctx.checkpoint()?;
            let Some(file) = files.get(&path) else {
                continue;
            };
            if !file.text.contains(word) {
                continue;
            }
            let references = code
                .references(file.text.as_bytes(), &language, word)
                .unwrap_or_default();
            for reference in references
                .into_iter()
                .filter(|item| item.class == SpanClass::Code)
            {
                let role = match reference.role {
                    ReferenceRole::Definition => RecordRole::Definition,
                    ReferenceRole::Import => RecordRole::Import,
                    _ if is_test_path(&path) => RecordRole::Test,
                    _ => RecordRole::Reference,
                };
                found.push(Occurrence {
                    path: path.clone(),
                    start: reference.span.start,
                    end: reference.span.end,
                    offset: reference.span.bytes.start,
                    role,
                    container: reference.container,
                    text: reference.line_text.trim().to_owned(),
                });
            }
        }
        Ok(found)
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
