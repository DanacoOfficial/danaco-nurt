// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Differential tests of the INI and property readers: Git configuration
//! against the output of `git config --list`, the generic profile against
//! `rust-ini`, and property files against `java.util.Properties`.
//!
//! The expected results of Git and of the Java library are recorded under
//! `tests/d5/data/u03`; the recorded results are compared in every run, and
//! the installed `git` is run again when it is in the search path.
// Helpers outside `#[test]` functions fail the test on unexpected errors.
#![allow(clippy::unwrap_used, clippy::panic)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use ini::{Ini, ParseOption};
use nurt_contracts::value::Value;
use nurt_doc::formats::ini::{IniDocument, IniProfile};
use nurt_doc::formats::properties::PropertiesFormat;
use nurt_doc::registry::{DocFormat, DocOptions};

/// Entry of a flattened Git configuration: key and value; `None` for a key
/// without `=`.
type GitEntry = (String, Option<String>);

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/d5")
}

fn read(path: &Path) -> Vec<u8> {
    std::fs::read(path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

/// Entries of `git config --list --null` output.
fn parse_list(bytes: &[u8]) -> Vec<GitEntry> {
    let text = String::from_utf8(bytes.to_vec()).unwrap();
    text.split_terminator('\0')
        .map(|record| match record.split_once('\n') {
            Some((key, value)) => (key.to_owned(), Some(value.to_owned())),
            None => (record.to_owned(), None),
        })
        .collect()
}

/// Entries of a value tree of the `gitconfig` profile, in key order.
fn flatten(prefix: &str, value: &Value, out: &mut Vec<GitEntry>) {
    match value {
        Value::Map(fields) => {
            for (name, field) in fields {
                let key = if prefix.is_empty() {
                    name.clone()
                } else {
                    format!("{prefix}.{name}")
                };
                flatten(&key, field, out);
            }
        }
        Value::List(items) => {
            for item in items {
                flatten(prefix, item, out);
            }
        }
        Value::Text(text) => out.push((prefix.to_owned(), Some(text.clone()))),
        Value::Bool(true) => out.push((prefix.to_owned(), None)),
        other => panic!("{prefix}: unexpected value {other:?}"),
    }
}

/// Entries of the document read with the `gitconfig` profile, stably sorted
/// by key.
fn read_gitconfig(text: &str) -> Vec<GitEntry> {
    let document = IniDocument::parse(text, IniProfile::Gitconfig).unwrap();
    let mut entries = Vec::new();
    flatten("", &document.to_value(), &mut entries);
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    entries
}

fn sorted(mut entries: Vec<GitEntry>) -> Vec<GitEntry> {
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    entries
}

/// Pairs of a Git configuration file and its recorded list.
fn git_cases() -> Vec<(PathBuf, PathBuf)> {
    let data = root().join("data/u03/git-config");
    let corpus = root().join("corpus/ini");
    let mut cases = vec![
        (data.join("edge.gitconfig"), data.join("edge.list")),
        (
            corpus.join("gitconfig/.gitconfig"),
            data.join("gitconfig.list"),
        ),
    ];
    for number in 0..25 {
        cases.push((
            corpus.join(format!("generated/gitconfig/{number:03}.ini")),
            data.join(format!("generated/{number:03}.list")),
        ));
    }
    cases
}

fn assert_same(file: &Path, expected: &[GitEntry], actual: &[GitEntry]) {
    assert_eq!(expected, actual, "{}", file.display());
}

#[test]
fn gitconfig_profile_reads_the_recorded_git_output() {
    let cases = git_cases();
    assert_eq!(cases.len(), 27);
    for (file, list) in cases {
        let text = String::from_utf8(read(&file)).unwrap();
        let expected = sorted(parse_list(&read(&list)));
        assert_same(&file, &expected, &read_gitconfig(&text));
    }
}

fn installed_git() -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join("git"))
        .find(|candidate| candidate.is_file())
}

/// [U] Runs the installed `git` on every file and compares the output with
/// the gitconfig profile; returns without checks when `git` is not in the
/// search path.
#[test]
fn gitconfig_profile_reads_the_output_of_the_installed_git_skipped_without_git() {
    let Some(git) = installed_git() else {
        return;
    };
    for (file, _) in git_cases() {
        let output = Command::new(&git)
            .args(["config", "--list", "--null", "--file"])
            .arg(&file)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .unwrap();
        assert!(output.status.success(), "{}", file.display());
        let text = String::from_utf8(read(&file)).unwrap();
        assert_same(
            &file,
            &sorted(parse_list(&output.stdout)),
            &read_gitconfig(&text),
        );
    }
}

