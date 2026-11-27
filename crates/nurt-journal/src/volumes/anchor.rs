// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-journal::volumes::anchor
// Responsibility:   Selection of the journal anchor directory and creation of the journal.
// Dependencies:     nurt_contracts, nurt_testkit, serde
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Selection of the journal anchor directory and creation of the journal
//! directory with its markers and format files.
//!
//! Previous content is moved into the journal by renaming, which works only
//! within one mount of one file system (`EXDEV` otherwise). The anchor is
//! therefore a directory of the same volume and mount as the target: the
//! mount point when the account may write there, otherwise the highest
//! writable ancestor of the target on that volume.
//!
//! Directories of an overlay mount report the device of the overlay itself, so
//! the walk stays on the overlay mount and the journal directory is created in
//! its upper layer.

use std::io::{self, Write};

use nurt_contracts::env::Os;
use nurt_contracts::fs::{
    AbsPath, Durability, FileKind, Follow, Fs, FsError, FsErrorKind, FsFeatures, FsResult, FsWrite,
    RenameMode, TxScope, VolumeInfo,
};
use serde::Serialize;

use super::maintenance_scope;
use crate::layout::{DIR_NAME, DIR_NAME_MACOS, JournalLayout, VERSION_TEXT};

/// Operating system the program was built for.
pub(crate) const HOST_OS: Os = if cfg!(target_os = "macos") {
    Os::Macos
} else if cfg!(windows) {
    Os::Windows
} else {
    Os::Linux
};

/// Permission bits of the journal directory and its subdirectories: the
/// account only.
pub(crate) const JOURNAL_DIR_MODE: u32 = 0o700;

/// Cache directory tag (<https://bford.info/cachedir/>): backup and indexing
/// tools skip directories that contain it.
const CACHEDIR_TAG_NAME: &str = "CACHEDIR.TAG";

/// Content of the cache directory tag; the first line is the fixed signature.
pub(crate) const CACHEDIR_TAG: &str = "Signature: 8a477f597d28d172789f06886806bc55\n\
# This file is a cache directory tag created by Danaco Nurt.\n\
# For information about cache directory tags see https://bford.info/cachedir/\n";

/// Ignore rule that keeps the journal out of version control.
const GITIGNORE_NAME: &str = ".gitignore";
const GITIGNORE: &str = "*\n";

/// Suffix that excludes a directory from Spotlight indexing on macOS.
const APPLE_NOINDEX_SUFFIX: &str = ".noindex";

/// Empty marker files that exclude a directory from Spotlight indexing and
/// from iCloud synchronisation on macOS.
const APPLE_MARKERS: [&str; 2] = [".metadata_never_index", ".nosync"];

/// Name of the journal directory: the configured name, with the `.noindex`
/// suffix on macOS.
pub(crate) fn journal_dir_name(configured: &str, os: Os) -> String {
    if os != Os::Macos || configured.ends_with(APPLE_NOINDEX_SUFFIX) {
        configured.to_owned()
    } else if configured == DIR_NAME {
        DIR_NAME_MACOS.to_owned()
    } else {
        format!("{configured}{APPLE_NOINDEX_SUFFIX}")
    }
}

/// Marker files of the journal directory with their content.
fn markers(os: Os) -> Vec<(&'static str, &'static str)> {
    let mut markers = vec![
        (CACHEDIR_TAG_NAME, CACHEDIR_TAG),
        (GITIGNORE_NAME, GITIGNORE),
    ];
    if os == Os::Macos {
        markers.extend(APPLE_MARKERS.iter().map(|name| (*name, "")));
    }
    markers
}

/// Volume description kept in the journal directory (`vol.json`).
#[derive(Debug, Serialize)]
struct VolumeDescription<'a> {
    dev: u64,
    fs: &'a str,
    uuid: Option<&'a str>,
    subvolume: u64,
    case_sensitive: bool,
    anchor: &'a AbsPath,
    features: Vec<&'static str>,
}

