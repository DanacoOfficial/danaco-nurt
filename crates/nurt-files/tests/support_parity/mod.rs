// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Data model and validation of the parity scenario files.

// Each test target links a different subset of the model.
#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use serde::Deserialize;

/// Kind of a scenario, which determines how the expectation is evaluated.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Class {
    /// Result and file state follow from the previous implementation.
    Direct,
    /// A change that the previous implementation held back with an alarm.
    Notable,
    /// A two-step change expressed as a preview and a repeated call with `if_match`.
    Preview,
    /// Behaviour that depends on the requirement to read before writing.
    Freshness,
}

/// File of the tree before the call.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FsEntry {
    /// Name of a fixture that is instantiated whole.
    pub(crate) fixture: Option<String>,
    /// Path of a single file, relative to the root.
    pub(crate) path: Option<String>,
    /// Literal content.
    pub(crate) content: Option<String>,
    /// Generated content, see `fixtures.toml`.
    pub(crate) generate: Option<String>,
    /// Target of a symbolic link.
    pub(crate) symlink_to: Option<String>,
    /// Permission bits in octal.
    pub(crate) mode: Option<String>,
    /// Encoding of `content`; UTF-8 when absent.
    pub(crate) encoding: Option<String>,
    /// Modification times applied to files of a fixture.
    pub(crate) modified: Option<BTreeMap<String, String>>,
}

/// Assertion on a file of the tree after the call.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AfterEntry {
    /// Path relative to the root.
    pub(crate) path: String,
    /// The file equals this content.
    pub(crate) content: Option<String>,
    /// The file contains this text.
    pub(crate) contains: Option<String>,
    /// The file does not contain this text.
    pub(crate) excludes: Option<String>,
    /// The file equals the file of the same path in a fixture.
    pub(crate) same_as: Option<String>,
    /// The file equals the original with `from` replaced by `to`.
    pub(crate) replace_in: Option<String>,
    /// Text replaced in the original.
    pub(crate) from: Option<String>,
    /// Replacement text.
    pub(crate) to: Option<String>,
    /// The file equals generated content.
    pub(crate) generate: Option<String>,
    /// The path is a symbolic link to this target.
    pub(crate) symlink_to: Option<String>,
    /// Permission bits in octal.
    pub(crate) mode: Option<String>,
    /// Encoding of `content`; UTF-8 when absent.
    pub(crate) encoding: Option<String>,
}

/// Assertion on a field of the report.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FieldCheck {
    /// Path of the field in the report.
    pub(crate) field: String,
    /// The field equals this value.
    pub(crate) value: Option<toml::Value>,
    /// The field contains this value.
    pub(crate) contains: Option<toml::Value>,
    /// The field does not contain this value.
    pub(crate) excludes: Option<toml::Value>,
}

/// Expected outcome of a scenario.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Expect {
    /// The tree is identical to the tree before the call.
    #[serde(default)]
    pub(crate) fs_unchanged: bool,
    /// Assertions on the tree after the call.
    #[serde(default)]
    pub(crate) fs_after: Vec<AfterEntry>,
    /// Assertions on the report.
    #[serde(default)]
    pub(crate) fields: Vec<FieldCheck>,
}

/// One scenario.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Scenario {
    /// Name of the source case.
    pub(crate) name: String,
    /// Name of the source case when several scenarios stem from one case.
    pub(crate) case: Option<String>,
    /// Kind of the scenario.
    pub(crate) class: Class,
    /// Rows of the parity table the scenario covers.
    pub(crate) parity_rows: Vec<String>,
    /// Open item that the scenario depends on.
    pub(crate) open_gap: Option<String>,
    /// Tree before the call.
    pub(crate) fs_before: Vec<FsEntry>,
    /// Description of the call of the previous implementation.
    pub(crate) native_call: String,
    /// Program run by Nurt.
    pub(crate) nurt_program: String,
    /// Expected outcome.
    pub(crate) expect: Expect,
}

