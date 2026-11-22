// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The `code.implementations` stage.
//!
//! Implementations come from the language server
//! ([`SemanticProvider::implementations`]). Under `source: auto` without a
//! usable server, or with an empty answer, syntax analysis lists the
//! classes, structures, interfaces and enumerations of the project whose
//! signature names the identifier after `extends`, `implements` or `for`, or
//! in a list of base types.

use std::sync::Arc;

use nurt_contracts::error::Result;
use nurt_contracts::record::SymbolSource;
use nurt_contracts::services::SymbolKind;
use nurt_contracts::stage::{
    BoundArgs, ExecCtx, InputKinds, ReadStage, RecordStream, StageDescriptor, StageImpl,
    StageInput, StageOutput,
};
use nurt_contracts::value::Record;
use regex::Regex;

use crate::commands::definition::shared::{self, Files, Source, Target};
use crate::semantic::SemanticProvider;

const COMMAND: &str = "code.implementations";

/// Implementation of `code.implementations`.
///
/// ```
/// use nurt_code::commands::implementations::CodeImplementations;
/// use nurt_contracts::stage::StageImpl;
///
/// let stage = CodeImplementations::new(None);
/// assert_eq!(stage.descriptor().command, "code.implementations");
/// ```
pub struct CodeImplementations {
    semantic: Option<Arc<dyn SemanticProvider>>,
}

impl CodeImplementations {
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
            provider.implementations(ctx, &target.scope(), &target.at())
        })?;
        match served {
            Some(locations) if !locations.is_empty() || source == Source::Lsp => {
                return Ok(locations
                    .iter()
                    .map(|location| {
                        shared::symbol_record(&files, &target, location, SymbolSource::Lsp).into()
                    })
                    .collect());
            }
            Some(_) => shared::note_fallback(ctx, "the language server found no implementation"),
            None => {}
        }
        local_implementations(ctx, &files, &target)
    }
}

/// Pattern of a signature that derives from or implements `name`.
///
/// ```
/// use nurt_code::commands::implementations::derives_pattern;
///
/// let pattern = derives_pattern("Repo").unwrap();
/// assert!(pattern.is_match("class SqlRepo implements Repo {"));
/// assert!(pattern.is_match("class Circle(Repo):"));
/// assert!(pattern.is_match("impl Repo for Sql"));
/// assert!(!pattern.is_match("class Repository {"));
/// ```
#[must_use]
pub fn derives_pattern(name: &str) -> Option<Regex> {
    let name = regex::escape(name);
    Regex::new(&format!(
        r"(?:\b(?:extends|implements|for)\b[^{{]*|[(:,]\s*|\bimpl\s+)\b{name}\b"
    ))
    .ok()
}

fn local_implementations(ctx: &ExecCtx, files: &Files<'_>, target: &Target) -> Result<Vec<Record>> {
    let (Some(word), Some(code), Some(language)) =
        (target.word.as_deref(), ctx.code(), target.language.as_ref())
    else {
        return Ok(Vec::new());
    };
    let Some(pattern) = derives_pattern(word) else {
        return Ok(Vec::new());
    };
    let mut records = Vec::new();
    for path in shared::project_files(ctx, files, target) {
        ctx.checkpoint()?;
        let Some(file) = files.get(&path) else {
            continue;
        };
        if !file.text.contains(word) {
            continue;
        }
        let outline = code
            .outline(file.text.as_bytes(), language, None)
            .unwrap_or_default();
        for info in outline {
            let typed = matches!(
                info.kind,
                SymbolKind::Class | SymbolKind::Struct | SymbolKind::Interface | SymbolKind::Enum
            );
            let derives = info
                .signature
                .as_deref()
                .is_some_and(|signature| pattern.is_match(signature));
            if typed && derives && info.name != word {
                records.push(shared::local_symbol_record(files, target, &path, &info).into());
            }
        }
    }
    Ok(records)
}

impl StageImpl for CodeImplementations {
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

impl ReadStage for CodeImplementations {
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
