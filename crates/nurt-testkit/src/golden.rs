// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Runner of golden files.
//!
//! A case directory holds pairs of files with a common case name:
//!
//! | File | Content |
//! |---|---|
//! | `<name>.in.json` | input as a JSON value ([`GoldenInput::Json`]) |
//! | `<name>.in.<ext>` with any other extension | input as UTF-8 text ([`GoldenInput::Text`]) |
//! | `<name>.out.json` | expected output as a JSON value |
//!
//! [`GoldenRunner::run`] calls a function for every case in byte order of
//! the case names and compares its result with the expected value as JSON
//! values (key order and formatting do not matter). Differences are listed
//! by JSON pointer. When the environment variable [`BLESS_VAR`] is `1`, the
//! runner writes the actual result to the expected file instead of failing.
//! A [`case::Normalizer`] set with [`GoldenRunner::normalizer`] replaces
//! volatile values on both sides before the comparison.
//!
//! Case directories with one directory per case (`case.toml`, program
//! variants, expected trees and reports) are run by [`case::CaseRunner`].

pub mod case;

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

/// Environment variable that switches the runner to writing expected files.
pub const BLESS_VAR: &str = "NURT_BLESS";

/// Suffix of input files before the extension.
pub const INPUT_MARK: &str = ".in.";

/// Suffix of expected files.
pub const EXPECTED_SUFFIX: &str = ".out.json";

/// Largest number of differences listed per case.
pub const MAX_LISTED_DIFFERENCES: usize = 50;

/// Input of one case.
#[derive(Debug, Clone, PartialEq)]
pub enum GoldenInput {
    /// Content of a `.in.json` file.
    Json(Value),
    /// Content of any other input file.
    Text(String),
}

/// One case of a golden directory.
#[derive(Debug, Clone, PartialEq)]
pub struct GoldenCase {
    /// Case name: the file name before [`INPUT_MARK`].
    pub name: String,
    /// Input file.
    pub input_path: PathBuf,
    /// Input.
    pub input: GoldenInput,
    /// Expected file; it need not exist.
    pub expected_path: PathBuf,
}

/// Failure of the runner.
#[derive(Clone, PartialEq)]
#[non_exhaustive]
pub enum GoldenError {
    /// A file or the directory could not be read or written.
    Io {
        /// File or directory.
        path: PathBuf,
        /// System message.
        message: String,
    },
    /// A JSON file did not parse, or a result could not be serialized.
    Json {
        /// File.
        path: PathBuf,
        /// Parser message.
        message: String,
    },
    /// The directory holds no case.
    NoCases {
        /// Directory.
        dir: PathBuf,
    },
    /// Results differ from the expected files, or expected files are
    /// missing.
    Mismatch {
        /// Failed cases in case order.
        failures: Vec<GoldenFailure>,
    },
}

/// Failed case.
#[derive(Debug, Clone, PartialEq)]
pub struct GoldenFailure {
    /// Case name.
    pub name: String,
    /// Expected file.
    pub expected_path: PathBuf,
    /// Differences, one line each; a single line for a missing file.
    pub differences: Vec<String>,
}

impl fmt::Display for GoldenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, message } => write!(f, "cannot access {}: {message}", path.display()),
            Self::Json { path, message } => {
                write!(f, "invalid JSON in {}: {message}", path.display())
            }
            Self::NoCases { dir } => write!(f, "no golden case in {}", dir.display()),
            Self::Mismatch { failures } => {
                writeln!(
                    f,
                    "{} golden case(s) failed; set {BLESS_VAR}=1 to accept the actual results",
                    failures.len()
                )?;
                for failure in failures {
                    writeln!(
                        f,
                        "case `{}` ({}):",
                        failure.name,
                        failure.expected_path.display()
                    )?;
                    for line in &failure.differences {
                        writeln!(f, "  {line}")?;
                    }
                }
                Ok(())
            }
        }
    }
}

impl fmt::Debug for GoldenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl std::error::Error for GoldenError {}

/// Result of a run without failures.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GoldenReport {
    /// Cases whose result matched.
    pub passed: Vec<String>,
    /// Cases whose expected file was written.
    pub blessed: Vec<String>,
}

/// Runner over one case directory.
///
/// ```
/// use nurt_testkit::golden::{GoldenInput, GoldenRunner};
///
/// let dir = tempfile::tempdir().unwrap();
/// std::fs::write(dir.path().join("sum.in.json"), "[1, 2, 3]").unwrap();
/// std::fs::write(dir.path().join("sum.out.json"), "{\"sum\": 6}").unwrap();
///
/// let report = GoldenRunner::new(dir.path())
///     .bless(false)
///     .run(|case| match &case.input {
///         GoldenInput::Json(value) => {
///             let sum: i64 = value
///                 .as_array()
///                 .map(|items| items.iter().filter_map(|item| item.as_i64()).sum())
///                 .unwrap_or(0);
///             serde_json::json!({ "sum": sum })
///         }
///         GoldenInput::Text(_) => serde_json::Value::Null,
///     })
///     .unwrap();
/// assert_eq!(report.passed, ["sum"]);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoldenRunner {
    dir: PathBuf,
    bless: bool,
    normalizer: case::Normalizer,
}

