// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Rename plans built from workspace edits.
//!
//! A server answers `textDocument/rename` (and code actions with edits) with a
//! `WorkspaceEdit`: text edits keyed by document URI, optionally mixed with
//! file creations, renames and deletions. [`workspace_change`] converts it to
//! the [`WorkspaceChange`] of the code group: absolute paths, one-based lines
//! and columns counted in Unicode scalar values, changes in the order the
//! server requires them to be applied.
//!
//! Protocol positions count characters in the negotiated position encoding
//! (UTF-16 code units unless the server announced another encoding), so the
//! conversion needs the text of every edited document as it was before the
//! change. Text edits of a document that a preceding operation renamed or
//! created refer to the content after that operation; the conversion follows
//! the operations to find that content.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use lsp_types::{
    AnnotatedTextEdit, DocumentChangeOperation, DocumentChanges, OneOf, Position,
    PositionEncodingKind, ResourceOp, TextDocumentEdit, TextEdit, Uri, WorkspaceEdit,
};
use nurt_code::semantic::{
    DocumentChange, Location, ResourceChange, SemanticError, SemanticErrorKind, SemanticResult,
    TextChange, WorkspaceChange,
};
use nurt_contracts::services::TextPosition;
use serde_json::Value;

/// Unit in which a protocol position counts characters within a line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PositionUnit {
    /// UTF-8 code units (bytes).
    Utf8,
    /// UTF-16 code units, the protocol default.
    #[default]
    Utf16,
    /// Unicode scalar values.
    Utf32,
}

impl PositionUnit {
    /// Unit of a negotiated `positionEncoding`; unknown kinds fall back to
    /// the protocol default, UTF-16.
    ///
    /// ```
    /// use lsp_types::PositionEncodingKind;
    /// use nurt_lspclient::rename::PositionUnit;
    ///
    /// assert_eq!(PositionUnit::from_kind(&PositionEncodingKind::UTF8), PositionUnit::Utf8);
    /// assert_eq!(PositionUnit::from_kind(&"utf-7".to_owned().into()), PositionUnit::Utf16);
    /// ```
    #[must_use]
    pub fn from_kind(kind: &PositionEncodingKind) -> Self {
        if *kind == PositionEncodingKind::UTF8 {
            Self::Utf8
        } else if *kind == PositionEncodingKind::UTF32 {
            Self::Utf32
        } else {
            Self::Utf16
        }
    }

    fn width(self, character: char) -> u32 {
        match self {
            Self::Utf8 => code_units(character.len_utf8()),
            Self::Utf16 => code_units(character.len_utf16()),
            Self::Utf32 => 1,
        }
    }
}

/// A character is at most four UTF-8 code units, so the conversion cannot
/// overflow.
fn code_units(count: usize) -> u32 {
    u32::try_from(count).unwrap_or(u32::MAX)
}

