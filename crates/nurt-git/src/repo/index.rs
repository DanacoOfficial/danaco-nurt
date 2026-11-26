// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Reading and writing the index, and building trees from it.
//!
//! The repository library serializes the index entries, but it writes the
//! cached tree extension (`TREE`) without recomputing it and drops the
//! resolve-undo extension (`REUC`). [`write_index`] therefore removes the
//! cached tree before every write, serializes the entries with the library,
//! appends the resolve-undo records kept by [`IndexFile`], recomputes the
//! trailing checksum and replaces `index` in the transaction, refusing to
//! write while `index.lock` exists. The untracked cache (`UNTR`) and the file system monitor
//! data (`FSMN`) are not written; `git` rebuilds them when it needs them.

use std::path::{Path, PathBuf};

use bstr::{BStr, BString, ByteSlice};
use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::journal::{Applied, Expect, MutationTx};
use sha1::Digest as _;

use super::lock::{LockedWrite, LockedWrites, content_hash};
use super::{GitRepo, error};

/// Signature of the resolve-undo extension.
pub const RESOLVE_UNDO: [u8; 4] = *b"REUC";
/// Signature of the end-of-index-entry extension.
pub const END_OF_INDEX_ENTRY: [u8; 4] = *b"EOIE";
/// Signature of the index file header.
const HEADER_SIGNATURE: &[u8; 4] = b"DIRC";
/// Bytes of the index file header: signature, version and entry count.
const HEADER_LEN: usize = 12;
/// Bytes of the fixed part of an entry before the object id: ten 32-bit
/// fields of times, device, inode, mode, ids and size.
const ENTRY_STAT_LEN: usize = 40;
/// Bytes of the flags field of an entry.
const ENTRY_FLAGS_LEN: usize = 2;
/// Flag of an entry with a second flags field (index version 3 and later).
const ENTRY_EXTENDED: u16 = 0x4000;
/// Bytes of an extension header: signature and size.
const EXTENSION_HEADER_LEN: usize = 8;
/// Alignment of entries in index versions 2 and 3.
const ENTRY_ALIGNMENT: usize = 8;
/// Index version with prefix-compressed paths.
const VERSION_PATH_COMPRESSION: u32 = 4;
/// Number of stages recorded by a resolve-undo record: base, ours, theirs.
const STAGES: usize = 3;
/// Radix of the modes written in resolve-undo records.
const OCTAL: u32 = 8;

/// Previous conflict stages of one path, recorded when the conflict was
/// resolved; `git checkout -m <path>` uses them to recreate the conflict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolveUndo {
    /// Path relative to the repository root.
    pub path: BString,
    /// Mode and object of the base, ours and theirs stages; `None` for a
    /// stage that was absent.
    pub stages: [Option<(u32, gix::ObjectId)>; STAGES],
}

/// The index of a working tree with the resolve-undo records the library
/// does not keep.
#[derive(Debug, Clone)]
pub struct IndexFile {
    /// Entries and extensions decoded by the library.
    pub state: gix::index::State,
    /// Resolve-undo records, in file order.
    pub resolve_undo: Vec<ResolveUndo>,
    /// Location of the index file.
    pub path: PathBuf,
    /// Content of the file when it was read, which a later write must still
    /// find: another process may have replaced the index in between.
    pub expect: Expect,
}

impl GitRepo {
    /// Reads the index of this working tree; an empty index when the file
    /// does not exist.
    ///
    /// # Errors
    ///
    /// Fails with `E_IO` when the index exists but cannot be read or
    /// decoded.
    pub fn read_index(&self) -> Result<IndexFile, Error> {
        let path = self.index_path();
        let hash = self.gix().object_hash();
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(failure) if failure.kind() == std::io::ErrorKind::NotFound => {
                return Ok(IndexFile {
                    state: gix::index::State::new(hash),
                    resolve_undo: Vec::new(),
                    path,
                    expect: Expect::Absent,
                });
            }
            Err(failure) => return Err(error::io(&path, &failure)),
        };
        let file = self.index_file()?;
        let resolve_undo = match resolve_undo_extension(&bytes, hash) {
            Some(data) => decode_resolve_undo(data, hash).ok_or_else(|| {
                Error::new(
                    ErrorCode::Io,
                    format!(
                        "the resolve-undo data of {} cannot be decoded and would be lost by the next write; repair or remove the index",
                        error::quote_path(&path)
                    ),
                )
                .with_problem("corrupt_index")
            })?,
            None => Vec::new(),
        };
        Ok(IndexFile {
            state: file.into(),
            resolve_undo,
            path,
            expect: Expect::Hash(content_hash(&bytes)),
        })
    }
}

