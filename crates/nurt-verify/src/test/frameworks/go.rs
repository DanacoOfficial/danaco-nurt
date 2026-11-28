// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-verify::test::frameworks::go
// Responsibility:   The command plan of a test run of go and the parsing of the event stream of go test -json.
// Dependencies:     nurt_contracts, regex, serde
// Author:           Danaco Official
// Version:          0.1.2, 2026-11-28
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Go tests.

use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use nurt_contracts::record::{TestResult, TestStatus};
use regex::Regex;
use serde::Deserialize;

use super::{ID_SEPARATOR, PACKAGE_MARKER, make_id, non_blank, seconds, unique_ids};
use crate::test::command::{escape_pattern, path_text};
use crate::test::{CommandPlan, Framework, ParseError, ParseInput, PlanRequest};

/// Package pattern used when the run names no target.
const ALL_PACKAGES: &str = "./...";

/// Lines of the test output that report the state of the run and not the
/// reason of a failure.
static STATE_LINE: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(r"^(?:=== (?:RUN|PAUSE|CONT|NAME)\b.*|--- (?:FAIL|PASS|SKIP): .*|FAIL|PASS)$").ok()
});

/// Command of a run: the JSON event stream, name pattern, fail-fast and a
/// coverage profile.
pub(crate) fn plan(request: &PlanRequest<'_>) -> CommandPlan {
    let packages: Vec<String> = if !request.only.is_empty() {
        let mut packages: Vec<String> = Vec::new();
        for case in request.only {
            // A name that starts with `-` would be read as an option of
            // `go test`; it counts as a missing package.
            let package = case
                .suite
                .clone()
                .filter(|package| !package.starts_with('-'))
                .unwrap_or_default();
            if !packages.contains(&package) {
                packages.push(package);
            }
        }
        if packages.iter().any(String::is_empty) {
            vec![ALL_PACKAGES.to_owned()]
        } else {
            packages
        }
    } else if request.targets.is_empty() {
        vec![ALL_PACKAGES.to_owned()]
    } else {
        request.targets.to_vec()
    };
    let mut names: Vec<String> = Vec::new();
    for case in request
        .only
        .iter()
        .filter(|case| case.name != PACKAGE_MARKER)
    {
        let top = case.name.split('/').next().unwrap_or(&case.name);
        let escaped = escape_pattern(top);
        if !names.contains(&escaped) {
            names.push(escaped);
        }
    }
    let mut argv = vec!["go".to_owned(), "test".to_owned(), "-json".to_owned()];
    if !names.is_empty() {
        argv.push("-run".to_owned());
        argv.push(format!("^({})$", names.join("|")));
    } else if request.only.is_empty()
        && let Some(filter) = request.filter
    {
        argv.push("-run".to_owned());
        argv.push(filter.to_owned());
    }
    if request.bail {
        argv.push("-failfast".to_owned());
    }
    let coverage_file = request.coverage.map(path_text);
    if let Some(file) = &coverage_file {
        argv.push(format!("-coverprofile={file}"));
    }
    argv.extend(packages);
    CommandPlan {
        framework: Framework::Go,
        argv,
        report: None,
        coverage_file,
        env: Vec::new(),
        notes: Vec::new(),
    }
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "PascalCase")]
struct Event {
    #[serde(default)]
    action: String,
    #[serde(default)]
    package: String,
    #[serde(default)]
    import_path: String,
    #[serde(default)]
    test: String,
    #[serde(default)]
    elapsed: Option<f64>,
    #[serde(default)]
    output: Option<String>,
    #[serde(default)]
    failed_build: Option<String>,
}

#[derive(Default)]
struct State {
    outputs: HashMap<String, String>,
    results: Vec<TestResult>,
    index: HashMap<String, usize>,
    started: Vec<String>,
}

fn key(package: &str, test: &str) -> String {
    format!("{package}{ID_SEPARATOR}{test}")
}

/// Reads the event stream of `go test -json` from the output. Lines that are
/// not events are ignored.
pub(crate) fn parse(input: &ParseInput<'_>) -> Result<Vec<TestResult>, ParseError> {
    let mut state = State::default();
    for line in input.output.lines() {
        if !line.starts_with('{') {
            continue;
        }
        let Ok(event) = serde_json::from_str::<Event>(line) else {
            continue;
        };
        collect_output(&mut state, &event);
        collect_result(&mut state, &event);
    }
    finish_unfinished(&mut state);
    let mut results = drop_covered(state.results);
    unique_ids(&mut results);
    Ok(results)
}

