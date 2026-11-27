// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-migrate::translate
// Responsibility:   Interface between the migration and the translation of the bash and PowerShell habit tables.
// Dependencies:     nurt_bind, nurt_contracts, nurt_correct, nurt_syntax
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Interface between the migration and the translation of the bash and
//! PowerShell habit tables.
//!
//! The migration reads the structure of a script and decides which
//! statements are translated; the rules that turn one statement into a
//! program belong to the habit tables and are applied by a [`Translator`].
//! [`TableTranslator`] applies the tables of `nurt-correct` in script mode,
//! prints with the canonical printer of syntax variant A and checks that the
//! result parses.

use std::fmt;

use nurt_bind::{BindContext, Catalog, bind};
use nurt_contracts::ast::AstDocument;
use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::report::Correction;
use nurt_contracts::spec::Os;
use nurt_correct::bash::{self, Assumption, BashTable, FunctionSignature, Mode, TranslateContext};
use nurt_correct::policy::{InvariantInput, check_invariants, is_effecting};
use nurt_correct::powershell::{self, PowerShellTable};
use nurt_correct::record::Log;
use nurt_correct::tables::{Tables, load_embedded};
use nurt_syntax::parse::{ParseOptions, Variant, parse};
use nurt_syntax::print::print;

use crate::language::ScriptLanguage;

/// Problem of a statement that only previews its changes (`-WhatIf`).
const PREVIEW_PROBLEM: &str = "a preview of a single statement has no equivalent in a script; run the program with the invocation parameter preview set to true";

/// Result of translating one shell statement.
#[derive(Debug, Clone, PartialEq)]
pub struct Translation {
    /// Translated program: zero or more `let` statements and pipelines.
    pub tree: AstDocument,
    /// Corrections applied, in order; each names a table rule such as `B15`.
    pub rules: Vec<Correction>,
    /// Assumptions of probable rules, as stated in the habit tables.
    pub assumptions: Vec<String>,
    /// Names bound by the translated program, with the record type of their
    /// values, in source order.
    pub records: Vec<(String, Option<String>)>,
}

/// Shell variable that an earlier statement of the script bound to a named
/// result.
///
/// The assignment `X=$(find ...)` translated to `let x = find ...` binds the
/// variable `X` to the name `x`; later statements refer to the result as `@x`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LetBinding {
    /// Variable name in the script, without `$`.
    pub variable: String,
    /// Name of the result in the program, without `@`.
    pub name: String,
    /// Record type of the value (`entry`, `line`, `process`, …) when the habit
    /// tables know it; PowerShell uses it to resolve property reads.
    pub record: Option<String>,
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
    /// Functions defined by earlier statements, in script order.
    pub functions: &'a [FunctionSignature],
}

/// Translation engine used by the migration.
///
/// Implementations apply the habit tables to one statement in script mode,
/// print programs in the canonical form of syntax variant A and check that a
/// script parses. The migration never applies table rules itself.
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

    /// Checks that a script parses.
    ///
    /// The script consists of the `let` statements accepted so far followed by
    /// the statement under test.
    ///
    /// # Errors
    ///
    /// Returns the first syntax error of the script.
    fn check(&self, script: &str) -> Result<(), Error>;
}

/// Translator that applies the bash and PowerShell habit tables of
/// `nurt-correct` in script mode.
///
/// Without further configuration the embedded tables are used; a table read
/// from another source replaces the embedded one for its language. The
/// context of every translation has no `base` and no current directory, so
/// targets that depend on them are reported, not guessed.
///
/// # Examples
///
/// ```
/// use nurt_migrate::{ScriptLanguage, TableTranslator, TranslateRequest, Translator};
///
/// let translator = TableTranslator::new();
/// let request = TranslateRequest {
///     language: ScriptLanguage::Bash,
///     statement: "mkdir -p /srv/out",
///     lets: &[],
///     functions: &[],
/// };
/// let translation = translator.translate(&request)?;
/// assert_eq!(translator.print(&translation.tree), "create \"/srv/out\" type: directory");
/// assert_eq!(translation.rules[0].rule, "B05");
/// # Ok::<(), nurt_contracts::error::Error>(())
/// ```
#[derive(Default)]
pub struct TableTranslator {
    bash: Option<BashTable>,
    powershell: Option<PowerShellTable>,
    binding: Option<Binding>,
}

/// Catalog and correction tables used to bind every translated statement.
struct Binding {
    host_os: Os,
    catalog: Catalog,
    tables: Tables,
}

impl TableTranslator {
    /// Creates a translator with the embedded habit tables.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Replaces the embedded bash habit table.
    #[must_use]
    pub fn with_bash_table(mut self, table: BashTable) -> Self {
        self.bash = Some(table);
        self
    }

