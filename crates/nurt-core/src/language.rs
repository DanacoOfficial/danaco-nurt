// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Language front end assembled from the language crates.
//!
//! [`LanguageFrontend`] implements [`Frontend`] over `nurt-syntax` (parsing and
//! printing), `nurt-correct` and `nurt-bind` (correction and binding to the
//! catalog) and `nurt-help` (descriptions and help). It holds the catalog and
//! the correction tables, which are loaded once.

use std::sync::Arc;

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
use nurt_help::help::{HelpQuery, TopicKind, render};
use nurt_syntax::fmt::convert;
use nurt_syntax::parse::{ParseOptions, parse};
use nurt_syntax::print::{PositionalNames, PrintError, name_positional, try_print};
use serde_json::json;

use crate::frontend::{Compiled, Corrected, Frontend, ast_hash};

/// Language front end of the executable.
#[derive(Debug)]
pub struct LanguageFrontend {
    bind_catalog: BindCatalog,
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
        let tables = load_embedded().map_err(|error| internal("correction tables", error))?;
        Ok(Self {
            bind_catalog,
            tables,
        })
    }

    /// The help catalog with the documentation compiled into the executable,
    /// loaded on first use.
    fn help_catalog(&self) -> Result<Arc<HelpCatalog>, Error> {
        HelpCatalog::embedded().map_err(|error| internal("embedded help", error))
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
            &bind_context(config),
        )
    }
}

/// Names of the positional parameters of the commands, from the catalog of
/// the help; variant C writes every argument with its name.
struct CatalogNames<'a>(&'a HelpCatalog);

impl PositionalNames for CatalogNames<'_> {
    fn name(&self, command: &str, index: usize) -> Option<String> {
        self.0
            .command(command)?
            .positional()
            .get(index)
            .map(|param| param.name.clone())
    }
}

/// Error of printing a program: a notation the variant cannot express is a
/// request the user can change, anything else is internal.
fn print_error(error: PrintError) -> Error {
    match error {
        PrintError::PositionalArgument { .. } | PrintError::UnnamedArgument { .. } => {
            Error::new(ErrorCode::Unsupported, error.to_string())
        }
        other => internal("printing", other),
    }
}

/// Context of binding for the host, with the call parameter `base` taken
/// from `scope.base` of the configuration of the request.
fn bind_context(config: &Config) -> BindContext {
    let context = BindContext::for_host();
    match config.scope.base.as_deref() {
        Some(base) => context.with_base(base),
        None => context,
    }
}

/// The topic of `help` without the quotes a program writes around it.
fn unquoted(topic: &str) -> &str {
    topic
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
        .unwrap_or(topic)
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

    fn bind(&self, ast: &AstDocument, config: &Config) -> Result<BoundDocument, Error> {
        nurt_bind::bind(ast, &self.bind_catalog, &bind_context(config)).map_err(Error::from)
    }

    fn print(&self, ast: &AstDocument, variant: SyntaxVariant) -> Result<String, Error> {
        if variant != SyntaxVariant::C {
            return try_print(ast, variant).map_err(print_error);
        }
        let catalog = self.help_catalog()?;
        let named = name_positional(ast, &CatalogNames(&catalog)).map_err(print_error)?;
        try_print(&named, variant).map_err(print_error)
    }

    fn format(
        &self,
        text: &str,
        from: Option<SyntaxVariant>,
        to: Option<SyntaxVariant>,
        config: &Config,
    ) -> Result<String, Error> {
        let from = from.unwrap_or(config.syntax.variant);
        let to = to.unwrap_or(config.syntax.variant);
        let formatted = if to == SyntaxVariant::C {
            let catalog = self.help_catalog()?;
            convert(text, from, to, Some(&CatalogNames(&catalog)))
        } else {
            convert(text, from, to, None)
        };
        formatted.map_err(Error::from)
    }

    fn describe(&self, subject: &str, language: Language) -> Result<serde_json::Value, Error> {
        let catalog = self.help_catalog()?;
        let (kind, name) = match subject {
            "" | "core" => (Layer::Core, ""),
            name if catalog.group(name).is_some() => (Layer::Group, name),
            name => (Layer::Reference, name),
        };
        let description = layer(&catalog, kind, name, language, Length::Default)?;
        Ok(json!({
            "text": description.text,
            "tokens": description.tokens,
            "estimate": description.estimate,
        }))
    }

    fn help(&self, topic: &str, language: Language) -> Result<String, Error> {
        self.help_query(topic, None, None, language)
    }

    fn help_query(
        &self,
        topic: &str,
        kind: Option<&str>,
        examples: Option<usize>,
        language: Language,
    ) -> Result<String, Error> {
        let topic = unquoted(topic.trim());
        let mut query = if topic.is_empty() {
            HelpQuery::new()
        } else {
            HelpQuery::topic(topic)
        };
        if let Some(name) = kind {
            let kind = TopicKind::from_name(name).ok_or_else(|| {
                Error::new(
                    ErrorCode::UnknownName,
                    format!("`{name}` is not a kind of help topic"),
                )
            })?;
            query = query.with_kind(kind);
        }
        if let Some(count) = examples {
            query = query.with_examples(count);
        }
        let catalog = self.help_catalog()?;
        render(&query.with_language(language), &catalog).map(|help| help.body)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_around_a_help_topic_are_removed() {
        assert_eq!(unquoted("\"commands\""), "commands");
        assert_eq!(unquoted("commands"), "commands");
        assert_eq!(unquoted("\""), "\"");
        assert_eq!(unquoted("\"\""), "");
    }

    #[test]
    fn help_answers_a_quoted_topic_with_the_embedded_documentation() {
        let frontend = LanguageFrontend::new().unwrap();
        let commands = frontend.help("\"commands\"", Language::En).unwrap();
        assert!(commands.contains("program.catalog — "));
        let page = frontend.help("replace", Language::En).unwrap();
        assert!(page.contains("examples:"));
    }

    #[test]
    fn help_query_honours_the_kind_and_the_number_of_examples() {
        let frontend = LanguageFrontend::new().unwrap();
        let plain = frontend.help("replace", Language::En).unwrap();
        let typed = frontend
            .help_query("replace", Some("command"), None, Language::En)
            .unwrap();
        assert_eq!(typed, plain);
        let none = frontend
            .help_query("replace", Some("command"), Some(0), Language::En)
            .unwrap();
        assert!(
            none.len() < plain.len(),
            "zero examples must shorten the page"
        );
        assert!(
            frontend
                .help_query("replace", Some("error"), None, Language::En)
                .is_err(),
            "a command is not an error code"
        );
        assert!(
            frontend
                .help_query("replace", Some("page"), None, Language::En)
                .is_err()
        );
    }

    #[test]
    fn a_relative_path_binds_against_the_base_of_the_configuration() {
        let frontend = LanguageFrontend::new().unwrap();
        let mut config = Config::default();
        let error = frontend
            .compile("read \"c.toml\"", None, &config)
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::PathRelative);
        config.scope.base = Some("/srv/app".to_owned());
        let compiled = frontend.compile("read \"c.toml\"", None, &config).unwrap();
        let bound = serde_json::to_string(&compiled.bound).unwrap();
        assert!(bound.contains("/srv/app/c.toml"), "{bound}");
    }

    #[test]
    fn describe_of_a_group_uses_the_documentation() {
        let frontend = LanguageFrontend::new().unwrap();
        let described = frontend.describe("core", Language::Pl).unwrap();
        assert!(
            described["text"]
                .as_str()
                .is_some_and(|text| !text.is_empty())
        );
    }
}
