// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-cli::tool_definition
// Responsibility:   `nurt schema tool` with options: the definition of the `nurt` tool in the format of an AI platform.
// Dependencies:     nurt_contracts, nurt_core, nurt_tooldef, serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! `nurt schema tool` with options: the definition of the `nurt` tool in the
//! format of an AI platform.
//!
//! Without options the command prints the published schema `nurt-tool/1`, as
//! every other name of `schema` does. With `--format`, `--language` or
//! `--groups` it renders the definition with `nurt-tooldef`, from the
//! description texts and the revision of the catalog of the engine.

use nurt_contracts::envelope::Language;
use nurt_contracts::error::{Error, ErrorCode};
use nurt_core::Engine;
use nurt_tooldef::{GroupSelection, RenderOptions, ToolDefError, ToolFormat};
use serde_json::{Value as Json, json};

use crate::app::CliError;

/// Options of `schema tool`.
pub(crate) const FLAGS: &[&str] = &["--format", "--language", "--groups"];

/// Group that the core description of the language covers.
const CORE_GROUP: &str = nurt_tooldef::CORE_GROUP;

/// Values of the options of `schema tool`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ToolOptions {
    pub(crate) format: ToolFormat,
    pub(crate) language: Language,
    pub(crate) groups: GroupSelection,
}

impl ToolOptions {
    /// Parses the options that follow `schema tool`: `--format <format>`
    /// (default `json-schema`), `--language en|pl` (default `en`) and
    /// `--groups core|all|<group>,<group>...` (default `core`). The value
    /// follows the option as a separate argument or after `=`.
    ///
    /// # Errors
    ///
    /// A usage error for an unknown option, a missing value or an argument
    /// that is not an option; `E_UNKNOWN_VALUE` for a value that is not one of
    /// the accepted ones.
    pub(crate) fn parse(args: &[String]) -> Result<Self, CliError> {
        let mut options = Self {
            format: ToolFormat::JsonSchema,
            language: Language::En,
            groups: GroupSelection::Core,
        };
        let mut words = args.iter();
        while let Some(word) = words.next() {
            let (name, inline) = match word.split_once('=') {
                Some((name, value)) => (name, Some(value.to_owned())),
                None => (word.as_str(), None),
            };
            if !word.starts_with("--") {
                return Err(CliError::Usage(format!(
                    "`schema tool` takes options only, found `{word}`"
                )));
            }
            if !FLAGS.contains(&name) {
                return Err(CliError::Usage(format!("unknown option `{name}`")));
            }
            let value = match inline {
                Some(value) => value,
                None => words
                    .next()
                    .cloned()
                    .ok_or_else(|| CliError::Usage(format!("option `{name}` needs a value")))?,
            };
            match name {
                "--format" => options.format = parse_format(&value)?,
                "--language" => options.language = parse_language(&value)?,
                _ => options.groups = parse_groups(&value)?,
            }
        }
        Ok(options)
    }
}

fn parse_format(value: &str) -> Result<ToolFormat, CliError> {
    value
        .parse()
        .map_err(|_| unknown_value("format", value, ToolFormat::ALL.iter().map(|f| f.name())))
}

fn parse_language(value: &str) -> Result<Language, CliError> {
    Language::ALL
        .iter()
        .copied()
        .find(|language| language.as_str() == value)
        .ok_or_else(|| {
            unknown_value(
                "language",
                value,
                Language::ALL.iter().map(|language| language.as_str()),
            )
        })
}

fn parse_groups(value: &str) -> Result<GroupSelection, CliError> {
    value.parse().map_err(|error: ToolDefError| {
        CliError::from(Error::new(ErrorCode::UnknownValue, error.to_string()).with_found(value))
    })
}

fn unknown_value<'a>(what: &str, found: &str, known: impl Iterator<Item = &'a str>) -> CliError {
    let known: Vec<&str> = known.collect();
    Error::new(
        ErrorCode::UnknownValue,
        format!("unknown {what} `{found}`; known: {}", known.join(", ")),
    )
    .with_found(found)
    .with_detail("accepted", json!(known))
    .into()
}

