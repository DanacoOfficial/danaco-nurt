// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Data set of pairs "task → Danaco Nurt program", `nurt-bench-pair/1`.
//!
//! Candidate pairs come from the public tasks (description and reference
//! solutions in bash and PowerShell), the examples of `docs/commands`, the
//! tables of shell habits and canonical forms, and the passed runs of the
//! Danaco Nurt conditions. A candidate is accepted when `check` binds its
//! program and, when the pair has an initial file tree, the program runs on
//! that tree without an error. Held-out tasks never enter the data set.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::sync::Arc;

use nurt_contracts::envelope::{Request, RequestOptions};
use nurt_contracts::error::{Error, ErrorCode};
use nurt_core::config::ConfigSources;
use nurt_core::{DeliveredEvent, Engine, EngineConfig, EventListener, Frontend};
use nurt_testkit::clock::VirtualClock;
use nurt_testkit::host::TestHost;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{BenchError, Result};
use crate::rpc::{CALL_TIMEOUT, RpcClient};
use crate::runlog::{self, Event};
use crate::runner::sha256_hex;
use crate::snapshot::{self, Tree};
use crate::task::TaskFile;

/// Format identifier of a pair.
pub const PAIR_FORMAT: &str = "nurt-bench-pair/1";

/// Format identifier of a habit table.
pub const HABITS_FORMAT: &str = "nurt-bench-habits/1";

/// License of the data set.
pub const LICENSE: &str = "CC-BY-4.0";

/// Time limit of the execution of one program.
const RUN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

/// Hexadecimal digits of the pair identifier.
const ID_DIGITS: usize = 16;

/// Source of a pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    /// Description and reference program of a task.
    Task,
    /// Reference shell solution and reference program of a task.
    Translation,
    /// Example of the command documentation.
    Docs,
    /// Row of a habit table.
    Habit,
    /// Programs of a passed run.
    Run,
}

/// Kind of the input of a pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InputKind {
    /// Description in English.
    Nl,
    /// bash command.
    Bash,
    /// PowerShell command.
    Pwsh,
}

/// Validation of a pair.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Validation {
    /// Whether `check` bound the program.
    pub checked: bool,
    /// Whether the program ran on the initial tree of the pair.
    pub executed: bool,
    /// Status of the report of the execution.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// Why the pair was not executed although it was not rejected.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Why the pair was rejected.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rejected: Option<String>,
}

/// One pair of the data set.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Pair {
    /// Format identifier, [`PAIR_FORMAT`].
    pub format: String,
    /// Identifier: the first hexadecimal digits of the SHA-256 of the
    /// input kind, the input and the program.
    pub id: String,
    /// Source.
    pub source: Source,
    /// Task, documentation page, habit table or run the pair comes from.
    pub origin: String,
    /// Kind of the input.
    pub input_kind: InputKind,
    /// Input.
    pub input: String,
    /// Program in variant A; several programs of a run are separated by new
    /// lines and form a script.
    pub program: String,
    /// Whether the program is a script of several pipelines.
    pub script: bool,
    /// Validation result.
    pub validation: Validation,
    /// License of the pair.
    pub license: String,
}

/// Candidate pair before validation.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    /// The pair without validation.
    pub pair: Pair,
    /// Initial tree on which the program is executed.
    pub tree: Option<Tree>,
}

fn candidate(
    source: Source,
    origin: &str,
    input_kind: InputKind,
    input: &str,
    program: &str,
    tree: Option<Tree>,
) -> Candidate {
    let script = program.trim().contains('\n');
    let id_source = format!("{input_kind:?}\n{input}\n{program}");
    Candidate {
        pair: Pair {
            format: PAIR_FORMAT.to_owned(),
            id: sha256_hex(&id_source)[..ID_DIGITS].to_owned(),
            source,
            origin: origin.to_owned(),
            input_kind,
            input: input.trim().to_owned(),
            program: program.trim().to_owned(),
            script,
            validation: Validation::default(),
            license: LICENSE.to_owned(),
        },
        tree,
    }
}

