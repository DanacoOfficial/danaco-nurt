---
command: attributes
group: core
summary: Change permissions, account, times and extended attributes of entries.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/commands/attributes.md -->
<!-- Responsibility:   Documents attributes. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# attributes

## Synopsis

```text
attributes <path>... [name: value]...
entry |> attributes [name: value]...
```

## Description

`attributes` changes the metadata of the given entries, or of the input
entries in the stage form. At least one change is required. Every target must
exist; a missing target fails the call before anything is changed.

The plan step reads every target and computes its change from the current
state. A target whose metadata already has the requested values is left out
and reported as `unchanged`. In the stage form an input entry carrying an
`etag` is checked against the current state; a changed entry fails with
`E_STALE`.

A link given as a target is resolved: the change applies to the entry the
link points to, and a link to a missing entry fails with `E_NOT_FOUND`. With
`recursive: true` the entries below a directory are changed as well; links
below it are neither followed nor changed.

With an empty input the call changes nothing and reports `nothing_matched`.

| Parameter | Linux | macOS | Windows |
|---|---|---|---|
| `mode` | permission bits | permission bits | only the read-only attribute, from the write permission of the account; the change has the flag `acl_unchanged` |
| `owner`, `group` | account and group ids | account and group ids | `E_UNSUPPORTED` |
| `acl` | POSIX access ACL | `E_UNSUPPORTED` | `E_UNSUPPORTED` |
| `readonly` | `true` removes every write permission, `false` adds the write permission of the account | as Linux | read-only attribute |
| `hidden` | follows the name; a different value gives the flag `hidden_unsupported` | as Linux | hidden attribute |
| `created` | not settable; the flag `created_unsupported` | birth time | creation time |
| `xattrs` | extended attributes | extended attributes | `E_UNSUPPORTED` |

A flag names a requested element that the system could not apply. It appears
in the `flags` field of the change, or in the detail of the `unchanged`
entry when nothing else changed.

Changing the account of a file needs administrator rights; a call by another
account fails with `E_PRIVILEGE` before anything is changed.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `path` | `list<path>` | required | Entries to change in the source form. |
| `mode` | `text` | none | Permissions, such as "0644" or "u+x". |
| `owner` | `text` | none | Account of the file. |
| `group` | `text` | none | Group of the file. |
| `acl` | `list<map<any>>` | none | Access control entries. |
| `readonly` | `bool` | none | Read-only attribute. |
| `hidden` | `bool` | none | Hidden attribute. |
| `modified` | `instant` | none | Modification time. |
| `accessed` | `instant` | none | Access time. |
| `created` | `instant` | none | Creation time. |
| `xattrs` | `map<text>` | none | Extended attributes to set. |
| `recursive` | `bool` | `false` | Apply to the content of directories. |

## Input and output

| Form | Input | Output |
|---|---|---|
| `source` | `none` | `effect` |
| `stage` | `entry` | `effect` |

## Effect

| Property | Value |
|---|---|
| Category | `change` |
| Effect | `local` |
| Reversibility | `reversible` |
| Implementation | `builtin` |

## Examples

Make a script executable for the account of the file:

```nurt example
attributes "/home/user/run.sh" mode: "u+x"
```

```json fs-before
{ "format": "nurt-fs-snapshot/1", "entries": [
  { "path": "/home/user/run.sh", "type": "file", "mode": "0644", "content_utf8": "content\n" }
] }
```

Restrict a file to its account and set its modification time:

```nurt example
attributes "/home/user/notes.txt" mode: "0600" modified: "2026-01-31T12:00:00Z"
```

```json fs-before
{ "format": "nurt-fs-snapshot/1", "entries": [
  { "path": "/home/user/notes.txt", "type": "file", "content_utf8": "content\n" }
] }
```

Remove the write permission of the group and others in a whole tree:

```nurt example
attributes "/home/user/site" mode: "go-w" recursive: true
```

```json fs-before
{ "format": "nurt-fs-snapshot/1", "entries": [
  { "path": "/home/user/site", "type": "dir", "mode": "0777" },
  { "path": "/home/user/site/index.html", "type": "file", "mode": "0777", "content_utf8": "content\n" },
  { "path": "/home/user/site/css/main.css", "type": "file", "content_utf8": "content\n" }
] }
```

Make the files found by `find` read-only:

```nurt example
find "/home/user/site" type: file |> attributes readonly: true
```

```json fs-before
{ "format": "nurt-fs-snapshot/1", "entries": [
  { "path": "/home/user/site/index.html", "type": "file", "content_utf8": "content\n" },
  { "path": "/home/user/site/css/main.css", "type": "file", "content_utf8": "content\n" }
] }
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_MISSING_PARAM` | no change was requested | give at least one of `mode`, `owner`, `group`, `acl`, `readonly`, `hidden`, `modified`, `accessed`, `created`, `xattrs` |
| `E_INVALID_VALUE` | a mode, time, account, group or ACL entry is not valid | use the forms in the parameter table |
| `E_TYPE` | a parameter has a value of another type | pass texts to `xattrs` and maps to `acl` |
| `E_NOT_FOUND` | a target or the entry a link points to does not exist | check the path |
| `E_STALE` | an input entry changed since it was read | read the entries again |
| `E_PRIVILEGE` | changing the account of a file needs administrator rights | run as an administrator or leave `owner` out |
| `E_PERMISSION` | the system refused the change | check the account of the file and the permissions of its directory |
| `E_UNSUPPORTED` | the system or volume has no form for a requested element | see the table in Notes |

## Related

`info`, `create`, `copy`, `undo`.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
