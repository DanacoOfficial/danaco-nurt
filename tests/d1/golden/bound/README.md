<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
# Bound golden trees

Bound trees (`nurt-bound/1`) of programs bound with the reference catalog.

## Layout

| Path | Content |
|---|---|
| `a/<name>.json` | bound tree of the program written in variant A |
| `b/<name>.json` | bound tree of the program written in variant B |
| `c/<name>.json` | bound tree of the program written in variant C |
| `<variant>/<name>.hash` | `ast_hash` of `<variant>/<name>.json` |

`<name>` is the name of the program in `spec/golden/ast/` (`task-NN-<name>`,
`example-NN-<name>`) or a name of its own for a case without a surface golden
file. Trees are written with spans and without `meta`.

## Rules checked

For every tree:

1. the tree is valid under `spec/schemas/nurt-bound-1.schema.json`;
2. the tree decodes and passes `validate_bound`: phase order, agreement of
   `effect` with `phase`, canonical absolute paths, RFC 3339 instants, unique
   parameter names and map keys;
3. the tree survives the round trip JSON → tree → CBOR → tree unchanged and
   its deterministic CBOR encoding is accepted by the strict decoder;
4. the hash of the tree equals `<name>.hash`.

A program written in several variants binds to one tree: the files of one
name under `a/`, `b/` and `c/` have the same hash.

## Regeneration

```
NURT_BLESS=1 cargo test -p nurt-syntax --test hash_golden
```

writes every `<name>.hash` and the index `../hashes.tsv`. A regenerated file
is committed after review of the difference.
