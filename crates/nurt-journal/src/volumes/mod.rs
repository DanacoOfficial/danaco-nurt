// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Journal directories of volumes: eligibility, anchor selection and the
//! registry of the account.
//!
//! The journal is kept separately on every volume, in a hidden directory on
//! the same file system as the targets, so that previous content is preserved
//! by renaming. A volume without an eligible class or without a writable
//! anchor has no journal, and changes on it are reported as irreversible.

pub(crate) mod anchor;
pub(crate) mod eligibility;
pub(crate) mod registry;
#[cfg(test)]
mod testfs;

use std::sync::Arc;

use nurt_contracts::config::JournalConfig;
use nurt_contracts::fs::{
    AbsPath, FileKind, Follow, FsResult, FsWrite, ScopePurpose, TxScope, VolumeInfo,
};
use nurt_contracts::journal::{Eligibility, InactiveReason, OpId};

pub use self::eligibility::evaluate as evaluate_eligibility;
pub use self::registry::{Registry, RegistryChange, VolumeRecord, state_dir};

use self::registry::Lookup;
use crate::store::{JournalLocation, VolumeJournals, scope};

/// Operation of maintenance scopes: maintenance of journal directories and
/// state files belongs to no user operation, and operation numbers start at 1.
const MAINTENANCE_OP: OpId = OpId(0);

/// Scope for writes that maintain journal directories and state files.
pub(crate) fn maintenance_scope() -> TxScope {
    scope(MAINTENANCE_OP, ScopePurpose::Maintenance)
}

/// Journal location of a target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    /// Volume that contains the target.
    pub volume: VolumeInfo,
    /// Journal directory of the volume, or why there is none.
    pub place: JournalPlace,
}

/// Journal directory of a volume, or the reason why the volume has none.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JournalPlace {
    /// The journal is kept in `dir`.
    Active {
        /// Journal directory with its markers, format version file, volume
        /// description and layout directories.
        dir: AbsPath,
        /// Difference from the registry found while resolving, when any.
        change: Option<RegistryChange>,
    },
    /// The journal is not kept on the volume.
    Inactive(InactiveReason),
}

/// Finds or creates the journal directory for `target`.
///
/// The volume of the target is classified first. An ineligible volume yields
/// an inactive place without touching the disk. A journal directory recorded
/// in the registry is used when it still exists. Otherwise the anchor is the
/// mount point or the highest writable ancestor of the target on the same
/// volume and mount. The journal directory is created there and the choice is
/// recorded. A volume without a usable anchor yields the reason `no_anchor`.
///
/// Admission decides liveness of the space service and free space.
///
/// # Errors
///
/// Fails with the error of the file system when the volume of the target
/// cannot be determined, when the registry cannot be read or written, or when
/// creating the journal directory fails for another reason than a refused
/// anchor. Callers keep the operation and report the target as changed
/// without preservation.
pub fn resolve(
    fs: &dyn FsWrite,
    config: &JournalConfig,
    registry: &Registry,
    target: &AbsPath,
) -> FsResult<Resolved> {
    let volume = fs.volume(target)?;
    let within: Vec<AbsPath> = target.parent().into_iter().collect();
    let place = place(fs, config, registry, &volume, &within)?;
    Ok(Resolved { volume, place })
}

/// Journal place of `volume`; anchors are searched at the mount point and
/// then along the paths of `within` from the top.
fn place(
    fs: &dyn FsWrite,
    config: &JournalConfig,
    registry: &Registry,
    volume: &VolumeInfo,
    within: &[AbsPath],
) -> FsResult<JournalPlace> {
    if let Eligibility::Inactive(reason) = evaluate_eligibility(volume, config) {
        return Ok(JournalPlace::Inactive(reason));
    }
    let change = match registry.lookup(fs, volume)? {
        Lookup::Current(dir) => return Ok(JournalPlace::Active { dir, change: None }),
        Lookup::Relocated { dir, previous } => {
            registry.store(fs, record(volume, &dir))?;
            return Ok(JournalPlace::Active {
                dir,
                change: Some(RegistryChange::Remounted { previous }),
            });
        }
        Lookup::Changed(change) => change,
        Lookup::Unknown => RegistryChange::Created,
    };
    let name = anchor::journal_dir_name(&config.dir_name, anchor::HOST_OS);
    match anchor::select(fs, volume, within, &name, anchor::HOST_OS)? {
        Some(dir) => {
            registry.store(fs, record(volume, &dir))?;
            Ok(JournalPlace::Active {
                dir,
                change: Some(change),
            })
        }
        None => Ok(JournalPlace::Inactive(InactiveReason::NoAnchor)),
    }
}

