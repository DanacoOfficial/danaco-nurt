// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-correct::lexical::front
// Responsibility:   Front part of correction: from program text to a surface tree.
// Dependencies:     nurt_contracts, nurt_syntax
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Front part of correction: from program text to a surface tree.
//!
//! [`front`] runs the following steps in this order and stops at the first
//! that yields a tree:
//!
//! 1. Strict parse of the text as written. A program that parses strictly
//!    gets no correction, unless a statement of it is a habit of another
//!    shell: its first word is a habit word that is not a Nurt command and
//!    it shows notation that exists only in that shell (`rm -rf /srv/x`,
//!    `curl -s https://example.com`). Such a program continues with step 3,
//!    because the strict parser reads the flags and the URL as expressions.
//! 2. Character normalization: Unicode spaces and zero-width characters
//!    outside literals (`L17`).
//! 3. Habits of other shells: a statement that does not parse strictly and
//!    is a bash or PowerShell command is replaced by its translation, with
//!    the corrections of the translation rules.
//! 4. Another syntax variant (`L20`, only with `syntax.tolerant`): a
//!    statement that parses strictly neither as written nor after the token
//!    rules, but parses strictly in other variants with one tree, is written
//!    in the configured variant. Readings with different trees give
//!    `E_AMBIGUOUS`.
//! 5. Token rules `L03`, `L04`, `L07`, `L13`, `L16` and `L21`.
//! 6. Parse with the repair policy of the rules `L10`, `L11`, `L12`, `L13`,
//!    `L14`, `L15`, `L08`, `L09`: a strict parse first, then at most three
//!    repairs per statement; other repaired readings become alternatives.
//! 7. Tree rules `L14` (a command name after the arguments of a stage) and
//!    `L18` (positional arguments after named ones).
//!
//! Every correction has its position in the text as written, and the list
//! is sorted by position and numbered `C1`, `C2`, …. The rules that need
//! the type of an argument place (`L01`, `L02`, `L05`, `L06`, `L19`, `L22`,
//! `L23`, `L24`) run during binding.

use std::ops::Range;

use nurt_contracts::ast::{AstDocument, Expr, Head, Span, Statement};
use nurt_contracts::config::{CorrectionConfig, SyntaxConfig};
use nurt_contracts::error::{At, Candidate, Error, ErrorCode, Stage};
use nurt_contracts::report::TextPosition;
use nurt_syntax::parse::{
    ParseOptions, Repaired, Variant, parse, parse_keeping_escapes, parse_tolerant,
    parse_tolerant_keeping_escapes,
};
use nurt_syntax::print::try_print;

use super::locate::{Locator, offset_of};
use super::repairs::LexicalRepairs;
use super::rules::{Rule, number_corrections};
use super::scan::{Piece, PieceKind, scan};
use super::tokens::{TokenContext, rewrite, rewrite_located};
use super::tree::{TreeChange, TreeRule, reorder_arguments, split_missing_pipes};
use super::{ArgumentShape, CatalogView};
use crate::bash::{self, Assumption, TranslateContext};
use crate::distance::NameKind;
use crate::normalize::{Edit, Normalized, PositionMap, apply_edits, normalize};
use crate::powershell;
use crate::record::{Correction, CorrectionKind};
use crate::tables::Tables;

/// Maximum number of alternatives of a result.
pub const MAX_ALTERNATIVES: usize = 3;

/// Every syntax variant, in the order in which other readings are tried.
const VARIANTS: [Variant; 3] = [Variant::A, Variant::B, Variant::C];

/// Operators after which a line break continues the statement.
const CONTINUING: [&str; 6] = ["|>", "|", "->", "|->", ",", "="];

/// Result of the front part of correction.
#[derive(Debug, Clone, PartialEq)]
pub struct Front {
    /// Surface tree of the corrected program. Its spans refer to
    /// [`Front::source`]; arguments moved by `L18` and stages split by `L14`
    /// keep the spans they were written with.
    pub document: AstDocument,
    /// Corrections in the order of their position in the text as written,
    /// numbered `C1`, `C2`, …; empty for a program that parses strictly.
    pub corrections: Vec<Correction>,
    /// Other readings with different trees, the likeliest first; at most
    /// [`MAX_ALTERNATIVES`]. Alternatives have no alternatives of their own.
    pub alternatives: Vec<Front>,
    /// Assumptions of the translation of habits of other shells.
    pub assumptions: Vec<Assumption>,
    /// The text after character normalization.
    pub normalized: String,
    /// The text the tree was parsed from.
    pub source: String,
}

impl Front {
    fn strict(text: &str, document: AstDocument) -> Self {
        Self {
            document,
            corrections: Vec::new(),
            alternatives: Vec::new(),
            assumptions: Vec::new(),
            normalized: text.to_owned(),
            source: text.to_owned(),
        }
    }
}