/// Serializes `state` as `git` stores it: no cached tree, the resolve-undo
/// records appended, and the trailing checksum of the object format.
///
/// # Errors
///
/// Fails with `E_IO` when the library cannot serialize the entries.
pub fn serialize(
    state: &mut gix::index::State,
    resolve_undo: &[ResolveUndo],
) -> Result<Vec<u8>, Error> {
    state.remove_tree();
    let hash = state.object_hash();
    let mut out = Vec::new();
    state
        .write_to(
            &mut out,
            gix::index::write::Options {
                extensions: gix::index::write::Extensions::All,
                skip_hash: true,
            },
        )
        .map_err(|failure| io_error("serialize the index", &failure))?;
    let extensions_start = entries_end(&out).ok_or_else(|| {
        Error::new(ErrorCode::Io, "the serialized index has an invalid layout")
            .with_problem("corrupt_index")
    })?;
    let mut rebuilt = out[..extensions_start].to_vec();
    for (signature, data) in extensions(&out[extensions_start..]) {
        if signature == END_OF_INDEX_ENTRY || signature == RESOLVE_UNDO {
            continue;
        }
        push_extension(&mut rebuilt, signature, data);
    }
    if !resolve_undo.is_empty() {
        push_extension(
            &mut rebuilt,
            RESOLVE_UNDO,
            &encode_resolve_undo(resolve_undo),
        );
    }
    let checksum = checksum(hash, &rebuilt);
    rebuilt.extend_from_slice(&checksum);
    Ok(rebuilt)
}

fn io_error(what: &str, failure: &dyn std::fmt::Display) -> Error {
    Error::new(ErrorCode::Io, format!("{what} failed: {failure}")).with_problem("io")
}

fn checksum(hash: gix::hash::Kind, bytes: &[u8]) -> Vec<u8> {
    match hash {
        gix::hash::Kind::Sha256 => sha2::Sha256::digest(bytes).to_vec(),
        _ => sha1::Sha1::digest(bytes).to_vec(),
    }
}

fn push_extension(out: &mut Vec<u8>, signature: [u8; 4], data: &[u8]) {
    out.extend_from_slice(&signature);
    let size = u32::try_from(data.len()).unwrap_or(u32::MAX);
    out.extend_from_slice(&size.to_be_bytes());
    out.extend_from_slice(data);
}

/// The locked write that replaces the index of `file` with its
/// serialization, for a transaction that declares other targets as well.
///
/// # Errors
///
/// As [`serialize`].
pub fn locked_write(file: &mut IndexFile) -> Result<LockedWrite, Error> {
    let bytes = serialize(&mut file.state, &file.resolve_undo)?;
    Ok(LockedWrite::new(&file.path, bytes).expecting(file.expect.clone()))
}

/// Replaces the index with `file` in a transaction with no other targets:
/// the index is one target that expects its previous content.
///
/// # Errors
///
/// Fails with `E_GIT_LOCKED` when `index.lock` exists, `E_IO` when the
/// index cannot be serialized, and with the errors of the transaction.
pub fn write_index(tx: &mut dyn MutationTx, file: &mut IndexFile) -> Result<Applied, Error> {
    let write = locked_write(file)?;
    let mut applied = LockedWrites::new(vec![write]).run(tx)?;
    applied
        .pop()
        .ok_or_else(|| Error::new(ErrorCode::Internal, "index write applied nothing"))
}

/// Offset of the first extension of a serialized index without its
/// checksum, or `None` when the entries do not fit the data.
#[must_use]
pub fn entries_end(data: &[u8]) -> Option<usize> {
    if data.len() < HEADER_LEN || &data[..4] != HEADER_SIGNATURE {
        return None;
    }
    let version = u32::from_be_bytes(data[4..8].try_into().ok()?);
    let count = u32::from_be_bytes(data[8..12].try_into().ok()?);
    let hash_len = hash_len_from_size(data, version, count)?;
    let mut offset = HEADER_LEN;
    for _ in 0..count {
        offset = entry_end(data, offset, version, hash_len)?;
    }
    Some(offset)
}

