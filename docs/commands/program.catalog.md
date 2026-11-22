---
command: program.catalog
group: program
summary: Browse programs by category: categories, then subcategories or programs.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# program.catalog

## Synopsis

```text
program.catalog [<category>] [name: value]...
```

## Description

`program.catalog` lists the programs of the index by category. It starts a pipeline and accepts no input.

Without `category` the result is one `program_category` record per category that holds at least one program, ordered by the number of programs descending and then by id. Categories come from the built-in taxonomy and from external catalogs; a catalog category with the id of a built-in category, or one that declares it in `same_as`, is the same category. Programs of the computer are placed by their desktop entry, package section or group, manual page section and, failing those, by a keyword classifier; `category_source` on each card tells which.

With `category` and a category divided into subcategories, the result is one `program_category` record per subcategory with programs, in catalog order, followed by the subcategory `_other` for the programs outside every subcategory. With a category without subcategories, or with `subcategory`, the result is the `program_card` records of its programs ordered by name. `subcategory` accepts the id or an alias of the subcategory, and `_other`.

With `query` the programs of the category are ranked as by `program.search` and at most 30 are emitted; `subcategory` is then ignored.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `category` | `text` | none | Category to open, by id or alias; compared after folding. |
| `subcategory` | `text` | none | Subcategory to open; needs `category`. |
| `query` | `text` | none | Keywords, 2 to 200 characters, that rank the programs of the category. |
| `source` | `text` | none | Only programs from this kind of provider (`path`, `package`, `app`, `catalog`) or this provider id, such as `dpkg` or `catalog:<id>`. |

## Input and output

| Form | Input | Output |
|---|---|---|
| default | `none` | `program_category`, `program_card` |

Records of `program_category` carry the additional fields `level` (`category` or `subcategory`), `parent`, `summary`, `aliases`, `sources` and `counts_by_source`.

## Effect

| Property | Value |
|---|---|
| Category | `source` |
| Effect | `read` |
| Reversibility | `read` |
| Implementation | `builtin` |

## Examples

List the categories of the programs of the computer.

```nurt example
program.catalog
```

```text expected
name      count  title
system    412    System and administration
code      138    Development
network   57     Network
```

Open one category and show its programs.

```nurt example
program.catalog "archives" |> select name, summary
```

Rank the programs of a category from one catalog.

```nurt example
program.catalog "video" query: "subtitles" source: "catalog" |> first 5
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_INVALID_VALUE` | The category is unknown or holds no program; `candidates` lists the nearest categories. | Use a listed category. |
| `E_ARG` | `subcategory` was given without `category`, the category has no subcategories, the subcategory is unknown, or the query is shorter than 2 or longer than 200 characters. | Pass `category`, omit `subcategory`, or use a listed subcategory. |
| `E_CATALOG` | `source` names a program catalog that is not configured or cannot be read. | Check `programs.catalogs`. |

## Related

`program.search`, `program.describe`, `program.find`.
