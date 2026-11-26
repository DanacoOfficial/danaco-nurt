// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Edit distance between names and the acceptance rules of fuzzy name
//! correction.
//!
//! Names are compared after [`fold`]: case is ignored and the `kebab-case`,
//! `camelCase` and `PascalCase` conventions are rewritten to `snake_case`.
//! The distance is the optimal string alignment variant of the
//! Damerau–Levenshtein distance ([`osa`]) counted in Unicode scalar values.
//!
//! A fuzzy candidate `v*` for a written name `s` is accepted only when all of
//! the following hold:
//!
//! 1. `osa(s, v*) <= threshold(len(v*))`;
//! 2. every other entry of the comparison set of the same kind, including
//!    synonyms and habit words, is strictly farther from `s` than `v*`;
//! 3. `s` and `v*` start with the same character, unless
//!    [`Options::require_first_char`] is `false`;
//! 4. a command that changes the world or runs a program is accepted only at
//!    distance 1, and the caller confirms that the whole corrected program
//!    binds and type-checks ([`Nearest::Unique::effecting`]).

use std::collections::HashMap;

use nurt_contracts::config::CorrectionConfig;

/// Longest name, in characters, that is corrected only through the synonym
/// table and never by edit distance.
pub const EXACT_ONLY_MAX_LEN: usize = 3;

/// Longest name, in characters, that accepts a single edit; longer names
/// accept two edits.
pub const ONE_EDIT_MAX_LEN: usize = 7;

/// Largest distance accepted for a name longer than [`ONE_EDIT_MAX_LEN`].
pub const MAX_DISTANCE: usize = 2;

/// Largest distance accepted for a reference to a `let` name, at any length.
pub const LET_MAX_DISTANCE: usize = 1;

/// Largest distance accepted for a command that changes the world or runs a
/// program.
pub const EFFECTING_MAX_DISTANCE: usize = 1;

/// Shortest prefix, in characters, that selects a parameter or an enumeration
/// value.
pub const MIN_PREFIX_LEN: usize = 3;

/// Number of names listed in `expected` of an `E_UNKNOWN_*` error.
pub const EXPECTED_LIMIT: usize = 8;

