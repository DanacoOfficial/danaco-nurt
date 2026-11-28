// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-syntax::lex::cursor
// Responsibility:   Line and column of a byte offset; both start at 1 and columns count Unicode scalar values.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

/// Line and column of a byte offset; both start at 1 and columns count
/// Unicode scalar values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Cursor {
    pub(crate) line: u32,
    pub(crate) column: u32,
}

impl Cursor {
    /// Position of the first character.
    pub(crate) const START: Self = Self { line: 1, column: 1 };

    /// Moves the cursor over `passed`. Only `\n` starts a line; a carriage
    /// return is an ordinary character of its line.
    pub(crate) fn advance(&mut self, passed: &str) {
        let bytes = passed.as_bytes();
        match bytes.iter().rposition(|&byte| byte == b'\n') {
            Some(last_break) => {
                let breaks = bytes.iter().filter(|&&byte| byte == b'\n').count();
                self.line = self.line.saturating_add(saturate(breaks));
                let last_line = bytes.get(last_break + 1..).unwrap_or_default();
                self.column = 1_u32.saturating_add(count_scalars(last_line));
            }
            None => self.column = self.column.saturating_add(count_scalars(bytes)),
        }
    }

    /// Number of Unicode scalar values of `text`.
    pub(crate) fn count(text: &str) -> u32 {
        count_scalars(text.as_bytes())
    }

    /// Cursor moved over `passed`, leaving `self` unchanged.
    pub(crate) fn after(self, passed: &str) -> Self {
        let mut moved = self;
        moved.advance(passed);
        moved
    }
}

fn count_scalars(bytes: &[u8]) -> u32 {
    if bytes.is_ascii() {
        return saturate(bytes.len());
    }
    saturate(bytes.iter().filter(|&&byte| !is_continuation(byte)).count())
}

const fn is_continuation(byte: u8) -> bool {
    byte & 0b1100_0000 == 0b1000_0000
}

fn saturate(count: usize) -> u32 {
    u32::try_from(count).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::Cursor;

    #[test]
    fn counts_scalar_values_and_lines() {
        let cursor = Cursor::START.after("a\u{1F600}e\u{301}");
        assert_eq!((cursor.line, cursor.column), (1, 5));
        let cursor = Cursor::START.after("ab\r\ncd");
        assert_eq!((cursor.line, cursor.column), (2, 3));
        let cursor = Cursor::START.after("a\rb");
        assert_eq!((cursor.line, cursor.column), (1, 4));
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
