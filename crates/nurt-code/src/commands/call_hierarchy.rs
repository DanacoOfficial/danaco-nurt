// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-code::commands::call_hierarchy
// Responsibility:   The `code.call_hierarchy` stage.
// Dependencies:     nurt_contracts, regex
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! The `code.call_hierarchy` stage.
//!
//! Callers and callees come from the language server
//! ([`SemanticProvider::call_hierarchy`]). Under `source: auto` without a
//! usable server, syntax analysis approximates them: callers are the
//! functions and methods that contain a reference to the identifier, callees
//! are the functions of the project called in the body of its definition.
//!
//! Each `reference` record is one caller or callee. Its `text` holds the
//! direction (`<-` for a caller, `->` for a callee), the kind, the qualified
//! name and, for more than one call, the multiplicity (`x3`).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::record::{Reference, ReferenceRole, SymbolKind as RecordKind, SymbolSource};
use nurt_contracts::services::{SpanClass, SymbolInfo, SymbolKind, TextPosition};
use nurt_contracts::stage::{
    BoundArgs, ExecCtx, InputKinds, ReadStage, RecordStream, StageDescriptor, StageImpl,
    StageInput, StageOutput,
};
use nurt_contracts::value::Record;
use regex::Regex;

use crate::commands::definition::shared::{self, Files, Source, Target};
use crate::semantic::{CallDirection, CallEdge, SemanticProvider};

const COMMAND: &str = "code.call_hierarchy";

static CALL: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"([A-Za-z_$][\w$]*)\s*\(").ok());

/// Implementation of `code.call_hierarchy`.
///
/// ```
/// use nurt_code::commands::call_hierarchy::CodeCallHierarchy;
/// use nurt_contracts::stage::StageImpl;
///
/// let stage = CodeCallHierarchy::new(None);
/// assert_eq!(stage.descriptor().command, "code.call_hierarchy");
/// ```
pub struct CodeCallHierarchy {
    semantic: Option<Arc<dyn SemanticProvider>>,
}

/// Caller or callee before conversion to a record.
struct Call {
    incoming: bool,
    name: String,
    kind: RecordKind,
    container: Option<String>,
    path: PathBuf,
    position: TextPosition,
    count: usize,
}

impl CodeCallHierarchy {
    /// Stage that asks `semantic` first; without a provider every answer
    /// comes from syntax analysis.
    #[must_use]
    pub fn new(semantic: Option<Arc<dyn SemanticProvider>>) -> Self {
        Self { semantic }
    }

    fn records(&self, args: &BoundArgs, ctx: &ExecCtx) -> Result<Vec<Record>> {
        let files = Files::new(ctx);
        let source = Source::from_args(args)?;
        let direction = direction(args)?;
        let target = Target::resolve(args, ctx, &files, COMMAND)?;
        let served = shared::ask(self.semantic.as_deref(), source, ctx, |provider| {
            provider.call_hierarchy(ctx, &target.scope(), &target.at(), direction)
        })?;
        let (calls, source) = match served {
            Some(edges) => (edges.iter().map(served_call).collect(), SymbolSource::Lsp),
            None => (
                local_calls(ctx, &files, &target, direction)?,
                SymbolSource::Syntax,
            ),
        };
        Ok(calls
            .into_iter()
            .map(|call| call_record(&files, call, source))
            .collect())
    }
}

fn direction(args: &BoundArgs) -> Result<CallDirection> {
    match shared::word_arg(args, "direction")?.as_deref() {
        None | Some("both") => Ok(CallDirection::Both),
        Some("incoming") => Ok(CallDirection::Incoming),
        Some("outgoing") => Ok(CallDirection::Outgoing),
        Some(other) => Err(Error::new(
            ErrorCode::InvalidValue,
            format!("unknown direction `{other}`; use incoming, outgoing or both"),
        )),
    }
}

fn served_call(edge: &CallEdge) -> Call {
    Call {
        incoming: edge.direction == CallDirection::Incoming,
        name: edge.symbol.name.clone(),
        kind: edge.symbol.kind,
        container: edge.symbol.container.clone(),
        path: edge.symbol.location.path.clone(),
        position: edge.symbol.name_position,
        count: edge.calls.len().max(1),
    }
}

fn call_record(files: &Files<'_>, call: Call, source: SymbolSource) -> Record {
    let arrow = if call.incoming { "<-" } else { "->" };
    let qualified = match &call.container {
        Some(container) => format!("{container}.{}", call.name),
        None => call.name.clone(),
    };
    let times = if call.count > 1 {
        format!(" x{}", call.count)
    } else {
        String::new()
    };
    Reference {
        name: call.name.clone(),
        path: shared::record_path(&call.path),
        line: u64::from(call.position.line),
        column: u64::from(call.position.column),
        role: ReferenceRole::Reference,
        source,
        etag: files
            .get(&call.path)
            .map(|file| file.etag.clone())
            .unwrap_or_default(),
        end_column: None,
        text: Some(format!("{arrow} {} {qualified}{times}", call.kind.as_str())),
        container: call.container,
    }
    .into()
}

fn callable(kind: SymbolKind) -> bool {
    matches!(
        kind,
        SymbolKind::Function | SymbolKind::Method | SymbolKind::Class | SymbolKind::Struct
    )
}

