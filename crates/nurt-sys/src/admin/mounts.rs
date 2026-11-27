// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! `mount` and `unmount`.
//!
//! | System | Read | Change |
//! |---|---|---|
//! | Linux | `findmnt` | `mount`, `umount`; `persistent: true` edits `/etc/fstab` |
//! | macOS | `mount` | `mount`, `umount` |
//! | Windows | `mountvol` | `mountvol` (drive letters and folder mounts; always persistent) |
//!
//! The edit of `/etc/fstab` is a file change of the journal, so `undo`
//! restores the file and runs the inverse program of the mount.

use nurt_contracts::env::Os;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::AbsPath;
use nurt_contracts::journal::FileOp;
use nurt_contracts::stage::BoundArgs;
use serde::{Deserialize, Serialize};

use super::sys::{Call, Sys};
use super::{Plan, StateChange, args};

/// File that lists the mounts made at boot on Linux.
pub(crate) const FSTAB: &str = "/etc/fstab";

/// State of a mount point as recorded in the journal.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub(crate) struct MountState {
    /// Device or volume.
    pub device: String,
    /// File system type; empty when it is detected by the system.
    #[serde(default)]
    pub fstype: String,
    /// Mount options.
    #[serde(default)]
    pub options: Vec<String>,
    /// The mount is also listed in `/etc/fstab`.
    #[serde(default)]
    pub persistent: bool,
}

impl MountState {
    /// The JSON document kept as `before` or `after`.
    pub(crate) fn to_text(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    /// The state from its JSON document.
    pub(crate) fn from_text(text: &str) -> Option<Self> {
        serde_json::from_str(text).ok()
    }
}

/// Target of the change of a mount point.
pub(crate) fn target(path: &str) -> String {
    format!("mount:{path}")
}

pub(crate) fn plan(sys: &dyn Sys, args: &BoundArgs) -> Result<Plan> {
    let mount_point = args::required(args, "target")?;
    args::operand("mount point", &mount_point)?;
    let persistent = args::flag(args, "persistent", false)?;
    if persistent && sys.os() == Os::Macos {
        return Err(Error::new(
            ErrorCode::Unsupported,
            "`persistent` is not supported on macOS; use `fstab` of the system or a launch daemon",
        ));
    }
    if args.command() == "mount" {
        mount(sys, args, &mount_point, persistent)
    } else {
        unmount(sys, &mount_point, persistent)
    }
}

fn mount(sys: &dyn Sys, args: &BoundArgs, mount_point: &str, persistent: bool) -> Result<Plan> {
    let device = args::required(args, "device")?;
    args::operand("device", &device)?;
    let fstype = args::text(args, "type")?.unwrap_or_default();
    if !fstype.is_empty() {
        args::name("file system type", &fstype, false)?;
    }
    let options = args::list(args, "options")?.unwrap_or_default();
    for option in &options {
        args::operand("mount option", option)?;
        if option.contains(',') || option.contains(char::is_whitespace) {
            return Err(Error::new(
                ErrorCode::InvalidValue,
                format!("`{option}` is not a mount option; give one option per list element"),
            )
            .with_found(option.clone()));
        }
    }
    let after = MountState {
        device: device.clone(),
        fstype: fstype.clone(),
        options: options.clone(),
        persistent,
    };
    if let Some(current) = read(sys, mount_point)? {
        if current.device == device {
            return Ok(Plan {
                unchanged: Some(format!("`{device}` is already mounted at `{mount_point}`")),
                ..Plan::default()
            });
        }
        return Err(Error::new(
            ErrorCode::InUse,
            format!("`{mount_point}` is a mount point of `{}`", current.device),
        )
        .with_fix(format!("unmount \"{mount_point}\"")));
    }
    let call = match sys.os() {
        Os::Windows => Call::new("mountvol", [mount_point, device.as_str()]),
        _ => {
            let mut call = Call::new("mount", Vec::<String>::new());
            if !fstype.is_empty() {
                call.args.extend(["-t".to_owned(), fstype.clone()]);
            }
            if !options.is_empty() {
                call.args.extend(["-o".to_owned(), options.join(",")]);
            }
            call.args.extend([device.clone(), mount_point.to_owned()]);
            call
        }
    };
    let mut files = Vec::new();
    if persistent && sys.os() == Os::Linux {
        let text = sys.read_file(FSTAB)?.unwrap_or_default();
        let edited = fstab_set(&text, &after, mount_point)?;
        files.push(FileOp::write(fstab_path()?, edited.into_bytes()));
    }
    Ok(Plan {
        calls: vec![call],
        files,
        changes: vec![StateChange::new(
            target(mount_point),
            None,
            Some(after.to_text()),
        )],
        ..Plan::default()
    })
}

fn unmount(sys: &dyn Sys, mount_point: &str, persistent: bool) -> Result<Plan> {
    let Some(before) = read(sys, mount_point)? else {
        return Err(Error::new(
            ErrorCode::NotFound,
            format!("`{mount_point}` is not a mount point"),
        ));
    };
    let call = match sys.os() {
        Os::Windows => Call::new("mountvol", [mount_point, "/D"]),
        _ => Call::new("umount", [mount_point]),
    };
    let mut files = Vec::new();
    let mut recorded = before.clone();
    if persistent && sys.os() == Os::Linux {
        let text = sys.read_file(FSTAB)?.unwrap_or_default();
        let edited = fstab_remove(&text, mount_point);
        if edited != text {
            files.push(FileOp::write(fstab_path()?, edited.into_bytes()));
            recorded.persistent = true;
        }
    }
    Ok(Plan {
        calls: vec![call],
        files,
        changes: vec![StateChange::new(
            target(mount_point),
            Some(recorded.to_text()),
            None,
        )],
        ..Plan::default()
    })
}

fn fstab_path() -> Result<AbsPath> {
    super::system_file(FSTAB)
}

/// The text of `/etc/fstab` with the line of `mount_point` replaced by the
/// line of `state`, or appended when the mount point has no line.
///
/// # Errors
///
/// Fails with `E_INVALID_VALUE` when a field holds a space, which the file
/// would need to escape.
pub(crate) fn fstab_set(text: &str, state: &MountState, mount_point: &str) -> Result<String> {
    for field in [state.device.as_str(), mount_point, state.fstype.as_str()] {
        if field.chars().any(char::is_whitespace) {
            return Err(Error::new(
                ErrorCode::InvalidValue,
                "a persistent mount cannot hold a space in the device, the mount point or the type",
            ));
        }
    }
    let fstype = if state.fstype.is_empty() {
        "auto"
    } else {
        &state.fstype
    };
    let options = if state.options.is_empty() {
        "defaults".to_owned()
    } else {
        state.options.join(",")
    };
    let line = format!(
        "{} {} {} {} 0 0",
        state.device, mount_point, fstype, options
    );
    let mut replaced = false;
    let mut out: Vec<String> = Vec::new();
    for existing in text.lines() {
        if fstab_target(existing) == Some(mount_point) {
            if !replaced {
                out.push(line.clone());
                replaced = true;
            }
        } else {
            out.push(existing.to_owned());
        }
    }
    if !replaced {
        out.push(line);
    }
    Ok(join_lines(&out))
}

/// The text of `/etc/fstab` without the lines of `mount_point`.
pub(crate) fn fstab_remove(text: &str, mount_point: &str) -> String {
    let kept: Vec<String> = text
        .lines()
        .filter(|line| fstab_target(line) != Some(mount_point))
        .map(str::to_owned)
        .collect();
    join_lines(&kept)
}

fn join_lines(lines: &[String]) -> String {
    if lines.is_empty() {
        String::new()
    } else {
        format!("{}\n", lines.join("\n"))
    }
}

/// Mount point of a line of `/etc/fstab`; comments and blank lines have none.
fn fstab_target(line: &str) -> Option<&str> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    line.split_whitespace().nth(1)
}

