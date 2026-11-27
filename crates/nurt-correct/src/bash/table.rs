// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Data of the bash translation table and of the habit word set.
//!
//! The table is a TOML document of the format `nurt-correction-table/1`
//! embedded in the program. Each `[[entry]]` entry gives the bash command,
//! the rule and certainty class, the result template, the mapping of every
//! supported flag to parameters of the target command and the forms that
//! depend on operands or flags. Interpreters of embedded languages (`find`
//! predicates, `sed` scripts, `awk` programs, `test` expressions) read their
//! value maps from `[values]`. Texts of errors and assumptions are kept in
//! `[messages]` and `[assumptions]`.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::LazyLock;

use nurt_contracts::report::CorrectionClass;
use serde::Deserialize;
use toml_edit::{DocumentMut, Item, Value};

use super::template::{
    TemplateArg, TemplatePipeline, TemplateStage, parse_args, parse_pipeline, parse_stages,
};

/// Error code of an entry that is never translated.
const ERROR_CODE: &str = "E_UNCORRECTABLE";

/// Format name written in the header of every correction table.
pub const TABLE_FORMAT: &str = "nurt-correction-table/1";

const BASH_TABLE: &str = include_str!("../../tables/bash.toml");
const HABITS_TABLE: &str = include_str!("../../tables/habits_bash.toml");

static EMBEDDED: LazyLock<Result<BashTable, TableError>> =
    LazyLock::new(|| BashTable::parse(BASH_TABLE, HABITS_TABLE));

/// Error in the data of a translation table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableError {
    /// Table and entry concerned, for example `bash: command rm`.
    pub place: String,
    /// What is wrong.
    pub message: String,
}

impl fmt::Display for TableError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.place, self.message)
    }
}

impl std::error::Error for TableError {}

/// The bash translation table with the habit word set, ready for use.
#[derive(Debug, Clone)]
pub struct BashTable {
    pub(crate) commands: Vec<CommandEntry>,
    pub(crate) habits: Vec<String>,
    pub(crate) shell_only: Vec<String>,
    pub(crate) changing: Vec<String>,
    pub(crate) running: Vec<String>,
    pub(crate) params: BTreeMap<String, Vec<String>>,
    pub(crate) messages: BTreeMap<String, Message>,
    pub(crate) assumptions: BTreeMap<String, String>,
    pub(crate) values: BTreeMap<String, BTreeMap<String, serde_json::Value>>,
    pub(crate) glob_source: Option<TemplatePipeline>,
    pub(crate) glob_hidden: Vec<TemplateArg>,
}

