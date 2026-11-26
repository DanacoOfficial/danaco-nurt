// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Binding of program text with notation correction.
//!
//! [`bind_with_correction`] reads the text with the lexical correction of
//! `nurt-correct` (character normalization, habits of other shells, other
//! syntax variants, token and repair rules), binds the tree with the
//! correction of names, values and literals, writes every correction into
//! the surface tree, checks the correction invariants, and applies the
//! budget and the policy of the correction classes before the program may
//! run.

use nurt_contracts::ast::{
    AstDocument, Expr, Head, Pipeline, Ref, Stage as SurfaceStage, Statement,
};
use nurt_contracts::bound::{BoundDocument, BoundHead, BoundPipeline, BoundStatement, Phase};
use nurt_contracts::config::Config;
use nurt_contracts::error::{At, Candidate as ErrorCandidate, Error, ErrorCode, Stage};
use nurt_contracts::kinds::SyntaxVariant;
use nurt_contracts::report::TextPosition;
use nurt_contracts::spec::{ParamType, ScalarType};
use nurt_correct::candidates::{self, Candidate, Clock, Store};
use nurt_correct::lexical::paths::{escape_error_in, escaped_texts};
use nurt_correct::lexical::tree::{TreeChange, TreeRule, reorder_arguments, split_missing_pipes};
use nurt_correct::lexical::{
    ArgumentShape, CatalogView, Front, Rule, front_for_binding, number_corrections,
};
use nurt_correct::policy::{
    Budget, Class, Decision, Facts, InvariantInput, ProgramKind, check_invariants, decide,
    is_effecting, over_budget,
};
use nurt_correct::record::{Correction, CorrectionKind};
use nurt_correct::tables::Tables;
use nurt_syntax::hash::ast_hash;
use nurt_syntax::print::try_print;

use crate::binder::{bind_corrected, bind_document};
use crate::cast::expr_span;
use crate::catalog::Catalog;
use crate::context::{BindContext, UNCORRECTED_REV};
use crate::correct::{Applied, Change, Corrector, Edit, located};

/// Help topic of correction errors.
const HELP: &str = "corrections";

/// A program read, corrected and bound.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Outcome {
    /// Bound tree of the corrected program.
    pub bound: BoundDocument,
    /// Surface tree of the corrected program.
    pub surface: AstDocument,
    /// Corrections in the order of their position, numbered `C1`, `C2`, …;
    /// empty for a program accepted as written.
    pub corrections: Vec<Correction>,
    /// The corrected program in canonical text of the requested variant.
    pub canonical: String,
}

