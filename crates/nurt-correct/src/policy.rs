// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Policy that decides whether a correction is applied.
//!
//! | Class | Reading program | Program that changes the world or runs a program |
//! |---|---|---|
//! | C | apply, show | apply, show |
//! | S | apply, show | apply when the structure is unambiguous and literal bytes are unchanged; otherwise `E_AMBIGUOUS` |
//! | F | apply, show | apply when the name is not an effecting stage, or at distance 1 when the whole program binds and type-checks; otherwise `E_AMBIGUOUS` |
//! | P | apply the fixed-priority interpretation, show at most three alternatives | do not apply: `E_AMBIGUOUS` with ranked candidates |
//!
//! The number of corrections of one program is limited by [`Budget`]; a
//! program over budget is not run and `E_AMBIGUOUS` carries its full
//! canonical form as the single candidate.

use std::collections::HashMap;
use std::fmt;

use nurt_contracts::ast::{AstDocument, Expr, Head, Pipeline, Stage, Statement};
use nurt_contracts::bound::{
    BoundArg, BoundDocument, BoundExpr, BoundHead, BoundPipeline, BoundStage, BoundStatement, Phase,
};
use nurt_contracts::config::CorrectionConfig;
use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::kinds::{Category, EffectKind};
use nurt_contracts::spec::StageSpec;

pub use nurt_contracts::report::CorrectionClass as Class;

use crate::distance::{EFFECTING_MAX_DISTANCE, fold};
use crate::record::{Correction, CorrectionKind, RuleId};
use crate::tables::Tables;
use crate::types::{self, Outcome, Written};

/// Default correction budget of a reading program (`correction.max_read`).
pub const DEFAULT_MAX_READ: u32 = 8;

/// Default correction budget of a changing program (`correction.max_effect`).
pub const DEFAULT_MAX_EFFECT: u32 = 4;

/// Largest number of alternatives shown for a probable correction of a
/// reading program.
pub const MAX_ALTERNATIVES: usize = 3;

/// Whether a program only reads or changes the world.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProgramKind {
    /// No stage of phase `mutate` or `run`.
    Read,
    /// A stage of phase `mutate` or `run`.
    Effect,
}

impl ProgramKind {
    /// Kind of a program with stages of the given phases.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_contracts::bound::Phase;
    /// use nurt_correct::policy::ProgramKind;
    ///
    /// assert_eq!(ProgramKind::from_phases([Phase::Source, Phase::Transform]), ProgramKind::Read);
    /// assert_eq!(ProgramKind::from_phases([Phase::Source, Phase::Mutate]), ProgramKind::Effect);
    /// ```
    #[must_use]
    pub fn from_phases(phases: impl IntoIterator<Item = Phase>) -> Self {
        if phases
            .into_iter()
            .any(|phase| matches!(phase, Phase::Mutate | Phase::Run))
        {
            Self::Effect
        } else {
            Self::Read
        }
    }

    /// Kind of a bound program, from the phases of its top-level stages.
    #[must_use]
    pub fn of_bound(document: &BoundDocument) -> Self {
        let pipelines =
            document
                .program
                .statements
                .iter()
                .filter_map(|statement| match statement {
                    BoundStatement::Let(binding) => Some(&binding.value),
                    BoundStatement::Pipeline(pipeline) => Some(pipeline),
                    _ => None,
                });
        Self::from_phases(pipelines.flat_map(phases))
    }
}

fn phases(pipeline: &BoundPipeline) -> impl Iterator<Item = Phase> + '_ {
    let head = match &pipeline.head {
        BoundHead::Stage(stage) => Some(stage.phase),
        _ => None,
    };
    head.into_iter()
        .chain(pipeline.stages.iter().map(|stage| stage.phase))
}

/// Whether a catalog command changes the world or runs a program.
///
/// A command is effecting when its category is `change` or `run`, or when its
/// effect or the effect of any of its actions is other than `read` (this
/// covers `undo`, `redo` and the service actions).
#[must_use]
pub fn is_effecting(spec: &StageSpec) -> bool {
    matches!(spec.category, Category::Change | Category::Run)
        || spec.effect != EffectKind::Read
        || spec
            .effects
            .iter()
            .any(|action| action.effect != EffectKind::Read)
}

