// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Implementations of the commands `help`, `describe` and `evaluate`.

use std::collections::HashMap;
use std::sync::Arc;

use nurt_contracts::config::Language as ConfigLanguage;
use nurt_contracts::envelope::{Language, OutputFormat};
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::record::{Evaluation, FieldInfo};
use nurt_contracts::stage::{
    BoundArgs, ExecCtx, InputKinds, ReadStage, RecordStream, StageDescriptor, StageImpl,
    StageInput, StageOutput,
};
use nurt_contracts::value::{Record, Value};

use crate::catalog::Catalog;
use crate::help::{self, HelpFormat, HelpQuery, TopicKind};

/// Name of the `help` command.
pub const HELP_COMMAND: &str = "help";

/// Name of the `describe` command.
pub const DESCRIBE_COMMAND: &str = "describe";

/// Name of the `evaluate` command.
pub const EVALUATE_COMMAND: &str = "evaluate";

/// Type shown for a field whose values have different types.
const MIXED_TYPE: &str = "mixed";

/// Type shown for a field that is `null` in every record.
const NULL_TYPE: &str = "null";

/// The `help` command: a source that returns the answer as text.
#[derive(Debug, Clone)]
pub struct HelpStage {
    catalog: Option<Arc<Catalog>>,
}

impl HelpStage {
    /// Creates the command over a catalog.
    #[must_use]
    pub fn new(catalog: Arc<Catalog>) -> Self {
        Self {
            catalog: Some(catalog),
        }
    }

    /// Creates the command over the catalog and the documentation compiled
    /// into the executable, which are loaded when the command first runs.
    #[must_use]
    pub fn embedded() -> Self {
        Self { catalog: None }
    }

    fn catalog(&self) -> Result<Arc<Catalog>> {
        match &self.catalog {
            Some(catalog) => Ok(Arc::clone(catalog)),
            None => Catalog::embedded().map_err(|error| {
                Error::new(ErrorCode::Internal, format!("embedded help: {error}"))
            }),
        }
    }
}

fn query_from(args: &BoundArgs, ctx: &ExecCtx) -> Result<HelpQuery> {
    let owned = |name: &str| -> Result<Option<String>> { Ok(args.text(name)?.map(str::to_owned)) };
    let kind = if args.is_defaulted("kind") {
        None
    } else {
        args.value("kind")
            .and_then(Value::as_str)
            .and_then(TopicKind::from_name)
    };
    let examples = args
        .int("examples")?
        .map_or(help::DEFAULT_EXAMPLES, |count| {
            usize::try_from(count).unwrap_or_default()
        });
    let options = ctx.options();
    Ok(HelpQuery {
        topic: owned("topic")?,
        kind,
        group: owned("group")?,
        skill: owned("skill")?,
        search: owned("search")?,
        examples,
        format: if options.format == Some(OutputFormat::Json) {
            HelpFormat::Json
        } else {
            HelpFormat::Text
        },
        language: options
            .language
            .unwrap_or(match ctx.config().output.language {
                ConfigLanguage::Pl => Language::Pl,
                _ => Language::En,
            }),
        variant: ctx.config().syntax.variant,
    })
}

impl StageImpl for HelpStage {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: HELP_COMMAND.to_owned(),
            source: true,
            input: InputKinds::None,
        }
    }

    fn as_read(&self) -> Option<&dyn ReadStage> {
        Some(self)
    }
}

impl ReadStage for HelpStage {
    fn execute<'a>(
        &'a self,
        args: &'a BoundArgs,
        _input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<StageOutput<'a>> {
        let query = query_from(args, ctx)?;
        let answer = help::render(&query, &self.catalog()?)?;
        Ok(StageOutput::value(Value::Text(answer.body)))
    }
}

/// The `describe` command: lists the fields of the input records.
#[derive(Debug, Clone, Copy, Default)]
pub struct DescribeStage;

impl DescribeStage {
    /// Creates the command.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

struct FieldSurvey {
    kind: String,
    example: Option<Value>,
    records: u64,
}

#[derive(Default)]
struct Survey {
    order: Vec<String>,
    fields: HashMap<String, FieldSurvey>,
    total: u64,
}

impl Survey {
    fn observe(&mut self, record: &Record) {
        self.total += 1;
        for (name, value) in &record.fields {
            let kind = value.type_name();
            match self.fields.get_mut(name) {
                Some(field) => {
                    field.records += 1;
                    if value.is_null() {
                        continue;
                    }
                    if field.kind == NULL_TYPE {
                        field.kind = kind.to_owned();
                        field.example = Some(value.clone());
                    } else if field.kind != kind {
                        field.kind = MIXED_TYPE.to_owned();
                    }
                }
                None => {
                    self.order.push(name.clone());
                    self.fields.insert(
                        name.clone(),
                        FieldSurvey {
                            kind: kind.to_owned(),
                            example: (!value.is_null()).then(|| value.clone()),
                            records: 1,
                        },
                    );
                }
            }
        }
    }

    fn into_records(self) -> Vec<Record> {
        let Self {
            order,
            mut fields,
            total,
        } = self;
        order
            .into_iter()
            .filter_map(|name| {
                let field = fields.remove(&name)?;
                Some(Record::from(FieldInfo {
                    field: name,
                    kind: field.kind,
                    example: field.example,
                    required: Some(field.records == total),
                    records: Some(field.records),
                }))
            })
            .collect()
    }
}

impl StageImpl for DescribeStage {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: DESCRIBE_COMMAND.to_owned(),
            source: false,
            input: InputKinds::Any,
        }
    }

    fn as_read(&self) -> Option<&dyn ReadStage> {
        Some(self)
    }
}

impl ReadStage for DescribeStage {
    fn execute<'a>(
        &'a self,
        _args: &'a BoundArgs,
        input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<StageOutput<'a>> {
        let mut survey = Survey::default();
        for item in input.into_records()?.checked(ctx) {
            survey.observe(&item?);
        }
        Ok(StageOutput::records(RecordStream::from_records(
            survey.into_records(),
        )))
    }
}

/// The `evaluate` command: computes the value of an expression.
///
/// The value comes from the evaluator of the execution context, the single
/// place where expressions are evaluated.
#[derive(Debug, Clone, Copy, Default)]
pub struct EvaluateStage;

impl EvaluateStage {
    /// Creates the command.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl StageImpl for EvaluateStage {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: EVALUATE_COMMAND.to_owned(),
            source: true,
            input: InputKinds::None,
        }
    }

    fn as_read(&self) -> Option<&dyn ReadStage> {
        Some(self)
    }
}

impl ReadStage for EvaluateStage {
    fn execute<'a>(
        &'a self,
        args: &'a BoundArgs,
        _input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<StageOutput<'a>> {
        let value = args.eval("expression", None, ctx)?.ok_or_else(|| {
            Error::new(
                ErrorCode::MissingParam,
                "evaluate needs an expression; give it as the first argument",
            )
        })?;
        let kind = value.type_name().to_owned();
        let record = Record::from(Evaluation { value, kind });
        Ok(StageOutput::records(RecordStream::once(record)))
    }
}
