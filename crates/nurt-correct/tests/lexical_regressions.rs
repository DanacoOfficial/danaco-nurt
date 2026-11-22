// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
// A failed unwrap, expectation or panic is how a test reports a failure.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Inputs that once made the front part of correction run without end. Each
//! input is read on its own thread within a time limit, so that a regression
//! fails the test instead of stopping the test run.

mod lexical_support;

use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use lexical_support::{Catalog, correction, syntax};
use nurt_contracts::kinds::SyntaxVariant;
use nurt_correct::lexical::front;
use nurt_correct::tables::load_embedded;

/// Time limit of reading one input; a debug build reads each of them in a
/// few milliseconds.
const LIMIT: Duration = Duration::from_secs(2);

/// Whether `front` returns within [`LIMIT`] for `text` in every variant.
fn returns_in_time(text: &'static str) -> bool {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let tables = load_embedded().expect("embedded tables");
        let catalog = Catalog::load();
        for variant in [SyntaxVariant::A, SyntaxVariant::B, SyntaxVariant::C] {
            let _ = front(text, &syntax(variant), &correction(), &tables, &catalog);
        }
        let _ = sender.send(());
    });
    receiver.recv_timeout(LIMIT).is_ok()
}

#[test]
fn reads_a_unicode_space_after_an_escaped_quote_in_time() {
    let text = "find \"/a\\\"\u{2003}|> count";
    assert!(returns_in_time(text), "{text:?}");
}

#[test]
fn reads_unicode_spaces_inside_an_unterminated_text_in_time() {
    for text in ["list \"\\\"\u{3000}x", "\"\\\"\u{2009}", "x\u{2002}y"] {
        assert!(returns_in_time(text), "{text:?}");
    }
}