/// Renders the definition of the `nurt` tool.
///
/// # Errors
///
/// The error of the engine for a description that cannot be read,
/// `E_UNKNOWN_VALUE` for a group that is not in the catalog, and a failure
/// for a definition that cannot be built.
pub(crate) fn render(engine: &Engine, options: &ToolOptions) -> Result<String, CliError> {
    let mut render = RenderOptions::new(description(engine, CORE_GROUP, options.language)?)
        .with_language(options.language)
        .with_groups(options.groups.clone())
        .with_catalog_version(engine.catalog_rev());
    for group in selected_groups(engine, &options.groups)? {
        let text = description(engine, &group, options.language)?;
        render = render.with_group_description(group, text);
    }
    nurt_tooldef::render(options.format, &render)
        .map_err(|error| CliError::Failure(format!("cannot build the tool definition: {error}")))
}

/// The groups of the catalog other than the core group, in catalog order.
fn catalog_groups(engine: &Engine) -> Result<Vec<String>, CliError> {
    let catalog = engine.catalog()?;
    Ok(catalog
        .get("groups")
        .and_then(Json::as_array)
        .into_iter()
        .flatten()
        .filter_map(|group| group.get("name").and_then(Json::as_str))
        .filter(|name| *name != CORE_GROUP)
        .map(str::to_owned)
        .collect())
}

/// The groups whose descriptions the selection needs.
fn selected_groups(engine: &Engine, selection: &GroupSelection) -> Result<Vec<String>, CliError> {
    let known = catalog_groups(engine)?;
    match selection {
        GroupSelection::Only(listed) => {
            let mut groups = Vec::new();
            for group in listed.iter().filter(|group| *group != CORE_GROUP) {
                if !known.contains(group) {
                    return Err(unknown_value(
                        "group",
                        group,
                        known.iter().map(String::as_str),
                    ));
                }
                groups.push(group.clone());
            }
            Ok(groups)
        }
        GroupSelection::All => Ok(known),
        _ => Ok(Vec::new()),
    }
}

/// The text of the description of a group, in a language.
fn description(engine: &Engine, group: &str, language: Language) -> Result<String, CliError> {
    let description = engine.describe(group, Some(language))?;
    description
        .get("text")
        .and_then(Json::as_str)
        .map(str::to_owned)
        .ok_or_else(|| CliError::Failure(format!("the description of `{group}` has no text")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(text: &str) -> Vec<String> {
        text.split_whitespace().map(str::to_owned).collect()
    }

    #[test]
    fn options_default_to_the_documented_values() {
        let options = ToolOptions::parse(&[]).unwrap();
        assert_eq!(options.format, ToolFormat::JsonSchema);
        assert_eq!(options.language, Language::En);
        assert_eq!(options.groups, GroupSelection::Core);
    }

    #[test]
    fn options_take_a_separate_or_an_inline_value() {
        let options = ToolOptions::parse(&words(
            "--format openai-strict --language=pl --groups core,git",
        ))
        .unwrap();
        assert_eq!(options.format, ToolFormat::OpenAiChatStrict);
        assert_eq!(options.language, Language::Pl);
        assert_eq!(options.groups, GroupSelection::Only(vec!["git".to_owned()]));
        assert_eq!(
            ToolOptions::parse(&words("--groups all")).unwrap().groups,
            GroupSelection::All
        );
    }

    #[test]
    fn wrong_options_are_usage_errors_and_wrong_values_are_unknown_values() {
        for text in ["--verbose", "ast", "--format"] {
            assert!(
                matches!(ToolOptions::parse(&words(text)), Err(CliError::Usage(_))),
                "{text}"
            );
        }
        for text in ["--format claude", "--language de", "--groups Git!"] {
            let Err(CliError::Language(error)) = ToolOptions::parse(&words(text)) else {
                panic!("{text}");
            };
            assert_eq!(error.code, ErrorCode::UnknownValue, "{text}");
        }
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
