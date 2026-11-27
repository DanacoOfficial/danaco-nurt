// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-fs/tests/common/mod.rs
// Responsibility:   Fixtures shared by the Windows back end tests: temporary directories.
// Dependencies:     nurt_contracts, tempfile, windows_sys
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Fixtures shared by the Windows back end tests: temporary directories,
//! handles that hold files against deletion, security descriptors in
//! self-relative form and privileges of the process token.

#![allow(
    dead_code,
    reason = "every test binary uses a subset of the shared fixtures"
)]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "a failing fixture fails the test that uses it"
)]

use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::os::windows::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use nurt_contracts::fs::{
    AbsPath, Durability, ExtendedMeta, Follow, Fs, FsErrorKind, FsWrite, Holder, MetaChange,
    ScopePurpose, TxScope,
};
use nurt_contracts::journal::OpId;
use nurt_fs::windows::WindowsFs;
use nurt_fs::windows::security::SecurityDescriptor;
use tempfile::TempDir;
use windows_sys::Win32::Storage::FileSystem::{FILE_SHARE_READ, FILE_SHARE_WRITE};

/// Environment variable naming a directory on a `ReFS` volume; the tests that
/// need block cloning are ignored by default and fail when it is unset.
pub(crate) const REFS_DIR_VARIABLE: &str = "NURT_TEST_REFS_DIR";

/// Permission bits passed to `create_dir`; Windows ignores them.
pub(crate) const DIR_MODE: u32 = 0o755;

#[expect(
    clippy::disallowed_methods,
    reason = "the tests act as the journal that owns the transaction"
)]
pub(crate) fn scope() -> TxScope {
    TxScope::new_for_journal(OpId(1), ScopePurpose::Operation)
}

/// Empty temporary directory with a back end and a transaction scope.
pub(crate) struct Fixture {
    _dir: TempDir,
    pub(crate) root: AbsPath,
    pub(crate) fs: WindowsFs,
    pub(crate) scope: TxScope,
}

/// Fixture in the temporary directory of the account, with the default
/// retry period.
pub(crate) fn fixture() -> Fixture {
    fixture_with(WindowsFs::new())
}

/// Fixture in the temporary directory of the account with the given back end.
pub(crate) fn fixture_with(fs: WindowsFs) -> Fixture {
    fixture_at(tempfile::tempdir().expect("temporary directory"), fs)
}

/// Fixture in a new directory below `parent`.
pub(crate) fn fixture_in(parent: &Path) -> Fixture {
    let dir = tempfile::tempdir_in(parent).expect("temporary directory below the parent");
    fixture_at(dir, WindowsFs::new())
}

/// The root is the real path of the directory, so that short (8.3) names in
/// the temporary directory of the account do not differ from the names the
/// back end reports.
fn fixture_at(dir: TempDir, fs: WindowsFs) -> Fixture {
    let given = AbsPath::new(dir.path()).expect("absolute temporary directory");
    let root = fs
        .real_path(&given)
        .expect("real path of the temporary directory");
    Fixture {
        _dir: dir,
        root,
        fs,
        scope: scope(),
    }
}

/// Directory named by [`REFS_DIR_VARIABLE`].
pub(crate) fn refs_dir() -> PathBuf {
    std::env::var_os(REFS_DIR_VARIABLE)
        .map(PathBuf::from)
        .unwrap_or_else(|| panic!("set {REFS_DIR_VARIABLE} to a directory on a ReFS volume"))
}

impl Fixture {
    pub(crate) fn path(&self, name: &str) -> AbsPath {
        self.root.join_name(name).expect("valid file name")
    }

    /// Creates the file `name` with `content` through the back end.
    pub(crate) fn file(&self, name: &str, content: &[u8]) -> AbsPath {
        let path = self.path(name);
        self.write_new(&path, content);
        path
    }

    pub(crate) fn write_new(&self, path: &AbsPath, content: &[u8]) {
        let mut handle = self.fs.create_new(&self.scope, path).expect("create_new");
        handle.write_all(content).expect("write");
        handle.sync(Durability::Full).expect("sync");
    }

    pub(crate) fn dir(&self, name: &str) -> AbsPath {
        let path = self.path(name);
        self.fs
            .create_dir(&self.scope, &path, DIR_MODE)
            .expect("create_dir");
        path
    }

    pub(crate) fn read(&self, path: &AbsPath) -> Vec<u8> {
        let mut handle = self.fs.open_read(path).expect("open_read");
        let mut content = Vec::new();
        handle.read_to_end(&mut content).expect("read");
        content
    }

