// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-git::revwalk::moves
// Responsibility:   Lines moved within a file and copied between files, attributed to the commit that wrote them, as `git blame -M` and `-C` attribute them.
// Dependencies:     bstr, gix, nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Lines moved within a file and copied between files, attributed to the
//! commit that wrote them, as `git blame -M` and `-C` attribute them.

use std::ops::Range;

use bstr::{BString, ByteSlice};
use gix::ObjectId;
use nurt_contracts::error::Error;
use nurt_contracts::record::GitFileState;

use super::changes::{Detect, tree_changes};
use super::lines;
use super::paths::entry_at;
use super::repo::OpenRepo;

/// Alphanumeric characters that a block of moved lines needs to be
/// attributed to the commit it was moved from; the default of `git blame -M`.
const MOVE_SCORE: usize = 20;

/// Alphanumeric characters that a block of lines copied from another file
/// needs; the default of `git blame -C`.
const COPY_SCORE: usize = 40;

/// Deepest chain of commits followed through moved blocks.
const MAX_DEPTH: usize = 64;

/// A run of lines of the annotated file attributed to one commit.
#[derive(Debug, Clone)]
pub(crate) struct Piece {
    /// First line in the annotated file, from 0.
    pub(crate) blamed_start: u32,
    /// Number of lines.
    pub(crate) len: u32,
    /// Commit the lines are attributed to.
    pub(crate) commit: ObjectId,
    /// Path of the file in that commit.
    pub(crate) path: BString,
    /// First line in the file of that commit, from 0.
    pub(crate) source_start: u32,
}

/// Lines of a chunk that were found in a parent, with the pieces that wrote
/// them.
struct Found {
    matched: Range<usize>,
    pieces: Vec<Piece>,
}

/// A candidate source of lines in a parent commit.
struct Source {
    path: BString,
    threshold: usize,
}

/// Detection of moved and copied lines over the history read by
/// `gix_blame`.
pub(crate) struct Moves<'a> {
    repo: &'a OpenRepo,
    source: &'a gix::Repository,
    cache: &'a mut gix::diff::blob::Platform,
    algorithm: gix::diff::blob::Algorithm,
    copies: bool,
}

impl<'a> Moves<'a> {
    /// Detection with `source` as the repository whose history is read;
    /// `copies` adds files changed by the same commit as sources of lines.
    pub(crate) fn new(
        repo: &'a OpenRepo,
        source: &'a gix::Repository,
        cache: &'a mut gix::diff::blob::Platform,
        copies: bool,
    ) -> Self {
        Self {
            repo,
            source,
            cache,
            algorithm: source
                .diff_algorithm()
                .unwrap_or(gix::diff::blob::Algorithm::Myers),
            copies,
        }
    }

    /// Replaces the lines of `pieces` that a commit only moved or copied by
    /// the pieces of the commit that wrote them.
    pub(crate) fn refine(&mut self, pieces: Vec<Piece>, depth: usize) -> Result<Vec<Piece>, Error> {
        let mut out = Vec::with_capacity(pieces.len());
        for piece in pieces {
            if depth >= MAX_DEPTH || piece.commit.is_null() {
                out.push(piece);
            } else {
                out.extend(self.resolve(piece, depth)?);
            }
        }
        Ok(out)
    }

    fn resolve(&mut self, piece: Piece, depth: usize) -> Result<Vec<Piece>, Error> {
        let info = self
            .repo
            .commits
            .info(&self.repo.repo, piece.commit)
            .map_err(|err| {
                self.repo
                    .read_error(format_args!("commit {}", piece.commit), err)
            })?;
        if info.parents.is_empty() {
            return Ok(vec![piece]);
        }
        let Some(blob) = self.blob_at(info.tree, piece.path.as_ref())? else {
            return Ok(vec![piece]);
        };
        let all = lines::split_lines(&blob);
        let first = (piece.source_start as usize).min(all.len());
        let last = (first + piece.len as usize).min(all.len());
        let chunk = &all[first..last];
        let mut found: Vec<Piece> = Vec::new();
        let mut work: Vec<Range<usize>> = std::iter::once(0..chunk.len()).collect();
        while let Some(part) = work.pop() {
            match self.find(&piece, &info.parents, info.tree, chunk, &part, depth)? {
                Some(Found { matched, pieces }) => {
                    found.extend(pieces);
                    work.push(part.start..matched.start);
                    work.push(matched.end..part.end);
                }
                None => found.push(Piece {
                    blamed_start: piece.blamed_start + len32(part.start),
                    len: len32(part.len()),
                    commit: piece.commit,
                    path: piece.path.clone(),
                    source_start: piece.source_start + len32(part.start),
                }),
            }
        }
        found.retain(|piece| piece.len > 0);
        found.sort_by_key(|piece| piece.blamed_start);
        Ok(found)
    }

    fn find(
        &mut self,
        piece: &Piece,
        parents: &[ObjectId],
        tree: ObjectId,
        chunk: &[&[u8]],
        part: &Range<usize>,
        depth: usize,
    ) -> Result<Option<Found>, Error> {
        if part.is_empty() {
            return Ok(None);
        }
        let text: Vec<u8> = chunk[part.clone()].concat();
        for parent in parents {
            let parent_tree = self.repo.tree_of(*parent)?;
            for source in self.sources(parent_tree, tree, &piece.path)? {
                let Some(parent_blob) = self.blob_at(parent_tree, source.path.as_ref())? else {
                    continue;
                };
                let Some((chunk_at, parent_at, count)) =
                    best_block(&parent_blob, &text, part.start, source.threshold)
                else {
                    continue;
                };
                let blamed = self.blame_lines(*parent, &source.path, parent_at, count)?;
                let pieces = blamed
                    .into_iter()
                    .map(|found| Piece {
                        blamed_start: piece.blamed_start
                            + len32(chunk_at)
                            + (found.blamed_start - len32(parent_at)),
                        ..found
                    })
                    .collect();
                let pieces = self.refine(pieces, depth + 1)?;
                return Ok(Some(Found {
                    matched: chunk_at..chunk_at + count,
                    pieces,
                }));
            }
        }
        Ok(None)
    }