/// Reads, corrects and binds a program text.
///
/// The budget is `correction.max_read` or `correction.max_effect`; with
/// `correction.enabled = false` the text is bound as written. The `base` of
/// `context` falls back to `scope.base`. Positions of the corrections of
/// binding refer to the text after the lexical corrections.
///
/// # Errors
///
/// Returns the reading error (`E_SYNTAX`, `E_LEXICAL`), the binding error
/// of stage `1`, `E_AMBIGUOUS` with ranked candidates when the policy or
/// the budget rejects a correction, and `E_INTERNAL_CORRECTION` when a
/// correction would change the meaning of the program.
///
/// # Examples
///
/// ```
/// use std::path::Path;
///
/// use nurt_bind::{BindContext, Catalog, bind_with_correction};
/// use nurt_contracts::config::Config;
/// use nurt_contracts::kinds::SyntaxVariant;
/// use nurt_contracts::spec::Os;
/// use nurt_correct::tables::load_embedded;
///
/// let catalog = Catalog::load_dir(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../catalog"))?;
/// let tables = load_embedded()?;
/// let outcome = bind_with_correction(
///     "find \"/srv/app\" |> sort size order: decs",
///     SyntaxVariant::A,
///     &catalog,
///     &tables,
///     &Config::default(),
///     &BindContext::new(Os::Linux),
/// )?;
/// assert_eq!(outcome.corrections[0].rule.to_string(), "V01");
/// assert!(outcome.canonical.ends_with("order: desc"));
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn bind_with_correction(
    text: &str,
    variant: SyntaxVariant,
    catalog: &Catalog,
    tables: &Tables,
    config: &Config,
    context: &BindContext,
) -> Result<Outcome, Error> {
    let mut syntax = config.syntax.clone();
    syntax.variant = variant;
    let view = CommandView::new(catalog);
    let mut context = context.clone();
    if context.base.is_none() {
        context.base.clone_from(&config.scope.base);
    }
    let read = front_for_binding(
        text,
        &syntax,
        &config.correction,
        tables,
        &view,
        context.base.as_deref(),
    )?;
    if !config.correction.enabled {
        let context = context.with_corr_rev(UNCORRECTED_REV);
        let bound = bind_document(&read.document, catalog, &context).map_err(Error::from)?;
        let canonical = print(&read.document, variant)?;
        return Ok(Outcome {
            bound,
            surface: read.document,
            corrections: Vec::new(),
            canonical,
        });
    }
    let context = context.with_corr_rev(tables.corr_rev());
    let corrector = Corrector::new(tables, &config.correction, &read.source);
    let Tree {
        document,
        changes,
        bound,
        applied,
    } = bind_tree(
        &read.document,
        catalog,
        &context,
        &corrector,
        &view,
        variant,
    )?;
    check_escapes(&read.source, &applied)?;
    let mut surface = document.clone();
    write_edits(&mut surface, applied.iter().map(|applied| &applied.edit))?;
    if !applied.is_empty() {
        check_rebinding(&surface, &bound, catalog, &context)?;
    }
    let mut lexical = read.corrections.clone();
    lexical.extend(changes);
    let corrections = merged(&lexical, &applied);
    let request = Request {
        read: &read,
        document: &document,
        lexical: &lexical,
        applied: &applied,
        corrections: &corrections,
        bound: &bound,
        catalog,
        view: &view,
        tables,
        context: &context,
        config,
        variant,
    };
    request.check_meaning()?;
    let canonical = print(&surface, variant)?;
    request.check_policy(&canonical)?;
    Ok(Outcome {
        bound,
        surface,
        corrections,
        canonical,
    })
}

/// Keeps the candidates of an `E_AMBIGUOUS` error in the store and gives
/// them the identifiers that `accept` takes; other errors are returned
/// unchanged.
///
/// # Examples
///
/// ```
/// use std::time::Duration;
///
/// use nurt_bind::offer_candidates;
/// use nurt_contracts::error::{Candidate, Error, ErrorCode};
/// use nurt_correct::candidates::{ManualClock, Store};
///
/// let clock = ManualClock::new();
/// let mut store = Store::new(Duration::from_secs(600), &clock);
/// let error = Error::new(ErrorCode::Ambiguous, "two readings").with_candidate(Candidate {
///     rank: 1,
///     reference: None,
///     program: "remove \"/srv/a\"".to_owned(),
///     why: None,
/// });
/// let offered = offer_candidates(error, &mut store);
/// assert_eq!(offered.candidates[0].reference.as_deref(), Some("c1"));
/// assert_eq!(store.accept("c1").unwrap(), "remove \"/srv/a\"");
/// ```
#[must_use]
pub fn offer_candidates<C: Clock>(mut error: Error, store: &mut Store<C>) -> Error {
    if error.code() != ErrorCode::Ambiguous || error.candidates.is_empty() {
        return error;
    }
    let ranked = error
        .candidates
        .iter()
        .enumerate()
        .map(|(position, candidate)| Candidate {
            program: candidate.program.clone(),
            why: candidate.why.clone().unwrap_or_default(),
            probable: 0,
            targets: u64::try_from(position).unwrap_or(u64::MAX),
            distance: 0,
        })
        .collect();
    error.candidates = store.insert(ranked);
    error
}

/// Fails with `E_ESCAPE` for the first text of the program that has an escape
/// the language does not have and that no place read as written (`L24`).
fn check_escapes(source: &str, applied: &[Applied]) -> Result<(), Error> {
    let read = |at: TextPosition| {
        applied.iter().any(|applied| {
            applied.correction.rule == Rule::L24.rule_id()
                && applied
                    .correction
                    .at
                    .is_some_and(|found| (found.line, found.column) == (at.line, at.column))
        })
    };
    match escaped_texts(source).iter().find(|text| !read(text.at)) {
        Some(text) => Err(escape_error_in(source, text)),
        None => Ok(()),
    }
}

