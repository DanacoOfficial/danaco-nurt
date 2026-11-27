// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-code::profile::overview
// Responsibility:   Languages, modules, manifests and instruction files of the tree.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Languages, modules, manifests and instruction files of the tree.

use std::collections::BTreeMap;
use std::path::Path;

use crate::registry::Registry;

use super::manifests::Found;
use super::model::{Entry, Section, file_name, parent};
use super::source::Source;

/// Languages counted as data formats rather than code.
const DATA_LANGUAGES: [&str; 5] = ["json", "yaml", "toml", "markdown", "xml"];

/// Files with instructions for contributors and tools, by exact path.
const INSTRUCTION_FILES: [&str; 9] = [
    "AGENTS.md",
    "CLAUDE.md",
    "GEMINI.md",
    ".cursorrules",
    ".github/copilot-instructions.md",
    "CONTRIBUTING.md",
    "README.md",
    "README",
    "PROGRAM.md",
];

/// Lines of an instruction file searched for its first heading.
const HEADING_LINES_MAX: usize = 40;

/// One entry per language, code before data formats, most files first.
pub(crate) fn languages(files: &[String], registry: &Registry) -> Vec<Entry> {
    let mut counts: BTreeMap<String, (String, usize)> = BTreeMap::new();
    for path in files {
        if let Some(id) = registry.detect_by_name(Path::new(file_name(path))) {
            let name = registry
                .spec(&id)
                .map_or_else(|| id.to_string(), |spec| spec.name.clone());
            counts.entry(id.to_string()).or_insert((name, 0)).1 += 1;
        }
    }
    let (data, code): (Vec<_>, Vec<_>) = counts
        .into_iter()
        .partition(|(id, _)| DATA_LANGUAGES.contains(&id.as_str()));
    let code_total: usize = code.iter().map(|(_, (_, count))| count).sum();
    let mut entries = Vec::new();
    for (group, is_data) in [(code, false), (data, true)] {
        let mut group: Vec<(String, usize)> = group.into_iter().map(|(_, value)| value).collect();
        group.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        for (name, count) in group {
            let origin = if is_data {
                format!("{count} files, data")
            } else {
                format!("{count} files, {}%", percent(count, code_total))
            };
            entries.push(Entry::new(Section::Languages, name).origin(origin));
        }
    }
    entries
}

fn percent(part: usize, total: usize) -> usize {
    const HUNDRED: usize = 100;
    (part * HUNDRED + total / 2)
        .checked_div(total)
        .unwrap_or_default()
}

/// Directories holding a project manifest; without manifests below the root,
/// the top-level directories with code.
pub(crate) fn modules(source: &Source<'_>, manifests: &[Found], registry: &Registry) -> Vec<Entry> {
    let mut by_dir: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for manifest in manifests.iter().filter(|m| m.kind.defines_module()) {
        by_dir
            .entry(parent(&manifest.path))
            .or_default()
            .push(file_name(&manifest.path));
    }
    if by_dir.keys().all(|dir| dir.is_empty()) {
        for path in source.files() {
            if let Some((top, _)) = path.split_once('/') {
                let code = registry
                    .detect_by_name(Path::new(file_name(path)))
                    .is_some_and(|id| !DATA_LANGUAGES.contains(&id.as_str()));
                if code {
                    by_dir.entry(top).or_default();
                }
            }
        }
    }
    by_dir
        .into_iter()
        .map(|(dir, names)| {
            let name = if dir.is_empty() { "." } else { dir };
            let mut entry = Entry::new(Section::Modules, name).path(name);
            if !names.is_empty() {
                entry = entry.origin(names.join(", "));
            }
            entry
        })
        .collect()
}

/// One entry per manifest.
pub(crate) fn manifest_entries(manifests: &[Found]) -> Vec<Entry> {
    manifests
        .iter()
        .map(|manifest| {
            Entry::new(Section::Manifests, manifest.path.clone())
                .path(manifest.path.clone())
                .origin(manifest.kind.tool())
        })
        .collect()
}

/// Instruction files with their first heading.
pub(crate) fn instructions(source: &Source<'_>) -> Vec<Entry> {
    source
        .files()
        .iter()
        .filter(|path| is_instruction_file(path))
        .map(|path| {
            let mut entry = Entry::new(Section::Instructions, path.clone()).path(path.clone());
            if let Some(heading) = source.text(path).as_deref().and_then(first_heading) {
                entry = entry.origin(heading);
            }
            entry
        })
        .collect()
}

fn is_instruction_file(path: &str) -> bool {
    if INSTRUCTION_FILES.contains(&path) {
        return true;
    }
    path.strip_prefix("docs/")
        .is_some_and(|rest| !rest.contains('/') && rest.ends_with(".md"))
}

/// First Markdown heading of a text, without the markers.
fn first_heading(text: &str) -> Option<String> {
    text.lines()
        .take(HEADING_LINES_MAX)
        .map(str::trim)
        .find_map(|line| {
            let heading = line.trim_start_matches('#');
            (heading.len() < line.len() && heading.starts_with(' '))
                .then(|| heading.trim().to_owned())
        })
        .filter(|heading| !heading.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_heading_skips_text_before_it() {
        assert_eq!(
            first_heading("badge\n\n## Build guide\n"),
            Some("Build guide".to_owned())
        );
        assert_eq!(first_heading("#hashtag\nplain"), None);
    }

    #[test]
    fn instruction_files_include_top_level_docs_only() {
        assert!(is_instruction_file("README.md"));
        assert!(is_instruction_file("docs/setup.md"));
        assert!(!is_instruction_file("docs/api/setup.md"));
        assert!(!is_instruction_file("src/README.md"));
    }

    #[test]
    fn percentages_are_rounded() {
        assert_eq!(percent(1, 3), 33);
        assert_eq!(percent(2, 3), 67);
        assert_eq!(percent(0, 0), 0);
    }

    fn listed(paths: &[&str]) -> Vec<String> {
        paths.iter().map(|path| (*path).to_owned()).collect()
    }

    #[test]
    fn languages_list_code_by_share_before_data_formats() {
        let files = listed(&["a.rs", "src/b.rs", "tool.py", "data.json", "notes.txt"]);
        let entries = languages(&files, Registry::builtin());
        let found: Vec<(&str, &str)> = entries
            .iter()
            .map(|entry| (entry.name.as_str(), entry.origin.as_deref().unwrap_or("")))
            .collect();
        assert_eq!(
            found,
            [
                ("Rust", "2 files, 67%"),
                ("Python", "1 files, 33%"),
                ("JSON", "1 files, data"),
            ]
        );
        assert!(languages(&[], Registry::builtin()).is_empty());
    }

    #[test]
    fn languages_with_equal_counts_are_ordered_by_name() {
        let files = listed(&["x.rs", "y.go", "z.py"]);
        let names: Vec<String> = languages(&files, Registry::builtin())
            .into_iter()
            .map(|entry| entry.name)
            .collect();
        assert_eq!(names, ["Go", "Python", "Rust"]);
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
