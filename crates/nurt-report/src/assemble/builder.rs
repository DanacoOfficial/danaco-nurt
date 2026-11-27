// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-report::assemble::builder
// Responsibility:   Builder of `nurt-report/1` reports.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Builder of `nurt-report/1` reports.

use nurt_contracts::error::Error;
use nurt_contracts::kinds::EffectKind;
use nurt_contracts::record::Operation;
use nurt_contracts::report::{
    Change, Correction, Hunk, IrreversibleEntry, Notable, NotableEntry, RecordsPage, Report,
    ReportPart, ReportStatus, ReportViolation, Targets, Timing,
};
use nurt_contracts::value::Record;

/// The largest number of parts of a control construct a report lists.
const MAX_PARTS: usize = 200;

use super::bulk::summarize_bulk;
use super::context::ReportContext;
use super::input::{ScopeDirectory, TargetChange};
use super::summary::{count_targets, summarize_changes};
use crate::rules::{collect_irreversible, derive_notable, merge_notable};

/// Whether the program changes targets or only reads them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Read,
    Change,
}

/// Builder of the report of one program.
///
/// A builder for a changing program ([`ReportBuilder::changing`]) derives
/// from the changes:
///
/// * `targets`, unless set with [`ReportBuilder::targets`];
/// * `summary`, the totals of every change;
/// * `bulk`, when the number of targets reaches `report.mass_threshold`;
/// * the notable effects computed from changes, merged with the entries
///   emitted by commands, and `nothing_matched` when a program that
///   finished or was previewed had no targets, unless it runs a program
///   ([`ReportBuilder::effect`] with a process effect, or an `exec` or `job`
///   record);
/// * `irreversible`, from the changes and the entries of commands and the
///   journal;
/// * `diff`, from the hunks carried by the listed changes.
///
/// The operation number is kept only for a committed operation with at least
/// one target: it is dropped for previews, reading programs and changing
/// programs without targets. The report carries nothing about undo.
///
/// A builder for a reading program ([`ReportBuilder::reading`]) carries the
/// records, the targets as given and the entries emitted by commands.
#[derive(Debug, Clone)]
pub struct ReportBuilder<'c> {
    ctx: &'c ReportContext,
    mode: Mode,
    report: Report,
    op: Option<u64>,
    targets: Option<Targets>,
    changes: Vec<TargetChange>,
    scope_directories: Vec<ScopeDirectory>,
    notes: Vec<NotableEntry>,
    irreversible: Vec<IrreversibleEntry>,
    list_limit: Option<usize>,
    effect: Option<EffectKind>,
}

impl<'c> ReportBuilder<'c> {
    /// A builder for a program with a changing stage; `kind` is the name of
    /// the changing command, for example `replace` or `remove`.
    #[must_use]
    pub fn changing(ctx: &'c ReportContext, kind: impl Into<String>, status: ReportStatus) -> Self {
        Self::with_mode(ctx, Mode::Change, kind.into(), status)
    }

    /// A builder for a reading program; `kind` is the name of its source
    /// command, for example `find`.
    #[must_use]
    pub fn reading(ctx: &'c ReportContext, kind: impl Into<String>, status: ReportStatus) -> Self {
        Self::with_mode(ctx, Mode::Read, kind.into(), status)
    }

    fn with_mode(ctx: &'c ReportContext, mode: Mode, kind: String, status: ReportStatus) -> Self {
        Self {
            ctx,
            mode,
            report: Report::new(status, kind),
            op: None,
            targets: None,
            changes: Vec::new(),
            scope_directories: Vec::new(),
            notes: Vec::new(),
            irreversible: Vec::new(),
            list_limit: None,
            effect: None,
        }
    }

    /// Sets the kind of effect of the changing command. A command that runs a
    /// program has no targets to match, so the report does not say
    /// `nothing_matched` for it. Commands with an external effect that act on
    /// input records, such as `start`, still report it.
    #[must_use]
    pub fn effect(mut self, effect: EffectKind) -> Self {
        self.effect = Some(effect);
        self
    }

    /// Sets the number of the journal entry of the operation.
    #[must_use]
    pub fn op(mut self, op: u64) -> Self {
        self.op = Some(op);
        self
    }