impl GoldenRunner {
    /// Runner over `dir` without normalization; writing expected files is
    /// enabled when [`BLESS_VAR`] is `1`.
    #[must_use]
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        let bless = std::env::var_os(BLESS_VAR).is_some_and(|value| value == "1");
        Self {
            dir: dir.into(),
            bless,
            normalizer: case::Normalizer::none(),
        }
    }

    /// Normalizes expected and actual values with `normalizer` before they
    /// are compared; written expected files are not normalized.
    #[must_use]
    pub fn normalizer(mut self, normalizer: case::Normalizer) -> Self {
        self.normalizer = normalizer;
        self
    }

    /// Enables or disables writing expected files, overriding [`BLESS_VAR`].
    #[must_use]
    pub fn bless(mut self, bless: bool) -> Self {
        self.bless = bless;
        self
    }

    /// Cases of the directory in byte order of their names.
    ///
    /// Fails with [`GoldenError::Io`] when the directory or an input file
    /// cannot be read, with [`GoldenError::Json`] for an input JSON file that
    /// does not parse, and with [`GoldenError::NoCases`] for a directory
    /// without input files.
    pub fn cases(&self) -> Result<Vec<GoldenCase>, GoldenError> {
        let entries = fs::read_dir(&self.dir).map_err(|error| io_error(&self.dir, &error))?;
        let mut cases = BTreeMap::new();
        for entry in entries {
            let entry = entry.map_err(|error| io_error(&self.dir, &error))?;
            let file_name = entry.file_name().to_string_lossy().into_owned();
            let Some((name, extension)) = file_name.split_once(INPUT_MARK) else {
                continue;
            };
            let input_path = entry.path();
            if cases.contains_key(name) {
                // The order of a directory listing is not defined, so a case
                // with two inputs would run with either of them.
                return Err(GoldenError::Io {
                    path: input_path,
                    message: format!("the case `{name}` has more than one input file"),
                });
            }
            let text =
                fs::read_to_string(&input_path).map_err(|error| io_error(&input_path, &error))?;
            let input = if extension == "json" {
                GoldenInput::Json(parse(&input_path, &text)?)
            } else {
                GoldenInput::Text(text)
            };
            let expected_path = self.dir.join(format!("{name}{EXPECTED_SUFFIX}"));
            cases.insert(
                name.to_owned(),
                GoldenCase {
                    name: name.to_owned(),
                    input_path,
                    input,
                    expected_path,
                },
            );
        }
        if cases.is_empty() {
            return Err(GoldenError::NoCases {
                dir: self.dir.clone(),
            });
        }
        Ok(cases.into_values().collect())
    }

    /// Runs `produce` for every case and compares the results with the
    /// expected files, or writes them when blessing.
    ///
    /// Fails as [`GoldenRunner::cases`], with [`GoldenError::Io`] when an
    /// expected file cannot be read or written, and with
    /// [`GoldenError::Mismatch`] listing every failed case.
    pub fn run<F>(&self, mut produce: F) -> Result<GoldenReport, GoldenError>
    where
        F: FnMut(&GoldenCase) -> Value,
    {
        let mut report = GoldenReport::default();
        let mut failures = Vec::new();
        for case in self.cases()? {
            let actual = produce(&case);
            if self.bless {
                write_expected(&case.expected_path, &actual)?;
                report.blessed.push(case.name);
                continue;
            }
            let expected = match fs::read_to_string(&case.expected_path) {
                Ok(text) => parse(&case.expected_path, &text)?,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    failures.push(GoldenFailure {
                        name: case.name,
                        expected_path: case.expected_path,
                        differences: vec!["expected file is missing".to_owned()],
                    });
                    continue;
                }
                Err(error) => return Err(io_error(&case.expected_path, &error)),
            };
            let differences = diff(
                &self.normalizer.normalize(&expected),
                &self.normalizer.normalize(&actual),
            );
            if differences.is_empty() {
                report.passed.push(case.name);
            } else {
                failures.push(GoldenFailure {
                    name: case.name,
                    expected_path: case.expected_path,
                    differences,
                });
            }
        }
        if failures.is_empty() {
            Ok(report)
        } else {
            Err(GoldenError::Mismatch { failures })
        }
    }
}