fn record(volume: &VolumeInfo, dir: &AbsPath) -> VolumeRecord {
    VolumeRecord {
        id: volume.id.clone(),
        mount: volume.mount.clone(),
        fs_type: volume.fs_type.clone(),
        journal: dir.clone(),
    }
}

/// Journal directories of the account, located through the registry and
/// created on demand; the implementation of [`VolumeJournals`] used by the
/// journal store.
///
/// [`VolumeJournals::locate`] receives a volume without a target path, so the
/// anchor search uses the mount point and then the configured `anchor_paths`
/// (for example the home directory of the account) that lie on the volume.
pub struct JournalDirectories {
    fs: Arc<dyn FsWrite>,
    config: JournalConfig,
    registry: Registry,
    anchor_paths: Vec<AbsPath>,
}

impl JournalDirectories {
    /// Journal directories over `fs` with the `[journal]` configuration, the
    /// registry of the account and the paths searched for an anchor when the
    /// mount point of a volume is not writable.
    #[must_use]
    pub fn new(
        fs: Arc<dyn FsWrite>,
        config: JournalConfig,
        registry: Registry,
        anchor_paths: Vec<AbsPath>,
    ) -> Self {
        Self {
            fs,
            config,
            registry,
            anchor_paths,
        }
    }
}

impl VolumeJournals for JournalDirectories {
    /// Journal directory of `volume`. A file system error while locating it
    /// leaves the volume without a journal ([`InactiveReason::NoAnchor`]), so
    /// the operation proceeds without preservation.
    fn locate(&self, volume: &VolumeInfo) -> JournalLocation {
        let within: Vec<AbsPath> = self
            .anchor_paths
            .iter()
            .filter(|path| path.starts_with(&volume.mount))
            .cloned()
            .collect();
        match place(
            self.fs.as_ref(),
            &self.config,
            &self.registry,
            volume,
            &within,
        ) {
            Ok(JournalPlace::Active { dir, .. }) => JournalLocation::Active(dir),
            Ok(JournalPlace::Inactive(reason)) => JournalLocation::Inactive(reason),
            Err(_) => JournalLocation::Inactive(InactiveReason::NoAnchor),
        }
    }

    /// Recorded journal directories that exist, in path order; an unreadable
    /// registry yields none.
    fn active(&self) -> Vec<AbsPath> {
        let fs = self.fs.as_ref();
        let mut dirs: Vec<AbsPath> = self
            .registry
            .load(fs)
            .unwrap_or_default()
            .into_iter()
            .map(|record| record.journal)
            .filter(|dir| {
                fs.metadata(dir, Follow::NoFollow)
                    .is_ok_and(|meta| meta.kind == FileKind::Directory)
            })
            .collect();
        dirs.sort();
        dirs.dedup();
        dirs
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use nurt_contracts::config::{JournalConfig, VolumeOverride};
    use nurt_contracts::fs::{FsErrorKind, FsWrite, RenameMode, SpaceInfo, VolumeKind};
    use nurt_contracts::journal::InactiveReason;

    use super::{
        JournalDirectories, JournalPlace, Registry, RegistryChange, Resolved, maintenance_scope,
        resolve,
    };
    use crate::store::{JournalLocation, VolumeJournals};
    use crate::volumes::testfs::{TestFs, abs, volume_info};

    const SPACE: SpaceInfo = SpaceInfo {
        total: 1 << 30,
        available: 1 << 29,
        quota_remaining: None,
        quota_limit: None,
        purgeable_system: None,
    };

    /// Home volume with a read-only mount point, a writable data volume, a
    /// memory volume and the state directory on the home volume.
    fn setup() -> (TestFs, Registry) {
        let fs = TestFs::new();
        fs.add_volume(
            "/home",
            volume_info(1, "/home", "ext4", VolumeKind::Fixed),
            SPACE,
        );
        fs.add_volume(
            "/mnt/data",
            volume_info(2, "/mnt/data", "xfs", VolumeKind::Fixed),
            SPACE,
        );
        fs.add_volume(
            "/tmp",
            volume_info(3, "/tmp", "tmpfs", VolumeKind::Memory),
            SPACE,
        );
        fs.mkdir_p("/home/user/project");
        fs.deny_write("/home");
        (fs, Registry::new(abs("/home/user/.local/state/nurt")))
    }

    fn journal_dir(resolved: &Resolved) -> String {
        match &resolved.place {
            JournalPlace::Active { dir, .. } => dir.to_string(),
            JournalPlace::Inactive(reason) => panic!("inactive: {}", reason.as_str()),
        }
    }

    fn expected_name() -> &'static str {
        if cfg!(target_os = "macos") {
            ".nurt-journal.noindex"
        } else {
            ".nurt-journal"
        }
    }

