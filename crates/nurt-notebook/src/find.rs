// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Text patterns for searching and replacing within cells.

use std::fmt;
use std::ops::Range;

/// Pattern that finds text in a cell source or output.
///
/// [`Literal`] covers literal text. Regular expressions are provided by the
/// caller through their own implementation.
pub trait Pattern: fmt::Debug + Send + Sync {
    /// Byte ranges of the non-overlapping matches in `haystack`, in order.
    fn find_all(&self, haystack: &str) -> Vec<Range<usize>>;

    /// Replacement text for the match `found` of `haystack`; `with` is the
    /// replacement as written. Implementations with capture groups expand
    /// them here.
    fn expand(&self, haystack: &str, found: Range<usize>, with: &str) -> String {
        let _ = (haystack, found);
        with.to_owned()
    }

    /// Replacement texts for the matches `found` of `haystack`, in order.
    ///
    /// The default expands each match with [`Pattern::expand`]; an
    /// implementation whose `expand` has to search `haystack` again for the
    /// groups of every match evaluates them for all matches in one pass, so
    /// that a cell with many matches costs one search, not one per match.
    fn expand_all(&self, haystack: &str, found: &[Range<usize>], with: &str) -> Vec<String> {
        found
            .iter()
            .map(|range| self.expand(haystack, range.clone(), with))
            .collect()
    }

    /// The pattern as written, for messages.
    fn display(&self) -> String;
}

/// Literal text, optionally matched only as a whole word.
///
/// A whole-word match is neither preceded nor followed by a letter, a digit
/// or `_`, so `x` does not match within `max_x`.
///
/// # Examples
///
/// ```
/// use nurt_notebook::{Literal, Pattern};
///
/// let pattern = Literal::new("x", true);
/// assert_eq!(pattern.find_all("x = max_x + x"), vec![0..1, 12..13]);
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Literal {
    text: String,
    whole_word: bool,
}

impl Literal {
    /// Literal pattern for `text`.
    #[must_use]
    pub fn new(text: impl Into<String>, whole_word: bool) -> Self {
        Self {
            text: text.into(),
            whole_word,
        }
    }
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

impl Pattern for Literal {
    fn find_all(&self, haystack: &str) -> Vec<Range<usize>> {
        if self.text.is_empty() {
            return Vec::new();
        }
        haystack
            .match_indices(&self.text)
            .map(|(start, found)| start..start + found.len())
            .filter(|found| {
                !self.whole_word
                    || (!haystack[..found.start]
                        .chars()
                        .next_back()
                        .is_some_and(is_word_char)
                        && !haystack[found.end..]
                            .chars()
                            .next()
                            .is_some_and(is_word_char))
            })
            .collect()
    }

    fn display(&self) -> String {
        self.text.clone()
    }
}

/// Replaces every match of `pattern` in `haystack`; returns the new text and
/// the number of replacements.
pub(crate) fn replace_all(pattern: &dyn Pattern, haystack: &str, with: &str) -> (String, usize) {
    let found = pattern.find_all(haystack);
    if found.is_empty() {
        return (haystack.to_owned(), 0);
    }
    let replacements = pattern.expand_all(haystack, &found, with);
    let mut out = String::with_capacity(haystack.len());
    let mut cursor = 0;
    for (range, replacement) in found.iter().zip(&replacements) {
        out.push_str(&haystack[cursor..range.start]);
        out.push_str(replacement);
        cursor = range.end;
    }
    out.push_str(&haystack[cursor..]);
    (out, found.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whole_word_skips_matches_inside_identifiers() {
        let pattern = Literal::new("df", true);
        assert_eq!(
            pattern.find_all("df = df2 + my_df + df"),
            vec![0..2, 19..21]
        );
    }

    #[test]
    fn plain_literal_matches_anywhere() {
        let pattern = Literal::new("df", false);
        assert_eq!(pattern.find_all("df2 my_df").len(), 2);
    }

    #[test]
    fn empty_literal_matches_nothing() {
        assert!(Literal::new("", false).find_all("abc").is_empty());
    }

    #[test]
    fn replaces_every_match_and_counts() {
        let pattern = Literal::new("a", false);
        assert_eq!(
            replace_all(&pattern, "banana", "o"),
            ("bonono".to_owned(), 3)
        );
        assert_eq!(replace_all(&pattern, "xyz", "o"), ("xyz".to_owned(), 0));
    }
}
