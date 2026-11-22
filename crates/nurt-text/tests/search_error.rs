// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Codes, messages and sources of search errors.

use std::error::Error;
use std::io;

use nurt_contracts::error::ErrorCode;
use nurt_text::search::SearchError;

fn pattern_error(position: Option<usize>) -> SearchError {
    SearchError::Pattern {
        pattern: 1,
        position,
        message: "unclosed group".to_owned(),
    }
}

#[test]
fn maps_variants_to_error_codes() {
    assert_eq!(pattern_error(None).code(), ErrorCode::Regex);
    assert_eq!(
        SearchError::TooLarge {
            pattern: 0,
            limit: 10
        }
        .code(),
        ErrorCode::Limit
    );
    assert_eq!(
        SearchError::Io(io::Error::other("read failed")).code(),
        ErrorCode::Io
    );
}

#[test]
fn names_pattern_and_character_position_in_message() {
    assert_eq!(
        pattern_error(Some(4)).to_string(),
        "invalid regular expression in pattern 1 at character 4: unclosed group"
    );
    assert_eq!(
        pattern_error(None).to_string(),
        "invalid regular expression in pattern 1: unclosed group"
    );
}

#[test]
fn names_size_limit_in_message() {
    let text = SearchError::TooLarge {
        pattern: 2,
        limit: 4096,
    }
    .to_string();
    assert!(text.contains("pattern 2"), "{text}");
    assert!(text.contains("4096 bytes"), "{text}");
}

#[test]
fn exposes_io_error_as_source() {
    let error = SearchError::from(io::Error::other("read failed"));
    assert!(error.to_string().contains("read failed"));
    assert!(error.source().is_some());
    assert!(pattern_error(None).source().is_none());
    assert!(
        SearchError::TooLarge {
            pattern: 0,
            limit: 1
        }
        .source()
        .is_none()
    );
}
