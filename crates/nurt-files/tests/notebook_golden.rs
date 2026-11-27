// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-files/tests/notebook_golden.rs
// Responsibility:   The examples of the `notebook.*` pages of the command documentation, run on the virtual file system.
// Dependencies:     notebook_support, nurt_contracts, nurt_notebook, serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! The examples of the `notebook.*` pages of the command documentation, run
//! on the virtual file system.

// Tests report failures by panicking.
#![allow(clippy::expect_used)]

mod notebook_support;

use nurt_contracts::error::ErrorCode;
use nurt_contracts::record::CellType;
use nurt_contracts::value::{Range, Regex, Value};
use nurt_notebook::Notebook;
use serde_json::json;

use notebook_support::{
    INTRO, LOAD, PLOT, SALES, after, args, cells, choice, counter, map, notebook,
    notebook_with_minor, read, run, sales, text, texts, with_notebook, without_id,
};

fn regex(pattern: &str) -> Value {
    Value::Regex(Regex::new(pattern, "").expect("the pattern has valid flags"))
}

fn parsed(text: &str) -> Notebook {
    Notebook::parse(text.as_bytes()).expect("the result is a notebook")
}

fn ids(text: &str) -> Vec<Option<String>> {
    let notebook = parsed(text);
    (0..notebook.len())
        .map(|index| notebook.cell(index).expect("the cell exists").id)
        .collect()
}

fn generated_id(value: Option<Value>) -> String {
    let id = value
        .and_then(|value| value.as_str().map(str::to_owned))
        .expect("the new cell has an id");
    assert_eq!(id.len(), 8, "{id}");
    assert!(id.bytes().all(|byte| byte.is_ascii_hexdigit()), "{id}");
    id
}

#[test]
fn cells_example_lists_an_outline() {
    let test = with_notebook(SALES, &sales());
    let cells = cells(&test, &args("notebook.cells", SALES, &[])).expect("the example runs");
    let sources: Vec<&str> = cells.iter().map(|cell| cell.source.as_str()).collect();
    assert_eq!(
        sources,
        [
            "# Sales",
            "import pandas as pd",
            "df.plot(x=\"month\", y=\"total\")"
        ]
    );
    let ids: Vec<Option<&str>> = cells.iter().map(|cell| cell.id.as_deref()).collect();
    assert_eq!(ids, [Some("intro"), Some("load"), Some("plot")]);
    assert!(
        cells
            .iter()
            .all(|cell| cell.metadata.is_none() && cell.outputs.is_none())
    );
    assert_eq!(cells[2].execution_count, Some(2));
    assert!(
        cells
            .iter()
            .all(|cell| cell.etag == cells[0].etag && !cell.etag.is_empty())
    );
}

#[test]
fn cells_example_shows_one_cell_with_text_outputs() {
    let test = with_notebook(SALES, &sales());
    let cells = cells(
        &test,
        &args(
            "notebook.cells",
            SALES,
            &[("index", Value::Int(1)), ("outputs", choice("text"))],
        ),
    )
    .expect("the example runs");
    assert_eq!(cells.len(), 1);
    assert_eq!(
        cells[0].source,
        "import pandas as pd\ndf = pd.read_csv(\"sales.csv\")\nprint(len(df), \"rows\")"
    );
    let outputs: Vec<serde_json::Value> = cells[0]
        .outputs
        .as_ref()
        .expect("outputs are listed")
        .iter()
        .map(Value::to_plain_json)
        .collect();
    assert_eq!(
        outputs,
        [json!({"output_type": "stream", "name": "stdout", "text": "12 rows\n"})]
    );
    assert_eq!(
        cells[0].outputs_summary.as_deref(),
        Some("1 output: stream stdout")
    );
    assert!(cells[0].metadata.is_some());
}

#[test]
fn cells_example_finds_cells_by_pattern() {
    let test = with_notebook(SALES, &sales());
    let cells = cells(
        &test,
        &args("notebook.cells", SALES, &[("find", regex(r"read_\w+"))]),
    )
    .expect("the example runs");
    assert_eq!(cells.len(), 1);
    assert_eq!(cells[0].index, 1);
    assert!(cells[0].source.contains("pd.read_csv"));
}

