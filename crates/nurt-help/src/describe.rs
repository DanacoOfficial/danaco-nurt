// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Generator of the description layers.
//!
//! The layers are generated from the catalog and the documentation, never
//! written by hand: the core layer, the description of each group, the
//! guides and the reference. The core and group layers can be fitted to a
//! target length: the generator adds or drops detail in a fixed order until
//! the estimated token count is the nearest to the target, so the same
//! catalog and length always give the same bytes.

use nurt_contracts::envelope::Language;
use nurt_contracts::error::Error;
use nurt_contracts::kinds::{Category, EffectKind};
use nurt_contracts::spec::{CORE_GROUP, CatalogFile, ParamSpec, StageSpec};

use crate::catalog::Catalog;
use crate::lookup;
use crate::signature::{self, named_params};
use crate::text::{self, Block};
use crate::tokens::{EstimateCounter, TokenCounter};
use crate::variant::{self, Variant};

/// Token budget of the English core layer.
pub const CORE_TOKENS_EN: u32 = 2230;

/// Token budget of the Polish core layer.
pub const CORE_TOKENS_PL: u32 = 2640;

/// Token budget of a group layer when no length is given.
pub const GROUP_TOKENS: u32 = 320;

/// Target lengths used by the syntax test, in tokens.
pub const TARGET_LENGTHS: [u32; 4] = [1000, 2000, 3000, 4000];

/// Layer of the documentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Layer {
    /// Core description: rules, syntax and the core commands.
    Core,
    /// Description of one group of commands.
    Group,
    /// One guide.
    Guide,
    /// Reference in Markdown: one command or, with an empty name, all.
    Reference,
}

/// Length of a generated layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Length {
    /// The default length of the layer.
    #[default]
    Default,
    /// Target length in tokens.
    Tokens(u32),
}

/// A generated layer with its token count.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Description {
    /// Text of the layer.
    pub text: String,
    /// Number of tokens of the text.
    pub tokens: u32,
    /// The count is an estimate, not the count of a real tokenizer.
    pub estimate: bool,
    /// Language of the text.
    pub language: Language,
}

/// Generates a layer with the estimating token counter.
///
/// `name` is the group for [`Layer::Group`], the guide for [`Layer::Guide`]
/// and the command for [`Layer::Reference`]; it is ignored for
/// [`Layer::Core`]. The length applies to the core and group layers.
///
/// # Errors
///
/// Fails with `E_UNKNOWN_NAME` when the group, guide or command does not
/// exist.
pub fn layer(
    catalog: &Catalog,
    layer: Layer,
    name: &str,
    language: Language,
    length: Length,
) -> Result<Description, Error> {
    layer_with_counter(&EstimateCounter, catalog, layer, name, language, length)
}

/// Generates a layer like [`layer`] with the programs of the examples
/// written in `variant`.
///
/// The examples are the same programs in every variant: they are parsed in
/// variant A and printed in `variant`, so the layer differs only in the
/// notation. The length is fitted to the notation that is shown.
///
/// # Errors
///
/// Fails like [`layer`], and with `E_INTERNAL` when an example does not
/// convert to `variant`.
pub fn layer_in_variant(
    catalog: &Catalog,
    layer: Layer,
    name: &str,
    language: Language,
    length: Length,
    variant: Variant,
) -> Result<Description, Error> {
    build(
        &EstimateCounter,
        catalog,
        layer,
        name,
        language,
        length,
        variant,
    )
}

/// Generates a layer and counts its tokens with `counter`.
///
/// # Errors
///
/// Fails with `E_UNKNOWN_NAME` when the group, guide or command does not
/// exist.
pub fn layer_with_counter(
    counter: &dyn TokenCounter,
    catalog: &Catalog,
    layer: Layer,
    name: &str,
    language: Language,
    length: Length,
) -> Result<Description, Error> {
    build(counter, catalog, layer, name, language, length, Variant::A)
}

