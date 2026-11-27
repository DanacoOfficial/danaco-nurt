// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-verify::duplication
// Responsibility:   Duplicated code findings: token normalization, windows and fingerprints.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Duplicated code findings: token normalization, windows and fingerprints.
//!
//! Tokens come from the code service. Comments are dropped, identifiers and
//! literals are replaced by a class, so a copy with renamed variables still
//! matches. A fixed-size window of tokens slides over every file and its
//! polynomial fingerprint is kept in a table; consecutive matching windows of
//! the same pair of places form one block.

use std::collections::{HashMap, HashSet};

use nurt_contracts::error::Result;
use nurt_contracts::record::{Finding, FindingKind, FindingSeverity, RelatedLocation};
use nurt_contracts::services::{CodeService, LanguageId, SpanClass, SyntaxToken};
use nurt_contracts::value::Path as RecordPath;

/// Tool name of the findings.
pub(crate) const TOOL: &str = "builtin";

/// Rule of the findings.
pub(crate) const RULE: &str = "duplication";

/// Fewest lines of code in a reported block.
pub(crate) const DUPLICATION_MIN_LINES: usize = 8;

/// Tokens in a window.
const WINDOW_TOKENS: usize = 25;

/// Multiplier of the polynomial fingerprint; odd, so it is invertible modulo
/// a power of two.
const FINGERPRINT_BASE: u64 = 0x0000_0100_0000_01b3;

/// Offset basis of the hash of token text.
const TEXT_HASH_BASIS: u64 = 0xcbf2_9ce4_8422_2325;

/// Multiplier of the hash of token text.
const TEXT_HASH_PRIME: u64 = 0x0000_0100_0000_01b3;

/// Marks a hash of token text, so it differs from the class symbols.
const TEXT_HASH_FLAG: u64 = 1 << 63;

/// Symbol of an identifier.
const IDENTIFIER_SYMBOL: u64 = 1;

/// Symbol of a literal.
const LITERAL_SYMBOL: u64 = 2;

/// Words that start a line of declarations of dependencies; such lines are
/// not counted as code.
const DEPENDENCY_LINE_STARTS: &[&str] = &["import", "use", "package", "from"];

/// Token after normalization.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct NormalizedToken {
    /// Symbol: a class for identifiers and literals, a hash of the text for
    /// everything else.
    symbol: u64,
    /// Line of the first character, from 1.
    line: u32,
    /// Line of the last character.
    end_line: u32,
}

/// Normalized tokens of one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SourceTokens {
    /// File relative to the project directory.
    pub(crate) path: String,
    /// Tokens in source order.
    pub(crate) tokens: Vec<NormalizedToken>,
}

/// Result of the detection.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct DuplicationReport {
    /// One finding per block, located in the later copy.
    pub(crate) findings: Vec<Finding>,
    /// Lines of code in all files.
    pub(crate) code_lines: usize,
    /// Lines of code that belong to a block, in both copies.
    pub(crate) duplicated_lines: usize,
}

/// Scale of the share that keeps one decimal of a percentage.
const PERCENT_DECIMAL_SCALE: f64 = 1000.0;

/// Divisor that turns the scaled share into a percentage.
const PERCENT_DECIMAL_DIVISOR: f64 = 10.0;

impl DuplicationReport {
    /// Share of duplicated lines in the code, in percent with one decimal;
    /// `0.0` when the files hold no code.
    pub(crate) fn percent(&self) -> f64 {
        if self.code_lines == 0 {
            return 0.0;
        }
        let share = self.duplicated_lines as f64 / self.code_lines as f64;
        (share * PERCENT_DECIMAL_SCALE).round() / PERCENT_DECIMAL_DIVISOR
    }
}

/// Tokens of one file read from the code service and normalized.
///
/// # Errors
///
/// Fails with the error of the code service.
pub(crate) fn tokenize_file(
    code: &dyn CodeService,
    path: &str,
    bytes: &[u8],
    language: &LanguageId,
) -> Result<SourceTokens> {
    let tokens = code.tokens(bytes, language)?;
    Ok(normalize(path, bytes, &tokens))
}

/// Normalizes the tokens of a file: comments are dropped, identifiers and
/// literals become classes, lines of dependency declarations are skipped.
pub(crate) fn normalize(path: &str, bytes: &[u8], tokens: &[SyntaxToken]) -> SourceTokens {
    let line_starts = line_starts(bytes);
    let line_of = |offset: u64| -> u32 {
        let offset = usize::try_from(offset).unwrap_or(usize::MAX);
        let index = line_starts.partition_point(|start| *start <= offset);
        u32::try_from(index).unwrap_or(u32::MAX)
    };
    let mut normalized: Vec<NormalizedToken> = Vec::new();
    let mut skipped_line = 0;
    let mut last_line = 0;
    for token in tokens
        .iter()
        .filter(|token| token.class != SpanClass::Comment)
    {
        let line = line_of(token.bytes.start);
        if line == skipped_line {
            continue;
        }
        let text = slice(bytes, token.bytes.start, token.bytes.end);
        if line != last_line && starts_dependency_line(text) {
            skipped_line = line;
            continue;
        }
        last_line = line;
        let end_line = line_of(token.bytes.end.saturating_sub(1).max(token.bytes.start));
        normalized.push(NormalizedToken {
            symbol: symbol_of(token, text),
            line,
            end_line,
        });
    }
    SourceTokens {
        path: path.to_owned(),
        tokens: normalized,
    }
}

