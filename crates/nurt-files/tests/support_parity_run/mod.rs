// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Execution of the parity scenarios on the stages of this crate.
//!
//! A scenario runs on a fresh [`TestContext`]: the tree `fs_before` is
//! written to the virtual file system, the program is parsed, every stage is
//! bound to its catalog signature and executed through the stages returned
//! by `nurt_files::register`, effect stages through a transaction of the
//! recording journal. The resulting report and tree are compared with
//! `expect`. A scenario whose command has no stage in the registry, whose
//! parameters are not in the catalog or whose fields have no counterpart in
//! the report is pending, with the reason.

// Each test target links a different subset of the helpers.
#![allow(dead_code)]

pub(crate) mod bind;
pub(crate) mod program;
pub(crate) mod report;
pub(crate) mod tree;

use std::path::Path;
use std::sync::Arc;

use nurt_contracts::envelope::RequestOptions;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::AbsPath;
use nurt_contracts::journal::{JournalStore, RollbackReason};
use nurt_contracts::record::Change;
use nurt_contracts::report::NotableEntry;
use nurt_contracts::stage::{
    ArgValue, BoundArgs, EffectStage, ExecCtx, InputKinds, RecordStream, StageData, StageImpl,
    StageInput, StageOutput,
};
use nurt_contracts::value::{Record, RecordKind, Value};
use nurt_files::common::etag::{etag_text, observe};
use nurt_testkit::ctx::TestContext;
use nurt_testkit::fixtures::begin_request;
use nurt_testkit::journal::JournalCall;
use serde_json::{Map, Value as Json};

use self::bind::{BindCx, BindFailure, Bound, Catalog, UNKNOWN_ETAG, bind};
use self::program::{Call, Lit, Pipeline, parse_script};
use self::report::{
    Comparison, Item, Report, STATUS_FAILED, STATUS_OK, STATUS_PLANNED, STATUS_ROLLED_BACK,
    check_field, resolve,
};
use crate::support_parity::{Model, Scenario};

/// Reason prefix of scenarios whose command belongs to a unit of this crate
/// that is not merged yet; such scenarios run once the stage is registered.
pub(crate) const NOT_MERGED: &str = "command not merged";
/// Reason prefix of scenarios that need a stage of another crate.
pub(crate) const OUTSIDE_CRATE: &str = "stage outside nurt-files";
/// Reason prefix of scenarios that name a command the catalog lacks.
pub(crate) const NOT_IN_CATALOG: &str = "command not in the catalog";

/// Commands of this crate implemented by other units.
const CRATE_COMMANDS: [&str; 4] = ["compress", "decompress", "diff", "patch"];
const CRATE_GROUPS: [&str; 2] = ["archive.", "notebook."];

/// File name extensions of files that only the non-text reader reads.
const NON_TEXT_EXTENSIONS: [&str; 13] = [
    "png", "jpg", "jpeg", "gif", "webp", "bmp", "tif", "tiff", "pdf", "gz", "bz2", "xz", "zst",
];
/// Parameters of `read` that only the non-text reader accepts.
const NON_TEXT_PARAMS: [&str; 3] = ["pages", "mode", "decompress"];
const READ: &str = "read";
/// Longest description of the observed report in a difference.
const OBSERVED_LIMIT: usize = 900;

/// Result of one scenario.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// Report and tree match the expectation.
    Agree,
    /// Differences between the expectation and the outcome.
    Diverge(Vec<String>),
    /// The scenario cannot run on the stages of this crate.
    Pending(String),
}

struct Carried {
    records: Vec<Result<Record>>,
    header: Option<Record>,
    order: Option<nurt_contracts::stage::StreamOrder>,
    render: Option<nurt_contracts::stage::RenderHint>,
}

/// Runs scenarios on the registered stages.
pub(crate) struct Runner {
    catalog: Catalog,
    stages: Vec<Arc<dyn StageImpl>>,
    non_text_read: bool,
}

