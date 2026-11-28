// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-doc/tests/u08_support/mod.rs
// Responsibility:   Shared fixtures of the XML entity, encoding and differential tests.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Shared fixtures of the XML entity, encoding and differential tests.
// Each test target compiles this module and uses a different subset of it.
#![allow(dead_code)]

use std::path::{Path, PathBuf};

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::services::{LineEnding, TextCodecService, TextDetection, TextEncoding};

/// Text codec backed by `encoding_rs` that refuses lossy conversions.
pub(crate) struct Codec;

impl Codec {
    fn utf16(label: &str) -> Option<bool> {
        match label {
            "utf-16le" => Some(false),
            "utf-16be" => Some(true),
            _ => None,
        }
    }

    fn lookup(label: &str) -> Result<&'static encoding_rs::Encoding> {
        encoding_rs::Encoding::for_label(label.as_bytes())
            .ok_or_else(|| Error::new(ErrorCode::Encoding, format!("unknown encoding `{label}`")))
    }
}

impl TextCodecService for Codec {
    fn detect(&self, bytes: &[u8]) -> TextDetection {
        TextDetection {
            binary: false,
            encoding: Some(TextEncoding::utf8()),
            line_ending: if bytes.contains(&b'\n') {
                LineEnding::Lf
            } else {
                LineEnding::None
            },
        }
    }

    fn decode(&self, bytes: &[u8], encoding: &TextEncoding) -> Result<String> {
        let decoder = Self::lookup(&encoding.label)?;
        decoder
            .decode_without_bom_handling_and_without_replacement(bytes)
            .map(|text| text.into_owned())
            .ok_or_else(|| {
                Error::new(
                    ErrorCode::Encoding,
                    format!("bytes are not valid {}", encoding.label),
                )
            })
    }

    fn encode(&self, text: &str, encoding: &TextEncoding) -> Result<Vec<u8>> {
        if let Some(big_endian) = Self::utf16(&encoding.label) {
            return Ok(text
                .encode_utf16()
                .flat_map(|unit| {
                    if big_endian {
                        unit.to_be_bytes()
                    } else {
                        unit.to_le_bytes()
                    }
                })
                .collect());
        }
        let (bytes, _, unmappable) = Self::lookup(&encoding.label)?.encode(text);
        if unmappable {
            return Err(Error::new(
                ErrorCode::Encoding,
                format!("text has characters outside {}", encoding.label),
            ));
        }
        Ok(bytes.into_owned())
    }
}

/// Root of the repository.
pub(crate) fn repository_root() -> PathBuf {
    Path::new(
        std::env::var("CARGO_MANIFEST_DIR")
            .as_deref()
            .unwrap_or(env!("CARGO_MANIFEST_DIR")),
    )
    .join("../..")
}

/// Directory of the test corpus of a format.
pub(crate) fn corpus_dir(format: &str) -> PathBuf {
    repository_root().join("tests/d5/corpus").join(format)
}

/// Data directory of the XML entity and encoding tests.
pub(crate) fn data_dir() -> PathBuf {
    repository_root().join("tests/d5/data/u08")
}

/// Regular files below `dir` in sorted order.
pub(crate) fn files_below(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(current) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&current) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

/// Path of `path` relative to the corpus root, with `/` separators.
pub(crate) fn corpus_relative(path: &Path) -> String {
    let root = repository_root().join("tests/d5/corpus");
    path.strip_prefix(&root)
        .unwrap_or(path)
        .components()
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

/// Writes a JSON summary next to the build artifacts of the tests.
pub(crate) fn write_summary(name: &str, summary: &serde_json::Value) {
    let path = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let text = serde_json::to_string_pretty(summary).unwrap_or_default();
    std::fs::write(path, text).unwrap_or_default();
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