/// Reads the mount at `mount_point`; `None` when nothing is mounted there.
pub(crate) fn read(sys: &dyn Sys, mount_point: &str) -> Result<Option<MountState>> {
    match sys.os() {
        Os::Windows => {
            let output = sys.run(&Call::new("mountvol", [mount_point, "/L"]))?;
            let volume = output.stdout.trim();
            Ok(
                (output.ok() && volume.starts_with("\\\\?\\Volume")).then(|| MountState {
                    device: volume.to_owned(),
                    ..MountState::default()
                }),
            )
        }
        Os::Macos => {
            let output = sys.run(&Call::new("mount", Vec::<String>::new()))?;
            Ok(parse_mount_list(&output.stdout, mount_point))
        }
        _ => {
            let output = sys.run(&Call::new(
                "findmnt",
                ["-n", "-r", "-o", "SOURCE,FSTYPE,OPTIONS", "-M", mount_point],
            ))?;
            if !output.ok() {
                return Ok(None);
            }
            Ok(parse_findmnt(output.stdout.trim()))
        }
    }
}

/// A line of `findmnt -n -r -o SOURCE,FSTYPE,OPTIONS`.
pub(crate) fn parse_findmnt(line: &str) -> Option<MountState> {
    let mut fields = line.split_whitespace();
    let device = unescape(fields.next()?);
    let fstype = fields.next()?.to_owned();
    let options = fields
        .next()
        .map(|options| options.split(',').map(str::to_owned).collect())
        .unwrap_or_default();
    Some(MountState {
        device,
        fstype,
        options,
        persistent: false,
    })
}

fn unescape(text: &str) -> String {
    text.replace("\\x20", " ")
}

/// The line of the output of `mount` on macOS for `mount_point`:
/// `device on /path (type, option, option)`.
pub(crate) fn parse_mount_list(output: &str, mount_point: &str) -> Option<MountState> {
    output.lines().find_map(|line| {
        let (device, rest) = line.split_once(" on ")?;
        let (path, flags) = rest.rsplit_once(" (")?;
        if path != mount_point {
            return None;
        }
        let mut parts = flags.trim_end_matches(')').split(", ");
        let fstype = parts.next()?.to_owned();
        Some(MountState {
            device: device.to_owned(),
            fstype,
            options: parts.map(str::to_owned).collect(),
            persistent: false,
        })
    })
}
