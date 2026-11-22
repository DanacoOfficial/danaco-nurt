// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Reading of trees and commits and writing of replayed commits.

use std::collections::BTreeSet;

use bstr::{BStr, BString, ByteSlice};
use gix::ObjectId;
use gix::prelude::ObjectIdExt;
use gix_object::tree::EntryMode;
use nurt_contracts::error::Result;

use super::error;

/// A leaf of a tree: a blob, a symbolic link or a submodule commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Leaf {
    /// Mode of the entry.
    pub(super) mode: EntryMode,
    /// Object of the entry.
    pub(super) id: ObjectId,
}

/// Change of one leaf between two trees.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct LeafChange {
    /// Path relative to the root.
    pub(super) path: BString,
    /// Leaf in the old tree.
    pub(super) old: Option<Leaf>,
    /// Leaf in the new tree.
    pub(super) new: Option<Leaf>,
}

/// One entry of a tree.
struct Entry {
    name: BString,
    mode: EntryMode,
    id: ObjectId,
}

/// Commit fields used by the sequencer.
#[derive(Debug, Clone)]
pub(super) struct CommitInfo {
    /// Root tree.
    pub(super) tree: ObjectId,
    /// Parents in order.
    pub(super) parents: Vec<ObjectId>,
    /// Raw value of the `author` header.
    pub(super) author: BString,
    /// Raw value of the `encoding` header.
    pub(super) encoding: Option<BString>,
    /// Message after the headers, byte for byte.
    pub(super) message: BString,
}

impl CommitInfo {
    /// First line of the message, as used in reflog messages.
    pub(super) fn subject(&self) -> &BStr {
        first_line(self.message.as_bstr())
    }

    /// First paragraph of the message joined into one line, as shown in
    /// the pick list and in conflict marker labels.
    pub(super) fn oneline(&self) -> String {
        oneline(self.message.as_bstr())
    }
}

/// A commit to write on top of a parent.
pub(super) struct NewCommit<'a> {
    /// Root tree.
    pub(super) tree: ObjectId,
    /// Only parent.
    pub(super) parent: ObjectId,
    /// Raw `author` header, copied byte for byte.
    pub(super) author: &'a BStr,
    /// Raw `encoding` header.
    pub(super) encoding: Option<&'a BStr>,
    /// Message, copied byte for byte.
    pub(super) message: &'a [u8],
}

/// First line of a message.
pub(super) fn first_line(message: &BStr) -> &BStr {
    let end = message.find_byte(b'\n').unwrap_or(message.len());
    message[..end].as_bstr()
}

/// First paragraph of a message with line breaks replaced by spaces and
/// surrounding white space removed.
pub(super) fn oneline(message: &BStr) -> String {
    let mut words: Vec<String> = Vec::new();
    for line in message.lines() {
        let line = line.trim();
        if line.is_empty() {
            if words.is_empty() {
                continue;
            }
            break;
        }
        words.push(line.to_str_lossy().into_owned());
    }
    words.join(" ")
}

/// Identifier of the empty tree in the hash of the repository.
pub(super) fn empty_tree(repo: &gix::Repository) -> ObjectId {
    ObjectId::empty_tree(repo.object_hash())
}

/// Shortest unique abbreviation of an object name, at least `core.abbrev`
/// digits long.
pub(super) fn short(repo: &gix::Repository, id: ObjectId) -> String {
    id.attach(repo).shorten_or_id().to_string()
}

/// Reads a commit.
pub(super) fn read_commit(repo: &gix::Repository, id: ObjectId) -> Result<CommitInfo> {
    let object = repo
        .find_object(id)
        .map_err(|cause| error::io(&format!("read commit {id}"), cause))?;
    let commit = gix_object::CommitRef::from_bytes(&object.data, repo.object_hash())
        .map_err(|cause| error::corrupt(&format!("commit {id} cannot be parsed"), cause))?;
    let tree = parse_id(commit.tree, id)?;
    let mut parents = Vec::with_capacity(commit.parents.len());
    for parent in &commit.parents {
        parents.push(parse_id(parent, id)?);
    }
    Ok(CommitInfo {
        tree,
        parents,
        author: commit.author.to_owned(),
        encoding: commit.encoding.map(ToOwned::to_owned),
        message: commit.message.to_owned(),
    })
}

/// Parses a hexadecimal object name from a commit header.
fn parse_id(hex: &BStr, commit: ObjectId) -> Result<ObjectId> {
    ObjectId::from_hex(hex)
        .map_err(|cause| error::corrupt(&format!("commit {commit} names an invalid object"), cause))
}

/// Root tree of a commit.
pub(super) fn tree_of(repo: &gix::Repository, commit: ObjectId) -> Result<ObjectId> {
    Ok(read_commit(repo, commit)?.tree)
}

/// Content of a blob.
pub(super) fn read_blob(repo: &gix::Repository, id: ObjectId) -> Result<Vec<u8>> {
    let object = repo
        .find_object(id)
        .map_err(|cause| error::io(&format!("read blob {id}"), cause))?;
    Ok(object.data.clone())
}

