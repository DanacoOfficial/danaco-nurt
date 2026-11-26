// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Mutation corpus: generation from valid programs and the check of name,
//! unit and boolean correction against it.
//!
//! The corpus is generated from the golden programs and from one-command
//! programs of the catalog. The expected reaction of every case follows
//! from the mutation and from a naive implementation of the acceptance
//! rules written here, independent of the crate. The file in the
//! repository must equal a fresh generation; set `NURT_REGENERATE_CORPUS=1`
//! to rewrite it.

#![allow(
    clippy::unwrap_used,
    reason = "helpers of the test read repository fixtures; a malformed fixture fails the test"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use nurt_correct::distance::{NameEntry, NameKind, NameRole, Options};
use nurt_correct::policy::Class;
use nurt_correct::tables::{Tables, load_embedded};
use nurt_correct::types::{self, Outcome, Written};
use nurt_correct::values::{self, Match};
use serde_json::{Value, json};
use toml_edit::DocumentMut;

const CORPUS: &str = "tests/d1/correction/mutate/generated.jsonl";
const REGENERATE: &str = "NURT_REGENERATE_CORPUS";
const SEED: u64 = 0x6e75_7274_7538_0001;
const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz";
const MODE_PARAMS: [&str; 3] = ["mode", "on_conflict", "recursive"];
const MIN_PREFIX: usize = 3;

/// Largest number of cases of one mutation operator; cases of golden
/// programs come first.
const MAX_PER_OPERATOR: usize = 80;

struct Command {
    name: String,
    effecting: bool,
    params: Vec<(String, Vec<String>, String)>,
}

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
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

