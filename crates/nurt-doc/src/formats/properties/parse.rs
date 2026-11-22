// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Lossless parser of property files.

use std::ops::Range;

use nurt_contracts::error::Error;

use super::escape::unescape;
use crate::formats::ini::support::errors::parse_error;
use crate::formats::ini::support::text::Line;

/// Name of the format in error messages.
const FORMAT: &str = "properties";

/// Key and value of one logical line.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Entry {
    pub(crate) key: String,
    pub(crate) value: String,
    pub(crate) key_range: Range<usize>,
    /// Raw value up to the end of the last physical line of the entry.
    pub(crate) value_range: Range<usize>,
    pub(crate) first_line: usize,
    pub(crate) last_line: usize,
    pub(crate) comment: Option<String>,
}

/// Parsed property file.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Parsed {
    pub(crate) lines: Vec<Line>,
    pub(crate) entries: Vec<Entry>,
    pub(crate) comment_lines: Vec<bool>,
}

/// Whitespace of property files.
pub(crate) fn is_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\u{c}')
}

/// Physical lines; `\n`, `\r\n` and a lone `\r` end a line.
fn split_lines(text: &str) -> Vec<Line> {
    let bytes = text.as_bytes();
    let mut lines = Vec::new();
    let mut start = 0;
    while start < bytes.len() {
        let rest = &bytes[start..];
        let line = match rest.iter().position(|&b| b == b'\n' || b == b'\r') {
            Some(offset) => {
                let content_end = start + offset;
                let width = if rest[offset] == b'\r' && rest.get(offset + 1) == Some(&b'\n') {
                    2
                } else {
                    1
                };
                Line {
                    start,
                    content_end,
                    end: content_end + width,
                }
            }
            None => Line {
                start,
                content_end: bytes.len(),
                end: bytes.len(),
            },
        };
        start = line.end;
        lines.push(line);
    }
    lines
}

/// Returns `true` when a physical line continues on the next one: it ends
/// with an odd number of backslashes.
pub(crate) fn continues(content: &str) -> bool {
    (content.len() - content.trim_end_matches('\\').len()) % 2 == 1
}

/// Parses a property file.
pub(crate) fn parse(text: &str) -> Result<Parsed, Error> {
    let lines = split_lines(text);
    let mut parsed = Parsed {
        comment_lines: vec![false; lines.len()],
        lines,
        entries: Vec::new(),
    };
    let mut comment: Vec<String> = Vec::new();
    let mut index = 0;
    while index < parsed.lines.len() {
        let line = parsed.lines[index];
        let content = &text[line.start..line.content_end];
        let body = content.trim_start_matches(is_space);
        match body.chars().next() {
            None => {
                comment.clear();
                index += 1;
            }
            Some(marker @ ('#' | '!')) => {
                parsed.comment_lines[index] = true;
                comment.push(body[marker.len_utf8()..].trim().to_owned());
                index += 1;
            }
            Some(_) => {
                let start = line.start + content.len() - body.len();
                let mut entry = logical_line(text, &parsed.lines, index, start)?;
                entry.comment = (!comment.is_empty()).then(|| comment.join("\n"));
                comment.clear();
                index = entry.last_line + 1;
                parsed.entries.push(entry);
            }
        }
    }
    Ok(parsed)
}

/// Characters of a logical line with their offsets, without continuation
/// backslashes, line breaks and the leading whitespace of continuation
/// lines.
fn logical_chars(text: &str, lines: &[Line], first: usize, start: usize) -> (Vec<(usize, char)>, usize) {
    let mut chars = Vec::new();
    let mut index = first;
    let mut from = start;
    loop {
        let line = lines[index];
        let content = &text[from..line.content_end];
        let more = continues(content) && index + 1 < lines.len();
        let kept = if continues(content) {
            &content[..content.len() - 1]
        } else {
            content
        };
        chars.extend(kept.char_indices().map(|(i, c)| (from + i, c)));
        if !more {
            return (chars, index);
        }
        index += 1;
        let next = lines[index];
        let next_content = &text[next.start..next.content_end];
        from = next.start + next_content.len() - next_content.trim_start_matches(is_space).len();
    }
}

