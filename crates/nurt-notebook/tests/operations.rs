// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Operations of the command group `notebook` on generated notebooks.
//!
//! Expected results are reference serializations of the expected JSON
//! value, so each test also checks that cells outside the change keep their
//! exact text.

// Fixtures are built with `unwrap`; invalid fixture data fails the test.
#![allow(
    clippy::unwrap_used,
    reason = "fixtures in tests fail the test on invalid data"
)]

mod support;

use nurt_contracts::error::ErrorCode;
use nurt_contracts::record::CellType;
use nurt_contracts::value::Range;
use nurt_notebook::{
    CellQuery, CellRef, Change, ChangeOutcome, ExportFormat, IdGenerator, Literal, Notebook,
    NotebookError, OutputsDetail, Position, Replace, Scope, Selection, View, cells_as_content,
    literal_pattern,
};
use serde_json::{Value, json};
use support::{code, image, markdown, notebook, render, sample, stream};

const INDENT: &str = " ";

fn open(value: &Value) -> Notebook {
    let mut notebook = Notebook::parse(render(value, INDENT).as_bytes()).unwrap();
    notebook.set_id_generator(IdGenerator::sequential());
    notebook
}

fn cells_of(value: &mut Value) -> &mut Vec<Value> {
    value["cells"].as_array_mut().unwrap()
}

fn assert_renders(notebook: &Notebook, expected: &Value) {
    assert_eq!(notebook.render(), render(expected, INDENT));
}

fn change(value: Value) -> Change {
    Change::from_json(value.as_object().unwrap(), &mut literal_pattern).unwrap()
}

#[test]
fn cells_lists_an_outline_by_default() {
    let cells = open(&sample()).cells(&CellQuery::default()).unwrap();
    let outline: Vec<&str> = cells.iter().map(|c| c.source.as_str()).collect();
    assert_eq!(
        outline,
        [
            "# Analysis",
            "import pandas as pd",
            "df.plot()",
            "Notes\tand\u{1b} control characters"
        ]
    );
    assert!(
        cells
            .iter()
            .all(|c| c.metadata.is_none() && c.outputs.is_none())
    );
    assert_eq!(cells[1].id.as_deref(), Some("load"));
}

#[test]
fn cells_selects_by_index_id_and_type() {
    let notebook = open(&sample());
    let by_range = notebook
        .cells(&CellQuery {
            index: Some(Range::from_start(2)),
            ..CellQuery::default()
        })
        .unwrap();
    assert_eq!(by_range.iter().map(|c| c.index).collect::<Vec<_>>(), [2, 3]);
    assert_eq!(by_range[0].source, "df.plot()");
    assert!(by_range[0].metadata.is_some());

    let past_end = notebook
        .cells(&CellQuery {
            index: Some(Range::new(Some(3), Some(99)).unwrap()),
            ..CellQuery::default()
        })
        .unwrap();
    assert_eq!(past_end.len(), 1);

    let by_id = notebook
        .cells(&CellQuery {
            ids: vec!["notes".into(), "intro".into()],
            view: Some(View::Outline),
            ..CellQuery::default()
        })
        .unwrap();
    assert_eq!(by_id.iter().map(|c| c.index).collect::<Vec<_>>(), [0, 3]);
    assert_eq!(by_id[0].source, "# Analysis");

    let code_cells = notebook
        .cells(&CellQuery {
            cell_type: Some(CellType::Code),
            ..CellQuery::default()
        })
        .unwrap();
    assert_eq!(
        code_cells.iter().map(|c| c.index).collect::<Vec<_>>(),
        [1, 2]
    );
}

#[test]
fn cells_finds_text_in_sources_or_outputs() {
    let notebook = open(&sample());
    let df = Literal::new("df", true);
    let in_source = notebook
        .cells(&CellQuery {
            find: Some(&df),
            ..CellQuery::default()
        })
        .unwrap();
    assert_eq!(
        in_source.iter().map(|c| c.index).collect::<Vec<_>>(),
        [1, 2]
    );
    assert!(
        in_source[0].source.contains("read_csv"),
        "full view when find is given"
    );

    let answer = Literal::new("42", false);
    let in_outputs = notebook
        .cells(&CellQuery {
            find: Some(&answer),
            scope: Scope::Outputs,
            ..CellQuery::default()
        })
        .unwrap();
    assert_eq!(in_outputs.iter().map(|c| c.index).collect::<Vec<_>>(), [1]);
}