/// Writes a commit object and returns its name.
///
/// The headers are written in the order and form used by Git; the author
/// is copied byte for byte, so the author date and its offset from UTC
/// stay unchanged.
pub(super) fn write_commit(
    repo: &gix::Repository,
    commit: &NewCommit<'_>,
    committer: &gix::actor::Signature,
) -> Result<ObjectId> {
    let mut data: Vec<u8> = Vec::with_capacity(commit.message.len() + 256);
    data.extend_from_slice(b"tree ");
    data.extend_from_slice(commit.tree.to_hex().to_string().as_bytes());
    data.extend_from_slice(b"\nparent ");
    data.extend_from_slice(commit.parent.to_hex().to_string().as_bytes());
    data.extend_from_slice(b"\nauthor ");
    data.extend_from_slice(commit.author);
    data.extend_from_slice(b"\ncommitter ");
    committer
        .write_to(&mut data)
        .map_err(|cause| error::io("format the committer", cause))?;
    if let Some(encoding) = commit.encoding {
        data.extend_from_slice(b"\nencoding ");
        data.extend_from_slice(encoding);
    }
    data.extend_from_slice(b"\n\n");
    data.extend_from_slice(commit.message);
    gix_object::Write::write_buf(repo, gix_object::Kind::Commit, &data)
        .map_err(|cause| error::io("write a commit", cause))
}

/// Entries of a tree; an absent tree has no entries.
fn entries(repo: &gix::Repository, tree: Option<ObjectId>) -> Result<Vec<Entry>> {
    let Some(tree) = tree else {
        return Ok(Vec::new());
    };
    let object = repo
        .find_object(tree)
        .map_err(|cause| error::io(&format!("read tree {tree}"), cause))?;
    let decoded = gix_object::TreeRef::from_bytes(&object.data, repo.object_hash())
        .map_err(|cause| error::corrupt(&format!("tree {tree} cannot be parsed"), cause))?;
    Ok(decoded
        .entries
        .iter()
        .map(|entry| Entry {
            name: entry.filename.to_owned(),
            mode: entry.mode,
            id: entry.oid.to_owned(),
        })
        .collect())
}

/// Joins a parent path and a name with `/`.
fn join(prefix: &BStr, name: &BStr) -> BString {
    if prefix.is_empty() {
        return name.to_owned();
    }
    let mut path = prefix.to_owned();
    path.push(b'/');
    path.extend_from_slice(name);
    path
}

/// Changed leaves between two trees, sorted by path; identical subtrees are
/// not read.
pub(super) fn diff_trees(
    repo: &gix::Repository,
    old: ObjectId,
    new: ObjectId,
) -> Result<Vec<LeafChange>> {
    let mut out = Vec::new();
    if old != new {
        diff_into(repo, Some(old), Some(new), BStr::new(""), &mut out)?;
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}

fn diff_into(
    repo: &gix::Repository,
    old: Option<ObjectId>,
    new: Option<ObjectId>,
    prefix: &BStr,
    out: &mut Vec<LeafChange>,
) -> Result<()> {
    let old_entries = entries(repo, old)?;
    let new_entries = entries(repo, new)?;
    let names: BTreeSet<&BString> = old_entries
        .iter()
        .chain(new_entries.iter())
        .map(|entry| &entry.name)
        .collect();
    for name in names {
        let before = old_entries.iter().find(|entry| &entry.name == name);
        let after = new_entries.iter().find(|entry| &entry.name == name);
        let path = join(prefix, name.as_bstr());
        diff_entry(repo, before, after, path, out)?;
    }
    Ok(())
}

fn diff_entry(
    repo: &gix::Repository,
    before: Option<&Entry>,
    after: Option<&Entry>,
    path: BString,
    out: &mut Vec<LeafChange>,
) -> Result<()> {
    if let (Some(a), Some(b)) = (before, after)
        && a.id == b.id
        && a.mode == b.mode
    {
        return Ok(());
    }
    let old_tree = before
        .filter(|entry| entry.mode.is_tree())
        .map(|entry| entry.id);
    let new_tree = after
        .filter(|entry| entry.mode.is_tree())
        .map(|entry| entry.id);
    if old_tree.is_some() || new_tree.is_some() {
        diff_into(repo, old_tree, new_tree, path.as_bstr(), out)?;
    }
    let old_leaf = before.filter(|entry| !entry.mode.is_tree()).map(leaf);
    let new_leaf = after.filter(|entry| !entry.mode.is_tree()).map(leaf);
    if old_leaf.is_some() || new_leaf.is_some() {
        out.push(LeafChange {
            path,
            old: old_leaf,
            new: new_leaf,
        });
    }
    Ok(())
}

fn leaf(entry: &Entry) -> Leaf {
    Leaf {
        mode: entry.mode,
        id: entry.id,
    }
}

/// Octal text of a tree entry mode as Git writes it in patches (`100644`).
pub(super) fn mode_text(mode: EntryMode) -> String {
    format!("{:06o}", u32::from(mode.value()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_line_stops_at_the_first_line_break() {
        assert_eq!(first_line(BStr::new("subj  \n\nbody\n")), "subj  ");
        assert_eq!(first_line(BStr::new("no newline")), "no newline");
    }

    #[test]
    fn oneline_joins_the_first_paragraph() {
        assert_eq!(oneline(BStr::new("one\ntwo \n\nbody\n")), "one two");
        assert_eq!(oneline(BStr::new("\n\nlate subject\n")), "late subject");
    }

    #[test]
    fn join_omits_the_separator_at_the_root() {
        assert_eq!(join(BStr::new(""), BStr::new("a")), "a");
        assert_eq!(join(BStr::new("d"), BStr::new("a")), "d/a");
    }

    #[test]
    fn mode_text_has_six_octal_digits() {
        let blob = EntryMode::from(gix_object::tree::EntryKind::Blob);
        let tree = EntryMode::from(gix_object::tree::EntryKind::Tree);
        assert_eq!(mode_text(blob), "100644");
        assert_eq!(mode_text(tree), "040000");
    }
}