#[test]
fn cells_example_summarizes_outputs_of_code_cells() {
    let test = with_notebook(SALES, &sales());
    let cells = cells(
        &test,
        &args(
            "notebook.cells",
            SALES,
            &[
                ("cell_type", choice("code")),
                ("outputs", choice("summary")),
            ],
        ),
    )
    .expect("the example runs");
    let summaries: Vec<Option<&str>> = cells
        .iter()
        .map(|cell| cell.outputs_summary.as_deref())
        .collect();
    assert_eq!(
        summaries,
        [
            Some("1 output: stream stdout"),
            Some("1 output: display_data image/png text/plain")
        ]
    );
    assert!(cells.iter().all(|cell| cell.cell_type == CellType::Code));
}

#[test]
fn insert_example_appends_a_markdown_cell() {
    let test = with_notebook(SALES, &sales());
    let effect = run(
        &test,
        &args(
            "notebook.insert",
            SALES,
            &[
                ("cell_type", choice("markdown")),
                ("source", text("## Totals")),
            ],
        ),
    )
    .expect("the example runs");
    assert_eq!(counter(&effect, "cells_inserted"), Some(1));
    assert_eq!(after(&effect, "index"), Some(Value::Int(3)));
    let id = generated_id(after(&effect, "id"));
    let written = read(&test, SALES);
    assert!(written.starts_with(&format!("{{\n \"cells\": [\n{INTRO},\n{LOAD},\n{PLOT},\n")));
    let cell = parsed(&written).cell(3).expect("the new cell exists");
    assert_eq!(cell.cell_type, CellType::Markdown);
    assert_eq!(cell.source, "## Totals");
    assert_eq!(cell.id, Some(id));
}

#[test]
fn insert_example_places_a_code_cell_at_an_index() {
    let test = with_notebook(SALES, &sales());
    let effect = run(
        &test,
        &args(
            "notebook.insert",
            SALES,
            &[
                ("index", Value::Int(1)),
                ("cell_type", choice("code")),
                ("source", text("import numpy as np")),
            ],
        ),
    )
    .expect("the example runs");
    let id = generated_id(after(&effect, "id"));
    let written = read(&test, SALES);
    assert_eq!(
        ids(&written),
        [
            Some("intro".to_owned()),
            Some(id),
            Some("load".to_owned()),
            Some("plot".to_owned())
        ]
    );
    let cell = parsed(&written).cell(1).expect("the new cell exists");
    assert_eq!(cell.source, "import numpy as np");
    assert_eq!(cell.execution_count, None);
    assert_eq!(cell.outputs, Some(Vec::new()));
    assert!(written.contains(LOAD) && written.contains(PLOT));
}

#[test]
fn insert_example_places_a_cell_after_an_identifier() {
    let test = with_notebook(SALES, &sales());
    let effect = run(
        &test,
        &args(
            "notebook.insert",
            SALES,
            &[
                ("after_id", text("load")),
                ("cell_type", choice("code")),
                ("source", text("df.describe()")),
            ],
        ),
    )
    .expect("the example runs");
    assert_eq!(after(&effect, "index"), Some(Value::Int(2)));
    let cell = parsed(&read(&test, SALES))
        .cell(2)
        .expect("the new cell exists");
    assert_eq!(cell.source, "df.describe()");
}

#[test]
fn replace_example_sets_the_source_of_a_cell() {
    let test = with_notebook(SALES, &sales());
    let effect = run(
        &test,
        &args(
            "notebook.replace",
            SALES,
            &[
                ("index", Value::Int(2)),
                (
                    "source",
                    text("df.plot(x=\"month\", y=\"total\", kind=\"bar\")"),
                ),
            ],
        ),
    )
    .expect("the example runs");
    assert_eq!(counter(&effect, "cells_changed"), Some(1));
    let plot = PLOT.replace(
        r#""df.plot(x=\"month\", y=\"total\")""#,
        r#""df.plot(x=\"month\", y=\"total\", kind=\"bar\")""#,
    );
    assert_eq!(read(&test, SALES), notebook(&[INTRO, LOAD, &plot]));
}

