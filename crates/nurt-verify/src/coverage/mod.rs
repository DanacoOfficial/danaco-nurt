// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Coverage report parsers.

use std::collections::BTreeMap;
use std::path::Path;

use nurt_contracts::record::{Coverage, CoverageFile, CoverageFormat};
use nurt_contracts::value::{Path as PathValue, Range};

pub(crate) mod cobertura;
pub(crate) mod go;
pub(crate) mod istanbul;
pub(crate) mod lcov;
pub(crate) mod llvm;

/// Decimal places of a reported percentage.
const PERCENT_SCALE: f64 = 100.0;

/// Percentage reported for a count of zero items.
const EMPTY_PERCENT: f64 = 100.0;

/// Longest range of lines that one block of a report marks.
pub(crate) const RANGE_LINES_MAX: u64 = 1_000_000;

/// Items found and items covered.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Tally {
    pub(crate) found: u64,
    pub(crate) hit: u64,
}

impl Tally {
    pub(crate) fn add(&mut self, other: Self) {
        self.found = self.found.saturating_add(other.found);
        self.hit = self.hit.saturating_add(other.hit);
    }
}

/// Coverage of one source file as a report states it.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct FileData {
    /// Path relative to the project directory.
    pub(crate) path: String,
    /// Hits per executable line.
    pub(crate) lines: BTreeMap<u64, u64>,
    /// Line totals when the report states them instead of per-line hits.
    pub(crate) line_tally: Option<Tally>,
    pub(crate) branches: Tally,
    pub(crate) functions: Tally,
    pub(crate) statements: Tally,
}

impl FileData {
    pub(crate) fn new(path: String) -> Self {
        Self {
            path,
            ..Self::default()
        }
    }

    /// Lines found and covered.
    pub(crate) fn line_totals(&self) -> Tally {
        self.line_tally.unwrap_or_else(|| Tally {
            found: self.lines.len() as u64,
            hit: self.lines.values().filter(|hits| **hits > 0).count() as u64,
        })
    }

    /// Records the hits of a line; the larger count wins when a report
    /// states a line more than once.
    pub(crate) fn hit_line(&mut self, line: u64, hits: u64) {
        let entry = self.lines.entry(line).or_insert(0);
        *entry = (*entry).max(hits);
    }

    /// Records the hits of the lines from `start` to `end`, both included.
    /// A report states the bounds, so a range longer than
    /// [`RANGE_LINES_MAX`] lines is cut instead of filling the memory.
    pub(crate) fn hit_range(&mut self, start: u64, end: u64, hits: u64) {
        let last = end.min(start.saturating_add(RANGE_LINES_MAX));
        for line in start..=last {
            self.hit_line(line, hits);
        }
    }

    fn merge(&mut self, other: Self) {
        if self.line_tally.is_some() || other.line_tally.is_some() {
            let mut sum = self.line_totals();
            sum.add(other.line_totals());
            self.line_tally = Some(sum);
        }
        for (line, hits) in other.lines {
            self.hit_line(line, hits);
        }
        self.branches.add(other.branches);
        self.functions.add(other.functions);
        self.statements.add(other.statements);
    }
}

/// Percentage of `hit` in `found`, with two decimals; 100 when nothing was
/// found.
pub(crate) fn percent(tally: Tally) -> f64 {
    if tally.found == 0 {
        return EMPTY_PERCENT;
    }
    let ratio = tally.hit as f64 / tally.found as f64;
    (ratio * 100.0 * PERCENT_SCALE).round() / PERCENT_SCALE
}

/// Path of a source file relative to the project directory, with `/`
/// separators.
pub(crate) fn normalize_path(file: &str, dir: &Path) -> String {
    let slashed = file.replace('\\', "/");
    let root = dir.to_string_lossy().replace('\\', "/");
    let root = root.trim_end_matches('/');
    let relative = slashed
        .strip_prefix(&format!("{root}/"))
        .unwrap_or(&slashed);
    relative.trim_start_matches("./").to_owned()
}

/// Joins the entries of a report that name the same file.
pub(crate) fn merge_files(files: Vec<FileData>) -> Vec<FileData> {
    let mut merged: Vec<FileData> = Vec::new();
    for file in files {
        match merged.iter_mut().find(|known| known.path == file.path) {
            Some(known) => known.merge(file),
            None => merged.push(file),
        }
    }
    merged
}

/// Ranges of consecutive lines that have no hits.
pub(crate) fn uncovered_ranges(lines: &BTreeMap<u64, u64>) -> Vec<Range> {
    let mut ranges: Vec<(u64, u64)> = Vec::new();
    for (line, hits) in lines {
        if *hits > 0 {
            continue;
        }
        match ranges.last_mut() {
            Some((_, end)) if *end + 1 == *line => *end = *line,
            _ => ranges.push((*line, *line)),
        }
    }
    ranges
        .into_iter()
        .filter_map(|(start, end)| {
            let start = i64::try_from(start).ok()?;
            let end = i64::try_from(end).ok()?;
            Range::new(Some(start), Some(end)).ok()
        })
        .collect()
}

/// Builds the coverage record of the files of a report.
pub(crate) fn summarize(files: Vec<FileData>, format: CoverageFormat) -> Coverage {
    let files = merge_files(files);
    let mut lines = Tally::default();
    let mut branches = Tally::default();
    let mut functions = Tally::default();
    let mut statements = Tally::default();
    for file in &files {
        lines.add(file.line_totals());
        branches.add(file.branches);
        functions.add(file.functions);
        statements.add(file.statements);
    }
    let optional = |tally: Tally| (tally.found > 0).then(|| percent(tally));
    let per_file: Vec<CoverageFile> = files
        .iter()
        .map(|file| {
            let uncovered = uncovered_ranges(&file.lines);
            CoverageFile {
                path: PathValue::from(file.path.as_str()),
                lines_pct: percent(file.line_totals()),
                uncovered: (!uncovered.is_empty()).then_some(uncovered),
            }
        })
        .collect();
    Coverage {
        lines_pct: percent(lines),
        branches_pct: optional(branches),
        functions_pct: optional(functions),
        statements_pct: optional(statements),
        files: Some(per_file),
        report_file: None,
        format: Some(format),
    }
}

/// Format of a report, from its content.
pub(crate) fn detect_format(text: &str) -> CoverageFormat {
    let start = text.trim_start();
    if start.starts_with("mode:") {
        CoverageFormat::Go
    } else if start.starts_with('<') {
        CoverageFormat::Cobertura
    } else if start.starts_with('{') {
        if start.contains("llvm.coverage.json.export") {
            CoverageFormat::Llvm
        } else {
            CoverageFormat::Istanbul
        }
    } else {
        CoverageFormat::Lcov
    }
}

/// Reads a report of the given format; `dir` is the project directory.
///
/// Fails with the reason when the content is not a report of the format.
pub(crate) fn parse(text: &str, format: CoverageFormat, dir: &Path) -> Result<Coverage, String> {
    let files = match format {
        CoverageFormat::Lcov => lcov::parse(text, dir)?,
        CoverageFormat::Cobertura => cobertura::parse(text, dir)?,
        CoverageFormat::Istanbul => istanbul::parse(text, dir)?,
        CoverageFormat::Go => go::parse(text, dir)?,
        CoverageFormat::Llvm => llvm::parse(text, dir)?,
        _ => return Err("the coverage format is not supported".to_owned()),
    };
    if files.is_empty() {
        return Err("the report has no files".to_owned());
    }
    Ok(summarize(files, format))
}

#[cfg(test)]
mod tests;
