// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Rename and copy detection.
//!
//! Deleted files are paired with added files. Identical content pairs first,
//! preferring the same file name. The remaining pairs are scored by the
//! share of content they have in common: content is cut into chunks that end
//! at a line feed or after [`CHUNK_MAX`] bytes (a carriage return before a
//! line feed is not counted in text), and the score is the number of bytes
//! of common chunks divided by the size of the larger file. Pairs at or
//! above the threshold are taken from the highest score down, each file at
//! most once. Empty files are never paired. With copy detection, modified
//! files are also sources and may be copied more than once.

use std::collections::HashMap;

use bstr::{BStr, BString, ByteSlice};
use nurt_contracts::error::Error;

use super::canonical::FilePair;
use super::source::{Entry, MODE_GITLINK, looks_binary};

/// Longest chunk compared by the similarity score, in bytes.
pub const CHUNK_MAX: usize = 64;

/// Similarity required by default, in percent.
pub const DEFAULT_THRESHOLD: u32 = 50;

/// Largest number of sources and destinations compared by content; above
/// it only identical content pairs.
pub const DEFAULT_LIMIT: usize = 1000;

/// Full score.
const FULL: u64 = 100;

/// Settings of the detection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenameOptions {
    /// Detect copies as well as renames.
    pub copies: bool,
    /// Similarity required, in percent.
    pub threshold: u32,
    /// Largest number of sources and of destinations scored by content.
    pub limit: usize,
}

impl Default for RenameOptions {
    fn default() -> Self {
        Self {
            copies: false,
            threshold: DEFAULT_THRESHOLD,
            limit: DEFAULT_LIMIT,
        }
    }
}

impl RenameOptions {
    /// Detection settings from a `diff.renames` or `status.renames` value;
    /// `None` when detection is off.
    #[must_use]
    pub fn from_config(value: Option<&str>) -> Option<Self> {
        let value = value.map(str::to_ascii_lowercase);
        match value.as_deref() {
            None | Some("true" | "yes" | "on" | "1" | "") => Some(Self::default()),
            Some("copy" | "copies") => Some(Self {
                copies: true,
                ..Self::default()
            }),
            _ => None,
        }
    }
}

/// Content loader of one side of a pair.
pub type Loader<'a> = dyn FnMut(&BStr, &Entry) -> Result<Vec<u8>, Error> + 'a;

struct Candidate {
    pair: usize,
    path: BString,
    entry: Entry,
    content: Vec<u8>,
}

/// Replaces deletion and addition pairs by renames and copies; the result
/// is sorted by path.
///
/// # Errors
///
/// Fails when the content of a candidate cannot be loaded.
pub fn detect_renames(pairs: Vec<FilePair>, options: &RenameOptions, load: &mut Loader<'_>) -> Result<Vec<FilePair>, Error> {
    let mut sources = Vec::new();
    let mut destinations = Vec::new();
    for (index, pair) in pairs.iter().enumerate() {
        match (&pair.old, &pair.new) {
            (Some(old), None) if eligible(old) => sources.push((index, true, *old)),
            (Some(old), Some(_)) if options.copies && eligible(old) => sources.push((index, false, *old)),
            (None, Some(new)) if eligible(new) => destinations.push((index, *new)),
            _ => {}
        }
    }
    if sources.is_empty() || destinations.is_empty() {
        return Ok(pairs);
    }
    let sources = load_all(&pairs, &sources, true, load)?;
    let destinations = load_all(
        &pairs,
        &destinations.iter().map(|(index, entry)| (*index, true, *entry)).collect::<Vec<_>>(),
        false,
        load,
    )?;
    let matches = pair_up(&sources, &destinations, options);
    Ok(apply_matches(pairs, &sources, &destinations, &matches))
}

fn eligible(entry: &Entry) -> bool {
    entry.mode != MODE_GITLINK
}

fn load_all(
    pairs: &[FilePair],
    list: &[(usize, bool, Entry)],
    old_side: bool,
    load: &mut Loader<'_>,
) -> Result<Vec<(Candidate, bool)>, Error> {
    let mut out = Vec::with_capacity(list.len());
    for (index, deleted, entry) in list {
        let pair = &pairs[*index];
        let path = if old_side {
            pair.old_path.clone().unwrap_or_else(|| pair.path.clone())
        } else {
            pair.path.clone()
        };
        let content = load(path.as_bstr(), entry)?;
        if content.is_empty() {
            continue;
        }
        out.push((
            Candidate {
                pair: *index,
                path,
                entry: *entry,
                content,
            },
            *deleted,
        ));
    }
    Ok(out)
}

/// A match: source index, destination index, score.
type Match = (usize, usize, u32);

