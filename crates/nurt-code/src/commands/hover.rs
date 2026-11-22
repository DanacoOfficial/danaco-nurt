// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The `code.hover` stage.
//!
//! The answer of the language server is shortened to the signature (the
//! first code block, with a signature written over several lines joined into
//! one line) and the first sentence of the documentation; with the detailed
//! verbosity of the request the documentation is the whole answer. Without a
//! usable server under `source: auto`, the definition of the identifier is
//! found by syntax analysis and its documentation is the comment above it or
//! the docstring below it.

use std::sync::{Arc, LazyLock};

use nurt_contracts::envelope::Verbosity;
use nurt_contracts::error::Result;
use nurt_contracts::record::{Hover, SymbolSource};
use nurt_contracts::services::SymbolInfo;
use nurt_contracts::stage::{
    BoundArgs, ExecCtx, InputKinds, ReadStage, RecordStream, StageDescriptor, StageImpl,
    StageInput, StageOutput,
};
use nurt_contracts::value::Record;
use regex::Regex;

use crate::commands::definition::shared::{self, FileText, Files, Source, Target};
use crate::semantic::SemanticProvider;

const COMMAND: &str = "code.hover";

/// Lines of a code block joined into a signature.
const SIGNATURE_LINES_MAX: usize = 12;

static CODE_BLOCK: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"```[^\n]*\n([\s\S]*?)```").ok());

static RULE_LINE: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new(r"(?m)^[\s\-*_]+$").ok());

static COMMENT_PREFIX: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r#"^\s*(?://+|/\*+|\*+/?|#+|"""|''')\s?"#).ok());

static DOCSTRING: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r#"^[rRbBuU]{0,2}("""|''')"#).ok());

/// Implementation of `code.hover`.
///
/// ```
/// use nurt_code::commands::hover::CodeHover;
/// use nurt_contracts::stage::StageImpl;
///
/// let stage = CodeHover::new(None);
/// assert_eq!(stage.descriptor().command, "code.hover");
/// ```
pub struct CodeHover {
    semantic: Option<Arc<dyn SemanticProvider>>,
}

impl CodeHover {
    /// Stage that asks `semantic` first; without a provider every answer
    /// comes from syntax analysis.
    #[must_use]
    pub fn new(semantic: Option<Arc<dyn SemanticProvider>>) -> Self {
        Self { semantic }
    }

    fn records(&self, args: &BoundArgs, ctx: &ExecCtx) -> Result<Vec<Record>> {
        let files = Files::new(ctx);
        let source = Source::from_args(args)?;
        let target = Target::resolve(args, ctx, &files, COMMAND)?;
        let served = shared::ask(self.semantic.as_deref(), source, ctx, |provider| {
            provider.hover(ctx, &target.scope(), &target.at())
        })?;
        match served {
            Some(Some(info)) => {
                let (signature, sentence) = shorten_hover(&info.contents);
                let documentation = if ctx.options().verbosity() == Verbosity::Detailed {
                    info.contents.clone()
                } else {
                    sentence
                };
                return Ok(vec![hover_record(&target, SymbolSource::Lsp, signature, documentation)]);
            }
            Some(None) if source == Source::Lsp => return Ok(Vec::new()),
            Some(None) => shared::note_fallback(ctx, "the language server has no information"),
            None => {}
        }
        let definitions = shared::local_definitions(ctx, &files, &target)?;
        Ok(definitions
            .first()
            .map(|(path, info)| {
                let documentation = files
                    .get(path)
                    .map(|file| local_documentation(&file, info))
                    .unwrap_or_default();
                let signature = info
                    .signature
                    .as_deref()
                    .map(|signature| shared::clip(signature, shared::SIGNATURE_MAX))
                    .unwrap_or_default();
                hover_record(&target, SymbolSource::Syntax, signature, documentation)
            })
            .into_iter()
            .collect())
    }
}

fn hover_record(target: &Target, source: SymbolSource, signature: String, documentation: String) -> Record {
    Hover {
        path: shared::record_path(&target.path),
        line: u64::from(target.position.line),
        column: u64::from(target.position.column),
        source,
        signature: (!signature.is_empty()).then_some(signature),
        documentation: (!documentation.is_empty()).then_some(documentation),
    }
    .into()
}