/// Facts about one correction that the policy needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Facts {
    /// Key `correction.enabled`.
    pub enabled: bool,
    /// The structure of the corrected program is unambiguous (class S).
    pub structure_unambiguous: bool,
    /// The bytes of every literal are unchanged (class S).
    pub literals_unchanged: bool,
    /// The corrected name is the name of an effecting stage (class F).
    pub effecting_name: bool,
    /// Edit distance of a fuzzy name correction (class F).
    pub distance: usize,
}

impl Facts {
    /// Facts of a correction with no structural doubt and no effecting name.
    #[must_use]
    pub fn new(config: &CorrectionConfig) -> Self {
        Self {
            enabled: config.enabled,
            ..Self::default()
        }
    }

    /// Facts of a fuzzy name correction.
    #[must_use]
    pub fn fuzzy(mut self, effecting_name: bool, distance: usize) -> Self {
        self.effecting_name = effecting_name;
        self.distance = distance;
        self
    }

    /// Facts of a structural correction.
    #[must_use]
    pub fn structural(mut self, structure_unambiguous: bool, literals_unchanged: bool) -> Self {
        self.structure_unambiguous = structure_unambiguous;
        self.literals_unchanged = literals_unchanged;
        self
    }
}

impl Default for Facts {
    fn default() -> Self {
        Self {
            enabled: true,
            structure_unambiguous: true,
            literals_unchanged: true,
            effecting_name: false,
            distance: 0,
        }
    }
}

/// Decision of the policy for one correction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Decision {
    /// Apply the correction and show it.
    Apply,
    /// Apply the fixed-priority interpretation, show it and list at most
    /// [`MAX_ALTERNATIVES`] alternatives.
    ApplyWithAlternatives,
    /// Apply only when the whole corrected program binds and type-checks;
    /// otherwise treat as [`Decision::Ambiguous`].
    ApplyIfProgramChecks,
    /// Do not run; return `E_AMBIGUOUS` with ranked candidates.
    Ambiguous,
    /// Correction is disabled; report the original error.
    Disabled,
}

/// Decides how a correction of `class` is handled in a program of kind
/// `program`.
///
/// # Examples
///
/// ```
/// use nurt_correct::policy::{Class, Decision, Facts, ProgramKind, decide};
///
/// let facts = Facts::default();
/// assert_eq!(decide(Class::Probable, ProgramKind::Read, &facts), Decision::ApplyWithAlternatives);
/// assert_eq!(decide(Class::Probable, ProgramKind::Effect, &facts), Decision::Ambiguous);
/// let renamed = facts.fuzzy(true, 1);
/// assert_eq!(decide(Class::Fuzzy, ProgramKind::Effect, &renamed), Decision::ApplyIfProgramChecks);
/// ```
#[must_use]
pub fn decide(class: Class, program: ProgramKind, facts: &Facts) -> Decision {
    if !facts.enabled {
        return Decision::Disabled;
    }
    match (class, program) {
        (Class::Certain, _) | (Class::Structural | Class::Fuzzy, ProgramKind::Read) => {
            Decision::Apply
        }
        (Class::Probable, ProgramKind::Read) => Decision::ApplyWithAlternatives,
        (Class::Structural, ProgramKind::Effect) => {
            if facts.structure_unambiguous && facts.literals_unchanged {
                Decision::Apply
            } else {
                Decision::Ambiguous
            }
        }
        (Class::Fuzzy, ProgramKind::Effect) => {
            if !facts.effecting_name {
                Decision::Apply
            } else if facts.distance == EFFECTING_MAX_DISTANCE {
                Decision::ApplyIfProgramChecks
            } else {
                Decision::Ambiguous
            }
        }
        _ => Decision::Ambiguous,
    }
}

/// Largest number of corrections of one program.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Budget {
    /// Key `correction.max_read`.
    pub max_read: u32,
    /// Key `correction.max_effect`.
    pub max_effect: u32,
}

impl Budget {
    /// Budget of a program of the given kind.
    #[must_use]
    pub const fn limit(&self, program: ProgramKind) -> u32 {
        match program {
            ProgramKind::Read => self.max_read,
            ProgramKind::Effect => self.max_effect,
        }
    }
}

impl Default for Budget {
    fn default() -> Self {
        Self {
            max_read: DEFAULT_MAX_READ,
            max_effect: DEFAULT_MAX_EFFECT,
        }
    }
}

impl From<&CorrectionConfig> for Budget {
    fn from(config: &CorrectionConfig) -> Self {
        Self {
            max_read: config.max_read,
            max_effect: config.max_effect,
        }
    }
}

