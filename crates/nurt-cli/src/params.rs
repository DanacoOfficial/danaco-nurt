// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-cli::params
// Responsibility:   Parameters of a program file on the command line.
// Dependencies:     nurt_contracts, nurt_core, serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Parameters of a program file on the command line.
//!
//! `nurt run cleanup.nurt dir: "/srv/app" days: 7 dry` passes `dir`, `days`
//! and `dry`. A value is given as `name: value` (two arguments) or
//! `name:value`; a name without a value is a flag of type `bool` and means
//! `true`, `false` is written `name: false`. Every value is passed as text and
//! read by the declared type before anything runs.

use nurt_contracts::ast::{AstDocument, Program, Statement};
use nurt_contracts::kinds::SyntaxVariant;
use nurt_core::Engine;
use serde_json::{Map, Value as Json};

use crate::app::CliError;

/// Whether the text is the name of a parameter.
fn is_name(text: &str) -> bool {
    let mut characters = text.chars();
    characters
        .next()
        .is_some_and(|first| first.is_ascii_lowercase())
        && characters.all(|next| next.is_ascii_lowercase() || next.is_ascii_digit() || next == '_')
}

/// Reads the arguments after the file into the values of the parameters;
/// `None` when there are none.
///
/// # Errors
///
/// Returns a usage error for an argument that is no `name: value` or flag, a
/// name without a value at the end of the line and a name given twice.
pub fn from_words(words: &[String]) -> Result<Option<Map<String, Json>>, CliError> {
    if words.is_empty() {
        return Ok(None);
    }
    let mut values = Map::new();
    let mut index = 0;
    while index < words.len() {
        let word = &words[index];
        let (name, value, used) = if let Some(name) = word.strip_suffix(':').filter(|n| is_name(n))
        {
            let value = words.get(index + 1).ok_or_else(|| {
                CliError::Usage(format!("`{name}:` has no value; write `{name}: <value>`"))
            })?;
            (name.to_owned(), Json::String(value.clone()), 2)
        } else if let Some((name, value)) = word.split_once(':').filter(|(name, value)| {
            is_name(name) && name.len() > 1 && !value.starts_with(['/', '\\'])
        }) {
            (name.to_owned(), Json::String(value.to_owned()), 1)
        } else if is_name(word) {
            (word.clone(), Json::Bool(true), 1)
        } else {
            return Err(CliError::Usage(format!(
                "expected `name: value` or the name of a flag, found `{word}`"
            )));
        };
        if values.insert(name.clone(), value).is_some() {
            return Err(CliError::Usage(format!("`{name}` is given twice")));
        }
        index += used;
    }
    Ok(Some(values))
}

/// The declaration of the parameters of a program as program text, written
/// by the engine in the notation of the program: the lines between the braces
/// of `params { … }`, one parameter each. Empty when the program declares no
/// parameters.
///
/// # Errors
///
/// Returns the error of the engine when the declaration cannot be written.
pub fn declaration(
    engine: &Engine,
    document: &AstDocument,
    variant: Option<SyntaxVariant>,
) -> Result<String, CliError> {
    let Some(params) = document
        .program
        .statements
        .iter()
        .find(|statement| matches!(statement, Statement::Params(_)))
    else {
        return Ok(String::new());
    };
    let alone = AstDocument::new(Program {
        statements: vec![params.clone()],
    });
    let text = engine.print(&alone, variant)?;
    let lines: Vec<&str> = text.lines().collect();
    let inner = lines
        .get(1..lines.len().saturating_sub(1))
        .unwrap_or_default();
    Ok(inner
        .iter()
        .map(|line| line.trim())
        .collect::<Vec<_>>()
        .join("\n"))
}

