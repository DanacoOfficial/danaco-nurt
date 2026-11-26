---
command: archive.extract
group: archive
summary: Extract an archive into a directory, skipping entries that resolve outside it.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# archive.extract

## Synopsis

```text
archive.extract <path> into: <path> [name: value]...
```

## Description

`archive.extract` extracts the members of the archive `path` into the
directory `into`, which is created with its missing parents when it does not
exist. The format is recognised from the content of the file, as by
`archive.list`.

Before anything is written, every member path and link target is checked. A
member is not extracted, and is reported with the notable entry
`archive_path_escape`, when its path:

- contains a `..` component, or a component of only dots and spaces;
- is absolute, starts with a drive letter or contains `\`;
- passes through a symbolic link created by the same archive, or through an
  existing symbolic link inside `into`;
- is a symbolic link whose target is absolute or leads outside `into`;
- is a hard link to a file that is not extracted from the same archive.

With `allow_outside: true` these members are extracted at the path stored in
the archive, relative to `into`, and still reported. A member whose name is
empty, contains NUL or is not valid UTF-8 is never extracted. Devices, FIFOs
and other special files are never extracted and are reported with
`special_file`.

`only` selects members by their path in the archive, with the pattern rules
of `archive.list`. `strip_components` removes leading components from member
paths and hard link targets; a member left without components is skipped.
When several members have the same destination, the last one is extracted.

An existing regular file is replaced, kept or reported as an error according
to `on_conflict`. An existing directory receives the content of the archive.
An existing entry of another type than the member is kept with
`on_conflict: skip` and is an error otherwise; a symbolic link with the same
target as the member is left unchanged.

Regular files receive the permissions stored in the archive; the set-user-id
and set-group-id bits are kept only when `preserve_permissions: true` is given
explicitly and the process runs as `root`, and are dropped otherwise, so that
an archive extracted by an unprivileged process or without that option never
creates a privileged program. With `preserve_permissions: false` the sticky bit
and the write permission of the group and others are dropped as well. A replaced file keeps
its own permissions. Directories always grant the account full access, so
that their content can be written. Files receive the time they are written as
their modification time.

The extracted size and the ratio of the extracted size to the size of the
archive are limited (1 TiB, and a ratio of 1000 above 64 MiB, so that an archive
of 300 KiB that declares 300 MiB is refused with `E_LIMIT`); an archive that
declares more fails before anything is written.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `path` | `path` | required | Archive file. |
| `into` | `path` | required | Directory to extract into; created when missing. |
| `only` | `glob` | none | Only entries whose path matches this pattern. |
| `strip_components` | `int` | `0` | Number of leading path components removed from entry paths. |
| `on_conflict` | `enum(overwrite\|skip\|error)` | `overwrite` | Behaviour when a target file already exists. |
| `preserve_permissions` | `bool` | `true` | Apply the permissions stored in the archive. |
| `allow_outside` | `bool` | `false` | Extract entries whose resolved path lies outside the target directory. |

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

Notable entries: `archive_path_escape` for every rejected or, with
`allow_outside`, extracted escaping member, with the member as
`archive:<archive>!/<member>` and the reason (`parent_component`,
`absolute`, `drive_letter`, `backslash`, `invalid_name`, `link_outside`,
`through_link`, `hard_link_outside`); `special_file` for special files;
`overwrote_existing` with the number of replaced files.

## Examples

Extract a compressed tar archive into a new directory:

```nurt example
archive.extract "/srv/dl/data.tar.gz" into: "/srv/data"
```

```text expected
effect archive.extract  files: 2  directories: 2  bytes: 18
```

Extract the Markdown files of a release without its top directory:

```nurt example
archive.extract "/home/user/dl/project-1.2.zip" into: "/home/user/docs" only: "*.md" strip_components: 1
```

Extract a release over an installation and keep the files that exist there:

```nurt example
archive.extract "/srv/dl/release.tar" into: "/srv/app" on_conflict: skip
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_NOT_FOUND` | `path` does not exist | Check the path. |
| `E_FORMAT` | the file is not an archive of a supported format, or the archive is damaged | Check the file. |
| `E_ENCRYPTED` | a selected member is encrypted | Decrypt the archive with its own tool, or exclude the member with `only`. |
| `E_LIMIT` | the declared extracted size or compression ratio exceeds the limit | Check the archive; it may be a decompression bomb. |
| `E_EXISTS` | a target exists and `on_conflict` is `error`, an existing entry has another type than the member, or `into` is not a directory | Set `on_conflict`, remove the entry, or choose another `into`. |
| `E_STALE` | the archive or a target changed after the plan | Run the command again. |
| `E_INVALID_VALUE` | `strip_components` is negative or `only` is not a valid pattern | Correct the value. |
| `E_UNKNOWN_VALUE` | the value of `on_conflict` is unknown | Use one of the listed values. |
| `E_PATH_RELATIVE` | a path is relative and no base directory is set | Give an absolute path or set the `base` option. |

## Related

`archive.list` shows the members before extraction; `archive.create`
creates an archive; `undo` reverts an extraction.
