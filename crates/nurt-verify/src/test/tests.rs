// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Tests of detection, commands and output parsers.

use std::path::Path;

use nurt_contracts::record::{TestResult, TestStatus};
use nurt_contracts::value::Path as PathValue;

use super::command::{escape_pattern, plan};
use super::detect::detect;
use super::frameworks::junit::{self, Flavor};
use super::frameworks::parse;
use super::{Framework, ParseInput, PlanRequest};
use crate::stage::test::support::{corpus, write};

fn statuses(results: &[TestResult]) -> (usize, usize, usize, usize, usize) {
    let count = |status: TestStatus| results.iter().filter(|r| r.status == status).count();
    (
        count(TestStatus::Passed),
        count(TestStatus::Failed),
        count(TestStatus::Skipped),
        count(TestStatus::Error),
        count(TestStatus::Todo),
    )
}

fn read(framework: Framework, output: &str, report: Option<&str>, dir: &Path) -> Vec<TestResult> {
    let input = ParseInput {
        output,
        report,
        dir,
    };
    parse(framework, &input).unwrap_or_default()
}

fn failed_case(path: &str, suite: Option<&str>, name: &str) -> TestResult {
    TestResult {
        id: format!("{path}::{name}"),
        name: name.to_owned(),
        status: TestStatus::Failed,
        suite: suite.map(str::to_owned),
        path: (!path.is_empty()).then(|| PathValue::from(path)),
        ..TestResult::default()
    }
}

struct Request {
    framework: Framework,
    targets: Vec<String>,
    filter: Option<String>,
    bail: bool,
    only: Vec<TestResult>,
    coverage: Option<String>,
    command: Option<Vec<String>>,
}

impl Request {
    fn new(framework: Framework) -> Self {
        Self {
            framework,
            targets: Vec::new(),
            filter: None,
            bail: false,
            only: Vec::new(),
            coverage: None,
            command: None,
        }
    }

    fn plan(&self, report: &str) -> super::CommandPlan {
        let none = |_: &str| None;
        let report = Path::new(report);
        let coverage = self.coverage.as_ref().map(Path::new);
        plan(&PlanRequest {
            framework: self.framework,
            targets: &self.targets,
            filter: self.filter.as_deref(),
            bail: self.bail,
            only: &self.only,
            report,
            coverage,
            command: self.command.as_deref(),
            program: &none,
        })
    }
}

fn words(argv: &[&str]) -> Vec<String> {
    argv.iter().map(|word| (*word).to_owned()).collect()
}

#[test]
fn detect_orders_frameworks_by_precedence() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "bun.lock", "");
    write(
        dir.path(),
        "package.json",
        r#"{"scripts":{"test":"vitest"},"devDependencies":{"vitest":"1","jest":"29"}}"#,
    );
    write(dir.path(), "tests/test_a.py", "");
    write(dir.path(), "go.mod", "module x\n");
    write(dir.path(), "Cargo.toml", "[package]\n");
    assert_eq!(
        detect(dir.path()),
        [
            Framework::Bun,
            Framework::Vitest,
            Framework::Jest,
            Framework::Pytest,
            Framework::Go,
            Framework::Cargo
        ]
    );
}

#[test]
fn detect_uses_package_script_only_as_last_node_option() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "package.json",
        r#"{"scripts":{"test":"node --test"}}"#,
    );
    write(dir.path(), "pyproject.toml", "[tool.pytest.ini_options]\n");
    assert_eq!(detect(dir.path()), [Framework::Npm, Framework::Pytest]);
    write(
        dir.path(),
        "package.json",
        r#"{"scripts":{"test":"echo \"Error: no test specified\" && exit 1"}}"#,
    );
    assert_eq!(detect(dir.path()), [Framework::Pytest]);
    write(dir.path(), "package.json", "{ not json");
    assert_eq!(detect(dir.path()), [Framework::Pytest]);
}

#[test]
fn detect_falls_back_to_bun_for_script_test_files() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "src/sum.spec.ts", "");
    assert_eq!(detect(dir.path()), [Framework::Bun]);
}

#[test]
fn detect_finds_nothing_in_an_empty_project() {
    let dir = tempfile::tempdir().unwrap();
    assert!(detect(dir.path()).is_empty());
}