/// File of scenarios derived from one source test file.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ScenarioFile {
    /// Source test file.
    pub(crate) source: String,
    /// Number of cases in the source file.
    pub(crate) cases: u32,
    /// Scenarios.
    #[serde(default)]
    pub(crate) scenario: Vec<Scenario>,
}

/// Case that stays with the engine or with another kind of test.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HostOnly {
    /// Name of the source case.
    pub(crate) name: String,
    /// Source test file.
    pub(crate) source: String,
    /// Party that verifies the behaviour.
    pub(crate) owner: String,
    /// Number of source cases the entry stands for.
    #[serde(default = "one")]
    pub(crate) cases: u32,
    /// Justification.
    pub(crate) reason: String,
}

fn one() -> u32 {
    1
}

/// List of host-only cases.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HostOnlyFile {
    /// Entries.
    pub(crate) host_only: Vec<HostOnly>,
}

/// Source test file of the previous implementation.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Source {
    /// Path of the source test file.
    pub(crate) path: String,
    /// Number of cases.
    pub(crate) cases: u32,
    /// File of scenarios, when the source has any.
    pub(crate) scenarios: Option<String>,
}

/// List of source files.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SourcesFile {
    /// Entries.
    pub(crate) source: Vec<Source>,
}

/// File of a fixture.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FixtureFile {
    /// Path relative to the root.
    pub(crate) path: String,
    /// Literal content.
    pub(crate) content: Option<String>,
    /// Generated content.
    pub(crate) generate: Option<String>,
}

/// Named tree.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Fixture {
    /// Name.
    pub(crate) name: String,
    /// Files.
    pub(crate) file: Vec<FixtureFile>,
}

/// List of fixtures.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FixturesFile {
    /// Entries.
    pub(crate) fixture: Vec<Fixture>,
}

/// Row of the parity table.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Row {
    /// Identifier.
    pub(crate) id: String,
    /// State: `equal`, `partial`, `gap` or `dropped`.
    pub(crate) state: String,
    /// Open item, for gaps.
    pub(crate) gap: Option<String>,
}

/// List of rows.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RowsFile {
    /// Entries.
    pub(crate) row: Vec<Row>,
}

/// Row without a scenario yet.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Pending {
    /// Identifier of the row.
    pub(crate) id: String,
    /// Justification.
    pub(crate) reason: String,
}

/// List of rows without a scenario.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PendingFile {
    /// Entries.
    #[serde(default)]
    pub(crate) pending: Vec<Pending>,
    /// Scenarios that do not run, with the reason.
    #[serde(default)]
    pub(crate) scenario: Vec<PendingScenario>,
}

/// Scenario that does not run on the stages of the crate.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PendingScenario {
    /// Name of the scenario.
    pub(crate) name: String,
    /// Justification.
    pub(crate) reason: String,
}

/// Scenario that runs and differs from the expectation.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Divergence {
    /// Name of the scenario.
    pub(crate) name: String,
    /// Command whose behaviour differs.
    pub(crate) command: String,
    /// Observed difference.
    pub(crate) detail: String,
}

/// List of known differences.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DivergenceFile {
    /// Entries.
    #[serde(default)]
    pub(crate) divergence: Vec<Divergence>,
}

/// All parity data.
#[derive(Debug)]
pub(crate) struct Model {
    /// Rows of the parity table.
    pub(crate) rows: Vec<Row>,
    /// Fixtures.
    pub(crate) fixtures: Vec<Fixture>,
    /// Scenario files by name.
    pub(crate) files: BTreeMap<String, ScenarioFile>,
    /// Host-only cases.
    pub(crate) host_only: Vec<HostOnly>,
    /// Source test files.
    pub(crate) sources: Vec<Source>,
    /// Rows without a scenario.
    pub(crate) pending: Vec<Pending>,
    /// Scenarios that do not run.
    pub(crate) pending_scenarios: Vec<PendingScenario>,
    /// Scenarios that run and differ.
    pub(crate) divergences: Vec<Divergence>,
}

const NOTEBOOK_KINDS: [&str; 6] = [
    "cells4",
    "cells2",
    "flat",
    "standard_crlf_bom",
    "missing_ids",
    "empty",
];
const HOST_OWNERS: [&str; 3] = ["host", "crate-unit", "differential"];
const HOST_ONLY_FILE: &str = "host-only.toml";