    /// Copies the registry of `from` into a new file system that sees other
    /// volumes.
    fn with_registry_of(from: &TestFs, to: &TestFs) {
        to.mkdir_p("/home/user/.local/state/nurt");
        to.write_file(
            "/home/user/.local/state/nurt/volumes.json",
            &from
                .read("/home/user/.local/state/nurt/volumes.json")
                .unwrap(),
        );
    }

    #[test]
    fn creates_and_records_journal_directory_once() {
        let (fs, registry) = setup();
        let config = JournalConfig::default();
        let target = abs("/home/user/project/notes.txt");
        let first = resolve(&fs, &config, &registry, &target).unwrap();
        let dir = format!("/home/user/{}", expected_name());
        assert_eq!(
            first.place,
            JournalPlace::Active {
                dir: abs(&dir),
                change: Some(RegistryChange::Created),
            }
        );
        assert_eq!(registry.load(&fs).unwrap()[0].journal, abs(&dir));

        let calls = fs.calls().len();
        let second = resolve(&fs, &config, &registry, &target).unwrap();
        assert_eq!(
            second.place,
            JournalPlace::Active {
                dir: abs(&dir),
                change: None,
            }
        );
        assert_eq!(fs.calls().len(), calls);
    }

    #[test]
    fn reports_ineligible_volume_without_writing() {
        let (fs, registry) = setup();
        let resolved = resolve(
            &fs,
            &JournalConfig::default(),
            &registry,
            &abs("/tmp/build/out.o"),
        )
        .unwrap();
        assert_eq!(
            resolved.place,
            JournalPlace::Inactive(InactiveReason::MemoryVolume)
        );
        assert_eq!(resolved.volume.mount, abs("/tmp"));
        assert!(fs.calls().is_empty());
    }

    #[test]
    fn enables_memory_volume_by_per_volume_setting() {
        let (fs, registry) = setup();
        let mut config = JournalConfig::default();
        let mut settings = VolumeOverride::default();
        settings.enabled = Some(true);
        config.volume.insert("/tmp".to_owned(), settings);
        let resolved = resolve(&fs, &config, &registry, &abs("/tmp/out.o")).unwrap();
        assert_eq!(journal_dir(&resolved), format!("/tmp/{}", expected_name()));
    }

    #[test]
    fn reports_no_anchor_when_volume_is_not_writable() {
        let (fs, registry) = setup();
        fs.deny_write("/home/user");
        fs.deny_write("/home/user/project");
        let resolved = resolve(
            &fs,
            &JournalConfig::default(),
            &registry,
            &abs("/home/user/project/notes.txt"),
        )
        .unwrap();
        assert_eq!(
            resolved.place,
            JournalPlace::Inactive(InactiveReason::NoAnchor)
        );
    }

    #[test]
    fn recreates_journal_directory_that_disappeared() {
        let (fs, registry) = setup();
        let config = JournalConfig::default();
        let target = abs("/mnt/data/file");
        let first = resolve(&fs, &config, &registry, &target).unwrap();
        let dir = journal_dir(&first);
        fs.remove_all(&dir);
        let second = resolve(&fs, &config, &registry, &target).unwrap();
        assert_eq!(
            second.place,
            JournalPlace::Active {
                dir: abs(&dir),
                change: Some(RegistryChange::JournalMissing {
                    previous: abs(&dir),
                }),
            }
        );
        assert!(fs.exists(&format!("{dir}/CACHEDIR.TAG")));
    }

    #[test]
    fn detects_another_volume_mounted_at_the_same_place() {
        let (fs, registry) = setup();
        let config = JournalConfig::default();
        let target = abs("/mnt/data/file");
        resolve(&fs, &config, &registry, &target).unwrap();

        let replaced = TestFs::new();
        replaced.add_volume(
            "/home",
            volume_info(1, "/home", "ext4", VolumeKind::Fixed),
            SPACE,
        );
        replaced.add_volume(
            "/mnt/data",
            volume_info(5, "/mnt/data", "ext4", VolumeKind::Fixed),
            SPACE,
        );
        with_registry_of(&fs, &replaced);
        let resolved = resolve(&replaced, &config, &registry, &target).unwrap();
        let JournalPlace::Active {
            change: Some(RegistryChange::VolumeReplaced { previous }),
            ..
        } = resolved.place
        else {
            panic!("expected a replaced volume: {:?}", resolved.place);
        };
        assert_eq!(previous.dev, 2);
        assert_eq!(registry.load(&replaced).unwrap()[0].id.dev, 5);
    }

