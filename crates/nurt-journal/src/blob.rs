// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Preserved content and metadata files.
//!
//! A clone or copy of previous content is created under a temporary name in
//! the `tmp/` directory of the entry, flushed and only then renamed to its
//! final name in `blobs/`: an existing blob is always complete. Content moved
//! or exchanged into the journal keeps its file identity, so it is recognised
//! by the identity recorded in the manifest.

use std::collections::HashSet;
use std::io::Read;

use nurt_contracts::fs::{
    AbsPath, Durability, ExtendedMeta, FileId, FileKind, Follow, FsError, FsErrorKind, FsResult,
    FsWrite, LinkKind, RenameMode, TxScope,
};
use nurt_contracts::journal::PreserveMode;

use crate::manifest;

/// Size of the buffer of streamed copies.
pub(crate) const COPY_BUFFER: usize = 64 * 1024;

/// Preserves the content of the file `source` as the blob `blob`: a clone
/// when `allow_clone` holds and the volume supports it, otherwise a byte
/// copy. Returns the mode used.
///
/// The copy is written under the name of the blob in `temp_dir`, flushed
/// and renamed; the blob never exists incomplete. Both directories are
/// flushed before the function returns, so the blob is durable.
pub(crate) fn preserve_copy(
    fs: &dyn FsWrite,
    scope: &TxScope,
    source: &AbsPath,
    temp_dir: &AbsPath,
    blob: &AbsPath,
    allow_clone: bool,
) -> FsResult<PreserveMode> {
    let name = blob
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| FsError::new(FsErrorKind::InvalidPath, "preserve", Some(blob.as_path())))?;
    let temp = temp_dir.join_name(name)?;
    discard(fs, scope, &temp)?;
    let mode = copy_into(fs, scope, source, &temp, allow_clone).inspect_err(|_| {
        let _ = discard(fs, scope, &temp);
    })?;
    let durable = fs
        .open_in_place(scope, &temp)
        .and_then(|mut handle| handle.sync(Durability::Full))
        .and_then(|()| fs.rename(scope, &temp, blob, RenameMode::NoReplace));
    if let Err(error) = durable {
        let _ = discard(fs, scope, &temp);
        return Err(error);
    }
    sync_parents(fs, scope, &[&temp, blob])?;
    Ok(mode)
}

/// Flushes the directories holding `paths`, each once. A rename is durable
/// when the directories of its source and destination are flushed.
pub(crate) fn sync_parents(fs: &dyn FsWrite, scope: &TxScope, paths: &[&AbsPath]) -> FsResult<()> {
    let mut flushed: Vec<AbsPath> = Vec::with_capacity(paths.len());
    for path in paths {
        if let Some(parent) = path.parent()
            && !flushed.contains(&parent)
        {
            fs.sync_dir(scope, &parent)?;
            flushed.push(parent);
        }
    }
    Ok(())
}

/// Flushes the metadata of the entry at `path`: a file through a write
/// handle, a directory directly. A file that cannot be opened for writing
/// (read-only permission bits) and entries of other kinds are flushed
/// through their directory.
pub(crate) fn sync_entry(fs: &dyn FsWrite, scope: &TxScope, path: &AbsPath) -> FsResult<()> {
    match fs.metadata(path, Follow::NoFollow)?.kind {
        FileKind::Directory => fs.sync_dir(scope, path),
        FileKind::File => match fs.open_in_place(scope, path) {
            Ok(mut handle) => handle.sync(Durability::Full),
            Err(error) if error.kind == FsErrorKind::PermissionDenied => {
                sync_parents(fs, scope, &[path])
            }
            Err(error) => Err(error),
        },
        _ => sync_parents(fs, scope, &[path]),
    }
}

/// Flushes a copied entry: every file and directory below `path` and the
/// directory holding `path`.
pub(crate) fn sync_tree(fs: &dyn FsWrite, scope: &TxScope, path: &AbsPath) -> FsResult<()> {
    let mut pending = vec![path.clone()];
    while let Some(next) = pending.pop() {
        if fs.metadata(&next, Follow::NoFollow)?.kind == FileKind::Directory {
            for child in fs.read_dir(&next)? {
                let name = child?.name;
                let name = name.to_str().ok_or_else(|| {
                    FsError::new(FsErrorKind::InvalidPath, "sync_tree", Some(next.as_path()))
                })?;
                pending.push(next.join_name(name)?);
            }
        }
        sync_entry(fs, scope, &next)?;
    }
    sync_parents(fs, scope, &[path])
}

