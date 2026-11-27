// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-doc/tests/u08_differential.rs
// Responsibility:   Differential tests of the value model against `yq`.
// Dependencies:     nurt_contracts, serde_json, u08_support
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Differential tests of the value model against `yq`.
//!
//! `tests/d5/differential/yq/<format>.jsonl` records the output of
//! `yq -p <format> -o json -I0` for every corpus file of the format, one JSON
//! object per line: `path` relative to `tests/d5`, then `value`, or
//! `documents` with the value of each document of a YAML stream of more than
//! one document, or `error` with the first line of the error message. Nurt
//! reads such a stream as the list of its document values, and a stream
//! without documents as `null`. Each format test reads the
//! corpus with Nurt, compares the value with the record and accepts only the
//! differences classified in `tests/d5/differential/known-differences.toml`.
//!
//! The tests named `..._when_the_recorded_yq_version_is_installed` run `yq`
//! from `PATH` and check that the records are still its output. They pass
//! without checking when `yq` is not installed or has another version.
// Helper functions of this test target are not `#[test]` functions, which
// `allow-unwrap-in-tests` and `allow-panic-in-tests` cover.
#![allow(clippy::unwrap_used, clippy::panic)]

mod u08_support;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use nurt_contracts::error::{Error, ErrorCode};
use nurt_doc::formats::json::{Dialect, JsonDocument};
use nurt_doc::formats::toml::TomlDocument;
use nurt_doc::formats::xml::encoding::parse_document;
use nurt_doc::formats::yaml::YamlModel;
use serde_json::Value as Json;
use u08_support::Codec;

/// Number of files with unclassified differences listed in a failure
/// message.
const SHOWN_DIFFERENCES: usize = 40;

/// Number of characters of a value shown in a failure message.
const SHOWN_VALUE_CHARS: usize = 120;

/// Entities that `yq` decodes like every XML processor.
const PREDEFINED_ENTITIES: [&str; 5] = ["lt", "gt", "amp", "apos", "quot"];

/// Classes of a known difference.
const CLASSES: [&str; 3] = ["convention", "yq", "nurt"];

/// A format with its corpus, its `yq` input format and its reader.
struct Format {
    /// Value of `yq -p` and stem of the record file.
    name: &'static str,
    /// Corpus directory relative to `tests/d5`.
    root: &'static str,
    /// File extensions of the corpus files.
    extensions: &'static [&'static str],
    /// Reads a file as Nurt does.
    read: fn(&[u8]) -> Result<Json, Error>,
}

const XML: Format = Format {
    name: "xml",
    root: "corpus/xml",
    extensions: &[
        "xml", "atom", "csproj", "props", "rss", "svg", "targets", "xhtml", "xlf", "plist",
    ],
    read: read_xml,
};

const JSON: Format = Format {
    name: "json",
    root: "corpus/json",
    extensions: &["json"],
    read: read_json,
};

const TOML: Format = Format {
    name: "toml",
    root: "third_party/toml/toml-test/valid",
    extensions: &["toml"],
    read: read_toml,
};

const YAML: Format = Format {
    name: "yaml",
    root: "corpus/yaml",
    extensions: &["yaml", "yml"],
    read: read_yaml,
};

/// Value of a stream: `null` without documents, the value of a single
/// document, otherwise the list of document values.
fn stream_value(mut documents: Vec<Json>) -> Json {
    match documents.len() {
        0 => Json::Null,
        1 => documents.remove(0),
        _ => Json::Array(documents),
    }
}

fn utf8(bytes: &[u8]) -> Result<&str, Error> {
    std::str::from_utf8(bytes).map_err(|error| Error::new(ErrorCode::DocParse, error.to_string()))
}

fn read_xml(bytes: &[u8]) -> Result<Json, Error> {
    let (_, tree) = parse_document(bytes, Some(&Codec))?;
    Ok(tree.to_value()?.to_plain_json())
}

/// Byte order mark that a reader of UTF-8 text skips.
const BYTE_ORDER_MARK: char = '\u{FEFF}';

