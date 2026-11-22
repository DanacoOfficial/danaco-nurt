// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Property tests of the XML parser and editor on mutated corpus files.
//!
//! Mutations insert, delete or replace characters taken from XML markup.
//! The parser either rejects the result with `E_DOC_PARSE` or
//! `E_UNSUPPORTED_CONSTRUCT`, or accepts it with node ranges that lie on
//! character boundaries and cover the document; edits of accepted
//! documents parse again.

// Helpers outside `#[test]` functions fail the test on invalid fixtures.
#![allow(clippy::unwrap_used, clippy::panic)]

use std::fs;
use std::path::{Path, PathBuf};

use nurt_contracts::error::ErrorCode;
use nurt_contracts::keypath::KeyPath;
use nurt_contracts::value::Value;
use nurt_doc::formats::xml::{Node, XmlTree};
use proptest::prelude::*;

/// Characters used by mutations.
const ALPHABET: &[char] = &[
    '<', '>', '/', '?', '!', '-', '[', ']', '&', ';', '#', 'x', '"', '\'', '=', ' ', '\n', 'a',
    ':', 'C', 'D', 'A', 'T', '\u{e9}',
];

/// Mutations applied to one document at most.
const MAX_MUTATIONS: usize = 4;

fn corpus_files() -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/d5/corpus/xml/core");
    let mut files: Vec<PathBuf> = fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext != "md"))
        .collect();
    files.sort();
    files
}

#[derive(Debug, Clone)]
enum Mutation {
    Insert(usize, char),
    Delete(usize),
    Replace(usize, char),
}

fn mutation() -> impl Strategy<Value = Mutation> {
    let symbol = proptest::sample::select(ALPHABET);
    prop_oneof![
        (any::<usize>(), symbol.clone()).prop_map(|(at, c)| Mutation::Insert(at, c)),
        any::<usize>().prop_map(Mutation::Delete),
        (any::<usize>(), symbol).prop_map(|(at, c)| Mutation::Replace(at, c)),
    ]
}

fn mutate(text: &str, mutations: &[Mutation]) -> String {
    let mut chars: Vec<char> = text.chars().collect();
    for mutation in mutations {
        let len = chars.len().max(1);
        match *mutation {
            Mutation::Insert(at, c) => chars.insert(at % (chars.len() + 1), c),
            Mutation::Delete(at) if !chars.is_empty() => {
                chars.remove(at % len);
            }
            Mutation::Replace(at, c) if !chars.is_empty() => chars[at % len] = c,
            _ => {}
        }
    }
    chars.into_iter().collect()
}

fn check_ranges(tree: &XmlTree) {
    let text = tree.text();
    for id in tree.node_ids() {
        let span = tree.node(id).span();
        assert!(span.start <= span.end && span.end <= text.len());
        assert!(text.is_char_boundary(span.start) && text.is_char_boundary(span.end));
        if let Node::Element(element) = tree.node(id) {
            for attribute in &element.attributes {
                assert_eq!(
                    text.as_bytes()[attribute.span.end - 1] as char,
                    attribute.quote
                );
            }
        }
    }
    let last = tree
        .document_nodes()
        .last()
        .map_or(0, |id| tree.node(*id).span().end);
    assert_eq!(last, text.len());
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn mutated_documents_are_rejected_or_parsed_consistently(
        file in 0usize..1024,
        mutations in proptest::collection::vec(mutation(), 1..=MAX_MUTATIONS),
    ) {
        let files = corpus_files();
        let source = fs::read_to_string(&files[file % files.len()]).unwrap();
        let text = mutate(&source, &mutations);
        match XmlTree::parse(&text) {
            Ok(tree) => {
                prop_assert_eq!(tree.text(), text.as_str());
                check_ranges(&tree);
                let _ = tree.to_value();
                let root = KeyPath::parse(&tree.canonical_key(nurt_doc::formats::xml::Target::Element(tree.root()))).unwrap();
                let added = KeyPath::parse(&format!("{root}.mutation-check")).unwrap();
                if let Ok(change) = tree.set(&added, &Value::Text("v".into()), true) {
                    let output = change.apply(&text).unwrap();
                    prop_assert!(XmlTree::parse(&output).is_ok(), "{}", output);
                }
            }
            Err(error) => prop_assert!(
                matches!(error.code(), ErrorCode::DocParse | ErrorCode::UnsupportedConstruct),
                "{}", error
            ),
        }
    }
}