#[test]
fn command_for_bun_selects_report_filter_bail_and_coverage() {
    let mut request = Request::new(Framework::Bun);
    request.targets = words(&["a.test.ts"]);
    request.filter = Some("name".to_owned());
    request.bail = true;
    request.coverage = Some("/s/cov".to_owned());
    let planned = request.plan("/s/run-1.xml");
    assert_eq!(
        planned.argv,
        words(&[
            "bun",
            "test",
            "a.test.ts",
            "--reporter=junit",
            "--reporter-outfile=/s/run-1.xml",
            "-t",
            "name",
            "--bail",
            "--coverage",
            "--coverage-reporter=lcov",
            "--coverage-dir=/s/cov",
        ])
    );
    // The file is joined to the coverage directory with the separator of
    // the system.
    assert_eq!(
        planned
            .coverage_file
            .as_deref()
            .map(|path| path.replace('\\', "/")),
        Some("/s/cov/lcov.info".to_owned())
    );
}

#[test]
fn command_for_bun_reruns_failed_names_in_their_files() {
    let mut request = Request::new(Framework::Bun);
    request.only = vec![failed_case("a.test.ts", Some("g"), "bad (1)")];
    assert_eq!(
        request.plan("/s/run-1.xml").argv,
        words(&[
            "bun",
            "test",
            "a.test.ts",
            "--reporter=junit",
            "--reporter-outfile=/s/run-1.xml",
            "-t",
            "(?:bad \\(1\\))$",
        ])
    );
}

#[test]
fn command_for_vitest_and_jest_write_reports_and_coverage() {
    let mut vitest = Request::new(Framework::Vitest);
    vitest.filter = Some("x".to_owned());
    vitest.bail = true;
    vitest.coverage = Some("/s/c".to_owned());
    assert_eq!(
        vitest.plan("/s/run-1.xml").argv,
        words(&[
            "vitest",
            "run",
            "--reporter=junit",
            "--outputFile=/s/run-1.xml",
            "-t",
            "x",
            "--bail=1",
            "--coverage",
            "--coverage.reporter=lcov",
            "--coverage.reportsDirectory=/s/c",
        ])
    );
    let mut jest = Request::new(Framework::Jest);
    jest.filter = Some("x".to_owned());
    jest.bail = true;
    jest.coverage = Some("/s/c".to_owned());
    assert_eq!(
        jest.plan("/s/run-1.json").argv,
        words(&[
            "jest",
            "--json",
            "--outputFile=/s/run-1.json",
            "-t",
            "x",
            "--bail",
            "--coverage",
            "--coverageReporters=lcov",
            "--coverageDirectory=/s/c",
        ])
    );
}

#[test]
fn command_for_jest_skips_name_pattern_for_file_failures() {
    let mut request = Request::new(Framework::Jest);
    request.only = vec![failed_case("b.test.js", None, "(file)")];
    let argv = request.plan("/s/run-1.json").argv;
    assert!(argv.contains(&"b.test.js".to_owned()));
    assert!(!argv.contains(&"-t".to_owned()));
}

#[test]
fn command_for_jest_reruns_files_by_their_exact_paths() {
    let mut request = Request::new(Framework::Jest);
    request.only = vec![failed_case("pages/[id].test.tsx", Some("g"), "bad")];
    let argv = request.plan("/s/run-1.json").argv;
    assert_eq!(
        argv[..3],
        words(&["jest", "pages/[id].test.tsx", "--runTestsByPath"])
    );
}

#[test]
fn command_for_pytest_builds_nodes_and_coverage_environment() {
    let mut request = Request::new(Framework::Pytest);
    request.filter = Some("abc".to_owned());
    request.bail = true;
    request.coverage = Some("/s/run-1.coverage".to_owned());
    let planned = request.plan("/s/run-1.xml");
    assert_eq!(
        planned.argv,
        words(&[
            "python3",
            "-m",
            "pytest",
            "-q",
            "--junitxml=/s/run-1.xml",
            "-o",
            "junit_family=xunit2",
            "-k",
            "abc",
            "-x",
            "--cov=.",
            "--cov-report=lcov:/s/run-1.coverage",
        ])
    );
    assert_eq!(
        planned.env,
        [(
            "COVERAGE_FILE".to_owned(),
            "/s/run-1.coverage.data".to_owned()
        )]
    );
    let mut rerun = Request::new(Framework::Pytest);
    rerun.only = vec![
        failed_case("tests/test_a.py", Some("TestK"), "test_bad"),
        failed_case("test_b.py", None, "test_x"),
    ];
    let argv = rerun.plan("/s/run-1.xml").argv;
    assert_eq!(
        argv[argv.len() - 2..],
        words(&["tests/test_a.py::TestK::test_bad", "test_b.py::test_x"])
    );
    rerun.bail = true;
    rerun.only = vec![failed_case("", None, "--collect-only")];
    let argv = rerun.plan("/s/run-1.xml").argv;
    assert_eq!(argv[argv.len() - 2..], words(&["--", "--collect-only"]));
    assert!(argv.iter().position(|word| word == "-x") < argv.iter().position(|word| word == "--"));
}

