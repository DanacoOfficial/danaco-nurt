// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-langserver::assist
// Responsibility:   Completion and hover from the command catalog and the help.
// Dependencies:     lsp_types, nurt_bind, nurt_contracts, nurt_help, nurt_syntax
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Completion and hover from the command catalog and the help.

use std::ops::Range;

use lsp_types::{
    CompletionItem, CompletionItemKind, CompletionTextEdit, Diagnostic, Documentation, Hover,
    HoverContents, MarkupContent, MarkupKind, NumberOrString, Position, TextEdit,
};
use nurt_contracts::spec::{ParamSpec, StageSpec};
use nurt_help::catalog::Catalog as HelpCatalog;
use nurt_help::help::{HelpQuery, TopicKind, render};
use nurt_syntax::lex::Token;
use nurt_syntax::parse::Variant;

use crate::catalog::{self, Commands};
use crate::context::{self, Place, Target};
use crate::fields::{self, InputField};
use crate::position::{Encoding, Lines};

/// Prefix of the sort text of a field, which places fields after the
/// parameters, whose sort texts are digits.
const FIELD_SORT_PREFIX: char = '~';

/// Prefix of the error codes that the help describes.
const ERROR_CODE_PREFIX: &str = "E_";

/// Document state shared by completion, hover and signatures.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Request<'a> {
    /// Text of the document.
    pub(crate) text: &'a str,
    /// Syntax variant of the document.
    pub(crate) variant: Variant,
    /// Position encoding of the session.
    pub(crate) encoding: Encoding,
    /// Command signatures.
    pub(crate) commands: &'a Commands,
    /// Catalog with the documentation, for the help shown on hover.
    pub(crate) help: Option<&'a HelpCatalog>,
    /// Diagnostics published for the document.
    pub(crate) diagnostics: &'a [Diagnostic],
}

/// Returns the completion items at `position`.
pub(crate) fn completion(request: Request<'_>, position: Position) -> Vec<CompletionItem> {
    let lines = Lines::new(request.text);
    let offset = lines.offset(position, request.encoding);
    let tokens = context::tokens(request.text, request.variant);
    let edit = |replace: Range<usize>, text: String| {
        Some(CompletionTextEdit::Edit(TextEdit::new(
            lines.range(replace, request.encoding),
            text,
        )))
    };
    let input_fields = |replace: &Range<usize>| {
        let upstream = context::upstream(request.text, &tokens, offset, request.variant);
        fields::input_fields(request.commands, &upstream)
            .into_iter()
            .map(|input| field_item(input, edit(replace.clone(), input.field.name.clone())))
            .collect::<Vec<_>>()
    };
    match context::place(request.text, &tokens, offset, request.variant) {
        Place::Command { replace } => request
            .commands
            .iter()
            .map(|spec| command_item(spec, edit(replace.clone(), spec.name.clone())))
            .collect(),
        Place::Parameter {
            command,
            used,
            replace,
        } => {
            let Some(spec) = request.commands.get(&command) else {
                return Vec::new();
            };
            let separator = catalog::label_separator(request.variant);
            let mut items: Vec<CompletionItem> = spec
                .params
                .iter()
                .enumerate()
                .filter(|(_, param)| !used.contains(&param.name))
                .map(|(index, param)| {
                    let text = format!("{}{separator}", param.name);
                    param_item(index, param, edit(replace.clone(), text))
                })
                .collect();
            let names_fields = spec
                .positional()
                .iter()
                .any(|param| fields::takes_field(&param.ty));
            if names_fields {
                items.extend(input_fields(&replace));
            }
            items
        }
        Place::Value {
            command,
            param,
            replace,
        } => {
            let Some(param) = request
                .commands
                .get(&command)
                .and_then(|spec| spec.param(&param))
            else {
                return Vec::new();
            };
            let mut items: Vec<CompletionItem> = catalog::enum_values(param)
                .into_iter()
                .map(|value| CompletionItem {
                    label: value.to_owned(),
                    kind: Some(CompletionItemKind::ENUM_MEMBER),
                    detail: Some(param.ty.to_string()),
                    text_edit: edit(replace.clone(), value.to_owned()),
                    ..CompletionItem::default()
                })
                .collect();
            if fields::takes_field(&param.ty) {
                items.extend(input_fields(&replace));
            }
            items
        }
        Place::Field { replace, .. } => input_fields(&replace),
        Place::Reference { names, replace } => names
            .into_iter()
            .map(|name| CompletionItem {
                label: name.clone(),
                kind: Some(CompletionItemKind::VARIABLE),
                text_edit: edit(replace.clone(), name),
                ..CompletionItem::default()
            })
            .collect(),
        Place::Other => Vec::new(),
    }
}

