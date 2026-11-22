// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Rule tables in the format `nurt-argv-rules/1`.

use serde::Deserialize;

use super::argv::{Invocation, ValueOptions, Wrapper};
use super::pattern::{OptionToken, Pattern, Tail, Words};

/// Format name of the rule tables.
const FORMAT: &str = "nurt-argv-rules/1";

/// Table of programs with effects outside the file system.
const RUN_EFFECTS: &str = include_str!("../../data/run-effects.toml");

/// Table of command-line classes.
const ARGV_CLASSES: &str = include_str!("../../data/argv-classes.toml");

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    format: String,
    #[serde(default)]
    wrapper: Vec<WrapperEntry>,
    #[serde(default)]
    program: Vec<ProgramEntry>,
    #[serde(default)]
    rule: Vec<RuleEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WrapperEntry {
    program: String,
    #[serde(default)]
    command: String,
    #[serde(default)]
    value_options: Vec<String>,
    #[serde(default)]
    assignments: bool,
    #[serde(default)]
    skip: usize,
    module_option: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProgramEntry {
    name: String,
    value_options: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RuleEntry {
    class: Option<Class>,
    name: Option<String>,
    pattern: String,
    #[serde(default)]
    unless: Vec<String>,
    max_positionals: Option<usize>,
    #[serde(default)]
    without_input: bool,
    script_option: Option<String>,
    #[serde(default)]
    script_rest: bool,
    #[serde(default)]
    exact_options: bool,
}

/// Class of a command line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Class {
    /// Waits without end.
    Waits,
    /// Needs a terminal.
    Interactive,
    /// Interpreter given a script in its arguments.
    OpaqueScript,
}

/// Where the script text of an interpreter is found.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ScriptSource {
    option: Option<OptionToken>,
    rest: bool,
}

/// Compiled rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Rule {
    /// Name reported for a match.
    pub(super) name: String,
    /// Class, for rules of the class table.
    pub(super) class: Option<Class>,
    /// Interactive only without standard input.
    pub(super) without_input: bool,
    pattern: Pattern,
    unless: Vec<Tail>,
    max_positionals: Option<usize>,
    script: Option<ScriptSource>,
    exact_options: bool,
}

impl Rule {
    /// Rule of the key `run.irreversible_patterns`.
    pub(super) fn from_pattern(text: &str) -> Result<Self, String> {
        Ok(Self {
            name: text.trim().to_owned(),
            class: None,
            without_input: false,
            pattern: Pattern::parse(text)?,
            unless: Vec::new(),
            max_positionals: None,
            script: None,
            exact_options: false,
        })
    }

    fn from_entry(entry: RuleEntry) -> Result<Self, String> {
        let script = match (&entry.script_option, entry.script_rest) {
            (None, false) => None,
            (option, rest) => Some(ScriptSource {
                option: option.as_deref().map(OptionToken::parse).transpose()?,
                rest,
            }),
        };
        let unless = entry
            .unless
            .iter()
            .map(|text| Tail::parse(text))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            name: entry.name.unwrap_or_else(|| entry.pattern.clone()),
            class: entry.class,
            without_input: entry.without_input,
            pattern: Pattern::parse(&entry.pattern)?,
            unless,
            max_positionals: entry.max_positionals,
            script,
            exact_options: entry.exact_options,
        })
    }

    /// Whether the rule matches; returns the index in the arguments after the
    /// last positional argument consumed by the pattern.
    pub(super) fn matches(&self, invocation: &Invocation<'_>) -> Option<usize> {
        if !self.pattern.program.matches(&invocation.program) {
            return None;
        }
        if self
            .max_positionals
            .is_some_and(|max| invocation.positionals.len() > max)
        {
            return None;
        }
        let end = self.pattern.tail.matches(invocation, self.exact_options)?;
        let excluded = self
            .unless
            .iter()
            .any(|tail| tail.matches(invocation, self.exact_options).is_some());
        (!excluded).then_some(end)
    }

    /// Script text of a matched interpreter rule; `None` when empty.
    pub(super) fn script(&self, invocation: &Invocation<'_>, end: usize) -> Option<String> {
        let source = self.script.as_ref()?;
        let args = invocation.args;
        let text = match &source.option {
            Some(option) => {
                let hit = option.find(args, self.exact_options)?;
                let before_operands = invocation
                    .positionals
                    .first()
                    .is_none_or(|&(first, _)| hit.index < first);
                if !self.exact_options && !before_operands {
                    return None;
                }
                if source.rest {
                    args.get(hit.index + 1..)?.join(" ")
                } else {
                    hit.value?
                }
            }
            None => args.get(end..)?.join(" "),
        };
        (!text.is_empty()).then_some(text)
    }
}

/// Wrappers, program options and rules of both tables.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Tables {
    /// Wrappers.
    pub(super) wrappers: Vec<Wrapper>,
    /// Options followed by a value, per program.
    pub(super) value_options: Vec<ValueOptions>,
    /// Rules of programs with external effects.
    pub(super) effects: Vec<Rule>,
    /// Rules of command-line classes.
    pub(super) classes: Vec<Rule>,
}

impl Tables {
    /// Tables built into the program.
    pub(super) fn builtin() -> Result<Self, String> {
        Self::load(RUN_EFFECTS, ARGV_CLASSES)
    }

    /// Loads the effect table and the class table.
    pub(super) fn load(effects: &str, classes: &str) -> Result<Self, String> {
        let mut tables = Self {
            wrappers: Vec::new(),
            value_options: Vec::new(),
            effects: Vec::new(),
            classes: Vec::new(),
        };
        tables.add(effects, false)?;
        tables.add(classes, true)?;
        Ok(tables)
    }

    fn add(&mut self, text: &str, classes: bool) -> Result<(), String> {
        let document: Document = toml::from_str(text).map_err(|error| error.to_string())?;
        if document.format != FORMAT {
            return Err(format!(
                "unknown table format `{}`, expected `{FORMAT}`",
                document.format
            ));
        }
        for entry in document.wrapper {
            self.wrappers.push(wrapper(entry)?);
        }
        for entry in document.program {
            self.value_options.push(ValueOptions {
                program: Words::parse(&entry.name)?,
                options: entry.value_options,
            });
        }
        for entry in document.rule {
            if entry.class.is_some() != classes {
                return Err(format!(
                    "rule `{}` must {} a class",
                    entry.pattern,
                    if classes { "have" } else { "not have" }
                ));
            }
            let rule = Rule::from_entry(entry)?;
            if classes {
                self.classes.push(rule);
            } else {
                self.effects.push(rule);
            }
        }
        Ok(())
    }
}

fn wrapper(entry: WrapperEntry) -> Result<Wrapper, String> {
    Ok(Wrapper {
        program: Words::parse(&entry.program)?,
        command: entry
            .command
            .split_whitespace()
            .map(Words::parse)
            .collect::<Result<Vec<_>, _>>()?,
        value_options: entry.value_options,
        assignments: entry.assignments,
        skip: entry.skip,
        module_option: entry.module_option,
    })
}
