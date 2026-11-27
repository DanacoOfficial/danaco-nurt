// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-correct::powershell::table
// Responsibility:   The PowerShell translation table: cmdlets, parameters, variants of the.
// Dependencies:     nurt_contracts, toml_edit
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! The PowerShell translation table: cmdlets, parameters, variants of the
//! translation, object properties and comparison operators.
//!
//! The table is read from the correction tables `powershell` and
//! `habits_powershell` (format `nurt-correction-table/1`). Every key is
//! checked: an unknown key, a missing required key or a value of the wrong
//! type is a [`TableError`].
//!
//! # Cmdlet entries
//!
//! Every `[[entry]]` describes one cmdlet or Windows program. `from` lists
//! its name and aliases; `shared` lists the aliases that are also commands of
//! another shell or of Nurt. `[[entry.param]]` describes a parameter:
//!
//! | Key | Meaning |
//! |---|---|
//! | `name`, `aliases` | parameter name and aliases; unique prefixes are accepted as in PowerShell |
//! | `position` | position of an unnamed argument, from 0 |
//! | `switch` | the parameter takes no value |
//! | `type` | conversion of the value: `path`, `text`, `texts`, `line_text`, `int`, `depth`, `upto`, `seconds`, `milliseconds`, `fields`, `field`, `enum`, `enum_text`, `name`, `name_glob`, `regex`, `map`, `value`, `program`, `arguments`, `type_check`, `context`, `new_name`, `content_path`, `output`, `cim_equals` (a CIM filter that compares the property named as the parameter with the value), `int_equals` (a condition that compares the Nurt field named by `arg` with a whole number; use with `emit = "filter"`) |
//! | `arg`, `positional` | the Nurt argument the value becomes |
//! | `stage`, `emit` | the Nurt stage that receives the argument; `emit` appends a stage of its own |
//! | `emit_with` | the emitted stage receives the converted value of the named parameter instead of its own |
//! | `set` | value of the argument given by a switch |
//! | `values` | PowerShell values and the Nurt values that replace them |
//! | `drop` | the parameter is removed; the text explains why |
//! | `error` | the parameter has no Nurt form; the text explains it |
//!
//! `[[entry.variant]]` lists the translations in order of preference; the
//! first variant whose conditions hold is used. A variant names its rule and
//! class (or an error code with `problem`, `message`, `fix` and `help`), the
//! Nurt stages it produces and the arguments it adds.

use std::sync::LazyLock;

use nurt_contracts::ast::BinOp;
use toml_edit::{Item, TableLike, Value};

use crate::policy::Class;
use crate::record::RuleId;
use crate::tables::{TableError, Tables};

/// Name of the translation table.
pub const TABLE_NAME: &str = "powershell";

/// Name of the habit table.
pub const HABITS_TABLE_NAME: &str = "habits_powershell";

const EMBEDDED_TABLE: &str = include_str!("../../tables/powershell.toml");
const EMBEDDED_HABITS: &str = include_str!("../../tables/habits_powershell.toml");

static EMBEDDED: LazyLock<Result<PowerShellTable, TableError>> =
    LazyLock::new(|| PowerShellTable::parse(EMBEDDED_TABLE, EMBEDDED_HABITS));

/// The PowerShell translation table.
#[derive(Debug, Clone)]
pub struct PowerShellTable {
    pub(crate) effect_commands: Vec<String>,
    pub(crate) path_rule: RuleId,
    pub(crate) syntax: SyntaxRules,
    pub(crate) registry: Registry,
    pub(crate) dates: Dates,
    pub(crate) wildcard: Wildcard,
    pub(crate) operators: Vec<Operator>,
    pub(crate) comparison: Comparison,
    pub(crate) records: Vec<Record>,
    pub(crate) common: Vec<Param>,
    pub(crate) cmdlets: Vec<Cmdlet>,
    pub(crate) verbs: Vec<String>,
}

