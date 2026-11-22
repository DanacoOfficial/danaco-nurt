// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! LCOV tracefiles.

use std::path::Path;

use super::{FileData, Tally, normalize_path};

/// Reads the records of a tracefile.
///
/// Per-line `DA` records give the lines; a record without them falls back
/// to its `LF` and `LH` totals.
pub(crate) fn parse(text: &str, dir: &Path) -> Result<Vec<FileData>, String> {
    let mut files = Vec::new();
    let mut current: Option<FileData> = None;
    let mut totals = Tally::default();
    let mut seen_record = false;
    for raw in text.lines() {
        let line = raw.trim();
        if let Some(path) = line.strip_prefix("SF:") {
            seen_record = true;
            current = Some(FileData::new(normalize_path(path, dir)));
            totals = Tally::default();
        } else if let Some(file) = current.as_mut() {
            if let Some(data) = line.strip_prefix("DA:") {
                let mut parts = data.split(',');
                let number = parts.next().and_then(|part| part.parse::<u64>().ok());
                let hits = parts.next().and_then(|part| part.parse::<u64>().ok());
                if let (Some(number), Some(hits)) = (number, hits) {
                    file.hit_line(number, hits);
                }
            } else if let Some(value) = counter(line, "LF:") {
                totals.found = value;
            } else if let Some(value) = counter(line, "LH:") {
                totals.hit = value;
            } else if let Some(value) = counter(line, "FNF:") {
                file.functions.found = value;
            } else if let Some(value) = counter(line, "FNH:") {
                file.functions.hit = value;
            } else if let Some(value) = counter(line, "BRF:") {
                file.branches.found = value;
            } else if let Some(value) = counter(line, "BRH:") {
                file.branches.hit = value;
            } else if line == "end_of_record" {
                if let Some(mut done) = current.take() {
                    if done.lines.is_empty() && totals.found > 0 {
                        done.line_tally = Some(totals);
                    }
                    files.push(done);
                }
            }
        }
    }
    if !seen_record {
        return Err("the report has no SF records".to_owned());
    }
    Ok(files)
}

fn counter(line: &str, prefix: &str) -> Option<u64> {
    line.strip_prefix(prefix)?.trim().parse().ok()
}
