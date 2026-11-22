// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Front end answering from prepared syntax and bound trees.
//!
//! [`FixtureFrontend`] maps program texts to bound trees (`nurt-bound/1`) and
//! optionally to syntax trees (`nurt-ast/1`) or to errors. It lets the
//! interfaces built on the engine (the C interface, the protocol server, the
//! command line) be tested before the language components are assembled. It
//! is compiled with the Cargo feature `fixtures`.
//!
//! A fixture directory holds, for every program, a file `<name>.nurt` with
//! the program text, `<name>.bound.json` with its bound tree and optionally
//! `<name>.ast.json` with its syntax tree.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use nurt_contracts::ast::AstDocument;
use nurt_contracts::bound::BoundDocument;
use nurt_contracts::config::Config;
use nurt_contracts::envelope::Language;
use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::kinds::SyntaxVariant;

use crate::error::unavailable;
use crate::frontend::{Compiled, Corrected, Frontend};

const PROGRAM_EXTENSION: &str = "nurt";
const BOUND_SUFFIX: &str = "bound.json";
const AST_SUFFIX: &str = "ast.json";

#[derive(Debug, Clone, PartialEq)]
struct Trees {
    ast: Option<AstDocument>,
    bound: BoundDocument,
}

#[derive(Debug, Clone, PartialEq)]
enum Fixture {
    Trees(Box<Trees>),
    Failure(Error),
}

/// Front end answering from prepared trees.
///
/// A program text is looked up after trimming surrounding white space;
/// programs without a fixture fail with `E_UNSUPPORTED`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FixtureFrontend {
    programs: HashMap<String, Fixture>,
}

impl FixtureFrontend {
    /// Front end without programs.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a program bound to `bound`.
    #[must_use]
    pub fn with_bound(mut self, program: &str, bound: BoundDocument) -> Self {
        self.programs.insert(
            key(program),
            Fixture::Trees(Box::new(Trees { ast: None, bound })),
        );
        self
    }

    /// Adds a program with its syntax tree and bound tree.
    #[must_use]
    pub fn with_trees(mut self, program: &str, ast: AstDocument, bound: BoundDocument) -> Self {
        self.programs.insert(
            key(program),
            Fixture::Trees(Box::new(Trees {
                ast: Some(ast),
                bound,
            })),
        );
        self
    }

    /// Adds a program that fails with `error` when it is parsed or compiled.
    #[must_use]
    pub fn with_error(mut self, program: &str, error: Error) -> Self {
        self.programs.insert(key(program), Fixture::Failure(error));
        self
    }

    /// Loads every program of a fixture directory.
    ///
    /// # Errors
    ///
    /// Returns `E_IO` when a file cannot be read and `E_AST_SCHEMA` when a
    /// tree does not match its format.
    pub fn load_dir(dir: &Path) -> Result<Self, Error> {
        let mut frontend = Self::new();
        let entries = fs::read_dir(dir).map_err(|error| io_error(dir, &error))?;
        let mut programs: Vec<_> = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|ext| ext == PROGRAM_EXTENSION))
            .collect();
        programs.sort();
        for path in programs {
            let text = fs::read_to_string(&path).map_err(|error| io_error(&path, &error))?;
            let bound = read_tree::<BoundDocument>(&path.with_extension(BOUND_SUFFIX))?;
            let ast_path = path.with_extension(AST_SUFFIX);
            frontend = if ast_path.is_file() {
                frontend.with_trees(&text, read_tree(&ast_path)?, bound)
            } else {
                frontend.with_bound(&text, bound)
            };
        }
        Ok(frontend)
    }

    /// Number of programs.
    #[must_use]
    pub fn len(&self) -> usize {
        self.programs.len()
    }

    /// Whether there are no programs.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.programs.is_empty()
    }

    fn lookup(&self, text: &str) -> Result<(Option<&AstDocument>, &BoundDocument), Error> {
        match self.programs.get(&key(text)) {
            Some(Fixture::Trees(trees)) => Ok((trees.ast.as_ref(), &trees.bound)),
            Some(Fixture::Failure(error)) => Err(error.clone()),
            None => Err(Error::new(
                ErrorCode::Unsupported,
                format!("program `{}` has no fixture", text.trim()),
            )),
        }
    }

    fn by_ast(&self, ast: &AstDocument) -> Option<(&str, &BoundDocument)> {
        self.programs
            .iter()
            .find_map(|(text, fixture)| match fixture {
                Fixture::Trees(trees) if trees.ast.as_ref() == Some(ast) => {
                    Some((text.as_str(), &trees.bound))
                }
                _ => None,
            })
    }
}

fn key(program: &str) -> String {
    program.trim().to_owned()
}

fn io_error(path: &Path, error: &std::io::Error) -> Error {
    Error::new(
        ErrorCode::Io,
        format!("cannot read `{}`: {error}", path.display()),
    )
}

fn read_tree<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, Error> {
    let bytes = fs::read(path).map_err(|error| io_error(path, &error))?;
    serde_json::from_slice(&bytes).map_err(|error| {
        Error::new(
            ErrorCode::AstSchema,
            format!("`{}` is not a valid tree: {error}", path.display()),
        )
    })
}

fn missing_tree() -> Error {
    Error::new(ErrorCode::Unsupported, "the fixture has no syntax tree")
}