fn collect_output(state: &mut State, event: &Event) {
    let Some(text) = &event.output else { return };
    let target = match event.action.as_str() {
        "build-output" => format!("build{ID_SEPARATOR}{}", event.import_path),
        "output" => key(&event.package, &event.test),
        _ => return,
    };
    state.outputs.entry(target).or_default().push_str(text);
}

fn collect_result(state: &mut State, event: &Event) {
    if event.action == "run" {
        state.started.push(key(&event.package, &event.test));
        return;
    }
    let status = match event.action.as_str() {
        "pass" => TestStatus::Passed,
        "fail" if event.test.is_empty() => TestStatus::Error,
        "fail" => TestStatus::Failed,
        "skip" => TestStatus::Skipped,
        _ => return,
    };
    if event.test.is_empty() && status != TestStatus::Error {
        return;
    }
    let name = if event.test.is_empty() {
        PACKAGE_MARKER.to_owned()
    } else {
        event.test.clone()
    };
    let message = if status == TestStatus::Passed || status == TestStatus::Skipped {
        None
    } else {
        failure_text(state, event)
    };
    let result = TestResult {
        id: make_id(&[&event.package, &name]),
        name,
        status,
        suite: non_blank(&event.package),
        duration: event.elapsed.and_then(seconds),
        message,
        ..TestResult::default()
    };
    let slot = key(&event.package, &event.test);
    match state.index.get(&slot) {
        Some(position) => state.results[*position] = result,
        None => {
            state.index.insert(slot, state.results.len());
            state.results.push(result);
        }
    }
}

fn failure_text(state: &State, event: &Event) -> Option<String> {
    let mut text = String::new();
    if let Some(import) = &event.failed_build
        && let Some(build) = state.outputs.get(&format!("build{ID_SEPARATOR}{import}"))
    {
        text.push_str(build);
    }
    if let Some(output) = state.outputs.get(&key(&event.package, &event.test)) {
        text.push_str(output);
    }
    let noise = STATE_LINE.as_ref();
    let kept: Vec<&str> = text
        .lines()
        .filter(|line| !noise.is_some_and(|noise| noise.is_match(line.trim())))
        .collect();
    non_blank(&kept.join("\n"))
}

/// Tests that started and never reported a result were cut off.
fn finish_unfinished(state: &mut State) {
    let finished: HashSet<&String> = state.index.keys().collect();
    let mut cut_off: Vec<TestResult> = Vec::new();
    for slot in &state.started {
        if finished.contains(slot) {
            continue;
        }
        let Some((package, test)) = slot.split_once(ID_SEPARATOR) else {
            continue;
        };
        cut_off.push(TestResult {
            id: make_id(&[package, test]),
            name: test.to_owned(),
            status: TestStatus::Error,
            suite: non_blank(package),
            message: Some("the test did not report a result".to_owned()),
            ..TestResult::default()
        });
    }
    state.results.extend(cut_off);
}

/// Drops the results that a more specific result already covers: parents of
/// subtests, and package results of packages with a failing test.
fn drop_covered(results: Vec<TestResult>) -> Vec<TestResult> {
    let mut parents: HashSet<String> = HashSet::new();
    for result in &results {
        let package = result.suite.clone().unwrap_or_default();
        let mut name = result.name.as_str();
        while let Some(cut) = name.rfind('/') {
            name = &name[..cut];
            parents.insert(key(&package, name));
        }
    }
    let failing: HashSet<String> = results
        .iter()
        .filter(|result| result.status == TestStatus::Failed && result.name != PACKAGE_MARKER)
        .filter_map(|result| result.suite.clone())
        .collect();
    results
        .into_iter()
        .filter(|result| {
            let package = result.suite.clone().unwrap_or_default();
            if result.name == PACKAGE_MARKER {
                !failing.contains(&package)
            } else {
                !parents.contains(&key(&package, &result.name))
            }
        })
        .collect()
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
