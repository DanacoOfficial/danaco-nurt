// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
// A failed unwrap, expectation or panic is how a test reports a failure.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
// Every test crate that includes this module uses a part of it.
#![allow(dead_code)]

//! Helpers shared by the lexical correction tests: a command catalog read
//! from the repository and the cases of the lexical correction corpus.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use nurt_contracts::config::{CorrectionConfig, SyntaxConfig};
use nurt_contracts::kinds::SyntaxVariant;
use nurt_correct::lexical::{ArgumentShape, CatalogView};
use serde_json::Value;
use toml_edit::{Document, Item};

/// Commands that take a bare word as a positional argument, such as a field
/// name or an expression.
const WORD_TAKING: [&str; 6] = ["filter", "sort", "select", "group", "assert", "map"];

/// Command catalog of the repository.
pub(crate) struct Catalog {
    names: Vec<&'static str>,
    mutating: BTreeSet<String>,
}

impl Catalog {
    /// Reads every `[[command]]` of `catalog/*.toml`.
    pub(crate) fn load() -> Self {
        let directory = repository_root().join("catalog");
        let mut files: Vec<PathBuf> = fs::read_dir(&directory)
            .expect("catalog directory")
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "toml"))
            .collect();
        files.sort();
        let mut names = Vec::new();
        let mut mutating = BTreeSet::new();
        for file in files {
            let text = fs::read_to_string(&file).expect("catalog file");
            let document = Document::parse(text).expect("catalog TOML");
            let Some(commands) = document.get("command").and_then(Item::as_array_of_tables) else {
                continue;
            };
            for command in commands {
                let Some(name) = command.get("name").and_then(Item::as_str) else {
                    continue;
                };
                // The same rule as `policy::is_effecting`: the category, the
                // effect of the command or the effect of one of its actions.
                let by_category = matches!(
                    command.get("category").and_then(Item::as_str),
                    Some("change" | "run")
                );
                let by_actions = command
                    .get("effects")
                    .and_then(Item::as_array_of_tables)
                    .is_some_and(|actions| {
                        actions.iter().any(|action| {
                            action.get("effect").and_then(Item::as_str) != Some("read")
                        })
                    });
                if by_category
                    || by_actions
                    || command.get("effect").and_then(Item::as_str) != Some("read")
                {
                    mutating.insert(name.to_owned());
                }
                names.push(&*Box::leak(name.to_owned().into_boxed_str()));
            }
        }
        Self { names, mutating }
    }
}

impl CatalogView for Catalog {
    fn is_command(&self, name: &str) -> bool {
        self.names.contains(&name)
    }

    fn command_names(&self) -> &[&str] {
        &self.names
    }

    fn is_mutating(&self, name: &str) -> bool {
        self.mutating.contains(name)
    }

    fn can_take_argument(&self, command: &str, _: usize, shape: ArgumentShape) -> bool {
        shape == ArgumentShape::Word && WORD_TAKING.contains(&command)
    }
}

/// One case of a corpus file.
pub(crate) struct Case {
    /// File name and line, for failure messages.
    pub(crate) location: String,
    /// File name relative to the corpus directory.
    pub(crate) file: String,
    /// Whether the case belongs to the adversarial corpus.
    pub(crate) adversarial: bool,
    /// The case as written.
    pub(crate) value: Value,
}

impl Case {
    /// A string member, empty when absent.
    pub(crate) fn text(&self, member: &str) -> &str {
        self.value
            .get(member)
            .and_then(Value::as_str)
            .unwrap_or_default()
    }

    /// The expected reaction.
    pub(crate) fn expect(&self) -> &Value {
        self.value.get("expect").unwrap_or(&Value::Null)
    }

    /// Expected rule identifiers, in report order.
    pub(crate) fn rules(&self) -> Vec<String> {
        self.expect()
            .get("rules")
            .and_then(Value::as_array)
            .map(|rules| {
                rules
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Expected canonical text, when the case expects one.
    pub(crate) fn canonical(&self) -> Option<&str> {
        self.expect().get("canonical").and_then(Value::as_str)
    }

    /// Whether the case expects no correction.
    pub(crate) fn unchanged(&self) -> bool {
        self.expect().get("unchanged").and_then(Value::as_bool) == Some(true)
    }

    /// Syntax variant the input is read in.
    pub(crate) fn variant(&self) -> SyntaxVariant {
        match self.text("variant") {
            "b" => SyntaxVariant::B,
            "c" => SyntaxVariant::C,
            _ => SyntaxVariant::A,
        }
    }

    /// Whether the case has conditions such as a base directory.
    pub(crate) fn has_context(&self) -> bool {
        self.value.get("context").is_some()
    }
}

/// Root of the repository.
pub(crate) fn repository_root() -> PathBuf {
    Path::new(
        std::env::var("CARGO_MANIFEST_DIR")
            .as_deref()
            .unwrap_or(env!("CARGO_MANIFEST_DIR")),
    )
    .join("../..")
}

fn corpus_files(directory: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for path in entries.filter_map(Result::ok).map(|entry| entry.path()) {
        if path.is_dir() {
            corpus_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "jsonl") {
            out.push(path);
        }
    }
}

/// Every case of the lexical corpus, files in name order.
pub(crate) fn load_cases() -> Vec<Case> {
    let root = repository_root().join("tests/d1/correction/lexical");
    let mut files = Vec::new();
    corpus_files(&root, &mut files);
    files.sort();
    let mut cases = Vec::new();
    for file in files {
        let relative = file
            .strip_prefix(&root)
            .map(|path| path.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default();
        let in_adversarial = relative.starts_with("adversarial/");
        let text = fs::read_to_string(&file).expect("corpus file");
        for (index, line) in text.lines().enumerate() {
            let value: Value = serde_json::from_str(line).expect("corpus line is JSON");
            let marked = value.get("corpus").and_then(Value::as_str) == Some("adversarial");
            cases.push(Case {
                location: format!("{relative}:{}", index + 1),
                file: relative.clone(),
                adversarial: in_adversarial || marked,
                value,
            });
        }
    }
    cases
}

/// Syntax configuration of a variant with tolerant reading.
pub(crate) fn syntax(variant: SyntaxVariant) -> SyntaxConfig {
    let mut config = SyntaxConfig::default();
    config.variant = variant;
    config
}

/// Default correction configuration.
pub(crate) fn correction() -> CorrectionConfig {
    CorrectionConfig::default()
}
