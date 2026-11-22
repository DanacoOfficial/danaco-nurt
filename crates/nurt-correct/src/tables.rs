// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Correction tables built into the executable and their revision
//! `corr_rev`.
//!
//! # Table format
//!
//! Every table is a TOML document in the format `nurt-correction-table/1`:
//!
//! ```toml
//! format = "nurt-correction-table/1"
//! table = "synonyms"
//! summary = "Synonyms of command, parameter, value, field and function names."
//!
//! [[entry]]
//! rule = "L06"
//! class = "C"
//! kind = "command"
//! from = ["delete", "del", "erase", "rm"]
//! to = "remove"
//! ```
//!
//! | Key | Type | Meaning |
//! |---|---|---|
//! | `format` | string | always `nurt-correction-table/1` |
//! | `table` | string | name of the table, equal to the file stem |
//! | `summary` | string | one sentence in English |
//! | `words` | array of strings | habit tables only: the words of other shells that start a habit |
//! | `entry` | array of tables | the entries |
//! | `entry.rule` | string | rule identifier, `^[LTVBP][0-9]{2}$` |
//! | `entry.class` | string | `C`, `S`, `F` or `P`; absent only when `error` is present |
//! | `entry.error` | string | error code returned instead of a correction (`E_UNCORRECTABLE`, `E_ONE_EFFECT`, …) |
//! | `entry.kind` | string | kind of the written element; in `synonyms` a name kind (`command`, `param`, `value`, `field`, `function`), in `units` one of `size`, `size_letter`, `duration`, `bool`, `instant` |
//! | `entry.from` | array of strings | written forms, compared after folding |
//! | `entry.to` | string | canonical form |
//! | `entry.alternative` | string | `units` only: the other reading of a probable unit |
//! | `entry.problem` | string | `units` only: explanation of an error entry |
//!
//! The translation tables of shell habits (`bash`, `powershell`) and the habit
//! tables (`habits_bash`, `habits_powershell`) use the same header and the
//! same `rule`, `class` and `error` keys; their other keys belong to the
//! translators and are read through [`Table::entries`].
//!
//! # Validation
//!
//! Loading checks the header, the rule and class of every entry, that no
//! written form of `synonyms` and `units` appears twice within one kind, that
//! a synonym differs from its own target and that habit words are unique.
//! Checks against the catalog names are made by [`Tables::check_names`].
//!
//! # Revision
//!
//! `corr_rev` is `sha256:` followed by the lower-case hexadecimal SHA-256
//! digest of the JSON Canonicalization Scheme (RFC 8785) form of a JSON
//! object that maps every table name to the JSON form of its document.
//! Comments, layout and key order of the TOML files do not change the
//! revision; any change of a value does.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt;

use sha2::{Digest, Sha256};
use toml_edit::{Document, Item, Value};

use crate::distance::{
    EFFECTING_MAX_DISTANCE, NameEntry, NameKind, NamePair, NameRole, close_pairs, fold,
};
use crate::policy::Class;
use crate::record::RuleId;

/// Format identifier of correction tables.
pub const FORMAT: &str = "nurt-correction-table/1";

/// Prefix of the table revision.
pub const REVISION_PREFIX: &str = "sha256:";

/// Names of the built-in tables, in the order of the revision.
pub const TABLE_NAMES: [&str; 6] = [
    "bash",
    "habits_bash",
    "habits_powershell",
    "powershell",
    "synonyms",
    "units",
];

const EMBEDDED: [(&str, &str); 6] = [
    ("bash", include_str!("../tables/bash.toml")),
    ("habits_bash", include_str!("../tables/habits_bash.toml")),
    (
        "habits_powershell",
        include_str!("../tables/habits_powershell.toml"),
    ),
    ("powershell", include_str!("../tables/powershell.toml")),
    ("synonyms", include_str!("../tables/synonyms.toml")),
    ("units", include_str!("../tables/units.toml")),
];

const SYNONYMS: &str = "synonyms";
const UNITS: &str = "units";
const HABIT_TABLES: [&str; 2] = ["habits_bash", "habits_powershell"];