fn copy_into(
    fs: &dyn FsWrite,
    scope: &TxScope,
    source: &AbsPath,
    dest: &AbsPath,
    allow_clone: bool,
) -> FsResult<PreserveMode> {
    if allow_clone {
        match fs.clone_file(scope, source, dest) {
            Ok(()) => return Ok(PreserveMode::Clone),
            Err(error) if error.kind == FsErrorKind::Unsupported => {}
            Err(error) => return Err(error),
        }
    }
    fs.copy_file(scope, source, dest)?;
    Ok(PreserveMode::Copy)
}

/// Removes a leftover file; a missing file is not an error.
pub(crate) fn discard(fs: &dyn FsWrite, scope: &TxScope, path: &AbsPath) -> FsResult<()> {
    match fs.remove_file(scope, path) {
        Err(error) if error.kind != FsErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

/// Removes an entry of any kind; a missing entry is not an error.
pub(crate) fn remove_entry(fs: &dyn FsWrite, scope: &TxScope, path: &AbsPath) -> FsResult<()> {
    let kind = match fs.metadata(path, Follow::NoFollow) {
        Ok(meta) => meta.kind,
        Err(error) if error.kind == FsErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if kind == FileKind::Directory {
        fs.remove_tree(scope, path, 1).map(|_| ())
    } else {
        fs.remove_file(scope, path)
    }
}

/// Fails with `Unsupported` when `source` or an entry below it is a device,
/// a pipe or a socket, which cannot be copied. The check runs before anything
/// is written, so a refused tree leaves no partial copy behind.
fn refuse_special_entries(fs: &dyn FsWrite, source: &AbsPath) -> FsResult<()> {
    let mut pending = vec![source.clone()];
    while let Some(path) = pending.pop() {
        match fs.extended_metadata(&path, Follow::NoFollow)?.basic.kind {
            FileKind::Directory => {
                for child in fs.read_dir(&path)? {
                    let name = child?.name;
                    let name = name.to_str().ok_or_else(|| {
                        FsError::new(FsErrorKind::InvalidPath, "copy", Some(path.as_path()))
                    })?;
                    pending.push(path.join_name(name)?);
                }
            }
            FileKind::Special => {
                return Err(FsError::new(
                    FsErrorKind::Unsupported,
                    "copy",
                    Some(path.as_path()),
                ));
            }
            _ => {}
        }
    }
    Ok(())
}

/// Copies an entry of any kind to the new path `dest`: files by
/// [`FsWrite::copy_file`], directories recursively, links as links. Used when
/// a rename crosses volumes or overlay layers. Returns the copied bytes.
pub(crate) fn copy_entry(
    fs: &dyn FsWrite,
    scope: &TxScope,
    source: &AbsPath,
    dest: &AbsPath,
) -> FsResult<u64> {
    refuse_special_entries(fs, source)?;
    let mut copied = 0;
    let mut pending = vec![(source.clone(), dest.clone())];
    while let Some((from, to)) = pending.pop() {
        let meta = fs.extended_metadata(&from, Follow::NoFollow)?;
        match meta.basic.kind {
            FileKind::Directory => {
                fs.create_dir(scope, &to, meta.basic.mode)?;
                for child in fs.read_dir(&from)? {
                    let name = child?.name;
                    let name = name.to_str().ok_or_else(|| {
                        FsError::new(FsErrorKind::InvalidPath, "copy", Some(from.as_path()))
                    })?;
                    pending.push((from.join_name(name)?, to.join_name(name)?));
                }
            }
            FileKind::Symlink | FileKind::Junction => {
                let target = fs.read_link(&from)?;
                let kind = if meta.basic.kind == FileKind::Junction {
                    LinkKind::Junction
                } else {
                    LinkKind::SymlinkFile
                };
                fs.create_link(scope, &target, &to, kind)?;
            }
            FileKind::File | FileKind::Special | _ => {
                copied += fs.copy_file(scope, &from, &to)?;
            }
        }
    }
    Ok(copied)
}

/// Writes preserved metadata atomically.
pub(crate) fn write_meta(
    fs: &dyn FsWrite,
    scope: &TxScope,
    temp_dir: &AbsPath,
    path: &AbsPath,
    meta: &ExtendedMeta,
) -> FsResult<()> {
    let bytes = serde_json::to_vec(meta)
        .map_err(|_| FsError::new(FsErrorKind::Io, "encode_meta", Some(path.as_path())))?;
    manifest::write_atomic(fs, scope, temp_dir, path, &bytes)
}

/// Copies the content of `source` into the open handle `dest` from its
/// current position, feeding every byte to `hasher`. Returns the number of
/// bytes.
pub(crate) fn stream_copy(
    fs: &dyn FsWrite,
    source: &AbsPath,
    dest: &mut dyn nurt_contracts::fs::WriteHandle,
    mut hasher: Option<&mut sha2::Sha256>,
) -> FsResult<u64> {
    use sha2::Digest;
    let mut reader = fs.open_read(source)?;
    let mut buffer = vec![0_u8; COPY_BUFFER];
    let mut total = 0_u64;
    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|_| FsError::new(FsErrorKind::Io, "read", Some(source.as_path())))?;
        if read == 0 {
            return Ok(total);
        }
        let chunk = &buffer[..read];
        if let Some(hasher) = hasher.as_deref_mut() {
            hasher.update(chunk);
        }
        manifest::write_all(dest, chunk)?;
        total += read as u64;
    }
}

/// Accounting of logical bytes held by one operation.
///
/// Moved, exchanged, hard-linked, cloned and copied content counts with its
/// logical size; content that keeps its file identity counts once per
/// identity; content that still has names outside the journal counts zero.
#[derive(Debug, Default)]
pub(crate) struct HeldBytes {
    seen: HashSet<FileId>,
}

impl HeldBytes {
    /// Bytes held for one preserved target.
    ///
    /// `identity` is the identity of the preserved file when it keeps the
    /// identity of the previous file (moved or exchanged content); `shared`
    /// holds when the preserved file still has other names.
    pub(crate) fn count(
        &mut self,
        mode: PreserveMode,
        size: u64,
        identity: Option<FileId>,
        shared: bool,
    ) -> u64 {
        let keeps_content = matches!(
            mode,
            PreserveMode::Moved
                | PreserveMode::Exchange
                | PreserveMode::ReplaceBackup
                | PreserveMode::Hardlink
                | PreserveMode::Clone
                | PreserveMode::Copy
        );
        if !keeps_content || shared {
            return 0;
        }
        match identity {
            Some(identity) if !self.seen.insert(identity) => 0,
            _ => size,
        }
    }
}

#[cfg(test)]
mod tests {
    use nurt_contracts::fs::{Fs, ScopePurpose};
    use nurt_contracts::journal::OpId;

    use super::*;
    use crate::store::scope;
    use crate::store::test_support::{Fixture, unflushed};

    fn id(low: u64) -> FileId {
        FileId {
            volume: 1,
            low,
            high: 0,
        }
    }

    #[test]
    fn bytes_held_follow_the_counting_rules() {
        let mut held = HeldBytes::default();
        assert_eq!(
            held.count(PreserveMode::Exchange, 100, Some(id(1)), false),
            100
        );
        assert_eq!(held.count(PreserveMode::Moved, 100, Some(id(1)), false), 0);
        assert_eq!(held.count(PreserveMode::Moved, 50, Some(id(2)), true), 0);
        assert_eq!(held.count(PreserveMode::Clone, 70, None, false), 70);
        assert_eq!(held.count(PreserveMode::Copy, 70, None, false), 70);
        assert_eq!(held.count(PreserveMode::Hardlink, 9, Some(id(3)), false), 9);
        for mode in [
            PreserveMode::None,
            PreserveMode::Created,
            PreserveMode::Meta,
            PreserveMode::Append,
            PreserveMode::Rename,
            PreserveMode::Value,
        ] {
            assert_eq!(held.count(mode, 10, None, false), 0, "{mode:?}");
        }
    }

    #[test]
    fn shared_content_is_not_remembered_by_identity_and_copies_count_every_time() {
        let mut held = HeldBytes::default();
        assert_eq!(held.count(PreserveMode::Moved, 50, Some(id(2)), true), 0);
        assert_eq!(held.count(PreserveMode::Moved, 50, Some(id(2)), false), 50);
        assert_eq!(
            held.count(PreserveMode::Exchange, 50, Some(id(2)), false),
            0
        );
        assert_eq!(
            held.count(PreserveMode::ReplaceBackup, 8, Some(id(3)), false),
            8
        );
        assert_eq!(held.count(PreserveMode::Copy, 4, None, false), 4);
        assert_eq!(held.count(PreserveMode::Copy, 4, None, false), 4);
        assert_eq!(held.count(PreserveMode::Clone, 0, None, false), 0);
        assert_eq!(held.count(PreserveMode::Copy, u64::MAX, None, true), 0);
        assert_eq!(
            held.count(PreserveMode::Copy, u64::MAX, None, false),
            u64::MAX
        );
    }

    #[test]
    fn parent_directories_are_flushed_once() {
        let fixture = Fixture::new();
        let fs = &fixture.fs;
        let scope = scope(OpId(1), ScopePurpose::Maintenance);
        let first = fs.path("vol-a/one.txt");
        let second = fs.path("vol-a/two.txt");
        fixture.write(&first, b"1");
        fixture.write(&second, b"2");
        fs.clear_log();
        sync_parents(fs, &scope, &[&first, &second, &first]).unwrap();
        let flushes = fs
            .log()
            .iter()
            .filter(|entry| entry.starts_with("sync_dir "))
            .count();
        assert_eq!(flushes, 1);
        sync_parents(fs, &scope, &[]).unwrap();
    }

    #[test]
    fn streams_of_empty_and_multibyte_content_are_copied_and_hashed() {
        use sha2::Digest;

        let fixture = Fixture::new();
        let fs = &fixture.fs;
        let scope = scope(OpId(1), ScopePurpose::Maintenance);
        let contents = [Vec::new(), "zażółć gęślą jaźń\r\n".as_bytes().to_vec()];
        for (n, content) in contents.into_iter().enumerate() {
            let source = fs.path(&format!("vol-a/s{n}.bin"));
            fixture.write(&source, &content);
            let target = fs.path(&format!("vol-a/d{n}.bin"));
            let mut dest = fs.create_new(&scope, &target).unwrap();
            let mut hasher = sha2::Sha256::default();
            let copied = stream_copy(fs, &source, dest.as_mut(), Some(&mut hasher)).unwrap();
            assert_eq!(copied, u64::try_from(content.len()).unwrap());
            let digest: [u8; 32] = hasher.finalize().into();
            assert_eq!(fs.content_hash(&source).unwrap().0, digest);
            assert_eq!(fixture.read(&target), content);
        }
    }

    #[test]
    fn removing_a_missing_entry_or_leftover_is_not_an_error() {
        let fixture = Fixture::new();
        let fs = &fixture.fs;
        let scope = scope(OpId(1), ScopePurpose::Maintenance);
        let missing = fs.path("vol-a/missing");
        assert_eq!(remove_entry(fs, &scope, &missing), Ok(()));
        assert_eq!(discard(fs, &scope, &missing), Ok(()));
        let file = fs.path("vol-a/leftover.txt");
        fixture.write(&file, b"x");
        assert_eq!(discard(fs, &scope, &file), Ok(()));
        assert!(!fixture.exists(&file));
    }

    #[test]
    fn copies_become_visible_only_when_complete() {
        let fixture = Fixture::new();
        let fs = &fixture.fs;
        let scope = scope(OpId(1), ScopePurpose::Maintenance);
        let source = fs.path("vol-a/a.txt");
        fixture.write(&source, b"old");
        let tmp = fs.path("vol-a/tmp");
        let blobs = fs.path("vol-a/blobs");
        std::fs::create_dir(tmp.as_path()).unwrap();
        std::fs::create_dir(blobs.as_path()).unwrap();
        let blob = blobs.join_name("0.blob").unwrap();
        assert_eq!(
            preserve_copy(fs, &scope, &source, &tmp, &blob, true),
            Ok(PreserveMode::Copy)
        );
        assert_eq!(fixture.read(&blob), b"old");
        assert!(fixture.names(&tmp).is_empty());
        let log = fs.log();
        let sync = log.iter().position(|e| e.starts_with("sync_file")).unwrap();
        let rename = log.iter().position(|e| e.starts_with("rename")).unwrap();
        assert!(sync < rename);
        assert!(unflushed(&log, log.len()).is_empty(), "{log:#?}");

        fs.configure(0, |volume| volume.features.reflink = true);
        let cloned = blobs.join_name("1.blob").unwrap();
        assert_eq!(
            preserve_copy(fs, &scope, &source, &tmp, &cloned, true),
            Ok(PreserveMode::Clone)
        );
        assert_eq!(fs.cloned(), 3);

        fs.fail("copy_file", "a.txt", FsErrorKind::NoSpace);
        let failed = blobs.join_name("2.blob").unwrap();
        let error = preserve_copy(fs, &scope, &source, &tmp, &failed, false).unwrap_err();
        assert_eq!(error.kind, FsErrorKind::NoSpace);
        assert!(!fixture.exists(&failed));
        fs.fail("rename", "3.blob", FsErrorKind::Io);
        let unrenamed = blobs.join_name("3.blob").unwrap();
        assert!(preserve_copy(fs, &scope, &source, &tmp, &unrenamed, false).is_err());
        assert!(fixture.names(&tmp).is_empty());
        fs.fail("clone_file", "a.txt", FsErrorKind::Io);
        let refused = blobs.join_name("4.blob").unwrap();
        assert!(preserve_copy(fs, &scope, &source, &tmp, &refused, true).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn copy_of_a_tree_with_a_socket_is_refused_before_anything_is_written() {
        let fixture = Fixture::new();
        let fs = &fixture.fs;
        let scope = scope(OpId(1), ScopePurpose::Operation);
        let tree = fs.path("vol-a/tree");
        std::fs::create_dir_all(tree.as_path().join("sub")).unwrap();
        std::fs::write(tree.as_path().join("a"), b"12345").unwrap();
        let _listener =
            std::os::unix::net::UnixListener::bind(tree.as_path().join("sub/socket")).unwrap();
        let copy = fs.path("vol-b/tree");
        let error = copy_entry(fs, &scope, &tree, &copy).unwrap_err();
        assert_eq!(error.kind, FsErrorKind::Unsupported);
        assert!(!fixture.exists(&copy));
    }

    #[test]
    fn copies_and_removes_trees_with_links() {
        let fixture = Fixture::new();
        let fs = &fixture.fs;
        let scope = scope(OpId(1), ScopePurpose::Operation);
        let tree = fs.path("vol-a/tree");
        std::fs::create_dir_all(tree.as_path().join("sub")).unwrap();
        std::fs::write(tree.as_path().join("sub/f"), b"12345").unwrap();
        crate::store::test_support::symlink("sub/f", tree.as_path().join("link")).unwrap();
        let copy = fs.path("vol-b/tree");
        assert_eq!(copy_entry(fs, &scope, &tree, &copy), Ok(5));
        assert_eq!(
            std::fs::read(copy.as_path().join("sub/f")).unwrap(),
            b"12345"
        );
        assert!(fs.read_link(&copy.join_name("link").unwrap()).is_ok());
        remove_entry(fs, &scope, &copy).unwrap();
        assert!(!fixture.exists(&copy));
        assert_eq!(remove_entry(fs, &scope, &copy), Ok(()));
        let file = tree.join_name("link").unwrap();
        remove_entry(fs, &scope, &file).unwrap();
        assert_eq!(discard(fs, &scope, &file), Ok(()));
        fs.fail("remove_file", "sub", FsErrorKind::Io);
        assert!(discard(fs, &scope, &fs.path("vol-a/tree/sub/f")).is_err());
    }

    #[test]
    fn metadata_and_streams_are_written_completely() {
        let fixture = Fixture::new();
        let fs = &fixture.fs;
        let scope = scope(OpId(1), ScopePurpose::Maintenance);
        let source = fs.path("vol-a/a.txt");
        fixture.write(&source, &vec![7_u8; COPY_BUFFER + 3]);
        let meta = fs.extended_metadata(&source, Follow::NoFollow).unwrap();
        let path = fs.path("vol-a/a.meta");
        write_meta(fs, &scope, &fs.mount(0), &path, &meta).unwrap();
        assert_eq!(manifest::read_meta(fs, &path).unwrap(), meta);
        let mut dest = fs.create_new(&scope, &fs.path("vol-a/b.txt")).unwrap();
        let mut hasher = sha2::Sha256::default();
        let copied = stream_copy(fs, &source, dest.as_mut(), Some(&mut hasher)).unwrap();
        assert_eq!(copied, (COPY_BUFFER + 3) as u64);
        use sha2::Digest;
        let digest: [u8; 32] = hasher.finalize().into();
        assert_eq!(fs.content_hash(&source).unwrap().0, digest);
    }
}
