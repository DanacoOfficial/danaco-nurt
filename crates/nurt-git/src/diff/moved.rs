// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Detection of moved blocks.
//!
//! A moved block is a run of at least [`MOVED_MIN_LINES`] consecutive removed
//! lines that appears as a run of consecutive added lines in the same
//! difference, in the same file or in another one. Lines are compared with
//! all white space removed; a block of blank lines is not a move. Each added
//! line belongs to at most one block.

use std::collections::HashMap;

use bstr::BString;

use super::canonical::FileDiff;
use super::hunk::{LineKind, MovedBlock};
use super::lines::is_space;

/// Fewest lines of a moved block.
pub const MOVED_MIN_LINES: usize = 3;

/// A changed line with its place.
#[derive(Debug, Clone)]
struct Located {
    key: Vec<u8>,
    file: usize,
    hunk: usize,
    line: u32,
    run: usize,
}

/// Finds moved blocks and attaches each to the hunks that hold its removed
/// and its added lines.
pub fn mark_moved(files: &mut [FileDiff]) {
    let (removed, added) = collect(files);
    let mut by_key: HashMap<&[u8], Vec<usize>> = HashMap::new();
    for (index, line) in added.iter().enumerate() {
        by_key.entry(line.key.as_slice()).or_default().push(index);
    }
    let mut used = vec![false; added.len()];
    let mut index = 0;
    while index < removed.len() {
        let length = best_match(&removed, index, &added, &by_key, &used);
        match length {
            Some((start, length)) if length >= MOVED_MIN_LINES => {
                let block_lines = &removed[index..index + length];
                if block_lines.iter().all(|line| line.key.is_empty()) {
                    index += 1;
                    continue;
                }
                for slot in &mut used[start..start + length] {
                    *slot = true;
                }
                attach(files, &removed[index], &added[start], length);
                index += length;
            }
            _ => index += 1,
        }
    }
}

fn best_match(
    removed: &[Located],
    at: usize,
    added: &[Located],
    by_key: &HashMap<&[u8], Vec<usize>>,
    used: &[bool],
) -> Option<(usize, usize)> {
    let candidates = by_key.get(removed[at].key.as_slice())?;
    let mut best: Option<(usize, usize)> = None;
    for &start in candidates {
        let mut length = 0;
        while let (Some(from), Some(to)) = (removed.get(at + length), added.get(start + length)) {
            let continues = length == 0
                || (from.run == removed[at].run && to.run == added[start].run);
            if !continues || used[start + length] || from.key != to.key {
                break;
            }
            length += 1;
        }
        if best.is_none_or(|(_, found)| length > found) {
            best = Some((start, length));
        }
    }
    best
}

fn attach(files: &mut [FileDiff], from: &Located, to: &Located, length: usize) {
    let block = MovedBlock {
        from_path: block_path(files, from, true),
        from_line: from.line,
        to_path: block_path(files, to, false),
        to_line: to.line,
        lines: u32::try_from(length).unwrap_or(u32::MAX),
    };
    for place in [from, to] {
        if let Some(hunk) = files
            .get_mut(place.file)
            .and_then(|file| file.hunks.get_mut(place.hunk))
        {
            if !hunk.moved.contains(&block) {
                hunk.moved.push(block.clone());
            }
        }
    }
}

fn block_path(files: &[FileDiff], place: &Located, old_side: bool) -> BString {
    let pair = &files[place.file].pair;
    if old_side {
        pair.old_path.clone().unwrap_or_else(|| pair.path.clone())
    } else {
        pair.path.clone()
    }
}

fn collect(files: &[FileDiff]) -> (Vec<Located>, Vec<Located>) {
    let mut removed = Vec::new();
    let mut added = Vec::new();
    let mut run = 0;
    for (file_index, file) in files.iter().enumerate() {
        for (hunk_index, hunk) in file.hunks.iter().enumerate() {
            let mut old_line = if hunk.old_len == 0 { hunk.old_start + 1 } else { hunk.old_start };
            let mut new_line = if hunk.new_len == 0 { hunk.new_start + 1 } else { hunk.new_start };
            let mut previous = LineKind::Context;
            for line in &hunk.lines {
                if line.kind != previous && line.kind != LineKind::NoNewline {
                    run += 1;
                }
                let located = |number: u32, run: usize| Located {
                    key: line.text.iter().copied().filter(|&byte| !is_space(byte)).collect(),
                    file: file_index,
                    hunk: hunk_index,
                    line: number,
                    run,
                };
                match line.kind {
                    LineKind::Removed => {
                        removed.push(located(old_line, run));
                        old_line += 1;
                    }
                    LineKind::Added => {
                        added.push(located(new_line, run));
                        new_line += 1;
                    }
                    LineKind::Context => {
                        old_line += 1;
                        new_line += 1;
                    }
                    LineKind::NoNewline => {}
                }
                if line.kind != LineKind::NoNewline {
                    previous = line.kind;
                }
            }
        }
    }
    (removed, added)
}

