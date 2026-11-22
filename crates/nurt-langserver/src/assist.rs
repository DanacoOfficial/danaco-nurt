// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Completion and hover from the command catalog.

use std::ops::Range;

use lsp_types::{
    CompletionItem, CompletionItemKind, CompletionTextEdit, Documentation, Hover, HoverContents,
    MarkupContent, MarkupKind, Position, TextEdit,
};
use nurt_contracts::spec::{ParamSpec, StageSpec};
use nurt_syntax::parse::Variant;

use crate::catalog::{self, Commands};
use crate::context::{self, Place, Target};
use crate::position::{Encoding, Lines};

/// Document state shared by completion and hover.
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
            spec.params
                .iter()
                .enumerate()
                .filter(|(_, param)| !used.contains(&param.name))
                .map(|(index, param)| {
                    let text = format!("{}{separator}", param.name);
                    param_item(index, param, edit(replace.clone(), text))
                })
                .collect()
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
            catalog::enum_values(param)
                .into_iter()
                .map(|value| CompletionItem {
                    label: value.to_owned(),
                    kind: Some(CompletionItemKind::ENUM_MEMBER),
                    detail: Some(param.ty.to_string()),
                    text_edit: edit(replace.clone(), value.to_owned()),
                    ..CompletionItem::default()
                })
                .collect()
        }
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

/// Returns the description of the command or parameter at `position`.
pub(crate) fn hover(request: Request<'_>, position: Position) -> Option<Hover> {
    let lines = Lines::new(request.text);
    let offset = lines.offset(position, request.encoding);
    let tokens = context::tokens(request.text, request.variant);
    let (value, range) = match context::target(request.text, &tokens, offset, request.variant)? {
        Target::Command { name, range } => {
            let spec = request.commands.get(&name)?;
            (catalog::command_markdown(spec, request.variant), range)
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
    Some(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value,
        }),
        range: Some(lines.range(range, request.encoding)),
    })
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

#[cfg(test)]
mod tests {
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
        }
    }

    fn new_text(item: &CompletionItem) -> (&str, lsp_types::Range) {
        match &item.text_edit {
            Some(CompletionTextEdit::Edit(edit)) => (edit.new_text.as_str(), edit.range),
            other => panic!("unexpected edit {other:?}"),
        }
    }

    #[test]
    fn completes_command_names_with_their_group() {
        let commands = sample();
        let items = completion(
            request("count |> so", Variant::A, &commands),
            Position::new(0, 11),
        );
        assert_eq!(labels(&items), ["count", "sort"]);
        assert_eq!(items[1].detail.as_deref(), Some("core"));
        let (text, range) = new_text(&items[1]);
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
        let unknown = completion(
            request("find \"/srv\" ", Variant::A, &commands),
            Position::new(0, 12),
        );
        assert!(unknown.is_empty());
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
                request("find \"/srv\"", Variant::A, &commands),
                Position::new(0, 1)
            )
            .is_none()
        );
    }
}