/// Folds a name for comparison: lower case and `snake_case`.
///
/// `-` becomes `_`; an upper-case letter that starts a word of a `camelCase`
/// or `PascalCase` name is preceded by `_`. Dots that separate a group from a
/// command name are kept.
///
/// # Examples
///
/// ```
/// use nurt_correct::distance::fold;
///
/// assert_eq!(fold("Find"), "find");
/// assert_eq!(fold("case-sensitive"), "case_sensitive");
/// assert_eq!(fold("caseSensitive"), "case_sensitive");
/// assert_eq!(fold("LastWriteTime"), "last_write_time");
/// assert_eq!(fold("git.Log"), "git.log");
/// ```
#[must_use]
pub fn fold(name: &str) -> String {
    let chars: Vec<char> = name.chars().collect();
    let mut out = String::with_capacity(name.len() + 4);
    for (i, &c) in chars.iter().enumerate() {
        if c == '-' {
            out.push('_');
            continue;
        }
        if c.is_uppercase() {
            let prev = i.checked_sub(1).and_then(|j| chars.get(j)).copied();
            let next = chars.get(i + 1).copied();
            if starts_word(prev, next) && !out.ends_with('_') {
                out.push('_');
            }
            out.extend(c.to_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

fn starts_word(prev: Option<char>, next: Option<char>) -> bool {
    match prev {
        Some(p) if p.is_lowercase() || p.is_numeric() => true,
        Some(p) if p.is_uppercase() => next.is_some_and(char::is_lowercase),
        _ => false,
    }
}

/// Returns the optimal string alignment distance between two names after
/// [`fold`].
///
/// The edits are insertion, deletion, substitution and transposition of two
/// adjacent characters; no substring is edited twice.
///
/// # Examples
///
/// ```
/// use nurt_correct::distance::osa;
///
/// assert_eq!(osa("remvoe", "remove"), 1);
/// assert_eq!(osa("serach", "search"), 1);
/// assert_eq!(osa("Find", "find"), 0);
/// assert_eq!(osa("list", "last"), 1);
/// assert_eq!(osa("ca", "abc"), 3);
/// ```
#[must_use]
pub fn osa(a: &str, b: &str) -> usize {
    let a: Vec<char> = fold(a).chars().collect();
    let b: Vec<char> = fold(b).chars().collect();
    osa_chars(&a, &b, usize::MAX).unwrap_or(usize::MAX)
}

/// Distance between folded character sequences, or `None` when it exceeds
/// `limit`.
fn osa_chars(a: &[char], b: &[char], limit: usize) -> Option<usize> {
    if a.len().abs_diff(b.len()) > limit {
        return None;
    }
    let width = b.len() + 1;
    let mut before: Vec<usize> = vec![0; width];
    let mut previous: Vec<usize> = (0..width).collect();
    let mut current: Vec<usize> = vec![0; width];
    for (i, &ca) in a.iter().enumerate() {
        current[0] = i + 1;
        let mut row_min = current[0];
        for (j, &cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            let mut cell = (previous[j + 1] + 1)
                .min(current[j] + 1)
                .min(previous[j] + cost);
            if i > 0 && j > 0 && ca == b[j - 1] && a[i - 1] == cb {
                cell = cell.min(before[j - 1] + 1);
            }
            current[j + 1] = cell;
            row_min = row_min.min(cell);
        }
        if row_min > limit {
            return None;
        }
        std::mem::swap(&mut before, &mut previous);
        std::mem::swap(&mut previous, &mut current);
    }
    let distance = previous[b.len()];
    (distance <= limit).then_some(distance)
}

/// Largest edit distance accepted for a candidate name of `len` characters.
///
/// | Length | Distance |
/// |---|---|
/// | up to 3 | 0 (synonym table only) |
/// | 4 to 7 | 1 |
/// | 8 and more | 2 |
///
/// # Examples
///
/// ```
/// use nurt_correct::distance::threshold;
///
/// assert_eq!(threshold(3), 0);
/// assert_eq!(threshold(6), 1);
/// assert_eq!(threshold(14), 2);
/// ```
#[must_use]
pub const fn threshold(len: usize) -> usize {
    if len <= EXACT_ONLY_MAX_LEN {
        0
    } else if len <= ONE_EDIT_MAX_LEN {
        1
    } else {
        MAX_DISTANCE
    }
}

/// Kind of a name; names are compared only with names of the same kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum NameKind {
    /// Command name, including the group prefix (`git.log`).
    Command,
    /// Command group (`git`, `verify`).
    Group,
    /// Parameter of one command.
    Param,
    /// Enumeration value of one parameter.
    Value,
    /// Field of the input record.
    Field,
    /// Expression function.
    Function,
    /// Name introduced by `let`.
    Let,
}

impl NameKind {
    /// Every kind, in declaration order.
    pub const ALL: [Self; 7] = [
        Self::Command,
        Self::Group,
        Self::Param,
        Self::Value,
        Self::Field,
        Self::Function,
        Self::Let,
    ];

    /// Returns the name of the kind as written in correction tables.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Command => "command",
            Self::Group => "group",
            Self::Param => "param",
            Self::Value => "value",
            Self::Field => "field",
            Self::Function => "function",
            Self::Let => "let",
        }
    }

    /// Parses the name of a kind as written in correction tables.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.as_str() == text)
    }

    const fn index(self) -> usize {
        self as usize
    }

    fn accepts_prefix(self) -> bool {
        matches!(self, Self::Param | Self::Value)
    }

    fn max_distance(self) -> usize {
        if self == Self::Let {
            LET_MAX_DISTANCE
        } else {
            MAX_DISTANCE
        }
    }

    fn accepts(self, distance: usize, len: usize) -> bool {
        if self == Self::Let {
            distance <= LET_MAX_DISTANCE
        } else {
            distance <= threshold(len)
        }
    }
}

/// Role of an entry in the comparison set.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum NameRole {
    /// A name of the language or the catalog.
    Name,
    /// A synonym that stands for the name `to` of the same kind.
    Synonym {
        /// Canonical name.
        to: String,
    },
    /// A word of another shell (`cd`, `grep`, `sed`); it competes with
    /// command names but is never a correction target.
    Habit,
}

/// Entry of the comparison set.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NameEntry {
    /// Name as written in the catalog or the table.
    pub text: String,
    /// Kind of the name.
    pub kind: NameKind,
    /// Role of the entry.
    pub role: NameRole,
    /// Whether the entry is a command that changes the world or runs a
    /// program.
    pub effecting: bool,
}

