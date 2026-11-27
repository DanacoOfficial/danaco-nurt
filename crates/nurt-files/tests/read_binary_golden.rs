// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Examples of the reference page of `read` for PDF documents, images,
//! hexadecimal dumps and compressed files, run on a virtual file system.

// Tests report failures by panicking.
#![allow(clippy::expect_used)]

mod read_binary_support;

use nurt_archive::codec::Algorithm;
use nurt_contracts::value::{Range, Value};
use nurt_testkit::ctx::TestContext;
use read_binary_support::{compress, file, pdf_of_lines, png, read};

fn range(start: i64, end: i64) -> Range {
    Range::new(Some(start), Some(end)).expect("start is not after end")
}

/// The first 48 bytes of a 64-bit ELF executable header.
fn elf_header() -> Vec<u8> {
    let mut bytes = b"\x7fELF\x02\x01\x01\x00".to_vec();
    bytes.extend_from_slice(&[0; 8]);
    bytes.extend_from_slice(b"\x03\x00\x3e\x00\x01\x00\x00\x00\x40\x10\x00\x00\x00\x00\x00\x00");
    bytes.extend_from_slice(&[0; 16]);
    bytes
}

#[test]
fn pages_of_a_pdf_document() {
    let test = TestContext::new();
    let path = file(
        &test,
        "/srv/docs/report.pdf",
        &pdf_of_lines(&[
            "Quarterly report",
            "Revenue grew in the third quarter.",
            "Costs fell by four percent.",
        ]),
    );
    let read = read(
        &test.ctx(),
        &path,
        vec![("pages", Value::Range(range(2, 3)))],
    )
    .expect("the document is readable");
    let pages: Vec<(Option<u64>, Option<u64>, &str)> = read
        .iter()
        .map(|content| (content.page, content.pages_total, content.text.as_str()))
        .collect();
    assert_eq!(
        pages,
        [
            (Some(2), Some(3), "Revenue grew in the third quarter."),
            (Some(3), Some(3), "Costs fell by four percent."),
        ]
    );
}

#[test]
fn hexadecimal_dump_of_the_first_rows() {
    let test = TestContext::new();
    let path = file(&test, "/srv/app/bin/app", &elf_header());
    let read = read(
        &test.ctx(),
        &path,
        vec![
            ("view", Value::Enum("hex".to_owned())),
            ("lines", Value::Range(range(1, 2))),
        ],
    )
    .expect("the file is readable");
    assert_eq!(
        read[0].text,
        "00000000: 7f45 4c46 0201 0100 0000 0000 0000 0000  .ELF............\n00000010: 0300 3e00 0100 0000 4010 0000 0000 0000  ..>.....@.......\n"
    );
    assert_eq!(read[0].range, Some(range(1, 2)));
    assert_eq!(read[0].total_lines, Some(3));
}

#[test]
fn last_line_of_a_compressed_log() {
    let test = TestContext::new();
    let path = file(
        &test,
        "/var/log/app.log.gz",
        &compress(Algorithm::Gzip, b"start\nrequest 1\nstop\n"),
    );
    let read =
        read(&test.ctx(), &path, vec![("last", Value::Int(1))]).expect("the log is readable");
    assert_eq!(read[0].text, "stop\n");
    assert_eq!(
        read[0]
            .decompressed_from
            .as_ref()
            .map(|from| from.as_str().to_owned()),
        Some("/var/log/app.log.gz".to_owned())
    );
}

#[test]
fn header_of_an_image() {
    let test = TestContext::new();
    let path = file(&test, "/srv/site/logo.png", &png(64, 32));
    let read = read(
        &test.ctx(),
        &path,
        vec![("view", Value::Enum("header".to_owned()))],
    )
    .expect("the image is readable");
    let image = read[0].image.clone().expect("an image record");
    assert_eq!(image.mime, "image/png");
    assert_eq!((image.width, image.height), (Some(64), Some(32)));
    assert!(image.data.is_none());
}
