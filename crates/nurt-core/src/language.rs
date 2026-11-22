// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Language front end assembled from the language crates.
//!
//! [`LanguageFrontend`] implements [`Frontend`] over `nurt-syntax` (parsing and
//! printing), `nurt-correct` and `nurt-bind` (correction and binding to the
//! catalog) and `nurt-help` (descriptions and help). It holds the catalog and
//! the correction tables, which are loaded once.

use nurt_bind::{BindContext, Catalog as BindCatalog, bind_with_correction};
use nurt_contracts::ast::AstDocument;
use nurt_contracts::bound::BoundDocument;
use nurt_contracts::config::Config;
use nurt_contracts::envelope::Language;
use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::kinds::SyntaxVariant;
use nurt_correct::tables::{Tables, load_embedded};
use nurt_help::catalog::Catalog as HelpCatalog;
use nurt_help::describe::{Layer, Length, layer};
use nurt_help::docs::Docs;
use nurt_help::help::{HelpQuery, render};
use nurt_syntax::parse::{ParseOptions, parse};
use nurt_syntax::print::try_print;
use serde_json::json;

use crate::frontend::{Compiled, Corrected, Frontend, ast_hash};

/// Language front end of the executable.
#[derive(Debug)]
pub struct LanguageFrontend {
    bind_catalog: BindCatalog,
    help_catalog: HelpCatalog,
    tables: Tables,
}

fn internal(what: &str, error: impl std::fmt::Display) -> Error {
    Error::new(ErrorCode::Internal, format!("{what}: {error}"))
}

impl LanguageFrontend {
    /// Loads the embedded catalog and the correction tables.
    ///
    /// # Errors
    ///
    /// Returns `E_INTERNAL` when the embedded data is invalid.
    pub fn new() -> Result<Self, Error> {
        let (index, files) = nurt_catalog::embedded_sources();
        let texts: Vec<&str> = files.iter().map(|(_, text)| *text).collect();
        let bind_catalog = BindCatalog::from_toml(index, &texts)
            .map_err(|error| internal("embedded catalog", error))?;
        let help_catalog = HelpCatalog::from_sources(index, files, Docs::empty())
            .map_err(|error| internal("embedded catalog", error))?;
        let tables = load_embedded().map_err(|error| internal("correction tables", error))?;
        Ok(Self {
            bind_catalog,
            help_catalog,
            tables,
        })
    }

    fn outcome(
        &self,
        text: &str,
        variant: Option<SyntaxVariant>,
        config: &Config,
    ) -> Result<nurt_bind::Outcome, Error> {
        bind_with_correction(
            text,
            variant.unwrap_or(config.syntax.variant),
            &self.bind_catalog,
            &self.tables,
            config,
            &BindContext::for_host(),
        )
    }
}

impl Frontend for LanguageFrontend {
    fn parse(
        &self,
        text: &str,
        variant: Option<SyntaxVariant>,
        config: &Config,
    ) -> Result<AstDocument, Error> {
        parse(
            text,
            variant.unwrap_or(config.syntax.variant),
            &ParseOptions::default(),
        )
        .map_err(Error::from)
    }

    fn correct(
        &self,
        text: &str,
        variant: Option<SyntaxVariant>,
        config: &Config,
    ) -> Result<Corrected, Error> {
        let outcome = self.outcome(text, variant, config)?;
        let ran = (!outcome.corrections.is_empty()).then_some(outcome.canonical);
        Ok(Corrected {
            ast: outcome.surface,
            corrections: outcome.corrections.into_iter().map(Into::into).collect(),
            ran,
        })
    }

    fn bind(&self, ast: &AstDocument, _config: &Config) -> Result<BoundDocument, Error> {
        nurt_bind::bind(ast, &self.bind_catalog, &BindContext::for_host()).map_err(Error::from)
    }

    fn print(&self, ast: &AstDocument, variant: SyntaxVariant) -> Result<String, Error> {
        try_print(ast, variant).map_err(|error| internal("printing", error))
    }

    fn describe(&self, subject: &str, language: Language) -> Result<serde_json::Value, Error> {
        let (kind, name) = match subject {
            "" | "core" => (Layer::Core, ""),
            name if self.help_catalog.group(name).is_some() => (Layer::Group, name),
            name => (Layer::Reference, name),
        };
        let description = layer(&self.help_catalog, kind, name, language, Length::Default)?;
        Ok(json!({
            "text": description.text,
            "tokens": description.tokens,
            "estimate": description.estimate,
        }))
    }

    fn help(&self, topic: &str, language: Language) -> Result<String, Error> {
        let query = if topic.is_empty() {
            HelpQuery::new()
        } else {
            HelpQuery::topic(topic)
        };
        render(&query.with_language(language), &self.help_catalog).map(|help| help.body)
    }

    fn compile(
        &self,
        text: &str,
        variant: Option<SyntaxVariant>,
        config: &Config,
    ) -> Result<Compiled, Error> {
        let outcome = self.outcome(text, variant, config)?;
        let ran = (!outcome.corrections.is_empty()).then_some(outcome.canonical);
        Ok(Compiled {
            bound: outcome.bound,
            ast_hash: ast_hash(&outcome.surface),
            corrections: outcome.corrections.into_iter().map(Into::into).collect(),
            ran,
        })
    }
}