fn build(
    counter: &dyn TokenCounter,
    catalog: &Catalog,
    layer: Layer,
    name: &str,
    language: Language,
    length: Length,
    variant: Variant,
) -> Result<Description, Error> {
    let text = match layer {
        Layer::Core => core_text(counter, catalog, language, length, variant)?,
        Layer::Group => group_text(counter, catalog, name, language, length, variant)?,
        Layer::Guide => guide_text(catalog, name, variant)?,
        Layer::Reference => reference_text(catalog, name)?,
    };
    Ok(Description {
        tokens: counter.count(&text),
        estimate: counter.is_estimate(),
        language,
        text,
    })
}

fn pick(pair: (&'static str, &'static str), language: Language) -> &'static str {
    match language {
        Language::Pl => pair.1,
        _ => pair.0,
    }
}

/// Largest prefix of the steps whose text does not exceed the target, or
/// the next one when it is nearer to the target.
fn fit(
    counter: &dyn TokenCounter,
    target: u32,
    steps: usize,
    render: &dyn Fn(usize) -> String,
) -> String {
    let base = render(0);
    if steps == 0 || counter.count(&base) >= target {
        return base;
    }
    let (mut low, mut high) = (0usize, steps);
    while low < high {
        let middle = (low + high).div_ceil(2);
        if counter.count(&render(middle)) <= target {
            low = middle;
        } else {
            high = middle - 1;
        }
    }
    let best = render(low);
    if low < steps {
        let next = render(low + 1);
        let best_gap = target.abs_diff(counter.count(&best));
        if target.abs_diff(counter.count(&next)) < best_gap {
            return next;
        }
    }
    best
}

fn target_tokens(length: Length, default: u32) -> u32 {
    match length {
        Length::Default => default,
        Length::Tokens(tokens) => tokens,
    }
}

/// One increment of detail of the core layer.
#[derive(Debug, Clone, Copy)]
enum Step {
    Line { block: usize, line: usize },
    Groups(u8),
    Param { command: usize, index: usize },
    Summary(usize),
}

/// Parameters of a command added in each round of the core layer.
const PARAM_ROUNDS_BEFORE_EXAMPLES: usize = 2;

/// Rounds after which the detail lines of priority 3 and 4 are added.
const PARAM_ROUNDS_BEFORE_DETAIL: usize = 4;

struct CoreModel<'a> {
    blocks: Vec<&'static Block>,
    /// Programs of the blocks that hold programs, in the shown variant.
    converted: Vec<Vec<Option<String>>>,
    commands: &'a [StageSpec],
    groups: Vec<(&'a str, &'a str)>,
    language: Language,
}

struct Selection {
    lines: Vec<Vec<bool>>,
    groups: u8,
    params: Vec<usize>,
    summaries: Vec<bool>,
}

impl<'a> CoreModel<'a> {
    fn new(catalog: &'a Catalog, language: Language, variant: Variant) -> Result<Self, Error> {
        let blocks: Vec<&'static Block> = text::HEAD
            .iter()
            .chain(text::MIDDLE)
            .chain(text::TAIL)
            .collect();
        let groups = catalog
            .groups()
            .filter(|file| file.group != CORE_GROUP)
            .map(|file| (file.group.as_str(), file.summary.as_str()))
            .collect();
        let converted = blocks
            .iter()
            .map(|block| {
                block
                    .lines
                    .iter()
                    .map(|line| {
                        if block.programs {
                            variant::convert_program(catalog, line.text(language), variant)
                                .map(Some)
                        } else {
                            Ok(None)
                        }
                    })
                    .collect::<Result<Vec<_>, Error>>()
            })
            .collect::<Result<Vec<_>, Error>>()?;
        Ok(Self {
            blocks,
            converted,
            commands: catalog.core_commands(),
            groups,
            language,
        })
    }

    fn lines_of_priority(&self, priority: u32) -> Vec<Step> {
        let mut steps = Vec::new();
        for (block, content) in self.blocks.iter().enumerate() {
            for (line, item) in content.lines.iter().enumerate() {
                if item.priority == priority {
                    steps.push(Step::Line { block, line });
                }
            }
        }
        steps
    }

    fn param_round(&self, round: usize) -> Vec<Step> {
        self.commands
            .iter()
            .enumerate()
            .filter(|(_, command)| named_params(command).count() > round)
            .map(|(command, _)| Step::Param {
                command,
                index: round,
            })
            .collect()
    }

