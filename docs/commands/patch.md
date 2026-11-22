---
command: patch
group: text
summary: Apply a unified or git diff to files under a root directory.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# patch

## Synopsis

```text
patch diff root: path [reverse: bool] [fuzz: int] [strip: int] [validate: off|warn|reject] [if_match: text|map<text>]
patch from: path root: path [reverse: bool] [fuzz: int] [strip: int] [validate: off|warn|reject] [if_match: text|map<text>]
```

## Description

`patch` applies a diff in unified format, as written by `diff -u`, or in git
format, as written by `git diff`, to the files below `root`. The diff is
given as text in `diff` or read from the file `from`; exactly one of them is
required. Text outside the file sections, such as a commit message, is
skipped.

The whole diff is applied in memory before anything changes. A hunk is
placed at the line of its header, shifted by the offset of the previous
hunk; when the old lines are not there, they are searched for at increasing
distances. With `fuzz: n`, up to `n` context lines at each end of a hunk may
differ. A hunk that cannot be placed fails the whole command with
`E_PATCH_REJECTED`, and no file changes. Later file sections see the result
of earlier ones, and each file is written once with its final content, in
one operation that `undo` reverts as a whole.

Paths in the diff are relative to `root`; `strip: n` removes `n` leading
components, as `patch -p n` (git diffs need `strip: 1` for their `a/` and
`b/` prefixes; `rename` and `copy` lines lose one component less). Absolute
paths and paths with `..` are rejected.

A changed file keeps its encoding, byte order mark and line endings: a diff
with `lf` line endings applies to a file with `crlf` line endings, and added
lines take the line ending of the file. New files are written in UTF-8 with
the line endings of the diff, and missing directories are created.

Git headers create (`new file mode`), delete (`deleted file mode`), rename
(`rename from`, `rename to`) and copy (`copy from`, `copy to`) files. A
rename without a change of content renames the file; a rename with changes
writes the new file and deletes the old one. A file section without git
headers whose old file is missing and whose hunks only add lines creates the
file. As with GNU `patch`, a file that such a section empties is deleted when
the time stamp of its new side is the Unix epoch, which `diff -N` writes for
a missing file; otherwise the empty file is kept and reported as
`file_emptied`.

A git mode change (`old mode`, `new mode`) is applied to a file whose content
does not change. For a file whose content changes, and for a new file with
execute bits, the mode is not applied and the change carries the flag
`mode_not_applied`.

`reverse: true` undoes the changes the diff describes: added lines are
removed, created files are deleted and renames are reverted.

The text of every created and changed file of a known format is checked
after the patch is applied: a document format of the documents group,
otherwise a language of the code group. Only syntax errors that the previous
content did not have count; a file created by a rename is compared with the
file it is renamed from. With `validate: reject` such an error in any file
rolls the whole patch back with `E_ROLLED_BACK`; with `warn` it is reported
as a warning and the patch is kept; `off` skips the check. Deleted files and
mode changes are not checked, and without the documents and code groups
nothing is checked.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `root` | `path` | | Directory that the paths in the diff are relative to; required. |
| `diff` | `text` | | Text of the diff; position 1. Exclusive with `from`. |
| `from` | `path` | | File that contains the diff. Exclusive with `diff`. |
| `reverse` | `bool` | `false` | Apply the diff in reverse. |
| `fuzz` | `int` | `0` | Number of context lines at each end of a hunk that may differ when the hunk is placed. |
| `strip` | `int` | `0` | Number of leading path components removed from the paths in the diff. |
| `validate` | `enum(off\|warn\|reject)` | `reject` | Check created and changed text of a known format: `reject` rolls back a patch that adds a syntax error, `warn` reports it, `off` skips the check. |
| `if_match` | `text \| map<text>` | | Expected etag of the patched file, or a map from path to etag for several files. A file with another etag fails with `E_STALE` and nothing changes. |

## Input and output

| Form | Input | Output |
|---|---|---|
| source | none | `effect` |

The `effect` record has one `change` per target in the order of execution:
created directories, renames, then the other files in path order. A change
of file content has `added`, `removed` and `diff` (the `hunk` records of the
change, with three lines of context); a changed file also has
`encoding_before`, `encoding_after`, `line_ending_before`,
`line_ending_after`, `bom_before` and `bom_after`. The flags `hunk_offset`
and `hunk_fuzz` mark a file whose hunks were placed at another line or with
ignored context lines.

## Effect