impl NameEntry {
    /// A name of the given kind.
    #[must_use]
    pub fn name(text: impl Into<String>, kind: NameKind) -> Self {
        Self {
            text: text.into(),
            kind,
            role: NameRole::Name,
            effecting: false,
        }
    }

    /// A command name; `effecting` marks a command that changes the world or
    /// runs a program.
    #[must_use]
    pub fn command(text: impl Into<String>, effecting: bool) -> Self {
        Self {
            text: text.into(),
            kind: NameKind::Command,
            role: NameRole::Name,
            effecting,
        }
    }

    /// A synonym of the name `to` of the given kind.
    #[must_use]
    pub fn synonym(text: impl Into<String>, kind: NameKind, to: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            kind,
            role: NameRole::Synonym { to: to.into() },
            effecting: false,
        }
    }

    /// A habit word of another shell; it competes with command names.
    #[must_use]
    pub fn habit(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            kind: NameKind::Command,
            role: NameRole::Habit,
            effecting: false,
        }
    }
}

/// Options of the fuzzy name match.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Options {
    /// Key `correction.require_first_char`: a fuzzy candidate starts with the
    /// same character as the written name.
    pub require_first_char: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            require_first_char: true,
        }
    }
}

impl From<&CorrectionConfig> for Options {
    fn from(config: &CorrectionConfig) -> Self {
        Self {
            require_first_char: config.require_first_char,
        }
    }
}

/// Result of matching a written name against the comparison set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Nearest {
    /// The name exists; `to` differs from the written text only in case or
    /// naming convention, or not at all.
    Exact {
        /// Canonical name.
        to: String,
    },
    /// The name is a synonym of `to`.
    Synonym {
        /// Canonical name.
        to: String,
    },
    /// A single name passes every acceptance rule.
    Unique {
        /// Canonical name.
        to: String,
        /// Edit distance after folding.
        distance: usize,
        /// The name is a command that changes the world or runs a program;
        /// the caller applies the correction only when the whole corrected
        /// program binds and type-checks.
        effecting: bool,
    },
    /// The name is an unambiguous prefix of a parameter or a value.
    Prefix {
        /// Canonical name.
        to: String,
    },
    /// Two or more entries are equally close; nothing is corrected.
    Tie {
        /// Competing entries, sorted.
        names: Vec<String>,
    },
    /// No entry is close enough.
    None,
}

#[derive(Debug, Clone)]
struct Indexed {
    entry: NameEntry,
    folded: Vec<char>,
}

impl Indexed {
    fn target(&self) -> &str {
        match &self.entry.role {
            NameRole::Synonym { to } => to,
            NameRole::Name | NameRole::Habit => &self.entry.text,
        }
    }
}

/// Comparison set indexed by kind, folded text and length.
///
/// # Examples
///
/// ```
/// use nurt_correct::distance::{NameEntry, NameIndex, NameKind, Nearest, Options, nearest};
///
/// let index = NameIndex::new([
///     NameEntry::command("remove", true),
///     NameEntry::command("move", true),
///     NameEntry::synonym("delete", NameKind::Command, "remove"),
/// ]);
/// let found = nearest("remvoe", &index, NameKind::Command, &Options::default());
/// assert_eq!(
///     found,
///     Nearest::Unique { to: "remove".into(), distance: 1, effecting: true }
/// );
/// ```
#[derive(Debug, Clone, Default)]
pub struct NameIndex {
    entries: Vec<Indexed>,
    exact: HashMap<(NameKind, String), Vec<usize>>,
    by_len: [Vec<Vec<usize>>; NameKind::ALL.len()],
}

impl NameIndex {
    /// Builds the index from the entries of the comparison set.
    #[must_use]
    pub fn new(entries: impl IntoIterator<Item = NameEntry>) -> Self {
        let mut index = Self::default();
        for entry in entries {
            index.insert(entry);
        }
        index
    }

    /// Adds an entry to the comparison set.
    pub fn insert(&mut self, entry: NameEntry) {
        let folded_text = fold(&entry.text);
        let folded: Vec<char> = folded_text.chars().collect();
        let id = self.entries.len();
        let lengths = &mut self.by_len[entry.kind.index()];
        if lengths.len() <= folded.len() {
            lengths.resize_with(folded.len() + 1, Vec::new);
        }
        lengths[folded.len()].push(id);
        self.exact
            .entry((entry.kind, folded_text))
            .or_default()
            .push(id);
        self.entries.push(Indexed { entry, folded });
    }

