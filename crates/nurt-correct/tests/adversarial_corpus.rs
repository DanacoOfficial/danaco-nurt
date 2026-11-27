// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-correct/tests/adversarial_corpus.rs
// Responsibility:   Adversarial corpus: generation of cases whose correct reaction is no.
// Dependencies:     nurt_contracts, serde_json, toml_edit
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Adversarial corpus: generation of cases whose correct reaction is no
//! correction or `E_AMBIGUOUS`, the component check of every case, the share
//! of ambiguous changing programs, and the format check of every correction
//! corpus file.
//!
//! Cases are built from the catalog and from fixed templates. The expected
//! reaction follows from the construction and from a naive implementation of
//! the acceptance rules written here, independent of the crate. The file in
//! the repository must equal a fresh generation; set
//! `NURT_REGENERATE_CORPUS=1` to rewrite it.

#![allow(
    clippy::unwrap_used,
    reason = "helpers of the test read repository fixtures; a malformed fixture fails the test"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use nurt_contracts::ast::AstDocument;
use nurt_contracts::bound::BoundDocument;
use nurt_contracts::error::ErrorCode;
use nurt_correct::distance::{NameEntry, NameIndex, NameKind, NameRole, Options};
use nurt_correct::policy::{
    Class, Decision, Facts, Invariant, InvariantInput, InvariantViolation, ProgramKind,
    check_invariants, decide,
};
use nurt_correct::record::RuleId;
use nurt_correct::tables::{Tables, load_embedded};
use nurt_correct::types::{self, Outcome, Written};
use nurt_correct::values::{self, Match};
use serde_json::{Map, Value, json};
use toml_edit::DocumentMut;

const CORPUS: &str = "tests/d1/correction/adversarial/generated.jsonl";
const MANUAL: &str = "tests/d1/correction/adversarial/manual.jsonl";

/// Least number of adversarial cases over all corpus files.
const MIN_ADVERSARIAL: usize = 400;
const CORPORA: &str = "tests/d1/correction";
const REGENERATE: &str = "NURT_REGENERATE_CORPUS";
const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz";

/// Explicit base used by the cases that resolve a relative path.
const BASE: &str = "/srv/app";

/// Largest share of `E_AMBIGUOUS` among changing programs whose intent is
/// unambiguous.
const MAX_AMB: f64 = 0.05;

/// Admissible number of generated cases.
const MIN_CASES: usize = 180;
const MAX_CASES: usize = 240;

/// Cases of the literal class and of each name class.
const LITERAL_CASES: usize = 40;
const NAME_CASES: usize = 20;

/// Variants kept for one short name.
const SHORT_VARIANTS: usize = 7;

/// Name lengths of the generated name classes.
const SHORT_NAME_LEN: usize = 3;
const TIE_NAME_LENS: std::ops::RangeInclusive<usize> = 4..=10;
const FIRST_CHAR_MIN_LEN: usize = 5;
const FAR_MIN_LEN: usize = 8;
const LITERAL_MIN_LEN: usize = 4;

/// Shortest prefix tried for a mode value.
const MIN_PREFIX: usize = 3;

/// Largest number of candidates, fixes and alternatives in an expectation.
const MAX_LISTED: usize = 3;

const MEMBERS: [&str; 12] = [
    "id",
    "input",
    "variant",
    "context",
    "expect",
    "program_kind",
    "intent",
    "corpus",
    "source",
    "note",
    "mutation",
    "probe",
];
const REQUIRED: [&str; 7] = [
    "id",
    "input",
    "variant",
    "expect",
    "program_kind",
    "intent",
    "source",
];
const CONTEXT_MEMBERS: [&str; 6] = ["base", "os", "config", "lets", "fields", "timezone"];
const ENUM_MEMBERS: [(&str, &[&str]); 5] = [
    ("variant", &["a", "b", "c"]),
    ("program_kind", &["read", "effect"]),
    ("intent", &["unambiguous", "ambiguous"]),
    ("corpus", &["mistake", "adversarial"]),
    ("source", &["manual", "generated", "example", "model"]),
];
const OS_NAMES: [&str; 3] = ["linux", "macos", "windows"];
const CLASSES: [&str; 4] = ["C", "S", "F", "P"];

// Catalog, golden programs and comparison set.

struct Command {
    name: String,
    effecting: bool,
    required: Vec<(String, bool)>,
    params: Vec<(String, Vec<String>)>,
}

fn root() -> PathBuf {
    PathBuf::from(
        std::env::var("CARGO_MANIFEST_DIR")
            .as_deref()
            .unwrap_or(env!("CARGO_MANIFEST_DIR")),
    )
    .join("../..")
}

fn sorted_files(directory: &Path, extension: &str) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == extension))
        .collect();
    files.sort();
    files
}

