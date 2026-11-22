// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Detection of the character encoding of text and of binary content.
//!
//! Detection examines a sample from the start of the content, 64 KiB by
//! default, and applies the following rules in order:
//!
//! 1. A byte order mark selects UTF-8, `utf-16le` or `utf-16be`.
//! 2. Content that is valid UTF-8 and contains no NUL character is UTF-8.
//! 3. Content in which every second byte is zero for one character in ten
//!    or more is UTF-16 without a byte order mark.
//! 4. Content with a NUL byte is binary.
//! 5. The statistical detector `chardetng` proposes a legacy encoding. The
//!    configured legacy encodings, `windows-1250`, `ibm852`, `iso-8859-2` and
//!    `windows-1252` by default, override the proposal when one of them
//!    decodes the sample to Polish text and the proposal is either Polish
//!    text itself or implausible. Polish text has Polish letters, few other
//!    characters outside ASCII and no symbol between two letters; an
//!    implausible reading has control characters or symbols inside words.
//!    Among the Polish readings the one with the largest number of Polish
//!    letters wins, and earlier entries of the list win ties. This separates
//!    `ibm852` from `windows-1250`, which place the Polish letters at
//!    different byte values, and keeps a plausible proposal such as German
//!    text in `windows-1252`, whose letter U+00E4 reads as U+0144 in `ibm852`.
//! 6. Content whose decoded sample consists of more than one control
//!    character in twenty is binary.
//!
//! UTF-16 without a byte order mark is recognised only for text in which at
//! least one character in ten lies in the range U+0000 to U+00FF, such as
//! text in Latin script. Samples shorter than 200 bytes contain few letters
//! outside ASCII, so the choice between legacy encodings is less certain
//! for them.

use std::borrow::Cow;
use std::cmp::Reverse;
use std::num::NonZeroUsize;

use encoding_rs::{Encoding, UTF_8, UTF_16BE, UTF_16LE};
use nurt_contracts::services::TextEncoding;
use oem_cp::code_table::DECODING_TABLE_CP852;

/// Number of bytes examined when the options do not set another size.
pub const DEFAULT_SAMPLE_SIZE: usize = 64 * 1024;

/// Legacy encodings preferred when no other list is configured, in order of
/// preference.
pub const DEFAULT_LEGACY_ENCODINGS: [&str; 4] =
    ["windows-1250", "ibm852", "iso-8859-2", "windows-1252"];

const IBM852_LABEL: &str = "ibm852";
const IBM852_ALIASES: [&str; 3] = ["ibm852", "cp852", "852"];

/// Content with more than one disallowed control character per this many
/// characters is binary.
const CONTROL_CHARACTER_RATIO: usize = 20;

/// UTF-16 without a byte order mark requires that at least one code unit in
/// this many has a zero high byte.
const UTF16_MIN_ZERO_RATIO: usize = 10;

/// UTF-16 without a byte order mark allows no more than one code unit in
/// this many with a zero low byte.
const UTF16_MAX_OPPOSITE_ZERO_RATIO: usize = 50;

/// A candidate decodes to Polish text when Polish letters outnumber other
/// letters and symbols outside ASCII at least this many times.
const POLISH_DOMINANCE: usize = 4;

const UTF8_MAX_SEQUENCE_TAIL: usize = 3;
const HIGH_SURROGATES: std::ops::RangeInclusive<u16> = 0xD800..=0xDBFF;

const POLISH_LOWERCASE: [char; 9] = [
    '\u{105}', '\u{107}', '\u{119}', '\u{142}', '\u{144}', '\u{f3}', '\u{15b}', '\u{17a}',
    '\u{17c}',
];
const POLISH_UPPERCASE: [char; 9] = [
    '\u{104}', '\u{106}', '\u{118}', '\u{141}', '\u{143}', '\u{d3}', '\u{15a}', '\u{179}',
    '\u{17b}',
];

