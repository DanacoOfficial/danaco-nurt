// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-syntax::lex::foreign
// Responsibility:   Typographic quotation marks accepted in place of `"` by the correction.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

use super::error::Fault;
use super::scan::{self, Lexeme, Scanned, char_len_at};
use super::token::{ForeignKind, TokenKind};

/// Typographic quotation marks accepted in place of `"` by the correction.
const TYPOGRAPHIC_QUOTES: [char; 10] = [
    '\u{201E}', '\u{201C}', '\u{201D}', '\u{201A}', '\u{2018}', '\u{2019}', '\u{AB}', '\u{BB}',
    '\u{2039}', '\u{203A}',
];

/// Characters that end an unquoted path.
const PATH_DELIMITERS: [char; 11] = ['"', '\'', '`', ',', ';', '(', ')', '[', ']', '{', '}'];

/// Whether `character` is whitespace other than space, tab and line feed:
/// Unicode white space, a carriage return, zero-width characters and the
/// byte order mark.
pub(crate) fn is_extra_whitespace(character: char) -> bool {
    match character {
        ' ' | '\t' | '\n' => false,
        '\u{200B}'..='\u{200D}' | '\u{2060}' | '\u{FEFF}' | '\u{180E}' => true,
        other => other.is_whitespace(),
    }
}

/// Whether `character` is a typographic quotation mark.
pub(crate) fn is_typographic_quote(character: char) -> bool {
    TYPOGRAPHIC_QUOTES.contains(&character)
}

/// Whether `character` separates tokens like a space: any whitespace.
pub(crate) fn is_space(character: char) -> bool {
    matches!(character, ' ' | '\t' | '\n') || is_extra_whitespace(character)
}

/// End of the unquoted path that starts at `start`: the path ends before
/// whitespace, `|` or one of ``"'`,;()[]{}``.
pub(crate) fn bare_path_end(text: &str, start: usize) -> usize {
    let rest = text.get(start..).unwrap_or_default();
    let length = rest
        .char_indices()
        .find(|&(_, character)| {
            is_space(character) || character == '|' || PATH_DELIMITERS.contains(&character)
        })
        .map_or(rest.len(), |(offset, _)| offset);
    start + length
}

/// Scans a run of whitespace other than space, tab and line feed.
pub(crate) fn scan_whitespace(text: &str, start: usize) -> Scanned {
    let rest = text.get(start..).unwrap_or_default();
    let line_break = |offset: usize| {
        rest.get(offset..)
            .is_some_and(|tail| tail.starts_with("\r\n"))
    };
    let length = rest
        .char_indices()
        .find(|&(offset, character)| !is_extra_whitespace(character) || line_break(offset))
        .map_or(rest.len(), |(offset, _)| offset);
    let end = start + length.max(char_len_at(text, start));
    let found = text.get(start..end).unwrap_or_default();
    let (problem, message) = if found.chars().all(|character| character == '\r') {
        (
            "carriage_return",
            "carriage return outside a line break; end lines with \\n or \\r\\n",
        )
    } else {
        (
            "unicode_whitespace",
            "whitespace other than space and tab; separate tokens with spaces",
        )
    };
    let fault = Fault::lexical(start..end, problem, message, "syntax");
    let fault = if found.chars().any(char::is_whitespace) {
        fault.with_fix(" ")
    } else {
        fault.with_fix("")
    };
    Err(scan::reject_with(
        fault,
        Lexeme::new(end, TokenKind::Foreign(ForeignKind::Whitespace)),
    ))
}

/// Scans a text in single quotes `'…'` on one line.
pub(crate) fn scan_single_quoted(text: &str, start: usize) -> Scanned {
    let content_start = start + 1;
    let closing = text
        .get(content_start..)
        .unwrap_or_default()
        .char_indices()
        .take_while(|&(_, character)| character != '\n')
        .find(|&(_, character)| character == '\'');
    let Some((offset, _)) = closing else {
        return Err(lone_character(
            text,
            start,
            "single_quote",
            "`'` is not a quotation mark; write texts in double quotes \"…\"",
        ));
    };
    let content = content_start..content_start + offset;
    let value = text.get(content.clone()).unwrap_or_default();
    let fault = Fault::lexical(
        start..content.end + 1,
        "single_quote",
        "texts are written in double quotes \"…\", not '…'",
        "text",
    )
    .with_fix(scan::quote(value));
    Err(scan::reject_with(
        fault,
        Lexeme::new(
            content.end + 1,
            TokenKind::Foreign(ForeignKind::SingleQuoted { content }),
        ),
    ))
}

