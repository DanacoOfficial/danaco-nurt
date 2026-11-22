---
command: info
group: core
summary: Describe paths, including paths that do not exist.
since: 0.1.0
---

# info

## Synopsis

```text
info <path>... [hash: none|sha256|sha512|sha1|md5|blake3|crc32] [details: [text|exif|pdf|media|git|symbols]] [known: <map<text>>]
```

## Description

`info` produces one `entry` record per path, in the order given. A path that
does not exist, or whose parent is not a directory, gives an entry with
`exists: false`, `size` zero and the Unix epoch as `modified`; it is not an
error.

An existing entry carries the path and name fields, `type`, `size`, the
times, `mode` in octal, `readonly`, `hidden`, `links`, `etag`, the account
(`owner`) and group of the file, `acl`, `xattrs`, `streams`, `volume` and
`file_id`. A link is described without being followed; `link_target` holds
the target as stored in the link and `real_path` the path after resolving
links, when the target exists.

A regular file, and a link that resolves to one, also carries `mime`,
`binary`, `encoding`, `line_ending` and `bom`, recognised from the first
8192 bytes of the content. The media type comes from the signature of the
content, then, for text, from the file name; otherwise it is `text/plain`
or `application/octet-stream`.

`lines`, `hash` and the `details` groups read more of the content and are
computed only when requested. Files larger than the text limit of the
execution get no line count and no `text`, `pdf` or `symbols` details.

An entry whose entity tag equals the one given for its path in `known` is
reported with the path, name, type, size, times, mode and `etag` only, and
an `unchanged` notable entry names the path.

With no path the result is empty.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `path` | `list<path>` | | Paths to describe; a missing path gives `exists: false`. Relative paths are resolved against the base directory. |
| `hash` | `enum(none\|sha256\|sha512\|sha1\|md5\|blake3\|crc32)` | `none` | Hash algorithm of the content. The `hash` field holds the algorithm name, `:` and lower-case hexadecimal digits. Directories get no hash. |
| `details` | `list<enum(text\|exif\|pdf\|media\|git\|symbols)>` | `[]` | Additional groups of metadata, in the `details` field by group name. |
| `known` | `map<text>` | | Map from path to etag; unchanged paths are reported without details. |

The groups of `details`:

| Group | Content |
|---|---|
| `text` | `encoding`, `line_ending`, `bom` and `lines` of the whole text; sets the `lines` field. For binary content: `binary: true`. |
| `exif` | EXIF tags of the primary image by tag name, with values in display form. Images without EXIF data have no group. |
| `pdf` | `version`, `pages`, and `title`, `author`, `subject`, `keywords`, `creator`, `producer` when the document has them. |
| `media` | `mime`, `width` and `height` of images. Audio and video files have no group. |
| `git` | `status` in the repository (`clean`, `modified`, `untracked`, `ignored`, `conflicted`, or `none` outside every repository) and `root` of the working tree. |
| `symbols` | `language`, `total` number of code symbols and their number by kind in `kinds`. |

## Input and output

| Form | Input | Output |
|---|---|---|
| source | none | `entry` |

## Effect

Category `source`, effect `read`, reversibility `read`. `info` reads the file
system only and starts no external program.

## Notes

`owner` and `group` hold the name and the numeric identifier, as in
`user (1000)`, on Linux and macOS; the names come from the account tables of
the system, and an account without an entry there is shown by its
identifier. On Windows `owner` holds the security identifier of the account
and `group` is absent.

`acl` lists the POSIX ACL in the short form of `getfacl` on Linux (for
example `user:alice:r--`) and the entries of the access control list on
macOS; on Windows it is absent. Values of extended attributes that are not
UTF-8 text are written as `hex:` followed by hexadecimal digits.

`created` is present where the system reports a creation time.

The groups `git` and `symbols` need the repository and code services of the
engine. Without the service the group is left out and a `source_unavailable`
notable entry names the missing service once per call; a file in a language
without a grammar has no `symbols` group.

## Examples

Describe one file:

```nurt example
info "/home/user/notes.txt"
```

```json fs-before
{ "format": "nurt-fs-snapshot/1", "entries": [
  { "path": "/home/user/notes.txt", "type": "file", "content_utf8": "line one\nline two\n" }
] }
```

Describe an existing and a missing path; the second entry has
`exists: false`:

```nurt example
info "/home/user/notes.txt" "/home/user/draft.txt"
```

```json fs-before
{ "format": "nurt-fs-snapshot/1", "entries": [
  { "path": "/home/user/notes.txt", "type": "file", "content_utf8": "line one\nline two\n" }
] }
```

Add the SHA-256 hash and the number of lines:

```nurt example
info "/home/user/notes.txt" hash: sha256 details: [text]
```

```json fs-before
{ "format": "nurt-fs-snapshot/1", "entries": [
  { "path": "/home/user/notes.txt", "type": "file", "content_utf8": "line one\nline two\n" }
] }
```

Describe a link; `link_target` and `real_path` name the file it points to:

```nurt example
info "/home/user/current"
```

```json fs-before
{ "format": "nurt-fs-snapshot/1", "entries": [
  { "path": "/home/user/notes.txt", "type": "file", "content_utf8": "line one\nline two\n" },
  { "path": "/home/user/current", "type": "link", "target": "/home/user/notes.txt" }
] }
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_PATH_RELATIVE` | a relative path without a base directory | give an absolute path or set the `base` option |
| `E_UNKNOWN_VALUE` | an unknown hash algorithm or `details` group | use a value listed in the parameter table |
| `E_INVALID_VALUE` | a value of `known` is not an etag | pass the `etag` field of an entry read earlier |
| `E_TYPE` | a parameter has a value of another type | pass a list of names to `details` and a map to `known` |
| `E_PERMISSION` | the metadata or content of an entry cannot be read | check the permissions of the entry and its parents |
| `E_IO` | reading the file system failed | repeat the call; check the volume |

## Related

`list`, `find`, `read`, `hash`, `attributes`.
