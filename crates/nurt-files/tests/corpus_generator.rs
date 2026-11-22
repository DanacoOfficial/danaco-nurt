// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Tests of the deterministic corpus generator.

// Test code unwraps results so that a failure aborts the test with the original error.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support_corpus;

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use support_corpus::{
    Corpus, CorpusSpec, Kind, LONG_LINE_BYTES, NEEDLE, encode_cp852, encode_utf16le_marked,
    generate_empty_tree, generate_large, generate_replace_tree, polish_sentence,
};
use unicode_normalization::{is_nfc, is_nfd};

const SAMPLE_FILES: usize = 1_500;

fn snapshot(root: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for item in fs::read_dir(&dir).unwrap() {
            let path = item.unwrap().path();
            let rel = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            let meta = fs::symlink_metadata(&path).unwrap();
            if meta.file_type().is_symlink() {
                let target = fs::read_link(&path).unwrap();
                out.insert(rel, target.to_string_lossy().into_owned().into_bytes());
            } else if meta.is_dir() {
                stack.push(path);
            } else {
                out.insert(rel, fs::read(&path).unwrap());
            }
        }
    }
    out
}

fn sample(seed: u64) -> (tempfile::TempDir, Corpus) {
    let dir = tempfile::tempdir().unwrap();
    let corpus = Corpus::generate(dir.path(), &CorpusSpec::new(seed, SAMPLE_FILES)).unwrap();
    (dir, corpus)
}

#[test]
fn generates_identical_trees_for_equal_seeds() {
    let (first_dir, first) = sample(7);
    let (second_dir, second) = sample(7);
    assert_eq!(first.entries, second.entries);
    assert_eq!(first.dirs, second.dirs);
    assert_eq!(snapshot(first_dir.path()), snapshot(second_dir.path()));
}

#[test]
fn generates_different_trees_for_different_seeds() {
    let (first_dir, _) = sample(7);
    let (second_dir, _) = sample(8);
    assert_ne!(snapshot(first_dir.path()), snapshot(second_dir.path()));
}

#[test]
fn manifest_matches_the_files_on_disk() {
    let (dir, corpus) = sample(11);
    let on_disk = snapshot(dir.path());
    let listed: Vec<&String> = corpus.entries.iter().map(|e| &e.path).collect();
    for path in &listed {
        assert!(on_disk.contains_key(*path), "missing {path}");
    }
    let regular: usize = on_disk.len()
        - corpus
            .dirs
            .iter()
            .filter(|d| on_disk.contains_key(*d))
            .count();
    assert_eq!(regular, corpus.entries.len());
    let mut sorted = corpus.entries.clone();
    sorted.sort_by(|a, b| a.path.cmp(&b.path));
    assert_eq!(sorted, corpus.entries);
    for entry in corpus
        .entries
        .iter()
        .filter(|e| !matches!(e.kind, Kind::FileLink | Kind::DirLink | Kind::DanglingLink))
    {
        assert_eq!(
            on_disk[&entry.path].len() as u64,
            entry.size,
            "{}",
            entry.path
        );
    }
}

#[test]
fn creates_the_requested_number_of_files_of_every_kind() {
    let (_dir, corpus) = sample(3);
    let generated = corpus
        .entries
        .iter()
        .filter(|e| !matches!(e.kind, Kind::GitIgnore | Kind::GitMarker))
        .count();
    assert_eq!(generated, SAMPLE_FILES);
    for kind in [
        Kind::Text,
        Kind::Binary,
        Kind::LongLine,
        Kind::Cp852,
        Kind::Cp1250,
        Kind::Utf16,
        Kind::Hidden,
        Kind::FileLink,
        Kind::DanglingLink,
        Kind::GitIgnore,
        Kind::GitMarker,
    ] {
        assert!(
            !corpus.of_kind(kind).is_empty(),
            "no entry of kind {kind:?}"
        );
    }
}

#[test]
fn includes_nfc_and_nfd_names() {
    let (_dir, corpus) = sample(5);
    let names: Vec<&str> = corpus
        .all_paths()
        .into_iter()
        .map(|p| p.rsplit('/').next().unwrap_or("").to_string())
        .map(|n| &*Box::leak(n.into_boxed_str()))
        .collect();
    assert!(
        names
            .iter()
            .any(|n| !n.is_ascii() && is_nfc(n) && !is_nfd(n))
    );
    assert!(
        names
            .iter()
            .any(|n| !n.is_ascii() && is_nfd(n) && !is_nfc(n))
    );
}

#[test]
fn nests_ignore_files_below_the_root() {
    let (_dir, corpus) = sample(5);
    let ignores = corpus.of_kind(Kind::GitIgnore);
    assert!(ignores.iter().any(|e| e.path == ".gitignore"));
    assert!(ignores.iter().any(|e| e.path.contains('/')));
}

