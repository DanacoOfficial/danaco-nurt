// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Arch Linux package database.
//!
//! The database directory is `DBPath` of `/etc/pacman.conf`, by default
//! `/var/lib/pacman/`. Each installed package has a directory
//! `local/<name>-<version>-<release>/` with a `desc` file of `%SECTION%`
//! blocks and a `files` file whose `%FILES%` block lists paths relative to
//! the root.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::model::{
    CategoryHint, CategoryScheme, EntryKey, LocalizedText, PackageRef, RawPackage, RawProgram,
    SourceData,
};
use crate::source::{
    Deadline, Fingerprint, FingerprintBuilder, HostEnv, ProgramSource, SourceError,
    SourceErrorKind, SourceId, SourceKind, SourceWarning, WarningCode,
};
use crate::sources::{exists, in_bin_dir, is_dir, list_dir, read_file, rfc3339};

/// Configuration file of pacman.
pub const PACMAN_CONF: &str = "/etc/pacman.conf";
/// Default database directory.
pub const DEFAULT_DB_PATH: &str = "/var/lib/pacman";
const LOCAL_DIR: &str = "local";
const DESC_FILE: &str = "desc";
const FILES_FILE: &str = "files";
/// Value of `%REASON%` for packages installed as dependencies.
const REASON_DEPENDENCY: &str = "1";

/// Parses `%SECTION%` blocks: each block is a header line and the
/// following non-empty lines.
#[must_use]
pub fn parse_sections(text: &str) -> HashMap<String, Vec<String>> {
    let mut sections: HashMap<String, Vec<String>> = HashMap::new();
    let mut current: Option<String> = None;
    for line in text.lines() {
        let line = line.trim_end();
        if line.len() > 2 && line.starts_with('%') && line.ends_with('%') {
            let name = line[1..line.len() - 1].to_owned();
            sections.entry(name.clone()).or_default();
            current = Some(name);
        } else if line.is_empty() {
            current = None;
        } else if let Some(name) = &current {
            sections
                .entry(name.clone())
                .or_default()
                .push(line.to_owned());
        }
    }
    sections
}

fn first(sections: &HashMap<String, Vec<String>>, name: &str) -> Option<String> {
    sections
        .get(name)?
        .first()
        .cloned()
        .filter(|value| !value.is_empty())
}

/// Package of a `desc` file; `None` without `%NAME%`.
#[must_use]
pub fn package_from_desc(text: &str) -> Option<RawPackage> {
    let sections = parse_sections(text);
    let name = first(&sections, "NAME")?;
    Some(RawPackage {
        name,
        version: first(&sections, "VERSION"),
        architecture: first(&sections, "ARCH"),
        summary: first(&sections, "DESC").map(|text| LocalizedText::in_lang(text, "en")),
        homepage: first(&sections, "URL"),
        license: sections
            .get("LICENSE")
            .map(|items| items.join(" AND "))
            .filter(|text| !text.is_empty()),
        groups: sections.get("GROUPS").cloned().unwrap_or_default(),
        origin: first(&sections, "BASE"),
        installed_size: first(&sections, "SIZE").and_then(|size| size.parse().ok()),
        installed_at: first(&sections, "INSTALLDATE")
            .and_then(|seconds| seconds.parse::<i64>().ok())
            .map(rfc3339),
        explicit: Some(first(&sections, "REASON").as_deref() != Some(REASON_DEPENDENCY)),
        ..RawPackage::default()
    })
}

/// Executables of a `files` file: entries of `%FILES%` in binary
/// directories, as absolute paths.
#[must_use]
pub fn executables_from_files(text: &str) -> Vec<String> {
    parse_sections(text)
        .remove("FILES")
        .unwrap_or_default()
        .into_iter()
        .map(|path| format!("/{}", path.trim_start_matches('/')))
        .filter(|path| in_bin_dir(path))
        .collect()
}

/// `DBPath` of a `pacman.conf` text, when set.
#[must_use]
pub fn db_path_from_conf(text: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let line = line.split('#').next().unwrap_or_default().trim();
        let (key, value) = line.split_once('=')?;
        (key.trim() == "DBPath")
            .then(|| value.trim().to_owned())
            .filter(|value| !value.is_empty())
    })
}

/// Database directory: `DBPath` of the configuration or the default.
#[must_use]
pub fn db_path(env: &HostEnv) -> PathBuf {
    let id = SourceId::new(SourceId::PACMAN);
    read_file(env, &id, Path::new(PACMAN_CONF))
        .ok()
        .and_then(|bytes| db_path_from_conf(&String::from_utf8_lossy(&bytes)))
        .map_or_else(|| PathBuf::from(DEFAULT_DB_PATH), PathBuf::from)
}

/// Provider of the pacman database.
#[derive(Debug, Clone)]
pub struct PacmanSource {
    id: SourceId,
}

impl PacmanSource {
    /// Creates the provider.
    #[must_use]
    pub fn new() -> Self {
        Self {
            id: SourceId::new(SourceId::PACMAN),
        }
    }
}

