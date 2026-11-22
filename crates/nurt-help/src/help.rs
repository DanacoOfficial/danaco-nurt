// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The `help` topics: overview, command, group, error code, guide, syntax,
//! corrections and search.
//!
//! Help is generated from the catalog and the documentation. No command
//! requires it to be read before it runs.

use nurt_contracts::envelope::Language;
use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::spec::{CORE_GROUP, StageSpec};
use serde_json::{Value, json};

use crate::catalog::Catalog;
use crate::describe::{self, Layer, Length};
use crate::docs::Guide;
pub use crate::errors::help_reference;
use crate::errors::{NOTES, error_note};
use crate::lookup;
use crate::signature::{self, named_label, named_params, positional_label, value_hint};
use crate::text::{self, Labels};

/// Number of examples shown when the call gives none.
pub const DEFAULT_EXAMPLES: usize = 3;

/// Width at which lists of parameters are wrapped.
const WRAP_WIDTH: usize = 96;

/// Largest number of results of one kind in a search.
const MAX_SEARCH_RESULTS: usize = 8;

/// Score of a search word equal to a name.
const SCORE_NAME_EQUAL: u32 = 6;
/// Score of a search word contained in a name.
const SCORE_NAME_PART: u32 = 3;
/// Score of a search word contained in a summary or a description.
const SCORE_TEXT_PART: u32 = 2;
/// Score of a search word contained in a parameter name.
const SCORE_PARAM_PART: u32 = 1;

/// Shortest search word that is looked up.
const MIN_SEARCH_WORD: usize = 2;

/// Names of the `help` parameters that select a subject.
const SUBJECT_PARAMS: [&str; 4] = ["topic", "group", "skill", "search"];

/// Format of the answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum HelpFormat {
    /// Plain UTF-8 text.
    #[default]
    Text,
    /// A JSON document for tools.
    Json,
}

/// Kind of a help topic, the value of the parameter `kind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TopicKind {
    /// A command.
    Command,
    /// A group of commands.
    Group,
    /// An error code.
    Error,
    /// A guide.
    Skill,
    /// The grammar and the literals.
    Syntax,
    /// The correction rules.
    Corrections,
}

impl TopicKind {
    /// Every kind, in the order of the catalog enumeration.
    pub const ALL: [TopicKind; 6] = [
        Self::Command,
        Self::Group,
        Self::Error,
        Self::Skill,
        Self::Syntax,
        Self::Corrections,
    ];

    /// The name of the kind as written in a program.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Command => "command",
            Self::Group => "group",
            Self::Error => "error",
            Self::Skill => "skill",
            Self::Syntax => "syntax",
            Self::Corrections => "corrections",
        }
    }

    /// The kind with the given name.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.name() == name)
    }
}

/// Question put to `help`.
///
/// One of `topic`, `group`, `skill` and `search` is given at the latest; none
/// asks for the overview. `kind` is `None` when the call did not give it:
/// the topic is then looked up among commands, error codes, the topics
/// `syntax` and `corrections`, groups and guides, in this order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelpQuery {
    /// Command, group, error code, guide, `syntax` or `corrections`.
    pub topic: Option<String>,
    /// Kind of the topic, when stated.
    pub kind: Option<TopicKind>,
    /// Group to describe.
    pub group: Option<String>,
    /// Guide to show.
    pub skill: Option<String>,
    /// Words to look up.
    pub search: Option<String>,
    /// Number of examples of a command.
    pub examples: usize,
    /// Format of the answer.
    pub format: HelpFormat,
    /// Language of the fixed texts.
    pub language: Language,
}

impl Default for HelpQuery {
    fn default() -> Self {
        Self {
            topic: None,
            kind: None,
            group: None,
            skill: None,
            search: None,
            examples: DEFAULT_EXAMPLES,
            format: HelpFormat::Text,
            language: Language::En,
        }
    }
}

impl HelpQuery {
    /// Question for the overview.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Question about a topic.
    #[must_use]
    pub fn topic(topic: impl Into<String>) -> Self {
        Self {
            topic: Some(topic.into()),
            ..Self::default()
        }
    }

    /// States the kind of the topic.
    #[must_use]
    pub fn with_kind(mut self, kind: TopicKind) -> Self {
        self.kind = Some(kind);
        self
    }

    /// Sets the number of examples.
    #[must_use]
    pub fn with_examples(mut self, examples: usize) -> Self {
        self.examples = examples;
        self
    }