fn catalog() -> Vec<Command> {
    sorted_files(&root().join("catalog"), "toml")
        .iter()
        .filter(|path| path.file_name().is_some_and(|name| name != "_index.toml"))
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
    let params: Vec<&toml_edit::Table> = table
        .get("param")
        .and_then(|item| item.as_array_of_tables())
        .map(|params| params.iter().collect())
        .unwrap_or_default();
    let required = params
        .iter()
        .filter(|param| {
            param
                .get("required")
                .and_then(|item| item.as_bool())
                .unwrap_or(false)
        })
        .map(|param| {
            let ty = param
                .get("type")
                .and_then(|item| item.as_str())
                .unwrap_or_default()
                .to_owned();
            (ty, param.contains_key("position"))
        })
        .collect();
    let params = params
        .iter()
        .map(|param| {
            let values = param
                .get("values")
                .and_then(|item| item.as_array())
                .map(|values| {
                    values
                        .iter()
                        .filter_map(|value| value.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default();
            (param["name"].as_str().unwrap().to_owned(), values)
        })
        .collect();
    Command {
        name: text("name"),
        effecting,
        required,
        params,
    }
}

/// Single-line golden programs of variant A; the files of variants B and C
/// are named `<stem>.b.nurt` and `<stem>.c.nurt`.
fn golden_programs() -> Vec<String> {
    sorted_files(&root().join("spec/golden/ast"), "nurt")
        .iter()
        .filter(|path| {
            path.file_stem()
                .is_some_and(|stem| !stem.to_string_lossy().contains('.'))
        })
        .map(|path| std::fs::read_to_string(path).unwrap().trim_end().to_owned())
        .filter(|program| !program.contains('\n'))
        .collect()
}

#[derive(Debug, Clone)]
struct Candidate {
    text: String,
    name: bool,
    effecting: bool,
}

struct World {
    commands: Vec<Command>,
    golden: Vec<String>,
    tables: Tables,
    command_set: Vec<Candidate>,
    command_index: NameIndex,
}

impl World {
    fn new() -> Self {
        let commands = catalog();
        let tables = load_embedded().unwrap();
        let mut command_set: Vec<Candidate> = commands
            .iter()
            .map(|command| Candidate {
                text: command.name.clone(),
                name: true,
                effecting: command.effecting,
            })
            .collect();
        for entry in tables.name_entries() {
            let keep = match &entry.role {
                NameRole::Synonym { to } => {
                    entry.kind == NameKind::Command && commands.iter().any(|c| &c.name == to)
                }
                NameRole::Habit => true,
                NameRole::Name => false,
            };
            if keep {
                command_set.push(Candidate {
                    text: entry.text,
                    name: false,
                    effecting: false,
                });
            }
        }
        let names = commands
            .iter()
            .map(|command| NameEntry::command(command.name.clone(), command.effecting));
        let command_index = values::index(NameKind::Command, names, &tables);
        Self {
            commands,
            golden: golden_programs(),
            tables,
            command_set,
            command_index,
        }
    }

    fn command(&self, name: &str) -> Option<&Command> {
        self.commands.iter().find(|command| command.name == name)
    }

    fn effecting(&self, name: &str) -> bool {
        self.command(name).is_some_and(|command| command.effecting)
    }

    fn kind(&self, name: &str) -> &'static str {
        if self.effecting(name) {
            "effect"
        } else {
            "read"
        }
    }

    /// A valid program with `name` as its head stage.
    fn seed(&self, name: &str) -> Option<String> {
        let command = self.command(name)?;
        match command.required.as_slice() {
            [] => Some(name.to_owned()),
            [(ty, true)] if ty.contains("path") => Some(format!("{name} \"{BASE}/data\"")),
            _ => {
                let prefix = format!("{name} ");
                self.golden
                    .iter()
                    .find(|program| program.starts_with(&prefix))
                    .cloned()
            }
        }
    }

    fn values(&self, command: &str, param: &str) -> Vec<String> {
        self.command(command)
            .and_then(|command| command.params.iter().find(|(name, _)| name == param))
            .map(|(_, values)| values.clone())
            .unwrap_or_default()
    }

    /// Comparison set of the values of one parameter with their synonyms.
    fn value_set(&self, values: &[String]) -> Vec<Candidate> {
        let mut set: Vec<Candidate> = values
            .iter()
            .map(|value| Candidate {
                text: value.clone(),
                name: true,
                effecting: false,
            })
            .collect();
        for entry in self.tables.name_entries() {
            if let NameRole::Synonym { to } = &entry.role
                && entry.kind == NameKind::Value
                && values.contains(to)
            {
                set.push(Candidate {
                    text: entry.text.clone(),
                    name: false,
                    effecting: false,
                });
            }
        }
        set
    }
}

// Naive acceptance rules, independent of the crate.

fn naive_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut table = vec![vec![0usize; b.len() + 1]; a.len() + 1];
    for (i, row) in table.iter_mut().enumerate() {
        row[0] = i;
    }
    for (j, cell) in table[0].iter_mut().enumerate() {
        *cell = j;
    }
    for i in 1..=a.len() {
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            let mut best = (table[i - 1][j] + 1)
                .min(table[i][j - 1] + 1)
                .min(table[i - 1][j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                best = best.min(table[i - 2][j - 2] + 1);
            }
            table[i][j] = best;
        }
    }
    table[a.len()][b.len()]
}

fn naive_threshold(length: usize) -> usize {
    match length {
        0..=3 => 0,
        4..=7 => 1,
        _ => 2,
    }
}

fn closest<'a>(written: &str, set: &'a [Candidate]) -> (usize, Vec<&'a Candidate>) {
    let best = set
        .iter()
        .map(|candidate| naive_distance(written, &candidate.text))
        .min()
        .unwrap_or(usize::MAX);
    let winners = set
        .iter()
        .filter(|candidate| naive_distance(written, &candidate.text) == best)
        .collect();
    (best, winners)
}

/// Whether a unique name at distance `best` would be corrected.
fn admissible(written: &str, name: &Candidate, best: usize) -> bool {
    best >= 1
        && best <= naive_threshold(name.text.chars().count())
        && written.chars().next() == name.text.chars().next()
        && !(name.effecting && best > 1)
}

/// Whether `written` resolves: an exact entry of the set, or a unique closest
/// name that passes the thresholds.
fn naive_resolves(written: &str, set: &[Candidate]) -> bool {
    let (best, winners) = closest(written, set);
    if best == 0 {
        return true;
    }
    let texts: BTreeSet<&str> = winners
        .iter()
        .map(|candidate| candidate.text.as_str())
        .collect();
    texts.len() == 1
        && winners
            .iter()
            .any(|candidate| candidate.name && admissible(written, candidate, best))
}

/// Texts tied at the smallest distance when one of them is a name that
/// would be corrected if it were unique.
fn naive_tie(written: &str, set: &[Candidate]) -> Option<Vec<String>> {
    let (best, winners) = closest(written, set);
    let texts: BTreeSet<&str> = winners
        .iter()
        .map(|candidate| candidate.text.as_str())
        .collect();
    let correctable = winners
        .iter()
        .any(|candidate| candidate.name && admissible(written, candidate, best));
    (texts.len() > 1 && correctable).then(|| texts.into_iter().map(str::to_owned).collect())
}

// Edits of names.

fn swap(word: &str, at: usize) -> Option<String> {
    let mut chars: Vec<char> = word.chars().collect();
    if at + 1 >= chars.len()
        || chars[at] == chars[at + 1]
        || chars[at] == '.'
        || chars[at + 1] == '.'
    {
        return None;
    }
    chars.swap(at, at + 1);
    Some(chars.into_iter().collect())
}

fn substitute(word: &str, at: usize, letter: char) -> Option<String> {
    let mut chars: Vec<char> = word.chars().collect();
    if at >= chars.len() || chars[at] == letter || chars[at] == '.' {
        return None;
    }
    chars[at] = letter;
    Some(chars.into_iter().collect())
}