fn line_starts(bytes: &[u8]) -> Vec<usize> {
    let mut starts = vec![0];
    starts.extend(
        bytes
            .iter()
            .enumerate()
            .filter(|(_, byte)| **byte == b'\n')
            .map(|(index, _)| index + 1),
    );
    starts
}

fn slice(bytes: &[u8], start: u64, end: u64) -> &[u8] {
    let start = usize::try_from(start)
        .unwrap_or(usize::MAX)
        .min(bytes.len());
    let end = usize::try_from(end).unwrap_or(usize::MAX).min(bytes.len());
    &bytes[start..end.max(start)]
}

fn starts_dependency_line(text: &[u8]) -> bool {
    DEPENDENCY_LINE_STARTS
        .iter()
        .any(|word| word.as_bytes() == text)
}

fn symbol_of(token: &SyntaxToken, text: &[u8]) -> u64 {
    if token.class == SpanClass::String || is_literal_kind(&token.kind) {
        LITERAL_SYMBOL
    } else if is_identifier_kind(&token.kind) {
        IDENTIFIER_SYMBOL
    } else {
        text_hash(text)
    }
}

fn is_identifier_kind(kind: &str) -> bool {
    kind == "name" || kind.ends_with("identifier")
}

fn is_literal_kind(kind: &str) -> bool {
    kind.contains("literal")
        || kind.contains("string")
        || matches!(kind, "number" | "integer" | "float" | "decimal" | "char")
}

fn text_hash(text: &[u8]) -> u64 {
    let hash = text.iter().fold(TEXT_HASH_BASIS, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(TEXT_HASH_PRIME)
    });
    hash | TEXT_HASH_FLAG
}

/// Fingerprints of every window of `window` tokens, by start index.
fn window_fingerprints(tokens: &[NormalizedToken], window: usize) -> Vec<u64> {
    if window == 0 || tokens.len() < window {
        return Vec::new();
    }
    let leading = (1..window).fold(1u64, |power, _| power.wrapping_mul(FINGERPRINT_BASE));
    let mut current = tokens[..window].iter().fold(0u64, |hash, token| {
        hash.wrapping_mul(FINGERPRINT_BASE)
            .wrapping_add(token.symbol)
    });
    let mut all = vec![current];
    for index in window..tokens.len() {
        current = current
            .wrapping_sub(tokens[index - window].symbol.wrapping_mul(leading))
            .wrapping_mul(FINGERPRINT_BASE)
            .wrapping_add(tokens[index].symbol);
        all.push(current);
    }
    all
}

/// Place of a window: file index and start token index.
type Place = (usize, usize);

/// Consecutive matching windows of one file against one earlier place.
#[derive(Debug, Clone, Copy)]
struct Run {
    file: usize,
    first: usize,
    last: usize,
    partner_file: usize,
    partner_first: usize,
    partner_last: usize,
}

struct Index<'a> {
    files: &'a [SourceTokens],
    window: usize,
    places: HashMap<u64, Vec<Place>>,
    hashes: Vec<Vec<u64>>,
}

impl<'a> Index<'a> {
    fn build(files: &'a [SourceTokens], window: usize) -> Self {
        let hashes: Vec<Vec<u64>> = files
            .iter()
            .map(|file| window_fingerprints(&file.tokens, window))
            .collect();
        let mut places: HashMap<u64, Vec<Place>> = HashMap::new();
        for (file, own) in hashes.iter().enumerate() {
            for (index, hash) in own.iter().enumerate() {
                places.entry(*hash).or_default().push((file, index));
            }
        }
        Self {
            files,
            window,
            places,
            hashes,
        }
    }

