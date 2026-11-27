// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Tests of the symbol index: refresh by size and modification time,
//! storage in the state directory, the limits `code.index_max_files` and
//! `code.file_bytes_max`, listings with and without the file listing
//! service, and concurrent writes of one index.

// Tests that write POSIX paths run on Unix only; their helpers are unused on Windows.
#![cfg_attr(windows, allow(dead_code, unused_imports))]
// Tests report failures by panicking.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod u11_support;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::Duration;

use nurt_code::commands::symbols::CodeSymbols;
use nurt_code::index::store::index_file;
use nurt_code::index::{
    IndexOptions, SourceFilter, SymbolIndex, content_hash, index_tree, list_sources, read_source,
};
use nurt_code::registry::Registry;
use nurt_contracts::config::{Config, SizeSetting};
use nurt_contracts::error::ErrorCode;
use nurt_contracts::fs::Fs;
use nurt_contracts::report::Notable;
use nurt_contracts::services::ExcludePreset;
use nurt_contracts::stage::{ExecCtx, Limits};
use nurt_testkit::hash::sha256;
use nurt_testkit::vfs::test_path;

use u11_support::{ROOT, Workspace, args, config_with_max_files, run_symbols, text};

const MAIN_RS: &str = "pub fn main() {}\n";
const UTIL_PY: &str = "def helper():\n    return 1\n";
const API_GO: &str = "package api\n\nfunc Serve() {}\n";

fn registry() -> &'static Registry {
    Registry::builtin()
}

fn root() -> PathBuf {
    PathBuf::from(ROOT)
}

fn project() -> Workspace {
    let workspace = Workspace::new();
    workspace.write_all(&[
        ("src/main.rs", MAIN_RS),
        ("tools/util.py", UTIL_PY),
        ("api/api.go", API_GO),
    ]);
    workspace
}

fn index(ctx: &ExecCtx) -> (SymbolIndex, nurt_code::index::RefreshStats) {
    index_tree(ctx, registry(), &root(), &SourceFilter::default(), &|_| {
        true
    })
    .unwrap()
}

fn symbol_names(index: &SymbolIndex) -> Vec<String> {
    index
        .files()
        .iter()
        .flat_map(|file| file.symbols.iter().map(|symbol| symbol.name.clone()))
        .collect()
}

#[cfg(unix)] // the test writes POSIX absolute paths, which Windows reads as relative
#[test]
fn first_refresh_parses_every_listed_file() {
    let workspace = project();
    let (index, stats) = index(&workspace.ctx());
    assert_eq!(stats.files, 3);
    assert_eq!(stats.parsed, 3);
    assert_eq!(stats.reused, 0);
    assert!(!stats.truncated);
    let paths: Vec<&Path> = index
        .files()
        .iter()
        .map(|file| file.path.as_path())
        .collect();
    assert_eq!(
        paths,
        [
            Path::new("/repo/api/api.go"),
            Path::new("/repo/src/main.rs"),
            Path::new("/repo/tools/util.py"),
        ]
    );
    assert_eq!(symbol_names(&index), ["Serve", "main", "helper"]);
    assert_eq!(index.root(), Path::new(ROOT));
}

#[cfg(unix)] // the test writes POSIX absolute paths, which Windows reads as relative
#[test]
fn unchanged_files_are_reused_without_reading() {
    let workspace = project();
    let ctx = workspace.ctx();
    index(&ctx);
    let (_, stats) = index(&ctx);
    assert_eq!(stats.reused, 3);
    assert_eq!(stats.parsed, 0);
}

#[cfg(unix)] // the test writes POSIX absolute paths, which Windows reads as relative
#[test]
fn changed_modification_time_refreshes_the_file() {
    let workspace = project();
    let ctx = workspace.ctx();
    index(&ctx);
    workspace.test.clock().advance(Duration::from_secs(5));
    workspace.write("src/main.rs", "pub fn start() {}\n");
    let (index, stats) = index(&ctx);
    assert_eq!(stats.parsed, 1);
    assert_eq!(stats.reused, 2);
    assert!(symbol_names(&index).contains(&"start".to_owned()));
    assert!(!symbol_names(&index).contains(&"main".to_owned()));
}