    pub(crate) fn read_stream(&self, path: &AbsPath, stream: &str) -> Vec<u8> {
        let mut handle = self
            .fs
            .open_stream_read(path, stream)
            .expect("open_stream_read");
        let mut content = Vec::new();
        handle.read_to_end(&mut content).expect("read stream");
        content
    }

    pub(crate) fn write_stream(&self, path: &AbsPath, stream: &str, content: &[u8]) {
        let mut handle = self
            .fs
            .open_stream_write(&self.scope, path, stream)
            .expect("open_stream_write");
        handle.write_all(content).expect("write stream");
        handle.sync(Durability::Full).expect("sync stream");
    }

    /// Whether the entry exists; errors other than a missing entry fail the
    /// test.
    pub(crate) fn exists(&self, path: &AbsPath) -> bool {
        match self.fs.metadata(path, Follow::NoFollow) {
            Ok(_) => true,
            Err(error) if error.kind == FsErrorKind::NotFound => false,
            Err(error) => panic!("metadata of {path}: {error}"),
        }
    }

    pub(crate) fn extended(&self, path: &AbsPath) -> ExtendedMeta {
        self.fs
            .extended_metadata(path, Follow::NoFollow)
            .expect("extended_metadata")
    }

    /// Security descriptor of the entry in self-relative form.
    pub(crate) fn descriptor(&self, path: &AbsPath) -> Vec<u8> {
        self.extended(path)
            .acl
            .expect("security descriptor of an NTFS entry")
    }

    /// Account SID of the entry in binary form.
    pub(crate) fn account(&self, path: &AbsPath) -> Vec<u8> {
        let bytes = self.descriptor(path);
        let descriptor = SecurityDescriptor::parse(&bytes).expect("valid descriptor");
        descriptor.owner().expect("account SID").to_vec()
    }

    /// Writes the elements present in `descriptor` to the entry.
    pub(crate) fn set_descriptor(&self, path: &AbsPath, descriptor: Vec<u8>) {
        let change = MetaChange {
            acl: Some(descriptor),
            ..MetaChange::default()
        };
        self.fs
            .set_meta(&self.scope, path, &change)
            .unwrap_or_else(|error| panic!("set_meta with a descriptor on {path}: {error}"));
    }
}

/// Opens `path` for reading without `FILE_SHARE_DELETE`; while the handle is
/// open the file cannot be renamed, replaced or removed.
pub(crate) fn hold_without_delete_sharing(path: &AbsPath) -> File {
    OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .open(path.as_path())
        .expect("open without delete sharing")
}

/// Opens `path` for reading with the default sharing of the standard library
/// (read, write and delete).
pub(crate) fn hold_with_delete_sharing(path: &AbsPath) -> File {
    File::open(path.as_path()).expect("open with delete sharing")
}

/// Fails unless the Restart Manager reported the running test process.
pub(crate) fn assert_held_by_this_process(holders: &[Holder]) {
    let pid = std::process::id();
    let holder = holders
        .iter()
        .find(|holder| holder.pid == pid)
        .unwrap_or_else(|| panic!("process {pid} missing from holders {holders:?}"));
    assert!(!holder.name.is_empty(), "holder without a name: {holder:?}");
}

/// Self-relative security descriptors built byte by byte.
pub(crate) mod descriptor {
    use nurt_fs::windows::security::control::{
        SE_DACL_PRESENT, SE_DACL_PROTECTED, SE_SACL_PRESENT, SE_SELF_RELATIVE,
    };

    const DESCRIPTOR_REVISION: u8 = 1;
    const ACL_REVISION: u8 = 2;
    const DESCRIPTOR_HEADER_LEN: usize = 20;
    const ACL_HEADER_LEN: usize = 8;
    /// Type, flags, size and access mask of an ACE that precede its SID.
    const ACE_FIXED_LEN: usize = 8;
    const OWNER_OFFSET_AT: usize = 4;
    const SACL_OFFSET_AT: usize = 12;
    const DACL_OFFSET_AT: usize = 16;

    /// `ACCESS_ALLOWED_ACE_TYPE`.
    pub(crate) const ACCESS_ALLOWED: u8 = 0;
    /// `SYSTEM_AUDIT_ACE_TYPE`.
    pub(crate) const SYSTEM_AUDIT: u8 = 2;
    /// `SUCCESSFUL_ACCESS_ACE_FLAG`.
    pub(crate) const SUCCESSFUL_ACCESS: u8 = 0x40;
    /// `FILE_ALL_ACCESS`.
    pub(crate) const FILE_ALL_ACCESS: u32 = 0x001F_01FF;
    /// `FILE_GENERIC_READ`.
    pub(crate) const FILE_GENERIC_READ: u32 = 0x0012_0089;
    /// `FILE_GENERIC_WRITE`.
    pub(crate) const FILE_GENERIC_WRITE: u32 = 0x0012_0116;

