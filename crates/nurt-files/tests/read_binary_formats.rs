// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-files/tests/read_binary_formats.rs
// Responsibility:   `read` of PDF documents, images and compressed files: page selection.
// Dependencies:     nurt_archive, nurt_contracts, nurt_testkit, read_binary_support
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! `read` of PDF documents, images and compressed files: page selection,
//! extraction modes, damaged and protected documents and limits.

// Tests report failures by panicking.
#![allow(clippy::expect_used)]

mod read_binary_support;

use nurt_archive::codec::Algorithm;
use nurt_contracts::error::ErrorCode;
use nurt_contracts::stage::Limits;
use nurt_contracts::value::{Range, Value};
use nurt_testkit::ctx::TestContext;
use read_binary_support::{
    Run, compress, encrypted_pdf, file, pdf, pdf_of_lines, png, read, read_entries,
};

/// Polish pangram with all nine Polish lowercase letters.
const PANGRAM: &str = "Za\u{17c}\u{f3}\u{142}\u{107} g\u{119}\u{15b}l\u{105} ja\u{17a}\u{144}";
/// Polish words in capitals.
const CAPITALS: &str = "\u{141}\u{d3}D\u{179}, \u{15a}RODA";

fn three_pages() -> Vec<u8> {
    pdf_of_lines(&["First page", PANGRAM, CAPITALS])
}

fn range(start: i64, end: i64) -> Range {
    Range::new(Some(start), Some(end)).expect("start is not after end")
}

fn texts(read: &[nurt_contracts::record::Content]) -> Vec<(Option<u64>, &str)> {
    read.iter()
        .map(|content| (content.page, content.text.as_str()))
        .collect()
}

#[test]
fn every_page_is_a_record_with_polish_letters_kept() {
    let test = TestContext::new();
    let path = file(&test, "/d/a.pdf", &three_pages());
    let read = read(&test.ctx(), &path, Vec::new()).expect("readable");
    assert_eq!(
        texts(&read),
        [
            (Some(1), "First page"),
            (Some(2), PANGRAM),
            (Some(3), CAPITALS)
        ]
    );
    assert!(read.iter().all(|content| content.pages_total == Some(3)));
    assert!(read.iter().all(|content| content.encoding == "binary"));
    assert!(read[0].etag.contains("sha256:"));
    assert_eq!(read[1].lines, 1);
}

#[test]
fn page_ranges_are_read_in_the_order_given() {
    let test = TestContext::new();
    let path = file(&test, "/d/a.pdf", &three_pages());
    let read = read(
        &test.ctx(),
        &path,
        vec![(
            "pages",
            Value::List(vec![Value::Int(3), Value::Range(range(1, 9))]),
        )],
    )
    .expect("readable");
    let pages: Vec<Option<u64>> = read.iter().map(|content| content.page).collect();
    assert_eq!(pages, [Some(3), Some(1), Some(2), Some(3)]);
}

#[test]
fn page_outside_the_document_is_out_of_range() {
    let test = TestContext::new();
    let path = file(&test, "/d/a.pdf", &three_pages());
    let ctx = test.ctx();
    let error = read(&ctx, &path, vec![("pages", Value::Int(4))]).expect_err("page 4");
    assert_eq!(error.code(), ErrorCode::OutOfRange);
    assert!(
        error.message.contains("use pages 1 to 3"),
        "{}",
        error.message
    );
    let zero = read(&ctx, &path, vec![("pages", Value::Int(0))]).expect_err("page 0");
    assert_eq!(zero.code(), ErrorCode::InvalidValue);
}

#[test]
fn layout_mode_keeps_columns() {
    let test = TestContext::new();
    let bytes = pdf(&[&[
        Run {
            x: 72,
            y: 760,
            text: "Name",
        },
        Run {
            x: 300,
            y: 760,
            text: "Size",
        },
        Run {
            x: 72,
            y: 745,
            text: "a.txt",
        },
        Run {
            x: 300,
            y: 745,
            text: "12",
        },
    ]]);
    let path = file(&test, "/d/table.pdf", &bytes);
    let read = read(
        &test.ctx(),
        &path,
        vec![("mode", Value::Enum("layout".to_owned()))],
    )
    .expect("readable");
    let lines: Vec<&str> = read[0].text.lines().collect();
    assert_eq!(lines.len(), 2, "{:?}", read[0].text);
    assert_eq!(lines[0].find("Size"), lines[1].find("12"));
}

#[test]
fn header_view_gives_the_page_count_without_text() {
    let test = TestContext::new();
    let path = file(&test, "/d/a.pdf", &three_pages());
    let read = read(
        &test.ctx(),
        &path,
        vec![("view", Value::Enum("header".to_owned()))],
    )
    .expect("readable");
    assert_eq!(read.len(), 1);
    assert_eq!(read[0].pages_total, Some(3));
    assert_eq!(read[0].text, "");
    assert_eq!(read[0].view.as_deref(), Some("header"));
}