    /// Sets the format of the answer.
    #[must_use]
    pub fn with_format(mut self, format: HelpFormat) -> Self {
        self.format = format;
        self
    }

    /// Sets the language of the fixed texts.
    #[must_use]
    pub fn with_language(mut self, language: Language) -> Self {
        self.language = language;
        self
    }

    fn subjects(&self) -> Vec<&'static str> {
        let given = [
            self.topic.is_some(),
            self.group.is_some(),
            self.skill.is_some(),
            self.search.is_some(),
        ];
        SUBJECT_PARAMS
            .into_iter()
            .zip(given)
            .filter(|(_, given)| *given)
            .map(|(name, _)| name)
            .collect()
    }
}

/// Answer of `help`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelpText {
    /// Format of the body.
    pub format: HelpFormat,
    /// Text or the serialized JSON document.
    pub body: String,
}

enum Subject<'a> {
    Overview,
    Command(&'a StageSpec),
    Group(&'a str),
    Error(ErrorCode),
    Skill(&'a Guide),
    Syntax,
    Corrections,
    Search(&'a str),
}

/// Answers a help question.
///
/// # Errors
///
/// Fails with `E_UNKNOWN_NAME` and the nearest names in `expected` when the
/// topic does not exist, and with `E_PARAM_CONFLICT` when more than one of
/// `topic`, `group`, `skill` and `search` is given.
///
/// # Examples
///
/// ```no_run
/// use nurt_help::catalog::Catalog;
/// use nurt_help::help::{render, HelpQuery};
///
/// fn show(catalog: &Catalog) -> Result<(), nurt_contracts::error::Error> {
///     let answer = render(&HelpQuery::topic("replace"), catalog)?;
///     assert!(answer.body.starts_with("replace"));
///     Ok(())
/// }
/// ```
pub fn render(query: &HelpQuery, catalog: &Catalog) -> Result<HelpText, Error> {
    let subject = subject(query, catalog)?;
    let labels = text::labels(query.language);
    let (text, document) = match subject {
        Subject::Overview => overview(catalog, query.language, labels),
        Subject::Command(spec) => command(catalog, spec, query.examples, labels),
        Subject::Group(name) => group(catalog, name, query.language)?,
        Subject::Error(code) => error_topic(code, labels),
        Subject::Skill(guide) => skill(guide),
        Subject::Syntax => plain("syntax", "syntax", text::syntax(query.language).join("\n")),
        Subject::Corrections => plain(
            "corrections",
            "corrections",
            text::corrections(query.language).join("\n"),
        ),
        Subject::Search(words) => search(catalog, words, labels),
    };
    let body = match query.format {
        HelpFormat::Text => text,
        HelpFormat::Json => serde_json::to_string(&document).unwrap_or_default(),
    };
    Ok(HelpText {
        format: query.format,
        body,
    })
}

fn subject<'a>(query: &'a HelpQuery, catalog: &'a Catalog) -> Result<Subject<'a>, Error> {
    let given = query.subjects();
    if given.len() > 1 {
        return Err(Error::new(
            ErrorCode::ParamConflict,
            format!("give only one of {}", SUBJECT_PARAMS.join(", ")),
        )
        .with_expected(json!(SUBJECT_PARAMS)));
    }
    if let Some(words) = &query.search {
        return Ok(Subject::Search(words));
    }
    if let Some(name) = &query.group {
        return named_group(catalog, name);
    }
    if let Some(name) = &query.skill {
        return named_guide(catalog, name);
    }
    let Some(topic) = &query.topic else {
        return Ok(Subject::Overview);
    };
    match query.kind {
        Some(TopicKind::Command) => named_command(catalog, topic),
        Some(TopicKind::Group) => named_group(catalog, topic),
        Some(TopicKind::Error) => named_error(topic),
        Some(TopicKind::Skill) => named_guide(catalog, topic),
        Some(TopicKind::Syntax) => Ok(Subject::Syntax),
        Some(TopicKind::Corrections) => Ok(Subject::Corrections),
        None => detect(catalog, topic),
    }
}

fn named_command<'a>(catalog: &'a Catalog, name: &str) -> Result<Subject<'a>, Error> {
    catalog.command(name).map(Subject::Command).ok_or_else(|| {
        let names: Vec<String> = catalog.commands().map(|c| c.name.clone()).collect();
        lookup::unknown("command", name, &names)
    })
}

fn named_group<'a>(catalog: &'a Catalog, name: &str) -> Result<Subject<'a>, Error> {
    catalog
        .group(name)
        .map(|file| Subject::Group(file.group.as_str()))
        .ok_or_else(|| lookup::unknown("group", name, &group_names(catalog)))
}