#[test]
fn replace_example_renames_a_variable_in_every_cell() {
    let test = with_notebook(SALES, &sales());
    let effect = run(
        &test,
        &args(
            "notebook.replace",
            SALES,
            &[
                ("find", text("df")),
                ("with", text("sales")),
                ("whole_word", Value::Bool(true)),
            ],
        ),
    )
    .expect("the example runs");
    assert_eq!(counter(&effect, "replacements"), Some(3));
    assert_eq!(counter(&effect, "cells_changed"), Some(2));
    let changes = effect.changes.expect("the effect lists its changes");
    assert_eq!(changes[0].replacements, Some(3));
    let load = LOAD
        .replace("\"df = ", "\"sales = ")
        .replace("len(df)", "len(sales)");
    let plot = PLOT.replace("\"df.plot", "\"sales.plot");
    assert_eq!(read(&test, SALES), notebook(&[INTRO, &load, &plot]));
}

#[test]
fn replace_example_rewrites_a_call_with_a_regular_expression() {
    let test = with_notebook(SALES, &sales());
    let effect = run(
        &test,
        &args(
            "notebook.replace",
            SALES,
            &[
                ("id", text("load")),
                ("find", regex(r"read_(\w+)")),
                ("with", text("load_$1")),
            ],
        ),
    )
    .expect("the example runs");
    assert_eq!(counter(&effect, "replacements"), Some(1));
    let load = LOAD.replace("pd.read_csv", "pd.load_csv");
    assert_eq!(read(&test, SALES), notebook(&[INTRO, &load, PLOT]));
}

#[test]
fn replace_example_resets_the_outputs_of_a_cell() {
    let test = with_notebook(SALES, &sales());
    run(
        &test,
        &args(
            "notebook.replace",
            SALES,
            &[("id", text("load")), ("reset_outputs", Value::Bool(true))],
        ),
    )
    .expect("the example runs");
    let written = read(&test, SALES);
    let cell = parsed(&written).cell(1).expect("the cell exists");
    assert_eq!(cell.outputs, Some(Vec::new()));
    assert_eq!(cell.execution_count, None);
    assert!(written.contains(INTRO) && written.contains(PLOT));
    assert!(!written.contains("12 rows"));
}

#[test]
fn move_example_moves_the_last_cell_to_the_front() {
    let test = with_notebook(SALES, &sales());
    let effect = run(
        &test,
        &args(
            "notebook.move",
            SALES,
            &[("from", Value::Int(2)), ("to", Value::Int(0))],
        ),
    )
    .expect("the example runs");
    assert_eq!(counter(&effect, "cells_moved"), Some(1));
    assert_eq!(read(&test, SALES), notebook(&[PLOT, INTRO, LOAD]));
}

#[test]
fn move_example_swaps_the_first_two_cells() {
    let test = with_notebook(SALES, &sales());
    run(
        &test,
        &args(
            "notebook.move",
            SALES,
            &[("from", Value::Int(0)), ("to", Value::Int(1))],
        ),
    )
    .expect("the example runs");
    assert_eq!(read(&test, SALES), notebook(&[LOAD, INTRO, PLOT]));
}

#[test]
fn move_example_rejects_a_position_past_the_end() {
    let test = with_notebook(SALES, &sales());
    let error = run(
        &test,
        &args(
            "notebook.move",
            SALES,
            &[("from", Value::Int(0)), ("to", Value::Int(3))],
        ),
    )
    .expect_err("index 3 is past the last cell");
    assert_eq!(error.code(), ErrorCode::OutOfRange);
    assert_eq!(error.expected, Some(json!("0..2")));
    assert_eq!(read(&test, SALES), sales());
}

#[test]
fn copy_example_duplicates_a_cell_after_itself() {
    let test = with_notebook(SALES, &sales());
    let effect = run(
        &test,
        &args(
            "notebook.copy",
            SALES,
            &[("from", Value::Int(1)), ("to", Value::Int(2))],
        ),
    )
    .expect("the example runs");
    assert_eq!(counter(&effect, "cells_copied"), Some(1));
    let id = generated_id(after(&effect, "id"));
    let copy = LOAD.replace("\"id\": \"load\"", &format!("\"id\": \"{id}\""));
    assert_eq!(read(&test, SALES), notebook(&[INTRO, LOAD, &copy, PLOT]));
}