/// Punctuation outside ASCII that occurs in Polish text and is neutral for
/// the choice of an encoding.
const NEUTRAL_PUNCTUATION: [char; 16] = [
    '\u{a0}', '\u{a7}', '\u{ab}', '\u{b0}', '\u{bb}', '\u{2013}', '\u{2014}', '\u{2018}',
    '\u{2019}', '\u{201a}', '\u{201c}', '\u{201d}', '\u{201e}', '\u{2022}', '\u{2026}', '\u{20ac}',
];

/// Punctuation outside ASCII that occurs inside words, such as the right
/// single quotation mark U+2019 used as an apostrophe.
const WORD_PUNCTUATION: [char; 2] = ['\u{2018}', '\u{2019}'];

/// Legacy encoding that detection may choose.
///
/// The encoding is either `ibm852` or a single-byte or multi-byte encoding
/// of the WHATWG Encoding Standard other than UTF-8 and UTF-16.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyEncoding(LegacyKind);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LegacyKind {
    Ibm852,
    Whatwg(&'static Encoding),
}

impl LegacyEncoding {
    /// Parses an encoding label, compared without regard to case.
    ///
    /// Accepts `ibm852` and its aliases `cp852` and `852`, and every label of
    /// the WHATWG Encoding Standard except those of UTF-8, UTF-16 and the
    /// replacement encoding.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_text::detect::LegacyEncoding;
    ///
    /// assert_eq!(LegacyEncoding::from_label("CP852").map(LegacyEncoding::label).as_deref(), Some("ibm852"));
    /// assert_eq!(LegacyEncoding::from_label("latin2").map(LegacyEncoding::label).as_deref(), Some("iso-8859-2"));
    /// assert!(LegacyEncoding::from_label("utf-8").is_none());
    /// ```
    pub fn from_label(label: &str) -> Option<Self> {
        let label = label.trim();
        if IBM852_ALIASES
            .iter()
            .any(|alias| label.eq_ignore_ascii_case(alias))
        {
            return Some(Self(LegacyKind::Ibm852));
        }
        let encoding = Encoding::for_label_no_replacement(label.as_bytes())?;
        if encoding == UTF_8 || encoding == UTF_16LE || encoding == UTF_16BE {
            return None;
        }
        Some(Self(LegacyKind::Whatwg(encoding)))
    }

    /// Canonical label in lower case, for example `ibm852` or `windows-1250`.
    pub fn label(self) -> String {
        match self.0 {
            LegacyKind::Ibm852 => IBM852_LABEL.to_owned(),
            LegacyKind::Whatwg(encoding) => whatwg_label(encoding),
        }
    }

    fn decode(self, bytes: &[u8]) -> Cow<'_, str> {
        match self.0 {
            LegacyKind::Ibm852 => Cow::Owned(oem_cp::decode_string_complete_table(
                bytes,
                &DECODING_TABLE_CP852,
            )),
            LegacyKind::Whatwg(encoding) => encoding.decode_without_bom_handling(bytes).0,
        }
    }
}

/// Settings of [`detect`] and [`detect_prefix`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectOptions {
    legacy_encodings: Vec<LegacyEncoding>,
    sample_size: Option<NonZeroUsize>,
}

impl Default for DetectOptions {
    /// Prefers [`DEFAULT_LEGACY_ENCODINGS`] and examines
    /// [`DEFAULT_SAMPLE_SIZE`] bytes.
    fn default() -> Self {
        Self {
            legacy_encodings: DEFAULT_LEGACY_ENCODINGS
                .iter()
                .filter_map(|label| LegacyEncoding::from_label(label))
                .collect(),
            sample_size: NonZeroUsize::new(DEFAULT_SAMPLE_SIZE),
        }
    }
}

impl DetectOptions {
    /// Replaces the preferred legacy encodings, in order of preference.
    ///
    /// An empty list leaves the choice of a legacy encoding to `chardetng`
    /// alone.
    #[must_use]
    pub fn with_legacy_encodings(
        mut self,
        encodings: impl IntoIterator<Item = LegacyEncoding>,
    ) -> Self {
        self.legacy_encodings = encodings.into_iter().collect();
        self
    }

    /// Examines no more than `bytes` bytes from the start of the content.
    #[must_use]
    pub fn with_sample_size(mut self, bytes: NonZeroUsize) -> Self {
        self.sample_size = Some(bytes);
        self
    }

