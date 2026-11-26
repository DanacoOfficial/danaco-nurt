// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Casting of surface expressions to the type of their place.
//!
//! A quoted text becomes a `path`, `glob`, `instant`, `key`, `size` or
//! `duration` in a place of that type; a bare identifier becomes an `enum`
//! value or a record `field`; an integer becomes the range `n..n` in a place
//! of type `range`; a single value becomes a one-element list in a place of
//! a list type; constant negation is folded. Expressions evaluated on
//! records keep their structure and their identifiers become fields of the
//! input records. With correction, a literal that does not have the form of
//! its place is corrected by the type rules before it is reported.

use nurt_contracts::ast::{Expr, KeySeg, MapEntry, Pipeline, Span, UnOp};
use nurt_contracts::bound::{BoundExpr, BoundMapEntry};
use nurt_contracts::error::ErrorCode;
use nurt_contracts::keypath::KeyPath;
use nurt_contracts::spec::{ParamType, ScalarType};

use crate::binder::{Binder, unsupported};
use crate::error::BindError;
use crate::literal::{duration_ms, instant, is_percent, size_bytes};
use crate::path::{PathHost, PathProblem, resolve};

/// The range written as the word `all`: every line, page or record.
const ALL_RANGE: &str = "all";

/// Parameter place of a value, for error messages.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Place<'p> {
    pub(crate) command: &'p str,
    pub(crate) param: &'p str,
    pub(crate) per_record: bool,
}

/// Failure of a cast: the kind of the expression does not fit the type, or
/// it fits and the value is not valid.
enum Cast {
    Mismatch,
    Failed(BindError),
}

type CastResult = Result<BoundExpr, Cast>;

impl From<BindError> for Cast {
    fn from(error: BindError) -> Self {
        Self::Failed(error)
    }
}

