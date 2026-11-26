// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Correction of names, values and literals during binding.
//!
//! When a [`Corrector`] is present, a name or a literal that does not fit
//! its place is corrected against the names and types expected there instead
//! of being reported: command names (`L01`, `L03`, `L04`, `L06`), parameter
//! names (`L02`, `L05`), enumeration values (`V01`, `T09`), record fields
//! (`V02`), expression functions (`V03`), references to `let` names (`V04`),
//! literals of typed places (`T01`–`T08`) and paths (`L19`, `L22`, `L23`).
//! Every correction is recorded with the change of the surface tree that
//! writes the corrected program. Without a corrector the same mistakes are
//! errors with the nearest names in `expected`.

use std::cell::OnceCell;

use nurt_contracts::ast::{Expr, Ref, Span, Stage};
use nurt_contracts::bound::BoundExpr;
use nurt_contracts::config::CorrectionConfig;
use nurt_contracts::error::ErrorCode;
use nurt_contracts::kinds::SyntaxVariant;
use nurt_contracts::report::TextPosition;
use nurt_contracts::spec::{ParamType, ScalarType, StageSpec};
use nurt_correct::distance::{EXPECTED_LIMIT, NameEntry, NameIndex, NameKind, Options, expected};
use nurt_correct::lexical::Rule;
use nurt_correct::lexical::paths::{EscapePlace, LiteralFix, normalize_path, read_escapes_at};
use nurt_correct::policy::{Class, MODE_PARAMS, is_effecting};
use nurt_correct::record::{Correction, CorrectionKind, RuleId};
use nurt_correct::tables::Tables;
use nurt_correct::types::{self, Outcome, TypeTarget, Written};
use nurt_correct::values::{self, FUNCTIONS, Match};
use nurt_syntax::print::print_expr;

use crate::binder::{Binder, Flow};
use crate::cast::Place;
use crate::catalog::Catalog;
use crate::error::BindError;
use crate::path::{PathHost, resolve};

/// Command whose `type` parameter selects the kind of the created entry.
const CREATE_COMMAND: &str = "create";

/// Parameter of `create` that selects the kind of the created entry.
const CREATE_TYPE: &str = "type";

/// Prefix of a reference to a `let` name.
const REF_SIGIL: &str = "@";

/// Tables and options of the correction made during binding.
#[derive(Debug)]
pub(crate) struct Corrector<'t> {
    pub(crate) tables: &'t Tables,
    pub(crate) options: Options,
    /// The program text the bound tree was read from; the spans of the tree
    /// refer to it.
    pub(crate) source: &'t str,
    commands: OnceCell<NameIndex>,
}

impl<'t> Corrector<'t> {
    /// Creates the corrector of one binding call for the tree read from
    /// `source`.
    pub(crate) fn new(tables: &'t Tables, config: &CorrectionConfig, source: &'t str) -> Self {
        Self {
            tables,
            options: Options::from(config),
            source,
            commands: OnceCell::new(),
        }
    }

    /// Command names of the catalog with their synonyms and the habit words
    /// of other shells, built on first use.
    fn commands(&self, catalog: &Catalog) -> &NameIndex {
        self.commands.get_or_init(|| {
            values::index(
                NameKind::Command,
                catalog
                    .commands()
                    .iter()
                    .map(|command| NameEntry::command(command.name.clone(), is_effecting(command))),
                self.tables,
            )
        })
    }
}

/// How the written literal of a path was read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PathReading {
    /// A text, as written.
    Text,
    /// A regular expression written in a path place (`L22`).
    Regex,
    /// A text with escapes, read as written (`L24`).
    Escapes,
}

/// Place of a text literal for the reading of its escapes: a regular
/// expression when the type takes one, else a path when it takes one.
fn escape_place(ty: &ParamType) -> EscapePlace {
    let takes = |scalar| {
        ty.alternatives()
            .iter()
            .any(|alternative| matches!(alternative, ParamType::Scalar(found) if *found == scalar))
    };
    if takes(ScalarType::Regex) {
        EscapePlace::Regex
    } else if takes(ScalarType::Path) {
        EscapePlace::Path
    } else {
        EscapePlace::Text
    }
}

