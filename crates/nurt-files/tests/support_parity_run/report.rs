// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Report of a scenario run and evaluation of the `fields` assertions.
//!
//! The report follows the effect report of the engine (status, error,
//! changes, notable codes, irreversible effects) and adds the records of the
//! last stage. Field paths of the scenarios name the report in the
//! vocabulary of the previous implementation; [`resolve`] maps that
//! vocabulary onto the report and returns an explanation for fields without
//! a counterpart.

use serde_json::{Value as Json, json};

/// Item of the record stream of the last stage.
#[derive(Clone, Debug)]
pub(crate) enum Item {
    /// Record of the given kind in its plain JSON form.
    Record(String, Json),
    /// Error item of the stream.
    Error(Json),
}

impl Item {
    fn json(&self) -> Json {
        match self {
            Self::Record(_, body) => body.clone(),
            Self::Error(error) => json!({ "error": error }),
        }
    }
}

/// Outcome of a program on the stages of the crate.
#[derive(Clone, Debug, Default)]
pub(crate) struct Report {
    /// `ok`, `planned`, `failed` or `rolled_back`.
    pub(crate) status: String,
    /// Error that ended the program.
    pub(crate) error: Option<Json>,
    /// Codes of unusual effects.
    pub(crate) notable: Vec<String>,
    /// Changes in their plain JSON form.
    pub(crate) changes: Vec<Json>,
    /// Number of irreversible effects.
    pub(crate) irreversible: usize,
    /// Records and error items of the last stage.
    pub(crate) items: Vec<Item>,
    /// Single value of the last stage.
    pub(crate) value: Option<Json>,
    /// The last stage was an effect stage.
    pub(crate) effect: bool,
}

/// Status of a successful program.
pub(crate) const STATUS_OK: &str = "ok";
/// Status of a preview.
pub(crate) const STATUS_PLANNED: &str = "planned";
/// Status of a program that stopped with an error.
pub(crate) const STATUS_FAILED: &str = "failed";
/// Status of an effect reverted after a part of it was applied.
pub(crate) const STATUS_ROLLED_BACK: &str = "rolled_back";

impl Report {
    /// Replaces the root directory in every text of the report by a
    /// relative form, as the scenarios name files relative to the root.
    pub(crate) fn relative_to(mut self, root: &str) -> Self {
        let strip = |value: &mut Json| relativize(value, root);
        self.error.iter_mut().for_each(strip);
        self.changes.iter_mut().for_each(strip);
        self.value.iter_mut().for_each(strip);
        for item in &mut self.items {
            match item {
                Item::Record(_, body) | Item::Error(body) => relativize(body, root),
            }
        }
        self
    }

    /// Makes the first error item the error of the program when the stream
    /// holds no record, as the executor ends such a program with that error.
    pub(crate) fn promote_stream_error(&mut self) {
        let records = self
            .items
            .iter()
            .any(|item| matches!(item, Item::Record(..)));
        if self.error.is_none() && !records {
            let first = self.items.iter().find_map(|item| match item {
                Item::Error(error) => Some(error.clone()),
                Item::Record(..) => None,
            });
            if let Some(error) = first {
                self.error = Some(error);
                self.status = STATUS_FAILED.to_owned();
            }
        }
    }

    fn records(&self, kinds: Option<&[&str]>) -> Vec<Json> {
        self.items
            .iter()
            .filter(|item| match (item, kinds) {
                (Item::Record(kind, _), Some(kinds)) => kinds.contains(&kind.as_str()),
                _ => true,
            })
            .map(|item| match item {
                Item::Record(kind, body) => vocabulary(kind, body.clone()),
                Item::Error(_) => item.json(),
            })
            .collect()
    }

    /// Number of matches: match records, or the sum of the match counts of
    /// the entries of the `files` output.
    fn match_total(&self) -> usize {
        let matches = self.records(Some(&["match"])).len();
        if matches > 0 {
            return matches;
        }
        self.records(Some(&["entry"]))
            .iter()
            .filter_map(|entry| entry.get("match_count").and_then(Json::as_u64))
            .filter_map(|count| usize::try_from(count).ok())
            .sum()
    }

