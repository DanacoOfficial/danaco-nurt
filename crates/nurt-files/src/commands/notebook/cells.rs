// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-files::commands::notebook::cells
// Responsibility:   `notebook.cells`: the cells of a notebook as `cell` records.
// Dependencies:     nurt_contracts, nurt_notebook
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! `notebook.cells`: the cells of a notebook as `cell` records.
//!
//! Without `index`, `id` and `find`, every cell is listed as an outline: the
//! first non-blank line of its source, without metadata. With any of them,
//! the selected cells are listed in full. A `view` given explicitly overrides
//! that choice. Records come in the order of the cells and carry the entity
//! tag of the file, which the editing commands accept as `if_match`.

use std::sync::Arc;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::record::CellType;
use nurt_contracts::stage::{
    BoundArgs, CheckCtx, ExecCtx, InputKinds, OrderKey, ReadStage, RecordStream, StageDescriptor,
    StageImpl, StageInput, StageOutput, StaticInfo, StreamOrder,
};
use nurt_contracts::value::{Range, Record};
use nurt_notebook::{CellQuery, OutputsDetail, Scope, View};

use super::{
    FIND, FindPattern, ID, INDEX, PATH, cell_record, cell_type_param, choice, count, find_param,
    ids_param, load, notebook_error, notebook_path, range_param, static_info,
};
use crate::common::etag::etag_text;

const COMMAND: &str = "notebook.cells";
const OUTPUTS: &str = "outputs";
const SCOPE: &str = "scope";
const VIEW: &str = "view";
const OUTPUTS_VALUES: [&str; 4] = ["none", "summary", "text", "full"];
const SCOPE_VALUES: [&str; 2] = ["source", "outputs"];
const VIEW_VALUES: [&str; 2] = ["outline", "full"];

/// Field the records are ordered by.
const ORDER_FIELD: &str = "index";

/// Implementation of `notebook.cells`.
pub(crate) struct NotebookCells;

/// Stages of this module.
pub(crate) fn stages() -> Vec<Arc<dyn StageImpl>> {
    vec![Arc::new(NotebookCells)]
}

impl StageImpl for NotebookCells {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: COMMAND.to_owned(),
            source: true,
            input: InputKinds::None,
        }
    }

    fn check(&self, args: &BoundArgs, _ctx: &CheckCtx<'_>) -> Result<StaticInfo> {
        CellsParams::read(args)?;
        Ok(static_info(args, &[PATH]))
    }

    fn as_read(&self) -> Option<&dyn ReadStage> {
        Some(self)
    }
}

impl ReadStage for NotebookCells {
    fn execute<'a>(
        &'a self,
        args: &'a BoundArgs,
        _input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<StageOutput<'a>> {
        let params = CellsParams::read(args)?;
        let path = notebook_path(args, ctx)?;
        let loaded = load(ctx, &path)?;
        let cells = loaded
            .notebook
            .cells(&params.query())
            .map_err(|error| notebook_error(&error, &path))?;
        let limit = ctx.limits().max_records;
        if count(cells.len()) > limit {
            return Err(Error::new(
                ErrorCode::Limit,
                format!(
                    "`{path}` has {} matching cells, more than the limit of {limit} records: select cells with `index` or `id`",
                    cells.len()
                ),
            ));
        }
        let etag = etag_text(&loaded.etag());
        let records: Vec<Record> = cells
            .into_iter()
            .map(|data| Record::from(cell_record(&path, &etag, data)))
            .collect();
        Ok(
            StageOutput::records(RecordStream::from_records(records)).with_order(StreamOrder {
                keys: vec![OrderKey {
                    field: ORDER_FIELD.to_owned(),
                    descending: false,
                }],
            }),
        )
    }
}

/// Parameters of `notebook.cells`.
struct CellsParams {
    index: Option<Range>,
    ids: Vec<String>,
    cell_type: Option<CellType>,
    outputs: OutputsDetail,
    find: Option<FindPattern>,
    scope: Scope,
    view: Option<View>,
}