/// The hunk consists only of lines of moved blocks.
#[must_use]
pub fn only_moved(hunk: &super::hunk::Hunk) -> bool {
    if hunk.moved.is_empty() {
        return false;
    }
    let mut old_line = if hunk.old_len == 0 { hunk.old_start + 1 } else { hunk.old_start };
    let mut new_line = if hunk.new_len == 0 { hunk.new_start + 1 } else { hunk.new_start };
    for line in &hunk.lines {
        match line.kind {
            LineKind::Removed => {
                let inside = hunk.moved.iter().any(|block| {
                    (block.from_line..block.from_line + block.lines).contains(&old_line) && block.from_path == hunk_old_path(hunk)
                });
                if !inside {
                    return false;
                }
                old_line += 1;
            }
            LineKind::Added => {
                let inside = hunk.moved.iter().any(|block| {
                    (block.to_line..block.to_line + block.lines).contains(&new_line) && block.to_path == hunk.path
                });
                if !inside {
                    return false;
                }
                new_line += 1;
            }
            LineKind::Context => {
                old_line += 1;
                new_line += 1;
            }
            LineKind::NoNewline => {}
        }
    }
    true
}

fn hunk_old_path(hunk: &super::hunk::Hunk) -> BString {
    hunk.old_path.clone().unwrap_or_else(|| hunk.path.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::canonical::{Driver, FileOptions, FilePair, diff_contents};
    use crate::diff::source::{Entry, MODE_FILE};

    fn file(path: &str, old: &str, new: &str) -> FileDiff {
        let side = Some(Entry { mode: MODE_FILE, id: None });
        let pair = FilePair {
            path: path.into(),
            old_path: None,
            old: side,
            new: side,
            similarity: None,
            copied: false,
        };
        diff_contents(pair, old.as_bytes(), new.as_bytes(), &FileOptions::default(), &Driver::default())
    }

    #[test]
    fn finds_block_moved_within_a_file() {
        let old = "const A = 1;\nconst B = 2;\nconst C = 3;\nconst D = 4;\nfn x() {}\nfn y() {}\nfn z() {}\nfn w() {}\nfn v() {}\nfn u() {}\nfn t() {}\n";
        let new = "fn x() {}\nfn y() {}\nfn z() {}\nfn w() {}\nfn v() {}\nfn u() {}\nfn t() {}\nconst A = 1;\nconst B = 2;\nconst C = 3;\nconst D = 4;\n";
        let mut files = vec![file("a.ts", old, new)];
        mark_moved(&mut files);
        let blocks: Vec<_> = files[0].hunks.iter().flat_map(|hunk| hunk.moved.clone()).collect();
        assert!(!blocks.is_empty());
        let block = &blocks[0];
        assert_eq!(block.lines, 4);
        assert_eq!((block.from_line, block.to_line), (1, 8));
        assert!(files[0].hunks.iter().all(only_moved));
    }

    #[test]
    fn finds_block_moved_between_files_ignoring_indentation() {
        let mut files = vec![
            file("a.txt", "keep\none\ntwo\nthree\n", "keep\n"),
            file("b.txt", "start\n", "start\n    one\n    two\n    three\n"),
        ];
        mark_moved(&mut files);
        let block = files[1].hunks[0].moved.first().cloned().unwrap();
        assert_eq!(block.from_path, "a.txt");
        assert_eq!(block.to_path, "b.txt");
        assert_eq!(block.lines, 3);
        assert_eq!(files[0].hunks[0].moved, vec![block]);
    }

    #[test]
    fn ignores_short_and_blank_blocks() {
        let mut files = vec![
            file("a.txt", "x\ny\n\n\n\nz\n", "z\nx\ny\n"),
        ];
        mark_moved(&mut files);
        assert!(files[0].hunks.iter().all(|hunk| hunk.moved.is_empty()));
        assert!(!only_moved(&files[0].hunks[0]));
    }
}
