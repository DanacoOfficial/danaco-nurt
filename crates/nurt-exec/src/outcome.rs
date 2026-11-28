// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-exec::outcome
// Responsibility:   Result of an execution, in contract types.
// Dependencies:     nurt_contracts, serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Result of an execution, in contract types.

use std::time::Duration;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::FileKind;
use nurt_contracts::journal::{OpHandle, UndoOutcome};
use nurt_contracts::record::Change;
use nurt_contracts::report::{IrreversibleEntry, NotableEntry, ReportStatus};
use nurt_contracts::stage::{
    EffectPlan, ExecStep, RecordStream, RenderHint, StageData, StageInput, StageOutput, StreamOrder,
};
use nurt_contracts::value::{Instant, Record, Value};
use serde_json::json;

/// Result of [`crate::Executor::run`].
///
/// One [`OperationOutcome`] per executed operation, in program order; a
/// script stops at the first operation that ends with an error. The status
/// and the error of the whole execution are those of the operation that
/// ended it, or of the named result whose computation failed.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Outcome {
    /// Status of the execution.
    pub status: ReportStatus,
    /// Error that ended the execution.
    pub error: Option<Error>,
    /// Executed operations in program order.
    pub operations: Vec<OperationOutcome>,
    /// Named results computed, in program order, without `@`.
    pub lets: Vec<String>,
    /// Wall time at the start of the execution.
    pub started: Instant,
    /// Wall time at the end of the execution.
    pub finished: Instant,
    /// Duration of the execution on the monotonic clock.
    pub elapsed: Duration,
}

impl Outcome {
    pub(crate) fn new(started: Instant) -> Self {
        Self {
            status: ReportStatus::Ok,
            error: None,
            operations: Vec::new(),
            lets: Vec::new(),
            started,
            finished: started,
            elapsed: Duration::ZERO,
        }
    }

    pub(crate) fn fail(&mut self, error: Error) {
        self.status = ReportStatus::Failed;
        self.error = Some(error);
    }

    pub(crate) fn push(&mut self, operation: OperationOutcome) {
        self.status = operation.status;
        self.error.clone_from(&operation.error);
        self.operations.push(operation);
    }

    /// The last executed operation.
    #[must_use]
    pub fn last(&self) -> Option<&OperationOutcome> {
        self.operations.last()
    }

    /// Returns `true` when the execution ended without an error.
    #[must_use]
    pub fn is_ok(&self) -> bool {
        self.error.is_none()
    }
}

/// Result of one operation.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct OperationOutcome {
    /// Index of the operation in the program, from 0.
    pub index: u32,
    /// Status of the operation.
    pub status: ReportStatus,
    /// Records or value produced by the last executed stage. After a failed
    /// stage of step `5a` these are the records of the effect.
    pub result: ResultData,
    /// The effect, for pipelines with an effect stage.
    pub effect: Option<EffectReport>,
    /// Unusual effects noted by the stages and by the executor, in order.
    pub notable: Vec<NotableEntry>,
    /// Error that ended the operation.
    pub error: Option<Error>,
    /// Time spent per execution step, in execution order. Steps before the
    /// effect run lazily, so their time is counted in the step that pulls
    /// their records: `read` for a pipeline without effect, `plan` for a
    /// pipeline with one.
    pub timings: Vec<StepTiming>,
    /// The parts of a control construct, for an operation that is one.
    pub construct: Option<ConstructOutcome>,
}

/// What a control construct did, part by part.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct ConstructOutcome {
    /// Name of the construct: `when`, `each`, `steps`, `call`.
    pub name: String,
    /// Parts in order: the chosen branch, every iteration, every step.
    pub parts: Vec<nurt_contracts::report::ReportPart>,
}

impl OperationOutcome {
    pub(crate) fn new(index: u32) -> Self {
        Self {
            index,
            status: ReportStatus::Ok,
            result: ResultData::default(),
            effect: None,
            notable: Vec::new(),
            error: None,
            timings: Vec::new(),
            construct: None,
        }
    }

    pub(crate) fn fail(&mut self, error: Error) {
        self.status = ReportStatus::Failed;
        self.error = Some(error);
    }
}

/// Time spent in one execution step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StepTiming {
    /// Execution step.
    pub step: ExecStep,
    /// Time spent, on the monotonic clock.
    pub elapsed: Duration,
}

/// Records or value collected from the output of a pipeline.
#[derive(Debug, Clone, PartialEq, Default)]
#[non_exhaustive]
pub struct ResultData {
    /// Records, in output order; empty when the output is a single value.
    pub records: Vec<Record>,
    /// Single value, for example the result of `count`.
    pub value: Option<Value>,
    /// Header record of the stream.
    pub header: Option<Record>,
    /// Order of the stream declared by its producer.
    pub order: Option<StreamOrder>,
    /// Presentation hint of the producer.
    pub render: Option<RenderHint>,
}