fn read_json(bytes: &[u8]) -> Result<Json, Error> {
    let text = utf8(bytes)?;
    let text = text.strip_prefix(BYTE_ORDER_MARK).unwrap_or(text);
    Ok(JsonDocument::parse(text, Dialect::Json)?
        .to_value()
        .to_plain_json())
}

fn read_toml(bytes: &[u8]) -> Result<Json, Error> {
    Ok(TomlDocument::parse(utf8(bytes)?)?
        .to_value()
        .to_plain_json())
}

fn read_yaml(bytes: &[u8]) -> Result<Json, Error> {
    let model = YamlModel::parse(utf8(bytes)?)?;
    let documents = (0..model.documents().len())
        .map(|index| Ok(model.document_value(index)?.to_plain_json()))
        .collect::<Result<Vec<_>, Error>>()?;
    Ok(stream_value(documents))
}

fn data_root() -> PathBuf {
    u08_support::repository_root().join("tests/d5")
}

fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .components()
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

/// Output of `yq` for one file.
struct Record {
    path: String,
    outcome: Result<Json, String>,
}

fn records(format: &Format) -> Vec<Record> {
    let path = data_root()
        .join("differential/yq")
        .join(format!("{}.jsonl", format.name));
    std::fs::read_to_string(&path)
        .unwrap()
        .lines()
        .map(|line| {
            let mut object: serde_json::Map<String, Json> = serde_json::from_str(line).unwrap();
            let path = object["path"].as_str().unwrap().to_owned();
            let outcome = match (object.remove("error"), object.remove("documents")) {
                (Some(Json::String(message)), _) => Err(message),
                (_, Some(Json::Array(documents))) => Ok(stream_value(documents)),
                _ => Ok(object.remove("value").unwrap_or(Json::Null)),
            };
            Record { path, outcome }
        })
        .collect()
}

fn corpus_files(format: &Format) -> BTreeSet<String> {
    let root = data_root();
    u08_support::files_below(&root.join(format.root))
        .into_iter()
        .filter(|path| {
            path.extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| format.extensions.contains(&extension))
        })
        .map(|path| relative(&root, &path))
        .collect()
}

/// Difference between the value read by Nurt and the output of `yq`.
#[derive(Debug)]
enum Mismatch {
    /// Values differ at a JSON pointer; `None` marks a missing member.
    Value {
        pointer: String,
        nurt: Option<Json>,
        yq: Option<Json>,
    },
    /// Nurt rejects a file that `yq` reads.
    NurtError(Error),
    /// `yq` rejects a file that Nurt reads.
    YqError(String),
}

fn pointer_token(key: &str) -> String {
    key.replace('~', "~0").replace('/', "~1")
}

/// Largest magnitude below which every integer has an exact `f64`.
const EXACT_IN_F64: i128 = 1 << 53;

fn integer_of(number: &serde_json::Number) -> Option<i128> {
    number
        .as_i64()
        .map(i128::from)
        .or_else(|| number.as_u64().map(i128::from))
}

/// Whether `float` is exactly the integer `integer`; an integer beyond
/// [`EXACT_IN_F64`] has no exact `f64`, so a float never equals it.
#[expect(
    clippy::cast_precision_loss,
    reason = "the integer is below 2^53 and exact in f64"
)]
fn float_equals(integer: i128, float: &serde_json::Number) -> bool {
    integer.abs() <= EXACT_IN_F64 && float.as_f64().is_some_and(|value| value == integer as f64)
}

/// Equality of numbers, exact for integers: two integers are equal when they
/// are the same integer, not when their `f64` approximations are.
fn same_number(nurt: &serde_json::Number, yq: &serde_json::Number) -> bool {
    match (integer_of(nurt), integer_of(yq)) {
        (Some(left), Some(right)) => left == right,
        (Some(integer), None) => float_equals(integer, yq),
        (None, Some(integer)) => float_equals(integer, nurt),
        (None, None) => nurt == yq || nurt.as_f64() == yq.as_f64(),
    }
}

