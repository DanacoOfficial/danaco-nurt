// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-programs::sources
// Responsibility:   Built-in providers of program information and helpers shared by them.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Built-in providers of program information and helpers shared by them.

pub mod brew;
pub mod catalog;
pub mod choco;
pub mod desktop;
pub mod dpkg;
pub mod macapp;
pub mod man;
pub mod pacman;
pub mod path;
pub mod rpm;
pub mod scoop;
pub mod winapp;
pub mod winget;

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use nurt_contracts::config::{ProgramSource as ConfigSource, ProgramsConfig};

use crate::source::{
    CatalogSpec, HostEnv, ProgramSource, SourceError, SourceErrorKind, SourceId, SourceRegistry,
};

/// Directories whose executables are mapped to packages. Files in other
/// directories (libraries, `libexec`) do not create cards.
pub const BIN_DIRS: &[&str] = &[
    "/usr/bin",
    "/bin",
    "/usr/sbin",
    "/sbin",
    "/usr/games",
    "/usr/local/bin",
    "/usr/local/sbin",
];

/// Largest number of symbolic links followed when resolving one path.
pub const MAX_LINK_HOPS: usize = 16;

/// Largest metadata file read whole by a provider.
pub const MAX_METADATA_BYTES: u64 = 64 * 1024 * 1024;

/// Built-in providers enabled by `config`, in priority order of system
/// data: search path, package databases, applications, manual pages, then
/// the external catalogs of `programs.catalogs` in configuration order.
///
/// Every provider is registered on every system; [`ProgramSource::detect`]
/// decides whether it applies to the environment. Catalog ids are taken
/// from the `id` of each `catalog.json` when it is valid and unique.
#[must_use]
pub fn builtin(env: &HostEnv, config: &ProgramsConfig) -> SourceRegistry {
    let disabled = |kind: ConfigSource| config.sources_disabled.contains(&kind);
    let mut registry = SourceRegistry::new();
    let mut add = |kind: ConfigSource, source: Arc<dyn ProgramSource>| {
        if !disabled(kind) {
            registry.add(source);
        }
    };
    add(ConfigSource::Path, Arc::new(path::PathSource::new()));
    add(ConfigSource::Package, Arc::new(dpkg::DpkgSource::new()));
    add(ConfigSource::Package, Arc::new(rpm::RpmSource::new()));
    add(ConfigSource::Package, Arc::new(pacman::PacmanSource::new()));
    add(ConfigSource::Package, Arc::new(brew::BrewSource::new()));
    add(ConfigSource::Package, Arc::new(winget::WingetSource::new()));
    add(ConfigSource::Package, Arc::new(scoop::ScoopSource::new()));
    add(ConfigSource::Package, Arc::new(choco::ChocoSource::new()));
    add(ConfigSource::App, Arc::new(desktop::DesktopSource::new()));
    add(ConfigSource::App, Arc::new(winapp::WinAppSource::new()));
    add(ConfigSource::App, Arc::new(macapp::MacAppSource::new()));
    add(ConfigSource::Man, Arc::new(man::ManSource::new()));
    if !disabled(ConfigSource::Catalog) {
        for spec in catalog::resolve_ids(CatalogSpec::from_config(config), env) {
            registry.add(Arc::new(catalog::CatalogProvider::new(spec)));
        }
    }
    registry
}

/// Reads a whole file given as a host path.
///
/// # Errors
///
/// Fails when the file cannot be read or is larger than
/// [`MAX_METADATA_BYTES`].
pub fn read_file(env: &HostEnv, id: &SourceId, host_path: &Path) -> Result<Vec<u8>, SourceError> {
    read_prefix(env, id, host_path, MAX_METADATA_BYTES).and_then(|(bytes, complete)| {
        if complete {
            Ok(bytes)
        } else {
            Err(
                SourceError::new(id, SourceErrorKind::Corrupt, "file too large")
                    .at(host_path.display().to_string()),
            )
        }
    })
}

/// Reads at most `limit` bytes of a file given as a host path; the flag is
/// `true` when the whole file was read.
///
/// # Errors
///
/// Fails when the file cannot be opened or read.
pub fn read_prefix(
    env: &HostEnv,
    id: &SourceId,
    host_path: &Path,
    limit: u64,
) -> Result<(Vec<u8>, bool), SourceError> {
    let file = env.sys_path(host_path);
    let handle =
        std::fs::File::open(&file).map_err(|error| SourceError::io(id, host_path, &error))?;
    let mut bytes = Vec::new();
    handle
        .take(limit.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|error| SourceError::io(id, host_path, &error))?;
    let complete = bytes.len() as u64 <= limit;
    bytes.truncate(usize::try_from(limit).unwrap_or(usize::MAX));
    Ok((bytes, complete))
}

