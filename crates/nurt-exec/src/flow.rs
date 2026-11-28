// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-exec::flow
// Responsibility:   Stream plumbing shared by the executor and the pipeline runner.
// Dependencies:     nurt_catalog, nurt_contracts, nurt_testkit, serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Stream plumbing shared by the executor and the pipeline runner:
//! resolution of implementations, error positions and record kind checks.

use std::sync::Arc;

use nurt_catalog::Registry;
use nurt_contracts::bound::Phase;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::stage::{
    BoundArgs, EffectStage, ExecCtx, ExecStep, InputKinds, ReadStage, RecordStream, StageData,
    StageImpl, StageInput, StageOutput,
};
use nurt_contracts::value::{Record, RecordKind};
use serde_json::json;

use crate::outcome::ResultData;

/// Sets the step of an error that has none.
pub(crate) fn at_step(error: Error, step: ExecStep) -> Error {
    if error.stage().is_some() {
        error
    } else {
        error.with_stage(step)
    }
}

/// Error for a state the executor never produces for valid plans.
pub(crate) fn internal(message: impl Into<String>) -> Error {
    Error::new(ErrorCode::Internal, message)
}

/// First execution step of a stage of the given phase.
pub(crate) fn step_of(phase: Phase) -> ExecStep {
    match phase {
        Phase::Check => ExecStep::Check,
        Phase::Mutate | Phase::Run => ExecStep::Plan,
        Phase::After => ExecStep::After,
        Phase::Verify => ExecStep::Verify,
        _ => ExecStep::Read,
    }
}

/// The read interface of an implementation.
pub(crate) fn read_interface<'a>(
    implementation: &'a Arc<dyn StageImpl>,
    command: &str,
) -> Result<&'a dyn ReadStage> {
    implementation
        .as_read()
        .ok_or_else(|| internal(format!("`{command}` has no read implementation")))
}

/// The effect interface of an implementation.
pub(crate) fn effect_interface<'a>(
    implementation: &'a Arc<dyn StageImpl>,
    command: &str,
) -> Result<&'a dyn EffectStage> {
    implementation
        .as_effect()
        .ok_or_else(|| internal(format!("`{command}` has no effect implementation")))
}

/// Implementations selected for the input of a stage.
pub(crate) enum Selection<'a> {
    /// One implementation serves the whole input.
    One(Arc<dyn StageImpl>, StageInput<'a>),
    /// The input holds records of kinds served by different implementations.
    Split(Split),
}

/// Records of one input split by the implementation that serves their kind.
pub(crate) struct Split {
    groups: Vec<Group>,
    header: Option<Record>,
}

struct Group {
    implementation: Arc<dyn StageImpl>,
    records: Vec<Record>,
}

impl Split {
    /// The implementations in order of the first record of their kinds, each
    /// with the records it serves.
    pub(crate) fn into_inputs(self) -> Vec<(Arc<dyn StageImpl>, StageInput<'static>)> {
        let Self { groups, header } = self;
        groups
            .into_iter()
            .map(|group| {
                let input = StageInput {
                    data: StageData::Records(RecordStream::from_records(group.records)),
                    header: header.clone(),
                    order: None,
                    render: None,
                };
                (group.implementation, input)
            })
            .collect()
    }
}

/// Whether more than one implementation of `command` is registered, so that
/// the implementation depends on the kind of the record.
fn is_polymorphic(registry: &Registry, command: &str) -> bool {
    registry.implementations(command).nth(1).is_some()
}

/// Selects the implementation of `command` for its input.
///
/// A stage without input uses the source implementation. For a record
/// stream the first record is read to learn its kind and put back in front
/// of the stream; an empty stream or a single value uses the first
/// registered implementation of the command.
///
/// A command with several implementations reads its input (at most `limit`
/// records when the rest of the pipeline needs no more, and within `max`
/// records) and splits it by the implementation that serves the kind of each
/// record; the groups keep the order of the first record of their kinds and
/// the order of records within a group.
pub(crate) fn resolve<'a>(
    registry: &Registry,
    command: &str,
    input: StageInput<'a>,
    limit: Option<u64>,
    max: u64,
) -> Result<Selection<'a>> {
    let StageInput {
        data,
        header,
        order,
        render,
    } = input;
    let (implementation, data) = match data {
        StageData::None => (registry.resolve_source(command)?, StageData::None),
        StageData::Records(stream) if is_polymorphic(registry, command) => {
            let stream = match limit.map(usize::try_from) {
                Some(Ok(count)) => RecordStream::new(stream.take(count)),
                _ => stream,
            };
            let mut groups =
                split_by_implementation(registry, command, stream.collect_bounded(max)?)?;
            if groups.len() > 1 {
                return Ok(Selection::Split(Split { groups, header }));
            }
            return Ok(match groups.pop() {
                Some(group) => Selection::One(
                    group.implementation,
                    StageInput {
                        data: StageData::Records(RecordStream::from_records(group.records)),
                        header,
                        order,
                        render,
                    },
                ),
                None => Selection::One(
                    resolve_any(registry, command)?,
                    StageInput {
                        data: StageData::Records(RecordStream::empty()),
                        header,
                        order,
                        render,
                    },
                ),
            });
        }
        StageData::Records(mut stream) => match stream.next() {
            Some(Ok(first)) => {
                let found = registry.resolve(command, first.kind)?;
                let stream = RecordStream::new(std::iter::once(Ok(first)).chain(stream));
                (found, StageData::Records(stream))
            }
            Some(Err(error)) => return Err(error),
            None => (
                resolve_any(registry, command)?,
                StageData::Records(RecordStream::empty()),
            ),
        },
        other => (resolve_any(registry, command)?, other),
    };
    Ok(Selection::One(
        implementation,
        StageInput {
            data,
            header,
            order,
            render,
        },
    ))
}