/// Returns the description of the command, parameter or error at
/// `position`.
pub(crate) fn hover(request: Request<'_>, position: Position) -> Option<Hover> {
    let lines = Lines::new(request.text);
    let offset = lines.offset(position, request.encoding);
    let tokens = context::tokens(request.text, request.variant);
    name_hover(request, &lines, &tokens, offset).or_else(|| error_hover(request, position))
}

fn name_hover(
    request: Request<'_>,
    lines: &Lines<'_>,
    tokens: &[Token],
    offset: usize,
) -> Option<Hover> {
    let (value, range) = match context::target(request.text, tokens, offset, request.variant)? {
        Target::Command { name, range } => {
            let spec = request.commands.get(&name)?;
            (command_text(spec, request.variant, request.help), range)
        }
        Target::Parameter {
            command,
            param,
            range,
        } => {
            let spec = request.commands.get(&command)?;
            let param = spec.param(&param)?;
            (catalog::param_markdown(spec, param, request.variant), range)
        }
    };
    Some(markdown(value, Some(lines.range(range, request.encoding))))
}

/// Describes a command: its call form and its help when the help catalog
/// knows it, otherwise its signature from the catalog.
fn command_text(spec: &StageSpec, variant: Variant, help: Option<&HelpCatalog>) -> String {
    let query = HelpQuery::topic(spec.name.clone()).with_kind(TopicKind::Command);
    match help.map(|help| render(&query, help)) {
        Some(Ok(answer)) => format!(
            "```nurt\n{}\n```\n\n```text\n{}\n```\n",
            catalog::signature(spec, variant),
            answer.body
        ),
        _ => catalog::command_markdown(spec, variant),
    }
}

/// Describes the error code of the diagnostic at `position`.
fn error_hover(request: Request<'_>, position: Position) -> Option<Hover> {
    let help = request.help?;
    let (code, diagnostic) = request.diagnostics.iter().find_map(|diagnostic| {
        let range = diagnostic.range;
        let NumberOrString::String(code) = diagnostic.code.as_ref()? else {
            return None;
        };
        let inside = range.start <= position && position <= range.end;
        (inside && code.starts_with(ERROR_CODE_PREFIX)).then_some((code, diagnostic))
    })?;
    let query = HelpQuery::topic(code.clone()).with_kind(TopicKind::Error);
    let answer = render(&query, help).ok()?;
    Some(markdown(
        format!("```text\n{}\n```\n", answer.body),
        Some(diagnostic.range),
    ))
}

fn markdown(value: String, range: Option<lsp_types::Range>) -> Hover {
    Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value,
        }),
        range,
    }
}

fn command_item(spec: &StageSpec, text_edit: Option<CompletionTextEdit>) -> CompletionItem {
    CompletionItem {
        label: spec.name.clone(),
        kind: Some(CompletionItemKind::FUNCTION),
        detail: Some(spec.group.clone()),
        documentation: Some(Documentation::String(spec.summary.clone())),
        text_edit,
        ..CompletionItem::default()
    }
}

/// Completion item of the parameter at `index` of its signature; the sort
/// text keeps the signature order.
fn param_item(
    index: usize,
    param: &ParamSpec,
    text_edit: Option<CompletionTextEdit>,
) -> CompletionItem {
    CompletionItem {
        label: param.name.clone(),
        kind: Some(CompletionItemKind::PROPERTY),
        detail: Some(catalog::param_detail(param)),
        documentation: Some(Documentation::String(param.summary.clone())),
        sort_text: Some(format!("{index:04}")),
        filter_text: Some(param.name.clone()),
        text_edit,
        ..CompletionItem::default()
    }
}

