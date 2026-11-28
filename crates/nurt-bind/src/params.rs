// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-bind::params
// Responsibility:   Values of the parameters of a script.
// Dependencies:     nurt_contracts, nurt_correct, serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Values of the parameters of a script.
//!
//! A script declares its parameters with `params { … }`. The caller gives the
//! values as a JSON object: the command line passes every value as text, the
//! engine interface passes typed JSON. Before any effect every name, every
//! type and every missing value is checked against the declaration.

use nurt_contracts::ast::{AstDocument, Expr, MapEntry, Program};
use nurt_contracts::bound::{BoundExpr, BoundParam};
use nurt_contracts::error::ErrorCode;
use nurt_contracts::spec::{ParamType, ScalarType};
use nurt_correct::distance::{EXPECTED_LIMIT, NameEntry, NameIndex, NameKind, expected};
use serde_json::{Map, Value as Json};

use crate::binder::Binder;
use crate::cast::Place;
use crate::catalog::Catalog;
use crate::context::BindContext;
use crate::error::BindError;
use crate::literal::literal_expr;

/// One declared parameter as a person reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParamDescription {
    /// Name of the parameter.
    pub name: String,
    /// Type as declared.
    pub ty: String,
    /// Default value as text; `None` for a required parameter.
    pub default: Option<String>,
}

impl ParamDescription {
    /// The declaration as written in the script: `days: int = 30`.
    #[must_use]
    pub fn declaration(&self) -> String {
        match &self.default {
            Some(default) => format!("{}: {} = {default}", self.name, self.ty),
            None => format!("{}: {}", self.name, self.ty),
        }
    }
}

/// Describes the declared parameters, for `--help` and for errors.
#[must_use]
pub fn describe_params(declared: &[BoundParam]) -> Vec<ParamDescription> {
    declared
        .iter()
        .map(|param| ParamDescription {
            name: param.name.clone(),
            ty: param.ty.clone(),
            default: param.default.as_ref().map(default_text),
        })
        .collect()
}

/// Writes `value` in the largest unit that divides it exactly.
fn scaled(value: u64, units: &[(&str, u64)]) -> String {
    let (name, factor) = units
        .iter()
        .find(|(_, factor)| value != 0 && value.is_multiple_of(*factor))
        .copied()
        .unwrap_or(("", 1));
    format!("{}{name}", value / factor)
}

fn default_text(value: &BoundExpr) -> String {
    match value {
        BoundExpr::Null => "null".to_owned(),
        BoundExpr::Bool { v } => v.to_string(),
        BoundExpr::Int { v } => v.to_string(),
        BoundExpr::Num { v } => v.to_string(),
        BoundExpr::Text { v } | BoundExpr::Path { v } | BoundExpr::Glob { v } => {
            serde_json::to_string(v).unwrap_or_default()
        }
        BoundExpr::Enum { v } => v.clone(),
        BoundExpr::Dur { v } => scaled(
            *v,
            &[
                ("w", 604_800_000),
                ("d", 86_400_000),
                ("h", 3_600_000),
                ("m", 60_000),
                ("s", 1000),
                ("ms", 1),
            ],
        ),
        BoundExpr::Size { v } => scaled(
            *v,
            &[
                ("tib", 1 << 40),
                ("gib", 1 << 30),
                ("mib", 1 << 20),
                ("kib", 1 << 10),
                ("b", 1),
            ],
        ),
        BoundExpr::List { items } => {
            let items: Vec<String> = items.iter().map(default_text).collect();
            format!("[{}]", items.join(", "))
        }
        other => serde_json::to_string(other).unwrap_or_default(),
    }
}

fn listing(declared: &[BoundParam]) -> Vec<String> {
    describe_params(declared)
        .iter()
        .map(ParamDescription::declaration)
        .collect()
}

