// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
// A failed unwrap, expectation or panic is how a test reports a failure.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Every command of the catalog binds with sample values of its required
//! parameters, and every parameter is accepted by a form of its command.
//! A command that needs input records is bound after a source that
//! produces them.

mod bind_support;

use bind_support::{catalog, named, pipeline, program};
use nurt_bind::{BindContext, bind};
use nurt_contracts::ast::{Arg, AstDocument, Expr, MapEntry, Statement};
use nurt_contracts::spec::{FormSpec, IoSpec, Os, ParamType, ScalarType, StageSpec};
use nurt_correct::policy::is_effecting;

/// Number of commands of the reference catalog.
const COMMANDS: usize = 177;

/// Field names tried, in order, where a parameter names a record field.
const FIELDS: [&str; 6] = ["name", "path", "text", "pid", "id", "port"];

/// Command used as the sample of a sub-pipeline.
const SUB_COMMAND: &str = "count";

/// Sample values of the scalar types.
fn scalar(ty: ScalarType, field: &str, os: Os) -> Expr {
    let text = |v: &str| Expr::Text {
        v: v.to_owned(),
        span: None,
    };
    match ty {
        ScalarType::Path if os == Os::Windows => text("C:/srv/app/a.txt"),
        ScalarType::Path => text("/srv/app/a.txt"),
        ScalarType::Int | ScalarType::Number => Expr::Int { v: 1, span: None },
        ScalarType::Bool | ScalarType::Expression => Expr::Bool {
            v: true,
            span: None,
        },
        ScalarType::Glob => text("*"),
        ScalarType::Regex => Expr::Regex {
            v: "x".to_owned(),
            flags: String::new(),
            span: None,
        },
        ScalarType::Size => Expr::Size {
            v: 1_024,
            span: None,
        },
        ScalarType::Duration => Expr::Dur {
            v: 1_000,
            span: None,
        },
        ScalarType::Percent => text("10%"),
        ScalarType::Instant => text("2026-01-01T00:00:00Z"),
        ScalarType::Key => text("a"),
        ScalarType::Range => Expr::Range {
            lo: Some(1),
            hi: Some(2),
            span: None,
        },
        ScalarType::Field => Expr::Ident {
            v: field.to_owned(),
            span: None,
        },
        ScalarType::NamedExpression => Expr::Map {
            entries: vec![MapEntry {
                k: "total".to_owned(),
                v: Expr::Int { v: 1, span: None },
                span: None,
            }],
            span: None,
        },
        ScalarType::Pipeline => Expr::Ident {
            v: SUB_COMMAND.to_owned(),
            span: None,
        },
        _ => text("x"),
    }
}

/// Sample value of a parameter type.
fn sample(ty: &ParamType, field: &str, os: Os) -> Expr {
    match ty {
        ParamType::Scalar(scalar_type) => scalar(*scalar_type, field, os),
        ParamType::Enum(values) => Expr::Ident {
            v: values.first().cloned().unwrap_or_default(),
            span: None,
        },
        ParamType::List(inner) => sample(inner, field, os),
        ParamType::Map(inner) => Expr::Map {
            entries: vec![MapEntry {
                k: "a".to_owned(),
                v: sample(inner, field, os),
                span: None,
            }],
            span: None,
        },
        ParamType::Union(alternatives) => alternatives
            .first()
            .map_or(Expr::Null { span: None }, |first| sample(first, field, os)),
        _ => scalar(ScalarType::Path, field, os),
    }
}

/// Form of a command for records of the kind; the form for no input when
/// `input` is `None`.
fn form_for<'s>(spec: &'s StageSpec, input: &IoSpec) -> Option<&'s FormSpec> {
    spec.form_for(input).or_else(|| spec.form_for(&IoSpec::Any))
}