#[test]
fn cells_returns_outputs_at_each_detail() {
    let notebook = open(&sample());
    let query = |outputs| CellQuery {
        index: Some(Range::single(2)),
        outputs,
        ..CellQuery::default()
    };
    let none = notebook.cells(&query(OutputsDetail::None)).unwrap();
    assert_eq!(
        (none[0].outputs.as_ref(), none[0].outputs_summary.as_ref()),
        (None, None)
    );
    let summary = notebook.cells(&query(OutputsDetail::Summary)).unwrap();
    assert_eq!(
        summary[0].outputs_summary.as_deref(),
        Some("1 output: display_data image/png text/plain")
    );
    assert_eq!(summary[0].outputs, None);
    let text = notebook.cells(&query(OutputsDetail::Text)).unwrap();
    assert_eq!(
        text[0].outputs.as_deref(),
        Some(
            &[json!({"output_type": "display_data", "text": "<Figure size 640x480 with 1 Axes>"})]
                [..]
        )
    );
    let full = notebook.cells(&query(OutputsDetail::Full)).unwrap();
    assert_eq!(full[0].outputs.as_deref(), Some(&[image()][..]));
}

#[test]
fn cells_rejects_unknown_ids_with_candidates() {
    let error = open(&sample())
        .cells(&CellQuery {
            ids: vec!["laod".into()],
            ..CellQuery::default()
        })
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::Arg);
    assert!(error.to_string().contains("closest ids: load"), "{error}");
}

#[test]
fn insert_adds_cells_at_the_end_at_an_index_and_after_an_id() {
    let mut notebook = open(&sample());
    let end = notebook
        .insert(&Position::End, CellType::Code, "x = 1\ny = 2")
        .unwrap();
    let first = notebook
        .insert(&Position::Index(0), CellType::Raw, "raw")
        .unwrap();
    let after = notebook
        .insert(
            &Position::AfterId("load".into()),
            CellType::Markdown,
            "## Plot",
        )
        .unwrap();
    assert_eq!((end.index, end.id.as_deref()), (4, Some("00000001")));
    assert_eq!((first.index, first.id.as_deref()), (0, Some("00000002")));
    assert_eq!((after.index, after.id.as_deref()), (3, Some("00000003")));

    let mut expected = sample();
    let cells = cells_of(&mut expected);
    cells.push(code(Some("00000001"), &["x = 1\n", "y = 2"], json!([])));
    cells.insert(
        0,
        json!({"cell_type": "raw", "id": "00000002", "metadata": {}, "source": ["raw"]}),
    );
    cells.insert(3, markdown(Some("00000003"), &["## Plot"]));
    assert_renders(&notebook, &expected);
}

#[test]
fn insert_without_ids_before_nbformat_4_5() {
    let original = notebook(4, vec![code(None, &["a"], json!([]))]);
    let mut notebook = open(&original);
    let inserted = notebook
        .insert(&Position::Index(1), CellType::Markdown, "b")
        .unwrap();
    assert_eq!(inserted.id, None);
    let mut expected = original;
    cells_of(&mut expected).push(markdown(None, &["b"]));
    assert_renders(&notebook, &expected);
}

#[test]
fn insert_rejects_positions_outside_the_notebook() {
    let mut notebook = open(&sample());
    let error = notebook
        .insert(&Position::Index(5), CellType::Code, "x")
        .unwrap_err();
    assert!(error.to_string().contains("valid indexes: 0..4"), "{error}");
    let error = notebook
        .insert(&Position::AfterId("none".into()), CellType::Code, "x")
        .unwrap_err();
    assert!(matches!(error, NotebookError::UnknownId { .. }));
    assert!(!notebook.is_modified());
}

