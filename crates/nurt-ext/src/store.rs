// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-ext::store
// Responsibility:   Installation, removal and listing of extensions.
// Dependencies:     nurt_contracts, serde
// Author:           Danaco Official
// Version:          0.1.2, 2026-11-28
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Installation, removal and listing of extensions.
//!
//! [`ExtStore`] works on the directory of [`ExtLocations`]. Installing
//! copies an extension directory (manifest, component or library, grammar
//! and query files) into `<dir>/<name>` after checking the manifest against
//! the host; the copy is made in a staging directory and switched in by
//! rename, so an interrupted installation leaves the previous version in
//! place. Symbolic links inside an extension directory are refused.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use nurt_contracts::error::{ErrorCode, Result};
use serde::Serialize;

use crate::error::{error, io_error};
use crate::locations::ExtLocations;
use crate::manifest::{ApiVersion, ExtKind, HOST_API, MANIFEST_FILE, Manifest};

/// Prefix of the temporary directories used while installing.
const STAGING_PREFIX: &str = ".";
/// Suffix of the directory holding the new version during installation.
const STAGING_SUFFIX: &str = ".staging";
/// Suffix of the directory holding the replaced version during installation.
const PREVIOUS_SUFFIX: &str = ".previous";
/// Number of the next installation of this process; it keeps the temporary
/// directories of concurrent installations of one extension apart.
static INSTALLATIONS: AtomicU64 = AtomicU64::new(0);

/// Byte order mark of UTF-8.
const UTF8_MARK: [u8; 3] = [0xEF, 0xBB, 0xBF];
/// Byte order mark of UTF-16 with the low byte first.
const UTF16_LE_MARK: [u8; 2] = [0xFF, 0xFE];
/// Byte order mark of UTF-16 with the high byte first.
const UTF16_BE_MARK: [u8; 2] = [0xFE, 0xFF];

fn invalid_text(error: impl std::error::Error + Send + Sync + 'static) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, error)
}

/// Text of a manifest file: UTF-8 with or without a byte order mark, or
/// UTF-16 with one, which is how PowerShell 5.1 and Notepad write a file on
/// Windows.
fn read_manifest_text(path: &Path) -> std::io::Result<String> {
    let bytes = fs::read(path)?;
    if let Some(rest) = bytes.strip_prefix(&UTF8_MARK) {
        return String::from_utf8(rest.to_vec()).map_err(invalid_text);
    }
    let little = bytes.starts_with(&UTF16_LE_MARK);
    if little || bytes.starts_with(&UTF16_BE_MARK) {
        let units: Vec<u16> = bytes
            .get(2..)
            .unwrap_or_default()
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| {
                if little {
                    u16::from_le_bytes(*pair)
                } else {
                    u16::from_be_bytes(*pair)
                }
            })
            .collect();
        return String::from_utf16(&units).map_err(invalid_text);
    }
    String::from_utf8(bytes).map_err(invalid_text)
}

/// An extension directory with a manifest accepted by the host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledExt {
    /// The manifest.
    pub manifest: Manifest,
    /// Directory of the extension.
    pub dir: PathBuf,
}

impl InstalledExt {
    /// Extension name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.manifest.ext.name
    }

    /// Path of the component or library named by the manifest.
    #[must_use]
    pub fn artifact(&self) -> Option<PathBuf> {
        let file = match self.manifest.ext.kind {
            ExtKind::Wasm => self.manifest.ext.wasm.as_deref(),
            ExtKind::Native => self.manifest.ext.library.as_deref(),
        };
        file.map(|file| self.dir.join(file))
    }

    fn files(&self) -> Vec<PathBuf> {
        let grammars = self.manifest.grammar.iter().flat_map(|grammar| {
            std::iter::once(&grammar.wasm)
                .chain(&grammar.tags)
                .chain(&grammar.locals)
        });
        self.artifact()
            .into_iter()
            .chain(grammars.map(|file| self.dir.join(file)))
            .collect()
    }
}

/// One entry of [`ExtStore::list`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExtListing {
    /// Extension name, or the directory name when the manifest is unusable.
    pub name: String,
    /// Directory of the extension.
    pub dir: String,
    /// Version of the extension.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// Required API version.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api: Option<String>,
    /// Distribution form.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<ExtKind>,
    /// Full names of the commands.
    pub commands: Vec<String>,
    /// Why the extension cannot be loaded; absent for a usable extension.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub problem: Option<String>,
}

