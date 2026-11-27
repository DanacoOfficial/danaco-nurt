// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-sys::admin::disks
// Responsibility:   `disks.partition` and `disks.format`.
// Dependencies:     nurt_contracts, serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! `disks.partition` and `disks.format`.
//!
//! | System | Read | Partition | Format |
//! |---|---|---|---|
//! | Linux | `lsblk`, `parted -m` | `parted mkpart`, `rm`, `resizepart` | `mkfs.<type>` |
//! | macOS | `diskutil` | `diskutil addPartition`, `eraseVolume`, `resizeVolume` | `diskutil eraseVolume` |
//! | Windows | `Get-Partition` | `New-Partition`, `Remove-Partition`, `Resize-Partition` | `Format-Volume` |
//!
//! Both commands are irreversible. Their preview lists the device, its
//! partitions, file systems, mount points and the size of the data that the
//! change destroys, and a partition or device that is mounted is refused with
//! `E_IN_USE`.

use nurt_contracts::env::Os;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::stage::BoundArgs;
use nurt_contracts::value::Value;
use serde_json::Value as Json;

use super::sys::{Call, Sys};
use super::{Plan, StateChange, args};

/// Start of the first partition when `start` is not given.
const DEFAULT_START: u64 = 1024 * 1024;

pub(crate) fn plan(sys: &dyn Sys, args: &BoundArgs) -> Result<Plan> {
    let device = args::required(args, "device")?;
    check_device(sys.os(), &device)?;
    if args.command() == "disks.format" {
        format(sys, args, &device)
    } else {
        partition(sys, args, &device)
    }
}

fn check_device(os: Os, device: &str) -> Result<()> {
    let good = match os {
        Os::Windows => {
            device.chars().all(|c| c.is_ascii_digit())
                || (device.len() == 2 && device.ends_with(':'))
        }
        _ => device.starts_with("/dev/") && !device.contains(".."),
    };
    if good && args::operand("device", device).is_ok() {
        Ok(())
    } else {
        Err(Error::new(
            ErrorCode::InvalidValue,
            format!("`{device}` is not a disk or a partition of this system"),
        )
        .with_found(device.to_owned()))
    }
}

fn size_arg(args: &BoundArgs, name: &str) -> Result<Option<u64>> {
    match args.value(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Size(size)) => Ok(Some(size.bytes())),
        Some(Value::Int(bytes)) if *bytes >= 0 => Ok(u64::try_from(*bytes).ok()),
        Some(other) => Err(Error::new(
            ErrorCode::Type,
            format!("`{name}` expects a size, found {}", other.type_name()),
        )),
    }
}

/// The path of partition `number` of `device`: `/dev/sda` gives `/dev/sda1`,
/// `/dev/nvme0n1` gives `/dev/nvme0n1p1`.
pub(crate) fn partition_path(device: &str, number: u32) -> String {
    if device.ends_with(|c: char| c.is_ascii_digit()) {
        format!("{device}p{number}")
    } else {
        format!("{device}{number}")
    }
}

fn partition(sys: &dyn Sys, args: &BoundArgs, device: &str) -> Result<Plan> {
    let action = args::required(args, "action")?;
    let number = args::number(args, "number", 1_000_000)?.and_then(|n| u32::try_from(n).ok());
    let start = size_arg(args, "start")?;
    let size = size_arg(args, "size")?;
    let fs_type = args::text(args, "fs_type")?;
    if let Some(fs_type) = &fs_type {
        args::name("file system type", fs_type, false)?;
    }
    let needs_number = action != "create";
    if needs_number && number.is_none() {
        return Err(Error::new(
            ErrorCode::MissingParam,
            format!("`disks.partition action: {action}` needs `number`"),
        ));
    }
    if action == "resize" && size.is_none() {
        return Err(Error::new(
            ErrorCode::MissingParam,
            "`disks.partition action: resize` needs `size`",
        ));
    }
    let layout = if sys.os() == Os::Linux {
        read_layout(sys, device)?
    } else {
        Layout::default()
    };
    let mut notes = vec![layout.describe(device)];
    let mut lost = 0;
    if let Some(number) = number.filter(|_| needs_number) {
        let path = partition_path(device, number);
        if let Some(part) = layout.find(&path) {
            if let Some(mount) = &part.mountpoint {
                return Err(in_use(&path, mount));
            }
            lost = part.used;
        } else if sys.os() == Os::Linux {
            return Err(Error::new(
                ErrorCode::NotFound,
                format!("`{device}` has no partition {number}"),
            ));
        }
    }
    notes.push(format!("data that will be lost: {lost} bytes"));
    let call = match sys.os() {
        Os::Windows => windows_partition(device, &action, number, size)?,
        Os::Macos => macos_partition(device, &action, number, size, fs_type.as_deref())?,
        _ => linux_partition(
            sys,
            device,
            &action,
            number,
            start,
            size,
            fs_type.as_deref(),
        )?,
    };
    let target = format!("disk:{device}");
    Ok(Plan {
        calls: vec![call],
        changes: vec![StateChange::new(
            target,
            Some(layout.to_text()),
            Some(format!("partition {action}")),
        )],
        irreversible: Some(format!(
            "a partition change of `{device}` cannot be undone; {lost} bytes of data are lost"
        )),
        notes,
        ..Plan::default()
    })
}