/// Reads a program text with the lexical correction rules.
///
/// The steps and their order are described in the [module](self)
/// documentation. With `correction.enabled = false` the function is a strict
/// parse. With `syntax.tolerant = false` rule `L20` does not apply.
///
/// # Errors
///
/// Returns the error of the strict parse of the text as written (`E_SYNTAX`,
/// `E_LEXICAL`, `E_ESCAPE`, with ready fixes where the parser has them) when
/// correction is disabled or no reading yields a tree; `E_AMBIGUOUS` with
/// candidates when a statement reads differently in two other variants; and
/// the error of the translation of a habit of another shell, such as
/// `E_ONE_EFFECT`.
///
/// # Examples
///
/// ```
/// use nurt_contracts::config::{CorrectionConfig, SyntaxConfig};
/// use nurt_correct::lexical::{ArgumentShape, CatalogView, front};
/// use nurt_correct::tables::load_embedded;
///
/// struct Commands;
///
/// impl CatalogView for Commands {
///     fn is_command(&self, name: &str) -> bool {
///         ["find", "search"].contains(&name)
///     }
///     fn command_names(&self) -> &[&str] {
///         &["find", "search"]
///     }
///     fn is_mutating(&self, _: &str) -> bool {
///         false
///     }
///     fn can_take_argument(&self, _: &str, _: usize, _: ArgumentShape) -> bool {
///         false
///     }
/// }
///
/// let tables = load_embedded()?;
/// let syntax = SyntaxConfig::default();
/// let correction = CorrectionConfig::default();
/// let result = front("Find /srv/app | search /x/", &syntax, &correction, &tables, &Commands)?;
/// let rules: Vec<String> = result.corrections.iter().map(|c| c.rule.to_string()).collect();
/// assert_eq!(rules, ["L03", "L21", "L13"]);
/// assert_eq!(result.source, "find \"/srv/app\" |> search /x/");
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn front(
    text: &str,
    syntax: &SyntaxConfig,
    correction: &CorrectionConfig,
    tables: &Tables,
    catalog: &dyn CatalogView,
) -> Result<Front, Error> {
    read(text, syntax, correction, tables, catalog, None, false)
}

/// Reads a program text like [`front`], for the binder.
///
/// The translation of a habit of another shell takes the directory from the
/// call parameter `base` where the habit gives none, for example the working
/// directory of `timeout 60 make test`, and offers it in the fixes of its
/// errors. A text with escapes that the language does not have (`"\d+"`)
/// does not end the reading with `E_ESCAPE`: the tree keeps the characters of
/// the text and the binder reads it according to its place (`L24`), see
/// [`paths::reinterpret_escapes`](super::paths::reinterpret_escapes).
///
/// # Errors
///
/// As [`front`], except `E_ESCAPE` for a text that the program reads with no
/// other repair.
pub fn front_for_binding(
    text: &str,
    syntax: &SyntaxConfig,
    correction: &CorrectionConfig,
    tables: &Tables,
    catalog: &dyn CatalogView,
    base: Option<&str>,
) -> Result<Front, Error> {
    read(text, syntax, correction, tables, catalog, base, true)
}

fn read(
    text: &str,
    syntax: &SyntaxConfig,
    correction: &CorrectionConfig,
    tables: &Tables,
    catalog: &dyn CatalogView,
    base: Option<&str>,
    keep_escapes: bool,
) -> Result<Front, Error> {
    let variant = syntax.variant;
    let options = ParseOptions::default();
    let is_habit = |word: &str| tables.habits().any(|habit| habit == word);
    let context = TokenContext {
        variant,
        catalog,
        is_habit: &is_habit,
    };
    let keep_escapes = keep_escapes && correction.enabled;
    let read_strict = if keep_escapes {
        parse_keeping_escapes
    } else {
        parse
    };
    let strict = match read_strict(text, variant, &options) {
        Ok(document) if !correction.enabled || !has_habit_statement(text, &context, tables) => {
            return Ok(Front::strict(text, document));
        }
        Err(error) if !correction.enabled => return Err(error.into()),
        strict => strict,
    };
    let mut work = Work::new(text, normalize(text));
    work.translate_habits(&context, tables, base)?;
    if strict.is_ok() && work.text == work.normalized {
        return strict
            .map(|document| Front::strict(text, document))
            .map_err(Into::into);
    }
    if syntax.tolerant {
        work.other_variants(&context)?;
    }
    work.rewrite(&context);
    let policy = LexicalRepairs::new(variant, catalog, &is_habit);
    let read_tolerant = if keep_escapes {
        parse_tolerant_keeping_escapes
    } else {
        parse_tolerant
    };
    let Ok(repaired) = read_tolerant(&work.text, variant, &options, &policy) else {
        return strict
            .map(|document| Front::strict(text, document))
            .map_err(Into::into);
    };
    Ok(work.finish(repaired, variant, catalog))
}

/// State of the text between the steps of [`front`].
struct Work<'a> {
    original: &'a str,
    normalized: String,
    text: String,
    map: PositionMap,
    corrections: Vec<Correction>,
    assumptions: Vec<Assumption>,
}

/// Result of the translation of one statement.
struct Habit {
    tree: AstDocument,
    rules: Vec<Correction>,
    assumptions: Vec<Assumption>,
}

impl<'a> Work<'a> {
    fn new(original: &'a str, normalized: Normalized) -> Self {
        Self {
            original,
            text: normalized.text.clone(),
            normalized: normalized.text,
            map: normalized.positions,
            corrections: normalized.corrections,
            assumptions: Vec::new(),
        }
    }

