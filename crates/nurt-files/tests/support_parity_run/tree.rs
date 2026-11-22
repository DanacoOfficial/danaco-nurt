// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! File trees of the parity scenarios on the virtual file system.
//!
//! The module writes `fs_before` (fixtures, literal content in an encoding,
//! generated content, links, modes and modification times), takes snapshots
//! of the tree below the root and checks the `fs_after` assertions.

use std::collections::BTreeMap;
use std::time::Duration;

use nurt_contracts::env::{Clock, WallTime};
use nurt_contracts::fs::{AbsPath, FileKind, Follow, Fs};
use nurt_testkit::ctx::TestContext;
use nurt_testkit::vfs::test_path;
use serde_json::json;

use crate::support_parity::{AfterEntry, FsEntry, Model};

/// Root directory of every scenario.
pub(crate) const ROOT: &str = "/work";

/// Interval between two entries of `fs_before`, so that modification times
/// follow their order; the files of one fixture share one modification time.
const WRITE_INTERVAL: Duration = Duration::from_secs(1);

const PERMISSION_BITS: u32 = 0o7777;
const NANOS_PER_SECOND: i128 = 1_000_000_000;
const SECONDS_PER_DAY: i64 = 86_400;

/// Absolute path of the root.
///
/// # Errors
///
/// Returns the message of the virtual file system.
pub(crate) fn root() -> Result<AbsPath, String> {
    test_path(ROOT).map_err(|e| e.to_string())
}

/// Absolute path of `relative` below the root.
///
/// # Errors
///
/// Returns the message of the virtual file system.
pub(crate) fn abs(relative: &str) -> Result<AbsPath, String> {
    test_path(&format!("{ROOT}/{relative}")).map_err(|e| e.to_string())
}

/// Entry of a snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Node {
    /// Directory with its permission bits.
    Dir(u32),
    /// File with its content and permission bits.
    File(Vec<u8>, u32),
    /// Symbolic link with its target.
    Link(String),
}

/// Tree below the root by relative path.
pub(crate) type Snapshot = BTreeMap<String, Node>;

/// Encodes `text` in the named encoding; UTF-8 when `None`.
///
/// # Errors
///
/// Returns a message for an unknown encoding or an unrepresentable
/// character.
pub(crate) fn encode(text: &str, encoding: Option<&str>) -> Result<Vec<u8>, String> {
    let Some(label) = encoding else {
        return Ok(text.as_bytes().to_vec());
    };
    match label.to_ascii_lowercase().as_str() {
        "utf-8" | "utf8" => Ok(text.as_bytes().to_vec()),
        "utf-16le" => Ok(text.encode_utf16().flat_map(u16::to_le_bytes).collect()),
        "utf-16be" => Ok(text.encode_utf16().flat_map(u16::to_be_bytes).collect()),
        other => {
            let codec = encoding_rs::Encoding::for_label(other.as_bytes())
                .or_else(|| encoding_rs::Encoding::for_label(other.replace('-', "").as_bytes()))
                .ok_or_else(|| format!("unknown encoding {other}"))?;
            let (bytes, _, unmappable) = codec.encode(text);
            if unmappable {
                return Err(format!("text is not representable in {other}"));
            }
            Ok(bytes.into_owned())
        }
    }
}

/// Content produced by a generator directive (see `fixtures.toml`).
///
/// # Errors
///
/// Returns a message for an invalid directive.
pub(crate) fn generate(directive: &str) -> Result<Vec<u8>, String> {
    let invalid = || format!("invalid generator {directive:?}");
    let (kind, argument) = directive.split_once(':').ok_or_else(invalid)?;
    match kind {
        "big_source" => argument
            .parse()
            .map(|count: u32| big_source(count).into_bytes())
            .map_err(|_| invalid()),
        "repeat" => {
            let (text, count) = argument.rsplit_once('*').ok_or_else(invalid)?;
            let count: usize = count.parse().map_err(|_| invalid())?;
            Ok(text.repeat(count).into_bytes())
        }
        "bytes" => hex(argument).ok_or_else(invalid),
        "png" => size(argument).map(|(w, h)| png(w, h)).ok_or_else(invalid),
        "webp_image" => size(argument)
            .map(|(w, h)| webp_image(w, h))
            .ok_or_else(invalid),
        "webp" => {
            let (variant, dimensions) = argument.split_once(',').ok_or_else(invalid)?;
            let (width, height) = size(dimensions).ok_or_else(invalid)?;
            webp(variant, width, height).ok_or_else(invalid)
        }
        "pdf" => Ok(pdf(argument)),
        "notebook" => notebook(argument).ok_or_else(invalid),
        _ => Err(invalid()),
    }
}

