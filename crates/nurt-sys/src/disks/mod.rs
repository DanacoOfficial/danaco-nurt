// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-sys::disks
// Responsibility:   Mounted volumes with capacity and free space.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Mounted volumes with capacity and free space.
//!
//! The `disks` command lists the mounted volumes of the host as `disk`
//! records, ordered by mount point. Pseudo file systems of the kernel
//! (`proc`, `sysfs`, `cgroup2` and similar) are not volumes and are left out.
//!
//! Volumes are classified as in the journal space rules: network file
//! systems, removable media, file systems in memory (`ram`) and fixed disks.
//! The overlay file system of containers counts as a fixed disk and keeps
//! `overlay` in `filesystem`.
//!
//! | System | Volumes | Space | Removable media |
//! |---|---|---|---|
//! | Linux | `/proc/self/mountinfo` | `statvfs` | `/sys/dev/block/<major>:<minor>/removable` |
//! | macOS | `getmntinfo` | `statfs` | `NSURLVolumeIsRemovableKey`, `NSURLVolumeIsInternalKey` |
//! | Windows | `GetLogicalDriveStringsW` | `GetDiskFreeSpaceExW` | `GetDriveTypeW` |
//!
//! `free` is the space available to the account; `used` is the capacity
//! minus all free blocks. Volumes whose space cannot be read are left out
//! and counted in one `source_unavailable` note of the report.

#[cfg(any(unix, test))]
mod classify;
#[cfg(any(target_os = "linux", all(test, unix)))]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(any(target_os = "linux", test))]
mod mountinfo;
#[cfg(test)]
mod tests;
#[cfg(windows)]
mod windows;

use std::sync::Arc;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::VolumeKind;
use nurt_contracts::record::{Disk, DiskType};
use nurt_contracts::report::{Notable, NotableEntry};
use nurt_contracts::stage::{
    BoundArgs, CheckCtx, ExecCtx, InputKinds, OrderKey, ReadStage, RecordStream, StageDescriptor,
    StageImpl, StageInput, StageOutput, StaticInfo, StreamOrder,
};
use nurt_contracts::value::{Path, Record, Size, Value};

/// Name of the command in the catalog.
pub const COMMAND: &str = "disks";

const PARAM_TYPE: &str = "type";

/// Values of the `type` parameter.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum TypeFilter {
    /// Every volume.
    #[default]
    All,
    /// Fixed disks.
    Fixed,
    /// Removable media.
    Removable,
    /// Network file systems.
    Network,
}

impl TypeFilter {
    const VALUES: [Self; 4] = [Self::All, Self::Fixed, Self::Removable, Self::Network];

    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Fixed => "fixed",
            Self::Removable => "removable",
            Self::Network => "network",
        }
    }

    fn admits(self, kind: DiskType) -> bool {
        match self {
            Self::All => true,
            Self::Fixed => kind == DiskType::Fixed,
            Self::Removable => kind == DiskType::Removable,
            Self::Network => kind == DiskType::Network,
        }
    }
}

/// Maps the volume class of the file system contract to the disk type of
/// the record.
pub(crate) fn disk_type(kind: VolumeKind) -> DiskType {
    match kind {
        VolumeKind::Fixed | VolumeKind::Overlay => DiskType::Fixed,
        VolumeKind::Removable => DiskType::Removable,
        VolumeKind::Network => DiskType::Network,
        VolumeKind::Memory => DiskType::Ram,
        _ => DiskType::Other,
    }
}

/// One mounted volume read from the operating system.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Volume {
    /// Mount point in canonical form.
    pub(crate) mount: String,
    pub(crate) device: String,
    pub(crate) filesystem: String,
    pub(crate) kind: Option<VolumeKind>,
    pub(crate) total: u64,
    /// Bytes available to the account.
    pub(crate) free: u64,
    pub(crate) used: u64,
    pub(crate) case_sensitive: bool,
    pub(crate) readonly: bool,
    pub(crate) label: Option<String>,
    pub(crate) inodes_total: Option<u64>,
    pub(crate) inodes_free: Option<u64>,
    pub(crate) options: Option<Vec<String>>,
}

impl Volume {
    fn into_record(self) -> Disk {
        Disk {
            mount: Path::from(self.mount),
            device: self.device,
            filesystem: self.filesystem,
            total: Size::from_bytes(self.total),
            free: Size::from_bytes(self.free),
            used: Size::from_bytes(self.used),
            kind: self.kind.map_or(DiskType::Other, disk_type),
            case_sensitive: self.case_sensitive,
            readonly: self.readonly,
            label: self.label,
            inodes_total: self.inodes_total,
            inodes_free: self.inodes_free,
            options: self.options,
            os_raw: None,
        }
    }
}

