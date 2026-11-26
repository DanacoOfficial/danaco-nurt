// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Recognition of statements written in PowerShell notation.
//!
//! A statement is PowerShell when it assigns a variable, starts with the
//! call operator `&`, starts with a variable or a string, or starts with a
//! cmdlet or alias of the translation table or a name of the form
//! `Verb-Noun` with an approved verb.
//!
//! An alias that is also a command of bash, cmd or Nurt (`ls`, `rm`, `cat`,
//! `cp`, `curl`, `ps`, `kill`, …) is PowerShell only when the statement shows
//! a feature that the other shells do not have: a parameter of the cmdlet
//! written with at least two letters (`-Recurse`, `-Path`), the pipeline
//! variable `$_`, a scoped variable `$env:NAME`, a member access, a script
//! block or hash table standing as its own argument, a type literal, a path
//! in Windows notation or a registry drive, or a later pipeline element that
//! is a cmdlet of its own. A dash and one letter (`-r`, `-l`) is a POSIX flag
//! and a plain `$name` is a shell variable, so `rm -rf /tmp/x` and
//! `ls $HOME` stay with the bash translation. A brace group glued to a word
//! (`cp a{,.bak}`) is a brace expansion of bash, not a script block.

use super::convert;
use super::lex::StrPart;
use super::syntax::{Command, Element, Pipeline, RawArg, Script, StatementKind, Value, ValueKind};
use super::table::{Cmdlet, PowerShellTable};

/// Shortest parameter name that marks a shared alias as PowerShell; a dash
/// followed by one letter is a POSIX flag.
const MIN_PARAMETER_LEN: usize = 2;

/// Drive of PowerShell that holds the environment variables, `env:`.
const ENVIRONMENT_DRIVE: &str = "env";

/// Whether any statement of the script is written in PowerShell notation.
pub(crate) fn is_habit(table: &PowerShellTable, script: &Script) -> bool {
    script
        .statements
        .iter()
        .any(|statement| match &statement.kind {
            StatementKind::Assign { .. } => true,
            StatementKind::Pipeline(pipeline) => pipeline_is_habit(table, pipeline),
        })
}

fn pipeline_is_habit(table: &PowerShellTable, pipeline: &Pipeline) -> bool {
    match pipeline.elements.first() {
        None => false,
        Some(Element::Expression(value)) => expression_is_habit(table, value),
        Some(Element::Command(command)) if command.call.is_some() => true,
        Some(Element::Command(command)) => match table.cmdlet(&command.name) {
            Some(cmdlet) if cmdlet.is_shared(&command.name) => {
                shows_powershell(table, cmdlet, command)
                    || pipeline
                        .elements
                        .iter()
                        .skip(1)
                        .any(|element| is_own_cmdlet(table, element))
            }
            Some(_) => true,
            None => table.is_cmdlet_form(&command.name),
        },
    }
}

/// A pipeline that starts with a value: `$x | …`, `"text"`, `$env:NAME`,
/// `(Get-Content a) -replace …`.
fn expression_is_habit(table: &PowerShellTable, value: &Value) -> bool {
    match &value.kind {
        ValueKind::Variable(_) | ValueKind::Literal(_) | ValueKind::Expandable(_) => true,
        ValueKind::Paren(script) => is_habit(table, script),
        ValueKind::Member { base, .. } => expression_is_habit(table, base),
        ValueKind::Binary { left, .. } => expression_is_habit(table, left),
        _ => false,
    }
}

/// A later pipeline element that is a cmdlet and not a shared alias.
fn is_own_cmdlet(table: &PowerShellTable, element: &Element) -> bool {
    match element {
        Element::Command(command) if command.call.is_none() => {
            table.cmdlet(&command.name).map_or_else(
                || table.is_cmdlet_form(&command.name),
                |cmdlet| !cmdlet.is_shared(&command.name),
            )
        }
        _ => false,
    }
}

/// Whether the arguments of a command show Nurt notation: a named argument,
/// `to: "x"`, or a raw text, `r"x"`. A shared alias written so is a command of
/// Nurt, whatever its other arguments show.
fn shows_nurt(table: &PowerShellTable, command: &Command) -> bool {
    command.args.iter().enumerate().any(|(index, arg)| {
        let RawArg::Value(Value {
            kind: ValueKind::Bare(text),
            end,
            ..
        }) = arg
        else {
            return false;
        };
        is_named_argument(table, text)
            || (is_raw_text_prefix(text)
                && matches!(
                    command.args.get(index + 1),
                    Some(RawArg::Value(next)) if next.start == *end
                ))
    })
}

/// A bare word of lower-case letters, digits and `_` with a colon, other
/// than a drive of PowerShell: `to:`, `into:`, `mode:`.
fn is_named_argument(table: &PowerShellTable, word: &str) -> bool {
    let Some(name) = word.strip_suffix(':') else {
        return false;
    };
    name.chars().count() >= MIN_PARAMETER_LEN
        && name.starts_with(|c: char| c.is_ascii_lowercase())
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        && table.registry.hive(name).is_none()
        && !name.eq_ignore_ascii_case(ENVIRONMENT_DRIVE)
}

/// `r` or `r#`, the start of a raw text that the tokenizer of PowerShell
/// splits from its quotation mark.
fn is_raw_text_prefix(word: &str) -> bool {
    word.strip_prefix('r')
        .is_some_and(|rest| rest.chars().all(|c| c == '#'))
}