fn field_item(input: InputField<'_>, text_edit: Option<CompletionTextEdit>) -> CompletionItem {
    CompletionItem {
        label: input.field.name.clone(),
        kind: Some(CompletionItemKind::FIELD),
        detail: Some(format!("field of {}", input.kind)),
        documentation: Some(Documentation::String(input.field.summary.clone())),
        sort_text: Some(format!("{FIELD_SORT_PREFIX}{}", input.field.name)),
        text_edit,
        ..CompletionItem::default()
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use lsp_types::{DiagnosticSeverity, Range as Span};
    use nurt_bind::INDEX_FILE;
    use nurt_help::docs::Docs;

    use super::*;
    use crate::catalog::tests::sample;

    fn labels(items: &[CompletionItem]) -> Vec<&str> {
        items.iter().map(|item| item.label.as_str()).collect()
    }

    fn request<'a>(text: &'a str, variant: Variant, commands: &'a Commands) -> Request<'a> {
        Request {
            text,
            variant,
            encoding: Encoding::Utf16,
            commands,
            help: None,
            diagnostics: &[],
        }
    }

    fn new_text(item: &CompletionItem) -> (&str, lsp_types::Range) {
        match &item.text_edit {
            Some(CompletionTextEdit::Edit(edit)) => (edit.new_text.as_str(), edit.range),
            other => panic!("unexpected edit {other:?}"),
        }
    }

    /// The help catalog of the catalog files of the workspace, without
    /// documentation pages.
    fn help_catalog() -> HelpCatalog {
        let dir = Path::new(
            std::env::var("CARGO_MANIFEST_DIR")
                .as_deref()
                .unwrap_or(env!("CARGO_MANIFEST_DIR")),
        )
        .join("../../catalog");
        let index = fs::read_to_string(dir.join(INDEX_FILE)).unwrap();
        let mut files: Vec<(String, String)> = fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "toml")
                    && path.file_name().is_some_and(|name| name != INDEX_FILE)
            })
            .map(|path| {
                let name = path.file_name().unwrap().to_string_lossy().into_owned();
                (name, fs::read_to_string(&path).unwrap())
            })
            .collect();
        files.sort();
        let sources: Vec<(&str, &str)> = files
            .iter()
            .map(|(name, text)| (name.as_str(), text.as_str()))
            .collect();
        HelpCatalog::from_sources(&index, &sources, Docs::empty()).unwrap()
    }

    #[test]
    fn completes_command_names_with_their_group() {
        let commands = sample();
        let items = completion(
            request("count |> so", Variant::A, &commands),
            Position::new(0, 11),
        );
        assert_eq!(labels(&items), ["count", "find", "sort"]);
        assert_eq!(items[2].detail.as_deref(), Some("core"));
        let (text, range) = new_text(&items[2]);
        assert_eq!(text, "sort");
        assert_eq!((range.start.character, range.end.character), (9, 11));
    }

    #[test]
    fn completes_parameters_not_yet_written() {
        let commands = sample();
        let items = completion(
            request("sort size ", Variant::A, &commands),
            Position::new(0, 10),
        );
        assert_eq!(labels(&items), ["by", "order"]);
        assert_eq!(new_text(&items[1]).0, "order: ");
        assert_eq!(items[1].sort_text.as_deref(), Some("0001"));
        let named = completion(
            request("sort by=size ", Variant::C, &commands),
            Position::new(0, 13),
        );
        assert_eq!(labels(&named), ["order"]);
        assert_eq!(new_text(&named[0]).0, "order=");
    }

    #[test]
    fn completes_fields_of_the_input_records() {
        let commands = sample();
        let text = "find \"/srv\" |> sort ";
        let items = completion(request(text, Variant::A, &commands), Position::new(0, 20));
        let size = items.iter().find(|item| item.label == "size").unwrap();
        assert_eq!(size.kind, Some(CompletionItemKind::FIELD));
        assert_eq!(size.detail.as_deref(), Some("field of entry"));
        assert_eq!(size.sort_text.as_deref(), Some("~size"));
        assert!(labels(&items).starts_with(&["by", "order"]));
        let value = "find \"/srv\" |> sort by: ";
        let values = completion(request(value, Variant::A, &commands), Position::new(0, 24));
        assert!(labels(&values).contains(&"path"));
        let logical = "find \"/srv\" |> sort size and ";
        let fields = completion(
            request(logical, Variant::A, &commands),
            Position::new(0, 29),
        );
        assert!(
            fields
                .iter()
                .all(|item| item.kind == Some(CompletionItemKind::FIELD))
        );
        assert!(labels(&fields).contains(&"name"));
        let headless = completion(request("sort ", Variant::A, &commands), Position::new(0, 5));
        assert_eq!(labels(&headless), ["by", "order"]);
    }

    #[test]
    fn completes_enumeration_values_and_references() {
        let commands = sample();
        let values = completion(
            request("sort size order: ", Variant::A, &commands),
            Position::new(0, 17),
        );
        assert_eq!(labels(&values), ["asc", "desc"]);
        let text = "let all = count\n@";
        let references = completion(request(text, Variant::A, &commands), Position::new(1, 1));
        assert_eq!(labels(&references), ["all"]);
        for (text, character) in [("frobnicate \"/srv\" ", 18), ("frobnicate x: ", 14)] {
            let unknown = completion(
                request(text, Variant::A, &commands),
                Position::new(0, character),
            );
            assert!(unknown.is_empty(), "{text}");
        }
        let other = completion(
            request("filter size > ", Variant::A, &commands),
            Position::new(0, 14),
        );
        assert!(other.is_empty());
    }

    #[test]
    fn describes_commands_and_parameters_on_hover() {
        let commands = sample();
        let text = "count |> sort size order: desc";
        let command = hover(request(text, Variant::A, &commands), Position::new(0, 10)).unwrap();
        let HoverContents::Markup(markup) = &command.contents else {
            panic!("expected markup")
        };
        assert!(markup.value.starts_with("```nurt\nsort <by>\n```"));
        assert_eq!(command.range.map(|range| range.start.character), Some(9));
        let param = hover(request(text, Variant::A, &commands), Position::new(0, 20)).unwrap();
        let HoverContents::Markup(markup) = &param.contents else {
            panic!("expected markup")
        };
        assert!(markup.value.contains("Parameter of `sort`"));
        assert!(hover(request(text, Variant::A, &commands), Position::new(0, 15)).is_none());
        assert!(
            hover(
                request("frobnicate \"/srv\"", Variant::A, &commands),
                Position::new(0, 1)
            )
            .is_none()
        );
    }

    #[test]
    fn describes_commands_and_error_codes_from_the_help() {
        let commands = sample();
        let help = help_catalog();
        let diagnostic = Diagnostic {
            range: Span::new(Position::new(0, 6), Position::new(0, 7)),
            severity: Some(DiagnosticSeverity::ERROR),
            code: Some(NumberOrString::String("E_SYNTAX".to_owned())),
            ..Diagnostic::default()
        };
        let correction = Diagnostic {
            code: Some(NumberOrString::String("L03".to_owned())),
            ..diagnostic.clone()
        };
        let diagnostics = [correction, diagnostic];
        let request = Request {
            help: Some(&help),
            diagnostics: &diagnostics,
            ..request("count ) |> sort size", Variant::A, &commands)
        };
        let command = hover(request, Position::new(0, 1)).unwrap();
        let HoverContents::Markup(markup) = &command.contents else {
            panic!("expected markup")
        };
        assert!(
            markup
                .value
                .starts_with("```nurt\ncount\n```\n\n```text\ncount — "),
            "{}",
            markup.value
        );
        let error = hover(request, Position::new(0, 6)).unwrap();
        let HoverContents::Markup(markup) = &error.contents else {
            panic!("expected markup")
        };
        assert!(
            markup.value.starts_with("```text\nE_SYNTAX"),
            "{}",
            markup.value
        );
        assert_eq!(error.range, Some(diagnostics[1].range));
        assert!(hover(request, Position::new(0, 8)).is_none());
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