impl Runner {
    /// Registers the stages of the crate and loads the catalog in `catalog_dir`.
    ///
    /// # Errors
    ///
    /// Returns the message of the catalog loader or of the registrar.
    pub(crate) fn new(catalog_dir: &Path) -> std::result::Result<Self, String> {
        let catalog = Catalog::load(catalog_dir)?;
        let mut stages: Vec<Arc<dyn StageImpl>> = Vec::new();
        nurt_files::register(&mut stages).map_err(|e| e.to_string())?;
        let mut runner = Self {
            catalog,
            stages,
            non_text_read: false,
        };
        runner.non_text_read = runner.probe_non_text_read();
        Ok(runner)
    }

    /// Whether `read` reads images: the reader of non-text files is merged.
    pub(crate) fn reads_non_text(&self) -> bool {
        self.non_text_read
    }

    /// Whether a stage of `command` is registered.
    pub(crate) fn registered(&self, command: &str) -> bool {
        self.stages
            .iter()
            .any(|stage| stage.descriptor().command == command)
    }

    fn probe_non_text_read(&self) -> bool {
        let test = TestContext::new();
        let Ok(path) = tree::abs("probe.png") else {
            return false;
        };
        let written = tree::root()
            .and_then(|root| test.fs().create_dir_all(&root).map_err(|e| e.to_string()))
            .and_then(|()| {
                test.fs()
                    .write_file(&path, &tree::png(1, 1))
                    .map_err(|e| e.to_string())
            });
        let Some(stage) = self.select(READ, true, None) else {
            return false;
        };
        let Some(reader) = stage.as_read() else {
            return false;
        };
        if written.is_err() {
            return false;
        }
        let args = BoundArgs::new(READ).with(
            "path",
            ArgValue::Value(Value::List(vec![Value::Path(path.to_language_string())])),
            false,
        );
        let ctx = test.ctx();

        match reader.execute(&args, StageInput::none(), &ctx) {
            Ok(output) => match output.data {
                StageData::Records(mut stream) => match stream.next() {
                    Some(Ok(_)) => true,
                    Some(Err(error)) => error.code() != ErrorCode::Binary,
                    None => false,
                },
                _ => false,
            },
            Err(_) => false,
        }
    }

    fn select(
        &self,
        command: &str,
        source: bool,
        kind: Option<RecordKind>,
    ) -> Option<Arc<dyn StageImpl>> {
        let mut candidates = self
            .stages
            .iter()
            .filter(|stage| stage.descriptor().command == command);
        let found = if source {
            candidates.find(|stage| stage.descriptor().source)
        } else {
            match kind {
                Some(kind) => candidates.find(|stage| stage.descriptor().input.accepts(kind)),
                None => {
                    candidates.find(|stage| !matches!(stage.descriptor().input, InputKinds::None))
                }
            }
        };
        found.cloned()
    }

    fn is_effect(&self, command: &str) -> bool {
        self.catalog
            .command(command)
            .is_some_and(|spec| spec.effect != "read")
    }

    /// Reason a stage of `call` cannot run, or `None`.
    fn unavailable(&self, call: &Call) -> Option<String> {
        let command = call.command.as_str();
        if self.catalog.command(command).is_none() {
            return Some(format!("{NOT_IN_CATALOG}: {command}"));
        }
        if !self.registered(command) {
            let in_crate = CRATE_COMMANDS.contains(&command)
                || CRATE_GROUPS.iter().any(|group| command.starts_with(group));
            return Some(if in_crate {
                format!("{NOT_MERGED}: {command}")
            } else {
                format!("{OUTSIDE_CRATE}: {command}")
            });
        }
        if command == READ && !self.non_text_read && needs_non_text_reader(call) {
            return Some(format!("{NOT_MERGED}: read of non-text files"));
        }
        None
    }

    /// Runs `scenario`.
    pub(crate) fn run(&self, model: &Model, scenario: &Scenario) -> Outcome {
        match self.try_run(model, scenario) {
            Ok(outcome) => outcome,
            Err(reason) => Outcome::Pending(reason),
        }
    }

