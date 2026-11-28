// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-git::diff::words
// Responsibility:   Word differences in the porcelain form of `git diff --word-diff=porcelain`.
// Dependencies:     bstr, regex
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Word differences in the porcelain form of `git diff --word-diff=porcelain`.
//!
//! Within a hunk, each run of removed and added lines is split into words
//! and compared word by word. Words are runs of non-white-space bytes, or the
//! matches of the word pattern when one is configured; bytes between words
//! are not compared. The output lines start with ` ` (common text, taken from
//! the new side), `-` (removed text), `+` (added text) or are `~`, which
//! stands for a line feed of the compared text.

use bstr::BString;
use regex::bytes::Regex;

use super::hunk::{Hunk, LineKind};
use super::lines::{LineOptions, changes, is_space};

/// Line that stands for a line feed.
pub const NEWLINE_MARK: &str = "~";

/// A word: byte range in the compared text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Word {
    start: usize,
    end: usize,
}

/// Porcelain word difference lines of a hunk.
#[must_use]
pub fn word_lines(hunk: &Hunk, pattern: Option<&Regex>) -> Vec<BString> {
    let mut out = Vec::new();
    let mut minus = Vec::new();
    let mut plus = Vec::new();
    let mut last_kind = LineKind::Context;
    for line in &hunk.lines {
        match line.kind {
            LineKind::Removed => {
                minus.extend_from_slice(&line.text);
                minus.push(b'\n');
            }
            LineKind::Added => {
                plus.extend_from_slice(&line.text);
                plus.push(b'\n');
            }
            LineKind::NoNewline => match last_kind {
                LineKind::Removed => {
                    minus.pop();
                }
                LineKind::Added => {
                    plus.pop();
                }
                LineKind::Context => {
                    if out
                        .last()
                        .is_some_and(|line: &BString| line.as_slice() == NEWLINE_MARK.as_bytes())
                    {
                        out.pop();
                    }
                }
                LineKind::NoNewline => {}
            },
            LineKind::Context => {
                flush(&mut out, &minus, &plus, pattern);
                minus.clear();
                plus.clear();
                let mut text = Vec::with_capacity(line.text.len() + 1);
                text.push(b' ');
                text.extend_from_slice(&line.text);
                out.push(text.into());
                out.push(NEWLINE_MARK.into());
            }
        }
        if line.kind != LineKind::NoNewline {
            last_kind = line.kind;
        }
    }
    flush(&mut out, &minus, &plus, pattern);
    out
}

fn flush(out: &mut Vec<BString>, minus: &[u8], plus: &[u8], pattern: Option<&Regex>) {
    if minus.is_empty() && plus.is_empty() {
        return;
    }
    let minus_words = split_words(minus, pattern);
    let plus_words = split_words(plus, pattern);
    let minus_tokens: Vec<&[u8]> = minus_words
        .iter()
        .map(|word| &minus[word.start..word.end])
        .collect();
    let plus_tokens: Vec<&[u8]> = plus_words
        .iter()
        .map(|word| &plus[word.start..word.end])
        .collect();
    let options = LineOptions {
        indent_heuristic: false,
        ..LineOptions::default()
    };
    let mut current = 0;
    for change in changes(&minus_tokens, &plus_tokens, &options) {
        let (minus_begin, minus_end) = span(&minus_words, &change.old);
        let (plus_begin, plus_end) = span(&plus_words, &change.new);
        segment(out, b' ', &plus[current..plus_begin]);
        segment(out, b'-', &minus[minus_begin..minus_end]);
        segment(out, b'+', &plus[plus_begin..plus_end]);
        current = plus_end;
    }
    segment(out, b' ', &plus[current..]);
}

/// Byte span of a run of words; an empty run sits at the end of the word
/// before it.
fn span(words: &[Word], run: &std::ops::Range<usize>) -> (usize, usize) {
    if run.is_empty() {
        let at = run.start.checked_sub(1).map_or(0, |index| words[index].end);
        return (at, at);
    }
    (words[run.start].start, words[run.end - 1].end)
}

