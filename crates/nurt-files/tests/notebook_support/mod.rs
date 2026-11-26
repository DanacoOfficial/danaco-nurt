// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Notebooks and helpers shared by the tests of the `notebook` commands.
//!
//! The notebook of the examples is written the way Jupyter writes it: one
//! space of indentation, sorted keys and sources as lists of lines. It has a
//! Markdown cell, a code cell with a text output and a code cell with an
//! image output in base64.

// Each test binary uses a different part of these helpers.
#![allow(dead_code)]

use std::sync::Arc;

use nurt_contracts::error::Result;
use nurt_contracts::fs::AbsPath;
use nurt_contracts::journal::{JournalStore, TargetIndex, TargetPath};
use nurt_contracts::record::{Cell, Change, Effect};
use nurt_contracts::stage::{ArgValue, BoundArgs, StageImpl, StageInput};
use nurt_contracts::value::{Fields, Value};
use nurt_testkit::ctx::TestContext;
use nurt_testkit::fixtures::begin_request;
use nurt_testkit::journal::JournalCall;
use nurt_testkit::vfs::test_path;

#[path = "../support_report/mod.rs"]
mod support_report;

/// Notebook of the examples.
pub(crate) const SALES: &str = "/home/user/sales.ipynb";

/// Markdown cell with the title.
pub(crate) const INTRO: &str = r##"  {
   "cell_type": "markdown",
   "id": "intro",
   "metadata": {},
   "source": [
    "# Sales\n",
    "Monthly totals."
   ]
  }"##;

/// Code cell that loads the data, with a stream output.
pub(crate) const LOAD: &str = r##"  {
   "cell_type": "code",
   "execution_count": 1,
   "id": "load",
   "metadata": {},
   "outputs": [
    {
     "name": "stdout",
     "output_type": "stream",
     "text": [
      "12 rows\n"
     ]
    }
   ],
   "source": [
    "import pandas as pd\n",
    "df = pd.read_csv(\"sales.csv\")\n",
    "print(len(df), \"rows\")"
   ]
  }"##;

/// Code cell that plots the data, with an image output.
pub(crate) const PLOT: &str = r##"  {
   "cell_type": "code",
   "execution_count": 2,
   "id": "plot",
   "metadata": {},
   "outputs": [
    {
     "data": {
      "image/png": "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGNgYGD4DwABBAEAwS2OUAAAAABJRU5ErkJggg==",
      "text/plain": [
       "<Figure size 640x480 with 1 Axes>"
      ]
     },
     "metadata": {},
     "output_type": "display_data"
    }
   ],
   "source": [
    "df.plot(x=\"month\", y=\"total\")"
   ]
  }"##;

/// Notebook text with the given cell texts and `nbformat_minor`, in the
/// layout of Jupyter.
pub(crate) fn notebook_with_minor(cells: &[&str], minor: u64) -> String {
    format!(
        "{{\n \"cells\": [\n{}\n ],\n \"metadata\": {{\n  \"kernelspec\": {{\n   \"display_name\": \"Python 3\",\n   \"language\": \"python\",\n   \"name\": \"python3\"\n  }},\n  \"language_info\": {{\n   \"name\": \"python\"\n  }}\n }},\n \"nbformat\": 4,\n \"nbformat_minor\": {minor}\n}}\n",
        cells.join(",\n")
    )
}

/// Notebook text in nbformat 4.5 with the given cell texts.
pub(crate) fn notebook(cells: &[&str]) -> String {
    notebook_with_minor(cells, 5)
}

/// The notebook of the examples.
pub(crate) fn sales() -> String {
    notebook(&[INTRO, LOAD, PLOT])
}