/// Text of a small file given as a system path; bytes that are not UTF-8 are
/// replaced, so that one damaged byte does not hide the whole file. `None`
/// when the file cannot be read.
#[must_use]
pub fn read_lossy(system_path: &Path) -> Option<String> {
    let bytes = std::fs::read(system_path).ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

/// Entries of a directory given as a host path, sorted by name: file name
/// and host path. A missing or unreadable directory has no entries.
#[must_use]
pub fn list_dir(env: &HostEnv, host_dir: &Path) -> Vec<(String, PathBuf)> {
    let Ok(entries) = std::fs::read_dir(env.sys_path(host_dir)) else {
        return Vec::new();
    };
    let mut out: Vec<(String, PathBuf)> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            let host = host_dir.join(&name);
            Some((name, host))
        })
        .collect();
    out.sort();
    out
}

/// Returns `true` when the host path names an existing file or directory.
#[must_use]
pub fn exists(env: &HostEnv, host_path: &Path) -> bool {
    env.sys_path(host_path).exists()
}

/// Returns `true` when the host path names a directory.
#[must_use]
pub fn is_dir(env: &HostEnv, host_path: &Path) -> bool {
    env.sys_path(host_path).is_dir()
}

/// Final host path of a chain of symbolic links, resolving absolute link
/// targets through the environment; `None` for a loop or a broken link.
#[must_use]
pub fn resolve_links(env: &HostEnv, host_path: &Path) -> Option<PathBuf> {
    let mut current = normalize(host_path);
    for _ in 0..MAX_LINK_HOPS {
        let file = env.sys_path(&current);
        let meta = std::fs::symlink_metadata(&file).ok()?;
        if !meta.file_type().is_symlink() {
            return Some(current);
        }
        let target = std::fs::read_link(&file).ok()?;
        current = if target.has_root() || env.is_absolute(&target) {
            normalize(&target)
        } else {
            normalize(&current.parent().unwrap_or(Path::new("/")).join(target))
        };
    }
    None
}

/// Host path whose directory part has its symbolic links resolved, so that
/// `/bin/ls` and `/usr/bin/ls` are equal on systems with a merged `/usr`.
#[must_use]
pub fn canonical_dir_path(env: &HostEnv, host_path: &Path) -> PathBuf {
    let Some(name) = host_path.file_name() else {
        return host_path.to_path_buf();
    };
    let dir = host_path.parent().unwrap_or(Path::new("/"));
    let mut resolved = PathBuf::new();
    for component in dir.components() {
        resolved.push(component);
        if let Some(target) = resolve_links(env, &resolved) {
            resolved = target;
        }
    }
    resolved.join(name)
}

/// Removes `.` and resolves `..` lexically.
#[must_use]
pub fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

/// Returns `true` when the host path lies directly in one of [`BIN_DIRS`].
#[must_use]
pub fn in_bin_dir(host_path: &str) -> bool {
    let Some((dir, name)) = host_path.rsplit_once('/') else {
        return false;
    };
    !name.is_empty() && BIN_DIRS.contains(&dir)
}

/// Seconds since the Unix epoch as RFC 3339 text in UTC.
///
/// # Examples
///
/// ```
/// use nurt_programs::sources::rfc3339;
///
/// assert_eq!(rfc3339(0), "1970-01-01T00:00:00Z");
/// assert_eq!(rfc3339(1_767_225_600), "2026-01-01T00:00:00Z");
/// ```
#[must_use]
pub fn rfc3339(seconds: i64) -> String {
    const DAY: i64 = 86_400;
    let days = seconds.div_euclid(DAY);
    let rest = seconds.rem_euclid(DAY);
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rest / 3600,
        rest % 3600 / 60,
        rest % 60
    )
}

/// Proleptic Gregorian date of a day number counted from 1970-01-01
/// (algorithm of H. Hinnant, `civil_from_days`).
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    const ERA_DAYS: i64 = 146_097;
    let z = days + 719_468;
    let era = z.div_euclid(ERA_DAYS);
    let doe = z.rem_euclid(ERA_DAYS);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// Host path as text with `/` separators on Unix and the native form on
/// Windows.
#[must_use]
pub fn path_text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// Compression of a file, from its name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compression {
    /// Not compressed.
    None,
    /// gzip.
    Gzip,
    /// xz.
    Xz,
    /// bzip2.
    Bzip2,
    /// Zstandard.
    Zstd,
    /// LZ4 frame.
    Lz4,
}

impl Compression {
    /// Compression of a file named `name` and the name without the
    /// compression extension.
    #[must_use]
    pub fn of(name: &str) -> (Self, &str) {
        const SUFFIXES: &[(&str, Compression)] = &[
            (".gz", Compression::Gzip),
            (".xz", Compression::Xz),
            (".bz2", Compression::Bzip2),
            (".zst", Compression::Zstd),
            (".lz4", Compression::Lz4),
        ];
        for (suffix, compression) in SUFFIXES {
            if let Some(stem) = name.strip_suffix(suffix) {
                return (*compression, stem);
            }
        }
        (Self::None, name)
    }
}

