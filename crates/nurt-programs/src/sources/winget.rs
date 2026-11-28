// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-programs::sources::winget
// Responsibility:   Windows Package Manager packages and links.
// Dependencies:     nurt_contracts, rusqlite
// Author:           Danaco Official
// Version:          0.1.2, 2026-11-28
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Windows Package Manager packages and links.
//!
//! The provider reads the command links in `%LOCALAPPDATA%\Microsoft\WinGet\Links`,
//! the portable packages in `…\WinGet\Packages`, the uninstall entries that
//! carry `WinGetPackageIdentifier` and, when readable, the installation
//! tracking database of the package manager. Paths and the schema of the
//! tracking database follow the layout of WinGet 1.x; a database with a
//! different schema is skipped with a warning.
//!
//! The tracking database is a record of what was installed, not of what is
//! installed: the package manager keeps the row of a removed package and one
//! row per version of an upgraded one. A row is therefore reported only when
//! the system confirms it, as [`installed_row`] describes.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use nurt_contracts::env::Os;
use rusqlite::{Connection, OpenFlags};

use crate::model::{
    CategoryHint, CategoryScheme, LocalizedText, PackageRef, RawPackage, RawProgram, SourceData,
};
use crate::source::{
    Deadline, Fingerprint, FingerprintBuilder, HostEnv, ProgramSource, RegistryHive, RegistryView,
    SourceError, SourceId, SourceKind, SourceWarning, WarningCode,
};

/// Uninstall key read for entries of portable packages.
pub const UNINSTALL_KEY: &str = "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Uninstall";
/// Directory of the tracking database below `%LOCALAPPDATA%`.
pub const TRACKING_DIR: &str = "Packages\\Microsoft.DesktopAppInstaller_8wekyb3d8bbwe\\LocalState\\Microsoft.Winget.Source_8wekyb3d8bbwe";
/// File name of the tracking database.
pub const TRACKING_DB: &str = "installed.db";
const EXE_EXTENSION: &str = ".exe";

const MANIFEST_QUERY: &str = "SELECT manifest.rowid, ids.id, names.name, versions.version, monikers.moniker \
     FROM manifest \
     JOIN ids ON manifest.id = ids.rowid \
     JOIN names ON manifest.name = names.rowid \
     JOIN versions ON manifest.version = versions.rowid \
     LEFT JOIN monikers ON manifest.moniker = monikers.rowid \
     ORDER BY manifest.rowid";
const TAGS_QUERY: &str = "SELECT tags_map.manifest, tags.tag FROM tags_map \
     JOIN tags ON tags_map.tag = tags.rowid";
const PRODUCT_CODES_QUERY: &str = "SELECT productcodes_map.manifest, productcodes.productcode \
     FROM productcodes_map \
     JOIN productcodes ON productcodes_map.productcode = productcodes.rowid";
const NORMALIZED_NAMES_QUERY: &str = "SELECT norm_names_map.manifest, norm_names.norm_name \
     FROM norm_names_map \
     JOIN norm_names ON norm_names_map.norm_name = norm_names.rowid";
/// Words of a program name that state the processor architecture.
const ARCHITECTURE_WORDS: [&str; 10] = [
    "x64", "x86", "amd64", "arm64", "win64", "win32", "64-bit", "32-bit", "64bit", "32bit",
];
/// Endings of a comparison name that state the processor architecture.
const ARCHITECTURE_ENDINGS: [&str; 4] = ["x64", "x86", "amd64", "arm64"];

/// Provider of Windows Package Manager packages.
#[derive(Debug, Clone)]
pub struct WingetSource {
    id: SourceId,
}

impl Default for WingetSource {
    fn default() -> Self {
        Self::new()
    }
}

impl WingetSource {
    /// Provider with the identifier `winget`.
    #[must_use]
    pub fn new() -> Self {
        Self {
            id: SourceId::new(SourceId::WINGET),
        }
    }
}