/// Volumes of the system.
#[derive(Debug, Default)]
pub(crate) struct VolumeList {
    pub(crate) volumes: Vec<Volume>,
    /// Volumes left out because their space could not be read.
    pub(crate) unreadable: u64,
}

/// Source of volume lists: the operating system or test data.
pub(crate) trait VolumeSource: Send + Sync {
    /// Returns the mounted volumes.
    ///
    /// # Errors
    ///
    /// Fails with `E_IO` when the mount table cannot be read.
    fn volumes(&self) -> Result<VolumeList>;
}

/// Implementation of the `disks` command.
///
/// ```
/// use nurt_contracts::stage::StageImpl;
/// use nurt_sys::disks::Disks;
///
/// assert_eq!(Disks::new().descriptor().command, "disks");
/// ```
#[derive(Clone)]
pub struct Disks {
    source: Arc<dyn VolumeSource>,
}

impl Disks {
    /// The command reading the volumes of the running system.
    #[must_use]
    pub fn new() -> Self {
        Self {
            source: system_source(),
        }
    }

    #[cfg(test)]
    pub(crate) fn with_source(source: Arc<dyn VolumeSource>) -> Self {
        Self { source }
    }
}

impl Default for Disks {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for Disks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Disks").finish_non_exhaustive()
    }
}

#[cfg(target_os = "linux")]
fn system_source() -> Arc<dyn VolumeSource> {
    Arc::new(linux::MountTable::live())
}

#[cfg(target_os = "macos")]
fn system_source() -> Arc<dyn VolumeSource> {
    Arc::new(macos::MountList)
}

#[cfg(windows)]
fn system_source() -> Arc<dyn VolumeSource> {
    Arc::new(windows::Drives)
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn system_source() -> Arc<dyn VolumeSource> {
    Arc::new(Unsupported)
}

/// Source of systems without a volume reader.
#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
struct Unsupported;

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
impl VolumeSource for Unsupported {
    fn volumes(&self) -> Result<VolumeList> {
        Err(Error::new(
            ErrorCode::OsOnly,
            "`disks` is available on Linux, macOS and Windows",
        ))
    }
}

fn type_filter(args: &BoundArgs) -> Result<TypeFilter> {
    match args.value(PARAM_TYPE) {
        None | Some(Value::Null) => Ok(TypeFilter::default()),
        Some(Value::Enum(text) | Value::Text(text)) => TypeFilter::VALUES
            .into_iter()
            .find(|value| value.as_str() == text)
            .ok_or_else(|| {
                Error::new(
                    ErrorCode::InvalidValue,
                    format!(
                        "`type` must be one of `all`, `fixed`, `removable`, `network`, got \
                         `{text}`"
                    ),
                )
            }),
        Some(_) => Err(Error::new(
            ErrorCode::Type,
            "`disks` expects an enumeration value for `type`",
        )),
    }
}

impl StageImpl for Disks {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: COMMAND.to_owned(),
            source: true,
            input: InputKinds::None,
        }
    }

    fn check(&self, args: &BoundArgs, _ctx: &CheckCtx<'_>) -> Result<StaticInfo> {
        type_filter(args)?;
        Ok(StaticInfo::default())
    }

    fn as_read(&self) -> Option<&dyn ReadStage> {
        Some(self)
    }
}

impl ReadStage for Disks {
    fn execute<'a>(
        &'a self,
        args: &'a BoundArgs,
        _input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<StageOutput<'a>> {
        let filter = type_filter(args)?;
        ctx.checkpoint()?;
        let list = self.source.volumes()?;
        if list.unreadable > 0 {
            let mut note = NotableEntry::new(Notable::SourceUnavailable);
            note.count = Some(list.unreadable);
            note.detail = Some(format!(
                "the space of {} mounted volumes could not be read",
                list.unreadable
            ));
            ctx.note(note);
        }
        let mut disks: Vec<Disk> = list
            .volumes
            .into_iter()
            .map(Volume::into_record)
            .filter(|disk| filter.admits(disk.kind))
            .collect();
        disks.sort_by(|a, b| a.mount.as_str().cmp(b.mount.as_str()));
        let records: Vec<Record> = disks.into_iter().map(Record::from).collect();
        Ok(
            StageOutput::records(RecordStream::from_records(records).checked(ctx)).with_order(
                StreamOrder {
                    keys: vec![OrderKey {
                        field: "mount".to_owned(),
                        descending: false,
                    }],
                },
            ),
        )
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
