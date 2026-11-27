// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-code/tests/u14_rename_syntax.rs
// Responsibility:   `code.rename` without a language server: whole identifiers in code are.
// Dependencies:     nurt_contracts, u14_support
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! `code.rename` without a language server: whole identifiers in code are
//! replaced according to the syntax analysis of the built-in grammars, for
//! Python, TypeScript, Go, Rust and Java.

// Tests report failures by panicking.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod u14_support;

use std::path::Path;
use std::sync::{Arc, Mutex};

use nurt_code::commands::rename::CodeRename;
use nurt_code::service::CodeAnalysis;
use nurt_contracts::config::Config;
use nurt_contracts::report::Notable;
use nurt_contracts::services::Services;
use nurt_contracts::stage::{ExecCtx, StageInput};
use nurt_contracts::value::{RecordKind, Value};
use u14_support::{JournalLog, StdFs, TestClock, TestHost, args, execute, path_value, preview};

/// Project files of one language and the expected content after the rename.
struct Case {
    /// File holding the definition, with the line and column of its name.
    at: (&'static str, u32, u32),
    to: &'static str,
    /// Path, content before, content after.
    files: &'static [(&'static str, &'static str, &'static str)],
}

fn context() -> ExecCtx {
    ExecCtx::builder(Arc::new(TestClock), Arc::new(TestHost::new()))
        .fs(Arc::new(StdFs))
        .services(Arc::new(
            Services::new().with_code(Arc::new(CodeAnalysis::builtin())),
        ))
        .config(Arc::new(Config::default()))
        .build()
}

fn write_case(root: &Path, case: &Case) {
    for (path, before, _) in case.files {
        u14_support::write(&root.join(path), before);
    }
}

fn rename_args(
    root: &Path,
    case: &Case,
    extra: &[(&str, Value)],
) -> nurt_contracts::stage::BoundArgs {
    let (file, line, column) = case.at;
    let mut pairs = vec![
        ("to", Value::Text(case.to.to_owned())),
        ("path", path_value(&root.join(file))),
        ("line", Value::Int(i64::from(line))),
        ("column", Value::Int(i64::from(column))),
        ("source", Value::Enum("syntax".to_owned())),
    ];
    pairs.extend(extra.iter().cloned());
    args(&pairs)
}

/// Renames by syntax analysis, checks every file against the expected
/// content and restores the files from the journal double.
fn check(case: &Case) {
    let (_dir, root) = u14_support::project();
    write_case(&root, case);
    let ctx = context();
    let stage = CodeRename::new(None);
    let call = rename_args(&root, case, &[]);

    let plan = preview(&stage, &call, &ctx).unwrap();
    let summary = plan
        .preview
        .iter()
        .find(|record| record.kind == RecordKind::Map)
        .unwrap();
    assert_eq!(summary.fields["source"], Value::Enum("syntax".to_owned()));
    assert_eq!(
        summary.fields["confidence"],
        Value::Enum("identifier".to_owned())
    );

    let log = Arc::new(Mutex::new(JournalLog::default()));
    let outcome = execute(&stage, &call, StageInput::none(), &ctx, &log).unwrap();
    for (path, _, after) in case.files {
        assert_eq!(&u14_support::read(&root.join(path)), after, "{path}");
    }
    assert!(!outcome.notable.contains(&Notable::RenameIncomplete));

    log.lock().unwrap().restore();
    for (path, before, _) in case.files {
        assert_eq!(&u14_support::read(&root.join(path)), before, "{path}");
    }
}

#[test]
fn renames_python_identifiers_in_code_and_keeps_comments_and_strings() {
    check(&Case {
        at: ("lib.py", 1, 5),
        to: "sum_values",
        files: &[
            (
                "lib.py",
                "def compute_total(values):\n    # compute_total adds the values\n    return sum(values)\n",
                "def sum_values(values):\n    # compute_total adds the values\n    return sum(values)\n",
            ),
            (
                "main.py",
                "from lib import compute_total\n\nlabel = \"compute_total\"\nprint(compute_total([1, 2]), label)\n",
                "from lib import sum_values\n\nlabel = \"compute_total\"\nprint(sum_values([1, 2]), label)\n",
            ),
        ],
    });
}

