// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The `code.references` stage.
//!
//! References come from the language server ([`SemanticProvider::references`])
//! or, under `source: auto` without a usable server, from syntax analysis of
//! the files of the project in the same language (occurrences in code only,
//! not in comments and strings). Every reference has a role: `definition`,
//! `import`, `test` (a use in a test file) or `reference`. Records are
//! ordered by path, line and column.
//!
//! In the stage form every input `symbol` record is a target.

use std::path::PathBuf;
use std::sync::Arc;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::record::{Reference, ReferenceRole, Symbol, SymbolSource};
use nurt_contracts::stage::{
    BoundArgs, ExecCtx, InputKinds, ReadStage, RecordStream, StageDescriptor, StageImpl,
    StageInput, StageOutput,
};
use nurt_contracts::value::{Record, RecordKind};

use crate::commands::definition::shared::{self, Files, Occurrence, Source, Target};
use crate::semantic::{Location, SemanticProvider};

const COMMAND: &str = "code.references";

/// Implementation of `code.references`.
///
/// ```
/// use nurt_code::commands::references::CodeReferences;
/// use nurt_contracts::stage::StageImpl;
///
/// let stage = CodeReferences::new(None);
/// let descriptor = stage.descriptor();
/// assert_eq!(descriptor.command, "code.references");
/// assert!(descriptor.source);
/// ```
pub struct CodeReferences {
    semantic: Option<Arc<dyn SemanticProvider>>,
}

struct Options {
    source: Source,
    include_definition: bool,
    whole_project: bool,
}

impl CodeReferences {
    /// Stage that asks `semantic` first; without a provider every answer
    /// comes from syntax analysis.
    #[must_use]
    pub fn new(semantic: Option<Arc<dyn SemanticProvider>>) -> Self {
        Self { semantic }
    }

    fn options(args: &BoundArgs) -> Result<Options> {
        let whole_project = match shared::word_arg(args, "scope")?.as_deref() {
            None | Some("project") => true,
            Some("file") => false,
            Some(other) => {
                return Err(Error::new(
                    ErrorCode::InvalidValue,
                    format!("unknown scope `{other}`; use project or file"),
                ));
            }
        };
        Ok(Options {
            source: Source::from_args(args)?,
            include_definition: args.bool("include_definition")?.unwrap_or(true),
            whole_project,
        })
    }

    fn for_target(&self, ctx: &ExecCtx, files: &Files<'_>, target: &Target, options: &Options) -> Result<Vec<Record>> {
        let served = shared::ask(self.semantic.as_deref(), options.source, ctx, |provider| {
            provider.references(ctx, &target.scope(), &target.at(), options.include_definition)
        })?;
        let (mut found, source) = match served {
            Some(locations) => (
                locations
                    .iter()
                    .map(|location| classify(ctx, files, target, location))
                    .collect(),
                SymbolSource::Lsp,
            ),
            None => (
                shared::local_references(ctx, files, target, options.whole_project)?,
                SymbolSource::Syntax,
            ),
        };
        if !options.whole_project {
            found.retain(|item| item.path == target.path);
        }
        if !options.include_definition {
            found.retain(|item| item.role != ReferenceRole::Definition);
        }
        found.sort_by(|left, right| (&left.path, left.start).cmp(&(&right.path, right.start)));
        found.dedup_by(|left, right| left.path == right.path && left.start == right.start);
        let name = target.word.clone().unwrap_or_default();
        Ok(found
            .into_iter()
            .map(|item| reference_record(files, &name, item, source))
            .collect())
    }

    fn from_input(&self, args: &BoundArgs, input: StageInput<'_>, ctx: &ExecCtx, options: &Options) -> Result<Vec<Record>> {
        let files = Files::new(ctx);
        let requested = shared::requested_language(args)?;
        let root = match shared::path_arg(args, "root")? {
            Some(text) => Some(shared::resolve_path(ctx, &text)?),
            None => None,
        };
        let mut records = Vec::new();
        for item in input.into_records()? {
            let symbol = Symbol::try_from(item?).map_err(|error| {
                Error::new(ErrorCode::Type, format!("`{COMMAND}` expects symbol records: {error}"))
            })?;
            let position = (
                u32::try_from(symbol.line).ok(),
                u32::try_from(symbol.column).ok(),
                None,
            );
            let target = Target::from_parts(
                ctx,
                &files,
                PathBuf::from(symbol.path.as_str()),
                requested.clone(),
                root.clone(),
                position,
                COMMAND,
            )?;
            records.extend(self.for_target(ctx, &files, &target, options)?);
        }
        Ok(records)
    }
}

/// Role of a reference answered by a server.
fn classify(ctx: &ExecCtx, files: &Files<'_>, target: &Target, location: &Location) -> Occurrence {
    let line = files
        .get(&location.path)
        .and_then(|file| file.line(location.start.line).map(str::to_owned))
        .unwrap_or_default();
    let defined_here = shared::symbol_at(ctx, files, &location.path, location.start)
        .filter(|info| info.name_span.start.line == location.start.line)
        .is_some_and(|info| Some(&info.name) == target.word.as_ref());
    let role = if defined_here {
        ReferenceRole::Definition
    } else if shared::is_import_line(&line) {
        ReferenceRole::Import
    } else if shared::is_test_path(&location.path) {
        ReferenceRole::Test
    } else {
        ReferenceRole::Reference
    };
    let container = enclosing_name(ctx, files, location);
    Occurrence {
        path: location.path.clone(),
        start: location.start,
        end: location.end,
        offset: 0,
        role,
        container,
        text: line.trim().to_owned(),
    }
}

fn enclosing_name(ctx: &ExecCtx, files: &Files<'_>, location: &Location) -> Option<String> {
    let file = files.get(&location.path)?;
    let code = ctx.code()?;
    let language = code.language_of(Some(&location.path), file.text.as_bytes())?;
    let offset = shared::offset_of(&file.text, location.start)?;
    let info = code.enclosing(file.text.as_bytes(), &language, offset).ok()??;
    Some(match info.container {
        Some(container) => format!("{container}.{}", info.name),
        None => info.name,
    })
}

fn reference_record(files: &Files<'_>, name: &str, item: Occurrence, source: SymbolSource) -> Record {
    let etag = files.get(&item.path).map(|file| file.etag.clone()).unwrap_or_default();
    Reference {
        name: name.to_owned(),
        path: shared::record_path(&item.path),
        line: u64::from(item.start.line),
        column: u64::from(item.start.column),
        role: item.role,
        source,
        etag,
        end_column: Some(u64::from(item.end.column)),
        text: Some(item.text),
        container: item.container,
    }
    .into()
}

impl StageImpl for CodeReferences {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: COMMAND.to_owned(),
            source: true,
            input: InputKinds::Only(vec![RecordKind::Symbol]),
        }
    }

    fn as_read(&self) -> Option<&dyn ReadStage> {
        Some(self)
    }
}

impl ReadStage for CodeReferences {
    fn execute<'a>(
        &'a self,
        args: &'a BoundArgs,
        input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<StageOutput<'a>> {
        let options = Self::options(args)?;
        let records = if input.is_none() {
            let files = Files::new(ctx);
            let target = Target::resolve(args, ctx, &files, COMMAND)?;
            self.for_target(ctx, &files, &target, &options)?
        } else {
            self.from_input(args, input, ctx, &options)?
        };
        Ok(StageOutput::records(RecordStream::from_records(records)))
    }
}
