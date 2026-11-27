// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-notebook::export
// Responsibility:   Export of notebooks to scripts, Markdown and plain text.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Export of notebooks to scripts, Markdown and plain text.

use nurt_contracts::record::CellType;

use crate::cell::output_text;
use crate::notebook::Notebook;
use crate::source;

/// Format of [`Notebook::export`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExportFormat {
    /// Script in the kernel language in the percent format: each cell
    /// starts with a `%%` marker comment; Markdown and raw cells become
    /// comments.
    Script,
    /// Markdown: Markdown cells as written, code cells and their text
    /// outputs as fenced blocks, raw cells as written.
    Markdown,
    /// Plain text: the sources of all cells.
    Text,
}

/// Shortest fence of a fenced code block in Markdown.
const MIN_FENCE: usize = 3;

/// Info string of fenced blocks with cell outputs.
const OUTPUT_INFO: &str = "text";

/// Line comment marker of a programming language; `#` for languages not
/// listed.
fn comment_marker(language: &str) -> &'static str {
    match language {
        "javascript" | "typescript" | "java" | "c" | "c++" | "cpp" | "c#" | "csharp" | "rust"
        | "go" | "scala" | "kotlin" | "swift" | "groovy" | "dart" | "f#" | "fsharp" => "//",
        "sql" | "haskell" | "lua" => "--",
        "matlab" | "octave" => "%",
        "clojure" | "scheme" | "racket" | "common-lisp" | "lisp" => ";;",
        _ => "#",
    }
}

impl Notebook {
    /// Text of the notebook in the given format (`notebook.export`).
    ///
    /// The language of a script and of fenced code blocks comes from
    /// [`Notebook::language`]. The result ends with exactly one line break,
    /// or is empty for a notebook without cells.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_notebook::{ExportFormat, Notebook};
    ///
    /// let text = r##"{"cells": [
    ///   {"cell_type": "markdown", "metadata": {}, "source": ["# Title\n", "Notes"]},
    ///   {"cell_type": "code", "execution_count": 1, "metadata": {}, "outputs": [], "source": ["x = 1"]}
    /// ], "metadata": {"language_info": {"name": "python"}}, "nbformat": 4, "nbformat_minor": 5}"##;
    /// let notebook = Notebook::parse(text.as_bytes())?;
    /// assert_eq!(
    ///     notebook.export(ExportFormat::Script),
    ///     "# %% [markdown]\n# # Title\n# Notes\n\n# %%\nx = 1\n"
    /// );
    /// # Ok::<(), nurt_notebook::NotebookError>(())
    /// ```
    #[must_use]
    pub fn export(&self, format: ExportFormat) -> String {
        let language = self.language().unwrap_or_default();
        let blocks: Vec<String> = (0..self.len())
            .filter_map(|index| self.stored(index))
            .map(|cell| {
                let source = cell.source();
                match format {
                    ExportFormat::Script => {
                        script_cell(comment_marker(&language), cell.cell_type, &source)
                    }
                    ExportFormat::Markdown => match cell.cell_type {
                        CellType::Code => {
                            let mut block = fenced(&language, &source);
                            for text in cell.outputs().iter().filter_map(output_text) {
                                block.push('\n');
                                block.push_str(&fenced(OUTPUT_INFO, &text));
                            }
                            block
                        }
                        _ => line_terminated(&source),
                    },
                    ExportFormat::Text => line_terminated(&source),
                }
            })
            .collect();
        blocks.join("\n")
    }
}

fn line_terminated(text: &str) -> String {
    let mut out = text.trim_end_matches(['\n', '\r']).to_owned();
    out.push('\n');
    out
}

fn script_cell(marker: &str, cell_type: CellType, source: &str) -> String {
    let source = source.trim_end_matches(['\n', '\r']);
    match cell_type {
        CellType::Code if source.is_empty() => format!("{marker} %%\n"),
        CellType::Code => format!("{marker} %%\n{source}\n"),
        _ => {
            let mut out = format!("{marker} %% [{}]\n", cell_type.as_str());
            for line in source::lines(source) {
                if line.is_empty() {
                    out.push_str(marker);
                } else {
                    out.push_str(marker);
                    out.push(' ');
                    out.push_str(line);
                }
                out.push('\n');
            }
            out
        }
    }
}

