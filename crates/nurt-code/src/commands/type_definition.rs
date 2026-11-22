// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The `code.type_definition` stage.
//!
//! The definition of the type of a symbol is known only to a language
//! server ([`SemanticProvider::type_definition`]); without one the stage
//! fails with `E_EXTERNAL_MISSING`, and a failure of the server is the error
//! of the stage.

use std::sync::Arc;

use nurt_contracts::error::Result;
use nurt_contracts::record::SymbolSource;
use nurt_contracts::stage::{
    BoundArgs, ExecCtx, InputKinds, ReadStage, RecordStream, StageDescriptor, StageImpl,
    StageInput, StageOutput,
};
use nurt_contracts::value::Record;

use crate::commands::definition::shared::{self, Files, Source, Target};
use crate::semantic::SemanticProvider;

const COMMAND: &str = "code.type_definition";

/// Implementation of `code.type_definition`.
///
/// ```
/// use nurt_code::commands::type_definition::CodeTypeDefinition;
/// use nurt_contracts::stage::StageImpl;
///
/// let stage = CodeTypeDefinition::new(None);
/// assert_eq!(stage.descriptor().command, "code.type_definition");
/// ```
pub struct CodeTypeDefinition {
    semantic: Option<Arc<dyn SemanticProvider>>,
}

impl CodeTypeDefinition {
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
        let locations =
            shared::require_server(self.semantic.as_deref(), source, COMMAND, |provider| {
                provider.type_definition(ctx, &target.scope(), &target.at())
            })?;
        Ok(locations
            .iter()
            .map(|location| {
                shared::symbol_record(&files, &target, location, SymbolSource::Lsp).into()
            })
            .collect())
    }
}

impl StageImpl for CodeTypeDefinition {
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

impl ReadStage for CodeTypeDefinition {
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
