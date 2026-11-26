// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! State directory of the engine.
//!
//! The state directory holds what the engine keeps between calls and between
//! processes: complete output of programs (`spool`), background jobs
//! (`jobs`), result files and stored pages of results (`results`), the read
//! cache (`readcache`) and test and analysis runs (`verify`). Its location is
//! `state.dir` (variable `NURT_STATE_DIR`) or the state directory of the
//! operating system ([`crate::paths`]).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use sha2::{Digest, Sha256};

use crate::error::EngineError;

/// Permissions of the directories created by the engine on Unix systems:
/// the state can contain content of user files, so only the account reads it.
#[cfg(unix)]
const PRIVATE_DIR_MODE: u32 = 0o700;

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Area of the state directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum StateArea {
    /// Complete output of programs.
    Spool,
    /// Background jobs.
    Jobs,
    /// Result files and stored pages of results.
    Results,
    /// Content kept by the read cache.
    ReadCache,
    /// Test and analysis runs kept for comparison.
    Verify,
}

impl StateArea {
    /// Every area, in the order of creation.
    pub const ALL: [Self; 5] = [
        Self::Spool,
        Self::Jobs,
        Self::Results,
        Self::ReadCache,
        Self::Verify,
    ];

    /// Name of the subdirectory of the area.
    #[must_use]
    pub const fn dir_name(self) -> &'static str {
        match self {
            Self::Spool => "spool",
            Self::Jobs => "jobs",
            Self::Results => "results",
            Self::ReadCache => "readcache",
            Self::Verify => "verify",
        }
    }
}

/// An open state directory with all its areas.
///
/// # Examples
///
/// ```
/// use nurt_core::state::{StateArea, StateDir};
///
/// let temp = tempfile::tempdir()?;
/// let state = StateDir::open(temp.path().join("state")).map_err(std::io::Error::other)?;
/// assert!(state.area(StateArea::Results).is_dir());
/// # Ok::<(), std::io::Error>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateDir {
    root: PathBuf,
}

impl StateDir {
    /// Opens the state directory at `root`, creating it and its areas when
    /// they are missing.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError`] when `root` is not absolute or a directory
    /// cannot be created.
    pub fn open(root: impl Into<PathBuf>) -> Result<Self, EngineError> {
        let root = root.into();
        if !root.is_absolute() {
            return Err(EngineError::relative_path("state directory", &root));
        }
        create_private_dir(&root)
            .map_err(|error| EngineError::io("create the state directory", &root, &error))?;
        for area in StateArea::ALL {
            let dir = root.join(area.dir_name());
            create_private_dir(&dir)
                .map_err(|error| EngineError::io("create the state directory", &dir, &error))?;
        }
        Ok(Self { root })
    }

    /// Root of the state directory.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Directory of one area.
    #[must_use]
    pub fn area(&self, area: StateArea) -> PathBuf {
        self.root.join(area.dir_name())
    }
}

fn create_private_dir(path: &Path) -> io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(PRIVATE_DIR_MODE);
    }
    builder.create(path)
}

/// Writes `bytes` to `path` through a temporary file in the same directory,
/// so that readers never see a partly written file.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let dir = path.parent().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "path has no parent directory")
    })?;
    fs::create_dir_all(dir)?;
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let temp = dir.join(format!(
        ".{name}.{}.{}.tmp",
        std::process::id(),
        TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    fs::write(&temp, bytes)?;
    fs::rename(&temp, path).inspect_err(|_| {
        let _ = fs::remove_file(&temp);
    })
}

/// Content of `path`, or `None` when the file does not exist.
pub(crate) fn read_optional(path: &Path) -> io::Result<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

/// Lowercase hexadecimal SHA-256 digest of `data`.
pub(crate) fn sha256_hex(data: &[u8]) -> String {
    let digest = Sha256::digest(data);
    let mut text = String::with_capacity(digest.len() * 2);
    for byte in digest.iter() {
        text.push(char::from(HEX_DIGITS[usize::from(byte >> 4)]));
        text.push(char::from(HEX_DIGITS[usize::from(byte & 0x0f)]));
    }
    text
}

const HEX_DIGITS: &[u8; 16] = b"0123456789abcdef";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_creates_every_area() {
        let temp = tempfile::tempdir().unwrap();
        let state = StateDir::open(temp.path().join("nurt")).unwrap();
        for area in StateArea::ALL {
            assert!(state.area(area).is_dir(), "{}", area.dir_name());
        }
        assert_eq!(state.root(), temp.path().join("nurt"));
    }

    #[cfg(unix)]
    #[test]
    fn state_directory_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let state = StateDir::open(temp.path().join("nurt")).unwrap();
        let mode = fs::metadata(state.root()).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, PRIVATE_DIR_MODE);
    }

    #[test]
    fn relative_root_is_rejected() {
        let error = StateDir::open("state").unwrap_err();
        assert!(error.message().contains("absolute"));
    }

    #[test]
    fn unwritable_root_is_an_io_error() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("file");
        fs::write(&file, b"x").unwrap();
        let error = StateDir::open(file.join("state")).unwrap_err();
        assert_eq!(error.status(), nurt_contracts::error::AbiStatus::Io);
    }

    #[test]
    fn atomic_write_replaces_content() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("dir").join("value.json");
        write_atomic(&path, b"one").unwrap();
        write_atomic(&path, b"two").unwrap();
        assert_eq!(read_optional(&path).unwrap().unwrap(), b"two");
        let leftovers = fs::read_dir(temp.path().join("dir")).unwrap().count();
        assert_eq!(leftovers, 1);
    }

    #[test]
    fn missing_file_reads_as_none() {
        let temp = tempfile::tempdir().unwrap();
        assert!(
            read_optional(&temp.path().join("absent"))
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn reading_a_directory_is_an_error_and_not_a_missing_file() {
        let temp = tempfile::tempdir().unwrap();
        assert!(read_optional(temp.path()).is_err());
    }

    #[test]
    fn atomic_write_to_a_path_without_a_parent_is_an_error() {
        let error = write_atomic(Path::new("/"), b"x").unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn digest_of_empty_input_is_the_known_value() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn digest_is_lowercase_hexadecimal() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
