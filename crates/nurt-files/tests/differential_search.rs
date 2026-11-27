// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-files/tests/differential_search.rs
// Responsibility:   Differential tests of `nurt_text::search` and the walker against `rg`.
// Dependencies:     nurt_contracts, nurt_text, support_corpus, support_diff, support_walk
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Differential tests of `nurt_text::search` and the walker against `rg`.

#![cfg(all(feature = "differential", unix))]
// Test helpers unwrap results so that a failure aborts the test with the original error.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support_corpus;
mod support_diff;
mod support_fs;
#[path = "support_diff/walk.rs"]
mod support_walk;

use std::collections::BTreeMap;
use std::fs::File;
use std::io::Read;
use std::ops::ControlFlow;
use std::path::Path;

use nurt_contracts::services::ExcludePreset;
use nurt_files::walk::{KindFilter, WalkOptions};
use nurt_text::search::{Pattern, SearchOptions, Searcher};
use support_corpus::{Corpus, CorpusSpec, NEEDLE};
use support_diff::{assert_same, normalize_path, require, run_expect};
use support_walk::walk_relative;

const SEED: u64 = 0xC0FF_EE01;
const FILES: usize = 1_500;
/// Separates the fields of a matching line in the compared output.
const MATCH_SEP: char = '\u{1f}';
/// Separates the fields of a context line in the compared output.
const CONTEXT_SEP: char = '\u{1e}';
const CONTEXT_LINES: usize = 2;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Lines,
    OnlyMatching,
    Counts,
    Files,
    Unmatched,
    Context,
}

/// One compared search: the Nurt configuration and the equivalent `rg` arguments.
struct Spec {
    label: &'static str,
    patterns: Vec<Pattern>,
    options: SearchOptions,
    mode: Mode,
    rg: Vec<&'static str>,
    hidden: bool,
    gitignore: bool,
    globs: Vec<&'static str>,
}

impl Spec {
    fn new(label: &'static str, patterns: Vec<Pattern>, mode: Mode) -> Self {
        Self {
            label,
            patterns,
            options: SearchOptions::new().max_line(0),
            mode,
            rg: Vec::new(),
            hidden: true,
            gitignore: true,
            globs: Vec::new(),
        }
    }

    fn options(mut self, options: SearchOptions) -> Self {
        self.options = options.max_line(0);
        self
    }