/// Kind of an entry of the `units` table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum UnitKind {
    /// Size unit (`mb`, `megabytes`).
    Size,
    /// Single-letter size unit (`M`).
    SizeLetter,
    /// Duration unit (`hours`).
    Duration,
    /// Boolean word (`yes`).
    Bool,
    /// Relative instant (`yesterday`).
    Instant,
}

impl UnitKind {
    /// Every kind, in declaration order.
    pub const ALL: [Self; 5] = [
        Self::Size,
        Self::SizeLetter,
        Self::Duration,
        Self::Bool,
        Self::Instant,
    ];

    /// Name of the kind as written in the table.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Size => "size",
            Self::SizeLetter => "size_letter",
            Self::Duration => "duration",
            Self::Bool => "bool",
            Self::Instant => "instant",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.as_str() == text)
    }
}

/// Entry of a correction table, with the keys common to every table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Rule identifier.
    pub rule: RuleId,
    /// Certainty class; absent for error entries.
    pub class: Option<Class>,
    /// Error code returned instead of a correction.
    pub error: Option<String>,
    /// Kind of the written element.
    pub kind: Option<String>,
    /// Written forms.
    pub from: Vec<String>,
    /// Canonical form.
    pub to: Option<String>,
    /// Other reading of a probable unit.
    pub alternative: Option<String>,
    /// Explanation of an error entry.
    pub problem: Option<String>,
    /// Line of the entry header in the table file, from 1.
    pub line: usize,
}

/// A loaded table: its document and its entries.
#[derive(Debug, Clone)]
pub struct Table {
    name: String,
    document: Document<String>,
    entries: Vec<Entry>,
    words: Vec<String>,
}

impl Table {
    /// Name of the table.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Parsed TOML document, for keys specific to one translator.
    #[must_use]
    pub fn document(&self) -> &Document<String> {
        &self.document
    }

    /// Entries with the common keys, in file order.
    #[must_use]
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// Raw entry tables, in file order, for keys specific to one translator.
    pub fn raw_entries(&self) -> impl Iterator<Item = &toml_edit::Table> {
        self.document
            .get("entry")
            .and_then(Item::as_array_of_tables)
            .into_iter()
            .flat_map(toml_edit::ArrayOfTables::iter)
    }

    /// Habit words declared by the table.
    #[must_use]
    pub fn words(&self) -> &[String] {
        &self.words
    }
}

/// A synonym with its rule and class.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Synonym {
    /// Canonical name.
    pub to: String,
    /// Rule identifier.
    pub rule: RuleId,
    /// Certainty class.
    pub class: Class,
}

/// A unit word with its canonical form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unit {
    /// Canonical unit, value or expression; absent for error entries.
    pub to: Option<String>,
    /// Other reading of a probable unit.
    pub alternative: Option<String>,
    /// Rule identifier.
    pub rule: RuleId,
    /// Certainty class; absent for error entries.
    pub class: Option<Class>,
    /// Error code of an error entry.
    pub error: Option<String>,
    /// Explanation of an error entry.
    pub problem: Option<String>,
}

/// All correction tables, indexed for lookup.
#[derive(Debug, Clone)]
pub struct Tables {
    tables: BTreeMap<String, Table>,
    synonyms: HashMap<(NameKind, String), Synonym>,
    units: HashMap<(UnitKind, String), Unit>,
    habits: BTreeSet<String>,
    revision: String,
}

