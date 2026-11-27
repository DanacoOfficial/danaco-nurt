// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-correct/tests/lexical_hygiene.rs
// Responsibility:   The sources and corpus of the lexical correction rules contain no internal identifiers, build paths, work markers or Polish text.
// Dependencies:     regex
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! The sources and corpus of the lexical correction rules contain no
//! identifiers of internal decisions, no paths of a build environment, no
//! work markers and no text in Polish.

use std::fs;
use std::path::{Path, PathBuf};

use regex::Regex;

/// Letters of the Polish alphabet outside ASCII, written as code points.
const POLISH_LETTERS: [char; 18] = [
    '\u{105}', '\u{107}', '\u{119}', '\u{142}', '\u{144}', '\u{f3}', '\u{15b}', '\u{17a}',
    '\u{17c}', '\u{104}', '\u{106}', '\u{118}', '\u{141}', '\u{143}', '\u{d3}', '\u{15a}',
    '\u{179}', '\u{17b}',
];

fn repository_root() -> PathBuf {
    Path::new(
        std::env::var("CARGO_MANIFEST_DIR")
            .as_deref()
            .unwrap_or(env!("CARGO_MANIFEST_DIR")),
    )
    .join("../..")
}

fn files(directory: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for path in entries.filter_map(Result::ok).map(|entry| entry.path()) {
        if path.is_dir() {
            files(&path, out);
        } else {
            out.push(path);
        }
    }
}

/// Files checked: the modules, the integration tests and the corpus of the
/// lexical rules. This file is left out because it spells the patterns.
fn checked_files() -> Vec<PathBuf> {
    let crate_root = PathBuf::from(
        std::env::var("CARGO_MANIFEST_DIR")
            .unwrap_or_else(|_| env!("CARGO_MANIFEST_DIR").to_owned()),
    );
    let mut out = Vec::new();
    files(&crate_root.join("src/normalize"), &mut out);
    files(&crate_root.join("src/lexical"), &mut out);
    files(
        &repository_root().join("tests/d1/correction/lexical"),
        &mut out,
    );
    let tests = fs::read_dir(crate_root.join("tests"))
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| {
                    let name = path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or_default();
                    name.starts_with("lexical_") && name != "lexical_hygiene.rs"
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    out.extend(tests);
    out.retain(|path| {
        path.extension()
            .is_some_and(|ext| ext == "rs" || ext == "jsonl")
    });
    out.sort();
    out
}

fn patterns() -> Vec<(&'static str, Regex)> {
    let environment = format!(
        "{}|{}",
        ["dana", "co-nexus"].concat(),
        ["dana", "co-kvm"].concat()
    );
    let markers = ["TO", "DO|FIX", "ME|X", "XX|HA", "CK"].concat();
    [
        ("decision identifier", r"\b[PDV]-[0-9]+\b".to_owned()),
        ("environment path", environment),
        ("work marker", format!(r"\b({markers})\b")),
    ]
    .into_iter()
    .filter_map(|(name, pattern)| Regex::new(&pattern).ok().map(|regex| (name, regex)))
    .collect()
}

#[test]
fn sources_and_corpus_follow_the_repository_rules() {
    let patterns = patterns();
    assert_eq!(patterns.len(), 3);
    let checked = checked_files();
    assert!(checked.len() > 10, "{} files", checked.len());
    let mut problems = Vec::new();
    for path in checked {
        let text = fs::read_to_string(&path).unwrap_or_default();
        for (index, line) in text.lines().enumerate() {
            let location = format!("{}:{}", path.display(), index + 1);
            for (name, regex) in &patterns {
                if regex.is_match(line) {
                    problems.push(format!("{location}: {name}"));
                }
            }
            if line.chars().any(|c| POLISH_LETTERS.contains(&c)) {
                problems.push(format!("{location}: Polish text"));
            }
        }
    }
    assert!(problems.is_empty(), "{problems:#?}");
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