fn logical_line(text: &str, lines: &[Line], first: usize, start: usize) -> Result<Entry, Error> {
    let (chars, last_line) = logical_chars(text, lines, first, start);
    let mut i = 0;
    while i < chars.len() {
        match chars[i].1 {
            '\\' => i += 2,
            c if c == '=' || c == ':' || is_space(c) => break,
            _ => i += 1,
        }
    }
    let key_end = i.min(chars.len());
    let mut j = key_end;
    while j < chars.len() && is_space(chars[j].1) {
        j += 1;
    }
    if j < chars.len() && matches!(chars[j].1, '=' | ':') {
        j += 1;
        while j < chars.len() && is_space(chars[j].1) {
            j += 1;
        }
    }
    let end_offset = lines[last_line].content_end;
    let offset_at = |k: usize| chars.get(k).map_or(end_offset, |&(o, _)| o);
    let raw_key: Vec<char> = chars[..key_end].iter().map(|&(_, c)| c).collect();
    let raw_value: Vec<char> = chars[j..].iter().map(|&(_, c)| c).collect();
    let key = unescape(&raw_key).map_err(|m| parse_error(FORMAT, m, text, start))?;
    let value = unescape(&raw_value).map_err(|m| parse_error(FORMAT, m, text, offset_at(j)))?;
    let key_end_offset = chars
        .get(key_end.saturating_sub(1))
        .filter(|_| key_end > 0)
        .map_or(start, |&(o, c)| o + c.len_utf8());
    Ok(Entry {
        key,
        value,
        key_range: start..key_end_offset,
        value_range: offset_at(j)..end_offset,
        first_line: first,
        last_line,
        comment: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pairs(text: &str) -> Vec<(String, String)> {
        parse(text)
            .unwrap()
            .entries
            .into_iter()
            .map(|e| (e.key, e.value))
            .collect()
    }

    #[test]
    fn reads_the_separators_of_java_properties() {
        let text = "a=1\nb : 2\nc 3\n  d\t=\t4\ne\nf=\n";
        let expected = [("a", "1"), ("b", "2"), ("c", "3"), ("d", "4"), ("e", ""), ("f", "")];
        let found = pairs(text);
        assert_eq!(found.len(), expected.len());
        for ((key, value), (k, v)) in found.iter().zip(expected) {
            assert_eq!((key.as_str(), value.as_str()), (k, v));
        }
    }

    #[test]
    fn joins_continuation_lines_and_skips_their_indentation() {
        let text = "list = a,\\\n       b,\\\r\n   c\nnext=\\\\\n";
        let parsed = parse(text).unwrap();
        assert_eq!(parsed.entries[0].value, "a,b,c");
        assert_eq!(parsed.entries[0].last_line, 2);
        assert_eq!(&text[parsed.entries[0].value_range.clone()], "a,\\\n       b,\\\r\n   c");
        assert_eq!(parsed.entries[1].value, "\\");
    }

    #[test]
    fn keeps_comments_and_escaped_keys() {
        let text = "# first\n! second\nkey\\ with\\:colon = v\\u00e9\r";
        let parsed = parse(text).unwrap();
        let entry = &parsed.entries[0];
        assert_eq!(entry.key, "key with:colon");
        assert_eq!(entry.value, "vé");
        assert_eq!(entry.comment.as_deref(), Some("first\nsecond"));
        assert_eq!(&text[entry.key_range.clone()], "key\\ with\\:colon");
        assert_eq!(parsed.lines.len(), 3);
    }

    #[test]
    fn comment_lines_do_not_continue() {
        let found = pairs("# note \\\nkey=v\n");
        assert_eq!(found, [("key".to_owned(), "v".to_owned())]);
    }

    #[test]
    fn reports_malformed_escapes_with_their_line() {
        let error = parse("ok=1\nbad=\\u12\n").unwrap_err();
        assert_eq!(error.location.as_ref().and_then(|l| l.line), Some(2));
    }
}