    /// Examines the whole content instead of a sample.
    #[must_use]
    pub fn with_full_scan(mut self) -> Self {
        self.sample_size = None;
        self
    }

    /// Preferred legacy encodings, in order of preference.
    pub fn legacy_encodings(&self) -> &[LegacyEncoding] {
        &self.legacy_encodings
    }

    /// Size of the examined sample; `None` when the whole content is
    /// examined.
    pub fn sample_size(&self) -> Option<NonZeroUsize> {
        self.sample_size
    }
}

/// Result of [`detect`] and [`detect_prefix`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Detection {
    /// Text in the given encoding.
    Text(TextEncoding),
    /// Content that is not text in any supported encoding.
    Binary,
}

impl Detection {
    /// Returns `true` for binary content.
    pub fn is_binary(&self) -> bool {
        matches!(self, Self::Binary)
    }

    /// Encoding of text; `None` for binary content.
    pub fn encoding(&self) -> Option<&TextEncoding> {
        match self {
            Self::Text(encoding) => Some(encoding),
            Self::Binary => None,
        }
    }

    /// Converts into the encoding of text; `None` for binary content.
    pub fn into_encoding(self) -> Option<TextEncoding> {
        match self {
            Self::Text(encoding) => Some(encoding),
            Self::Binary => None,
        }
    }
}

/// Detects whether complete content is text and in which encoding.
///
/// Only the sample selected by [`DetectOptions`] is examined; a multi-byte
/// sequence cut by the end of the sample is not an error.
///
/// # Examples
///
/// ```
/// use nurt_text::detect::{DetectOptions, detect};
///
/// let options = DetectOptions::default();
/// // Polish pangram with all nine Polish lowercase letters.
/// let text = detect("Za\u{17c}\u{f3}\u{142}\u{107} g\u{119}\u{15b}l\u{105} ja\u{17a}\u{144}\n".as_bytes(), &options);
/// assert_eq!(text.encoding().map(|encoding| encoding.label.as_str()), Some("utf-8"));
///
/// let cp1250 = detect(b"Zielona \xb3\xb9ka i \xbf\xf3\xb3ty \x9cwierszcz\n", &options);
/// assert_eq!(cp1250.encoding().map(|encoding| encoding.label.as_str()), Some("windows-1250"));
///
/// assert!(detect(b"\x7fELF\x02\x01\x01\x00\x00\x00", &options).is_binary());
/// ```
pub fn detect(bytes: &[u8], options: &DetectOptions) -> Detection {
    detect_sample(bytes, false, options)
}

/// Detects the encoding from the first bytes of longer content.
///
/// Behaves as [`detect`], but a multi-byte sequence cut at the end of
/// `prefix` is never an error, because the content continues.
///
/// # Examples
///
/// ```
/// use nurt_text::detect::{DetectOptions, detect_prefix};
///
/// // Polish word of four characters, three of them outside ASCII.
/// let text = "\u{142}\u{f3}d\u{17a}".as_bytes();
/// let cut = &text[..text.len() - 1];
/// let detection = detect_prefix(cut, &DetectOptions::default());
/// assert_eq!(detection.encoding().map(|encoding| encoding.label.as_str()), Some("utf-8"));
/// ```
pub fn detect_prefix(prefix: &[u8], options: &DetectOptions) -> Detection {
    detect_sample(prefix, true, options)
}

#[derive(Clone, Copy)]
struct Sample<'a> {
    bytes: &'a [u8],
    truncated: bool,
}

fn detect_sample(bytes: &[u8], is_prefix: bool, options: &DetectOptions) -> Detection {
    let size = options
        .sample_size
        .map_or(bytes.len(), |size| size.get().min(bytes.len()));
    let sample = Sample {
        bytes: &bytes[..size],
        truncated: is_prefix || size < bytes.len(),
    };
    detect_bom(sample)
        .or_else(|| detect_utf8(sample))
        .or_else(|| detect_utf16(sample))
        .unwrap_or_else(|| {
            if memchr::memchr(0, sample.bytes).is_some() {
                Detection::Binary
            } else {
                detect_legacy(sample, &options.legacy_encodings)
            }
        })
}