#[test]
fn replace_sets_the_source_in_the_stored_form() {
    let mut value = sample();
    value["cells"][3]["source"] = json!("one string");
    let mut notebook = open(&value);
    notebook
        .replace(&Replace {
            target: Some(CellRef::Index(3)),
            source: Some("a\nb".into()),
            ..Replace::default()
        })
        .unwrap();
    notebook
        .replace(&Replace {
            target: Some(CellRef::Id("intro".into())),
            source: Some("# New\n".into()),
            ..Replace::default()
        })
        .unwrap();
    value["cells"][3]["source"] = json!("a\nb");
    value["cells"][0]["source"] = json!(["# New\n"]);
    assert_renders(&notebook, &value);
}

#[test]
fn replace_finds_whole_words_in_every_cell() {
    let mut notebook = open(&sample());
    let df = Literal::new("df", true);
    let outcome = notebook
        .replace(&Replace {
            find: Some(&df),
            with: Some("frame".into()),
            ..Replace::default()
        })
        .unwrap();
    assert_eq!((outcome.cells, outcome.replacements), (vec![1, 2], 3));
    let mut expected = sample();
    expected["cells"][1]["source"] = json!([
        "import pandas as pd\n",
        "frame = pd.read_csv('a.csv')\n",
        "print(len(frame))"
    ]);
    expected["cells"][2]["source"] = json!(["frame.plot()"]);
    assert_renders(&notebook, &expected);
}

#[test]
fn replace_changes_type_and_resets_outputs() {
    let mut notebook = open(&sample());
    notebook
        .replace(&Replace {
            target: Some(CellRef::Index(1)),
            reset_outputs: true,
            ..Replace::default()
        })
        .unwrap();
    notebook
        .replace(&Replace {
            target: Some(CellRef::Index(2)),
            cell_type: Some(CellType::Markdown),
            ..Replace::default()
        })
        .unwrap();
    notebook
        .replace(&Replace {
            target: Some(CellRef::Index(3)),
            cell_type: Some(CellType::Code),
            ..Replace::default()
        })
        .unwrap();
    let mut expected = sample();
    expected["cells"][1]["outputs"] = json!([]);
    expected["cells"][1]["execution_count"] = Value::Null;
    expected["cells"][2] =
        json!({"cell_type": "markdown", "id": "plot", "metadata": {}, "source": ["df.plot()"]});
    expected["cells"][3] = json!({
        "cell_type": "code", "execution_count": null, "id": "notes", "metadata": {}, "outputs": [],
        "source": ["Notes\tand\u{1b} control characters"]
    });
    assert_renders(&notebook, &expected);
}

#[test]
fn replace_reports_mismatch_no_match_and_invalid_arguments() {
    let mut notebook = open(&sample());
    let error = notebook
        .replace(&Replace {
            target: Some(CellRef::IndexAndId(0, "plot".into())),
            source: Some("x".into()),
            ..Replace::default()
        })
        .unwrap_err();
    assert!(
        error.to_string().contains("cell `plot` is at index 2"),
        "{error}"
    );
    let missing = Literal::new("missing", false);
    let error = notebook
        .replace(&Replace {
            find: Some(&missing),
            with: Some("x".into()),
            ..Replace::default()
        })
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::NoMatch);
    let error = notebook
        .replace(&Replace {
            target: Some(CellRef::Index(0)),
            ..Replace::default()
        })
        .unwrap_err();
    assert!(
        error.to_string().starts_with("nothing to change"),
        "{error}"
    );
    assert!(!notebook.is_modified());
}

#[test]
fn move_places_the_cell_at_the_target_index() {
    let mut notebook = open(&sample());
    assert_eq!(notebook.move_cell(&CellRef::Index(0), 2).unwrap(), 0);
    assert_eq!(
        notebook.move_cell(&CellRef::Id("notes".into()), 0).unwrap(),
        3
    );
    let mut expected = sample();
    let cells = cells_of(&mut expected);
    let intro = cells.remove(0);
    cells.insert(2, intro);
    let notes = cells.remove(3);
    cells.insert(0, notes);
    assert_renders(&notebook, &expected);
}