fn size(text: &str) -> Option<(u32, u32)> {
    let (width, height) = text.split_once('x')?;
    Some((width.parse().ok()?, height.parse().ok()?))
}

fn hex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(text.get(at..at + 2)?, 16).ok())
        .collect()
}

/// TypeScript module of `count` functions of five lines each, after a
/// one-line header; function `i` is named `step{i}` and every function has
/// the same second line.
pub(crate) fn big_source(count: u32) -> String {
    let mut text = String::from("// Module with numbered steps.\n");
    for index in 0..count {
        text.push_str(&format!(
            "export function step{index}(value: number): number {{\n  const doubled = value * 2;\n  return doubled + {index};\n}}\n\n"
        ));
    }
    text
}

/// Decodable PNG image of the given size.
pub(crate) fn png(width: u32, height: u32) -> Vec<u8> {
    let mut out = Vec::new();
    let encoded = image::DynamicImage::new_rgb8(width, height)
        .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png);
    if encoded.is_err() {
        out.clear();
    }
    out
}

/// Decodable lossless WebP image of the given size.
pub(crate) fn webp_image(width: u32, height: u32) -> Vec<u8> {
    let mut out = Vec::new();
    let encoded = image::DynamicImage::new_rgb8(width, height).write_to(
        &mut std::io::Cursor::new(&mut out),
        image::ImageFormat::WebP,
    );
    if encoded.is_err() {
        out.clear();
    }
    out
}

/// WebP container with a `VP8 `, `VP8L` or `VP8X` chunk of the given size.
pub(crate) fn webp(variant: &str, width: u32, height: u32) -> Option<Vec<u8>> {
    const VP8_START_CODE: [u8; 3] = [0x9D, 0x01, 0x2A];
    const VP8_KEY_FRAME_TAG: [u8; 3] = [0x10, 0x00, 0x00];
    const VP8L_SIGNATURE: u8 = 0x2F;
    const VP8L_BITS: u32 = 14;
    let (w, h) = (width.checked_sub(1)?, height.checked_sub(1)?);
    let (name, mut data): (&[u8; 4], Vec<u8>) = match variant {
        "VP8X" => {
            let mut data = vec![0; 4];
            data.extend_from_slice(&w.to_le_bytes()[..3]);
            data.extend_from_slice(&h.to_le_bytes()[..3]);
            (b"VP8X", data)
        }
        "VP8L" => {
            let bits = w | (h << VP8L_BITS);
            let mut data = vec![VP8L_SIGNATURE];
            data.extend_from_slice(&bits.to_le_bytes());
            (b"VP8L", data)
        }
        "VP8" => {
            let mut data = VP8_KEY_FRAME_TAG.to_vec();
            data.extend_from_slice(&VP8_START_CODE);
            data.extend_from_slice(&u16::try_from(width).ok()?.to_le_bytes());
            data.extend_from_slice(&u16::try_from(height).ok()?.to_le_bytes());
            (b"VP8 ", data)
        }
        _ => return None,
    };
    let length = u32::try_from(data.len()).ok()?;
    if data.len() % 2 == 1 {
        data.push(0);
    }
    let mut chunk = name.to_vec();
    chunk.extend_from_slice(&length.to_le_bytes());
    chunk.extend_from_slice(&data);
    let mut out = b"RIFF".to_vec();
    out.extend_from_slice(&u32::try_from(chunk.len() + 4).ok()?.to_le_bytes());
    out.extend_from_slice(b"WEBP");
    out.extend_from_slice(&chunk);
    Some(out)
}

/// One-page PDF that shows `text` in a standard font.
pub(crate) fn pdf(text: &str) -> Vec<u8> {
    let escaped: String = text
        .chars()
        .flat_map(|c| match c {
            '(' | ')' | '\\' => vec!['\\', c],
            other => vec![other],
        })
        .collect();
    let stream = format!("BT /F1 12 Tf 72 720 Td ({escaped}) Tj ET");
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>".to_owned(),
        format!("<< /Length {} >>\nstream\n{stream}\nendstream", stream.len()),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_owned(),
    ];
    let mut out = String::from("%PDF-1.4\n");
    let mut offsets = Vec::new();
    for (index, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.push_str(&format!("{} 0 obj\n{body}\nendobj\n", index + 1));
    }
    let xref = out.len();
    out.push_str(&format!(
        "xref\n0 {}\n0000000000 65535 f \n",
        objects.len() + 1
    ));
    for offset in offsets {
        out.push_str(&format!("{offset:010} 00000 n \n"));
    }
    out.push_str(&format!(
        "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
        objects.len() + 1
    ));
    out.into_bytes()
}