impl<'a> VolumeDescription<'a> {
    fn new(volume: &'a VolumeInfo, anchor: &'a AbsPath) -> Self {
        Self {
            dev: volume.id.dev,
            fs: &volume.fs_type,
            uuid: volume.id.uuid.as_deref(),
            subvolume: volume.id.subvolume,
            case_sensitive: volume.features.case_sensitive,
            anchor,
            features: feature_names(&volume.features),
        }
    }
}

/// Names of the capabilities a volume has.
fn feature_names(features: &FsFeatures) -> Vec<&'static str> {
    [
        (features.exchange, "exchange"),
        (features.rename_noreplace, "rename_noreplace"),
        (features.replace_with_backup, "replace_with_backup"),
        (features.hardlink, "hardlink"),
        (features.reflink, "reflink"),
        (features.xattr, "xattr"),
        (features.acl, "acl"),
        (features.alternate_streams, "alternate_streams"),
        (features.durable_metadata, "durable_metadata"),
        (features.decomposes_names, "decomposes_names"),
    ]
    .into_iter()
    .filter_map(|(present, name)| present.then_some(name))
    .collect()
}

/// Selects the anchor of `volume` and makes sure the journal directory `name`
/// exists there; returns the journal directory, or `None` when the volume has
/// no usable anchor.
///
/// Candidates are tried in order: the mount point, then for each path of
/// `within` the path itself and its ancestors from the top. A candidate is
/// used when it is a directory (not a link) of the same volume and mount, the
/// account may write in it, and the journal directory in it is a writable
/// directory or can be created.
pub(crate) fn select(
    fs: &dyn FsWrite,
    volume: &VolumeInfo,
    within: &[AbsPath],
    name: &str,
    os: Os,
) -> FsResult<Option<AbsPath>> {
    for candidate in candidates(volume, within) {
        if !is_candidate(fs, &candidate, volume)? {
            continue;
        }
        match ensure_journal_dir(fs, &candidate, name, os, volume) {
            Ok(dir) => return Ok(Some(dir)),
            Err(error) if rejects_anchor(error.kind) => {}
            Err(error) => return Err(error),
        }
    }
    Ok(None)
}

/// Candidate anchors in the order of preference, without repetition.
fn candidates(volume: &VolumeInfo, within: &[AbsPath]) -> Vec<AbsPath> {
    let mut candidates = vec![volume.mount.clone()];
    for path in within {
        let mut chain: Vec<AbsPath> =
            std::iter::successors(Some(path.clone()), AbsPath::parent).collect();
        chain.reverse();
        for dir in chain {
            if !candidates.contains(&dir) {
                candidates.push(dir);
            }
        }
    }
    candidates
}

/// Errors that make a candidate unusable without making the volume unusable.
fn rejects_anchor(kind: FsErrorKind) -> bool {
    matches!(
        kind,
        FsErrorKind::PermissionDenied | FsErrorKind::NotADirectory | FsErrorKind::ReadOnly
    )
}

fn is_candidate(fs: &dyn Fs, dir: &AbsPath, volume: &VolumeInfo) -> FsResult<bool> {
    let meta = match fs.metadata(dir, Follow::NoFollow) {
        Ok(meta) => meta,
        Err(error) if is_absent(error.kind) => return Ok(false),
        Err(error) => return Err(error),
    };
    if meta.kind != FileKind::Directory {
        return Ok(false);
    }
    let found = fs.volume(dir)?;
    if found.id != volume.id || found.mount != volume.mount {
        return Ok(false);
    }
    fs.can_write_dir(dir)
}

fn is_absent(kind: FsErrorKind) -> bool {
    matches!(
        kind,
        FsErrorKind::NotFound | FsErrorKind::NotADirectory | FsErrorKind::PermissionDenied
    )
}

