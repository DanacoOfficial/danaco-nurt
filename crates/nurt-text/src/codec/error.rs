// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-text::codec::error
// Responsibility:   Error of [`decode`](super::decode) and [`encode`](super::encode).
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

use std::fmt;

use nurt_contracts::error::{Error, ErrorCode, Location};

use super::Codec;
use crate::eol::Position;

/// Error of [`decode`](super::decode) and [`encode`](super::encode).
///
/// [`CodecError::code`] gives the code of the contract error and
/// [`CodecError::into_error`] the full error:
///
/// | Variant | Code |
/// |---|---|
/// | [`UnknownEncoding`](Self::UnknownEncoding) | `E_UNKNOWN_VALUE` |
/// | [`BomNotSupported`](Self::BomNotSupported) | `E_INVALID_VALUE` |
/// | [`MissingBom`](Self::MissingBom), [`Malformed`](Self::Malformed), [`Unrepresentable`](Self::Unrepresentable) | `E_ENCODING` |
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum CodecError {
    /// The label denotes no supported encoding.
    UnknownEncoding {
        /// The label as given.
        label: String,
    },
    /// A byte order mark was requested for an encoding that has none.
    BomNotSupported {
        /// The encoding.
        codec: Codec,
    },
    /// A byte order mark was expected and the content does not start with
    /// it.
    MissingBom {
        /// The encoding.
        codec: Codec,
    },
    /// A byte sequence is not valid in the encoding.
    Malformed {
        /// The encoding.
        codec: Codec,
        /// Offset of the sequence from the start of the content, byte order
        /// mark included.
        offset: u64,
        /// The sequence.
        bytes: Vec<u8>,
        /// Position in the decoded text where the sequence starts.
        position: Position,
    },
    /// A character cannot be represented in the encoding.
    Unrepresentable {
        /// The encoding.
        codec: Codec,
        /// The character.
        character: char,
        /// Byte offset of the character in the text.
        offset: u64,
        /// Position of the character in the text.
        position: Position,
    },
}

impl CodecError {
    /// Code of the contract error.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_contracts::error::ErrorCode;
    /// use nurt_text::codec::Codec;
    ///
    /// let error = Codec::from_label("ebcdic").unwrap_err();
    /// assert_eq!(error.code(), ErrorCode::UnknownValue);
    /// ```
    #[must_use]
    pub fn code(&self) -> ErrorCode {
        match self {
            Self::UnknownEncoding { .. } => ErrorCode::UnknownValue,
            Self::BomNotSupported { .. } => ErrorCode::InvalidValue,
            Self::MissingBom { .. } | Self::Malformed { .. } | Self::Unrepresentable { .. } => {
                ErrorCode::Encoding
            }
        }
    }

    /// Converts the error into a contract error.
    ///
    /// With a `path`, errors of the content carry a [`Location`], and an
    /// unrepresentable character carries a `convert` call to UTF-8 as the
    /// fix.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_text::codec::{Codec, encode};
    ///
    /// let error = encode("€", &Codec::Ibm852.text_encoding(false)).unwrap_err();
    /// let error = error.into_error(Some("/srv/legacy/menu.txt"));
    /// assert_eq!(error.fix, ["convert \"/srv/legacy/menu.txt\" encoding: \"utf-8\""]);
    /// ```
    #[must_use]
    pub fn into_error(self, path: Option<&str>) -> Error {
        let error = Error::new(self.code(), self.to_string());
        match self {
            Self::UnknownEncoding { label } => error
                .with_expected(Codec::ALL.map(Codec::label).to_vec().into())
                .with_actual(label.into()),
            Self::BomNotSupported { .. } => error.with_problem("bom_not_supported"),
            Self::MissingBom { codec } => with_location(error, path, None, 0)
                .with_problem("missing_bom")
                .with_detail("encoding", codec.label().into()),
            Self::Malformed {
                codec,
                offset,
                bytes,
                position,
            } => with_location(error, path, Some(position), offset)
                .with_problem("malformed_sequence")
                .with_found(hex(&bytes))
                .with_detail("encoding", codec.label().into())
                .with_detail("line", position.line.into())
                .with_detail("column", position.column.into())
                .with_detail("byte", offset.into()),
            Self::Unrepresentable {
                codec,
                character,
                offset,
                position,
            } => {
                let error = with_location(error, path, Some(position), offset)
                    .with_problem("unrepresentable_character")
                    .with_found(character.to_string())
                    .with_detail("encoding", codec.label().into())
                    .with_detail("character", code_point(character).into())
                    .with_detail("line", position.line.into())
                    .with_detail("column", position.column.into())
                    .with_detail("byte", offset.into());
                match path {
                    Some(path) => error.with_fix(format!(
                        "convert \"{}\" encoding: \"{}\"",
                        escape(path),
                        Codec::Utf8.label()
                    )),
                    None => error,
                }
            }
        }
    }
}

fn with_location(error: Error, path: Option<&str>, at: Option<Position>, byte: u64) -> Error {
    let Some(path) = path else {
        return error;
    };
    error.with_location(Location {
        path: path.to_owned(),
        line: at.map(|position| position.line),
        column: at.map(|position| position.column),
        end_line: None,
        end_column: None,
        byte: Some(byte),
    })
}

fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn code_point(character: char) -> String {
    format!("U+{:04X}", u32::from(character))
}

/// Escapes a path for a string literal of a program.
fn escape(path: &str) -> String {
    path.replace('\\', "\\\\").replace('"', "\\\"")
}

impl fmt::Display for CodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownEncoding { label } => {
                write!(f, "unknown encoding `{label}`: use one of ")?;
                for (index, codec) in Codec::ALL.iter().enumerate() {
                    if index > 0 {
                        f.write_str(", ")?;
                    }
                    f.write_str(codec.label())?;
                }
                Ok(())
            }
            Self::BomNotSupported { codec } => write!(
                f,
                "encoding `{}` has no byte order mark: set bom to false",
                codec.label()
            ),
            Self::MissingBom { codec } => write!(
                f,
                "content does not start with the `{}` byte order mark: decode it without the byte order mark",
                codec.label()
            ),
            Self::Malformed {
                codec,
                bytes,
                position,
                ..
            } => write!(
                f,
                "invalid `{}` byte sequence `{}` at line {}, column {}: read the file with its actual encoding",
                codec.label(),
                hex(bytes),
                position.line,
                position.column
            ),
            Self::Unrepresentable {
                codec,
                character,
                position,
                ..
            } => write!(
                f,
                "character `{}` ({}) at line {}, column {} cannot be represented in `{}`: convert the file to utf-8 or remove the character",
                character.escape_debug(),
                code_point(*character),
                position.line,
                position.column,
                codec.label()
            ),
        }
    }
}

impl std::error::Error for CodecError {}

impl From<CodecError> for Error {
    fn from(error: CodecError) -> Self {
        error.into_error(None)
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
