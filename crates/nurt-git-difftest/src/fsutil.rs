// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! File system helpers: byte-exact copies of case directories, scenario
//! writes and directory walks.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{Result, io_error};
use crate::scenario::{FileContent, FileWrite};

/// One entry found by [`walk`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    /// Regular file.
    File {
        /// The executable bit is set (always `false` on Windows).
        executable: bool,
        /// Content.
        bytes: Vec<u8>,
    },
    /// Directory.
    Dir,
    /// Symbolic link.
    Link {
        /// Target as stored in the link, with `/` separators.
        target: String,
    },
    /// Another kind of entry (socket, device, pipe).
    Other,
}

/// Copies the directory tree `from` to `to`, keeping symbolic links as links
/// and the executable bits of files.
///
/// # Errors
///
/// Returns [`crate::error::HarnessError::Io`] for the first entry that cannot
/// be read or written.
pub fn copy_tree(from: &Path, to: &Path) -> Result<()> {
    fs::create_dir_all(to).map_err(io_error("create directory", to))?;
    for item in fs::read_dir(from).map_err(io_error("read directory", from))? {
        let item = item.map_err(io_error("read directory", from))?;
        let source = item.path();
        let target = to.join(item.file_name());
        let kind = item.file_type().map_err(io_error("inspect", &source))?;
        if kind.is_symlink() {
            let link = fs::read_link(&source).map_err(io_error("read link", &source))?;
            make_symlink(&link, &target, source.is_dir())?;
        } else if kind.is_dir() {
            copy_tree(&source, &target)?;
        } else {
            fs::copy(&source, &target).map_err(io_error("copy", &source))?;
        }
    }
    Ok(())
}

/// Applies one scenario write below `root`.
///
/// Missing parent directories are created; an existing entry is replaced.
///
/// # Errors
///
/// Returns [`crate::error::HarnessError::Io`] when the entry cannot be
/// written or removed.
pub fn apply_write(root: &Path, write: &FileWrite) -> Result<()> {
    let path = root.join(&write.path);
    remove_entry(&path)?;
    if write.content == FileContent::Delete {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(io_error("create directory", parent))?;
    }
    match &write.content {
        FileContent::Bytes(bytes) => {
            fs::write(&path, bytes).map_err(io_error("write", &path))?;
            set_executable(&path, write.executable)
        }
        FileContent::Symlink(target) => make_symlink(Path::new(target), &path, false),
        FileContent::Delete => Ok(()),
    }
}

/// Removes a file, link or directory tree; a missing entry is not an error.
///
/// # Errors
///
/// Returns [`crate::error::HarnessError::Io`] when the entry exists and
/// cannot be removed.
pub fn remove_entry(path: &Path) -> Result<()> {
    let Ok(meta) = fs::symlink_metadata(path) else {
        return Ok(());
    };
    if meta.is_dir() {
        fs::remove_dir_all(path).map_err(io_error("remove directory", path))
    } else {
        fs::remove_file(path).map_err(io_error("remove", path))
    }
}

/// Creates a symbolic link at `link` pointing to `target`.
///
/// # Errors
///
/// Returns [`crate::error::HarnessError::Io`] when the link cannot be created
/// (on Windows also when the account may not create links).
pub fn make_symlink(target: &Path, link: &Path, directory: bool) -> Result<()> {
    #[cfg(unix)]
    {
        let _ = directory;
        std::os::unix::fs::symlink(target, link).map_err(io_error("create link", link))
    }
    #[cfg(windows)]
    {
        let created = if directory {
            std::os::windows::fs::symlink_dir(target, link)
        } else {
            std::os::windows::fs::symlink_file(target, link)
        };
        created.map_err(io_error("create link", link))
    }
}

/// Sets or clears the executable bits of a file (no effect on Windows).
///
/// # Errors
///
/// Returns [`crate::error::HarnessError::Io`] when the permissions cannot be
/// changed.
pub fn set_executable(path: &Path, executable: bool) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        const EXECUTABLE_BITS: u32 = 0o111;
        const READ_BITS: u32 = 0o444;
        let meta = fs::metadata(path).map_err(io_error("inspect", path))?;
        let mode = meta.permissions().mode();
        let mode = if executable {
            mode | ((mode & READ_BITS) >> 2)
        } else {
            mode & !EXECUTABLE_BITS
        };
        fs::set_permissions(path, fs::Permissions::from_mode(mode))
            .map_err(io_error("change permissions of", path))
    }
    #[cfg(not(unix))]
    {
        let _ = (path, executable);
        Ok(())
    }
}

/// Returns `true` when the metadata has an executable bit (always `false` on
/// Windows).
#[must_use]
pub fn is_executable(meta: &fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        let _ = meta;
        false
    }
}

