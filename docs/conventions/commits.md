<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/conventions/commits.md -->
<!-- Responsibility:   Documents Commit conventions. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.2, 2026-11-28 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# Commit conventions

Commit messages follow [Conventional Commits 1.0.0](https://www.conventionalcommits.org/en/v1.0.0/)
and every commit is signed off under the Developer Certificate of Origin 1.1
(see [CONTRIBUTING.md](../../CONTRIBUTING.md)). Pull requests are checked by
`tools/ci/check-commits.sh`.

Commits that were published before these checks applied to them are listed
with their full 40-character identifiers in `tools/ci/commits-allow.txt` and
are skipped by the script, because published history is not rewritten. A new
commit is never added to that list.

## Format

```text
<type>(<scope>)[!]: <summary>

<body>

<footers>
Signed-off-by: Jane Doe <jane.doe@example.com>
```

### Type

| Type | Use |
|---|---|
| `feat` | a new capability visible to users or integrators |
| `fix` | a correction of a defect |
| `perf` | a performance improvement without a change of behaviour |
| `refactor` | a change of the code without a change of behaviour |
| `docs` | documentation only |
| `test` | tests only |
| `build` | build configuration, dependencies, packaging |
| `ci` | continuous integration |
| `style` | formatting only |
| `chore` | maintenance that does not fit another type |
| `revert` | reverts an earlier commit; the body names the reverted commit |

### Scope

The scope is the name of the affected crate without the `nurt-` prefix, or a
repository area:

| Scope | Content |
|---|---|
| `contracts`, `syntax`, `journal`, `git-ops`, … | the crate `nurt-<scope>` in `crates/` |
| `workspace` | the root manifest, the lock file, the toolchain file |
| `deps` | dependency updates |
| `ci` | workflows and scripts of continuous integration |
| `spec` | the specification in `spec/` |
| `catalog` | the command catalog in `catalog/` |
| `tables` | data tables in `tables/` |
| `docs` | documentation in `docs/` |
| `tools` | scripts in `tools/` |
| `release` | release preparation |
| `repo` | repository files: license, notice, readme, contribution guide |

The scope may be omitted for a change that affects many areas.

### Summary

- imperative mood (`add`, `fix`, `remove`), starting with a lowercase letter;
- no period at the end;
- at most 72 characters for the whole first line.

### Body and footers

The body explains the motivation and the effect of the change, wrapped at 72
characters. Footers follow the Git trailer format:

- `BREAKING CHANGE: <description>` for a change that breaks compatibility,
  together with `!` after the scope;
- `Refs: #<issue>` or `Closes: #<issue>` for related issues;
- `Signed-off-by: <name> <e-mail>`, matching the author of the commit.

## Examples

```text
feat(journal): record the pre-image of truncated files

Truncation is now recorded before the file is opened for writing, so
that undo restores the original content after an interrupted write.

Closes: #142
Signed-off-by: Jane Doe <jane.doe@example.com>
```

```text
fix(syntax)!: reject duplicate named arguments

BREAKING CHANGE: a stage with the same named argument given twice is now
rejected with an error instead of using the last value.
Signed-off-by: Jane Doe <jane.doe@example.com>
```

```text
build(deps): update regex to 1.13.2

Signed-off-by: Jane Doe <jane.doe@example.com>
```

## Breaking changes

A breaking change of the language, the syntax tree, the report format, the
command catalog or the C interface is permitted only in a new major version of
the affected element, as defined by the specification.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