/// Number of records of `partial_read` in the error of a failed stage.
const PARTIAL_READ_SAMPLE: usize = 10;
/// Name of the error detail with the records read before a failure.
const PARTIAL_READ: &str = "partial_read";

/// Collects a stream within `max` records. The error of a failed stage
/// carries the records read before the failure as the detail `partial_read`:
/// the number read and a sample of the first records.
fn collect_records(stream: RecordStream<'_>, max: u64) -> Result<Vec<Record>> {
    let mut records = Vec::new();
    for item in stream {
        match item {
            Ok(_) if u64::try_from(records.len()).unwrap_or(u64::MAX) >= max => {
                return Err(Error::new(
                    ErrorCode::Limit,
                    format!("the stream holds more than {max} records"),
                ));
            }
            Ok(record) => records.push(record),
            Err(error) => return Err(with_partial_read(error, &records)),
        }
    }
    Ok(records)
}

fn with_partial_read(error: Error, records: &[Record]) -> Error {
    if error.code() != ErrorCode::StageFailed || error.details.contains_key(PARTIAL_READ) {
        return error;
    }
    let sample: Vec<&Record> = records.iter().take(PARTIAL_READ_SAMPLE).collect();
    error.with_detail(
        PARTIAL_READ,
        json!({ "count": records.len(), "records": sample }),
    )
}

impl ResultData {
    /// Result consisting of records.
    #[must_use]
    pub fn from_records(records: Vec<Record>) -> Self {
        Self {
            records,
            ..Self::default()
        }
    }

    /// Result of a named value: a list of records becomes a record stream,
    /// any other value stays a single value.
    #[must_use]
    pub fn from_value(value: Value) -> Self {
        match value {
            Value::List(items) if items.iter().all(|item| matches!(item, Value::Record(_))) => {
                let records = items
                    .into_iter()
                    .filter_map(|item| match item {
                        Value::Record(record) => Some(record),
                        _ => None,
                    })
                    .collect();
                Self::from_records(records)
            }
            other => Self {
                value: Some(other),
                ..Self::default()
            },
        }
    }

    /// Value of the result as a named result (`let`): the records as a list,
    /// or the single value.
    #[must_use]
    pub fn to_value(&self) -> Value {
        match &self.value {
            Some(value) => value.clone(),
            None => Value::List(self.records.iter().cloned().map(Value::Record).collect()),
        }
    }

    /// Collects the input of a stage, holding no more than `max` records.
    ///
    /// # Errors
    ///
    /// Fails with `E_LIMIT` when the stream holds more than `max` records and
    /// with the first error item of the stream.
    pub(crate) fn collect(input: StageInput<'_>, max: u64) -> Result<Self> {
        let StageInput {
            data,
            header,
            order,
            render,
        } = input;
        let (records, value) = match data {
            StageData::Records(stream) => (collect_records(stream, max)?, None),
            StageData::Value(value) => (Vec::new(), Some(value)),
            _ => (Vec::new(), None),
        };
        Ok(Self {
            records,
            value,
            header,
            order,
            render,
        })
    }

    /// The result as the input of a following stage.
    pub(crate) fn into_input(self) -> StageInput<'static> {
        let data = match self.value {
            Some(value) => StageData::Value(value),
            None => StageData::Records(RecordStream::from_records(self.records)),
        };
        StageInput {
            data,
            header: self.header,
            order: self.order,
            render: self.render,
        }
    }

    /// The result as the output of a pipeline.
    pub(crate) fn into_output(self) -> StageOutput<'static> {
        let input = self.into_input();
        StageOutput {
            data: input.data,
            header: input.header,
            order: input.order,
            render: input.render,
        }
    }
}

/// One change of an effect with the facts the report rules need beyond the
/// `change` record.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct ChangeFacts {
    /// The change as listed in the report.
    pub change: Change,
    /// Kind of the entry concerned, from the journal; `None` when the
    /// journal did not report the target (preview, targets outside the file
    /// system).
    pub entry: Option<FileKind>,
    /// The change replaced an existing entry: a target declared as a
    /// replacement that existed before, or a change carrying the flag
    /// `overwrote_existing`.
    pub replaced_existing: bool,
    /// Number of files (links included) and of directories below a directory
    /// that the change removed or restored with its content, from the notes
    /// of the change; `None` when the change does not say.
    pub inside: Option<(u64, u64)>,
}

