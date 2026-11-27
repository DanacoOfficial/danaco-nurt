// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-programs::catalog::index
// Responsibility:   Reading of a catalog directory: the index, its included parts, the front.
// Dependencies:     serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Reading of a catalog directory: the index, its included parts, the front
//! matter of its guides and the consistency of references.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Path, PathBuf};

use serde_json::value::RawValue;
use serde_json::{Map, Value};

use crate::catalog::diag::{At, Diagnostic, DiagnosticCode, Diagnostics};
use crate::catalog::entries;
use crate::catalog::fields::{Fault, Fields, Findings, Pattern, len};
use crate::catalog::guide::{self, FrontMatter};
use crate::catalog::json::{self, JsonError, RawObject};
use crate::catalog::paths::{PathError, Root, check_relpath};
use crate::catalog::types::{
    Catalog, DEFAULT_LANGUAGE, FORMAT_MAJOR, FORMAT_MINOR, FORMAT_PREFIX, GUIDE_EXTENSION,
    GuideHeader, INDEX_FILE, Origin, ReadOptions, is_valid_catalog_id, is_valid_language,
};

const CATALOG_ID: Pattern = Pattern {
    check: is_valid_catalog_id,
    rule: "use lower-case letters, digits, `.`, `_` and `-`, starting with a letter or digit, at most 64 characters",
};
const LANGUAGE: Pattern = Pattern {
    check: is_valid_language,
    rule: "use a language tag such as `en` or `pt-BR`",
};
const MAX_INCLUDES: usize = 256;
const MAX_LANGUAGES: usize = 256;
const MAX_SYNONYM_GROUPS: usize = 5000;
const SYNONYM_WORDS: std::ops::RangeInclusive<usize> = 2..=16;

/// Members of the index read separately from the header.
const LIST_MEMBERS: [&str; 4] = ["format", "categories", "programs", "guides"];

/// A catalog with the findings made while reading it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedCatalog {
    /// Objects that satisfy the format.
    pub catalog: Catalog,
    /// Findings, in the order they were made.
    pub diagnostics: Vec<Diagnostic>,
}

/// Reason a catalog cannot be read as a whole.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CatalogErrorKind {
    /// The directory or `catalog.json` cannot be read.
    Unreadable,
    /// `catalog.json` is not valid UTF-8 JSON.
    Syntax,
    /// `format` names a major version the reader does not implement.
    Format,
    /// `catalog.json` exceeds a limit of the format.
    Limit,
    /// A required member of the index is missing or has the wrong type.
    Structure,
}

/// A catalog that cannot be read as a whole.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogError {
    /// Kind of the failure.
    pub kind: CatalogErrorKind,
    /// File relative to the catalog root.
    pub file: String,
    /// Line in the file, when known.
    pub line: Option<u64>,
    /// What is wrong and how to correct it.
    pub message: String,
}

impl CatalogError {
    fn new(kind: CatalogErrorKind, line: Option<u64>, message: impl Into<String>) -> Self {
        Self {
            kind,
            file: INDEX_FILE.to_owned(),
            line,
            message: message.into(),
        }
    }

    fn from_json(error: &JsonError) -> Self {
        let kind = match error {
            JsonError::Io(_) => CatalogErrorKind::Unreadable,
            JsonError::TooLarge(_) | JsonError::TooDeep(_) => CatalogErrorKind::Limit,
            JsonError::Encoding | JsonError::Syntax { .. } => CatalogErrorKind::Syntax,
        };
        Self::new(kind, error.line(), error.message())
    }
}

impl fmt::Display for CatalogError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.file)?;
        if let Some(line) = self.line {
            write!(f, ":{line}")?;
        }
        write!(f, ": {}", self.message)
    }
}

impl std::error::Error for CatalogError {}

