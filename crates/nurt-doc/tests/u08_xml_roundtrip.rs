// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Round trip over the XML corpus `tests/d5/corpus/xml/advanced`: entities
//! of the internal subset, DocBook, XHTML, SVG, property lists, declared
//! single-byte encodings and references to external entities.
//!
//! Every file is decoded, parsed and encoded back to the same bytes. Its
//! value is read, or reading fails with `E_UNSUPPORTED_CONSTRUCT` for an
//! entity that the model does not expand. Up to [`EDITS_PER_FILE`] text
//! chunks per file are replaced: replacing a chunk with its own value changes
//! nothing, and a new value changes only the bytes of that chunk.
// Helper functions of this test target are not `#[test]` functions, which
// `allow-unwrap-in-tests` covers.
#![allow(clippy::unwrap_used)]

mod u08_support;

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use nurt_contracts::error::ErrorCode;
use nurt_doc::formats::xml::encoding::{XmlEncoding, parse_document};
use nurt_doc::formats::xml::entity::{ENTITY_MARKUP, EXTERNAL_ENTITY};
use nurt_doc::formats::xml::{Node, NodeId, XmlChange, XmlTree};
use u08_support::Codec;

/// Minimum number of corpus files.
const MIN_FILES: usize = 200;

/// Text chunks replaced in each file.
const EDITS_PER_FILE: usize = 4;

/// New value of a text chunk; no single-byte encoding of the corpus
/// represents all its characters.
const NEW_VALUE: &str = "\u{3A9} \u{20AC} \u{4E2D} new";

/// Encodings that the corpus must contain.
const ENCODINGS: [&str; 6] = [
    "utf-8",
    "us-ascii",
    "iso-8859-1",
    "iso-8859-2",
    "windows-1250",
    "windows-1252",
];

#[derive(Default)]
struct Outcome {
    files: usize,
    readable: usize,
    unsupported: BTreeMap<String, usize>,
    encodings: BTreeSet<String>,
    edits: usize,
    refused: usize,
    failures: Vec<String>,
}

fn chunks(tree: &XmlTree) -> Vec<NodeId> {
    tree.node_ids()
        .filter(|id| tree.element(*id).is_some())
        .flat_map(|id| tree.text_chunks(id))
        .collect()
}

fn chunk_values(tree: &XmlTree) -> Vec<Option<String>> {
    chunks(tree)
        .into_iter()
        .map(|id| tree.chunk_value(id).ok().flatten())
        .collect()
}

/// `true` when the new value of the chunk is written inside a CDATA
/// section, where character references are not recognized.
fn written_as_cdata(tree: &XmlTree, node: NodeId, change: &XmlChange) -> bool {
    matches!(tree.node(node), Node::CData(_))
        || change
            .edits
            .iter()
            .any(|edit| edit.replacement.contains("<![CDATA["))
}

fn encoded_len(encoding: &XmlEncoding, text: &str) -> Result<usize, String> {
    encoding
        .encode(text, Some(&Codec))
        .map(|bytes| bytes.len())
        .map_err(|error| error.to_string())
}

fn check_edit(
    file: &str,
    bytes: &[u8],
    encoding: &XmlEncoding,
    tree: &XmlTree,
    index: usize,
    outcome: &mut Outcome,
) -> Result<(), String> {
    let fail = |message: String| format!("{file}: chunk {index}: {message}");
    let node = chunks(tree)[index];
    let value = tree
        .chunk_value(node)
        .map_err(|error| fail(error.to_string()))?
        .unwrap_or_default();
    let same = tree
        .replace_text(node, &value)
        .map_err(|error| fail(error.to_string()))?;
    if !same.is_empty() {
        return Err(fail(
            "replacing a value with itself changes the document".to_owned(),
        ));
    }
    let change = tree
        .replace_text(node, NEW_VALUE)
        .map_err(|error| fail(error.to_string()))?;
    let written = match encoding.encode_change(tree, &change, Some(&Codec)) {
        Ok(written) => written,
        Err(error)
            if error.code() == ErrorCode::Encoding
                && !encoding.is_unicode()
                && written_as_cdata(tree, node, &change) =>
        {
            outcome.refused += 1;
            return Ok(());
        }
        Err(error) => return Err(fail(error.to_string())),
    };
    let start = change
        .edits
        .iter()
        .map(|edit| edit.range.start)
        .min()
        .unwrap_or_default();
    let end = change
        .edits
        .iter()
        .map(|edit| edit.range.end)
        .max()
        .unwrap_or_default();
    let text = tree.text();
    let head = encoded_len(encoding, &text[..start]).map_err(fail)?;
    let tail = bytes.len() - encoded_len(encoding, &text[..end]).map_err(fail)?;
    if !written.starts_with(&bytes[..head]) || !written.ends_with(&bytes[bytes.len() - tail..]) {
        return Err(fail("bytes outside the edited text changed".to_owned()));
    }
    let (_, reread) = parse_document(&written, Some(&Codec))
        .map_err(|error| fail(format!("the edited document is not read: {error}")))?;
    let mut expected = chunk_values(tree);
    expected[index] = Some(NEW_VALUE.to_owned());
    if chunk_values(&reread) != expected {
        return Err(fail(
            "the edit changed the value of another text chunk".to_owned(),
        ));
    }
    outcome.edits += 1;
    Ok(())
}