/// Named sample arguments of the required parameters and groups.
fn arguments(spec: &StageSpec, input: &IoSpec, field: &str, os: Os) -> Vec<Arg> {
    let required: Vec<String> = match form_for(spec, input) {
        Some(form) => form.required.clone(),
        None => spec
            .params
            .iter()
            .filter(|param| param.required)
            .map(|param| param.name.clone())
            .chain(spec.required_groups.iter().cloned())
            .collect(),
    };
    let groups = spec.exclusive_groups();
    required
        .iter()
        .filter_map(|name| {
            let name = groups
                .get(name.as_str())
                .and_then(|members| members.first().copied())
                .unwrap_or(name.as_str());
            spec.param(name)
                .map(|param| named(&param.name, sample(&param.ty, field, os)))
        })
        .collect()
}

fn host_of(spec: &StageSpec) -> Os {
    if spec.os.is_empty() || spec.os.contains(&Os::Linux) {
        Os::Linux
    } else {
        spec.os[0]
    }
}

fn binds(document: &AstDocument, os: Os) -> Result<(), String> {
    bind(document, catalog(), &BindContext::new(os))
        .map(|_| ())
        .map_err(|error| format!("{} {}", error.code(), error.error().message))
}

/// A source command that produces records of the kind and binds alone.
fn source_for(kind: &IoSpec, effecting: bool) -> Option<(&'static StageSpec, Vec<Arg>)> {
    catalog().commands().iter().find_map(|spec| {
        let produces = spec.output.contains(kind) || *kind == IoSpec::Any;
        let fits = spec.input.contains(&IoSpec::None)
            && produces
            && is_effecting(spec) == effecting
            && host_of(spec) == Os::Linux;
        if !fits {
            return None;
        }
        let args = arguments(spec, &IoSpec::None, FIELDS[0], Os::Linux);
        let document = program(vec![Statement::Pipeline(pipeline(
            bind_support::stage(&spec.name, args.clone()),
            Vec::new(),
        ))]);
        binds(&document, Os::Linux).ok().map(|()| (spec, args))
    })
}

/// Binds a command alone or after a source of one of its input kinds.
fn bind_command(spec: &StageSpec) -> Result<(), String> {
    let os = host_of(spec);
    if spec.input.contains(&IoSpec::None) {
        let mut last = String::new();
        for field in FIELDS {
            let args = arguments(spec, &IoSpec::None, field, os);
            let document = program(vec![Statement::Pipeline(pipeline(
                bind_support::stage(&spec.name, args),
                Vec::new(),
            ))]);
            match binds(&document, os) {
                Ok(()) => return Ok(()),
                Err(error) => last = error,
            }
        }
        return Err(last);
    }
    let effect_input = spec.input == [IoSpec::Record("effect".to_owned())];
    let mut last = format!("no source produces {:?}", spec.input);
    for kind in &spec.input {
        let source = source_for(kind, effect_input).or_else(|| {
            (!is_effecting(spec))
                .then(|| source_for(kind, true))
                .flatten()
        });
        let Some((source, source_args)) = source else {
            continue;
        };
        for field in FIELDS {
            let args = arguments(spec, kind, field, os);
            let document = program(vec![Statement::Pipeline(pipeline(
                bind_support::stage(&source.name, source_args.clone()),
                vec![bind_support::stage(&spec.name, args)],
            ))]);
            match binds(&document, os) {
                Ok(()) => return Ok(()),
                Err(error) => last = format!("after `{}`: {error}", source.name),
            }
        }
    }
    Err(last)
}

#[test]
fn every_command_of_the_catalog_binds() {
    assert_eq!(catalog().len(), COMMANDS);
    let failures: Vec<String> = catalog()
        .commands()
        .iter()
        .filter_map(|spec| {
            bind_command(spec)
                .err()
                .map(|error| format!("{}: {error}", spec.name))
        })
        .collect();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn every_parameter_is_accepted_by_a_form() {
    let unreachable: Vec<String> = catalog()
        .commands()
        .iter()
        .flat_map(|spec| {
            spec.params.iter().filter_map(move |param| {
                let reachable = spec.forms.is_empty()
                    || spec
                        .forms
                        .iter()
                        .any(|form| form.params.is_empty() || form.params.contains(&param.name));
                (!reachable).then(|| format!("{}.{}", spec.name, param.name))
            })
        })
        .collect();
    assert!(unreachable.is_empty(), "{}", unreachable.join(", "));
}