#[test]
fn command_for_scripts_gives_files_that_start_with_a_dash_as_relative_paths() {
    let mut rerun = Request::new(Framework::Bun);
    rerun.only = vec![failed_case("-x.test.ts", None, "t")];
    let argv = rerun.plan("/s/run-1.xml").argv;
    assert_eq!(argv[..3], words(&["bun", "test", "./-x.test.ts"]));
}

#[test]
fn command_for_cargo_does_not_pass_names_that_would_read_as_options() {
    let mut rerun = Request::new(Framework::Cargo);
    rerun.only = vec![
        failed_case("", None, "--list"),
        failed_case("", None, "tests::ok"),
    ];
    let planned = rerun.plan("/s/unused");
    assert_eq!(
        planned.argv,
        words(&["cargo", "test", "--", "tests::ok", "--exact"])
    );
    assert!(planned.notes[0].contains("start with `-`"));
    rerun.only = vec![failed_case("", None, "--list")];
    let planned = rerun.plan("/s/unused");
    assert_eq!(planned.argv, words(&["cargo", "test", "--", "", "--exact"]));
}

#[test]
fn command_for_go_does_not_pass_packages_that_would_read_as_options() {
    let mut rerun = Request::new(Framework::Go);
    rerun.only = vec![failed_case("", Some("-exec=evil"), "TestA")];
    assert_eq!(
        rerun.plan("/s/unused").argv,
        words(&["go", "test", "-json", "-run", "^(TestA)$", "./..."])
    );
}

#[test]
fn command_for_go_selects_packages_and_top_level_names() {
    let mut request = Request::new(Framework::Go);
    request.filter = Some("TestA".to_owned());
    request.bail = true;
    request.coverage = Some("/s/c.out".to_owned());
    assert_eq!(
        request.plan("/s/unused").argv,
        words(&[
            "go",
            "test",
            "-json",
            "-run",
            "TestA",
            "-failfast",
            "-coverprofile=/s/c.out",
            "./..."
        ])
    );
    let mut rerun = Request::new(Framework::Go);
    rerun.only = vec![failed_case("", Some("example.com/p"), "TestA/sub")];
    assert_eq!(
        rerun.plan("/s/unused").argv,
        words(&["go", "test", "-json", "-run", "^(TestA)$", "example.com/p"])
    );
}

#[test]
fn command_for_cargo_uses_exact_names_and_llvm_cov_for_coverage() {
    let mut request = Request::new(Framework::Cargo);
    request.filter = Some("name".to_owned());
    request.bail = true;
    let planned = request.plan("/s/unused");
    assert_eq!(planned.argv, words(&["cargo", "test", "name"]));
    assert_eq!(planned.notes.len(), 1);
    let mut rerun = Request::new(Framework::Cargo);
    rerun.only = vec![failed_case("", None, "tests::bad")];
    assert_eq!(
        rerun.plan("/s/unused").argv,
        words(&["cargo", "test", "--", "tests::bad", "--exact"])
    );
    let mut coverage = Request::new(Framework::Cargo);
    coverage.coverage = Some("/s/c".to_owned());
    assert_eq!(
        coverage.plan("/s/unused").argv,
        words(&["cargo", "llvm-cov", "--lcov", "--output-path", "/s/c"])
    );
}

#[test]
fn command_for_cargo_reruns_a_should_panic_test_by_the_name_the_harness_selects() {
    let mut rerun = Request::new(Framework::Cargo);
    rerun.only = vec![
        failed_case("", None, "tests::panics - should panic"),
        failed_case("", None, "src/lib.rs - documented (line 3) - should panic"),
        failed_case("", None, "tests::plain"),
    ];
    assert_eq!(
        rerun.plan("/s/unused").argv,
        words(&[
            "cargo",
            "test",
            "--",
            "tests::panics",
            "src/lib.rs - documented (line 3)",
            "tests::plain",
            "--exact"
        ])
    );
}