    fn try_run(&self, model: &Model, scenario: &Scenario) -> std::result::Result<Outcome, String> {
        let pipelines = parse_script(&scenario.nurt_program)
            .map_err(|e| format!("program does not parse: {e}"))?;
        for call in pipelines.iter().flatten() {
            if let Some(reason) = self.unavailable(call) {
                return Err(reason);
            }
        }
        let soft_reason = match precondition(scenario, &pipelines) {
            Some(Precondition::Blocks(reason)) => return Err(reason),
            Some(Precondition::Explains(reason)) => Some(reason),
            None => None,
        };
        for pipeline in &pipelines {
            let inner = &pipeline[..pipeline.len().saturating_sub(1)];
            if inner.iter().any(|call| self.is_effect(&call.command)) {
                return Err("stages after an effect stage are run by the executor".to_owned());
            }
        }
        let empty = Report::default();
        for check in &scenario.expect.fields {
            resolve(&empty, &check.field)?;
        }
        let test = TestContext::new();
        tree::populate(&test, model, &scenario.fs_before)
            .map_err(|e| format!("tree cannot be built: {e}"))?;
        let before = tree::snapshot(&test)?;
        let root = tree::root()?.to_language_string();
        let mut report = Report::default();
        for pipeline in &pipelines {
            report = self.run_pipeline(&test, pipeline, &root)?;
            if report.error.is_some() {
                break;
            }
        }
        let report = report.relative_to(&root);
        let after = tree::snapshot(&test)?;
        let mut problems = Vec::new();
        if scenario.expect.fs_unchanged {
            problems.extend(tree::tree_differences(&before, &after));
        }
        for entry in &scenario.expect.fs_after {
            if let Err(problem) = tree::check_after(model, entry, &after) {
                problems.push(problem);
            }
        }
        for check in &scenario.expect.fields {
            let (comparison, value) = match (&check.value, &check.contains, &check.excludes) {
                (Some(value), _, _) => (Comparison::Value, value),
                (None, Some(value), _) => (Comparison::Contains, value),
                (None, None, Some(value)) => (Comparison::Excludes, value),
                (None, None, None) => {
                    return Err(format!("field check {} has no value", check.field));
                }
            };
            let expected = serde_json::to_value(value).map_err(|e| e.to_string())?;
            match check_field(&report, &check.field, comparison, &expected) {
                Ok(()) => {}
                Err(Ok(problem)) => problems.push(problem),
                Err(Err(reason)) => return Err(reason),
            }
        }
        if problems.is_empty() {
            return Ok(Outcome::Agree);
        }
        if let Some(reason) = soft_reason {
            return Err(reason);
        }
        let observed: String = observed(&report)
            .to_string()
            .chars()
            .take(OBSERVED_LIMIT)
            .collect();
        problems.push(format!("observed: {observed}"));
        let error_checked = scenario
            .expect
            .fields
            .iter()
            .any(|f| f.field.contains("error"));
        if let (Some(error), false) = (&report.error, error_checked) {
            let code = error.get("code").cloned().unwrap_or(Json::Null);
            let message = error.get("message").cloned().unwrap_or(Json::Null);
            problems.push(format!("program ended with {code}: {message}"));
        }
        Ok(Outcome::Diverge(problems))
    }

    fn run_pipeline(
        &self,
        test: &TestContext,
        pipeline: &Pipeline,
        root: &str,
    ) -> std::result::Result<Report, String> {
        let fs = Arc::clone(test.fs());
        let etag_of = move |text: &str| -> String {
            AbsPath::new(text)
                .ok()
                .and_then(|path| observe(fs.as_ref(), &path).ok())
                .and_then(|observed| observed.etag())
                .map_or_else(|| UNKNOWN_ETAG.to_owned(), |etag| etag_text(&etag))
        };
        let cx = BindCx {
            root,
            etag_of: &etag_of,
        };
        let mut bound = Vec::new();
        for (index, call) in pipeline.iter().enumerate() {
            let spec = self
                .catalog
                .command(&call.command)
                .ok_or_else(|| format!("{NOT_IN_CATALOG}: {}", call.command))?;
            match bind(call, spec, index == 0, &cx) {
                Ok(arguments) => bound.push(arguments),
                Err(BindFailure::Pending(reason)) => return Err(reason),
                Err(BindFailure::Error(error)) => return Ok(failed(&error, false)),
            }
        }
        let mut options = Map::new();
        for arguments in &bound {
            options.extend(arguments.options.clone());
        }
        let options: RequestOptions = serde_json::from_value(Json::Object(options))
            .map_err(|e| format!("request options are invalid: {e}"))?;
        let preview = options.preview();
        let ctx = test.builder().options(Arc::new(options)).build();
        self.execute(test, pipeline, &bound, &ctx, preview)
    }