    fn max_rounds(&self) -> usize {
        self.commands
            .iter()
            .map(|command| named_params(command).count())
            .max()
            .unwrap_or(0)
    }

    /// Order in which detail is added: the optional rules, one round of
    /// parameters, the records and the first examples, more parameters, the
    /// remaining examples, summaries and finally the remaining parameters.
    fn plan(&self) -> Vec<Step> {
        let mut steps = self.lines_of_priority(1);
        steps.push(Step::Groups(1));
        steps.extend(self.param_round(0));
        steps.extend(self.lines_of_priority(2));
        for round in 1..PARAM_ROUNDS_BEFORE_EXAMPLES {
            steps.extend(self.param_round(round));
        }
        steps.extend(self.lines_of_priority(3));
        for round in PARAM_ROUNDS_BEFORE_EXAMPLES..PARAM_ROUNDS_BEFORE_DETAIL {
            steps.extend(self.param_round(round));
        }
        steps.extend(self.lines_of_priority(4));
        steps.push(Step::Groups(2));
        steps.extend((0..self.commands.len()).map(Step::Summary));
        for round in PARAM_ROUNDS_BEFORE_DETAIL..self.max_rounds() {
            steps.extend(self.param_round(round));
        }
        steps
    }

    fn select(&self, steps: &[Step]) -> Selection {
        let mut selection = Selection {
            lines: self
                .blocks
                .iter()
                .map(|block| block.lines.iter().map(|line| line.priority == 0).collect())
                .collect(),
            groups: 0,
            params: vec![0; self.commands.len()],
            summaries: vec![false; self.commands.len()],
        };
        for step in steps {
            match *step {
                Step::Line { block, line } => selection.lines[block][line] = true,
                Step::Groups(level) => selection.groups = level,
                Step::Param { command, index } => selection.params[command] = index + 1,
                Step::Summary(command) => selection.summaries[command] = true,
            }
        }
        selection
    }

    fn render_block(&self, block: usize, selection: &Selection, out: &mut Vec<String>) {
        let content = self.blocks[block];
        let shown: Vec<String> = content
            .lines
            .iter()
            .zip(&selection.lines[block])
            .zip(&self.converted[block])
            .filter(|((_, on), _)| **on)
            .map(|((line, _), program)| {
                program
                    .clone()
                    .unwrap_or_else(|| line.text(self.language).to_owned())
            })
            .collect();
        if shown.is_empty() {
            return;
        }
        let mut lines: Vec<String> = Vec::new();
        if let Some(header) = content.header(self.language) {
            lines.push(header.to_owned());
        }
        lines.extend(shown);
        out.push(lines.join("\n"));
    }

    fn render_commands(&self, selection: &Selection) -> String {
        let mut lines = vec![text::COMMANDS_LEGEND.text(self.language).to_owned()];
        let sections = [
            (text::SOURCES_HEADING, 0usize),
            (text::STAGES_HEADING, 1),
            (text::CHANGES_HEADING, 2),
        ];
        for (heading, section) in sections {
            lines.push(pick(heading, self.language).to_owned());
            for (index, command) in self.commands.iter().enumerate() {
                if command_section(command) != section {
                    continue;
                }
                lines.push(command_line(
                    command,
                    selection.params[index],
                    selection.summaries[index],
                ));
            }
        }
        lines.join("\n")
    }

    fn render_groups(&self, selection: &Selection) -> Option<String> {
        if selection.groups == 0 {
            return None;
        }
        let mut lines = vec![text::GROUPS_LEAD.text(self.language).to_owned()];
        if selection.groups == 1 {
            let names: Vec<&str> = self.groups.iter().map(|(name, _)| *name).collect();
            lines.push(names.join(" "));
        } else {
            lines.extend(
                self.groups
                    .iter()
                    .map(|(name, summary)| format!("{name} — {summary}")),
            );
        }
        Some(lines.join("\n"))
    }

