// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Reader of JUnit XML reports.

use std::path::Path;

use nurt_contracts::record::{TestResult, TestStatus};

use super::{GROUP_SEPARATOR, ID_SEPARATOR, make_id, non_blank, path_of, seconds, unique_ids};
use crate::test::ParseError;
use crate::test::xml::{self, Element};

/// Attribute value of a skipped test that marks it as not yet written.
const TODO_MARKER: &str = "todo";

/// Way a framework writes file and group of a test case into the report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Flavor {
    /// Group from the nesting of suites, file from the suite.
    Bun,
    /// File in the class name, groups in the test name separated by `>`.
    Vitest,
    /// Module in the class name, found relative to the project directory.
    Pytest,
    /// File and group as the attributes say.
    Generic,
}

struct Place {
    file: String,
    group: String,
    name: String,
}

struct Suite<'a> {
    name: &'a str,
    file: &'a str,
}

/// Reads the test cases of a report.
///
/// `dir` is the project directory, used by the pytest flavor to find the
/// module of a class name.
pub(crate) fn parse(
    text: &str,
    flavor: Flavor,
    dir: Option<&Path>,
) -> Result<Vec<TestResult>, ParseError> {
    let parsed = xml::parse(text);
    let mut results = Vec::new();
    let mut suites: Vec<Suite<'_>> = Vec::new();
    let mut found_suite = false;
    walk(
        &parsed.root,
        flavor,
        dir,
        &mut suites,
        &mut results,
        &mut found_suite,
    );
    unique_ids(&mut results);
    if let Some(reason) = parsed.error {
        return Err(ParseError {
            reason,
            partial: results,
        });
    }
    if !found_suite {
        return Err(ParseError {
            reason: "the report has no test suites".to_owned(),
            partial: results,
        });
    }
    Ok(results)
}

fn walk<'a>(
    element: &'a Element,
    flavor: Flavor,
    dir: Option<&Path>,
    suites: &mut Vec<Suite<'a>>,
    results: &mut Vec<TestResult>,
    found_suite: &mut bool,
) {
    for child in &element.children {
        match child.name.as_str() {
            "testsuites" => {
                *found_suite = true;
                walk(child, flavor, dir, suites, results, found_suite);
            }
            "testsuite" => {
                *found_suite = true;
                let file = child
                    .attribute("file")
                    .or_else(|| suites.last().map(|suite| suite.file))
                    .unwrap_or_default();
                suites.push(Suite {
                    name: child.attribute("name").unwrap_or_default(),
                    file,
                });
                walk(child, flavor, dir, suites, results, found_suite);
                suites.pop();
            }
            "testcase" => results.push(case(child, flavor, dir, suites)),
            _ => {}
        }
    }
}

fn case(element: &Element, flavor: Flavor, dir: Option<&Path>, suites: &[Suite<'_>]) -> TestResult {
    let suite_file = suites.last().map_or("", |suite| suite.file);
    let place = locate(
        flavor,
        dir,
        suites,
        Place {
            file: element.attribute("file").unwrap_or(suite_file).to_owned(),
            group: element.attribute("classname").unwrap_or_default().to_owned(),
            name: element.attribute("name").unwrap_or_default().to_owned(),
        },
    );
    let mut result = TestResult {
        id: make_id(&[&place.file, &place.group, &place.name]),
        name: place.name,
        status: TestStatus::Passed,
        suite: non_blank(&place.group),
        path: path_of(&place.file),
        duration: element
            .attribute("time")
            .and_then(|time| time.parse::<f64>().ok())
            .and_then(seconds),
        line: element.attribute("line").and_then(|line| line.parse().ok()),
        ..TestResult::default()
    };
    for child in &element.children {
        match child.name.as_str() {
            "skipped" => skipped(&mut result, child),
            "failure" => failed(&mut result, child, TestStatus::Failed),
            "error" => failed(&mut result, child, TestStatus::Error),
            "system-out" if result.stdout.is_none() => result.stdout = non_blank(&child.text),
            "system-err" if result.stderr.is_none() => result.stderr = non_blank(&child.text),
            _ => {}
        }
    }
    if matches!(result.status, TestStatus::Passed | TestStatus::Skipped | TestStatus::Todo) {
        result.stdout = None;
        result.stderr = None;
    }
    result
}

fn skipped(result: &mut TestResult, element: &Element) {
    if result.status != TestStatus::Passed {
        return;
    }
    let todo = element
        .attribute("message")
        .is_some_and(|message| message.trim().eq_ignore_ascii_case(TODO_MARKER));
    result.status = if todo {
        TestStatus::Todo
    } else {
        TestStatus::Skipped
    };
}

fn failed(result: &mut TestResult, element: &Element, status: TestStatus) {
    if result.status == TestStatus::Failed || result.status == TestStatus::Error {
        return;
    }
    result.status = status;
    let body = element.text.trim();
    let attribute = element
        .attribute("message")
        .map(str::trim)
        .filter(|message| !message.is_empty())
        .or_else(|| element.attribute("type").map(str::trim));
    match attribute {
        Some(message) if !message.is_empty() => {
            result.message = Some(message.to_owned());
            if !body.is_empty() && body != message {
                result.stack = Some(body.to_owned());
            }
        }
        _ => result.message = non_blank(body),
    }
}

fn locate(flavor: Flavor, dir: Option<&Path>, suites: &[Suite<'_>], place: Place) -> Place {
    match flavor {
        Flavor::Bun => bun_place(place, suites),
        Flavor::Vitest => vitest_place(place),
        Flavor::Pytest if place.file.is_empty() && !place.group.is_empty() => match dir {
            Some(dir) => pytest_place(dir, place),
            None => place,
        },
        _ => place,
    }
}

/// Group of a bun case: the names of the nested suites without the suite
/// of the file.
fn bun_place(place: Place, suites: &[Suite<'_>]) -> Place {
    let groups: Vec<&str> = suites
        .iter()
        .enumerate()
        .filter(|(index, suite)| *index > 0 || suite.name != suite.file)
        .map(|(_, suite)| suite.name)
        .collect();
    if groups.is_empty() {
        place
    } else {
        Place {
            group: groups.join(GROUP_SEPARATOR),
            ..place
        }
    }
}

/// A vitest class name is the file and the test name carries the groups.
fn vitest_place(place: Place) -> Place {
    let parts: Vec<&str> = place.name.split(GROUP_SEPARATOR).collect();
    let (name, groups) = parts.split_last().map_or((place.name.as_str(), &[][..]), |(last, rest)| (*last, rest));
    let file = if place.file.is_empty() {
        place.group.clone()
    } else {
        place.file.clone()
    };
    Place {
        file,
        group: groups.join(GROUP_SEPARATOR),
        name: name.to_owned(),
    }
}

/// Module of a pytest class name: the longest prefix of the dotted name
/// that is a Python file under the project directory; the rest are classes.
fn pytest_place(dir: &Path, place: Place) -> Place {
    let parts: Vec<&str> = place.group.split('.').collect();
    for length in (1..=parts.len()).rev() {
        let file = format!("{}.py", parts[..length].join("/"));
        if dir.join(&file).is_file() {
            return Place {
                file,
                group: parts[length..].join(ID_SEPARATOR),
                name: place.name,
            };
        }
    }
    place
}