fn delete(word: &str, at: usize) -> Option<String> {
    let mut chars: Vec<char> = word.chars().collect();
    if at >= chars.len() || chars[at] == '.' {
        return None;
    }
    chars.remove(at);
    Some(chars.into_iter().collect())
}

fn next_letter(letter: char) -> char {
    match letter {
        'z' => 'a',
        other => char::from_u32(u32::from(other) + 1).unwrap_or('a'),
    }
}

fn at(word: &str, index: usize) -> char {
    word.chars().nth(index).unwrap_or('a')
}

fn misspell(word: &str) -> Option<String> {
    swap(word, 1).or_else(|| substitute(word, 1, next_letter(at(word, 1))))
}

fn is_plain(name: &str) -> bool {
    name.bytes().all(|byte| byte.is_ascii_lowercase())
}

fn replace_head(seed: &str, name: &str, written: &str) -> String {
    format!("{written}{}", &seed[name.len()..])
}

// Cases.

struct Case {
    class: &'static str,
    input: String,
    expect: Value,
    program_kind: &'static str,
    intent: &'static str,
    probe: Value,
    note: String,
}

fn literal_probe(stage: &str, place: &str, written: &str, corrected: &str, allowed: bool) -> Value {
    json!({ "check": "literal", "stage": stage, "place": place, "written": written, "corrected": corrected, "allowed": allowed })
}

fn name_probe(written: &str) -> Value {
    json!({ "check": "name", "kind": "command", "written": written })
}

fn unknown_command() -> Value {
    json!({ "error": "E_UNKNOWN_COMMAND" })
}

/// Stage, literal with `{}` for the misspelled name, literal place and the
/// rest of the program.
const LITERAL_TEMPLATES: [(&str, &str, &str, &str); 5] = [
    ("find", "/srv/app/{}", "path", ""),
    ("list", "/srv/{}/logs", "path", ""),
    ("read", "/srv/app/{}.txt", "path", ""),
    ("search", "{}", "text", " \"/srv/app\""),
    ("remove", "/srv/app/{}.log", "path", ""),
];

fn literal_cases(world: &World) -> Vec<Case> {
    world
        .commands
        .iter()
        .filter(|command| command.name.len() >= LITERAL_MIN_LEN && is_plain(&command.name))
        .filter_map(|command| misspell(&command.name).map(|typo| (command.name.clone(), typo)))
        .take(LITERAL_CASES)
        .enumerate()
        .map(|(index, (name, typo))| {
            let (stage, literal, place, tail) = LITERAL_TEMPLATES[index % LITERAL_TEMPLATES.len()];
            let written = literal.replace("{}", &typo);
            let corrected = literal.replace("{}", &name);
            Case {
                class: "literal",
                input: format!("{stage} \"{written}\"{tail}"),
                expect: json!({ "unchanged": true }),
                program_kind: world.kind(stage),
                intent: "unambiguous",
                probe: literal_probe(stage, place, &written, &corrected, false),
                note: format!("The literal contains `{typo}`, close to the command `{name}`; literals are never corrected."),
            }
        })
        .collect()
}

const ARROWS: [&str; 5] = ["->", "|->", "| >", "|", "=>"];

fn arrow_cases(world: &World) -> Vec<Case> {
    let mut cases = Vec::new();
    for arrow in ARROWS {
        let written = format!("old {arrow} new");
        let corrected = "old |> new";
        let templates = [
            ("search", format!("search \"{written}\" \"/srv/app\"")),
            (
                "replace",
                format!("search \"old\" \"/srv/app/src\" |> replace \"{written}\""),
            ),
        ];
        for (stage, input) in templates {
            cases.push(Case {
                class: "arrow",
                input,
                expect: json!({ "unchanged": true }),
                program_kind: world.kind(stage),
                intent: "unambiguous",
                probe: literal_probe(stage, "text", &written, corrected, false),
                note: format!("The operator `{arrow}` inside a text literal is part of the text."),
            });
        }
    }
    cases
}

fn tie_cases(world: &World) -> Vec<Case> {
    let mut cases = Vec::new();
    for command in &world.commands {
        if cases.len() == NAME_CASES {
            break;
        }
        if !TIE_NAME_LENS.contains(&command.name.len()) || !is_plain(&command.name) {
            continue;
        }
        let Some(seed) = world.seed(&command.name) else {
            continue;
        };
        let name = &command.name;
        let tie = (1..name.len())
            .flat_map(|index| {
                ALPHABET
                    .iter()
                    .filter_map(move |&letter| substitute(name, index, char::from(letter)))
            })
            .find_map(|written| {
                naive_tie(&written, &world.command_set).map(|names| (written, names))
            });
        if let Some((written, names)) = tie {
            let effect = names.iter().any(|name| world.effecting(name));
            cases.push(Case {
                class: "tie",
                input: replace_head(&seed, name, &written),
                expect: unknown_command(),
                program_kind: if effect { "effect" } else { "read" },
                intent: "ambiguous",
                probe: name_probe(&written),
                note: format!("The name is equally close to {}.", names.join(" and ")),
            });
        }
    }
    cases
}

fn short_variants(word: &str) -> Vec<String> {
    let length = word.chars().count();
    let swaps = (0..length).filter_map(|index| swap(word, index));
    let deletions = (0..length).filter_map(|index| delete(word, index));
    let substitutions = (1..length).flat_map(|index| {
        ['a', 'e', 'o', 'u']
            .into_iter()
            .filter_map(move |letter| substitute(word, index, letter))
    });
    let mut seen = BTreeSet::new();
    swaps
        .chain(deletions)
        .chain(substitutions)
        .filter(|variant| seen.insert(variant.clone()))
        .collect()
}

fn short_cases(world: &World) -> Vec<Case> {
    let mut cases = Vec::new();
    for command in world
        .commands
        .iter()
        .filter(|command| command.name.len() <= SHORT_NAME_LEN)
    {
        let Some(seed) = world.seed(&command.name) else {
            continue;
        };
        let variants = short_variants(&command.name)
            .into_iter()
            .filter(|variant| {
                variant.chars().count() > 1 && !naive_resolves(variant, &world.command_set)
            })
            .take(SHORT_VARIANTS);
        for written in variants {
            cases.push(Case {
                class: "short",
                input: replace_head(&seed, &command.name, &written),
                expect: unknown_command(),
                program_kind: world.kind(&command.name),
                intent: "unambiguous",
                probe: name_probe(&written),
                note: format!("A name of three characters or fewer is corrected only by an exact match; `{written}` is not `{}`.", command.name),
            });
        }
    }
    cases.truncate(NAME_CASES);
    cases
}

