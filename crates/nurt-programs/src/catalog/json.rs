// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Reading of the JSON files of a catalog under size and nesting limits.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

use serde_json::value::RawValue;

/// Byte order mark of UTF-8.
const BOM: &str = "\u{feff}";

/// Failure to read a JSON file of a catalog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum JsonError {
    /// The file cannot be read.
    Io(String),
    /// The file is larger than the limit.
    TooLarge(u64),
    /// The file is not UTF-8.
    Encoding,
    /// Values are nested deeper than the limit.
    TooDeep(usize),
    /// The text is not JSON, or not of the expected shape.
    Syntax { line: u64, message: String },
}

impl JsonError {
    /// Line of the fault, when known.
    pub(crate) fn line(&self) -> Option<u64> {
        match self {
            Self::Syntax { line, .. } => Some(*line),
            _ => None,
        }
    }

    /// Message that states what to correct.
    pub(crate) fn message(&self) -> String {
        match self {
            Self::Io(reason) => format!("file cannot be read: {reason}"),
            Self::TooLarge(limit) => {
                format!("file is larger than {limit} bytes; split it or shorten the texts")
            }
            Self::Encoding => "file is not valid UTF-8; save it as UTF-8".to_owned(),
            Self::TooDeep(limit) => {
                format!("values are nested deeper than {limit} levels; flatten the data")
            }
            Self::Syntax { message, .. } => format!("invalid JSON: {message}"),
        }
    }
}

/// Reads a file of at most `limit` bytes as text without the byte order mark.
pub(crate) fn read_text(path: &Path, limit: u64) -> Result<String, JsonError> {
    let file = File::open(path).map_err(|error| JsonError::Io(error.to_string()))?;
    let mut bytes = Vec::new();
    file.take(limit.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|error| JsonError::Io(error.to_string()))?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > limit {
        return Err(JsonError::TooLarge(limit));
    }
    let text = String::from_utf8(bytes).map_err(|_| JsonError::Encoding)?;
    Ok(match text.strip_prefix(BOM) {
        Some(rest) => rest.to_owned(),
        None => text,
    })
}

/// Size of a file in bytes.
pub(crate) fn file_size(path: &Path) -> io::Result<u64> {
    std::fs::metadata(path).map(|meta| meta.len())
}

/// Deepest nesting of arrays and objects in a JSON text; brackets inside
/// strings are not counted.
pub(crate) fn max_depth(text: &str) -> usize {
    let mut depth = 0usize;
    let mut deepest = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for byte in text.bytes() {
        if in_string {
            match byte {
                _ if escaped => escaped = false,
                b'\\' => escaped = true,
                b'"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match byte {
            b'"' => in_string = true,
            b'[' | b'{' => {
                depth += 1;
                deepest = deepest.max(depth);
            }
            b']' | b'}' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    deepest
}

/// Line, counted from 1, of the byte at `offset`.
pub(crate) fn line_at(text: &str, offset: usize) -> u64 {
    let end = offset.min(text.len());
    let lines = text.as_bytes()[..end]
        .iter()
        .filter(|byte| **byte == b'\n')
        .count();
    u64::try_from(lines).map_or(u64::MAX, |lines| lines + 1)
}

/// Byte offset of `part` within `text`, where `part` is a slice of `text`.
pub(crate) fn offset_of(text: &str, part: &str) -> usize {
    (part.as_ptr() as usize).saturating_sub(text.as_ptr() as usize)
}

/// Members of a JSON object, each kept as unparsed text with its position.
pub(crate) type RawObject<'a> = BTreeMap<String, &'a RawValue>;

/// Parses `text` as a JSON object whose members stay unparsed.
pub(crate) fn parse_object(text: &str, depth_limit: usize) -> Result<RawObject<'_>, JsonError> {
    if max_depth(text) > depth_limit {
        return Err(JsonError::TooDeep(depth_limit));
    }
    if !text.trim_start().starts_with('{') {
        return Err(JsonError::Syntax {
            line: 1,
            message: "the file must contain one JSON object".to_owned(),
        });
    }
    serde_json::from_str(text).map_err(|error| syntax(&error))
}

/// Parses an unparsed member as an array whose items stay unparsed.
pub(crate) fn parse_array<'a>(text: &'a str, raw: &'a RawValue) -> Result<Vec<&'a RawValue>, JsonError> {
    serde_json::from_str(raw.get()).map_err(|error| JsonError::Syntax {
        line: line_at(text, offset_of(text, raw.get())),
        message: error.to_string(),
    })
}

fn syntax(error: &serde_json::Error) -> JsonError {
    JsonError::Syntax {
        line: u64::try_from(error.line()).unwrap_or(0).max(1),
        message: error.to_string(),
    }
}
