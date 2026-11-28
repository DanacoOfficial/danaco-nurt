---
title: Effect report
order: 6
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/site/effect-report.md -->
<!-- Responsibility:   Documents Effect report. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->
# Effect report

Each run returns a description of the effect, not only the fact that something ran. The format is `nurt-report/1`; the schema and the complete field list are in the [specification](../../spec/report.md).

## Status

| Status | Meaning |
|---|---|
| `ok` | the program ran and the effect was applied |
| `planned` | a preview computed the targets and the effect; nothing changed |
| `partial` | the effect was applied to part of the targets; `irreversible` or `error` lists the rest |
| `failed` | the program failed; the member `error` describes it |

## Fields

| Field | Content |
|---|---|
| `op` | number of the journal entry of the operation; absent for reading programs and previews |
| `targets` | number of files, directories and bytes that the program touched |
| `changes` | changed targets on the current page |
| `notable` | unusual effects such as `mass_delete`, `size_shrunk` or `nothing_matched` |
| `irreversible` | effects that undo cannot reverse |
| `records` | result of a reading program: record type, number of matches and rows |
| `error` | code, message, stage, position and corrections of a failed program |

## Verbosity and paging

The call parameter `verbosity` selects `quiet`, `normal` or `detailed`. `limit` sets the number of records or changes on a page and `cursor` reads the next page; reading a page again never runs a changing program again. A response that leaves lines, records or the end of long lines out carries the field `continuation` and its text ends with `more: next`; the command `next` returns the next part (`next 200`, `next all`, `next full`). `format` selects `text`, `json`, `markdown` or `compact`.

## Operations on many targets

An operation with 200 or more targets is a bulk operation. The description shows the extensions, the first and last targets and the unusual effects, and the full list is available through the cursor.

## Errors

An error carries a stable code. [Error codes](../../spec/errors.md) lists the codes, the stage that reports each and the status of the C interface.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