fn first_char_cases(world: &World) -> Vec<Case> {
    let mut cases = Vec::new();
    let names = world
        .commands
        .iter()
        .filter(|command| command.name.len() >= FIRST_CHAR_MIN_LEN && is_plain(&command.name));
    for (index, command) in names.enumerate() {
        if cases.len() == NAME_CASES {
            break;
        }
        let Some(seed) = world.seed(&command.name) else {
            continue;
        };
        let name = &command.name;
        let mut variants = [
            delete(name, 0),
            substitute(name, 0, next_letter(at(name, 0))),
        ];
        if index % 2 == 1 {
            variants.reverse();
        }
        let found = variants
            .into_iter()
            .flatten()
            .find(|variant| !naive_resolves(variant, &world.command_set));
        if let Some(written) = found {
            cases.push(Case {
                class: "first-char",
                input: replace_head(&seed, name, &written),
                expect: unknown_command(),
                program_kind: world.kind(name),
                intent: "unambiguous",
                probe: name_probe(&written),
                note: format!("A correction never changes the first character of a name; `{written}` is not `{name}`."),
            });
        }
    }
    cases
}

fn two_edits(name: &str) -> Vec<String> {
    let start = name.rfind('.').map_or(0, |dot| dot + 1);
    let last = name.chars().count() - 1;
    let swapped = swap(name, start + 1).and_then(|once| swap(&once, last - 1));
    let substituted = substitute(name, start + 1, next_letter(at(name, start + 1)))
        .and_then(|once| delete(&once, last));
    [swapped, substituted].into_iter().flatten().collect()
}

fn far_cases(world: &World) -> Vec<Case> {
    let mut cases = Vec::new();
    let names = world
        .commands
        .iter()
        .filter(|command| command.effecting && command.name.len() >= FAR_MIN_LEN);
    for command in names {
        if cases.len() == NAME_CASES {
            break;
        }
        let Some(seed) = world.seed(&command.name) else {
            continue;
        };
        let name = &command.name;
        let found = two_edits(name).into_iter().find(|variant| {
            naive_distance(variant, name) == 2
                && at(variant, 0) == at(name, 0)
                && !naive_resolves(variant, &world.command_set)
        });
        if let Some(written) = found {
            cases.push(Case {
                class: "effect-far",
                input: replace_head(&seed, name, &written),
                expect: unknown_command(),
                program_kind: "effect",
                intent: "unambiguous",
                probe: name_probe(&written),
                note: format!("A changing command is corrected only at distance 1; `{written}` is two edits from `{name}`."),
            });
        }
    }
    cases
}

const SIZE_NUMBERS: [u32; 10] = [10, 64, 2, 512, 100, 1, 250, 5, 20, 8];
const SIZE_LETTERS: [char; 4] = ['M', 'G', 'k', 'T'];

fn size_cases() -> Vec<Case> {
    SIZE_NUMBERS
        .iter()
        .enumerate()
        .map(|(index, number)| {
            let letter = SIZE_LETTERS[index % SIZE_LETTERS.len()];
            let lower = letter.to_ascii_lowercase();
            let written = format!("{number}{letter}");
            let binary = format!("{number}{lower}ib");
            let decimal = format!("{number}{lower}b");
            let program = |size: &str| format!("find \"/srv/app\" |> filter size > {size} |> remove");
            Case {
                class: "size-letter",
                input: program(&written),
                expect: json!({ "error": "E_AMBIGUOUS", "candidates": [program(&binary), program(&decimal)] }),
                program_kind: "effect",
                intent: "ambiguous",
                probe: json!({ "check": "size", "written": written, "to": binary, "alternative": decimal }),
                note: format!("The unit of `{written}` reads as {binary} or {decimal}; a changing program is not run."),
            }
        })
        .collect()
}

const SLASH_PATHS: [&str; 5] = ["/etc/", "/tmp/", "/srv/", "/opt/", "/var/"];

fn slash_cases() -> Vec<Case> {
    SLASH_PATHS
        .iter()
        .map(|path| {
            let trimmed = path.trim_end_matches('/');
            Case {
                class: "trailing-slash",
                input: format!("list {path}"),
                expect: json!({ "canonical": format!("list \"{trimmed}\""), "rules": ["L22", "L19"], "class": "S" }),
                program_kind: "read",
                intent: "unambiguous",
                probe: literal_probe("list", "path", path, trimmed, true),
                note: "A path written like a regular expression in a reading program is read as a path.".to_owned(),
            }
        })
        .collect()
}

const RELATIVE_PATHS: [&str; 10] = [
    "build/old.log",
    "src",
    "./logs",
    "notes.txt",
    "./tmp/cache",
    "data/input.csv",
    "docs/readme.md",
    "./out",
    "lib/util.rs",
    "reports",
];
const PATH_STAGES: [&str; 4] = ["find", "list", "read", "remove"];

fn relative_cases(world: &World) -> Vec<Case> {
    RELATIVE_PATHS
        .iter()
        .enumerate()
        .map(|(index, path)| {
            let stage = PATH_STAGES[index % PATH_STAGES.len()];
            let resolved = format!("{BASE}/{}", path.trim_start_matches("./"));
            Case {
                class: "relative",
                input: format!("{stage} \"{path}\""),
                expect: json!({ "error": "E_PATH_RELATIVE" }),
                program_kind: world.kind(stage),
                intent: "unambiguous",
                probe: json!({ "check": "relative", "stage": stage, "written": path, "resolved": resolved }),
                note: "A relative path without an explicit base is not resolved.".to_owned(),
            }
        })
        .collect()
}

const CASED_PATHS: [&str; 15] = [
    "/Srv/App",
    "/home/User/Documents",
    "C:/Users/Ana/notes.txt",
    "/srv/App/Build",
    "/opt/Tools/bin",
    "/Volumes/Data/photos",
    "/srv/data/README.md",
    "C:/Program Files/App/config.ini",
    "/srv/app/Makefile",
    "/home/ana/Downloads",
    "/srv/www/Index.html",
    "/etc/X11/xorg.conf",
    "/srv/app/src/Main.java",
    "D:/Backup/2024",
    "/mnt/Share/docs",
];

