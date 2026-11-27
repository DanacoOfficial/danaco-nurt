// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-files/tests/read_binary_support/mod.rs
// Responsibility:   Helpers of the tests of `read` on content that is not text.
// Dependencies:     image, nurt_archive, nurt_contracts, nurt_testkit, pdf_extract
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Helpers of the tests of `read` on content that is not text: PDF
//! documents, images and compressed files generated in the test and stored
//! on a virtual file system.

// Each test crate uses a part of the helpers.
#![allow(dead_code)]

use std::io::{Cursor, Write};
use std::sync::Arc;

use image::{DynamicImage, ImageFormat};
use nurt_archive::codec::{self, Algorithm};
use nurt_contracts::error::Result;
use nurt_contracts::fs::AbsPath;
use nurt_contracts::record::Content;
use nurt_contracts::stage::{
    ArgValue, BoundArgs, ExecCtx, RecordStream, StageData, StageImpl, StageInput,
};
use nurt_contracts::value::{Record, RecordKind, Value};
use nurt_testkit::ctx::TestContext;
use nurt_testkit::vfs::test_path;
use pdf_extract::content::{Content as PageContent, Operation};
use pdf_extract::{
    Dictionary, Document, EncryptionState, EncryptionVersion, Object, Permissions, Stream,
    StringFormat,
};

/// Polish letters that the test font maps to the codes from 128 on, by
/// their PostScript glyph names.
const POLISH_GLYPHS: [(char, &str); 16] = [
    ('\u{105}', "aogonek"),
    ('\u{107}', "cacute"),
    ('\u{119}', "eogonek"),
    ('\u{142}', "lslash"),
    ('\u{144}', "nacute"),
    ('\u{15b}', "sacute"),
    ('\u{17a}', "zacute"),
    ('\u{17c}', "zdotaccent"),
    ('\u{104}', "Aogonek"),
    ('\u{106}', "Cacute"),
    ('\u{118}', "Eogonek"),
    ('\u{141}', "Lslash"),
    ('\u{143}', "Nacute"),
    ('\u{15a}', "Sacute"),
    ('\u{179}', "Zacute"),
    ('\u{17b}', "Zdotaccent"),
];
const FIRST_POLISH_CODE: u8 = 128;

/// Font size of the generated text, in points.
const FONT_SIZE: i64 = 12;

/// Position of a single line of text near the top left corner of an A4
/// page, in points.
const LINE_X: i64 = 72;
const LINE_Y: i64 = 760;

/// Text placed at a position in points from the lower left corner.
pub(crate) struct Run<'a> {
    pub(crate) x: i64,
    pub(crate) y: i64,
    pub(crate) text: &'a str,
}

fn name(value: &str) -> Object {
    Object::Name(value.as_bytes().to_vec())
}

/// Helvetica with `WinAnsiEncoding`, in which U+00F3 and U+00D3 already
/// exist, and the other Polish letters added through `Differences`.
fn font() -> Dictionary {
    let mut differences = vec![Object::Integer(i64::from(FIRST_POLISH_CODE))];
    differences.extend(POLISH_GLYPHS.iter().map(|(_, glyph)| name(glyph)));
    let mut encoding = Dictionary::new();
    encoding.set("Type", name("Encoding"));
    encoding.set("BaseEncoding", name("WinAnsiEncoding"));
    encoding.set("Differences", Object::Array(differences));
    let mut font = Dictionary::new();
    font.set("Type", name("Font"));
    font.set("Subtype", name("Type1"));
    font.set("BaseFont", name("Helvetica"));
    font.set("Encoding", Object::Dictionary(encoding));
    font
}

fn encode(text: &str) -> Vec<u8> {
    text.chars()
        .map(|ch| match ch {
            '\u{f3}' => 0xf3,
            '\u{d3}' => 0xd3,
            ch if ch.is_ascii() => ch as u8,
            ch => {
                let index = POLISH_GLYPHS
                    .iter()
                    .position(|(letter, _)| *letter == ch)
                    .expect("the test font has the letter");
                FIRST_POLISH_CODE + u8::try_from(index).expect("16 glyphs")
            }
        })
        .collect()
}

