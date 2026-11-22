// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Byte-exact output of unchanged and changed notebooks.

// Fixtures are built with `unwrap`; invalid fixture data fails the test.
#![allow(
    clippy::unwrap_used,
    reason = "fixtures in tests fail the test on invalid data"
)]

mod support;

use nurt_contracts::record::CellType;
use nurt_notebook::{CellRef, IdGenerator, Notebook, Position, Replace};
use proptest::prelude::*;
use proptest::test_runner::{Config, FileFailurePersistence, RngSeed};
use serde_json::{Value, json};
use support::{code, markdown, notebook, render, sample};

/// Indentations of the corpus: Jupyter (1 space), 2 and 4 spaces, a tab and
/// a single line.
const INDENTS: &[&str] = &[" ", "  ", "    ", "\t", ""];

/// Seed of the property tests, fixed for reproducible runs.
const SEED: u64 = 0x6e62_666f_726d_6174;

/// Cases per property.
const CASES: u32 = 512;

fn corpus() -> Vec<(String, String)> {
    let mut texts = Vec::new();
    for indent in INDENTS {
        let text = render(&sample(), indent);
        texts.push((format!("indent {indent:?}"), text.clone()));
        texts.push((
            format!("indent {indent:?} without final line break"),
            text.trim_end().to_owned(),
        ));
        texts.push((
            format!("indent {indent:?} with CRLF"),
            text.replace('\n', "\r\n"),
        ));
    }
    let mut string_sources = sample();
    for cell in string_sources["cells"].as_array_mut().unwrap() {
        let joined: String = cell["source"]
            .as_array()
            .unwrap()
            .iter()
            .map(|l| l.as_str().unwrap())
            .collect();
        cell["source"] = json!(joined);
    }
    texts.push(("sources as strings".into(), render(&string_sources, " ")));
    texts.push((
        "byte order mark".into(),
        format!("\u{feff}{}", render(&sample(), " ")),
    ));
    texts.push((
        "python compact separators".into(),
        r#"{"cells": [{"cell_type": "code", "execution_count": null, "metadata": {}, "outputs": [], "source": ["x = 1.50e3"]}], "metadata": {}, "nbformat": 4, "nbformat_minor": 4}"#.into(),
    ));
    texts.push((
        "ascii escapes and unsorted keys".into(),
        "{\n \"nbformat\": 4,\n \"nbformat_minor\": 5,\n \"cells\": [\n  {\n   \"source\": \"caf\\u00e9\",\n   \"cell_type\": \"markdown\",\n   \"metadata\": {\"z\": 1, \"a\": 2.0},\n   \"id\": \"m\"\n  }\n ],\n \"metadata\": {}\n}\n".into(),
    ));
    texts
}

#[test]
fn unchanged_notebooks_render_byte_for_byte() {
    for (name, text) in corpus() {
        let notebook = Notebook::parse(text.as_bytes()).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(!notebook.is_modified(), "{name}");
        assert_eq!(notebook.render(), text, "{name}");
    }
}

#[test]
fn changed_cell_is_the_only_difference_in_every_layout() {
    for indent in INDENTS {
        let text = render(&sample(), indent);
        let mut notebook = Notebook::parse(text.as_bytes()).unwrap();
        notebook
            .replace(&Replace {
                target: Some(CellRef::Id("plot".into())),
                source: Some("df.plot(kind='bar')\nplt.show()".into()),
                ..Replace::default()
            })
            .unwrap();
        let mut expected = sample();
        expected["cells"][2]["source"] = json!(["df.plot(kind='bar')\n", "plt.show()"]);
        assert_eq!(
            notebook.render(),
            render(&expected, indent),
            "indent {indent:?}"
        );
    }
}

#[test]
fn crlf_notebooks_keep_crlf_in_new_text() {
    let text = render(&sample(), " ").replace('\n', "\r\n");
    let mut notebook = Notebook::parse(text.as_bytes()).unwrap();
    notebook.set_id_generator(IdGenerator::sequential());
    notebook
        .insert(&Position::End, CellType::Code, "x")
        .unwrap();
    let mut expected = sample();
    expected["cells"]
        .as_array_mut()
        .unwrap()
        .push(code(Some("00000001"), &["x"], json!([])));
    assert_eq!(
        notebook.render(),
        render(&expected, " ").replace('\n', "\r\n")
    );
}