/// Reads the catalog in the directory `path`, or the catalog whose
/// `catalog.json` is `path`.
///
/// Objects that violate the format are skipped and reported in
/// [`LoadedCatalog::diagnostics`]; with [`ReadOptions::strict`] the findings
/// have the severities of validation.
///
/// # Errors
///
/// Fails when the directory or `catalog.json` cannot be read, is not JSON,
/// exceeds a limit, lacks `format`, `categories` or `programs`, or names a
/// major version other than 1.
pub fn read_catalog(path: &Path, options: &ReadOptions) -> Result<LoadedCatalog, CatalogError> {
    let dir = catalog_dir(path);
    let root = Root::open(&dir).map_err(|error| {
        CatalogError::new(
            CatalogErrorKind::Unreadable,
            None,
            format!("catalog directory cannot be opened: {error}"),
        )
    })?;
    let mut reader = Reader::new(root, options);
    reader.read()?;
    Ok(reader.finish())
}

fn catalog_dir(path: &Path) -> PathBuf {
    let names_index = path.file_name().is_some_and(|name| name == INDEX_FILE);
    match path.parent() {
        Some(parent) if names_index && path.is_file() => parent.to_path_buf(),
        _ => path.to_path_buf(),
    }
}

/// State of the reading of one catalog.
pub(crate) struct Reader<'o> {
    pub(crate) options: &'o ReadOptions,
    pub(crate) root: Root,
    pub(crate) diags: Diagnostics,
    pub(crate) catalog: Catalog,
    total_bytes: u64,
    includes: Vec<String>,
}

impl<'o> Reader<'o> {
    fn new(root: Root, options: &'o ReadOptions) -> Self {
        let catalog = Catalog {
            root: root.path().to_path_buf(),
            language: DEFAULT_LANGUAGE.to_owned(),
            ..Catalog::default()
        };
        Self {
            options,
            root,
            diags: Diagnostics::new(options.strict),
            catalog,
            total_bytes: 0,
            includes: Vec::new(),
        }
    }

    fn finish(self) -> LoadedCatalog {
        LoadedCatalog {
            catalog: self.catalog,
            diagnostics: self.diags.into_vec(),
        }
    }

    fn read(&mut self) -> Result<(), CatalogError> {
        let text = self.read_index()?;
        let members = json::parse_object(&text, self.options.limits.depth)
            .map_err(|error| CatalogError::from_json(&error))?;
        read_format(&members)?;
        let categories = required_array(&text, &members, "categories")?;
        let programs = required_array(&text, &members, "programs")?;
        self.read_header(&text, &members);
        self.read_categories(&text, &categories);
        self.read_programs(INDEX_FILE, &text, &programs);
        self.read_parts(&text, &members);
        self.resolve_aliases();
        self.read_standalone_guides(&text, &members);
        self.read_guide_headers();
        self.read_overlays();
        Ok(())
    }

    fn read_index(&mut self) -> Result<String, CatalogError> {
        let path = self.root.resolve(INDEX_FILE).map_err(|error| match error {
            PathError::Missing => CatalogError::new(
                CatalogErrorKind::Unreadable,
                None,
                "catalog.json is missing; the catalog root must contain the index file",
            ),
            other => CatalogError::new(CatalogErrorKind::Unreadable, None, other.to_string()),
        })?;
        let text = json::read_text(&path, self.options.limits.index_bytes)
            .map_err(|error| CatalogError::from_json(&error))?;
        self.total_bytes = u64::try_from(text.len()).unwrap_or(u64::MAX);
        Ok(text)
    }

    /// Records the findings about one object; returns `true` when the
    /// object satisfies the format.
    pub(crate) fn report(
        &mut self,
        file: &str,
        line: Option<u64>,
        name: Option<&str>,
        findings: Findings,
    ) -> bool {
        let valid = findings.faults.is_empty();
        let at = |field: String| At::new(file, line).field(field).name(name);
        for Fault { field, message } in findings.faults {
            self.diags
                .push(DiagnosticCode::EntryInvalid, at(field), message);
        }
        for Fault { field, message } in findings.unknown {
            self.diags
                .push(DiagnosticCode::UnknownField, at(field), message);
        }
        for field in findings.cleaned {
            self.diags.push(
                DiagnosticCode::ControlCharacter,
                at(field),
                "control characters were removed; keep only line feeds and tabs in texts",
            );
        }
        valid
    }

    /// Records one violation.
    pub(crate) fn invalid(&mut self, at: At, message: impl Into<String>) {
        self.diags.push(DiagnosticCode::EntryInvalid, at, message);
    }