fn case_cases(world: &World) -> Vec<Case> {
    CASED_PATHS
        .iter()
        .enumerate()
        .map(|(index, path)| {
            let stage = PATH_STAGES[index % PATH_STAGES.len()];
            Case {
                class: "path-case",
                input: format!("{stage} \"{path}\""),
                expect: json!({ "unchanged": true }),
                program_kind: world.kind(stage),
                intent: "unambiguous",
                probe: literal_probe(stage, "path", path, &path.to_lowercase(), false),
                note: "The case of path segments is never changed.".to_owned(),
            }
        })
        .collect()
}

/// Program with `{}` for a number in an integer place, and the number.
const NUMBER_TEMPLATES: [(&str, &str); 10] = [
    ("list \"/srv/app\" |> first {}", "1.5"),
    ("list \"/srv/app\" |> first {}", "2.25"),
    ("list \"/srv/app\" |> first {}", "1.000"),
    ("list \"/srv/app\" |> first {}", "3.000"),
    ("list \"/srv/app\" |> first {}", "10.000"),
    ("list \"/srv/app\" |> skip {}", "0.5"),
    ("list \"/srv/app\" |> skip {}", "2.000"),
    ("list \"/srv/app\" |> last {}", "2.5"),
    ("find \"/srv/app\" depth: {}", "1.5"),
    ("find \"/srv/app\" depth: {}", "2.000"),
];
const CALENDAR_DURATIONS: [&str; 5] = ["2 months", "1 year", "3 months", "1 month", "2 years"];

fn number_cases() -> Vec<Case> {
    let numbers = NUMBER_TEMPLATES.iter().map(|(template, number)| Case {
        class: "number",
        input: template.replace("{}", number),
        expect: json!({ "error": "E_TYPE" }),
        program_kind: "read",
        intent: "unambiguous",
        probe: json!({ "check": "number", "written": number }),
        note: format!("The number {number} is not an integer and is not rounded."),
    });
    let durations = CALENDAR_DURATIONS.iter().map(|duration| Case {
        class: "number",
        input: format!("run [\"make\"] cwd: \"/srv/app\" timeout: \"{duration}\""),
        expect: json!({ "error": "E_UNCORRECTABLE" }),
        program_kind: "effect",
        intent: "unambiguous",
        probe: json!({ "check": "duration", "written": duration }),
        note: "Months and years have no fixed length and are not converted.".to_owned(),
    });
    numbers.chain(durations).collect()
}

/// Command, mode parameter and program with `{}` for the value.
const MODE_TEMPLATES: [(&str, &str, &str); 6] = [
    (
        "write",
        "mode",
        "write \"/srv/app/notes.txt\" \"x\" mode: {}",
    ),
    (
        "copy",
        "on_conflict",
        "copy \"/srv/app\" into: \"/srv/backup\" on_conflict: {}",
    ),
    (
        "archive.extract",
        "on_conflict",
        "archive.extract \"/srv/dl/data.tar.gz\" into: \"/srv/data\" on_conflict: {}",
    ),
    ("create", "type", "create \"/srv/app/cache\" type: {}"),
    (
        "git.reset",
        "mode",
        "git.reset \"/srv/app\" revision: \"HEAD~1\" mode: {}",
    ),
    ("git.pull", "mode", "git.pull \"/srv/app\" mode: {}"),
];

/// Shortest prefix of `value` that selects it uniquely among `values` and
/// that the naive rules do not resolve.
fn mode_prefix(value: &str, values: &[String], set: &[Candidate]) -> Option<String> {
    (MIN_PREFIX..value.len().saturating_sub(1))
        .map(|length| value[..length].to_owned())
        .find(|prefix| {
            values
                .iter()
                .filter(|other| other.starts_with(prefix.as_str()))
                .count()
                == 1
                && !naive_resolves(prefix, set)
        })
}

fn mode_cases(world: &World) -> Vec<Case> {
    let mut cases = Vec::new();
    for (command, param, template) in MODE_TEMPLATES {
        let values = world.values(command, param);
        let set = world.value_set(&values);
        for value in &values {
            let Some(prefix) = mode_prefix(value, &values, &set) else {
                continue;
            };
            cases.push(Case {
                class: "mode-prefix",
                input: template.replace("{}", &prefix),
                expect: json!({ "error": "E_UNKNOWN_VALUE" }),
                program_kind: "effect",
                intent: "unambiguous",
                probe: json!({ "check": "mode", "written": prefix, "in": format!("{command}.{param}") }),
                note: format!("A prefix never selects the mode of an effect; `{prefix}` is not expanded to `{value}`."),
            });
        }
    }
    cases
}

fn generate(world: &World) -> String {
    let groups = [
        literal_cases(world),
        arrow_cases(world),
        tie_cases(world),
        short_cases(world),
        first_char_cases(world),
        far_cases(world),
        size_cases(),
        slash_cases(),
        relative_cases(world),
        case_cases(world),
        number_cases(),
        mode_cases(world),
    ];
    let mut counters: BTreeMap<&str, usize> = BTreeMap::new();
    let mut seen = BTreeSet::new();
    let mut lines: Vec<(String, String)> = Vec::new();
    for case in groups.into_iter().flatten() {
        if !seen.insert(case.input.clone()) {
            continue;
        }
        let counter = counters.entry(case.class).or_default();
        *counter += 1;
        let id = format!("adversarial-{}-{:04}", case.class, counter);
        let line = json!({
            "id": id,
            "input": case.input,
            "variant": "a",
            "expect": case.expect,
            "program_kind": case.program_kind,
            "intent": case.intent,
            "corpus": "adversarial",
            "source": "generated",
            "probe": case.probe,
            "note": case.note,
        });
        lines.push((id, serde_json::to_string(&line).unwrap()));
    }
    lines.sort();
    lines.into_iter().map(|(_, line)| line + "\n").collect()
}

#[test]
fn generated_corpus_matches_the_repository() {
    let world = World::new();
    let generated = generate(&world);
    let path = root().join(CORPUS);
    if std::env::var(REGENERATE).is_ok_and(|value| value == "1") {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &generated).unwrap();
    }
    let stored = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(
        stored == generated,
        "{CORPUS} is out of date; run the test with {REGENERATE}=1"
    );
    let count = generated.lines().count();
    assert!((MIN_CASES..=MAX_CASES).contains(&count), "{count} cases");
}

// Component check of every case.