    /// Files of the parent that may hold lines of the file `path`: the file
    /// itself, and with copies the files that the commit changed.
    fn sources(
        &self,
        parent_tree: ObjectId,
        tree: ObjectId,
        path: &BString,
    ) -> Result<Vec<Source>, Error> {
        let mut sources = Vec::new();
        if self.blob_at(parent_tree, path.as_ref())?.is_some() {
            sources.push(Source {
                path: path.clone(),
                threshold: MOVE_SCORE,
            });
        } else if let Some(change) =
            tree_changes(self.repo, Some(parent_tree), tree, Detect::Renames, None)?
                .into_iter()
                .find(|change| change.path == *path && change.old_path.is_some())
            && let Some(origin) = change.old_path
        {
            sources.push(Source {
                path: origin,
                threshold: MOVE_SCORE,
            });
        }
        if self.copies {
            let changes = tree_changes(self.repo, Some(parent_tree), tree, Detect::Nothing, None)?;
            for change in changes {
                let readable = matches!(
                    change.status,
                    GitFileState::Modified | GitFileState::Deleted
                );
                if readable && change.path != *path {
                    sources.push(Source {
                        path: change.path,
                        threshold: COPY_SCORE,
                    });
                }
            }
        }
        Ok(sources)
    }

    fn blob_at(&self, tree: ObjectId, path: &bstr::BStr) -> Result<Option<Vec<u8>>, Error> {
        let entry = entry_at(self.repo, tree, path)
            .map_err(|err| self.repo.read_error("a tree of the history", err))?;
        match entry {
            Some((mode, id))
                if !mode.is_tree() && mode.kind() != gix::objs::tree::EntryKind::Commit =>
            {
                self.repo.blob(id).map(Some)
            }
            _ => Ok(None),
        }
    }

    /// The commits that wrote `count` lines of `path` from line `first`
    /// (from 0) of `commit`.
    fn blame_lines(
        &mut self,
        commit: ObjectId,
        path: &BString,
        first: usize,
        count: usize,
    ) -> Result<Vec<Piece>, Error> {
        let reading = |err: &dyn std::fmt::Display| {
            self.repo
                .read_error(format_args!("the history of `{}`", path.as_bstr()), err)
        };
        let ranges = gix_blame::BlameRanges::from_one_based_inclusive_range(
            len32(first + 1)..=len32(first + count),
        )
        .map_err(|err| reading(&err))?;
        let options = gix_blame::Options {
            diff_algorithm: self.algorithm,
            ranges,
            since: None,
            rewrites: Some(gix::diff::Rewrites::default()),
            debug_track_path: false,
        };
        let graph = self.source.commit_graph_if_enabled().ok().flatten();
        let outcome = gix_blame::file(
            &self.source.objects,
            gix_blame::Start::Commit(commit),
            graph,
            self.cache,
            path.as_bstr(),
            options,
        )
        .map_err(|err| reading(&err))?;
        Ok(outcome
            .entries
            .iter()
            .map(|entry| Piece {
                blamed_start: entry.start_in_blamed_file,
                len: entry.len.get(),
                commit: entry.commit_id,
                path: entry
                    .source_file_name
                    .clone()
                    .unwrap_or_else(|| path.clone()),
                source_start: entry.start_in_source_file,
            })
            .collect())
    }
}

/// The block of lines that `text` shares with `parent` and that holds the
/// most alphanumeric characters, when it holds at least `threshold`: the
/// first line of the block in `text` plus `offset`, its first line in
/// `parent` and its length.
fn best_block(
    parent: &[u8],
    text: &[u8],
    offset: usize,
    threshold: usize,
) -> Option<(usize, usize, usize)> {
    let text_lines = lines::split_lines(text);
    let mut best: Option<(usize, usize, usize, usize)> = None;
    let mut consider = |chunk_at: usize, parent_at: usize, count: usize| {
        let score: usize = text_lines[chunk_at..chunk_at + count]
            .iter()
            .map(|line| {
                line.iter()
                    .filter(|byte| byte.is_ascii_alphanumeric())
                    .count()
            })
            .sum();
        if count > 0 && score >= threshold && best.is_none_or(|(_, _, _, top)| score > top) {
            best = Some((chunk_at, parent_at, count, score));
        }
    };
    let (mut chunk_at, mut parent_at) = (0usize, 0usize);
    for edit in lines::edits(parent, text) {
        let after = edit.after.start as usize;
        consider(chunk_at, parent_at, after - chunk_at);
        chunk_at = edit.after.end as usize;
        parent_at = edit.before.end as usize;
    }
    consider(chunk_at, parent_at, text_lines.len() - chunk_at);
    best.map(|(chunk_at, parent_at, count, _)| (chunk_at + offset, parent_at, count))
}

fn len32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_block_with_the_most_alphanumeric_characters() {
        let parent = b"alpha\nbeta\ngamma\ndelta\n";
        let text = b"x\nalpha\nbeta\ny\ngamma\ndelta\nz\n";
        let found = best_block(parent, text, 10, 5);
        assert_eq!(found, Some((14, 2, 2)));
        assert_eq!(best_block(parent, text, 0, 100), None);
    }

    #[test]
    fn ignores_text_without_common_lines() {
        assert_eq!(best_block(b"one\n", b"two\n", 0, 1), None);
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
