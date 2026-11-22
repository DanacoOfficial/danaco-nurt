// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Detection of the field separator and the quote character.
//!
//! The separator is chosen among `,`, `;`, tab and `|` by reading a sample
//! with the `csv` crate under each candidate: the candidate whose most common
//! field count covers the largest share of records wins, and among equal
//! shares the one with more fields. A sample where every candidate yields one
//! field per record keeps the default separator of the format.

use super::scan::{Dialect, scan};

/// Separators tried by detection, in order of preference on a tie.
const CANDIDATES: [u8; 4] = [b',', b'\t', b';', b'|'];

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
    Dialect { delimiter, quote: detect_quote(sample, delimiter) }
}

/// Start of `text` cut after the last line break within [`SAMPLE_BYTES`], so
/// that the last sampled record is complete when possible.
fn sample(text: &[u8]) -> &[u8] {
    if text.len() <= SAMPLE_BYTES {
        return text;
    }
    let head = &text[..SAMPLE_BYTES];
    head.iter().rposition(|&byte| byte == b'\n').map_or(head, |end| &head[..=end])
}

fn detect_delimiter(sample: &[u8], default_delimiter: u8) -> u8 {
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

fn detect_quote(sample: &[u8], delimiter: u8) -> u8 {
    if sample.contains(&DOUBLE_QUOTE) || !sample.contains(&SINGLE_QUOTE) {
        return DOUBLE_QUOTE;
    }
    let dialect = Dialect { delimiter, quote: SINGLE_QUOTE };
    let records = scan(sample, dialect);
    let mut quoted = 0usize;
    for field in records.iter().flat_map(|record| &record.fields) {
        if field.irregular {
            return DOUBLE_QUOTE;
        }
        if field.quoted {
            quoted += 1;
        }
    }
    if quoted > 0 { SINGLE_QUOTE } else { DOUBLE_QUOTE }
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
        assert_eq!(detect(b"it's,fine\nyes,no\n", None, b',').quote, DOUBLE_QUOTE);
        assert_eq!(detect(b"\"a\",'b'\n", None, b',').quote, DOUBLE_QUOTE);
    }
}
