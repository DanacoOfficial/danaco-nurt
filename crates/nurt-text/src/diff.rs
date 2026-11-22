// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Line differences between two texts and their unified rendering.
//!
//! [`unified_diff`] compares two texts line by line and returns the hunks of
//! the difference. [`Unified`] renders hunks in the unified format. For the
//! same input and the same number of context lines the rendered text equals
//! the output of GNU `diff -u` with file labels instead of time stamps.
//!
//! A line is the text up to and including a line feed. A carriage return
//! before the line feed belongs to the line text, so `crlf` and `lf` lines
//! differ unless white space is ignored. The last line of a text without a
//! line feed is marked with [`HunkLine::no_newline`] and rendered with the
//! marker line `\ No newline at end of file`.
//!
//! # Examples
//!
//! ```
//! use nurt_text::diff::{Unified, unified_diff};
//!
//! let hunks = unified_diff("a\nb\nc\n", "a\nB\nc\n", 3, false);
//! let text = Unified::new("a/notes.txt", "b/notes.txt", &hunks).to_string();
//! assert_eq!(
//!     text,
//!     "--- a/notes.txt\n+++ b/notes.txt\n@@ -1,3 +1,3 @@\n a\n-b\n+B\n c\n"
//! );
//! ```

use std::borrow::Cow;
use std::collections::HashMap;
use std::fmt;

/// Marker line that follows a line without a line feed in unified format.
pub const NO_NEWLINE_MARKER: &str = "\\ No newline at end of file";

/// Smallest number of edit steps after which the search for a shortest edit
/// script stops and takes the furthest reaching paths instead.
///
/// The limit grows with the square root of the input size and bounds the
/// running time on large inputs with many changes; below it the difference
/// is minimal. The value and the growth are those of GNU `diff`.
const MIN_EXPENSIVE_COST: usize = 4096;

/// Role of a line in a hunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LineKind {
    /// The line is present in both texts.
    Context,
    /// The line is present only in the old text.
    Removed,
    /// The line is present only in the new text.
    Added,
}

impl LineKind {
    /// Returns the prefix of the line in unified format: ` `, `-` or `+`.
    #[must_use]
    pub const fn prefix(self) -> char {
        match self {
            Self::Context => ' ',
            Self::Removed => '-',
            Self::Added => '+',
        }
    }

    /// Returns the role of the line after the old and new texts are swapped.
    #[must_use]
    pub const fn reversed(self) -> Self {
        match self {
            Self::Context => Self::Context,
            Self::Removed => Self::Added,
            Self::Added => Self::Removed,
        }
    }

    /// Returns `true` for lines of the old text: context and removed lines.
    #[must_use]
    pub const fn in_old(self) -> bool {
        matches!(self, Self::Context | Self::Removed)
    }

    /// Returns `true` for lines of the new text: context and added lines.
    #[must_use]
    pub const fn in_new(self) -> bool {
        matches!(self, Self::Context | Self::Added)
    }
}

/// Line of a hunk.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct HunkLine {
    /// Role of the line.
    pub kind: LineKind,
    /// Text of the line without the line feed. The carriage return of a
    /// `crlf` line is part of the text.
    pub text: String,
    /// The line is the last line of its text and has no line feed.
    pub no_newline: bool,
}

impl HunkLine {
    /// Creates a line that ends with a line feed.
    #[must_use]
    pub fn new(kind: LineKind, text: impl Into<String>) -> Self {
        Self {
            kind,
            text: text.into(),
            no_newline: false,
        }
    }
}

/// Contiguous region of a difference with its context lines.
///
/// Line numbers are 1-based. A range of length zero starts at the number of
/// the line before it, so a hunk that adds lines to an empty text has
/// `old_start` and `old_len` equal to zero, as in unified format.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Hunk {
    /// First line of the range in the old text.
    pub old_start: usize,
    /// Number of lines of the range in the old text.
    pub old_len: usize,
    /// First line of the range in the new text.
    pub new_start: usize,
    /// Number of lines of the range in the new text.
    pub new_len: usize,
    /// Section text after the range line, as in `@@ -1,3 +1,4 @@ fn main()`.
    pub header: Option<String>,
    /// Lines of the hunk in order.
    pub lines: Vec<HunkLine>,
}

impl Hunk {
    /// Returns the hunk that undoes this hunk: removed and added lines and
    /// the old and new ranges are swapped.
    #[must_use]
    pub fn reversed(&self) -> Self {
        Self {
            old_start: self.new_start,
            old_len: self.new_len,
            new_start: self.old_start,
            new_len: self.old_len,
            header: self.header.clone(),
            lines: self
                .lines
                .iter()
                .map(|line| HunkLine {
                    kind: line.kind.reversed(),
                    ..line.clone()
                })
                .collect(),
        }
    }

    /// Returns the lines with their prefixes, each marker line
    /// [`NO_NEWLINE_MARKER`] as a separate item, as they appear in unified
    /// format.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_text::diff::unified_diff;
    ///
    /// let hunks = unified_diff("a\n", "b", 3, false);
    /// assert_eq!(
    ///     hunks[0].prefixed_lines(),
    ///     ["-a", "+b", "\\ No newline at end of file"]
    /// );
    /// ```
    #[must_use]
    pub fn prefixed_lines(&self) -> Vec<String> {
        let mut out = Vec::with_capacity(self.lines.len());
        for line in &self.lines {
            let mut item = String::with_capacity(line.text.len() + 1);
            item.push(line.kind.prefix());
            item.push_str(&line.text);
            out.push(item);
            if line.no_newline {
                out.push(NO_NEWLINE_MARKER.to_owned());
            }
        }
        out
    }

