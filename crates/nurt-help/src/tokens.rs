// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-help::tokens
// Responsibility:   Token counting for generated descriptions.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Token counting for generated descriptions.
//!
//! The counter is a trait so that an exact tokenizer can replace the
//! estimate without changing the generator.

/// Counts the tokens of a text.
pub trait TokenCounter {
    /// Number of tokens of `text`.
    fn count(&self, text: &str) -> u32;

    /// Whether the count is an estimate of the tokenizer named by
    /// [`TokenCounter::tokenizer`].
    fn is_estimate(&self) -> bool;

    /// Name of the tokenizer the count refers to.
    fn tokenizer(&self) -> &str;
}

/// Characters of a run of letters and digits that make one token.
const RUN_CHARS_PER_TOKEN: usize = 6;

/// Punctuation characters that make one token.
const PUNCTUATION_PER_TOKEN: usize = 2;

/// Weight of a non-ASCII letter in a run: such letters split into more
/// tokens than ASCII ones.
const NON_ASCII_WEIGHT: usize = 2;

/// Name of the tokenizer the estimate approximates.
const ESTIMATED_TOKENIZER: &str = "o200k_base";

/// Approximate token counter.
///
/// A run of letters and digits counts one token per six characters, two
/// punctuation characters count one token and every line break counts one
/// token. On the English core description the estimate stays within 3% of
/// the count of the `o200k_base` tokenizer.
#[derive(Debug, Clone, Copy, Default)]
pub struct EstimateCounter;

impl TokenCounter for EstimateCounter {
    fn count(&self, text: &str) -> u32 {
        let mut tokens = 0usize;
        let mut punctuation = 0usize;
        let mut run = 0usize;
        for ch in text.chars() {
            if ch.is_alphanumeric() {
                run += if ch.is_ascii() { 1 } else { NON_ASCII_WEIGHT };
                continue;
            }
            tokens += run.div_ceil(RUN_CHARS_PER_TOKEN);
            run = 0;
            if ch == '\n' {
                tokens += 1;
            } else if !ch.is_whitespace() {
                punctuation += 1;
            }
        }
        tokens += run.div_ceil(RUN_CHARS_PER_TOKEN);
        tokens += punctuation.div_ceil(PUNCTUATION_PER_TOKEN);
        u32::try_from(tokens).unwrap_or(u32::MAX)
    }

    fn is_estimate(&self) -> bool {
        true
    }

    fn tokenizer(&self) -> &str {
        ESTIMATED_TOKENIZER
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_text_has_no_tokens() {
        assert_eq!(EstimateCounter.count(""), 0);
    }

    #[test]
    fn long_word_counts_one_token_per_six_characters() {
        assert_eq!(EstimateCounter.count("abcdefghijkl"), 2);
        assert_eq!(EstimateCounter.count("abcdefghijklm"), 3);
    }

    #[test]
    fn punctuation_and_line_breaks_are_counted() {
        assert_eq!(EstimateCounter.count("a |> b\nc"), 1 + 1 + 1 + 1 + 1);
    }

    #[test]
    fn non_ascii_letters_weigh_more_than_ascii_letters() {
        assert!(EstimateCounter.count("zażółć") > EstimateCounter.count("zazolc"));
    }

    #[test]
    fn estimate_is_marked_as_estimate() {
        assert!(EstimateCounter.is_estimate());
        assert_eq!(EstimateCounter.tokenizer(), "o200k_base");
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
