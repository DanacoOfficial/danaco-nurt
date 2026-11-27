// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-files/tests/support_corpus/mod.rs
// Responsibility:   Deterministic corpus generator for differential, parity and performance tests.
// Dependencies:     unicode_normalization
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Deterministic corpus generator for differential, parity and performance tests.
//!
//! A corpus is a directory tree whose content depends only on the seed and the
//! specification. The generator never touches the network and writes only below
//! the root it receives.

// Each test target links a different subset of the generator.
#![allow(dead_code)]

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use unicode_normalization::UnicodeNormalization;

/// Token present in a fraction of the generated text lines.
pub(crate) const NEEDLE: &str = "nurt_needle";

/// Length in bytes of the line written to `LongLine` files.
pub(crate) const LONG_LINE_BYTES: usize = 70_000;

const TEXT_WEIGHT: u64 = 60;
const BINARY_WEIGHT: u64 = 8;
const LONG_LINE_WEIGHT: u64 = 4;
const CP852_WEIGHT: u64 = 4;
const CP1250_WEIGHT: u64 = 4;
const UTF16_WEIGHT: u64 = 4;
const HIDDEN_WEIGHT: u64 = 8;
const SYMLINK_WEIGHT: u64 = 4;
const DANGLING_WEIGHT: u64 = 2;
const WEIGHT_TOTAL: u64 = TEXT_WEIGHT
    + BINARY_WEIGHT
    + LONG_LINE_WEIGHT
    + CP852_WEIGHT
    + CP1250_WEIGHT
    + UTF16_WEIGHT
    + HIDDEN_WEIGHT
    + SYMLINK_WEIGHT
    + DANGLING_WEIGHT;

const FILES_PER_DIRECTORY: usize = 20;
const NEEDLE_LINE_ODDS: u64 = 10;
const GITIGNORE_ODDS: u64 = 8;
const LARGE_BLOCK_BYTES: usize = 256 * 1024;
const LARGE_FILE_BYTES: u64 = 8 * 1024 * 1024;
const LARGE_NEEDLE_PERIOD: u64 = 2_000;
const DIR_LINK_ODDS: u64 = 4;

const WORDS: [&str; 16] = [
    "alpha", "bravo", "report", "index", "stream", "buffer", "token", "module", "driver", "cache",
    "ledger", "vector", "kernel", "socket", "parser", "schema",
];
const POLISH_WORDS: [&str; 8] = [
    "za\u{17c}\u{f3}\u{142}\u{107}",
    "g\u{119}\u{15b}l\u{105}",
    "ja\u{17a}\u{144}",
    "\u{142}\u{f3}d\u{17a}",
    "\u{15b}\u{107}ma",
    "\u{17a}d\u{17a}b\u{142}o",
    "w\u{105}\u{17c}",
    "d\u{17a}wi\u{119}k",
];
const POLISH_SENTENCE: &str = "Za\u{17c}\u{f3}\u{142}\u{107} g\u{119}\u{15b}l\u{105} ja\u{17a}\u{144}, \u{142}\u{f3}d\u{17a} \u{15b}\u{107}ma w\u{105}\u{17c}.";
const GITIGNORE_BODIES: [&str; 3] = [
    "*.log\nbuild/\n",
    "*.tmp\n!keep.tmp\n/local/\n",
    "# generated\n*.dat\n",
];

/// Parameters of a generated corpus.
#[derive(Clone, Debug)]
pub(crate) struct CorpusSpec {
    /// Seed of the pseudo-random generator.
    pub(crate) seed: u64,
    /// Number of regular files, symbolic links excluded from the count of kinds below.
    pub(crate) files: usize,
    /// Greatest directory depth below the root.
    pub(crate) max_depth: usize,
    /// Whether a `.git` directory is created at the root.
    pub(crate) git_marker: bool,
}

impl CorpusSpec {
    /// Specification with the default depth and a git marker.
    pub(crate) fn new(seed: u64, files: usize) -> Self {
        Self {
            seed,
            files,
            max_depth: 5,
            git_marker: true,
        }
    }
}