fn hash_len_from_size(data: &[u8], version: u32, count: u32) -> Option<usize> {
    if count == 0 {
        return Some(gix::hash::Kind::Sha1.len_in_bytes());
    }
    [gix::hash::Kind::Sha1, gix::hash::Kind::Sha256]
        .into_iter()
        .map(|kind| kind.len_in_bytes())
        .find(|hash_len| {
            let mut offset = HEADER_LEN;
            for _ in 0..count {
                match entry_end(data, offset, version, *hash_len) {
                    Some(end) => offset = end,
                    None => return false,
                }
            }
            extensions_valid(&data[offset..])
        })
}

fn extensions_valid(mut data: &[u8]) -> bool {
    while !data.is_empty() {
        if data.len() < EXTENSION_HEADER_LEN {
            return false;
        }
        let Ok(size_bytes) = <[u8; 4]>::try_from(&data[4..8]) else {
            return false;
        };
        let Ok(size) = usize::try_from(u32::from_be_bytes(size_bytes)) else {
            return false;
        };
        let end = EXTENSION_HEADER_LEN + size;
        if data.len() < end {
            return false;
        }
        data = &data[end..];
    }
    true
}

fn entry_end(data: &[u8], start: usize, version: u32, hash_len: usize) -> Option<usize> {
    let flags_at = start + ENTRY_STAT_LEN + hash_len;
    let flags = u16::from_be_bytes(
        data.get(flags_at..flags_at + ENTRY_FLAGS_LEN)?
            .try_into()
            .ok()?,
    );
    let mut path_at = flags_at + ENTRY_FLAGS_LEN;
    if flags & ENTRY_EXTENDED != 0 {
        path_at += ENTRY_FLAGS_LEN;
    }
    if version >= VERSION_PATH_COMPRESSION {
        let mut at = path_at;
        loop {
            let byte = *data.get(at)?;
            at += 1;
            if byte & 0x80 == 0 {
                break;
            }
        }
        let nul = data.get(at..)?.find_byte(0)?;
        return Some(at + nul + 1);
    }
    let nul = data.get(path_at..)?.find_byte(0)?;
    let unpadded = path_at + nul + 1 - start;
    let padded = unpadded.div_ceil(ENTRY_ALIGNMENT) * ENTRY_ALIGNMENT;
    let end = start + padded;
    (end <= data.len()).then_some(end)
}

/// Extensions of the part of an index after its entries, as pairs of
/// signature and data; a truncated extension ends the list.
#[must_use]
pub fn extensions(mut data: &[u8]) -> Vec<([u8; 4], &[u8])> {
    let mut found = Vec::new();
    while data.len() >= EXTENSION_HEADER_LEN {
        let Ok(signature) = <[u8; 4]>::try_from(&data[..4]) else {
            break;
        };
        let Ok(size_bytes) = <[u8; 4]>::try_from(&data[4..8]) else {
            break;
        };
        let Ok(size) = usize::try_from(u32::from_be_bytes(size_bytes)) else {
            break;
        };
        let end = EXTENSION_HEADER_LEN + size;
        if data.len() < end {
            break;
        }
        found.push((signature, &data[EXTENSION_HEADER_LEN..end]));
        data = &data[end..];
    }
    found
}

/// Data of the `REUC` extension of a complete index file; `None` when there
/// is none or the layout of the file cannot be read.
fn resolve_undo_extension(file: &[u8], hash: gix::hash::Kind) -> Option<&[u8]> {
    let body = file.get(..file.len().checked_sub(hash.len_in_bytes())?)?;
    let start = entries_end(body)?;
    extensions(&body[start..])
        .into_iter()
        .find(|(signature, _)| *signature == RESOLVE_UNDO)
        .map(|(_, data)| data)
}

/// Resolve-undo records of a complete index file; `None` when the file
/// layout cannot be read.
#[must_use]
pub fn read_resolve_undo(file: &[u8], hash: gix::hash::Kind) -> Option<Vec<ResolveUndo>> {
    let body = file.get(..file.len().checked_sub(hash.len_in_bytes())?)?;
    let start = entries_end(body)?;
    let data = extensions(&body[start..])
        .into_iter()
        .find(|(signature, _)| *signature == RESOLVE_UNDO)
        .map(|(_, data)| data);
    match data {
        Some(data) => decode_resolve_undo(data, hash),
        None => Some(Vec::new()),
    }
}

