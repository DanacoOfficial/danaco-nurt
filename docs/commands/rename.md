---
command: rename
group: core
summary: Rename entries by pattern or to a new name.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# rename

## Synopsis

```text
rename [<pattern>] [<with>] path: <path> (with: <text> | to: <text>) [name: value]...
entry |> rename [<pattern>] [<with>] (with: <text> | to: <text>) [name: value]...
```

## Description

`rename` changes the name of entries within their directory; `move` changes
the directory. The new name is given by `to`, evaluated per input record, or
derived from the old name: a text `pattern` replaces every occurrence of the
text with `with`, a regex `pattern` replaces every match with the template
`with`, in which `$1`, `${name}` and `$$` stand for a numbered group, a named
group and a dollar sign. `to` and `with` exclude each other.

The new name is a single name: a name with `/`, `\`, an empty name, `.` and
`..` are rejected. An entry whose new name equals its old name is counted as
`unchanged`.

`on_conflict` decides what happens when an entry with the new name exists:
`error` fails before anything changes, `skip` keeps both entries, `overwrite`
replaces an existing file or link. An existing directory is never replaced.
When the new name is the old name of another entry renamed in the same call,
that entry is renamed first; renames that form a cycle are rejected.

On a volume that does not distinguish letter case, a rename that changes only
the letter case renames the entry itself, and a new name that differs from
another existing name only in letter case is a conflict reported as
`case_collision`.

`check_references: true` reports whole-word occurrences of the old names in
other files as `references_left`, with the same rules as `remove`.

An entry record with `etag` must still have that entity tag; a changed entry
fails with `E_STALE` before anything changes. When nothing is renamed, the
result reports `nothing_matched`.

| Notable code | Meaning |
|---|---|
| `case_collision` | the new name differs from an existing name only in letter case on a volume that does not distinguish case |
| `references_left` | other files still name a renamed entry by its old name |
| `nothing_matched` | nothing was renamed |

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `path` | `path` | required | Entry to rename in the source form. |
| `pattern` | `text \| regex` | none | Part of the name to replace; used with with. |
| `with` | `text` | one of `replacement` | Replacement of the pattern in the name. |
| `to` | `text` | one of `replacement` | New name. |
| `on_conflict` | `enum(overwrite\|skip\|error)` | `error` | Behaviour when an entry with the new name exists. |
| `check_references` | `bool` | `false` | Report references to the old names left after the change. |

## Input and output

| Form | Input | Output |
|---|---|---|
| `source` | `none` | `effect` |
| `stage` | `entry` | `effect` |

The `effect` record has one `change` with action `rename` per renamed entry,
with `path` set to the old path and `to` to the new path, in the order the
renames run. Its `summary` counts `renamed`, `overwritten`, `skipped` and
`unchanged`.

## Effect

| Property | Value |
|---|---|
| Category | `change` |
| Effect | `local` |
| Reversibility | `reversible` |
| Implementation | `builtin` |

## Examples

Rename a file:

```nurt example
rename path: "/home/user/draft.md" to: "final.md"
```

```text expected
rename: 1 target, renamed 1
```

Replace an extension in every listed name:

```nurt example
find "/home/user/photos" glob: "*.jpeg" |> rename ".jpeg" ".jpg"
```

Rewrite dates in names with a regex template:

```nurt example
list "/home/user/logs" |> rename /(\d{4})-(\d{2})-(\d{2})/ "$3.$2.$1"
```

Change only the letter case of a name:

```nurt example
rename path: "/Volumes/data/readme.md" to: "README.md"
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_MISSING_PARAM` | no new name: neither `to` nor `pattern` with `with` | give `to`, or `pattern` and `with` |
| `E_PARAM_CONFLICT` | both `to` and `with`, or `pattern` with `to` | give one form of the new name |
| `E_INVALID_VALUE` | the new name contains a directory, is empty, `.` or `..`, or `pattern` is empty | use `move` to change the directory |
| `E_EXISTS` | an entry with the new name exists with `on_conflict: error`, is a directory, or the renames form a cycle | choose another name or `on_conflict` value |
| `E_REGEX` | the regex pattern is invalid, or the template names a group the pattern does not have | correct the pattern or the template |
| `E_NOT_FOUND` | the entry does not exist | check the path |
| `E_STALE` | an input entry changed since it was read | read the entries again |
| `E_IN_USE` | Windows: an entry is open in another process; the error lists the processes | close the entry there |

## Related

`move`, `copy`, `remove`, `replace`, `list`, `find`.