fn detect_bom(sample: Sample<'_>) -> Option<Detection> {
    let (encoding, bom_length) = Encoding::for_bom(sample.bytes)?;
    let body = Sample {
        bytes: &sample.bytes[bom_length..],
        ..sample
    };
    let textual = decode_strict(encoding, body).is_some_and(|text| is_textual(&text));
    Some(text_or_binary(textual, whatwg_label(encoding), true))
}

fn detect_utf8(sample: Sample<'_>) -> Option<Detection> {
    let text = decode_strict(UTF_8, sample)?;
    if text.contains('\0') {
        return None;
    }
    Some(text_or_binary(
        is_textual(&text),
        whatwg_label(UTF_8),
        false,
    ))
}

fn detect_utf16(sample: Sample<'_>) -> Option<Detection> {
    let encoding = utf16_byte_order(sample.bytes)?;
    let text = decode_strict(encoding, sample)?;
    is_textual(&text).then(|| {
        Detection::Text(TextEncoding {
            label: whatwg_label(encoding),
            bom: false,
        })
    })
}

fn detect_legacy(sample: Sample<'_>, preferred: &[LegacyEncoding]) -> Detection {
    let mut detector = chardetng::EncodingDetector::new(chardetng::Iso2022JpDetection::Deny);
    detector.feed(sample.bytes, !sample.truncated);
    let guess = LegacyEncoding(LegacyKind::Whatwg(
        detector.guess(None, chardetng::Utf8Detection::Deny),
    ));
    let guess_stats = LetterStats::of(&guess.decode(sample.bytes));
    let chosen = if guess_stats.is_plausible() && !guess_stats.is_polish() {
        guess
    } else {
        let minimum = if guess_stats.is_polish() {
            guess_stats.polish
        } else {
            1
        };
        preferred
            .iter()
            .enumerate()
            .filter_map(|(index, &candidate)| {
                let stats = LetterStats::of(&candidate.decode(sample.bytes));
                (stats.is_polish() && stats.polish >= minimum).then_some((
                    Reverse(stats.polish),
                    index,
                    candidate,
                ))
            })
            .min_by_key(|&(polish, index, _)| (polish, index))
            .map_or(guess, |(_, _, candidate)| candidate)
    };
    let textual = is_textual(&chosen.decode(sample.bytes));
    text_or_binary(textual, chosen.label(), false)
}

fn text_or_binary(textual: bool, label: String, bom: bool) -> Detection {
    if textual {
        Detection::Text(TextEncoding { label, bom })
    } else {
        Detection::Binary
    }
}

fn whatwg_label(encoding: &'static Encoding) -> String {
    encoding.name().to_ascii_lowercase()
}

/// Decodes UTF-8 or UTF-16 without replacing malformed sequences.
///
/// A sequence cut by the end of a truncated sample is dropped.
fn decode_strict<'a>(encoding: &'static Encoding, sample: Sample<'a>) -> Option<Cow<'a, str>> {
    if encoding == UTF_8 {
        return decode_utf8(sample).map(Cow::Borrowed);
    }
    let mut bytes = sample.bytes;
    if sample.truncated {
        bytes = &bytes[..bytes.len() & !1];
        if let Some(last) = bytes.len().checked_sub(2).map(|start| &bytes[start..]) {
            let unit = if encoding == UTF_16LE {
                u16::from_le_bytes([last[0], last[1]])
            } else {
                u16::from_be_bytes([last[0], last[1]])
            };
            if HIGH_SURROGATES.contains(&unit) {
                bytes = &bytes[..bytes.len() - 2];
            }
        }
    }
    encoding.decode_without_bom_handling_and_without_replacement(bytes)
}

fn decode_utf8(sample: Sample<'_>) -> Option<&str> {
    match std::str::from_utf8(sample.bytes) {
        Ok(text) => Some(text),
        Err(error)
            if sample.truncated
                && error.error_len().is_none()
                && sample.bytes.len() - error.valid_up_to() <= UTF8_MAX_SEQUENCE_TAIL =>
        {
            std::str::from_utf8(&sample.bytes[..error.valid_up_to()]).ok()
        }
        Err(_) => None,
    }
}