    /// Number of entries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the index has no entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Entries of the comparison set, in insertion order.
    pub fn entries(&self) -> impl Iterator<Item = &NameEntry> {
        self.entries.iter().map(|indexed| &indexed.entry)
    }

    fn of_kind(&self, kind: NameKind) -> impl Iterator<Item = &Indexed> {
        self.by_len[kind.index()]
            .iter()
            .flatten()
            .filter_map(|&id| self.entries.get(id))
    }

    fn within_lengths(
        &self,
        kind: NameKind,
        len: usize,
        spread: usize,
    ) -> impl Iterator<Item = &Indexed> {
        let lengths = &self.by_len[kind.index()];
        let low = len.saturating_sub(spread);
        let high = (len + spread).min(lengths.len().saturating_sub(1));
        lengths
            .get(low..=high)
            .unwrap_or_default()
            .iter()
            .flatten()
            .filter_map(|&id| self.entries.get(id))
    }
}

/// Matches the written name `s` against the names of `kind` in `index`.
///
/// The match is tried in order: exact name after [`fold`], exact synonym,
/// fuzzy name under the acceptance rules of this module, unambiguous prefix
/// of a parameter or a value. A fuzzy match never ends on a synonym or a
/// habit word: these only compete with names.
///
/// # Examples
///
/// ```
/// use nurt_correct::distance::{NameEntry, NameIndex, NameKind, Nearest, Options, nearest};
///
/// let index = NameIndex::new([
///     NameEntry::command("list", false),
///     NameEntry::command("last", false),
///     NameEntry::command("find", false),
/// ]);
/// let options = Options::default();
/// assert_eq!(
///     nearest("fnid", &index, NameKind::Command, &options),
///     Nearest::Unique { to: "find".into(), distance: 1, effecting: false }
/// );
/// assert_eq!(
///     nearest("lest", &index, NameKind::Command, &options),
///     Nearest::Tie { names: vec!["last".into(), "list".into()] }
/// );
/// assert_eq!(nearest("Find", &index, NameKind::Command, &options), Nearest::Exact { to: "find".into() });
/// ```
#[must_use]
pub fn nearest(s: &str, index: &NameIndex, kind: NameKind, options: &Options) -> Nearest {
    let folded_text = fold(s);
    if let Some(found) = exact_match(&folded_text, index, kind) {
        return found;
    }
    let folded: Vec<char> = folded_text.chars().collect();
    let fuzzy = fuzzy_match(&folded, index, kind, options);
    if !kind.accepts_prefix() || matches!(fuzzy, Nearest::Tie { .. }) {
        return fuzzy;
    }
    match (fuzzy, prefix_match(&folded, index, kind)) {
        (found, None) => found,
        (Nearest::None, Some(to)) => Nearest::Prefix { to },
        (
            Nearest::Unique {
                to,
                distance,
                effecting,
            },
            Some(prefix),
        ) if prefix == to => Nearest::Unique {
            to,
            distance,
            effecting,
        },
        (Nearest::Unique { to, .. }, Some(prefix)) => Nearest::Tie {
            names: sorted_unique(vec![to, prefix]),
        },
        (found, Some(_)) => found,
    }
}

fn exact_match(folded: &str, index: &NameIndex, kind: NameKind) -> Option<Nearest> {
    let ids = index.exact.get(&(kind, folded.to_owned()))?;
    let hits: Vec<&Indexed> = ids.iter().filter_map(|&id| index.entries.get(id)).collect();
    let names = distinct(
        hits.iter()
            .filter(|hit| hit.entry.role == NameRole::Name)
            .map(|hit| hit.entry.text.clone()),
    );
    if !names.is_empty() {
        return Some(one_or_tie(names, |to| Nearest::Exact { to }));
    }
    let targets = distinct(
        hits.iter()
            .filter(|hit| matches!(hit.entry.role, NameRole::Synonym { .. }))
            .map(|hit| hit.target().to_owned()),
    );
    if !targets.is_empty() {
        return Some(one_or_tie(targets, |to| Nearest::Synonym { to }));
    }
    Some(Nearest::None)
}