#[cfg(unix)] // the test writes POSIX absolute paths, which Windows reads as relative
#[test]
fn changed_size_refreshes_the_file() {
    let workspace = project();
    let ctx = workspace.ctx();
    let (first, _) = index(&ctx);
    workspace.write(
        "tools/util.py",
        "def helper():\n    return 1\n\n\ndef extra():\n    pass\n",
    );
    let (second, stats) = index(&ctx);
    assert_eq!(stats.parsed, 1);
    let before = first
        .files()
        .iter()
        .find(|file| file.path.ends_with("util.py"))
        .unwrap();
    let after = second
        .files()
        .iter()
        .find(|file| file.path.ends_with("util.py"))
        .unwrap();
    assert_ne!(before.size, after.size);
    assert_ne!(before.etag, after.etag);
    assert_eq!(after.symbols.len(), 2);
}

#[cfg(unix)] // the test writes POSIX absolute paths, which Windows reads as relative
#[test]
fn files_no_longer_listed_are_removed() {
    let workspace = project();
    let ctx = workspace.ctx();
    let options = IndexOptions::from_ctx(&ctx);
    let mut index = SymbolIndex::new(ROOT);
    let all = [workspace.path("api/api.go"), workspace.path("src/main.rs")];
    let stats = index.refresh(&ctx, registry(), &all, &options).unwrap();
    assert_eq!(stats.parsed, 2);
    let stats = index
        .refresh(&ctx, registry(), &all[1..], &options)
        .unwrap();
    assert_eq!(stats.removed, 1);
    assert_eq!(stats.reused, 1);
    assert_eq!(index.files().len(), 1);
}

#[cfg(unix)] // the test writes POSIX absolute paths, which Windows reads as relative
#[test]
fn index_is_kept_in_the_state_directory_between_executions() {
    let workspace = project();
    index(&workspace.ctx());
    let stored = index_file(workspace.state.path(), Path::new(ROOT));
    assert!(stored.is_file(), "{}", stored.display());
    assert!(stored.starts_with(workspace.state.path().join("code-index")));
    let later = workspace.ctx();
    let reopened = SymbolIndex::open(&later, registry(), Path::new(ROOT));
    assert_eq!(reopened.files().len(), 3);
    let (_, stats) = index(&later);
    assert_eq!(stats.reused, 3);
    assert_eq!(stats.parsed, 0);
}

#[cfg(unix)] // the test writes POSIX absolute paths, which Windows reads as relative
#[test]
fn unreadable_stored_index_is_rebuilt() {
    let workspace = project();
    let stored = index_file(workspace.state.path(), Path::new(ROOT));
    std::fs::create_dir_all(stored.parent().unwrap()).unwrap();
    std::fs::write(&stored, b"{ not an index").unwrap();
    let ctx = workspace.ctx();
    assert!(
        SymbolIndex::open(&ctx, registry(), Path::new(ROOT))
            .files()
            .is_empty()
    );
    let (_, stats) = index(&ctx);
    assert_eq!(stats.parsed, 3);
    let reopened = SymbolIndex::open(&ctx, registry(), Path::new(ROOT));
    assert_eq!(reopened.files().len(), 3);
}

#[cfg(unix)] // the test writes POSIX absolute paths, which Windows reads as relative
#[test]
fn index_of_another_root_is_ignored() {
    let workspace = project();
    let ctx = workspace.ctx();
    index(&ctx);
    let stored = index_file(workspace.state.path(), Path::new(ROOT));
    let other = index_file(workspace.state.path(), Path::new("/elsewhere"));
    std::fs::copy(&stored, &other).unwrap();
    let reopened = SymbolIndex::open(&ctx, registry(), Path::new("/elsewhere"));
    assert!(reopened.files().is_empty());
}

#[cfg(unix)] // the test writes POSIX absolute paths, which Windows reads as relative
#[test]
fn without_state_directory_the_index_lives_for_one_execution() {
    let workspace = project();
    let ctx = workspace.ctx_without_state();
    let (index_once, stats) = index(&ctx);
    assert_eq!(stats.parsed, 3);
    assert!(!index_once.save(&ctx, registry()).unwrap());
    let (_, stats) = index(&ctx);
    assert_eq!(stats.parsed, 3);
}

#[cfg(unix)] // the test writes POSIX absolute paths, which Windows reads as relative
#[test]
fn listing_stops_at_index_max_files() {
    let workspace = project();
    workspace.write_all(&[("more/a.rs", "fn a() {}\n"), ("more/b.rs", "fn b() {}\n")]);
    let ctx = workspace.ctx_with(config_with_max_files(3));
    let (index, stats) = index(&ctx);
    assert!(stats.truncated);
    assert_eq!(stats.files, 3);
    assert_eq!(index.files().len(), 3);
}

