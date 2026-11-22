// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Reference documentation in Markdown, man and HTML.

// A failed unwrap, expectation or panic is how a test reports a failure.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use nurt_contracts::error::ErrorCode;
use nurt_help::reference::{ReferenceFormat, render};

#[test]
fn every_format_lists_every_command_and_parameter() {
    let catalog = common::catalog();
    for format in ReferenceFormat::ALL {
        let text = render(&catalog, "", format).unwrap();
        for spec in catalog.commands() {
            assert!(text.contains(&spec.name), "{format:?}: {}", spec.name);
            for param in &spec.params {
                assert!(
                    text.contains(&param.name),
                    "{format:?}: {}.{}",
                    spec.name,
                    param.name
                );
            }
        }
    }
}

#[test]
fn man_page_has_the_name_section_with_the_product_sentence() {
    let catalog = common::catalog();
    let text = render(&catalog, "", ReferenceFormat::Man).unwrap();
    assert!(text.starts_with(".TH NURT-REFERENCE 7\n.SH NAME\n"));
    assert!(text.contains("nurt \\- command shell and language where every change can be undone"));
}

#[test]
fn man_page_of_one_command_is_named_after_it() {
    let catalog = common::catalog();
    let text = render(&catalog, "git.status", ReferenceFormat::Man).unwrap();
    assert!(text.starts_with(".TH NURT-GIT-STATUS 7\n"));
    assert!(text.contains(".SS git.status"));
}

#[test]
fn man_text_has_no_unescaped_hyphens_or_leading_control_characters() {
    let catalog = common::catalog();
    let text = render(&catalog, "", ReferenceFormat::Man).unwrap();
    for line in text.lines().filter(|line| !line.starts_with('.')) {
        assert!(!line.contains("\\-\\-\\-\\-"), "{line}");
        assert!(!line.starts_with('\''), "{line}");
    }
    for line in text.lines().filter(|line| !line.starts_with(".B ")) {
        let plain = line.replace("\\-", "").replace("\\e", "");
        assert!(!plain.contains('-') || line.starts_with('.'), "{line}");
    }
}

#[test]
fn html_document_is_complete_and_escapes_markup() {
    let catalog = common::catalog();
    let text = render(&catalog, "", ReferenceFormat::Html).unwrap();
    assert!(text.starts_with("<!DOCTYPE html>"));
    assert!(text.ends_with("</html>"));
    assert_eq!(
        text.matches("<section ").count(),
        text.matches("</section>").count()
    );
    assert!(text.contains("|&gt; "));
    assert!(!text.contains("|> "));
}

#[test]
fn reference_of_a_page_command_carries_examples_and_errors() {
    let catalog = common::catalog();
    for format in [ReferenceFormat::Man, ReferenceFormat::Html] {
        let text = render(&catalog, "evaluate", format).unwrap();
        let page = catalog.docs().page("evaluate").unwrap();
        assert!(!page.examples.is_empty());
        assert!(text.contains("Examples"), "{format:?}");
        for note in &page.errors {
            assert!(text.contains(&note.code), "{format:?}: {}", note.code);
        }
    }
}

#[test]
fn unknown_command_lists_the_nearest_names() {
    let catalog = common::catalog();
    for format in ReferenceFormat::ALL {
        let error = render(&catalog, "remvoe", format).unwrap_err();
        assert_eq!(error.code(), ErrorCode::UnknownName);
    }
}

#[test]
fn generation_is_deterministic() {
    let catalog = common::catalog();
    for format in ReferenceFormat::ALL {
        assert_eq!(
            render(&catalog, "", format).unwrap(),
            render(&catalog, "", format).unwrap()
        );
    }
}