/// Candidates from the public tasks.
///
/// # Errors
///
/// Returns an error when the initial tree of a task cannot be resolved.
pub fn from_tasks(tasks: &[TaskFile], fixtures: &Path) -> Result<Vec<Candidate>> {
    let mut candidates = Vec::new();
    for file in tasks.iter().filter(|file| !file.is_holdout()) {
        let task = &file.task;
        let tree = snapshot::resolve_fs_before(&task.fs_before, fixtures)?;
        let program = &task.oracle_program;
        candidates.push(candidate(
            Source::Task,
            &task.id,
            InputKind::Nl,
            &task.nl,
            program,
            Some(tree.clone()),
        ));
        for (kind, reference) in [
            (InputKind::Bash, &task.reference_bash),
            (InputKind::Pwsh, &task.reference_pwsh),
        ] {
            if let Some(reference) = reference {
                candidates.push(candidate(
                    Source::Translation,
                    &task.id,
                    kind,
                    reference,
                    program,
                    Some(tree.clone()),
                ));
            }
        }
    }
    Ok(candidates)
}

fn fenced_block(lines: &[&str], start: usize) -> Option<(String, usize)> {
    let end = lines[start + 1..]
        .iter()
        .position(|line| line.trim_start().starts_with("```"))?
        + start
        + 1;
    Some((lines[start + 1..end].join("\n"), end))
}

fn paragraph_before(lines: &[&str], index: usize) -> String {
    let mut collected = Vec::new();
    for line in lines[..index].iter().rev() {
        let line = line.trim();
        if line.is_empty() {
            if collected.is_empty() {
                continue;
            }
            break;
        }
        if line.starts_with('#') || line.starts_with("```") {
            break;
        }
        collected.push(line);
    }
    collected.reverse();
    collected.join(" ").trim_end_matches(':').trim().to_owned()
}

/// Candidates from the examples of one documentation page: the paragraph
/// before a block `nurt example` is the input, and a following block
/// `json fs-before` is the initial tree.
///
/// # Errors
///
/// Returns an error when an initial tree is invalid.
pub fn from_page(origin: &str, text: &str) -> Result<Vec<Candidate>> {
    let lines: Vec<&str> = text.lines().collect();
    let mut candidates = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        if lines[index].trim() != "```nurt example" {
            index += 1;
            continue;
        }
        let Some((program, end)) = fenced_block(&lines, index) else {
            break;
        };
        let description = paragraph_before(&lines, index);
        let mut next = end + 1;
        while next < lines.len() && lines[next].trim().is_empty() {
            next += 1;
        }
        let mut tree = None;
        if next < lines.len()
            && lines[next].trim() == "```json fs-before"
            && let Some((json, block_end)) = fenced_block(&lines, next)
        {
            let value: Value = serde_json::from_str(&json).map_err(|error| {
                BenchError::format(Path::new(origin), "fs-before", error.to_string())
            })?;
            tree = Some(snapshot::resolve_fs_before(&value, Path::new(origin))?);
            next = block_end;
        }
        if !description.is_empty() {
            candidates.push(candidate(
                Source::Docs,
                origin,
                InputKind::Nl,
                &description,
                &program,
                tree,
            ));
        }
        index = next.max(end) + 1;
    }
    Ok(candidates)
}

/// Candidates from every page of `docs/commands`, except files starting
/// with `_` and `README.md`.
///
/// # Errors
///
/// Returns an error when the directory or a page cannot be read.
pub fn from_docs(directory: &Path) -> Result<Vec<Candidate>> {
    let mut pages: Vec<_> = fs::read_dir(directory)
        .map_err(|error| BenchError::io(directory, error))?
        .filter_map(std::result::Result::ok)
        .map(|item| item.path())
        .filter(|path| {
            let name = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            name.ends_with(".md") && !name.starts_with('_') && name != "README.md"
        })
        .collect();
    pages.sort();
    let mut candidates = Vec::new();
    for page in pages {
        let text = fs::read_to_string(&page).map_err(|error| BenchError::io(&page, error))?;
        let origin = format!(
            "docs/commands/{}",
            page.file_name()
                .map(|name| name.to_string_lossy())
                .unwrap_or_default()
        );
        candidates.extend(from_page(&origin, &text)?);
    }
    Ok(candidates)
}

/// Row of a habit table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Habit {
    /// `bash` or `pwsh`.
    pub shell: String,
    /// Command as written out of habit.
    pub habit: String,
    /// Canonical Danaco Nurt program.
    pub canonical: String,
    /// Identifier of the translation rule, for example `B03`.
    #[serde(default)]
    pub rule: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct HabitTable {
    format: String,
    #[serde(default)]
    pair: Vec<Habit>,
}