    fn read_header(&mut self, text: &str, members: &RawObject<'_>) {
        let minor = members
            .get("format_minor")
            .and_then(|raw| serde_json::from_str::<u64>(raw.get()).ok());
        if minor.is_some_and(|minor| minor > FORMAT_MINOR) {
            self.diags.set_newer_minor();
        }
        let map: Map<String, Value> = members
            .iter()
            .filter(|(key, _)| !LIST_MEMBERS.contains(&key.as_str()))
            .filter_map(|(key, raw)| Some((key.clone(), serde_json::from_str(raw.get()).ok()?)))
            .collect();
        let mut fields = Fields::new(&map, "");
        let catalog = &mut self.catalog;
        catalog.format_minor = fields
            .integer("format_minor", 0)
            .and_then(|minor| u64::try_from(minor).ok())
            .unwrap_or(0);
        catalog.id = fields.pattern("id", CATALOG_ID);
        catalog.title = fields.text("title", len(0, 120));
        catalog.version = fields.text("version", len(0, 60));
        if let Some(language) = fields.pattern("language", LANGUAGE) {
            catalog.language = language;
        }
        catalog.languages = fields.pattern_list("languages", MAX_LANGUAGES, LANGUAGE, true);
        catalog.license = fields.text("license", len(0, 200));
        catalog.synonyms = read_synonyms(&mut fields);
        self.includes = read_includes(&mut fields);
        fields.extension();
        let findings = fields.finish();
        for (line, group) in split_by_member(text, members, findings) {
            self.report(INDEX_FILE, line, None, group);
        }
    }

    fn read_categories(&mut self, text: &str, items: &[&RawValue]) {
        let limit = self.options.limits.categories;
        for (index, raw) in items.iter().enumerate() {
            let number = json::line_at(text, json::offset_of(text, raw.get()));
            let line = Some(number);
            let pointer = format!("categories[{index}]");
            if index >= limit {
                let message = format!("more than {limit} categories; merge or remove categories");
                self.invalid(At::new(INDEX_FILE, line).field(pointer), message);
                break;
            }
            let Some(value) = parse_value(raw) else {
                continue;
            };
            let mut fields = match Fields::of(&value, pointer.clone()) {
                Ok(fields) => fields,
                Err(fault) => {
                    self.invalid(At::new(INDEX_FILE, line).field(fault.field), fault.message);
                    continue;
                }
            };
            let mut category = entries::read_category(&mut fields);
            let findings = fields.finish();
            let name = category.id.clone();
            if !self.report(INDEX_FILE, line, Some(&name), findings) {
                continue;
            }
            if let Some(first) = self.catalog.category(&name) {
                let message = format!(
                    "category `{name}` is already declared at {}; keep one",
                    first.origin
                );
                self.invalid(
                    At::new(INDEX_FILE, line)
                        .field(format!("{pointer}.id"))
                        .name(Some(&name)),
                    message,
                );
                continue;
            }
            category.origin = Origin {
                file: INDEX_FILE.to_owned(),
                line: number,
                pointer,
            };
            self.catalog.categories.push(category);
        }
        self.check_parents();
    }

    fn check_parents(&mut self) {
        let parents: BTreeMap<String, Option<String>> = self
            .catalog
            .categories
            .iter()
            .map(|category| (category.id.clone(), category.parent.clone()))
            .collect();
        let mut rejected = Vec::new();
        for category in &self.catalog.categories {
            let Some(parent) = &category.parent else {
                continue;
            };
            let message = match parents.get(parent) {
                None => format!(
                    "parent `{parent}` is not declared in `categories`; declare it or correct the identifier"
                ),
                Some(Some(_)) => format!(
                    "parent `{parent}` is itself a subcategory; categories have at most two levels"
                ),
                Some(None) => continue,
            };
            rejected.push((category.id.clone(), category.origin.clone(), message));
        }
        for (id, origin, message) in rejected {
            let at = At::new(&origin.file, Some(origin.line))
                .field(format!("{}.parent", origin.pointer))
                .name(Some(&id));
            self.invalid(at, message);
            self.catalog.categories.retain(|category| category.id != id);
        }
    }