    /// Replaces the embedded PowerShell habit table.
    #[must_use]
    pub fn with_powershell_table(mut self, table: PowerShellTable) -> Self {
        self.powershell = Some(table);
        self
    }

    /// Binds every translated statement to `catalog` for a script that runs on
    /// `host_os` and checks that binding keeps its meaning. Paths foreign to
    /// `host_os`, such as a drive letter on Linux, fail the binding.
    ///
    /// The check of a statement then also fails with the binding error of
    /// stage `1`, or with `E_INTERNAL_CORRECTION` when the bound program does
    /// not keep the meaning of the printed one; the migration reports such a
    /// statement as an error, not as translated.
    ///
    /// # Errors
    ///
    /// Returns `E_INTERNAL_CORRECTION` when the embedded correction tables
    /// are invalid.
    pub fn with_catalog(mut self, catalog: Catalog, host_os: Os) -> Result<Self, Error> {
        let tables = load_embedded().map_err(|error| {
            Error::new(
                ErrorCode::InternalCorrection,
                format!("invalid correction tables: {error}"),
            )
        })?;
        self.binding = Some(Binding {
            host_os,
            catalog,
            tables,
        });
        Ok(self)
    }

    fn translate_bash(
        &self,
        statement: &str,
        context: &TranslateContext<'_>,
    ) -> Option<Result<Translation, Error>> {
        let outcome = match &self.bash {
            Some(table) => bash::translate_with_table(statement, context, table),
            None => bash::translate_with(statement, context),
        };
        outcome.map(|result| {
            result.map(|translation| {
                let mut log = Log::new();
                for correction in translation.rules {
                    log.push(correction);
                }
                Translation {
                    tree: translation.tree,
                    rules: log.into_report(),
                    assumptions: assumption_texts(translation.assumptions),
                    records: Vec::new(),
                }
            })
        })
    }

    fn translate_powershell(
        &self,
        statement: &str,
        context: &TranslateContext<'_>,
        lets: &[LetBinding],
    ) -> Option<Result<Translation, Error>> {
        let known: Vec<powershell::LetBinding> = lets
            .iter()
            .map(|binding| powershell::LetBinding {
                name: binding.name.clone(),
                record: binding.record.clone(),
            })
            .collect();
        let table = match &self.powershell {
            Some(table) => table,
            None => match PowerShellTable::embedded() {
                Ok(table) => table,
                Err(error) => {
                    return Some(Err(Error::new(
                        ErrorCode::InternalCorrection,
                        format!("invalid PowerShell translation table: {error}"),
                    )));
                }
            },
        };
        let outcome = powershell::translate_with_lets(statement, context, &known, table);
        outcome.map(|result| {
            result.and_then(|translation| {
                if translation.options.preview == Some(true) {
                    let program = print_program(&translation.tree);
                    return Err(Error::new(
                        ErrorCode::Uncorrectable,
                        "the statement only previews its changes",
                    )
                    .with_problem(PREVIEW_PROBLEM)
                    .with_fix(program));
                }
                Ok(Translation {
                    tree: translation.tree,
                    rules: translation.rules,
                    assumptions: assumption_texts(translation.assumptions),
                    records: translation
                        .bindings
                        .into_iter()
                        .map(|binding| (binding.name, binding.record))
                        .collect(),
                })
            })
        })
    }
}

impl fmt::Debug for TableTranslator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TableTranslator")
            .field("custom_bash_table", &self.bash.is_some())
            .field("custom_powershell_table", &self.powershell.is_some())
            .field("binds", &self.binding.is_some())
            .finish()
    }
}

impl Translator for TableTranslator {
    fn translate(&self, request: &TranslateRequest<'_>) -> Result<Translation, Error> {
        let names: Vec<String> = request
            .lets
            .iter()
            .map(|binding| binding.name.clone())
            .collect();
        let context = TranslateContext {
            base: None,
            lets: &names,
            functions: request.functions,
            mode: Mode::Script,
        };
        let outcome = match request.language {
            ScriptLanguage::Bash => self.translate_bash(request.statement, &context),
            ScriptLanguage::PowerShell => {
                self.translate_powershell(request.statement, &context, request.lets)
            }
        };
        outcome.unwrap_or_else(|| Err(not_a_habit(request)))
    }

    fn print(&self, tree: &AstDocument) -> String {
        print_program(tree)
    }

    fn check(&self, script: &str) -> Result<(), Error> {
        let tree = parse(script, Variant::A, &ParseOptions::default()).map_err(Error::from)?;
        match &self.binding {
            Some(binding) => binding.check(&tree),
            None => Ok(()),
        }
    }
}