/// Candidates from a habit table `nurt-bench-habits/1` (TOML, rows
/// `[[pair]]`).
///
/// # Errors
///
/// Returns an error when the file cannot be read or is invalid.
pub fn from_habits(path: &Path) -> Result<Vec<Candidate>> {
    let text = fs::read_to_string(path).map_err(|error| BenchError::io(path, error))?;
    let table: HabitTable = toml::from_str(&text)
        .map_err(|error| BenchError::format(path, "habit table", error.to_string()))?;
    if table.format != HABITS_FORMAT {
        return Err(BenchError::format(
            path,
            "habit table",
            format!("format must be {HABITS_FORMAT}"),
        ));
    }
    let origin = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    table
        .pair
        .iter()
        .map(|habit| {
            let kind = match habit.shell.as_str() {
                "bash" => InputKind::Bash,
                "pwsh" => InputKind::Pwsh,
                other => {
                    return Err(BenchError::format(
                        path,
                        "habit table",
                        format!("unknown shell {other:?}"),
                    ));
                }
            };
            let origin = habit
                .rule
                .as_ref()
                .map_or_else(|| origin.clone(), |rule| format!("{origin}#{rule}"));
            Ok(candidate(
                Source::Habit,
                &origin,
                kind,
                &habit.habit,
                &habit.canonical,
                None,
            ))
        })
        .collect()
}

/// Candidates from the passed runs of the Danaco Nurt conditions on public
/// tasks: the description and the programs of the calls without an error.
///
/// # Errors
///
/// Returns an error when a log cannot be read or a task tree is invalid.
pub fn from_runs(root: &Path, tasks: &[TaskFile], fixtures: &Path) -> Result<Vec<Candidate>> {
    let mut candidates = Vec::new();
    for path in runlog::find_logs(root)? {
        let events = runlog::read(&path)?;
        let Some(Event::Run(header)) = events.first() else {
            continue;
        };
        let passed = events
            .iter()
            .any(|event| matches!(event, Event::Oracle(o) if o.verdict.passed));
        let Some(task) = tasks
            .iter()
            .find(|task| task.task.id == header.task.id && !task.is_holdout())
        else {
            continue;
        };
        if !header.condition.is_nurt() || !passed {
            continue;
        }
        let programs: Vec<&str> = events
            .iter()
            .filter_map(|event| match event {
                Event::Tool(tool) if !tool.is_error => {
                    tool.input.get("program").and_then(Value::as_str)
                }
                _ => None,
            })
            .collect();
        if programs.is_empty() {
            continue;
        }
        let tree = snapshot::resolve_fs_before(&task.task.fs_before, fixtures)?;
        candidates.push(candidate(
            Source::Run,
            &header.run_id,
            InputKind::Nl,
            &task.task.nl,
            &programs.join("\n"),
            Some(tree),
        ));
    }
    Ok(candidates)
}

/// Removes candidates with the same input kind, input and program, keeping
/// the first.
#[must_use]
pub fn deduplicate(candidates: Vec<Candidate>) -> Vec<Candidate> {
    // The identifier hashes the text before it is trimmed and is kept as it
    // was published, so pairs are also compared by their trimmed text.
    let mut ids = BTreeSet::new();
    let mut texts = BTreeSet::new();
    candidates
        .into_iter()
        .filter(|candidate| {
            let pair = &candidate.pair;
            let text = (
                format!("{:?}", pair.input_kind),
                pair.input.clone(),
                pair.program.clone(),
            );
            let new_id = ids.insert(pair.id.clone());
            let new_text = texts.insert(text);
            new_id && new_text
        })
        .collect()
}

/// Result of one call of a [`Validator`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Outcome {
    /// Code of the error of the response, when it has one.
    pub error: Option<String>,
    /// Status of the effect report.
    pub status: Option<String>,
    /// Whether the call could not do its work because a component of the
    /// engine is not connected yet; the pair is then neither accepted as
    /// executed nor rejected.
    pub unavailable: bool,
}

/// Source of the verdicts that validate a pair: binding and execution of a
/// program.
pub trait Validator {
    /// Binds the program of `request` without running it.
    ///
    /// # Errors
    ///
    /// Returns an error when the validator itself fails.
    fn check(&mut self, request: &Request) -> Result<Outcome>;