/// Recognises UTF-16 without a byte order mark by the zero bytes that the
/// characters U+0000 to U+00FF have in their high byte.
fn utf16_byte_order(bytes: &[u8]) -> Option<&'static Encoding> {
    let units = bytes.len() / 2;
    if units == 0 {
        return None;
    }
    let (mut zero_even, mut zero_odd) = (0, 0);
    for pair in bytes.as_chunks::<2>().0 {
        zero_even += usize::from(pair[0] == 0);
        zero_odd += usize::from(pair[1] == 0);
    }
    let matches = |high: usize, low: usize| {
        high * UTF16_MIN_ZERO_RATIO >= units && low * UTF16_MAX_OPPOSITE_ZERO_RATIO <= units
    };
    if matches(zero_odd, zero_even) {
        Some(UTF_16LE)
    } else if matches(zero_even, zero_odd) {
        Some(UTF_16BE)
    } else {
        None
    }
}

/// Returns `false` for text with a NUL character or with more than one
/// disallowed control character in [`CONTROL_CHARACTER_RATIO`] characters.
fn is_textual(text: &str) -> bool {
    let mut total = 0;
    let mut disallowed = 0;
    for ch in text.chars() {
        if ch == '\0' {
            return false;
        }
        total += 1;
        disallowed += usize::from(is_disallowed(ch));
    }
    disallowed * CONTROL_CHARACTER_RATIO <= total
}

/// Control characters that do not occur in text, and the replacement
/// character produced for malformed input.
///
/// Tab, line feed, vertical tab, form feed, carriage return, backspace and
/// escape occur in text files and terminal output.
fn is_disallowed(ch: char) -> bool {
    (ch.is_control()
        && !matches!(
            ch,
            '\t' | '\n' | '\u{b}' | '\u{c}' | '\r' | '\u{8}' | '\u{1b}'
        ))
        || ch == char::REPLACEMENT_CHARACTER
}

/// Counts of characters that indicate whether a decoded sample is Polish.
#[derive(Debug, Default, PartialEq, Eq)]
struct LetterStats {
    polish: usize,
    other: usize,
    symbols_in_words: usize,
    disallowed: usize,
}

impl LetterStats {
    /// An uppercase Polish letter directly after a lowercase letter counts as
    /// another letter, because Polish words do not change case inside. One
    /// or more symbols outside ASCII between two letters are a sign of a
    /// wrong encoding, because no language writes such symbols inside words.
    fn of(text: &str) -> Self {
        let mut stats = Self::default();
        let mut after_lowercase = false;
        let mut after_letter = false;
        let mut symbol_after_letter = false;
        for ch in text.chars() {
            let is_letter = ch.is_alphabetic();
            let is_other = !ch.is_ascii() && !NEUTRAL_PUNCTUATION.contains(&ch);
            if is_letter && symbol_after_letter {
                stats.symbols_in_words += 1;
            }
            let is_symbol = !ch.is_ascii() && !is_letter && !WORD_PUNCTUATION.contains(&ch);
            symbol_after_letter = (after_letter || symbol_after_letter) && is_symbol;
            if is_disallowed(ch) {
                stats.disallowed += 1;
            } else if POLISH_LOWERCASE.contains(&ch)
                || (POLISH_UPPERCASE.contains(&ch) && !after_lowercase)
            {
                stats.polish += 1;
            } else if is_other {
                stats.other += 1;
            }
            after_lowercase = ch.is_lowercase();
            after_letter = is_letter;
        }
        stats
    }

    fn is_plausible(&self) -> bool {
        self.disallowed == 0 && self.symbols_in_words == 0
    }