/// Fenced code block with a fence longer than any backtick run in `text`.
///
/// The info string comes from the notebook metadata, so white space, control
/// characters and backticks are removed from it: it stays on the line of the
/// fence and cannot close the block.
fn fenced(info: &str, text: &str) -> String {
    let info: String = info
        .chars()
        .filter(|c| !c.is_whitespace() && !c.is_control() && *c != '`')
        .collect();
    let longest_run = text
        .split(|c| c != '`')
        .map(str::len)
        .max()
        .unwrap_or_default();
    let fence = "`".repeat((longest_run + 1).max(MIN_FENCE));
    let body = text.trim_end_matches(['\n', '\r']);
    if body.is_empty() {
        format!("{fence}{info}\n{fence}\n")
    } else {
        format!("{fence}{info}\n{body}\n{fence}\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOTEBOOK: &str = r##"{"cells": [
      {"cell_type": "markdown", "metadata": {}, "source": "# Title\n\nText"},
      {"cell_type": "code", "execution_count": 2, "metadata": {}, "source": ["print('a')\n"],
       "outputs": [{"output_type": "stream", "name": "stdout", "text": ["a\n"]},
                   {"output_type": "display_data", "data": {"image/png": "iVBORw0KGgo="}, "metadata": {}}]},
      {"cell_type": "raw", "metadata": {}, "source": "raw text"}
    ], "metadata": {"kernelspec": {"language": "python", "name": "python3"}}, "nbformat": 4, "nbformat_minor": 5}"##;

    fn notebook() -> Notebook {
        Notebook::parse(NOTEBOOK.as_bytes()).unwrap()
    }

    #[test]
    fn exports_percent_script() {
        assert_eq!(
            notebook().export(ExportFormat::Script),
            "# %% [markdown]\n# # Title\n#\n# Text\n\n# %%\nprint('a')\n\n# %% [raw]\n# raw text\n"
        );
    }

    #[test]
    fn exports_markdown_with_text_outputs() {
        assert_eq!(
            notebook().export(ExportFormat::Markdown),
            "# Title\n\nText\n\n```python\nprint('a')\n```\n\n```text\na\n```\n\nraw text\n"
        );
    }

    #[test]
    fn exports_plain_text() {
        assert_eq!(
            notebook().export(ExportFormat::Text),
            "# Title\n\nText\n\nprint('a')\n\nraw text\n"
        );
    }

    #[test]
    fn lengthens_fences_around_backticks() {
        assert_eq!(fenced("md", "````\nx"), "`````md\n````\nx\n`````\n");
    }

    #[test]
    fn uses_language_comment_markers() {
        assert_eq!(comment_marker("rust"), "//");
        assert_eq!(comment_marker("sql"), "--");
        assert_eq!(comment_marker("python"), "#");
        assert_eq!(
            script_cell("//", CellType::Markdown, "a\n\nb"),
            "// %% [markdown]\n// a\n//\n// b\n"
        );
    }

    #[test]
    fn script_comments_never_hold_a_line_break_of_the_reader() {
        let text = "{\"cells\": [{\"cell_type\": \"markdown\", \"metadata\": {}, \
                    \"source\": \"a\\rimport os\\u000bimport sys\\u2028x\"}], \
                    \"nbformat\": 4, \"nbformat_minor\": 5}";
        let exported = Notebook::parse(text.as_bytes())
            .unwrap()
            .export(ExportFormat::Script);
        assert_eq!(
            exported,
            "# %% [markdown]\n# a\n# import os\n# import sys\n# x\n"
        );
        assert!(!exported.contains('\r'));
    }

    #[test]
    fn fence_info_string_stays_on_one_line() {
        assert_eq!(
            fenced("py\n```\nINJECT `x` y", "1"),
            "```pyINJECTxy\n1\n```\n"
        );
        let text = "{\"cells\": [{\"cell_type\": \"code\", \"execution_count\": null, \
                    \"metadata\": {}, \"outputs\": [], \"source\": \"x\"}], \
                    \"metadata\": {\"language_info\": {\"name\": \"p\\ny\\u0060\"}}, \
                    \"nbformat\": 4, \"nbformat_minor\": 5}";
        let exported = Notebook::parse(text.as_bytes())
            .unwrap()
            .export(ExportFormat::Markdown);
        assert_eq!(exported, "```py\nx\n```\n");
    }

    #[test]
    fn exports_empty_notebook_as_empty_text() {
        let empty =
            Notebook::parse(br#"{"cells": [], "nbformat": 4, "nbformat_minor": 5}"#).unwrap();
        assert_eq!(empty.export(ExportFormat::Script), "");
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
