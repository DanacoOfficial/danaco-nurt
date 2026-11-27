---
command: modules.unload
group: admin
summary: Unload a kernel module.
since: 0.1.1
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# modules.unload

## Synopsis

```text
modules.unload <name>
```

## Description

`modules.unload` unloads a kernel module with `modprobe -r` on Linux. A module that is not loaded is `unchanged`.

Rights: the command accepts `privilege`; write `privilege: elevated` when the account that runs Nurt is not the administrator. A program that fails for lack of rights ends the command with `E_PERMISSION`, and `privilege: auto` repeats it once elevated. An elevated entry is undone by the same elevated process.

`undo` runs `modules.load`.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `name` | `text` | required | Module name. |

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
| Reversibility conditions | Undo loads the module. |
| Operating systems | `linux` |
| External use | `system_program`: `modprobe`, `lsmod`; Modules are listed with lsmod and unloaded with modprobe -r. |

## Examples

Unload a module.

```nurt example
modules.unload "dummy" privilege: elevated
```

Unload and show the effect.

```nurt example
modules.unload "dummy" privilege: elevated |> select target
```

Preview the unload.

```nurt example
modules.unload "dummy" preview
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_OS_ONLY` | The system is not Linux. | Use the tools of the system. |
| `E_STAGE_FAILED` | The module is in use. | Stop its users first. |
| `E_PERMISSION` | The account lacks the rights of the administrator. | Add `privilege: elevated`. |
| `E_EXTERNAL_MISSING` | The system program of the command is not installed. | Install the program named in the message. |
| `E_STAGE_FAILED` | The system program ended with an error. | Read the end of the error output in the message. |

## Related

`modules.load`.