#[test]
fn unsorted_cells_keep_their_key_order_after_a_change() {
    let (_, text) = corpus().pop().unwrap();
    let mut notebook = Notebook::parse(text.as_bytes()).unwrap();
    notebook
        .replace(&Replace {
            target: Some(CellRef::Index(0)),
            source: Some("thé".into()),
            ..Replace::default()
        })
        .unwrap();
    assert_eq!(notebook.render(), text.replace("caf\\u00e9", "th\\u00e9"));
}

#[test]
fn single_line_notebook_changes_only_the_changed_value() {
    let text = r#"{"cells": [{"cell_type": "code", "execution_count": null, "metadata": {}, "outputs": [], "source": ["x = 1.50e3"]}], "metadata": {}, "nbformat": 4, "nbformat_minor": 4}"#;
    let mut notebook = Notebook::parse(text.as_bytes()).unwrap();
    notebook
        .insert(&Position::End, CellType::Markdown, "# T")
        .unwrap();
    assert_eq!(
        notebook.render(),
        text.replace(
            "\"x = 1.50e3\"]}]",
            "\"x = 1.50e3\"]}, {\"cell_type\": \"markdown\", \"metadata\": {}, \"source\": [\"# T\"]}]"
        )
    );
}

fn source_text() -> impl Strategy<Value = String> {
    proptest::string::string_regex("[a-zA-Z0-9 _=()\"'\\\\\n\r\t\u{1}\u{1b}ąęż€😀\u{2028}]{0,40}")
        .unwrap()
}

fn cell_strategy() -> impl Strategy<Value = Value> {
    (any::<bool>(), source_text(), any::<bool>(), any::<bool>()).prop_map(
        |(is_code, text, as_list, has_output)| {
            let lines: Vec<String> = split_like_python(&text);
            let line_refs: Vec<&str> = lines.iter().map(String::as_str).collect();
            let mut cell = if is_code {
                let outputs = if has_output {
                    json!([support::stream(&text), support::image()])
                } else {
                    json!([])
                };
                code(None, &line_refs, outputs)
            } else {
                markdown(None, &line_refs)
            };
            if !as_list {
                cell["source"] = json!(text);
            }
            cell
        },
    )
}

/// Line split of nbformat, which keeps the line breaks.
fn split_like_python(text: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        current.push(c);
        let ends = match c {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    current.push('\n');
                    chars.next();
                }
                true
            }
            '\n' | '\u{2028}' => true,
            _ => false,
        };
        if ends {
            lines.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

fn config() -> Config {
    Config {
        cases: CASES,
        rng_seed: RngSeed::Fixed(SEED),
        failure_persistence: Some(Box::new(FileFailurePersistence::Off)),
        ..Config::default()
    }
}

proptest! {
    #![proptest_config(config())]

    #[test]
    fn generated_notebooks_round_trip(
        cells in proptest::collection::vec(cell_strategy(), 0..6),
        indent in proptest::sample::select(INDENTS),
    ) {
        let text = render(&notebook(4, cells), indent);
        let parsed = Notebook::parse(text.as_bytes()).unwrap();
        prop_assert_eq!(parsed.render(), text);
    }

    #[test]
    fn replaced_source_matches_reference_serialization(
        cells in proptest::collection::vec(cell_strategy(), 1..6),
        indent in proptest::sample::select(INDENTS),
        target in any::<prop::sample::Index>(),
        new_text in source_text(),
    ) {
        let value = notebook(4, cells);
        let mut parsed = Notebook::parse(render(&value, indent).as_bytes()).unwrap();
        let index = target.index(parsed.len());
        parsed.replace(&Replace {
            target: Some(CellRef::Index(index as i64)),
            source: Some(new_text.clone()),
            ..Replace::default()
        }).unwrap();
        let mut expected = value;
        let cell = &mut expected["cells"][index];
        cell["source"] = if cell["source"].is_array() { json!(split_like_python(&new_text)) } else { json!(new_text) };
        prop_assert_eq!(parsed.render(), render(&expected, indent));
    }

    #[test]
    fn normalize_is_idempotent(
        cells in proptest::collection::vec(cell_strategy(), 0..6),
        indent in proptest::sample::select(INDENTS),
    ) {
        let mut first = Notebook::parse(render(&notebook(4, cells), indent).as_bytes()).unwrap();
        first.set_id_generator(IdGenerator::sequential());
        first.normalize();
        let once = first.render();
        let mut second = Notebook::parse(once.as_bytes()).unwrap();
        let outcome = second.normalize();
        prop_assert!(outcome.cells.is_empty() && !outcome.notebook);
        prop_assert_eq!(second.render(), once);
    }
}
