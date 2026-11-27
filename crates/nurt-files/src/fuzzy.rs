// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-files::fuzzy
// Responsibility:   Similarity ranking of names for `find fuzzy:`.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Similarity ranking of names for `find fuzzy:`.
//!
//! A candidate matches a query when the characters of the query occur in the
//! candidate in the same order. Among the possible alignments the best one is
//! scored: every matched character earns [`SCORE_MATCH`], a character at the
//! start of the candidate, after a delimiter (`_`, `-`, `.`, `/`, `\`, space)
//! or at a lower-to-upper case or letter-to-digit transition earns a bonus,
//! a character that continues a run of matched characters earns the largest
//! of its own bonus, the bonus of the first character of the run and
//! [`BONUS_CONSECUTIVE`], and gaps between matched characters cost [`PENALTY_GAP_START`] plus
//! [`PENALTY_GAP_EXTENSION`] for every further skipped character. Characters
//! before the first match cost nothing. The weights follow the scoring of
//! common fuzzy finders, so rankings agree with what users of such tools
//! expect.
//!
//! # Examples
//!
//! ```
//! use nurt_files::fuzzy::FuzzyQuery;
//!
//! let query = FuzzyQuery::new("fb", false);
//! let boundary = query.rank("foo_bar.rs").expect("matches");
//! let inside = query.rank("fabric.rs").expect("matches");
//! assert!(boundary > inside);
//! assert_eq!(query.rank("bar.rs"), None);
//! ```

/// Score of one matched character.
pub const SCORE_MATCH: i32 = 16;

/// Cost of the first skipped character between two matched characters.
pub const PENALTY_GAP_START: i32 = 3;

/// Cost of every further skipped character of the same gap.
pub const PENALTY_GAP_EXTENSION: i32 = 1;

/// Bonus of a match at the first character of the candidate.
pub const BONUS_START: i32 = 10;

/// Bonus of a match right after a delimiter.
pub const BONUS_BOUNDARY: i32 = 8;

/// Bonus of a match at a lower-to-upper case or letter-to-digit transition.
pub const BONUS_CAMEL_CASE: i32 = 7;

/// Smallest bonus of a match that continues a run of matched characters.
pub const BONUS_CONSECUTIVE: i32 = PENALTY_GAP_START + PENALTY_GAP_EXTENSION;

/// Factor applied to the bonus of the first character of the query.
pub const FIRST_CHARACTER_MULTIPLIER: i32 = 2;

/// Score of an impossible alignment; far from overflow when penalties are
/// added.
const IMPOSSIBLE: i32 = i32::MIN / 2;

/// Query compiled for repeated ranking.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FuzzyQuery {
    chars: Vec<char>,
    case_sensitive: bool,
}

impl FuzzyQuery {
    /// Compiles a query. With `case_sensitive: false`, letters match
    /// regardless of case.
    #[must_use]
    pub fn new(query: &str, case_sensitive: bool) -> Self {
        let chars = query
            .chars()
            .map(|c| if case_sensitive { c } else { fold(c) })
            .collect();
        Self {
            chars,
            case_sensitive,
        }
    }

    /// Returns `true` for an empty query, which matches every candidate with
    /// rank 0.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.chars.is_empty()
    }

    /// Rank of `candidate`; `None` when the query does not match it. A
    /// higher rank denotes a closer match.
    ///
    /// ```
    /// use nurt_files::fuzzy::FuzzyQuery;
    ///
    /// let query = FuzzyQuery::new("cfg", true);
    /// let prefix = query.rank("cfg.toml").expect("matches");
    /// let scattered = query.rank("conflagration").expect("matches");
    /// assert!(prefix > scattered);
    /// ```
    #[must_use]
    pub fn rank(&self, candidate: &str) -> Option<u32> {
        if self.chars.is_empty() {
            return Some(0);
        }
        let text: Vec<char> = candidate.chars().collect();
        if text.len() < self.chars.len() {
            return None;
        }
        let bonuses = bonuses(&text);
        let folded: Vec<char> = if self.case_sensitive {
            text
        } else {
            text.into_iter().map(fold).collect()
        };
        let best = self.align(&folded, &bonuses)?;
        Some(u32::try_from(best.max(0)).unwrap_or(0))
    }

    /// Best alignment score, computed row by row: `previous[j]` is the best
    /// score of the query prefix ending with its last character at
    /// candidate position `j`, and `previous_run[j]` is the bonus of the
    /// first character of the run of consecutive matches ending there.
    fn align(&self, text: &[char], bonuses: &[i32]) -> Option<i32> {
        let mut previous = vec![IMPOSSIBLE; text.len()];
        let mut current = vec![IMPOSSIBLE; text.len()];
        let mut previous_run = vec![0; text.len()];
        let mut current_run = vec![0; text.len()];
        for (row, &wanted) in self.chars.iter().enumerate() {
            let mut gap = IMPOSSIBLE;
            for (column, &found) in text.iter().enumerate() {
                if row > 0 && column >= 2 {
                    gap =
                        (gap - PENALTY_GAP_EXTENSION).max(previous[column - 2] - PENALTY_GAP_START);
                }
                let bonus = bonuses[column];
                (current[column], current_run[column]) = if found != wanted {
                    (IMPOSSIBLE, 0)
                } else if row == 0 {
                    (SCORE_MATCH + bonus * FIRST_CHARACTER_MULTIPLIER, bonus)
                } else if column == 0 {
                    (IMPOSSIBLE, 0)
                } else {
                    let run = previous_run[column - 1];
                    let consecutive = previous[column - 1] + bonus.max(run).max(BONUS_CONSECUTIVE);
                    let after_gap = gap + bonus;
                    if consecutive >= after_gap {
                        (SCORE_MATCH + consecutive, run)
                    } else {
                        (SCORE_MATCH + after_gap, bonus)
                    }
                };
            }
            std::mem::swap(&mut previous, &mut current);
            std::mem::swap(&mut previous_run, &mut current_run);
        }
        previous
            .into_iter()
            .filter(|score| *score > IMPOSSIBLE / 2)
            .max()
    }
}