/// Permission bits of an entry (`0o644`-style on Unix, `0` on Windows).
#[must_use]
pub fn mode_bits(meta: &fs::Metadata) -> u32 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        meta.permissions().mode() & 0o7777
    }
    #[cfg(not(unix))]
    {
        let _ = meta;
        0
    }
}

/// Path of `path` relative to `root` with `/` separators.
#[must_use]
pub fn relative_key(root: &Path, path: &Path) -> String {
    let relative = path.strip_prefix(root).unwrap_or(path);
    relative
        .components()
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

/// Reads one entry without following a final symbolic link.
///
/// # Errors
///
/// Returns [`crate::error::HarnessError::Io`] when the entry cannot be read.
pub fn read_entry(path: &Path) -> Result<Entry> {
    let meta = fs::symlink_metadata(path).map_err(io_error("inspect", path))?;
    let kind = meta.file_type();
    Ok(if kind.is_symlink() {
        let target = fs::read_link(path).map_err(io_error("read link", path))?;
        Entry::Link {
            target: target.to_string_lossy().replace('\\', "/"),
        }
    } else if kind.is_dir() {
        Entry::Dir
    } else if kind.is_file() {
        Entry::File {
            executable: is_executable(&meta),
            bytes: fs::read(path).map_err(io_error("read", path))?,
        }
    } else {
        Entry::Other
    })
}

/// Lists every entry below `root`, keyed by its relative path with `/`
/// separators, without descending into directories for which `skip` returns
/// `true` (those are not listed either).
///
/// # Errors
///
/// Returns [`crate::error::HarnessError::Io`] when an entry cannot be read.
pub fn walk(root: &Path, skip: &dyn Fn(&Path) -> bool) -> Result<BTreeMap<String, Entry>> {
    let mut entries = BTreeMap::new();
    let mut pending: Vec<PathBuf> = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for item in fs::read_dir(&dir).map_err(io_error("read directory", &dir))? {
            let path = item.map_err(io_error("read directory", &dir))?.path();
            if skip(&path) {
                continue;
            }
            let entry = read_entry(&path)?;
            if entry == Entry::Dir {
                pending.push(path.clone());
            }
            entries.insert(relative_key(root, &path), entry);
        }
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copies_files_directories_links_and_modes() {
        let dir = tempfile::tempdir().unwrap();
        let from = dir.path().join("from");
        fs::create_dir_all(from.join("sub/empty")).unwrap();
        fs::write(from.join("sub/a.txt"), b"a").unwrap();
        fs::write(from.join("run.sh"), b"#!/bin/sh\n").unwrap();
        set_executable(&from.join("run.sh"), true).unwrap();
        let linked = make_symlink(Path::new("sub/a.txt"), &from.join("link"), false).is_ok();
        let to = dir.path().join("to");
        copy_tree(&from, &to).unwrap();
        let none = |_: &Path| false;
        assert_eq!(walk(&from, &none).unwrap(), walk(&to, &none).unwrap());
        let listed = walk(&to, &none).unwrap();
        assert_eq!(listed.get("sub/empty"), Some(&Entry::Dir));
        if linked {
            assert_eq!(
                listed.get("link"),
                Some(&Entry::Link {
                    target: "sub/a.txt".into()
                })
            );
        }
        if cfg!(unix) {
            assert!(matches!(listed.get("run.sh"), Some(Entry::File { executable: true, .. })));
        }
    }

    #[test]
    fn walk_skips_selected_directories() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".git/objects")).unwrap();
        fs::write(dir.path().join("a"), b"x").unwrap();
        let listed = walk(dir.path(), &|path: &Path| path.ends_with(".git")).unwrap();
        assert_eq!(listed.keys().collect::<Vec<_>>(), vec!["a"]);
    }

    #[test]
    fn applies_writes_replacing_and_deleting_entries() {
        let dir = tempfile::tempdir().unwrap();
        let write = |path: &str, content: FileContent, executable: bool| FileWrite {
            path: path.into(),
            content,
            executable,
        };
        apply_write(dir.path(), &write("d/x", FileContent::Bytes(b"1".to_vec()), true)).unwrap();
        assert_eq!(fs::read(dir.path().join("d/x")).unwrap(), b"1");
        apply_write(dir.path(), &write("d", FileContent::Bytes(b"2".to_vec()), false)).unwrap();
        assert_eq!(fs::read(dir.path().join("d")).unwrap(), b"2");
        apply_write(dir.path(), &write("d", FileContent::Delete, false)).unwrap();
        assert!(!dir.path().join("d").exists());
        apply_write(dir.path(), &write("missing", FileContent::Delete, false)).unwrap();
    }

    #[test]
    fn relative_keys_use_forward_slashes() {
        let root = Path::new("/r");
        assert_eq!(relative_key(root, &root.join("a").join("b")), "a/b");
    }
}
