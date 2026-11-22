// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Differential tests of `nurt_archive` against `unzip`, `tar`, `bsdtar`, `zip` and `7z`.

#![cfg(all(feature = "differential", unix))]
// Test helpers unwrap results so that a failure aborts the test with the original error.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support_corpus;
mod support_diff;

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};

use nurt_archive::create::{CreateOptions, Input, create};
use nurt_archive::list::{Format, MemberKind, list};
use nurt_archive::sevenz::{self, EntryKind, Metadata};
use support_corpus::Rng;
use support_diff::{require, run_expect};

const SEEDS: u64 = 6;
const DIRECTORIES: [&str; 4] = ["d0", "d0/d1", "d2", "d2/d3/d4"];
const FILES: u64 = 24;
const MAX_FILE_BYTES: u64 = 3_000;
const FILE_MODE: u32 = 0o644;
const DIRECTORY_MODE: u32 = 0o755;
const ARCHIVE_DIR: &str = "out";

/// Names with only ASCII characters; Unicode names are covered by unit tests.
struct Tree {
    directories: Vec<String>,
    files: BTreeMap<String, Vec<u8>>,
}

fn tree(seed: u64) -> Tree {
    let mut rng = Rng::new(seed);
    let directories: Vec<String> = DIRECTORIES.iter().map(|d| (*d).to_owned()).collect();
    let mut files = BTreeMap::new();
    for index in 0..FILES {
        let name = match rng.below(DIRECTORIES.len() as u64 + 1) {
            slot if slot as usize == DIRECTORIES.len() => format!("root_{index}.dat"),
            slot => format!("{}/file_{index}.txt", DIRECTORIES[slot as usize]),
        };
        let length = if index == 0 {
            0
        } else {
            rng.below(MAX_FILE_BYTES)
        };
        let content = (0..length).map(|_| rng.next_u64() as u8).collect();
        files.insert(name, content);
    }
    Tree { directories, files }
}

/// Directory entries with their parents, in creation order.
fn all_directories(tree: &Tree) -> Vec<String> {
    let mut all = Vec::new();
    for directory in &tree.directories {
        let mut prefix = String::new();
        for part in directory.split('/') {
            if !prefix.is_empty() {
                prefix.push('/');
            }
            prefix.push_str(part);
            if !all.contains(&prefix) {
                all.push(prefix.clone());
            }
        }
    }
    all
}

fn write_tree(root: &Path, tree: &Tree) {
    for directory in &tree.directories {
        fs::create_dir_all(root.join(directory)).unwrap();
    }
    for (name, content) in &tree.files {
        let path = root.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }
}

fn assert_tree_extracted(root: &Path, tree: &Tree, label: &str) {
    for (name, content) in &tree.files {
        let path: PathBuf = root.join(name);
        assert_eq!(
            &fs::read(&path).unwrap(),
            content,
            "{label}: content of {name}"
        );
    }
    for directory in all_directories(tree) {
        assert!(
            root.join(&directory).is_dir(),
            "{label}: directory {directory}"
        );
    }
}

fn inputs(tree: &Tree) -> Vec<Input> {
    let mut inputs: Vec<Input> = all_directories(tree)
        .into_iter()
        .map(|directory| Input::directory(directory).with_mode(DIRECTORY_MODE))
        .collect();
    inputs.extend(tree.files.iter().map(|(name, content)| {
        Input::file(name.clone(), content.len() as u64).with_mode(FILE_MODE)
    }));
    inputs
}

fn nurt_archive_bytes(format: Format, tree: &Tree) -> Vec<u8> {
    let contents: HashMap<&str, &[u8]> = tree
        .files
        .iter()
        .map(|(name, content)| (name.as_str(), content.as_slice()))
        .collect();
    create(
        Vec::new(),
        &CreateOptions::new(format),
        inputs(tree),
        |input| {
            let content = contents
                .get(input.path.as_str())
                .copied()
                .unwrap_or_default();
            Ok(Box::new(content))
        },
    )
    .unwrap()
}

/// Name without a leading `./` and a trailing `/`; empty for the archive root.
fn clean(name: &str) -> String {
    let mut name = name;
    while let Some(rest) = name.strip_prefix("./") {
        name = rest;
    }
    name.trim_end_matches('/')
        .trim_start_matches("./")
        .to_owned()
}