/// The declaration of the parameters of a program, as `--help` prints it;
/// `declaration` is the text of [`declaration`].
#[must_use]
pub fn help(file: &str, document: &AstDocument, declaration: &str) -> String {
    let declared = document
        .program
        .statements
        .iter()
        .find_map(|statement| match statement {
            Statement::Params(params) => Some(&params.params),
            _ => None,
        });
    let Some(params) = declared else {
        return format!("{file} declares no parameters.");
    };
    let mut usage = format!("Usage: nurt run {file}");
    for param in params {
        if param.default.is_some() {
            usage.push_str(&format!(" [{}: <{}>]", param.name, param.ty));
        } else if param.ty == "bool" {
            usage.push_str(&format!(" {}", param.name));
        } else {
            usage.push_str(&format!(" {}: <{}>", param.name, param.ty));
        }
    }
    let lines: Vec<String> = declaration
        .lines()
        .map(|line| format!("  {line}"))
        .collect();
    format!(
        "{usage}\n\nParameters of {file}:\n{}\n\nA parameter of type bool is given without a value (`dry`) for true and as `dry: false` for false.",
        lines.join("\n")
    )
}

#[cfg(test)]
mod tests {
    use nurt_contracts::ast::{Expr, Param, Params};
    use serde_json::json;

    use super::*;

    fn words(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| (*item).to_owned()).collect()
    }

    fn param(name: &str, ty: &str, default: Option<Expr>) -> Param {
        Param {
            name: name.to_owned(),
            ty: ty.to_owned(),
            default,
            span: None,
        }
    }

    fn document(params: Vec<Param>) -> AstDocument {
        AstDocument::new(Program {
            statements: vec![Statement::Params(Params { params, span: None })],
        })
    }

    #[test]
    fn no_words_give_no_values() {
        assert_eq!(from_words(&[]).unwrap(), None);
    }

    #[test]
    fn a_name_with_a_colon_takes_the_next_word_and_a_bare_name_is_true() {
        let values = from_words(&words(&["dir:", "/srv/app", "days:7", "dry"]))
            .unwrap()
            .unwrap();
        assert_eq!(
            Json::Object(values),
            json!({"dir": "/srv/app", "days": "7", "dry": true})
        );
    }

    #[test]
    fn false_is_written_out() {
        let values = from_words(&words(&["dry:", "false"])).unwrap().unwrap();
        assert_eq!(values["dry"], json!("false"));
    }

    #[test]
    fn a_drive_path_is_a_value_not_a_name() {
        let error = from_words(&words(&["C:/proj"])).unwrap_err();
        assert!(matches!(error, CliError::Usage(_)));
    }

    #[test]
    fn a_missing_value_and_a_repeated_name_are_usage_errors() {
        assert!(matches!(
            from_words(&words(&["dir:"])).unwrap_err(),
            CliError::Usage(_)
        ));
        assert!(matches!(
            from_words(&words(&["days:", "1", "days:", "2"])).unwrap_err(),
            CliError::Usage(_)
        ));
        assert!(matches!(
            from_words(&words(&["Not-A-Name"])).unwrap_err(),
            CliError::Usage(_)
        ));
    }

    #[test]
    fn help_prints_the_usage_and_the_declaration() {
        let days = Expr::Int { v: 30, span: None };
        let dry = Expr::Bool {
            v: false,
            span: None,
        };
        let document = document(vec![
            param("dir", "path", None),
            param("days", "int", Some(days)),
            param("dry", "bool", Some(dry)),
        ]);
        let text = help(
            "cleanup.nurt",
            &document,
            "dir: path\ndays: int = 30\ndry: bool = false",
        );
        assert!(
            text.contains("Usage: nurt run cleanup.nurt dir: <path> [days: <int>] [dry: <bool>]")
        );
        assert!(text.contains("  dir: path\n"));
        assert!(text.contains("  days: int = 30\n"));
        assert!(text.contains("  dry: bool = false"));
    }

    #[test]
    fn a_required_flag_is_written_by_its_name() {
        let text = help(
            "a.nurt",
            &document(vec![param("force", "bool", None)]),
            "force: bool",
        );
        assert!(text.contains("Usage: nurt run a.nurt force\n"));
    }

    #[test]
    fn help_of_a_file_without_parameters_says_so() {
        let plain = AstDocument::new(Program {
            statements: Vec::new(),
        });
        assert_eq!(
            help("plain.nurt", &plain, ""),
            "plain.nurt declares no parameters."
        );
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
