// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-code::symbols::matching
// Responsibility:   Matching of symbol names for `code.symbols`.
// Dependencies:     nurt_contracts, regex
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Matching of symbol names for `code.symbols`.
//!
//! A [`NameMatcher`] ranks a name from 0 (no match) to 6 (exact match):
//!
//! | Rank | Condition | Modes |
//! |---|---|---|
//! | 6 | same text | all |
//! | 5 | same text regardless of case; for `regex`, equal to the pattern | all |
//! | 4 | starts with the query, regardless of case | `prefix`, `substring`, `fuzzy` |
//! | 3 | contains the query, regardless of case | `substring`, `fuzzy` |
//! | 2 | the words of the query start words of the name in order; any `regex` match | `fuzzy`, `regex` |
//! | 1 | the letters and digits of the query appear in order | `fuzzy` |
//!
//! Words are split at camelCase boundaries, at digits followed by capitals
//! and at every character that is not a letter or a digit, so `getUserById`,
//! `get_user_by_id` and `GetUserByID` share the words `get`, `user`, `by`.

use std::str::FromStr;

use nurt_contracts::error::{Error, ErrorCode, Result};
use regex::{Regex, RegexBuilder};

/// Rank of an exact match.
pub const RANK_EXACT: u8 = 6;

const RANK_CASELESS: u8 = 5;
const RANK_PREFIX: u8 = 4;
const RANK_SUBSTRING: u8 = 3;
const RANK_WORDS: u8 = 2;
const RANK_LETTERS: u8 = 1;

/// How a name is compared with the query.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MatchMode {
    /// Equal text, regardless of case.
    Exact,
    /// The name starts with the query.
    Prefix,
    /// The name contains the query.
    Substring,
    /// Prefix, substring, words or letters in order.
    Fuzzy,
    /// Regular expression.
    Regex,
}

impl FromStr for MatchMode {
    type Err = Error;

    fn from_str(text: &str) -> Result<Self> {
        Ok(match text {
            "exact" => Self::Exact,
            "prefix" => Self::Prefix,
            "substring" => Self::Substring,
            "fuzzy" => Self::Fuzzy,
            "regex" => Self::Regex,
            other => {
                return Err(Error::new(
                    ErrorCode::InvalidValue,
                    format!(
                        "unknown match mode `{other}`; use exact, prefix, substring, fuzzy or regex"
                    ),
                ));
            }
        })
    }
}

/// Ranking of names against one query.
#[derive(Debug, Clone)]
pub struct NameMatcher {
    query: String,
    lower: String,
    words: Vec<String>,
    compact: String,
    mode: MatchMode,
    regex: Option<Regex>,
}

impl NameMatcher {
    /// Creates a matcher of `query` in `mode`; in [`MatchMode::Regex`] the
    /// query is a regular expression.
    ///
    /// # Errors
    ///
    /// Fails with `E_REGEX` when the regular expression is not valid.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_code::symbols::matching::{MatchMode, NameMatcher};
    ///
    /// let matcher = NameMatcher::new("gUBI", MatchMode::Fuzzy)?;
    /// assert!(matcher.rank("getUserById") > 0);
    /// assert_eq!(matcher.rank("deleteAll"), 0);
    /// # Ok::<(), nurt_contracts::error::Error>(())
    /// ```
    pub fn new(query: &str, mode: MatchMode) -> Result<Self> {
        let regex = match mode {
            MatchMode::Regex => Some(compile(RegexBuilder::new(query), query)?),
            _ => None,
        };
        Ok(Self::with_regex(query, mode, regex))
    }

    /// Creates a matcher of a regular expression value of the language,
    /// with its flags.
    ///
    /// # Errors
    ///
    /// Fails with `E_REGEX` when the regular expression is not valid.
    pub fn from_regex(regex: &nurt_contracts::value::Regex) -> Result<Self> {
        let mut builder = RegexBuilder::new(&regex.pattern);
        builder
            .case_insensitive(regex.flags.case_insensitive)
            .multi_line(regex.flags.multi_line)
            .dot_matches_new_line(regex.flags.dot_matches_new_line)
            .ignore_whitespace(regex.flags.extended);
        let compiled = compile(builder, &regex.pattern)?;
        Ok(Self::with_regex(
            &regex.pattern,
            MatchMode::Regex,
            Some(compiled),
        ))
    }

    fn with_regex(query: &str, mode: MatchMode, regex: Option<Regex>) -> Self {
        let lower = query.to_lowercase();
        Self {
            query: query.to_owned(),
            compact: lower.chars().filter(|c| c.is_alphanumeric()).collect(),
            lower,
            words: name_words(query),
            mode,
            regex,
        }
    }

    /// The query as written.
    #[must_use]
    pub fn query(&self) -> &str {
        &self.query
    }

    /// The match mode.
    #[must_use]
    pub fn mode(&self) -> MatchMode {
        self.mode
    }