/// Kind of a generated entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    /// UTF-8 text with ASCII and Polish words.
    Text,
    /// Random bytes containing NUL.
    Binary,
    /// UTF-8 text with a line of `LONG_LINE_BYTES` bytes.
    LongLine,
    /// Polish text in code page 852.
    Cp852,
    /// Polish text in code page 1250.
    Cp1250,
    /// Polish text in UTF-16LE with a byte order mark.
    Utf16,
    /// UTF-8 text in a hidden file.
    Hidden,
    /// Symbolic link to a text file of the corpus.
    FileLink,
    /// Symbolic link to a directory of the corpus.
    DirLink,
    /// Symbolic link whose target does not exist.
    DanglingLink,
    /// `.gitignore` file.
    GitIgnore,
    /// Content of the `.git` marker.
    GitMarker,
}

/// One generated entry, other than a directory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Entry {
    /// Path relative to the root, separated by `/`.
    pub(crate) path: String,
    /// Kind of the entry.
    pub(crate) kind: Kind,
    /// Size in bytes; zero for symbolic links.
    pub(crate) size: u64,
}

/// Generated tree with its manifest.
#[derive(Debug)]
pub(crate) struct Corpus {
    /// Root directory of the tree.
    pub(crate) root: PathBuf,
    /// Entries sorted by path.
    pub(crate) entries: Vec<Entry>,
    /// Directories relative to the root, sorted, root excluded.
    pub(crate) dirs: Vec<String>,
}

impl Corpus {
    /// Writes the corpus described by `spec` below `root`.
    ///
    /// # Errors
    ///
    /// Returns the first I/O error raised while creating directories or files.
    pub(crate) fn generate(root: &Path, spec: &CorpusSpec) -> io::Result<Self> {
        let mut rng = Rng::new(spec.seed);
        fs::create_dir_all(root)?;
        let mut entries = Vec::new();
        if spec.git_marker {
            let git = root.join(".git");
            fs::create_dir_all(&git)?;
            let head = b"ref: refs/heads/main\n";
            fs::write(git.join("HEAD"), head)?;
            entries.push(Entry {
                path: ".git/HEAD".into(),
                kind: Kind::GitMarker,
                size: head.len() as u64,
            });
        }
        let dirs = create_directories(root, spec, &mut rng)?;
        write_gitignores(root, &dirs, &mut rng, &mut entries)?;
        let mut text_files: Vec<String> = Vec::new();
        for index in 0..spec.files {
            let dir = &dirs[rng.below(dirs.len() as u64) as usize];
            let kind = pick_kind(&mut rng);
            let entry = write_entry(root, dir, index, kind, &dirs, &text_files, &mut rng)?;
            if matches!(entry.kind, Kind::Text | Kind::Hidden) {
                text_files.push(entry.path.clone());
            }
            entries.push(entry);
        }
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        let mut dir_paths: Vec<String> = dirs.into_iter().filter(|d| !d.is_empty()).collect();
        dir_paths.sort();
        Ok(Self {
            root: root.to_path_buf(),
            entries,
            dirs: dir_paths,
        })
    }

    /// Entries of the given kind, in manifest order.
    pub(crate) fn of_kind(&self, kind: Kind) -> Vec<&Entry> {
        self.entries.iter().filter(|e| e.kind == kind).collect()
    }

    /// Relative paths of all entries and directories, sorted.
    pub(crate) fn all_paths(&self) -> Vec<String> {
        let mut all: Vec<String> = self
            .entries
            .iter()
            .map(|e| e.path.clone())
            .chain(self.dirs.iter().cloned())
            .collect();
        all.sort();
        all
    }
}

/// Large text corpus used by performance measurements.
#[derive(Debug)]
pub(crate) struct LargeCorpus {
    /// Files of the corpus.
    pub(crate) files: Vec<PathBuf>,
    /// Total size in bytes.
    pub(crate) bytes: u64,
    /// Number of lines containing `NEEDLE`.
    pub(crate) needle_lines: u64,
}