/// Effect of an operation.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct EffectReport {
    /// Full name of the effect command.
    pub command: String,
    /// The effect was applied and committed.
    pub applied: bool,
    /// Changes, one per target, in target order: as applied, or as planned
    /// when the effect was not applied.
    pub changes: Vec<ChangeFacts>,
    /// Unusual effects reported by the effect stage.
    pub notable: Vec<NotableEntry>,
    /// Irreversible effects reported by the effect stage, the journal and
    /// the rollback.
    pub irreversible: Vec<IrreversibleEntry>,
    /// Records shown by a preview for effects without file targets.
    pub preview: Vec<Record>,
    /// The operation has more than one target and is undone as a whole.
    pub bulk: bool,
    /// The effect continues in the background.
    pub detached: bool,
    /// Handle of the committed journal operation.
    pub handle: Option<OpHandle>,
    /// Result of the rollback after a failed verification.
    pub rollback: Option<UndoOutcome>,
}

impl EffectReport {
    pub(crate) fn planned(command: &str, plan: &EffectPlan) -> Self {
        Self {
            command: command.to_owned(),
            applied: false,
            changes: crate::effect::change_facts(plan.changes.clone(), None),
            notable: plan.notable.clone(),
            irreversible: plan.irreversible.clone(),
            preview: plan.preview.clone(),
            bulk: plan.bulk,
            detached: false,
            handle: None,
            rollback: None,
        }
    }

    pub(crate) fn add_irreversible(&mut self, entries: &[IrreversibleEntry]) {
        for entry in entries {
            if !self.irreversible.contains(entry) {
                self.irreversible.push(entry.clone());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use nurt_contracts::value::RecordKind;

    use super::*;

    #[test]
    fn named_list_of_records_becomes_a_stream() {
        let record = Record::new(RecordKind::Entry).with("size", Value::Int(1));
        let data = ResultData::from_value(Value::List(vec![Value::Record(record.clone())]));
        assert_eq!(data.records, vec![record]);
        assert!(data.value.is_none());
        assert_eq!(data.to_value().as_list().map(<[Value]>::len), Some(1));
    }

    #[test]
    fn other_values_stay_single_values() {
        let data = ResultData::from_value(Value::List(vec![Value::Int(3)]));
        assert!(data.records.is_empty());
        assert_eq!(data.to_value(), Value::List(vec![Value::Int(3)]));
        let input = data.into_input();
        assert!(matches!(input.data, StageData::Value(_)));
    }

    #[test]
    fn collect_stops_at_the_record_limit() {
        let records = vec![Record::new(RecordKind::Entry); 3];
        let input = StageInput::records(RecordStream::from_records(records));
        let error = ResultData::collect(input, 2)
            .err()
            .map(|error| error.code());
        assert_eq!(error, Some(nurt_contracts::error::ErrorCode::Limit));
    }

    #[test]
    fn empty_named_list_stays_an_empty_record_stream() {
        let data = ResultData::from_value(Value::List(Vec::new()));
        assert!(data.records.is_empty());
        assert!(data.value.is_none());
        assert_eq!(data.to_value(), Value::List(Vec::new()));
    }

    #[test]
    fn failure_of_another_code_than_a_stage_carries_no_partial_read() {
        let items = std::iter::once(Ok(Record::new(RecordKind::Entry)))
            .chain(std::iter::once(Err(Error::new(ErrorCode::Io, "broken"))));
        let input = StageInput::records(RecordStream::new(items));
        let error = ResultData::collect(input, 10).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Io);
        assert!(!error.details.contains_key(PARTIAL_READ));
    }

    #[test]
    fn failed_stage_carries_the_count_and_a_sample_of_the_records_read() {
        let items = (0..12)
            .map(|_| Ok(Record::new(RecordKind::Entry)))
            .chain(std::iter::once(Err(Error::new(
                ErrorCode::StageFailed,
                "broken",
            ))));
        let input = StageInput::records(RecordStream::new(items));
        let error = ResultData::collect(input, 100).unwrap_err();
        let partial = &error.details[PARTIAL_READ];
        assert_eq!(partial["count"], 12);
        assert_eq!(
            partial["records"].as_array().map(Vec::len),
            Some(PARTIAL_READ_SAMPLE)
        );
    }

    #[test]
    fn script_outcome_takes_status_of_last_operation() {
        let mut outcome = Outcome::new(Instant::UNIX_EPOCH);
        let mut operation = OperationOutcome::new(0);
        operation.status = ReportStatus::Planned;
        outcome.push(operation);
        assert_eq!(outcome.status, ReportStatus::Planned);
        assert!(outcome.is_ok());
        assert_eq!(outcome.last().map(|last| last.index), Some(0));
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