fn named_guide<'a>(catalog: &'a Catalog, name: &str) -> Result<Subject<'a>, Error> {
    catalog
        .docs()
        .guide(name)
        .map(Subject::Skill)
        .ok_or_else(|| {
            let names: Vec<String> = catalog.docs().guides().map(|g| g.name.clone()).collect();
            lookup::unknown("guide", name, &names)
        })
}

fn named_error<'a>(name: &str) -> Result<Subject<'a>, Error> {
    name.parse::<ErrorCode>().map(Subject::Error).map_err(|_| {
        let names: Vec<String> = ErrorCode::ALL
            .iter()
            .map(|c| c.as_str().to_owned())
            .collect();
        lookup::unknown("error code", name, &names)
    })
}

fn group_names(catalog: &Catalog) -> Vec<String> {
    catalog.groups().map(|file| file.group.clone()).collect()
}

fn detect<'a>(catalog: &'a Catalog, topic: &str) -> Result<Subject<'a>, Error> {
    if let Some(spec) = catalog.command(topic) {
        return Ok(Subject::Command(spec));
    }
    if let Ok(code) = topic.parse::<ErrorCode>() {
        return Ok(Subject::Error(code));
    }
    match topic {
        "syntax" => return Ok(Subject::Syntax),
        "corrections" => return Ok(Subject::Corrections),
        _ => {}
    }
    if let Some(file) = catalog.group(topic) {
        return Ok(Subject::Group(file.group.as_str()));
    }
    if let Some(guide) = catalog.docs().guide(topic) {
        return Ok(Subject::Skill(guide));
    }
    let names: Vec<String> = if topic.starts_with("E_") {
        ErrorCode::ALL
            .iter()
            .map(|c| c.as_str().to_owned())
            .collect()
    } else {
        let mut names: Vec<String> = catalog.commands().map(|c| c.name.clone()).collect();
        names.extend(group_names(catalog));
        names.extend(catalog.docs().guides().map(|g| g.name.clone()));
        names.extend(["syntax".to_owned(), "corrections".to_owned()]);
        names
    };
    Err(lookup::unknown("help topic", topic, &names))
}

fn plain(kind: &str, name: &str, text: String) -> (String, Value) {
    let document = json!({ "kind": kind, "name": name, "text": text });
    (text, document)
}

fn core_names(catalog: &Catalog) -> Vec<&str> {
    catalog
        .core_commands()
        .iter()
        .map(|command| command.name.as_str())
        .collect()
}

fn overview(catalog: &Catalog, language: Language, labels: &Labels) -> (String, Value) {
    let first = text::first_line();
    let commands = core_names(catalog);
    let groups: Vec<&str> = catalog
        .groups()
        .map(|file| file.group.as_str())
        .filter(|name| *name != CORE_GROUP)
        .collect();
    let skills: Vec<&str> = catalog
        .docs()
        .guides()
        .map(|guide| guide.name.as_str())
        .collect();
    let mut lines = vec![first.clone(), String::new()];
    lines.extend(labels.intro.iter().map(|line| (*line).to_owned()));
    lines.push(String::new());
    lines.push(format!("{} {}", labels.commands, commands.join(" ")));
    lines.push(format!("{} {}", labels.groups, groups.join(" ")));
    if !skills.is_empty() {
        lines.push(format!("{} {}", labels.skills, skills.join(" ")));
    }
    lines.push(format!("{} {}", labels.topics, labels.topic_names));
    lines.push(String::new());
    lines.extend(labels.closing.iter().map(|line| (*line).to_owned()));
    let document = json!({
        "title": first,
        "language": language.as_str(),
        "commands": commands,
        "groups": groups,
        "skills": skills,
        "topics": ["syntax", "corrections"],
    });
    (lines.join("\n"), document)
}

fn parameter_items(spec: &StageSpec) -> Vec<String> {
    let positional = spec.positional().into_iter().filter_map(|param| {
        let hint = value_hint(param);
        (!hint.is_empty()).then(|| format!("{}:{hint}", positional_label(param, false)))
    });
    positional
        .chain(named_params(spec).map(named_label))
        .collect()
}