fn parse<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
}

impl Model {
    /// Loads the data below `dir`.
    ///
    /// # Errors
    ///
    /// Returns a message naming the file that cannot be read or parsed.
    pub(crate) fn load(dir: &Path) -> Result<Self, String> {
        let rows: RowsFile = parse(&dir.join("rows.toml"))?;
        let fixtures: FixturesFile = parse(&dir.join("fixtures.toml"))?;
        let sources: SourcesFile = parse(&dir.join("sources.toml"))?;
        let host_only: HostOnlyFile = parse(&dir.join("files").join(HOST_ONLY_FILE))?;
        let pending: PendingFile = parse(&dir.join("pending.toml"))?;
        let divergences: DivergenceFile = parse(&dir.join("divergences.toml"))?;
        let mut files = BTreeMap::new();
        let listing = fs::read_dir(dir.join("files")).map_err(|e| e.to_string())?;
        for item in listing {
            let path = item.map_err(|e| e.to_string())?.path();
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default()
                .to_string();
            if name.ends_with(".toml") && name != HOST_ONLY_FILE {
                files.insert(name, parse::<ScenarioFile>(&path)?);
            }
        }
        Ok(Self {
            rows: rows.row,
            fixtures: fixtures.fixture,
            files,
            host_only: host_only.host_only,
            sources: sources.source,
            pending: pending.pending,
            pending_scenarios: pending.scenario,
            divergences: divergences.divergence,
        })
    }

    /// All scenarios.
    pub(crate) fn scenarios(&self) -> impl Iterator<Item = &Scenario> {
        self.files.values().flat_map(|f| f.scenario.iter())
    }

    /// Identifiers of rows covered by at least one scenario.
    pub(crate) fn covered_rows(&self) -> BTreeSet<&str> {
        self.scenarios()
            .flat_map(|s| s.parity_rows.iter().map(String::as_str))
            .collect()
    }

    /// Returns every violation of the rules of the data files.
    pub(crate) fn validate(&self) -> Vec<String> {
        let mut problems = Vec::new();
        self.check_rows(&mut problems);
        self.check_fixtures(&mut problems);
        self.check_scenarios(&mut problems);
        self.check_sources(&mut problems);
        self.check_coverage(&mut problems);
        self.check_listed_scenarios(&mut problems);
        problems
    }

    fn check_listed_scenarios(&self, problems: &mut Vec<String>) {
        let names: BTreeSet<&str> = self.scenarios().map(|s| s.name.as_str()).collect();
        let mut pending = BTreeSet::new();
        for entry in &self.pending_scenarios {
            if !names.contains(entry.name.as_str()) {
                problems.push(format!("pending scenario {} names no scenario", entry.name));
            }
            if !pending.insert(entry.name.as_str()) {
                problems.push(format!("pending scenario {} is listed twice", entry.name));
            }
            if entry.reason.trim().is_empty() {
                problems.push(format!("pending scenario {} needs a reason", entry.name));
            }
        }
        let mut diverging = BTreeSet::new();
        for entry in &self.divergences {
            if !names.contains(entry.name.as_str()) {
                problems.push(format!("divergence {} names no scenario", entry.name));
            }
            if !diverging.insert(entry.name.as_str()) {
                problems.push(format!("divergence {} is listed twice", entry.name));
            }
            if pending.contains(entry.name.as_str()) {
                problems.push(format!(
                    "scenario {} is both pending and diverging",
                    entry.name
                ));
            }
            if entry.command.trim().is_empty() || entry.detail.trim().is_empty() {
                problems.push(format!(
                    "divergence {} needs a command and a detail",
                    entry.name
                ));
            }
        }
    }

    fn row(&self, id: &str) -> Option<&Row> {
        self.rows.iter().find(|r| r.id == id)
    }

    fn fixture_or_generator_exists(&self, name: &str) -> bool {
        self.fixtures.iter().any(|f| f.name == name) || generator_error(name).is_none()
    }

