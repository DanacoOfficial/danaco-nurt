// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The commands `evaluate`, `help` and `describe`.
//!
//! The three commands need components that the command crates do not own:
//! the expression evaluator of the execution, the help catalog and the
//! stream of records of the whole pipeline. They are implemented here and
//! registered by [`register`].

use std::sync::Arc;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::record::{Evaluation, FieldInfo};
use nurt_contracts::stage::{
    BoundArgs, CheckCtx, ExecCtx, InputKinds, ReadStage, RecordStream, StageData, StageDescriptor,
    StageImpl, StageInput, StageOutput, StageRegistrar, StaticInfo,
};
use nurt_contracts::value::{Record, Value};
use nurt_help::commands::HelpStage;

const EXPRESSION: &str = "expression";

fn source(command: &str) -> StageDescriptor {
    StageDescriptor {
        command: command.to_owned(),
        source: true,
        input: InputKinds::None,
    }
}

fn output<'a>(records: Vec<Record>) -> StageOutput<'a> {
    StageOutput {
        data: StageData::Records(RecordStream::from_records(records)),
        header: None,
        order: None,
        render: None,
    }
}

/// `evaluate expression`: the value of one expression with its type.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Evaluate;

impl StageImpl for Evaluate {
    fn descriptor(&self) -> StageDescriptor {
        source("evaluate")
    }

    fn check(&self, args: &BoundArgs, _ctx: &CheckCtx<'_>) -> Result<StaticInfo> {
        if args.expr(EXPRESSION).is_none() && args.value(EXPRESSION).is_none() {
            return Err(Error::new(
                ErrorCode::MissingParam,
                "`evaluate` needs a value for `expression`",
            ));
        }
        Ok(StaticInfo::default())
    }

    fn as_read(&self) -> Option<&dyn ReadStage> {
        Some(self)
    }
}

impl ReadStage for Evaluate {
    fn execute<'a>(
        &'a self,
        args: &'a BoundArgs,
        _input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<StageOutput<'a>> {
        let value = args.eval(EXPRESSION, None, ctx)?.ok_or_else(|| {
            Error::new(
                ErrorCode::MissingParam,
                "`evaluate` needs a value for `expression`",
            )
        })?;
        let kind = value.type_name().to_owned();
        Ok(output(vec![Record::from(Evaluation { value, kind })]))
    }
}

/// `describe`: the fields of the input records with their types and an
/// example value.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Describe;

struct Seen {
    kind: &'static str,
    example: Option<Value>,
    records: u64,
}

impl StageImpl for Describe {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: "describe".to_owned(),
            source: false,
            input: InputKinds::Any,
        }
    }

    fn as_read(&self) -> Option<&dyn ReadStage> {
        Some(self)
    }
}

impl ReadStage for Describe {
    fn execute<'a>(
        &'a self,
        _args: &'a BoundArgs,
        input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<StageOutput<'a>> {
        let records = input
            .into_records()?
            .collect_bounded(ctx.limits().max_records)?;
        let total = u64::try_from(records.len()).unwrap_or(u64::MAX);
        let mut fields: Vec<(String, Seen)> = Vec::new();
        for record in &records {
            for (name, value) in &record.fields {
                let position = match fields.iter().position(|(known, _)| known == name) {
                    Some(position) => position,
                    None => {
                        fields.push((
                            name.clone(),
                            Seen {
                                kind: "null",
                                example: None,
                                records: 0,
                            },
                        ));
                        fields.len() - 1
                    }
                };
                let seen = &mut fields[position].1;
                seen.records += 1;
                if seen.example.is_none() && !matches!(value, Value::Null) {
                    seen.kind = value.type_name();
                    seen.example = Some(value.clone());
                }
            }
        }
        let rows = fields
            .into_iter()
            .map(|(field, seen)| {
                Record::from(FieldInfo {
                    field,
                    kind: seen.kind.to_owned(),
                    example: seen.example,
                    required: Some(seen.records == total),
                    records: Some(seen.records),
                })
            })
            .collect();
        Ok(output(rows))
    }
}

/// Registers `evaluate`, `help` and `describe`.
///
/// # Errors
///
/// Returns the error of the registrar.
pub(crate) fn register(registrar: &mut dyn StageRegistrar) -> Result<()> {
    registrar.register(Arc::new(Evaluate))?;
    registrar.register(Arc::new(HelpStage::embedded()))?;
    registrar.register(Arc::new(Describe))
}

#[cfg(test)]
mod tests {
    use nurt_contracts::envelope::Language;
    use nurt_contracts::value::RecordKind;
    use nurt_testkit::ctx::TestContext;

