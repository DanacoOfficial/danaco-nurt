// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Recognition of a statement written in bash notation.
//!
//! A statement is a bash habit when its first word belongs to the habit word
//! set and it shows at least one feature that exists only in the shell: a
//! dashed flag, a control or redirection operator other than a single pipe,
//! a parameter expansion or command substitution, text in single quotes, an
//! assignment before the command, an unquoted pattern, `{}`, or a first word
//! that is a shell
//! keyword or builtin without a Nurt meaning. Text with Nurt notation
//! (`|>` or a named argument `name:`) is never a habit, so
//! `find "/a" glob: "*.go"` stays a Nurt program while `find /a -name "*.go"`
//! is a habit.

use super::catalog::is_parameter_name;
use super::lex::{PosixToken, PosixTokenKind, Quoting, RedirectOp, Word, WordPart};
use super::table::BashTable;

pub(crate) fn is_habit(table: &BashTable, tokens: &[PosixToken]) -> bool {
    let Some(first) = first_command_word(tokens) else {
        return false;
    };
    if has_nurt_notation(tokens) {
        return false;
    }
    if !table.is_habit(&first.raw) {
        return starts_with_assignment(tokens);
    }
    table.is_shell_only(&first.raw) || has_shell_feature(tokens)
}

/// Whether the statement starts with a habit word and shows no Nurt
/// notation, whatever else it shows. The caller uses this for a first word
/// that has no meaning in Nurt (`touch`, `gzip`), where no features are needed
/// to tell that the statement is written for a shell.
pub(crate) fn is_habit_by_word(table: &BashTable, tokens: &[PosixToken]) -> bool {
    first_command_word(tokens)
        .is_some_and(|first| table.is_habit(&first.raw) && !has_nurt_notation(tokens))
}

/// An assignment before the program (`CI=1 npm test`) exists only in the
/// shell, so the program that follows it may be any program.
fn starts_with_assignment(tokens: &[PosixToken]) -> bool {
    tokens.iter().find_map(|token| match &token.kind {
        PosixTokenKind::Word(word) => Some(is_assignment(word)),
        _ => None,
    }) == Some(true)
}

/// First word that is not an assignment.
fn first_command_word(tokens: &[PosixToken]) -> Option<&Word> {
    tokens.iter().find_map(|token| match &token.kind {
        PosixTokenKind::Word(word) if !is_assignment(word) => Some(word),
        _ => None,
    })
}

fn is_assignment(word: &Word) -> bool {
    word.raw.split_once('=').is_some_and(|(name, _)| {
        name.chars()
            .next()
            .is_some_and(|c| c == '_' || c.is_ascii_alphabetic())
            && name.chars().all(|c| c == '_' || c.is_ascii_alphanumeric())
    })
}

fn has_nurt_notation(tokens: &[PosixToken]) -> bool {
    tokens.windows(2).any(|pair| {
        matches!(
            (&pair[0].kind, &pair[1].kind),
            (
                PosixTokenKind::Operator(super::lex::Operator::Pipe),
                PosixTokenKind::Redirect(redirect),
            ) if redirect.op == RedirectOp::Out
                && redirect.fd.is_none()
                && pair[0].span.end_column == pair[1].span.column
                && pair[0].span.line == pair[1].span.line
        )
    }) || tokens.iter().any(|token| match &token.kind {
        PosixTokenKind::Word(word) => is_named_argument(word),
        _ => false,
    })
}

/// A bare word `name:` where `name` is a parameter of some command. Another
/// word that ends in a colon, such as the `error:` of `grep -i error: /a`, is
/// text of the command.
fn is_named_argument(word: &Word) -> bool {
    word.is_bare()
        && word.raw.strip_suffix(':').is_some_and(|name| {
            name.chars().next().is_some_and(|c| c.is_ascii_lowercase())
                && name
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
                && is_parameter_name(name)
        })
}

fn has_shell_feature(tokens: &[PosixToken]) -> bool {
    let mut seen_command = false;
    for token in tokens {
        let feature = match &token.kind {
            PosixTokenKind::Operator(operator) => !matches!(
                operator,
                super::lex::Operator::Pipe
                    | super::lex::Operator::OpenParen
                    | super::lex::Operator::CloseParen
            ),
            PosixTokenKind::Redirect(_) => true,
            PosixTokenKind::Word(word) => {
                let feature = word_feature(word, seen_command);
                seen_command = seen_command || !is_assignment(word);
                feature
            }
            PosixTokenKind::Newline | PosixTokenKind::Comment(_) => false,
        };
        if feature {
            return true;
        }
    }
    false
}