/// Installed extensions of one extension directory.
///
/// ```
/// use std::collections::BTreeSet;
/// use nurt_ext::locations::ExtLocations;
/// use nurt_ext::store::ExtStore;
///
/// let dir = tempfile::tempdir()?;
/// let store = ExtStore::new(ExtLocations::new(dir.path().join("ext")), BTreeSet::new());
/// assert!(store.list()?.is_empty());
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug, Clone)]
pub struct ExtStore {
    locations: ExtLocations,
    host_api: ApiVersion,
    core_names: BTreeSet<String>,
}

impl ExtStore {
    /// Store over `locations` for a host implementing [`HOST_API`] whose
    /// core uses the names in `core_names`
    /// ([`core_names`](crate::register::core_names) of the registry).
    #[must_use]
    pub fn new(locations: ExtLocations, core_names: BTreeSet<String>) -> Self {
        Self {
            locations,
            host_api: HOST_API,
            core_names,
        }
    }

    /// Locations of the store.
    #[must_use]
    pub fn locations(&self) -> &ExtLocations {
        &self.locations
    }

    /// Reads and checks the extension in `dir`.
    ///
    /// # Errors
    ///
    /// Fails with `E_NOT_FOUND` or `E_IO` when the manifest cannot be read,
    /// with `E_CATALOG` when the manifest is rejected, with `E_NOT_FOUND`
    /// when a file named by the manifest is missing, and with
    /// `E_UNSUPPORTED` for a native extension when native libraries are not
    /// allowed.
    pub fn inspect(&self, dir: &Path) -> Result<InstalledExt> {
        let path = dir.join(MANIFEST_FILE);
        let text = read_manifest_text(&path).map_err(|source| io_error("read", &path, &source))?;
        let manifest = Manifest::parse(&text)?;
        manifest.validate(self.host_api, &self.core_names)?;
        let extension = InstalledExt {
            manifest,
            dir: dir.to_path_buf(),
        };
        if extension.manifest.ext.kind == ExtKind::Native && !self.locations.native_allowed() {
            return Err(error(
                ErrorCode::Unsupported,
                format!(
                    "`{}` is a native library; set `ext.native = true` to allow native extensions",
                    extension.name()
                ),
            ));
        }
        for file in extension.files() {
            if !file.is_file() {
                return Err(error(
                    ErrorCode::NotFound,
                    format!(
                        "{} names {}, which does not exist",
                        MANIFEST_FILE,
                        file.display()
                    ),
                ));
            }
        }
        Ok(extension)
    }

    /// Installs the extension in `source`.
    ///
    /// With `replace` an installed version is replaced; without it an
    /// installed version is an error.
    ///
    /// # Errors
    ///
    /// Fails as [`ExtStore::inspect`], with `E_EXISTS` when the extension is
    /// installed and `replace` is not set, with `E_UNSUPPORTED` for a
    /// symbolic link in `source`, and with `E_IO` when copying fails.
    pub fn install(&self, source: &Path, replace: bool) -> Result<InstalledExt> {
        let extension = self.inspect(source)?;
        let name = extension.name().to_owned();
        let target = self.locations.extension_dir(&name)?;
        if target.exists() && !replace {
            return Err(error(
                ErrorCode::Exists,
                format!("extension `{name}` is installed; remove it first or replace it"),
            ));
        }
        let root = self.locations.dir();
        fs::create_dir_all(root).map_err(|source| io_error("create", root, &source))?;
        let holds_store = matches!(
            (fs::canonicalize(root), fs::canonicalize(source)),
            (Ok(root), Ok(source)) if root.starts_with(&source)
        );
        if holds_store {
            return Err(error(
                ErrorCode::InvalidValue,
                format!(
                    "{} holds the extension directory {}; copy the extension to a directory outside it",
                    source.display(),
                    root.display()
                ),
            ));
        }
        let unique = format!(
            "{}-{}",
            std::process::id(),
            INSTALLATIONS.fetch_add(1, Ordering::Relaxed)
        );
        let staging = root.join(format!("{STAGING_PREFIX}{name}.{unique}{STAGING_SUFFIX}"));
        if let Err(problem) = copy_tree(source, &staging) {
            remove_if_present(&staging)?;
            return Err(problem);
        }
        let previous = root.join(format!("{STAGING_PREFIX}{name}.{unique}{PREVIOUS_SUFFIX}"));
        switch_in(&staging, &target, &previous)?;
        Ok(InstalledExt {
            manifest: extension.manifest,
            dir: target,
        })
    }