impl PowerShellTable {
    /// The table built into the executable.
    ///
    /// # Errors
    ///
    /// Returns [`TableError`] when the built-in table is invalid.
    pub fn embedded() -> Result<&'static Self, TableError> {
        EMBEDDED.as_ref().map_err(Clone::clone)
    }

    /// Reads the table from the texts of the translation table and of the
    /// habit table.
    ///
    /// # Errors
    ///
    /// Returns [`TableError`] when a text is not a valid correction table or
    /// a key of the translation is missing, unknown or of the wrong type.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_correct::powershell::PowerShellTable;
    ///
    /// let table = PowerShellTable::parse(
    ///     "format = \"nurt-correction-table/1\"\ntable = \"powershell\"\npath_rule = \"L19\"",
    ///     "format = \"nurt-correction-table/1\"\ntable = \"habits_powershell\"",
    /// );
    /// assert!(table.is_err());
    /// ```
    pub fn parse(table: &str, habits: &str) -> Result<Self, TableError> {
        let tables = Tables::load([(TABLE_NAME, table), (HABITS_TABLE_NAME, habits)])?;
        Self::from_tables(&tables)
    }

    /// Reads the table from loaded correction tables.
    ///
    /// # Errors
    ///
    /// As [`PowerShellTable::parse`].
    pub fn from_tables(tables: &Tables) -> Result<Self, TableError> {
        let missing = |name: &str| error(name, format!("table `{name}` is missing"));
        let table = tables
            .table(TABLE_NAME)
            .ok_or_else(|| missing(TABLE_NAME))?;
        let habits = tables
            .table(HABITS_TABLE_NAME)
            .ok_or_else(|| missing(HABITS_TABLE_NAME))?;
        let root = Node::new(TABLE_NAME, "table", table.document().as_table());
        root.allow(&[
            "format",
            "table",
            "summary",
            "effect_commands",
            "path_rule",
            "syntax",
            "registry",
            "dates",
            "wildcard",
            "operator",
            "comparison",
            "record",
            "common",
            "entry",
        ])?;
        let habits_root = Node::new(HABITS_TABLE_NAME, "table", habits.document().as_table());
        Ok(Self {
            effect_commands: root.strings("effect_commands")?,
            path_rule: root.rule("path_rule")?,
            syntax: SyntaxRules::read(&root.child("syntax")?)?,
            registry: Registry::read(&root.child("registry")?)?,
            dates: Dates::read(&root.child("dates")?)?,
            wildcard: Wildcard::read(&root.child("wildcard")?)?,
            operators: root
                .children("operator")?
                .iter()
                .map(Operator::read)
                .collect::<Result<_, _>>()?,
            comparison: Comparison::read(&root.child("comparison")?)?,
            records: root
                .children("record")?
                .iter()
                .map(Record::read)
                .collect::<Result<_, _>>()?,
            common: root
                .children("common")?
                .iter()
                .map(Param::read)
                .collect::<Result<_, _>>()?,
            cmdlets: root
                .children("entry")?
                .iter()
                .map(Cmdlet::read)
                .collect::<Result<_, _>>()?,
            verbs: habits_root.opt_strings("verbs")?.unwrap_or_default(),
        })
    }

    /// The cmdlet entry for a written command name, case-insensitive.
    pub(crate) fn cmdlet(&self, name: &str) -> Option<&Cmdlet> {
        self.cmdlets.iter().find(|cmdlet| {
            cmdlet
                .names
                .iter()
                .any(|known| known.eq_ignore_ascii_case(name))
        })
    }

    /// Whether a word has the form `Verb-Noun` with an approved verb.
    pub(crate) fn is_cmdlet_form(&self, word: &str) -> bool {
        let Some((verb, noun)) = word.split_once('-') else {
            return false;
        };
        !noun.is_empty()
            && noun.chars().all(|c| c.is_ascii_alphanumeric())
            && self
                .verbs
                .iter()
                .any(|known| known.eq_ignore_ascii_case(verb))
    }

    pub(crate) fn record(&self, name: &str) -> Option<&Record> {
        self.records.iter().find(|record| record.name == name)
    }

    pub(crate) fn operator(&self, name: &str) -> Option<&Operator> {
        self.operators.iter().find(|operator| {
            operator
                .names
                .iter()
                .any(|known| known.eq_ignore_ascii_case(name))
        })
    }

    pub(crate) fn is_effect(&self, command: &str) -> bool {
        self.effect_commands.iter().any(|known| known == command)
    }

    /// Every Nurt command name the table produces, for consistency checks.
    #[must_use]
    pub fn command_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .cmdlets
            .iter()
            .flat_map(|cmdlet| {
                cmdlet
                    .variants
                    .iter()
                    .flat_map(|variant| variant.stages.iter().chain(variant.merge.iter()))
                    .chain(cmdlet.params.iter().filter_map(|param| param.emit.as_ref()))
                    .cloned()
            })
            .chain([
                self.wildcard.command.clone(),
                self.syntax.output_command.clone(),
            ])
            .chain([self.syntax.environment_command.clone()])
            .collect();
        names.sort();
        names.dedup();
        names
    }

    /// Every `(command, parameter)` pair the table writes, for consistency
    /// checks against the catalog.
    #[must_use]
    pub fn argument_names(&self) -> Vec<(String, String)> {
        let mut pairs = Vec::new();
        for cmdlet in &self.cmdlets {
            for variant in &cmdlet.variants {
                let first = variant.stages.first().or(variant.merge.as_ref());
                let stage_of = |stage: &Option<String>| stage.as_ref().or(first).cloned();
                for set in &variant.set {
                    if let (Some(stage), Some(arg)) = (stage_of(&set.stage), &set.arg) {
                        pairs.push((stage, arg.clone()));
                    }
                }
                for absent in &variant.absent {
                    if let (Some(stage), Some(arg)) = (first.cloned(), &absent.arg) {
                        pairs.push((stage, arg.clone()));
                    }
                }
                let excluded = |name: &str| {
                    variant
                        .unless
                        .iter()
                        .any(|known| known.eq_ignore_ascii_case(name))
                        || variant
                            .drop
                            .iter()
                            .any(|(known, _)| known.eq_ignore_ascii_case(name))
                };
                let named = |param: &&Param| {
                    !excluded(&param.name) && param.ty != Some(ValueType::IntEquals)
                };
                for param in cmdlet.params.iter().filter(named) {
                    let arg = variant
                        .args
                        .iter()
                        .find(|(name, _)| *name == param.name)
                        .map(|(_, arg)| arg)
                        .or(param.arg.as_ref());
                    let stage = param.emit.clone().or_else(|| stage_of(&param.stage));
                    let in_variant = param.emit.is_some()
                        || param
                            .stage
                            .as_ref()
                            .is_none_or(|stage| variant.stages.contains(stage));
                    if let (Some(stage), Some(arg), true) = (stage, arg, in_variant) {
                        pairs.push((stage, arg.clone()));
                    }
                }
            }
        }
        pairs.push((
            self.syntax.environment_command.clone(),
            self.syntax.environment_param.clone(),
        ));
        for set in &self.wildcard.set {
            if let Some(arg) = &set.arg {
                pairs.push((self.wildcard.command.clone(), arg.clone()));
            }
        }
        pairs.push((
            self.wildcard.command.clone(),
            self.wildcard.glob_arg.clone(),
        ));
        pairs.sort();
        pairs.dedup();
        pairs
    }

    /// Every `(record, field)` pair of the property mapping.
    #[must_use]
    pub fn field_names(&self) -> Vec<(String, String)> {
        self.records
            .iter()
            .flat_map(|record| {
                record
                    .fields
                    .iter()
                    .map(|field| (record.name.clone(), field.field.clone()))
                    .chain(
                        record
                            .self_field
                            .iter()
                            .map(|field| (record.name.clone(), field.clone())),
                    )
            })
            .collect()
    }
}

/// Rules of PowerShell syntax that is not a cmdlet.
#[derive(Debug, Clone)]
pub(crate) struct SyntaxRules {
    pub(crate) assignment: RuleId,
    pub(crate) environment: RuleId,
    pub(crate) call: RuleId,
    pub(crate) output: RuleId,
    pub(crate) output_command: String,
    pub(crate) environment_command: String,
    pub(crate) environment_param: String,
}