impl Tables {
    /// Builds the tables from `(name, TOML text)` pairs; a name of
    /// [`TABLE_NAMES`] without text is an empty table.
    ///
    /// # Errors
    ///
    /// Returns [`TableError`] when a text is not valid TOML, a header or an
    /// entry is invalid, or a written form appears twice within one kind.
    pub fn load<'a>(
        sources: impl IntoIterator<Item = (&'a str, &'a str)>,
    ) -> Result<Self, TableError> {
        let mut tables = BTreeMap::new();
        for (name, text) in sources {
            let table = parse_table(name, text)?;
            if tables.insert(name.to_owned(), table).is_some() {
                return Err(TableError::new(name, 0, "table given twice"));
            }
        }
        for name in TABLE_NAMES {
            if !tables.contains_key(name) {
                let header = format!("format = \"{FORMAT}\"\ntable = \"{name}\"\n");
                tables.insert(name.to_owned(), parse_table(name, &header)?);
            }
        }
        Self::index(tables)
    }

    fn index(tables: BTreeMap<String, Table>) -> Result<Self, TableError> {
        let synonyms = index_synonyms(tables.get(SYNONYMS))?;
        let units = index_units(tables.get(UNITS))?;
        let habits = index_habits(&tables)?;
        let revision = revision(&tables);
        Ok(Self {
            tables,
            synonyms,
            units,
            habits,
            revision,
        })
    }

    /// The table of the given name.
    #[must_use]
    pub fn table(&self, name: &str) -> Option<&Table> {
        self.tables.get(name)
    }

    /// Every table, in name order.
    pub fn tables(&self) -> impl Iterator<Item = &Table> {
        self.tables.values()
    }

    /// Synonym of `word` among names of `kind`, after [`fold`].
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_correct::distance::NameKind;
    /// use nurt_correct::tables::load_embedded;
    ///
    /// let tables = load_embedded().unwrap();
    /// assert_eq!(tables.synonym(NameKind::Command, "Delete"), Some("remove"));
    /// assert_eq!(tables.synonym(NameKind::Field, "LastWriteTime"), Some("modified"));
    /// assert_eq!(tables.synonym(NameKind::Function, "length"), Some("len"));
    /// assert_eq!(tables.synonym(NameKind::Field, "length"), Some("size"));
    /// ```
    #[must_use]
    pub fn synonym(&self, kind: NameKind, word: &str) -> Option<&str> {
        self.synonym_entry(kind, word)
            .map(|synonym| synonym.to.as_str())
    }

    /// Synonym of `word` among names of `kind` with its rule and class.
    #[must_use]
    pub fn synonym_entry(&self, kind: NameKind, word: &str) -> Option<&Synonym> {
        self.synonyms.get(&(kind, fold(word)))
    }

    /// Unit word of the given kind, compared in lower case.
    #[must_use]
    pub fn unit(&self, kind: UnitKind, word: &str) -> Option<&Unit> {
        self.units.get(&(kind, word.to_lowercase()))
    }

    /// Words of other shells that start a habit.
    pub fn habits(&self) -> impl Iterator<Item = &str> {
        self.habits.iter().map(String::as_str)
    }

    /// Synonyms and habit words as entries of the comparison set of fuzzy
    /// name matching.
    #[must_use]
    pub fn name_entries(&self) -> Vec<NameEntry> {
        let mut entries: Vec<NameEntry> = self
            .synonyms
            .iter()
            .map(|((kind, word), synonym)| {
                NameEntry::synonym(word.clone(), *kind, synonym.to.clone())
            })
            .chain(
                self.habits
                    .iter()
                    .map(|word| NameEntry::habit(word.clone())),
            )
            .collect();
        entries.sort_by(|a, b| (a.kind, &a.text).cmp(&(b.kind, &b.text)));
        entries
    }

    /// Revision of the tables, `sha256:` and 64 hexadecimal digits.
    #[must_use]
    pub fn corr_rev(&self) -> &str {
        &self.revision
    }

    /// Checks the synonyms and habit words against the names of the catalog
    /// and the language.
    ///
    /// Reports a synonym that is itself a name of its kind, a synonym whose
    /// target is not a name of its kind, and every synonym or habit word at
    /// distance 1 or less from a name of the same kind other than its
    /// target.
    #[must_use]
    pub fn check_names(&self, names: &[NameEntry]) -> Vec<TableIssue> {
        let known: BTreeSet<(NameKind, String)> = names
            .iter()
            .filter(|entry| entry.role == NameRole::Name)
            .map(|entry| (entry.kind, fold(&entry.text)))
            .collect();
        let mut issues = Vec::new();
        for ((kind, word), synonym) in &self.synonyms {
            if known.contains(&(*kind, word.clone())) {
                issues.push(TableIssue::SynonymIsName {
                    kind: *kind,
                    word: word.clone(),
                });
            }
            if !known.contains(&(*kind, fold(&synonym.to))) {
                issues.push(TableIssue::UnknownTarget {
                    kind: *kind,
                    word: word.clone(),
                    to: synonym.to.clone(),
                });
            }
        }
        let mut all: Vec<NameEntry> = names.to_vec();
        all.extend(self.name_entries());
        for pair in close_pairs(&all, EFFECTING_MAX_DISTANCE) {
            let both_names = known.contains(&(pair.kind, fold(&pair.a)))
                && known.contains(&(pair.kind, fold(&pair.b)));
            if pair.distance > 0 && !both_names && !self.is_own_target(&pair) {
                issues.push(TableIssue::Close(pair));
            }
        }
        issues.sort();
        issues
    }

    fn is_own_target(&self, pair: &NamePair) -> bool {
        let target = |word: &str| self.synonym(pair.kind, word).map(fold);
        target(&pair.a) == Some(fold(&pair.b)) || target(&pair.b) == Some(fold(&pair.a))
    }
}