/// First sentence of `text` with whitespace collapsed, shortened to 200
/// characters.
///
/// ```
/// use nurt_code::commands::hover::first_sentence;
///
/// assert_eq!(first_sentence("Base shape.  Second   sentence."), "Base shape.");
/// assert_eq!(first_sentence("No end"), "No end");
/// ```
#[must_use]
pub fn first_sentence(text: &str) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let characters: Vec<char> = flat.chars().collect();
    let end = characters.iter().enumerate().position(|(index, character)| {
        matches!(character, '.' | '!' | '?')
            && characters.get(index + 1).is_none_or(|next| next.is_whitespace())
    });
    let sentence: String = match end {
        Some(end) => characters[..=end].iter().collect(),
        None => flat,
    };
    shared::clip(&sentence, shared::SENTENCE_MAX)
}

/// Lines of a code block joined into one line: no space after an opening
/// bracket or before a closing one, and no trailing comma before a closing
/// bracket.
///
/// ```
/// use nurt_code::commands::hover::flat_signature;
///
/// let block = "def area(\n    shape,\n    scale,\n) -> int";
/// assert_eq!(flat_signature(block), "def area(shape, scale) -> int");
/// ```
#[must_use]
pub fn flat_signature(block: &str) -> String {
    let mut flat = String::new();
    let lines = block
        .trim()
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .take(SIGNATURE_LINES_MAX);
    for line in lines {
        let closing = line.starts_with([')', ']', '}']);
        if closing && flat.ends_with(',') {
            flat.pop();
        }
        let opening = flat.ends_with(['(', '[', '{']);
        if !(flat.is_empty() || closing || opening) {
            flat.push(' ');
        }
        flat.push_str(line);
    }
    flat
}

/// Signature and first documentation sentence of a hover text.
///
/// The signature is the first code block; without one it is the first
/// non-empty line, and the documentation is the first sentence of the rest.
///
/// ```
/// use nurt_code::commands::hover::shorten_hover;
///
/// let text = "```python\ndef area(\n    shape,\n)\n```\n---\nCompute the area. Uses the scale.";
/// let (signature, documentation) = shorten_hover(text);
/// assert_eq!(signature, "def area(shape)");
/// assert_eq!(documentation, "Compute the area.");
/// ```
#[must_use]
pub fn shorten_hover(text: &str) -> (String, String) {
    let block = CODE_BLOCK
        .as_ref()
        .and_then(|pattern| pattern.captures(text))
        .and_then(|captures| captures.get(1))
        .map(|block| block.as_str().to_owned());
    let Some(block) = block else {
        let mut lines = text.lines().filter(|line| !line.trim().is_empty());
        let signature = lines.next().map(str::trim).unwrap_or_default();
        let rest = lines.collect::<Vec<_>>().join(" ");
        return (shared::clip(signature, shared::SIGNATURE_MAX), first_sentence(&rest));
    };
    let prose = CODE_BLOCK
        .as_ref()
        .map_or_else(|| text.to_owned(), |pattern| pattern.replace_all(text, " ").into_owned());
    let prose = RULE_LINE
        .as_ref()
        .map_or(prose.clone(), |pattern| pattern.replace_all(&prose, " ").into_owned());
    (
        shared::clip(&flat_signature(&block), shared::SIGNATURE_MAX),
        first_sentence(&prose),
    )
}

/// Documentation of a definition from the source: a docstring on the line
/// after the name, or the comment lines directly above the definition.
fn local_documentation(file: &FileText, info: &SymbolInfo) -> String {
    let below = file
        .line(info.name_span.start.line + 1)
        .map(str::trim)
        .unwrap_or_default();
    if let Some(pattern) = DOCSTRING.as_ref()
        && pattern.is_match(below)
    {
        let inner = pattern.replace(below, "");
        let inner = inner.trim_end_matches("\"\"\"").trim_end_matches("'''");
        return first_sentence(inner);
    }
    let Some(prefix) = COMMENT_PREFIX.as_ref() else {
        return String::new();
    };
    let mut collected = Vec::new();
    let mut line = info.span.start.line;
    while line > 1 {
        line -= 1;
        let Some(text) = file.line(line) else {
            break;
        };
        if !prefix.is_match(text) {
            break;
        }
        let stripped = prefix.replace(text, "");
        collected.push(stripped.trim_end_matches("*/").trim().to_owned());
    }
    collected.reverse();
    first_sentence(&collected.join(" "))
}

impl StageImpl for CodeHover {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: COMMAND.to_owned(),
            source: true,
            input: InputKinds::None,
        }
    }

    fn as_read(&self) -> Option<&dyn ReadStage> {
        Some(self)
    }
}

impl ReadStage for CodeHover {
    fn execute<'a>(
        &'a self,
        args: &'a BoundArgs,
        _input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<StageOutput<'a>> {
        let records = self.records(args, ctx)?;
        Ok(StageOutput::records(RecordStream::from_records(records)))
    }
}
