// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-git-difftest/tests/no_process.rs
// Responsibility:   Checks of the product crates of the Git commands: no process is started.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Checks of the product crates of the Git commands: no process is started
//! outside the hook module, and no native Git, TLS or HTTP library is
//! linked.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// Product crates of the Git commands.
const PRODUCT_CRATES: &[&str] = &["nurt-git", "nurt-git-ops", "nurt-git-transport"];

/// Text that starts a process or prepares one.
const PROCESS_PATTERNS: &[&str] = &[
    "std::process::Command",
    "process::Command",
    "std::process::{",
];

/// Packages that would bring a native Git, TLS or HTTP library.
const FORBIDDEN_PACKAGES: &[&str] = &["git2", "libgit2-sys", "openssl-sys", "curl", "curl-sys"];

/// Marker after which a source file holds only its unit tests.
const TEST_MODULE_MARKER: &str = "#[cfg(test)]";

fn workspace_root() -> PathBuf {
    Path::new(
        std::env::var("CARGO_MANIFEST_DIR")
            .as_deref()
            .unwrap_or(env!("CARGO_MANIFEST_DIR")),
    )
    .join("..")
    .join("..")
}

fn rust_files(dir: &Path, found: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, found);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            found.push(path);
        }
    }
}

/// Hook modules start the programs of the user; test modules start `git`.
/// A module is a test module when its name starts with `test` or ends with
/// `_tests`; a name that only contains the letters, such as `latest.rs`, is
/// not exempt.
fn is_exempt(src: &Path, file: &Path) -> bool {
    file.strip_prefix(src).is_ok_and(|relative| {
        relative.components().any(|component| {
            let name = component.as_os_str().to_string_lossy();
            let stem = name.strip_suffix(".rs").unwrap_or(&*name);
            name.starts_with("hooks") || stem.starts_with("test") || stem.ends_with("_tests")
        })
    })
}

#[test]
fn only_hook_and_test_modules_are_exempt_from_the_process_check() {
    let src = Path::new("/crate/src");
    for exempt in [
        "/crate/src/hooks.rs",
        "/crate/src/hooks/run.rs",
        "/crate/src/tests.rs",
        "/crate/src/apply/testjournal.rs",
        "/crate/src/push/ssh_tests.rs",
        "/crate/src/testutil/mod.rs",
    ] {
        assert!(is_exempt(src, Path::new(exempt)), "{exempt}");
    }
    for checked in [
        "/crate/src/latest.rs",
        "/crate/src/attestation.rs",
        "/crate/src/contest/run.rs",
        "/crate/src/contests.rs",
        "/crate/src/apply/mod.rs",
    ] {
        assert!(!is_exempt(src, Path::new(checked)), "{checked}");
    }
    assert!(!is_exempt(src, Path::new("/elsewhere/hooks.rs")));
}

#[test]
fn product_crates_start_no_process_outside_the_hook_module() {
    let mut found = Vec::new();
    for name in PRODUCT_CRATES {
        let src = workspace_root().join("crates").join(name).join("src");
        let mut files = Vec::new();
        rust_files(&src, &mut files);
        for file in files.iter().filter(|file| !is_exempt(&src, file)) {
            let text = std::fs::read_to_string(file).unwrap();
            let production = text.split(TEST_MODULE_MARKER).next().unwrap_or_default();
            for (number, line) in production.lines().enumerate() {
                let code = !line.trim_start().starts_with("//");
                if code
                    && PROCESS_PATTERNS
                        .iter()
                        .any(|pattern| line.contains(pattern))
                {
                    found.push(format!(
                        "{}:{}: {}",
                        file.display(),
                        number + 1,
                        line.trim()
                    ));
                }
            }
        }
    }
    assert!(
        found.is_empty(),
        "processes outside the hook module:\n{}",
        found.join("\n")
    );
}

#[test]
fn product_crates_link_no_native_git_tls_or_http_library() {
    let text = std::fs::read_to_string(workspace_root().join("Cargo.lock")).unwrap();
    let lock: toml::Table = toml::from_str(&text).unwrap();
    let mut dependencies: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for package in lock["package"].as_array().unwrap() {
        let name = package["name"].as_str().unwrap().to_owned();
        let entry = dependencies.entry(name).or_default();
        for dependency in package
            .get("dependencies")
            .and_then(toml::Value::as_array)
            .into_iter()
            .flatten()
        {
            let text = dependency.as_str().unwrap();
            entry.insert(text.split(' ').next().unwrap_or(text).to_owned());
        }
    }
    let mut reached = BTreeSet::new();
    let mut pending: Vec<String> = PRODUCT_CRATES
        .iter()
        .map(|name| (*name).to_owned())
        .collect();
    while let Some(name) = pending.pop() {
        if reached.insert(name.clone()) {
            pending.extend(dependencies.get(&name).into_iter().flatten().cloned());
        }
    }
    let linked: Vec<&str> = FORBIDDEN_PACKAGES
        .iter()
        .copied()
        .filter(|package| reached.contains(*package))
        .collect();
    assert!(
        linked.is_empty(),
        "the Git crates depend on {}",
        linked.join(", ")
    );
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