#[test]
fn copy_example_repeats_a_cell_without_identifiers() {
    let (intro, load, plot) = (without_id(INTRO), without_id(LOAD), without_id(PLOT));
    let original = notebook_with_minor(&[&intro, &load, &plot], 4);
    let test = with_notebook(SALES, &original);
    let effect = run(
        &test,
        &args(
            "notebook.copy",
            SALES,
            &[("from", Value::Int(0)), ("to", Value::Int(0))],
        ),
    )
    .expect("the example runs");
    assert_eq!(after(&effect, "id"), Some(Value::Null));
    assert_eq!(
        read(&test, SALES),
        notebook_with_minor(&[&intro, &intro, &load, &plot], 4)
    );
}

#[test]
fn copy_example_rejects_an_unknown_cell() {
    let test = with_notebook(SALES, &sales());
    let error = run(
        &test,
        &args(
            "notebook.copy",
            SALES,
            &[("from", Value::Int(9)), ("to", Value::Int(0))],
        ),
    )
    .expect_err("cell 9 does not exist");
    assert_eq!(error.code(), ErrorCode::OutOfRange);
    assert!(
        error.fix[0].starts_with("notebook.cells "),
        "{:?}",
        error.fix
    );
}

#[test]
fn remove_example_removes_one_cell() {
    let test = with_notebook(SALES, &sales());
    let effect = run(
        &test,
        &args("notebook.remove", SALES, &[("index", Value::Int(2))]),
    )
    .expect("the example runs");
    assert_eq!(counter(&effect, "cells_removed"), Some(1));
    assert_eq!(read(&test, SALES), notebook(&[INTRO, LOAD]));
}

#[test]
fn remove_example_removes_cells_by_identifier() {
    let test = with_notebook(SALES, &sales());
    let effect = run(
        &test,
        &args(
            "notebook.remove",
            SALES,
            &[("id", texts(&["intro", "plot"]))],
        ),
    )
    .expect("the example runs");
    assert_eq!(counter(&effect, "cells_removed"), Some(2));
    assert_eq!(read(&test, SALES), notebook(&[LOAD]));
}

#[test]
fn remove_example_removes_a_range() {
    let test = with_notebook(SALES, &sales());
    let range = Range::new(Some(0), Some(1)).expect("the range is ordered");
    run(
        &test,
        &args("notebook.remove", SALES, &[("index", Value::Range(range))]),
    )
    .expect("the example runs");
    assert_eq!(read(&test, SALES), notebook(&[PLOT]));
}

#[test]
fn clear_outputs_example_clears_every_cell() {
    let test = with_notebook(SALES, &sales());
    let effect = run(&test, &args("notebook.clear_outputs", SALES, &[])).expect("the example runs");
    assert_eq!(counter(&effect, "cells_cleared"), Some(2));
    let written = read(&test, SALES);
    let notebook = parsed(&written);
    for index in [1, 2] {
        let cell = notebook.cell(index).expect("the cell exists");
        assert_eq!(cell.outputs, Some(Vec::new()));
        assert_eq!(cell.execution_count, None);
    }
    assert!(written.contains(INTRO));
    assert!(!written.contains("iVBORw0KGgo"));
}

#[test]
fn clear_outputs_example_clears_one_cell() {
    let test = with_notebook(SALES, &sales());
    let effect = run(
        &test,
        &args("notebook.clear_outputs", SALES, &[("index", Value::Int(1))]),
    )
    .expect("the example runs");
    assert_eq!(counter(&effect, "cells_cleared"), Some(1));
    let written = read(&test, SALES);
    assert!(written.contains(PLOT));
    assert!(!written.contains("12 rows"));
}

#[test]
fn clear_outputs_example_clears_cells_by_identifier() {
    let test = with_notebook(SALES, &sales());
    run(
        &test,
        &args("notebook.clear_outputs", SALES, &[("id", texts(&["plot"]))]),
    )
    .expect("the example runs");
    let written = read(&test, SALES);
    assert!(written.contains(LOAD));
    assert!(!written.contains("iVBORw0KGgo"));
}

