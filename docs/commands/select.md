---
command: select
group: core
summary: Keep the given fields of each record and add computed fields.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# select

## Synopsis

```text
select <fields>...
```

## Description

`select` keeps the named fields of each record, in the order written, and
adds computed fields written as `name: expression`. A field path such as
`owner.name` reads a field inside another field and is named after its last
segment.

Records that only keep top-level fields keep their record kind. Records with
a computed field, or with a field read from inside another field, become
`map` records. A field the record does not have stops the program with
`E_UNKNOWN_FIELD`, which lists the fields of the record.

The stage is lazy and keeps the header of the stream. The order declared by
the source and its presentation hint are dropped, because they may name
fields that are no longer present. An empty input gives an empty output.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `fields` | `list<field \| named_expression>` | required | Fields to keep and name: expression pairs to compute. |

## Input and output

| Form | Input | Output |
|---|---|---|
| stage | any record kind | records of the input kind with the kept fields, or `map` records with computed fields |

## Effect

Category `transform`, effect `read`, reversibility `read`. The command reads
no files and changes nothing.

## Examples

Keep the path and the size of each file:

```nurt example
find "/home/user/project" |> select path size
```

Add a computed field that marks large files:

```nurt example
find "/home/user/project" |> select path large: size > 2kb
```

Keep the exit code and the output of a program run:

```nurt example
run ["git", "status", "--short"] cwd: "/home/user/project" |> select exit_code stdout
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_MISSING_PARAM` | no field is given | name at least one field |
| `E_UNKNOWN_FIELD` | a field is not in the record; `expected` lists the fields | use one of the listed fields |
| `E_TYPE` | an item is neither a field nor `name: expression` | write a field name or `name: expression` |

## Related

`filter`, `sort`, `distinct`, `describe`.
