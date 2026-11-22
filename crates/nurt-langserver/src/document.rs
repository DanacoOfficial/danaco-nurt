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
    fn ignores_changes_and_closing_of_unknown_documents() {
        let mut documents = Documents::default();
        assert!(!documents.change(&uri(), 2, vec![edit(None, "count")], Encoding::Utf32));
        assert!(!documents.close(&uri()));
        documents.open(uri(), "count".to_owned(), 1);
        assert!(documents.close(&uri()));
        assert!(documents.get(&uri()).is_none());
    }
}