/// Whether `count` corrections exceed the budget of a program of kind
/// `program`.
///
/// # Examples
///
/// ```
/// use nurt_correct::policy::{Budget, ProgramKind, over_budget};
///
/// let budget = Budget::default();
/// assert!(!over_budget(8, ProgramKind::Read, &budget));
/// assert!(over_budget(5, ProgramKind::Effect, &budget));
/// ```
#[must_use]
pub fn over_budget(count: usize, program: ProgramKind, budget: &Budget) -> bool {
    count > usize::try_from(budget.limit(program)).unwrap_or(usize::MAX)
}

/// Stages that select the targets of a change.
pub const SELECTING_STAGES: [&str; 5] = ["find", "list", "search", "filter", "assert"];

/// Stages whose integer arguments are never changed by correction.
pub const COUNTING_STAGES: [&str; 4] = ["assert", "first", "last", "skip"];

/// Parameters whose numbers are never changed by correction.
pub const NUMBER_PARAMS: [&str; 3] = ["depth", "limit", "timeout"];

/// Parameters that select the mode of an effect; `type` is one of them in
/// `create` only.
pub const MODE_PARAMS: [&str; 3] = ["mode", "on_conflict", "recursive"];

const CREATE_STAGE: &str = "create";
const TYPE_PARAM: &str = "type";

/// A correction invariant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Invariant {
    /// The bytes of path, glob, regular expression and replacement literals
    /// are kept, apart from the separator, repeated and trailing slashes and
    /// the case of a drive letter.
    LiteralBytes,
    /// The numbers of counting stages, ranges, `depth`, `limit` and
    /// `timeout` are kept.
    Numbers,
    /// No stage that selects targets is added, removed or replaced.
    Targets,
    /// The kind of the changing stage is kept and no effect is added or
    /// removed.
    EffectKind,
    /// Mode parameters come from the program or from an exact or table
    /// correction, never from a guess.
    ModeParameters,
    /// Relative paths are resolved only against an explicit base.
    Base,
}

impl Invariant {
    /// Number of the invariant, from 1 to 6.
    #[must_use]
    pub const fn number(self) -> u8 {
        match self {
            Self::LiteralBytes => 1,
            Self::Numbers => 2,
            Self::Targets => 3,
            Self::EffectKind => 4,
            Self::ModeParameters => 5,
            Self::Base => 6,
        }
    }
}

/// A correction that would change the meaning of the program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvariantViolation {
    /// Invariant broken.
    pub invariant: Invariant,
    /// What changed.
    pub detail: String,
}

impl InvariantViolation {
    fn new(invariant: Invariant, detail: impl Into<String>) -> Self {
        Self {
            invariant,
            detail: detail.into(),
        }
    }

    /// The `E_INTERNAL_CORRECTION` error returned instead of running the
    /// program.
    #[must_use]
    pub fn to_error(&self) -> Error {
        Error::new(
            ErrorCode::InternalCorrection,
            "correction would change the meaning of the program; nothing was run",
        )
        .with_problem(self.detail.clone())
        .with_help("corrections")
    }
}

impl fmt::Display for InvariantViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.detail)
    }
}

impl std::error::Error for InvariantViolation {}

/// Input of [`check_invariants`].
#[derive(Clone, Copy)]
pub struct InvariantInput<'a> {
    /// Surface tree of the program before name, type and value correction;
    /// absent for a program received as a tree.
    pub before: Option<&'a AstDocument>,
    /// Bound tree after correction.
    pub after: &'a BoundDocument,
    /// Corrections applied.
    pub corrections: &'a [Correction],
    /// Explicit base of relative paths.
    pub base: Option<&'a str>,
    /// Correction tables, for literals rewritten by type rules.
    pub tables: &'a Tables,
    /// Whether a command name is a command that changes the world or runs
    /// a program.
    pub effecting: &'a dyn Fn(&str) -> bool,
}

/// Checks that the corrections of a program keep its meaning.
///
/// The check compares the surface tree before correction with the bound
/// tree after it: literal bytes, protected numbers, the stages that select
/// targets, the changing stages, the mode parameters and the base of
/// relative paths. Arguments filled from catalog defaults are not
/// corrections and are not checked.
///
/// # Errors
///
/// Returns the first [`InvariantViolation`] found; the caller converts it
/// with [`InvariantViolation::to_error`] and does not run the program.
pub fn check_invariants(input: &InvariantInput<'_>) -> Result<(), InvariantViolation> {
    check_renamed_effects(input)?;
    let Some(before) = input.before else {
        return Ok(());
    };
    let before = SurfaceFacts::collect(before, input);
    let after = BoundFacts::collect(input.after);
    check_stages(&before, &after, input)?;
    check_literals(&before, &after, input.base)?;
    check_numbers(&before, &after)?;
    check_modes(&before, &after, input)
}