#[test]
fn normalize_example_adds_identifiers() {
    let original = notebook_with_minor(
        &[&without_id(INTRO), &without_id(LOAD), &without_id(PLOT)],
        4,
    );
    let test = with_notebook(SALES, &original);
    let effect = run(&test, &args("notebook.normalize", SALES, &[])).expect("the example runs");
    assert_eq!(counter(&effect, "cells_changed"), Some(3));
    let written = read(&test, SALES);
    let notebook = parsed(&written);
    assert_eq!(notebook.nbformat(), (4, 5));
    let ids: Vec<String> = ids(&written).into_iter().flatten().collect();
    assert_eq!(ids.len(), 3);
    assert!(ids.iter().all(|id| id.len() == 8));
    assert!(ids[0] != ids[1] && ids[1] != ids[2] && ids[0] != ids[2]);

    let again = run(&test, &args("notebook.normalize", SALES, &[])).expect("normalize runs again");
    assert_eq!(again.targets, Some(0));
    assert_eq!(read(&test, SALES), written);
}

#[test]
fn normalize_example_repairs_duplicate_identifiers() {
    let plot = PLOT.replace("\"id\": \"plot\"", "\"id\": \"load\"");
    let test = with_notebook(SALES, &notebook(&[INTRO, LOAD, &plot]));
    run(&test, &args("notebook.normalize", SALES, &[])).expect("the example runs");
    let written = read(&test, SALES);
    let ids = ids(&written);
    assert_eq!(ids[1].as_deref(), Some("load"));
    assert!(
        ids[2]
            .as_deref()
            .is_some_and(|id| id != "load" && id.len() == 8)
    );
    assert!(written.contains(INTRO) && written.contains(LOAD));
}

#[test]
fn normalize_example_adds_missing_members_of_code_cells() {
    let original = "{\"cells\": [{\"cell_type\": \"code\", \"id\": \"a\", \"metadata\": {}, \"source\": \"x = 1\"}], \"metadata\": {}, \"nbformat\": 4, \"nbformat_minor\": 5}\n";
    let test = with_notebook(SALES, original);
    let effect = run(&test, &args("notebook.normalize", SALES, &[])).expect("the example runs");
    assert_eq!(counter(&effect, "cells_changed"), Some(1));
    let written = read(&test, SALES);
    assert!(written.contains("\"outputs\": []"), "{written}");
    assert!(written.contains("\"execution_count\": null"), "{written}");
    assert!(written.contains("\"source\": \"x = 1\""), "{written}");
}

#[test]
fn ops_example_inserts_and_removes_in_one_write() {
    let test = with_notebook(SALES, &sales());
    let changes = Value::List(vec![
        map(&[
            ("action", text("insert")),
            ("after_id", text("intro")),
            ("cell_type", text("markdown")),
            ("source", text("Data source: sales.csv")),
        ]),
        map(&[("action", text("remove")), ("id", text("plot"))]),
    ]);
    let effect = run(&test, &args("notebook.ops", SALES, &[("changes", changes)]))
        .expect("the example runs");
    assert_eq!(counter(&effect, "operations"), Some(2));
    assert_eq!(counter(&effect, "cells_inserted"), Some(1));
    assert_eq!(counter(&effect, "cells_removed"), Some(1));
    let written = read(&test, SALES);
    let notebook = parsed(&written);
    assert_eq!(notebook.len(), 3);
    assert_eq!(
        notebook.cell(1).expect("the new cell exists").source,
        "Data source: sales.csv"
    );
    assert!(written.contains(INTRO) && written.contains(LOAD));
    let new_cells = effect.after.expect("the new cell is reported");
    let first = &new_cells.as_list().expect("a list of new cells")[0];
    assert_eq!(first.get("change"), Some(&Value::Int(0)));
    assert_eq!(first.get("index"), Some(&Value::Int(1)));
}