fn linux_partition(
    sys: &dyn Sys,
    device: &str,
    action: &str,
    number: Option<u32>,
    start: Option<u64>,
    size: Option<u64>,
    fs_type: Option<&str>,
) -> Result<Call> {
    let base = ["-s", "-a", "optimal", device];
    let mut call = Call::new("parted", base);
    match action {
        "create" => {
            let start = start.unwrap_or(DEFAULT_START);
            call.args
                .extend(["mkpart".to_owned(), "primary".to_owned()]);
            if let Some(fs_type) = fs_type {
                call.args.push(fs_type.to_owned());
            }
            call.args.push(format!("{start}B"));
            call.args.push(size.map_or_else(
                || "100%".to_owned(),
                |size| format!("{}B", start.saturating_add(size).saturating_sub(1)),
            ));
        }
        "delete" => {
            call.args
                .extend(["rm".to_owned(), number.unwrap_or(0).to_string()]);
        }
        "resize" => {
            let number = number.unwrap_or(0);
            let begin = partition_start(sys, device, number)?;
            let end = begin.saturating_add(size.unwrap_or(0)).saturating_sub(1);
            call.args.extend([
                "resizepart".to_owned(),
                number.to_string(),
                format!("{end}B"),
            ]);
        }
        other => {
            return Err(Error::new(
                ErrorCode::InvalidValue,
                format!("`action` is `create`, `delete` or `resize`, not `{other}`"),
            ));
        }
    }
    Ok(call)
}

/// First byte of partition `number` according to `parted -m`.
fn partition_start(sys: &dyn Sys, device: &str, number: u32) -> Result<u64> {
    let output = sys.run(&Call::new(
        "parted",
        ["-m", "-s", device, "unit", "B", "print"],
    ))?;
    parse_parted_start(&output.stdout, number).ok_or_else(|| {
        Error::new(
            ErrorCode::NotFound,
            format!("`{device}` has no partition {number}"),
        )
    })
}

/// Start of a partition in the machine output of `parted -m ... print`:
/// `number:startB:endB:sizeB:filesystem:name:flags;`.
pub(crate) fn parse_parted_start(output: &str, number: u32) -> Option<u64> {
    output.lines().find_map(|line| {
        let fields: Vec<&str> = line.trim_end_matches(';').split(':').collect();
        if fields.first()?.parse::<u32>().ok()? != number {
            return None;
        }
        fields.get(1)?.trim_end_matches('B').parse().ok()
    })
}

fn macos_partition(
    device: &str,
    action: &str,
    number: Option<u32>,
    size: Option<u64>,
    fs_type: Option<&str>,
) -> Result<Call> {
    let part = format!("{device}s{}", number.unwrap_or(0));
    let call = match action {
        "create" => Call::new(
            "diskutil",
            [
                "addPartition".to_owned(),
                device.to_owned(),
                fs_type.unwrap_or("ExFAT").to_owned(),
                "NURT".to_owned(),
                size.map_or_else(|| "R".to_owned(), |size| format!("{size}b")),
            ],
        ),
        "delete" => Call::new(
            "diskutil",
            ["eraseVolume", "Free Space", "%noformat%", &part],
        ),
        "resize" => Call::new(
            "diskutil",
            [
                "resizeVolume".to_owned(),
                part,
                format!("{}b", size.unwrap_or(0)),
            ],
        ),
        other => {
            return Err(Error::new(
                ErrorCode::InvalidValue,
                format!("`action` is `create`, `delete` or `resize`, not `{other}`"),
            ));
        }
    };
    Ok(call)
}

