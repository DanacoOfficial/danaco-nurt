// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Translation of commands written in bash notation into Nurt programs.
//!
//! A statement whose first word belongs to the habit word set and which
//! shows a feature found only in the shell (a dashed flag, `&&`, `;`, a
//! redirection, `$`, a backtick, an assignment, an unquoted pattern) is read
//! with the POSIX lexer and translated by the bash translation table. Every
//! translation is reported as a correction with its rule (`B01` to `B40`)
//! and certainty class. A translation either keeps the meaning of the
//! command (class `C`) or names its assumption (class `P`); a probable
//! translation of a changing program is not applied and yields
//! `E_AMBIGUOUS` with ranked candidates. Targets of changes (working
//! directory, `base`, paths, privileges) are never guessed: commands that
//! cannot be translated yield an error with a ready fix where one exists.
//!
//! # Examples
//!
//! ```
//! use nurt_correct::bash;
//!
//! let translation = bash::translate("rm -rf /srv/app/build").expect("a bash habit");
//! assert_eq!(translation.text(), "remove \"/srv/app/build\"");
//! assert_eq!(translation.rules[0].rule.to_string(), "B01");
//! assert!(bash::translate("remove \"/srv/app/build\"").is_none());
//! ```

mod admin;
mod awk;
mod catalog;
mod compound;
mod construct;
mod detect;
mod engine;
mod find;
mod lex;
mod params;
mod pattern;
mod produce;
mod sed;
mod session;
mod syntax;
mod table;
mod template;
mod tree;

use nurt_contracts::ast::AstDocument;
use nurt_contracts::error::Error;

use crate::record::Correction;

pub use lex::{
    HereDoc, LexError, Operator, PosixToken, PosixTokenKind, Quoting, Redirect, RedirectOp, Word,
    WordPart, lex,
};
pub use params::{ParamKind, Rewritten, ScriptParam, rewrite_positionals};
pub use table::{BashTable, TABLE_FORMAT, TableError};

/// How the translated text is executed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum Mode {
    /// One call: `let` statements and one pipeline; a probable translation
    /// of a changing program yields `E_AMBIGUOUS`, and two statements joined
    /// by `&&` or `;` yield their separate programs as next steps.
    #[default]
    Live,
    /// A script: every statement becomes its own pipeline and the script
    /// stops at the first error; probable translations are applied with
    /// their assumptions. The caller states that the text is a script in the
    /// language of the translator, so a statement is not checked for being a
    /// habit of that language; `nurt migrate` is the caller and it refuses
    /// program files (`.nurt`) before any translation.
    Script,
}

/// A function defined earlier in the script, so that a call of it is a call
/// and not a program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionSignature {
    /// Name of the function.
    pub name: String,
    /// Number of its parameters.
    pub params: usize,
}

/// Context of a translation.
#[derive(Debug, Clone, Copy, Default)]
pub struct TranslateContext<'a> {
    /// Call parameter `base`: the directory relative paths are resolved
    /// against. Without it a relative path is an error.
    pub base: Option<&'a str>,
    /// Names of results bound with `let` earlier in the program or script.
    pub lets: &'a [String],
    /// Functions defined earlier in the script.
    pub functions: &'a [FunctionSignature],
    /// Execution mode.
    pub mode: Mode,
}

/// Result of a translation.
#[derive(Debug, Clone, PartialEq)]
pub struct Translation {
    /// The translated program as a surface tree; spans point into the input.
    pub tree: AstDocument,
    /// Corrections applied, in source order.
    pub rules: Vec<Correction>,
    /// Assumptions of probable translations.
    pub assumptions: Vec<Assumption>,
    /// Other interpretations of a probable translation, the likeliest first;
    /// at most three.
    pub alternatives: Vec<AstDocument>,
}

impl Translation {
    /// Returns the canonical text of every alternative, in rank order.
    ///
    /// # Examples
    ///
    /// ```
    /// let translation = nurt_correct::bash::translate("grep -rl \"oldName(\" /srv/app").expect("a bash habit");
    /// assert_eq!(translation.text(), "search \"oldName(\" \"/srv/app\" output: files");
    /// assert_eq!(translation.alternative_texts(), vec!["search /oldName\\(/ \"/srv/app\" output: files".to_owned()]);
    /// ```
    #[must_use]
    pub fn alternative_texts(&self) -> Vec<String> {
        self.alternatives.iter().map(tree::print_document).collect()
    }

