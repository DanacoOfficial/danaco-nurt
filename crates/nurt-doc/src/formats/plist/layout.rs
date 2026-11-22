// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Line and indentation facts of text property lists, used to place new
//! entries and to remove entries together with their line.

use std::ops::Range;

/// Indentation used when the document has no indented line.
pub(crate) const DEFAULT_INDENT: &str = "\t";

/// Start of the line that holds `offset`.
pub(crate) fn line_start(text: &str, offset: usize) -> usize {
    text[..offset].rfind('\n').map_or(0, |index| index + 1)
}

/// Spaces and tabs at the start of the line that holds `offset`.
pub(crate) fn indentation(text: &str, offset: usize) -> &str {
    let line = &text[line_start(text, offset)..offset];
    &line[..line.len() - line.trim_start_matches([' ', '\t']).len()]
}

/// Only spaces and tabs precede `offset` on its line.
pub(crate) fn starts_line(text: &str, offset: usize) -> bool {
    text[line_start(text, offset)..offset].bytes().all(is_blank)
}

/// Offset after the line break that ends the line of `offset`, when only
/// spaces and tabs follow `offset` on that line.
fn rest_of_line(text: &str, offset: usize) -> Option<usize> {
    for (index, byte) in text[offset..].bytes().enumerate() {
        match byte {
            b'\n' => return Some(offset + index + 1),
            b' ' | b'\t' | b'\r' => {}
            _ => return None,
        }
    }
    Some(text.len())
}

const fn is_blank(byte: u8) -> bool {
    byte == b' ' || byte == b'\t'
}

/// Line break of the document: `\r\n` when at least half of the line breaks
/// are `\r\n`, otherwise `\n`.
pub(crate) fn line_break(text: &str) -> &'static str {
    let crlf = text.matches("\r\n").count();
    let all = text.matches('\n').count();
    if crlf > 0 && crlf * 2 >= all {
        "\r\n"
    } else {
        "\n"
    }
}

/// Indentation unit of the document: a tab when any line is indented with
/// a tab, otherwise the smallest indentation by spaces, otherwise a tab.
pub(crate) fn indent_unit(text: &str) -> String {
    let mut spaces: Option<usize> = None;
    for line in text.lines() {
        if line.starts_with('\t') {
            return DEFAULT_INDENT.to_owned();
        }
        let width = line.len() - line.trim_start_matches(' ').len();
        if width > 0 && width < line.len() {
            spaces = Some(spaces.map_or(width, |current| current.min(width)));
        }
    }
    spaces.map_or_else(|| DEFAULT_INDENT.to_owned(), |width| " ".repeat(width))
}

/// Bytes to remove for a member written at `span`, where `next` is the start
/// of the following member.
///
/// A member on lines of its own is removed with those lines. A member
/// followed by white space and another member is removed up to that member.
/// Otherwise the white space before the member is removed with it.
pub(crate) fn removal(text: &str, span: &Range<usize>, next: Option<usize>) -> Range<usize> {
    if starts_line(text, span.start)
        && let Some(end) = rest_of_line(text, span.end)
    {
        return line_start(text, span.start)..end;
    }
    if let Some(next) = next
        && text[span.end..next]
            .bytes()
            .all(|byte| byte.is_ascii_whitespace())
    {
        return span.start..next;
    }
    let start = text[..span.start]
        .trim_end_matches(|c: char| c.is_ascii_whitespace())
        .len();
    start..span.end
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indentation_and_line_starts() {
        let text = "a\n\t\tb c\n";
        assert_eq!(line_start(text, 4), 2);
        assert_eq!(indentation(text, 4), "\t\t");
        assert!(starts_line(text, 4));
        assert!(!starts_line(text, 6));
    }

    #[test]
    fn detects_line_break_and_indent_unit() {
        assert_eq!(line_break("a\r\nb\r\n"), "\r\n");
        assert_eq!(line_break("a\nb\r\nc\n"), "\n");
        assert_eq!(indent_unit("{\n    a\n  b\n}"), "  ");
        assert_eq!(indent_unit("{\n\ta\n}"), "\t");
        assert_eq!(indent_unit("{ a }"), "\t");
    }

    #[test]
    fn removes_own_lines_inline_members_and_last_members() {
        let lines = "{\n\ta = 1;\n\tb = 2;\n}";
        assert_eq!(removal(lines, &(3..9), Some(11)), 2..10);
        let inline = "{ a = 1; b = 2; }";
        assert_eq!(removal(inline, &(2..8), Some(9)), 2..9);
        assert_eq!(removal(inline, &(9..15), None), 8..15);
        let commented = "{ a = 1; /* c */ b = 2; }";
        assert_eq!(removal(commented, &(2..8), Some(17)), 1..8);
    }
}