fn fuzzy_match(folded: &[char], index: &NameIndex, kind: NameKind, options: &Options) -> Nearest {
    let limit = kind.max_distance();
    let mut best = usize::MAX;
    let mut winners: Vec<&Indexed> = Vec::new();
    for candidate in index.within_lengths(kind, folded.len(), limit) {
        let Some(distance) = osa_chars(folded, &candidate.folded, limit.min(best)) else {
            continue;
        };
        if distance < best {
            best = distance;
            winners.clear();
        }
        if distance == best {
            winners.push(candidate);
        }
    }
    let Some(name) = winners
        .iter()
        .find(|winner| winner.entry.role == NameRole::Name)
    else {
        return Nearest::None;
    };
    let texts = distinct(winners.iter().map(|winner| winner.entry.text.clone()));
    if texts.len() > 1 {
        return Nearest::Tie { names: texts };
    }
    accept(folded, name, best, kind, options)
}

fn accept(
    folded: &[char],
    winner: &Indexed,
    distance: usize,
    kind: NameKind,
    options: &Options,
) -> Nearest {
    if winner.entry.role != NameRole::Name || !kind.accepts(distance, winner.folded.len()) {
        return Nearest::None;
    }
    if options.require_first_char && folded.first() != winner.folded.first() {
        return Nearest::None;
    }
    if winner.entry.effecting && distance > EFFECTING_MAX_DISTANCE {
        return Nearest::None;
    }
    Nearest::Unique {
        to: winner.entry.text.clone(),
        distance,
        effecting: winner.entry.effecting,
    }
}

fn prefix_match(folded: &[char], index: &NameIndex, kind: NameKind) -> Option<String> {
    if folded.len() < MIN_PREFIX_LEN {
        return None;
    }
    let targets = distinct(
        index
            .of_kind(kind)
            .filter(|candidate| candidate.entry.role != NameRole::Habit)
            .filter(|candidate| candidate.folded.starts_with(folded))
            .map(|candidate| candidate.target().to_owned()),
    );
    match targets.as_slice() {
        [to] => Some(to.clone()),
        _ => None,
    }
}

fn distinct(items: impl Iterator<Item = String>) -> Vec<String> {
    sorted_unique(items.collect())
}

fn sorted_unique(mut items: Vec<String>) -> Vec<String> {
    items.sort();
    items.dedup();
    items
}

fn one_or_tie(mut names: Vec<String>, one: impl FnOnce(String) -> Nearest) -> Nearest {
    if names.len() == 1 {
        names.pop().map_or(Nearest::None, one)
    } else {
        Nearest::Tie { names }
    }
}

/// Returns up to `limit` names of `kind` nearest to `s`, for the `expected`
/// member of an `E_UNKNOWN_*` error.
///
/// Only names are listed, never synonyms or habit words. Names are ordered by
/// distance and then by text.
///
/// # Examples
///
/// ```
/// use nurt_correct::distance::{NameEntry, NameIndex, NameKind, expected};
///
/// let index = NameIndex::new(
///     ["depth", "glob", "type", "hidden"].map(|name| NameEntry::name(name, NameKind::Param)),
/// );
/// assert_eq!(expected("dept", &index, NameKind::Param, 2), ["depth", "type"]);
/// ```
#[must_use]
pub fn expected(s: &str, index: &NameIndex, kind: NameKind, limit: usize) -> Vec<String> {
    let folded: Vec<char> = fold(s).chars().collect();
    let mut ranked: Vec<(usize, &str)> = index
        .of_kind(kind)
        .filter(|candidate| candidate.entry.role == NameRole::Name)
        .map(|candidate| {
            let distance = osa_chars(&folded, &candidate.folded, usize::MAX).unwrap_or(usize::MAX);
            (distance, candidate.entry.text.as_str())
        })
        .collect();
    ranked.sort_unstable();
    ranked.dedup_by(|a, b| a.1 == b.1);
    ranked
        .into_iter()
        .take(limit)
        .map(|(_, name)| name.to_owned())
        .collect()
}

/// Pair of entries of the same kind that lie close to each other.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NamePair {
    /// Kind shared by both entries.
    pub kind: NameKind,
    /// First entry, the lesser of the two texts.
    pub a: String,
    /// Second entry.
    pub b: String,
    /// Edit distance after folding.
    pub distance: usize,
}

