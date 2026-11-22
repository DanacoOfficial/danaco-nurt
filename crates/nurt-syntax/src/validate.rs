// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Invariants of syntax tree documents beyond the Rust types.
//!
//! The types of `nurt_contracts::ast` and `nurt_contracts::bound` accept some
//! documents that the published schemas reject (identifier patterns, integer
//! ranges, regular expression flags, an empty program) and cannot express the
//! rules that a JSON Schema cannot express either: unique map keys and
//! parameter names, the position of `spread`, the order of phases in a bound
//! pipeline and the agreement of `effect` with `phase`. [`validate_surface`]
//! and [`validate_bound`] check these rules on a typed tree;
//! [`check_members`] finds members of a received document that the typed tree
//! does not keep. Every failure has the code `E_AST_SCHEMA` and the JSON
//! Pointer (RFC 6901) of the offending value.
//!
//! The nesting depth of a document is the number of JSON values on the path
//! from the root to its deepest value, the root being at depth 1. A document
//! deeper than [`MAX_DEPTH`] is rejected before any deeper node is visited.

// `SyntaxError` is the error type shared by every module of the crate; its
// layout is defined with the parser diagnostics, not per module.
#![allow(clippy::result_large_err)]

use std::collections::HashSet;

use nurt_contracts::ast::{
    AstDocument, Expr, Head, KeySeg, Meta, Pipeline, Ref, Span, Stage, Statement,
};
use nurt_contracts::bound::{
    BoundDocument, BoundExpr, BoundHead, BoundPipeline, BoundRef, BoundStage, BoundStatement, Phase,
};
use nurt_contracts::kinds::EffectKind;
use serde_json::Value;

use crate::error::SyntaxError;

/// Largest admissible nesting depth of a tree document.
pub const MAX_DEPTH: usize = 128;

/// Largest magnitude of an integer in a tree document, 2^53 - 1: the range
/// that canonical JSON (RFC 8785) represents exactly.
pub const MAX_SAFE_INTEGER: i64 = (1 << 53) - 1;

/// Version member values of a supported major version: `1.MINOR`.
const MAJOR_PREFIX: &str = "1.";

/// Characters of regular expression flags in canonical order.
const REGEX_FLAGS: &str = "imsx";

/// Largest number of segments of a bound command name: a group and a command.
const MAX_BOUND_NAME_SEGMENTS: usize = 2;

/// Members omitted by the typed tree when they hold their default value.
const NORMALIZED_MEMBERS: [(&str, NormalizedDefault); 3] = [
    ("fields", NormalizedDefault::EmptyArray),
    ("waits", NormalizedDefault::False),
    ("default", NormalizedDefault::False),
];

const DEPTH_MESSAGE: &str = "nesting deeper than 128 levels; flatten the document";

type Checked = Result<(), SyntaxError>;

#[derive(Clone, Copy)]
enum NormalizedDefault {
    EmptyArray,
    False,
}

impl NormalizedDefault {
    fn matches(self, value: &Value) -> bool {
        match self {
            Self::EmptyArray => value.as_array().is_some_and(Vec::is_empty),
            Self::False => value == &Value::Bool(false),
        }
    }
}

/// Returns the error of a violated tree rule; [`SyntaxError::found`] holds the
/// JSON Pointer of the offending value, empty for the whole document.
pub(crate) fn violation(pointer: &str, message: impl Into<String>) -> SyntaxError {
    SyntaxError::ast_schema(message).with_found(pointer)
}

/// Escapes a member name for a JSON Pointer (RFC 6901, section 3).
fn escape_pointer_segment(name: &str) -> String {
    name.replace('~', "~0").replace('/', "~1")
}

/// Position of a value in a document: its JSON Pointer and its depth.
#[derive(Debug, Clone)]
struct Place {
    pointer: String,
    level: usize,
}

impl Place {
    fn root() -> Self {
        Self {
            pointer: String::new(),
            level: 1,
        }
    }

    fn member(&self, name: &str) -> Self {
        Self {
            pointer: format!("{}/{name}", self.pointer),
            level: self.level + 1,
        }
    }

    fn index(&self, index: usize) -> Self {
        Self {
            pointer: format!("{}/{index}", self.pointer),
            level: self.level + 1,
        }
    }

