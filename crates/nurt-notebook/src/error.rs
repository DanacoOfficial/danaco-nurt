// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Errors of notebook parsing and editing.

use std::fmt;

use nurt_contracts::error::{Error, ErrorCode};
use serde_json::json;

/// Error of reading or changing a notebook.
///
/// Every variant maps to one error code of the language through
/// [`NotebookError::code`]:
///
/// | Variant | Code |
/// |---|---|
/// | [`InvalidJson`](Self::InvalidJson), [`NotUtf8`](Self::NotUtf8), [`Structure`](Self::Structure), [`Version`](Self::Version), [`DuplicateId`](Self::DuplicateId) | `E_FORMAT` |
/// | [`IndexOutOfRange`](Self::IndexOutOfRange), [`UnknownId`](Self::UnknownId), [`AddressMismatch`](Self::AddressMismatch), [`InvalidChange`](Self::InvalidChange) | `E_ARG` |
/// | [`NoMatch`](Self::NoMatch) | `E_NO_MATCH` |
/// | [`Change`](Self::Change) | code of the wrapped error |
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum NotebookError {
    /// The file is not valid JSON.
    InvalidJson {
        /// Line of the error, from 1.
        line: usize,
        /// Column of the error, from 1.
        column: usize,
        /// Description of the syntax error.
        detail: String,
    },
    /// The file is not valid UTF-8.
    NotUtf8 {
        /// Byte offset of the first invalid sequence.
        offset: usize,
    },
    /// The JSON document does not have the structure of a notebook.
    Structure {
        /// What is missing or has the wrong type.
        detail: String,
    },
    /// The notebook uses a format version other than nbformat 4.
    Version {
        /// Major version found in the file.
        nbformat: u64,
    },
    /// Two cells share an identifier.
    DuplicateId {
        /// The repeated identifier.
        id: String,
    },
    /// A cell index lies outside the notebook.
    IndexOutOfRange {
        /// The requested index.
        index: i64,
        /// Number of cells, or number of valid positions for an insertion.
        len: usize,
    },
    /// No cell has the requested identifier.
    UnknownId {
        /// The requested identifier.
        id: String,
        /// Existing identifiers closest to the requested one.
        candidates: Vec<String>,
        /// Whether the notebook has any cell identifiers at all.
        notebook_has_ids: bool,
    },
    /// An index and an identifier name different cells.
    AddressMismatch {
        /// The requested index.
        index: usize,
        /// The requested identifier.
        id: String,
        /// Index of the cell with the requested identifier.
        id_index: usize,
        /// Identifier of the cell at the requested index.
        index_id: Option<String>,
    },
    /// The text or pattern to replace occurs in no selected cell.
    NoMatch {
        /// The text or pattern as written.
        pattern: String,
    },
    /// The arguments of an operation are inconsistent or incomplete.
    InvalidChange {
        /// What is wrong and how to correct it.
        detail: String,
    },
    /// An operation of a list of changes failed; nothing was changed.
    Change {
        /// Position of the failed operation in the list, from 0.
        position: usize,
        /// The error of the operation.
        source: Box<NotebookError>,
    },
}

impl NotebookError {
    /// Error code of the language for this error.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_contracts::error::ErrorCode;
    /// use nurt_notebook::NotebookError;
    ///
    /// let error = NotebookError::Version { nbformat: 3 };
    /// assert_eq!(error.code(), ErrorCode::Format);
    /// ```
    #[must_use]
    pub fn code(&self) -> ErrorCode {
        match self {
            Self::InvalidJson { .. }
            | Self::NotUtf8 { .. }
            | Self::Structure { .. }
            | Self::Version { .. }
            | Self::DuplicateId { .. } => ErrorCode::Format,
            Self::IndexOutOfRange { .. }
            | Self::UnknownId { .. }
            | Self::AddressMismatch { .. }
            | Self::InvalidChange { .. } => ErrorCode::Arg,
            Self::NoMatch { .. } => ErrorCode::NoMatch,
            Self::Change { source, .. } => source.code(),
        }
    }

    /// Error of the language with the code, the message and, where they
    /// apply, the fields `expected`, `actual`, `found` and `details`.
    ///
    /// The caller adds the file location and `fix` programs, which need the
    /// path of the notebook.
    #[must_use]
    pub fn to_error(&self) -> Error {
        let error = Error::new(self.code(), self.to_string());
        match self {
            Self::InvalidJson { line, column, .. } => error
                .with_detail("format", json!("ipynb"))
                .with_detail("line", json!(line))
                .with_detail("column", json!(column)),
            Self::NotUtf8 { offset } => error
                .with_detail("format", json!("ipynb"))
                .with_detail("offset", json!(offset)),
            Self::Structure { detail } => error
                .with_detail("format", json!("ipynb"))
                .with_detail("detail", json!(detail)),
            Self::Version { nbformat } => error
                .with_detail("format", json!("ipynb"))
                .with_expected(json!(SUPPORTED_MAJOR))
                .with_actual(json!(nbformat)),
            Self::DuplicateId { id } => error
                .with_detail("format", json!("ipynb"))
                .with_found(id.clone()),
            Self::IndexOutOfRange { index, len } => error
                .with_expected(json!(valid_range(*len)))
                .with_actual(json!(index)),
            Self::UnknownId { id, candidates, .. } => error
                .with_found(id.clone())
                .with_detail("candidates", json!(candidates)),
            Self::AddressMismatch {
                index,
                id,
                id_index,
                index_id,
            } => error
                .with_expected(json!({ "index": id_index, "id": id }))
                .with_actual(json!({ "index": index, "id": index_id })),
            Self::NoMatch { pattern } => error.with_found(pattern.clone()),
            Self::InvalidChange { .. } => error,
            Self::Change { position, source } => {
                source.to_error().with_detail("change", json!(position))
            }
        }
    }

