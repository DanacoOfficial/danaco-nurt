// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! File system of a process that runs with elevated rights and keeps the
//! journal of an ordinary account (Linux).
//!
//! The elevated process changes targets with the rights of the administrator
//! and writes the journal of the account that asked for the change. The
//! journal belongs to the account: every file and directory below it must be
//! created with the user and the group of the account, and the elevated
//! process must not follow a link that the account placed there with more
//! rights than the account has.
//!
//! [`AccountFs`] gets both from the file system user and group (`fsuid`,
//! `fsgid`) of the calling thread. A call that concerns a path of the account
//! (the journal directories `.nurt-journal` and the state directory of the
//! account) runs with the `fsuid` and `fsgid` of the account, so the kernel
//! checks every component of the path, a link included, with the permissions
//! of the account and creates files that the account owns; the other calls run
//! with the rights of the process. A copy from a target into the journal
//! reads the target with the rights of the process and writes the copy with
//! those of the account, block by block. Operations that would leave a file
//! of the target in the journal under its old owner (hard link, clone,
//! rename into the journal) are not offered; the journal then copies.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use nurt_contracts::fs::{
    AbsPath, ContentHash, DirEntry, Durability, ExtendedMeta, FileLock, FileMeta, Follow, Fs,
    FsError, FsErrorKind, FsResult, FsWrite, Holder, LinkKind, MetaChange, MetaTransferReport,
    PathKey, ReadHandle, RemoveStats, RenameMode, SpaceInfo, SpecialNode, TxScope, VolumeId,
    VolumeInfo, WriteHandle,
};

/// Name that marks a journal directory.
const JOURNAL_DIR: &str = ".nurt-journal";

/// User and group of an account, and the state directory of the account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountIdentity {
    /// Numeric user.
    pub uid: u32,
    /// Numeric group.
    pub gid: u32,
    /// State directory of the account.
    pub state_dir: AbsPath,
}

/// Switches the file system user and group of the calling thread and
/// restores them when it is dropped.
struct FsIdentity {
    uid: u32,
    gid: u32,
    previous: Option<(u32, u32)>,
}

impl FsIdentity {
    fn enter(uid: u32, gid: u32) -> Self {
        // SAFETY: `setfsgid` and `setfsuid` take plain numbers, have no
        // memory effects and return the previous value.
        let old_gid = unsafe { libc::setfsgid(gid) };
        // SAFETY: as above.
        let old_uid = unsafe { libc::setfsuid(uid) };
        Self {
            uid,
            gid,
            previous: Some((
                u32::try_from(old_uid).unwrap_or(0),
                u32::try_from(old_gid).unwrap_or(0),
            )),
        }
    }
}

impl Drop for FsIdentity {
    fn drop(&mut self) {
        if let Some((uid, gid)) = self.previous.take() {
            // SAFETY: plain numbers, no memory effects.
            unsafe {
                libc::setfsuid(uid);
                libc::setfsgid(gid);
            }
            let _ = (self.uid, self.gid);
        }
    }
}

/// Runs `f` with the file system identity of `identity`.
pub fn as_account<T>(identity: &AccountIdentity, f: impl FnOnce() -> T) -> T {
    let _guard = FsIdentity::enter(identity.uid, identity.gid);
    f()
}

/// Whether `path` lies in the state directory of the account or in a journal
/// directory.
fn is_account_path(identity: &AccountIdentity, path: &AbsPath) -> bool {
    path.starts_with(&identity.state_dir)
        || path
            .as_path()
            .components()
            .any(|part| part.as_os_str().to_string_lossy().starts_with(JOURNAL_DIR))
}

/// The file system of the elevated process with the journal of an account.
pub struct AccountFs {
    inner: Arc<dyn FsWrite>,
    identity: AccountIdentity,
}

impl std::fmt::Debug for AccountFs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AccountFs")
            .field("identity", &self.identity)
            .finish_non_exhaustive()
    }
}

impl AccountFs {
    /// Wraps `inner` for the account `identity`.
    #[must_use]
    pub fn new(inner: Arc<dyn FsWrite>, identity: AccountIdentity) -> Self {
        Self { inner, identity }
    }

    fn mine(&self, path: &AbsPath) -> bool {
        is_account_path(&self.identity, path)
    }

    /// Runs `f` as the account when `path` is the path of the account.
    fn at<T>(&self, path: &AbsPath, f: impl FnOnce() -> T) -> T {
        if self.mine(path) {
            as_account(&self.identity, f)
        } else {
            f()
        }
    }