/// Converts the JSON form of a `WorkspaceEdit` to a [`WorkspaceChange`].
///
/// `text_of` returns the current content of a file, or `None` when the file
/// cannot be read; positions in an unreadable file are taken as counted in
/// Unicode scalar values. `null` converts to an empty change. Edits listed
/// under `changes` are ordered by path and position, because the protocol
/// gives that map no order; `documentChanges` keep the order of the server.
///
/// # Errors
///
/// Fails with [`SemanticErrorKind::Failed`] when the value is not a
/// `WorkspaceEdit` or names a document that is not a local file.
///
/// # Examples
///
/// ```
/// use std::path::Path;
///
/// use nurt_code::semantic::DocumentChange;
/// use nurt_lspclient::rename::{PositionUnit, workspace_change};
/// use serde_json::json;
///
/// let edit = json!({"changes": {"file:///work/a.py": [{
///     "range": {"start": {"line": 0, "character": 4}, "end": {"line": 0, "character": 7}},
///     "newText": "total"
/// }]}});
/// let text = |_: &Path| Some("def old(): pass\n".to_owned());
/// let change = workspace_change(&edit, PositionUnit::Utf16, &text)?;
/// let DocumentChange::Text(first) = &change.changes[0] else { unreachable!() };
/// assert_eq!(first.range.start.column, 5);
/// assert_eq!(first.new_text, "total");
/// # Ok::<(), nurt_code::semantic::SemanticError>(())
/// ```
pub fn workspace_change(
    edit: &Value,
    unit: PositionUnit,
    text_of: &dyn Fn(&Path) -> Option<String>,
) -> SemanticResult<WorkspaceChange> {
    if edit.is_null() {
        return Ok(WorkspaceChange::default());
    }
    let edit: WorkspaceEdit = serde_json::from_value(edit.clone()).map_err(|error| {
        failed(format!(
            "the server sent an invalid workspace edit: {error}"
        ))
    })?;
    let mut converter = Converter::new(unit, text_of);
    if let Some(document_changes) = &edit.document_changes {
        match document_changes {
            DocumentChanges::Edits(edits) => {
                for document in edits {
                    converter.document_edit(document)?;
                }
            }
            DocumentChanges::Operations(operations) => {
                for operation in operations {
                    match operation {
                        DocumentChangeOperation::Edit(document) => {
                            converter.document_edit(document)?;
                        }
                        DocumentChangeOperation::Op(resource) => converter.resource(resource)?,
                    }
                }
            }
        }
    } else if let Some(changes) = &edit.changes {
        let mut files = changes
            .iter()
            .map(|(uri, edits)| Ok((file_path(uri)?, edits)))
            .collect::<SemanticResult<Vec<_>>>()?;
        files.sort_by(|left, right| left.0.cmp(&right.0));
        for (path, edits) in files {
            let mut sorted: Vec<&TextEdit> = edits.iter().collect();
            sorted.sort_by_key(|edit| (edit.range.start.line, edit.range.start.character));
            converter.text_edits(&path, sorted)?;
        }
    }
    Ok(WorkspaceChange {
        changes: converter.changes,
    })
}

/// Content of a document as seen by the operations converted so far.
#[derive(Clone)]
enum Content {
    /// The file on disk at this path.
    Disk(PathBuf),
    /// A file created empty by an earlier operation.
    Empty,
}

struct Converter<'a> {
    unit: PositionUnit,
    text_of: &'a dyn Fn(&Path) -> Option<String>,
    /// Current path → where its content comes from, for paths touched by
    /// resource operations.
    moved: HashMap<PathBuf, Content>,
    cache: HashMap<PathBuf, Option<String>>,
    changes: Vec<DocumentChange>,
}

impl<'a> Converter<'a> {
    fn new(unit: PositionUnit, text_of: &'a dyn Fn(&Path) -> Option<String>) -> Self {
        Self {
            unit,
            text_of,
            moved: HashMap::new(),
            cache: HashMap::new(),
            changes: Vec::new(),
        }
    }

    fn document_edit(&mut self, document: &TextDocumentEdit) -> SemanticResult<()> {
        let path = file_path(&document.text_document.uri)?;
        let edits = document.edits.iter().map(|edit| match edit {
            OneOf::Left(edit) => edit,
            OneOf::Right(AnnotatedTextEdit { text_edit, .. }) => text_edit,
        });
        self.text_edits(&path, edits)
    }

