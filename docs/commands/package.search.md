---
command: package.search
group: package
summary: Search the package repositories.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# package.search

## Synopsis

```text
package.search <query> [name: value]...
```

## Description

`package.search` finds packages in the local indexes of the repositories of the managers found on the system. It starts a pipeline and accepts no input, and it starts no program. The indexes are those the managers keep after a refresh; the command never contacts a repository.

`query` is matched against the package name and description without regard to case. A package that appears in several repositories or architectures is reported once, with the highest version. `installed` is `true` for packages that are installed; for them `version` is the installed version and `available_version` is set when the repository offers a newer one. The package whose name equals `query` comes first, the rest follow in name order.

Supported indexes: apt lists (plain, gzip, xz, bzip2, lz4 and zstd), dnf and zypper repository metadata, pacman sync databases, the Homebrew API cache, Scoop bucket manifests and the winget source index. Chocolatey keeps no local index and is reported with the warning `no_local_index`. Missing or damaged indexes are reported as warnings and skipped.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `query` | `text` | required | Search terms. |
| `manager` | `enum(auto\|apt\|dnf\|pacman\|zypper\|brew\|winget\|choco\|scoop)` | `auto` | Package manager; auto selects the manager of the system. |

## Input and output

| Form | Input | Output |
|---|---|---|
| default | `none` | `package` |

## Effect

| Property | Value |
|---|---|
| Category | `source` |
| Effect | `read` |
| Reversibility | `read` |
| Implementation | `package_manager` |

## Examples

Search for a package by name.

```nurt example
package.search "ripgrep"
```

Search one manager and keep the packages that are not installed.

```nurt example
package.search "editor" manager: apt |> filter installed == false
```

Show the first ten matches with their repositories.

```nurt example
package.search "json" |> first 10 |> select name version source
```

A response that leaves records out ends with the hint "more: next" and carries the field
`continuation`; the command `next` returns the next part without running the
command again.

```text
matched 330  shown 50  more: next
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_EXTERNAL_MISSING` | No supported package manager was found, or the given one is not installed. | Pass `manager` of an installed manager. |
| `E_FORMAT` | Every repository index of the given manager is unreadable. | Refresh the indexes with the manager. |
| `E_LIMIT` | The matches exceed the record limit. | Use a more specific query. |

## Related

`package.list`, `package.install`.
