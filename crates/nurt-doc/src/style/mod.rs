// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Detection of indentation, quoting, separators and line endings.
//!
//! New content written into a document follows the style of the content
//! around it. [`StyleProbe`] reads that style from the text once: the
//! indentation unit, the dominant line ending, the byte order mark, the
//! final line break, and on request the spacing around a separator, the
//! preferred quote, trailing commas and alignment of values.
//!
//! ```
//! use nurt_doc::style::{Indent, LineEnding, StyleProbe};
//!
//! let probe = StyleProbe::new(b"{\r\n    \"a\": 1,\r\n    \"b\": [\r\n        2\r\n    ]\r\n}\r\n");
//! assert_eq!(probe.indent_unit(), Some(Indent::Spaces(4)));
//! assert_eq!(probe.line_ending(), LineEnding::CrLf);
//! assert!(probe.final_newline());
//! assert_eq!(probe.separator(b':').after, " ");
//! ```

use std::collections::HashMap;

use crate::model::Span;

/// Byte order mark of UTF-8.
pub const UTF8_BOM: &[u8] = b"\xEF\xBB\xBF";

/// Largest indentation step, in columns, taken as an indentation unit.
const MAX_INDENT_STEP: usize = 8;

/// Columns of a tab when tabs and spaces are compared.
const TAB_WIDTH: usize = 4;

/// Line ending of a text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum LineEnding {
    /// `\n`.
    #[default]
    Lf,
    /// `\r\n`.
    CrLf,
}

impl LineEnding {
    /// The bytes of the line ending.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Lf => "\n",
            Self::CrLf => "\r\n",
        }
    }
}

/// Unit of indentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Indent {
    /// The given number of spaces.
    Spaces(usize),
    /// One tab.
    Tab,
}

impl Indent {
    /// Text of `levels` units of indentation.
    ///
    /// ```
    /// use nurt_doc::style::Indent;
    ///
    /// assert_eq!(Indent::Spaces(2).repeat(2), "    ");
    /// assert_eq!(Indent::Tab.repeat(1), "\t");
    /// ```
    #[must_use]
    pub fn repeat(self, levels: usize) -> String {
        match self {
            Self::Spaces(width) => " ".repeat(width * levels),
            Self::Tab => "\t".repeat(levels),
        }
    }
}

/// Quote character preferred by a text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Quote {
    /// `"`.
    Double,
    /// `'`.
    Single,
}

impl Quote {
    /// The quote character.
    #[must_use]
    pub const fn as_char(self) -> char {
        match self {
            Self::Double => '"',
            Self::Single => '\'',
        }
    }
}

/// Blanks written before and after a separator such as `:` or `=`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct Spacing {
    /// Blanks before the separator.
    pub before: String,
    /// Blanks after the separator.
    pub after: String,
}

/// Style of a text, read once and queried by formats when they write new
/// content.
#[derive(Debug, Clone)]
pub struct StyleProbe<'t> {
    text: &'t [u8],
    line_ending: LineEnding,
    mixed: bool,
    final_newline: bool,
    bom: bool,
    indent: Option<Indent>,
}

impl<'t> StyleProbe<'t> {
    /// Reads the style of `text`.
    #[must_use]
    pub fn new(text: &'t [u8]) -> Self {
        let crlf = text.windows(2).filter(|pair| pair == b"\r\n").count();
        let lf = text.iter().filter(|byte| **byte == b'\n').count() - crlf;
        let line_ending = if crlf > lf {
            LineEnding::CrLf
        } else {
            LineEnding::Lf
        };
        Self {
            text,
            line_ending,
            mixed: crlf > 0 && lf > 0,
            final_newline: text.ends_with(b"\n"),
            bom: text.starts_with(UTF8_BOM),
            indent: detect_indent(text),
        }
    }

    /// Dominant line ending; `\n` for a text without line breaks.
    #[must_use]
    pub const fn line_ending(&self) -> LineEnding {
        self.line_ending
    }

