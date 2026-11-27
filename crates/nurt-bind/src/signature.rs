// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Binding of one stage to its command signature.
//!
//! Positional arguments are named in the order of the form, every parameter
//! of the signature appears once in signature order, a parameter that was
//! not written receives its catalog default with `default: true` (or `null`
//! when it has none), and a short source form named by `expand` is expanded
//! to an `info` head followed by the command in its form for `entry` input.

use std::collections::BTreeMap;

use nurt_contracts::ast::{Arg, Expr, Stage};
use nurt_contracts::bound::{
    BoundArg, BoundExpr, BoundMapEntry, BoundStage, Phase, STAGE_CALL_PARAMETERS,
};
use nurt_contracts::envelope::Privilege;
use nurt_contracts::error::ErrorCode;
use nurt_contracts::kinds::Category;
use nurt_contracts::spec::{FormSpec, IoSpec, Os, ParamSpec, ParamType, ScalarType, StageSpec};
use nurt_correct::distance::{EXPECTED_LIMIT, NameEntry, NameIndex, NameKind, expected};

use crate::binder::{Binder, Flow, internal};
use crate::cast::Place;
use crate::error::{BindError, join_spans};
use crate::fields::{flow_field_types, flow_fields};
use crate::literal::default_expr;
use crate::phase::{effect_of, waits_of};

/// Command that heads an expanded source form.
const EXPANSION_HEAD: &str = "info";

/// Record kind consumed by the form an expanded source form binds to.
const ENTRY_RECORD: &str = "entry";

/// Call parameter that `default_from` may name.
const BASE_PARAMETER: &str = "base";

/// Parameter whose value `auto` is resolved from the kind of the pattern.
const MODE_PARAMETER: &str = "mode";

/// Parameter that holds the pattern of a search or a replacement.
const PATTERN_PARAMETER: &str = "pattern";

/// Mode value resolved from the pattern.
const AUTO_MODE: &str = "auto";

/// Mode of a text pattern.
const LITERAL_MODE: &str = "literal";

/// Mode of a regular expression pattern.
const REGEX_MODE: &str = "regex";

/// Largest length of a chain of parameters whose default is another
/// parameter.
const MAX_DERIVATION_DEPTH: usize = 4;

/// A bound stage with the facts the pipeline needs to assign its phase.
pub(crate) struct Bound {
    pub(crate) stage: BoundStage,
    pub(crate) output: Flow,
    pub(crate) category: Category,
}

/// Surface arguments written for one parameter, with their source positions.
#[derive(Debug, Clone, Default)]
struct Written<'s> {
    args: Vec<(usize, &'s Arg)>,
}

impl Written<'_> {
    fn by_name(&self, param: &str) -> bool {
        self.args
            .iter()
            .any(|(_, arg)| arg.name.as_deref() == Some(param))
    }
}

/// Parameters accepted by the selected form.
struct Accepted<'s> {
    names: Vec<&'s str>,
}

impl<'s> Accepted<'s> {
    fn new(spec: &'s StageSpec, form: Option<&'s FormSpec>) -> Self {
        let names = match form {
            Some(form) if !form.params.is_empty() => {
                form.params.iter().map(String::as_str).collect()
            }
            _ => spec
                .params
                .iter()
                .map(|param| param.name.as_str())
                .collect(),
        };
        Self { names }
    }

    fn contains(&self, name: &str) -> bool {
        self.names.contains(&name)
    }

    fn nearest(&self, written: &str) -> Vec<String> {
        let index = NameIndex::new(
            self.names
                .iter()
                .map(|name| NameEntry::name(*name, NameKind::Param)),
        );
        expected(written, &index, NameKind::Param, EXPECTED_LIMIT)
    }
}

impl<'a> Binder<'a> {
    /// Binds a stage that receives records of the given flow; an expanded
    /// source form gives two stages.
    pub(crate) fn command(&mut self, stage: &Stage, flow: &Flow) -> Result<Vec<Bound>, BindError> {
        let fields = std::mem::replace(&mut self.fields, flow_fields(flow));
        let field_types = std::mem::replace(&mut self.field_types, flow_field_types(flow));
        let bound = self.bind_command(stage, flow);
        self.fields = fields;
        self.field_types = field_types;
        bound
    }

    fn bind_command(&mut self, stage: &Stage, flow: &Flow) -> Result<Vec<Bound>, BindError> {
        let spec = self.spec(stage)?;
        let elevated = self.elevated_run(stage);
        let stage = elevated.as_ref().unwrap_or(stage);
        let (stripped, call) = call_parameters(stage, spec)?;
        let stage = stripped.as_ref().unwrap_or(stage);
        let mut bound = self.bind_signature(stage, spec, flow)?;
        if let Some(last) = bound.last_mut() {
            last.stage.args.extend(call);
        }
        Ok(bound)
    }