    /// Returns the number of added and the number of removed lines.
    #[must_use]
    pub fn counts(&self) -> (usize, usize) {
        self.lines
            .iter()
            .fold((0, 0), |(added, removed), line| match line.kind {
                LineKind::Added => (added + 1, removed),
                LineKind::Removed => (added, removed + 1),
                LineKind::Context => (added, removed),
            })
    }
}

impl fmt::Display for Hunk {
    /// Writes the range line and the lines of the hunk in unified format.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("@@ -")?;
        write_range(f, self.old_start, self.old_len)?;
        f.write_str(" +")?;
        write_range(f, self.new_start, self.new_len)?;
        f.write_str(" @@")?;
        match &self.header {
            Some(header) => writeln!(f, " {header}")?,
            None => writeln!(f)?,
        }
        for line in &self.lines {
            writeln!(f, "{}{}", line.kind.prefix(), line.text)?;
            if line.no_newline {
                writeln!(f, "{NO_NEWLINE_MARKER}")?;
            }
        }
        Ok(())
    }
}

/// Writes a range as `start,len`, or as `start` when the range has one line.
fn write_range(f: &mut fmt::Formatter<'_>, start: usize, len: usize) -> fmt::Result {
    if len == 1 {
        write!(f, "{start}")
    } else {
        write!(f, "{start},{len}")
    }
}

/// Difference of two texts in unified format.
///
/// The rendering starts with the lines `--- old_label` and `+++ new_label`
/// followed by the hunks. A difference without hunks renders as empty text.
#[derive(Debug, Clone, Copy)]
pub struct Unified<'a> {
    old_label: &'a str,
    new_label: &'a str,
    hunks: &'a [Hunk],
}

impl<'a> Unified<'a> {
    /// Creates the rendering of `hunks` with the given file labels.
    #[must_use]
    pub const fn new(old_label: &'a str, new_label: &'a str, hunks: &'a [Hunk]) -> Self {
        Self {
            old_label,
            new_label,
            hunks,
        }
    }
}

impl fmt::Display for Unified<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.hunks.is_empty() {
            return Ok(());
        }
        writeln!(f, "--- {}\n+++ {}", self.old_label, self.new_label)?;
        self.hunks.iter().try_for_each(|hunk| hunk.fmt(f))
    }
}

/// Line of a text: its content without the line feed and whether the line
/// feed is present.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Line<'t> {
    pub(crate) text: &'t str,
    pub(crate) newline: bool,
}

/// Splits a text into lines at line feeds.
pub(crate) fn split_lines(text: &str) -> Vec<Line<'_>> {
    text.split_inclusive('\n')
        .map(|segment| match segment.strip_suffix('\n') {
            Some(text) => Line {
                text,
                newline: true,
            },
            None => Line {
                text: segment,
                newline: false,
            },
        })
        .collect()
}

/// Compares two texts and returns the hunks of their difference.
///
/// `context` is the number of unchanged lines shown before and after each
/// change; changes separated by at most twice that many unchanged lines share
/// a hunk. With `ignore_whitespace` lines that differ only in white space
/// (space, tab, vertical tab, form feed and carriage return) are equal, as
/// with `diff -w`; context lines are then taken from the old text.
///
/// Equal texts give no hunks.
///
/// # Examples
///
/// ```
/// use nurt_text::diff::unified_diff;
///
/// let hunks = unified_diff("one\ntwo\n", "one\n2\n", 1, false);
/// assert_eq!(hunks.len(), 1);
/// assert_eq!((hunks[0].old_start, hunks[0].old_len), (1, 2));
/// assert_eq!(hunks[0].prefixed_lines(), [" one", "-two", "+2"]);
/// ```
#[must_use]
pub fn unified_diff(old: &str, new: &str, context: usize, ignore_whitespace: bool) -> Vec<Hunk> {
    let old_lines = split_lines(old);
    let new_lines = split_lines(new);
    let (old_changed, new_changed) =
        changed_lines(&old_lines, &new_lines, context, ignore_whitespace);
    let changes = collect_changes(&old_changed, &new_changed);
    group_hunks(&changes, context)
        .map(|group| build_hunk(group, &old_lines, &new_lines, context))
        .collect()
}