#[test]
fn numbers_are_compared_exactly() {
    let number = |text: &str| serde_json::from_str::<serde_json::Number>(text).unwrap();
    assert!(same_number(&number("1"), &number("1.0")));
    assert!(same_number(&number("-3"), &number("-3")));
    assert!(same_number(&number("0.5"), &number("0.50")));
    assert!(!same_number(
        &number("9007199254740993"),
        &number("9007199254740992")
    ));
    assert!(!same_number(
        &number("9007199254740993"),
        &number("9007199254740992.0")
    ));
    assert!(!same_number(
        &number("18446744073709551615"),
        &number("1.8446744073709552e19")
    ));
    assert!(!same_number(&number("1"), &number("2")));
}

fn compare(pointer: &str, nurt: &Json, yq: &Json, out: &mut Vec<Mismatch>) {
    match (nurt, yq) {
        (Json::Object(left), Json::Object(right)) => {
            for (key, value) in left {
                let pointer = format!("{pointer}/{}", pointer_token(key));
                match right.get(key) {
                    Some(other) => compare(&pointer, value, other, out),
                    None => out.push(Mismatch::Value {
                        pointer,
                        nurt: Some(value.clone()),
                        yq: None,
                    }),
                }
            }
            for (key, value) in right.iter().filter(|(key, _)| !left.contains_key(*key)) {
                out.push(Mismatch::Value {
                    pointer: format!("{pointer}/{}", pointer_token(key)),
                    nurt: None,
                    yq: Some(value.clone()),
                });
            }
        }
        (Json::Array(left), Json::Array(right)) if left.len() == right.len() => {
            for (index, (value, other)) in left.iter().zip(right).enumerate() {
                compare(&format!("{pointer}/{index}"), value, other, out);
            }
        }
        (Json::Number(left), Json::Number(right)) if same_number(left, right) => {}
        _ if nurt == yq => {}
        _ => out.push(Mismatch::Value {
            pointer: pointer.to_owned(),
            nurt: Some(nurt.clone()),
            yq: Some(yq.clone()),
        }),
    }
}

fn mismatches(format: &Format, bytes: &[u8], recorded: &Result<Json, String>) -> Vec<Mismatch> {
    match ((format.read)(bytes), recorded) {
        (Ok(nurt), Ok(yq)) => {
            let mut out = Vec::new();
            compare("", &nurt, yq, &mut out);
            out
        }
        (Err(error), Ok(_)) => vec![Mismatch::NurtError(error)],
        (Ok(_), Err(message)) => vec![Mismatch::YqError(message.clone())],
        (Err(_), Err(_)) => Vec::new(),
    }
}

/// `true` when `text` contains a reference to a general entity other than
/// the predefined ones.
fn has_entity_reference(text: &str) -> bool {
    text.match_indices('&').any(|(index, _)| {
        let rest = &text[index + 1..];
        let len = rest
            .find(|c: char| !(c.is_alphanumeric() || matches!(c, '_' | '-' | '.' | ':')))
            .unwrap_or(rest.len());
        len > 0 && rest[len..].starts_with(';') && !PREDEFINED_ENTITIES.contains(&&rest[..len])
    })
}

/// Matches `text` against a pattern in which `*` stands for any sequence of
/// characters, `/` included.
fn glob_match(pattern: &str, text: &str) -> bool {
    let mut parts = pattern.split('*');
    let first = parts.next().unwrap_or_default();
    let Some(mut rest) = text.strip_prefix(first) else {
        return false;
    };
    let parts: Vec<&str> = parts.collect();
    let Some((last, middle)) = parts.split_last() else {
        return rest.is_empty();
    };
    for part in middle {
        match rest.find(part) {
            Some(index) => rest = &rest[index + part.len()..],
            None => return false,
        }
    }
    rest.ends_with(last)
}

/// Kinds of differences that a known difference accepts.
#[derive(Debug)]
enum Rule {
    /// `yq` keeps a reference to a declared entity where Nurt has its value.
    UnexpandedEntity,
    /// Nurt reports `E_UNSUPPORTED_CONSTRUCT` with the construct.
    UnsupportedConstruct(String),
    /// `yq` fails with a message containing the text.
    YqError(String),
    /// Values differ at a pointer matching the pattern.
    Value(String),
    /// Nurt keeps a number as its text where `yq` has the same number.
    NumberAsText,
    /// Nurt reads an empty table or map where `yq` writes no value.
    EmptyDocument,
    /// Both are floats that differ only by rounding of the last digit.
    FloatRounding,
}

