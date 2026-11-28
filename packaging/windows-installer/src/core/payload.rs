// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           packaging/windows-installer/src/core/payload.rs
// Responsibility:   The embedded program archive: verification of its SHA-256 sum, listing, reading of texts and extraction with the sum of every file.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.2, 2026-11-28
// License:          see the LICENSE file or https://www.danaco-group.pl

//! The embedded program archive.
//!
//! `build.rs` copies the archive named by `NURT_SETUP_PAYLOAD` to the build
//! directory and writes its SHA-256 sum; the program embeds both. The sum is
//! checked before anything is extracted.

use std::fs::{self, File};
use std::io::{self, Cursor, Read, Write};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use zip::ZipArchive;

use crate::error::{SetupError, SetupResult};
use crate::model::Layout;

static ARCHIVE: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/payload.zip"));
static EXPECTED: &str = include_str!(concat!(env!("OUT_DIR"), "/payload.sha256"));

/// Icon of the program; written next to the program for shortcuts and for the list of applications.
pub(crate) static ICON: &[u8] = include_bytes!("../../../../branding/icons/windows/nurt.ico");

/// A file of the archive.
#[derive(Debug, Clone)]
pub(crate) struct Item {
    /// Path in the archive, with forward slashes.
    pub(crate) name: String,
    /// Index in the archive.
    pub(crate) index: usize,
    /// Size of the unpacked file.
    pub(crate) size: u64,
}

/// A file that was extracted.
#[derive(Debug, Clone)]
pub(crate) struct Extracted {
    /// Path in the archive.
    pub(crate) name: String,
    /// Where the file is now.
    pub(crate) path: PathBuf,
    /// SHA-256 sum of its content.
    pub(crate) sha256: String,
    /// Size in bytes.
    pub(crate) size: u64,
}

/// An archive is embedded.
pub(crate) fn present() -> bool {
    !ARCHIVE.is_empty()
}

/// Sum that a test substitutes for the recorded one.
#[cfg(test)]
pub(crate) static TEST_SUM: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// SHA-256 sum recorded at build time.
pub(crate) fn expected_sum() -> String {
    #[cfg(test)]
    if let Ok(guard) = TEST_SUM.lock()
        && let Some(sum) = guard.as_ref()
    {
        return sum.clone();
    }
    EXPECTED.trim().to_string()
}

fn open() -> SetupResult<ZipArchive<Cursor<&'static [u8]>>> {
    if !present() {
        return Err(SetupError::new(
            "payload-missing",
            "the setup was built without the program archive",
        ));
    }
    ZipArchive::new(Cursor::new(ARCHIVE))
        .map_err(|error| SetupError::new("payload-corrupt", error.to_string()))
}