    /// Takes the operation number and the irreversible effects recorded by
    /// the journal for the operation.
    #[must_use]
    pub fn operation(mut self, operation: &Operation) -> Self {
        self.op = Some(operation.id);
        self.irreversible
            .extend(operation.irreversible.iter().cloned());
        self
    }

    /// Adds one change, in enumeration order.
    #[must_use]
    pub fn change(mut self, change: impl Into<TargetChange>) -> Self {
        self.changes.push(change.into());
        self
    }

    /// Adds changes, in enumeration order.
    #[must_use]
    pub fn changes<I>(mut self, changes: I) -> Self
    where
        I: IntoIterator,
        I::Item: Into<TargetChange>,
    {
        self.changes.extend(changes.into_iter().map(Into::into));
        self
    }

    /// Sets the targets enumerated by the program, when they differ from the
    /// targets counted from the changes. The `listed` field is always
    /// derived from the listed changes.
    #[must_use]
    pub fn targets(mut self, targets: Targets) -> Self {
        self.targets = Some(targets);
        self
    }

    /// Adds a directory whose deleted share is measured for `mass_delete`.
    #[must_use]
    pub fn scope_directory(mut self, directory: ScopeDirectory) -> Self {
        self.scope_directories.push(directory);
        self
    }

    /// Adds a notable effect emitted by a command.
    #[must_use]
    pub fn note(mut self, entry: NotableEntry) -> Self {
        self.notes.push(entry);
        self
    }

    /// Adds notable effects emitted by commands.
    #[must_use]
    pub fn notes(mut self, entries: impl IntoIterator<Item = NotableEntry>) -> Self {
        self.notes.extend(entries);
        self
    }

    /// Adds an irreversible effect reported by a command or the journal.
    #[must_use]
    pub fn irreversible(mut self, entry: IrreversibleEntry) -> Self {
        self.irreversible.push(entry);
        self
    }

    /// Adds irreversible effects reported by commands or the journal.
    #[must_use]
    pub fn irreversible_entries(
        mut self,
        entries: impl IntoIterator<Item = IrreversibleEntry>,
    ) -> Self {
        self.irreversible.extend(entries);
        self
    }

    /// Lists no more than `limit` changes, in enumeration order, together with
    /// their hunks. Totals, bulk summary and notable effects still cover
    /// every change.
    #[must_use]
    pub fn list_limit(mut self, limit: usize) -> Self {
        self.list_limit = Some(limit);
        self
    }

    /// Sets the canonical text of the program that ran after corrections.
    #[must_use]
    pub fn ran(mut self, text: impl Into<String>) -> Self {
        self.report.ran = Some(text.into());
        self
    }

    /// Sets the hash of the syntax tree, `sha256:` and 64 hexadecimal digits.
    #[must_use]
    pub fn ast_hash(mut self, hash: impl Into<String>) -> Self {
        self.report.ast_hash = Some(hash.into());
        self
    }

    /// Sets the path of the file that holds the full result.
    #[must_use]
    pub fn result_file(mut self, path: impl Into<String>) -> Self {
        self.report.result_file = Some(path.into());
        self
    }

    /// Sets the corrections applied to the program text.
    #[must_use]
    pub fn corrections(mut self, corrections: Vec<Correction>) -> Self {
        self.report.corrections = corrections;
        self
    }

    /// Sets the record of kind `exec` of a program that ran.
    #[must_use]
    pub fn exec(mut self, record: Record) -> Self {
        self.report.exec = Some(record);
        self
    }

    /// Sets the record of kind `job` of a program that continues in the
    /// background.
    #[must_use]
    pub fn job(mut self, record: Record) -> Self {
        self.report.job = Some(record);
        self
    }

    /// Sets the record of kind `test_run` of a test run.
    #[must_use]
    pub fn test(mut self, record: Record) -> Self {
        self.report.test = Some(record);
        self
    }

    /// Sets the record of kind `analysis` of an analysis verdict.
    #[must_use]
    pub fn analysis(mut self, record: Record) -> Self {
        self.report.analysis = Some(record);
        self
    }

    /// Sets the record of the effect on a repository.
    #[must_use]
    pub fn git(mut self, record: Record) -> Self {
        self.report.git = Some(record);
        self
    }