    fn execute(
        &self,
        test: &TestContext,
        pipeline: &Pipeline,
        bound: &[Bound],
        ctx: &ExecCtx,
        preview: bool,
    ) -> std::result::Result<Report, String> {
        let mut carried: Option<Carried> = None;
        let last = pipeline.len().saturating_sub(1);
        for (index, (call, arguments)) in pipeline.iter().zip(bound).enumerate() {
            let kind = carried.as_ref().and_then(|c| {
                c.records
                    .iter()
                    .find_map(|r| r.as_ref().ok().map(|r| r.kind))
            });
            let stage = self
                .select(&call.command, index == 0, kind)
                .ok_or_else(|| format!("no stage of `{}` accepts this input", call.command))?;
            let input = match carried.take() {
                None => StageInput::none(),
                Some(c) => StageInput {
                    data: StageData::Records(RecordStream::new(c.records.into_iter())),
                    header: c.header,
                    order: c.order,
                    render: c.render,
                },
            };
            if self.is_effect(&call.command)
                && let Some(effect) = stage.as_effect()
            {
                return Ok(run_effect(
                    test,
                    effect,
                    &call.command,
                    &arguments.args,
                    input,
                    ctx,
                    preview,
                ));
            }
            let reader = stage
                .as_read()
                .ok_or_else(|| format!("`{}` has no read interface", call.command))?;
            let output = match reader.execute(&arguments.args, input, ctx) {
                Ok(output) => output,
                Err(error) => return Ok(failed(&error, false)),
            };
            let StageOutput {
                data,
                header,
                order,
                render,
            } = output;
            let (records, value) = match data {
                StageData::Records(stream) => (stream.collect::<Vec<_>>(), None),
                StageData::Value(value) => (Vec::new(), Some(value)),
                _ => (Vec::new(), None),
            };
            if index == last {
                return Ok(read_report(records, value));
            }
            if value.is_some() {
                return Err(
                    "a single value passed between stages is not run by the scenario runner"
                        .to_owned(),
                );
            }
            carried = Some(Carried {
                records,
                header,
                order,
                render,
            });
        }
        Ok(Report::default())
    }
}

/// Parameters whose effect needs the document or code service of another
/// group; the test doubles provide neither.
const SERVICE_PARAMS: [&str; 6] = [
    "symbol",
    "scope",
    "enclosing",
    "check_references",
    "key",
    "section",
];
/// Views of `read` computed by the code service.
const SERVICE_VIEWS: [&str; 3] = ["outline", "symbols", "outline_first"];
/// Details of `info` computed by services of other groups.
const SERVICE_DETAILS: [&str; 2] = ["symbols", "git"];
/// Notable codes reported only with the code service.
const SERVICE_NOTABLE: [&str; 1] = ["references_left"];
const SERVICE_REASON: &str =
    "needs the document or code service of another group, which the test doubles do not provide";