/// Scans a text in typographic quotes on one line; the closing mark is any
/// typographic quotation mark or `"`.
pub(crate) fn scan_typographic(text: &str, start: usize) -> Scanned {
    let content_start = start + char_len_at(text, start);
    let closing = text
        .get(content_start..)
        .unwrap_or_default()
        .char_indices()
        .take_while(|&(_, character)| character != '\n')
        .find(|&(_, character)| character == '"' || is_typographic_quote(character));
    let Some((offset, closing_mark)) = closing else {
        return Err(lone_character(
            text,
            start,
            "typographic_quote",
            "typographic quotation marks do not delimit texts; write \"…\"",
        ));
    };
    let content = content_start..content_start + offset;
    let end = content.end + closing_mark.len_utf8();
    let value = text.get(content.clone()).unwrap_or_default();
    let fault = Fault::lexical(
        start..end,
        "typographic_quote",
        "typographic quotation marks do not delimit texts; write \"…\"",
        "text",
    )
    .with_fix(scan::quote(value));
    Err(scan::reject_with(
        fault,
        Lexeme::new(
            end,
            TokenKind::Foreign(ForeignKind::TypographicQuoted { content }),
        ),
    ))
}

/// Rejects the single character at `start`; the tolerant lexer returns it as
/// [`ForeignKind::Other`].
pub(crate) fn lone_character(
    text: &str,
    start: usize,
    problem: &'static str,
    message: impl Into<String>,
) -> Box<scan::Rejected> {
    let end = start + char_len_at(text, start);
    scan::reject_with(
        Fault::lexical(start..end, problem, message, "syntax"),
        Lexeme::new(end, TokenKind::Foreign(ForeignKind::Other)),
    )
}

/// Identifier form of a word with upper-case letters: `caseSensitive` and
/// `CaseSensitive` become `case_sensitive`, `HTTPServer` becomes
/// `http_server`.
pub(crate) fn snake_case(word: &str) -> String {
    let characters: Vec<char> = word.chars().collect();
    let mut name = String::with_capacity(word.len() + 4);
    for (index, &character) in characters.iter().enumerate() {
        if character.is_ascii_uppercase() && index > 0 {
            let previous = characters.get(index - 1).copied().unwrap_or('_');
            let next = characters.get(index + 1).copied().unwrap_or('_');
            let boundary = previous.is_ascii_lowercase()
                || previous.is_ascii_digit()
                || (previous.is_ascii_uppercase() && next.is_ascii_lowercase());
            if boundary && !name.ends_with('_') {
                name.push('_');
            }
        }
        name.push(character.to_ascii_lowercase());
    }
    name
}

#[cfg(test)]
mod tests {
    use super::{bare_path_end, is_extra_whitespace, snake_case};

    #[test]
    fn converts_words_to_snake_case() {
        assert_eq!(snake_case("Find"), "find");
        assert_eq!(snake_case("caseSensitive"), "case_sensitive");
        assert_eq!(snake_case("HTTPServer"), "http_server");
        assert_eq!(snake_case("open_Files"), "open_files");
    }

    #[test]
    fn ends_bare_paths_at_delimiters() {
        assert_eq!(bare_path_end("/srv/app glob", 0), 8);
        assert_eq!(bare_path_end("C:\\x\\y)", 0), 6);
        assert_eq!(bare_path_end("/a|>b", 0), 2);
    }

    #[test]
    fn classifies_extra_whitespace() {
        assert!(is_extra_whitespace('\u{A0}'));
        assert!(is_extra_whitespace('\u{200B}'));
        assert!(is_extra_whitespace('\r'));
        assert!(!is_extra_whitespace(' '));
        assert!(!is_extra_whitespace('x'));
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