#[test]
fn command_for_go_reruns_results_without_a_package_in_every_package() {
    let mut rerun = Request::new(Framework::Go);
    rerun.only = vec![failed_case("", None, "TestA")];
    assert_eq!(
        rerun.plan("/s/unused").argv,
        words(&["go", "test", "-json", "-run", "^(TestA)$", "./..."])
    );
    rerun.only = vec![
        failed_case("", None, "TestA"),
        failed_case("", Some("example.com/p"), "TestB"),
    ];
    assert_eq!(
        rerun.plan("/s/unused").argv,
        words(&["go", "test", "-json", "-run", "^(TestA|TestB)$", "./..."])
    );
}

#[test]
fn command_for_scripts_and_custom_commands_notes_missing_features() {
    let mut npm = Request::new(Framework::Npm);
    npm.targets = words(&["a"]);
    npm.coverage = Some("/s/c".to_owned());
    let planned = npm.plan("/s/unused");
    assert_eq!(planned.argv, words(&["npm", "test", "--silent", "--", "a"]));
    assert!(planned.notes[0].contains("coverage_file"));
    let mut custom = Request::new(Framework::Custom);
    custom.command = Some(words(&["make", "test"]));
    custom.only = vec![failed_case("", None, "t")];
    let planned = custom.plan("/s/unused");
    assert_eq!(planned.argv, words(&["make", "test"]));
    assert!(planned.notes[0].contains("whole scope"));
}

#[test]
fn escape_pattern_quotes_special_characters_only() {
    assert_eq!(escape_pattern("a (1) [x]"), "a \\(1\\) \\[x\\]");
    assert_eq!(escape_pattern("plain name"), "plain name");
}

#[test]
fn parse_bun_reads_nested_suites_failures_and_todo() {
    let results = read(
        Framework::Bun,
        "",
        Some(&corpus("bun/1.4.2/mixed.xml")),
        Path::new("/p"),
    );
    assert_eq!(statuses(&results), (4, 3, 1, 0, 1));
    assert_eq!(results[0].id, "sum.test.ts::sum::adds numbers");
    let failed = &results[1];
    assert_eq!(failed.name, "characters <a> & \"b\"");
    assert!(
        failed
            .message
            .as_deref()
            .unwrap_or_default()
            .contains("Expected: 3")
    );
    assert!(
        failed
            .stack
            .as_deref()
            .unwrap_or_default()
            .contains("sum.test.ts:8:19")
    );
    assert_eq!(failed.line, Some(7));
}

#[test]
fn parse_bun_without_report_reads_printed_lines() {
    let results = read(
        Framework::Bun,
        "(pass) a > b [1.50ms]\n(fail) a > c\n",
        None,
        Path::new("/p"),
    );
    assert_eq!(statuses(&results), (1, 1, 0, 0, 0));
    assert_eq!(results[0].duration.map(|d| d.as_millis()), Some(2));
}

#[test]
fn parse_vitest_splits_groups_from_test_names() {
    let results = read(
        Framework::Vitest,
        "",
        Some(&corpus("vitest/5.0.3/mixed.xml")),
        Path::new("/p"),
    );
    assert_eq!(statuses(&results), (2, 3, 2, 0, 0));
    assert_eq!(results[1].id, "sum.test.js::sum::characters <a> & \"b\"");
    assert_eq!(results[1].suite.as_deref(), Some("sum"));
    assert!(
        results[1]
            .stack
            .as_deref()
            .unwrap_or_default()
            .contains("sum.test.js:6:19")
    );
    assert_eq!(results[6].suite, None);
}

#[test]
fn parse_jest_reads_file_failures_pending_and_todo() {
    let results = read(
        Framework::Jest,
        "",
        Some(&corpus("jest/30.5.2/mixed.json")),
        Path::new("/project"),
    );
    assert_eq!(statuses(&results), (2, 3, 1, 1, 1));
    let file = &results[0];
    assert_eq!(file.id, "broken.test.js::(file)");
    assert_eq!(file.status, TestStatus::Error);
    assert!(
        file.message
            .as_deref()
            .unwrap_or_default()
            .contains("Cannot find module")
    );
    assert!(
        !file
            .stack
            .as_deref()
            .unwrap_or_default()
            .contains("node_modules")
    );
}