#[derive(Debug, Clone)]
pub(crate) struct CommandEntry {
    pub(crate) words: Vec<String>,
    pub(crate) rule: String,
    pub(crate) class: CorrectionClass,
    pub(crate) help: String,
    pub(crate) interpreter: Option<String>,
    pub(crate) min: usize,
    pub(crate) max: Option<usize>,
    pub(crate) template: Option<TemplatePipeline>,
    pub(crate) output: String,
    pub(crate) assumption: Option<String>,
    pub(crate) glob: Option<GlobEntry>,
    pub(crate) options: Vec<OptionEntry>,
    pub(crate) forms: Vec<FormEntry>,
    pub(crate) stages: Vec<StageEntry>,
    pub(crate) target: String,
    pub(crate) error: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct GlobEntry {
    pub(crate) rule: String,
    pub(crate) stages: Vec<TemplateStage>,
    /// Number of operands of a command whose single pattern is expanded.
    pub(crate) operands: usize,
}

#[derive(Debug, Clone)]
pub(crate) struct OptionEntry {
    pub(crate) id: String,
    pub(crate) flags: Vec<String>,
    pub(crate) value: Option<String>,
    pub(crate) args: Vec<TemplateArg>,
    pub(crate) stages: Vec<TemplateStage>,
    pub(crate) absent: Vec<TemplateArg>,
    pub(crate) default: bool,
    pub(crate) class: Option<CorrectionClass>,
}

#[derive(Debug, Clone)]
pub(crate) struct FormEntry {
    pub(crate) when: When,
    pub(crate) template: TemplatePipeline,
    pub(crate) rule: Option<String>,
    pub(crate) class: Option<CorrectionClass>,
    pub(crate) output: Option<String>,
    pub(crate) assumption: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct StageEntry {
    pub(crate) options: bool,
    pub(crate) input: Vec<String>,
    pub(crate) when: When,
    pub(crate) template: Vec<TemplateStage>,
    pub(crate) output: Option<String>,
    pub(crate) class: Option<CorrectionClass>,
    pub(crate) assumption: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct When {
    #[serde(default)]
    pub(crate) operands: Option<usize>,
    #[serde(default)]
    pub(crate) min_operands: Option<usize>,
    #[serde(default)]
    pub(crate) dest_dir: Option<bool>,
    #[serde(default)]
    pub(crate) options: Vec<String>,
    #[serde(default)]
    pub(crate) without: Vec<String>,
    #[serde(default)]
    pub(crate) piped: Option<bool>,
    #[serde(default)]
    pub(crate) followed: Option<bool>,
    #[serde(default)]
    pub(crate) words: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Message {
    pub(crate) message: String,
    #[serde(default)]
    pub(crate) help: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTable {
    format: String,
    table: String,
    summary: String,
    #[serde(default)]
    effects: RawEffects,
    #[serde(default)]
    params: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    messages: BTreeMap<String, Message>,
    #[serde(default)]
    assumptions: BTreeMap<String, String>,
    #[serde(default)]
    values: BTreeMap<String, BTreeMap<String, serde_json::Value>>,
    #[serde(default)]
    glob: Option<RawGlobSource>,
    #[serde(default)]
    entry: Vec<RawCommand>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawGlobSource {
    source: String,
    hidden: String,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawEffects {
    #[serde(default)]
    change: Vec<String>,
    #[serde(default)]
    run: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCommand {
    name: String,
    rule: String,
    class: String,
    help: String,
    #[serde(default)]
    interpreter: Option<String>,
    #[serde(default)]
    min: usize,
    #[serde(default)]
    max: Option<usize>,
    #[serde(default)]
    template: Option<String>,
    #[serde(default)]
    output: Option<String>,
    #[serde(default)]
    assumption: Option<String>,
    #[serde(default)]
    glob: Option<RawGlob>,
    #[serde(default)]
    option: Vec<RawOption>,
    #[serde(default)]
    form: Vec<RawForm>,
    #[serde(default)]
    stage: Vec<RawStage>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    problem: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawGlob {
    rule: String,
    stages: String,
    #[serde(default = "one")]
    operands: usize,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawOption {
    flags: Vec<String>,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    value: Option<String>,
    #[serde(default)]
    args: Option<String>,
    #[serde(default)]
    stages: Option<String>,
    #[serde(default)]
    absent: Option<String>,
    #[serde(default)]
    default: bool,
    #[serde(default)]
    class: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawForm {
    #[serde(default)]
    when: When,
    template: String,
    #[serde(default)]
    rule: Option<String>,
    #[serde(default)]
    class: Option<String>,
    #[serde(default)]
    output: Option<String>,
    #[serde(default)]
    assumption: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawStage {
    #[serde(default = "applies")]
    options: bool,
    input: Vec<String>,
    #[serde(default)]
    when: When,
    template: String,
    #[serde(default)]
    output: Option<String>,
    #[serde(default)]
    class: Option<String>,
    #[serde(default)]
    assumption: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawHabits {
    format: String,
    table: String,
    summary: String,
    words: Vec<String>,
    #[serde(default)]
    shell_only: Vec<String>,
}

impl BashTable {
    /// Returns the table embedded in the program.
    ///
    /// # Errors
    ///
    /// Returns [`TableError`] when the embedded data is invalid; the test
    /// suite keeps it valid.
    pub fn embedded() -> Result<&'static Self, TableError> {
        EMBEDDED.as_ref().map_err(Clone::clone)
    }

    /// Reads a table from the text of the translation table and of the
    /// habit word set.
    ///
    /// # Errors
    ///
    /// Returns [`TableError`] when a document is not valid TOML, has another
    /// format, contains unknown keys or a template that does not parse.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_correct::bash::BashTable;
    ///
    /// let table = BashTable::parse(
    ///     "format = \"nurt-correction-table/1\"\ntable = \"bash\"\nsummary = \"s\"\n",
    ///     "format = \"nurt-correction-table/1\"\ntable = \"habits_bash\"\nsummary = \"s\"\nwords = [\"rm\"]\n",
    /// )?;
    /// assert!(table.is_habit("rm"));
    /// # Ok::<(), nurt_correct::bash::TableError>(())
    /// ```
    pub fn parse(table: &str, habits: &str) -> Result<Self, TableError> {
        let raw: RawTable = from_toml(table, "bash")?;
        check_header(&raw.format, &raw.table, &raw.summary, "bash")?;
        let habits_raw: RawHabits = from_toml(habits, "habits_bash")?;
        check_header(
            &habits_raw.format,
            &habits_raw.table,
            &habits_raw.summary,
            "habits_bash",
        )?;
        let commands = raw
            .entry
            .into_iter()
            .map(compile_command)
            .collect::<Result<Vec<_>, _>>()?;
        let glob_source = raw
            .glob
            .as_ref()
            .map(|glob| pipeline(&glob.source, "bash: glob"))
            .transpose()?;
        let glob_hidden = raw
            .glob
            .as_ref()
            .map(|glob| args(&glob.hidden, "bash: glob"))
            .transpose()?
            .unwrap_or_default();
        Ok(Self {
            glob_source,
            glob_hidden,
            commands,
            habits: habits_raw.words,
            shell_only: habits_raw.shell_only,
            changing: raw.effects.change,
            running: raw.effects.run,
            params: raw.params,
            messages: raw.messages,
            assumptions: raw.assumptions,
            values: raw.values,
        })
    }

    /// Whether the word belongs to the habit set: words that start a command
    /// written in bash notation.
    #[must_use]
    pub fn is_habit(&self, word: &str) -> bool {
        self.habits.iter().any(|habit| habit == word)
    }

    /// The habit word set in table order.
    #[must_use]
    pub fn habits(&self) -> &[String] {
        &self.habits
    }

    /// Rule identifiers of the table entries, sorted and without duplicates.
    #[must_use]
    pub fn rules(&self) -> Vec<String> {
        let mut rules: Vec<String> = self
            .commands
            .iter()
            .flat_map(|command| {
                std::iter::once(command.rule.clone())
                    .chain(command.forms.iter().filter_map(|form| form.rule.clone()))
                    .chain(command.glob.iter().map(|glob| glob.rule.clone()))
            })
            .collect();
        rules.sort();
        rules.dedup();
        rules
    }

    /// Every target command name with its parameter names used by the
    /// table, for checks against the command catalog.
    #[must_use]
    pub fn parameters(&self) -> &BTreeMap<String, Vec<String>> {
        &self.params
    }

    /// Commands the table classifies as changing or running.
    pub fn effect_commands(&self) -> impl Iterator<Item = &str> {
        self.changing
            .iter()
            .chain(&self.running)
            .map(String::as_str)
    }

    pub(crate) fn is_shell_only(&self, word: &str) -> bool {
        self.shell_only.iter().any(|known| known == word)
    }

    pub(crate) fn has_effect(&self, command: &str) -> bool {
        self.effect_commands().any(|name| name == command)
    }

    /// The entry with the longest word sequence that prefixes the command
    /// words.
    pub(crate) fn command(&self, words: &[String]) -> Option<&CommandEntry> {
        self.commands
            .iter()
            .filter(|entry| {
                entry.words.len() <= words.len()
                    && entry.words.iter().zip(words).all(|(a, b)| a == b)
            })
            .max_by_key(|entry| entry.words.len())
    }

    pub(crate) fn message(&self, problem: &str) -> Option<&Message> {
        self.messages.get(problem)
    }

    pub(crate) fn assumption(&self, key: &str) -> Option<&str> {
        self.assumptions.get(key).map(String::as_str)
    }

    pub(crate) fn value(&self, map: &str, key: &str) -> Option<&serde_json::Value> {
        self.values.get(map).and_then(|values| values.get(key))
    }

    pub(crate) fn value_str(&self, map: &str, key: &str) -> Option<&str> {
        self.value(map, key).and_then(serde_json::Value::as_str)
    }
}

fn applies() -> bool {
    true
}

fn one() -> usize {
    1
}

fn check_header(
    format: &str,
    table: &str,
    summary: &str,
    expected: &str,
) -> Result<(), TableError> {
    if summary.is_empty() {
        return Err(table_error(expected, "empty `summary`"));
    }
    if format != TABLE_FORMAT {
        return Err(table_error(expected, &format!("unknown format `{format}`")));
    }
    if table != expected {
        return Err(table_error(
            expected,
            &format!("table name `{table}`, expected `{expected}`"),
        ));
    }
    Ok(())
}

fn table_error(place: &str, message: &str) -> TableError {
    TableError {
        place: place.to_owned(),
        message: message.to_owned(),
    }
}

pub(crate) fn parse_class(text: &str, place: &str) -> Result<CorrectionClass, TableError> {
    text.parse()
        .map_err(|_| table_error(place, &format!("unknown class `{text}`")))
}

fn compile_command(raw: RawCommand) -> Result<CommandEntry, TableError> {
    let place = format!("bash: command {}", raw.name);
    let template = raw
        .template
        .as_deref()
        .map(|text| pipeline(text, &place))
        .transpose()?;
    let forms = raw
        .form
        .into_iter()
        .map(|form| compile_form(form, &place))
        .collect::<Result<Vec<_>, _>>()?;
    let target = template
        .as_ref()
        .or_else(|| forms.first().map(|form| &form.template))
        .and_then(|t| t.stages.last())
        .map(|stage| stage.name.clone())
        .unwrap_or_default();
    let glob = raw
        .glob
        .map(|glob| {
            Ok::<_, TableError>(GlobEntry {
                stages: stages(&glob.stages, &place)?,
                operands: glob.operands,
                rule: glob.rule,
            })
        })
        .transpose()?;
    Ok(CommandEntry {
        words: raw.name.split_whitespace().map(str::to_owned).collect(),
        class: parse_class(&raw.class, &place)?,
        rule: raw.rule,
        help: raw.help,
        interpreter: raw.interpreter,
        min: raw.min,
        max: raw.max,
        template,
        output: raw.output.unwrap_or_else(|| "other".to_owned()),
        assumption: raw.assumption,
        glob,
        options: raw
            .option
            .into_iter()
            .map(|option| compile_option(option, &place))
            .collect::<Result<_, _>>()?,
        forms,
        stages: raw
            .stage
            .into_iter()
            .map(|stage| {
                Ok::<_, TableError>(StageEntry {
                    options: stage.options,
                    input: stage.input,
                    when: stage.when,
                    template: stages(&format!("|> {}", stage.template), &place)?,
                    output: stage.output,
                    class: stage
                        .class
                        .as_deref()
                        .map(|c| parse_class(c, &place))
                        .transpose()?,
                    assumption: stage.assumption,
                })
            })
            .collect::<Result<_, _>>()?,
        target,
        error: match (raw.error.as_deref(), raw.problem) {
            (Some(ERROR_CODE), Some(problem)) => Some(problem),
            (None, None) => None,
            _ => {
                return Err(table_error(
                    &place,
                    "an error entry needs `error = \"E_UNCORRECTABLE\"` and `problem`",
                ));
            }
        },
    })
}

fn compile_option(raw: RawOption, place: &str) -> Result<OptionEntry, TableError> {
    let id = raw.id.unwrap_or_else(|| {
        raw.flags
            .first()
            .map(|flag| flag.trim_start_matches('-').to_owned())
            .unwrap_or_default()
    });
    let place = format!("{place} option {id}");
    Ok(OptionEntry {
        args: raw
            .args
            .as_deref()
            .map(|a| args(a, &place))
            .transpose()?
            .unwrap_or_default(),
        stages: raw
            .stages
            .as_deref()
            .map(|s| stages(s, &place))
            .transpose()?
            .unwrap_or_default(),
        absent: raw
            .absent
            .as_deref()
            .map(|a| args(a, &place))
            .transpose()?
            .unwrap_or_default(),
        class: raw
            .class
            .as_deref()
            .map(|c| parse_class(c, &place))
            .transpose()?,
        id,
        flags: raw.flags,
        value: raw.value,
        default: raw.default,
    })
}

fn compile_form(raw: RawForm, place: &str) -> Result<FormEntry, TableError> {
    Ok(FormEntry {
        template: pipeline(&raw.template, place)?,
        class: raw
            .class
            .as_deref()
            .map(|c| parse_class(c, place))
            .transpose()?,
        when: raw.when,
        rule: raw.rule,
        output: raw.output,
        assumption: raw.assumption,
    })
}

fn pipeline(text: &str, place: &str) -> Result<TemplatePipeline, TableError> {
    parse_pipeline(text)
        .map_err(|error| table_error(place, &format!("template `{text}`: {}", error.0)))
}

fn stages(text: &str, place: &str) -> Result<Vec<TemplateStage>, TableError> {
    parse_stages(text).map_err(|error| table_error(place, &format!("stages `{text}`: {}", error.0)))
}

fn args(text: &str, place: &str) -> Result<Vec<TemplateArg>, TableError> {
    parse_args(text)
        .map_err(|error| table_error(place, &format!("arguments `{text}`: {}", error.0)))
}

fn from_toml<T: serde::de::DeserializeOwned>(text: &str, place: &str) -> Result<T, TableError> {
    let document: DocumentMut = text
        .parse()
        .map_err(|error: toml_edit::TomlError| table_error(place, &error.to_string()))?;
    let json = table_to_json(document.as_table());
    serde_json::from_value(json).map_err(|error| table_error(place, &error.to_string()))
}

fn table_to_json(table: &toml_edit::Table) -> serde_json::Value {
    serde_json::Value::Object(
        table
            .iter()
            .map(|(key, item)| (key.to_owned(), item_to_json(item)))
            .collect(),
    )
}

fn item_to_json(item: &Item) -> serde_json::Value {
    match item {
        Item::Value(value) => value_to_json(value),
        Item::Table(table) => table_to_json(table),
        Item::ArrayOfTables(array) => {
            serde_json::Value::Array(array.iter().map(table_to_json).collect())
        }
        Item::None => serde_json::Value::Null,
    }
}

fn value_to_json(value: &Value) -> serde_json::Value {
    match value {
        Value::String(text) => serde_json::Value::String(text.value().clone()),
        Value::Integer(number) => serde_json::Value::from(*number.value()),
        Value::Float(number) => serde_json::Value::from(*number.value()),
        Value::Boolean(flag) => serde_json::Value::Bool(*flag.value()),
        Value::Datetime(datetime) => serde_json::Value::String(datetime.value().to_string()),
        Value::Array(array) => serde_json::Value::Array(array.iter().map(value_to_json).collect()),
        Value::InlineTable(table) => serde_json::Value::Object(
            table
                .iter()
                .map(|(key, value)| (key.to_owned(), value_to_json(value)))
                .collect(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HABITS: &str = "format = \"nurt-correction-table/1\"\ntable = \"habits_bash\"\nsummary = \"s\"\nwords = [\"rm\"]\n";

    fn table(body: &str) -> Result<BashTable, TableError> {
        let text = format!(
            "format = \"nurt-correction-table/1\"\ntable = \"bash\"\nsummary = \"s\"\n{body}"
        );
        BashTable::parse(&text, HABITS)
    }

    #[test]
    fn embedded_table_is_valid() {
        let table = BashTable::embedded().unwrap();
        assert!(table.is_habit("rm"));
        assert!(table.is_habit("sudo"));
        assert!(!table.is_habit("remove"));
    }

    #[test]
    fn header_is_checked() {
        let wrong = "format = \"x\"\ntable = \"bash\"\nsummary = \"s\"\n";
        assert!(
            BashTable::parse(wrong, HABITS)
                .unwrap_err()
                .message
                .contains("unknown format")
        );
        let other = "format = \"nurt-correction-table/1\"\ntable = \"p\"\nsummary = \"s\"\n";
        assert!(
            BashTable::parse(other, HABITS)
                .unwrap_err()
                .message
                .contains("table name")
        );
        assert!(BashTable::parse("x = ", HABITS).is_err());
    }

    #[test]
    fn unknown_keys_and_bad_templates_are_rejected() {
        assert!(table("unknown = 1\n").is_err());
        let bad = "[[entry]]\nname = \"rm\"\nrule = \"B01\"\nclass = \"C\"\nhelp = \"remove\"\ntemplate = \"remove $\"\n";
        let error = table(bad).unwrap_err();
        assert_eq!(error.place, "bash: command rm");
        let bad_class =
            "[[entry]]\nname = \"rm\"\nrule = \"B01\"\nclass = \"Q\"\nhelp = \"remove\"\n";
        assert!(
            table(bad_class)
                .unwrap_err()
                .message
                .contains("unknown class")
        );
    }

    #[test]
    fn longest_command_prefix_wins() {
        let body = "[[entry]]\nname = \"git\"\nrule = \"B38\"\nclass = \"C\"\nhelp = \"run\"\n\
                    [[entry]]\nname = \"git status\"\nrule = \"B38\"\nclass = \"C\"\nhelp = \"git.status\"\n";
        let parsed = table(body).unwrap();
        let words = ["git".to_owned(), "status".to_owned(), "-s".to_owned()];
        assert_eq!(parsed.command(&words).unwrap().words.len(), 2);
        assert_eq!(parsed.command(&words[..1]).unwrap().words.len(), 1);
        assert!(parsed.command(&["ls".to_owned()]).is_none());
        assert_eq!(parsed.rules(), vec!["B38".to_owned()]);
    }

    #[test]
    fn values_of_every_toml_kind_are_read() {
        let body = "[values.kinds]\na = 1\nb = 1.5\nc = true\nd = 2026-11-22\ne = [1]\nf = { g = \"h\" }\n";
        let parsed = table(body).unwrap();
        assert_eq!(parsed.value("kinds", "a"), Some(&serde_json::json!(1)));
        assert_eq!(
            parsed.value("kinds", "d"),
            Some(&serde_json::json!("2026-11-22"))
        );
        assert_eq!(parsed.value_str("kinds", "f"), None);
        assert_eq!(
            parsed.value("kinds", "f"),
            Some(&serde_json::json!({"g": "h"}))
        );
    }

    #[test]
    fn table_error_displays_place() {
        let error = table_error("bash", "broken");
        assert_eq!(error.to_string(), "bash: broken");
    }

    /// Parameters of one catalog command: allowed values and default.
    type CatalogParams = BTreeMap<String, (Vec<String>, Option<String>)>;

    /// Commands that the translator builds outside the templates of the
    /// table.
    const BUILT_STAGES: &[&str] = &[
        "write",
        "run",
        "evaluate",
        "schedule.create",
        "lines",
        "info",
        "assert",
        "find",
    ];

    fn catalog() -> BTreeMap<String, CatalogParams> {
        let dir = std::path::Path::new(
            std::env::var("CARGO_MANIFEST_DIR")
                .as_deref()
                .unwrap_or(env!("CARGO_MANIFEST_DIR")),
        )
        .join("../../catalog");
        let mut commands = BTreeMap::new();
        for file in std::fs::read_dir(&dir).unwrap() {
            let path = file.unwrap().path();
            let toml_file = path
                .extension()
                .is_some_and(|extension| extension == "toml");
            if !toml_file || path.file_name().is_some_and(|name| name == "_index.toml") {
                continue;
            }
            let document: DocumentMut = std::fs::read_to_string(&path).unwrap().parse().unwrap();
            let Some(list) = document.get("command").and_then(Item::as_array_of_tables) else {
                continue;
            };
            for command in list {
                let name = command
                    .get("name")
                    .and_then(Item::as_str)
                    .unwrap()
                    .to_owned();
                let mut params = CatalogParams::new();
                for param in command
                    .get("param")
                    .and_then(Item::as_array_of_tables)
                    .into_iter()
                    .flatten()
                {
                    let param_name = param.get("name").and_then(Item::as_str).unwrap().to_owned();
                    let values = param
                        .get("values")
                        .and_then(Item::as_array)
                        .map(|values| {
                            values
                                .iter()
                                .filter_map(|v| v.as_str().map(str::to_owned))
                                .collect()
                        })
                        .unwrap_or_default();
                    let default = param
                        .get("default")
                        .and_then(Item::as_str)
                        .map(str::to_owned);
                    params.insert(param_name, (values, default));
                }
                commands.insert(name, params);
            }
        }
        commands
    }

    /// Every stage of the table; option arguments are attached to the
    /// target command of their entry.
    fn stages_of(table: &BashTable) -> Vec<TemplateStage> {
        let mut stages: Vec<TemplateStage> = Vec::new();
        for command in &table.commands {
            let target = |args: &[TemplateArg]| TemplateStage {
                name: command.target.clone(),
                args: args.to_vec(),
            };
            if let Some(template) = &command.template {
                stages.extend(template.stages.iter().cloned());
            }
            for form in &command.forms {
                stages.extend(form.template.stages.iter().cloned());
            }
            for stage in &command.stages {
                stages.extend(stage.template.iter().cloned());
            }
            if let Some(glob) = &command.glob {
                stages.extend(glob.stages.iter().cloned());
            }
            for option in &command.options {
                stages.push(target(&option.args));
                stages.push(target(&option.absent));
                stages.extend(option.stages.iter().cloned());
            }
        }
        if let Some(source) = &table.glob_source {
            stages.extend(source.stages.iter().cloned());
        }
        stages.push(TemplateStage {
            name: "find".to_owned(),
            args: table.glob_hidden.clone(),
        });
        for map in ["sed", "awk"] {
            for value in table.values.get(map).into_iter().flat_map(BTreeMap::values) {
                let text = value.as_str().unwrap();
                stages.extend(parse_pipeline(text).unwrap().stages);
            }
        }
        stages.retain(|stage| !stage.name.is_empty());
        stages
    }

    #[test]
    fn commands_parameters_and_values_exist_in_the_catalog() {
        /// Commands of the group `administration`, which the translation of
        /// the administration programs writes; the catalog of that group is
        /// merged separately from this table.
        const ADMINISTRATION: &[&str] = &[
            "accounts.create",
            "accounts.modify",
            "accounts.remove",
            "accounts.password",
            "clock.ntp",
            "clock.set",
            "clock.timezone_set",
            "disks.format",
            "disks.partition",
            "firewall.allow",
            "firewall.deny",
            "firewall.remove",
            "groups.add_member",
            "groups.create",
            "groups.remove",
            "groups.remove_member",
            "modules.load",
            "modules.unload",
            "mount",
            "network.dns_set",
            "network.route_add",
            "network.route_remove",
            "power.cancel",
            "power.restart",
            "power.shutdown",
            "sysctl.set",
            "unmount",
        ];
        let catalog = catalog();
        let table = BashTable::embedded().unwrap();
        let mut missing = Vec::new();
        for stage in stages_of(table) {
            let Some(params) = catalog.get(&stage.name) else {
                if !ADMINISTRATION.contains(&stage.name.as_str()) {
                    missing.push(format!("command {}", stage.name));
                }
                continue;
            };
            for arg in &stage.args {
                let Some(name) = &arg.name else { continue };
                match params.get(name) {
                    None => missing.push(format!("parameter {}.{name}", stage.name)),
                    Some((values, _)) => {
                        if let super::super::template::Node::Const(
                            nurt_contracts::ast::Expr::Ident { v, .. },
                        ) = &arg.value
                            && !values.is_empty()
                            && !values.contains(v)
                        {
                            missing.push(format!("value {}.{name} = {v}", stage.name));
                        }
                    }
                }
            }
        }
        for name in BUILT_STAGES {
            if !catalog.contains_key(*name) && !ADMINISTRATION.contains(name) {
                missing.push(format!("command {name}"));
            }
        }
        for (command, params) in table.parameters() {
            for param in params {
                if !catalog
                    .get(command)
                    .is_some_and(|known| known.contains_key(param))
                {
                    missing.push(format!("listed parameter {command}.{param}"));
                }
            }
        }
        for command in table.effect_commands() {
            if !catalog.contains_key(command) && !ADMINISTRATION.contains(&command) {
                missing.push(format!("effect command {command}"));
            }
        }
        let types = &catalog["find"]["type"].0;
        for kind in table.values["find_type"].values() {
            let kind = kind.as_str().unwrap().to_owned();
            if !types.contains(&kind) {
                missing.push(format!("value find.type = {kind}"));
            }
        }
        assert!(missing.is_empty(), "{missing:#?}");
    }

    #[test]
    fn default_flags_match_catalog_defaults() {
        let catalog = catalog();
        let table = BashTable::embedded().unwrap();
        let mut wrong = Vec::new();
        for command in &table.commands {
            for option in command.options.iter().filter(|option| option.default) {
                for arg in &option.args {
                    let (Some(name), super::super::template::Node::Const(value)) =
                        (&arg.name, &arg.value)
                    else {
                        continue;
                    };
                    let written = match value {
                        nurt_contracts::ast::Expr::Ident { v, .. } => v.clone(),
                        nurt_contracts::ast::Expr::Bool { v, .. } => v.to_string(),
                        _ => continue,
                    };
                    let default = catalog
                        .get(&command.target)
                        .and_then(|params| params.get(name))
                        .and_then(|p| p.1.clone());
                    if default.as_deref() != Some(written.as_str()) {
                        wrong.push(format!(
                            "{} {:?}: {name} = {written}, catalog {default:?}",
                            command.words.join(" "),
                            option.flags
                        ));
                    }
                }
            }
        }
        assert!(wrong.is_empty(), "{wrong:#?}");
    }

    #[test]
    fn habit_words_cover_the_first_word_of_every_entry() {
        let table = BashTable::embedded().unwrap();
        for command in &table.commands {
            assert!(table.is_habit(&command.words[0]), "{}", command.words[0]);
        }
        for word in &table.shell_only {
            assert!(table.is_habit(word), "{word}");
        }
        let mut words = table.habits().to_vec();
        words.sort();
        words.dedup();
        assert_eq!(words.len(), table.habits().len());
    }

    #[test]
    fn rules_of_the_table_are_bash_rules() {
        let table = BashTable::embedded().unwrap();
        for rule in table.rules() {
            let number: u8 = rule.strip_prefix('B').unwrap().parse().unwrap();
            assert!((1..=44).contains(&number), "{rule}");
        }
    }

    #[test]
    fn glob_entries_name_their_operand_count() {
        let table = BashTable::embedded().unwrap();
        let counts: Vec<(String, usize)> = table
            .commands
            .iter()
            .filter_map(|command| {
                command
                    .glob
                    .as_ref()
                    .map(|glob| (command.words.join(" "), glob.operands))
            })
            .collect();
        assert!(counts.contains(&("rm".to_owned(), 1)));
        assert!(counts.contains(&("mv".to_owned(), 2)));
        assert!(counts.contains(&("chmod".to_owned(), 2)));
    }
}