    /// Reads program entries of the index or of an included part.
    pub(crate) fn read_programs(&mut self, file: &str, text: &str, items: &[&RawValue]) {
        for (index, raw) in items.iter().enumerate() {
            let number = json::line_at(text, json::offset_of(text, raw.get()));
            let line = Some(number);
            let pointer = format!("programs[{index}]");
            let at = At::new(file, line).field(pointer.clone());
            if self.catalog.programs.len() >= self.options.limits.programs {
                let limit = self.options.limits.programs;
                self.invalid(at, format!("more than {limit} programs; split the catalog"));
                break;
            }
            if raw.get().len() > self.options.limits.entry_bytes {
                let limit = self.options.limits.entry_bytes;
                self.invalid(
                    at,
                    format!("entry is larger than {limit} bytes; shorten its texts"),
                );
                continue;
            }
            let Some(value) = parse_value(raw) else {
                continue;
            };
            let mut fields = match Fields::of(&value, pointer.clone()) {
                Ok(fields) => fields,
                Err(fault) => {
                    self.invalid(At::new(file, line).field(fault.field), fault.message);
                    continue;
                }
            };
            let mut program = entries::read_program(&mut fields);
            if !program.category.is_empty() && self.catalog.category(&program.category).is_none() {
                let path = fields.path("category");
                fields.fault(
                    path,
                    format!("category `{}` is not declared in `categories`; declare it or correct the identifier", program.category),
                );
            }
            let findings = fields.finish();
            let name = program.name.clone();
            if !self.report(file, line, Some(&name), findings) {
                continue;
            }
            if let Some(first) = self.catalog.program(&name) {
                let message = format!(
                    "program `{name}` is already defined at {}; keep one entry",
                    first.origin
                );
                self.invalid(
                    At::new(file, line)
                        .field(format!("{pointer}.name"))
                        .name(Some(&name)),
                    message,
                );
                continue;
            }
            program.origin = Origin {
                file: file.to_owned(),
                line: number,
                pointer,
            };
            self.catalog.programs.push(program);
        }
    }

    fn read_parts(&mut self, index_text: &str, members: &RawObject<'_>) {
        let include_line = members
            .get("include")
            .map(|raw| json::line_at(index_text, json::offset_of(index_text, raw.get())));
        for (position, part) in std::mem::take(&mut self.includes).into_iter().enumerate() {
            let at = At::new(INDEX_FILE, include_line)
                .field(format!("include[{position}]"))
                .name(Some(&part));
            let path = match self.root.resolve(&part) {
                Ok(path) => path,
                Err(error) => {
                    self.invalid(at, error.to_string());
                    continue;
                }
            };
            let size = json::file_size(&path).unwrap_or(0);
            if self.total_bytes.saturating_add(size) > self.options.limits.total_bytes {
                let limit = self.options.limits.total_bytes;
                self.invalid(
                    at,
                    format!("index and parts exceed {limit} bytes in total; reduce the catalog"),
                );
                continue;
            }
            let text = match json::read_text(&path, self.options.limits.part_bytes) {
                Ok(text) => text,
                Err(error) => {
                    self.invalid(At::new(&part, error.line()), error.message());
                    continue;
                }
            };
            self.total_bytes = self.total_bytes.saturating_add(size);
            self.read_part(&part, &text);
        }
    }

    fn read_part(&mut self, part: &str, text: &str) {
        let members = match json::parse_object(text, self.options.limits.depth) {
            Ok(members) => members,
            Err(error) => {
                self.invalid(At::new(part, error.line()), error.message());
                return;
            }
        };
        for (key, raw) in &members {
            let line = Some(json::line_at(text, json::offset_of(text, raw.get())));
            match key.as_str() {
                "programs" => {}
                "x" if raw.get().trim_start().starts_with('{') => {}
                "x" => self.invalid(At::new(part, line).field("x"), "must be an object"),
                _ => self.diags.push(
                    DiagnosticCode::UnknownField,
                    At::new(part, line).field(key.clone()),
                    "member is not defined for included parts; a part holds `programs` and `x`",
                ),
            }
        }
        let Some(raw) = members.get("programs") else {
            self.invalid(
                At::new(part, Some(1)).field("programs"),
                "member `programs` is required; add it",
            );
            return;
        };
        match json::parse_array(text, raw) {
            Ok(items) => {
                self.read_programs(part, text, &items);
                self.catalog.parts.push(part.to_owned());
            }
            Err(error) => self.invalid(
                At::new(part, error.line()).field("programs"),
                "must be an array",
            ),
        }
    }

