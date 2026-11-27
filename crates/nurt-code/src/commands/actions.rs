// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-code::commands::actions
// Responsibility:   The `code.actions` stage.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! The `code.actions` stage.
//!
//! Code actions are offered only by a language server
//! ([`SemanticProvider::code_actions`]); the stage lists them for a range of
//! lines and applies none. Diagnostics published for the range are sent as
//! the context of the request. Without a server the stage fails with
//! `E_EXTERNAL_MISSING`.

use std::sync::Arc;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::record::{CodeAction, SymbolSource};
use nurt_contracts::services::TextPosition;
use nurt_contracts::stage::{
    BoundArgs, ExecCtx, InputKinds, ReadStage, RecordStream, StageDescriptor, StageImpl,
    StageInput, StageOutput,
};
use nurt_contracts::value::Record;

use crate::commands::definition::shared::{self, Files, Source, Target};
use crate::semantic::{Location, SemanticProvider};

const COMMAND: &str = "code.actions";

/// Kind reported for an action that only runs a server command.
const COMMAND_KIND: &str = "command";

/// Implementation of `code.actions`.
///
/// ```
/// use nurt_code::commands::actions::CodeActions;
/// use nurt_contracts::stage::StageImpl;
///
/// let stage = CodeActions::new(None);
/// assert_eq!(stage.descriptor().command, "code.actions");
/// ```
pub struct CodeActions {
    semantic: Option<Arc<dyn SemanticProvider>>,
}

impl CodeActions {
    /// Stage backed by `semantic`; without a provider it fails with
    /// `E_EXTERNAL_MISSING`.
    #[must_use]
    pub fn new(semantic: Option<Arc<dyn SemanticProvider>>) -> Self {
        Self { semantic }
    }

    fn records(&self, args: &BoundArgs, ctx: &ExecCtx) -> Result<Vec<Record>> {
        let files = Files::new(ctx);
        let source = Source::from_args(args)?;
        let target = Target::resolve(args, ctx, &files, COMMAND)?;
        let first = target.position.line;
        let last = shared::number_arg(args, "end_line")?.unwrap_or(first);
        if last < first {
            return Err(Error::new(
                ErrorCode::InvalidValue,
                format!("end_line {last} is before line {first}; give end_line at or after line"),
            ));
        }
        let last_text = target.file.line(last).ok_or_else(|| {
            Error::new(
                ErrorCode::InvalidValue,
                format!("{} has fewer than {last} lines", target.path.display()),
            )
        })?;
        let end_column = u32::try_from(last_text.chars().count() + 1).unwrap_or(u32::MAX);
        let range = Location {
            path: target.path.clone(),
            start: TextPosition {
                line: first,
                column: if shared::number_arg(args, "column")?.is_some() {
                    target.position.column
                } else {
                    1
                },
            },
            end: TextPosition {
                line: last,
                column: end_column,
            },
        };
        let actions =
            shared::require_server(self.semantic.as_deref(), source, COMMAND, |provider| {
                provider.code_actions(ctx, &target.scope(), &range)
            })?;
        Ok(actions
            .into_iter()
            .map(|action| {
                CodeAction {
                    path: shared::record_path(&target.path),
                    line: u64::from(first),
                    title: action.title,
                    source: SymbolSource::Lsp,
                    end_line: Some(u64::from(last)),
                    kind: Some(action.kind.unwrap_or_else(|| COMMAND_KIND.to_owned())),
                }
                .into()
            })
            .collect())
    }
}

impl StageImpl for CodeActions {
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

impl ReadStage for CodeActions {
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