/// Bonus of a match at every position of the candidate.
fn bonuses(text: &[char]) -> Vec<i32> {
    let mut previous = None;
    text.iter()
        .map(|&current| {
            let bonus = match previous {
                None => BONUS_START,
                Some(before) if is_delimiter(before) && !is_delimiter(current) => BONUS_BOUNDARY,
                Some(before)
                    if (before.is_lowercase() && current.is_uppercase())
                        || (!before.is_numeric() && current.is_numeric()) =>
                {
                    BONUS_CAMEL_CASE
                }
                Some(_) => 0,
            };
            previous = Some(current);
            bonus
        })
        .collect()
}

fn is_delimiter(c: char) -> bool {
    matches!(c, '_' | '-' | '.' | '/' | '\\' | ' ')
}

/// Simple case folding: the first character of the lower-case mapping.
fn fold(c: char) -> char {
    c.to_lowercase().next().unwrap_or(c)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rank(query: &str, candidate: &str) -> Option<u32> {
        FuzzyQuery::new(query, false).rank(candidate)
    }

    #[test]
    fn requires_characters_in_order() {
        assert!(rank("abc", "a_b_c").is_some());
        assert!(rank("abc", "cba").is_none());
        assert!(rank("abcd", "abc").is_none());
    }

    #[test]
    fn empty_query_matches_with_rank_zero() {
        let query = FuzzyQuery::new("", true);
        assert!(query.is_empty());
        assert_eq!(query.rank("anything"), Some(0));
    }

    #[test]
    fn prefers_word_boundaries() {
        assert!(rank("fb", "foo_bar") > rank("fb", "fabric"));
        assert!(rank("mr", "main-routes.ts") > rank("mr", "mirror.ts"));
    }

    #[test]
    fn prefers_camel_case_transitions() {
        assert!(rank("fb", "fooBar") > rank("fb", "foobar"));
    }

    #[test]
    fn prefers_consecutive_characters() {
        assert!(rank("main", "main.rs") > rank("main", "m_a_i_n.rs"));
    }

    #[test]
    fn prefers_match_at_start() {
        assert!(rank("lib", "lib.rs") > rank("lib", "xlib.rs"));
    }

    #[test]
    fn shorter_gaps_rank_higher() {
        assert!(rank("ab", "axb") > rank("ab", "axxxxb"));
    }

    #[test]
    fn case_sensitivity_follows_the_query_setting() {
        assert!(FuzzyQuery::new("readme", true).rank("README.md").is_none());
        assert!(FuzzyQuery::new("readme", false).rank("README.md").is_some());
        assert!(FuzzyQuery::new("README", false).rank("readme.md").is_some());
    }

    #[test]
    fn handles_non_ascii_names() {
        assert!(rank("übr", "Über.txt").is_some());
        assert!(rank("ubr", "über.txt").is_none());
    }

    #[test]
    fn best_alignment_is_chosen() {
        let direct = rank("abc", "abc").expect("matches");
        let with_decoy = rank("abc", "a__abc").expect("matches");
        assert!(direct > with_decoy);
        assert!(with_decoy > rank("abc", "axxabxc").expect("matches"));
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