/// Marks the changed lines of each text.
///
/// Lines identical in both texts at their start and at their end are left
/// out of the comparison, except `horizon` lines next to the compared part,
/// as GNU `diff` does with its horizon set to the number of context lines.
/// Runs of changes slide only within the compared part.
fn changed_lines(
    old: &[Line<'_>],
    new: &[Line<'_>],
    horizon: usize,
    ignore_whitespace: bool,
) -> (Vec<bool>, Vec<bool>) {
    let (start, old_end, new_end) = compared_part(old, new, horizon);
    let mut old_changed = vec![false; old.len()];
    let mut new_changed = vec![false; new.len()];
    let (old_part, new_part) = compare(
        &old[start..old_end],
        &new[start..new_end],
        ignore_whitespace,
    );
    old_changed[start..old_end].copy_from_slice(&old_part);
    new_changed[start..new_end].copy_from_slice(&new_part);
    (old_changed, new_changed)
}

/// Returns the first compared line and the end of the compared part of the
/// old and of the new text.
fn compared_part(old: &[Line<'_>], new: &[Line<'_>], horizon: usize) -> (usize, usize, usize) {
    let prefix = old.iter().zip(new).take_while(|(a, b)| a == b).count();
    let start = prefix - prefix.min(horizon);
    let limit = old.len().min(new.len()) - start;
    let suffix = old
        .iter()
        .rev()
        .zip(new.iter().rev())
        .take(limit)
        .take_while(|(a, b)| a == b)
        .count();
    let excluded = suffix - suffix.min(horizon);
    (start, old.len() - excluded, new.len() - excluded)
}

/// Compares two sequences of lines and marks the changed lines of each.
fn compare(old: &[Line<'_>], new: &[Line<'_>], ignore_whitespace: bool) -> (Vec<bool>, Vec<bool>) {
    let (old_classes, new_classes, class_count) = classify(old, new, ignore_whitespace);
    let mut old_count = vec![0_usize; class_count];
    let mut new_count = vec![0_usize; class_count];
    old_classes.iter().for_each(|&class| old_count[class] += 1);
    new_classes.iter().for_each(|&class| new_count[class] += 1);
    let old_marks = marks(&old_classes, &new_count);
    let new_marks = marks(&new_classes, &old_count);
    let (old_kept, old_index) = kept(&old_classes, &old_marks);
    let (new_kept, new_index) = kept(&new_classes, &new_marks);

    let mut search = Search::new(&old_kept, &new_kept);
    search.run();
    let mut old_changed: Vec<bool> = old_marks.iter().map(|&mark| mark != Mark::Keep).collect();
    let mut new_changed: Vec<bool> = new_marks.iter().map(|&mark| mark != Mark::Keep).collect();
    for (position, &changed) in search.old_changed.iter().enumerate() {
        old_changed[old_index[position]] |= changed;
    }
    for (position, &changed) in search.new_changed.iter().enumerate() {
        new_changed[new_index[position]] |= changed;
    }

    slide_runs(&old_classes, &mut old_changed, &new_changed);
    slide_runs(&new_classes, &mut new_changed, &old_changed);
    (old_changed, new_changed)
}

/// Assigns each line a class number; lines of one class are equal.
///
/// Without `ignore_whitespace` the last line without a line feed is equal
/// only to another such line, so a missing line feed is a change.
fn classify<'t>(
    old: &[Line<'t>],
    new: &[Line<'t>],
    ignore_whitespace: bool,
) -> (Vec<usize>, Vec<usize>, usize) {
    let mut classes: HashMap<(Cow<'t, str>, bool), usize> = HashMap::new();
    let mut class_of = |line: &Line<'t>| {
        let key = if ignore_whitespace {
            (
                Cow::Owned(line.text.chars().filter(|&ch| !is_space(ch)).collect()),
                true,
            )
        } else {
            (Cow::Borrowed(line.text), line.newline)
        };
        let next = classes.len();
        *classes.entry(key).or_insert(next)
    };
    let old_classes: Vec<usize> = old.iter().map(&mut class_of).collect();
    let new_classes: Vec<usize> = new.iter().map(&mut class_of).collect();
    let count = old_classes
        .iter()
        .chain(&new_classes)
        .max()
        .map_or(0, |&max| max + 1);
    (old_classes, new_classes, count)
}

/// White space ignored by `ignore_whitespace`; the set of the C locale
/// without the line feed, which ends a line.
const fn is_space(ch: char) -> bool {
    matches!(ch, ' ' | '\t' | '\u{0B}' | '\u{0C}' | '\r')
}

/// Treatment of a line before the search for the common subsequence.
///
/// Lines that cannot or need not take part in the search are left out of it,
/// which shrinks the problem; the rules are those of GNU `diff`, so that the
/// resulting difference is the same.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mark {
    /// The line takes part in the search.
    Keep,
    /// The line does not occur in the other text and is a change.
    Discard,
    /// The line occurs in the other text very often; it is a change when it
    /// lies well inside a run of discarded lines.
    Provisional,
}

/// Number of occurrences in the other text above which a line is marked
/// provisional, for a text shorter than 4 times [`MANY_SCALE`] lines.
const MANY_MATCHES: usize = 5;
/// Text length unit of the provisional threshold; the threshold doubles for
/// each factor of 4 in the number of units.
const MANY_SCALE: usize = 64;
/// A run of discarded lines keeps all its provisional lines when more than
/// one in this many lines of the run are provisional.
const PROVISIONAL_SHARE: usize = 4;
/// Lines scanned from each edge of a run before provisional lines may stay
/// discarded.
const EDGE_SCAN: usize = 8;
/// Number of consecutive discarded lines that ends the scan of an edge.
const EDGE_RUN: usize = 3;

/// Marks the lines of one text given the number of occurrences of each class
/// in the other text.
fn marks(classes: &[usize], other_count: &[usize]) -> Vec<Mark> {
    let mut many = MANY_MATCHES;
    let mut scale = classes.len() / MANY_SCALE;
    loop {
        scale >>= 2;
        if scale == 0 {
            break;
        }
        many *= 2;
    }
    let mut marks: Vec<Mark> = classes
        .iter()
        .map(|&class| match other_count[class] {
            0 => Mark::Discard,
            count if count > many => Mark::Provisional,
            _ => Mark::Keep,
        })
        .collect();
    settle(&mut marks);
    marks
}

/// Decides each provisional mark: provisional lines stay out of the search
/// only inside a run of discarded lines that starts and ends with a
/// discarded line.
fn settle(marks: &mut [Mark]) {
    let mut i = 0;
    while i < marks.len() {
        match marks[i] {
            Mark::Provisional => marks[i] = Mark::Keep,
            Mark::Discard => {
                let mut end = i + marks[i..]
                    .iter()
                    .take_while(|&&mark| mark != Mark::Keep)
                    .count();
                while end > i && marks[end - 1] == Mark::Provisional {
                    end -= 1;
                    marks[end] = Mark::Keep;
                }
                let run = &mut marks[i..end];
                let provisional = run
                    .iter()
                    .filter(|&&mark| mark == Mark::Provisional)
                    .count();
                if provisional * PROVISIONAL_SHARE > run.len() {
                    run.iter_mut()
                        .filter(|mark| **mark == Mark::Provisional)
                        .for_each(|mark| *mark = Mark::Keep);
                } else {
                    settle_run(run);
                    i = end - 1;
                }
            }
            Mark::Keep => {}
        }
        i += 1;
    }
}

/// Decides the provisional marks of a run of discarded lines whose first and
/// last lines are discarded.
fn settle_run(run: &mut [Mark]) {
    // Subruns of provisional lines at least as long as the square root of a
    // quarter of the run, plus one, are kept.
    let mut minimum = 1;
    let mut scale = run.len() >> 2;
    loop {
        scale >>= 2;
        if scale == 0 {
            break;
        }
        minimum <<= 1;
    }
    minimum += 1;
    let (mut j, mut consecutive) = (0, 0);
    while j < run.len() {
        if run[j] == Mark::Provisional {
            consecutive += 1;
            if consecutive == minimum {
                j = j + 1 - consecutive;
                continue;
            }
            if consecutive > minimum {
                run[j] = Mark::Keep;
            }
        } else {
            consecutive = 0;
        }
        j += 1;
    }
    settle_edge(run.iter_mut());
    settle_edge(run.iter_mut().rev());
}

/// Keeps the provisional lines at an edge of a run until three consecutive
/// discarded lines, or a discarded line far enough from the edge, are found.
fn settle_edge<'m>(marks: impl Iterator<Item = &'m mut Mark>) {
    let mut consecutive = 0;
    for (j, mark) in marks.enumerate() {
        if j >= EDGE_SCAN && *mark == Mark::Discard {
            break;
        }
        match *mark {
            Mark::Provisional => {
                consecutive = 0;
                *mark = Mark::Keep;
            }
            Mark::Keep => consecutive = 0,
            Mark::Discard => consecutive += 1,
        }
        if consecutive == EDGE_RUN {
            break;
        }
    }
}

/// Returns the classes of the lines that take part in the search and their
/// positions.
fn kept(classes: &[usize], marks: &[Mark]) -> (Vec<usize>, Vec<usize>) {
    classes
        .iter()
        .zip(marks)
        .enumerate()
        .filter(|(_, (_, mark))| **mark == Mark::Keep)
        .map(|(position, (&class, _))| (class, position))
        .unzip()
}

/// Rectangle of the edit graph between `(x_lo, y_lo)` and `(x_hi, y_hi)`.
#[derive(Debug, Clone, Copy)]
struct Bounds {
    x_lo: isize,
    x_hi: isize,
    y_lo: isize,
    y_hi: isize,
}

/// Point that splits a problem in two, and whether each part must be solved
/// minimally.
#[derive(Debug, Clone, Copy)]
struct Split {
    x: usize,
    y: usize,
    lo_minimal: bool,
    hi_minimal: bool,
}

impl Split {
    fn exact(x: isize, y: isize) -> Self {
        Self {
            x: to_usize(x),
            y: to_usize(y),
            lo_minimal: true,
            hi_minimal: true,
        }
    }
}

/// Search for a shortest edit script by the linear space variant of the
/// algorithm of E. Myers, "An O(ND) Difference Algorithm and Its
/// Variations", Algorithmica 1 (1986), with the cost limit of GNU `diff`.
struct Search<'s> {
    old: &'s [usize],
    new: &'s [usize],
    old_changed: Vec<bool>,
    new_changed: Vec<bool>,
    /// Furthest reaching `x` of the forward search, by diagonal `x - y`.
    forward: Vec<isize>,
    /// Furthest reaching `x` of the backward search, by diagonal `x - y`.
    backward: Vec<isize>,
    /// Index of diagonal zero in `forward` and `backward`.
    origin: isize,
    /// Cost after which a search that need not be minimal stops.
    too_expensive: usize,
}