    fn locator(&self) -> Locator<'a> {
        Locator::new(self.original, self.map.clone())
    }

    fn edit(&mut self, edits: &[Edit]) {
        if edits.is_empty() {
            return;
        }
        let (text, map) = apply_edits(&self.text, edits);
        self.map = self.map.then(&map);
        self.text = text;
    }

    fn rewrite(&mut self, context: &TokenContext<'_>) {
        let locator = self.locator();
        let (text, corrections, map) = rewrite_located(&self.text, context, &locator);
        self.corrections.extend(corrections);
        self.map = self.map.then(&map);
        self.text = text;
    }

    /// Step 3: statements written as habits of other shells; a statement
    /// that the token rules make valid is not a habit, unless it starts with
    /// a word that is both a habit and a command and shows a shell feature
    /// (`sort -u /a/b`).
    fn translate_habits(
        &mut self,
        context: &TokenContext<'_>,
        tables: &Tables,
        base: Option<&str>,
    ) -> Result<(), Error> {
        let variant = context.variant;
        let locator = self.locator();
        let mut edits = Vec::new();
        for range in statements(&self.text) {
            let statement = self.text.get(range.clone()).unwrap_or_default();
            let kind = habit_statement(statement, context, tables);
            if parses(statement, variant) && kind.is_none() {
                continue;
            }
            if parses(&rewrite(statement, context).text, variant) && kind.is_none() {
                continue;
            }
            let by_word = kind == Some(HabitKind::Word);
            let Some(habit) = translate_statement(statement, by_word, base)? else {
                continue;
            };
            let Ok(printed) = try_print(&habit.tree, variant) else {
                continue;
            };
            let start = locator.at(range.start..range.start);
            self.corrections.extend(
                habit
                    .rules
                    .into_iter()
                    .map(|correction| shift(correction, start)),
            );
            self.assumptions.extend(habit.assumptions);
            edits.push(Edit::new(range, printed.trim_end()));
        }
        self.edit(&edits);
        Ok(())
    }

    /// Step 4: statements written in another syntax variant (`L20`).
    fn other_variants(&mut self, context: &TokenContext<'_>) -> Result<(), Error> {
        let variant = context.variant;
        let locator = self.locator();
        let mut edits = Vec::new();
        for range in statements(&self.text) {
            let owned = self.text.get(range.clone()).unwrap_or_default().to_owned();
            let statement = owned.as_str();
            if parses(statement, variant) || parses(&rewrite(statement, context).text, variant) {
                continue;
            }
            match other_readings(statement, variant, context.catalog).as_slice() {
                [] => self.other_variant_stages(&range, context, &locator, &mut edits)?,
                [(_, reading)] => {
                    let at = locator.at(range.clone());
                    self.corrections.push(Rule::L20.record(
                        at,
                        statement,
                        reading.clone(),
                        CorrectionKind::Syntax,
                    ));
                    edits.push(Edit::new(range, reading.clone()));
                }
                readings => return Err(self.ambiguous_variant(&range, readings, &locator)),
            }
        }
        self.edit(&edits);
        Ok(())
    }

    /// Stage by stage `L20` for a statement that reads in no other variant
    /// as a whole: each stage that does not read in the configured variant
    /// is written in it when exactly one other variant reads it. The
    /// statement is changed only when the result reads in the configured
    /// variant.
    fn other_variant_stages(
        &mut self,
        range: &Range<usize>,
        context: &TokenContext<'_>,
        locator: &Locator<'_>,
        edits: &mut Vec<Edit>,
    ) -> Result<(), Error> {
        let variant = context.variant;
        let text = self.text.clone();
        let mut staged = Vec::new();
        for stage in stage_ranges(&text, range) {
            let source = text.get(stage.clone()).unwrap_or_default();
            if parses(source, variant) || parses(&rewrite(source, context).text, variant) {
                continue;
            }
            match other_readings(source, variant, context.catalog).as_slice() {
                [] => return Ok(()),
                [(_, reading)] => staged.push((stage, reading.clone())),
                readings => return Err(self.ambiguous_variant(&stage, readings, locator)),
            }
        }
        if staged.is_empty() {
            return Ok(());
        }
        let mut changed = text.clone();
        for (stage, reading) in staged.iter().rev() {
            changed.replace_range(stage.clone(), reading);
        }
        let statement = changed
            .get(range.start..changed.len() - (text.len() - range.end))
            .unwrap_or_default();
        if !parses(statement, variant) && !parses(&rewrite(statement, context).text, variant) {
            return Ok(());
        }
        for (stage, reading) in staged {
            let source = text.get(stage.clone()).unwrap_or_default();
            self.corrections.push(Rule::L20.record(
                locator.at(stage.clone()),
                source,
                reading.clone(),
                CorrectionKind::Syntax,
            ));
            edits.push(Edit::new(stage, reading));
        }
        Ok(())
    }

    fn ambiguous_variant(
        &self,
        range: &Range<usize>,
        readings: &[(Variant, String)],
        locator: &Locator<'_>,
    ) -> Error {
        let at = locator.at(range.clone());
        let mut error = Error::new(
            ErrorCode::Ambiguous,
            "the statement reads differently in two syntax variants; write it in the configured variant",
        )
        .with_stage(Stage::Parse)
        .with_at(to_at(at))
        .with_found(self.text.get(range.clone()).unwrap_or_default())
        .with_help("help corrections");
        for (rank, (variant, reading)) in readings.iter().take(MAX_ALTERNATIVES).enumerate() {
            let mut program = self.text.clone();
            program.replace_range(range.clone(), reading);
            error = error.with_candidate(Candidate {
                rank: u32::try_from(rank + 1).unwrap_or(u32::MAX),
                reference: None,
                program,
                why: Some(format!("read in variant {}", variant_name(*variant))),
            });
        }
        error
    }

    /// Steps 6 and 7 for the repaired reading and its alternatives.
    fn finish(&self, mut repaired: Repaired, variant: Variant, catalog: &dyn CatalogView) -> Front {
        let others: Vec<Repaired> = repaired
            .alternatives
            .drain(..)
            .take(MAX_ALTERNATIVES)
            .collect();
        let mut result = self.reading(repaired, variant, catalog);
        result.alternatives = others
            .into_iter()
            .map(|other| self.reading(other, variant, catalog))
            .collect();
        result
    }

    fn reading(&self, repaired: Repaired, variant: Variant, catalog: &dyn CatalogView) -> Front {
        let mut corrections = self.corrections.clone();
        let locator = self.locator();
        let mut edits = Vec::with_capacity(repaired.repairs.len());
        for applied in &repaired.repairs {
            let Some(rule) = Rule::ALL
                .iter()
                .copied()
                .find(|rule| rule.id() == applied.rule)
            else {
                continue;
            };
            let start = offset_of(
                &self.text,
                applied.at.line.unwrap_or(1),
                applied.at.column.unwrap_or(1),
            );
            let range = start..start + applied.from.len();
            corrections.push(rule.record(
                locator.at(range.clone()),
                applied.from.clone(),
                applied.to.clone(),
                repair_kind(rule),
            ));
            edits.push(Edit::new(range, applied.to.clone()));
        }
        edits.sort_by_key(|edit| edit.range.start);
        let (replayed, map) = apply_edits(&self.text, &edits);
        let source_map = if replayed == repaired.source {
            self.map.then(&map)
        } else {
            self.map.clone()
        };
        let source_locator = Locator::new(self.original, source_map);
        let mut document = repaired.document;
        let changes: Vec<TreeChange> = split_missing_pipes(&mut document, catalog)
            .into_iter()
            .chain(reorder_arguments(&mut document, variant))
            .collect();
        for change in changes {
            let at = span_range(&repaired.source, change.span)
                .map_or_else(|| source_locator.at(0..0), |range| source_locator.at(range));
            let (rule, kind) = match change.rule {
                TreeRule::MissingPipe => (Rule::L14, CorrectionKind::Pipe),
                TreeRule::ArgumentOrder => (Rule::L18, CorrectionKind::Order),
            };
            corrections.push(rule.record(at, change.from, change.to, kind));
        }
        corrections.sort_by_key(|correction| correction.at.map(|at| (at.line, at.column)));
        number_corrections(&mut corrections);
        Front {
            document,
            corrections,
            alternatives: Vec::new(),
            assumptions: self.assumptions.clone(),
            normalized: self.normalized.clone(),
            source: repaired.source,
        }
    }
}