/// Change of the surface tree that writes a correction into the program.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Change {
    /// The name of the stage with the span.
    Command(String),
    /// The name of the named argument with the span.
    Param(String),
    /// The expression with the span is replaced.
    Expr(Expr),
    /// The name of the reference with the span.
    Ref(String),
    /// The function of the call with the span.
    Function(String),
}

/// A change at the span of a surface node.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Edit {
    pub(crate) span: Option<Span>,
    pub(crate) change: Change,
}

/// A correction made during binding.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Applied {
    pub(crate) correction: Correction,
    pub(crate) edit: Edit,
    /// Other readings of a probable correction, as surface expressions.
    pub(crate) alternatives: Vec<Expr>,
    /// The corrected name is a command that changes the world or runs a
    /// program.
    pub(crate) effecting: bool,
    /// The text that the literal of the correction was read as before the
    /// correction: the regular expression written in a path place (`L22`),
    /// or the characters of a text with escapes (`L24`).
    pub(crate) reading: Option<String>,
}

impl Applied {
    pub(crate) fn new(correction: Correction, span: Option<Span>, change: Change) -> Self {
        Self {
            correction: located(correction, span, None),
            edit: Edit { span, change },
            alternatives: Vec::new(),
            effecting: false,
            reading: None,
        }
    }

    /// Limits the position of the correction to the name written at the
    /// start of its node.
    fn named(mut self, name: &str) -> Self {
        self.correction = located(self.correction.clone(), self.edit.span, name_length(name));
        self
    }
}

/// Sets the position of a correction from the span of its node; `length`
/// overrides the length of the span.
pub(crate) fn located(
    correction: Correction,
    span: Option<Span>,
    length: Option<u32>,
) -> Correction {
    let Some(span) = span else {
        return correction;
    };
    let length = length.or_else(|| {
        (span.line == span.end_line)
            .then(|| span.end_column.saturating_sub(span.column))
            .filter(|length| *length > 0)
    });
    correction.with_at(TextPosition {
        line: span.line,
        column: span.column,
        length,
    })
}

fn name_length(name: &str) -> Option<u32> {
    u32::try_from(name.chars().count()).ok()
}

/// Canonical text of a text literal.
pub(crate) fn quoted(text: &str) -> String {
    print_expr(
        &Expr::Text {
            v: text.to_owned(),
            span: None,
        },
        SyntaxVariant::A,
    )
}

/// Surface expression of a bound literal produced by a correction.
pub(crate) fn surface_expr(value: &BoundExpr) -> Option<Expr> {
    Some(match value {
        BoundExpr::Text { v } | BoundExpr::Path { v } | BoundExpr::Instant { v } => Expr::Text {
            v: v.clone(),
            span: None,
        },
        BoundExpr::Enum { v } => Expr::Ident {
            v: v.clone(),
            span: None,
        },
        BoundExpr::Int { v } => Expr::Int { v: *v, span: None },
        BoundExpr::Num { v } => Expr::Num { v: *v, span: None },
        BoundExpr::Size { v } => Expr::Size { v: *v, span: None },
        BoundExpr::Dur { v } => Expr::Dur { v: *v, span: None },
        BoundExpr::Bool { v } => Expr::Bool { v: *v, span: None },
        BoundExpr::Null => Expr::Null { span: None },
        BoundExpr::Range { lo, hi } => Expr::Range {
            lo: *lo,
            hi: *hi,
            span: None,
        },
        BoundExpr::Regex { v, flags } => Expr::Regex {
            v: v.clone(),
            flags: flags.clone(),
            span: None,
        },
        BoundExpr::Call { function, args } => Expr::Call {
            function: function.clone(),
            args: args.iter().map(surface_expr).collect::<Option<Vec<_>>>()?,
            span: None,
        },
        _ => return None,
    })
}

fn type_target(scalar: ScalarType) -> Option<TypeTarget> {
    Some(match scalar {
        ScalarType::Size => TypeTarget::Size,
        ScalarType::Duration => TypeTarget::Duration,
        ScalarType::Bool => TypeTarget::Bool,
        ScalarType::Int => TypeTarget::Int,
        ScalarType::Range => TypeTarget::Range,
        ScalarType::Regex => TypeTarget::Regex,
        ScalarType::Instant => TypeTarget::Instant,
        _ => return None,
    })
}