    /// Sets the records returned by a reading program.
    #[must_use]
    pub fn records(mut self, page: RecordsPage) -> Self {
        self.report.records = Some(page);
        self
    }

    /// Sets the error that stopped the program, or the check that failed
    /// after the effect.
    #[must_use]
    pub fn error(mut self, error: Error) -> Self {
        self.report.error = Some(error);
        self
    }

    /// Sets the cursor of the next page.
    #[must_use]
    pub fn next_cursor(mut self, cursor: impl Into<String>) -> Self {
        self.report.next_cursor = Some(cursor.into());
        self
    }

    /// Sets the parts of a control construct. The report keeps the first
    /// `MAX_PARTS` parts and states the total when there are more.
    #[must_use]
    pub fn parts(mut self, mut parts: Vec<ReportPart>) -> Self {
        let total = u64::try_from(parts.len()).unwrap_or(u64::MAX);
        if parts.len() > MAX_PARTS {
            parts.truncate(MAX_PARTS);
            self.report.parts_total = Some(total);
        }
        self.report.parts = parts;
        self
    }

    /// Sets the durations of the processing steps.
    #[must_use]
    pub fn timing(mut self, timing: Timing) -> Self {
        self.report.timing = Some(timing);
        self
    }

    /// Assembles the report.
    ///
    /// # Errors
    ///
    /// Returns the first rule of the report format that the assembled report
    /// breaks, for example a status `failed` without an error, a status
    /// `running` without a job record or a rename without its previous path.
    pub fn build(self) -> Result<Report, ReportViolation> {
        let report = match self.mode {
            Mode::Change => self.assemble_change(),
            Mode::Read => self.assemble_read(),
        };
        report.validate().map(|()| report)
    }

    fn assemble_change(self) -> Report {
        let Self {
            ctx,
            mut report,
            op,
            targets,
            changes,
            scope_directories,
            notes,
            irreversible,
            list_limit,
            effect,
            ..
        } = self;
        let targets = targets.unwrap_or_else(|| count_targets(&changes));
        let config = ctx.config();
        report.summary = summarize_changes(&changes);
        if !changes.is_empty() && targets.count >= u64::from(config.mass_threshold) {
            report.bulk = Some(summarize_bulk(&changes));
        }
        let mut derived = derive_notable(ctx, &changes, targets.count, &scope_directories);
        let matches_targets =
            effect != Some(EffectKind::Process) && report.exec.is_none() && report.job.is_none();
        if matches_targets
            && targets.count == 0
            && matches!(report.status, ReportStatus::Ok | ReportStatus::Planned)
        {
            derived.push(NotableEntry::new(Notable::NothingMatched));
        }
        report.notable = merge_notable(derived, notes);
        report.irreversible = collect_irreversible(&changes, irreversible);
        report.op = op.filter(|_| report.status != ReportStatus::Planned && targets.count > 0);
        list(report, Some(targets), changes, list_limit)
    }

    fn assemble_read(self) -> Report {
        let Self {
            mut report,
            targets,
            changes,
            notes,
            irreversible,
            list_limit,
            ..
        } = self;
        report.notable = merge_notable(Vec::new(), notes);
        report.irreversible = collect_irreversible(&changes, irreversible);
        list(report, targets, changes, list_limit)
    }
}

/// Places the listed changes and their hunks in the report and sets
/// `targets.listed` to the number of listed changes.
fn list(
    mut report: Report,
    targets: Option<Targets>,
    changes: Vec<TargetChange>,
    limit: Option<usize>,
) -> Report {
    let limit = limit.unwrap_or(usize::MAX);
    let mut listed: Vec<Change> = Vec::with_capacity(changes.len().min(limit));
    let mut diff: Vec<Hunk> = Vec::new();
    for target in changes.into_iter().take(limit) {
        let mut change = target.change;
        if let Some(hunks) = change.diff.take() {
            diff.extend(hunks);
        }
        listed.push(change);
    }
    report.targets = targets.map(|targets| Targets {
        listed: (!listed.is_empty()).then_some(listed.len() as u64),
        ..targets
    });
    report.changes = listed;
    report.diff = diff;
    report
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
