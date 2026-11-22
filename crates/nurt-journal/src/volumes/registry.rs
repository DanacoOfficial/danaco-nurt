// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Registry of journal directories of the account (`volumes.json` in the
//! state directory).
//!
//! The registry maps each mounted volume to the journal directory chosen for
//! it, so that later operations find the journal without selecting the anchor
//! again. It is a cache: a missing or unreadable registry is rebuilt by
//! resolution. Writers serialise on `volumes.json.lock` and replace the file
//! atomically (temporary file, flush, rename, directory flush); readers need
//! no lock.

use std::io::Read;
use std::path::PathBuf;
use std::time::Duration;

use nurt_contracts::config::StateConfig;
use nurt_contracts::env::{Host, Os};
use nurt_contracts::fs::{
    AbsPath, FileKind, Follow, Fs, FsError, FsErrorKind, FsResult, FsWrite, RenameMode, TxScope,
    VolumeId, VolumeInfo,
};
use serde::{Deserialize, Serialize};

use super::anchor::{discard, io_failure, write_temp};
use super::maintenance_scope;
use crate::layout::{LOCK_SUFFIX, StateLayout, VOLUMES_FILE};

/// Version of the registry format.
const FORMAT_VERSION: u32 = 1;

/// Longest wait for the registry lock held by another process.
const LOCK_TIMEOUT: Duration = Duration::from_secs(2);

/// Permission bits of state directories created for the registry.
const STATE_DIR_MODE: u32 = 0o700;

/// Journal directory recorded for one mounted volume.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VolumeRecord {
    /// Volume identity at the time of recording.
    pub id: VolumeId,
    /// Mount point of the volume.
    pub mount: AbsPath,
    /// File system type name.
    pub fs_type: String,
    /// Journal directory of the volume.
    pub journal: AbsPath,
}

#[derive(Serialize, Deserialize)]
struct RegistryFile {
    version: u32,
    volumes: Vec<VolumeRecord>,
}

/// Difference between the registry and the mounted volume, found while
/// resolving a journal directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistryChange {
    /// The volume had no journal directory; one was created.
    Created,
    /// Another volume is mounted where the recorded one was; its journal
    /// entries are not available on the current volume.
    VolumeReplaced {
        /// Identity of the volume recorded at this mount point.
        previous: VolumeId,
    },
    /// The recorded journal directory no longer exists; it was created again
    /// and the entries it held are lost.
    JournalMissing {
        /// Journal directory that disappeared.
        previous: AbsPath,
    },
    /// The volume is mounted at another place than recorded.
    Remounted {
        /// Mount point recorded earlier.
        previous: AbsPath,
    },
}

/// Result of looking a mounted volume up in the registry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Lookup {
    /// The recorded journal directory exists.
    Current(AbsPath),
    /// The volume was recorded at another mount point and its journal
    /// directory exists at the corresponding place under the new mount.
    Relocated {
        /// Journal directory under the current mount point.
        dir: AbsPath,
        /// Mount point recorded earlier.
        previous: AbsPath,
    },
    /// The record no longer matches; a journal directory must be selected.
    Changed(RegistryChange),
    /// The volume is not recorded.
    Unknown,
}

/// Registry of journal directories in the state directory of the account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Registry {
    dir: AbsPath,
}

impl Registry {
    /// Registry kept in `state_dir`; nothing is read or created until used.
    #[must_use]
    pub fn new(state_dir: AbsPath) -> Self {
        Self { dir: state_dir }
    }

    fn file(&self) -> FsResult<AbsPath> {
        StateLayout::new(self.dir.clone()).volumes()
    }