    fn is_polish(&self) -> bool {
        self.is_plausible() && self.polish > 0 && self.polish >= self.other * POLISH_DOMINANCE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn label(bytes: &[u8]) -> Option<String> {
        detect(bytes, &DetectOptions::default())
            .into_encoding()
            .map(|e| e.label)
    }

    #[test]
    fn empty_content_is_utf8() {
        assert_eq!(label(b""), Some("utf-8".to_owned()));
    }

    #[test]
    fn ascii_is_utf8_without_bom() {
        let detection = detect(b"fn main() {}\n", &DetectOptions::default());
        assert_eq!(detection, Detection::Text(TextEncoding::utf8()));
    }

    #[test]
    fn utf8_bom_is_reported() {
        let detection = detect(
            b"\xef\xbb\xbfza\xc5\xbc\xc3\xb3\xc5\x82\xc4\x87",
            &DetectOptions::default(),
        );
        let encoding = detection.into_encoding().unwrap();
        assert_eq!(encoding.label, "utf-8");
        assert!(encoding.bom);
    }

    #[test]
    fn utf16_boms_select_byte_order() {
        let le: Vec<u8> = [0xff, 0xfe]
            .into_iter()
            .chain("\u{107}ma".encode_utf16().flat_map(u16::to_le_bytes))
            .collect();
        let be: Vec<u8> = [0xfe, 0xff]
            .into_iter()
            .chain("\u{107}ma".encode_utf16().flat_map(u16::to_be_bytes))
            .collect();
        assert_eq!(label(&le), Some("utf-16le".to_owned()));
        assert_eq!(label(&be), Some("utf-16be".to_owned()));
    }

    #[test]
    fn bom_followed_by_binary_is_binary() {
        let mut bytes = vec![0xef, 0xbb, 0xbf];
        bytes.extend_from_slice(&[0, 1, 2, 3, 0, 0]);
        assert!(detect(&bytes, &DetectOptions::default()).is_binary());
    }

    #[test]
    fn utf16_without_bom_is_recognised() {
        // Polish pangram with all nine Polish lowercase letters.
        let text = "Pchn\u{105}\u{107} w t\u{119} \u{142}\u{f3}d\u{17a} je\u{17c}a lub o\u{15b}m skrzy\u{144} fig.";
        let le: Vec<u8> = text.encode_utf16().flat_map(u16::to_le_bytes).collect();
        let be: Vec<u8> = text.encode_utf16().flat_map(u16::to_be_bytes).collect();
        assert_eq!(label(&le), Some("utf-16le".to_owned()));
        assert_eq!(label(&be), Some("utf-16be".to_owned()));
    }

    #[test]
    fn odd_length_utf16_is_not_utf16_unless_truncated() {
        let mut bytes: Vec<u8> = "abc".encode_utf16().flat_map(u16::to_le_bytes).collect();
        bytes.push(b'd');
        assert!(detect(&bytes, &DetectOptions::default()).is_binary());
        let prefix = detect_prefix(&bytes, &DetectOptions::default());
        assert_eq!(prefix.encoding().unwrap().label, "utf-16le");
    }

    #[test]
    fn truncated_utf16_drops_cut_surrogate_pair() {
        let text = "ab\u{1f600}";
        let bytes: Vec<u8> = text.encode_utf16().flat_map(u16::to_le_bytes).collect();
        let cut = &bytes[..bytes.len() - 2];
        assert!(detect(cut, &DetectOptions::default()).is_binary());
        assert_eq!(
            detect_prefix(cut, &DetectOptions::default())
                .encoding()
                .unwrap()
                .label,
            "utf-16le"
        );
    }

    #[test]
    fn nul_byte_makes_content_binary() {
        assert!(detect(b"name=value\0\x01\x02more", &DetectOptions::default()).is_binary());
    }

    #[test]
    fn control_characters_make_valid_utf8_binary() {
        let bytes: Vec<u8> = (1u8..=26).cycle().take(400).collect();
        assert!(detect(&bytes, &DetectOptions::default()).is_binary());
    }

    #[test]
    fn terminal_escapes_and_form_feeds_stay_text() {
        assert_eq!(
            label(b"\x1b[1mbold\x1b[0m\x0cnext page\n"),
            Some("utf-8".to_owned())
        );
    }

    #[test]
    fn sample_size_limits_examined_bytes() {
        let mut bytes = "a".repeat(100).into_bytes();
        bytes.extend_from_slice(&[0, 0, 0, 0xff]);
        let options = DetectOptions::default().with_sample_size(NonZeroUsize::new(100).unwrap());
        assert_eq!(detect(&bytes, &options).encoding().unwrap().label, "utf-8");
        let full = DetectOptions::default().with_full_scan();
        assert!(detect(&bytes, &full).is_binary());
        assert_eq!(full.sample_size(), None);
    }

    #[test]
    fn sample_cut_inside_utf8_sequence_is_utf8() {
        let bytes = "\u{17c}\u{17c}\u{17c}\u{17c}".as_bytes();
        let options = DetectOptions::default().with_sample_size(NonZeroUsize::new(3).unwrap());
        assert_eq!(detect(bytes, &options).encoding().unwrap().label, "utf-8");
    }

    #[test]
    fn invalid_utf8_inside_sample_is_not_utf8() {
        assert_eq!(label(b"pi\xb3ka no\xbfna"), Some("windows-1250".to_owned()));
    }

    #[test]
    fn cp852_is_preferred_for_polish_letters_at_cp852_positions() {
        let bytes = oem_cp::encode_string_checked(
            "\u{179}r\u{f3}d\u{142}o: \u{17c}\u{f3}\u{142}w, g\u{119}\u{15b} i \u{142}\u{105}ka.",
            &oem_cp::code_table::ENCODING_TABLE_CP852,
        )
        .unwrap();
        assert_eq!(label(&bytes), Some("ibm852".to_owned()));
    }

    #[test]
    fn iso_8859_2_is_chosen_for_its_own_letters() {
        let (bytes, _, _) = encoding_rs::ISO_8859_2
            .encode("\u{15a}l\u{105} si\u{119} \u{17a}rebi\u{119}ta, \u{105}\u{119}.");
        assert_eq!(label(&bytes), Some("iso-8859-2".to_owned()));
    }

    #[test]
    fn empty_preference_list_keeps_statistical_guess() {
        let (bytes, _, _) =
            encoding_rs::WINDOWS_1252.encode("Gr\u{f6}\u{df}e und Wei\u{df}, sch\u{f6}n");
        let options = DetectOptions::default().with_legacy_encodings([]);
        assert_eq!(
            detect(&bytes, &options).encoding().unwrap().label,
            "windows-1252"
        );
    }

    #[test]
    fn german_with_a_umlaut_only_is_not_cp852() {
        let (bytes, _, _) = encoding_rs::WINDOWS_1252
            .encode("Das M\u{e4}dchen hatte einen B\u{e4}ren gesehen, aber die V\u{e4}ter nicht.");
        assert_ne!(label(&bytes), Some("ibm852".to_owned()));
    }

    #[test]
    fn legacy_labels_are_parsed_and_canonical() {
        assert_eq!(
            LegacyEncoding::from_label(" 852 ").unwrap().label(),
            "ibm852"
        );
        assert_eq!(
            LegacyEncoding::from_label("WINDOWS-1250").unwrap().label(),
            "windows-1250"
        );
        assert!(LegacyEncoding::from_label("utf-16le").is_none());
        assert!(LegacyEncoding::from_label("no-such-encoding").is_none());
        assert_eq!(
            DetectOptions::default()
                .legacy_encodings()
                .iter()
                .map(|encoding| encoding.label())
                .collect::<Vec<_>>(),
            DEFAULT_LEGACY_ENCODINGS
        );
    }

    #[test]
    fn letter_stats_penalise_uppercase_inside_word() {
        let stats = LetterStats::of("m\u{104}ka \u{141}\u{f3}d\u{17a}");
        assert_eq!(
            stats,
            LetterStats {
                polish: 3,
                other: 1,
                symbols_in_words: 0,
                disallowed: 0
            }
        );
    }

    #[test]
    fn letter_stats_count_symbols_inside_words() {
        let stats = LetterStats::of("g\u{b6}sto \u{b1} 2, ma\u{bf}e");
        assert_eq!(stats.symbols_in_words, 2);
        assert!(!stats.is_plausible());
    }

    #[test]
    fn detection_accessors_report_binary() {
        assert!(Detection::Binary.encoding().is_none());
        assert!(Detection::Binary.into_encoding().is_none());
        assert!(!Detection::Text(TextEncoding::utf8()).is_binary());
    }
}