/// A tree bound with corrections, after the rules applied to the parsed
/// tree.
struct Tree {
    /// The tree read, with the changes of the parsed-tree rules.
    document: AstDocument,
    /// Corrections of the parsed-tree rules (`L14`, `L18`).
    changes: Vec<Correction>,
    bound: BoundDocument,
    applied: Vec<Applied>,
}

/// Binds a tree read with corrections. Positional arguments written after
/// named ones are moved in front (`L18`); a program that does not bind is
/// split before command names (`L14`) and reordered again, as the reading
/// does, and the first error is returned when that does not bind either.
fn bind_tree(
    read: &AstDocument,
    catalog: &Catalog,
    context: &BindContext,
    corrector: &Corrector<'_>,
    view: &CommandView<'_>,
    variant: SyntaxVariant,
) -> Result<Tree, Error> {
    let mut document = read.clone();
    let changes = reorder_arguments(&mut document, variant);
    let error = match bind_corrected(&document, catalog, context, corrector) {
        Ok((bound, applied)) => {
            return Ok(Tree {
                document,
                changes: tree_corrections(changes)?,
                bound,
                applied,
            });
        }
        Err(error) => Error::from(error),
    };
    let mut split = read.clone();
    let mut changes = split_missing_pipes(&mut split, view);
    if changes.is_empty() {
        return Err(error);
    }
    changes.extend(reorder_arguments(&mut split, variant));
    let Ok((bound, applied)) = bind_corrected(&split, catalog, context, corrector) else {
        return Err(error);
    };
    Ok(Tree {
        document: split,
        changes: tree_corrections(changes)?,
        bound,
        applied,
    })
}

/// Corrections of the changes made to the parsed tree, positioned at the
/// spans of the changed nodes.
fn tree_corrections(changes: Vec<TreeChange>) -> Result<Vec<Correction>, Error> {
    changes
        .into_iter()
        .map(|change| {
            let (rule, kind) = match change.rule {
                TreeRule::MissingPipe => (Rule::L14, CorrectionKind::Pipe),
                TreeRule::ArgumentOrder => (Rule::L18, CorrectionKind::Order),
                _ => {
                    return Err(internal(
                        "a change of the parsed tree has no correction rule",
                    ));
                }
            };
            let correction =
                Correction::new(rule.rule_id(), rule.class(), change.from, change.to, kind);
            Ok(located(correction, change.span, None))
        })
        .collect()
}

/// Facts of one correction request.
struct Request<'r> {
    read: &'r Front,
    /// The tree read, with the changes of the parsed-tree rules.
    document: &'r AstDocument,
    /// Corrections of reading and of the parsed-tree rules.
    lexical: &'r [Correction],
    applied: &'r [Applied],
    corrections: &'r [Correction],
    bound: &'r BoundDocument,
    catalog: &'r Catalog,
    view: &'r CommandView<'r>,
    tables: &'r Tables,
    context: &'r BindContext,
    config: &'r Config,
    variant: SyntaxVariant,
}