fn page_content(runs: &[Run<'_>]) -> Vec<u8> {
    let mut operations = Vec::new();
    for run in runs {
        operations.push(Operation::new("BT", vec![]));
        operations.push(Operation::new(
            "Tf",
            vec![name("F1"), Object::Integer(FONT_SIZE)],
        ));
        operations.push(Operation::new(
            "Td",
            vec![Object::Integer(run.x), Object::Integer(run.y)],
        ));
        operations.push(Operation::new(
            "Tj",
            vec![Object::String(encode(run.text), StringFormat::Literal)],
        ));
        operations.push(Operation::new("ET", vec![]));
    }
    PageContent { operations }
        .encode()
        .expect("the operations encode")
}

/// Document with the given pages and an unreferenced stream of every byte
/// value, as real documents carry compressed binary streams.
pub(crate) fn pdf_document(pages: &[&[Run<'_>]]) -> Document {
    let mut document = Document::with_version("1.5");
    let pages_id = document.new_object_id();
    let font_id = document.add_object(font());
    let mut fonts = Dictionary::new();
    fonts.set("F1", Object::Reference(font_id));
    let mut resources = Dictionary::new();
    resources.set("Font", Object::Dictionary(fonts));
    let mut kids = Vec::new();
    for runs in pages {
        let content_id = document.add_object(Stream::new(Dictionary::new(), page_content(runs)));
        let mut page = Dictionary::new();
        page.set("Type", name("Page"));
        page.set("Parent", Object::Reference(pages_id));
        page.set("Contents", Object::Reference(content_id));
        kids.push(Object::Reference(document.add_object(page)));
    }
    document.add_object(Stream::new(Dictionary::new(), (0..=u8::MAX).collect()));
    let mut tree = Dictionary::new();
    tree.set("Type", name("Pages"));
    tree.set(
        "Count",
        Object::Integer(i64::try_from(kids.len()).expect("few pages")),
    );
    tree.set("Kids", Object::Array(kids));
    tree.set("Resources", Object::Dictionary(resources));
    tree.set(
        "MediaBox",
        Object::Array([0, 0, 595, 842].map(Object::Integer).to_vec()),
    );
    document.objects.insert(pages_id, Object::Dictionary(tree));
    let mut catalog = Dictionary::new();
    catalog.set("Type", name("Catalog"));
    catalog.set("Pages", Object::Reference(pages_id));
    let catalog_id = document.add_object(catalog);
    document.trailer.set("Root", Object::Reference(catalog_id));
    document.trailer.set(
        "ID",
        Object::Array(vec![
            Object::String(vec![7; 16], StringFormat::Hexadecimal),
            Object::String(vec![7; 16], StringFormat::Hexadecimal),
        ]),
    );
    document
}

pub(crate) fn save(mut document: Document) -> Vec<u8> {
    let mut bytes = Vec::new();
    document.save_to(&mut bytes).expect("the document saves");
    bytes
}

pub(crate) fn pdf(pages: &[&[Run<'_>]]) -> Vec<u8> {
    save(pdf_document(pages))
}

/// Document with one line of text on each page.
pub(crate) fn pdf_of_lines(lines: &[&str]) -> Vec<u8> {
    let runs: Vec<[Run<'_>; 1]> = lines
        .iter()
        .map(|text| {
            [Run {
                x: LINE_X,
                y: LINE_Y,
                text,
            }]
        })
        .collect();
    let pages: Vec<&[Run<'_>]> = runs.iter().map(|page| &page[..]).collect();
    pdf(&pages)
}

/// Document that opens only with the user password `user`.
pub(crate) fn encrypted_pdf() -> Vec<u8> {
    let mut document = pdf_document(&[&[Run {
        x: LINE_X,
        y: LINE_Y,
        text: "secret",
    }]]);
    let state = EncryptionState::try_from(EncryptionVersion::V2 {
        document: &document,
        owner_password: "permissions",
        user_password: "user",
        key_length: 128,
        permissions: Permissions::all(),
    })
    .expect("RC4 encryption state");
    document.encrypt(&state).expect("the document encrypts");
    save(document)
}

pub(crate) fn png(width: u32, height: u32) -> Vec<u8> {
    let mut bytes = Vec::new();
    DynamicImage::new_rgb8(width, height)
        .write_to(&mut Cursor::new(&mut bytes), ImageFormat::Png)
        .expect("the image encodes");
    bytes
}

pub(crate) fn compress(algorithm: Algorithm, data: &[u8]) -> Vec<u8> {
    let mut encoder = codec::encoder(algorithm, None, Vec::new()).expect("the encoder starts");
    encoder.write_all(data).expect("the encoder accepts data");
    encoder.finish().expect("the encoder finishes")
}

pub(crate) fn file(test: &TestContext, path: &str, content: &[u8]) -> AbsPath {
    let path = test_path(path).expect("test paths are absolute");
    test.fs()
        .write_file(&path, content)
        .expect("the virtual file system accepts the file");
    path
}

fn read_stage() -> Arc<dyn StageImpl> {
    nurt_files::stages()
        .into_iter()
        .find(|stage| stage.descriptor().command == "read")
        .expect("read is registered")
}

fn with_params(args: BoundArgs, params: Vec<(&str, Value)>) -> BoundArgs {
    params.into_iter().fold(args, |args, (name, value)| {
        args.with(name, ArgValue::Value(value), false)
    })
}

fn execute(ctx: &ExecCtx, args: &BoundArgs, input: StageInput<'_>) -> Result<Vec<Content>> {
    let stage = read_stage();
    let output = stage
        .as_read()
        .expect("read is a read stage")
        .execute(args, input, ctx)?;
    match output.data {
        StageData::Records(stream) => stream
            .map(|record| record.map(|record| Content::try_from(record).expect("content")))
            .collect(),
        _ => Ok(Vec::new()),
    }
}

/// Records of `read` in the source form for one file.
pub(crate) fn read(
    ctx: &ExecCtx,
    path: &AbsPath,
    params: Vec<(&str, Value)>,
) -> Result<Vec<Content>> {
    let args = with_params(
        BoundArgs::new("read").with(
            "path",
            ArgValue::Value(Value::Path(path.to_language_string())),
            false,
        ),
        params,
    );
    execute(ctx, &args, StageInput::none())
}

/// Records of `read` in the stage form for `entry` records of files.
pub(crate) fn read_entries(
    ctx: &ExecCtx,
    paths: &[&AbsPath],
    params: Vec<(&str, Value)>,
) -> Result<Vec<Content>> {
    let entries: Vec<Record> = paths
        .iter()
        .map(|path| {
            Record::new(RecordKind::Entry)
                .with("path", Value::Path(path.to_language_string()))
                .with("type", Value::Enum("file".to_owned()))
        })
        .collect();
    let args = with_params(BoundArgs::new("read"), params);
    execute(
        ctx,
        &args,
        StageInput::records(RecordStream::from_records(entries)),
    )
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