/// Names and sizes reported by `nurt_archive::list`, directories with size 0.
fn nurt_listing(bytes: Vec<u8>) -> (Format, Vec<(String, u64)>) {
    let mut members = Vec::new();
    let format = list(Cursor::new(bytes), None, |member| {
        let size = if member.kind == MemberKind::Directory {
            0
        } else {
            member.size
        };
        members.push((clean(&member.path), size));
        Ok(())
    })
    .unwrap();
    members.retain(|(name, _)| !name.is_empty() && name != ".");
    members.sort();
    (format, members)
}

fn manifest(tree: &Tree) -> Vec<(String, u64)> {
    let mut expected: Vec<(String, u64)> =
        all_directories(tree).into_iter().map(|d| (d, 0)).collect();
    expected.extend(
        tree.files
            .iter()
            .map(|(name, content)| (name.clone(), content.len() as u64)),
    );
    expected.sort();
    expected
}

fn sorted(mut listing: Vec<(String, u64)>) -> Vec<(String, u64)> {
    listing.retain(|(name, _)| !name.is_empty() && name != ".");
    listing.sort();
    listing
}

/// Parses the table printed by `unzip -l`.
fn parse_unzip_listing(text: &str) -> Vec<(String, u64)> {
    let mut inside = false;
    let mut entries = Vec::new();
    for line in text.lines() {
        if line.trim_start().starts_with("------") {
            if inside {
                break;
            }
            inside = true;
            continue;
        }
        if !inside {
            continue;
        }
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() >= 4 {
            entries.push((clean(&fields[3..].join(" ")), fields[0].parse().unwrap()));
        }
    }
    sorted(entries)
}

/// Parses `tar -tvf`: `mode user/group size date time name`.
fn parse_tar_listing(text: &str) -> Vec<(String, u64)> {
    let entries = text
        .lines()
        .map(|line| {
            let fields: Vec<&str> = line.split_whitespace().collect();
            (clean(&fields[5..].join(" ")), fields[2].parse().unwrap())
        })
        .collect();
    sorted(entries)
}

fn names_only(listing: &[(String, u64)]) -> Vec<String> {
    listing.iter().map(|(name, _)| name.clone()).collect()
}

fn parse_name_list(text: &str) -> Vec<String> {
    let mut names: Vec<String> = text
        .lines()
        .map(clean)
        .filter(|n| !n.is_empty() && n != ".")
        .collect();
    names.sort();
    names
}

/// Parses `7z l -slt` into `(path, size)` pairs; directories have size 0.
fn parse_sevenz_listing(text: &str) -> Vec<(String, u64)> {
    let mut entries = Vec::new();
    for block in text.split("\n\n") {
        let mut fields: HashMap<&str, &str> = HashMap::new();
        for line in block.lines() {
            if let Some((key, value)) = line.split_once(" = ") {
                fields.insert(key, value);
            }
        }
        let Some(path) = fields.get("Path") else {
            continue;
        };
        let directory = fields.get("Folder") == Some(&"+")
            || fields.get("Attributes").is_some_and(|a| a.starts_with('D'));
        let size = if directory {
            0
        } else {
            fields.get("Size").map_or(0, |s| s.parse().unwrap())
        };
        entries.push((clean(path), size));
    }
    sorted(entries)
}

#[test]
fn zip_created_by_nurt_is_listed_by_unzip() {
    let Some(unzip) = require("unzip") else {
        return;
    };
    for seed in 0..SEEDS {
        let tree = tree(seed);
        let dir = tempfile::tempdir().unwrap();
        let bytes = nurt_archive_bytes(Format::Zip, &tree);
        fs::write(dir.path().join("a.zip"), &bytes).unwrap();
        let output = run_expect(&unzip, &["-l", "a.zip"], dir.path(), &[0]);
        let reference = parse_unzip_listing(&String::from_utf8(output.stdout).unwrap());
        let (format, nurt) = nurt_listing(bytes);
        assert_eq!(format, Format::Zip);
        assert_eq!(nurt, reference, "seed {seed}");
        assert_eq!(reference, manifest(&tree), "seed {seed}: manifest");
    }
}