/// Lists every pair of entries of the same kind within `max_distance` of each
/// other, where at least one entry of the pair is a name.
///
/// Pairs are sorted by kind, distance and text.
#[must_use]
pub fn close_pairs(entries: &[NameEntry], max_distance: usize) -> Vec<NamePair> {
    let folded: Vec<Vec<char>> = entries
        .iter()
        .map(|entry| fold(&entry.text).chars().collect())
        .collect();
    let mut pairs = Vec::new();
    for (i, (first, first_folded)) in entries.iter().zip(&folded).enumerate() {
        for (second, second_folded) in entries.iter().zip(&folded).skip(i + 1) {
            if first.kind != second.kind
                || (first.role != NameRole::Name && second.role != NameRole::Name)
            {
                continue;
            }
            if let Some(distance) = osa_chars(first_folded, second_folded, max_distance) {
                let (a, b) = ordered(&first.text, &second.text);
                pairs.push(NamePair {
                    kind: first.kind,
                    a,
                    b,
                    distance,
                });
            }
        }
    }
    pairs.sort_by(|x, y| (x.kind, x.distance, &x.a, &x.b).cmp(&(y.kind, y.distance, &y.a, &y.b)));
    pairs.dedup();
    pairs
}

fn ordered(first: &str, second: &str) -> (String, String) {
    if first <= second {
        (first.to_owned(), second.to_owned())
    } else {
        (second.to_owned(), first.to_owned())
    }
}

