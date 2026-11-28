// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-files::commands::notebook::export
// Responsibility:   `notebook.export`: a notebook written to a new script, Markdown or text file.
// Dependencies:     nurt_contracts, nurt_notebook, serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! `notebook.export`: a notebook written to a new script, Markdown or text
//! file.
//!
//! The notebook itself is read and not changed. The exported file must not
//! exist; it is created through the journal, so undoing the operation
//! removes it.

use std::sync::Arc;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::AbsPath;
use nurt_contracts::journal::{FileOp, MutationTx};
use nurt_contracts::stage::{
    BoundArgs, CheckCtx, EffectOutcome, EffectPlan, EffectStage, ExecCtx, InputKinds,
    StageDescriptor, StageImpl, StageInput, StaticInfo,
};
use nurt_contracts::value::Size;
use nurt_notebook::ExportFormat;
use serde_json::json;

use super::{
    PATH, choice, count, effect_outcome, load, missing_param, missing_state, notebook_path,
    static_info,
};
use crate::common::etag::observe;
use crate::common::mutation::{apply_changes, planned_change};
use crate::common::path::path_arg;

const COMMAND: &str = "notebook.export";
const TO: &str = "to";
const FORMAT: &str = "format";
const FORMAT_VALUES: [&str; 3] = ["script", "markdown", "text"];

const FILES_CREATED: &str = "files_created";
const CELLS_EXPORTED: &str = "cells_exported";

/// Implementation of `notebook.export`.
pub(crate) struct NotebookExport;

/// Stages of this module.
pub(crate) fn stages() -> Vec<Arc<dyn StageImpl>> {
    vec![Arc::new(NotebookExport)]
}

impl StageImpl for NotebookExport {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: COMMAND.to_owned(),
            source: true,
            input: InputKinds::None,
        }
    }

    fn check(&self, args: &BoundArgs, _ctx: &CheckCtx<'_>) -> Result<StaticInfo> {
        format_param(args)?;
        Ok(static_info(args, &[PATH, TO]))
    }

    fn as_effect(&self) -> Option<&dyn EffectStage> {
        Some(self)
    }
}

/// State of a plan: the file to create and the number of exported cells.
struct PlannedExport {
    op: FileOp,
    size: u64,
    cells: usize,
}

impl EffectStage for NotebookExport {
    fn plan<'a>(
        &'a self,
        args: &'a BoundArgs,
        _input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<EffectPlan> {
        let format = format_param(args)?;
        let path = notebook_path(args, ctx)?;
        let to = path_arg(args, TO, ctx)?.ok_or_else(|| missing_param(args, TO))?;
        let loaded = load(ctx, &path)?;
        if observe(ctx.require_fs()?, &to)?.exists() {
            return Err(exists_error(&to));
        }
        let bytes = loaded.notebook.export(format).into_bytes();
        let size = count(bytes.len());
        let planned = PlannedExport {
            op: FileOp::create(to, bytes),
            size,
            cells: loaded.notebook.len(),
        };
        let mut plan = EffectPlan::new();
        let mut change = planned_change(&planned.op.target);
        change.size_after = Some(Size::from_bytes(size));
        plan.changes.push(change);
        Ok(plan.with_state(planned))
    }

    fn apply<'a>(
        &'a self,
        _args: &'a BoundArgs,
        mut plan: EffectPlan,
        tx: &mut dyn MutationTx,
        _ctx: &'a ExecCtx,
    ) -> Result<EffectOutcome<'a>> {
        let planned: PlannedExport = plan.take_state().ok_or_else(|| missing_state(COMMAND))?;
        let mut changes = apply_changes(tx, vec![planned.op])?;
        for change in &mut changes {
            change
                .size_after
                .get_or_insert(Size::from_bytes(planned.size));
        }
        Ok(effect_outcome(
            COMMAND,
            &*tx,
            changes,
            &[(FILES_CREATED, 1), (CELLS_EXPORTED, count(planned.cells))],
            None,
        ))
    }
}

fn format_param(args: &BoundArgs) -> Result<ExportFormat> {
    match choice(args, FORMAT, &FORMAT_VALUES)? {
        Some("script") => Ok(ExportFormat::Script),
        Some("markdown") => Ok(ExportFormat::Markdown),
        Some(_) => Ok(ExportFormat::Text),
        None => Err(missing_param(args, FORMAT)),
    }
}

fn exists_error(to: &AbsPath) -> Error {
    Error::new(
        ErrorCode::Exists,
        format!("`{to}` already exists: export to a new file or remove this one first"),
    )
    .with_detail("path", json!(to.to_language_string()))
}

#[cfg(test)]
mod tests {
    use nurt_contracts::stage::ArgValue;
    use nurt_contracts::value::Value;

    use super::*;
    use crate::commands::notebook::fixture::with_file;

    const NOTEBOOK: &str = r#"{"cells": [{"cell_type": "code", "execution_count": null, "metadata": {}, "outputs": [], "source": "x = 1"}], "metadata": {}, "nbformat": 4, "nbformat_minor": 4}"#;

    fn args(path: &AbsPath, to: &str, format: Option<&str>) -> BoundArgs {
        let mut args = BoundArgs::new(COMMAND)
            .with(
                PATH,
                ArgValue::Value(Value::Path(path.to_language_string())),
                false,
            )
            .with(TO, ArgValue::Value(Value::Path(to.to_owned())), false);
        if let Some(format) = format {
            args.insert(
                FORMAT,
                ArgValue::Value(Value::Enum(format.to_owned())),
                false,
            );
        }
        args
    }

    #[test]
    fn an_existing_target_is_not_overwritten() {
        let (test, path) = with_file("/data/a.ipynb", NOTEBOOK.as_bytes());
        let target = path.to_language_string();
        let error = NotebookExport
            .plan(
                &args(&path, &target, Some("text")),
                StageInput::none(),
                &test.ctx(),
            )
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::Exists);
    }

    #[test]
    fn the_format_is_required() {
        let (test, path) = with_file("/data/a.ipynb", NOTEBOOK.as_bytes());
        let target = path.parent().unwrap().join_name("a.py").unwrap();
        let error = NotebookExport
            .plan(
                &args(&path, &target.to_language_string(), None),
                StageInput::none(),
                &test.ctx(),
            )
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::MissingParam);
    }

    #[test]
    fn the_plan_creates_one_file() {
        let (test, path) = with_file("/data/a.ipynb", NOTEBOOK.as_bytes());
        let target = path.parent().unwrap().join_name("a.py").unwrap();
        let plan = NotebookExport
            .plan(
                &args(&path, &target.to_language_string(), Some("script")),
                StageInput::none(),
                &test.ctx(),
            )
            .unwrap();
        assert_eq!(plan.changes.len(), 1);
        assert_eq!(plan.changes[0].path.as_str(), target.to_language_string());
        assert_eq!(plan.changes[0].size_after, Some(Size::from_bytes(11)));
        assert!(!test.fs().exists(&target));
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