/// Decodes the data of a `REUC` extension.
#[must_use]
pub fn decode_resolve_undo(mut data: &[u8], hash: gix::hash::Kind) -> Option<Vec<ResolveUndo>> {
    let hash_len = hash.len_in_bytes();
    let mut records = Vec::new();
    while !data.is_empty() {
        let nul = data.find_byte(0)?;
        let path = BString::from(&data[..nul]);
        data = &data[nul + 1..];
        let mut modes = [0_u32; STAGES];
        for mode in &mut modes {
            let nul = data.find_byte(0)?;
            *mode = u32::from_str_radix(data[..nul].to_str().ok()?, OCTAL).ok()?;
            data = &data[nul + 1..];
        }
        let mut stages = [None; STAGES];
        for (stage, mode) in stages.iter_mut().zip(modes) {
            if mode == 0 {
                continue;
            }
            let id = gix::ObjectId::try_from(data.get(..hash_len)?).ok()?;
            data = &data[hash_len..];
            *stage = Some((mode, id));
        }
        records.push(ResolveUndo { path, stages });
    }
    Some(records)
}

/// Encodes resolve-undo records as the data of a `REUC` extension.
#[must_use]
pub fn encode_resolve_undo(records: &[ResolveUndo]) -> Vec<u8> {
    let mut out = Vec::new();
    for record in records {
        out.extend_from_slice(&record.path);
        out.push(0);
        for stage in &record.stages {
            let mode = stage.map_or(0, |(mode, _)| mode);
            out.extend_from_slice(format!("{mode:o}").as_bytes());
            out.push(0);
        }
        for (_, id) in record.stages.iter().flatten() {
            out.extend_from_slice(id.as_bytes());
        }
    }
    out
}

/// Writes the trees of the stage-0 entries of `state` to the object store
/// and returns the root tree, as `git write-tree` does.
///
/// Entries marked intent-to-add are left out; sparse directory entries
/// become subtrees with their recorded tree.
///
/// # Errors
///
/// Fails with `E_GIT_IN_PROGRESS` (problem `unmerged_paths`) when the index
/// has conflict stages, and with `E_IO` when an object cannot be written.
pub fn index_to_tree(repo: &GitRepo, state: &gix::index::State) -> Result<gix::ObjectId, Error> {
    let unmerged = unmerged_paths(state);
    if !unmerged.is_empty() {
        let mut failure = Error::new(
            ErrorCode::GitInProgress,
            "the index has unresolved conflicts; resolve them and stage the files first",
        )
        .with_problem("unmerged_paths")
        .with_next_step(format!(
            "git.stage {} action: add paths: [{}]",
            error::quote_path(repo.root()),
            unmerged
                .iter()
                .map(|path| error::quote(&path.to_str_lossy()))
                .collect::<Vec<_>>()
                .join(", ")
        ));
        for path in unmerged.iter().take(error::MAX_CANDIDATES) {
            failure = failure.with_sample(path.to_str_lossy());
        }
        return Err(failure);
    }
    let write_error = |failure: gix::Error| error::from_gix("write a tree", &failure);
    let mut editor = repo.gix().empty_tree().edit().map_err(write_error)?;
    for entry in state.entries() {
        if entry
            .flags
            .contains(gix::index::entry::Flags::INTENT_TO_ADD)
        {
            continue;
        }
        let kind = tree_kind(entry.mode);
        editor
            .upsert(
                entry.path(state).trim_end_with(|ch| ch == '/').as_bstr(),
                kind,
                entry.id,
            )
            .map_err(write_error)?;
    }
    editor.write().map(gix::Id::detach).map_err(write_error)
}

fn tree_kind(mode: gix::index::entry::Mode) -> gix::object::tree::EntryKind {
    use gix::index::entry::Mode;
    use gix::object::tree::EntryKind;
    if mode == Mode::DIR {
        EntryKind::Tree
    } else if mode == Mode::SYMLINK {
        EntryKind::Link
    } else if mode == Mode::COMMIT {
        EntryKind::Commit
    } else if mode == Mode::FILE_EXECUTABLE {
        EntryKind::BlobExecutable
    } else {
        EntryKind::Blob
    }
}