    fn bind_signature(
        &mut self,
        stage: &Stage,
        spec: &StageSpec,
        flow: &Flow,
    ) -> Result<Vec<Bound>, BindError> {
        self.check_os(spec, stage)?;
        let (previewless, preview) = split_preview(stage, spec);
        let stage = previewless.as_ref().unwrap_or(stage);
        let (input, form) = select(spec, flow).ok_or_else(|| input_error(spec, flow, stage))?;
        let accepted = Accepted::new(spec, form);
        let collects_names = named_expression_param(spec).is_some();
        let corrected = self.correct_params(stage, spec, &accepted.names, collects_names);
        let stage = corrected.as_ref().unwrap_or(stage);
        let flagged = expand_flags(stage, spec);
        let stage = flagged.as_ref().unwrap_or(stage);
        let written = self.assign(stage, spec, form, &accepted)?;
        if let Some(form) = form
            && input.is_none()
            && let Some(target) = form.expand.as_deref()
        {
            return self.expand(stage, spec, form, &written, (target, preview.as_ref()));
        }
        let mut args = self.arguments(stage, spec, form, &written)?;
        if let Some(preview) = preview {
            let value = self.value(
                &preview,
                &ParamType::Scalar(ScalarType::Bool),
                Place {
                    command: &spec.name,
                    param: PREVIEW,
                    per_record: false,
                },
            )?;
            args.push(BoundArg::given(PREVIEW, value));
        }
        let effect = effect_of(spec, &args, &input);
        let waits = waits_of(spec, &args);
        Ok(vec![Bound {
            stage: BoundStage {
                name: spec.name.clone(),
                effect,
                phase: Phase::Source,
                waits,
                args,
                span: stage.span,
            },
            output: output_flow(spec, form, flow),
            category: spec.category,
        }])
    }

    fn check_os(&self, spec: &StageSpec, stage: &Stage) -> Result<(), BindError> {
        if spec.os.is_empty() || spec.os.contains(&self.context.host_os) {
            return Ok(());
        }
        let systems: Vec<String> = spec.os.iter().map(|os| os_name(*os).to_owned()).collect();
        Err(BindError::new(
            ErrorCode::OsOnly,
            format!(
                "`{}` is available only on {}; use a command of this system",
                spec.name,
                systems.join(", ")
            ),
        )
        .at(stage.span)
        .expected(&systems)
        .help(spec.name.clone()))
    }