/// Writes at least `total_bytes` of text in files of about 8 MiB below `root`.
///
/// Files consist of whole repetitions of one generated block, so the number of
/// lines containing `NEEDLE` is known exactly.
///
/// # Errors
///
/// Returns the first I/O error raised while writing.
pub(crate) fn generate_large(root: &Path, total_bytes: u64, seed: u64) -> io::Result<LargeCorpus> {
    fs::create_dir_all(root)?;
    let mut files = Vec::new();
    let mut written = 0u64;
    let mut needle_lines = 0u64;
    let mut index = 0u64;
    while written < total_bytes {
        let mut rng = Rng::new(seed ^ index.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let (block, block_needles) = large_block(&mut rng);
        let repeats = (LARGE_FILE_BYTES / block.len() as u64).max(1);
        let path = root.join(format!("part_{index:05}.txt"));
        let mut out = io::BufWriter::new(fs::File::create(&path)?);
        for _ in 0..repeats {
            out.write_all(&block)?;
        }
        out.flush()?;
        written += repeats * block.len() as u64;
        needle_lines += repeats * block_needles;
        files.push(path);
        index += 1;
    }
    Ok(LargeCorpus {
        files,
        bytes: written,
        needle_lines,
    })
}

/// Writes `entries` empty files spread over directories with `fanout` children each.
///
/// # Errors
///
/// Returns the first I/O error raised while writing.
pub(crate) fn generate_empty_tree(root: &Path, entries: u64, fanout: u64) -> io::Result<()> {
    fs::create_dir_all(root)?;
    let mut dir = root.to_path_buf();
    for index in 0..entries {
        if index % fanout == 0 {
            dir = root.join(format!("d{:06}", index / fanout));
            fs::create_dir_all(&dir)?;
        }
        fs::File::create(dir.join(format!("f{index:07}.txt")))?;
    }
    Ok(())
}

/// Writes `files` text files of `bytes` bytes each, every line containing `NEEDLE`.
///
/// # Errors
///
/// Returns the first I/O error raised while writing.
pub(crate) fn generate_replace_tree(
    root: &Path,
    files: u64,
    bytes: usize,
    seed: u64,
) -> io::Result<()> {
    fs::create_dir_all(root)?;
    let mut rng = Rng::new(seed);
    for index in 0..files {
        let dir = root.join(format!("g{:04}", index / 100));
        fs::create_dir_all(&dir)?;
        let mut body = String::with_capacity(bytes + 64);
        while body.len() < bytes {
            body.push_str(&format!("{} {NEEDLE} {}\n", word(&mut rng), word(&mut rng)));
        }
        fs::write(dir.join(format!("r{index:05}.txt")), body)?;
    }
    Ok(())
}

/// Encodes text in code page 852; characters outside the Polish set and ASCII become `?`.
pub(crate) fn encode_cp852(text: &str) -> Vec<u8> {
    const MAP: [(char, u8); 18] = [
        ('\u{105}', 0xA5),
        ('\u{104}', 0xA4),
        ('\u{107}', 0x86),
        ('\u{106}', 0x8F),
        ('\u{119}', 0xA9),
        ('\u{118}', 0xA8),
        ('\u{142}', 0x88),
        ('\u{141}', 0x9D),
        ('\u{144}', 0xE4),
        ('\u{143}', 0xE3),
        ('\u{f3}', 0xA2),
        ('\u{d3}', 0xE0),
        ('\u{15b}', 0x98),
        ('\u{15a}', 0x97),
        ('\u{17a}', 0xAB),
        ('\u{179}', 0x8D),
        ('\u{17c}', 0xBE),
        ('\u{17b}', 0xBD),
    ];
    text.chars()
        .map(|c| {
            if c.is_ascii() {
                c as u8
            } else {
                MAP.iter()
                    .find(|(k, _)| *k == c)
                    .map_or(b'?', |(_, byte)| *byte)
            }
        })
        .collect()
}

/// Encodes text as UTF-16LE preceded by a byte order mark.
pub(crate) fn encode_utf16le_marked(text: &str) -> Vec<u8> {
    let mut out = vec![0xFF, 0xFE];
    for unit in text.encode_utf16() {
        out.extend_from_slice(&unit.to_le_bytes());
    }
    out
}

/// Text of the Polish sentence written to the code page and UTF-16 files.
pub(crate) fn polish_sentence() -> &'static str {
    POLISH_SENTENCE
}

/// SplitMix64 generator; stable across platforms and releases.
#[derive(Clone, Debug)]
pub(crate) struct Rng {
    state: u64,
}

impl Rng {
    /// Creates a generator from `seed`.
    pub(crate) fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    /// Next 64-bit value.
    pub(crate) fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Value in `0..bound`; `bound` must be positive.
    pub(crate) fn below(&mut self, bound: u64) -> u64 {
        self.next_u64() % bound
    }
}

fn word(rng: &mut Rng) -> &'static str {
    if rng.below(5) == 0 {
        POLISH_WORDS[rng.below(POLISH_WORDS.len() as u64) as usize]
    } else {
        WORDS[rng.below(WORDS.len() as u64) as usize]
    }
}