/// Result of comparing a written path with its bound form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathCheck {
    /// Only the allowed rewrites were made.
    Preserved,
    /// A relative path was resolved without an explicit base.
    BaseChosen,
    /// The path was changed.
    Changed,
}

/// Compares the path `written` in the program with the path `bound` after
/// correction.
///
/// The allowed rewrites are `\` to `/`, repeated `/` to one (except the `//`
/// of a UNC path), removal of a trailing `/` (except of a root), an
/// upper-case drive letter, resolution of a relative path against an
/// explicit `base`, and resolution of `~` to a home directory. The case of
/// path segments is never changed.
///
/// # Examples
///
/// ```
/// use nurt_correct::policy::{PathCheck, path_check};
///
/// assert_eq!(path_check("c:\\proj\\src\\", "C:/proj/src", None), PathCheck::Preserved);
/// assert_eq!(path_check("./src", "/srv/app/src", Some("/srv/app")), PathCheck::Preserved);
/// assert_eq!(path_check("src", "/srv/app/src", None), PathCheck::BaseChosen);
/// assert_eq!(path_check("/Srv/App", "/srv/app", None), PathCheck::Changed);
/// ```
#[must_use]
pub fn path_check(written: &str, bound: &str, base: Option<&str>) -> PathCheck {
    let normalized = normalize_path(written);
    if is_absolute(&normalized) {
        return if normalized == bound {
            PathCheck::Preserved
        } else {
            PathCheck::Changed
        };
    }
    if let Some(home) = normalized.strip_prefix('~') {
        let tail = home.trim_start_matches('/');
        let preserved =
            is_absolute(bound) && (tail.is_empty() || bound.ends_with(&format!("/{tail}")));
        return if preserved {
            PathCheck::Preserved
        } else {
            PathCheck::Changed
        };
    }
    let relative = strip_current(&normalized);
    match base {
        Some(base) => {
            if normalize_path(&format!("{base}/{relative}")) == bound {
                PathCheck::Preserved
            } else {
                PathCheck::Changed
            }
        }
        None if bound.ends_with(&format!("/{relative}")) => PathCheck::BaseChosen,
        None => PathCheck::Changed,
    }
}

fn strip_current(path: &str) -> &str {
    let mut rest = path;
    while let Some(stripped) = rest.strip_prefix("./") {
        rest = stripped;
    }
    if rest == "." { "" } else { rest }
}