impl Request<'_> {
    /// Checks the correction invariants against the tree read before the
    /// corrections of binding; a regular expression read as a path is
    /// compared as the path it was read as.
    fn check_meaning(&self) -> Result<(), Error> {
        let mut before = self.document.clone();
        let readings: Vec<Edit> = self
            .applied
            .iter()
            .filter_map(|applied| {
                applied.reading.as_ref().map(|written| Edit {
                    span: applied.edit.span,
                    change: Change::Expr(Expr::Text {
                        v: written.clone(),
                        span: None,
                    }),
                })
            })
            .collect();
        write_edits(&mut before, readings.iter())?;
        let effecting = |name: &str| effecting_stage(self.bound, self.catalog, name);
        check_invariants(&InvariantInput {
            before: Some(&before),
            after: self.bound,
            corrections: self.corrections,
            base: self.context.base.as_deref(),
            tables: self.tables,
            effecting: &effecting,
        })
        .map_err(|violation| violation.to_error().with_stage(Stage::Bind))
    }

    /// Applies the budget and the policy of the correction classes.
    fn check_policy(&self, canonical: &str) -> Result<(), Error> {
        let kind = ProgramKind::of_bound(self.bound);
        let budget = Budget::from(&self.config.correction);
        if over_budget(self.corrections.len(), kind, &budget) {
            return Err(self.over_budget(kind, budget.limit(kind), canonical));
        }
        let general =
            Facts::new(&self.config.correction).structural(self.read.alternatives.is_empty(), true);
        let mut doubtful: Vec<&Correction> = self
            .lexical
            .iter()
            .filter(|correction| decide(correction.class, kind, &general) == Decision::Ambiguous)
            .collect();
        for applied in self.applied {
            let correction = &applied.correction;
            let facts =
                if correction.kind == CorrectionKind::Command && correction.class == Class::Fuzzy {
                    general.fuzzy(applied.effecting)
                } else if applied.reading.is_some() {
                    general.structural(self.read.alternatives.is_empty(), false)
                } else {
                    general
                };
            if decide(correction.class, kind, &facts) == Decision::Ambiguous {
                doubtful.push(correction);
            }
        }
        match doubtful.first() {
            None => Ok(()),
            Some(first) => Err(self.ambiguous(first, canonical)),
        }
    }

    fn over_budget(&self, kind: ProgramKind, limit: u32, canonical: &str) -> Error {
        let program = match kind {
            ProgramKind::Read => "a program that reads",
            ProgramKind::Effect => "a program that changes the world",
        };
        Error::new(
            ErrorCode::Ambiguous,
            format!(
                "the program needs {} corrections and {program} takes at most {limit}; send the corrected program",
                self.corrections.len()
            ),
        )
        .with_stage(Stage::Bind)
        .with_help(HELP)
        .with_candidate(ErrorCandidate {
            rank: 1,
            reference: None,
            program: canonical.to_owned(),
            why: Some("Applies every correction.".to_owned()),
        })
    }

    fn ambiguous(&self, first: &Correction, canonical: &str) -> Error {
        let probable = self
            .corrections
            .iter()
            .filter(|correction| correction.class == Class::Probable)
            .count();
        let mut offered = vec![Candidate {
            program: canonical.to_owned(),
            why: format!("Reads `{}` as `{}`.", first.from, first.to),
            probable,
            targets: 0,
            distance: self.corrections.len(),
        }];
        offered.extend(self.alternative_programs(probable));
        let mut error = Error::new(
            ErrorCode::Ambiguous,
            format!(
                "the correction of `{}` to `{}` is not certain in this program; send one of the candidates",
                first.from, first.to
            ),
        )
        .with_stage(Stage::Bind)
        .with_found(first.from.clone())
        .with_help(HELP);
        if let Some(at) = first.at {
            error = error.with_at(At {
                line: Some(at.line),
                column: Some(at.column),
                length: at.length,
                stage: Some(Stage::Bind),
            });
        }
        for (position, candidate) in candidates::rank(offered).into_iter().enumerate() {
            error = error.with_candidate(ErrorCandidate {
                rank: u32::try_from(position + 1).unwrap_or(u32::MAX),
                reference: None,
                program: candidate.program,
                why: Some(candidate.why),
            });
        }
        error
    }

    /// Programs with the other readings of the probable corrections, and
    /// the other readings of the lexical repairs.
    fn alternative_programs(&self, probable: usize) -> Vec<Candidate> {
        let mut programs = Vec::new();
        for (index, applied) in self.applied.iter().enumerate() {
            for (rank, alternative) in applied.alternatives.iter().enumerate() {
                let mut edits: Vec<Edit> = self
                    .applied
                    .iter()
                    .map(|other| other.edit.clone())
                    .collect();
                if let Some(edit) = edits.get_mut(index) {
                    edit.change = Change::Expr(alternative.clone());
                }
                let mut surface = self.document.clone();
                if write_edits(&mut surface, edits.iter()).is_err() {
                    continue;
                }
                if let Ok(program) = print(&surface, self.variant) {
                    programs.push(Candidate {
                        program,
                        why: format!(
                            "Reads `{}` as `{}`.",
                            applied.correction.from,
                            applied
                                .correction
                                .alternatives
                                .get(rank)
                                .cloned()
                                .unwrap_or_default()
                        ),
                        probable,
                        targets: u64::try_from(rank + 1).unwrap_or(u64::MAX),
                        distance: self.corrections.len(),
                    });
                }
            }
        }
        for (rank, reading) in self.read.alternatives.iter().enumerate() {
            if let Some(program) = self.corrected_reading(reading) {
                programs.push(Candidate {
                    program,
                    why: "Reads the program with another repair.".to_owned(),
                    probable,
                    targets: u64::try_from(rank + 1).unwrap_or(u64::MAX),
                    distance: reading.corrections.len(),
                });
            }
        }
        programs
    }

    fn corrected_reading(&self, reading: &Front) -> Option<String> {
        let corrector = Corrector::new(self.tables, &self.config.correction, &reading.source);
        let tree = bind_tree(
            &reading.document,
            self.catalog,
            self.context,
            &corrector,
            self.view,
            self.variant,
        )
        .ok()?;
        let mut surface = tree.document;
        write_edits(
            &mut surface,
            tree.applied.iter().map(|applied| &applied.edit),
        )
        .ok()?;
        print(&surface, self.variant).ok()
    }
}

