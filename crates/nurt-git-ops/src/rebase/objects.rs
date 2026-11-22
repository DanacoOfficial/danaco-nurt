// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

//! Reading of trees and commits and writing of commits.

use std::collections::{BTreeMap, BTreeSet};

use bstr::{BStr, BString, ByteSlice};
use gix::ObjectId;
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

/// Leaves of a tree by path relative to the root.
pub(super) type Flat = BTreeMap<BString, Leaf>;

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
    /// Commit.
    pub(super) id: ObjectId,
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
    /// First line of the message, as used in reflog messages and labels.
    pub(super) fn subject(&self) -> &BStr {
        first_line(self.message.as_bstr())
    }

    /// First paragraph of the message joined into one line, as shown in the
    /// lines of the todo list.
    pub(super) fn oneline(&self) -> String {
        oneline(self.message.as_bstr())
    }
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

/// Text of a reflog message: every run of white space becomes one space and
/// white space at both ends is removed.
pub(super) fn reflog_text(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Identifier of the empty tree in the hash of the repository.
pub(super) fn empty_tree(repo: &gix::Repository) -> ObjectId {
    ObjectId::empty_tree(repo.object_hash())
}

/// Reads a commit.
pub(super) fn read_commit(repo: &gix::Repository, id: ObjectId) -> Result<CommitInfo> {
    let object = repo
        .find_object(id)
        .map_err(|cause| error::io(&format!("cannot read commit {id}"), cause))?;
    let commit = gix_object::CommitRef::from_bytes(&object.data)
        .map_err(|cause| error::corrupt(&format!("cannot parse commit {id}"), cause))?;
    let tree = parse_id(commit.tree, id)?;
    let mut parents = Vec::with_capacity(commit.parents.len());
    for parent in &commit.parents {
        parents.push(parse_id(parent, id)?);
    }
    Ok(CommitInfo {
        id,
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
        .map_err(|cause| error::corrupt(&format!("invalid object name in commit {commit}"), cause))
}

/// Root tree of a commit.
pub(super) fn tree_of(repo: &gix::Repository, commit: ObjectId) -> Result<ObjectId> {
    Ok(read_commit(repo, commit)?.tree)
}

/// Content of a blob.
pub(super) fn read_blob(repo: &gix::Repository, id: ObjectId) -> Result<Vec<u8>> {
    let object = repo
        .find_object(id)
        .map_err(|cause| error::io(&format!("cannot read blob {id}"), cause))?;
    Ok(object.data.clone())
}

/// Writes a commit object and returns its name.
///
/// The headers are written in the order and form used by Git; `author` is
/// copied byte for byte, so the author date and time zone stay unchanged.
pub(super) fn write_commit(
    repo: &gix::Repository,
    tree: ObjectId,
    parent: ObjectId,
    author: &BStr,
    committer: &BStr,
    message: &BStr,
) -> Result<ObjectId> {
    let mut data: Vec<u8> = Vec::with_capacity(message.len() + 256);
    data.extend_from_slice(b"tree ");
    data.extend_from_slice(tree.to_hex().to_string().as_bytes());
    data.extend_from_slice(b"\nparent ");
    data.extend_from_slice(parent.to_hex().to_string().as_bytes());
    data.extend_from_slice(b"\nauthor ");
    data.extend_from_slice(author);
    data.extend_from_slice(b"\ncommitter ");
    data.extend_from_slice(committer);
    data.extend_from_slice(b"\n\n");
    data.extend_from_slice(message);
    write_object(repo, gix_object::Kind::Commit, &data)
}

/// Writes an object and returns its name.
pub(super) fn write_object(
    repo: &gix::Repository,
    kind: gix_object::Kind,
    data: &[u8],
) -> Result<ObjectId> {
    use gix_object::Write;
    repo.write_buf(kind, data)
        .map_err(|cause| error::io("cannot write an object", cause))
}

/// Entries of a tree; an absent tree has no entries.
fn entries(repo: &gix::Repository, tree: Option<ObjectId>) -> Result<Vec<Entry>> {
    let Some(tree) = tree else {
        return Ok(Vec::new());
    };
    let object = repo
        .find_object(tree)
        .map_err(|cause| error::io(&format!("cannot read tree {tree}"), cause))?;
    let decoded = gix_object::TreeRef::from_bytes(&object.data, repo.object_hash())
        .map_err(|cause| error::corrupt(&format!("cannot parse tree {tree}"), cause))?;
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

/// All leaves of a tree by path.
pub(super) fn flatten(repo: &gix::Repository, tree: ObjectId) -> Result<Flat> {
    let mut out = Flat::new();
    flatten_into(repo, tree, BStr::new(""), &mut out)?;
    Ok(out)
}

fn flatten_into(repo: &gix::Repository, tree: ObjectId, prefix: &BStr, out: &mut Flat) -> Result<()> {
    for entry in entries(repo, Some(tree))? {
        let path = join(prefix, entry.name.as_bstr());
        if entry.mode.is_tree() {
            flatten_into(repo, entry.id, path.as_bstr(), out)?;
        } else {
            out.insert(
                path,
                Leaf {
                    mode: entry.mode,
                    id: entry.id,
                },
            );
        }
    }
    Ok(())
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
    let old_tree = before.filter(|entry| entry.mode.is_tree()).map(|entry| entry.id);
    let new_tree = after.filter(|entry| entry.mode.is_tree()).map(|entry| entry.id);
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
    format!("{:06o}", mode_bits(mode))
}

/// Numeric value of a tree entry mode.
pub(super) fn mode_bits(mode: EntryMode) -> u32 {
    u32::from(mode.value())
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
    fn reflog_text_collapses_white_space() {
        assert_eq!(reflog_text("  subj \t x  "), "subj x");
    }

    #[test]
    fn join_omits_the_separator_at_the_root() {
        assert_eq!(join(BStr::new(""), BStr::new("a")), "a");
        assert_eq!(join(BStr::new("d"), BStr::new("a")), "d/a");
    }
}
