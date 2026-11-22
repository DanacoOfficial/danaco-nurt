// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Notebooks generated for tests and their reference serialization.
//!
//! The reference serialization is the output of Jupyter: `json.dumps` with
//! sorted cell keys, `ensure_ascii=False` and a final line break. The JSON
//! writer of `serde_json` with the same indentation produces identical text,
//! so a changed notebook can be compared with the reference text of the
//! expected value.

// Each test binary compiles this module and uses a different subset of it.
#![allow(
    dead_code,
    reason = "each test binary uses a different subset of the helpers"
)]

use serde::Serialize;
use serde_json::ser::{PrettyFormatter, Serializer};
use serde_json::{Value, json};

/// Base64 text of a small PNG image, as Jupyter stores `image/png` data.
pub(crate) const PNG_BASE64: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==\n";

/// Text of `value` indented with `indent`, or on one line when `indent` is
/// empty, followed by a line break.
pub(crate) fn render(value: &Value, indent: &str) -> String {
    let mut out = Vec::new();
    if indent.is_empty() {
        value.serialize(&mut Serializer::new(&mut out)).unwrap();
    } else {
        let formatter = PrettyFormatter::with_indent(indent.as_bytes());
        value
            .serialize(&mut Serializer::with_formatter(&mut out, formatter))
            .unwrap();
    }
    let mut text = String::from_utf8(out).unwrap();
    text.push('\n');
    text
}

/// Notebook of nbformat 4 with the given minor version and cells.
pub(crate) fn notebook(minor: u64, cells: Vec<Value>) -> Value {
    json!({
        "cells": cells,
        "metadata": {
            "kernelspec": {"display_name": "Python 3", "language": "python", "name": "python3"},
            "language_info": {"name": "python", "version": "3.12.1"}
        },
        "nbformat": 4,
        "nbformat_minor": minor
    })
}

/// Code cell with the source as a list of lines and the given outputs.
pub(crate) fn code(id: Option<&str>, lines: &[&str], outputs: Value) -> Value {
    let mut cell = json!({
        "cell_type": "code",
        "execution_count": if outputs.as_array().is_some_and(|o| !o.is_empty()) { json!(1) } else { Value::Null },
    });
    let map = cell.as_object_mut().unwrap();
    if let Some(id) = id {
        map.insert("id".into(), json!(id));
    }
    map.insert("metadata".into(), json!({}));
    map.insert("outputs".into(), outputs);
    map.insert("source".into(), json!(lines));
    cell
}

/// Markdown cell with the source as a list of lines.
pub(crate) fn markdown(id: Option<&str>, lines: &[&str]) -> Value {
    let mut cell = json!({"cell_type": "markdown"});
    let map = cell.as_object_mut().unwrap();
    if let Some(id) = id {
        map.insert("id".into(), json!(id));
    }
    map.insert("metadata".into(), json!({}));
    map.insert("source".into(), json!(lines));
    cell
}

/// Stream output on standard output.
pub(crate) fn stream(text: &str) -> Value {
    json!({"name": "stdout", "output_type": "stream", "text": [text]})
}

/// Display output with a PNG image and its text form.
pub(crate) fn image() -> Value {
    json!({
        "data": {"image/png": PNG_BASE64, "text/plain": ["<Figure size 640x480 with 1 Axes>"]},
        "metadata": {},
        "output_type": "display_data"
    })
}

/// Notebook of nbformat 4.5 with four cells: Markdown, code with a stream
/// output, code with an image, Markdown.
pub(crate) fn sample() -> Value {
    notebook(
        5,
        vec![
            markdown(
                Some("intro"),
                &["# Analysis\n", "Data from \"sensors\" — ąę."],
            ),
            code(
                Some("load"),
                &[
                    "import pandas as pd\n",
                    "df = pd.read_csv('a.csv')\n",
                    "print(len(df))",
                ],
                json!([stream("42\n")]),
            ),
            code(Some("plot"), &["df.plot()"], json!([image()])),
            markdown(Some("notes"), &["Notes\tand\u{1b} control characters"]),
        ],
    )
}
