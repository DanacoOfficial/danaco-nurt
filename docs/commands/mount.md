---
command: mount
group: administration
summary: Mount a device or volume.
since: 0.1.1
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/commands/mount.md -->
<!-- Responsibility:   Documents mount. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# mount

## Synopsis

```text
mount <device> <target> [name: value]...
```

## Description

`mount` mounts a device or volume with `mount` on Linux and macOS and `mountvol` on Windows (drive letters and folder mounts). The mount point is read first: the same device already mounted there is `unchanged`, another device is `E_IN_USE`. With `persistent: true` on Linux the line of `/etc/fstab` for the mount point is written or replaced; the edit is a file change of the journal. macOS does not support `persistent`.

Rights: the command accepts `privilege`; write `privilege: elevated` when the account that runs Nurt is not the administrator. A program that fails for lack of rights ends the command with `E_PERMISSION`, and `privilege: auto` repeats it once elevated. An elevated entry is undone by the same elevated process.

`undo` runs `unmount`; the journal restores `/etc/fstab`.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `device` | `text` | required | Device, volume or source of the mount. |
| `target` | `text` | required | Mount point: a directory, or a drive letter or folder on Windows. |
| `type` | `text` | none | File system type; detected by the system when omitted. |
| `options` | `list<text>` | none | Mount options. |
| `persistent` | `bool` | `false` | Also write the mount to /etc/fstab (Linux); the file is journaled. |

## Input and output

| Form | Input | Output |
|---|---|---|
| default | `none` | `effect` |

## Effect

| Property | Value |
|---|---|
| Category | `change` |
| Effect | `external` |
| Reversibility | `reversible_if_preserved` |
| Implementation | `system_program` |
| Reversibility conditions | Undo unmounts the target; the edit of /etc/fstab is restored by the journal. |
| External use | `system_program`: `mount`, `mountvol`; Mounts use mount on Linux and macOS and mountvol on Windows. |

## Examples

Mount a device.

```nurt example
mount "/dev/sdb1" "/srv/data" type: "ext4" privilege: elevated
```

Mount read-only and keep the mount after a restart.

```nurt example
mount "/dev/sdb1" "/srv/data" type: "ext4" options: ["ro", "noatime"] persistent: true privilege: elevated
```

Mount a USB stick for one account.

```nurt example
mount "/dev/sdc1" "/mnt/usb" type: "vfat" options: ["uid=1000"] privilege: elevated
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_IN_USE` | Another device is mounted at the mount point. | Unmount it first. |
| `E_UNSUPPORTED` | `persistent` on macOS. | Omit `persistent`. |
| `E_PERMISSION` | The account lacks the rights of the administrator. | Add `privilege: elevated`. |
| `E_EXTERNAL_MISSING` | The system program of the command is not installed. | Install the program named in the message. |
| `E_STAGE_FAILED` | The system program ended with an error. | Read the end of the error output in the message. |

## Related

`unmount`, `disks`, `disks.format`.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