    fn fail(&self, message: impl Into<String>) -> SyntaxError {
        violation(&self.pointer, message)
    }

    fn ensure(&self, condition: bool, message: &str) -> Checked {
        if condition {
            Ok(())
        } else {
            Err(self.fail(message))
        }
    }

    /// Checks the depth of an object at this place whose members are scalars,
    /// arrays checked separately, or a span (an array of integers).
    fn object(&self, span: Option<&Span>) -> Checked {
        let below = if span.is_some() { 2 } else { 1 };
        self.ensure(self.level + below <= MAX_DEPTH, DEPTH_MESSAGE)
    }
}

/// Validates a surface tree.
///
/// # Errors
///
/// Returns `E_AST_SCHEMA` with the JSON Pointer of the first violation in
/// document order.
///
/// # Examples
///
/// ```
/// use nurt_contracts::ast::AstDocument;
/// use nurt_syntax::validate::validate_surface;
///
/// let document: AstDocument = serde_json::from_str(
///     r#"{"nurt":"1.0","program":{"statements":[{"t":"pipeline",
///         "head":{"t":"stage","name":"first","args":[{"value":{"t":"int","v":-1}}]},
///         "stages":[]}]}}"#,
/// )?;
/// assert!(validate_surface(&document).is_err());
/// # Ok::<(), serde_json::Error>(())
/// ```
pub fn validate_surface(document: &AstDocument) -> Result<(), SyntaxError> {
    let root = Place::root();
    version(&root.member("nurt"), &document.nurt)?;
    if let Some(meta) = &document.meta {
        free_form(&root.member("meta"), meta)?;
    }
    let statements = root.member("program").member("statements");
    statements.ensure(
        !document.program.statements.is_empty(),
        "a program must contain at least one statement",
    )?;
    for (index, statement) in document.program.statements.iter().enumerate() {
        surface_statement(&statements.index(index), statement)?;
    }
    Ok(())
}

fn surface_statement(place: &Place, statement: &Statement) -> Checked {
    match statement {
        Statement::Let(binding) => {
            place.object(binding.span.as_ref())?;
            identifier(&place.member("name"), &binding.name)?;
            surface_pipeline(&place.member("value"), &binding.value)
        }
        Statement::Pipeline(pipeline) => surface_pipeline(place, pipeline),
        _ => Err(place.fail("unsupported statement kind")),
    }
}

fn surface_pipeline(place: &Place, pipeline: &Pipeline) -> Checked {
    place.object(pipeline.span.as_ref())?;
    let head = place.member("head");
    match &pipeline.head {
        Head::Stage(stage) => surface_stage(&head, stage)?,
        Head::Ref(reference) => surface_ref(&head, reference)?,
        _ => return Err(head.fail("unsupported pipeline head kind")),
    }
    let stages = place.member("stages");
    for (index, stage) in pipeline.stages.iter().enumerate() {
        surface_stage(&stages.index(index), stage)?;
    }
    Ok(())
}

fn surface_stage(place: &Place, stage: &Stage) -> Checked {
    place.object(stage.span.as_ref())?;
    surface_command_name(&place.member("name"), &stage.name)?;
    let args = place.member("args");
    for (index, arg) in stage.args.iter().enumerate() {
        let arg_place = args.index(index);
        arg_place.object(arg.span.as_ref())?;
        if let Some(name) = &arg.name {
            identifier(&arg_place.member("name"), name)?;
        }
        surface_expr(&arg_place.member("value"), &arg.value, false)?;
    }
    Ok(())
}

fn surface_ref(place: &Place, reference: &Ref) -> Checked {
    place.object(reference.span.as_ref())?;
    reference_names(place, &reference.name, &reference.fields)
}