    /// Returns `true` when the text has both `\n` and `\r\n` line endings.
    #[must_use]
    pub const fn mixed_line_endings(&self) -> bool {
        self.mixed
    }

    /// Returns `true` when the text ends with a line break.
    #[must_use]
    pub const fn final_newline(&self) -> bool {
        self.final_newline
    }

    /// Returns `true` when the text starts with the UTF-8 byte order mark.
    #[must_use]
    pub const fn bom(&self) -> bool {
        self.bom
    }

    /// Indentation unit: the most frequent increase of indentation between
    /// a line and the next more indented line; `None` for a text without
    /// indented lines.
    #[must_use]
    pub const fn indent_unit(&self) -> Option<Indent> {
        self.indent
    }

    /// Leading blanks of the line containing `offset`.
    ///
    /// ```
    /// use nurt_doc::style::StyleProbe;
    ///
    /// let text = b"a:\n  b: 1\n";
    /// assert_eq!(StyleProbe::new(text).indent_at(6), b"  ");
    /// ```
    #[must_use]
    pub fn indent_at(&self, offset: usize) -> &'t [u8] {
        let line = self.line_at(offset);
        let blanks = self.text[line.start..line.end]
            .iter()
            .take_while(|byte| matches!(byte, b' ' | b'\t'))
            .count();
        &self.text[line.start..line.start + blanks]
    }

    /// The line containing `offset`, without its line ending.
    #[must_use]
    pub fn line_at(&self, offset: usize) -> Span {
        let offset = offset.min(self.text.len());
        let start = self.text[..offset]
            .iter()
            .rposition(|byte| *byte == b'\n')
            .map_or(0, |at| at + 1);
        let end = self.text[offset..]
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(self.text.len(), |at| offset + at);
        let end = if end > start && self.text.get(end - 1) == Some(&b'\r') {
            end - 1
        } else {
            end
        };
        Span::new(start, end)
    }

    /// Most frequent blanks around the first `separator` of each line
    /// outside quotes; a single space after and none before when the text
    /// has no such separator.
    #[must_use]
    pub fn separator(&self, separator: u8) -> Spacing {
        let mut counts: HashMap<(&[u8], &[u8]), usize> = HashMap::new();
        for line in self.text.split(|byte| *byte == b'\n') {
            let Some(at) = unquoted_position(line, separator) else {
                continue;
            };
            let before = trailing_blanks(&line[..at]);
            let after = leading_blanks(&line[at + 1..]);
            let rest = &line[at + 1 + after.len()..];
            if rest.is_empty() || rest == b"\r" {
                continue;
            }
            *counts.entry((before, after)).or_default() += 1;
        }
        counts
            .into_iter()
            .max_by_key(|(spacing, count)| {
                (*count, std::cmp::Reverse(spacing.0.len() + spacing.1.len()))
            })
            .map_or_else(
                || Spacing {
                    before: String::new(),
                    after: " ".to_owned(),
                },
                |((before, after), _)| Spacing {
                    before: String::from_utf8_lossy(before).into_owned(),
                    after: String::from_utf8_lossy(after).into_owned(),
                },
            )
    }

    /// Quote that opens most quoted texts; `None` for a text without
    /// quotes.
    #[must_use]
    pub fn quote(&self) -> Option<Quote> {
        let mut double = 0usize;
        let mut single = 0usize;
        for line in self.text.split(|byte| *byte == b'\n') {
            match line.iter().find(|byte| matches!(byte, b'"' | b'\'')) {
                Some(b'"') => double += 1,
                Some(_) => single += 1,
                None => {}
            }
        }
        match (double, single) {
            (0, 0) => None,
            (double, single) if single > double => Some(Quote::Single),
            _ => Some(Quote::Double),
        }
    }

    /// Returns `true` when a comma is followed, after blanks and line
    /// breaks, by a closing `]` or `}` (JSONC, JSON5, TOML arrays).
    #[must_use]
    pub fn trailing_comma(&self) -> bool {
        self.text.iter().enumerate().any(|(at, byte)| {
            *byte == b','
                && self.text[at + 1..]
                    .iter()
                    .find(|next| !next.is_ascii_whitespace())
                    .is_some_and(|next| matches!(next, b']' | b'}'))
        })
    }

    /// Returns `true` when consecutive lines with keys of different lengths
    /// put `separator` or the value after it in the same column.
    ///
    /// ```
    /// use nurt_doc::style::StyleProbe;
    ///
    /// assert!(StyleProbe::new(b"a    = 1\nlong = 2\n").aligned(b'='));
    /// assert!(!StyleProbe::new(b"a = 1\nlong = 2\n").aligned(b'='));
    /// ```
    #[must_use]
    pub fn aligned(&self, separator: u8) -> bool {
        let mut previous: Option<(usize, usize, usize)> = None;
        for line in self.text.split(|byte| *byte == b'\n') {
            let Some(at) = unquoted_position(line, separator) else {
                previous = None;
                continue;
            };
            let indent = leading_blanks(line).len();
            let key_end = at - trailing_blanks(&line[..at]).len();
            let value = at + 1 + leading_blanks(&line[at + 1..]).len();
            if let Some((key, column, value_column)) = previous
                && key != key_end
                && (column == at || value_column == value)
                && indent < key_end
            {
                return true;
            }
            previous = Some((key_end, at, value));
        }
        false
    }
}