fn command(
    catalog: &Catalog,
    spec: &StageSpec,
    examples: usize,
    labels: &Labels,
) -> (String, Value) {
    let page = catalog.docs().page(&spec.name);
    let usages = signature::usages(spec);
    let mut lines = vec![format!("{} — {}", spec.name, spec.summary)];
    for usage in &usages {
        if usage.form.is_empty() {
            lines.push(format!("  {}", usage.text));
        } else {
            lines.push(format!("  {}  — {}", usage.text, usage.summary));
        }
    }
    let items = parameter_items(spec);
    if !items.is_empty() {
        let prefix = format!("  {} ", labels.params);
        let indent = " ".repeat(prefix.chars().count());
        let wrapped = signature::wrap(&items, &indent, WRAP_WIDTH);
        lines.push(format!("{prefix}{}", wrapped.trim_start()));
    }
    lines.push(format!(
        "  {} {}   {} {}",
        labels.input,
        signature::io_text(&spec.input),
        labels.output,
        signature::io_text(&spec.output)
    ));
    lines.push(format!(
        "  {} {}",
        labels.effect,
        signature::effect_text(spec)
    ));
    let shown: Vec<&str> = page
        .map(|page| {
            page.examples
                .iter()
                .take(examples)
                .map(|example| example.program.as_str())
                .collect()
        })
        .unwrap_or_default();
    if !shown.is_empty() {
        lines.push(labels.examples.to_owned());
        for program in &shown {
            lines.push(format!("  {}", program.replace('\n', "\n  ")));
        }
    }
    let errors: Vec<(&str, &str)> = page
        .map(|page| {
            page.errors
                .iter()
                .map(|note| (note.code.as_str(), note.meaning.as_str()))
                .collect()
        })
        .unwrap_or_default();
    if !errors.is_empty() {
        let items: Vec<String> = errors
            .iter()
            .map(|(code, meaning)| format!("{code} ({meaning})"))
            .collect();
        lines.push(format!("{} {}", labels.errors, items.join(" · ")));
    }
    let related: Vec<&str> = page
        .map(|page| page.related.iter().map(String::as_str).collect())
        .unwrap_or_default();
    if !related.is_empty() {
        lines.push(format!("{} {}", labels.related, related.join(", ")));
    }
    let document = command_document(spec, &usages, &shown, &errors, &related);
    (lines.join("\n"), document)
}

fn command_document(
    spec: &StageSpec,
    usages: &[signature::Usage],
    examples: &[&str],
    errors: &[(&str, &str)],
    related: &[&str],
) -> Value {
    let params: Vec<Value> = spec
        .params
        .iter()
        .map(|param| {
            json!({
                "name": param.name,
                "type": param.ty.to_string(),
                "position": param.position,
                "required": param.required,
                "default": param.default.as_ref().map(|value| value.as_str().to_owned()),
                "values": param.values,
                "summary": param.summary,
            })
        })
        .collect();
    json!({
        "command": spec.name,
        "group": spec.group,
        "summary": spec.summary,
        "usage": usages.iter().map(|usage| usage.text.clone()).collect::<Vec<_>>(),
        "params": params,
        "input": spec.input.iter().map(ToString::to_string).collect::<Vec<_>>(),
        "output": spec.output.iter().map(ToString::to_string).collect::<Vec<_>>(),
        "effect": signature::effect_text(spec),
        "examples": examples,
        "errors": errors
            .iter()
            .map(|(code, meaning)| json!({ "code": code, "meaning": meaning }))
            .collect::<Vec<_>>(),
        "related": related,
    })
}

fn group(catalog: &Catalog, name: &str, language: Language) -> Result<(String, Value), Error> {
    let description = describe::layer(catalog, Layer::Group, name, language, Length::Default)?;
    let commands: Vec<Value> = catalog
        .group(name)
        .map(|file| {
            file.commands
                .iter()
                .map(|command| json!({ "name": command.name, "summary": command.summary }))
                .collect()
        })
        .unwrap_or_default();
    let document = json!({
        "kind": "group",
        "name": name,
        "commands": commands,
        "text": description.text,
    });
    Ok((description.text, document))
}

fn skill(guide: &Guide) -> (String, Value) {
    let text = guide.render();
    let document = json!({
        "kind": "skill",
        "name": guide.name,
        "description": guide.description,
        "commands": guide.commands,
        "text": text,
    });
    (text, document)
}