/// Binds the values given for the parameters of a script.
///
/// Returns every declared parameter with its bound value, in declaration
/// order: the given value, or the default.
///
/// # Errors
///
/// Returns `E_UNKNOWN_PARAM` for a name that is not declared, with the
/// nearest names, `E_MISSING_PARAM` for a required parameter without value
/// and `E_TYPE` or `E_INVALID_VALUE` for a value that does not fit its type.
/// Every error lists the declaration.
pub fn bind_params(
    catalog: &Catalog,
    context: &BindContext,
    declared: &[BoundParam],
    given: Option<&Map<String, Json>>,
) -> Result<Vec<(String, BoundExpr)>, BindError> {
    let empty = Map::new();
    let given = given.unwrap_or(&empty);
    let index = NameIndex::new(
        declared
            .iter()
            .map(|param| NameEntry::name(param.name.clone(), NameKind::Param)),
    );
    for name in given.keys() {
        if !declared.iter().any(|param| &param.name == name) {
            let message = if declared.is_empty() {
                format!("the script declares no parameters, `{name}` is not one")
            } else {
                format!("the script has no parameter `{name}`")
            };
            return Err(BindError::new(ErrorCode::UnknownParam, message)
                .found(name.clone())
                .expected(&expected(name, &index, NameKind::Param, EXPECTED_LIMIT))
                .detail("declaration", declaration_detail(declared)));
        }
    }
    let document = AstDocument::new(Program {
        statements: Vec::new(),
    });
    let mut binder = Binder::bare(catalog, context, &document);
    let mut bound = Vec::with_capacity(declared.len());
    for param in declared {
        let ty = ParamType::parse(&param.ty).map_err(|error| {
            BindError::new(
                ErrorCode::Type,
                format!("`{}` is not a type: {error}", param.ty),
            )
        })?;
        let value = match given.get(&param.name) {
            Some(Json::Bool(true)) if param.ty != "bool" && !ty_holds_bool(&ty) => {
                return Err(BindError::new(
                    ErrorCode::MissingParam,
                    format!(
                        "`{}` needs a value: write `{}: <{}>`; only a parameter of type bool is given without one",
                        param.name, param.name, param.ty
                    ),
                )
                .found(param.name.clone())
                .expected(std::slice::from_ref(&param.ty))
                .detail("declaration", declaration_detail(declared)));
            }
            Some(json) => {
                let expr = param_expr(json, &ty).ok_or_else(|| {
                    BindError::new(
                        ErrorCode::Type,
                        format!(
                            "the value of `{}` does not fit the type `{}`",
                            param.name, param.ty
                        ),
                    )
                    .expected(std::slice::from_ref(&param.ty))
                    .detail("declaration", declaration_detail(declared))
                })?;
                binder
                    .value(
                        &expr,
                        &ty,
                        Place {
                            command: "params",
                            param: &param.name,
                            per_record: false,
                        },
                    )
                    .map_err(|error| error.detail("declaration", declaration_detail(declared)))?
            }
            None => match &param.default {
                Some(default) => default.clone(),
                None => {
                    return Err(BindError::new(
                        ErrorCode::MissingParam,
                        format!("the parameter `{}: {}` has no value", param.name, param.ty),
                    )
                    .expected(std::slice::from_ref(&param.ty))
                    .detail("declaration", declaration_detail(declared)));
                }
            },
        };
        bound.push((param.name.clone(), value));
    }
    Ok(bound)
}

fn ty_holds_bool(ty: &ParamType) -> bool {
    ty.alternatives().iter().any(|alternative| {
        matches!(
            alternative,
            ParamType::Scalar(ScalarType::Bool | ScalarType::Any)
        )
    })
}

fn declaration_detail(declared: &[BoundParam]) -> serde_json::Value {
    serde_json::json!(listing(declared))
}

/// The surface expression a JSON value stands for, for the type.
fn param_expr(json: &Json, ty: &ParamType) -> Option<Expr> {
    match json {
        Json::Null => Some(Expr::Null { span: None }),
        Json::Bool(v) => Some(Expr::Bool { v: *v, span: None }),
        Json::Number(number) => number
            .as_i64()
            .map(|v| Expr::Int { v, span: None })
            .or_else(|| number.as_f64().map(|v| Expr::Num { v, span: None })),
        Json::String(text) => string_expr(text, ty),
        Json::Array(items) => {
            let inner = match ty {
                ParamType::List(inner) => inner.as_ref().clone(),
                _ => ParamType::Scalar(ScalarType::Any),
            };
            let items: Option<Vec<Expr>> =
                items.iter().map(|item| param_expr(item, &inner)).collect();
            items.map(|items| Expr::List { items, span: None })
        }
        Json::Object(members) => {
            let inner = match ty {
                ParamType::Map(inner) => inner.as_ref().clone(),
                _ => ParamType::Scalar(ScalarType::Any),
            };
            let entries: Option<Vec<MapEntry>> = members
                .iter()
                .map(|(key, value)| {
                    param_expr(value, &inner).map(|v| MapEntry {
                        k: key.clone(),
                        v,
                        span: None,
                    })
                })
                .collect();
            entries.map(|entries| Expr::Map {
                entries,
                span: None,
            })
        }
    }
}

/// A text from the command line: text for the types that hold text, a
/// literal for the others (`7`, `10mb`, `5s`, `true`).
fn string_expr(text: &str, ty: &ParamType) -> Option<Expr> {
    let holds_text = ty.alternatives().iter().any(|alternative| {
        matches!(
            alternative,
            ParamType::Scalar(
                ScalarType::Text
                    | ScalarType::Path
                    | ScalarType::Glob
                    | ScalarType::Key
                    | ScalarType::Instant
                    | ScalarType::Any
            )
        )
    });
    let enumerated = ty
        .alternatives()
        .iter()
        .any(|alternative| matches!(alternative, ParamType::Enum(_)));
    if holds_text {
        return Some(Expr::Text {
            v: text.to_owned(),
            span: None,
        });
    }
    if enumerated {
        return Some(Expr::Ident {
            v: text.to_owned(),
            span: None,
        });
    }
    literal_expr(text)
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