fn windows_partition(
    device: &str,
    action: &str,
    number: Option<u32>,
    size: Option<u64>,
) -> Result<Call> {
    if !device.chars().all(|c| c.is_ascii_digit()) {
        return Err(Error::new(
            ErrorCode::InvalidValue,
            "`device` of `disks.partition` on Windows is the number of the disk",
        ));
    }
    let script = match action {
        "create" => format!(
            "New-Partition -DiskNumber {device} {} -AssignDriveLetter | Out-Null",
            size.map_or_else(
                || "-UseMaximumSize".to_owned(),
                |size| format!("-Size {size}")
            )
        ),
        "delete" => format!(
            "Remove-Partition -DiskNumber {device} -PartitionNumber {} -Confirm:$false",
            number.unwrap_or(0)
        ),
        "resize" => format!(
            "Resize-Partition -DiskNumber {device} -PartitionNumber {} -Size {}",
            number.unwrap_or(0),
            size.unwrap_or(0)
        ),
        other => {
            return Err(Error::new(
                ErrorCode::InvalidValue,
                format!("`action` is `create`, `delete` or `resize`, not `{other}`"),
            ));
        }
    };
    Ok(Call::new(
        "powershell",
        ["-NoProfile", "-NonInteractive", "-Command", &script],
    ))
}

fn format(sys: &dyn Sys, args: &BoundArgs, device: &str) -> Result<Plan> {
    let fs = args::required(args, "fs")?;
    let label = args::text(args, "label")?;
    if let Some(label) = &label {
        args::name("label", label, true)?;
    }
    let layout = if sys.os() == Os::Linux {
        read_layout(sys, device)?
    } else {
        Layout::default()
    };
    if sys.os() == Os::Linux {
        if layout.parts.is_empty() {
            return Err(Error::new(
                ErrorCode::NotFound,
                format!("`{device}` is not a block device"),
            ));
        }
        if let Some((path, mount)) = layout.mounted() {
            return Err(in_use(&path, &mount));
        }
    }
    let lost = layout.used();
    let call = match sys.os() {
        Os::Windows => windows_format(device, &fs, label.as_deref())?,
        Os::Macos => macos_format(device, &fs, label.as_deref())?,
        _ => linux_format(device, &fs, label.as_deref())?,
    };
    Ok(Plan {
        calls: vec![call],
        changes: vec![StateChange::new(
            format!("disk:{device}"),
            Some(layout.to_text()),
            Some(format!("file system {fs}")),
        )],
        irreversible: Some(format!(
            "the file system of `{device}` is replaced; {lost} bytes of data are lost"
        )),
        notes: vec![
            layout.describe(device),
            format!("data that will be lost: {lost} bytes"),
        ],
        ..Plan::default()
    })
}

fn linux_format(device: &str, fs: &str, label: Option<&str>) -> Result<Call> {
    let (program, force, label_flag): (&str, &[&str], &str) = match fs {
        "ext4" => ("mkfs.ext4", &["-F"], "-L"),
        "xfs" => ("mkfs.xfs", &["-f"], "-L"),
        "btrfs" => ("mkfs.btrfs", &["-f"], "-L"),
        "vfat" => ("mkfs.vfat", &[], "-n"),
        "ntfs" => ("mkfs.ntfs", &["-Q"], "-L"),
        "exfat" => ("mkfs.exfat", &[], "-L"),
        other => {
            return Err(Error::new(
                ErrorCode::InvalidValue,
                format!("`fs` is one of ext4, xfs, btrfs, vfat, ntfs, exfat, not `{other}`"),
            ));
        }
    };
    let mut call = Call::new(program, force.iter().copied());
    if let Some(label) = label {
        call.args.extend([label_flag.to_owned(), label.to_owned()]);
    }
    call.args.push(device.to_owned());
    Ok(call)
}

fn macos_format(device: &str, fs: &str, label: Option<&str>) -> Result<Call> {
    let kind = match fs {
        "vfat" => "FAT32",
        "exfat" => "ExFAT",
        other => {
            return Err(Error::new(
                ErrorCode::Unsupported,
                format!("macOS formats `vfat` and `exfat` only, not `{other}`"),
            ));
        }
    };
    Ok(Call::new(
        "diskutil",
        ["eraseVolume", kind, label.unwrap_or("NURT"), device],
    ))
}

fn windows_format(device: &str, fs: &str, label: Option<&str>) -> Result<Call> {
    let letter = device.trim_end_matches(':');
    if device.len() != 2
        || !device.ends_with(':')
        || !letter.chars().all(|c| c.is_ascii_alphabetic())
    {
        return Err(Error::new(
            ErrorCode::InvalidValue,
            "`device` of `disks.format` on Windows is a drive letter such as `D:`",
        ));
    }
    let kind = match fs {
        "ntfs" => "NTFS",
        "exfat" => "exFAT",
        "vfat" => "FAT32",
        other => {
            return Err(Error::new(
                ErrorCode::Unsupported,
                format!("Windows formats `ntfs`, `exfat` and `vfat`, not `{other}`"),
            ));
        }
    };
    let mut script =
        format!("Format-Volume -DriveLetter {letter} -FileSystem {kind} -Confirm:$false");
    if let Some(label) = label {
        script.push_str(" -NewFileSystemLabel $env:NURT_LABEL");
        return Ok(Call::new(
            "powershell",
            ["-NoProfile", "-NonInteractive", "-Command", &script],
        )
        .with_env("NURT_LABEL", label));
    }
    Ok(Call::new(
        "powershell",
        ["-NoProfile", "-NonInteractive", "-Command", &script],
    ))
}