impl Binding {
    /// Binds the program and checks the invariants of the result.
    fn check(&self, tree: &AstDocument) -> Result<(), Error> {
        let bound = bind(tree, &self.catalog, &BindContext::new(self.host_os))
            .map_err(|e| e.into_error())?;
        let effecting = |name: &str| self.catalog.command(name).is_some_and(is_effecting);
        check_invariants(&InvariantInput {
            before: Some(tree),
            after: &bound,
            corrections: &[],
            base: None,
            tables: &self.tables,
            effecting: &effecting,
        })
        .map_err(|violation| violation.to_error())
    }
}

/// Canonical text of a program in syntax variant A, without the final line
/// break.
fn print_program(tree: &AstDocument) -> String {
    let mut text = print(tree, Variant::A);
    let trimmed = text.trim_end().len();
    text.truncate(trimmed);
    text
}

fn assumption_texts(assumptions: Vec<Assumption>) -> Vec<String> {
    assumptions
        .into_iter()
        .map(|assumption| assumption.text)
        .collect()
}

/// `E_UNCORRECTABLE` for a statement that the habit table does not read as a
/// command of its shell.
fn not_a_habit(request: &TranslateRequest<'_>) -> Error {
    let command = request
        .statement
        .split_whitespace()
        .next()
        .unwrap_or_default();
    let table = match request.language {
        ScriptLanguage::Bash => "bash",
        ScriptLanguage::PowerShell => "PowerShell",
    };
    Error::new(ErrorCode::Uncorrectable, "no translation rule").with_problem(format!(
        "`{command}` is not a command of the {table} habit table"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request<'a>(
        language: ScriptLanguage,
        statement: &'a str,
        lets: &'a [LetBinding],
    ) -> TranslateRequest<'a> {
        TranslateRequest {
            language,
            statement,
            lets,
            functions: &[],
        }
    }

    #[test]
    fn translates_bash_statement_with_table_rule() -> Result<(), Error> {
        let translator = TableTranslator::new();
        let translation =
            translator.translate(&request(ScriptLanguage::Bash, "rm -rf /srv/app/build", &[]))?;
        assert_eq!(
            translator.print(&translation.tree),
            "remove \"/srv/app/build\""
        );
        assert_eq!(translation.rules[0].rule, "B01");
        Ok(())
    }

    #[test]
    fn translates_powershell_statement_with_table_rule() -> Result<(), Error> {
        let translator = TableTranslator::new();
        let translation = translator.translate(&request(
            ScriptLanguage::PowerShell,
            "Remove-Item -Path C:\\build\\out -Recurse",
            &[],
        ))?;
        assert_eq!(
            translator.print(&translation.tree),
            "remove \"C:/build/out\""
        );
        assert_eq!(translation.rules[0].rule, "P08");
        Ok(())
    }

    #[test]
    fn translates_privilege_elevation_with_the_call_parameter_on_the_stage() -> Result<(), Error> {
        let translator = TableTranslator::new();
        let translation =
            translator.translate(&request(ScriptLanguage::Bash, "sudo rm -f /etc/x", &[]))?;
        assert_eq!(
            translator.print(&translation.tree),
            "remove \"/etc/x\" privilege: elevated"
        );
        Ok(())
    }

    #[test]
    fn rejects_an_elevated_session_with_next_steps() {
        let translator = TableTranslator::new();
        let error = translator
            .translate(&request(ScriptLanguage::Bash, "sudo -i", &[]))
            .err();
        let data = error.as_ref().map(Error::data);
        assert_eq!(data.map(|data| data.code), Some(ErrorCode::Ambiguous));
        assert!(data.is_some_and(|data| !data.next_steps.is_empty()));
    }

    #[test]
    fn rejects_preview_of_single_statement() {
        let translator = TableTranslator::new();
        let error = translator
            .translate(&request(
                ScriptLanguage::PowerShell,
                "Remove-Item -Path C:\\build\\out -Recurse -WhatIf",
                &[],
            ))
            .err();
        let data = error.as_ref().map(Error::data);
        assert_eq!(data.map(|data| data.code), Some(ErrorCode::Uncorrectable));
        assert_eq!(
            data.and_then(|data| data.problem.clone()).as_deref(),
            Some(PREVIEW_PROBLEM)
        );
    }

    #[test]
    fn check_reports_syntax_error() {
        let translator = TableTranslator::new();
        assert!(translator.check("remove \"/srv/a\"").is_ok());
        let error = translator.check("filter a < b < c").err();
        assert_eq!(error.map(|error| error.code()), Some(ErrorCode::Syntax));
    }

    #[test]
    fn debug_names_custom_tables() {
        let text = format!("{:?}", TableTranslator::new());
        assert!(text.contains("custom_bash_table: false"));
    }

    #[test]
    fn not_a_habit_names_the_command() {
        let error = not_a_habit(&request(ScriptLanguage::PowerShell, "Get-Foo -Bar", &[]));
        assert_eq!(
            error.data().problem.as_deref(),
            Some("`Get-Foo` is not a command of the PowerShell habit table")
        );
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