    /// Runs the program of `request`.
    ///
    /// # Errors
    ///
    /// Returns an error when the validator itself fails.
    fn run(&mut self, request: &Request) -> Result<Outcome>;
}

fn error_text(value: &Value) -> Option<String> {
    let error = value
        .get("error")
        .or_else(|| value["report"].get("error"))?;
    Some(error["code"].as_str().unwrap_or("error").to_owned())
}

impl Validator for RpcClient {
    fn check(&mut self, request: &Request) -> Result<Outcome> {
        let response = self.request("check", request, CALL_TIMEOUT)?;
        Ok(Outcome {
            error: error_text(&response),
            ..Outcome::default()
        })
    }

    fn run(&mut self, request: &Request) -> Result<Outcome> {
        let response = self.request("run", request, RUN_TIMEOUT)?;
        Ok(Outcome {
            error: error_text(&response),
            status: response["report"]["status"].as_str().map(str::to_owned),
            unavailable: false,
        })
    }
}

struct NullListener;

impl EventListener for NullListener {
    fn deliver(&self, _event: DeliveredEvent) {}
}

/// Engine with the configuration sources isolated from the host, no state
/// directory and the given language front end.
///
/// # Errors
///
/// Returns an error when the engine cannot be opened.
pub fn open_engine(frontend: Option<Arc<dyn Frontend>>) -> Result<Engine> {
    let mut config = EngineConfig::new(Arc::new(VirtualClock::new()), Arc::new(TestHost::linux()))
        .with_sources(ConfigSources::isolated())
        .without_state_dir();
    if let Some(frontend) = frontend {
        config = config.with_frontend(frontend);
    }
    Engine::open(config).map_err(|error| BenchError::Argument(error.to_string()))
}

/// Validator that calls an [`Engine`] in the process: `check` through
/// [`Engine::check`] and execution through [`Engine::start`] and its
/// [`ExecHandle`].
///
/// While the planner of the engine is not connected, `check` reports
/// `E_UNSUPPORTED` after the program was bound successfully, which counts as
/// a program that binds, and a run fails with `E_UNSUPPORTED`, which is
/// reported as [`Outcome::unavailable`]. An engine without a language front
/// end makes `check` unavailable as well.
#[derive(Debug)]
pub struct EngineValidator {
    engine: Engine,
}

impl EngineValidator {
    /// Validator of the given engine.
    #[must_use]
    pub fn new(engine: Engine) -> Self {
        Self { engine }
    }
}

impl Validator for EngineValidator {
    fn check(&mut self, request: &Request) -> Result<Outcome> {
        Ok(match self.engine.check(request) {
            Ok(_) => Outcome::default(),
            Err(error) if bound_without_planner(&error) => Outcome::default(),
            Err(error) if error.code == ErrorCode::Unsupported => Outcome {
                unavailable: true,
                ..Outcome::default()
            },
            Err(error) => Outcome {
                error: Some(error.code.as_str().to_owned()),
                ..Outcome::default()
            },
        })
    }

    fn run(&mut self, request: &Request) -> Result<Outcome> {
        let handle = self.engine.start(request, Arc::new(NullListener));
        let Some(response) = handle.wait(Some(RUN_TIMEOUT)) else {
            handle.cancel();
            return Ok(Outcome {
                error: Some(ErrorCode::Timeout.as_str().to_owned()),
                ..Outcome::default()
            });
        };
        let report = &response.report;
        let status = serde_json::to_value(report.status)
            .ok()
            .and_then(|value| value.as_str().map(str::to_owned));
        let unavailable = report
            .error
            .as_ref()
            .is_some_and(|error| error.code == ErrorCode::Unsupported);
        Ok(Outcome {
            error: report
                .error
                .as_ref()
                .filter(|_| !unavailable)
                .map(|error| error.code.as_str().to_owned()),
            status,
            unavailable,
        })
    }
}

/// Whether `error` is the answer of an engine without a planner to a program
/// that was bound: the engine adds the `command` detail after binding.
fn bound_without_planner(error: &Error) -> bool {
    error.code == ErrorCode::Unsupported && error.details.contains_key("command")
}

fn request(pair: &Pair, base: &Path) -> Request {
    let options = RequestOptions {
        base: Some(base.display().to_string().replace('\\', "/")),
        script: pair.script.then_some(true),
        ..RequestOptions::default()
    };
    Request::program(pair.program.clone()).with_options(options)
}

