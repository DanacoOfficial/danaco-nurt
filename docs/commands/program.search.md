---
command: program.search
group: program
summary: Search installed programs, package databases, system applications and configured program catalogs.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# program.search

## Synopsis

```text
program.search <query> [name: value]...
```

## Description

`program.search` ranks the cards of the program index against a query and emits the best `program_card` records with their `score`, highest first. It starts a pipeline and accepts no input.

The index merges the programs in `PATH`, the package databases (dpkg, rpm, pacman, Homebrew, winget, Scoop, Chocolatey), registered applications (desktop entries, Windows uninstall entries and application paths, macOS application bundles), the `NAME` lines of manual pages and the external catalogs of `programs.catalogs`. It works without configuration on the programs of the computer. The first call of a process builds the index; later calls reuse it while the files of every provider are unchanged. With `programs.index_dir` set, each provider is also kept in a cache file and a new process starts from it.

Query words are compared after folding: lower case, compatibility decomposition and no diacritics, so a query typed without diacritics matches a text with them. Common conjunctions and prepositions are ignored when other words remain. Each word scores 100 when it equals the program name, 40 when the name starts with it, 20 when the name contains it, 8 in the situations of use, 5 in the summary, 5 in the category and 3 in the guide description; a query whose every word is found somewhere gains 15. A word without any match is retried at half weight through the built-in table of synonyms, the synonyms of catalogs and a prefix stem of words of six characters or more. A program whose `not_for` entry covers every word of the query loses 30 points and gets the field `instead` with the program to use. Equal scores are ordered by provider priority and then by name. Results with no points are dropped.

A list of queries gives one ranked list per query; each record then has the field `query`.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `query` | `text \| list<text>` | required | Task, topic or program name, 2 to 200 characters; a list holds up to 8 queries. |
| `category` | `text` | none | Only programs in this category or subcategory; compared after folding, aliases of catalog categories are accepted. |
| `source` | `list<enum(path\|package\|app\|catalog)>` | `[path, package, app, catalog]` | Kinds of providers searched. |
| `limit` | `int` | `15` | Maximum number of programs per query, 1 to 60. |

## Input and output

| Form | Input | Output |
|---|---|---|
| default | `none` | `program_card` |

Each record carries the fields of `program_card` and the additional fields `score`, `kind` (`program`, `app` or `guide`), `has_guide`, `category_source` and, when they apply, `query`, `instead`, `guide_name`, `guide_shared_with`, `overview`, `alternatives`, `catalog_path`, `manager`, `app_id` and `language`.

## Effect

| Property | Value |
|---|---|
| Category | `source` |
| Effect | `read` |
| Reversibility | `read` |
| Implementation | `builtin` |

Providers that cannot be read, and providers cut by the time budget of the index, are reported as warnings (`source_unavailable`, `index_partial`); the command still returns the results of the other providers.

## Examples

Find programs for a task.

```nurt example
program.search "convert video" limit: 5
```

```text expected
name     category  summary
ffmpeg   video     Converts and streams audio and video.
handbrake video    Transcodes video files.
```

Search only the installed packages and the search path.

```nurt example
program.search "pdf merge" source: [path, package] |> select name, path, score
```

Run two queries at once and keep the best match of each.

```nurt example
program.search ["compress files", "edit images"] limit: 1 |> select query, name
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_MISSING_PARAM` | No query was given. | Pass a query. |
| `E_ARG` | A query is shorter than 2 or longer than 200 characters. | Shorten or extend the query. |
| `E_INVALID_VALUE` | `limit` is outside 1 to 60, more than 8 queries were given, or the category is unknown; `candidates` lists the nearest categories. | Correct the value, or list the categories with `program.catalog`. |
| `E_CATALOG` | `source` names a program catalog that is not configured or cannot be read. | Check `programs.catalogs`. |

## Related

`program.describe`, `program.catalog`, `program.find`.