    fn unsupported(op: &str, path: &AbsPath) -> FsError {
        FsError::new(FsErrorKind::Unsupported, op, Some(path.as_path()))
    }
}

impl Fs for AccountFs {
    fn metadata(&self, path: &AbsPath, follow: Follow) -> FsResult<FileMeta> {
        self.at(path, || self.inner.metadata(path, follow))
    }

    fn special_node(&self, path: &AbsPath) -> FsResult<Option<SpecialNode>> {
        self.at(path, || self.inner.special_node(path))
    }

    fn extended_metadata(&self, path: &AbsPath, follow: Follow) -> FsResult<ExtendedMeta> {
        self.at(path, || self.inner.extended_metadata(path, follow))
    }

    fn open_read(&self, path: &AbsPath) -> FsResult<Box<dyn ReadHandle>> {
        self.at(path, || self.inner.open_read(path))
    }

    fn open_stream_read(&self, path: &AbsPath, stream: &str) -> FsResult<Box<dyn ReadHandle>> {
        self.at(path, || self.inner.open_stream_read(path, stream))
    }

    fn read_dir<'a>(
        &'a self,
        path: &AbsPath,
    ) -> FsResult<Box<dyn Iterator<Item = FsResult<DirEntry>> + Send + 'a>> {
        self.at(path, || self.inner.read_dir(path))
    }

    fn read_link(&self, path: &AbsPath) -> FsResult<PathBuf> {
        self.at(path, || self.inner.read_link(path))
    }

    fn real_path(&self, path: &AbsPath) -> FsResult<AbsPath> {
        self.at(path, || self.inner.real_path(path))
    }

    fn content_hash(&self, path: &AbsPath) -> FsResult<ContentHash> {
        self.at(path, || self.inner.content_hash(path))
    }

    fn volume(&self, path: &AbsPath) -> FsResult<VolumeInfo> {
        self.inner.volume(path)
    }

    fn space(&self, volume: &VolumeId) -> FsResult<SpaceInfo> {
        self.inner.space(volume)
    }

    fn path_key(&self, path: &AbsPath) -> FsResult<PathKey> {
        self.inner.path_key(path)
    }

    fn holders(&self, path: &AbsPath) -> FsResult<Vec<Holder>> {
        self.inner.holders(path)
    }

    /// The journal is created by the account, so the directory must be
    /// writable for the account, not for the administrator.
    fn can_write_dir(&self, dir: &AbsPath) -> FsResult<bool> {
        as_account(&self.identity, || self.inner.can_write_dir(dir))
    }
}

/// Error of a call made through the standard library.
fn io_error(error: &std::io::Error, op: &str, path: &Path) -> FsError {
    let kind = match error.kind() {
        std::io::ErrorKind::NotFound => FsErrorKind::NotFound,
        std::io::ErrorKind::PermissionDenied => FsErrorKind::PermissionDenied,
        std::io::ErrorKind::AlreadyExists => FsErrorKind::Exists,
        _ => FsErrorKind::Io,
    };
    let mut failure = FsError::new(kind, op, Some(path));
    failure.os_code = error.raw_os_error();
    failure
}

/// Block size of a copy from a target into the journal.
const COPY_BLOCK: usize = 256 * 1024;

impl FsWrite for AccountFs {
    fn create_temp_in(
        &self,
        scope: &TxScope,
        dir: &AbsPath,
        name_hint: &str,
    ) -> FsResult<Box<dyn WriteHandle>> {
        self.at(dir, || self.inner.create_temp_in(scope, dir, name_hint))
    }

    fn create_new(&self, scope: &TxScope, path: &AbsPath) -> FsResult<Box<dyn WriteHandle>> {
        self.at(path, || self.inner.create_new(scope, path))
    }

    fn open_in_place(&self, scope: &TxScope, path: &AbsPath) -> FsResult<Box<dyn WriteHandle>> {
        self.at(path, || self.inner.open_in_place(scope, path))
    }

    fn open_stream_write(
        &self,
        scope: &TxScope,
        path: &AbsPath,
        stream: &str,
    ) -> FsResult<Box<dyn WriteHandle>> {
        self.at(path, || self.inner.open_stream_write(scope, path, stream))
    }

    fn rename(
        &self,
        scope: &TxScope,
        from: &AbsPath,
        to: &AbsPath,
        mode: RenameMode,
    ) -> FsResult<()> {
        match (self.mine(from), self.mine(to)) {
            (true, true) => as_account(&self.identity, || self.inner.rename(scope, from, to, mode)),
            (false, false) => self.inner.rename(scope, from, to, mode),
            _ => Err(Self::unsupported("rename", to)),
        }
    }

