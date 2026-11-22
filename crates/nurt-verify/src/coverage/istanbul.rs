// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Istanbul JSON reports: `coverage-final.json` and `coverage-summary.json`.

use std::collections::HashMap;
use std::path::Path;

use serde::Deserialize;
use serde_json::Value;

use super::{FileData, Tally, normalize_path};

/// Key of the entry that holds the totals of a summary report.
const SUMMARY_TOTAL_KEY: &str = "total";

#[derive(Deserialize, Default)]
struct Position {
    #[serde(default)]
    line: u64,
}

#[derive(Deserialize, Default)]
struct StatementLocation {
    #[serde(default)]
    start: Position,
}

#[derive(Deserialize, Default)]
struct FileReport {
    #[serde(default)]
    path: Option<String>,
    #[serde(rename = "statementMap", default)]
    statement_map: HashMap<String, StatementLocation>,
    #[serde(default)]
    s: HashMap<String, u64>,
    #[serde(default)]
    f: HashMap<String, u64>,
    #[serde(default)]
    b: HashMap<String, Vec<u64>>,
}

#[derive(Deserialize, Default)]
struct SummaryItem {
    #[serde(default)]
    total: u64,
    #[serde(default)]
    covered: u64,
}

#[derive(Deserialize, Default)]
struct SummaryFile {
    #[serde(default)]
    lines: SummaryItem,
    #[serde(default)]
    statements: SummaryItem,
    #[serde(default)]
    functions: SummaryItem,
    #[serde(default)]
    branches: SummaryItem,
}

/// Reads either form of the report; the summary form is recognized by its
/// `total` entry.
pub(crate) fn parse(text: &str, dir: &Path) -> Result<Vec<FileData>, String> {
    let document: Value = serde_json::from_str(text)
        .map_err(|failure| format!("the report is not valid JSON: {failure}"))?;
    let Value::Object(entries) = document else {
        return Err("the report is not a JSON object".to_owned());
    };
    let mut files = Vec::new();
    if entries.contains_key(SUMMARY_TOTAL_KEY) {
        for (path, entry) in entries {
            if path == SUMMARY_TOTAL_KEY {
                continue;
            }
            let summary: SummaryFile = serde_json::from_value(entry)
                .map_err(|failure| format!("a summary entry is not valid: {failure}"))?;
            files.push(summary_file(&path, &summary, dir));
        }
    } else {
        for (key, entry) in entries {
            let report: FileReport = serde_json::from_value(entry)
                .map_err(|failure| format!("a file entry is not valid: {failure}"))?;
            let path = report.path.clone().unwrap_or(key);
            files.push(detail_file(&path, &report, dir));
        }
    }
    Ok(files)
}

fn summary_file(path: &str, summary: &SummaryFile, dir: &Path) -> FileData {
    let mut file = FileData::new(normalize_path(path, dir));
    file.line_tally = Some(tally(&summary.lines));
    file.statements = tally(&summary.statements);
    file.functions = tally(&summary.functions);
    file.branches = tally(&summary.branches);
    file
}

fn tally(item: &SummaryItem) -> Tally {
    Tally {
        found: item.total,
        hit: item.covered,
    }
}

fn detail_file(path: &str, report: &FileReport, dir: &Path) -> FileData {
    let mut file = FileData::new(normalize_path(path, dir));
    for (id, location) in &report.statement_map {
        let hits = report.s.get(id).copied().unwrap_or(0);
        file.hit_line(location.start.line, hits);
        file.statements.found += 1;
        if hits > 0 {
            file.statements.hit += 1;
        }
    }
    file.functions.found = report.f.len() as u64;
    file.functions.hit = report.f.values().filter(|hits| **hits > 0).count() as u64;
    for counts in report.b.values() {
        file.branches.found += counts.len() as u64;
        file.branches.hit += counts.iter().filter(|hits| **hits > 0).count() as u64;
    }
    file
}