fn surface_expr(place: &Place, expr: &Expr, list_item: bool) -> Checked {
    match expr {
        Expr::Ref(reference) => surface_ref(place, reference),
        Expr::Text { span, .. } | Expr::Bool { span, .. } | Expr::Null { span } => {
            place.object(span.as_ref())
        }
        Expr::Ident { v, span } => {
            place.object(span.as_ref())?;
            identifier(&place.member("v"), v)
        }
        Expr::Regex { flags, span, .. } => {
            place.object(span.as_ref())?;
            regex_flags(&place.member("flags"), flags)
        }
        Expr::Int { v, span } => {
            place.object(span.as_ref())?;
            integer(&place.member("v"), *v, 0)
        }
        Expr::Num { v, span } => {
            place.object(span.as_ref())?;
            finite(&place.member("v"), *v)
        }
        Expr::Size { v, span } | Expr::Dur { v, span } => {
            place.object(span.as_ref())?;
            unsigned(&place.member("v"), *v)
        }
        Expr::Range { lo, hi, span } => {
            place.object(span.as_ref())?;
            range(place, *lo, *hi, 0)
        }
        Expr::Key { segs, span } => surface_key(place, segs, span.as_ref()),
        _ => surface_composite(place, expr, list_item),
    }
}

fn surface_composite(place: &Place, expr: &Expr, list_item: bool) -> Checked {
    match expr {
        Expr::List { items, span } => {
            place.object(span.as_ref())?;
            let items_place = place.member("items");
            items
                .iter()
                .enumerate()
                .try_for_each(|(index, item)| surface_expr(&items_place.index(index), item, true))
        }
        Expr::Map { entries, span } => {
            place.object(span.as_ref())?;
            let entries_place = place.member("entries");
            let mut keys = HashSet::new();
            for (index, entry) in entries.iter().enumerate() {
                let entry_place = entries_place.index(index);
                entry_place.object(None)?;
                unique(
                    &entry_place.member("k"),
                    &mut keys,
                    &entry.k,
                    MAP_KEY_MESSAGE,
                )?;
                surface_expr(&entry_place.member("v"), &entry.v, false)?;
            }
            Ok(())
        }
        Expr::Spread { x, span } => {
            place.object(span.as_ref())?;
            place.ensure(list_item, SPREAD_MESSAGE)?;
            surface_expr(&place.member("x"), x, false)
        }
        Expr::Sub { p, span } => {
            place.object(span.as_ref())?;
            surface_pipeline(&place.member("p"), p)
        }
        _ => surface_operation(place, expr),
    }
}

fn surface_operation(place: &Place, expr: &Expr) -> Checked {
    match expr {
        Expr::Field { name, of, span } => {
            place.object(span.as_ref())?;
            identifier(&place.member("name"), name)?;
            of.as_deref()
                .map_or(Ok(()), |of| surface_expr(&place.member("of"), of, false))
        }
        Expr::Call {
            function,
            args,
            span,
        } => {
            place.object(span.as_ref())?;
            identifier(&place.member("fn"), function)?;
            let args_place = place.member("args");
            args.iter()
                .enumerate()
                .try_for_each(|(index, arg)| surface_expr(&args_place.index(index), arg, false))
        }
        Expr::Un { x, span, .. } => {
            place.object(span.as_ref())?;
            surface_expr(&place.member("x"), x, false)
        }
        Expr::Bin { l, r, span, .. } => {
            place.object(span.as_ref())?;
            surface_expr(&place.member("l"), l, false)?;
            surface_expr(&place.member("r"), r, false)
        }
        _ => Err(place.fail("unsupported node kind")),
    }
}

fn surface_key(place: &Place, segs: &[KeySeg], span: Option<&Span>) -> Checked {
    place.object(span)?;
    let segs_place = place.member("segs");
    segs_place.ensure(!segs.is_empty(), "a key must have at least one segment")?;
    for (index, seg) in segs.iter().enumerate() {
        let seg_place = segs_place.index(index);
        match seg {
            KeySeg::Name(_) => {}
            KeySeg::Index(value) => integer(&seg_place, *value, -MAX_SAFE_INTEGER)?,
            KeySeg::Wild { .. } => seg_place.object(None)?,
            KeySeg::Filter { filter } => {
                seg_place.object(None)?;
                surface_expr(&seg_place.member("filter"), filter, false)?;
            }
            _ => return Err(seg_place.fail("unsupported key segment kind")),
        }
    }
    Ok(())
}

