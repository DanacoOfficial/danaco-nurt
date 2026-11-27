// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Calls of CIM methods: the parameters `invoke` and `arguments` of
//! `windows.cim`.
//!
//! The call is the effect of the program. It is planned on every system
//! (the arguments are validated) and runs only on Windows. A method with a
//! documented opposite is undone by the opposite method on the same
//! instances; any other method is marked irreversible.

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::journal::MutationTx;
use nurt_contracts::record::EffectStatus;
use nurt_contracts::stage::{
    BoundArgs, EffectOutcome, EffectPlan, ExecCtx, RecordStream, StageOutput,
};
use nurt_contracts::value::Value;

use super::wql::CimQuery;
use crate::admin::effects::{effect_record, marker, state_change};
use crate::admin::undo::CIM_PREFIX;

/// Name of the parameter that selects the method.
pub(super) const PARAM_INVOKE: &str = "invoke";
/// Name of the parameter with the input parameters of the method.
pub(super) const PARAM_ARGUMENTS: &str = "arguments";

/// Value of an input parameter of a method.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Argument {
    /// Text.
    Text(String),
    /// Integer.
    Int(i64),
    /// Boolean.
    Bool(bool),
}

/// A validated call of a method.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Invocation {
    /// Name of the method.
    pub(super) method: String,
    /// Input parameters in the order given.
    pub(super) arguments: Vec<(String, Argument)>,
}

/// Result of one call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Called {
    /// Instance path, or the class name for a static method.
    pub(super) target: String,
    /// `ReturnValue` of the method when it returned one.
    pub(super) return_value: Option<i64>,
}