/// Lists the files of the archive; directories are not listed.
pub(crate) fn list() -> SetupResult<Vec<Item>> {
    let mut archive = open()?;
    let mut items = Vec::with_capacity(archive.len());
    for index in 0..archive.len() {
        let file = archive
            .by_index(index)
            .map_err(|error| SetupError::new("payload-corrupt", error.to_string()))?;
        let Some(name) = file.enclosed_name() else {
            return Err(SetupError::new(
                "payload-corrupt",
                format!(
                    "unsafe path in the archive: {}",
                    String::from_utf8_lossy(file.name_raw())
                ),
            ));
        };
        if file.is_dir() {
            continue;
        }
        let name = name
            .components()
            .map(|part| part.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/");
        items.push(Item {
            name,
            index,
            size: file.size(),
        });
    }
    Ok(items)
}

/// Whether the archive file is installed also without the documentation: the
/// program, the licence files and the completions of PowerShell.
fn is_program_file(name: &str) -> bool {
    name.eq_ignore_ascii_case("nurt.exe")
        || name == "LICENSE"
        || name == "NOTICE"
        || name == "completions/_nurt.ps1"
}

/// Size of the files that an installation puts on the disk, in bytes: every
/// file of the archive with the documentation, the program files without it.
pub(crate) fn installed_size(docs: bool) -> u64 {
    list()
        .map(|items| {
            items
                .iter()
                .filter(|item| docs || is_program_file(&item.name))
                .map(|item| item.size)
                .sum()
        })
        .unwrap_or(0)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// SHA-256 sum of a text of bytes.
pub(crate) fn sha256_of(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

/// SHA-256 sum of a file.
pub(crate) fn sha256_of_file(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 256 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(hex(&hasher.finalize()))
}

/// Computes the SHA-256 sum of the archive and compares it with the recorded one.
///
/// `progress` receives the bytes done and the total.
pub(crate) fn verify(progress: &mut dyn FnMut(u64, u64)) -> SetupResult<String> {
    if !present() {
        return Err(SetupError::new(
            "payload-missing",
            "the setup was built without the program archive",
        ));
    }
    let mut hasher = Sha256::new();
    let total = ARCHIVE.len() as u64;
    let mut done = 0u64;
    for chunk in ARCHIVE.chunks(1 << 20) {
        hasher.update(chunk);
        done += chunk.len() as u64;
        progress(done, total);
    }
    let sum = hex(&hasher.finalize());
    let expected = expected_sum();
    if sum != expected {
        return Err(SetupError::new(
            "payload-corrupt",
            format!("expected {expected}, found {sum}"),
        ));
    }
    Ok(sum)
}

/// Reads a text file of the archive.
pub(crate) fn read_text(name: &str) -> SetupResult<String> {
    let mut archive = open()?;
    let mut file = archive
        .by_name(name)
        .map_err(|error| SetupError::new("payload-corrupt", format!("{name}: {error}")))?;
    let mut text = String::new();
    file.read_to_string(&mut text)
        .map_err(|error| SetupError::new("payload-corrupt", format!("{name}: {error}")))?;
    Ok(text)
}

/// Extracts the files below `staging`, keeping the paths of the archive.
///
/// Only the files for which `wanted` holds are extracted; the others stay in the archive.
/// `progress` receives the bytes done, the total and the file being written.
pub(crate) fn extract(
    staging: &Path,
    wanted: &dyn Fn(&str) -> bool,
    cancel: &dyn Fn() -> bool,
    progress: &mut dyn FnMut(u64, u64, &str),
) -> SetupResult<Vec<Extracted>> {
    let items: Vec<Item> = list()?
        .into_iter()
        .filter(|item| wanted(&item.name))
        .collect();
    let total: u64 = items.iter().map(|item| item.size).sum();
    let mut archive = open()?;
    let mut done = 0u64;
    let mut files = Vec::with_capacity(items.len());
    let mut buffer = vec![0u8; 256 * 1024];
    for item in &items {
        if cancel() {
            return Err(SetupError::new("cancelled", ""));
        }
        let target = staging.join(item.name.replace('/', "\\"));
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|error| SetupError::io(parent, &error))?;
        }
        let mut source = archive
            .by_index(item.index)
            .map_err(|error| SetupError::new("payload-corrupt", error.to_string()))?;
        let mut output = File::create(&target).map_err(|error| SetupError::io(&target, &error))?;
        let mut hasher = Sha256::new();
        loop {
            let count = source.read(&mut buffer).map_err(|error| {
                SetupError::new("payload-corrupt", format!("{}: {error}", item.name))
            })?;
            if count == 0 {
                break;
            }
            hasher.update(&buffer[..count]);
            output
                .write_all(&buffer[..count])
                .map_err(|error| SetupError::io(&target, &error))?;
            done += count as u64;
            progress(done, total, &item.name);
        }
        output
            .flush()
            .map_err(|error| SetupError::io(&target, &error))?;
        files.push(Extracted {
            name: item.name.clone(),
            path: target,
            sha256: hex(&hasher.finalize()),
            size: item.size,
        });
    }
    Ok(files)
}

/// Where a file of the archive goes in an installation; `None` when it is not installed.
///
/// `nurt.exe` goes to the folder of the program. With the documentation the other files go to
/// `doc` as `install.ps1` places them (the files of the archive folder `doc` at the top of
/// `doc`). `LICENSE` and `NOTICE` are always installed, because the license asks for the notice
/// to travel with every distribution; the completions only when `completions` is set.
pub(crate) fn destination(name: &str, layout: &Layout, completions: bool) -> Option<PathBuf> {
    let relative = name.replace('/', "\\");
    if name.eq_ignore_ascii_case("nurt.exe") {
        return Some(layout.exe());
    }
    let base = if layout.docs {
        layout.doc()
    } else {
        layout.root.clone()
    };
    if name.starts_with("completions/") {
        return if layout.docs {
            Some(base.join(&relative))
        } else {
            (completions && name == "completions/_nurt.ps1").then(|| base.join(&relative))
        };
    }
    if name == "LICENSE" || name == "NOTICE" {
        return Some(base.join(&relative));
    }
    if !layout.docs {
        return None;
    }
    match name.strip_prefix("doc/") {
        Some(rest) => Some(layout.doc().join(rest.replace('/', "\\"))),
        None => Some(layout.doc().join(relative)),
    }
}

/// Copies the running executable to a path; used for `uninstall.exe`.
pub(crate) fn copy_self(destination: &Path) -> io::Result<u64> {
    let exe = std::env::current_exe()?;
    fs::copy(exe, destination)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn destinations_follow_the_layout() {
        let layout = Layout::of(Path::new("C:\\N\\bin"), true);
        assert_eq!(
            destination("nurt.exe", &layout, false),
            Some(PathBuf::from("C:\\N\\bin\\nurt.exe"))
        );
        assert_eq!(
            destination("doc/index.html", &layout, false),
            Some(PathBuf::from("C:\\N\\doc\\index.html"))
        );
        assert_eq!(
            destination("README.md", &layout, false),
            Some(PathBuf::from("C:\\N\\doc\\README.md"))
        );
        assert_eq!(
            destination("completions/_nurt.ps1", &layout, false),
            Some(PathBuf::from("C:\\N\\doc\\completions\\_nurt.ps1"))
        );
        let bare = Layout::of(Path::new("C:\\Tools\\Nurt"), true);
        assert_eq!(destination("README.md", &bare, false), None);
        assert_eq!(
            destination("LICENSE", &bare, false),
            Some(PathBuf::from("C:\\Tools\\Nurt\\LICENSE"))
        );
        assert_eq!(destination("completions/_nurt.ps1", &bare, false), None);
        assert_eq!(
            destination("completions/_nurt.ps1", &bare, true),
            Some(PathBuf::from("C:\\Tools\\Nurt\\completions\\_nurt.ps1"))
        );
        assert_eq!(destination("completions/nurt.bash", &bare, true), None);
    }

    #[test]
    fn the_documentation_follows_the_archive_as_install_ps1_places_it() {
        let layout = Layout::of(Path::new("D:\\Nurt\\bin"), true);
        let place = |name: &str| destination(name, &layout, false);
        assert_eq!(
            place("doc/spec/journal.html"),
            Some(PathBuf::from("D:\\Nurt\\doc\\spec\\journal.html"))
        );
        assert_eq!(
            place("CHANGELOG.md"),
            Some(PathBuf::from("D:\\Nurt\\doc\\CHANGELOG.md"))
        );
        assert_eq!(
            place("NOTICE"),
            Some(PathBuf::from("D:\\Nurt\\doc\\NOTICE"))
        );
        assert_eq!(
            place("completions/nurt.bash"),
            Some(PathBuf::from("D:\\Nurt\\doc\\completions\\nurt.bash"))
        );
        assert_eq!(
            place("NURT.EXE"),
            Some(PathBuf::from("D:\\Nurt\\bin\\nurt.exe"))
        );
    }

    #[test]
    fn a_folder_not_named_bin_takes_the_program_and_the_notices_only() {
        let layout = Layout::of(Path::new("D:\\Tools"), true);
        assert!(!layout.docs);
        assert_eq!(
            destination("nurt.exe", &layout, false),
            Some(PathBuf::from("D:\\Tools\\nurt.exe"))
        );
        assert_eq!(destination("doc/index.html", &layout, true), None);
        assert_eq!(destination("CHANGELOG.md", &layout, true), None);
        assert_eq!(
            destination("NOTICE", &layout, false),
            Some(PathBuf::from("D:\\Tools\\NOTICE"))
        );
    }

    #[test]
    fn the_documentation_can_be_left_out() {
        let layout = Layout::of(Path::new("D:\\Nurt\\bin"), false);
        assert_eq!(
            destination("nurt.exe", &layout, false),
            Some(PathBuf::from("D:\\Nurt\\bin\\nurt.exe"))
        );
        assert_eq!(destination("doc/index.html", &layout, false), None);
        assert_eq!(destination("README.md", &layout, false), None);
        assert_eq!(
            destination("LICENSE", &layout, false),
            Some(PathBuf::from("D:\\Nurt\\LICENSE"))
        );
    }

    #[test]
    fn sums_are_lowercase_hexadecimal() {
        assert_eq!(
            sha256_of(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        let path = std::env::temp_dir().join(format!("nurt-sum-{}.bin", std::process::id()));
        fs::write(&path, b"abc").unwrap_or_default();
        assert_eq!(sha256_of_file(&path).ok(), Some(sha256_of(b"abc")));
        let _ = fs::remove_file(&path);
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