#[test]
fn move_to_the_same_index_changes_nothing() {
    let mut notebook = open(&sample());
    notebook.move_cell(&CellRef::Index(1), 1).unwrap();
    assert_eq!(notebook.render(), render(&sample(), INDENT));
    assert!(notebook.move_cell(&CellRef::Index(1), 4).is_err());
}

#[test]
fn copy_repeats_the_cell_with_a_new_id() {
    let mut notebook = open(&sample());
    let copied = notebook
        .copy_cell(&CellRef::Id("load".into()), None)
        .unwrap();
    assert_eq!((copied.index, copied.id.as_deref()), (2, Some("00000001")));
    let to_start = notebook.copy_cell(&CellRef::Index(0), Some(0)).unwrap();
    assert_eq!(to_start.index, 0);
    let mut expected = sample();
    let cells = cells_of(&mut expected);
    let mut load = cells[1].clone();
    load["id"] = json!("00000001");
    cells.insert(2, load);
    let mut intro = cells[0].clone();
    intro["id"] = json!("00000002");
    cells.insert(0, intro);
    assert_renders(&notebook, &expected);
}

#[test]
fn copy_without_ids_repeats_the_text_exactly() {
    let original = notebook(4, vec![code(None, &["a"], json!([stream("1\n")]))]);
    let mut notebook = open(&original);
    let copied = notebook.copy_cell(&CellRef::Index(0), Some(1)).unwrap();
    assert_eq!(copied.id, None);
    let mut expected = original;
    let first = expected["cells"][0].clone();
    cells_of(&mut expected).push(first);
    assert_renders(&notebook, &expected);
}

#[test]
fn remove_deletes_ranges_and_ids() {
    let mut notebook = open(&sample());
    assert_eq!(
        notebook
            .remove(&Selection::Ids(vec!["notes".into(), "intro".into()]))
            .unwrap(),
        [0, 3]
    );
    assert_eq!(
        notebook
            .remove(&Selection::Range(Range::single(1)))
            .unwrap(),
        [1]
    );
    let mut expected = sample();
    let load = expected["cells"][1].clone();
    expected["cells"] = json!([load]);
    assert_renders(&notebook, &expected);
}

#[test]
fn remove_rejects_ranges_past_the_end_without_changes() {
    let mut notebook = open(&sample());
    let error = notebook
        .remove(&Selection::Range(Range::new(Some(2), Some(7)).unwrap()))
        .unwrap_err();
    assert!(matches!(
        error,
        NotebookError::IndexOutOfRange { index: 7, len: 4 }
    ));
    assert!(!notebook.is_modified());
    assert_eq!(notebook.remove(&Selection::All).unwrap().len(), 4);
    assert_renders(&notebook, &notebook_without_cells());
}

fn notebook_without_cells() -> Value {
    let mut value = sample();
    value["cells"] = json!([]);
    value
}

#[test]
fn clear_outputs_empties_outputs_and_execution_counts() {
    let mut notebook = open(&sample());
    assert_eq!(notebook.clear_outputs(&Selection::All).unwrap(), [1, 2]);
    let mut expected = sample();
    for index in [1, 2] {
        expected["cells"][index]["outputs"] = json!([]);
        expected["cells"][index]["execution_count"] = Value::Null;
    }
    assert_renders(&notebook, &expected);
}

#[test]
fn clear_outputs_limits_to_the_selection() {
    let mut notebook = open(&sample());
    assert_eq!(
        notebook
            .clear_outputs(&Selection::Ids(vec!["plot".into()]))
            .unwrap(),
        [2]
    );
    let mut expected = sample();
    expected["cells"][2]["outputs"] = json!([]);
    expected["cells"][2]["execution_count"] = Value::Null;
    assert_renders(&notebook, &expected);
}

