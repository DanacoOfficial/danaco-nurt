// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Cells as plain data, independent of the stored JSON.

use nurt_contracts::record::CellType;
use serde_json::{Map, Value, json};

use crate::json::JsonObject;
use crate::source;

/// Detail of the outputs of code cells in [`CellData`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum OutputsDetail {
    /// No outputs and no summary.
    #[default]
    None,
    /// Number and kinds of outputs in [`CellData::outputs_summary`].
    Summary,
    /// The summary and the text of each output; images and other binary
    /// data are left out.
    Text,
    /// The summary and the outputs as stored in the notebook.
    Full,
}

/// Content of a cell returned by a listing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum View {
    /// First non-blank line of the source, without metadata.
    #[default]
    Outline,
    /// Whole source and metadata.
    Full,
}

/// Cell of a notebook as plain data.
///
/// The fields match the record `cell`; the entity tag of the file is added
/// by the caller, which knows the file.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct CellData {
    /// Position of the cell, from 0.
    pub index: usize,
    /// Identifier of the cell (nbformat 4.5 and later).
    pub id: Option<String>,
    /// Type of the cell.
    pub cell_type: CellType,
    /// Source of the cell, or its first non-blank line in an outline.
    pub source: String,
    /// Outputs at [`OutputsDetail::Text`] or [`OutputsDetail::Full`].
    pub outputs: Option<Vec<Value>>,
    /// Number and kinds of outputs, for code cells unless the detail is
    /// [`OutputsDetail::None`].
    pub outputs_summary: Option<String>,
    /// Execution counter of a code cell.
    pub execution_count: Option<u64>,
    /// Metadata of the cell in the full view.
    pub metadata: Option<Map<String, Value>>,
}

/// Cell members read from the stored JSON object.
pub(crate) struct StoredCell<'a> {
    pub(crate) index: usize,
    pub(crate) cell_type: CellType,
    pub(crate) object: &'a JsonObject,
}