/// Whether a statement of the text is a habit of another shell although the
/// text may parse strictly.
fn has_habit_statement(text: &str, context: &TokenContext<'_>, tables: &Tables) -> bool {
    statements(text).into_iter().any(|range| {
        text.get(range)
            .is_some_and(|statement| habit_statement(statement, context, tables).is_some())
    })
}

/// What shows that a statement is a habit of another shell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HabitKind {
    /// Notation that only the shell has: a flag, an operator, an assignment.
    Notation,
    /// The first word alone, because Nurt has no command or synonym of that
    /// name (`touch /srv/app/a.txt`).
    Word,
}

/// Whether a statement is a habit of another shell that the strict parser
/// may read as a Nurt program.
///
/// - An assignment before the program (`CI=1 npm test`) is shell notation.
/// - A habit word that is a Nurt command counts only with a dashed flag
///   (`sort -u`), as no Nurt command has that notation.
/// - A habit word that is neither a command nor a synonym of one counts
///   whatever follows it.
/// - A habit word that is a synonym of a command (`cat`, `ps`, `which`)
///   counts with a feature of the shell, or when the statement gives the
///   command a bare word that it cannot take (`ps aux`, `which git`).
fn habit_statement(
    statement: &str,
    context: &TokenContext<'_>,
    tables: &Tables,
) -> Option<HabitKind> {
    let first = statement
        .split(char::is_whitespace)
        .next()
        .unwrap_or_default();
    if is_assignment(first) {
        return bash::detect(statement).then_some(HabitKind::Notation);
    }
    if !(context.is_habit)(first) {
        return None;
    }
    if context.catalog.is_command(first) {
        return is_command_habit(statement, context).then_some(HabitKind::Notation);
    }
    let Some(synonym) = tables.synonym_entry(NameKind::Command, first) else {
        if bash::detect(statement) || powershell::detect(statement) {
            return Some(HabitKind::Notation);
        }
        return bash::detect_by_word(statement).then_some(HabitKind::Word);
    };
    if bash::detect(statement) || powershell::detect(statement) {
        return Some(HabitKind::Notation);
    }
    (gives_bare_word(statement, &synonym.to, context) && bash::detect_by_word(statement))
        .then_some(HabitKind::Word)
}

/// Whether a word is an assignment of the shell, `NAME=value`.
fn is_assignment(word: &str) -> bool {
    word.split_once('=').is_some_and(|(name, _)| {
        name.starts_with(|c: char| c == '_' || c.is_ascii_alphabetic())
            && name.chars().all(|c| c == '_' || c.is_ascii_alphanumeric())
    })
}

/// Whether a statement gives the command `target` a positional argument
/// written as a bare word that the command cannot take: `ps aux` read as
/// `processes aux`, where a bare word is neither text nor a path.
fn gives_bare_word(statement: &str, target: &str, context: &TokenContext<'_>) -> bool {
    let Ok(document) = parse(statement, context.variant, &ParseOptions::default()) else {
        return false;
    };
    let Some(Statement::Pipeline(pipeline)) = document.program.statements.first() else {
        return false;
    };
    let Head::Stage(stage) = &pipeline.head else {
        return false;
    };
    stage
        .args
        .iter()
        .filter(|arg| arg.name.is_none())
        .enumerate()
        .any(|(position, arg)| {
            matches!(arg.value, Expr::Ident { .. })
                && !context
                    .catalog
                    .can_take_argument(target, position, ArgumentShape::Word)
        })
}

/// Whether a statement starts with a word that is a habit of another shell
/// and also a Nurt command, and has an argument written as a dashed flag
/// (`sort -u`), which Nurt has no notation for.
fn is_command_habit(statement: &str, context: &TokenContext<'_>) -> bool {
    let first = statement
        .split(char::is_whitespace)
        .next()
        .unwrap_or_default();
    if !(context.is_habit)(first) || !context.catalog.is_command(first) {
        return false;
    }
    let pieces = scan(statement);
    let has_flag = pieces.windows(3).any(|window| {
        let [space, dash, word] = window else {
            return false;
        };
        space.kind == PieceKind::Space
            && dash.kind == PieceKind::Punct
            && matches!(dash.text(statement), "-" | "--")
            && word.kind == PieceKind::Word
    });
    has_flag && bash::detect(statement)
}