/// Validates a bound tree.
///
/// Besides the value rules shared with the surface tree, the function checks
/// the phases of every pipeline: the head stage has the phase `source`,
/// `mutate` or `run`; stages of the phases `transform` and `check` precede the
/// single stage of the phase `mutate` or `run`; stages of the phase `after`
/// follow it; a stage of the phase `verify` is the last one and follows the
/// effect; the pipeline of a `let` has no effect. Stages of every phase other
/// than `mutate` and `run` have the effect `read`; stages of these two phases
/// have another effect.
///
/// # Errors
///
/// Returns `E_AST_SCHEMA` with the JSON Pointer of the first violation in
/// document order.
pub fn validate_bound(document: &BoundDocument) -> Result<(), SyntaxError> {
    let root = Place::root();
    version(&root.member("nurt"), &document.nurt)?;
    version(&root.member("bound"), &document.bound)?;
    root.member("catalog_rev").ensure(
        !document.catalog_rev.is_empty(),
        "the catalog revision must not be empty",
    )?;
    root.member("corr_rev").ensure(
        !document.corr_rev.is_empty(),
        "the correction tables revision must not be empty",
    )?;
    if let Some(meta) = &document.meta {
        free_form(&root.member("meta"), meta)?;
    }
    let statements = root.member("program").member("statements");
    statements.ensure(
        !document.program.statements.is_empty(),
        "a program must contain at least one statement",
    )?;
    for (index, statement) in document.program.statements.iter().enumerate() {
        bound_statement(&statements.index(index), statement)?;
    }
    Ok(())
}

fn bound_statement(place: &Place, statement: &BoundStatement) -> Checked {
    match statement {
        BoundStatement::Let(binding) => {
            place.object(binding.span.as_ref())?;
            identifier(&place.member("name"), &binding.name)?;
            let value = place.member("value");
            let position = bound_pipeline(&value, &binding.value)?;
            value.ensure(
                position == Position::BeforeEffect,
                "the pipeline of a `let` must not contain an effect; move the effect to a pipeline statement",
            )
        }
        BoundStatement::Pipeline(pipeline) => bound_pipeline(place, pipeline).map(drop),
        _ => Err(place.fail("unsupported statement kind")),
    }
}

/// Position in a pipeline relative to its effect stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Position {
    BeforeEffect,
    AfterEffect,
    Verified,
}

fn bound_pipeline(place: &Place, pipeline: &BoundPipeline) -> Result<Position, SyntaxError> {
    place.object(pipeline.span.as_ref())?;
    let head = place.member("head");
    let mut position = match &pipeline.head {
        BoundHead::Stage(stage) => {
            bound_stage(&head, stage)?;
            match stage.phase {
                Phase::Source => Position::BeforeEffect,
                Phase::Mutate | Phase::Run => Position::AfterEffect,
                _ => {
                    return Err(head
                        .member("phase")
                        .fail("a head stage must have the phase `source`, `mutate` or `run`"));
                }
            }
        }
        BoundHead::Ref(reference) => {
            head.object(None)?;
            bound_ref(&head, reference)?;
            Position::BeforeEffect
        }
        _ => return Err(head.fail("unsupported pipeline head kind")),
    };
    let stages = place.member("stages");
    for (index, stage) in pipeline.stages.iter().enumerate() {
        let stage_place = stages.index(index);
        bound_stage(&stage_place, stage)?;
        position = next_position(&stage_place, position, stage.phase)?;
    }
    Ok(position)
}

fn next_position(place: &Place, position: Position, phase: Phase) -> Result<Position, SyntaxError> {
    match (position, phase) {
        (Position::BeforeEffect, Phase::Transform | Phase::Check) => Ok(Position::BeforeEffect),
        (Position::BeforeEffect, Phase::Mutate | Phase::Run) => Ok(Position::AfterEffect),
        (Position::AfterEffect, Phase::After) => Ok(Position::AfterEffect),
        (Position::AfterEffect, Phase::Verify) => Ok(Position::Verified),
        _ => Err(place.member("phase").fail(
            "the phase is not admissible at this position; a pipeline has at most one \
             `mutate` or `run` stage, `after` stages follow it and `verify` is the last stage",
        )),
    }
}