/// Whether a command name is a changing or running stage of the program;
/// a name that is not a stage of the program is judged by the catalog.
fn effecting_stage(bound: &BoundDocument, catalog: &Catalog, name: &str) -> bool {
    let mut found = false;
    for pipeline in bound
        .program
        .statements
        .iter()
        .filter_map(|statement| match statement {
            BoundStatement::Let(binding) => Some(&binding.value),
            BoundStatement::Pipeline(pipeline) => Some(pipeline),
            _ => None,
        })
    {
        for (stage, phase) in stage_phases(pipeline) {
            if stage == name {
                if matches!(phase, Phase::Mutate | Phase::Run) {
                    return true;
                }
                found = true;
            }
        }
    }
    !found && catalog.command(name).is_some_and(is_effecting)
}

fn stage_phases(pipeline: &BoundPipeline) -> impl Iterator<Item = (&str, Phase)> {
    let head = match &pipeline.head {
        BoundHead::Stage(stage) => Some(stage),
        _ => None,
    };
    head.into_iter()
        .chain(pipeline.stages.iter())
        .map(|stage| (stage.name.as_str(), stage.phase))
}

/// Binds the corrected surface tree without correction and checks that it
/// gives the tree bound with corrections.
fn check_rebinding(
    surface: &AstDocument,
    bound: &BoundDocument,
    catalog: &Catalog,
    context: &BindContext,
) -> Result<(), Error> {
    let rebound = bind_document(surface, catalog, context).map_err(|error| {
        internal(format!(
            "the corrected program does not bind: {}",
            error.error().message
        ))
    })?;
    let same = match (ast_hash(&rebound), ast_hash(bound)) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    };
    if same {
        Ok(())
    } else {
        Err(internal(
            "the corrected program binds to another tree than the corrections",
        ))
    }
}

/// The lexical corrections and the corrections of binding, in the order of
/// their position, numbered.
fn merged(lexical: &[Correction], applied: &[Applied]) -> Vec<Correction> {
    let mut corrections: Vec<Correction> = lexical.to_vec();
    corrections.extend(applied.iter().map(|applied| applied.correction.clone()));
    corrections.sort_by_key(|correction| {
        correction
            .at
            .map_or((u32::MAX, u32::MAX), |at| (at.line, at.column))
    });
    number_corrections(&mut corrections);
    corrections
}

fn print(document: &AstDocument, variant: SyntaxVariant) -> Result<String, Error> {
    try_print(document, variant)
        .map(|text| text.trim_end().to_owned())
        .map_err(|error| {
            internal(format!(
                "the corrected program cannot be printed: {error:?}"
            ))
        })
}

fn internal(problem: impl Into<String>) -> Error {
    Error::new(
        ErrorCode::InternalCorrection,
        "correction would change the meaning of the program; nothing was run",
    )
    .with_stage(Stage::Bind)
    .with_problem(problem)
    .with_help(HELP)
}

/// Writes the edits into a surface tree; every distinct edit must find its
/// node.
fn write_edits<'e>(
    document: &mut AstDocument,
    edits: impl Iterator<Item = &'e Edit>,
) -> Result<(), Error> {
    let mut unique: Vec<&Edit> = Vec::new();
    for edit in edits {
        if edit.span.is_none() {
            return Err(internal("a correction has no position in the program"));
        }
        if !unique.contains(&edit) {
            unique.push(edit);
        }
    }
    if unique.is_empty() {
        return Ok(());
    }
    let mut editor = Editor {
        placed: vec![false; unique.len()],
        edits: unique,
    };
    for statement in &mut document.program.statements {
        match statement {
            Statement::Let(binding) => editor.pipeline(&mut binding.value),
            Statement::Pipeline(pipeline) => editor.pipeline(pipeline),
            _ => {}
        }
    }
    if editor.placed.iter().all(|placed| *placed) {
        Ok(())
    } else {
        Err(internal(
            "a correction could not be written into the program",
        ))
    }
}