/// Byte ranges of the stages of a statement: the parts between the pipes
/// `|>` and `|` outside brackets and literals, without surrounding space.
fn stage_ranges(text: &str, statement: &Range<usize>) -> Vec<Range<usize>> {
    let slice = text.get(statement.clone()).unwrap_or_default();
    let mut stages = Vec::new();
    let mut depth = 0usize;
    let mut start = statement.start;
    for piece in scan(slice) {
        if piece.kind != PieceKind::Punct {
            continue;
        }
        match piece.text(slice) {
            "(" | "[" | "{" => depth += 1,
            ")" | "]" | "}" => depth = depth.saturating_sub(1),
            "|>" | "|" if depth == 0 => {
                stages.push(start..statement.start + piece.range.start);
                start = statement.start + piece.range.end;
            }
            _ => {}
        }
    }
    stages.push(start..statement.end);
    stages
        .into_iter()
        .filter_map(|stage| {
            let source = text.get(stage.clone())?;
            let lead = source.len() - source.trim_start().len();
            let trimmed = source.trim();
            (!trimmed.is_empty()).then(|| stage.start + lead..stage.start + lead + trimmed.len())
        })
        .collect()
}

fn parses(text: &str, variant: Variant) -> bool {
    parse(text, variant, &ParseOptions::default()).is_ok()
}

/// Readings of a statement in the other variants, written in `variant`;
/// readings with the same text are listed once. A reading that takes a
/// command name for an argument value (`find("/a") count()` read in word
/// notation) is not a reading of the statement.
fn other_readings(
    statement: &str,
    variant: Variant,
    catalog: &dyn CatalogView,
) -> Vec<(Variant, String)> {
    let mut readings: Vec<(Variant, String)> = Vec::new();
    for other in VARIANTS.into_iter().filter(|other| *other != variant) {
        let Ok(document) = parse(statement, other, &ParseOptions::default()) else {
            continue;
        };
        if names_a_command_as_value(&document, catalog) {
            continue;
        }
        let Ok(printed) = try_print(&document, variant) else {
            continue;
        };
        let reading = printed.trim_end().to_owned();
        let known = readings.iter().any(|(_, text)| *text == reading);
        if !known && parses(&reading, variant) {
            readings.push((other, reading));
        }
    }
    readings
}

/// Whether a positional argument of a stage is a command name, bare
/// (`count`) or called (`count()`).
fn names_a_command_as_value(document: &AstDocument, catalog: &dyn CatalogView) -> bool {
    let stages = document
        .program
        .statements
        .iter()
        .filter_map(|statement| match statement {
            Statement::Let(binding) => Some(&binding.value),
            Statement::Pipeline(pipeline) => Some(pipeline),
            _ => None,
        })
        .flat_map(|pipeline| {
            let head = match &pipeline.head {
                Head::Stage(stage) => Some(stage),
                _ => None,
            };
            head.into_iter().chain(pipeline.stages.iter())
        });
    stages
        .flat_map(|stage| stage.args.iter())
        .filter(|arg| arg.name.is_none())
        .any(|arg| match &arg.value {
            Expr::Ident { v, .. } => catalog.is_command(v),
            Expr::Call { function, .. } => catalog.is_command(function),
            _ => false,
        })
}

/// Translates a statement of another shell; `by_word` says that its first
/// word alone shows the habit.
fn translate_statement(
    statement: &str,
    by_word: bool,
    base: Option<&str>,
) -> Result<Option<Habit>, Error> {
    let context = TranslateContext {
        base,
        ..TranslateContext::default()
    };
    let bash = if bash::detect(statement) {
        bash::translate_with(statement, &context)
    } else if by_word {
        bash::translate_by_word(statement, &context)
    } else {
        None
    };
    if let Some(translated) = bash {
        return match translated {
            Ok(translation) => Ok(Some(Habit {
                tree: translation.tree,
                rules: translation.rules,
                assumptions: translation.assumptions,
            })),
            Err(error) => Err(error),
        };
    }
    if powershell::detect(statement) {
        return match powershell::translate_with(statement, &context) {
            Some(Ok(translation)) => Ok(Some(Habit {
                tree: translation.tree,
                rules: translation
                    .rules
                    .into_iter()
                    .filter_map(|rule| Correction::try_from(rule).ok())
                    .collect(),
                assumptions: translation.assumptions,
            })),
            Some(Err(error)) => Err(error),
            None => Ok(None),
        };
    }
    Ok(None)
}

/// Moves the position of a correction made on a statement alone to the
/// statement that starts at `start`.
fn shift(mut correction: Correction, start: TextPosition) -> Correction {
    if let Some(at) = &mut correction.at {
        if at.line == 1 {
            at.column = at.column.saturating_add(start.column.saturating_sub(1));
        }
        at.line = at.line.saturating_add(start.line.saturating_sub(1));
    }
    correction
}