    fn rg(mut self, args: &[&'static str]) -> Self {
        self.rg.extend_from_slice(args);
        self
    }
}

fn literal_needle() -> Vec<Pattern> {
    vec![Pattern::literal(NEEDLE)]
}

fn regexes(sources: &[&str]) -> Vec<Pattern> {
    sources
        .iter()
        .map(|source| Pattern::regex(*source))
        .collect()
}

fn specs() -> Vec<Spec> {
    vec![
        Spec::new("literal", literal_needle(), Mode::Lines).rg(&["-F", "-e", NEEDLE]),
        Spec::new("ignore case", regexes(&["NURT_NEEDLE"]), Mode::Lines)
            .options(SearchOptions::new().case_sensitive(false))
            .rg(&["-i", "-e", "NURT_NEEDLE"]),
        Spec::new("whole word", regexes(&["alpha"]), Mode::Lines)
            .options(SearchOptions::new().whole_word(true))
            .rg(&["-w", "-e", "alpha"]),
        Spec::new("invert", regexes(&["alpha"]), Mode::Lines)
            .options(SearchOptions::new().invert(true))
            .rg(&["-v", "-e", "alpha"]),
        Spec::new(
            "only matching",
            regexes(&["(alpha|index) \\w+"]),
            Mode::OnlyMatching,
        )
        .options(SearchOptions::new().only_matching(true))
        .rg(&["-o", "-e", "(alpha|index) \\w+"]),
        Spec::new("counts", regexes(&["report"]), Mode::Counts).rg(&["-c", "-e", "report"]),
        Spec::new("files with matches", literal_needle(), Mode::Files)
            .rg(&["-l", "-F", "-e", NEEDLE]),
        Spec::new("files without match", literal_needle(), Mode::Unmatched).rg(&[
            "--files-without-match",
            "-F",
            "-e",
            NEEDLE,
        ]),
        Spec::new("context", regexes(&["report index"]), Mode::Context)
            .options(SearchOptions::new().context(CONTEXT_LINES))
            .rg(&["-C", "2", "-e", "report index"]),
        Spec::new("multiline", regexes(&["kernel\\nbravo"]), Mode::Lines)
            .options(SearchOptions::new().multiline(true))
            .rg(&["-U", "-e", "kernel\\nbravo"]),
        Spec::new(
            "several patterns",
            regexes(&["alpha", "driver"]),
            Mode::OnlyMatching,
        )
        .options(SearchOptions::new().only_matching(true))
        .rg(&["-o", "-e", "alpha", "-e", "driver"]),
        Spec::new(
            "unicode literal",
            vec![Pattern::literal("\u{17c}\u{f3}\u{142}\u{107}")],
            Mode::Lines,
        )
        .rg(&["-F", "-e", "\u{17c}\u{f3}\u{142}\u{107}"]),
        Spec::new(
            "unicode ignore case",
            regexes(&["ZA\u{17b}\u{d3}\u{141}\u{106}"]),
            Mode::Lines,
        )
        .options(SearchOptions::new().case_sensitive(false))
        .rg(&["-i", "-e", "ZA\u{17b}\u{d3}\u{141}\u{106}"]),
        Spec {
            globs: vec!["**/*.txt"],
            ..Spec::new("glob", literal_needle(), Mode::Lines)
                .rg(&["-F", "-e", NEEDLE, "-g", "*.txt"])
        },
        Spec {
            hidden: false,
            ..Spec::new("visible entries only", literal_needle(), Mode::Files)
                .rg(&["-l", "-F", "-e", NEEDLE])
        },
        Spec {
            gitignore: false,
            ..Spec::new("ignore files not applied", literal_needle(), Mode::Files).rg(&[
                "-l",
                "-F",
                "-e",
                NEEDLE,
                "--no-ignore",
            ])
        },
    ]
}

fn walk_options(spec: &Spec) -> WalkOptions {
    WalkOptions {
        kind: KindFilter::File,
        hidden: spec.hidden,
        gitignore: spec.gitignore,
        exclude_presets: vec![ExcludePreset::Vcs],
        globs: spec.globs.iter().map(|glob| (*glob).to_owned()).collect(),
        ..WalkOptions::default()
    }
}

/// Lines of one file in the compared form.
fn render(spec: &Spec, path: &str, searcher: &Searcher, data: &mut impl Read) -> Vec<String> {
    let mut lines: BTreeMap<u64, (char, String)> = BTreeMap::new();
    let mut only = Vec::new();
    let summary = searcher
        .search_reader(data, |hit| {
            let text = hit.text();
            match spec.mode {
                Mode::OnlyMatching => {
                    for found in hit.matches() {
                        let bytes = hit.matched(found).unwrap();
                        only.push(format!(
                            "{path}{MATCH_SEP}{}{MATCH_SEP}{}",
                            found.line,
                            String::from_utf8_lossy(bytes)
                        ));
                    }
                }
                _ => {
                    for (offset, line) in text.split('\n').enumerate() {
                        lines.insert(hit.line() + offset as u64, (MATCH_SEP, line.to_owned()));
                    }
                    for context in hit.before().iter().chain(hit.after()) {
                        lines
                            .entry(context.line())
                            .or_insert((CONTEXT_SEP, context.text().into_owned()));
                    }
                }
            }
            ControlFlow::Continue(())
        })
        .unwrap();
    match spec.mode {
        Mode::Counts => {
            if summary.hits > 0 {
                vec![format!("{path}{MATCH_SEP}{}", summary.hits)]
            } else {
                Vec::new()
            }
        }
        Mode::Files => (summary.hits > 0)
            .then(|| path.to_owned())
            .into_iter()
            .collect(),
        Mode::Unmatched => (summary.hits == 0)
            .then(|| path.to_owned())
            .into_iter()
            .collect(),
        Mode::OnlyMatching => only,
        Mode::Lines | Mode::Context => lines
            .into_iter()
            .map(|(line, (sep, text))| format!("{path}{sep}{line}{sep}{text}"))
            .collect(),
    }
}

fn nurt_search(root: &Path, spec: &Spec, skip_binary: bool) -> (Vec<String>, usize) {
    let searcher = Searcher::new(&spec.patterns, spec.options.clone()).unwrap();
    let mut paths = walk_relative(root, walk_options(spec));
    paths.sort();
    let mut skipped = 0;
    let mut out = Vec::new();
    for path in paths {
        let mut bytes = Vec::new();
        File::open(root.join(&path))
            .unwrap()
            .read_to_end(&mut bytes)
            .unwrap();
        if skip_binary && bytes.contains(&0) {
            skipped += 1;
            continue;
        }
        let key = normalize_path(&path, root);
        out.extend(render(spec, &key, &searcher, &mut bytes.as_slice()));
    }
    out.sort();
    (out, skipped)
}

fn rg_arguments(spec: &Spec, binary_default: bool) -> Vec<String> {
    let mut args: Vec<String> = [
        "--no-config",
        "--color",
        "never",
        "--no-ignore-global",
        "--no-messages",
        "-g",
        "!.git",
    ]
    .iter()
    .map(|a| (*a).to_owned())
    .collect();
    if !binary_default {
        args.extend(["-a", "-E", "none"].map(str::to_owned));
    }
    let separators = [
        "--field-match-separator",
        "\u{1f}",
        "--field-context-separator",
        "\u{1e}",
    ];
    if matches!(spec.mode, Mode::Lines | Mode::OnlyMatching | Mode::Context) {
        args.extend(["--no-heading", "--with-filename", "--line-number"].map(str::to_owned));
    }
    if spec.mode == Mode::Counts {
        args.push("--with-filename".to_owned());
    }
    args.extend(separators.map(str::to_owned));
    if spec.hidden {
        args.push("--hidden".to_owned());
    }
    args.extend(spec.rg.iter().map(|a| (*a).to_owned()));
    args.push(".".to_owned());
    args
}

fn rg_search(rg: &Path, root: &Path, spec: &Spec, binary_default: bool) -> Vec<String> {
    let args = rg_arguments(spec, binary_default);
    let output = run_expect(rg, &args, root, &[0, 1]);
    let mut lines: Vec<String> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|line| *line != "--")
        .map(|line| match spec.mode {
            Mode::Files | Mode::Unmatched => normalize_path(line, root),
            Mode::Counts => {
                let (path, count) = line.rsplit_once(':').unwrap();
                format!("{}{MATCH_SEP}{count}", normalize_path(path, root))
            }
            _ => {
                let split = line.find([MATCH_SEP, CONTEXT_SEP]).unwrap();
                format!("{}{}", normalize_path(&line[..split], root), &line[split..])
            }
        })
        .collect();
    lines.sort();
    lines
}