fn error_topic(code: ErrorCode, labels: &Labels) -> (String, Value) {
    let note = error_note(code);
    let meaning = note.map_or("", |note| note.meaning);
    let fix = note.map_or("", |note| note.fix);
    let stages: Vec<String> = code
        .stages()
        .iter()
        .map(|stage| format!("{} {}", stage.as_str(), stage.name()))
        .collect();
    let text = format!(
        "{code} — {meaning}\n{} {}\n{} {fix}",
        labels.stage,
        stages.join(" · "),
        labels.fix
    );
    let document = json!({
        "kind": "error",
        "code": code.as_str(),
        "meaning": meaning,
        "stages": code
            .stages()
            .iter()
            .map(|stage| json!({ "number": stage.as_str(), "name": stage.name() }))
            .collect::<Vec<_>>(),
        "fix": fix,
    });
    (text, document)
}

fn search_words(text: &str) -> Vec<String> {
    text.split(|ch: char| !ch.is_alphanumeric() && ch != '_')
        .filter(|word| word.chars().count() >= MIN_SEARCH_WORD)
        .map(str::to_lowercase)
        .collect()
}

fn score_text(words: &[String], name: &str, text: &str) -> u32 {
    let name = name.to_lowercase();
    let text = text.to_lowercase();
    words
        .iter()
        .map(|word| {
            let on_name = if name == *word {
                SCORE_NAME_EQUAL
            } else if name.contains(word.as_str()) {
                SCORE_NAME_PART
            } else {
                0
            };
            on_name
                + if text.contains(word.as_str()) {
                    SCORE_TEXT_PART
                } else {
                    0
                }
        })
        .sum()
}

fn score_command(words: &[String], spec: &StageSpec) -> u32 {
    let params: u32 = spec
        .params
        .iter()
        .map(|param| {
            words
                .iter()
                .filter(|word| param.name.contains(word.as_str()))
                .map(|_| SCORE_PARAM_PART)
                .sum::<u32>()
        })
        .sum();
    score_text(words, &spec.name, &spec.summary) + params
}

/// Items with a positive score, best first, ties in alphabetical order.
fn ranked(mut scored: Vec<(u32, String, String)>) -> Vec<(String, String)> {
    scored.retain(|(score, _, _)| *score > 0);
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    scored
        .into_iter()
        .take(MAX_SEARCH_RESULTS)
        .map(|(_, name, text)| (name, text))
        .collect()
}

fn search(catalog: &Catalog, query: &str, labels: &Labels) -> (String, Value) {
    let words = search_words(query);
    let commands = ranked(
        catalog
            .commands()
            .map(|spec| {
                (
                    score_command(&words, spec),
                    spec.name.clone(),
                    spec.summary.clone(),
                )
            })
            .collect(),
    );
    let errors = ranked(
        NOTES
            .iter()
            .map(|note| {
                let code = note.code.as_str();
                (
                    score_text(&words, code, note.meaning),
                    code.to_owned(),
                    note.meaning.to_owned(),
                )
            })
            .collect(),
    );
    let skills = ranked(
        catalog
            .docs()
            .guides()
            .map(|guide| {
                let text = format!("{} {}", guide.description, guide.commands.join(" "));
                (
                    score_text(&words, &guide.name, &text),
                    guide.name.clone(),
                    guide.description.clone(),
                )
            })
            .collect(),
    );
    let mut lines = vec![format!("{} {query}", labels.search_for)];
    let sections = [
        (labels.commands, &commands),
        (labels.errors, &errors),
        (labels.skills, &skills),
    ];
    for (label, items) in sections {
        if items.is_empty() {
            continue;
        }
        lines.push(label.to_owned());
        lines.extend(
            items
                .iter()
                .map(|(name, text)| format!("  {name} — {text}")),
        );
    }
    if commands.is_empty() && errors.is_empty() && skills.is_empty() {
        lines.push(labels.no_matches.to_owned());
    }
    let listing = |items: &[(String, String)]| -> Vec<Value> {
        items
            .iter()
            .map(|(name, text)| json!({ "name": name, "summary": text }))
            .collect()
    };
    let document = json!({
        "search": query,
        "commands": listing(&commands),
        "errors": listing(&errors),
        "skills": listing(&skills),
    });
    (lines.join("\n"), document)
}

/// Note shown after the result of a command whose group is not loaded:
/// `note: group documents is described by: help group: documents`.
///
/// Returns `None` for a command of the core group, of a loaded group or
/// unknown to the catalog.
#[must_use]
pub fn group_note(catalog: &Catalog, command: &str, loaded_groups: &[&str]) -> Option<String> {
    let spec = catalog.command(command)?;
    let group = spec.group.as_str();
    if group == CORE_GROUP || loaded_groups.contains(&group) {
        return None;
    }
    Some(format!(
        "note: group {group} is described by: help group: {group}"
    ))
}