    /// Drops every alias whose name equals a program name or an earlier
    /// alias, in document order.
    fn resolve_aliases(&mut self) {
        let names: BTreeSet<String> = self
            .catalog
            .programs
            .iter()
            .map(|program| program.name.clone())
            .collect();
        let mut seen: BTreeSet<String> = BTreeSet::new();
        let mut collisions = Vec::new();
        for program in &mut self.catalog.programs {
            let origin = program.origin.clone();
            let owner = program.name.clone();
            program.aliases.retain(|alias| {
                let kept = !names.contains(&alias.name) && seen.insert(alias.name.clone());
                if !kept {
                    collisions.push((origin.clone(), owner.clone(), alias.name.clone()));
                }
                kept
            });
        }
        for (origin, owner, alias) in collisions {
            let at = At::new(&origin.file, Some(origin.line))
                .field(format!("{}.aliases", origin.pointer))
                .name(Some(&owner));
            self.invalid(at, format!("alias `{alias}` collides with a program or an earlier alias; rename or remove it"));
        }
    }

    fn read_standalone_guides(&mut self, text: &str, members: &RawObject<'_>) {
        let Some(raw) = members.get("guides") else {
            return;
        };
        let items = match json::parse_array(text, raw) {
            Ok(items) => items,
            Err(error) => {
                self.invalid(
                    At::new(INDEX_FILE, error.line()).field("guides"),
                    "must be an array",
                );
                return;
            }
        };
        let taken: BTreeSet<String> = self
            .catalog
            .programs
            .iter()
            .flat_map(|program| {
                std::iter::once(program.name.clone())
                    .chain(program.aliases.iter().map(|alias| alias.name.clone()))
            })
            .collect();
        for (index, raw) in items.iter().enumerate() {
            let line = Some(json::line_at(text, json::offset_of(text, raw.get())));
            let pointer = format!("guides[{index}]");
            if index >= self.options.limits.guides {
                let limit = self.options.limits.guides;
                self.invalid(
                    At::new(INDEX_FILE, line).field(pointer),
                    format!("more than {limit} standalone guides"),
                );
                break;
            }
            let Some(value) = parse_value(raw) else {
                continue;
            };
            let mut fields = match Fields::of(&value, pointer.clone()) {
                Ok(fields) => fields,
                Err(fault) => {
                    self.invalid(At::new(INDEX_FILE, line).field(fault.field), fault.message);
                    continue;
                }
            };
            let guide = entries::read_standalone_guide(&mut fields);
            if taken.contains(&guide.name) || self.catalog.standalone_guide(&guide.name).is_some() {
                let path = fields.path("name");
                fields.fault(path, format!("name `{}` is already used by a program, an alias or a guide; rename the guide", guide.name));
            }
            if let Some(category) = guide
                .category
                .as_deref()
                .filter(|id| self.catalog.category(id).is_none())
            {
                let path = fields.path("category");
                fields.fault(
                    path,
                    format!("category `{category}` is not declared in `categories`"),
                );
            }
            let findings = fields.finish();
            let name = guide.name.clone();
            if self.report(INDEX_FILE, line, Some(&name), findings) {
                self.catalog.guides.push(guide);
            }
        }
    }

    /// Reads the front matter of every guide named by programs and
    /// standalone guides; references to unusable guides are removed.
    fn read_guide_headers(&mut self) {
        let mut references: BTreeMap<String, Vec<Origin>> = BTreeMap::new();
        for program in &self.catalog.programs {
            if let Some(guide) = &program.guide {
                references
                    .entry(guide.clone())
                    .or_default()
                    .push(program.origin.clone());
            }
        }
        for guide in &self.catalog.guides {
            references.entry(guide.guide.clone()).or_default();
        }
        let mut unusable = BTreeSet::new();
        for (path, origins) in references {
            match self.read_guide_header(&path, &origins) {
                Some(header) => {
                    self.catalog.guide_headers.insert(path, header);
                }
                None => {
                    unusable.insert(path);
                }
            }
        }
        for program in &mut self.catalog.programs {
            if program
                .guide
                .as_ref()
                .is_some_and(|guide| unusable.contains(guide))
            {
                program.guide = None;
            }
        }
        self.catalog
            .guides
            .retain(|guide| !unusable.contains(&guide.guide));
        if self.options.strict {
            self.check_guide_programs();
        }
    }