impl Default for PacmanSource {
    fn default() -> Self {
        Self::new()
    }
}

impl ProgramSource for PacmanSource {
    fn id(&self) -> &SourceId {
        &self.id
    }

    fn kind(&self) -> SourceKind {
        SourceKind::Package
    }

    fn detect(&self, env: &HostEnv) -> bool {
        is_dir(env, &db_path(env).join(LOCAL_DIR))
    }

    fn fingerprint(&self, env: &HostEnv) -> Result<Fingerprint, SourceError> {
        Ok(FingerprintBuilder::new(env)
            .path(PACMAN_CONF)
            .path(db_path(env).join(LOCAL_DIR))
            .finish())
    }

    fn load(&self, env: &HostEnv, deadline: &Deadline<'_>) -> Result<SourceData, SourceError> {
        read_with_deadline(env, deadline, &self.id)
    }
}

/// Reads the installed packages, their executables and the programs they
/// provide.
///
/// # Errors
///
/// Fails when the `local` directory of the database does not exist.
pub fn read_packages(env: &HostEnv) -> Result<SourceData, SourceError> {
    read_with_deadline(env, &Deadline::none(), &SourceId::new(SourceId::PACMAN))
}

fn read_with_deadline(
    env: &HostEnv,
    deadline: &Deadline<'_>,
    id: &SourceId,
) -> Result<SourceData, SourceError> {
    let local = db_path(env).join(LOCAL_DIR);
    if !is_dir(env, &local) {
        return Err(
            SourceError::new(id, SourceErrorKind::Unavailable, "no local database")
                .at(local.display().to_string()),
        );
    }
    let mut data = SourceData::default();
    for (_, dir) in list_dir(env, &local) {
        deadline.check(id)?;
        let desc = dir.join(DESC_FILE);
        if !exists(env, &desc) {
            continue;
        }
        let package = read_file(env, id, &desc)
            .map(|bytes| package_from_desc(&String::from_utf8_lossy(&bytes)));
        match package {
            Ok(Some(mut package)) => {
                package.executables = read_file(env, id, &dir.join(FILES_FILE))
                    .map(|bytes| executables_from_files(&String::from_utf8_lossy(&bytes)))
                    .unwrap_or_default();
                add_package(&mut data, package, id);
            }
            Ok(None) => data.warnings.push(
                SourceWarning::new(WarningCode::SourceUnavailable, id, "package without name")
                    .at(desc.display().to_string()),
            ),
            Err(error) => data.warnings.push(error.into()),
        }
    }
    Ok(data)
}

fn add_package(data: &mut SourceData, package: RawPackage, id: &SourceId) {
    let reference = PackageRef {
        manager: id.clone(),
        name: package.name.clone(),
    };
    for file in &package.executables {
        data.file_packages.push((file.clone(), reference.clone()));
        let mut program = RawProgram::new(file.rsplit('/').next().unwrap_or(file));
        program.key = Some(EntryKey::new(id, package.name.clone()));
        program.path = Some(file.clone());
        program.package = Some(reference.clone());
        program.version.clone_from(&package.version);
        program.homepage.clone_from(&package.homepage);
        program.license.clone_from(&package.license);
        program.category_hints.extend(
            package
                .groups
                .iter()
                .map(|group| CategoryHint::new(CategoryScheme::PacmanGroup, group.clone())),
        );
        data.programs.push(program);
    }
    data.packages.push(package);
}

#[cfg(test)]
mod tests {
    use super::*;

    const DESC: &str = "%NAME%\nripgrep\n\n%VERSION%\n14.1.0-1\n\n%DESC%\nA search tool\n\n%URL%\nhttps://example.org/rg\n\n%LICENSE%\nMIT\nUnlicense\n\n%GROUPS%\nutils\n\n%REASON%\n1\n\n%INSTALLDATE%\n0\n";

    #[test]
    fn pacman_desc_parse() {
        let package = package_from_desc(DESC).unwrap();
        assert_eq!(package.name, "ripgrep");
        assert_eq!(package.version.as_deref(), Some("14.1.0-1"));
        assert_eq!(package.summary.unwrap().text, "A search tool");
        assert_eq!(package.license.as_deref(), Some("MIT AND Unlicense"));
        assert_eq!(package.groups, vec!["utils"]);
        assert_eq!(package.explicit, Some(false));
        assert_eq!(
            package.installed_at.as_deref(),
            Some("1970-01-01T00:00:00Z")
        );
        assert!(package_from_desc("%VERSION%\n1\n").is_none());
    }

    #[test]
    fn files_and_conf() {
        let files = "%FILES%\nusr/\nusr/bin/\nusr/bin/rg\nusr/share/doc/rg/README\n\n%BACKUP%\n";
        assert_eq!(executables_from_files(files), vec!["/usr/bin/rg"]);
        let conf = "[options]\n# DBPath = /ignored\nDBPath = /srv/pacman/ # custom\n";
        assert_eq!(db_path_from_conf(conf).as_deref(), Some("/srv/pacman/"));
        assert_eq!(db_path_from_conf("[options]\n"), None);
    }
}