/// Cell text without its `id` member, as written before nbformat 4.5.
pub(crate) fn without_id(cell: &str) -> String {
    cell.lines()
        .filter(|line| !line.trim_start().starts_with("\"id\": "))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Absolute path of the virtual file system.
pub(crate) fn abs(path: &str) -> AbsPath {
    test_path(path).expect("test paths are absolute")
}

/// Path as written in programs.
pub(crate) fn language(path: &str) -> String {
    abs(path).to_language_string()
}

/// Test context whose file system holds one notebook.
pub(crate) fn with_notebook(path: &str, text: &str) -> TestContext {
    let test = TestContext::new();
    test.fs()
        .write_file(&abs(path), text.as_bytes())
        .expect("the virtual file system accepts the file");
    test
}

/// Content of a file as text.
pub(crate) fn read(test: &TestContext, path: &str) -> String {
    String::from_utf8(test.fs().read_file(&abs(path)).expect("the file exists"))
        .expect("the file is UTF-8")
}

/// Registered stage of a command.
pub(crate) fn stage(command: &str) -> Arc<dyn StageImpl> {
    nurt_files::stages()
        .into_iter()
        .find(|stage| stage.descriptor().command == command)
        .expect("the command is registered")
}

/// Enumeration value.
pub(crate) fn choice(text: &str) -> Value {
    Value::Enum(text.to_owned())
}

/// Text value.
pub(crate) fn text(text: &str) -> Value {
    Value::Text(text.to_owned())
}

/// List of text values.
pub(crate) fn texts(items: &[&str]) -> Value {
    Value::List(items.iter().map(|item| text(item)).collect())
}

/// Map value.
pub(crate) fn map(entries: &[(&str, Value)]) -> Value {
    Value::Map(
        entries
            .iter()
            .map(|(name, value)| ((*name).to_owned(), value.clone()))
            .collect::<Fields>(),
    )
}

/// Arguments of `command` for the notebook `path`.
pub(crate) fn args(command: &str, path: &str, given: &[(&str, Value)]) -> BoundArgs {
    given.iter().fold(
        BoundArgs::new(command).with("path", ArgValue::Value(Value::Path(language(path))), false),
        |args, (name, value)| args.with(*name, ArgValue::Value(value.clone()), false),
    )
}

/// Records of `notebook.cells`.
pub(crate) fn cells(test: &TestContext, args: &BoundArgs) -> Result<Vec<Cell>> {
    let stage = stage(args.command());
    let ctx = test.ctx();
    let output =
        stage
            .as_read()
            .expect("notebook.cells reads")
            .execute(args, StageInput::none(), &ctx)?;
    StageInput::from(output)
        .into_records()?
        .map(|record| Ok(Cell::try_from(record?).expect("every record is a cell")))
        .collect()
}

/// Changes of the plan of an effect command.
pub(crate) fn plan(test: &TestContext, args: &BoundArgs) -> Result<Vec<Change>> {
    let stage = stage(args.command());
    let ctx = test.ctx();
    let effect = stage.as_effect().expect("the command changes files");
    let changes = effect.plan(args, StageInput::none(), &ctx)?.changes;
    support_report::assert_plan_valid(args.command(), &changes);
    Ok(changes)
}

/// Plans, applies and commits an effect command; returns its `effect`
/// record.
pub(crate) fn run(test: &TestContext, args: &BoundArgs) -> Result<Effect> {
    let stage = stage(args.command());
    let ctx = test.ctx();
    let effect = stage.as_effect().expect("the command changes files");
    let plan = effect.plan(args, StageInput::none(), &ctx)?;
    support_report::assert_plan_valid(args.command(), &plan.changes);
    let mut tx = test.journal().begin(begin_request(args.command()));
    let outcome = effect.apply(args, plan, tx.as_mut(), &ctx)?;
    support_report::assert_changes_valid(args.command(), &outcome.changes);
    tx.commit()?;
    let record = StageInput::from(outcome.output)
        .into_records()?
        .next()
        .expect("the command produces an effect record")?;
    Ok(Effect::try_from(record).expect("the record is an effect"))
}

/// Counter of the summary of an effect.
pub(crate) fn counter(effect: &Effect, name: &str) -> Option<u64> {
    effect.summary.as_ref()?.get(name).copied()
}

/// Map member of the `after` value of an effect.
pub(crate) fn after(effect: &Effect, name: &str) -> Option<Value> {
    effect.after.as_ref()?.get(name).cloned()
}

/// Targets declared to the journal, in order.
pub(crate) fn declared(test: &TestContext) -> Vec<TargetPath> {
    test.journal()
        .calls()
        .into_iter()
        .filter_map(|call| match call {
            JournalCall::Preserve { targets, .. } => Some(targets),
            _ => None,
        })
        .flatten()
        .collect()
}

/// Targets changed successfully, in order.
pub(crate) fn applied(test: &TestContext) -> Vec<TargetIndex> {
    test.journal()
        .calls()
        .into_iter()
        .filter_map(|call| match call {
            JournalCall::Apply {
                index, ok: true, ..
            } => Some(index),
            _ => None,
        })
        .collect()
}

/// Whether the journal received any call that changes a target.
pub(crate) fn journal_untouched(test: &TestContext) -> bool {
    !test.journal().calls().iter().any(|call| {
        matches!(
            call,
            JournalCall::Preserve { .. } | JournalCall::Apply { .. }
        )
    })
}
