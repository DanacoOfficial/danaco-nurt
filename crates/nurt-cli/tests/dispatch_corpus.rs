// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Corpus of invocations with the expected resolution: a subcommand is chosen
//! only by an exact name in the first argument; every other invocation is a
//! program of the language.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "assertions of a test file"
)]

use nurt_cli::args::{Invocation, SUBCOMMANDS, resolve};

#[derive(Debug, Clone, Copy)]
enum Expect {
    Sub(&'static str),
    Program,
    Version(bool),
    Help,
}

use Expect::{Help, Program, Sub, Version};

/// Invocations as the shell passes them, with the expected resolution.
const CORPUS: &[(&[&str], Expect)] = &[
    // No arguments start the interactive session.
    (&[], Sub("repl")),
    // Every subcommand by its exact name.
    (&["exec"], Sub("exec")),
    (&["script"], Sub("script")),
    (&["repl"], Sub("repl")),
    (&["check"], Sub("check")),
    (&["fmt"], Sub("fmt")),
    (&["ast"], Sub("ast")),
    (&["schema"], Sub("schema")),
    (&["catalog"], Sub("catalog")),
    (&["help"], Sub("help")),
    (&["history"], Sub("history")),
    (&["undo"], Sub("undo")),
    (&["redo"], Sub("redo")),
    (&["serve"], Sub("serve")),
    (&["mcp"], Sub("mcp")),
    (&["lsp"], Sub("lsp")),
    (&["migrate"], Sub("migrate")),
    (&["service"], Sub("service")),
    (&["journal"], Sub("journal")),
    (&["ext"], Sub("ext")),
    (&["doctor"], Sub("doctor")),
    (&["version"], Sub("version")),
    // Subcommands with arguments.
    (&["exec", "find \"/srv\""], Sub("exec")),
    (&["exec", "--preview", "remove \"/tmp/a\""], Sub("exec")),
    (&["exec", "exec"], Sub("exec")),
    (&["exec", "check"], Sub("exec")),
    (&["script", "build.nurt"], Sub("script")),
    (&["script", "build.nurt", "--preview"], Sub("script")),
    (&["check", "-e", "find \"/srv\""], Sub("check")),
    (&["check", "build.nurt"], Sub("check")),
    (&["fmt", "build.nurt", "--variant", "b"], Sub("fmt")),
    (&["ast", "--from", "a", "--to", "json", "find"], Sub("ast")),
    (&["schema", "report"], Sub("schema")),
    (&["catalog", "remove"], Sub("catalog")),
    (&["catalog", "check-distance"], Sub("catalog")),
    (&["help", "remove"], Sub("help")),
    (&["help", "run"], Sub("help")),
    (&["history", "limit: 5"], Sub("history")),
    (&["history", "--json"], Sub("history")),
    (&["undo", "1"], Sub("undo")),
    (&["redo"], Sub("redo")),
    (&["serve", "--stdio"], Sub("serve")),
    (&["serve", "--socket", "/run/nurt.sock"], Sub("serve")),
    (&["mcp", "--stdio"], Sub("mcp")),
    (&["mcp", "--http", "127.0.0.1:8080"], Sub("mcp")),
    (&["lsp"], Sub("lsp")),
    (&["migrate", "deploy.sh"], Sub("migrate")),
    (&["migrate", "deploy.ps1"], Sub("migrate")),
    (&["service", "install"], Sub("service")),
    (&["service", "status"], Sub("service")),
    (&["journal", "salvage", "/"], Sub("journal")),
    (&["ext", "list"], Sub("ext")),
    (&["ext", "install", "/tmp/x.wasm"], Sub("ext")),
    (&["doctor", "--json"], Sub("doctor")),
    (&["version", "--short"], Sub("version")),
    // Words that look like subcommands are programs.
    (&["run", "[\"go\",\"test\"]", "cwd: \"/x\""], Program),
    (&["run", "[\"go\",\"test\"]"], Program),
    (&["run"], Program),
    (&["find", "\"/srv\""], Program),
    (&["find \"/srv\""], Program),
    (&["list", "\"/srv\""], Program),
    (&["remove", "\"/tmp/a\""], Program),
    (&["copy", "\"/a\"", "\"/b\""], Program),
    (&["checks"], Program),
    (&["checkout"], Program),
    (&["check-distance"], Program),
    (&["execute"], Program),
    (&["executable"], Program),
    (&["scripts"], Program),
    (&["scripting"], Program),
    (&["replace"], Program),
    (&["replay"], Program),
    (&["format"], Program),
    (&["fmt2"], Program),
    (&["astral"], Program),
    (&["schemas"], Program),
    (&["catalogue"], Program),
    (&["helper"], Program),
    (&["helpme"], Program),
    (&["histories"], Program),
    (&["history2"], Program),
    (&["undone"], Program),
    (&["redoing"], Program),
    (&["servers"], Program),
    (&["serving"], Program),
    (&["mcp2"], Program),
    (&["lspconfig"], Program),
    (&["migrates"], Program),
    (&["services"], Program),
    (&["journals"], Program),
    (&["extension"], Program),
    (&["ext2"], Program),
    (&["doctors"], Program),
    (&["versions"], Program),
    (&["version2"], Program),
    // Case matters.
    (&["Check"], Program),
    (&["CHECK"], Program),
    (&["Exec"], Program),
    (&["EXEC"], Program),
    (&["Help"], Program),
    (&["HELP"], Program),
    (&["History"], Program),
    (&["Version"], Program),
    (&["Doctor"], Program),
    (&["Serve", "--stdio"], Program),
    (&["Script"], Program),
    (&["REPL"], Program),
    // Quotes and white space make the argument a program.
    (&["\"check\""], Program),
    (&["'check'"], Program),
    (&["`check`"], Program),
    (&[" check"], Program),
    (&["check "], Program),
    (&["check\n"], Program),
    (&["\thelp"], Program),
    (&["help "], Program),
    (&["\"help\""], Program),
    (&["check x"], Program),
    (&["history limit: 5"], Program),
    (&[""], Program),
    (&[" "], Program),
    // Options before a program.
    (&["--preview", "find"], Program),
    (&["--json", "check"], Program),
    (&["--format", "json", "help"], Program),
    (&["--limit", "5", "history"], Program),
    (&["--variant", "b", "find"], Program),
    (&["--"], Program),
    (&["--", "check"], Program),
    (&["-v"], Program),
    (&["-x", "check"], Program),
    (&["-e", "check"], Program),
    (&["--Version"], Program),
    (&["--VERSION"], Program),
    (&["-v", "--version"], Program),
    // Version and help.
    (&["-V"], Version(false)),
    (&["--version"], Version(true)),
    (&["-h"], Help),
    (&["--help"], Help),
    (&["--version", "extra"], Version(true)),
    (&["--help", "check"], Help),
    (&["-V", "check"], Version(false)),
    // Paths and programs in the notations of the language.
    (&["./check"], Program),
    (&["/usr/bin/check"], Program),
    (&["C:\\check"], Program),
    (&["check.nurt"], Program),
    (&["call(\"check\")"], Program),
    (&["help()"], Program),
    (&["exec()"], Program),
    (&["version:"], Program),
    (&["serve:"], Program),
    (&["check=1"], Program),
];

fn words(args: &[&str]) -> Vec<String> {
    args.iter().map(|word| (*word).to_owned()).collect()
}

fn expected(args: &[&str], expect: Expect) -> Invocation {
    match expect {
        Sub(name) => Invocation::Subcommand {
            name: SUBCOMMANDS
                .iter()
                .find(|known| **known == name)
                .unwrap_or_else(|| panic!("`{name}` is not a subcommand")),
            args: words(&args[1.min(args.len())..]),
        },
        Program => Invocation::Program { args: words(args) },
        Version(full) => Invocation::Version { full },
        Help => Invocation::Help,
    }
}

#[test]
fn corpus_has_at_least_one_hundred_invocations() {
    assert!(CORPUS.len() >= 100, "{} invocations", CORPUS.len());
}

#[test]
fn every_invocation_resolves_as_expected() {
    for (args, expect) in CORPUS {
        let actual = resolve(&words(args));
        let want = match expect {
            Sub("repl") if args.is_empty() => Invocation::Subcommand {
                name: "repl",
                args: Vec::new(),
            },
            other => expected(args, *other),
        };
        assert_eq!(actual, want, "arguments {args:?}");
    }
}

#[test]
fn every_subcommand_appears_in_the_corpus() {
    for name in SUBCOMMANDS {
        assert!(
            CORPUS
                .iter()
                .any(|(args, expect)| args.first() == Some(name)
                    && matches!(expect, Sub(found) if found == name)),
            "no case for `{name}`"
        );
    }
}

#[test]
fn only_exact_names_select_a_subcommand() {
    let selected = CORPUS
        .iter()
        .filter(|(_, expect)| matches!(expect, Sub(_)))
        .all(|(args, _)| args.is_empty() || SUBCOMMANDS.contains(&args[0]));
    assert!(selected);
    let strays = CORPUS
        .iter()
        .filter(|(args, _)| {
            args.first()
                .is_some_and(|first| SUBCOMMANDS.contains(first))
        })
        .all(|(_, expect)| matches!(expect, Sub(_) | Version(_) | Help));
    assert!(strays);
}