fn pair_up(sources: &[(Candidate, bool)], destinations: &[(Candidate, bool)], options: &RenameOptions) -> Vec<Match> {
    let mut matches = Vec::new();
    let mut source_taken = vec![false; sources.len()];
    let mut destination_taken = vec![false; destinations.len()];
    for (destination_index, (destination, _)) in destinations.iter().enumerate() {
        let identical: Vec<usize> = sources
            .iter()
            .enumerate()
            .filter(|(index, (source, deleted))| {
                (!*deleted || !source_taken[*index]) && source.content == destination.content
            })
            .map(|(index, _)| index)
            .collect();
        let preferred = identical
            .iter()
            .copied()
            .find(|&index| base_name(&sources[index].0.path) == base_name(&destination.path))
            .or_else(|| identical.first().copied());
        if let Some(source_index) = preferred {
            source_taken[source_index] = sources[source_index].1;
            destination_taken[destination_index] = true;
            matches.push((source_index, destination_index, 100));
        }
    }
    let open_sources = source_taken.iter().filter(|taken| !**taken).count();
    let open_destinations = destination_taken.iter().filter(|taken| !**taken).count();
    if open_sources > options.limit || open_destinations > options.limit {
        return matches;
    }
    let mut scored = Vec::new();
    for (destination_index, (destination, _)) in destinations.iter().enumerate() {
        if destination_taken[destination_index] {
            continue;
        }
        let destination_chunks = chunk_keys(&destination.content);
        for (source_index, (source, _)) in sources.iter().enumerate() {
            if source_taken[source_index] || !compatible(&source.entry, &destination.entry) {
                continue;
            }
            let score = similarity(&source.content, &destination.content, &destination_chunks);
            if score >= options.threshold {
                scored.push((source_index, destination_index, score));
            }
        }
    }
    scored.sort_by(|a, b| b.2.cmp(&a.2).then(a.1.cmp(&b.1)).then(a.0.cmp(&b.0)));
    for (source_index, destination_index, score) in scored {
        if destination_taken[destination_index] || source_taken[source_index] {
            continue;
        }
        destination_taken[destination_index] = true;
        source_taken[source_index] = sources[source_index].1;
        matches.push((source_index, destination_index, score));
    }
    matches
}

fn compatible(source: &Entry, destination: &Entry) -> bool {
    source.is_link() == destination.is_link()
}

fn base_name(path: &BString) -> &[u8] {
    path.rsplit_str("/").next().unwrap_or(path.as_slice())
}

fn chunk_keys(data: &[u8]) -> HashMap<&[u8], u64> {
    let text = !looks_binary(data);
    let mut counts: HashMap<&[u8], u64> = HashMap::new();
    let mut start = 0;
    while start < data.len() {
        let rest = &data[start..];
        let end = rest
            .iter()
            .take(CHUNK_MAX)
            .position(|&byte| byte == b'\n')
            .map_or(rest.len().min(CHUNK_MAX), |index| index + 1);
        let mut chunk = &rest[..end];
        let mut weight = chunk.len() as u64;
        if text && chunk.ends_with(b"\r\n") {
            chunk = &chunk[..chunk.len() - 2];
            weight -= 1;
        }
        *counts.entry(chunk).or_default() += weight;
        start += end;
    }
    counts
}

/// Similarity of two contents in percent.
fn similarity(source: &[u8], destination: &[u8], destination_chunks: &HashMap<&[u8], u64>) -> u32 {
    let largest = source.len().max(destination.len()) as u64;
    if largest == 0 {
        return 0;
    }
    let source_chunks = chunk_keys(source);
    let common: u64 = source_chunks
        .iter()
        .map(|(chunk, weight)| destination_chunks.get(chunk).map_or(0, |other| (*weight).min(*other)))
        .sum();
    u32::try_from(common * FULL / largest).unwrap_or(0)
}