/// Value outside every diagonal of the forward search.
const FORWARD_NONE: isize = -1;
/// Value outside every diagonal of the backward search.
const BACKWARD_NONE: isize = isize::MAX;

impl<'s> Search<'s> {
    fn new(old: &'s [usize], new: &'s [usize]) -> Self {
        // Diagonals range from -len(new) to len(old); one guard on each side.
        let span = old.len() + new.len() + 3;
        let mut too_expensive = 1_usize;
        let mut diagonals = span;
        while diagonals != 0 {
            too_expensive <<= 1;
            diagonals >>= 2;
        }
        Self {
            old,
            new,
            old_changed: vec![false; old.len()],
            new_changed: vec![false; new.len()],
            forward: vec![FORWARD_NONE; span],
            backward: vec![BACKWARD_NONE; span],
            origin: to_isize(new.len()) + 1,
            too_expensive: too_expensive.max(MIN_EXPENSIVE_COST),
        }
    }

    fn run(&mut self) {
        let mut pending = vec![(0, self.old.len(), 0, self.new.len(), false)];
        while let Some((mut x_lo, mut x_hi, mut y_lo, mut y_hi, minimal)) = pending.pop() {
            while x_lo < x_hi && y_lo < y_hi && self.old[x_lo] == self.new[y_lo] {
                x_lo += 1;
                y_lo += 1;
            }
            while x_lo < x_hi && y_lo < y_hi && self.old[x_hi - 1] == self.new[y_hi - 1] {
                x_hi -= 1;
                y_hi -= 1;
            }
            if x_lo == x_hi {
                self.new_changed[y_lo..y_hi].fill(true);
            } else if y_lo == y_hi {
                self.old_changed[x_lo..x_hi].fill(true);
            } else {
                let bounds = Bounds {
                    x_lo: to_isize(x_lo),
                    x_hi: to_isize(x_hi),
                    y_lo: to_isize(y_lo),
                    y_hi: to_isize(y_hi),
                };
                let split = self.split(bounds, minimal);
                pending.push((split.x, x_hi, split.y, y_hi, split.hi_minimal));
                pending.push((x_lo, split.x, y_lo, split.y, split.lo_minimal));
            }
        }
    }