/// Row of the tracking database: one version of a package that the package
/// manager installed.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TrackedPackage {
    /// Package identifier, for example `BurntSushi.ripgrep.MSVC`.
    pub id: String,
    /// Display name.
    pub name: Option<String>,
    /// Version of the row.
    pub version: Option<String>,
    /// Short command name.
    pub moniker: Option<String>,
    /// Tags.
    pub tags: Vec<String>,
    /// Names of the uninstall keys that the installer of the row writes, in
    /// lower case; empty when the database records none.
    pub product_codes: Vec<String>,
    /// Names of the program in the form of [`comparison_name`], from the
    /// display name of the row and the names that the database records for
    /// the comparison with the uninstall entries.
    pub comparison_names: Vec<String>,
}

/// The name of a program reduced for the comparison of an uninstall entry
/// with a row of the tracking database: lower case, without text in brackets,
/// without words that are a version or a processor architecture, letters and
/// digits only, and without a number or an architecture at the end.
///
/// # Examples
///
/// ```
/// use nurt_programs::sources::winget::comparison_name;
///
/// assert_eq!(comparison_name("Microsoft .NET SDK 10.0.401 (x64)"), "microsoftnetsdk");
/// assert_eq!(comparison_name("PowerShell 7-x64"), "powershell");
/// assert_eq!(comparison_name("7-Zip 26.04 (x64 edition)"), "7zip");
/// ```
#[must_use]
pub fn comparison_name(name: &str) -> String {
    let mut open = 0_usize;
    let outside: String = name
        .chars()
        .filter(|character| match character {
            '(' | '[' => {
                open += 1;
                false
            }
            ')' | ']' => {
                open = open.saturating_sub(1);
                false
            }
            _ => open == 0,
        })
        .collect();
    let is_version = |word: &str| {
        let digits = word.strip_prefix('v').unwrap_or(word);
        digits.starts_with(|character: char| character.is_ascii_digit())
            && digits
                .chars()
                .all(|character| character.is_ascii_digit() || matches!(character, '.' | '_'))
    };
    let mut reduced: String = outside
        .to_lowercase()
        .split_whitespace()
        .filter(|word| !is_version(word) && !ARCHITECTURE_WORDS.contains(word))
        .flat_map(str::chars)
        .filter(|character| character.is_alphanumeric())
        .collect();
    loop {
        let kept = ARCHITECTURE_ENDINGS
            .iter()
            .find_map(|ending| reduced.strip_suffix(ending))
            .unwrap_or(reduced.as_str())
            .trim_end_matches(|character: char| character.is_ascii_digit())
            .len();
        // A name that consists of a number or of an architecture only is kept.
        if kept == 0 || kept == reduced.len() {
            return reduced;
        }
        reduced.truncate(kept);
    }
}

/// Reads the rows of a tracking database, opened read-only, in the order in
/// which the package manager wrote them.
///
/// A package has one row per version that was installed, and the row stays
/// when the package is removed.
///
/// # Errors
///
/// Returns a short reason when the file cannot be opened or its schema
/// differs from the expected tables `manifest`, `ids`, `names`, `versions`,
/// `monikers`, `tags` and `tags_map`.
pub fn read_tracking_db(path: &Path) -> Result<Vec<TrackedPackage>, String> {
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let connection = Connection::open_with_flags(path, flags)
        .map_err(|error| format!("tracking database cannot be opened: {error}"))?;
    let schema =
        |error: rusqlite::Error| format!("tracking database has an unknown schema: {error}");
    let mut packages: Vec<(i64, TrackedPackage)> = Vec::new();
    let mut statement = connection.prepare(MANIFEST_QUERY).map_err(schema)?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                TrackedPackage {
                    id: row.get(1)?,
                    name: row.get(2)?,
                    version: row.get(3)?,
                    moniker: row.get(4)?,
                    tags: Vec::new(),
                    product_codes: Vec::new(),
                    comparison_names: Vec::new(),
                },
            ))
        })
        .map_err(schema)?;
    for row in rows {
        let (manifest, mut package) = row.map_err(schema)?;
        package
            .comparison_names
            .extend(package.name.as_deref().map(comparison_name));
        packages.push((manifest, package));
    }
    let by_manifest =
        |row: &rusqlite::Row<'_>| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?));
    if let Ok(mut statement) = connection.prepare(TAGS_QUERY) {
        let rows = statement.query_map([], by_manifest).map_err(schema)?;
        for (manifest, tag) in rows.flatten() {
            if let Some((_, package)) = packages.iter_mut().find(|(row, _)| *row == manifest) {
                package.tags.push(tag);
            }
        }
    }
    if let Ok(mut statement) = connection.prepare(PRODUCT_CODES_QUERY) {
        let rows = statement.query_map([], by_manifest).map_err(schema)?;
        for (manifest, code) in rows.flatten() {
            if let Some((_, package)) = packages.iter_mut().find(|(row, _)| *row == manifest) {
                package.product_codes.push(code.to_lowercase());
            }
        }
    }
    if let Ok(mut statement) = connection.prepare(NORMALIZED_NAMES_QUERY) {
        let rows = statement.query_map([], by_manifest).map_err(schema)?;
        for (manifest, name) in rows.flatten() {
            if let Some((_, package)) = packages.iter_mut().find(|(row, _)| *row == manifest) {
                package.comparison_names.push(comparison_name(&name));
            }
        }
    }
    Ok(packages.into_iter().map(|(_, package)| package).collect())
}

