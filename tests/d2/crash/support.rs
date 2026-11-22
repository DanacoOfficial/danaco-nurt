// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Fixtures shared by the crash tests.

use std::io::{Read, Write};

use nurt_contracts::fs::{
    AbsPath, Durability, Follow, Fs, FsResult, FsWrite, ScopePurpose, TxScope,
};
use nurt_contracts::journal::OpId;
use nurt_crashfs::crash::{CrashFs, CrashProfile, VolumeSpec};

/// Number of seeds tried by tests that look for a possible crash outcome.
pub const SEEDS: u64 = 64;

/// Root of the first simulated volume.
pub fn volume_root() -> AbsPath {
    let text = if cfg!(windows) { "C:\\vol" } else { "/vol" };
    AbsPath::new(text).unwrap()
}

/// Root of the second simulated volume.
pub fn second_root() -> AbsPath {
    let text = if cfg!(windows) { "D:\\vol" } else { "/other" };
    AbsPath::new(text).unwrap()
}

/// Path of `name` in the first volume root.
pub fn at(name: &str) -> AbsPath {
    volume_root().join_name(name).unwrap()
}

/// Scope accepted by every mutation of the model.
pub fn scope() -> TxScope {
    TxScope::new_for_journal(OpId(1), ScopePurpose::Maintenance)
}

/// File system with one volume of default capacity.
pub fn new_fs(profile: CrashProfile, seed: u64) -> CrashFs {
    CrashFs::new(profile, seed, vec![VolumeSpec::new(volume_root())]).unwrap()
}

/// Creates `path` with `content`, flushes the file and its directory.
pub fn durable_file(fs: &CrashFs, path: &AbsPath, content: &[u8]) {
    let mut file = fs.create_new(&scope(), path).unwrap();
    file.write_all(content).unwrap();
    file.sync(Durability::Full).unwrap();
    fs.sync_dir(&scope(), &path.parent().unwrap()).unwrap();
}

/// Content of `path`, or `None` when it does not exist.
pub fn content(fs: &dyn Fs, path: &AbsPath) -> Option<Vec<u8>> {
    let mut handle = fs.open_read(path).ok()?;
    let mut bytes = Vec::new();
    handle.read_to_end(&mut bytes).unwrap();
    Some(bytes)
}

/// Whether `path` exists, links not followed.
pub fn exists(fs: &dyn Fs, path: &AbsPath) -> bool {
    fs.metadata(path, Follow::NoFollow).is_ok()
}

/// Writes `content` over the start of an existing file without flushing.
pub fn overwrite(fs: &CrashFs, path: &AbsPath, content: &[u8]) -> FsResult<()> {
    let mut file = fs.open_in_place(&scope(), path)?;
    file.write_all(content).map_err(|err| {
        err.into_inner()
            .and_then(|inner| inner.downcast::<nurt_contracts::fs::FsError>().ok())
            .map_or_else(
                || {
                    nurt_contracts::fs::FsError::new(
                        nurt_contracts::fs::FsErrorKind::Io,
                        "write",
                        None,
                    )
                },
                |inner| *inner,
            )
    })
}
