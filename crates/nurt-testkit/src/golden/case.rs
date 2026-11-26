// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Case directories: one directory per case, shared by the conformance
//! suite and the crate tests.
//!
//! ```text
//! <root>/<case>/case.toml       description and settings of the case
//! <root>/<case>/a.nurt          program, first syntax variant
//! <root>/<case>/b.nurt          the same program, second variant
//! <root>/<case>/c.nurt          the same program, third variant
//! <root>/<case>/fs_before.json  initial file tree (nurt-fs-snapshot/1)
//! <root>/<case>/ast.json        expected syntax tree
//! <root>/<case>/bound.json      expected bound tree
//! <root>/<case>/report.json     expected report
//! <root>/<case>/fs_after.json   expected final file tree (nurt-fs-snapshot/1)
//! ```
//!
//! Only `case.toml` is required. Its keys `description` (text) and `base`
//! (absolute path of the tree described by the snapshots) are read by the
//! runner; other keys are kept in [`CaseDir::config`] for the caller.
//!
//! [`CaseRunner::run`] calls a function for every case directory in byte
//! order of the directory names and compares every JSON file it produces
//! with the file of the same name in the case directory, after
//! [`Normalizer`] replaced volatile members on both sides. Expected files
//! that the function does not produce are not compared, so that different
//! callers check different stages of the same case. With blessing, the
//! produced files are written instead.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::{
    BLESS_VAR, GoldenError, GoldenFailure, GoldenReport, diff, io_error, parse, write_expected,
};
use crate::snapshot::FsSnapshot;

/// Name of the description file of a case directory.
pub const CASE_FILE: &str = "case.toml";

/// File holding the initial file tree of a case.
pub const FS_BEFORE_FILE: &str = "fs_before.json";

/// Syntax variants of the program of a case, by file stem.
pub const SOURCE_VARIANTS: [&str; 3] = ["a", "b", "c"];

/// Extension of program files.
pub const SOURCE_EXTENSION: &str = "nurt";

/// Text that replaces volatile values.
pub const VOLATILE: &str = "<volatile>";

/// Text that replaces the base path in strings.
pub const BASE_MARK: &str = "<base>";

/// Member names whose values differ between runs of the same program: the
/// operation number, entity tags, cursors, result files, timings, durations
/// and instants.
pub const VOLATILE_KEYS: [&str; 15] = [
    "op",
    "etag",
    "cursor",
    "next_cursor",
    "result_file",
    "timing",
    "elapsed",
    "duration_ms",
    "time",
    "started",
    "finished",
    "modified",
    "accessed",
    "created",
    "mtime",
];

/// Replaces volatile members and the base path before comparison.
///
/// A member whose name is listed has its value replaced by [`VOLATILE`],
/// unless the value is `null`; in every string the base path is replaced by
/// [`BASE_MARK`], also when written with `\` separators.
///
/// ```
/// use nurt_testkit::golden::case::Normalizer;
/// use serde_json::json;
///
/// let normalizer = Normalizer::default().with_base("/tmp/run-1");
/// let value = json!({"op": 7, "path": "/tmp/run-1/a.txt", "next_cursor": null});
/// assert_eq!(
///     normalizer.normalize(&value),
///     json!({"op": "<volatile>", "path": "<base>/a.txt", "next_cursor": null})
/// );
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Normalizer {
    keys: BTreeSet<String>,
    base: Option<String>,
}

impl Default for Normalizer {
    /// Normalizer of the members in [`VOLATILE_KEYS`] without a base path.
    fn default() -> Self {
        Self {
            keys: VOLATILE_KEYS.iter().map(|key| (*key).to_owned()).collect(),
            base: None,
        }
    }
}

impl Normalizer {
    /// Normalizer that changes nothing.
    #[must_use]
    pub fn none() -> Self {
        Self {
            keys: BTreeSet::new(),
            base: None,
        }
    }

    /// Also replaces the values of members named `key`.
    #[must_use]
    pub fn with_key(mut self, key: impl Into<String>) -> Self {
        self.keys.insert(key.into());
        self
    }

    /// Replaces `base` in strings by [`BASE_MARK`].
    #[must_use]
    pub fn with_base(mut self, base: impl Into<String>) -> Self {
        let base = base.into();
        self.base = (!base.is_empty()).then_some(base);
        self
    }