fn add_call(
    calls: &mut Vec<Call>,
    index: &mut HashMap<(bool, PathBuf, TextPosition), usize>,
    incoming: bool,
    path: &Path,
    info: &SymbolInfo,
) {
    let key = (incoming, path.to_path_buf(), info.name_span.start);
    if let Some(known) = index
        .get(&key)
        .and_then(|position| calls.get_mut(*position))
    {
        known.count += 1;
        return;
    }
    index.insert(key, calls.len());
    calls.push(Call {
        incoming,
        name: info.name.clone(),
        kind: shared::record_kind(info.kind),
        container: info.container.clone(),
        path: path.to_path_buf(),
        position: info.name_span.start,
        count: 1,
    });
}

fn local_calls(
    ctx: &ExecCtx,
    files: &Files<'_>,
    target: &Target,
    direction: CallDirection,
) -> Result<Vec<Call>> {
    let mut calls = Vec::new();
    let mut index = HashMap::new();
    if target.word.is_none() {
        return Ok(calls);
    }
    if direction != CallDirection::Outgoing {
        local_incoming(ctx, files, target, &mut calls, &mut index)?;
    }
    if direction != CallDirection::Incoming {
        local_outgoing(ctx, files, target, &mut calls, &mut index)?;
    }
    Ok(calls)
}

fn local_incoming(
    ctx: &ExecCtx,
    files: &Files<'_>,
    target: &Target,
    calls: &mut Vec<Call>,
    index: &mut HashMap<(bool, PathBuf, TextPosition), usize>,
) -> Result<()> {
    let (Some(code), Some(language)) = (ctx.code(), target.language.as_ref()) else {
        return Ok(());
    };
    let word = target.word.as_deref().unwrap_or_default();
    let references = shared::local_references(ctx, files, target, true)?;
    for reference in references
        .iter()
        .filter(|item| !matches!(item.role, ReferenceRole::Import | ReferenceRole::Definition))
    {
        let Some(file) = files.get(&reference.path) else {
            continue;
        };
        let caller = code
            .enclosing(file.text.as_bytes(), language, reference.offset)
            .ok()
            .flatten();
        if let Some(caller) = caller.filter(|caller| callable(caller.kind) && caller.name != word) {
            add_call(calls, index, true, &reference.path, &caller);
        }
    }
    Ok(())
}

/// Callable definitions of the project by name; the first definition of a
/// name wins.
fn project_callables(
    ctx: &ExecCtx,
    files: &Files<'_>,
    target: &Target,
) -> Result<HashMap<String, (PathBuf, SymbolInfo)>> {
    let mut callables = HashMap::new();
    let (Some(code), Some(language)) = (ctx.code(), target.language.as_ref()) else {
        return Ok(callables);
    };
    for path in shared::project_files(ctx, files, target)? {
        ctx.checkpoint()?;
        let Some(file) = files.get(&path) else {
            continue;
        };
        for info in code
            .outline(file.text.as_bytes(), language, None)
            .unwrap_or_default()
        {
            if callable(info.kind) {
                callables
                    .entry(info.name.clone())
                    .or_insert_with(|| (path.clone(), info));
            }
        }
    }
    Ok(callables)
}

/// Text of `body` with comments and strings replaced by spaces.
fn mask_non_code(body: &str, spans: &[nurt_contracts::services::ClassifiedSpan]) -> String {
    let mut bytes = body.as_bytes().to_vec();
    for span in spans.iter().filter(|span| span.class != SpanClass::Code) {
        let start = usize::try_from(span.bytes.start)
            .unwrap_or(usize::MAX)
            .min(bytes.len());
        let end = usize::try_from(span.bytes.end)
            .unwrap_or(usize::MAX)
            .min(bytes.len());
        for byte in &mut bytes[start..end] {
            if *byte != b'\n' {
                *byte = b' ';
            }
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

fn local_outgoing(
    ctx: &ExecCtx,
    files: &Files<'_>,
    target: &Target,
    calls: &mut Vec<Call>,
    index: &mut HashMap<(bool, PathBuf, TextPosition), usize>,
) -> Result<()> {
    let (Some(code), Some(language), Some(pattern)) =
        (ctx.code(), target.language.as_ref(), CALL.as_ref())
    else {
        return Ok(());
    };
    let definitions = shared::local_definitions(ctx, files, target)?;
    let Some((path, own)) = definitions.iter().find(|(_, info)| callable(info.kind)) else {
        return Ok(());
    };
    let Some(file) = files.get(path) else {
        return Ok(());
    };
    let start = usize::try_from(own.span.bytes.start).unwrap_or(0);
    let end = usize::try_from(own.span.bytes.end).unwrap_or(0);
    let Some(body) = file.text.get(start..end) else {
        return Ok(());
    };
    let spans = code
        .classify_spans(body.as_bytes(), language)
        .unwrap_or_default();
    let masked = mask_non_code(body, &spans);
    let callables = project_callables(ctx, files, target)?;
    for found in pattern.captures_iter(&masked) {
        let Some(name) = found.get(1) else {
            continue;
        };
        let member = masked[..name.start()].ends_with('.');
        if member || name.as_str() == own.name {
            continue;
        }
        if let Some((callee_path, callee)) = callables.get(name.as_str()) {
            add_call(calls, index, false, callee_path, callee);
        }
    }
    Ok(())
}

impl StageImpl for CodeCallHierarchy {
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

impl ReadStage for CodeCallHierarchy {
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

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