    fn record_count(&self) -> usize {
        self.items
            .iter()
            .filter(|item| matches!(item, Item::Record(..)))
            .count()
    }

    /// Status in the vocabulary of the scenarios.
    fn status_word(&self) -> &'static str {
        match self.status.as_str() {
            STATUS_PLANNED => "previewed",
            STATUS_OK if self.notable.iter().any(|code| code == "unchanged") => "unchanged",
            STATUS_OK => "applied",
            _ => "rolled_back",
        }
    }
}

/// Fields of a record under the names of the scenario vocabulary.
///
/// A content record gives `range` as its shown ranges joined by commas, or
/// the whole file when no range was selected, and `lines` as the number of
/// lines of the file, which is what the previous implementation reported in
/// both fields; a hash `algorithm:digest` becomes an object with
/// `algorithm` and `digest`.
fn vocabulary(kind: &str, mut body: Json) -> Json {
    let Json::Object(fields) = &mut body else {
        return body;
    };
    if kind == "content" {
        let total = fields.get("total_lines").cloned();
        if !fields.contains_key("range") {
            let range = match (fields.get("ranges"), total.as_ref().and_then(Json::as_u64)) {
                (Some(Json::Array(ranges)), _) => Some(
                    ranges
                        .iter()
                        .filter_map(Json::as_str)
                        .collect::<Vec<_>>()
                        .join(","),
                ),
                (None, Some(lines)) if lines > 0 => Some(format!("1..{lines}")),
                _ => None,
            };
            if let Some(range) = range {
                fields.insert("range".to_owned(), Json::String(range));
            }
        }
        if let Some(total) = total {
            fields.insert("lines".to_owned(), total);
        }
    }
    if let Some(Json::String(hash)) = fields.get("hash").cloned()
        && let Some((algorithm, digest)) = hash.split_once(':')
    {
        fields.insert(
            "hash".to_owned(),
            json!({ "algorithm": algorithm, "digest": digest }),
        );
    }
    body
}

