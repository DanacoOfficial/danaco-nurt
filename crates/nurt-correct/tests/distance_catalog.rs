// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Distance checks on the command catalog: pairs of close names, parameters
//! and values of one command, synonyms against catalog names, and the time
//! of a name lookup on the full comparison set.

#![allow(
    clippy::unwrap_used,
    reason = "helpers of the test read repository fixtures; a malformed fixture fails the test"
)]

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use nurt_correct::distance::{
    NameEntry, NameIndex, NameKind, Options, check_names, close_pairs, nearest,
};
use nurt_correct::tables::{TableIssue, load_embedded};
use nurt_correct::values::{FUNCTIONS, index};
use toml_edit::DocumentMut;

/// Command of the catalog with the data used by correction.
struct Command {
    name: String,
    effecting: bool,
    params: Vec<(String, Vec<String>)>,
}

fn catalog() -> Vec<Command> {
    let root = PathBuf::from(
        std::env::var("CARGO_MANIFEST_DIR")
            .as_deref()
            .unwrap_or(env!("CARGO_MANIFEST_DIR")),
    )
    .join("../../catalog");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "toml"))
        .filter(|path| path.file_name().is_some_and(|name| name != "_index.toml"))
        .collect();
    files.sort();
    files
        .iter()
        .flat_map(|path| {
            let document: DocumentMut = std::fs::read_to_string(path).unwrap().parse().unwrap();
            let commands = document["command"].as_array_of_tables().unwrap().clone();
            commands.iter().map(command).collect::<Vec<_>>()
        })
        .collect()
}

fn command(table: &toml_edit::Table) -> Command {
    let text = |key: &str| {
        table
            .get(key)
            .and_then(|item| item.as_str())
            .unwrap_or_default()
            .to_owned()
    };
    let action_effect = table
        .get("effects")
        .and_then(|item| item.as_array_of_tables())
        .is_some_and(|actions| {
            actions
                .iter()
                .any(|action| action["effect"].as_str() != Some("read"))
        });
    let effecting = matches!(text("category").as_str(), "change" | "run")
        || text("effect") != "read"
        || action_effect;
    let params = table
        .get("param")
        .and_then(|item| item.as_array_of_tables())
        .map(|params| {
            params
                .iter()
                .map(|param| {
                    let values = param
                        .get("values")
                        .and_then(|item| item.as_array())
                        .map(|values| {
                            values
                                .iter()
                                .filter_map(|v| v.as_str().map(str::to_owned))
                                .collect()
                        })
                        .unwrap_or_default();
                    (param["name"].as_str().unwrap().to_owned(), values)
                })
                .collect()
        })
        .unwrap_or_default();
    Command {
        name: text("name"),
        effecting,
        params,
    }
}

fn command_entries(commands: &[Command]) -> Vec<NameEntry> {
    commands
        .iter()
        .map(|command| NameEntry::command(command.name.clone(), command.effecting))
        .collect()
}

#[test]
fn commands_have_one_pair_at_distance_one_and_twenty_seven_at_two() {
    let commands = catalog();
    assert_eq!(commands.len(), 149);
    let pairs = close_pairs(&command_entries(&commands), 2);
    let at = |distance| -> Vec<String> {
        pairs
            .iter()
            .filter(|pair| pair.distance == distance)
            .map(|pair| format!("{}-{}", pair.a, pair.b))
            .collect()
    };
    assert_eq!(at(1), ["last-list"]);
    assert_eq!(
        at(2),
        [
            "assert-insert",
            "compress-decompress",
            "first-list",
            "git.log-git.tag",
            "git.pull-git.push",
            "git.stage-git.stash",
            "git.stage-git.tag",
            "hash-host",
            "hash-last",
            "host-last",
            "host-list",
            "host-sort",
            "info-undo",
            "jobs-logs",
            "move-remove",
            "next-set",
            "notebook.copy-notebook.ops",
            "notebook.move-notebook.remove",
            "ports-sort",
            "read-redo",
            "redo-undo",
            "rename-resume",
            "restart-start",
            "set-sort",
            "set-sql",
            "skip-stop",
            "sort-start",
        ]
    );
}

#[test]
fn command_names_conflict_only_in_the_documented_exception() {
    let entries = command_entries(&catalog());
    assert_eq!(check_names(&entries, &[]).len(), 1);
    assert!(check_names(&entries, &[("list", "last")]).is_empty());
}

#[test]
fn parameters_of_one_command_conflict_only_in_documented_pairs() {
    let mut found = Vec::new();
    for command in catalog() {
        let entries: Vec<NameEntry> = command
            .params
            .iter()
            .map(|(name, _)| NameEntry::name(name.clone(), NameKind::Param))
            .collect();
        for pair in check_names(&entries, &[]) {
            found.push(format!("{} {}-{}", command.name, pair.a, pair.b));
        }
    }
    assert_eq!(
        found,
        [
            "code.rename path-paths",
            "run bail-tail",
            "run batch-watch",
            "set value-values"
        ]
    );
}