/// Byte ranges of the statements of a text, from the first to the last
/// significant character; a line break inside brackets, after a continuing
/// operator or before a line that starts with a pipe does not end a
/// statement.
fn statements(text: &str) -> Vec<Range<usize>> {
    let pieces: Vec<Piece> = scan(text)
        .into_iter()
        .filter(|piece| !piece.kind.is_trivia())
        .collect();
    let mut ranges = Vec::new();
    let mut depth = 0usize;
    let mut current: Option<Range<usize>> = None;
    for (index, piece) in pieces.iter().enumerate() {
        let slice = piece.text(text);
        if piece.kind == PieceKind::Newline {
            let after_operator = index
                .checked_sub(1)
                .and_then(|before| pieces.get(before))
                .is_some_and(|before| {
                    before.kind == PieceKind::Punct && CONTINUING.contains(&before.text(text))
                });
            let before_pipe = pieces
                .get(index + 1..)
                .unwrap_or_default()
                .iter()
                .find(|next| next.kind != PieceKind::Newline)
                .is_some_and(|next| matches!(next.text(text), "|>" | "|"));
            if depth == 0 && !after_operator && !before_pipe {
                ranges.extend(current.take());
            }
            continue;
        }
        if piece.kind == PieceKind::Punct {
            match slice {
                "(" | "[" | "{" => depth += 1,
                ")" | "]" | "}" => depth = depth.saturating_sub(1),
                _ => {}
            }
        }
        current = Some(match current {
            Some(range) => range.start..piece.range.end,
            None => piece.range.clone(),
        });
    }
    ranges.extend(current);
    ranges
}

/// Byte range of a span in the text it refers to.
fn span_range(text: &str, span: Option<Span>) -> Option<Range<usize>> {
    let span = span?;
    let start = offset_of(text, span.line, span.column);
    let end = offset_of(text, span.end_line, span.end_column).max(start);
    Some(start..end)
}

fn repair_kind(rule: Rule) -> CorrectionKind {
    match rule {
        Rule::L08 | Rule::L09 => CorrectionKind::Quote,
        Rule::L10 | Rule::L11 | Rule::L12 => CorrectionKind::Bracket,
        Rule::L13 | Rule::L14 => CorrectionKind::Pipe,
        Rule::L15 => CorrectionKind::Separator,
        _ => CorrectionKind::Syntax,
    }
}

fn variant_name(variant: Variant) -> &'static str {
    match variant {
        Variant::A => "a",
        Variant::B => "b",
        Variant::C => "c",
    }
}

fn to_at(position: TextPosition) -> At {
    At {
        line: Some(position.line),
        column: Some(position.column),
        length: position.length,
        stage: Some(Stage::Parse),
    }
}

#[cfg(test)]
mod tests {
    use nurt_contracts::report::CorrectionClass;
    use nurt_syntax::print::print;

    use super::*;
    use crate::lexical::ArgumentShape;
    use crate::tables::load_embedded;

    const COMMANDS: [&str; 11] = [
        "find", "search", "count", "verify", "run", "replace", "remove", "list", "first", "filter",
        "sort",
    ];

    struct Commands;

    impl CatalogView for Commands {
        fn is_command(&self, name: &str) -> bool {
            self.command_names().contains(&name)
        }
        fn command_names(&self) -> &[&str] {
            &COMMANDS
        }
        fn is_mutating(&self, name: &str) -> bool {
            matches!(name, "run" | "replace" | "remove")
        }
        fn can_take_argument(&self, command: &str, _: usize, _: ArgumentShape) -> bool {
            command == "filter"
        }
    }

    fn syntax(variant: Variant, tolerant: bool) -> SyntaxConfig {
        let mut config = SyntaxConfig::default();
        config.variant = variant;
        config.tolerant = tolerant;
        config
    }

    fn run_with(
        text: &str,
        syntax: &SyntaxConfig,
        correction: &CorrectionConfig,
    ) -> Result<Front, Error> {
        let tables = load_embedded().expect("embedded tables");
        front(text, syntax, correction, &tables, &Commands)
    }

    fn run(text: &str) -> Front {
        run_with(
            text,
            &syntax(Variant::A, true),
            &CorrectionConfig::default(),
        )
        .expect("corrected")
    }

    fn rules(result: &Front) -> Vec<String> {
        result
            .corrections
            .iter()
            .map(|correction| correction.rule.to_string())
            .collect()
    }

    fn canonical(result: &Front) -> String {
        print(&result.document, Variant::A).trim_end().to_owned()
    }

    #[test]
    fn translates_a_habit_that_the_strict_parser_reads_as_a_program() {
        let removed = run("rm -rf /srv/app/build");
        assert_eq!(rules(&removed), ["B01", "L21"]);
        assert_eq!(canonical(&removed), "remove \"/srv/app/build\"");
        let fetched = run("curl -s https://example.com/health");
        assert_eq!(rules(&fetched), ["B32"]);
        assert_eq!(
            canonical(&fetched),
            "http.request \"https://example.com/health\" follow_redirects: false"
        );
    }

    #[test]
    fn translates_a_habit_word_without_a_meaning_in_nurt_whatever_follows_it() {
        let created = run("touch /srv/app/a.txt");
        assert_eq!(rules(&created), ["B06", "L21"]);
        assert_eq!(
            canonical(&created),
            "create \"/srv/app/a.txt\" type: file on_exists: touch"
        );
    }

    #[test]
    fn translates_a_synonym_that_is_given_a_bare_word_it_cannot_take() {
        assert_eq!(canonical(&run("ps aux")), "processes");
        let read = run("cat \"/srv/app/a.txt\"");
        assert!(read.corrections.is_empty());
        assert_eq!(canonical(&read), "cat \"/srv/app/a.txt\"");
    }

    #[test]
    fn translates_an_assignment_before_any_program_with_the_base_in_the_fix() {
        let syntax = syntax(Variant::A, true);
        let correction = CorrectionConfig::default();
        let tables = load_embedded().expect("embedded tables");
        let error = front("CI=1 npm test", &syntax, &correction, &tables, &Commands)
            .expect_err("the working directory is not known");
        assert_eq!(error.code(), ErrorCode::Uncorrectable);
        assert!(error.data().fix.is_empty());
        let error = front_for_binding(
            "CI=1 npm test",
            &syntax,
            &correction,
            &tables,
            &Commands,
            Some("/srv/app"),
        )
        .expect_err("the working directory is only offered");
        assert_eq!(
            error.data().fix,
            ["run [\"npm\", \"test\"] cwd: \"/srv/app\" env: {\"CI\": \"1\"}"]
        );
    }