#[test]
fn writes_binary_files_containing_nul() {
    let (dir, corpus) = sample(5);
    for entry in corpus.of_kind(Kind::Binary) {
        assert!(fs::read(dir.path().join(&entry.path)).unwrap().contains(&0));
    }
}

#[test]
fn writes_a_line_longer_than_sixty_four_kibibytes() {
    let (dir, corpus) = sample(5);
    let entry = corpus.of_kind(Kind::LongLine)[0];
    let text = fs::read_to_string(dir.path().join(&entry.path)).unwrap();
    assert_eq!(text.lines().next().map(str::len), Some(LONG_LINE_BYTES));
    assert!(text.contains(NEEDLE));
}

#[test]
fn writes_polish_text_in_legacy_and_wide_encodings() {
    let (dir, corpus) = sample(5);
    let expected = format!("{}\n", polish_sentence());
    let cp852 = fs::read(dir.path().join(&corpus.of_kind(Kind::Cp852)[0].path)).unwrap();
    assert_eq!(cp852, encode_cp852(&expected));
    assert!(std::str::from_utf8(&cp852).is_err());
    let cp1250 = fs::read(dir.path().join(&corpus.of_kind(Kind::Cp1250)[0].path)).unwrap();
    let (decoded, _, failed) = encoding_rs::WINDOWS_1250.decode(&cp1250);
    assert!(!failed);
    assert_eq!(decoded, expected);
    let utf16 = fs::read(dir.path().join(&corpus.of_kind(Kind::Utf16)[0].path)).unwrap();
    assert_eq!(utf16, encode_utf16le_marked(&expected));
    let (decoded, _, failed) = encoding_rs::UTF_16LE.decode(&utf16);
    assert!(!failed);
    assert_eq!(decoded, expected);
}

#[test]
fn encodes_cp852_with_the_documented_byte_values() {
    assert_eq!(
        encode_cp852("ąęłńóśźż"),
        [0xA5, 0xA9, 0x88, 0xE4, 0xA2, 0x98, 0xAB, 0xBE]
    );
    assert_eq!(encode_cp852("a€"), b"a?");
}

#[cfg(unix)]
#[test]
fn links_resolve_or_dangle_as_recorded() {
    let (dir, corpus) = sample(5);
    for entry in corpus.of_kind(Kind::FileLink) {
        assert!(
            fs::metadata(dir.path().join(&entry.path))
                .unwrap()
                .is_file()
        );
    }
    for entry in corpus.of_kind(Kind::DirLink) {
        assert!(fs::metadata(dir.path().join(&entry.path)).unwrap().is_dir());
    }
    for entry in corpus.of_kind(Kind::DanglingLink) {
        assert!(fs::metadata(dir.path().join(&entry.path)).is_err());
    }
}

#[test]
fn keeps_directory_depth_within_the_limit() {
    let dir = tempfile::tempdir().unwrap();
    let mut spec = CorpusSpec::new(9, 400);
    spec.max_depth = 2;
    let corpus = Corpus::generate(dir.path(), &spec).unwrap();
    assert!(corpus.dirs.iter().all(|d| d.split('/').count() <= 2));
}

#[test]
fn omits_the_git_marker_on_request() {
    let dir = tempfile::tempdir().unwrap();
    let mut spec = CorpusSpec::new(9, 50);
    spec.git_marker = false;
    let corpus = Corpus::generate(dir.path(), &spec).unwrap();
    assert!(corpus.of_kind(Kind::GitMarker).is_empty());
    assert!(!dir.path().join(".git").exists());
}

#[test]
fn large_corpus_reports_exact_needle_count() {
    let dir = tempfile::tempdir().unwrap();
    let large = generate_large(dir.path(), 3 * 1024 * 1024, 1).unwrap();
    assert!(large.bytes >= 3 * 1024 * 1024);
    let mut counted = 0u64;
    let mut size = 0u64;
    for file in &large.files {
        let text = fs::read_to_string(file).unwrap();
        size += text.len() as u64;
        counted += text.lines().filter(|l| l.contains(NEEDLE)).count() as u64;
    }
    assert_eq!(size, large.bytes);
    assert_eq!(counted, large.needle_lines);
}

#[test]
fn empty_tree_has_the_requested_entry_count() {
    let dir = tempfile::tempdir().unwrap();
    generate_empty_tree(dir.path(), 250, 100).unwrap();
    let files = snapshot(dir.path());
    assert_eq!(files.len(), 250);
    assert!(files.values().all(Vec::is_empty));
}

#[test]
fn replace_tree_lines_all_contain_the_needle() {
    let dir = tempfile::tempdir().unwrap();
    generate_replace_tree(dir.path(), 30, 512, 4).unwrap();
    let files = snapshot(dir.path());
    assert_eq!(files.len(), 30);
    for body in files.values() {
        let text = std::str::from_utf8(body).unwrap();
        assert!(text.len() >= 512);
        assert!(text.lines().all(|l| l.contains(NEEDLE)));
    }
}
