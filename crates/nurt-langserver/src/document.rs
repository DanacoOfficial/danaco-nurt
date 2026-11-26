// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Open documents of a session.

use std::collections::BTreeMap;

use lsp_types::{TextDocumentContentChangeEvent, Uri};

use crate::position::{Encoding, Lines};

/// Text and version of an open document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Document {
    /// Current text.
    pub(crate) text: String,
    /// Version assigned by the client.
    pub(crate) version: i32,
}

/// Open documents by URI.
#[derive(Debug, Default)]
pub(crate) struct Documents {
    open: BTreeMap<Uri, Document>,
}

impl Documents {
    /// Opens a document, replacing an open document with the same URI.
    pub(crate) fn open(&mut self, uri: Uri, text: String, version: i32) {
        self.open.insert(uri, Document { text, version });
    }

    /// Applies the changes of a `didChange` notification in order. A change
    /// without a range replaces the whole text. Returns `false` when the
    /// document is not open.
    pub(crate) fn change(
        &mut self,
        uri: &Uri,
        version: i32,
        changes: Vec<TextDocumentContentChangeEvent>,
        encoding: Encoding,
    ) -> bool {
        let Some(document) = self.open.get_mut(uri) else {
            return false;
        };
        for change in changes {
            apply(&mut document.text, change, encoding);
        }
        document.version = version;
        true
    }

    /// Closes a document. Returns `false` when it was not open.
    pub(crate) fn close(&mut self, uri: &Uri) -> bool {
        self.open.remove(uri).is_some()
    }

    /// Returns an open document.
    pub(crate) fn get(&self, uri: &Uri) -> Option<&Document> {
        self.open.get(uri)
    }
}

fn apply(text: &mut String, change: TextDocumentContentChangeEvent, encoding: Encoding) {
    let Some(range) = change.range else {
        *text = change.text;
        return;
    };
    let (start, end) = {
        let lines = Lines::new(text);
        let start = lines.offset(range.start, encoding);
        (start, lines.offset(range.end, encoding).max(start))
    };
    text.replace_range(start..end, &change.text);
}

#[cfg(test)]
mod tests {
    use lsp_types::{Position, Range};

    use super::*;

    fn uri() -> Uri {
        "file:///work/a.nurt".parse().unwrap()
    }

    fn edit(range: Option<Range>, text: &str) -> TextDocumentContentChangeEvent {
        TextDocumentContentChangeEvent {
            range,
            range_length: None,
            text: text.to_owned(),
        }
    }

    #[test]
    fn replaces_the_whole_text_without_a_range() {
        let mut documents = Documents::default();
        documents.open(uri(), "count".to_owned(), 1);
        assert!(documents.change(&uri(), 2, vec![edit(None, "first 2")], Encoding::Utf16));
        let document = documents.get(&uri()).unwrap();
        assert_eq!((document.text.as_str(), document.version), ("first 2", 2));
    }

    #[test]
    fn applies_ranged_changes_in_utf16_units() {
        let mut documents = Documents::default();
        documents.open(uri(), "find \"\u{1f600}\"\n|> count".to_owned(), 1);
        let after_emoji = Range::new(Position::new(0, 9), Position::new(0, 9));
        let last_word = Range::new(Position::new(1, 3), Position::new(1, 8));
        let changes = vec![
            edit(Some(after_emoji), "x"),
            edit(Some(last_word), "first 2"),
        ];
        assert!(documents.change(&uri(), 3, changes, Encoding::Utf16));
        assert_eq!(
            documents.get(&uri()).unwrap().text,
            "find \"\u{1f600}\"x\n|> first 2"
        );
    }

    #[test]
    fn ranges_beyond_the_text_are_clamped_and_reversed_ranges_insert() {
        let mut documents = Documents::default();
        documents.open(uri(), "ab".to_owned(), 1);
        let beyond = Range::new(Position::new(5, 0), Position::new(9, 9));
        assert!(documents.change(&uri(), 2, vec![edit(Some(beyond), "x")], Encoding::Utf16));
        assert_eq!(documents.get(&uri()).unwrap().text, "abx");
        let reversed = Range::new(Position::new(0, 2), Position::new(0, 0));
        assert!(documents.change(&uri(), 3, vec![edit(Some(reversed), "-")], Encoding::Utf16));
        assert_eq!(documents.get(&uri()).unwrap().text, "ab-x");
    }

    #[test]
    fn counts_scalar_values_in_utf32_and_keeps_line_breaks() {
        let mut documents = Documents::default();
        documents.open(uri(), "a\u{1f600}b\r\nc".to_owned(), 1);
        let after_emoji = Range::new(Position::new(0, 2), Position::new(0, 3));
        let second_line = Range::new(Position::new(1, 0), Position::new(1, 1));
        let changes = vec![edit(Some(after_emoji), "B"), edit(Some(second_line), "C")];
        assert!(documents.change(&uri(), 2, changes, Encoding::Utf32));
        assert_eq!(documents.get(&uri()).unwrap().text, "a\u{1f600}B\r\nC");
        let past_the_end = Range::new(Position::new(0, 99), Position::new(0, 99));
        assert!(documents.change(
            &uri(),
            3,
            vec![edit(Some(past_the_end), "!")],
            Encoding::Utf32
        ));
        assert_eq!(documents.get(&uri()).unwrap().text, "a\u{1f600}B!\r\nC");
    }

    #[test]
    fn a_range_inside_a_surrogate_pair_moves_to_the_start_of_the_character() {
        let mut documents = Documents::default();
        documents.open(uri(), "\u{1f600}".to_owned(), 1);
        let inside = Range::new(Position::new(0, 1), Position::new(0, 1));
        assert!(documents.change(&uri(), 2, vec![edit(Some(inside), "x")], Encoding::Utf16));
        assert_eq!(documents.get(&uri()).unwrap().text, "x\u{1f600}");
    }

    #[test]
    fn opening_again_replaces_the_text_and_the_version() {
        let mut documents = Documents::default();
        documents.open(uri(), "count".to_owned(), 1);
        documents.open(uri(), "find".to_owned(), 7);
        let document = documents.get(&uri()).unwrap();
        assert_eq!((document.text.as_str(), document.version), ("find", 7));
        let other: Uri = "file:///work/b.nurt".parse().unwrap();
        assert!(documents.get(&other).is_none());
    }

    #[test]
    fn changes_are_applied_in_order_and_the_last_version_is_kept() {
        let mut documents = Documents::default();
        documents.open(uri(), "count".to_owned(), 1);
        let changes = vec![
            edit(None, "first"),
            edit(
                Some(Range::new(Position::new(0, 5), Position::new(0, 5))),
                " 2",
            ),
        ];
        assert!(documents.change(&uri(), 9, changes, Encoding::Utf16));
        let document = documents.get(&uri()).unwrap();
        assert_eq!((document.text.as_str(), document.version), ("first 2", 9));
    }

    #[test]
    fn ignores_changes_and_closing_of_unknown_documents() {
        let mut documents = Documents::default();
        assert!(!documents.change(&uri(), 2, vec![edit(None, "count")], Encoding::Utf32));
        assert!(!documents.close(&uri()));
        documents.open(uri(), "count".to_owned(), 1);
        assert!(documents.close(&uri()));
        assert!(documents.get(&uri()).is_none());
    }
}