    fn slot(&self, diagonal: isize) -> usize {
        to_usize(diagonal + self.origin)
    }

    fn equal(&self, x: isize, y: isize) -> bool {
        match (usize::try_from(x), usize::try_from(y)) {
            (Ok(x), Ok(y)) => self
                .old
                .get(x)
                .is_some_and(|class| self.new.get(y) == Some(class)),
            _ => false,
        }
    }

    /// Finds a point on an optimal path through `bounds` that splits the
    /// problem into two smaller ones.
    ///
    /// Both ranges are non-empty and their first and last elements differ, so
    /// the edit distance is at least two and the point found is neither
    /// corner. When the search need not be minimal and becomes too
    /// expensive, the point is taken from the furthest reaching paths.
    fn split(&mut self, bounds: Bounds, minimal: bool) -> Split {
        let diagonals = (bounds.x_lo - bounds.y_hi, bounds.x_hi - bounds.y_lo);
        let forward_mid = bounds.x_lo - bounds.y_lo;
        let backward_mid = bounds.x_hi - bounds.y_hi;
        // With an odd distance between the middle diagonals the paths meet
        // during a forward step, otherwise during a backward step.
        let odd = (forward_mid - backward_mid) % 2 != 0;
        let slot = self.slot(forward_mid);
        self.forward[slot] = bounds.x_lo;
        let slot = self.slot(backward_mid);
        self.backward[slot] = bounds.x_hi;
        let mut forward = (forward_mid, forward_mid);
        let mut backward = (backward_mid, backward_mid);

        let mut cost = 0;
        loop {
            cost += 1;
            forward = self.widen(forward, diagonals, true);
            if let Some(split) = self.forward_step(forward, backward, odd, bounds) {
                return split;
            }
            backward = self.widen(backward, diagonals, false);
            if let Some(split) = self.backward_step(backward, forward, odd, bounds) {
                return split;
            }
            if !minimal && cost >= self.too_expensive {
                return self.halfway(forward, backward, bounds);
            }
        }
    }

    /// Extends the range of active diagonals of one search by one step.
    fn widen(
        &mut self,
        (mut low, mut high): (isize, isize),
        (d_min, d_max): (isize, isize),
        forward: bool,
    ) -> (isize, isize) {
        let none = if forward { FORWARD_NONE } else { BACKWARD_NONE };
        if low > d_min {
            low -= 1;
            let slot = self.slot(low - 1);
            self.values(forward)[slot] = none;
        } else {
            low += 1;
        }
        if high < d_max {
            high += 1;
            let slot = self.slot(high + 1);
            self.values(forward)[slot] = none;
        } else {
            high -= 1;
        }
        (low, high)
    }

    fn values(&mut self, forward: bool) -> &mut [isize] {
        if forward {
            &mut self.forward
        } else {
            &mut self.backward
        }
    }

    fn forward_step(
        &mut self,
        (f_min, f_max): (isize, isize),
        (b_min, b_max): (isize, isize),
        odd: bool,
        bounds: Bounds,
    ) -> Option<Split> {
        let mut d = f_max;
        while d >= f_min {
            let below = self.forward[self.slot(d - 1)];
            let above = self.forward[self.slot(d + 1)];
            let mut x = if below < above { above } else { below + 1 };
            let mut y = x - d;
            while x < bounds.x_hi && y < bounds.y_hi && self.equal(x, y) {
                x += 1;
                y += 1;
            }
            let slot = self.slot(d);
            self.forward[slot] = x;
            if odd && (b_min..=b_max).contains(&d) && self.backward[slot] <= x {
                return Some(Split::exact(x, y));
            }
            d -= 2;
        }
        None
    }