fn split_by_implementation(
    registry: &Registry,
    command: &str,
    records: Vec<Record>,
) -> Result<Vec<Group>> {
    let mut groups: Vec<Group> = Vec::new();
    let mut slots: Vec<(RecordKind, usize)> = Vec::new();
    for record in records {
        let known = slots.iter().find(|(kind, _)| *kind == record.kind);
        let slot = match known {
            Some((_, slot)) => *slot,
            None => {
                let found = registry.resolve(command, record.kind)?;
                let slot = match groups
                    .iter()
                    .position(|group| Arc::ptr_eq(&group.implementation, &found))
                {
                    Some(slot) => slot,
                    None => {
                        groups.push(Group {
                            implementation: found,
                            records: Vec::new(),
                        });
                        groups.len() - 1
                    }
                };
                slots.push((record.kind, slot));
                slot
            }
        };
        groups[slot].records.push(record);
    }
    Ok(groups)
}

/// Runs the read stage of every group on its records and joins the outputs
/// in the order of the groups.
///
/// Every group output is collected within `max` records. A stage that
/// answers with a single value cannot be joined with the output of another
/// implementation.
pub(crate) fn execute_split(
    split: Split,
    command: &str,
    args: &BoundArgs,
    ctx: &ExecCtx,
    max: u64,
) -> Result<StageOutput<'static>> {
    let mut records = Vec::new();
    let mut header = None;
    for (implementation, input) in split.into_inputs() {
        let read = read_interface(&implementation, command)?;
        let output = read.execute(args, input, ctx)?;
        let collected = ResultData::collect(output.into(), max)?;
        if collected.value.is_some() {
            return Err(Error::new(
                ErrorCode::Type,
                format!(
                    "`{command}` answers with a single value for one kind of records; split the pipeline by record kind"
                ),
            ));
        }
        header = header.or(collected.header);
        records.extend(collected.records);
    }
    let mut output = StageOutput::records(RecordStream::from_records(records));
    output.header = header;
    Ok(output)
}

fn resolve_any(registry: &Registry, command: &str) -> Result<Arc<dyn StageImpl>> {
    let kind = registry
        .implementations(command)
        .find_map(|descriptor| match &descriptor.input {
            InputKinds::Any => Some(RecordKind::Entry),
            InputKinds::Only(kinds) => kinds.first().copied(),
            _ => None,
        });
    match kind {
        Some(kind) => registry.resolve(command, kind),
        None => registry.resolve_source(command),
    }
}

/// Fails the stream at the first record of a kind the selected
/// implementation does not accept.
///
/// The implementation is selected by the kind of the first record; a
/// stream that mixes kinds served by different implementations has to be
/// split by kind in the program.
pub(crate) fn guard_kinds<'a>(
    input: StageInput<'a>,
    command: &str,
    accepted: &InputKinds,
) -> StageInput<'a> {
    let InputKinds::Only(kinds) = accepted else {
        return input;
    };
    let StageInput {
        data,
        header,
        order,
        render,
    } = input;
    let data = match data {
        StageData::Records(stream) => {
            let kinds = kinds.clone();
            let command = command.to_owned();
            StageData::Records(RecordStream::new(stream.map(move |item| {
                item.and_then(|record| check_kind(record, &command, &kinds))
            })))
        }
        other => other,
    };
    StageInput {
        data,
        header,
        order,
        render,
    }
}

