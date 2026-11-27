// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-doc::formats::yaml::multidoc
// Responsibility:   Documents of a YAML stream: the document selector of a key path.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Documents of a YAML stream: the document selector of a key path
//! (`#1:spec.replicas`, `#*:kind`), `item.document` and the position of new
//! content in an empty document.
//!
//! A key path without a selector addresses the first document. Directives,
//! `---` and `...` markers and comments between documents are outside every
//! node, so edits of nodes never touch them.

use nurt_contracts::error::{Candidate, Error, ErrorCode};
use nurt_contracts::keypath::{DocSelector, KeyPath};

use super::comment::Comments;
use super::model::YamlModel;

/// Indices of the documents selected by `selector`, in stream order.
///
/// No selector selects the first document, `#N:` the document with index
/// `N`, `#*:` every document. The result is empty when the stream has no
/// such document.
pub fn select(model: &YamlModel, selector: Option<DocSelector>) -> Vec<usize> {
    let count = model.documents().len();
    match selector {
        None => (count > 0).then_some(0).into_iter().collect(),
        Some(DocSelector::Index(index)) => usize::try_from(index)
            .ok()
            .filter(|&index| index < count)
            .into_iter()
            .collect(),
        Some(DocSelector::All) => (0..count).collect(),
    }
}

/// Value of `item.document` for a node of the document `index`: the index
/// when the stream has more than one document, otherwise `None`.
pub fn item_document(model: &YamlModel, index: usize) -> Option<u64> {
    (model.documents().len() > 1)
        .then(|| u64::try_from(index).ok())
        .flatten()
}

/// Selector that addresses the document `index` in a canonical key path;
/// `None` for the first document.
pub fn selector_of(index: usize) -> Option<DocSelector> {
    u32::try_from(index)
        .ok()
        .filter(|&index| index > 0)
        .map(DocSelector::Index)
}

/// Error for a key path whose selector addresses a document that the
/// stream does not have.
///
/// The candidates are the selectors of the existing documents.
pub fn document_not_found(model: &YamlModel, key: &KeyPath) -> Error {
    let count = model.documents().len();
    let mut error = Error::new(
        ErrorCode::KeyNotFound,
        format!("key `{key}` selects a document that does not exist; the file has {count} documents, select one with `#0:` to `#{}:`", count.saturating_sub(1)),
    )
    .with_detail("key", key.to_string().into())
    .with_detail("documents", count.into());
    for index in 0..count.min(MAX_DOCUMENT_CANDIDATES) {
        let candidate = KeyPath::new(selector_of(index), key.segments().to_vec())
            .map_or_else(|_| format!("#{index}:"), |path| path.to_string());
        error = error.with_candidate(Candidate {
            rank: u32::try_from(index + 1).unwrap_or(u32::MAX),
            reference: None,
            program: candidate,
            why: None,
        });
    }
    error
}

/// Number of document selectors offered as candidates in an error.
pub const MAX_DOCUMENT_CANDIDATES: usize = 8;

/// `item.comment` of a document root: the comment lines before the root
/// node that are not the leading comment of its first entry or item.
pub fn document_comment(model: &YamlModel, index: usize) -> Option<String> {
    let document = model.documents().get(index)?;
    let root = model.node(document.root);
    let first = first_token(model, index)?;
    let start = document
        .directives
        .last()
        .map(|directive| directive.span.end)
        .or_else(|| document.start_marker.as_ref().map(|marker| marker.start))
        .unwrap_or_else(|| previous_document_end(model, index));
    let comments = Comments::new(model);
    let first = if root.is_empty_node() {
        document.span.end.max(first)
    } else {
        first
    };
    comments.join(&comments.document(start, first))
}

/// Offset where content is inserted into the document `index` when its root
/// is the empty node, and whether the content starts on a new line.
///
/// Content follows the `---` marker on the next line; in a document
/// without a marker it starts at the empty root.
pub fn empty_root_insertion(model: &YamlModel, index: usize) -> Option<(usize, bool)> {
    let document = model.documents().get(index)?;
    let root = model.node(document.root);
    if !root.is_empty_node() {
        return None;
    }
    match &document.start_marker {
        Some(marker) if model.line_start(root.span.start) == model.line_start(marker.start) => {
            let after_marker = model.line_end(marker.end);
            let comment = Comments::new(model).inline_after(marker.end);
            let end = comment.map_or(after_marker, |span| span.end.max(after_marker));
            Some((end, true))
        }
        _ => Some((root.span.start, false)),
    }
}

/// First byte of the root of the document `index`, including its anchor and
/// tag; for an empty root, the end of the document.
fn first_token(model: &YamlModel, index: usize) -> Option<usize> {
    let document = model.documents().get(index)?;
    let root = model.node(document.root);
    Some(if root.is_empty_node() {
        document.span.end
    } else {
        root.full_span().start
    })
}

fn previous_document_end(model: &YamlModel, index: usize) -> usize {
    index
        .checked_sub(1)
        .and_then(|previous| model.documents().get(previous))
        .map_or(0, |document| document.span.end)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> YamlModel {
        YamlModel::parse(text).expect("valid YAML")
    }

    /// Stream of `count` documents, each with the key `index`.
    fn stream(count: usize) -> String {
        (0..count)
            .map(|index| format!("---\nindex: {index}\n"))
            .collect()
    }

    #[test]
    fn selects_documents_by_index_or_all() {
        let model = parse(&stream(3));
        assert_eq!(select(&model, None), vec![0]);
        assert_eq!(select(&model, Some(DocSelector::Index(2))), vec![2]);
        assert!(select(&model, Some(DocSelector::Index(3))).is_empty());
        assert_eq!(select(&model, Some(DocSelector::All)), vec![0, 1, 2]);
        assert_eq!(item_document(&model, 1), Some(1));
        assert_eq!(item_document(&parse("a: 1\n"), 0), None);
        assert_eq!(selector_of(0), None);
        assert_eq!(selector_of(2), Some(DocSelector::Index(2)));
    }

    #[test]
    fn offers_the_first_documents_as_candidates_of_a_missing_document() {
        let count = MAX_DOCUMENT_CANDIDATES + 2;
        let model = parse(&stream(count));
        let key = KeyPath::parse("#12:index").expect("key path");
        let error = document_not_found(&model, &key);
        assert_eq!(error.code, ErrorCode::KeyNotFound);
        assert_eq!(error.candidates.len(), MAX_DOCUMENT_CANDIDATES);
        assert_eq!(error.candidates[0].program, "index");
        assert_eq!(error.candidates[1].program, "#1:index");
        assert!(error.message.contains(&format!("{count} documents")));
    }

    #[test]
    fn reads_the_comment_of_a_document_with_an_empty_root() {
        let model = parse("---\n# reserved\n...\n");
        let comment = document_comment(&model, 0);
        assert!(
            comment
                .as_deref()
                .is_some_and(|comment| comment.contains("reserved")),
            "{comment:?}"
        );
        assert_eq!(document_comment(&model, 1), None);
        assert_eq!(empty_root_insertion(&model, 1), None);
        assert_eq!(empty_root_insertion(&parse("a: 1\n"), 0), None);
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