/// Whether the parameter selects the mode of an effect; its value is never
/// completed from a prefix.
fn is_mode(place: Place<'_>) -> bool {
    MODE_PARAMS.contains(&place.param)
        || (place.command == CREATE_COMMAND && place.param == CREATE_TYPE)
}

/// Joins the names of a tie and the nearest names, without repetition, up to
/// the limit of `expected`.
fn nearest_first(tie: Vec<String>, nearest: Vec<String>) -> Vec<String> {
    let mut names = tie;
    for name in nearest {
        if !names.contains(&name) {
            names.push(name);
        }
    }
    names.truncate(EXPECTED_LIMIT);
    names
}

impl<'a> Binder<'a> {
    /// Returns the corrector when corrections may be made at this point.
    pub(crate) fn active(&self) -> Option<&'a Corrector<'a>> {
        self.corrector.filter(|_| self.strict == 0)
    }

    /// Records a correction once; a node bound twice (a source form checked
    /// before its expansion) gives one correction.
    fn record(&mut self, applied: Applied) {
        if !self
            .applied
            .iter()
            .any(|known| known.correction == applied.correction && known.edit == applied.edit)
        {
            self.applied.push(applied);
        }
    }

    /// Runs `bind` with corrections suppressed: catalog defaults and the
    /// alternatives of a type tried for an exact fit are never corrected.
    pub(crate) fn exactly<T>(&mut self, bind: impl FnOnce(&mut Self) -> T) -> T {
        self.strict += 1;
        let result = bind(self);
        self.strict -= 1;
        result
    }

