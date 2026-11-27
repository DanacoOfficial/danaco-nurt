---
command: sysctl.set
group: administration
summary: Set a kernel parameter.
since: 0.1.1
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/commands/sysctl.set.md -->
<!-- Responsibility:   Documents sysctl.set. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# sysctl.set

## Synopsis

```text
sysctl.set <key> <value> [name: value]...
```

## Description

`sysctl.set` sets a kernel parameter with `sysctl -w` on Linux and macOS. The current value is read with `sysctl -n` and recorded; a parameter that already has the value is `unchanged`. With `persistent: true` on Linux the line `key = value` is written to `/etc/sysctl.d/90-nurt.conf` through the journal.

Rights: the command accepts `privilege`; write `privilege: elevated` when the account that runs Nurt is not the administrator. A program that fails for lack of rights ends the command with `E_PERMISSION`, and `privilege: auto` repeats it once elevated. An elevated entry is undone by the same elevated process.

`undo` runs `sysctl.set` with the recorded value; the journal restores the file.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `key` | `text` | required | Name of the parameter, for example net.ipv4.ip_forward. |
| `value` | `text \| int` | required | New value. |
| `persistent` | `bool` | `false` | Also write the value to /etc/sysctl.d/90-nurt.conf (Linux); the file is journaled. |

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
| Reversibility conditions | Undo sets the recorded value; the file is restored by the journal. |
| Operating systems | `linux`, `macos` |
| External use | `system_program`: `sysctl`; Parameters are read and set with sysctl. |

## Examples

Enable forwarding of packets.

```nurt example
sysctl.set "net.ipv4.ip_forward" 1 privilege: elevated
```

Set a value and keep it after a restart.

```nurt example
sysctl.set "vm.swappiness" 10 persistent: true privilege: elevated
```

Run the change with elevated rights; adding the call parameter `preview: true` shows the effect first.

```nurt example
sysctl.set "vm.swappiness" 10 privilege: elevated
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_NOT_FOUND` | The kernel has no such parameter. | Check the name with `sysctl -a`. |
| `E_UNSUPPORTED` | `persistent` outside Linux. | Omit `persistent`. |
| `E_PERMISSION` | The account lacks the rights of the administrator. | Add `privilege: elevated`. |
| `E_EXTERNAL_MISSING` | The system program of the command is not installed. | Install the program named in the message. |
| `E_STAGE_FAILED` | The system program ended with an error. | Read the end of the error output in the message. |

## Related

`modules.load`, `modules.unload`.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