    /// The normalized copy of `value`.
    #[must_use]
    pub fn normalize(&self, value: &Value) -> Value {
        match value {
            Value::Object(members) => Value::Object(
                members
                    .iter()
                    .map(|(key, member)| {
                        let replaced = if self.keys.contains(key) && !member.is_null() {
                            Value::from(VOLATILE)
                        } else {
                            self.normalize(member)
                        };
                        (key.clone(), replaced)
                    })
                    .collect(),
            ),
            Value::Array(items) => {
                Value::Array(items.iter().map(|item| self.normalize(item)).collect())
            }
            Value::String(text) => Value::from(self.replace_base(text)),
            other => other.clone(),
        }
    }

    fn replace_base(&self, text: &str) -> String {
        let Some(base) = &self.base else {
            return text.to_owned();
        };
        let native = base.replace('/', "\\");
        replace_prefix_at_boundary(
            &replace_prefix_at_boundary(text, base.as_str()),
            native.as_str(),
        )
    }
}

/// Replaces `needle` by [`BASE_MARK`] where it ends a path component, so that
/// the base `/tmp/run-1` does not rewrite the start of `/tmp/run-10/a`.
fn replace_prefix_at_boundary(text: &str, needle: &str) -> String {
    let ends_with_separator = needle.ends_with(['/', '\\']);
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(needle) {
        let after = &rest[at + needle.len()..];
        let boundary = ends_with_separator
            || after
                .chars()
                .next()
                .is_none_or(|next| !(next.is_alphanumeric() || matches!(next, '-' | '_' | '.')));
        out.push_str(&rest[..at]);
        out.push_str(if boundary { BASE_MARK } else { needle });
        rest = after;
    }
    out.push_str(rest);
    out
}

/// One case directory.
#[derive(Debug, Clone, PartialEq)]
pub struct CaseDir {
    /// Case name: the directory name.
    pub name: String,
    /// The directory.
    pub dir: PathBuf,
    /// Description from `case.toml`.
    pub description: Option<String>,
    /// Base path from `case.toml`.
    pub base: Option<String>,
    /// All keys of `case.toml`.
    pub config: toml::Table,
    /// Program variants present, by variant (`a`, `b`, `c`), in variant
    /// order.
    pub sources: Vec<(String, String)>,
    /// Initial file tree, when `fs_before.json` exists.
    pub fs_before: Option<FsSnapshot>,
}

impl CaseDir {
    /// Program text of a variant.
    #[must_use]
    pub fn source(&self, variant: &str) -> Option<&str> {
        self.sources
            .iter()
            .find(|(name, _)| name == variant)
            .map(|(_, text)| text.as_str())
    }

    /// Path of a file of the case.
    #[must_use]
    pub fn file(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }

    /// Normalizer for this case: `base` of the case added to `normalizer`.
    #[must_use]
    pub fn normalizer(&self, normalizer: &Normalizer) -> Normalizer {
        match &self.base {
            Some(base) => normalizer.clone().with_base(base.clone()),
            None => normalizer.clone(),
        }
    }
}

/// JSON files produced for one case, by file name (`report.json`,
/// `fs_after.json`, ...).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CaseOutput {
    files: BTreeMap<String, Value>,
}

impl CaseOutput {
    /// Output without files.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds or replaces a produced file.
    #[must_use]
    pub fn with(mut self, name: impl Into<String>, value: Value) -> Self {
        self.files.insert(name.into(), value);
        self
    }

    /// Adds the final file tree as `fs_after.json`.
    #[must_use]
    pub fn with_fs_after(self, snapshot: &FsSnapshot) -> Self {
        let value = serde_json::to_value(snapshot).unwrap_or(Value::Null);
        self.with("fs_after.json", value)
    }

    /// Produced files in name order.
    pub fn files(&self) -> impl Iterator<Item = (&str, &Value)> {
        self.files
            .iter()
            .map(|(name, value)| (name.as_str(), value))
    }
}

/// Runner over a directory of case directories.
///
/// ```
/// use nurt_testkit::golden::case::{CaseOutput, CaseRunner};
/// use serde_json::json;
///
/// let root = tempfile::tempdir().unwrap();
/// let case = root.path().join("count-files");
/// std::fs::create_dir(&case).unwrap();
/// std::fs::write(case.join("case.toml"), "description = \"counts files\"\n").unwrap();
/// std::fs::write(case.join("a.nurt"), "find |> count\n").unwrap();
/// std::fs::write(case.join("report.json"), "{\"status\": \"ok\", \"op\": 3}").unwrap();
///
/// let report = CaseRunner::new(root.path())
///     .bless(false)
///     .run(|case| {
///         assert_eq!(case.source("a"), Some("find |> count\n"));
///         CaseOutput::new().with("report.json", json!({"status": "ok", "op": 9}))
///     })
///     .unwrap();
/// assert_eq!(report.passed, ["count-files"]);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaseRunner {
    root: PathBuf,
    bless: bool,
    normalizer: Normalizer,
}