#[derive(Debug, PartialEq, Eq)]
enum Verdict {
    Kept,
    Ambiguous,
    FalseCorrection(String),
    Mismatch(String),
}

fn text<'a>(probe: &'a Value, member: &str) -> &'a str {
    probe[member].as_str().unwrap_or_default()
}

fn surface(stage: &str, literal: &str) -> AstDocument {
    serde_json::from_value(json!({
        "nurt": "1.0",
        "program": { "statements": [{
            "t": "pipeline",
            "head": { "t": "stage", "name": stage, "args": [{ "value": { "t": "text", "v": literal } }] },
            "stages": []
        }] }
    }))
    .unwrap()
}

fn bound(world: &World, stage: &str, place: &str, literal: &str) -> BoundDocument {
    let (effect, phase) = if world.effecting(stage) {
        ("local", "mutate")
    } else {
        ("read", "source")
    };
    let arg = if place == "path" { "path" } else { "pattern" };
    serde_json::from_value(json!({
        "nurt": "1.0",
        "bound": "1.0",
        "catalog_rev": "cat-1",
        "corr_rev": "corr-1",
        "program": { "statements": [{
            "t": "pipeline",
            "head": {
                "t": "stage",
                "name": stage,
                "effect": effect,
                "phase": phase,
                "args": [{ "name": arg, "value": { "t": place, "v": literal }, "default": false }]
            },
            "stages": []
        }] }
    }))
    .unwrap()
}

fn invariants(
    world: &World,
    before: &AstDocument,
    after: &BoundDocument,
    base: Option<&str>,
) -> Result<(), InvariantViolation> {
    let effecting = |name: &str| world.effecting(name);
    check_invariants(&InvariantInput {
        before: Some(before),
        after,
        corrections: &[],
        base,
        tables: &world.tables,
        effecting: &effecting,
    })
}

fn literal_check(world: &World, probe: &Value) -> Verdict {
    let (stage, place) = (text(probe, "stage"), text(probe, "place"));
    let (written, corrected) = (text(probe, "written"), text(probe, "corrected"));
    let before = surface(stage, written);
    let rewritten = invariants(world, &before, &bound(world, stage, place, corrected), None);
    if probe["allowed"].as_bool() == Some(true) {
        return match rewritten {
            Ok(()) => Verdict::Kept,
            Err(violation) => Verdict::Mismatch(format!("allowed rewrite rejected: {violation}")),
        };
    }
    if let Err(violation) = invariants(world, &before, &bound(world, stage, place, written), None) {
        return Verdict::Mismatch(format!("unchanged literal rejected: {violation}"));
    }
    match rewritten {
        Err(violation) if violation.invariant == Invariant::LiteralBytes => Verdict::Kept,
        Err(violation) => Verdict::Mismatch(format!("unexpected invariant: {violation}")),
        Ok(()) => Verdict::FalseCorrection(format!(
            "`{written}` -> `{corrected}` passed the invariants"
        )),
    }
}

fn relative_check(world: &World, probe: &Value) -> Verdict {
    let stage = text(probe, "stage");
    let before = surface(stage, text(probe, "written"));
    let after = bound(world, stage, "path", text(probe, "resolved"));
    if invariants(world, &before, &after, Some(BASE)).is_err() {
        return Verdict::Mismatch("resolution against the explicit base rejected".to_owned());
    }
    match invariants(world, &before, &after, None) {
        Err(violation) if violation.invariant == Invariant::Base => Verdict::Kept,
        Err(violation) => Verdict::Mismatch(format!("unexpected invariant: {violation}")),
        Ok(()) => Verdict::FalseCorrection("a base was chosen".to_owned()),
    }
}

fn name_check(world: &World, probe: &Value, program: ProgramKind) -> Verdict {
    let written = text(probe, "written");
    match values::resolve(
        written,
        NameKind::Command,
        &world.command_index,
        &world.tables,
        &Options::default(),
    ) {
        Match::Tie(_) | Match::Unknown => Verdict::Kept,
        Match::Same => Verdict::Mismatch(format!("`{written}` is a command")),
        Match::Corrected(found) => {
            let facts = Facts::default().fuzzy(found.effecting);
            match decide(found.class, program, &facts) {
                Decision::Ambiguous => Verdict::Ambiguous,
                _ => Verdict::FalseCorrection(format!("`{written}` -> `{}`", found.to)),
            }
        }
    }
}

fn mode_check(world: &World, probe: &Value) -> Verdict {
    let written = text(probe, "written");
    let Some((command, param)) = text(probe, "in").rsplit_once('.') else {
        return Verdict::Mismatch("probe without a parameter".to_owned());
    };
    let allowed = world.values(command, param);
    let allowed: Vec<&str> = allowed.iter().map(String::as_str).collect();
    match values::mode_value(written, &allowed, &world.tables, &Options::default()) {
        Match::Corrected(found) => {
            Verdict::FalseCorrection(format!("`{written}` -> `{}`", found.to))
        }
        _ => Verdict::Kept,
    }
}

fn size_check(world: &World, probe: &Value) -> Verdict {
    let written = text(probe, "written");
    let Outcome::Converted(converted) = types::size(Written::bare(written), &world.tables) else {
        return Verdict::Mismatch(format!("`{written}` is not read as a size"));
    };
    let alternative = converted
        .alternatives
        .first()
        .map(|alternative| alternative.text.as_str());
    if converted.to != text(probe, "to") || alternative != Some(text(probe, "alternative")) {
        return Verdict::Mismatch(format!(
            "`{written}` reads as {} or {alternative:?}",
            converted.to
        ));
    }
    match decide(converted.class, ProgramKind::Effect, &Facts::default()) {
        Decision::Ambiguous if converted.class == Class::Probable => Verdict::Ambiguous,
        _ => Verdict::FalseCorrection(format!(
            "`{written}` -> `{}` applied in a changing program",
            converted.to
        )),
    }
}

fn number_check(probe: &Value) -> Verdict {
    let written = text(probe, "written");
    match types::integer(Written::bare(written)) {
        Outcome::Converted(converted) => {
            Verdict::FalseCorrection(format!("`{written}` -> `{}`", converted.to))
        }
        _ => Verdict::Kept,
    }
}