    /// `S-1-5-18`, the local system account.
    pub(crate) const LOCAL_SYSTEM: &[u8] = &[1, 1, 0, 0, 0, 0, 0, 5, 18, 0, 0, 0];
    /// Text form of [`LOCAL_SYSTEM`].
    pub(crate) const LOCAL_SYSTEM_TEXT: &str = "S-1-5-18";
    /// `S-1-1-0`, everyone.
    pub(crate) const EVERYONE: &[u8] = &[1, 1, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0];

    /// Access control entry with a binary SID.
    #[derive(Debug, Clone, Copy)]
    pub(crate) struct Ace<'a> {
        pub(crate) kind: u8,
        pub(crate) flags: u8,
        pub(crate) mask: u32,
        pub(crate) sid: &'a [u8],
    }

    /// Entry that grants `mask` to `sid` without inheritance flags.
    pub(crate) fn allow(sid: &[u8], mask: u32) -> Ace<'_> {
        Ace {
            kind: ACCESS_ALLOWED,
            flags: 0,
            mask,
            sid,
        }
    }

    /// Entry that audits successful `mask` access by `sid`.
    pub(crate) fn audit(sid: &[u8], mask: u32) -> Ace<'_> {
        Ace {
            kind: SYSTEM_AUDIT,
            flags: SUCCESSFUL_ACCESS,
            mask,
            sid,
        }
    }

    /// Descriptor with the chosen elements; absent elements are left
    /// unchanged by `set_meta`.
    #[derive(Debug, Clone, Default)]
    pub(crate) struct Builder<'a> {
        owner: Option<&'a [u8]>,
        dacl: Option<(Vec<Ace<'a>>, bool)>,
        sacl: Option<Vec<Ace<'a>>>,
    }

    impl<'a> Builder<'a> {
        pub(crate) fn owner(mut self, sid: &'a [u8]) -> Self {
            self.owner = Some(sid);
            self
        }

        pub(crate) fn dacl(mut self, aces: Vec<Ace<'a>>, protected: bool) -> Self {
            self.dacl = Some((aces, protected));
            self
        }

        pub(crate) fn sacl(mut self, aces: Vec<Ace<'a>>) -> Self {
            self.sacl = Some(aces);
            self
        }

        pub(crate) fn build(&self) -> Vec<u8> {
            let mut out = vec![0_u8; DESCRIPTOR_HEADER_LEN];
            out[0] = DESCRIPTOR_REVISION;
            let mut control = SE_SELF_RELATIVE;
            if let Some(sid) = self.owner {
                set_offset(&mut out, OWNER_OFFSET_AT);
                out.extend_from_slice(sid);
            }
            if let Some(aces) = &self.sacl {
                control |= SE_SACL_PRESENT;
                set_offset(&mut out, SACL_OFFSET_AT);
                out.extend(acl(aces));
            }
            if let Some((aces, protected)) = &self.dacl {
                control |= SE_DACL_PRESENT;
                if *protected {
                    control |= SE_DACL_PROTECTED;
                }
                set_offset(&mut out, DACL_OFFSET_AT);
                out.extend(acl(aces));
            }
            out[2..4].copy_from_slice(&control.to_le_bytes());
            out
        }
    }

    /// Records the current end of `out` as the offset stored at `at`.
    fn set_offset(out: &mut [u8], at: usize) {
        let offset = u32::try_from(out.len()).expect("descriptor below 4 GiB");
        out[at..at + 4].copy_from_slice(&offset.to_le_bytes());
    }

    /// ACL with the given entries; SIDs are multiples of four bytes, so every
    /// entry stays aligned.
    fn acl(aces: &[Ace<'_>]) -> Vec<u8> {
        let mut body = Vec::new();
        for ace in aces {
            let size = u16::try_from(ACE_FIXED_LEN + ace.sid.len()).expect("ACE below 64 KiB");
            body.push(ace.kind);
            body.push(ace.flags);
            body.extend_from_slice(&size.to_le_bytes());
            body.extend_from_slice(&ace.mask.to_le_bytes());
            body.extend_from_slice(ace.sid);
        }
        let size = u16::try_from(ACL_HEADER_LEN + body.len()).expect("ACL below 64 KiB");
        let count = u16::try_from(aces.len()).expect("ACE count below 65536");
        let mut out = vec![ACL_REVISION, 0];
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(&count.to_le_bytes());
        out.extend_from_slice(&[0, 0]);
        out.extend(body);
        out
    }
}

