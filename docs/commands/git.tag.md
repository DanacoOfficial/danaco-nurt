---
command: git.tag
group: git
summary: List tags, or create or delete a tag.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# git.tag

## Synopsis

```text
git.tag <repo: path>
git.tag <repo: path> name: <text> [revision: <text>] [message: <text>] [sign: <bool>]
git.tag <repo: path> delete: <text>
```

## Description

Without `name` and `delete` the command lists the tags in the order of
`git tag --list`. An annotated tag reports its tagger, time, message and the
object it points to; a lightweight tag reports only its target.

`name` creates `refs/tags/<name>` at `revision`. Without `message` the tag is
lightweight. With `message` an annotated tag object is written first; the
message is cleaned as `git tag -m` cleans it (trailing white space removed,
runs of blank lines collapsed, leading and trailing blank lines removed, one
final line feed) and the object is byte-identical to the one `git tag -a -m`
writes for the same tagger and time. The tagger is the committer identity of
the repository: `GIT_COMMITTER_NAME`, `GIT_COMMITTER_EMAIL` and
`GIT_COMMITTER_DATE`, then `user.name` and `user.email`, and the clock of the
execution.

With `sign: true` an annotated tag is signed by the built-in signer with the
key named by `user.signingKey` in the format of `gpg.format` (`openpgp` or
`ssh`); without `sign` the setting `tag.gpgSign` decides, and `sign: false`
creates an unsigned tag when `tag.gpgSign` is `true`. Programs named by
`gpg.program` and similar keys are not run. A signed tag needs a message; a
tag without a message stays lightweight and unsigned.

`delete` removes the tag reference, from its loose file and from
`packed-refs`; the tag object stays in the object database.

A reflog of a tag is written only when `core.logAllRefUpdates` is `always`,
with the message `tag: tagging <id> (<subject>, <date>)` of `git tag`.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `repo` | `path` | required | Repository or a path inside it. |
| `name` | `text` | none | Name of a tag to create, without `refs/tags/`. |
| `revision` | `text` | `"HEAD"` | Revision the new tag points to: a commit, tree, blob or tag. |
| `message` | `text` | none | Message of an annotated tag. |
| `sign` | `bool` | `tag.gpgSign` | Sign the annotated tag with the signing key of the repository configuration. |
| `delete` | `text` | none | Tag to delete. |

`name` and `delete` exclude each other.

## Input and output

No input. A listing produces `git_tag` records with the fields `name`,
`target`, `kind` (`lightweight` or `annotated`), `tagger`, `time` and
`message`. Creating a tag produces the `git_tag` record of the new tag;
deleting a tag produces one `effect` record.

## Effect

Without `name` and `delete` the command only reads. Creating and deleting a
tag are local and reversible: the reference file, `packed-refs` and a reflog
are targets of one journal entry and `undo` restores them byte for byte. The
object of an annotated tag is added to the object database.

## Examples

List the tags.

```nurt example
git.tag "/home/user/repo"
```

```text expected
git_tag name: v1.2.0 kind: annotated target: 4f2a9c1
```

Create a lightweight tag at the current commit.

```nurt example
git.tag "/home/user/repo" name: "build-417"
```

Create an annotated tag at another commit.

```nurt example
git.tag "/home/user/repo" name: "v1.3.0" revision: "release/1.3" message: "Release 1.3.0"
```

Create a signed annotated tag.

```nurt example
git.tag "/home/user/repo" name: "v1.3.1" message: "Release 1.3.1" sign: true
```

Delete a tag.

```nurt example
git.tag "/home/user/repo" delete: "build-417"
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_GIT_REF` | The name is invalid (`invalid_name`), the tag exists (`exists`) or does not exist (`missing`). | `git.tag "/home/user/repo" delete: "v1.3.0"` |
| `E_GIT_REVISION` | `revision` cannot be resolved. | `git.log "/home/user/repo"` |
| `E_GIT_SIGN` | The tag is signed and no key is available (`signer_unavailable`), or signing fails. | Configure `user.signingKey`, or pass `sign: false` |
| `E_MISSING_PARAM` | `sign: true` without `message` (`missing_message`). | Add `message` |
| `E_GIT_LOCKED` | A reference lock exists. | Wait for the other process or remove the stale lock |
| `E_STALE` | A reference changed between planning and writing. | Run the command again |
| `E_ARG` | `name` and `delete` are both given (`param_conflict`). | Pass one of them |

## Related

`git.branch`, `git.log`, `git.show`, `undo`.