#[cfg(unix)] // the test writes POSIX absolute paths, which Windows reads as relative
#[test]
fn symbol_search_reports_a_truncated_index() {
    let workspace = project();
    let ctx = workspace.ctx_with(config_with_max_files(2));
    let stage = CodeSymbols::new(None);
    let found = run_symbols(
        &stage,
        &args("code.symbols", &[("source", text("syntax"))]),
        &ctx,
    )
    .unwrap();
    assert!(found.len() <= 2);
    let notes = ctx.take_notes();
    let note = notes
        .iter()
        .find(|entry| entry.code == Notable::SourceUnavailable)
        .expect("a source_unavailable note");
    assert_eq!(note.path.as_deref(), Some(ROOT));
    assert!(
        note.detail
            .as_deref()
            .unwrap()
            .contains("code.index_max_files (2)")
    );
}

#[cfg(unix)] // the test writes POSIX absolute paths, which Windows reads as relative
#[test]
fn files_larger_than_file_bytes_max_are_skipped() {
    let workspace = project();
    let large = format!("pub fn large() {{}}\n{}", "// padding\n".repeat(20));
    workspace.write("src/large.rs", &large);
    let mut config = Config::default();
    config.code.file_bytes_max = SizeSetting::from_bytes(64);
    let ctx = workspace.ctx_with(config);
    let (index, stats) = index(&ctx);
    assert_eq!(stats.skipped, 1);
    assert!(!symbol_names(&index).contains(&"large".to_owned()));
    let error = read_source(&ctx, &workspace.path("src/large.rs")).unwrap_err();
    assert_eq!(error.code(), ErrorCode::Limit);
    assert!(error.to_string().contains("code.file_bytes_max"));
}

#[cfg(unix)] // the test writes POSIX absolute paths, which Windows reads as relative
#[test]
fn content_hash_equals_the_file_system_digest() {
    let workspace = project();
    let path = workspace.path("src/main.rs");
    let ctx = workspace.ctx();
    let source = read_source(&ctx, &path).unwrap();
    assert_eq!(source.etag, content_hash(MAIN_RS.as_bytes()));
    assert_eq!(source.etag, sha256(MAIN_RS.as_bytes()));
    let absolute = test_path(&path.to_string_lossy()).unwrap();
    assert_eq!(
        workspace.test.fs().content_hash(&absolute).unwrap(),
        source.etag
    );
    let (index, _) = index(&ctx);
    let indexed = index.files().iter().find(|file| file.path == path).unwrap();
    assert_eq!(indexed.etag, source.etag.to_string());
}

#[cfg(unix)] // the test writes POSIX absolute paths, which Windows reads as relative
#[test]
fn parsing_on_several_threads_gives_the_same_index() {
    let workspace = project();
    for index in 0..12 {
        workspace.write(
            &format!("gen/f{index:02}.rs"),
            &format!("fn f{index}() {{}}\n"),
        );
    }
    let single = workspace
        .test
        .builder()
        .limits(Limits {
            threads: 1,
            ..Limits::default()
        })
        .build();
    let many = workspace
        .test
        .builder()
        .limits(Limits {
            threads: 4,
            ..Limits::default()
        })
        .build();
    let (first, _) = index(&single);
    let (second, _) = index(&many);
    assert_eq!(first.files(), second.files());
    assert_eq!(first.files().len(), 15);
}

#[cfg(unix)] // the test writes POSIX absolute paths, which Windows reads as relative
#[test]
fn listing_service_receives_a_source_tree_request() {
    let workspace = project();
    workspace.write_all(&[
        ("target/debug/build.rs", "fn generated() {}\n"),
        ("gen/out.rs", "fn out() {}\n"),
        ("node_modules/pkg/index.js", "function dep() {}\n"),
    ]);
    let ctx = workspace.ctx();
    let filter = SourceFilter {
        globs: Vec::new(),
        exclude: vec!["gen".to_owned()],
        presets: vec![ExcludePreset::Build],
    };
    let listing = list_sources(&ctx, &root(), &filter, &|_| true, 100).unwrap();
    assert_eq!(listing.files.len(), 3);
    let requests = workspace.listing.requests();
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert!(request.gitignore);
    assert_eq!(request.root, root());
    assert_eq!(request.exclude, ["gen"]);
    for preset in [
        ExcludePreset::Vcs,
        ExcludePreset::Deps,
        ExcludePreset::Cache,
        ExcludePreset::Build,
    ] {
        assert!(request.exclude_presets.contains(&preset), "{preset:?}");
    }
}