fn leading_blanks(bytes: &[u8]) -> &[u8] {
    let count = bytes
        .iter()
        .take_while(|byte| matches!(byte, b' ' | b'\t'))
        .count();
    &bytes[..count]
}

fn trailing_blanks(bytes: &[u8]) -> &[u8] {
    let count = bytes
        .iter()
        .rev()
        .take_while(|byte| matches!(byte, b' ' | b'\t'))
        .count();
    &bytes[bytes.len() - count..]
}

/// Position of the first `separator` of `line` outside single or double
/// quotes.
fn unquoted_position(line: &[u8], separator: u8) -> Option<usize> {
    let mut quote: Option<u8> = None;
    let mut escaped = false;
    for (at, byte) in line.iter().enumerate() {
        match quote {
            Some(open) => {
                if escaped {
                    escaped = false;
                } else if *byte == b'\\' && open == b'"' {
                    escaped = true;
                } else if *byte == open {
                    quote = None;
                }
            }
            None if *byte == separator => return Some(at),
            None if matches!(byte, b'"' | b'\'') => quote = Some(*byte),
            None => {}
        }
    }
    None
}

/// Most frequent positive step of indentation between consecutive
/// non-blank lines.
fn detect_indent(text: &[u8]) -> Option<Indent> {
    let mut steps: HashMap<usize, usize> = HashMap::new();
    let mut tabs = 0usize;
    let mut spaces = 0usize;
    let mut previous = 0usize;
    for line in text.split(|byte| *byte == b'\n') {
        let blanks = leading_blanks(line);
        if blanks.len() == line.len() || line == b"\r" {
            continue;
        }
        let width: usize = blanks
            .iter()
            .map(|byte| if *byte == b'\t' { TAB_WIDTH } else { 1 })
            .sum();
        match blanks.first() {
            Some(b'\t') => tabs += 1,
            Some(_) => spaces += 1,
            None => {}
        }
        if width > previous && width - previous <= MAX_INDENT_STEP {
            *steps.entry(width - previous).or_default() += 1;
        }
        previous = width;
    }
    if tabs > spaces {
        return Some(Indent::Tab);
    }
    steps
        .into_iter()
        .max_by_key(|(step, count)| (*count, std::cmp::Reverse(*step)))
        .map(|(step, _)| Indent::Spaces(step))
}
