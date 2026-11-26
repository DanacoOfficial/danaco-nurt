// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Lines of rendered text with optional styles, aligned tables and width
//! fitting.

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

const ELLIPSIS: char = '\u{2026}';
const COLUMN_GAP: &str = "  ";
const RESET: &str = "\u{1b}[0m";

/// Style of a whole line, written as an ANSI color when colors are on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Style {
    /// Success.
    Green,
    /// Attention.
    Yellow,
    /// Failure or alert.
    Red,
    /// Secondary information.
    Dim,
}

impl Style {
    fn code(self) -> &'static str {
        match self {
            Self::Green => "\u{1b}[32m",
            Self::Yellow => "\u{1b}[33m",
            Self::Red => "\u{1b}[31m",
            Self::Dim => "\u{1b}[2m",
        }
    }
}

/// A line of text and its style.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Line {
    text: String,
    style: Option<Style>,
}

/// A document built line by line.
#[derive(Debug, Default)]
pub(crate) struct Lines(Vec<Line>);

impl Lines {
    /// Creates an empty document.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Appends a line without a style.
    pub(crate) fn plain(&mut self, text: impl Into<String>) {
        self.0.push(Line {
            text: text.into(),
            style: None,
        });
    }

    /// Appends a styled line.
    pub(crate) fn styled(&mut self, text: impl Into<String>, style: Style) {
        self.0.push(Line {
            text: text.into(),
            style: Some(style),
        });
    }

    /// Appends the rows of a table with aligned columns, each row indented.
    ///
    /// A cell is written on one line: line breaks and control characters are
    /// written as escape sequences, so that one row stays one line.
    pub(crate) fn table(&mut self, indent: &str, rows: &[Vec<String>]) {
        let rows: Vec<Vec<String>> = rows
            .iter()
            .map(|row| row.iter().map(|cell| single_line(cell)).collect())
            .collect();
        let columns = rows.iter().map(Vec::len).max().unwrap_or(0);
        let mut widths = vec![0; columns];
        for row in &rows {
            for (index, cell) in row.iter().enumerate() {
                widths[index] = widths[index].max(cell.width());
            }
        }
        for row in &rows {
            let last = row.iter().rposition(|cell| !cell.is_empty()).unwrap_or(0);
            let mut text = String::from(indent);
            for (index, cell) in row.iter().enumerate().take(last + 1) {
                text.push_str(cell);
                if index < last {
                    text.push_str(&" ".repeat(widths[index] - cell.width()));
                    text.push_str(COLUMN_GAP);
                }
            }
            self.plain(text);
        }
    }

    /// Joins the lines, shortening each to `width` columns and applying
    /// colors when `color` is set.
    pub(crate) fn finish(self, width: Option<usize>, color: bool) -> String {
        let lines: Vec<String> = self
            .0
            .into_iter()
            .map(|line| {
                let text = match width {
                    Some(width) => fit(&line.text, width),
                    None => line.text,
                };
                match line.style {
                    Some(style) if color => format!("{}{text}{RESET}", style.code()),
                    _ => text,
                }
            })
            .collect();
        lines.join("\n")
    }
}

/// Writes a text on one line: line breaks, tabs and other control characters
/// become escape sequences such as `\n`.
pub(crate) fn single_line(text: &str) -> String {
    let mut line = String::with_capacity(text.len());
    for character in text.chars() {
        escape_control(character, &mut line);
    }
    line
}

/// Writes a line of file content with control characters shown as escape
/// sequences, except the tab, which indents source text. A carriage return
/// kept raw would move the cursor of a terminal to the start of the line and
/// overwrite what was printed before it.
pub(crate) fn visible_controls(text: &str) -> String {
    let mut line = String::with_capacity(text.len());
    for character in text.chars() {
        if character == '\t' {
            line.push(character);
        } else {
            escape_control(character, &mut line);
        }
    }
    line
}

fn escape_control(character: char, out: &mut String) {
    match character {
        '\n' => out.push_str("\\n"),
        '\r' => out.push_str("\\r"),
        '\t' => out.push_str("\\t"),
        other if other.is_control() => out.extend(other.escape_default()),
        other => out.push(other),
    }
}

/// Shortens a line to `width` display columns, marking the cut.
fn fit(text: &str, width: usize) -> String {
    if text.width() <= width || width == 0 {
        return text.to_owned();
    }
    let mut used = 0;
    let mut cut = String::new();
    for character in text.chars() {
        let advance = character.width().unwrap_or(0);
        if used + advance >= width {
            break;
        }
        used += advance;
        cut.push(character);
    }
    cut.push(ELLIPSIS);
    cut
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aligns_columns_and_leaves_the_last_unpadded() {
        let mut lines = Lines::new();
        lines.table(
            "  ",
            &[
                vec!["a.go".to_owned(), "3 repl".to_owned(), "x".to_owned()],
                vec!["long/b.go".to_owned(), "2 repl".to_owned(), String::new()],
            ],
        );
        assert_eq!(
            lines.finish(None, false),
            "  a.go       3 repl  x\n  long/b.go  2 repl"
        );
    }

    #[test]
    fn counts_display_width_of_wide_characters() {
        let mut lines = Lines::new();
        lines.table(
            "",
            &[
                vec!["\u{4e2d}\u{6587}".to_owned(), "x".to_owned()],
                vec!["ab".to_owned(), "y".to_owned()],
            ],
        );
        assert_eq!(lines.finish(None, false), "\u{4e2d}\u{6587}  x\nab    y");
    }

    #[test]
    fn writes_line_breaks_of_a_cell_as_escape_sequences() {
        let mut lines = Lines::new();
        lines.table(
            "",
            &[
                vec!["name".to_owned(), "text".to_owned()],
                vec!["a.txt".to_owned(), "one\ntwo\r\nthree\tx".to_owned()],
                vec!["b.txt".to_owned(), "bell\u{7}".to_owned()],
            ],
        );
        assert_eq!(
            lines.finish(None, false),
            "name   text\na.txt  one\\ntwo\\r\\nthree\\tx\nb.txt  bell\\u{7}"
        );
    }

    #[test]
    fn leaves_text_without_control_characters_unchanged() {
        assert_eq!(single_line(""), "");
        assert_eq!(
            single_line("caf\u{e9} \u{4e2d}\\path"),
            "caf\u{e9} \u{4e2d}\\path"
        );
    }

    #[test]
    fn shortens_long_lines_to_the_width() {
        let mut lines = Lines::new();
        lines.plain("0123456789");
        lines.plain("short");
        assert_eq!(lines.finish(Some(6), false), "01234\u{2026}\nshort");
    }

    #[test]
    fn colors_styled_lines_only_when_asked() {
        let mut lines = Lines::new();
        lines.styled("ok", Style::Green);
        lines.plain("plain");
        let plain = lines.finish(None, false);
        assert_eq!(plain, "ok\nplain");
        let mut lines = Lines::new();
        lines.styled("ok", Style::Green);
        lines.styled("bad", Style::Red);
        lines.styled("note", Style::Yellow);
        lines.styled("info", Style::Dim);
        assert_eq!(
            lines.finish(None, true),
            "\u{1b}[32mok\u{1b}[0m\n\u{1b}[31mbad\u{1b}[0m\n\u{1b}[33mnote\u{1b}[0m\n\u{1b}[2minfo\u{1b}[0m"
        );
    }
}
