// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-bind::fieldcheck
// Responsibility:   Check of the use of record fields in the condition of a construct.
// Dependencies:     nurt_contracts, nurt_types
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Check of the use of record fields in the condition of a construct.
//!
//! A field of the loop record whose type the output record of the source
//! command declares is checked at binding: a use that no value of that type
//! can satisfy is `E_TYPE`. A field of a type that depends on the data
//! (`any`, `json`) has no declared type and is checked when the condition is
//! evaluated.

use nurt_contracts::ast::{BinOp, UnOp};
use nurt_contracts::bound::BoundExpr;
use nurt_types::ValueType;

use crate::fields::FieldTypes;

/// What a value is for the purpose of the check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Class {
    Number,
    Measure,
    Text,
    Bool,
    Other,
}

fn class_of_type(ty: ValueType) -> Class {
    match ty {
        ValueType::Int | ValueType::Number => Class::Number,
        ValueType::Size | ValueType::Duration => Class::Measure,
        ValueType::Text
        | ValueType::Path
        | ValueType::Key
        | ValueType::Glob
        | ValueType::Regex
        | ValueType::Enum => Class::Text,
        ValueType::Bool => Class::Bool,
        _ => Class::Other,
    }
}

/// A field of a loop variable with its declared type.
struct Typed {
    shown: String,
    ty: ValueType,
}

/// The scope of the check: loop variables and the types of their fields.
pub(crate) struct Scope<'a> {
    pub(crate) vars: &'a [(String, FieldTypes)],
}

impl Scope<'_> {
    fn field(&self, expr: &BoundExpr) -> Option<Typed> {
        let BoundExpr::Ref(reference) = expr else {
            return None;
        };
        let [field] = reference.fields.as_slice() else {
            return None;
        };
        let (_, types) = self
            .vars
            .iter()
            .rev()
            .find(|(name, _)| name == &reference.name)?;
        let ty = *types.get(field.as_str())?;
        (ty != ValueType::Null).then(|| Typed {
            shown: format!("{}.{field}", reference.name),
            ty,
        })
    }

    fn class(&self, expr: &BoundExpr) -> Option<Class> {
        if let Some(field) = self.field(expr) {
            return Some(class_of_type(field.ty));
        }
        match expr {
            BoundExpr::Int { .. } | BoundExpr::Num { .. } => Some(Class::Number),
            BoundExpr::Size { .. } | BoundExpr::Dur { .. } => Some(Class::Measure),
            BoundExpr::Text { .. }
            | BoundExpr::Path { .. }
            | BoundExpr::Glob { .. }
            | BoundExpr::Key { .. }
            | BoundExpr::Enum { .. }
            | BoundExpr::Regex { .. } => Some(Class::Text),
            BoundExpr::Bool { .. } => Some(Class::Bool),
            _ => None,
        }
    }

    /// Returns the description of the first misuse of a typed field.
    pub(crate) fn misuse(&self, expr: &BoundExpr) -> Option<String> {
        match expr {
            BoundExpr::Un { op: UnOp::Not, x } => self.needs_bool(x).or_else(|| self.misuse(x)),
            BoundExpr::Bin { op, l, r } => self
                .operands(*op, l, r)
                .or_else(|| self.misuse(l))
                .or_else(|| self.misuse(r)),
            BoundExpr::Call { args, .. } => args.iter().find_map(|arg| self.misuse(arg)),
            _ => None,
        }
    }

    fn needs_bool(&self, operand: &BoundExpr) -> Option<String> {
        let field = self.field(operand)?;
        (field.ty != ValueType::Bool).then(|| {
            format!(
                "`{}` is `{}`, and a logical operator takes `bool`",
                field.shown,
                field.ty.as_str()
            )
        })
    }

    fn operands(&self, op: BinOp, l: &BoundExpr, r: &BoundExpr) -> Option<String> {
        match op {
            BinOp::And | BinOp::Or => self.needs_bool(l).or_else(|| self.needs_bool(r)),
            BinOp::Match | BinOp::NotMatch if matches!(r, BoundExpr::Text { .. }) => Some(format!(
                "`{}` takes a regular expression on the right; write /pattern/ or regex(\"pattern\")",
                op.as_str()
            )),
            BinOp::Match | BinOp::NotMatch => [l, r].into_iter().find_map(|side| {
                let field = self.field(side)?;
                (class_of_type(field.ty) != Class::Text).then(|| {
                    format!(
                        "`{}` is `{}`, and `{}` takes text",
                        field.shown,
                        field.ty.as_str(),
                        op.as_str()
                    )
                })
            }),
            BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div => {
                [l, r].into_iter().find_map(|side| {
                    let field = self.field(side)?;
                    let fits = match class_of_type(field.ty) {
                        Class::Number | Class::Measure => true,
                        Class::Text => op == BinOp::Add && field.ty == ValueType::Text,
                        _ => {
                            (matches!(field.ty, ValueType::Instant | ValueType::Date)
                                && matches!(op, BinOp::Add | BinOp::Sub))
                                || (op == BinOp::Add && field.ty == ValueType::List)
                        }
                    };
                    (!fits).then(|| {
                        format!(
                            "`{}` is `{}`, and `{}` does not apply to it",
                            field.shown,
                            field.ty.as_str(),
                            op.as_str()
                        )
                    })
                })
            }
            BinOp::In => {
                let field = self.field(r)?;
                let fits = matches!(
                    field.ty,
                    ValueType::List | ValueType::Map | ValueType::Range | ValueType::Text
                );
                (!fits).then(|| {
                    format!(
                        "`{}` is `{}`, and `in` takes a list, a map, a range or text on the right",
                        field.shown,
                        field.ty.as_str()
                    )
                })
            }
            BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge | BinOp::Eq | BinOp::Ne => {
                self.comparison(op, l, r)
            }
            _ => None,
        }
    }

    fn comparison(&self, op: BinOp, l: &BoundExpr, r: &BoundExpr) -> Option<String> {
        let ordered = !matches!(op, BinOp::Eq | BinOp::Ne);
        for side in [l, r] {
            if let Some(field) = self.field(side)
                && ordered
                && matches!(
                    field.ty,
                    ValueType::Bool | ValueType::Map | ValueType::Record
                )
            {
                return Some(format!(
                    "`{}` is `{}`, and `{}` needs values that have an order",
                    field.shown,
                    field.ty.as_str(),
                    op.as_str()
                ));
            }
        }
        let (a, b) = (self.class(l)?, self.class(r)?);
        let one_field = self.field(l).or_else(|| self.field(r))?;
        let fits = a == Class::Other
            || b == Class::Other
            || a == b
            || matches!(
                (a, b),
                (Class::Text, Class::Measure) | (Class::Measure, Class::Text)
            );
        (!fits).then(|| {
            format!(
                "`{}` is `{}`, and `{}` compares it with a value of another type",
                one_field.shown,
                one_field.ty.as_str(),
                op.as_str()
            )
        })
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