/// Whether the arguments of a shared alias show PowerShell notation.
fn shows_powershell(table: &PowerShellTable, cmdlet: &Cmdlet, command: &Command) -> bool {
    if shows_nurt(table, command) {
        return false;
    }
    let mut previous_end = command.name_end;
    for arg in &command.args {
        let (found, end) = match arg {
            RawArg::Param { name, end, .. } => (is_parameter(table, cmdlet, name), *end),
            RawArg::Value(value) => (powershell_value(table, value, previous_end), value.end),
        };
        if found {
            return true;
        }
        previous_end = end;
    }
    false
}

/// A parameter of the cmdlet or a common parameter, written with at least
/// two letters: the full name, an alias or a unique prefix.
fn is_parameter(table: &PowerShellTable, cmdlet: &Cmdlet, written: &str) -> bool {
    if written.chars().count() < MIN_PARAMETER_LEN {
        return false;
    }
    let mut params = cmdlet.params.iter().chain(table.common.iter());
    if params.clone().any(|param| param.is_named(written)) {
        return true;
    }
    params
        .by_ref()
        .filter(|param| param.has_prefix(written))
        .count()
        == 1
}

fn powershell_value(table: &PowerShellTable, value: &Value, previous_end: usize) -> bool {
    match &value.kind {
        ValueKind::Variable(var) => var.scope.is_some() || var.is_pipeline_item(),
        ValueKind::Block(_) | ValueKind::Hash(_) => value.start != previous_end,
        ValueKind::Type(_) | ValueKind::Member { .. } => true,
        ValueKind::Expandable(parts) => parts.iter().any(|part| match part {
            StrPart::Variable(var) => var.scope.is_some(),
            StrPart::Text(text) => is_powershell_path(table, text),
            _ => false,
        }),
        ValueKind::Bare(text) | ValueKind::Literal(text) => is_powershell_path(table, text),
        ValueKind::List(items) => items
            .iter()
            .any(|item| powershell_value(table, item, item.start)),
        _ => false,
    }
}

/// A path in Windows notation or on a registry drive.
fn is_powershell_path(table: &PowerShellTable, text: &str) -> bool {
    convert::is_windows_path(text)
        || convert::path(text, &table.registry, None).is_ok_and(|path| path.registry)
}

#[cfg(test)]
mod tests {
    use super::super::lex::lex;
    use super::super::syntax::parse;
    use super::*;

    fn detected(input: &str) -> bool {
        let table = PowerShellTable::embedded().expect("embedded table");
        let tokens = lex(input).expect("tokens");
        let script = parse(&tokens).expect("script");
        is_habit(table, &script)
    }

    #[test]
    fn cmdlets_and_own_aliases_are_powershell() {
        assert!(detected("Get-ChildItem C:\\proj"));
        assert!(detected("gci"));
        assert!(detected("get-process"));
        assert!(detected("Get-Widget -Name x"));
        assert!(detected("? { $_.Length -gt 1 }"));
    }

    #[test]
    fn shared_aliases_need_powershell_features() {
        assert!(!detected("ls"));
        assert!(!detected("ls -la /srv"));
        assert!(!detected("rm -rf /tmp/x"));
        assert!(!detected("cat /etc/hosts"));
        assert!(!detected("echo $HOME"));
        assert!(!detected("cp a{,.bak}"));
        assert!(detected("ls -Recurse /srv"));
        assert!(detected("rm -Force /srv/x"));
        assert!(detected("cat C:\\logs\\a.log"));
        assert!(detected("dir HKLM:\\Software"));
        assert!(detected("echo $env:PATH"));
        assert!(detected("ls /srv | Sort-Object Length"));
        assert!(detected("ls /srv | ? { $_.Length -gt 1 }"));
    }

    #[test]
    fn shared_aliases_written_in_nurt_notation_are_not_powershell() {
        assert!(!detected(
            r#"move "C:\praca\t1\sub\b.go" to: "C:\praca\t1\sub\c.go""#
        ));
        assert!(!detected(r#"move r"C:\a\b.go" to: r"C:\a\c.go""#));
        assert!(!detected(
            r#"write "C:\praca\t1\m.txt" "new" mode: overwrite"#
        ));
        assert!(!detected(r#"write r"C:\a\m.txt" "x""#));
        assert!(!detected(r#"copy "C:\a\x" into: "C:\b""#));
        assert!(!detected(r##"write r#"C:\a\m.txt"# "x""##));
    }

    #[test]
    fn shared_aliases_with_powershell_paths_stay_powershell() {
        assert!(detected(r"move C:\a\b.go C:\a\c.go"));
        assert!(detected(r"write C:\a\m.txt"));
        assert!(detected(r"copy C:\a\x C:\b -Recurse"));
        assert!(detected(r"dir hklm: -Recurse"));
        assert!(detected(r"ls env: -Recurse"));
    }

    #[test]
    fn prefixes_must_be_unique() {
        assert!(detected("ls -Rec /srv"));
        assert!(!detected("ls -xyz /srv"));
    }

    #[test]
    fn assignments_calls_variables_and_strings_are_powershell() {
        assert!(detected("$files = gci C:\\a"));
        assert!(detected("& 'C:\\Tools\\x.exe' a"));
        assert!(detected("$env:PATH"));
        assert!(detected("'text'"));
        assert!(detected("(Get-Content C:\\a.txt) -replace 'a','b'"));
    }

    #[test]
    fn nurt_programs_and_other_commands_are_not_powershell() {
        assert!(!detected("find \"/srv/app\" glob: \"*.go\""));
        assert!(!detected("copy \"/a\" to: \"/b\""));
        assert!(!detected("sort size order: desc"));
        assert!(!detected("docker-compose up"));
        assert!(!detected("5"));
    }
}
