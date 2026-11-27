// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Notable entries of the effect report.
//!
//! Stages record unusual effects with
//! [`ExecCtx::note`](nurt_contracts::stage::ExecCtx::note) or in the
//! `notable` list of an effect plan; these constructors give the entries the
//! form the report expects.

use nurt_contracts::fs::AbsPath;
use nurt_contracts::report::{Notable, NotableEntry};
use nurt_contracts::services::LineEnding;
use nurt_text::codec::Codec;

use super::text::{TextFile, record_line_ending};

/// Entry concerning one target.
#[must_use]
pub fn at_path(code: Notable, path: &AbsPath) -> NotableEntry {
    NotableEntry {
        path: Some(path.to_language_string()),
        raw_path: path.raw_bytes(),
        ..NotableEntry::new(code)
    }
}

/// Entry concerning `count` targets or occurrences, for example
/// [`Notable::BinarySkipped`].
#[must_use]
pub fn counted(code: Notable, count: u64) -> NotableEntry {
    NotableEntry {
        count: Some(count),
        ..NotableEntry::new(code)
    }
}

/// Entries for a text file written in its original form: an encoding other
/// than UTF-8 ([`Notable::EncodingPreserved`]), line endings other than `lf`
/// ([`Notable::LineEndingPreserved`]) and mixed line endings
/// ([`Notable::LineEndingMixed`]). The `detail` holds the encoding or the
/// line endings.
#[must_use]
pub fn preserved_text(original: &TextFile) -> Vec<NotableEntry> {
    let mut entries = Vec::new();
    if original.encoding.label != Codec::Utf8.label() {
        entries.push(NotableEntry {
            detail: Some(original.encoding.label.clone()),
            ..at_path(Notable::EncodingPreserved, &original.path)
        });
    }
    let code = match original.line_ending {
        LineEnding::CrLf | LineEnding::Cr => Some(Notable::LineEndingPreserved),
        LineEnding::Mixed => Some(Notable::LineEndingMixed),
        _ => None,
    };
    if let Some(code) = code {
        entries.push(NotableEntry {
            detail: Some(record_line_ending(original.line_ending).as_str().to_owned()),
            ..at_path(code, &original.path)
        });
    }
    entries
}

#[cfg(test)]
mod tests {
    use nurt_contracts::fs::{Follow, Fs};
    use nurt_contracts::services::TextEncoding;
    use nurt_testkit::ctx::TestContext;

    use super::*;
    use crate::common::fixture::abs;

    fn text_file(label: &str, line_ending: LineEnding) -> TextFile {
        let test = TestContext::new();
        let path = abs("/data/a.txt");
        test.fs().write_file(&path, b"a").unwrap();
        TextFile {
            meta: test.fs().metadata(&path, Follow::NoFollow).unwrap(),
            path,
            size: 1,
            hash: nurt_text::hash::content_hash(b"a"),
            text: "a".to_owned(),
            encoding: TextEncoding {
                label: label.to_owned(),
                bom: false,
            },
            line_ending,
        }
    }

    #[test]
    fn entries_carry_path_or_count() {
        let path = abs("/data/a.txt");
        let entry = at_path(Notable::Unchanged, &path);
        assert_eq!(entry.code, Notable::Unchanged);
        assert_eq!(entry.path, Some(path.to_language_string()));
        assert_eq!(entry.severity, Notable::Unchanged.default_severity());
        let entry = counted(Notable::BinarySkipped, 3);
        assert_eq!(entry.count, Some(3));
        assert_eq!(entry.path, None);
    }

    #[test]
    fn utf8_with_line_feeds_is_not_notable() {
        assert!(preserved_text(&text_file("utf-8", LineEnding::Lf)).is_empty());
        assert!(preserved_text(&text_file("utf-8", LineEnding::None)).is_empty());
    }

    #[test]
    fn legacy_encoding_and_crlf_are_notable() {
        let entries = preserved_text(&text_file("windows-1250", LineEnding::CrLf));
        let codes: Vec<Notable> = entries.iter().map(|entry| entry.code).collect();
        assert_eq!(
            codes,
            [Notable::EncodingPreserved, Notable::LineEndingPreserved]
        );
        assert_eq!(entries[0].detail.as_deref(), Some("windows-1250"));
        assert_eq!(entries[1].detail.as_deref(), Some("crlf"));
    }

    #[test]
    fn mixed_line_endings_are_notable() {
        let entries = preserved_text(&text_file("utf-8", LineEnding::Mixed));
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].code, Notable::LineEndingMixed);
        let entries = preserved_text(&text_file("utf-8", LineEnding::Cr));
        assert_eq!(entries[0].detail.as_deref(), Some("cr"));
    }
}