    /// Recorded journal directories, in mount point order.
    ///
    /// The list is empty when the registry is missing, cannot be parsed or has
    /// another format version.
    ///
    /// # Errors
    ///
    /// Fails with the error of opening or reading the registry file, other
    /// than [`FsErrorKind::NotFound`].
    pub fn load(&self, fs: &dyn Fs) -> FsResult<Vec<VolumeRecord>> {
        let file = self.file()?;
        let mut handle = match fs.open_read(&file) {
            Ok(handle) => handle,
            Err(error) if error.kind == FsErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error),
        };
        let mut bytes = Vec::new();
        handle
            .read_to_end(&mut bytes)
            .map_err(|error| io_failure("read", &file, &error))?;
        Ok(serde_json::from_slice::<RegistryFile>(&bytes)
            .ok()
            .filter(|registry| registry.version == FORMAT_VERSION)
            .map(|registry| registry.volumes)
            .unwrap_or_default())
    }

    /// Looks up the journal directory of a mounted volume.
    pub(crate) fn lookup(&self, fs: &dyn Fs, volume: &VolumeInfo) -> FsResult<Lookup> {
        let records = self.load(fs)?;
        if let Some(record) = records.iter().find(|record| record.mount == volume.mount) {
            if !same_identity(&record.id, &volume.id) {
                return Ok(Lookup::Changed(RegistryChange::VolumeReplaced {
                    previous: record.id.clone(),
                }));
            }
            return Ok(if is_dir(fs, &record.journal)? {
                Lookup::Current(record.journal.clone())
            } else {
                Lookup::Changed(RegistryChange::JournalMissing {
                    previous: record.journal.clone(),
                })
            });
        }
        let moved = records
            .iter()
            .find(|record| record.id.uuid.is_some() && same_identity(&record.id, &volume.id));
        let Some(record) = moved else {
            return Ok(Lookup::Unknown);
        };
        if let Some(dir) = relocate(&record.journal, &record.mount, &volume.mount)
            && is_dir(fs, &dir)?
        {
            return Ok(Lookup::Relocated {
                dir,
                previous: record.mount.clone(),
            });
        }
        Ok(Lookup::Changed(RegistryChange::Remounted {
            previous: record.mount.clone(),
        }))
    }

    /// Records the journal directory of a mount point, replacing an earlier
    /// record of the same mount point.
    ///
    /// Records of other mount points are kept, also when they name the same
    /// volume: a volume may be mounted at several places at once (bind
    /// mounts), and renames do not cross mounts.
    pub(crate) fn store(&self, fs: &dyn FsWrite, record: VolumeRecord) -> FsResult<()> {
        let scope = maintenance_scope();
        let file = self.file()?;
        ensure_dir_all(fs, &scope, &self.dir)?;
        let lock = self.dir.join_name(&format!("{VOLUMES_FILE}{LOCK_SUFFIX}"))?;
        let _lock = fs.lock_exclusive(&lock, LOCK_TIMEOUT)?;
        let mut volumes = self.load(fs)?;
        volumes.retain(|existing| existing.mount != record.mount);
        volumes.push(record);
        volumes.sort_by(|left, right| left.mount.cmp(&right.mount));
        let registry = RegistryFile {
            version: FORMAT_VERSION,
            volumes,
        };
        let bytes = serde_json::to_vec_pretty(&registry)
            .map_err(|_| FsError::new(FsErrorKind::Io, "encode", Some(file.as_path())))?;
        let temp = write_temp(fs, &scope, &self.dir, VOLUMES_FILE, &bytes)?;
        if let Err(error) = fs.rename(&scope, &temp, &file, RenameMode::Replace) {
            discard(fs, &scope, &temp);
            return Err(error);
        }
        fs.sync_dir(&scope, &self.dir)
    }
}

/// Whether two identities denote the same volume: by UUID and subvolume when
/// both carry a UUID (device numbers may change between boots), otherwise by
/// device number and subvolume.
fn same_identity(left: &VolumeId, right: &VolumeId) -> bool {
    let same_device = match (&left.uuid, &right.uuid) {
        (Some(left_uuid), Some(right_uuid)) => left_uuid == right_uuid,
        _ => left.dev == right.dev,
    };
    same_device && left.subvolume == right.subvolume
}

fn relocate(journal: &AbsPath, from: &AbsPath, to: &AbsPath) -> Option<AbsPath> {
    let relative = journal.as_path().strip_prefix(from.as_path()).ok()?;
    AbsPath::new(to.as_path().join(relative)).ok()
}

