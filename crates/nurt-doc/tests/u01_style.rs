// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-doc/tests/u01_style.rs
// Responsibility:   Detection of indentation, line endings, separators, quotes, trailing commas and alignment.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Detection of indentation, line endings, separators, quotes, trailing
//! commas and alignment.

use nurt_doc::model::Span;
use nurt_doc::style::{Indent, LineEnding, Quote, StyleProbe, UTF8_BOM};

#[test]
fn indentation_unit_is_the_most_frequent_step() {
    let yaml = b"a:\n  b:\n    c: 1\n  d: 2\n";
    assert_eq!(StyleProbe::new(yaml).indent_unit(), Some(Indent::Spaces(2)));
    let json = b"{\n    \"a\": {\n        \"b\": 1\n    }\n}\n";
    assert_eq!(StyleProbe::new(json).indent_unit(), Some(Indent::Spaces(4)));
    let tabs = b"[a]\n\tb = 1\n\tc = 2\n";
    assert_eq!(StyleProbe::new(tabs).indent_unit(), Some(Indent::Tab));
    assert_eq!(StyleProbe::new(b"a = 1\nb = 2\n").indent_unit(), None);
}

#[test]
fn blank_lines_do_not_change_indentation() {
    let text = b"a:\n\n  b: 1\n   \n  c: 2\n";
    assert_eq!(StyleProbe::new(text).indent_unit(), Some(Indent::Spaces(2)));
}

#[test]
fn dominant_line_ending_wins_and_mixing_is_reported() {
    let crlf = StyleProbe::new(b"a\r\nb\r\nc\n");
    assert_eq!(crlf.line_ending(), LineEnding::CrLf);
    assert!(crlf.mixed_line_endings());
    let lf = StyleProbe::new(b"a\nb\n");
    assert_eq!(lf.line_ending(), LineEnding::Lf);
    assert!(!lf.mixed_line_endings());
    assert_eq!(StyleProbe::new(b"a").line_ending(), LineEnding::Lf);
    assert_eq!(LineEnding::CrLf.as_str(), "\r\n");
}

#[test]
fn final_newline_and_byte_order_mark_are_detected() {
    let probe = StyleProbe::new(b"\xEF\xBB\xBFa = 1");
    assert!(probe.bom());
    assert!(!probe.final_newline());
    assert!(StyleProbe::new(b"a\n").final_newline());
    assert_eq!(UTF8_BOM.len(), 3);
}

#[test]
fn line_at_excludes_line_ending() {
    let text = b"first\r\n  second\r\nthird";
    let probe = StyleProbe::new(text);
    assert_eq!(probe.line_at(0), Span::new(0, 5));
    assert_eq!(probe.line_at(10), Span::new(7, 15));
    assert_eq!(probe.line_at(100), Span::new(17, 22));
    assert_eq!(probe.indent_at(10), b"  ");
    assert_eq!(probe.indent_at(0), b"");
}

#[test]
fn separator_spacing_follows_most_lines() {
    assert_eq!(
        StyleProbe::new(b"a = 1\nb = 2\nc=3\n")
            .separator(b'=')
            .before,
        " "
    );
    let colon = StyleProbe::new(b"a: 1\nb: 2\n").separator(b':');
    assert_eq!((colon.before.as_str(), colon.after.as_str()), ("", " "));
    let tight = StyleProbe::new(b"a=1\nb=2\n").separator(b'=');
    assert_eq!((tight.before.as_str(), tight.after.as_str()), ("", ""));
}

#[test]
fn separator_ignores_quotes_and_lines_without_value() {
    let quoted = StyleProbe::new(b"\"x:y\" :  1\nparent:\n  \"a\\\"b:c\" :  2\n").separator(b':');
    assert_eq!((quoted.before.as_str(), quoted.after.as_str()), (" ", "  "));
    let none = StyleProbe::new(b"no separators here\n").separator(b'=');
    assert_eq!((none.before.as_str(), none.after.as_str()), ("", " "));
}

#[test]
fn preferred_quote_opens_most_quoted_lines() {
    assert_eq!(
        StyleProbe::new(b"a: 'x'\nb: 'y'\nc: \"z\"\n").quote(),
        Some(Quote::Single)
    );
    assert_eq!(
        StyleProbe::new(b"a: \"x\"\nb: 'y'\n").quote(),
        Some(Quote::Double)
    );
    assert_eq!(StyleProbe::new(b"a: x\n").quote(), None);
    assert_eq!(Quote::Single.as_char(), '\'');
}

#[test]
fn trailing_comma_before_closing_bracket_is_detected() {
    assert!(StyleProbe::new(b"[1, 2,\n]").trailing_comma());
    assert!(StyleProbe::new(b"{\"a\": 1,\n  }").trailing_comma());
    assert!(!StyleProbe::new(b"[1, 2]").trailing_comma());
    assert!(!StyleProbe::new(b"a,").trailing_comma());
}

#[test]
fn alignment_of_separators_or_values_is_detected() {
    assert!(StyleProbe::new(b"a    = 1\nlong = 2\n").aligned(b'='));
    assert!(!StyleProbe::new(b"a = 1\nlong = 2\n").aligned(b'='));
    assert!(StyleProbe::new(b"a:    1\nlong: 2\n").aligned(b':'));
    assert!(!StyleProbe::new(b"a = 1\n\nlong = 2\n").aligned(b'='));
}

#[test]
fn indentation_repeats_by_level() {
    assert_eq!(Indent::Spaces(4).repeat(2), "        ");
    assert_eq!(Indent::Tab.repeat(3), "\t\t\t");
    assert_eq!(Indent::Spaces(2).repeat(0), "");
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