fn generated() -> (tempfile::TempDir, Corpus) {
    let dir = tempfile::tempdir().unwrap();
    let corpus = Corpus::generate(dir.path(), &CorpusSpec::new(SEED, FILES)).unwrap();
    (dir, corpus)
}

#[test]
fn matches_rg_for_every_mapped_option() {
    let Some(rg) = require("rg") else {
        return;
    };
    let (dir, _corpus) = generated();
    for spec in specs() {
        let (nurt, _) = nurt_search(dir.path(), &spec, false);
        let reference = rg_search(&rg, dir.path(), &spec, false);
        assert!(
            !reference.is_empty() || spec.mode == Mode::Unmatched,
            "{}: the corpus has no result",
            spec.label
        );
        assert_same(spec.label, &nurt, &reference);
    }
}

#[test]
fn binary_files_are_skipped_by_rg_and_by_the_nurt_filter() {
    let Some(rg) = require("rg") else {
        return;
    };
    let (dir, corpus) = generated();
    let spec = Spec::new("default binary handling", literal_needle(), Mode::Files)
        .rg(&["-l", "-F", "-e", NEEDLE]);
    let (nurt, skipped) = nurt_search(dir.path(), &spec, true);
    let reference = rg_search(&rg, dir.path(), &spec, true);
    assert!(skipped > 0, "the corpus has no binary file");
    assert!(
        corpus
            .entries
            .iter()
            .any(|entry| entry.kind == support_corpus::Kind::Binary),
        "the manifest has no binary file"
    );
    assert_same(spec.label, &nurt, &reference);
}

#[test]
fn long_lines_are_returned_whole_when_the_limit_is_off() {
    let Some(rg) = require("rg") else {
        return;
    };
    let (dir, corpus) = generated();
    assert!(
        corpus
            .entries
            .iter()
            .any(|entry| entry.kind == support_corpus::Kind::LongLine),
        "the manifest has no long line file"
    );
    let spec =
        Spec::new("long lines", regexes(&["^x{100}"]), Mode::Counts).rg(&["-c", "-e", "^x{100}"]);
    let (nurt, _) = nurt_search(dir.path(), &spec, false);
    let reference = rg_search(&rg, dir.path(), &spec, false);
    assert!(!reference.is_empty());
    assert_same(spec.label, &nurt, &reference);
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