impl Binder<'_> {
    /// Casts a written value to the type of its parameter.
    pub(crate) fn value(
        &mut self,
        expr: &Expr,
        ty: &ParamType,
        place: Place<'_>,
    ) -> Result<BoundExpr, BindError> {
        self.cast(expr, ty, place).map_err(|failure| match failure {
            Cast::Failed(error) => error,
            Cast::Mismatch => self.mismatch(expr, ty, place),
        })
    }

    /// Type error of a value; a bare word in a place of type `text` comes
    /// with the program that writes it as a text.
    fn mismatch(&self, expr: &Expr, ty: &ParamType, place: Place<'_>) -> BindError {
        let error = type_error(expr, ty, place);
        let takes_text = ty
            .alternatives()
            .iter()
            .any(|alternative| matches!(alternative, ParamType::Scalar(ScalarType::Text)));
        match expr {
            Expr::Ident { span, .. } if takes_text => match self.program_quoting(*span) {
                Some(program) => error.fix(program),
                None => error,
            },
            _ => error,
        }
    }

    /// Casts an element of a list; a spread of a reference or of a list is
    /// kept as a spread.
    pub(crate) fn element(
        &mut self,
        expr: &Expr,
        ty: &ParamType,
        place: Place<'_>,
    ) -> Result<BoundExpr, BindError> {
        if let Expr::Spread { x, .. } = expr {
            let spread = match x.as_ref() {
                Expr::Ref(reference) => BoundExpr::Ref(self.reference(reference)?),
                Expr::List { .. } => {
                    let list = ParamType::List(Box::new(ty.clone()));
                    self.value(x, &list, place)?
                }
                other => return Err(type_error(other, ty, place)),
            };
            return Ok(BoundExpr::Spread {
                x: Box::new(spread),
            });
        }
        self.value(expr, ty, place)
    }

    fn cast(&mut self, expr: &Expr, ty: &ParamType, place: Place<'_>) -> CastResult {
        match expr {
            Expr::Ref(reference) => return Ok(BoundExpr::Ref(self.reference(reference)?)),
            Expr::Null { .. } => return Ok(BoundExpr::Null),
            _ => {}
        }
        if place.per_record && is_record_expression(expr) {
            return Ok(self.expression(expr)?);
        }
        match ty {
            ParamType::Scalar(scalar) => self.scalar(expr, *scalar, place),
            ParamType::Enum(values) => match expr {
                Expr::Ident { v, span } => Ok(self.enum_value(v, *span, values, place)?),
                _ => Err(Cast::Mismatch),
            },
            ParamType::List(inner) => self.list(expr, inner, place),
            ParamType::Map(inner) => self.map(expr, inner, place),
            ParamType::Union(alternatives) => self.union(expr, alternatives, place),
            _ => Err(Cast::Mismatch),
        }
    }

    /// Takes the first alternative that fits as written; with correction,
    /// and only when no alternative fits as written, the first alternative
    /// that fits after a correction. When none fits, the first alternative
    /// whose kind fits but whose value is not valid reports its error.
    fn union(&mut self, expr: &Expr, alternatives: &[ParamType], place: Place<'_>) -> CastResult {
        let exact = self.exactly(|binder| binder.first_fit(expr, alternatives, place));
        let failure = match exact {
            Ok(value) => return Ok(value),
            Err(failure) => failure,
        };
        if self.active().is_none() {
            return Err(failure);
        }
        match (self.first_fit(expr, alternatives, place), failure) {
            (Ok(value), _) => Ok(value),
            (Err(Cast::Failed(error)), Cast::Mismatch) => Err(Cast::Failed(error)),
            (Err(_), failure) => Err(failure),
        }
    }

    fn first_fit(
        &mut self,
        expr: &Expr,
        alternatives: &[ParamType],
        place: Place<'_>,
    ) -> CastResult {
        let mut failure = None;
        for alternative in alternatives {
            let mark = self.applied.len();
            match self.cast(expr, alternative, place) {
                Ok(value) => return Ok(value),
                Err(Cast::Failed(error)) => {
                    self.applied.truncate(mark);
                    failure.get_or_insert(error);
                }
                Err(Cast::Mismatch) => self.applied.truncate(mark),
            }
        }
        failure.map_or(Err(Cast::Mismatch), |error| Err(Cast::Failed(error)))
    }

    fn scalar(&mut self, expr: &Expr, scalar: ScalarType, place: Place<'_>) -> CastResult {
        let cast = match scalar {
            ScalarType::Any => Ok(self.literal(expr)?),
            ScalarType::Expression => Ok(self.expression(expr)?),
            ScalarType::Pipeline => self.pipeline_value(expr),
            ScalarType::Field => self.field(expr),
            ScalarType::NamedExpression => self.named_expression(expr),
            ScalarType::Text | ScalarType::Bytes => text(expr),
            ScalarType::Int => integer(expr),
            ScalarType::Number => number(expr),
            ScalarType::Bool => boolean(expr),
            ScalarType::Path => self.path(expr, place),
            ScalarType::Glob => glob(expr, place),
            ScalarType::Regex => regex(expr),
            ScalarType::Size => size(expr),
            ScalarType::Duration => duration(expr),
            ScalarType::Percent => percent(expr),
            ScalarType::Instant => self.instant(expr),
            ScalarType::Key => key(expr, place),
            ScalarType::Range => range(expr),
            _ => Err(Cast::Mismatch),
        };
        match cast {
            Err(Cast::Mismatch) => match self.typed_literal(expr, scalar, place) {
                Some(corrected) => Ok(corrected?),
                None => Err(Cast::Mismatch),
            },
            other => other,
        }
    }

    fn list(&mut self, expr: &Expr, inner: &ParamType, place: Place<'_>) -> CastResult {
        let Expr::List { items, .. } = expr else {
            return self
                .cast(expr, inner, place)
                .map(|value| BoundExpr::List { items: vec![value] });
        };
        let mut bound = Vec::with_capacity(items.len());
        for item in items {
            bound.push(self.element(item, inner, place)?);
        }
        Ok(BoundExpr::List { items: bound })
    }

    fn map(&mut self, expr: &Expr, inner: &ParamType, place: Place<'_>) -> CastResult {
        let Expr::Map { entries, .. } = expr else {
            return Err(Cast::Mismatch);
        };
        let mut bound: Vec<BoundMapEntry> = Vec::with_capacity(entries.len());
        for entry in entries {
            if bound.iter().any(|known| known.k == entry.k) {
                return Err(duplicate_key(entry).into());
            }
            let v = self.value(&entry.v, inner, place)?;
            bound.push(BoundMapEntry {
                k: entry.k.clone(),
                v,
            });
        }
        Ok(BoundExpr::Map { entries: bound })
    }

    fn path(&mut self, expr: &Expr, place: Place<'_>) -> CastResult {
        let (written, span, from_regex) = match expr {
            Expr::Text { v, span } => (v.clone(), *span, false),
            Expr::Regex { v, flags, span } => match self.regex_as_path(v, flags) {
                Some(written) => (written, *span, true),
                None => return Err(Cast::Mismatch),
            },
            _ => return Err(Cast::Mismatch),
        };
        let host = PathHost {
            os: self.context.host_os,
            base: self.context.base.as_deref(),
            home: self.context.home.as_deref(),
        };
        let path = resolve(&written, host)
            .map_err(|problem| Cast::Failed(path_error(&written, problem, span, place)))?;
        self.record_path(&written, &path, span, from_regex);
        Ok(BoundExpr::Path { v: path })
    }

    fn instant(&mut self, expr: &Expr) -> CastResult {
        match expr {
            Expr::Text { v, span } => match instant(v, &self.context.local_tz) {
                Some(value) => {
                    self.record_local_instant(v, &value, *span);
                    Ok(BoundExpr::Instant { v: value })
                }
                None => Err(Cast::Mismatch),
            },
            Expr::Call { .. } => Ok(self.expression(expr)?),
            _ => Err(Cast::Mismatch),
        }
    }

    fn pipeline_value(&mut self, expr: &Expr) -> CastResult {
        let pipeline = match expr {
            Expr::Sub { p, .. } => self.sub_pipeline(p)?,
            Expr::Ident { v, span } if self.catalog.command(v).is_some() => {
                let stage = nurt_contracts::ast::Stage {
                    name: v.clone(),
                    args: Vec::new(),
                    span: *span,
                };
                self.sub_pipeline(&Pipeline {
                    head: nurt_contracts::ast::Head::Stage(stage),
                    stages: Vec::new(),
                    span: *span,
                })?
            }
            _ => return Err(Cast::Mismatch),
        };
        Ok(BoundExpr::Sub { p: pipeline })
    }

    fn field(&mut self, expr: &Expr) -> CastResult {
        match expr {
            Expr::Ident { v, span } => Ok(BoundExpr::Field {
                name: self.field_name(v, *span)?,
                of: None,
            }),
            Expr::Field { .. } => Ok(self.expression(expr)?),
            _ => Err(Cast::Mismatch),
        }
    }

    fn named_expression(&mut self, expr: &Expr) -> CastResult {
        let Expr::Map { entries, .. } = expr else {
            return Err(Cast::Mismatch);
        };
        Ok(self.expression_map(entries)?)
    }

    /// Binds a value of type `any`: identifiers are enumeration values and
    /// lists and maps keep their elements as written.
    fn literal(&mut self, expr: &Expr) -> Result<BoundExpr, BindError> {
        match expr {
            Expr::Ident { v, .. } => Ok(BoundExpr::Enum { v: v.clone() }),
            Expr::List { items, .. } => {
                let mut bound = Vec::with_capacity(items.len());
                for item in items {
                    bound.push(match item {
                        Expr::Spread { x, .. } => BoundExpr::Spread {
                            x: Box::new(self.literal(x)?),
                        },
                        other => self.literal(other)?,
                    });
                }
                Ok(BoundExpr::List { items: bound })
            }
            Expr::Map { entries, .. } => {
                let mut bound: Vec<BoundMapEntry> = Vec::with_capacity(entries.len());
                for entry in entries {
                    if bound.iter().any(|known| known.k == entry.k) {
                        return Err(duplicate_key(entry));
                    }
                    bound.push(BoundMapEntry {
                        k: entry.k.clone(),
                        v: self.literal(&entry.v)?,
                    });
                }
                Ok(BoundExpr::Map { entries: bound })
            }
            Expr::Sub { p, .. } => Ok(BoundExpr::Sub {
                p: self.sub_pipeline(p)?,
            }),
            _ => self.expression(expr),
        }
    }

    /// Binds an expression evaluated on records: identifiers are fields of
    /// the current record and constant negation is folded.
    pub(crate) fn expression(&mut self, expr: &Expr) -> Result<BoundExpr, BindError> {
        if let Some(value) = constant(expr) {
            return Ok(value);
        }
        match expr {
            Expr::Ref(reference) => Ok(BoundExpr::Ref(self.reference(reference)?)),
            Expr::Ident { v, span } => Ok(BoundExpr::Field {
                name: self.field_name(v, *span)?,
                of: None,
            }),
            Expr::Key { segs, span } => key_segments(segs, *span),
            Expr::List { items, .. } => {
                let mut bound = Vec::with_capacity(items.len());
                for item in items {
                    bound.push(self.expression(item)?);
                }
                Ok(BoundExpr::List { items: bound })
            }
            Expr::Map { entries, .. } => self.expression_map(entries),
            _ => self.operation(expr),
        }
    }

    fn operation(&mut self, expr: &Expr) -> Result<BoundExpr, BindError> {
        match expr {
            Expr::Spread { x, .. } => Ok(BoundExpr::Spread {
                x: Box::new(self.expression(x)?),
            }),
            Expr::Field { name, of, .. } => Ok(BoundExpr::Field {
                name: name.clone(),
                of: match of {
                    Some(of) => Some(Box::new(self.expression(of)?)),
                    None => None,
                },
            }),
            Expr::Call {
                function,
                args,
                span,
            } => {
                let function = self.function_name(function, *span);
                let mut bound = Vec::with_capacity(args.len());
                for arg in args {
                    bound.push(self.expression(arg)?);
                }
                Ok(BoundExpr::Call {
                    function,
                    args: bound,
                })
            }
            Expr::Un { op, x, .. } => Ok(BoundExpr::Un {
                op: *op,
                x: Box::new(self.expression(x)?),
            }),
            Expr::Bin { op, l, r, .. } => Ok(BoundExpr::Bin {
                op: *op,
                l: Box::new(self.expression(l)?),
                r: Box::new(self.expression(r)?),
            }),
            Expr::Sub { span, .. } => Err(BindError::new(
                ErrorCode::Type,
                "a sub-pipeline is not an expression; use it as the argument of `verify` or of a parameter of type pipeline",
            )
            .at(*span)),
            _ => Err(unsupported("expression")),
        }
    }

    fn expression_map(&mut self, entries: &[MapEntry]) -> Result<BoundExpr, BindError> {
        let mut bound: Vec<BoundMapEntry> = Vec::with_capacity(entries.len());
        for entry in entries {
            if bound.iter().any(|known| known.k == entry.k) {
                return Err(duplicate_key(entry));
            }
            bound.push(BoundMapEntry {
                k: entry.k.clone(),
                v: self.expression(&entry.v)?,
            });
        }
        Ok(BoundExpr::Map { entries: bound })
    }
}