impl StoredCell<'_> {
    pub(crate) fn id(&self) -> Option<&str> {
        self.object.get("id").and_then(Value::as_str)
    }

    pub(crate) fn source(&self) -> String {
        self.object
            .get("source")
            .and_then(source::join)
            .unwrap_or_default()
    }

    pub(crate) fn outputs(&self) -> &[Value] {
        self.object
            .get("outputs")
            .and_then(Value::as_array)
            .map_or(&[], Vec::as_slice)
    }

    /// Plain data of the cell with the requested view and output detail.
    pub(crate) fn data(&self, view: View, detail: OutputsDetail) -> CellData {
        let source = self.source();
        let is_code = self.cell_type == CellType::Code;
        let outputs = match detail {
            OutputsDetail::Text if is_code => {
                Some(self.outputs().iter().map(text_output).collect())
            }
            OutputsDetail::Full if is_code => Some(self.outputs().to_vec()),
            _ => None,
        };
        let outputs_summary =
            (is_code && detail != OutputsDetail::None).then(|| summarize(self.outputs()));
        CellData {
            index: self.index,
            id: self.id().map(str::to_owned),
            cell_type: self.cell_type,
            source: match view {
                View::Full => source,
                View::Outline => outline(&source).to_owned(),
            },
            outputs,
            outputs_summary,
            execution_count: self.object.get("execution_count").and_then(Value::as_u64),
            metadata: match view {
                View::Full => Some(
                    self.object
                        .get("metadata")
                        .and_then(Value::as_object)
                        .cloned()
                        .unwrap_or_default(),
                ),
                View::Outline => None,
            },
        }
    }

    /// Text of all outputs, separated by line breaks.
    pub(crate) fn outputs_text(&self) -> String {
        self.outputs()
            .iter()
            .filter_map(output_text)
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// First non-blank line of `source`, without its line break.
fn outline(source: &str) -> &str {
    source
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or_default()
}

/// Kind of an output: its type with the stream name, the error name or the
/// MIME types of its data.
fn output_kind(output: &Value) -> String {
    let output_type = output
        .get("output_type")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let detail = match output_type {
        "stream" => output
            .get("name")
            .and_then(Value::as_str)
            .map(str::to_owned),
        "error" => output
            .get("ename")
            .and_then(Value::as_str)
            .map(str::to_owned),
        _ => output
            .get("data")
            .and_then(Value::as_object)
            .map(|data| data.keys().cloned().collect::<Vec<_>>().join(" ")),
    };
    match detail {
        Some(detail) if !detail.is_empty() => format!("{output_type} {detail}"),
        _ => output_type.to_owned(),
    }
}

/// Number and kinds of outputs, for example
/// `2 outputs: stream stdout, display_data image/png text/plain`.
pub(crate) fn summarize(outputs: &[Value]) -> String {
    match outputs.len() {
        0 => "no outputs".to_owned(),
        count => {
            let noun = if count == 1 { "output" } else { "outputs" };
            let kinds: Vec<String> = outputs.iter().map(output_kind).collect();
            format!("{count} {noun}: {}", kinds.join(", "))
        }
    }
}

/// Textual content of an output: stream text, the `text/plain` data or
/// another `text/*` type, or the error with its traceback.
pub(crate) fn output_text(output: &Value) -> Option<String> {
    match output.get("output_type").and_then(Value::as_str)? {
        "stream" => output.get("text").and_then(source::join),
        "error" => {
            let name = output
                .get("ename")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let value = output
                .get("evalue")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let mut text = format!("{name}: {value}");
            if let Some(lines) = output.get("traceback").and_then(Value::as_array) {
                for line in lines.iter().filter_map(Value::as_str) {
                    text.push('\n');
                    text.push_str(line);
                }
            }
            Some(text)
        }
        _ => {
            let data = output.get("data")?.as_object()?;
            data.get("text/plain")
                .or_else(|| {
                    data.iter()
                        .find(|(mime, _)| mime.starts_with("text/"))
                        .map(|(_, value)| value)
                })
                .and_then(source::join)
        }
    }
}

/// Output reduced to its type, stream name and text.
fn text_output(output: &Value) -> Value {
    let mut reduced = Map::new();
    for key in ["output_type", "name", "execution_count"] {
        if let Some(value) = output.get(key) {
            reduced.insert(key.to_owned(), value.clone());
        }
    }
    if let Some(text) = output_text(output) {
        reduced.insert("text".to_owned(), json!(text));
    }
    Value::Object(reduced)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code_cell() -> Map<String, Value> {
        json!({
            "cell_type": "code",
            "execution_count": 3,
            "id": "a1",
            "metadata": {"tags": ["x"]},
            "outputs": [
                {"output_type": "stream", "name": "stdout", "text": ["1\n", "2\n"]},
                {"output_type": "display_data", "data": {"image/png": "iVBORw0KGgo=", "text/plain": ["<Figure>"]}, "metadata": {}},
                {"output_type": "error", "ename": "ValueError", "evalue": "bad", "traceback": ["line 1"]}
            ],
            "source": ["\n", "  import os\n", "print(1)"]
        })
        .as_object()
        .cloned()
        .unwrap()
    }

    #[test]
    fn summarizes_outputs_by_kind() {
        let members = code_cell();
        let outputs = members["outputs"].as_array().unwrap();
        assert_eq!(
            summarize(outputs),
            "3 outputs: stream stdout, display_data image/png text/plain, error ValueError"
        );
        assert_eq!(summarize(&[]), "no outputs");
    }

    #[test]
    fn outline_shows_first_non_blank_line() {
        let members = code_cell();
        let cell = StoredCell {
            index: 0,
            cell_type: CellType::Code,
            object: &JsonObject::from_map(members),
        };
        let data = cell.data(View::Outline, OutputsDetail::None);
        assert_eq!(data.source, "  import os");
        assert_eq!(data.metadata, None);
        assert_eq!(data.outputs_summary, None);
        assert_eq!(data.execution_count, Some(3));
        assert_eq!(data.id.as_deref(), Some("a1"));
    }

    #[test]
    fn text_detail_drops_binary_data() {
        let members = code_cell();
        let cell = StoredCell {
            index: 0,
            cell_type: CellType::Code,
            object: &JsonObject::from_map(members),
        };
        let data = cell.data(View::Full, OutputsDetail::Text);
        let outputs = data.outputs.unwrap();
        assert_eq!(
            outputs[0],
            json!({"output_type": "stream", "name": "stdout", "text": "1\n2\n"})
        );
        assert_eq!(
            outputs[1],
            json!({"output_type": "display_data", "text": "<Figure>"})
        );
        assert_eq!(outputs[2]["text"], json!("ValueError: bad\nline 1"));
        assert_eq!(data.source, "\n  import os\nprint(1)");
        assert!(data.metadata.is_some());
    }

    #[test]
    fn markdown_cells_have_no_output_fields() {
        let members = json!({"cell_type": "markdown", "metadata": {}, "source": "# T"})
            .as_object()
            .cloned()
            .unwrap();
        let cell = StoredCell {
            index: 1,
            cell_type: CellType::Markdown,
            object: &JsonObject::from_map(members),
        };
        let data = cell.data(View::Full, OutputsDetail::Full);
        assert_eq!(data.outputs, None);
        assert_eq!(data.outputs_summary, None);
    }
}
