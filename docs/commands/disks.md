---
command: disks
group: system
summary: List mounted volumes with capacity and free space.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/commands/disks.md -->
<!-- Responsibility:   Documents disks. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# disks

## Synopsis

```text
disks [name: value]...
```

## Description

`disks` lists the mounted volumes of the host as `disk` records, ordered by
`mount`. Pseudo file systems of the kernel, such as `proc`, `sysfs`,
`cgroup2`, `devpts` and `autofs`, are not volumes and are left out. When
several file systems are mounted on the same point, only the visible one is
listed.

`type` selects volumes by kind. The kind of a volume follows the rules of the
journal space:

| `type` | Volumes |
|---|---|
| `network` | `nfs`, `nfs4`, `cifs`, `smb3`, `smbfs`, `sshfs`, `fuse.sshfs`, `9p`, `afs`, `afpfs`, `webdav`; on macOS every volume without `MNT_LOCAL`; on Windows `DRIVE_REMOTE` |
| `ram` | `tmpfs`, `ramfs`, `devtmpfs`; on Windows `DRIVE_RAMDISK` |
| `removable` | removable media and, on macOS, external devices |
| `fixed` | every other local volume, including the `overlay` file system of containers |
| `other` | file systems in user space other than the listed network file systems |

`ram` and `other` volumes appear only with `type: all`.

`total` is the capacity, `free` the space available to the account, and
`used` the capacity minus all free blocks, so `used` and `free` do not add up
to `total` on file systems that reserve blocks. `inodes_total` and
`inodes_free` are set on Linux and macOS for file systems that report them.
`options` holds the mount options on Linux.

| System | Volumes | Space | Removable media | Label |
|---|---|---|---|---|
| Linux | `/proc/self/mountinfo` | `statvfs` | `removable` attribute of the block device | `/dev/disk/by-label` |
| macOS | `getmntinfo` | `statfs` | volume resource keys | volume name |
| Windows | drive letters | `GetDiskFreeSpaceExW` | `GetDriveTypeW` | `GetVolumeInformationW` |

On Windows `mount` is the drive root in canonical form, for example `C:/`. A
volume whose space cannot be read, for example an unreachable network share,
is left out; the report carries one `source_unavailable` note with the number
of such volumes.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `type` | `enum(all\|fixed\|removable\|network)` | `all` | Kind of volume. |

## Input and output

| Form | Input | Output |
|---|---|---|
| default | `none` | `disk` |

## Effect

| Property | Value |
|---|---|
| Category | `source` |
| Effect | `read` |
| Reversibility | `read` |
| Implementation | `builtin` |

## Examples

List the network volumes:

```nurt example
disks type: network
```

List the removable media:

```nurt example
disks type: removable
```

List all mounted volumes:

```nurt example
disks
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_INVALID_VALUE` | `type` has a value outside the enumeration. | Use one of the listed values. |
| `E_IO` | The mount table cannot be read. | Check that `/proc` is mounted. |

## Related

`journal.space`, `host`, `list`.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
