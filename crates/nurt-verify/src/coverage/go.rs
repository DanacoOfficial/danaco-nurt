// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Go cover profiles.

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;

use super::{FileData, Tally, normalize_path};

/// Block of a profile: `file:start.col,end.col statements count`.
static BLOCK: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"^(.+):(\d+)\.\d+,(\d+)\.\d+ (\d+) (\d+)$").ok());

/// Module path of the project, followed by `/`, or an empty text.
fn module_prefix(dir: &Path) -> String {
    let Ok(text) = fs::read_to_string(dir.join("go.mod")) else {
        return String::new();
    };
    text.lines()
        .find_map(|line| line.trim().strip_prefix("module "))
        .map(|name| format!("{}/", name.trim()))
        .unwrap_or_default()
}

struct Block {
    file: String,
    start: u64,
    end: u64,
    statements: u64,
    count: u64,
}

/// Reads a profile. Blocks that several packages report are counted once,
/// with the highest count.
pub(crate) fn parse(text: &str, dir: &Path) -> Result<Vec<FileData>, String> {
    let Some(block) = BLOCK.as_ref() else {
        return Err("the block pattern is not valid".to_owned());
    };
    if !text.trim_start().starts_with("mode:") {
        return Err("the report has no mode line".to_owned());
    }
    let prefix = module_prefix(dir);
    let mut order: Vec<String> = Vec::new();
    let mut blocks: HashMap<String, Block> = HashMap::new();
    for line in text.lines() {
        let Some(captures) = block.captures(line.trim()) else {
            continue;
        };
        let key = format!("{}:{}-{}", &captures[1], &captures[2], &captures[3]);
        let count: u64 = captures[5].parse().unwrap_or(0);
        match blocks.get_mut(&key) {
            Some(known) => known.count = known.count.max(count),
            None => {
                let name = &captures[1];
                let file = match name.strip_prefix(&prefix) {
                    Some(local) if !prefix.is_empty() => local.to_owned(),
                    _ => normalize_path(name, dir),
                };
                order.push(key.clone());
                blocks.insert(
                    key,
                    Block {
                        file,
                        start: captures[2].parse().unwrap_or(0),
                        end: captures[3].parse().unwrap_or(0),
                        statements: captures[4].parse().unwrap_or(0),
                        count,
                    },
                );
            }
        }
    }
    let mut files: Vec<FileData> = Vec::new();
    for key in order {
        let Some(entry) = blocks.remove(&key) else {
            continue;
        };
        let position = match files.iter().position(|file| file.path == entry.file) {
            Some(position) => position,
            None => {
                files.push(FileData::new(entry.file.clone()));
                files.len() - 1
            }
        };
        let file = &mut files[position];
        let covered = if entry.count > 0 { entry.statements } else { 0 };
        file.statements.add(Tally {
            found: entry.statements,
            hit: covered,
        });
        for line in entry.start..=entry.end {
            file.hit_line(line, entry.count);
        }
    }
    for file in &mut files {
        file.line_tally = Some(file.statements);
    }
    Ok(files)
}
