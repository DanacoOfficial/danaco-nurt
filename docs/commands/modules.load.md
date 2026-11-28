---
command: modules.load
group: administration
summary: Load a kernel module.
since: 0.1.1
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/commands/modules.load.md -->
<!-- Responsibility:   Documents modules.load. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# modules.load

## Synopsis

```text
modules.load <name> [name: value]...
```

## Description

`modules.load` loads a kernel module with `modprobe` on Linux; `params` are passed as `name=value`. A module that is already loaded is `unchanged`. `lsmod` decides; `-` and `_` are the same character in the name of a module.

Rights: the command accepts `privilege`; write `privilege: elevated` when the account that runs Nurt is not the administrator. A program that fails for lack of rights ends the command with `E_PERMISSION`, and `privilege: auto` repeats it once elevated. An elevated entry is undone by the same elevated process.

`undo` runs `modules.unload`.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `name` | `text` | required | Module name. |
| `params` | `list<text>` | none | Module parameters as name=value. |

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
| Reversibility conditions | Undo unloads the module. |
| Operating systems | `linux` |
| External use | `system_program`: `modprobe`, `lsmod`; Modules are listed with lsmod and loaded with modprobe. |

## Examples

Load a module.

```nurt example
modules.load "br_netfilter" privilege: elevated
```

Load a module with a parameter.

```nurt example
modules.load "nf_conntrack" params: ["hashsize=65536"] privilege: elevated
```

Run the load with elevated rights; adding the call parameter `preview: true` shows the effect first.

```nurt example
modules.load "overlay" privilege: elevated
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_OS_ONLY` | The system is not Linux. | Use the tools of the system. |
| `E_STAGE_FAILED` | The module does not exist. | Check the name. |
| `E_PERMISSION` | The account lacks the rights of the administrator. | Add `privilege: elevated`. |
| `E_EXTERNAL_MISSING` | The system program of the command is not installed. | Install the program named in the message. |
| `E_STAGE_FAILED` | The system program ended with an error. | Read the end of the error output in the message. |

## Related

`modules.unload`, `sysctl.set`.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