fn is_dir(fs: &dyn Fs, path: &AbsPath) -> FsResult<bool> {
    match fs.metadata(path, Follow::NoFollow) {
        Ok(meta) => Ok(meta.kind == FileKind::Directory),
        Err(error) if matches!(error.kind, FsErrorKind::NotFound | FsErrorKind::NotADirectory) => {
            Ok(false)
        }
        Err(error) => Err(error),
    }
}

/// Creates `dir` and its missing ancestors.
fn ensure_dir_all(fs: &dyn FsWrite, scope: &TxScope, dir: &AbsPath) -> FsResult<()> {
    let mut missing = Vec::new();
    let mut current = Some(dir.clone());
    while let Some(path) = current {
        match fs.metadata(&path, Follow::Follow) {
            Ok(meta) if meta.kind == FileKind::Directory => break,
            Ok(_) => {
                return Err(FsError::new(
                    FsErrorKind::NotADirectory,
                    "create_dir",
                    Some(path.as_path()),
                ));
            }
            Err(error) if error.kind == FsErrorKind::NotFound => {
                current = path.parent();
                missing.push(path);
            }
            Err(error) => return Err(error),
        }
    }
    for path in missing.iter().rev() {
        if let Err(error) = fs.create_dir(scope, path, STATE_DIR_MODE)
            && error.kind != FsErrorKind::Exists
        {
            return Err(error);
        }
    }
    Ok(())
}

/// State directory of the account.
///
/// The setting `state.dir` decides when set. Otherwise the directory is
/// `$XDG_STATE_HOME/nurt` on Linux and other Unix systems (default
/// `$HOME/.local/state/nurt`), `$HOME/Library/Application Support/Nurt` on
/// macOS and `%LOCALAPPDATA%\Nurt` on Windows (default
/// `%USERPROFILE%\AppData\Local\Nurt`). Relative values of the variables are
/// ignored, as the XDG base directory specification requires.
///
/// # Errors
///
/// Fails with [`FsErrorKind::InvalidPath`] when the resulting directory is
/// not absolute, for example when `state.dir` is a relative path.
pub fn state_dir(host: &dyn Host, config: &StateConfig) -> FsResult<AbsPath> {
    if let Some(dir) = &config.dir {
        return AbsPath::new(PathBuf::from(dir));
    }
    let info = host.info();
    let home = &info.home;
    let dir = match info.os {
        Os::Macos => home.join("Library").join("Application Support").join("Nurt"),
        Os::Windows => absolute_var(host, "LOCALAPPDATA")
            .unwrap_or_else(|| home.join("AppData").join("Local"))
            .join("Nurt"),
        _ => absolute_var(host, "XDG_STATE_HOME")
            .unwrap_or_else(|| home.join(".local").join("state"))
            .join("nurt"),
    };
    AbsPath::new(dir)
}