fn round_trip(file: &str, bytes: &[u8], outcome: &mut Outcome) -> Result<(), String> {
    let (encoding, tree) = parse_document(bytes, Some(&Codec))
        .map_err(|error| format!("{file}: not read: {error}"))?;
    outcome.encodings.insert(encoding.label().to_owned());
    if encoding.encode(tree.text(), Some(&Codec)).ok().as_deref() != Some(bytes) {
        return Err(format!(
            "{file}: the decoded text does not encode to the original bytes"
        ));
    }
    match tree.to_value() {
        Ok(_) => outcome.readable += 1,
        Err(error) if error.code() == ErrorCode::UnsupportedConstruct => {
            let construct = error.data().found.clone().unwrap_or_default();
            *outcome.unsupported.entry(construct).or_default() += 1;
        }
        Err(error) => return Err(format!("{file}: the value is not read: {error}")),
    }
    let editable: Vec<usize> = chunk_values(&tree)
        .iter()
        .enumerate()
        .filter(|(_, value)| value.as_deref().is_some_and(|value| !value.is_empty()))
        .map(|(index, _)| index)
        .take(EDITS_PER_FILE)
        .collect();
    for index in editable {
        check_edit(file, bytes, &encoding, &tree, index, outcome)?;
    }
    Ok(())
}

fn check_file(path: &Path, outcome: &mut Outcome) {
    let file = u08_support::corpus_relative(path);
    let bytes = std::fs::read(path).unwrap();
    outcome.files += 1;
    if let Err(failure) = round_trip(&file, &bytes, outcome) {
        outcome.failures.push(failure);
    }
}

#[test]
fn advanced_corpus_round_trips_and_isolates_text_changes() {
    let mut outcome = Outcome::default();
    for path in u08_support::files_below(&u08_support::corpus_dir("xml/advanced")) {
        check_file(&path, &mut outcome);
    }
    u08_support::write_summary(
        "u08-xml-roundtrip.json",
        &serde_json::json!({
            "files": outcome.files,
            "readable": outcome.readable,
            "unsupported": outcome.unsupported,
            "encodings": outcome.encodings,
            "edits": outcome.edits,
            "refused_cdata_edits": outcome.refused,
            "failures": outcome.failures,
        }),
    );
    assert!(outcome.files >= MIN_FILES, "{} files", outcome.files);
    assert!(
        outcome.failures.is_empty(),
        "{} of {} files:\n{}",
        outcome.failures.len(),
        outcome.files,
        outcome.failures.join("\n")
    );
    for label in ENCODINGS {
        assert!(outcome.encodings.contains(label), "no file in {label}");
    }
    let constructs: BTreeSet<&str> = outcome.unsupported.keys().map(String::as_str).collect();
    assert!(constructs.contains(EXTERNAL_ENTITY), "{constructs:?}");
    assert!(
        constructs.is_subset(&BTreeSet::from([EXTERNAL_ENTITY, ENTITY_MARKUP])),
        "{constructs:?}"
    );
    assert!(outcome.edits > outcome.files, "{} edits", outcome.edits);
}