    fn render(&self, steps: &[Step]) -> String {
        let selection = self.select(steps);
        let mut parts: Vec<String> = Vec::new();
        let commands_at = text::HEAD.len();
        let tail_at = text::HEAD.len() + text::MIDDLE.len();
        for block in 0..self.blocks.len() {
            if block == commands_at {
                parts.push(self.render_commands(&selection));
            }
            if block == tail_at {
                parts.extend(self.render_groups(&selection));
            }
            self.render_block(block, &selection, &mut parts);
        }
        parts.join("\n\n")
    }
}

/// Section of the command list: 0 sources, 1 stages, 2 changes.
fn command_section(command: &StageSpec) -> usize {
    match command.category {
        Category::Source | Category::Help => 0,
        Category::Transform | Category::Check => 1,
        Category::Change | Category::Run | Category::System => 2,
        Category::Journal => usize::from(command.effect != EffectKind::Read) * 2,
    }
}

fn command_line(command: &StageSpec, named: usize, summary: bool) -> String {
    let signature = signature::compact(command, named);
    if summary {
        format!("{signature} — {}", command.summary)
    } else {
        signature
    }
}

fn core_text(
    counter: &dyn TokenCounter,
    catalog: &Catalog,
    language: Language,
    length: Length,
    variant: Variant,
) -> Result<String, Error> {
    let default = match language {
        Language::Pl => CORE_TOKENS_PL,
        _ => CORE_TOKENS_EN,
    };
    let model = CoreModel::new(catalog, language, variant)?;
    let plan = model.plan();
    let groups: Vec<GroupModel<'_>> = catalog
        .groups()
        .filter(|file| file.group != CORE_GROUP)
        .map(|file| GroupModel::new(catalog, file, variant))
        .collect::<Result<_, _>>()?;
    let extension: usize = groups.iter().map(|group| group.plan.len()).sum();
    Ok(fit(
        counter,
        target_tokens(length, default),
        plan.len() + extension,
        &|count| {
            if count <= plan.len() {
                return model.render(&plan[..count]);
            }
            let mut text = model.render(&plan);
            let mut left = count - plan.len();
            for group in &groups {
                let take = left.min(group.plan.len());
                if take == 0 {
                    break;
                }
                text.push_str("\n\n");
                text.push_str(&group.render(language, take));
                left -= take;
            }
            text
        },
    ))
}

/// A group with the order in which its detail is added.
struct GroupModel<'a> {
    file: &'a CatalogFile,
    examples: Vec<String>,
    plan: Vec<GroupStep>,
}

impl<'a> GroupModel<'a> {
    fn new(catalog: &Catalog, file: &'a CatalogFile, variant: Variant) -> Result<Self, Error> {
        let examples = group_examples(&file.group)
            .into_iter()
            .map(|program| variant::convert_program(catalog, program, variant))
            .collect::<Result<Vec<_>, _>>()?;
        let plan = group_plan(&file.commands, examples.len());
        Ok(Self {
            file,
            examples,
            plan,
        })
    }

    fn render(&self, language: Language, steps: usize) -> String {
        render_group(
            &self.file.group,
            &self.file.summary,
            &self.file.commands,
            &self.examples,
            language,
            &self.plan[..steps],
        )
    }
}