#[test]
fn parse_pytest_finds_modules_and_separates_errors_from_failures() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "test_sample.py", "");
    write(dir.path(), "tests/test_sub.py", "");
    let results = read(
        Framework::Pytest,
        "",
        Some(&corpus("pytest/9.1.1/mixed.xml")),
        dir.path(),
    );
    assert_eq!(statuses(&results), (4, 3, 2, 1, 0));
    assert_eq!(results[0].id, "test_sample.py::test_adds");
    assert_eq!(results[6].id, "test_sample.py::TestGroup::test_method");
    assert_eq!(results[9].id, "tests/test_sub.py::test_in_directory");
    assert_eq!(
        results[1]
            .message
            .as_deref()
            .map(|m| m.lines().next().unwrap_or_default()),
        Some("AssertionError: bad <result> & \"quote\"")
    );
}

#[test]
fn parse_pytest_does_not_probe_paths_made_of_empty_class_name_parts() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "a/b.py", "");
    let report = "<testsuites><testsuite name=\"s\"><testcase classname=\"a..b\" name=\"t\"/><testcase classname=\".a.b\" name=\"u\"/><testcase classname=\"a.b\" name=\"v\"/></testsuite></testsuites>";
    let results = read(Framework::Pytest, "", Some(report), dir.path());
    let paths: Vec<Option<&str>> = results
        .iter()
        .map(|result| result.path.as_ref().map(|path| path.as_str()))
        .collect();
    assert_eq!(paths, [None, None, Some("a/b.py")]);
}

#[test]
fn parse_pytest_does_not_probe_an_absolute_path_named_by_a_class() {
    let project = tempfile::tempdir().unwrap();
    let outside = tempfile::Builder::new().prefix("probe").tempdir().unwrap();
    write(outside.path(), "mod.py", "");
    let class = format!("{}/mod", outside.path().display());
    let report = format!(
        "<testsuites><testsuite name=\"s\"><testcase classname=\"{class}\" name=\"t\"/></testsuite></testsuites>"
    );
    let results = read(Framework::Pytest, "", Some(&report), project.path());
    assert_eq!(results[0].path, None);
}

#[test]
fn parse_pytest_keeps_class_name_when_module_is_missing() {
    let dir = tempfile::tempdir().unwrap();
    let results = read(
        Framework::Pytest,
        "",
        Some(&corpus("pytest/9.1.1/mixed.xml")),
        dir.path(),
    );
    assert_eq!(results[0].id, "test_sample::test_adds");
    assert_eq!(results[0].path, None);
}

#[test]
fn parse_go_reports_build_failures_subtests_and_skips() {
    let results = read(
        Framework::Go,
        &corpus("go/1.26.0/mixed.jsonl"),
        None,
        Path::new("/p"),
    );
    assert_eq!(statuses(&results), (2, 2, 1, 1, 0));
    let package = &results[0];
    assert_eq!(package.name, "(package)");
    assert!(
        package
            .message
            .as_deref()
            .unwrap_or_default()
            .contains("cannot use")
    );
    assert!(results.iter().all(|r| r.name != "TestTable"));
    let failure = results.iter().find(|r| r.name == "TestFailure").unwrap();
    assert!(
        failure
            .message
            .as_deref()
            .unwrap_or_default()
            .contains("<input> & \"quote\"")
    );
}

#[test]
fn parse_go_marks_tests_that_never_finished_as_errors() {
    let output = "{\"Action\":\"run\",\"Package\":\"p\",\"Test\":\"TestA\"}\n{\"Action\":\"pass\",\"Package\":\"p\",\"Test\":\"TestA\",\"Elapsed\":0.1}\n{\"Action\":\"run\",\"Package\":\"p\",\"Test\":\"TestSlow\"}\n{\"Action\":\"output\",\"Package\":\"p\",\"Test\":\"TestSlow\",\"Output\":\"=== RUN   TestSl";
    let results = read(Framework::Go, output, None, Path::new("/p"));
    assert_eq!(statuses(&results), (1, 0, 0, 1, 0));
    assert_eq!(results[1].id, "p::TestSlow");
}

#[test]
fn parse_go_without_tests_has_no_results() {
    let output = "{\"Action\":\"output\",\"Package\":\"p\",\"Output\":\"?   \\tp\\t[no test files]\\n\"}\n{\"Action\":\"skip\",\"Package\":\"p\",\"Elapsed\":0}\n";
    assert!(read(Framework::Go, output, None, Path::new("/p")).is_empty());
}