    /// Returns the canonical text of the translated program.
    ///
    /// # Examples
    ///
    /// ```
    /// let translation = nurt_correct::bash::translate("mkdir -p /srv/out").expect("a bash habit");
    /// assert_eq!(translation.text(), "create \"/srv/out\" type: directory");
    /// ```
    #[must_use]
    pub fn text(&self) -> String {
        tree::print_document(&self.tree)
    }
}

/// An assumption made by a probable translation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assumption {
    /// Rule of the translation, for example `B15`.
    pub rule: String,
    /// The assumption in English.
    pub text: String,
}

/// Whether the text is a command written in bash notation rather than a
/// Nurt program.
///
/// # Examples
///
/// ```
/// use nurt_correct::bash::detect;
///
/// assert!(detect("find /a -name \"*.go\""));
/// assert!(!detect("find \"/a\" glob: \"*.go\""));
/// assert!(!detect("cat /a/b"));
/// ```
#[must_use]
pub fn detect(input: &str) -> bool {
    let Ok(table) = BashTable::embedded() else {
        return false;
    };
    lex(input).is_ok_and(|tokens| detect::is_habit(table, &tokens))
}

/// Whether the text starts with a word of the habit word set and shows no
/// Nurt notation, whether or not it shows a feature of the shell.
///
/// For a first word that has a meaning in Nurt, such as `cat`, this is not
/// enough to tell a habit from a program: use [`detect`]. For a word that
/// has none, such as `touch`, it is.
///
/// # Examples
///
/// ```
/// use nurt_correct::bash::{detect, detect_by_word};
///
/// assert!(detect_by_word("touch /srv/app/a.txt"));
/// assert!(!detect("touch /srv/app/a.txt"));
/// assert!(!detect_by_word("touch /srv/app/a.txt glob: \"*\""));
/// ```
#[must_use]
pub fn detect_by_word(input: &str) -> bool {
    let Ok(table) = BashTable::embedded() else {
        return false;
    };
    lex(input).is_ok_and(|tokens| detect::is_habit_by_word(table, &tokens))
}

/// Translates a statement that [`detect_by_word`] recognizes, with the
/// embedded table.
///
/// Returns `None` when the text does not start with a habit word.
///
/// # Errors
///
/// As [`translate_with`].
///
/// # Examples
///
/// ```
/// use nurt_correct::bash::{TranslateContext, translate_by_word};
///
/// let translation = translate_by_word("touch /srv/app/a.txt", &TranslateContext::default())
///     .expect("a habit word")
///     .expect("a translation");
/// assert_eq!(translation.text(), "create \"/srv/app/a.txt\" type: file on_exists: touch");
/// ```
#[must_use]
pub fn translate_by_word(
    input: &str,
    context: &TranslateContext<'_>,
) -> Option<Result<Translation, Error>> {
    match BashTable::embedded() {
        Ok(table) => session::translate_by_word(input, context, table),
        Err(error) => Some(Err(Error::new(
            nurt_contracts::error::ErrorCode::InternalCorrection,
            format!("invalid bash translation table: {error}"),
        ))),
    }
}

/// Translates a bash command in a live call without `base` and without
/// known `let` names.
///
/// Returns `None` when the text is not a bash habit or when the translation
/// ends with an error; [`translate_with`] returns the error.
///
/// # Examples
///
/// ```
/// let translation = nurt_correct::bash::translate("cd /srv/app && go test ./...").expect("a bash habit");
/// assert_eq!(translation.text(), "run [\"go\", \"test\", \"./...\"] cwd: \"/srv/app\"");
/// ```
#[must_use]
pub fn translate(input: &str) -> Option<Translation> {
    translate_with(input, &TranslateContext::default()).and_then(Result::ok)
}