/// Issue found by [`Tables::check_names`].
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum TableIssue {
    /// The synonym is itself a name of its kind.
    SynonymIsName {
        /// Kind of the synonym.
        kind: NameKind,
        /// Written form.
        word: String,
    },
    /// The target of the synonym is not a name of its kind.
    UnknownTarget {
        /// Kind of the synonym.
        kind: NameKind,
        /// Written form.
        word: String,
        /// Target.
        to: String,
    },
    /// A synonym or habit word lies at distance 1 from another name.
    Close(NamePair),
}

impl fmt::Display for TableIssue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SynonymIsName { kind, word } => {
                write!(
                    f,
                    "synonym `{word}` is a {} name; remove the synonym",
                    kind.as_str()
                )
            }
            Self::UnknownTarget { kind, word, to } => write!(
                f,
                "synonym `{word}` points to `{to}`, which is not a {} name",
                kind.as_str()
            ),
            Self::Close(pair) => write!(
                f,
                "`{}` and `{}` are {} edit apart; fuzzy correction between them is a tie",
                pair.a, pair.b, pair.distance
            ),
        }
    }
}

/// Error of a correction table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableError {
    /// Name of the table.
    pub table: String,
    /// Line in the table file, from 1; 0 when unknown.
    pub line: usize,
    /// Description of the problem.
    pub message: String,
}

impl TableError {
    fn new(table: &str, line: usize, message: impl Into<String>) -> Self {
        Self {
            table: table.to_owned(),
            line,
            message: message.into(),
        }
    }
}

impl fmt::Display for TableError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "correction table `{}`", self.table)?;
        if self.line > 0 {
            write!(f, " line {}", self.line)?;
        }
        write!(f, ": {}", self.message)
    }
}

impl std::error::Error for TableError {}

/// Loads the tables built into the executable.
///
/// # Errors
///
/// Returns [`TableError`] when a built-in table is invalid.
///
/// # Examples
///
/// ```
/// let tables = nurt_correct::tables::load_embedded().unwrap();
/// assert!(tables.corr_rev().starts_with("sha256:"));
/// ```
pub fn load_embedded() -> Result<Tables, TableError> {
    Tables::load(EMBEDDED)
}

/// Revision of the built-in tables.
///
/// # Errors
///
/// Returns [`TableError`] when a built-in table is invalid.
pub fn corr_rev() -> Result<String, TableError> {
    load_embedded().map(|tables| tables.revision)
}

fn parse_table(name: &str, text: &str) -> Result<Table, TableError> {
    let document = Document::parse(text.to_owned()).map_err(|error| {
        let line = error.span().map_or(0, |span| line_of(text, span.start));
        TableError::new(name, line, error.message().to_owned())
    })?;
    check_header(name, &document)?;
    let entries = match document.get("entry") {
        None => Vec::new(),
        Some(item) => {
            let array = item
                .as_array_of_tables()
                .ok_or_else(|| TableError::new(name, 0, "`entry` must be an array of tables"))?;
            array
                .iter()
                .map(|table| parse_entry(name, text, table))
                .collect::<Result<_, _>>()?
        }
    };
    let words = match document.get("words") {
        None => Vec::new(),
        Some(item) => strings(name, line_of_item(text, item), item, "words")?,
    };
    Ok(Table {
        name: name.to_owned(),
        document,
        entries,
        words,
    })
}