impl Rule {
    /// `true` for a rule whose matches depend on the float parsing of
    /// `serde_json`, which a dependency may change with the `float_roundtrip`
    /// feature, so that the rule need not match any file.
    fn is_optional(&self) -> bool {
        matches!(self, Rule::FloatRounding)
    }
}

/// Relative difference of two floats that differ only by rounding.
const FLOAT_ROUNDING: f64 = 1e-15;

fn rounding_only(nurt: &serde_json::Number, yq: &serde_json::Number) -> bool {
    match (nurt.as_f64(), yq.as_f64()) {
        (Some(left), Some(right)) => {
            (left - right).abs() <= FLOAT_ROUNDING * left.abs().max(right.abs())
        }
        _ => false,
    }
}

/// Entry of `known-differences.toml`.
#[derive(Debug)]
struct Known {
    id: String,
    format: String,
    class: String,
    rule: Rule,
    paths: Vec<String>,
}

impl Known {
    fn covers(&self, path: &str, mismatch: &Mismatch) -> bool {
        if !self.paths.is_empty() && !self.paths.iter().any(|pattern| glob_match(pattern, path)) {
            return false;
        }
        match (&self.rule, mismatch) {
            (
                Rule::UnexpandedEntity,
                Mismatch::Value {
                    nurt: Some(_),
                    yq: Some(Json::String(yq)),
                    ..
                },
            ) => has_entity_reference(yq),
            (Rule::UnsupportedConstruct(construct), Mismatch::NurtError(error)) => {
                error.code() == ErrorCode::UnsupportedConstruct
                    && error.data().found.as_deref() == Some(construct.as_str())
            }
            (Rule::YqError(text), Mismatch::YqError(message)) => message.contains(text.as_str()),
            (Rule::Value(pattern), Mismatch::Value { pointer, .. }) => glob_match(pattern, pointer),
            (
                Rule::NumberAsText,
                Mismatch::Value {
                    nurt: Some(Json::String(text)),
                    yq: Some(Json::Number(number)),
                    ..
                },
            ) => text.parse::<f64>().ok() == number.as_f64(),
            (
                Rule::EmptyDocument,
                Mismatch::Value {
                    pointer,
                    nurt: Some(nurt),
                    yq: Some(Json::Null),
                },
            ) => pointer.is_empty() && nurt.as_object().is_some_and(serde_json::Map::is_empty),
            (
                Rule::FloatRounding,
                Mismatch::Value {
                    nurt: Some(Json::Number(nurt)),
                    yq: Some(Json::Number(yq)),
                    ..
                },
            ) => rounding_only(nurt, yq),
            _ => false,
        }
    }
}

fn known_differences_document() -> toml_edit::DocumentMut {
    let path = data_root().join("differential/known-differences.toml");
    std::fs::read_to_string(path).unwrap().parse().unwrap()
}

fn text_field(table: &toml_edit::Table, name: &str) -> Option<String> {
    table
        .get(name)
        .and_then(|item| item.as_str())
        .map(str::to_owned)
}

fn known_difference(table: &toml_edit::Table) -> Known {
    let id = text_field(table, "id").unwrap();
    let field = |name: &str| {
        text_field(table, name).unwrap_or_else(|| panic!("{id}: field `{name}` is required"))
    };
    let rule = match field("rule").as_str() {
        "unexpanded-entity" => Rule::UnexpandedEntity,
        "unsupported-construct" => Rule::UnsupportedConstruct(field("construct")),
        "yq-error" => Rule::YqError(field("error")),
        "value" => Rule::Value(field("pointer")),
        "number-as-text" => Rule::NumberAsText,
        "empty-document" => Rule::EmptyDocument,
        "float-rounding" => Rule::FloatRounding,
        other => panic!("{id}: unknown rule `{other}`"),
    };
    let class = field("class");
    assert!(
        CLASSES.contains(&class.as_str()),
        "{id}: unknown class `{class}`"
    );
    assert!(!field("reason").is_empty(), "{id}: the reason is empty");
    let paths = table
        .get("paths")
        .and_then(|item| item.as_array())
        .map(|array| {
            array
                .iter()
                .map(|value| value.as_str().unwrap().to_owned())
                .collect()
        })
        .unwrap_or_default();
    Known {
        format: field("format"),
        id,
        class,
        rule,
        paths,
    }
}