fn uses_service(call: &Call) -> bool {
    call.named.iter().any(|(name, lit)| {
        let words = |lit: &Lit| -> Vec<String> {
            match lit {
                Lit::Ident(word) | Lit::Str(word) => vec![word.clone()],
                Lit::List(items) => items
                    .iter()
                    .filter_map(|item| match item {
                        Lit::Ident(word) | Lit::Str(word) => Some(word.clone()),
                        _ => None,
                    })
                    .collect(),
                _ => Vec::new(),
            }
        };
        let in_maps = match lit {
            Lit::List(items) => items.iter().any(|item| match item {
                Lit::Map(entries) => entries
                    .iter()
                    .any(|(key, _)| SERVICE_PARAMS.contains(&key.as_str())),
                _ => false,
            }),
            _ => false,
        };
        let scope_any = name == "scope" && words(lit) == ["any"];
        (SERVICE_PARAMS.contains(&name.as_str()) && !scope_any)
            || in_maps
            || (name == "view"
                && words(lit)
                    .iter()
                    .any(|w| SERVICE_VIEWS.contains(&w.as_str())))
            || (name == "details"
                && words(lit)
                    .iter()
                    .any(|w| SERVICE_DETAILS.contains(&w.as_str())))
            || (name == "mode" && words(lit) == ["syntax"])
    })
}

/// Condition of a scenario that follows from its program and expectation.
enum Precondition {
    /// The scenario cannot run.
    Blocks(String),
    /// The scenario runs; a difference is explained by this reason.
    Explains(String),
}

/// Condition of `scenario`, or `None` when nothing limits the run.
fn precondition(scenario: &Scenario, pipelines: &[Pipeline]) -> Option<Precondition> {
    let calls = || pipelines.iter().flatten();
    let external_step = calls().any(|call| {
        call.positional.iter().any(|lit| match lit {
            Lit::Ident(word) => word == "after",
            Lit::Raw(raw) => raw.ends_with(','),
            _ => false,
        })
    });
    if external_step {
        return Some(Precondition::Blocks(
            "the program changes the tree or the task between two steps, which the runner does not perform"
                .to_owned(),
        ));
    }
    let fields = &scenario.expect.fields;
    for field in fields.iter().filter(|f| f.field == "notable") {
        if let Some(code) = field.value.as_ref().and_then(|v| v.as_str())
            && serde_json::from_value::<nurt_contracts::report::Notable>(Json::from(code)).is_err()
        {
            return Some(Precondition::Blocks(format!(
                "notable code `{code}` is not in the contract; open item LP-23"
            )));
        }
    }
    if calls().any(uses_service) {
        return Some(Precondition::Explains(SERVICE_REASON.to_owned()));
    }
    let expects = |field: &str, word: &str| {
        fields
            .iter()
            .any(|f| f.field == field && f.value.as_ref().and_then(|v| v.as_str()) == Some(word))
    };
    if expects("effect.status", "rolled_back") {
        return Some(Precondition::Explains(format!(
            "syntax validation {SERVICE_REASON}"
        )));
    }
    if SERVICE_NOTABLE.iter().any(|code| expects("notable", code)) {
        return Some(Precondition::Explains(format!(
            "references {SERVICE_REASON}"
        )));
    }
    let freshness = scenario.class == crate::support_parity::Class::Freshness;
    if freshness && !scenario.nurt_program.contains("if_match") {
        return Some(Precondition::Explains(
            "the requirement to read before a change depends on open item LP-24".to_owned(),
        ));
    }
    None
}

/// Compact form of a report for the description of a difference.
fn observed(report: &Report) -> Json {
    let items: Vec<Json> = report
        .items
        .iter()
        .take(2)
        .map(|item| match item {
            Item::Record(kind, body) => serde_json::json!({ kind.as_str(): body }),
            Item::Error(error) => serde_json::json!({ "error": error.get("code") }),
        })
        .collect();
    serde_json::json!({
        "status": report.status,
        "error": report.error.as_ref().and_then(|e| e.get("code")),
        "notable": report.notable,
        "change": report.changes.first(),
        "records": items,
    })
}