fn strings(item: Option<&toml_edit::Item>) -> Vec<String> {
    item.and_then(|item| item.as_array())
        .map(|array| {
            array
                .iter()
                .filter_map(|value| value.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
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
                    let required = param
                        .get("required")
                        .and_then(|item| item.as_bool())
                        .unwrap_or(false);
                    let positional = param.get("position").is_some();
                    let shape = format!(
                        "{}{}{}",
                        param
                            .get("type")
                            .and_then(|item| item.as_str())
                            .unwrap_or_default(),
                        if required { " required" } else { "" },
                        if positional { " positional" } else { "" }
                    );
                    (
                        param["name"].as_str().unwrap().to_owned(),
                        strings(param.get("values")),
                        shape,
                    )
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

/// One-command programs of the catalog: the command alone, or with a path
/// when its only required parameter is a positional path.
fn catalog_seeds(commands: &[Command]) -> Vec<(String, String)> {
    commands
        .iter()
        .filter_map(|command| {
            let required: Vec<&(String, Vec<String>, String)> = command
                .params
                .iter()
                .filter(|(_, _, shape)| shape.contains("required"))
                .collect();
            let program = match required.as_slice() {
                [] => command.name.clone(),
                [(_, _, shape)] if shape.starts_with("path") && shape.contains("positional") => {
                    format!("{} \"/srv/app/data\"", command.name)
                }
                _ => return None,
            };
            Some((format!("catalog-{}", command.name), program))
        })
        .collect()
}

/// Golden programs of variant A; the files of variants B and C are named
/// `<stem>.b.nurt` and `<stem>.c.nurt`.
fn golden_seeds() -> Vec<(String, String)> {
    sorted_files(&root().join("spec/golden/ast"), "nurt")
        .iter()
        .filter_map(|path| {
            let stem = path.file_stem().unwrap().to_string_lossy().into_owned();
            (!stem.contains('.')).then(|| {
                (
                    stem,
                    std::fs::read_to_string(path).unwrap().trim_end().to_owned(),
                )
            })
        })
        .collect()
}

// Deterministic pseudo-random numbers (SplitMix64).

struct Random(u64);

impl Random {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    fn below(&mut self, bound: usize) -> usize {
        usize::try_from(self.next() % u64::try_from(bound.max(1)).unwrap()).unwrap()
    }

    fn letter(&mut self) -> char {
        char::from(ALPHABET[self.below(ALPHABET.len())])
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum TokenKind {
    Command,
    Param { command: String },
    Value { command: String, param: String },
    Bool,
    Size,
    Text,
    Pipe,
}

#[derive(Debug, Clone)]
struct Token {
    kind: TokenKind,
    start: usize,
    end: usize,
}

struct Scanner<'a> {
    text: &'a str,
    bytes: &'a [u8],
    position: usize,
    tokens: Vec<Token>,
    expect_command: bool,
    commands: Vec<String>,
    braces: usize,
    last_param: Option<(String, usize)>,
}

fn is_word(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'.'
}

impl<'a> Scanner<'a> {
    fn scan(text: &'a str) -> Vec<Token> {
        let mut scanner = Self {
            text,
            bytes: text.as_bytes(),
            position: 0,
            tokens: Vec::new(),
            expect_command: true,
            commands: vec![String::new()],
            braces: 0,
            last_param: None,
        };
        while scanner.position < scanner.bytes.len() {
            scanner.step();
        }
        scanner.tokens
    }

    fn peek(&self, offset: usize) -> u8 {
        self.bytes.get(self.position + offset).copied().unwrap_or(0)
    }

    fn step(&mut self) {
        let byte = self.peek(0);
        match byte {
            b'"' => self.string(),
            b'r' if self.peek(1) == b'#' && self.peek(2) == b'"' => self.raw_string(),
            b'/' => self.regex(),
            b'|' if self.peek(1) == b'>' => self.pipe(),
            b'\n' => self.newline(),
            b'(' | b'[' | b'{' => self.open(byte),
            b')' | b']' | b'}' => self.close(byte),
            b'@' => self.reference(),
            _ if byte.is_ascii_digit() => self.number(),
            _ if byte.is_ascii_alphabetic() || byte == b'_' => self.word(),
            _ => self.position += 1,
        }
    }

    fn push(&mut self, kind: TokenKind, start: usize, end: usize) {
        self.tokens.push(Token { kind, start, end });
    }

    fn string(&mut self) {
        let start = self.position;
        self.position += 1;
        while self.position < self.bytes.len() && self.peek(0) != b'"' {
            self.position += if self.peek(0) == b'\\' { 2 } else { 1 };
        }
        self.position += 1;
        self.push(TokenKind::Text, start, self.position);
        self.expect_command = false;
    }

    fn raw_string(&mut self) {
        let end = self.text[self.position + 3..]
            .find("\"#")
            .map_or(self.bytes.len(), |index| self.position + 3 + index + 2);
        self.position = end;
        self.expect_command = false;
    }

    fn regex(&mut self) {
        self.position += 1;
        while self.position < self.bytes.len() && self.peek(0) != b'/' {
            self.position += if self.peek(0) == b'\\' { 2 } else { 1 };
        }
        self.position += 1;
        while self.peek(0).is_ascii_alphabetic() {
            self.position += 1;
        }
        self.expect_command = false;
    }

    fn pipe(&mut self) {
        self.push(TokenKind::Pipe, self.position, self.position + 2);
        self.position += 2;
        self.expect_command = true;
    }

    fn newline(&mut self) {
        self.position += 1;
        let rest = self.text[self.position..].trim_start();
        if !rest.starts_with("|>") && self.braces == 0 && self.commands.len() == 1 {
            self.expect_command = true;
        }
    }

    fn open(&mut self, byte: u8) {
        self.position += 1;
        if byte == b'(' {
            self.commands.push(String::new());
            self.expect_command = true;
        } else if byte == b'{' {
            self.braces += 1;
        }
    }

    fn close(&mut self, byte: u8) {
        self.position += 1;
        if byte == b')' && self.commands.len() > 1 {
            self.commands.pop();
        } else if byte == b'}' {
            self.braces = self.braces.saturating_sub(1);
        }
        self.expect_command = false;
    }

    fn reference(&mut self) {
        self.position += 1;
        while is_word(self.peek(0)) {
            self.position += 1;
        }
        self.expect_command = false;
    }

    fn number(&mut self) {
        let start = self.position;
        while is_word(self.peek(0)) {
            self.position += 1;
        }
        let token = &self.text[start..self.position];
        let unit_start = token.find(|c: char| c.is_ascii_alphabetic());
        if unit_start.is_some_and(|index| ["b", "kb", "mb", "gb", "tb"].contains(&&token[index..]))
        {
            self.push(TokenKind::Size, start, self.position);
        }
        self.expect_command = false;
    }

    fn word(&mut self) {
        let start = self.position;
        while is_word(self.peek(0)) {
            self.position += 1;
        }
        let word = self.text[start..self.position].to_owned();
        if word == "let" && self.expect_command {
            self.skip_let();
            return;
        }
        if self.expect_command {
            self.push(TokenKind::Command, start, self.position);
            if let Some(current) = self.commands.last_mut() {
                *current = word;
            }
            self.expect_command = false;
            return;
        }
        self.argument_word(word, start);
    }

    fn skip_let(&mut self) {
        while self.position < self.bytes.len() && self.peek(0) != b'=' {
            self.position += 1;
        }
        self.position += 1;
        self.expect_command = true;
    }

    fn argument_word(&mut self, word: String, start: usize) {
        let command = self.commands.last().cloned().unwrap_or_default();
        if self.peek(0) == b':' && self.braces == 0 {
            self.push(TokenKind::Param { command }, start, self.position);
            self.last_param = Some((word, self.position + 1));
            return;
        }
        let Some((param, after)) = self.last_param.take() else {
            return;
        };
        if self.text[after..start].trim().is_empty() && self.braces == 0 {
            let kind = if word == "true" || word == "false" {
                TokenKind::Bool
            } else {
                TokenKind::Value { command, param }
            };
            self.push(kind, start, self.position);
        }
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

#[derive(Debug, Clone)]
struct Candidate {
    text: String,
    target: Option<String>,
    name: bool,
    effecting: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Verdict {
    Fuzzy(String),
    Prefix(String),
    Other,
    None,
}

fn naive_fuzzy(written: &str, set: &[Candidate]) -> Option<Verdict> {
    let distances: Vec<usize> = set
        .iter()
        .map(|candidate| naive_distance(written, &candidate.text))
        .collect();
    let best = *distances.iter().min()?;
    let winners: Vec<&Candidate> = set
        .iter()
        .zip(&distances)
        .filter(|(_, d)| **d == best)
        .map(|(c, _)| c)
        .collect();
    let texts: BTreeSet<&str> = winners.iter().map(|winner| winner.text.as_str()).collect();
    let name = winners.iter().find(|winner| winner.name)?;
    if texts.len() > 1 {
        return Some(Verdict::None);
    }
    let accepted = best <= naive_threshold(name.text.chars().count())
        && written.chars().next() == name.text.chars().next()
        && !(name.effecting && best > 1);
    Some(if accepted {
        Verdict::Fuzzy(name.text.clone())
    } else {
        Verdict::None
    })
}

fn naive_prefix(written: &str, set: &[Candidate]) -> Option<String> {
    if written.chars().count() < MIN_PREFIX {
        return None;
    }
    let targets: BTreeSet<String> = set
        .iter()
        .filter(|candidate| candidate.name || candidate.target.is_some())
        .filter(|candidate| candidate.text.starts_with(written))
        .map(|candidate| {
            candidate
                .target
                .clone()
                .unwrap_or_else(|| candidate.text.clone())
        })
        .collect();
    (targets.len() == 1).then(|| targets.into_iter().next().unwrap_or_default())
}

/// Expected reaction to the written name `written` whose intended name is
/// `intended`.
fn naive_verdict(written: &str, set: &[Candidate], prefix: bool) -> Verdict {
    if let Some(exact) = set.iter().find(|candidate| candidate.text == written) {
        return if exact.name || exact.target.is_some() {
            Verdict::Other
        } else {
            Verdict::None
        };
    }
    let fuzzy = naive_fuzzy(written, set);
    if !prefix {
        return fuzzy.unwrap_or(Verdict::None);
    }
    match (fuzzy, naive_prefix(written, set)) {
        (Some(Verdict::Fuzzy(name)), Some(other)) if name != other => Verdict::None,
        (Some(found), _) if found != Verdict::None => found,
        (Some(Verdict::None), _) if set_has_tie(written, set) => Verdict::None,
        (_, Some(target)) => Verdict::Prefix(target),
        _ => Verdict::None,
    }
}

fn set_has_tie(written: &str, set: &[Candidate]) -> bool {
    let distances: Vec<usize> = set
        .iter()
        .map(|candidate| naive_distance(written, &candidate.text))
        .collect();
    let Some(best) = distances.iter().min() else {
        return false;
    };
    let winners: Vec<&Candidate> = set
        .iter()
        .zip(&distances)
        .filter(|(_, d)| *d == best)
        .map(|(c, _)| c)
        .collect();
    let texts: BTreeSet<&str> = winners.iter().map(|winner| winner.text.as_str()).collect();
    winners.iter().any(|winner| winner.name) && texts.len() > 1
}

struct World {
    commands: Vec<Command>,
    tables: Tables,
    command_set: Vec<Candidate>,
}

impl World {
    fn new() -> Self {
        let commands = catalog();
        let tables = load_embedded().unwrap();
        let names: Vec<(String, bool)> = commands
            .iter()
            .map(|c| (c.name.clone(), c.effecting))
            .collect();
        let command_set = Self::set(&tables, NameKind::Command, &names, true);
        Self {
            commands,
            tables,
            command_set,
        }
    }

    fn set(
        tables: &Tables,
        kind: NameKind,
        names: &[(String, bool)],
        habits: bool,
    ) -> Vec<Candidate> {
        let mut set: Vec<Candidate> = names
            .iter()
            .map(|(text, effecting)| Candidate {
                text: text.clone(),
                target: None,
                name: true,
                effecting: *effecting,
            })
            .collect();
        for entry in tables.name_entries() {
            let candidate = match &entry.role {
                NameRole::Synonym { to }
                    if entry.kind == kind && names.iter().any(|(name, _)| name == to) =>
                {
                    Candidate {
                        text: entry.text.clone(),
                        target: Some(to.clone()),
                        name: false,
                        effecting: false,
                    }
                }
                NameRole::Habit if habits => Candidate {
                    text: entry.text.clone(),
                    target: None,
                    name: false,
                    effecting: false,
                },
                _ => continue,
            };
            set.push(candidate);
        }
        set
    }

    fn command(&self, name: &str) -> Option<&Command> {
        self.commands.iter().find(|command| command.name == name)
    }

    fn effecting(&self, name: &str) -> bool {
        self.command(name).is_some_and(|command| command.effecting)
    }

    fn param_set(&self, command: &str) -> Vec<Candidate> {
        let names: Vec<(String, bool)> = self
            .command(command)
            .map(|command| {
                command
                    .params
                    .iter()
                    .map(|(name, _, _)| (name.clone(), false))
                    .collect()
            })
            .unwrap_or_default();
        Self::set(&self.tables, NameKind::Param, &names, false)
    }

    fn values(&self, command: &str, param: &str) -> Vec<String> {
        self.command(command)
            .and_then(|command| command.params.iter().find(|(name, _, _)| name == param))
            .map(|(_, values, _)| values.clone())
            .unwrap_or_default()
    }

    fn value_set(&self, command: &str, param: &str) -> Vec<Candidate> {
        let names: Vec<(String, bool)> = self
            .values(command, param)
            .into_iter()
            .map(|value| (value, false))
            .collect();
        Self::set(&self.tables, NameKind::Value, &names, false)
    }

    fn habit(&self, word: &str) -> bool {
        self.tables.habits().any(|habit| habit == word)
    }
}

fn is_mode(command: &str, param: &str) -> bool {
    MODE_PARAMS.contains(&param) || (command == "create" && param == "type")
}

fn typo(word: &str, random: &mut Random) -> String {
    let chars: Vec<char> = word.chars().collect();
    let position = random.below(chars.len());
    let mut out = chars.clone();
    match random.below(4) {
        0 => out.insert(position, random.letter()),
        1 if chars.len() > 1 => {
            out.remove(position);
        }
        2 if position + 1 < chars.len() => out.swap(position, position + 1),
        _ => out[position] = random.letter(),
    }
    out.into_iter().collect()
}

fn far_typo(word: &str, random: &mut Random) -> String {
    let once = typo(word, random);
    typo(&once, random)
}

struct Case {
    operator: &'static str,
    input: String,
    expect: Value,
    kind: &'static str,
    from: String,
    to: String,
    scope: Option<String>,
}

fn canonical(program: &str, rule: &str, class: &str) -> Value {
    json!({ "canonical": program, "rules": [rule], "class": class })
}

/// Reaction to a misspelt name of a command that changes the world: the
/// program does not run and the corrected program is the only candidate.
fn ambiguous(program: &str) -> Value {
    json!({ "error": "E_AMBIGUOUS", "candidates": [program] })
}

fn replace(program: &str, token: &Token, text: &str) -> String {
    format!("{}{text}{}", &program[..token.start], &program[token.end..])
}

struct Generator<'a> {
    world: &'a World,
    random: Random,
    cases: Vec<(String, String, Case)>,
}

impl Generator<'_> {
    fn add(&mut self, seed: &str, program: &str, case: Case) {
        if case.input != program {
            self.cases.push((seed.to_owned(), program.to_owned(), case));
        }
    }

    fn seed(&mut self, seed: &str, program: &str) {
        let tokens = Scanner::scan(program);
        for token in &tokens {
            self.token(seed, program, token);
        }
        self.notation(seed, program, &tokens);
    }

    fn token(&mut self, seed: &str, program: &str, token: &Token) {
        match &token.kind {
            TokenKind::Command => self.command(seed, program, token),
            TokenKind::Param { command } => self.param(seed, program, token, command),
            TokenKind::Value { command, param } => self.value(seed, program, token, command, param),
            TokenKind::Bool => self.boolean(seed, program, token),
            TokenKind::Size => self.size(seed, program, token),
            TokenKind::Text | TokenKind::Pipe => {}
        }
    }

    fn name_case(
        &mut self,
        operator: &'static str,
        program: &str,
        token: &Token,
        written: String,
        verdict: &Verdict,
        names: (&'static str, &'static str, Option<String>),
    ) -> Option<Case> {
        let (kind, unknown, scope) = names;
        let word = &program[token.start..token.end];
        let rule = match kind {
            "command" => "L01",
            "param" => "L02",
            _ => "V01",
        };
        let expect = match verdict {
            Verdict::Fuzzy(name)
                if name == word && kind == "command" && self.world.effecting(word) =>
            {
                ambiguous(program)
            }
            Verdict::Fuzzy(name) if name == word => canonical(program, rule, "F"),
            Verdict::Prefix(name) if name == word => canonical(program, "L05", "F"),
            Verdict::None => json!({ "error": unknown }),
            _ => return None,
        };
        Some(Case {
            operator,
            input: replace(program, token, &written),
            expect,
            kind,
            from: word.to_owned(),
            to: written,
            scope,
        })
    }

    fn command(&mut self, seed: &str, program: &str, token: &Token) {
        let word = program[token.start..token.end].to_owned();
        for (operator, written) in [
            ("typo-command", typo(&word, &mut self.random)),
            ("far-command", far_typo(&word, &mut self.random)),
        ] {
            let verdict = naive_verdict(&written, &self.world.command_set, false);
            if let Some(case) = self.name_case(
                operator,
                program,
                token,
                written,
                &verdict,
                ("command", "E_UNKNOWN_COMMAND", None),
            ) {
                self.add(seed, program, case);
            }
        }
        let mut capital = word.clone();
        capital.replace_range(..1, &word[..1].to_ascii_uppercase());
        self.add(
            seed,
            program,
            Case {
                operator: "case-command",
                input: replace(program, token, &capital),
                expect: canonical(program, "L03", "C"),
                kind: "command",
                from: word.clone(),
                to: capital,
                scope: None,
            },
        );
        self.synonym(seed, program, token, &word);
    }

    fn synonym(&mut self, seed: &str, program: &str, token: &Token, word: &str) {
        let synonyms: Vec<String> = self
            .world
            .command_set
            .iter()
            .filter(|candidate| {
                candidate.target.as_deref() == Some(word) && !self.world.habit(&candidate.text)
            })
            .map(|candidate| candidate.text.clone())
            .filter(|text| text.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
            .collect();
        if synonyms.is_empty() {
            return;
        }
        let written = synonyms[self.random.below(synonyms.len())].clone();
        self.add(
            seed,
            program,
            Case {
                operator: "synonym-command",
                input: replace(program, token, &written),
                expect: canonical(program, "L06", "C"),
                kind: "command",
                from: word.to_owned(),
                to: written,
                scope: None,
            },
        );
    }

    fn param(&mut self, seed: &str, program: &str, token: &Token, command: &str) {
        let word = program[token.start..token.end].to_owned();
        let set = self.world.param_set(command);
        if !set
            .iter()
            .any(|candidate| candidate.name && candidate.text == word)
        {
            return;
        }
        let written = typo(&word, &mut self.random);
        let verdict = naive_verdict(&written, &set, true);
        if let Some(case) = self.name_case(
            "typo-param",
            program,
            token,
            written,
            &verdict,
            ("param", "E_UNKNOWN_PARAM", Some(command.to_owned())),
        ) {
            self.add(seed, program, case);
        }
        if word.contains('_') {
            let camel = camel_case(&word);
            self.add(
                seed,
                program,
                Case {
                    operator: "convention-param",
                    input: replace(program, token, &camel),
                    expect: canonical(program, "L04", "C"),
                    kind: "param",
                    from: word,
                    to: camel,
                    scope: Some(command.to_owned()),
                },
            );
        }
    }

    fn value(&mut self, seed: &str, program: &str, token: &Token, command: &str, param: &str) {
        let word = program[token.start..token.end].to_owned();
        if !self.world.values(command, param).contains(&word) {
            return;
        }
        let written = typo(&word, &mut self.random);
        let set = self.world.value_set(command, param);
        let verdict = naive_verdict(&written, &set, !is_mode(command, param));
        let scope = Some(format!("{command}.{param}"));
        if let Some(case) = self.name_case(
            "typo-value",
            program,
            token,
            written,
            &verdict,
            ("value", "E_UNKNOWN_VALUE", scope),
        ) {
            self.add(seed, program, case);
        }
    }

    fn boolean(&mut self, seed: &str, program: &str, token: &Token) {
        let word = &program[token.start..token.end];
        let written = if word == "true" { "yes" } else { "no" };
        self.add(
            seed,
            program,
            Case {
                operator: "word-bool",
                input: replace(program, token, written),
                expect: canonical(program, "T04", "C"),
                kind: "bool",
                from: word.to_owned(),
                to: written.to_owned(),
                scope: None,
            },
        );
    }

    fn size(&mut self, seed: &str, program: &str, token: &Token) {
        let word = &program[token.start..token.end];
        let split = word
            .find(|c: char| c.is_ascii_alphabetic())
            .unwrap_or(word.len());
        let (number, unit) = word.split_at(split);
        let written = if self.random.below(2) == 0 {
            format!("{number} {}", unit.to_ascii_uppercase())
        } else {
            format!("{number} {}", spelled_unit(unit))
        };
        self.add(
            seed,
            program,
            Case {
                operator: "unit-size",
                input: replace(program, token, &written),
                expect: canonical(program, "T01", "C"),
                kind: "size",
                from: word.to_owned(),
                to: written,
                scope: None,
            },
        );
    }

    fn notation(&mut self, seed: &str, program: &str, tokens: &[Token]) {
        self.pipe(seed, program, tokens);
        self.separators(seed, program, tokens);
        self.quotes(seed, program, tokens);
        self.paths(seed, program, tokens);
    }

    fn pipe(&mut self, seed: &str, program: &str, tokens: &[Token]) {
        let pipes: Vec<&Token> = tokens
            .iter()
            .filter(|token| token.kind == TokenKind::Pipe)
            .collect();
        if pipes.is_empty() {
            return;
        }
        for written in ["|", "| >", "|->", "->"] {
            let token = pipes[self.random.below(pipes.len())];
            self.add(
                seed,
                program,
                Case {
                    operator: "pipe",
                    input: replace(program, token, written),
                    expect: canonical(program, "L13", "C"),
                    kind: "pipe",
                    from: "|>".into(),
                    to: written.into(),
                    scope: None,
                },
            );
        }
    }

    fn separators(&mut self, seed: &str, program: &str, tokens: &[Token]) {
        let params: Vec<&Token> = tokens
            .iter()
            .filter(|token| matches!(token.kind, TokenKind::Param { .. }))
            .collect();
        if params.is_empty() {
            return;
        }
        let token = params[self.random.below(params.len())];
        let colon = Token {
            kind: TokenKind::Text,
            start: token.end,
            end: token.end + 2,
        };
        if program.get(colon.start..colon.end) == Some(": ") {
            self.add(
                seed,
                program,
                Case {
                    operator: "separator-equals",
                    input: replace(program, &colon, "="),
                    expect: canonical(program, "L16", "C"),
                    kind: "separator",
                    from: ": ".into(),
                    to: "=".into(),
                    scope: None,
                },
            );
        }
        let before = Token {
            kind: TokenKind::Text,
            start: token.start - 1,
            end: token.start,
        };
        if program.get(before.start..before.end) == Some(" ")
            && !program[..token.start]
                .trim_end()
                .ends_with(['(', '[', '{', ',', '>'])
        {
            self.add(
                seed,
                program,
                Case {
                    operator: "separator-comma",
                    input: replace(program, &before, ", "),
                    expect: canonical(program, "L15", "S"),
                    kind: "separator",
                    from: " ".into(),
                    to: ", ".into(),
                    scope: None,
                },
            );
        }
    }

    fn simple_texts<'t>(program: &str, tokens: &'t [Token]) -> Vec<&'t Token> {
        tokens
            .iter()
            .filter(|token| token.kind == TokenKind::Text)
            .filter(|token| {
                let inner = &program[token.start + 1..token.end - 1];
                !inner.is_empty() && !inner.contains(['\'', '"', '\\', '\u{201c}', '\u{201d}'])
            })
            .collect()
    }

    fn quotes(&mut self, seed: &str, program: &str, tokens: &[Token]) {
        let texts = Self::simple_texts(program, tokens);
        if texts.is_empty() {
            return;
        }
        for (operator, open, close) in [
            ("quote-single", '\'', '\''),
            ("quote-typographic", '\u{201c}', '\u{201d}'),
        ] {
            let token = texts[self.random.below(texts.len())];
            let inner = &program[token.start + 1..token.end - 1];
            let written = format!("{open}{inner}{close}");
            self.add(
                seed,
                program,
                Case {
                    operator,
                    input: replace(program, token, &written),
                    expect: canonical(program, "L07", "C"),
                    kind: "quote",
                    from: program[token.start..token.end].to_owned(),
                    to: written,
                    scope: None,
                },
            );
        }
    }

    fn paths(&mut self, seed: &str, program: &str, tokens: &[Token]) {
        let paths: Vec<&Token> = Self::simple_texts(program, tokens)
            .into_iter()
            .filter(|token| {
                let inner = &program[token.start + 1..token.end - 1];
                (inner.starts_with('/') || inner.starts_with("C:/"))
                    && inner.len() > 1
                    && !inner.contains([' ', '*', '?'])
                    && !inner.ends_with('/')
            })
            .collect();
        if paths.is_empty() {
            return;
        }
        let token = paths[self.random.below(paths.len())];
        let inner = program[token.start + 1..token.end - 1].to_owned();
        let quoted = format!("\"{inner}/\"");
        self.add(
            seed,
            program,
            Case {
                operator: "path-trailing",
                input: replace(program, token, &quoted),
                expect: canonical(program, "L19", "C"),
                kind: "path",
                from: format!("\"{inner}\""),
                to: quoted,
                scope: None,
            },
        );
        if let Some(rest) = inner.strip_prefix("C:") {
            let lower = format!("\"c:{rest}\"");
            self.add(
                seed,
                program,
                Case {
                    operator: "path-drive",
                    input: replace(program, token, &lower),
                    expect: canonical(program, "L19", "C"),
                    kind: "path",
                    from: format!("\"{inner}\""),
                    to: lower,
                    scope: None,
                },
            );
        } else if unquoted_is_path(&inner) {
            self.add(
                seed,
                program,
                Case {
                    operator: "path-unquoted",
                    input: replace(program, token, &inner),
                    expect: canonical(program, "L21", "C"),
                    kind: "path",
                    from: format!("\"{inner}\""),
                    to: inner.clone(),
                    scope: None,
                },
            );
        }
    }
}

/// An unquoted path is never read as a regular expression: the text after
/// its second `/` is not made only of flag letters.
fn unquoted_is_path(path: &str) -> bool {
    let mut segments = path.trim_start_matches('/').splitn(2, '/');
    let _first = segments.next();
    segments.next().is_some_and(|rest| {
        !rest
            .split('/')
            .next()
            .unwrap_or_default()
            .chars()
            .all(|c| "imsx".contains(c))
    })
}

fn camel_case(word: &str) -> String {
    let mut out = String::new();
    let mut upper = false;
    for c in word.chars() {
        if c == '_' {
            upper = true;
        } else if upper {
            out.push(c.to_ascii_uppercase());
            upper = false;
        } else {
            out.push(c);
        }
    }
    out
}

fn spelled_unit(unit: &str) -> &'static str {
    match unit {
        "b" => "bytes",
        "kb" => "kilobytes",
        "mb" => "megabytes",
        "gb" => "gigabytes",
        _ => "terabytes",
    }
}

fn program_kind(world: &World, program: &str) -> &'static str {
    let effect = Scanner::scan(program)
        .iter()
        .filter(|token| token.kind == TokenKind::Command)
        .any(|token| world.effecting(&program[token.start..token.end]));
    if effect { "effect" } else { "read" }
}

fn generate(world: &World) -> String {
    let mut generator = Generator {
        world,
        random: Random(SEED),
        cases: Vec::new(),
    };
    let mut seeds = golden_seeds();
    seeds.extend(catalog_seeds(&world.commands));
    for (seed, program) in &seeds {
        generator.seed(seed, program);
    }
    let mut counters: BTreeMap<&str, usize> = BTreeMap::new();
    let mut seen = BTreeSet::new();
    let mut lines: Vec<(String, String)> = Vec::new();
    for (seed, program, case) in generator.cases {
        if !seen.insert(case.input.clone()) {
            continue;
        }
        let counter = counters.entry(case.operator).or_default();
        if *counter == MAX_PER_OPERATOR {
            continue;
        }
        *counter += 1;
        let id = format!("mutate-{}-{:04}", case.operator, counter);
        let mut mutation = json!({ "operator": case.operator, "kind": case.kind, "from": case.from, "to": case.to });
        if let Some(scope) = case.scope {
            mutation["in"] = json!(scope);
        }
        let line = json!({
            "id": id,
            "input": case.input,
            "variant": "a",
            "expect": case.expect,
            "program_kind": program_kind(world, &program),
            "intent": if case.expect["error"] == "E_AMBIGUOUS" { "ambiguous" } else { "unambiguous" },
            "source": "generated",
            "mutation": mutation,
            "note": format!("Generated from {seed}."),
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
    assert!((700..=1_100).contains(&count), "{count} cases");
}

// Component check: name, unit and boolean correction against the corpus.

#[derive(Debug, Default)]
struct Tally {
    checked: usize,
    corrected: usize,
    false_corrections: Vec<String>,
    mismatches: Vec<String>,
}

fn resolve_mutation(world: &World, mutation: &Value) -> Option<Match> {
    let written = mutation["to"].as_str()?;
    let scope = mutation["in"].as_str().unwrap_or_default();
    let options = Options::default();
    let tables = &world.tables;
    match mutation["kind"].as_str()? {
        "command" => {
            let names = world
                .commands
                .iter()
                .map(|c| NameEntry::command(c.name.clone(), c.effecting));
            let index = values::index(NameKind::Command, names, tables);
            Some(values::resolve(
                written,
                NameKind::Command,
                &index,
                tables,
                &options,
            ))
        }
        "param" => {
            let params: Vec<String> = world
                .command(scope)?
                .params
                .iter()
                .map(|(name, _, _)| name.clone())
                .collect();
            let params: Vec<&str> = params.iter().map(String::as_str).collect();
            Some(values::resolve_in(
                written,
                NameKind::Param,
                &params,
                tables,
                &options,
            ))
        }
        "value" => {
            let (command, param) = scope.rsplit_once('.')?;
            let allowed = world.values(command, param);
            let allowed: Vec<&str> = allowed.iter().map(String::as_str).collect();
            Some(if is_mode(command, param) {
                values::mode_value(written, &allowed, tables, &options)
            } else {
                values::value(written, &allowed, tables, &options)
            })
        }
        _ => None,
    }
}

fn check_name_case(tally: &mut Tally, id: &str, expect: &Value, mutation: &Value, found: Match) {
    tally.checked += 1;
    let intended = mutation["from"].as_str().unwrap_or_default();
    let expected_rule = expect["rules"][0].as_str();
    match (found, expected_rule) {
        (Match::Corrected(correction), None)
            if expect["error"] == "E_AMBIGUOUS"
                && correction.to == intended
                && correction.effecting =>
        {
            tally.corrected += 1;
        }
        (Match::Corrected(correction), Some(rule))
            if correction.to == intended && correction.rule.to_string() == rule =>
        {
            tally.corrected += 1;
        }
        (Match::Corrected(correction), _) if correction.to != intended => {
            tally
                .false_corrections
                .push(format!("{id}: {} -> {}", mutation["to"], correction.to));
        }
        (Match::Tie(_) | Match::Unknown, None) => {}
        (found, _) => tally.mismatches.push(format!("{id}: {found:?}")),
    }
}

fn check_literal_case(tally: &mut Tally, world: &World, id: &str, mutation: &Value) {
    let written = mutation["to"].as_str().unwrap_or_default();
    let intended = mutation["from"].as_str().unwrap_or_default();
    let outcome = match mutation["kind"].as_str() {
        Some("size") => types::size(Written::bare(written), &world.tables),
        Some("bool") => types::boolean(Written::bare(written), &world.tables),
        _ => return,
    };
    tally.checked += 1;
    match outcome {
        Outcome::Converted(converted)
            if converted.to == intended && converted.class == Class::Certain =>
        {
            tally.corrected += 1
        }
        Outcome::Converted(converted) => tally
            .false_corrections
            .push(format!("{id}: {written} -> {}", converted.to)),
        other => tally.mismatches.push(format!("{id}: {other:?}")),
    }
}

#[test]
fn generated_cases_agree_with_name_and_literal_correction() {
    let world = World::new();
    let corpus = generate(&world);
    let mut tally = Tally::default();
    for line in corpus.lines() {
        let case: Value = serde_json::from_str(line).unwrap();
        let id = case["id"].as_str().unwrap_or_default();
        let mutation = &case["mutation"];
        if let Some(found) = resolve_mutation(&world, mutation) {
            if mutation["operator"]
                .as_str()
                .is_some_and(|operator| operator.starts_with("typo") || operator.starts_with("far"))
            {
                check_name_case(&mut tally, id, &case["expect"], mutation, found);
            }
        } else {
            check_literal_case(&mut tally, &world, id, mutation);
        }
    }
    eprintln!(
        "component check: {} cases, {} corrected ({:.1}%), {} false corrections",
        tally.checked,
        tally.corrected,
        100.0 * tally.corrected as f64 / tally.checked.max(1) as f64,
        tally.false_corrections.len()
    );
    assert!(
        tally.false_corrections.is_empty(),
        "{:#?}",
        tally.false_corrections
    );
    assert!(tally.mismatches.is_empty(), "{:#?}", tally.mismatches);
}

#[test]
fn naive_rules_follow_the_documented_examples() {
    let world = World::new();
    assert_eq!(
        naive_verdict("remvoe", &world.command_set, false),
        Verdict::Fuzzy("remove".into())
    );
    assert_eq!(
        naive_verdict("lest", &world.command_set, false),
        Verdict::None
    );
    assert_eq!(
        naive_verdict("rnu", &world.command_set, false),
        Verdict::None
    );
    assert_eq!(
        naive_verdict("work", &world.command_set, false),
        Verdict::Other
    );
    assert_eq!(naive_distance("ca", "abc"), 3);
    assert!(unquoted_is_path("/srv/app"));
    assert!(!unquoted_is_path("/srv/x"));
    assert_eq!(camel_case("line_numbers"), "lineNumbers");
}