fn known_differences(format: &str) -> Vec<Known> {
    let document = known_differences_document();
    document["difference"]
        .as_array_of_tables()
        .unwrap()
        .iter()
        .map(known_difference)
        .filter(|known| known.format == format)
        .collect()
}

fn shorten(value: Option<&Json>) -> String {
    let text = value.map_or_else(|| "(missing)".to_owned(), Json::to_string);
    if text.chars().count() <= SHOWN_VALUE_CHARS {
        return text;
    }
    let mut short: String = text.chars().take(SHOWN_VALUE_CHARS).collect();
    short.push_str("...");
    short
}

fn describe(path: &str, mismatch: &Mismatch) -> String {
    match mismatch {
        Mismatch::Value { pointer, nurt, yq } => format!(
            "{path} {pointer}: nurt {} yq {}",
            shorten(nurt.as_ref()),
            shorten(yq.as_ref())
        ),
        Mismatch::NurtError(error) => format!(
            "{path}: nurt fails with {} ({}): {error}",
            error.code(),
            error.data().found.as_deref().unwrap_or("-")
        ),
        Mismatch::YqError(message) => format!("{path}: yq fails: {message}"),
    }
}

fn check_format(format: &Format) {
    let known = known_differences(format.name);
    let records = records(format);
    let recorded: BTreeSet<String> = records.iter().map(|record| record.path.clone()).collect();
    assert_eq!(
        corpus_files(format),
        recorded,
        "the records of yq do not list the corpus files; record the output of yq for the corpus again"
    );
    let mut used = vec![0_usize; known.len()];
    let mut classes: BTreeMap<String, usize> = BTreeMap::new();
    let mut unclassified: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for record in &records {
        let bytes = std::fs::read(data_root().join(&record.path)).unwrap();
        for mismatch in mismatches(format, &bytes, &record.outcome) {
            match known
                .iter()
                .position(|entry| entry.covers(&record.path, &mismatch))
            {
                Some(index) => {
                    used[index] += 1;
                    *classes.entry(known[index].class.clone()).or_default() += 1;
                }
                None => unclassified
                    .entry(record.path.clone())
                    .or_default()
                    .push(describe(&record.path, &mismatch)),
            }
        }
    }
    let rules: BTreeMap<&str, usize> = known
        .iter()
        .zip(&used)
        .map(|(entry, count)| (entry.id.as_str(), *count))
        .collect();
    u08_support::write_summary(
        &format!("u08-differential-{}.json", format.name),
        &serde_json::json!({
            "files": records.len(),
            "classified": classes,
            "rules": rules,
            "unclassified": unclassified,
        }),
    );
    assert!(
        unclassified.is_empty(),
        "unclassified differences in {} files, the first of each file:\n{}",
        unclassified.len(),
        unclassified
            .values()
            .take(SHOWN_DIFFERENCES)
            .map(|found| format!("({} in the file) {}", found.len(), found[0]))
            .collect::<Vec<_>>()
            .join("\n")
    );
    let unused: Vec<&str> = known
        .iter()
        .zip(&used)
        .filter(|(entry, count)| **count == 0 && !entry.rule.is_optional())
        .map(|(entry, _)| entry.id.as_str())
        .collect();
    assert!(
        unused.is_empty(),
        "known differences without a match: {unused:?}"
    );
}

fn installed_yq() -> Option<PathBuf> {
    let name = if cfg!(windows) { "yq.exe" } else { "yq" };
    let paths = std::env::var_os("PATH")?;
    std::env::split_paths(&paths)
        .map(|dir| dir.join(name))
        .find(|path| path.is_file())
}

fn recorded_yq_version() -> String {
    known_differences_document()["yq_version"]
        .as_str()
        .unwrap()
        .to_owned()
}