#[test]
fn renames_typescript_identifiers_across_imports() {
    check(&Case {
        at: ("lib.ts", 1, 17),
        to: "sumValues",
        files: &[
            (
                "lib.ts",
                "export function computeTotal(values: number[]): number {\n  // computeTotal adds the values\n  return values.reduce((a, b) => a + b, 0);\n}\n",
                "export function sumValues(values: number[]): number {\n  // computeTotal adds the values\n  return values.reduce((a, b) => a + b, 0);\n}\n",
            ),
            (
                "main.ts",
                "import { computeTotal } from \"./lib\";\n\nconst label = \"computeTotal\";\nconsole.log(computeTotal([1, 2]), label);\n",
                "import { sumValues } from \"./lib\";\n\nconst label = \"computeTotal\";\nconsole.log(sumValues([1, 2]), label);\n",
            ),
        ],
    });
}

#[test]
fn renames_go_identifiers_in_every_file_of_the_package() {
    check(&Case {
        at: ("lib.go", 4, 6),
        to: "sumValues",
        files: &[
            (
                "lib.go",
                "package demo\n\n// computeTotal adds the values.\nfunc computeTotal(values []int) int {\n\ttotal := 0\n\tfor _, value := range values {\n\t\ttotal += value\n\t}\n\treturn total\n}\n",
                "package demo\n\n// computeTotal adds the values.\nfunc sumValues(values []int) int {\n\ttotal := 0\n\tfor _, value := range values {\n\t\ttotal += value\n\t}\n\treturn total\n}\n",
            ),
            (
                "main.go",
                "package demo\n\nconst label = \"computeTotal\"\n\nfunc run() int {\n\treturn computeTotal([]int{1, 2})\n}\n",
                "package demo\n\nconst label = \"computeTotal\"\n\nfunc run() int {\n\treturn sumValues([]int{1, 2})\n}\n",
            ),
        ],
    });
}

#[test]
fn renames_rust_identifiers_in_use_declarations_and_calls() {
    check(&Case {
        at: ("src/lib.rs", 4, 8),
        to: "sum_values",
        files: &[
            (
                "src/lib.rs",
                "mod other;\n\n/// compute_total adds the values.\npub fn compute_total(values: &[i32]) -> i32 {\n    values.iter().sum()\n}\n",
                "mod other;\n\n/// compute_total adds the values.\npub fn sum_values(values: &[i32]) -> i32 {\n    values.iter().sum()\n}\n",
            ),
            (
                "src/other.rs",
                "use crate::compute_total;\n\npub fn three() -> usize {\n    let label = \"compute_total\";\n    compute_total(&[3]) as usize + label.len()\n}\n",
                "use crate::sum_values;\n\npub fn three() -> usize {\n    let label = \"compute_total\";\n    sum_values(&[3]) as usize + label.len()\n}\n",
            ),
        ],
    });
}

#[test]
fn renames_java_identifiers_in_qualified_calls() {
    check(&Case {
        at: ("Lib.java", 3, 16),
        to: "sumValues",
        files: &[
            (
                "Lib.java",
                "public class Lib {\n    // computeTotal adds the values\n    static int computeTotal(int[] values) {\n        int total = 0;\n        for (int value : values) {\n            total += value;\n        }\n        return total;\n    }\n}\n",
                "public class Lib {\n    // computeTotal adds the values\n    static int sumValues(int[] values) {\n        int total = 0;\n        for (int value : values) {\n            total += value;\n        }\n        return total;\n    }\n}\n",
            ),
            (
                "Main.java",
                "public class Main {\n    public static void main(String[] args) {\n        System.out.println(\"computeTotal\");\n        System.out.println(Lib.computeTotal(new int[] {1, 2}));\n    }\n}\n",
                "public class Main {\n    public static void main(String[] args) {\n        System.out.println(\"computeTotal\");\n        System.out.println(Lib.sumValues(new int[] {1, 2}));\n    }\n}\n",
            ),
        ],
    });
}

#[test]
fn renames_comments_and_strings_only_when_included() {
    let case = Case {
        at: ("lib.py", 1, 5),
        to: "sum_values",
        files: &[(
            "lib.py",
            "def compute_total(values):\n    # compute_total adds the values\n    return \"compute_total\"\n",
            "def sum_values(values):\n    # sum_values adds the values\n    return \"sum_values\"\n",
        )],
    };
    let (_dir, root) = u14_support::project();
    write_case(&root, &case);
    let ctx = context();
    let stage = CodeRename::new(None);
    let call = rename_args(
        &root,
        &case,
        &[
            ("include_comments", Value::Bool(true)),
            ("include_strings", Value::Bool(true)),
        ],
    );
    let log = Arc::new(Mutex::new(JournalLog::default()));
    let outcome = execute(&stage, &call, StageInput::none(), &ctx, &log).unwrap();
    assert_eq!(u14_support::read(&root.join("lib.py")), case.files[0].2);
    assert!(!outcome.notable.contains(&Notable::ReferencesLeft));
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