    fn backward_step(
        &mut self,
        (b_min, b_max): (isize, isize),
        (f_min, f_max): (isize, isize),
        odd: bool,
        bounds: Bounds,
    ) -> Option<Split> {
        let mut d = b_max;
        while d >= b_min {
            let below = self.backward[self.slot(d - 1)];
            let above = self.backward[self.slot(d + 1)];
            let mut x = if below < above { below } else { above - 1 };
            let mut y = x - d;
            while x > bounds.x_lo && y > bounds.y_lo && self.equal(x - 1, y - 1) {
                x -= 1;
                y -= 1;
            }
            let slot = self.slot(d);
            self.backward[slot] = x;
            if !odd && (f_min..=f_max).contains(&d) && x <= self.forward[slot] {
                return Some(Split::exact(x, y));
            }
            d -= 2;
        }
        None
    }

    /// Chooses the better of the furthest reaching forward and backward
    /// points; only the part on the side of the chosen search must then be
    /// solved minimally.
    fn halfway(
        &self,
        (f_min, f_max): (isize, isize),
        (b_min, b_max): (isize, isize),
        bounds: Bounds,
    ) -> Split {
        let (mut forward_sum, mut forward_x) = (-1, 0);
        let mut d = f_max;
        while d >= f_min {
            let x = self.forward[self.slot(d)].min(bounds.x_hi);
            let (x, y) = if bounds.y_hi < x - d {
                (bounds.y_hi + d, bounds.y_hi)
            } else {
                (x, x - d)
            };
            if forward_sum < x + y {
                (forward_sum, forward_x) = (x + y, x);
            }
            d -= 2;
        }
        let (mut backward_sum, mut backward_x) = (isize::MAX, 0);
        let mut d = b_max;
        while d >= b_min {
            let x = self.backward[self.slot(d)].max(bounds.x_lo);
            let (x, y) = if x - d < bounds.y_lo {
                (bounds.y_lo + d, bounds.y_lo)
            } else {
                (x, x - d)
            };
            if x + y < backward_sum {
                (backward_sum, backward_x) = (x + y, x);
            }
            d -= 2;
        }
        let backward_gain = (bounds.x_hi + bounds.y_hi) - backward_sum;
        let forward_gain = forward_sum - (bounds.x_lo + bounds.y_lo);
        if backward_gain < forward_gain {
            Split {
                x: to_usize(forward_x),
                y: to_usize(forward_sum - forward_x),
                lo_minimal: true,
                hi_minimal: false,
            }
        } else {
            Split {
                x: to_usize(backward_x),
                y: to_usize(backward_sum - backward_x),
                lo_minimal: false,
                hi_minimal: true,
            }
        }
    }
}

fn to_isize(value: usize) -> isize {
    isize::try_from(value).unwrap_or(isize::MAX)
}

fn to_usize(value: isize) -> usize {
    usize::try_from(value).unwrap_or(0)
}

/// Moves each run of changed lines of one text to a canonical position.
///
/// A run of changed lines can slide over equal neighbouring lines without
/// changing the difference. Each run is first slid up and down as far as
/// possible, absorbing adjacent runs, until it stops growing. It is then
/// placed at the lowest position where it ends next to a change in the other
/// text, so removed and added lines form one block, or at the lowest position
/// reached when there is none. This gives the placement of GNU `diff`.
///
/// `other` holds the changed lines of the other text. Unchanged lines of both
/// texts pair up in order; [`Partner`] follows the line of the other text
/// paired with the line after the current run.
fn slide_runs(classes: &[usize], changed: &mut [bool], other: &[bool]) {
    let mut runs = Runs {
        classes,
        changed,
        partner: Partner {
            changed: other,
            position: 0,
        },
    };
    let end = runs.changed.len();
    let mut i = 0;
    loop {
        while i < end && !runs.changed[i] {
            runs.partner.pass_unchanged();
            i += 1;
        }
        if i == end {
            return;
        }
        let start = i;
        while i < end && runs.changed[i] {
            i += 1;
        }
        runs.partner.skip_changed();
        i = runs.place(start, i);
    }
}

/// Changed lines of one text during [`slide_runs`].
struct Runs<'a> {
    classes: &'a [usize],
    changed: &'a mut [bool],
    partner: Partner<'a>,
}

impl Runs<'_> {
    /// Moves the run `start..end` to its canonical position; returns the end
    /// of the run there.
    fn place(&mut self, mut start: usize, mut end: usize) -> usize {
        let mut aligned_end;
        loop {
            let length = end - start;
            (start, end) = self.slide_up(start, end);
            aligned_end = if self.partner.after_change() {
                end
            } else {
                self.changed.len()
            };
            (start, end, aligned_end) = self.slide_down(start, end, aligned_end);
            if length == end - start {
                break;
            }
        }
        while aligned_end < end {
            start -= 1;
            self.changed[start] = true;
            end -= 1;
            self.changed[end] = false;
            self.partner.step_back();
        }
        end
    }

    /// Slides the run up while the line before it equals its last line,
    /// joining runs it meets.
    fn slide_up(&mut self, mut start: usize, mut end: usize) -> (usize, usize) {
        while start > 0 && self.classes[start - 1] == self.classes[end - 1] {
            start -= 1;
            self.changed[start] = true;
            end -= 1;
            self.changed[end] = false;
            while start > 0 && self.changed[start - 1] {
                start -= 1;
            }
            self.partner.step_back();
        }
        (start, end)
    }

    /// Slides the run down while the line after it equals its first line,
    /// joining runs it meets; `aligned_end` becomes the last end at which
    /// the run meets a change of the other text.
    fn slide_down(
        &mut self,
        mut start: usize,
        mut end: usize,
        mut aligned_end: usize,
    ) -> (usize, usize, usize) {
        let limit = self.changed.len();
        while end < limit && self.classes[start] == self.classes[end] {
            self.changed[start] = false;
            start += 1;
            self.changed[end] = true;
            end += 1;
            while end < limit && self.changed[end] {
                end += 1;
            }
            if self.partner.step_forward() {
                aligned_end = end;
            }
        }
        (start, end, aligned_end)
    }
}