impl Frontend for FixtureFrontend {
    fn parse(
        &self,
        text: &str,
        _variant: Option<SyntaxVariant>,
        _config: &Config,
    ) -> Result<AstDocument, Error> {
        let (ast, _) = self.lookup(text)?;
        ast.cloned().ok_or_else(missing_tree)
    }

    fn correct(
        &self,
        text: &str,
        variant: Option<SyntaxVariant>,
        config: &Config,
    ) -> Result<Corrected, Error> {
        Ok(Corrected {
            ast: self.parse(text, variant, config)?,
            corrections: Vec::new(),
            ran: None,
        })
    }

    fn bind(&self, ast: &AstDocument, _config: &Config) -> Result<BoundDocument, Error> {
        self.by_ast(ast)
            .map(|(_, bound)| bound.clone())
            .ok_or_else(|| Error::new(ErrorCode::Unsupported, "the syntax tree has no fixture"))
    }

    fn print(&self, ast: &AstDocument, _variant: SyntaxVariant) -> Result<String, Error> {
        self.by_ast(ast)
            .map(|(text, _)| text.to_owned())
            .ok_or_else(|| Error::new(ErrorCode::Unsupported, "the syntax tree has no fixture"))
    }

    fn describe(&self, _subject: &str, _language: Language) -> Result<serde_json::Value, Error> {
        Err(unavailable("description"))
    }

    fn help(&self, _topic: &str, _language: Language) -> Result<String, Error> {
        Err(unavailable("help"))
    }

    fn compile(
        &self,
        text: &str,
        _variant: Option<SyntaxVariant>,
        _config: &Config,
    ) -> Result<Compiled, Error> {
        let (_, bound) = self.lookup(text)?;
        Ok(Compiled {
            bound: bound.clone(),
            corrections: Vec::new(),
            ran: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use nurt_testkit::fixtures::empty_program;

    use super::*;

    fn ast() -> AstDocument {
        serde_json::from_value(serde_json::json!({"nurt": "1.0", "program": {"statements": []}}))
            .unwrap()
    }

    #[test]
    fn programs_compile_to_their_bound_tree() {
        let frontend = FixtureFrontend::new().with_bound("find \"/srv\"\n", empty_program());
        let config = Config::default();
        let compiled = frontend.compile("  find \"/srv\"", None, &config).unwrap();
        assert_eq!(compiled.bound, empty_program());
        assert_eq!(frontend.len(), 1);
        assert!(!frontend.is_empty());
        let missing = frontend.parse("find \"/srv\"", None, &config).unwrap_err();
        assert_eq!(missing.code, ErrorCode::Unsupported);
    }

    #[test]
    fn syntax_trees_parse_print_and_bind() {
        let frontend = FixtureFrontend::new().with_trees("list", ast(), empty_program());
        let config = Config::default();
        let corrected = frontend.correct("list", None, &config).unwrap();
        assert_eq!(corrected.ast, ast());
        assert_eq!(frontend.print(&ast(), SyntaxVariant::A).unwrap(), "list");
        assert_eq!(frontend.bind(&ast(), &config).unwrap(), empty_program());
        let compiled = Frontend::compile(&frontend, "list", None, &config).unwrap();
        assert_eq!(compiled.bound, empty_program());
    }

    #[test]
    fn error_fixtures_fail() {
        let error = Error::new(ErrorCode::Ambiguous, "ambiguous");
        let frontend = FixtureFrontend::new().with_error("fnd", error.clone());
        let config = Config::default();
        assert_eq!(frontend.compile("fnd", None, &config).unwrap_err(), error);
        assert_eq!(frontend.parse("fnd", None, &config).unwrap_err(), error);
        assert!(frontend.describe("core", Language::En).is_err());
        assert!(frontend.help("find", Language::En).is_err());
        assert!(frontend.bind(&ast(), &config).is_err());
        assert!(frontend.print(&ast(), SyntaxVariant::A).is_err());
    }

    #[test]
    fn directories_load_program_and_tree_files() {
        let temp = tempfile::tempdir().unwrap();
        let bound = serde_json::to_vec(&empty_program()).unwrap();
        fs::write(temp.path().join("one.nurt"), "find \"/srv\"\n").unwrap();
        fs::write(temp.path().join("one.bound.json"), &bound).unwrap();
        fs::write(temp.path().join("two.nurt"), "list").unwrap();
        fs::write(temp.path().join("two.bound.json"), &bound).unwrap();
        fs::write(
            temp.path().join("two.ast.json"),
            serde_json::to_vec(&ast()).unwrap(),
        )
        .unwrap();
        let frontend = FixtureFrontend::load_dir(temp.path()).unwrap();
        assert_eq!(frontend.len(), 2);
        assert!(frontend.parse("list", None, &Config::default()).is_ok());
        fs::write(temp.path().join("three.nurt"), "count").unwrap();
        fs::write(temp.path().join("three.bound.json"), b"{}").unwrap();
        let error = FixtureFrontend::load_dir(temp.path()).unwrap_err();
        assert_eq!(error.code, ErrorCode::AstSchema);
        let missing = FixtureFrontend::load_dir(&temp.path().join("absent")).unwrap_err();
        assert_eq!(missing.code, ErrorCode::Io);
    }
}