    /// Reads the front matter of one guide; `origins` are the entries that
    /// name it, used to locate a missing file.
    pub(crate) fn read_guide_header(
        &mut self,
        path: &str,
        origins: &[Origin],
    ) -> Option<GuideHeader> {
        let file = match self.root.resolve(path) {
            Ok(file) => file,
            Err(error) => {
                let message = format!("guide `{path}`: {error}");
                match origins.first() {
                    Some(origin) => {
                        let at = At::new(&origin.file, Some(origin.line))
                            .field(format!("{}.guide", origin.pointer));
                        self.invalid(at, message);
                    }
                    None => self.invalid(At::new(INDEX_FILE, None).name(Some(path)), message),
                }
                return None;
            }
        };
        let (front, faults) = match guide::read_head(&file, &self.options.limits) {
            Ok(head) => head,
            Err(message) => {
                self.invalid(At::new(path, Some(1)), message);
                return None;
            }
        };
        if !faults.is_empty() {
            for fault in faults {
                self.invalid(
                    At::new(path, Some(1)).field(format!("front matter `{}`", fault.key)),
                    fault.message,
                );
            }
            return None;
        }
        let name = guide_name(path);
        self.check_front(path, &name, &front);
        Some(GuideHeader {
            name,
            path: path.to_owned(),
            front,
        })
    }

    fn check_front(&mut self, path: &str, name: &str, front: &FrontMatter) {
        if front.version.is_none() {
            self.diags.push(
                DiagnosticCode::GuideVersionMissing,
                At::new(path, Some(1)).field("front matter `version`"),
                "front matter has no `version`; the version of the program entry is used",
            );
        }
        if front
            .name
            .as_deref()
            .is_some_and(|declared| declared != name)
        {
            self.diags.push(
                DiagnosticCode::GuideMismatch,
                At::new(path, Some(1)).field("front matter `name`"),
                format!("`name` must equal the file name without `.md`, `{name}`"),
            );
        }
    }

    fn check_guide_programs(&mut self) {
        let mut findings = Vec::new();
        for (path, header) in &self.catalog.guide_headers {
            if header.front.programs.is_empty() {
                continue;
            }
            for listed in &header.front.programs {
                let names_guide = self
                    .catalog
                    .program(listed)
                    .and_then(|program| program.guide.as_deref())
                    == Some(path.as_str());
                if !names_guide {
                    findings.push((
                        path.clone(),
                        format!("front matter lists `{listed}`, which does not name this guide"),
                    ));
                }
            }
            for program in &self.catalog.programs {
                if program.guide.as_deref() == Some(path.as_str())
                    && !header.front.programs.contains(&program.name)
                {
                    findings.push((
                        path.clone(),
                        format!(
                            "`{}` names this guide but is not listed in `programs`",
                            program.name
                        ),
                    ));
                }
            }
        }
        for (path, message) in findings {
            self.diags.push(
                DiagnosticCode::GuideMismatch,
                At::new(&path, Some(1)).field("front matter `programs`"),
                message,
            );
        }
    }
}

fn read_format(members: &RawObject<'_>) -> Result<(), CatalogError> {
    let unsupported = |line| {
        CatalogError::new(
            CatalogErrorKind::Format,
            line,
            "`format` must be `nurt-program-catalog/1`",
        )
    };
    let Some(raw) = members.get("format") else {
        return Err(CatalogError::new(
            CatalogErrorKind::Structure,
            Some(1),
            "member `format` is required; add \"format\": \"nurt-program-catalog/1\"",
        ));
    };
    let format: String = serde_json::from_str(raw.get()).map_err(|_| unsupported(None))?;
    let major = format
        .strip_prefix(FORMAT_PREFIX)
        .ok_or_else(|| unsupported(None))?;
    match major.parse::<u64>() {
        Ok(FORMAT_MAJOR) => Ok(()),
        Ok(other) => Err(CatalogError::new(
            CatalogErrorKind::Format,
            None,
            format!(
                "format version {other} is not supported; this reader implements version {FORMAT_MAJOR}"
            ),
        )),
        Err(_) => Err(unsupported(None)),
    }
}