    fn exchange(&self, scope: &TxScope, a: &AbsPath, b: &AbsPath) -> FsResult<()> {
        match (self.mine(a), self.mine(b)) {
            (true, true) => as_account(&self.identity, || self.inner.exchange(scope, a, b)),
            (false, false) => self.inner.exchange(scope, a, b),
            _ => Err(Self::unsupported("exchange", b)),
        }
    }

    fn replace_with_backup(
        &self,
        scope: &TxScope,
        target: &AbsPath,
        replacement: &AbsPath,
        backup: &AbsPath,
    ) -> FsResult<()> {
        if self.mine(target) || self.mine(replacement) || self.mine(backup) {
            if self.mine(target) && self.mine(replacement) && self.mine(backup) {
                return as_account(&self.identity, || {
                    self.inner
                        .replace_with_backup(scope, target, replacement, backup)
                });
            }
            return Err(Self::unsupported("replace_with_backup", target));
        }
        self.inner
            .replace_with_backup(scope, target, replacement, backup)
    }

    fn hard_link(&self, scope: &TxScope, existing: &AbsPath, new: &AbsPath) -> FsResult<()> {
        match (self.mine(existing), self.mine(new)) {
            (true, true) => as_account(&self.identity, || {
                self.inner.hard_link(scope, existing, new)
            }),
            (false, false) => self.inner.hard_link(scope, existing, new),
            _ => Err(Self::unsupported("hard_link", new)),
        }
    }

    fn clone_file(&self, scope: &TxScope, src: &AbsPath, dst: &AbsPath) -> FsResult<()> {
        match (self.mine(src), self.mine(dst)) {
            (true, true) => as_account(&self.identity, || self.inner.clone_file(scope, src, dst)),
            (false, false) => self.inner.clone_file(scope, src, dst),
            _ => Err(Self::unsupported("clone_file", dst)),
        }
    }

    fn copy_file(&self, scope: &TxScope, src: &AbsPath, dst: &AbsPath) -> FsResult<u64> {
        match (self.mine(src), self.mine(dst)) {
            (true, true) => as_account(&self.identity, || self.inner.copy_file(scope, src, dst)),
            (false, false) => self.inner.copy_file(scope, src, dst),
            (false, true) => {
                let mut from = self.inner.open_read(src)?;
                let mut to = self.at(dst, || self.inner.create_new(scope, dst))?;
                let mut block = vec![0_u8; COPY_BLOCK];
                let mut total = 0_u64;
                loop {
                    let read = from
                        .read(&mut block)
                        .map_err(|error| io_error(&error, "copy_file", src.as_path()))?;
                    if read == 0 {
                        break;
                    }
                    as_account(&self.identity, || to.write_all(&block[..read]))
                        .map_err(|error| io_error(&error, "copy_file", dst.as_path()))?;
                    total += read as u64;
                }
                as_account(&self.identity, || to.sync(Durability::Full))?;
                Ok(total)
            }
            (true, false) => {
                let mut from = self.at(src, || self.inner.open_read(src))?;
                let mut to = self.inner.create_new(scope, dst)?;
                std::io::copy(&mut from, &mut to)
                    .map_err(|error| io_error(&error, "copy_file", dst.as_path()))
            }
        }
    }

    fn remove_file(&self, scope: &TxScope, path: &AbsPath) -> FsResult<()> {
        self.at(path, || self.inner.remove_file(scope, path))
    }

    fn remove_dir(&self, scope: &TxScope, path: &AbsPath) -> FsResult<()> {
        self.at(path, || self.inner.remove_dir(scope, path))
    }

    fn remove_tree(
        &self,
        scope: &TxScope,
        path: &AbsPath,
        parallelism: usize,
    ) -> FsResult<RemoveStats> {
        self.at(path, || self.inner.remove_tree(scope, path, parallelism))
    }

    fn create_dir(&self, scope: &TxScope, path: &AbsPath, mode: u32) -> FsResult<()> {
        self.at(path, || self.inner.create_dir(scope, path, mode))
    }

    fn create_node(
        &self,
        scope: &TxScope,
        path: &AbsPath,
        node: SpecialNode,
        mode: Option<u32>,
    ) -> FsResult<()> {
        self.at(path, || self.inner.create_node(scope, path, node, mode))
    }

    fn create_link(
        &self,
        scope: &TxScope,
        target: &Path,
        link: &AbsPath,
        kind: LinkKind,
    ) -> FsResult<()> {
        self.at(link, || self.inner.create_link(scope, target, link, kind))
    }

