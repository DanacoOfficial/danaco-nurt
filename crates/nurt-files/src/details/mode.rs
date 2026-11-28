// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-files::details::mode
// Responsibility:   Permission modes in octal and symbolic notation.
// Dependencies:     nurt_contracts, serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Permission modes in octal and symbolic notation.
//!
//! The octal form has one to four digits, as in `644` or `4755`. The
//! symbolic form follows `chmod`: comma-separated clauses of classes (`u`,
//! `g`, `o`, `a`), an operator (`+`, `-`, `=`) and permissions (`r`, `w`,
//! `x`, `X`, `s`, `t`), as in `u+x,go-w`. A clause without classes applies
//! to all classes; the process umask is not consulted. `X` grants execute
//! permission to directories and to files that are executable for some
//! class.

use nurt_contracts::error::{Error, ErrorCode, Result};
use serde_json::Value as Json;

/// Permission bits a mode sets, including the set-id and sticky bits.
pub(crate) const PERMISSION_BITS: u32 = 0o7777;

/// Largest number of octal digits.
const MAX_OCTAL_DIGITS: usize = 4;

/// Radix of the octal form.
const OCTAL_RADIX: u32 = 8;

/// Execute permission of every class.
const ANY_EXECUTE: u32 = 0o111;

const SET_UID: u32 = 0o4000;
const SET_GID: u32 = 0o2000;
const STICKY: u32 = 0o1000;

/// Shift of the permission triple of each class.
const USER_SHIFT: u32 = 6;
const GROUP_SHIFT: u32 = 3;
const OTHER_SHIFT: u32 = 0;

const READ: u32 = 4;
const WRITE: u32 = 2;
const EXECUTE: u32 = 1;
const TRIPLE: u32 = READ | WRITE | EXECUTE;

/// Classes of a clause.
const USER: u8 = 1;
const GROUP: u8 = 2;
const OTHER: u8 = 4;
const ALL_CLASSES: u8 = USER | GROUP | OTHER;

/// Separator of clauses.
const CLAUSE_SEPARATOR: char = ',';

/// Permission mode requested by the `mode` parameter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ModeSpec {
    /// Permission bits given in octal.
    Absolute(u32),
    /// Changes of the current permission bits.
    Symbolic(Vec<Clause>),
}