impl SyntaxRules {
    fn read(node: &Node<'_>) -> Result<Self, TableError> {
        node.allow(&[
            "assignment",
            "environment",
            "call",
            "output",
            "output_command",
            "environment_command",
            "environment_param",
        ])?;
        Ok(Self {
            assignment: node.rule("assignment")?,
            environment: node.rule("environment")?,
            call: node.rule("call")?,
            output: node.rule("output")?,
            output_command: node.string("output_command")?,
            environment_command: node.string("environment_command")?,
            environment_param: node.string("environment_param")?,
        })
    }
}

/// Registry drives and the Nurt registry path scheme.
#[derive(Debug, Clone)]
pub(crate) struct Registry {
    pub(crate) scheme: String,
    pub(crate) provider: String,
    pub(crate) drives: Vec<(String, String)>,
}

impl Registry {
    fn read(node: &Node<'_>) -> Result<Self, TableError> {
        node.allow(&["scheme", "provider", "drives"])?;
        Ok(Self {
            scheme: node.string("scheme")?,
            provider: node.string("provider")?,
            drives: node.pairs("drives")?,
        })
    }

    /// Canonical hive name of a registry drive or hive name.
    pub(crate) fn hive(&self, drive: &str) -> Option<&str> {
        self.drives
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(drive))
            .map(|(_, hive)| hive.as_str())
    }
}

/// Date arithmetic of `(Get-Date).AddDays(-n)`.
#[derive(Debug, Clone)]
pub(crate) struct Dates {
    pub(crate) cmdlet: String,
    pub(crate) now: String,
    pub(crate) ago: String,
    pub(crate) methods: Vec<(String, String)>,
}

impl Dates {
    fn read(node: &Node<'_>) -> Result<Self, TableError> {
        node.allow(&["cmdlet", "now", "ago", "methods"])?;
        Ok(Self {
            cmdlet: node.string("cmdlet")?,
            now: node.string("now")?,
            ago: node.string("ago")?,
            methods: node.pairs("methods")?,
        })
    }

    pub(crate) fn unit(&self, method: &str) -> Option<&str> {
        self.methods
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(method))
            .map(|(_, unit)| unit.as_str())
    }
}

/// Selecting stage written before a command whose path has a wildcard.
#[derive(Debug, Clone)]
pub(crate) struct Wildcard {
    pub(crate) command: String,
    pub(crate) glob_arg: String,
    pub(crate) set: Vec<SetArg>,
    pub(crate) absent: Vec<Absent>,
}

impl Wildcard {
    fn read(node: &Node<'_>) -> Result<Self, TableError> {
        node.allow(&["command", "glob_arg", "set", "absent"])?;
        Ok(Self {
            command: node.string("command")?,
            glob_arg: node.string("glob_arg")?,
            set: node
                .inline_tables("set")?
                .iter()
                .map(SetArg::read)
                .collect::<Result<_, _>>()?,
            absent: node
                .inline_tables("absent")?
                .iter()
                .map(Absent::read)
                .collect::<Result<_, _>>()?,
        })
    }
}

/// How a string comparison of PowerShell, which ignores case, is kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Fold {
    /// Both sides are compared in lower case.
    Lower,
    /// The regular expression gets the flag `i`.
    Flag,
    /// The case-sensitive comparison of Nurt is kept as an assumption.
    Assume,
}

/// Pattern language of the right operand.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Pattern {
    /// `-like` wildcards.
    Wildcard,
    /// `-match` regular expression.
    Regex,
}

/// A comparison operator of PowerShell.
#[derive(Debug, Clone)]
pub(crate) struct Operator {
    pub(crate) names: Vec<String>,
    pub(crate) op: BinOp,
    pub(crate) pattern: Option<Pattern>,
    pub(crate) fold: Option<Fold>,
    pub(crate) swap: bool,
    pub(crate) negate: bool,
}

impl Operator {
    fn read(node: &Node<'_>) -> Result<Self, TableError> {
        node.allow(&["names", "op", "pattern", "fold", "swap", "negate"])?;
        let op_text = node.string("op")?;
        let op = [
            BinOp::Eq,
            BinOp::Ne,
            BinOp::Lt,
            BinOp::Le,
            BinOp::Gt,
            BinOp::Ge,
            BinOp::Match,
            BinOp::NotMatch,
            BinOp::In,
        ]
        .into_iter()
        .find(|op| op.as_str() == op_text)
        .ok_or_else(|| node.error(format!("unknown operator `{op_text}`")))?;
        let pattern = match node.opt_string("pattern")?.as_deref() {
            None => None,
            Some("wildcard") => Some(Pattern::Wildcard),
            Some("regex") => Some(Pattern::Regex),
            Some(other) => return Err(node.error(format!("unknown pattern `{other}`"))),
        };
        let fold = match node.opt_string("fold")?.as_deref() {
            None => None,
            Some("lower") => Some(Fold::Lower),
            Some("flag") => Some(Fold::Flag),
            Some("assume") => Some(Fold::Assume),
            Some(other) => return Err(node.error(format!("unknown fold `{other}`"))),
        };
        Ok(Self {
            names: node.strings("names")?,
            op,
            pattern,
            fold,
            swap: node.flag("swap")?,
            negate: node.flag("negate")?,
        })
    }
}

/// Texts of the assumptions of comparisons.
#[derive(Debug, Clone)]
pub(crate) struct Comparison {
    pub(crate) text_assumption: String,
    pub(crate) order_assumption: String,
    pub(crate) prefix_assumption: String,
}

impl Comparison {
    fn read(node: &Node<'_>) -> Result<Self, TableError> {
        node.allow(&["text_assumption", "order_assumption", "prefix_assumption"])?;
        Ok(Self {
            text_assumption: node.string("text_assumption")?,
            order_assumption: node.string("order_assumption")?,
            prefix_assumption: node.string("prefix_assumption")?,
        })
    }
}

/// Properties of a kind of PowerShell object and the Nurt record fields that
/// replace them.
#[derive(Debug, Clone)]
pub(crate) struct Record {
    pub(crate) name: String,
    pub(crate) self_field: Option<String>,
    pub(crate) fields: Vec<FieldMap>,
}

