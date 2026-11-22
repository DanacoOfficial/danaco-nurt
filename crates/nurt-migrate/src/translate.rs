// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Interface between the migration and the translation engines of the
//! bash and PowerShell habit tables.
//!
//! The migration reads the structure of a script and decides which
//! statements are translated; the rules that turn one statement into a
//! program belong to the habit tables and are applied by a [`Translator`].

use nurt_contracts::ast::AstDocument;
use nurt_contracts::error::Error;
use nurt_contracts::report::Correction;

use crate::language::ScriptLanguage;

/// Result of translating one shell statement.
#[derive(Debug, Clone, PartialEq)]
pub struct Translation {
    /// Translated program: zero or more `let` statements and pipelines.
    pub tree: AstDocument,
    /// Corrections applied, in order; each names a table rule such as `B15`.
    pub rules: Vec<Correction>,
    /// Assumptions of probable rules, as stated in the habit tables.
    pub assumptions: Vec<String>,
}

/// Shell variable that an earlier statement of the script bound to a named
/// result.
///
/// The assignment `X=$(find …)` translated to `let x = find …` produces the
/// binding `X` → `x`; later statements refer to the result as `@x`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LetBinding {
    /// Variable name in the script, without `$`.
    pub variable: String,
    /// Name of the result in the program, without `@`.
    pub name: String,
}

/// One statement of a script submitted for translation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TranslateRequest<'a> {
    /// Language of the script.
    pub language: ScriptLanguage,
    /// Source text of the statement, possibly spanning several lines.
    pub statement: &'a str,
    /// Variables bound by earlier statements, in script order.
    pub lets: &'a [LetBinding],
}

/// Translation engine used by the migration.
///
/// Implementations apply the habit tables to one statement in script mode,
/// print programs in the canonical form of syntax variant A and check that a
/// script parses and binds. The migration never applies table rules itself.
pub trait Translator {
    /// Translates one shell statement.
    ///
    /// # Errors
    ///
    /// Returns the error that the habit tables define for the statement:
    /// `E_UNCORRECTABLE` for a construct without a translation, `E_AMBIGUOUS`
    /// with candidates when the translation depends on an interpretation,
    /// `E_ONE_EFFECT` when the statement contains more than one effect and
    /// `E_PATH_RELATIVE` when a path depends on the current directory.
    fn translate(&self, request: &TranslateRequest<'_>) -> Result<Translation, Error>;

    /// Prints a program in the canonical form of syntax variant A.
    fn print(&self, tree: &AstDocument) -> String;

    /// Checks that a script parses and binds with the command catalog.
    ///
    /// The script consists of the `let` statements accepted so far followed by
    /// the statement under test.
    ///
    /// # Errors
    ///
    /// Returns the first syntax or binding error of the script.
    fn check(&self, script: &str) -> Result<(), Error>;
}
