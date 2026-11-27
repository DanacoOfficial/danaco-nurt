// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-code/tests/u11_perf.rs
// Responsibility:   Measurements of the symbol index on a generated repository of 20 000 source files in the virtual file system.
// Dependencies:     nurt_contracts, nurt_testkit, u11_support
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Measurements of the symbol index on a generated repository of 20 000
//! source files in the virtual file system.
//!
//! The tests are ignored by default; each one repeats the work of the
//! previous one and adds a step, so the differences of their run times
//! give the time of each step:
//!
//! ```text
//! cargo test -p nurt-code --release --test u11_perf -- --ignored --exact generates_the_repository
//! cargo test -p nurt-code --release --test u11_perf -- --ignored --exact indexes_the_repository
//! cargo test -p nurt-code --release --test u11_perf -- --ignored --exact refreshes_and_searches_the_repository
//! ```

// Tests report failures by panicking.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod u11_support;

use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::thread;

use nurt_code::commands::symbols::CodeSymbols;
use nurt_code::index::{RefreshStats, SourceFilter, SymbolIndex, index_tree};
use nurt_code::registry::Registry;
use nurt_contracts::config::Config;
use nurt_contracts::stage::{ExecCtx, Limits};
use nurt_testkit::vfs::test_path;

use u11_support::{ROOT, Workspace, args, names, path_value, run_symbols, text, word};

/// Files of the generated repository.
const FILES: usize = 20_000;

/// Files per directory.
const FILES_PER_DIRECTORY: usize = 100;

/// Smallest number of symbols defined by a generated file.
const MIN_SYMBOLS_PER_FILE: usize = 2;

fn source(index: usize) -> (String, String) {
    let directory = index / FILES_PER_DIRECTORY;
    match index % 4 {
        0 => (
            format!("pkg{directory:03}/module{index:05}.rs"),
            format!(
                "pub struct Item{index} {{\n    value: u32,\n}}\n\npub fn build_{index}(value: u32) -> Item{index} {{\n    Item{index} {{ value }}\n}}\n"
            ),
        ),
        1 => (
            format!("pkg{directory:03}/module{index:05}.py"),
            format!(
                "class Item{index}:\n    pass\n\n\ndef build_{index}(value):\n    return Item{index}()\n"
            ),
        ),
        2 => (
            format!("pkg{directory:03}/module{index:05}.go"),
            format!(
                "package pkg\n\ntype Item{index} struct {{\n\tValue int\n}}\n\nfunc Build{index}(value int) Item{index} {{\n\treturn Item{index}{{Value: value}}\n}}\n"
            ),
        ),
        _ => (
            format!("pkg{directory:03}/module{index:05}.ts"),
            format!(
                "export interface Item{index} {{\n  value: number;\n}}\n\nexport function build{index}(value: number): Item{index} {{\n  return {{ value }};\n}}\n"
            ),
        ),
    }
}

fn repository() -> Workspace {
    let workspace = Workspace::new();
    for index in 0..FILES {
        let (path, content) = source(index);
        workspace.write(&path, &content);
    }
    workspace
}

fn context(workspace: &Workspace) -> ExecCtx {
    let mut config = Config::default();
    config.code.index_max_files = u32::try_from(FILES * 2).unwrap();
    let threads = thread::available_parallelism().map_or(1, NonZeroUsize::get);
    workspace
        .test
        .builder()
        .config(std::sync::Arc::new(config))
        .limits(Limits {
            threads: u32::try_from(threads).unwrap(),
            ..Limits::default()
        })
        .state_dir(workspace.state.path().to_path_buf())
        .build()
}

fn root() -> PathBuf {
    PathBuf::from(ROOT)
}

fn index(ctx: &ExecCtx) -> (SymbolIndex, RefreshStats) {
    index_tree(
        ctx,
        Registry::builtin(),
        &root(),
        &SourceFilter::default(),
        &|_| true,
    )
    .unwrap()
}

#[test]
#[ignore = "measurement on 20 000 files; run with --ignored"]
fn generates_the_repository() {
    let workspace = repository();
    let (last, _) = source(FILES - 1);
    let path = Path::new(ROOT).join(last);
    assert!(
        workspace
            .test
            .fs()
            .exists(&test_path(&path.to_string_lossy()).unwrap())
    );
}

#[test]
#[ignore = "measurement on 20 000 files; run with --ignored"]
fn indexes_the_repository() {
    let workspace = repository();
    let (index, stats) = index(&context(&workspace));
    assert_eq!(stats.files, FILES);
    assert_eq!(stats.parsed, FILES);
    assert!(!stats.truncated);
    let symbols: usize = index.files().iter().map(|file| file.symbols.len()).sum();
    assert!(symbols >= FILES * MIN_SYMBOLS_PER_FILE, "{symbols} symbols");
}

#[test]
#[ignore = "measurement on 20 000 files; run with --ignored"]
fn refreshes_and_searches_the_repository() {
    let workspace = repository();
    let ctx = context(&workspace);
    index(&ctx);
    let (_, stats) = index(&ctx);
    assert_eq!(stats.reused, FILES);
    assert_eq!(stats.parsed, 0);
    let stage = CodeSymbols::new(None);
    let found = run_symbols(
        &stage,
        &args(
            "code.symbols",
            &[
                ("path", path_value(&root())),
                ("name", text("build_19996")),
                ("match", word("exact")),
                ("source", word("syntax")),
            ],
        ),
        &ctx,
    )
    .unwrap();
    assert_eq!(names(&found), ["build_19996"]);
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