#[test]
fn zip_created_by_nurt_passes_the_unzip_test_and_extracts() {
    let Some(unzip) = require("unzip") else {
        return;
    };
    for seed in 0..SEEDS {
        let tree = tree(seed);
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("a.zip"),
            nurt_archive_bytes(Format::Zip, &tree),
        )
        .unwrap();
        run_expect(&unzip, &["-tqq", "a.zip"], dir.path(), &[0]);
        run_expect(
            &unzip,
            &["-qq", "a.zip", "-d", ARCHIVE_DIR],
            dir.path(),
            &[0],
        );
        assert_tree_extracted(&dir.path().join(ARCHIVE_DIR), &tree, "unzip");
    }
}

#[test]
fn zip_created_by_zip_is_listed_by_nurt() {
    let Some(zip) = require("zip") else {
        return;
    };
    for seed in 0..SEEDS {
        let tree = tree(seed);
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("src");
        write_tree(&source, &tree);
        run_expect(&zip, &["-q", "-r", "-X", "../ref.zip", "."], &source, &[0]);
        let bytes = fs::read(dir.path().join("ref.zip")).unwrap();
        let (format, nurt) = nurt_listing(bytes);
        assert_eq!(format, Format::Zip);
        assert_eq!(nurt, manifest(&tree), "seed {seed}");
    }
}

#[test]
fn tar_created_by_nurt_is_listed_by_tar_and_bsdtar() {
    let tar = require("tar");
    let bsdtar = require("bsdtar");
    for seed in 0..SEEDS {
        let tree = tree(seed);
        let dir = tempfile::tempdir().unwrap();
        let bytes = nurt_archive_bytes(Format::Tar(None), &tree);
        fs::write(dir.path().join("a.tar"), &bytes).unwrap();
        let (format, nurt) = nurt_listing(bytes);
        assert_eq!(format, Format::Tar(None));
        assert_eq!(nurt, manifest(&tree), "seed {seed}: manifest");
        if let Some(tar) = &tar {
            let output = run_expect(tar, &["-tvf", "a.tar"], dir.path(), &[0]);
            let reference = parse_tar_listing(&String::from_utf8(output.stdout).unwrap());
            assert_eq!(nurt, reference, "seed {seed}: tar -tvf");
        }
        if let Some(bsdtar) = &bsdtar {
            let output = run_expect(bsdtar, &["-tf", "a.tar"], dir.path(), &[0]);
            let reference = parse_name_list(&String::from_utf8(output.stdout).unwrap());
            assert_eq!(names_only(&nurt), reference, "seed {seed}: bsdtar -tf");
        }
    }
}

#[test]
fn tar_created_by_nurt_extracts_with_tar() {
    let Some(tar) = require("tar") else {
        return;
    };
    for seed in 0..SEEDS {
        let tree = tree(seed);
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("a.tar"),
            nurt_archive_bytes(Format::Tar(None), &tree),
        )
        .unwrap();
        fs::create_dir_all(dir.path().join(ARCHIVE_DIR)).unwrap();
        run_expect(&tar, &["-xf", "a.tar", "-C", ARCHIVE_DIR], dir.path(), &[0]);
        assert_tree_extracted(&dir.path().join(ARCHIVE_DIR), &tree, "tar");
    }
}

#[test]
fn tar_created_by_tar_is_listed_by_nurt() {
    let Some(tar) = require("tar") else {
        return;
    };
    for seed in 0..SEEDS {
        let tree = tree(seed);
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("src");
        write_tree(&source, &tree);
        run_expect(&tar, &["-cf", "../ref.tar", "."], &source, &[0]);
        let (format, nurt) = nurt_listing(fs::read(dir.path().join("ref.tar")).unwrap());
        assert_eq!(format, Format::Tar(None));
        assert_eq!(nurt, manifest(&tree), "seed {seed}");
    }
}

fn nurt_sevenz_bytes(tree: &Tree) -> Vec<u8> {
    let mut creator = sevenz::Creator::new(Cursor::new(Vec::new())).unwrap();
    let directory = Metadata {
        modified: None,
        mode: Some(DIRECTORY_MODE),
    };
    let file = Metadata {
        modified: None,
        mode: Some(FILE_MODE),
    };
    for name in all_directories(tree) {
        creator.add_directory(&name, directory).unwrap();
    }
    for (name, content) in &tree.files {
        creator.add_file(name, file, content.as_slice()).unwrap();
    }
    creator.finish().unwrap().into_inner()
}