impl CellsParams {
    fn read(args: &BoundArgs) -> Result<Self> {
        let outputs = match choice(args, OUTPUTS, &OUTPUTS_VALUES)? {
            Some("summary") => OutputsDetail::Summary,
            Some("text") => OutputsDetail::Text,
            Some("full") => OutputsDetail::Full,
            _ => OutputsDetail::None,
        };
        let scope = match choice(args, SCOPE, &SCOPE_VALUES)? {
            Some("outputs") => Scope::Outputs,
            _ => Scope::Source,
        };
        let view = choice(args, VIEW, &VIEW_VALUES)?;
        let view = if args.is_given(VIEW) {
            Some(match view {
                Some("full") => View::Full,
                _ => View::Outline,
            })
        } else {
            None
        };
        Ok(Self {
            index: range_param(args, INDEX)?,
            ids: ids_param(args, ID)?,
            cell_type: cell_type_param(args, super::CELL_TYPE)?,
            outputs,
            find: find_param(args, FIND, false)?,
            scope,
            view,
        })
    }

    fn query(&self) -> CellQuery<'_> {
        CellQuery {
            index: self.index,
            ids: self.ids.clone(),
            cell_type: self.cell_type,
            outputs: self.outputs,
            find: self.find.as_ref().map(FindPattern::as_pattern),
            scope: self.scope,
            view: self.view,
        }
    }
}

#[cfg(test)]
mod tests {
    use nurt_contracts::record::Cell;
    use nurt_contracts::stage::ArgValue;
    use nurt_contracts::value::Value;

    use super::*;
    use crate::commands::notebook::fixture::with_file;

    const NOTEBOOK: &str = r##"{"cells": [
 {"cell_type": "markdown", "metadata": {}, "source": "# Title\n\nText"},
 {"cell_type": "code", "execution_count": 1, "metadata": {"tags": ["a"]}, "outputs": [], "source": "\nx = 1\ny = 2"}
], "metadata": {}, "nbformat": 4, "nbformat_minor": 4}"##;

    fn run(given: &[(&str, Value)]) -> Result<Vec<Cell>> {
        let (test, path) = with_file("/data/a.ipynb", NOTEBOOK.as_bytes());
        let args = given.iter().fold(
            BoundArgs::new(COMMAND).with(
                PATH,
                ArgValue::Value(Value::Path(path.to_language_string())),
                false,
            ),
            |args, (name, value)| args.with(*name, ArgValue::Value(value.clone()), false),
        );
        let ctx = test.ctx();
        let output = NotebookCells.execute(&args, StageInput::none(), &ctx)?;
        StageInput::from(output)
            .into_records()?
            .map(|record| Ok(Cell::try_from(record?).unwrap()))
            .collect()
    }

    #[test]
    fn lists_an_outline_by_default() {
        let cells = run(&[]).unwrap();
        let sources: Vec<&str> = cells.iter().map(|cell| cell.source.as_str()).collect();
        assert_eq!(sources, ["# Title", "x = 1"]);
        assert!(cells.iter().all(|cell| cell.metadata.is_none()));
        assert_eq!(cells[1].execution_count, Some(1));
    }

    #[test]
    fn selected_cells_are_listed_in_full() {
        let cells = run(&[("index", Value::Int(1))]).unwrap();
        assert_eq!(cells.len(), 1);
        assert_eq!(cells[0].source, "\nx = 1\ny = 2");
        assert!(cells[0].metadata.is_some());
    }

    #[test]
    fn an_explicit_view_overrides_the_selection() {
        let cells = run(&[
            ("index", Value::Int(1)),
            ("view", Value::Enum("outline".to_owned())),
        ])
        .unwrap();
        assert_eq!(cells[0].source, "x = 1");
    }

    #[test]
    fn filters_by_type() {
        let cells = run(&[("cell_type", Value::Enum("markdown".to_owned()))]).unwrap();
        assert_eq!(cells.len(), 1);
        assert_eq!(cells[0].cell_type, CellType::Markdown);
    }

    #[test]
    fn a_range_past_the_cells_is_out_of_range() {
        let range = Range::new(Some(5), Some(6)).unwrap();
        let error = run(&[("index", Value::Range(range))]).unwrap_err();
        assert_eq!(error.code(), ErrorCode::OutOfRange);
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