fn normalized(rng: &mut Rng, name: &str) -> String {
    match rng.below(3) {
        0 => name.nfc().collect(),
        1 => name.nfd().collect(),
        _ => name.to_string(),
    }
}

fn create_directories(root: &Path, spec: &CorpusSpec, rng: &mut Rng) -> io::Result<Vec<String>> {
    let count = (spec.files / FILES_PER_DIRECTORY).max(1);
    let mut dirs: Vec<String> = vec![String::new()];
    let mut depths: Vec<usize> = vec![0];
    for index in 0..count {
        let parent = rng.below(dirs.len() as u64) as usize;
        if depths[parent] >= spec.max_depth {
            continue;
        }
        let base = match rng.below(10) {
            0 => format!(".hidden_{index}"),
            1 => normalized(rng, &format!("za\u{17c}\u{f3}\u{142}\u{107}_{index}")),
            2 => format!("dir with space {index}"),
            _ => format!("{}_{index}", WORDS[rng.below(WORDS.len() as u64) as usize]),
        };
        let path = if dirs[parent].is_empty() {
            base
        } else {
            format!("{}/{base}", dirs[parent])
        };
        fs::create_dir_all(root.join(&path))?;
        depths.push(depths[parent] + 1);
        dirs.push(path);
    }
    Ok(dirs)
}

fn write_gitignores(
    root: &Path,
    dirs: &[String],
    rng: &mut Rng,
    entries: &mut Vec<Entry>,
) -> io::Result<()> {
    for (position, dir) in dirs.iter().enumerate() {
        if position != 0 && rng.below(GITIGNORE_ODDS) != 0 {
            continue;
        }
        let body = GITIGNORE_BODIES[rng.below(GITIGNORE_BODIES.len() as u64) as usize];
        let rel = join(dir, ".gitignore");
        fs::write(root.join(&rel), body)?;
        entries.push(Entry {
            path: rel,
            kind: Kind::GitIgnore,
            size: body.len() as u64,
        });
    }
    Ok(())
}

fn join(dir: &str, name: &str) -> String {
    if dir.is_empty() {
        name.to_string()
    } else {
        format!("{dir}/{name}")
    }
}

fn pick_kind(rng: &mut Rng) -> Kind {
    let mut roll = rng.below(WEIGHT_TOTAL);
    let table = [
        (TEXT_WEIGHT, Kind::Text),
        (BINARY_WEIGHT, Kind::Binary),
        (LONG_LINE_WEIGHT, Kind::LongLine),
        (CP852_WEIGHT, Kind::Cp852),
        (CP1250_WEIGHT, Kind::Cp1250),
        (UTF16_WEIGHT, Kind::Utf16),
        (HIDDEN_WEIGHT, Kind::Hidden),
        (SYMLINK_WEIGHT, Kind::FileLink),
        (DANGLING_WEIGHT, Kind::DanglingLink),
    ];
    for (weight, kind) in table {
        if roll < weight {
            return kind;
        }
        roll -= weight;
    }
    Kind::Text
}

fn text_body(rng: &mut Rng) -> String {
    let lines = 5 + rng.below(36);
    let mut body = String::new();
    for _ in 0..lines {
        let words = 3 + rng.below(8);
        let mut line: Vec<&str> = (0..words).map(|_| word(rng)).collect();
        if rng.below(NEEDLE_LINE_ODDS) == 0 {
            line.push(NEEDLE);
        }
        body.push_str(&line.join(" "));
        body.push('\n');
    }
    body
}

fn file_name(rng: &mut Rng, index: usize, extension: &str) -> String {
    let stem = if rng.below(6) == 0 {
        let polish = POLISH_WORDS[rng.below(POLISH_WORDS.len() as u64) as usize];
        normalized(rng, &format!("{polish}_{index}"))
    } else {
        format!("{}_{index}", WORDS[rng.below(WORDS.len() as u64) as usize])
    };
    format!("{stem}.{extension}")
}