/// Returns the bound form of a literal constant, folding the negation of a
/// number.
fn constant(expr: &Expr) -> Option<BoundExpr> {
    match expr {
        Expr::Text { v, .. } => Some(BoundExpr::Text { v: v.clone() }),
        Expr::Regex { v, flags, .. } => Some(BoundExpr::Regex {
            v: v.clone(),
            flags: flags.clone(),
        }),
        Expr::Int { v, .. } => Some(BoundExpr::Int { v: *v }),
        Expr::Num { v, .. } => Some(BoundExpr::Num { v: *v }),
        Expr::Size { v, .. } => Some(BoundExpr::Size { v: *v }),
        Expr::Dur { v, .. } => Some(BoundExpr::Dur { v: *v }),
        Expr::Bool { v, .. } => Some(BoundExpr::Bool { v: *v }),
        Expr::Null { .. } => Some(BoundExpr::Null),
        Expr::Range { lo, hi, .. } => Some(BoundExpr::Range { lo: *lo, hi: *hi }),
        Expr::Un {
            op: UnOp::Neg, x, ..
        } => match x.as_ref() {
            Expr::Int { v, .. } => v.checked_neg().map(|v| BoundExpr::Int { v }),
            Expr::Num { v, .. } => Some(BoundExpr::Num { v: -v }),
            _ => None,
        },
        _ => None,
    }
}