/// Decompresses at most `limit` bytes of `bytes`.
///
/// # Errors
///
/// Fails with the error of the decoder when the data is damaged before
/// `limit` bytes were produced.
pub fn decompress(bytes: &[u8], compression: Compression, limit: u64) -> std::io::Result<Vec<u8>> {
    decompress_reader(bytes, compression, limit)
}

/// Decompresses at most `limit` bytes of the stream `source`, reading only
/// as much of it as the decoder needs for them.
///
/// # Errors
///
/// Fails with the error of the decoder or of `source` when the data is
/// damaged before `limit` bytes were produced.
pub fn decompress_reader<'a>(
    source: impl Read + 'a,
    compression: Compression,
    limit: u64,
) -> std::io::Result<Vec<u8>> {
    let reader: Box<dyn Read + 'a> = match compression {
        Compression::None => Box::new(source),
        Compression::Gzip => Box::new(flate2::read::MultiGzDecoder::new(source)),
        Compression::Xz => Box::new(liblzma::read::XzDecoder::new(source)),
        Compression::Bzip2 => Box::new(bzip2::read::BzDecoder::new(source)),
        Compression::Zstd => Box::new(zstd::stream::read::Decoder::new(source)?),
        Compression::Lz4 => Box::new(lz4_flex::frame::FrameDecoder::new(source)),
    };
    let mut out = Vec::new();
    reader.take(limit).read_to_end(&mut out)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use nurt_contracts::env::Os;

    #[test]
    fn compression_from_name() {
        assert_eq!(Compression::of("ls.1.gz"), (Compression::Gzip, "ls.1"));
        assert_eq!(Compression::of("ls.1.zst"), (Compression::Zstd, "ls.1"));
        assert_eq!(Compression::of("ls.1"), (Compression::None, "ls.1"));
    }

    #[test]
    fn bin_dir_membership() {
        assert!(in_bin_dir("/usr/bin/ls"));
        assert!(!in_bin_dir("/usr/lib/x/ls"));
        assert!(!in_bin_dir("/usr/bin/"));
    }

    #[test]
    fn normalize_resolves_parent_components() {
        assert_eq!(
            normalize(Path::new("/usr/bin/../lib/./x")),
            PathBuf::from("/usr/lib/x")
        );
    }

    #[cfg(unix)]
    #[test]
    fn resolve_links_follows_absolute_targets_below_root() {
        let dir = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
        let env = HostEnv::new(Os::Linux).with_root(dir.path());
        std::fs::create_dir_all(dir.path().join("usr/bin")).unwrap_or_default();
        std::fs::write(dir.path().join("usr/bin/vim"), b"").unwrap_or_default();
        std::os::unix::fs::symlink("/usr/bin", dir.path().join("bin")).unwrap_or_default();
        std::os::unix::fs::symlink("/usr/bin/vim", dir.path().join("usr/bin/vi"))
            .unwrap_or_default();
        assert_eq!(
            resolve_links(&env, Path::new("/usr/bin/vi")),
            Some(PathBuf::from("/usr/bin/vim"))
        );
        assert_eq!(
            canonical_dir_path(&env, Path::new("/bin/vi")),
            PathBuf::from("/usr/bin/vi")
        );
        std::os::unix::fs::symlink("loop", dir.path().join("loop")).unwrap_or_default();
        assert_eq!(resolve_links(&env, Path::new("/loop")), None);
    }

    /// Reader that counts the bytes taken from it.
    struct Counting<'a> {
        data: &'a [u8],
        taken: usize,
    }

    impl Read for Counting<'_> {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            let count = self.data.len().min(buffer.len());
            buffer[..count].copy_from_slice(&self.data[..count]);
            self.data = &self.data[count..];
            self.taken += count;
            Ok(count)
        }
    }

    #[test]
    fn decompress_reader_stops_reading_at_the_limit() {
        use std::io::Write;
        let mut state = 0x9e37_79b9_u32;
        let noise: Vec<u8> = (0..2 * 1024 * 1024)
            .map(|_| {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (state >> 24) as u8
            })
            .collect();
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        encoder.write_all(&noise).unwrap_or_default();
        let packed = encoder.finish().unwrap_or_default();
        assert!(packed.len() > 1024 * 1024);
        let mut source = Counting {
            data: &packed,
            taken: 0,
        };
        let head = decompress_reader(&mut source, Compression::Gzip, 1024).unwrap_or_default();
        assert_eq!(head, &noise[..1024]);
        assert!(source.taken < 256 * 1024, "{} bytes read", source.taken);
    }

    #[test]
    fn decompress_round_trip_and_limit() {
        use std::io::Write;
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(b"hello world").unwrap_or_default();
        let packed = encoder.finish().unwrap_or_default();
        assert_eq!(
            decompress(&packed, Compression::Gzip, 5).unwrap_or_default(),
            b"hello"
        );
        assert_eq!(
            decompress(b"abc", Compression::None, 10).unwrap_or_default(),
            b"abc"
        );
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