    /// Removes the installed extension `name`, usable or not, and returns the
    /// removed directory.
    ///
    /// # Errors
    ///
    /// Fails with `E_INVALID_VALUE` for a malformed name, with `E_NOT_FOUND`
    /// when the extension is not installed, and with `E_IO` when the
    /// directory cannot be removed.
    pub fn remove(&self, name: &str) -> Result<PathBuf> {
        let dir = self.locations.extension_dir(name)?;
        if !dir.is_dir() {
            return Err(not_installed(name));
        }
        fs::remove_dir_all(&dir).map_err(|source| io_error("remove", &dir, &source))?;
        Ok(dir)
    }

    /// The installed extension `name`.
    ///
    /// # Errors
    ///
    /// Fails with `E_NOT_FOUND` when the extension is not installed and as
    /// [`ExtStore::inspect`].
    pub fn get(&self, name: &str) -> Result<InstalledExt> {
        let dir = self.locations.extension_dir(name)?;
        if !dir.is_dir() {
            return Err(not_installed(name));
        }
        self.inspect_installed(&dir, name)
    }

    /// Every installed extension in name order, usable or not.
    ///
    /// # Errors
    ///
    /// Fails with `E_IO` when the extension directory cannot be listed; a
    /// missing directory has no extensions.
    pub fn list(&self) -> Result<Vec<ExtListing>> {
        let mut listings = Vec::new();
        for (name, dir) in self.directories()? {
            let listing = match self.inspect_installed(&dir, &name) {
                Ok(extension) => ExtListing {
                    name,
                    dir: dir.display().to_string(),
                    version: Some(extension.manifest.ext.version.clone()),
                    api: Some(extension.manifest.ext.api.clone()),
                    kind: Some(extension.manifest.ext.kind),
                    commands: extension.manifest.commands(),
                    problem: None,
                },
                Err(problem) => ExtListing {
                    name,
                    dir: dir.display().to_string(),
                    version: None,
                    api: None,
                    kind: None,
                    commands: Vec::new(),
                    problem: Some(problem.message.clone()),
                },
            };
            listings.push(listing);
        }
        Ok(listings)
    }

    /// Every usable installed extension in name order, for registration.
    ///
    /// # Errors
    ///
    /// Fails as [`ExtStore::list`].
    pub fn installed(&self) -> Result<Vec<InstalledExt>> {
        Ok(self
            .directories()?
            .into_iter()
            .filter_map(|(name, dir)| self.inspect_installed(&dir, &name).ok())
            .collect())
    }

    fn inspect_installed(&self, dir: &Path, name: &str) -> Result<InstalledExt> {
        let extension = self.inspect(dir)?;
        if extension.name() == name {
            Ok(extension)
        } else {
            Err(error(
                ErrorCode::Catalog,
                format!(
                    "directory `{name}` holds extension `{}`; reinstall the extension",
                    extension.name()
                ),
            ))
        }
    }

    fn directories(&self) -> Result<Vec<(String, PathBuf)>> {
        let root = self.locations.dir();
        let entries = match fs::read_dir(root) {
            Ok(entries) => entries,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(source) => return Err(io_error("list", root, &source)),
        };
        let mut found = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|source| io_error("list", root, &source))?;
            let Ok(name) = entry.file_name().into_string() else {
                continue;
            };
            if name.starts_with(STAGING_PREFIX) || !entry.path().is_dir() {
                continue;
            }
            found.push((name, entry.path()));
        }
        found.sort();
        Ok(found)
    }
}

fn not_installed(name: &str) -> nurt_contracts::error::Error {
    error(
        ErrorCode::NotFound,
        format!("extension `{name}` is not installed"),
    )
}

fn remove_if_present(dir: &Path) -> Result<()> {
    match fs::remove_dir_all(dir) {
        Ok(()) => Ok(()),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(io_error("remove", dir, &source)),
    }
}