impl Record {
    fn read(node: &Node<'_>) -> Result<Self, TableError> {
        node.allow(&["name", "self", "fields"])?;
        Ok(Self {
            name: node.string("name")?,
            self_field: node.opt_string("self")?,
            fields: node
                .inline_tables("fields")?
                .iter()
                .map(FieldMap::read)
                .collect::<Result<_, _>>()?,
        })
    }

    pub(crate) fn field(&self, property: &str) -> Option<&FieldMap> {
        self.fields
            .iter()
            .find(|field| field.property.eq_ignore_ascii_case(property))
    }

    pub(crate) fn property_names(&self) -> Vec<&str> {
        self.fields
            .iter()
            .map(|field| field.property.as_str())
            .collect()
    }
}

/// A property and its Nurt field.
#[derive(Debug, Clone)]
pub(crate) struct FieldMap {
    pub(crate) property: String,
    pub(crate) field: String,
    pub(crate) ty: Option<String>,
    pub(crate) prefix: Option<String>,
    pub(crate) function: Option<String>,
    pub(crate) equals: Option<String>,
    pub(crate) values: Vec<(String, String)>,
}

impl FieldMap {
    fn read(node: &Node<'_>) -> Result<Self, TableError> {
        node.allow(&[
            "property", "field", "type", "prefix", "function", "equals", "values",
        ])?;
        Ok(Self {
            property: node.string("property")?,
            field: node.string("field")?,
            ty: node.opt_string("type")?,
            prefix: node.opt_string("prefix")?,
            function: node.opt_string("function")?,
            equals: node.opt_string("equals")?,
            values: node.opt_pairs("values")?,
        })
    }

    pub(crate) fn is_text(&self) -> bool {
        self.ty.as_deref() == Some("text")
    }
}

/// Handler of a cmdlet whose arguments are not bound by name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Handler {
    /// `Where-Object`.
    Where,
    /// `ForEach-Object`.
    ForEach,
    /// `attrib +r F`.
    Attrib,
    /// `icacls P`.
    Icacls,
}

/// A cmdlet or Windows program.
#[derive(Debug, Clone)]
pub(crate) struct Cmdlet {
    pub(crate) rule: RuleId,
    pub(crate) class: Option<Class>,
    pub(crate) names: Vec<String>,
    pub(crate) shared: Vec<String>,
    pub(crate) output: Option<String>,
    pub(crate) handler: Option<Handler>,
    pub(crate) presets: Vec<(String, Vec<(String, String)>)>,
    pub(crate) raw_args: bool,
    pub(crate) params: Vec<Param>,
    pub(crate) variants: Vec<Variant>,
}

impl Cmdlet {
    fn read(node: &Node<'_>) -> Result<Self, TableError> {
        node.allow(&[
            "rule", "class", "error", "kind", "from", "shared", "output", "handler", "presets",
            "raw_args", "param", "variant",
        ])?;
        let names = node.strings("from")?;
        let node = node.named(names.first().map_or("", String::as_str));
        let handler = match node.opt_string("handler")?.as_deref() {
            None => None,
            Some("where") => Some(Handler::Where),
            Some("foreach") => Some(Handler::ForEach),
            Some("attrib") => Some(Handler::Attrib),
            Some("icacls") => Some(Handler::Icacls),
            Some(other) => return Err(node.error(format!("unknown handler `{other}`"))),
        };
        let presets = match node.get("presets") {
            None => Vec::new(),
            Some(item) => {
                let table = item
                    .as_table_like()
                    .ok_or_else(|| node.error("`presets` must be a table".to_owned()))?;
                table
                    .iter()
                    .map(|(alias, values)| {
                        let inner = values.as_table_like().ok_or_else(|| {
                            node.error(format!("preset `{alias}` must be a table"))
                        })?;
                        Ok((
                            alias.to_owned(),
                            Node::new(node.table, &node.at, inner).all_pairs()?,
                        ))
                    })
                    .collect::<Result<_, TableError>>()?
            }
        };
        let cmdlet = Self {
            rule: node.rule("rule")?,
            class: node.opt_class("class")?,
            shared: node.opt_strings("shared")?.unwrap_or_default(),
            output: node.opt_string("output")?,
            handler,
            presets,
            raw_args: node.flag("raw_args")?,
            params: node
                .children("param")?
                .iter()
                .map(Param::read)
                .collect::<Result<_, _>>()?,
            variants: node
                .children("variant")?
                .iter()
                .map(Variant::read)
                .collect::<Result<_, _>>()?,
            names,
        };
        if cmdlet.variants.is_empty() && cmdlet.handler.is_none() {
            return Err(node.error("a cmdlet needs a variant or a handler".to_owned()));
        }
        Ok(cmdlet)
    }

    pub(crate) fn name(&self) -> &str {
        self.names.first().map_or("", String::as_str)
    }

    pub(crate) fn is_shared(&self, word: &str) -> bool {
        self.shared
            .iter()
            .any(|known| known.eq_ignore_ascii_case(word))
    }

    pub(crate) fn preset(&self, word: &str) -> &[(String, String)] {
        self.presets
            .iter()
            .find(|(alias, _)| alias.eq_ignore_ascii_case(word))
            .map_or(&[], |(_, values)| values.as_slice())
    }
}

/// Conversion of a parameter value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ValueType {
    Path,
    Text,
    Texts,
    LineText,
    Int,
    Depth,
    Upto,
    Seconds,
    Milliseconds,
    Fields,
    Field,
    Enum,
    EnumText,
    Name,
    NameGlob,
    Regex,
    Map,
    Value,
    Program,
    Arguments,
    TypeCheck,
    Context,
    NewName,
    ContentPath,
    Output,
    CimEquals,
    /// A condition that compares the Nurt field named by `arg` with a whole number.
    IntEquals,
}