/// Note printed under the heading of a group: group, English, Polish.
const GROUP_NOTES: &[(&str, &str, &str)] = &[
    (
        "documents",
        "Layout, comments, key order and style are preserved; only the bytes of the changed key change. Key forms: a.b · a[0].b · a[*] · a.** · a[\"dotted.key\"] · a[?name == \"x\"] · a[+] appends to a list. SQLite databases through sql.",
        "Układ, komentarze, kolejność kluczy i styl są zachowane; zmieniają się tylko bajty zmienianego klucza. Formy klucza: a.b · a[0].b · a[*] · a.** · a[\"dotted.key\"] · a[?name == \"x\"] · a[+] dopisuje do listy. Bazy SQLite przez sql.",
    ),
    (
        "code",
        "Syntax analysis is built in; language servers are used when installed (source: lsp), otherwise local syntax analysis (source: syntax).",
        "Analiza składni jest wbudowana; serwery języków są używane, gdy są zainstalowane (source: lsp), w przeciwnym razie lokalna analiza składni (source: syntax).",
    ),
    (
        "git",
        "All operations are built in and work without an installed git. fetch, clone, pull and push act outside the local file system.",
        "Wszystkie operacje są wbudowane i działają bez zainstalowanego gita. fetch, clone, pull i push działają poza lokalnym systemem plików.",
    ),
    (
        "archive",
        "Formats: zip tar (gz zst xz bz2) 7z jar whl apk. Entries outside into are skipped and reported.",
        "Formaty: zip tar (gz zst xz bz2) 7z jar whl apk. Wpisy poza into są pomijane i raportowane.",
    ),
    (
        "admin",
        "Registry paths such as \"registry:/HKCU/Software/X\" work in list, find, read, write, remove, copy and move.",
        "Ścieżki rejestru, na przykład \"registry:/HKCU/Software/X\", działają w list, find, read, write, remove, copy i move.",
    ),
    (
        "package",
        "install, remove and upgrade call the package manager of the system and cannot be undone.",
        "install, remove i upgrade wywołują menedżera pakietów systemu i nie dają się cofnąć.",
    ),
    (
        "network",
        "Methods other than get and head change the remote system and cannot be undone.",
        "Metody inne niż get i head zmieniają zdalny system i nie dają się cofnąć.",
    ),
    (
        "schedule",
        "Schedules run only while nurt service is running; the result says active: false otherwise.",
        "Harmonogramy działają tylko wtedy, gdy działa nurt service; w przeciwnym razie wynik zawiera active: false.",
    ),
    (
        "journal",
        "Only the command journal.status shows the state of the journal: entries, size held, oldest entry and active limits.",
        "Tylko polecenie journal.status pokazuje stan dziennika: wpisy, zajęte miejsce, najstarszy wpis i aktywne limity.",
    ),
    (
        "verify",
        "Typed wrappers over test frameworks and analyzers; the output is parsed into test_result and finding records, and runs are stored for show and rerun.",
        "Typowane opakowania frameworków testowych i analizatorów; wyjście jest parsowane do rekordów test_result i finding, a przebiegi są zapamiętywane dla show i ponownego uruchomienia.",
    ),
];