fn io_error(path: &Path, error: &std::io::Error) -> GoldenError {
    GoldenError::Io {
        path: path.to_path_buf(),
        message: error.to_string(),
    }
}

fn parse(path: &Path, text: &str) -> Result<Value, GoldenError> {
    serde_json::from_str(text).map_err(|error| GoldenError::Json {
        path: path.to_path_buf(),
        message: error.to_string(),
    })
}

fn write_expected(path: &Path, value: &Value) -> Result<(), GoldenError> {
    let mut text = serde_json::to_string_pretty(value).map_err(|error| GoldenError::Json {
        path: path.to_path_buf(),
        message: error.to_string(),
    })?;
    text.push('\n');
    fs::write(path, text).map_err(|error| io_error(path, &error))
}

/// Differences between two JSON values, one line per difference, each
/// starting with the JSON pointer of the differing member (`(root)` for the
/// whole value). Object member order is ignored. No more than
/// [`MAX_LISTED_DIFFERENCES`] differences are listed; a final line reports
/// how many were omitted.
///
/// ```
/// use nurt_testkit::golden::diff;
/// use serde_json::json;
///
/// let lines = diff(&json!({"a": 1, "b": [1, 2]}), &json!({"b": [1, 3], "a": 1, "c": null}));
/// assert_eq!(lines, [
///     "/b/1: expected 2, actual 3",
///     "/c: unexpected member, actual null",
/// ]);
/// ```
#[must_use]
pub fn diff(expected: &Value, actual: &Value) -> Vec<String> {
    let mut lines = Vec::new();
    collect(String::new(), expected, actual, &mut lines);
    if lines.len() > MAX_LISTED_DIFFERENCES {
        let omitted = lines.len() - MAX_LISTED_DIFFERENCES;
        lines.truncate(MAX_LISTED_DIFFERENCES);
        lines.push(format!("{omitted} more difference(s) omitted"));
    }
    lines
}

fn shown(pointer: &str) -> &str {
    if pointer.is_empty() {
        "(root)"
    } else {
        pointer
    }
}

fn escape(key: &str) -> String {
    key.replace('~', "~0").replace('/', "~1")
}

fn collect(pointer: String, expected: &Value, actual: &Value, lines: &mut Vec<String>) {
    match (expected, actual) {
        (Value::Object(left), Value::Object(right)) => {
            let keys: std::collections::BTreeSet<&String> =
                left.keys().chain(right.keys()).collect();
            for key in keys {
                let child = format!("{pointer}/{}", escape(key));
                match (left.get(key), right.get(key)) {
                    (Some(left), Some(right)) => collect(child, left, right, lines),
                    (Some(left), None) => {
                        lines.push(format!("{child}: missing member, expected {left}"));
                    }
                    (None, Some(right)) => {
                        lines.push(format!("{child}: unexpected member, actual {right}"));
                    }
                    (None, None) => {}
                }
            }
        }
        (Value::Array(left), Value::Array(right)) => {
            for (index, (left, right)) in left.iter().zip(right).enumerate() {
                collect(format!("{pointer}/{index}"), left, right, lines);
            }
            if left.len() != right.len() {
                lines.push(format!(
                    "{}: expected {} item(s), actual {}",
                    shown(&pointer),
                    left.len(),
                    right.len()
                ));
            }
        }
        _ if expected != actual => {
            lines.push(format!(
                "{}: expected {expected}, actual {actual}",
                shown(&pointer)
            ));
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{MAX_LISTED_DIFFERENCES, diff};

    #[test]
    fn equal_values_have_no_differences() {
        assert!(
            diff(
                &json!({"a": [1, {"b": "c"}]}),
                &json!({"a": [1, {"b": "c"}]})
            )
            .is_empty()
        );
    }

    #[test]
    fn differences_name_the_pointer() {
        assert_eq!(
            diff(&json!({"a/b": {"c": 1}}), &json!({"a/b": {}})),
            ["/a~1b/c: missing member, expected 1"]
        );
        assert_eq!(
            diff(&json!(1), &json!("1")),
            ["(root): expected 1, actual \"1\""]
        );
        assert_eq!(
            diff(&json!([1]), &json!([1, 2])),
            ["(root): expected 1 item(s), actual 2"]
        );
    }

    #[test]
    fn long_lists_are_cut() {
        let expected = json!((0..100).collect::<Vec<i32>>());
        let actual = json!((1..101).collect::<Vec<i32>>());
        let lines = diff(&expected, &actual);
        assert_eq!(lines.len(), MAX_LISTED_DIFFERENCES + 1);
        assert_eq!(
            lines[MAX_LISTED_DIFFERENCES],
            "50 more difference(s) omitted"
        );
    }
}