/// Validates a candidate: `check`, then `run` on its initial tree when it
/// has one.
///
/// # Errors
///
/// Returns an error when the validator fails or a temporary directory cannot
/// be created.
pub fn validate(validator: &mut dyn Validator, candidate: &Candidate) -> Result<Pair> {
    let mut pair = candidate.pair.clone();
    let scratch =
        tempfile::tempdir().map_err(|error| BenchError::io(&std::env::temp_dir(), error))?;
    let checked = validator.check(&request(&pair, scratch.path()))?;
    if let Some(code) = checked.error {
        pair.validation.rejected = Some(format!("check: {code}"));
        return Ok(pair);
    }
    if checked.unavailable {
        pair.validation.note = Some("check unavailable".to_owned());
        return Ok(pair);
    }
    pair.validation.checked = true;
    let Some(tree) = &candidate.tree else {
        return Ok(pair);
    };
    snapshot::materialize(tree, scratch.path())?;
    let outcome = validator.run(&request(&pair, scratch.path()))?;
    pair.validation.status = outcome.status;
    match outcome.error {
        Some(code) => pair.validation.rejected = Some(format!("run: {code}")),
        None if outcome.unavailable => {
            pair.validation.note = Some("execution unavailable".to_owned());
        }
        None if matches!(
            pair.validation.status.as_deref(),
            Some("failed" | "rolled_back" | "partial")
        ) =>
        {
            pair.validation.rejected = Some("run: failed".to_owned());
        }
        None => pair.validation.executed = true,
    }
    Ok(pair)
}

/// Counts of a generation.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceCount {
    /// Source.
    pub source: Option<Source>,
    /// Candidates after removal of duplicates.
    pub candidates: usize,
    /// Accepted pairs.
    pub accepted: usize,
    /// Rejected pairs.
    pub rejected: usize,
    /// Pairs that `check` could not judge.
    #[serde(default)]
    pub unvalidated: usize,
}

/// Summary of a generation, written as `summary.json`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DatasetSummary {
    /// Whether the candidates were validated.
    pub validated: bool,
    /// Counts per source.
    pub sources: Vec<SourceCount>,
    /// Accepted pairs in total.
    pub accepted: usize,
    /// Pairs that `check` could not judge, in total.
    #[serde(default)]
    pub unvalidated: usize,
}

fn write_lines(path: &Path, pairs: &[Pair]) -> Result<()> {
    let mut text = String::new();
    for pair in pairs {
        let line = serde_json::to_string(pair)
            .map_err(|error| BenchError::format(path, "pair", error.to_string()))?;
        text.push_str(&line);
        text.push('\n');
    }
    fs::write(path, text).map_err(|error| BenchError::io(path, error))
}

