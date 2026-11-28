---
command: disks.format
group: administration
summary: Create a file system on a device.
since: 0.1.1
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/commands/disks.format.md -->
<!-- Responsibility:   Documents disks.format. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# disks.format

## Synopsis

```text
disks.format <device> fs: <enum(ext4|xfs|btrfs|vfat|ntfs|exfat)> [name: value]...
```

## Description

`disks.format` creates a file system with `mkfs.<type>` on Linux, `diskutil eraseVolume` on macOS and `Format-Volume` on Windows. The change is irreversible. The preview and the report list the device, its partitions, file systems, mount points and the size of the data that is lost; a device with a mounted partition is refused with `E_IN_USE`. `fs` is `ext4`, `xfs`, `btrfs`, `vfat`, `ntfs` or `exfat` (macOS accepts `vfat` and `exfat`).

Rights: the command accepts `privilege`; write `privilege: elevated` when the account that runs Nurt is not the administrator. A program that fails for lack of rights ends the command with `E_PERMISSION`, and `privilege: auto` repeats it once elevated. An elevated entry is undone by the same elevated process.

A format is irreversible: the preview is the only safeguard.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `device` | `text` | required | Partition or disk such as /dev/sdb1, or a drive letter such as D: on Windows. |
| `fs` | `enum(ext4\|xfs\|btrfs\|vfat\|ntfs\|exfat)` | required | File system. |
| `label` | `text` | none | Label of the file system. |

## Input and output

| Form | Input | Output |
|---|---|---|
| default | `none` | `effect` |

## Effect

| Property | Value |
|---|---|
| Category | `change` |
| Effect | `external` |
| Reversibility | `irreversible` |
| Implementation | `system_program` |
| Reversibility conditions | Irreversible; the preview lists the device, its partitions, file systems, mount points and the size of the data that is lost. |
| External use | `system_program`: `mkfs`, `lsblk`, `diskutil`, `powershell`; File systems are created with mkfs on Linux, diskutil on macOS and Format-Volume on Windows. |

## Examples

Run a format with elevated rights; adding the call parameter `preview: true` shows the effect first.

```nurt example
disks.format "/dev/sdb1" fs: ext4 label: "data" privilege: elevated
```

Format a partition.

```nurt example
disks.format "/dev/sdb1" fs: xfs label: "backup" privilege: elevated
```

Format a drive of Windows.

```nurt example
disks.format "D:" fs: ntfs label: "Data" privilege: elevated
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_IN_USE` | A partition of the device is mounted. | Unmount it first. |
| `E_INVALID_VALUE` | The device is not a block device or the file system is unknown. | Check the device with `disks`. |
| `E_PERMISSION` | The account lacks the rights of the administrator. | Add `privilege: elevated`. |
| `E_EXTERNAL_MISSING` | The system program of the command is not installed. | Install the program named in the message. |
| `E_STAGE_FAILED` | The system program ended with an error. | Read the end of the error output in the message. |

## Related

`disks`, `disks.partition`, `unmount`.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