fn markdown(id: &str, source: &str) -> serde_json::Value {
    json!({ "cell_type": "markdown", "id": id, "metadata": {}, "source": [source] })
}

fn code(id: &str, source: &str, output: Option<&str>) -> serde_json::Value {
    let outputs: Vec<serde_json::Value> = output
        .map(|text| json!({ "name": "stdout", "output_type": "stream", "text": [text] }))
        .into_iter()
        .collect();
    json!({
        "cell_type": "code",
        "execution_count": if outputs.is_empty() { serde_json::Value::Null } else { json!(1) },
        "id": id,
        "metadata": {},
        "outputs": outputs,
        "source": [source]
    })
}

/// Jupyter notebook of the named kind (see `fixtures.toml`).
pub(crate) fn notebook(kind: &str) -> Option<Vec<u8>> {
    let cells4 = || {
        vec![
            markdown("intro", "# Analysis"),
            code("load", "data = load()", Some("wczytano\n")),
            code("use", "print(len(data))", None),
            markdown("tail", "End of the analysis."),
        ]
    };
    let document = |cells: Vec<serde_json::Value>, minor: u32| {
        json!({
            "cells": cells,
            "metadata": { "kernelspec": { "display_name": "Python 3", "language": "python", "name": "python3" } },
            "nbformat": 4,
            "nbformat_minor": minor
        })
    };
    const MINOR_WITH_IDS: u32 = 5;
    const MINOR_WITHOUT_IDS: u32 = 4;
    let value = match kind {
        "cells4" => document(cells4(), MINOR_WITH_IDS),
        "cells2" => document(cells4().into_iter().take(2).collect(), MINOR_WITH_IDS),
        "empty" => document(Vec::new(), MINOR_WITH_IDS),
        "flat" => {
            let cells = cells4()
                .into_iter()
                .map(|mut cell| {
                    let joined = cell["source"][0].clone();
                    cell["source"] = joined;
                    cell
                })
                .collect();
            document(cells, MINOR_WITH_IDS)
        }
        "missing_ids" => {
            let cells = cells4()
                .into_iter()
                .map(|mut cell| {
                    if let Some(object) = cell.as_object_mut() {
                        object.remove("id");
                    }
                    cell
                })
                .collect();
            document(cells, MINOR_WITHOUT_IDS)
        }
        "standard_crlf_bom" => {
            let text = serde_json::to_string_pretty(&document(cells4(), MINOR_WITH_IDS)).ok()?;
            let mut bytes = vec![0xEF, 0xBB, 0xBF];
            bytes.extend_from_slice(text.replace('\n', "\r\n").as_bytes());
            bytes.extend_from_slice(b"\r\n");
            return Some(bytes);
        }
        _ => return None,
    };
    let mut text = serde_json::to_string_pretty(&value).ok()?;
    text.push('\n');
    Some(text.into_bytes())
}

/// Content of the file at `path` of a fixture, or of a generator directive.
///
/// # Errors
///
/// Returns a message when neither a fixture file nor a directive matches.
pub(crate) fn reference_content(
    model: &Model,
    reference: &str,
    path: &str,
) -> Result<Vec<u8>, String> {
    if let Some(fixture) = model.fixtures.iter().find(|f| f.name == reference) {
        let file = fixture
            .file
            .iter()
            .find(|f| f.path == path)
            .ok_or_else(|| format!("fixture {reference} has no file {path}"))?;
        return match (&file.content, &file.generate) {
            (Some(content), _) => Ok(content.as_bytes().to_vec()),
            (None, Some(directive)) => generate(directive),
            (None, None) => Err(format!("fixture {reference}: {path} has no content")),
        };
    }
    generate(reference)
}

fn write(test: &TestContext, relative: &str, bytes: &[u8]) -> Result<(), String> {
    let path = abs(relative)?;
    if let Some(parent) = path.parent() {
        test.fs()
            .create_dir_all(&parent)
            .map_err(|e| e.to_string())?;
    }
    test.fs()
        .write_file(&path, bytes)
        .map_err(|e| e.to_string())
}