    /// Looks up the command of a stage, correcting its name.
    pub(crate) fn spec(&mut self, stage: &Stage) -> Result<&'a StageSpec, BindError> {
        if let Some(spec) = self.catalog.command(&stage.name) {
            return Ok(spec);
        }
        let mut tie = Vec::new();
        if let Some(corrector) = self.active() {
            let index = corrector.commands(self.catalog);
            match values::resolve(
                &stage.name,
                NameKind::Command,
                index,
                corrector.tables,
                &corrector.options,
            ) {
                Match::Corrected(found) => {
                    if let Some(spec) = self.catalog.command(&found.to) {
                        let mut applied = Applied::new(
                            found.correction(&stage.name),
                            stage.span,
                            Change::Command(found.to.clone()),
                        )
                        .named(&stage.name);
                        applied.effecting = found.effecting;
                        self.record(applied);
                        return Ok(spec);
                    }
                }
                Match::Tie(names) => tie = names,
                Match::Same | Match::Unknown => {}
            }
        }
        let index = NameIndex::new(
            self.catalog
                .commands()
                .iter()
                .map(|command| NameEntry::name(command.name.clone(), NameKind::Command)),
        );
        Err(BindError::new(
            ErrorCode::UnknownCommand,
            format!(
                "unknown command `{}`; use a command of the catalog",
                stage.name
            ),
        )
        .at(stage.span)
        .found(stage.name.clone())
        .expected(&nearest_first(
            tie,
            expected(&stage.name, &index, NameKind::Command, EXPECTED_LIMIT),
        ))
        .help("commands"))
    }

    /// Returns the stage with its unknown parameter names corrected among
    /// the accepted names, or `None` when no name changes.
    pub(crate) fn correct_params(
        &mut self,
        stage: &Stage,
        spec: &StageSpec,
        accepted: &[&str],
        collects_names: bool,
    ) -> Option<Stage> {
        let corrector = self.active()?;
        if collects_names {
            return None;
        }
        let mut corrected: Option<Stage> = None;
        for (index, arg) in stage.args.iter().enumerate() {
            let Some(name) = arg.name.as_deref() else {
                continue;
            };
            if spec.param(name).is_some() {
                continue;
            }
            let Match::Corrected(found) = values::resolve_in(
                name,
                NameKind::Param,
                accepted,
                corrector.tables,
                &corrector.options,
            ) else {
                continue;
            };
            if !accepted.contains(&found.to.as_str()) {
                continue;
            }
            let applied = Applied::new(
                found.correction(name),
                arg.span,
                Change::Param(found.to.clone()),
            )
            .named(name);
            self.record(applied);
            let stage = corrected.get_or_insert_with(|| stage.clone());
            if let Some(arg) = stage.args.get_mut(index) {
                arg.name = Some(found.to);
            }
        }
        corrected
    }

    /// Binds an identifier in an enumeration place, correcting the value.
    pub(crate) fn enum_value(
        &mut self,
        written: &str,
        span: Option<Span>,
        values: &[String],
        place: Place<'_>,
    ) -> Result<BoundExpr, BindError> {
        if values.iter().any(|value| value == written) {
            return Ok(BoundExpr::Enum {
                v: written.to_owned(),
            });
        }
        let names: Vec<&str> = values.iter().map(String::as_str).collect();
        let mut tie = Vec::new();
        if let Some(corrector) = self.active() {
            let found = if is_mode(place) {
                values::mode_value(written, &names, corrector.tables, &corrector.options)
            } else {
                values::value(written, &names, corrector.tables, &corrector.options)
            };
            match found {
                Match::Corrected(found) if names.contains(&found.to.as_str()) => {
                    let change = Change::Expr(Expr::Ident {
                        v: found.to.clone(),
                        span: None,
                    });
                    self.record(Applied::new(found.correction(written), span, change));
                    return Ok(BoundExpr::Enum { v: found.to });
                }
                Match::Tie(names) => tie = names,
                _ => {}
            }
        }
        let index = NameIndex::new(
            values
                .iter()
                .map(|value| NameEntry::name(value.clone(), NameKind::Value)),
        );
        Err(BindError::new(
            ErrorCode::UnknownValue,
            format!(
                "`{written}` is not a value of `{}`; write one of {}",
                place.param,
                values.join(", ")
            ),
        )
        .at(span)
        .found(written.to_owned())
        .expected(&nearest_first(
            tie,
            expected(written, &index, NameKind::Value, EXPECTED_LIMIT),
        ))
        .help(place.command.to_owned()))
    }

    /// Checks a field of the input records, correcting its name.
    pub(crate) fn field_name(
        &mut self,
        written: &str,
        span: Option<Span>,
    ) -> Result<String, BindError> {
        let Some(fields) = self.fields.clone() else {
            return Ok(written.to_owned());
        };
        if fields.contains(&written) {
            return Ok(written.to_owned());
        }
        let mut tie = Vec::new();
        if let Some(corrector) = self.active() {
            match values::field(written, &fields, corrector.tables, &corrector.options) {
                Match::Corrected(found) if fields.contains(&found.to.as_str()) => {
                    let change = Change::Expr(Expr::Ident {
                        v: found.to.clone(),
                        span: None,
                    });
                    self.record(Applied::new(found.correction(written), span, change));
                    return Ok(found.to);
                }
                Match::Tie(names) => tie = names,
                _ => {}
            }
        }
        let index = NameIndex::new(
            fields
                .iter()
                .map(|field| NameEntry::name(*field, NameKind::Field)),
        );
        let mut known: Vec<String> = fields.iter().map(|field| (*field).to_owned()).collect();
        known.sort();
        Err(BindError::new(
            ErrorCode::UnknownField,
            format!(
                "the input records have no field `{written}`; use one of {}",
                known.join(", ")
            ),
        )
        .at(span)
        .found(written.to_owned())
        .expected(&nearest_first(
            tie,
            expected(written, &index, NameKind::Field, EXPECTED_LIMIT),
        ))
        .help("records"))
    }

    /// Returns the function of a call, corrected when it is not a function
    /// of the expression language.
    pub(crate) fn function_name(&mut self, written: &str, span: Option<Span>) -> String {
        if FUNCTIONS.contains(&written) {
            return written.to_owned();
        }
        let Some(corrector) = self.active() else {
            return written.to_owned();
        };
        match values::function(written, &FUNCTIONS, corrector.tables, &corrector.options) {
            Match::Corrected(found) if FUNCTIONS.contains(&found.to.as_str()) => {
                let applied = Applied::new(
                    found.correction(written),
                    span,
                    Change::Function(found.to.clone()),
                )
                .named(written);
                self.record(applied);
                found.to
            }
            _ => written.to_owned(),
        }
    }

    /// Resolves a reference to a `let` name, correcting a misspelled name.
    pub(crate) fn resolve_let(&mut self, reference: &Ref) -> Result<(String, Flow), BindError> {
        if let Some(flow) = self.let_flow(&reference.name) {
            return Ok((reference.name.clone(), flow));
        }
        if let Some(corrector) = self.active() {
            let names = self.let_names();
            let known: Vec<&str> = names.iter().map(String::as_str).collect();
            if let Match::Corrected(found) =
                values::let_name(&reference.name, &known, &corrector.options)
                && let Some(flow) = self.let_flow(&found.to)
            {
                let from = format!("{REF_SIGIL}{}", reference.name);
                let correction = Correction::new(
                    found.rule,
                    found.class,
                    from.clone(),
                    format!("{REF_SIGIL}{}", found.to),
                    CorrectionKind::Ref,
                );
                let applied =
                    Applied::new(correction, reference.span, Change::Ref(found.to.clone()))
                        .named(&from);
                self.record(applied);
                return Ok((found.to, flow));
            }
        }
        Err(self.unknown_name(reference))
    }

    /// Corrects a literal that does not have the form of its typed place
    /// (`T01`–`T08`); `None` when no rule applies.
    pub(crate) fn typed_literal(
        &mut self,
        expr: &Expr,
        scalar: ScalarType,
        place: Place<'_>,
    ) -> Option<Result<BoundExpr, BindError>> {
        let corrector = self.active()?;
        let target = type_target(scalar)?;
        let integer;
        let (written, span) = match expr {
            Expr::Text { v, span } => (Written::quoted(v), *span),
            Expr::Ident { v, span } => (Written::bare(v), *span),
            Expr::Int { v, span } if scalar == ScalarType::Bool => {
                integer = v.to_string();
                (Written::bare(&integer), *span)
            }
            _ => return None,
        };
        let context = types::Context {
            tables: corrector.tables,
            time_zone: &self.context.local_tz,
            command: Some(place.command),
            options: corrector.options,
        };
        match types::correct(written, &target, &context) {
            Outcome::Converted(converted) => {
                let surface = surface_expr(&converted.value)?;
                let mut applied = Applied::new(converted.correction(), span, Change::Expr(surface));
                applied.alternatives = converted
                    .alternatives
                    .iter()
                    .filter_map(|alternative| surface_expr(&alternative.value))
                    .collect();
                self.record(applied);
                Some(Ok(converted.value))
            }
            Outcome::Uncorrectable(problem) => Some(Err(BindError::new(
                ErrorCode::Uncorrectable,
                format!(
                    "{} cannot be read as a value of `{}`: {}",
                    problem.found, place.param, problem.problem
                ),
            )
            .at(span)
            .found(problem.found)
            .help(place.command.to_owned()))),
            Outcome::NotApplicable => None,
        }
    }

    /// Returns the path written as a regular expression in a path place
    /// (`L22`), as the text of the path.
    pub(crate) fn regex_as_path(&self, v: &str, flags: &str) -> Option<String> {
        self.active()?;
        let written = format!("/{v}/{flags}");
        let at = TextPosition {
            line: 1,
            column: 1,
            length: None,
        };
        nurt_correct::lexical::paths::regex_to_path(&written, at).map(|_| written)
    }

    /// Reads a text literal that has escapes changing a pattern or a path
    /// (`L24`) according to the place of its type; `None` when the literal
    /// needs no other reading, or when correction is off.
    ///
    /// The literal is taken from the program text at the span of the node.
    /// A text place never reads it another way: the unknown escape is then
    /// reported by [`crate::facade`] after binding.
    ///
    /// # Errors
    ///
    /// `E_ESCAPE` when the place cannot read an unknown escape, with the whole
    /// program as the fix.
    pub(crate) fn escape_reading(
        &self,
        span: Option<Span>,
        ty: &ParamType,
    ) -> Result<Option<LiteralFix>, BindError> {
        let (Some(corrector), Some(span)) = (self.active(), span) else {
            return Ok(None);
        };
        read_escapes_at(corrector.source, span, escape_place(ty)).map_err(BindError::from)
    }

    /// Records the corrections of a path: `L22` for a regular expression
    /// read as a path, `L19` for its notation and `L23` for its resolution
    /// against `base` or the home directory.
    pub(crate) fn record_path(
        &mut self,
        written: &str,
        resolved: &str,
        span: Option<Span>,
        reading: PathReading,
    ) {
        if self.active().is_none() {
            return;
        }
        let change = Change::Expr(Expr::Text {
            v: resolved.to_owned(),
            span: None,
        });
        let mut chain = Vec::new();
        if reading == PathReading::Regex {
            chain.push((Rule::L22, written.to_owned(), quoted(written)));
        }
        let absolute = PathHost {
            os: self.context.host_os,
            base: None,
            home: None,
        };
        match resolve(written, absolute) {
            Ok(path) if path != written => {
                chain.push((Rule::L19, quoted(written), quoted(&path)));
            }
            Ok(_) => {}
            Err(_) => {
                let at = TextPosition {
                    line: 1,
                    column: 1,
                    length: None,
                };
                let relative = match normalize_path(written, at) {
                    Some(fix) => {
                        let normalized = match &fix.value {
                            Expr::Text { v, .. } => v.clone(),
                            _ => written.to_owned(),
                        };
                        chain.push((Rule::L19, quoted(written), quoted(&normalized)));
                        normalized
                    }
                    None => written.to_owned(),
                };
                chain.push((Rule::L23, quoted(&relative), quoted(resolved)));
            }
        }
        for (rule, from, to) in chain {
            let correction =
                Correction::new(rule.rule_id(), rule.class(), from, to, CorrectionKind::Path);
            let mut applied = Applied::new(correction, span, change.clone());
            if reading != PathReading::Text {
                applied.reading = Some(written.to_owned());
            }
            self.record(applied);
        }
    }

    /// Records the conversion of a local date and time to UTC (`T08`).
    pub(crate) fn record_local_instant(&mut self, written: &str, value: &str, span: Option<Span>) {
        let Some(corrector) = self.active() else {
            return;
        };
        let local = matches!(
            types::instant(Written::quoted(written), corrector.tables, &self.context.local_tz),
            Outcome::Converted(converted) if converted.rule == RuleId::T08
                && matches!(converted.value, BoundExpr::Instant { .. })
        );
        if !local {
            return;
        }
        let correction = Correction::new(
            RuleId::T08,
            Class::Probable,
            quoted(written),
            quoted(value),
            CorrectionKind::Instant,
        );
        let change = Change::Expr(Expr::Text {
            v: value.to_owned(),
            span: None,
        });
        self.record(Applied::new(correction, span, change));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_corrected_literals_to_surface_expressions() {
        assert_eq!(
            surface_expr(&BoundExpr::Size { v: 10 }),
            Some(Expr::Size { v: 10, span: None })
        );
        let ago = BoundExpr::Call {
            function: "ago".to_owned(),
            args: vec![BoundExpr::Dur { v: 1_000 }],
        };
        assert!(matches!(surface_expr(&ago), Some(Expr::Call { args, .. }) if args.len() == 1));
        assert_eq!(
            surface_expr(&BoundExpr::Field {
                name: "a".to_owned(),
                of: None
            }),
            None
        );
    }

    #[test]
    fn lists_the_names_of_a_tie_first() {
        let names = nearest_first(
            vec!["last".to_owned(), "list".to_owned()],
            vec!["list".to_owned(), "lines".to_owned()],
        );
        assert_eq!(names, ["last", "list", "lines"]);
    }

    #[test]
    fn positions_a_correction_at_its_node() {
        let span = Span {
            line: 2,
            column: 4,
            end_line: 2,
            end_column: 9,
        };
        let correction = Correction::new(
            RuleId::V01,
            Class::Fuzzy,
            "decs",
            "desc",
            CorrectionKind::Value,
        );
        let at = located(correction.clone(), Some(span), None).at.unwrap();
        assert_eq!((at.line, at.column, at.length), (2, 4, Some(5)));
        let at = located(correction, Some(span), Some(4)).at.unwrap();
        assert_eq!(at.length, Some(4));
    }

    #[test]
    fn quotes_texts_in_canonical_notation() {
        assert_eq!(quoted("/srv/app"), "\"/srv/app\"");
    }
}