#[test]
fn protected_damaged_and_image_modes_fail_with_their_codes() {
    let test = TestContext::new();
    let ctx = test.ctx();
    let encrypted = file(&test, "/d/secret.pdf", &encrypted_pdf());
    assert_eq!(
        read(&ctx, &encrypted, Vec::new())
            .expect_err("password")
            .code(),
        ErrorCode::Encrypted
    );
    let mut damaged = b"%PDF-1.5\n".to_vec();
    damaged.extend_from_slice(&[0; 32]);
    let damaged = file(&test, "/d/damaged.pdf", &damaged);
    assert_eq!(
        read(&ctx, &damaged, Vec::new())
            .expect_err("damaged")
            .code(),
        ErrorCode::Format
    );
    let path = file(&test, "/d/a.pdf", &three_pages());
    assert_eq!(
        read(
            &ctx,
            &path,
            vec![("mode", Value::Enum("images".to_owned()))]
        )
        .expect_err("no PDFium")
        .code(),
        ErrorCode::ExternalMissing
    );
    assert_eq!(
        read(&ctx, &path, vec![("lines", Value::Int(1))])
            .expect_err("lines of a document")
            .code(),
        ErrorCode::Unsupported
    );
}

#[test]
fn document_above_the_text_limit_fails() {
    let test = TestContext::new();
    let path = file(&test, "/d/a.pdf", &three_pages());
    let ctx = test
        .builder()
        .limits(Limits {
            max_file_bytes_text: 64,
            ..Limits::default()
        })
        .build();
    assert_eq!(
        read(&ctx, &path, Vec::new()).expect_err("too large").code(),
        ErrorCode::Limit
    );
}

#[test]
fn compressed_documents_are_read_after_decompression() {
    let test = TestContext::new();
    let path = file(
        &test,
        "/d/a.pdf.xz",
        &compress(Algorithm::Xz, &three_pages()),
    );
    let read = read(&test.ctx(), &path, vec![("pages", Value::Int(2))]).expect("readable");
    assert_eq!(texts(&read), [(Some(2), PANGRAM)]);
    assert!(read[0].decompressed_from.is_some());
}

#[test]
fn entries_of_documents_and_images_are_read_in_the_stage_form() {
    let test = TestContext::new();
    let document = file(&test, "/d/a.pdf", &pdf_of_lines(&["only page"]));
    let image = file(&test, "/d/b.png", &png(5, 7));
    let read = read_entries(&test.ctx(), &[&document, &image], Vec::new()).expect("readable");
    assert_eq!(read.len(), 2);
    assert_eq!(read[0].text, "only page");
    assert_eq!(
        read[1]
            .image
            .as_ref()
            .map(|image| (image.width, image.height)),
        Some((Some(5), Some(7)))
    );
}

/// PDF document of one page showing `text` in Helvetica, written in ASCII
/// characters only, with an exact cross-reference table.
fn ascii_pdf(text: &str) -> Vec<u8> {
    let content = format!("BT /F1 12 Tf 72 760 Td ({text}) Tj ET");
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 595 842] >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>"
            .to_owned(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>"
            .to_owned(),
        format!(
            "<< /Length {} >>\nstream\n{content}\nendstream",
            content.len()
        ),
    ];
    let mut bytes = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (index, object) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", index + 1).as_bytes());
    }
    let xref = bytes.len();
    let size = objects.len() + 1;
    bytes.extend_from_slice(format!("xref\n0 {size}\n0000000000 65535 f \n").as_bytes());
    for offset in offsets {
        bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    bytes.extend_from_slice(
        format!("trailer\n<< /Size {size} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
    );
    bytes
}

#[test]
fn pdf_of_ascii_characters_only_is_read_as_a_document() {
    let test = TestContext::new();
    let bytes = ascii_pdf("Plain ASCII page");
    assert!(bytes.is_ascii());
    let path = file(&test, "/d/plain.pdf", &bytes);
    let ctx = test.ctx();
    let pages = read(&ctx, &path, Vec::new()).expect("readable");
    assert_eq!(texts(&pages), [(Some(1), "Plain ASCII page")]);
    assert_eq!(pages[0].pages_total, Some(1));
    assert_eq!(pages[0].encoding, "binary");
    let entries = read_entries(&ctx, &[&path], Vec::new()).expect("readable");
    assert_eq!(entries[0].page, Some(1));
    assert_eq!(
        read(&ctx, &path, vec![("lines", Value::Int(1))])
            .expect_err("lines of a document")
            .code(),
        ErrorCode::Unsupported
    );
    let source = read(
        &ctx,
        &path,
        vec![("encoding", Value::Text("utf-8".to_owned()))],
    )
    .expect("the source as text");
    assert!(source[0].text.starts_with("%PDF-1.4\n"));
    assert_eq!(source[0].page, None);
}

#[test]
fn text_that_starts_like_an_image_signature_stays_text() {
    let test = TestContext::new();
    let ctx = test.ctx();
    for (name, text) in [
        ("/d/cars.txt", "BMW and GIF notes\n"),
        ("/d/formats.txt", "GIF is an image format\n"),
    ] {
        let path = file(&test, name, text.as_bytes());
        let read = read(&ctx, &path, Vec::new()).expect("readable");
        assert_eq!(read[0].text, text);
        assert!(read[0].image.is_none());
        assert_ne!(read[0].encoding, "binary");
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
