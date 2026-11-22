// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Reports of `llvm-cov export`.

use std::path::Path;

use serde::Deserialize;

use super::{FileData, Tally, normalize_path};

#[derive(Deserialize)]
struct Export {
    data: Vec<Unit>,
}

#[derive(Deserialize)]
struct Unit {
    #[serde(default)]
    files: Vec<SourceFile>,
}

#[derive(Deserialize)]
struct SourceFile {
    filename: String,
    #[serde(default)]
    summary: Summary,
    #[serde(default)]
    segments: Vec<Segment>,
}

#[derive(Deserialize, Default)]
struct Summary {
    #[serde(default)]
    lines: Item,
    #[serde(default)]
    functions: Item,
    #[serde(default)]
    branches: Item,
}

#[derive(Deserialize, Default)]
struct Item {
    #[serde(default)]
    count: u64,
    #[serde(default)]
    covered: u64,
}

/// Position and count of a region boundary: line, column, count, whether
/// the segment has a count, whether it starts a region, whether it is a gap.
type Segment = (u64, u64, u64, bool, bool, bool);

/// Reads the files of an export. Uncovered lines come from the segments
/// when the export has them.
pub(crate) fn parse(text: &str, dir: &Path) -> Result<Vec<FileData>, String> {
    let export: Export = serde_json::from_str(text)
        .map_err(|failure| format!("the report is not a coverage export: {failure}"))?;
    let mut files = Vec::new();
    for unit in &export.data {
        for source in &unit.files {
            files.push(source_file(source, dir));
        }
    }
    Ok(files)
}

fn source_file(source: &SourceFile, dir: &Path) -> FileData {
    let mut file = FileData::new(normalize_path(&source.filename, dir));
    file.line_tally = Some(tally(&source.summary.lines));
    file.functions = tally(&source.summary.functions);
    file.branches = tally(&source.summary.branches);
    for (index, segment) in source.segments.iter().enumerate() {
        let (line, _, count, has_count, _, is_gap) = *segment;
        if !has_count || is_gap {
            continue;
        }
        let end = source
            .segments
            .get(index + 1)
            .map_or(line, |next| if next.1 > 1 { next.0 } else { next.0.saturating_sub(1).max(line) });
        for covered in line..=end {
            file.hit_line(covered, count);
        }
    }
    file
}

fn tally(item: &Item) -> Tally {
    Tally {
        found: item.count,
        hit: item.covered,
    }
}