/// Paths with entries of stage 1, 2 or 3, each once, in index order.
#[must_use]
pub fn unmerged_paths(state: &gix::index::State) -> Vec<BString> {
    let mut paths: Vec<BString> = Vec::new();
    for entry in state.entries() {
        if entry.stage_raw() == 0 {
            continue;
        }
        let path = entry.path(state);
        if paths.last().is_none_or(|last| last.as_bstr() != path) {
            paths.push(path.to_owned());
        }
    }
    paths
}

/// Replaces every sparse directory entry of `state` by the files of its
/// tree, marked skip-worktree, so that the index can be compared with trees
/// path by path. An index without sparse directories is returned unchanged.
///
/// # Errors
///
/// Fails with `E_IO` when a tree cannot be read.
pub fn expand_sparse(
    repo: &GitRepo,
    state: &gix::index::State,
) -> Result<gix::index::State, Error> {
    let has_directories = state
        .entries()
        .iter()
        .any(|entry| entry.mode == gix::index::entry::Mode::DIR);
    if !has_directories {
        return Ok(state.clone());
    }
    let mut expanded = gix::index::State::new(state.object_hash());
    for entry in state.entries() {
        let path = entry.path(state);
        if entry.mode == gix::index::entry::Mode::DIR {
            let prefix = path.trim_end_with(|ch| ch == '/');
            push_tree(repo, &mut expanded, entry.id, prefix.as_bstr(), 0)?;
        } else {
            expanded.dangerously_push_entry(entry.stat, entry.id, entry.flags, entry.mode, path);
        }
    }
    expanded.sort_entries();
    Ok(expanded)
}

/// Deepest nesting of trees below a sparse directory entry that
/// [`expand_sparse`] follows; a deeper tree is corrupt or hostile and would
/// exhaust the stack.
const MAX_TREE_DEPTH: usize = 512;

fn push_tree(
    repo: &GitRepo,
    state: &mut gix::index::State,
    tree: gix::ObjectId,
    prefix: &BStr,
    depth: usize,
) -> Result<(), Error> {
    if depth > MAX_TREE_DEPTH {
        return Err(Error::new(
            ErrorCode::Io,
            format!("the trees below `{prefix}` are nested deeper than {MAX_TREE_DEPTH} levels"),
        )
        .with_problem("tree_too_deep"));
    }
    let read_error = |failure: gix::Error| error::from_gix("read a tree", &failure);
    let object = repo.gix().find_tree(tree).map_err(read_error)?;
    let decoded = object.decode().map_err(read_error)?;
    for item in &decoded.entries {
        let mut path = BString::from(prefix.to_vec());
        path.push(b'/');
        path.extend_from_slice(item.filename);
        let id = item.oid.to_owned();
        if item.mode.is_tree() {
            push_tree(repo, state, id, path.as_bstr(), depth + 1)?;
            continue;
        }
        let mode = gix::index::entry::Mode::from_bits(u32::from(item.mode.value()))
            .unwrap_or(gix::index::entry::Mode::FILE);
        state.dangerously_push_entry(
            gix::index::entry::Stat::default(),
            id,
            gix::index::entry::Flags::SKIP_WORKTREE,
            mode,
            path.as_bstr(),
        );
    }
    Ok(())
}

/// Location of the index of `repo`'s working tree.
#[must_use]
pub fn index_path(repo: &GitRepo) -> PathBuf {
    repo.index_path()
}

