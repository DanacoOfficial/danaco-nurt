---
command: create
group: core
summary: Create files, directories, links, junctions, named pipes, device nodes or temporary entries.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# create

## Synopsis

```text
create <path>... type: <enum(file|directory|link|junction|fifo|device)> [name: value]...
```

## Description

`create` creates one entry per path: an empty file, a directory, a symbolic
or hard link, a junction, a named pipe or a device node. `type` has no default. Missing parent
directories are created unless `parents` is `false`; each one is a target of
its own, and a directory named twice, as a path and as a parent, is created
once.

An existing entry of the requested kind is left as it is with `on_exists:
skip` and reported as `unchanged`, fails with `E_EXISTS` with `on_exists:
error`, and gets the current time as its access and modification time with
`on_exists: touch`. An existing entry of another kind always fails with
`E_EXISTS`.

A symbolic link stores `target` as written; a relative target is relative to
the directory of the link, and the target need not exist. `hard: true`
creates a hard link to the existing file `target`. A junction needs an
existing directory as its target and exists only on Windows.

With `temporary: true`, every `path` is a directory, and `create` makes one
file or directory with a unique name that starts with `nurt-` in each; without
`path`, in the temporary directory of the host. The `change` records of the
result hold the new paths.

`type: fifo` creates a named pipe (FIFO) with the permission bits of `mode`,
written in octal, for example `mode: "0660"` (without `mode` the default `0666` is reduced by the
umask; with `mode` the bits are set exactly, without following a link). `type: device` creates a device node and needs `kind: char` or
`kind: block`, `major` and `minor`; `mode` applies as for a pipe. Both exist
on Linux and macOS only; on Windows they fail with `E_OS_ONLY`, and for a
pipe the message points to the named pipes of the system, which are objects
of the `\\.\pipe\` namespace created by the program that serves them. A
device node needs `privilege: elevated`; without it the system refuses with
`E_PRIVILEGE`. An exact `mode` is set through `/proc/self/fd` on Linux; without `/proc` the command fails and removes the new node. Both are journaled as created files, and `undo` removes them.
`info` and `list` report such an entry with the type `fifo` or `device`;
a device node also reports `major` and `minor`. `kind`, `major`, `minor`
and `mode` cannot be combined with other types, and `temporary` cannot be
combined with `fifo` or `device`.

On Windows a symbolic link needs the privilege to create symbolic links or
developer mode; without it the command fails with `E_PRIVILEGE`, and a
junction is the alternative for directories.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `type` | `enum(file\|directory\|link\|junction\|fifo\|device)` | required | Kind of entry; there is no default. fifo (named pipe) and device (device node) exist on Linux and macOS; a device needs privilege elevated. |
| `path` | `list<path>` | none | Paths to create; with temporary, the directory of the new entry. |
| `target` | `text` | none | Target of a link or junction. |
| `hard` | `bool` | `false` | Create a hard link instead of a symbolic link. |
| `temporary` | `bool` | `false` | Create an entry with a unique name in the temporary directory. |
| `parents` | `bool` | `true` | Create missing parent directories. |
| `on_exists` | `enum(skip\|error\|touch)` | `skip` | Behaviour when the entry exists; touch updates its modification time. |
| `kind` | `enum(char\|block)` | none | Kind of device node for type device. |
| `major` | `int` | none | Major device number for type device. |
| `minor` | `int` | none | Minor device number for type device. |
| `mode` | `text` | none | Permission bits of a fifo or device node in octal, for example "0660"; the default is 0666 reduced by the umask. |

## Input and output

| Form | Input | Output |
|---|---|---|
| default | `none` | `effect` |

## Effect

| Property | Value |
|---|---|
| Category | `change` |
| Effect | `local` |
| Reversibility | `reversible` |
| Implementation | `builtin` |

## Examples

Create a directory and its missing parent:

```nurt example
create "/home/user/app/build/cache" type: directory
```

Update the modification time of an existing file:

```nurt example
create "/home/user/notes.txt" type: file on_exists: touch
```

```json fs-before
{ "format": "nurt-fs-snapshot/1", "entries": [
  { "path": "/home/user/notes.txt", "type": "file", "content_utf8": "keep\n" }
] }
```

Point a link at a release directory:

```nurt example
create "/home/user/app/current" type: link target: "releases/v2"
```

Create an empty file with a unique name:

```nurt example
create "/home/user/tmp" type: file temporary: true
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_MISSING_PARAM` | `type` is missing, `path` is missing without `temporary`, a link has no `target`, or a device lacks `kind`, `major` or `minor` | give the missing parameter |
| `E_EXISTS` | the entry exists with `on_exists: error`, or exists as another kind | choose another path or `on_exists: skip` |
| `E_PARAM_CONFLICT` | `target` or `hard` with a type other than `link`, `kind`, `major` or `minor` with a type other than `device`, `mode` with a type other than `fifo` or `device`, `temporary` with a link, a fifo or a device, or a path given twice | remove the parameter |
| `E_INVALID_VALUE` | `major` or `minor` exceeds the device number of the system (Linux: major up to 4095, minor up to 1048575; macOS: 255 and 16777215), or `mode` is not octal permission bits | give a valid value |
| `E_NOT_FOUND` | a parent directory is missing and `parents` is `false`, or the target of a hard link or junction is missing | set `parents: true` or create the target |
| `E_OS_ONLY` | `type: junction` outside Windows, `type: fifo` or `type: device` on Windows | use `type: link`; on Windows create the named pipe in the program that serves it |
| `E_PRIVILEGE` | Windows refuses to create a symbolic link, or a device node is created without `privilege: elevated` | enable developer mode, use a junction, or add `privilege: elevated` |

## Related

`write`, `copy`, `move`, `remove`, `attributes`, `list`.