fn read_optional(path: &Path) -> Result<Option<String>, GoldenError> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(io_error(path, &error)),
    }
}

fn load_case(name: String, dir: PathBuf) -> Result<CaseDir, GoldenError> {
    let case_path = dir.join(CASE_FILE);
    let text = fs::read_to_string(&case_path).map_err(|error| io_error(&case_path, &error))?;
    let config: toml::Table = toml::from_str(&text).map_err(|error| GoldenError::Json {
        path: case_path.clone(),
        message: error.to_string(),
    })?;
    let text_key = |key: &str| {
        config
            .get(key)
            .and_then(toml::Value::as_str)
            .map(str::to_owned)
    };
    let mut sources = Vec::new();
    for variant in SOURCE_VARIANTS {
        let path = dir.join(format!("{variant}.{SOURCE_EXTENSION}"));
        if let Some(text) = read_optional(&path)? {
            sources.push((variant.to_owned(), text));
        }
    }
    let before_path = dir.join(FS_BEFORE_FILE);
    let fs_before = read_optional(&before_path)?
        .map(|text| {
            FsSnapshot::from_json(&text).map_err(|error| GoldenError::Json {
                path: before_path.clone(),
                message: error.to_string(),
            })
        })
        .transpose()?;
    Ok(CaseDir {
        name,
        description: text_key("description"),
        base: text_key("base"),
        dir,
        config,
        sources,
        fs_before,
    })
}

