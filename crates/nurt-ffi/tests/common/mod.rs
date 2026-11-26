// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Helpers shared by the interface tests.

#![allow(dead_code, reason = "each test target uses a subset of the helpers")]

use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// Directory of the crate.
pub(crate) fn manifest_dir() -> PathBuf {
    PathBuf::from(
        std::env::var("CARGO_MANIFEST_DIR")
            .as_deref()
            .unwrap_or(env!("CARGO_MANIFEST_DIR")),
    )
}

/// Directory of the public header.
pub(crate) fn include_dir() -> PathBuf {
    manifest_dir().join("include")
}

/// Shared library built for this test run, next to the test executable.
pub(crate) fn shared_library() -> PathBuf {
    let name = format!(
        "{}nurt_ffi{}",
        env::consts::DLL_PREFIX,
        env::consts::DLL_SUFFIX
    );
    let exe = env::current_exe().expect("path of the test executable");
    let deps = exe.parent().expect("directory of the test executable");
    let candidates = [
        deps.join(&name),
        deps.parent()
            .map_or_else(|| deps.join(&name), |dir| dir.join(&name)),
    ];
    candidates
        .iter()
        .find(|path| path.is_file())
        .cloned()
        .unwrap_or_else(|| panic!("the shared library {name} is not next to {}", exe.display()))
}

/// Program found in `PATH`, when present.
pub(crate) fn find_program(name: &str) -> Option<PathBuf> {
    let path = env::var_os("PATH")?;
    env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}

/// Runs a command and fails the test with its output when it does not
/// succeed.
pub(crate) fn run(command: &mut Command) -> Output {
    let output = command
        .output()
        .unwrap_or_else(|error| panic!("cannot run {command:?}: {error}"));
    assert!(
        output.status.success(),
        "{command:?} failed with {}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

/// Runs `command` with an empty environment whose `HOME` is a new directory
/// under `work`, so that the engine finds no configuration of the account
/// and keeps its state there.
pub(crate) fn run_isolated(command: &mut Command, work: &Path) -> Output {
    let home = work.join("home");
    fs::create_dir_all(&home).expect("home directory of the test");
    run(command.env_clear().env("HOME", &home))
}

/// Like [`run_isolated`], with the environment variable `name` set to `value`.
pub(crate) fn run_isolated_with(
    command: &mut Command,
    work: &Path,
    name: &str,
    value: &str,
) -> Output {
    let home = work.join("home");
    fs::create_dir_all(&home).expect("home directory of the test");
    run(command.env_clear().env("HOME", &home).env(name, value))
}

/// Name under which the dynamic loader of Linux searches the library: the
/// SONAME that `build.rs` gives to the shared library.
const LIBRARY_SONAME: &str = "libnurt.so.1";

/// Compiles a C program with `clang` against the header and links it with
/// the shared library.
///
/// The program records the SONAME of the library, not the file name
/// `libnurt_ffi.so`, so a link of that name to the library is placed next to
/// the output and its directory is the run path of the program.
pub(crate) fn compile_c(clang: &Path, sources: &[PathBuf], output: &Path, extra: &[&str]) {
    let library = shared_library();
    let library_dir = library.parent().expect("directory of the library");
    let output_dir = output.parent().expect("directory of the output");
    #[cfg(unix)]
    {
        let soname_link = output_dir.join(LIBRARY_SONAME);
        if !soname_link.exists() {
            std::os::unix::fs::symlink(&library, &soname_link)
                .expect("link of the library under its SONAME");
        }
    }
    let mut command = Command::new(clang);
    command
        .args([
            "-std=c11",
            "-Wall",
            "-Wextra",
            "-Werror",
            "-D_POSIX_C_SOURCE=200809L",
        ])
        .arg("-I")
        .arg(include_dir())
        .args(sources)
        .args(extra)
        .arg(&library)
        .arg(format!(
            "-Wl,-rpath,{},-rpath,{}",
            output_dir.display(),
            library_dir.display()
        ))
        .arg("-o")
        .arg(output);
    run(&mut command);
}

/// Statuses of the header, in the order of the columns of its table.
pub(crate) const STATUS_NAMES: [&str; 7] =
    ["OK", "TIMEOUT", "CANCELLED", "ARG", "ABI", "IO", "INTERNAL"];

/// Table "Possible status codes of each function" of `nurt.h`: function
/// name to the status names marked for it.
pub(crate) fn header_matrix() -> BTreeMap<String, BTreeSet<&'static str>> {
    let header = fs::read_to_string(include_dir().join("nurt.h")).expect("nurt.h");
    let mut lines = header
        .lines()
        .skip_while(|line| !line.contains("function                  OK"));
    let columns_line = lines.next().expect("header of the status table");
    let columns: Vec<(usize, &'static str)> = STATUS_NAMES
        .iter()
        .map(|name| {
            let position = columns_line
                .match_indices(name)
                .map(|(index, _)| index)
                .find(|index| {
                    columns_line[*index..].starts_with(&format!("{name} "))
                        || columns_line[*index..] == **name
                })
                .expect("status column");
            (position, *name)
        })
        .collect();
    let mut matrix = BTreeMap::new();
    for line in lines {
        let row = line.trim_start_matches(" *").trim_start();
        if !row.starts_with("nurt_") {
            break;
        }
        let function = row
            .split_whitespace()
            .next()
            .expect("function name")
            .to_owned();
        if function.ends_with(',') {
            break;
        }
        let statuses = line
            .match_indices('x')
            .filter(|(index, _)| *index >= columns[0].0)
            .map(|(index, _)| {
                columns
                    .iter()
                    .rev()
                    .find(|(start, _)| *start <= index)
                    .map(|(_, name)| *name)
                    .expect("column of the mark")
            })
            .collect();
        matrix.insert(function, statuses);
    }
    matrix
}

/// Exported functions declared in `nurt.h`.
pub(crate) fn header_functions() -> BTreeSet<String> {
    let header = fs::read_to_string(include_dir().join("nurt.h")).expect("nurt.h");
    header
        .split("NURT_API")
        .skip(1)
        .filter_map(|declaration| {
            let before_paren = declaration.split('(').next()?;
            let name = before_paren
                .split_whitespace()
                .last()?
                .trim_start_matches('*');
            name.starts_with("nurt_").then(|| name.to_owned())
        })
        .collect()
}

/// Names listed in `nurt.symbols`.
pub(crate) fn listed_symbols() -> BTreeSet<String> {
    fs::read_to_string(manifest_dir().join("nurt.symbols"))
        .expect("nurt.symbols")
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect()
}
