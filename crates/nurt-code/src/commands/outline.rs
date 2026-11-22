// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The `code.outline` stage.
//!
//! The symbols of one file are listed in source order, outer symbols before
//! the symbols they contain, no deeper than `depth`. A file of a language
//! with a built-in grammar is outlined by syntax analysis; a file of
//! another language by the language server
//! ([`SemanticProvider::document_symbols`]) when one is available.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::stage::{
    BoundArgs, ExecCtx, InputKinds, ReadStage, RecordStream, StageDescriptor, StageImpl,
    StageInput, StageOutput,
};
use nurt_contracts::value::Record;

use crate::commands::definition::shared;
use crate::index::read_source;
use crate::outline::limit_depth;
use crate::registry::Registry;
use crate::semantic::{SemanticError, SemanticErrorKind, SemanticProvider, SemanticScope};
use crate::symbols::extract;
use crate::symbols::records::{FileFacts, RecordFields, semantic_record, symbol_record};

const COMMAND: &str = "code.outline";

/// Fields of outline records: signatures are part of an outline.
const FIELDS: RecordFields = RecordFields {
    signature: true,
    references: None,
};

/// Implementation of `code.outline`.
///
/// ```
/// use nurt_code::commands::outline::CodeOutline;
/// use nurt_contracts::stage::StageImpl;
///
/// let stage = CodeOutline::new(None);
/// assert_eq!(stage.descriptor().command, "code.outline");
/// ```
pub struct CodeOutline {
    semantic: Option<Arc<dyn SemanticProvider>>,
    registry: &'static Registry,
}

impl CodeOutline {
    /// Stage that asks `semantic` for files without a built-in grammar.
    #[must_use]
    pub fn new(semantic: Option<Arc<dyn SemanticProvider>>) -> Self {
        Self {
            semantic,
            registry: Registry::builtin(),
        }
    }

    fn outline(&self, ctx: &ExecCtx, path: &Path, depth: Option<u32>) -> Result<Vec<Record>> {
        let source = read_source(ctx, path)?;
        let etag = source.etag.to_string();
        if let Some(language) = self.registry.detect(Some(path), &source.bytes) {
            let parsed = self.registry.parse(&language, &source.bytes)?;
            let facts = FileFacts {
                path,
                language: &language,
                etag: &etag,
            };
            return Ok(limit_depth(extract(&parsed), depth)
                .iter()
                .map(|info| symbol_record(info, &facts, FIELDS).into())
                .collect());
        }
        let Some(provider) = self.semantic.as_deref() else {
            return Err(no_language(path));
        };
        let directory = path
            .parent()
            .map_or_else(|| path.to_path_buf(), Path::to_path_buf);
        let symbols = provider
            .document_symbols(ctx, &SemanticScope::new(directory), path)
            .map_err(|error| server_error(path, &error))?;
        Ok(symbols
            .iter()
            .filter(|symbol| depth.is_none_or(|max| symbol.depth <= max))
            .map(|symbol| semantic_record(symbol, "", &etag, FIELDS).into())
            .collect())
    }
}

fn no_language(path: &Path) -> Error {
    Error::new(
        ErrorCode::UnsupportedLanguage,
        format!(
            "no grammar for {}; list the languages with `code.languages`",
            path.display()
        ),
    )
}

/// Error of a failed server request: a file without a grammar and without
/// an available server has no supported language; other failures keep the
/// error of the server.
fn server_error(path: &Path, error: &SemanticError) -> Error {
    if error.kind != SemanticErrorKind::Unavailable {
        return error.to_error();
    }
    let unsupported = Error::new(
        ErrorCode::UnsupportedLanguage,
        format!(
            "no grammar and no language server for {}: {error}; list the languages with `code.languages`",
            path.display()
        ),
    );
    match &error.fix {
        Some(fix) => unsupported.with_fix(fix.clone()),
        None => unsupported,
    }
}

fn depth_arg(args: &BoundArgs) -> Result<Option<u32>> {
    let Some(depth) = args.int("depth")? else {
        return Ok(None);
    };
    u32::try_from(depth).map(Some).map_err(|_| {
        Error::new(
            ErrorCode::InvalidValue,
            format!("`depth` must be 0 or more, not {depth}"),
        )
    })
}

fn path_arg(args: &BoundArgs, ctx: &ExecCtx) -> Result<PathBuf> {
    let text = shared::path_arg(args, "path")?.ok_or_else(|| {
        Error::new(
            ErrorCode::MissingParam,
            format!("`{COMMAND}` needs `path`: the source file to outline"),
        )
    })?;
    shared::resolve_path(ctx, &text)
}

impl StageImpl for CodeOutline {
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

impl ReadStage for CodeOutline {
    fn execute<'a>(
        &'a self,
        args: &'a BoundArgs,
        _input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<StageOutput<'a>> {
        let path = path_arg(args, ctx)?;
        let depth = depth_arg(args)?;
        let records = self.outline(ctx, &path, depth)?;
        Ok(StageOutput::records(RecordStream::from_records(records)))
    }
}

#[cfg(test)]
mod tests {
    use nurt_contracts::stage::ArgValue;
    use nurt_contracts::value::Value;

    use super::*;

    #[test]
    fn negative_depth_is_rejected() {
        let args = BoundArgs::new(COMMAND).with("depth", ArgValue::Value(Value::Int(-1)), false);
        assert_eq!(
            depth_arg(&args).unwrap_err().code(),
            ErrorCode::InvalidValue
        );
        let args = BoundArgs::new(COMMAND).with("depth", ArgValue::Absent, true);
        assert_eq!(depth_arg(&args).unwrap(), None);
    }
}