fn word_feature(word: &Word, after_command: bool) -> bool {
    if word.has_expansion() || is_single_quoted(word) {
        return true;
    }
    if !after_command {
        return is_assignment(word);
    }
    is_flag(word) || word.has_unquoted_glob() || word.raw == "{}"
}

/// Text in single quotes exists only in the shell; the language quotes
/// text with `"`.
fn is_single_quoted(word: &Word) -> bool {
    word.parts.iter().any(|part| {
        matches!(
            part,
            WordPart::Literal {
                quoting: Quoting::Single,
                ..
            }
        )
    })
}

fn is_flag(word: &Word) -> bool {
    let raw = &word.raw;
    let Some(rest) = raw.strip_prefix('-') else {
        return false;
    };
    let rest = rest.strip_prefix('-').unwrap_or(rest);
    word.is_bare()
        && rest
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphanumeric())
}

#[cfg(test)]
mod tests {
    use super::super::lex::lex;
    use super::*;

    fn habit(input: &str) -> bool {
        let table = BashTable::embedded().unwrap();
        is_habit(table, &lex(input).unwrap())
    }

    #[test]
    fn flags_mark_a_habit() {
        assert!(habit("find /a -name \"*.go\""));
        assert!(habit("rm -rf /srv/x"));
        assert!(habit("ls --all /a"));
        assert!(habit("kill -9 42"));
    }

    #[test]
    fn nurt_programs_are_not_habits() {
        assert!(!habit("find \"/a\" glob: \"*.go\""));
        assert!(!habit("find \"/a\" |> remove"));
        assert!(!habit("remove \"/a\""));
        assert!(!habit("sort size order: desc"));
    }

    #[test]
    fn a_word_that_ends_in_a_colon_is_a_named_argument_only_for_a_known_parameter() {
        assert!(habit("grep -i error: /var/log/x"));
        assert!(habit("grep -n 'TODO:' /srv/a"));
        assert!(!habit("find /a -name x glob: \"*.go\""));
        assert!(!habit("ls -l hidden: true"));
    }

    #[test]
    fn plain_words_without_features_go_the_nurt_way() {
        assert!(!habit("cat /a/b"));
        assert!(!habit("rm /a/b"));
        assert!(!habit("cat /a | grep x"));
    }

    #[test]
    fn operators_expansions_and_patterns_mark_a_habit() {
        assert!(habit("mkdir /a && cp /b /a"));
        assert!(habit("ls /a; ls /b"));
        assert!(habit("echo x > /a/f"));
        assert!(habit("rm $FILE"));
        assert!(habit("rm `cat /list`"));
        assert!(habit("rm /a/*.tmp"));
        assert!(habit("find /a -exec rm {} +"));
        assert!(habit("sleep 10 &"));
        assert!(habit("awk '{print $1}' /a"));
        assert!(!habit("awk \"{print}\" /a"));
    }

    #[test]
    fn assignment_before_a_habit_word() {
        assert!(habit("LC_ALL=C sort /a"));
        assert!(habit("LC_ALL=C make"));
        assert!(habit("CI=1 npm test"));
        assert!(!habit("CI=1"));
    }

    #[test]
    fn shell_keywords_and_builtins_are_habits() {
        assert!(habit("cd /srv/app"));
        assert!(habit("for f in a b; do rm $f; done"));
        assert!(habit("sudo apt update"));
    }

    #[test]
    fn first_word_outside_the_habit_set_is_not_a_habit() {
        assert!(!habit("go test ./... && echo ok"));
        assert!(!habit(""));
    }

    #[test]
    fn crlf_and_multibyte_text_do_not_change_the_decision() {
        assert!(habit("rm -rf \"/srv/zażółć\"\r\n"));
        assert!(!habit("remove \"/srv/zażółć\"\r\n"));
        assert!(!habit("\r\n"));
    }

    #[test]
    fn named_arguments_and_pipes_of_nurt_are_not_habits() {
        assert!(!habit("find \"/a\" glob: \"*.go\" |> count"));
        assert!(!habit("sort size order: desc"));
        assert!(habit("cat /a | grep -i x"));
    }

    #[test]
    fn negative_number_and_quoted_dash_are_not_flags() {
        let quoted = &lex("rm '-x'").unwrap()[1];
        let PosixTokenKind::Word(word) = &quoted.kind else {
            panic!()
        };
        assert!(!is_flag(word));
        assert!(!habit("cat /a \"-\""));
    }
}