    /// Rank of `name`, from 0 (no match) to [`RANK_EXACT`].
    #[must_use]
    pub fn rank(&self, name: &str) -> u8 {
        if let Some(regex) = &self.regex {
            return match (regex.is_match(name), name == self.query) {
                (true, true) => RANK_CASELESS,
                (true, false) => RANK_WORDS,
                (false, _) => 0,
            };
        }
        if name == self.query {
            return RANK_EXACT;
        }
        let candidate = name.to_lowercase();
        let ranks = [
            (
                candidate == self.lower,
                RANK_CASELESS,
                Some(MatchMode::Exact),
            ),
            (
                candidate.starts_with(&self.lower),
                RANK_PREFIX,
                Some(MatchMode::Prefix),
            ),
            (
                candidate.contains(&self.lower),
                RANK_SUBSTRING,
                Some(MatchMode::Substring),
            ),
            (
                words_match(&self.words, &name_words(name)),
                RANK_WORDS,
                None,
            ),
        ];
        for (matched, rank, last_mode) in ranks {
            if matched {
                return rank;
            }
            if last_mode == Some(self.mode) {
                return 0;
            }
        }
        let letters: String = candidate.chars().filter(|c| c.is_alphanumeric()).collect();
        if !self.compact.is_empty() && is_subsequence(&self.compact, &letters) {
            RANK_LETTERS
        } else {
            0
        }
    }
}

fn compile(builder: RegexBuilder, pattern: &str) -> Result<Regex> {
    builder.build().map_err(|error| {
        Error::new(
            ErrorCode::Regex,
            format!("invalid regular expression `{pattern}`: {error}"),
        )
    })
}

/// Words of a name in lower case, split as described in the module
/// documentation.
///
/// ```
/// use nurt_code::symbols::matching::name_words;
///
/// assert_eq!(name_words("parseHTTPResponse2Fast"), ["parse", "http", "response2", "fast"]);
/// assert_eq!(name_words("user_id"), ["user", "id"]);
/// ```
#[must_use]
pub fn name_words(name: &str) -> Vec<String> {
    let chars: Vec<char> = name.chars().collect();
    let mut words = Vec::new();
    let mut current = String::new();
    for (index, &c) in chars.iter().enumerate() {
        if !c.is_alphanumeric() {
            flush(&mut words, &mut current);
            continue;
        }
        if !current.is_empty() && c.is_uppercase() {
            let previous = chars[index - 1];
            let next_lower = chars.get(index + 1).is_some_and(|n| n.is_lowercase());
            if previous.is_lowercase()
                || previous.is_ascii_digit()
                || (previous.is_uppercase() && next_lower)
            {
                flush(&mut words, &mut current);
            }
        }
        current.extend(c.to_lowercase());
    }
    flush(&mut words, &mut current);
    words
}

fn flush(words: &mut Vec<String>, current: &mut String) {
    if !current.is_empty() {
        words.push(std::mem::take(current));
    }
}

fn words_match(query: &[String], name: &[String]) -> bool {
    let mut position = 0;
    for word in query {
        while position < name.len() && !name[position].starts_with(word.as_str()) {
            position += 1;
        }
        if position >= name.len() {
            return false;
        }
        position += 1;
    }
    !query.is_empty()
}

fn is_subsequence(needle: &str, haystack: &str) -> bool {
    let mut wanted = needle.chars().peekable();
    for c in haystack.chars() {
        if wanted.peek() == Some(&c) {
            wanted.next();
        }
    }
    wanted.peek().is_none()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rank(query: &str, mode: MatchMode, name: &str) -> u8 {
        NameMatcher::new(query, mode).unwrap().rank(name)
    }

    #[test]
    fn exact_mode_ignores_case_and_rejects_prefixes() {
        assert_eq!(rank("Parse", MatchMode::Exact, "Parse"), RANK_EXACT);
        assert_eq!(rank("parse", MatchMode::Exact, "Parse"), RANK_CASELESS);
        assert_eq!(rank("parse", MatchMode::Exact, "parser"), 0);
    }

    #[test]
    fn prefix_and_substring_modes_stop_at_their_rank() {
        assert_eq!(rank("par", MatchMode::Prefix, "parser"), RANK_PREFIX);
        assert_eq!(rank("ars", MatchMode::Prefix, "parser"), 0);
        assert_eq!(rank("ars", MatchMode::Substring, "parser"), RANK_SUBSTRING);
        assert_eq!(rank("pr", MatchMode::Substring, "parser"), 0);
    }

    #[test]
    fn fuzzy_mode_matches_words_and_letters() {
        assert_eq!(rank("user id", MatchMode::Fuzzy, "getUserById"), RANK_WORDS);
        assert_eq!(
            rank("UsrId", MatchMode::Fuzzy, "get_user_by_id"),
            RANK_LETTERS
        );
        assert_eq!(rank("xyz", MatchMode::Fuzzy, "getUserById"), 0);
    }

    #[test]
    fn regex_mode_uses_the_pattern_and_its_flags() {
        assert_eq!(rank("^get", MatchMode::Regex, "getUser"), RANK_WORDS);
        assert_eq!(rank("^get", MatchMode::Regex, "setUser"), 0);
        let value = nurt_contracts::value::Regex::new("^GET", "i").unwrap();
        assert!(NameMatcher::from_regex(&value).unwrap().rank("getUser") > 0);
        let error = NameMatcher::new("(", MatchMode::Regex).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Regex);
    }

    #[test]
    fn match_mode_parses_catalog_values() {
        assert_eq!("fuzzy".parse::<MatchMode>().unwrap(), MatchMode::Fuzzy);
        assert_eq!(
            "loose".parse::<MatchMode>().unwrap_err().code(),
            ErrorCode::InvalidValue
        );
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