fn check_kind(record: Record, command: &str, kinds: &[RecordKind]) -> Result<Record> {
    if kinds.contains(&record.kind) {
        return Ok(record);
    }
    let expected: Vec<&str> = kinds.iter().map(|kind| kind.as_str()).collect();
    Err(Error::new(
        ErrorCode::Type,
        format!(
            "`{command}` does not accept `{}` records in a stream of other kinds; split the pipeline by record kind",
            record.kind.as_str()
        ),
    )
    .with_expected(json!(expected))
    .with_found(record.kind.as_str()))
}

/// The output of a stage as the input of the next one: every pull first
/// checks cancellation and the deadline, and errors without a step get the
/// step of the stage.
pub(crate) fn tag_output<'a>(
    output: StageOutput<'a>,
    step: ExecStep,
    ctx: &'a ExecCtx,
) -> StageInput<'a> {
    let StageOutput {
        data,
        header,
        order,
        render,
    } = output;
    let data = match data {
        StageData::Records(stream) => StageData::Records(RecordStream::new(
            stream
                .checked(ctx)
                .map(move |item| item.map_err(|error| at_step(error, step))),
        )),
        other => other,
    };
    StageInput {
        data,
        header,
        order,
        render,
    }
}

#[cfg(test)]
mod tests {
    use nurt_contracts::stage::{CancelReason, CancelToken};
    use nurt_contracts::value::Value;
    use nurt_testkit::clock::VirtualClock;
    use nurt_testkit::host::TestHost;

    use super::*;

    fn records(kinds: &[RecordKind]) -> StageInput<'static> {
        StageInput::records(RecordStream::from_records(
            kinds.iter().map(|kind| Record::new(*kind)).collect(),
        ))
    }

    fn codes(input: StageInput<'_>) -> Vec<std::result::Result<RecordKind, ErrorCode>> {
        match input.data {
            StageData::Records(stream) => stream
                .map(|item| item.map(|record| record.kind).map_err(|error| error.code()))
                .collect(),
            _ => Vec::new(),
        }
    }

    #[test]
    fn errors_keep_an_existing_step() {
        let error = Error::new(ErrorCode::Assert, "no").with_stage(ExecStep::Check);
        assert_eq!(
            at_step(error, ExecStep::After).stage(),
            Some(ExecStep::Check)
        );
        let error = Error::new(ErrorCode::Io, "no");
        assert_eq!(
            at_step(error, ExecStep::Apply).stage(),
            Some(ExecStep::Apply)
        );
    }

    #[test]
    fn phases_map_to_their_first_step() {
        assert_eq!(step_of(Phase::Source), ExecStep::Read);
        assert_eq!(step_of(Phase::Transform), ExecStep::Read);
        assert_eq!(step_of(Phase::Check), ExecStep::Check);
        assert_eq!(step_of(Phase::Run), ExecStep::Plan);
        assert_eq!(step_of(Phase::Mutate), ExecStep::Plan);
        assert_eq!(step_of(Phase::After), ExecStep::After);
        assert_eq!(step_of(Phase::Verify), ExecStep::Verify);
    }

    #[test]
    fn kind_guard_stops_at_a_foreign_record() {
        let guarded = guard_kinds(
            records(&[RecordKind::Entry, RecordKind::Exec, RecordKind::Entry]),
            "remove",
            &InputKinds::Only(vec![RecordKind::Entry]),
        );
        assert_eq!(
            codes(guarded),
            [
                Ok(RecordKind::Entry),
                Err(ErrorCode::Type),
                Ok(RecordKind::Entry)
            ]
        );
        let open = guard_kinds(records(&[RecordKind::Exec]), "count", &InputKinds::Any);
        assert_eq!(codes(open), [Ok(RecordKind::Exec)]);
        let value = guard_kinds(
            StageInput::value(Value::Int(1)),
            "count",
            &InputKinds::Only(vec![RecordKind::Entry]),
        );
        assert!(matches!(value.data, StageData::Value(_)));
    }

    #[test]
    fn tagged_output_checks_cancellation_and_sets_the_step() {
        let cancel = CancelToken::new();
        let ctx = ExecCtx::builder(Arc::new(VirtualClock::new()), Arc::new(TestHost::linux()))
            .cancel(cancel.clone())
            .build();
        let output = StageOutput::records(RecordStream::from_records(vec![
            Record::new(RecordKind::Entry),
            Record::new(RecordKind::Entry),
        ]));
        let StageData::Records(mut stream) = tag_output(output, ExecStep::Read, &ctx).data else {
            panic!("records expected");
        };
        assert!(matches!(stream.next(), Some(Ok(_))));
        cancel.cancel(CancelReason::Requested);
        let error = stream.next().and_then(Result::err).unwrap();
        assert_eq!(error.code(), ErrorCode::Cancelled);
        assert_eq!(error.stage(), Some(ExecStep::Read));
        assert!(stream.next().is_none());
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