#[test]
fn clear_outputs_of_clean_cells_keeps_the_file() {
    let original = notebook(
        4,
        vec![code(None, &["a"], json!([])), markdown(None, &["b"])],
    );
    let mut notebook = open(&original);
    assert!(notebook.clear_outputs(&Selection::All).unwrap().is_empty());
    assert!(!notebook.is_modified());
}

#[test]
fn normalize_assigns_ids_and_upgrades_the_minor_version() {
    let original = notebook(
        4,
        vec![markdown(None, &["a"]), code(None, &["b"], json!([]))],
    );
    let mut notebook = open(&original);
    let outcome = notebook.normalize();
    assert_eq!((outcome.cells, outcome.notebook), (vec![0, 1], true));
    let expected = support::notebook(
        5,
        vec![
            markdown(Some("00000001"), &["a"]),
            code(Some("00000002"), &["b"], json!([])),
        ],
    );
    assert_renders(&notebook, &expected);
}

#[test]
fn normalize_repairs_duplicate_ids_and_misplaced_members() {
    let mut value = sample();
    value["cells"][3]["id"] = json!("intro");
    value["cells"][0]["outputs"] = json!([]);
    let mut notebook = open(&value);
    let outcome = notebook.normalize();
    assert_eq!((outcome.cells, outcome.notebook), (vec![0, 3], false));
    let mut expected = sample();
    expected["cells"][3]["id"] = json!("00000001");
    assert_renders(&notebook, &expected);
}

#[test]
fn normalize_leaves_normal_notebooks_untouched() {
    let mut notebook = open(&sample());
    let outcome = notebook.normalize();
    assert!(outcome.cells.is_empty() && !outcome.notebook);
    assert!(!notebook.is_modified());
}

#[test]
fn ops_applies_changes_in_order() {
    let mut notebook = open(&sample());
    let changes = [
        change(
            json!({"action": "insert", "after_id": "intro", "cell_type": "markdown", "source": "## Load"}),
        ),
        change(
            json!({"action": "replace", "id": "load", "find": "df", "with": "frame", "whole_word": true}),
        ),
        change(json!({"action": "move", "id": "notes", "to": 0})),
        change(json!({"action": "copy", "index": 0})),
        change(json!({"action": "remove", "id": ["plot"]})),
        change(json!({"action": "clear_outputs", "index": "all"})),
    ];
    let outcomes = notebook.apply(&changes).unwrap();
    assert_eq!(outcomes.len(), changes.len());
    assert!(matches!(&outcomes[2], ChangeOutcome::Moved(4)));
    assert!(matches!(&outcomes[5], ChangeOutcome::Cleared(cleared) if cleared == &[4]));

    let mut expected = sample();
    let cells = cells_of(&mut expected);
    let notes = cells[3].clone();
    let mut notes_copy = notes.clone();
    notes_copy["id"] = json!("00000002");
    let mut load = cells[1].clone();
    load["source"] = json!([
        "import pandas as pd\n",
        "frame = pd.read_csv('a.csv')\n",
        "print(len(frame))"
    ]);
    load["outputs"] = json!([]);
    load["execution_count"] = Value::Null;
    let intro = cells[0].clone();
    *cells = vec![
        notes,
        notes_copy,
        intro,
        markdown(Some("00000001"), &["## Load"]),
        load,
    ];
    assert_renders(&notebook, &expected);
}

#[test]
fn ops_with_a_failing_change_leaves_the_notebook_unchanged() {
    let mut notebook = open(&sample());
    let changes = [
        change(json!({"action": "insert", "cell_type": "code", "source": "x"})),
        change(json!({"action": "remove", "index": 0})),
        change(json!({"action": "move", "id": "missing", "to": 0})),
        change(json!({"action": "clear_outputs"})),
    ];
    let error = notebook.apply(&changes).unwrap_err();
    assert!(
        matches!(&error, NotebookError::Change { position: 2, .. }),
        "{error}"
    );
    assert!(
        error
            .to_string()
            .starts_with("change 2 failed, nothing was changed"),
        "{error}"
    );
    assert_eq!(
        error.to_error().data().details.get("change"),
        Some(&json!(2))
    );
    assert!(!notebook.is_modified());
    assert_eq!(notebook.render(), render(&sample(), INDENT));
}