impl ValueType {
    const NAMES: [(&'static str, Self); 27] = [
        ("path", Self::Path),
        ("text", Self::Text),
        ("texts", Self::Texts),
        ("line_text", Self::LineText),
        ("int", Self::Int),
        ("depth", Self::Depth),
        ("upto", Self::Upto),
        ("seconds", Self::Seconds),
        ("milliseconds", Self::Milliseconds),
        ("fields", Self::Fields),
        ("field", Self::Field),
        ("enum", Self::Enum),
        ("enum_text", Self::EnumText),
        ("name", Self::Name),
        ("name_glob", Self::NameGlob),
        ("regex", Self::Regex),
        ("map", Self::Map),
        ("value", Self::Value),
        ("program", Self::Program),
        ("arguments", Self::Arguments),
        ("type_check", Self::TypeCheck),
        ("context", Self::Context),
        ("new_name", Self::NewName),
        ("content_path", Self::ContentPath),
        ("output", Self::Output),
        ("cim_equals", Self::CimEquals),
        ("int_equals", Self::IntEquals),
    ];
}

/// Special meaning of a parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Role {
    Path,
    Include,
    Exclude,
    NoNewline,
    ChildName,
    RegistryName,
    Selector,
    Program,
    Arguments,
    Location,
    Preview,
    ErrorAction,
}

impl Role {
    const NAMES: [(&'static str, Self); 12] = [
        ("path", Self::Path),
        ("include", Self::Include),
        ("exclude", Self::Exclude),
        ("no_newline", Self::NoNewline),
        ("child_name", Self::ChildName),
        ("registry_name", Self::RegistryName),
        ("selector", Self::Selector),
        ("program", Self::Program),
        ("arguments", Self::Arguments),
        ("location", Self::Location),
        ("preview", Self::Preview),
        ("error_action", Self::ErrorAction),
    ];
}

/// A constant value written by the table.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum SetValue {
    Bool(bool),
    Int(i64),
    Ident(String),
    Text(String),
    Call(String),
}

/// A parameter of a cmdlet.
#[derive(Debug, Clone)]
pub(crate) struct Param {
    pub(crate) name: String,
    pub(crate) aliases: Vec<String>,
    pub(crate) position: Option<usize>,
    pub(crate) switch: bool,
    pub(crate) ty: Option<ValueType>,
    pub(crate) role: Option<Role>,
    pub(crate) rule: Option<RuleId>,
    pub(crate) arg: Option<String>,
    pub(crate) positional: bool,
    pub(crate) stage: Option<String>,
    pub(crate) emit: Option<String>,
    pub(crate) emit_with: Option<String>,
    pub(crate) set: Option<SetValue>,
    pub(crate) values: Vec<(String, String)>,
    pub(crate) drop: Option<String>,
    pub(crate) error: Option<String>,
    pub(crate) wildcards: bool,
    pub(crate) from_pipeline: bool,
    pub(crate) literal_when: Option<String>,
    pub(crate) order_assumption: bool,
    pub(crate) required: Option<String>,
    pub(crate) assumption: Option<String>,
}

impl Param {
    fn read(node: &Node<'_>) -> Result<Self, TableError> {
        node.allow(&[
            "name",
            "aliases",
            "position",
            "switch",
            "type",
            "role",
            "rule",
            "arg",
            "positional",
            "stage",
            "emit",
            "emit_with",
            "set",
            "values",
            "drop",
            "error",
            "wildcards",
            "from_pipeline",
            "literal_when",
            "order_assumption",
            "required",
            "assumption",
        ])?;
        let name = node.string("name")?;
        let node = node.named(&name);
        let ty = match node.opt_string("type")? {
            None => None,
            Some(text) => Some(
                ValueType::NAMES
                    .iter()
                    .find(|(known, _)| *known == text)
                    .map(|(_, ty)| *ty)
                    .ok_or_else(|| node.error(format!("unknown type `{text}`")))?,
            ),
        };
        let role = match node.opt_string("role")? {
            None => None,
            Some(text) => Some(
                Role::NAMES
                    .iter()
                    .find(|(known, _)| *known == text)
                    .map(|(_, role)| *role)
                    .ok_or_else(|| node.error(format!("unknown role `{text}`")))?,
            ),
        };
        let switch = node.flag("switch")?;
        if !switch && ty.is_none() && role.is_none() {
            return Err(node.error("a parameter that is not a switch needs a type".to_owned()));
        }
        let position = match node.opt_int("position")? {
            None => None,
            Some(position) => Some(
                usize::try_from(position)
                    .map_err(|_| node.error("`position` must not be negative".to_owned()))?,
            ),
        };
        Ok(Self {
            aliases: node.opt_strings("aliases")?.unwrap_or_default(),
            position,
            switch,
            ty,
            role,
            rule: node.opt_rule("rule")?,
            arg: node.opt_string("arg")?,
            positional: node.flag("positional")?,
            stage: node.opt_string("stage")?,
            emit: node.opt_string("emit")?,
            emit_with: node.opt_string("emit_with")?,
            set: node.opt_set_value("set")?,
            values: node.opt_pairs("values")?,
            drop: node.opt_string("drop")?,
            error: node.opt_string("error")?,
            wildcards: node.flag("wildcards")?,
            from_pipeline: node.flag("from_pipeline")?,
            literal_when: node.opt_string("literal_when")?,
            order_assumption: node.flag("order_assumption")?,
            required: node.opt_string("required")?,
            assumption: node.opt_string("assumption")?,
            name,
        })
    }

    /// Whether the written name is the parameter name or one of its
    /// aliases.
    pub(crate) fn is_named(&self, written: &str) -> bool {
        self.name.eq_ignore_ascii_case(written)
            || self
                .aliases
                .iter()
                .any(|alias| alias.eq_ignore_ascii_case(written))
    }

    /// Whether the written name is a prefix of the name or of an alias.
    pub(crate) fn has_prefix(&self, written: &str) -> bool {
        let lower = written.to_ascii_lowercase();
        std::iter::once(&self.name)
            .chain(&self.aliases)
            .any(|name| name.to_ascii_lowercase().starts_with(&lower))
    }

    /// The Nurt value for a PowerShell value of the `values` map.
    pub(crate) fn mapped_value(&self, written: &str) -> Option<&str> {
        self.values
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(written))
            .map(|(_, value)| value.as_str())
    }
}

/// An argument added with a constant value.
#[derive(Debug, Clone)]
pub(crate) struct SetArg {
    pub(crate) stage: Option<String>,
    pub(crate) arg: Option<String>,
    pub(crate) value: SetValue,
}