    fn text_edits<'e>(
        &mut self,
        path: &Path,
        edits: impl IntoIterator<Item = &'e TextEdit>,
    ) -> SemanticResult<()> {
        let text = self.text(path);
        for edit in edits {
            let start = position(text.as_deref(), edit.range.start, self.unit);
            let end = position(text.as_deref(), edit.range.end, self.unit);
            self.changes.push(DocumentChange::Text(TextChange {
                range: Location {
                    path: path.to_path_buf(),
                    start,
                    end,
                },
                new_text: edit.new_text.clone(),
            }));
        }
        Ok(())
    }

    fn resource(&mut self, operation: &ResourceOp) -> SemanticResult<()> {
        let change = match operation {
            ResourceOp::Create(create) => {
                let path = file_path(&create.uri)?;
                let options = create.options.as_ref();
                self.moved.insert(path.clone(), Content::Empty);
                ResourceChange::Create {
                    path,
                    overwrite: options.and_then(|options| options.overwrite) == Some(true),
                    ignore_if_exists: options.and_then(|options| options.ignore_if_exists)
                        == Some(true),
                }
            }
            ResourceOp::Rename(rename) => {
                let from = file_path(&rename.old_uri)?;
                let to = file_path(&rename.new_uri)?;
                let options = rename.options.as_ref();
                let content = self
                    .moved
                    .remove(&from)
                    .unwrap_or_else(|| Content::Disk(from.clone()));
                self.moved.insert(to.clone(), content);
                ResourceChange::Rename {
                    from,
                    to,
                    overwrite: options.and_then(|options| options.overwrite) == Some(true),
                    ignore_if_exists: options.and_then(|options| options.ignore_if_exists)
                        == Some(true),
                }
            }
            ResourceOp::Delete(delete) => {
                let path = file_path(&delete.uri)?;
                let options = delete.options.as_ref();
                self.moved.remove(&path);
                ResourceChange::Delete {
                    path,
                    recursive: options.and_then(|options| options.recursive) == Some(true),
                    ignore_if_not_exists: options.and_then(|options| options.ignore_if_not_exists)
                        == Some(true),
                }
            }
        };
        self.changes.push(DocumentChange::Resource(change));
        Ok(())
    }

    fn text(&mut self, path: &Path) -> Option<String> {
        let source = match self.moved.get(path) {
            Some(Content::Empty) => return Some(String::new()),
            Some(Content::Disk(source)) => source.clone(),
            None => path.to_path_buf(),
        };
        let text_of = self.text_of;
        self.cache
            .entry(source)
            .or_insert_with_key(|source| text_of(source))
            .clone()
    }
}

/// Converts a protocol position to a one-based position counted in Unicode
/// scalar values. Following the protocol, a character offset past the end
/// of the line denotes the end of the line, and a line past the end of the
/// text denotes the end of the text.
fn position(text: Option<&str>, at: Position, unit: PositionUnit) -> TextPosition {
    let Some(text) = text else {
        return TextPosition {
            line: at.line.saturating_add(1),
            column: at.character.saturating_add(1),
        };
    };
    let lines = split_lines(text);
    let index = usize::try_from(at.line).unwrap_or(usize::MAX);
    let Some(line) = lines.get(index) else {
        let last = lines.last().copied().unwrap_or_default();
        return TextPosition {
            line: code_units(lines.len().max(1)),
            column: code_units(line_body(last).chars().count()).saturating_add(1),
        };
    };
    let mut units = 0;
    let mut column = 1;
    for character in line_body(line).chars() {
        if units >= at.character {
            break;
        }
        units += unit.width(character);
        column += 1;
    }
    TextPosition {
        line: at.line.saturating_add(1),
        column,
    }
}

/// Lines of a text: split at line feeds; a text with carriage returns and no
/// line feed is split at the carriage returns, as Nurt numbers its lines.
pub(crate) fn split_lines(text: &str) -> Vec<&str> {
    if !text.contains('\n') && text.contains('\r') {
        text.split('\r').collect()
    } else {
        text.split('\n').collect()
    }
}

/// Line without its carriage return.
fn line_body(line: &str) -> &str {
    line.strip_suffix('\r').unwrap_or(line)
}

/// Local path of a `file:` URI.
fn file_path(uri: &Uri) -> SemanticResult<PathBuf> {
    url::Url::parse(uri.as_str())
        .ok()
        .filter(|url| url.scheme() == "file")
        .and_then(|url| url.to_file_path().ok())
        .ok_or_else(|| {
            failed(format!(
                "the server sent an edit for `{}`, which is not a local file",
                uri.as_str()
            ))
        })
}

fn failed(message: String) -> SemanticError {
    SemanticError::new(SemanticErrorKind::Failed, message)
}
