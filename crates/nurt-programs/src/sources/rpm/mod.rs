// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-programs::sources::rpm
// Responsibility:   RPM package databases.
// Dependencies:     header, rusqlite
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! RPM package databases.
//!
//! Three storage formats share one header format ([`header`]): SQLite
//! (`rpmdb.sqlite`, rpm 4.16 and later), NDB (`Packages.db`, [`ndb`]) and
//! Berkeley DB (`Packages`). SQLite and NDB are read; a database present
//! only in the Berkeley DB format is reported as unavailable. SQLite files
//! are opened read-only; when the account cannot create the shared memory
//! file of a database in write-ahead mode, the file is opened as immutable.

pub mod header;
pub mod ndb;

use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags};

use crate::model::{
    CategoryHint, CategoryScheme, EntryKey, PackageRef, RawPackage, RawProgram, SourceData,
};
use crate::source::{
    Deadline, Fingerprint, FingerprintBuilder, HostEnv, ProgramSource, SourceError,
    SourceErrorKind, SourceId, SourceKind, SourceWarning, WarningCode,
};
use crate::sources::{exists, in_bin_dir, read_file, rfc3339};

use header::{
    Header, TAG_ARCH, TAG_DESCRIPTION, TAG_GROUP, TAG_INSTALL_TIME, TAG_LICENSE, TAG_LONG_SIZE,
    TAG_NAME, TAG_SIZE, TAG_SOURCE_RPM, TAG_SUMMARY, TAG_URL,
};

/// Database directories, newest layout first.
pub const DB_DIRS: &[&str] = &["/usr/lib/sysimage/rpm", "/var/lib/rpm"];
/// File name of the SQLite database.
pub const SQLITE_FILE: &str = "rpmdb.sqlite";
/// File name of the NDB package file.
pub const NDB_FILE: &str = "Packages.db";
/// File name of the Berkeley DB package file.
pub const BDB_FILE: &str = "Packages";
const WAL_SUFFIX: &str = "-wal";
const PACKAGES_QUERY: &str = "SELECT hnum, blob FROM Packages";

/// Storage format of a database.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DbFormat {
    /// SQLite.
    Sqlite,
    /// NDB.
    Ndb,
    /// Berkeley DB.
    BerkeleyDb,
}

/// Database file found on the host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DbFile {
    /// Format.
    pub format: DbFormat,
    /// Host path.
    pub path: PathBuf,
}

/// First database file present, in the order SQLite, NDB, Berkeley DB of
/// each directory of [`DB_DIRS`].
#[must_use]
pub fn locate(env: &HostEnv) -> Option<DbFile> {
    for dir in DB_DIRS {
        let dir = Path::new(dir);
        for (format, name) in [
            (DbFormat::Sqlite, SQLITE_FILE),
            (DbFormat::Ndb, NDB_FILE),
            (DbFormat::BerkeleyDb, BDB_FILE),
        ] {
            let path = dir.join(name);
            if exists(env, &path) {
                return Some(DbFile { format, path });
            }
        }
    }
    None
}

/// Provider of the RPM package database.
#[derive(Debug, Clone)]
pub struct RpmSource {
    id: SourceId,
}

impl RpmSource {
    /// Creates the provider.
    #[must_use]
    pub fn new() -> Self {
        Self {
            id: SourceId::new(SourceId::RPM),
        }
    }
}

impl Default for RpmSource {
    fn default() -> Self {
        Self::new()
    }
}

impl ProgramSource for RpmSource {
    fn id(&self) -> &SourceId {
        &self.id
    }

    fn kind(&self) -> SourceKind {
        SourceKind::Package
    }

    fn detect(&self, env: &HostEnv) -> bool {
        locate(env).is_some()
    }