fn required_array<'a>(
    text: &'a str,
    members: &RawObject<'a>,
    key: &str,
) -> Result<Vec<&'a RawValue>, CatalogError> {
    let Some(raw) = members.get(key) else {
        return Err(CatalogError::new(
            CatalogErrorKind::Structure,
            Some(1),
            format!("member `{key}` is required; add it, possibly as an empty array"),
        ));
    };
    json::parse_array(text, raw).map_err(|error| {
        CatalogError::new(
            CatalogErrorKind::Structure,
            error.line(),
            format!("member `{key}` must be an array"),
        )
    })
}

fn parse_value(raw: &RawValue) -> Option<Value> {
    serde_json::from_str(raw.get()).ok()
}

fn read_includes(fields: &mut Fields<'_>) -> Vec<String> {
    let Some((items, path)) = fields.array("include", MAX_INCLUDES) else {
        return Vec::new();
    };
    let mut includes: Vec<String> = Vec::new();
    for (index, item) in items.iter().enumerate() {
        let item_path = format!("{path}[{index}]");
        let Value::String(part) = item else {
            fields.fault(item_path, "must be text");
            continue;
        };
        if let Err(error) = check_relpath(part) {
            fields.fault(item_path, error.to_string());
        } else if part == INDEX_FILE || includes.contains(part) {
            fields.fault(
                item_path,
                format!("`{part}` is the index or is listed twice; list each part once"),
            );
        } else {
            includes.push(part.clone());
        }
    }
    includes
}

fn read_synonyms(fields: &mut Fields<'_>) -> Vec<Vec<String>> {
    let Some((groups, path)) = fields.array("synonyms", MAX_SYNONYM_GROUPS) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (index, group) in groups.iter().enumerate() {
        let group_path = format!("{path}[{index}]");
        let Value::Array(words) = group else {
            fields.fault(group_path, "must be a list of 2 to 16 words");
            continue;
        };
        if !SYNONYM_WORDS.contains(&words.len()) {
            fields.fault(group_path, "must be a list of 2 to 16 words");
            continue;
        }
        let texts: Vec<String> = words
            .iter()
            .enumerate()
            .filter_map(|(word, value)| {
                fields.text_value(value, format!("{group_path}[{word}]"), len(1, 60))
            })
            .collect();
        if texts.len() == words.len() {
            out.push(texts);
        }
    }
    out
}

/// Findings about members of the index header, each with the line of the
/// member it concerns.
fn split_by_member(
    text: &str,
    members: &RawObject<'_>,
    findings: Findings,
) -> Vec<(Option<u64>, Findings)> {
    let line_of = |field: &str| {
        let member = field.split(['.', '[']).next().unwrap_or(field);
        members
            .get(member)
            .map(|raw| json::line_at(text, json::offset_of(text, raw.get())))
    };
    let mut grouped: BTreeMap<String, (Option<u64>, Findings)> = BTreeMap::new();
    for fault in findings.faults {
        grouped
            .entry(fault.field.clone())
            .or_insert_with(|| (line_of(&fault.field), Findings::default()))
            .1
            .faults
            .push(fault);
    }
    for fault in findings.unknown {
        grouped
            .entry(fault.field.clone())
            .or_insert_with(|| (line_of(&fault.field), Findings::default()))
            .1
            .unknown
            .push(fault);
    }
    for field in findings.cleaned {
        grouped
            .entry(field.clone())
            .or_insert_with(|| (line_of(&field), Findings::default()))
            .1
            .cleaned
            .push(field);
    }
    grouped.into_values().collect()
}

/// Name of the guide at `path`: the file name without `.md`.
pub(crate) fn guide_name(path: &str) -> String {
    let file = path.rsplit('/').next().unwrap_or(path);
    file.strip_suffix(GUIDE_EXTENSION)
        .unwrap_or(file)
        .to_owned()
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