fn check_header(name: &str, document: &Document<String>) -> Result<(), TableError> {
    let format = document.get("format").and_then(Item::as_str);
    if format != Some(FORMAT) {
        return Err(TableError::new(
            name,
            0,
            format!("`format` must be \"{FORMAT}\""),
        ));
    }
    let table = document.get("table").and_then(Item::as_str);
    if table != Some(name) {
        return Err(TableError::new(
            name,
            0,
            format!("`table` must be \"{name}\""),
        ));
    }
    Ok(())
}

fn parse_entry(name: &str, text: &str, table: &toml_edit::Table) -> Result<Entry, TableError> {
    let line = table.span().map_or(0, |span| line_of(text, span.start));
    let error = |message: String| TableError::new(name, line, message);
    let rule_text =
        string(name, line, table, "rule")?.ok_or_else(|| error("`rule` is missing".into()))?;
    let rule: RuleId = rule_text
        .parse()
        .map_err(|invalid: crate::record::InvalidRuleId| error(invalid.to_string()))?;
    let class = match string(name, line, table, "class")? {
        Some(code) => Some(
            code.parse::<Class>()
                .map_err(|_| error(format!("unknown class `{code}`; use C, S, F or P")))?,
        ),
        None => None,
    };
    let error_code = string(name, line, table, "error")?;
    if class.is_none() && error_code.is_none() {
        return Err(error(
            "`class` is missing; an entry without class needs `error`".into(),
        ));
    }
    let from = match table.get("from") {
        Some(item) => strings(name, line, item, "from")?,
        None => Vec::new(),
    };
    Ok(Entry {
        rule,
        class,
        error: error_code,
        kind: string(name, line, table, "kind")?,
        from,
        to: string(name, line, table, "to")?,
        alternative: string(name, line, table, "alternative")?,
        problem: string(name, line, table, "problem")?,
        line,
    })
}

fn string(
    name: &str,
    line: usize,
    table: &toml_edit::Table,
    key: &str,
) -> Result<Option<String>, TableError> {
    match table.get(key) {
        None => Ok(None),
        Some(item) => item
            .as_str()
            .map(|text| Some(text.to_owned()))
            .ok_or_else(|| TableError::new(name, line, format!("`{key}` must be a string"))),
    }
}

fn strings(name: &str, line: usize, item: &Item, key: &str) -> Result<Vec<String>, TableError> {
    let invalid = || {
        TableError::new(
            name,
            line,
            format!("`{key}` must be a non-empty array of strings"),
        )
    };
    let array = item.as_array().ok_or_else(invalid)?;
    let words: Option<Vec<String>> = array
        .iter()
        .map(|value| value.as_str().map(str::to_owned))
        .collect();
    match words {
        Some(words) if !words.is_empty() => Ok(words),
        _ => Err(invalid()),
    }
}

fn line_of(text: &str, offset: usize) -> usize {
    text.get(..offset)
        .map_or(0, |before| before.matches('\n').count() + 1)
}

fn line_of_item(text: &str, item: &Item) -> usize {
    item.span().map_or(0, |span| line_of(text, span.start))
}