fn effect_matches_phase(stage: &BoundStage) -> bool {
    if stage.phase.is_effect() {
        stage.effect != EffectKind::Read
    } else {
        stage.effect == EffectKind::Read
    }
}

fn bound_stage(place: &Place, stage: &BoundStage) -> Checked {
    place.object(stage.span.as_ref())?;
    bound_command_name(&place.member("name"), &stage.name)?;
    place.member("effect").ensure(
        effect_matches_phase(stage),
        "the effect does not match the phase; only `mutate` and `run` stages have an effect other than `read`",
    )?;
    let args = place.member("args");
    let mut names = HashSet::new();
    for (index, arg) in stage.args.iter().enumerate() {
        let arg_place = args.index(index);
        arg_place.object(arg.span.as_ref())?;
        let name = arg_place.member("name");
        identifier(&name, &arg.name)?;
        unique(
            &name,
            &mut names,
            &arg.name,
            "duplicate parameter; keep a single argument per parameter",
        )?;
        bound_expr(&arg_place.member("value"), &arg.value, false)?;
    }
    Ok(())
}

fn bound_ref(place: &Place, reference: &BoundRef) -> Checked {
    reference_names(place, &reference.name, &reference.fields)
}

fn bound_expr(place: &Place, expr: &BoundExpr, list_item: bool) -> Checked {
    place.object(None)?;
    match expr {
        BoundExpr::Null
        | BoundExpr::Bool { .. }
        | BoundExpr::Text { .. }
        | BoundExpr::Glob { .. }
        | BoundExpr::Key { .. }
        | BoundExpr::Enum { .. } => Ok(()),
        BoundExpr::Int { v } => integer(&place.member("v"), *v, -MAX_SAFE_INTEGER),
        BoundExpr::Num { v } => finite(&place.member("v"), *v),
        BoundExpr::Size { v } | BoundExpr::Dur { v } => unsigned(&place.member("v"), *v),
        BoundExpr::Regex { flags, .. } => regex_flags(&place.member("flags"), flags),
        BoundExpr::Range { lo, hi } => range(place, *lo, *hi, -MAX_SAFE_INTEGER),
        BoundExpr::Path { v } => place.member("v").ensure(
            is_canonical_path(v),
            "a path must be absolute and canonical: `/` separators, no empty, `.` or `..` segments",
        ),
        BoundExpr::Instant { v } => place.member("v").ensure(
            is_rfc3339(v),
            "an instant must be an RFC 3339 date and time, for example `2026-01-31T12:00:00Z`",
        ),
        BoundExpr::Ref(reference) => bound_ref(place, reference),
        _ => bound_composite(place, expr, list_item),
    }
}

fn bound_composite(place: &Place, expr: &BoundExpr, list_item: bool) -> Checked {
    match expr {
        BoundExpr::List { items } => {
            let items_place = place.member("items");
            items
                .iter()
                .enumerate()
                .try_for_each(|(index, item)| bound_expr(&items_place.index(index), item, true))
        }
        BoundExpr::Map { entries } => {
            let entries_place = place.member("entries");
            let mut keys = HashSet::new();
            for (index, entry) in entries.iter().enumerate() {
                let entry_place = entries_place.index(index);
                entry_place.object(None)?;
                unique(
                    &entry_place.member("k"),
                    &mut keys,
                    &entry.k,
                    MAP_KEY_MESSAGE,
                )?;
                bound_expr(&entry_place.member("v"), &entry.v, false)?;
            }
            Ok(())
        }
        BoundExpr::Spread { x } => {
            place.ensure(list_item, SPREAD_MESSAGE)?;
            bound_expr(&place.member("x"), x, false)
        }
        BoundExpr::Sub { p } => bound_pipeline(&place.member("p"), p).map(drop),
        _ => bound_operation(place, expr),
    }
}

fn bound_operation(place: &Place, expr: &BoundExpr) -> Checked {
    match expr {
        BoundExpr::Field { name, of } => {
            identifier(&place.member("name"), name)?;
            of.as_deref()
                .map_or(Ok(()), |of| bound_expr(&place.member("of"), of, false))
        }
        BoundExpr::Call { function, args } => {
            identifier(&place.member("fn"), function)?;
            let args_place = place.member("args");
            args.iter()
                .enumerate()
                .try_for_each(|(index, arg)| bound_expr(&args_place.index(index), arg, false))
        }
        BoundExpr::Un { x, .. } => bound_expr(&place.member("x"), x, false),
        BoundExpr::Bin { l, r, .. } => {
            bound_expr(&place.member("l"), l, false)?;
            bound_expr(&place.member("r"), r, false)
        }
        _ => Err(place.fail("unsupported node kind")),
    }
}