    fn fingerprint(&self, env: &HostEnv) -> Result<Fingerprint, SourceError> {
        let mut builder = FingerprintBuilder::new(env);
        if let Some(file) = locate(env) {
            let wal = PathBuf::from(format!("{}{WAL_SUFFIX}", file.path.display()));
            builder = builder.path(&file.path).path(wal);
        }
        Ok(builder.finish())
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
/// Fails when no database is present, the database cannot be opened, or it
/// uses the Berkeley DB format.
pub fn read_packages(env: &HostEnv) -> Result<SourceData, SourceError> {
    read_with_deadline(env, &Deadline::none(), &SourceId::new(SourceId::RPM))
}

fn read_with_deadline(
    env: &HostEnv,
    deadline: &Deadline<'_>,
    id: &SourceId,
) -> Result<SourceData, SourceError> {
    let file = locate(env)
        .ok_or_else(|| SourceError::new(id, SourceErrorKind::Unavailable, "no rpm database"))?;
    let location = file.path.display().to_string();
    let blobs = match file.format {
        DbFormat::Sqlite => read_sqlite(env, id, &file.path)?,
        DbFormat::Ndb => {
            let bytes = read_file(env, id, &file.path)?;
            let content = ndb::read(&bytes).map_err(|reason| {
                SourceError::new(id, SourceErrorKind::Corrupt, reason).at(location.clone())
            })?;
            content.blobs
        }
        DbFormat::BerkeleyDb => {
            return Err(SourceError::new(
                id,
                SourceErrorKind::Unavailable,
                "berkeley db package databases are not supported",
            )
            .at(location));
        }
    };
    let mut data = SourceData::default();
    for (number, blob) in blobs.iter().enumerate() {
        deadline.check(id)?;
        match Header::parse(blob) {
            Ok(header) => add_package(&mut data, &header, id),
            Err(error) => data.warnings.push(
                SourceWarning::new(
                    WarningCode::SourceUnavailable,
                    id,
                    format!("damaged header: {error}"),
                )
                .at(location.clone())
                .line(number as u64 + 1),
            ),
        }
    }
    Ok(data)
}

/// Stored headers of a SQLite database.
fn read_sqlite(
    env: &HostEnv,
    id: &SourceId,
    host_path: &Path,
) -> Result<Vec<Vec<u8>>, SourceError> {
    let file = env.sys_path(host_path);
    let fail = |error: rusqlite::Error| {
        SourceError::new(id, SourceErrorKind::Unavailable, error.to_string())
            .at(host_path.display().to_string())
    };
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let first = Connection::open_with_flags(&file, flags).and_then(|connection| query(&connection));
    match first {
        Ok(blobs) => Ok(blobs),
        Err(_) => {
            let uri = format!("file:{}?immutable=1", uri_path(&file));
            let connection = Connection::open_with_flags(uri, flags | OpenFlags::SQLITE_OPEN_URI)
                .map_err(fail)?;
            query(&connection).map_err(fail)
        }
    }
}

fn query(connection: &Connection) -> rusqlite::Result<Vec<Vec<u8>>> {
    let mut statement = connection.prepare(PACKAGES_QUERY)?;
    let rows = statement.query_map([], |row| row.get::<_, Vec<u8>>(1))?;
    rows.collect()
}

/// Path written for a SQLite URI: `%`, `?` and `#` escaped.
fn uri_path(path: &Path) -> String {
    let mut out = String::new();
    for c in path.to_string_lossy().chars() {
        match c {
            '%' => out.push_str("%25"),
            '?' => out.push_str("%3f"),
            '#' => out.push_str("%23"),
            '\\' => out.push('/'),
            other => out.push(other),
        }
    }
    out
}

/// Package of a parsed header; `None` without a name.
#[must_use]
pub fn package_from_header(header: &Header) -> Option<RawPackage> {
    let name = header.string(TAG_NAME)?;
    let size = header
        .integer(TAG_LONG_SIZE)
        .or_else(|| header.integer(TAG_SIZE));
    Some(RawPackage {
        name,
        version: header.full_version(),
        architecture: header.string(TAG_ARCH),
        summary: header.localized(TAG_SUMMARY),
        description: header.localized(TAG_DESCRIPTION),
        section: header
            .string(TAG_GROUP)
            .filter(|group| group != "Unspecified"),
        homepage: header.string(TAG_URL),
        license: header.string(TAG_LICENSE),
        origin: header.string(TAG_SOURCE_RPM),
        installed_size: size,
        installed_at: header
            .integer(TAG_INSTALL_TIME)
            .and_then(|seconds| i64::try_from(seconds).ok())
            .map(rfc3339),
        executables: header
            .file_paths()
            .into_iter()
            .filter(|path| in_bin_dir(path))
            .collect(),
        ..RawPackage::default()
    })
}

fn add_package(data: &mut SourceData, header: &Header, id: &SourceId) {
    let Some(package) = package_from_header(header) else {
        data.warnings.push(SourceWarning::new(
            WarningCode::SourceUnavailable,
            id,
            "header without name",
        ));
        return;
    };
    let reference = PackageRef {
        manager: id.clone(),
        name: package.name.clone(),
    };
    for file in &package.executables {
        data.file_packages.push((file.clone(), reference.clone()));
        let name = file.rsplit('/').next().unwrap_or(file).to_owned();
        let mut program = RawProgram::new(name);
        program.key = Some(EntryKey::new(id, package.name.clone()));
        program.path = Some(file.clone());
        program.package = Some(reference.clone());
        program.version.clone_from(&package.version);
        program.homepage.clone_from(&package.homepage);
        program.license.clone_from(&package.license);
        if let Some(group) = &package.section {
            program
                .category_hints
                .push(CategoryHint::new(CategoryScheme::RpmGroup, group.clone()));
        }
        data.programs.push(program);
    }
    data.packages.push(package);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uri_path_escapes_reserved_characters() {
        assert_eq!(uri_path(Path::new("/a b/c?d#e%f")), "/a b/c%3fd%23e%25f");
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