    #[test]
    fn keeps_a_nurt_program_that_uses_a_habit_word_as_a_command() {
        for text in [
            "find \"/srv/app\" |> sort size order: desc",
            "sort size order: desc",
            "remove \"/srv/app/build\"",
        ] {
            assert!(run(text).corrections.is_empty(), "{text}");
        }
    }

    #[test]
    fn gives_no_correction_to_a_valid_program() {
        let text = "find \"/srv/app\" glob: \"*.go\"\n|> filter size > 1mb\n|> first 20";
        let result = run(text);
        assert!(result.corrections.is_empty());
        assert_eq!(result.source, text);
        assert!(result.alternatives.is_empty());
    }

    #[test]
    fn applies_token_rules_and_numbers_corrections_by_position() {
        let result = run("Find /srv/app glob: '*.go' | count");
        assert_eq!(
            canonical(&result),
            "find \"/srv/app\" glob: \"*.go\"\n|> count"
        );
        assert_eq!(rules(&result), ["L03", "L21", "L07", "L13"]);
        let ids: Vec<&str> = result.corrections.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, ["C1", "C2", "C3", "C4"]);
    }

    #[test]
    fn closes_brackets_and_reports_the_repair_in_the_text_as_written() {
        let result = run("verify (run [\"go\", \"build\"] cwd: \"/a\"");
        assert_eq!(rules(&result), ["L10"]);
        assert_eq!(result.corrections[0].class, CorrectionClass::Structural);
        let at = result.corrections[0].at.expect("position");
        assert_eq!((at.line, at.column), (1, 38));
    }

    #[test]
    fn positions_of_repairs_skip_earlier_normalization() {
        let result = run("find\u{a0}\"/a\" |> count)");
        assert_eq!(rules(&result), ["L17", "L12"]);
        let at = result.corrections[1].at.expect("position");
        assert_eq!((at.line, at.column), (1, 19));
    }

    #[test]
    fn closes_an_unterminated_text_as_a_probable_correction() {
        let result = run("replace \"x\" \"newName(");
        assert_eq!(rules(&result), ["L08"]);
        assert_eq!(result.corrections[0].class, CorrectionClass::Probable);
    }

    #[test]
    fn writes_another_variant_in_the_configured_one() {
        let result = run("find(\"/srv\", glob: \"*.go\")");
        assert_eq!(rules(&result), ["L20"]);
        assert_eq!(canonical(&result), "find \"/srv\" glob: \"*.go\"");
        let called = run_with(
            "find \"/srv\" glob: \"*.go\"",
            &syntax(Variant::B, true),
            &CorrectionConfig::default(),
        )
        .expect("corrected");
        assert_eq!(rules(&called), ["L20"]);
    }

    #[test]
    fn writes_the_stages_of_a_mixed_statement_in_the_configured_variant() {
        let result = run("find(\"/srv\", glob: \"*.go\") |> first 5 |> count()");
        assert_eq!(rules(&result), ["L20", "L20"]);
        assert_eq!(
            canonical(&result),
            "find \"/srv\" glob: \"*.go\"\n|> first 5\n|> count"
        );
        let piped = run("find(\"/srv\") | count()");
        assert_eq!(rules(&piped), ["L13", "L20"]);
        let ordered: Vec<u32> = result
            .corrections
            .iter()
            .filter_map(|correction| correction.at.map(|at| at.column))
            .collect();
        assert_eq!(ordered, [1, 42]);
    }

    #[test]
    fn leaves_a_mixed_statement_whose_stages_do_not_read_alone() {
        let result = run_with(
            "find(\"/srv\") |> count(",
            &syntax(Variant::A, true),
            &CorrectionConfig::default(),
        );
        assert!(result.is_err());
        let strict = run_with(
            "find(\"/srv\") |> count()",
            &syntax(Variant::A, false),
            &CorrectionConfig::default(),
        );
        assert!(strict.is_err());
    }

    #[test]
    fn translates_a_habit_that_is_also_a_command_after_the_token_rules() {
        let result = run("sort -u /a/b\nfind /srv -name x");
        let rules = rules(&result);
        assert!(rules.contains(&"B37".to_owned()), "{rules:?}");
        assert!(canonical(&result).starts_with("lines \"/a/b\""));
        let called = run("sort size\nfind \"/srv\" |> count)");
        assert!(canonical(&called).starts_with("sort size"));
    }

    #[test]
    fn keeps_other_variants_when_the_syntax_is_not_tolerant() {
        let error = run_with(
            "find(\"/srv\", glob: \"*.go\")",
            &syntax(Variant::A, false),
            &CorrectionConfig::default(),
        );
        assert!(error.is_err());
    }

    #[test]
    fn parses_strictly_when_correction_is_disabled() {
        let mut correction = CorrectionConfig::default();
        correction.enabled = false;
        let error =
            run_with("Find \"/a\"", &syntax(Variant::A, true), &correction).expect_err("strict");
        assert_eq!(error.code, ErrorCode::Lexical);
        let valid = run_with("find \"/a\"", &syntax(Variant::A, true), &correction).expect("valid");
        assert!(valid.corrections.is_empty());
    }

    #[test]
    fn moves_positional_arguments_in_a_corrected_program() {
        let result = run("Find glob: \"*.go\" \"/srv/app\"");
        assert_eq!(rules(&result), ["L03", "L18"]);
        assert_eq!(canonical(&result), "find \"/srv/app\" glob: \"*.go\"");
    }

    #[test]
    fn returns_the_strict_error_when_nothing_reads() {
        let error = run_with(
            "find \"/a\" ; ; )(",
            &syntax(Variant::A, true),
            &CorrectionConfig::default(),
        )
        .expect_err("no reading");
        assert_eq!(error.code, ErrorCode::Uncorrectable);
    }

    #[test]
    fn running_front_on_its_result_makes_no_correction() {
        for text in [
            "Find /srv/app glob: '*.go' | count",
            "verify (run [\"go\", \"build\"] cwd: \"/a\"",
            "find(\"/srv\", glob: \"*.go\")",
        ] {
            let first = run(text);
            let again = run(&print(&first.document, Variant::A));
            assert!(again.corrections.is_empty(), "{text}");
        }
    }

    #[test]
    fn splits_statements_at_line_ends_outside_brackets() {
        let text = "find \"/a\"\n|> count\nlist [\n\"/b\"\n]\nfind \"/c\" |>\n  count";
        let ranges: Vec<&str> = statements(text)
            .into_iter()
            .map(|range| &text[range])
            .collect();
        assert_eq!(
            ranges,
            [
                "find \"/a\"\n|> count",
                "list [\n\"/b\"\n]",
                "find \"/c\" |>\n  count"
            ]
        );
    }

    /// Regular expressions of the top-level stages of a tree.
    fn patterns(document: &AstDocument) -> Vec<String> {
        document
            .program
            .statements
            .iter()
            .filter_map(|statement| match statement {
                Statement::Pipeline(pipeline) => Some(pipeline),
                _ => None,
            })
            .flat_map(|pipeline| {
                let head = match &pipeline.head {
                    Head::Stage(stage) => Some(stage),
                    _ => None,
                };
                head.into_iter().chain(pipeline.stages.iter())
            })
            .flat_map(|stage| stage.args.iter())
            .filter_map(|arg| match &arg.value {
                Expr::Regex { v, .. } => Some(v.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn keeps_the_bytes_of_a_regex_after_a_command_name() {
        for text in [
            "search/ oldName/ \"/srv/app\" -> count",
            "search/ Pending/i \"/srv\"",
            "find \"/a\" | > search /\u{a0}x/",
        ] {
            for variant in VARIANTS {
                let Ok(result) =
                    run_with(text, &syntax(variant, true), &CorrectionConfig::default())
                else {
                    continue;
                };
                for pattern in patterns(&result.document) {
                    assert!(
                        text.contains(&pattern),
                        "{text:?} in {variant:?}: {pattern:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn translates_a_bash_habit_and_keeps_its_rules() {
        let result = run("cd /srv/app && touch .ready");
        let rules = rules(&result);
        assert!(rules.iter().any(|rule| rule.starts_with('B')), "{rules:?}");
        assert!(canonical(&result).starts_with("create \"/srv/app/.ready\""));
    }

    #[test]
    fn returns_the_error_of_a_powershell_translation() {
        let error = run_with(
            "Get-ChildItem src -Recurse",
            &syntax(Variant::A, true),
            &CorrectionConfig::default(),
        );
        assert!(error.is_err());
    }

    #[test]
    fn statement_of_two_variant_readings_is_ambiguous_with_candidates() {
        let text = "x\nfind(\"/a\")";
        let work = Work::new(text, normalize(text));
        let readings = [
            (Variant::B, "find \"/a\"".to_owned()),
            (Variant::C, "find \"/b\"".to_owned()),
        ];
        let error = work.ambiguous_variant(&(2..text.len()), &readings, &work.locator());
        assert_eq!(error.code, ErrorCode::Ambiguous);
        let at = error.at.expect("position");
        assert_eq!((at.line, at.column), (Some(2), Some(1)));
        let programs: Vec<&str> = error
            .candidates
            .iter()
            .map(|candidate| candidate.program.as_str())
            .collect();
        assert_eq!(programs, ["x\nfind \"/a\"", "x\nfind \"/b\""]);
        let why: Vec<Option<&str>> = error
            .candidates
            .iter()
            .map(|candidate| candidate.why.as_deref())
            .collect();
        assert_eq!(why, [Some("read in variant b"), Some("read in variant c")]);
        assert_eq!(variant_name(Variant::A), "a");
    }

    #[test]
    fn splits_stages_outside_brackets_and_trims_white_space() {
        let text = "find \"/ä\" |>  count (a |> b) | first 1";
        let stages: Vec<&str> = stage_ranges(text, &(0..text.len()))
            .into_iter()
            .map(|range| &text[range])
            .collect();
        assert_eq!(stages, ["find \"/ä\"", "count (a |> b)", "first 1"]);
        assert!(stage_ranges("", &(0..0)).is_empty());
    }

    #[test]
    fn finds_the_byte_range_of_a_span_in_multibyte_text() {
        let text = "ä\nb ö";
        let span = Span {
            line: 2,
            column: 3,
            end_line: 2,
            end_column: 4,
        };
        assert_eq!(span_range(text, Some(span)), Some(5..7));
        assert_eq!(span_range(text, None), None);
    }

    #[test]
    fn shifts_positions_of_statement_corrections() {
        let correction = Rule::L13.record(
            TextPosition {
                line: 1,
                column: 4,
                length: Some(1),
            },
            "|",
            "|>",
            CorrectionKind::Pipe,
        );
        let start = TextPosition {
            line: 3,
            column: 2,
            length: None,
        };
        let at = shift(correction, start).at.expect("position");
        assert_eq!((at.line, at.column), (3, 5));
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