impl CaseRunner {
    /// Runner over `root` with the default [`Normalizer`]; writing expected
    /// files is enabled when [`BLESS_VAR`] is `1`.
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        let bless = std::env::var_os(BLESS_VAR).is_some_and(|value| value == "1");
        Self {
            root: root.into(),
            bless,
            normalizer: Normalizer::default(),
        }
    }

    /// Enables or disables writing expected files, overriding [`BLESS_VAR`].
    #[must_use]
    pub fn bless(mut self, bless: bool) -> Self {
        self.bless = bless;
        self
    }

    /// Replaces the normalizer.
    #[must_use]
    pub fn normalizer(mut self, normalizer: Normalizer) -> Self {
        self.normalizer = normalizer;
        self
    }

    /// Case directories below the root, in byte order of their names;
    /// directories without `case.toml` are skipped.
    ///
    /// # Errors
    ///
    /// Fails with [`GoldenError::Io`] when a directory or file cannot be
    /// read, with [`GoldenError::Json`] for a `case.toml` that does not
    /// parse or an invalid `fs_before.json`, and with
    /// [`GoldenError::NoCases`] when no case directory exists.
    pub fn cases(&self) -> Result<Vec<CaseDir>, GoldenError> {
        let entries = fs::read_dir(&self.root).map_err(|error| io_error(&self.root, &error))?;
        let mut dirs = BTreeMap::new();
        for entry in entries {
            let entry = entry.map_err(|error| io_error(&self.root, &error))?;
            let path = entry.path();
            if path.join(CASE_FILE).is_file() {
                dirs.insert(entry.file_name().to_string_lossy().into_owned(), path);
            }
        }
        if dirs.is_empty() {
            return Err(GoldenError::NoCases {
                dir: self.root.clone(),
            });
        }
        dirs.into_iter()
            .map(|(name, dir)| load_case(name, dir))
            .collect()
    }

    /// Runs `produce` for every case and compares the produced files with
    /// the expected files, or writes them when blessing.
    ///
    /// # Errors
    ///
    /// Fails as [`CaseRunner::cases`], with [`GoldenError::Io`] when an
    /// expected file cannot be read or written, and with
    /// [`GoldenError::Mismatch`] listing every failed case; each difference
    /// starts with the file name.
    pub fn run<F>(&self, mut produce: F) -> Result<GoldenReport, GoldenError>
    where
        F: FnMut(&CaseDir) -> CaseOutput,
    {
        let mut report = GoldenReport::default();
        let mut failures = Vec::new();
        for case in self.cases()? {
            let output = produce(&case);
            if self.bless {
                for (name, value) in output.files() {
                    write_expected(&case.file(name), value)?;
                }
                report.blessed.push(case.name);
                continue;
            }
            let normalizer = case.normalizer(&self.normalizer);
            let mut differences = Vec::new();
            for (name, actual) in output.files() {
                let path = case.file(name);
                let Some(text) = read_optional(&path)? else {
                    differences.push(format!("{name}: expected file is missing"));
                    continue;
                };
                let expected = parse(&path, &text)?;
                differences.extend(
                    diff(
                        &normalizer.normalize(&expected),
                        &normalizer.normalize(actual),
                    )
                    .into_iter()
                    .map(|line| format!("{name}: {line}")),
                );
            }
            if differences.is_empty() {
                report.passed.push(case.name);
            } else {
                failures.push(GoldenFailure {
                    expected_path: case.dir.clone(),
                    name: case.name,
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

    /// Runs every program variant of every case through `call` and compares
    /// each answer, parsed as JSON, with `report.json`.
    ///
    /// `call` receives the case and the program text and returns the JSON
    /// text of the answer, as an engine call such as `run_json` does. All
    /// variants of a case must give the same report; a case without program
    /// files fails. An answer that is not JSON is compared as a JSON string.
    ///
    /// # Errors
    ///
    /// Fails as [`CaseRunner::run`].
    pub fn run_sources<F>(&self, mut call: F) -> Result<GoldenReport, GoldenError>
    where
        F: FnMut(&CaseDir, &str) -> String,
    {
        let mut failures = Vec::new();
        let mut report = GoldenReport::default();
        for case in self.cases()? {
            if case.sources.is_empty() {
                failures.push(GoldenFailure {
                    expected_path: case.dir.clone(),
                    name: case.name.clone(),
                    differences: vec!["case has no program file".to_owned()],
                });
                continue;
            }
            let path = case.file("report.json");
            let expected = match read_optional(&path)? {
                Some(text) => Some(parse(&path, &text)?),
                None => None,
            };
            let normalizer = case.normalizer(&self.normalizer);
            let mut differences = Vec::new();
            let mut first = None;
            for (variant, source) in &case.sources {
                let answer = call(&case, source);
                let actual = serde_json::from_str(&answer).unwrap_or(Value::String(answer));
                match (&expected, self.bless) {
                    (_, true) => {
                        if first.is_none() {
                            first = Some(actual);
                        }
                    }
                    (Some(expected), false) => differences.extend(
                        diff(
                            &normalizer.normalize(expected),
                            &normalizer.normalize(&actual),
                        )
                        .into_iter()
                        .map(|line| format!("{variant}.{SOURCE_EXTENSION}: {line}")),
                    ),
                    (None, false) => {
                        differences.push("report.json: expected file is missing".to_owned());
                        break;
                    }
                }
            }
            if let Some(value) = first {
                write_expected(&path, &value)?;
                report.blessed.push(case.name);
            } else if differences.is_empty() {
                report.passed.push(case.name);
            } else {
                failures.push(GoldenFailure {
                    expected_path: path,
                    name: case.name,
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

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use serde_json::json;

    use super::{CaseOutput, CaseRunner, Normalizer, VOLATILE};
    use crate::golden::GoldenError;
    use crate::snapshot::{FsSnapshot, SnapshotEntry};

    fn write_case(root: &Path, name: &str, files: &[(&str, &str)]) {
        let dir = root.join(name);
        fs::create_dir_all(&dir).unwrap();
        for (file, text) in files {
            fs::write(dir.join(file), text).unwrap();
        }
    }

    #[test]
    fn normalizer_replaces_listed_members_and_native_base() {
        let normalizer = Normalizer::none().with_key("id").with_base("C:/work");
        let value = json!([{"id": 1, "p": "C:\\work\\x"}, "C:/work"]);
        assert_eq!(
            normalizer.normalize(&value),
            json!([{"id": VOLATILE, "p": "<base>\\x"}, "<base>"])
        );
        assert_eq!(
            Normalizer::none().with_base("").normalize(&json!("a")),
            json!("a")
        );
    }

    #[test]
    fn normalizer_replaces_the_base_only_at_the_end_of_a_component() {
        let normalizer = Normalizer::none().with_base("/tmp/run-1");
        let value = json!([
            "/tmp/run-1/a",
            "/tmp/run-10/a",
            "/tmp/run-1",
            "x /tmp/run-1: y"
        ]);
        assert_eq!(
            normalizer.normalize(&value),
            json!(["<base>/a", "/tmp/run-10/a", "<base>", "x <base>: y"])
        );
    }

    #[test]
    fn mismatch_lists_the_file_and_the_pointer() {
        let root = tempfile::tempdir().unwrap();
        let before = FsSnapshot::from_entries(vec![SnapshotEntry::file("a.txt", "x")]).to_json();
        write_case(
            root.path(),
            "remove",
            &[
                ("case.toml", "base = \"/w\"\nkind = \"report\"\n"),
                ("fs_before.json", &before),
                (
                    "report.json",
                    "{\"status\": \"ok\", \"path\": \"<base>/a.txt\"}",
                ),
            ],
        );
        write_case(root.path(), "skipped", &[("notes.txt", "no case file")]);
        let runner = CaseRunner::new(root.path()).bless(false);
        let cases = runner.cases().unwrap();
        assert_eq!(cases.len(), 1);
        assert_eq!(cases[0].base.as_deref(), Some("/w"));
        assert!(cases[0].config.contains_key("kind"));
        assert_eq!(cases[0].fs_before.as_ref().unwrap().entries.len(), 1);
        let error = runner
            .run(|_| {
                CaseOutput::new()
                    .with(
                        "report.json",
                        json!({"status": "failed", "path": "/w/a.txt"}),
                    )
                    .with("bound.json", json!({}))
            })
            .unwrap_err();
        let GoldenError::Mismatch { failures } = &error else {
            panic!("expected a mismatch, got {error}");
        };
        assert_eq!(
            failures[0].differences,
            [
                "bound.json: expected file is missing",
                "report.json: /status: expected \"ok\", actual \"failed\"",
            ]
        );
    }

    #[test]
    fn variants_are_compared_with_one_report_and_blessed_once() {
        let root = tempfile::tempdir().unwrap();
        write_case(
            root.path(),
            "count",
            &[
                ("case.toml", ""),
                ("a.nurt", "find |> count"),
                ("b.nurt", "find\n|> count"),
            ],
        );
        write_case(root.path(), "empty", &[("case.toml", "")]);
        let runner = CaseRunner::new(root.path()).bless(false);
        let error = runner.run_sources(|_, _| "{}".to_owned()).unwrap_err();
        let GoldenError::Mismatch { failures } = &error else {
            panic!("expected a mismatch, got {error}");
        };
        assert_eq!(failures.len(), 2);
        assert_eq!(
            failures[0].differences,
            ["report.json: expected file is missing"]
        );
        assert_eq!(failures[1].differences, ["case has no program file"]);

        fs::remove_dir_all(root.path().join("empty")).unwrap();
        let blessed = CaseRunner::new(root.path())
            .bless(true)
            .run_sources(|_, source| format!("{{\"lines\": {}}}", source.lines().count()))
            .unwrap();
        assert_eq!(blessed.blessed, ["count"]);
        let error = runner
            .run_sources(|_, source| format!("{{\"lines\": {}}}", source.lines().count()))
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("b.nurt: /lines: expected 1, actual 2")
        );
        let passed = runner
            .run_sources(|_, _| "{\"lines\": 1}".to_owned())
            .unwrap();
        assert_eq!(passed.passed, ["count"]);
    }

    #[test]
    fn blessing_writes_every_produced_file() {
        let root = tempfile::tempdir().unwrap();
        write_case(
            root.path(),
            "one",
            &[("case.toml", "description = \"d\"\n")],
        );
        let after = FsSnapshot::from_entries(vec![SnapshotEntry::dir("out")]);
        let report = CaseRunner::new(root.path())
            .bless(true)
            .run(|case| {
                assert_eq!(case.description.as_deref(), Some("d"));
                CaseOutput::new().with_fs_after(&after)
            })
            .unwrap();
        assert_eq!(report.blessed, ["one"]);
        let written = fs::read_to_string(root.path().join("one/fs_after.json")).unwrap();
        assert_eq!(FsSnapshot::from_json(&written).unwrap(), after);
        let empty = tempfile::tempdir().unwrap();
        assert!(matches!(
            CaseRunner::new(empty.path()).cases(),
            Err(GoldenError::NoCases { .. })
        ));
    }
}
