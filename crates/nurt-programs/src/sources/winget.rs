// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Windows Package Manager packages and links.
//!
//! The provider reads the command links in `%LOCALAPPDATA%\Microsoft\WinGet\Links`,
//! the portable packages in `…\WinGet\Packages`, the uninstall entries that
//! carry `WinGetPackageIdentifier` and, when readable, the installation
//! tracking database of the package manager. Paths and the schema of the
//! tracking database follow the layout of WinGet 1.x; a database with a
//! different schema is skipped with a warning.

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

const MANIFEST_QUERY: &str = "SELECT ids.id, names.name, versions.version, monikers.moniker \
     FROM manifest \
     JOIN ids ON manifest.id = ids.rowid \
     JOIN names ON manifest.name = names.rowid \
     JOIN versions ON manifest.version = versions.rowid \
     LEFT JOIN monikers ON manifest.moniker = monikers.rowid";
const TAGS_QUERY: &str = "SELECT ids.id, tags.tag FROM tags_map \
     JOIN manifest ON tags_map.manifest = manifest.rowid \
     JOIN ids ON manifest.id = ids.rowid \
     JOIN tags ON tags_map.tag = tags.rowid";

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

/// Package recorded in the tracking database.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TrackedPackage {
    /// Package identifier, for example `BurntSushi.ripgrep.MSVC`.
    pub id: String,
    /// Display name.
    pub name: Option<String>,
    /// Installed version.
    pub version: Option<String>,
    /// Short command name.
    pub moniker: Option<String>,
    /// Tags.
    pub tags: Vec<String>,
}

/// Reads the packages of a tracking database, opened read-only.
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
    let mut packages: BTreeMap<String, TrackedPackage> = BTreeMap::new();
    let mut statement = connection.prepare(MANIFEST_QUERY).map_err(schema)?;
    let rows = statement
        .query_map([], |row| {
            Ok(TrackedPackage {
                id: row.get(0)?,
                name: row.get(1)?,
                version: row.get(2)?,
                moniker: row.get(3)?,
                tags: Vec::new(),
            })
        })
        .map_err(schema)?;
    for row in rows {
        let package = row.map_err(schema)?;
        packages.insert(package.id.clone(), package);
    }
    if let Ok(mut statement) = connection.prepare(TAGS_QUERY) {
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(schema)?;
        for (id, tag) in rows.flatten() {
            if let Some(package) = packages.get_mut(&id) {
                package.tags.push(tag);
            }
        }
    }
    Ok(packages.into_values().collect())
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
    let (id, source) = name.split_once('_')?;
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

impl WingetSource {
    fn read_registry(
        &self,
        env: &HostEnv,
        packages: &mut BTreeMap<String, Package>,
        data: &mut SourceData,
    ) {
        let Some(registry) = env.registry() else {
            return;
        };
        let views = [
            (RegistryHive::CurrentUser, RegistryView::Native),
            (RegistryHive::LocalMachine, RegistryView::Native),
            (RegistryHive::LocalMachine, RegistryView::Wow32),
        ];
        for (hive, view) in views {
            let subkeys = match registry.subkeys(hive, view, UNINSTALL_KEY) {
                Ok(subkeys) => subkeys.unwrap_or_default(),
                Err(error) => {
                    data.warnings.push(SourceWarning::from(error));
                    continue;
                }
            };
            for subkey in subkeys {
                let key = format!("{UNINSTALL_KEY}\\{subkey}");
                let Ok(Some(values)) = registry.values(hive, view, &key) else {
                    continue;
                };
                let text = |name: &str| {
                    values
                        .get(name)
                        .and_then(|value| value.as_text())
                        .map(str::to_owned)
                };
                let Some(id) = text("WinGetPackageIdentifier") else {
                    continue;
                };
                let package = packages.entry(id).or_default();
                package.name = package.name.take().or_else(|| text("DisplayName"));
                package.version = package.version.take().or_else(|| text("DisplayVersion"));
                package.publisher = package.publisher.take().or_else(|| text("Publisher"));
            }
        }
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
        if let Some(db) = tracking_db(env) {
            let file = env.sys_path(&db);
            if file.is_file() {
                match read_tracking_db(&file) {
                    Ok(tracked) => {
                        for item in tracked {
                            let package = packages.entry(item.id).or_default();
                            package.name = package.name.take().or(item.name);
                            package.version = package.version.take().or(item.version);
                            package.tags = item.tags;
                        }
                    }
                    Err(reason) => data.warnings.push(
                        SourceWarning::new(WarningCode::SourceUnavailable, &self.id, reason)
                            .at(db.display().to_string()),
                    ),
                }
            }
        }
        self.read_registry(env, &mut packages, &mut data);
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