/// Example programs of the groups: group, program.
const GROUP_EXAMPLES: &[(&str, &str)] = &[
    (
        "documents",
        r#"set "/srv/app/package.json" key: "scripts.test" value: "vitest""#,
    ),
    (
        "documents",
        r#"read "/srv/app/values.yaml" key: "image.tag""#,
    ),
    (
        "documents",
        r#"sql "/srv/app/app.db" "select id, name from users where active = 1 limit 5""#,
    ),
    ("text", r#"diff "/srv/app/a.go" "/srv/app.orig/a.go""#),
    (
        "text",
        r#"convert "/srv/legacy/readme.txt" encoding: "utf-8" line_ending: lf"#,
    ),
    ("text", r#"find "/srv/app" |> summarize size by: type"#),
    ("text", "evaluate now() - 3d"),
    (
        "code",
        r#"code.symbols "oldName" path: ["/srv/app/src"] kind: function |> code.rename "newName""#,
    ),
    (
        "code",
        r#"code.references "/srv/app/src/a.go" line: 12 column: 6"#,
    ),
    (
        "code",
        r#"code.diagnostics "/srv/app/src/a.go" severity: warning"#,
    ),
    (
        "notebook",
        r#"notebook.replace "/srv/a.ipynb" index: 3 source: "print(1)""#,
    ),
    ("git", r#"git.status "/srv/app""#),
    (
        "git",
        r#"git.log "/srv/app" count: 5 path: ["/srv/app/a.go"]"#,
    ),
    (
        "git",
        r#"git.commit "/srv/app" message: "fix: parser" paths: ["/srv/app/a.go"]"#,
    ),
    (
        "archive",
        r#"archive.extract "/srv/dl/data.tar.gz" into: "/srv/data""#,
    ),
    ("system", "ports state: listen |> filter port == 8080"),
    (
        "system",
        r#"processes name: "node" |> filter memory > 500mb |> select pid name memory"#,
    ),
    ("system", "disks |> filter free < 10gib"),
    ("admin", r#"services name: "nginx" |> restart"#),
    ("package", r#"package.install ["jq"] manager: apt"#),
    (
        "network",
        r#"http.request "https://example.com/health" |> assert count: 1"#,
    ),
    (
        "network",
        r#"http.request "https://api.example.com/items" method: post json: {a: 1}"#,
    ),
    (
        "schedule",
        r#"schedule.create "backup" cron: "0 3 * * *" action: (run ["backup"] cwd: "/srv")"#,
    ),
    ("journal", "journal.status"),
    ("program", r#"program.find "git""#),
    (
        "verify",
        r#"test.run "/srv/app" filter: "parser" |> filter status == "failed""#,
    ),
];

/// Example programs of a group, in the order of the table.
#[must_use]
pub fn group_examples(group: &str) -> Vec<&'static str> {
    GROUP_EXAMPLES
        .iter()
        .filter(|(name, _)| *name == group)
        .map(|(_, program)| *program)
        .collect()
}

/// Detail added to a group layer.
#[derive(Debug, Clone, Copy)]
enum GroupStep {
    Param { command: usize, index: usize },
    Summary(usize),
    Example(usize),
}

/// Parameter rounds that precede the summaries in a group layer.
const GROUP_ROUNDS_BEFORE_SUMMARIES: usize = 2;

fn group_plan(commands: &[StageSpec], examples: usize) -> Vec<GroupStep> {
    let rounds = |from: usize, to: usize, steps: &mut Vec<GroupStep>| {
        for round in from..to {
            for (command, spec) in commands.iter().enumerate() {
                if named_params(spec).count() > round {
                    steps.push(GroupStep::Param {
                        command,
                        index: round,
                    });
                }
            }
        }
    };
    let longest = commands
        .iter()
        .map(|spec| named_params(spec).count())
        .max()
        .unwrap_or(0);
    let mut steps = Vec::new();
    rounds(0, 1, &mut steps);
    steps.extend((0..examples.min(1)).map(GroupStep::Example));
    rounds(1, GROUP_ROUNDS_BEFORE_SUMMARIES, &mut steps);
    steps.extend((1..examples).map(GroupStep::Example));
    steps.extend((0..commands.len()).map(GroupStep::Summary));
    rounds(GROUP_ROUNDS_BEFORE_SUMMARIES, longest, &mut steps);
    steps
}

fn render_group(
    name: &str,
    summary: &str,
    commands: &[StageSpec],
    examples: &[String],
    language: Language,
    steps: &[GroupStep],
) -> String {
    let mut params = vec![0usize; commands.len()];
    let mut summaries = vec![false; commands.len()];
    let mut shown_examples: Vec<usize> = Vec::new();
    for step in steps {
        match *step {
            GroupStep::Param { command, index } => params[command] = index + 1,
            GroupStep::Summary(command) => summaries[command] = true,
            GroupStep::Example(index) => shown_examples.push(index),
        }
    }
    shown_examples.sort_unstable();
    let mut lines = vec![format!("GROUP {name}"), summary.to_owned()];
    if let Some((_, en, pl)) = GROUP_NOTES.iter().find(|(group, _, _)| *group == name) {
        lines.push(
            match language {
                Language::Pl => *pl,
                _ => *en,
            }
            .to_owned(),
        );
    }
    for (index, command) in commands.iter().enumerate() {
        lines.push(command_line(command, params[index], summaries[index]));
    }
    if !shown_examples.is_empty() {
        lines.push(pick(("Examples:", "Przykłady:"), language).to_owned());
        lines.extend(shown_examples.iter().map(|index| examples[*index].clone()));
    }
    lines.join("\n")
}

fn group_text(
    counter: &dyn TokenCounter,
    catalog: &Catalog,
    name: &str,
    language: Language,
    length: Length,
    variant: Variant,
) -> Result<String, Error> {
    let Some(file) = catalog.group(name) else {
        let names: Vec<String> = catalog.groups().map(|file| file.group.clone()).collect();
        return Err(lookup::unknown("group", name, &names));
    };
    let group = GroupModel::new(catalog, file, variant)?;
    Ok(fit(
        counter,
        target_tokens(length, GROUP_TOKENS),
        group.plan.len(),
        &|count| group.render(language, count),
    ))
}

fn guide_text(catalog: &Catalog, name: &str, variant: Variant) -> Result<String, Error> {
    match catalog.docs().guide(name) {
        Some(guide) => variant::convert_fenced(catalog, &guide.render(), variant),
        None => {
            let names: Vec<String> = catalog
                .docs()
                .guides()
                .map(|guide| guide.name.clone())
                .collect();
            Err(lookup::unknown("guide", name, &names))
        }
    }
}

/// Markdown reference of one command, or of all commands for an empty name.
pub(crate) fn reference_text(catalog: &Catalog, name: &str) -> Result<String, Error> {
    if name.is_empty() {
        let pages: Vec<String> = catalog.commands().map(reference_page).collect();
        return Ok(format!("# Command reference\n\n{}", pages.join("\n\n")));
    }
    match catalog.command(name) {
        Some(command) => Ok(reference_page(command)),
        None => {
            let names: Vec<String> = catalog.commands().map(|c| c.name.clone()).collect();
            Err(lookup::unknown("command", name, &names))
        }
    }
}

pub(crate) fn default_cell(param: &ParamSpec) -> String {
    match (&param.default, param.required) {
        (Some(value), _) => value.as_str().to_owned(),
        (None, true) => "required".to_owned(),
        (None, false) => "-".to_owned(),
    }
}

fn escape_cell(text: &str) -> String {
    text.replace('|', "\\|")
}

fn reference_page(command: &StageSpec) -> String {
    let mut lines = vec![
        format!("## {}", command.name),
        String::new(),
        command.summary.clone(),
        String::new(),
        "```text".to_owned(),
    ];
    lines.extend(
        signature::usages(command)
            .into_iter()
            .map(|usage| usage.text),
    );
    lines.push("```".to_owned());
    lines.push(String::new());
    lines.push(format!(
        "Group: `{}`. Input: {}. Output: {}. Effect: {}.",
        command.group,
        signature::io_text(&command.input),
        signature::io_text(&command.output),
        signature::effect_text(command),
    ));
    if !command.params.is_empty() {
        lines.push(String::new());
        lines.push("| Parameter | Type | Default | Description |".to_owned());
        lines.push("|---|---|---|---|".to_owned());
        for param in &command.params {
            let default = match (&param.default, param.required) {
                (Some(value), _) => format!("`{}`", escape_cell(value.as_str())),
                (None, true) => "required".to_owned(),
                (None, false) => "-".to_owned(),
            };
            lines.push(format!(
                "| `{}` | `{}` | {} | {} |",
                param.name,
                escape_cell(&param.ty.to_string()),
                default,
                escape_cell(&param.summary)
            ));
        }
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    struct ByteCounter;

    impl TokenCounter for ByteCounter {
        fn count(&self, text: &str) -> u32 {
            u32::try_from(text.len()).unwrap_or(u32::MAX)
        }

        fn is_estimate(&self) -> bool {
            false
        }

        fn tokenizer(&self) -> &str {
            "bytes"
        }
    }

    #[test]
    fn fit_returns_the_largest_prefix_within_the_target() {
        let text = fit(&ByteCounter, 10, 20, &|count| "x".repeat(count));
        assert_eq!(text.len(), 10);
    }

    #[test]
    fn fit_returns_the_base_when_it_exceeds_the_target() {
        let text = fit(&ByteCounter, 3, 20, &|count| "x".repeat(count + 5));
        assert_eq!(text.len(), 5);
    }

    #[test]
    fn fit_takes_the_next_step_when_it_is_nearer() {
        let text = fit(&ByteCounter, 10, 5, &|count| "x".repeat(count * 6));
        assert_eq!(text.len(), 12);
    }

    #[test]
    fn group_examples_are_selected_by_group() {
        assert_eq!(group_examples("journal"), ["journal.status"]);
        assert!(group_examples("core").is_empty());
    }

    #[test]
    fn every_group_note_belongs_to_a_group_with_examples_or_is_known() {
        for (group, en, pl) in GROUP_NOTES {
            assert!(!group.is_empty() && !en.is_empty() && !pl.is_empty());
        }
    }
}