    fn check_rows(&self, problems: &mut Vec<String>) {
        let mut seen = BTreeSet::new();
        for row in &self.rows {
            if !seen.insert(row.id.as_str()) {
                problems.push(format!("row {} is listed twice", row.id));
            }
            match row.state.as_str() {
                "equal" | "partial" | "dropped" => {
                    if row.gap.is_some() {
                        problems.push(format!("row {} has a gap but is not a gap", row.id));
                    }
                }
                "gap" => {
                    if !row.gap.as_deref().is_some_and(is_gap_id) {
                        problems.push(format!("gap row {} lacks an open item", row.id));
                    }
                }
                other => problems.push(format!("row {} has unknown state {other}", row.id)),
            }
        }
    }

    fn check_fixtures(&self, problems: &mut Vec<String>) {
        let mut names = BTreeSet::new();
        for fixture in &self.fixtures {
            if !names.insert(fixture.name.as_str()) {
                problems.push(format!("fixture {} is defined twice", fixture.name));
            }
            if fixture.file.is_empty() {
                problems.push(format!("fixture {} has no files", fixture.name));
            }
            for file in &fixture.file {
                if let Some(error) = path_error(&file.path) {
                    problems.push(format!("fixture {}: {error}", fixture.name));
                }
                if file.content.is_some() == file.generate.is_some() {
                    problems.push(format!(
                        "fixture {}: {} needs exactly one of content and generate",
                        fixture.name, file.path
                    ));
                }
                if let Some(error) = file.generate.as_deref().and_then(generator_error) {
                    problems.push(format!("fixture {}: {error}", fixture.name));
                }
            }
        }
    }

    fn check_scenarios(&self, problems: &mut Vec<String>) {
        let mut names = BTreeSet::new();
        for (file_name, file) in &self.files {
            for scenario in &file.scenario {
                let at = format!("{file_name}: {}", scenario.name);
                if !names.insert(scenario.name.as_str()) {
                    problems.push(format!("{at}: name is not unique"));
                }
                self.check_scenario(&at, scenario, problems);
            }
        }
    }

    fn check_scenario(&self, at: &str, s: &Scenario, problems: &mut Vec<String>) {
        if s.name.trim().is_empty()
            || s.native_call.trim().is_empty()
            || s.nurt_program.trim().is_empty()
        {
            problems.push(format!(
                "{at}: name, native_call and nurt_program must not be empty"
            ));
        }
        if has_absolute_path(&s.nurt_program) {
            problems.push(format!("{at}: program contains an absolute path"));
        }
        if s.parity_rows.is_empty() {
            problems.push(format!("{at}: no parity rows"));
        }
        for id in &s.parity_rows {
            match self.row(id).map(|r| r.state.as_str()) {
                None => problems.push(format!("{at}: unknown row {id}")),
                Some("equal" | "partial") => {}
                Some(state) => problems.push(format!("{at}: row {id} is {state}")),
            }
        }
        if let Some(gap) = &s.open_gap
            && !self.rows.iter().any(|r| r.gap.as_deref() == Some(gap))
        {
            problems.push(format!("{at}: open item {gap} is not tracked"));
        }
        for entry in &s.fs_before {
            self.check_before(at, entry, problems);
        }
        let expect = &s.expect;
        if !expect.fs_unchanged && expect.fs_after.is_empty() && expect.fields.is_empty() {
            problems.push(format!("{at}: nothing is expected"));
        }
        if expect.fs_unchanged && !expect.fs_after.is_empty() {
            problems.push(format!("{at}: fs_unchanged excludes fs_after"));
        }
        for entry in &expect.fs_after {
            self.check_after(at, entry, problems);
        }
        for check in &expect.fields {
            let kinds = [&check.value, &check.contains, &check.excludes]
                .iter()
                .filter(|v| v.is_some())
                .count();
            if check.field.is_empty() || kinds != 1 {
                problems.push(format!(
                    "{at}: field check {} needs one comparison",
                    check.field
                ));
            }
        }
    }