/// Parses `YYYY-MM-DDTHH:MM:SSZ`.
pub(crate) fn parse_utc(text: &str) -> Option<WallTime> {
    let (date, time) = text.strip_suffix('Z')?.split_once('T')?;
    let mut date_parts = date.split('-').map(str::parse::<i64>);
    let (year, month, day) = (
        date_parts.next()?.ok()?,
        date_parts.next()?.ok()?,
        date_parts.next()?.ok()?,
    );
    let mut time_parts = time.split(':').map(str::parse::<i64>);
    let (hour, minute, second) = (
        time_parts.next()?.ok()?,
        time_parts.next()?.ok()?,
        time_parts.next()?.ok()?,
    );
    let seconds =
        days_from_civil(year, month, day) * SECONDS_PER_DAY + hour * 3600 + minute * 60 + second;
    Some(WallTime::from_unix_nanos(
        i128::from(seconds) * NANOS_PER_SECOND,
    ))
}

/// Days since 1970-01-01 of a proleptic Gregorian date.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let shifted_month = (month + 9) % 12;
    let day_of_year = (153 * shifted_month + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

fn write_dated(
    test: &TestContext,
    relative: &str,
    bytes: &[u8],
    modified: Option<&String>,
) -> Result<(), String> {
    let Some(stamp) = modified else {
        return write(test, relative, bytes);
    };
    let wall = parse_utc(stamp).ok_or_else(|| format!("invalid time {stamp}"))?;
    let saved = test.clock().now();
    test.clock().set_wall(wall);
    let written = write(test, relative, bytes);
    test.clock().set_wall(saved);
    written
}

/// Writes the tree `entries` below the root.
///
/// # Errors
///
/// Returns a message for an unknown fixture, an invalid directive or a
/// failure of the virtual file system.
pub(crate) fn populate(
    test: &TestContext,
    model: &Model,
    entries: &[FsEntry],
) -> Result<(), String> {
    test.fs()
        .create_dir_all(&root()?)
        .map_err(|e| e.to_string())?;
    for entry in entries {
        test.clock().advance(WRITE_INTERVAL);
        if let Some(name) = &entry.fixture {
            let fixture = model
                .fixtures
                .iter()
                .find(|f| &f.name == name)
                .ok_or_else(|| format!("unknown fixture {name}"))?;
            for file in &fixture.file {
                let bytes = reference_content(model, name, &file.path)?;
                let modified = entry
                    .modified
                    .as_ref()
                    .and_then(|times| times.get(&file.path));
                write_dated(test, &file.path, &bytes, modified)?;
            }
            continue;
        }
        let relative = entry
            .path
            .as_deref()
            .ok_or("fs_before entry without path")?;
        if let Some(target) = &entry.symlink_to {
            let link = abs(relative)?;
            if let Some(parent) = link.parent() {
                test.fs()
                    .create_dir_all(&parent)
                    .map_err(|e| e.to_string())?;
            }
            test.fs()
                .symlink(target.as_str(), &link)
                .map_err(|e| e.to_string())?;
            continue;
        }
        let bytes = match (&entry.content, &entry.generate) {
            (Some(content), _) => encode(content, entry.encoding.as_deref())?,
            (None, Some(directive)) => generate(directive)?,
            (None, None) => return Err(format!("{relative} has no content")),
        };
        write(test, relative, &bytes)?;
        if let Some(mode) = &entry.mode {
            let bits = u32::from_str_radix(mode, 8).map_err(|e| e.to_string())?;
            test.fs()
                .set_mode(&abs(relative)?, bits)
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// Snapshot of the tree below the root.
///
/// # Errors
///
/// Returns the message of the first failing file system call.
pub(crate) fn snapshot(test: &TestContext) -> Result<Snapshot, String> {
    let mut tree = Snapshot::new();
    let fs: &dyn Fs = test.fs().as_ref();
    let mut pending = vec![(root()?, String::new())];
    while let Some((dir, prefix)) = pending.pop() {
        let entries = fs.read_dir(&dir).map_err(|e| e.to_string())?;
        for entry in entries {
            let entry = entry.map_err(|e| e.to_string())?;
            let name = entry.name.to_string_lossy().into_owned();
            let relative = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{prefix}/{name}")
            };
            let path = dir.join_name(&name).map_err(|e| e.to_string())?;
            let meta = fs
                .metadata(&path, Follow::NoFollow)
                .map_err(|e| e.to_string())?;
            let mode = meta.mode & PERMISSION_BITS;
            let node = match meta.kind {
                FileKind::Directory => {
                    pending.push((path, relative.clone()));
                    Node::Dir(mode)
                }
                FileKind::Symlink => {
                    let target = fs.read_link(&path).map_err(|e| e.to_string())?;
                    Node::Link(target.to_string_lossy().replace('\\', "/"))
                }
                _ => Node::File(test.fs().read_file(&path).map_err(|e| e.to_string())?, mode),
            };
            tree.insert(relative, node);
        }
    }
    Ok(tree)
}

fn lossy(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// Describes the differences of two snapshots.
pub(crate) fn tree_differences(before: &Snapshot, after: &Snapshot) -> Vec<String> {
    let mut problems = Vec::new();
    for (path, node) in before {
        match after.get(path) {
            None => problems.push(format!("{path} was removed")),
            Some(other) if other != node => problems.push(format!("{path} was changed")),
            Some(_) => {}
        }
    }
    for path in after.keys().filter(|path| !before.contains_key(*path)) {
        problems.push(format!("{path} was created"));
    }
    problems
}

/// Checks one `fs_after` assertion.
///
/// # Errors
///
/// Returns the description of the difference.
pub(crate) fn check_after(
    model: &Model,
    entry: &AfterEntry,
    tree: &Snapshot,
) -> Result<(), String> {
    let path = &entry.path;
    let node = tree.get(path);
    if let Some(target) = &entry.symlink_to {
        return match node {
            Some(Node::Link(actual)) if actual == target => Ok(()),
            other => Err(format!(
                "{path}: expected a link to {target}, found {}",
                describe(other)
            )),
        };
    }
    let Some(Node::File(bytes, mode)) = node else {
        if entry.excludes.is_some() && node.is_none() {
            return Ok(());
        }
        return Err(format!("{path}: expected a file, found {}", describe(node)));
    };
    if let Some(expected) = &entry.mode {
        let bits = u32::from_str_radix(expected, 8).map_err(|e| e.to_string())?;
        if bits != *mode {
            return Err(format!("{path}: expected mode {expected}, found {mode:o}"));
        }
    }
    let expected_bytes = if let Some(content) = &entry.content {
        Some(encode(content, entry.encoding.as_deref())?)
    } else if let Some(reference) = &entry.same_as {
        Some(reference_content(model, reference, path)?)
    } else if let Some(reference) = &entry.replace_in {
        let original = lossy(&reference_content(model, reference, path)?);
        let from = entry.from.as_deref().unwrap_or_default();
        let to = entry.to.as_deref().unwrap_or_default();
        Some(original.replace(from, to).into_bytes())
    } else if let Some(directive) = &entry.generate {
        Some(generate(directive)?)
    } else {
        None
    };
    if let Some(expected) = expected_bytes {
        return if &expected == bytes {
            Ok(())
        } else {
            Err(format!(
                "{path}: expected {:?}, found {:?}",
                shorten(&lossy(&expected)),
                shorten(&lossy(bytes))
            ))
        };
    }
    let text = lossy(bytes);
    if let Some(fragment) = &entry.contains
        && !text.contains(fragment.as_str())
    {
        return Err(format!(
            "{path}: expected to contain {fragment:?}, found {:?}",
            shorten(&text)
        ));
    }
    if let Some(fragment) = &entry.excludes
        && text.contains(fragment.as_str())
    {
        return Err(format!("{path}: expected not to contain {fragment:?}"));
    }
    Ok(())
}

fn describe(node: Option<&Node>) -> String {
    match node {
        None => "nothing".to_owned(),
        Some(Node::Dir(_)) => "a directory".to_owned(),
        Some(Node::File(bytes, _)) => format!("a file of {} bytes", bytes.len()),
        Some(Node::Link(target)) => format!("a link to {target}"),
    }
}

/// Longest text quoted in a difference.
const QUOTE_LIMIT: usize = 160;

/// Text shortened for messages.
pub(crate) fn shorten(text: &str) -> String {
    if text.chars().count() <= QUOTE_LIMIT {
        text.to_owned()
    } else {
        let head: String = text.chars().take(QUOTE_LIMIT).collect();
        format!("{head}...")
    }
}