Category `change`, effect `local`, reversibility `reversible`. Every target
is declared to the journal before the first change, and the journal checks
atomically that each file still has the etag seen by the plan; a file changed
in between fails with `E_STALE` and the operation is rolled back. A preview
shows the same changes as the execution, with their differences.

Notable entries: `unchanged` for a file the diff leaves as it is,
`file_emptied` for a file left empty, `encoding_preserved` and
`line_ending_preserved` for files written in an encoding other than UTF-8 or
with line endings other than `lf`.

## Examples

Apply a fix written by `git diff` to a project with `crlf` line endings.

```nurt example
patch from: "/srv/patches/fix-timeout.diff" root: "/srv/app" strip: 1
```

```text expected
op 41  patch  ok
targets 1 file
  ~ /srv/app/src/config.rs  +1 -1
```

Apply a git diff that creates, deletes and renames files.

```nurt example
patch from: "/srv/patches/layout.diff" root: "/srv/app" strip: 1
```

```text expected
op 42  patch  ok
targets 5
  + /srv/app/docs/
  + /srv/app/src/support/
  > /srv/app/src/util.rs -> /srv/app/src/support/util.rs
  - /srv/app/INSTALL.txt
  + /srv/app/docs/setup.md  +2
```

Undo the first example.

```nurt example
patch from: "/srv/patches/fix-timeout.diff" root: "/srv/app" strip: 1 reverse: true
```

A diff that no longer matches the file changes nothing.

```nurt example
patch from: "/srv/patches/fix-timeout.diff" root: "/srv/app" strip: 1
```

```text expected
error E_PATCH_REJECTED  stage 1  /srv/app/src/config.rs:1
  hunk 1 does not apply to src/config.rs at line 1; check the file version or raise fuzz
```

A diff that removes the closing brace of `package.json` is rolled back
together with its change of `README.md`.

```nurt example
patch from: "/srv/patches/bump.diff" root: "/srv/app" strip: 1
```

```text expected
error E_ROLLED_BACK  stage 1
  the change adds 1 syntax error to `/srv/app/package.json` and was rolled back: correct the new text or pass `validate: warn`
```

The same diff with `validate: warn` is applied, and the new error is
reported as a warning.

```nurt example
patch from: "/srv/patches/bump.diff" root: "/srv/app" strip: 1 validate: warn
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_PATCH_REJECTED` | A hunk does not match its file (`problem: hunk_rejected`, with the expected and actual lines and the location), or a file to delete has other content (`problem: delete_mismatch`). | Check the version of the file, adjust `strip`, or raise `fuzz`. |
| `E_FORMAT` | The diff is malformed, contains no file section, is a binary diff, names no file, has fewer path components than `strip`, or has an absolute path or a path with `..` (`problem` names the case). | Provide a text diff in unified or git format with paths relative to `root`. |
| `E_NOT_FOUND` | A file to change does not exist, or `root` or `from` does not exist. | Check `root` and `strip`. |
| `E_EXISTS` | A file to create already exists. | Remove the file, or apply the diff in reverse first. |
| `E_STALE` | A file does not have the etag of `if_match`, or changed after the plan. | Read the file again and repeat the call. |
| `E_MISSING_PARAM` | Neither `diff` nor `from`, or no `root`. | Give the diff and the root directory. |
| `E_PARAM_CONFLICT` | Both `diff` and `from`. | Give one of them. |
| `E_INVALID_VALUE` | `strip` or `fuzz` is negative, `root` is not a directory (`problem: not_a_directory`), or a path of the diff is a directory or a link (`problem: not_a_file`). | Correct the value. |
| `E_UNKNOWN_VALUE` | `validate` is not `off`, `warn` or `reject`. | Use one of the listed values. |
| `E_ROLLED_BACK` | The patch adds a syntax error to a file and `validate` is `reject` (`problem: validate_failed`, one `sample` line per new error). | Correct the diff or pass `validate: warn`. |
| `E_BINARY` | A file to change has binary content. | Patch text files only. |
| `E_ENCODING` | A file is not valid in its detected encoding, or an added character cannot be represented in it. | Convert the file with `convert`. |
| `E_LIMIT` | A file is larger than the limit of text files. | Patch smaller files. |
| `E_PERMISSION`, `E_IO` | A file cannot be read or written. | Check the permissions. |

## Related

`diff` returns the differences of files as `hunk` records. `replace`,
`insert` and `edit` change files by content instead of by line numbers.
`undo` reverts an applied patch.