/// Creates the journal directory `name` in `anchor` with its markers, the
/// format version file, the volume description and the directories of the
/// journal layout, or completes an existing one; returns the journal
/// directory.
///
/// Every step tolerates a concurrent process doing the same: directories are
/// created with `Exists` accepted, and each file is written to a temporary
/// file and renamed without replacing, so a file is never seen partially
/// written and two processes never fail each other. Existing files are kept.
///
/// # Errors
///
/// Fails with [`FsErrorKind::NotADirectory`] when the journal name is taken by
/// a file or a link, with [`FsErrorKind::PermissionDenied`] when the journal
/// directory belongs to another account, and with the error of any failing
/// file system call.
pub(crate) fn ensure_journal_dir(
    fs: &dyn FsWrite,
    anchor: &AbsPath,
    name: &str,
    os: Os,
    volume: &VolumeInfo,
) -> FsResult<AbsPath> {
    let scope = maintenance_scope();
    let dir = anchor.join_name(name)?;
    if create_dir_once(fs, &scope, &dir)? {
        fs.sync_dir(&scope, anchor)?;
    }
    check_journal_dir(fs, &dir)?;
    fs.mark_hidden(&scope, &dir)?;
    for (marker, content) in markers(os) {
        write_file_once(fs, &scope, &dir.join_name(marker)?, content.as_bytes())?;
    }
    let layout = JournalLayout::new(dir.clone());
    write_file_once(fs, &scope, &layout.version()?, VERSION_TEXT.as_bytes())?;
    let description = serde_json::to_vec_pretty(&VolumeDescription::new(volume, anchor))
        .map_err(|_| FsError::new(FsErrorKind::Io, "encode", Some(dir.as_path())))?;
    write_file_once(fs, &scope, &layout.volume_file()?, &description)?;
    for subdir in layout.structure()? {
        create_dir_once(fs, &scope, &subdir)?;
    }
    fs.sync_dir(&scope, &dir)?;
    Ok(dir)
}

/// Creates a directory; returns whether it was created now.
fn create_dir_once(fs: &dyn FsWrite, scope: &TxScope, dir: &AbsPath) -> FsResult<bool> {
    match fs.create_dir(scope, dir, JOURNAL_DIR_MODE) {
        Ok(()) => Ok(true),
        Err(error) if error.kind == FsErrorKind::Exists => Ok(false),
        Err(error) => Err(error),
    }
}

/// Accepts an existing journal directory only when it is a real directory the
/// account may write in; a link planted under the journal name is refused.
fn check_journal_dir(fs: &dyn Fs, dir: &AbsPath) -> FsResult<()> {
    let meta = fs.metadata(dir, Follow::NoFollow)?;
    if meta.kind != FileKind::Directory {
        return Err(FsError::new(
            FsErrorKind::NotADirectory,
            "journal_dir",
            Some(dir.as_path()),
        ));
    }
    if !fs.can_write_dir(dir)? {
        return Err(FsError::new(
            FsErrorKind::PermissionDenied,
            "journal_dir",
            Some(dir.as_path()),
        ));
    }
    Ok(())
}