fn is_record_expression(expr: &Expr) -> bool {
    matches!(
        expr,
        Expr::Bin { .. } | Expr::Field { .. } | Expr::Call { .. } | Expr::Un { op: UnOp::Not, .. }
    )
}

fn text(expr: &Expr) -> CastResult {
    match expr {
        Expr::Text { v, .. } => Ok(BoundExpr::Text { v: v.clone() }),
        _ => Err(Cast::Mismatch),
    }
}

fn integer(expr: &Expr) -> CastResult {
    match constant(expr) {
        Some(value @ BoundExpr::Int { .. }) => Ok(value),
        _ => Err(Cast::Mismatch),
    }
}

fn number(expr: &Expr) -> CastResult {
    match constant(expr) {
        Some(value @ (BoundExpr::Int { .. } | BoundExpr::Num { .. })) => Ok(value),
        _ => Err(Cast::Mismatch),
    }
}

fn boolean(expr: &Expr) -> CastResult {
    match expr {
        Expr::Bool { v, .. } => Ok(BoundExpr::Bool { v: *v }),
        _ => Err(Cast::Mismatch),
    }
}

fn glob(expr: &Expr, place: Place<'_>) -> CastResult {
    let Expr::Text { v, span } = expr else {
        return Err(Cast::Mismatch);
    };
    match globset::Glob::new(v) {
        Ok(_) if !v.is_empty() => Ok(BoundExpr::Glob { v: v.clone() }),
        Ok(_) => Err(Cast::Failed(
            BindError::new(
                ErrorCode::InvalidValue,
                format!(
                    "`{}` needs a non-empty pattern; write \"**\" for every name",
                    place.param
                ),
            )
            .at(*span),
        )),
        Err(error) => Err(Cast::Failed(
            BindError::new(
                ErrorCode::InvalidValue,
                format!("pattern `{v}` of `{}` is not valid: {error}", place.param),
            )
            .at(*span)
            .found(v.clone()),
        )),
    }
}