const MAP_KEY_MESSAGE: &str = "duplicate map key; keep a single entry per key";

const SPREAD_MESSAGE: &str = "a spread is admissible only as a list element";

fn reference_names(place: &Place, name: &str, fields: &[String]) -> Checked {
    identifier(&place.member("name"), name)?;
    let fields_place = place.member("fields");
    for (index, field) in fields.iter().enumerate() {
        identifier(&fields_place.index(index), field)?;
    }
    Ok(())
}

fn version(place: &Place, text: &str) -> Checked {
    let supported = text.strip_prefix(MAJOR_PREFIX).is_some_and(|minor| {
        minor == "0"
            || (!minor.starts_with('0')
                && !minor.is_empty()
                && minor.bytes().all(|byte| byte.is_ascii_digit()))
    });
    place.ensure(
        supported,
        "the version must have the form `1.MINOR`, for example `1.0`",
    )
}

fn free_form(place: &Place, meta: &Meta) -> Checked {
    let mut pending: Vec<(&Value, usize)> = meta
        .values()
        .map(|value| (value, place.level + 1))
        .collect();
    while let Some((value, level)) = pending.pop() {
        place.ensure(level <= MAX_DEPTH, DEPTH_MESSAGE)?;
        match value {
            Value::Object(members) => {
                pending.extend(members.values().map(|member| (member, level + 1)));
            }
            Value::Array(items) => pending.extend(items.iter().map(|item| (item, level + 1))),
            _ => {}
        }
    }
    Ok(())
}

/// Returns `true` for an identifier: `[a-z][a-z0-9_]*`.
#[must_use]
pub fn is_identifier(text: &str) -> bool {
    let mut bytes = text.bytes();
    bytes.next().is_some_and(|first| first.is_ascii_lowercase())
        && bytes.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

fn identifier(place: &Place, text: &str) -> Checked {
    place.ensure(
        is_identifier(text),
        "an identifier must match `[a-z][a-z0-9_]*`",
    )
}

fn surface_command_name(place: &Place, name: &str) -> Checked {
    place.ensure(
        name.split('.').all(is_identifier),
        "a command name must be identifiers joined by `.`",
    )
}

fn bound_command_name(place: &Place, name: &str) -> Checked {
    place.ensure(
        name.split('.').count() <= MAX_BOUND_NAME_SEGMENTS && name.split('.').all(is_identifier),
        "a bound command name must be an identifier or a group and an identifier joined by `.`",
    )
}

fn regex_flags(place: &Place, flags: &str) -> Checked {
    let mut remaining = REGEX_FLAGS;
    let canonical = flags.chars().all(|flag| match remaining.find(flag) {
        Some(position) => {
            remaining = &remaining[position + flag.len_utf8()..];
            true
        }
        None => false,
    });
    place.ensure(
        canonical,
        "regular expression flags must be drawn from `imsx`, each at most once, in this order",
    )
}

fn integer(place: &Place, value: i64, min: i64) -> Checked {
    place.ensure(
        (min..=MAX_SAFE_INTEGER).contains(&value),
        if min == 0 {
            "the integer must lie within 0..=2^53 - 1; write a negative number as `neg` applied to `int`"
        } else {
            "the integer must lie within -(2^53 - 1)..=2^53 - 1"
        },
    )
}

fn unsigned(place: &Place, value: u64) -> Checked {
    place.ensure(
        i64::try_from(value).is_ok_and(|value| value <= MAX_SAFE_INTEGER),
        "the integer must not exceed 2^53 - 1",
    )
}

fn finite(place: &Place, value: f64) -> Checked {
    place.ensure(
        value.is_finite(),
        "the number must be finite; NaN and infinities are not admissible",
    )
}

fn range(place: &Place, lo: Option<i64>, hi: Option<i64>, min: i64) -> Checked {
    if let Some(lo) = lo {
        integer(&place.member("lo"), lo, min)?;
    }
    if let Some(hi) = hi {
        integer(&place.member("hi"), hi, min)?;
    }
    Ok(())
}

fn unique<'a>(place: &Place, seen: &mut HashSet<&'a str>, key: &'a str, message: &str) -> Checked {
    place.ensure(seen.insert(key), message)
}