/// Privileges of the process token.
#[allow(
    unsafe_code,
    reason = "token privileges are adjusted only through Win32 calls"
)]
pub(crate) mod privilege {
    use std::mem::size_of;
    use std::ptr;

    use windows_sys::Win32::Foundation::{
        CloseHandle, ERROR_NOT_ALL_ASSIGNED, GetLastError, HANDLE, LUID,
    };
    use windows_sys::Win32::Security::{
        AdjustTokenPrivileges, LUID_AND_ATTRIBUTES, LookupPrivilegeValueW, SE_PRIVILEGE_ENABLED,
        TOKEN_ADJUST_PRIVILEGES, TOKEN_PRIVILEGES, TOKEN_QUERY,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    /// Assigns any account to a file and writes any descriptor element.
    pub(crate) const RESTORE: &str = "SeRestorePrivilege";
    /// Reads and writes system access control lists.
    pub(crate) const SECURITY: &str = "SeSecurityPrivilege";

    #[allow(
        clippy::cast_possible_truncation,
        reason = "the structure has a fixed size of 16 bytes"
    )]
    const TOKEN_PRIVILEGES_LEN: u32 = size_of::<TOKEN_PRIVILEGES>() as u32;

    struct Token(HANDLE);

    impl Drop for Token {
        fn drop(&mut self) {
            // SAFETY: the handle was opened by `OpenProcessToken` and is closed
            // once.
            unsafe { CloseHandle(self.0) };
        }
    }

    /// Privilege enabled in the process token; the previous state is
    /// restored on drop.
    pub(crate) struct Enabled {
        token: Token,
        previous: TOKEN_PRIVILEGES,
    }

    impl Drop for Enabled {
        fn drop(&mut self) {
            // SAFETY: the token handle is open with `TOKEN_ADJUST_PRIVILEGES`
            // and `previous` is the state returned when the privilege was
            // enabled; no previous state is requested.
            unsafe {
                AdjustTokenPrivileges(
                    self.token.0,
                    0,
                    &self.previous,
                    0,
                    ptr::null_mut(),
                    ptr::null_mut(),
                )
            };
        }
    }

    const fn no_luid() -> LUID {
        LUID {
            LowPart: 0,
            HighPart: 0,
        }
    }

    /// Enables the privilege `name`, or returns the Win32 error code when the
    /// account does not hold it.
    pub(crate) fn enable(name: &str) -> Result<Enabled, u32> {
        let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
        let mut luid = no_luid();
        // SAFETY: `wide` is terminated by zero and `luid` is writable.
        if unsafe { LookupPrivilegeValueW(ptr::null(), wide.as_ptr(), &mut luid) } == 0 {
            // SAFETY: reads the error of the failed call on this thread.
            return Err(unsafe { GetLastError() });
        }
        let mut handle: HANDLE = ptr::null_mut();
        // SAFETY: the pseudo handle of the current process is always valid and
        // `handle` is writable.
        let opened = unsafe {
            OpenProcessToken(
                GetCurrentProcess(),
                TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY,
                &mut handle,
            )
        };
        if opened == 0 {
            // SAFETY: reads the error of the failed call on this thread.
            return Err(unsafe { GetLastError() });
        }
        let token = Token(handle);
        let requested = TOKEN_PRIVILEGES {
            PrivilegeCount: 1,
            Privileges: [LUID_AND_ATTRIBUTES {
                Luid: luid,
                Attributes: SE_PRIVILEGE_ENABLED,
            }],
        };
        let mut previous = TOKEN_PRIVILEGES {
            PrivilegeCount: 0,
            Privileges: [LUID_AND_ATTRIBUTES {
                Luid: no_luid(),
                Attributes: 0,
            }],
        };
        let mut returned = 0_u32;
        // SAFETY: the token is open with `TOKEN_ADJUST_PRIVILEGES`, `requested`
        // holds one entry and `previous` is writable for
        // `TOKEN_PRIVILEGES_LEN` bytes, enough for one entry.
        let adjusted = unsafe {
            AdjustTokenPrivileges(
                token.0,
                0,
                &requested,
                TOKEN_PRIVILEGES_LEN,
                &mut previous,
                &mut returned,
            )
        };
        // SAFETY: reads the error of the call above on this thread; it is
        // `ERROR_NOT_ALL_ASSIGNED` when the account lacks the privilege.
        let code = unsafe { GetLastError() };
        if adjusted == 0 || code == ERROR_NOT_ALL_ASSIGNED {
            return Err(code);
        }
        Ok(Enabled { token, previous })
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