#[test]
fn values_of_one_parameter_conflict_only_in_documented_pairs() {
    let mut found = Vec::new();
    for command in catalog() {
        for (param, values) in &command.params {
            let entries: Vec<NameEntry> = values
                .iter()
                .map(|value| NameEntry::name(value.clone(), NameKind::Value))
                .collect();
            for pair in check_names(&entries, &[]) {
                found.push(format!("{}.{param} {}-{}", command.name, pair.a, pair.b));
            }
        }
    }
    assert_eq!(
        found,
        [
            "archive.create.format tar_gz-tar_xz",
            "write.format csv-tsv",
            "write.value_type dword-qword",
            "set.format csv-tsv",
            "set.format json-jsonc",
        ]
    );
}

fn catalog_names(commands: &[Command]) -> Vec<NameEntry> {
    let mut names: BTreeSet<(NameKind, String)> = BTreeSet::new();
    for command in commands {
        for (param, values) in &command.params {
            names.insert((NameKind::Param, param.clone()));
            names.extend(values.iter().map(|value| (NameKind::Value, value.clone())));
        }
    }
    names.extend(
        FUNCTIONS
            .iter()
            .map(|function| (NameKind::Function, (*function).to_owned())),
    );
    let mut entries = command_entries(commands);
    entries.extend(
        names
            .into_iter()
            .map(|(kind, text)| NameEntry::name(text, kind)),
    );
    entries
}

#[test]
fn synonyms_point_to_catalog_names_and_never_shadow_them() {
    let commands = catalog();
    let tables = load_embedded().unwrap();
    let issues = tables.check_names(&catalog_names(&commands));
    let wrong: Vec<String> = issues
        .iter()
        .filter(|issue| match issue {
            TableIssue::SynonymIsName { kind, .. } => {
                matches!(kind, NameKind::Command | NameKind::Function)
            }
            TableIssue::UnknownTarget { kind, .. } => *kind != NameKind::Field,
            TableIssue::Close(_) => false,
        })
        .map(ToString::to_string)
        .collect();
    assert!(wrong.is_empty(), "{wrong:#?}");
}

#[test]
fn synonyms_and_habits_close_to_commands_are_documented_ties() {
    let commands = catalog();
    let tables = load_embedded().unwrap();
    let close: Vec<String> = tables
        .check_names(&command_entries(&commands))
        .into_iter()
        .filter_map(|issue| match issue {
            TableIssue::Close(pair) if pair.kind == NameKind::Command => {
                Some(format!("{}-{}", pair.a, pair.b))
            }
            _ => None,
        })
        .collect();
    assert_eq!(close, DOCUMENTED_COMMAND_TIES, "{close:?}");
}

/// Synonyms and habit words at distance 1 from a command other than their
/// target. A string at distance 1 from both is a tie and is not corrected;
/// the written word itself resolves exactly.
const DOCUMENTED_COMMAND_TIES: [&str; 6] = [
    "head-read",
    "ren-run",
    "run-run0",
    "sed-set",
    "sl-sql",
    "start-stat",
];

fn command_index() -> NameIndex {
    let tables = load_embedded().unwrap();
    index(NameKind::Command, command_entries(&catalog()), &tables)
}

#[test]
fn comparison_set_covers_commands_synonyms_and_habits() {
    let index = command_index();
    assert!(index.len() >= 300, "{}", index.len());
}

/// Names of a typical program: commands, misspelled commands and a habit.
const TYPICAL_PROGRAM: [&str; 6] = ["find", "fitler", "sort", "frist", "remvoe", "grep"];

/// Repetitions of the timed lookup.
const RUNS: usize = 1_000;

/// Median time of correcting the names of a typical program.
const TYPICAL_LIMIT: Duration = Duration::from_millis(1);

/// Limit of the 99th percentile of correcting the names of a typical
/// program; the largest sample also measures scheduling delays of the host.
const WORST_LIMIT: Duration = Duration::from_millis(5);

/// Percentile compared with [`WORST_LIMIT`].
const WORST_PERCENTILE: usize = 99;

#[test]
#[ignore = "timing test; run explicitly with --ignored on an idle machine"]
fn corrects_a_typical_program_within_one_millisecond() {
    let index = command_index();
    let options = Options::default();
    let mut times: Vec<Duration> = (0..RUNS)
        .map(|_| {
            let start = Instant::now();
            for name in TYPICAL_PROGRAM {
                std::hint::black_box(nearest(name, &index, NameKind::Command, &options));
            }
            start.elapsed()
        })
        .collect();
    times.sort_unstable();
    let median = times[RUNS / 2];
    let worst = times[RUNS * WORST_PERCENTILE / 100];
    eprintln!(
        "comparison set {} names, median {median:?}, 99th percentile {worst:?}, largest {:?}",
        index.len(),
        times[RUNS - 1]
    );
    assert!(median < TYPICAL_LIMIT, "{median:?}");
    assert!(worst < WORST_LIMIT, "{worst:?}");
}