    pub(crate) fn structure(detail: impl Into<String>) -> Self {
        Self::Structure {
            detail: detail.into(),
        }
    }

    pub(crate) fn invalid(detail: impl Into<String>) -> Self {
        Self::InvalidChange {
            detail: detail.into(),
        }
    }
}

/// The only supported major version of the notebook format.
pub(crate) const SUPPORTED_MAJOR: u64 = 4;

fn valid_range(len: usize) -> String {
    match len {
        0 => "no cells".to_owned(),
        1 => "0".to_owned(),
        _ => format!("0..{}", len - 1),
    }
}

impl fmt::Display for NotebookError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidJson {
                line,
                column,
                detail,
            } => write!(
                f,
                "notebook is not valid JSON at line {line} column {column}: {detail}; repair the file or restore it from the journal"
            ),
            Self::NotUtf8 { offset } => write!(
                f,
                "notebook is not valid UTF-8 at byte {offset}; save the file as UTF-8"
            ),
            Self::Structure { detail } => write!(f, "file is not an nbformat 4 notebook: {detail}"),
            Self::Version { nbformat } => write!(
                f,
                "notebook format {nbformat} is not supported; convert the notebook to nbformat 4, for example with `jupyter nbconvert --to notebook`"
            ),
            Self::DuplicateId { id } => write!(
                f,
                "cell id `{id}` occurs more than once; run notebook.normalize to assign unique ids"
            ),
            Self::IndexOutOfRange { index, len } => write!(
                f,
                "cell index {index} is out of range; valid indexes: {}",
                valid_range(*len)
            ),
            Self::UnknownId {
                id,
                candidates,
                notebook_has_ids,
            } => {
                if !notebook_has_ids {
                    write!(
                        f,
                        "no cell has id `{id}`; the notebook has no cell ids, address cells by index or run notebook.normalize"
                    )
                } else if candidates.is_empty() {
                    write!(f, "no cell has id `{id}`; list the ids with notebook.cells")
                } else {
                    write!(
                        f,
                        "no cell has id `{id}`; closest ids: {}",
                        candidates.join(", ")
                    )
                }
            }
            Self::AddressMismatch {
                index,
                id,
                id_index,
                ..
            } => write!(
                f,
                "index {index} and id `{id}` name different cells; cell `{id}` is at index {id_index}, give only one of them"
            ),
            Self::NoMatch { pattern } => write!(
                f,
                "`{pattern}` occurs in no selected cell; check the text or the cell selection"
            ),
            Self::InvalidChange { detail } => f.write_str(detail),
            Self::Change { position, source } => {
                write!(f, "change {position} failed, nothing was changed: {source}")
            }
        }
    }
}

impl std::error::Error for NotebookError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Change { source, .. } => Some(source.as_ref()),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_variants_to_language_codes() {
        assert_eq!(
            NotebookError::structure("missing `cells`").code(),
            ErrorCode::Format
        );
        assert_eq!(
            NotebookError::IndexOutOfRange { index: 4, len: 2 }.code(),
            ErrorCode::Arg
        );
        assert_eq!(
            NotebookError::NoMatch {
                pattern: "x".into()
            }
            .code(),
            ErrorCode::NoMatch
        );
        let wrapped = NotebookError::Change {
            position: 2,
            source: Box::new(NotebookError::NoMatch {
                pattern: "x".into(),
            }),
        };
        assert_eq!(wrapped.code(), ErrorCode::NoMatch);
    }

    #[test]
    fn messages_start_lowercase_without_final_period() {
        let errors = [
            NotebookError::InvalidJson {
                line: 1,
                column: 2,
                detail: "expected value".into(),
            },
            NotebookError::NotUtf8 { offset: 3 },
            NotebookError::structure("missing `cells`"),
            NotebookError::Version { nbformat: 3 },
            NotebookError::DuplicateId { id: "a".into() },
            NotebookError::IndexOutOfRange { index: 9, len: 0 },
            NotebookError::UnknownId {
                id: "a".into(),
                candidates: vec!["b".into()],
                notebook_has_ids: true,
            },
            NotebookError::AddressMismatch {
                index: 1,
                id: "a".into(),
                id_index: 2,
                index_id: None,
            },
            NotebookError::NoMatch {
                pattern: "x".into(),
            },
        ];
        for error in errors {
            let text = error.to_string();
            assert!(text.starts_with(|c: char| !c.is_uppercase()), "{text}");
            assert!(!text.ends_with('.'), "{text}");
        }
    }

    #[test]
    fn version_error_reports_expected_and_actual() {
        let error = NotebookError::Version { nbformat: 3 }.to_error();
        assert_eq!(error.data().expected, Some(json!(4)));
        assert_eq!(error.data().actual, Some(json!(3)));
    }

    #[test]
    fn out_of_range_error_names_valid_indexes() {
        let text = NotebookError::IndexOutOfRange { index: 5, len: 3 }.to_string();
        assert!(text.contains("0..2"), "{text}");
    }
}