#[test]
fn ops_rejects_an_empty_list() {
    let mut notebook = open(&sample());
    assert_eq!(notebook.apply(&[]).unwrap_err().code(), ErrorCode::Arg);
}

#[test]
fn export_writes_script_markdown_and_text() {
    let notebook = open(&sample());
    assert_eq!(
        notebook.export(ExportFormat::Script),
        "# %% [markdown]\n# # Analysis\n# Data from \"sensors\" — ąę.\n\n# %%\nimport pandas as pd\ndf = pd.read_csv('a.csv')\nprint(len(df))\n\n# %%\ndf.plot()\n\n# %% [markdown]\n# Notes\tand\u{1b} control characters\n"
    );
    assert_eq!(
        notebook.export(ExportFormat::Markdown),
        "# Analysis\nData from \"sensors\" — ąę.\n\n```python\nimport pandas as pd\ndf = pd.read_csv('a.csv')\nprint(len(df))\n```\n\n```text\n42\n```\n\n```python\ndf.plot()\n```\n\n```text\n<Figure size 640x480 with 1 Axes>\n```\n\nNotes\tand\u{1b} control characters\n"
    );
    assert_eq!(
        notebook.export(ExportFormat::Text),
        "# Analysis\nData from \"sensors\" — ąę.\n\nimport pandas as pd\ndf = pd.read_csv('a.csv')\nprint(len(df))\n\ndf.plot()\n\nNotes\tand\u{1b} control characters\n"
    );
}

#[test]
fn export_uses_the_comment_marker_of_the_kernel_language() {
    let mut value = notebook(
        5,
        vec![
            markdown(Some("a"), &["Title"]),
            code(Some("b"), &["let x = 1;"], json!([])),
        ],
    );
    value["metadata"] = json!({"kernelspec": {"language": "rust", "name": "rust"}});
    assert_eq!(
        open(&value).export(ExportFormat::Script),
        "// %% [markdown]\n// Title\n\n// %%\nlet x = 1;\n"
    );
}

#[test]
fn malformed_and_old_notebooks_are_format_errors() {
    let cases: [(&[u8], &str); 5] = [
        (b"{\"cells\": [", "not valid JSON at line 1"),
        (
            b"{\"nbformat\": 3, \"nbformat_minor\": 0, \"worksheets\": []}",
            "notebook format 3 is not supported",
        ),
        (b"\xff\xfe{}", "not valid UTF-8 at byte 0"),
        (b"{\"cells\": {}, \"nbformat\": 4}", "`cells` is not a list"),
        (
            b"{\"cells\": [1], \"nbformat\": 4}",
            "cell 0 is not an object",
        ),
    ];
    for (bytes, message) in cases {
        let error = Notebook::parse(bytes).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Format, "{error}");
        assert!(error.to_string().contains(message), "{error}");
    }
}

#[test]
fn duplicate_ids_are_reported_when_addressed() {
    let mut value = sample();
    value["cells"][2]["id"] = json!("load");
    let mut notebook = open(&value);
    let error = notebook
        .move_cell(&CellRef::Id("load".into()), 0)
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::Format);
    assert!(
        error.to_string().contains("run notebook.normalize"),
        "{error}"
    );
}

#[test]
fn content_lists_every_cell_with_an_outputs_summary() {
    let content = cells_as_content(render(&sample(), INDENT).as_bytes()).unwrap();
    assert_eq!((content.nbformat, content.nbformat_minor), (4, 5));
    assert_eq!(content.language.as_deref(), Some("python"));
    assert_eq!(content.cells.len(), 4);
    assert_eq!(
        content.cells[1].source,
        "import pandas as pd\ndf = pd.read_csv('a.csv')\nprint(len(df))"
    );
    assert_eq!(
        content.cells[1].outputs_summary.as_deref(),
        Some("1 output: stream stdout")
    );
    assert_eq!(content.cells[1].execution_count, Some(1));
    assert_eq!(content.cells[0].outputs_summary, None);
}