fn in_use(path: &str, mount: &str) -> Error {
    Error::new(
        ErrorCode::InUse,
        format!("`{path}` is mounted at `{mount}`; unmount it first"),
    )
    .with_fix(format!("unmount \"{mount}\""))
}

/// One partition or device of a layout.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct Part {
    /// Path of the device node.
    pub path: String,
    /// Size in bytes.
    pub size: u64,
    /// File system type.
    pub fstype: Option<String>,
    /// Label of the file system.
    pub label: Option<String>,
    /// Mount point.
    pub mountpoint: Option<String>,
    /// Bytes in use.
    pub used: u64,
}

/// The device and its partitions as `lsblk` reports them.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct Layout {
    /// The device first, then its partitions.
    pub parts: Vec<Part>,
}

impl Layout {
    fn find(&self, path: &str) -> Option<&Part> {
        self.parts.iter().find(|part| part.path == path)
    }

    fn mounted(&self) -> Option<(String, String)> {
        self.parts
            .iter()
            .find_map(|part| Some((part.path.clone(), part.mountpoint.clone()?)))
    }

    fn used(&self) -> u64 {
        self.parts.iter().map(|part| part.used).sum()
    }

    /// One line for the preview.
    pub(crate) fn describe(&self, device: &str) -> String {
        if self.parts.is_empty() {
            return format!("{device}: layout not read on this system");
        }
        let items: Vec<String> = self
            .parts
            .iter()
            .map(|part| {
                format!(
                    "{} {} bytes fs={} label={} mount={} used={}",
                    part.path,
                    part.size,
                    part.fstype.as_deref().unwrap_or("-"),
                    part.label.as_deref().unwrap_or("-"),
                    part.mountpoint.as_deref().unwrap_or("-"),
                    part.used
                )
            })
            .collect();
        format!("{device}: {}", items.join("; "))
    }

    /// The text kept as the state before.
    pub(crate) fn to_text(&self) -> String {
        let items: Vec<Json> = self
            .parts
            .iter()
            .map(|part| {
                serde_json::json!({
                    "path": part.path,
                    "size": part.size,
                    "fstype": part.fstype,
                    "label": part.label,
                    "mountpoint": part.mountpoint,
                    "used": part.used,
                })
            })
            .collect();
        serde_json::to_string(&items).unwrap_or_default()
    }
}

fn read_layout(sys: &dyn Sys, device: &str) -> Result<Layout> {
    let output = sys.run(&Call::new(
        "lsblk",
        [
            "-J",
            "-b",
            "-p",
            "-o",
            "NAME,SIZE,FSTYPE,LABEL,MOUNTPOINT,FSUSED",
            device,
        ],
    ))?;
    if !output.ok() {
        return Err(Error::new(
            ErrorCode::NotFound,
            format!("`{device}` is not a block device"),
        ));
    }
    parse_lsblk(&output.stdout)
}

/// The JSON output of `lsblk -J -b -p -o NAME,SIZE,FSTYPE,LABEL,MOUNTPOINT,FSUSED`.
///
/// # Errors
///
/// Fails with `E_OUTPUT_PARSE` when the output is not JSON.
pub(crate) fn parse_lsblk(text: &str) -> Result<Layout> {
    let json: Json = serde_json::from_str(text)
        .map_err(|error| Error::new(ErrorCode::OutputParse, format!("lsblk output: {error}")))?;
    let mut parts = Vec::new();
    collect(json.get("blockdevices"), &mut parts);
    Ok(Layout { parts })
}

fn collect(devices: Option<&Json>, parts: &mut Vec<Part>) {
    let Some(devices) = devices.and_then(Json::as_array) else {
        return;
    };
    for device in devices {
        let text = |key: &str| device.get(key).and_then(Json::as_str).map(str::to_owned);
        let number = |key: &str| match device.get(key) {
            Some(Json::Number(number)) => number.as_u64(),
            Some(Json::String(text)) => text.parse().ok(),
            _ => None,
        };
        parts.push(Part {
            path: text("name").unwrap_or_default(),
            size: number("size").unwrap_or(0),
            fstype: text("fstype"),
            label: text("label"),
            mountpoint: text("mountpoint"),
            used: number("fsused").unwrap_or(0),
        });
        collect(device.get("children"), parts);
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