impl SetArg {
    fn read(node: &Node<'_>) -> Result<Self, TableError> {
        node.allow(&["stage", "arg", "ident", "text", "bool", "int", "call"])?;
        Ok(Self {
            stage: node.opt_string("stage")?,
            arg: node.opt_string("arg")?,
            value: node.typed_value()?,
        })
    }
}

/// An argument added when none of the listed parameters is given.
#[derive(Debug, Clone)]
pub(crate) struct Absent {
    pub(crate) params: Vec<String>,
    pub(crate) arg: Option<String>,
    pub(crate) value: Option<SetValue>,
    pub(crate) self_field: bool,
}

impl Absent {
    fn read(node: &Node<'_>) -> Result<Self, TableError> {
        node.allow(&[
            "params",
            "arg",
            "ident",
            "text",
            "bool",
            "int",
            "self_field",
        ])?;
        let self_field = node.flag("self_field")?;
        Ok(Self {
            params: node.strings("params")?,
            arg: node.opt_string("arg")?,
            value: if self_field {
                None
            } else {
                Some(node.typed_value()?)
            },
            self_field,
        })
    }
}

/// A translation of a cmdlet under conditions.
#[derive(Debug, Clone)]
pub(crate) struct Variant {
    pub(crate) rule: RuleId,
    pub(crate) class: Option<Class>,
    pub(crate) error: Option<String>,
    pub(crate) when: Vec<String>,
    pub(crate) when_any: Vec<String>,
    pub(crate) unless: Vec<String>,
    pub(crate) when_value: Vec<(String, String)>,
    pub(crate) piped: Option<bool>,
    pub(crate) continued: Option<bool>,
    pub(crate) wildcard: Option<bool>,
    pub(crate) registry: Option<bool>,
    pub(crate) stages: Vec<String>,
    pub(crate) recursive: bool,
    pub(crate) set: Vec<SetArg>,
    pub(crate) absent: Vec<Absent>,
    pub(crate) args: Vec<(String, String)>,
    pub(crate) drop: Vec<(String, String)>,
    pub(crate) output: Option<String>,
    pub(crate) assumption: Option<String>,
    pub(crate) problem: Option<String>,
    pub(crate) message: Option<String>,
    pub(crate) fix: Vec<String>,
    pub(crate) help: Option<String>,
    pub(crate) wildcard_arg: Option<String>,
    pub(crate) merge: Option<String>,
    pub(crate) merge_next: bool,
}

impl Variant {
    fn read(node: &Node<'_>) -> Result<Self, TableError> {
        node.allow(&[
            "rule",
            "class",
            "error",
            "when",
            "when_any",
            "unless",
            "when_value",
            "piped",
            "continued",
            "wildcard",
            "registry",
            "stages",
            "recursive",
            "set",
            "absent",
            "args",
            "drop",
            "output",
            "assumption",
            "problem",
            "message",
            "fix",
            "help",
            "wildcard_arg",
            "merge",
            "merge_next",
        ])?;
        let class = node.opt_class("class")?;
        let error = node.opt_string("error")?;
        if class.is_none() == error.is_none() {
            return Err(node.error("a variant needs exactly one of `class` and `error`".to_owned()));
        }
        if error.is_some() && node.opt_string("message")?.is_none() {
            return Err(node.error("an error variant needs `message`".to_owned()));
        }
        if class == Some(Class::Probable) && node.opt_string("assumption")?.is_none() {
            return Err(node.error("a probable variant needs `assumption`".to_owned()));
        }
        Ok(Self {
            rule: node.rule("rule")?,
            class,
            error,
            when: node.opt_strings("when")?.unwrap_or_default(),
            when_any: node.opt_strings("when_any")?.unwrap_or_default(),
            unless: node.opt_strings("unless")?.unwrap_or_default(),
            when_value: node.opt_pairs("when_value")?,
            piped: node.opt_bool("piped")?,
            continued: node.opt_bool("continued")?,
            wildcard: node.opt_bool("wildcard")?,
            registry: node.opt_bool("registry")?,
            stages: node.opt_strings("stages")?.unwrap_or_default(),
            recursive: node.flag("recursive")?,
            set: node
                .inline_tables("set")?
                .iter()
                .map(SetArg::read)
                .collect::<Result<_, _>>()?,
            absent: node
                .inline_tables("absent")?
                .iter()
                .map(Absent::read)
                .collect::<Result<_, _>>()?,
            args: node.opt_pairs("args")?,
            drop: node.opt_pairs("drop")?,
            output: node.opt_string("output")?,
            assumption: node.opt_string("assumption")?,
            problem: node.opt_string("problem")?,
            message: node.opt_string("message")?,
            fix: node.opt_strings("fix")?.unwrap_or_default(),
            help: node.opt_string("help")?,
            wildcard_arg: node.opt_string("wildcard_arg")?,
            merge: node.opt_string("merge")?,
            merge_next: node.flag("merge_next")?,
        })
    }
}

fn error(table: &str, message: String) -> TableError {
    TableError {
        table: table.to_owned(),
        line: 0,
        message,
    }
}

/// A table of the document with the place it was read from, for messages.
struct Node<'a> {
    table: &'a str,
    at: String,
    inner: &'a dyn TableLike,
}

impl<'a> Node<'a> {
    fn new(table: &'a str, at: &str, inner: &'a dyn TableLike) -> Self {
        Self {
            table,
            at: at.to_owned(),
            inner,
        }
    }

    fn named(&self, name: &str) -> Self {
        Self {
            table: self.table,
            at: format!("{} `{name}`", self.at),
            inner: self.inner,
        }
    }

    fn error(&self, message: String) -> TableError {
        error(self.table, format!("{}: {message}", self.at))
    }