#[test]
fn ops_example_renames_and_clears_outputs() {
    let test = with_notebook(SALES, &sales());
    let changes = Value::List(vec![
        map(&[
            ("action", text("replace")),
            ("id", text("load")),
            ("find", text("df")),
            ("with", text("sales")),
            ("whole_word", Value::Bool(true)),
        ]),
        map(&[("action", text("clear_outputs"))]),
    ]);
    let effect = run(&test, &args("notebook.ops", SALES, &[("changes", changes)]))
        .expect("the example runs");
    assert_eq!(counter(&effect, "replacements"), Some(2));
    assert_eq!(counter(&effect, "cells_cleared"), Some(2));
    let written = read(&test, SALES);
    let notebook = parsed(&written);
    let load = notebook.cell(1).expect("the cell exists");
    assert!(load.source.contains("sales = pd.read_csv"));
    assert_eq!(load.outputs, Some(Vec::new()));
    let plot = notebook.cell(2).expect("the cell exists");
    assert_eq!(plot.source, "df.plot(x=\"month\", y=\"total\")");
    assert_eq!(plot.outputs, Some(Vec::new()));
}

#[test]
fn ops_example_changes_nothing_when_one_operation_fails() {
    let test = with_notebook(SALES, &sales());
    let changes = Value::List(vec![
        map(&[("action", text("remove")), ("index", Value::Int(0))]),
        map(&[
            ("action", text("move")),
            ("index", Value::Int(9)),
            ("to", Value::Int(0)),
        ]),
    ]);
    let error = run(&test, &args("notebook.ops", SALES, &[("changes", changes)]))
        .expect_err("the second change names no cell");
    assert_eq!(error.code(), ErrorCode::OutOfRange);
    assert_eq!(error.details.get("change"), Some(&json!(1)));
    assert!(
        error.message.contains("cell index 9 is out of range"),
        "{}",
        error.message
    );
    assert_eq!(read(&test, SALES), sales());
}

#[test]
fn export_example_writes_a_percent_script() {
    let test = with_notebook(SALES, &sales());
    let effect = run(
        &test,
        &args(
            "notebook.export",
            SALES,
            &[
                (
                    "to",
                    Value::Path(notebook_support::language("/home/user/sales.py")),
                ),
                ("format", choice("script")),
            ],
        ),
    )
    .expect("the example runs");
    assert_eq!(counter(&effect, "files_created"), Some(1));
    assert_eq!(counter(&effect, "cells_exported"), Some(3));
    assert_eq!(
        read(&test, "/home/user/sales.py"),
        "# %% [markdown]\n# # Sales\n# Monthly totals.\n\n# %%\nimport pandas as pd\ndf = pd.read_csv(\"sales.csv\")\nprint(len(df), \"rows\")\n\n# %%\ndf.plot(x=\"month\", y=\"total\")\n"
    );
    assert_eq!(read(&test, SALES), sales());
}

#[test]
fn export_example_writes_markdown_with_text_outputs() {
    let test = with_notebook(SALES, &sales());
    run(
        &test,
        &args(
            "notebook.export",
            SALES,
            &[
                (
                    "to",
                    Value::Path(notebook_support::language("/home/user/sales.md")),
                ),
                ("format", choice("markdown")),
            ],
        ),
    )
    .expect("the example runs");
    assert_eq!(
        read(&test, "/home/user/sales.md"),
        "# Sales\nMonthly totals.\n\n```python\nimport pandas as pd\ndf = pd.read_csv(\"sales.csv\")\nprint(len(df), \"rows\")\n```\n\n```text\n12 rows\n```\n\n```python\ndf.plot(x=\"month\", y=\"total\")\n```\n\n```text\n<Figure size 640x480 with 1 Axes>\n```\n"
    );
}

#[test]
fn export_example_writes_the_sources_as_text() {
    let test = with_notebook(SALES, &sales());
    run(
        &test,
        &args(
            "notebook.export",
            SALES,
            &[
                (
                    "to",
                    Value::Path(notebook_support::language("/home/user/sales.txt")),
                ),
                ("format", choice("text")),
            ],
        ),
    )
    .expect("the example runs");
    assert_eq!(
        read(&test, "/home/user/sales.txt"),
        "# Sales\nMonthly totals.\n\nimport pandas as pd\ndf = pd.read_csv(\"sales.csv\")\nprint(len(df), \"rows\")\n\ndf.plot(x=\"month\", y=\"total\")\n"
    );
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
