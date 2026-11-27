// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-programs::text
// Responsibility:   Text folding, stop words, sentences, edit distance and decoding of description files.
// Dependencies:     unicode_normalization
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Text folding, stop words, sentences, edit distance and decoding of
//! description files.
//!
//! [`fold`] defines how every text is compared in search: lower case,
//! U+0142 (Latin small letter l with stroke) as `l`, compatibility
//! decomposition (NFKD) and no combining marks, so a query without
//! diacritics matches a text with them.

use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

/// Words left out of a query when other words remain: conjunctions and
/// prepositions of Polish and English.
pub const STOP_WORDS: &[&str] = &[
    "a", "i", "o", "w", "z", "ze", "we", "na", "do", "od", "po", "dla", "sie", "oraz", "the", "of",
    "to", "for", "and",
];

/// Further English prepositions left out outside the compatibility mode of
/// search.
pub const EXTRA_STOP_WORDS: &[&str] = &["in", "on", "with", "by", "from"];

/// Distance returned by [`edit_distance`] when the lengths differ by more
/// than [`MAX_LENGTH_DIFFERENCE`].
pub const FAR: usize = 99;

/// Largest difference of lengths for which [`edit_distance`] is computed.
pub const MAX_LENGTH_DIFFERENCE: usize = 3;

/// Shortest word that gets a prefix stem in fallback matching.
pub const STEM_MIN_CHARS: usize = 6;

/// Shortest prefix stem.
pub const STEM_MIN_PREFIX: usize = 5;

/// Characters cut from the end of a word to form its prefix stem.
pub const STEM_CUT: usize = 2;

/// Folds a text for comparison: lower case, U+0142 as `l`, NFKD, combining
/// marks removed.
///
/// # Examples
///
/// ```
/// use nurt_programs::text::fold;
///
/// assert_eq!(fold("D\u{17a}wi\u{119}k \u{141}\u{104}CZE"), "dzwiek lacze");
/// ```
#[must_use]
pub fn fold(text: &str) -> String {
    text.to_lowercase()
        .replace('\u{142}', "l")
        .nfkd()
        .filter(|c| !is_combining_mark(*c))
        .collect()
}

/// Words of a query after folding, without repetitions, in order. Stop words
/// are left out when other words remain; `extended` adds
/// [`EXTRA_STOP_WORDS`].
///
/// # Examples
///
/// ```
/// use nurt_programs::text::query_words;
///
/// assert_eq!(query_words("konwersja do PDF", false), vec!["konwersja", "pdf"]);
/// assert_eq!(query_words("to do", false), vec!["to", "do"]);
/// ```
#[must_use]
pub fn query_words(query: &str, extended: bool) -> Vec<String> {
    let mut words: Vec<String> = Vec::new();
    for word in fold(query).split_whitespace() {
        if !words.iter().any(|known| known == word) {
            words.push(word.to_owned());
        }
    }
    let stop =
        |word: &str| STOP_WORDS.contains(&word) || (extended && EXTRA_STOP_WORDS.contains(&word));
    let meaningful: Vec<String> = words.iter().filter(|word| !stop(word)).cloned().collect();
    if meaningful.is_empty() {
        words
    } else {
        meaningful
    }
}

/// Text in one line, cut at a word boundary to at most `limit` characters
/// with `...` at the end when cut.
///
/// # Examples
///
/// ```
/// use nurt_programs::text::clip;
///
/// assert_eq!(clip("Converts  video\nfiles", 40), "Converts video files");
/// assert_eq!(clip("Converts video and audio files", 16), "Converts video\u{2026}");
/// ```
#[must_use]
pub fn clip(text: &str, limit: usize) -> String {
    let flat = flatten(text);
    let length = flat.chars().count();
    if length <= limit {
        return flat;
    }
    let keep = limit.saturating_sub(1).max(1);
    let cut: String = flat.chars().take(keep).collect();
    let space = cut.rfind(' ').map(|byte| cut[..byte].chars().count());
    match space {
        Some(chars) if chars >= limit / 2 => {
            let head: String = cut.chars().take(chars).collect();
            let trimmed = head.trim_end_matches([' ', ',', ';', ':', '\u{2014}', '-']);
            format!("{trimmed}\u{2026}")
        }
        _ => format!("{cut}\u{2026}"),
    }
}