/// Package identifier of a portable package directory
/// `<identifier>_<source>`, for example
/// `BurntSushi.ripgrep.MSVC_Microsoft.Winget.Source_8wekyb3d8bbwe`.
///
/// # Examples
///
/// ```
/// use nurt_programs::sources::winget::package_of_dir;
///
/// assert_eq!(
///     package_of_dir("BurntSushi.ripgrep.MSVC_Microsoft.Winget.Source_8wekyb3d8bbwe").as_deref(),
///     Some("BurntSushi.ripgrep.MSVC")
/// );
/// ```
#[must_use]
pub fn package_of_dir(name: &str) -> Option<String> {
    // The source family name is `<name>_<publisher hash>` and holds one
    // underscore, so the identifier ends at the second underscore from the
    // end; an identifier may contain underscores itself.
    let (id, source) = match name.rmatch_indices('_').nth(1) {
        Some((at, _)) => (&name[..at], &name[at + 1..]),
        None => name.split_once('_')?,
    };
    (!id.is_empty() && !source.is_empty()).then(|| id.to_owned())
}

/// Package identifier of the target of a link: the directory below
/// `Packages` in the target path.
#[must_use]
pub fn package_of_link(target: &str) -> Option<String> {
    let parts: Vec<&str> = target.split(['\\', '/']).collect();
    let at = parts
        .iter()
        .position(|part| part.eq_ignore_ascii_case("Packages"))?;
    package_of_dir(parts.get(at + 1)?)
}

/// Directory `%LOCALAPPDATA%\Microsoft\WinGet` as a host path.
fn winget_dir(env: &HostEnv) -> Option<PathBuf> {
    let local = env.var_text("LOCALAPPDATA")?;
    Some(Path::new(local).join("Microsoft").join("WinGet"))
}

fn tracking_db(env: &HostEnv) -> Option<PathBuf> {
    let local = env.var_text("LOCALAPPDATA")?;
    Some(Path::new(local).join(TRACKING_DIR).join(TRACKING_DB))
}

fn names_in(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .flatten()
        .filter_map(|entry| entry.file_name().to_str().map(str::to_owned))
        .collect();
    names.sort();
    names
}

/// Information about one package, merged from every place that names it.
#[derive(Debug, Clone, Default)]
struct Package {
    name: Option<String>,
    version: Option<String>,
    publisher: Option<String>,
    tags: Vec<String>,
    commands: Vec<(String, String)>,
}

/// Uninstall entry of the system.
#[derive(Debug, Clone, Default)]
struct UninstallEntry {
    /// Name of the key below the uninstall key.
    key: String,
    /// Display name in the form of [`comparison_name`]; empty without a name.
    comparison_name: String,
    /// Value `WinGetPackageIdentifier`, which the package manager writes for
    /// a portable package.
    package: Option<String>,
    name: Option<String>,
    version: Option<String>,
    publisher: Option<String>,
}