#[test]
fn parse_cargo_reads_statuses_and_failure_blocks() {
    let results = read(
        Framework::Cargo,
        &corpus("cargo/1.98.1/mixed.txt"),
        None,
        Path::new("/p"),
    );
    assert_eq!(statuses(&results), (2, 2, 1, 0, 0));
    assert_eq!(results[3].name, "tests::group::nested");
    assert!(
        results[2]
            .message
            .as_deref()
            .unwrap_or_default()
            .contains("left: 2")
    );
    assert_eq!(results[4].name, "tests::panic - should panic");
}

#[test]
fn parse_cargo_ignores_compiler_errors_and_cut_lines() {
    let output = "error[E0425]: cannot find value `missing` in this scope\nerror: could not compile `sample`\n";
    assert!(read(Framework::Cargo, output, None, Path::new("/p")).is_empty());
    let cut = "running 3 tests\ntest tests::adds ... ok\ntest tests::sl";
    assert_eq!(read(Framework::Cargo, cut, None, Path::new("/p")).len(), 1);
}

#[test]
fn parse_plain_reads_tap_directives_and_repeated_names() {
    let output = "TAP version 13\nok 1 - a\nnot ok 2 - b\nok 3 c # TODO later\nok 4 d # skip no display\nok 5 - a\n";
    let results = read(Framework::Custom, output, None, Path::new("/p"));
    assert_eq!(statuses(&results), (2, 1, 1, 0, 1));
    assert_eq!(results[4].id, "a#2");
}

#[test]
fn parse_plain_counts_a_subtest_with_its_parent_once() {
    let output = "TAP version 13\n    ok 1 - inner a\n    not ok 2 - inner b\nnot ok 1 - suite\nok 2 - other\n";
    let results = read(Framework::Custom, output, None, Path::new("/p"));
    let names: Vec<&str> = results.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names, ["suite", "other"]);
    assert_eq!(statuses(&results), (1, 1, 0, 0, 0));
    let indented = "    ok 1 - a\n    not ok 2 - b\n";
    assert_eq!(
        read(Framework::Custom, indented, None, Path::new("/p")).len(),
        2
    );
}

#[test]
fn parse_junit_reports_a_truncated_document_with_its_partial_results() {
    let text = "<testsuites><testsuite name=\"s\"><testcase name=\"a\" classname=\"c\" time=\"0.1\"/><testcase name=\"b\" cl";
    let failure = junit::parse(text, Flavor::Pytest, None).unwrap_err();
    assert_eq!(failure.partial.len(), 1);
    assert_eq!(failure.partial[0].name, "a");
}

#[test]
fn parse_junit_rejects_a_document_without_suites() {
    assert!(junit::parse("<html></html>", Flavor::Pytest, None).is_err());
}

#[test]
fn parse_junit_distinguishes_failure_from_error_and_numbers_repeats() {
    let text = "<testsuites><testsuite name=\"a.test.ts\"><testcase name=\"bad\" classname=\"g\" time=\"0.25\"><failure type=\"AssertionError\" message=\"short\">long body</failure></testcase><testcase name=\"bad\" classname=\"g\"><error message=\"boom\"/></testcase></testsuite></testsuites>";
    let results = junit::parse(text, Flavor::Pytest, None).unwrap();
    assert_eq!(results[0].status, TestStatus::Failed);
    assert_eq!(results[0].message.as_deref(), Some("short"));
    assert_eq!(results[0].stack.as_deref(), Some("long body"));
    assert_eq!(results[0].duration.map(|d| d.as_millis()), Some(250));
    assert_eq!(results[1].status, TestStatus::Error);
    assert_eq!(results[1].id, "g::bad#2");
}

#[test]
fn parse_jest_rejects_malformed_report() {
    let input = ParseInput {
        output: "",
        report: Some("{\"numTotalTests\":1,\"testResults\":[{\"name\":\"/w"),
        dir: Path::new("/w"),
    };
    let failure = parse(Framework::Jest, &input).unwrap_err();
    assert!(failure.reason.contains("not valid JSON"));
    assert!(failure.partial.is_empty());
}

#[test]
fn parse_jest_without_report_has_no_results() {
    assert!(read(Framework::Jest, "", None, Path::new("/w")).is_empty());
}