    fn symbols(&self, (file, index): Place) -> impl Iterator<Item = u64> + '_ {
        self.files[file].tokens[index..index + self.window]
            .iter()
            .map(|token| token.symbol)
    }

    /// Earliest place before `(file, index)` with the same tokens that does
    /// not overlap the window.
    fn earlier_partner(&self, file: usize, index: usize) -> Option<Place> {
        let here = (file, index);
        let candidates = self.places.get(&self.hashes[file][index])?;
        candidates
            .iter()
            .copied()
            .take_while(|place| *place < here)
            .find(|place| {
                let overlaps = place.0 == file && place.1 + self.window > index;
                !overlaps && self.symbols(*place).eq(self.symbols(here))
            })
    }

    /// Place that continues `run` at `(file, index)`: the window after the
    /// last partner, when it holds the same tokens and does not overlap.
    fn continuation(&self, run: &Run, index: usize) -> Option<Place> {
        let here = (run.file, index);
        let next = (run.partner_file, run.partner_last + 1);
        let exists = self.hashes.get(next.0)?.get(next.1).is_some();
        let overlaps = next.0 == run.file && next.1 + self.window > index;
        (exists && next < here && !overlaps && self.symbols(next).eq(self.symbols(here)))
            .then_some(next)
    }

    fn runs_of(&self, file: usize) -> Vec<Run> {
        let mut runs = Vec::new();
        let mut current: Option<Run> = None;
        for index in 0..self.hashes[file].len() {
            let continued = current
                .as_ref()
                .and_then(|run| self.continuation(run, index));
            if let (Some(run), Some(place)) = (current.as_mut(), continued) {
                run.last = index;
                run.partner_last = place.1;
                continue;
            }
            let partner = self.earlier_partner(file, index);
            runs.extend(current.take());
            current = partner.map(|(partner_file, partner_first)| Run {
                file,
                first: index,
                last: index,
                partner_file,
                partner_first,
                partner_last: partner_first,
            });
        }
        runs.extend(current);
        runs
    }
}

/// Lines with code in a range of tokens, as `(first, last, count)`.
fn line_span(tokens: &[NormalizedToken]) -> (u32, u32, usize) {
    let first = tokens.first().map_or(0, |token| token.line);
    let last = tokens.last().map_or(0, |token| token.end_line);
    let mut count = 0;
    let mut previous = 0;
    for token in tokens {
        if token.line != previous {
            count += 1;
            previous = token.line;
        }
    }
    (first, last, count)
}

fn lines_of(tokens: &[NormalizedToken]) -> impl Iterator<Item = u32> + '_ {
    tokens.iter().map(|token| token.line)
}

/// Finds duplicated blocks of at least `min_lines` lines of code.
///
/// The files are processed in the order of their paths, so the result does
/// not depend on the order of `files`. The later copy is reported, the
/// earlier one is its original. A line of code counts once towards the
/// duplicated lines, however many blocks cover it.
pub(crate) fn detect(files: &[SourceTokens], min_lines: usize) -> DuplicationReport {
    let mut ordered: Vec<&SourceTokens> = files.iter().collect();
    ordered.sort_by(|left, right| left.path.cmp(&right.path));
    let ordered: Vec<SourceTokens> = ordered.into_iter().cloned().collect();
    let index = Index::build(&ordered, WINDOW_TOKENS);
    let mut report = DuplicationReport {
        code_lines: ordered.iter().map(|file| line_span(&file.tokens).2).sum(),
        ..DuplicationReport::default()
    };
    let mut marked: HashSet<(usize, u32)> = HashSet::new();
    for file in 0..ordered.len() {
        for run in index.runs_of(file) {
            if let Some(finding) = report_run(&ordered, &run, min_lines, &mut marked) {
                report.findings.push(finding);
            }
        }
    }
    report.duplicated_lines = marked.len();
    report
}

fn report_run(
    files: &[SourceTokens],
    run: &Run,
    min_lines: usize,
    marked: &mut HashSet<(usize, u32)>,
) -> Option<Finding> {
    let own = &files[run.file].tokens[run.first..run.last + WINDOW_TOKENS];
    let original =
        &files[run.partner_file].tokens[run.partner_first..run.partner_last + WINDOW_TOKENS];
    let (first, last, count) = line_span(own);
    if count < min_lines {
        return None;
    }
    let (original_first, original_last, _) = line_span(original);
    marked.extend(lines_of(own).map(|line| (run.file, line)));
    marked.extend(lines_of(original).map(|line| (run.partner_file, line)));
    Some(Finding {
        path: RecordPath::new(files[run.file].path.clone()),
        kind: FindingKind::Duplication,
        severity: FindingSeverity::Warning,
        message: format!("duplicated block of {count} lines of code"),
        line: Some(u64::from(first)),
        column: Some(1),
        end_line: Some(u64::from(last)),
        tool: Some(TOOL.to_owned()),
        rule: Some(RULE.to_owned()),
        value: Some(count as f64),
        related: Some(vec![RelatedLocation {
            path: RecordPath::new(files[run.partner_file].path.clone()),
            line: u64::from(original_first),
            end_line: Some(u64::from(original_last)),
            message: Some("original".to_owned()),
        }]),
        ..Finding::default()
    })
}

#[cfg(test)]
mod tests;

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