fn duration_check(world: &World, probe: &Value) -> Verdict {
    let written = text(probe, "written");
    match types::duration(Written::quoted(written), &world.tables) {
        Outcome::Uncorrectable(_) => Verdict::Kept,
        Outcome::Converted(converted) => {
            Verdict::FalseCorrection(format!("`{written}` -> `{}`", converted.to))
        }
        Outcome::NotApplicable => {
            Verdict::Mismatch(format!("`{written}` is not recognized as a duration"))
        }
    }
}

fn check_case(world: &World, case: &Value) -> Verdict {
    let probe = &case["probe"];
    let program = if case["program_kind"] == "effect" {
        ProgramKind::Effect
    } else {
        ProgramKind::Read
    };
    match probe["check"].as_str() {
        Some("literal") => literal_check(world, probe),
        Some("relative") => relative_check(world, probe),
        Some("name") => name_check(world, probe, program),
        Some("mode") => mode_check(world, probe),
        Some("size") => size_check(world, probe),
        Some("number") => number_check(probe),
        Some("duration") => duration_check(world, probe),
        other => Verdict::Mismatch(format!("unknown probe {other:?}")),
    }
}

#[derive(Debug, Default)]
struct Tally {
    kept: usize,
    ambiguous: usize,
    false_corrections: Vec<String>,
    mismatches: Vec<String>,
}

#[test]
fn adversarial_cases_are_never_corrected() {
    let world = World::new();
    let mut tally = Tally::default();
    for line in generate(&world).lines() {
        let case: Value = serde_json::from_str(line).unwrap();
        let id = text(&case, "id").to_owned();
        let expects_ambiguity = case["expect"]["error"] == "E_AMBIGUOUS";
        match check_case(&world, &case) {
            Verdict::Kept if !expects_ambiguity => tally.kept += 1,
            Verdict::Ambiguous if expects_ambiguity => tally.ambiguous += 1,
            Verdict::FalseCorrection(detail) => {
                tally.false_corrections.push(format!("{id}: {detail}"))
            }
            Verdict::Mismatch(detail) => tally.mismatches.push(format!("{id}: {detail}")),
            other => tally
                .mismatches
                .push(format!("{id}: {other:?} against {}", case["expect"])),
        }
    }
    eprintln!(
        "adversarial component check: {} kept, {} ambiguous, {} false corrections",
        tally.kept,
        tally.ambiguous,
        tally.false_corrections.len()
    );
    assert!(
        tally.false_corrections.is_empty(),
        "{:#?}",
        tally.false_corrections
    );
    assert!(tally.mismatches.is_empty(), "{:#?}", tally.mismatches);
}

// Measures and format of every corpus file.

fn corpus_files(directory: &Path, files: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            corpus_files(&path, files);
        } else if path.extension().is_some_and(|ext| ext == "jsonl") {
            files.push(path);
        }
    }
}

fn all_cases() -> Vec<(PathBuf, usize, String)> {
    let mut files = Vec::new();
    corpus_files(&root().join(CORPORA), &mut files);
    files.sort();
    files
        .into_iter()
        .flat_map(|path| {
            let text = std::fs::read_to_string(&path).unwrap();
            text.lines()
                .enumerate()
                .map(|(index, line)| (path.clone(), index + 1, line.to_owned()))
                .collect::<Vec<_>>()
        })
        .collect()
}

/// Share of `E_AMBIGUOUS` among changing programs with an unambiguous intent.
fn amb(cases: &[Value]) -> (usize, usize) {
    let counted: Vec<&Value> = cases
        .iter()
        .filter(|case| case["program_kind"] == "effect" && case["intent"] == "unambiguous")
        .collect();
    let ambiguous = counted
        .iter()
        .filter(|case| case["expect"]["error"] == "E_AMBIGUOUS")
        .count();
    (ambiguous, counted.len())
}

fn share((part, whole): (usize, usize)) -> f64 {
    if whole == 0 {
        0.0
    } else {
        part as f64 / whole as f64
    }
}

#[test]
fn ambiguity_stays_within_the_limit() {
    let world = World::new();
    let generated: Vec<Value> = generate(&world)
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let adversarial: Vec<Value> = all_cases()
        .iter()
        .filter_map(|(_, _, line)| serde_json::from_str::<Value>(line).ok())
        .filter(|case| case["corpus"] == "adversarial")
        .collect();
    let own = amb(&generated);
    let all = amb(&adversarial);
    eprintln!(
        "AMB: generated {}/{} ({:.1}%), all adversarial files {}/{} ({:.1}%), {} adversarial cases",
        own.0,
        own.1,
        100.0 * share(own),
        all.0,
        all.1,
        100.0 * share(all),
        adversarial.len()
    );
    assert!(share(own) <= MAX_AMB, "{own:?}");
}

fn is_id(id: &str) -> bool {
    !id.is_empty()
        && id.split('-').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        })
}

fn texts_problem(object: &Map<String, Value>, member: &str, limit: usize) -> Option<String> {
    let value = object.get(member)?;
    let valid = value
        .as_array()
        .is_some_and(|items| items.len() <= limit && items.iter().all(Value::is_string));
    (!valid).then(|| format!("`{member}` is not a list of at most {limit} texts"))
}

fn unknown_members(object: &Map<String, Value>, allowed: &[&str], within: &str) -> Vec<String> {
    object
        .keys()
        .filter(|key| !allowed.contains(&key.as_str()))
        .map(|key| format!("unknown member `{key}` in {within}"))
        .collect()
}

fn canonical_problems(object: &Map<String, Value>) -> Vec<String> {
    let mut problems = unknown_members(
        object,
        &["canonical", "rules", "class", "alternatives"],
        "expect",
    );
    if !object["canonical"].is_string() {
        problems.push("`canonical` is not a text".to_owned());
    }
    let rules_valid = object
        .get("rules")
        .and_then(Value::as_array)
        .is_some_and(|rules| {
            !rules.is_empty()
                && rules.iter().all(|rule| {
                    rule.as_str()
                        .is_some_and(|rule| rule.parse::<RuleId>().is_ok())
                })
        });
    if !rules_valid {
        problems.push("`rules` is not a non-empty list of rule identifiers".to_owned());
    }
    if !object
        .get("class")
        .and_then(Value::as_str)
        .is_some_and(|class| CLASSES.contains(&class))
    {
        problems.push("`class` is not one of C, S, F, P".to_owned());
    }
    problems.extend(texts_problem(object, "alternatives", MAX_LISTED));
    problems
}