/// Position in the other text paired with the line after a run.
struct Partner<'a> {
    changed: &'a [bool],
    position: isize,
}

impl Partner<'_> {
    fn at(&self, position: isize) -> bool {
        usize::try_from(position)
            .is_ok_and(|index| self.changed.get(index).copied().unwrap_or(false))
    }

    fn skip_changed(&mut self) {
        while self.at(self.position) {
            self.position += 1;
        }
    }

    /// Moves past the changed lines and the next unchanged line.
    fn pass_unchanged(&mut self) {
        self.skip_changed();
        self.position += 1;
    }

    /// Moves to the previous unchanged line.
    fn step_back(&mut self) {
        self.position -= 1;
        while self.at(self.position) {
            self.position -= 1;
        }
    }

    /// Moves to the next unchanged line; returns `true` when changed lines
    /// precede it.
    fn step_forward(&mut self) -> bool {
        self.position += 1;
        let passed = self.at(self.position);
        self.skip_changed();
        passed
    }

    /// Returns `true` when a changed line precedes the current position.
    fn after_change(&self) -> bool {
        self.at(self.position - 1)
    }
}

/// Block of changed lines: `removed` lines of the old text from `old` and
/// `added` lines of the new text from `new`, both 0-based.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Change {
    old: usize,
    removed: usize,
    new: usize,
    added: usize,
}

fn collect_changes(old_changed: &[bool], new_changed: &[bool]) -> Vec<Change> {
    let (old_end, new_end) = (old_changed.len(), new_changed.len());
    let (mut i, mut j) = (0, 0);
    let mut changes = Vec::new();
    loop {
        while i < old_end && j < new_end && !old_changed[i] && !new_changed[j] {
            i += 1;
            j += 1;
        }
        let (old, new) = (i, j);
        while i < old_end && old_changed[i] {
            i += 1;
        }
        while j < new_end && new_changed[j] {
            j += 1;
        }
        if i == old && j == new {
            return changes;
        }
        changes.push(Change {
            old,
            removed: i - old,
            new,
            added: j - new,
        });
    }
}

/// Splits the changes into groups that share a hunk: the gap between
/// consecutive changes of a group is at most `2 * context` lines.
fn group_hunks(changes: &[Change], context: usize) -> impl Iterator<Item = &[Change]> {
    let limit = context.saturating_mul(2);
    changes.chunk_by(move |previous, next| next.old - (previous.old + previous.removed) <= limit)
}

fn build_hunk(group: &[Change], old: &[Line<'_>], new: &[Line<'_>], context: usize) -> Hunk {
    // Groups produced by `chunk_by` are never empty.
    let (first, last) = (group[0], group[group.len() - 1]);
    let old_from = first.old.saturating_sub(context);
    let new_from = first.new.saturating_sub(context);
    let old_to = (last.old + last.removed)
        .saturating_add(context)
        .min(old.len());
    let new_to = (last.new + last.added)
        .saturating_add(context)
        .min(new.len());

    let line_of = |kind, line: &Line<'_>| HunkLine {
        kind,
        text: line.text.to_owned(),
        no_newline: !line.newline,
    };
    let mut lines = Vec::with_capacity(old_to - old_from + new_to - new_from);
    let mut i = old_from;
    for change in group {
        let removed = change.old..change.old + change.removed;
        let added = change.new..change.new + change.added;
        lines.extend(
            old[i..change.old]
                .iter()
                .map(|line| line_of(LineKind::Context, line)),
        );
        lines.extend(
            old[removed]
                .iter()
                .map(|line| line_of(LineKind::Removed, line)),
        );
        lines.extend(new[added].iter().map(|line| line_of(LineKind::Added, line)));
        i = change.old + change.removed;
    }
    lines.extend(
        old[i..old_to]
            .iter()
            .map(|line| line_of(LineKind::Context, line)),
    );

    let old_len = old_to - old_from;
    let new_len = new_to - new_from;
    Hunk {
        old_start: range_start(old_from, old_len),
        old_len,
        new_start: range_start(new_from, new_len),
        new_len,
        header: None,
        lines,
    }
}