    /// Expands a source form: binds it as written to check it, then binds
    /// the target parameter as the `path` of an `info` head and the other
    /// written parameters as the command in its form for `entry` input.
    fn expand(
        &mut self,
        stage: &Stage,
        spec: &StageSpec,
        form: &FormSpec,
        written: &[Option<Written<'_>>],
        (target, preview): (&str, Option<&Expr>),
    ) -> Result<Vec<Bound>, BindError> {
        self.arguments(stage, spec, Some(form), written)?;
        let mut head_args = Vec::new();
        let mut rest_args = Vec::new();
        for (param, given) in spec.params.iter().zip(written) {
            let Some(given) = given else {
                continue;
            };
            if param.name == target {
                head_args.extend(given.args.iter().map(|(_, arg)| Arg {
                    name: None,
                    value: arg.value.clone(),
                    span: arg.span,
                }));
            } else {
                rest_args.push(named_arg(&param.name, given));
            }
        }
        if self.catalog.command(EXPANSION_HEAD).is_none() {
            return Err(internal(format!(
                "the catalog has no `{EXPANSION_HEAD}` command for the source form of `{}`",
                spec.name
            )));
        }
        if let Some(preview) = preview {
            rest_args.push(Arg {
                name: Some(PREVIEW.to_owned()),
                value: preview.clone(),
                span: preview_span(preview),
            });
        }
        let head = Stage {
            name: EXPANSION_HEAD.to_owned(),
            args: head_args,
            span: stage.span,
        };
        let rest = Stage {
            name: spec.name.clone(),
            args: rest_args,
            span: stage.span,
        };
        let mut bound = self.command(&head, &Flow::Start)?;
        let entries = Flow::Records(vec![IoSpec::Record(ENTRY_RECORD.to_owned())]);
        bound.extend(self.command(&rest, &entries)?);
        Ok(bound)
    }

    /// Assigns the written arguments to the parameters of the signature.
    fn assign<'s>(
        &self,
        stage: &'s Stage,
        spec: &'s StageSpec,
        form: Option<&'s FormSpec>,
        accepted: &Accepted<'_>,
    ) -> Result<Vec<Option<Written<'s>>>, BindError> {
        let mut written: Vec<Option<Written<'s>>> = vec![None; spec.params.len()];
        let mut positional = Vec::new();
        for (index, arg) in stage.args.iter().enumerate() {
            match arg.name.as_deref() {
                None => positional.push((index, arg)),
                Some(name) => {
                    assign_named(stage, spec, accepted, (index, arg), name, &mut written)?
                }
            }
        }
        assign_positional(stage, spec, form, &positional, &mut written)?;
        for given in written.iter_mut().flatten() {
            given.args.sort_by_key(|(index, _)| *index);
        }
        Ok(written)
    }

    /// Binds every parameter of the signature, in signature order.
    fn arguments(
        &mut self,
        stage: &Stage,
        spec: &StageSpec,
        form: Option<&FormSpec>,
        written: &[Option<Written<'_>>],
    ) -> Result<Vec<BoundArg>, BindError> {
        check_conflicts(spec, written)?;
        self.check_required(stage, spec, form, written)?;
        let mut values: Vec<Option<BoundArg>> = vec![None; spec.params.len()];
        for (slot, (param, given)) in values.iter_mut().zip(spec.params.iter().zip(written)) {
            *slot = match given {
                Some(given) => Some(self.written_arg(spec, param, given)?),
                None if param.default_from.is_none() => Some(self.default_arg(spec, param)?),
                None => None,
            };
        }
        self.derive(spec, &mut values)?;
        resolve_mode(spec, written, &mut values);
        Ok(values
            .into_iter()
            .zip(&spec.params)
            .map(|(value, param)| {
                value.unwrap_or_else(|| BoundArg::from_default(&param.name, BoundExpr::Null))
            })
            .collect())
    }

    fn written_arg(
        &mut self,
        spec: &StageSpec,
        param: &ParamSpec,
        given: &Written<'_>,
    ) -> Result<BoundArg, BindError> {
        let place = Place {
            command: &spec.name,
            param: &param.name,
            per_record: param.per_record,
        };
        let value = match given.args.as_slice() {
            [(_, arg)] if !is_extra_named(arg, param) => {
                self.value(&arg.value, &param.ty, place)?
            }
            args => self.repeated(args, param, place)?,
        };
        let first = given.args.first().and_then(|(_, arg)| arg.span);
        let last = given.args.last().and_then(|(_, arg)| arg.span);
        Ok(BoundArg {
            name: param.name.clone(),
            value,
            default: false,
            span: join_spans(first, last),
        })
    }

    /// Binds the arguments collected by a repeated parameter into a list;
    /// a named argument outside the signature becomes a named expression
    /// `{name: expression}`.
    fn repeated(
        &mut self,
        args: &[(usize, &Arg)],
        param: &ParamSpec,
        place: Place<'_>,
    ) -> Result<BoundExpr, BindError> {
        let element = list_element(&param.ty);
        let mut items = Vec::with_capacity(args.len());
        for (_, arg) in args {
            let item = match arg.name.as_deref() {
                Some(name) if name != param.name => BoundExpr::Map {
                    entries: vec![BoundMapEntry {
                        k: name.to_owned(),
                        v: self.expression(&arg.value)?,
                    }],
                },
                _ => self.element(&arg.value, element, place)?,
            };
            items.push(item);
        }
        Ok(BoundExpr::List { items })
    }

    fn default_arg(&mut self, spec: &StageSpec, param: &ParamSpec) -> Result<BoundArg, BindError> {
        let Some(default) = &param.default else {
            return Ok(BoundArg::from_default(&param.name, BoundExpr::Null));
        };
        let unreadable = || {
            internal(format!(
                "the catalog default `{default}` of `{}` parameter `{}` does not fit its type {}",
                spec.name, param.name, param.ty
            ))
        };
        let expr = default_expr(default).ok_or_else(unreadable)?;
        let place = Place {
            command: &spec.name,
            param: &param.name,
            per_record: false,
        };
        let value = self
            .exactly(|binder| binder.value(&expr, &param.ty, place))
            .map_err(|_| unreadable())?;
        Ok(BoundArg::from_default(&param.name, value))
    }

    /// Fills the parameters whose default is another parameter or the call
    /// parameter `base`, converted to the type of the parameter.
    fn derive(
        &mut self,
        spec: &StageSpec,
        values: &mut [Option<BoundArg>],
    ) -> Result<(), BindError> {
        for _ in 0..MAX_DERIVATION_DEPTH {
            for (position, param) in spec.params.iter().enumerate() {
                let Some(source) = param.default_from.as_deref() else {
                    continue;
                };
                if values[position].is_some() {
                    continue;
                }
                let derived = if source == BASE_PARAMETER {
                    Some(self.base_value(spec, param)?)
                } else {
                    spec.params
                        .iter()
                        .position(|other| other.name == source)
                        .and_then(|other| values[other].as_ref())
                        .map(|other| converted(other.value.clone(), &param.ty))
                };
                values[position] = derived.map(|value| BoundArg::from_default(&param.name, value));
            }
        }
        Ok(())
    }

    fn base_value(&mut self, spec: &StageSpec, param: &ParamSpec) -> Result<BoundExpr, BindError> {
        let Some(base) = self.context.base.clone() else {
            return Ok(BoundExpr::Null);
        };
        let place = Place {
            command: &spec.name,
            param: &param.name,
            per_record: false,
        };
        let text = Expr::Text {
            v: base,
            span: None,
        };
        self.exactly(|binder| binder.value(&text, &param.ty, place))
    }

    fn check_required(
        &self,
        stage: &Stage,
        spec: &StageSpec,
        form: Option<&FormSpec>,
        written: &[Option<Written<'_>>],
    ) -> Result<(), BindError> {
        let required: Vec<&str> = match form {
            Some(form) => form.required.iter().map(String::as_str).collect(),
            None => spec
                .params
                .iter()
                .filter(|param| param.required)
                .map(|param| param.name.as_str())
                .chain(spec.required_groups.iter().map(String::as_str))
                .collect(),
        };
        let groups = spec.exclusive_groups();
        for name in required {
            if let Some(members) = groups.get(name) {
                let given = members
                    .iter()
                    .any(|member| is_written(spec, written, member));
                if !given {
                    return Err(missing_group(stage, spec, name, members));
                }
                continue;
            }
            let Some(position) = spec.params.iter().position(|param| param.name == name) else {
                continue;
            };
            let param = &spec.params[position];
            if written[position].is_none() && !self.derivable(spec, param, written) {
                return Err(self.missing_param(stage, spec, param));
            }
        }
        Ok(())
    }

    fn derivable(
        &self,
        spec: &StageSpec,
        param: &ParamSpec,
        written: &[Option<Written<'_>>],
    ) -> bool {
        match param.default_from.as_deref() {
            None => false,
            Some(BASE_PARAMETER) => self.context.base.is_some(),
            Some(source) => spec
                .params
                .iter()
                .position(|other| other.name == source)
                .is_some_and(|other| {
                    written[other].is_some() || spec.params[other].default.is_some()
                }),
        }
    }

    fn missing_param(&self, stage: &Stage, spec: &StageSpec, param: &ParamSpec) -> BindError {
        let hint = if param.default_from.as_deref() == Some(BASE_PARAMETER) {
            format!("write `{}:` or give the call parameter base", param.name)
        } else {
            format!("write `{}:`", param.name)
        };
        let mut error = BindError::new(
            ErrorCode::MissingParam,
            format!(
                "`{}` needs the parameter `{}`; {hint}",
                spec.name, param.name
            ),
        )
        .at(stage.span)
        .expected(std::slice::from_ref(&param.name))
        .help(spec.name.clone());
        for value in param.ty.enum_values().into_iter().take(EXPECTED_LIMIT) {
            if let Some(program) = self.program_with(&param.name, value) {
                error = error.fix(program);
            }
        }
        error
    }
}

fn assign_named<'s>(
    stage: &Stage,
    spec: &'s StageSpec,
    accepted: &Accepted<'_>,
    arg: (usize, &'s Arg),
    name: &str,
    written: &mut [Option<Written<'s>>],
) -> Result<(), BindError> {
    let position = spec.params.iter().position(|param| param.name == name);
    let target = match position {
        Some(position) if accepted.contains(name) => {
            if written[position]
                .as_ref()
                .is_some_and(|given| given.by_name(name))
            {
                return Err(given_twice(spec, name, arg.1));
            }
            position
        }
        Some(_) => return Err(not_accepted(stage, spec, accepted, name, arg.1)),
        None => match named_expression_param(spec) {
            Some(position) => position,
            None => return Err(unknown_param(spec, accepted, name, arg.1)),
        },
    };
    written[target]
        .get_or_insert_with(Written::default)
        .args
        .push(arg);
    Ok(())
}

fn assign_positional<'s>(
    stage: &Stage,
    spec: &'s StageSpec,
    form: Option<&'s FormSpec>,
    positional: &[(usize, &'s Arg)],
    written: &mut [Option<Written<'s>>],
) -> Result<(), BindError> {
    let names: Vec<&str> = positional_names(spec, form)
        .into_iter()
        .filter(|name| {
            spec.params
                .iter()
                .position(|param| param.name == *name)
                .is_none_or(|position| {
                    written[position]
                        .as_ref()
                        .is_none_or(|given| !given.by_name(name))
                })
        })
        .collect();
    let repeated = names
        .iter()
        .position(|name| spec.param(name).is_some_and(|param| param.repeated));
    let targets = distribute(names.len(), repeated, positional.len());
    if targets.len() < positional.len() {
        let extra = positional.get(targets.len()).map(|(_, arg)| *arg);
        return Err(too_many(stage, spec, &names, positional.len(), extra));
    }
    for (target, arg) in targets.into_iter().zip(positional) {
        let Some(position) = names
            .get(target)
            .and_then(|name| spec.params.iter().position(|param| param.name == *name))
        else {
            continue;
        };
        written[position]
            .get_or_insert_with(Written::default)
            .args
            .push(*arg);
    }
    Ok(())
}

/// Maps each positional argument to the index of its parameter among the
/// positional names. A repeated parameter collects the arguments that the
/// parameters after it leave, and at least one.
fn distribute(slots: usize, repeated: Option<usize>, count: usize) -> Vec<usize> {
    let Some(repeated) = repeated else {
        return (0..count.min(slots)).collect();
    };
    if count <= repeated {
        return (0..count).collect();
    }
    let after = slots - repeated - 1;
    let rest = count - repeated;
    let collected = if rest > after { rest - after } else { 1 };
    let mut targets: Vec<usize> = (0..repeated).collect();
    targets.extend(std::iter::repeat_n(repeated, collected));
    targets.extend((repeated + 1..slots).take(rest - collected));
    targets
}

fn positional_names<'s>(spec: &'s StageSpec, form: Option<&'s FormSpec>) -> Vec<&'s str> {
    match form {
        Some(form) => form.positional.iter().map(String::as_str).collect(),
        None => spec
            .positional()
            .into_iter()
            .map(|param| param.name.as_str())
            .collect(),
    }
}

/// Repeated parameter that collects named expressions (`select total: …`).
fn named_expression_param(spec: &StageSpec) -> Option<usize> {
    spec.params
        .iter()
        .position(|param| param.repeated && param.ty.contains_scalar(ScalarType::NamedExpression))
}

fn is_extra_named(arg: &Arg, param: &ParamSpec) -> bool {
    arg.name.as_deref().is_some_and(|name| name != param.name)
}

fn is_written(spec: &StageSpec, written: &[Option<Written<'_>>], name: &str) -> bool {
    spec.params
        .iter()
        .position(|param| param.name == name)
        .is_some_and(|position| written[position].is_some())
}

fn check_conflicts(spec: &StageSpec, written: &[Option<Written<'_>>]) -> Result<(), BindError> {
    let groups: BTreeMap<&str, Vec<&str>> = spec.exclusive_groups();
    for members in groups.values() {
        let given: Vec<&str> = members
            .iter()
            .copied()
            .filter(|member| is_written(spec, written, member))
            .collect();
        if let [first, second, ..] = given.as_slice() {
            let span = spec
                .params
                .iter()
                .position(|param| param.name == *second)
                .and_then(|position| written[position].as_ref())
                .and_then(|given| given.args.first())
                .and_then(|(_, arg)| arg.span);
            let names: Vec<String> = members.iter().map(|member| (*member).to_owned()).collect();
            return Err(BindError::new(
                ErrorCode::ParamConflict,
                format!(
                    "`{first}` and `{second}` of `{}` exclude each other; keep one of {}",
                    spec.name,
                    names.join(", ")
                ),
            )
            .at(span)
            .expected(&names)
            .help(spec.name.clone()));
        }
    }
    Ok(())
}

/// Resolves the mode `auto` of a pattern that was not written to `literal`
/// for a text pattern and to `regex` for a regular expression.
fn resolve_mode(
    spec: &StageSpec,
    written: &[Option<Written<'_>>],
    values: &mut [Option<BoundArg>],
) {
    let position = |name: &str| spec.params.iter().position(|param| param.name == name);
    let (Some(mode), Some(pattern)) = (position(MODE_PARAMETER), position(PATTERN_PARAMETER))
    else {
        return;
    };
    let modes = spec.params[mode].ty.enum_values();
    if written[mode].is_some() || !modes.contains(&LITERAL_MODE) || !modes.contains(&REGEX_MODE) {
        return;
    }
    let resolved = match values[pattern].as_ref().map(|arg| &arg.value) {
        Some(BoundExpr::Regex { .. }) => REGEX_MODE,
        Some(BoundExpr::Text { .. }) => LITERAL_MODE,
        _ => return,
    };
    if let Some(arg) = values[mode].as_mut()
        && matches!(&arg.value, BoundExpr::Enum { v } if v == AUTO_MODE)
    {
        arg.value = BoundExpr::Enum {
            v: resolved.to_owned(),
        };
    }
}

/// Converts a value to the type of a parameter whose default it is: a single
/// value becomes a one-element list where the parameter is a list.
fn converted(value: BoundExpr, ty: &ParamType) -> BoundExpr {
    match value {
        BoundExpr::Null | BoundExpr::List { .. } | BoundExpr::Ref(_) => value,
        single if ty.accepts_list() && !accepts_single(ty) => BoundExpr::List {
            items: vec![single],
        },
        single => single,
    }
}

fn accepts_single(ty: &ParamType) -> bool {
    ty.alternatives()
        .iter()
        .any(|alternative| !matches!(alternative, ParamType::List(_)))
}

/// Element type of the list alternative of a type, or the type itself.
fn list_element(ty: &ParamType) -> &ParamType {
    ty.alternatives()
        .iter()
        .find_map(|alternative| match alternative {
            ParamType::List(inner) => Some(inner.as_ref()),
            _ => None,
        })
        .unwrap_or(ty)
}

/// Takes the call parameters `privilege` and `as_user` out of a stage.
///
/// The parameters apply to the whole call, like the options of the request,
/// and are accepted on every command that does not declare a parameter of
/// the same name. The stage without them is returned when there were any,
/// together with their bound values, which follow the parameters of the
/// signature.
fn call_parameters(
    stage: &Stage,
    spec: &StageSpec,
) -> Result<(Option<Stage>, Vec<BoundArg>), BindError> {
    let is_call = |arg: &Arg| {
        arg.name.as_deref().is_some_and(|name| {
            STAGE_CALL_PARAMETERS.contains(&name) && !spec.params.iter().any(|p| p.name == name)
        })
    };
    if !stage.args.iter().any(is_call) {
        return Ok((None, Vec::new()));
    }
    let mut rest = stage.clone();
    rest.args.retain(|arg| !is_call(arg));
    let mut bound: Vec<BoundArg> = Vec::new();
    for arg in stage.args.iter().filter(|arg| is_call(arg)) {
        let name = arg.name.as_deref().unwrap_or_default();
        if bound.iter().any(|known| known.name == name) {
            return Err(given_twice(spec, name, arg));
        }
        let word = match &arg.value {
            Expr::Ident { v, .. } | Expr::Text { v, .. } => v.as_str(),
            _ => return Err(call_value_error(spec, name, arg, "a word or a text")),
        };
        if name == "privilege" && !Privilege::ALL.iter().any(|value| value.as_str() == word) {
            return Err(call_value_error(
                spec,
                name,
                arg,
                "account, elevated or auto",
            ));
        }
        if name == "as_user" && word.is_empty() {
            return Err(call_value_error(spec, name, arg, "the name of an account"));
        }
        bound.push(BoundArg {
            name: name.to_owned(),
            value: if name == "privilege" {
                BoundExpr::Enum { v: word.to_owned() }
            } else {
                BoundExpr::Text { v: word.to_owned() }
            },
            default: false,
            span: arg.span,
        });
    }
    Ok((Some(rest), bound))
}

fn call_value_error(spec: &StageSpec, name: &str, arg: &Arg, expected: &str) -> BindError {
    BindError::new(
        ErrorCode::InvalidValue,
        format!("`{name}` of `{}` takes {expected}", spec.name),
    )
    .at(arg.span)
    .found(name.to_owned())
    .help(spec.name.clone())
}

/// Joins the arguments written for a parameter into one named argument.
fn named_arg(name: &str, given: &Written<'_>) -> Arg {
    let first = given.args.first().and_then(|(_, arg)| arg.span);
    let last = given.args.last().and_then(|(_, arg)| arg.span);
    let value = match given.args.as_slice() {
        [(_, arg)] => arg.value.clone(),
        args => Expr::List {
            items: args.iter().map(|(_, arg)| arg.value.clone()).collect(),
            span: join_spans(first, last),
        },
    };
    Arg {
        name: Some(name.to_owned()),
        value,
        span: join_spans(first, last),
    }
}

/// Chooses the input kind of a stage and the form that applies to it.
fn select<'s>(spec: &'s StageSpec, flow: &Flow) -> Option<(IoSpec, Option<&'s FormSpec>)> {
    let input = choose_input(&spec.input, flow)?;
    if spec.forms.is_empty() {
        return Some((input, None));
    }
    let form = spec
        .form_for(&input)
        .or_else(|| spec.form_for(&IoSpec::Any))?;
    Some((input, Some(form)))
}

fn choose_input(inputs: &[IoSpec], flow: &Flow) -> Option<IoSpec> {
    let first_record = || inputs.iter().find(|input| !input.is_none()).cloned();
    match flow {
        Flow::Start => inputs.contains(&IoSpec::None).then_some(IoSpec::None),
        Flow::Open if inputs.contains(&IoSpec::None) => Some(IoSpec::None),
        Flow::Open => first_record(),
        Flow::Records(kinds) => {
            if let Some(kind) = kinds
                .iter()
                .find(|kind| **kind != IoSpec::Any && inputs.contains(kind))
            {
                return Some(kind.clone());
            }
            if kinds.contains(&IoSpec::Any) {
                return first_record();
            }
            inputs
                .contains(&IoSpec::Any)
                .then(|| kinds.first().cloned())
                .flatten()
        }
    }
}

/// Kind of the records a bound stage produces.
fn output_flow(spec: &StageSpec, form: Option<&FormSpec>, flow: &Flow) -> Flow {
    let outputs = form
        .map(|form| &form.output)
        .filter(|outputs| !outputs.is_empty())
        .unwrap_or(&spec.output);
    let mut kinds = Vec::new();
    for output in outputs {
        match (output, flow) {
            (IoSpec::Same, Flow::Records(input)) => kinds.extend(input.iter().cloned()),
            (IoSpec::Same, _) => kinds.push(IoSpec::Any),
            (other, _) => kinds.push(other.clone()),
        }
    }
    Flow::Records(kinds)
}

fn input_error(spec: &StageSpec, flow: &Flow, stage: &Stage) -> BindError {
    let accepted: Vec<String> = spec.input.iter().map(ToString::to_string).collect();
    let message = match flow {
        Flow::Start | Flow::Open => format!(
            "`{}` needs input records and cannot start a pipeline; start with a source such as `find` and add `|> {}`",
            spec.name, spec.name
        ),
        Flow::Records(_) if spec.input == [IoSpec::None] => format!(
            "`{}` starts a pipeline and takes no input; write it as the first stage",
            spec.name
        ),
        Flow::Records(kinds) => format!(
            "`{}` does not accept {} records; it accepts {}",
            spec.name,
            kinds
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", "),
            accepted.join(", ")
        ),
    };
    BindError::new(ErrorCode::Type, message)
        .at(stage.span)
        .expected(&accepted)
        .help(spec.name.clone())
}

fn missing_group(stage: &Stage, spec: &StageSpec, group: &str, members: &[&str]) -> BindError {
    let names: Vec<String> = members.iter().map(|member| (*member).to_owned()).collect();
    BindError::new(
        ErrorCode::MissingParam,
        format!(
            "`{}` needs one {group} parameter; write one of {}",
            spec.name,
            names.join(", ")
        ),
    )
    .at(stage.span)
    .expected(&names)
    .help(spec.name.clone())
}

fn given_twice(spec: &StageSpec, name: &str, arg: &Arg) -> BindError {
    BindError::new(
        ErrorCode::Arity,
        format!(
            "`{name}` of `{}` is given twice; keep one argument",
            spec.name
        ),
    )
    .at(arg.span)
    .found(name.to_owned())
    .help(spec.name.clone())
}

fn not_accepted(
    stage: &Stage,
    spec: &StageSpec,
    accepted: &Accepted<'_>,
    name: &str,
    arg: &Arg,
) -> BindError {
    BindError::new(
        ErrorCode::UnknownParam,
        format!(
            "`{name}` is not a parameter of `{}` in this position of the pipeline; it belongs to its source form",
            stage.name
        ),
    )
    .at(arg.span)
    .found(name.to_owned())
    .expected(&accepted.nearest(name))
    .help(spec.name.clone())
}

fn unknown_param(spec: &StageSpec, accepted: &Accepted<'_>, name: &str, arg: &Arg) -> BindError {
    BindError::new(
        ErrorCode::UnknownParam,
        format!("`{}` has no parameter `{name}`", spec.name),
    )
    .at(arg.span)
    .found(name.to_owned())
    .expected(&accepted.nearest(name))
    .help(spec.name.clone())
}

fn too_many(
    stage: &Stage,
    spec: &StageSpec,
    names: &[&str],
    count: usize,
    extra: Option<&Arg>,
) -> BindError {
    let expected: Vec<String> = names.iter().map(|name| (*name).to_owned()).collect();
    BindError::new(
        ErrorCode::Arity,
        format!(
            "`{}` takes {} positional arguments here and {count} are given; name the other arguments",
            spec.name,
            names.len()
        ),
    )
    .at(extra.and_then(|arg| arg.span).or(stage.span))
    .expected(&expected)
    .help(spec.name.clone())
}

fn os_name(os: Os) -> &'static str {
    match os {
        Os::Linux => "linux",
        Os::Macos => "macos",
        Os::Windows => "windows",
        _ => "other",
    }
}

/// Source range of an expression, when it has one.
fn preview_span(expr: &Expr) -> Option<nurt_contracts::ast::Span> {
    crate::cast::expr_span(expr)
}

/// Name of the parameter every change accepts, besides its own.
const PREVIEW: &str = "preview";

/// Takes `preview` out of the arguments of a command that has no parameter of
/// that name: `remove preview`, `remove preview: true` and
/// `remove preview: @dry` compute the effect of that stage without applying
/// it. Returns the stage without the argument and the value written.
fn split_preview(stage: &Stage, spec: &StageSpec) -> (Option<Stage>, Option<Expr>) {
    if spec.param(PREVIEW).is_some() {
        return (None, None);
    }
    let Some(index) = stage
        .args
        .iter()
        .position(|arg| match (&arg.name, &arg.value) {
            (Some(name), _) => name == PREVIEW,
            (None, Expr::Ident { v, .. }) => v == PREVIEW,
            _ => false,
        })
    else {
        return (None, None);
    };
    let mut rest = stage.clone();
    let arg = rest.args.remove(index);
    let value = match arg.name {
        Some(_) => arg.value,
        None => Expr::Bool {
            v: true,
            span: arg.span,
        },
    };
    (Some(rest), Some(value))
}

/// Reads a parameter of type `bool` written without a value as `true`:
/// `remove preview` is `remove preview: true`. A word that names a `bool`
/// parameter that is not written otherwise, and that is no value of an
/// enumeration parameter of the command, is the parameter; the long form
/// stays valid and `false` is always written out.
fn expand_flags(stage: &Stage, spec: &StageSpec) -> Option<Stage> {
    let named: Vec<&str> = stage
        .args
        .iter()
        .filter_map(|arg| arg.name.as_deref())
        .collect();
    let mut changed: Option<Stage> = None;
    for (index, arg) in stage.args.iter().enumerate() {
        let (None, Expr::Ident { v, span }) = (&arg.name, &arg.value) else {
            continue;
        };
        let is_flag = spec
            .param(v)
            .is_some_and(|param| matches!(param.ty, ParamType::Scalar(ScalarType::Bool)));
        let is_value = spec.params.iter().any(|param| match &param.ty {
            ParamType::Enum(values) => values.contains(v),
            _ => false,
        });
        let repeated = changed
            .as_ref()
            .map_or(&stage.args, |changed| &changed.args)
            .iter()
            .any(|other| other.name.as_deref() == Some(v.as_str()));
        if !is_flag || is_value || repeated || named.contains(&v.as_str()) {
            continue;
        }
        let edited = changed.get_or_insert_with(|| stage.clone());
        edited.args[index] = Arg {
            name: Some(v.clone()),
            value: Expr::Bool {
                v: true,
                span: *span,
            },
            span: arg.span,
        };
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distributes_positional_arguments_around_a_repeated_parameter() {
        assert_eq!(distribute(2, Some(0), 3), [0, 0, 1]);
        assert_eq!(distribute(2, Some(0), 2), [0, 1]);
        assert_eq!(distribute(2, Some(0), 1), [0]);
        assert_eq!(distribute(1, Some(0), 3), [0, 0, 0]);
        assert_eq!(distribute(3, Some(1), 4), [0, 1, 1, 2]);
        assert_eq!(distribute(3, Some(1), 1), [0]);
    }

    #[test]
    fn distributes_positional_arguments_without_a_repeated_parameter() {
        assert_eq!(distribute(2, None, 2), [0, 1]);
        assert_eq!(distribute(2, None, 3), [0, 1]);
        assert!(distribute(0, None, 1).is_empty());
    }

    #[test]
    fn chooses_the_input_kind_by_preference() {
        let inputs = [IoSpec::None, IoSpec::Record("entry".to_owned())];
        assert_eq!(choose_input(&inputs, &Flow::Start), Some(IoSpec::None));
        let matches = Flow::Records(vec![
            IoSpec::Record("match".to_owned()),
            IoSpec::Record("entry".to_owned()),
        ]);
        assert_eq!(
            choose_input(&inputs, &matches),
            Some(IoSpec::Record("entry".to_owned()))
        );
        assert_eq!(
            choose_input(&inputs, &Flow::any()),
            Some(IoSpec::Record("entry".to_owned()))
        );
        assert_eq!(choose_input(&[IoSpec::Any], &Flow::Start), None);
        assert_eq!(choose_input(&[IoSpec::Any], &Flow::Open), Some(IoSpec::Any));
    }

    #[test]
    fn converts_a_single_value_to_a_list() {
        let ty: ParamType = "list<path>".parse().unwrap();
        let path = BoundExpr::Path {
            v: "/srv".to_owned(),
        };
        assert_eq!(
            converted(path.clone(), &ty),
            BoundExpr::List {
                items: vec![path.clone()]
            }
        );
        let single: ParamType = "path".parse().unwrap();
        assert_eq!(converted(path.clone(), &single), path);
        assert_eq!(converted(BoundExpr::Null, &ty), BoundExpr::Null);
    }
}
