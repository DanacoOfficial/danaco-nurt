// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-doc::formats::csv::dialect
// Responsibility:   Detection of the field separator and the quote character.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Detection of the field separator and the quote character.
//!
//! The separator is chosen among `,`, `;`, tab and `|` by reading a sample
//! with the `csv` crate under each candidate: the candidate whose most common
//! field count covers the largest share of records wins, and among equal
//! shares the one with more fields. The default separator of the format wins
//! when it splits every record into the same number of fields. A sample where every candidate yields one
//! field per record keeps the default separator of the format.

use super::scan::{Dialect, scan};

/// Separators tried by detection, in order of preference on a tie.
const CANDIDATES: [u8; 4] = *b",\t;|";

/// Double quote, the quote character of RFC 4180.
pub(crate) const DOUBLE_QUOTE: u8 = b'"';

/// Single quote, accepted as the quote character when the sample has no
/// double quote and quotes whole fields with it.
const SINGLE_QUOTE: u8 = b'\'';

/// Largest sample read for detection, in bytes.
const SAMPLE_BYTES: usize = 64 * 1024;

/// Largest number of records read for detection.
const SAMPLE_RECORDS: usize = 200;

/// Share of records, in thousandths, that agree on the field count.
type Consistency = usize;

const PER_MILLE: usize = 1000;

/// Detects the dialect of `text` (without a byte order mark).
///
/// `delimiter` is an explicit separator; `default_delimiter` is used when the
/// sample does not decide.
pub(crate) fn detect(text: &[u8], delimiter: Option<u8>, default_delimiter: u8) -> Dialect {
    let sample = sample(text);
    let delimiter = delimiter.unwrap_or_else(|| detect_delimiter(sample, default_delimiter));
    Dialect {
        delimiter,
        quote: detect_quote(sample, delimiter),
    }
}

/// Start of `text` cut after the last line break within [`SAMPLE_BYTES`], so
/// that the last sampled record is complete when possible.
fn sample(text: &[u8]) -> &[u8] {
    if text.len() <= SAMPLE_BYTES {
        return text;
    }
    let head = &text[..SAMPLE_BYTES];
    head.iter()
        .rposition(|&byte| byte == b'\n')
        .map_or(head, |end| &head[..=end])
}

fn detect_delimiter(sample: &[u8], default_delimiter: u8) -> u8 {
    if score(sample, default_delimiter).is_some_and(|(consistency, _)| consistency == PER_MILLE) {
        return default_delimiter;
    }
    let mut best: Option<(Consistency, usize, u8)> = None;
    for candidate in CANDIDATES {
        let Some((consistency, fields)) = score(sample, candidate) else {
            continue;
        };
        let better = best.is_none_or(|(best_consistency, best_fields, _)| {
            (consistency, fields) > (best_consistency, best_fields)
        });
        if better {
            best = Some((consistency, fields, candidate));
        }
    }
    best.map_or(default_delimiter, |(_, _, candidate)| candidate)
}

/// Returns `true` when `delimiter` splits every record of the start of
/// `text` into the same number of fields, at least two, and that start holds
/// at least two records.
pub(crate) fn uniform(text: &[u8], delimiter: u8) -> bool {
    let sample = sample(text);
    let lines = sample
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.trim_ascii().is_empty())
        .count();
    lines >= 2 && score(sample, delimiter).is_some_and(|(consistency, _)| consistency == PER_MILLE)
}

/// Share of records with the most common field count and that count, or
/// `None` when the candidate does not split any record.
fn score(sample: &[u8], delimiter: u8) -> Option<(Consistency, usize)> {
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .delimiter(delimiter)
        .from_reader(sample);
    let counts: Vec<usize> = reader
        .byte_records()
        .take(SAMPLE_RECORDS)
        .map_while(Result::ok)
        .map(|record| record.len())
        .collect();
    let (mode, frequency) = mode(&counts)?;
    if mode < 2 {
        return None;
    }
    Some((frequency * PER_MILLE / counts.len(), mode))
}

/// Most common value and its frequency; the larger value wins a tie.
fn mode(counts: &[usize]) -> Option<(usize, usize)> {
    let mut sorted = counts.to_vec();
    sorted.sort_unstable();
    let mut best: Option<(usize, usize)> = None;
    for run in sorted.chunk_by(|a, b| a == b) {
        let candidate = (run[0], run.len());
        if best.is_none_or(|(_, frequency)| candidate.1 >= frequency) {
            best = Some(candidate);
        }
    }
    best
}

/// Quote character of the sample: `'` when it quotes more fields than `"`
/// and every field quoted with it is closed properly, otherwise `"`.
fn detect_quote(sample: &[u8], delimiter: u8) -> u8 {
    if !sample.contains(&SINGLE_QUOTE) {
        return DOUBLE_QUOTE;
    }
    let single = quoted_fields(
        sample,
        Dialect {
            delimiter,
            quote: SINGLE_QUOTE,
        },
    );
    let double = quoted_fields(
        sample,
        Dialect {
            delimiter,
            quote: DOUBLE_QUOTE,
        },
    )
    .unwrap_or_default();
    match single {
        Some(single) if single > double => SINGLE_QUOTE,
        _ => DOUBLE_QUOTE,
    }
}

/// Number of fields quoted under `dialect`, or `None` when a quoted field is
/// irregular.
fn quoted_fields(sample: &[u8], dialect: Dialect) -> Option<usize> {
    let records = scan(sample, dialect);
    let mut quoted = 0usize;
    for field in records.iter().flat_map(|record| &record.fields) {
        if field.irregular {
            return None;
        }
        quoted += usize::from(field.quoted);
    }
    Some(quoted)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_semicolon_with_decimal_commas() {
        let text = b"name;price\napple;1,50\npear;2,25\n";
        assert_eq!(detect(text, None, b',').delimiter, b';');
    }

    #[test]
    fn detects_tab_and_pipe() {
        assert_eq!(detect(b"a\tb\tc\n1\t2\t3\n", None, b',').delimiter, b'\t');
        assert_eq!(detect(b"a|b\n1|2\n", None, b',').delimiter, b'|');
    }

    #[test]
    fn single_column_keeps_default_separator() {
        assert_eq!(detect(b"name\nalpha\nbeta\n", None, b'\t').delimiter, b'\t');
    }

    #[test]
    fn default_separator_wins_when_consistent() {
        let text = b"a\tb\n1,5\t2,5\n3,5\t4,5\n";
        assert_eq!(detect(text, None, b'\t').delimiter, b'\t');
        assert_eq!(detect(text, None, b',').delimiter, b'\t');
        assert!(uniform(text, b'\t'));
        assert!(!uniform(b"a\tb\n", b'\t'));
        assert!(!uniform(b"a,b\n1\n", b','));
    }

    #[test]
    fn explicit_separator_wins() {
        assert_eq!(detect(b"a,b;c\n", Some(b';'), b',').delimiter, b';');
    }

    #[test]
    fn quoted_separator_does_not_split() {
        let text = b"\"a;b\",c\n\"d;e\",f\n";
        assert_eq!(detect(text, None, b',').delimiter, b',');
    }

    #[test]
    fn single_quote_is_detected_only_without_double_quotes() {
        assert_eq!(detect(b"'a,b',c\n'd',e\n", None, b',').quote, SINGLE_QUOTE);
        assert_eq!(
            detect(b"it's,fine\nyes,no\n", None, b',').quote,
            DOUBLE_QUOTE
        );
        assert_eq!(detect(b"\"a\",'b'\n", None, b',').quote, DOUBLE_QUOTE);
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