/// Validates the candidates (or only lists them when `validator` is `None`)
/// and writes `pairs.jsonl`, `rejected.jsonl`, `unvalidated.jsonl` and
/// `summary.json` to `out`;
/// without validation `candidates.jsonl` replaces the first two files.
///
/// # Errors
///
/// Returns an error when the validator fails or a file cannot be written.
pub fn build(
    candidates: Vec<Candidate>,
    validator: Option<&mut dyn Validator>,
    out: &Path,
) -> Result<DatasetSummary> {
    let candidates = deduplicate(candidates);
    fs::create_dir_all(out).map_err(|error| BenchError::io(out, error))?;
    // The files of the other mode are removed, so that a rebuild never
    // leaves output of an earlier run next to the new summary.
    let stale: &[&str] = if validator.is_some() {
        &["candidates.jsonl"]
    } else {
        &["pairs.jsonl", "rejected.jsonl", "unvalidated.jsonl"]
    };
    for name in stale {
        match fs::remove_file(out.join(name)) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(BenchError::io(&out.join(name), error)),
        }
    }
    let mut summary = DatasetSummary {
        validated: validator.is_some(),
        ..DatasetSummary::default()
    };
    let mut accepted = Vec::new();
    let mut rejected = Vec::new();
    let mut unvalidated = Vec::new();
    match validator {
        Some(validator) => {
            for candidate in &candidates {
                let pair = validate(validator, candidate)?;
                if pair.validation.rejected.is_some() {
                    rejected.push(pair);
                } else if pair.validation.checked {
                    accepted.push(pair);
                } else {
                    unvalidated.push(pair);
                }
            }
            write_lines(&out.join("pairs.jsonl"), &accepted)?;
            write_lines(&out.join("rejected.jsonl"), &rejected)?;
            write_lines(&out.join("unvalidated.jsonl"), &unvalidated)?;
        }
        None => {
            let pairs: Vec<Pair> = candidates
                .iter()
                .map(|candidate| candidate.pair.clone())
                .collect();
            write_lines(&out.join("candidates.jsonl"), &pairs)?;
        }
    }
    let sources: BTreeSet<Source> = candidates
        .iter()
        .map(|candidate| candidate.pair.source)
        .collect();
    for source in sources {
        let count = |pairs: &[Pair]| pairs.iter().filter(|pair| pair.source == source).count();
        summary.sources.push(SourceCount {
            source: Some(source),
            candidates: candidates
                .iter()
                .filter(|candidate| candidate.pair.source == source)
                .count(),
            accepted: count(&accepted),
            rejected: count(&rejected),
            unvalidated: count(&unvalidated),
        });
    }
    summary.accepted = accepted.len();
    summary.unvalidated = unvalidated.len();
    let path = out.join("summary.json");
    let text = serde_json::to_string_pretty(&summary)
        .map_err(|error| BenchError::format(&path, "summary", error.to_string()))?;
    fs::write(&path, text).map_err(|error| BenchError::io(&path, error))?;
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;

    use serde_json::json;

    use super::*;
    use crate::rpc::testing::connect;
    use crate::task::tests::task_document;

    const PAGE: &str = "---\ncommand: replace\n---\n# replace\n\n## Examples\n\nReplace a call in one file:\n\n```nurt example\nreplace \"old(\" \"new(\" in: [\"a.go\"]\n```\n\n```json fs-before\n{\"format\": \"nurt-fs-snapshot/1\", \"entries\": [{\"path\": \"a.go\", \"type\": \"file\", \"content_utf8\": \"old(1)\\n\"}]}\n```\n\n```text expected\n1 file changed\n```\n\nPreview the change\nin every file:\n\n```nurt example\nreplace \"a\" \"b\" in: [\"**\"] preview: true\n```\n\n```nurt example\nfind \".\"\n```\n";

    fn task(id: &str, split: &str, program: &str) -> TaskFile {
        let mut document = task_document(id);
        document["split"] = json!(split);
        document["oracle_program"] = json!(program);
        document["reference_pwsh"] = json!("Set-Content out.txt hello");
        TaskFile::from_document(&PathBuf::from("t.json"), document).unwrap()
    }

    #[test]
    fn reads_examples_of_a_documentation_page() {
        let candidates = from_page("docs/commands/replace.md", PAGE).unwrap();
        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0].pair.input, "Replace a call in one file");
        assert!(candidates[0].tree.as_ref().unwrap().contains_key("a.go"));
        assert_eq!(candidates[1].pair.input, "Preview the change in every file");
        assert!(candidates[1].tree.is_none());
        assert_eq!(candidates[1].pair.source, Source::Docs);
    }

    #[test]
    fn takes_public_tasks_and_their_translations() {
        let dir = tempfile::tempdir().unwrap();
        let tasks = [
            task("edit-001", "public", "write"),
            task("edit-002", "holdout", "write"),
        ];
        let candidates = from_tasks(&tasks, dir.path()).unwrap();
        let kinds: Vec<(Source, InputKind)> = candidates
            .iter()
            .map(|c| (c.pair.source, c.pair.input_kind))
            .collect();
        assert_eq!(
            kinds,
            [
                (Source::Task, InputKind::Nl),
                (Source::Translation, InputKind::Bash),
                (Source::Translation, InputKind::Pwsh)
            ]
        );
        assert!(candidates.iter().all(|c| c.pair.origin == "edit-001"));
    }

    #[test]
    fn pairs_that_differ_only_in_white_space_are_one_pair_and_stale_files_go() {
        let first = candidate(
            Source::Docs,
            "a",
            InputKind::Nl,
            "Find files",
            "find \".\"",
            None,
        );
        let second = candidate(
            Source::Habit,
            "b",
            InputKind::Nl,
            "Find files \n",
            "find \".\"\n",
            None,
        );
        assert_ne!(first.pair.id, second.pair.id);
        assert_eq!(deduplicate(vec![first.clone(), second]).len(), 1);
        let dir = tempfile::tempdir().unwrap();
        build(vec![first.clone()], None, dir.path()).unwrap();
        assert!(dir.path().join("candidates.jsonl").is_file());
        for name in ["pairs.jsonl", "rejected.jsonl", "unvalidated.jsonl"] {
            fs::write(dir.path().join(name), "old\n").unwrap();
        }
        build(vec![first], None, dir.path()).unwrap();
        for name in ["pairs.jsonl", "rejected.jsonl", "unvalidated.jsonl"] {
            assert!(!dir.path().join(name).exists(), "{name}");
        }
    }

    struct Fixed(Outcome);

    impl Validator for Fixed {
        fn check(&mut self, _request: &Request) -> Result<Outcome> {
            Ok(Outcome::default())
        }

        fn run(&mut self, _request: &Request) -> Result<Outcome> {
            Ok(self.0.clone())
        }
    }

    #[test]
    fn a_partial_execution_is_not_an_executed_pair() {
        for (status, executed) in [("ok", true), ("partial", false), ("failed", false)] {
            let mut pair = candidate(Source::Docs, "a", InputKind::Nl, "Find", "find", None);
            pair.tree = Some(Tree::default());
            let mut validator = Fixed(Outcome {
                status: Some(status.to_owned()),
                ..Outcome::default()
            });
            let checked = validate(&mut validator, &pair).unwrap();
            assert_eq!(checked.validation.executed, executed, "{status}");
            assert_eq!(checked.validation.rejected.is_some(), !executed, "{status}");
        }
    }

    #[test]
    fn reads_habit_tables() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bash.toml");
        fs::write(&path, "format = \"nurt-bench-habits/1\"\n[[pair]]\nshell = \"bash\"\nhabit = \"mv a b\"\ncanonical = \"move \\\"a\\\" to: \\\"b\\\"\"\nrule = \"B03\"\n").unwrap();
        let candidates = from_habits(&path).unwrap();
        assert_eq!(candidates[0].pair.origin, "bash.toml#B03");
        assert_eq!(candidates[0].pair.input_kind, InputKind::Bash);
        fs::write(&path, "format = \"nurt-bench-habits/1\"\n[[pair]]\nshell = \"zsh\"\nhabit = \"x\"\ncanonical = \"y\"\n").unwrap();
        assert!(from_habits(&path).is_err());
        fs::write(&path, "format = \"other\"\n").unwrap();
        assert!(from_habits(&path).is_err());
    }

    #[test]
    fn validates_and_writes_pairs() {
        let dir = tempfile::tempdir().unwrap();
        let tasks = [
            task("edit-001", "public", "write out.txt hello"),
            task("edit-003", "public", "bad"),
        ];
        let mut candidates = from_tasks(&tasks, dir.path()).unwrap();
        candidates.extend(from_page("docs/commands/replace.md", PAGE).unwrap());
        candidates.push(candidates[0].clone());
        let server = Arc::new(|method: &str, params: &Value| match method {
            "check"
                if params["program"]
                    .as_str()
                    .unwrap_or_default()
                    .starts_with("bad") =>
            {
                Ok(json!({"error": {"code": "E_UNKNOWN_COMMAND"}}))
            }
            "check" => Ok(json!({"bound": {}})),
            "run"
                if params["program"]
                    .as_str()
                    .unwrap_or_default()
                    .starts_with("replace") =>
            {
                Ok(json!({
                    "report": {"status": "failed", "error": {"code": "E_NOT_FOUND"}}
                }))
            }
            "run" => Ok(json!({"report": {"status": "ok"}})),
            _ => Err((-32601, "unknown".to_owned())),
        });
        let mut client = connect(server);
        let out = dir.path().join("out");
        let summary = build(candidates.clone(), Some(&mut client), &out).unwrap();
        assert!(summary.validated);
        let task_count = summary
            .sources
            .iter()
            .find(|s| s.source == Some(Source::Task))
            .unwrap();
        assert_eq!(
            (
                task_count.candidates,
                task_count.accepted,
                task_count.rejected
            ),
            (2, 1, 1)
        );
        let docs = summary
            .sources
            .iter()
            .find(|s| s.source == Some(Source::Docs))
            .unwrap();
        assert_eq!((docs.candidates, docs.accepted, docs.rejected), (2, 1, 1));
        let accepted = fs::read_to_string(out.join("pairs.jsonl")).unwrap();
        assert_eq!(accepted.lines().count(), summary.accepted);
        let first: Pair = serde_json::from_str(accepted.lines().next().unwrap()).unwrap();
        assert!(first.validation.checked && first.validation.executed);
        assert_eq!(first.license, LICENSE);
        let listed = build(candidates, None, &dir.path().join("dry")).unwrap();
        assert!(!listed.validated);
        assert!(dir.path().join("dry/candidates.jsonl").is_file());
    }

    fn fixture_engine() -> Engine {
        let frontend = nurt_core::fixtures::FixtureFrontend::new()
            .with_bound(
                "write out.txt hello",
                nurt_testkit::fixtures::empty_program(),
            )
            .with_error(
                "unknown",
                Error::new(ErrorCode::UnknownCommand, "unknown command `unknown`"),
            );
        open_engine(Some(Arc::new(frontend))).unwrap()
    }

    #[test]
    fn engine_validator_binds_known_programs_and_rejects_unknown_ones() {
        let dir = tempfile::tempdir().unwrap();
        let tasks = [
            task("edit-001", "public", "write out.txt hello"),
            task("edit-003", "public", "unknown"),
        ];
        let candidates = from_tasks(&tasks, dir.path()).unwrap();
        let mut validator = EngineValidator::new(fixture_engine());
        let summary = build(candidates, Some(&mut validator), &dir.path().join("out")).unwrap();
        assert_eq!((summary.accepted, summary.unvalidated), (3, 0));
        let rejected = fs::read_to_string(dir.path().join("out/rejected.jsonl")).unwrap();
        assert_eq!(rejected.lines().count(), 3);
        let accepted = fs::read_to_string(dir.path().join("out/pairs.jsonl")).unwrap();
        let pair: Pair = serde_json::from_str(accepted.lines().next().unwrap()).unwrap();
        assert!(pair.validation.checked);
        assert!(!pair.validation.executed);
        assert_eq!(
            pair.validation.note.as_deref(),
            Some("execution unavailable")
        );
    }

    #[test]
    fn engine_without_front_end_leaves_pairs_unvalidated() {
        let dir = tempfile::tempdir().unwrap();
        let tasks = [task("edit-001", "public", "write out.txt hello")];
        let candidates = from_tasks(&tasks, dir.path()).unwrap();
        let mut validator = EngineValidator::new(open_engine(None).unwrap());
        let summary = build(candidates, Some(&mut validator), &dir.path().join("out")).unwrap();
        assert_eq!((summary.accepted, summary.unvalidated), (0, 3));
        assert!(dir.path().join("out/unvalidated.jsonl").is_file());
    }

    #[test]
    fn takes_programs_of_passed_nurt_runs() {
        use crate::condition::Condition;
        use crate::oracle::{Observation, Verdict};
        use crate::runlog::tests::{end_event, header};
        use crate::runlog::{OracleEvent, Outcome, RunLog, ToolEvent};
        let dir = tempfile::tempdir().unwrap();
        let tool = |program: &str, is_error: bool| {
            Event::Tool(Box::new(ToolEvent {
                step: 1,
                call_id: "c".into(),
                t_ms: 0,
                duration_ms: 0,
                input: json!({"program": program}),
                output: String::new(),
                is_error,
                exit: None,
                timed_out: false,
                nurt: None,
            }))
        };
        for (condition, name) in [(Condition::Nurt, "a"), (Condition::Bash, "b")] {
            let mut log = RunLog::create(&dir.path().join(format!("runs/{name}.jsonl"))).unwrap();
            log.write(&Event::Run(Box::new(header(condition)))).unwrap();
            log.write(&tool("bad", true)).unwrap();
            log.write(&tool("first", false)).unwrap();
            log.write(&tool("second", false)).unwrap();
            log.write(&Event::Oracle(Box::new(OracleEvent {
                observation: Observation::default(),
                verdict: Verdict {
                    completed: true,
                    passed: true,
                    ..Verdict::default()
                },
            })))
            .unwrap();
            log.write(&end_event(Outcome::Answered)).unwrap();
            log.finish().unwrap();
        }
        let tasks = [task("edit-001", "public", "write")];
        let candidates = from_runs(&dir.path().join("runs"), &tasks, dir.path()).unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].pair.program, "first\nsecond");
        assert!(candidates[0].pair.script);
    }
}