/// Returns `true` for an absolute path in the canonical form of a bound tree.
///
/// Admissible forms are `/a/b` (POSIX), `C:/a/b` (drive letter in upper case),
/// `//server/share/a` (UNC) and `scheme:/a/b` (a lower-case provider name of
/// at least two letters). Backslashes, NUL, empty segments, `.` and `..`
/// segments and a trailing `/` other than the root are not admissible.
///
/// # Examples
///
/// ```
/// use nurt_syntax::validate::is_canonical_path;
///
/// assert!(is_canonical_path("/srv/app"));
/// assert!(is_canonical_path("C:/Users"));
/// assert!(!is_canonical_path("src/app"));
/// assert!(!is_canonical_path("/srv/../etc"));
/// ```
#[must_use]
pub fn is_canonical_path(path: &str) -> bool {
    if path.contains(['\\', '\0']) {
        return false;
    }
    let rest = if let Some(unc) = path.strip_prefix("//") {
        let mut parts = unc.splitn(3, '/');
        let server = parts.next().unwrap_or_default();
        let share = parts.next().unwrap_or_default();
        if server.is_empty() || share.is_empty() {
            return false;
        }
        match parts.next() {
            Some(tail) => format!("/{tail}"),
            None => String::new(),
        }
    } else if path.starts_with('/') {
        path.to_owned()
    } else if let Some((prefix, tail)) = path.split_once(':') {
        let drive = prefix.len() == 1 && prefix.bytes().all(|byte| byte.is_ascii_uppercase());
        let provider = prefix.len() >= 2 && prefix.bytes().all(|byte| byte.is_ascii_lowercase());
        if !(drive || provider) || !tail.starts_with('/') {
            return false;
        }
        tail.to_owned()
    } else {
        return false;
    };
    if rest.is_empty() || rest == "/" {
        return true;
    }
    rest.split('/')
        .skip(1)
        .all(|segment| !segment.is_empty() && segment != "." && segment != "..")
}

/// Layout of the date and time of an RFC 3339 instant; `d` stands for a digit.
const INSTANT_LAYOUT: &[u8] = b"dddd-dd-ddTdd:dd:dd";

/// Layout of a numeric UTC offset after its sign.
const OFFSET_LAYOUT: &[u8] = b"dd:dd";

/// Number of months in a year.
const MONTHS: u32 = 12;

/// Number of hours in a day.
const HOURS: u32 = 24;

/// Number of minutes in an hour.
const MINUTES: u32 = 60;

/// Largest second of a minute; RFC 3339 admits the leap second 60.
const MAX_SECOND: u32 = 60;

fn layout_matches(text: &str, layout: &[u8]) -> bool {
    text.len() == layout.len()
        && text
            .bytes()
            .zip(layout)
            .all(|(byte, &expected)| match expected {
                b'd' => byte.is_ascii_digit(),
                literal => byte == literal,
            })
}

fn number(text: &str, range: std::ops::Range<usize>) -> u32 {
    text.get(range)
        .and_then(|digits| digits.parse().ok())
        .unwrap_or_default()
}