/// One symbolic clause: classes and the changes applied to them in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Clause {
    classes: u8,
    actions: Vec<Action>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Operator {
    Add,
    Remove,
    Set,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Action {
    operator: Operator,
    read: bool,
    write: bool,
    execute: bool,
    conditional_execute: bool,
    set_id: bool,
    sticky: bool,
}

impl Action {
    fn new(operator: Operator) -> Self {
        Self {
            operator,
            read: false,
            write: false,
            execute: false,
            conditional_execute: false,
            set_id: false,
            sticky: false,
        }
    }
}

impl ModeSpec {
    /// Parses the octal or the symbolic form.
    pub(crate) fn parse(text: &str) -> Result<Self> {
        let trimmed = text.trim();
        let octal = !trimmed.is_empty()
            && trimmed.len() <= MAX_OCTAL_DIGITS
            && trimmed.bytes().all(|byte| (b'0'..=b'7').contains(&byte));
        if octal {
            return u32::from_str_radix(trimmed, OCTAL_RADIX)
                .map(Self::Absolute)
                .map_err(|_| syntax_error(text));
        }
        if trimmed.is_empty() {
            return Err(syntax_error(text));
        }
        trimmed
            .split(CLAUSE_SEPARATOR)
            .map(|clause| parse_clause(clause).ok_or_else(|| syntax_error(text)))
            .collect::<Result<Vec<_>>>()
            .map(Self::Symbolic)
    }

    /// Permission bits after applying the mode to the current bits of an
    /// entry; `directory` decides the meaning of `X`.
    pub(crate) fn apply(&self, current: u32, directory: bool) -> u32 {
        match self {
            Self::Absolute(bits) => bits & PERMISSION_BITS,
            Self::Symbolic(clauses) => {
                clauses
                    .iter()
                    .fold(current & PERMISSION_BITS, |mode, clause| {
                        clause.actions.iter().fold(mode, |mode, action| {
                            apply_action(mode, clause.classes, *action, directory)
                        })
                    })
            }
        }
    }
}

fn parse_clause(text: &str) -> Option<Clause> {
    let mut classes = 0;
    let mut rest = text;
    while let Some(letter) = rest.chars().next() {
        let class = match letter {
            'u' => USER,
            'g' => GROUP,
            'o' => OTHER,
            'a' => ALL_CLASSES,
            _ => break,
        };
        classes |= class;
        rest = &rest[letter.len_utf8()..];
    }
    let mut actions = Vec::new();
    for letter in rest.chars() {
        let operator = match letter {
            '+' => Some(Operator::Add),
            '-' => Some(Operator::Remove),
            '=' => Some(Operator::Set),
            _ => None,
        };
        if let Some(operator) = operator {
            actions.push(Action::new(operator));
            continue;
        }
        let action = actions.last_mut()?;
        match letter {
            'r' => action.read = true,
            'w' => action.write = true,
            'x' => action.execute = true,
            'X' => action.conditional_execute = true,
            's' => action.set_id = true,
            't' => action.sticky = true,
            _ => return None,
        }
    }
    if actions.is_empty() {
        return None;
    }
    Some(Clause {
        classes: if classes == 0 { ALL_CLASSES } else { classes },
        actions,
    })
}

fn apply_action(mode: u32, classes: u8, action: Action, directory: bool) -> u32 {
    let executable = directory || mode & ANY_EXECUTE != 0;
    let mut triple = 0;
    if action.read {
        triple |= READ;
    }
    if action.write {
        triple |= WRITE;
    }
    if action.execute || (action.conditional_execute && executable) {
        triple |= EXECUTE;
    }
    let mut bits = 0;
    let mut affected = 0;
    for (class, shift, set_id) in [
        (USER, USER_SHIFT, SET_UID),
        (GROUP, GROUP_SHIFT, SET_GID),
        (OTHER, OTHER_SHIFT, 0),
    ] {
        if classes & class == 0 {
            continue;
        }
        bits |= triple << shift;
        affected |= TRIPLE << shift | set_id;
        if action.set_id {
            bits |= set_id;
        }
    }
    if classes & OTHER != 0 {
        affected |= STICKY;
        if action.sticky {
            bits |= STICKY;
        }
    }
    match action.operator {
        Operator::Add => mode | bits,
        Operator::Remove => mode & !bits,
        Operator::Set => (mode & !affected) | bits,
    }
}

fn syntax_error(text: &str) -> Error {
    Error::new(
        ErrorCode::InvalidValue,
        format!(
            "`{text}` is not a mode: give octal digits such as \"0644\" or changes such as \"u+x,go-w\""
        ),
    )
    .with_found(text)
    .with_expected(Json::from("octal digits or symbolic changes"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply(spec: &str, current: u32, directory: bool) -> u32 {
        ModeSpec::parse(spec).unwrap().apply(current, directory)
    }

    #[test]
    fn octal_modes_replace_the_bits() {
        assert_eq!(ModeSpec::parse("0644").unwrap(), ModeSpec::Absolute(0o644));
        assert_eq!(apply("755", 0o100_600, false), 0o755);
        assert_eq!(apply("4755", 0o644, false), 0o4755);
        assert_eq!(apply("0", 0o777, true), 0);
    }

    #[test]
    fn symbolic_clauses_change_the_current_bits() {
        let cases = [
            ("u+x", 0o644, false, 0o744),
            ("go-w", 0o666, false, 0o644),
            ("a=r", 0o755, false, 0o444),
            ("+x", 0o644, false, 0o755),
            ("u=rw,go=", 0o777, false, 0o600),
            ("o-rwx,g+w", 0o755, false, 0o770),
            ("u+s,g+s", 0o755, false, 0o6755),
            ("+t", 0o777, true, 0o1777),
            ("u+t", 0o777, true, 0o777),
            ("o=", 0o1777, true, 0o770),
            ("u-x+w", 0o544, false, 0o644),
        ];
        for (spec, current, directory, expected) in cases {
            assert_eq!(apply(spec, current, directory), expected, "{spec}");
        }
    }

    #[test]
    fn conditional_execute_follows_kind_and_bits() {
        assert_eq!(apply("a+X", 0o644, true), 0o755);
        assert_eq!(apply("a+X", 0o644, false), 0o644);
        assert_eq!(apply("a+X", 0o744, false), 0o755);
    }

    #[test]
    fn malformed_modes_are_rejected() {
        for text in [
            "", "  ", "99", "07777x", "u", "u+q", "z+x", "u+x,", "12345", "rwx",
        ] {
            let error = ModeSpec::parse(text).unwrap_err();
            assert_eq!(error.code(), ErrorCode::InvalidValue, "{text}");
            assert_eq!(error.found.as_deref(), Some(text));
        }
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