/// `yq` from `PATH` when its version is the recorded one.
fn recorded_yq() -> Option<PathBuf> {
    let yq = installed_yq()?;
    let output = Command::new(&yq).arg("--version").output().ok()?;
    let version = String::from_utf8_lossy(&output.stdout).into_owned();
    version
        .split_whitespace()
        .any(|word| word.trim_start_matches('v') == recorded_yq_version())
        .then_some(yq)
}

fn run_yq(yq: &Path, format: &Format, path: &str) -> Result<Json, String> {
    let output = Command::new(yq)
        .args(["-p", format.name, "-o", "json", "-I0", path])
        .current_dir(data_root())
        .output()
        .unwrap();
    if !output.status.success() {
        let message = String::from_utf8_lossy(&output.stderr);
        return Err(message.lines().next().unwrap_or_default().trim().to_owned());
    }
    let text = String::from_utf8(output.stdout).unwrap();
    Ok(stream_value(
        text.lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| serde_json::from_str(line).unwrap())
            .collect(),
    ))
}

fn check_records(format: &Format) {
    let Some(yq) = recorded_yq() else {
        return;
    };
    let stale: Vec<String> = records(format)
        .into_iter()
        .filter(
            |record| match (run_yq(&yq, format, &record.path), &record.outcome) {
                (Ok(value), Ok(recorded)) => value != *recorded,
                (Err(_), Err(_)) => false,
                _ => true,
            },
        )
        .map(|record| record.path)
        .collect();
    assert!(
        stale.is_empty(),
        "the records differ from the output of yq; record them again: {stale:?}"
    );
}

#[test]
fn xml_corpus_matches_recorded_yq_output() {
    check_format(&XML);
}

#[test]
fn json_corpus_matches_recorded_yq_output() {
    check_format(&JSON);
}

#[test]
fn toml_corpus_matches_recorded_yq_output() {
    check_format(&TOML);
}

#[test]
fn yaml_corpus_matches_recorded_yq_output() {
    check_format(&YAML);
}

#[test]
fn yaml_records_match_yq_when_the_recorded_yq_version_is_installed() {
    check_records(&YAML);
}

#[test]
fn streams_of_several_documents_are_lists_of_document_values() {
    assert_eq!(stream_value(Vec::new()), Json::Null);
    assert_eq!(
        stream_value(vec![serde_json::json!([1])]),
        serde_json::json!([1])
    );
    assert_eq!(
        read_yaml(b"a: 1\n---\n- 2\n").unwrap(),
        serde_json::json!([{"a": 1}, [2]])
    );
    assert_eq!(read_yaml(b"# comment only\n").unwrap(), Json::Null);
}

#[test]
fn xml_records_match_yq_when_the_recorded_yq_version_is_installed() {
    check_records(&XML);
}

#[test]
fn json_records_match_yq_when_the_recorded_yq_version_is_installed() {
    check_records(&JSON);
}

#[test]
fn toml_records_match_yq_when_the_recorded_yq_version_is_installed() {
    check_records(&TOML);
}

#[test]
fn comparison_reports_each_differing_member() {
    let nurt = serde_json::json!({"a": {"b": 1, "c": "x"}, "n": 1.0, "l": [1, 2]});
    let yq = serde_json::json!({"a": {"b": 2, "d/e": "y"}, "n": 1, "l": [1, 2, 3]});
    let mut out = Vec::new();
    compare("", &nurt, &yq, &mut out);
    let pointers: Vec<&str> = out
        .iter()
        .map(|mismatch| match mismatch {
            Mismatch::Value { pointer, .. } => pointer.as_str(),
            _ => "",
        })
        .collect();
    assert_eq!(pointers, ["/a/b", "/a/c", "/a/d~1e", "/l"]);
}

#[test]
fn entity_references_and_patterns_are_recognized() {
    assert!(has_entity_reference("Gateway &version;"));
    assert!(!has_entity_reference("a &amp; b &lt;"));
    assert!(!has_entity_reference("a & b; c"));
    assert!(glob_match(
        "corpus/xml/*/dtd/*",
        "corpus/xml/advanced/dtd/x.xml"
    ));
    assert!(glob_match("*", "anything"));
    assert!(!glob_match("corpus/json/*", "corpus/xml/a.xml"));
    assert!(glob_match("/a/*/b", "/a/1/2/b"));
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