/// Places edits on the surface nodes with their spans.
struct Editor<'e> {
    edits: Vec<&'e Edit>,
    placed: Vec<bool>,
}

impl Editor<'_> {
    /// Takes the first edit not yet placed at the span whose change fits.
    fn take(
        &mut self,
        span: Option<nurt_contracts::ast::Span>,
        fits: impl Fn(&Change) -> bool,
    ) -> Option<Change> {
        let span = span?;
        let position = self
            .edits
            .iter()
            .zip(&self.placed)
            .position(|(edit, placed)| !placed && edit.span == Some(span) && fits(&edit.change))?;
        self.placed[position] = true;
        Some(self.edits[position].change.clone())
    }

    fn pipeline(&mut self, pipeline: &mut Pipeline) {
        match &mut pipeline.head {
            Head::Stage(stage) => self.stage(stage),
            Head::Ref(reference) => self.reference(reference),
            _ => {}
        }
        for stage in &mut pipeline.stages {
            self.stage(stage);
        }
    }

    fn stage(&mut self, stage: &mut SurfaceStage) {
        if let Some(Change::Command(name)) =
            self.take(stage.span, |change| matches!(change, Change::Command(_)))
        {
            stage.name = name;
        }
        for arg in &mut stage.args {
            if arg.name.is_some()
                && let Some(Change::Param(name)) =
                    self.take(arg.span, |change| matches!(change, Change::Param(_)))
            {
                arg.name = Some(name);
            }
            self.expr(&mut arg.value);
        }
    }

    fn reference(&mut self, reference: &mut Ref) {
        if let Some(Change::Ref(name)) =
            self.take(reference.span, |change| matches!(change, Change::Ref(_)))
        {
            reference.name = name;
        }
    }

    fn expr(&mut self, expr: &mut Expr) {
        if let Some(Change::Expr(replacement)) =
            self.take(expr_span(expr), |change| matches!(change, Change::Expr(_)))
        {
            *expr = replacement;
            return;
        }
        match expr {
            Expr::Ref(reference) => self.reference(reference),
            Expr::Call {
                function,
                args,
                span,
            } => {
                if let Some(Change::Function(name)) =
                    self.take(*span, |change| matches!(change, Change::Function(_)))
                {
                    *function = name;
                }
                for arg in args {
                    self.expr(arg);
                }
            }
            Expr::List { items, .. } => {
                for item in items {
                    self.expr(item);
                }
            }
            Expr::Map { entries, .. } => {
                for entry in entries {
                    self.expr(&mut entry.v);
                }
            }
            Expr::Spread { x, .. } | Expr::Un { x, .. } => self.expr(x),
            Expr::Field { of: Some(of), .. } => self.expr(of),
            Expr::Bin { l, r, .. } => {
                self.expr(l);
                self.expr(r);
            }
            Expr::Sub { p, .. } => self.pipeline(p),
            _ => {}
        }
    }
}

/// View of the catalog needed by the lexical correction rules.
struct CommandView<'c> {
    catalog: &'c Catalog,
    names: Vec<&'c str>,
}

impl<'c> CommandView<'c> {
    fn new(catalog: &'c Catalog) -> Self {
        Self {
            catalog,
            names: catalog
                .commands()
                .iter()
                .map(|command| command.name.as_str())
                .collect(),
        }
    }
}

impl CatalogView for CommandView<'_> {
    fn is_command(&self, name: &str) -> bool {
        self.catalog.command(name).is_some()
    }

    fn command_names(&self) -> &[&str] {
        &self.names
    }

    fn is_mutating(&self, name: &str) -> bool {
        self.catalog.command(name).is_some_and(is_effecting)
    }

    fn can_take_argument(&self, command: &str, position: usize, shape: ArgumentShape) -> bool {
        let Some(spec) = self.catalog.command(command) else {
            return false;
        };
        let signature: Vec<&str> = spec
            .positional()
            .into_iter()
            .map(|param| param.name.as_str())
            .collect();
        std::iter::once(signature)
            .chain(
                spec.forms
                    .iter()
                    .map(|form| form.positional.iter().map(String::as_str).collect()),
            )
            .any(|names| {
                names
                    .get(position)
                    .and_then(|name| spec.param(name))
                    .is_some_and(|param| takes(&param.ty, shape))
            })
    }
}