    use super::*;

    fn described(input: Vec<Record>) -> Vec<FieldInfo> {
        let ctx = TestContext::new().ctx();
        let args = BoundArgs::new("describe");
        let stage = Describe;
        let output = stage
            .execute(
                &args,
                StageInput::records(RecordStream::from_records(input)),
                &ctx,
            )
            .unwrap();
        let StageData::Records(stream) = output.data else {
            panic!("records expected");
        };
        stream
            .map(|item| FieldInfo::try_from(item.unwrap()).unwrap())
            .collect()
    }

    fn map(fields: &[(&str, Value)]) -> Record {
        fields
            .iter()
            .fold(Record::new(RecordKind::Map), |record, (name, value)| {
                record.with(*name, value.clone())
            })
    }

    #[test]
    fn describe_lists_the_fields_in_the_order_of_their_first_appearance() {
        let rows = described(vec![
            map(&[("size", Value::Int(3)), ("name", Value::from("ą"))]),
            map(&[("owner", Value::from("ż")), ("size", Value::Int(4))]),
        ]);
        let names: Vec<&str> = rows.iter().map(|row| row.field.as_str()).collect();
        assert_eq!(names, ["size", "name", "owner"]);
        assert_eq!(rows[0].kind, "int");
        assert_eq!(rows[1].kind, "text");
    }

    #[test]
    fn describe_marks_a_field_required_only_when_every_record_has_it() {
        let rows = described(vec![
            map(&[("a", Value::Int(1)), ("b", Value::Int(2))]),
            map(&[("a", Value::Int(3))]),
        ]);
        assert_eq!(rows[0].required, Some(true));
        assert_eq!(rows[0].records, Some(2));
        assert_eq!(rows[1].required, Some(false));
        assert_eq!(rows[1].records, Some(1));
    }

    #[test]
    fn describe_takes_the_type_and_example_from_the_first_value_other_than_null() {
        let rows = described(vec![
            map(&[("a", Value::Null)]),
            map(&[("a", Value::from("x"))]),
            map(&[("a", Value::Int(5))]),
        ]);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].kind, "text");
        assert_eq!(rows[0].example, Some(Value::from("x")));
        assert_eq!(rows[0].records, Some(3));
    }

    #[test]
    fn describe_of_an_empty_stream_lists_nothing() {
        assert!(described(Vec::new()).is_empty());
    }

    fn help_text(language: Option<Language>, topic: Option<&str>) -> String {
        let context = TestContext::new();
        let options = nurt_contracts::envelope::RequestOptions {
            language,
            ..Default::default()
        };
        let ctx = context.builder().options(Arc::new(options)).build();
        let mut args = BoundArgs::new("help");
        if let Some(topic) = topic {
            args.insert(
                "topic",
                nurt_contracts::stage::ArgValue::Value(Value::from(topic)),
                false,
            );
        }
        let stage = HelpStage::embedded();
        let output = stage.execute(&args, StageInput::none(), &ctx).unwrap();
        let StageData::Value(Value::Text(text)) = output.data else {
            panic!("text expected");
        };
        text
    }

    #[test]
    fn help_uses_the_language_of_the_request() {
        let polish = help_text(Some(Language::Pl), None);
        assert!(polish.contains("Program jest potokiem"));
        let english = help_text(Some(Language::En), None);
        assert!(english.contains("A program is a pipeline"));
        assert!(help_text(None, None).contains("A program is a pipeline"));
    }

    #[test]
    fn help_lists_the_commands_of_every_group() {
        let text = help_text(None, Some("commands"));
        assert!(text.contains("help — "));
        assert!(text.contains("git.log — "));
        assert!(text.contains("program.catalog — "));
    }

    #[test]
    fn help_shows_the_examples_and_the_guides_of_the_embedded_documentation() {
        let page = help_text(None, Some("replace"));
        assert!(page.contains("examples:"), "{page}");
        let guide = help_text(None, Some("bash-migration"));
        assert!(guide.contains("bash"));
    }

    #[test]
    fn help_does_not_mistake_a_topic_for_the_default_kind() {
        assert!(help_text(None, Some("syntax")).contains("program ="));
        assert!(help_text(None, Some("E_ASSERT")).contains("E_ASSERT"));
    }

    #[test]
    fn describe_of_a_single_value_is_a_type_error() {
        let ctx = TestContext::new().ctx();
        let args = BoundArgs::new("describe");
        let stage = Describe;
        let error = stage
            .execute(&args, StageInput::value(Value::Int(1)), &ctx)
            .err()
            .unwrap();
        assert_eq!(error.code, ErrorCode::Type);
    }
}