/// Translates a bash command with the embedded table.
///
/// Returns `None` when the text is not a bash habit; in script mode every
/// statement is translated.
///
/// # Errors
///
/// The inner result is an [`Error`] with the code `E_UNCORRECTABLE`,
/// `E_ONE_EFFECT`, `E_AMBIGUOUS` or `E_PATH_RELATIVE`, with ready fixes,
/// next steps or candidates where they exist. An invalid embedded table
/// yields `E_INTERNAL_CORRECTION`.
///
/// # Examples
///
/// ```
/// use nurt_contracts::error::ErrorCode;
/// use nurt_correct::bash::{TranslateContext, translate_with};
///
/// let translation = translate_with("sudo rm -f /etc/x", &TranslateContext::default())
///     .expect("a bash habit")
///     .expect("sudo is translated");
/// assert_eq!(translation.text(), "remove \"/etc/x\" privilege: elevated");
///
/// let error = translate_with("sudo -i", &TranslateContext::default())
///     .expect("a bash habit")
///     .expect_err("an elevated session is not a command");
/// assert_eq!(error.code(), ErrorCode::Ambiguous);
/// ```
#[must_use]
pub fn translate_with(
    input: &str,
    context: &TranslateContext<'_>,
) -> Option<Result<Translation, Error>> {
    match BashTable::embedded() {
        Ok(table) => translate_with_table(input, context, table),
        Err(error) => Some(Err(Error::new(
            nurt_contracts::error::ErrorCode::InternalCorrection,
            format!("invalid bash translation table: {error}"),
        ))),
    }
}

/// Translates a bash command with the given table.
///
/// Returns `None` when the text is not a bash habit.
///
/// # Errors
///
/// As [`translate_with`].
#[must_use]
pub fn translate_with_table(
    input: &str,
    context: &TranslateContext<'_>,
    table: &BashTable,
) -> Option<Result<Translation, Error>> {
    session::translate(input, context, table)
}

#[cfg(test)]
mod tests {
    use nurt_contracts::error::ErrorCode;

    use super::*;

    fn failure(input: &str) -> Error {
        translate_with(input, &TranslateContext::default())
            .expect("a bash habit")
            .expect_err("an error")
    }

    #[test]
    fn sed_with_several_expressions_is_not_translated() {
        assert_eq!(
            failure("sed -i -e 's/a/b/' -e 's/c/d/' /srv/f").code(),
            ErrorCode::Uncorrectable
        );
        assert!(translate("sed -i -e 's/a/b/' /srv/f").is_some());
    }

    #[test]
    fn sed_replacement_with_case_conversion_is_not_translated() {
        assert_eq!(
            failure(r"sed -i 's/a/\U&/' /srv/f").code(),
            ErrorCode::Uncorrectable
        );
    }

    #[test]
    fn env_prefix_keeps_the_text_of_a_quoted_value() {
        let translation = translate("cd /srv/app && env FOO=\"a b\" make").expect("a bash habit");
        assert_eq!(
            translation.text(),
            "run [\"make\"] cwd: \"/srv/app\" env: {\"FOO\": \"a b\"}"
        );
    }

    #[test]
    fn environment_values_with_expansions_are_rejected() {
        for input in [
            "cd /srv/app && export PATH=$PATH:/opt/bin && make",
            "cd /srv/app && FOO=$BAR make",
        ] {
            assert_eq!(failure(input).code(), ErrorCode::Uncorrectable, "{input}");
        }
    }

    #[test]
    fn deeply_nested_prefixes_are_an_error_not_a_stack_overflow() {
        let nested = format!("cd /srv/app && {}make", "time ".repeat(200));
        assert_eq!(failure(&nested).code(), ErrorCode::Uncorrectable);
        assert!(translate("cd /srv/app && time time make").is_some());
    }

    #[test]
    fn remote_name_of_a_download_ignores_the_query() {
        let context = TranslateContext {
            base: Some("/srv/dl"),
            ..TranslateContext::default()
        };
        let translation =
            translate_with("curl -O 'https://example.com/a.tar.gz?token=1'", &context)
                .expect("a bash habit")
                .expect("a translation");
        assert!(translation.text().contains("save: \"/srv/dl/a.tar.gz\""));
    }

    #[test]
    fn multibyte_text_is_kept_in_a_translation() {
        let translation = translate("rm -rf \"/srv/zażółć\"").expect("a bash habit");
        assert_eq!(translation.text(), "remove \"/srv/zażółć\"");
    }
}