    fn set_meta(&self, scope: &TxScope, path: &AbsPath, change: &MetaChange) -> FsResult<()> {
        self.at(path, || self.inner.set_meta(scope, path, change))
    }

    fn restore_meta(
        &self,
        scope: &TxScope,
        path: &AbsPath,
        meta: &ExtendedMeta,
    ) -> FsResult<MetaTransferReport> {
        self.at(path, || self.inner.restore_meta(scope, path, meta))
    }

    fn truncate(&self, scope: &TxScope, path: &AbsPath, length: u64) -> FsResult<()> {
        self.at(path, || self.inner.truncate(scope, path, length))
    }

    fn sync_dir(&self, scope: &TxScope, dir: &AbsPath) -> FsResult<()> {
        self.at(dir, || self.inner.sync_dir(scope, dir))
    }

    fn lock_exclusive(&self, path: &AbsPath, timeout: Duration) -> FsResult<Box<dyn FileLock>> {
        self.at(path, || self.inner.lock_exclusive(path, timeout))
    }

    fn mark_hidden(&self, scope: &TxScope, path: &AbsPath) -> FsResult<()> {
        self.at(path, || self.inner.mark_hidden(scope, path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity() -> AccountIdentity {
        AccountIdentity {
            uid: 1000,
            gid: 1000,
            state_dir: AbsPath::new("/home/ann/.local/state/nurt").unwrap(),
        }
    }

    #[test]
    fn journal_directories_and_the_state_directory_belong_to_the_account() {
        let who = identity();
        let path = |text: &str| AbsPath::new(text).unwrap();
        assert!(is_account_path(&who, &path("/.nurt-journal/ops/1/op.json")));
        assert!(is_account_path(&who, &path("/home/ann/.nurt-journal")));
        assert!(is_account_path(
            &who,
            &path("/home/ann/.local/state/nurt/user-config.toml")
        ));
        assert!(!is_account_path(&who, &path("/etc/hosts")));
        assert!(!is_account_path(&who, &path("/etc/.nurt-tmp-1-1")));
    }

    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    #[allow(clippy::disallowed_methods, reason = "tests stand in for the journal")]
    fn scope() -> TxScope {
        TxScope::new_for_journal(
            nurt_contracts::journal::OpId(1),
            nurt_contracts::fs::ScopePurpose::Maintenance,
        )
    }

    fn chown(path: &Path, uid: u32, gid: u32) {
        let text = std::ffi::CString::new(path.to_str().unwrap()).unwrap();
        // SAFETY: `text` is a valid C string that lives through the call.
        assert_eq!(unsafe { libc::chown(text.as_ptr(), uid, gid) }, 0);
    }

    #[test]
    #[ignore = "needs root"]
    fn the_copy_of_a_root_only_file_is_owned_by_the_account_and_links_are_not_followed() {
        const ACCOUNT: u32 = 54_321;
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
        let state = dir.path().join("state");
        std::fs::create_dir(&state).unwrap();
        chown(&state, ACCOUNT, ACCOUNT);
        let secret = dir.path().join("secret");
        std::fs::write(&secret, b"root only").unwrap();
        std::fs::set_permissions(&secret, std::fs::Permissions::from_mode(0o600)).unwrap();
        let who = AccountIdentity {
            uid: ACCOUNT,
            gid: ACCOUNT,
            state_dir: AbsPath::new(&state).unwrap(),
        };
        let fs = AccountFs::new(crate::system_fs(), who);
        let journal = AbsPath::new(state.join(".nurt-journal")).unwrap();
        fs.create_dir(&scope(), &journal, 0o700).unwrap();
        let copy = journal.join_name("blob").unwrap();
        let source = AbsPath::new(&secret).unwrap();
        assert_eq!(fs.copy_file(&scope(), &source, &copy).unwrap(), 9);
        let meta = std::fs::metadata(copy.as_path()).unwrap();
        assert_eq!((meta.uid(), meta.gid()), (ACCOUNT, ACCOUNT));
        assert_eq!(std::fs::metadata(journal.as_path()).unwrap().uid(), ACCOUNT);

        // A link that the account puts in its journal to a directory that only
        // the administrator may write is not followed with more rights.
        let outside = dir.path().join("outside");
        std::fs::create_dir(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, state.join(".nurt-journal/link")).unwrap();
        let through = AbsPath::new(state.join(".nurt-journal/link/planted")).unwrap();
        assert!(fs.create_new(&scope(), &through).is_err());
        assert!(!outside.join("planted").exists());
    }
}
