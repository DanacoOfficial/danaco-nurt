// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Language front end of the engine.
//!
//! The front end turns program text into a syntax tree (parsing), corrects
//! the written form, binds the tree to the command catalog and prints a tree
//! back as text. It also answers description and help requests. The engine
//! reaches it only through [`Frontend`], so that the language components are
//! assembled with the engine rather than compiled into it.
//! [`UnavailableFrontend`] is used when no front end is given.

use nurt_contracts::ast::AstDocument;
use nurt_contracts::bound::BoundDocument;
use nurt_contracts::config::Config;
use nurt_contracts::envelope::Language;
use nurt_contracts::error::Error;
use nurt_contracts::kinds::SyntaxVariant;
use nurt_contracts::report::Correction;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::unavailable;

/// Program text after correction.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Corrected {
    /// Syntax tree of the corrected program.
    pub ast: AstDocument,
    /// Corrections applied to the text.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub corrections: Vec<Correction>,
    /// Canonical text of the corrected program; present only when
    /// corrections were applied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ran: Option<String>,
}

/// Program ready for planning.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Compiled {
    /// The bound program.
    pub bound: BoundDocument,
    /// Corrections applied to the text.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub corrections: Vec<Correction>,
    /// Canonical text of the corrected program; present only when
    /// corrections were applied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ran: Option<String>,
}

/// Language front end: parsing, correction, binding, printing and help.
///
/// Errors are language errors of the parse and bind steps, for example
/// `E_SYNTAX`, `E_UNKNOWN_COMMAND` or `E_AMBIGUOUS` with candidates.
pub trait Frontend: Send + Sync {
    /// Parses program text without correcting it.
    ///
    /// # Errors
    ///
    /// Returns the lexical or syntax error of the text.
    fn parse(
        &self,
        text: &str,
        variant: Option<SyntaxVariant>,
        config: &Config,
    ) -> Result<AstDocument, Error>;

    /// Parses and corrects program text within the correction budget of
    /// `config`.
    ///
    /// # Errors
    ///
    /// Returns the syntax error, or `E_AMBIGUOUS` or `E_UNCORRECTABLE` with
    /// candidates.
    fn correct(
        &self,
        text: &str,
        variant: Option<SyntaxVariant>,
        config: &Config,
    ) -> Result<Corrected, Error>;

    /// Binds a syntax tree to the command catalog.
    ///
    /// # Errors
    ///
    /// Returns the bind error, for example `E_UNKNOWN_PARAM`.
    fn bind(&self, ast: &AstDocument, config: &Config) -> Result<BoundDocument, Error>;

    /// Prints a syntax tree as text of a notation variant.
    ///
    /// # Errors
    ///
    /// Returns `E_AST_SCHEMA` for a tree that cannot be printed.
    fn print(&self, ast: &AstDocument, variant: SyntaxVariant) -> Result<String, Error>;

    /// Description of a command, a group or the language (`core`) as JSON.
    ///
    /// # Errors
    ///
    /// Returns `E_UNKNOWN_NAME` for an unknown subject.
    fn describe(&self, subject: &str, language: Language) -> Result<serde_json::Value, Error>;

    /// Help text of a topic.
    ///
    /// # Errors
    ///
    /// Returns `E_UNKNOWN_NAME` for an unknown topic.
    fn help(&self, topic: &str, language: Language) -> Result<String, Error>;

    /// Corrects and binds program text.
    ///
    /// # Errors
    ///
    /// Returns the first error of correction or binding.
    fn compile(
        &self,
        text: &str,
        variant: Option<SyntaxVariant>,
        config: &Config,
    ) -> Result<Compiled, Error> {
        let corrected = self.correct(text, variant, config)?;
        let bound = self.bind(&corrected.ast, config)?;
        Ok(Compiled {
            bound,
            corrections: corrected.corrections,
            ran: corrected.ran,
        })
    }
}

/// Front end of an engine assembled without language components; every call
/// fails with `E_UNSUPPORTED`.
#[derive(Debug, Clone, Copy, Default)]
pub struct UnavailableFrontend;

const FRONTEND: &str = "language front end";

impl Frontend for UnavailableFrontend {
    fn parse(
        &self,
        _text: &str,
        _variant: Option<SyntaxVariant>,
        _config: &Config,
    ) -> Result<AstDocument, Error> {
        Err(unavailable(FRONTEND))
    }

    fn correct(
        &self,
        _text: &str,
        _variant: Option<SyntaxVariant>,
        _config: &Config,
    ) -> Result<Corrected, Error> {
        Err(unavailable(FRONTEND))
    }

    fn bind(&self, _ast: &AstDocument, _config: &Config) -> Result<BoundDocument, Error> {
        Err(unavailable(FRONTEND))
    }

    fn print(&self, _ast: &AstDocument, _variant: SyntaxVariant) -> Result<String, Error> {
        Err(unavailable(FRONTEND))
    }

    fn describe(&self, _subject: &str, _language: Language) -> Result<serde_json::Value, Error> {
        Err(unavailable(FRONTEND))
    }

    fn help(&self, _topic: &str, _language: Language) -> Result<String, Error> {
        Err(unavailable(FRONTEND))
    }
}

#[cfg(test)]
mod tests {
    use nurt_contracts::error::ErrorCode;

    use super::*;

    #[test]
    fn unavailable_front_end_rejects_every_call() {
        let frontend = UnavailableFrontend;
        let config = Config::default();
        let ast: AstDocument = serde_json::from_value(serde_json::json!({
            "nurt": "1.0", "program": {"statements": []}
        }))
        .unwrap();
        let codes = [
            frontend.parse("find", None, &config).unwrap_err().code,
            frontend.correct("find", None, &config).unwrap_err().code,
            frontend.compile("find", None, &config).unwrap_err().code,
            frontend.bind(&ast, &config).unwrap_err().code,
            frontend.print(&ast, SyntaxVariant::A).unwrap_err().code,
            frontend.describe("core", Language::En).unwrap_err().code,
            frontend.help("find", Language::En).unwrap_err().code,
        ];
        assert!(codes.iter().all(|code| *code == ErrorCode::Unsupported));
    }
}
