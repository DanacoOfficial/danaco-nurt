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

use nurt_contracts::ast::{AstDocument, strip_for_hash};
use nurt_contracts::bound::BoundDocument;
use nurt_contracts::config::Config;
use nurt_contracts::envelope::Language;
use nurt_contracts::error::Error;
use nurt_contracts::kinds::SyntaxVariant;
use nurt_contracts::report::Correction;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::unavailable;
use crate::state::sha256_hex;

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
    /// Hash of the syntax tree the program was bound from, as
    /// `sha256:<hex>`; absent when the tree is not known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ast_hash: Option<String>,
}

/// Hash of a syntax tree: `sha256:` and the hex digest of the canonical
/// JSON of the tree without positions and metadata.
///
/// The hash identifies the program independently of its layout in the text.
#[must_use]
pub fn ast_hash(ast: &AstDocument) -> Option<String> {
    let tree = serde_json::to_value(ast).ok()?;
    let canonical = serde_json_canonicalizer::to_vec(&strip_for_hash(&tree)).ok()?;
    Some(format!("sha256:{}", sha256_hex(&canonical)))
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

    /// Formats program text written in `from` as canonical text of `to`; a
    /// missing variant is the configured one.
    ///
    /// A front end that keeps comments overrides this method. The default
    /// parses and prints the tree, which does not carry comments.
    ///
    /// # Errors
    ///
    /// Returns the syntax error of the text, or the error of printing it.
    fn format(
        &self,
        text: &str,
        from: Option<SyntaxVariant>,
        to: Option<SyntaxVariant>,
        config: &Config,
    ) -> Result<String, Error> {
        let tree = self.parse(text, from, config)?;
        self.print(&tree, to.unwrap_or(config.syntax.variant))
    }

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

    /// Help text of a topic of the stated kind with a number of examples.
    ///
    /// `kind` is the name of a topic kind (`command`, `group`, `error`,
    /// `skill`, `syntax`, `corrections`). The default implementation ignores
    /// `kind` and `examples` and answers like [`Frontend::help`].
    ///
    /// # Errors
    ///
    /// Returns `E_UNKNOWN_NAME` for an unknown topic or a topic of another
    /// kind.
    fn help_query(
        &self,
        topic: &str,
        kind: Option<&str>,
        examples: Option<usize>,
        language: Language,
    ) -> Result<String, Error> {
        let _ = (kind, examples);
        self.help(topic, language)
    }

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
            ast_hash: ast_hash(&corrected.ast),
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

    fn tree(spans: bool) -> AstDocument {
        let span = if spans {
            "[1, 1, 1, 6]"
        } else {
            "[2, 3, 2, 9]"
        };
        serde_json::from_str(&format!(
            r#"{{"nurt": "1.0", "program": {{"statements": [{{"t": "pipeline",
            "head": {{"t": "stage", "name": "count", "args": [], "span": {span}}},
            "stages": [], "span": {span}}}]}}}}"#
        ))
        .unwrap()
    }

    #[test]
    fn tree_hash_is_well_formed_stable_and_independent_of_positions() {
        let hash = ast_hash(&tree(true)).unwrap();
        assert!(nurt_contracts::report::is_ast_hash(&hash));
        assert_eq!(ast_hash(&tree(true)).unwrap(), hash);
        assert_eq!(ast_hash(&tree(false)).unwrap(), hash);
        let other: AstDocument = serde_json::from_value(serde_json::json!({
            "nurt": "1.0", "program": {"statements": []}
        }))
        .unwrap();
        assert_ne!(ast_hash(&other).unwrap(), hash);
    }

    #[test]
    fn tree_hash_matches_the_golden_files() {
        let dir = std::path::Path::new(
            std::env::var("CARGO_MANIFEST_DIR")
                .as_deref()
                .unwrap_or(env!("CARGO_MANIFEST_DIR")),
        )
        .join("../../spec/golden/ast");
        let mut checked = 0;
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().and_then(|e| e.to_str()) != Some("hash") {
                continue;
            }
            let ast: AstDocument = serde_json::from_str(
                &std::fs::read_to_string(path.with_extension("json")).unwrap(),
            )
            .unwrap();
            let expected = std::fs::read_to_string(&path).unwrap();
            assert_eq!(
                ast_hash(&ast).unwrap(),
                expected.trim(),
                "{}",
                path.display()
            );
            checked += 1;
        }
        assert!(checked >= 3);
    }
}
