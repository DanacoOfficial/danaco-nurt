// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-verify::coverage::cobertura
// Responsibility:   Cobertura XML reports.
// Dependencies:     regex
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Cobertura XML reports.

use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;

use super::{FileData, Tally, normalize_path};
use crate::test::xml::{self, Element};

/// Covered and total conditions in a `condition-coverage` attribute, as in
/// `50% (1/2)`.
static CONDITIONS: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new(r"\((\d+)/(\d+)\)").ok());

/// Reads the classes of a report as files. A class that appears in more than
/// one package entry is one file.
pub(crate) fn parse(text: &str, dir: &Path) -> Result<Vec<FileData>, String> {
    let parsed = xml::parse(text);
    if let Some(reason) = parsed.error {
        return Err(reason);
    }
    let Some(coverage) = parsed.root.children_named("coverage").next() else {
        return Err("the report has no coverage element".to_owned());
    };
    let prefix = source_prefix(coverage, dir);
    let mut files = Vec::new();
    collect(coverage, dir, &prefix, &mut files);
    Ok(files)
}

fn collect(element: &Element, dir: &Path, prefix: &str, files: &mut Vec<FileData>) {
    for child in &element.children {
        if child.name == "class" {
            files.push(class_file(child, dir, prefix));
        } else {
            collect(child, dir, prefix, files);
        }
    }
}

/// Directory the file names of the report are relative to, when it lies
/// inside the project directory.
fn source_prefix(coverage: &Element, dir: &Path) -> String {
    let source = coverage
        .children_named("sources")
        .flat_map(|sources| sources.children_named("source"))
        .map(|source| source.text.trim().replace('\\', "/"))
        .find(|text| !text.is_empty());
    let Some(source) = source else {
        return String::new();
    };
    let root = dir.to_string_lossy().replace('\\', "/");
    let root = root.trim_end_matches('/');
    if source.trim_end_matches('/') == root {
        return String::new();
    }
    match source.strip_prefix(&format!("{root}/")) {
        Some(relative) => format!("{}/", relative.trim_end_matches('/')),
        None => String::new(),
    }
}

fn class_file(class: &Element, dir: &Path, prefix: &str) -> FileData {
    let name = class.attribute("filename").unwrap_or_default();
    let joined = if name.starts_with('/') || name.contains(":\\") {
        name.to_owned()
    } else {
        format!("{prefix}{name}")
    };
    let mut file = FileData::new(normalize_path(&joined, dir));
    for lines in class.children_named("lines") {
        for line in lines.children_named("line") {
            line_hits(&mut file, line);
        }
    }
    for methods in class.children_named("methods") {
        for method in methods.children_named("method") {
            file.functions.found += 1;
            if method_hit(method) {
                file.functions.hit += 1;
            }
        }
    }
    file
}

fn line_hits(file: &mut FileData, line: &Element) {
    let number = line
        .attribute("number")
        .and_then(|text| text.parse::<u64>().ok());
    let hits = line
        .attribute("hits")
        .and_then(|text| text.parse::<u64>().ok())
        .unwrap_or(0);
    if let Some(number) = number {
        file.hit_line(number, hits);
    }
    if line.attribute("branch") == Some("true") {
        let tally = line
            .attribute("condition-coverage")
            .and_then(conditions)
            .unwrap_or_default();
        file.branches.add(tally);
    }
}

fn conditions(text: &str) -> Option<Tally> {
    let captures = CONDITIONS.as_ref()?.captures(text)?;
    Some(Tally {
        hit: captures[1].parse().ok()?,
        found: captures[2].parse().ok()?,
    })
}

fn method_hit(method: &Element) -> bool {
    method.children_named("lines").any(|lines| {
        lines.children_named("line").any(|line| {
            line.attribute("hits")
                .and_then(|text| text.parse::<u64>().ok())
                .is_some_and(|hits| hits > 0)
        })
    })
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