fn segment(out: &mut Vec<BString>, prefix: u8, text: &[u8]) {
    let mut pieces = text.split(|&byte| byte == b'\n').peekable();
    while let Some(piece) = pieces.next() {
        if !piece.is_empty() {
            let mut line = Vec::with_capacity(piece.len() + 1);
            line.push(prefix);
            line.extend_from_slice(piece);
            out.push(line.into());
        }
        if pieces.peek().is_some() {
            out.push(NEWLINE_MARK.into());
        }
    }
}

fn split_words(text: &[u8], pattern: Option<&Regex>) -> Vec<Word> {
    match pattern {
        Some(regex) => text
            .split(|&byte| byte == b'\n')
            .scan(0, |offset, line| {
                let base = *offset;
                *offset += line.len() + 1;
                Some((base, line))
            })
            .flat_map(|(base, line)| {
                regex
                    .find_iter(line)
                    .filter(|found| !found.is_empty())
                    .map(move |found| Word {
                        start: base + found.start(),
                        end: base + found.end(),
                    })
                    .collect::<Vec<_>>()
            })
            .collect(),
        None => {
            let mut words = Vec::new();
            let mut start = None;
            for (index, &byte) in text.iter().enumerate() {
                match (is_space(byte), start) {
                    (false, None) => start = Some(index),
                    (true, Some(begin)) => {
                        words.push(Word {
                            start: begin,
                            end: index,
                        });
                        start = None;
                    }
                    _ => {}
                }
            }
            if let Some(begin) = start {
                words.push(Word {
                    start: begin,
                    end: text.len(),
                });
            }
            words
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::text_hunks;

    fn porcelain(old: &str, new: &str, pattern: Option<&Regex>) -> Vec<String> {
        let hunks = text_hunks("w.txt", old.as_bytes(), new.as_bytes());
        word_lines(&hunks[0], pattern)
            .into_iter()
            .map(|line| line.to_string())
            .collect()
    }

    #[test]
    fn marks_replaced_and_added_words() {
        let lines = porcelain(
            "one two  three\nfour five\n",
            "one TWO  three\nfour five six\n",
            None,
        );
        assert_eq!(
            lines,
            [
                " one ",
                "-two",
                "+TWO",
                "   three",
                "~",
                " four five ",
                "+six",
                "~"
            ]
        );
    }

    #[test]
    fn context_lines_end_with_newline_mark() {
        let old = "alpha\nbeta gamma\nctx1\nctx2\ndelta\nlast\n";
        let new = "alpha\nbeta GAMMA\nctx1\nctx2\nnew line here\ndelta\nlast\n";
        let lines = porcelain(old, new, None);
        assert_eq!(
            lines,
            [
                " alpha",
                "~",
                " beta ",
                "-gamma",
                "+GAMMA",
                "~",
                " ctx1",
                "~",
                " ctx2",
                "~",
                "+new line here",
                "~",
                " delta",
                "~",
                " last",
                "~"
            ]
        );
    }

    #[test]
    fn removed_word_leaves_following_space_in_context() {
        let lines = porcelain("a b c\n", "a c\n", None);
        assert_eq!(lines, [" a", "-b", "  c", "~"]);
    }

    #[test]
    fn pattern_splits_inside_runs() {
        let regex = Regex::new("[a-z]+|[0-9]+|[^[:space:]]").unwrap();
        let lines = porcelain("x=1;\n", "x=10;\n", Some(&regex));
        assert_eq!(lines, [" x=", "-1", "+10", " ;", "~"]);
    }

    #[test]
    fn missing_final_line_feed_has_no_mark() {
        let lines = porcelain("a\nb", "a\nc", None);
        assert_eq!(lines, [" a", "~", "-b", "+c"]);
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