/// The row of the tracking database that describes the installed state of
/// the package `id`; `None` when the package is not installed.
///
/// A row counts when the package has a directory or a link (`present`), when
/// an uninstall entry names the package, carries one of the product codes of
/// the row or shows a name of the row, and when the row has no product code,
/// so that nothing can be checked. The name is compared because an installer
/// that updates the program outside the package manager writes a new product
/// code. Without a list of the uninstall entries (`entries` is `None`: the
/// registry was not read) every row counts. Of several rows that count, the
/// one whose version an uninstall entry shows is taken, otherwise the one
/// written last.
fn installed_row<'a>(
    id: &str,
    rows: &'a [TrackedPackage],
    entries: Option<&[UninstallEntry]>,
    present: bool,
) -> Option<&'a TrackedPackage> {
    let Some(entries) = entries else {
        return rows.last();
    };
    let confirms = |row: &TrackedPackage, entry: &UninstallEntry| {
        entry.package.as_deref() == Some(id)
            || row
                .product_codes
                .iter()
                .any(|code| code.eq_ignore_ascii_case(&entry.key))
            || (!entry.comparison_name.is_empty()
                && row.comparison_names.contains(&entry.comparison_name))
    };
    let counted: Vec<&TrackedPackage> = rows
        .iter()
        .filter(|row| {
            present
                || row.product_codes.is_empty()
                || entries.iter().any(|entry| confirms(row, entry))
        })
        .collect();
    let shown = counted.iter().rev().find(|row| {
        row.version.is_some()
            && entries
                .iter()
                .any(|entry| confirms(row, entry) && entry.version == row.version)
    });
    shown.or(counted.last()).copied()
}

impl WingetSource {
    /// The uninstall entries of the account and of the machine; `None` when
    /// the host has no registry or none of its uninstall keys could be read.
    fn uninstall_entries(
        &self,
        env: &HostEnv,
        data: &mut SourceData,
    ) -> Option<Vec<UninstallEntry>> {
        let registry = env.registry()?;
        let views = [
            (RegistryHive::CurrentUser, RegistryView::Native),
            (RegistryHive::LocalMachine, RegistryView::Native),
            (RegistryHive::LocalMachine, RegistryView::Wow32),
        ];
        let mut entries = Vec::new();
        let mut read = false;
        for (hive, view) in views {
            let subkeys = match registry.subkeys(hive, view, UNINSTALL_KEY) {
                Ok(subkeys) => subkeys.unwrap_or_default(),
                Err(error) => {
                    data.warnings.push(SourceWarning::from(error));
                    continue;
                }
            };
            read = true;
            for subkey in subkeys {
                let key = format!("{UNINSTALL_KEY}\\{subkey}");
                let values = registry
                    .values(hive, view, &key)
                    .ok()
                    .flatten()
                    .unwrap_or_default();
                let text = |name: &str| {
                    values
                        .get(name)
                        .and_then(|value| value.as_text())
                        .map(str::to_owned)
                };
                let name = text("DisplayName");
                entries.push(UninstallEntry {
                    comparison_name: name.as_deref().map(comparison_name).unwrap_or_default(),
                    package: text("WinGetPackageIdentifier"),
                    name,
                    version: text("DisplayVersion"),
                    publisher: text("Publisher"),
                    key: subkey,
                });
            }
        }
        read.then_some(entries)
    }
}

impl ProgramSource for WingetSource {
    fn id(&self) -> &SourceId {
        &self.id
    }

    fn kind(&self) -> SourceKind {
        SourceKind::Package
    }

    fn detect(&self, env: &HostEnv) -> bool {
        env.os() == Os::Windows
            && winget_dir(env).is_some_and(|dir| {
                env.sys_path(dir.join("Links")).is_dir()
                    || env.sys_path(dir.join("Packages")).is_dir()
            })
    }

    fn fingerprint(&self, env: &HostEnv) -> Result<Fingerprint, SourceError> {
        let mut builder = FingerprintBuilder::new(env);
        if let Some(dir) = winget_dir(env) {
            builder = builder.path(dir.join("Links")).path(dir.join("Packages"));
        }
        if let Some(db) = tracking_db(env) {
            builder = builder.path(db);
        }
        builder = builder
            .registry_key(
                RegistryHive::CurrentUser,
                RegistryView::Native,
                UNINSTALL_KEY,
            )
            .registry_key(
                RegistryHive::LocalMachine,
                RegistryView::Native,
                UNINSTALL_KEY,
            )
            .registry_key(
                RegistryHive::LocalMachine,
                RegistryView::Wow32,
                UNINSTALL_KEY,
            );
        Ok(builder.finish())
    }