/// Writes a file that does not exist yet; an existing file is kept.
fn write_file_once(
    fs: &dyn FsWrite,
    scope: &TxScope,
    path: &AbsPath,
    content: &[u8],
) -> FsResult<()> {
    match fs.metadata(path, Follow::NoFollow) {
        Ok(_) => return Ok(()),
        Err(error) if error.kind == FsErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let (Some(dir), Some(name)) = (path.parent(), path.file_name()) else {
        return Err(FsError::new(
            FsErrorKind::InvalidPath,
            "write",
            Some(path.as_path()),
        ));
    };
    let temp = write_temp(fs, scope, &dir, &name.to_string_lossy(), content)?;
    match fs.rename(scope, &temp, path, RenameMode::NoReplace) {
        Ok(()) => Ok(()),
        Err(error) if error.kind == FsErrorKind::Exists => fs.remove_file(scope, &temp),
        Err(error) => {
            discard(fs, scope, &temp);
            Err(error)
        }
    }
}

/// Writes `content` to a new temporary file in `dir` and flushes it; returns
/// the temporary file.
pub(crate) fn write_temp(
    fs: &dyn FsWrite,
    scope: &TxScope,
    dir: &AbsPath,
    name_hint: &str,
    content: &[u8],
) -> FsResult<AbsPath> {
    let mut handle = fs.create_temp_in(scope, dir, name_hint)?;
    let temp = handle.path().clone();
    let written = handle
        .write_all(content)
        .map_err(|error| io_failure("write", &temp, &error))
        .and_then(|()| handle.sync(Durability::Full));
    drop(handle);
    match written {
        Ok(()) => Ok(temp),
        Err(error) => {
            discard(fs, scope, &temp);
            Err(error)
        }
    }
}

/// Removes a temporary file after a failure. A removal error is not reported:
/// the original failure is the one that explains the outcome, and a leftover
/// temporary file carries no state.
pub(crate) fn discard(fs: &dyn FsWrite, scope: &TxScope, temp: &AbsPath) {
    let _ = fs.remove_file(scope, temp);
}

/// File system error for a failed read or write through a handle.
pub(crate) fn io_failure(op: &str, path: &AbsPath, error: &io::Error) -> FsError {
    let kind = if error.kind() == io::ErrorKind::StorageFull {
        FsErrorKind::NoSpace
    } else {
        FsErrorKind::Io
    };
    let mut failure = FsError::new(kind, op, Some(path.as_path()));
    failure.os_code = error.raw_os_error();
    failure
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::thread;

    use nurt_contracts::env::Os;
    use nurt_contracts::fs::{AbsPath, FsErrorKind, SpaceInfo, VolumeInfo, VolumeKind};

    use super::{CACHEDIR_TAG, ensure_journal_dir, journal_dir_name, select};
    use crate::layout::VERSION_TEXT;
    use crate::volumes::testfs::{TestFs, abs, volume_info};

    const SPACE: SpaceInfo = SpaceInfo {
        total: 1 << 30,
        available: 1 << 29,
        quota_remaining: None,
        quota_limit: None,
        purgeable_system: None,
    };

    /// Entries of a complete journal directory on Linux, in byte order.
    const JOURNAL_ENTRIES: [&str; 9] = [
        ".gitignore",
        "CACHEDIR.TAG",
        "VERSION",
        "lock",
        "ops",
        "pending",
        "reap",
        "tomb",
        "vol.json",
    ];

    /// Volume `/home` (device 1) with `/home/user/project` writable and the
    /// mount point itself read-only.
    fn home() -> (TestFs, VolumeInfo) {
        let fs = TestFs::new();
        let volume = volume_info(1, "/home", "ext4", VolumeKind::Fixed);
        fs.add_volume("/home", volume.clone(), SPACE);
        fs.mkdir_p("/home/user/project");
        fs.deny_write("/home");
        (fs, volume)
    }

    fn select_for(fs: &TestFs, volume: &VolumeInfo, within: &str) -> Option<AbsPath> {
        select(fs, volume, &[abs(within)], ".nurt-journal", Os::Linux).unwrap()
    }

    fn sorted_children(fs: &TestFs, dir: &str) -> Vec<String> {
        let mut children = fs.children(dir);
        children.sort();
        children
    }

    #[test]
    fn uses_mount_point_when_writable() {
        let fs = TestFs::new();
        let volume = volume_info(2, "/mnt/data", "ext4", VolumeKind::Fixed);
        fs.add_volume("/mnt/data", volume.clone(), SPACE);
        fs.mkdir_p("/mnt/data/a/b");
        assert_eq!(
            select_for(&fs, &volume, "/mnt/data/a/b"),
            Some(abs("/mnt/data/.nurt-journal"))
        );
    }

    #[test]
    fn uses_highest_writable_ancestor_on_the_same_volume() {
        let (fs, volume) = home();
        assert_eq!(
            select_for(&fs, &volume, "/home/user/project/src"),
            Some(abs("/home/user/.nurt-journal"))
        );
    }

    #[test]
    fn tries_every_given_path_in_order() {
        let (fs, volume) = home();
        fs.deny_write("/home/user");
        fs.mkdir_p("/home/shared");
        let within = [abs("/home/user/project"), abs("/home/shared")];
        let dir = select(&fs, &volume, &within, ".nurt-journal", Os::Linux).unwrap();
        assert_eq!(dir, Some(abs("/home/user/project/.nurt-journal")));
        assert!(
            select(&fs, &volume, &[], ".nurt-journal", Os::Linux)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn skips_ancestors_of_another_subvolume_or_mount() {
        let fs = TestFs::new();
        let outer = volume_info(3, "/srv", "btrfs", VolumeKind::Fixed);
        let mut inner = volume_info(4, "/srv", "btrfs", VolumeKind::Fixed);
        inner.id.subvolume = 257;
        fs.add_volume("/srv", outer, SPACE);
        fs.add_volume("/srv/sub", inner.clone(), SPACE);
        fs.mkdir_p("/srv/sub/data");
        assert_eq!(
            select_for(&fs, &inner, "/srv/sub/data"),
            Some(abs("/srv/sub/.nurt-journal"))
        );
    }

    #[test]
    fn reports_no_anchor_when_nothing_is_writable() {
        let (fs, volume) = home();
        fs.deny_write("/home/user");
        fs.deny_write("/home/user/project");
        assert_eq!(select_for(&fs, &volume, "/home/user/project"), None);
        assert!(fs.calls().is_empty());
    }

    #[test]
    fn ignores_missing_ancestors_and_links() {
        let (fs, volume) = home();
        fs.symlink("/home/user/link", "/elsewhere");
        assert_eq!(
            select_for(&fs, &volume, "/home/user/link/new/dir"),
            Some(abs("/home/user/.nurt-journal"))
        );
    }

    #[test]
    fn refuses_link_planted_under_the_journal_name() {
        let (fs, volume) = home();
        fs.symlink("/home/user/.nurt-journal", "/home/other/.nurt-journal");
        assert_eq!(
            select_for(&fs, &volume, "/home/user/project"),
            Some(abs("/home/user/project/.nurt-journal"))
        );
    }

    #[test]
    fn passes_on_errors_other_than_unusable_anchor() {
        let (fs, volume) = home();
        let within = [abs("/home/user/project")];
        let failures = [
            ("volume", "/home/user", FsErrorKind::Io),
            (
                "create_dir",
                "/home/user/.nurt-journal",
                FsErrorKind::NoSpace,
            ),
            ("metadata", "/home/user", FsErrorKind::Io),
        ];
        for (op, path, kind) in failures {
            fs.clear_failures();
            fs.fail(op, path, kind);
            let error = select(&fs, &volume, &within, ".nurt-journal", Os::Linux).unwrap_err();
            assert_eq!(error.kind, kind, "{op}");
        }
    }

    #[test]
    fn creates_private_hidden_directory_with_markers_and_format_files() {
        let (fs, volume) = home();
        let dir = ensure_journal_dir(&fs, &abs("/home/user"), ".nurt-journal", Os::Linux, &volume)
            .unwrap();
        let root = "/home/user/.nurt-journal";
        assert_eq!(dir, abs(root));
        assert_eq!(fs.mode(root), Some(0o700));
        assert_eq!(fs.mode(&format!("{root}/ops")), Some(0o700));
        assert!(fs.is_hidden(root));
        let tag = fs.read(&format!("{root}/CACHEDIR.TAG")).unwrap();
        assert!(tag.starts_with(b"Signature: 8a477f597d28d172789f06886806bc55"));
        assert_eq!(tag, CACHEDIR_TAG.as_bytes());
        assert_eq!(
            fs.read(&format!("{root}/.gitignore")).unwrap(),
            b"*\n".to_vec()
        );
        assert_eq!(
            fs.read(&format!("{root}/VERSION")).unwrap(),
            VERSION_TEXT.as_bytes()
        );
        let description: serde_json::Value =
            serde_json::from_slice(&fs.read(&format!("{root}/vol.json")).unwrap()).unwrap();
        assert_eq!(description["dev"], 1);
        assert_eq!(description["fs"], "ext4");
        assert_eq!(
            description["anchor"],
            nurt_testkit::paths::abs("/home/user")
        );
        assert_eq!(
            description["features"],
            serde_json::json!(["durable_metadata"])
        );
        assert_eq!(sorted_children(&fs, root), JOURNAL_ENTRIES);
    }

    #[test]
    fn adds_spotlight_and_icloud_markers_on_macos() {
        let (fs, volume) = home();
        let name = journal_dir_name(".nurt-journal", Os::Macos);
        assert_eq!(name, ".nurt-journal.noindex");
        assert_eq!(journal_dir_name(&name, Os::Macos), name);
        assert_eq!(journal_dir_name(".journal", Os::Macos), ".journal.noindex");
        assert_eq!(
            journal_dir_name(".nurt-journal", Os::Windows),
            ".nurt-journal"
        );
        ensure_journal_dir(&fs, &abs("/home/user"), &name, Os::Macos, &volume).unwrap();
        let root = "/home/user/.nurt-journal.noindex";
        assert_eq!(
            fs.read(&format!("{root}/.metadata_never_index")),
            Some(Vec::new())
        );
        assert_eq!(fs.read(&format!("{root}/.nosync")), Some(Vec::new()));
    }

    #[test]
    fn completes_existing_directory_without_rewriting_files() {
        let (fs, volume) = home();
        fs.mkdir_p("/home/user/.nurt-journal/ops");
        fs.write_file("/home/user/.nurt-journal/.gitignore", b"custom\n");
        ensure_journal_dir(&fs, &abs("/home/user"), ".nurt-journal", Os::Linux, &volume).unwrap();
        assert_eq!(
            fs.read("/home/user/.nurt-journal/.gitignore").unwrap(),
            b"custom\n".to_vec()
        );
        assert_eq!(
            sorted_children(&fs, "/home/user/.nurt-journal"),
            JOURNAL_ENTRIES
        );
        assert!(fs.is_hidden("/home/user/.nurt-journal"));
    }

    #[test]
    fn rejects_journal_directory_of_another_account() {
        let (fs, volume) = home();
        fs.mkdir_p("/home/user/.nurt-journal");
        fs.deny_write("/home/user/.nurt-journal");
        let error =
            ensure_journal_dir(&fs, &abs("/home/user"), ".nurt-journal", Os::Linux, &volume)
                .unwrap_err();
        assert_eq!(error.kind, FsErrorKind::PermissionDenied);
    }

    #[test]
    fn leaves_no_temporary_file_when_a_write_fails() {
        let (fs, volume) = home();
        fs.mkdir_p("/home/user/.nurt-journal");
        for op in ["sync", "rename", "write"] {
            fs.clear_failures();
            fs.fail(op, "/home/user/.nurt-journal", FsErrorKind::Io);
            let error =
                ensure_journal_dir(&fs, &abs("/home/user"), ".nurt-journal", Os::Linux, &volume)
                    .unwrap_err();
            assert_eq!(error.kind, FsErrorKind::Io, "{op}");
            assert!(fs.children("/home/user/.nurt-journal").is_empty(), "{op}");
        }
    }

    #[test]
    fn concurrent_creation_yields_one_directory_without_errors() {
        let (fs, volume) = home();
        let fs = Arc::new(fs);
        let workers: Vec<_> = (0..8)
            .map(|_| {
                let fs = Arc::clone(&fs);
                let volume = volume.clone();
                thread::spawn(move || {
                    ensure_journal_dir(
                        &*fs,
                        &abs("/home/user"),
                        ".nurt-journal",
                        Os::Linux,
                        &volume,
                    )
                })
            })
            .collect();
        for worker in workers {
            assert_eq!(
                worker.join().unwrap().unwrap(),
                abs("/home/user/.nurt-journal")
            );
        }
        assert_eq!(
            sorted_children(&fs, "/home/user/.nurt-journal"),
            JOURNAL_ENTRIES
        );
        assert_eq!(fs.mode("/home/user/.nurt-journal"), Some(0o700));
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