/// Whether `path` is the index file or its lock.
#[must_use]
pub fn is_index_file(path: &Path) -> bool {
    path.file_name()
        .is_some_and(|name| name == "index" || name == "index.lock")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{RecordingJournal, RepoBuilder, test_ctx};

    fn open(fixture: &crate::testutil::Fixture) -> GitRepo {
        GitRepo::open(fixture.path(), &test_ctx()).unwrap()
    }

    #[test]
    fn index_to_tree_matches_the_committed_tree() {
        let fixture = RepoBuilder::new()
            .file("a.txt", "1\n")
            .executable("bin/run", "#!/bin/sh\n")
            .file("dir/sub/b.txt", "2\n")
            .commit("one")
            .build();
        let repo = open(&fixture);
        let index = repo.read_index().unwrap();
        let tree = index_to_tree(&repo, &index.state).unwrap();
        assert_eq!(tree, repo.resolve_rev("HEAD^{tree}").unwrap());
    }

    #[test]
    fn index_to_tree_of_an_empty_index_is_the_empty_tree() {
        let fixture = RepoBuilder::new().build();
        let repo = open(&fixture);
        let index = repo.read_index().unwrap();
        assert_eq!(
            index_to_tree(&repo, &index.state).unwrap(),
            gix::ObjectId::empty_tree(gix::hash::Kind::Sha1)
        );
    }

    #[test]
    fn index_to_tree_refuses_conflicts() {
        let fixture = RepoBuilder::new()
            .file("a.txt", "1\n")
            .commit("one")
            .build();
        let repo = open(&fixture);
        let mut index = repo.read_index().unwrap();
        let entry = index.state.entries()[0].clone();
        let flags = gix::index::entry::Flags::from_stage(gix::index::entry::Stage::Ours);
        index.state.dangerously_push_entry(
            entry.stat,
            entry.id,
            flags,
            entry.mode,
            b"a.txt".as_bstr(),
        );
        index.state.sort_entries();
        let failure = index_to_tree(&repo, &index.state).unwrap_err();
        assert_eq!(failure.code, ErrorCode::GitInProgress);
        assert_eq!(failure.sample, vec!["a.txt".to_owned()]);
    }

    #[test]
    fn written_index_round_trips_and_undo_restores_it() {
        let fixture = RepoBuilder::new()
            .file("a.txt", "1\n")
            .file("b.txt", "2\n")
            .commit("one")
            .build();
        let before = fixture.snapshot();
        let repo = open(&fixture);
        let mut index = repo.read_index().unwrap();
        let removed: Vec<usize> = vec![0];
        index
            .state
            .remove_entries(|position, _, _| removed.contains(&position));
        let journal = RecordingJournal::new();
        let mut tx = journal.begin();
        write_index(tx.as_mut(), &mut index).unwrap();
        tx.commit().unwrap();
        let reread = repo.read_index().unwrap();
        assert_eq!(reread.state.entries().len(), 1);
        assert_eq!(reread.state.entries()[0].path(&reread.state), "b.txt");
        assert!(!fixture.git_dir().join("index.lock").exists());
        journal.undo_last();
        assert_eq!(fixture.snapshot(), before);
    }

    #[test]
    fn resolve_undo_records_survive_a_write() {
        let fixture = RepoBuilder::new()
            .file("a.txt", "1\n")
            .commit("one")
            .build();
        let repo = open(&fixture);
        let mut index = repo.read_index().unwrap();
        let blob = index.state.entries()[0].id;
        index.resolve_undo = vec![ResolveUndo {
            path: "a.txt".into(),
            stages: [Some((0o100_644, blob)), None, Some((0o100_755, blob))],
        }];
        let journal = RecordingJournal::new();
        let mut tx = journal.begin();
        write_index(tx.as_mut(), &mut index).unwrap();
        tx.commit().unwrap();
        let reread = repo.read_index().unwrap();
        assert_eq!(reread.resolve_undo, index.resolve_undo);
        assert_eq!(reread.state.entries().len(), 1);
    }

    #[test]
    fn a_sparse_directory_with_too_deep_trees_is_refused() {
        use gix::objs::tree::{Entry, EntryKind};
        let fixture = RepoBuilder::new()
            .file("a.txt", "1\n")
            .commit("one")
            .build();
        let repo = open(&fixture);
        let write = |name: &str, kind: EntryKind, oid: gix::ObjectId| {
            let tree = gix::objs::Tree {
                entries: vec![Entry {
                    mode: kind.into(),
                    filename: name.into(),
                    oid,
                }],
            };
            repo.gix().write_object(&tree).unwrap().detach()
        };
        let blob = repo.gix().write_blob(b"x").unwrap().detach();
        let mut tree = write("f", EntryKind::Blob, blob);
        for _ in 0..=MAX_TREE_DEPTH + 1 {
            tree = write("d", EntryKind::Tree, tree);
        }
        let mut state = gix::index::State::new(gix::hash::Kind::Sha1);
        state.dangerously_push_entry(
            gix::index::entry::Stat::default(),
            tree,
            gix::index::entry::Flags::empty(),
            gix::index::entry::Mode::DIR,
            "sparse/".into(),
        );
        let error = expand_sparse(&repo, &state).unwrap_err();
        assert_eq!(error.problem.as_deref(), Some("tree_too_deep"));
    }

    #[test]
    fn undecodable_resolve_undo_data_is_an_error_not_a_silent_loss() {
        let fixture = RepoBuilder::new()
            .file("a.txt", "1\n")
            .commit("one")
            .build();
        let repo = open(&fixture);
        let mut index = repo.read_index().unwrap();
        let bytes = serialize(&mut index.state, &[]).unwrap();
        let mut body = bytes[..bytes.len() - 20].to_vec();
        push_extension(&mut body, RESOLVE_UNDO, b"a.txt\x00100644\x00");
        let sum = checksum(gix::hash::Kind::Sha1, &body);
        body.extend_from_slice(&sum);
        std::fs::write(repo.index_path(), body).unwrap();
        let error = repo.read_index().unwrap_err();
        assert_eq!(error.problem.as_deref(), Some("corrupt_index"));
    }
    #[test]
    fn written_index_has_a_valid_checksum_and_no_cached_tree() {
        let fixture = RepoBuilder::new()
            .file("a.txt", "1\n")
            .commit("one")
            .build();
        let repo = open(&fixture);
        let mut index = repo.read_index().unwrap();
        let bytes = serialize(&mut index.state, &[]).unwrap();
        let (body, sum) = bytes.split_at(bytes.len() - 20);
        assert_eq!(sha1::Sha1::digest(body).as_slice(), sum);
        let start = entries_end(body).unwrap();
        assert!(
            extensions(&body[start..])
                .iter()
                .all(|(signature, _)| signature != b"TREE")
        );
    }

    #[test]
    fn existing_index_lock_fails_with_git_locked() {
        let fixture = RepoBuilder::new()
            .file("a.txt", "1\n")
            .commit("one")
            .build();
        std::fs::write(fixture.git_dir().join("index.lock"), b"").unwrap();
        let repo = open(&fixture);
        let mut index = repo.read_index().unwrap();
        let journal = RecordingJournal::new();
        let mut tx = journal.begin();
        let failure = write_index(tx.as_mut(), &mut index).unwrap_err();
        assert_eq!(failure.code, ErrorCode::GitLocked);
    }

    #[test]
    fn resolve_undo_encoding_round_trips() {
        let id = gix::ObjectId::empty_blob(gix::hash::Kind::Sha1);
        let records = vec![
            ResolveUndo {
                path: "x/y".into(),
                stages: [None, Some((0o100_644, id)), Some((0o120_000, id))],
            },
            ResolveUndo {
                path: "z".into(),
                stages: [Some((0o100_644, id)), None, None],
            },
        ];
        let data = encode_resolve_undo(&records);
        assert_eq!(
            decode_resolve_undo(&data, gix::hash::Kind::Sha1),
            Some(records)
        );
    }

    #[test]
    fn sparse_directories_are_expanded_with_skip_worktree() {
        let fixture = RepoBuilder::new()
            .file("keep.txt", "k\n")
            .file("out/a.txt", "a\n")
            .file("out/deep/b.txt", "b\n")
            .commit("one")
            .build();
        let repo = open(&fixture);
        let tree = repo.resolve_rev("HEAD:out").unwrap();
        let keep = repo.read_index().unwrap().state.entries()[0].clone();
        let mut sparse = gix::index::State::new(gix::hash::Kind::Sha1);
        sparse.dangerously_push_entry(
            keep.stat,
            keep.id,
            keep.flags,
            keep.mode,
            b"keep.txt".as_bstr(),
        );
        sparse.dangerously_push_entry(
            gix::index::entry::Stat::default(),
            tree,
            gix::index::entry::Flags::SKIP_WORKTREE,
            gix::index::entry::Mode::DIR,
            b"out/".as_bstr(),
        );
        let expanded = expand_sparse(&repo, &sparse).unwrap();
        let paths: Vec<String> = expanded
            .entries()
            .iter()
            .map(|entry| entry.path(&expanded).to_string())
            .collect();
        assert_eq!(paths, vec!["keep.txt", "out/a.txt", "out/deep/b.txt"]);
        assert!(
            expanded.entries()[1]
                .flags
                .contains(gix::index::entry::Flags::SKIP_WORKTREE)
        );
        assert_eq!(
            index_to_tree(&repo, &sparse).unwrap(),
            repo.resolve_rev("HEAD^{tree}").unwrap()
        );
    }

    #[test]
    fn index_file_names_are_recognized() {
        assert!(is_index_file(Path::new("/r/.git/index")));
        assert!(is_index_file(Path::new("/r/.git/index.lock")));
        assert!(!is_index_file(Path::new("/r/.git/HEAD")));
    }
}
