---
command: unmount
group: administration
summary: Unmount a mount point.
since: 0.1.1
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# unmount

## Synopsis

```text
unmount <target> [name: value]...
```

## Description

`unmount` unmounts a mount point with `umount` on Linux and macOS and `mountvol /D` on Windows. The device, the type and the options are read first. With `persistent: true` on Linux the line of the mount point is removed from `/etc/fstab` through the journal.

Rights: the command accepts `privilege`; write `privilege: elevated` when the account that runs Nurt is not the administrator. A program that fails for lack of rights ends the command with `E_PERMISSION`, and `privilege: auto` repeats it once elevated. An elevated entry is undone by the same elevated process.

`undo` runs `mount` with the recorded device, type and options.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `target` | `text` | required | Mount point. |
| `persistent` | `bool` | `false` | Also remove the mount from /etc/fstab (Linux); the file is journaled. |

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
| Reversibility conditions | Undo mounts the recorded device with its type and options. |
| External use | `system_program`: `umount`, `mountvol`; Unmounts use umount on Linux and macOS and mountvol on Windows. |

## Examples

Unmount a directory.

```nurt example
unmount "/srv/data" privilege: elevated
```

Unmount and remove the line of `/etc/fstab`.

```nurt example
unmount "/srv/data" persistent: true privilege: elevated
```

Run the unmount with elevated rights; adding the call parameter `preview: true` shows the effect first.

```nurt example
unmount "/srv/data" privilege: elevated
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_NOT_FOUND` | Nothing is mounted at the mount point. | Check the mount point with `disks`. |
| `E_IN_USE` | A process uses the mount. | Stop the processes listed by `open_files`. |
| `E_PERMISSION` | The account lacks the rights of the administrator. | Add `privilege: elevated`. |
| `E_EXTERNAL_MISSING` | The system program of the command is not installed. | Install the program named in the message. |
| `E_STAGE_FAILED` | The system program ended with an error. | Read the end of the error output in the message. |

## Related

`mount`, `disks`.
