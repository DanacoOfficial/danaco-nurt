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
//! assert_eq!(translation.rules[0].rule, "B01");
//! assert!(bash::translate("remove \"/srv/app/build\"").is_none());
//! ```

mod awk;
mod detect;
mod engine;
mod find;
mod lex;
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
use nurt_contracts::report::Correction;

pub use lex::{
    HereDoc, LexError, Operator, PosixToken, PosixTokenKind, Quoting, Redirect, RedirectOp, Word,
    WordPart, lex,
};
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
    /// their assumptions.
    Script,
}

/// Context of a translation.
#[derive(Debug, Clone, Copy, Default)]
pub struct TranslateContext<'a> {
    /// Call parameter `base`: the directory relative paths are resolved
    /// against. Without it a relative path is an error.
    pub base: Option<&'a str>,
    /// Names of results bound with `let` earlier in the program or script.
    pub lets: &'a [String],
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
    /// Other interpretations of a probable translation, most likely first;
    /// at most three.
    pub alternatives: Vec<AstDocument>,
}

impl Translation {
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
/// let error = translate_with("sudo rm /etc/x", &TranslateContext::default())
///     .expect("a bash habit")
///     .expect_err("sudo is not translated");
/// assert_eq!(error.code, ErrorCode::Uncorrectable);
/// assert_eq!(error.fix, vec!["remove \"/etc/x\"".to_owned()]);
/// ```
#[must_use]
pub fn translate_with(input: &str, context: &TranslateContext<'_>) -> Option<Result<Translation, Error>> {
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