fn error_problems(object: &Map<String, Value>) -> Vec<String> {
    let mut problems = unknown_members(
        object,
        &["error", "candidates", "fix", "next_steps"],
        "expect",
    );
    if !object["error"]
        .as_str()
        .is_some_and(|code| code.parse::<ErrorCode>().is_ok())
    {
        problems.push(format!("unknown error code {}", object["error"]));
    }
    problems.extend(texts_problem(object, "candidates", MAX_LISTED));
    problems.extend(texts_problem(object, "fix", MAX_LISTED));
    problems.extend(texts_problem(object, "next_steps", usize::MAX));
    problems
}

fn expect_problems(expect: &Value) -> Vec<String> {
    let Some(object) = expect.as_object() else {
        return vec!["`expect` is not an object".to_owned()];
    };
    let forms = ["canonical", "unchanged", "error"]
        .iter()
        .filter(|form| object.contains_key(**form))
        .count();
    if forms != 1 {
        return vec![format!("`expect` has {forms} forms")];
    }
    if object.contains_key("canonical") {
        canonical_problems(object)
    } else if object.contains_key("error") {
        error_problems(object)
    } else if object.len() == 1 && object["unchanged"] == true {
        Vec::new()
    } else {
        vec!["`unchanged` is not the only member or is not true".to_owned()]
    }
}

fn context_problems(context: Option<&Value>) -> Vec<String> {
    match context {
        None => Vec::new(),
        Some(Value::Object(object)) => {
            let mut problems = unknown_members(object, &CONTEXT_MEMBERS, "context");
            if object
                .get("os")
                .is_some_and(|os| !os.as_str().is_some_and(|os| OS_NAMES.contains(&os)))
            {
                problems.push("`os` is not linux, macos or windows".to_owned());
            }
            problems
        }
        Some(_) => vec!["`context` is not an object".to_owned()],
    }
}

fn member_problems(object: &Map<String, Value>) -> Vec<String> {
    let mut problems = unknown_members(object, &MEMBERS, "the case");
    problems.extend(
        REQUIRED
            .iter()
            .filter(|member| !object.contains_key(**member))
            .map(|member| format!("missing member `{member}`")),
    );
    for (member, allowed) in ENUM_MEMBERS {
        if object
            .get(member)
            .is_some_and(|value| !value.as_str().is_some_and(|value| allowed.contains(&value)))
        {
            problems.push(format!("`{member}` is not one of {allowed:?}"));
        }
    }
    for member in ["input", "note"] {
        if object.get(member).is_some_and(|value| !value.is_string()) {
            problems.push(format!("`{member}` is not a text"));
        }
    }
    problems
}

fn case_problems(case: &Value, in_adversarial: bool) -> Vec<String> {
    let Some(object) = case.as_object() else {
        return vec!["the case is not an object".to_owned()];
    };
    let mut problems = member_problems(object);
    if !object.get("id").and_then(Value::as_str).is_some_and(is_id) {
        problems.push("`id` is not lower case words joined by `-`".to_owned());
    }
    problems.extend(context_problems(object.get("context")));
    problems.extend(expect_problems(&case["expect"]));
    if in_adversarial && case["corpus"] != "adversarial" {
        problems.push(
            "a case of an adversarial file is not marked `\"corpus\": \"adversarial\"`".to_owned(),
        );
    }
    problems
}

#[test]
fn every_corpus_file_follows_the_format() {
    let mut ids = BTreeSet::new();
    let mut problems = Vec::new();
    let cases = all_cases();
    for (path, line, text) in &cases {
        let place = format!(
            "{}:{line}",
            path.strip_prefix(root()).unwrap_or(path).display()
        );
        let in_adversarial = path
            .components()
            .any(|component| component.as_os_str() == "adversarial");
        match serde_json::from_str::<Value>(text) {
            Ok(case) => {
                problems.extend(
                    case_problems(&case, in_adversarial)
                        .into_iter()
                        .map(|problem| format!("{place}: {problem}")),
                );
                if let Some(id) = case["id"].as_str()
                    && !ids.insert(id.to_owned())
                {
                    problems.push(format!("{place}: duplicate id `{id}`"));
                }
            }
            Err(error) => problems.push(format!("{place}: {error}")),
        }
    }
    assert!(!cases.is_empty());
    assert!(problems.is_empty(), "{problems:#?}");
}

#[test]
fn naive_rules_follow_the_documented_examples() {
    let world = World::new();
    let tie = naive_tie("lest", &world.command_set).unwrap_or_default();
    assert!(
        tie.contains(&"last".to_owned()) && tie.contains(&"list".to_owned()),
        "{tie:?}"
    );
    assert!(naive_resolves("remvoe", &world.command_set));
    assert!(!naive_resolves("rnu", &world.command_set));
    assert!(!naive_resolves("emove", &world.command_set));
    assert_eq!(two_edits("compress"), ["cpmpres"]);
    assert_eq!(two_edits("git.commit"), ["git.cmomti", "git.cpmmi"]);
    assert_eq!(swap("git.push", 2), None);
    assert!(is_id("adversarial-tie-0001"));
    assert!(!is_id("Adversarial--1"));
}

#[test]
fn manual_cases_are_never_corrected() {
    let world = World::new();
    let stored = std::fs::read_to_string(root().join(MANUAL)).unwrap();
    let mut problems = Vec::new();
    let mut count = 0;
    for line in stored.lines() {
        let case: Value = serde_json::from_str(line).unwrap();
        let id = text(&case, "id").to_owned();
        let expects_ambiguity = case["expect"]["error"] == "E_AMBIGUOUS";
        count += 1;
        match check_case(&world, &case) {
            Verdict::Kept if !expects_ambiguity => {}
            Verdict::Ambiguous if expects_ambiguity => {}
            other => problems.push(format!("{id}: {other:?} against {}", case["expect"])),
        }
        if case["source"] != "manual" || case["probe"].is_null() {
            problems.push(format!("{id}: a manual case without a probe"));
        }
    }
    assert!(count > 0);
    assert!(problems.is_empty(), "{problems:#?}");
}

#[test]
fn adversarial_corpus_has_at_least_the_required_cases() {
    let adversarial = all_cases()
        .iter()
        .filter_map(|(_, _, line)| serde_json::from_str::<Value>(line).ok())
        .filter(|case| case["corpus"] == "adversarial")
        .count();
    assert!(
        adversarial >= MIN_ADVERSARIAL,
        "{adversarial} adversarial cases"
    );
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