fn regex(expr: &Expr) -> CastResult {
    match expr {
        Expr::Regex { v, flags, .. } => Ok(BoundExpr::Regex {
            v: v.clone(),
            flags: flags.clone(),
        }),
        _ => Err(Cast::Mismatch),
    }
}

fn size(expr: &Expr) -> CastResult {
    match expr {
        Expr::Size { v, .. } => Ok(BoundExpr::Size { v: *v }),
        Expr::Text { v, .. } => size_bytes(v)
            .map(|v| BoundExpr::Size { v })
            .ok_or(Cast::Mismatch),
        _ => Err(Cast::Mismatch),
    }
}

fn duration(expr: &Expr) -> CastResult {
    match expr {
        Expr::Dur { v, .. } => Ok(BoundExpr::Dur { v: *v }),
        Expr::Text { v, .. } => duration_ms(v)
            .map(|v| BoundExpr::Dur { v })
            .ok_or(Cast::Mismatch),
        _ => Err(Cast::Mismatch),
    }
}

fn percent(expr: &Expr) -> CastResult {
    match expr {
        Expr::Text { v, .. } if is_percent(v) => Ok(BoundExpr::Text { v: v.clone() }),
        _ => Err(Cast::Mismatch),
    }
}

fn key(expr: &Expr, place: Place<'_>) -> CastResult {
    match expr {
        Expr::Text { v, span } => match KeyPath::parse(v) {
            Ok(path) => Ok(BoundExpr::Key {
                v: path.to_string(),
            }),
            Err(error) => Err(Cast::Failed(
                BindError::new(
                    ErrorCode::KeySyntax,
                    format!("key `{v}` of `{}` is not valid: {error}", place.param),
                )
                .at(*span)
                .found(v.clone()),
            )),
        },
        Expr::Key { segs, span } => Ok(key_segments(segs, *span)?),
        _ => Err(Cast::Mismatch),
    }
}

fn key_segments(segs: &[KeySeg], span: Option<Span>) -> Result<BoundExpr, BindError> {
    KeyPath::from_key_segs(segs)
        .map(|path| BoundExpr::Key {
            v: path.to_string(),
        })
        .map_err(|error| {
            BindError::new(
                ErrorCode::KeySyntax,
                format!("structured key is not valid: {error}"),
            )
            .at(span)
        })
}

