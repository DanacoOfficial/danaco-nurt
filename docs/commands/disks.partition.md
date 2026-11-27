---
command: disks.partition
group: administration
summary: Create, delete or resize a partition.
since: 0.1.1
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# disks.partition

## Synopsis

```text
disks.partition <device> action: <enum(create|delete|resize)> [name: value]...
```

## Description

`disks.partition` creates, deletes or resizes a partition with `parted` on Linux, `diskutil` on macOS and the storage cmdlets on Windows. The change is irreversible. The preview and the report list the device, its partitions with file systems, labels and mount points, and the size of the data that the change destroys; a partition that is mounted is refused with `E_IN_USE`. `device` is the disk (`/dev/sdb`, or the number of the disk on Windows); `number` selects the partition to delete or resize; a new partition starts at `start` (1 MiB by default) and takes `size` or the rest of the disk.

Rights: the command accepts `privilege`; write `privilege: elevated` when the account that runs Nurt is not the administrator. A program that fails for lack of rights ends the command with `E_PERMISSION`, and `privilege: auto` repeats it once elevated. An elevated entry is undone by the same elevated process.

A partition change is irreversible: the preview is the only safeguard.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `device` | `text` | required | Disk device such as /dev/sdb, or the number of the disk on Windows. |
| `action` | `enum(create\|delete\|resize)` | required | Change to make. |
| `number` | `int` | none | Number of the partition to delete or resize. |
| `start` | `size` | none | Start of a new partition; 1 MiB when omitted. |
| `size` | `size` | none | Size of a new or resized partition; the rest of the disk when omitted for create. |
| `fs_type` | `text` | none | File system type hint of a new partition. |

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
| External use | `system_program`: `parted`, `lsblk`, `diskutil`, `powershell`; Partitions are changed with parted on Linux, diskutil on macOS and the storage cmdlets on Windows. |

## Examples

Run the deletion of a partition with elevated rights; adding the call parameter `preview: true` shows the effect first.

```nurt example
disks.partition "/dev/sdb" action: delete number: 2 privilege: elevated
```

Create a partition of 10 GiB.

```nurt example
disks.partition "/dev/sdb" action: create size: 10gib privilege: elevated
```

Resize a partition.

```nurt example
disks.partition "/dev/sdb" action: resize number: 1 size: 20gib privilege: elevated
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_IN_USE` | The partition is mounted. | Unmount it first. |
| `E_NOT_FOUND` | The partition does not exist. | Check the number with `disks`. |
| `E_PERMISSION` | The account lacks the rights of the administrator. | Add `privilege: elevated`. |
| `E_EXTERNAL_MISSING` | The system program of the command is not installed. | Install the program named in the message. |
| `E_STAGE_FAILED` | The system program ended with an error. | Read the end of the error output in the message. |

## Related

`disks`, `disks.format`, `unmount`.