    fn check_before(&self, at: &str, entry: &FsEntry, problems: &mut Vec<String>) {
        match (&entry.fixture, &entry.path) {
            (Some(name), None) => {
                if !self.fixtures.iter().any(|f| &f.name == name) {
                    problems.push(format!("{at}: unknown fixture {name}"));
                }
            }
            (None, Some(path)) => {
                if let Some(error) = path_error(path) {
                    problems.push(format!("{at}: {error}"));
                }
                let kinds = [
                    entry.content.is_some(),
                    entry.generate.is_some(),
                    entry.symlink_to.is_some(),
                ]
                .iter()
                .filter(|k| **k)
                .count();
                if kinds != 1 {
                    problems.push(format!(
                        "{at}: {path} needs one of content, generate and symlink_to"
                    ));
                }
                if let Some(error) = entry.generate.as_deref().and_then(generator_error) {
                    problems.push(format!("{at}: {error}"));
                }
            }
            _ => problems.push(format!(
                "{at}: fs_before entry needs exactly one of fixture and path"
            )),
        }
        if let Some(mode) = &entry.mode
            && !is_octal_mode(mode)
        {
            problems.push(format!("{at}: invalid mode {mode}"));
        }
    }

    fn check_after(&self, at: &str, entry: &AfterEntry, problems: &mut Vec<String>) {
        if let Some(error) = path_error(&entry.path) {
            problems.push(format!("{at}: {error}"));
        }
        let kinds = [
            entry.content.is_some(),
            entry.contains.is_some(),
            entry.excludes.is_some(),
            entry.same_as.is_some(),
            entry.replace_in.is_some(),
            entry.generate.is_some(),
            entry.symlink_to.is_some(),
        ]
        .iter()
        .filter(|k| **k)
        .count();
        if kinds != 1 {
            problems.push(format!("{at}: {} needs exactly one assertion", entry.path));
        }
        if entry.replace_in.is_some() != (entry.from.is_some() && entry.to.is_some()) {
            problems.push(format!(
                "{at}: {} needs replace_in together with from and to",
                entry.path
            ));
        }
        for reference in [&entry.same_as, &entry.replace_in].into_iter().flatten() {
            if !self.fixture_or_generator_exists(reference) {
                problems.push(format!("{at}: unknown fixture {reference}"));
            }
        }
        if let Some(error) = entry.generate.as_deref().and_then(generator_error) {
            problems.push(format!("{at}: {error}"));
        }
        if let Some(mode) = &entry.mode
            && !is_octal_mode(mode)
        {
            problems.push(format!("{at}: invalid mode {mode}"));
        }
    }

    fn check_sources(&self, problems: &mut Vec<String>) {
        let mut host: BTreeMap<&str, (u32, BTreeSet<&str>)> = BTreeMap::new();
        for entry in &self.host_only {
            if !HOST_OWNERS.contains(&entry.owner.as_str()) {
                problems.push(format!(
                    "host-only {}: unknown owner {}",
                    entry.name, entry.owner
                ));
            }
            if entry.reason.trim().is_empty() || entry.cases == 0 {
                problems.push(format!(
                    "host-only {}: reason and cases are required",
                    entry.name
                ));
            }
            let slot = host.entry(entry.source.as_str()).or_default();
            slot.0 += entry.cases;
            if !slot.1.insert(entry.name.as_str()) {
                problems.push(format!(
                    "host-only {}: listed twice for {}",
                    entry.name, entry.source
                ));
            }
        }
        let mut seen = BTreeSet::new();
        for source in &self.sources {
            if !seen.insert(source.path.as_str()) {
                problems.push(format!("source {} is listed twice", source.path));
            }
            let (host_cases, host_names) =
                host.get(source.path.as_str()).cloned().unwrap_or_default();
            let mut scenario_cases = 0;
            if let Some(name) = &source.scenarios {
                let Some(file) = self.files.get(name) else {
                    problems.push(format!("source {}: missing file {name}", source.path));
                    continue;
                };
                if file.source != source.path || file.cases != source.cases {
                    problems.push(format!(
                        "{name}: source or case count differs from sources.toml"
                    ));
                }
                let cases: BTreeSet<&str> = file
                    .scenario
                    .iter()
                    .map(|s| s.case.as_deref().unwrap_or(&s.name))
                    .collect();
                scenario_cases = cases.len() as u32;
                for name in &host_names {
                    if cases.contains(name) {
                        problems.push(format!(
                            "{}: {name} is both a scenario and host-only",
                            source.path
                        ));
                    }
                }
            }
            if scenario_cases + host_cases != source.cases {
                problems.push(format!(
                    "source {}: {scenario_cases} scenario cases and {host_cases} host-only cases, expected {}",
                    source.path, source.cases
                ));
            }
        }
        for source in host.keys() {
            if !self.sources.iter().any(|s| s.path == *source) {
                problems.push(format!("host-only entries name unknown source {source}"));
            }
        }
    }