/// Whether a value of the shape fits a place of the type.
fn takes(ty: &ParamType, shape: ArgumentShape) -> bool {
    match ty {
        ParamType::Scalar(scalar) => scalar_takes(*scalar, shape),
        ParamType::Enum(_) => shape == ArgumentShape::Word,
        ParamType::List(inner) => takes(inner, shape),
        ParamType::Union(alternatives) => alternatives
            .iter()
            .any(|alternative| takes(alternative, shape)),
        _ => false,
    }
}

fn scalar_takes(scalar: ScalarType, shape: ArgumentShape) -> bool {
    use ScalarType as S;
    let open = matches!(scalar, S::Any | S::Expression);
    open || match shape {
        ArgumentShape::Word => matches!(
            scalar,
            S::Field | S::Pipeline | S::NamedExpression | S::Bool
        ),
        ArgumentShape::Text => matches!(
            scalar,
            S::Text
                | S::Bytes
                | S::Path
                | S::Glob
                | S::Key
                | S::Instant
                | S::Percent
                | S::Size
                | S::Duration
        ),
        ArgumentShape::Regex => scalar == S::Regex,
        ArgumentShape::Number => matches!(
            scalar,
            S::Int | S::Number | S::Size | S::Duration | S::Range | S::Percent
        ),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use nurt_contracts::ast::{Arg, Program, Span};

    use super::*;

    fn span(column: u32, end_column: u32) -> Option<Span> {
        Some(Span {
            line: 1,
            column,
            end_line: 1,
            end_column,
        })
    }

    fn document() -> AstDocument {
        let stage = SurfaceStage {
            name: "sotr".to_owned(),
            args: vec![Arg {
                name: Some("ordr".to_owned()),
                value: Expr::Ident {
                    v: "decs".to_owned(),
                    span: span(18, 22),
                },
                span: span(12, 22),
            }],
            span: span(1, 22),
        };
        AstDocument::new(Program {
            statements: vec![Statement::Pipeline(Pipeline {
                head: Head::Stage(stage),
                stages: Vec::new(),
                span: span(1, 22),
            })],
        })
    }

    #[test]
    fn writes_names_and_values_at_their_spans() {
        let mut tree = document();
        let edits = [
            Edit {
                span: span(1, 22),
                change: Change::Command("sort".to_owned()),
            },
            Edit {
                span: span(12, 22),
                change: Change::Param("order".to_owned()),
            },
            Edit {
                span: span(18, 22),
                change: Change::Expr(Expr::Ident {
                    v: "desc".to_owned(),
                    span: None,
                }),
            },
        ];
        write_edits(&mut tree, edits.iter()).unwrap();
        let printed = print(&tree, SyntaxVariant::A).unwrap();
        assert_eq!(printed, "sort order: desc");
    }

    #[test]
    fn rejects_an_edit_without_its_node() {
        let mut tree = document();
        let edits = [Edit {
            span: span(30, 32),
            change: Change::Command("sort".to_owned()),
        }];
        let error = write_edits(&mut tree, edits.iter()).unwrap_err();
        assert_eq!(error.code(), ErrorCode::InternalCorrection);
        let unplaced = [Edit {
            span: None,
            change: Change::Command("sort".to_owned()),
        }];
        assert!(write_edits(&mut tree, unplaced.iter()).is_err());
    }

    #[test]
    fn matches_argument_shapes_to_types() {
        let path: ParamType = "path".parse().unwrap();
        let pattern: ParamType = "text | regex".parse().unwrap();
        let order: ParamType = "enum(asc|desc)".parse().unwrap();
        assert!(takes(&path, ArgumentShape::Text));
        assert!(!takes(&path, ArgumentShape::Word));
        assert!(takes(&pattern, ArgumentShape::Regex));
        assert!(takes(&order, ArgumentShape::Word));
        assert!(!takes(&order, ArgumentShape::Number));
    }
}