fn apply_matches(
    pairs: Vec<FilePair>,
    sources: &[(Candidate, bool)],
    destinations: &[(Candidate, bool)],
    matches: &[Match],
) -> Vec<FilePair> {
    let mut remove = vec![false; pairs.len()];
    let mut joined = Vec::new();
    for &(source_index, destination_index, score) in matches {
        let (source, deleted) = &sources[source_index];
        let (destination, _) = &destinations[destination_index];
        let renamed = *deleted && !joined.iter().any(|(pair, _): &(usize, FilePair)| *pair == source.pair);
        if renamed {
            remove[source.pair] = true;
        }
        remove[destination.pair] = true;
        joined.push((
            source.pair,
            FilePair {
                path: destination.path.clone(),
                old_path: Some(source.path.clone()),
                old: Some(source.entry),
                new: Some(destination.entry),
                similarity: Some(score),
                copied: !renamed,
            },
        ));
    }
    let mut out: Vec<FilePair> = pairs
        .into_iter()
        .enumerate()
        .filter(|(index, _)| !remove[*index])
        .map(|(_, pair)| pair)
        .chain(joined.into_iter().map(|(_, pair)| pair))
        .collect();
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::source::{MODE_FILE, MODE_LINK};

    fn side() -> Option<Entry> {
        Some(Entry { mode: MODE_FILE, id: None })
    }

    fn pair(path: &str, old: bool, new: bool) -> FilePair {
        FilePair {
            path: path.into(),
            old_path: None,
            old: if old { side() } else { None },
            new: if new { side() } else { None },
            similarity: None,
            copied: false,
        }
    }

    fn run(pairs: Vec<FilePair>, contents: &[(&str, &str)], options: &RenameOptions) -> Vec<FilePair> {
        let mut load = |path: &BStr, _entry: &Entry| -> Result<Vec<u8>, Error> {
            Ok(contents
                .iter()
                .find(|(name, _)| path == name.as_bytes())
                .map(|(_, content)| content.as_bytes().to_vec())
                .unwrap_or_default())
        };
        detect_renames(pairs, options, &mut load).unwrap()
    }

    #[test]
    fn pairs_identical_content_as_full_rename() {
        let pairs = vec![pair("old.txt", true, false), pair("new.txt", false, true)];
        let found = run(pairs, &[("old.txt", "same\n"), ("new.txt", "same\n")], &RenameOptions::default());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].path, "new.txt");
        assert_eq!(found[0].old_path.as_deref(), Some(b"old.txt".as_bstr()));
        assert_eq!(found[0].similarity, Some(100));
        assert!(!found[0].copied);
    }

    #[test]
    fn pairs_similar_content_above_threshold() {
        let old = "line one\nline two\nline three\nline four\n";
        let new = "line one\nline two\nline three\nline 4\n";
        let pairs = vec![pair("a.txt", true, false), pair("b.txt", false, true)];
        let found = run(pairs, &[("a.txt", old), ("b.txt", new)], &RenameOptions::default());
        assert_eq!(found.len(), 1);
        let score = found[0].similarity.unwrap();
        assert!((50..100).contains(&score), "{score}");
    }

    #[test]
    fn keeps_dissimilar_files_apart() {
        let pairs = vec![pair("a.txt", true, false), pair("b.txt", false, true)];
        let found = run(pairs, &[("a.txt", "alpha\nbeta\n"), ("b.txt", "gamma\ndelta\n")], &RenameOptions::default());
        assert_eq!(found.len(), 2);
        assert!(found.iter().all(|pair| pair.old_path.is_none()));
    }

    #[test]
    fn prefers_same_file_name_for_identical_content() {
        let pairs = vec![
            pair("x/a.txt", true, false),
            pair("x/b.txt", true, false),
            pair("y/b.txt", false, true),
        ];
        let found = run(
            pairs,
            &[("x/a.txt", "same\n"), ("x/b.txt", "same\n"), ("y/b.txt", "same\n")],
            &RenameOptions::default(),
        );
        let renamed = found.iter().find(|pair| pair.path == "y/b.txt").unwrap();
        assert_eq!(renamed.old_path.as_deref(), Some(b"x/b.txt".as_bstr()));
    }

    #[test]
    fn detects_copies_of_modified_files() {
        let options = RenameOptions::from_config(Some("copies")).unwrap();
        let pairs = vec![pair("a.txt", true, true), pair("c.txt", false, true)];
        let found = run(pairs, &[("a.txt", "shared text\nmore\n"), ("c.txt", "shared text\nmore\n")], &options);
        let copy = found.iter().find(|pair| pair.path == "c.txt").unwrap();
        assert!(copy.copied);
        assert_eq!(copy.old_path.as_deref(), Some(b"a.txt".as_bstr()));
        assert!(found.iter().any(|pair| pair.path == "a.txt" && pair.old_path.is_none()));
    }

    #[test]
    fn ignores_empty_files_and_mismatched_types() {
        let pairs = vec![pair("a", true, false), pair("b", false, true)];
        assert_eq!(run(pairs, &[], &RenameOptions::default()).len(), 2);
        let mut link = pair("l", false, true);
        link.new = Some(Entry { mode: MODE_LINK, id: None });
        let pairs = vec![pair("f", true, false), link];
        let found = run(pairs, &[("f", "target text\n"), ("l", "target text\n")], &RenameOptions::default());
        assert_eq!(found.len(), 1);
    }

    #[test]
    fn reads_configuration_values() {
        assert!(RenameOptions::from_config(None).is_some());
        assert!(RenameOptions::from_config(Some("false")).is_none());
        assert!(RenameOptions::from_config(Some("true")).is_some_and(|options| !options.copies));
    }
}
