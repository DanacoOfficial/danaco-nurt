// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-migrate::output
// Responsibility:   Text of the `.nurt` file written by a migration.
// Dependencies:     serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Text of the `.nurt` file written by a migration.

use std::fmt::Write as _;

use serde_json::json;

use crate::report::{Item, MigrationReport};

/// Lines of the program file that stand for one comment or statement of the
/// script, with the source lines it covers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Block {
    pub(crate) start_line: u32,
    pub(crate) end_line: u32,
    pub(crate) lines: Vec<String>,
}

/// Lines for one report item: rule and assumption comments followed by the
/// program, or the original statement as comments.
pub(crate) fn item_lines(item: &Item) -> Vec<String> {
    let mut lines = Vec::new();
    if let (true, Some(output)) = (item.status.is_translated(), &item.output) {
        if !item.rules.is_empty() {
            lines.push(format!("# translated: {}", item.rules.join(", ")));
        }
        for assumption in &item.assumptions {
            lines.push(format!("# assumption: {assumption}"));
        }
        lines.extend(output.lines().map(str::to_owned));
        return lines;
    }
    let code = item.code.map_or("", |code| code.as_str());
    let mut source = item.source.lines();
    let first = source.next().unwrap_or_default();
    lines.push(format!("# untranslated: {code}: {first}"));
    lines.extend(source.map(|line| comment(line, "")));
    if let Some(problem) = &item.problem {
        let mut problem_lines = problem.lines();
        let first = problem_lines.next().unwrap_or_default();
        lines.push(format!("#   problem: {first}"));
        lines.extend(problem_lines.map(|line| comment(line, "           ")));
    }
    for fix in &item.fix {
        let mut fix_lines = fix.lines();
        let first = fix_lines.next().unwrap_or_default();
        lines.push(format!("#   fix: {first}"));
        lines.extend(fix_lines.map(|line| comment(line, "        ")));
    }
    lines
}

fn comment(line: &str, indent: &str) -> String {
    if line.is_empty() {
        "#".to_owned()
    } else {
        format!("# {indent}{line}")
    }
}

/// Renders the program file: a header with the counts and proposals, then the
/// blocks in source order. A blank line separates blocks that were separated
/// by blank lines in the script.
pub(crate) fn render(report: &MigrationReport, blocks: &[Block]) -> String {
    let summary = report.summary();
    let mut text = String::new();
    let script = report.script.replace(['\r', '\n'], " ");
    let _ = writeln!(text, "# Source: {script} ({})", report.language);
    text.push_str("# Statements run in order; the first error stops the script.\n");
    let _ = writeln!(
        text,
        "# Statements: {}; translated: {}; with assumptions: {}; impossible: {}; errors: {}.",
        summary.statements,
        summary.translated,
        summary.assumption,
        summary.impossible,
        summary.error,
    );
    if let Some(fallback) = &report.fallback {
        let mut fallback_lines = fallback.lines();
        let first = fallback_lines.next().unwrap_or_default();
        let _ = writeln!(
            text,
            "# Untranslated statements run unchanged with: {first}"
        );
        for line in fallback_lines {
            let _ = writeln!(text, "{}", comment(line, ""));
        }
    }
    if let Some(base) = &report.base {
        let _ = writeln!(
            text,
            "# Relative paths require the invocation parameter base: {}",
            json!(base)
        );
    }
    let mut previous_end = None;
    for block in blocks {
        let separated =
            previous_end.is_none_or(|end: u32| block.start_line > end.saturating_add(1));
        if separated {
            text.push('\n');
        }
        for line in &block.lines {
            text.push_str(line);
            text.push('\n');
        }
        previous_end = Some(block.end_line);
    }
    text
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