    fn load(&self, env: &HostEnv, deadline: &Deadline<'_>) -> Result<SourceData, SourceError> {
        let mut data = SourceData::default();
        let mut packages: BTreeMap<String, Package> = BTreeMap::new();
        if let Some(dir) = winget_dir(env) {
            for name in names_in(&env.sys_path(dir.join("Packages"))) {
                if let Some(id) = package_of_dir(&name) {
                    packages.entry(id).or_default();
                }
            }
            let links = dir.join("Links");
            for file in names_in(&env.sys_path(&links)) {
                deadline.check(&self.id)?;
                let lower = file.to_ascii_lowercase();
                let Some(command) = lower
                    .strip_suffix(EXE_EXTENSION)
                    .map(|_| &file[..file.len() - EXE_EXTENSION.len()])
                else {
                    continue;
                };
                let Ok(target) = std::fs::read_link(env.sys_path(links.join(&file))) else {
                    continue;
                };
                let Some(id) = package_of_link(&target.to_string_lossy()) else {
                    continue;
                };
                let path = links.join(&file).display().to_string();
                packages
                    .entry(id)
                    .or_default()
                    .commands
                    .push((command.to_owned(), path));
            }
        }
        let entries = self.uninstall_entries(env, &mut data);
        if let Some(db) = tracking_db(env) {
            let file = env.sys_path(&db);
            if file.is_file() {
                match read_tracking_db(&file) {
                    Ok(tracked) => {
                        let mut rows: BTreeMap<String, Vec<TrackedPackage>> = BTreeMap::new();
                        for row in tracked {
                            rows.entry(row.id.clone()).or_default().push(row);
                        }
                        for (id, rows) in rows {
                            let present = packages.contains_key(&id);
                            let Some(row) = installed_row(&id, &rows, entries.as_deref(), present)
                            else {
                                continue;
                            };
                            let package = packages.entry(id).or_default();
                            package.name = package.name.take().or_else(|| row.name.clone());
                            package.version =
                                package.version.take().or_else(|| row.version.clone());
                            package.tags.clone_from(&row.tags);
                        }
                    }
                    Err(reason) => data.warnings.push(
                        SourceWarning::new(WarningCode::SourceUnavailable, &self.id, reason)
                            .at(db.display().to_string()),
                    ),
                }
            }
        }
        for entry in entries.unwrap_or_default() {
            let Some(id) = entry.package else {
                continue;
            };
            let package = packages.entry(id).or_default();
            package.name = package.name.take().or(entry.name);
            package.version = package.version.take().or(entry.version);
            package.publisher = package.publisher.take().or(entry.publisher);
        }
        for (id, package) in packages {
            let reference = PackageRef {
                manager: self.id.clone(),
                name: id.clone(),
            };
            let summary = package.name.as_deref().map(LocalizedText::new);
            let mut hints = vec![CategoryHint::new(CategoryScheme::Keyword, id.clone())];
            hints.extend(
                package
                    .tags
                    .iter()
                    .map(|tag| CategoryHint::new(CategoryScheme::Keyword, tag.clone())),
            );
            for (command, path) in &package.commands {
                data.file_packages.push((path.clone(), reference.clone()));
                data.programs.push(RawProgram {
                    path: Some(path.clone()),
                    summary: summary.clone(),
                    version: package.version.clone(),
                    package: Some(reference.clone()),
                    tags: package.tags.clone(),
                    category_hints: hints.clone(),
                    ..RawProgram::new(command.clone())
                });
            }
            data.packages.push(RawPackage {
                name: id,
                version: package.version,
                summary,
                origin: package.publisher,
                groups: package.tags,
                executables: package.commands.into_iter().map(|(_, path)| path).collect(),
                ..RawPackage::default()
            });
        }
        Ok(data)
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