/// Checks a set of names for conflicts with fuzzy correction.
///
/// A conflict is a pair of entries of the same kind at distance 0 or 1, where
/// at least one entry is a name: two identical names, a synonym equal or
/// close to a name, a habit word equal or close to a command, or two close
/// names. Pairs listed in `exceptions` (in either order, compared after
/// [`fold`]) are documented exceptions and are not reported. A string at
/// distance 1 from both names of an exception is a tie and is never
/// corrected.
///
/// # Examples
///
/// ```
/// use nurt_correct::distance::{NameEntry, check_names};
///
/// let names = [
///     NameEntry::command("list", false),
///     NameEntry::command("last", false),
///     NameEntry::command("find", false),
/// ];
/// assert_eq!(check_names(&names, &[]).len(), 1);
/// assert!(check_names(&names, &[("list", "last")]).is_empty());
/// ```
#[must_use]
pub fn check_names(entries: &[NameEntry], exceptions: &[(&str, &str)]) -> Vec<NamePair> {
    let allowed: Vec<(String, String)> = exceptions
        .iter()
        .map(|(a, b)| ordered(&fold(a), &fold(b)))
        .collect();
    close_pairs(entries, EFFECTING_MAX_DISTANCE)
        .into_iter()
        .filter(|pair| !allowed.contains(&ordered(&fold(&pair.a), &fold(&pair.b))))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn commands() -> NameIndex {
        NameIndex::new([
            NameEntry::command("remove", true),
            NameEntry::command("move", true),
            NameEntry::command("find", false),
            NameEntry::command("search", false),
            NameEntry::command("list", false),
            NameEntry::command("last", false),
            NameEntry::command("run", true),
            NameEntry::command("read", false),
            NameEntry::command("decompress", true),
            NameEntry::command("compress", true),
            NameEntry::command("environment", false),
            NameEntry::synonym("delete", NameKind::Command, "remove"),
            NameEntry::synonym("rm", NameKind::Command, "remove"),
            NameEntry::synonym("head", NameKind::Command, "first"),
            NameEntry::synonym("work", NameKind::Command, "jobs"),
            NameEntry::habit("sed"),
            NameEntry::habit("head"),
        ])
    }

    fn params() -> NameIndex {
        NameIndex::new(
            [
                "case_sensitive",
                "depth",
                "glob",
                "on_conflict",
                "on_missing",
                "type",
                "hidden",
            ]
            .map(|name| NameEntry::name(name, NameKind::Param)),
        )
    }

    fn find(s: &str, index: &NameIndex, kind: NameKind) -> Nearest {
        nearest(s, index, kind, &Options::default())
    }

    #[test]
    fn folds_conventions_to_snake_case() {
        assert_eq!(fold("open-files"), "open_files");
        assert_eq!(fold("OnConflict"), "on_conflict");
        assert_eq!(fold("HTTPRequest"), "http_request");
        assert_eq!(fold("base64Encode"), "base64_encode");
        assert_eq!(fold("snake_Case"), "snake_case");
        assert_eq!(fold("ÄBC"), "äbc");
    }

    #[test]
    fn counts_each_edit_operation_once() {
        assert_eq!(osa("remove", "remove"), 0);
        assert_eq!(osa("remov", "remove"), 1);
        assert_eq!(osa("removee", "remove"), 1);
        assert_eq!(osa("remeve", "remove"), 1);
        assert_eq!(osa("fnid", "find"), 1);
        assert_eq!(osa("case_sensitve", "case_sensitive"), 1);
        assert_eq!(osa("depht", "depth"), 1);
        assert_eq!(osa("", "abc"), 3);
        assert_eq!(osa("żółw", "żółe"), 1);
    }

    #[test]
    fn stops_early_beyond_the_limit() {
        let a: Vec<char> = "abcdefgh".chars().collect();
        let b: Vec<char> = "zyxwvuts".chars().collect();
        assert_eq!(osa_chars(&a, &b, 2), None);
        assert_eq!(osa_chars(&a, &a, 0), Some(0));
        assert_eq!(osa_chars(&a, &a[..4], 2), None);
    }

    #[test]
    fn thresholds_follow_name_length() {
        assert_eq!(
            (0..=9).map(threshold).collect::<Vec<_>>(),
            [0, 0, 0, 0, 1, 1, 1, 1, 2, 2]
        );
    }

    #[test]
    fn corrects_document_examples() {
        let index = commands();
        assert_eq!(
            find("remvoe", &index, NameKind::Command),
            Nearest::Unique {
                to: "remove".into(),
                distance: 1,
                effecting: true
            }
        );
        assert_eq!(
            find("serach", &index, NameKind::Command),
            Nearest::Unique {
                to: "search".into(),
                distance: 1,
                effecting: false
            }
        );
        let params = params();
        assert_eq!(
            find("case_sensitve", &params, NameKind::Param),
            Nearest::Unique {
                to: "case_sensitive".into(),
                distance: 1,
                effecting: false
            }
        );
        assert_eq!(
            find("depht", &params, NameKind::Param),
            Nearest::Unique {
                to: "depth".into(),
                distance: 1,
                effecting: false
            }
        );
    }

    #[test]
    fn prefers_exact_names_and_synonyms() {
        let index = commands();
        assert_eq!(
            find("REMOVE", &index, NameKind::Command),
            Nearest::Exact {
                to: "remove".into()
            }
        );
        assert_eq!(
            find("delete", &index, NameKind::Command),
            Nearest::Synonym {
                to: "remove".into()
            }
        );
        assert_eq!(
            find("Work", &index, NameKind::Command),
            Nearest::Synonym { to: "jobs".into() }
        );
        assert_eq!(find("sed", &index, NameKind::Command), Nearest::None);
    }

    #[test]
    fn never_corrects_short_names_by_distance() {
        let index = commands();
        assert_eq!(find("rnu", &index, NameKind::Command), Nearest::None);
        assert_eq!(
            find("ru", &index, NameKind::Command),
            Nearest::Tie {
                names: vec!["rm".into(), "run".into()]
            }
        );
    }

    #[test]
    fn requires_the_first_character() {
        let index = commands();
        assert_eq!(find("temove", &index, NameKind::Command), Nearest::None);
        assert_eq!(
            find("emove", &index, NameKind::Command),
            Nearest::Tie {
                names: vec!["move".into(), "remove".into()]
            }
        );
        let relaxed = Options {
            require_first_char: false,
        };
        assert_eq!(
            nearest("temove", &index, NameKind::Command, &relaxed),
            Nearest::Unique {
                to: "remove".into(),
                distance: 1,
                effecting: true
            }
        );
    }

    #[test]
    fn rejects_effecting_commands_at_distance_two() {
        let index = commands();
        assert_eq!(find("decmpres", &index, NameKind::Command), Nearest::None);
        assert_eq!(
            find("environmnt", &index, NameKind::Command),
            Nearest::Unique {
                to: "environment".into(),
                distance: 1,
                effecting: false
            }
        );
        assert_eq!(
            find("enviromnet", &index, NameKind::Command),
            Nearest::Unique {
                to: "environment".into(),
                distance: 2,
                effecting: false
            }
        );
    }

    #[test]
    fn treats_ties_and_habits_as_competitors() {
        let index = commands();
        assert_eq!(
            find("lest", &index, NameKind::Command),
            Nearest::Tie {
                names: vec!["last".into(), "list".into()]
            }
        );
        assert_eq!(find("heaf", &index, NameKind::Command), Nearest::None);
        assert_eq!(find("deleet", &index, NameKind::Command), Nearest::None);
    }

    #[test]
    fn selects_unambiguous_prefixes_of_parameters() {
        let params = params();
        assert_eq!(
            find("on_conf", &params, NameKind::Param),
            Nearest::Prefix {
                to: "on_conflict".into()
            }
        );
        assert_eq!(find("on_", &params, NameKind::Param), Nearest::None);
        assert_eq!(find("hi", &params, NameKind::Param), Nearest::None);
        assert_eq!(
            find("hid", &params, NameKind::Param),
            Nearest::Prefix {
                to: "hidden".into()
            }
        );
        let index = commands();
        assert_eq!(find("decomp", &index, NameKind::Command), Nearest::None);
    }

    #[test]
    fn reports_a_tie_when_prefix_and_distance_disagree() {
        let values = NameIndex::new(
            ["directory", "dirt", "file"].map(|name| NameEntry::name(name, NameKind::Value)),
        );
        assert_eq!(
            find("dire", &values, NameKind::Value),
            Nearest::Tie {
                names: vec!["directory".into(), "dirt".into()]
            }
        );
        assert_eq!(
            find("dirt", &values, NameKind::Value),
            Nearest::Exact { to: "dirt".into() }
        );
    }

    #[test]
    fn corrects_let_names_at_distance_one() {
        let lets =
            NameIndex::new(["changed", "ab"].map(|name| NameEntry::name(name, NameKind::Let)));
        assert_eq!(
            find("chnaged", &lets, NameKind::Let),
            Nearest::Unique {
                to: "changed".into(),
                distance: 1,
                effecting: false
            }
        );
        assert_eq!(
            find("ac", &lets, NameKind::Let),
            Nearest::Unique {
                to: "ab".into(),
                distance: 1,
                effecting: false
            }
        );
        assert_eq!(find("chngd", &lets, NameKind::Let), Nearest::None);
    }

    #[test]
    fn lists_expected_names_by_distance() {
        let index = commands();
        let names = expected("remvoe", &index, NameKind::Command, EXPECTED_LIMIT);
        assert_eq!(names.first().map(String::as_str), Some("remove"));
        assert!(names.len() <= EXPECTED_LIMIT);
        assert!(!names.contains(&"delete".to_owned()));
    }

    #[test]
    fn reports_conflicts_with_synonyms_and_habits() {
        let entries = [
            NameEntry::command("read", false),
            NameEntry::command("set", true),
            NameEntry::command("find", false),
            NameEntry::synonym("head", NameKind::Command, "first"),
            NameEntry::habit("sed"),
            NameEntry::habit("find"),
            NameEntry::habit("cd"),
            NameEntry::name("verify", NameKind::Group),
            NameEntry::command("verify", false),
        ];
        let conflicts = check_names(&entries, &[]);
        let found: Vec<(&str, &str, usize)> = conflicts
            .iter()
            .map(|pair| (pair.a.as_str(), pair.b.as_str(), pair.distance))
            .collect();
        assert_eq!(
            found,
            [("find", "find", 0), ("head", "read", 1), ("sed", "set", 1)]
        );
        assert_eq!(
            check_names(
                &entries,
                &[("find", "find"), ("read", "head"), ("set", "sed")]
            ),
            []
        );
    }

    #[test]
    fn options_follow_the_configuration() {
        let mut config = CorrectionConfig::default();
        config.require_first_char = false;
        assert!(!Options::from(&config).require_first_char);
        assert!(Options::default().require_first_char);
    }

    #[test]
    fn handles_empty_and_multibyte_names() {
        let index = NameIndex::new(
            ["żółw", "żółć"].map(|name| NameEntry::command(name, false)),
        );
        assert_eq!(find("", &index, NameKind::Command), Nearest::None);
        assert_eq!(
            find("ŻÓŁW", &index, NameKind::Command),
            Nearest::Exact {
                to: "żółw".into()
            }
        );
        assert_eq!(
            find("żółx", &index, NameKind::Command),
            Nearest::Tie {
                names: vec!["żółw".into(), "żółć".into()]
            }
        );
        assert_eq!(osa("żółw", ""), 4);
    }

    #[test]
    fn index_reports_size() {
        let index = commands();
        assert_eq!(index.len(), 17);
        assert!(!index.is_empty());
        assert!(NameIndex::default().is_empty());
        assert_eq!(index.entries().count(), 17);
        assert_eq!(NameKind::parse("value"), Some(NameKind::Value));
        assert_eq!(NameKind::parse("other"), None);
    }
}