fn days_in_month(year: u32, month: u32) -> u32 {
    const DAYS: [u32; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let leap = (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400);
    let index = usize::try_from(month.saturating_sub(1)).unwrap_or_default();
    match DAYS.get(index) {
        Some(28) if leap => 29,
        Some(days) => *days,
        None => 0,
    }
}

/// Returns `true` for an RFC 3339 date and time (`date-time` of section 5.6):
/// `YYYY-MM-DDTHH:MM:SS`, an optional fraction of at least one digit and the
/// offset `Z` or `±HH:MM`. The date must exist in the calendar.
///
/// # Examples
///
/// ```
/// use nurt_syntax::validate::is_rfc3339;
///
/// assert!(is_rfc3339("2026-02-28T23:59:59.5Z"));
/// assert!(is_rfc3339("2024-02-29T00:00:00+01:00"));
/// assert!(!is_rfc3339("2026-02-29T00:00:00Z"));
/// assert!(!is_rfc3339("2026-01-01 00:00:00Z"));
/// ```
#[must_use]
pub fn is_rfc3339(text: &str) -> bool {
    let Some((date_time, rest)) = text.split_at_checked(INSTANT_LAYOUT.len()) else {
        return false;
    };
    if !layout_matches(date_time, INSTANT_LAYOUT) {
        return false;
    }
    let (year, month, day) = (
        number(date_time, 0..4),
        number(date_time, 5..7),
        number(date_time, 8..10),
    );
    let (hour, minute, second) = (
        number(date_time, 11..13),
        number(date_time, 14..16),
        number(date_time, 17..19),
    );
    let valid_date =
        (1..=MONTHS).contains(&month) && (1..=days_in_month(year, month)).contains(&day);
    let valid_time = hour < HOURS && minute < MINUTES && second <= MAX_SECOND;
    let offset = match rest.strip_prefix('.') {
        Some(fraction) => {
            let digits = fraction.bytes().take_while(u8::is_ascii_digit).count();
            if digits == 0 {
                return false;
            }
            &fraction[digits..]
        }
        None => rest,
    };
    let valid_offset = offset == "Z"
        || offset.strip_prefix(['+', '-']).is_some_and(|numeric| {
            layout_matches(numeric, OFFSET_LAYOUT)
                && number(numeric, 0..2) < HOURS
                && number(numeric, 3..5) < MINUTES
        });
    valid_date && valid_time && valid_offset
}

/// Checks that every member of a received document is kept by its typed form.
///
/// `received` is the JSON value of a document as decoded from its encoding;
/// `typed` is the JSON value produced by serializing the typed tree decoded
/// from it. A member present in `received` and absent from `typed` at the same
/// position is unknown to the format, unless it is a member that the typed
/// tree omits when it holds its default value: `"fields": []`,
/// `"waits": false` and `"default": false`. The check rejects, for example,
/// the member `effect` in a stage of a surface tree, which the types ignore
/// but the published schema forbids.
///
/// # Errors
///
/// Returns `E_AST_SCHEMA` with the JSON Pointer of the first unknown member.
///
/// # Examples
///
/// ```
/// use nurt_contracts::ast::AstDocument;
/// use nurt_syntax::validate::check_members;
///
/// let received: serde_json::Value = serde_json::from_str(
///     r#"{"nurt":"1.0","program":{"statements":[{"t":"pipeline",
///         "head":{"t":"stage","name":"count","args":[],"effect":"read"},"stages":[]}]}}"#,
/// )?;
/// let document: AstDocument = serde_json::from_value(received.clone())?;
/// let typed = serde_json::to_value(&document)?;
/// assert!(check_members(&received, &typed).is_err());
/// # Ok::<(), serde_json::Error>(())
/// ```
pub fn check_members(received: &Value, typed: &Value) -> Result<(), SyntaxError> {
    let mut pending = vec![(received, typed, String::new())];
    while let Some((received, typed, pointer)) = pending.pop() {
        match (received, typed) {
            (Value::Object(members), Value::Object(kept)) => {
                for (name, value) in members {
                    let member = format!("{pointer}/{}", escape_pointer_segment(name));
                    match kept.get(name) {
                        Some(kept_value) => pending.push((value, kept_value, member)),
                        None if is_normalized_default(name, value) => {}
                        None => {
                            return Err(violation(
                                &member,
                                format!("unknown member `{name}`; remove it"),
                            ));
                        }
                    }
                }
            }
            (Value::Array(items), Value::Array(kept)) => {
                for (index, (item, kept_item)) in items.iter().zip(kept).enumerate() {
                    pending.push((item, kept_item, format!("{pointer}/{index}")));
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn is_normalized_default(name: &str, value: &Value) -> bool {
    NORMALIZED_MEMBERS
        .iter()
        .any(|(member, default)| *member == name && default.matches(value))
}