fn index_synonyms(
    table: Option<&Table>,
) -> Result<HashMap<(NameKind, String), Synonym>, TableError> {
    let mut index = HashMap::new();
    let Some(table) = table else {
        return Ok(index);
    };
    for entry in &table.entries {
        let error = |message: String| TableError::new(&table.name, entry.line, message);
        let kind = entry
            .kind
            .as_deref()
            .and_then(NameKind::parse)
            .filter(|kind| *kind != NameKind::Let)
            .ok_or_else(|| {
                error("`kind` must be command, group, param, value, field or function".into())
            })?;
        let to = entry
            .to
            .clone()
            .ok_or_else(|| error("`to` is missing".into()))?;
        let class = entry
            .class
            .ok_or_else(|| error("a synonym needs `class`".into()))?;
        for word in &entry.from {
            let folded = fold(word);
            if folded == fold(&to) {
                return Err(error(format!("synonym `{word}` equals its target")));
            }
            let synonym = Synonym {
                to: to.clone(),
                rule: entry.rule,
                class,
            };
            if index.insert((kind, folded), synonym).is_some() {
                return Err(error(format!(
                    "`{word}` appears twice among {} synonyms",
                    kind.as_str()
                )));
            }
        }
    }
    Ok(index)
}

fn index_units(table: Option<&Table>) -> Result<HashMap<(UnitKind, String), Unit>, TableError> {
    let mut index = HashMap::new();
    let Some(table) = table else {
        return Ok(index);
    };
    for entry in &table.entries {
        let error = |message: String| TableError::new(&table.name, entry.line, message);
        let kind = entry
            .kind
            .as_deref()
            .and_then(UnitKind::parse)
            .ok_or_else(|| {
                error("`kind` must be size, size_letter, duration, bool or instant".into())
            })?;
        if entry.to.is_none() && entry.error.is_none() {
            return Err(error("a unit entry needs `to` or `error`".into()));
        }
        for word in &entry.from {
            let unit = Unit {
                to: entry.to.clone(),
                alternative: entry.alternative.clone(),
                rule: entry.rule,
                class: entry.class,
                error: entry.error.clone(),
                problem: entry.problem.clone(),
            };
            if index.insert((kind, word.to_lowercase()), unit).is_some() {
                return Err(error(format!(
                    "`{word}` appears twice among {} units",
                    kind.as_str()
                )));
            }
        }
    }
    Ok(index)
}

fn index_habits(tables: &BTreeMap<String, Table>) -> Result<BTreeSet<String>, TableError> {
    let mut habits = BTreeSet::new();
    for name in HABIT_TABLES {
        let Some(table) = tables.get(name) else {
            continue;
        };
        let mut own = BTreeSet::new();
        for word in &table.words {
            let folded = fold(word);
            if !own.insert(folded.clone()) {
                return Err(TableError::new(
                    name,
                    0,
                    format!("habit word `{word}` appears twice"),
                ));
            }
            habits.insert(folded);
        }
    }
    Ok(habits)
}

fn revision(tables: &BTreeMap<String, Table>) -> String {
    let object: serde_json::Map<String, serde_json::Value> = tables
        .iter()
        .map(|(name, table)| (name.clone(), table_json(table.document.as_table())))
        .collect();
    let canonical =
        serde_json_canonicalizer::to_vec(&serde_json::Value::Object(object)).unwrap_or_default();
    format!(
        "{REVISION_PREFIX}{}",
        hex::encode(Sha256::digest(&canonical))
    )
}

fn table_json<'a>(pairs: impl IntoIterator<Item = (&'a str, &'a Item)>) -> serde_json::Value {
    serde_json::Value::Object(
        pairs
            .into_iter()
            .filter_map(|(key, item)| item_json(item).map(|value| (key.to_owned(), value)))
            .collect(),
    )
}

fn item_json(item: &Item) -> Option<serde_json::Value> {
    match item {
        Item::None => None,
        Item::Value(value) => Some(value_json(value)),
        Item::Table(table) => Some(table_json(table)),
        Item::ArrayOfTables(array) => Some(serde_json::Value::Array(
            array.iter().map(table_json).collect(),
        )),
    }
}