fn range(expr: &Expr) -> CastResult {
    match expr {
        Expr::Ident { v, .. } if v == ALL_RANGE => Ok(BoundExpr::Range { lo: None, hi: None }),
        _ => match constant(expr) {
            Some(BoundExpr::Int { v }) => Ok(BoundExpr::Range {
                lo: Some(v),
                hi: Some(v),
            }),
            Some(value @ BoundExpr::Range { .. }) => Ok(value),
            _ => Err(Cast::Mismatch),
        },
    }
}

fn path_error(text: &str, problem: PathProblem, span: Option<Span>, place: Place<'_>) -> BindError {
    let error = match problem {
        PathProblem::Empty => BindError::new(
            ErrorCode::InvalidValue,
            format!("`{}` needs a path, not an empty text", place.param),
        ),
        PathProblem::Relative => BindError::new(
            ErrorCode::PathRelative,
            format!(
                "relative path `{text}` needs the call parameter base; write an absolute path or give base"
            ),
        ),
        PathProblem::NoHome => BindError::new(
            ErrorCode::PathRelative,
            format!(
                "path `{text}` starts with `~` and the home directory is not known; write an absolute path"
            ),
        ),
        PathProblem::Foreign(reason) => BindError::new(
            ErrorCode::PathForeign,
            format!("path `{text}` does not fit this system: {reason}"),
        ),
    };
    error.at(span).found(text.to_owned()).help("paths")
}

fn duplicate_key(entry: &MapEntry) -> BindError {
    BindError::new(
        ErrorCode::InvalidValue,
        format!(
            "key `{}` is given twice in the map; keep one entry",
            entry.k
        ),
    )
    .at(entry.span)
    .found(entry.k.clone())
}

fn type_error(expr: &Expr, ty: &ParamType, place: Place<'_>) -> BindError {
    let accepted: Vec<String> = ty.alternatives().iter().map(ToString::to_string).collect();
    BindError::new(
        ErrorCode::Type,
        format!(
            "`{}` of `{}` takes {ty}, not {}",
            place.param,
            place.command,
            expr_kind(expr)
        ),
    )
    .at(expr_span(expr))
    .expected(&accepted)
    .found(expr_kind(expr))
    .help(place.command.to_owned())
}

/// Returns the source range of an expression.
pub(crate) fn expr_span(expr: &Expr) -> Option<Span> {
    match expr {
        Expr::Ref(reference) => reference.span,
        Expr::Text { span, .. }
        | Expr::Ident { span, .. }
        | Expr::Regex { span, .. }
        | Expr::Int { span, .. }
        | Expr::Num { span, .. }
        | Expr::Size { span, .. }
        | Expr::Dur { span, .. }
        | Expr::Bool { span, .. }
        | Expr::Null { span }
        | Expr::Key { span, .. }
        | Expr::List { span, .. }
        | Expr::Map { span, .. }
        | Expr::Spread { span, .. }
        | Expr::Field { span, .. }
        | Expr::Call { span, .. }
        | Expr::Un { span, .. }
        | Expr::Bin { span, .. }
        | Expr::Range { span, .. }
        | Expr::Sub { span, .. } => *span,
        _ => None,
    }
}

fn expr_kind(expr: &Expr) -> &'static str {
    match expr {
        Expr::Ref(_) => "a reference",
        Expr::Text { .. } => "a text",
        Expr::Ident { .. } => "a bare word",
        Expr::Regex { .. } => "a regular expression",
        Expr::Int { .. } => "an integer",
        Expr::Num { .. } => "a number",
        Expr::Size { .. } => "a size",
        Expr::Dur { .. } => "a duration",
        Expr::Bool { .. } => "a boolean",
        Expr::Null { .. } => "null",
        Expr::Key { .. } => "a key",
        Expr::List { .. } => "a list",
        Expr::Map { .. } => "a map",
        Expr::Spread { .. } => "a spread",
        Expr::Field { .. } => "a field",
        Expr::Call { .. } => "a function call",
        Expr::Un { .. } | Expr::Bin { .. } => "an expression",
        Expr::Range { .. } => "a range",
        Expr::Sub { .. } => "a sub-pipeline",
        _ => "an unknown node",
    }
}