#[test]
fn gitconfig_distinguishes_subsection_names_by_case_only_there() {
    let text = "[Remote \"Origin\"]\n\turl = a\n[remote \"origin\"]\n\turl = b\n[REMOTE.Dotted]\n\tURL = c\n";
    let entries = read_gitconfig(text);
    let expected: Vec<GitEntry> = [
        ("remote.Origin.url", "a"),
        ("remote.dotted.url", "c"),
        ("remote.origin.url", "b"),
    ]
    .iter()
    .map(|(key, value)| ((*key).to_owned(), Some((*value).to_owned())))
    .collect();
    assert_eq!(entries, expected);
}

/// Content of a document read with the generic profile as sections of
/// keys; keys outside sections are in the section named `""`.
fn read_generic(text: &str) -> BTreeMap<String, BTreeMap<String, String>> {
    let document = IniDocument::parse(text, IniProfile::Generic).unwrap();
    let Value::Map(fields) = document.to_value() else {
        panic!("a document is a map");
    };
    let mut sections: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    for (name, field) in &fields {
        match field {
            Value::Map(keys) => {
                let section = sections.entry(name.clone()).or_default();
                for (key, value) in keys {
                    section.insert(key.clone(), text_of(value));
                }
            }
            value => {
                sections
                    .entry(String::new())
                    .or_default()
                    .insert(name.clone(), text_of(value));
            }
        }
    }
    sections
}

fn text_of(value: &Value) -> String {
    match value {
        Value::Text(text) => text.clone(),
        Value::Null => String::new(),
        other => panic!("unexpected value {other:?}"),
    }
}

/// Content of a document read by `rust-ini`; the last value of a repeated
/// key wins.
fn read_rust_ini(text: &str) -> Option<BTreeMap<String, BTreeMap<String, String>>> {
    let options = ParseOption {
        enabled_quote: false,
        enabled_escape: false,
        ..ParseOption::default()
    };
    let parsed = Ini::load_from_str_opt(text, options).ok()?;
    let mut sections: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    for (name, properties) in &parsed {
        let section = sections.entry(name.unwrap_or("").to_owned()).or_default();
        for (key, value) in properties {
            section.insert(key.to_owned(), value.to_owned());
        }
    }
    sections.retain(|_, keys| !keys.is_empty());
    Some(sections)
}

fn files_in(dir: &Path, extension: &str) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|e| e == extension))
        .collect();
    files.sort();
    files
}

/// Files with a key line that has no separator are read differently by
/// `rust-ini`, which joins the next line to such a key; they are not
/// compared.
const KEY_WITHOUT_SEPARATOR: [&str; 1] = ["setup.cfg"];

#[test]
fn generic_profile_reads_as_rust_ini_does() {
    let corpus = root().join("corpus/ini");
    let mut files = files_in(&corpus.join("generic"), "ini");
    files.extend(files_in(&corpus.join("generated/generic"), "ini"));
    files.extend(files_in(&corpus.join("generic"), "cfg"));
    let mut compared = 0;
    let mut excluded = 0;
    for file in files {
        let name = file.file_name().unwrap().to_str().unwrap();
        if KEY_WITHOUT_SEPARATOR.contains(&name) {
            excluded += 1;
            continue;
        }
        let text = String::from_utf8(read(&file)).unwrap();
        let expected = read_rust_ini(&text)
            .unwrap_or_else(|| panic!("{}: rust-ini rejects the file", file.display()));
        let mut actual = read_generic(&text);
        actual.retain(|_, keys| !keys.is_empty());
        assert_eq!(expected, actual, "{}", file.display());
        compared += 1;
    }
    assert_eq!(excluded, KEY_WITHOUT_SEPARATOR.len());
    assert_eq!(compared, 26);
}

/// Properties of a file as the format reads them.
fn read_properties(bytes: &[u8]) -> BTreeMap<String, String> {
    let handle = PropertiesFormat
        .open(bytes.to_vec(), &DocOptions::default())
        .unwrap();
    let Value::Map(fields) = handle.to_value().unwrap() else {
        panic!("a property file is a map");
    };
    fields
        .iter()
        .map(|(key, value)| (key.clone(), text_of(value)))
        .collect()
}

#[test]
fn properties_read_as_java_util_properties_does() {
    let text = String::from_utf8(read(&root().join("data/u03/java-properties.json"))).unwrap();
    let recorded: serde_json::Value = serde_json::from_str(&text).unwrap();
    let files = recorded["files"].as_object().unwrap();
    assert!(files.len() >= 100, "{} files", files.len());
    let corpus = root().join("corpus/properties");
    let mut keys = 0;
    for (name, expected) in files {
        let expected: BTreeMap<String, String> = expected
            .as_object()
            .unwrap()
            .iter()
            .map(|(key, value)| (key.clone(), value.as_str().unwrap().to_owned()))
            .collect();
        keys += expected.len();
        assert_eq!(
            expected,
            read_properties(&read(&corpus.join(name))),
            "{name}"
        );
    }
    assert!(keys >= 500, "{keys} keys");
}