#[cfg(unix)] // the test writes POSIX absolute paths, which Windows reads as relative
#[test]
fn listing_without_service_skips_dependency_and_version_control_directories() {
    let workspace = Workspace::without_listing();
    workspace.write_all(&[
        ("src/main.rs", MAIN_RS),
        (".git/hooks/pre-commit.py", UTIL_PY),
        ("node_modules/pkg/index.js", "function dep() {}\n"),
        ("venv/lib/site.py", UTIL_PY),
        ("app/__pycache__/cached.py", UTIL_PY),
        ("generated/code.rs", "fn generated() {}\n"),
        ("build/out.rs", "fn out() {}\n"),
    ]);
    let ctx = workspace.ctx();
    let filter = SourceFilter {
        globs: vec!["**/*.rs".to_owned()],
        exclude: vec!["generated".to_owned()],
        presets: vec![ExcludePreset::Build],
    };
    let listing = list_sources(&ctx, &root(), &filter, &|_| true, 100).unwrap();
    assert_eq!(listing.files, [workspace.path("src/main.rs")]);
    let everything = list_sources(&ctx, &root(), &SourceFilter::default(), &|_| true, 100).unwrap();
    assert_eq!(
        everything.files,
        [
            workspace.path("build/out.rs"),
            workspace.path("generated/code.rs"),
            workspace.path("src/main.rs"),
        ]
    );
    assert!(workspace.listing.requests().is_empty());
}

#[cfg(unix)] // the test writes POSIX absolute paths, which Windows reads as relative
#[test]
fn listing_applies_the_acceptance_function() {
    let workspace = project();
    let ctx = workspace.ctx();
    let only_python = |path: &Path| path.extension().is_some_and(|ext| ext == "py");
    let listing = list_sources(&ctx, &root(), &SourceFilter::default(), &only_python, 10).unwrap();
    assert_eq!(listing.files, [workspace.path("tools/util.py")]);
    assert!(!listing.truncated);
}

/// Rounds of the concurrent write test.
const CONCURRENT_ROUNDS: usize = 25;

#[test]
fn concurrent_writes_leave_one_complete_index() {
    let shared = tempfile::tempdir().unwrap();
    let small = project();
    let large = project();
    large.write_all(&[("more/a.rs", "fn a() {}\n"), ("more/b.rs", "fn b() {}\n")]);
    let ctx_of = |workspace: &Workspace| {
        workspace
            .test
            .builder()
            .state_dir(shared.path().to_path_buf())
            .build()
    };
    let small_ctx = ctx_of(&small);
    let large_ctx = ctx_of(&large);
    let (small_index, _) = index(&small_ctx);
    let (large_index, _) = index(&large_ctx);
    let barrier = Arc::new(Barrier::new(3));
    thread::scope(|scope| {
        for (ctx, index) in [(&small_ctx, &small_index), (&large_ctx, &large_index)] {
            let barrier = barrier.clone();
            scope.spawn(move || {
                for _ in 0..CONCURRENT_ROUNDS {
                    barrier.wait();
                    assert!(index.save(ctx, registry()).unwrap());
                }
            });
        }
        for _ in 0..CONCURRENT_ROUNDS {
            barrier.wait();
            let seen = SymbolIndex::open(&small_ctx, registry(), Path::new(ROOT));
            assert!(
                seen.files() == small_index.files() || seen.files() == large_index.files(),
                "a reader saw a partial index of {} files",
                seen.files().len()
            );
        }
    });
    let stored = SymbolIndex::open(&small_ctx, registry(), Path::new(ROOT));
    assert!(stored.files() == small_index.files() || stored.files() == large_index.files());
    let leftovers: Vec<_> = std::fs::read_dir(shared.path().join("code-index"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(leftovers.len(), 1, "{leftovers:?}");
}

#[test]
fn concurrent_refreshes_of_one_root_agree() {
    let workspace = project();
    let first = workspace.ctx();
    let second = workspace.ctx();
    let results = thread::scope(|scope| {
        let left = scope.spawn(|| index(&first).0);
        let right = scope.spawn(|| index(&second).0);
        [left.join().unwrap(), right.join().unwrap()]
    });
    assert_eq!(results[0].files(), results[1].files());
    let stored = SymbolIndex::open(&first, registry(), Path::new(ROOT));
    assert_eq!(stored.files(), results[0].files());
}