    fn get(&self, key: &str) -> Option<&'a Item> {
        self.inner.get(key)
    }

    fn allow(&self, keys: &[&str]) -> Result<(), TableError> {
        match self.inner.iter().find(|(key, _)| !keys.contains(key)) {
            Some((key, _)) => Err(self.error(format!("unknown key `{key}`"))),
            None => Ok(()),
        }
    }

    fn opt_value(&self, key: &str) -> Option<&'a Value> {
        self.get(key).and_then(Item::as_value)
    }

    fn opt_string(&self, key: &str) -> Result<Option<String>, TableError> {
        match self.get(key) {
            None => Ok(None),
            Some(item) => item
                .as_str()
                .map(|text| Some(text.to_owned()))
                .ok_or_else(|| self.error(format!("`{key}` must be a string"))),
        }
    }

    fn string(&self, key: &str) -> Result<String, TableError> {
        self.opt_string(key)?
            .ok_or_else(|| self.error(format!("`{key}` is missing")))
    }

    fn opt_bool(&self, key: &str) -> Result<Option<bool>, TableError> {
        match self.get(key) {
            None => Ok(None),
            Some(item) => item
                .as_bool()
                .map(Some)
                .ok_or_else(|| self.error(format!("`{key}` must be a boolean"))),
        }
    }

    fn flag(&self, key: &str) -> Result<bool, TableError> {
        Ok(self.opt_bool(key)?.unwrap_or(false))
    }

    fn opt_int(&self, key: &str) -> Result<Option<i64>, TableError> {
        match self.get(key) {
            None => Ok(None),
            Some(item) => item
                .as_integer()
                .map(Some)
                .ok_or_else(|| self.error(format!("`{key}` must be an integer"))),
        }
    }

    fn opt_strings(&self, key: &str) -> Result<Option<Vec<String>>, TableError> {
        let Some(item) = self.get(key) else {
            return Ok(None);
        };
        let array = item
            .as_array()
            .ok_or_else(|| self.error(format!("`{key}` must be an array of strings")))?;
        array
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| self.error(format!("`{key}` must be an array of strings")))
            })
            .collect::<Result<Vec<_>, _>>()
            .map(Some)
    }

    fn strings(&self, key: &str) -> Result<Vec<String>, TableError> {
        self.opt_strings(key)?
            .ok_or_else(|| self.error(format!("`{key}` is missing")))
    }

    fn opt_rule(&self, key: &str) -> Result<Option<RuleId>, TableError> {
        match self.opt_string(key)? {
            None => Ok(None),
            Some(text) => text
                .parse()
                .map(Some)
                .map_err(|_| self.error(format!("`{key}` is not a rule identifier: `{text}`"))),
        }
    }

    fn rule(&self, key: &str) -> Result<RuleId, TableError> {
        self.opt_rule(key)?
            .ok_or_else(|| self.error(format!("`{key}` is missing")))
    }

    fn opt_class(&self, key: &str) -> Result<Option<Class>, TableError> {
        match self.opt_string(key)? {
            None => Ok(None),
            Some(text) => text
                .parse::<Class>()
                .map(Some)
                .map_err(|_| self.error(format!("unknown class `{text}`"))),
        }
    }

    fn table_like(&self, key: &str) -> Result<Option<&'a dyn TableLike>, TableError> {
        match self.get(key) {
            None => Ok(None),
            Some(item) => item
                .as_table_like()
                .map(Some)
                .ok_or_else(|| self.error(format!("`{key}` must be a table"))),
        }
    }

    fn child(&self, key: &str) -> Result<Node<'a>, TableError> {
        let inner = self
            .table_like(key)?
            .ok_or_else(|| self.error(format!("`{key}` is missing")))?;
        Ok(Node::new(
            self.table,
            &format!("{} `{key}`", self.at),
            inner,
        ))
    }

    fn children(&self, key: &str) -> Result<Vec<Node<'a>>, TableError> {
        let Some(item) = self.get(key) else {
            return Ok(Vec::new());
        };
        let array = item
            .as_array_of_tables()
            .ok_or_else(|| self.error(format!("`{key}` must be an array of tables")))?;
        Ok(array
            .iter()
            .enumerate()
            .map(|(index, table)| {
                Node::new(
                    self.table,
                    &format!("{} `{key}` {}", self.at, index + 1),
                    table,
                )
            })
            .collect())
    }

    fn inline_tables(&self, key: &str) -> Result<Vec<Node<'a>>, TableError> {
        let Some(item) = self.get(key) else {
            return Ok(Vec::new());
        };
        let array = item
            .as_array()
            .ok_or_else(|| self.error(format!("`{key}` must be an array of tables")))?;
        array
            .iter()
            .enumerate()
            .map(|(index, value)| {
                value
                    .as_inline_table()
                    .map(|table| {
                        Node::new(
                            self.table,
                            &format!("{} `{key}` {}", self.at, index + 1),
                            table,
                        )
                    })
                    .ok_or_else(|| self.error(format!("`{key}` must be an array of tables")))
            })
            .collect()
    }

    fn all_pairs(&self) -> Result<Vec<(String, String)>, TableError> {
        self.inner
            .iter()
            .map(|(key, item)| {
                item.as_str()
                    .map(|text| (key.to_owned(), text.to_owned()))
                    .ok_or_else(|| self.error(format!("`{key}` must be a string")))
            })
            .collect()
    }

    fn opt_pairs(&self, key: &str) -> Result<Vec<(String, String)>, TableError> {
        match self.table_like(key)? {
            None => Ok(Vec::new()),
            Some(inner) => {
                Node::new(self.table, &format!("{} `{key}`", self.at), inner).all_pairs()
            }
        }
    }

    fn pairs(&self, key: &str) -> Result<Vec<(String, String)>, TableError> {
        if self.get(key).is_none() {
            return Err(self.error(format!("`{key}` is missing")));
        }
        self.opt_pairs(key)
    }

    fn opt_set_value(&self, key: &str) -> Result<Option<SetValue>, TableError> {
        let Some(value) = self.opt_value(key) else {
            return Ok(None);
        };
        match value {
            Value::Boolean(flag) => Ok(Some(SetValue::Bool(*flag.value()))),
            Value::Integer(int) => Ok(Some(SetValue::Int(*int.value()))),
            Value::String(text) => Ok(Some(SetValue::Ident(text.value().clone()))),
            _ => Err(self.error(format!("`{key}` must be a boolean, an integer or a string"))),
        }
    }

    /// The single value key of a constant: `ident`, `text`, `bool`, `int`
    /// or `call`.
    fn typed_value(&self) -> Result<SetValue, TableError> {
        let mut found = Vec::new();
        if let Some(text) = self.opt_string("ident")? {
            found.push(SetValue::Ident(text));
        }
        if let Some(text) = self.opt_string("text")? {
            found.push(SetValue::Text(text));
        }
        if let Some(flag) = self.opt_bool("bool")? {
            found.push(SetValue::Bool(flag));
        }
        if let Some(int) = self.opt_int("int")? {
            found.push(SetValue::Int(int));
        }
        if let Some(name) = self.opt_string("call")? {
            found.push(SetValue::Call(name));
        }
        match found.len() {
            1 => found
                .pop()
                .ok_or_else(|| self.error("a value is missing".to_owned())),
            _ => Err(self.error(
                "exactly one of `ident`, `text`, `bool`, `int` and `call` is required".to_owned(),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HABITS: &str = "format = \"nurt-correction-table/1\"\ntable = \"habits_powershell\"\nwords = [\"gci\"]\nverbs = [\"Get\"]\n";

    fn minimal(extra: &str) -> String {
        format!(
            "format = \"nurt-correction-table/1\"\ntable = \"powershell\"\neffect_commands = []\npath_rule = \"L19\"\n\
             [syntax]\nassignment = \"P32\"\nenvironment = \"P31\"\ncall = \"P42\"\noutput = \"P36\"\n\
             output_command = \"evaluate\"\nenvironment_command = \"environment\"\nenvironment_param = \"name\"\n\
             [registry]\nscheme = \"registry:\"\nprovider = \"Registry::\"\ndrives = {{ HKLM = \"HKLM\" }}\n\
             [dates]\ncmdlet = \"Get-Date\"\nnow = \"now\"\nago = \"ago\"\nmethods = {{ AddDays = \"d\" }}\n\
             [wildcard]\ncommand = \"find\"\nglob_arg = \"glob\"\n\
             [comparison]\ntext_assumption = \"a\"\norder_assumption = \"b\"\nprefix_assumption = \"c\"\n{extra}"
        )
    }

    #[test]
    fn embedded_table_is_valid() {
        let table = PowerShellTable::embedded();
        assert!(table.is_ok(), "{table:?}");
        assert!(table.is_ok_and(|table| table.cmdlet("gci").is_some()));
    }

    #[test]
    fn minimal_table_without_cmdlets_is_valid() {
        let table = PowerShellTable::parse(&minimal(""), HABITS);
        assert!(table.is_ok(), "{table:?}");
    }

    #[test]
    fn rejects_unknown_key() {
        let table = PowerShellTable::parse(&minimal("colour = 1\n"), HABITS);
        assert!(table.is_err_and(|error| error.message.contains("unknown key `colour`")));
    }

    #[test]
    fn rejects_variant_with_class_and_error() {
        let extra = "[[entry]]\nrule = \"P01\"\nclass = \"C\"\nfrom = [\"Get-X\"]\n\
                     [[entry.variant]]\nrule = \"P01\"\nclass = \"C\"\nerror = \"E_UNCORRECTABLE\"\nmessage = \"m\"\n";
        let table = PowerShellTable::parse(&minimal(extra), HABITS);
        assert!(
            table.is_err_and(|error| error.message.contains("exactly one of `class` and `error`"))
        );
    }

    #[test]
    fn rejects_probable_variant_without_assumption() {
        let extra = "[[entry]]\nrule = \"P01\"\nclass = \"P\"\nfrom = [\"Get-X\"]\n\
                     [[entry.variant]]\nrule = \"P01\"\nclass = \"P\"\n";
        let table = PowerShellTable::parse(&minimal(extra), HABITS);
        assert!(table.is_err_and(|error| error.message.contains("needs `assumption`")));
    }

    #[test]
    fn rejects_unknown_parameter_type_and_constant_without_value() {
        let extra = "[[entry]]\nrule = \"P01\"\nclass = \"C\"\nfrom = [\"Get-X\"]\n\
                     [[entry.param]]\nname = \"A\"\ntype = \"colour\"\n\
                     [[entry.variant]]\nrule = \"P01\"\nclass = \"C\"\n";
        let table = PowerShellTable::parse(&minimal(extra), HABITS);
        assert!(table.is_err_and(|error| error.message.contains("unknown type `colour`")));
        let extra = "[[entry]]\nrule = \"P01\"\nclass = \"C\"\nfrom = [\"Get-X\"]\n\
                     [[entry.variant]]\nrule = \"P01\"\nclass = \"C\"\nset = [{ arg = \"a\" }]\n";
        let table = PowerShellTable::parse(&minimal(extra), HABITS);
        assert!(table.is_err_and(|error| error.message.contains("exactly one of")));
    }

    #[test]
    fn rejects_cmdlet_without_variant() {
        let extra = "[[entry]]\nrule = \"P01\"\nclass = \"C\"\nfrom = [\"Get-X\"]\n";
        let table = PowerShellTable::parse(&minimal(extra), HABITS);
        assert!(table.is_err_and(|error| error.message.contains("needs a variant")));
    }

    #[test]
    fn recognizes_approved_verbs_only() {
        let Ok(table) = PowerShellTable::embedded() else {
            return;
        };
        assert!(table.is_cmdlet_form("Get-Widget"));
        assert!(table.is_cmdlet_form("get-widget"));
        assert!(!table.is_cmdlet_form("docker-compose"));
        assert!(!table.is_cmdlet_form("Get-"));
    }

    #[test]
    fn parameter_prefixes_and_aliases() {
        let Some(param) = PowerShellTable::embedded()
            .ok()
            .and_then(|table| table.cmdlet("Get-ChildItem"))
            .and_then(|cmdlet| {
                cmdlet
                    .params
                    .iter()
                    .find(|param| param.name == "LiteralPath")
            })
        else {
            panic!("LiteralPath is missing");
        };
        assert!(param.is_named("lp"));
        assert!(param.has_prefix("Lit"));
        assert!(!param.has_prefix("Path"));
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