fn relativize(value: &mut Json, root: &str) {
    match value {
        Json::String(text) => {
            if text == root {
                *text = ".".to_owned();
            } else {
                *text = text.replace(&format!("{root}/"), "");
            }
        }
        Json::Array(items) => items.iter_mut().for_each(|item| relativize(item, root)),
        Json::Object(fields) => fields.values_mut().for_each(|item| relativize(item, root)),
        _ => {}
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Seg {
    Name(String),
    Index(usize),
    All,
}

fn segments(field: &str) -> Result<Vec<Seg>, String> {
    let mut out = Vec::new();
    for part in field.split('.') {
        let (name, mut rest) = part
            .split_once('[')
            .map_or((part, ""), |(name, rest)| (name, rest));
        if !name.is_empty() {
            out.push(Seg::Name(name.to_owned()));
        }
        while !rest.is_empty() {
            let (index, tail) = rest
                .split_once(']')
                .ok_or_else(|| format!("unbalanced brackets in {field}"))?;
            out.push(if index == "*" {
                Seg::All
            } else {
                Seg::Index(
                    index
                        .parse()
                        .map_err(|_| format!("invalid index in {field}"))?,
                )
            });
            rest = tail.strip_prefix('[').unwrap_or(tail);
        }
    }
    Ok(out)
}

/// Record kinds named by the roots of field paths.
const RECORD_ROOTS: [(&str, Option<&[&str]>); 10] = [
    ("content", Some(&["content"])),
    ("item", Some(&["item"])),
    ("entries", Some(&["entry"])),
    ("entry", Some(&["entry"])),
    ("matches", Some(&["match"])),
    ("cells", Some(&["cell"])),
    ("files", None),
    ("counts", None),
    ("outline", None),
    ("lines", Some(&["line"])),
];

/// Roots without a counterpart and the reason.
const UNSUPPORTED_ROOTS: [(&str, &str); 6] = [
    ("next_cursor", "paging is applied by the executor"),
    ("undoable", "the report carries no undo state"),
    ("preview_etag", "the report has no entity tag of a preview"),
    ("per_pattern", "matches carry no per-pattern totals"),
    ("check_references", "the report does not echo parameters"),
    ("warnings", "the report has no warnings list"),
];

/// Value of the field path `field` of the scenario vocabulary.
///
/// # Errors
///
/// Returns why the field has no counterpart in the report.
pub(crate) fn resolve(report: &Report, field: &str) -> Result<Json, String> {
    let segs = segments(field)?;
    let Some((Seg::Name(root), rest)) = segs.split_first() else {
        return Err(format!("field `{field}` does not start with a name"));
    };
    if let Some((_, reason)) = UNSUPPORTED_ROOTS.iter().find(|(name, _)| name == root) {
        return Err(format!("field `{root}`: {reason}"));
    }
    let count = |n: usize| Json::from(n);
    match root.as_str() {
        "error" => Ok(navigate(report.error.clone().unwrap_or(Json::Null), rest)),
        "notable" if rest.is_empty() => Ok(Json::from(report.notable.clone())),
        "effect" => match rest.split_first() {
            Some((Seg::Name(name), [])) if name == "status" => Ok(Json::from(report.status_word())),
            Some((Seg::Name(name), tail)) if name == "change" => Ok(navigate(
                report.changes.first().cloned().unwrap_or(Json::Null),
                tail,
            )),
            Some((Seg::Name(name), [])) if name == "encoding" => Ok(navigate(
                report.changes.first().cloned().unwrap_or(Json::Null),
                &[Seg::Name("encoding_after".to_owned())],
            )),
            _ => Err(format!(
                "field `{field}` has no counterpart in the change records"
            )),
        },
        "preview" if rest.is_empty() => Ok(Json::Bool(report.status == STATUS_PLANNED)),
        "targets" if rest.is_empty() => Ok(count(report.changes.len())),
        "irreversible" if rest.is_empty() => Ok(Json::Bool(report.irreversible > 0)),
        "total" | "shown" if rest.is_empty() => Ok(count(if report.effect {
            report.changes.len()
        } else {
            report.record_count()
        })),
        "total_matches" if rest.is_empty() => Ok(count(report.match_total())),
        "count" if rest.is_empty() => Ok(report
            .value
            .clone()
            .unwrap_or_else(|| count(report.record_count()))),
        "skipped" => Err(format!(
            "field `{field}`: skipped files are not counted in the records"
        )),
        name => match RECORD_ROOTS.iter().find(|(root, _)| *root == name) {
            Some((_, kinds)) => Ok(navigate_list(report.records(*kinds), rest)),
            None => Err(format!("field `{field}` has no counterpart in the report")),
        },
    }
}

fn navigate_list(items: Vec<Json>, rest: &[Seg]) -> Json {
    match rest.split_first() {
        None => Json::Array(items),
        Some((Seg::Index(index), tail)) => {
            navigate(items.get(*index).cloned().unwrap_or(Json::Null), tail)
        }
        Some((Seg::All, tail)) => {
            Json::Array(items.into_iter().map(|item| navigate(item, tail)).collect())
        }
        Some((Seg::Name(name), [])) if name == "count" => Json::from(items.len()),
        Some((Seg::Name(_), _)) => navigate(items.into_iter().next().unwrap_or(Json::Null), rest),
    }
}

fn navigate(value: Json, rest: &[Seg]) -> Json {
    let Some((first, tail)) = rest.split_first() else {
        return value;
    };
    match (first, value) {
        (Seg::Name(_), Json::Array(items)) if items.len() == 1 => {
            navigate(items.into_iter().next().unwrap_or(Json::Null), rest)
        }
        (Seg::Name(name), Json::Object(mut fields)) => {
            navigate(fields.remove(name).unwrap_or(Json::Null), tail)
        }
        (Seg::Index(index), Json::Array(items)) => {
            navigate(items.get(*index).cloned().unwrap_or(Json::Null), tail)
        }
        (Seg::All, Json::Array(items)) => {
            Json::Array(items.into_iter().map(|item| navigate(item, tail)).collect())
        }
        _ => Json::Null,
    }
}

/// Error codes of the previous implementation and the codes of the contract
/// that stand for them.
const ERROR_ALIASES: [(&str, &[&str]); 4] = [
    ("E_RANGE", &["E_OUT_OF_RANGE"]),
    ("E_EXPECT", &["E_ASSERT"]),
    (
        "E_ARGUMENT",
        &[
            "E_ARG",
            "E_ARITY",
            "E_MISSING_PARAM",
            "E_INVALID_VALUE",
            "E_PARAM_CONFLICT",
            "E_TYPE",
            "E_UNKNOWN_VALUE",
        ],
    ),
    (
        "E_SYNTAX",
        &["E_SYNTAX", "E_REGEX", "E_PATTERN_SYNTAX", "E_DOC_PARSE"],
    ),
];

fn range_text(text: &str) -> Option<String> {
    let parts: Vec<String> = text
        .split(',')
        .map(|part| match part.split_once("..") {
            Some((start, end)) if start == end && !start.is_empty() => start.to_owned(),
            _ => part.to_owned(),
        })
        .collect();
    (parts.join(",") != text).then(|| parts.join(","))
}

/// Equality of an expected and an actual value: numbers compare by value,
/// an absent value equals `false`, zero, an empty text and an empty list
/// (the report omits empty fields), an integer equals the length of a list,
/// and a range `a..a` equals its single-line form `a`.
pub(crate) fn same(expected: &Json, actual: &Json) -> bool {
    match (expected, actual) {
        (Json::Number(a), Json::Number(b)) => a.as_f64() == b.as_f64(),
        (Json::Bool(false), Json::Null) | (Json::Null, Json::Null) => true,
        (Json::Number(a), Json::Null) => a.as_f64() == Some(0.0),
        (Json::String(a), Json::Null) => a.is_empty(),
        (Json::Array(a), Json::Null) => a.is_empty(),
        (Json::Number(a), Json::Array(items)) => a.as_u64() == u64::try_from(items.len()).ok(),
        (Json::String(a), Json::String(b)) => {
            a == b || range_text(a).as_deref() == Some(b.as_str())
        }
        (Json::Array(a), Json::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(x, y)| same(x, y))
        }
        (Json::Object(a), Json::Object(b)) => {
            a.len() == b.len()
                && a.iter()
                    .all(|(key, x)| b.get(key).is_some_and(|y| same(x, y)))
        }
        (a, b) => a == b,
    }
}

fn contains(actual: &Json, expected: &Json) -> bool {
    match (actual, expected) {
        (Json::Array(items), _) => items.iter().any(|item| same(expected, item)),
        (Json::String(text), Json::String(fragment)) => text.contains(fragment.as_str()),
        _ => false,
    }
}

/// Kind of comparison of a field assertion.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Comparison {
    /// The field equals the value.
    Value,
    /// The field contains the value.
    Contains,
    /// The field does not contain the value.
    Excludes,
}