fn absolute_var(host: &dyn Host, name: &str) -> Option<PathBuf> {
    host.env_var(name)
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::ffi::OsString;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::thread;

    use nurt_contracts::config::StateConfig;
    use nurt_contracts::env::{Host, HostInfo, Os, PathStyle};
    use nurt_contracts::fs::{FsErrorKind, SpaceInfo, VolumeKind};

    use super::{Lookup, Registry, RegistryChange, VolumeRecord, state_dir};
    use crate::volumes::testfs::{TestFs, abs, volume_info};

    const SPACE: SpaceInfo = SpaceInfo {
        total: 1 << 30,
        available: 1 << 29,
        quota_remaining: None,
        purgeable_system: None,
    };

    fn record(dev: u64, mount: &str, journal: &str) -> VolumeRecord {
        VolumeRecord {
            id: volume_info(dev, mount, "ext4", VolumeKind::Fixed).id,
            mount: abs(mount),
            fs_type: "ext4".to_owned(),
            journal: abs(journal),
        }
    }

    /// State directory `/home/user/.local/state/nurt` (not yet created) on the
    /// volume `/home`, and the volume `/mnt/data` with a journal directory.
    fn setup() -> (TestFs, Registry) {
        let fs = TestFs::new();
        fs.add_volume(
            "/home",
            volume_info(1, "/home", "ext4", VolumeKind::Fixed),
            SPACE,
        );
        fs.add_volume(
            "/mnt/data",
            volume_info(2, "/mnt/data", "ext4", VolumeKind::Fixed),
            SPACE,
        );
        fs.mkdir_p("/home/user");
        fs.mkdir_p("/mnt/data/.nurt-journal");
        let registry = Registry::new(abs("/home/user/.local/state/nurt"));
        (fs, registry)
    }

    #[test]
    fn stores_records_atomically_and_reads_them_back() {
        let (fs, registry) = setup();
        assert!(registry.load(&fs).unwrap().is_empty());
        registry
            .store(&fs, record(2, "/mnt/data", "/mnt/data/.nurt-journal"))
            .unwrap();
        registry
            .store(&fs, record(1, "/home", "/home/user/.nurt-journal"))
            .unwrap();
        let records = registry.load(&fs).unwrap();
        assert_eq!(
            records,
            [
                record(1, "/home", "/home/user/.nurt-journal"),
                record(2, "/mnt/data", "/mnt/data/.nurt-journal"),
            ]
        );
        assert_eq!(fs.mode("/home/user/.local/state"), Some(0o700));
        let calls = fs.calls();
        let write = calls
            .iter()
            .position(|call| call.starts_with("create_temp_in /home/user/.local/state/nurt/"))
            .unwrap();
        let rename = calls
            .iter()
            .position(|call| call == "rename /home/user/.local/state/nurt/volumes.json")
            .unwrap();
        assert!(write < rename);
        let mut children = fs.children("/home/user/.local/state/nurt");
        children.sort();
        assert_eq!(children, ["volumes.json", "volumes.json.lock"]);
    }

    #[test]
    fn keeps_previous_registry_when_writing_fails() {
        let (fs, registry) = setup();
        registry
            .store(&fs, record(2, "/mnt/data", "/mnt/data/.nurt-journal"))
            .unwrap();
        let before = fs.read("/home/user/.local/state/nurt/volumes.json");
        fs.fail("rename", "/home/user/.local/state/nurt", FsErrorKind::Io);
        let error = registry
            .store(&fs, record(1, "/home", "/home/user/.nurt-journal"))
            .unwrap_err();
        assert_eq!(error.kind, FsErrorKind::Io);
        assert_eq!(fs.read("/home/user/.local/state/nurt/volumes.json"), before);
        let mut children = fs.children("/home/user/.local/state/nurt");
        children.sort();
        assert_eq!(children, ["volumes.json", "volumes.json.lock"]);
    }

    #[test]
    fn treats_unreadable_registry_as_empty() {
        let (fs, registry) = setup();
        fs.mkdir_p("/home/user/.local/state/nurt");
        fs.write_file("/home/user/.local/state/nurt/volumes.json", b"{ not json");
        assert!(registry.load(&fs).unwrap().is_empty());
        fs.write_file(
            "/home/user/.local/state/nurt/volumes.json",
            br#"{"version": 99, "volumes": []}"#,
        );
        assert!(registry.load(&fs).unwrap().is_empty());
        fs.fail("open_read", "/home/user/.local/state/nurt", FsErrorKind::Io);
        assert_eq!(registry.load(&fs).unwrap_err().kind, FsErrorKind::Io);
    }

    #[test]
    fn finds_current_journal_directory() {
        let (fs, registry) = setup();
        registry
            .store(&fs, record(2, "/mnt/data", "/mnt/data/.nurt-journal"))
            .unwrap();
        let volume = volume_info(2, "/mnt/data", "ext4", VolumeKind::Fixed);
        assert_eq!(
            registry.lookup(&fs, &volume).unwrap(),
            Lookup::Current(abs("/mnt/data/.nurt-journal"))
        );
        let other = volume_info(1, "/home", "ext4", VolumeKind::Fixed);
        assert_eq!(registry.lookup(&fs, &other).unwrap(), Lookup::Unknown);
    }

    #[test]
    fn detects_other_volume_under_the_same_mount_point() {
        let (fs, registry) = setup();
        registry
            .store(&fs, record(2, "/mnt/data", "/mnt/data/.nurt-journal"))
            .unwrap();
        let replacement = volume_info(9, "/mnt/data", "ext4", VolumeKind::Fixed);
        assert_eq!(
            registry.lookup(&fs, &replacement).unwrap(),
            Lookup::Changed(RegistryChange::VolumeReplaced {
                previous: record(2, "/mnt/data", "/mnt/data/.nurt-journal").id,
            })
        );
    }

    #[test]
    fn detects_missing_journal_directory() {
        let (fs, registry) = setup();
        registry
            .store(&fs, record(2, "/mnt/data", "/mnt/data/.nurt-journal"))
            .unwrap();
        fs.remove_all("/mnt/data/.nurt-journal");
        let volume = volume_info(2, "/mnt/data", "ext4", VolumeKind::Fixed);
        assert_eq!(
            registry.lookup(&fs, &volume).unwrap(),
            Lookup::Changed(RegistryChange::JournalMissing {
                previous: abs("/mnt/data/.nurt-journal"),
            })
        );
    }

    #[test]
    fn follows_volume_mounted_at_another_place() {
        let (fs, registry) = setup();
        registry
            .store(&fs, record(2, "/media/old", "/media/old/.nurt-journal"))
            .unwrap();
        let mut volume = volume_info(2, "/mnt/data", "ext4", VolumeKind::Fixed);
        volume.id.dev = 77;
        assert_eq!(
            registry.lookup(&fs, &volume).unwrap(),
            Lookup::Relocated {
                dir: abs("/mnt/data/.nurt-journal"),
                previous: abs("/media/old"),
            }
        );
        fs.remove_all("/mnt/data/.nurt-journal");
        assert_eq!(
            registry.lookup(&fs, &volume).unwrap(),
            Lookup::Changed(RegistryChange::Remounted {
                previous: abs("/media/old"),
            })
        );
    }

    #[test]
    fn compares_device_numbers_without_uuid() {
        let (fs, registry) = setup();
        let mut stored = record(2, "/mnt/data", "/mnt/data/.nurt-journal");
        stored.id.uuid = None;
        registry.store(&fs, stored).unwrap();
        let mut volume = volume_info(2, "/mnt/data", "ext4", VolumeKind::Fixed);
        volume.id.uuid = None;
        assert_eq!(
            registry.lookup(&fs, &volume).unwrap(),
            Lookup::Current(abs("/mnt/data/.nurt-journal"))
        );
        volume.id.dev = 3;
        assert!(matches!(
            registry.lookup(&fs, &volume).unwrap(),
            Lookup::Changed(RegistryChange::VolumeReplaced { .. })
        ));
    }

    #[test]
    fn concurrent_writers_keep_every_record() {
        let (fs, registry) = setup();
        let fs = Arc::new(fs);
        let workers: Vec<_> = (0..6_u64)
            .map(|index| {
                let fs = Arc::clone(&fs);
                let registry = registry.clone();
                thread::spawn(move || {
                    let mount = format!("/mnt/v{index}");
                    let journal = format!("{mount}/.nurt-journal");
                    registry
                        .store(&*fs, record(10 + index, &mount, &journal))
                        .unwrap();
                })
            })
            .collect();
        for worker in workers {
            worker.join().unwrap();
        }
        assert_eq!(registry.load(&*fs).unwrap().len(), 6);
    }

    #[test]
    fn reports_lock_and_directory_failures() {
        let (fs, registry) = setup();
        fs.fail(
            "lock_exclusive",
            "/home/user/.local/state/nurt",
            FsErrorKind::InUse,
        );
        let error = registry
            .store(&fs, record(2, "/mnt/data", "/mnt/data/.nurt-journal"))
            .unwrap_err();
        assert_eq!(error.kind, FsErrorKind::InUse);

        let fs = TestFs::new();
        fs.add_volume(
            "/home",
            volume_info(1, "/home", "ext4", VolumeKind::Fixed),
            SPACE,
        );
        fs.write_file("/home/state", b"");
        let blocked = Registry::new(abs("/home/state/nurt"));
        let error = blocked
            .store(&fs, record(2, "/mnt/data", "/mnt/data/.nurt-journal"))
            .unwrap_err();
        assert_eq!(error.kind, FsErrorKind::NotADirectory);

        fs.mkdir_p("/home/user");
        fs.deny_write("/home/user");
        let denied = Registry::new(abs("/home/user/state"));
        let error = denied
            .store(&fs, record(2, "/mnt/data", "/mnt/data/.nurt-journal"))
            .unwrap_err();
        assert_eq!(error.kind, FsErrorKind::PermissionDenied);

        fs.fail("metadata", "/home/user", FsErrorKind::Io);
        let error = denied
            .store(&fs, record(2, "/mnt/data", "/mnt/data/.nurt-journal"))
            .unwrap_err();
        assert_eq!(error.kind, FsErrorKind::Io);
    }

    struct TestHost {
        info: HostInfo,
        vars: BTreeMap<String, String>,
    }

    impl TestHost {
        fn new(os: Os, vars: &[(&str, &str)]) -> Self {
            Self {
                info: HostInfo {
                    os,
                    os_version: "1".to_owned(),
                    architecture: "x86_64".to_owned(),
                    hostname: "example".to_owned(),
                    user: "user".to_owned(),
                    home: PathBuf::from("/home/user"),
                    temp: PathBuf::from("/tmp"),
                    path_style: PathStyle::Posix,
                    timezone: "UTC".to_owned(),
                    locale: None,
                    cpus: 1,
                },
                vars: vars
                    .iter()
                    .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
                    .collect(),
            }
        }
    }

    impl Host for TestHost {
        fn info(&self) -> &HostInfo {
            &self.info
        }

        fn env_var(&self, name: &str) -> Option<OsString> {
            self.vars.get(name).map(OsString::from)
        }

        fn env_vars(&self) -> Vec<(OsString, OsString)> {
            self.vars
                .iter()
                .map(|(name, value)| (name.into(), value.into()))
                .collect()
        }
    }

    #[test]
    fn locates_state_directory_per_system() {
        let config = StateConfig::default();
        let cases = [
            (Os::Linux, vec![], "/home/user/.local/state/nurt"),
            (
                Os::Linux,
                vec![("XDG_STATE_HOME", "/var/state")],
                "/var/state/nurt",
            ),
            (
                Os::Linux,
                vec![("XDG_STATE_HOME", "relative")],
                "/home/user/.local/state/nurt",
            ),
            (
                Os::Macos,
                vec![],
                "/home/user/Library/Application Support/Nurt",
            ),
            (
                Os::Windows,
                vec![("LOCALAPPDATA", "/home/user/local")],
                "/home/user/local/Nurt",
            ),
            (Os::Windows, vec![], "/home/user/AppData/Local/Nurt"),
        ];
        for (os, vars, expected) in cases {
            let host = TestHost::new(os, &vars);
            assert_eq!(state_dir(&host, &config).unwrap(), abs(expected), "{os:?}");
        }
    }

    #[test]
    fn state_directory_setting_takes_precedence() {
        let host = TestHost::new(Os::Linux, &[("XDG_STATE_HOME", "/var/state")]);
        let mut config = StateConfig::default();
        config.dir = Some("/srv/nurt-state".to_owned());
        assert_eq!(state_dir(&host, &config).unwrap(), abs("/srv/nurt-state"));
        config.dir = Some("relative".to_owned());
        assert_eq!(
            state_dir(&host, &config).unwrap_err().kind,
            FsErrorKind::InvalidPath
        );
    }
}