    #[test]
    fn follows_volume_mounted_at_another_place() {
        let (fs, registry) = setup();
        let config = JournalConfig::default();
        resolve(&fs, &config, &registry, &abs("/mnt/data/file")).unwrap();

        let moved = TestFs::new();
        moved.add_volume(
            "/home",
            volume_info(1, "/home", "ext4", VolumeKind::Fixed),
            SPACE,
        );
        moved.add_volume(
            "/media/data",
            volume_info(2, "/media/data", "xfs", VolumeKind::Fixed),
            SPACE,
        );
        moved.mkdir_p(&format!("/media/data/{}", expected_name()));
        with_registry_of(&fs, &moved);
        let resolved = resolve(&moved, &config, &registry, &abs("/media/data/file")).unwrap();
        assert_eq!(
            resolved.place,
            JournalPlace::Active {
                dir: abs(&format!("/media/data/{}", expected_name())),
                change: Some(RegistryChange::Remounted {
                    previous: abs("/mnt/data"),
                }),
            }
        );
        assert_eq!(registry.load(&moved).unwrap().len(), 2);
    }

    #[test]
    fn passes_on_file_system_failures() {
        let (fs, registry) = setup();
        let config = JournalConfig::default();
        let failures = [
            ("volume", "/mnt/data", FsErrorKind::Io),
            ("open_read", "/home/user/.local", FsErrorKind::Io),
            ("lock_exclusive", "/home/user/.local", FsErrorKind::InUse),
        ];
        for (op, path, kind) in failures {
            fs.clear_failures();
            fs.fail(op, path, kind);
            let error = resolve(&fs, &config, &registry, &abs("/mnt/data/file")).unwrap_err();
            assert_eq!(error.kind, kind, "{op}");
        }
    }

    #[test]
    fn overlay_journal_lives_in_upper_layer_and_lower_directories_need_copy() {
        let fs = TestFs::new();
        fs.add_volume(
            "/",
            volume_info(8, "/", "overlay", VolumeKind::Overlay),
            SPACE,
        );
        fs.mkdir_p("/home/user");
        fs.mkdir_p("/usr/share/doc/tool");
        fs.write_file("/usr/share/doc/tool/README", b"lower layer content");
        fs.mark_lower("/usr/share/doc/tool");
        let registry = Registry::new(abs("/home/user/.local/state/nurt"));
        let resolved = resolve(
            &fs,
            &JournalConfig::default(),
            &registry,
            &abs("/usr/share/doc/tool"),
        )
        .unwrap();
        let dir = journal_dir(&resolved);
        assert_eq!(dir, format!("/{}", expected_name()));

        let scope = maintenance_scope();
        let kept = abs(&format!("{dir}/ops/tool"));
        let error = fs
            .rename(
                &scope,
                &abs("/usr/share/doc/tool"),
                &kept,
                RenameMode::NoReplace,
            )
            .unwrap_err();
        assert_eq!(error.kind, FsErrorKind::CrossDevice);

        fs.create_dir(&scope, &kept, 0o700).unwrap();
        let copied = fs
            .copy_file(
                &scope,
                &abs("/usr/share/doc/tool/README"),
                &abs(&format!("{dir}/ops/tool/README")),
            )
            .unwrap();
        assert_eq!(copied, 19);
        assert_eq!(
            fs.read(&format!("{dir}/ops/tool/README")).unwrap(),
            b"lower layer content".to_vec()
        );
    }

    #[test]
    fn locates_journals_for_the_journal_store() {
        let (fs, registry) = setup();
        let fs = Arc::new(fs);
        let journals = JournalDirectories::new(
            fs.clone(),
            JournalConfig::default(),
            registry,
            vec![abs("/home/user"), abs("/mnt/data/other")],
        );
        let home = volume_info(1, "/home", "ext4", VolumeKind::Fixed);
        let data = volume_info(2, "/mnt/data", "xfs", VolumeKind::Fixed);
        let memory = volume_info(3, "/tmp", "tmpfs", VolumeKind::Memory);
        let home_dir = abs(&format!("/home/user/{}", expected_name()));
        let data_dir = abs(&format!("/mnt/data/{}", expected_name()));
        assert_eq!(
            journals.locate(&home),
            JournalLocation::Active(home_dir.clone())
        );
        assert_eq!(
            journals.locate(&data),
            JournalLocation::Active(data_dir.clone())
        );
        assert_eq!(
            journals.locate(&memory),
            JournalLocation::Inactive(InactiveReason::MemoryVolume)
        );
        assert_eq!(journals.active(), [home_dir, data_dir.clone()]);

        fs.remove_all(&data_dir.to_string());
        fs.deny_write("/mnt/data");
        assert_eq!(
            journals.locate(&data),
            JournalLocation::Inactive(InactiveReason::NoAnchor)
        );
        fs.fail("volume", "/mnt/data", FsErrorKind::Io);
        fs.fail("open_read", "/home/user/.local", FsErrorKind::Io);
        assert_eq!(
            journals.locate(&home),
            JournalLocation::Inactive(InactiveReason::NoAnchor)
        );
        assert!(journals.active().is_empty());
    }
}