    fn check_coverage(&self, problems: &mut Vec<String>) {
        let covered = self.covered_rows();
        let mut pending = BTreeSet::new();
        for entry in &self.pending {
            if entry.reason.trim().is_empty() {
                problems.push(format!("pending row {} needs a reason", entry.id));
            }
            if !pending.insert(entry.id.as_str()) {
                problems.push(format!("pending row {} is listed twice", entry.id));
            }
            match self.row(&entry.id).map(|r| r.state.as_str()) {
                Some("equal" | "partial") => {}
                _ => problems.push(format!(
                    "pending row {} is not an equal or partial row",
                    entry.id
                )),
            }
            if covered.contains(entry.id.as_str()) {
                problems.push(format!("pending row {} has a scenario", entry.id));
            }
        }
        for row in &self.rows {
            let needs_scenario = matches!(row.state.as_str(), "equal" | "partial");
            if needs_scenario
                && !covered.contains(row.id.as_str())
                && !pending.contains(row.id.as_str())
            {
                problems.push(format!(
                    "row {} has neither a scenario nor a pending entry",
                    row.id
                ));
            }
        }
    }
}

fn has_absolute_path(program: &str) -> bool {
    ["\"/", "'/"].iter().any(|quote| {
        program
            .match_indices(quote)
            .any(|(at, _)| !program[at + 2..].starts_with('/'))
    })
}

fn is_gap_id(text: &str) -> bool {
    text.strip_prefix("LP-")
        .is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
}

fn is_octal_mode(text: &str) -> bool {
    (3..=4).contains(&text.len()) && text.chars().all(|c| ('0'..='7').contains(&c))
}

fn path_error(path: &str) -> Option<String> {
    let bad = path.is_empty()
        || path.starts_with('/')
        || path.contains('\\')
        || path.split('/').any(|part| part.is_empty() || part == "..");
    bad.then(|| format!("path {path:?} must be relative and normalized"))
}

/// Returns the reason a generator directive is invalid, or `None`.
pub(crate) fn generator_error(directive: &str) -> Option<String> {
    let invalid = || Some(format!("invalid generator {directive:?}"));
    let Some((kind, argument)) = directive.split_once(':') else {
        return invalid();
    };
    let valid = match kind {
        "big_source" => argument.parse::<u32>().is_ok(),
        "png" => parse_size(argument).is_some(),
        "pdf" => !argument.is_empty(),
        "repeat" => argument
            .rsplit_once('*')
            .is_some_and(|(text, count)| !text.is_empty() && count.parse::<u32>().is_ok()),
        "webp" => argument.split_once(',').is_some_and(|(variant, size)| {
            ["VP8", "VP8L", "VP8X"].contains(&variant) && parse_size(size).is_some()
        }),
        "bytes" => argument.len() % 2 == 0 && argument.chars().all(|c| c.is_ascii_hexdigit()),
        "notebook" => NOTEBOOK_KINDS.contains(&argument),
        _ => false,
    };
    if valid { None } else { invalid() }
}

fn parse_size(text: &str) -> Option<(u32, u32)> {
    let (width, height) = text.split_once('x')?;
    Some((width.parse().ok()?, height.parse().ok()?))
}