fn has_drive(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

fn is_absolute(path: &str) -> bool {
    path.starts_with('/')
        || (has_drive(path) && path.as_bytes().get(2) == Some(&b'/'))
        || is_scheme(path)
}

fn is_scheme(path: &str) -> bool {
    path.split_once(":/").is_some_and(|(scheme, _)| {
        scheme.len() > 1 && scheme.bytes().all(|b| b.is_ascii_lowercase())
    })
}

fn normalize_path(path: &str) -> String {
    let slashed = path.replace('\\', "/");
    let unc = slashed.starts_with("//");
    let mut out = String::with_capacity(slashed.len());
    for c in slashed.chars() {
        if c == '/' && out.ends_with('/') && !(unc && out.len() == 1) {
            continue;
        }
        out.push(c);
    }
    if has_drive(&out) {
        let drive = out[..1].to_ascii_uppercase();
        out.replace_range(..1, &drive);
    }
    let root = if has_drive(&out) { 3 } else { 1 };
    while out.len() > root && out.ends_with('/') && !(unc && out.len() <= 2) {
        out.pop();
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LiteralKind {
    Path,
    Glob,
    Regex,
    Text,
}

#[derive(Debug, Default)]
struct SurfaceFacts {
    texts: Vec<String>,
    ints: Vec<i64>,
    durations: Vec<u64>,
    pipelines: Vec<Vec<String>>,
}

impl SurfaceFacts {
    fn collect(document: &AstDocument, input: &InvariantInput<'_>) -> Self {
        let renames = command_renames(input.corrections);
        let mut facts = Self::default();
        for statement in &document.program.statements {
            let pipeline = match statement {
                Statement::Let(binding) => &binding.value,
                Statement::Pipeline(pipeline) => pipeline,
                _ => continue,
            };
            let names = surface_stages(pipeline)
                .map(|stage| rename(&renames, &stage.name))
                .collect();
            facts.pipelines.push(names);
            facts.walk_pipeline(pipeline);
        }
        facts.add_text_numbers(input.tables);
        facts
    }

    fn walk_pipeline(&mut self, pipeline: &Pipeline) {
        for stage in surface_stages(pipeline) {
            for arg in &stage.args {
                self.walk(&arg.value);
            }
        }
    }

    fn walk(&mut self, expr: &Expr) {
        match expr {
            Expr::Text { v, .. } | Expr::Ident { v, .. } | Expr::Regex { v, .. } => {
                self.texts.push(v.clone())
            }
            Expr::Int { v, .. } => self.ints.push(*v),
            Expr::Num { v, .. } => self.add_number(*v),
            Expr::Bool { v, .. } => self.texts.push(v.to_string()),
            Expr::Dur { v, .. } => self.durations.push(*v),
            Expr::Range { lo, hi, .. } => self.ints.extend(lo.iter().chain(hi.iter())),
            Expr::List { items, .. } => items.iter().for_each(|item| self.walk(item)),
            Expr::Map { entries, .. } => entries.iter().for_each(|entry| self.walk(&entry.v)),
            Expr::Spread { x, .. } | Expr::Un { x, .. } => self.walk(x),
            Expr::Field { of: Some(of), .. } => self.walk(of),
            Expr::Call { args, .. } => args.iter().for_each(|arg| self.walk(arg)),
            Expr::Bin { l, r, .. } => {
                self.walk(l);
                self.walk(r);
            }
            Expr::Sub { p, .. } => self.walk_pipeline(p),
            _ => {}
        }
    }

    /// Keeps a number with a zero fraction as an integer; the range check
    /// makes the conversion exact.
    fn add_number(&mut self, value: f64) {
        if value.fract() == 0.0 && value.abs() < I64_LIMIT {
            self.ints.push(value as i64);
        }
    }

    fn add_text_numbers(&mut self, tables: &Tables) {
        for text in &self.texts {
            for outcome in [
                types::integer(Written::bare(text)),
                types::range(Written::bare(text)),
                types::duration(Written::quoted(text), tables),
            ] {
                if let Outcome::Converted(converted) = outcome {
                    match converted.value {
                        BoundExpr::Int { v } => self.ints.push(v),
                        BoundExpr::Range { lo, hi } => self.ints.extend(lo.iter().chain(hi.iter())),
                        BoundExpr::Dur { v } => self.durations.push(v),
                        _ => {}
                    }
                }
            }
        }
    }
}

/// Largest magnitude of a number with a fraction that is compared as an
/// integer (2^63).
const I64_LIMIT: f64 = 9_223_372_036_854_775_808.0;

fn surface_stages(pipeline: &Pipeline) -> impl Iterator<Item = &Stage> {
    let head = match &pipeline.head {
        Head::Stage(stage) => Some(stage),
        _ => None,
    };
    head.into_iter().chain(pipeline.stages.iter())
}

fn bound_stages(pipeline: &BoundPipeline) -> impl Iterator<Item = &BoundStage> {
    let head = match &pipeline.head {
        BoundHead::Stage(stage) => Some(stage),
        _ => None,
    };
    head.into_iter().chain(pipeline.stages.iter())
}

fn command_renames(corrections: &[Correction]) -> HashMap<String, String> {
    corrections
        .iter()
        .filter(|correction| correction.kind == CorrectionKind::Command)
        .map(|correction| (fold(&correction.from), correction.to.clone()))
        .collect()
}

fn rename(renames: &HashMap<String, String>, name: &str) -> String {
    renames
        .get(&fold(name))
        .cloned()
        .unwrap_or_else(|| name.to_owned())
}

/// Statement index of a value and whether it belongs to the head stage of
/// the statement.
type Origin = (usize, bool);

#[derive(Debug)]
struct BoundFacts {
    literals: Vec<(LiteralKind, String)>,
    ints: Vec<(Origin, i64)>,
    durations: Vec<(Origin, u64)>,
    pipelines: Vec<Vec<(String, Phase)>>,
    modes: Vec<(String, String, String)>,
    at: Origin,
}

impl BoundFacts {
    fn collect(document: &BoundDocument) -> Self {
        let mut facts = Self {
            literals: Vec::new(),
            ints: Vec::new(),
            durations: Vec::new(),
            pipelines: Vec::new(),
            modes: Vec::new(),
            at: (0, false),
        };
        let pipelines =
            document
                .program
                .statements
                .iter()
                .filter_map(|statement| match statement {
                    BoundStatement::Let(binding) => Some(&binding.value),
                    BoundStatement::Pipeline(pipeline) => Some(pipeline),
                    _ => None,
                });
        for (position, pipeline) in pipelines.enumerate() {
            facts.pipelines.push(
                bound_stages(pipeline)
                    .map(|stage| (stage.name.clone(), stage.phase))
                    .collect(),
            );
            facts.walk_pipeline(pipeline, position, false);
        }
        facts
    }

    fn walk_pipeline(&mut self, pipeline: &BoundPipeline, position: usize, nested: bool) {
        for (index, stage) in bound_stages(pipeline).enumerate() {
            for arg in stage.args.iter().filter(|arg| !arg.default) {
                self.at = (position, !nested && index == 0);
                self.add_arg(stage, arg);
            }
        }
    }

    fn add_arg(&mut self, stage: &BoundStage, arg: &BoundArg) {
        let protected = COUNTING_STAGES.contains(&stage.name.as_str())
            || NUMBER_PARAMS.contains(&arg.name.as_str());
        self.walk(&arg.value, protected);
        let mode = MODE_PARAMS.contains(&arg.name.as_str())
            || (stage.name == CREATE_STAGE && arg.name == TYPE_PARAM);
        if let (true, Some(value)) = (mode, scalar_text(&arg.value)) {
            self.modes
                .push((stage.name.clone(), arg.name.clone(), value));
        }
    }

    fn walk(&mut self, expr: &BoundExpr, protected: bool) {
        match expr {
            BoundExpr::Path { v } => self.literals.push((LiteralKind::Path, v.clone())),
            BoundExpr::Glob { v } => self.literals.push((LiteralKind::Glob, v.clone())),
            BoundExpr::Regex { v, .. } => self.literals.push((LiteralKind::Regex, v.clone())),
            BoundExpr::Text { v } => self.literals.push((LiteralKind::Text, v.clone())),
            BoundExpr::Int { v } if protected => self.ints.push((self.at, *v)),
            BoundExpr::Dur { v } if protected => self.durations.push((self.at, *v)),
            BoundExpr::Range { lo, hi } => {
                let at = self.at;
                self.ints
                    .extend(lo.iter().chain(hi.iter()).map(|bound| (at, *bound)));
            }
            BoundExpr::List { items } => items.iter().for_each(|item| self.walk(item, protected)),
            BoundExpr::Map { entries } => entries
                .iter()
                .for_each(|entry| self.walk(&entry.v, protected)),
            BoundExpr::Spread { x } | BoundExpr::Un { x, .. } => self.walk(x, protected),
            BoundExpr::Field { of: Some(of), .. } => self.walk(of, protected),
            BoundExpr::Call { args, .. } => args.iter().for_each(|arg| self.walk(arg, protected)),
            BoundExpr::Bin { l, r, .. } => {
                self.walk(l, protected);
                self.walk(r, protected);
            }
            BoundExpr::Sub { p } => {
                let saved = self.at;
                self.walk_pipeline(p, saved.0, true);
                self.at = saved;
            }
            _ => {}
        }
    }
}

fn scalar_text(expr: &BoundExpr) -> Option<String> {
    match expr {
        BoundExpr::Enum { v } | BoundExpr::Text { v } => Some(v.clone()),
        BoundExpr::Bool { v } => Some(v.to_string()),
        _ => None,
    }
}

fn check_renamed_effects(input: &InvariantInput<'_>) -> Result<(), InvariantViolation> {
    let renamed = input.corrections.iter().find(|correction| {
        correction.kind == CorrectionKind::Command
            && (input.effecting)(&correction.from)
            && fold(&correction.from) != fold(&correction.to)
    });
    match renamed {
        Some(correction) => Err(InvariantViolation::new(
            Invariant::EffectKind,
            format!(
                "the changing command `{}` was replaced by `{}`",
                correction.from, correction.to
            ),
        )),
        None => Ok(()),
    }
}

fn check_literals(
    before: &SurfaceFacts,
    after: &BoundFacts,
    base: Option<&str>,
) -> Result<(), InvariantViolation> {
    let mut unused: Vec<&str> = before.texts.iter().map(String::as_str).collect();
    for (kind, value) in &after.literals {
        let found = unused.iter().position(|text| match kind {
            LiteralKind::Path => path_check(text, value, base) == PathCheck::Preserved,
            LiteralKind::Glob | LiteralKind::Regex | LiteralKind::Text => text == value,
        });
        if let Some(position) = found {
            unused.swap_remove(position);
            continue;
        }
        let base_chosen = *kind == LiteralKind::Path
            && unused
                .iter()
                .any(|text| path_check(text, value, base) == PathCheck::BaseChosen);
        return Err(if base_chosen {
            InvariantViolation::new(
                Invariant::Base,
                format!("the relative path was resolved to `{value}` without a base"),
            )
        } else {
            InvariantViolation::new(
                Invariant::LiteralBytes,
                format!("the literal `{value}` does not appear in the program"),
            )
        });
    }
    Ok(())
}

/// Statements whose head stage was added by the expansion of a source form
/// (`remove "/a"` binds as `find "/a" depth: 0 |> remove`).
fn expanded_heads(before: &SurfaceFacts, after: &BoundFacts) -> Vec<bool> {
    let selecting = |name: &str| SELECTING_STAGES.contains(&name);
    before
        .pipelines
        .iter()
        .zip(&after.pipelines)
        .map(|(written, bound)| {
            written.first().is_some_and(|head| !selecting(head))
                && bound.first().is_some_and(|(head, _)| selecting(head))
                && bound.len() == written.len() + 1
        })
        .collect()
}

fn check_numbers(before: &SurfaceFacts, after: &BoundFacts) -> Result<(), InvariantViolation> {
    let expanded = expanded_heads(before, after);
    let checked =
        |&(statement, head): &Origin| !(head && expanded.get(statement).copied().unwrap_or(false));
    let int = after
        .ints
        .iter()
        .filter(|(origin, _)| checked(origin))
        .map(|&(_, value)| value)
        .find(|value| !before.ints.contains(value));
    if let Some(value) = int {
        return Err(InvariantViolation::new(
            Invariant::Numbers,
            format!("the number {value} does not appear in the program"),
        ));
    }
    let duration = after
        .durations
        .iter()
        .filter(|(origin, _)| checked(origin))
        .map(|&(_, value)| value)
        .find(|value| !before.durations.contains(value));
    match duration {
        Some(value) => Err(InvariantViolation::new(
            Invariant::Numbers,
            format!("the duration of {value} ms does not appear in the program"),
        )),
        None => Ok(()),
    }
}

fn check_stages(
    before: &SurfaceFacts,
    after: &BoundFacts,
    input: &InvariantInput<'_>,
) -> Result<(), InvariantViolation> {
    if before.pipelines.len() != after.pipelines.len() {
        return Err(InvariantViolation::new(
            Invariant::Targets,
            "the number of statements changed",
        ));
    }
    for (written, bound) in before.pipelines.iter().zip(&after.pipelines) {
        check_selecting(written, bound)?;
        check_effects(written, bound, input.effecting)?;
    }
    Ok(())
}

fn check_selecting(
    written: &[String],
    bound: &[(String, Phase)],
) -> Result<(), InvariantViolation> {
    let selecting = |name: &str| SELECTING_STAGES.contains(&name);
    let mut before: Vec<&str> = written
        .iter()
        .map(String::as_str)
        .filter(|name| selecting(name))
        .collect();
    let mut after: Vec<&str> = bound
        .iter()
        .map(|(name, _)| name.as_str())
        .filter(|name| selecting(name))
        .collect();
    let expanded_head = written.first().is_some_and(|head| !selecting(head))
        && bound.first().is_some_and(|(head, _)| selecting(head));
    if expanded_head && after.len() == before.len() + 1 {
        after.remove(0);
    }
    before.sort_unstable();
    after.sort_unstable();
    if before == after {
        Ok(())
    } else {
        Err(InvariantViolation::new(
            Invariant::Targets,
            format!("the stages that select targets changed from {before:?} to {after:?}"),
        ))
    }
}

fn check_effects(
    written: &[String],
    bound: &[(String, Phase)],
    effecting: &dyn Fn(&str) -> bool,
) -> Result<(), InvariantViolation> {
    let mut before: Vec<&str> = written
        .iter()
        .map(String::as_str)
        .filter(|name| effecting(name))
        .collect();
    let mut after: Vec<&str> = bound
        .iter()
        .filter(|(_, phase)| matches!(phase, Phase::Mutate | Phase::Run))
        .map(|(name, _)| name.as_str())
        .collect();
    before.sort_unstable();
    after.sort_unstable();
    if before == after {
        Ok(())
    } else {
        Err(InvariantViolation::new(
            Invariant::EffectKind,
            format!("the changing stages changed from {before:?} to {after:?}"),
        ))
    }
}

fn check_modes(
    before: &SurfaceFacts,
    after: &BoundFacts,
    input: &InvariantInput<'_>,
) -> Result<(), InvariantViolation> {
    for (stage, param, value) in &after.modes {
        let written = before.texts.iter().any(|text| fold(text) == fold(value));
        let corrected = input
            .corrections
            .iter()
            .any(|correction| is_mode_correction(correction, value));
        if !written && !corrected {
            return Err(InvariantViolation::new(
                Invariant::ModeParameters,
                format!("`{param}: {value}` of `{stage}` does not come from the program"),
            ));
        }
    }
    Ok(())
}

/// A mode value may come from a value correction of the written text (an
/// exact, table or fuzzy match), never from a prefix.
fn is_mode_correction(correction: &Correction, value: &str) -> bool {
    matches!(
        correction.kind,
        CorrectionKind::Value | CorrectionKind::Bool
    ) && correction.rule != RuleId::L05
        && fold(&correction.to) == fold(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    const READ: ProgramKind = ProgramKind::Read;
    const EFFECT: ProgramKind = ProgramKind::Effect;

    #[test]
    fn applies_certain_corrections_everywhere() {
        let facts = Facts::default();
        assert_eq!(decide(Class::Certain, READ, &facts), Decision::Apply);
        assert_eq!(decide(Class::Certain, EFFECT, &facts), Decision::Apply);
    }

    #[test]
    fn applies_structural_corrections_of_effects_only_without_doubt() {
        let facts = Facts::default();
        assert_eq!(
            decide(Class::Structural, READ, &facts.structural(false, false)),
            Decision::Apply
        );
        assert_eq!(decide(Class::Structural, EFFECT, &facts), Decision::Apply);
        assert_eq!(
            decide(Class::Structural, EFFECT, &facts.structural(false, true)),
            Decision::Ambiguous
        );
        assert_eq!(
            decide(Class::Structural, EFFECT, &facts.structural(true, false)),
            Decision::Ambiguous
        );
    }

    #[test]
    fn limits_fuzzy_names_of_effecting_stages() {
        let facts = Facts::default();
        assert_eq!(
            decide(Class::Fuzzy, READ, &facts.fuzzy(true, 2)),
            Decision::Apply
        );
        assert_eq!(
            decide(Class::Fuzzy, EFFECT, &facts.fuzzy(false, 2)),
            Decision::Apply
        );
        assert_eq!(
            decide(Class::Fuzzy, EFFECT, &facts.fuzzy(true, 1)),
            Decision::ApplyIfProgramChecks
        );
        assert_eq!(
            decide(Class::Fuzzy, EFFECT, &facts.fuzzy(true, 2)),
            Decision::Ambiguous
        );
    }

    #[test]
    fn never_applies_probable_corrections_of_effects() {
        let facts = Facts::default();
        assert_eq!(
            decide(Class::Probable, READ, &facts),
            Decision::ApplyWithAlternatives
        );
        assert_eq!(decide(Class::Probable, EFFECT, &facts), Decision::Ambiguous);
    }

    #[test]
    fn respects_the_switch() {
        let mut config = CorrectionConfig::default();
        config.enabled = false;
        let facts = Facts::new(&config);
        for class in Class::ALL {
            assert_eq!(decide(*class, READ, &facts), Decision::Disabled);
        }
        assert!(Facts::new(&CorrectionConfig::default()).enabled);
    }

    #[test]
    fn counts_the_budget_by_program_kind() {
        let budget = Budget::from(&CorrectionConfig::default());
        assert_eq!(budget, Budget::default());
        assert_eq!(budget.limit(READ), DEFAULT_MAX_READ);
        assert_eq!(budget.limit(EFFECT), DEFAULT_MAX_EFFECT);
        assert!(!over_budget(4, EFFECT, &budget));
        assert!(over_budget(9, READ, &budget));
        assert!(!over_budget(
            0,
            READ,
            &Budget {
                max_read: 0,
                max_effect: 0
            }
        ));
    }

    #[test]
    fn classifies_programs_by_phase() {
        assert_eq!(ProgramKind::from_phases([]), READ);
        assert_eq!(ProgramKind::from_phases([Phase::Run]), EFFECT);
        assert_eq!(
            ProgramKind::from_phases([Phase::Check, Phase::Transform]),
            READ
        );
    }
}