fn copy_tree(source: &Path, target: &Path) -> Result<()> {
    fs::create_dir(target).map_err(|problem| io_error("create", target, &problem))?;
    let entries = fs::read_dir(source).map_err(|problem| io_error("list", source, &problem))?;
    for entry in entries {
        let entry = entry.map_err(|problem| io_error("list", source, &problem))?;
        let from = entry.path();
        let to = target.join(entry.file_name());
        let kind = fs::symlink_metadata(&from)
            .map_err(|problem| io_error("read", &from, &problem))?
            .file_type();
        if kind.is_dir() {
            copy_tree(&from, &to)?;
        } else if kind.is_file() {
            fs::copy(&from, &to).map_err(|problem| io_error("copy", &from, &problem))?;
        } else {
            return Err(error(
                ErrorCode::Unsupported,
                format!(
                    "{} is not a regular file or directory; extension directories hold files only",
                    from.display()
                ),
            ));
        }
    }
    Ok(())
}

fn switch_in(staging: &Path, target: &Path, previous: &Path) -> Result<()> {
    let replacing = target.exists();
    if replacing {
        remove_if_present(previous)?;
        fs::rename(target, previous).map_err(|source| io_error("move", target, &source))?;
    }
    if let Err(source) = fs::rename(staging, target) {
        if replacing {
            fs::rename(previous, target)
                .map_err(|problem| io_error("restore", target, &problem))?;
        }
        return Err(io_error("install", target, &source));
    }
    if replacing {
        remove_if_present(previous)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::fs;
    use std::path::Path;

    use nurt_contracts::error::ErrorCode;

    use super::ExtStore;
    use crate::locations::ExtLocations;
    use crate::manifest::{ExtKind, MANIFEST_FILE};

    fn manifest(name: &str, version: &str) -> String {
        format!(
            "[ext]\nname = \"{name}\"\nversion = \"{version}\"\napi = \"nurt:ext@1.0\"\nkind = \"wasm\"\nwasm = \"ext.wasm\"\n\n[[stage]]\nname = \"lint\"\ninput = [\"entry\"]\n\n[[grammar]]\nlanguage = \"acme\"\nwasm = \"grammar/acme.wasm\"\n"
        )
    }

    fn source(root: &Path, name: &str, version: &str) -> std::path::PathBuf {
        let dir = root.join(format!("src-{name}-{version}"));
        fs::create_dir_all(dir.join("grammar")).unwrap();
        fs::write(dir.join(MANIFEST_FILE), manifest(name, version)).unwrap();
        fs::write(dir.join("ext.wasm"), version).unwrap();
        fs::write(dir.join("grammar/acme.wasm"), b"grammar").unwrap();
        dir
    }

    fn store(root: &Path) -> ExtStore {
        let core: BTreeSet<String> = ["git".to_owned()].into_iter().collect();
        ExtStore::new(ExtLocations::new(root.join("ext")), core)
    }

    #[test]
    fn a_source_that_holds_the_extension_directory_is_refused() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        fs::create_dir_all(root.join("grammar")).unwrap();
        fs::write(root.join(MANIFEST_FILE), manifest("acme", "1.0.0")).unwrap();
        fs::write(root.join("ext.wasm"), "1.0.0").unwrap();
        fs::write(root.join("grammar/acme.wasm"), b"grammar").unwrap();
        let error = store(root).install(root, false).unwrap_err();
        assert_eq!(error.code(), ErrorCode::InvalidValue);
        assert!(!root.join("ext/acme").exists());
    }

    #[test]
    fn an_installation_leaves_no_staging_directory() {
        let temp = tempfile::tempdir().unwrap();
        let store = store(temp.path());
        let source = source(temp.path(), "acme", "1.0.0");
        store.install(&source, false).unwrap();
        store.install(&source, true).unwrap();
        let names: Vec<String> = fs::read_dir(temp.path().join("ext"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["acme"]);
    }

    #[test]
    fn installs_lists_and_removes_an_extension() {
        let temp = tempfile::tempdir().unwrap();
        let store = store(temp.path());
        let installed = store
            .install(&source(temp.path(), "acme", "1.0.0"), false)
            .unwrap();
        assert_eq!(installed.name(), "acme");
        assert_eq!(installed.dir, temp.path().join("ext/acme"));
        assert_eq!(
            fs::read_to_string(installed.dir.join("ext.wasm")).unwrap(),
            "1.0.0"
        );
        assert!(installed.dir.join("grammar/acme.wasm").is_file());

        let listed = store.list().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].commands, ["acme.lint"]);
        assert_eq!(listed[0].kind, Some(ExtKind::Wasm));
        assert!(listed[0].problem.is_none());
        assert_eq!(store.installed().unwrap().len(), 1);
        assert_eq!(store.get("acme").unwrap(), installed);

        let removed = store.remove("acme").unwrap();
        assert_eq!(removed, installed.dir);
        assert!(store.list().unwrap().is_empty());
        assert_eq!(
            store.remove("acme").unwrap_err().code(),
            ErrorCode::NotFound
        );
        assert_eq!(store.get("acme").unwrap_err().code(), ErrorCode::NotFound);
    }

    #[test]
    fn installed_versions_are_replaced_only_on_request() {
        let temp = tempfile::tempdir().unwrap();
        let store = store(temp.path());
        store
            .install(&source(temp.path(), "acme", "1.0.0"), false)
            .unwrap();
        let again = store.install(&source(temp.path(), "acme", "2.0.0"), false);
        assert_eq!(again.unwrap_err().code(), ErrorCode::Exists);
        let replaced = store
            .install(&source(temp.path(), "acme", "2.0.0"), true)
            .unwrap();
        assert_eq!(replaced.manifest.ext.version, "2.0.0");
        assert_eq!(
            fs::read_to_string(replaced.dir.join("ext.wasm")).unwrap(),
            "2.0.0"
        );
        let names: Vec<String> = fs::read_dir(temp.path().join("ext"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(names, ["acme"]);
    }

    #[test]
    fn rejected_extensions_are_not_installed() {
        let temp = tempfile::tempdir().unwrap();
        let store = store(temp.path());
        let core = source(temp.path(), "git", "1.0.0");
        assert_eq!(
            store.install(&core, false).unwrap_err().code(),
            ErrorCode::Catalog
        );
        let missing = source(temp.path(), "acme", "1.0.0");
        fs::remove_file(missing.join("grammar/acme.wasm")).unwrap();
        assert_eq!(
            store.install(&missing, false).unwrap_err().code(),
            ErrorCode::NotFound
        );
        let empty = temp.path().join("empty");
        fs::create_dir(&empty).unwrap();
        assert_eq!(
            store.install(&empty, false).unwrap_err().code(),
            ErrorCode::NotFound
        );
        let native = source(temp.path(), "lib", "1.0.0");
        let text = fs::read_to_string(native.join(MANIFEST_FILE))
            .unwrap()
            .replace(
                "kind = \"wasm\"\nwasm = \"ext.wasm\"",
                "kind = \"native\"\nlibrary = \"ext.wasm\"",
            );
        fs::write(native.join(MANIFEST_FILE), text).unwrap();
        assert_eq!(
            store.install(&native, false).unwrap_err().code(),
            ErrorCode::Unsupported
        );
        assert!(!temp.path().join("ext/acme").exists());
    }

    #[cfg(unix)]
    #[test]
    fn symbolic_links_in_an_extension_are_refused() {
        let temp = tempfile::tempdir().unwrap();
        let store = store(temp.path());
        let dir = source(temp.path(), "acme", "1.0.0");
        std::os::unix::fs::symlink("/etc", dir.join("link")).unwrap();
        assert_eq!(
            store.install(&dir, false).unwrap_err().code(),
            ErrorCode::Unsupported
        );
        assert!(!temp.path().join("ext/acme").exists());
        assert!(store.list().unwrap().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn a_failed_replacement_keeps_the_installed_version() {
        let temp = tempfile::tempdir().unwrap();
        let store = store(temp.path());
        store
            .install(&source(temp.path(), "acme", "1.0.0"), false)
            .unwrap();
        let broken = source(temp.path(), "acme", "2.0.0");
        std::os::unix::fs::symlink("/etc/hostname", broken.join("grammar/link")).unwrap();
        let error = store.install(&broken, true).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Unsupported);
        assert_eq!(store.get("acme").unwrap().manifest.ext.version, "1.0.0");
        let names: Vec<String> = fs::read_dir(temp.path().join("ext"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(names, ["acme"]);
    }

    #[cfg(unix)]
    #[test]
    fn a_manifest_file_that_is_a_link_is_not_installed() {
        let temp = tempfile::tempdir().unwrap();
        let store = store(temp.path());
        let dir = source(temp.path(), "acme", "1.0.0");
        let outside = temp.path().join("outside.wasm");
        fs::write(&outside, b"x").unwrap();
        fs::remove_file(dir.join("ext.wasm")).unwrap();
        std::os::unix::fs::symlink(&outside, dir.join("ext.wasm")).unwrap();
        assert_eq!(
            store.install(&dir, false).unwrap_err().code(),
            ErrorCode::Unsupported
        );
        assert!(!temp.path().join("ext/acme").exists());
    }

    #[test]
    fn names_outside_the_directory_are_refused_everywhere() {
        let temp = tempfile::tempdir().unwrap();
        let store = store(temp.path());
        for name in ["..", "../acme", "a/b", "", "Acme", "a b", "żółw"] {
            assert_eq!(
                store.get(name).unwrap_err().code(),
                ErrorCode::InvalidValue,
                "{name}"
            );
            assert_eq!(
                store.remove(name).unwrap_err().code(),
                ErrorCode::InvalidValue,
                "{name}"
            );
        }
    }

    #[test]
    fn a_manifest_written_by_a_windows_editor_is_read() {
        let temp = tempfile::tempdir().unwrap();
        let text = manifest("acme", "1.0.0");
        let units: Vec<u16> = text.encode_utf16().collect();
        let mut utf8_marked = vec![0xEF, 0xBB, 0xBF];
        utf8_marked.extend_from_slice(text.as_bytes());
        let mut little = vec![0xFF, 0xFE];
        little.extend(units.iter().flat_map(|unit| unit.to_le_bytes()));
        let mut big = vec![0xFE, 0xFF];
        big.extend(units.iter().flat_map(|unit| unit.to_be_bytes()));
        let path = temp.path().join(MANIFEST_FILE);
        for bytes in [text.as_bytes(), &utf8_marked, &little, &big] {
            fs::write(&path, bytes).unwrap();
            assert_eq!(super::read_manifest_text(&path).unwrap(), text);
        }
        fs::write(&path, [0xFF, 0xFE, 0x00, 0xD8]).unwrap();
        assert!(super::read_manifest_text(&path).is_err());

        let source = source(temp.path(), "acme", "1.0.0");
        fs::write(source.join(MANIFEST_FILE), &little).unwrap();
        let store = store(temp.path());
        let installed = store.install(&source, false).unwrap();
        assert_eq!(installed.manifest.ext.name, "acme");
        assert_eq!(store.installed().unwrap().len(), 1);
    }

    #[test]
    fn a_missing_directory_has_no_extensions() {
        let temp = tempfile::tempdir().unwrap();
        let store = store(temp.path());
        assert!(store.list().unwrap().is_empty());
        assert!(store.installed().unwrap().is_empty());
        assert_eq!(store.locations().dir(), temp.path().join("ext"));
    }

    #[test]
    fn unusable_directories_are_listed_with_their_problem() {
        let temp = tempfile::tempdir().unwrap();
        let store = store(temp.path());
        let installed = store
            .install(&source(temp.path(), "acme", "1.0.0"), false)
            .unwrap();
        fs::rename(&installed.dir, temp.path().join("ext/other")).unwrap();
        fs::create_dir_all(temp.path().join("ext/broken")).unwrap();
        fs::write(temp.path().join("ext/broken").join(MANIFEST_FILE), "[ext").unwrap();
        fs::write(temp.path().join("ext/notes.txt"), "x").unwrap();

        let listed = store.list().unwrap();
        let names: Vec<&str> = listed.iter().map(|entry| entry.name.as_str()).collect();
        assert_eq!(names, ["broken", "other"]);
        assert!(listed.iter().all(|entry| entry.problem.is_some()));
        assert!(store.installed().unwrap().is_empty());
        assert_eq!(store.get("other").unwrap_err().code(), ErrorCode::Catalog);
        store.remove("broken").unwrap();
        assert!(!temp.path().join("ext/broken").exists());
        assert_eq!(
            store.remove("../x").unwrap_err().code(),
            ErrorCode::InvalidValue
        );
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