/// Checks one field assertion.
///
/// # Errors
///
/// `Err(Ok(message))` describes a difference; `Err(Err(reason))` explains
/// a field without a counterpart.
pub(crate) fn check_field(
    report: &Report,
    field: &str,
    comparison: Comparison,
    expected: &Json,
) -> Result<(), Result<String, String>> {
    let actual = resolve(report, field).map_err(Err)?;
    let is_error_code = field.ends_with("error.code");
    let ok = match comparison {
        Comparison::Value if is_error_code => error_code_matches(expected, &actual),
        Comparison::Value if field == "notable" && expected.is_string() => {
            contains(&actual, expected)
        }
        Comparison::Value => same(expected, &actual),
        Comparison::Contains => contains(&actual, expected),
        Comparison::Excludes => !contains(&actual, expected),
    };
    if ok {
        Ok(())
    } else {
        let mut message = format!("{field}: expected {comparison:?} {expected}, found {actual}");
        if is_error_code
            && let Some(error_message) = report.error.as_ref().and_then(|e| e.get("message"))
        {
            message.push_str(&format!(" ({error_message})"));
        }
        Err(Ok(message))
    }
}

fn error_code_matches(expected: &Json, actual: &Json) -> bool {
    let (Some(expected), Some(actual)) = (expected.as_str(), actual.as_str()) else {
        return same(expected, actual);
    };
    expected == actual
        || ERROR_ALIASES
            .iter()
            .any(|(name, codes)| *name == expected && codes.contains(&actual))
}