fn value_json(value: &Value) -> serde_json::Value {
    match value {
        Value::String(text) => serde_json::Value::from(text.value().as_str()),
        Value::Integer(number) => serde_json::Value::from(*number.value()),
        Value::Float(number) => serde_json::Number::from_f64(*number.value()).map_or_else(
            || serde_json::Value::from(number.to_string()),
            serde_json::Value::Number,
        ),
        Value::Boolean(flag) => serde_json::Value::from(*flag.value()),
        Value::Datetime(datetime) => serde_json::Value::from(datetime.value().to_string()),
        Value::Array(array) => serde_json::Value::Array(array.iter().map(value_json).collect()),
        Value::InlineTable(table) => serde_json::Value::Object(
            table
                .iter()
                .map(|(key, value)| (key.to_owned(), value_json(value)))
                .collect(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEADER: &str = "format = \"nurt-correction-table/1\"\ntable = \"synonyms\"\n";

    fn synonyms(body: &str) -> Result<Tables, TableError> {
        let text = format!("{HEADER}{body}");
        Tables::load([("synonyms", text.as_str())])
    }

    #[test]
    fn loads_the_built_in_tables() {
        let tables = load_embedded().unwrap();
        assert_eq!(tables.tables().count(), TABLE_NAMES.len());
        assert_eq!(tables.synonym(NameKind::Command, "work"), Some("jobs"));
        assert_eq!(tables.synonym(NameKind::Value, "dir"), Some("directory"));
        assert_eq!(tables.synonym(NameKind::Command, "remove"), None);
        let unit = tables.unit(UnitKind::SizeLetter, "M").unwrap();
        assert_eq!(unit.to.as_deref(), Some("mib"));
        assert_eq!(unit.alternative.as_deref(), Some("mb"));
        assert_eq!(unit.class, Some(Class::Probable));
        let months = tables.unit(UnitKind::Duration, "months").unwrap();
        assert_eq!(months.error.as_deref(), Some("E_UNCORRECTABLE"));
        assert!(tables.table("synonyms").unwrap().raw_entries().count() > 0);
        assert_eq!(corr_rev().unwrap(), tables.corr_rev());
    }

    #[test]
    fn revision_ignores_layout_and_key_order() {
        let first = synonyms("[[entry]]\nrule = \"L06\"\nclass = \"C\"\nkind = \"command\"\nfrom = [\"rm\"]\nto = \"remove\"\n").unwrap();
        let reordered = synonyms("# comment\n[[entry]]\nto = \"remove\"\nfrom = [ \"rm\" ]\nkind = \"command\"\nclass = \"C\"\nrule = \"L06\"\n").unwrap();
        let changed = synonyms("[[entry]]\nrule = \"L06\"\nclass = \"C\"\nkind = \"command\"\nfrom = [\"rm\"]\nto = \"move\"\n").unwrap();
        assert_eq!(first.corr_rev(), reordered.corr_rev());
        assert_ne!(first.corr_rev(), changed.corr_rev());
        assert_eq!(first.corr_rev().len(), REVISION_PREFIX.len() + 64);
    }

    #[test]
    fn rejects_invalid_headers_and_entries() {
        let wrong_format =
            Tables::load([("units", "format = \"other\"\ntable = \"units\"\n")]).unwrap_err();
        assert!(wrong_format.message.contains("format"));
        let wrong_name = Tables::load([(
            "units",
            "format = \"nurt-correction-table/1\"\ntable = \"bash\"\n",
        )])
        .unwrap_err();
        assert!(wrong_name.message.contains("`table`"));
        let bad_rule = synonyms("[[entry]]\nrule = \"X01\"\nclass = \"C\"\n").unwrap_err();
        assert_eq!(bad_rule.line, 3);
        assert!(
            bad_rule
                .to_string()
                .starts_with("correction table `synonyms` line 3: invalid rule identifier")
        );
        let bad_class = synonyms("[[entry]]\nrule = \"L06\"\nclass = \"Q\"\n").unwrap_err();
        assert!(bad_class.message.contains("unknown class"));
        let no_class = synonyms("[[entry]]\nrule = \"L06\"\n").unwrap_err();
        assert!(no_class.message.contains("needs `error`"));
        let syntax = Tables::load([("units", "format = ")]).unwrap_err();
        assert_eq!(syntax.line, 1);
    }

    #[test]
    fn rejects_duplicate_and_self_synonyms() {
        let twice = synonyms(
            "[[entry]]\nrule = \"L06\"\nclass = \"C\"\nkind = \"command\"\nfrom = [\"rm\"]\nto = \"remove\"\n\
             [[entry]]\nrule = \"L06\"\nclass = \"C\"\nkind = \"command\"\nfrom = [\"RM\"]\nto = \"move\"\n",
        )
        .unwrap_err();
        assert!(twice.message.contains("appears twice"));
        let itself = synonyms("[[entry]]\nrule = \"L06\"\nclass = \"C\"\nkind = \"command\"\nfrom = [\"Remove\"]\nto = \"remove\"\n").unwrap_err();
        assert!(itself.message.contains("equals its target"));
        let bad_kind = synonyms("[[entry]]\nrule = \"L06\"\nclass = \"C\"\nkind = \"let\"\nfrom = [\"x\"]\nto = \"y\"\n").unwrap_err();
        assert!(bad_kind.message.contains("`kind`"));
        let empty_from = synonyms(
            "[[entry]]\nrule = \"L06\"\nclass = \"C\"\nkind = \"command\"\nfrom = []\nto = \"y\"\n",
        )
        .unwrap_err();
        assert!(empty_from.message.contains("non-empty array"));
        let duplicate_table =
            Tables::load([("synonyms", HEADER), ("synonyms", HEADER)]).unwrap_err();
        assert!(duplicate_table.message.contains("twice"));
    }

    #[test]
    fn collects_habit_words_from_both_shells() {
        let bash = "format = \"nurt-correction-table/1\"\ntable = \"habits_bash\"\nwords = [\"ls\", \"grep\"]\n";
        let powershell = "format = \"nurt-correction-table/1\"\ntable = \"habits_powershell\"\nwords = [\"ls\", \"gci\"]\n";
        let tables =
            Tables::load([("habits_bash", bash), ("habits_powershell", powershell)]).unwrap();
        assert_eq!(tables.habits().collect::<Vec<_>>(), ["gci", "grep", "ls"]);
        let repeated = "format = \"nurt-correction-table/1\"\ntable = \"habits_bash\"\nwords = [\"ls\", \"LS\"]\n";
        assert!(Tables::load([("habits_bash", repeated)]).is_err());
        let entries = tables.name_entries();
        assert!(
            entries
                .iter()
                .any(|entry| entry.text == "grep" && entry.role == NameRole::Habit)
        );
    }

    #[test]
    fn checks_synonyms_against_names() {
        let tables = synonyms(
            "[[entry]]\nrule = \"L06\"\nclass = \"C\"\nkind = \"command\"\nfrom = [\"head\", \"find\"]\nto = \"first\"\n\
             [[entry]]\nrule = \"L06\"\nclass = \"C\"\nkind = \"command\"\nfrom = [\"wrok\"]\nto = \"work\"\n",
        )
        .unwrap();
        let names = ["first", "read", "find", "jobs"].map(|name| NameEntry::command(name, false));
        let issues = tables.check_names(&names);
        let text: Vec<String> = issues.iter().map(ToString::to_string).collect();
        assert_eq!(
            text,
            [
                "synonym `find` is a command name; remove the synonym",
                "synonym `wrok` points to `work`, which is not a command name",
                "`head` and `read` are 1 edit apart; fuzzy correction between them is a tie",
            ]
        );
    }

    #[test]
    fn converts_every_toml_value_to_json() {
        let text = "format = \"nurt-correction-table/1\"\ntable = \"units\"\n\
                    number = 1\nratio = 0.5\nflag = true\nwhen = 2026-11-21\nlist = [1, \"a\"]\n\
                    inline = { a = 1 }\n[nested]\nkey = \"v\"\n";
        let tables = Tables::load([("units", text)]).unwrap();
        let json = table_json(tables.table("units").unwrap().document().as_table());
        assert_eq!(json["number"], 1);
        assert_eq!(json["ratio"], 0.5);
        assert_eq!(json["flag"], true);
        assert_eq!(json["when"], "2026-11-21");
        assert_eq!(json["list"][1], "a");
        assert_eq!(json["inline"]["a"], 1);
        assert_eq!(json["nested"]["key"], "v");
    }
}