fn needs_non_text_reader(call: &Call) -> bool {
    let non_text_path = call.positional.iter().any(|lit| match lit {
        Lit::Str(text) => text.rsplit_once('.').is_some_and(|(_, extension)| {
            NON_TEXT_EXTENSIONS.contains(&extension.to_ascii_lowercase().as_str())
        }),
        _ => false,
    });
    let non_text_param = call.named.iter().any(|(name, lit)| {
        NON_TEXT_PARAMS.contains(&name.as_str())
            || (name == "view" && *lit == Lit::Ident("hex".to_owned()))
    });
    non_text_path || non_text_param
}

fn error_json(error: &Error) -> Json {
    serde_json::to_value(error.data()).unwrap_or(Json::Null)
}

fn failed(error: &Error, effect: bool) -> Report {
    Report {
        status: STATUS_FAILED.to_owned(),
        error: Some(error_json(error)),
        effect,
        ..Report::default()
    }
}

fn kind_name(kind: RecordKind) -> String {
    serde_json::to_value(kind)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_default()
}

fn items(records: Vec<Result<Record>>) -> Vec<Item> {
    records
        .into_iter()
        .map(|record| match record {
            Ok(record) => Item::Record(kind_name(record.kind), record.to_plain_json()),
            Err(error) => Item::Error(error_json(&error)),
        })
        .collect()
}

fn read_report(records: Vec<Result<Record>>, value: Option<Value>) -> Report {
    let mut report = Report {
        status: STATUS_OK.to_owned(),
        items: items(records),
        value: value.map(|value| value.to_plain_json()),
        ..Report::default()
    };
    report.promote_stream_error();
    report
}

fn codes(entries: &[NotableEntry]) -> Vec<String> {
    entries
        .iter()
        .filter_map(|entry| serde_json::to_value(entry.code).ok())
        .filter_map(|value| value.as_str().map(str::to_owned))
        .collect()
}

fn changes(entries: &[Change]) -> Vec<Json> {
    entries
        .iter()
        .map(|change| Record::from(change.clone()).to_plain_json())
        .collect()
}

fn applied_count(test: &TestContext) -> usize {
    test.journal()
        .calls()
        .into_iter()
        .filter(|call| matches!(call, JournalCall::Apply { ok: true, .. }))
        .count()
}

fn output_items(output: StageOutput<'_>) -> Vec<Item> {
    match output.data {
        StageData::Records(stream) => items(stream.collect()),
        _ => Vec::new(),
    }
}

fn run_effect<'a>(
    test: &TestContext,
    effect: &'a dyn EffectStage,
    command: &str,
    args: &'a BoundArgs,
    input: StageInput<'a>,
    ctx: &'a ExecCtx,
    preview: bool,
) -> Report {
    let plan = match effect.plan(args, input, ctx) {
        Ok(plan) => plan,
        Err(error) => return failed(&error, true),
    };
    let mut report = Report {
        effect: true,
        notable: codes(&plan.notable),
        ..Report::default()
    };
    if preview {
        report.status = STATUS_PLANNED.to_owned();
        report.changes = changes(&plan.changes);
        report.irreversible = plan.irreversible.len();
        report.items = items(plan.preview.iter().cloned().map(Ok).collect());
        return report;
    }
    let applied_before = applied_count(test);
    let mut tx = test.journal().begin(begin_request(command));
    match effect.apply(args, plan, tx.as_mut(), ctx) {
        Ok(outcome) => {
            report.changes = changes(&outcome.changes);
            report.notable.extend(codes(&outcome.notable));
            report.irreversible = outcome.irreversible.len();
            report.items = output_items(outcome.output);
            match tx.commit() {
                Ok(_) => report.status = STATUS_OK.to_owned(),
                Err(error) => {
                    report.status = STATUS_FAILED.to_owned();
                    report.error = Some(error_json(&error));
                }
            }
        }
        Err(error) => {
            let applied = applied_count(test) > applied_before;
            report.status = if applied {
                STATUS_ROLLED_BACK
            } else {
                STATUS_FAILED
            }
            .to_owned();
            report.error = Some(error_json(&error));
            if let Err(rollback) = tx.rollback(RollbackReason::TargetFailed) {
                report
                    .notable
                    .push(format!("rollback failed: {}", rollback.code()));
            }
        }
    }
    report
}