fn is_identifier(text: &str) -> bool {
    let mut chars = text.chars();
    chars
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
        && text.len() <= 128
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn invalid(name: &str, text: &str, expected: &str) -> Error {
    Error::new(
        ErrorCode::InvalidValue,
        format!("`{text}` is not valid for `{name}`: {expected}"),
    )
    .with_found(text)
}

/// The call requested by `invoke` and `arguments`, `None` for a query.
///
/// # Errors
///
/// `E_INVALID_VALUE` for a name that is not an identifier, `E_TYPE` for an
/// argument that is not text, int or bool, and `E_MISSING_PARAM` for
/// `arguments` without `invoke`.
pub(super) fn invocation_from_args(args: &BoundArgs) -> Result<Option<Invocation>> {
    let method = match args.value(PARAM_INVOKE) {
        None | Some(Value::Null) => None,
        Some(Value::Text(text)) => Some(text.clone()),
        Some(_) => return args.text(PARAM_INVOKE).map(|_| None),
    };
    let given = !matches!(args.value(PARAM_ARGUMENTS), None | Some(Value::Null));
    let Some(method) = method else {
        if given {
            return Err(Error::new(
                ErrorCode::MissingParam,
                "`windows.cim` uses `arguments` only with `invoke`",
            ));
        }
        return Ok(None);
    };
    if !is_identifier(&method) {
        return Err(invalid(PARAM_INVOKE, &method, "a method name"));
    }
    let mut arguments = Vec::new();
    if let Some(Value::Map(fields)) = args.value(PARAM_ARGUMENTS) {
        for (name, value) in fields {
            if !is_identifier(name) {
                return Err(invalid(PARAM_ARGUMENTS, name, "a parameter name"));
            }
            let argument = match value {
                Value::Text(text) => Argument::Text(text.clone()),
                Value::Int(number) => Argument::Int(*number),
                Value::Bool(flag) => Argument::Bool(*flag),
                other => {
                    return Err(Error::new(
                        ErrorCode::Type,
                        format!(
                            "`windows.cim` expects text, int or bool for `{name}`, not {}",
                            other.type_name()
                        ),
                    ));
                }
            };
            arguments.push((name.clone(), argument));
        }
    } else if given {
        return args.text(PARAM_ARGUMENTS).map(|_| None);
    }
    Ok(Some(Invocation { method, arguments }))
}

/// Target of the change: `cim:<class>.<method>`.
pub(super) fn target_of(query: &CimQuery, invocation: &Invocation) -> String {
    format!("{CIM_PREFIX}{}.{}", query.class, invocation.method)
}

/// State recorded after the call: the namespace and the condition that
/// selected the instances.
pub(super) fn state_of(query: &CimQuery) -> String {
    format!(
        "namespace={};where={}",
        query.namespace,
        query.condition.as_deref().unwrap_or_default()
    )
}

/// Plans the call.
///
/// # Errors
///
/// The error of the count of the instances on Windows.
pub(super) fn plan(query: &CimQuery, invocation: &Invocation, matched: Option<u64>) -> EffectPlan {
    let target = target_of(query, invocation);
    let state = state_of(query);
    let mut plan = EffectPlan::new();
    plan.changes.push(state_change(&target, None, Some(&state)));
    plan.irreversible.push(marker("windows.cim", &target));
    let mut record = effect_record(
        "windows.cim",
        &target,
        EffectStatus::Planned,
        None,
        Some(&state),
    );
    if let Some(count) = matched {
        record.set(
            "targets",
            Value::Int(i64::try_from(count).unwrap_or(i64::MAX)),
        );
    }
    plan.preview.push(record);
    plan
}

/// Runs the call through `call` and builds the outcome.
///
/// # Errors
///
/// The error of `call`, and `E_STAGE_FAILED` with the `ReturnValue` when a
/// method returns a non-zero code.
pub(super) fn apply<'a>(
    query: &CimQuery,
    invocation: &Invocation,
    tx: &mut dyn MutationTx,
    ctx: &ExecCtx,
    call: impl FnOnce(&CimQuery, &Invocation, &ExecCtx) -> Result<Vec<Called>>,
) -> Result<EffectOutcome<'a>> {
    let target = target_of(query, invocation);
    let state = state_of(query);
    let entry = marker("windows.cim", &target);
    tx.mark_irreversible(entry.clone());
    let called = call(query, invocation, ctx).map_err(|error| error.with_effect_applied(false))?;
    let mut outcome = EffectOutcome::new(StageOutput::none());
    outcome
        .changes
        .push(state_change(&target, None, Some(&state)));
    outcome.irreversible.push(entry);
    let mut records = Vec::new();
    let mut failure = None;
    for item in &called {
        let status = if item.return_value.is_none_or(|code| code == 0) {
            EffectStatus::Ok
        } else {
            EffectStatus::Failed
        };
        let mut record = effect_record("windows.cim", &item.target, status, None, Some(&state));
        if let Some(code) = item.return_value {
            record.set("return_value", Value::Int(code));
            if code != 0 && failure.is_none() {
                failure = Some((item.target.clone(), code));
            }
        }
        records.push(record);
    }
    if let Some((instance, code)) = failure {
        return Err(Error::new(
            ErrorCode::StageFailed,
            format!(
                "`{}.{}` on `{instance}` returned {code}",
                query.class, invocation.method
            ),
        )
        .with_detail("return_value", code.into())
        .with_effect_applied(true));
    }
    outcome.output = StageOutput::records(RecordStream::from_records(records));
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nurt_contracts::stage::ArgValue;

    fn query(condition: Option<&str>) -> CimQuery {
        CimQuery::new("Win32_Service", condition, "root/cimv2", &[]).unwrap()
    }

    fn args(values: &[(&str, Value)]) -> BoundArgs {
        values
            .iter()
            .fold(BoundArgs::new("windows.cim"), |args, (name, value)| {
                args.with(*name, ArgValue::Value(value.clone()), false)
            })
    }

    #[test]
    fn a_query_without_invoke_is_not_a_call() {
        assert_eq!(invocation_from_args(&args(&[])).unwrap(), None);
    }

    #[test]
    fn invoke_collects_typed_arguments() {
        let mut fields = nurt_contracts::value::Fields::new();
        fields.insert("CommandLine".to_owned(), Value::Text("notepad".to_owned()));
        fields.insert("Flags".to_owned(), Value::Int(3));
        fields.insert("Wait".to_owned(), Value::Bool(true));
        let given = args(&[
            ("invoke", Value::Text("Create".to_owned())),
            ("arguments", Value::Map(fields)),
        ]);
        let call = invocation_from_args(&given).unwrap().unwrap();
        assert_eq!(call.method, "Create");
        assert_eq!(
            call.arguments,
            [
                (
                    "CommandLine".to_owned(),
                    Argument::Text("notepad".to_owned())
                ),
                ("Flags".to_owned(), Argument::Int(3)),
                ("Wait".to_owned(), Argument::Bool(true)),
            ]
        );
    }

    #[test]
    fn invalid_calls_are_rejected() {
        let method = |name: &str| args(&[("invoke", Value::Text(name.to_owned()))]);
        assert!(invocation_from_args(&method("Stop Service")).is_err());
        assert!(invocation_from_args(&method("")).is_err());
        assert!(
            invocation_from_args(&args(&[(
                "arguments",
                Value::Map(nurt_contracts::value::Fields::new())
            )]))
            .is_err()
        );
        let mut fields = nurt_contracts::value::Fields::new();
        fields.insert("X".to_owned(), Value::List(Vec::new()));
        let bad = args(&[
            ("invoke", Value::Text("M".to_owned())),
            ("arguments", Value::Map(fields)),
        ]);
        assert_eq!(
            invocation_from_args(&bad).unwrap_err().code(),
            ErrorCode::Type
        );
    }

    #[test]
    fn the_plan_names_the_target_and_marks_the_effect() {
        let call = Invocation {
            method: "StopService".to_owned(),
            arguments: Vec::new(),
        };
        let planned = plan(&query(Some("Name='Spooler'")), &call, Some(1));
        assert_eq!(
            planned.changes[0].path.as_str(),
            "cim:Win32_Service.StopService"
        );
        assert_eq!(
            planned.changes[0].after.as_deref(),
            Some("namespace=root/cimv2;where=Name='Spooler'")
        );
        assert_eq!(planned.irreversible.len(), 1);
        assert_eq!(planned.preview.len(), 1);
    }
}