#[test]
fn sevenz_created_by_nurt_is_listed_by_7z() {
    let Some(sevenz_tool) = require("7z") else {
        return;
    };
    for seed in 0..SEEDS {
        let tree = tree(seed);
        let dir = tempfile::tempdir().unwrap();
        let bytes = nurt_sevenz_bytes(&tree);
        fs::write(dir.path().join("a.7z"), &bytes).unwrap();
        let output = run_expect(
            &sevenz_tool,
            &["l", "-ba", "-slt", "a.7z"],
            dir.path(),
            &[0],
        );
        let reference = parse_sevenz_listing(&String::from_utf8(output.stdout).unwrap());
        let nurt: Vec<(String, u64)> = sorted(
            sevenz::list(Cursor::new(bytes))
                .unwrap()
                .into_iter()
                .map(|entry| {
                    let size = if entry.kind == EntryKind::Directory {
                        0
                    } else {
                        entry.size
                    };
                    (clean(&entry.path), size)
                })
                .collect(),
        );
        assert_eq!(nurt, reference, "seed {seed}");
        assert_eq!(reference, manifest(&tree), "seed {seed}: manifest");
    }
}

#[test]
fn sevenz_created_by_nurt_passes_the_7z_test_and_extracts() {
    let Some(sevenz_tool) = require("7z") else {
        return;
    };
    for seed in 0..SEEDS {
        let tree = tree(seed);
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.7z"), nurt_sevenz_bytes(&tree)).unwrap();
        run_expect(&sevenz_tool, &["t", "-bd", "a.7z"], dir.path(), &[0]);
        run_expect(
            &sevenz_tool,
            &["x", "-bd", "-y", "-oout", "a.7z"],
            dir.path(),
            &[0],
        );
        assert_tree_extracted(&dir.path().join(ARCHIVE_DIR), &tree, "7z");
    }
}

#[test]
fn sevenz_created_by_7z_is_read_by_nurt() {
    let Some(sevenz_tool) = require("7z") else {
        return;
    };
    for seed in 0..SEEDS {
        let tree = tree(seed);
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("src");
        write_tree(&source, &tree);
        let target = dir.path().join("ref.7z");
        let target = target.to_str().unwrap();
        run_expect(
            &sevenz_tool,
            &["a", "-bd", "-y", target, "*"],
            &source,
            &[0],
        );
        let mut contents: BTreeMap<String, Vec<u8>> = BTreeMap::new();
        let mut names = Vec::new();
        sevenz::read_entries(
            Cursor::new(fs::read(target).unwrap()),
            None,
            |entry, reader| {
                let mut data = Vec::new();
                std::io::Read::read_to_end(reader, &mut data).map_err(sevenz::Error::Io)?;
                let size = if entry.kind == EntryKind::Directory {
                    0
                } else {
                    entry.size
                };
                names.push((clean(&entry.path), size));
                if entry.kind == EntryKind::File {
                    contents.insert(clean(&entry.path), data);
                }
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(sorted(names), manifest(&tree), "seed {seed}");
        assert_eq!(contents, tree.files, "seed {seed}: content");
    }
}

#[test]
fn damaged_zip_is_rejected_by_both() {
    let Some(unzip) = require("unzip") else {
        return;
    };
    let tree = tree(1);
    let dir = tempfile::tempdir().unwrap();
    let mut bytes = nurt_archive_bytes(Format::Zip, &tree);
    bytes.truncate(bytes.len() / 2);
    fs::write(dir.path().join("cut.zip"), &bytes).unwrap();
    let output = support_diff::run(
        &unzip,
        &["-tqq", "cut.zip"],
        dir.path(),
        support_diff::DEFAULT_TIMEOUT,
    )
    .unwrap();
    assert_ne!(output.code, Some(0), "unzip accepts the truncated archive");
    let result = list(Cursor::new(bytes), None, |_| Ok(()));
    assert!(result.is_err(), "Nurt accepts the truncated archive");
}