fn write_entry(
    root: &Path,
    dir: &str,
    index: usize,
    kind: Kind,
    dirs: &[String],
    text_files: &[String],
    rng: &mut Rng,
) -> io::Result<Entry> {
    let (name, bytes): (String, Vec<u8>) = match kind {
        Kind::Text => (file_name(rng, index, "txt"), text_body(rng).into_bytes()),
        Kind::Hidden => (
            format!(".{}", file_name(rng, index, "env")),
            text_body(rng).into_bytes(),
        ),
        Kind::Binary => {
            let count = 64 + rng.below(4096) as usize;
            let mut data: Vec<u8> = (0..count).map(|_| rng.next_u64() as u8).collect();
            data[0] = 0;
            (file_name(rng, index, "bin"), data)
        }
        Kind::LongLine => {
            let mut body = "x".repeat(LONG_LINE_BYTES);
            body.push('\n');
            body.push_str(&format!("{NEEDLE} after the long line\n"));
            (file_name(rng, index, "log"), body.into_bytes())
        }
        Kind::Cp852 => (
            file_name(rng, index, "cp852"),
            encode_cp852(&format!("{POLISH_SENTENCE}\n")),
        ),
        Kind::Cp1250 => {
            let text = format!("{POLISH_SENTENCE}\n");
            let (bytes, _, _) = encoding_rs::WINDOWS_1250.encode(&text);
            (file_name(rng, index, "cp1250"), bytes.into_owned())
        }
        Kind::Utf16 => (
            file_name(rng, index, "u16"),
            encode_utf16le_marked(&format!("{POLISH_SENTENCE}\n")),
        ),
        Kind::FileLink | Kind::DirLink | Kind::DanglingLink => {
            return write_link(root, dir, index, kind, dirs, text_files, rng);
        }
        Kind::GitIgnore | Kind::GitMarker => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "ignore files and the git marker are written separately",
            ));
        }
    };
    let rel = join(dir, &name);
    fs::write(root.join(&rel), &bytes)?;
    Ok(Entry {
        path: rel,
        kind,
        size: bytes.len() as u64,
    })
}

fn write_link(
    root: &Path,
    dir: &str,
    index: usize,
    kind: Kind,
    dirs: &[String],
    text_files: &[String],
    rng: &mut Rng,
) -> io::Result<Entry> {
    let rel = join(dir, &format!("link_{index}"));
    let missing = || (format!("missing_target_{index}"), Kind::DanglingLink);
    let (target, effective) = if kind == Kind::DanglingLink {
        missing()
    } else if rng.below(DIR_LINK_ODDS) == 0 {
        match pick_link_dir(dir, dirs, rng) {
            Some(picked) => (relative_target(dir, &picked), Kind::DirLink),
            None => missing(),
        }
    } else if text_files.is_empty() {
        missing()
    } else {
        let picked = &text_files[rng.below(text_files.len() as u64) as usize];
        (relative_target(dir, picked), Kind::FileLink)
    };
    make_symlink(&target, &root.join(&rel), effective == Kind::DirLink)?;
    Ok(Entry {
        path: rel,
        kind: effective,
        size: 0,
    })
}

fn pick_link_dir(dir: &str, dirs: &[String], rng: &mut Rng) -> Option<String> {
    let related = |a: &str, b: &str| {
        a == b || a.starts_with(&format!("{b}/")) || b.starts_with(&format!("{a}/"))
    };
    let candidates: Vec<&String> = dirs
        .iter()
        .filter(|d| !d.is_empty() && (dir.is_empty() || !related(dir, d)))
        .collect();
    if candidates.is_empty() {
        return None;
    }
    Some(candidates[rng.below(candidates.len() as u64) as usize].clone())
}

fn relative_target(from_dir: &str, target: &str) -> String {
    let depth = if from_dir.is_empty() {
        0
    } else {
        from_dir.split('/').count()
    };
    format!("{}{target}", "../".repeat(depth))
}

#[cfg(unix)]
fn make_symlink(target: &str, link: &Path, _directory: bool) -> io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(windows)]
fn make_symlink(target: &str, link: &Path, directory: bool) -> io::Result<()> {
    if directory {
        std::os::windows::fs::symlink_dir(target, link)
    } else {
        std::os::windows::fs::symlink_file(target, link)
    }
}

fn large_block(rng: &mut Rng) -> (Vec<u8>, u64) {
    let mut block = String::with_capacity(LARGE_BLOCK_BYTES + 128);
    let mut needles = 0u64;
    let mut line = 0u64;
    while block.len() < LARGE_BLOCK_BYTES {
        let words = 4 + rng.below(8);
        let mut parts: Vec<&str> = (0..words).map(|_| word(rng)).collect();
        if line % LARGE_NEEDLE_PERIOD == LARGE_NEEDLE_PERIOD - 1 {
            parts.push(NEEDLE);
            needles += 1;
        }
        block.push_str(&parts.join(" "));
        block.push('\n');
        line += 1;
    }
    (block.into_bytes(), needles)
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