/// Converts a 0-based start index to the 1-based start of a range; an empty
/// range starts at the line before it.
const fn range_start(index: usize, len: usize) -> usize {
    if len == 0 { index } else { index + 1 }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(old: &str, new: &str, context: usize) -> String {
        Unified::new("a", "b", &unified_diff(old, new, context, false)).to_string()
    }

    fn numbered(count: usize) -> String {
        (1..=count).map(|n| format!("{n}\n")).collect()
    }

    #[test]
    fn equal_texts_have_no_hunks() {
        assert!(unified_diff("a\nb\n", "a\nb\n", 3, false).is_empty());
        assert_eq!(render("", "", 3), "");
    }

    #[test]
    fn splits_lines_at_line_feeds() {
        assert_eq!(split_lines(""), []);
        assert_eq!(
            split_lines("a\r\nb"),
            [
                Line {
                    text: "a\r",
                    newline: true
                },
                Line {
                    text: "b",
                    newline: false
                }
            ]
        );
    }

    #[test]
    fn adds_lines_to_empty_text() {
        assert_eq!(
            render("", "x\ny\n", 3),
            "--- a\n+++ b\n@@ -0,0 +1,2 @@\n+x\n+y\n"
        );
    }

    #[test]
    fn removes_all_lines() {
        assert_eq!(render("x\n", "", 3), "--- a\n+++ b\n@@ -1 +0,0 @@\n-x\n");
    }

    #[test]
    fn marks_missing_line_feed_as_change() {
        assert_eq!(
            render("a\nb", "a\nb\n", 3),
            "--- a\n+++ b\n@@ -1,2 +1,2 @@\n a\n-b\n\\ No newline at end of file\n+b\n"
        );
    }

    #[test]
    fn ignores_whitespace_on_request() {
        assert!(unified_diff("a b\n", "a  b\r\n", 3, true).is_empty());
        assert_eq!(unified_diff("a b\n", "a  b\n", 3, false).len(), 1);
    }

    #[test]
    fn keeps_distant_changes_in_separate_hunks() {
        let old = numbered(20);
        let new = old
            .replace("\n2\n", "\ntwo\n")
            .replace("\n18\n", "\neighteen\n");
        let hunks = unified_diff(&old, &new, 3, false);
        assert_eq!(hunks.len(), 2);
        assert_eq!((hunks[1].old_start, hunks[1].old_len), (15, 6));
    }

    #[test]
    fn merges_changes_within_twice_the_context() {
        let old = numbered(12);
        let new = old.replace("\n3\n", "\nc\n").replace("\n10\n", "\nj\n");
        assert_eq!(unified_diff(&old, &new, 3, false).len(), 1);
        assert_eq!(unified_diff(&old, &new, 2, false).len(), 2);
    }

    #[test]
    fn slides_insertion_of_repeated_line_down() {
        assert_eq!(
            render("a\nb\n", "a\nb\nb\n", 0),
            "--- a\n+++ b\n@@ -2,0 +3 @@\n+b\n"
        );
    }

    #[test]
    fn keeps_slides_inside_compared_part() {
        assert_eq!(
            render("b\n\na\n", "\na\na\n", 0),
            "--- a\n+++ b\n@@ -1 +0,0 @@\n-b\n@@ -2,0 +2 @@\n+a\n"
        );
    }

    #[test]
    fn limits_compared_part_by_horizon() {
        let old: Vec<Line<'_>> = split_lines("a\nb\nc\nd\ne\n");
        let new: Vec<Line<'_>> = split_lines("a\nb\nX\nd\ne\n");
        assert_eq!(compared_part(&old, &new, 0), (2, 3, 3));
        assert_eq!(compared_part(&old, &new, 1), (1, 4, 4));
        assert_eq!(compared_part(&old, &old, 0), (5, 5, 5));
    }

    #[test]
    fn keeps_provisional_lines_at_run_edges() {
        use Mark::{Discard, Keep, Provisional};
        let mut marks = vec![Provisional, Discard, Provisional, Discard, Provisional];
        settle(&mut marks);
        assert_eq!(marks, [Keep, Discard, Keep, Discard, Keep]);
    }

    #[test]
    fn discards_provisional_lines_deep_inside_long_runs() {
        use Mark::{Discard, Provisional};
        let mut marks = vec![Discard; 24];
        marks[12] = Provisional;
        settle(&mut marks);
        assert_eq!(marks[12], Provisional);
    }

    #[test]
    fn reversed_hunk_swaps_ranges_and_kinds() {
        let hunk = &unified_diff("a\nb\n", "a\nc\nd\n", 3, false)[0];
        let reversed = hunk.reversed();
        assert_eq!(
            (reversed.old_len, reversed.new_len),
            (hunk.new_len, hunk.old_len)
        );
        assert_eq!(reversed.counts(), (1, 2));
        assert_eq!(reversed.reversed(), *hunk);
    }

    #[test]
    fn renders_section_header() {
        let mut hunk = unified_diff("a\n", "b\n", 3, false).remove(0);
        hunk.header = Some("fn main()".to_owned());
        assert_eq!(hunk.to_string(), "@@ -1 +1 @@ fn main()\n-a\n+b\n");
    }

    #[test]
    fn finds_minimal_difference_of_shuffled_lines() {
        let old = "a\nb\nc\nd\ne\nf\n";
        let new = "b\na\nd\nc\nf\ne\n";
        let (added, removed) = unified_diff(old, new, 0, false)
            .iter()
            .map(Hunk::counts)
            .fold((0, 0), |(a, r), (x, y)| (a + x, r + y));
        assert_eq!((added, removed), (3, 3));
    }

    #[test]
    fn stops_expensive_search_with_valid_difference() {
        let old: String = (0..6000).map(|n| format!("{}\n", n % 7)).collect();
        let new: String = (0..6000).map(|n| format!("{}\n", n % 11)).collect();
        let hunks = unified_diff(&old, &new, 3, false);
        let old_lines: usize = hunks
            .iter()
            .map(|hunk| hunk.lines.iter().filter(|line| line.kind.in_old()).count())
            .sum();
        assert!(old_lines > 0);
        let rebuilt = crate::patch::apply_hunks(&old, &hunks, 0).unwrap();
        assert_eq!(rebuilt.text, new);
    }
}