/// Whitespace runs replaced by single spaces, ends trimmed.
#[must_use]
pub fn flatten(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// First sentence of a text: up to the first `.`, `!` or `?` followed by
/// whitespace.
///
/// # Examples
///
/// ```
/// use nurt_programs::text::first_sentence;
///
/// assert_eq!(first_sentence("Lists files. Sorts them."), "Lists files.");
/// assert_eq!(first_sentence("v1.2 parser"), "v1.2 parser");
/// ```
#[must_use]
pub fn first_sentence(text: &str) -> String {
    let flat = flatten(text);
    let mut previous: Option<char> = None;
    for (index, c) in flat.char_indices() {
        if c == ' ' && matches!(previous, Some('.' | '!' | '?')) {
            return flat[..index].to_owned();
        }
        previous = Some(c);
    }
    flat
}

/// Levenshtein distance of two texts in characters; [`FAR`] when the
/// lengths differ by more than [`MAX_LENGTH_DIFFERENCE`].
///
/// # Examples
///
/// ```
/// use nurt_programs::text::{edit_distance, FAR};
///
/// assert_eq!(edit_distance("ffmpeg", "ffmpg"), 1);
/// assert_eq!(edit_distance("a", "abcdef"), FAR);
/// ```
#[must_use]
pub fn edit_distance(left: &str, right: &str) -> usize {
    let left: Vec<char> = left.chars().collect();
    let right: Vec<char> = right.chars().collect();
    if left.len().abs_diff(right.len()) > MAX_LENGTH_DIFFERENCE {
        return FAR;
    }
    let mut previous: Vec<usize> = (0..=right.len()).collect();
    for (row, left_char) in left.iter().enumerate() {
        let mut current = Vec::with_capacity(right.len() + 1);
        current.push(row + 1);
        for (column, right_char) in right.iter().enumerate() {
            let change = previous[column] + usize::from(left_char != right_char);
            let value = (previous[column + 1] + 1)
                .min(current[column] + 1)
                .min(change);
            current.push(value);
        }
        previous = current;
    }
    previous[right.len()]
}

/// Largest edit distance at which a name counts as similar to a requested
/// name of `length` characters: a third of the length, at least one.
#[must_use]
pub fn similar_distance(length: usize) -> usize {
    (length / 3).max(1)
}

/// Prefix stem of a folded word for fallback matching: words of at least
/// [`STEM_MIN_CHARS`] characters lose [`STEM_CUT`] characters, keeping at
/// least [`STEM_MIN_PREFIX`]; shorter words have no stem.
///
/// # Examples
///
/// ```
/// use nurt_programs::text::stem;
///
/// assert_eq!(stem("archiwow").as_deref(), Some("archiw"));
/// assert_eq!(stem("video"), None);
/// ```
#[must_use]
pub fn stem(word: &str) -> Option<String> {
    let length = word.chars().count();
    if length < STEM_MIN_CHARS {
        return None;
    }
    let keep = length.saturating_sub(STEM_CUT).max(STEM_MIN_PREFIX);
    Some(word.chars().take(keep).collect())
}

/// Decodes the bytes of a text file: byte order marks of UTF-8 and UTF-16
/// are honoured, valid UTF-8 is taken as it is, anything else is decoded in
/// the encoding detected from the content.
///
/// # Examples
///
/// ```
/// use nurt_programs::text::decode;
///
/// assert_eq!(decode(b"plain"), "plain");
/// assert_eq!(decode(&[0xff, 0xfe, b'a', 0]), "a");
/// ```
#[must_use]
pub fn decode(bytes: &[u8]) -> String {
    if let Some((encoding, bom)) = encoding_rs::Encoding::for_bom(bytes) {
        let (text, _) = encoding.decode_without_bom_handling(&bytes[bom..]);
        return text.into_owned();
    }
    match std::str::from_utf8(bytes) {
        Ok(text) => return text.to_owned(),
        // Input cut inside a character, such as a prefix of a file: the part
        // before the cut is UTF-8 when it already holds non-ASCII text.
        Err(error) if error.error_len().is_none() => {
            let valid = &bytes[..error.valid_up_to()];
            if let Ok(text) = std::str::from_utf8(valid)
                && !text.is_ascii()
            {
                return text.to_owned();
            }
        }
        Err(_) => {}
    }
    let mut detector = chardetng::EncodingDetector::new(chardetng::Iso2022JpDetection::Deny);
    detector.feed(bytes, true);
    let encoding = detector.guess(None, chardetng::Utf8Detection::Allow);
    let (text, _, _) = encoding.decode(bytes);
    text.into_owned()
}

/// Removes control characters other than line feed and tab.
#[must_use]
pub fn strip_controls(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_control() || *c == '\n' || *c == '\t')
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fold_removes_marks_and_slashed_l() {
        assert_eq!(
            fold("Za\u{17c}\u{f3}\u{142}\u{107} g\u{119}\u{15b}l\u{105} ja\u{17a}\u{144}"),
            "zazolc gesla jazn"
        );
        assert_eq!(fold("\u{fb01}le"), "file");
    }

    #[test]
    fn query_words_drop_stop_words_and_repetitions() {
        assert_eq!(
            query_words("PDF i pdf oraz Word", false),
            vec!["pdf", "word"]
        );
        assert_eq!(
            query_words("search in files", true),
            vec!["search", "files"]
        );
        assert_eq!(
            query_words("search in files", false),
            vec!["search", "in", "files"]
        );
        assert!(query_words("   ", false).is_empty());
    }

    #[test]
    fn clip_cuts_short_text_without_space() {
        assert_eq!(clip("abcdefghij", 5), "abcd\u{2026}");
        assert_eq!(clip("ab cdefghij", 6), "ab cd\u{2026}");
        assert_eq!(clip("one two, three", 10), "one two\u{2026}");
    }

    #[test]
    fn first_sentence_handles_question_and_exclamation() {
        assert_eq!(first_sentence("Why? Because."), "Why?");
        assert_eq!(first_sentence("Stop! Now"), "Stop!");
        assert_eq!(first_sentence(""), "");
    }

    #[test]
    fn edit_distance_counts_characters() {
        assert_eq!(edit_distance("\u{17c}\u{f3}\u{142}w", "zolw"), 3);
        assert_eq!(edit_distance("", "abc"), 3);
        assert_eq!(edit_distance("kitten", "sitting"), 3);
        assert_eq!(similar_distance(2), 1);
        assert_eq!(similar_distance(9), 3);
    }

    #[test]
    fn stem_keeps_minimum_prefix() {
        assert_eq!(stem("kompresja").as_deref(), Some("kompres"));
        assert_eq!(stem("szyfry").as_deref(), Some("szyfr"));
    }

    #[test]
    fn decode_detects_legacy_encoding() {
        let latin2 = [b'z', 0xb3, b'o', b't', b'y', b' ', b'k', 0xb1, b't'];
        let text = decode(&latin2);
        assert!(text.starts_with('z'));
        assert_eq!(text.chars().count(), latin2.len());
        assert_eq!(decode(&[0xef, 0xbb, 0xbf, b'x']), "x");
        assert_eq!(decode(&[0xfe, 0xff, 0, b'y']), "y");
    }

    #[test]
    fn decode_keeps_utf8_text_cut_inside_a_character() {
        let text = "Za\u{17c}\u{f3}\u{142}\u{107} g\u{119}\u{15b}l\u{105}";
        let bytes = text.as_bytes();
        assert!(std::str::from_utf8(&bytes[..bytes.len() - 1]).is_err());
        assert_eq!(
            decode(&bytes[..bytes.len() - 1]),
            "Za\u{17c}\u{f3}\u{142}\u{107} g\u{119}\u{15b}l"
        );
    }

    #[test]
    fn decode_treats_a_lone_trailing_lead_byte_as_legacy_text() {
        let latin1 = [b'c', b'a', b'f', 0xe9];
        let text = decode(&latin1);
        assert!(text.starts_with("caf"));
        assert_eq!(text.chars().count(), latin1.len());
    }

    #[test]
    fn strip_controls_keeps_newline_and_tab() {
        assert_eq!(strip_controls("a\u{1}b\n\tc\r"), "ab\n\tc");
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
